//! Asset index area (`05_assets`) queries.
//!
//! Outgoing references cover exported JSON only; non-exported images, meshes, audio, and other
//! binary packages have no outgoing edges even though incoming edges can point to them.

use std::fs::File;
use std::io::BufReader;
use std::path::{Component, Path};

use ffr_data::assets::config::ConfigEntry;
use ffr_data::assets::registry::{AssetEntry, AssetRef, AssetRefKind, AssetTag};
use ffr_data::error::DataError;
use rusqlite::{Connection, OptionalExtension, Params, Row, params};
use serde_json::Value;

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record::{decode_doc, query_doc_opt};

impl GameDb {
    pub fn asset(&self, path_or_package: &str) -> Result<Option<AssetEntry>> {
        self.with_conn(|conn| {
            let Some(asset_id) = resolve_asset_id(conn, path_or_package)? else {
                return Ok(None);
            };
            let mut entry = query_doc_opt(
                conn,
                "asset",
                asset_id,
                "SELECT doc FROM asset WHERE id = ?1",
                [asset_id],
            )?;
            if let Some(entry) = &mut entry {
                load_asset_tags(conn, asset_id, entry)?;
            }
            Ok(entry)
        })
    }

    pub fn assets_by_class(&self, class: &str, limit: u32) -> Result<Vec<AssetEntry>> {
        self.with_conn(|conn| {
            query_asset_list(
                conn,
                "SELECT id, doc FROM asset WHERE primary_class = ?1 \
                 ORDER BY path LIMIT ?2",
                params![class, limit],
            )
        })
    }

    pub fn assets_in_dir(&self, prefix: &str, limit: u32) -> Result<Vec<AssetEntry>> {
        let upper = format!("{prefix}\u{10ffff}");
        self.with_conn(|conn| {
            query_asset_list(
                conn,
                "SELECT id, doc FROM asset WHERE path >= ?1 AND path < ?2 \
                 ORDER BY path LIMIT ?3",
                params![prefix, upper, limit],
            )
        })
    }

    pub fn asset_refs(&self, path_or_package: &str) -> Result<Vec<AssetRef>> {
        self.with_conn(|conn| {
            let Some(asset_id) = resolve_asset_id(conn, path_or_package)? else {
                return Ok(Vec::new());
            };
            query_asset_refs(
                conn,
                "SELECT asset_ref.from_asset_id, source_asset.path, source_asset.package_name, \
                        asset_ref.to_package, asset_ref.to_asset_id, target.path, asset_ref.kind \
                 FROM asset_ref \
                 JOIN asset AS source_asset ON source_asset.id = asset_ref.from_asset_id \
                 LEFT JOIN asset AS target ON target.id = asset_ref.to_asset_id \
                 WHERE asset_ref.from_asset_id = ?1 \
                 ORDER BY asset_ref.to_package, asset_ref.kind",
                asset_id,
            )
        })
    }

    pub fn asset_referrers(&self, path_or_package: &str) -> Result<Vec<AssetRef>> {
        self.with_conn(|conn| {
            let Some(asset_id) = resolve_asset_id(conn, path_or_package)? else {
                return Ok(Vec::new());
            };
            query_asset_refs(
                conn,
                "SELECT asset_ref.from_asset_id, source_asset.path, source_asset.package_name, \
                        asset_ref.to_package, asset_ref.to_asset_id, target.path, asset_ref.kind \
                 FROM asset_ref \
                 JOIN asset AS source_asset ON source_asset.id = asset_ref.from_asset_id \
                 JOIN asset AS target ON target.id = asset_ref.to_asset_id \
                 WHERE asset_ref.to_asset_id = ?1 \
                 ORDER BY source_asset.path, asset_ref.kind",
                asset_id,
            )
        })
    }

    pub fn config_values(&self, section: &str, key: &str) -> Result<Vec<ConfigEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT file, section, key, value, op, ord FROM config_entry \
                     WHERE section = ?1 AND key = ?2 ORDER BY file, ord",
                )
                .map_err(|err| DbError::sqlite("准备查询 config_entry", err))?;
            let mut rows = stmt
                .query(params![section, key])
                .map_err(|err| DbError::sqlite("查询 config_entry", err))?;
            let mut entries = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 config_entry 行", err))?
            {
                entries.push(ConfigEntry {
                    file: row
                        .get(0)
                        .map_err(|err| DbError::sqlite("读取 config_entry.file", err))?,
                    section: row
                        .get(1)
                        .map_err(|err| DbError::sqlite("读取 config_entry.section", err))?,
                    key: row
                        .get(2)
                        .map_err(|err| DbError::sqlite("读取 config_entry.key", err))?,
                    value: row
                        .get(3)
                        .map_err(|err| DbError::sqlite("读取 config_entry.value", err))?,
                    op: row
                        .get(4)
                        .map_err(|err| DbError::sqlite("读取 config_entry.op", err))?,
                    ord: row
                        .get(5)
                        .map_err(|err| DbError::sqlite("读取 config_entry.ord", err))?,
                });
            }
            Ok(entries)
        })
    }
}

