//! Integration tests for the Task 20 `ffr-data` additions:
//! 1. `DT_CommandSkillData` typed queries (`CommandSkillData`)
//! 2. Main quests (`MainQuest` / `MainQuestId = 1000 + ChapterNo`)
//! 3. Current main scenario resolution by save `progress` (`CurrentMainScenario`)
//! 4. Playable field map-unit lookup (`DT_MapUnitPlayableData` + `DT_MapUnitAsset`)
//! 5. Named event flag conditions (`Flag/CDT_FlagCondition`)
//! 6. Derived Enemy -> Chapter index (`EnemyChapterIndex`)

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ffr_data::battle::{
    CommandSkillData, EnemyChapterIndex, EnemyChapterSourceKind, parse_command_skills,
};
use ffr_data::{
    DataError, DataStore, DataTable, DataTableType, FFR_DEPS_ENV, FlagConditionData, FlagTerm,
    MapUnitPlayableData, chapter_no_from_main_quest_id, datatable_dir, export_dir_from_deps,
    localization_file, main_quest_id_for_chapter, parse_flag_condition_expr, parse_flag_conditions,
    parse_map_unit_playable,
};
use serde_json::{Map, Value, json};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ffr-data-gaps-{prefix}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ffr-data sits two levels under repo root")
        .to_path_buf()
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create dir");
    for entry in fs::read_dir(src).expect("read dir") {
        let entry = entry.expect("dir entry");
        let target = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn build_combined_synthetic_export(dst: &Path) {
    let root = repo_root();
    let sources = [
        root.join("tests/fixtures/rust/world/export"),
        root.join("tests/fixtures/rust/battle"),
        root.join("tests/fixtures/rust/gaps/export"),
    ];
    for src in &sources {
        copy_dir_recursive(src, dst);
    }

    for lang in ["zh-Hans", "en", "ja"] {
        let mut merged = Map::new();
        for src in &sources {
            let loc_path = src.join(format!("Localization/{lang}.json"));
            if !loc_path.is_file() {
                continue;
            }
            let raw = fs::read_to_string(&loc_path).expect("read localization");
            let parsed: Value = serde_json::from_str(&raw).expect("parse localization");
            for (ns, entries) in parsed.as_object().expect("loc object") {
                merged.insert(ns.clone(), entries.clone());
            }
        }
        let out_path = dst.join(format!("Localization/{lang}.json"));
        fs::write(
            out_path,
            serde_json::to_vec_pretty(&Value::Object(merged)).expect("encode loc"),
        )
        .expect("write loc");
    }
}

fn real_export_dir() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return export_dir_from_deps(PathBuf::from(deps));
    }
    export_dir_from_deps(repo_root().join(".deps"))
}

// ---- 1. CommandSkillData ----

