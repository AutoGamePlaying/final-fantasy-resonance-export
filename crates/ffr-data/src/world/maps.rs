//! Typed loaders for maps, islands, regions, save points, world-map teleport landmarks, and
//! treasure boxes.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::datatable::{DataTable, load_data_table};
use crate::error::{DataError, Result};
use crate::ftext::FTextValue;
use crate::paths::datatable_dir;
use crate::world::parse::{
    WorldPoint3, expect_array, expect_bool, expect_i64, expect_object, expect_row_object,
    expect_row_struct, expect_string, non_negative_id, parse_point3,
};

/// Relative path of `CDT_MapData_Prod` under `FFRS/Content/Datatable/`.
pub const MAP_DATA_TABLE: &str = "Map/CDT_MapData_Prod";
/// Relative path of `DT_MapIslandData` under `FFRS/Content/Datatable/`.
pub const MAP_ISLAND_TABLE: &str = "Map/DT_MapIslandData";
/// Relative path of `DT_MapRegionData` under `FFRS/Content/Datatable/`.
pub const MAP_REGION_TABLE: &str = "Map/DT_MapRegionData";
/// Relative path of `DT_MapReleaseConditionData` under `FFRS/Content/Datatable/`.
pub const MAP_RELEASE_CONDITION_TABLE: &str = "Map/DT_MapReleaseConditionData";
/// Relative path of `DA_MapLandmark` (`CPP_DA_MapLandmark`) under `FFRS/Content/Datatable/`.
pub const MAP_LANDMARK_ASSET: &str = "Map/DA_MapLandmark";
/// Relative path of `CDT_MapGimmickTreasureBox` under `FFRS/Content/Datatable/`.
pub const TREASURE_BOX_TABLE: &str = "Map/Gimmick/TreasureBox/CDT_MapGimmickTreasureBox";
/// Relative path of `DT_SavePointData` under `FFRS/Content/Datatable/`.
pub const SAVE_POINT_DATA_TABLE: &str = "UI/SavePoint/DT_SavePointData";
/// Relative path of `DT_SavePointCommand` under `FFRS/Content/Datatable/`.
pub const SAVE_POINT_COMMAND_TABLE: &str = "UI/SavePoint/DT_SavePointCommand";

/// Facility marker declared on a map (`MAP_DATA.facilityDataList`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapFacility {
    pub condition: String,
    pub interact_type: String,
}

/// Conditional startup event bound to a map (`MAP_DATA.startupEventList`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapStartupEvent {
    pub condition: String,
    pub event_id: String,
}

/// One map record from `Map/CDT_MapData_Prod.json` (`RowStruct = MAP_DATA`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapInfo {
    pub id: i64,
    pub row_key: String,
    pub sort_id: i64,
    pub group_id: i64,
    pub region_id: Option<i64>,
    pub same_map_id: Option<i64>,
    pub island: String,
    pub name: FTextValue,
    /// Full UE `SoftObjectPath` (`Level.AssetPathName`), e.g.
    /// `"/Game/Map/Dng/01Gra/01Gra_43/Dng_01Gra_43_PL.Dng_01Gra_43_PL"`.
    pub level_asset_path: String,
    /// Extracted level stem (`*_PL`) used by runtime bridge reverse lookups, e.g. `"Dng_01Gra_43_PL"`.
    pub level_name: String,
    pub map_type: String,
    pub dungeon_type: String,
    pub danger_level: i64,
    pub battle_stage: Option<i64>,
    pub do_movement_encount: bool,
    pub do_encount_condition: String,
    pub encount_group_id: Option<i64>,
    pub facilities: Vec<MapFacility>,
    pub startup_events: Vec<MapStartupEvent>,
}

/// Island/continent metadata from `Map/DT_MapIslandData.json` (`MAP_ISLAND_DATA`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapIsland {
    pub row_key: String,
    pub island_type: String,
    pub name: FTextValue,
    pub area_ids: Vec<i64>,
    pub display_condition: String,
}

