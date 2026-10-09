//! Divisão de scripts em instruções, respeitando strings, identificadores
//! entre aspas, dollar quoting e comentários do PostgreSQL.

use std::ops::Range;

/// Faixas de bytes de cada instrução não vazia (sem o `;` final e sem
/// espaços nas bordas).
pub fn statements(sql: &str) -> Vec<Range<usize>> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b';' => {
                push_trimmed(sql, start..i, &mut out);
                i += 1;
                start = i;
            }
            b'\'' => {
                // E'..' aceita escapes com barra invertida.
                let escape = i > 0
                    && matches!(bytes[i - 1], b'e' | b'E')
                    && !is_ident_char_before(bytes, i - 1);
                i = skip_quoted(bytes, i, b'\'', escape);
            }
            b'"' => i = skip_quoted(bytes, i, b'"', false),
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = memchr(bytes, i, b'\n').map_or(bytes.len(), |p| p + 1);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'$' => i = skip_dollar_quoted(sql, i),
            _ => i += 1,
        }
    }
    push_trimmed(sql, start..sql.len(), &mut out);
    out
}

/// Instrução que contém (ou termina imediatamente antes de) `offset`.
pub fn statement_at(sql: &str, offset: usize) -> Option<Range<usize>> {
    let all = statements(sql);
    all.iter()
        .find(|r| r.start <= offset && offset <= r.end)
        .or_else(|| all.iter().rev().find(|r| r.end <= offset))
        .or_else(|| all.first())
        .cloned()
}

fn push_trimmed(sql: &str, range: Range<usize>, out: &mut Vec<Range<usize>>) {
    let slice = &sql[range.clone()];
    let lead = slice.len() - slice.trim_start().len();
    let trail = slice.len() - slice.trim_end().len();
    let r = range.start + lead..range.end - trail;
    if r.start < r.end && has_code(&sql[r.clone()]) {
        out.push(r);
    }
}

/// `true` se o trecho tiver algo além de comentários.
fn has_code(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = memchr(bytes, i, b'\n').map_or(bytes.len(), |p| p + 1);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            c if c.is_ascii_whitespace() => i += 1,
            _ => return true,
        }
    }
    false
}

fn is_ident_char_before(bytes: &[u8], i: usize) -> bool {
    i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')
}

fn memchr(bytes: &[u8], from: usize, needle: u8) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|&b| b == needle)
        .map(|p| p + from)
}

fn skip_quoted(bytes: &[u8], start: usize, quote: u8, backslash_escapes: bool) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if backslash_escapes && bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// `$tag$ ... $tag$`. Um `$` que não inicia uma tag válida (ex.: `$1`) é
/// tratado como caractere comum.
fn skip_dollar_quoted(sql: &str, start: usize) -> usize {
    let bytes = sql.as_bytes();
    if is_ident_char_before(bytes, start) {
        return start + 1;
    }
    let mut j = start + 1;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    if j >= bytes.len() || bytes[j] != b'$' || (j > start + 1 && bytes[start + 1].is_ascii_digit())
    {
        return start + 1;
    }
    let tag = &sql[start..=j];
    match sql[j + 1..].find(tag) {
        Some(p) => j + 1 + p + tag.len(),
        None => bytes.len(),
    }
}

/// Converte um deslocamento em bytes para (linha, coluna), ambos a partir de 1.
/// A coluna conta caracteres, não bytes.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |p| p + 1);
    let col = text[line_start..offset].chars().count() + 1;
    (line, col)
}

/// Converte uma posição em caracteres (1-based, como o PostgreSQL informa)
/// para deslocamento em bytes.
pub fn char_pos_to_byte(text: &str, char_pos: usize) -> usize {
    text.char_indices()
        .nth(char_pos.saturating_sub(1))
        .map_or(text.len(), |(b, _)| b)
}

/// Converte (linha, coluna) do editor, ambos a partir de 0 e com a coluna em
/// bytes dentro da linha, para deslocamento em bytes no texto.
pub fn byte_offset(text: &str, line: usize, column: usize) -> usize {
    let mut offset = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i == line {
            let content = l.trim_end_matches('\n').trim_end_matches('\r');
            return offset + column.min(content.len());
        }
        offset += l.len();
    }
    text.len()
}

/// Inverso de [`byte_offset`]: (linha, coluna em bytes), a partir de 0.
pub fn line_and_byte_column(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |p| p + 1);
    (line, offset - line_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str) -> Vec<&str> {
        statements(sql).into_iter().map(|r| &sql[r]).collect()
    }

    #[test]
    fn splits_on_semicolons() {
        assert_eq!(texts("select 1; select 2 ;\n\n"), ["select 1", "select 2"]);
    }

    #[test]
    fn ignores_semicolons_in_strings_and_comments() {
        let sql = "select ';' as a, \"x;y\" -- c;\n from t; /* a; /* b; */ */ select E'\\';' ; select 'it''s;'";
        assert_eq!(
            texts(sql),
            [
                "select ';' as a, \"x;y\" -- c;\n from t",
                "/* a; /* b; */ */ select E'\\';'",
                "select 'it''s;'"
            ]
        );
    }

    #[test]
    fn handles_dollar_quoting() {
        let sql = "create function f() returns int as $body$ select 1; $body$ language sql; select $1, $$;$$";
        assert_eq!(texts(sql).len(), 2);
    }

    #[test]
    fn comment_only_chunks_are_dropped() {
        assert_eq!(texts("select 1; -- fim\n"), ["select 1"]);
    }

    #[test]
    fn statement_under_cursor() {
        let sql = "select 1;\nselect 2;";
        assert_eq!(&sql[statement_at(sql, 12).unwrap()], "select 2");
        assert_eq!(&sql[statement_at(sql, 8).unwrap()], "select 1");
        assert_eq!(&sql[statement_at(sql, 9).unwrap()], "select 1");
    }

    #[test]
    fn positions() {
        let text = "ab\ncãd";
        assert_eq!(line_col(text, 0), (1, 1));
        assert_eq!(line_col(text, 3), (2, 1));
        assert_eq!(line_col(text, text.find('d').unwrap()), (2, 3));
        assert_eq!(char_pos_to_byte(text, 6), text.find('d').unwrap());
        assert_eq!(byte_offset(text, 1, 3), text.find('d').unwrap());
        assert_eq!(line_and_byte_column(text, text.find('d').unwrap()), (1, 3));
    }
}
