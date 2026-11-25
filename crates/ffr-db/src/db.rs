//! Thread-safe read-only [`GameDb`] handle backed by a bounded on-demand SQLite connection pool.

use std::collections::{BTreeMap, HashSet};
use std::ffi::{c_int, c_void};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Params};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};

use crate::build::{BuildIssue, SourceRecord, StageReport, StageStatus};
use crate::error::{DbError, Result};
use crate::schema::{META_KEY_SCHEMA_VERSION, SCHEMA_VERSION};
use crate::util::{hex_encode, validate_readonly_sql};

/// Maximum number of concurrent SQLite connections held by a [`GameDb`] pool.
pub const MAX_POOL_CONNECTIONS: usize = 4;

/// Number of SQLite VM instructions between progress handler timeout checks.
const PROGRESS_HANDLER_VM_OPS: c_int = 1_000;

/// Result of a row-capped, timeout-protected read-only SQL query
/// ([`GameDb::query_readonly_limited`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadonlyQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<JsonMap<String, JsonValue>>,
    pub row_limit: usize,
    pub truncated_rows: bool,
    pub elapsed_ms: u64,
}

/// Returns `true` if `err` was caused by SQLite interrupting a query (such as a progress-handler
/// timeout in [`GameDb::query_readonly_limited`]).
pub fn is_query_timeout_error(err: &DbError) -> bool {
    matches!(
        err,
        DbError::Sqlite {
            source: rusqlite::Error::SqliteFailure(code, _),
            ..
        } if code.code == rusqlite::ErrorCode::OperationInterrupted
    )
}

/// Read-only memory-mapped I/O window size (256 MiB).
const READONLY_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;
/// Read-only SQLite page cache size (-65536 = 64 MiB).
const READONLY_CACHE_SIZE_KIB: i64 = -65_536;

/// One row returned by SQLite `EXPLAIN QUERY PLAN`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryPlanRow {
    pub id: i64,
    pub parent: i64,
    pub notused: i64,
    pub detail: String,
}

/// Row count summary for one table in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRowCount {
    pub name: String,
    pub rows: u64,
}

/// High-level database statistics returned by [`GameDb::stats`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbStats {
    pub path: PathBuf,
    pub file_size_bytes: u64,
    pub meta: BTreeMap<String, String>,
    pub tables: Vec<TableRowCount>,
}

struct PoolState {
    idle: Vec<Connection>,
    total: usize,
}

struct DbInner {
    path: PathBuf,
    pool: Mutex<PoolState>,
    available: Condvar,
}

/// Thread-safe (`Send + Sync`), read-only handle to an `ffr-db` SQLite file.
///
/// Internally maintains a small on-demand connection pool (up to [`MAX_POOL_CONNECTIONS`] = 4
/// connections). Every connection is opened read-only with `query_only = ON`, 256 MiB `mmap_size`,
/// a 64 MiB page cache, and `temp_store = MEMORY`.
#[derive(Clone)]
pub struct GameDb {
    inner: Arc<DbInner>,
}

impl std::fmt::Debug for GameDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameDb")
            .field("path", &self.inner.path)
            .finish()
    }
}

struct PooledConn<'a> {
    inner: &'a DbInner,
    conn: Option<Connection>,
}

impl PooledConn<'_> {
    fn conn(&self) -> &Connection {
        self.conn
            .as_ref()
            .expect("pooled connection must be present before drop")
    }
}

impl Drop for PooledConn<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let mut state = self
                .inner
                .pool
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.idle.push(conn);
            self.inner.available.notify_one();
        }
    }
}

impl GameDb {
    /// Open an existing `ffr-db` SQLite file read-only and verify its `meta.schema_version`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = open_readonly_connection(&path)?;
        verify_schema_version(&conn, &path)?;

