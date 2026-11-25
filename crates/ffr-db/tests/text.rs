use std::path::PathBuf;
use std::time::Instant;

use ffr_data::{FTextValue, ResolveOptions, TextResolver};
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db_stages, build_temp_db, fixture_area_dir,
};
use ffr_db::{BuildInputs, Stage};
use rusqlite::params;

const TABLE_ID: &str = "/Game/Datatable/Strings/Fake/ST_Fake.ST_Fake";

#[test]
fn localized_lookup_and_search_use_indexes() {
    let test_db =
        build_fixture_db_stages("text", Some(vec![Stage::Text])).expect("build text fixture");
    let db = test_db.db();

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
        "SELECT value FROM text WHERE ns = ?1 AND key = ?2 AND lang = ?3",
        params!["FakeTextNamespace", "Greeting", "zh-Hans"],
        &["text"],
    );
    assert_db_uses_index(
        db,
        "SELECT lang, value FROM text WHERE ns = ?1 AND key = ?2 ORDER BY lang",
        params!["FakeTextNamespace", "Greeting"],
        &["text"],
    );
    assert_db_uses_index(
        db,
        "SELECT t.ns, t.key, t.lang, t.value FROM text_fts \
         JOIN text AS t ON t.id = text_fts.rowid \
         WHERE text_fts MATCH ?1 AND t.lang = ?2 \
         ORDER BY t.ns, t.key LIMIT ?3",
        params!["\"钢铁蝙\"", "zh-Hans", 20_i64],
        &["text_fts", "text"],
    );
    assert_db_uses_index(
        db,
        "SELECT t.ns, t.key, t.lang, t.value FROM text_fts \
         JOIN text AS t ON t.id = text_fts.rowid \
         WHERE text_fts MATCH ?1 \
         ORDER BY t.ns, t.key, t.lang LIMIT ?2",
        params!["\"钢铁蝙\"", 20_i64],
        &["text_fts", "text"],
    );
    assert_db_uses_index(
        db,
        "SELECT ns, key, lang, value FROM text INDEXED BY idx_text_lang_ns_key \
         WHERE lang = ?1 AND value LIKE ?2 ESCAPE '\\' \
         ORDER BY ns, key LIMIT ?3",
        params!["zh-Hans", "%钢蝠%", 20_i64],
        &["text"],
    );
    assert_db_uses_index(
        db,
        "SELECT ns, key, lang, value FROM text INDEXED BY idx_text_lang_ns_key \
         WHERE value LIKE ?1 ESCAPE '\\' \
         ORDER BY ns, key, lang LIMIT ?2",
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

    assert_db_uses_index(
        db,
        "SELECT 1 FROM text_language WHERE lang = ?1",
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
