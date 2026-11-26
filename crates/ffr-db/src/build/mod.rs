//! Staged atomic, reproducible builder for `ffr-db` SQLite databases.
//!
//! Build pipeline:
//! 1. Create a temporary SQLite file in the same directory as the target output, fixing
//!    deterministic database header parameters (`page_size = 4096`, `auto_vacuum = NONE`,
//!    `encoding = 'UTF-8'`, `application_id`, `user_version`) and bulk-load PRAGMAs.
//! 2. Execute all table DDL scripts (`00_meta.sql` .. `11_links.sql`).
//! 3. Run every stage in canonical order, each inside its own transaction, inserting rows in
//!    deterministic natural primary-key order.
//! 4. Write deterministic `meta` and `stage_report` rows (no timestamps, timings, or host paths).
//! 5. Execute all index DDL scripts (`00_meta.idx.sql` .. `11_links.idx.sql`).
//! 6. Run `ANALYZE` and `VACUUM`.
//! 7. Close the connection and atomically replace the target output file.
//! 8. Compute file + content SHA-256 hashes and atomically write `<db>.manifest.json`.
//!
//! Any error along the way removes temporary files via drop guards, leaving any previously built
//! target database untouched.

pub mod assets;
pub mod battle;
pub mod datatable;
pub mod input_ui;
pub mod levels;
pub mod links;
pub mod reflection;
pub mod sequences;
pub mod snapshots;
pub mod text;
pub mod world;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ffr_data::DataStore;
use rusqlite::{Connection, OpenFlags, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, Result};
use crate::hash::{DbHashReport, hash_database, hash_manifest_file};
use crate::schema::{
    DB_APPLICATION_ID, DB_PAGE_SIZE, META_KEY_BUILDER_VERSION, META_KEY_GAME_BUILD_ID,
    META_KEY_SCHEMA_VERSION, SCHEMA_VERSION, apply_index_schemas, apply_table_schemas,
};
use crate::util::{path_to_slash_string, utc_now_rfc3339};

/// Format version of the sidecar `<db>.manifest.json` written by [`build`].
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// Environment variable allowing callers to override the worker thread count during build.
pub const BUILD_THREADS_ENV: &str = "FFR_DB_THREADS";

static TEMP_DB_NONCE: AtomicU64 = AtomicU64::new(0);

/// Stage function signature implemented by every domain module under `src/build/`.
pub type StageRunner = fn(&mut ImportCtx<'_>) -> Result<StageStats>;

/// Canonical list of import stages in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Text,
    Datatable,
    Battle,
    World,
    Assets,
    Reflection,
    InputUi,
    Levels,
    Sequences,
    Snapshots,
    Links,
}

impl Stage {
    /// All stages in canonical build order.
    pub const ALL: [Stage; 11] = [
        Self::Text,
        Self::Datatable,
        Self::Battle,
        Self::World,
        Self::Assets,
        Self::Reflection,
        Self::InputUi,
        Self::Levels,
        Self::Sequences,
        Self::Snapshots,
        Self::Links,
    ];

    /// Comma-separated list of valid CLI stage names.
    pub const VALID_NAMES: &'static str = "text, datatable, battle, world, assets, reflection, input_ui, levels, sequences, snapshots, links";

    /// Stable stage identifier written to `stage_report`, `source`, and `build_issue`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Datatable => "datatable",
            Self::Battle => "battle",
            Self::World => "world",
            Self::Assets => "assets",
            Self::Reflection => "reflection",
            Self::InputUi => "input_ui",
            Self::Levels => "levels",
            Self::Sequences => "sequences",
            Self::Snapshots => "snapshots",
            Self::Links => "links",
        }
    }

    /// Parse a stage name supplied on the CLI (`--stages`).
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim() {
            "text" => Ok(Self::Text),
            "datatable" => Ok(Self::Datatable),
            "battle" => Ok(Self::Battle),
            "world" => Ok(Self::World),
            "assets" => Ok(Self::Assets),
            "reflection" => Ok(Self::Reflection),
            "input_ui" => Ok(Self::InputUi),
            "levels" => Ok(Self::Levels),
            "sequences" => Ok(Self::Sequences),
            "snapshots" => Ok(Self::Snapshots),
            "links" => Ok(Self::Links),
            other => Err(DbError::UnknownStage {
                name: other.to_owned(),
                valid: Self::VALID_NAMES,
            }),
        }
    }
}

