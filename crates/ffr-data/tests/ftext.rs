use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ffr_data::{
    FTextValue, LocalizationCatalog, MODEL_LANGUAGE, NATIVE_LANGUAGE, ResolveOptions, TextResolver,
    TextResolverOptions, TextSource, parse_ftext, resolve_ftext, resolve_model_text,
};
use serde_json::json;
use tempfile::tempdir;

const CHAPTER_TABLE_ID: &str =
    "/Game/Datatable/Strings/ChapterTitle/ST_ChapterTitle.ST_ChapterTitle";

fn fixture_export() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rust/data/export")
        .canonicalize()
        .expect("fixture export directory exists")
}

#[test]
fn parse_ftext_recognizes_three_shapes_and_rejects_invalid_values() {
    assert_eq!(
        parse_ftext(&json!({
            "TableId": CHAPTER_TABLE_ID,
            "Key": "Title_001",
            "SourceString": "はじまりの森",
            "LocalizedString": "はじまりの森",
        })),
        Some(FTextValue::StringTable {
            table_id: CHAPTER_TABLE_ID.to_owned(),
            key: "Title_001".to_owned(),
            source_string: Some("はじまりの森".to_owned()),
            localized_string: Some("はじまりの森".to_owned()),
        })
    );
    assert_eq!(
        parse_ftext(&json!({ "CultureInvariantString": null })),
        Some(FTextValue::CultureInvariant { value: None })
    );
    assert_eq!(
        parse_ftext(&json!({ "CultureInvariantString": "HP 100" })),
        Some(FTextValue::CultureInvariant {
            value: Some("HP 100".to_owned()),
        })
    );
    assert_eq!(
        parse_ftext(&json!({
            "Namespace": "DemoInline",
            "Key": "Inline_001",
            "SourceString": "テスト",
            "LocalizedString": "テスト",
        })),
        Some(FTextValue::Inline {
            namespace: "DemoInline".to_owned(),
            key: "Inline_001".to_owned(),
            source_string: Some("テスト".to_owned()),
            localized_string: Some("テスト".to_owned()),
        })
    );
    assert_eq!(parse_ftext(&json!("plain string")), None);
    assert_eq!(parse_ftext(&json!({ "foo": 1 })), None);
    assert_eq!(
        parse_ftext(&json!({
            "CultureInvariantString": 123,
            "Namespace": "DemoInline",
            "Key": "Inline_001",
        })),
        None
    );
}

