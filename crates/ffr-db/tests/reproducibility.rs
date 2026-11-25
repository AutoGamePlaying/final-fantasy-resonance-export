//! Reproducibility, build manifest, and `ffr-db hash` integration tests.

use std::fs;
use std::path::Path;
use std::process::Command;

use ffr_db::testing::fixture_area_dir;
use ffr_db::{
    BuildInputs, BuildManifest, BuildOptions, DB_APPLICATION_ID, DB_PAGE_SIZE, DbHashReport,
    GameDb, HASH_FORMAT_VERSION, MANIFEST_FORMAT_VERSION, SCHEMA_VERSION, Stage, build,
    hash_database, manifest_path_for,
};
use rusqlite::{Connection, params};

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create destination dir");
    for entry in fs::read_dir(src).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("copy fixture file");
        }
    }
}

#[test]
fn reproducible_build_across_dirs_threads_and_timezones_produces_identical_hashes() {
    let bin = env!("CARGO_BIN_EXE_ffr-db");
    let work = tempfile::tempdir().expect("create temp workspace");

    // Prepare two completely separate input copies so even input directory absolute paths differ.
    let input_a = work.path().join("machine_alpha_inputs");
    let input_b = work.path().join("machine_beta_completely_different_root");

    // Combine `links` fixture (export, export-full, maps, jmap, snapshots) with `assets` fixture
    // (`inventory/registry.jsonl` + `export-full` JSON/INI) so parallel asset scanning runs too.
    let links_fixture = fixture_area_dir("links");
    let assets_fixture = fixture_area_dir("assets");
    for root in [&input_a, &input_b] {
        copy_dir_recursive(&links_fixture, root);
        copy_dir_recursive(&assets_fixture.join("inventory"), &root.join("inventory"));
        copy_dir_recursive(
            &assets_fixture.join("export-full"),
            &root.join("export-full"),
        );
    }

    let db_a = work.path().join("out_alpha/nested_a/game.sqlite");
    let db_b = work.path().join("out_beta/deep/nested_b/other_name.sqlite");

    // Build A: 1 thread, TZ=UTC, input_a -> db_a.
    let run_a = Command::new(bin)
        .env_remove("FFR_DEPS")
        .env("TZ", "UTC")
        .args(["build", "--threads", "1", "--export"])
        .arg(input_a.join("export"))
        .arg("--full")
        .arg(input_a.join("export-full"))
        .arg("--inventory")
        .arg(input_a.join("inventory"))
        .arg("--maps")
        .arg(input_a.join("maps"))
        .arg("--jmap")
        .arg(input_a.join("01-mock.jmap"))
        .arg("--snapshots")
        .arg(input_a.join("snapshots"))
        .arg("--out")
        .arg(&db_a)
        .output()
        .expect("run ffr-db build A");
    assert!(
        run_a.status.success(),
        "build A failed: {}",
        String::from_utf8_lossy(&run_a.stderr)
    );

    // Build B: 4 threads, TZ=Asia/Tokyo, input_b -> db_b.
    let run_b = Command::new(bin)
        .env_remove("FFR_DEPS")
        .env("TZ", "Asia/Tokyo")
        .args(["build", "--threads", "4", "--export"])
        .arg(input_b.join("export"))
        .arg("--full")
        .arg(input_b.join("export-full"))
        .arg("--inventory")
        .arg(input_b.join("inventory"))
        .arg("--maps")
        .arg(input_b.join("maps"))
        .arg("--jmap")
        .arg(input_b.join("01-mock.jmap"))
        .arg("--snapshots")
        .arg(input_b.join("snapshots"))
        .arg("--out")
        .arg(&db_b)
        .output()
        .expect("run ffr-db build B");
    assert!(
        run_b.status.success(),
        "build B failed: {}",
        String::from_utf8_lossy(&run_b.stderr)
    );

    // Whole SQLite files must be byte-for-byte identical.
    let bytes_a = fs::read(&db_a).expect("read db A");
    let bytes_b = fs::read(&db_b).expect("read db B");
    assert_eq!(
        bytes_a, bytes_b,
        "reproducible builds across different dirs/threads/TZ must yield identical bytes"
    );

    // Both CLI `ffr-db hash --json` outputs must match identically and conform to the schema.
    let hash_cmd_a = Command::new(bin)
        .env_remove("FFR_DEPS")
        .arg("hash")
        .arg(&db_a)
        .arg("--json")
        .output()
        .expect("run ffr-db hash A");
    assert!(
        hash_cmd_a.status.success(),
        "hash A failed: {}",
        String::from_utf8_lossy(&hash_cmd_a.stderr)
    );
    let hash_cmd_b = Command::new(bin)
        .env_remove("FFR_DEPS")
        .arg("hash")
        .arg(&db_b)
        .arg("--json")
        .output()
        .expect("run ffr-db hash B");
    assert!(
        hash_cmd_b.status.success(),
        "hash B failed: {}",
        String::from_utf8_lossy(&hash_cmd_b.stderr)
    );

    let json_a = String::from_utf8(hash_cmd_a.stdout).expect("utf8 hash json A");
    let json_b = String::from_utf8(hash_cmd_b.stdout).expect("utf8 hash json B");
    assert_eq!(json_a, json_b);

    let parsed_val: serde_json::Value =
        serde_json::from_str(&json_a).expect("valid JSON from ffr-db hash --json");
    let obj = parsed_val.as_object().expect("top-level JSON object");
    assert_eq!(
        obj.get("format").and_then(|v| v.as_u64()),
        Some(u64::from(HASH_FORMAT_VERSION))
    );
    assert_eq!(
        obj.get("schema_version").and_then(|v| v.as_u64()),
        Some(u64::from(SCHEMA_VERSION))
    );
    assert!(
        obj.get("sqlite_version")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.starts_with("3."))
    );

    let hash_report: DbHashReport =
        serde_json::from_str(&json_a).expect("deserialize DbHashReport");
    assert_eq!(hash_report.file_sha256.len(), 64);
    assert_eq!(hash_report.content_sha256.len(), 64);

    // Ensure `ffr-db build` printed both hashes.
    let stdout_a = String::from_utf8_lossy(&run_a.stdout);
    assert!(
        stdout_a.contains(&hash_report.file_sha256),
        "build stdout missing file_sha256: {stdout_a}"
    );
    assert!(
        stdout_a.contains(&hash_report.content_sha256),
        "build stdout missing content_sha256: {stdout_a}"
    );

    // Ensure virtual tables, shadow tables, and sqlite_* tables are excluded from `tables`.
    let table_names: Vec<&str> = hash_report.tables.iter().map(|t| t.name.as_str()).collect();
    let mut sorted_names = table_names.clone();
    sorted_names.sort_unstable();
    assert_eq!(table_names, sorted_names, "tables must be sorted by name");
    for forbidden in [
        "sqlite_stat1",
        "sqlite_master",
        "text_fts",
        "text_fts_data",
        "text_fts_idx",
        "unit_name_fts",
        "skill_name_fts",
        "world_name_fts",
        "actor_rtree",
        "actor_rtree_node",
        "actor_rtree_rowid",
        "actor_rtree_parent",
    ] {
        assert!(
            !table_names.contains(&forbidden),
            "derived/internal table {forbidden} must be excluded from content hash"
        );
    }
    assert!(table_names.contains(&"meta"));
    assert!(table_names.contains(&"source"));
    assert!(table_names.contains(&"asset"));
    assert!(table_names.contains(&"level"));

    // Verify sidecar manifest `<db>.manifest.json` was written with full metadata.
    let manifest_a_path = manifest_path_for(&db_a);
    let manifest_b_path = manifest_path_for(&db_b);
    assert!(manifest_a_path.is_file());
    assert!(manifest_b_path.is_file());
    let manifest_a: BuildManifest =
        serde_json::from_str(&fs::read_to_string(&manifest_a_path).expect("read manifest A"))
            .expect("parse manifest A");
    let manifest_b: BuildManifest =
        serde_json::from_str(&fs::read_to_string(&manifest_b_path).expect("read manifest B"))
            .expect("parse manifest B");
    assert_eq!(manifest_a.format, MANIFEST_FORMAT_VERSION);
    assert!(manifest_a.built_at.ends_with('Z'));
    assert_eq!(manifest_a.tool.crate_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(manifest_a.tool.schema_version, SCHEMA_VERSION);
    assert_eq!(manifest_a.tool.threads, 1);
    assert_eq!(manifest_b.tool.threads, 4);
    assert_eq!(manifest_a.hash, hash_report);
    assert_eq!(manifest_b.hash, hash_report);
    assert_eq!(manifest_a.inputs, manifest_b.inputs);
    assert!(!manifest_a.inputs.is_empty());

    // Verify no volatile columns, timestamps, or host paths leaked into SQLite.
    let db = GameDb::open(&db_a).expect("open db A");
    let meta = db.meta().expect("read meta");
    assert!(!meta.contains_key("built_at"));

    let host_marker_a = input_a.to_string_lossy().into_owned();
    for src in db.sources().expect("read sources") {
        assert!(
            !src.path.contains(&host_marker_a) && !src.path.starts_with('/'),
            "source.path leaked host path: {}",
            src.path
        );
        assert!(
            !src.path.contains('\\'),
            "source.path must use forward slashes: {}",
            src.path
        );
    }
    let source_paths: Vec<String> = db
        .sources()
        .expect("read sources")
        .into_iter()
        .map(|s| s.path)
        .collect();
    assert!(source_paths.iter().any(|p| p == "export-full/maps"));
    assert!(source_paths.iter().any(|p| p == "jmap/01-mock.jmap"));
    assert!(source_paths.iter().any(|p| p.starts_with("export")));
    assert!(source_paths.iter().any(|p| p == "inventory"));
    assert!(source_paths.iter().any(|p| p == "snapshots"));

    for issue in db.build_issues().expect("read build_issues") {
        assert!(
            !issue.subject.contains(&host_marker_a) && !issue.detail.contains(&host_marker_a),
            "build_issue leaked host path: {issue:?}"
        );
    }
    for rep in db.stage_reports().expect("read stage_reports") {
        assert!(
            !rep.note.contains(&host_marker_a),
            "stage_report.note leaked host path: {rep:?}"
        );
    }

    // Verify fixed SQLite build parameters.
    db.with_conn(|conn| {
        let page_size: u32 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .expect("page_size");
        let auto_vacuum: u32 = conn
            .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
            .expect("auto_vacuum");
        let encoding: String = conn
            .query_row("PRAGMA encoding", [], |r| r.get(0))
            .expect("encoding");
        let app_id: u32 = conn
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .expect("application_id");
        let user_ver: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("user_version");
        let stage_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(stage_report)")
            .expect("prepare table_info")
            .query_map([], |r| r.get(1))
            .expect("query table_info")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect cols");

        assert_eq!(page_size, DB_PAGE_SIZE);
        assert_eq!(auto_vacuum, 0);
        assert_eq!(encoding, "UTF-8");
        assert_eq!(app_id, DB_APPLICATION_ID);
        assert_eq!(user_ver, SCHEMA_VERSION);
        assert!(
            !stage_cols.iter().any(|c| c == "millis"),
            "volatile millis column must not exist in stage_report: {stage_cols:?}"
        );
        Ok(())
    })
    .expect("verify pragmas");
}