#[test]
fn command_skill_data_queries_and_validation() {
    let tmp = TempDir::new("cmd");
    build_combined_synthetic_export(tmp.path());

    let store = DataStore::new(tmp.path());
    let commands = store.command_skills().expect("load command skills");
    assert_eq!(commands.commands.len(), 6);
    assert!(commands.command_by_id(-1).is_none());

    let ability = commands.command_by_id(1).expect("command 1");
    assert_eq!(ability.row_key, "Ability");
    assert!(!ability.is_removable);
    assert_eq!(commands.command_name(1).text.as_deref(), Some("招式"));
    assert_eq!(
        commands.command_name_in(1, "en").text.as_deref(),
        Some("Abilities")
    );
    assert_eq!(
        commands.command_name_in(1, "ja").text.as_deref(),
        Some("アビリティ")
    );
    assert_eq!(
        commands.command_description(1).text.as_deref(),
        Some("使用已习得的招式")
    );

    let item_cmd = commands
        .commands_by_name("道具")
        .expect("lookup command by name");
    assert_eq!(item_cmd.len(), 1);
    assert_eq!(item_cmd[0].id, 7);

    let vision_cmds = commands.commands_for_unit(13017);
    assert_eq!(vision_cmds.len(), 1);
    assert_eq!(vision_cmds[0].id, 100);
    assert!(vision_cmds[0].is_removable);
    assert_eq!(vision_cmds[0].equip_cost, 3);

    assert!(commands.command_name(9999).text.is_none());
    assert!(commands.command_description(9999).text.is_none());

    // Cross-link: BattleData::skills_in_command filters by belong_command_ids.
    let battle = store.battle().expect("load battle");
    let ability_skills = battle.skills_in_command(1);
    assert!(!ability_skills.is_empty());
    assert!(
        ability_skills
            .iter()
            .all(|s| s.belong_command_ids.contains(&1))
    );

    // Schema error: wrong RowStruct.
    let bad_struct = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_CommandSkillData".to_owned(),
        package_path: "/Game/Datatable/Skill/DT_CommandSkillData".to_owned(),
        row_struct: "WRONG_STRUCT".to_owned(),
        rows: BTreeMap::new(),
    };
    assert!(matches!(
        parse_command_skills(&bad_struct, "fake.json"),
        Err(DataError::UnexpectedRowStruct { .. })
    ));

    // Schema error: duplicate command ID.
    let mut dup_rows = BTreeMap::new();
    let sample_row = json!({
        "ID": 1,
        "SortId": 1,
        "unitIdToUseSkill": -1,
        "Name": { "CultureInvariantString": "A" },
        "Description": { "CultureInvariantString": null },
        "SkillIcon": { "TagName": "None" },
        "isRemovable": false,
        "equipCost": 0,
        "filters": []
    });
    dup_rows.insert("RowA".to_owned(), sample_row.clone());
    dup_rows.insert("RowB".to_owned(), sample_row);
    let dup_table = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_CommandSkillData".to_owned(),
        package_path: "/Game/Datatable/Skill/DT_CommandSkillData".to_owned(),
        row_struct: "COMMAND_SKILL_DATA".to_owned(),
        rows: dup_rows,
    };
    assert!(matches!(
        parse_command_skills(&dup_table, "dup.json"),
        Err(DataError::InvalidSchema { .. })
    ));
}

// ---- 2 & 3. Main quests & current main scenario by save progress ----

