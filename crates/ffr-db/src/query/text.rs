//! Text area (`01_text`) `impl GameDb` queries.

use std::path::Path;

use ffr_data::{
    FTextValue, NATIVE_LANGUAGE, ResolveOptions, ResolvedText, TextSource, UNTRANSLATED_REASON,
    is_safe_language_tag, ue_object_path_to_export_json,
};
use rusqlite::types::FromSql;
use rusqlite::{Connection, OptionalExtension, Params, Statement, params};
use serde::{Deserialize, Serialize};

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::schema::LangCol;

/// One localized text search result (from `text`, which excludes untranslated non-native rows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextHit {
    pub ns: String,
    pub key: String,
    pub lang: String,
    pub value: String,
}

/// One game-official untranslated entry stored in `text_untranslated`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntranslatedTextEntry {
    pub ns: String,
    pub key: String,
    pub lang: String,
    pub value: String,
    pub reason: String,
}

/// One entity text field recorded in a `<table_name>_untranslated` companion table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntranslatedField<K> {
    pub key: K,
    pub field: String,
    pub lang: String,
    pub text_ns: String,
    pub text_key: String,
    pub value: String,
}

/// Exposed SQL statements executed by `01_text` queries so integration tests can verify every
/// query plan with `assert_db_uses_index`.
pub mod sql {
    pub const TEXT_BY_KEY_JA: &str = "SELECT ja FROM text WHERE ns = ?1 AND key = ?2";
    pub const TEXT_BY_KEY_EN: &str = "SELECT en FROM text WHERE ns = ?1 AND key = ?2";
    pub const TEXT_BY_KEY_ZH: &str = "SELECT zh FROM text WHERE ns = ?1 AND key = ?2";
    pub const TEXT_ALL_LANGS: &str = "SELECT en, ja, zh FROM text WHERE ns = ?1 AND key = ?2";
    pub const TEXT_LANGUAGE_COLUMN: &str = "SELECT column_name FROM text_language WHERE lang = ?1";

    pub const SEARCH_TEXT_FTS_JA: &str = "\
        SELECT t.ns, t.key, t.ja \
        FROM text_fts JOIN text AS t ON t.id = text_fts.rowid \
        WHERE text_fts MATCH ?1 \
        ORDER BY t.ns, t.key LIMIT ?2";
    pub const SEARCH_TEXT_FTS_EN: &str = "\
        SELECT t.ns, t.key, t.en \
        FROM text_fts JOIN text AS t ON t.id = text_fts.rowid \
        WHERE text_fts MATCH ?1 \
        ORDER BY t.ns, t.key LIMIT ?2";
    pub const SEARCH_TEXT_FTS_ZH: &str = "\
        SELECT t.ns, t.key, t.zh \
        FROM text_fts JOIN text AS t ON t.id = text_fts.rowid \
        WHERE text_fts MATCH ?1 \
        ORDER BY t.ns, t.key LIMIT ?2";
    pub const SEARCH_TEXT_FTS_ALL: &str = "\
        SELECT t.ns, t.key, t.en, t.ja, t.zh \
        FROM text_fts JOIN text AS t ON t.id = text_fts.rowid \
        WHERE text_fts MATCH ?1 \
        ORDER BY t.ns, t.key LIMIT ?2";

    pub const SEARCH_TEXT_LIKE_JA: &str = "\
        SELECT ns, key, ja \
        FROM text INDEXED BY idx_text_ja \
        WHERE ja IS NOT NULL AND ja LIKE ?1 ESCAPE '\\' \
        ORDER BY ns, key LIMIT ?2";
    pub const SEARCH_TEXT_LIKE_EN: &str = "\
        SELECT ns, key, en \
        FROM text INDEXED BY idx_text_en \
        WHERE en IS NOT NULL AND en LIKE ?1 ESCAPE '\\' \
        ORDER BY ns, key LIMIT ?2";
    pub const SEARCH_TEXT_LIKE_ZH: &str = "\
        SELECT ns, key, zh \
        FROM text INDEXED BY idx_text_zh \
        WHERE zh IS NOT NULL AND zh LIKE ?1 ESCAPE '\\' \
        ORDER BY ns, key LIMIT ?2";
    pub const SEARCH_TEXT_LIKE_ALL: &str = "\
        SELECT ns, key, 'en' AS lang, en AS value \
        FROM text INDEXED BY idx_text_en \
        WHERE en IS NOT NULL AND en LIKE ?1 ESCAPE '\\' \
        UNION ALL \
        SELECT ns, key, 'ja' AS lang, ja AS value \
        FROM text INDEXED BY idx_text_ja \
        WHERE ja IS NOT NULL AND ja LIKE ?1 ESCAPE '\\' \
        UNION ALL \
        SELECT ns, key, 'zh-Hans' AS lang, zh AS value \
        FROM text INDEXED BY idx_text_zh \
        WHERE zh IS NOT NULL AND zh LIKE ?1 ESCAPE '\\' \
        ORDER BY ns, key, lang LIMIT ?2";
}

