//! Integration tests for the `08_levels` database area: fixture build, typed queries,
//! shortest-hop routing, various-shop expansion, 3D R*Tree spatial boundaries, and query plan
//! index verification (`assert_db_uses_index`).

use std::path::Path;

use ffr_data::assets::level::{
    LevelObjectData, LevelObjectKind, MapEdgeKind, Vec3, load_level_document, load_map_graph,
};
use ffr_db::query::levels::{ActorScope, sql};
use ffr_db::testing::{assert_db_uses_index, build_fixture_db_stages};
use ffr_db::{GameDb, Stage, StageStatus};
use rusqlite::params;

const ALL_LEVEL_TABLES: &[&str] = &[
    "level",
    "level_map",
    "actor",
    "actor_rtree",
    "level_object",
    "map_edge",
    "transition_point",
    "map_shop",
];

#[test]
fn levels_fixture_build_issues_and_stage_report() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let report = test_db.report();

    let stage_rep = report
        .stages
        .iter()
        .find(|s| s.stage == "levels")
        .expect("levels stage report");
    assert_eq!(stage_rep.status, StageStatus::Ok);
    assert!(stage_rep.rows > 0, "levels stage must insert rows");
    assert!(
        stage_rep.note.contains("跳过跨区域校验"),
        "standalone levels build must note skipped world checks, got: {}",
        stage_rep.note
    );

    let issues = test_db.db().build_issues().expect("read build_issue");
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "invalid_condition" && i.subject.contains("BP_Event_1")),
        "expected invalid_condition for malformed `=<`, got: {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "duplicate_transition_point" && i.subject == "map:100:point:0"),
        "expected duplicate_transition_point for (100, 0), got: {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "duplicate_level_name" && i.subject == "DupSub_KD"),
        "expected duplicate_level_name for disambiguated DupSub_KD, got: {issues:?}"
    );
}

#[test]
fn level_lookup_and_multi_map_levels_of_map() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let db = test_db.db();

    let pl = db
        .level("TestTown_PL")
        .expect("query level")
        .expect("found");
    assert_eq!(pl.sublevel_type, "PL");
    assert_eq!(pl.map_id, Some(100));
    assert_eq!(pl.streaming_levels.len(), 3);

    // Disambiguated duplicate-basename sublevel is accessible by its unique stem.
    let disambig = db
        .level("TestDisambig__DupSub_KD")
        .expect("query disambiguated level")
        .expect("disambiguated level exists");
    assert_eq!(disambig.name, "DupSub_KD");
    assert_eq!(disambig.map_id, Some(300));

    assert!(db.level("NoSuchLevel_GD").expect("query missing").is_none());

    // Multi-map level `SharedCorridor_GD` must appear in both map 100 and map 200.
    let m100_names: Vec<String> = db
        .levels_of_map(100)
        .expect("levels_of_map 100")
        .into_iter()
        .map(|l| l.unique_name().to_owned())
        .collect();
    assert_eq!(
        m100_names,
        vec![
            "SharedCorridor_GD",
            "TestTown_EV",
            "TestTown_GD",
            "TestTown_PL",
        ]
    );

    let m200_names: Vec<String> = db
        .levels_of_map(200)
        .expect("levels_of_map 200")
        .into_iter()
        .map(|l| l.unique_name().to_owned())
        .collect();
    assert_eq!(
        m200_names,
        vec!["DupSub_KD", "SharedCorridor_GD", "TestField_GD"]
    );
}

