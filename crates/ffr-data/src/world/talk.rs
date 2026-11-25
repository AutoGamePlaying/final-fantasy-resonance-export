//! Talk events (`Event/TalkEvent/CDT_TalkEventData_Prod`) and dialogue line tables
//! (`DT_*_TalkEvent` / `RowStruct = TalkEventRowData`).
//!
//! Provides typed Rust objects ([`TalkEvent`], [`TalkLine`], continuous/branch transition structs),
//! UE asset path normalization, and order-preserving JSON parsers so dialogue tables retain their
//! original author row order (critical for associating `TalkId` section headers with subsequent
//! lines).

use std::fmt;
use std::fs;
use std::path::Path;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::datatable::{DataTable, load_data_table, normalize_row_struct_name};
use crate::error::{DataError, Result};
use crate::ftext::{FTextValue, parse_ftext};
use crate::paths::datatable_dir;
use crate::world::parse::{expect_row_object, expect_row_struct};

/// Relative path of the composite talk-event master table under `FFRS/Content/Datatable/`.
pub const TALK_EVENT_DATA_TABLE: &str = "Event/TalkEvent/CDT_TalkEventData_Prod";

/// Expected UE `RowStruct` of `CDT_TalkEventData_Prod`.
pub const TALK_EVENT_ROW_STRUCT: &str = "TalkEventData";

/// Expected UE `RowStruct` of dialogue line DataTables (`DT_*_TalkEvent`, etc.).
pub const TALK_LINE_ROW_STRUCT: &str = "TalkEventRowData";

const GAME_DATATABLE_PREFIX: &str = "/Game/Datatable/";

/// Key-value parameter attached to a [`TalkEvent`] (`ArgList[]`, e.g. `{ Key: "id", Value: "1" }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkEventArg {
    pub key: String,
    pub value: String,
}

/// Conditional next-event transition entry (`ContinuousEventList[]` or
/// `ChoicesBranchEventList[].EventList[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkEventTransition {
    #[serde(default)]
    pub condition: String,
    pub event_id: String,
}

/// One player-choice branch target list (`ChoicesBranchEventList[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkEventChoiceBranch {
    pub index: i64,
    #[serde(default)]
    pub events: Vec<TalkEventTransition>,
}

/// Condition entry for one option in a custom choice dialog (`ItemConditionList[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkCustomChoiceItemCondition {
    pub index: i64,
    #[serde(default)]
    pub condition: String,
}

/// Custom choice dialog specification (`CustomChoiceDialogDataList[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkCustomChoiceDialog {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_dialog_tag: Option<String>,
    #[serde(default)]
    pub item_conditions: Vec<TalkCustomChoiceItemCondition>,
}

/// One talk/cutscene event definition from `Event/TalkEvent/CDT_TalkEventData_Prod`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkEvent {
    /// Row key in `CDT_TalkEventData_Prod` (e.g. `"C01_001_01"`, `"DT_Dng_01Gra_43_01_1060_2"`).
    pub id: String,
    /// Gameplay tag category (`Category.TagName`), if non-empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub description: String,
    /// UE enum value (e.g. `"ETalkEventType::Sequence"`, `"ETalkEventType::Blueprint"`).
    pub event_type: String,
    /// Canonical UE package path to the dialogue table (`TalkEventDataTable.AssetPathName`),
    /// stripped of its `.AssetName` suffix (e.g. `"/Game/Datatable/Event/Text/Chapter01/DT_C01_001_01_TalkEvent"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talk_table_path: Option<String>,
    /// Canonical UE package path to the cutscene `LevelSequence` (`EventSequence.AssetPathName`),
    /// stripped of its `.AssetName` suffix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_sequence_path: Option<String>,
    /// Canonical UE package path to the event Blueprint (`EventBlueprint.AssetPathName`), if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_blueprint_path: Option<String>,
    pub is_skip_confirm: bool,
    /// Triggered battle group ID (`DT_BattleGroup.ID`), or `None` when `-1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encount_group_id: Option<i64>,
    #[serde(default)]
    pub args: Vec<TalkEventArg>,
    #[serde(default)]
    pub continuous_events: Vec<TalkEventTransition>,
    #[serde(default)]
    pub choice_branches: Vec<TalkEventChoiceBranch>,
    #[serde(default)]
    pub custom_choice_dialogs: Vec<TalkCustomChoiceDialog>,
}