/// Region record from `Map/DT_MapRegionData.json` (`MAP_REGION_DATA`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRegion {
    pub id: i64,
    pub row_key: String,
    pub island_type: String,
}

/// Island unlock condition from `Map/DT_MapReleaseConditionData.json` (`MAP_RELEASE_CONDITION_DATA`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapReleaseCondition {
    pub row_key: String,
    pub condition: String,
    pub island_type: String,
}

/// Save-point capability profile from `UI/SavePoint/DT_SavePointData.json` (`SavePointData`).
///
/// Note: Individual purple magic circle actors are placed in `.umap` levels rather than static
/// DataTables. This table defines what actions a save point supports in each map type
/// (`eMapType::Town` vs `eMapType::Dungeon`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePoint {
    pub row_key: String,
    pub map_type: String,
    pub can_save: bool,
    pub can_use_tent: bool,
    pub can_teleport: bool,
    pub commands: Vec<(String, bool)>,
}

/// One save-point menu command definition from `UI/SavePoint/DT_SavePointCommand.json`
/// (`SavePoint_CommnadInfo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePointCommand {
    pub row_key: String,
    pub command_type: String,
    pub label: FTextValue,
    pub tooltip: FTextValue,
    pub item_id: Option<i64>,
    pub is_enable: bool,
}

/// Conditional map target on a world-map landmark (`DA_MapLandmark.landmarkDataList[].landmarkDataList`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeleportTarget {
    pub condition: String,
    pub map_id: Option<i64>,
}

/// One world-map fast-travel landmark and airship landing point from `Map/DA_MapLandmark.json`
/// (`CPP_DA_MapLandmark`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeleportPoint {
    pub index: usize,
    pub targets: Vec<TeleportTarget>,
    pub location: WorldPoint3,
    pub airship_landing_point_id: Option<i64>,
    pub airship_landing_location: WorldPoint3,
}

impl TeleportPoint {
    /// Returns all active (non-`-1`) map IDs reachable via this landmark.
    pub fn target_map_ids(&self) -> Vec<i64> {
        self.targets
            .iter()
            .filter_map(|target| target.map_id)
            .collect()
    }
}

/// One static treasure box entry from `Map/Gimmick/TreasureBox/CDT_MapGimmickTreasureBox.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreasureBox {
    pub row_key: String,
    pub map_id: i64,
    pub id: i64,
    pub item_id: Option<i64>,
    pub item_num: i64,
    pub money: Option<i64>,
    pub is_trap: bool,
    pub battle_group_id: Option<i64>,
    pub battle_finish_event_id: Option<String>,
}

/// Extract the level stem (`*_PL`) from a UE level path or runtime level name.
///
/// Handles full UE object paths (`"/Game/Map/.../Com_00Com_45_PL.Com_00Com_45_PL"`), package
/// paths (`"/Game/Map/.../Com_00Com_45_PL"`), sub-object paths (`"Com_00Com_45_PL:PersistentLevel"`),
/// and bare level names (`"Com_00Com_45_PL"`).
pub fn extract_level_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let last_segment = trimmed.rsplit('/').next()?;
    let before_colon = last_segment.split(':').next()?;
    let stem = before_colon.split('.').next()?;
    if stem.is_empty() {
        return None;
    }
    Some(stem.to_owned())
}

