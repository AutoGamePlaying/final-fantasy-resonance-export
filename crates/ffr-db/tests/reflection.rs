//! Integration tests for the reflection area (`06_reflection`):
//! multi-file `.jmap` merge, conflict & dangling-ref reporting, SHA-256 source tracking,
//! all `GameDb` reflection queries, `assert_uses_index` verification, repeatable CLI `--jmap`,
//! and optional spot checks on real `.deps/r1/` `.jmap` dumps when available.

use std::path::{Path, PathBuf};
use std::process::Command;

use ffr_data::assets::reflection::UeClassKind;
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db_stages, build_temp_db, fixture_area_dir,
};
use ffr_db::{BuildInputs, GameDb, Stage, StageStatus};
use rusqlite::params;

const REFLECTION_TABLES: &[&str] = &["ue_class", "ue_property", "ue_function", "ue_enum_value"];

#[test]
fn reflection_fixture_merges_multiple_jmaps_with_conflicts_dangling_refs_and_sha256() {
    let test_db = build_fixture_db_stages("reflection", Some(vec![Stage::Reflection]))
        .expect("build reflection fixture db");

    // Verify stage report succeeded.
    let stage_report = test_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "reflection")
        .expect("reflection stage report");
    assert_eq!(stage_report.status, StageStatus::Ok);
    assert!(
        stage_report.note.contains("files=2"),
        "note={}",
        stage_report.note
    );
    assert!(
        stage_report.note.contains("added_classes=2"),
        "note={}",
        stage_report.note
    );
    assert!(
        stage_report.note.contains("conflicts=1"),
        "note={}",
        stage_report.note
    );

    let db = test_db.db();

    // Verify source table recorded both jmap files with 64-char hex SHA-256 digests.
    let sources = db.sources().expect("read sources");
    let refl_sources: Vec<_> = sources.iter().filter(|s| s.stage == "reflection").collect();
    assert_eq!(refl_sources.len(), 2);
    for src in &refl_sources {
        let sha = src
            .manifest_sha256
            .as_deref()
            .expect("jmap source must have sha256");
        assert_eq!(sha.len(), 64, "invalid sha256: {sha}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(src.file_count.unwrap_or(0) > 0);
    }

    // Verify build_issue recorded exactly one merge conflict (not the identical CPP_TestBase
    // whose runtime memory addresses differed) and one dangling super_class.
    let issues = db.build_issues().expect("read build issues");
    let conflicts: Vec<_> = issues
        .iter()
        .filter(|i| i.kind == "merge_conflict")
        .collect();
    assert_eq!(conflicts.len(), 1, "issues={issues:?}");
    assert_eq!(conflicts[0].subject, "/Script/mucha.CPP_ConflictTarget");
    assert!(conflicts[0].detail.contains("size: 64 -> 80"));

    let dangling: Vec<_> = issues
        .iter()
        .filter(|i| i.kind == "dangling_super_class")
        .collect();
    assert_eq!(dangling.len(), 1, "issues={issues:?}");
    assert_eq!(dangling[0].subject, "/Game/BP/BP_Dangling.BP_Dangling_C");
    assert!(dangling[0].detail.contains("/Script/Missing.NoSuchBase"));

    // Later jmap must win for conflicting class definition.
    let resolved = db
        .ue_class("CPP_ConflictTarget")
        .expect("query CPP_ConflictTarget")
        .expect("CPP_ConflictTarget must exist");
    assert_eq!(resolved.size, 80);
    let conflict_props = db
        .class_properties("CPP_ConflictTarget", false)
        .expect("props of CPP_ConflictTarget");
    let prop_names: Vec<&str> = conflict_props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(prop_names, vec!["VersionTag", "ExtraFlag"]);

    // Class added only in the second jmap must be present.
    let grand_child = db
        .ue_class("CPP_TestGrandChild")
        .expect("query CPP_TestGrandChild")
        .expect("CPP_TestGrandChild must exist");
    assert_eq!(grand_child.size, 256);
    assert_eq!(
        grand_child.super_class.as_deref(),
        Some("/Script/mucha.CPP_TestChild")
    );
}

