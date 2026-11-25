//! Integration tests for the optional live-game snapshot area (`10_snapshots`).
//!
//! The fixture under `tests/fixtures/rust/db/snapshots/` contains three normal snapshots (one of them
//! unfinished and missing a file), one hotkey snapshot, and both marker logs.

use std::path::Path;

use ffr_db::testing::{assert_uses_index, build_temp_db, fixture_area_dir};
use ffr_db::{BuildInputs, GameDb, Stage, StageStatus};

const LABEL_SQL: &str = "SELECT id, doc FROM snapshot WHERE label = ?1 ORDER BY time, id";
const OBJECTS_SQL: &str = "SELECT id, doc FROM snapshot_object WHERE snapshot_id = ?1 AND class_short = ?2 ORDER BY name, id";
const FIELD_HISTORY_SQL: &str = "SELECT s.id, s.label, s.time, o.name, \
        json_quote(json_extract(o.doc, ?2)), json_type(o.doc, ?2) \
     FROM snapshot_object o JOIN snapshot s ON s.id = o.snapshot_id \
     WHERE o.class_short = ?1 \
     ORDER BY s.time, s.id, o.name";
const WITH_CLASS_SQL: &str = "SELECT s.id, s.doc \
     FROM (SELECT DISTINCT snapshot_id FROM snapshot_object WHERE class_short = ?1) hits \
     JOIN snapshot s ON s.id = hits.snapshot_id \
     ORDER BY s.time, s.id";
const TEXTS_SQL: &str = "SELECT id, doc FROM snapshot_text WHERE snapshot_id = ?1 ORDER BY id";

fn build_fixture() -> ffr_db::testing::TestDb {
    let inputs = BuildInputs {
        snapshots: Some(fixture_area_dir("snapshots")),
        ..BuildInputs::default()
    };
    build_temp_db("snapshots", inputs, Some(vec![Stage::Snapshots]))
        .expect("build snapshot fixture database")
}

#[test]
fn imports_complete_partial_and_hotkey_snapshots_with_issues() {
    let test_db = build_fixture();
    let report = test_db.report();
    let stage = report
        .stages
        .iter()
        .find(|stage| stage.stage == "snapshots")
        .expect("snapshots stage report");
    assert_eq!(stage.status, StageStatus::Ok);
    assert_eq!(
        stage.rows, 19,
        "4 snapshots + 6 objects + 4 widgets + 3 texts + 2 markers"
    );
    assert!(stage.note.contains("快照 4 份"), "note={}", stage.note);

    let kinds: Vec<(&str, u64)> = report
        .issues
        .iter()
        .map(|issue| (issue.kind.as_str(), issue.count))
        .collect();
    assert!(
        kinds.contains(&("incomplete_snapshot", 1)),
        "kinds={kinds:?}"
    );
    assert!(kinds.contains(&("missing_file", 1)), "kinds={kinds:?}");
    assert!(kinds.contains(&("invalid_line", 1)), "kinds={kinds:?}");

    let db = test_db.db();
    let incomplete = db
        .snapshots(Some("field-idle"))
        .expect("query incomplete snapshot");
    assert_eq!(incomplete.len(), 1);
    assert!(!incomplete[0].completed);
    assert_eq!(incomplete[0].status, "incomplete");
    assert_eq!(
        incomplete[0].world.as_deref(),
        Some("World /Game/Map/Field/Field_PL.Field_PL")
    );
    assert!(
        incomplete[0]
            .issues
            .iter()
            .any(|issue| issue.contains("missing_file")),
        "issues={:?}",
        incomplete[0].issues
    );

    let hotkey = db
        .snapshots(Some("hotkey-0001"))
        .expect("query hotkey snapshot");
    assert_eq!(hotkey.len(), 1);
    assert!(hotkey[0].hotkey);
    assert!(hotkey[0].completed);

    let marker_rows = db
        .query_readonly_json("SELECT COUNT(*) AS n FROM marker")
        .expect("count markers");
    assert_eq!(marker_rows[0]["n"].as_i64(), Some(2));
}

#[test]
fn snapshots_filters_by_label_and_uses_index() {
    let test_db = build_fixture();
    let db = test_db.db();

    assert_eq!(db.snapshots(None).expect("all snapshots").len(), 4);
    let hits = db.snapshots(Some("battle-command")).expect("label query");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].time, Some(1_767_229_261));

    db.with_conn(|conn| {
        assert_uses_index(conn, LABEL_SQL, ["battle-command"], &["snapshot"]);
        Ok(())
    })
    .expect("label query plan");
}

#[test]
fn snapshot_objects_reads_one_class_and_uses_index() {
    let test_db = build_fixture();
    let db = test_db.db();

    let all = db
        .snapshot_objects("20260101-010101-battle-command-snapshot", None)
        .expect("all objects of snapshot");
    assert_eq!(all.len(), 2);

    let objects = db
        .snapshot_objects(
            "20260101-010101-battle-command-snapshot",
            Some("VM_BtlCmd_C"),
        )
        .expect("objects by class");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].class_short, "VM_BtlCmd_C");
    assert_eq!(
        objects[0].properties["FocusCmdData"]["Value"]["ActionKey"]["ID"],
        serde_json::json!(220030)
    );

    db.with_conn(|conn| {
        assert_uses_index(
            conn,
            OBJECTS_SQL,
            ["20260101-010101-battle-command-snapshot", "VM_BtlCmd_C"],
            &["snapshot_object"],
        );
        Ok(())
    })
    .expect("object query plan");
}

