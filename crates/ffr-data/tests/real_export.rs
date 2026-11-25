//! Smoke tests against the real unpacked export tree (`<FFR_DEPS or .deps>/ffr/export/`).
//!
//! Skips cleanly when unpacked game data or locres exports are absent (CI or fresh checkouts).
//! Asserts only structural invariants so no game text or row content enters the public repository.

use std::path::{Path, PathBuf};

use ffr_data::{
    DataStore, DataTableType, FFR_DEPS_ENV, ResolveOptions, TextSource, datatable_dir,
    export_dir_from_deps, localization_file,
};

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
fn real_export_scenario_and_composite_map_structure() {
    let export_root = real_export_dir();
    let chapter_file = datatable_dir(&export_root).join("Scenario/DT_ChapterData.json");
    if !chapter_file.is_file() {
        eprintln!(
            "跳过真实导出测试：未找到 {}（设置 FFR_DEPS 或先运行 ../final-fantasy-resonance-export/scripts/run.sh export）",
            chapter_file.display()
        );
        return;
    }

    let store = DataStore::new(&export_root);
    assert_eq!(store.cached_data_table_count(), 0);
    assert_eq!(store.cached_string_table_count(), 0);
    assert_eq!(store.cached_localization_count(), 0);

    let scenario = store.scenario().expect("load real ScenarioData");
    // Only the two Scenario tables were loaded on demand, never all 2618 files.
    assert_eq!(store.cached_data_table_count(), 2);

    assert_eq!(scenario.chapters.len(), 9);
    let chapter_nos: Vec<i64> = scenario.chapters.iter().map(|ch| ch.chapter_no).collect();
    assert_eq!(chapter_nos, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);

    let ch1_ja = scenario.chapter_name_in(1, "ja");
    assert_eq!(ch1_ja.lang.as_deref(), Some("ja"));
    assert!(matches!(
        ch1_ja.source,
        TextSource::Localization | TextSource::StringTable
    ));
    assert!(ch1_ja.text.as_deref().is_some_and(|text| !text.is_empty()));

    // Unexported language must report missing strictly, and tag lang="ja" with fallback enabled.
    let ch1_missing = scenario.chapter_name_in(1, "xx");
    assert_eq!(ch1_missing.text, None);
    assert_eq!(ch1_missing.lang, None);
    assert_eq!(ch1_missing.source, TextSource::Missing);
    assert!(ch1_missing.reason.as_deref().unwrap().contains("xx"));

    let ch1_fallback = scenario.chapter_name_with(1, Some("xx"), ResolveOptions::with_fallback());
    assert_eq!(ch1_fallback.lang.as_deref(), Some("ja"));
    assert_eq!(ch1_fallback.requested_language, "xx");
    assert_eq!(ch1_fallback.text, ch1_ja.text);

    assert_eq!(scenario.main_scenario.len(), 162);
    for chapter_no in 1..=8 {
        let entries = scenario.chapter_main_scenario(chapter_no);
        assert!(!entries.is_empty(), "第 {chapter_no} 章应有主线剧情条目");
        for entry in entries {
            let progress = entry.min_progress.expect("chapter entry has min_progress");
            assert!(progress >= chapter_no * 1000 && progress < (chapter_no + 1) * 1000);
        }
    }

    let first_ch1 = scenario.chapter_main_scenario(1)[0];
    let texts_ja = scenario.resolve_main_scenario_texts_in(first_ch1, "ja");
    assert!(
        texts_ja
            .text
            .text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
    );
    assert!(
        texts_ja
            .summary_text
            .text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
    );

    let map_table = store
        .data_table("Map/CDT_MapData_Prod")
        .expect("load CDT_MapData_Prod");
    assert_eq!(map_table.table_type, DataTableType::CompositeDataTable);
    assert_eq!(map_table.row_struct, "MAP_DATA");
    assert_eq!(map_table.len(), 186);
    assert_eq!(store.cached_data_table_count(), 3);
}

#[test]
fn real_export_multilingual_locres_resolution() {
    let export_root = real_export_dir();
    let chapter_file = datatable_dir(&export_root).join("Scenario/DT_ChapterData.json");
    let has_all = chapter_file.is_file()
        && localization_file(&export_root, "zh-Hans").is_file()
        && localization_file(&export_root, "en").is_file()
        && localization_file(&export_root, "ja").is_file();
    if !has_all {
        eprintln!(
            "跳过真实导出多语言测试：未找到完整的 Localization/{{zh-Hans,en,ja}}.json（可运行 ../final-fantasy-resonance-export/scripts/run.sh locres）"
        );
        return;
    }

    let store = DataStore::new(&export_root);
    let scenario = store.scenario().expect("load real ScenarioData");

    let ch1_zh = scenario.chapter_name(1);
    let ch1_en = scenario.chapter_name_in(1, "en");
    let ch1_ja = scenario.chapter_name_in(1, "ja");

    assert_eq!(ch1_zh.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(ch1_zh.source, TextSource::Localization);
    assert!(ch1_zh.text.as_deref().is_some_and(|text| !text.is_empty()));

    assert_eq!(ch1_en.lang.as_deref(), Some("en"));
    assert_eq!(ch1_en.source, TextSource::Localization);
    assert!(ch1_en.text.as_deref().is_some_and(|text| !text.is_empty()));

    assert_eq!(ch1_ja.lang.as_deref(), Some("ja"));
    assert_eq!(ch1_ja.source, TextSource::Localization);
    assert!(ch1_ja.text.as_deref().is_some_and(|text| !text.is_empty()));

    // Confirm translations are genuinely distinct from Japanese source text.
    assert_ne!(ch1_zh.text, ch1_ja.text);
    assert_ne!(ch1_en.text, ch1_ja.text);

    let first_ch1 = scenario.chapter_main_scenario(1)[0];
    let model_text = store.text_resolver().resolve_model_text(&first_ch1.text);
    assert_eq!(model_text.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(model_text.source, TextSource::Localization);
    assert!(
        model_text
            .text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
    );
}