        let inner = Arc::new(DbInner {
            path,
            pool: Mutex::new(PoolState {
                idle: vec![conn],
                total: 1,
            }),
            available: Condvar::new(),
        });
        Ok(Self { inner })
    }

    /// Filesystem path of this database file.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Number of SQLite connections currently created in the pool (for diagnostics/tests).
    pub fn pool_size(&self) -> usize {
        self.inner
            .pool
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .total
    }

    /// Borrow a pooled read-only SQLite connection and run `f`.
    ///
    /// Connections are opened lazily up to [`MAX_POOL_CONNECTIONS`] and returned to the pool
    /// automatically when `f` finishes or unwinds. Query code should use `conn.prepare_cached`.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.checkout()?;
        f(guard.conn())
    }

    /// Read all key-value pairs from the `meta` table.
    pub fn meta(&self) -> Result<BTreeMap<String, String>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached("SELECT key, value FROM meta ORDER BY key")
                .map_err(|err| DbError::sqlite("准备查询 meta 表", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("查询 meta 表", err))?;
            let mut map = BTreeMap::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 meta 行", err))?
            {
                let key: String = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 meta.key", err))?;
                let value: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 meta.value", err))?;
                map.insert(key, value);
            }
            Ok(map)
        })
    }

    /// Read all rows from `source`.
    pub fn sources(&self) -> Result<Vec<SourceRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT stage, path, manifest_sha256, file_count FROM source ORDER BY stage, path",
                )
                .map_err(|err| DbError::sqlite("准备查询 source 表", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("查询 source 表", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 source 行", err))?
            {
                let file_count: Option<i64> = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 source.file_count", err))?;
                out.push(SourceRecord {
                    stage: row
                        .get(0)
                        .map_err(|err| DbError::sqlite("读取 source.stage", err))?,
                    path: row
                        .get(1)
                        .map_err(|err| DbError::sqlite("读取 source.path", err))?,
                    manifest_sha256: row
                        .get(2)
                        .map_err(|err| DbError::sqlite("读取 source.manifest_sha256", err))?,
                    file_count: file_count.and_then(|v| u64::try_from(v).ok()),
                });
            }
            Ok(out)
        })
    }

    /// Read all rows from `stage_report` in insertion order.
    pub fn stage_reports(&self) -> Result<Vec<StageReport>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached("SELECT stage, status, rows, note FROM stage_report ORDER BY rowid")
                .map_err(|err| DbError::sqlite("准备查询 stage_report 表", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("查询 stage_report 表", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 stage_report 行", err))?
            {
                let stage: String = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 stage_report.stage", err))?;
                let status_str: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 stage_report.status", err))?;
                let status = StageStatus::parse(&status_str).ok_or_else(|| {
                    DbError::sqlite(
                        format!("解析 stage_report.status（阶段 {stage}，值 {status_str}）"),
                        rusqlite::Error::InvalidQuery,
                    )
                })?;
                let row_count: i64 = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 stage_report.rows", err))?;
                let note: String = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 stage_report.note", err))?;
                out.push(StageReport {
                    stage,
                    status,
                    rows: u64::try_from(row_count).unwrap_or(0),
                    millis: 0,
                    note,
                });
            }
            Ok(out)
        })
    }

    /// Read all rows from `build_issue` in insertion order.
    pub fn build_issues(&self) -> Result<Vec<BuildIssue>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT stage, kind, subject, detail FROM build_issue ORDER BY rowid",
                )
                .map_err(|err| DbError::sqlite("准备查询 build_issue 表", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("查询 build_issue 表", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 build_issue 行", err))?
            {
                out.push(BuildIssue {
                    stage: row
                        .get(0)
                        .map_err(|err| DbError::sqlite("读取 build_issue.stage", err))?,
                    kind: row
                        .get(1)
                        .map_err(|err| DbError::sqlite("读取 build_issue.kind", err))?,
                    subject: row
                        .get(2)
                        .map_err(|err| DbError::sqlite("读取 build_issue.subject", err))?,
                    detail: row
                        .get(3)
                        .map_err(|err| DbError::sqlite("读取 build_issue.detail", err))?,
                });
            }
            Ok(out)
        })
    }

    /// Collect database file size, `meta` entries, and per-table row counts.
    pub fn stats(&self) -> Result<DbStats> {
        let file_size_bytes = std::fs::metadata(&self.inner.path)
            .map_err(|err| DbError::io("读取数据库文件元信息", &self.inner.path, err))?
            .len();
        let meta = self.meta()?;
        let tables = self.with_conn(count_user_tables)?;
        Ok(DbStats {
            path: self.inner.path.clone(),
            file_size_bytes,
            meta,
            tables,
        })
    }

    /// Run a strictly read-only SQL query and return each result row as a JSON object.
    ///
    /// Rejects any non-query, multi-statement, or write statement before execution.
    pub fn query_readonly_json(&self, sql: &str) -> Result<Vec<JsonMap<String, JsonValue>>> {
        let trimmed = validate_readonly_sql(sql)?;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(trimmed).map_err(|err| {
                if is_readonly_error(&err) {
                    DbError::WriteQueryRejected {
                        detail: err.to_string(),
                    }
                } else {
                    DbError::sqlite("解析只读 SQL", err)
                }
            })?;
            if !stmt.readonly() {
                return Err(DbError::WriteQueryRejected {
                    detail: "SQLite 判定该语句不是只读语句".to_owned(),
                });
            }

            let col_names: Vec<String> = stmt.column_names().map(ToOwned::to_owned).collect();
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("执行只读 SQL", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取只读 SQL 结果行", err))?
            {
                let mut obj = JsonMap::with_capacity(col_names.len());
                for (idx, name) in col_names.iter().enumerate() {
                    let val_ref = row
                        .get_ref(idx)
                        .map_err(|err| DbError::sqlite(format!("读取列 {name}"), err))?;
                    obj.insert(name.clone(), value_ref_to_json(val_ref));
                }
                out.push(obj);
            }
            Ok(out)
        })
    }

    /// Run a strictly read-only SQL query bounded by `max_rows` and a SQLite progress-handler
    /// wall-clock `timeout`.
    ///
    /// Rejects empty, non-query, multi-statement, or write SQL statements before execution.
    /// When the query exceeds `timeout`, SQLite aborts execution via `sqlite3_progress_handler`
    /// and returns a `DbError::Sqlite` matching [`is_query_timeout_error`].
    pub fn query_readonly_limited(
        &self,
        sql: &str,
        max_rows: usize,
        timeout: Duration,
    ) -> Result<ReadonlyQueryResult> {
        let trimmed = validate_readonly_sql(sql)?;
        let started = Instant::now();
        let timeout_ms = timeout.as_millis();

        self.with_conn(|conn| {
            let mut stmt = conn.prepare(trimmed).map_err(|err| {
                if is_readonly_error(&err) {
                    DbError::WriteQueryRejected {
                        detail: err.to_string(),
                    }
                } else {
                    DbError::sqlite("解析只读 SQL", err)
                }
            })?;
            if !stmt.readonly() {
                return Err(DbError::WriteQueryRejected {
                    detail: "SQLite 判定该语句不是只读语句".to_owned(),
                });
            }

            let col_names: Vec<String> = stmt.column_names().map(ToOwned::to_owned).collect();
            let progress_state = ProgressTimeoutState {
                deadline: started + timeout,
                timed_out: AtomicBool::new(false),
            };
            let _progress_guard =
                ProgressHandlerGuard::install(conn, PROGRESS_HANDLER_VM_OPS, &progress_state);

            let map_exec_err = |action: &str, err: rusqlite::Error| -> DbError {
                if progress_state.timed_out.load(Ordering::Relaxed) || is_interrupted_error(&err) {
                    DbError::sqlite(format!("只读 SQL 执行超时（超过 {timeout_ms} ms）"), err)
                } else {
                    DbError::sqlite(action, err)
                }
            };

            let mut rows = stmt
                .query([])
                .map_err(|err| map_exec_err("执行只读 SQL", err))?;
            let mut out = Vec::with_capacity(max_rows.min(256));
            let mut truncated_rows = false;
            while let Some(row) = rows
                .next()
                .map_err(|err| map_exec_err("读取只读 SQL 结果行", err))?
            {
                if out.len() >= max_rows {
                    truncated_rows = true;
                    break;
                }
                let mut obj = JsonMap::with_capacity(col_names.len());
                for (idx, name) in col_names.iter().enumerate() {
                    let val_ref = row
                        .get_ref(idx)
                        .map_err(|err| map_exec_err(&format!("读取列 {name}"), err))?;
                    obj.insert(name.clone(), value_ref_to_json(val_ref));
                }
                out.push(obj);
            }

            drop(rows);
            drop(stmt);
            drop(_progress_guard);

            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            Ok(ReadonlyQueryResult {
                columns: col_names,
                rows: out,
                row_limit: max_rows,
                truncated_rows,
                elapsed_ms,
            })
        })
    }

    /// Run `EXPLAIN QUERY PLAN` on a strictly read-only SQL statement with no bound parameters.
    pub fn explain_query_plan(&self, sql: &str) -> Result<Vec<QueryPlanRow>> {
        self.explain_query_plan_with_params(sql, [])
    }

    /// Run `EXPLAIN QUERY PLAN` on a strictly read-only SQL statement with bound parameters.
    pub fn explain_query_plan_with_params<P: Params>(
        &self,
        sql: &str,
        params: P,
    ) -> Result<Vec<QueryPlanRow>> {
        let trimmed = validate_readonly_sql(sql)?;
        self.with_conn(|conn| {
            {
                let stmt = conn
                    .prepare(trimmed)
                    .map_err(|err| DbError::sqlite("解析待分析 SQL", err))?;
                if !stmt.readonly() {
                    return Err(DbError::WriteQueryRejected {
                        detail: "SQLite 判定该语句不是只读语句".to_owned(),
                    });
                }
            }
            explain_query_plan_on_conn(conn, trimmed, params)
        })
    }

    fn checkout(&self) -> Result<PooledConn<'_>> {
        let mut state = self
            .inner
            .pool
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(conn) = state.idle.pop() {
                return Ok(PooledConn {
                    inner: &self.inner,
                    conn: Some(conn),
                });
            }
            if state.total < MAX_POOL_CONNECTIONS {
                state.total += 1;
                drop(state);
                match open_readonly_connection(&self.inner.path) {
                    Ok(conn) => {
                        return Ok(PooledConn {
                            inner: &self.inner,
                            conn: Some(conn),
                        });
                    }
                    Err(err) => {
                        let mut state = self
                            .inner
                            .pool
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        state.total -= 1;
                        self.inner.available.notify_one();
                        return Err(err);
                    }
                }
            }
            state = self
                .inner
                .available
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