#[test]
fn map_objects_and_unique_id_lookup() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let db = test_db.db();

    let all_100 = db.map_objects(100, None).expect("all objects in 100");
    assert!(all_100.len() >= 15, "unexpected count: {}", all_100.len());

    let chests = db
        .map_objects(100, Some(LevelObjectKind::TreasureBox))
        .expect("chests in 100");
    assert_eq!(chests.len(), 1);
    match &chests[0].data {
        LevelObjectData::TreasureBox(tb) => {
            assert_eq!(tb.chest_id, Some(1));
            assert_eq!(tb.item_id, Some(501));
            assert_eq!(tb.item_num, Some(2));
        }
        other => panic!("expected TreasureBox, got {other:?}"),
    }

    let inns = db
        .map_objects(100, Some(LevelObjectKind::Inn))
        .expect("inns in 100");
    assert_eq!(inns.len(), 1);

    let events = db
        .map_objects(100, Some(LevelObjectKind::EventTrigger))
        .expect("events in 100");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].ref_id.as_deref(), Some("EV_TEST_01"));

    // Shared multi-map gimmick `BP_SharedTorch` (uniqueId 77) is visible in both map 100 and 200.
    let shared_100 = db
        .map_object_by_unique_id(100, 77)
        .expect("unique_id 77 in 100");
    let shared_200 = db
        .map_object_by_unique_id(200, 77)
        .expect("unique_id 77 in 200");
    assert_eq!(shared_100.len(), 1);
    assert_eq!(shared_200.len(), 1);
    assert_eq!(shared_100[0].actor_name, "BP_SharedTorch");

    // Shop NPC with unique_id 101 appears as both MapUnit and Shop.
    let npc_101 = db
        .map_object_by_unique_id(100, 101)
        .expect("unique_id 101 in 100");
    let kinds: Vec<LevelObjectKind> = npc_101.iter().map(|o| o.kind).collect();
    assert!(kinds.contains(&LevelObjectKind::MapUnit));
    assert!(kinds.contains(&LevelObjectKind::Shop));

    assert!(
        db.map_object_by_unique_id(100, 99999)
            .expect("missing unique_id")
            .is_empty()
    );
}

#[test]
fn map_exits_and_shortest_route_reachable_and_unreachable() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let db = test_db.db();

    // Map 100 exits: cross-map to 200 (point 0 -> (1000, 2000, 50)) and same-map to 100 (point 1 -> (600, 0, 0)).
    let exits_100 = db.map_exits(100).expect("map_exits 100");
    assert_eq!(exits_100.len(), 2);
    assert_eq!(exits_100[0].kind, MapEdgeKind::CrossMap);
    assert_eq!(exits_100[0].target_map_id, Some(200));
    assert_eq!(
        exits_100[0].target_location,
        Some(Vec3::new(1000.0, 2000.0, 50.0))
    );
    assert_eq!(
        exits_100[0].target_direction.as_deref(),
        Some("eMapDirectionType::North")
    );
    assert!(exits_100[0].is_auto_transition);

    assert_eq!(exits_100[1].kind, MapEdgeKind::SameMap);
    assert_eq!(exits_100[1].target_map_id, Some(100));
    assert_eq!(
        exits_100[1].target_location,
        Some(Vec3::new(600.0, 0.0, 0.0))
    );

    // Map 300 savepoint teleport -> map 100 point 0 must resolve to named BP_Point_0 (0, 0, 0),
    // not the unnamed duplicate BP_Point_0_Dup at (10, 0, 0).
    let exits_300 = db.map_exits(300).expect("map_exits 300");
    assert_eq!(exits_300.len(), 1);
    assert_eq!(exits_300[0].kind, MapEdgeKind::SavePointTeleport);
    assert_eq!(exits_300[0].target_location, Some(Vec3::new(0.0, 0.0, 0.0)));

    // Same start and destination -> 0 hops.
    assert_eq!(db.map_route(100, 100).expect("self route"), Some(vec![]));

    // Multi-hop route 100 -> 200 -> 300 traverses conditional edge and preserves its condition.
    let route_100_300 = db
        .map_route(100, 300)
        .expect("route 100->300")
        .expect("route exists");
    assert_eq!(route_100_300.len(), 2);
    assert_eq!(route_100_300[0].from_map_id, 100);
    assert_eq!(route_100_300[0].target_map_id, Some(200));
    assert_eq!(route_100_300[0].condition, None);
    assert_eq!(route_100_300[1].from_map_id, 200);
    assert_eq!(route_100_300[1].target_map_id, Some(300));
    assert_eq!(route_100_300[1].condition.as_deref(), Some("{flag:500}"));
    assert_eq!(
        route_100_300[1].target_location,
        Some(Vec3::new(-500.0, 0.0, -100.0))
    );

    // 1-hop return route via savepoint teleport.
    let route_300_100 = db
        .map_route(300, 100)
        .expect("route 300->100")
        .expect("return route exists");
    assert_eq!(route_300_100.len(), 1);
    assert_eq!(route_300_100[0].kind, MapEdgeKind::SavePointTeleport);

    // Unreachable isolated island map 400.
    assert_eq!(db.map_route(100, 400).expect("route 100->400"), None);
    assert_eq!(db.map_route(400, 100).expect("route 400->100"), None);
    assert_eq!(db.map_route(100, 9999).expect("route unknown"), None);
}

