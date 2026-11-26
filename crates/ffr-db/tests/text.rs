use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ffr_data::world::WorldData;
use ffr_data::{
    DataStore, FTextValue, ResolveOptions, TextResolver, TextSource, UNTRANSLATED_REASON,
    UNTRANSLATED_REASON_CJK, UNTRANSLATED_REASON_KANA, UNTRANSLATED_REASON_NON_SIMPLIFIED_HANZI,
    untranslated_reason_code,
};
use ffr_db::query::text::sql as text_sql;
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db, build_fixture_db_stages, build_temp_db,
    fixture_area_dir,
};
use ffr_db::{BuildInputs, Stage, UntranslatedField, UntranslatedTextEntry};
use rusqlite::params;

const TABLE_ID: &str = "/Game/Datatable/Strings/Fake/ST_Fake.ST_Fake";

#[test]
fn localized_lookup_and_search_use_indexes() {
    let test_db =
        build_fixture_db_stages("text", Some(vec![Stage::Text])).expect("build text fixture");
    let db = test_db.db();

    let text_stage = test_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "text")
        .expect("text stage report");
    assert!(
        text_stage
            .note
            .contains("text_untranslated=5(en=3, zh-Hans=2)"),
        "stage_report.note missing text_untranslated counts: {}",
        text_stage.note
    );

    // Schema v4: `text` has one row per (ns, key) with `ja`, `en`, `zh` columns and no
    // `untranslated` column; `text_language` maps BCP-47 tags to column names.
    let text_cols: Vec<String> = db
        .query_readonly_json("SELECT name FROM pragma_table_info('text') ORDER BY cid")
        .expect("pragma_table_info(text)")
        .into_iter()
        .map(|row| row["name"].as_str().expect("col name").to_owned())
        .collect();
    assert_eq!(text_cols, vec!["id", "ns", "key", "ja", "en", "zh"]);

    let lang_map: Vec<(String, String)> = db
        .query_readonly_json("SELECT lang, column_name FROM text_language ORDER BY lang")
        .expect("query text_language")
        .into_iter()
        .map(|row| {
            (
                row["lang"].as_str().expect("lang").to_owned(),
                row["column_name"].as_str().expect("column_name").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        lang_map,
        vec![
            ("en".to_owned(), "en".to_owned()),
            ("ja".to_owned(), "ja".to_owned()),
            ("zh-Hans".to_owned(), "zh".to_owned()),
        ]
    );

    assert_eq!(
        db.text("FakeTextNamespace", "Greeting", "zh-Hans")
            .expect("zh text"),
        Some("钢蝠".to_owned())
    );
    assert_eq!(
        db.text("FakeTextNamespace", "Greeting", "en")
            .expect("en text"),
        Some("Steel bat".to_owned())
    );
    assert_eq!(
        db.text("FakeTextNamespace", "Missing", "zh-Hans")
            .expect("missing text"),
        None
    );

    // Untranslated entries are excluded from `text` and placed in `text_untranslated`.
    assert_eq!(
        db.text("FakeTextNamespace", "UntranslatedKana", "zh-Hans")
            .expect("untranslated zh text"),
        None
    );
    assert_eq!(
        db.text("FakeTextNamespace", "UntranslatedKana", "en")
            .expect("untranslated en text"),
        None
    );
    assert_eq!(
        db.text("FakeTextNamespace", "UntranslatedKana", "ja")
            .expect("ja text"),
        Some("テスト未翻訳の技".to_owned())
    );
    // Pure Hanzi identical to ja is kept in `text` for zh-Hans, moved to `text_untranslated` for en.
    assert_eq!(
        db.text("FakeTextNamespace", "SameKanjiName", "zh-Hans")
            .expect("same kanji zh text"),
        Some("白虎".to_owned())
    );
    assert_eq!(
        db.text("FakeTextNamespace", "SameKanjiName", "en")
            .expect("same kanji en text"),
        None
    );
    assert_eq!(
        db.text("FakeTextNamespace", "SameKanjiName", "ja")
            .expect("same kanji ja text"),
        Some("白虎".to_owned())
    );

    assert_eq!(
        db.text_untranslated("FakeTextNamespace", "Greeting", "zh-Hans")
            .expect("translated greeting not in text_untranslated"),
        None
    );
    assert_eq!(
        db.text_untranslated("FakeTextNamespace", "UntranslatedKana", "zh-Hans")
            .expect("untranslated kana zh"),
        Some(UntranslatedTextEntry {
            ns: "FakeTextNamespace".to_owned(),
            key: "UntranslatedKana".to_owned(),
            lang: "zh-Hans".to_owned(),
            value: "テスト未翻訳の技".to_owned(),
            reason: UNTRANSLATED_REASON_KANA.to_owned(),
        })
    );
    assert_eq!(
        db.text_untranslated("FakeTextNamespace", "UntranslatedKana", "en")
            .expect("untranslated kana en"),
        Some(UntranslatedTextEntry {
            ns: "FakeTextNamespace".to_owned(),
            key: "UntranslatedKana".to_owned(),
            lang: "en".to_owned(),
            value: "テスト未翻訳の技".to_owned(),
            reason: UNTRANSLATED_REASON_KANA.to_owned(),
        })
    );
    assert_eq!(
        db.text_untranslated("FakeTextNamespace", "SameKanjiName", "zh-Hans")
            .expect("pure hanzi zh not untranslated"),
        None
    );
    assert_eq!(
        db.text_untranslated("FakeTextNamespace", "SameKanjiName", "en")
            .expect("pure hanzi en untranslated"),
        Some(UntranslatedTextEntry {
            ns: "FakeTextNamespace".to_owned(),
            key: "SameKanjiName".to_owned(),
            lang: "en".to_owned(),
            value: "白虎".to_owned(),
            reason: UNTRANSLATED_REASON_CJK.to_owned(),
        })
    );
    assert_eq!(
        db.untranslated_texts("zh-Hans")
            .expect("all untranslated zh-Hans"),
        vec![
            UntranslatedTextEntry {
                ns: "FakeTextNamespace".to_owned(),
                key: "UntranslatedKana".to_owned(),
                lang: "zh-Hans".to_owned(),
                value: "テスト未翻訳の技".to_owned(),
                reason: UNTRANSLATED_REASON_KANA.to_owned(),
            },
            UntranslatedTextEntry {
                ns: "InlineNs".to_owned(),
                key: "UntranslatedInline".to_owned(),
                lang: "zh-Hans".to_owned(),
                value: "仮の未翻訳メッセージ".to_owned(),
                reason: UNTRANSLATED_REASON_KANA.to_owned(),
            },
        ]
    );
    assert_eq!(
        db.untranslated_texts("en").expect("all untranslated en"),
        vec![
            UntranslatedTextEntry {
                ns: "FakeTextNamespace".to_owned(),
                key: "SameKanjiName".to_owned(),
                lang: "en".to_owned(),
                value: "白虎".to_owned(),
                reason: UNTRANSLATED_REASON_CJK.to_owned(),
            },
            UntranslatedTextEntry {
                ns: "FakeTextNamespace".to_owned(),
                key: "UntranslatedKana".to_owned(),
                lang: "en".to_owned(),
                value: "テスト未翻訳の技".to_owned(),
                reason: UNTRANSLATED_REASON_KANA.to_owned(),
            },
            UntranslatedTextEntry {
                ns: "InlineNs".to_owned(),
                key: "UntranslatedInline".to_owned(),
                lang: "en".to_owned(),
                value: "仮の未翻訳メッセージ".to_owned(),
                reason: UNTRANSLATED_REASON_KANA.to_owned(),
            },
        ]
    );
    assert!(
        db.untranslated_texts("ja")
            .expect("ja never untranslated")
            .is_empty()
    );

    let all = db
        .text_all_langs("FakeTextNamespace", "Greeting")
        .expect("all languages");
    assert_eq!(
        all,
        vec![
            ("en".to_owned(), "Steel bat".to_owned()),
            ("ja".to_owned(), "鋼の蝙蝠".to_owned()),
            ("zh-Hans".to_owned(), "钢蝠".to_owned()),
        ]
    );
    assert_eq!(
        db.text_all_langs("FakeTextNamespace", "UntranslatedKana")
            .expect("untranslated all langs"),
        vec![("ja".to_owned(), "テスト未翻訳の技".to_owned())]
    );
    assert_eq!(
        db.text_all_langs("FakeTextNamespace", "SameKanjiName")
            .expect("same kanji all langs"),
        vec![
            ("ja".to_owned(), "白虎".to_owned()),
            ("zh-Hans".to_owned(), "白虎".to_owned()),
        ]
    );

    let short = db
        .search_text("钢蝠", Some("zh-Hans"), 20)
        .expect("two-character LIKE search");
    assert_eq!(short.len(), 1);
    assert_eq!(short[0].key, "Greeting");
    let long = db
        .search_text("钢铁蝙", Some("zh-Hans"), 20)
        .expect("three-character FTS search");
    assert_eq!(long.len(), 2);
    assert!(long.iter().any(|hit| hit.key == "LongText"));
    assert!(long.iter().any(|hit| hit.key == "InlineKey"));
    assert_eq!(
        db.search_text("deep", Some("en"), 20)
            .expect("English FTS search")
            .len(),
        1
    );
    let literal = db
        .search_text("%_", Some("zh-Hans"), 20)
        .expect("LIKE metacharacters are literal");
    assert_eq!(literal.len(), 1);
    assert_eq!(literal[0].key, "Literal");

    // `text_fts` and `search_text` only index `text` (translated entries + native `ja`).
    let fts_untrans = db
        .search_text("未翻訳の", None, 20)
        .expect("FTS search for untranslated text");
    assert_eq!(fts_untrans.len(), 1);
    assert_eq!(fts_untrans[0].key, "UntranslatedKana");
    assert_eq!(fts_untrans[0].lang, "ja");
    assert!(
        db.search_text("未翻訳の", Some("zh-Hans"), 20)
            .expect("zh-Hans search excludes untranslated")
            .is_empty()
    );
    let like_kanji = db
        .search_text("白虎", None, 20)
        .expect("LIKE search for same-kanji name");
    assert_eq!(like_kanji.len(), 2);
    assert_eq!(
        like_kanji
            .iter()
            .map(|h| h.lang.as_str())
            .collect::<Vec<_>>(),
        vec!["ja", "zh-Hans"]
    );

    assert!(
        db.search_text("", None, 20)
            .expect("empty query")
            .is_empty()
    );
    assert!(
        db.search_text("钢蝠", None, 0)
            .expect("zero limit")
            .is_empty()
    );

    assert_db_uses_index(
        db,
        text_sql::TEXT_BY_KEY_JA,
        params!["FakeTextNamespace", "Greeting"],
        &["text"],
    );
    assert_db_uses_index(
        db,
        text_sql::TEXT_BY_KEY_EN,
        params!["FakeTextNamespace", "Greeting"],
        &["text"],
    );
    assert_db_uses_index(
        db,
        text_sql::TEXT_BY_KEY_ZH,
        params!["FakeTextNamespace", "Greeting"],
        &["text"],
    );
    assert_db_uses_index(
        db,
        text_sql::TEXT_ALL_LANGS,
        params!["FakeTextNamespace", "Greeting"],
        &["text"],
    );
    assert_db_uses_index(
        db,
        "SELECT ns, key, lang, value, reason FROM text_untranslated \
         WHERE ns = ?1 AND key = ?2 AND lang = ?3",
        params!["FakeTextNamespace", "UntranslatedKana", "zh-Hans"],
        &["text_untranslated"],
    );
    assert_db_uses_index(
        db,
        "SELECT ns, key, lang, value, reason FROM text_untranslated \
         WHERE lang = ?1 ORDER BY ns, key",
        ["zh-Hans"],
        &["text_untranslated"],
    );
    assert_db_uses_index(
        db,
        "SELECT value FROM text_untranslated WHERE ns = ?1 AND key = ?2 AND lang = ?3",
        params!["FakeTextNamespace", "UntranslatedKana", "zh-Hans"],
        &["text_untranslated"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_FTS_JA,
        params!["ja : \"未翻訳\"", 20_i64],
        &["text_fts", "text"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_FTS_EN,
        params!["en : \"deep\"", 20_i64],
        &["text_fts", "text"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_FTS_ZH,
        params!["zh : \"钢铁蝙\"", 20_i64],
        &["text_fts", "text"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_FTS_ALL,
        params!["\"钢铁蝙\"", 20_i64],
        &["text_fts", "text"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_LIKE_JA,
        params!["%白虎%", 20_i64],
        &["text"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_LIKE_EN,
        params!["%ba%", 20_i64],
        &["text"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_LIKE_ZH,
        params!["%钢蝠%", 20_i64],
        &["text"],
    );
    assert_db_uses_index(
        db,
        text_sql::SEARCH_TEXT_LIKE_ALL,
        params!["%钢蝠%", 20_i64],
        &["text"],
    );
}

#[test]
fn resolve_ftext_matches_text_resolver_for_every_shape_and_branch() {
    let test_db =
        build_fixture_db_stages("text", Some(vec![Stage::Text])).expect("build text fixture");
    let db = test_db.db();
    let export = fixture_area_dir("text").join("export");
    let resolver = TextResolver::new(export);
    let values = vec![
        FTextValue::StringTable {
            table_id: TABLE_ID.to_owned(),
            key: "Greeting".to_owned(),
            source_string: None,
            localized_string: None,
        },
        FTextValue::StringTable {
            table_id: TABLE_ID.to_owned(),
            key: "UntranslatedKana".to_owned(),
            source_string: None,
            localized_string: None,
        },
        FTextValue::StringTable {
            table_id: TABLE_ID.to_owned(),
            key: "SameKanjiName".to_owned(),
            source_string: None,
            localized_string: None,
        },
        FTextValue::StringTable {
            table_id: TABLE_ID.to_owned(),
            key: "SourceOnly".to_owned(),
            source_string: Some("不应优先使用".to_owned()),
            localized_string: None,
        },
        FTextValue::StringTable {
            table_id: TABLE_ID.to_owned(),
            key: "MissingKey".to_owned(),
            source_string: Some("也不能使用".to_owned()),
            localized_string: None,
        },
        FTextValue::StringTable {
            table_id: "/Game/Datatable/Strings/Fake/ST_Missing.ST_Missing".to_owned(),
            key: "Key".to_owned(),
            source_string: None,
            localized_string: None,
        },
        FTextValue::StringTable {
            table_id: "/Engine/ST_Bad.ST_Bad".to_owned(),
            key: "Key".to_owned(),
            source_string: None,
            localized_string: None,
        },
        FTextValue::Inline {
            namespace: "InlineNs".to_owned(),
            key: "InlineKey".to_owned(),
            source_string: Some("内联ソース".to_owned()),
            localized_string: None,
        },
        FTextValue::Inline {
            namespace: "InlineNs".to_owned(),
            key: "UntranslatedInline".to_owned(),
            source_string: None,
            localized_string: None,
        },
        FTextValue::Inline {
            namespace: "InlineNs".to_owned(),
            key: "UntranslatedInline".to_owned(),
            source_string: Some("仮の未翻訳メッセージ".to_owned()),
            localized_string: None,
        },
        FTextValue::Inline {
            namespace: "InlineNs".to_owned(),
            key: "MissingInline".to_owned(),
            source_string: Some("缺失翻译的源文".to_owned()),
            localized_string: None,
        },
        FTextValue::Inline {
            namespace: String::new(),
            key: "MissingWithoutSource".to_owned(),
            source_string: None,
            localized_string: None,
        },
        FTextValue::CultureInvariant {
            value: Some("LV 99".to_owned()),
        },
        FTextValue::CultureInvariant { value: None },
    ];

    for value in &values {
        for language in ["zh-Hans", "en", "ja", "ko", "bad/lang"] {
            for options in [ResolveOptions::default(), ResolveOptions::with_fallback()] {
                let expected = resolver.resolve_with(value, Some(language), options);
                let actual = db
                    .resolve_ftext(value, language, options)
                    .expect("database resolution");
                assert_eq!(
                    actual, expected,
                    "value={value:?}, language={language}, options={options:?}"
                );
            }
        }
    }

    // Explicit checks for untranslated strict-missing and ja-fallback semantics.
    let untrans_st = &values[1];
    let strict_zh = db
        .resolve_ftext(untrans_st, "zh-Hans", ResolveOptions::default())
        .expect("strict untranslated zh");
    assert_eq!(strict_zh.text, None);
    assert_eq!(strict_zh.lang, None);
    assert_eq!(strict_zh.source, TextSource::Missing);
    assert_eq!(strict_zh.reason.as_deref(), Some(UNTRANSLATED_REASON));

    let fb_zh = db
        .resolve_ftext(untrans_st, "zh-Hans", ResolveOptions::with_fallback())
        .expect("fallback untranslated zh");
    assert_eq!(fb_zh.text.as_deref(), Some("テスト未翻訳の技"));
    assert_eq!(fb_zh.lang.as_deref(), Some("ja"));
    assert_eq!(fb_zh.source, TextSource::Localization);
    assert!(fb_zh.is_fallback());

    let kanji_st = &values[2];
    let kanji_zh = db
        .resolve_ftext(kanji_st, "zh-Hans", ResolveOptions::default())
        .expect("same kanji zh");
    assert_eq!(kanji_zh.text.as_deref(), Some("白虎"));
    assert_eq!(kanji_zh.lang.as_deref(), Some("zh-Hans"));
    let kanji_en = db
        .resolve_ftext(kanji_st, "en", ResolveOptions::default())
        .expect("same kanji en strict");
    assert_eq!(kanji_en.text, None);
    assert_eq!(kanji_en.reason.as_deref(), Some(UNTRANSLATED_REASON));
    let kanji_en_fb = db
        .resolve_ftext(kanji_st, "en", ResolveOptions::with_fallback())
        .expect("same kanji en fallback");
    assert_eq!(kanji_en_fb.text.as_deref(), Some("白虎"));
    assert_eq!(kanji_en_fb.lang.as_deref(), Some("ja"));

    assert_db_uses_index(
        db,
        text_sql::TEXT_LANGUAGE_COLUMN,
        ["zh-Hans"],
        &["text_language"],
    );
    assert_db_uses_index(
        db,
        "SELECT ns FROM string_table WHERE table_id = ?1",
        [TABLE_ID],
        &["string_table"],
    );
    assert_db_uses_index(
        db,
        "SELECT source FROM string_table_entry WHERE table_id = ?1 AND key = ?2",
        params![TABLE_ID, "SourceOnly"],
        &["string_table_entry"],
    );
}

#[test]
fn battle_stage_sets_untranslated_name_zh_to_null_and_keeps_pure_hanzi() {
    let temp = tempfile::tempdir().expect("create temp workspace");
    let export_copy = temp.path().join("export");
    copy_dir_recursive(&fixture_area_dir("battle").join("export"), &export_copy);

    let loc_dir = export_copy.join("Localization");
    let mut ja: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(loc_dir.join("ja.json")).expect("read ja.json"))
            .expect("parse ja.json");
    let mut zh: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(loc_dir.join("zh-Hans.json")).expect("read zh-Hans.json"),
    )
    .expect("parse zh-Hans.json");
    let mut en: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(loc_dir.join("en.json")).expect("read en.json"))
            .expect("parse en.json");

    // Unit 9101: zh-Hans left as Japanese kana (same as ja) -> name_zh must become NULL and
    // unit_untranslated must record it.
    let ja_9101 = ja["ST_UnitName"]["UnitName9101"]
        .as_str()
        .expect("ja 9101")
        .to_owned();
    zh["ST_UnitName"]["UnitName9101"] = serde_json::Value::String(ja_9101.clone());
    // Unit 9201: pure Hanzi identical across ja/zh-Hans/en -> kept for zh-Hans, dropped for en.
    for doc in [&mut ja, &mut zh, &mut en] {
        doc["ST_UnitName"]["UnitName9201"] = serde_json::Value::String("青龙".to_owned());
    }
    // Skill 95001 name (zh-Hans) & Skill 95002 description (en) left as Japanese kana.
    let ja_skill_name = ja["ST_SkillName"]["SkillName95001"]
        .as_str()
        .expect("ja skill 95001")
        .to_owned();
    zh["ST_SkillName"]["SkillName95001"] = serde_json::Value::String(ja_skill_name.clone());
    let ja_skill_desc = ja["ST_SkillDescription"]["SkillDescription95002"]
        .as_str()
        .expect("ja skill desc 95002")
        .to_owned();
    en["ST_SkillDescription"]["SkillDescription95002"] =
        serde_json::Value::String(ja_skill_desc.clone());
    // CommandSkill 1 name (zh-Hans) left as Japanese kana.
    let ja_cmd_name = ja["ST_CommandSkillName"]["CmdName1"]
        .as_str()
        .expect("ja cmd 1")
        .to_owned();
    zh["ST_CommandSkillName"]["CmdName1"] = serde_json::Value::String(ja_cmd_name.clone());

    fs::write(
        loc_dir.join("ja.json"),
        serde_json::to_string_pretty(&ja).expect("serialize ja"),
    )
    .expect("write ja.json");
    fs::write(
        loc_dir.join("zh-Hans.json"),
        serde_json::to_string_pretty(&zh).expect("serialize zh"),
    )
    .expect("write zh-Hans.json");
    fs::write(
        loc_dir.join("en.json"),
        serde_json::to_string_pretty(&en).expect("serialize en"),
    )
    .expect("write en.json");

    let test_db = build_temp_db(
        "battle-untranslated-name-zh",
        BuildInputs {
            export: Some(export_copy),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Text, Stage::Battle]),
    )
    .expect("build temp battle db");
    let db = test_db.db();

    let rows = db
        .query_readonly_json(
            "SELECT id, name_ja, name_en, name_zh FROM unit WHERE id IN (9101, 9201) ORDER BY id",
        )
        .expect("query unit names");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], 9101);
    assert_eq!(rows[0]["name_ja"].as_str(), Some(ja_9101.as_str()));
    assert!(!rows[0]["name_en"].is_null());
    assert!(
        rows[0]["name_zh"].is_null(),
        "untranslated Japanese kana must become NULL name_zh: {:?}",
        rows[0]
    );
    assert_eq!(rows[1]["id"], 9201);
    assert_eq!(rows[1]["name_ja"].as_str(), Some("青龙"));
    assert!(
        rows[1]["name_en"].is_null(),
        "untranslated CJK in en must become NULL name_en: {:?}",
        rows[1]
    );
    assert_eq!(rows[1]["name_zh"].as_str(), Some("青龙"));

    let un_9101 = db
        .query_readonly_json("SELECT ja, en, zh FROM unit_name WHERE unit_id = 9101")
        .expect("unit_name 9101");
    assert_eq!(un_9101.len(), 1);
    assert_eq!(un_9101[0]["ja"].as_str(), Some(ja_9101.as_str()));
    assert!(!un_9101[0]["en"].is_null());
    assert!(un_9101[0]["zh"].is_null());

    let un_9201 = db
        .query_readonly_json("SELECT ja, en, zh FROM unit_name WHERE unit_id = 9201")
        .expect("unit_name 9201");
    assert_eq!(un_9201.len(), 1);
    assert_eq!(un_9201[0]["ja"].as_str(), Some("青龙"));
    assert!(un_9201[0]["en"].is_null());
    assert_eq!(un_9201[0]["zh"].as_str(), Some("青龙"));

    // Companion tables record every untranslated entity field cleanly.
    assert_eq!(
        db.untranslated_units("zh-Hans")
            .expect("untranslated units zh"),
        vec![UntranslatedField {
            key: 9101,
            field: "name".to_owned(),
            lang: "zh-Hans".to_owned(),
            text_ns: "ST_UnitName".to_owned(),
            text_key: "UnitName9101".to_owned(),
            value: ja_9101.clone(),
        }]
    );
    assert_eq!(
        db.unit_untranslated_fields(9101).expect("unit 9101 fields"),
        vec![UntranslatedField {
            key: 9101,
            field: "name".to_owned(),
            lang: "zh-Hans".to_owned(),
            text_ns: "ST_UnitName".to_owned(),
            text_key: "UnitName9101".to_owned(),
            value: ja_9101,
        }]
    );
    assert_eq!(
        db.unit_untranslated_fields(9201).expect("unit 9201 fields"),
        vec![UntranslatedField {
            key: 9201,
            field: "name".to_owned(),
            lang: "en".to_owned(),
            text_ns: "ST_UnitName".to_owned(),
            text_key: "UnitName9201".to_owned(),
            value: "青龙".to_owned(),
        }]
    );

    assert_eq!(
        db.untranslated_skills("zh-Hans")
            .expect("untranslated skills zh"),
        vec![UntranslatedField {
            key: 95001,
            field: "name".to_owned(),
            lang: "zh-Hans".to_owned(),
            text_ns: "ST_SkillName".to_owned(),
            text_key: "SkillName95001".to_owned(),
            value: ja_skill_name.clone(),
        }]
    );
    assert_eq!(
        db.skill_untranslated_fields(95002)
            .expect("skill 95002 fields"),
        vec![UntranslatedField {
            key: 95002,
            field: "description".to_owned(),
            lang: "en".to_owned(),
            text_ns: "ST_SkillDescription".to_owned(),
            text_key: "SkillDescription95002".to_owned(),
            value: ja_skill_desc,
        }]
    );
    assert_eq!(
        db.untranslated_command_skills("zh-Hans")
            .expect("untranslated command skills zh"),
        vec![UntranslatedField {
            key: 1,
            field: "name".to_owned(),
            lang: "zh-Hans".to_owned(),
            text_ns: "ST_CommandSkillName".to_owned(),
            text_key: "CmdName1".to_owned(),
            value: ja_cmd_name.clone(),
        }]
    );
    assert_eq!(
        db.command_skill_untranslated_fields(1)
            .expect("cmd 1 fields"),
        vec![UntranslatedField {
            key: 1,
            field: "name".to_owned(),
            lang: "zh-Hans".to_owned(),
            text_ns: "ST_CommandSkillName".to_owned(),
            text_key: "CmdName1".to_owned(),
            value: ja_cmd_name,
        }]
    );

    let battle_stage = test_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "battle")
        .expect("battle stage report");
    assert!(
        battle_stage
            .note
            .contains("unit_untranslated(en=1, zh-Hans=1)")
            && battle_stage
                .note
                .contains("skill_untranslated(en=1, zh-Hans=1)")
            && battle_stage
                .note
                .contains("command_skill_untranslated(en=0, zh-Hans=1)"),
        "battle stage_report.note missing companion counts: {}",
        battle_stage.note
    );
}

#[test]
fn world_and_links_stages_populate_companion_untranslated_tables() {
    // 1. World stage: mark one key per text-bearing world table as untranslated Japanese.
    let temp_world = tempfile::tempdir().expect("temp world dir");
    let world_export = temp_world.path().join("export");
    copy_dir_recursive(&fixture_area_dir("world").join("export"), &world_export);
    let loc_dir = world_export.join("Localization");
    let ja: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(loc_dir.join("ja.json")).expect("read ja"))
            .expect("parse ja");
    let mut zh: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(loc_dir.join("zh-Hans.json")).expect("read zh-Hans"),
    )
    .expect("parse zh-Hans");
    let mut en: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(loc_dir.join("en.json")).expect("read en"))
            .expect("parse en");

    for key in [
        "Map_Harbor",
        "Island_North",
        "SP_Save",
        "Item_Herb",
        "Shop_Item",
        "Quest_Lantern",
        "Quest_Bonus_Text",
        "QB_Deliver",
        "Chapter_1",
        "Main_1020_Goal",
    ] {
        zh["ST_FakeWorld"][key] = ja["ST_FakeWorld"][key].clone();
    }
    en["ST_FakeWorld"]["Item_Herb_Desc"] = ja["ST_FakeWorld"]["Item_Herb_Desc"].clone();
    zh["ST_UnitName"]["UnitName9001"] = ja["ST_UnitName"]["UnitName9001"].clone();
    zh["ST_UnitName"]["UnitName9101"] = ja["ST_UnitName"]["UnitName9101"].clone();

    fs::write(
        loc_dir.join("zh-Hans.json"),
        serde_json::to_string_pretty(&zh).expect("serialize zh"),
    )
    .expect("write zh-Hans");
    fs::write(
        loc_dir.join("en.json"),
        serde_json::to_string_pretty(&en).expect("serialize en"),
    )
    .expect("write en");

    let world_db = build_temp_db(
        "world-untranslated-companions",
        BuildInputs {
            export: Some(world_export),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Text, Stage::World]),
    )
    .expect("build world temp db");
    let wdb = world_db.db();

    assert!(!wdb.untranslated_maps("zh-Hans").unwrap().is_empty());
    assert!(!wdb.untranslated_islands("zh-Hans").unwrap().is_empty());
    assert!(
        !wdb.untranslated_save_point_commands("zh-Hans")
            .unwrap()
            .is_empty()
    );
    let zh_items = wdb.untranslated_items("zh-Hans").unwrap();
    let en_items = wdb.untranslated_items("en").unwrap();
    assert_eq!(zh_items.len(), 1);
    assert_eq!(zh_items[0].field, "name");
    assert_eq!(en_items.len(), 1);
    assert_eq!(en_items[0].field, "description");
    assert_eq!(
        wdb.item_untranslated_fields(zh_items[0].key).unwrap().len(),
        2
    );
    assert!(!wdb.untranslated_shops("zh-Hans").unwrap().is_empty());
    assert!(!wdb.untranslated_quests("zh-Hans").unwrap().is_empty());
    assert!(!wdb.untranslated_quest_prizes("zh-Hans").unwrap().is_empty());
    assert!(!wdb.untranslated_quest_boards("zh-Hans").unwrap().is_empty());
    assert!(!wdb.untranslated_chapters("zh-Hans").unwrap().is_empty());
    assert!(
        !wdb.untranslated_main_scenarios("zh-Hans")
            .unwrap()
            .is_empty()
    );
    assert!(!wdb.untranslated_main_quests("zh-Hans").unwrap().is_empty());
    assert!(
        !wdb.untranslated_playable_units("zh-Hans")
            .unwrap()
            .is_empty()
    );
    assert!(!wdb.untranslated_item_drops("zh-Hans").unwrap().is_empty());

    let world_stage = world_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "world")
        .expect("world stage report");
    assert!(
        world_stage
            .note
            .contains("item_untranslated(en=1, zh-Hans=1)")
            && world_stage
                .note
                .contains("map_untranslated(en=0, zh-Hans=1)")
            && world_stage
                .note
                .contains("item_drop_untranslated(en=0, zh-Hans=2)"),
        "world stage_report.note missing companion counts: {}",
        world_stage.note
    );

    // 2. Links stage: mark one talk line (zh-Hans & en) and one playable map_unit (zh-Hans)
    // as untranslated Japanese.
    let temp_links = tempfile::tempdir().expect("temp links dir");
    let links_root = temp_links.path().join("links");
    copy_dir_recursive(&fixture_area_dir("links"), &links_root);
    let lloc = links_root.join("export/Localization");
    let lja: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(lloc.join("ja.json")).expect("read links ja"))
            .expect("parse links ja");
    let mut lzh: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(lloc.join("zh-Hans.json")).expect("read links zh-Hans"),
    )
    .expect("parse links zh-Hans");
    let mut len: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(lloc.join("en.json")).expect("read links en"))
            .expect("parse links en");

    lzh["ST_MockStory"]["L1"] = lja["ST_MockStory"]["L1"].clone();
    len["ST_MockStory"]["L1"] = lja["ST_MockStory"]["L1"].clone();
    lzh["ST_MockWorld"]["UNIT_1001"] = lja["ST_MockWorld"]["UNIT_1001"].clone();

    fs::write(
        lloc.join("zh-Hans.json"),
        serde_json::to_string_pretty(&lzh).expect("serialize links zh"),
    )
    .expect("write links zh");
    fs::write(
        lloc.join("en.json"),
        serde_json::to_string_pretty(&len).expect("serialize links en"),
    )
    .expect("write links en");

    let links_db = build_temp_db(
        "links-untranslated-companions",
        BuildInputs {
            export: Some(links_root.join("export")),
            ..BuildInputs::default()
        },
        Some(vec![
            Stage::Text,
            Stage::Datatable,
            Stage::World,
            Stage::Links,
        ]),
    )
    .expect("build links temp db");
    let ldb = links_db.db();

    let zh_lines = ldb.untranslated_talk_lines("zh-Hans").unwrap();
    assert_eq!(zh_lines.len(), 1);
    assert_eq!(zh_lines[0].field, "text");
    assert_eq!(zh_lines[0].text_ns, "ST_MockStory");
    assert_eq!(zh_lines[0].text_key, "L1");
    let (tp, row_k) = &zh_lines[0].key;
    assert_eq!(
        ldb.talk_line_untranslated_fields(tp, row_k).unwrap().len(),
        2
    );
    let resolved_lines = ldb.talk_lines("EV_STORY_01", "zh-Hans").unwrap();
    let first_line = resolved_lines
        .iter()
        .find(|l| l.text_key.as_deref() == Some("L1"))
        .expect("L1 talk line exists");
    assert_eq!(
        first_line.text, None,
        "untranslated talk line must resolve to None"
    );

    let zh_mu = ldb.untranslated_map_units("zh-Hans").unwrap();
    assert_eq!(zh_mu.len(), 1);
    assert_eq!(zh_mu[0].field, "name");
    assert_eq!(
        ldb.map_unit_untranslated_fields(zh_mu[0].key).unwrap(),
        zh_mu
    );
    let mu = ldb
        .map_unit(zh_mu[0].key)
        .unwrap()
        .expect("map_unit exists");
    assert_eq!(
        mu.name_zh, None,
        "untranslated map_unit name_zh must be None"
    );

    let links_stage = links_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "links")
        .expect("links stage report");
    assert!(
        links_stage
            .note
            .contains("talk_line_untranslated(en=1, zh-Hans=1)")
            && links_stage
                .note
                .contains("map_unit_untranslated(en=0, zh-Hans=1)"),
        "links stage_report.note missing companion counts: {}",
        links_stage.note
    );
}

