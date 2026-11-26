//! Integration tests for the `ffr-db` skeleton: bundled SQLite extensions, builder pipeline,
//! schema version checks, read-only guards, connection pool concurrency, document record helpers,
//! `assert_uses_index`, fixture DB helper, and the `ffr-db` CLI.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;

use ffr_db::testing::{
    assert_db_uses_index, assert_uses_index, build_fixture_db, check_uses_index,
};
use ffr_db::{
    BuildInputs, BuildOptions, DbError, GameDb, MAX_POOL_CONNECTIONS, META_KEY_BUILDER_VERSION,
    META_KEY_GAME_BUILD_ID, META_KEY_SCHEMA_VERSION, ResolvedBuildPaths, SCHEMA_PARTS,
    SCHEMA_VERSION, Stage, StageStatus, build, record,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[test]
fn bundled_sqlite_supports_fts5_trigram_rtree_and_json() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    // 1. FTS5 with trigram tokenizer (CJK + English substring matching).
    conn.execute_batch(
        "CREATE VIRTUAL TABLE demo_fts USING fts5(lang, body, tokenize='trigram');
         INSERT INTO demo_fts (lang, body) VALUES
           ('zh-Hans', '洞窟深处的钢铁蝙蝠'),
           ('ja', 'ファイナルファンタジー'),
           ('en', 'Steel Bat Guardian');",
    )
    .expect("create and populate FTS5 trigram table");

    let zh_hits: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT body FROM demo_fts WHERE demo_fts MATCH '\"钢铁蝙\"'")
            .expect("prepare FTS5 zh query");
        stmt.query_map([], |row| row.get(0))
            .expect("run FTS5 zh query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect FTS5 zh hits")
    };
    assert_eq!(zh_hits, vec!["洞窟深处的钢铁蝙蝠"]);

    let ja_hits: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT body FROM demo_fts WHERE demo_fts MATCH '\"ナルフ\"'")
            .expect("prepare FTS5 ja query");
        stmt.query_map([], |row| row.get(0))
            .expect("run FTS5 ja query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect FTS5 ja hits")
    };
    assert_eq!(ja_hits, vec!["ファイナルファンタジー"]);

    assert_uses_index(
        &conn,
        "SELECT body FROM demo_fts WHERE demo_fts MATCH '\"钢铁蝙\"'",
        [],
        &["demo_fts"],
    );

    // 2. R*Tree 3D spatial index.
    conn.execute_batch(
        "CREATE VIRTUAL TABLE demo_rtree USING rtree(
             id, min_x, max_x, min_y, max_y, min_z, max_z
         );
         INSERT INTO demo_rtree VALUES
           (1, 100.0, 100.0, 200.0, 200.0, 0.0, 0.0),
           (2, 5000.0, 5000.0, 5000.0, 5000.0, 0.0, 0.0);",
    )
    .expect("create and populate R*Tree table");

    let near_ids: Vec<i64> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM demo_rtree \
                 WHERE max_x >= ?1 AND min_x <= ?2 AND max_y >= ?3 AND min_y <= ?4",
            )
            .expect("prepare R*Tree query");
        stmt.query_map(params![0.0_f64, 300.0_f64, 0.0_f64, 300.0_f64], |row| {
            row.get(0)
        })
        .expect("run R*Tree query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect R*Tree hits")
    };
    assert_eq!(near_ids, vec![1]);
    assert_uses_index(
        &conn,
        "SELECT id FROM demo_rtree WHERE max_x >= ?1 AND min_x <= ?2",
        params![0.0_f64, 300.0_f64],
        &["demo_rtree"],
    );

    // 3. Built-in JSON functions (json_valid, json_extract, json_array_length, json_each).
    let (valid, name, skill_count): (i64, String, i64) = conn
        .query_row(
            "SELECT \
                json_valid(?1), \
                json_extract(?1, '$.name'), \
                json_array_length(?1, '$.skills')",
            [r#"{"name":"钢蝠","skills":[1001,1002,1003]}"#],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("execute SQLite JSON functions");
    assert_eq!(valid, 1);
    assert_eq!(name, "钢蝠");
    assert_eq!(skill_count, 3);

    let skill_sum: i64 = conn
        .query_row(
            "SELECT SUM(value) FROM json_each(?1, '$.skills')",
            [r#"{"skills":[10,20,30]}"#],
            |row| row.get(0),
        )
        .expect("execute json_each");
    assert_eq!(skill_sum, 60);
}

#[test]
fn skeleton_build_on_empty_inputs_marks_all_stages_skipped() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let out_path = dir.path().join("nested").join("ffr.sqlite");

    // Case A: no input dirs provided at all.
    let report = build(&BuildOptions {
        inputs: BuildInputs::default(),
        output: out_path.clone(),
        stages: None,
        game_build_id: None,
        threads: None,
    })
    .expect("build with empty inputs");

    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(report.stages.len(), Stage::ALL.len());
    assert_eq!(report.total_issues, 0);
    assert_eq!(report.file_sha256.len(), 64);
    assert_eq!(report.content_sha256.len(), 64);
    assert!(report.manifest_path.is_file());
    for stage in &report.stages {
        assert_eq!(stage.status, StageStatus::Skipped);
        assert_eq!(stage.rows, 0);
        assert!(
            !stage.note.is_empty(),
            "skipped stage {} must include reason",
            stage.stage
        );
    }

    let db = GameDb::open(&out_path).expect("open built db");
    let meta = db.meta().expect("read meta");
    let expected_schema_ver = SCHEMA_VERSION.to_string();
    assert_eq!(
        meta.get(META_KEY_SCHEMA_VERSION).map(String::as_str),
        Some(expected_schema_ver.as_str())
    );
    assert_eq!(
        meta.get(META_KEY_BUILDER_VERSION).map(String::as_str),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(
        !meta.contains_key("built_at"),
        "volatile built_at must not be stored in SQLite meta table"
    );
    assert_eq!(
        meta.get(META_KEY_GAME_BUILD_ID).map(String::as_str),
        Some("unknown")
    );

    let stored_reports = db.stage_reports().expect("read stage_report");
    assert_eq!(stored_reports.len(), Stage::ALL.len());
    assert!(
        stored_reports
            .iter()
            .all(|r| r.status == StageStatus::Skipped)
    );
    // Release the read-only handle before rebuilding into the same path: Windows refuses to
    // replace a file that is still open (os error 5), unlike Linux.
    drop(db);

    // Case B: fake empty directories exist for export/full/inventory/maps, while jmap/snapshots
    // remain omitted -> every stage skips cleanly with a non-empty Chinese reason.
    let export = dir.path().join("export");
    let full = dir.path().join("export-full");
    let inventory = dir.path().join("inventory");
    let maps = full.join("maps");
    for path in [&export, &full, &inventory, &maps] {
        fs::create_dir_all(path).expect("create fake input dir");
    }

    let report2 = build(&BuildOptions {
        inputs: BuildInputs {
            export: Some(export),
            full: Some(full),
            inventory: Some(inventory),
            maps: Some(maps),
            jmaps: Vec::new(),
            snapshots: None,
        },
        output: out_path.clone(),
        stages: None,
        game_build_id: None,
        threads: None,
    })
    .expect("build with fake empty dirs");

    for stage in &report2.stages {
        assert_eq!(stage.status, StageStatus::Skipped, "stage={}", stage.stage);
        assert!(
            !stage.note.is_empty(),
            "skipped stage {} must explain why",
            stage.stage
        );
        if stage.stage == "reflection" {
            assert!(stage.note.contains("--jmap"), "note={}", stage.note);
        } else if stage.stage == "snapshots" {
            assert!(stage.note.contains("--snapshots"), "note={}", stage.note);
        }
    }

    // Case C: input directory exists with partial/incomplete files -> fails with Chinese error.
    let incomplete_maps = dir.path().join("incomplete-maps");
    fs::create_dir_all(&incomplete_maps).expect("create incomplete maps dir");
    fs::write(incomplete_maps.join("OnlyOneLevel.json"), "{}").expect("write partial level json");
    let maps_err = build(&BuildOptions {
        inputs: BuildInputs {
            maps: Some(incomplete_maps),
            ..BuildInputs::default()
        },
        output: dir.path().join("fail_maps.sqlite"),
        stages: Some(vec![Stage::Levels]),
        game_build_id: None,
        threads: None,
    })
    .expect_err("incomplete maps dir without _graph.json must fail");
    let maps_msg = maps_err.to_string();
    assert!(
        maps_msg.contains("缺少 _graph.json") && maps_msg.contains("levels"),
        "expected Chinese error for incomplete maps dir, got: {maps_msg}"
    );

    let incomplete_export = dir.path().join("incomplete-export");
    let map_dt_dir = incomplete_export
        .join("FFRS")
        .join("Content")
        .join("Datatable")
        .join("Map");
    fs::create_dir_all(&map_dt_dir).expect("create incomplete export dir");
    fs::write(
        map_dt_dir.join("CDT_MapData_Prod.json"),
        r#"[{"Type":"CompositeDataTable","Name":"CDT_MapData_Prod","Package":"/Game/Datatable/Map/CDT_MapData_Prod","Properties":{"RowStruct":{"ObjectName":"Class'MAP_DATA'"}},"Rows":{}}]"#,
    )
    .expect("write minimal CDT_MapData_Prod.json");
    let world_err = build(&BuildOptions {
        inputs: BuildInputs {
            export: Some(incomplete_export),
            ..BuildInputs::default()
        },
        output: dir.path().join("fail_world.sqlite"),
        stages: Some(vec![Stage::World]),
        game_build_id: None,
        threads: None,
    })
    .expect_err("incomplete export dir missing required world tables must fail");
    let world_msg = world_err.to_string();
    assert!(
        world_msg.contains("world") && world_msg.contains("失败"),
        "expected Chinese error for incomplete world export, got: {world_msg}"
    );

    // Verify `--stages` filters stages properly.
    let report3 = build(&BuildOptions {
        inputs: BuildInputs::default(),
        output: out_path,
        stages: Some(vec![Stage::Text, Stage::Battle]),
        game_build_id: None,
        threads: None,
    })
    .expect("build with --stages filter");
    for stage in &report3.stages {
        assert_eq!(stage.status, StageStatus::Skipped);
        if stage.stage == "text" || stage.stage == "battle" {
            assert!(stage.note.contains("--export"), "note={}", stage.note);
        } else {
            assert!(stage.note.contains("--stages"), "note={}", stage.note);
        }
    }
}

#[test]
fn open_rejects_mismatched_or_missing_schema_version() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let db_path = dir.path().join("version_test.sqlite");

    build(&BuildOptions {
        inputs: BuildInputs::default(),
        output: db_path.clone(),
        stages: None,
        game_build_id: None,
        threads: None,
    })
    .expect("build base db");

    // Mutate schema_version to an incompatible value.
    {
        let conn = Connection::open(&db_path).expect("open writable");
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            params![(SCHEMA_VERSION + 99).to_string(), META_KEY_SCHEMA_VERSION],
        )
        .expect("update schema_version");
    }

    let err = GameDb::open(&db_path).expect_err("version mismatch must fail");
    match err {
        DbError::SchemaVersionMismatch {
            expected, actual, ..
        } => {
            assert_eq!(expected, SCHEMA_VERSION);
            assert_eq!(actual, SCHEMA_VERSION + 99);
        }
        other => panic!("expected SchemaVersionMismatch, got: {other:?}"),
    }
    assert!(err.to_string().contains("重新构建"));

    // Delete schema_version row -> MissingSchemaVersion.
    {
        let conn = Connection::open(&db_path).expect("open writable");
        conn.execute("DELETE FROM meta WHERE key = ?1", [META_KEY_SCHEMA_VERSION])
            .expect("delete schema_version");
    }
    let err2 = GameDb::open(&db_path).expect_err("missing schema_version must fail");
    assert!(
        matches!(err2, DbError::MissingSchemaVersion { .. }),
        "unexpected error: {err2:?}"
    );
}