#[test]
fn main_quest_and_current_main_scenario_queries() {
    assert_eq!(main_quest_id_for_chapter(0), None);
    assert_eq!(main_quest_id_for_chapter(1), Some(1001));
    assert_eq!(main_quest_id_for_chapter(9), Some(1009));
    assert_eq!(chapter_no_from_main_quest_id(1000), None);
    assert_eq!(chapter_no_from_main_quest_id(1001), Some(1));

    // Use `tests/fixtures/rust/data/export` (3 chapters, 3 main scenario phases) plus `ST_UI_QuestMenu`.
    let tmp = TempDir::new("scenario");
    copy_dir_recursive(
        &repo_root().join("tests/fixtures/rust/data/export"),
        tmp.path(),
    );
    copy_dir_recursive(
        &repo_root().join("tests/fixtures/rust/gaps/export/FFRS/Content/Datatable/Strings/UI"),
        &tmp.path().join("FFRS/Content/Datatable/Strings/UI"),
    );
    // Merge ST_UI_QuestMenu into Localization/{zh-Hans,en,ja}.json.
    for lang in ["zh-Hans", "en", "ja"] {
        let base_path = tmp.path().join(format!("Localization/{lang}.json"));
        let extra_path = repo_root().join(format!(
            "tests/fixtures/rust/gaps/export/Localization/{lang}.json"
        ));
        let mut base: Value =
            serde_json::from_str(&fs::read_to_string(&base_path).expect("read base loc"))
                .expect("parse base loc");
        let extra: Value =
            serde_json::from_str(&fs::read_to_string(&extra_path).expect("read extra loc"))
                .expect("parse extra loc");
        base.as_object_mut().expect("base obj").insert(
            "ST_UI_QuestMenu".to_owned(),
            extra["ST_UI_QuestMenu"].clone(),
        );
        fs::write(base_path, serde_json::to_vec_pretty(&base).expect("encode"))
            .expect("write merged loc");
    }

    let store = DataStore::new(tmp.path());
    let scenario = store.scenario().expect("load scenario");

    // Main quests by list, chapter, and ID.
    let all_main = scenario.main_quests();
    assert_eq!(all_main.len(), 3);
    assert_eq!(all_main[0].quest_id, 1001);
    assert_eq!(all_main[0].chapter_no, 1);
    assert_eq!(all_main[0].phases.len(), 2);

    let mq1 = scenario.main_quest_by_id(1001).expect("mq 1001");
    assert_eq!(mq1, scenario.main_quest_by_chapter(1).expect("mq ch1"));
    assert!(scenario.main_quest_by_id(999).is_none());
    assert!(scenario.main_quest_by_id(1099).is_none());

    assert_eq!(
        scenario.main_quest_title(1001).text.as_deref(),
        Some("第1章起始之森")
    );
    assert_eq!(
        scenario.main_quest_title_in(1001, "en").text.as_deref(),
        Some("Chapter 1 - Forest of Beginnings")
    );
    // Chapter 3 has no zh-Hans translation -> strict missing, never fakes ja.
    assert_eq!(scenario.main_quest_title(1003).text, None);

    let resolved_mq1 = scenario.resolve_main_quest(1001).expect("resolve mq 1001");
    assert_eq!(resolved_mq1.title.text.as_deref(), Some("第1章起始之森"));
    assert_eq!(resolved_mq1.objectives.len(), 2);
    assert_eq!(
        resolved_mq1.objectives[0].text.as_deref(),
        Some("与村口向导交谈")
    );
    // Step_1050 intentionally lacks a zh-Hans entry in `tests/fixtures/rust/data/export` -> strict None.
    assert_eq!(resolved_mq1.objectives[1].text, None);
    let resolved_mq1_ja = scenario
        .resolve_main_quest_in(1001, "ja")
        .expect("resolve mq 1001 in ja");
    assert_eq!(
        resolved_mq1_ja.objectives[1].text.as_deref(),
        Some("森の奥の遺跡を調べる")
    );

    // Current main scenario resolution across title, prologue, Ch 1 phases, Ch 2, and unmapped Ch 3.
    assert!(scenario.current_main_scenario(0).is_none());
    assert!(scenario.current_main_scenario(999).is_none());

    // In `tests/fixtures/rust/data/export`, Ch 1 starts at 1010 -> 1005 is prologue.
    let prologue = scenario
        .current_main_scenario(1005)
        .expect("prologue state");
    assert!(prologue.is_prologue);
    assert_eq!(prologue.chapter_no, Some(0));
    assert_eq!(prologue.main_quest_id, None);
    assert!(prologue.entry.is_none());
    assert_eq!(prologue.progress_start, Some(1000));
    assert_eq!(prologue.next_progress, Some(1010));

    let ch1_a = scenario.current_main_scenario(1015).expect("ch1 phase 1");
    assert!(!ch1_a.is_prologue);
    assert_eq!(ch1_a.chapter_no, Some(1));
    assert_eq!(ch1_a.main_quest_id, Some(1001));
    assert_eq!(ch1_a.progress_start, Some(1010));
    assert_eq!(ch1_a.next_progress, Some(1050));
    assert_eq!(ch1_a.destination_map_ids, vec![2001]);
    assert_eq!(
        ch1_a.entry.as_ref().map(|e| e.row_key.as_str()),
        Some("Step_1010")
    );

    let ch1_b = scenario.current_main_scenario(1050).expect("ch1 phase 2");
    assert_eq!(ch1_b.progress_start, Some(1050));
    assert_eq!(ch1_b.next_progress, Some(2000));
    assert_eq!(ch1_b.destination_map_ids, vec![2001]);
    assert_eq!(
        ch1_b.entry.as_ref().map(|e| e.row_key.as_str()),
        Some("Step_1050")
    );

    // `main_quest_objective_at_progress` validates chapter match.
    let obj_at_1015 = scenario
        .main_quest_objective_at_progress(1001, 1015)
        .expect("objective at 1015");
    assert_eq!(obj_at_1015.text.text.as_deref(), Some("与村口向导交谈"));
    assert!(
        scenario
            .main_quest_objective_at_progress(1002, 1015)
            .is_none()
    );

    // Chapter 3 exists in DT_ChapterData but has no DT_MainScenario rows (like post-game Ch 9).
    let ch3 = scenario.current_main_scenario(3000).expect("ch3 state");
    assert_eq!(ch3.chapter_no, Some(3));
    assert_eq!(ch3.main_quest_id, Some(1003));
    assert!(ch3.entry.is_none());
    assert!(ch3.destination_map_ids.is_empty());
}

