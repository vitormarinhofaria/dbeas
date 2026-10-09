// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Metadados do banco: schemas, tabelas, views, colunas, funções e sequências.
//!
//! O catálogo é carregado de uma vez pela conexão do workspace e fica em cache
//! no estado transitório do workspace, nunca na sessão persistida.

use tokio_postgres::Client;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelationKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}

impl RelationKind {
    fn from_relkind(c: &str) -> Option<Self> {
        Some(match c {
            "r" => Self::Table,
            "p" => Self::PartitionedTable,
            "v" => Self::View,
            "m" => Self::MaterializedView,
            "f" => Self::ForeignTable,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "tabela",
            Self::PartitionedTable => "tabela particionada",
            Self::View => "view",
            Self::MaterializedView => "view materializada",
            Self::ForeignTable => "tabela externa",
        }
    }

    pub fn is_view(self) -> bool {
        matches!(self, Self::View | Self::MaterializedView)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnMeta {
    /// Número da coluna na tabela (`attnum`).
    pub number: i16,
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
    pub columns: Vec<ColumnMeta>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Schema {
    pub name: String,
    pub relations: Vec<Relation>,
    pub functions: Vec<String>,
    pub sequences: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Catalog {
    pub schemas: Vec<Schema>,
}

impl Catalog {
    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.schemas.iter().flat_map(|s| s.relations.iter())
    }

    pub fn by_oid(&self, oid: u32) -> Option<&Relation> {
        self.relations().find(|r| r.oid == oid)
    }

    pub fn find(&self, schema: &str, name: &str) -> Option<&Relation> {
        self.schemas
            .iter()
            .find(|s| s.name == schema)
            .and_then(|s| s.relations.iter().find(|r| r.name == name))
    }
}

const SCHEMAS_SQL: &str = "
    SELECT n.nspname::text
    FROM pg_namespace n
    WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
      AND n.nspname NOT LIKE 'pg\\_toast%'
      AND n.nspname NOT LIKE 'pg\\_temp%'
      AND has_schema_privilege(n.oid, 'USAGE')
    ORDER BY n.nspname";

const RELATIONS_SQL: &str = "
    SELECT n.nspname::text, c.relname::text, c.relkind::text, c.oid
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
      AND NOT c.relispartition
      AND n.nspname NOT IN ('pg_catalog', 'information_schema')
      AND n.nspname NOT LIKE 'pg\\_toast%'
      AND n.nspname NOT LIKE 'pg\\_temp%'
      AND has_schema_privilege(n.oid, 'USAGE')
      AND has_table_privilege(c.oid, 'SELECT')
    ORDER BY n.nspname, c.relname";

const COLUMNS_SQL: &str = "
    SELECT n.nspname::text, c.relname::text, a.attname::text,
           format_type(a.atttypid, a.atttypmod)::text, NOT a.attnotnull,
           COALESCE(i.indisprimary, false), a.attnum
    FROM pg_attribute a
    JOIN pg_class c ON c.oid = a.attrelid
    JOIN pg_namespace n ON n.oid = c.relnamespace
    LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey)
    WHERE a.attnum > 0 AND NOT a.attisdropped
      AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
      AND NOT c.relispartition
      AND n.nspname NOT IN ('pg_catalog', 'information_schema')
      AND n.nspname NOT LIKE 'pg\\_toast%'
      AND n.nspname NOT LIKE 'pg\\_temp%'
      AND has_schema_privilege(n.oid, 'USAGE')
      AND has_table_privilege(c.oid, 'SELECT')
    ORDER BY n.nspname, c.relname, a.attnum";

const FUNCTIONS_SQL: &str = "
    SELECT n.nspname::text, p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')'
    FROM pg_proc p
    JOIN pg_namespace n ON n.oid = p.pronamespace
    WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
      AND has_schema_privilege(n.oid, 'USAGE')
    ORDER BY 1, 2";

const SEQUENCES_SQL: &str = "
    SELECT n.nspname::text, c.relname::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind = 'S'
      AND n.nspname NOT IN ('pg_catalog', 'information_schema')
      AND has_schema_privilege(n.oid, 'USAGE')
    ORDER BY 1, 2";

pub async fn load(client: &Client) -> Result<Catalog, String> {
    let err = |e: tokio_postgres::Error| format!("falha ao ler metadados: {e}");

    let mut schemas: Vec<Schema> = client
        .query(SCHEMAS_SQL, &[])
        .await
        .map_err(err)?
        .iter()
        .map(|r| Schema {
            name: r.get(0),
            ..Schema::default()
        })
        .collect();

    let index = |schemas: &Vec<Schema>, name: &str| schemas.iter().position(|s| s.name == name);

    for row in client.query(RELATIONS_SQL, &[]).await.map_err(err)? {
        let schema: String = row.get(0);
        let kind: String = row.get(2);
        let (Some(i), Some(kind)) = (index(&schemas, &schema), RelationKind::from_relkind(&kind))
        else {
            continue;
        };
        schemas[i].relations.push(Relation {
            schema,
            name: row.get(1),
            oid: row.get(3),
            kind,
            columns: Vec::new(),
        });
    }

    for row in client.query(COLUMNS_SQL, &[]).await.map_err(err)? {
        let schema: String = row.get(0);
        let table: String = row.get(1);
        let Some(i) = index(&schemas, &schema) else {
            continue;
        };
        if let Some(rel) = schemas[i].relations.iter_mut().find(|r| r.name == table) {
            rel.columns.push(ColumnMeta {
                name: row.get(2),
                type_name: row.get(3),
                nullable: row.get(4),
                primary_key: row.get(5),
                number: row.get(6),
            });
        }
    }

    for row in client.query(FUNCTIONS_SQL, &[]).await.map_err(err)? {
        let schema: String = row.get(0);
        if let Some(i) = index(&schemas, &schema) {
            schemas[i].functions.push(row.get(1));
        }
    }

    for row in client.query(SEQUENCES_SQL, &[]).await.map_err(err)? {
        let schema: String = row.get(0);
        if let Some(i) = index(&schemas, &schema) {
            schemas[i].sequences.push(row.get(1));
        }
    }

    Ok(Catalog { schemas })
}

/// Identificador citado para uso em SQL gerado pela aplicação.
pub fn quote_ident(name: &str) -> String {
    let simple = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !is_reserved(name);
    if simple {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// Palavras reservadas do PostgreSQL (tabela C.1 da documentação, colunas
/// "reserved" e "reserved (can be function or type)").
fn is_reserved(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "all",
        "analyse",
        "analyze",
        "and",
        "any",
        "array",
        "as",
        "asc",
        "asymmetric",
        "authorization",
        "binary",
        "both",
        "case",
        "cast",
        "check",
        "collate",
        "collation",
        "column",
        "concurrently",
        "constraint",
        "create",
        "cross",
        "current_catalog",
        "current_date",
        "current_role",
        "current_schema",
        "current_time",
        "current_timestamp",
        "current_user",
        "default",
        "deferrable",
        "desc",
        "distinct",
        "do",
        "else",
        "end",
        "except",
        "false",
        "fetch",
        "for",
        "foreign",
        "freeze",
        "from",
        "full",
        "grant",
        "group",
        "having",
        "ilike",
        "in",
        "initially",
        "inner",
        "intersect",
        "into",
        "is",
        "isnull",
        "join",
        "lateral",
        "leading",
        "left",
        "like",
        "limit",
        "localtime",
        "localtimestamp",
        "natural",
        "not",
        "notnull",
        "null",
        "offset",
        "on",
        "only",
        "or",
        "order",
        "outer",
        "overlaps",
        "placing",
        "primary",
        "references",
        "returning",
        "right",
        "select",
        "session_user",
        "similar",
        "some",
        "symmetric",
        "system_user",
        "table",
        "tablesample",
        "then",
        "to",
        "trailing",
        "true",
        "union",
        "unique",
        "user",
        "using",
        "variadic",
        "verbose",
        "when",
        "where",
        "window",
        "with",
    ];
    RESERVED.contains(&name)
}

/// Consulta inicial de uma aba de tabela: seleciona as colunas explicitamente.
pub fn select_for(relation: &Relation) -> String {
    let table = format!(
        "{}.{}",
        quote_ident(&relation.schema),
        quote_ident(&relation.name)
    );
    if relation.columns.is_empty() {
        return format!("SELECT *\nFROM {table}");
    }
    let columns: Vec<String> = relation
        .columns
        .iter()
        .map(|c| quote_ident(&c.name))
        .collect();
    format!("SELECT {}\nFROM {table}", columns.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote_ident("users"), "users");
        assert_eq!(quote_ident("Users"), "\"Users\"");
        assert_eq!(quote_ident("order"), "\"order\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident("1x"), "\"1x\"");
    }

    #[test]
    fn select_lists_columns() {
        let rel = Relation {
            oid: 1,
            schema: "public".into(),
            name: "Pedidos".into(),
            kind: RelationKind::Table,
            columns: vec![
                ColumnMeta {
                    number: 1,
                    name: "id".into(),
                    type_name: "int4".into(),
                    nullable: false,
                    primary_key: true,
                },
                ColumnMeta {
                    number: 2,
                    name: "user".into(),
                    type_name: "text".into(),
                    nullable: true,
                    primary_key: false,
                },
            ],
        };
        assert_eq!(
            select_for(&rel),
            "SELECT id, \"user\"\nFROM public.\"Pedidos\""
        );
    }
}
