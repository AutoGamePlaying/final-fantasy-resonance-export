use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ffr_data::world::{
    ItemCategory, ItemDetail, WorldData, extract_level_name, item_summary, main_objective_summary,
    map_summary, parse_consumables, parse_maps, parse_quests, parse_shops,
    parse_teleport_points_document, quest_summary, shop_summary,
};
use ffr_data::{
    DataError, DataStore, DataTable, DataTableType, FFR_DEPS_ENV, TextSource, datatable_dir,
    export_dir_from_deps, localization_file,
};
use serde_json::json;

fn fixture_world_export() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rust/world/export")
        .canonicalize()
        .expect("world fixture export directory exists")
}

fn real_export_dir() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return export_dir_from_deps(PathBuf::from(deps));
    }
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ffr-data sits two levels below repository root");
    export_dir_from_deps(repo_root.join(".deps"))
}

#[test]
fn extract_level_name_handles_ue_paths_and_runtime_names() {
    assert_eq!(
        extract_level_name("/Game/Map/Com/00Com/00Com_45/Com_00Com_45_PL.Com_00Com_45_PL")
            .as_deref(),
        Some("Com_00Com_45_PL")
    );
    assert_eq!(
        extract_level_name("/Game/Map/Dng/01Gra/01Gra_43/Dng_01Gra_43_PL").as_deref(),
        Some("Dng_01Gra_43_PL")
    );
    assert_eq!(
        extract_level_name("Event_00Eve_86_PL:PersistentLevel").as_deref(),
        Some("Event_00Eve_86_PL")
    );
    assert_eq!(
        extract_level_name("  Com_00Com_46_PL  ").as_deref(),
        Some("Com_00Com_46_PL")
    );
    assert_eq!(extract_level_name(""), None);
    assert_eq!(extract_level_name("   "), None);
}

#[test]
fn fake_world_maps_savepoints_teleports_and_treasures() {
    let export = fixture_world_export();
    let store = DataStore::new(&export);
    let world = WorldData::from_store(&store).expect("load fake WorldData via DataStore");
    assert_eq!(store.cached_data_table_count(), 20);

    // Map ID and level_name reverse lookups.
    assert_eq!(world.maps().len(), 5);
    assert_eq!(world.map_id_by_level_name("Town_01Fake_01_PL"), Some(2001));
    assert_eq!(
        world.map_id_by_level_name("/Game/Map/Dng/01Fake/Dng_01Fake_43_PL.Dng_01Fake_43_PL"),
        Some(3001)
    );
    assert_eq!(
        world.map_id_by_level_name("Com_00Fake_45_PL:PersistentLevel"),
        Some(10040)
    );
    assert_eq!(world.map_id_by_level_name("Com_00Fake_46_PL"), Some(10041));
    assert_eq!(
        world.map_id_by_level_name("Event_00Fake_86_PL"),
        Some(20000)
    );
    assert_eq!(world.map_id_by_level_name("Missing_PL"), None);

    let cavern = world.map_by_id(3001).expect("cavern exists");
    assert_eq!(cavern.level_name, "Dng_01Fake_43_PL");
    assert_eq!(cavern.danger_level, 12);
    assert_eq!(cavern.encount_group_id, Some(77));
    assert_eq!(world.map_name_zh(cavern).text.as_deref(), Some("微光洞窟"));

    // Duplicate zh-Hans map names return all matching maps sorted by ID.
    let trials: Vec<i64> = world
        .find_maps_by_name("试炼场")
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(trials, vec![10040, 10041]);
    assert_eq!(world.search_maps_by_name("港镇").len(), 1);
    assert_eq!(world.maps_by_island("eMapIslandType::Grandshelt").len(), 2);
    assert_eq!(
        world
            .release_condition_for_island("eMapIslandType::Grandshelt")
            .map(|r| r.condition.as_str()),
        Some("{progress:0}>=2000")
    );
    assert!(world.region_by_id(1000).is_some());

    // Save points & commands.
    let town_sp = world
        .save_point_for_map_type("eMapType::Town")
        .expect("town save point");
    assert!(town_sp.can_save);
    assert!(!town_sp.can_use_tent);
    assert!(!town_sp.can_teleport);

    let dungeon_sp = world
        .save_point_for_map_type("eMapType::Dungeon")
        .expect("dungeon save point");
    assert!(dungeon_sp.can_save && dungeon_sp.can_use_tent && dungeon_sp.can_teleport);

    let tent_cmd = world
        .save_point_command("ESavePointCommandType::Tent")
        .expect("tent command");
    assert_eq!(tent_cmd.item_id, Some(500));

    // World-map teleport landmarks.
    assert_eq!(world.teleport_points().len(), 2);
    assert!(world.is_teleport_destination(2001));
    assert!(world.is_teleport_destination(3001));
    assert!(!world.is_teleport_destination(10040));
    let tp_ids: Vec<i64> = world
        .teleportable_maps()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(tp_ids, vec![2001, 3001]);
    let cavern_tps = world.teleport_points_for_map(3001);
    assert_eq!(cavern_tps.len(), 1);
    assert_eq!(cavern_tps[0].airship_landing_point_id, Some(7002));

    // Treasure boxes.
    let chests = world.treasure_boxes_in_map(3001);
    assert_eq!(chests.len(), 2);
    assert_eq!(chests[0].item_id, Some(1));
    assert_eq!(chests[0].item_num, 2);
    assert_eq!(chests[0].money, None);
    let trap_chest = world.treasure_box(3001, 1).expect("trap chest exists");
    assert!(trap_chest.is_trap);
    assert_eq!(trap_chest.money, Some(500));
    assert_eq!(trap_chest.battle_group_id, Some(99));
    assert_eq!(
        trap_chest.battle_finish_event_id.as_deref(),
        Some("Ev_ChestTrap_Done")
    );
}

