//! Integration, index-plan, and spot-check tests for the `11_links` cross-area linkage stage.
//!
//! Covers:
//! - Talk event chain (`talk_event`, `talk_event_next`, `talk_line`, `level_object_talk`):
//!   order preservation from JSON, `ArgList["id"]` section filtering, strict language lookup
//!   without Japanese fallback, sequence linkage, battle-triggered talk events, and dangling
//!   reference issue recording (including `/Game/Developers/` developer assets).
//! - Map units (`map_unit`, `npc`, `where_is_map_unit`): composite NPC profile with shop
//!   (single + VariousShop expansion), inn (including default inn `-1` fallback), talk events,
//!   strict `name_zh`, and re-validation of `level_object` map-unit IDs against `DT_MapUnitAsset`.
//! - Unified class resolution (`class_ref`, `class_info`): prefix stripping (`Class ...`,
//!   `BlueprintGeneratedClass ...`), case-insensitive Blueprint path matching, `_C` suffix
//!   handling, and full Blueprint -> Blueprint -> C++ -> Object inheritance chain.
//! - Partial-build graceful skipping when upstream areas are absent.
//! - Dangling-reference fixes in `levels` (same-map stale targetMapId + default point -1) and
//!   `battle` (0%-probability Unit ID entries in `skillRandData` + `DT_BtlHitEffectData` details).
//! - `assert_db_uses_index` verification on EVERY SQL statement in `query::links::sql`.

use std::path::{Path, PathBuf};

use ffr_db::query::links::sql;
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db, build_fixture_db_stages, fixture_area_dir,
};
use ffr_db::{GameDb, Stage, StageStatus};
use rusqlite::params;

const LINK_AND_RELATED_TABLES: &[&str] = &[
    "talk_event",
    "talk_event_next",
    "talk_line",
    "level_object_talk",
    "map_unit",
    "class_ref",
    "level_object",
    "text",
    "sequence",
    "map",
    "inn",
    "various_shop_member",
    "ue_class",
    "blueprint",
];

