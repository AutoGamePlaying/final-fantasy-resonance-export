//! Typed loaders and queries for Scenario tables: `DT_ChapterData` and `DT_MainScenario`.
//!
//! Chapter membership of main scenario entries comes from `DT_MainScenario.Condition`
//! (`"{progress:0}>=<N>"`), where progress `1000..1999` is chapter 1, `2000..2999` is chapter 2,
//! and so on through chapter 8.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::datatable::{DataTable, load_data_table};
use crate::error::{DataError, Result};
use crate::ftext::{
    FTextValue, ResolveOptions, ResolvedText, TextResolver, TextResolverOptions, TextSource,
    parse_ftext,
};
use crate::paths::datatable_dir;

/// Relative path of `DT_ChapterData` under `FFRS/Content/Datatable/`.
pub const CHAPTER_DATA_TABLE: &str = "Scenario/DT_ChapterData";
/// Relative path of `DT_MainScenario` under `FFRS/Content/Datatable/`.
pub const MAIN_SCENARIO_TABLE: &str = "Scenario/DT_MainScenario";
/// Base ID offset for main quests in `DA_Quest` (`EQuestCategory::Main.IdBase = 1000`).
///
/// The game package contains no `DT_QuestData` table (`eDataTableType::QuestData` is unused);
/// `CPP_QuestSubsystem` synthesizes `MainQuestId = MAIN_QUEST_ID_BASE + ChapterNo` (`1001..=1009`)
/// from `DA_Quest`, `DT_ChapterData`, and `DT_MainScenario`.
pub const MAIN_QUEST_ID_BASE: i64 = 1000;
/// UE `TableId` for `ST_UI_QuestMenu` containing the `QuestChapter` title format string.
pub const QUEST_MENU_STRING_TABLE: &str =
    "/Game/Datatable/Strings/UI/UI_Common/ST_UI_QuestMenu.ST_UI_QuestMenu";

/// One chapter record from `Scenario/DT_ChapterData.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub row_key: String,
    pub chapter_no: i64,
    pub name: FTextValue,
}

/// One navigation destination entry in `DT_MainScenario.DestinationList`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioDestination {
    pub point_type: String,
    pub destination_type: String,
    pub destination_id: i64,
    pub is_point_to_object: bool,
    pub object_type: String,
    pub object_id: i64,
    pub condition: String,
}

/// One main scenario phase from `Scenario/DT_MainScenario.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainScenarioEntry {
    pub row_key: String,
    pub condition: String,
    pub min_progress: Option<i64>,
    pub chapter_no: Option<i64>,
    pub text: FTextValue,
    pub summary_text: FTextValue,
    pub destinations: Vec<ScenarioDestination>,
}

/// Resolved objective and summary texts for one [`MainScenarioEntry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMainScenarioTexts {
    pub text: ResolvedText,
    pub summary_text: ResolvedText,
}

/// Main quest record matching `CPP_QuestSubsystem.MainQuestId` (`1000 + ChapterNo`).
///
/// Synthesized from `DT_ChapterData` and `DT_MainScenario` following `DA_Quest`
/// (`EQuestCategory::Main.IdBase = 1000`), because no standalone `DT_QuestData` table exists in
/// the game package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainQuest {
    pub quest_id: i64,
    pub chapter_no: i64,
    pub chapter_row_key: String,
    pub chapter_name: FTextValue,
    /// Ordered main scenario phases belonging to this chapter.
    pub phases: Vec<MainScenarioEntry>,
}

/// Resolved localized title and phase objectives for one [`MainQuest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMainQuest {
    pub quest_id: i64,
    pub chapter_no: i64,
    /// Formatted title (e.g. `"第1章探究的开端"` in `zh-Hans`), matching `ST_UI_QuestMenu/QuestChapter`.
    pub title: ResolvedText,
    /// Raw chapter display title from `DT_ChapterData`.
    pub chapter_name: ResolvedText,
    /// Objective text for each phase in this chapter, in progress order.
    pub objectives: Vec<ResolvedText>,
}

