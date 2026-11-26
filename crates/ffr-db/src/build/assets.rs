//! Asset index area (`05_assets`) import stage.
//!
//! Only exported JSON packages have outgoing references. Image, mesh, audio, and other skipped
//! packages can still have incoming references, but no dependency graph is inferred for them.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use ffr_data::assets::config::parse_ini;
use ffr_data::assets::registry::{
    AssetPackageRef, AssetRefKind, collect_package_refs, parse_registry_jsonl,
};
use ffr_data::error::DataError;
use rusqlite::params;

use crate::build::{ImportCtx, StageStats, UePathLookup};
use crate::error::{DbError, Result};
use crate::record::encode_doc;
use crate::util::path_to_slash_string;

const EXCLUDED_TAGS: [&str; 1] = ["ActorsMetaData"];

pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let inventory_dir = ctx.inputs().inventory.as_deref();
    let full_dir = ctx.inputs().full.as_deref();
    let registry_path = inventory_dir
        .map(|dir| dir.join("registry.jsonl"))
        .filter(|path| path.is_file());

    let mut entries = match registry_path.as_deref() {
        Some(path) => {
            let file = File::open(path).map_err(|source| DataError::Io {
                kind: "资源注册表",
                path: path.to_path_buf(),
                source,
            })?;
            parse_registry_jsonl(BufReader::new(file))?
        }
        None => Vec::new(),
    };
    // Sort assets by natural unique key (`path`) so assigned `asset.id` is deterministic.
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let ini_files = match full_dir {
        Some(dir) if dir.join("FFRS/Config").is_dir() => {
            let mut files = Vec::new();
            collect_ini_files(&dir.join("FFRS/Config"), &mut files)?;
            files.sort_by(|a, b| {
                let ra = path_to_slash_string(a.strip_prefix(dir).unwrap_or(a));
                let rb = path_to_slash_string(b.strip_prefix(dir).unwrap_or(b));
                ra.cmp(&rb)
            });
            files
        }
        _ => Vec::new(),
    };

    if entries.is_empty() && ini_files.is_empty() {
        return Ok(StageStats::skipped("未实现"));
    }

    let mut package_ids: UePathLookup<i64> = UePathLookup::new();
    let mut source_paths: BTreeMap<i64, usize> = BTreeMap::new();
    let mut asset_rows = 0_u64;
    let mut tag_rows = 0_u64;
    let mut excluded_tag_stats: BTreeMap<&str, (u64, u64)> =
        EXCLUDED_TAGS.iter().map(|key| (*key, (0, 0))).collect();
    let mut exported = Vec::new();
    for (entry_idx, entry) in entries.iter_mut().enumerate() {
        if let Some(full_dir) = full_dir {
            let relative = export_relative_path(&entry.path);
            if full_dir.join(&relative).is_file() {
                entry.export_path = Some(relative);
            }
        }
        entry.tags.sort_by(|a, b| a.key.cmp(&b.key));
        let tags = std::mem::take(&mut entry.tags);
        let doc = encode_doc("asset", &entry.path, entry)?;
        entry.tags = tags;
        let mut asset_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO asset (\
                    path, package_name, primary_class, asset_name, size, package_size, bulk_size, \
                    category, decision, has_export, export_path, doc\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .map_err(|err| DbError::sqlite("准备写入 asset", err))?;
        asset_stmt
            .execute(params![
                entry.path,
                entry.package_name,
                entry.primary_class,
                entry.asset_name,
                entry.size,
                entry.package_size,
                entry.bulk_size,
                entry.category,
                entry.decision,
                entry.export_path.is_some(),
                entry.export_path,
                doc,
            ])
            .map_err(|err| DbError::sqlite(format!("写入 asset {}", entry.path), err))?;
        let asset_id = ctx.tx().last_insert_rowid();
        drop(asset_stmt);
        package_ids.insert(entry.package_name.clone(), asset_id);
        source_paths.insert(asset_id, entry_idx);
        asset_rows += 1;

        if !entry.tags.is_empty() {
            let mut tag_stmt = ctx
                .tx()
                .prepare_cached(
                    "INSERT INTO asset_tag (asset_id, key, value, value_short) \
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|err| DbError::sqlite("准备写入 asset_tag", err))?;
            for tag in &entry.tags {
                if let Some((count, bytes)) = excluded_tag_stats.get_mut(tag.key.as_str()) {
                    *count += 1;
                    *bytes += tag.value.len() as u64;
                    continue;
                }
                let value_short = (tag.value.len() <= 256).then_some(tag.value.as_str());
                tag_stmt
                    .execute(params![asset_id, tag.key, tag.value, value_short])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 asset_tag {}", entry.path), err)
                    })?;
                tag_rows += 1;
            }
        }
        if let Some(relative) = entry.export_path.clone() {
            exported.push((asset_id, entry.package_name.clone(), relative));
        }
    }

    let mut references: Vec<(i64, AssetPackageRef)> = Vec::new();
    if let Some(full_dir) = full_dir {
        let collected = parse_exports_parallel(full_dir, &exported, ctx.worker_threads())?;
        references = collected.into_iter().collect();
        // Sort by exact SQL PRIMARY KEY `(from_asset_id, to_package, kind)` string order.
        references.sort_by(|(a_id, a_ref), (b_id, b_ref)| {
            a_id.cmp(b_id)
                .then_with(|| a_ref.to_package.cmp(&b_ref.to_package))
                .then_with(|| a_ref.kind.as_str().cmp(b_ref.kind.as_str()))
        });
    }

    let mut unresolved = 0_u64;
    {
        let mut ref_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO asset_ref (from_asset_id, to_package, to_asset_id, kind) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 asset_ref", err))?;
        for (from_asset_id, reference) in &references {
            let to_asset_id = package_ids.get(&reference.to_package).copied();
            ref_stmt
                .execute(params![
                    from_asset_id,
                    reference.to_package,
                    to_asset_id,
                    reference.kind.as_str(),
                ])
                .map_err(|err| DbError::sqlite("写入 asset_ref", err))?;
            if to_asset_id.is_none() && reference.kind != AssetRefKind::Script {
                unresolved += 1;
            }
        }
    }

    package_ids.record_collisions(ctx)?;

    for (from_asset_id, reference) in &references {
        if reference.kind != AssetRefKind::Script && !package_ids.contains(&reference.to_package) {
            let subject = source_paths
                .get(from_asset_id)
                .map(|&idx| entries[idx].path.as_str())
                .unwrap_or("unknown");
            ctx.record_issue(
                "dangling_asset_ref",
                subject,
                &format!(
                    "kind={} to_package={}",
                    reference.kind.as_str(),
                    reference.to_package
                ),
            )?;
        }
    }

    // Record sources in sorted logical path order (`export-full` before `inventory`).
    if let Some(full_dir) = full_dir {
        ctx.record_dir_source(full_dir, Some(exported.len() as u64))?;
    }
    if let Some(dir) = inventory_dir.filter(|_| registry_path.is_some()) {
        ctx.record_dir_source(dir, Some(entries.len() as u64))?;
    }

    let mut config_rows = 0_u64;
    for path in &ini_files {
        let mut contents = String::new();
        File::open(path)
            .and_then(|mut file| file.read_to_string(&mut contents))
            .map_err(|source| DataError::Io {
                kind: "配置文件",
                path: path.clone(),
                source,
            })?;
        let relative = path_to_slash_string(
            path.strip_prefix(full_dir.expect("INI files require a full export directory"))
                .expect("INI file must be below the full export directory"),
        );
        let config_entries = parse_ini(&relative, &contents)?;
        let mut config_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO config_entry (file, section, key, value, op, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 config_entry", err))?;
        for entry in config_entries {
            config_stmt
                .execute(params![
                    entry.file,
                    entry.section,
                    entry.key,
                    entry.value,
                    entry.op,
                    entry.ord,
                ])
                .map_err(|err| DbError::sqlite(format!("写入 config_entry {relative}"), err))?;
            config_rows += 1;
        }
    }

    let excluded_note = EXCLUDED_TAGS
        .iter()
        .map(|key| {
            let (count, bytes) = excluded_tag_stats[key];
            format!("{key}({count} 条,{bytes} 字节)")
        })
        .collect::<Vec<_>>()
        .join("、");
    let total = asset_rows + tag_rows + references.len() as u64 + config_rows;
    Ok(StageStats::ok_with_note(
        total,
        format!(
            "assets={asset_rows} tags={tag_rows} exports={} refs={} unresolved={unresolved} configs={config_rows}；排除标签={excluded_note}；原始数据在 registry.jsonl",
            exported.len(),
            references.len()
        ),
    ))
}