#[test]
fn shops_in_map_expands_various_shops_and_maps_with_shop_reverse_lookup() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let db = test_db.db();

    let shops = db.shops_in_map(100).expect("shops_in_map 100");
    // Direct shop 1001 + various shop 10001 expanded into 2001 & 4001.
    let pairs: Vec<(i64, Option<i64>)> = shops
        .iter()
        .map(|s| (s.shop_id, s.various_shop_id))
        .collect();
    assert_eq!(
        pairs,
        vec![(1001, None), (2001, Some(10001)), (4001, Some(10001))]
    );

    assert_eq!(db.maps_with_shop(1001).expect("direct shop"), vec![100]);
    assert_eq!(
        db.maps_with_shop(2001).expect("expanded weapon shop"),
        vec![100]
    );
    assert_eq!(
        db.maps_with_shop(4001).expect("expanded armor shop"),
        vec![100]
    );
    assert_eq!(
        db.maps_with_shop(10001).expect("various shop id"),
        vec![100]
    );
    assert!(db.maps_with_shop(8888).expect("missing shop").is_empty());
    assert!(db.shops_in_map(200).expect("map without shops").is_empty());
}

#[test]
fn actors_near_rtree_boundaries_scopes_and_kind_filter() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let db = test_db.db();
    let origin = Vec3::new(0.0, 0.0, 0.0);

    // 1. Exact point boundary: BP_Npc_VariousShop is at (100, 0, 0).
    let at_100 = db
        .actors_near(ActorScope::Level("TestTown_EV"), origin, 100.0, None)
        .expect("radius 100.0");
    let names_100: Vec<&str> = at_100.iter().map(|a| a.actor.name.as_str()).collect();
    // BP_Npc_SingleShop at (80, 80, 0) has dist ~113.137 -> must be excluded by spherical radius 100.
    assert_eq!(names_100, vec!["BP_Npc_VariousShop"]);

    let at_99_9 = db
        .actors_near(ActorScope::Level("TestTown_EV"), origin, 99.9, None)
        .expect("radius 99.9");
    assert!(at_99_9.is_empty(), "point at 100 must be outside 99.9");

    let at_113_2 = db
        .actors_near(ActorScope::Level("TestTown_EV"), origin, 113.2, None)
        .expect("radius 113.2");
    let names_113_2: Vec<&str> = at_113_2.iter().map(|a| a.actor.name.as_str()).collect();
    assert_eq!(names_113_2, vec!["BP_Npc_VariousShop", "BP_Npc_SingleShop"]);

    // 2. BoxExtent boundary: BP_ExitToField is centered at (100, 0, 0) with boxExtent (50, 50, 50)
    //    -> AABB X=[50, 150], so distance from origin is 50.0.
    let box_in = db
        .actors_near(
            ActorScope::Level("TestTown_GD"),
            origin,
            50.0,
            Some(LevelObjectKind::Transition),
        )
        .expect("box boundary in");
    assert_eq!(box_in.len(), 1);
    assert_eq!(box_in[0].actor.name, "BP_ExitToField");
    assert!((box_in[0].distance - 50.0).abs() < 1e-6);

    let box_out = db
        .actors_near(
            ActorScope::Level("TestTown_GD"),
            origin,
            49.9,
            Some(LevelObjectKind::Transition),
        )
        .expect("box boundary out");
    assert!(box_out.is_empty());

    // 3. SphereRadius boundary: BP_Encount_Sphere at (2000, 2000, 50) with sphereRadius 150
    //    -> AABB X=[1850, 2150]; distance from (1800, 2000, 50) is 50.0.
    let probe = Vec3::new(1800.0, 2000.0, 50.0);
    let sph_in = db
        .actors_near(
            ActorScope::Map(200),
            probe,
            50.0,
            Some(LevelObjectKind::EncounterZone),
        )
        .expect("sphere boundary in");
    assert_eq!(sph_in.len(), 1);
    assert_eq!(sph_in[0].actor.name, "BP_Encount_Sphere");

    let sph_out = db
        .actors_near(
            ActorScope::Map(200),
            probe,
            49.9,
            Some(LevelObjectKind::EncounterZone),
        )
        .expect("sphere boundary out");
    assert!(sph_out.is_empty());

    // 4. Map scope includes multi-map SharedCorridor_GD's BP_SharedTorch at (20, 0, 0) in BOTH
    //    map 100 and map 200, while excluding far StaticMesh_Far.
    let map100_near = db
        .actors_near(ActorScope::Map(100), origin, 30.0, None)
        .expect("map 100 near origin");
    let map100_names: Vec<&str> = map100_near.iter().map(|a| a.actor.name.as_str()).collect();
    assert!(map100_names.contains(&"BP_Point_0"));
    assert!(map100_names.contains(&"BP_SharedTorch"));
    assert!(!map100_names.contains(&"StaticMesh_Far"));

    let map200_near = db
        .actors_near(
            ActorScope::Map(200),
            Vec3::new(20.0, 0.0, 0.0),
            5.0,
            Some(LevelObjectKind::Gimmick),
        )
        .expect("map 200 shared torch");
    assert_eq!(map200_near.len(), 1);
    assert_eq!(map200_near[0].actor.name, "BP_SharedTorch");

    // 5. Negative radius returns empty cleanly.
    assert!(
        db.actors_near(ActorScope::Map(100), origin, -10.0, None)
            .expect("negative radius")
            .is_empty()
    );

    // 6. `level_objects_near` supports both 3D (z=Some) and 2D (z=None) queries + kind filters.
    let near_shops = db
        .level_objects_near(
            100,
            0.0,
            0.0,
            Some(0.0),
            120.0,
            Some(&[LevelObjectKind::Shop]),
        )
        .expect("level_objects_near shops");
    assert_eq!(near_shops.len(), 2);
    assert_eq!(near_shops[0].object.actor_name, "BP_Npc_VariousShop");
    assert!((near_shops[0].distance - 100.0).abs() < 1e-2);
    assert_eq!(near_shops[1].object.actor_name, "BP_Npc_SingleShop");

    // 2D horizontal query (`z = None`) finds objects regardless of Z elevation.
    let near_2d = db
        .level_objects_near(100, 100.0, 0.0, None, 10.0, Some(&[LevelObjectKind::Shop]))
        .expect("level_objects_near 2D");
    assert_eq!(near_2d.len(), 1);
    assert_eq!(near_2d[0].object.actor_name, "BP_Npc_VariousShop");

    // 7. `shop_locations` and `treasure_box_objects_for_item`.
    let locs_2001 = db.shop_locations(2001).expect("shop_locations 2001");
    assert_eq!(locs_2001.len(), 1);
    assert_eq!(locs_2001[0].map_id, 100);
    assert_eq!(locs_2001[0].various_shop_id, Some(10001));
    assert_eq!(locs_2001[0].actor_name, "BP_Npc_VariousShop");

    let chest_objs = db
        .treasure_box_objects_for_item(501)
        .expect("treasure_box_objects_for_item 501");
    assert_eq!(chest_objs.len(), 1);
    assert_eq!(chest_objs[0].map_id, Some(100));
    assert_eq!(chest_objs[0].kind, LevelObjectKind::TreasureBox);
}