#[test]
fn readonly_sql_rejects_write_and_multi_statements() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let db_path = dir.path().join("ro.sqlite");
    build(&BuildOptions {
        inputs: BuildInputs::default(),
        output: db_path.clone(),
        stages: None,
        game_build_id: None,
        threads: None,
    })
    .expect("build db");

    let db = GameDb::open(&db_path).expect("open db");

    // Valid SELECT succeeds.
    let rows = db
        .query_readonly_json("SELECT key, value FROM meta WHERE key = 'schema_version'")
        .expect("select meta");
    assert_eq!(rows.len(), 1);
    let expected_schema_ver = SCHEMA_VERSION.to_string();
    assert_eq!(
        rows[0].get("value").and_then(|v| v.as_str()),
        Some(expected_schema_ver.as_str())
    );

    // Write / DDL / PRAGMA / CTE-write / multi-statement queries must all be rejected.
    let rejected = [
        "INSERT INTO meta (key, value) VALUES ('evil', '1')",
        "UPDATE meta SET value = '999' WHERE key = 'schema_version'",
        "DELETE FROM meta",
        "DROP TABLE meta",
        "CREATE TABLE hacked (x INT)",
        "PRAGMA query_only = OFF",
        "WITH cte AS (SELECT 1) DELETE FROM meta",
        "SELECT 1; DROP TABLE meta",
        "   ",
    ];
    for sql in rejected {
        let err = db
            .query_readonly_json(sql)
            .expect_err(&format!("expected rejection for SQL: {sql}"));
        assert!(
            matches!(err, DbError::WriteQueryRejected { .. }),
            "expected WriteQueryRejected for {sql}, got {err:?}"
        );
    }

    // Even direct `with_conn` must fail writes because of SQLITE_OPEN_READ_ONLY + query_only=ON.
    let direct_write = db.with_conn(|conn| {
        conn.execute("INSERT INTO meta (key, value) VALUES ('x', 'y')", [])
            .map_err(|e| DbError::Sqlite {
                action: "direct write".to_owned(),
                source: e,
            })
    });
    assert!(direct_write.is_err(), "read-only connection allowed write!");
}