#[test]
fn modifying_one_row_changes_content_hash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("mutate.sqlite");

    let fixture = fixture_area_dir("battle");
    build(&BuildOptions {
        inputs: BuildInputs {
            export: Some(fixture.join("export")),
            ..BuildInputs::default()
        },
        output: db_path.clone(),
        stages: Some(vec![Stage::Text, Stage::Battle]),
        game_build_id: Some("test-1".to_owned()),
        threads: Some(2),
    })
    .expect("build battle fixture");

    let before = hash_database(&db_path).expect("hash before mutation");
    let unit_before = before
        .tables
        .iter()
        .find(|t| t.name == "unit")
        .expect("unit table hash")
        .clone();
    let skill_before = before
        .tables
        .iter()
        .find(|t| t.name == "skill")
        .expect("skill table hash")
        .clone();
    assert!(unit_before.rows > 0);

    // Mutate exactly one row in `unit`.
    {
        let conn = Connection::open(&db_path).expect("open writable");
        let changed = conn
            .execute(
                "UPDATE unit SET max_hp = max_hp + 1 WHERE id = (SELECT MIN(id) FROM unit)",
                [],
            )
            .expect("update one unit row");
        assert_eq!(changed, 1);
    }

    let after = hash_database(&db_path).expect("hash after mutation");
    let unit_after = after
        .tables
        .iter()
        .find(|t| t.name == "unit")
        .expect("unit table hash after");
    let skill_after = after
        .tables
        .iter()
        .find(|t| t.name == "skill")
        .expect("skill table hash after");

    assert_ne!(before.file_sha256, after.file_sha256);
    assert_ne!(before.content_sha256, after.content_sha256);
    assert_ne!(unit_before.sha256, unit_after.sha256);
    assert_eq!(unit_before.rows, unit_after.rows);
    assert_eq!(
        skill_before.sha256, skill_after.sha256,
        "untouched table hash must remain unchanged"
    );
}