#[test]
fn actors_by_class_and_encounter_zones() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let db = test_db.db();

    let all_gimmicks = db
        .actors_by_class("/Game/Blueprint/Map/BP_MapGimmick_", 20)
        .expect("actors_by_class gimmicks");
    assert_eq!(all_gimmicks.len(), 6);

    let limited = db
        .actors_by_class("/Game/Blueprint/Map/BP_MapGimmick_", 2)
        .expect("actors_by_class limit 2");
    assert_eq!(limited.len(), 2);

    // Actor_NoLoc (null location) must still be stored and queryable in `actor`.
    let no_loc = db
        .actors_by_class("/Script/Engine.WorldSettings", 10)
        .expect("actors_by_class WorldSettings");
    assert_eq!(no_loc.len(), 1);
    assert_eq!(no_loc[0].actor.name, "Actor_NoLoc");
    assert!(no_loc[0].actor.location.is_none());

    assert!(db.actors_by_class("", 10).expect("empty prefix").is_empty());

    let zones = db.encounter_zones(200).expect("encounter_zones 200");
    assert_eq!(zones.len(), 2);
    assert_eq!(zones[0].kind, "Box");
    assert_eq!(zones[0].battle_stage_id, Some(7));
    assert_eq!(zones[0].terrain_id, Some(2));
    assert_eq!(
        zones[0].encount_datatable.as_deref(),
        Some("/Game/Datatable/Encount/DT_TestField_Encount")
    );
    assert_eq!(zones[1].kind, "Sphere");
    assert_eq!(zones[1].sphere_radius, Some(150.0));
}

