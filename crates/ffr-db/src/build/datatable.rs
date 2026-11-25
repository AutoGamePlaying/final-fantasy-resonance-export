//! Raw DataTable area (`02_datatable`) import stage.

use std::fs;
use std::path::{Path, PathBuf};

use ffr_data::{DataTableType, datatable_dir, parse_data_table_document};
use rusqlite::params;
use serde_json::Value;

use crate::build::{ImportCtx, StageStats, relative_label_under};
use crate::error::{DbError, Result};
use crate::record::encode_doc;
use crate::util::path_to_slash_string;

/// Import raw DataTable metadata and rows in normalized relative-path order.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let export_root = ctx.require_data_store()?.export_root();
    let root = datatable_dir(export_root);
    let files = json_files_under(&root)?;
    let mut tables = 0_u64;
    let mut rows = 0_u64;

    {
        let tx = ctx.tx();
        let mut insert_table = tx
            .prepare_cached(
                "INSERT INTO dt_table (path, kind, row_struct, row_count) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 dt_table", err))?;
        let mut insert_row = tx
            .prepare_cached("INSERT INTO dt_row (table_id, row_name, doc) VALUES (?1, ?2, ?3)")
            .map_err(|err| DbError::sqlite("准备写入 dt_row", err))?;

        for file in &files {
            let document = read_export_json(file)?;
            match export_type(&document) {
                Some("DataTable" | "CompositeDataTable") => {}
                Some("StringTable") => continue,
                Some(_) => continue,
                None => {
                    let subject = relative_label_under(export_root, "export", file);
                    ctx.record_issue(
                        "missing_table_type",
                        &subject,
                        "导出 JSON 首项缺少 Type，已跳过",
                    )?;
                    continue;
                }
            }

            let Some(relative) = relative_table_path(&root, file) else {
                let subject = relative_label_under(export_root, "export", file);
                ctx.record_issue(
                    "invalid_table_path",
                    &subject,
                    "无法转换为 Datatable 下的 UTF-8 相对路径，已跳过",
                )?;
                continue;
            };
            let label = relative_label_under(export_root, "export", file);
            let table = parse_data_table_document(&document, &label)?;
            let kind = match table.table_type {
                DataTableType::DataTable => "DataTable",
                DataTableType::CompositeDataTable => "CompositeDataTable",
            };
            let row_count = i64::try_from(table.rows.len()).unwrap_or(i64::MAX);
            insert_table
                .execute(params![relative, kind, table.row_struct, row_count])
                .map_err(|err| DbError::sqlite(format!("写入 dt_table {relative}"), err))?;
            let table_id = tx.last_insert_rowid();
            for (row_name, row) in &table.rows {
                let key = format!("{relative}/{row_name}");
                let doc = encode_doc("dt_row", key, row)?;
                insert_row
                    .execute(params![table_id, row_name, doc])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 dt_row {relative}/{row_name}"), err)
                    })?;
                rows = rows.saturating_add(1);
            }
            tables = tables.saturating_add(1);
        }
    }

    if tables == 0 {
        return Ok(StageStats::skipped("未实现"));
    }
    ctx.record_dir_source(&root, Some(files.len() as u64))?;
    Ok(StageStats::ok_with_note(
        tables.saturating_add(rows),
        format!("dt_table={tables}, dt_row={rows}"),
    ))
}

pub(crate) fn json_files_under(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if root.is_dir() {
        collect_json_files(root, &mut files)?;
    }
    // Sort by slash-normalized relative table path so Linux and Windows visit files identically.
    files.sort_by(|a, b| {
        let ka = relative_sort_key(root, a);
        let kb = relative_sort_key(root, b);
        ka.cmp(&kb)
    });
    Ok(files)
}

fn relative_sort_key(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    let slash = path_to_slash_string(rel);
    slash.strip_suffix(".json").unwrap_or(&slash).to_owned()
}

fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|err| DbError::io("遍历数据表目录", dir, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("读取数据表目录项", dir, err))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| DbError::io("读取数据表文件类型", &path, err))?;
        if file_type.is_dir() {
            collect_json_files(&path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) fn read_export_json(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path).map_err(|err| DbError::io("读取导出 JSON", path, err))?;
    serde_json::from_str(&raw).map_err(|source| DbError::DocDeserialize {
        table: "export_json".to_owned(),
        key: path.display().to_string(),
        source,
    })
}

pub(crate) fn export_type(document: &Value) -> Option<&str> {
    document
        .as_array()?
        .first()?
        .as_object()?
        .get("Type")?
        .as_str()
}

fn relative_table_path(root: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(root).ok()?.to_str()?;
    let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
    Some(
        relative
            .strip_suffix(".json")
            .unwrap_or(&relative)
            .to_owned(),
    )
}
