use std::fs;

use ffr_data::assets::config::parse_ini;
use ffr_data::assets::registry::{AssetRefKind, collect_package_refs, parse_registry_line};
use ffr_db::query::assets::read_export_doc;
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db_stages, build_temp_db, fixture_area_dir,
};
use ffr_db::{BuildInputs, Stage, StageStatus};
use rusqlite::params;

const MAP_UNIT_PATH: &str = "FFRS/Content/Input/DataAsset/Map/DA_Input_MapUnit.uasset";
const MAP_UNIT_PACKAGE: &str = "/Game/Input/DataAsset/Map/DA_Input_MapUnit";
const CONTEXT_PATH: &str = "FFRS/Content/Input/Mapping/Map/IMC_Map_Unit_KBM.uasset";

fn fixture_db() -> ffr_db::testing::TestDb {
    build_fixture_db_stages("assets", Some(vec![Stage::Assets])).expect("build assets fixture")
}

#[test]
fn registry_and_ini_parsers_preserve_typed_data_and_operations() {
    let area = fixture_area_dir("assets");
    let registry =
        fs::read_to_string(area.join("inventory/registry.jsonl")).expect("read registry");
    let entry = parse_registry_line(registry.lines().next().expect("first registry line"))
        .expect("parse registry entry");
    assert_eq!(entry.package_name, MAP_UNIT_PACKAGE);
    assert_eq!(entry.tags.len(), 3);
    assert_eq!(entry.tags[0].key, "ActorsMetaData");
    assert_eq!(entry.tags[1].key, "Keywords");
    assert_eq!(entry.tags[2].key, "NativeClass");

    let export = fs::read_to_string(
        area.join("export-full/FFRS/Content/Input/DataAsset/Map/DA_Input_MapUnit.json"),
    )
    .expect("read export fixture");
    let document = serde_json::from_str(&export).expect("parse export fixture");
    let refs = collect_package_refs(&document, MAP_UNIT_PACKAGE);
    assert_eq!(refs.len(), 4, "self and duplicate references are removed");
    assert!(refs.iter().any(|reference| {
        reference.kind == AssetRefKind::Soft
            && reference.to_package == "/Game/Input/Action/Map/IA_Map_UnitMove"
    }));
    assert!(refs.iter().any(|reference| {
        reference.kind == AssetRefKind::Script
            && reference.to_package == "/Script/EnhancedInput.InputAction"
    }));

    let ini = "\u{feff}; comment\n[Section]\nKey=first\n+Key=second\n-Key=third\n.Key=fourth\n!Key=fifth\nKey=sixth\n";
    let entries = parse_ini("Fixture.ini", ini).expect("parse INI operations");
    let ops: Vec<&str> = entries.iter().map(|entry| entry.op.as_str()).collect();
    assert_eq!(ops, vec!["=", "+", "-", ".", "!", "="]);
    let values: Vec<&str> = entries.iter().map(|entry| entry.value.as_str()).collect();
    assert_eq!(
        values,
        vec!["first", "second", "third", "fourth", "fifth", "sixth"]
    );
    let ords: Vec<i64> = entries.iter().map(|entry| entry.ord).collect();
    assert_eq!(ords, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn asset_build_references_and_queries_round_trip() {
    let test_db = fixture_db();
    let db = test_db.db();
    let report = test_db
        .report()
        .stages
        .iter()
        .find(|report| report.stage == "assets")
        .expect("assets stage report");
    assert_eq!(report.status, StageStatus::Ok);
    assert_eq!(report.rows, 19);
    assert_eq!(test_db.report().total_issues, 1);

    let entry = db
        .asset(MAP_UNIT_PATH)
        .expect("asset by path")
        .expect("asset exists");
    assert_eq!(entry.asset_name, "DA_Input_MapUnit");
    assert_eq!(
        entry.export_path.as_deref(),
        Some("FFRS/Content/Input/DataAsset/Map/DA_Input_MapUnit.json")
    );
    assert_eq!(
        db.asset(MAP_UNIT_PACKAGE)
            .expect("asset by package")
            .expect("asset exists")
            .path,
        MAP_UNIT_PATH
    );
    assert_eq!(
        db.assets_by_class("CPP_DA_MUCHA_INPUT", 10)
            .expect("by class")
            .len(),
        1
    );
    assert_eq!(
        db.assets_by_class("CPP_DA_MUCHA_INPUT", 0)
            .expect("zero limit")
            .len(),
        0
    );
    assert_eq!(
        db.assets_in_dir("FFRS/Content/Input/", 10)
            .expect("by directory")
            .len(),
        3
    );

    let refs = db.asset_refs(MAP_UNIT_PATH).expect("outgoing refs");
    assert_eq!(refs.len(), 4);
    assert!(
        !refs
            .iter()
            .any(|reference| reference.to_package == MAP_UNIT_PACKAGE)
    );
    let soft = refs
        .iter()
        .find(|reference| reference.kind == AssetRefKind::Soft)
        .expect("soft ref");
    assert!(soft.to_asset_id.is_some());
    assert_eq!(
        soft.to_path.as_deref(),
        Some("FFRS/Content/Input/Action/Map/IA_Map_UnitMove.uasset")
    );
    let missing = refs
        .iter()
        .find(|reference| reference.to_package == "/Game/DoesNotExist/Missing")
        .expect("dangling ref");
    assert_eq!(missing.to_asset_id, None);
    assert!(refs.iter().any(|reference| {
        reference.kind == AssetRefKind::Script && reference.to_asset_id.is_none()
    }));

    let referrers = db.asset_referrers(MAP_UNIT_PACKAGE).expect("incoming refs");
    assert_eq!(referrers.len(), 1);
    assert_eq!(referrers[0].from_path, CONTEXT_PATH);
    assert_eq!(referrers[0].kind, AssetRefKind::Hard);

    let axis = db
        .config_values("/Script/Engine.InputSettings", "AxisConfig")
        .expect("config values");
    assert_eq!(axis.len(), 3);
    assert_eq!(axis[0].op, "-");
    assert_eq!(axis[1].op, "+");
    assert_eq!(axis[2].op, "+");
    let repeated = db
        .config_values("/Script/Engine.InputSettings", "bAltEnterTogglesFullscreen")
        .expect("duplicate config key");
    assert_eq!(repeated.len(), 2);
    assert_eq!(repeated[0].value, "True");
    assert_eq!(repeated[1].value, "False");

    let document = read_export_doc(area_export_full(), &entry).expect("read export doc");
    assert_eq!(document[0]["Name"], "DA_Input_MapUnit");
    let without_export = db
        .asset("FFRS/Content/Textures/T_Test.uasset")
        .expect("texture asset")
        .expect("texture exists");
    assert!(read_export_doc(area_export_full(), &without_export).is_err());

    let issues = db.build_issues().expect("build issues");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "dangling_asset_ref");
    assert!(issues[0].detail.contains("/Game/DoesNotExist/Missing"));
}

#[test]
fn excluded_actors_metadata_is_absent_and_reported() {
    let test_db = fixture_db();
    let db = test_db.db();
    let entry = db
        .asset(MAP_UNIT_PATH)
        .expect("query asset")
        .expect("asset");
    assert!(!entry.tags.iter().any(|tag| tag.key == "ActorsMetaData"));
    let rows = db
        .query_readonly_json("SELECT COUNT(*) AS count FROM asset_tag WHERE key = 'ActorsMetaData'")
        .expect("count excluded tag");
    assert_eq!(rows[0]["count"], 0);

    let registry = fs::read_to_string(fixture_area_dir("assets").join("inventory/registry.jsonl"))
        .expect("read registry");
    let source = parse_registry_line(registry.lines().next().expect("first registry line"))
        .expect("parse registry entry");
    let excluded_bytes = source
        .tags
        .iter()
        .find(|tag| tag.key == "ActorsMetaData")
        .expect("source tag")
        .value
        .len();
    let note = &test_db
        .report()
        .stages
        .iter()
        .find(|report| report.stage == "assets")
        .expect("assets report")
        .note;
    assert!(
        note.contains(&format!("ActorsMetaData(1 条,{excluded_bytes} 字节)")),
        "note={note}"
    );
    assert!(note.contains("原始数据在 registry.jsonl"));
}

#[test]
fn long_tag_values_stay_out_of_docs_and_indexes_but_round_trip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = temp.path().join("inventory");
    fs::create_dir_all(&inventory).expect("create inventory");
    let long_value = "x".repeat(300);
    let source = serde_json::json!({
        "path": MAP_UNIT_PATH,
        "packageName": MAP_UNIT_PACKAGE,
        "primaryClass": "CPP_DA_MUCHA_INPUT",
        "assetClasses": ["CPP_DA_MUCHA_INPUT"],
        "assetName": "DA_Input_MapUnit",
        "size": 100,
        "packageSize": 100,
        "bulkSize": 0,
        "tags": {"LongMetadata": long_value, "ShortTag": "short"},
        "dependencyCount": null,
        "matchedInVfs": true,
        "decision": "skip_binary",
        "category": "data_asset"
    });
    fs::write(
        inventory.join("registry.jsonl"),
        format!(
            "{}\n",
            serde_json::to_string(&source).expect("encode registry")
        ),
    )
    .expect("write registry");
    let test_db = build_temp_db(
        "assets-long-tag",
        BuildInputs {
            inventory: Some(inventory),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Assets]),
    )
    .expect("build long tag fixture");
    let db = test_db.db();
    let entry = db
        .asset(MAP_UNIT_PATH)
        .expect("query asset")
        .expect("asset");
    let long_tag = entry
        .tags
        .iter()
        .find(|tag| tag.key == "LongMetadata")
        .expect("long tag returned by asset query");
    assert_eq!(long_tag.value.len(), 300);

    let (stored_value, value_short, doc): (String, Option<String>, String) = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT asset_tag.value, asset_tag.value_short, asset.doc \
                 FROM asset_tag JOIN asset ON asset.id = asset_tag.asset_id \
                 WHERE asset.path = ?1 AND asset_tag.key = 'LongMetadata'",
                [MAP_UNIT_PATH],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|err| ffr_db::DbError::Sqlite {
                action: "读取长标签测试数据".to_owned(),
                source: err,
            })
        })
        .expect("read stored long tag");
    assert_eq!(stored_value, long_tag.value);
    assert_eq!(value_short, None);
    assert!(!doc.contains("tags"));
    assert!(!doc.contains(&stored_value));
}

