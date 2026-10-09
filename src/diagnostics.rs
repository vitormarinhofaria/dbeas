// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Diagnóstico de sintaxe com o parser real do PostgreSQL (libpg_query), sem
//! precisar do servidor (spec §7.1–7.2).
//!
//! O wrapper `pg_query::parse` descarta a posição do erro; por isso chamamos
//! diretamente `pg_query_parse` da libpg_query, que o crate `pg_query` liga
//! estaticamente, e lemos `cursorpos`. A análise é só sintática: nomes de
//! tabelas e colunas inexistentes só aparecem ao executar.

use std::ffi::{CStr, CString, c_char, c_int};

use crate::query::split;

// Garante que a biblioteca estática do crate seja ligada.
use pg_query as _;

#[repr(C)]
struct PgQueryError {
    message: *mut c_char,
    funcname: *mut c_char,
    filename: *mut c_char,
    lineno: c_int,
    cursorpos: c_int,
    context: *mut c_char,
}

#[repr(C)]
struct PgQueryParseResult {
    parse_tree: *mut c_char,
    stderr_buffer: *mut c_char,
    error: *mut PgQueryError,
}

unsafe extern "C" {
    fn pg_query_parse(input: *const c_char) -> PgQueryParseResult;
    fn pg_query_free_parse_result(result: PgQueryParseResult);
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Deslocamento em bytes no texto analisado.
    pub offset: usize,
    /// Linha e coluna a partir de 1 (coluna em caracteres).
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Erro de sintaxe de uma instrução: (posição em caracteres a partir de 1, se
/// houver; mensagem).
fn parse_error(sql: &str) -> Option<(Option<usize>, String)> {
    let input = CString::new(sql).ok()?;
    // SAFETY: `input` é uma string C válida durante a chamada; o resultado é
    // lido antes de ser liberado por `pg_query_free_parse_result`.
    unsafe {
        let result = pg_query_parse(input.as_ptr());
        let outcome = if result.error.is_null() {
            None
        } else {
            let error = &*result.error;
            let message = if error.message.is_null() {
                "erro de sintaxe".to_string()
            } else {
                CStr::from_ptr(error.message).to_string_lossy().into_owned()
            };
            let position = usize::try_from(error.cursorpos).ok().filter(|p| *p > 0);
            Some((position, message))
        };
        pg_query_free_parse_result(result);
        outcome
    }
}

/// Analisa cada instrução do texto e retorna os erros de sintaxe com posição
/// no documento.
pub fn check(text: &str) -> Vec<Diagnostic> {
    split::statements(text)
        .into_iter()
        .filter_map(|range| {
            let statement = &text[range.clone()];
            let (position, message) = parse_error(statement)?;
            let offset = match position {
                Some(p) => range.start + split::char_pos_to_byte(statement, p),
                None => range.start,
            };
            let (line, column) = split::line_col(text, offset);
            Some(Diagnostic {
                offset,
                line,
                column,
                message,
            })
        })
        .collect()
}

/// Trecho a sublinhar para um erro em `offset`: o token que começa ali
/// (palavra, número, string entre aspas ou um símbolo). Se a posição estiver no
/// fim do texto ou em espaço (ex.: "syntax error at end of input"), usa o
/// último token antes dela.
pub fn error_span(text: &str, offset: usize) -> std::ops::Range<usize> {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';

    let at = text[offset..].chars().next();
    if at.is_none_or(char::is_whitespace) {
        // Último caractere significativo antes da posição.
        let Some((start, c)) = text[..offset]
            .char_indices()
            .rev()
            .find(|(_, c)| !c.is_whitespace())
        else {
            return offset..offset;
        };
        if is_word(c) {
            let word_start = text[..start]
                .char_indices()
                .rev()
                .take_while(|(_, c)| is_word(*c))
                .last()
                .map_or(start, |(i, _)| i);
            return word_start..start + c.len_utf8();
        }
        return start..start + c.len_utf8();
    }

    let c = at.expect("caractere na posição");
    let end = if is_word(c) {
        offset
            + text[offset..]
                .char_indices()
                .find(|(_, c)| !is_word(*c))
                .map_or(text.len() - offset, |(i, _)| i)
    } else if c == '\'' || c == '"' {
        // String/identificador entre aspas, até a aspa de fechamento (ou fim da linha).
        let rest = &text[offset + 1..];
        let close = rest.find(c).map(|p| p + 2);
        let line_end = rest.find('\n').map(|p| p + 1);
        offset + close.or(line_end).unwrap_or(text.len() - offset)
    } else {
        offset + c.len_utf8()
    };
    offset..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_sql_has_no_diagnostics() {
        assert!(check("select 1; select a from t where b = $1; create table x (a int)").is_empty());
    }

    #[test]
    fn reports_position_in_document() {
        let text = "select 1;\nselect * fromm t;";
        let d = check(text);
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].line, d[0].column), (2, 10));
        assert_eq!(&text[d[0].offset..d[0].offset + 5], "fromm");
        assert!(d[0].message.contains("syntax error"), "{}", d[0].message);
    }

    #[test]
    fn positions_count_characters() {
        // `frm` vira alias; o erro está em `x`, depois de caracteres multibyte.
        let text = "select 'ação' frm x";
        let d = check(text);
        assert_eq!(d.len(), 1);
        assert_eq!(&text[d[0].offset..], "x");
        assert_eq!(d[0].column, 19);
    }

    #[test]
    fn error_spans() {
        let text = "select * fromm t";
        assert_eq!(&text[error_span(text, 9)], "fromm");
        let text = "select (1";
        assert_eq!(&text[error_span(text, text.len())], "1");
        let text = "select 1 +  ";
        assert_eq!(&text[error_span(text, text.len())], "+");
        let text = "select 'abc' x";
        assert_eq!(&text[error_span(text, 7)], "'abc'");
        let text = "select ação, 1";
        assert_eq!(&text[error_span(text, 7)], "ação");
        assert_eq!(&text[error_span(text, text.find(',').unwrap())], ",");
        assert_eq!(error_span("", 0), 0..0);
    }

    #[test]
    fn unterminated_input_is_reported() {
        let d = check("select (1");
        assert_eq!(d.len(), 1);
    }
}