/// Fixed ordered stage dispatch table. Domain authors only implement their own module file.
pub const STAGES: &[(Stage, StageRunner)] = &[
    (Stage::Text, text::import),
    (Stage::Datatable, datatable::import),
    (Stage::Battle, battle::import),
    (Stage::World, world::import),
    (Stage::Assets, assets::import),
    (Stage::Reflection, reflection::import),
    (Stage::InputUi, input_ui::import),
    (Stage::Levels, levels::import),
    (Stage::Sequences, sequences::import),
    (Stage::Snapshots, snapshots::import),
    (Stage::Links, links::import),
];

/// Input locations supplied to a database build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildInputs {
    /// Core DataTable & localization export directory (`$FFR_DEPS/ffr/export`).
    pub export: Option<PathBuf>,
    /// Full asset export directory (`$FFR_DEPS/ffr/export-full`).
    pub full: Option<PathBuf>,
    /// AssetRegistry inventory directory (`$FFR_DEPS/ffr/inventory`).
    pub inventory: Option<PathBuf>,
    /// Exported level maps directory (`$FFR_DEPS/ffr/export-full/maps`).
    pub maps: Option<PathBuf>,
    /// Explicit `.jmap` reflection dump files (repeatable, never defaulted).
    pub jmaps: Vec<PathBuf>,
    /// Explicit live-game R1 snapshot directory (never defaulted).
    pub snapshots: Option<PathBuf>,
}

/// Full configuration for [`build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    pub inputs: BuildInputs,
    pub output: PathBuf,
    /// Optional subset of stages to execute (`--stages`). `None` runs all stages.
    pub stages: Option<Vec<Stage>>,
    /// Optional explicit game build identifier recorded in `meta.game_build_id`.
    pub game_build_id: Option<String>,
    /// Optional worker thread override for parallel scan stages.
    pub threads: Option<usize>,
}

/// Execution outcome of a single stage recorded in `stage_report.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Ok,
    Skipped,
    Failed,
}

impl StageStatus {
    /// Canonical string stored in `stage_report.status`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// Parse a stored `stage_report.status` string.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "ok" => Some(Self::Ok),
            "skipped" => Some(Self::Skipped),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Statistics returned by a stage's `import(ctx)` function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageStats {
    pub status: StageStatus,
    pub rows: u64,
    pub note: String,
}

impl StageStats {
    /// Stage completed normally and inserted `rows` rows.
    pub fn ok(rows: u64) -> Self {
        Self {
            status: StageStatus::Ok,
            rows,
            note: String::new(),
        }
    }

    /// Stage completed normally with an additional explanatory note.
    pub fn ok_with_note(rows: u64, note: impl Into<String>) -> Self {
        Self {
            status: StageStatus::Ok,
            rows,
            note: note.into(),
        }
    }

    /// Stage was skipped with an explanatory Chinese reason.
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self {
            status: StageStatus::Skipped,
            rows: 0,
            note: reason.into(),
        }
    }
}

/// Persisted row in `source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub stage: String,
    pub path: String,
    pub manifest_sha256: Option<String>,
    pub file_count: Option<u64>,
}

/// Persisted row in `stage_report`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReport {
    pub stage: String,
    pub status: StageStatus,
    pub rows: u64,
    pub millis: u64,
    pub note: String,
}

/// Persisted row in `build_issue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIssue {
    pub stage: String,
    pub kind: String,
    pub subject: String,
    pub detail: String,
}

/// Grouped issue count by `(stage, kind)` printed at the end of a build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIssueSummary {
    pub stage: String,
    pub kind: String,
    pub count: u64,
}

/// Tool and runtime metadata recorded in `<db>.manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestToolInfo {
    pub crate_version: String,
    pub git_commit: Option<String>,
    pub schema_version: u32,
    pub sqlite_version: String,
    pub threads: usize,
    pub total_millis: u64,
}

/// One input source recorded in `<db>.manifest.json`.
///
/// Carries both the canonicalized `manifest_sha256` written into SQLite `source` and the raw
/// on-disk file SHA-256 (`raw_manifest_sha256`), which is never stored inside SQLite so volatile
/// exporter timestamps/timings do not affect database reproducibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestInputRecord {
    pub stage: String,
    pub path: String,
    pub manifest_sha256: Option<String>,
    #[serde(default)]
    pub raw_manifest_sha256: Option<String>,
    pub file_count: Option<u64>,
}

/// Sidecar build manifest written alongside the SQLite database as `<db>.manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildManifest {
    pub format: u32,
    pub built_at: String,
    pub game_build_id: String,
    pub tool: ManifestToolInfo,
    pub inputs: Vec<ManifestInputRecord>,
    pub stages: Vec<StageReport>,
    pub hash: DbHashReport,
}

/// Compute the sidecar manifest path `<db_path>.manifest.json` next to `db_path`.
pub fn manifest_path_for(db_path: &Path) -> PathBuf {
    let mut os = db_path.as_os_str().to_os_string();
    os.push(".manifest.json");
    PathBuf::from(os)
}

