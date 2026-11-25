//! Generic readers for CUE4Parse `DataTable`, `CompositeDataTable`, `StringTable`, and
//! exported `Game.locres` JSON files.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{DataError, Result};

/// Concrete Unreal Engine table class of an exported data table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataTableType {
    DataTable,
    CompositeDataTable,
}

impl fmt::Display for DataTableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataTable => f.write_str("DataTable"),
            Self::CompositeDataTable => f.write_str("CompositeDataTable"),
        }
    }
}

/// Generic CUE4Parse `DataTable` or `CompositeDataTable`.
#[derive(Debug, Clone, PartialEq)]
pub struct DataTable {
    pub table_type: DataTableType,
    pub name: String,
    pub package_path: String,
    pub row_struct: String,
    pub rows: BTreeMap<String, Value>,
}

impl DataTable {
    /// Look up one row by row key.
    pub fn get_row(&self, row_key: &str) -> Option<&Value> {
        self.rows.get(row_key)
    }

    /// Number of rows in the table.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns `true` if the table has no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Exported Unreal Engine `StringTable` (source-language text table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTable {
    pub name: String,
    pub package_path: String,
    pub table_namespace: String,
    pub entries: BTreeMap<String, String>,
}

impl StringTable {
    /// Look up one source string entry by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Number of entries in the string table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the string table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Exported `Game.locres` catalog (`namespace -> key -> localized text`) for one language.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalizationCatalog {
    pub namespaces: BTreeMap<String, BTreeMap<String, String>>,
}

impl LocalizationCatalog {
    /// Look up a localized string by `(namespace, key)`.
    pub fn get(&self, namespace: &str, key: &str) -> Option<&str> {
        self.namespaces
            .get(namespace)
            .and_then(|bucket| bucket.get(key))
            .map(String::as_str)
    }

    /// Number of namespaces present in the catalog.
    pub fn namespace_count(&self) -> usize {
        self.namespaces.len()
    }

    /// Total number of localized entries across all namespaces.
    pub fn entry_count(&self) -> usize {
        self.namespaces.values().map(BTreeMap::len).sum()
    }

    /// Returns `true` if the catalog has no namespaces.
    pub fn is_empty(&self) -> bool {
        self.namespaces.is_empty()
    }
}

/// Extract `"ChapterData"` from CUE4Parse's `"Class'ChapterData'"` representation.
pub fn normalize_row_struct_name(object_name: &str) -> &str {
    if let Some((prefix, rest)) = object_name.split_once('\'')
        && !prefix.is_empty()
        && prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && let Some(inner) = rest.strip_suffix('\'')
        && !inner.is_empty()
        && !inner.contains('\'')
    {
        return inner;
    }
    object_name
}

/// Narrow a parsed CUE4Parse JSON document into a generic [`DataTable`].
pub fn parse_data_table_document(document: &Value, source_label: &str) -> Result<DataTable> {
    parse_data_table_value(document.clone(), source_label)
}

/// Load one CUE4Parse `DataTable` or `CompositeDataTable` JSON file from disk.
pub fn load_data_table(path: impl AsRef<Path>) -> Result<DataTable> {
    let path = path.as_ref();
    let parsed = read_json_file(path, "数据表")?;
    let label = path.display().to_string();
    parse_data_table_value(parsed, &label)
}

/// Narrow a parsed CUE4Parse JSON document into a [`StringTable`].
pub fn parse_string_table_document(document: &Value, source_label: &str) -> Result<StringTable> {
    parse_string_table_value(document.clone(), source_label)
}

/// Load one CUE4Parse `StringTable` JSON file from disk.
pub fn load_string_table(path: impl AsRef<Path>) -> Result<StringTable> {
    let path = path.as_ref();
    let parsed = read_json_file(path, "字符串表")?;
    let label = path.display().to_string();
    parse_string_table_value(parsed, &label)
}

/// Narrow a parsed `{ namespace: { key: text } }` locres JSON document into [`LocalizationCatalog`].
pub fn parse_localization_document(
    document: &Value,
    source_label: &str,
) -> Result<LocalizationCatalog> {
    parse_localization_value(document.clone(), source_label)
}

/// Load one exported locres JSON file (`<export_root>/Localization/<lang>.json`) from disk.
pub fn load_localization(path: impl AsRef<Path>) -> Result<LocalizationCatalog> {
    let path = path.as_ref();
    let parsed = read_json_file(path, "本地化文件")?;
    let label = path.display().to_string();
    parse_localization_value(parsed, &label)
}

fn read_json_file(path: &Path, kind: &'static str) -> Result<Value> {
    let raw_text = fs::read_to_string(path).map_err(|source| DataError::Io {
        kind,
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&raw_text).map_err(|source| DataError::Json {
        kind,
        path: path.to_path_buf(),
        source,
    })
}

fn first_export_object(document: Value, source_label: &str) -> Result<Map<String, Value>> {
    let Value::Array(items) = document else {
        return Err(DataError::invalid_schema(
            source_label,
            "<root>",
            "不是非空 JSON 数组",
        ));
    };
    let Some(first) = items.into_iter().next() else {
        return Err(DataError::invalid_schema(
            source_label,
            "<root>",
            "不是非空 JSON 数组",
        ));
    };
    match first {
        Value::Object(map) => Ok(map),
        _ => Err(DataError::invalid_schema(
            source_label,
            "[0]",
            "首项不是对象",
        )),
    }
}

fn take_required_string(
    object: &mut Map<String, Value>,
    field: &str,
    source_label: &str,
) -> Result<String> {
    match object.remove(field) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(DataError::invalid_schema(
            source_label,
            field,
            "缺少非空字符串",
        )),
    }
}