/// Active main-story progress phase resolved for a save's `{progress:0}` value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentMainScenario {
    pub progress: i64,
    /// Active chapter number (`Some(0)` during prologue before chapter 1's first scenario phase;
    /// `Some(1..=9)` otherwise).
    pub chapter_no: Option<i64>,
    pub is_prologue: bool,
    /// Corresponding `MainQuestId` (`1000 + chapter_no`) when the chapter exists in `DT_ChapterData`.
    pub main_quest_id: Option<i64>,
    /// Active `DT_MainScenario` row whose progress segment covers `progress`, if any.
    pub entry: Option<MainScenarioEntry>,
    /// Inclusive lower bound of the active progress segment.
    pub progress_start: Option<i64>,
    /// Exclusive upper bound (start of the next `DT_MainScenario` phase), if any.
    pub next_progress: Option<i64>,
    /// Deduplicated destination map IDs from `entry.destinations` in table order.
    pub destination_map_ids: Vec<i64>,
}

/// Loaded Scenario tables paired with a cached [`TextResolver`].
#[derive(Debug, Clone)]
pub struct ScenarioData {
    pub chapters: Vec<Chapter>,
    pub main_scenario: Vec<MainScenarioEntry>,
    pub text_resolver: TextResolver,
}

impl ScenarioData {
    /// Load `DT_ChapterData` and `DT_MainScenario` from `export_root` with default text options
    /// (`language = "zh-Hans"`, strict missing reporting).
    pub fn load(export_root: impl Into<PathBuf>) -> Result<Self> {
        Self::load_with_options(export_root, TextResolverOptions::default())
    }

    /// Load Scenario tables together with a customized [`TextResolver`].
    pub fn load_with_options(
        export_root: impl Into<PathBuf>,
        options: TextResolverOptions,
    ) -> Result<Self> {
        let export_root = export_root.into();
        let chapters = load_chapters(&export_root)?;
        let main_scenario = load_main_scenario(&export_root)?;
        let text_resolver = TextResolver::with_options(export_root, options);
        Ok(Self {
            chapters,
            main_scenario,
            text_resolver,
        })
    }

    /// Find chapter `chapter_no`, or `None` if absent.
    pub fn find_chapter(&self, chapter_no: i64) -> Option<&Chapter> {
        find_chapter(&self.chapters, chapter_no)
    }

    /// Resolve the display title of chapter `chapter_no` in the resolver's default language.
    pub fn chapter_name(&self, chapter_no: i64) -> ResolvedText {
        self.chapter_name_with(chapter_no, None, ResolveOptions::default())
    }

    /// Resolve the display title of chapter `chapter_no` in an explicit language.
    pub fn chapter_name_in(&self, chapter_no: i64, language: &str) -> ResolvedText {
        self.chapter_name_with(chapter_no, Some(language), ResolveOptions::default())
    }

    /// Resolve the display title of chapter `chapter_no` with optional language and fallback overrides.
    pub fn chapter_name_with(
        &self,
        chapter_no: i64,
        language: Option<&str>,
        options: ResolveOptions,
    ) -> ResolvedText {
        let target_language = language.unwrap_or_else(|| self.text_resolver.language());
        let Some(chapter) = self.find_chapter(chapter_no) else {
            return ResolvedText {
                text: None,
                lang: None,
                requested_language: target_language.to_owned(),
                reason: Some(format!("章节表中找不到第 {chapter_no} 章")),
                source: TextSource::Missing,
            };
        };
        self.text_resolver
            .resolve_with(&chapter.name, Some(target_language), options)
    }

    /// Return all main scenario entries belonging to chapter `chapter_no`, sorted by `min_progress`.
    pub fn chapter_main_scenario(&self, chapter_no: i64) -> Vec<&MainScenarioEntry> {
        get_chapter_main_scenario(&self.main_scenario, chapter_no)
    }

    /// Resolve both objective and summary text of one main scenario entry in the default language.
    pub fn resolve_main_scenario_texts(
        &self,
        entry: &MainScenarioEntry,
    ) -> ResolvedMainScenarioTexts {
        self.resolve_main_scenario_texts_with(entry, None, ResolveOptions::default())
    }