/// Summary returned by [`build`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildReport {
    pub output: PathBuf,
    pub manifest_path: PathBuf,
    pub schema_version: u32,
    pub builder_version: String,
    pub built_at: String,
    pub game_build_id: String,
    pub file_sha256: String,
    pub content_sha256: String,
    pub total_millis: u64,
    pub stages: Vec<StageReport>,
    pub issues: Vec<BuildIssueSummary>,
    pub total_issues: u64,
}

/// Context passed into each stage's `pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats>`.
///
/// Owns the active per-stage SQLite transaction, exposes resolved input paths and a shared lazy
/// [`ffr_data::DataStore`], and offers helpers for recording `build_issue` and `source` rows.
pub struct ImportCtx<'conn> {
    stage: Stage,
    tx: Transaction<'conn>,
    inputs: &'conn BuildInputs,
    data_store: Option<&'conn DataStore>,
    worker_threads: usize,
    raw_source_hashes: RefCell<&'conn mut BTreeMap<(String, String), Option<String>>>,
}

impl<'conn> ImportCtx<'conn> {
    fn new(
        stage: Stage,
        tx: Transaction<'conn>,
        inputs: &'conn BuildInputs,
        data_store: Option<&'conn DataStore>,
        worker_threads: usize,
        raw_source_hashes: &'conn mut BTreeMap<(String, String), Option<String>>,
    ) -> Self {
        Self {
            stage,
            tx,
            inputs,
            data_store,
            worker_threads,
            raw_source_hashes: RefCell::new(raw_source_hashes),
        }
    }

    /// Stage currently executing.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// Active SQLite transaction for this stage.
    pub fn tx(&self) -> &Transaction<'conn> {
        &self.tx
    }

    /// Mutable access to the active SQLite transaction (e.g. for savepoints).
    pub fn tx_mut(&mut self) -> &mut Transaction<'conn> {
        &mut self.tx
    }

    /// Configured input directories/files for the current build.
    pub fn inputs(&self) -> &BuildInputs {
        self.inputs
    }

    /// Effective worker thread count for parallel scan stages (always >= 1).
    pub fn worker_threads(&self) -> usize {
        self.worker_threads
    }

    /// Shared lazy [`DataStore`] backed by `inputs.export`, if available.
    pub fn data_store(&self) -> Option<&'conn DataStore> {
        self.data_store
    }

    /// Return the shared [`DataStore`] or fail with a Chinese error naming the current stage.
    pub fn require_data_store(&self) -> Result<&'conn DataStore> {
        self.data_store.ok_or(DbError::MissingDataStore {
            stage: self.stage.as_str(),
        })
    }

    /// Record a non-fatal data integrity issue (such as a dangling reference) in `build_issue`.
    pub fn record_issue(&self, kind: &str, subject: &str, detail: &str) -> Result<()> {
        let mut stmt = self
            .tx
            .prepare_cached(
                "INSERT INTO build_issue (stage, kind, subject, detail) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 build_issue", err))?;
        stmt.execute(params![self.stage.as_str(), kind, subject, detail])
            .map_err(|err| DbError::sqlite("写入 build_issue", err))?;
        Ok(())
    }

    /// Record a raw input manifest/file SHA-256 for the sidecar `<db>.manifest.json` without
    /// writing it into SQLite.
    pub(crate) fn record_raw_source_hash(&self, logical_path: &str, raw_sha256: Option<String>) {
        self.raw_source_hashes.borrow_mut().insert(
            (self.stage.as_str().to_owned(), logical_path.to_owned()),
            raw_sha256,
        );
    }

    /// Record an input source row in `source` using a pre-normalized relative logical path.
    pub fn record_logical_source(
        &self,
        logical_path: &str,
        manifest_sha256: Option<&str>,
        file_count: Option<u64>,
    ) -> Result<()> {
        self.record_raw_source_hash(logical_path, manifest_sha256.map(ToOwned::to_owned));
        record_logical_source_on_conn(
            &self.tx,
            self.stage,
            logical_path,
            manifest_sha256,
            file_count,
        )
    }

    /// Record an input source row in `source`, normalizing `path` to a machine-independent relative
    /// path under the logical input root (`export/...`, `export-full/...`, `inventory/...`,
    /// `export-full/maps`, `jmap/<file>`, `snapshots`).
    pub fn record_source(
        &self,
        path: &Path,
        manifest_sha256: Option<&str>,
        file_count: Option<u64>,
    ) -> Result<()> {
        let logical = normalize_source_path(self.inputs, path);
        self.record_logical_source(&logical, manifest_sha256, file_count)
    }

    /// Record an input directory in `source`, automatically hashing `<dir>/manifest.json` if it
    /// exists (storing its canonicalized JSON SHA-256 in SQLite `source` and its raw file SHA-256
    /// only in `<db>.manifest.json`) and normalizing `dir` to a logical relative path.
    pub fn record_dir_source(&self, dir: &Path, file_count: Option<u64>) -> Result<()> {
        let logical = normalize_source_path(self.inputs, dir);
        let manifest_path = dir.join("manifest.json");
        let (canonical_sha, raw_sha) = if manifest_path.is_file() {
            let digest = hash_manifest_file(&manifest_path)?;
            (Some(digest.canonical_sha256), Some(digest.raw_sha256))
        } else {
            (None, None)
        };
        self.record_raw_source_hash(&logical, raw_sha);
        record_logical_source_on_conn(
            &self.tx,
            self.stage,
            &logical,
            canonical_sha.as_deref(),
            file_count,
        )
    }

    fn commit(self) -> Result<()> {
        self.tx
            .commit()
            .map_err(|err| DbError::sqlite(format!("提交阶段 {} 事务", self.stage.as_str()), err))
    }
}

