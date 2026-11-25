//! Typed loaders for side quests (`Request/DT_RequestData`) and quest-board delivery entries
//! (`Quest/CDT_QuestBoardData`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::datatable::{DataTable, load_data_table};
use crate::error::{DataError, Result};
use crate::ftext::FTextValue;
use crate::paths::datatable_dir;
use crate::scenario::{ScenarioDestination, chapter_no_from_progress};
use crate::world::parse::{
    expect_array, expect_ftext, expect_i64, expect_object, expect_row_object, expect_row_struct,
    expect_string, extract_main_progress_threshold, non_negative_id, parse_scenario_destination,
    positive_id,
};

/// Relative path of `DT_RequestData` under `FFRS/Content/Datatable/`.
pub const REQUEST_DATA_TABLE: &str = "Request/DT_RequestData";
/// Relative path of `CDT_QuestBoardData` under `FFRS/Content/Datatable/`.
pub const QUEST_BOARD_TABLE: &str = "Quest/CDT_QuestBoardData";

/// One stage objective inside a side quest (`RequestData.ProgressList[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestObjective {
    pub condition: String,
    pub goal_text: FTextValue,
    pub summary_text: FTextValue,
    pub destinations: Vec<ScenarioDestination>,
    pub quest_board_ids: Vec<i64>,
}

/// One reward entry inside a side quest (`RequestData.Prize[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestPrize {
    /// Global `ItemId` when `ID > 0`; `None` for non-item bonus rewards described in `text`.
    pub item_id: Option<i64>,
    pub count: i64,
    pub text: FTextValue,
}

/// One side quest / character request from `Request/DT_RequestData.json` (`RowStruct = RequestData`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quest {
    pub id: i64,
    pub row_key: String,
    pub title: FTextValue,
    pub requester: FTextValue,
    pub requester_map_id: Option<i64>,
    pub requester_object_type: String,
    pub requester_object_id: Option<i64>,
    pub report_map_id: Option<i64>,
    pub category: String,
    /// Minimum `{progress:0}` threshold found in `accept_condition` or `occurrence_condition`.
    pub min_main_progress: Option<i64>,
    /// Derived chapter number (`1..8` or `9` post-game) when a `{progress:0}>=<N>` threshold exists.
    pub chapter_no: Option<i64>,
    pub accept_condition: String,
    pub occurrence_condition: String,
    pub achievement_condition: String,
    pub completion_condition: String,
    pub objectives: Vec<QuestObjective>,
    pub ending_summary: FTextValue,
    pub prizes: Vec<QuestPrize>,
}

/// One delivery / target clear condition on a quest-board entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestClearCondition {
    pub target_id: i64,
    pub delivery_quantity: i64,
}

/// One reward on a quest-board entry (`Quest_BaseMenu_Data.RewardList[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestBoardReward {
    pub money: Option<i64>,
    pub item_id: Option<i64>,
    pub item_num: Option<i64>,
}

/// One entry from `Quest/CDT_QuestBoardData.json` (`RowStruct = Quest_BaseMenu_Data`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestBoardEntry {
    pub row_key: String,
    pub quest_type: String,
    pub quest_board_id: i64,
    pub quest_id: i64,
    pub title: FTextValue,
    pub description: FTextValue,
    pub map_name: FTextValue,
    pub condition: String,
    pub clear_conditions: Vec<QuestClearCondition>,
    pub rewards: Vec<QuestBoardReward>,
    pub event_ids: Vec<String>,
}

impl QuestBoardEntry {
    /// Returns `true` for placeholder rows (`QuestId < 0`, such as `NewRow`).
    pub fn is_placeholder(&self) -> bool {
        self.quest_id < 0
    }
}

