//! Test-only helpers for `ffr-db` and downstream crate tests.
//!
//! **For testing only**: do not call helpers in this module from production code paths.
//! Provides:
//! - [`assert_uses_index`] / [`check_uses_index`]: inspect `EXPLAIN QUERY PLAN` and fail if a
//!   query triggers a full table scan (or transient automatic index scan) on specified large tables.
//! - [`build_fixture_db`] / [`build_fixture_db_stages`]: build a temporary SQLite database from
//!   `tests/fixtures/rust/db/<area>/` and open it with [`GameDb`].

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{Connection, Params};

use crate::build::{BuildInputs, BuildOptions, BuildReport, Stage, build};
use crate::db::{GameDb, QueryPlanRow, explain_query_plan_on_conn};
use crate::error::{DbError, Result};

static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Return the repository fixture directory `tests/fixtures/rust/db/<area>` for an area name.
///
/// **For testing only.**
pub fn fixture_area_dir(area: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests")
        .join("fixtures")
        .join("rust")
        .join("db")
        .join(area)
}

/// Owned temporary database built from a fixture directory; deleted automatically on drop.
///
/// **For testing only.**
pub struct TestDb {
    temp_dir: PathBuf,
    db_path: PathBuf,
    db: GameDb,
    report: BuildReport,
}

impl TestDb {
    /// Read-only [`GameDb`] handle opened on the built fixture database.
    pub fn db(&self) -> &GameDb {
        &self.db
    }

    /// Path to the temporary SQLite file.
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Build report produced when constructing this test database.
    pub fn report(&self) -> &BuildReport {
        &self.report
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

/// Build a temporary database in `std::env::temp_dir()` from `tests/fixtures/rust/db/<area>/` and open
/// it read-only.
///
/// Auto-discovers conventional subdirectories under the fixture directory (`export`, `export-full`,
/// `inventory`, `maps`, `snapshots`, and any `*.jmap` file).
///
/// **For testing only.**
pub fn build_fixture_db(area: &str) -> Result<TestDb> {
    build_fixture_db_stages(area, None)
}

/// Same as [`build_fixture_db`], optionally restricting the build to selected `stages`.
///
/// **For testing only.**
pub fn build_fixture_db_stages(area: &str, stages: Option<Vec<Stage>>) -> Result<TestDb> {
    let fixture_dir = fixture_area_dir(area);
    let inputs = discover_fixture_inputs(&fixture_dir)?;
    build_temp_db(area, inputs, stages)
}

/// Build a temporary database from explicit [`BuildInputs`] and open it read-only.
///
/// **For testing only.**
pub fn build_temp_db(
    label: &str,
    inputs: BuildInputs,
    stages: Option<Vec<Stage>>,
) -> Result<TestDb> {
    let temp_dir = create_temp_work_dir(label)?;
    let db_path = temp_dir.join("ffr.sqlite");
    let options = BuildOptions {
        inputs,
        output: db_path.clone(),
        stages,
        game_build_id: None,
        threads: None,
    };
    let report = match build(&options) {
        Ok(report) => report,
        Err(err) => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(err);
        }
    };
    let db = match GameDb::open(&db_path) {
        Ok(db) => db,
        Err(err) => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(err);
        }
    };
    Ok(TestDb {
        temp_dir,
        db_path,
        db,
        report,
    })
}