#[test]
fn changing_row_insertion_order_keeps_content_hash_identical() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_a = dir.path().join("order_a.sqlite");
    let db_b = dir.path().join("order_b.sqlite");

    let ddl = "
        PRAGMA user_version = 1;
        CREATE TABLE items (
            id INTEGER PRIMARY KEY,
            score REAL,
            name TEXT,
            raw BLOB
        );
        CREATE TABLE pairs (
            k1 TEXT NOT NULL,
            k2 INTEGER NOT NULL,
            val TEXT,
            PRIMARY KEY (k1, k2)
        );
        CREATE VIRTUAL TABLE items_fts USING fts5(name, tokenize='trigram');
        CREATE VIRTUAL TABLE items_rtree USING rtree(id, min_x, max_x, min_y, max_y);
    ";

    type ItemRow<'a> = (i64, Option<f64>, Option<&'a str>, Option<&'a [u8]>);
    let rows_items: Vec<ItemRow<'_>> = vec![
        (1, Some(1.25), Some("alpha"), Some(&[0x01, 0x02][..])),
        (2, None, Some("钢蝠"), None),
        (3, Some(-42.5), None, Some(&[][..])),
        (10, Some(0.0), Some(""), Some(&[0xff, 0x00, 0xaa][..])),
    ];
    let rows_pairs: Vec<(&str, i64, Option<&str>)> =
        vec![("a", 1, Some("one")), ("a", 2, None), ("b", 1, Some("two"))];

    // DB A: insert in forward order.
    {
        let conn = Connection::open(&db_a).expect("open db A");
        conn.execute_batch(ddl).expect("ddl A");
        for &(id, score, name, raw) in &rows_items {
            conn.execute(
                "INSERT INTO items (id, score, name, raw) VALUES (?1, ?2, ?3, ?4)",
                params![id, score, name, raw],
            )
            .expect("insert item A");
            if let Some(n) = name {
                conn.execute("INSERT INTO items_fts (name) VALUES (?1)", [n])
                    .expect("insert fts A");
            }
            conn.execute(
                "INSERT INTO items_rtree VALUES (?1, 0.0, 1.0, 0.0, 1.0)",
                [id],
            )
            .expect("insert rtree A");
        }
        for &(k1, k2, val) in &rows_pairs {
            conn.execute(
                "INSERT INTO pairs (k1, k2, val) VALUES (?1, ?2, ?3)",
                params![k1, k2, val],
            )
            .expect("insert pair A");
        }
        conn.execute_batch("ANALYZE;").expect("analyze A");
    }

    // DB B: insert the exact same rows in reversed / shuffled order.
    {
        let conn = Connection::open(&db_b).expect("open db B");
        conn.execute_batch(ddl).expect("ddl B");
        for &(id, score, name, raw) in rows_items.iter().rev() {
            conn.execute(
                "INSERT INTO items (id, score, name, raw) VALUES (?1, ?2, ?3, ?4)",
                params![id, score, name, raw],
            )
            .expect("insert item B");
            if let Some(n) = name {
                conn.execute("INSERT INTO items_fts (name) VALUES (?1)", [n])
                    .expect("insert fts B");
            }
            conn.execute(
                "INSERT INTO items_rtree VALUES (?1, 0.0, 1.0, 0.0, 1.0)",
                [id],
            )
            .expect("insert rtree B");
        }
        for &(k1, k2, val) in rows_pairs.iter().rev() {
            conn.execute(
                "INSERT INTO pairs (k1, k2, val) VALUES (?1, ?2, ?3)",
                params![k1, k2, val],
            )
            .expect("insert pair B");
        }
        conn.execute_batch("ANALYZE;").expect("analyze B");
    }

    let hash_a = hash_database(&db_a).expect("hash A");
    let hash_b = hash_database(&db_b).expect("hash B");

    assert_eq!(hash_a.content_sha256, hash_b.content_sha256);
    assert_eq!(hash_a.tables, hash_b.tables);
    let names: Vec<&str> = hash_a.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["items", "pairs"]);
}