/// Parse `Map/CDT_MapData_Prod` into typed [`MapInfo`] entries sorted by `id` ascending.
pub fn parse_maps(table: &DataTable, source_label: &str) -> Result<Vec<MapInfo>> {
    expect_row_struct(table, "MAP_DATA", source_label)?;
    let mut maps = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;

        let level_prefix = format!("{row_prefix}.Level");
        let level_obj = expect_object(row, "Level", source_label, &row_prefix)?;
        let level_asset_path =
            expect_string(level_obj, "AssetPathName", source_label, &level_prefix)?;
        let level_name = extract_level_name(&level_asset_path).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{level_prefix}.AssetPathName"),
                "关卡路径为空或无法提取关卡名",
            )
        })?;

        let raw_facilities = expect_array(row, "facilityDataList", source_label, &row_prefix)?;
        let mut facilities = Vec::with_capacity(raw_facilities.len());
        for (idx, item) in raw_facilities.iter().enumerate() {
            let item_prefix = format!("{row_prefix}.facilityDataList[{idx}]");
            let obj = expect_row_object(item, source_label, &item_prefix)?;
            facilities.push(MapFacility {
                condition: expect_string(obj, "Condition", source_label, &item_prefix)?,
                interact_type: expect_string(obj, "Type", source_label, &item_prefix)?,
            });
        }

        let raw_events = expect_array(row, "startupEventList", source_label, &row_prefix)?;
        let mut startup_events = Vec::with_capacity(raw_events.len());
        for (idx, item) in raw_events.iter().enumerate() {
            let item_prefix = format!("{row_prefix}.startupEventList[{idx}]");
            let obj = expect_row_object(item, source_label, &item_prefix)?;
            startup_events.push(MapStartupEvent {
                condition: expect_string(obj, "Condition", source_label, &item_prefix)?,
                event_id: expect_string(obj, "EventId", source_label, &item_prefix)?,
            });
        }

        maps.push(MapInfo {
            id: expect_i64(row, "ID", source_label, &row_prefix)?,
            row_key: row_key.clone(),
            sort_id: expect_i64(row, "SortId", source_label, &row_prefix)?,
            group_id: expect_i64(row, "GroupId", source_label, &row_prefix)?,
            region_id: non_negative_id(expect_i64(row, "regionId", source_label, &row_prefix)?),
            same_map_id: non_negative_id(expect_i64(row, "sameMapId", source_label, &row_prefix)?),
            island: expect_string(row, "island", source_label, &row_prefix)?,
            name: crate::world::parse::expect_ftext(row, "Name", source_label, &row_prefix)?,
            level_asset_path,
            level_name,
            map_type: expect_string(row, "MapType", source_label, &row_prefix)?,
            dungeon_type: expect_string(row, "dungeonType", source_label, &row_prefix)?,
            danger_level: expect_i64(row, "DangerLevel", source_label, &row_prefix)?,
            battle_stage: non_negative_id(expect_i64(
                row,
                "battleStage",
                source_label,
                &row_prefix,
            )?),
            do_movement_encount: expect_bool(row, "doMovementEncount", source_label, &row_prefix)?,
            do_encount_condition: expect_string(
                row,
                "doEncountCondition",
                source_label,
                &row_prefix,
            )?,
            encount_group_id: non_negative_id(expect_i64(
                row,
                "encountGroupId",
                source_label,
                &row_prefix,
            )?),
            facilities,
            startup_events,
        });
    }

    maps.sort_by_key(|map| map.id);
    Ok(maps)
}

/// Load `Map/CDT_MapData_Prod.json` from `export_root`.
pub fn load_maps(export_root: impl AsRef<Path>) -> Result<Vec<MapInfo>> {
    let path = datatable_dir(export_root)
        .join("Map")
        .join("CDT_MapData_Prod.json");
    let table = load_data_table(&path)?;
    parse_maps(&table, &path.display().to_string())
}

/// Parse `Map/DT_MapIslandData` into typed [`MapIsland`] entries.
pub fn parse_map_islands(table: &DataTable, source_label: &str) -> Result<Vec<MapIsland>> {
    expect_row_struct(table, "MAP_ISLAND_DATA", source_label)?;
    let mut islands = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let raw_areas = expect_array(row, "areaIds", source_label, &row_prefix)?;
        let mut area_ids = Vec::with_capacity(raw_areas.len());
        for (idx, val) in raw_areas.iter().enumerate() {
            let area = val.as_i64().ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.areaIds[{idx}]"),
                    "不是整数",
                )
            })?;
            area_ids.push(area);
        }
        let cond_prefix = format!("{row_prefix}.DisplayCondition");
        let cond_obj = expect_object(row, "DisplayCondition", source_label, &row_prefix)?;
        islands.push(MapIsland {
            row_key: row_key.clone(),
            island_type: expect_string(row, "IslandType", source_label, &row_prefix)?,
            name: crate::world::parse::expect_ftext(row, "Name", source_label, &row_prefix)?,
            area_ids,
            display_condition: expect_string(cond_obj, "Expression", source_label, &cond_prefix)?,
        });
    }

    Ok(islands)
}

