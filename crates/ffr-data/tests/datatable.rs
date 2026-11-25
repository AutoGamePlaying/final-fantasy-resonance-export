use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ffr_data::{
    DataError, DataStore, DataTableType, datatable_dir, datatable_relative_path_to_json,
    export_dir_from_deps, export_dir_from_env_value, load_data_table, load_localization,
    load_string_table, localization_file, normalize_row_struct_name, parse_data_table_document,
    parse_localization_document, parse_string_table_document, resolve_export_dir,
    ue_object_path_to_export_json,
};
use serde_json::json;
use tempfile::tempdir;

fn fixture_export() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rust/data/export")
        .canonicalize()
        .expect("fixture export directory exists")
}

#[test]
fn load_data_table_reads_normal_and_composite_tables() {
    let export = fixture_export();

    let chapter_path = datatable_dir(&export).join("Scenario/DT_ChapterData.json");
    let chapter_table = load_data_table(&chapter_path).expect("load DT_ChapterData");
    assert_eq!(chapter_table.table_type, DataTableType::DataTable);
    assert_eq!(chapter_table.name, "DT_ChapterData");
    assert_eq!(
        chapter_table.package_path,
        "/Game/Datatable/Scenario/DT_ChapterData"
    );
    assert_eq!(chapter_table.row_struct, "ChapterData");
    assert_eq!(chapter_table.len(), 3);
    assert_eq!(chapter_table.get_row("Ch1").unwrap()["ChapterNo"], 1);

    let map_path = datatable_dir(&export).join("Map/CDT_MapData_Prod.json");
    let map_table = load_data_table(&map_path).expect("load CDT_MapData_Prod");
    assert_eq!(map_table.table_type, DataTableType::CompositeDataTable);
    assert_eq!(map_table.row_struct, "MAP_DATA");
    assert_eq!(map_table.len(), 2);
    assert_eq!(map_table.get_row("Map_Alpha").unwrap()["ID"], 2001);
}

#[test]
fn load_string_table_and_localization_read_entries() {
    let export = fixture_export();

    let st_path = datatable_dir(&export).join("Strings/ChapterTitle/ST_ChapterTitle.json");
    let st = load_string_table(&st_path).expect("load ST_ChapterTitle");
    assert_eq!(st.table_namespace, "ST_ChapterTitle");
    assert_eq!(st.len(), 3);
    assert_eq!(st.get("Title_001"), Some("はじまりの森"));

    let zh = load_localization(localization_file(&export, "zh-Hans")).expect("load zh-Hans.json");
    assert_eq!(zh.get("ST_ChapterTitle", "Title_001"), Some("起始之森"));
    assert_eq!(zh.namespace_count(), 4);
    assert_eq!(zh.entry_count(), 5);
}