#[test]
fn wide_multilingual_tables_match_long_per_language_expansion() {
    // 1. `text`: synthetic localization files covering normal 3-lang entries, empty-string values,
    //    missing languages, untranslated kana, pure GB2312 Hanzi, and non-GB2312 Japanese kanji.
    let temp = tempfile::tempdir().expect("create temp export");
    let export_dir = temp.path().join("export");
    let loc_dir = export_dir.join("Localization");
    fs::create_dir_all(&loc_dir).expect("create Localization dir");

    let ja_doc = serde_json::json!({
        "NS_Equiv": {
            "Normal": "鉄の剣",
            "EmptyTrans": "空欄テスト",
            "JaOnly": "日本語のみ",
            "UntransKana": "ポーション",
            "SameGb2312": "白虎",
            "NonGb2312": "剣技",
        }
    });
    let en_doc = serde_json::json!({
        "NS_Equiv": {
            "Normal": "Iron Sword",
            "EmptyTrans": "",
            "NoJa": "English Only Key",
            "UntransKana": "ポーション",
            "SameGb2312": "白虎",
            "NonGb2312": "剣技",
        }
    });
    let zh_doc = serde_json::json!({
        "NS_Equiv": {
            "Normal": "铁剑",
            "EmptyTrans": "",
            "NoJa": "仅中英文键",
            "UntransKana": "ポーション",
            "SameGb2312": "白虎",
            "NonGb2312": "剣技",
        }
    });

    for (file, doc) in [
        ("ja.json", &ja_doc),
        ("en.json", &en_doc),
        ("zh-Hans.json", &zh_doc),
    ] {
        fs::write(
            loc_dir.join(file),
            serde_json::to_string_pretty(doc).expect("serialize loc"),
        )
        .expect("write loc");
    }

    // Compute the expected v3-style "one row per language" expansion directly from JSON.
    let mut expected_long_text: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    let mut expected_untranslated: BTreeSet<(String, String, String, String, String)> =
        BTreeSet::new();
    for (lang, doc) in [("ja", &ja_doc), ("en", &en_doc), ("zh-Hans", &zh_doc)] {
        for (ns, entries) in doc.as_object().expect("root object") {
            for (key, val) in entries.as_object().expect("ns object") {
                let value = val.as_str().expect("string value");
                if lang != "ja"
                    && let Some(ja_val) = ja_doc
                        .get(ns)
                        .and_then(|m| m.get(key))
                        .and_then(|v| v.as_str())
                    && let Some(reason) = untranslated_reason_code(lang, value, Some(ja_val))
                {
                    expected_untranslated.insert((
                        ns.clone(),
                        key.clone(),
                        lang.to_owned(),
                        value.to_owned(),
                        reason.to_owned(),
                    ));
                    continue;
                }
                expected_long_text.insert((
                    ns.clone(),
                    key.clone(),
                    lang.to_owned(),
                    value.to_owned(),
                ));
            }
        }
    }

    let text_db = build_temp_db(
        "wide-vs-long-text",
        BuildInputs {
            export: Some(export_dir),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Text]),
    )
    .expect("build wide-vs-long text db");
    let tdb = text_db.db();

    // Unpivot v4 wide `text(ns, key, ja, en, zh)` back to `(ns, key, lang, value)`.
    let actual_long_text: BTreeSet<(String, String, String, String)> = tdb
        .query_readonly_json(
            "SELECT ns, key, 'ja' AS lang, ja AS value FROM text WHERE ja IS NOT NULL \
             UNION ALL \
             SELECT ns, key, 'en' AS lang, en AS value FROM text WHERE en IS NOT NULL \
             UNION ALL \
             SELECT ns, key, 'zh-Hans' AS lang, zh AS value FROM text WHERE zh IS NOT NULL",
        )
        .expect("unpivot wide text")
        .into_iter()
        .map(|row| {
            (
                row["ns"].as_str().expect("ns").to_owned(),
                row["key"].as_str().expect("key").to_owned(),
                row["lang"].as_str().expect("lang").to_owned(),
                row["value"].as_str().expect("value").to_owned(),
            )
        })
        .collect();
    assert_eq!(actual_long_text, expected_long_text);

    // Empty strings stay "" (not NULL), and non-GB2312 Hanzi is recorded in `text_untranslated`.
    assert_eq!(
        tdb.text("NS_Equiv", "EmptyTrans", "en").expect("empty en"),
        Some(String::new())
    );
    assert_eq!(
        tdb.text("NS_Equiv", "EmptyTrans", "zh-Hans")
            .expect("empty zh"),
        Some(String::new())
    );
    assert_eq!(
        tdb.text_untranslated("NS_Equiv", "NonGb2312", "zh-Hans")
            .expect("non-gb2312 zh")
            .map(|e| e.reason),
        Some(UNTRANSLATED_REASON_NON_SIMPLIFIED_HANZI.to_owned())
    );
    let actual_untranslated: BTreeSet<(String, String, String, String, String)> = tdb
        .query_readonly_json("SELECT ns, key, lang, value, reason FROM text_untranslated")
        .expect("read text_untranslated")
        .into_iter()
        .map(|row| {
            (
                row["ns"].as_str().expect("ns").to_owned(),
                row["key"].as_str().expect("key").to_owned(),
                row["lang"].as_str().expect("lang").to_owned(),
                row["value"].as_str().expect("value").to_owned(),
                row["reason"].as_str().expect("reason").to_owned(),
            )
        })
        .collect();
    assert_eq!(actual_untranslated, expected_untranslated);

    // 2. `unit_name` and `skill_name`: wide unpivot equals long per-language expansion.
    let battle_export = fixture_area_dir("battle").join("export");
    let battle_db = build_fixture_db_stages("battle", Some(vec![Stage::Text, Stage::Battle]))
        .expect("build battle fixture");
    let bdb = battle_db.db();
    let battle_store = DataStore::new(&battle_export);
    let battle = battle_store.battle().expect("load battle fixture");
    let battle_resolver = battle_store.text_resolver();

    let mut expected_unit_names: BTreeSet<(i64, String, String)> = BTreeSet::new();
    for unit in &battle.units {
        for lang in ["en", "ja", "zh-Hans"] {
            if let Some(name) = battle_resolver
                .resolve_with(&unit.name, Some(lang), ResolveOptions::default())
                .text
                .filter(|s| !s.is_empty())
            {
                expected_unit_names.insert((unit.id, lang.to_owned(), name));
            }
        }
    }
    let actual_unit_names: BTreeSet<(i64, String, String)> = bdb
        .query_readonly_json(
            "SELECT unit_id, 'ja' AS lang, ja AS name FROM unit_name WHERE ja IS NOT NULL \
             UNION ALL \
             SELECT unit_id, 'en' AS lang, en AS name FROM unit_name WHERE en IS NOT NULL \
             UNION ALL \
             SELECT unit_id, 'zh-Hans' AS lang, zh AS name FROM unit_name WHERE zh IS NOT NULL",
        )
        .expect("unpivot unit_name")
        .into_iter()
        .map(|row| {
            (
                row["unit_id"].as_i64().expect("unit_id"),
                row["lang"].as_str().expect("lang").to_owned(),
                row["name"].as_str().expect("name").to_owned(),
            )
        })
        .collect();
    assert_eq!(actual_unit_names, expected_unit_names);

    let mut expected_skill_names: BTreeSet<(i64, String, String)> = BTreeSet::new();
    for skill in &battle.skills {
        for lang in ["en", "ja", "zh-Hans"] {
            if let Some(name) = battle_resolver
                .resolve_with(&skill.name, Some(lang), ResolveOptions::default())
                .text
                .filter(|s| !s.is_empty())
            {
                expected_skill_names.insert((skill.id, lang.to_owned(), name));
            }
        }
    }
    let actual_skill_names: BTreeSet<(i64, String, String)> = bdb
        .query_readonly_json(
            "SELECT skill_id, 'ja' AS lang, ja AS name FROM skill_name WHERE ja IS NOT NULL \
             UNION ALL \
             SELECT skill_id, 'en' AS lang, en AS name FROM skill_name WHERE en IS NOT NULL \
             UNION ALL \
             SELECT skill_id, 'zh-Hans' AS lang, zh AS name FROM skill_name WHERE zh IS NOT NULL",
        )
        .expect("unpivot skill_name")
        .into_iter()
        .map(|row| {
            (
                row["skill_id"].as_i64().expect("skill_id"),
                row["lang"].as_str().expect("lang").to_owned(),
                row["name"].as_str().expect("name").to_owned(),
            )
        })
        .collect();
    assert_eq!(actual_skill_names, expected_skill_names);

    // 3. `world_name`: wide unpivot + `en_lower` equal long per-language expansion.
    let world_export = fixture_area_dir("world").join("export");
    let world_db = build_fixture_db_stages("world", Some(vec![Stage::Text, Stage::World]))
        .expect("build world fixture");
    let wdb = world_db.db();
    let world_store = DataStore::new(&world_export);
    let world = WorldData::from_store(&world_store).expect("load world fixture");
    let world_resolver = world_store.text_resolver();

    let mut expected_world_names: BTreeSet<(String, i64, String, String)> = BTreeSet::new();
    for map in world.maps() {
        for lang in ["en", "ja", "zh-Hans"] {
            if let Some(name) = world_resolver
                .resolve_with(&map.name, Some(lang), ResolveOptions::default())
                .text
                .filter(|s| !s.trim().is_empty())
            {
                expected_world_names.insert(("map".to_owned(), map.id, lang.to_owned(), name));
            }
        }
    }
    for item in world.items() {
        for lang in ["en", "ja", "zh-Hans"] {
            if let Some(name) = world_resolver
                .resolve_with(&item.name, Some(lang), ResolveOptions::default())
                .text
                .filter(|s| !s.trim().is_empty())
            {
                expected_world_names.insert(("item".to_owned(), item.id, lang.to_owned(), name));
            }
        }
    }
    let actual_world_names: BTreeSet<(String, i64, String, String)> = wdb
        .query_readonly_json(
            "SELECT entity_kind, entity_id, 'ja' AS lang, ja AS name FROM world_name WHERE ja IS NOT NULL \
             UNION ALL \
             SELECT entity_kind, entity_id, 'en' AS lang, en AS name FROM world_name WHERE en IS NOT NULL \
             UNION ALL \
             SELECT entity_kind, entity_id, 'zh-Hans' AS lang, zh AS name FROM world_name WHERE zh IS NOT NULL",
        )
        .expect("unpivot world_name")
        .into_iter()
        .map(|row| {
            (
                row["entity_kind"].as_str().expect("entity_kind").to_owned(),
                row["entity_id"].as_i64().expect("entity_id"),
                row["lang"].as_str().expect("lang").to_owned(),
                row["name"].as_str().expect("name").to_owned(),
            )
        })
        .collect();
    assert_eq!(actual_world_names, expected_world_names);

    let world_en_lower_mismatches = wdb
        .query_readonly_json(
            "SELECT entity_kind, entity_id, en, en_lower FROM world_name \
             WHERE (en IS NULL AND en_lower IS NOT NULL) \
                OR (en IS NOT NULL AND en_lower != lower(en))",
        )
        .expect("check world_name.en_lower");
    assert!(
        world_en_lower_mismatches.is_empty(),
        "world_name.en_lower mismatches: {world_en_lower_mismatches:?}"
    );

    // 4. Verify all 4 FTS5 tables have columns [ja, en, zh] and all 17 object tables have
    //    consecutive (_ja, _en, _zh) sibling columns in that exact order.
    let full_db = build_fixture_db("links").expect("build full links fixture");
    let fdb = full_db.db();
    for fts in [
        "text_fts",
        "unit_name_fts",
        "skill_name_fts",
        "world_name_fts",
    ] {
        let cols: Vec<String> = fdb
            .query_readonly_json(&format!(
                "SELECT name FROM pragma_table_info('{fts}') ORDER BY cid"
            ))
            .expect("pragma_table_info fts")
            .into_iter()
            .map(|r| r["name"].as_str().expect("col").to_owned())
            .collect();
        assert_eq!(cols, vec!["ja", "en", "zh"], "FTS5 columns for {fts}");
    }

    let sibling_checks = [
        ("unit", "name"),
        ("skill", "name"),
        ("command_skill", "name"),
        ("map", "name"),
        ("island", "name"),
        ("save_point_command", "label"),
        ("item", "name"),
        ("shop", "name"),
        ("quest", "name"),
        ("quest_prize", "text"),
        ("quest_board", "name"),
        ("chapter", "name"),
        ("main_scenario", "text"),
        ("main_quest", "name"),
        ("playable_unit", "name"),
        ("item_drop", "unit_name"),
        ("map_unit", "name"),
    ];
    for (table, prefix) in sibling_checks {
        let cols: Vec<String> = fdb
            .query_readonly_json(&format!(
                "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
            ))
            .expect("pragma_table_info object table")
            .into_iter()
            .map(|r| r["name"].as_str().expect("col").to_owned())
            .collect();
        let ja_col = format!("{prefix}_ja");
        let en_col = format!("{prefix}_en");
        let zh_col = format!("{prefix}_zh");
        let pos = cols
            .iter()
            .position(|c| c == &ja_col)
            .unwrap_or_else(|| panic!("{table} missing {ja_col}: {cols:?}"));
        assert_eq!(
            &cols[pos..pos + 3],
            &[ja_col, en_col, zh_col],
            "{table} sibling columns must be ordered (_ja, _en, _zh): {cols:?}"
        );
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create dir");
    for entry in fs::read_dir(src).expect("read dir") {
        let entry = entry.expect("dir entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("copy file");
        }
    }
}

#[test]
fn real_data_short_and_trigram_search_timings() {
    let export = real_export();
    let Some(export) = export else {
        eprintln!("跳过真实文本搜索计时：.deps/ffr/export 不存在");
        return;
    };
    let test_db = build_temp_db(
        "real-text-search",
        BuildInputs {
            export: Some(export),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Text]),
    )
    .expect("build real text database");
    for query in ["钢蝠", "钢铁蝙"] {
        let start = Instant::now();
        let hits = test_db
            .db()
            .search_text(query, Some("zh-Hans"), 20)
            .expect("search real text");
        let millis = start.elapsed().as_secs_f64() * 1000.0;
        println!("真实搜索 {query:?}：{} 条，{millis:.3} ms", hits.len());
    }
}

fn real_export() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.deps/ffr/export");
    path.join("Localization/zh-Hans.json")
        .is_file()
        .then_some(path)
}
