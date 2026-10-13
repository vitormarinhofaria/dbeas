// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Classificação conservadora de SQL para o modo `readOnly` (spec §6.3).
//!
//! É a primeira camada de proteção. A segunda é a sessão PostgreSQL com
//! `default_transaction_read_only = on`, configurada em `connection`.

use sqlparser::ast::{Query, SetExpr, Statement};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

/// `Ok(())` se a instrução é reconhecidamente somente leitura; `Err(motivo)`
/// caso contrário. SQL que não pode ser analisado é tratado como escrita.
pub fn check_read_only(sql: &str) -> Result<(), String> {
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, sql).map_err(|e| {
        format!("não foi possível analisar a instrução para confirmar que é somente leitura ({e})")
    })?;

    match statements.as_slice() {
        [] => Ok(()),
        [statement] => check_statement(statement),
        _ => Err(
            "múltiplas instruções em uma única execução não são permitidas no modo somente leitura"
                .into(),
        ),
    }
}

fn check_statement(statement: &Statement) -> Result<(), String> {
    match statement {
        Statement::Query(query) => check_query(query),
        Statement::Explain {
            analyze,
            statement,
            options,
            ..
        } => {
            let analyze_option = options
                .iter()
                .flatten()
                .any(|o| o.name.value.eq_ignore_ascii_case("analyze"));
            if *analyze || analyze_option {
                // EXPLAIN ANALYZE executa a instrução.
                check_statement(statement)
            } else {
                Ok(())
            }
        }
        Statement::ShowVariable { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowTables { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowFunctions { .. } => Ok(()),
        other => Err(format!(
            "`{}` não é uma instrução de leitura",
            first_words(&other.to_string())
        )),
    }
}

fn check_query(query: &Query) -> Result<(), String> {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            check_query(&cte.query)?;
        }
    }
    if !query.locks.is_empty() {
        return Err(
            "SELECT ... FOR UPDATE/SHARE bloqueia linhas e não é permitido no modo somente leitura"
                .into(),
        );
    }
    check_set_expr(&query.body)
}

fn check_set_expr(expr: &SetExpr) -> Result<(), String> {
    match expr {
        SetExpr::Select(select) => {
            if select.into.is_some() {
                Err("SELECT ... INTO cria uma tabela".into())
            } else {
                Ok(())
            }
        }
        SetExpr::Query(q) => check_query(q),
        SetExpr::SetOperation { left, right, .. } => {
            check_set_expr(left)?;
            check_set_expr(right)
        }
        SetExpr::Values(_) | SetExpr::Table(_) => Ok(()),
        SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_) => {
            Err("a consulta contém uma instrução de modificação de dados".into())
        }
        #[allow(unreachable_patterns)]
        _ => Err("construção de consulta não reconhecida".into()),
    }
}

fn first_words(s: &str) -> String {
    s.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
}

/// Uma única consulta `SELECT` (ou `WITH`/`VALUES`/`TABLE`) somente leitura.
/// Usado para decidir se o resultado de uma aba restaurada pode ser refeito
/// sozinho: `EXPLAIN`, `SHOW`, escritas e scripts com várias instruções não.
pub fn is_select(sql: &str) -> bool {
    match Parser::parse_sql(&PostgreSqlDialect {}, sql).as_deref() {
        Ok([Statement::Query(query)]) => check_query(query).is_ok(),
        _ => false,
    }
}

/// Comando de controle de transação no início de uma instrução.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxControl {
    /// `BEGIN`, `START TRANSACTION`.
    Begin,
    /// `COMMIT`, `END`, `COMMIT AND CHAIN` etc. Num bloco com erro, o
    /// PostgreSQL transforma o COMMIT em ROLLBACK; o efeito é encerrar o bloco.
    Commit,
    /// `ROLLBACK`, `ABORT` (sem `TO SAVEPOINT`).
    Rollback,
    /// `ROLLBACK TO [SAVEPOINT]`: desfaz até o savepoint e tira a transação do
    /// estado de erro.
    RollbackToSavepoint,
    /// `SAVEPOINT`, `RELEASE`, `PREPARE TRANSACTION`, `COMMIT/ROLLBACK PREPARED`,
    /// `SET TRANSACTION`.
    Other,
}