impl TalkEvent {
    /// Return the numeric dialogue section ID requested by `ArgList` (`id`, `ID`, or `talkId`),
    /// used by shared free-talk tables to select one NPC's dialogue block (`TalkId`).
    pub fn talk_id_arg(&self) -> Option<i64> {
        for arg in &self.args {
            if matches!(arg.key.as_str(), "id" | "ID" | "talkId") {
                let normalized: String = arg
                    .value
                    .trim()
                    .chars()
                    .map(|ch| match ch {
                        '０'..='９' => char::from(b'0' + (ch as u32 - '０' as u32) as u8),
                        other => other,
                    })
                    .collect();
                if let Ok(id) = normalized.parse::<i64>() {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Return `true` if `event_sequence_path` points to a developer test asset under
    /// `/Game/Developers/`.
    pub fn has_developer_sequence(&self) -> bool {
        self.event_sequence_path
            .as_deref()
            .is_some_and(is_developer_asset_path)
    }

    /// Return `true` if `talk_table_path` points to a developer test asset under
    /// `/Game/Developers/`.
    pub fn has_developer_talk_table(&self) -> bool {
        self.talk_table_path
            .as_deref()
            .is_some_and(is_developer_asset_path)
    }
}

/// One dialogue line row from a `TalkEventRowData` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkLine {
    /// Canonical UE package path of the owning dialogue table.
    pub table_path: String,
    /// Row key in the dialogue DataTable.
    pub row_key: String,
    /// 0-based order of this dialogue line within its table (preserving original JSON order).
    pub ord: usize,
    /// Enclosing `TalkId` section index from the preceding header row (`TalkId >= 0`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talk_id: Option<i64>,
    /// Speaker label (usually Japanese character/role name or `"None"`).
    pub speaker: String,
    /// UE speaker emotion enum (`ETalkSpeakerEmotion::*`).
    pub emotion: String,
    /// UE dialogue window type enum (`ETalkEventWindowType::*`).
    pub window_type: String,
    /// UE dialogue window position enum (`ETalkEventWindowPosition::*`).
    pub window_position: String,
    /// Raw parsed `ContentText` `FText`.
    pub content: FTextValue,
    /// Resolved localization namespace (from `StringTable` or inline `FText`), if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_ns: Option<String>,
    /// Localization key (from `StringTable` or inline `FText`), if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_key: Option<String>,
    /// Voice cue name, if any.
    #[serde(default)]
    pub voice: String,
    pub is_voice_valid: bool,
}

/// Returns `true` if a UE asset path belongs to `/Game/Developers/`.
pub fn is_developer_asset_path(path: &str) -> bool {
    path.contains("/Developers/")
}

/// Normalize a UE soft/object `AssetPathName` (e.g.
/// `"/Game/Datatable/Event/Text/Chapter01/DT_C01_001_01_TalkEvent.DT_C01_001_01_TalkEvent"`)
/// into its canonical package path without the `.AssetName` suffix.
///
/// Returns `None` for empty strings and `"None"`.
pub fn normalize_ue_asset_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("None") {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let stripped = match normalized.rfind('/') {
        Some(slash) => match normalized[slash + 1..].find('.') {
            Some(dot) => &normalized[..slash + 1 + dot],
            None => normalized.as_str(),
        },
        None => normalized.split('.').next().unwrap_or(normalized.as_str()),
    };
    let stripped = stripped.trim_end_matches('/');
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_owned())
    }
}

/// Convert a canonical UE DataTable path under `/Game/Datatable/` (with or without `.AssetName`)
/// into the relative path used by `dt_table.path` and `DataStore::data_table` (e.g.
/// `"Event/Text/Chapter01/DT_C01_001_01_TalkEvent"`).
///
/// Returns `None` for paths outside `/Game/Datatable/` (such as `/Game/Developers/...`).
pub fn talk_table_relative_path(ue_path: &str) -> Option<String> {
    let normalized = normalize_ue_asset_path(ue_path)?;
    let rel = normalized.strip_prefix(GAME_DATATABLE_PREFIX)?;
    if rel.is_empty() {
        None
    } else {
        Some(rel.to_owned())
    }
}

/// Parse one row of `Event/TalkEvent/CDT_TalkEventData_Prod` into a [`TalkEvent`].
pub fn parse_talk_event_row(
    row_key: &str,
    raw_row: &Value,
    source_label: &str,
) -> Result<TalkEvent> {
    let row_prefix = format!("Rows.{row_key}");
    let row = expect_row_object(raw_row, source_label, &row_prefix)?;

    let category = row
        .get("Category")
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("TagName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("None"))
        .map(str::to_owned);

    let description = optional_str(row, "Description").unwrap_or_default();
    let event_type = optional_str(row, "EventType").unwrap_or_default();
    let talk_table_path = extract_soft_asset_path(row, "TalkEventDataTable");
    let event_sequence_path = extract_soft_asset_path(row, "EventSequence");
    let event_blueprint_path = extract_soft_asset_path(row, "EventBlueprint");
    let is_skip_confirm = row
        .get("IsSkipConfirm")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let encount_group_id = row
        .get("encountGroupId")
        .and_then(Value::as_i64)
        .filter(|&id| id >= 0);

    let mut args = Vec::new();
    if let Some(raw_args) = row.get("ArgList").and_then(Value::as_array) {
        for item in raw_args {
            if let Some(obj) = item.as_object() {
                let key = optional_str(obj, "Key").unwrap_or_default();
                let value = optional_str(obj, "Value").unwrap_or_default();
                if !key.is_empty() {
                    args.push(TalkEventArg { key, value });
                }
            }
        }
    }

    let continuous_events = parse_transition_list(row.get("ContinuousEventList"));

    let mut choice_branches = Vec::new();
    if let Some(raw_choices) = row.get("ChoicesBranchEventList").and_then(Value::as_array) {
        for (fallback_idx, item) in raw_choices.iter().enumerate() {
            if let Some(obj) = item.as_object() {
                let index = obj
                    .get("Index")
                    .and_then(Value::as_i64)
                    .unwrap_or(fallback_idx as i64);
                let events = parse_transition_list(obj.get("EventList"));
                choice_branches.push(TalkEventChoiceBranch { index, events });
            }
        }
    }

    let mut custom_choice_dialogs = Vec::new();
    if let Some(raw_dialogs) = row
        .get("CustomChoiceDialogDataList")
        .and_then(Value::as_array)
    {
        for item in raw_dialogs {
            if let Some(obj) = item.as_object() {
                let choice_dialog_tag = obj
                    .get("ChoiceDialogTag")
                    .and_then(Value::as_object)
                    .and_then(|tag| tag.get("TagName"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("None"))
                    .map(str::to_owned);
                let mut item_conditions = Vec::new();
                if let Some(conds) = obj.get("ItemConditionList").and_then(Value::as_array) {
                    for (fallback_idx, cond_item) in conds.iter().enumerate() {
                        if let Some(cond_obj) = cond_item.as_object() {
                            let index = cond_obj
                                .get("Index")
                                .and_then(Value::as_i64)
                                .unwrap_or(fallback_idx as i64);
                            let condition = optional_str(cond_obj, "Condition").unwrap_or_default();
                            item_conditions
                                .push(TalkCustomChoiceItemCondition { index, condition });
                        }
                    }
                }
                custom_choice_dialogs.push(TalkCustomChoiceDialog {
                    choice_dialog_tag,
                    item_conditions,
                });
            }
        }
    }

    Ok(TalkEvent {
        id: row_key.to_owned(),
        category,
        description,
        event_type,
        talk_table_path,
        event_sequence_path,
        event_blueprint_path,
        is_skip_confirm,
        encount_group_id,
        args,
        continuous_events,
        choice_branches,
        custom_choice_dialogs,
    })
}

/// Parse all rows of `Event/TalkEvent/CDT_TalkEventData_Prod` into [`TalkEvent`] records sorted
/// by `id`.
pub fn parse_talk_events(table: &DataTable, source_label: &str) -> Result<Vec<TalkEvent>> {
    expect_row_struct(table, TALK_EVENT_ROW_STRUCT, source_label)?;
    let mut events = Vec::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        events.push(parse_talk_event_row(row_key, raw_row, source_label)?);
    }
    Ok(events)
}

/// Load `Event/TalkEvent/CDT_TalkEventData_Prod.json` from `export_root`.
pub fn load_talk_events(export_root: impl AsRef<Path>) -> Result<Vec<TalkEvent>> {
    let path = datatable_dir(export_root)
        .join("Event")
        .join("TalkEvent")
        .join("CDT_TalkEventData_Prod.json");
    let table = load_data_table(&path)?;
    parse_talk_events(&table, &path.display().to_string())
}

/// Parse ordered rows of a `TalkEventRowData` table into [`TalkLine`] records.
///
/// `ordered_rows` must preserve the table's original row order so `TalkId >= 0` section headers
/// associate accurately with subsequent `TalkId == -1` dialogue lines. Empty section-header rows
/// (`TalkId >= 0`, `Speaker == "None"` or empty, and empty `CultureInvariantString: null`) update
/// the active `talk_id` without producing blank dialogue lines.
pub fn parse_talk_lines_ordered<F>(
    table_path: &str,
    ordered_rows: &[(String, Value)],
    source_label: &str,
    mut resolve_string_table_ns: F,
) -> Result<Vec<TalkLine>>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut lines = Vec::new();
    let mut current_talk_id: Option<i64> = None;

    for (row_key, raw_row) in ordered_rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;

        let raw_talk_id = row.get("TalkId").and_then(Value::as_i64).unwrap_or(-1);
        let speaker = optional_str(row, "Speaker").unwrap_or_else(|| "None".to_owned());
        let emotion = optional_str(row, "Emotion").unwrap_or_default();
        let window_type = optional_str(row, "WindowType").unwrap_or_default();
        let window_position = optional_str(row, "WindowPosition").unwrap_or_default();
        let voice = optional_str(row, "Voice").unwrap_or_default();
        let is_voice_valid = row
            .get("IsVoiceValid")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let content = row
            .get("ContentText")
            .and_then(parse_ftext)
            .unwrap_or(FTextValue::CultureInvariant { value: None });

        let is_empty_content = matches!(&content, FTextValue::CultureInvariant { value: None });
        if raw_talk_id >= 0 {
            current_talk_id = Some(raw_talk_id);
            if is_empty_content && (speaker.is_empty() || speaker.eq_ignore_ascii_case("None")) {
                continue;
            }
        }

        let (text_ns, text_key) = match &content {
            FTextValue::StringTable { table_id, key, .. } => {
                let ns = resolve_string_table_ns(table_id)
                    .or_else(|| fallback_namespace_from_table_id(table_id));
                (ns, Some(key.clone()))
            }
            FTextValue::Inline { namespace, key, .. } => {
                (Some(namespace.clone()), Some(key.clone()))
            }
            FTextValue::CultureInvariant { .. } => (None, None),
        };

        let ord = lines.len();
        lines.push(TalkLine {
            table_path: table_path.to_owned(),
            row_key: row_key.clone(),
            ord,
            talk_id: current_talk_id,
            speaker,
            emotion,
            window_type,
            window_position,
            content,
            text_ns,
            text_key,
            voice,
            is_voice_valid,
        });
    }