#[test]
fn links_fixture_end_to_end_talk_npc_class_and_issues() {
    let test_db = build_fixture_db("links").expect("build full links fixture db");
    let db = test_db.db();
    let report = test_db.report();

    let links_stage = report
        .stages
        .iter()
        .find(|s| s.stage == "links")
        .expect("links stage report");
    assert_eq!(links_stage.status, StageStatus::Ok);
    assert!(links_stage.rows > 0);

    // 1. Talk event, continuous + branch transitions, sequence link, and battle events.
    let ev1 = db
        .talk_event("EV_STORY_01")
        .expect("query EV_STORY_01")
        .expect("EV_STORY_01 exists");
    assert!(ev1.is_skip_confirm);
    assert_eq!(
        ev1.event_sequence_path.as_deref(),
        Some("/Game/Sequencer/Event/Mock/SEQ_Mock_01")
    );
    assert_eq!(ev1.continuous_events.len(), 1);
    assert_eq!(ev1.choice_branches.len(), 2);

    let nexts = db.talk_event_next("EV_STORY_01").expect("talk_event_next");
    assert_eq!(nexts.len(), 3);
    assert!(
        nexts
            .iter()
            .any(|n| n.kind == "continuous" && n.next_event == "EV_STORY_02")
    );
    assert!(
        nexts.iter().any(|n| n.kind == "choice"
            && n.choice_index == Some(0)
            && n.next_event == "EV_BRANCH_A")
    );
    assert!(nexts.iter().any(|n| {
        n.kind == "choice"
            && n.choice_index == Some(1)
            && n.next_event == "EV_MISSING_NEXT"
            && n.condition == "{flag:1}"
    }));

    let seq = db
        .sequence_of_talk_event("EV_STORY_01")
        .expect("sequence_of_talk_event")
        .expect("linked sequence exists");
    assert_eq!(seq.path, "/Game/Sequencer/Event/Mock/SEQ_Mock_01");
    assert!(
        db.sequence_of_talk_event("EV_STORY_02")
            .expect("dev sequence returns None")
            .is_none()
    );

    let battle_events = db
        .talk_events_with_battle()
        .expect("talk_events_with_battle");
    let battle_ids: Vec<&str> = battle_events.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(battle_ids, vec!["EV_STORY_02", "EV_BRANCH_A"]);

    // 2. Talk lines preserve original JSON key order (`Line_Z` -> `Line_A` -> `Line_M`) and
    //    never fall back to Japanese when zh-Hans is missing.
    let zh_lines = db
        .talk_lines("EV_STORY_01", "zh-Hans")
        .expect("talk_lines zh-Hans");
    assert_eq!(zh_lines.len(), 3);
    assert_eq!(zh_lines[0].row_key, "Line_Z");
    assert_eq!(zh_lines[0].talk_id, Some(0));
    assert_eq!(zh_lines[0].speaker, "テロップ");
    assert_eq!(zh_lines[0].text.as_deref(), Some("第一章开场旁白第一句。"));
    assert_eq!(zh_lines[1].row_key, "Line_A");
    assert_eq!(zh_lines[1].talk_id, Some(1));
    assert_eq!(zh_lines[1].text.as_deref(), Some("勇者准备出发。"));
    assert_eq!(zh_lines[2].row_key, "Line_M");
    assert_eq!(zh_lines[2].talk_id, Some(1));
    assert_eq!(
        zh_lines[2].text, None,
        "missing zh-Hans line must stay None, never fallback to ja"
    );

    let ja_lines = db.talk_lines("EV_STORY_01", "ja").expect("talk_lines ja");
    assert_eq!(ja_lines.len(), 3);
    assert_eq!(ja_lines[2].text.as_deref(), Some("未翻訳テスト台詞。"));

    // Shared free-talk table filtered by `ArgList["id"]`.
    let shop_lines = db
        .talk_lines("EV_NPC_SHOP", "zh-Hans")
        .expect("shop talk_lines");
    assert_eq!(shop_lines.len(), 1);
    assert_eq!(shop_lines[0].row_key, "Npc_Shop_Line");
    assert_eq!(shop_lines[0].text.as_deref(), Some("欢迎光临道具店。"));

    let inn_lines = db
        .talk_lines("EV_NPC_INN", "zh-Hans")
        .expect("inn talk_lines");
    assert_eq!(inn_lines.len(), 1);
    assert_eq!(inn_lines[0].row_key, "Npc_Inn_Line");
    assert_eq!(inn_lines[0].text.as_deref(), Some("欢迎光临旅店。"));

    // 3. Level object -> talk events (`EventTrigger.interactEventId` and `MapUnit.events`).
    let trig_events = db
        .talk_events_of_object(3001, 20)
        .expect("talk_events_of_object trigger");
    assert_eq!(trig_events.len(), 1);
    assert_eq!(trig_events[0].id, "EV_STORY_01");

    // 4. Composite `npc()` for single shop, inn (default -1 fallback), and VariousShop NPCs.
    let shop_npc = db.npc(3001, 3).expect("npc shop").expect("shop npc exists");
    assert_eq!(shop_npc.map_name_zh.as_deref(), Some("试作神殿入口"));
    assert_eq!(shop_npc.level_name, "MockTown_EV");
    assert_eq!(shop_npc.map_unit_id, Some(1060));
    assert_eq!(shop_npc.name.as_deref(), Some("テスト商人"));
    assert_eq!(
        shop_npc.name_zh, None,
        "untranslated NPC map_unit must not copy Japanese into name_zh"
    );
    assert_eq!(
        shop_npc
            .map_unit
            .as_ref()
            .and_then(|m| m.primary_asset.as_deref()),
        Some("/Game/Chara/Mock/npc1060")
    );
    assert_eq!(shop_npc.talk_events.len(), 1);
    assert_eq!(shop_npc.talk_events[0].id, "EV_NPC_SHOP");
    assert_eq!(shop_npc.shops.len(), 1);
    assert_eq!(shop_npc.shops[0].shop.id, 2);
    assert_eq!(shop_npc.shops[0].name_zh.as_deref(), Some("试作道具店"));
    assert_eq!(shop_npc.shops[0].stock.len(), 1);
    assert!(shop_npc.inn.is_none());

    let inn_npc = db.npc(3001, 2).expect("npc inn").expect("inn npc exists");
    assert_eq!(inn_npc.inn_id, Some(0));
    let inn = inn_npc.inn.expect("default inn -1 resolved");
    assert_eq!(inn.id, -1);
    assert_eq!(inn.price, 100);
    assert_eq!(inn_npc.talk_events.len(), 1);
    assert_eq!(inn_npc.talk_events[0].id, "EV_NPC_INN");

    let various_npc = db
        .npc(3001, 4)
        .expect("npc various shop")
        .expect("various shop npc exists");
    assert_eq!(various_npc.map_unit_id, Some(10));
    assert_eq!(various_npc.name_zh.as_deref(), Some("测试勇者"));
    assert_eq!(various_npc.various_shop_id, Some(100));
    let various_shop_ids: Vec<i64> = various_npc.shops.iter().map(|s| s.shop.id).collect();
    assert_eq!(various_shop_ids, vec![2, 20]);

    assert!(db.npc(3001, 99999).expect("missing npc").is_none());

    // 5. `where_is_map_unit()` & `search_map_units_by_name()`
    let placements_1060 = db.where_is_map_unit(1060).expect("where_is_map_unit 1060");
    assert_eq!(placements_1060.len(), 2);
    assert_eq!(placements_1060[0].map_id, Some(3001));
    assert_eq!(
        placements_1060[0].map_name_zh.as_deref(),
        Some("试作神殿入口")
    );

    let by_zh = db
        .search_map_units_by_name("测试勇者")
        .expect("search_map_units_by_name zh");
    assert_eq!(by_zh.len(), 1);
    assert_eq!(by_zh[0].id, 10);

    let by_row_key = db
        .search_map_units_by_name("テスト商人")
        .expect("search_map_units_by_name row_key");
    assert_eq!(by_row_key.len(), 1);
    assert_eq!(by_row_key[0].id, 1060);

    assert!(
        db.search_map_units_by_name("")
            .expect("empty query")
            .is_empty()
    );
    assert!(
        db.search_map_units_by_name("不存在的NPC")
            .expect("missing query")
            .is_empty()
    );

    // 6. `class_info()` across raw snapshot prefixes, case-mismatched paths, short names, and
    //    multi-hop Blueprint -> Blueprint -> C++ -> Actor -> Object chain.
    let shop_cls = db
        .class_info("BlueprintGeneratedClass /Game/BP/Mock/BP_MockShopNpc.BP_MockShopNpc_C")
        .expect("class_info BP_MockShopNpc")
        .expect("BP_MockShopNpc resolved");
    assert_eq!(
        shop_cls.blueprint_id.as_deref(),
        Some("/Game/BP/Mock/BP_MockShopNpc")
    );
    assert!(shop_cls.blueprint.is_some());
    let chain_shorts: Vec<&str> = shop_cls
        .parent_chain
        .iter()
        .map(|n| n.short_name.as_str())
        .collect();
    assert_eq!(
        chain_shorts,
        vec![
            "BP_MockShopNpc_C",
            "BP_MockNpc_C",
            "CPP_MockUnit",
            "Actor",
            "Object",
        ]
    );

    // Same class queried by short name without `_C`.
    let shop_short = db
        .class_info("BP_MockShopNpc")
        .expect("class_info short")
        .expect("found by short name");
    assert_eq!(shop_short.normalized, shop_cls.normalized);

    // Case-insensitive `/Game/BP/mock/BP_MockNpc.BP_MockNpc_C` links to BOTH `ue_class` AND `blueprint`.
    let npc_cls = db
        .class_info("/Game/BP/mock/BP_MockNpc.BP_MockNpc_C")
        .expect("class_info case-insensitive")
        .expect("BP_MockNpc resolved");
    assert_eq!(
        npc_cls.ue_class_id.as_deref(),
        Some("/Game/BP/Mock/BP_MockNpc.BP_MockNpc_C")
    );
    assert_eq!(
        npc_cls.blueprint_id.as_deref(),
        Some("/Game/BP/Mock/BP_MockNpc")
    );
    assert!(npc_cls.ue_class.is_some());
    assert!(npc_cls.blueprint.is_some());

    // Runtime `"Class /Script/mucha.CPP_MockUnit"` prefix stripped and matched to `ue_class`.
    let cpp_cls = db
        .class_info("Class /Script/mucha.CPP_MockUnit")
        .expect("class_info CPP_MockUnit")
        .expect("CPP_MockUnit resolved");
    assert_eq!(
        cpp_cls.ue_class_id.as_deref(),
        Some("/Script/mucha.CPP_MockUnit")
    );
    assert!(cpp_cls.ue_class.is_some());
    assert_eq!(
        cpp_cls
            .parent_chain
            .iter()
            .map(|n| n.short_name.as_str())
            .collect::<Vec<_>>(),
        vec!["CPP_MockUnit", "Actor", "Object"]
    );

    assert!(
        db.class_info("NoSuchClassXYZ")
            .expect("missing class")
            .is_none()
    );

    // 7. Verify all expected `build_issue`s across links, levels, and battle.
    let issues = db.build_issues().expect("read build_issues");

    // Links issues:
    assert!(issues.iter().any(|i| {
        i.stage == "links"
            && i.kind == "dangling_talk_sequence"
            && i.subject == "EV_STORY_02"
            && i.detail.contains("开发者测试过场")
    }));
    assert!(issues.iter().any(|i| {
        i.stage == "links"
            && i.kind == "dangling_talk_table"
            && i.subject == "EV_STORY_02"
            && i.detail.contains("开发者测试对话表")
    }));
    assert!(issues.iter().any(|i| {
        i.stage == "links" && i.kind == "dangling_talk_battle_group" && i.subject == "EV_BRANCH_A"
    }));
    assert!(issues.iter().any(|i| {
        i.stage == "links"
            && i.kind == "dangling_talk_next"
            && i.subject == "EV_STORY_01"
            && i.detail.contains("EV_MISSING_NEXT")
    }));
    assert!(issues.iter().any(|i| {
        i.stage == "links" && i.kind == "dangling_talk_event" && i.detail.contains("EV_NONEXISTENT")
    }));
    assert!(issues.iter().any(|i| {
        i.stage == "links" && i.kind == "dangling_map_unit" && i.detail.contains("9999")
    }));
    assert!(issues.iter().any(|i| {
        i.stage == "links"
            && i.kind == "unresolved_class_ref"
            && i.subject == "/Game/Map/MockTown_EV.MockTown_EV_C"
            && i.detail.contains("关卡蓝图")
    }));

    // Levels fixes:
    // - No `dangling_playable_unit` (old bug removed).
    // - Same-map stale `targetMapId=31002` resolved to (3001, 0) without `dangling_edge_target_map`
    //   or `unresolved_transition_point`.
    // - `isEventOnly=true, point=-1` and default `point=-1` (resolved to 0) produce 0 unresolved points.
    assert!(
        !issues.iter().any(|i| i.kind == "dangling_playable_unit"),
        "dangling_playable_unit must not be emitted: {issues:?}"
    );
    assert!(
        !issues.iter().any(|i| i.kind == "dangling_edge_target_map"),
        "same-map stale mapId must resolve to from_map: {issues:?}"
    );
    assert!(
        !issues
            .iter()
            .any(|i| i.kind == "unresolved_transition_point"),
        "same-map, event-only -1, and default -1->0 must resolve cleanly: {issues:?}"
    );
    let exits = db.map_exits(3001).expect("map_exits 3001");
    let same_exit = exits
        .iter()
        .find(|e| e.actor_name == "BP_SameMap_StaleId")
        .expect("same-map exit exists");
    assert_eq!(same_exit.target_map_id, Some(3001));
    assert!(same_exit.target_location.is_some());
    let default_exit = exits
        .iter()
        .find(|e| e.actor_name == "BP_DefaultPoint_Neg1")
        .expect("default -1 exit exists");
    assert_eq!(default_exit.target_point_id, Some(0));
    assert!(default_exit.target_location.is_some());

    // Battle fixes:
    // - Unit 462's 0%-prob `skillRandData1` entry with Unit ID 462 is skipped (no dangling_skill_ref).
    // - `silenceFightId=401100` remains `dangling_skill_ref` with DT_BtlHitEffectData detail.
    let battle_skill_issues: Vec<_> = issues
        .iter()
        .filter(|i| i.stage == "battle" && i.kind == "dangling_skill_ref")
        .collect();
    assert_eq!(
        battle_skill_issues.len(),
        1,
        "issues={battle_skill_issues:?}"
    );
    assert!(
        battle_skill_issues[0].subject.contains("401100")
            && battle_skill_issues[0]
                .detail
                .contains("DT_BtlHitEffectData")
            && battle_skill_issues[0].detail.contains("テスト_仮通常攻撃"),
        "expected DT_BtlHitEffectData detail, got: {:?}",
        battle_skill_issues[0]
    );
}

