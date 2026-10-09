// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Valores tipados de resultados (spec §5.1).
//!
//! O tokio-postgres recebe resultados em formato binário. Cada valor é
//! decodificado para uma variante tipada; tipos sem decodificador ficam em
//! `Other` com os bytes originais, para que renderizadores futuros possam
//! tratá-los sem perda.

use std::fmt::Write as _;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use fallible_iterator::FallibleIterator;
use postgres_protocol::types as pg;
use tokio_postgres::types::{FromSql, Kind, Type};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// `numeric` preservado como texto decimal exato.
    Numeric(String),
    Text(String),
    Json(String),
    Bytes(Vec<u8>),
    Uuid([u8; 16]),
    Date(NaiveDate),
    Time(NaiveTime),
    Timestamp(NaiveDateTime),
    TimestampTz(DateTime<Utc>),
    Interval {
        months: i32,
        days: i32,
        micros: i64,
    },
    /// Valores especiais (`infinity`, `NaN`) e tipos de rede, já em texto.
    Special(String),
    Array {
        dims: Vec<usize>,
        elements: Vec<Value>,
    },
    Other {
        type_name: String,
        raw: Vec<u8>,
    },
}

/// Captura os bytes brutos de qualquer coluna, nula ou não.
pub struct Raw<'a>(pub Option<&'a [u8]>);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(Some(raw)))
    }

    fn from_sql_null(_: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(None))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

fn pg_epoch_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2000, 1, 1).expect("data válida")
}

pub fn decode(ty: &Type, raw: Option<&[u8]>) -> Value {
    let Some(raw) = raw else { return Value::Null };
    decode_bytes(ty, raw).unwrap_or_else(|| Value::Other {
        type_name: ty.name().to_string(),
        raw: raw.to_vec(),
    })
}

fn decode_bytes(ty: &Type, raw: &[u8]) -> Option<Value> {
    match ty.kind() {
        Kind::Enum(_) => {
            return pg::text_from_sql(raw)
                .ok()
                .map(|s| Value::Text(s.to_string()));
        }
        Kind::Domain(base) => return Some(decode(base, Some(raw))),
        Kind::Array(member) => return decode_array(member, raw),
        _ => {}
    }

    let value = match *ty {
        Type::BOOL => Value::Bool(pg::bool_from_sql(raw).ok()?),
        Type::INT2 => Value::Int(pg::int2_from_sql(raw).ok()?.into()),
        Type::INT4 => Value::Int(pg::int4_from_sql(raw).ok()?.into()),
        Type::INT8 => Value::Int(pg::int8_from_sql(raw).ok()?),
        Type::OID => Value::Int(pg::oid_from_sql(raw).ok()?.into()),
        Type::FLOAT4 => Value::Float(pg::float4_from_sql(raw).ok()?.into()),
        Type::FLOAT8 => Value::Float(pg::float8_from_sql(raw).ok()?),
        Type::NUMERIC => decode_numeric(raw)?,
        Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::UNKNOWN
        | Type::CHAR
        | Type::XML => Value::Text(pg::text_from_sql(raw).ok()?.to_string()),
        Type::JSON => Value::Json(pg::text_from_sql(raw).ok()?.to_string()),
        Type::JSONB => {
            let (version, rest) = raw.split_first()?;
            if *version != 1 {
                return None;
            }
            Value::Json(std::str::from_utf8(rest).ok()?.to_string())
        }
        Type::BYTEA => Value::Bytes(pg::bytea_from_sql(raw).to_vec()),
        Type::UUID => Value::Uuid(pg::uuid_from_sql(raw).ok()?),
        Type::DATE => {
            let days = pg::date_from_sql(raw).ok()?;
            match days {
                i32::MAX => Value::Special("infinity".into()),
                i32::MIN => Value::Special("-infinity".into()),
                d => Value::Date(pg_epoch_date().checked_add_signed(Duration::days(d.into()))?),
            }
        }
        Type::TIME => {
            let micros = pg::time_from_sql(raw).ok()?;
            Value::Time(NaiveTime::MIN + Duration::microseconds(micros))
        }
        Type::TIMESTAMP | Type::TIMESTAMPTZ => {
            let micros = pg::timestamp_from_sql(raw).ok()?;
            match micros {
                i64::MAX => Value::Special("infinity".into()),
                i64::MIN => Value::Special("-infinity".into()),
                m => {
                    let naive = pg_epoch_date()
                        .and_time(NaiveTime::MIN)
                        .checked_add_signed(Duration::microseconds(m))?;
                    if *ty == Type::TIMESTAMPTZ {
                        Value::TimestampTz(naive.and_utc())
                    } else {
                        Value::Timestamp(naive)
                    }
                }
            }
        }
        Type::INTERVAL => {
            if raw.len() != 16 {
                return None;
            }
            let micros = i64::from_be_bytes(raw[0..8].try_into().ok()?);
            let days = i32::from_be_bytes(raw[8..12].try_into().ok()?);
            let months = i32::from_be_bytes(raw[12..16].try_into().ok()?);
            Value::Interval {
                months,
                days,
                micros,
            }
        }
        Type::INET | Type::CIDR => {
            let inet = pg::inet_from_sql(raw).ok()?;
            let addr = inet.addr();
            let full = if addr.is_ipv4() { 32 } else { 128 };
            if *ty == Type::INET && inet.netmask() == full {
                Value::Special(addr.to_string())
            } else {
                Value::Special(format!("{addr}/{}", inet.netmask()))
            }
        }
        _ if ty.name() == "citext" => Value::Text(pg::text_from_sql(raw).ok()?.to_string()),
        _ => return None,
    };
    Some(value)
}