#[test]
fn field_history_lists_values_in_capture_order_and_uses_index() {
    let test_db = build_fixture();
    let db = test_db.db();

    let history = db
        .field_history("CPP_UI_BattleSubsystem", "$.bDuringBattle")
        .expect("field history");
    assert_eq!(history.len(), 4);
    let values: Vec<serde_json::Value> = history
        .iter()
        .map(|entry| entry.value.clone().expect("field present"))
        .collect();
    assert_eq!(
        values,
        vec![
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!(true),
        ]
    );
    let labels: Vec<&str> = history.iter().map(|entry| entry.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "battle-command",
            "battle-start",
            "field-idle",
            "hotkey-0001"
        ]
    );

    let missing = db
        .field_history("CPP_UI_BattleSubsystem", "$.NoSuchField")
        .expect("missing field history");
    assert_eq!(missing.len(), 4);
    assert!(missing.iter().all(|entry| entry.value.is_none()));

    db.with_conn(|conn| {
        assert_uses_index(
            conn,
            FIELD_HISTORY_SQL,
            ["CPP_UI_BattleSubsystem", "$.properties.bDuringBattle"],
            &["snapshot", "snapshot_object"],
        );
        Ok(())
    })
    .expect("field history query plan");
}

#[test]
fn snapshots_with_class_returns_each_snapshot_once_and_uses_index() {
    let test_db = build_fixture();
    let db = test_db.db();

    let hits = db
        .snapshots_with_class("VM_BtlCmd_C")
        .expect("snapshots with class");
    assert_eq!(hits.len(), 2);
    let labels: Vec<&str> = hits
        .iter()
        .map(|snapshot| snapshot.label.as_str())
        .collect();
    assert_eq!(labels, vec!["battle-command", "battle-start"]);

    db.with_conn(|conn| {
        assert_uses_index(
            conn,
            WITH_CLASS_SQL,
            ["VM_BtlCmd_C"],
            &["snapshot", "snapshot_object"],
        );
        Ok(())
    })
    .expect("snapshots-with-class query plan");
}

#[test]
fn texts_in_snapshot_reads_texts_and_uses_index() {
    let test_db = build_fixture();
    let db = test_db.db();

    let texts = db
        .texts_in_snapshot("20260101-010101-battle-command-snapshot")
        .expect("texts in snapshot");
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].text, "战斗指令");

    db.with_conn(|conn| {
        assert_uses_index(
            conn,
            TEXTS_SQL,
            ["20260101-010101-battle-command-snapshot"],
            &["snapshot_text"],
        );
        Ok(())
    })
    .expect("texts query plan");
}

/// Optional real `.deps/r1` spot checks.
///
/// Set `FFR_SNAPSHOT_REAL_DB=<prebuilt sqlite>` to run; otherwise this test prints a skip message
/// and succeeds, so machines without the real snapshot dump never fail.
#[test]
fn real_snapshot_db_spot_checks_when_configured() {
    let Some(path) = std::env::var_os("FFR_SNAPSHOT_REAL_DB") else {
        eprintln!("跳过真实快照抽查：未设置 FFR_SNAPSHOT_REAL_DB");
        return;
    };
    let path = Path::new(&path);
    if !path.is_file() {
        eprintln!("跳过真实快照抽查：{} 不存在", path.display());
        return;
    }
    let db = GameDb::open(path).expect("open real snapshot database");

    let history = db
        .field_history("CPP_UI_BattleSubsystem", "$.bDuringBattle")
        .expect("real field history");
    assert!(
        !history.is_empty(),
        "real database has no battle subsystem snapshots"
    );
    println!(
        "field_history(CPP_UI_BattleSubsystem, $.bDuringBattle): {} 条，首={:?}/{:?}，末={:?}/{:?}",
        history.len(),
        history.first().map(|e| e.label.as_str()),
        history.first().and_then(|e| e.value.clone()),
        history.last().map(|e| e.label.as_str()),
        history.last().and_then(|e| e.value.clone()),
    );

    let snapshots = db
        .snapshots(Some("battle-command"))
        .expect("real battle-command snapshot");
    if let Some(snapshot) = snapshots.first() {
        let objects = db
            .snapshot_objects(&snapshot.id, Some("VM_BtlCmd_C"))
            .expect("real VM_BtlCmd_C objects");
        let focus = objects
            .first()
            .and_then(|object| object.properties.get("FocusCmdData"));
        println!(
            "{} / VM_BtlCmd_C FocusCmdData = {}",
            snapshot.label,
            focus.map_or_else(|| "缺少".to_owned(), ToString::to_string)
        );
        assert!(
            focus.is_some(),
            "battle-command snapshot lacks VM_BtlCmd_C.FocusCmdData"
        );
    } else {
        panic!("real database has no battle-command snapshot");
    }

    let with_class = db
        .snapshots_with_class("CPP_UI_BattleSubsystem")
        .expect("real snapshots with class");
    assert!(!with_class.is_empty());
    println!(
        "snapshots_with_class(CPP_UI_BattleSubsystem) = {} 份",
        with_class.len()
    );
}
