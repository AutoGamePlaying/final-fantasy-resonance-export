//! Integration, index-plan, and alignment tests for the `04_world` database area.

use std::path::{Path, PathBuf};

use ffr_data::world::{
    Item, MapInfo, Quest, Shop, WorldData, WorldDataParts, item_summary, main_objective_summary,
    map_summary, quest_summary, shop_summary,
};
use ffr_data::{
    DataStore, FFR_DEPS_ENV, FlagConditionData, MapUnitPlayableData, datatable_dir,
    export_dir_from_deps,
};
use ffr_db::query::world::{ItemDropKind, ItemSource};
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db_stages, build_temp_db, fixture_area_dir,
};
use ffr_db::{BuildInputs, GameDb, Stage, StageStatus};
use rusqlite::params;

fn real_export_dir() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return export_dir_from_deps(PathBuf::from(deps));
    }
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ffr-db sits two levels under repository root");
    export_dir_from_deps(repo_root.join(".deps"))
}

fn rebuild_world_data_from_db(db: &GameDb, reference: &WorldData) -> WorldData {
    let maps: Vec<MapInfo> = reference
        .maps()
        .iter()
        .map(|m| db.map(m.id).expect("db.map").expect("map exists in db"))
        .collect();
    let islands = reference
        .islands()
        .iter()
        .map(|i| {
            db.island(&i.island_type)
                .expect("db.island")
                .expect("island exists in db")
        })
        .collect();
    let regions = reference
        .regions()
        .iter()
        .map(|r| {
            db.region(r.id)
                .expect("db.region")
                .expect("region exists in db")
        })
        .collect();
    let release_conditions = reference
        .release_conditions()
        .iter()
        .map(|rc| {
            db.map_release_condition(&rc.island_type)
                .expect("db.map_release_condition")
                .expect("release_condition exists in db")
        })
        .collect();
    let save_points = reference
        .save_points()
        .iter()
        .map(|sp| {
            db.save_point(&sp.map_type)
                .expect("db.save_point")
                .expect("save_point exists in db")
        })
        .collect();
    let save_point_commands = reference
        .save_point_commands()
        .iter()
        .map(|cmd| {
            db.save_point_command(&cmd.command_type)
                .expect("db.save_point_command")
                .expect("save_point_command exists in db")
        })
        .collect();
    let teleport_points = reference
        .teleport_points()
        .iter()
        .map(|tp| {
            db.teleport_point(tp.index)
                .expect("db.teleport_point")
                .expect("teleport_point exists in db")
        })
        .collect();
    let treasure_boxes = reference
        .treasure_boxes()
        .iter()
        .map(|tb| {
            db.treasure_box(tb.map_id, tb.id)
                .expect("db.treasure_box")
                .expect("treasure_box exists in db")
        })
        .collect();
    let items: Vec<Item> = reference
        .items()
        .iter()
        .map(|it| db.item(it.id).expect("db.item").expect("item exists in db"))
        .collect();
    let quests: Vec<Quest> = reference
        .quests()
        .iter()
        .map(|q| {
            db.quest(q.id)
                .expect("db.quest")
                .expect("quest exists in db")
        })
        .collect();
    let quest_board = reference
        .quest_board()
        .iter()
        .filter(|e| !e.is_placeholder())
        .map(|e| {
            db.quest_board(e.quest_id)
                .expect("db.quest_board")
                .expect("quest_board exists in db")
        })
        .collect();
    let shops: Vec<Shop> = reference
        .shops()
        .iter()
        .map(|s| {
            db.shop(s.id)
                .expect("db.shop")
                .expect("shop exists in db")
                .shop
        })
        .collect();
    let various_shops = reference
        .various_shops()
        .iter()
        .map(|vs| {
            db.various_shop(vs.id)
                .expect("db.various_shop")
                .expect("various_shop exists in db")
        })
        .collect();
    let inns = reference
        .inns()
        .iter()
        .map(|inn| db.inn(inn.id).expect("db.inn").expect("inn exists in db"))
        .collect();
    let chapters = reference
        .chapters()
        .iter()
        .map(|ch| {
            db.chapter(ch.chapter_no)
                .expect("db.chapter")
                .expect("chapter exists in db")
        })
        .collect();

    let mut main_scenario = Vec::new();
    for ch in reference.chapters() {
        main_scenario.extend(
            db.main_scenario_by_chapter(ch.chapter_no)
                .expect("db.main_scenario_by_chapter"),
        );
    }

    WorldData::from_parts(WorldDataParts {
        maps,
        islands,
        regions,
        release_conditions,
        save_points,
        save_point_commands,
        teleport_points,
        treasure_boxes,
        items,
        quests,
        quest_board,
        shops,
        various_shops,
        inns,
        chapters,
        main_scenario,
        text_resolver: reference.text_resolver().clone(),
    })
}