    Ok(lines)
}

/// Ordered rows and normalized `RowStruct` loaded from a CUE4Parse DataTable JSON file while
/// preserving original JSON object key order.
#[derive(Debug, Clone)]
pub struct OrderedDataTableRows {
    pub row_struct: String,
    pub rows: Vec<(String, Value)>,
}

/// Load a CUE4Parse `DataTable` / `CompositeDataTable` JSON file from disk while preserving the
/// exact key order of `Rows` in the JSON file.
pub fn load_ordered_datatable_rows(path: impl AsRef<Path>) -> Result<OrderedDataTableRows> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|source| DataError::Io {
        kind: "对话数据表",
        path: path.to_path_buf(),
        source,
    })?;
    let label = path.display().to_string();
    parse_ordered_datatable_json(&raw, &label, path)
}

/// Parse a CUE4Parse `DataTable` JSON string while preserving `Rows` key order.
pub fn parse_ordered_datatable_json(
    raw_json: &str,
    source_label: &str,
    path_for_err: &Path,
) -> Result<OrderedDataTableRows> {
    let parsed: OrderedExportArray =
        serde_json::from_str(raw_json).map_err(|source| DataError::Json {
            kind: "对话数据表",
            path: path_for_err.to_path_buf(),
            source,
        })?;
    let Some(first) = parsed.first else {
        return Err(DataError::invalid_schema(
            source_label,
            "<root>",
            "不是非空 JSON 数组",
        ));
    };
    let row_struct = first
        .properties
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|p| p.get("RowStruct"))
        .and_then(Value::as_object)
        .and_then(|rs| rs.get("ObjectName"))
        .and_then(Value::as_str)
        .map(|s| normalize_row_struct_name(s).to_owned())
        .unwrap_or_default();

    let rows = first
        .rows
        .ok_or_else(|| DataError::invalid_schema(source_label, "Rows", "缺少对象"))?
        .0;

    Ok(OrderedDataTableRows { row_struct, rows })
}

