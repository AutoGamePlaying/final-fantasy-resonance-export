//! Deterministic whole-file, canonical content, and canonical exporter-manifest hashing for
//! `ffr-db` SQLite databases.
//!
//! Computes:
//! - `file_sha256`: raw SHA-256 digest of the entire SQLite file on disk.
//! - `content_sha256`: canonical content digest over every ordinary table (excluding `sqlite_*`
//!   internal tables, `sqlite_stat*`, and FTS5/R*Tree virtual tables and their shadow tables),
//!   sorted by table name, hashing declared column names & types followed by rows ordered by
//!   explicit primary key (or `rowid` when no explicit primary key exists), with every value
//!   canonically encoded as `tag + length + bytes`.
//! - Canonical exporter manifest SHA-256 (`hash_manifest_file`): parses `manifest.json`,
//!   recursively strips volatile per-run keys listed in [`MANIFEST_VOLATILE_KEYS`] (`startedAt`,
//!   `finishedAt`, `elapsedSeconds`, `elapsedMs`, `generatedAt`, `jobs`), sorts all JSON object
//!   keys lexicographically, serializes to compact canonical JSON, and computes SHA-256. The raw
//!   file SHA-256 is returned alongside it for recording only in `<db>.manifest.json`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DbError, Result};
use crate::schema::META_KEY_SCHEMA_VERSION;
use crate::util::{Sha256, hex_encode, sha256_file, sha256_hex};

/// Format version of the JSON output emitted by `ffr-db hash --json`.
pub const HASH_FORMAT_VERSION: u32 = 1;

/// JSON object keys stripped recursively before computing a canonical exporter manifest SHA-256.
///
/// Matches the volatile per-run metadata emitted by `ffr-export`:
/// - `startedAt`, `finishedAt`: wall-clock UTC timestamps in `export`, `locres`, `inventory`,
///   `export-assets`, and `export-maps` manifests.
/// - `elapsedSeconds`: elapsed wall-clock seconds in `export-full/manifest.json`.
/// - `elapsedMs`: elapsed wall-clock milliseconds in `export-full/maps/manifest.json`.
/// - `generatedAt`: wall-clock UTC timestamp in `export-full/maps/_graph.json`.
/// - `jobs`: worker concurrency level in `export-full/manifest.json`.
pub const MANIFEST_VOLATILE_KEYS: &[&str] = &[
    "startedAt",
    "finishedAt",
    "elapsedSeconds",
    "elapsedMs",
    "generatedAt",
    "jobs",
];

/// Canonicalized and raw SHA-256 digests of an input exporter `manifest.json` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDigest {
    /// SHA-256 of canonical compact JSON after recursively stripping [`MANIFEST_VOLATILE_KEYS`]
    /// and sorting all object keys lexicographically. Stored in SQLite `source.manifest_sha256`.
    pub canonical_sha256: String,
    /// Raw byte-for-byte SHA-256 of the file on disk. Stored only in `<db>.manifest.json`.
    pub raw_sha256: String,
}

/// Serialize `value` to compact canonical JSON bytes, recursively removing every object key in
/// [`MANIFEST_VOLATILE_KEYS`] and sorting object keys in lexicographical UTF-8 byte order.
pub fn canonical_manifest_json(value: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_canonical_manifest_json(value, &mut out)?;
    Ok(out)
}

/// Compute the lowercase hex SHA-256 of [`canonical_manifest_json`].
pub fn canonical_manifest_sha256(value: &Value) -> Result<String> {
    let bytes = canonical_manifest_json(value)?;
    Ok(sha256_hex(&bytes))
}

/// Read an exporter JSON manifest from disk and compute both its canonicalized SHA-256 (for
/// SQLite `source.manifest_sha256`) and raw file SHA-256 (for `<db>.manifest.json`).
pub fn hash_manifest_file(path: &Path) -> Result<ManifestDigest> {
    let raw_bytes = fs::read(path).map_err(|err| DbError::io("读取清单文件", path, err))?;
    let raw_sha256 = sha256_hex(&raw_bytes);
    let json_bytes = raw_bytes
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(&raw_bytes);
    let value: Value =
        serde_json::from_slice(json_bytes).map_err(|source| DbError::DocDeserialize {
            table: "manifest.json".to_owned(),
            key: path.display().to_string(),
            source,
        })?;
    let canonical_sha256 = canonical_manifest_sha256(&value)?;
    Ok(ManifestDigest {
        canonical_sha256,
        raw_sha256,
    })
}