#[test]
fn data_store_loads_on_demand_and_caches_in_memory() {
    let temp = tempdir().expect("create tempdir");
    let root = temp.path();

    let dt_path = datatable_dir(root).join("Scenario/DT_Demo.json");
    fs::create_dir_all(dt_path.parent().unwrap()).unwrap();
    fs::write(
        &dt_path,
        r#"[
          {
            "Type": "DataTable",
            "Name": "DT_Demo",
            "Package": "/Game/Datatable/Scenario/DT_Demo",
            "Properties": { "RowStruct": { "ObjectName": "Class'DemoRow'" } },
            "Rows": { "R1": { "Value": 42 } }
          }
        ]"#,
    )
    .unwrap();

    let st_path = datatable_dir(root).join("Strings/ST_Demo.json");
    fs::create_dir_all(st_path.parent().unwrap()).unwrap();
    fs::write(
        &st_path,
        r#"[
          {
            "Type": "StringTable",
            "Name": "ST_Demo",
            "Package": "/Game/Datatable/Strings/ST_Demo",
            "StringTable": {
              "TableNamespace": "ST_Demo",
              "KeysToEntries": { "K1": "原文" }
            }
          }
        ]"#,
    )
    .unwrap();

    let loc_path = localization_file(root, "zh-Hans");
    fs::create_dir_all(loc_path.parent().unwrap()).unwrap();
    fs::write(&loc_path, r#"{ "ST_Demo": { "K1": "译文" } }"#).unwrap();

    let store = DataStore::new(root);
    assert_eq!(store.cached_data_table_count(), 0);
    assert_eq!(store.cached_string_table_count(), 0);
    assert_eq!(store.cached_localization_count(), 0);

    let first_dt = store.data_table("Scenario/DT_Demo").expect("load DT_Demo");
    let second_dt = store
        .data_table("Scenario/DT_Demo.json")
        .expect("cache hit with .json");
    let third_dt = store
        .data_table_by_ue_path("/Game/Datatable/Scenario/DT_Demo.DT_Demo")
        .expect("cache hit by UE path");
    assert!(Arc::ptr_eq(&first_dt, &second_dt));
    assert!(Arc::ptr_eq(&first_dt, &third_dt));
    assert_eq!(store.cached_data_table_count(), 1);

    let first_st = store.string_table("Strings/ST_Demo").expect("load ST_Demo");
    let second_st = store
        .string_table_by_id("/Game/Datatable/Strings/ST_Demo.ST_Demo")
        .expect("cache hit by TableId");
    assert!(Arc::ptr_eq(&first_st, &second_st));
    assert_eq!(store.cached_string_table_count(), 1);

    let first_loc = store.localization("zh-Hans").expect("load zh-Hans");
    assert_eq!(store.cached_localization_count(), 1);

    // Delete files on disk to prove subsequent reads serve from memory cache.
    fs::remove_file(&dt_path).unwrap();
    fs::remove_file(&st_path).unwrap();
    fs::remove_file(&loc_path).unwrap();

    let cached_dt = store.data_table("Scenario/DT_Demo").expect("dt cache hit");
    assert_eq!(cached_dt.get_row("R1").unwrap()["Value"], 42);
    let cached_st = store.string_table("Strings/ST_Demo").expect("st cache hit");
    assert_eq!(cached_st.get("K1"), Some("原文"));
    let cached_loc = store.localization("zh-Hans").expect("loc cache hit");
    assert!(Arc::ptr_eq(&first_loc, &cached_loc));
}

#[test]
fn errors_report_file_and_field_in_chinese() {
    let export = fixture_export();

    let missing_err = load_data_table(export.join("non-existent.json")).unwrap_err();
    assert!(matches!(missing_err, DataError::Io { .. }));
    let missing_msg = missing_err.to_string();
    assert!(missing_msg.contains("读取数据表失败"), "{missing_msg}");
    assert!(missing_msg.contains("non-existent.json"), "{missing_msg}");

    let st_path = datatable_dir(&export).join("Strings/ChapterTitle/ST_ChapterTitle.json");
    let wrong_type = load_data_table(&st_path).unwrap_err();
    let wrong_msg = wrong_type.to_string();
    assert!(
        wrong_msg.contains("不是 DataTable 或 CompositeDataTable"),
        "{wrong_msg}"
    );
    assert!(wrong_msg.contains("ST_ChapterTitle.json"), "{wrong_msg}");
    assert!(wrong_msg.contains("Type"), "{wrong_msg}");

    let temp = tempdir().unwrap();
    let broken_path = temp.path().join("broken.json");
    fs::write(&broken_path, "{ not-json").unwrap();
    let json_err = load_data_table(&broken_path).unwrap_err();
    assert!(matches!(json_err, DataError::Json { .. }));
    assert!(json_err.to_string().contains("JSON 格式错误"));
    assert!(json_err.to_string().contains("broken.json"));

    let empty_err = parse_data_table_document(&json!([]), "empty.json").unwrap_err();
    assert!(empty_err.to_string().contains("empty.json"));
    assert!(empty_err.to_string().contains("不是非空 JSON 数组"));

    let no_struct = parse_data_table_document(
        &json!([{
            "Type": "DataTable",
            "Name": "DT_X",
            "Package": "/Game/X",
            "Properties": {},
            "Rows": {}
        }]),
        "no-struct.json",
    )
    .unwrap_err();
    assert!(no_struct.to_string().contains("no-struct.json"));
    assert!(
        no_struct
            .to_string()
            .contains("Properties.RowStruct.ObjectName")
    );

    let bad_row = parse_data_table_document(
        &json!([{
            "Type": "DataTable",
            "Name": "DT_X",
            "Package": "/Game/X",
            "Properties": { "RowStruct": { "ObjectName": "Class'RowX'" } },
            "Rows": { "BadRow": 123 }
        }]),
        "bad-row.json",
    )
    .unwrap_err();
    assert!(bad_row.to_string().contains("bad-row.json"));
    assert!(bad_row.to_string().contains("Rows.BadRow"));

    let wrong_st = parse_string_table_document(
        &json!([{ "Type": "DataTable", "Name": "DT_X", "Package": "/Game/X" }]),
        "wrong-st.json",
    )
    .unwrap_err();
    assert!(wrong_st.to_string().contains("wrong-st.json"));
    assert!(wrong_st.to_string().contains("不是 StringTable"));

    let bad_loc_root = parse_localization_document(&json!([]), "bad.json").unwrap_err();
    assert!(bad_loc_root.to_string().contains("bad.json"));
    assert!(bad_loc_root.to_string().contains("不是 JSON 对象"));

    let bad_loc_entry =
        parse_localization_document(&json!({ "NS": { "K": 123 } }), "bad-entry.json").unwrap_err();
    assert!(bad_loc_entry.to_string().contains("bad-entry.json"));
    assert!(bad_loc_entry.to_string().contains("NS/K"));
    assert!(bad_loc_entry.to_string().contains("不是字符串"));

    assert_eq!(
        normalize_row_struct_name("Class'ChapterData'"),
        "ChapterData"
    );
    assert_eq!(normalize_row_struct_name("PlainStruct"), "PlainStruct");
}

