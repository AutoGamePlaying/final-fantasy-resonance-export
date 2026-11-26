//! Text area (`01_text`) import stage.

use std::collections::{BTreeMap, BTreeSet};

use ffr_data::{
    DEFAULT_LOCRES_LANGUAGES, NATIVE_LANGUAGE, datatable_dir, load_localization, localization_dir,
    localization_file, parse_string_table_document, untranslated_reason_code,
};
use rusqlite::params;

use crate::build::datatable::{export_type, json_files_under, read_export_json};
use crate::build::{ImportCtx, StageStats};
use crate::error::{DbError, Result};

/// Import multilingual localization text (translated + native `ja` into `text`, untranslated
/// non-native entries into `text_untranslated`) and string table entries in natural PK order.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let export_root = ctx.require_data_store()?.export_root();
    let loc_dir = localization_dir(export_root);
    let table_root = datatable_dir(export_root);
    let table_files = json_files_under(&table_root)?;

    let mut languages: BTreeSet<&'static str> = BTreeSet::new();
    // Group by `(ns, key)` -> `lang -> value` so iteration follows natural PK `(ns, key, lang)`
    // order while giving O(1) access to the native `"ja"` string for untranslated detection.
    let mut text_by_key: BTreeMap<(String, String), BTreeMap<&'static str, String>> =
        BTreeMap::new();
    for language in DEFAULT_LOCRES_LANGUAGES {
        let path = localization_file(export_root, language);
        if !path.is_file() {
            continue;
        }
        let catalog = load_localization(&path)?;
        languages.insert(language);
        for (namespace, entries) in catalog.namespaces {
            for (key, value) in entries {
                let by_lang = text_by_key.entry((namespace.clone(), key)).or_default();
                by_lang.insert(language, value);
            }
        }
    }

    let mut string_tables: BTreeMap<String, ffr_data::StringTable> = BTreeMap::new();
    for file in &table_files {
        let document = read_export_json(file)?;
        if export_type(&document) != Some("StringTable") {
            continue;
        }
        let label = file.display().to_string();
        let table = parse_string_table_document(&document, &label)?;
        let table_id = format!("{}.{}", table.package_path, table.name);
        string_tables.insert(table_id, table);
    }

    let language_count = languages.len() as u64;
    let string_table_count = string_tables.len() as u64;
    let mut translated_text_count = 0_u64;
    let mut untranslated_count = 0_u64;
    let mut string_entry_count = 0_u64;
    let mut untranslated_by_lang: BTreeMap<&'static str, u64> = languages
        .iter()
        .copied()
        .filter(|&lang| lang != NATIVE_LANGUAGE)
        .map(|lang| (lang, 0_u64))
        .collect();

    if language_count == 0 && string_table_count == 0 {
        return Ok(StageStats::skipped("未实现"));
    }

    {
        let tx = ctx.tx();
        let mut insert_language = tx
            .prepare_cached("INSERT INTO text_language (lang) VALUES (?1)")
            .map_err(|err| DbError::sqlite("准备写入 text_language", err))?;
        let mut insert_text = tx
            .prepare_cached(
                "INSERT INTO text (id, ns, key, lang, value) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| DbError::sqlite("准备写入 text", err))?;
        let mut insert_untranslated = tx
            .prepare_cached(
                "INSERT INTO text_untranslated (ns, key, lang, value, reason) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| DbError::sqlite("准备写入 text_untranslated", err))?;
        let mut insert_string_table = tx
            .prepare_cached("INSERT INTO string_table (table_id, ns) VALUES (?1, ?2)")
            .map_err(|err| DbError::sqlite("准备写入 string_table", err))?;
        let mut insert_string_entry = tx
            .prepare_cached(
                "INSERT INTO string_table_entry (table_id, key, source) VALUES (?1, ?2, ?3)",
            )
            .map_err(|err| DbError::sqlite("准备写入 string_table_entry", err))?;

        for language in &languages {
            insert_language
                .execute([language])
                .map_err(|err| DbError::sqlite(format!("写入语言 {language}"), err))?;
        }

        let mut text_id = 0_i64;
        for ((namespace, key), by_lang) in &text_by_key {
            let ja_value = by_lang.get(NATIVE_LANGUAGE).map(String::as_str);
            for (&language, value) in by_lang {
                if let Some(reason) = untranslated_reason_code(language, value, ja_value) {
                    insert_untranslated
                        .execute(params![namespace, key, language, value, reason])
                        .map_err(|err| {
                            DbError::sqlite(
                                format!("写入未翻译文本 {namespace}/{key}（{language}）"),
                                err,
                            )
                        })?;
                    *untranslated_by_lang.entry(language).or_insert(0) += 1;
                    untranslated_count = untranslated_count.saturating_add(1);
                } else {
                    text_id = text_id.saturating_add(1);
                    insert_text
                        .execute(params![text_id, namespace, key, language, value])
                        .map_err(|err| {
                            DbError::sqlite(
                                format!("写入文本 {namespace}/{key}（{language}）"),
                                err,
                            )
                        })?;
                    translated_text_count = translated_text_count.saturating_add(1);
                }
            }
        }

        for (table_id, table) in &string_tables {
            insert_string_table
                .execute(params![table_id, table.table_namespace])
                .map_err(|err| DbError::sqlite(format!("写入字符串表 {table_id}"), err))?;
            for (key, source) in &table.entries {
                insert_string_entry
                    .execute(params![table_id, key, source])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入字符串表条目 {table_id}/{key}"), err)
                    })?;
                string_entry_count = string_entry_count.saturating_add(1);
            }
        }

        let mut rebuild_fts = tx
            .prepare_cached(
                "INSERT INTO text_fts (rowid, value) SELECT id, value FROM text ORDER BY id",
            )
            .map_err(|err| DbError::sqlite("准备构建 text_fts", err))?;
        rebuild_fts
            .execute([])
            .map_err(|err| DbError::sqlite("构建 text_fts", err))?;
    }

    if string_table_count > 0 {
        ctx.record_dir_source(&table_root, Some(string_table_count))?;
    }
    if loc_dir.is_dir() {
        ctx.record_dir_source(&loc_dir, Some(language_count))?;
    }
    let untranslated_detail = if untranslated_by_lang.is_empty() {
        format!("text_untranslated={untranslated_count}")
    } else {
        let parts: Vec<String> = untranslated_by_lang
            .iter()
            .map(|(lang, count)| format!("{lang}={count}"))
            .collect();
        format!(
            "text_untranslated={untranslated_count}({})",
            parts.join(", ")
        )
    };
    Ok(StageStats::ok_with_note(
        translated_text_count
            .saturating_add(untranslated_count)
            .saturating_add(string_entry_count),
        format!(
            "text={translated_text_count}, {untranslated_detail}, string_table={string_table_count}, string_table_entry={string_entry_count}"
        ),
    ))
}
