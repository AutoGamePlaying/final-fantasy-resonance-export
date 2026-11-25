//! Levels area (`08_levels`) import stage.
//!
//! Imports `export-maps` level JSON files (`<level>.json`) and `_graph.json` into:
//! - `level` and `level_map`
//! - `actor` and `actor_rtree` (3D R*Tree spatial index using shape bounds when present)
//! - `level_object` (all 10 classified summary categories)
//! - `transition_point` (deduplicated `(map_id, point_id)` landing coordinates)
//! - `map_edge` (directed graph edges from `_graph.json`)
//! - `map_shop` (map-to-shop mapping with `VariousShop` expanded into single shops)
//!
//! Condition expressions are validated without mutating raw strings; syntax errors and dangling
//! cross-area references (when `04_world` tables are populated) are recorded in `build_issue`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use ffr_data::assets::level::{
    EventEntry, LevelExportDocument, LevelObject, LevelObjectData, LevelObjectKind, LevelSummary,
    MapEdgeKind, Vec3, load_level_document, load_map_graph, validate_condition_expr,
};
use ffr_data::world::{VariousShop, load_various_shops};
use ffr_data::{DataError, DataStore};
use rusqlite::{Connection, params};

use crate::build::{
    BuildInputs, ImportCtx, Stage, StageStats, StageStatus, record_logical_source_on_conn,
};
use crate::error::{DbError, Result};
use crate::hash::hash_manifest_file;
use crate::record::encode_doc;

/// World-area reference ID sets loaded from `04_world` tables if present and populated.
struct WorldRefs {
    maps: Option<BTreeSet<i64>>,
    shops: Option<BTreeSet<i64>>,
    various_shops: Option<BTreeSet<i64>>,
    items: Option<BTreeSet<i64>>,
}

impl WorldRefs {
    fn load(conn: &Connection) -> Result<Self> {
        Ok(Self {
            maps: load_id_set(conn, "map", &["id"])?,
            shops: load_id_set(conn, "shop", &["id", "shop_id"])?,
            various_shops: load_id_set(conn, "various_shop", &["id"])?,
            items: load_id_set(conn, "item", &["id"])?,
        })
    }

    fn skipped_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.maps.is_none() {
            names.push("map");
        }
        if self.shops.is_none() {
            names.push("shop");
        }
        if self.various_shops.is_none() {
            names.push("various_shop");
        }
        if self.items.is_none() {
            names.push("item");
        }
        names
    }
}

struct TransitionPointCandidate {
    level_id: i64,
    level_name: String,
    actor_id: Option<i64>,
    actor_name: String,
    direction: Option<String>,
    point_name: Option<String>,
    location: Option<Vec3>,
}

/// Import exported level files and `_graph.json` into the `08_levels` tables.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let (stats, raw_manifest_sha256) = import_on_conn(ctx.tx(), ctx.inputs(), ctx.data_store())?;
    if stats.status == StageStatus::Ok {
        ctx.record_raw_source_hash("export-full/maps", raw_manifest_sha256);
    }
    Ok(stats)
}

