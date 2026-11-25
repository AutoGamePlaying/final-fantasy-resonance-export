//! Typed loader and character-mapping resolver for `Map/DT_MapUnitPlayableData.json`.
//!
//! Field actors (`BP_MapUnit_Player_C.m_MapUnitId`, exposed as `field.player.unit_id` in
//! FfrBridge v1) use map-unit IDs (e.g. `60` in the prologue, `10` from chapter 1 onward) rather
//! than `DT_UnitParameter.ID` (`1006`, `1001`). `DT_MapUnitPlayableData` maps those IDs via their
//! row keys (and, for non-protagonist party sprites, `Asset/Map/DT_MapUnitAsset` row keys) to
//! playable entries in `Unit/DT_UnitParameter`.

use std::collections::{BTreeMap, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::battle::{UNIT_PARAMETER_TABLE, Unit, UnitCategory, parse_units};
use crate::datatable::{DataTable, load_data_table};
use crate::error::{DataError, Result};
use crate::ftext::{FTextValue, ResolveOptions, ResolvedText, TextResolver, TextSource};
use crate::paths::{datatable_dir, datatable_relative_path_to_json};
use crate::store::DataStore;

/// Relative path of `DT_MapUnitPlayableData` under `FFRS/Content/Datatable/`.
pub const MAP_UNIT_PLAYABLE_TABLE: &str = "Map/DT_MapUnitPlayableData";
/// Relative path of `DT_MapUnitAsset` under `FFRS/Content/Datatable/` (companion sprite index).
pub const MAP_UNIT_ASSET_TABLE: &str = "Asset/Map/DT_MapUnitAsset";

/// One row from `Map/DT_MapUnitPlayableData.json` (`RowStruct = MAP_UNIT_PLAYABLE_DATA`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapUnitPlayable {
    pub row_key: String,
    pub map_unit_id: i64,
    pub enable_conditions: String,
    pub is_idle_by_four_direction: bool,
}

/// Resolved link between a field `m_MapUnitId` and its playable [`Unit`] in `DT_UnitParameter`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMapPlayableUnit {
    /// Field actor ID (`BP_MapUnit_Player_C.m_MapUnitId`, e.g. `60` or `10`).
    pub map_unit_id: i64,
    /// Battle/party character ID (`Unit/DT_UnitParameter.ID`, e.g. `1006` or `1001`).
    pub unit_id: i64,
    /// Shared row key matching `DT_UnitParameter` (e.g. `"レイン"`, `"魔人フィーナ"`).
    pub row_key: String,
    /// Character display name `FText` from `DT_UnitParameter.Name`.
    pub name: FTextValue,
    /// Enable condition expressions declared for `map_unit_id` in `DT_MapUnitPlayableData`.
    pub enable_conditions: Vec<String>,
    /// Four-direction idle setting from `DT_MapUnitPlayableData` (`None` if resolved only via
    /// `DT_MapUnitAsset` fallback).
    pub is_idle_by_four_direction: Option<bool>,
}

/// Indexed resolver over `Map/DT_MapUnitPlayableData`, `Unit/DT_UnitParameter`, and optional
/// `Asset/Map/DT_MapUnitAsset`.
#[derive(Debug, Clone)]
pub struct MapUnitPlayableData {
    entries: Vec<MapUnitPlayable>,
    resolved_by_map_unit_id: BTreeMap<i64, ResolvedMapPlayableUnit>,
    text_resolver: TextResolver,
}

impl MapUnitPlayableData {
    /// Load `Map/DT_MapUnitPlayableData` together with playable units from `export_root`.
    pub fn load(export_root: impl Into<PathBuf>) -> Result<Self> {
        let store = DataStore::new(export_root);
        Self::from_store(&store)
    }

    /// Build [`MapUnitPlayableData`] through an existing [`DataStore`], reusing its caches.
    pub fn from_store(store: &DataStore) -> Result<Self> {
        let playable_table = store.data_table(MAP_UNIT_PLAYABLE_TABLE)?;
        let playable_label = table_label(store, MAP_UNIT_PLAYABLE_TABLE);
        let entries = parse_map_unit_playable(&playable_table, &playable_label)?;

        let unit_table = store.data_table(UNIT_PARAMETER_TABLE)?;
        let unit_label = table_label(store, UNIT_PARAMETER_TABLE);
        let units = parse_units(&unit_table, &unit_label)?;

        let asset_rows = match store.data_table(MAP_UNIT_ASSET_TABLE) {
            Ok(table) => {
                let label = table_label(store, MAP_UNIT_ASSET_TABLE);
                parse_map_unit_asset_ids(&table, &label)?
            }
            Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };

        let resolved_by_map_unit_id = build_resolved_map(&entries, &units, &asset_rows);

        Ok(Self {
            entries,
            resolved_by_map_unit_id,
            text_resolver: store.text_resolver().clone(),
        })
    }

