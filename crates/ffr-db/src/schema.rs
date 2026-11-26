//! Embedded SQLite table and index schemas for every `ffr-db` area.
//!
//! Each area owns two SQL files under `crates/ffr-db/schema/`:
//! - `NN_<area>.sql`: table definitions executed before data import.
//! - `NN_<area>.idx.sql`: index definitions executed after bulk import.

use rusqlite::Connection;

use crate::error::{DbError, Result};

/// Current database schema version stored in `meta(key = 'schema_version')` and
/// `PRAGMA user_version`.
///
/// Increment whenever any table or index schema changes in an incompatible way.
pub const SCHEMA_VERSION: u32 = 3;

/// Fixed SQLite page size (bytes) applied before creating any tables for reproducible builds.
pub const DB_PAGE_SIZE: u32 = 4096;

/// Fixed SQLite `PRAGMA application_id` (`"FFRD"` in ASCII big-endian = `0x46465244`).
pub const DB_APPLICATION_ID: u32 = 0x4646_5244;

/// `meta` key for the schema version integer.
pub const META_KEY_SCHEMA_VERSION: &str = "schema_version";
/// `meta` key for the `ffr-db` crate version that built the file.
pub const META_KEY_BUILDER_VERSION: &str = "builder_version";
/// `meta` key for the Steam/game build identifier recorded during build.
pub const META_KEY_GAME_BUILD_ID: &str = "game_build_id";

/// One numbered schema unit embedded at compile time via `include_str!`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaPart {
    /// Area identifier (e.g. `"00_meta"`, `"01_text"`).
    pub name: &'static str,
    /// Table creation DDL (`schema/<name>.sql`).
    pub tables_sql: &'static str,
    /// Post-import index creation DDL (`schema/<name>.idx.sql`).
    pub indexes_sql: &'static str,
}

/// Ordered list of all schema parts from `00_meta` through `11_links`.
pub const SCHEMA_PARTS: &[SchemaPart] = &[
    SchemaPart {
        name: "00_meta",
        tables_sql: include_str!("../schema/00_meta.sql"),
        indexes_sql: include_str!("../schema/00_meta.idx.sql"),
    },
    SchemaPart {
        name: "01_text",
        tables_sql: include_str!("../schema/01_text.sql"),
        indexes_sql: include_str!("../schema/01_text.idx.sql"),
    },
    SchemaPart {
        name: "02_datatable",
        tables_sql: include_str!("../schema/02_datatable.sql"),
        indexes_sql: include_str!("../schema/02_datatable.idx.sql"),
    },
    SchemaPart {
        name: "03_battle",
        tables_sql: include_str!("../schema/03_battle.sql"),
        indexes_sql: include_str!("../schema/03_battle.idx.sql"),
    },
    SchemaPart {
        name: "04_world",
        tables_sql: include_str!("../schema/04_world.sql"),
        indexes_sql: include_str!("../schema/04_world.idx.sql"),
    },
    SchemaPart {
        name: "05_assets",
        tables_sql: include_str!("../schema/05_assets.sql"),
        indexes_sql: include_str!("../schema/05_assets.idx.sql"),
    },
    SchemaPart {
        name: "06_reflection",
        tables_sql: include_str!("../schema/06_reflection.sql"),
        indexes_sql: include_str!("../schema/06_reflection.idx.sql"),
    },
    SchemaPart {
        name: "07_input_ui",
        tables_sql: include_str!("../schema/07_input_ui.sql"),
        indexes_sql: include_str!("../schema/07_input_ui.idx.sql"),
    },
    SchemaPart {
        name: "08_levels",
        tables_sql: include_str!("../schema/08_levels.sql"),
        indexes_sql: include_str!("../schema/08_levels.idx.sql"),
    },
    SchemaPart {
        name: "09_sequences",
        tables_sql: include_str!("../schema/09_sequences.sql"),
        indexes_sql: include_str!("../schema/09_sequences.idx.sql"),
    },
    SchemaPart {
        name: "10_snapshots",
        tables_sql: include_str!("../schema/10_snapshots.sql"),
        indexes_sql: include_str!("../schema/10_snapshots.idx.sql"),
    },
    SchemaPart {
        name: "11_links",
        tables_sql: include_str!("../schema/11_links.sql"),
        indexes_sql: include_str!("../schema/11_links.idx.sql"),
    },
];

/// Execute all table creation DDL scripts in canonical order.
pub fn apply_table_schemas(conn: &Connection) -> Result<()> {
    for part in SCHEMA_PARTS {
        conn.execute_batch(part.tables_sql)
            .map_err(|err| DbError::sqlite(format!("执行建表脚本 {}.sql", part.name), err))?;
    }
    Ok(())
}

/// Execute all index creation DDL scripts in canonical order.
pub fn apply_index_schemas(conn: &Connection) -> Result<()> {
    for part in SCHEMA_PARTS {
        conn.execute_batch(part.indexes_sql)
            .map_err(|err| DbError::sqlite(format!("执行建索引脚本 {}.idx.sql", part.name), err))?;
    }
    Ok(())
}