#[test]
fn query_readonly_limited_enforces_row_cap_timeout_and_write_rejection() {
    use std::time::Duration;

    let dir = tempfile::tempdir().expect("create tempdir");
    let db_path = dir.path().join("limited.sqlite");
    build(&BuildOptions {
        inputs: BuildInputs::default(),
        output: db_path.clone(),
        stages: None,
        game_build_id: None,
        threads: None,
    })
    .expect("build db");

    let db = GameDb::open(&db_path).expect("open db");

    // 1. Normal query under row cap.
    let res = db
        .query_readonly_limited(
            "SELECT key, value FROM meta WHERE key = 'schema_version'",
            10,
            Duration::from_secs(2),
        )
        .expect("bounded select meta");
    assert_eq!(res.columns, vec!["key", "value"]);
    assert_eq!(res.rows.len(), 1);
    assert!(!res.truncated_rows);

    // 2. Row-limit truncation (`max_rows = 5` on a 20-row recursive CTE).
    let capped = db
        .query_readonly_limited(
            "WITH RECURSIVE cnt(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM cnt WHERE n < 20) \
             SELECT n FROM cnt",
            5,
            Duration::from_secs(2),
        )
        .expect("row-capped recursive CTE");
    assert_eq!(capped.rows.len(), 5);
    assert!(capped.truncated_rows);
    assert_eq!(capped.rows[0].get("n").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(capped.rows[4].get("n").and_then(|v| v.as_i64()), Some(5));

    // 3. Progress-handler timeout interrupts a runaway recursive CTE.
    let timeout_err = db
        .query_readonly_limited(
            "WITH RECURSIVE cnt(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM cnt) \
             SELECT COUNT(*) FROM cnt",
            10,
            Duration::from_millis(50),
        )
        .expect_err("infinite CTE must time out");
    assert!(
        ffr_db::db::is_query_timeout_error(&timeout_err),
        "expected timeout error, got: {timeout_err:?}"
    );
    assert!(
        timeout_err.to_string().contains("超时"),
        "expected Chinese timeout message, got: {timeout_err}"
    );

    // 4. Pooled connection stays clean (progress handler unregistered) after a timeout.
    let after = db
        .query_readonly_limited("SELECT 42 AS answer", 10, Duration::from_secs(2))
        .expect("subsequent query succeeds after timeout");
    assert_eq!(
        after.rows[0].get("answer").and_then(|v| v.as_i64()),
        Some(42)
    );

    // 5. Rejects writes and multi-statements.
    for bad in ["DELETE FROM meta", "SELECT 1; SELECT 2"] {
        let err = db
            .query_readonly_limited(bad, 10, Duration::from_secs(2))
            .expect_err("bad sql must fail");
        assert!(
            matches!(err, DbError::WriteQueryRejected { .. }),
            "unexpected error for {bad}: {err:?}"
        );
    }
}

#[test]
fn assert_uses_index_passes_for_indexed_and_fails_for_full_scan() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(
        "CREATE TABLE big_units (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            unindexed_note TEXT NOT NULL
         );
         CREATE INDEX idx_big_units_name ON big_units(name);
         INSERT INTO big_units VALUES (1, '钢蝠', 'cave'), (2, '陆行鸟', 'field');",
    )
    .expect("create test table");

    // Primary key lookup -> passes.
    assert_uses_index(
        &conn,
        "SELECT * FROM big_units WHERE id = ?1",
        [1_i64],
        &["big_units"],
    );

    // Secondary index lookup -> passes.
    assert_uses_index(
        &conn,
        "SELECT * FROM big_units WHERE name = ?1",
        ["钢蝠"],
        &["big_units"],
    );

    // Full table scan -> check_uses_index returns Err(FullTableScan).
    let err = check_uses_index(
        &conn,
        "SELECT * FROM big_units WHERE unindexed_note = ?1",
        ["cave"],
        &["big_units"],
    )
    .expect_err("full table scan must be rejected");
    assert!(
        matches!(err, DbError::FullTableScan { ref table, .. } if table == "big_units"),
        "unexpected error: {err:?}"
    );

    // Also verify build_issue index on a real built DB via GameDb.
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("idx.sqlite");
    build(&BuildOptions {
        inputs: BuildInputs::default(),
        output: db_path.clone(),
        stages: None,
        game_build_id: None,
        threads: None,
    })
    .expect("build db");
    let db = GameDb::open(&db_path).expect("open db");
    assert_db_uses_index(
        &db,
        "SELECT stage, kind, subject, detail FROM build_issue WHERE stage = ?1 AND kind = ?2",
        ["battle", "dangling_ref"],
        &["build_issue"],
    );
}

