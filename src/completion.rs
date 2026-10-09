// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Autocomplete de SQL a partir do catálogo do workspace ativo (spec §7.1).
//!
//! Heurística léxica, tolerante a SQL incompleto (o texto está sendo digitado):
//! - depois de `alias.`/`tabela.` → colunas daquela tabela;
//! - depois de `schema.` → tabelas e views do schema;
//! - depois de FROM/JOIN/UPDATE/INTO/TABLE → tabelas, views e schemas;
//! - nos demais lugares → colunas das tabelas citadas na instrução e palavras-chave.

use crate::metadata::{Catalog, Relation, RelationKind, quote_ident};
use crate::query::split;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Column,
    Table,
    View,
    Schema,
    Keyword,
}

impl ItemKind {
    pub fn label(self) -> &'static str {
        match self {
            ItemKind::Column => "coluna",
            ItemKind::Table => "tabela",
            ItemKind::View => "view",
            ItemKind::Schema => "schema",
            ItemKind::Keyword => "palavra-chave",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub insert: String,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// Início (em bytes) do trecho digitado que será substituído.
    pub start: usize,
    pub prefix: String,
    pub items: Vec<Item>,
}

const MAX_ITEMS: usize = 50;

const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "EXISTS",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "FULL",
    "CROSS",
    "ON",
    "USING",
    "GROUP",
    "BY",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "ASC",
    "DESC",
    "NULLS",
    "FIRST",
    "LAST",
    "DISTINCT",
    "AS",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "ALL",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "RETURNING",
    "WITH",
    "TRUE",
    "FALSE",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "CAST",
    "NOW",
    "CREATE",
    "TABLE",
    "ALTER",
    "DROP",
    "INDEX",
    "VIEW",
    "PRIMARY",
    "KEY",
    "REFERENCES",
    "DEFAULT",
];

/// Palavras que encerram a lista de tabelas de um FROM.
const CLAUSE_WORDS: &[&str] = &[
    "WHERE",
    "GROUP",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "SET",
    "VALUES",
    "RETURNING",
    "WINDOW",
    "FOR",
    "ON",
    "USING",
    "SELECT",
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    Dot,
    Comma,
    Other,
}

