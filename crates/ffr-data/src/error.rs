//! Error definitions for `ffr-data`.
//!
//! All user-facing error messages are written in Chinese and identify the failing file and field.

use std::path::PathBuf;

use thiserror::Error;

/// Crate-wide result alias for game data operations.
pub type Result<T> = std::result::Result<T, DataError>;

/// Errors produced when locating, reading, or validating exported FFR game data.
#[derive(Debug, Error)]
pub enum DataError {
    /// Required environment variable (`FFR_DEPS`) is missing or empty.
    #[error("未设置环境变量 {var}，请显式传入导出目录或设置 {var}")]
    MissingEnvVar { var: &'static str },

    /// Caller supplied a UE path or relative table path that cannot be mapped safely.
    #[error("无法把{kind}路径映射到导出目录：{input}")]
    InvalidPath { kind: &'static str, input: String },

    /// Caller supplied an unsafe or malformed language tag.
    #[error("语言代码不合法：{language}")]
    InvalidLanguage { language: String },

    /// Disk read failure for an exported JSON file.
    #[error("读取{kind}失败（{}）：{source}", path.display())]
    Io {
        kind: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// JSON syntax failure for an exported JSON file.
    #[error("{kind} JSON 格式错误（{}）：{source}", path.display())]
    Json {
        kind: &'static str,
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// Exported JSON structure or typed row field did not match expectations.
    #[error("文件 {path} 的字段 {field} 不合法：{detail}")]
    InvalidSchema {
        path: String,
        field: String,
        detail: String,
    },

    /// DataTable had a different `RowStruct` than required by a typed loader.
    #[error("数据表 {path} 的 RowStruct 应为 {expected}，实际为 {actual}")]
    UnexpectedRowStruct {
        path: String,
        expected: String,
        actual: String,
    },
}

impl DataError {
    pub(crate) fn invalid_schema(
        path: impl Into<String>,
        field: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::InvalidSchema {
            path: path.into(),
            field: field.into(),
            detail: detail.into(),
        }
    }
}