/// Parse `Request/DT_RequestData` into typed [`Quest`] records sorted by `id` ascending.
pub fn parse_quests(table: &DataTable, source_label: &str) -> Result<Vec<Quest>> {
    expect_row_struct(table, "RequestData", source_label)?;
    let mut quests = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;

        let req_id_prefix = format!("{row_prefix}.RequesterId");
        let req_id_obj = expect_object(row, "RequesterId", source_label, &row_prefix)?;
        let req_data_prefix = format!("{req_id_prefix}.Data");
        let req_data = expect_object(req_id_obj, "Data", source_label, &req_id_prefix)?;

        let report_prefix = format!("{row_prefix}.ReportDestinationId");
        let report_obj = expect_object(row, "ReportDestinationId", source_label, &row_prefix)?;

        let occ_prefix = format!("{row_prefix}.Occurrence");
        let occ_obj = expect_object(row, "Occurrence", source_label, &row_prefix)?;

        let accept_condition = expect_string(row, "AcceptCondition", source_label, &row_prefix)?;
        let occurrence_condition = expect_string(occ_obj, "Condition", source_label, &occ_prefix)?;
        let min_main_progress = extract_main_progress_threshold(&accept_condition)
            .or_else(|| extract_main_progress_threshold(&occurrence_condition));
        let chapter_no = min_main_progress.and_then(chapter_no_from_progress);

        let raw_progress = expect_array(row, "ProgressList", source_label, &row_prefix)?;
        let mut objectives = Vec::with_capacity(raw_progress.len());
        for (idx, item) in raw_progress.iter().enumerate() {
            let stage_prefix = format!("{row_prefix}.ProgressList[{idx}]");
            let stage = expect_row_object(item, source_label, &stage_prefix)?;

            let raw_dests = expect_array(stage, "DestinationList", source_label, &stage_prefix)?;
            let mut destinations = Vec::with_capacity(raw_dests.len());
            for (d_idx, dest_val) in raw_dests.iter().enumerate() {
                let d_prefix = format!("{stage_prefix}.DestinationList[{d_idx}]");
                destinations.push(parse_scenario_destination(
                    dest_val,
                    source_label,
                    &d_prefix,
                )?);
            }

            let raw_qids = expect_array(stage, "QuestList", source_label, &stage_prefix)?;
            let mut quest_board_ids = Vec::with_capacity(raw_qids.len());
            for (q_idx, q_val) in raw_qids.iter().enumerate() {
                let qid = q_val.as_i64().ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("{stage_prefix}.QuestList[{q_idx}]"),
                        "不是整数",
                    )
                })?;
                quest_board_ids.push(qid);
            }

            objectives.push(QuestObjective {
                condition: expect_string(stage, "Condition", source_label, &stage_prefix)?,
                goal_text: expect_ftext(stage, "DestinationText", source_label, &stage_prefix)?,
                summary_text: expect_ftext(stage, "SummaryText", source_label, &stage_prefix)?,
                destinations,
                quest_board_ids,
            });
        }

        let raw_prizes = expect_array(row, "Prize", source_label, &row_prefix)?;
        let mut prizes = Vec::with_capacity(raw_prizes.len());
        for (idx, item) in raw_prizes.iter().enumerate() {
            let prize_prefix = format!("{row_prefix}.Prize[{idx}]");
            let prize = expect_row_object(item, source_label, &prize_prefix)?;
            prizes.push(QuestPrize {
                item_id: positive_id(expect_i64(prize, "ID", source_label, &prize_prefix)?),
                count: expect_i64(prize, "Num", source_label, &prize_prefix)?,
                text: expect_ftext(prize, "Text", source_label, &prize_prefix)?,
            });
        }

        quests.push(Quest {
            id: expect_i64(row, "ID", source_label, &row_prefix)?,
            row_key: row_key.clone(),
            title: expect_ftext(row, "Name", source_label, &row_prefix)?,
            requester: expect_ftext(row, "Requester", source_label, &row_prefix)?,
            requester_map_id: non_negative_id(expect_i64(
                req_id_obj,
                "mapId",
                source_label,
                &req_id_prefix,
            )?),
            requester_object_type: expect_string(req_data, "Type", source_label, &req_data_prefix)?,
            requester_object_id: non_negative_id(expect_i64(
                req_data,
                "ID",
                source_label,
                &req_data_prefix,
            )?),
            report_map_id: non_negative_id(expect_i64(
                report_obj,
                "mapId",
                source_label,
                &report_prefix,
            )?),
            category: expect_string(row, "Category", source_label, &row_prefix)?,
            min_main_progress,
            chapter_no,
            accept_condition,
            occurrence_condition,
            achievement_condition: expect_string(
                row,
                "AchievementCondition",
                source_label,
                &row_prefix,
            )?,
            completion_condition: expect_string(
                row,
                "CompletionCondition",
                source_label,
                &row_prefix,
            )?,
            objectives,
            ending_summary: expect_ftext(row, "EndingSummaryText", source_label, &row_prefix)?,
            prizes,
        });
    }

    quests.sort_by_key(|quest| quest.id);
    Ok(quests)
}

