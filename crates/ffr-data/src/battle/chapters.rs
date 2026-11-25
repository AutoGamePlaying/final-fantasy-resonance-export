//! Derived Enemy -> Chapter index built from static encounter pools, chapter story/sub events,
//! trap treasure boxes, map islands, island release conditions, and main scenario destinations.
//!
//! Static Datatable exports do not include a direct `UnitId -> ChapterNo` column. This module
//! reconstructs chapter membership from three verifiable static linkages:
//! 1. Regional encounter pools (`Encount/<Region>/DT_*_Encount.json`) -> dungeon map or region
//!    island (`Map/CDT_MapData_Prod`) -> island chapter (`Map/DT_MapReleaseConditionData` with
//!    fallback to `Scenario/DT_MainScenario` destinations) -> `DT_BattleGroup.UnitIdList`.
//! 2. Chapter story & sub-event tables (`Event/TalkEvent/MainEvent/DT_TalkEventData_ChapterNN`
//!    and `Event/TalkEvent/SubEvent/DT_TalkEventData_SubEv_ChapterNN`) -> `encountGroupId` ->
//!    `DT_BattleGroup.UnitIdList`.
//! 3. Trap treasure boxes (`Map/Gimmick/TreasureBox/CDT_MapGimmickTreasureBox`) -> `mapId` ->
//!    map island chapter -> `DT_BattleGroup.UnitIdList`.
//!
//! Enemies appearing only in non-chapter-bound optional content (Colosseum rank battles,
//! StrongEnemy, 12Arms, Summon/Legend/Chara events) or unused in any battle group remain unmapped.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::battle::{BattleData, EnemyProfile, parse_encounter_pool};
use crate::datatable::load_data_table;
use crate::error::{DataError, Result};
use crate::paths::{datatable_dir, datatable_relative_path_to_json};
use crate::scenario::{MainScenarioEntry, chapter_no_from_progress, parse_main_progress_condition};
use crate::store::DataStore;
use crate::world::{
    MAP_DATA_TABLE, MAP_RELEASE_CONDITION_TABLE, MapInfo, MapReleaseCondition, TREASURE_BOX_TABLE,
    TreasureBox, parse_map_release_conditions, parse_maps, parse_treasure_boxes,
};

/// Origin of one derived `(unit_id, chapter_no)` link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnemyChapterSourceKind {
    EncounterPool,
    MainStoryEvent,
    SubStoryEvent,
    TrapTreasureBox,
}

/// Provenance record explaining why an enemy is associated with a chapter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnemyChapterSource {
    pub chapter_no: i64,
    pub kind: EnemyChapterSourceKind,
    pub battle_group_id: i64,
    /// Relative table path or composite key identifying the static source.
    pub source_ref: String,
}

/// Static Enemy <-> Chapter index derived from encounter pools, maps, story events, and chests.
#[derive(Debug, Clone, Default)]
pub struct EnemyChapterIndex {
    enemy_to_chapters: BTreeMap<i64, Vec<i64>>,
    chapter_to_enemies: BTreeMap<i64, Vec<i64>>,
    enemy_sources: BTreeMap<i64, Vec<EnemyChapterSource>>,
    island_chapters: BTreeMap<String, Vec<i64>>,
}

impl EnemyChapterIndex {
    /// Load and build the enemy-to-chapter index directly from an export root.
    pub fn load(export_root: impl Into<PathBuf>) -> Result<Self> {
        let store = DataStore::new(export_root);
        Self::from_store(&store)
    }

    /// Build the enemy-to-chapter index through a [`DataStore`], reusing its table caches.
    pub fn from_store(store: &DataStore) -> Result<Self> {
        let battle = store.battle()?;
        Self::from_store_with_battle(store, &battle)
    }

