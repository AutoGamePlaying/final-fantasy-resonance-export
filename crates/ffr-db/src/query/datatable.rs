//! Raw DataTable area (`02_datatable`) `impl GameDb` queries.

use ffr_data::DataTableType;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record::{decode_doc, query_doc_opt};

/// Metadata for one raw exported DataTable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DtTableInfo {
    pub id: i64,
    pub path: String,
    pub kind: DataTableType,
    pub row_struct: String,
    pub row_count: u64,
}

impl GameDb {
    /// List tables whose relative paths start with `prefix`, ordered by path.
    pub fn dt_tables(&self, prefix: &str) -> Result<Vec<DtTableInfo>> {
        self.with_conn(|conn| {
            let upper = prefix_upper_bound(prefix);
            let mut stmt = if upper.is_some() {
                conn.prepare_cached(
                    "SELECT id, path, kind, row_struct, row_count FROM dt_table \
                     INDEXED BY idx_dt_table_path WHERE path >= ?1 AND path < ?2 ORDER BY path",
                )
            } else {
                conn.prepare_cached(
                    "SELECT id, path, kind, row_struct, row_count FROM dt_table \
                     INDEXED BY idx_dt_table_path WHERE path >= ?1 ORDER BY path",
                )
            }
            .map_err(|err| DbError::sqlite("准备按前缀查询 dt_table", err))?;
            let mut rows = if let Some(upper) = upper {
                stmt.query(params![prefix, upper])
            } else {
                stmt.query([prefix])
            }
            .map_err(|err| DbError::sqlite("按前缀查询 dt_table", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 dt_table 前缀结果", err))?
            {
                out.push(
                    read_table_info(row)
                        .map_err(|err| DbError::sqlite("读取 dt_table 元数据", err))?,
                );
            }
            Ok(out)
        })
    }

    /// Fetch one raw row by relative table path and row name.
    pub fn dt_row(&self, path: &str, row_name: &str) -> Result<Option<Value>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "dt_row",
                format!("{path}/{row_name}"),
                "SELECT r.doc FROM dt_table AS t INDEXED BY idx_dt_table_path \
                 JOIN dt_row AS r ON r.table_id = t.id \
                 WHERE t.path = ?1 AND r.row_name = ?2",
                params![path, row_name],
            )
        })
    }

    /// Fetch all raw rows in a table as `(row_name, document)` pairs ordered by row name.
    pub fn dt_rows(&self, path: &str) -> Result<Vec<(String, Value)>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT r.row_name, r.doc FROM dt_table AS t INDEXED BY idx_dt_table_path \
                     JOIN dt_row AS r ON r.table_id = t.id WHERE t.path = ?1 ORDER BY r.row_name",
                )
                .map_err(|err| DbError::sqlite("准备查询 dt_rows", err))?;
            let mut rows = stmt
                .query([path])
                .map_err(|err| DbError::sqlite(format!("查询数据表 {path} 的全部行"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 dt_row 结果", err))?
            {
                let row_name: String = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 dt_row.row_name", err))?;
                let doc: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 dt_row.doc", err))?;
                out.push((
                    row_name.clone(),
                    decode_doc("dt_row", format!("{path}/{row_name}"), &doc)?,
                ));
            }
            Ok(out)
        })
    }

    /// Fetch table metadata by `/Game/Datatable/...` UE object or package path.
    pub fn dt_table_by_ue_path(&self, ue_path: &str) -> Result<Option<DtTableInfo>> {
        let Some(path) = ue_path_to_relative(ue_path) else {
            return Ok(None);
        };
        self.with_conn(|conn| table_by_path(conn, &path))
    }
}

fn table_by_path(conn: &Connection, path: &str) -> Result<Option<DtTableInfo>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, path, kind, row_struct, row_count FROM dt_table \
             INDEXED BY idx_dt_table_path WHERE path = ?1",
        )
        .map_err(|err| DbError::sqlite("准备按 UE 路径查询 dt_table", err))?;
    stmt.query_row([path], read_table_info)
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询数据表 {path}"), err))
}

fn read_table_info(row: &Row<'_>) -> rusqlite::Result<DtTableInfo> {
    let raw_kind: String = row.get(2)?;
    let kind = match raw_kind.as_str() {
        "DataTable" => DataTableType::DataTable,
        "CompositeDataTable" => DataTableType::CompositeDataTable,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let raw_count: i64 = row.get(4)?;
    let row_count = u64::try_from(raw_count).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(DtTableInfo {
        id: row.get(0)?,
        path: row.get(1)?,
        kind,
        row_struct: row.get(3)?,
        row_count,
    })
}

fn ue_path_to_relative(ue_path: &str) -> Option<String> {
    let relative = ue_path.strip_prefix("/Game/Datatable/")?;
    let mut segments: Vec<&str> = relative.split('/').collect();
    let last = segments.pop()?;
    let stem = last.split('.').next().unwrap_or(last);
    if stem.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    let mut path = segments.join("/");
    if !path.is_empty() {
        path.push('/');
    }
    path.push_str(stem);
    Some(path)
}

fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let mut code = last as u32 + 1;
        while code <= 0x10ffff {
            if let Some(next) = char::from_u32(code) {
                chars.push(next);
                return Some(chars.into_iter().collect());
            }
            code += 1;
        }
    }
    None
}