fn decode_array(member: &Type, raw: &[u8]) -> Option<Value> {
    let array = pg::array_from_sql(raw).ok()?;
    let dims: Vec<usize> = array
        .dimensions()
        .map(|d| Ok(usize::try_from(d.len).unwrap_or(0)))
        .collect()
        .ok()?;
    let elements: Vec<Value> = array
        .values()
        .map(|v| Ok(decode(member, v)))
        .collect()
        .ok()?;
    Some(Value::Array { dims, elements })
}

/// Formato binário de `numeric`: ndigits, weight, sign, dscale e dígitos em
/// base 10000.
fn decode_numeric(raw: &[u8]) -> Option<Value> {
    if raw.len() < 8 {
        return None;
    }
    let read_i16 = |i: usize| i16::from_be_bytes([raw[i], raw[i + 1]]);
    let ndigits = usize::try_from(read_i16(0)).ok()?;
    let weight = i32::from(read_i16(2));
    let sign = u16::from_be_bytes([raw[4], raw[5]]);
    let dscale = usize::from(u16::from_be_bytes([raw[6], raw[7]]));
    if raw.len() < 8 + ndigits * 2 {
        return None;
    }
    match sign {
        0xC000 => return Some(Value::Special("NaN".into())),
        0xD000 => return Some(Value::Special("Infinity".into())),
        0xF000 => return Some(Value::Special("-Infinity".into())),
        0x0000 | 0x4000 => {}
        _ => return None,
    }
    let digits: Vec<i16> = (0..ndigits).map(|i| read_i16(8 + i * 2)).collect();
    let digit = |i: i32| -> i16 {
        usize::try_from(i)
            .ok()
            .and_then(|i| digits.get(i).copied())
            .unwrap_or(0)
    };

    let mut out = String::new();
    if sign == 0x4000 {
        out.push('-');
    }
    if weight < 0 {
        out.push('0');
    } else {
        for i in 0..=weight {
            if i == 0 {
                write!(out, "{}", digit(i)).ok()?;
            } else {
                write!(out, "{:04}", digit(i)).ok()?;
            }
        }
    }
    if dscale > 0 {
        out.push('.');
        let mut frac = String::new();
        let mut i = weight + 1;
        while frac.len() < dscale {
            write!(frac, "{:04}", digit(i)).ok()?;
            i += 1;
        }
        frac.truncate(dscale);
        out.push_str(&frac);
    }
    Some(Value::Numeric(out))
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Representação textual bruta, próxima da saída do `psql`. `NULL` é
    /// distinguido pela grade, não por este texto.
    pub fn display(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Numeric(s) | Value::Text(s) | Value::Json(s) | Value::Special(s) => s.clone(),
            Value::Bytes(b) => hex(b),
            Value::Uuid(u) => format_uuid(u),
            Value::Date(d) => d.to_string(),
            Value::Time(t) => t.format("%H:%M:%S%.f").to_string(),
            Value::Timestamp(t) => t.format("%Y-%m-%d %H:%M:%S%.f").to_string(),
            Value::TimestampTz(t) => t.format("%Y-%m-%d %H:%M:%S%.f+00").to_string(),
            Value::Interval {
                months,
                days,
                micros,
            } => format_interval(*months, *days, *micros),
            Value::Array { dims, elements } => format_array(dims, elements),
            Value::Other { type_name, raw } => format!("<{type_name}: {} bytes>", raw.len()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("\\x");
    for b in bytes {
        write!(s, "{b:02x}").expect("escrita em String");
    }
    s
}

fn format_uuid(u: &[u8; 16]) -> String {
    let h = hex(u);
    let h = &h[2..];
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn format_interval(months: i32, days: i32, micros: i64) -> String {
    let mut parts = Vec::new();
    let (years, months) = (months / 12, months % 12);
    let plural =
        |n: i64, one: &str, many: &str| format!("{n} {}", if n.abs() == 1 { one } else { many });
    if years != 0 {
        parts.push(plural(years.into(), "year", "years"));
    }
    if months != 0 {
        parts.push(plural(months.into(), "mon", "mons"));
    }
    if days != 0 {
        parts.push(plural(days.into(), "day", "days"));
    }
    if micros != 0 || parts.is_empty() {
        let sign = if micros < 0 { "-" } else { "" };
        let m = micros.unsigned_abs();
        let (h, rem) = (m / 3_600_000_000, m % 3_600_000_000);
        let (min, rem) = (rem / 60_000_000, rem % 60_000_000);
        let (s, us) = (rem / 1_000_000, rem % 1_000_000);
        let mut t = format!("{sign}{h:02}:{min:02}:{s:02}");
        if us != 0 {
            let frac = format!("{us:06}");
            t.push('.');
            t.push_str(frac.trim_end_matches('0'));
        }
        parts.push(t);
    }
    parts.join(" ")
}

fn format_array(dims: &[usize], elements: &[Value]) -> String {
    fn element(v: &Value) -> String {
        match v {
            Value::Null => "NULL".into(),
            Value::Text(s) | Value::Json(s) => {
                if s.is_empty()
                    || s.contains([',', '{', '}', '"', '\\', ' '])
                    || s.eq_ignore_ascii_case("null")
                {
                    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
                } else {
                    s.clone()
                }
            }
            other => other.display(),
        }
    }
    fn nest(dims: &[usize], elements: &[Value]) -> String {
        match dims {
            [] | [_] => format!(
                "{{{}}}",
                elements.iter().map(element).collect::<Vec<_>>().join(",")
            ),
            [first, rest @ ..] => {
                let chunk = rest.iter().product::<usize>().max(1);
                let inner: Vec<String> = elements
                    .chunks(chunk)
                    .take(*first)
                    .map(|c| nest(rest, c))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
        }
    }
    nest(dims, elements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numeric(ndigits: i16, weight: i16, sign: u16, dscale: u16, digits: &[i16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(ndigits.to_be_bytes());
        v.extend(weight.to_be_bytes());
        v.extend(sign.to_be_bytes());
        v.extend(dscale.to_be_bytes());
        for d in digits {
            v.extend(d.to_be_bytes());
        }
        v
    }

    #[test]
    fn decodes_numeric() {
        // 12345.678 => weight 1, dígitos [1, 2345, 6780], dscale 3
        let raw = numeric(3, 1, 0, 3, &[1, 2345, 6780]);
        assert_eq!(
            decode(&Type::NUMERIC, Some(&raw)),
            Value::Numeric("12345.678".into())
        );
        // -0.05 => weight -1, dígitos [500], dscale 2
        let raw = numeric(1, -1, 0x4000, 2, &[500]);
        assert_eq!(
            decode(&Type::NUMERIC, Some(&raw)),
            Value::Numeric("-0.05".into())
        );
        // 10000 => weight 1, dígitos [1], dscale 0
        let raw = numeric(1, 1, 0, 0, &[1]);
        assert_eq!(
            decode(&Type::NUMERIC, Some(&raw)),
            Value::Numeric("10000".into())
        );
        let raw = numeric(0, 0, 0xC000, 0, &[]);
        assert_eq!(
            decode(&Type::NUMERIC, Some(&raw)),
            Value::Special("NaN".into())
        );
    }

    #[test]
    fn null_is_distinct_from_empty_text() {
        assert_eq!(decode(&Type::TEXT, None), Value::Null);
        assert_eq!(decode(&Type::TEXT, Some(b"")), Value::Text(String::new()));
    }

    #[test]
    fn decodes_dates_and_intervals() {
        let raw = 31i32.to_be_bytes();
        assert_eq!(decode(&Type::DATE, Some(&raw)).display(), "2000-02-01");
        assert_eq!(
            format_interval(14, 3, 3_723_500_000),
            "1 year 2 mons 3 days 01:02:03.5"
        );
        assert_eq!(format_interval(0, 0, 0), "00:00:00");
    }

    #[test]
    fn unknown_types_keep_raw_bytes() {
        let ty = Type::POINT;
        match decode(&ty, Some(&[1, 2, 3])) {
            Value::Other { type_name, raw } => {
                assert_eq!(type_name, "point");
                assert_eq!(raw, vec![1, 2, 3]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn formats_arrays() {
        let v = Value::Array {
            dims: vec![2, 2],
            elements: vec![Value::Int(1), Value::Int(2), Value::Null, Value::Int(4)],
        };
        assert_eq!(v.display(), "{{1,2},{NULL,4}}");
        let v = Value::Array {
            dims: vec![2],
            elements: vec![Value::Text("a b".into()), Value::Text("c".into())],
        };
        assert_eq!(v.display(), "{\"a b\",c}");
    }
}