pub(crate) fn explain_query_plan_on_conn<P: Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<QueryPlanRow>> {
    let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
    let mut stmt = conn
        .prepare(&explain_sql)
        .map_err(|err| DbError::sqlite("准备 EXPLAIN QUERY PLAN", err))?;
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite("执行 EXPLAIN QUERY PLAN", err))?;
    let mut plan = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 EXPLAIN QUERY PLAN 行", err))?
    {
        plan.push(QueryPlanRow {
            id: row
                .get(0)
                .map_err(|err| DbError::sqlite("读取查询计划 id", err))?,
            parent: row
                .get(1)
                .map_err(|err| DbError::sqlite("读取查询计划 parent", err))?,
            notused: row
                .get(2)
                .map_err(|err| DbError::sqlite("读取查询计划 notused", err))?,
            detail: row
                .get(3)
                .map_err(|err| DbError::sqlite("读取查询计划 detail", err))?,
        });
    }
    Ok(plan)
}

fn open_readonly_connection(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        return Err(DbError::OpenFailed {
            path: path.to_path_buf(),
            detail: "文件不存在或不是普通文件".to_owned(),
        });
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|err| DbError::OpenFailed {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    let pragmas = format!(
        "PRAGMA query_only = ON;\
         PRAGMA mmap_size = {READONLY_MMAP_SIZE_BYTES};\
         PRAGMA cache_size = {READONLY_CACHE_SIZE_KIB};\
         PRAGMA temp_store = MEMORY;\
         PRAGMA busy_timeout = 5000;"
    );
    conn.execute_batch(&pragmas)
        .map_err(|err| DbError::OpenFailed {
            path: path.to_path_buf(),
            detail: format!("设置只读 PRAGMA 失败：{err}"),
        })?;
    Ok(conn)
}

fn verify_schema_version(conn: &Connection, path: &Path) -> Result<()> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_KEY_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| DbError::MissingSchemaVersion {
            path: path.to_path_buf(),
            detail: format!("读取 meta 表失败：{err}"),
        })?;

    let Some(version_str) = raw else {
        return Err(DbError::MissingSchemaVersion {
            path: path.to_path_buf(),
            detail: format!("meta 表中缺少键 {META_KEY_SCHEMA_VERSION}"),
        });
    };

    let actual: u32 = version_str
        .trim()
        .parse()
        .map_err(|err| DbError::MissingSchemaVersion {
            path: path.to_path_buf(),
            detail: format!(
                "meta.{META_KEY_SCHEMA_VERSION} 不是合法整数（{version_str:?}）：{err}"
            ),
        })?;

    if actual != SCHEMA_VERSION {
        return Err(DbError::SchemaVersionMismatch {
            path: path.to_path_buf(),
            expected: SCHEMA_VERSION,
            actual,
        });
    }
    Ok(())
}

