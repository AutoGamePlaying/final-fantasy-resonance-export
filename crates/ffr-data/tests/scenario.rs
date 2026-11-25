use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ffr_data::{
    DataError, DataStore, DataTable, DataTableType, ScenarioData, TextResolverOptions,
    chapter_no_from_progress, find_chapter, get_chapter_main_scenario, load_chapters,
    load_main_scenario, parse_chapters, parse_main_progress_condition, parse_main_scenario,
};
use serde_json::json;

fn fixture_export() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rust/data/export")
        .canonicalize()
        .expect("fixture export directory exists")
}

#[test]
fn parse_progress_condition_and_chapter_derivation() {
    assert_eq!(
        parse_main_progress_condition("{progress:0}>=1020"),
        Some(1020)
    );
    assert_eq!(
        parse_main_progress_condition(" {progress:0}>=8050 "),
        Some(8050)
    );
    assert_eq!(parse_main_progress_condition(""), None);
    assert_eq!(parse_main_progress_condition("{progress:1}==50"), None);
    assert_eq!(parse_main_progress_condition("{progress:0}>=+100"), None);

    assert_eq!(chapter_no_from_progress(1020), Some(1));
    assert_eq!(chapter_no_from_progress(2000), Some(2));
    assert_eq!(chapter_no_from_progress(8050), Some(8));
    assert_eq!(chapter_no_from_progress(999), None);
}

#[test]
fn chapters_and_chapter_name_queries() {
    let export = fixture_export();
    let chapters = load_chapters(&export).expect("load chapters");
    let nos: Vec<i64> = chapters.iter().map(|ch| ch.chapter_no).collect();
    assert_eq!(nos, vec![1, 2, 3]);
    assert_eq!(
        find_chapter(&chapters, 2).map(|ch| ch.row_key.as_str()),
        Some("Ch2")
    );
    assert!(find_chapter(&chapters, 99).is_none());

    let scenario = ScenarioData::load(&export).expect("load ScenarioData");
    let ch1_default = scenario.chapter_name(1);
    assert_eq!(ch1_default.text.as_deref(), Some("起始之森"));
    assert_eq!(ch1_default.lang.as_deref(), Some("zh-Hans"));

    let ch1_ja = scenario.chapter_name_in(1, "ja");
    assert_eq!(ch1_ja.text.as_deref(), Some("はじまりの森"));
    assert_eq!(ch1_ja.lang.as_deref(), Some("ja"));

    let ch1_en = scenario.chapter_name_in(1, "en");
    assert_eq!(ch1_en.text.as_deref(), Some("Forest of Beginnings"));
    assert_eq!(ch1_en.lang.as_deref(), Some("en"));

    let ch3_zh = scenario.chapter_name_in(3, "zh-Hans");
    assert_eq!(ch3_zh.text, None);
    assert_eq!(ch3_zh.lang, None);
    assert!(ch3_zh.reason.as_deref().unwrap().contains("Title_003"));

    let missing = scenario.chapter_name_in(99, "ja");
    assert_eq!(missing.text, None);
    assert_eq!(missing.lang, None);
    assert!(
        missing
            .reason
            .as_deref()
            .unwrap()
            .contains("找不到第 99 章")
    );

    let en_scenario = ScenarioData::load_with_options(
        &export,
        TextResolverOptions {
            language: "en".to_owned(),
            ..TextResolverOptions::default()
        },
    )
    .expect("load en ScenarioData");
    let ch2_en = en_scenario.chapter_name(2);
    assert_eq!(ch2_en.text.as_deref(), Some("Rippling Harbor"));
    assert_eq!(ch2_en.lang.as_deref(), Some("en"));
}