fn write_canonical_manifest_json(value: &Value, out: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_writer(&mut *out, value).map_err(|source| DbError::DocSerialize {
                table: "manifest.json".to_owned(),
                key: "canonical_json".to_owned(),
                source,
            })?;
        }
        Value::Array(items) => {
            out.push(b'[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                write_canonical_manifest_json(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            out.push(b'{');
            let mut entries: Vec<(&String, &Value)> = map
                .iter()
                .filter(|(k, _)| !MANIFEST_VOLATILE_KEYS.contains(&k.as_str()))
                .collect();
            entries.sort_by_key(|(a, _)| *a);
            for (idx, (k, v)) in entries.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, k).map_err(|source| DbError::DocSerialize {
                    table: "manifest.json".to_owned(),
                    key: (*k).clone(),
                    source,
                })?;
                out.push(b':');
                write_canonical_manifest_json(v, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// Canonical value encoding tag for SQLite `NULL` (tag only, no length or payload).
pub const VALUE_TAG_NULL: u8 = 0x00;
/// Canonical value encoding tag for SQLite `INTEGER` (`i64` little-endian, length = 8).
pub const VALUE_TAG_INTEGER: u8 = 0x01;
/// Canonical value encoding tag for SQLite `REAL` (`f64::to_bits` little-endian, length = 8).
pub const VALUE_TAG_REAL: u8 = 0x02;
/// Canonical value encoding tag for SQLite `TEXT` (UTF-8 bytes, length = byte length).
pub const VALUE_TAG_TEXT: u8 = 0x03;
/// Canonical value encoding tag for SQLite `BLOB` (raw bytes, length = byte length).
pub const VALUE_TAG_BLOB: u8 = 0x04;

/// Read-only memory-mapped I/O window used while hashing (256 MiB).
const HASH_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;
/// Read-only page cache size used while hashing (-65536 = 64 MiB).
const HASH_CACHE_SIZE_KIB: i64 = -65_536;

/// Known SQLite shadow table suffixes produced by FTS5, FTS3/4, and R*Tree virtual tables.
const VIRTUAL_SHADOW_SUFFIXES: &[&str] = &[
    "_data",
    "_idx",
    "_content",
    "_docsize",
    "_config",
    "_node",
    "_rowid",
    "_parent",
    "_segments",
    "_segdir",
    "_stat",
];

/// Canonical content hash summary for a single ordinary table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableHash {
    pub name: String,
    pub rows: u64,
    pub sha256: String,
}

/// Complete file and content hash report emitted by `ffr-db hash` and embedded in build manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbHashReport {
    pub format: u32,
    pub file_sha256: String,
    pub content_sha256: String,
    pub schema_version: u32,
    pub sqlite_version: String,
    pub tables: Vec<TableHash>,
}

/// Content-only hash summary computed directly over an open SQLite [`Connection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHashReport {
    pub content_sha256: String,
    pub schema_version: u32,
    pub sqlite_version: String,
    pub tables: Vec<TableHash>,
}