#[test]
fn every_asset_query_uses_indexes() {
    let test_db = fixture_db();
    let db = test_db.db();

    assert_db_uses_index(
        db,
        "SELECT doc FROM asset WHERE id = ?1",
        [1_i64],
        &["asset"],
    );
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM asset WHERE primary_class = ?1 ORDER BY path LIMIT ?2",
        params!["CPP_DA_MUCHA_INPUT", 10_u32],
        &["asset"],
    );
    let upper = "FFRS/Content/Input/\u{10ffff}".to_string();
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM asset WHERE path >= ?1 AND path < ?2 ORDER BY path LIMIT ?3",
        params!["FFRS/Content/Input/", upper, 10_u32],
        &["asset"],
    );
    assert_db_uses_index(
        db,
        "SELECT id FROM asset WHERE path = ?1 \
         UNION ALL SELECT id FROM asset WHERE package_name = ?1 LIMIT 1",
        [MAP_UNIT_PATH],
        &["asset"],
    );
    assert_db_uses_index(
        db,
        "SELECT key, value FROM asset_tag WHERE asset_id = ?1 ORDER BY key",
        [1_i64],
        &["asset_tag"],
    );
    assert_db_uses_index(
        db,
        "SELECT asset_ref.from_asset_id, source_asset.path, source_asset.package_name, \
                asset_ref.to_package, asset_ref.to_asset_id, target.path, asset_ref.kind \
         FROM asset_ref \
         JOIN asset AS source_asset ON source_asset.id = asset_ref.from_asset_id \
         LEFT JOIN asset AS target ON target.id = asset_ref.to_asset_id \
         WHERE asset_ref.from_asset_id = ?1 \
         ORDER BY asset_ref.to_package, asset_ref.kind",
        [1_i64],
        &["asset_ref", "source_asset", "target"],
    );
    assert_db_uses_index(
        db,
        "SELECT asset_ref.from_asset_id, source_asset.path, source_asset.package_name, \
                asset_ref.to_package, asset_ref.to_asset_id, target.path, asset_ref.kind \
         FROM asset_ref \
         JOIN asset AS source_asset ON source_asset.id = asset_ref.from_asset_id \
         JOIN asset AS target ON target.id = asset_ref.to_asset_id \
         WHERE asset_ref.to_asset_id = ?1 \
         ORDER BY source_asset.path, asset_ref.kind",
        [1_i64],
        &["asset_ref", "source_asset", "target"],
    );
    assert_db_uses_index(
        db,
        "SELECT file, section, key, value, op, ord FROM config_entry \
         WHERE section = ?1 AND key = ?2 ORDER BY file, ord",
        params!["/Script/Engine.InputSettings", "AxisConfig"],
        &["config_entry"],
    );
}