    /// Resolve both objective and summary text of one main scenario entry in an explicit language.
    pub fn resolve_main_scenario_texts_in(
        &self,
        entry: &MainScenarioEntry,
        language: &str,
    ) -> ResolvedMainScenarioTexts {
        self.resolve_main_scenario_texts_with(entry, Some(language), ResolveOptions::default())
    }

    /// Resolve both objective and summary text with optional language and fallback overrides.
    pub fn resolve_main_scenario_texts_with(
        &self,
        entry: &MainScenarioEntry,
        language: Option<&str>,
        options: ResolveOptions,
    ) -> ResolvedMainScenarioTexts {
        ResolvedMainScenarioTexts {
            text: self
                .text_resolver
                .resolve_with(&entry.text, language, options),
            summary_text: self
                .text_resolver
                .resolve_with(&entry.summary_text, language, options),
        }
    }

    // ---- Main quests (`MainQuestId = 1000 + ChapterNo`) ----

    /// Return all synthesized main quests (`1001..=1009`), sorted by `quest_id` ascending.
    pub fn main_quests(&self) -> Vec<MainQuest> {
        self.chapters
            .iter()
            .filter_map(|chapter| {
                let quest_id = main_quest_id_for_chapter(chapter.chapter_no)?;
                let phases = self
                    .chapter_main_scenario(chapter.chapter_no)
                    .into_iter()
                    .cloned()
                    .collect();
                Some(MainQuest {
                    quest_id,
                    chapter_no: chapter.chapter_no,
                    chapter_row_key: chapter.row_key.clone(),
                    chapter_name: chapter.name.clone(),
                    phases,
                })
            })
            .collect()
    }

    /// Look up the main quest belonging to `chapter_no` (1-based).
    pub fn main_quest_by_chapter(&self, chapter_no: i64) -> Option<MainQuest> {
        let quest_id = main_quest_id_for_chapter(chapter_no)?;
        let chapter = self.find_chapter(chapter_no)?;
        let phases = self
            .chapter_main_scenario(chapter_no)
            .into_iter()
            .cloned()
            .collect();
        Some(MainQuest {
            quest_id,
            chapter_no: chapter.chapter_no,
            chapter_row_key: chapter.row_key.clone(),
            chapter_name: chapter.name.clone(),
            phases,
        })
    }

    /// Look up a main quest by its runtime `MainQuestId` (`1000 + ChapterNo`).
    pub fn main_quest_by_id(&self, main_quest_id: i64) -> Option<MainQuest> {
        let chapter_no = chapter_no_from_main_quest_id(main_quest_id)?;
        self.main_quest_by_chapter(chapter_no)
    }

    /// Resolve a main quest's formatted display title in the resolver's default language (`"zh-Hans"`).
    pub fn main_quest_title(&self, main_quest_id: i64) -> ResolvedText {
        self.main_quest_title_in(main_quest_id, self.text_resolver.language())
    }

    /// Resolve a main quest's formatted display title in an explicit language.
    ///
    /// Uses `ST_UI_QuestMenu/QuestChapter` (`"第{Chapter}章{Name}"` in `zh-Hans`) when exported,
    /// falling back to the language's standard chapter-title format only after `DT_ChapterData.Name`
    /// itself has resolved strictly in `language`.
    pub fn main_quest_title_in(&self, main_quest_id: i64, language: &str) -> ResolvedText {
        let Some(chapter_no) = chapter_no_from_main_quest_id(main_quest_id) else {
            return ResolvedText {
                text: None,
                lang: None,
                requested_language: language.to_owned(),
                reason: Some(format!("无效的主线任务编号：{main_quest_id}")),
                source: TextSource::Missing,
            };
        };
        let chapter_name = self.chapter_name_in(chapter_no, language);
        let Some(name_text) = chapter_name.text.as_deref() else {
            return chapter_name;
        };
        let formatted = self.format_chapter_quest_title(chapter_no, name_text, language);
        ResolvedText {
            text: Some(formatted),
            lang: chapter_name.lang,
            requested_language: chapter_name.requested_language,
            reason: chapter_name.reason,
            source: chapter_name.source,
        }
    }

    /// Resolve the title and all phase objectives of main quest `main_quest_id` in the default language.
    pub fn resolve_main_quest(&self, main_quest_id: i64) -> Option<ResolvedMainQuest> {
        self.resolve_main_quest_in(main_quest_id, self.text_resolver.language())
    }

