// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Transformações estruturais de consultas a partir da grade (spec §5.2–5.5):
//! ordenar, remover e reordenar itens do `SELECT`.
//!
//! O `sqlparser` valida a consulta e informa sua forma (uma instrução, um
//! `SELECT` simples, curingas, `DISTINCT`, ordinais em `ORDER BY`/`GROUP BY`). As
//! posições vêm do tokenizer, que cobre o texto inteiro, inclusive espaços e
//! comentários. Só os trechos alterados são reescritos; todo o resto do texto
//! original é preservado. Quando a transformação não puder ser feita sem
//! ambiguidade, a função retorna `Err(motivo)` e o SQL não muda.

use std::ops::Range;

use sqlparser::ast::{Expr, GroupByExpr, OrderByKind, SelectItem, SetExpr, Statement, Value};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::keywords::Keyword;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer};

/// Forma da consulta relevante para as transformações.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Faixa de cada item da lista do `SELECT` principal, sem espaços nas bordas.
    pub items: Vec<Range<usize>>,
    /// Alias explícito de cada item, se houver.
    pub aliases: Vec<Option<String>>,
    /// `ORDER BY` principal, da palavra `ORDER` até o fim da cláusula.
    pub order_by: Option<Range<usize>>,
    /// Onde inserir um `ORDER BY` quando não existe nenhum.
    pub order_insert: usize,
    /// O ponto de inserção é o fim da consulta (e não antes de `LIMIT` etc.).
    pub insert_at_end: bool,
    pub has_wildcard: bool,
    pub distinct: bool,
    pub set_operation: bool,
    /// `ORDER BY`/`GROUP BY` referenciam colunas por posição (`ORDER BY 2`).
    pub ordinal_refs: bool,
}

struct Tok {
    token: Token,
    range: Range<usize>,
}