fn count_user_tables(conn: &Connection) -> Result<Vec<TableRowCount>> {
    let mut stmt = conn
        .prepare_cached(
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
        .map_err(|err| DbError::sqlite("迭代 sqlite_master 行", err))?
    {
        let name: String = row.get(0).map_err(|err| DbError::sqlite("读取表名", err))?;
        let sql: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取建表 SQL", err))?;
        raw_tables.push((name, sql));
    }

    // Collect shadow tables belonging to FTS5 / R*Tree virtual tables so stats stay readable.
    let mut shadow_tables: HashSet<String> = HashSet::new();
    for (name, sql) in &raw_tables {
        if sql
            .trim_start()
            .to_ascii_uppercase()
            .starts_with("CREATE VIRTUAL TABLE")
        {
            for suffix in [
                "_data", "_idx", "_content", "_docsize", "_config", "_node", "_rowid", "_parent",
            ] {
                shadow_tables.insert(format!("{name}{suffix}"));
            }
        }
    }

    let mut counts = Vec::new();
    for (name, _) in raw_tables {
        if shadow_tables.contains(&name) {
            continue;
        }
        let quoted = name.replace('"', "\"\"");
        let count_sql = format!("SELECT COUNT(*) FROM \"{quoted}\"");
        let row_count: i64 = conn
            .query_row(&count_sql, [], |row| row.get(0))
            .map_err(|err| DbError::sqlite(format!("统计表 {name} 行数"), err))?;
        counts.push(TableRowCount {
            name,
            rows: u64::try_from(row_count).unwrap_or(0),
        });
    }
    Ok(counts)
}

fn value_ref_to_json(value: ValueRef<'_>) -> JsonValue {
    match value {
        ValueRef::Null => JsonValue::Null,
        ValueRef::Integer(v) => JsonValue::Number(v.into()),
        ValueRef::Real(v) => JsonNumber::from_f64(v)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        ValueRef::Text(bytes) => JsonValue::String(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => JsonValue::String(format!("0x{}", hex_encode(bytes))),
    }
}

fn is_readonly_error(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ReadOnly
    )
}

fn is_interrupted_error(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::OperationInterrupted
    )
}