/// Tokens (com a posição de início) de `text`. Strings e comentários viram
/// `Other`; o último elemento indica se o texto termina dentro de string ou
/// comentário.
fn lex(text: &str) -> (Vec<(usize, Tok)>, bool) {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\'' => {
                let start = i;
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return (out, true),
                        Some(b'\'') if bytes.get(i + 1) == Some(&b'\'') => i += 2,
                        Some(b'\'') => {
                            i += 1;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
                out.push((start, Tok::Other));
            }
            b'"' => {
                let start = i;
                let end = text[i + 1..].find('"').map(|p| i + 1 + p);
                match end {
                    Some(end) => {
                        out.push((start, Tok::Quoted(text[i + 1..end].to_string())));
                        i = end + 1;
                    }
                    None => return (out, true),
                }
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => match text[i..].find('\n') {
                Some(p) => i += p + 1,
                None => return (out, true),
            },
            b'/' if bytes.get(i + 1) == Some(&b'*') => match text[i + 2..].find("*/") {
                Some(p) => i += p + 4,
                None => return (out, true),
            },
            b'.' => {
                out.push((i, Tok::Dot));
                i += 1;
            }
            b',' => {
                out.push((i, Tok::Comma));
                i += 1;
            }
            _ if is_ident_start(text, i) => {
                let start = i;
                while i < bytes.len() && is_ident_continue(text, i) {
                    i += text[i..].chars().next().map_or(1, char::len_utf8);
                }
                out.push((start, Tok::Word(text[start..i].to_string())));
            }
            _ if c.is_ascii_whitespace() => i += 1,
            _ => {
                out.push((i, Tok::Other));
                i += text[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    (out, false)
}

fn is_ident_start(text: &str, i: usize) -> bool {
    text[i..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
}

fn is_ident_continue(text: &str, i: usize) -> bool {
    text[i..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

fn upper(tok: &Tok) -> Option<String> {
    match tok {
        Tok::Word(w) => Some(w.to_ascii_uppercase()),
        _ => None,
    }
}

fn ident(tok: &Tok) -> Option<&str> {
    match tok {
        Tok::Word(w) | Tok::Quoted(w) => Some(w),
        _ => None,
    }
}

/// Tabela citada na instrução e seu alias.
#[derive(Debug, Clone, PartialEq)]
struct TableRef {
    schema: Option<String>,
    name: String,
    alias: Option<String>,
}

fn table_refs(tokens: &[(usize, Tok)]) -> Vec<TableRef> {
    let mut refs = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let starts_list = matches!(
            upper(&tokens[i].1).as_deref(),
            Some("FROM" | "JOIN" | "UPDATE" | "INTO" | "TABLE")
        );
        if !starts_list {
            i += 1;
            continue;
        }
        i += 1;
        loop {
            // [schema.]nome
            let Some(first) = tokens.get(i).and_then(|t| ident(&t.1)) else {
                break;
            };
            if upper(&tokens[i].1).is_some_and(|w| CLAUSE_WORDS.contains(&w.as_str())) {
                break;
            }
            let (schema, name) = if tokens.get(i + 1).map(|t| &t.1) == Some(&Tok::Dot) {
                match tokens.get(i + 2).and_then(|t| ident(&t.1)) {
                    Some(n) => {
                        i += 3;
                        (Some(first.to_string()), n.to_string())
                    }
                    None => break,
                }
            } else {
                i += 1;
                (None, first.to_string())
            };
            // [AS] alias
            if upper(&tokens.get(i).map_or(Tok::Other, |t| t.1.clone())).as_deref() == Some("AS") {
                i += 1;
            }
            let alias = tokens.get(i).and_then(|t| {
                let word = ident(&t.1)?;
                let is_keyword = upper(&t.1).is_some_and(|w| {
                    CLAUSE_WORDS.contains(&w.as_str())
                        || KEYWORDS.contains(&w.as_str())
                        || matches!(w.as_str(), "NATURAL" | "LATERAL")
                });
                (!is_keyword).then(|| word.to_string())
            });
            if alias.is_some() {
                i += 1;
            }
            refs.push(TableRef {
                schema,
                name,
                alias,
            });
            if tokens.get(i).map(|t| &t.1) == Some(&Tok::Comma) {
                i += 1;
                continue;
            }
            break;
        }
    }
    refs
}

fn resolve<'a>(catalog: &'a Catalog, schema: Option<&str>, name: &str) -> Option<&'a Relation> {
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match schema {
        Some(s) => catalog
            .relations()
            .find(|r| eq(&r.schema, s) && eq(&r.name, name)),
        None => catalog
            .relations()
            .find(|r| r.schema == "public" && eq(&r.name, name))
            .or_else(|| catalog.relations().find(|r| eq(&r.name, name))),
    }
}

fn relation_item(r: &Relation, qualified_by_user: bool) -> Item {
    let kind = if matches!(r.kind, RelationKind::View | RelationKind::MaterializedView) {
        ItemKind::View
    } else {
        ItemKind::Table
    };
    let (label, insert) = if qualified_by_user || r.schema == "public" {
        (r.name.clone(), quote_ident(&r.name))
    } else {
        (
            format!("{}.{}", r.schema, r.name),
            format!("{}.{}", quote_ident(&r.schema), quote_ident(&r.name)),
        )
    };
    Item {
        label,
        detail: r.kind.label().to_string(),
        insert,
        kind,
    }
}

fn column_items(r: &Relation, owner: &str) -> Vec<Item> {
    r.columns
        .iter()
        .map(|c| Item {
            label: c.name.clone(),
            detail: format!("{owner} · {}", c.type_name),
            insert: quote_ident(&c.name),
            kind: ItemKind::Column,
        })
        .collect()
}

/// Sugestões para o cursor em `cursor` (bytes). `forced` (Ctrl+Espaço) mostra
/// sugestões mesmo sem prefixo.
pub fn complete(text: &str, cursor: usize, catalog: &Catalog, forced: bool) -> Option<Completion> {
    let cursor = cursor.min(text.len());
    if !text.is_char_boundary(cursor) {
        return None;
    }

    // Instrução onde está o cursor (ou a que está sendo começada).
    let ranges = split::statements(text);
    // O cursor pertence a uma instrução se estiver nela ou logo depois dela,
    // sem `;` no meio (espaços finais não fazem parte da faixa).
    let current = ranges
        .iter()
        .rev()
        .find(|r| r.start <= cursor && (cursor <= r.end || !text[r.end..cursor].contains(';')));
    let stmt_start = match current {
        Some(r) => r.start,
        None => ranges
            .iter()
            .rev()
            .find(|r| r.end < cursor)
            .map_or(0, |r| (r.end + 1).min(cursor)),
    };
    let stmt_end = current.map_or(cursor, |r| r.end.max(cursor));
    let before = &text[stmt_start..cursor];
    let whole = &text[stmt_start..stmt_end];

    let (tokens_before, inside_literal) = lex(before);
    if inside_literal {
        return None;
    }

    // Prefixo: identificador imediatamente antes do cursor.
    let prefix_start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '$')
        .last()
        .map_or(before.len(), |(i, _)| i);
    let prefix = before[prefix_start..].to_string();
    let mut significant: Vec<&Tok> = tokens_before.iter().map(|t| &t.1).collect();
    if !prefix.is_empty() {
        significant.pop();
    }

    let qualifier = match significant.as_slice() {
        [.., q, Tok::Dot] => ident(q).map(str::to_string),
        _ => None,
    };
    if prefix.is_empty() && qualifier.is_none() && !forced {
        return None;
    }

    let (tokens_all, _) = lex(whole);
    let refs = table_refs(&tokens_all);

    let mut items: Vec<Item> = Vec::new();
    if let Some(q) = &qualifier {
        let by_alias = refs.iter().find(|r| {
            r.alias
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(q))
                || (r.alias.is_none() && r.name.eq_ignore_ascii_case(q))
        });
        if let Some(rel) = by_alias.and_then(|r| resolve(catalog, r.schema.as_deref(), &r.name)) {
            items.extend(column_items(rel, &rel.name));
        } else if let Some(schema) = catalog
            .schemas
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(q))
        {
            items.extend(schema.relations.iter().map(|r| relation_item(r, true)));
        } else if let Some(rel) = resolve(catalog, None, q) {
            items.extend(column_items(rel, &rel.name));
        }
    } else {
        let previous = significant.iter().rev().find_map(|t| match t {
            Tok::Word(w) => Some(w.to_ascii_uppercase()),
            Tok::Comma => Some(",".into()),
            _ => None,
        });
        let in_table_list = match previous.as_deref() {
            Some("FROM" | "JOIN" | "UPDATE" | "INTO" | "TABLE") => true,
            // `FROM a, |`
            Some(",") => {
                let last_kw = significant.iter().rev().find_map(|t| {
                    upper(t).filter(|w| {
                        matches!(
                            w.as_str(),
                            "FROM" | "SELECT" | "WHERE" | "SET" | "BY" | "JOIN" | "ON"
                        )
                    })
                });
                last_kw.as_deref() == Some("FROM")
            }
            _ => false,
        };
        if in_table_list {
            items.extend(catalog.relations().map(|r| relation_item(r, false)));
            items.extend(catalog.schemas.iter().map(|s| Item {
                label: s.name.clone(),
                detail: "schema".into(),
                insert: quote_ident(&s.name),
                kind: ItemKind::Schema,
            }));
        } else {
            for r in &refs {
                if let Some(rel) = resolve(catalog, r.schema.as_deref(), &r.name) {
                    let owner = r.alias.clone().unwrap_or_else(|| rel.name.clone());
                    items.extend(column_items(rel, &owner));
                }
            }
            let lowercase = !prefix.is_empty() && prefix.chars().all(|c| !c.is_uppercase());
            items.extend(KEYWORDS.iter().map(|k| {
                let word = if lowercase {
                    k.to_lowercase()
                } else {
                    k.to_string()
                };
                Item {
                    label: word.clone(),
                    detail: "palavra-chave".into(),
                    insert: word,
                    kind: ItemKind::Keyword,
                }
            }));
        }
    }

    // Filtra e ordena: começa com o prefixo > contém; colunas antes de palavras-chave.
    let needle = prefix.to_lowercase();
    let mut scored: Vec<(u8, Item)> = items
        .into_iter()
        .filter_map(|item| {
            let label = item.label.to_lowercase();
            let short = label.rsplit('.').next().unwrap_or(&label).to_string();
            let rank =
                if needle.is_empty() || short.starts_with(&needle) || label.starts_with(&needle) {
                    0
                } else if label.contains(&needle) {
                    1
                } else {
                    return None;
                };
            Some((rank * 10 + u8::from(item.kind == ItemKind::Keyword), item))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
    scored.dedup_by(|a, b| a.1.label == b.1.label && a.1.kind == b.1.kind);
    let items: Vec<Item> = scored.into_iter().map(|(_, i)| i).take(MAX_ITEMS).collect();

    // Nada a completar: única sugestão igual ao que já foi digitado.
    if items.is_empty()
        || (!forced && items.len() == 1 && items[0].insert.eq_ignore_ascii_case(&prefix))
    {
        return None;
    }
    Some(Completion {
        start: stmt_start + prefix_start,
        prefix,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{ColumnMeta, Schema};

    fn catalog() -> Catalog {
        let col = |name: &str| ColumnMeta {
            number: 1,
            name: name.into(),
            type_name: "text".into(),
            nullable: true,
            primary_key: false,
        };
        let rel = |schema: &str, name: &str, cols: &[&str]| Relation {
            oid: 1,
            schema: schema.into(),
            name: name.into(),
            kind: RelationKind::Table,
            columns: cols.iter().map(|c| col(c)).collect(),
        };
        Catalog {
            schemas: vec![
                Schema {
                    name: "public".into(),
                    relations: vec![
                        rel("public", "clientes", &["id", "nome", "email"]),
                        rel("public", "pedidos", &["id", "cliente_id", "total"]),
                    ],
                    ..Schema::default()
                },
                Schema {
                    name: "vendas".into(),
                    relations: vec![rel("vendas", "notas", &["numero", "valor"])],
                    ..Schema::default()
                },
            ],
        }
    }

    fn labels(text: &str, forced: bool) -> Vec<String> {
        let cursor = text.find('|').expect("marcador de cursor");
        let text = text.replace('|', "");
        complete(&text, cursor, &catalog(), forced).map_or(Vec::new(), |c| {
            c.items.into_iter().map(|i| i.label).collect()
        })
    }

    #[test]
    fn tables_after_from() {
        assert_eq!(labels("select * from cl|", false), ["clientes"]);
        let all = labels("select * from |", true);
        assert!(all.contains(&"clientes".to_string()) && all.contains(&"vendas.notas".to_string()));
        assert!(all.contains(&"vendas".to_string()));
        assert_eq!(labels("select * from clientes c, pe|", false), ["pedidos"]);
    }

    #[test]
    fn columns_by_alias_and_table() {
        assert_eq!(
            labels("select c.| from clientes c", false),
            ["email", "id", "nome"]
        );
        assert_eq!(labels("select pedidos.to| from pedidos", false), ["total"]);
        assert_eq!(
            labels("select x.n| from vendas.notas as x", false),
            ["numero"]
        );
    }

    #[test]
    fn schema_qualifier_lists_its_tables() {
        assert_eq!(labels("select * from vendas.|", false), ["notas"]);
    }

    #[test]
    fn columns_of_referenced_tables_then_keywords() {
        let l = labels("select no| from clientes", false);
        assert_eq!(l.first().map(String::as_str), Some("nome"));
        assert!(l.contains(&"not".to_string()));
        let l = labels(
            "select * from clientes c join pedidos p on p.cliente_id = c.id where to|",
            false,
        );
        assert_eq!(l.first().map(String::as_str), Some("total"));
    }

    #[test]
    fn nothing_inside_strings_or_comments_or_without_prefix() {
        assert!(labels("select 'cl|", false).is_empty());
        assert!(labels("-- from cl|", false).is_empty());
        assert!(labels("select |", false).is_empty());
    }

    #[test]
    fn uses_current_statement_only() {
        assert_eq!(
            labels("select * from pedidos; select c.| from clientes c", false),
            ["email", "id", "nome"]
        );
    }

    #[test]
    fn keyword_case_follows_prefix() {
        assert!(labels("sel|", false).contains(&"select".to_string()));
        assert!(labels("SEL|", false).contains(&"SELECT".to_string()));
    }
}
