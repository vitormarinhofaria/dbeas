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

#[cfg(test)]
mod tests {
    use super::*;

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
