//! Local SQLite game database (`ffr-db`) for FINAL FANTASY RESONANCE.
//!
//! Packs exported static game data and optional runtime snapshots into a single SQLite file with
//! indexed lookup columns, serialized `ffr-data` JSON documents (`doc`), and relationship tables.
//!
//! Layout:
//! - [`schema`]: embedded table/index DDL (`schema/00_meta.sql` .. `schema/10_snapshots.idx.sql`)
//!   and [`SCHEMA_VERSION`].
//! - [`build`]: staged atomic database builder (`build::build`) and per-domain stage modules.
//! - [`db`]: thread-safe read-only [`GameDb`] connection pool (`Send + Sync`, max 4 connections).
//! - [`record`]: `Serialize` / `Deserialize` helpers for `doc` columns with table + key errors.
//! - [`query`]: per-domain `impl GameDb` query modules.
//! - [`testing`]: test-only query-plan assertions (`assert_uses_index`) and fixture DB builders.

pub mod build;
pub mod db;
pub mod error;
pub mod hash;
pub mod paths;
pub mod query;
pub mod record;
pub mod schema;
pub mod testing;
pub(crate) mod util;

pub use build::{
    BuildInputs, BuildIssue, BuildIssueSummary, BuildManifest, BuildOptions, BuildReport,
    ImportCtx, MANIFEST_FORMAT_VERSION, ManifestInputRecord, ManifestToolInfo, SourceRecord, Stage,
    StageReport, StageStats, StageStatus, build, manifest_path_for,
};
pub use db::{DbStats, GameDb, MAX_POOL_CONNECTIONS, QueryPlanRow, TableRowCount};
pub use error::{DbError, Result};
pub use hash::{
    ContentHashReport, DbHashReport, HASH_FORMAT_VERSION, MANIFEST_VOLATILE_KEYS, ManifestDigest,
    TableHash, canonical_manifest_json, canonical_manifest_sha256, encode_canonical_value,
    hash_connection_content, hash_database, hash_manifest_file,
};
pub use paths::{
    DbPaths, ResolvedBuildPaths, default_db_path_from_deps, default_db_path_from_env,
    resolve_db_path,
};
pub use schema::{
    DB_APPLICATION_ID, DB_PAGE_SIZE, META_KEY_BUILDER_VERSION, META_KEY_GAME_BUILD_ID,
    META_KEY_SCHEMA_VERSION, SCHEMA_PARTS, SCHEMA_VERSION, SchemaPart,
};