    /// Resolve the title and all phase objectives of main quest `main_quest_id` in `language`.
    pub fn resolve_main_quest_in(
        &self,
        main_quest_id: i64,
        language: &str,
    ) -> Option<ResolvedMainQuest> {
        let quest = self.main_quest_by_id(main_quest_id)?;
        let title = self.main_quest_title_in(main_quest_id, language);
        let chapter_name = self.chapter_name_in(quest.chapter_no, language);
        let objectives = quest
            .phases
            .iter()
            .map(|phase| self.text_resolver.resolve_in(&phase.text, language))
            .collect();
        Some(ResolvedMainQuest {
            quest_id: quest.quest_id,
            chapter_no: quest.chapter_no,
            title,
            chapter_name,
            objectives,
        })
    }

    /// Resolve the active objective & summary text for `main_quest_id` at a save's `progress` value.
    pub fn main_quest_objective_at_progress(
        &self,
        main_quest_id: i64,
        progress: i64,
    ) -> Option<ResolvedMainScenarioTexts> {
        let chapter_no = chapter_no_from_main_quest_id(main_quest_id)?;
        let entry = self.phase_at_progress(progress)?;
        if entry.chapter_no != Some(chapter_no) {
            return None;
        }
        Some(self.resolve_main_scenario_texts(entry))
    }

    // ---- Current main scenario phase by save `progress` ----

    /// Look up the active [`MainScenarioEntry`] covering `progress` (`{progress:0}`).
    ///
    /// Matches the entry in `chapter_no_from_progress(progress)` with the largest
    /// `min_progress <= progress`. Returns `None` during title/prologue (before chapter 1's first
    /// milestone) and post-game (`progress >= 9000` when no chapter-9 rows exist).
    pub fn phase_at_progress(&self, progress: i64) -> Option<&MainScenarioEntry> {
        let chapter = chapter_no_from_progress(progress)?;
        self.main_scenario
            .iter()
            .filter_map(|entry| {
                if entry.chapter_no != Some(chapter) {
                    return None;
                }
                let min_p = entry.min_progress?;
                (min_p <= progress).then_some((min_p, entry))
            })
            .max_by_key(|(min_p, _)| *min_p)
            .map(|(_, entry)| entry)
    }

    /// Resolve the full current main-story stage (chapter, progress segment, destination maps, and
    /// corresponding `MainQuestId`) for a save's `progress` value (`{progress:0}`).
    ///
    /// Returns `None` when `progress < 1000` (e.g. title placeholder `0`).
    pub fn current_main_scenario(&self, progress: i64) -> Option<CurrentMainScenario> {
        let raw_chapter = chapter_no_from_progress(progress)?;

        let first_ch1_progress = self
            .chapter_main_scenario(1)
            .first()
            .and_then(|entry| entry.min_progress);
        if raw_chapter == 1
            && let Some(first_ch1) = first_ch1_progress
            && progress < first_ch1
        {
            return Some(CurrentMainScenario {
                progress,
                chapter_no: Some(0),
                is_prologue: true,
                main_quest_id: None,
                entry: None,
                progress_start: Some(1000),
                next_progress: Some(first_ch1),
                destination_map_ids: Vec::new(),
            });
        }

        let entry = self.phase_at_progress(progress).cloned();
        let progress_start = entry.as_ref().and_then(|e| e.min_progress);
        let next_progress = progress_start.and_then(|start| {
            self.main_scenario
                .iter()
                .filter_map(|e| e.min_progress)
                .filter(|&p| p > start)
                .min()
        });
        let mut destination_map_ids = Vec::new();
        if let Some(active) = &entry {
            for dest in &active.destinations {
                if !destination_map_ids.contains(&dest.destination_id) {
                    destination_map_ids.push(dest.destination_id);
                }
            }
        }

        let main_quest_id = self
            .find_chapter(raw_chapter)
            .and_then(|_| main_quest_id_for_chapter(raw_chapter));

        Some(CurrentMainScenario {
            progress,
            chapter_no: Some(raw_chapter),
            is_prologue: false,
            main_quest_id,
            entry,
            progress_start,
            next_progress,
            destination_map_ids,
        })
    }