#[test]
fn duplicate_jmap_filenames_are_disambiguated_deterministically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let refl = fixture_area_dir("reflection");

    let run1_dir = dir.path().join("host_one");
    let run2_dir = dir.path().join("host_two");
    for root in [&run1_dir, &run2_dir] {
        fs::create_dir_all(root.join("title-jmap")).expect("mkdir title");
        fs::create_dir_all(root.join("battle-jmap")).expect("mkdir battle");
        fs::copy(
            refl.join("01-base.jmap"),
            root.join("title-jmap/SameDump.jmap"),
        )
        .expect("copy first jmap");
        fs::copy(
            refl.join("02-override.jmap"),
            root.join("battle-jmap/SameDump.jmap"),
        )
        .expect("copy second jmap");
    }

    let db1 = dir.path().join("one.sqlite");
    let db2 = dir.path().join("two.sqlite");
    let rep1 = build(&BuildOptions {
        inputs: BuildInputs {
            jmaps: vec![
                run1_dir.join("title-jmap/SameDump.jmap"),
                run1_dir.join("battle-jmap/SameDump.jmap"),
            ],
            ..BuildInputs::default()
        },
        output: db1.clone(),
        stages: Some(vec![Stage::Reflection]),
        game_build_id: Some("dup-test".to_owned()),
        threads: Some(1),
    })
    .expect("build 1");
    let rep2 = build(&BuildOptions {
        inputs: BuildInputs {
            jmaps: vec![
                run2_dir.join("title-jmap/SameDump.jmap"),
                run2_dir.join("battle-jmap/SameDump.jmap"),
            ],
            ..BuildInputs::default()
        },
        output: db2.clone(),
        stages: Some(vec![Stage::Reflection]),
        game_build_id: Some("dup-test".to_owned()),
        threads: Some(2),
    })
    .expect("build 2");

    assert_eq!(rep1.file_sha256, rep2.file_sha256);
    assert_eq!(rep1.content_sha256, rep2.content_sha256);

    let db = GameDb::open(&db1).expect("open db1");
    let sources: Vec<String> = db
        .sources()
        .expect("sources")
        .into_iter()
        .map(|s| s.path)
        .collect();
    assert_eq!(
        sources,
        vec!["jmap/SameDump.jmap#1", "jmap/SameDump.jmap#2"]
    );
}