#[test]
#[should_panic(expected = "全表扫描")]
fn assert_uses_index_panics_on_full_table_scan() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(
        "CREATE TABLE big_units (
            id INTEGER PRIMARY KEY,
            unindexed_note TEXT NOT NULL
         );",
    )
    .expect("create test table");
    assert_uses_index(
        &conn,
        "SELECT * FROM big_units WHERE unindexed_note = ?1",
        ["cave"],
        &["big_units"],
    );
}

#[test]
fn game_db_is_send_sync_and_pool_caps_at_four_connections() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GameDb>();

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("pool.sqlite");
    build(&BuildOptions {
        inputs: BuildInputs::default(),
        output: db_path.clone(),
        stages: None,
        game_build_id: None,
        threads: None,
    })
    .expect("build db");

    let db = GameDb::open(&db_path).expect("open db");
    assert_eq!(db.pool_size(), 1);

    // Spawn 8 threads synchronizing on a 4-thread wave so all 4 pool slots are saturated
    // simultaneously while remaining threads queue cleanly on the condvar.
    let thread_count = 8;
    let start_barrier = Arc::new(Barrier::new(thread_count));
    let mut handles = Vec::new();
    for _ in 0..thread_count {
        let db_clone = db.clone();
        let barrier = Arc::clone(&start_barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            db_clone
                .with_conn(|conn| {
                    // Hold the connection briefly so concurrent threads force pool expansion.
                    thread::sleep(std::time::Duration::from_millis(15));
                    let v: String = conn
                        .prepare_cached("SELECT value FROM meta WHERE key = 'schema_version'")
                        .expect("prepare cached")
                        .query_row([], |row| row.get(0))
                        .expect("query schema_version");
                    assert_eq!(v, SCHEMA_VERSION.to_string());
                    Ok(())
                })
                .expect("concurrent query");
        }));
    }
    for handle in handles {
        handle.join().expect("thread finished");
    }

    assert!(
        db.pool_size() <= MAX_POOL_CONNECTIONS,
        "pool_size {} exceeded MAX_POOL_CONNECTIONS {}",
        db.pool_size(),
        MAX_POOL_CONNECTIONS
    );
}