    fn format_chapter_quest_title(
        &self,
        chapter_no: i64,
        chapter_name: &str,
        language: &str,
    ) -> String {
        let format_ftext = FTextValue::StringTable {
            table_id: QUEST_MENU_STRING_TABLE.to_owned(),
            key: "QuestChapter".to_owned(),
            source_string: None,
            localized_string: None,
        };
        let template = self
            .text_resolver
            .resolve_with(&format_ftext, Some(language), ResolveOptions::strict())
            .text;
        let pattern = template.as_deref().unwrap_or(match language {
            "zh-Hans" => "第{Chapter}章{Name}",
            "ja" => "第{Chapter}章 {Name}",
            "en" => "Chapter {Chapter} - {Name}",
            _ => "Chapter {Chapter} {Name}",
        });
        pattern
            .replace("{Chapter}", &chapter_no.to_string())
            .replace("{Name}", chapter_name)
    }
}

/// Convert a 1-based chapter number (`DT_ChapterData.ChapterNo`) to its runtime `MainQuestId`.
pub const fn main_quest_id_for_chapter(chapter_no: i64) -> Option<i64> {
    if chapter_no < 1 {
        return None;
    }
    Some(MAIN_QUEST_ID_BASE + chapter_no)
}

/// Convert a runtime `MainQuestId` (`1001..=1009`) back to its 1-based chapter number.
pub const fn chapter_no_from_main_quest_id(main_quest_id: i64) -> Option<i64> {
    if main_quest_id <= MAIN_QUEST_ID_BASE {
        return None;
    }
    Some(main_quest_id - MAIN_QUEST_ID_BASE)
}

/// Parse `"{progress:0}>=<N>"` into the numeric threshold `N`, or `None` if not in that form.
pub fn parse_main_progress_condition(condition: &str) -> Option<i64> {
    let digits = condition.trim().strip_prefix("{progress:0}>=")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// Derive chapter number from main progress: `1000..1999 -> 1`, `2000..2999 -> 2`, etc.
pub fn chapter_no_from_progress(progress: i64) -> Option<i64> {
    if progress < 1000 {
        return None;
    }
    Some(progress / 1000)
}

/// Parse a generic [`DataTable`] into typed [`Chapter`] records sorted by `chapter_no` ascending.
pub fn parse_chapters(table: &DataTable, source_label: &str) -> Result<Vec<Chapter>> {
    if table.row_struct != "ChapterData" {
        return Err(DataError::UnexpectedRowStruct {
            path: source_label.to_owned(),
            expected: "ChapterData".to_owned(),
            actual: table.row_struct.clone(),
        });
    }

    let mut chapters = Vec::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = raw_row
            .as_object()
            .ok_or_else(|| DataError::invalid_schema(source_label, &row_prefix, "行值不是对象"))?;
        chapters.push(Chapter {
            row_key: row_key.clone(),
            chapter_no: expect_i64(row, "ChapterNo", source_label, &row_prefix)?,
            name: expect_ftext(row, "Name", source_label, &row_prefix)?,
        });
    }
    chapters.sort_by_key(|chapter| chapter.chapter_no);
    Ok(chapters)
}

/// Load `Scenario/DT_ChapterData.json` into typed [`Chapter`] records sorted by `chapter_no`.
pub fn load_chapters(export_root: impl AsRef<Path>) -> Result<Vec<Chapter>> {
    let path = datatable_dir(export_root)
        .join("Scenario")
        .join("DT_ChapterData.json");
    let table = load_data_table(&path)?;
    let label = path.display().to_string();
    parse_chapters(&table, &label)
}