// ---- 4. MapUnitPlayableData ----

#[test]
fn map_unit_playable_resolves_map_unit_ids_to_characters() {
    let tmp = TempDir::new("playable");
    build_combined_synthetic_export(tmp.path());

    // Inject a second synthetic playable unit (`PrologueMage`, ID 9006) matching `MapUnitId = 60`.
    let unit_path = tmp
        .path()
        .join("FFRS/Content/Datatable/Unit/DT_UnitParameter.json");
    let mut unit_doc: Value =
        serde_json::from_str(&fs::read_to_string(&unit_path).expect("read unit table"))
            .expect("parse unit table");
    let mut mage_row = unit_doc[0]["Rows"]["SynthHero"].clone();
    mage_row["ID"] = json!(9006);
    mage_row["Name"]["Key"] = json!("UnitName9006");
    unit_doc[0]["Rows"]
        .as_object_mut()
        .expect("Rows")
        .insert("PrologueMage".to_owned(), mage_row);
    fs::write(
        &unit_path,
        serde_json::to_vec_pretty(&unit_doc).expect("encode unit table"),
    )
    .expect("write unit table");

    let loc_path = tmp.path().join("Localization/zh-Hans.json");
    let mut loc: Value =
        serde_json::from_str(&fs::read_to_string(&loc_path).expect("read loc")).expect("parse loc");
    loc["ST_UnitName"]
        .as_object_mut()
        .expect("ST_UnitName")
        .insert("UnitName9006".to_owned(), json!("序章法师"));
    fs::write(&loc_path, serde_json::to_vec_pretty(&loc).expect("encode")).expect("write loc");

    let store = DataStore::new(tmp.path());
    let playable = store.map_unit_playable().expect("load map_unit_playable");
    assert_eq!(playable.entries().len(), 3);
    assert_eq!(playable.entries_for_map_unit_id(10).len(), 2);

    let ch1_hero = playable.resolve_by_map_unit_id(10).expect("map_unit_id 10");
    assert_eq!(ch1_hero.unit_id, 9001);
    assert_eq!(ch1_hero.row_key, "SynthHero");
    assert_eq!(ch1_hero.is_idle_by_four_direction, Some(false));
    assert_eq!(ch1_hero.enable_conditions.len(), 2);
    assert_eq!(
        playable.character_name(10).text.as_deref(),
        Some("合成英雄")
    );

    let prologue_hero = playable.resolve_by_map_unit_id(60).expect("map_unit_id 60");
    assert_eq!(prologue_hero.unit_id, 9006);
    assert_eq!(prologue_hero.row_key, "PrologueMage");
    assert_eq!(prologue_hero.is_idle_by_four_direction, Some(true));
    assert_eq!(
        playable.character_name(60).text.as_deref(),
        Some("序章法师")
    );

    // Non-playable NPC in DT_MapUnitAsset is ignored.
    assert!(playable.resolve_by_map_unit_id(901).is_none());
    assert!(playable.character_name(999).text.is_none());

    // Wrong RowStruct fails cleanly.
    let bad = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_MapUnitPlayableData".to_owned(),
        package_path: "/Game/Datatable/Map/DT_MapUnitPlayableData".to_owned(),
        row_struct: "WRONG".to_owned(),
        rows: BTreeMap::new(),
    };
    assert!(matches!(
        parse_map_unit_playable(&bad, "bad.json"),
        Err(DataError::UnexpectedRowStruct { .. })
    ));
}

// ---- 5. FlagConditionData ----