#[test]
fn record_helpers_round_trip_and_report_table_and_key_on_corrupt_doc() {
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct SampleUnit {
        id: i64,
        name: String,
    }

    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch("CREATE TABLE sample_unit (id INTEGER PRIMARY KEY, doc TEXT NOT NULL);")
        .expect("create sample_unit");

    let u1 = SampleUnit {
        id: 101,
        name: "钢蝠".to_owned(),
    };
    let doc1 = record::encode_doc("sample_unit", u1.id, &u1).expect("encode u1");
    conn.execute(
        "INSERT INTO sample_unit (id, doc) VALUES (?1, ?2)",
        params![u1.id, doc1],
    )
    .expect("insert u1");

    let fetched: Option<SampleUnit> = record::query_doc_opt(
        &conn,
        "sample_unit",
        101,
        "SELECT doc FROM sample_unit WHERE id = ?1",
        [101_i64],
    )
    .expect("query_doc_opt");
    assert_eq!(fetched, Some(u1));

    let missing: Option<SampleUnit> = record::query_doc_opt(
        &conn,
        "sample_unit",
        999,
        "SELECT doc FROM sample_unit WHERE id = ?1",
        [999_i64],
    )
    .expect("query_doc_opt missing");
    assert_eq!(missing, None);

    // Insert a corrupted JSON document and verify error mentions both table and primary key.
    conn.execute(
        "INSERT INTO sample_unit (id, doc) VALUES (?1, ?2)",
        params![202_i64, "{not valid json"],
    )
    .expect("insert corrupt doc");

    let err = record::query_docs::<SampleUnit, _>(
        &conn,
        "sample_unit",
        "SELECT id, doc FROM sample_unit ORDER BY id",
        [],
    )
    .expect_err("corrupt doc must error");
    match err {
        DbError::DocDeserialize { table, key, .. } => {
            assert_eq!(table, "sample_unit");
            assert_eq!(key, "202");
        }
        other => panic!("expected DocDeserialize, got: {other:?}"),
    }
}

