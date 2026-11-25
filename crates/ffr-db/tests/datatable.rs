use std::fs;
use std::path::{Path, PathBuf};

use ffr_data::{DataStore, DataTableType};
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db_stages, build_temp_db, fixture_area_dir,
};
use ffr_db::{BuildInputs, GameDb, Stage};
use rusqlite::params;
use serde_json::Value;

#[test]
fn raw_table_queries_match_data_store_and_use_indexes() {
    let test_db = build_fixture_db_stages("datatable", Some(vec![Stage::Datatable]))
        .expect("build datatable fixture");
    let db = test_db.db();
    let store = DataStore::new(fixture_area_dir("datatable").join("export"));

    let tables = db.dt_tables("Map/").expect("prefix query");
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].path, "Map/CDT_MapData_Prod");
    assert_eq!(tables[0].kind, DataTableType::CompositeDataTable);
    assert_eq!(tables[0].row_struct, "MapData");
    assert_eq!(tables[0].row_count, 2);
    assert_eq!(tables[1].path, "Map/MapData/DT_MapData_00Com");
    assert_eq!(tables[1].kind, DataTableType::DataTable);
    assert_eq!(db.dt_tables("").expect("all tables").len(), 2);
    assert!(db.dt_tables("Missing/").expect("missing prefix").is_empty());

    let info = db
        .dt_table_by_ue_path("/Game/Datatable/Map/CDT_MapData_Prod")
        .expect("UE path query")
        .expect("table exists");
    assert_eq!(info, tables[0]);
    let object_path = db
        .dt_table_by_ue_path("/Game/Datatable/Map/CDT_MapData_Prod.CDT_MapData_Prod")
        .expect("UE object path query");
    assert_eq!(object_path, Some(tables[0].clone()));
    assert_eq!(
        db.dt_table_by_ue_path("/Engine/Datatable/Map/CDT_MapData_Prod")
            .expect("invalid UE path"),
        None
    );

    for relative in ["Map/CDT_MapData_Prod", "Map/MapData/DT_MapData_00Com"] {
        let expected = store.data_table(relative).expect("DataStore table");
        let rows = db.dt_rows(relative).expect("database rows");
        let expected_rows: Vec<(String, Value)> = expected
            .rows
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        assert_eq!(rows, expected_rows);
        for (row_name, expected_row) in &expected.rows {
            assert_eq!(
                db.dt_row(relative, row_name).expect("database row"),
                Some(expected_row.clone())
            );
        }
    }
    assert_eq!(
        db.dt_rows("Map/CDT_MapData_Prod").expect("rows")[0].0,
        "RowA"
    );
    assert_eq!(
        db.dt_row("Map/CDT_MapData_Prod", "Missing")
            .expect("missing row"),
        None
    );
    assert_eq!(
        db.dt_row("Missing/Table", "Row")
            .expect("missing table row"),
        None
    );

    assert_db_uses_index(
        db,
        "SELECT id, path, kind, row_struct, row_count FROM dt_table \
         INDEXED BY idx_dt_table_path WHERE path >= ?1 AND path < ?2 ORDER BY path",
        params!["Map/", "Map0"],
        &["dt_table"],
    );
    assert_db_uses_index(
        db,
        "SELECT r.doc FROM dt_table AS t INDEXED BY idx_dt_table_path \
         JOIN dt_row AS r ON r.table_id = t.id WHERE t.path = ?1 AND r.row_name = ?2",
        params!["Map/CDT_MapData_Prod", "RowA"],
        &["dt_table", "dt_row"],
    );
    assert_db_uses_index(
        db,
        "SELECT r.row_name, r.doc FROM dt_table AS t INDEXED BY idx_dt_table_path \
         JOIN dt_row AS r ON r.table_id = t.id WHERE t.path = ?1 ORDER BY r.row_name",
        ["Map/CDT_MapData_Prod"],
        &["dt_table", "dt_row"],
    );
    assert_db_uses_index(
        db,
        "SELECT id, path, kind, row_struct, row_count FROM dt_table \
         INDEXED BY idx_dt_table_path WHERE path = ?1",
        ["Map/CDT_MapData_Prod"],
        &["dt_table"],
    );
}

#[test]
fn real_data_all_2618_table_row_counts_match_data_store() {
    let Some(export) = real_export() else {
        eprintln!("跳过真实数据表对齐：.deps/ffr/export 不存在");
        return;
    };
    let store = DataStore::new(&export);
    let test_db = build_temp_db(
        "real-datatable-alignment",
        BuildInputs {
            export: Some(export.clone()),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Text, Stage::Datatable]),
    )
    .expect("build real text and datatable database");
    let db = test_db.db();
    let root = export.join("FFRS/Content/Datatable");
    let mut files = Vec::new();
    collect_json_files(&root, &mut files);
    files.sort();
    assert_eq!(files.len(), 2618, "真实导出文件数与任务基线不同");

    let mut data_tables = 0_usize;
    let mut string_tables = 0_usize;
    let mut non_tables = 0_usize;
    for file in files {
        let document: Value = serde_json::from_str(
            &fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("读取 {}：{err}", file.display())),
        )
        .unwrap_or_else(|err| panic!("解析 {}：{err}", file.display()));
        let kind = document[0]["Type"].as_str().expect("导出 Type");
        let relative = file
            .strip_prefix(&root)
            .expect("relative path")
            .to_str()
            .expect("UTF-8 path")
            .replace(std::path::MAIN_SEPARATOR, "/");
        let relative = relative.strip_suffix(".json").expect("json suffix");
        match kind {
            "DataTable" | "CompositeDataTable" => {
                let expected = store.data_table(relative).expect("DataStore data table");
                let info = db
                    .dt_tables(relative)
                    .expect("database table metadata")
                    .into_iter()
                    .find(|info| info.path == relative)
                    .unwrap_or_else(|| panic!("数据库缺少 {relative}"));
                assert_eq!(
                    info.row_count,
                    expected.len() as u64,
                    "{relative} 元数据行数"
                );
                assert_eq!(
                    db.dt_rows(relative).expect("database rows").len(),
                    expected.len(),
                    "{relative} 实际行数"
                );
                data_tables += 1;
            }
            "StringTable" => {
                let expected = store
                    .string_table(relative)
                    .expect("DataStore string table");
                let table_id = format!("{}.{}", expected.package_path, expected.name);
                let count = string_entry_count(db, &table_id);
                assert_eq!(count, expected.len() as u64, "{relative} 字符串行数");
                string_tables += 1;
            }
            _ => non_tables += 1,
        }
    }
    assert_eq!(data_tables, 1704 + 20);
    assert_eq!(string_tables, 870);
    assert_eq!(non_tables, 24);
    println!(
        "真实行数对齐：DataTable/CompositeDataTable {data_tables} 个，StringTable {string_tables} 个；非表资产 {non_tables} 个"
    );
}

fn string_entry_count(db: &GameDb, table_id: &str) -> u64 {
    db.with_conn(|conn| {
        let count: i64 = conn
            .prepare_cached("SELECT COUNT(*) FROM string_table_entry WHERE table_id = ?1")
            .expect("prepare string count")
            .query_row([table_id], |row| row.get(0))
            .expect("query string count");
        Ok(u64::try_from(count).expect("nonnegative count"))
    })
    .expect("string entry count")
}

fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("遍历 {}：{err}", dir.display()))
    {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_json_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
}

fn real_export() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.deps/ffr/export");
    path.join("FFRS/Content/Datatable").is_dir().then_some(path)
}