#[test]
fn synthetic_world_queries_dangling_issues_and_alignment() {
    let test_db =
        build_fixture_db_stages("world", Some(vec![Stage::World])).expect("build world fixture db");
    let db = test_db.db();

    let world_stage = test_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "world")
        .expect("world stage report present");
    assert_eq!(world_stage.status, StageStatus::Ok);
    assert!(world_stage.rows > 0);

    // Load reference objects directly from the same fixture export via ffr-data.
    let export_dir = fixture_area_dir("world").join("export");
    let store = DataStore::new(&export_dir);
    let ref_world = WorldData::from_store(&store).expect("load reference WorldData");
    let ref_scenario = store.scenario().expect("load reference ScenarioData");
    let ref_flags =
        FlagConditionData::from_store(&store).expect("load reference FlagConditionData");
    let ref_playable =
        MapUnitPlayableData::from_store(&store).expect("load reference MapUnitPlayableData");

    // 1. Dangling references must be recorded in `build_issue` without aborting or dropping rows.
    let issues = db.build_issues().expect("read build_issues");
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "dangling_map" && i.subject == "teleport_point:2"),
        "expected dangling teleport landmark 9997 in issues: {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "dangling_map" && i.subject == "treasure_box:DanglingChest"),
        "expected dangling treasure box map 9999 in issues: {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "dangling_item" && i.subject == "treasure_box:DanglingChest"),
        "expected dangling treasure box item 9998 in issues: {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "dangling_item" && i.subject == "shop:2001"),
        "expected dangling shop item 9001 in issues: {issues:?}"
    );
    // Dangling chest row itself is NOT silently dropped.
    assert!(db.treasure_box(9999, 0).expect("dangling chest").is_some());

    // 2. Maps: ID, level_name (bare + UE path + sub-object), and multilingual name search.
    for expected_map in ref_world.maps() {
        assert_eq!(
            db.map(expected_map.id).unwrap().as_ref(),
            Some(expected_map)
        );
        assert_eq!(
            db.map_by_level(&expected_map.level_name).unwrap().as_ref(),
            Some(expected_map)
        );
        assert_eq!(
            db.map_by_level(&expected_map.level_asset_path)
                .unwrap()
                .as_ref(),
            Some(expected_map)
        );
    }
    assert_eq!(
        db.map_by_level("Com_00Fake_45_PL:PersistentLevel")
            .unwrap()
            .map(|m| m.id),
        Some(10040)
    );
    assert_eq!(db.map_by_level("Missing_PL").unwrap(), None);
    assert_eq!(db.map(999_999).unwrap(), None);

    // 3+ char FTS5 trigram and 1-2 char covering-index name queries across zh-Hans, en, ja.
    let zh_exact: Vec<i64> = db
        .maps_by_name("微光洞窟", "zh-Hans")
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(zh_exact, vec![3001]);

    let zh_short: Vec<i64> = db
        .maps_by_name("港镇", "zh-Hans")
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(zh_short, vec![2001]);

    let zh_dups: Vec<i64> = db
        .maps_by_name("试炼场", "zh-Hans")
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(zh_dups, vec![10040, 10041]);

    let en_hits: Vec<i64> = db
        .maps_by_name("cavern", "en")
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(en_hits, vec![3001]);

    let ja_hits: Vec<i64> = db
        .maps_by_name("テスト洞窟", "ja")
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ja_hits, vec![3001]);

    // Japanese string must NEVER match zh-Hans queries.
    assert!(db.maps_by_name("テスト洞窟", "zh-Hans").unwrap().is_empty());
    assert!(db.maps_by_name("   ", "zh-Hans").unwrap().is_empty());

    // 3. Map contents & links.
    let cavern_contents = db.map_contents(3001).unwrap().expect("map 3001 contents");
    assert_eq!(cavern_contents.map.id, 3001);
    assert_eq!(
        cavern_contents
            .island
            .as_ref()
            .map(|i| i.island_type.as_str()),
        Some("eMapIslandType::Grandshelt")
    );
    let sp = cavern_contents
        .save_point
        .as_ref()
        .expect("dungeon save point");
    assert!(sp.can_save && sp.can_use_tent && sp.can_teleport);
    assert_eq!(cavern_contents.teleport_points.len(), 1);
    assert_eq!(
        cavern_contents.teleport_points[0].airship_landing_point_id,
        Some(7002)
    );
    assert_eq!(cavern_contents.treasure_boxes.len(), 2);
    assert!(cavern_contents.inns.is_empty());
    assert!(cavern_contents.shop_facilities.is_empty());

    let harbor_contents = db.map_contents(2001).unwrap().expect("map 2001 contents");
    assert_eq!(harbor_contents.inns.len(), 1);
    assert_eq!(harbor_contents.inns[0].price, 100);
    assert_eq!(harbor_contents.shop_facilities.len(), 2);
    assert!(db.map_contents(999_999).unwrap().is_none());

    let links_from_field = db.map_links_from(1000).unwrap();
    assert!(
        links_from_field
            .iter()
            .any(|l| l.to_map == 2001 && l.kind == "teleport_landmark")
    );
    assert!(
        links_from_field
            .iter()
            .any(|l| l.to_map == 3001 && l.kind == "teleport_landmark")
    );
    let links_from_3001 = db.map_links_from(3001).unwrap();
    assert_eq!(links_from_3001.len(), 1);
    assert_eq!(links_from_3001[0].to_map, 1000);
    assert_eq!(links_from_3001[0].kind, "world_map_exit");

    let links_from_10041 = db.map_links_from(10041).unwrap();
    assert!(
        links_from_10041
            .iter()
            .any(|l| l.to_map == 10040 && l.kind == "same_map")
    );
    let links_to_10040 = db.map_links_to(10040).unwrap();
    assert!(
        links_to_10040
            .iter()
            .any(|l| l.from_map == 10041 && l.kind == "same_map")
    );

    // 4. Items & multilingual name search (strict no-JP fallback on untranslated seed 9500).
    for expected_item in ref_world.items() {
        assert_eq!(
            db.item(expected_item.id).unwrap().as_ref(),
            Some(expected_item)
        );
    }
    assert_eq!(
        db.items_by_name("玄铁大剑", "zh-Hans")
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        vec![10100]
    );
    assert_eq!(
        db.items_by_name("大剑", "zh-Hans")
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        vec![10100]
    );
    assert_eq!(
        db.items_by_name("herb", "en")
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    // Untranslated seed 9500 must NOT match zh-Hans, only ja.
    assert!(db.items_by_name("未翻訳", "zh-Hans").unwrap().is_empty());
    assert_eq!(
        db.items_by_name("未翻訳", "ja")
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        vec![9500]
    );

    // 5. item_sources across ALL 5 channels (TreasureBox, Shop, QuestPrize, QuestBoardReward, EnemyDrop).
    let sources_1 = db.item_sources(1).unwrap();
    assert!(sources_1.iter().any(|s| matches!(
        s,
        ItemSource::TreasureBox {
            map_id: 3001,
            box_id: 0,
            count: 2,
            is_trap: false,
            ..
        }
    )));
    assert!(sources_1.iter().any(|s| matches!(
        s,
        ItemSource::Shop {
            shop_id: 1,
            price: Some(50),
            ..
        }
    )));
    assert!(sources_1.iter().any(|s| matches!(
        s,
        ItemSource::QuestBoardReward {
            quest_id: 1001,
            count: Some(2),
            ..
        }
    )));
    assert!(sources_1.iter().any(|s| matches!(
        s,
        ItemSource::EnemyDrop {
            unit_id: 9101,
            drop_kind: ItemDropKind::DropNormal,
            rate: 15,
            ..
        }
    )));
    assert!(sources_1.iter().any(|s| matches!(
        s,
        ItemSource::EnemyDrop {
            unit_id: 9101,
            drop_kind: ItemDropKind::StealNormal,
            rate: 10,
            ..
        }
    )));
    assert!(sources_1.iter().any(|s| matches!(
        s,
        ItemSource::EnemyDrop {
            unit_id: 9201,
            drop_kind: ItemDropKind::StealRare,
            rate: 20,
            ..
        }
    )));

    let sources_10100 = db.item_sources(10100).unwrap();
    assert!(sources_10100.iter().any(|s| matches!(
        s,
        ItemSource::Shop {
            shop_id: 2001,
            price: Some(2000),
            max_order_num: Some(1),
            ..
        }
    )));
    assert!(sources_10100.iter().any(|s| matches!(
        s,
        ItemSource::QuestPrize {
            quest_id: 10,
            count: 1,
            ..
        }
    )));
    assert!(sources_10100.iter().any(|s| matches!(
        s,
        ItemSource::EnemyDrop {
            unit_id: 9201,
            drop_kind: ItemDropKind::DropNormal,
            rate: 100,
            ..
        }
    )));

    // 6. Shops, resolved stock, shops_selling, various_shops.
    for expected_shop in ref_world.shops() {
        let resolved = db.shop(expected_shop.id).unwrap().expect("shop exists");
        assert_eq!(&resolved.shop, expected_shop);
        assert_eq!(resolved.stock.len(), expected_shop.items.len());
    }
    let weapon_shop = db.shop(2001).unwrap().expect("weapon shop 2001");
    assert_eq!(weapon_shop.name_zh, None); // JP-only inline shop name -> NULL, never JP!
    assert_eq!(
        weapon_shop.stock[0].item.as_ref().map(|i| i.id),
        Some(10100)
    );
    assert_eq!(weapon_shop.stock[0].price, Some(2000));
    assert_eq!(weapon_shop.stock[1].entry.item_id, 9001);
    assert_eq!(weapon_shop.stock[1].item, None);
    assert_eq!(weapon_shop.stock[1].price, None);

    let selling_500 = db.shops_selling(500).unwrap();
    assert_eq!(
        selling_500.iter().map(|s| s.id).collect::<Vec<_>>(),
        ref_world
            .shops_selling_item(500)
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>()
    );
    let vs_members = db.various_shop_members(10001).unwrap();
    assert_eq!(vs_members.len(), 2);

    // 7. Quests, chapters, main_quest, main_scenario_for_progress, flags, playable units.
    for expected_quest in ref_world.quests() {
        assert_eq!(
            db.quest(expected_quest.id).unwrap().as_ref(),
            Some(expected_quest)
        );
    }
    assert!(db.quest_board(-1).unwrap().is_none());
    assert_eq!(
        db.quest_board(1001).unwrap().as_ref(),
        ref_world.quest_board_by_id(1001)
    );

    for expected_ch in ref_world.chapters() {
        assert_eq!(
            db.chapter(expected_ch.chapter_no).unwrap().as_ref(),
            Some(expected_ch)
        );
    }
    for expected_mq in ref_scenario.main_quests() {
        assert_eq!(
            db.main_quest(expected_mq.quest_id).unwrap().as_ref(),
            Some(&expected_mq)
        );
    }

    for progress in [0, 999, 1000, 1015, 1020, 1049, 1050, 1999, 2000, 2500, 3000] {
        assert_eq!(
            db.main_scenario_for_progress(progress).unwrap(),
            ref_scenario.current_main_scenario(progress),
            "main_scenario_for_progress({progress}) mismatch"
        );
    }

    for expected_flag in ref_flags.entries() {
        assert_eq!(
            db.flag_condition(&expected_flag.row_key).unwrap().as_ref(),
            Some(expected_flag)
        );
    }
    let for_100: Vec<String> = db
        .flag_conditions_for_flag(100)
        .unwrap()
        .into_iter()
        .map(|e| e.row_key)
        .collect();
    assert_eq!(for_100, vec!["GateHalfOpen", "GateSealed"]);

    for expected_pl in ref_playable.all_mappings() {
        assert_eq!(
            db.playable_by_map_unit_id(expected_pl.map_unit_id)
                .unwrap()
                .as_ref(),
            Some(expected_pl)
        );
    }
    assert!(db.playable_by_map_unit_id(901).unwrap().is_none());

    // 7b. Catalog, quest/shop search, and availability helpers.
    assert_eq!(db.chapters().unwrap(), ref_world.chapters());
    let ms_counts = db.main_scenario_counts_by_chapter().unwrap();
    for ch in ref_world.chapters() {
        assert_eq!(
            ms_counts.get(&ch.chapter_no).copied().unwrap_or(0),
            ref_world.main_objectives_by_chapter(ch.chapter_no).len()
        );
    }
    let maps_cat = db.maps_catalog().unwrap();
    assert_eq!(maps_cat.len(), ref_world.maps().len());
    for (entry, expected) in maps_cat.iter().zip(ref_world.maps()) {
        assert_eq!(&entry.map, expected);
        assert_eq!(
            entry.teleportable,
            ref_world.is_teleport_destination(expected.id)
        );
        assert_eq!(
            entry.treasure_count,
            ref_world.treasure_boxes_in_map(expected.id).len()
        );
    }
    assert_eq!(
        db.quests_by_chapter(1)
            .unwrap()
            .iter()
            .map(|q| q.id)
            .collect::<Vec<_>>(),
        ref_world
            .quests_by_chapter(1)
            .iter()
            .map(|q| q.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        db.quests_by_name_zh("灯火")
            .unwrap()
            .iter()
            .map(|q| q.id)
            .collect::<Vec<_>>(),
        ref_world
            .search_quests_by_name("灯火")
            .iter()
            .map(|q| q.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        db.shops_by_name_zh("道具店")
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(db.has_world_data().unwrap());
    assert!(db.has_scenario_data().unwrap());
    assert!(db.has_playable_units().unwrap());
    assert!(db.has_flag_conditions().unwrap());

    // 8. Summary alignment: rebuild WorldData strictly from DB objects and compare all summaries.
    let db_world = rebuild_world_data_from_db(db, &ref_world);
    for map in ref_world.maps() {
        assert_eq!(
            map_summary(&db_world, db_world.map_by_id(map.id).unwrap()),
            map_summary(&ref_world, map),
            "map_summary mismatch for map {}",
            map.id
        );
    }
    for item in ref_world.items() {
        assert_eq!(
            item_summary(&db_world, db_world.item_by_id(item.id).unwrap()),
            item_summary(&ref_world, item),
            "item_summary mismatch for item {}",
            item.id
        );
    }
    for quest in ref_world.quests() {
        assert_eq!(
            quest_summary(&db_world, db_world.quest_by_id(quest.id).unwrap()),
            quest_summary(&ref_world, quest),
            "quest_summary mismatch for quest {}",
            quest.id
        );
    }
    for shop in ref_world.shops() {
        assert_eq!(
            shop_summary(&db_world, db_world.shop_by_id(shop.id).unwrap()),
            shop_summary(&ref_world, shop),
            "shop_summary mismatch for shop {}",
            shop.id
        );
    }
    for ch in ref_world.chapters() {
        let ref_entries = ref_world.main_objectives_by_chapter(ch.chapter_no);
        let db_entries = db_world.main_objectives_by_chapter(ch.chapter_no);
        assert_eq!(db_entries.len(), ref_entries.len());
        for (db_e, ref_e) in db_entries.iter().zip(ref_entries.iter()) {
            assert_eq!(
                main_objective_summary(&db_world, db_e),
                main_objective_summary(&ref_world, ref_e)
            );
        }
    }
}

#[test]
fn every_world_query_uses_indexes_with_no_full_table_scans() {
    let test_db =
        build_fixture_db_stages("world", Some(vec![Stage::World])).expect("build world fixture db");
    let db = test_db.db();

    let world_tables = [
        "map",
        "island",
        "region",
        "map_release_condition",
        "save_point",
        "save_point_command",
        "teleport_point",
        "teleport_point_target",
        "map_link",
        "treasure_box",
        "item",
        "shop",
        "shop_item",
        "various_shop",
        "various_shop_member",
        "inn",
        "quest",
        "quest_prize",
        "quest_board",
        "quest_board_reward",
        "chapter",
        "main_scenario",
        "main_quest",
        "flag_condition",
        "flag_condition_term",
        "playable_unit",
        "item_drop",
        "world_name",
        "world_name_fts",
    ];

    // map(id) & map_by_level(name)
    assert_db_uses_index(
        db,
        "SELECT doc FROM map WHERE id = ?1",
        [3001_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM map WHERE level_name = ?1",
        ["Dng_01Fake_43_PL"],
        &world_tables,
    );

    // maps_by_name (3+ char FTS5 and 1-2 char covering index)
    assert_db_uses_index(
        db,
        "SELECT m.id, m.doc FROM world_name_fts \
         JOIN map m ON m.id = world_name_fts.entity_id \
         WHERE world_name_fts MATCH ?1 \
           AND world_name_fts.entity_kind = 'map' \
           AND world_name_fts.lang = ?2 \
         ORDER BY m.id",
        params!["\"微光洞\"", "zh-Hans"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT m.id, m.doc FROM world_name w \
         JOIN map m ON m.id = w.entity_id \
         WHERE w.entity_kind = 'map' AND w.lang = ?1 AND instr(w.name_lower, ?2) > 0 \
         ORDER BY m.id",
        params!["zh-Hans", "港镇"],
        &world_tables,
    );

    // map_contents sub-queries
    assert_db_uses_index(
        db,
        "SELECT doc FROM island WHERE island_type = ?1",
        ["eMapIslandType::Grandshelt"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM region WHERE id = ?1",
        [1000_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM map_release_condition WHERE island_type = ?1",
        ["eMapIslandType::Grandshelt"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM save_point WHERE map_type = ?1",
        ["eMapType::Dungeon"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM save_point_command WHERE command_type = ?1",
        ["ESavePointCommandType::Tent"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT DISTINCT tp.id, tp.doc FROM teleport_point_target t \
         JOIN teleport_point tp ON tp.id = t.teleport_id \
         WHERE t.map_id = ?1 ORDER BY tp.id",
        [3001_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM treasure_box WHERE map_id = ?1 ORDER BY id",
        [3001_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM inn WHERE id = ?1",
        [-1_i64],
        &world_tables,
    );

    // map_links_from & map_links_to
    assert_db_uses_index(
        db,
        "SELECT from_map, to_map, kind, via, condition FROM map_link \
         WHERE from_map = ?1 ORDER BY to_map, kind, via, condition",
        [1000_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT from_map, to_map, kind, via, condition FROM map_link \
         WHERE to_map = ?1 ORDER BY from_map, kind, via, condition",
        [3001_i64],
        &world_tables,
    );

    // item(id) & items_by_name
    assert_db_uses_index(
        db,
        "SELECT doc FROM item WHERE id = ?1",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT i.id, i.doc FROM world_name_fts \
         JOIN item i ON i.id = world_name_fts.entity_id \
         WHERE world_name_fts MATCH ?1 \
           AND world_name_fts.entity_kind = 'item' \
           AND world_name_fts.lang = ?2 \
         ORDER BY i.id",
        params!["\"玄铁大\"", "zh-Hans"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT i.id, i.doc FROM world_name w \
         JOIN item i ON i.id = w.entity_id \
         WHERE w.entity_kind = 'item' AND w.lang = ?1 AND instr(w.name_lower, ?2) > 0 \
         ORDER BY i.id",
        params!["zh-Hans", "大剑"],
        &world_tables,
    );

    // item_sources sub-queries (5 channels)
    assert_db_uses_index(
        db,
        "SELECT tb.map_id, m.name_zh, tb.id, tb.item_num, tb.is_trap \
         FROM treasure_box tb LEFT JOIN map m ON m.id = tb.map_id \
         WHERE tb.item_id = ?1 ORDER BY tb.map_id, tb.id",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT si.shop_id, s.name_zh, si.price, si.price_ratio, si.max_order_num, si.condition \
         FROM shop_item si JOIN shop s ON s.id = si.shop_id \
         WHERE si.item_id = ?1 ORDER BY si.shop_id, si.slot_index",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT qp.quest_id, q.name_zh, qp.count \
         FROM quest_prize qp JOIN quest q ON q.id = qp.quest_id \
         WHERE qp.item_id = ?1 ORDER BY qp.quest_id, qp.prize_index",
        [10100_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT qbr.quest_id, qb.name_zh, qbr.item_num \
         FROM quest_board_reward qbr JOIN quest_board qb ON qb.row_key = qbr.quest_board_row_key \
         WHERE qbr.item_id = ?1 ORDER BY qbr.quest_id, qbr.reward_index",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT unit_id, unit_name_zh, drop_kind, rate \
         FROM item_drop WHERE item_id = ?1 ORDER BY unit_id, drop_kind",
        [1_i64],
        &world_tables,
    );

    // shop(id), shops_selling(item_id), various_shop_members(id)
    assert_db_uses_index(
        db,
        "SELECT name_zh, doc FROM shop WHERE id = ?1",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT si.slot_index, si.item_id, si.price, si.price_ratio, si.max_order_num, si.condition, i.doc \
         FROM shop_item si LEFT JOIN item i ON i.id = si.item_id \
         WHERE si.shop_id = ?1 ORDER BY si.slot_index",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT DISTINCT s.id, s.doc FROM shop_item si \
         JOIN shop s ON s.id = si.shop_id \
         WHERE si.item_id = ?1 ORDER BY s.id",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT m.slot, s.id, s.doc FROM various_shop_member m \
         JOIN shop s ON s.id = m.shop_id \
         WHERE m.various_shop_id = ?1 ORDER BY m.slot",
        [10001_i64],
        &world_tables,
    );

    // quest, quest_board, chapter, main_quest, main_scenario_for_progress
    assert_db_uses_index(
        db,
        "SELECT doc FROM quest WHERE id = ?1",
        [10_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT row_key, doc FROM quest_board WHERE quest_id = ?1 AND is_placeholder = 0 LIMIT 1",
        [1001_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM chapter WHERE chapter_no = ?1",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM main_quest WHERE quest_id = ?1",
        [1001_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT row_key, doc FROM main_scenario WHERE chapter_no = ?1 ORDER BY min_progress ASC",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT min_progress FROM main_scenario \
         WHERE chapter_no = 1 AND min_progress IS NOT NULL \
         ORDER BY min_progress ASC LIMIT 1",
        [],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT row_key, min_progress, progress_end, doc FROM main_scenario \
         WHERE chapter_no = ?1 AND min_progress <= ?2 \
         ORDER BY min_progress DESC LIMIT 1",
        params![1_i64, 1040_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT row_key FROM main_scenario WHERE min_progress <= ?1 AND (progress_end IS NULL OR progress_end > ?1)",
        [1040_i64],
        &world_tables,
    );

    // flag_condition & playable_unit
    assert_db_uses_index(
        db,
        "SELECT doc FROM flag_condition WHERE row_key = ?1",
        ["PlayerRideChocobo"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT DISTINCT fc.row_key, fc.doc FROM flag_condition_term t \
         JOIN flag_condition fc ON fc.row_key = t.condition_key \
         WHERE t.flag_id = ?1 ORDER BY fc.row_key",
        [100_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT doc FROM playable_unit WHERE map_unit_id = ?1",
        [10_i64],
        &world_tables,
    );

    // Catalog, chapter/quest/shop search, and availability checks
    assert_db_uses_index(
        db,
        "SELECT chapter_no, doc FROM chapter WHERE chapter_no >= ?1 ORDER BY chapter_no",
        [i64::MIN],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT chapter_no, COUNT(*) FROM main_scenario \
         WHERE chapter_no IS NOT NULL GROUP BY chapter_no ORDER BY chapter_no",
        [],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT id, name_zh, doc FROM map WHERE id >= ?1 ORDER BY id",
        [i64::MIN],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT DISTINCT map_id FROM teleport_point_target ORDER BY map_id",
        [],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT map_id, COUNT(*) FROM treasure_box GROUP BY map_id",
        [],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM quest WHERE chapter_no = ?1 ORDER BY id",
        [1_i64],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM quest INDEXED BY idx_quest_name_zh \
         WHERE name_zh IS NOT NULL AND instr(name_zh, ?1) > 0 ORDER BY id",
        ["灯火"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM shop INDEXED BY idx_shop_name_zh \
         WHERE name_zh IS NOT NULL AND instr(name_zh, ?1) > 0 ORDER BY id",
        ["道具店"],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT 1 FROM map WHERE id >= ?1 LIMIT 1",
        [i64::MIN],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT 1 FROM chapter WHERE chapter_no >= ?1 LIMIT 1",
        [i64::MIN],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT 1 FROM playable_unit WHERE map_unit_id >= ?1 LIMIT 1",
        [i64::MIN],
        &world_tables,
    );
    assert_db_uses_index(
        db,
        "SELECT 1 FROM flag_condition WHERE row_key >= '' LIMIT 1",
        [],
        &world_tables,
    );
}

#[test]
fn real_export_world_full_alignment_and_spot_checks() {
    let export_root = real_export_dir();
    let map_file = datatable_dir(&export_root).join("Map/CDT_MapData_Prod.json");
    if !map_file.is_file() {
        eprintln!(
            "跳过真实导出世界数据库对齐测试：未找到 {}（设置 FFR_DEPS 或先运行 ../final-fantasy-resonance-export/scripts/run.sh export）",
            map_file.display()
        );
        return;
    }

    let test_db = build_temp_db(
        "world-real",
        BuildInputs {
            export: Some(export_root.clone()),
            ..BuildInputs::default()
        },
        Some(vec![Stage::World]),
    )
    .expect("build real world db");
    let db = test_db.db();

    let store = DataStore::new(&export_root);
    let ref_world = WorldData::from_store(&store).expect("load real WorldData");
    let ref_scenario = store.scenario().expect("load real ScenarioData");
    let ref_flags = FlagConditionData::from_store(&store).expect("load real FlagConditionData");
    let ref_playable =
        MapUnitPlayableData::from_store(&store).expect("load real MapUnitPlayableData");

    // 1. Full alignment for ALL maps, items, shops, quests, chapters, main_quests, flags, playable.
    for map in ref_world.maps() {
        assert_eq!(db.map(map.id).unwrap().as_ref(), Some(map));
        assert_eq!(
            db.map_by_level(&map.level_name).unwrap().as_ref(),
            Some(map)
        );
    }
    for item in ref_world.items() {
        assert_eq!(db.item(item.id).unwrap().as_ref(), Some(item));
    }
    for shop in ref_world.shops() {
        let resolved = db.shop(shop.id).unwrap().expect("real shop exists");
        assert_eq!(&resolved.shop, shop);
        assert_eq!(resolved.stock.len(), shop.items.len());
    }
    for quest in ref_world.quests() {
        assert_eq!(db.quest(quest.id).unwrap().as_ref(), Some(quest));
    }
    for ch in ref_world.chapters() {
        assert_eq!(db.chapter(ch.chapter_no).unwrap().as_ref(), Some(ch));
    }
    for mq in ref_scenario.main_quests() {
        assert_eq!(db.main_quest(mq.quest_id).unwrap().as_ref(), Some(&mq));
    }
    for entry in ref_flags.entries() {
        assert_eq!(
            db.flag_condition(&entry.row_key).unwrap().as_ref(),
            Some(entry)
        );
    }
    for pl in ref_playable.all_mappings() {
        assert_eq!(
            db.playable_by_map_unit_id(pl.map_unit_id).unwrap().as_ref(),
            Some(pl)
        );
    }

    // 2. Main scenario progress sampling across prologue, Ch 1-8, and post-game.
    for progress in [0, 1005, 1020, 1050, 2000, 3500, 5000, 8000, 9000] {
        assert_eq!(
            db.main_scenario_for_progress(progress).unwrap(),
            ref_scenario.current_main_scenario(progress),
            "real main_scenario_for_progress({progress}) mismatch"
        );
    }

    // 3. Summary alignment across all maps, items, shops, quests.
    let db_world = rebuild_world_data_from_db(db, &ref_world);
    for map in ref_world.maps() {
        assert_eq!(
            map_summary(&db_world, db_world.map_by_id(map.id).unwrap()),
            map_summary(&ref_world, map)
        );
    }
    for item in ref_world.items() {
        assert_eq!(
            item_summary(&db_world, db_world.item_by_id(item.id).unwrap()),
            item_summary(&ref_world, item)
        );
    }
    for quest in ref_world.quests() {
        assert_eq!(
            quest_summary(&db_world, db_world.quest_by_id(quest.id).unwrap()),
            quest_summary(&ref_world, quest)
        );
    }
    for shop in ref_world.shops() {
        assert_eq!(
            shop_summary(&db_world, db_world.shop_by_id(shop.id).unwrap()),
            shop_summary(&ref_world, shop)
        );
    }

    // 4. Spot-check map 3001 contents & item 1 sources.
    let m3001 = db.map_contents(3001).unwrap().expect("map 3001 exists");
    assert_eq!(m3001.map.level_name, "Dng_01Gra_43_PL");
    assert_eq!(m3001.treasure_boxes.len(), 2);
    assert_eq!(m3001.teleport_points.len(), 1);
    assert!(
        m3001
            .save_point
            .as_ref()
            .is_some_and(|sp| sp.can_save && sp.can_use_tent && sp.can_teleport)
    );

    let potion_sources = db.item_sources(1).unwrap();
    let box_count = potion_sources
        .iter()
        .filter(|s| matches!(s, ItemSource::TreasureBox { .. }))
        .count();
    let shop_count = potion_sources
        .iter()
        .filter(|s| matches!(s, ItemSource::Shop { .. }))
        .count();
    let drop_count = potion_sources
        .iter()
        .filter(|s| matches!(s, ItemSource::EnemyDrop { .. }))
        .count();
    assert_eq!(box_count, 2);
    assert_eq!(shop_count, 16);
    assert!(drop_count >= 30);

    // 5. shops_selling(1) matches WorldData::shops_selling_item(1).
    let db_shops_1: Vec<i64> = db
        .shops_selling(1)
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    let ref_shops_1: Vec<i64> = ref_world
        .shops_selling_item(1)
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(db_shops_1, ref_shops_1);
}