#[test]
fn resolved_paths_follow_ffr_deps_and_require_explicit_jmap_and_snapshots() {
    let deps = PathBuf::from("/fake/deps");
    let resolved =
        ResolvedBuildPaths::resolve(BuildInputs::default(), None, Some(OsString::from(&deps)))
            .expect("resolve defaults");

    assert_eq!(resolved.inputs.export, Some(deps.join("ffr/export")));
    assert_eq!(resolved.inputs.full, Some(deps.join("ffr/export-full")));
    assert_eq!(resolved.inputs.inventory, Some(deps.join("ffr/inventory")));
    assert_eq!(
        resolved.inputs.maps,
        Some(deps.join("ffr/export-full/maps"))
    );
    assert!(resolved.inputs.jmaps.is_empty());
    assert_eq!(resolved.inputs.snapshots, None);
    assert_eq!(resolved.output, deps.join("ffr/db/ffr.sqlite"));

    // Without FFR_DEPS and without --out -> MissingEnvVar.
    let err = ResolvedBuildPaths::resolve(BuildInputs::default(), None, None)
        .expect_err("missing FFR_DEPS and --out must error");
    assert!(matches!(err, DbError::MissingEnvVar { var: "FFR_DEPS" }));

    // Verify all 12 schema parts exist in exact expected order.
    let names: Vec<&str> = SCHEMA_PARTS.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        vec![
            "00_meta",
            "01_text",
            "02_datatable",
            "03_battle",
            "04_world",
            "05_assets",
            "06_reflection",
            "07_input_ui",
            "08_levels",
            "09_sequences",
            "10_snapshots",
            "11_links",
        ]
    );
}