impl GameDb {
    /// Look up one translated localized string (or native `ja` string) by `(ns, key, lang)`.
    ///
    /// Game-official untranslated entries live in `text_untranslated` instead of `text`, so this
    /// strictly returns `None` for them.
    pub fn text(&self, ns: &str, key: &str, lang: &str) -> Result<Option<String>> {
        let Some(col) = LangCol::from_lang(lang) else {
            return Ok(None);
        };
        self.with_conn(|conn| lookup_text_col(conn, ns, key, col))
    }

    /// Return all translated entries (plus native `ja`) as `(lang, value)` pairs ordered by
    /// language.
    pub fn text_all_langs(&self, ns: &str, key: &str) -> Result<Vec<(String, String)>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::TEXT_ALL_LANGS)
                .map_err(|err| DbError::sqlite("准备查询 text 的全部语言", err))?;
            let row: Option<(Option<String>, Option<String>, Option<String>)> = stmt
                .query_row(params![ns, key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()
                .map_err(|err| DbError::sqlite("查询 text 的全部语言", err))?;
            let Some((en, ja, zh)) = row else {
                return Ok(Vec::new());
            };
            let mut out = Vec::with_capacity(3);
            for (lang, value) in [
                (LangCol::En.lang(), en),
                (LangCol::Ja.lang(), ja),
                (LangCol::Zh.lang(), zh),
            ] {
                if let Some(val) = value {
                    out.push((lang.to_owned(), val));
                }
            }
            Ok(out)
        })
    }

    /// Look up one game-official untranslated entry by its `(ns, key, lang)` primary key.
    pub fn text_untranslated(
        &self,
        ns: &str,
        key: &str,
        lang: &str,
    ) -> Result<Option<UntranslatedTextEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT ns, key, lang, value, reason FROM text_untranslated \
                     WHERE ns = ?1 AND key = ?2 AND lang = ?3",
                )
                .map_err(|err| DbError::sqlite("准备查询 text_untranslated", err))?;
            stmt.query_row(params![ns, key, lang], |row| {
                Ok(UntranslatedTextEntry {
                    ns: row.get(0)?,
                    key: row.get(1)?,
                    lang: row.get(2)?,
                    value: row.get(3)?,
                    reason: row.get(4)?,
                })
            })
            .optional()
            .map_err(|err| DbError::sqlite(format!("查询未翻译文本 {ns}/{key}（{lang}）"), err))
        })
    }

    /// Return all game-official untranslated entries for `lang`, ordered by `(ns, key)`.
    pub fn untranslated_texts(&self, lang: &str) -> Result<Vec<UntranslatedTextEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT ns, key, lang, value, reason FROM text_untranslated \
                     WHERE lang = ?1 ORDER BY ns, key",
                )
                .map_err(|err| DbError::sqlite("准备按语言查询 text_untranslated", err))?;
            let mut rows = stmt
                .query([lang])
                .map_err(|err| DbError::sqlite(format!("按语言查询未翻译文本（{lang}）"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 text_untranslated 行", err))?
            {
                out.push(UntranslatedTextEntry {
                    ns: row
                        .get(0)
                        .map_err(|err| DbError::sqlite("读取 text_untranslated.ns", err))?,
                    key: row
                        .get(1)
                        .map_err(|err| DbError::sqlite("读取 text_untranslated.key", err))?,
                    lang: row
                        .get(2)
                        .map_err(|err| DbError::sqlite("读取 text_untranslated.lang", err))?,
                    value: row
                        .get(3)
                        .map_err(|err| DbError::sqlite("读取 text_untranslated.value", err))?,
                    reason: row
                        .get(4)
                        .map_err(|err| DbError::sqlite("读取 text_untranslated.reason", err))?,
                });
            }
            Ok(out)
        })
    }

    /// Resolve an `FTextValue` with the same strict/fallback behavior as `TextResolver`.
    pub fn resolve_ftext(
        &self,
        value: &FTextValue,
        lang: &str,
        options: ResolveOptions,
    ) -> Result<ResolvedText> {
        self.with_conn(|conn| {
            let fallback_to_native = options.fallback_to_native.unwrap_or(false);
            match value {
                FTextValue::CultureInvariant { value } => match value {
                    None => Ok(missing(
                        lang,
                        "FText 为空（CultureInvariantString 为 null）",
                    )),
                    Some(text) => Ok(ResolvedText {
                        text: Some(text.clone()),
                        lang: Some(lang.to_owned()),
                        requested_language: lang.to_owned(),
                        reason: None,
                        source: TextSource::CultureInvariant,
                    }),
                },
                FTextValue::StringTable { table_id, key, .. } => {
                    resolve_string_table(conn, table_id, key, lang, fallback_to_native)
                }
                FTextValue::Inline {
                    namespace,
                    key,
                    source_string,
                    ..
                } => resolve_inline(
                    conn,
                    namespace,
                    key,
                    source_string.as_deref(),
                    lang,
                    fallback_to_native,
                ),
            }
        })
    }

    /// Search localized values in `text` by literal substring, using 3-column trigram FTS5
    /// (with column filtering when `lang` is specified) or a short-query covering-index LIKE
    /// fallback.
    pub fn search_text(
        &self,
        query: &str,
        lang: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TextHit>> {
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        self.with_conn(|conn| {
            if query.chars().count() >= 3 {
                if let Some(lang) = lang {
                    let Some(col) = LangCol::from_lang(lang) else {
                        return Ok(Vec::new());
                    };
                    let match_query = fts_column_phrase(col, query);
                    let query_sql = match col {
                        LangCol::Ja => sql::SEARCH_TEXT_FTS_JA,
                        LangCol::En => sql::SEARCH_TEXT_FTS_EN,
                        LangCol::Zh => sql::SEARCH_TEXT_FTS_ZH,
                    };
                    let mut stmt = conn
                        .prepare_cached(query_sql)
                        .map_err(|err| DbError::sqlite("准备按语言 trigram 文本搜索", err))?;
                    query_single_lang_hits(&mut stmt, params![match_query, limit_i64], col.lang())
                } else {
                    let match_query = fts_all_columns_phrase(query);
                    let query_lower = query.to_lowercase();
                    let mut stmt = conn
                        .prepare_cached(sql::SEARCH_TEXT_FTS_ALL)
                        .map_err(|err| DbError::sqlite("准备全语言 trigram 文本搜索", err))?;
                    let mut rows = stmt
                        .query(params![match_query, limit_i64])
                        .map_err(|err| DbError::sqlite("执行全语言 trigram 文本搜索", err))?;
                    let mut out = Vec::new();
                    while let Some(row) = rows
                        .next()
                        .map_err(|err| DbError::sqlite("读取全语言文本搜索结果", err))?
                    {
                        let ns: String = row
                            .get(0)
                            .map_err(|err| DbError::sqlite("读取搜索结果的 ns", err))?;
                        let key: String = row
                            .get(1)
                            .map_err(|err| DbError::sqlite("读取搜索结果的 key", err))?;
                        let cols: [Option<String>; 3] = [
                            row.get(2)
                                .map_err(|err| DbError::sqlite("读取搜索结果的 en", err))?,
                            row.get(3)
                                .map_err(|err| DbError::sqlite("读取搜索结果的 ja", err))?,
                            row.get(4)
                                .map_err(|err| DbError::sqlite("读取搜索结果的 zh", err))?,
                        ];
                        for (lang_col, maybe_val) in LangCol::BY_LANG_ORDER.into_iter().zip(cols) {
                            if let Some(val) = maybe_val
                                && val.to_lowercase().contains(&query_lower)
                            {
                                out.push(TextHit {
                                    ns: ns.clone(),
                                    key: key.clone(),
                                    lang: lang_col.lang().to_owned(),
                                    value: val,
                                });
                                if out.len() >= limit {
                                    return Ok(out);
                                }
                            }
                        }
                    }
                    Ok(out)
                }
            } else {
                let pattern = like_pattern(query);
                if let Some(lang) = lang {
                    let Some(col) = LangCol::from_lang(lang) else {
                        return Ok(Vec::new());
                    };
                    let query_sql = match col {
                        LangCol::Ja => sql::SEARCH_TEXT_LIKE_JA,
                        LangCol::En => sql::SEARCH_TEXT_LIKE_EN,
                        LangCol::Zh => sql::SEARCH_TEXT_LIKE_ZH,
                    };
                    let mut stmt = conn
                        .prepare_cached(query_sql)
                        .map_err(|err| DbError::sqlite("准备按语言短文本 LIKE 搜索", err))?;
                    query_single_lang_hits(&mut stmt, params![pattern, limit_i64], col.lang())
                } else {
                    let mut stmt = conn
                        .prepare_cached(sql::SEARCH_TEXT_LIKE_ALL)
                        .map_err(|err| DbError::sqlite("准备全语言短文本 LIKE 搜索", err))?;
                    query_hits(&mut stmt, params![pattern, limit_i64])
                }
            }
        })
    }
}

