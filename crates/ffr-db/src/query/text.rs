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

impl GameDb {
    /// Look up one translated localized string (or native `ja` string) by its primary key.
    ///
    /// Game-official untranslated entries live in `text_untranslated` instead of `text`, so this
    /// strictly returns `None` for them.
    pub fn text(&self, ns: &str, key: &str, lang: &str) -> Result<Option<String>> {
        self.with_conn(|conn| lookup_text(conn, ns, key, lang))
    }

    /// Return all translated entries (plus native `ja`) as `(lang, value)` pairs ordered by
    /// language.
    pub fn text_all_langs(&self, ns: &str, key: &str) -> Result<Vec<(String, String)>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT lang, value FROM text \
                     WHERE ns = ?1 AND key = ?2 \
                     ORDER BY lang",
                )
                .map_err(|err| DbError::sqlite("准备查询 text 的全部语言", err))?;
            let mut rows = stmt
                .query(params![ns, key])
                .map_err(|err| DbError::sqlite("查询 text 的全部语言", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 text 的全部语言", err))?
            {
                out.push((
                    row.get(0)
                        .map_err(|err| DbError::sqlite("读取 text.lang", err))?,
                    row.get(1)
                        .map_err(|err| DbError::sqlite("读取 text.value", err))?,
                ));
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

    /// Search localized values in `text` by literal substring, using trigram FTS or a short-query
    /// LIKE fallback.
    pub fn search_text(
        &self,
        query: &str,
        lang: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TextHit>> {
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.with_conn(|conn| {
            if query.chars().count() >= 3 {
                let match_query = format!("\"{}\"", query.replace('"', "\"\""));
                if let Some(lang) = lang {
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT t.ns, t.key, t.lang, t.value \
                             FROM text_fts JOIN text AS t ON t.id = text_fts.rowid \
                             WHERE text_fts MATCH ?1 AND t.lang = ?2 \
                             ORDER BY t.ns, t.key LIMIT ?3",
                        )
                        .map_err(|err| DbError::sqlite("准备按语言 trigram 文本搜索", err))?;
                    query_hits(&mut stmt, params![match_query, lang, limit])
                } else {
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT t.ns, t.key, t.lang, t.value \
                             FROM text_fts JOIN text AS t ON t.id = text_fts.rowid \
                             WHERE text_fts MATCH ?1 \
                             ORDER BY t.ns, t.key, t.lang LIMIT ?2",
                        )
                        .map_err(|err| DbError::sqlite("准备全语言 trigram 文本搜索", err))?;
                    query_hits(&mut stmt, params![match_query, limit])
                }
            } else {
                let pattern = like_pattern(query);
                if let Some(lang) = lang {
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT ns, key, lang, value \
                             FROM text INDEXED BY idx_text_lang_ns_key \
                             WHERE lang = ?1 AND value LIKE ?2 ESCAPE '\\' \
                             ORDER BY ns, key LIMIT ?3",
                        )
                        .map_err(|err| DbError::sqlite("准备按语言短文本 LIKE 搜索", err))?;
                    query_hits(&mut stmt, params![lang, pattern, limit])
                } else {
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT ns, key, lang, value \
                             FROM text INDEXED BY idx_text_lang_ns_key \
                             WHERE value LIKE ?1 ESCAPE '\\' \
                             ORDER BY ns, key, lang LIMIT ?2",
                        )
                        .map_err(|err| DbError::sqlite("准备全语言短文本 LIKE 搜索", err))?;
                    query_hits(&mut stmt, params![pattern, limit])
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

fn like_pattern(query: &str) -> String {
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

enum CatalogState {
    Loaded,
    NotFound(String),
    Error(String),
}

fn catalog_state(conn: &Connection, language: &str) -> Result<CatalogState> {
    if !is_safe_language_tag(language) {
        return Ok(CatalogState::Error(format!("语言代码不合法：{language}")));
    }
    let mut stmt = conn
        .prepare_cached("SELECT 1 FROM text_language WHERE lang = ?1")
        .map_err(|err| DbError::sqlite("准备检查本地化语言", err))?;
    let loaded: Option<i64> = stmt
        .query_row([language], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite("检查本地化语言", err))?;
    if loaded.is_some() {
        Ok(CatalogState::Loaded)
    } else {
        Ok(CatalogState::NotFound(format!(
            "缺少语言 {language} 的本地化表（可运行 ../final-fantasy-resonance-export/scripts/run.sh locres --lang {language} 导出到 Localization/{language}.json）"
        )))
    }
}

fn lookup_text(conn: &Connection, ns: &str, key: &str, lang: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare_cached("SELECT value FROM text WHERE ns = ?1 AND key = ?2 AND lang = ?3")
        .map_err(|err| DbError::sqlite("准备查询 text", err))?;
    stmt.query_row(params![ns, key, lang], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询文本 {ns}/{key}（{lang}）"), err))
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
    if matches!(catalog, CatalogState::Loaded) {
        if let Some(value) = lookup_text(conn, &namespace, key, language)? {
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
    if matches!(catalog, CatalogState::Loaded) {
        if let Some(value) = lookup_text(conn, namespace, key, language)? {
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