/// Append the canonical encoding of a single SQLite value to `out`.
///
/// Format:
/// - `NULL`: `[0x00]`
/// - `INTEGER`: `[0x01] || 8u64.to_le_bytes() || value.to_le_bytes()`
/// - `REAL`: `[0x02] || 8u64.to_le_bytes() || value.to_bits().to_le_bytes()`
/// - `TEXT`: `[0x03] || (bytes.len() as u64).to_le_bytes() || bytes`
/// - `BLOB`: `[0x04] || (bytes.len() as u64).to_le_bytes() || bytes`
pub fn encode_canonical_value(value: ValueRef<'_>, out: &mut Vec<u8>) {
    match value {
        ValueRef::Null => out.push(VALUE_TAG_NULL),
        ValueRef::Integer(v) => {
            out.push(VALUE_TAG_INTEGER);
            out.extend_from_slice(&8u64.to_le_bytes());
            out.extend_from_slice(&v.to_le_bytes());
        }
        ValueRef::Real(v) => {
            out.push(VALUE_TAG_REAL);
            out.extend_from_slice(&8u64.to_le_bytes());
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        ValueRef::Text(bytes) => {
            out.push(VALUE_TAG_TEXT);
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        ValueRef::Blob(bytes) => {
            out.push(VALUE_TAG_BLOB);
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(bytes);
        }
    }
}

fn update_canonical_value(hasher: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hasher.update(&[VALUE_TAG_NULL]),
        ValueRef::Integer(v) => {
            hasher.update(&[VALUE_TAG_INTEGER]);
            hasher.update(&8u64.to_le_bytes());
            hasher.update(&v.to_le_bytes());
        }
        ValueRef::Real(v) => {
            hasher.update(&[VALUE_TAG_REAL]);
            hasher.update(&8u64.to_le_bytes());
            hasher.update(&v.to_bits().to_le_bytes());
        }
        ValueRef::Text(bytes) => {
            hasher.update(&[VALUE_TAG_TEXT]);
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        ValueRef::Blob(bytes) => {
            hasher.update(&[VALUE_TAG_BLOB]);
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
    }
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Compute both the raw file SHA-256 and canonical content SHA-256 of a SQLite database file.
pub fn hash_database(path: impl AsRef<Path>) -> Result<DbHashReport> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(DbError::OpenFailed {
            path: path.to_path_buf(),
            detail: "文件不存在或不是普通文件".to_owned(),
        });
    }

    let file_sha256 = sha256_file(path)?;
    let conn = open_hash_connection(path)?;
    let content = hash_connection_content(&conn)?;
    conn.close()
        .map_err(|(_, err)| DbError::sqlite("关闭哈希计算数据库连接", err))?;

    Ok(DbHashReport {
        format: HASH_FORMAT_VERSION,
        file_sha256,
        content_sha256: content.content_sha256,
        schema_version: content.schema_version,
        sqlite_version: content.sqlite_version,
        tables: content.tables,
    })
}

/// Compute the canonical content hash directly over an open SQLite [`Connection`].
pub fn hash_connection_content(conn: &Connection) -> Result<ContentHashReport> {
    let sqlite_version = query_sqlite_version(conn)?;
    let schema_version = query_schema_version(conn)?;
    let table_names = list_ordinary_tables(conn)?;

    let mut tables = Vec::with_capacity(table_names.len());
    for name in table_names {
        tables.push(hash_table(conn, &name)?);
    }

    let mut overall = Sha256::new();
    overall.update(&(tables.len() as u64).to_le_bytes());
    for table in &tables {
        update_length_prefixed(&mut overall, table.name.as_bytes());
        update_length_prefixed(&mut overall, table.sha256.as_bytes());
    }
    let content_sha256 = hex_encode(&overall.finalize());

    Ok(ContentHashReport {
        content_sha256,
        schema_version,
        sqlite_version,
        tables,
    })
}

/// Return the SQLite engine version string reported by `SELECT sqlite_version()`.
pub fn query_sqlite_version(conn: &Connection) -> Result<String> {
    conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .map_err(|err| DbError::sqlite("查询 sqlite_version()", err))
}

fn open_hash_connection(path: &Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|err| DbError::OpenFailed {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    let pragmas = format!(
        "PRAGMA query_only = ON;\
         PRAGMA mmap_size = {HASH_MMAP_SIZE_BYTES};\
         PRAGMA cache_size = {HASH_CACHE_SIZE_KIB};\
         PRAGMA temp_store = MEMORY;\
         PRAGMA busy_timeout = 5000;"
    );
    conn.execute_batch(&pragmas)
        .map_err(|err| DbError::OpenFailed {
            path: path.to_path_buf(),
            detail: format!("设置哈希连接只读 PRAGMA 失败：{err}"),
        })?;
    Ok(conn)
}

fn query_schema_version(conn: &Connection) -> Result<u32> {
    let has_meta: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
            [],
            |row| row.get(0),
        )
        .map_err(|err| DbError::sqlite("检查 meta 表是否存在", err))?;

    if has_meta {
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [META_KEY_SCHEMA_VERSION],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| DbError::sqlite("读取 meta.schema_version", err))?;
        if let Some(version_str) = raw
            && let Ok(parsed) = version_str.trim().parse::<u32>()
        {
            return Ok(parsed);
        }
    }

    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|err| DbError::sqlite("读取 PRAGMA user_version", err))?;
    Ok(u32::try_from(user_version).unwrap_or(0))
}