fn parse_data_table_value(document: Value, source_label: &str) -> Result<DataTable> {
    let mut entry = first_export_object(document, source_label)?;
    let table_type = match entry.get("Type").and_then(Value::as_str) {
        Some("DataTable") => DataTableType::DataTable,
        Some("CompositeDataTable") => DataTableType::CompositeDataTable,
        other => {
            let actual = match other {
                Some(text) => text.to_owned(),
                None => entry
                    .get("Type")
                    .map_or_else(|| "缺失".to_owned(), Value::to_string),
            };
            return Err(DataError::invalid_schema(
                source_label,
                "Type",
                format!("值为 {actual}，不是 DataTable 或 CompositeDataTable"),
            ));
        }
    };

    let name = take_required_string(&mut entry, "Name", source_label)?;
    let package_path = take_required_string(&mut entry, "Package", source_label)?;

    let properties = match entry.get("Properties") {
        Some(Value::Object(map)) => map,
        _ => {
            return Err(DataError::invalid_schema(
                source_label,
                "Properties",
                "缺少对象",
            ));
        }
    };
    let row_struct_name = properties
        .get("RowStruct")
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("ObjectName"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                "Properties.RowStruct.ObjectName",
                "缺少字符串",
            )
        })?;
    let row_struct = normalize_row_struct_name(row_struct_name).to_owned();

    let raw_rows = match entry.remove("Rows") {
        Some(Value::Object(map)) => map,
        _ => {
            return Err(DataError::invalid_schema(source_label, "Rows", "缺少对象"));
        }
    };

    let mut rows = BTreeMap::new();
    for (row_key, row_value) in raw_rows {
        if !row_value.is_object() {
            return Err(DataError::invalid_schema(
                source_label,
                format!("Rows.{row_key}"),
                "行值不是对象",
            ));
        }
        rows.insert(row_key, row_value);
    }

    Ok(DataTable {
        table_type,
        name,
        package_path,
        row_struct,
        rows,
    })
}

fn parse_string_table_value(document: Value, source_label: &str) -> Result<StringTable> {
    let mut entry = first_export_object(document, source_label)?;
    match entry.get("Type").and_then(Value::as_str) {
        Some("StringTable") => {}
        other => {
            let actual = match other {
                Some(text) => text.to_owned(),
                None => entry
                    .get("Type")
                    .map_or_else(|| "缺失".to_owned(), Value::to_string),
            };
            return Err(DataError::invalid_schema(
                source_label,
                "Type",
                format!("值为 {actual}，不是 StringTable"),
            ));
        }
    }

    let name = take_required_string(&mut entry, "Name", source_label)?;
    let package_path = take_required_string(&mut entry, "Package", source_label)?;

    let mut table_body = match entry.remove("StringTable") {
        Some(Value::Object(map)) => map,
        _ => {
            return Err(DataError::invalid_schema(
                source_label,
                "StringTable",
                "缺少对象",
            ));
        }
    };
    let table_namespace = match table_body.remove("TableNamespace") {
        Some(Value::String(value)) if !value.is_empty() => value,
        _ => {
            return Err(DataError::invalid_schema(
                source_label,
                "StringTable.TableNamespace",
                "缺少非空字符串",
            ));
        }
    };
    let raw_entries = match table_body.remove("KeysToEntries") {
        Some(Value::Object(map)) => map,
        _ => {
            return Err(DataError::invalid_schema(
                source_label,
                "StringTable.KeysToEntries",
                "缺少对象",
            ));
        }
    };

    let mut entries = BTreeMap::new();
    for (key, value) in raw_entries {
        let Value::String(text) = value else {
            return Err(DataError::invalid_schema(
                source_label,
                format!("StringTable.KeysToEntries.{key}"),
                "值不是字符串",
            ));
        };
        entries.insert(key, text);
    }

    Ok(StringTable {
        name,
        package_path,
        table_namespace,
        entries,
    })
}

fn parse_localization_value(document: Value, source_label: &str) -> Result<LocalizationCatalog> {
    let Value::Object(raw_namespaces) = document else {
        return Err(DataError::invalid_schema(
            source_label,
            "<root>",
            "不是 JSON 对象",
        ));
    };

    let mut namespaces = BTreeMap::new();
    for (namespace, raw_bucket) in raw_namespaces {
        let Value::Object(raw_entries) = raw_bucket else {
            return Err(DataError::invalid_schema(
                source_label,
                &namespace,
                "命名空间不是对象",
            ));
        };
        let mut bucket = BTreeMap::new();
        for (key, entry) in raw_entries {
            let Value::String(text) = entry else {
                return Err(DataError::invalid_schema(
                    source_label,
                    format!("{namespace}/{key}"),
                    "值不是字符串",
                ));
            };
            bucket.insert(key, text);
        }
        namespaces.insert(namespace, bucket);
    }

    Ok(LocalizationCatalog { namespaces })
}