/// Load `Map/DT_MapIslandData.json` from `export_root`.
pub fn load_map_islands(export_root: impl AsRef<Path>) -> Result<Vec<MapIsland>> {
    let path = datatable_dir(export_root)
        .join("Map")
        .join("DT_MapIslandData.json");
    let table = load_data_table(&path)?;
    parse_map_islands(&table, &path.display().to_string())
}

/// Parse `Map/DT_MapRegionData` into typed [`MapRegion`] entries sorted by `id` ascending.
pub fn parse_map_regions(table: &DataTable, source_label: &str) -> Result<Vec<MapRegion>> {
    expect_row_struct(table, "MAP_REGION_DATA", source_label)?;
    let mut regions = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        regions.push(MapRegion {
            id: expect_i64(row, "ID", source_label, &row_prefix)?,
            row_key: row_key.clone(),
            island_type: expect_string(row, "IslandType", source_label, &row_prefix)?,
        });
    }

    regions.sort_by_key(|region| region.id);
    Ok(regions)
}

/// Load `Map/DT_MapRegionData.json` from `export_root`.
pub fn load_map_regions(export_root: impl AsRef<Path>) -> Result<Vec<MapRegion>> {
    let path = datatable_dir(export_root)
        .join("Map")
        .join("DT_MapRegionData.json");
    let table = load_data_table(&path)?;
    parse_map_regions(&table, &path.display().to_string())
}

/// Parse `Map/DT_MapReleaseConditionData` into typed [`MapReleaseCondition`] entries.
pub fn parse_map_release_conditions(
    table: &DataTable,
    source_label: &str,
) -> Result<Vec<MapReleaseCondition>> {
    expect_row_struct(table, "MAP_RELEASE_CONDITION_DATA", source_label)?;
    let mut conditions = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        conditions.push(MapReleaseCondition {
            row_key: row_key.clone(),
            condition: expect_string(row, "Condition", source_label, &row_prefix)?,
            island_type: expect_string(row, "IslandType", source_label, &row_prefix)?,
        });
    }

    Ok(conditions)
}

/// Load `Map/DT_MapReleaseConditionData.json` from `export_root`.
pub fn load_map_release_conditions(
    export_root: impl AsRef<Path>,
) -> Result<Vec<MapReleaseCondition>> {
    let path = datatable_dir(export_root)
        .join("Map")
        .join("DT_MapReleaseConditionData.json");
    let table = load_data_table(&path)?;
    parse_map_release_conditions(&table, &path.display().to_string())
}

/// Parse `UI/SavePoint/DT_SavePointData` into typed [`SavePoint`] capability profiles.
pub fn parse_save_points(table: &DataTable, source_label: &str) -> Result<Vec<SavePoint>> {
    expect_row_struct(table, "SavePointData", source_label)?;
    let mut save_points = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let map_type = expect_string(row, "SavePointType", source_label, &row_prefix)?;

        let cmd_data_prefix = format!("{row_prefix}.CommandData");
        let cmd_data = expect_object(row, "CommandData", source_label, &row_prefix)?;
        let display_map = expect_array(
            cmd_data,
            "CommandDisplayMap",
            source_label,
            &cmd_data_prefix,
        )?;

        let mut can_save = false;
        let mut can_use_tent = false;
        let mut can_teleport = false;
        let mut commands = Vec::with_capacity(display_map.len());

        for (idx, item) in display_map.iter().enumerate() {
            let item_prefix = format!("{cmd_data_prefix}.CommandDisplayMap[{idx}]");
            let obj = expect_row_object(item, source_label, &item_prefix)?;
            let key = expect_string(obj, "Key", source_label, &item_prefix)?;
            let value = expect_bool(obj, "Value", source_label, &item_prefix)?;
            match key.as_str() {
                "ESavePointCommandType::SavePoint" => can_save = value,
                "ESavePointCommandType::Tent" => can_use_tent = value,
                "ESavePointCommandType::Teleport" => can_teleport = value,
                _ => {}
            }
            commands.push((key, value));
        }

        save_points.push(SavePoint {
            row_key: row_key.clone(),
            map_type,
            can_save,
            can_use_tent,
            can_teleport,
            commands,
        });
    }

    Ok(save_points)
}