#[test]
fn links_stage_skips_missing_upstream_areas_gracefully() {
    // Only reflection + links (no datatable, no levels, no sequences, no blueprints).
    let test_db = build_fixture_db_stages("links", Some(vec![Stage::Reflection, Stage::Links]))
        .expect("build reflection + links");
    let links_rep = test_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "links")
        .expect("links report");
    assert_eq!(links_rep.status, StageStatus::Skipped);
    assert!(links_rep.note.contains("跳过"), "note={}", links_rep.note);

    // Datatable + links only (no sequences, no levels, no world, no battle, no reflection).
    let test_db2 = build_fixture_db_stages("links", Some(vec![Stage::Datatable, Stage::Links]))
        .expect("build datatable + links");
    let links_rep2 = test_db2
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "links")
        .expect("links report");
    assert_eq!(links_rep2.status, StageStatus::Ok);
    assert!(
        links_rep2.note.contains("跳过"),
        "partial links build must record skipped sub-passes: {}",
        links_rep2.note
    );
    assert!(
        links_rep2.note.contains("sequence"),
        "note={}",
        links_rep2.note
    );
    assert!(
        links_rep2.note.contains("level_object"),
        "note={}",
        links_rep2.note
    );
}

#[test]
fn all_links_queries_use_indexes_without_full_table_scans() {
    let test_db = build_fixture_db("links").expect("build links fixture db");
    let db = test_db.db();

    assert_db_uses_index(
        db,
        sql::TALK_EVENT_BY_ID,
        ["EV_STORY_01"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::TALK_EVENT_NEXT_BY_EVENT,
        ["EV_STORY_01"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::TALK_LINES_BY_TABLE,
        params!["/Game/Datatable/Event/Text/DT_MockStory_01", "zh-Hans"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::TALK_LINES_BY_TABLE_AND_TALK_ID,
        params!["/Game/Datatable/Event/Text/DT_MockNpc", 1_i64, "zh-Hans"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::TALK_EVENTS_OF_OBJECT,
        params![3001_i64, 3_i64],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::SEQUENCE_OF_TALK_EVENT,
        ["EV_STORY_01"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::TALK_EVENTS_WITH_BATTLE,
        [],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(db, sql::MAP_UNIT_BY_ID, [1060_i64], LINK_AND_RELATED_TABLES);
    assert_db_uses_index(
        db,
        sql::MAP_UNITS_BY_NAME_ZH,
        ["测试勇者"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::MAP_UNITS_BY_ROW_KEY,
        ["テスト商人"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::LEVEL_OBJECTS_FOR_NPC,
        params![3001_i64, 3_i64],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::MAP_NAME_ZH_BY_ID,
        [3001_i64],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(db, sql::INN_BY_ID, [-1_i64], LINK_AND_RELATED_TABLES);
    assert_db_uses_index(
        db,
        sql::VARIOUS_SHOP_MEMBER_IDS,
        [100_i64],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::WHERE_IS_MAP_UNIT,
        ["1060"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::CLASS_REF_BY_RAW,
        ["/Game/BP/Mock/BP_MockShopNpc.BP_MockShopNpc_C"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::CLASS_REF_BY_NORMALIZED,
        ["/Game/BP/Mock/BP_MockShopNpc.BP_MockShopNpc_C"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::UE_CLASS_BY_PATH,
        ["/Script/mucha.CPP_MockUnit"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::UE_CLASS_BY_SHORT_NAME,
        ["CPP_MockUnit"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::BLUEPRINT_BY_PATH,
        ["/Game/BP/Mock/BP_MockShopNpc"],
        LINK_AND_RELATED_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::BLUEPRINT_BY_CLASS_NAME,
        ["BP_MockShopNpc_C"],
        LINK_AND_RELATED_TABLES,
    );
}

#[test]
fn real_db_links_spot_checks_when_built() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let db_path: PathBuf = repo_root.join(".deps/ffr/db/ffr.sqlite");
    if !db_path.is_file() {
        return;
    }
    let Ok(db) = GameDb::open(&db_path) else {
        return;
    };
    let Ok(reports) = db.stage_reports() else {
        return;
    };
    let links_ok = reports
        .iter()
        .any(|r| r.stage == "links" && r.status == StageStatus::Ok);
    if !links_ok {
        return;
    }

    // Spot-check 1: map 3001 shop NPC (unique_id = 3).
    let shop_npc = db
        .npc(3001, 3)
        .expect("real npc(3001, 3)")
        .expect("map 3001 shop npc must exist");
    assert_eq!(shop_npc.map_unit_id, Some(1060));
    assert!(!shop_npc.shops.is_empty());
    assert_eq!(shop_npc.shops[0].shop.id, 2);
    assert!(!shop_npc.talk_events.is_empty());

    // Spot-check 2: Chapter 1 first story event (`C01_001_01`).
    let lines = db
        .talk_lines("C01_001_01", "zh-Hans")
        .expect("real talk_lines C01_001_01");
    assert!(lines.len() >= 3);
    assert!(lines[..3].iter().all(|l| l.text.is_some()));

    // Spot-check 3: `class_info("BP_MapUnit_Player_C")`.
    let player_cls = db
        .class_info("BP_MapUnit_Player_C")
        .expect("real class_info BP_MapUnit_Player_C")
        .expect("BP_MapUnit_Player_C must exist");
    assert!(player_cls.blueprint.is_some() || player_cls.ue_class.is_some());
    assert!(player_cls.parent_chain.len() >= 2);

    // Silence unused helper import if only used for path checks elsewhere.
    let _ = fixture_area_dir("links");
}