#[test]
fn flag_conditions_parse_index_and_evaluate() {
    assert_eq!(
        parse_flag_condition_expr("9"),
        Some(vec![FlagTerm {
            flag_id: 9,
            expected: true
        }])
    );
    assert_eq!(
        parse_flag_condition_expr("!100&!102"),
        Some(vec![
            FlagTerm {
                flag_id: 100,
                expected: false
            },
            FlagTerm {
                flag_id: 102,
                expected: false
            },
        ])
    );
    assert_eq!(parse_flag_condition_expr(""), None);
    assert_eq!(parse_flag_condition_expr("100&&102"), None);
    assert_eq!(parse_flag_condition_expr("abc"), None);

    let tmp = TempDir::new("flags");
    build_combined_synthetic_export(tmp.path());
    let store = DataStore::new(tmp.path());
    let flags = store.flag_conditions().expect("load flag conditions");
    assert_eq!(flags.entries().len(), 5);

    let chocobo = flags.by_name("PlayerRideChocobo").expect("chocobo entry");
    assert_eq!(chocobo.condition, "9");
    assert_eq!(chocobo.flag_ids(), vec![9]);
    assert!(chocobo.evaluate(|id| id == 9));
    assert!(!chocobo.evaluate(|_| false));

    let for_100 = flags.conditions_for_flag(100);
    let names_100: Vec<&str> = for_100.iter().map(|e| e.row_key.as_str()).collect();
    assert_eq!(names_100, vec!["GateHalfOpen", "GateSealed"]);

    assert_eq!(flags.evaluate("GateSealed", |_| false), Some(true));
    assert_eq!(flags.evaluate("GateSealed", |id| id == 100), Some(false));
    assert_eq!(flags.evaluate("GateHalfOpen", |id| id == 100), Some(true));
    assert_eq!(flags.evaluate("UnknownKey", |_| true), None);

    // Invalid expression fails with InvalidSchema.
    let mut bad_rows = BTreeMap::new();
    bad_rows.insert("Broken".to_owned(), json!({ "Condition": "not_a_flag" }));
    let bad_table = DataTable {
        table_type: DataTableType::CompositeDataTable,
        name: "CDT_FlagCondition".to_owned(),
        package_path: "/Game/Datatable/Flag/CDT_FlagCondition".to_owned(),
        row_struct: "FlagCondition".to_owned(),
        rows: bad_rows,
    };
    assert!(matches!(
        parse_flag_conditions(&bad_table, "bad.json"),
        Err(DataError::InvalidSchema { .. })
    ));
}

// ---- 6. EnemyChapterIndex ----