/// Load `UI/SavePoint/DT_SavePointData.json` from `export_root`.
pub fn load_save_points(export_root: impl AsRef<Path>) -> Result<Vec<SavePoint>> {
    let path = datatable_dir(export_root)
        .join("UI")
        .join("SavePoint")
        .join("DT_SavePointData.json");
    let table = load_data_table(&path)?;
    parse_save_points(&table, &path.display().to_string())
}

/// Parse `UI/SavePoint/DT_SavePointCommand` into typed [`SavePointCommand`] entries.
pub fn parse_save_point_commands(
    table: &DataTable,
    source_label: &str,
) -> Result<Vec<SavePointCommand>> {
    expect_row_struct(table, "SavePoint_CommnadInfo", source_label)?;
    let mut commands = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let info_prefix = format!("{row_prefix}.CommandInfo");
        let info = expect_object(row, "CommandInfo", source_label, &row_prefix)?;

        commands.push(SavePointCommand {
            row_key: row_key.clone(),
            command_type: expect_string(row, "CommandType", source_label, &row_prefix)?,
            label: crate::world::parse::expect_ftext(info, "Command", source_label, &info_prefix)?,
            tooltip: crate::world::parse::expect_ftext(
                info,
                "ToopTips",
                source_label,
                &info_prefix,
            )?,
            item_id: non_negative_id(expect_i64(info, "ItemId", source_label, &info_prefix)?),
            is_enable: expect_bool(info, "IsEnable", source_label, &info_prefix)?,
        });
    }

    Ok(commands)
}

/// Load `UI/SavePoint/DT_SavePointCommand.json` from `export_root`.
pub fn load_save_point_commands(export_root: impl AsRef<Path>) -> Result<Vec<SavePointCommand>> {
    let path = datatable_dir(export_root)
        .join("UI")
        .join("SavePoint")
        .join("DT_SavePointCommand.json");
    let table = load_data_table(&path)?;
    parse_save_point_commands(&table, &path.display().to_string())
}

/// Parse a raw CUE4Parse JSON document for `Map/DA_MapLandmark.json` (`CPP_DA_MapLandmark`)
/// into typed [`TeleportPoint`] entries.
pub fn parse_teleport_points_document(
    document: &Value,
    source_label: &str,
) -> Result<Vec<TeleportPoint>> {
    let first = document
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(Value::as_object)
        .ok_or_else(|| {
            DataError::invalid_schema(source_label, "<root>", "不是非空 JSON 数组或首项不是对象")
        })?;

    match first.get("Type").and_then(Value::as_str) {
        Some("CPP_DA_MapLandmark") => {}
        other => {
            let actual = other.unwrap_or("缺失");
            return Err(DataError::invalid_schema(
                source_label,
                "Type",
                format!("值为 {actual}，不是 CPP_DA_MapLandmark"),
            ));
        }
    }

    let props = expect_object(first, "Properties", source_label, "[0]")?;
    let list = expect_array(props, "landmarkDataList", source_label, "Properties")?;
    let mut points = Vec::with_capacity(list.len());

    for (idx, item) in list.iter().enumerate() {
        let entry_prefix = format!("Properties.landmarkDataList[{idx}]");
        let obj = expect_row_object(item, source_label, &entry_prefix)?;
        let raw_targets = expect_array(obj, "landmarkDataList", source_label, &entry_prefix)?;
        let mut targets = Vec::with_capacity(raw_targets.len());
        for (t_idx, t_val) in raw_targets.iter().enumerate() {
            let target_prefix = format!("{entry_prefix}.landmarkDataList[{t_idx}]");
            let target_obj = expect_row_object(t_val, source_label, &target_prefix)?;
            targets.push(TeleportTarget {
                condition: expect_string(target_obj, "Condition", source_label, &target_prefix)?,
                map_id: non_negative_id(expect_i64(
                    target_obj,
                    "mapId",
                    source_label,
                    &target_prefix,
                )?),
            });
        }

        points.push(TeleportPoint {
            index: idx,
            targets,
            location: parse_point3(obj, "Location", source_label, &entry_prefix)?,
            airship_landing_point_id: non_negative_id(expect_i64(
                obj,
                "airShipLandingPointId",
                source_label,
                &entry_prefix,
            )?),
            airship_landing_location: parse_point3(
                obj,
                "airShipLandingLocation",
                source_label,
                &entry_prefix,
            )?,
        });
    }

    Ok(points)
}