#[test]
fn path_mapping_and_env_resolution_work_safely() {
    assert_eq!(
        export_dir_from_deps("/opt/deps"),
        PathBuf::from("/opt/deps/ffr/export")
    );
    assert_eq!(
        export_dir_from_env_value(Some(OsString::from("/opt/deps"))).unwrap(),
        PathBuf::from("/opt/deps/ffr/export")
    );
    assert!(matches!(
        export_dir_from_env_value(None),
        Err(DataError::MissingEnvVar { var: "FFR_DEPS" })
    ));
    assert!(matches!(
        export_dir_from_env_value(Some(OsString::new())),
        Err(DataError::MissingEnvVar { var: "FFR_DEPS" })
    ));
    assert_eq!(
        resolve_export_dir(Some(Path::new("/explicit/export"))).unwrap(),
        PathBuf::from("/explicit/export")
    );

    assert_eq!(
        ue_object_path_to_export_json(
            "/Game/Datatable/Strings/ChapterTitle/ST_ChapterTitle.ST_ChapterTitle",
            "/tmp/export",
        ),
        Some(PathBuf::from(
            "/tmp/export/FFRS/Content/Datatable/Strings/ChapterTitle/ST_ChapterTitle.json"
        ))
    );
    assert_eq!(
        ue_object_path_to_export_json("/Game/Datatable/Scenario/DT_ChapterData", "/tmp/export"),
        Some(PathBuf::from(
            "/tmp/export/FFRS/Content/Datatable/Scenario/DT_ChapterData.json"
        ))
    );
    assert_eq!(
        ue_object_path_to_export_json("/Script/mucha.Foo", "/tmp/export"),
        None
    );
    assert_eq!(
        ue_object_path_to_export_json("/Game/Datatable/../Secret.Secret", "/tmp/export"),
        None
    );

    assert_eq!(
        datatable_relative_path_to_json("Scenario/DT_ChapterData", "/tmp/export"),
        Some(PathBuf::from(
            "/tmp/export/FFRS/Content/Datatable/Scenario/DT_ChapterData.json"
        ))
    );
    assert_eq!(
        datatable_relative_path_to_json("Scenario/DT_ChapterData.json", "/tmp/export"),
        Some(PathBuf::from(
            "/tmp/export/FFRS/Content/Datatable/Scenario/DT_ChapterData.json"
        ))
    );
    assert_eq!(
        datatable_relative_path_to_json("../Secret", "/tmp/export"),
        None
    );
    assert_eq!(
        datatable_relative_path_to_json("/etc/passwd", "/tmp/export"),
        None
    );
    assert_eq!(
        datatable_relative_path_to_json("Scenario\\DT_ChapterData", "/tmp/export"),
        None
    );
}