#[test]
fn enemy_chapter_index_derives_from_encounters_events_and_dungeon_island_override() {
    let tmp = TempDir::new("chapters");
    build_combined_synthetic_export(tmp.path());

    // Add a second dungeon under `01Fake` (`01Fake_41`) whose `CDT_MapData_Prod.island` is
    // `eMapIslandType::Pharm` (Ch 7), verifying dungeon-specific island overrides region prefix!
    let map_path = tmp
        .path()
        .join("FFRS/Content/Datatable/Map/CDT_MapData_Prod.json");
    let mut map_doc: Value =
        serde_json::from_str(&fs::read_to_string(&map_path).expect("read maps"))
            .expect("parse maps");
    let mut late_dng = map_doc[0]["Rows"]["FakeDungeon01"].clone();
    late_dng["ID"] = json!(3010);
    late_dng["island"] = json!("eMapIslandType::Pharm");
    late_dng["Level"]["AssetPathName"] =
        json!("/Game/Map/Dng/01Fake/Dng_01Fake_41_PL.Dng_01Fake_41_PL");
    map_doc[0]["Rows"]
        .as_object_mut()
        .expect("Rows")
        .insert("01Fake_41".to_owned(), late_dng);
    fs::write(
        &map_path,
        serde_json::to_vec_pretty(&map_doc).expect("encode maps"),
    )
    .expect("write maps");

    let rel_path = tmp
        .path()
        .join("FFRS/Content/Datatable/Map/DT_MapReleaseConditionData.json");
    let mut rel_doc: Value =
        serde_json::from_str(&fs::read_to_string(&rel_path).expect("read rel")).expect("parse rel");
    rel_doc[0]["Rows"].as_object_mut().expect("Rows").insert(
        "Rel_Pharm".to_owned(),
        json!({
            "Condition": "{progress:0}>=8000",
            "IslandType": "eMapIslandType::Pharm"
        }),
    );
    fs::write(
        &rel_path,
        serde_json::to_vec_pretty(&rel_doc).expect("encode rel"),
    )
    .expect("write rel");

    // Add a Ch 7 BattleGroup containing 9202 (`FrostWingBeta`) and an encounter pool for `01Fake_41`.
    let bg_path = tmp
        .path()
        .join("FFRS/Content/Datatable/Encount/DT_BattleGroup.json");
    let mut bg_doc: Value =
        serde_json::from_str(&fs::read_to_string(&bg_path).expect("read bg")).expect("parse bg");
    let mut bg_late = bg_doc[0]["Rows"]["GroupNormal"].clone();
    bg_late["ID"] = json!(9899);
    bg_late["UnitIdList"] = json!([9202]);
    bg_doc[0]["Rows"]
        .as_object_mut()
        .expect("Rows")
        .insert("GroupLateDungeon".to_owned(), bg_late);
    fs::write(
        &bg_path,
        serde_json::to_vec_pretty(&bg_doc).expect("encode bg"),
    )
    .expect("write bg");

    let late_encount = tmp
        .path()
        .join("FFRS/Content/Datatable/Encount/01Fake/DT_01Fake_41_01_Encount.json");
    fs::write(
        &late_encount,
        serde_json::to_vec_pretty(&json!([{
            "Type": "DataTable",
            "Name": "DT_01Fake_41_01_Encount",
            "Package": "/Game/Datatable/Encount/01Fake/DT_01Fake_41_01_Encount",
            "Properties": { "RowStruct": { "ObjectName": "Class'ENCOUNT_GROUP_DATA'" } },
            "Rows": {
                "SlotLate": { "battleGroupId": 9899, "encountRatio": 100 }
            }
        }]))
        .expect("encode late encount"),
    )
    .expect("write late encount");

    let store = DataStore::new(tmp.path());
    let battle = store.battle().expect("load battle");
    let index = store.enemy_chapters().expect("build enemy chapters");

    // 9101 appears in Ch 1 via encounter pool (BG 9801) AND story event (BG 9802).
    assert_eq!(index.chapters_for_enemy(9101), &[1]);
    let src_9101 = index.sources_for_enemy(9101);
    assert!(
        src_9101
            .iter()
            .any(|s| s.kind == EnemyChapterSourceKind::EncounterPool && s.battle_group_id == 9801)
    );
    assert!(
        src_9101
            .iter()
            .any(|s| s.kind == EnemyChapterSourceKind::MainStoryEvent && s.battle_group_id == 9802)
    );

    // 9201 appears in Ch 1 via main story event (BG 9802).
    assert_eq!(index.chapters_for_enemy(9201), &[1]);

    // 9202 is in `01Fake_41` whose island is Pharm (Ch 7), NOT Grandshelt (Ch 1)!
    assert_eq!(index.chapters_for_enemy(9202), &[7]);

    assert_eq!(index.enemies_in_chapter(1), &[9101, 9201]);
    assert_eq!(index.enemies_in_chapter(7), &[9202]);
    assert!(index.enemies_in_chapter(2).is_empty());

    let ch1_profiles = index.enemy_profiles_in_chapter(&battle, 1);
    let ids: Vec<i64> = ch1_profiles.iter().map(|p| p.unit_id).collect();
    assert_eq!(ids, vec![9101, 9201]);
    assert_eq!(index.mapped_enemy_count(), 3);
    assert_eq!(index.chapters(), vec![1, 7]);
}

// ---- Real-data spot checks (skipped cleanly if `.deps/ffr/export` is absent) ----