    /// Build the enemy-to-chapter index reusing an already-loaded [`BattleData`].
    pub fn from_store_with_battle(store: &DataStore, battle: &BattleData) -> Result<Self> {
        let maps = {
            let table = store.data_table(MAP_DATA_TABLE)?;
            let label = table_label(store, MAP_DATA_TABLE);
            parse_maps(&table, &label)?
        };
        let main_scenario = store.main_scenario()?;

        let release_conditions = match store.data_table(MAP_RELEASE_CONDITION_TABLE) {
            Ok(table) => {
                let label = table_label(store, MAP_RELEASE_CONDITION_TABLE);
                parse_map_release_conditions(&table, &label)?
            }
            Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };

        let treasure_boxes = match store.data_table(TREASURE_BOX_TABLE) {
            Ok(table) => {
                let label = table_label(store, TREASURE_BOX_TABLE);
                parse_treasure_boxes(&table, &label)?
            }
            Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };

        let island_chapters = derive_island_chapters(&maps, &release_conditions, &main_scenario);
        let region_islands = derive_region_default_islands(&maps);
        let maps_by_id: HashMap<i64, &MapInfo> = maps.iter().map(|m| (m.id, m)).collect();
        let maps_by_row_key: HashMap<&str, &MapInfo> =
            maps.iter().map(|m| (m.row_key.as_str(), m)).collect();

        let mut enemy_chapters_set: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
        let mut chapter_enemies_set: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
        let mut enemy_sources_set: BTreeMap<i64, BTreeSet<EnemyChapterSource>> = BTreeMap::new();

        let mut record_bg =
            |bg_id: i64, chapters: &[i64], kind: EnemyChapterSourceKind, source_ref: &str| {
                let Some(group) = battle.battle_group_by_id(bg_id) else {
                    return;
                };
                for &unit_id in &group.unit_ids {
                    let Some(unit) = battle.unit_by_id(unit_id) else {
                        continue;
                    };
                    if !unit.category.is_enemy_category() {
                        continue;
                    }
                    for &chapter_no in chapters {
                        enemy_chapters_set
                            .entry(unit_id)
                            .or_default()
                            .insert(chapter_no);
                        chapter_enemies_set
                            .entry(chapter_no)
                            .or_default()
                            .insert(unit_id);
                        enemy_sources_set
                            .entry(unit_id)
                            .or_default()
                            .insert(EnemyChapterSource {
                                chapter_no,
                                kind,
                                battle_group_id: bg_id,
                                source_ref: source_ref.to_owned(),
                            });
                    }
                }
            };

        let dt_root = datatable_dir(store.export_root());

        // 1. Regional random encounter pools (`Encount/<Region>/DT_*_Encount.json`).
        let encount_root = dt_root.join("Encount");
        if encount_root.is_dir() {
            for region_dir in sorted_subdirs(&encount_root)? {
                let Some(region_name) = region_dir.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if region_name.eq_ignore_ascii_case("Staging") {
                    continue;
                }
                for file_path in sorted_json_files(&region_dir)? {
                    let Some(file_stem) = file_path.file_stem().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    let table = load_data_table(&file_path)?;
                    if table.row_struct != "ENCOUNT_GROUP_DATA" {
                        continue;
                    }
                    let label = file_path.display().to_string();
                    let entries = parse_encounter_pool(&table, &label)?;

                    let island = resolve_encounter_pool_island(
                        region_name,
                        file_stem,
                        &maps_by_row_key,
                        &maps,
                        &region_islands,
                    );
                    let Some(chapters) = island.and_then(|isl| island_chapters.get(isl)) else {
                        continue;
                    };
                    let source_ref = format!("Encount/{region_name}/{file_stem}");
                    for entry in entries {
                        record_bg(
                            entry.battle_group_id,
                            chapters,
                            EnemyChapterSourceKind::EncounterPool,
                            &source_ref,
                        );
                    }
                }
            }
        }

        // 2. Main-story and sub-event chapter battle triggers (`Event/TalkEvent/{MainEvent,SubEvent}`).
        for (subdir, kind) in [
            (
                "Event/TalkEvent/MainEvent",
                EnemyChapterSourceKind::MainStoryEvent,
            ),
            (
                "Event/TalkEvent/SubEvent",
                EnemyChapterSourceKind::SubStoryEvent,
            ),
        ] {
            let dir_path = dt_root.join(subdir);
            if !dir_path.is_dir() {
                continue;
            }
            for file_path in sorted_json_files(&dir_path)? {
                let Some(file_stem) = file_path.file_stem().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some(chapter_no) = parse_chapter_no_from_event_stem(file_stem) else {
                    continue;
                };
                let table = load_data_table(&file_path)?;
                if table.row_struct != "TalkEventData" {
                    continue;
                }
                let source_ref = format!("{subdir}/{file_stem}");
                for raw_row in table.rows.values() {
                    let Some(bg_id) = raw_row
                        .as_object()
                        .and_then(|obj| obj.get("encountGroupId"))
                        .and_then(Value::as_i64)
                    else {
                        continue;
                    };
                    if bg_id >= 0 {
                        record_bg(bg_id, &[chapter_no], kind, &source_ref);
                    }
                }
            }
        }

        // 3. Trap treasure boxes (`Map/Gimmick/TreasureBox/CDT_MapGimmickTreasureBox`).
        for chest in &treasure_boxes {
            if let Some(bg_id) = chest.battle_group_id
                && let Some(map) = maps_by_id.get(&chest.map_id)
                && let Some(chapters) = island_chapters.get(&map.island)
            {
                let source_ref = format!("TreasureBox(map={},id={})", chest.map_id, chest.id);
                record_bg(
                    bg_id,
                    chapters,
                    EnemyChapterSourceKind::TrapTreasureBox,
                    &source_ref,
                );
            }
        }

        let enemy_to_chapters = enemy_chapters_set
            .into_iter()
            .map(|(uid, set)| (uid, set.into_iter().collect()))
            .collect();
        let chapter_to_enemies = chapter_enemies_set
            .into_iter()
            .map(|(ch, set)| (ch, set.into_iter().collect()))
            .collect();
        let enemy_sources = enemy_sources_set
            .into_iter()
            .map(|(uid, set)| (uid, set.into_iter().collect()))
            .collect();

        Ok(Self {
            enemy_to_chapters,
            chapter_to_enemies,
            enemy_sources,
            island_chapters,
        })
    }