/// Shared helper for reading `<table_name>_untranslated` rows with a single primary-key column.
///
/// Expected column order: `(pk, field, lang, text_ns, text_key, value)`.
pub(crate) fn query_untranslated_single_key<K, P>(
    conn: &Connection,
    table: &str,
    sql: &str,
    params: P,
) -> Result<Vec<UntranslatedField<K>>>
where
    K: FromSql,
    P: Params,
{
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备查询 {table}"), err))?;
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite(format!("查询 {table}"), err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取 {table} 行"), err))?
    {
        out.push(UntranslatedField {
            key: row
                .get(0)
                .map_err(|err| DbError::sqlite(format!("读取 {table} 主键"), err))?,
            field: row
                .get(1)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.field"), err))?,
            lang: row
                .get(2)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.lang"), err))?,
            text_ns: row
                .get(3)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.text_ns"), err))?,
            text_key: row
                .get(4)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.text_key"), err))?,
            value: row
                .get(5)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.value"), err))?,
        });
    }
    Ok(out)
}

/// Shared helper for reading `<table_name>_untranslated` rows with a 2-column composite primary key.
///
/// Expected column order: `(pk1, pk2, field, lang, text_ns, text_key, value)`.
pub(crate) fn query_untranslated_pair_key<K1, K2, P>(
    conn: &Connection,
    table: &str,
    sql: &str,
    params: P,
) -> Result<Vec<UntranslatedField<(K1, K2)>>>
where
    K1: FromSql,
    K2: FromSql,
    P: Params,
{
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备查询 {table}"), err))?;
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite(format!("查询 {table}"), err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取 {table} 行"), err))?
    {
        let k1: K1 = row
            .get(0)
            .map_err(|err| DbError::sqlite(format!("读取 {table} 主键列 1"), err))?;
        let k2: K2 = row
            .get(1)
            .map_err(|err| DbError::sqlite(format!("读取 {table} 主键列 2"), err))?;
        out.push(UntranslatedField {
            key: (k1, k2),
            field: row
                .get(2)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.field"), err))?,
            lang: row
                .get(3)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.lang"), err))?,
            text_ns: row
                .get(4)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.text_ns"), err))?,
            text_key: row
                .get(5)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.text_key"), err))?,
            value: row
                .get(6)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.value"), err))?,
        });
    }
    Ok(out)
}

fn query_hits<P: Params>(stmt: &mut Statement<'_>, params: P) -> Result<Vec<TextHit>> {
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite("执行文本搜索", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取文本搜索结果", err))?
    {
        out.push(TextHit {
            ns: row
                .get(0)
                .map_err(|err| DbError::sqlite("读取搜索结果的 ns", err))?,
            key: row
                .get(1)
                .map_err(|err| DbError::sqlite("读取搜索结果的 key", err))?,
            lang: row
                .get(2)
                .map_err(|err| DbError::sqlite("读取搜索结果的 lang", err))?,
            value: row
                .get(3)
                .map_err(|err| DbError::sqlite("读取搜索结果的 value", err))?,
        });
    }
    Ok(out)
}

fn query_single_lang_hits<P: Params>(
    stmt: &mut Statement<'_>,
    params: P,
    lang: &str,
) -> Result<Vec<TextHit>> {
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite("执行文本搜索", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取文本搜索结果", err))?
    {
        let value: Option<String> = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取搜索结果的 value", err))?;
        if let Some(value) = value {
            out.push(TextHit {
                ns: row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取搜索结果的 ns", err))?,
                key: row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取搜索结果的 key", err))?,
                lang: lang.to_owned(),
                value,
            });
        }
    }
    Ok(out)
}

/// Escape `%`, `_`, and `\` for a literal SQLite `LIKE ... ESCAPE '\'` substring match.
pub(crate) fn like_pattern(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len() + 2);
    escaped.push('%');
    for ch in query.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('%');
    escaped
}

/// Build an FTS5 column-filtered literal phrase query (`<col> : "<escaped>"`).
pub(crate) fn fts_column_phrase(col: LangCol, query: &str) -> String {
    let escaped = query.replace('"', "\"\"");
    format!("{} : \"{escaped}\"", col.column())
}

/// Build an FTS5 all-column literal phrase query (`"<escaped>"`).
pub(crate) fn fts_all_columns_phrase(query: &str) -> String {
    let escaped = query.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

enum CatalogState {
    Loaded(LangCol),
    NotFound(String),
    Error(String),
}

fn catalog_state(conn: &Connection, language: &str) -> Result<CatalogState> {
    if !is_safe_language_tag(language) {
        return Ok(CatalogState::Error(format!("语言代码不合法：{language}")));
    }
    let mut stmt = conn
        .prepare_cached(sql::TEXT_LANGUAGE_COLUMN)
        .map_err(|err| DbError::sqlite("准备检查本地化语言", err))?;
    let loaded: Option<String> = stmt
        .query_row([language], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite("检查本地化语言", err))?;
    if let Some(col_name) = loaded
        && let Some(col) = LangCol::from_column(&col_name)
    {
        Ok(CatalogState::Loaded(col))
    } else {
        Ok(CatalogState::NotFound(format!(
            "缺少语言 {language} 的本地化表（可运行 ../final-fantasy-resonance-export/scripts/run.sh locres --lang {language} 导出到 Localization/{language}.json）"
        )))
    }
}

fn lookup_text_col(conn: &Connection, ns: &str, key: &str, col: LangCol) -> Result<Option<String>> {
    let query_sql = match col {
        LangCol::Ja => sql::TEXT_BY_KEY_JA,
        LangCol::En => sql::TEXT_BY_KEY_EN,
        LangCol::Zh => sql::TEXT_BY_KEY_ZH,
    };
    let mut stmt = conn
        .prepare_cached(query_sql)
        .map_err(|err| DbError::sqlite("准备查询 text", err))?;
    let value: Option<Option<String>> = stmt
        .query_row(params![ns, key], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询文本 {ns}/{key}（{}）", col.lang()), err))?;
    Ok(value.flatten())
}

fn lookup_text_untranslated(
    conn: &Connection,
    ns: &str,
    key: &str,
    lang: &str,
) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT value FROM text_untranslated WHERE ns = ?1 AND key = ?2 AND lang = ?3",
        )
        .map_err(|err| DbError::sqlite("准备查询 text_untranslated", err))?;
    stmt.query_row(params![ns, key, lang], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询未翻译文本 {ns}/{key}（{lang}）"), err))
}

fn resolve_untranslated(
    native_text: String,
    language: &str,
    fallback_to_native: bool,
) -> ResolvedText {
    if !fallback_to_native {
        return missing(language, UNTRANSLATED_REASON);
    }
    ResolvedText {
        text: Some(native_text),
        lang: Some(NATIVE_LANGUAGE.to_owned()),
        requested_language: language.to_owned(),
        reason: Some(format!(
            "{UNTRANSLATED_REASON}，已回落到源语言 {NATIVE_LANGUAGE}"
        )),
        source: TextSource::Localization,
    }
}

fn string_table_namespace(conn: &Connection, table_id: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare_cached("SELECT ns FROM string_table WHERE table_id = ?1")
        .map_err(|err| DbError::sqlite("准备查询 string_table", err))?;
    stmt.query_row([table_id], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询字符串表 {table_id}"), err))
}

fn string_table_source(conn: &Connection, table_id: &str, key: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare_cached("SELECT source FROM string_table_entry WHERE table_id = ?1 AND key = ?2")
        .map_err(|err| DbError::sqlite("准备查询 string_table_entry", err))?;
    stmt.query_row(params![table_id, key], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询字符串表条目 {table_id}/{key}"), err))
}

fn resolve_string_table(
    conn: &Connection,
    table_id: &str,
    key: &str,
    language: &str,
    fallback_to_native: bool,
) -> Result<ResolvedText> {
    if ue_object_path_to_export_json(table_id, Path::new("")).is_none() {
        return Ok(missing(
            language,
            format!("无法把字符串表路径映射到导出目录：{table_id}"),
        ));
    }
    let Some(namespace) = string_table_namespace(conn, table_id)? else {
        return Ok(missing(language, format!("找不到字符串表文件：{table_id}")));
    };

    let catalog = catalog_state(conn, language)?;
    if let CatalogState::Error(reason) = &catalog {
        return Ok(missing(language, reason.clone()));
    }
    if let CatalogState::Loaded(col) = catalog {
        if let Some(value) = lookup_text_col(conn, &namespace, key, col)? {
            return Ok(localized(value, language, TextSource::Localization));
        }
        if let Some(value) = lookup_text_untranslated(conn, &namespace, key, language)? {
            return Ok(resolve_untranslated(value, language, fallback_to_native));
        }
    }

    let missing_reason = match catalog {
        CatalogState::NotFound(reason) => reason,
        _ => format!("语言 {language} 的本地化表中缺少 {namespace}/{key}"),
    };
    if language == NATIVE_LANGUAGE {
        let Some(found) = string_table_source(conn, table_id, key)? else {
            return Ok(missing(
                language,
                format!("字符串表 {namespace}（{table_id}）中不存在键 {key}"),
            ));
        };
        return Ok(localized(found, NATIVE_LANGUAGE, TextSource::StringTable));
    }
    if !fallback_to_native {
        return Ok(missing(language, missing_reason));
    }
    let Some(fallback_text) = string_table_source(conn, table_id, key)? else {
        return Ok(missing(
            language,
            format!("{missing_reason}；且源语言字符串表 {namespace} 中不存在键 {key}"),
        ));
    };
    Ok(ResolvedText {
        text: Some(fallback_text),
        lang: Some(NATIVE_LANGUAGE.to_owned()),
        requested_language: language.to_owned(),
        reason: Some(format!(
            "{missing_reason}，已回落到源语言 {NATIVE_LANGUAGE}"
        )),
        source: TextSource::StringTable,
    })
}

fn resolve_inline(
    conn: &Connection,
    namespace: &str,
    key: &str,
    source_string: Option<&str>,
    language: &str,
    fallback_to_native: bool,
) -> Result<ResolvedText> {
    let ns_label = if namespace.is_empty() {
        "(空命名空间)"
    } else {
        namespace
    };
    let catalog = catalog_state(conn, language)?;
    if let CatalogState::Error(reason) = &catalog {
        return Ok(missing(language, reason.clone()));
    }
    if let CatalogState::Loaded(col) = catalog {
        if let Some(value) = lookup_text_col(conn, namespace, key, col)? {
            return Ok(localized(value, language, TextSource::Localization));
        }
        if let Some(value) = lookup_text_untranslated(conn, namespace, key, language)? {
            return Ok(resolve_untranslated(value, language, fallback_to_native));
        }
    }

    let missing_reason = match catalog {
        CatalogState::NotFound(reason) => reason,
        _ => format!("语言 {language} 的本地化表中缺少内联文本 {ns_label}/{key}"),
    };
    if language == NATIVE_LANGUAGE {
        let Some(source_text) = source_string else {
            return Ok(missing(
                language,
                format!("内联 FText（{ns_label}/{key}）缺少 SourceString"),
            ));
        };
        return Ok(localized(
            source_text.to_owned(),
            NATIVE_LANGUAGE,
            TextSource::InlineSource,
        ));
    }
    if !fallback_to_native {
        return Ok(missing(language, missing_reason));
    }
    let Some(source_text) = source_string else {
        return Ok(missing(
            language,
            format!("{missing_reason}；且内联 FText 缺少 SourceString"),
        ));
    };
    Ok(ResolvedText {
        text: Some(source_text.to_owned()),
        lang: Some(NATIVE_LANGUAGE.to_owned()),
        requested_language: language.to_owned(),
        reason: Some(format!(
            "{missing_reason}，已回落到源语言 {NATIVE_LANGUAGE}"
        )),
        source: TextSource::InlineSource,
    })
}

fn localized(text: String, language: &str, source: TextSource) -> ResolvedText {
    ResolvedText {
        text: Some(text),
        lang: Some(language.to_owned()),
        requested_language: language.to_owned(),
        reason: None,
        source,
    }
}

fn missing(requested_language: &str, reason: impl Into<String>) -> ResolvedText {
    ResolvedText {
        text: None,
        lang: None,
        requested_language: requested_language.to_owned(),
        reason: Some(reason.into()),
        source: TextSource::Missing,
    }
}