pub fn read_export_doc(export_full_dir: impl AsRef<Path>, entry: &AssetEntry) -> Result<Value> {
    let relative = entry
        .export_path
        .as_deref()
        .ok_or_else(|| DataError::InvalidPath {
            kind: "资源导出 JSON（该包未导出）",
            input: entry.path.clone(),
        })?;
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || !relative_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(DataError::InvalidPath {
            kind: "资源导出 JSON",
            input: relative.to_owned(),
        }
        .into());
    }
    let path = export_full_dir.as_ref().join(relative_path);
    let file = File::open(&path).map_err(|source| DataError::Io {
        kind: "资源导出 JSON",
        path: path.clone(),
        source,
    })?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|source| DataError::Json {
            kind: "资源导出 JSON",
            path,
            source,
        })
        .map_err(Into::into)
}

fn query_asset_list<P: Params>(conn: &Connection, sql: &str, params: P) -> Result<Vec<AssetEntry>> {
    let mut assets = Vec::new();
    {
        let mut stmt = conn
            .prepare_cached(sql)
            .map_err(|err| DbError::sqlite("准备批量查询 asset", err))?;
        let mut rows = stmt
            .query(params)
            .map_err(|err| DbError::sqlite("批量查询 asset", err))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| DbError::sqlite("读取 asset 查询结果", err))?
        {
            let id: i64 = row
                .get(0)
                .map_err(|err| DbError::sqlite("读取 asset.id", err))?;
            let raw: String = row
                .get(1)
                .map_err(|err| DbError::sqlite(format!("读取 asset {id} 的 doc"), err))?;
            assets.push((id, decode_doc("asset", id, &raw)?));
        }
    }
    for (id, entry) in &mut assets {
        load_asset_tags(conn, *id, entry)?;
    }
    Ok(assets
        .into_iter()
        .map(|(_, entry)| entry)
        .collect::<Vec<AssetEntry>>())
}

fn load_asset_tags(conn: &Connection, asset_id: i64, entry: &mut AssetEntry) -> Result<()> {
    let mut stmt = conn
        .prepare_cached("SELECT key, value FROM asset_tag WHERE asset_id = ?1 ORDER BY key")
        .map_err(|err| DbError::sqlite("准备查询 asset_tag", err))?;
    let mut rows = stmt
        .query([asset_id])
        .map_err(|err| DbError::sqlite("查询 asset_tag", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 asset_tag 行", err))?
    {
        entry.tags.push(AssetTag {
            key: row
                .get(0)
                .map_err(|err| DbError::sqlite("读取 asset_tag.key", err))?,
            value: row
                .get(1)
                .map_err(|err| DbError::sqlite("读取 asset_tag.value", err))?,
        });
    }
    Ok(())
}

fn resolve_asset_id(conn: &Connection, path_or_package: &str) -> Result<Option<i64>> {
    conn.prepare_cached(
        "SELECT id FROM asset WHERE path = ?1 \
         UNION ALL SELECT id FROM asset WHERE package_name = ?1 LIMIT 1",
    )
    .map_err(|err| DbError::sqlite("准备定位 asset", err))?
    .query_row([path_or_package], |row| row.get(0))
    .optional()
    .map_err(|err| DbError::sqlite("定位 asset", err))
}

fn query_asset_refs(conn: &Connection, sql: &str, asset_id: i64) -> Result<Vec<AssetRef>> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite("准备查询 asset_ref", err))?;
    let mut rows = stmt
        .query([asset_id])
        .map_err(|err| DbError::sqlite("查询 asset_ref", err))?;
    let mut refs = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 asset_ref 行", err))?
    {
        refs.push(decode_asset_ref(row)?);
    }
    Ok(refs)
}

fn decode_asset_ref(row: &Row<'_>) -> Result<AssetRef> {
    let raw_kind: String = row
        .get(6)
        .map_err(|err| DbError::sqlite("读取 asset_ref.kind", err))?;
    let kind = AssetRefKind::parse(&raw_kind).ok_or_else(|| {
        DbError::sqlite(
            format!("解析 asset_ref.kind（值 {raw_kind}）"),
            rusqlite::Error::InvalidQuery,
        )
    })?;
    Ok(AssetRef {
        from_asset_id: row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 asset_ref.from_asset_id", err))?,
        from_path: row
            .get(1)
            .map_err(|err| DbError::sqlite("读取引用来源 path", err))?,
        from_package_name: row
            .get(2)
            .map_err(|err| DbError::sqlite("读取引用来源 package_name", err))?,
        to_package: row
            .get(3)
            .map_err(|err| DbError::sqlite("读取 asset_ref.to_package", err))?,
        to_asset_id: row
            .get(4)
            .map_err(|err| DbError::sqlite("读取 asset_ref.to_asset_id", err))?,
        to_path: row
            .get(5)
            .map_err(|err| DbError::sqlite("读取引用目标 path", err))?,
        kind,
    })
}