fn list_ordinary_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT name, COALESCE(sql, '') FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .map_err(|err| DbError::sqlite("查询 sqlite_master 表列表", err))?;

    let mut raw_tables: Vec<(String, String)> = Vec::new();
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("读取 sqlite_master 表列表", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 sqlite_master 表列表", err))?
    {
        let name: String = row.get(0).map_err(|err| DbError::sqlite("读取表名", err))?;
        let sql: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取建表 SQL", err))?;
        raw_tables.push((name, sql));
    }

    let mut excluded: BTreeSet<String> = BTreeSet::new();

    // Exclude virtual tables and their shadow tables identified from DDL.
    for (name, sql) in &raw_tables {
        if sql
            .trim_start()
            .to_ascii_uppercase()
            .starts_with("CREATE VIRTUAL TABLE")
        {
            excluded.insert(name.clone());
            for suffix in VIRTUAL_SHADOW_SUFFIXES {
                excluded.insert(format!("{name}{suffix}"));
            }
        }
    }

    // Also consult `PRAGMA table_list` so any virtual or shadow table reported by SQLite itself
    // is excluded even if its virtual-table module uses non-standard shadow suffixes.
    if let Ok(mut tl_stmt) = conn.prepare("PRAGMA table_list")
        && let Ok(mut tl_rows) = tl_stmt.query([])
    {
        while let Ok(Some(row)) = tl_rows.next() {
            let schema: String = row.get(0).unwrap_or_default();
            let name: String = row.get(1).unwrap_or_default();
            let kind: String = row.get(2).unwrap_or_default();
            if schema == "main" && matches!(kind.as_str(), "virtual" | "shadow" | "view") {
                excluded.insert(name);
            }
        }
    }

    let mut tables: Vec<String> = raw_tables
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !excluded.contains(name))
        .collect();
    tables.sort();
    Ok(tables)
}

struct ColumnInfo {
    cid: i64,
    name: String,
    col_type: String,
    pk: i64,
}

