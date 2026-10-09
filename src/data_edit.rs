//! Edição de dados pela grade (spec §6.1–6.2).
//!
//! Um resultado só é editável quando pode ser mapeado sem ambiguidade para uma
//! tabela: todas as colunas que vêm de tabela vêm da mesma, ela é uma tabela
//! (não view) com chave primária e todas as colunas da chave estão no
//! resultado. Colunas calculadas continuam visíveis, mas não editáveis.
//!
//! As alterações viram um `UPDATE ... RETURNING` por linha, com valores em
//! parâmetros (`$n::text::<tipo>`, conversão pela entrada textual do tipo). O
//! texto exibido no popup troca os parâmetros por literais equivalentes.

use std::collections::BTreeMap;

use crate::metadata::{Catalog, RelationKind, quote_ident};
use crate::query::{ResultSet, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct EditTarget {
    pub schema: String,
    pub table: String,
    /// Colunas do resultado que formam a chave primária.
    pub key: Vec<usize>,
    /// Para cada coluna do resultado, nome e tipo na tabela, se editável.
    pub columns: Vec<Option<TargetColumn>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetColumn {
    pub name: String,
    pub type_name: String,
}

/// Alterações pendentes: (linha, coluna do resultado) → novo valor
/// (`None` = NULL), em formato de entrada textual do PostgreSQL.
pub type Pending = BTreeMap<(usize, usize), Option<String>>;

pub fn edit_target(result: &ResultSet, catalog: &Catalog) -> Result<EditTarget, String> {
    let mut oids = result
        .columns
        .iter()
        .filter_map(|c| c.table_oid.filter(|o| *o != 0));
    let oid = oids.next().ok_or("o resultado não vem de uma tabela")?;
    if oids.any(|o| o != oid) {
        return Err("o resultado combina colunas de mais de uma tabela".into());
    }
    let relation = catalog
        .by_oid(oid)
        .ok_or("a tabela de origem não está nos metadados carregados (F5 atualiza)")?;
    if !matches!(
        relation.kind,
        RelationKind::Table | RelationKind::PartitionedTable
    ) {
        return Err(format!(
            "{} é uma {}, não uma tabela",
            relation.name,
            relation.kind.label()
        ));
    }

    let columns: Vec<Option<TargetColumn>> = result
        .columns
        .iter()
        .map(|c| {
            let number = c.column_id.filter(|_| c.table_oid == Some(oid))?;
            let meta = relation.columns.iter().find(|m| m.number == number)?;
            Some(TargetColumn {
                name: meta.name.clone(),
                type_name: meta.type_name.clone(),
            })
        })
        .collect();

    let pk: Vec<&str> = relation
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    if pk.is_empty() {
        return Err(format!("{} não tem chave primária", relation.name));
    }
    let mut key = Vec::new();
    for name in pk {
        let matches: Vec<usize> = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.as_ref().is_some_and(|c| c.name == name))
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [i] => key.push(*i),
            [] => {
                return Err(format!(
                    "a coluna de chave primária {name} não está no resultado"
                ));
            }
            _ => {
                return Err(format!(
                    "a coluna de chave primária {name} aparece mais de uma vez"
                ));
            }
        }
    }

    Ok(EditTarget {
        schema: relation.schema.clone(),
        table: relation.name.clone(),
        key,
        columns,
    })
}

/// Um `UPDATE` de uma linha.
#[derive(Debug, Clone, PartialEq)]
pub struct RowUpdate {
    pub row: usize,
    /// Colunas do resultado alteradas, na ordem do `SET` e do `RETURNING`.
    pub columns: Vec<usize>,
    pub sql: String,
    pub params: Vec<Option<String>>,
    /// SQL equivalente com literais, para exibir e copiar.
    pub display: String,
}