struct ProgressTimeoutState {
    deadline: Instant,
    timed_out: AtomicBool,
}

extern "C" fn progress_timeout_callback(ctx: *mut c_void) -> c_int {
    if ctx.is_null() {
        return 0;
    }
    // SAFETY: `ctx` points to a stack-pinned `ProgressTimeoutState` whose lifetime strictly
    // outlives `ProgressHandlerGuard`; `ProgressHandlerGuard::drop` always unregisters this
    // callback before `ProgressTimeoutState` is dropped.
    let state = unsafe { &*(ctx as *const ProgressTimeoutState) };
    if Instant::now() >= state.deadline {
        state.timed_out.store(true, Ordering::Relaxed);
        1
    } else {
        0
    }
}

/// RAII guard registering `sqlite3_progress_handler` for the duration of a single query and
/// unconditionally clearing it on drop before returning the connection to the pool.
struct ProgressHandlerGuard<'a> {
    raw_db: *mut rusqlite::ffi::sqlite3,
    _marker: PhantomData<&'a ProgressTimeoutState>,
}

impl<'a> ProgressHandlerGuard<'a> {
    fn install(conn: &Connection, vm_ops: c_int, state: &'a ProgressTimeoutState) -> Self {
        // SAFETY: `conn` is exclusively borrowed from the pool for the duration of this query,
        // and `state` is borrowed for `'a`, outliving this guard.
        let raw_db = unsafe { conn.handle() };
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                raw_db,
                vm_ops,
                Some(progress_timeout_callback),
                (state as *const ProgressTimeoutState).cast_mut().cast(),
            );
        }
        Self {
            raw_db,
            _marker: PhantomData,
        }
    }
}

impl Drop for ProgressHandlerGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: Unregisters the progress callback while `raw_db` is still exclusively checked out.
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(self.raw_db, 0, None, std::ptr::null_mut());
        }
    }
}
