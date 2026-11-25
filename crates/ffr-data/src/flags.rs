//! Typed loader and queries for `Flag/CDT_FlagCondition.json` (`RowStruct = FlagCondition`).
//!
//! `CDT_FlagCondition` is a `CompositeDataTable` merging `DT_FlagCondition_System` and
//! `Flag/Scenario/DT_FlagCondition_Chapter01..08`. Each row maps an internal condition name
//! (row key, e.g. `PlayerRideChocobo` or `C01_001A`) to a conjunction of event flag predicates
//! (`<id>` or `!<id>` joined by `&`). There is no separate localized description column in the
//! exported table.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::datatable::{DataTable, load_data_table};
use crate::error::{DataError, Result};
use crate::paths::{datatable_dir, datatable_relative_path_to_json};
use crate::store::DataStore;

/// Relative path of `CDT_FlagCondition` under `FFRS/Content/Datatable/`.
pub const FLAG_CONDITION_TABLE: &str = "Flag/CDT_FlagCondition";

/// One atomic flag predicate inside `FlagCondition.Condition` (`<id>` or `!<id>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagTerm {
    pub flag_id: i64,
    /// `true` if `flag_id` must be set (`<id>`); `false` if it must be clear (`!<id>`).
    pub expected: bool,
}

/// One named flag condition entry from `Flag/CDT_FlagCondition.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagConditionEntry {
    /// Internal condition name (table row key); the table contains no separate description column.
    pub row_key: String,
    /// Raw boolean expression string (e.g. `"!100&!102"`).
    pub condition: String,
    /// Parsed conjunction terms in table order.
    pub terms: Vec<FlagTerm>,
}

impl FlagConditionEntry {
    /// Evaluate this condition against a predicate checking whether a flag ID is currently set.
    pub fn evaluate(&self, is_set: impl Fn(i64) -> bool) -> bool {
        self.terms
            .iter()
            .all(|term| is_set(term.flag_id) == term.expected)
    }

    /// Return all distinct event flag IDs referenced by this condition, sorted ascending.
    pub fn flag_ids(&self) -> Vec<i64> {
        let unique: BTreeSet<i64> = self.terms.iter().map(|term| term.flag_id).collect();
        unique.into_iter().collect()
    }
}

/// Indexed view over `Flag/CDT_FlagCondition.json` supporting lookup by name and by flag ID.
#[derive(Debug, Clone, Default)]
pub struct FlagConditionData {
    entries: Vec<FlagConditionEntry>,
    by_name: BTreeMap<String, usize>,
    by_flag_id: BTreeMap<i64, Vec<usize>>,
}

impl FlagConditionData {
    /// Load `Flag/CDT_FlagCondition.json` from `export_root`.
    pub fn load(export_root: impl Into<PathBuf>) -> Result<Self> {
        let store = DataStore::new(export_root);
        Self::from_store(&store)
    }

    /// Build [`FlagConditionData`] through an existing [`DataStore`], reusing its table cache.
    pub fn from_store(store: &DataStore) -> Result<Self> {
        let table = store.data_table(FLAG_CONDITION_TABLE)?;
        let label = datatable_relative_path_to_json(FLAG_CONDITION_TABLE, store.export_root())
            .map_or_else(
                || FLAG_CONDITION_TABLE.to_owned(),
                |path| path.display().to_string(),
            );
        let entries = parse_flag_conditions(&table, &label)?;
        Ok(Self::from_entries(entries))
    }

    /// Assemble an indexed [`FlagConditionData`] from already-parsed entries.
    pub fn from_entries(entries: Vec<FlagConditionEntry>) -> Self {
        let mut by_name = BTreeMap::new();
        let mut by_flag_id: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
        for (idx, entry) in entries.iter().enumerate() {
            by_name.insert(entry.row_key.clone(), idx);
            for flag_id in entry.flag_ids() {
                by_flag_id.entry(flag_id).or_default().push(idx);
            }
        }
        Self {
            entries,
            by_name,
            by_flag_id,
        }
    }

