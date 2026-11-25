//! Asset registry entries and package cross-references.
//!
//! References are available only for packages with exported JSON. Packages skipped as images,
//! meshes, audio, or other binary data therefore have incoming but no outgoing references.

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DataError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetTag {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetEntry {
    pub path: String,
    pub package_name: String,
    pub primary_class: String,
    pub asset_classes: Vec<String>,
    pub asset_name: String,
    pub size: i64,
    pub package_size: i64,
    pub bulk_size: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<AssetTag>,
    pub dependency_count: Option<i64>,
    pub matched_in_vfs: bool,
    pub decision: String,
    pub category: String,
    pub export_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRefKind {
    Hard,
    Soft,
    Script,
}

impl AssetRefKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hard => "hard",
            Self::Soft => "soft",
            Self::Script => "script",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "hard" => Some(Self::Hard),
            "soft" => Some(Self::Soft),
            "script" => Some(Self::Script),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetPackageRef {
    pub to_package: String,
    pub kind: AssetRefKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRef {
    pub from_asset_id: i64,
    pub from_path: String,
    pub from_package_name: String,
    pub to_package: String,
    pub to_asset_id: Option<i64>,
    pub to_path: Option<String>,
    pub kind: AssetRefKind,
}

#[derive(Deserialize)]
struct RawAssetEntry {
    path: String,
    #[serde(rename = "packageName")]
    package_name: String,
    #[serde(rename = "primaryClass")]
    primary_class: String,
    #[serde(rename = "assetClasses")]
    asset_classes: Vec<String>,
    #[serde(rename = "assetName")]
    asset_name: String,
    size: i64,
    #[serde(rename = "packageSize")]
    package_size: i64,
    #[serde(rename = "bulkSize")]
    bulk_size: i64,
    tags: BTreeMap<String, String>,
    #[serde(rename = "dependencyCount")]
    dependency_count: Option<i64>,
    #[serde(rename = "matchedInVfs")]
    matched_in_vfs: bool,
    decision: String,
    category: String,
}

pub fn parse_registry_line(line: &str) -> Result<AssetEntry> {
    let raw: RawAssetEntry = serde_json::from_str(line).map_err(|err| {
        DataError::invalid_schema("inventory/registry.jsonl", "json", err.to_string())
    })?;
    Ok(AssetEntry {
        path: raw.path,
        package_name: raw.package_name,
        primary_class: raw.primary_class,
        asset_classes: raw.asset_classes,
        asset_name: raw.asset_name,
        size: raw.size,
        package_size: raw.package_size,
        bulk_size: raw.bulk_size,
        tags: raw
            .tags
            .into_iter()
            .map(|(key, value)| AssetTag { key, value })
            .collect(),
        dependency_count: raw.dependency_count,
        matched_in_vfs: raw.matched_in_vfs,
        decision: raw.decision,
        category: raw.category,
        export_path: None,
    })
}

pub fn parse_registry_jsonl(reader: impl BufRead) -> Result<Vec<AssetEntry>> {
    let mut entries = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|source| DataError::Io {
            kind: "资源注册表",
            path: "inventory/registry.jsonl".into(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let entry = parse_registry_line(&line).map_err(|err| {
            DataError::invalid_schema(
                "inventory/registry.jsonl",
                format!("line {}", index + 1),
                err.to_string(),
            )
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

pub fn collect_package_refs(document: &Value, from_package: &str) -> BTreeSet<AssetPackageRef> {
    let mut refs = BTreeSet::new();
    collect_value_refs(document, from_package, &mut refs);
    refs
}

fn collect_value_refs(value: &Value, from_package: &str, refs: &mut BTreeSet<AssetPackageRef>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_value_refs(item, from_package, refs);
            }
        }
        Value::Object(object) => {
            if let Some(path) = object.get("ObjectPath").and_then(Value::as_str) {
                insert_package_ref(path, AssetRefKind::Hard, from_package, refs);
            }
            if let Some(path) = object.get("AssetPathName").and_then(Value::as_str) {
                insert_package_ref(path, AssetRefKind::Soft, from_package, refs);
            }
            for child in object.values() {
                collect_value_refs(child, from_package, refs);
            }
        }
        _ => {}
    }
}

fn insert_package_ref(
    raw_path: &str,
    kind: AssetRefKind,
    from_package: &str,
    refs: &mut BTreeSet<AssetPackageRef>,
) {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() || raw_path == "None" || !raw_path.starts_with('/') {
        return;
    }
    let is_script = raw_path.starts_with("/Script/");
    let to_package = if is_script {
        raw_path.to_owned()
    } else {
        package_path(raw_path)
    };
    if to_package == from_package {
        return;
    }
    refs.insert(AssetPackageRef {
        to_package,
        kind: if is_script {
            AssetRefKind::Script
        } else {
            kind
        },
    });
}

fn package_path(object_path: &str) -> String {
    let last_segment = object_path.rsplit('/').next().unwrap_or(object_path);
    match last_segment.find('.') {
        Some(dot) => {
            let package_len = object_path.len() - last_segment.len() + dot;
            object_path[..package_len].to_owned()
        }
        None => object_path.to_owned(),
    }
}
