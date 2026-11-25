//! Error definitions for `ffr-db`.
//!
//! All user-facing error messages are written in Simplified Chinese and identify the failing path,
//! table, key, or stage.

use std::path::PathBuf;

use thiserror::Error;

/// Crate-wide result alias for `ffr-db`.
pub type Result<T> = std::result::Result<T, DbError>;

/// Errors produced when building, opening, or querying an `ffr-db` SQLite database.
#[derive(Debug, Error)]
pub enum DbError {
    /// Required `FFR_DEPS` environment variable is missing when no explicit path was supplied.
    #[error("未设置环境变量 {var}，请通过命令行参数显式指定路径或设置 {var}")]
    MissingEnvVar { var: &'static str },

    /// Unknown stage name supplied via `--stages`.
    #[error("未知的构建阶段 \"{name}\"，可选阶段：{valid}")]
    UnknownStage { name: String, valid: &'static str },

    /// Database file does not exist or cannot be opened.
    #[error("打开数据库文件失败（{}）：{detail}", path.display())]
    OpenFailed { path: PathBuf, detail: String },

    /// Database file is missing or has an unparseable `meta.schema_version` entry.
    #[error("数据库缺少合法的表结构版本记录（{}）：{detail}；请运行 ffr-db build 重新构建", path.display())]
    MissingSchemaVersion { path: PathBuf, detail: String },

    /// Database `schema_version` does not match the compiled [`crate::SCHEMA_VERSION`].
    #[error(
        "数据库表结构版本不兼容（文件为 v{actual}，当前程序要求 v{expected}）：{}；请运行 ffr-db build 重新构建",
        path.display()
    )]
    SchemaVersionMismatch {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },

    /// Read-only query endpoint rejected a write, DDL, multi-statement, or non-query SQL string.
    #[error("只读查询拒绝执行该 SQL：{detail}")]
    WriteQueryRejected { detail: String },

    /// Stage execution failed during database construction.
    #[error("构建阶段 {stage} 失败：{detail}")]
    StageFailed { stage: &'static str, detail: String },

    /// Required `DataStore` (export directory) was requested by a stage without an available export root.
    #[error("构建阶段 {stage} 需要导出目录（--export），但当前未配置或目录不存在")]
    MissingDataStore { stage: &'static str },

    /// Failed to serialize a Rust object into a `doc` JSON column.
    #[error("序列化表 {table} 主键 {key} 的对象文档失败：{source}")]
    DocSerialize {
        table: String,
        key: String,
        #[source]
        source: serde_json::Error,
    },

    /// Failed to deserialize a `doc` JSON column back into a Rust object.
    #[error("解析表 {table} 主键 {key} 的对象文档失败：{source}")]
    DocDeserialize {
        table: String,
        key: String,
        #[source]
        source: serde_json::Error,
    },

    /// Query plan check detected a disallowed full table scan (used by `testing::check_uses_index`).
    #[error("SQL 在大表 {table} 上触发了全表扫描（计划：{plan}），SQL：{sql}")]
    FullTableScan {
        table: String,
        plan: String,
        sql: String,
    },

    /// Underlying `ffr-data` parsing error.
    #[error("读取游戏导出数据失败：{0}")]
    Data(#[from] ffr_data::DataError),

    /// Underlying SQLite error with operation context.
    #[error("SQLite 操作失败（{action}）：{source}")]
    Sqlite {
        action: String,
        #[source]
        source: rusqlite::Error,
    },

    /// Filesystem I/O error with path and operation context.
    #[error("{action}失败（{}）：{source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl DbError {
    pub(crate) fn sqlite(action: impl Into<String>, source: rusqlite::Error) -> Self {
        Self::Sqlite {
            action: action.into(),
            source,
        }
    }

    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}