#[test]
fn all_level_queries_use_indexes_without_full_table_scans() {
    let test_db =
        build_fixture_db_stages("levels", Some(vec![Stage::Levels])).expect("build levels fixture");
    let db = test_db.db();

    assert_db_uses_index(db, sql::LEVEL_BY_NAME, ["TestTown_GD"], ALL_LEVEL_TABLES);
    assert_db_uses_index(db, sql::LEVEL_ID_BY_NAME, ["TestTown_GD"], ALL_LEVEL_TABLES);
    assert_db_uses_index(db, sql::LEVELS_OF_MAP, [100_i64], ALL_LEVEL_TABLES);
    assert_db_uses_index(db, sql::MAP_OBJECTS_ALL, [100_i64], ALL_LEVEL_TABLES);
    assert_db_uses_index(
        db,
        sql::MAP_OBJECTS_BY_KIND,
        params![100_i64, "treasure_box"],
        ALL_LEVEL_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::MAP_OBJECT_BY_UNIQUE_ID,
        params![100_i64, 101_i64],
        ALL_LEVEL_TABLES,
    );
    assert_db_uses_index(db, sql::MAP_EXITS, [100_i64], ALL_LEVEL_TABLES);
    assert_db_uses_index(
        db,
        sql::MAP_OUTGOING_CROSS_EDGES,
        [100_i64],
        ALL_LEVEL_TABLES,
    );
    assert_db_uses_index(db, sql::SHOPS_IN_MAP, [100_i64], ALL_LEVEL_TABLES);
    assert_db_uses_index(db, sql::MAPS_WITH_SHOP, [2001_i64], ALL_LEVEL_TABLES);
    assert_db_uses_index(
        db,
        sql::SHOP_LOCATIONS_BY_SHOP_ID,
        [2001_i64],
        ALL_LEVEL_TABLES,
    );
    assert_db_uses_index(db, sql::MAP_UNIT_NAMES_BY_ID, [1060_i64], &["map_unit"]);
    assert_db_uses_index(
        db,
        sql::TREASURE_BOX_OBJECTS_BY_ITEM,
        ["501"],
        ALL_LEVEL_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::ACTORS_BY_CLASS_PREFIX,
        params!["/Game/Blueprint/", "/Game/Blueprint/\u{10FFFF}", 10_i64],
        ALL_LEVEL_TABLES,
    );
    assert_db_uses_index(db, sql::ENCOUNTER_ZONES_BY_MAP, [200_i64], ALL_LEVEL_TABLES);

    // R*Tree queries must avoid full scans AND explicitly drive from `actor_rtree` virtual table index.
    let rtree_cases: &[(&str, bool)] = &[
        (sql::ACTORS_NEAR_LEVEL, false),
        (sql::ACTORS_NEAR_LEVEL_KIND, true),
        (sql::ACTORS_NEAR_MAP, false),
        (sql::ACTORS_NEAR_MAP_KIND, true),
        (sql::LEVEL_OBJECTS_NEAR_MAP, false),
    ];
    for &(query_sql, has_kind) in rtree_cases {
        let plan = if has_kind {
            assert_db_uses_index(
                db,
                query_sql,
                params![
                    -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, 100_i64,
                    "shop"
                ],
                ALL_LEVEL_TABLES,
            );
            db.explain_query_plan_with_params(
                query_sql,
                params![
                    -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, 100_i64,
                    "shop"
                ],
            )
            .expect("explain rtree kind query")
        } else {
            assert_db_uses_index(
                db,
                query_sql,
                params![
                    -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, 100_i64
                ],
                ALL_LEVEL_TABLES,
            );
            db.explain_query_plan_with_params(
                query_sql,
                params![
                    -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, -100.0_f64, 100.0_f64, 100_i64
                ],
            )
            .expect("explain rtree query")
        };
        assert!(
            plan.iter().any(|row| {
                (row.detail.contains("actor_rtree") || row.detail.contains("SCAN r "))
                    && row.detail.contains("VIRTUAL TABLE INDEX")
            }),
            "expected R*Tree virtual table index in plan for {query_sql}: {plan:?}"
        );
    }
}