fn area_export_full() -> std::path::PathBuf {
    fixture_area_dir("assets").join("export-full")
}

#[test]
fn case_insensitive_asset_refs_and_ambiguous_package_case_are_deterministic() {
    let make_registry_line = |path: &str, package_name: &str, asset_name: &str, decision: &str| {
        serde_json::to_string(&serde_json::json!({
            "path": path,
            "packageName": package_name,
            "primaryClass": "LevelSequence",
            "assetClasses": ["LevelSequence"],
            "assetName": asset_name,
            "size": 128,
            "packageSize": 128,
            "bulkSize": 0,
            "tags": {},
            "dependencyCount": null,
            "matchedInVfs": true,
            "decision": decision,
            "category": "sequence"
        }))
        .expect("encode registry line")
    };

    let caller_path = "FFRS/Content/Caller/BP_Caller.uasset";
    let caller_pkg = "/Game/Caller/BP_Caller";
    let target_path = "FFRS/Content/Sequencer/Battle/Summon/SEQ_Demo.uasset";
    let target_pkg = "/Game/Sequencer/Battle/Summon/SEQ_Demo";
    let dup_upper_path = "FFRS/Content/Case/DupPkg.uasset";
    let dup_upper_pkg = "/Game/Case/DupPkg";
    let dup_lower_path = "FFRS/Content/Case/duppkg.uasset";
    let dup_lower_pkg = "/Game/Case/duppkg";

    let build_with_order = |reverse_lines: bool| {
        let temp = tempfile::tempdir().expect("tempdir");
        let inventory = temp.path().join("inventory");
        let full = temp.path().join("export-full");
        fs::create_dir_all(&inventory).expect("create inventory");
        fs::create_dir_all(full.join("FFRS/Content/Caller")).expect("create export dir");

        let mut lines = [
            make_registry_line(caller_path, caller_pkg, "BP_Caller", "export_json"),
            make_registry_line(target_path, target_pkg, "SEQ_Demo", "skip_binary"),
            make_registry_line(dup_upper_path, dup_upper_pkg, "DupPkg", "skip_binary"),
            make_registry_line(dup_lower_path, dup_lower_pkg, "duppkg", "skip_binary"),
        ];
        if reverse_lines {
            lines.reverse();
        }
        fs::write(
            inventory.join("registry.jsonl"),
            format!("{}\n", lines.join("\n")),
        )
        .expect("write registry");

        let caller_export = serde_json::json!([
            {
                "Type": "BlueprintGeneratedClass",
                "Name": "BP_Caller_C",
                "Properties": {
                    "SummonSeq": {
                        "AssetPathName": "/Game/Sequencer/Battle/summon/SEQ_Demo.SEQ_Demo"
                    },
                    "AmbiguousTarget": {
                        "ObjectPath": "/Game/Case/DUPPKG.0"
                    }
                }
            }
        ]);
        fs::write(
            full.join("FFRS/Content/Caller/BP_Caller.json"),
            serde_json::to_string_pretty(&caller_export).expect("encode export json"),
        )
        .expect("write caller export");

        build_temp_db(
            "assets-case-insensitive",
            BuildInputs {
                inventory: Some(inventory),
                full: Some(full),
                ..BuildInputs::default()
            },
            Some(vec![Stage::Assets]),
        )
        .expect("build assets case test db")
    };

    let db_fwd = build_with_order(false);
    let db_rev = build_with_order(true);

    for test_db in [&db_fwd, &db_rev] {
        let db = test_db.db();
        let refs = db.asset_refs(caller_pkg).expect("outgoing refs");
        assert_eq!(refs.len(), 2);

        // Case-mismatched ref resolves `to_asset_id` while preserving raw `to_package` verbatim.
        let summon_ref = refs
            .iter()
            .find(|r| r.to_package == "/Game/Sequencer/Battle/summon/SEQ_Demo")
            .expect("raw to_package casing preserved");
        assert!(summon_ref.to_asset_id.is_some());
        assert_eq!(summon_ref.to_path.as_deref(), Some(target_path));

        // Incoming referrer query finds `BP_Caller` through resolved `to_asset_id`.
        let referrers = db.asset_referrers(target_pkg).expect("incoming refs");
        assert_eq!(referrers.len(), 1);
        assert_eq!(referrers[0].from_path, caller_path);

        // Case-colliding packages resolve deterministically to the lexicographically first
        // original package name (`/Game/Case/DupPkg` < `/Game/Case/duppkg`).
        let dup_ref = refs
            .iter()
            .find(|r| r.to_package == "/Game/Case/DUPPKG")
            .expect("ambiguous ref preserved");
        assert!(dup_ref.to_asset_id.is_some());
        assert_eq!(dup_ref.to_path.as_deref(), Some(dup_upper_path));

        // No `dangling_asset_ref` issue; exactly one `ambiguous_package_case` issue.
        let issues = db.build_issues().expect("build issues");
        assert!(
            !issues.iter().any(|i| i.kind == "dangling_asset_ref"),
            "unexpected dangling_asset_ref: {issues:?}"
        );
        let case_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.kind == "ambiguous_package_case")
            .collect();
        assert_eq!(
            case_issues.len(),
            1,
            "expected 1 ambiguous_package_case: {issues:?}"
        );
        assert_eq!(case_issues[0].stage, "assets");
        assert_eq!(case_issues[0].subject, dup_upper_pkg);
        assert!(case_issues[0].detail.contains(dup_upper_pkg));
        assert!(case_issues[0].detail.contains(dup_lower_pkg));
    }

    assert_eq!(
        db_fwd.report().content_sha256,
        db_rev.report().content_sha256,
        "case collision choice and issue recording must be order-independent"
    );
}