#[test]
fn fake_world_items_quests_shops_and_chinese_summaries() {
    let world = WorldData::load(fixture_world_export()).expect("load fake WorldData");

    // 6 item categories sharing global ItemId.
    assert_eq!(world.items().len(), 7);
    assert_eq!(world.items_by_category(ItemCategory::Consumable).len(), 2);
    assert_eq!(world.items_by_category(ItemCategory::Seed).len(), 1);
    assert_eq!(world.items_by_category(ItemCategory::Equipment).len(), 1);
    assert_eq!(world.items_by_category(ItemCategory::Vision).len(), 1);
    assert_eq!(world.items_by_category(ItemCategory::Valuable).len(), 1);
    assert_eq!(world.items_by_category(ItemCategory::Artifact).len(), 1);

    let blade = world.item_by_id(10100).expect("blade exists");
    assert_eq!(world.find_items_by_name("玄铁大剑").len(), 1);
    assert_eq!(world.search_items_by_name("大剑").len(), 1);
    match &blade.detail {
        ItemDetail::Equipment {
            params,
            passive_skill_ids,
            ..
        } => {
            assert_eq!(params.len(), 2);
            assert_eq!(passive_skill_ids, &vec![1100]);
        }
        other => panic!("unexpected detail: {other:?}"),
    }

    // Untranslated inline seed item must NOT pretend Japanese is zh-Hans.
    let seed = world.item_by_id(9500).expect("seed exists");
    let seed_name = world.item_name_zh(seed);
    assert_eq!(seed_name.text, None);
    assert_eq!(seed_name.source, TextSource::Missing);
    assert!(world.find_items_by_name("未翻訳の種").is_empty());

    // Quests & chapter derivation & quest board.
    let q10 = world.quest_by_id(10).expect("quest 10");
    assert_eq!(q10.chapter_no, Some(1));
    assert_eq!(q10.min_main_progress, Some(1050));
    assert_eq!(q10.objectives.len(), 2);
    assert_eq!(q10.objectives[0].quest_board_ids, vec![1001]);
    assert_eq!(world.find_quests_by_name("点亮港湾灯火").len(), 1);
    assert_eq!(world.quests_by_chapter(1).len(), 1);

    let q26 = world.quest_by_id(26).expect("quest 26");
    assert_eq!(q26.chapter_no, None);

    // QuestBoard ignores -1 placeholder rows in ID index.
    assert!(world.quest_board_by_id(-1).is_none());
    let qb = world.quest_board_by_id(1001).expect("quest board 1001");
    assert_eq!(qb.clear_conditions[0].target_id, 1);
    assert_eq!(qb.clear_conditions[0].delivery_quantity, 3);
    assert_eq!(world.find_quest_board_by_name("交付清露草药").len(), 1);

    // Shops, PriceRatio, VariousShops, Inns.
    let item_shop = world.shop_by_id(1).expect("item shop 1");
    assert_eq!(item_shop.items.len(), 2);
    assert_eq!(world.find_shops_by_name("港镇道具店").len(), 1);
    assert_eq!(world.shops_selling_item(500).len(), 1);

    let weapon_shop = world.shop_by_id(2001).expect("weapon shop 2001");
    // Inline-only JP shop name has no zh-Hans translation and must not match JP as zh-Hans.
    assert!(world.find_shops_by_name("テスト武器屋").is_empty());
    // PriceRatio 5.0 * 400 = 2000; unknown item 9001 returns None cleanly.
    assert_eq!(world.shop_item_price(&weapon_shop.items[0]), Some(2000));
    assert_eq!(world.shop_item_price(&weapon_shop.items[1]), None);

    let bundle = world.various_shop_by_id(10001).expect("various shop 10001");
    assert_eq!(bundle.shop_ids(), vec![1, 2001]);
    assert_eq!(world.inns().len(), 1);
    assert_eq!(world.inns()[0].price, 100);

    // Chinese summaries for the AI model.
    let cavern_sum = map_summary(&world, world.map_by_id(3001).unwrap());
    assert!(cavern_sum.contains("微光洞窟"));
    assert!(cavern_sum.contains("Dng_01Fake_43_PL"));
    assert!(cavern_sum.contains("迷宫"));
    assert!(cavern_sum.contains("世界地图可传送"));
    assert!(cavern_sum.contains("存档/帐篷/迷宫内传送"));
    assert!(cavern_sum.contains("宝箱：2 个"));

    let blade_sum = item_summary(&world, blade);
    assert!(blade_sum.contains("玄铁大剑"));
    assert!(blade_sum.contains("武器/大剑"));
    assert!(blade_sum.contains("买价 400 / 卖价 200"));
    assert!(blade_sum.contains("物攻+32"));
    assert!(blade_sum.contains("被动技能：#1100"));

    let seed_sum = item_summary(&world, seed);
    assert!(seed_sum.contains("无中文译名"));
    assert!(!seed_sum.contains("未翻訳"));

    let quest_sum = quest_summary(&world, q10);
    assert!(quest_sum.contains("点亮港湾灯火"));
    assert!(quest_sum.contains("第 1 章「启程之风」"));
    assert!(quest_sum.contains("灯塔守卫"));
    assert!(quest_sum.contains("微风港镇"));
    assert!(quest_sum.contains("前往洞窟寻找火种"));
    assert!(quest_sum.contains("玄铁大剑×1"));
    assert!(quest_sum.contains("称号：灯火守望者"));

    let wshop_sum = shop_summary(&world, weapon_shop);
    assert!(wshop_sum.contains("无中文店名"));
    assert!(!wshop_sum.contains("テスト武器屋"));
    assert!(wshop_sum.contains("玄铁大剑（#10100，2000G，限购 1）"));
    assert!(wshop_sum.contains("未知道具 #9001"));

    let ch1_main = world.main_objectives_by_chapter(1);
    assert_eq!(ch1_main.len(), 1);
    let main_sum = main_objective_summary(&world, ch1_main[0]);
    assert!(main_sum.contains("探索微光洞窟深处"));
    assert!(main_sum.contains("微光洞窟"));
}