    /// All parsed rows from `Map/DT_MapUnitPlayableData`.
    pub fn entries(&self) -> &[MapUnitPlayable] {
        &self.entries
    }

    /// Return all `DT_MapUnitPlayableData` rows matching `map_unit_id`.
    pub fn entries_for_map_unit_id(&self, map_unit_id: i64) -> Vec<&MapUnitPlayable> {
        self.entries
            .iter()
            .filter(|entry| entry.map_unit_id == map_unit_id)
            .collect()
    }

    /// Resolve a field `m_MapUnitId` (e.g. `60` or `10`) to its playable character record.
    pub fn resolve_by_map_unit_id(&self, map_unit_id: i64) -> Option<&ResolvedMapPlayableUnit> {
        self.resolved_by_map_unit_id.get(&map_unit_id)
    }

    /// Return all resolved `MapUnitId -> Playable Unit` mappings sorted by `map_unit_id`.
    pub fn all_mappings(&self) -> Vec<&ResolvedMapPlayableUnit> {
        self.resolved_by_map_unit_id.values().collect()
    }

    /// Resolve the playable character's display name for `map_unit_id` in the default language
    /// (`"zh-Hans"`).
    pub fn character_name(&self, map_unit_id: i64) -> ResolvedText {
        self.character_name_in(map_unit_id, self.text_resolver.language())
    }

    /// Resolve the playable character's display name for `map_unit_id` in an explicit language.
    pub fn character_name_in(&self, map_unit_id: i64, lang: &str) -> ResolvedText {
        let Some(resolved) = self.resolve_by_map_unit_id(map_unit_id) else {
            return ResolvedText {
                text: None,
                lang: None,
                requested_language: lang.to_owned(),
                reason: Some(format!("找不到地图可操作角色编号 {map_unit_id} 对应的角色")),
                source: TextSource::Missing,
            };
        };
        self.text_resolver
            .resolve_with(&resolved.name, Some(lang), ResolveOptions::strict())
    }

    /// Shared cached [`TextResolver`].
    pub fn text_resolver(&self) -> &TextResolver {
        &self.text_resolver
    }
}

impl DataStore {
    /// Build a [`MapUnitPlayableData`] handle backed by this store's cached tables.
    pub fn map_unit_playable(&self) -> Result<MapUnitPlayableData> {
        MapUnitPlayableData::from_store(self)
    }
}

/// Parse `Map/DT_MapUnitPlayableData` into typed [`MapUnitPlayable`] records sorted by
/// `(map_unit_id, row_key)`.
pub fn parse_map_unit_playable(
    table: &DataTable,
    source_label: &str,
) -> Result<Vec<MapUnitPlayable>> {
    if table.row_struct != "MAP_UNIT_PLAYABLE_DATA" {
        return Err(DataError::UnexpectedRowStruct {
            path: source_label.to_owned(),
            expected: "MAP_UNIT_PLAYABLE_DATA".to_owned(),
            actual: table.row_struct.clone(),
        });
    }

    let mut entries = Vec::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = raw_row
            .as_object()
            .ok_or_else(|| DataError::invalid_schema(source_label, &row_prefix, "行值不是对象"))?;
        entries.push(MapUnitPlayable {
            row_key: row_key.clone(),
            map_unit_id: expect_i64(row, "MapUnitId", source_label, &row_prefix)?,
            enable_conditions: expect_string(row, "EnableConditions", source_label, &row_prefix)?,
            is_idle_by_four_direction: expect_bool(
                row,
                "isIdleByFourDirection",
                source_label,
                &row_prefix,
            )?,
        });
    }

    entries.sort_by(|a, b| {
        a.map_unit_id
            .cmp(&b.map_unit_id)
            .then_with(|| a.row_key.cmp(&b.row_key))
    });
    Ok(entries)
}

