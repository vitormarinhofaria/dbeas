// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Execução SQL: resultados, erros com posição e utilitários de texto SQL.

pub mod classify;
pub mod split;
pub mod transform;
pub mod value;

use std::ops::Range;

pub use value::Value;

/// Quantidade de linhas buscadas por vez.
pub const PAGE_SIZE: usize = 500;
/// Limite de linhas mantidas em memória por resultado (spec §10.2).
pub const MAX_ROWS: usize = 20_000;

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    /// Tabela e coluna de origem, quando o PostgreSQL as informa. Base para
    /// decidir, futuramente, se o resultado é editável.
    pub table_oid: Option<u32>,
    pub column_id: Option<i16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Value>>,
    /// Há mais linhas disponíveis no cursor aberto.
    pub has_more: bool,
    /// O limite de memória foi atingido; linhas restantes foram descartadas.
    pub truncated: bool,
}

/// Resultado de uma execução (uma ou mais instruções).
#[derive(Debug, Clone)]
pub struct Outcome {
    /// Último conjunto de linhas retornado, se houver.
    pub result: Option<ResultSet>,
    /// Faixa, no SQL enviado, da instrução que produziu `result`.
    pub result_statement: Option<Range<usize>>,
    /// Identificador do cursor aberto para buscar mais linhas.
    pub cursor: Option<u64>,
    /// Mensagens de comandos (`UPDATE 3`, etc.).
    pub messages: Vec<String>,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryError {
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    pub code: Option<String>,
    /// Deslocamento em bytes do erro dentro do SQL enviado, se o servidor
    /// informou a posição.
    pub offset: Option<usize>,
    /// Índice da instrução que falhou e sua faixa no texto enviado.
    pub statement: Option<Range<usize>>,
}

impl QueryError {
    pub fn simple(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            detail: None,
            hint: None,
            code: None,
            offset: None,
            statement: None,
        }
    }

    pub fn from_pg(error: &tokio_postgres::Error, sql: &str) -> Self {
        match error.as_db_error() {
            Some(db) => {
                let offset = match db.position() {
                    Some(tokio_postgres::error::ErrorPosition::Original(pos)) => {
                        Some(split::char_pos_to_byte(sql, *pos as usize))
                    }
                    _ => None,
                };
                Self {
                    message: db.message().to_string(),
                    detail: db.detail().map(str::to_string),
                    hint: db.hint().map(str::to_string),
                    code: Some(db.code().code().to_string()),
                    offset,
                    statement: None,
                }
            }
            None => Self::simple(error.to_string()),
        }
    }
}