pub fn analyze(sql: &str) -> Result<Shape, String> {
    let dialect = PostgreSqlDialect {};
    let statements = Parser::parse_sql(&dialect, sql)
        .map_err(|e| format!("não foi possível analisar o SQL ({e})"))?;
    let [Statement::Query(query)] = statements.as_slice() else {
        return Err("a consulta precisa ser uma única instrução SELECT".into());
    };

    let mut ordinal_refs = false;
    if let Some(order_by) = &query.order_by {
        if let OrderByKind::Expressions(exprs) = &order_by.kind {
            ordinal_refs |= exprs.iter().any(|e| is_number(&e.expr));
        }
    }

    let (projection_len, aliases, has_wildcard, distinct, set_operation) = match query.body.as_ref()
    {
        SetExpr::Select(select) => {
            if let GroupByExpr::Expressions(exprs, _) = &select.group_by {
                ordinal_refs |= exprs.iter().any(is_number);
            }
            let aliases = select
                .projection
                .iter()
                .map(|item| match item {
                    SelectItem::ExprWithAlias { alias, .. } => Some(alias.value.clone()),
                    _ => None,
                })
                .collect();
            let wildcard = select.projection.iter().any(|i| {
                matches!(
                    i,
                    SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..)
                )
            });
            (
                select.projection.len(),
                aliases,
                wildcard,
                select.distinct.is_some(),
                false,
            )
        }
        SetExpr::SetOperation { .. } => (0, Vec::new(), false, false, true),
        _ => return Err("só consultas SELECT podem ser transformadas".into()),
    };

    let toks = tokenize(sql)?;
    let significant = |t: &Tok| {
        !matches!(
            t.token,
            Token::Whitespace(_) | Token::SemiColon | Token::EOF
        )
    };
    let end = toks
        .iter()
        .filter(|t| significant(t))
        .map(|t| t.range.end)
        .last()
        .unwrap_or(0);

    // Profundidade de parênteses de cada token.
    let mut depth = 0i32;
    let depths: Vec<i32> = toks
        .iter()
        .map(|t| {
            let d = depth;
            match t.token {
                Token::LParen => depth += 1,
                Token::RParen => depth -= 1,
                _ => {}
            }
            d
        })
        .collect();
    let keyword_at = |i: usize, kw: Keyword| {
        depths[i] == 0 && matches!(&toks[i].token, Token::Word(w) if w.keyword == kw)
    };

    let select_index = (0..toks.len())
        .find(|&i| keyword_at(i, Keyword::SELECT))
        .ok_or("SELECT principal não encontrado")?;

    // Lista de itens do SELECT principal.
    let mut items = Vec::new();
    if !set_operation {
        let mut i = select_index + 1;
        // Pula DISTINCT [ON (...)] e ALL.
        loop {
            while i < toks.len() && !significant(&toks[i]) {
                i += 1;
            }
            if i < toks.len() && (keyword_at(i, Keyword::DISTINCT) || keyword_at(i, Keyword::ALL)) {
                i += 1;
            } else if i < toks.len() && keyword_at(i, Keyword::ON) {
                i += 1;
                while i < toks.len() && !significant(&toks[i]) {
                    i += 1;
                }
                if i < toks.len() && matches!(toks[i].token, Token::LParen) {
                    let level = depths[i];
                    i += 1;
                    while i < toks.len() && depths[i] > level {
                        i += 1;
                    }
                    i += 1;
                }
            } else {
                break;
            }
        }
        const ENDS: &[Keyword] = &[
            Keyword::FROM,
            Keyword::INTO,
            Keyword::WHERE,
            Keyword::GROUP,
            Keyword::HAVING,
            Keyword::WINDOW,
            Keyword::ORDER,
            Keyword::LIMIT,
            Keyword::OFFSET,
            Keyword::FETCH,
            Keyword::FOR,
        ];
        let mut current: Option<Range<usize>> = None;
        while i < toks.len() {
            let t = &toks[i];
            if depths[i] == 0
                && (ENDS.iter().any(|k| keyword_at(i, *k))
                    || matches!(t.token, Token::SemiColon | Token::EOF))
            {
                break;
            }
            if depths[i] == 0 && matches!(t.token, Token::Comma) {
                items.extend(current.take());
            } else if significant(t) {
                current = Some(match current {
                    Some(r) => r.start..t.range.end,
                    None => t.range.clone(),
                });
            }
            i += 1;
        }
        items.extend(current.take());
        if items.len() != projection_len {
            return Err("não foi possível localizar os itens do SELECT com segurança".into());
        }
    }

    // ORDER BY e cláusulas finais no nível principal.
    let mut order_by = None;
    let mut tail_clause = None;
    for i in select_index..toks.len() {
        if keyword_at(i, Keyword::ORDER) && order_by.is_none() {
            order_by = Some(toks[i].range.start);
        }
        if [
            Keyword::LIMIT,
            Keyword::OFFSET,
            Keyword::FETCH,
            Keyword::FOR,
        ]
        .iter()
        .any(|k| keyword_at(i, *k))
        {
            tail_clause = Some(toks[i].range.start);
            break;
        }
    }
    let clause_end = |start: usize, stop: usize| -> usize {
        // Fim do último token significativo antes de `stop`.
        toks.iter()
            .filter(|t| significant(t) && t.range.start >= start && t.range.end <= stop)
            .map(|t| t.range.end)
            .last()
            .unwrap_or(start)
    };
    let order_by = order_by.map(|start| start..clause_end(start, tail_clause.unwrap_or(sql.len())));

    Ok(Shape {
        items,
        aliases,
        order_by,
        order_insert: tail_clause.unwrap_or(end),
        insert_at_end: tail_clause.is_none(),
        has_wildcard,
        distinct,
        set_operation,
        ordinal_refs,
    })
}

fn is_number(expr: &Expr) -> bool {
    matches!(expr, Expr::Value(v) if matches!(v.value, Value::Number(..)))
}

fn tokenize(sql: &str) -> Result<Vec<Tok>, String> {
    let dialect = PostgreSqlDialect {};
    let tokens = Tokenizer::new(&dialect, sql)
        .tokenize_with_location()
        .map_err(|e| format!("não foi possível analisar o SQL ({e})"))?;

    // Início de cada linha em bytes, para converter (linha, coluna em caracteres).
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(sql.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let to_byte = |line: u64, column: u64| -> usize {
        let start = line_starts
            .get(line.saturating_sub(1) as usize)
            .copied()
            .unwrap_or(sql.len());
        sql[start..]
            .char_indices()
            .nth(column.saturating_sub(1) as usize)
            .map_or(sql.len(), |(b, _)| start + b)
    };

    let starts: Vec<usize> = tokens
        .iter()
        .map(|t| to_byte(t.span.start.line, t.span.start.column))
        .collect();
    // Os tokens cobrem o texto todo (inclusive espaços); cada um termina onde
    // o próximo começa.
    Ok(tokens
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let end = starts.get(i + 1).copied().unwrap_or(sql.len());
            Tok {
                token: t.token,
                range: starts[i]..end.max(starts[i]),
            }
        })
        .collect())
}

fn validate(sql: String) -> Result<String, String> {
    Parser::parse_sql(&PostgreSqlDialect {}, &sql)
        .map(|_| sql.clone())
        .map_err(|e| format!("a transformação produziria SQL inválido ({e})"))
}