fn import_on_conn(
    conn: &Connection,
    inputs: &BuildInputs,
    data_store: Option<&DataStore>,
) -> Result<(StageStats, Option<String>)> {
    let Some(maps_dir) = inputs.maps.as_deref() else {
        return Ok((StageStats::skipped("未指定关卡目录（--maps）"), None));
    };

    let entries =
        fs::read_dir(maps_dir).map_err(|err| DbError::io("读取关卡目录", maps_dir, err))?;
    let mut level_files: Vec<PathBuf> = Vec::new();
    let mut graph_path: Option<PathBuf> = None;

    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("遍历关卡目录", maps_dir, err))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !file_name.ends_with(".json") {
            continue;
        }
        if file_name == "_graph.json" {
            graph_path = Some(path);
        } else if file_name == "manifest.json" {
            continue;
        } else {
            level_files.push(path);
        }
    }

    level_files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    if level_files.is_empty() && graph_path.is_none() {
        return Ok((
            StageStats::skipped("关卡目录为空（未找到关卡 JSON 与 _graph.json）"),
            None,
        ));
    }

    let Some(graph_path) = graph_path else {
        return Err(DbError::StageFailed {
            stage: "levels",
            detail: format!("关卡目录缺少 _graph.json：{}", maps_dir.display()),
        });
    };

    let raw_manifest_sha256 =
        record_dir_source_on_conn(conn, maps_dir, Some(level_files.len() as u64))?;

    let world_refs = WorldRefs::load(conn)?;
    let (various_shop_expansions, used_export_various_shops) =
        load_various_shop_expansions(conn, data_store)?;

    let mut level_id_by_unique_name: BTreeMap<String, i64> = BTreeMap::new();
    let mut level_id_by_raw_name: BTreeMap<String, i64> = BTreeMap::new();
    let mut level_id_by_map_and_raw_name: BTreeMap<(i64, String), i64> = BTreeMap::new();
    let mut actor_id_by_level_and_name: BTreeMap<(i64, String), i64> = BTreeMap::new();
    let mut tp_candidates: BTreeMap<(i64, i64), Vec<TransitionPointCandidate>> = BTreeMap::new();
    let mut total_rows: u64 = 0;

    for file_path in &level_files {
        let doc = load_level_document(file_path)?;
        let rows = import_single_level(
            conn,
            &doc,
            &world_refs,
            &various_shop_expansions,
            &mut level_id_by_unique_name,
            &mut level_id_by_raw_name,
            &mut level_id_by_map_and_raw_name,
            &mut actor_id_by_level_and_name,
            &mut tp_candidates,
        )?;
        total_rows = total_rows.saturating_add(rows);
    }

    // Insert deduplicated transition_point rows and remember all valid (map_id, point_id) keys.
    let mut known_tp: BTreeSet<(i64, i64)> = BTreeSet::new();
    let mut maps_with_tp: BTreeSet<i64> = BTreeSet::new();
    {
        let mut tp_stmt = conn
            .prepare_cached(
                "INSERT INTO transition_point \
                 (map_id, point_id, level_id, level_name, actor_id, actor_name, direction, point_name, x, y, z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .map_err(|err| DbError::sqlite("准备写入 transition_point", err))?;

        for ((map_id, point_id), mut candidates) in tp_candidates {
            candidates.sort_by(|a, b| {
                b.point_name
                    .is_some()
                    .cmp(&a.point_name.is_some())
                    .then_with(|| b.location.is_some().cmp(&a.location.is_some()))
                    .then_with(|| a.level_name.cmp(&b.level_name))
                    .then_with(|| a.actor_name.cmp(&b.actor_name))
            });
            let chosen = &candidates[0];
            let (x, y, z) = split_vec3(chosen.location);
            tp_stmt
                .execute(params![
                    map_id,
                    point_id,
                    chosen.level_id,
                    chosen.level_name,
                    chosen.actor_id,
                    chosen.actor_name,
                    chosen.direction,
                    chosen.point_name,
                    x,
                    y,
                    z,
                ])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 transition_point ({map_id}, {point_id})"), err)
                })?;
            known_tp.insert((map_id, point_id));
            maps_with_tp.insert(map_id);
            total_rows = total_rows.saturating_add(1);

            for dup in &candidates[1..] {
                record_issue_on_conn(
                    conn,
                    "duplicate_transition_point",
                    &format!("map:{map_id}:point:{point_id}"),
                    &format!(
                        "重复的落点（保留 {}:{}，跳过 {}:{}）",
                        chosen.level_name, chosen.actor_name, dup.level_name, dup.actor_name
                    ),
                )?;
            }
        }
    }

    // Import map_edge rows from `_graph.json`.
    let graph = load_map_graph(&graph_path)?;
    {
        let mut edge_stmt = conn
            .prepare_cached(
                "INSERT INTO map_edge \
                 (from_map, level_id, level_name, actor_id, actor_name, unique_id, to_map, to_point, \
                  kind, is_auto, is_event_only, condition, x, y, z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )
            .map_err(|err| DbError::sqlite("准备写入 map_edge", err))?;

        let mut sorted_exits = graph.all_exits();
        sorted_exits.sort_by(|a, b| {
            a.from_map_id
                .cmp(&b.from_map_id)
                .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
                .then_with(|| a.source_level.cmp(&b.source_level))
                .then_with(|| a.actor_name.cmp(&b.actor_name))
                .then_with(|| a.unique_id.cmp(&b.unique_id))
                .then_with(|| a.target_map_id.cmp(&b.target_map_id))
                .then_with(|| a.target_point_id.cmp(&b.target_point_id))
                .then_with(|| a.condition.cmp(&b.condition))
        });

        for exit in sorted_exits {
            let level_id = level_id_by_map_and_raw_name
                .get(&(exit.from_map_id, exit.source_level.clone()))
                .or_else(|| level_id_by_unique_name.get(&exit.source_level))
                .or_else(|| level_id_by_raw_name.get(&exit.source_level))
                .copied();
            let actor_id = level_id.and_then(|lid| {
                actor_id_by_level_and_name
                    .get(&(lid, exit.actor_name.clone()))
                    .copied()
            });

            if level_id.is_none() {
                // Validate edge condition if its source level was not imported from JSON.
                check_condition(
                    conn,
                    &format!(
                        "edge:{}:{}:{}",
                        exit.from_map_id, exit.source_level, exit.actor_name
                    ),
                    "condition",
                    exit.condition.as_deref(),
                )?;
            }

            // Same-map transitions (`m_IsSameMapTrasition = true`) always land inside
            // `from_map_id`, even when UE's blueprint property `m_MapId` carries a stale ID from
            // a copied trigger.
            let effective_target_map = if exit.kind == MapEdgeKind::SameMap {
                Some(exit.from_map_id)
            } else {
                exit.target_map_id
            };

            if let Some(known_maps) = &world_refs.maps
                && let Some(target_map) = effective_target_map
                && !known_maps.contains(&target_map)
            {
                record_issue_on_conn(
                    conn,
                    "dangling_edge_target_map",
                    &format!("map:{}:{}", exit.from_map_id, exit.actor_name),
                    &format!("出口边目标地图编号 {target_map} 不存在于 map 表"),
                )?;
            }

            let mut effective_target_point = exit.target_point_id;
            if let (Some(target_map), Some(target_point)) =
                (effective_target_map, exit.target_point_id)
            {
                if exit.is_event_only && target_point < 0 {
                    // Event-only triggers with sentinel point -1 play a cutscene/event instead of
                    // teleporting to a landing point.
                } else if target_point < 0 {
                    if known_tp.contains(&(target_map, 0)) {
                        effective_target_point = Some(0);
                    } else {
                        let detail = if !maps_with_tp.contains(&target_map) {
                            format!(
                                "出口边目标落点 (map={target_map}, point={target_point}) 未找到：目标地图无任何落点（多为剧情专用图）且出口使用未设/默认编号 {target_point}"
                            )
                        } else {
                            format!(
                                "出口边目标落点 (map={target_map}, point={target_point}) 未找到：出口使用未设/默认编号 {target_point}，且目标地图缺少默认落点 0"
                            )
                        };
                        record_issue_on_conn(
                            conn,
                            "unresolved_transition_point",
                            &format!("map:{}:{}", exit.from_map_id, exit.actor_name),
                            &detail,
                        )?;
                    }
                } else if !known_tp.contains(&(target_map, target_point)) {
                    let detail = if known_tp.contains(&(exit.from_map_id, target_point)) {
                        format!(
                            "出口边目标落点 (map={target_map}, point={target_point}) 未找到：目标地图缺少该落点，同编号落点误放于来源地图 {}",
                            exit.from_map_id
                        )
                    } else if !maps_with_tp.contains(&target_map) {
                        format!(
                            "出口边目标落点 (map={target_map}, point={target_point}) 未找到：目标地图无任何落点记录"
                        )
                    } else {
                        format!(
                            "出口边目标落点 (map={target_map}, point={target_point}) 未找到：目标地图存在其他落点但缺少该编号（原版关卡数据缺漏）"
                        )
                    };
                    record_issue_on_conn(
                        conn,
                        "unresolved_transition_point",
                        &format!("map:{}:{}", exit.from_map_id, exit.actor_name),
                        &detail,
                    )?;
                }
            }

            let (x, y, z) = split_vec3(exit.location);
            edge_stmt
                .execute(params![
                    exit.from_map_id,
                    level_id,
                    exit.source_level,
                    actor_id,
                    exit.actor_name,
                    exit.unique_id,
                    effective_target_map,
                    effective_target_point,
                    exit.kind.as_str(),
                    i64::from(exit.is_auto_transition),
                    i64::from(exit.is_event_only),
                    exit.condition,
                    x,
                    y,
                    z,
                ])
                .map_err(|err| {
                    DbError::sqlite(
                        format!(
                            "写入 map_edge ({} -> {:?})",
                            exit.from_map_id, effective_target_map
                        ),
                        err,
                    )
                })?;
            total_rows = total_rows.saturating_add(1);
        }
    }

    let skipped = world_refs.skipped_names();
    let mut notes = Vec::new();
    if !skipped.is_empty() {
        notes.push(format!(
            "已跳过跨区域校验（世界表为空或未建：{}）",
            skipped.join("、")
        ));
    }
    if used_export_various_shops {
        notes.push("综合商店展开回退读取了 DT_VariousShopsList".to_owned());
    }

    let stats = if notes.is_empty() {
        StageStats::ok(total_rows)
    } else {
        StageStats::ok_with_note(total_rows, notes.join("；"))
    };
    Ok((stats, raw_manifest_sha256))
}