#[test]
fn main_scenario_filtering_sorting_and_text_resolution() {
    let export = fixture_export();
    let entries = load_main_scenario(&export).expect("load main scenario");
    assert_eq!(entries.len(), 4);

    let placeholder = entries
        .iter()
        .find(|entry| entry.row_key == "NewRow")
        .expect("placeholder row exists");
    assert_eq!(placeholder.min_progress, None);
    assert_eq!(placeholder.chapter_no, None);

    let ch1 = get_chapter_main_scenario(&entries, 1);
    let summary: Vec<(&str, Option<i64>)> = ch1
        .iter()
        .map(|entry| (entry.row_key.as_str(), entry.min_progress))
        .collect();
    assert_eq!(
        summary,
        vec![("Step_1010", Some(1010)), ("Step_1050", Some(1050))]
    );

    let first_dest = &ch1[0].destinations[0];
    assert_eq!(first_dest.destination_id, 2001);
    assert_eq!(first_dest.object_type, "eMapObjectType::NPC");
    assert_eq!(first_dest.object_id, 12);
    assert_eq!(ch1[1].destinations[0].condition, "!{flag:101}");

    let ch2 = get_chapter_main_scenario(&entries, 2);
    assert_eq!(ch2.len(), 1);
    assert_eq!(ch2[0].destinations[0].destination_id, 3001);
    assert!(get_chapter_main_scenario(&entries, 3).is_empty());

    let store = DataStore::new(&export);
    let scenario = store.scenario().expect("scenario from DataStore");
    assert_eq!(store.cached_data_table_count(), 2);

    let texts_default = scenario.resolve_main_scenario_texts(ch1[0]);
    assert_eq!(texts_default.text.text.as_deref(), Some("与村口向导交谈"));
    assert_eq!(texts_default.text.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(
        texts_default.summary_text.text.as_deref(),
        Some("旅人抵达了森林入口。")
    );
    assert_eq!(store.cached_string_table_count(), 2);
    assert_eq!(store.cached_localization_count(), 1);

    let texts_en = scenario.resolve_main_scenario_texts_in(ch1[0], "en");
    assert_eq!(
        texts_en.text.text.as_deref(),
        Some("Speak with the village guide")
    );
    assert_eq!(
        texts_en.summary_text.text.as_deref(),
        Some("The travelers arrived at the forest entrance.")
    );

    let texts_ja = scenario.resolve_main_scenario_texts_in(ch1[0], "ja");
    assert_eq!(
        texts_ja.text.text.as_deref(),
        Some("村の案内人に話しかける")
    );
    assert_eq!(
        texts_ja.summary_text.text.as_deref(),
        Some("旅人は森の入口にたどり着いた。")
    );

    let placeholder_texts = scenario.resolve_main_scenario_texts_in(placeholder, "ja");
    assert_eq!(placeholder_texts.text.text, None);
    assert!(
        placeholder_texts
            .text
            .reason
            .as_deref()
            .unwrap()
            .contains("为空")
    );
}

#[test]
fn scenario_parsers_report_chinese_schema_errors() {
    let wrong_struct_table = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_Other".to_owned(),
        package_path: "/Game/DT_Other".to_owned(),
        row_struct: "WrongStruct".to_owned(),
        rows: BTreeMap::new(),
    };
    let err = parse_chapters(&wrong_struct_table, "DT_Other.json").unwrap_err();
    assert!(matches!(err, DataError::UnexpectedRowStruct { .. }));
    assert!(err.to_string().contains("DT_Other.json"));
    assert!(err.to_string().contains("ChapterData"));

    let mut bad_rows = BTreeMap::new();
    bad_rows.insert("Ch1".to_owned(), json!({ "ChapterNo": "one", "Name": {} }));
    let bad_chapter_table = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_ChapterData".to_owned(),
        package_path: "/Game/DT_ChapterData".to_owned(),
        row_struct: "ChapterData".to_owned(),
        rows: bad_rows,
    };
    let ch_err = parse_chapters(&bad_chapter_table, "DT_ChapterData.json").unwrap_err();
    assert!(ch_err.to_string().contains("DT_ChapterData.json"));
    assert!(ch_err.to_string().contains("Rows.Ch1.ChapterNo"));

    let mut bad_main_rows = BTreeMap::new();
    bad_main_rows.insert(
        "Step_1010".to_owned(),
        json!({
            "Condition": "{progress:0}>=1010",
            "Text": { "CultureInvariantString": null },
            "SummaryText": { "CultureInvariantString": null },
            "DestinationList": [{ "PointType": "Goal" }]
        }),
    );
    let bad_main_table = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_MainScenario".to_owned(),
        package_path: "/Game/DT_MainScenario".to_owned(),
        row_struct: "ScenarioSection".to_owned(),
        rows: bad_main_rows,
    };
    let main_err = parse_main_scenario(&bad_main_table, "DT_MainScenario.json").unwrap_err();
    assert!(main_err.to_string().contains("DT_MainScenario.json"));
    assert!(
        main_err
            .to_string()
            .contains("Rows.Step_1010.DestinationList[0].DestinationKey")
    );
}
