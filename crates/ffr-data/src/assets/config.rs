//! Unreal Engine configuration file (`Config/*.ini`) entries.

use serde::{Deserialize, Serialize};

use crate::error::{DataError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub file: String,
    pub section: String,
    pub key: String,
    pub value: String,
    pub op: String,
    pub ord: i64,
}

pub fn parse_ini(file: impl Into<String>, contents: &str) -> Result<Vec<ConfigEntry>> {
    let file = file.into();
    let mut entries = Vec::new();
    let mut section = String::new();

    for (line_index, raw_line) in contents.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let Some(section_name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
            else {
                return Err(DataError::invalid_schema(
                    &file,
                    format!("line {}", line_index + 1),
                    "配置节缺少右括号".to_owned(),
                ));
            };
            section = section_name.trim().to_owned();
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            return Err(DataError::invalid_schema(
                &file,
                format!("line {}", line_index + 1),
                "配置项缺少等号".to_owned(),
            ));
        };
        let raw_key = raw_key.trim();
        let (op, key) = match raw_key.as_bytes().first() {
            Some(b'+') => ("+", &raw_key[1..]),
            Some(b'-') => ("-", &raw_key[1..]),
            Some(b'.') => (".", &raw_key[1..]),
            Some(b'!') => ("!", &raw_key[1..]),
            _ => ("=", raw_key),
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(DataError::invalid_schema(
                &file,
                format!("line {}", line_index + 1),
                "配置项键名为空".to_owned(),
            ));
        }
        entries.push(ConfigEntry {
            file: file.clone(),
            section: section.clone(),
            key: key.to_owned(),
            value: raw_value.trim().to_owned(),
            op: op.to_owned(),
            ord: entries.len() as i64,
        });
    }
    Ok(entries)
}