/// Check via `EXPLAIN QUERY PLAN` that `sql` does not perform a full table scan on any table
/// listed in `disallowed_full_scan_tables`.
///
/// Returns the full [`QueryPlanRow`] list on success, or [`DbError::FullTableScan`] if a full
/// table scan (or transient `USING AUTOMATIC` index scan) is detected on any listed table.
///
/// **For testing only.**
pub fn check_uses_index<P: Params>(
    conn: &Connection,
    sql: &str,
    params: P,
    disallowed_full_scan_tables: &[&str],
) -> Result<Vec<QueryPlanRow>> {
    let plan = explain_query_plan_on_conn(conn, sql, params)?;
    for row in &plan {
        for &table in disallowed_full_scan_tables {
            if is_full_table_scan_on(&row.detail, table) {
                let summary = plan
                    .iter()
                    .map(|p| p.detail.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                return Err(DbError::FullTableScan {
                    table: table.to_owned(),
                    plan: summary,
                    sql: sql.to_owned(),
                });
            }
        }
    }
    Ok(plan)
}

/// Assert via `EXPLAIN QUERY PLAN` that `sql` does not perform a full table scan on any table
/// listed in `disallowed_full_scan_tables`. Panics with the query plan if a full scan occurs.
///
/// **For testing only.**
pub fn assert_uses_index<P: Params>(
    conn: &Connection,
    sql: &str,
    params: P,
    disallowed_full_scan_tables: &[&str],
) {
    if let Err(err) = check_uses_index(conn, sql, params, disallowed_full_scan_tables) {
        panic!("{err}");
    }
}

/// Convenience wrapper around [`assert_uses_index`] operating directly on a [`GameDb`].
///
/// **For testing only.**
pub fn assert_db_uses_index<P: Params>(
    db: &GameDb,
    sql: &str,
    params: P,
    disallowed_full_scan_tables: &[&str],
) {
    let res = db.with_conn(|conn| check_uses_index(conn, sql, params, disallowed_full_scan_tables));
    if let Err(err) = res {
        panic!("{err}");
    }
}

/// Inspect one `EXPLAIN QUERY PLAN` detail line and report whether it represents a full table
/// scan (or transient automatic index scan) on `table`.
///
/// **For testing only.**
pub fn is_full_table_scan_on(detail: &str, table: &str) -> bool {
    let normalized = detail.trim();
    let upper = normalized.to_ascii_uppercase();

    // Strip leading `SCAN [TABLE] ` or `SEARCH [TABLE] `.
    let (is_scan, rest_start) = if upper.starts_with("SCAN TABLE ") {
        (true, "SCAN TABLE ".len())
    } else if upper.starts_with("SCAN ") {
        (true, "SCAN ".len())
    } else if upper.starts_with("SEARCH TABLE ") {
        (false, "SEARCH TABLE ".len())
    } else if upper.starts_with("SEARCH ") {
        (false, "SEARCH ".len())
    } else {
        return false;
    };

    let rest = normalized[rest_start..].trim_start();
    let Some((raw_ident, after_ident)) = split_first_token(rest) else {
        return false;
    };
    let scanned_table = unquote_ident(raw_ident);
    if !scanned_table.eq_ignore_ascii_case(table) {
        return false;
    }

    let after_upper = after_ident.to_ascii_uppercase();
    // Transient automatic indexes still require scanning the whole source table.
    if after_upper.contains("USING AUTOMATIC") {
        return true;
    }
    if !is_scan {
        return false;
    }

    // Virtual table indexed scans (`SCAN vtab VIRTUAL TABLE INDEX ...`) and explicit index scans
    // (`USING INDEX`, `USING COVERING INDEX`, `USING INTEGER PRIMARY KEY`, `USING PRIMARY KEY`)
    // are not full table scans.
    let uses_explicit_index = after_upper.contains("USING INDEX")
        || after_upper.contains("USING COVERING INDEX")
        || after_upper.contains("USING INTEGER PRIMARY KEY")
        || after_upper.contains("USING PRIMARY KEY")
        || after_upper.contains("VIRTUAL TABLE INDEX");
    !uses_explicit_index
}

fn split_first_token(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.find(char::is_whitespace) {
        Some(pos) => Some((&trimmed[..pos], &trimmed[pos..])),
        None => Some((trimmed, "")),
    }
}

fn unquote_ident(token: &str) -> &str {
    let bytes = token.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'`' && bytes[bytes.len() - 1] == b'`')
            || (bytes[0] == b'[' && bytes[bytes.len() - 1] == b']'))
    {
        &token[1..bytes.len() - 1]
    } else {
        token
    }
}

fn discover_fixture_inputs(fixture_dir: &Path) -> Result<BuildInputs> {
    if !fixture_dir.is_dir() {
        return Ok(BuildInputs::default());
    }

    let export = if fixture_dir.join("export").is_dir() {
        Some(fixture_dir.join("export"))
    } else if fixture_dir.join("FFRS").is_dir() || fixture_dir.join("Localization").is_dir() {
        Some(fixture_dir.to_path_buf())
    } else {
        None
    };

    let full = if fixture_dir.join("export-full").is_dir() {
        Some(fixture_dir.join("export-full"))
    } else if fixture_dir.join("full").is_dir() {
        Some(fixture_dir.join("full"))
    } else {
        None
    };

    let inventory = if fixture_dir.join("inventory").is_dir() {
        Some(fixture_dir.join("inventory"))
    } else {
        None
    };

    let maps = if fixture_dir.join("maps").is_dir() {
        Some(fixture_dir.join("maps"))
    } else if fixture_dir.join("export-full").join("maps").is_dir() {
        Some(fixture_dir.join("export-full").join("maps"))
    } else {
        None
    };

    let jmaps = find_jmap_files(fixture_dir)?;

    let snapshots = if fixture_dir.join("snapshots").is_dir() {
        Some(fixture_dir.join("snapshots"))
    } else {
        None
    };

    Ok(BuildInputs {
        export,
        full,
        inventory,
        maps,
        jmaps,
        snapshots,
    })
}

fn find_jmap_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).map_err(|err| DbError::io("读取测试夹具目录", dir, err))?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("遍历测试夹具目录", dir, err))?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jmap"))
        {
            candidates.push(path);
        }
    }
    candidates.sort();
    Ok(candidates)
}

fn create_temp_work_dir(label: &str) -> Result<PathBuf> {
    let safe_label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let seq = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("ffr-db-test-{safe_label}-{pid}-{seq}"));
    fs::create_dir_all(&dir).map_err(|err| DbError::io("创建临时测试目录", &dir, err))?;
    Ok(dir)
}
