//! Helpers for storing and reading serialized Rust objects in SQLite `doc` JSON columns.
//!
//! All queries use [`rusqlite::Connection::prepare_cached`] and report the table name and primary
//! key whenever JSON serialization or deserialization fails.

use std::fmt;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{DbError, Result};
use crate::util::hex_encode;

/// Serialize `value` into a compact JSON string suitable for storing in a `doc` column.
pub fn encode_doc<T: Serialize>(table: &str, key: impl fmt::Display, value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|source| DbError::DocSerialize {
        table: table.to_owned(),
        key: key.to_string(),
        source,
    })
}

/// Deserialize a `doc` JSON string into `T`, identifying `table` and `key` on failure.
pub fn decode_doc<T: DeserializeOwned>(
    table: &str,
    key: impl fmt::Display,
    raw: &str,
) -> Result<T> {
    serde_json::from_str(raw).map_err(|source| DbError::DocDeserialize {
        table: table.to_owned(),
        key: key.to_string(),
        source,
    })
}

/// Execute an `INSERT` / `UPDATE` statement whose final parameter is the serialized `doc` JSON of
/// `value` for a simple `(key, doc)` write helper.
///
/// For tables with additional indexed columns alongside `doc`, call [`encode_doc`] directly when
/// binding the insert parameters.
pub fn write_keyed_doc<T: Serialize>(
    conn: &Connection,
    table: &str,
    key: &str,
    sql: &str,
    value: &T,
) -> Result<()> {
    let doc = encode_doc(table, key, value)?;
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备写入表 {table} 的语句"), err))?;
    stmt.execute(rusqlite::params![key, doc])
        .map_err(|err| DbError::sqlite(format!("写入表 {table} 主键 {key}"), err))?;
    Ok(())
}

/// Fetch at most one `doc` column (column 0) via `prepare_cached` and deserialize it into `T`.
///
/// The caller supplies `key` for error reporting (typically the primary key bound in `params`).
pub fn query_doc_opt<T, P>(
    conn: &Connection,
    table: &str,
    key: impl fmt::Display,
    sql: &str,
    params: P,
) -> Result<Option<T>>
where
    T: DeserializeOwned,
    P: Params,
{
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备查询表 {table} 的语句"), err))?;
    let raw: Option<String> = stmt
        .query_row(params, |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询表 {table} 主键 {key}"), err))?;
    raw.map(|json| decode_doc(table, &key, &json)).transpose()
}

/// Fetch at most one `(primary_key, doc)` row (columns 0 and 1) via `prepare_cached` and
/// deserialize `doc` into `T`.
///
/// Useful when querying by a secondary index where the primary key is not known beforehand.
pub fn query_keyed_doc_opt<T, P>(
    conn: &Connection,
    table: &str,
    sql: &str,
    params: P,
) -> Result<Option<T>>
where
    T: DeserializeOwned,
    P: Params,
{
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备查询表 {table} 的语句"), err))?;
    let pair: Option<(String, String)> = stmt
        .query_row(params, |row| {
            let key = format_value_ref(row.get_ref(0)?);
            let doc: String = row.get(1)?;
            Ok((key, doc))
        })
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询表 {table}"), err))?;
    pair.map(|(key, json)| decode_doc(table, &key, &json))
        .transpose()
}

/// Fetch zero or more `(primary_key, doc)` rows (columns 0 and 1) via `prepare_cached` and
/// deserialize each `doc` into `T`.
///
/// Column 0 must select the row's primary key so any JSON parse failure names both `table` and
/// the exact offending key.
pub fn query_docs<T, P>(conn: &Connection, table: &str, sql: &str, params: P) -> Result<Vec<T>>
where
    T: DeserializeOwned,
    P: Params,
{
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备批量查询表 {table} 的语句"), err))?;
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite(format!("批量查询表 {table}"), err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取表 {table} 查询结果"), err))?
    {
        let key_ref = row
            .get_ref(0)
            .map_err(|err| DbError::sqlite(format!("读取表 {table} 主键列"), err))?;
        let key = format_value_ref(key_ref);
        let raw: String = row
            .get(1)
            .map_err(|err| DbError::sqlite(format!("读取表 {table} 主键 {key} 的 doc 列"), err))?;
        out.push(decode_doc(table, &key, &raw)?);
    }
    Ok(out)
}

fn format_value_ref(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_owned(),
        ValueRef::Integer(v) => v.to_string(),
        ValueRef::Real(v) => v.to_string(),
        ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        ValueRef::Blob(bytes) => format!("0x{}", hex_encode(bytes)),
    }
}