#[test]
fn exporter_manifest_volatile_keys_do_not_change_source_hash_but_semantic_keys_do() {
    let work = tempfile::tempdir().expect("create temp workspace");
    let links_fixture = fixture_area_dir("links");
    let assets_fixture = fixture_area_dir("assets");

    let run_a = work.path().join("run_a");
    let run_b = work.path().join("run_b");
    let run_c = work.path().join("run_c");

    for root in [&run_a, &run_b, &run_c] {
        copy_dir_recursive(&links_fixture, root);
        copy_dir_recursive(&assets_fixture.join("inventory"), &root.join("inventory"));
        copy_dir_recursive(
            &assets_fixture.join("export-full"),
            &root.join("export-full"),
        );
    }

    // Write realistic exporter manifests matching `src/FfrExport/` formats.
    // Run A and Run B differ ONLY in volatile keys (`startedAt`, `finishedAt`, `elapsedSeconds`,
    // `elapsedMs`, `generatedAt`, `jobs`) and key ordering / whitespace.
    let write_manifests = |root: &Path,
                           started: &str,
                           finished: &str,
                           elapsed_secs: f64,
                           elapsed_ms: u64,
                           jobs: u32,
                           usmap_sha: &str,
                           compact: bool| {
        let export_manifest = if compact {
            format!(
                r#"{{"failureCount":0,"failures":[],"finishedAt":"{finished}","startedAt":"{started}","successCount":10,"usmapSha256":"{usmap_sha}","cue4ParseVersion":"1.2.2"}}"#
            )
        } else {
            format!(
                r#"{{
  "cue4ParseVersion": "1.2.2",
  "usmapSha256": "{usmap_sha}",
  "startedAt": "{started}",
  "finishedAt": "{finished}",
  "successCount": 10,
  "failureCount": 0,
  "failures": []
}}"#
            )
        };
        fs::write(root.join("export/manifest.json"), export_manifest)
            .expect("write export manifest");

        let loc_manifest = format!(
            r#"{{
  "cue4ParseVersion": "1.2.2",
  "command": "locres",
  "startedAt": "{started}",
  "finishedAt": "{finished}",
  "languages": [{{ "language": "zh-Hans", "entryCount": 5 }}]
}}"#
        );
        fs::write(root.join("export/Localization/manifest.json"), loc_manifest)
            .expect("write loc manifest");

        let inv_manifest = format!(
            r#"{{
  "cue4ParseVersion": "1.2.2",
  "command": "inventory",
  "startedAt": "{started}",
  "finishedAt": "{finished}",
  "packageCount": 12
}}"#
        );
        fs::write(root.join("inventory/manifest.json"), inv_manifest).expect("write inv manifest");

        let full_manifest = format!(
            r#"{{
  "cue4ParseVersion": "1.2.2",
  "command": "export-assets",
  "status": "completed",
  "usmapSha256": "{usmap_sha}",
  "jobs": {jobs},
  "startedAt": "{started}",
  "finishedAt": "{finished}",
  "elapsedSeconds": {elapsed_secs},
  "totals": {{ "exportedPackages": 4, "generatedAt": "{finished}" }}
}}"#
        );
        fs::write(root.join("export-full/manifest.json"), full_manifest)
            .expect("write full manifest");

        let maps_manifest = format!(
            r#"{{
  "cue4ParseVersion": "1.2.2",
  "command": "export-maps",
  "status": "completed",
  "usmapSha256": "{usmap_sha}",
  "startedAt": "{started}",
  "finishedAt": "{finished}",
  "elapsedMs": {elapsed_ms},
  "totalLevels": 3,
  "successCount": 3,
  "failureCount": 0,
  "graphFile": "_graph.json",
  "failures": []
}}"#
        );
        fs::write(root.join("maps/manifest.json"), maps_manifest).expect("write maps manifest");
    };

    write_manifests(
        &run_a,
        "2026-11-25T14:00:00.111Z",
        "2026-11-25T14:01:00.222Z",
        60.111,
        1234,
        4,
        "USMAP_HASH_V1",
        false,
    );
    write_manifests(
        &run_b,
        "2026-11-25T15:30:00.999Z",
        "2026-11-25T15:32:45.888Z",
        165.889,
        9876,
        16,
        "USMAP_HASH_V1",
        true,
    );
    write_manifests(
        &run_c,
        "2026-11-25T14:00:00.111Z",
        "2026-11-25T14:01:00.222Z",
        60.111,
        1234,
        4,
        "USMAP_HASH_V2",
        false,
    );

    let build_one = |root: &Path, out: &Path| {
        build(&BuildOptions {
            inputs: BuildInputs {
                export: Some(root.join("export")),
                full: Some(root.join("export-full")),
                inventory: Some(root.join("inventory")),
                maps: Some(root.join("maps")),
                jmaps: vec![root.join("01-mock.jmap")],
                snapshots: Some(root.join("snapshots")),
            },
            output: out.to_path_buf(),
            stages: None,
            game_build_id: Some("e2e-manifest-test".to_owned()),
            threads: Some(2),
        })
        .expect("build database")
    };

    let db_a = work.path().join("a.sqlite");
    let db_b = work.path().join("b.sqlite");
    let db_c = work.path().join("c.sqlite");
    let rep_a = build_one(&run_a, &db_a);
    let rep_b = build_one(&run_b, &db_b);
    let rep_c = build_one(&run_c, &db_c);

    // Run A and Run B only differed in volatile manifest keys -> byte-for-byte identical DBs!
    assert_eq!(rep_a.file_sha256, rep_b.file_sha256);
    assert_eq!(rep_a.content_sha256, rep_b.content_sha256);
    assert_eq!(
        fs::read(&db_a).expect("read db A"),
        fs::read(&db_b).expect("read db B")
    );

    // Run C changed a non-volatile manifest key (`usmapSha256`) -> source & content hash must differ!
    assert_ne!(rep_a.content_sha256, rep_c.content_sha256);
    assert_ne!(rep_a.file_sha256, rep_c.file_sha256);

    let hash_a = hash_database(&db_a).expect("hash db A");
    let hash_c = hash_database(&db_c).expect("hash db C");
    let diff_tables: Vec<&str> = hash_a
        .tables
        .iter()
        .zip(&hash_c.tables)
        .filter(|(a, c)| a.sha256 != c.sha256)
        .map(|(a, _)| a.name.as_str())
        .collect();
    assert_eq!(
        diff_tables,
        vec!["source"],
        "only `source` should differ when a non-volatile manifest key changes"
    );

    // Sidecar `.manifest.json` must record both canonical `manifest_sha256` (matching between A/B)
    // and raw file `raw_manifest_sha256` (distinct between A/B, and NOT stored inside SQLite).
    let manifest_a: BuildManifest =
        serde_json::from_str(&fs::read_to_string(manifest_path_for(&db_a)).expect("read mA"))
            .expect("parse mA");
    let manifest_b: BuildManifest =
        serde_json::from_str(&fs::read_to_string(manifest_path_for(&db_b)).expect("read mB"))
            .expect("parse mB");

    let db = GameDb::open(&db_a).expect("open db A");
    let db_sources = db.sources().expect("read db sources");

    for (in_a, in_b) in manifest_a.inputs.iter().zip(&manifest_b.inputs) {
        assert_eq!(in_a.stage, in_b.stage);
        assert_eq!(in_a.path, in_b.path);
        assert_eq!(
            in_a.manifest_sha256, in_b.manifest_sha256,
            "canonical manifest hash must match for {}/{}",
            in_a.stage, in_a.path
        );
        if matches!(
            in_a.path.as_str(),
            "export" | "export/Localization" | "inventory" | "export-full" | "export-full/maps"
        ) {
            assert!(in_a.manifest_sha256.is_some());
            assert!(in_a.raw_manifest_sha256.is_some());
            assert_ne!(
                in_a.raw_manifest_sha256, in_b.raw_manifest_sha256,
                "raw file hash must differ between A and B for {}/{}",
                in_a.stage, in_a.path
            );
            assert_ne!(
                in_a.manifest_sha256, in_a.raw_manifest_sha256,
                "canonical hash must not equal raw hash when volatile keys were stripped"
            );
            // Raw hash must never appear in SQLite `source.manifest_sha256`.
            let raw_a = in_a.raw_manifest_sha256.as_deref().unwrap();
            assert!(
                db_sources
                    .iter()
                    .all(|s| s.manifest_sha256.as_deref() != Some(raw_a)),
                "raw manifest hash leaked into SQLite source table"
            );
        }
    }
}