#[test]
fn world_parsers_report_chinese_schema_errors() {
    let wrong_table = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_Bad".to_owned(),
        package_path: "/Game/DT_Bad".to_owned(),
        row_struct: "WrongStruct".to_owned(),
        rows: BTreeMap::new(),
    };
    let map_err = parse_maps(&wrong_table, "CDT_MapData_Prod.json").unwrap_err();
    assert!(matches!(map_err, DataError::UnexpectedRowStruct { .. }));
    assert!(map_err.to_string().contains("MAP_DATA"));

    let item_err = parse_consumables(&wrong_table, "DT_ConsumablesItemData.json").unwrap_err();
    assert!(item_err.to_string().contains("CONSUMABLES_ITEM_DATA"));

    let quest_err = parse_quests(&wrong_table, "DT_RequestData.json").unwrap_err();
    assert!(quest_err.to_string().contains("RequestData"));

    let shop_err = parse_shops(&wrong_table, "DT_ShopList.json").unwrap_err();
    assert!(shop_err.to_string().contains("SHOP_ShopList1"));

    let bad_landmark = json!([{ "Type": "OtherAsset", "Properties": {} }]);
    let lm_err = parse_teleport_points_document(&bad_landmark, "DA_MapLandmark.json").unwrap_err();
    assert!(lm_err.to_string().contains("CPP_DA_MapLandmark"));
}