/// Direção da ordenação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

/// Substitui o `ORDER BY` principal por `ORDER BY <expr> [DESC]`, ou o remove
/// quando `direction` é `None`.
pub fn set_order(sql: &str, expr: &str, direction: Option<Direction>) -> Result<String, String> {
    let shape = analyze(sql)?;
    let clause = direction.map(|d| match d {
        Direction::Asc => format!("ORDER BY {expr}"),
        Direction::Desc => format!("ORDER BY {expr} DESC"),
    });
    let multiline = sql.trim().contains('\n');
    let sep = if multiline { "\n" } else { " " };

    let out = match (&shape.order_by, clause) {
        (Some(range), Some(clause)) => {
            format!("{}{clause}{}", &sql[..range.start], &sql[range.end..])
        }
        (Some(range), None) => {
            // Remove também o espaço que precedia a cláusula.
            let before = sql[..range.start].trim_end();
            format!("{before}{}", &sql[range.end..])
        }
        (None, Some(clause)) if shape.insert_at_end => {
            let at = shape.order_insert;
            format!("{}{sep}{clause}{}", &sql[..at], &sql[at..])
        }
        (None, Some(clause)) => {
            let at = shape.order_insert;
            format!("{}{clause}{sep}{}", &sql[..at], &sql[at..])
        }
        (None, None) => return Ok(sql.to_string()),
    };
    validate(out)
}

fn check_projection_edit(shape: &Shape) -> Result<(), String> {
    if shape.set_operation {
        return Err("consultas com UNION/INTERSECT/EXCEPT têm mais de uma lista de colunas".into());
    }
    if shape.has_wildcard {
        return Err("a lista de colunas usa `*`, então não há uma expressão por coluna".into());
    }
    if shape.ordinal_refs {
        return Err("ORDER BY/GROUP BY referenciam colunas por posição, que mudaria".into());
    }
    Ok(())
}

/// Remove o item `index` da lista do `SELECT`.
pub fn remove_item(sql: &str, index: usize) -> Result<String, String> {
    let shape = analyze(sql)?;
    check_projection_edit(&shape)?;
    if shape.distinct {
        return Err("remover uma coluna de um SELECT DISTINCT mudaria as linhas retornadas".into());
    }
    if shape.items.len() <= 1 {
        return Err("a consulta precisa manter ao menos uma coluna".into());
    }
    let Some(item) = shape.items.get(index) else {
        return Err("coluna fora da lista do SELECT".into());
    };
    if let (Some(alias), Some(order)) = (&shape.aliases[index], &shape.order_by) {
        let referenced = tokenize(&sql[order.clone()])?
            .iter()
            .any(|t| matches!(&t.token, Token::Word(w) if w.value.eq_ignore_ascii_case(alias)));
        if referenced {
            return Err(format!("o ORDER BY usa o alias `{alias}` desta coluna"));
        }
    }

    let range = if index + 1 < shape.items.len() {
        item.start..shape.items[index + 1].start
    } else {
        shape.items[index - 1].end..item.end
    };
    validate(format!("{}{}", &sql[..range.start], &sql[range.end..]))
}