/// Classificação léxica pelas primeiras palavras, depois de espaços e
/// comentários. Não depende do parser, para funcionar também com SQL que ele
/// não entende.
pub fn transaction_control(sql: &str) -> Option<TxControl> {
    let words = leading_words(sql, 3);
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    match w.as_slice() {
        ["begin", ..] | ["start", "transaction", ..] => Some(TxControl::Begin),
        ["commit", "prepared", ..] | ["rollback", "prepared", ..] => Some(TxControl::Other),
        ["commit", ..] | ["end", ..] => Some(TxControl::Commit),
        ["rollback" | "abort", "to", ..]
        | ["rollback" | "abort", "work" | "transaction", "to", ..] => {
            Some(TxControl::RollbackToSavepoint)
        }
        ["rollback" | "abort", ..] => Some(TxControl::Rollback),
        ["savepoint", ..] | ["release", ..] | ["prepare", "transaction", ..] => {
            Some(TxControl::Other)
        }
        ["set", "transaction", ..] | ["set", "session", "characteristics", ..] => {
            Some(TxControl::Other)
        }
        _ => None,
    }
}

fn leading_words(sql: &str, count: usize) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut words = Vec::new();
    let mut i = 0;
    while i < bytes.len() && words.len() < count {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(bytes.len(), |p| i + p + 1);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |p| i + 2 + p + 2);
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                words.push(sql[start..i].to_ascii_lowercase());
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => break,
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_plain_select() {
        assert!(is_select("select a from t order by a"));
        assert!(is_select("with x as (select 1) select * from x;"));
        assert!(!is_select("update t set a = 1"));
        assert!(!is_select("insert into t values (1)"));
        assert!(!is_select(
            "with x as (delete from t returning *) select * from x"
        ));
        assert!(!is_select("select * from t for update"));
        assert!(!is_select("select 1; select 2"));
        assert!(!is_select("explain select 1"));
        assert!(!is_select("selec"));
    }

    #[test]
    fn detects_transaction_control() {
        assert_eq!(
            transaction_control("  -- c\n BEGIN;"),
            Some(TxControl::Begin)
        );
        assert_eq!(
            transaction_control("start transaction isolation level serializable"),
            Some(TxControl::Begin)
        );
        assert_eq!(
            transaction_control("/* x */ commit"),
            Some(TxControl::Commit)
        );
        assert_eq!(transaction_control("END"), Some(TxControl::Commit));
        assert_eq!(transaction_control("rollback"), Some(TxControl::Rollback));
        assert_eq!(
            transaction_control("ROLLBACK TO SAVEPOINT a"),
            Some(TxControl::RollbackToSavepoint)
        );
        assert_eq!(
            transaction_control("rollback work to a"),
            Some(TxControl::RollbackToSavepoint)
        );
        assert_eq!(transaction_control("savepoint a"), Some(TxControl::Other));
        assert_eq!(
            transaction_control("commit prepared 'x'"),
            Some(TxControl::Other)
        );
        assert_eq!(transaction_control("select 'begin'"), None);
        assert_eq!(transaction_control("update t set commit = 1"), None);
    }

    #[test]
    fn allows_reads() {
        for sql in [
            "select 1",
            "SELECT a FROM t WHERE b = 1 ORDER BY a",
            "with x as (select 1) select * from x",
            "select 1 union select 2",
            "values (1)",
            "explain select * from t",
            "show search_path",
        ] {
            assert!(check_read_only(sql).is_ok(), "{sql}");
        }
    }

    #[test]
    fn blocks_writes_and_unknowns() {
        for sql in [
            "insert into t values (1)",
            "update t set a = 1",
            "delete from t",
            "drop table t",
            "create table x (a int)",
            "with d as (delete from t returning *) select * from d",
            "select * into novo from t",
            "select * from t for update",
            "explain analyze delete from t",
            "select 1; delete from t",
            "set role admin",
            "isto não é sql",
        ] {
            assert!(check_read_only(sql).is_err(), "{sql}");
        }
    }
}