pub fn build_updates(
    target: &EditTarget,
    result: &ResultSet,
    pending: &Pending,
) -> Result<Vec<RowUpdate>, String> {
    let mut by_row: BTreeMap<usize, Vec<(usize, &Option<String>)>> = BTreeMap::new();
    for ((row, col), value) in pending {
        by_row.entry(*row).or_default().push((*col, value));
    }

    let table = format!(
        "{}.{}",
        quote_ident(&target.schema),
        quote_ident(&target.table)
    );
    let mut updates = Vec::new();
    for (row, changes) in by_row {
        let values = result.rows.get(row).ok_or("linha inexistente")?;
        let mut params: Vec<Option<String>> = Vec::new();
        let mut set = Vec::new();
        let mut set_display = Vec::new();
        let mut columns = Vec::new();
        for (col, value) in changes {
            let column = target.columns[col]
                .as_ref()
                .ok_or_else(|| format!("a coluna {} não é editável", result.columns[col].name))?;
            params.push(value.clone());
            set.push(format!(
                "{} = ${}::text::{}",
                quote_ident(&column.name),
                params.len(),
                column.type_name
            ));
            set_display.push(format!(
                "{} = {}",
                quote_ident(&column.name),
                literal(value.as_deref(), &column.type_name)
            ));
            columns.push(col);
        }

        let mut condition = Vec::new();
        let mut condition_display = Vec::new();
        for &col in &target.key {
            let column = target.columns[col]
                .as_ref()
                .expect("coluna da chave mapeada");
            let original = key_text(&values[col]).ok_or_else(|| {
                format!(
                    "o valor da chave {} na linha {} não pode ser usado",
                    column.name,
                    row + 1
                )
            })?;
            params.push(Some(original.clone()));
            condition.push(format!(
                "{} = ${}::text::{}",
                quote_ident(&column.name),
                params.len(),
                column.type_name
            ));
            condition_display.push(format!(
                "{} = {}",
                quote_ident(&column.name),
                literal(Some(&original), &column.type_name)
            ));
        }

        let returning: Vec<String> = columns
            .iter()
            .map(|c| quote_ident(&target.columns[*c].as_ref().expect("coluna mapeada").name))
            .collect();
        updates.push(RowUpdate {
            row,
            sql: format!(
                "UPDATE {table} SET {} WHERE {} RETURNING {}",
                set.join(", "),
                condition.join(" AND "),
                returning.join(", ")
            ),
            display: format!(
                "UPDATE {table}\nSET {}\nWHERE {};",
                set_display.join(", "),
                condition_display.join(" AND ")
            ),
            params,
            columns,
        });
    }
    Ok(updates)
}

/// Texto do valor original de uma chave, na sintaxe de entrada do PostgreSQL.
fn key_text(value: &Value) -> Option<String> {
    match value {
        Value::Null | Value::Other { .. } | Value::Array { .. } => None,
        Value::Float(f) if !f.is_finite() => None,
        other => Some(other.display()),
    }
}

/// Literal SQL equivalente ao parâmetro `$n::text::tipo`.
pub fn literal(value: Option<&str>, type_name: &str) -> String {
    match value {
        None => format!("NULL::{type_name}"),
        Some(v) => format!("'{}'::{type_name}", v.replace('\'', "''")),
    }
}