/// Reordena os itens do `SELECT`: `order[k]` é o índice original do item que
/// passa a ocupar a posição `k`. Separadores e espaços entre itens ficam onde
/// estavam.
pub fn reorder_items(sql: &str, order: &[usize]) -> Result<String, String> {
    let shape = analyze(sql)?;
    check_projection_edit(&shape)?;
    let n = shape.items.len();
    let mut seen = vec![false; n];
    if order.len() != n
        || order
            .iter()
            .any(|&i| i >= n || std::mem::replace(&mut seen[i], true))
    {
        return Err("a nova ordem não corresponde às colunas da consulta".into());
    }

    let mut out = String::with_capacity(sql.len());
    out.push_str(&sql[..shape.items[0].start]);
    for (slot, &source) in order.iter().enumerate() {
        out.push_str(&sql[shape.items[source].clone()]);
        let gap_end = shape
            .items
            .get(slot + 1)
            .map_or(shape.items[n - 1].end, |r| r.start);
        out.push_str(&sql[shape.items[slot].end..gap_end]);
    }
    out.push_str(&sql[shape.items[n - 1].end..]);
    validate(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_select_items() {
        let sql = "select a, upper(b) as \"B\", f(x, y) from t where z = (select 1 from u order by 1) limit 5";
        let s = analyze(sql).unwrap();
        let items: Vec<&str> = s.items.iter().map(|r| &sql[r.clone()]).collect();
        assert_eq!(items, ["a", "upper(b) as \"B\"", "f(x, y)"]);
        assert_eq!(s.aliases[1].as_deref(), Some("B"));
        assert!(s.order_by.is_none());
        assert_eq!(&sql[s.order_insert..], "limit 5");
    }

    #[test]
    fn adds_replaces_and_removes_order_by() {
        let sql = "SELECT id, nome\nFROM t -- fim";
        let asc = set_order(sql, "nome", Some(Direction::Asc)).unwrap();
        assert_eq!(asc, "SELECT id, nome\nFROM t\nORDER BY nome -- fim");
        let desc = set_order(&asc, "id", Some(Direction::Desc)).unwrap();
        assert_eq!(desc, "SELECT id, nome\nFROM t\nORDER BY id DESC -- fim");
        assert_eq!(
            set_order(&desc, "id", None).unwrap(),
            "SELECT id, nome\nFROM t -- fim"
        );
    }

    #[test]
    fn order_by_goes_before_limit_and_ignores_subqueries() {
        let sql = "select a, row_number() over (order by b) from t limit 10;";
        assert_eq!(
            set_order(sql, "a", Some(Direction::Desc)).unwrap(),
            "select a, row_number() over (order by b) from t ORDER BY a DESC limit 10;"
        );
        let sql = "select a from t order by a desc, b limit 3";
        assert_eq!(
            set_order(sql, "b", Some(Direction::Asc)).unwrap(),
            "select a from t ORDER BY b limit 3"
        );
    }

    #[test]
    fn sorting_works_with_wildcards_ctes_and_unions() {
        assert_eq!(
            set_order("select * from t", "\"x\"", Some(Direction::Asc)).unwrap(),
            "select * from t ORDER BY \"x\""
        );
        let cte = "with c as (select 1 as a order by 1) select a from c";
        assert_eq!(
            set_order(cte, "a", Some(Direction::Asc)).unwrap(),
            format!("{cte} ORDER BY a")
        );
        let union = "select 1 as a union select 2 order by a";
        assert_eq!(
            set_order(union, "a", Some(Direction::Desc)).unwrap(),
            "select 1 as a union select 2 ORDER BY a DESC"
        );
    }

    #[test]
    fn removes_items_preserving_the_rest() {
        let sql = "SELECT id,\n       nome, -- nome do cliente\n       valor\nFROM t";
        assert_eq!(
            remove_item(sql, 0).unwrap(),
            "SELECT nome, -- nome do cliente\n       valor\nFROM t"
        );
        assert_eq!(
            remove_item(sql, 2).unwrap(),
            "SELECT id,\n       nome\nFROM t"
        );
        assert_eq!(
            remove_item(sql, 1).unwrap(),
            "SELECT id,\n       valor\nFROM t"
        );
    }

    #[test]
    fn refuses_ambiguous_projection_edits() {
        assert!(remove_item("select * from t", 0).is_err());
        assert!(remove_item("select a, t.* from t", 0).is_err());
        assert!(remove_item("select distinct a, b from t", 0).is_err());
        assert!(remove_item("select a, b from t order by 2", 0).is_err());
        assert!(remove_item("select a, count(*) from t group by 1", 1).is_err());
        assert!(remove_item("select a as x, b from t order by x", 0).is_err());
        assert!(remove_item("select a from t", 0).is_err());
        assert!(remove_item("select 1 union select 2", 0).is_err());
        assert!(remove_item("update t set a = 1", 0).is_err());
        assert!(remove_item("select a; select b", 0).is_err());
    }

    #[test]
    fn reorders_items() {
        let sql = "SELECT id, upper(nome) AS n,\n  valor FROM t";
        assert_eq!(
            reorder_items(sql, &[2, 0, 1]).unwrap(),
            "SELECT valor, id,\n  upper(nome) AS n FROM t"
        );
        assert!(reorder_items(sql, &[0, 0, 1]).is_err());
        assert!(reorder_items("select * from t", &[0]).is_err());
    }

    #[test]
    fn handles_distinct_on_and_unicode() {
        let sql = "select distinct on (a) a, 'ação' as b, c from t";
        let s = analyze(sql).unwrap();
        let items: Vec<&str> = s.items.iter().map(|r| &sql[r.clone()]).collect();
        assert_eq!(items, ["a", "'ação' as b", "c"]);
        assert_eq!(
            reorder_items(sql, &[2, 1, 0]).unwrap(),
            "select distinct on (a) c, 'ação' as b, a from t"
        );
    }
}