fn hash_table(conn: &Connection, table_name: &str) -> Result<TableHash> {
    let quoted_table = quote_ident(table_name);
    let pragma_sql = format!("PRAGMA table_info({quoted_table})");
    let mut columns = Vec::new();
    {
        let mut stmt = conn
            .prepare(&pragma_sql)
            .map_err(|err| DbError::sqlite(format!("读取表 {table_name} 列定义"), err))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| DbError::sqlite(format!("执行 {pragma_sql}"), err))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| DbError::sqlite(format!("迭代表 {table_name} 列定义"), err))?
        {
            columns.push(ColumnInfo {
                cid: row
                    .get(0)
                    .map_err(|err| DbError::sqlite(format!("读取表 {table_name} cid"), err))?,
                name: row
                    .get(1)
                    .map_err(|err| DbError::sqlite(format!("读取表 {table_name} 列名"), err))?,
                col_type: row
                    .get(2)
                    .map_err(|err| DbError::sqlite(format!("读取表 {table_name} 列类型"), err))?,
                pk: row
                    .get(5)
                    .map_err(|err| DbError::sqlite(format!("读取表 {table_name} pk"), err))?,
            });
        }
    }
    columns.sort_by_key(|c| c.cid);

    let mut hasher = Sha256::new();
    hasher.update(&(columns.len() as u64).to_le_bytes());
    for col in &columns {
        update_length_prefixed(&mut hasher, col.name.as_bytes());
        update_length_prefixed(&mut hasher, col.col_type.as_bytes());
    }

    let select_list = columns
        .iter()
        .map(|c| quote_ident(&c.name))
        .collect::<Vec<_>>()
        .join(", ");

    let mut pk_cols: Vec<(i64, &str)> = columns
        .iter()
        .filter(|c| c.pk > 0)
        .map(|c| (c.pk, c.name.as_str()))
        .collect();
    pk_cols.sort_by_key(|(pk, _)| *pk);

    let order_clause = if pk_cols.is_empty() {
        "rowid".to_owned()
    } else {
        pk_cols
            .iter()
            .map(|(_, name)| quote_ident(name))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let select_sql = format!("SELECT {select_list} FROM {quoted_table} ORDER BY {order_clause}");
    let mut stmt = conn
        .prepare(&select_sql)
        .map_err(|err| DbError::sqlite(format!("准备扫描表 {table_name} 计算内容哈希"), err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite(format!("扫描表 {table_name} 计算内容哈希"), err))?;

    let mut row_count: u64 = 0;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取表 {table_name} 行数据"), err))?
    {
        for (col_idx, col) in columns.iter().enumerate() {
            let val_ref = row.get_ref(col_idx).map_err(|err| {
                DbError::sqlite(format!("读取表 {table_name} 列 {}", col.name), err)
            })?;
            update_canonical_value(&mut hasher, val_ref);
        }
        row_count = row_count.saturating_add(1);
    }

    hasher.update(&row_count.to_le_bytes());
    let sha256 = hex_encode(&hasher.finalize());

    Ok(TableHash {
        name: table_name.to_owned(),
        rows: row_count,
        sha256,
    })
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use rusqlite::types::ValueRef;
    use serde_json::json;

    use super::{
        MANIFEST_VOLATILE_KEYS, VALUE_TAG_BLOB, VALUE_TAG_INTEGER, VALUE_TAG_NULL, VALUE_TAG_REAL,
        VALUE_TAG_TEXT, canonical_manifest_json, canonical_manifest_sha256, encode_canonical_value,
        hash_manifest_file, update_canonical_value,
    };
    use crate::util::{Sha256, hex_encode, sha256_hex};

    fn assert_stream_matches_buffer(value: ValueRef<'_>, expected: &[u8]) {
        let mut buf = Vec::new();
        encode_canonical_value(value, &mut buf);
        assert_eq!(buf, expected);

        let mut hasher = Sha256::new();
        update_canonical_value(&mut hasher, value);
        assert_eq!(hex_encode(&hasher.finalize()), sha256_hex(&buf));
    }

    #[test]
    fn canonical_manifest_hash_strips_volatile_keys_and_sorts_object_keys() {
        for required in [
            "startedAt",
            "finishedAt",
            "elapsedSeconds",
            "elapsedMs",
            "generatedAt",
            "jobs",
        ] {
            assert!(
                MANIFEST_VOLATILE_KEYS.contains(&required),
                "MANIFEST_VOLATILE_KEYS missing required key: {required}"
            );
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let path_a = dir.path().join("manifest_a.json");
        let path_b = dir.path().join("manifest_b.json");
        let path_c = dir.path().join("manifest_c.json");

        // Manifest A and B differ only in volatile keys (top-level and nested), key order,
        // indentation, and UTF-8 BOM.
        let raw_a = r#"{
  "usmapSha256": "AABBCC",
  "command": "export-assets",
  "jobs": 4,
  "startedAt": "2026-11-25T14:00:00.0000001Z",
  "finishedAt": "2026-11-25T14:00:15.1234567Z",
  "elapsedSeconds": 15.123,
  "elapsedMs": 15123,
  "generatedAt": "2026-11-25T14:00:15Z",
  "nested": {
    "b": 2,
    "a": 1,
    "generatedAt": "2026-11-25T14:00:01Z",
    "items": [
      { "z": "last", "a": "first", "elapsedMs": 12, "jobs": 4 }
    ]
  }
}"#;
        let raw_b = "\u{feff}{\"nested\":{\"items\":[{\"jobs\":16,\"a\":\"first\",\"elapsedMs\":999,\"z\":\"last\"}],\"a\":1,\"b\":2,\"startedAt\":\"2099-01-01T00:00:00Z\"},\"command\":\"export-assets\",\"usmapSha256\":\"AABBCC\",\"jobs\":16,\"startedAt\":\"2099-01-01T00:00:00Z\",\"finishedAt\":\"2099-01-01T01:00:00Z\",\"elapsedSeconds\":3600.0,\"elapsedMs\":3600000,\"generatedAt\":\"2099-01-01T01:00:00Z\"}";
        // Manifest C changes one non-volatile semantic key (`usmapSha256`).
        let raw_c = raw_a.replace("AABBCC", "DDEEFF");

        std::fs::write(&path_a, raw_a).expect("write manifest A");
        std::fs::write(&path_b, raw_b).expect("write manifest B");
        std::fs::write(&path_c, raw_c).expect("write manifest C");

        let digest_a = hash_manifest_file(&path_a).expect("hash manifest A");
        let digest_b = hash_manifest_file(&path_b).expect("hash manifest B");
        let digest_c = hash_manifest_file(&path_c).expect("hash manifest C");

        assert_eq!(digest_a.canonical_sha256, digest_b.canonical_sha256);
        assert_ne!(digest_a.raw_sha256, digest_b.raw_sha256);
        assert_ne!(digest_a.canonical_sha256, digest_c.canonical_sha256);

        // Verify canonical bytes have keys sorted and volatile keys stripped at every level.
        let val_a: serde_json::Value = serde_json::from_str(raw_a).expect("parse A");
        let canonical_str = String::from_utf8(canonical_manifest_json(&val_a).expect("canon A"))
            .expect("utf8 canon A");
        assert_eq!(
            canonical_str,
            r#"{"command":"export-assets","nested":{"a":1,"b":2,"items":[{"a":"first","z":"last"}]},"usmapSha256":"AABBCC"}"#
        );

        // Changing a nested semantic key also changes the canonical hash.
        let nested_changed = json!({
            "command": "export-assets",
            "usmapSha256": "AABBCC",
            "nested": {
                "a": 1,
                "b": 3,
                "items": [{ "a": "first", "z": "last" }]
            }
        });
        assert_ne!(
            digest_a.canonical_sha256,
            canonical_manifest_sha256(&nested_changed).expect("hash nested changed")
        );
    }

    #[test]
    fn canonical_value_encoding_covers_null_integer_real_text_and_blob() {
        // NULL: tag only
        assert_stream_matches_buffer(ValueRef::Null, &[VALUE_TAG_NULL]);

        // INTEGER: tag + u64(8) LE + i64 LE (positive, zero, negative, extremes)
        for v in [0_i64, 1, -1, 42, i64::MIN, i64::MAX] {
            let mut expected = vec![VALUE_TAG_INTEGER];
            expected.extend_from_slice(&8u64.to_le_bytes());
            expected.extend_from_slice(&v.to_le_bytes());
            assert_stream_matches_buffer(ValueRef::Integer(v), &expected);
        }

        // REAL: tag + u64(8) LE + f64::to_bits LE
        for v in [0.0_f64, -0.0, 1.5, -1234.5678, f64::MIN, f64::MAX] {
            let mut expected = vec![VALUE_TAG_REAL];
            expected.extend_from_slice(&8u64.to_le_bytes());
            expected.extend_from_slice(&v.to_bits().to_le_bytes());
            assert_stream_matches_buffer(ValueRef::Real(v), &expected);
        }

        // TEXT: tag + u64(len) LE + UTF-8 bytes (empty and CJK)
        for text in ["", "hello", "钢蝠/FFRS"] {
            let bytes = text.as_bytes();
            let mut expected = vec![VALUE_TAG_TEXT];
            expected.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            expected.extend_from_slice(bytes);
            assert_stream_matches_buffer(ValueRef::Text(bytes), &expected);
        }

        // BLOB: tag + u64(len) LE + raw bytes (empty and binary)
        for blob in [&[][..], &[0x00, 0x01, 0xfe, 0xff][..]] {
            let mut expected = vec![VALUE_TAG_BLOB];
            expected.extend_from_slice(&(blob.len() as u64).to_le_bytes());
            expected.extend_from_slice(blob);
            assert_stream_matches_buffer(ValueRef::Blob(blob), &expected);
        }

        // Distinct types with identical byte patterns never collide:
        // empty TEXT != empty BLOB != NULL; int 0 != float 0.0.
        let mut null_enc = Vec::new();
        let mut empty_text = Vec::new();
        let mut empty_blob = Vec::new();
        let mut int_zero = Vec::new();
        let mut real_zero = Vec::new();
        encode_canonical_value(ValueRef::Null, &mut null_enc);
        encode_canonical_value(ValueRef::Text(b""), &mut empty_text);
        encode_canonical_value(ValueRef::Blob(b""), &mut empty_blob);
        encode_canonical_value(ValueRef::Integer(0), &mut int_zero);
        encode_canonical_value(ValueRef::Real(0.0), &mut real_zero);
        assert_ne!(null_enc, empty_text);
        assert_ne!(empty_text, empty_blob);
        assert_ne!(int_zero, real_zero);
    }
}