/// Parse a generic [`DataTable`] into typed [`MainScenarioEntry`] records.
pub fn parse_main_scenario(
    table: &DataTable,
    source_label: &str,
) -> Result<Vec<MainScenarioEntry>> {
    if table.row_struct != "ScenarioSection" {
        return Err(DataError::UnexpectedRowStruct {
            path: source_label.to_owned(),
            expected: "ScenarioSection".to_owned(),
            actual: table.row_struct.clone(),
        });
    }

    let mut entries = Vec::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = raw_row
            .as_object()
            .ok_or_else(|| DataError::invalid_schema(source_label, &row_prefix, "行值不是对象"))?;
        let condition = expect_string(row, "Condition", source_label, &row_prefix)?;
        let min_progress = parse_main_progress_condition(&condition);
        let chapter_no = min_progress.and_then(chapter_no_from_progress);

        let dest_field = format!("{row_prefix}.DestinationList");
        let raw_destinations = row
            .get("DestinationList")
            .and_then(Value::as_array)
            .ok_or_else(|| DataError::invalid_schema(source_label, &dest_field, "不是数组"))?;

        let mut destinations = Vec::with_capacity(raw_destinations.len());
        for (index, item) in raw_destinations.iter().enumerate() {
            let item_prefix = format!("{dest_field}[{index}]");
            destinations.push(parse_destination(item, source_label, &item_prefix)?);
        }

        entries.push(MainScenarioEntry {
            row_key: row_key.clone(),
            condition,
            min_progress,
            chapter_no,
            text: expect_ftext(row, "Text", source_label, &row_prefix)?,
            summary_text: expect_ftext(row, "SummaryText", source_label, &row_prefix)?,
            destinations,
        });
    }
    Ok(entries)
}

/// Load `Scenario/DT_MainScenario.json` into typed [`MainScenarioEntry`] records.
pub fn load_main_scenario(export_root: impl AsRef<Path>) -> Result<Vec<MainScenarioEntry>> {
    let path = datatable_dir(export_root)
        .join("Scenario")
        .join("DT_MainScenario.json");
    let table = load_data_table(&path)?;
    let label = path.display().to_string();
    parse_main_scenario(&table, &label)
}

/// Find chapter `chapter_no` in a loaded chapter list.
pub fn find_chapter(chapters: &[Chapter], chapter_no: i64) -> Option<&Chapter> {
    chapters
        .iter()
        .find(|chapter| chapter.chapter_no == chapter_no)
}

/// Return all main scenario entries belonging to `chapter_no`, sorted by `min_progress` ascending.
pub fn get_chapter_main_scenario(
    entries: &[MainScenarioEntry],
    chapter_no: i64,
) -> Vec<&MainScenarioEntry> {
    let mut matched: Vec<&MainScenarioEntry> = entries
        .iter()
        .filter(|entry| entry.chapter_no == Some(chapter_no))
        .collect();
    matched.sort_by_key(|entry| entry.min_progress.unwrap_or(0));
    matched
}

fn parse_destination(
    value: &Value,
    source_label: &str,
    prefix: &str,
) -> Result<ScenarioDestination> {
    let obj = value
        .as_object()
        .ok_or_else(|| DataError::invalid_schema(source_label, prefix, "不是对象"))?;

    let dest_key_prefix = format!("{prefix}.DestinationKey");
    let dest_key = obj
        .get("DestinationKey")
        .and_then(Value::as_object)
        .ok_or_else(|| DataError::invalid_schema(source_label, &dest_key_prefix, "缺少对象"))?;

    let object_prefix = format!("{prefix}.Object");
    let object_ref = obj
        .get("Object")
        .and_then(Value::as_object)
        .ok_or_else(|| DataError::invalid_schema(source_label, &object_prefix, "缺少对象"))?;

    Ok(ScenarioDestination {
        point_type: expect_string(obj, "PointType", source_label, prefix)?,
        destination_type: expect_string(dest_key, "Type", source_label, &dest_key_prefix)?,
        destination_id: expect_i64(dest_key, "DestinationId", source_label, &dest_key_prefix)?,
        is_point_to_object: expect_bool(obj, "IsPointToObject", source_label, prefix)?,
        object_type: expect_string(object_ref, "Type", source_label, &object_prefix)?,
        object_id: expect_i64(object_ref, "ID", source_label, &object_prefix)?,
        condition: expect_string(obj, "Condition", source_label, prefix)?,
    })
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

fn expect_ftext(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<FTextValue> {
    object.get(field).and_then(parse_ftext).ok_or_else(|| {
        DataError::invalid_schema(
            source_label,
            format!("{prefix}.{field}"),
            "不是合法的 FText",
        )
    })
}