#[test]
fn fixture_helper_builds_temp_db_from_repo_fixture_area() {
    let test_db = build_fixture_db("text").expect("build fixture db for text area");
    let reports = test_db.db().stage_reports().expect("read stage_report");
    assert_eq!(reports.len(), Stage::ALL.len());
    let text = reports
        .iter()
        .find(|r| r.stage == "text")
        .expect("text stage report");
    assert_eq!(text.status, StageStatus::Ok);
    assert!(text.rows > 0);
    for skipped_stage in [
        "battle",
        "world",
        "assets",
        "reflection",
        "input_ui",
        "levels",
        "sequences",
        "snapshots",
        "links",
    ] {
        let rep = reports
            .iter()
            .find(|r| r.stage == skipped_stage)
            .expect("stage report exists");
        assert_eq!(
            rep.status,
            StageStatus::Skipped,
            "stage {skipped_stage} should skip when input missing"
        );
    }
}

#[test]
fn cli_build_stats_sql_and_explain_end_to_end() {
    let bin = env!("CARGO_BIN_EXE_ffr-db");
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("cli.sqlite");

    // 1. build
    let build_out = Command::new(bin)
        .env_remove("FFR_DEPS")
        .args(["build", "--out"])
        .arg(&db_path)
        .output()
        .expect("run ffr-db build");
    assert!(
        build_out.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build_out.stderr)
    );
    let build_stdout = String::from_utf8_lossy(&build_out.stdout);
    assert!(build_stdout.contains("构建完成"), "stdout={build_stdout}");
    assert!(
        build_stdout.contains("build_issue"),
        "stdout={build_stdout}"
    );

    // 2. stats
    let stats_out = Command::new(bin)
        .env_remove("FFR_DEPS")
        .args(["stats", "--db"])
        .arg(&db_path)
        .output()
        .expect("run ffr-db stats");
    assert!(
        stats_out.status.success(),
        "stats failed: {}",
        String::from_utf8_lossy(&stats_out.stderr)
    );
    let stats_stdout = String::from_utf8_lossy(&stats_out.stdout);
    assert!(stats_stdout.contains(&format!("schema_version = {SCHEMA_VERSION}")));
    assert!(stats_stdout.contains("stage_report"));

    // 3. sql (NDJSON output)
    let sql_out = Command::new(bin)
        .env_remove("FFR_DEPS")
        .args(["sql", "--db"])
        .arg(&db_path)
        .arg("SELECT stage, status FROM stage_report ORDER BY rowid")
        .output()
        .expect("run ffr-db sql");
    assert!(
        sql_out.status.success(),
        "sql failed: {}",
        String::from_utf8_lossy(&sql_out.stderr)
    );
    let lines: Vec<&str> = std::str::from_utf8(&sql_out.stdout)
        .expect("utf8 stdout")
        .lines()
        .collect();
    assert_eq!(lines.len(), Stage::ALL.len());
    for line in lines {
        let obj: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
        assert_eq!(obj["status"], "skipped");
    }

    // 4. sql rejects write statements
    let write_out = Command::new(bin)
        .env_remove("FFR_DEPS")
        .args(["sql", "--db"])
        .arg(&db_path)
        .arg("DELETE FROM meta")
        .output()
        .expect("run ffr-db sql DELETE");
    assert!(
        !write_out.status.success(),
        "write SQL should exit non-zero"
    );
    let write_stderr = String::from_utf8_lossy(&write_out.stderr);
    assert!(
        write_stderr.contains("只读查询拒绝"),
        "stderr={write_stderr}"
    );

    // 5. explain
    let explain_out = Command::new(bin)
        .env_remove("FFR_DEPS")
        .args(["explain", "--db"])
        .arg(&db_path)
        .arg("SELECT * FROM build_issue WHERE stage = 'battle'")
        .output()
        .expect("run ffr-db explain");
    assert!(
        explain_out.status.success(),
        "explain failed: {}",
        String::from_utf8_lossy(&explain_out.stderr)
    );
    let explain_stdout = String::from_utf8_lossy(&explain_out.stdout);
    assert!(
        explain_stdout.contains("idx_build_issue_stage_kind"),
        "expected index in explain output, got: {explain_stdout}"
    );
}