#[test]
fn reflection_queries_cover_classes_chains_properties_subclasses_enums_and_functions() {
    let test_db = build_fixture_db_stages("reflection", Some(vec![Stage::Reflection]))
        .expect("build reflection fixture db");
    let db = test_db.db();

    // 1. ue_class: full path, short name, struct, enum, missing, and ambiguous short name.
    let by_full = db
        .ue_class("/Script/mucha.CPP_TestChild")
        .expect("full path lookup")
        .expect("CPP_TestChild exists");
    let by_short = db
        .ue_class("CPP_TestChild")
        .expect("short name lookup")
        .expect("CPP_TestChild exists by short name");
    assert_eq!(by_full, by_short);
    assert_eq!(by_full.kind, UeClassKind::Class);

    let struct_def = db
        .ue_class("FMockData")
        .expect("struct lookup")
        .expect("FMockData exists");
    assert_eq!(struct_def.kind, UeClassKind::ScriptStruct);

    assert!(
        db.ue_class("NonExistentClass")
            .expect("missing class ok")
            .is_none()
    );

    let dup_err = db
        .ue_class("WBP_Dup_C")
        .expect_err("ambiguous short name must error");
    let dup_msg = dup_err.to_string();
    assert!(dup_msg.contains("/Game/UI/A.WBP_Dup_C"), "msg={dup_msg}");
    assert!(dup_msg.contains("/Game/UI/B.WBP_Dup_C"), "msg={dup_msg}");

    // Full path resolves ambiguous short names cleanly.
    let dup_a = db
        .ue_class("/Game/UI/A.WBP_Dup_C")
        .expect("full path disambiguates")
        .expect("WBP_Dup_C in A exists");
    assert_eq!(dup_a.package, "/Game/UI/A");

    // 2. class_chain (recursive CTE).
    let chain = db
        .class_chain("CPP_TestGrandChild")
        .expect("class_chain query");
    let chain_names: Vec<&str> = chain.iter().map(|c| c.short_name.as_str()).collect();
    assert_eq!(
        chain_names,
        vec![
            "CPP_TestGrandChild",
            "CPP_TestChild",
            "CPP_TestBase",
            "Subsystem",
            "Object",
        ]
    );
    assert!(
        db.class_chain("NoSuchClass")
            .expect("missing chain ok")
            .is_empty()
    );

    // 3. class_properties (own vs inherited, strictly ordered by byte offset).
    let own_child_props = db
        .class_properties("CPP_TestChild", false)
        .expect("own child props");
    let own_names_offsets: Vec<(&str, u32)> = own_child_props
        .iter()
        .map(|p| (p.name.as_str(), p.offset))
        .collect();
    // Fixture intentionally declared `Items` (152) before `Data` (64); query must sort by offset.
    assert_eq!(own_names_offsets, vec![("Data", 64), ("Items", 152)]);
    assert_eq!(
        own_child_props[0].struct_path.as_deref(),
        Some("/Script/mucha.FMockData")
    );
    assert_eq!(
        own_child_props[1].inner_summary().as_deref(),
        Some("ObjectProperty</Script/mucha.CPP_TestBase>")
    );

    let inherited_props = db
        .class_properties("CPP_TestGrandChild", true)
        .expect("inherited grandchild props");
    let inherited_pairs: Vec<(&str, u32)> = inherited_props
        .iter()
        .map(|p| (p.name.as_str(), p.offset))
        .collect();
    assert_eq!(
        inherited_pairs,
        vec![
            ("BaseFlag", 40),
            ("SharedProp", 48),
            ("Element", 52),
            ("Data", 64),
            ("Items", 152),
            ("Bonus", 168),
            ("TagSet", 176),
        ]
    );

    // 4. subclasses (direct vs recursive).
    let direct_subs = db
        .subclasses("CPP_TestBase", false)
        .expect("direct subclasses");
    assert_eq!(
        direct_subs
            .iter()
            .map(|c| c.short_name.as_str())
            .collect::<Vec<_>>(),
        vec!["CPP_TestChild"]
    );

    let all_subs = db
        .subclasses("CPP_TestBase", true)
        .expect("recursive subclasses");
    assert_eq!(
        all_subs
            .iter()
            .map(|c| c.short_name.as_str())
            .collect::<Vec<_>>(),
        vec!["CPP_TestChild", "CPP_TestGrandChild"]
    );

    // 5. ue_enum, enum_value, enum_name, enum_names.
    let elem_enum = db
        .ue_enum("eMockElement")
        .expect("ue_enum by short name")
        .expect("eMockElement exists");
    assert_eq!(elem_enum.kind, UeClassKind::Enum);
    assert_eq!(elem_enum.enum_values.len(), 5);
    assert!(
        db.ue_enum("CPP_TestBase")
            .expect("ue_enum ignores non-enum class")
            .is_none()
    );

    assert_eq!(
        db.enum_value("eMockElement", "eMockElement::Ice")
            .expect("full enum entry"),
        Some(2)
    );
    assert_eq!(
        db.enum_value("eMockElement", "Ice")
            .expect("short enum entry"),
        Some(2)
    );
    assert_eq!(
        db.enum_value("/Script/mucha.eMockElement", "Unknown")
            .expect("negative enum value"),
        Some(-1)
    );
    assert_eq!(
        db.enum_value("EMockRegular", "REG_Beta")
            .expect("regular enum entry"),
        Some(1)
    );
    assert_eq!(
        db.enum_value("eMockElement", "NoSuchValue")
            .expect("missing enum entry"),
        None
    );

    // Primary name follows declaration order (eMockElement::Ice before eMockElement::IceAlias).
    assert_eq!(
        db.enum_name("eMockElement", 2).expect("enum_name"),
        Some("eMockElement::Ice".to_owned())
    );
    assert_eq!(
        db.enum_names("eMockElement", 2).expect("enum_names"),
        vec![
            "eMockElement::Ice".to_owned(),
            "eMockElement::IceAlias".to_owned()
        ]
    );
    assert_eq!(
        db.enum_name("eMockElement", 999).expect("missing value"),
        None
    );

    // 6. classes_with_property.
    let owners = db
        .classes_with_property("SharedProp")
        .expect("classes_with_property");
    let owner_names: Vec<&str> = owners.iter().map(|c| c.short_name.as_str()).collect();
    assert_eq!(owner_names, vec!["CPP_TestBase", "FMockData"]);

    // 7. class_functions (verifying local non-CPF_Parm variables are excluded).
    let child_funcs = db
        .class_functions("CPP_TestChild", false)
        .expect("own functions");
    assert_eq!(child_funcs.len(), 1);
    assert_eq!(child_funcs[0].name, "Compute");
    let param_names: Vec<&str> = child_funcs[0]
        .params
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(param_names, vec!["Input", "OutVal"]);
    assert!(child_funcs[0].params[1].is_out_param());
    assert_eq!(
        child_funcs[0]
            .return_value
            .as_ref()
            .map(|r| r.name.as_str()),
        Some("ReturnValue")
    );

    let inherited_funcs = db
        .class_functions("CPP_TestGrandChild", true)
        .expect("inherited functions");
    assert_eq!(
        inherited_funcs
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Execute", "Compute"]
    );
}

