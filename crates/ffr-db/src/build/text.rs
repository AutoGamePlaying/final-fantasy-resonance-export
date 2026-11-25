//! Text area (`01_text`) import stage.

use std::collections::{BTreeMap, BTreeSet};

use ffr_data::{
    DEFAULT_LOCRES_LANGUAGES, datatable_dir, load_localization, localization_dir,
    localization_file, parse_string_table_document,
};
use rusqlite::params;

use crate::build::datatable::{export_type, json_files_under, read_export_json};
use crate::build::{ImportCtx, StageStats};
use crate::error::{DbError, Result};

/// Import multilingual localization text and string table entries in natural primary-key order.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let export_root = ctx.require_data_store()?.export_root();
    let loc_dir = localization_dir(export_root);
    let table_root = datatable_dir(export_root);
    let table_files = json_files_under(&table_root)?;

    let mut languages: BTreeSet<&'static str> = BTreeSet::new();
    let mut text_entries: BTreeMap<(String, String, &'static str), String> = BTreeMap::new();
    for language in DEFAULT_LOCRES_LANGUAGES {
        let path = localization_file(export_root, language);
        if !path.is_file() {
            continue;
        }
        let catalog = load_localization(&path)?;
        languages.insert(language);
        for (namespace, entries) in catalog.namespaces {
            for (key, value) in entries {
                text_entries.insert((namespace.clone(), key, language), value);
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
    let text_count = text_entries.len() as u64;
    let string_table_count = string_tables.len() as u64;
    let mut string_entry_count = 0_u64;

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

        for (idx, ((namespace, key, language), value)) in text_entries.into_iter().enumerate() {
            let text_id = i64::try_from(idx + 1).unwrap_or(i64::MAX);
            insert_text
                .execute(params![text_id, namespace, key, language, value])
                .map_err(|err| {
                    DbError::sqlite(format!("写入文本 {namespace}/{key}（{language}）"), err)
                })?;
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
    Ok(StageStats::ok_with_note(
        text_count.saturating_add(string_entry_count),
        format!(
            "text={text_count}, string_table={string_table_count}, string_table_entry={string_entry_count}"
        ),
    ))
}