pub(crate) fn record_logical_source_on_conn(
    conn: &Connection,
    stage: Stage,
    logical_path: &str,
    manifest_sha256: Option<&str>,
    file_count: Option<u64>,
) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(
            "INSERT OR REPLACE INTO source (stage, path, manifest_sha256, file_count) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|err| DbError::sqlite("准备写入 source", err))?;
    let count_i64 = file_count.and_then(|v| i64::try_from(v).ok());
    stmt.execute(params![
        stage.as_str(),
        logical_path,
        manifest_sha256,
        count_i64
    ])
    .map_err(|err| DbError::sqlite("写入 source", err))?;
    Ok(())
}

/// Compute machine-independent display labels for a slice of `--jmap` paths.
///
/// Unique file names keep `<filename>`; duplicate file names are disambiguated in CLI order as
/// `<filename>#1`, `<filename>#2`, etc.
pub(crate) fn jmap_display_labels(jmaps: &[PathBuf]) -> Vec<String> {
    let base_names: Vec<String> = jmaps
        .iter()
        .map(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown.jmap")
                .to_owned()
        })
        .collect();
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    for name in &base_names {
        *totals.entry(name.as_str()).or_insert(0) += 1;
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    base_names
        .iter()
        .map(|name| {
            if totals.get(name.as_str()).copied().unwrap_or(0) <= 1 {
                name.clone()
            } else {
                let occ = seen.entry(name.as_str()).or_insert(0);
                *occ += 1;
                format!("{name}#{occ}")
            }
        })
        .collect()
}

/// Compute normalized `source.path` strings (`jmap/<label>`) for `--jmap` inputs.
pub(crate) fn jmap_logical_paths(jmaps: &[PathBuf]) -> Vec<String> {
    jmap_display_labels(jmaps)
        .into_iter()
        .map(|label| format!("jmap/{label}"))
        .collect()
}