/// Load `Request/DT_RequestData.json` from `export_root`.
pub fn load_quests(export_root: impl AsRef<Path>) -> Result<Vec<Quest>> {
    let path = datatable_dir(export_root)
        .join("Request")
        .join("DT_RequestData.json");
    let table = load_data_table(&path)?;
    parse_quests(&table, &path.display().to_string())
}

/// Parse `Quest/CDT_QuestBoardData` (`Quest_BaseMenu_Data`) into typed [`QuestBoardEntry`] records.
pub fn parse_quest_board(table: &DataTable, source_label: &str) -> Result<Vec<QuestBoardEntry>> {
    expect_row_struct(table, "Quest_BaseMenu_Data", source_label)?;
    let mut entries = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;

        let raw_clears = expect_array(row, "ClearConditionList", source_label, &row_prefix)?;
        let mut clear_conditions = Vec::new();
        for (idx, item) in raw_clears.iter().enumerate() {
            let c_prefix = format!("{row_prefix}.ClearConditionList[{idx}]");
            let obj = expect_row_object(item, source_label, &c_prefix)?;
            let target_id = expect_i64(obj, "ID", source_label, &c_prefix)?;
            let delivery_quantity = expect_i64(obj, "DeliveryQuantity", source_label, &c_prefix)?;
            if target_id >= 0 {
                clear_conditions.push(QuestClearCondition {
                    target_id,
                    delivery_quantity,
                });
            }
        }

        let raw_rewards = expect_array(row, "RewardList", source_label, &row_prefix)?;
        let mut rewards = Vec::new();
        for (idx, item) in raw_rewards.iter().enumerate() {
            let r_prefix = format!("{row_prefix}.RewardList[{idx}]");
            let obj = expect_row_object(item, source_label, &r_prefix)?;
            let money = non_negative_id(expect_i64(obj, "Money", source_label, &r_prefix)?);
            let item_id = non_negative_id(expect_i64(obj, "ItemId", source_label, &r_prefix)?);
            let item_num = non_negative_id(expect_i64(obj, "ItemNum", source_label, &r_prefix)?);
            if money.is_some() || item_id.is_some() {
                rewards.push(QuestBoardReward {
                    money,
                    item_id,
                    item_num,
                });
            }
        }

        let req_data_prefix = format!("{row_prefix}.RequestData");
        let req_data = expect_object(row, "RequestData", source_label, &row_prefix)?;
        let raw_events = expect_array(req_data, "EventList", source_label, &req_data_prefix)?;
        let mut event_ids = Vec::with_capacity(raw_events.len());
        for (idx, val) in raw_events.iter().enumerate() {
            let ev = val.as_str().ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{req_data_prefix}.EventList[{idx}]"),
                    "不是字符串",
                )
            })?;
            event_ids.push(ev.to_owned());
        }

        entries.push(QuestBoardEntry {
            row_key: row_key.clone(),
            quest_type: expect_string(row, "QuestType", source_label, &row_prefix)?,
            quest_board_id: expect_i64(row, "QuestBoardID", source_label, &row_prefix)?,
            quest_id: expect_i64(row, "QuestId", source_label, &row_prefix)?,
            title: expect_ftext(row, "QuestName", source_label, &row_prefix)?,
            description: expect_ftext(row, "Description", source_label, &row_prefix)?,
            map_name: expect_ftext(row, "MapName", source_label, &row_prefix)?,
            condition: expect_string(row, "Condition", source_label, &row_prefix)?,
            clear_conditions,
            rewards,
            event_ids,
        });
    }

    entries.sort_by_key(|entry| entry.quest_id);
    Ok(entries)
}

/// Load `Quest/CDT_QuestBoardData.json` from `export_root`.
pub fn load_quest_board(export_root: impl AsRef<Path>) -> Result<Vec<QuestBoardEntry>> {
    let path = datatable_dir(export_root)
        .join("Quest")
        .join("CDT_QuestBoardData.json");
    let table = load_data_table(&path)?;
    parse_quest_board(&table, &path.display().to_string())
}