#[test]
fn real_export_world_tables_and_level_name_reverse_lookup() {
    let export_root = real_export_dir();
    let map_file = datatable_dir(&export_root).join("Map/CDT_MapData_Prod.json");
    if !map_file.is_file() {
        eprintln!(
            "跳过真实导出世界数据测试：未找到 {}（设置 FFR_DEPS 或先运行 ../final-fantasy-resonance-export/scripts/run.sh export）",
            map_file.display()
        );
        return;
    }

    let store = DataStore::new(&export_root);
    let world = WorldData::from_store(&store).expect("load real WorldData via DataStore");

    // Structural table row-count invariants (no game text literals).
    assert_eq!(world.maps().len(), 186);
    assert_eq!(world.islands().len(), 11);
    assert_eq!(world.regions().len(), 13);
    assert_eq!(world.release_conditions().len(), 8);
    assert_eq!(world.save_points().len(), 2);
    assert_eq!(world.save_point_commands().len(), 5);
    assert_eq!(world.teleport_points().len(), 61);
    assert_eq!(world.treasure_boxes().len(), 238);
    assert_eq!(world.items().len(), 589);
    assert_eq!(world.quests().len(), 33);
    assert_eq!(world.quest_board().len(), 10);
    assert_eq!(world.shops().len(), 105);
    assert_eq!(world.various_shops().len(), 20);
    assert_eq!(world.inns().len(), 1);

    // Every map must reverse-lookup uniquely by its extracted level_name.
    for map in world.maps() {
        assert_eq!(
            world.map_id_by_level_name(&map.level_name),
            Some(map.id),
            "关卡名 {} 反查应命中地图 {}",
            map.level_name,
            map.id
        );
    }

    // Spot-check the 4 level names observed on the real machine on 11-25.
    let expected_levels = [
        ("Com_00Com_45_PL", 10040, "eMapType::Dungeon"),
        ("Com_00Com_46_PL", 10041, "eMapType::Dungeon"),
        ("Event_00Eve_86_PL", 20000, "eMapType::Town"),
        ("Dng_01Gra_43_PL", 3001, "eMapType::Dungeon"),
    ];
    for (level_name, expected_id, expected_type) in expected_levels {
        let map = world
            .map_by_level_name(level_name)
            .unwrap_or_else(|| panic!("应能反查到关卡名 {level_name}"));
        assert_eq!(map.id, expected_id);
        assert_eq!(map.level_name, level_name);
        assert_eq!(map.map_type, expected_type);
        assert_eq!(
            world.map_id_by_level_name(&map.level_asset_path),
            Some(expected_id)
        );
    }

    // Tent item ID referenced by SavePointCommand must resolve to a Consumable item.
    let tent_cmd = world
        .save_point_command("ESavePointCommandType::Tent")
        .expect("real Tent command");
    let tent_id = tent_cmd.item_id.expect("Tent command references item ID");
    let tent_item = world.item_by_id(tent_id).expect("Tent item exists");
    assert_eq!(tent_item.category, ItemCategory::Consumable);

    // Teleportable landmarks hit real maps in CDT_MapData_Prod.
    assert!(world.teleportable_maps().len() >= 50);

    // Multilingual zh-Hans verification when Localization/zh-Hans.json is present.
    if localization_file(&export_root, "zh-Hans").is_file() {
        for (level_name, _, _) in expected_levels {
            let map = world.map_by_level_name(level_name).unwrap();
            let zh_name = world.map_name_zh(map);
            assert_eq!(zh_name.lang.as_deref(), Some("zh-Hans"));
            let name_str = zh_name.text.as_deref().expect("map has zh-Hans name");
            assert!(!name_str.is_empty());
            assert!(
                world
                    .find_maps_by_name(name_str)
                    .iter()
                    .any(|m| m.id == map.id)
            );
            let summary = map_summary(&world, map);
            assert!(summary.contains(level_name));
            assert!(summary.contains(name_str));
        }

        // Verify item & quest lookup by zh-Hans name and Chinese summary generation.
        let potion = world.item_by_id(1).expect("item 1 exists");
        let potion_zh = world
            .item_name_zh(potion)
            .text
            .expect("item 1 has zh-Hans name");
        assert!(
            world
                .find_items_by_name(&potion_zh)
                .iter()
                .any(|i| i.id == 1)
        );
        assert!(item_summary(&world, potion).contains(&potion_zh));

        let quest0 = world.quest_by_id(0).expect("quest 0 exists");
        let quest0_zh = world
            .quest_title_zh(quest0)
            .text
            .expect("quest 0 has zh-Hans title");
        assert!(
            world
                .find_quests_by_name(&quest0_zh)
                .iter()
                .any(|q| q.id == 0)
        );
        assert!(quest_summary(&world, quest0).contains(&quest0_zh));
    }
}