/// Load `Map/DA_MapLandmark.json` (`CPP_DA_MapLandmark`) from `export_root`.
pub fn load_teleport_points(export_root: impl AsRef<Path>) -> Result<Vec<TeleportPoint>> {
    let path = datatable_dir(export_root)
        .join("Map")
        .join("DA_MapLandmark.json");
    let raw_text = fs::read_to_string(&path).map_err(|source| DataError::Io {
        kind: "地标数据资产",
        path: path.clone(),
        source,
    })?;
    let document: Value = serde_json::from_str(&raw_text).map_err(|source| DataError::Json {
        kind: "地标数据资产",
        path: path.clone(),
        source,
    })?;
    parse_teleport_points_document(&document, &path.display().to_string())
}

/// Parse `Map/Gimmick/TreasureBox/CDT_MapGimmickTreasureBox` into typed [`TreasureBox`] records
/// sorted by `(map_id, id)` ascending.
pub fn parse_treasure_boxes(table: &DataTable, source_label: &str) -> Result<Vec<TreasureBox>> {
    expect_row_struct(table, "MAP_GIMMICK_TREASURE_BOX_DATA", source_label)?;
    let mut boxes = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let finish_event_raw =
            expect_string(row, "battleFinishEventId", source_label, &row_prefix)?;
        let battle_finish_event_id = (!finish_event_raw.is_empty() && finish_event_raw != "None")
            .then_some(finish_event_raw);

        boxes.push(TreasureBox {
            row_key: row_key.clone(),
            map_id: expect_i64(row, "mapId", source_label, &row_prefix)?,
            id: expect_i64(row, "ID", source_label, &row_prefix)?,
            item_id: non_negative_id(expect_i64(row, "ItemId", source_label, &row_prefix)?),
            item_num: expect_i64(row, "ItemNum", source_label, &row_prefix)?,
            money: non_negative_id(expect_i64(row, "Money", source_label, &row_prefix)?),
            is_trap: expect_bool(row, "IsTrap", source_label, &row_prefix)?,
            battle_group_id: non_negative_id(expect_i64(
                row,
                "battleGroupId",
                source_label,
                &row_prefix,
            )?),
            battle_finish_event_id,
        });
    }

    boxes.sort_by_key(|entry| (entry.map_id, entry.id));
    Ok(boxes)
}

/// Load `Map/Gimmick/TreasureBox/CDT_MapGimmickTreasureBox.json` from `export_root`.
pub fn load_treasure_boxes(export_root: impl AsRef<Path>) -> Result<Vec<TreasureBox>> {
    let path = datatable_dir(export_root)
        .join("Map")
        .join("Gimmick")
        .join("TreasureBox")
        .join("CDT_MapGimmickTreasureBox.json");
    let table = load_data_table(&path)?;
    parse_treasure_boxes(&table, &path.display().to_string())
}