#[allow(clippy::too_many_arguments)]
fn import_single_level(
    conn: &Connection,
    doc: &LevelExportDocument,
    world_refs: &WorldRefs,
    various_shop_expansions: &BTreeMap<i64, Vec<i64>>,
    level_id_by_unique_name: &mut BTreeMap<String, i64>,
    level_id_by_raw_name: &mut BTreeMap<String, i64>,
    level_id_by_map_and_raw_name: &mut BTreeMap<(i64, String), i64>,
    actor_id_by_level_and_name: &mut BTreeMap<(i64, String), i64>,
    tp_candidates: &mut BTreeMap<(i64, i64), Vec<TransitionPointCandidate>>,
) -> Result<u64> {
    let lv = &doc.level;
    let unique_name = lv.unique_name().to_owned();
    if level_id_by_unique_name.contains_key(&unique_name) {
        record_issue_on_conn(
            conn,
            "duplicate_level_name",
            &unique_name,
            &format!("重复的关卡唯一名，已跳过：{}", lv.path),
        )?;
        return Ok(0);
    }
    if unique_name != lv.name {
        record_issue_on_conn(
            conn,
            "duplicate_level_name",
            &lv.name,
            &format!(
                "跨目录重名子关卡已按输出文件消歧为 {unique_name}（{}）",
                lv.path
            ),
        )?;
    }

    let level_doc_json = encode_doc("level", &unique_name, lv)?;
    let level_id = {
        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO level \
                 (name, raw_name, path, package_path, sublevel_type, persistent_level, map_id, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|err| DbError::sqlite("准备写入 level", err))?;
        stmt.execute(params![
            unique_name,
            lv.name,
            lv.path,
            lv.package_path,
            lv.sublevel_type,
            lv.persistent_level,
            lv.map_id,
            level_doc_json,
        ])
        .map_err(|err| DbError::sqlite(format!("写入 level {unique_name}"), err))?;
        conn.last_insert_rowid()
    };

    let mut inserted_rows: u64 = 1;
    level_id_by_unique_name.insert(unique_name.clone(), level_id);
    level_id_by_raw_name
        .entry(lv.name.clone())
        .or_insert(level_id);

    let mut effective_map_ids = lv.effective_map_ids();
    effective_map_ids.sort_unstable();
    effective_map_ids.dedup();
    {
        let mut lm_stmt = conn
            .prepare_cached("INSERT OR IGNORE INTO level_map (level_id, map_id) VALUES (?1, ?2)")
            .map_err(|err| DbError::sqlite("准备写入 level_map", err))?;
        for &mid in &effective_map_ids {
            lm_stmt.execute(params![level_id, mid]).map_err(|err| {
                DbError::sqlite(format!("写入 level_map ({level_id}, {mid})"), err)
            })?;
            inserted_rows = inserted_rows.saturating_add(1);
            level_id_by_map_and_raw_name
                .entry((mid, lv.name.clone()))
                .or_insert(level_id);

            if let Some(known_maps) = &world_refs.maps
                && !known_maps.contains(&mid)
            {
                record_issue_on_conn(
                    conn,
                    "dangling_level_map",
                    &unique_name,
                    &format!("关卡关联的地图编号 {mid} 不存在于 map 表"),
                )?;
            }
        }
    }

    // Deduplicate actors in file order (first wins), then insert sorted by natural key `name`.
    let mut unique_actors = BTreeMap::new();
    for actor in &doc.actors {
        if unique_actors.contains_key(actor.name.as_str()) {
            record_issue_on_conn(
                conn,
                "duplicate_actor_name",
                &format!("{unique_name}:{}", actor.name),
                "同一关卡内出现重复 Actor 名称，已跳过重复项",
            )?;
            continue;
        }
        unique_actors.insert(actor.name.as_str(), actor);
    }

    // Insert actors and R*Tree entries.
    let mut local_actor_ids: BTreeMap<String, i64> = BTreeMap::new();
    {
        let mut actor_stmt = conn
            .prepare_cached(
                "INSERT INTO actor \
                 (level_id, name, label, class, is_game_class, x, y, z, pitch, yaw, roll, \
                  scale_x, scale_y, scale_z, props, asset_refs, subobjects) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )
            .map_err(|err| DbError::sqlite("准备写入 actor", err))?;
        let mut rtree_stmt = conn
            .prepare_cached(
                "INSERT INTO actor_rtree (id, min_x, max_x, min_y, max_y, min_z, max_z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|err| DbError::sqlite("准备写入 actor_rtree", err))?;

        for actor in unique_actors.into_values() {
            let (x, y, z) = split_vec3(actor.location);
            let (pitch, yaw, roll) = match actor.rotation {
                Some(r) => (Some(r.pitch), Some(r.yaw), Some(r.roll)),
                None => (None, None, None),
            };
            let (sx, sy, sz) = split_vec3(actor.scale);
            let props_json = actor
                .properties
                .as_ref()
                .map(|p| encode_doc("actor.props", &actor.name, p))
                .transpose()?;
            let asset_refs_json = actor
                .asset_refs
                .as_ref()
                .map(|r| encode_doc("actor.asset_refs", &actor.name, r))
                .transpose()?;
            let subobjects_json = actor
                .subobjects
                .as_ref()
                .map(|s| encode_doc("actor.subobjects", &actor.name, s))
                .transpose()?;

            actor_stmt
                .execute(params![
                    level_id,
                    actor.name,
                    actor.label,
                    actor.class,
                    i64::from(actor.is_game_class),
                    x,
                    y,
                    z,
                    pitch,
                    yaw,
                    roll,
                    sx,
                    sy,
                    sz,
                    props_json,
                    asset_refs_json,
                    subobjects_json,
                ])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 actor {unique_name}:{}", actor.name), err)
                })?;
            let actor_id = conn.last_insert_rowid();
            inserted_rows = inserted_rows.saturating_add(1);

            local_actor_ids.insert(actor.name.clone(), actor_id);
            actor_id_by_level_and_name.insert((level_id, actor.name.clone()), actor_id);

            if actor.location.is_some() {
                if let Some((min, max)) = actor.world_aabb() {
                    rtree_stmt
                        .execute(params![actor_id, min.x, max.x, min.y, max.y, min.z, max.z])
                        .map_err(|err| {
                            DbError::sqlite(
                                format!("写入 actor_rtree {unique_name}:{}", actor.name),
                                err,
                            )
                        })?;
                    inserted_rows = inserted_rows.saturating_add(1);
                } else {
                    record_issue_on_conn(
                        conn,
                        "invalid_actor_coords",
                        &format!("{unique_name}:{}", actor.name),
                        "Actor 坐标或包围盒含非有限浮点值，未写入 actor_rtree",
                    )?;
                }
            }
        }
    }

    // Validate summary conditions and cross-area references once per level object.
    validate_level_summary(conn, &unique_name, &doc.summary, world_refs)?;

    // Insert all 10 classified summary categories into `level_object` (and `map_shop` / `tp_candidates`).
    let map_scopes: Vec<Option<i64>> = if effective_map_ids.is_empty() {
        vec![None]
    } else {
        effective_map_ids.iter().copied().map(Some).collect()
    };

    let mut obj_stmt = conn
        .prepare_cached(
            "INSERT INTO level_object \
             (kind, map_id, level_id, actor_id, unique_id, ref_id, visible_condition, x, y, z, doc) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(|err| DbError::sqlite("准备写入 level_object", err))?;

    let mut map_shop_stmt = conn
        .prepare_cached(
            "INSERT INTO map_shop \
             (map_id, shop_id, source_shop_id, is_various_shop, level_object_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|err| DbError::sqlite("准备写入 map_shop", err))?;

    let s = &doc.summary;
    let mut sorted_map_units: Vec<_> = s.map_units.iter().collect();
    sorted_map_units.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_treasure_boxes: Vec<_> = s.treasure_boxes.iter().collect();
    sorted_treasure_boxes.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_event_triggers: Vec<_> = s.event_triggers.iter().collect();
    sorted_event_triggers.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_transitions: Vec<_> = s.transitions.iter().collect();
    sorted_transitions.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_transition_points: Vec<_> = s.transition_points.iter().collect();
    sorted_transition_points.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.point_id.cmp(&b.point_id))
    });
    let mut sorted_save_points: Vec<_> = s.save_points.iter().collect();
    sorted_save_points.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_shops: Vec<_> = s.shops.iter().collect();
    sorted_shops.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_inns: Vec<_> = s.inns.iter().collect();
    sorted_inns.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_gimmicks: Vec<_> = s.gimmicks.iter().collect();
    sorted_gimmicks.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    let mut sorted_encounter_zones: Vec<_> = s.encounter_zones.iter().collect();
    sorted_encounter_zones.sort_by(|a, b| {
        a.actor_name
            .cmp(&b.actor_name)
            .then_with(|| a.id.cmp(&b.id))
    });

    for &map_id_opt in &map_scopes {
        // 1. map_unit
        for &u in &sorted_map_units {
            let ref_id = u.map_unit_id.map(|id| id.to_string());
            let obj = LevelObject {
                kind: LevelObjectKind::MapUnit,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: u.actor_name.clone(),
                unique_id: u.unique_id,
                ref_id,
                visible_condition: u.visible_condition.clone(),
                location: u.location,
                data: LevelObjectData::MapUnit(u.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&u.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }

        // 2. treasure_box
        for &tb in &sorted_treasure_boxes {
            let ref_id = tb.item_id.map(|id| id.to_string());
            let obj = LevelObject {
                kind: LevelObjectKind::TreasureBox,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: tb.actor_name.clone(),
                unique_id: tb.unique_id,
                ref_id,
                visible_condition: tb.visible_condition.clone(),
                location: tb.location,
                data: LevelObjectData::TreasureBox(tb.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&tb.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }

        // 3. event_trigger
        for &et in &sorted_event_triggers {
            let ref_id = et
                .interact_event_id
                .clone()
                .or_else(|| et.events.first().map(|e| e.event_id.clone()));
            let obj = LevelObject {
                kind: LevelObjectKind::EventTrigger,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: et.actor_name.clone(),
                unique_id: et.unique_id,
                ref_id,
                visible_condition: et.visible_condition.clone(),
                location: et.location,
                data: LevelObjectData::EventTrigger(et.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&et.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }

        // 4. transition
        for &tr in &sorted_transitions {
            let ref_id = tr
                .targets
                .first()
                .and_then(|t| t.target_map_id)
                .map(|id| id.to_string());
            let obj = LevelObject {
                kind: LevelObjectKind::Transition,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: tr.actor_name.clone(),
                unique_id: tr.unique_id,
                ref_id,
                visible_condition: tr.condition.clone(),
                location: tr.location,
                data: LevelObjectData::Transition(tr.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&tr.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }

        // 5. transition_point
        for &tp in &sorted_transition_points {
            let actor_id = local_actor_ids.get(&tp.actor_name).copied();
            let obj = LevelObject {
                kind: LevelObjectKind::TransitionPoint,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: tp.actor_name.clone(),
                unique_id: None,
                ref_id: Some(tp.point_id.to_string()),
                visible_condition: None,
                location: tp.location,
                data: LevelObjectData::TransitionPoint(tp.clone()),
            };
            insert_level_object(conn, &mut obj_stmt, level_id, actor_id, &obj)?;
            inserted_rows = inserted_rows.saturating_add(1);

            if let Some(mid) = map_id_opt {
                tp_candidates.entry((mid, tp.point_id)).or_default().push(
                    TransitionPointCandidate {
                        level_id,
                        level_name: unique_name.clone(),
                        actor_id,
                        actor_name: tp.actor_name.clone(),
                        direction: tp.direction.clone(),
                        point_name: tp.point_name.clone(),
                        location: tp.location,
                    },
                );
            }
        }

        // 6. save_point
        for &sp in &sorted_save_points {
            let ref_id = sp.target_map_id.map(|id| id.to_string());
            let obj = LevelObject {
                kind: LevelObjectKind::SavePoint,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: sp.actor_name.clone(),
                unique_id: sp.unique_id,
                ref_id,
                visible_condition: sp.visible_condition.clone(),
                location: sp.location,
                data: LevelObjectData::SavePoint(sp.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&sp.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }

        // 7. shop (+ expanded map_shop rows)
        for &sh in &sorted_shops {
            let obj = LevelObject {
                kind: LevelObjectKind::Shop,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: sh.actor_name.clone(),
                unique_id: sh.unique_id,
                ref_id: Some(sh.shop_id.to_string()),
                visible_condition: sh.visible_condition.clone(),
                location: sh.location,
                data: LevelObjectData::Shop(sh.clone()),
            };
            let lo_id = insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&sh.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);

            if let Some(mid) = map_id_opt {
                let expanded_ids: Vec<i64> = if sh.is_various_shop {
                    match various_shop_expansions.get(&sh.shop_id) {
                        Some(ids) if !ids.is_empty() => ids.clone(),
                        _ => vec![sh.shop_id],
                    }
                } else {
                    vec![sh.shop_id]
                };

                for single_id in expanded_ids {
                    map_shop_stmt
                        .execute(params![
                            mid,
                            single_id,
                            sh.shop_id,
                            i64::from(sh.is_various_shop),
                            lo_id,
                        ])
                        .map_err(|err| {
                            DbError::sqlite(format!("写入 map_shop ({mid}, {single_id})"), err)
                        })?;
                    inserted_rows = inserted_rows.saturating_add(1);
                }
            }
        }

        // 8. inn
        for &inn in &sorted_inns {
            let obj = LevelObject {
                kind: LevelObjectKind::Inn,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: inn.actor_name.clone(),
                unique_id: inn.unique_id,
                ref_id: Some(inn.inn_id.to_string()),
                visible_condition: inn.visible_condition.clone(),
                location: inn.location,
                data: LevelObjectData::Inn(inn.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&inn.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }

        // 9. gimmick
        for &g in &sorted_gimmicks {
            let ref_id = g.events.first().map(|e| e.event_id.clone());
            let obj = LevelObject {
                kind: LevelObjectKind::Gimmick,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: g.actor_name.clone(),
                unique_id: g.unique_id,
                ref_id,
                visible_condition: g.visible_condition.clone(),
                location: g.location,
                data: LevelObjectData::Gimmick(g.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&g.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }

        // 10. encounter_zone
        for &ez in &sorted_encounter_zones {
            let ref_id = ez.encount_datatable.clone().or_else(|| {
                ez.terrain_datatables
                    .iter()
                    .find_map(|t| t.datatable.clone())
            });
            let obj = LevelObject {
                kind: LevelObjectKind::EncounterZone,
                map_id: map_id_opt,
                level_name: unique_name.clone(),
                actor_name: ez.actor_name.clone(),
                unique_id: ez.id,
                ref_id,
                visible_condition: ez.visible_condition.clone(),
                location: ez.location,
                data: LevelObjectData::EncounterZone(ez.clone()),
            };
            insert_level_object(
                conn,
                &mut obj_stmt,
                level_id,
                local_actor_ids.get(&ez.actor_name).copied(),
                &obj,
            )?;
            inserted_rows = inserted_rows.saturating_add(1);
        }
    }

    Ok(inserted_rows)
}

fn insert_level_object(
    conn: &Connection,
    stmt: &mut rusqlite::CachedStatement<'_>,
    level_id: i64,
    actor_id: Option<i64>,
    obj: &LevelObject,
) -> Result<i64> {
    let key = format!("{}:{}", obj.level_name, obj.actor_name);
    let doc_json = encode_doc("level_object", &key, obj)?;
    let (x, y, z) = split_vec3(obj.location);
    stmt.execute(params![
        obj.kind.as_str(),
        obj.map_id,
        level_id,
        actor_id,
        obj.unique_id,
        obj.ref_id,
        obj.visible_condition,
        x,
        y,
        z,
        doc_json,
    ])
    .map_err(|err| DbError::sqlite(format!("写入 level_object {key}"), err))?;
    Ok(conn.last_insert_rowid())
}

fn validate_level_summary(
    conn: &Connection,
    level_name: &str,
    summary: &LevelSummary,
    world_refs: &WorldRefs,
) -> Result<()> {
    for u in &summary.map_units {
        let subj = format!("{level_name}:{}", u.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            u.visible_condition.as_deref(),
        )?;
        check_event_conditions(conn, &subj, &u.events)?;
    }

    for tb in &summary.treasure_boxes {
        let subj = format!("{level_name}:{}", tb.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            tb.visible_condition.as_deref(),
        )?;
        if let (Some(items), Some(item_id)) = (&world_refs.items, tb.item_id)
            && !items.contains(&item_id)
        {
            record_issue_on_conn(
                conn,
                "dangling_item",
                &subj,
                &format!("宝箱道具编号 {item_id} 不存在于 item 表"),
            )?;
        }
    }

    for et in &summary.event_triggers {
        let subj = format!("{level_name}:{}", et.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            et.visible_condition.as_deref(),
        )?;
        check_condition(conn, &subj, "flagCondition", et.flag_condition.as_deref())?;
        check_event_conditions(conn, &subj, &et.events)?;
    }

    for tr in &summary.transitions {
        let subj = format!("{level_name}:{}", tr.actor_name);
        check_condition(conn, &subj, "condition", tr.condition.as_deref())?;
        for (idx, tgt) in tr.targets.iter().enumerate() {
            let field = format!("targets[{idx}].condition");
            check_condition(conn, &subj, &field, tgt.condition.as_deref())?;
            check_event_conditions(conn, &subj, &tgt.events)?;
            check_event_conditions(conn, &subj, &tgt.after_transition_events)?;
        }
    }

    for sp in &summary.save_points {
        let subj = format!("{level_name}:{}", sp.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            sp.visible_condition.as_deref(),
        )?;
    }

    for sh in &summary.shops {
        let subj = format!("{level_name}:{}", sh.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            sh.visible_condition.as_deref(),
        )?;
        if sh.is_various_shop {
            if let Some(vshops) = &world_refs.various_shops
                && !vshops.contains(&sh.shop_id)
            {
                record_issue_on_conn(
                    conn,
                    "dangling_various_shop",
                    &subj,
                    &format!("综合商店编号 {} 不存在于 various_shop 表", sh.shop_id),
                )?;
            }
        } else if let Some(shops) = &world_refs.shops
            && !shops.contains(&sh.shop_id)
        {
            record_issue_on_conn(
                conn,
                "dangling_shop",
                &subj,
                &format!("商店编号 {} 不存在于 shop 表", sh.shop_id),
            )?;
        }
    }

    for inn in &summary.inns {
        let subj = format!("{level_name}:{}", inn.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            inn.visible_condition.as_deref(),
        )?;
    }

    for g in &summary.gimmicks {
        let subj = format!("{level_name}:{}", g.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            g.visible_condition.as_deref(),
        )?;
        check_condition(conn, &subj, "flagCondition", g.flag_condition.as_deref())?;
        check_event_conditions(conn, &subj, &g.events)?;
    }

    for ez in &summary.encounter_zones {
        let subj = format!("{level_name}:{}", ez.actor_name);
        check_condition(
            conn,
            &subj,
            "visibleCondition",
            ez.visible_condition.as_deref(),
        )?;
        check_condition(conn, &subj, "flagCondition", ez.flag_condition.as_deref())?;
    }

    Ok(())
}

fn check_event_conditions(conn: &Connection, subject: &str, events: &[EventEntry]) -> Result<()> {
    for (idx, ev) in events.iter().enumerate() {
        let field = format!("events[{idx}].condition");
        check_condition(conn, subject, &field, ev.condition.as_deref())?;
    }
    Ok(())
}

fn check_condition(conn: &Connection, subject: &str, field: &str, raw: Option<&str>) -> Result<()> {
    let Some(cond) = raw else {
        return Ok(());
    };
    if let Err(reason) = validate_condition_expr(cond) {
        record_issue_on_conn(
            conn,
            "invalid_condition",
            subject,
            &format!("{field} 表达式解析失败（{cond:?}）：{reason}"),
        )?;
    }
    Ok(())
}

fn record_issue_on_conn(conn: &Connection, kind: &str, subject: &str, detail: &str) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(
            "INSERT INTO build_issue (stage, kind, subject, detail) VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|err| DbError::sqlite("准备写入 build_issue", err))?;
    stmt.execute(params![Stage::Levels.as_str(), kind, subject, detail])
        .map_err(|err| DbError::sqlite("写入 build_issue", err))?;
    Ok(())
}

fn record_dir_source_on_conn(
    conn: &Connection,
    dir: &Path,
    file_count: Option<u64>,
) -> Result<Option<String>> {
    let manifest_path = dir.join("manifest.json");
    let (canonical_sha, raw_sha) = if manifest_path.is_file() {
        let digest = hash_manifest_file(&manifest_path)?;
        (Some(digest.canonical_sha256), Some(digest.raw_sha256))
    } else {
        (None, None)
    };
    record_logical_source_on_conn(
        conn,
        Stage::Levels,
        "export-full/maps",
        canonical_sha.as_deref(),
        file_count,
    )?;
    Ok(raw_sha)
}

fn load_various_shop_expansions(
    conn: &Connection,
    data_store: Option<&DataStore>,
) -> Result<(BTreeMap<i64, Vec<i64>>, bool)> {
    // 1. Prefer `various_shop` table populated by stage `04_world`.
    if let Some(from_db) = load_various_shops_from_db(conn)?
        && !from_db.is_empty()
    {
        return Ok((from_db, false));
    }

    // 2. Fallback: load `Shop/DT_VariousShopsList.json` from `export` if available so
    // `--stages levels` still expands various shops cleanly.
    if let Some(store) = data_store {
        match load_various_shops(store.export_root()) {
            Ok(bundles) => {
                let map = bundles.into_iter().map(|b| (b.id, b.shop_ids())).collect();
                return Ok((map, true));
            }
            Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(DbError::from(err)),
        }
    }

    Ok((BTreeMap::new(), false))
}

fn load_various_shops_from_db(conn: &Connection) -> Result<Option<BTreeMap<i64, Vec<i64>>>> {
    let cols = table_columns(conn, "various_shop")?;
    if cols.is_empty() {
        return Ok(None);
    }

    let mut map = BTreeMap::new();
    if cols.iter().any(|c| c == "doc") {
        if !cols.iter().any(|c| c == "id") {
            return Ok(None);
        }
        let mut stmt = conn
            .prepare("SELECT id, doc FROM various_shop ORDER BY id")
            .map_err(|err| DbError::sqlite("准备读取 various_shop", err))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| DbError::sqlite("查询 various_shop", err))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| DbError::sqlite("迭代 various_shop", err))?
        {
            let id: i64 = row
                .get(0)
                .map_err(|err| DbError::sqlite("读取 various_shop.id", err))?;
            let raw_doc: String = row
                .get(1)
                .map_err(|err| DbError::sqlite("读取 various_shop.doc", err))?;
            if let Ok(bundle) = serde_json::from_str::<VariousShop>(&raw_doc) {
                map.insert(id, bundle.shop_ids());
            }
        }
        return Ok(Some(map));
    }

    let has_split = [
        "id",
        "tool_shop_id",
        "weapon_shop_id",
        "armor_shop_id",
        "accessory_shop_id",
    ]
    .iter()
    .all(|needed| cols.iter().any(|c| c == needed));
    if has_split {
        let mut stmt = conn
            .prepare(
                "SELECT id, tool_shop_id, weapon_shop_id, armor_shop_id, accessory_shop_id \
                 FROM various_shop ORDER BY id",
            )
            .map_err(|err| DbError::sqlite("准备读取 various_shop 分列", err))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| DbError::sqlite("查询 various_shop 分列", err))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| DbError::sqlite("迭代 various_shop 分列", err))?
        {
            let id: i64 = row
                .get(0)
                .map_err(|err| DbError::sqlite("读取 various_shop.id", err))?;
            let mut ids = Vec::new();
            for idx in 1..=4 {
                if let Some(sub_id) = row
                    .get::<_, Option<i64>>(idx)
                    .map_err(|err| DbError::sqlite("读取 various_shop 子店编号", err))?
                    && sub_id >= 0
                {
                    ids.push(sub_id);
                }
            }
            map.insert(id, ids);
        }
        return Ok(Some(map));
    }

    Ok(None)
}

fn load_id_set(
    conn: &Connection,
    table: &str,
    candidate_cols: &[&str],
) -> Result<Option<BTreeSet<i64>>> {
    let cols = table_columns(conn, table)?;
    if cols.is_empty() {
        return Ok(None);
    }
    let Some(id_col) = candidate_cols
        .iter()
        .find(|&&cand| cols.iter().any(|c| c == cand))
    else {
        return Ok(None);
    };

    let sql = format!("SELECT \"{id_col}\" FROM \"{table}\" WHERE \"{id_col}\" IS NOT NULL");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| DbError::sqlite(format!("准备读取跨区域校验表 {table}"), err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite(format!("查询跨区域校验表 {table}"), err))?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取表 {table} 编号列"), err))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite(format!("读取 {table}.{id_col}"), err))?;
        ids.insert(id);
    }
    if ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ids))
    }
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(|err| DbError::sqlite(format!("检查表 {table} 是否存在"), err))?;
    if !exists {
        return Ok(Vec::new());
    }

    let pragma = format!("PRAGMA table_info(\"{table}\")");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|err| DbError::sqlite(format!("读取表 {table} 列定义"), err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite(format!("执行 {pragma}"), err))?;
    let mut cols = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("迭代表 {table} 列定义"), err))?
    {
        let name: String = row
            .get(1)
            .map_err(|err| DbError::sqlite(format!("读取表 {table} 列名"), err))?;
        cols.push(name);
    }
    Ok(cols)
}

fn split_vec3(v: Option<Vec3>) -> (Option<f64>, Option<f64>, Option<f64>) {
    match v {
        Some(p) => (Some(p.x), Some(p.y), Some(p.z)),
        None => (None, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{SCHEMA_PARTS, apply_table_schemas};
    use crate::testing::fixture_area_dir;

    #[test]
    fn cross_area_checks_report_dangling_world_references_when_world_populated() {
        let fixture = fixture_area_dir("levels");
        let conn = Connection::open_in_memory().expect("open in-memory db");
        // Apply only 00_meta and 08_levels schemas so parallel edits to 04_world cannot conflict.
        for part in SCHEMA_PARTS {
            if part.name == "00_meta" || part.name == "08_levels" {
                conn.execute_batch(part.tables_sql)
                    .expect("apply meta/levels table schema");
            }
        }
        // Populate mock `04_world` tables with intentional omissions:
        // - map: has 100, 200, 400 (omits 300 -> dangling_level_map + dangling_edge_target_map)
        // - shop: has 2001, 4001 (omits 1001 -> dangling_shop)
        // - various_shop: has 10001 expanding to 2001, 4001 (valid!)
        // - item: has 999 (omits 501 -> dangling_item)
        conn.execute_batch(
            "CREATE TABLE map (id INTEGER PRIMARY KEY);
             CREATE TABLE shop (id INTEGER PRIMARY KEY);
             CREATE TABLE various_shop (
                 id INTEGER PRIMARY KEY,
                 tool_shop_id INTEGER,
                 weapon_shop_id INTEGER,
                 armor_shop_id INTEGER,
                 accessory_shop_id INTEGER
             );
             CREATE TABLE item (id INTEGER PRIMARY KEY);
             INSERT INTO map (id) VALUES (100), (200), (400);
             INSERT INTO shop (id) VALUES (2001), (4001);
             INSERT INTO various_shop VALUES (10001, NULL, 2001, 4001, NULL);
             INSERT INTO item (id) VALUES (999);",
        )
        .expect("create mock world tables");

        let inputs = BuildInputs {
            export: Some(fixture.join("export")),
            maps: Some(fixture.join("maps")),
            ..BuildInputs::default()
        };
        let store = DataStore::new(fixture.join("export"));

        let (stats, _) = import_on_conn(&conn, &inputs, Some(&store)).expect("run levels import");
        assert!(
            stats.note.is_empty(),
            "expected no skipped cross-area checks when world tables exist, got: {}",
            stats.note
        );
        assert!(stats.rows > 0);

        let mut stmt = conn
            .prepare("SELECT kind, subject, detail FROM build_issue WHERE stage = ?1")
            .expect("prepare issues query");
        let issues: Vec<(String, String, String)> = stmt
            .query_map([Stage::Levels.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("query issues")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect issues");

        assert!(
            issues
                .iter()
                .any(|(k, _, d)| k == "dangling_shop" && d.contains("1001")),
            "expected dangling_shop for 1001: {issues:?}"
        );
        assert!(
            !issues.iter().any(|(k, _, _)| k == "dangling_various_shop"),
            "10001 exists in various_shop: {issues:?}"
        );
        assert!(
            issues
                .iter()
                .any(|(k, _, d)| k == "dangling_item" && d.contains("501")),
            "expected dangling_item for 501: {issues:?}"
        );
        assert!(
            !issues.iter().any(|(k, _, _)| k == "dangling_playable_unit"),
            "levels stage must not validate map_unit_id against playable_unit: {issues:?}"
        );
        assert!(
            issues
                .iter()
                .any(|(k, _, d)| k == "dangling_edge_target_map" && d.contains("300")),
            "expected dangling_edge_target_map for 300: {issues:?}"
        );

        // Verify various_shop expansion came directly from the SQLite `various_shop` table.
        let mut shop_stmt = conn
            .prepare("SELECT shop_id FROM map_shop WHERE map_id = 100 ORDER BY shop_id")
            .expect("prepare map_shop query");
        let ids: Vec<i64> = shop_stmt
            .query_map([], |row| row.get(0))
            .expect("query map_shop")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect map_shop");
        assert_eq!(ids, vec![1001, 2001, 4001]);

        let conn2 = Connection::open_in_memory().expect("open second in-memory db");
        apply_table_schemas(&conn2).expect("apply all schemas");
    }
}