    /// Return the sorted chapter numbers in which enemy `unit_id` appears (empty if unmapped).
    pub fn chapters_for_enemy(&self, unit_id: i64) -> &[i64] {
        self.enemy_to_chapters
            .get(&unit_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return all enemy unit IDs mapped to `chapter_no`, sorted ascending.
    pub fn enemies_in_chapter(&self, chapter_no: i64) -> &[i64] {
        self.chapter_to_enemies
            .get(&chapter_no)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Resolve all enemies mapped to `chapter_no` into [`EnemyProfile`] records via `battle`.
    pub fn enemy_profiles_in_chapter(
        &self,
        battle: &BattleData,
        chapter_no: i64,
    ) -> Vec<EnemyProfile> {
        self.enemies_in_chapter(chapter_no)
            .iter()
            .filter_map(|&unit_id| {
                let unit = battle.unit_by_id(unit_id)?;
                Some(unit.to_enemy_profile(battle.unit_ai_by_id(unit_id)))
            })
            .collect()
    }

    /// Return provenance sources explaining why `unit_id` maps to its chapters.
    pub fn sources_for_enemy(&self, unit_id: i64) -> &[EnemyChapterSource] {
        self.enemy_sources
            .get(&unit_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return the derived island-to-chapter mapping used during indexing.
    pub fn island_chapters(&self) -> &BTreeMap<String, Vec<i64>> {
        &self.island_chapters
    }

    /// Number of distinct enemy/boss units mapped to at least one chapter.
    pub fn mapped_enemy_count(&self) -> usize {
        self.enemy_to_chapters.len()
    }

    /// Sorted list of all chapter numbers that have at least one mapped enemy.
    pub fn chapters(&self) -> Vec<i64> {
        self.chapter_to_enemies.keys().copied().collect()
    }
}

impl DataStore {
    /// Build a derived [`EnemyChapterIndex`] backed by this store's cached tables.
    pub fn enemy_chapters(&self) -> Result<EnemyChapterIndex> {
        EnemyChapterIndex::from_store(self)
    }
}

fn derive_island_chapters(
    maps: &[MapInfo],
    release_conditions: &[MapReleaseCondition],
    main_scenario: &[MainScenarioEntry],
) -> BTreeMap<String, Vec<i64>> {
    let mut island_to_chapters: BTreeMap<String, Vec<i64>> = BTreeMap::new();

    // Primary source: `DT_MapReleaseConditionData` (`{progress:0}>=(chapter + 1) * 1000`).
    for rel in release_conditions {
        if rel.island_type == "eMapIslandType::None" || rel.island_type.is_empty() {
            continue;
        }
        if let Some(threshold) = parse_main_progress_condition(&rel.condition)
            && threshold >= 2000
            && threshold % 1000 == 0
        {
            let chapter_no = (threshold / 1000) - 1;
            island_to_chapters.insert(rel.island_type.clone(), vec![chapter_no]);
        }
    }

    // Fallback for islands not listed in `DT_MapReleaseConditionData` (e.g. Lanzelt, Kolobos):
    // derive chapters from `DT_MainScenario.DestinationList` -> `CDT_MapData_Prod.island`.
    let maps_by_id: HashMap<i64, &MapInfo> = maps.iter().map(|m| (m.id, m)).collect();
    let mut scenario_islands: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for entry in main_scenario {
        let Some(chapter_no) = entry
            .chapter_no
            .or_else(|| entry.min_progress.and_then(chapter_no_from_progress))
        else {
            continue;
        };
        for dest in &entry.destinations {
            if let Some(map) = maps_by_id.get(&dest.destination_id)
                && map.island != "eMapIslandType::None"
                && !map.island.is_empty()
            {
                scenario_islands
                    .entry(map.island.clone())
                    .or_default()
                    .insert(chapter_no);
            }
        }
    }

    for (island, chapters) in scenario_islands {
        island_to_chapters
            .entry(island)
            .or_insert_with(|| chapters.into_iter().collect());
    }

    island_to_chapters
}

fn derive_region_default_islands(maps: &[MapInfo]) -> HashMap<String, String> {
    let mut counts: HashMap<String, BTreeMap<&str, usize>> = HashMap::new();
    for map in maps {
        if map.island == "eMapIslandType::None" || map.island.is_empty() {
            continue;
        }
        if let Some((prefix, _)) = map.row_key.split_once('_')
            && !prefix.is_empty()
        {
            *counts
                .entry(prefix.to_owned())
                .or_default()
                .entry(map.island.as_str())
                .or_insert(0) += 1;
        }
    }

    let mut result = HashMap::with_capacity(counts.len());
    for (prefix, island_counts) in counts {
        if let Some((&best_island, _)) =
            island_counts
                .iter()
                .max_by(|(a_isl, a_cnt), (b_isl, b_cnt)| {
                    a_cnt.cmp(b_cnt).then_with(|| b_isl.cmp(a_isl))
                })
        {
            result.insert(prefix, best_island.to_owned());
        }
    }
    result
}

fn resolve_encounter_pool_island<'a>(
    region_name: &str,
    file_stem: &str,
    maps_by_row_key: &HashMap<&str, &'a MapInfo>,
    maps: &'a [MapInfo],
    region_islands: &'a HashMap<String, String>,
) -> Option<&'a str> {
    let is_world_pool = file_stem.contains("_Wld_") || file_stem.ends_with("_Wld");
    if !is_world_pool && let Some(map_key) = extract_dungeon_map_key(region_name, file_stem) {
        if let Some(map) = maps_by_row_key.get(map_key.as_str())
            && map.island != "eMapIslandType::None"
            && !map.island.is_empty()
        {
            return Some(map.island.as_str());
        }
        let expected_level = format!("Dng_{map_key}_PL");
        if let Some(map) = maps.iter().find(|m| m.level_name == expected_level)
            && map.island != "eMapIslandType::None"
            && !map.island.is_empty()
        {
            return Some(map.island.as_str());
        }
    }

    region_islands.get(region_name).map(String::as_str)
}

/// Extract `<Region>_<DD>` from dungeon encounter pool stems such as:
/// - `DT_01Gra_40_01_Encount` -> `01Gra_40`
/// - `DT_Dng_04Dir_41_2_Encount` -> `04Dir_41`
/// - `DT_02Lan_Dng_40_01_Encount` -> `02Lan_40`
fn extract_dungeon_map_key(region_name: &str, file_stem: &str) -> Option<String> {
    let stripped = file_stem
        .strip_prefix("DT_")
        .unwrap_or(file_stem)
        .strip_suffix("_Encount")
        .unwrap_or(file_stem);
    let stripped = stripped.strip_prefix("Dng_").unwrap_or(stripped);
    let after_region = stripped.strip_prefix(region_name)?.strip_prefix('_')?;
    let after_dng = after_region.strip_prefix("Dng_").unwrap_or(after_region);
    let num_part = after_dng.split('_').next()?;
    if num_part.is_empty() || !num_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{region_name}_{num_part}"))
}

/// Extract `NN` from event table stems such as `DT_TalkEventData_Chapter01` or
/// `DT_TalkEventData_SubEv_Chapter09`.
fn parse_chapter_no_from_event_stem(file_stem: &str) -> Option<i64> {
    let (_, after) = file_stem.split_once("Chapter")?;
    let digits_len = after.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_len == 0 {
        return None;
    }
    let chapter = after[..digits_len].parse::<i64>().ok()?;
    (chapter >= 1).then_some(chapter)
}

fn sorted_subdirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).map_err(|source| DataError::Io {
        kind: "遇敌目录",
        path: dir.to_path_buf(),
        source,
    })?;
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| DataError::Io {
            kind: "遇敌目录",
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn sorted_json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).map_err(|source| DataError::Io {
        kind: "数据表目录",
        path: dir.to_path_buf(),
        source,
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| DataError::Io {
            kind: "数据表目录",
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn table_label(store: &DataStore, relative_path: &str) -> String {
    datatable_relative_path_to_json(relative_path, store.export_root()).map_or_else(
        || relative_path.to_owned(),
        |path| path.display().to_string(),
    )
}

// Keep `TreasureBox` import used in doc / type clarity.
const _: Option<TreasureBox> = None;