#[test]
fn resolver_defaults_to_zh_hans_and_prefers_locres_before_source() {
    let export = fixture_export();
    let resolver = TextResolver::new(&export);
    assert_eq!(resolver.language(), MODEL_LANGUAGE);
    assert_eq!(resolver.native_language(), NATIVE_LANGUAGE);

    // Omit SourceString to prove lookup reads exported Localization and StringTable files.
    let title1 = json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_001" });
    let title2 = json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_002" });

    let zh = resolver.resolve_value(&title1);
    assert_eq!(zh.text.as_deref(), Some("起始之森"));
    assert_eq!(zh.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(zh.requested_language, "zh-Hans");
    assert_eq!(zh.reason, None);
    assert_eq!(zh.source, TextSource::Localization);
    assert!(zh.is_found());
    assert!(!zh.is_fallback());

    let en_resolver = TextResolver::with_options(
        &export,
        TextResolverOptions {
            language: "en".to_owned(),
            ..TextResolverOptions::default()
        },
    );
    let en = en_resolver.resolve_value(&title1);
    assert_eq!(en.text.as_deref(), Some("Forest of Beginnings"));
    assert_eq!(en.lang.as_deref(), Some("en"));
    assert_eq!(en.source, TextSource::Localization);

    let ja_locres = resolver.resolve_value_with(&title1, Some("ja"), ResolveOptions::default());
    assert_eq!(ja_locres.text.as_deref(), Some("はじまりの森"));
    assert_eq!(ja_locres.lang.as_deref(), Some("ja"));
    assert_eq!(ja_locres.source, TextSource::Localization);

    let ja_source = resolver.resolve_value_with(&title2, Some("ja"), ResolveOptions::default());
    assert_eq!(ja_source.text.as_deref(), Some("さざなみの港"));
    assert_eq!(ja_source.lang.as_deref(), Some("ja"));
    assert_eq!(ja_source.reason, None);
    assert_eq!(ja_source.source, TextSource::StringTable);

    let one_shot = resolve_ftext(&title1, &export, None, ResolveOptions::default());
    assert_eq!(one_shot.text.as_deref(), Some("起始之森"));
    assert_eq!(one_shot.lang.as_deref(), Some("zh-Hans"));

    let parsed_title1 = parse_ftext(&title1).unwrap();
    let via_model = en_resolver.resolve_model_text(&parsed_title1);
    assert_eq!(via_model.text.as_deref(), Some("起始之森"));
    assert_eq!(via_model.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(via_model.requested_language, "zh-Hans");

    let one_shot_model = resolve_model_text(&title1, &export, ResolveOptions::default());
    assert_eq!(one_shot_model.text.as_deref(), Some("起始之森"));
}

#[test]
fn strict_mode_never_labels_japanese_as_target_language() {
    let resolver = TextResolver::new(fixture_export());

    let missing_key_ja = resolver.resolve_value_with(
        &json!({
            "TableId": CHAPTER_TABLE_ID,
            "Key": "Title_999",
            "SourceString": "不应该被回退返回",
        }),
        Some("ja"),
        ResolveOptions::default(),
    );
    assert_eq!(missing_key_ja.text, None);
    assert_eq!(missing_key_ja.lang, None);
    assert_eq!(missing_key_ja.source, TextSource::Missing);
    assert!(
        missing_key_ja
            .reason
            .as_deref()
            .unwrap()
            .contains("不存在键 Title_999")
    );

    let missing_key_zh = resolver.resolve_value_with(
        &json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_003" }),
        Some("zh-Hans"),
        ResolveOptions::default(),
    );
    assert_eq!(missing_key_zh.text, None);
    assert_eq!(missing_key_zh.lang, None);
    assert_eq!(missing_key_zh.requested_language, "zh-Hans");
    assert_eq!(missing_key_zh.source, TextSource::Missing);
    let reason = missing_key_zh.reason.as_deref().unwrap();
    assert!(reason.contains("zh-Hans") && reason.contains("ST_ChapterTitle/Title_003"));

    let missing_lang = resolver.resolve_value_with(
        &json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_001" }),
        Some("ko"),
        ResolveOptions::default(),
    );
    assert_eq!(missing_lang.text, None);
    assert_eq!(missing_lang.lang, None);
    assert_eq!(missing_lang.requested_language, "ko");
    assert_eq!(missing_lang.source, TextSource::Missing);
    assert!(
        missing_lang
            .reason
            .as_deref()
            .unwrap()
            .contains("缺少语言 ko")
    );

    let missing_table = resolver.resolve_value_with(
        &json!({ "TableId": "/Game/Datatable/Strings/NoSuch/ST_None.ST_None", "Key": "K1" }),
        Some("ja"),
        ResolveOptions::default(),
    );
    assert_eq!(missing_table.text, None);
    assert!(
        missing_table
            .reason
            .as_deref()
            .unwrap()
            .contains("找不到字符串表文件")
    );

    let bad_path = resolver.resolve_value_with(
        &json!({ "TableId": "/Engine/ST_Bad.ST_Bad", "Key": "K1" }),
        Some("ja"),
        ResolveOptions::default(),
    );
    assert_eq!(bad_path.text, None);
    assert!(
        bad_path
            .reason
            .as_deref()
            .unwrap()
            .contains("无法把字符串表路径映射到导出目录")
    );
}

#[test]
fn explicit_fallback_to_native_tags_true_language() {
    let export = fixture_export();
    let resolver = TextResolver::with_options(
        &export,
        TextResolverOptions {
            language: "zh-Hans".to_owned(),
            fallback_to_native: true,
            ..TextResolverOptions::default()
        },
    );

    let title3 =
        resolver.resolve_value(&json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_003" }));
    assert_eq!(title3.text.as_deref(), Some("しずかな塔"));
    assert_eq!(title3.lang.as_deref(), Some("ja"));
    assert_eq!(title3.requested_language, "zh-Hans");
    assert_eq!(title3.source, TextSource::StringTable);
    assert!(title3.is_fallback());
    assert!(
        title3
            .reason
            .as_deref()
            .unwrap()
            .contains("已回落到源语言 ja")
    );

    let ko = resolver.resolve_value_with(
        &json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_001" }),
        Some("ko"),
        ResolveOptions::default(),
    );
    assert_eq!(ko.text.as_deref(), Some("はじまりの森"));
    assert_eq!(ko.lang.as_deref(), Some("ja"));
    assert_eq!(ko.requested_language, "ko");
    assert_eq!(ko.source, TextSource::StringTable);

    // Per-call override can enable or disable fallback independently.
    let strict_resolver = TextResolver::new(&export);
    let per_call = strict_resolver.resolve_value_with(
        &json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_003" }),
        Some("zh-Hans"),
        ResolveOptions::with_fallback(),
    );
    assert_eq!(per_call.text.as_deref(), Some("しずかな塔"));
    assert_eq!(per_call.lang.as_deref(), Some("ja"));

    let per_call_strict = resolver.resolve_value_with(
        &json!({ "TableId": CHAPTER_TABLE_ID, "Key": "Title_003" }),
        Some("zh-Hans"),
        ResolveOptions::strict(),
    );
    assert_eq!(per_call_strict.text, None);
    assert_eq!(per_call_strict.source, TextSource::Missing);
}

#[test]
fn inline_invariant_preloaded_and_corrupt_locres_cases() {
    let export = fixture_export();
    let resolver = TextResolver::new(&export);

    let invariant = resolver.resolve_value_with(
        &json!({ "CultureInvariantString": "LV 99" }),
        Some("en"),
        ResolveOptions::default(),
    );
    assert_eq!(invariant.text.as_deref(), Some("LV 99"));
    assert_eq!(invariant.lang.as_deref(), Some("en"));
    assert_eq!(invariant.source, TextSource::CultureInvariant);

    let empty = resolver.resolve_value_with(
        &json!({ "CultureInvariantString": null }),
        Some("ja"),
        ResolveOptions::default(),
    );
    assert_eq!(empty.text, None);
    assert!(empty.reason.as_deref().unwrap().contains("为空"));

    let inline_ref = json!({
        "Namespace": "DemoInline",
        "Key": "Inline_001",
        "SourceString": "デモ内联",
    });
    let inline_zh =
        resolver.resolve_value_with(&inline_ref, Some("zh-Hans"), ResolveOptions::default());
    assert_eq!(inline_zh.text.as_deref(), Some("中文内联提示"));
    assert_eq!(inline_zh.source, TextSource::Localization);

    let inline_ja = resolver.resolve_value_with(&inline_ref, Some("ja"), ResolveOptions::default());
    assert_eq!(inline_ja.text.as_deref(), Some("デモ内联"));
    assert_eq!(inline_ja.source, TextSource::InlineSource);

    let inline_en_fb =
        resolver.resolve_value_with(&inline_ref, Some("en"), ResolveOptions::with_fallback());
    assert_eq!(inline_en_fb.text.as_deref(), Some("デモ内联"));
    assert_eq!(inline_en_fb.lang.as_deref(), Some("ja"));
    assert_eq!(inline_en_fb.requested_language, "en");
    assert_eq!(inline_en_fb.source, TextSource::InlineSource);

    // Preloaded in-memory catalog overrides disk lookup.
    let mut injected = HashMap::new();
    let mut bucket = BTreeMap::new();
    bucket.insert("Inline_001".to_owned(), "French Inline".to_owned());
    let mut namespaces = BTreeMap::new();
    namespaces.insert("DemoInline".to_owned(), bucket);
    injected.insert(
        "fr".to_owned(),
        Arc::new(LocalizationCatalog { namespaces }),
    );
    let preloaded = TextResolver::with_options(
        &export,
        TextResolverOptions {
            localizations: injected,
            ..TextResolverOptions::default()
        },
    );
    let fr = preloaded.resolve_value_with(&inline_ref, Some("fr"), ResolveOptions::default());
    assert_eq!(fr.text.as_deref(), Some("French Inline"));
    assert_eq!(fr.lang.as_deref(), Some("fr"));
    assert_eq!(fr.source, TextSource::Localization);

    // Corrupt ja.json must surface as an error rather than silently falling back.
    let temp = tempdir().unwrap();
    let loc_dir = temp.path().join("Localization");
    fs::create_dir_all(&loc_dir).unwrap();
    fs::write(loc_dir.join("ja.json"), "{ broken").unwrap();
    let bad_resolver = TextResolver::with_options(
        temp.path(),
        TextResolverOptions {
            language: "ja".to_owned(),
            ..TextResolverOptions::default()
        },
    );
    let bad = bad_resolver.resolve_value_with(&inline_ref, Some("ja"), ResolveOptions::default());
    assert_eq!(bad.text, None);
    assert_eq!(bad.source, TextSource::Missing);
    assert!(
        bad.reason
            .as_deref()
            .unwrap()
            .contains("加载语言 ja 的本地化表失败")
    );
}