fn parse_exports_parallel(
    full_dir: &Path,
    exported: &[(i64, String, String)],
    max_workers: usize,
) -> Result<BTreeSet<(i64, AssetPackageRef)>> {
    if exported.is_empty() {
        return Ok(BTreeSet::new());
    }
    let workers = max_workers.max(1).min(exported.len());
    let batch_size = exported.len().div_ceil(workers * 4);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for batch in exported.chunks(batch_size) {
            handles.push(scope.spawn(move || parse_export_batch(full_dir, batch)));
        }
        let mut references = BTreeSet::new();
        for handle in handles {
            let batch_refs = handle.join().map_err(|_| DbError::StageFailed {
                stage: "assets",
                detail: "解析导出 JSON 的线程异常终止".to_owned(),
            })??;
            references.extend(batch_refs);
        }
        Ok(references)
    })
}

fn parse_export_batch(
    full_dir: &Path,
    batch: &[(i64, String, String)],
) -> Result<BTreeSet<(i64, AssetPackageRef)>> {
    let mut references = BTreeSet::new();
    for (asset_id, package_name, relative) in batch {
        let path = full_dir.join(relative);
        let file = File::open(&path).map_err(|source| DataError::Io {
            kind: "全量资源导出",
            path: path.clone(),
            source,
        })?;
        let document =
            serde_json::from_reader(BufReader::new(file)).map_err(|source| DataError::Json {
                kind: "全量资源导出",
                path: path.clone(),
                source,
            })?;
        for reference in collect_package_refs(&document, package_name) {
            references.insert((*asset_id, reference));
        }
    }
    Ok(references)
}

fn export_relative_path(asset_path: &str) -> String {
    Path::new(asset_path)
        .with_extension("json")
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_ini_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|err| DbError::io("遍历配置目录", dir, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("读取配置目录项", dir, err))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| DbError::io("读取配置目录项类型", &path, err))?;
        if file_type.is_dir() {
            collect_ini_files(&path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ini"))
        {
            files.push(path);
        }
    }
    Ok(())
}
