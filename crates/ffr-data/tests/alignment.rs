//! Alignment test ensuring Rust `ffr-data` produces identical chapter names, main scenario
//! entries, and `FText` resolutions as the TypeScript `src/data/` implementation against the
//! shared synthetic export fixture (`tests/fixtures/rust/data/export/`).

use std::fs;
use std::path::{Path, PathBuf};

use ffr_data::{
    Chapter, DataStore, MainScenarioEntry, ResolveOptions, ResolvedMainScenarioTexts, ResolvedText,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlignmentFixture {
    chapters: Vec<Chapter>,
    chapter_names: Vec<ChapterNameExpectation>,
    main_scenario_by_chapter: Vec<ChapterMainScenarioExpectation>,
    placeholder_entry: MainScenarioItemExpectation,
    ftext_cases: Vec<FTextExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterNameExpectation {
    chapter_no: i64,
    language: Option<String>,
    fallback_to_native: bool,
    expected: ResolvedText,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterMainScenarioExpectation {
    chapter_no: i64,
    entries: Vec<MainScenarioItemExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MainScenarioItemExpectation {
    entry: MainScenarioEntry,
    texts_default: ResolvedMainScenarioTexts,
    texts_en: Option<ResolvedMainScenarioTexts>,
    texts_ja: ResolvedMainScenarioTexts,
    texts_zh_fallback: Option<ResolvedMainScenarioTexts>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FTextExpectation {
    id: String,
    input: Value,
    language: Option<String>,
    fallback_to_native: bool,
    expected: ResolvedText,
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rust/data")
        .canonicalize()
        .expect("tests/fixtures/rust/data exists")
}

#[test]
fn rust_matches_typescript_on_chapters_scenario_and_ftext() {
    let dir = fixture_dir();
    let raw = fs::read_to_string(dir.join("expected-alignment.json"))
        .expect("read expected-alignment.json");
    let expected: AlignmentFixture =
        serde_json::from_str(&raw).expect("parse expected-alignment.json");

    let store = DataStore::new(dir.join("export"));
    let scenario = store.scenario().expect("load ScenarioData");

    // 1. Chapters match TS exactly.
    assert_eq!(scenario.chapters, expected.chapters);

    // 2. Chapter names across default, zh-Hans, en, ja, missing language, and fallbackToNative.
    for case in &expected.chapter_names {
        let actual = scenario.chapter_name_with(
            case.chapter_no,
            case.language.as_deref(),
            ResolveOptions {
                fallback_to_native: Some(case.fallback_to_native),
            },
        );
        assert_eq!(
            actual, case.expected,
            "chapter_name mismatch for chapter={} lang={:?} fallback={}",
            case.chapter_no, case.language, case.fallback_to_native
        );
    }

    // 3. Per-chapter main scenario entries, order, destinations, and resolved texts.
    for group in &expected.main_scenario_by_chapter {
        let actual_entries = scenario.chapter_main_scenario(group.chapter_no);
        assert_eq!(
            actual_entries.len(),
            group.entries.len(),
            "entry count mismatch for chapter {}",
            group.chapter_no
        );
        for (actual_entry, exp_item) in actual_entries.into_iter().zip(&group.entries) {
            assert_eq!(actual_entry, &exp_item.entry);
            assert_eq!(
                scenario.resolve_main_scenario_texts(actual_entry),
                exp_item.texts_default
            );
            if let Some(exp_en) = &exp_item.texts_en {
                assert_eq!(
                    scenario.resolve_main_scenario_texts_in(actual_entry, "en"),
                    *exp_en
                );
            }
            assert_eq!(
                scenario.resolve_main_scenario_texts_in(actual_entry, "ja"),
                exp_item.texts_ja
            );
            if let Some(exp_fb) = &exp_item.texts_zh_fallback {
                assert_eq!(
                    scenario.resolve_main_scenario_texts_with(
                        actual_entry,
                        Some("zh-Hans"),
                        ResolveOptions::with_fallback(),
                    ),
                    *exp_fb
                );
            }
        }
    }

    // 4. Placeholder NewRow entry and its resolved texts.
    let actual_placeholder = scenario
        .main_scenario
        .iter()
        .find(|entry| entry.row_key == expected.placeholder_entry.entry.row_key)
        .expect("placeholder row exists");
    assert_eq!(actual_placeholder, &expected.placeholder_entry.entry);
    assert_eq!(
        scenario.resolve_main_scenario_texts(actual_placeholder),
        expected.placeholder_entry.texts_default
    );
    assert_eq!(
        scenario.resolve_main_scenario_texts_in(actual_placeholder, "ja"),
        expected.placeholder_entry.texts_ja
    );

    // 5. All FText resolution cases (StringTable, inline, CultureInvariant, missing, fallback).
    for case in &expected.ftext_cases {
        let actual = store.text_resolver().resolve_value_with(
            &case.input,
            case.language.as_deref(),
            ResolveOptions {
                fallback_to_native: Some(case.fallback_to_native),
            },
        );
        assert_eq!(actual, case.expected, "FText mismatch on case {}", case.id);
    }
}