    /// All parsed named flag conditions, sorted by `row_key`.
    pub fn entries(&self) -> &[FlagConditionEntry] {
        &self.entries
    }

    /// Look up one named flag condition by row key (e.g. `"PlayerRideChocobo"`).
    pub fn by_name(&self, row_key: &str) -> Option<&FlagConditionEntry> {
        self.by_name.get(row_key).map(|&idx| &self.entries[idx])
    }

    /// Look up all named flag conditions that reference event flag `flag_id`.
    pub fn conditions_for_flag(&self, flag_id: i64) -> Vec<&FlagConditionEntry> {
        self.by_flag_id
            .get(&flag_id)
            .map(|indices| indices.iter().map(|&idx| &self.entries[idx]).collect())
            .unwrap_or_default()
    }

    /// Return all distinct event flag IDs referenced across the entire table, sorted ascending.
    pub fn referenced_flag_ids(&self) -> Vec<i64> {
        self.by_flag_id.keys().copied().collect()
    }

    /// Evaluate named condition `row_key` against `is_set`, or `None` if `row_key` is unknown.
    pub fn evaluate(&self, row_key: &str, is_set: impl Fn(i64) -> bool) -> Option<bool> {
        self.by_name(row_key).map(|entry| entry.evaluate(is_set))
    }
}

impl DataStore {
    /// Build a [`FlagConditionData`] handle backed by this store's cached `Flag/CDT_FlagCondition`.
    pub fn flag_conditions(&self) -> Result<FlagConditionData> {
        FlagConditionData::from_store(self)
    }
}

/// Parse a `FlagCondition.Condition` expression (e.g. `"9"`, `"!100&!102"`) into [`FlagTerm`]s.
///
/// Returns `None` if the expression is empty or contains malformed tokens.
pub fn parse_flag_condition_expr(expr: &str) -> Option<Vec<FlagTerm>> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut terms = Vec::new();
    for raw_part in trimmed.split('&') {
        let part = raw_part.trim();
        let (expected, digits) = match part.strip_prefix('!') {
            Some(rest) => (false, rest.trim()),
            None => (true, part),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let flag_id = digits.parse::<i64>().ok()?;
        terms.push(FlagTerm { flag_id, expected });
    }
    Some(terms)
}

/// Parse `Flag/CDT_FlagCondition` into typed [`FlagConditionEntry`] records sorted by `row_key`.
pub fn parse_flag_conditions(
    table: &DataTable,
    source_label: &str,
) -> Result<Vec<FlagConditionEntry>> {
    if table.row_struct != "FlagCondition" {
        return Err(DataError::UnexpectedRowStruct {
            path: source_label.to_owned(),
            expected: "FlagCondition".to_owned(),
            actual: table.row_struct.clone(),
        });
    }

    let mut entries = Vec::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = raw_row
            .as_object()
            .ok_or_else(|| DataError::invalid_schema(source_label, &row_prefix, "行值不是对象"))?;
        let condition = row
            .get("Condition")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.Condition"),
                    "不是字符串",
                )
            })?;
        let terms = parse_flag_condition_expr(&condition).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.Condition"),
                format!("无法解析旗标条件表达式：{condition}"),
            )
        })?;
        entries.push(FlagConditionEntry {
            row_key: row_key.clone(),
            condition,
            terms,
        });
    }

    entries.sort_by(|a, b| a.row_key.cmp(&b.row_key));
    Ok(entries)
}

/// Load `Flag/CDT_FlagCondition.json` from `export_root`.
pub fn load_flag_conditions(export_root: impl AsRef<Path>) -> Result<Vec<FlagConditionEntry>> {
    let path = datatable_dir(export_root)
        .join("Flag")
        .join("CDT_FlagCondition.json");
    let table = load_data_table(&path)?;
    parse_flag_conditions(&table, &path.display().to_string())
}