#[test]
fn reflection_query_plans_use_indexes_for_every_lookup_pattern() {
    let test_db = build_fixture_db_stages("reflection", Some(vec![Stage::Reflection]))
        .expect("build reflection fixture db");
    let db = test_db.db();

    // ue_class(full_path) & ue_class(short_name)
    assert_db_uses_index(
        db,
        "SELECT doc FROM ue_class WHERE path = ?1",
        ["/Script/mucha.CPP_TestChild"],
        REFLECTION_TABLES,
    );
    assert_db_uses_index(
        db,
        "SELECT path, doc FROM ue_class WHERE short_name = ?1 ORDER BY path ASC",
        ["CPP_TestChild"],
        REFLECTION_TABLES,
    );

    // class_chain (recursive CTE)
    assert_db_uses_index(
        db,
        "WITH RECURSIVE chain(path, super_class, doc, depth) AS (\
            SELECT path, super_class, doc, 0 FROM ue_class WHERE path = ?1 \
            UNION ALL \
            SELECT ue_class.path, ue_class.super_class, ue_class.doc, chain.depth + 1 \
            FROM chain CROSS JOIN ue_class ON ue_class.path = chain.super_class \
            WHERE chain.super_class IS NOT NULL AND chain.depth < 64\
         ) SELECT path, doc FROM chain ORDER BY depth ASC",
        ["/Script/mucha.CPP_TestGrandChild"],
        REFLECTION_TABLES,
    );

    // class_properties (own & inherited)
    assert_db_uses_index(
        db,
        "SELECT owner || '.' || name, doc FROM ue_property WHERE owner = ?1 ORDER BY offset ASC, ord ASC",
        ["/Script/mucha.CPP_TestChild"],
        REFLECTION_TABLES,
    );
    assert_db_uses_index(
        db,
        "WITH RECURSIVE chain(path, super_class, depth) AS (\
            SELECT path, super_class, 0 FROM ue_class WHERE path = ?1 \
            UNION ALL \
            SELECT ue_class.path, ue_class.super_class, chain.depth + 1 \
            FROM chain CROSS JOIN ue_class ON ue_class.path = chain.super_class \
            WHERE chain.super_class IS NOT NULL AND chain.depth < 64\
         ) SELECT ue_property.owner || '.' || ue_property.name, ue_property.doc \
           FROM chain CROSS JOIN ue_property ON ue_property.owner = chain.path \
           ORDER BY ue_property.offset ASC, chain.depth DESC, ue_property.ord ASC",
        ["/Script/mucha.CPP_TestGrandChild"],
        REFLECTION_TABLES,
    );

    // subclasses (direct & recursive)
    assert_db_uses_index(
        db,
        "SELECT path, doc FROM ue_class WHERE super_class = ?1 ORDER BY path ASC",
        ["/Script/mucha.CPP_TestBase"],
        REFLECTION_TABLES,
    );
    assert_db_uses_index(
        db,
        "WITH RECURSIVE subs(path, doc, depth) AS (\
            SELECT path, doc, 1 FROM ue_class WHERE super_class = ?1 \
            UNION ALL \
            SELECT ue_class.path, ue_class.doc, subs.depth + 1 \
            FROM subs CROSS JOIN ue_class ON ue_class.super_class = subs.path \
            WHERE subs.depth < 64\
         ) SELECT path, doc FROM subs ORDER BY depth ASC, path ASC",
        ["/Script/CoreUObject.Object"],
        REFLECTION_TABLES,
    );

    // ue_enum (full path & short name)
    assert_db_uses_index(
        db,
        "SELECT doc FROM ue_class WHERE path = ?1 AND kind = 'enum'",
        ["/Script/mucha.eMockElement"],
        REFLECTION_TABLES,
    );
    assert_db_uses_index(
        db,
        "SELECT path, doc FROM ue_class WHERE kind = 'enum' AND short_name = ?1 ORDER BY path ASC",
        ["eMockElement"],
        REFLECTION_TABLES,
    );

    // enum_value (full & short) & enum_name
    assert_db_uses_index(
        db,
        "SELECT value FROM ue_enum_value WHERE enum = ?1 AND name = ?2",
        ["/Script/mucha.eMockElement", "eMockElement::Ice"],
        REFLECTION_TABLES,
    );
    assert_db_uses_index(
        db,
        "SELECT value FROM ue_enum_value WHERE enum = ?1 AND short_name = ?2 ORDER BY ord ASC LIMIT 1",
        ["/Script/mucha.eMockElement", "Ice"],
        REFLECTION_TABLES,
    );
    assert_db_uses_index(
        db,
        "SELECT name FROM ue_enum_value WHERE enum = ?1 AND value = ?2 ORDER BY ord ASC LIMIT 1",
        params!["/Script/mucha.eMockElement", 2_i64],
        REFLECTION_TABLES,
    );

    // classes_with_property
    assert_db_uses_index(
        db,
        "SELECT ue_class.path, ue_class.doc FROM ue_property \
         CROSS JOIN ue_class ON ue_class.path = ue_property.owner \
         WHERE ue_property.name = ?1 ORDER BY ue_class.path ASC",
        ["SharedProp"],
        REFLECTION_TABLES,
    );

    // class_functions (own & inherited)
    assert_db_uses_index(
        db,
        "SELECT owner || ':' || name, doc FROM ue_function WHERE owner = ?1 ORDER BY name ASC",
        ["/Script/mucha.CPP_TestChild"],
        REFLECTION_TABLES,
    );
    assert_db_uses_index(
        db,
        "WITH RECURSIVE chain(path, super_class, depth) AS (\
            SELECT path, super_class, 0 FROM ue_class WHERE path = ?1 \
            UNION ALL \
            SELECT ue_class.path, ue_class.super_class, chain.depth + 1 \
            FROM chain CROSS JOIN ue_class ON ue_class.path = chain.super_class \
            WHERE chain.super_class IS NOT NULL AND chain.depth < 64\
         ) SELECT ue_function.owner || ':' || ue_function.name, ue_function.doc \
           FROM chain CROSS JOIN ue_function ON ue_function.owner = chain.path \
           ORDER BY chain.depth ASC, ue_function.name ASC",
        ["/Script/mucha.CPP_TestGrandChild"],
        REFLECTION_TABLES,
    );
}