#[test]
fn real_export_spot_check_when_deps_present() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let maps_dir = repo_root.join(".deps/ffr/export-full/maps");
    let gd_file = maps_dir.join("Dng_01Gra_43_GD.json");
    let graph_file = maps_dir.join("_graph.json");
    if !gd_file.is_file() || !graph_file.is_file() {
        // Real exported data is not committed to the repo; skip automatically when absent.
        return;
    }

    let doc = load_level_document(&gd_file).expect("parse real Dng_01Gra_43_GD.json");
    assert_eq!(doc.level.map_id, Some(3001));
    assert_eq!(doc.summary.save_points.len(), 1);
    let sp_loc = doc.summary.save_points[0]
        .location
        .expect("save point location");
    assert!((sp_loc.x - -3861.933).abs() < 1e-2);
    assert!((sp_loc.y - -550.627).abs() < 1e-2);
    assert!((sp_loc.z - -200.0).abs() < 1e-2);

    let graph = load_map_graph(&graph_file).expect("parse real _graph.json");
    assert_eq!(graph.maps.len(), 186);

    // If a real DB was already built at `.deps/ffr/db/ffr.sqlite` with levels populated, verify queries on it.
    let db_path = repo_root.join(".deps/ffr/db/ffr.sqlite");
    if db_path.is_file()
        && let Ok(db) = GameDb::open(&db_path)
        && db.level("Dng_01Gra_43_GD").ok().flatten().is_some()
    {
        let route = db
            .map_route(3001, 1000)
            .expect("query real route 3001 -> 1000")
            .expect("route 3001 -> 1000 must exist");
        assert_eq!(route.len(), 1);
        assert_eq!(route[0].target_map_id, Some(1000));
        assert!(route[0].target_location.is_some());
    }
}
