use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ffr_data::{
    FTextValue, GB2312_HANZI, LocalizationCatalog, MODEL_LANGUAGE, NATIVE_LANGUAGE, ResolveOptions,
    TextResolver, TextResolverOptions, TextSource, UNTRANSLATED_REASON, UNTRANSLATED_REASON_CJK,
    UNTRANSLATED_REASON_KANA, UNTRANSLATED_REASON_NON_SIMPLIFIED_HANZI, is_cjk_ideograph,
    is_gb2312_hanzi, is_japanese_kana, is_untranslated_entry, parse_ftext, resolve_ftext,
    resolve_model_text, untranslated_reason_code,
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

#[test]
fn untranslated_detection_rules_cover_kana_pure_hanzi_and_translated_cases() {
    assert!(is_japanese_kana('あ'));
    assert!(is_japanese_kana('ア'));
    assert!(is_japanese_kana('ー'));
    assert!(is_japanese_kana('ｱ'));
    assert!(!is_japanese_kana('・'));
    assert!(!is_japanese_kana('白'));
    assert!(is_cjk_ideograph('白'));
    assert!(!is_cjk_ideograph('あ'));

    // 1. Same as ja and contains Japanese kana -> untranslated for both zh-Hans and en.
    assert!(is_untranslated_entry(
        "zh-Hans",
        "テスト未翻訳の技",
        Some("テスト未翻訳の技")
    ));
    assert_eq!(
        untranslated_reason_code("zh-Hans", "テスト未翻訳の技", Some("テスト未翻訳の技")),
        Some(UNTRANSLATED_REASON_KANA)
    );
    assert!(is_untranslated_entry(
        "en",
        "テスト未翻訳の技",
        Some("テスト未翻訳の技")
    ));
    assert_eq!(
        untranslated_reason_code("en", "テスト未翻訳の技", Some("テスト未翻訳の技")),
        Some(UNTRANSLATED_REASON_KANA)
    );
    assert!(!is_untranslated_entry(
        "ja",
        "テスト未翻訳の技",
        Some("テスト未翻訳の技")
    ));

    // 2. Same as ja, pure Hanzi -> valid Chinese, but untranslated for English.
    assert!(!is_untranslated_entry("zh-Hans", "白虎", Some("白虎")));
    assert_eq!(
        untranslated_reason_code("zh-Hans", "白虎", Some("白虎")),
        None
    );
    assert!(!is_untranslated_entry(
        "zh-Hans",
        "魔法回避・小",
        Some("魔法回避・小")
    ));
    assert!(is_untranslated_entry("en", "白虎", Some("白虎")));
    assert_eq!(
        untranslated_reason_code("en", "白虎", Some("白虎")),
        Some(UNTRANSLATED_REASON_CJK)
    );
    assert!(!is_untranslated_entry("en", "HP 100", Some("HP 100")));

    // 3. Different from ja (or ja absent) -> never marked untranslated.
    assert!(!is_untranslated_entry("zh-Hans", "钢蝠", Some("鋼の蝙蝠")));
    assert!(!is_untranslated_entry("en", "Steel bat", Some("鋼の蝙蝠")));
    assert!(!is_untranslated_entry(
        "zh-Hans",
        "テスト未翻訳の技",
        Some("別の文章")
    ));
    assert!(!is_untranslated_entry("zh-Hans", "テスト未翻訳の技", None));
}

#[test]
fn gb2312_hanzi_table_has_exact_size_strict_order_and_expected_membership() {
    assert_eq!(GB2312_HANZI.len(), 6_763);
    assert!(
        GB2312_HANZI.windows(2).all(|pair| pair[0] < pair[1]),
        "GB2312_HANZI must be strictly ascending for binary search"
    );
    for ch in ['剑', '贼', '骑', '术', '爱'] {
        assert!(
            is_gb2312_hanzi(ch),
            "expected simplified Hanzi '{ch}' to be in GB2312"
        );
    }
    for ch in ['剣', '賊', '騎', '術', '愛'] {
        assert!(
            !is_gb2312_hanzi(ch),
            "expected Japanese/non-GB2312 Hanzi '{ch}' to be absent from GB2312"
        );
    }
}

#[test]
fn simplified_chinese_flags_non_gb2312_hanzi_same_as_ja_and_keeps_resolver_aligned() {
    // Same as ja, no kana, contains non-GB2312 Hanzi ('剣') -> untranslated for zh-Hans / zh-CN.
    assert!(is_untranslated_entry("zh-Hans", "剣技", Some("剣技")));
    assert_eq!(
        untranslated_reason_code("zh-Hans", "剣技", Some("剣技")),
        Some(UNTRANSLATED_REASON_NON_SIMPLIFIED_HANZI)
    );
    assert_eq!(
        untranslated_reason_code("zh-CN", "剣技", Some("剣技")),
        Some(UNTRANSLATED_REASON_NON_SIMPLIFIED_HANZI)
    );
    assert_eq!(
        untranslated_reason_code("zh-Hans-CN", "剣技", Some("剣技")),
        Some(UNTRANSLATED_REASON_NON_SIMPLIFIED_HANZI)
    );
    // Traditional Chinese tags are not governed by the GB2312 simplified-Hanzi rule.
    assert_eq!(
        untranslated_reason_code("zh-Hant", "剣技", Some("剣技")),
        None
    );
    assert_eq!(
        untranslated_reason_code("zh-TW", "剣技", Some("剣技")),
        None
    );

    // Same as ja, all GB2312 simplified Hanzi -> remains a valid Chinese translation.
    assert!(!is_untranslated_entry("zh-Hans", "兵法", Some("兵法")));
    assert_eq!(
        untranslated_reason_code("zh-Hans", "兵法", Some("兵法")),
        None
    );

    // Different from ja -> translated.
    assert!(!is_untranslated_entry("zh-Hans", "剑技", Some("剣技")));
    assert_eq!(
        untranslated_reason_code("zh-Hans", "剑技", Some("剣技")),
        None
    );

    // Kana and English rules stay unchanged.
    assert_eq!(
        untranslated_reason_code("zh-Hans", "剣の技", Some("剣の技")),
        Some(UNTRANSLATED_REASON_KANA)
    );
    assert_eq!(
        untranslated_reason_code("en", "剣技", Some("剣技")),
        Some(UNTRANSLATED_REASON_CJK)
    );
    assert_eq!(
        untranslated_reason_code("en", "兵法", Some("兵法")),
        Some(UNTRANSLATED_REASON_CJK)
    );

    // Verify TextResolver::untranslated_reason matches untranslated_reason_code.
    let temp = tempdir().expect("tempdir");
    let loc_dir = temp.path().join("Localization");
    fs::create_dir_all(&loc_dir).expect("create Localization dir");
    fs::write(
        loc_dir.join("ja.json"),
        serde_json::to_string_pretty(&json!({
            "DemoSkill": {
                "KanjiUntrans": "剣技",
                "SameSimplified": "兵法",
                "Translated": "剣技"
            }
        }))
        .expect("serialize ja"),
    )
    .expect("write ja.json");
    fs::write(
        loc_dir.join("zh-Hans.json"),
        serde_json::to_string_pretty(&json!({
            "DemoSkill": {
                "KanjiUntrans": "剣技",
                "SameSimplified": "兵法",
                "Translated": "剑技"
            }
        }))
        .expect("serialize zh-Hans"),
    )
    .expect("write zh-Hans.json");

    let resolver = TextResolver::new(temp.path());
    let inline = |key: &str| FTextValue::Inline {
        namespace: "DemoSkill".to_owned(),
        key: key.to_owned(),
        source_string: None,
        localized_string: None,
    };

    let kanji_untrans = inline("KanjiUntrans");
    let strict = resolver.resolve_in(&kanji_untrans, "zh-Hans");
    assert_eq!(strict.text, None);
    assert_eq!(strict.source, TextSource::Missing);
    assert_eq!(strict.reason.as_deref(), Some(UNTRANSLATED_REASON));
    let fallback = resolver.resolve_with(
        &kanji_untrans,
        Some("zh-Hans"),
        ResolveOptions::with_fallback(),
    );
    assert_eq!(fallback.text.as_deref(), Some("剣技"));
    assert_eq!(fallback.lang.as_deref(), Some("ja"));
    assert_eq!(
        resolver.untranslated_entry(&kanji_untrans, "zh-Hans"),
        Some((
            "DemoSkill".to_owned(),
            "KanjiUntrans".to_owned(),
            "剣技".to_owned(),
            UNTRANSLATED_REASON_NON_SIMPLIFIED_HANZI,
        ))
    );

    let same_simplified = inline("SameSimplified");
    let resolved_same = resolver.resolve_in(&same_simplified, "zh-Hans");
    assert_eq!(resolved_same.text.as_deref(), Some("兵法"));
    assert_eq!(resolved_same.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(
        resolver.untranslated_entry(&same_simplified, "zh-Hans"),
        None
    );

    let translated = inline("Translated");
    let resolved_tr = resolver.resolve_in(&translated, "zh-Hans");
    assert_eq!(resolved_tr.text.as_deref(), Some("剑技"));
    assert_eq!(resolved_tr.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(resolver.untranslated_entry(&translated, "zh-Hans"), None);
}

#[test]
fn text_resolver_treats_untranslated_entries_as_missing_and_falls_back_to_ja() {
    let text_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rust/db/text/export")
        .canonicalize()
        .expect("db text fixture export exists");
    let resolver = TextResolver::new(&text_fixture);
    let table_id = "/Game/Datatable/Strings/Fake/ST_Fake.ST_Fake";

    // Case 1: same as ja + kana -> strict missing with exact Chinese reason, fallback -> lang="ja".
    let untrans_st = FTextValue::StringTable {
        table_id: table_id.to_owned(),
        key: "UntranslatedKana".to_owned(),
        source_string: None,
        localized_string: None,
    };
    for lang in ["zh-Hans", "en"] {
        let strict = resolver.resolve_with(&untrans_st, Some(lang), ResolveOptions::strict());
        assert_eq!(strict.text, None);
        assert_eq!(strict.lang, None);
        assert_eq!(strict.requested_language, lang);
        assert_eq!(strict.source, TextSource::Missing);
        assert_eq!(strict.reason.as_deref(), Some(UNTRANSLATED_REASON));

        let fallback =
            resolver.resolve_with(&untrans_st, Some(lang), ResolveOptions::with_fallback());
        assert_eq!(fallback.text.as_deref(), Some("テスト未翻訳の技"));
        assert_eq!(fallback.lang.as_deref(), Some("ja"));
        assert_eq!(fallback.requested_language, lang);
        assert_eq!(fallback.source, TextSource::Localization);
        assert!(fallback.is_fallback());
        assert!(
            fallback
                .reason
                .as_deref()
                .unwrap()
                .contains(UNTRANSLATED_REASON)
        );
    }
    let ja_direct = resolver.resolve_in(&untrans_st, "ja");
    assert_eq!(ja_direct.text.as_deref(), Some("テスト未翻訳の技"));
    assert_eq!(ja_direct.lang.as_deref(), Some("ja"));
    assert_eq!(ja_direct.reason, None);

    // Case 2: pure Hanzi same as ja -> valid zh-Hans, untranslated en.
    let kanji_st = FTextValue::StringTable {
        table_id: table_id.to_owned(),
        key: "SameKanjiName".to_owned(),
        source_string: None,
        localized_string: None,
    };
    let kanji_zh = resolver.resolve_in(&kanji_st, "zh-Hans");
    assert_eq!(kanji_zh.text.as_deref(), Some("白虎"));
    assert_eq!(kanji_zh.lang.as_deref(), Some("zh-Hans"));
    assert_eq!(kanji_zh.reason, None);
    assert_eq!(kanji_zh.source, TextSource::Localization);

    let kanji_en_strict = resolver.resolve_in(&kanji_st, "en");
    assert_eq!(kanji_en_strict.text, None);
    assert_eq!(kanji_en_strict.lang, None);
    assert_eq!(kanji_en_strict.source, TextSource::Missing);
    assert_eq!(kanji_en_strict.reason.as_deref(), Some(UNTRANSLATED_REASON));

    let kanji_en_fb = resolver.resolve_with(&kanji_st, Some("en"), ResolveOptions::with_fallback());
    assert_eq!(kanji_en_fb.text.as_deref(), Some("白虎"));
    assert_eq!(kanji_en_fb.lang.as_deref(), Some("ja"));
    assert!(kanji_en_fb.is_fallback());

    // Case 3: inline untranslated without SourceString still falls back to ja locres cleanly.
    let untrans_inline = FTextValue::Inline {
        namespace: "InlineNs".to_owned(),
        key: "UntranslatedInline".to_owned(),
        source_string: None,
        localized_string: None,
    };
    let inline_strict = resolver.resolve_in(&untrans_inline, "zh-Hans");
    assert_eq!(inline_strict.text, None);
    assert_eq!(inline_strict.source, TextSource::Missing);
    assert_eq!(inline_strict.reason.as_deref(), Some(UNTRANSLATED_REASON));

    let inline_fb = resolver.resolve_with(
        &untrans_inline,
        Some("zh-Hans"),
        ResolveOptions::with_fallback(),
    );
    assert_eq!(inline_fb.text.as_deref(), Some("仮の未翻訳メッセージ"));
    assert_eq!(inline_fb.lang.as_deref(), Some("ja"));
    assert_eq!(inline_fb.source, TextSource::Localization);

    // `untranslated_entry` exposes (ns, key, raw_value, reason_code) for companion-table building.
    assert_eq!(
        resolver.untranslated_entry(&untrans_st, "zh-Hans"),
        Some((
            "FakeTextNamespace".to_owned(),
            "UntranslatedKana".to_owned(),
            "テスト未翻訳の技".to_owned(),
            UNTRANSLATED_REASON_KANA,
        ))
    );
    assert_eq!(
        resolver.untranslated_entry(&untrans_st, "en"),
        Some((
            "FakeTextNamespace".to_owned(),
            "UntranslatedKana".to_owned(),
            "テスト未翻訳の技".to_owned(),
            UNTRANSLATED_REASON_KANA,
        ))
    );
    assert_eq!(resolver.untranslated_entry(&untrans_st, "ja"), None);
    assert_eq!(resolver.untranslated_entry(&kanji_st, "zh-Hans"), None);
    assert_eq!(
        resolver.untranslated_entry(&kanji_st, "en"),
        Some((
            "FakeTextNamespace".to_owned(),
            "SameKanjiName".to_owned(),
            "白虎".to_owned(),
            UNTRANSLATED_REASON_CJK,
        ))
    );
    assert_eq!(
        resolver.untranslated_entry(&untrans_inline, "zh-Hans"),
        Some((
            "InlineNs".to_owned(),
            "UntranslatedInline".to_owned(),
            "仮の未翻訳メッセージ".to_owned(),
            UNTRANSLATED_REASON_KANA,
        ))
    );
}