/// Load `Map/DT_MapUnitPlayableData.json` from `export_root`.
pub fn load_map_unit_playable(export_root: impl AsRef<Path>) -> Result<Vec<MapUnitPlayable>> {
    let path = datatable_dir(export_root)
        .join("Map")
        .join("DT_MapUnitPlayableData.json");
    let table = load_data_table(&path)?;
    parse_map_unit_playable(&table, &path.display().to_string())
}

fn parse_map_unit_asset_ids(table: &DataTable, source_label: &str) -> Result<Vec<(i64, String)>> {
    if table.row_struct != "UNIT_ASSET_DATA" {
        return Err(DataError::UnexpectedRowStruct {
            path: source_label.to_owned(),
            expected: "UNIT_ASSET_DATA".to_owned(),
            actual: table.row_struct.clone(),
        });
    }
    let mut pairs = Vec::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = raw_row
            .as_object()
            .ok_or_else(|| DataError::invalid_schema(source_label, &row_prefix, "行值不是对象"))?;
        let id = expect_i64(row, "ID", source_label, &row_prefix)?;
        pairs.push((id, row_key.clone()));
    }
    Ok(pairs)
}

fn build_resolved_map(
    entries: &[MapUnitPlayable],
    units: &[Unit],
    asset_rows: &[(i64, String)],
) -> BTreeMap<i64, ResolvedMapPlayableUnit> {
    let playable_by_row_key: HashMap<&str, &Unit> = units
        .iter()
        .filter(|unit| unit.category == UnitCategory::Playable)
        .map(|unit| (unit.row_key.as_str(), unit))
        .collect();

    let mut by_map_id: BTreeMap<i64, Vec<&MapUnitPlayable>> = BTreeMap::new();
    for entry in entries {
        by_map_id.entry(entry.map_unit_id).or_default().push(entry);
    }

    let mut resolved = BTreeMap::new();

    // 1. Resolve all `MapUnitId`s explicitly declared in `Map/DT_MapUnitPlayableData`.
    for (map_unit_id, group) in &by_map_id {
        let matched = group
            .iter()
            .find_map(|entry| {
                playable_by_row_key
                    .get(entry.row_key.as_str())
                    .map(|unit| (*entry, *unit))
            })
            .or_else(|| {
                // Fallback: match through `Asset/Map/DT_MapUnitAsset` row key if playable row key
                // differs (e.g. debug prefix).
                asset_rows.iter().find_map(|(asset_id, asset_row_key)| {
                    if *asset_id != *map_unit_id {
                        return None;
                    }
                    playable_by_row_key
                        .get(asset_row_key.as_str())
                        .map(|unit| (group[0], *unit))
                })
            });

        if let Some((primary_entry, unit)) = matched {
            let mut conditions = Vec::with_capacity(group.len());
            for entry in group {
                if !entry.enable_conditions.is_empty()
                    && !conditions.contains(&entry.enable_conditions)
                {
                    conditions.push(entry.enable_conditions.clone());
                }
            }
            resolved.insert(
                *map_unit_id,
                ResolvedMapPlayableUnit {
                    map_unit_id: *map_unit_id,
                    unit_id: unit.id,
                    row_key: unit.row_key.clone(),
                    name: unit.name.clone(),
                    enable_conditions: conditions,
                    is_idle_by_four_direction: Some(primary_entry.is_idle_by_four_direction),
                },
            );
        }
    }

    // 2. Also index remaining playable party sprites from `Asset/Map/DT_MapUnitAsset` (e.g. 20..80).
    for (asset_id, asset_row_key) in asset_rows {
        if resolved.contains_key(asset_id) {
            continue;
        }
        if let Some(&unit) = playable_by_row_key.get(asset_row_key.as_str()) {
            resolved.insert(
                *asset_id,
                ResolvedMapPlayableUnit {
                    map_unit_id: *asset_id,
                    unit_id: unit.id,
                    row_key: unit.row_key.clone(),
                    name: unit.name.clone(),
                    enable_conditions: Vec::new(),
                    is_idle_by_four_direction: None,
                },
            );
        }
    }

    resolved
}

fn table_label(store: &DataStore, relative_path: &str) -> String {
    datatable_relative_path_to_json(relative_path, store.export_root()).map_or_else(
        || relative_path.to_owned(),
        |path| path.display().to_string(),
    )
}

fn expect_i64(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<i64> {
    object.get(field).and_then(Value::as_i64).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是整数")
    })
}

fn expect_string(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是字符串")
        })
}

fn expect_bool(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<bool> {
    object.get(field).and_then(Value::as_bool).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是布尔值")
    })
}