/// Texto inicial do editor de uma célula.
pub fn edit_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        other => other.display(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{ColumnMeta, Relation, Schema};
    use crate::query::ColumnInfo;

    fn catalog() -> Catalog {
        let col = |number, name: &str, type_name: &str, pk: bool| ColumnMeta {
            number,
            name: name.into(),
            type_name: type_name.into(),
            nullable: !pk,
            primary_key: pk,
        };
        Catalog {
            schemas: vec![Schema {
                name: "public".into(),
                relations: vec![
                    Relation {
                        oid: 10,
                        schema: "public".into(),
                        name: "clientes".into(),
                        kind: RelationKind::Table,
                        columns: vec![
                            col(1, "id", "integer", true),
                            col(2, "nome", "text", false),
                            col(3, "valor", "numeric(10,2)", false),
                        ],
                    },
                    Relation {
                        oid: 20,
                        schema: "public".into(),
                        name: "sem_pk".into(),
                        kind: RelationKind::Table,
                        columns: vec![col(1, "a", "integer", false)],
                    },
                    Relation {
                        oid: 30,
                        schema: "public".into(),
                        name: "v".into(),
                        kind: RelationKind::View,
                        columns: vec![col(1, "a", "integer", false)],
                    },
                ],
                ..Schema::default()
            }],
        }
    }

    fn column(name: &str, table: Option<u32>, id: Option<i16>) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            type_name: "x".into(),
            table_oid: table,
            column_id: id,
        }
    }

    fn result(columns: Vec<ColumnInfo>, rows: Vec<Vec<Value>>) -> ResultSet {
        ResultSet {
            columns,
            rows,
            has_more: false,
            truncated: false,
        }
    }

    #[test]
    fn maps_single_table_with_primary_key() {
        let rs = result(
            vec![
                column("nome", Some(10), Some(2)),
                column("id", Some(10), Some(1)),
                column("x", None, None),
            ],
            vec![],
        );
        let t = edit_target(&rs, &catalog()).unwrap();
        assert_eq!(t.key, vec![1]);
        assert_eq!(t.columns[0].as_ref().unwrap().name, "nome");
        assert!(t.columns[2].is_none());
    }

    #[test]
    fn refuses_ambiguous_results() {
        let c = catalog();
        let no_pk = result(vec![column("nome", Some(10), Some(2))], vec![]);
        assert!(
            edit_target(&no_pk, &c)
                .unwrap_err()
                .contains("chave primária id")
        );
        let join = result(
            vec![
                column("id", Some(10), Some(1)),
                column("a", Some(20), Some(1)),
            ],
            vec![],
        );
        assert!(
            edit_target(&join, &c)
                .unwrap_err()
                .contains("mais de uma tabela")
        );
        let table_without_pk = result(vec![column("a", Some(20), Some(1))], vec![]);
        assert!(
            edit_target(&table_without_pk, &c)
                .unwrap_err()
                .contains("não tem chave primária")
        );
        let view = result(vec![column("a", Some(30), Some(1))], vec![]);
        assert!(edit_target(&view, &c).is_err());
        let computed = result(vec![column("?column?", None, None)], vec![]);
        assert!(edit_target(&computed, &c).is_err());
    }

    #[test]
    fn builds_parameterized_updates() {
        let rs = result(
            vec![
                column("id", Some(10), Some(1)),
                column("nome", Some(10), Some(2)),
                column("valor", Some(10), Some(3)),
            ],
            vec![
                vec![
                    Value::Int(7),
                    Value::Text("Ana".into()),
                    Value::Numeric("1.50".into()),
                ],
                vec![Value::Int(8), Value::Text("Bia".into()), Value::Null],
            ],
        );
        let t = edit_target(&rs, &catalog()).unwrap();
        let mut pending = Pending::new();
        pending.insert((0, 1), Some("O'Brien".into()));
        pending.insert((0, 2), None);
        pending.insert((1, 2), Some("2".into()));
        let ups = build_updates(&t, &rs, &pending).unwrap();
        assert_eq!(ups.len(), 2);
        assert_eq!(
            ups[0].sql,
            "UPDATE public.clientes SET nome = $1::text::text, valor = $2::text::numeric(10,2) \
             WHERE id = $3::text::integer RETURNING nome, valor"
        );
        assert_eq!(
            ups[0].params,
            vec![Some("O'Brien".into()), None, Some("7".into())]
        );
        assert_eq!(
            ups[0].display,
            "UPDATE public.clientes\nSET nome = 'O''Brien'::text, valor = NULL::numeric(10,2)\nWHERE id = '7'::integer;"
        );
        assert_eq!(ups[1].columns, vec![2]);
    }

    #[test]
    fn null_keys_cannot_be_used() {
        let rs = result(
            vec![
                column("id", Some(10), Some(1)),
                column("nome", Some(10), Some(2)),
            ],
            vec![vec![Value::Null, Value::Text("x".into())]],
        );
        let t = edit_target(&rs, &catalog()).unwrap();
        let mut pending = Pending::new();
        pending.insert((0, 1), Some("y".into()));
        assert!(build_updates(&t, &rs, &pending).is_err());
    }
}