#[test]
fn real_export_spot_checks_for_task_20_gaps() {
    let export_root = real_export_dir();
    let required = [
        datatable_dir(&export_root).join("Skill/DT_CommandSkillData.json"),
        datatable_dir(&export_root).join("Map/DT_MapUnitPlayableData.json"),
        datatable_dir(&export_root).join("Flag/CDT_FlagCondition.json"),
        datatable_dir(&export_root).join("Scenario/DT_MainScenario.json"),
        localization_file(&export_root, "zh-Hans"),
    ];
    if !required.iter().all(|path| path.is_file()) {
        eprintln!(
            "跳过 Task 20 真实导出抽查：未找到完整导出目录 {}",
            export_root.display()
        );
        return;
    }

    let store = DataStore::new(&export_root);

    // 1. DT_CommandSkillData
    let commands = CommandSkillData::from_store(&store).expect("load real command skills");
    assert_eq!(commands.commands.len(), 80);
    assert!(commands.command_by_id(-1).is_none());
    assert_eq!(commands.command_name(1).text.as_deref(), Some("招式"));
    assert_eq!(commands.command_name(6).text.as_deref(), Some("幻兽召唤"));
    assert_eq!(commands.command_name(7).text.as_deref(), Some("道具"));
    assert_eq!(commands.command_name(10).text.as_deref(), Some("极限技"));

    // 2 & 3. MainQuest & CurrentMainScenario (Chapter 1 phases)
    let scenario = store.scenario().expect("load real scenario");
    let main_quests = scenario.main_quests();
    assert_eq!(main_quests.len(), 9);
    assert_eq!(main_quests.first().map(|q| q.quest_id), Some(1001));
    assert_eq!(main_quests.last().map(|q| q.quest_id), Some(1009));

    let ch1_quest = scenario
        .resolve_main_quest(1001)
        .expect("resolve main quest 1001");
    assert_eq!(ch1_quest.chapter_no, 1);
    assert!(
        ch1_quest
            .title
            .text
            .as_deref()
            .is_some_and(|t| t.starts_with("第1章"))
    );
    assert_eq!(ch1_quest.objectives.len(), 19);

    let ch1_phases = scenario.chapter_main_scenario(1);
    assert_eq!(ch1_phases.len(), 19);
    assert_eq!(ch1_phases[0].min_progress, Some(1020));
    assert_eq!(ch1_phases[18].min_progress, Some(1260));

    let prologue = scenario
        .current_main_scenario(1005)
        .expect("prologue progress 1005");
    assert!(prologue.is_prologue);
    assert_eq!(prologue.chapter_no, Some(0));
    assert_eq!(prologue.next_progress, Some(1020));

    let ch1_start = scenario
        .current_main_scenario(1020)
        .expect("ch1 start progress 1020");
    assert!(!ch1_start.is_prologue);
    assert_eq!(ch1_start.chapter_no, Some(1));
    assert_eq!(ch1_start.main_quest_id, Some(1001));
    assert_eq!(ch1_start.progress_start, Some(1020));
    assert_eq!(ch1_start.next_progress, Some(1030));
    assert_eq!(ch1_start.destination_map_ids, vec![3001]);

    // 4. DT_MapUnitPlayableData: m_MapUnitId 60 & 10
    let playable = MapUnitPlayableData::from_store(&store).expect("load real map unit playable");
    let unit_60 = playable.resolve_by_map_unit_id(60).expect("m_MapUnitId 60");
    assert_eq!(unit_60.unit_id, 1006);
    assert_eq!(
        playable.character_name(60).text.as_deref(),
        Some("魔人菲娜")
    );
    let unit_10 = playable.resolve_by_map_unit_id(10).expect("m_MapUnitId 10");
    assert_eq!(unit_10.unit_id, 1001);
    assert_eq!(playable.character_name(10).text.as_deref(), Some("雷因"));

    // 5. CDT_FlagCondition
    let flags = FlagConditionData::from_store(&store).expect("load real flag conditions");
    assert_eq!(flags.entries().len(), 38);
    assert_eq!(
        flags
            .by_name("PlayerRideChocobo")
            .map(|e| e.condition.as_str()),
        Some("9")
    );
    assert_eq!(
        flags.by_name("ChocoboRide").map(|e| e.condition.as_str()),
        Some("10")
    );

    // 6. Enemy -> Chapter: 钢蝠 (unit_id = 2)
    let enemy_chapters = EnemyChapterIndex::from_store(&store).expect("build real enemy chapters");
    assert_eq!(enemy_chapters.chapters_for_enemy(2), &[1]);
    assert!(enemy_chapters.mapped_enemy_count() > 300);
}