/// Normalize an input path to a relative, slash-separated logical path for `source.path`.
pub(crate) fn normalize_source_path(inputs: &BuildInputs, path: &Path) -> String {
    if let Some(logical) = strip_logical_root(path, inputs.maps.as_deref(), "export-full/maps") {
        return logical;
    }
    if let Some(logical) = strip_logical_root(path, inputs.export.as_deref(), "export") {
        return logical;
    }
    if let Some(logical) = strip_logical_root(path, inputs.full.as_deref(), "export-full") {
        return logical;
    }
    if let Some(logical) = strip_logical_root(path, inputs.inventory.as_deref(), "inventory") {
        return logical;
    }
    if let Some(logical) = strip_logical_root(path, inputs.snapshots.as_deref(), "snapshots") {
        return logical;
    }
    if let Some(idx) = inputs.jmaps.iter().position(|p| p == path) {
        return jmap_logical_paths(&inputs.jmaps)[idx].clone();
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn strip_logical_root(path: &Path, root: Option<&Path>, prefix: &str) -> Option<String> {
    let root = root?;
    let rel = path.strip_prefix(root).ok()?;
    let rel_str = path_to_slash_string(rel);
    let trimmed = rel_str.trim_matches('/');
    if trimmed.is_empty() {
        Some(prefix.to_owned())
    } else {
        Some(format!("{prefix}/{trimmed}"))
    }
}

/// Format `path` relative to `root` using `/` separators, or fall back to its file name so
/// host-specific absolute paths never leak into database columns.
pub(crate) fn relative_label_under(root: &Path, prefix: &str, path: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(root) {
        let rel_str = path_to_slash_string(rel);
        let trimmed = rel_str.trim_matches('/');
        if trimmed.is_empty() {
            return prefix.to_owned();
        }
        return format!("{prefix}/{trimmed}");
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(|name| format!("{prefix}/{name}"))
        .unwrap_or_else(|| prefix.to_owned())
}

/// Format per-language untranslated row counts for one companion table deterministically for
/// `stage_report.note`, e.g. `skill_untranslated(en=448, zh-Hans=757)`.
pub(crate) fn format_untranslated_table_counts(
    table: &str,
    non_native_langs: &[String],
    counts: &BTreeMap<String, u64>,
) -> String {
    let mut keys: BTreeSet<&str> = non_native_langs.iter().map(String::as_str).collect();
    for lang in counts.keys() {
        keys.insert(lang.as_str());
    }
    if keys.is_empty() {
        return format!("{table}(0)");
    }
    let parts: Vec<String> = keys
        .into_iter()
        .map(|lang| {
            let count = counts.get(lang).copied().unwrap_or(0);
            format!("{lang}={count}")
        })
        .collect();
    format!("{table}({})", parts.join(", "))
}

fn resolve_worker_threads(explicit: Option<usize>) -> usize {
    if let Some(n) = explicit {
        return n.max(1);
    }
    if let Ok(raw) = std::env::var(BUILD_THREADS_ENV)
        && let Ok(parsed) = raw.trim().parse::<usize>()
        && parsed >= 1
    {
        return parsed;
    }
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(8)
}

/// Build an `ffr-db` SQLite database atomically using all canonical stages.
pub fn build(options: &BuildOptions) -> Result<BuildReport> {
    build_with_stages(options, STAGES)
}

pub(crate) fn build_with_stages(
    options: &BuildOptions,
    stage_runners: &[(Stage, StageRunner)],
) -> Result<BuildReport> {
    let build_start = Instant::now();
    let worker_threads = resolve_worker_threads(options.threads);
    let selected: Option<BTreeSet<Stage>> = options
        .stages
        .as_ref()
        .map(|list| list.iter().copied().collect());

    let parent_dir = options
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent_dir).map_err(|err| DbError::io("创建输出目录", parent_dir, err))?;

    let temp_path = make_temp_db_path(parent_dir, &options.output);
    let mut temp_guard = TempDbGuard::new(temp_path.clone());

    // Step 1: open temporary SQLite file with fixed reproducible parameters and bulk-load PRAGMAs.
    let mut conn = open_build_connection(&temp_path)?;

    // Step 2: apply all table schemas.
    apply_table_schemas(&conn)?;

    // Shared lazy DataStore for export-backed stages.
    let data_store = options
        .inputs
        .export
        .as_ref()
        .filter(|dir| dir.is_dir())
        .map(DataStore::new);

    // Step 3: execute each stage in canonical order, each in its own transaction.
    let mut stage_reports = Vec::with_capacity(stage_runners.len());
    let mut raw_source_hashes: BTreeMap<(String, String), Option<String>> = BTreeMap::new();
    for &(stage, runner) in stage_runners {
        let stage_start = Instant::now();
        if let Some(allowed) = &selected
            && !allowed.contains(&stage)
        {
            stage_reports.push(StageReport {
                stage: stage.as_str().to_owned(),
                status: StageStatus::Skipped,
                rows: 0,
                millis: stage_start.elapsed().as_millis() as u64,
                note: "未在 --stages 中选中".to_owned(),
            });
            continue;
        }

        if let Some(reason) = missing_input_reason(stage, &options.inputs) {
            stage_reports.push(StageReport {
                stage: stage.as_str().to_owned(),
                status: StageStatus::Skipped,
                rows: 0,
                millis: stage_start.elapsed().as_millis() as u64,
                note: reason,
            });
            continue;
        }

        let tx = conn
            .transaction()
            .map_err(|err| DbError::sqlite(format!("开启阶段 {} 事务", stage.as_str()), err))?;
        let mut ctx = ImportCtx::new(
            stage,
            tx,
            &options.inputs,
            data_store.as_ref(),
            worker_threads,
            &mut raw_source_hashes,
        );
        let stats = runner(&mut ctx).map_err(|err| DbError::StageFailed {
            stage: stage.as_str(),
            detail: err.to_string(),
        })?;
        ctx.commit()?;

        stage_reports.push(StageReport {
            stage: stage.as_str().to_owned(),
            status: stats.status,
            rows: stats.rows,
            millis: stage_start.elapsed().as_millis() as u64,
            note: stats.note,
        });
    }

    // Step 4: write deterministic `meta` and `stage_report` rows before indexing & VACUUM.
    let builder_version = env!("CARGO_PKG_VERSION").to_owned();
    let game_build_id = resolve_game_build_id(options);
    write_meta_and_reports(&mut conn, &builder_version, &game_build_id, &stage_reports)?;

    // Step 5: create all secondary indexes after bulk loading.
    apply_index_schemas(&conn)?;

    // Step 6: ANALYZE and VACUUM for deterministic layout and query planner statistics.
    conn.execute_batch("ANALYZE; VACUUM;")
        .map_err(|err| DbError::sqlite("执行 ANALYZE 与 VACUUM", err))?;

    let sources: Vec<ManifestInputRecord> = read_sources_sorted(&conn)?
        .into_iter()
        .map(|src| {
            let raw_manifest_sha256 = raw_source_hashes
                .get(&(src.stage.clone(), src.path.clone()))
                .cloned()
                .flatten();
            ManifestInputRecord {
                stage: src.stage,
                path: src.path,
                manifest_sha256: src.manifest_sha256,
                raw_manifest_sha256,
                file_count: src.file_count,
            }
        })
        .collect();
    let (issues, total_issues) = summarize_build_issues(&conn)?;

    // Step 7: close the connection cleanly before atomically replacing the target file.
    conn.close()
        .map_err(|(_, err)| DbError::sqlite("关闭临时数据库连接", err))?;

    // On Windows a file that is still open (for example by a running `ffr-module` or another
    // `GameDb`) cannot be replaced; say so instead of surfacing a bare "Access is denied".
    fs::rename(&temp_path, &options.output).map_err(|err| {
        let action = if err.kind() == std::io::ErrorKind::PermissionDenied {
            "替换目标数据库文件（被拒绝：目标库可能正被其他程序打开，例如正在运行的 ffr-module；\
             Windows 下无法替换正在使用的文件，请先关闭它，或换一个 --out 路径）"
        } else {
            "替换目标数据库文件"
        };
        DbError::io(action, &options.output, err)
    })?;
    temp_guard.disarm();

    // Step 8: compute whole-file and canonical content hashes and write `<db>.manifest.json`.
    let built_at = utc_now_rfc3339();
    let hash_report = hash_database(&options.output)?;
    let total_millis = build_start.elapsed().as_millis() as u64;
    let manifest_path = manifest_path_for(&options.output);
    let manifest = BuildManifest {
        format: MANIFEST_FORMAT_VERSION,
        built_at: built_at.clone(),
        game_build_id: game_build_id.clone(),
        tool: ManifestToolInfo {
            crate_version: builder_version.clone(),
            git_commit: option_env!("FFR_DB_GIT_COMMIT").map(ToOwned::to_owned),
            schema_version: SCHEMA_VERSION,
            sqlite_version: hash_report.sqlite_version.clone(),
            threads: worker_threads,
            total_millis,
        },
        inputs: sources,
        stages: stage_reports.clone(),
        hash: hash_report.clone(),
    };
    write_manifest_atomic(&manifest_path, &manifest)?;

    Ok(BuildReport {
        output: options.output.clone(),
        manifest_path,
        schema_version: SCHEMA_VERSION,
        builder_version,
        built_at,
        game_build_id,
        file_sha256: hash_report.file_sha256,
        content_sha256: hash_report.content_sha256,
        total_millis,
        stages: stage_reports,
        issues,
        total_issues,
    })
}

fn open_build_connection(path: &Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|err| DbError::sqlite(format!("创建临时数据库 {}", path.display()), err))?;
    let pragmas = format!(
        "PRAGMA page_size = {DB_PAGE_SIZE};\
         PRAGMA auto_vacuum = NONE;\
         PRAGMA encoding = 'UTF-8';\
         PRAGMA application_id = {DB_APPLICATION_ID};\
         PRAGMA user_version = {SCHEMA_VERSION};\
         PRAGMA journal_mode = OFF;\
         PRAGMA synchronous = OFF;\
         PRAGMA temp_store = MEMORY;\
         PRAGMA foreign_keys = OFF;"
    );
    conn.execute_batch(&pragmas)
        .map_err(|err| DbError::sqlite("配置构建期 PRAGMA", err))?;
    Ok(conn)
}

fn missing_input_reason(stage: Stage, inputs: &BuildInputs) -> Option<String> {
    match stage {
        Stage::Text | Stage::Datatable | Stage::Battle | Stage::World => {
            check_required_dir(inputs.export.as_deref(), "--export")
        }
        Stage::Assets => match (&inputs.inventory, &inputs.full) {
            (None, None) => Some("未指定资源目录（--inventory / --full）".to_owned()),
            (inv, full) => {
                let inv_ok = inv.as_deref().is_some_and(Path::is_dir);
                let full_ok = full.as_deref().is_some_and(Path::is_dir);
                if inv_ok || full_ok {
                    None
                } else {
                    let mut missing = Vec::new();
                    if inv.is_some() {
                        missing.push("--inventory");
                    }
                    if full.is_some() {
                        missing.push("--full");
                    }
                    Some(format!("输入目录不存在（{}）", missing.join(" / ")))
                }
            }
        },
        Stage::Reflection => {
            if inputs.jmaps.is_empty() {
                Some("未指定 --jmap 反射文件".to_owned())
            } else {
                let logical_paths = jmap_logical_paths(&inputs.jmaps);
                let missing: Vec<String> = inputs
                    .jmaps
                    .iter()
                    .zip(logical_paths)
                    .filter(|(path, _)| !path.is_file())
                    .map(|(_, logical)| logical)
                    .collect();
                if missing.is_empty() {
                    None
                } else {
                    Some(format!("输入文件不存在：{}", missing.join("、")))
                }
            }
        }
        Stage::InputUi | Stage::Sequences => check_required_dir(inputs.full.as_deref(), "--full"),
        Stage::Levels => check_required_dir(inputs.maps.as_deref(), "--maps"),
        Stage::Snapshots => match inputs.snapshots.as_deref() {
            None => Some("未指定 --snapshots 快照目录".to_owned()),
            Some(path) if !path.is_dir() => Some("输入目录不存在（--snapshots）".to_owned()),
            Some(_) => None,
        },
        Stage::Links => None,
    }
}

fn check_required_dir(path: Option<&Path>, flag: &str) -> Option<String> {
    match path {
        None => Some(format!("未指定输入目录（{flag}）")),
        Some(dir) if !dir.is_dir() => Some(format!("输入目录不存在（{flag}）")),
        Some(_) => None,
    }
}

fn resolve_game_build_id(options: &BuildOptions) -> String {
    if let Some(explicit) = &options.game_build_id {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    for dir in [
        options.inputs.export.as_deref(),
        options.inputs.full.as_deref(),
        options.inputs.inventory.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(found) = detect_build_id_from_manifest(&dir.join("manifest.json")) {
            return found;
        }
    }
    "unknown".to_owned()
}

fn detect_build_id_from_manifest(manifest_path: &Path) -> Option<String> {
    let raw = fs::read_to_string(manifest_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let obj = value.as_object()?;
    for key in ["gameBuildId", "game_build_id", "buildId", "buildid"] {
        if let Some(val) = obj.get(key) {
            if let Some(s) = val.as_str()
                && !s.trim().is_empty()
            {
                return Some(s.trim().to_owned());
            }
            if let Some(n) = val.as_u64() {
                return Some(n.to_string());
            }
        }
    }
    None
}

fn write_meta_and_reports(
    conn: &mut Connection,
    builder_version: &str,
    game_build_id: &str,
    reports: &[StageReport],
) -> Result<()> {
    let tx = conn
        .transaction()
        .map_err(|err| DbError::sqlite("开启写入 meta 与 stage_report 事务", err))?;

    {
        let mut meta_stmt = tx
            .prepare_cached("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)")
            .map_err(|err| DbError::sqlite("准备写入 meta", err))?;
        let schema_str = SCHEMA_VERSION.to_string();
        // Insert in natural PRIMARY KEY (`key`) order.
        for (key, value) in [
            (META_KEY_BUILDER_VERSION, builder_version),
            (META_KEY_GAME_BUILD_ID, game_build_id),
            (META_KEY_SCHEMA_VERSION, schema_str.as_str()),
        ] {
            meta_stmt
                .execute(params![key, value])
                .map_err(|err| DbError::sqlite(format!("写入 meta.{key}"), err))?;
        }

        let mut report_stmt = tx
            .prepare_cached(
                "INSERT OR REPLACE INTO stage_report (stage, status, rows, note) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 stage_report", err))?;
        let mut sorted_reports: Vec<&StageReport> = reports.iter().collect();
        sorted_reports.sort_by(|a, b| a.stage.cmp(&b.stage));
        for report in sorted_reports {
            report_stmt
                .execute(params![
                    report.stage,
                    report.status.as_str(),
                    i64::try_from(report.rows).unwrap_or(i64::MAX),
                    report.note,
                ])
                .map_err(|err| {
                    DbError::sqlite(format!("写入阶段 {} 的 stage_report", report.stage), err)
                })?;
        }
    }

    tx.commit()
        .map_err(|err| DbError::sqlite("提交 meta 与 stage_report 事务", err))?;
    Ok(())
}

fn read_sources_sorted(conn: &Connection) -> Result<Vec<SourceRecord>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT stage, path, manifest_sha256, file_count FROM source ORDER BY stage, path",
        )
        .map_err(|err| DbError::sqlite("准备读取 source 表", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("读取 source 表", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 source 表", err))?
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
}

fn write_manifest_atomic(manifest_path: &Path, manifest: &BuildManifest) -> Result<()> {
    let parent_dir = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temp_path = make_temp_db_path(parent_dir, manifest_path);
    let mut temp_guard = TempDbGuard::new(temp_path.clone());

    let mut payload =
        serde_json::to_string_pretty(manifest).map_err(|source| DbError::DocSerialize {
            table: "build_manifest".to_owned(),
            key: manifest_path.display().to_string(),
            source,
        })?;
    payload.push('\n');

    fs::write(&temp_path, payload.as_bytes())
        .map_err(|err| DbError::io("写入临时构建清单", &temp_path, err))?;
    fs::rename(&temp_path, manifest_path)
        .map_err(|err| DbError::io("替换目标构建清单", manifest_path, err))?;
    temp_guard.disarm();
    Ok(())
}

fn summarize_build_issues(conn: &Connection) -> Result<(Vec<BuildIssueSummary>, u64)> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT stage, kind, COUNT(*) FROM build_issue \
             GROUP BY stage, kind ORDER BY stage, kind",
        )
        .map_err(|err| DbError::sqlite("准备汇总 build_issue", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("汇总 build_issue", err))?;
    let mut summaries = Vec::new();
    let mut total: u64 = 0;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 build_issue 汇总行", err))?
    {
        let stage: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 build_issue 汇总 stage", err))?;
        let kind: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 build_issue 汇总 kind", err))?;
        let count_i64: i64 = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 build_issue 汇总 count", err))?;
        let count = u64::try_from(count_i64).unwrap_or(0);
        total = total.saturating_add(count);
        summaries.push(BuildIssueSummary { stage, kind, count });
    }
    Ok((summaries, total))
}

fn make_temp_db_path(parent_dir: &Path, output: &Path) -> PathBuf {
    let file_name = output
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ffr.sqlite");
    let pid = std::process::id();
    let nonce = TEMP_DB_NONCE.fetch_add(1, Ordering::Relaxed);
    parent_dir.join(format!(".{file_name}.tmp.{pid}.{nonce}"))
}

struct TempDbGuard {
    path: PathBuf,
    armed: bool,
}

impl TempDbGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempDbGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = fs::remove_file(&self.path);
        for suffix in ["-journal", "-wal", "-shm"] {
            let mut sidecar = self.path.as_os_str().to_os_string();
            sidecar.push(suffix);
            let _ = fs::remove_file(PathBuf::from(sidecar));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::GameDb;

    #[test]
    fn failed_build_cleans_up_temp_file_and_preserves_existing_target() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let export_dir = dir.path().join("export");
        fs::create_dir_all(&export_dir).expect("create fake export dir");
        let out_path = dir.path().join("ffr.sqlite");

        // Build a valid initial database first.
        let initial = BuildOptions {
            inputs: BuildInputs::default(),
            output: out_path.clone(),
            stages: None,
            game_build_id: Some("initial-build".to_owned()),
            threads: None,
        };
        build(&initial).expect("initial build must succeed");
        let original_bytes = fs::read(&out_path).expect("read original db");

        // Now run a build whose second stage fails mid-way.
        fn ok_stage(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
            ctx.record_issue("test_issue", "subject_1", "detail_1")?;
            Ok(StageStats::ok(1))
        }
        fn failing_stage(_ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
            Err(DbError::StageFailed {
                stage: "datatable",
                detail: "模拟阶段中途失败".to_owned(),
            })
        }

        let failing_opts = BuildOptions {
            inputs: BuildInputs {
                export: Some(export_dir),
                ..BuildInputs::default()
            },
            output: out_path.clone(),
            stages: None,
            game_build_id: Some("new-build".to_owned()),
            threads: None,
        };
        let err = build_with_stages(
            &failing_opts,
            &[(Stage::Text, ok_stage), (Stage::Datatable, failing_stage)],
        )
        .expect_err("mid-build failure must return Err");
        assert!(
            err.to_string().contains("模拟阶段中途失败"),
            "unexpected error: {err}"
        );

        // Existing target file must be byte-for-byte unchanged and readable.
        assert_eq!(fs::read(&out_path).expect("re-read db"), original_bytes);
        let db = GameDb::open(&out_path).expect("existing db still opens cleanly");
        assert_eq!(
            db.meta().expect("meta").get(META_KEY_GAME_BUILD_ID),
            Some(&"initial-build".to_owned())
        );

        // Temporary database file must have been removed.
        let leftover_temps: Vec<_> = fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftover_temps.is_empty(),
            "leftover temp files found: {leftover_temps:?}"
        );
    }
}