#[test]
fn cli_build_accepts_repeated_jmap_flags() {
    let bin = env!("CARGO_BIN_EXE_ffr-db");
    let fixture_dir = fixture_area_dir("reflection");
    let jmap1 = fixture_dir.join("01-base.jmap");
    let jmap2 = fixture_dir.join("02-override.jmap");

    let temp_dir = tempfile::tempdir().expect("create tempdir");
    let db_path = temp_dir.path().join("cli_reflection.sqlite");

    let output = Command::new(bin)
        .env_remove("FFR_DEPS")
        .args(["build", "--stages", "reflection", "--jmap"])
        .arg(&jmap1)
        .arg("--jmap")
        .arg(&jmap2)
        .arg("--out")
        .arg(&db_path)
        .output()
        .expect("run ffr-db build with repeated --jmap");

    assert!(
        output.status.success(),
        "cli build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = GameDb::open(&db_path).expect("open cli-built reflection db");
    let resolved = db
        .ue_class("CPP_ConflictTarget")
        .expect("query CPP_ConflictTarget")
        .expect("CPP_ConflictTarget present");
    assert_eq!(resolved.size, 80);
    assert!(
        db.ue_class("CPP_TestGrandChild")
            .expect("query CPP_TestGrandChild")
            .is_some()
    );
}

/// Optional real-data verification against `.deps/r1/` `.jmap` dumps; auto-skips if absent.
#[test]
fn real_jmap_merge_and_spot_checks_if_available() {
    let Some((title_jmap, battle_jmap)) = find_real_jmaps() else {
        return;
    };

    let test_db = build_temp_db(
        "reflection_real",
        BuildInputs {
            jmaps: vec![title_jmap, battle_jmap],
            ..BuildInputs::default()
        },
        Some(vec![Stage::Reflection]),
    )
    .expect("build db from real jmaps");
    let db = test_db.db();

    // Spot check 1: CPP_UI_BattleSubsystem inheritance chain & key properties.
    let chain = db
        .class_chain("CPP_UI_BattleSubsystem")
        .expect("query CPP_UI_BattleSubsystem chain");
    let chain_names: Vec<&str> = chain.iter().map(|c| c.short_name.as_str()).collect();
    assert_eq!(
        chain_names,
        vec![
            "CPP_UI_BattleSubsystem",
            "WorldSubsystem",
            "Subsystem",
            "Object",
        ]
    );

    let props = db
        .class_properties("CPP_UI_BattleSubsystem", true)
        .expect("query CPP_UI_BattleSubsystem properties");
    let by_name: std::collections::BTreeMap<&str, u32> =
        props.iter().map(|p| (p.name.as_str(), p.offset)).collect();
    assert_eq!(by_name.get("BtlWidget"), Some(&64));
    assert_eq!(by_name.get("ResultWidget"), Some(&72));
    assert_eq!(by_name.get("BtlPhaseType"), Some(&80));
    assert_eq!(by_name.get("CurrentActionUnitUniqueId"), Some(&84));
    assert_eq!(by_name.get("FocusSkillId"), Some(&88));
    assert_eq!(by_name.get("BtlCmdManager"), Some(&112));
    assert_eq!(by_name.get("BtlUnitManager"), Some(&120));
    assert_eq!(by_name.get("bDuringBattle"), Some(&128));

    // Spot check 2: eElement values.
    let element_enum = db
        .ue_enum("eElement")
        .expect("query eElement")
        .expect("eElement must exist");
    assert_eq!(element_enum.enum_values.len(), 11);
    assert_eq!(
        db.enum_value("eElement", "eElement::Ice")
            .expect("eElement::Ice"),
        Some(2)
    );
    assert_eq!(db.enum_value("eElement", "Fire").expect("Fire"), Some(1));
    assert_eq!(
        db.enum_name("eElement", 2).expect("enum_name(2)"),
        Some("eElement::Ice".to_owned())
    );

    // Spot check 3: known real-world ambiguous short name reports candidates.
    let dup_err = db
        .ue_class("WBP_VisionStatus_AwakeningLevel_C")
        .expect_err("ambiguous short name in real jmap should error");
    assert!(
        dup_err
            .to_string()
            .contains("WBP_VisionStatus_AwakeningLevel")
    );
}

fn find_real_jmaps() -> Option<(PathBuf, PathBuf)> {
    let deps_root = std::env::var_os("FFR_DEPS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.deps"));

    let title = first_jmap_in(&deps_root.join("r1/20261124-125820-title-jmap"))?;
    let battle = first_jmap_in(&deps_root.join("r1/20261125-battle-jmap"))?;
    Some((title, battle))
}

fn first_jmap_in(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("jmap"))
        })
        .collect();
    files.sort();
    files.into_iter().next()
}