struct OrderedExportArray {
    first: Option<OrderedExportObject>,
}

impl<'de> Deserialize<'de> for OrderedExportArray {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ArrayVisitor;
        impl<'de> Visitor<'de> for ArrayVisitor {
            type Value = OrderedExportArray;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a CUE4Parse export JSON array")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let first = seq.next_element::<OrderedExportObject>()?;
                while seq.next_element::<de::IgnoredAny>()?.is_some() {}
                Ok(OrderedExportArray { first })
            }
        }
        deserializer.deserialize_seq(ArrayVisitor)
    }
}

struct OrderedExportObject {
    properties: Option<Value>,
    rows: Option<OrderedRowList>,
}

impl<'de> Deserialize<'de> for OrderedExportObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ObjVisitor;
        impl<'de> Visitor<'de> for ObjVisitor {
            type Value = OrderedExportObject;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a CUE4Parse DataTable object")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut properties = None;
                let mut rows = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "Properties" => properties = Some(map.next_value::<Value>()?),
                        "Rows" => rows = Some(map.next_value::<OrderedRowList>()?),
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(OrderedExportObject { properties, rows })
            }
        }
        deserializer.deserialize_map(ObjVisitor)
    }
}

struct OrderedRowList(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for OrderedRowList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct RowsVisitor;
        impl<'de> Visitor<'de> for RowsVisitor {
            type Value = OrderedRowList;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a DataTable Rows map")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some((k, v)) = map.next_entry::<String, Value>()? {
                    entries.push((k, v));
                }
                Ok(OrderedRowList(entries))
            }
        }
        deserializer.deserialize_map(RowsVisitor)
    }
}

fn optional_str(obj: &Map<String, Value>, field: &str) -> Option<String> {
    obj.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn extract_soft_asset_path(obj: &Map<String, Value>, field: &str) -> Option<String> {
    let raw = obj
        .get(field)
        .and_then(Value::as_object)
        .and_then(|sub| sub.get("AssetPathName"))
        .and_then(Value::as_str)?;
    normalize_ue_asset_path(raw)
}

fn parse_transition_list(raw: Option<&Value>) -> Vec<TalkEventTransition> {
    let Some(items) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let event_id = optional_str(obj, "EventId")
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        if event_id.is_empty() || event_id.eq_ignore_ascii_case("None") {
            continue;
        }
        let condition = optional_str(obj, "Condition").unwrap_or_default();
        out.push(TalkEventTransition {
            condition,
            event_id,
        });
    }
    out
}

fn fallback_namespace_from_table_id(table_id: &str) -> Option<String> {
    let after_slash = table_id.rsplit('/').next().unwrap_or(table_id);
    let stem = after_slash.split('.').next().unwrap_or(after_slash);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_owned())
    }
}
