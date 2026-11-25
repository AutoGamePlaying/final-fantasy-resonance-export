//! Levels area (`08_levels`) `impl GameDb` queries.
//!
//! All queries use `Connection::prepare_cached` and indexed lookups (verified via
//! `EXPLAIN QUERY PLAN` / `assert_db_uses_index` in `tests/levels.rs`).

use std::collections::{HashMap, HashSet, VecDeque};

use ffr_data::assets::level::{
    Actor, ActorSubobject, EncounterZone, Level, LevelObject, LevelObjectData, LevelObjectKind,
    MapEdgeKind, MapExit, Rot3, Vec3,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record::{decode_doc, query_doc_opt, query_docs};

/// Scope selector for spatial proximity queries ([`GameDb::actors_near`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorScope<'a> {
    /// Restrict results to one level by its unique name (`level.name`).
    Level(&'a str),
    /// Restrict results to all levels mapped to `map_id` (via `level_map`).
    Map(i64),
}

/// An [`Actor`] paired with its database row ID and owning level name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelActor {
    pub actor_id: i64,
    pub level_name: String,
    pub actor: Actor,
}

/// Spatial search result returned by [`GameDb::actors_near`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyActor {
    pub actor_id: i64,
    pub level_name: String,
    pub actor: Actor,
    /// Exact 3D Euclidean distance from the query center to the actor's AABB (`0.0` if inside).
    pub distance: f64,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

/// Expanded shop entry inside a map returned by [`GameDb::shops_in_map`].
///
/// Multi-shop counters (`is_various_shop == true`) are expanded into individual single-shop entries
/// carrying both the expanded single `shop_id` and the parent `various_shop_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapShopEntry {
    pub shop_id: i64,
    pub various_shop_id: Option<i64>,
    pub level_name: String,
    pub actor_name: String,
    pub unique_id: Option<i64>,
    pub map_unit_id: Option<i64>,
    pub interact_type: Option<String>,
    pub visible_condition: Option<String>,
    pub location: Option<Vec3>,
}

/// Spatial search result for a classified level object returned by [`GameDb::level_objects_near`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyLevelObject {
    pub object_id: i64,
    pub actor_id: i64,
    pub object: LevelObject,
    /// Euclidean distance (3D when query `z` is provided, 2D XY otherwise) from the query center
    /// to the object's actor AABB (`0.0` if inside).
    pub distance: f64,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

/// One NPC shop placement returned by [`GameDb::shop_locations`], joining `map_shop`,
/// `level_object`, `map`, and `map_unit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopLocationEntry {
    pub map_id: i64,
    pub map_name_zh: Option<String>,
    pub shop_id: i64,
    pub various_shop_id: Option<i64>,
    pub level_name: String,
    pub actor_name: String,
    pub unique_id: Option<i64>,
    pub map_unit_id: Option<i64>,
    pub npc_name: Option<String>,
    pub npc_name_zh: Option<String>,
    pub interact_type: Option<String>,
    pub visible_condition: Option<String>,
    pub location: Option<Vec3>,
}

/// Exposed SQL statements used by `08_levels` queries so integration tests can verify their exact
/// query plans with `assert_db_uses_index`.
pub mod sql {
    pub const LEVEL_BY_NAME: &str = "SELECT doc FROM level WHERE name = ?1";

    pub const LEVELS_OF_MAP: &str = "\
        SELECT l.name, l.doc \
        FROM level_map lm \
        JOIN level l ON l.id = lm.level_id \
        WHERE lm.map_id = ?1 \
        ORDER BY l.name";

    pub const MAP_OBJECTS_ALL: &str = "\
        SELECT id, doc \
        FROM level_object \
        WHERE map_id = ?1 \
        ORDER BY id";

    pub const MAP_OBJECTS_BY_KIND: &str = "\
        SELECT id, doc \
        FROM level_object \
        WHERE map_id = ?1 AND kind = ?2 \
        ORDER BY id";

    pub const MAP_OBJECT_BY_UNIQUE_ID: &str = "\
        SELECT id, doc \
        FROM level_object \
        WHERE map_id = ?1 AND unique_id = ?2 \
        ORDER BY id";

    pub const MAP_EXITS: &str = "\
        SELECT \
            e.from_map, e.level_name, e.actor_name, e.unique_id, \
            e.to_map, e.to_point, e.kind, e.is_auto, e.is_event_only, e.condition, \
            e.x, e.y, e.z, \
            tp.x, tp.y, tp.z, tp.direction \
        FROM map_edge e \
        LEFT JOIN transition_point tp \
            ON tp.map_id = e.to_map AND tp.point_id = e.to_point \
        WHERE e.from_map = ?1 \
        ORDER BY e.id";

    pub const MAP_OUTGOING_CROSS_EDGES: &str = "\
        SELECT \
            e.from_map, e.level_name, e.actor_name, e.unique_id, \
            e.to_map, e.to_point, e.kind, e.is_auto, e.is_event_only, e.condition, \
            e.x, e.y, e.z, \
            tp.x, tp.y, tp.z, tp.direction \
        FROM map_edge e \
        LEFT JOIN transition_point tp \
            ON tp.map_id = e.to_map AND tp.point_id = e.to_point \
        WHERE e.from_map = ?1 AND e.to_map IS NOT NULL AND e.to_map != e.from_map \
        ORDER BY (e.condition IS NOT NULL), e.is_event_only, e.id";

    pub const SHOPS_IN_MAP: &str = "\
        SELECT ms.shop_id, ms.source_shop_id, ms.is_various_shop, lo.id, lo.doc \
        FROM map_shop ms \
        JOIN level_object lo ON lo.id = ms.level_object_id \
        WHERE ms.map_id = ?1 \
        ORDER BY ms.shop_id, ms.id";

    pub const MAPS_WITH_SHOP: &str = "\
        SELECT map_id FROM map_shop WHERE shop_id = ?1 \
        UNION \
        SELECT map_id FROM map_shop WHERE source_shop_id = ?1 \
        ORDER BY map_id";

    pub const LEVEL_ID_BY_NAME: &str = "SELECT id FROM level WHERE name = ?1";

    pub const ACTORS_NEAR_LEVEL: &str = "\
        SELECT \
            a.id, l.name, a.name, a.label, a.class, a.is_game_class, \
            a.x, a.y, a.z, a.pitch, a.yaw, a.roll, \
            a.scale_x, a.scale_y, a.scale_z, a.props, a.asset_refs, a.subobjects, \
            r.min_x, r.max_x, r.min_y, r.max_y, r.min_z, r.max_z \
        FROM actor_rtree r \
        CROSS JOIN actor a ON a.id = r.id \
        CROSS JOIN level l ON l.id = a.level_id \
        WHERE r.max_x >= ?1 AND r.min_x <= ?2 \
          AND r.max_y >= ?3 AND r.min_y <= ?4 \
          AND r.max_z >= ?5 AND r.min_z <= ?6 \
          AND a.level_id = ?7";

    pub const ACTORS_NEAR_LEVEL_KIND: &str = "\
        SELECT \
            a.id, l.name, a.name, a.label, a.class, a.is_game_class, \
            a.x, a.y, a.z, a.pitch, a.yaw, a.roll, \
            a.scale_x, a.scale_y, a.scale_z, a.props, a.asset_refs, a.subobjects, \
            r.min_x, r.max_x, r.min_y, r.max_y, r.min_z, r.max_z \
        FROM actor_rtree r \
        CROSS JOIN actor a ON a.id = r.id \
        CROSS JOIN level l ON l.id = a.level_id \
        WHERE r.max_x >= ?1 AND r.min_x <= ?2 \
          AND r.max_y >= ?3 AND r.min_y <= ?4 \
          AND r.max_z >= ?5 AND r.min_z <= ?6 \
          AND a.level_id = ?7 \
          AND EXISTS ( \
              SELECT 1 FROM level_object lo \
              WHERE lo.actor_id = a.id AND lo.kind = ?8 \
          )";

    pub const ACTORS_NEAR_MAP: &str = "\
        SELECT \
            a.id, l.name, a.name, a.label, a.class, a.is_game_class, \
            a.x, a.y, a.z, a.pitch, a.yaw, a.roll, \
            a.scale_x, a.scale_y, a.scale_z, a.props, a.asset_refs, a.subobjects, \
            r.min_x, r.max_x, r.min_y, r.max_y, r.min_z, r.max_z \
        FROM actor_rtree r \
        CROSS JOIN actor a ON a.id = r.id \
        CROSS JOIN level_map lm ON lm.level_id = a.level_id AND lm.map_id = ?7 \
        CROSS JOIN level l ON l.id = a.level_id \
        WHERE r.max_x >= ?1 AND r.min_x <= ?2 \
          AND r.max_y >= ?3 AND r.min_y <= ?4 \
          AND r.max_z >= ?5 AND r.min_z <= ?6";

    pub const ACTORS_NEAR_MAP_KIND: &str = "\
        SELECT \
            a.id, l.name, a.name, a.label, a.class, a.is_game_class, \
            a.x, a.y, a.z, a.pitch, a.yaw, a.roll, \
            a.scale_x, a.scale_y, a.scale_z, a.props, a.asset_refs, a.subobjects, \
            r.min_x, r.max_x, r.min_y, r.max_y, r.min_z, r.max_z \
        FROM actor_rtree r \
        CROSS JOIN actor a ON a.id = r.id \
        CROSS JOIN level_map lm ON lm.level_id = a.level_id AND lm.map_id = ?7 \
        CROSS JOIN level l ON l.id = a.level_id \
        WHERE r.max_x >= ?1 AND r.min_x <= ?2 \
          AND r.max_y >= ?3 AND r.min_y <= ?4 \
          AND r.max_z >= ?5 AND r.min_z <= ?6 \
          AND EXISTS ( \
              SELECT 1 FROM level_object lo \
              WHERE lo.actor_id = a.id AND lo.kind = ?8 \
          )";

    pub const ACTORS_BY_CLASS_PREFIX: &str = "\
        SELECT \
            a.id, l.name, a.name, a.label, a.class, a.is_game_class, \
            a.x, a.y, a.z, a.pitch, a.yaw, a.roll, \
            a.scale_x, a.scale_y, a.scale_z, a.props, a.asset_refs, a.subobjects \
        FROM actor a \
        JOIN level l ON l.id = a.level_id \
        WHERE a.class >= ?1 AND a.class <= ?2 \
        ORDER BY a.class, a.id \
        LIMIT ?3";

    pub const ENCOUNTER_ZONES_BY_MAP: &str = "\
        SELECT id, doc \
        FROM level_object \
        WHERE map_id = ?1 AND kind = 'encounter_zone' \
        ORDER BY id";

    pub const LEVEL_OBJECTS_NEAR_MAP: &str = "\
        SELECT \
            lo.id, a.id, lo.doc, \
            r.min_x, r.max_x, r.min_y, r.max_y, r.min_z, r.max_z \
        FROM actor_rtree r \
        CROSS JOIN actor a ON a.id = r.id \
        CROSS JOIN level_map lm ON lm.level_id = a.level_id AND lm.map_id = ?7 \
        CROSS JOIN level_object lo ON lo.actor_id = a.id AND lo.map_id = ?7 \
        WHERE r.max_x >= ?1 AND r.min_x <= ?2 \
          AND r.max_y >= ?3 AND r.min_y <= ?4 \
          AND r.max_z >= ?5 AND r.min_z <= ?6";

    pub const SHOP_LOCATIONS_BY_SHOP_ID: &str = "\
        SELECT \
            ms.map_id, m.name_zh, ms.shop_id, ms.source_shop_id, ms.is_various_shop, \
            lo.id, lo.doc \
        FROM map_shop ms \
        JOIN level_object lo ON lo.id = ms.level_object_id \
        LEFT JOIN map m ON m.id = ms.map_id \
        WHERE ms.shop_id = ?1 \
        ORDER BY ms.map_id, ms.id";

    pub const MAP_UNIT_NAMES_BY_ID: &str = "SELECT name, name_zh FROM map_unit WHERE id = ?1";

    pub const TREASURE_BOX_OBJECTS_BY_ITEM: &str = "\
        SELECT id, doc \
        FROM level_object \
        WHERE kind = 'treasure_box' AND ref_id = ?1 \
        ORDER BY map_id, id";
}

impl GameDb {
    /// Look up one exported [`Level`] by its unique name (e.g. `"Dng_01Gra_43_GD"`).
    pub fn level(&self, name: &str) -> Result<Option<Level>> {
        self.with_conn(|conn| query_doc_opt(conn, "level", name, sql::LEVEL_BY_NAME, [name]))
    }

    /// Return all [`Level`]s (persistent and sublevels, including shared multi-map levels)
    /// belonging to `map_id`, sorted by level name.
    pub fn levels_of_map(&self, map_id: i64) -> Result<Vec<Level>> {
        self.with_conn(|conn| query_docs(conn, "level", sql::LEVELS_OF_MAP, [map_id]))
    }

    /// Return classified [`LevelObject`]s placed in `map_id`, optionally filtered by `kind`.
    pub fn map_objects(
        &self,
        map_id: i64,
        kind: Option<LevelObjectKind>,
    ) -> Result<Vec<LevelObject>> {
        self.with_conn(|conn| match kind {
            Some(k) => query_docs(
                conn,
                "level_object",
                sql::MAP_OBJECTS_BY_KIND,
                params![map_id, k.as_str()],
            ),
            None => query_docs(conn, "level_object", sql::MAP_OBJECTS_ALL, [map_id]),
        })
    }

    /// Look up classified [`LevelObject`]s matching `(map_id, unique_id)` (e.g. when the runtime
    /// bridge reports an interactive object's `unique_id`).
    ///
    /// Returns a `Vec` because a single actor (such as a shopkeeper NPC) or distinct object
    /// categories in the same map can share the same `unique_id`.
    pub fn map_object_by_unique_id(&self, map_id: i64, unique_id: i64) -> Result<Vec<LevelObject>> {
        self.with_conn(|conn| {
            query_docs(
                conn,
                "level_object",
                sql::MAP_OBJECT_BY_UNIQUE_ID,
                params![map_id, unique_id],
            )
        })
    }

    /// Return all outgoing [`MapExit`] edges from `map_id`, with each edge's target landing
    /// coordinate resolved via `transition_point` when available.
    pub fn map_exits(&self, map_id: i64) -> Result<Vec<MapExit>> {
        self.with_conn(|conn| query_map_exits_on_conn(conn, sql::MAP_EXITS, map_id))
    }

    /// Find a shortest-hop route (BFS) from `from_map` to `to_map`.
    ///
    /// Conditional edges are traversed normally and keep their `condition` field populated on the
    /// returned [`MapExit`] steps. Unconditional non-event-only edges are preferred first when
    /// ties occur at the same hop count. Returns `Ok(Some(vec![]))` when `from_map == to_map`, and
    /// `Ok(None)` when unreachable.
    pub fn map_route(&self, from_map: i64, to_map: i64) -> Result<Option<Vec<MapExit>>> {
        if from_map == to_map {
            return Ok(Some(Vec::new()));
        }

        self.with_conn(|conn| {
            let mut visited: HashSet<i64> = HashSet::new();
            let mut prev: HashMap<i64, (i64, MapExit)> = HashMap::new();
            let mut queue: VecDeque<i64> = VecDeque::new();

            visited.insert(from_map);
            queue.push_back(from_map);

            let mut found = false;
            while let Some(cur) = queue.pop_front() {
                let edges = query_map_exits_on_conn(conn, sql::MAP_OUTGOING_CROSS_EDGES, cur)?;
                for edge in edges {
                    let Some(next) = edge.target_map_id else {
                        continue;
                    };
                    if visited.insert(next) {
                        prev.insert(next, (cur, edge));
                        if next == to_map {
                            found = true;
                            break;
                        }
                        queue.push_back(next);
                    }
                }
                if found {
                    break;
                }
            }

            if !found {
                return Ok(None);
            }

            let mut path = Vec::new();
            let mut node = to_map;
            while node != from_map {
                let Some((parent, edge)) = prev.remove(&node) else {
                    return Ok(None);
                };
                path.push(edge);
                node = parent;
            }
            path.reverse();
            Ok(Some(path))
        })
    }

    /// Return all shops located in `map_id`, with multi-shop counters (`VariousShop`) expanded
    /// into individual single shops.
    pub fn shops_in_map(&self, map_id: i64) -> Result<Vec<MapShopEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::SHOPS_IN_MAP)
                .map_err(|err| DbError::sqlite("准备查询地图商店列表", err))?;
            let mut rows = stmt
                .query([map_id])
                .map_err(|err| DbError::sqlite(format!("查询地图 {map_id} 商店列表"), err))?;

            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取地图商店查询行", err))?
            {
                let shop_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 map_shop.shop_id", err))?;
                let source_shop_id: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 map_shop.source_shop_id", err))?;
                let is_various: i64 = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 map_shop.is_various_shop", err))?;
                let lo_id: i64 = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 level_object.id", err))?;
                let raw_doc: String = row
                    .get(4)
                    .map_err(|err| DbError::sqlite("读取 level_object.doc", err))?;

                let obj: LevelObject = decode_doc("level_object", lo_id, &raw_doc)?;
                let (map_unit_id, interact_type) = match &obj.data {
                    LevelObjectData::Shop(s) => (s.map_unit_id, s.interact_type.clone()),
                    _ => (None, None),
                };

                out.push(MapShopEntry {
                    shop_id,
                    various_shop_id: (is_various != 0).then_some(source_shop_id),
                    level_name: obj.level_name,
                    actor_name: obj.actor_name,
                    unique_id: obj.unique_id,
                    map_unit_id,
                    interact_type,
                    visible_condition: obj.visible_condition,
                    location: obj.location,
                });
            }
            Ok(out)
        })
    }

    /// Return all distinct map IDs containing `shop_id` (matching both single shops—including
    /// those exposed through a `VariousShop` counter—and `VariousShop` bundle IDs), sorted
    /// ascending.
    pub fn maps_with_shop(&self, shop_id: i64) -> Result<Vec<i64>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::MAPS_WITH_SHOP)
                .map_err(|err| DbError::sqlite("准备反查商店所在地图", err))?;
            let mut rows = stmt
                .query([shop_id])
                .map_err(|err| DbError::sqlite(format!("反查商店 {shop_id} 所在地图"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取商店地图反查结果", err))?
            {
                let map_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 map_shop.map_id", err))?;
                out.push(map_id);
            }
            Ok(out)
        })
    }

    /// Spatial proximity query powered by the `actor_rtree` 3D R*Tree index.
    ///
    /// Returns all actors inside `scope` whose 3D AABB lies within Euclidean distance `radius` of
    /// `center`, optionally restricted to actors associated with a classified `level_object` of
    /// `kind`, sorted by `(distance, actor_id)`.
    pub fn actors_near(
        &self,
        scope: ActorScope<'_>,
        center: Vec3,
        radius: f64,
        kind: Option<LevelObjectKind>,
    ) -> Result<Vec<NearbyActor>> {
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return Ok(Vec::new());
        }

        // Pad query box slightly to account for SQLite R*Tree's internal 32-bit float rounding;
        // exact f64 Euclidean distance filtering is applied below.
        let r_pad = radius + 1e-3;
        let q_min_x = center.x - r_pad;
        let q_max_x = center.x + r_pad;
        let q_min_y = center.y - r_pad;
        let q_max_y = center.y + r_pad;
        let q_min_z = center.z - r_pad;
        let q_max_z = center.z + r_pad;

        self.with_conn(|conn| {
            let mut out = Vec::new();
            match (scope, kind) {
                (ActorScope::Level(level_name), None) => {
                    let Some(level_id) = lookup_level_id(conn, level_name)? else {
                        return Ok(Vec::new());
                    };
                    let mut stmt = conn
                        .prepare_cached(sql::ACTORS_NEAR_LEVEL)
                        .map_err(|err| DbError::sqlite("准备关卡空间查询", err))?;
                    let mut rows = stmt
                        .query(params![
                            q_min_x, q_max_x, q_min_y, q_max_y, q_min_z, q_max_z, level_id
                        ])
                        .map_err(|err| DbError::sqlite("执行关卡空间查询", err))?;
                    collect_nearby_rows(&mut rows, center, radius, &mut out)?;
                }
                (ActorScope::Level(level_name), Some(k)) => {
                    let Some(level_id) = lookup_level_id(conn, level_name)? else {
                        return Ok(Vec::new());
                    };
                    let mut stmt = conn
                        .prepare_cached(sql::ACTORS_NEAR_LEVEL_KIND)
                        .map_err(|err| DbError::sqlite("准备关卡分类空间查询", err))?;
                    let mut rows = stmt
                        .query(params![
                            q_min_x,
                            q_max_x,
                            q_min_y,
                            q_max_y,
                            q_min_z,
                            q_max_z,
                            level_id,
                            k.as_str()
                        ])
                        .map_err(|err| DbError::sqlite("执行关卡分类空间查询", err))?;
                    collect_nearby_rows(&mut rows, center, radius, &mut out)?;
                }
                (ActorScope::Map(map_id), None) => {
                    let mut stmt = conn
                        .prepare_cached(sql::ACTORS_NEAR_MAP)
                        .map_err(|err| DbError::sqlite("准备地图空间查询", err))?;
                    let mut rows = stmt
                        .query(params![
                            q_min_x, q_max_x, q_min_y, q_max_y, q_min_z, q_max_z, map_id
                        ])
                        .map_err(|err| DbError::sqlite("执行地图空间查询", err))?;
                    collect_nearby_rows(&mut rows, center, radius, &mut out)?;
                }
                (ActorScope::Map(map_id), Some(k)) => {
                    let mut stmt = conn
                        .prepare_cached(sql::ACTORS_NEAR_MAP_KIND)
                        .map_err(|err| DbError::sqlite("准备地图分类空间查询", err))?;
                    let mut rows = stmt
                        .query(params![
                            q_min_x,
                            q_max_x,
                            q_min_y,
                            q_max_y,
                            q_min_z,
                            q_max_z,
                            map_id,
                            k.as_str()
                        ])
                        .map_err(|err| DbError::sqlite("执行地图分类空间查询", err))?;
                    collect_nearby_rows(&mut rows, center, radius, &mut out)?;
                }
            }

            out.sort_by(|a, b| {
                a.distance
                    .total_cmp(&b.distance)
                    .then_with(|| a.actor_id.cmp(&b.actor_id))
            });
            Ok(out)
        })
    }

    /// Return up to `limit` actors whose `class` starts with `class_prefix`, using the `actor(class)`
    /// B-tree range index.
    pub fn actors_by_class(&self, class_prefix: &str, limit: usize) -> Result<Vec<LevelActor>> {
        if class_prefix.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let upper = format!("{class_prefix}\u{10FFFF}");
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);

        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::ACTORS_BY_CLASS_PREFIX)
                .map_err(|err| DbError::sqlite("准备按类名前缀查询 actor", err))?;
            let mut rows = stmt
                .query(params![class_prefix, upper, limit_i64])
                .map_err(|err| DbError::sqlite("按类名前缀查询 actor", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 actor 查询结果", err))?
            {
                let (actor_id, level_name, actor) = read_actor_from_row(row)?;
                out.push(LevelActor {
                    actor_id,
                    level_name,
                    actor,
                });
            }
            Ok(out)
        })
    }

    /// Return all [`EncounterZone`] definitions placed in `map_id`.
    pub fn encounter_zones(&self, map_id: i64) -> Result<Vec<EncounterZone>> {
        let objects: Vec<LevelObject> = self.with_conn(|conn| {
            query_docs(conn, "level_object", sql::ENCOUNTER_ZONES_BY_MAP, [map_id])
        })?;
        let mut zones = Vec::with_capacity(objects.len());
        for obj in objects {
            if let LevelObjectData::EncounterZone(ez) = obj.data {
                zones.push(ez);
            }
        }
        Ok(zones)
    }

    /// Spatial proximity query returning classified [`LevelObject`]s inside `map_id` within
    /// `radius` of `(x, y, z)`.
    ///
    /// When `z` is `Some(z_val)`, computes 3D Euclidean distance to each object's actor AABB;
    /// when `z` is `None`, queries across all Z elevations and computes 2D horizontal distance.
    /// Optionally filters results to `kinds`, sorted by `(distance, object_id)`.
    pub fn level_objects_near(
        &self,
        map_id: i64,
        x: f64,
        y: f64,
        z: Option<f64>,
        radius: f64,
        kinds: Option<&[LevelObjectKind]>,
    ) -> Result<Vec<NearbyLevelObject>> {
        if !x.is_finite()
            || !y.is_finite()
            || z.is_some_and(|zv| !zv.is_finite())
            || !radius.is_finite()
            || radius < 0.0
        {
            return Ok(Vec::new());
        }

        // Pad query box slightly for SQLite R*Tree's internal 32-bit float rounding.
        let r_pad = radius + 1e-3;
        let q_min_x = x - r_pad;
        let q_max_x = x + r_pad;
        let q_min_y = y - r_pad;
        let q_max_y = y + r_pad;
        let (q_min_z, q_max_z) = match z {
            Some(zv) => (zv - r_pad, zv + r_pad),
            None => (-1.0e9_f64, 1.0e9_f64),
        };
        let eps = 1e-6;

        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::LEVEL_OBJECTS_NEAR_MAP)
                .map_err(|err| DbError::sqlite("准备地图对象空间查询", err))?;
            let mut rows = stmt
                .query(params![
                    q_min_x, q_max_x, q_min_y, q_max_y, q_min_z, q_max_z, map_id
                ])
                .map_err(|err| DbError::sqlite(format!("执行地图 {map_id} 对象空间查询"), err))?;

            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取地图对象空间查询结果", err))?
            {
                let object_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 level_object.id", err))?;
                let actor_id: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 actor.id", err))?;
                let raw_doc: String = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 level_object.doc", err))?;
                let object: LevelObject = decode_doc("level_object", object_id, &raw_doc)?;

                if let Some(allowed) = kinds
                    && !allowed.contains(&object.kind)
                {
                    continue;
                }

                let bounds_min = Vec3::new(
                    row.get(3)
                        .map_err(|err| DbError::sqlite("读取 actor_rtree.min_x", err))?,
                    row.get(5)
                        .map_err(|err| DbError::sqlite("读取 actor_rtree.min_y", err))?,
                    row.get(7)
                        .map_err(|err| DbError::sqlite("读取 actor_rtree.min_z", err))?,
                );
                let bounds_max = Vec3::new(
                    row.get(4)
                        .map_err(|err| DbError::sqlite("读取 actor_rtree.max_x", err))?,
                    row.get(6)
                        .map_err(|err| DbError::sqlite("读取 actor_rtree.max_y", err))?,
                    row.get(8)
                        .map_err(|err| DbError::sqlite("读取 actor_rtree.max_z", err))?,
                );

                // For point objects (zero extent within f32 tolerance), use exact f64 location when
                // available; for volumes (encounter zones, trigger boxes), measure distance to AABB.
                let is_point_aabb = (bounds_max.x - bounds_min.x).abs() <= 1e-2
                    && (bounds_max.y - bounds_min.y).abs() <= 1e-2
                    && (bounds_max.z - bounds_min.z).abs() <= 1e-2;
                let (eff_min, eff_max) = if is_point_aabb
                    && let Some(loc) = object.location
                    && loc.is_finite()
                {
                    (loc, loc)
                } else {
                    (bounds_min, bounds_max)
                };

                let distance = match z {
                    Some(zv) => point_to_aabb_distance(Vec3::new(x, y, zv), eff_min, eff_max),
                    None => point_to_aabb_distance_2d(x, y, eff_min, eff_max),
                };

                if distance <= radius + eps {
                    out.push(NearbyLevelObject {
                        object_id,
                        actor_id,
                        object,
                        distance,
                        bounds_min: eff_min,
                        bounds_max: eff_max,
                    });
                }
            }

            out.sort_by(|a, b| {
                a.distance
                    .total_cmp(&b.distance)
                    .then_with(|| a.object_id.cmp(&b.object_id))
            });
            Ok(out)
        })
    }

    /// Return all NPC shop placements for single `shop_id` (including shops exposed via
    /// `VariousShop` counters), enriched with map Chinese name and optional `map_unit` names.
    pub fn shop_locations(&self, shop_id: i64) -> Result<Vec<ShopLocationEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::SHOP_LOCATIONS_BY_SHOP_ID)
                .map_err(|err| DbError::sqlite("准备查询商店 NPC 位置", err))?;
            let mut rows = stmt
                .query([shop_id])
                .map_err(|err| DbError::sqlite(format!("查询商店 {shop_id} 的 NPC 位置"), err))?;

            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取商店 NPC 位置行", err))?
            {
                let map_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 map_shop.map_id", err))?;
                let map_name_zh: Option<String> = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 map.name_zh", err))?;
                let resolved_shop_id: i64 = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 map_shop.shop_id", err))?;
                let source_shop_id: i64 = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 map_shop.source_shop_id", err))?;
                let is_various: i64 = row
                    .get(4)
                    .map_err(|err| DbError::sqlite("读取 map_shop.is_various_shop", err))?;
                let lo_id: i64 = row
                    .get(5)
                    .map_err(|err| DbError::sqlite("读取 level_object.id", err))?;
                let raw_doc: String = row
                    .get(6)
                    .map_err(|err| DbError::sqlite("读取 level_object.doc", err))?;

                let obj: LevelObject = decode_doc("level_object", lo_id, &raw_doc)?;
                let (map_unit_id, interact_type) = match &obj.data {
                    LevelObjectData::Shop(s) => (s.map_unit_id, s.interact_type.clone()),
                    _ => (None, None),
                };

                let (npc_name, npc_name_zh) = match map_unit_id {
                    Some(mu_id) => lookup_map_unit_names(conn, mu_id)?,
                    None => (None, None),
                };

                out.push(ShopLocationEntry {
                    map_id,
                    map_name_zh,
                    shop_id: resolved_shop_id,
                    various_shop_id: (is_various != 0).then_some(source_shop_id),
                    level_name: obj.level_name,
                    actor_name: obj.actor_name,
                    unique_id: obj.unique_id,
                    map_unit_id,
                    npc_name,
                    npc_name_zh,
                    interact_type,
                    visible_condition: obj.visible_condition,
                    location: obj.location,
                });
            }
            Ok(out)
        })
    }

    /// Return all placed treasure-box [`LevelObject`]s whose static item reference matches
    /// `item_id`, indexed by `idx_level_object_kind_ref`.
    pub fn treasure_box_objects_for_item(&self, item_id: i64) -> Result<Vec<LevelObject>> {
        let ref_key = item_id.to_string();
        self.with_conn(|conn| {
            query_docs(
                conn,
                "level_object",
                sql::TREASURE_BOX_OBJECTS_BY_ITEM,
                [ref_key.as_str()],
            )
        })
    }
}

// ---- Internal helpers ----

fn lookup_level_id(conn: &Connection, level_name: &str) -> Result<Option<i64>> {
    let mut stmt = conn
        .prepare_cached(sql::LEVEL_ID_BY_NAME)
        .map_err(|err| DbError::sqlite("准备查询 level.id", err))?;
    stmt.query_row([level_name], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询关卡 {level_name} 编号"), err))
}

fn query_map_exits_on_conn(
    conn: &Connection,
    query_sql: &str,
    map_id: i64,
) -> Result<Vec<MapExit>> {
    let mut stmt = conn
        .prepare_cached(query_sql)
        .map_err(|err| DbError::sqlite("准备查询 map_edge", err))?;
    let mut rows = stmt
        .query([map_id])
        .map_err(|err| DbError::sqlite(format!("查询地图 {map_id} 出口边"), err))?;

    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 map_edge 行", err))?
    {
        let from_map_id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 map_edge.from_map", err))?;
        let source_level: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 map_edge.level_name", err))?;
        let actor_name: String = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 map_edge.actor_name", err))?;
        let unique_id: Option<i64> = row
            .get(3)
            .map_err(|err| DbError::sqlite("读取 map_edge.unique_id", err))?;
        let target_map_id: Option<i64> = row
            .get(4)
            .map_err(|err| DbError::sqlite("读取 map_edge.to_map", err))?;
        let target_point_id: Option<i64> = row
            .get(5)
            .map_err(|err| DbError::sqlite("读取 map_edge.to_point", err))?;
        let kind_str: String = row
            .get(6)
            .map_err(|err| DbError::sqlite("读取 map_edge.kind", err))?;
        let kind = MapEdgeKind::parse(&kind_str).ok_or_else(|| {
            DbError::sqlite(
                format!("无法识别的 map_edge.kind：{kind_str}"),
                rusqlite::Error::InvalidQuery,
            )
        })?;
        let is_auto: i64 = row
            .get(7)
            .map_err(|err| DbError::sqlite("读取 map_edge.is_auto", err))?;
        let is_event_only: i64 = row
            .get(8)
            .map_err(|err| DbError::sqlite("读取 map_edge.is_event_only", err))?;
        let condition: Option<String> = row
            .get(9)
            .map_err(|err| DbError::sqlite("读取 map_edge.condition", err))?;

        let location = read_opt_vec3(row, 10, "map_edge")?;
        let target_location = read_opt_vec3(row, 13, "transition_point")?;
        let target_direction: Option<String> = row
            .get(16)
            .map_err(|err| DbError::sqlite("读取 transition_point.direction", err))?;

        let is_same_map = kind == MapEdgeKind::SameMap || target_map_id == Some(from_map_id);
        out.push(MapExit {
            from_map_id,
            kind,
            source_level,
            actor_name,
            unique_id,
            target_map_id,
            target_point_id,
            target_location,
            target_direction,
            is_same_map,
            is_auto_transition: is_auto != 0,
            is_event_only: is_event_only != 0,
            condition,
            location,
        });
    }
    Ok(out)
}

fn collect_nearby_rows(
    rows: &mut rusqlite::Rows<'_>,
    center: Vec3,
    radius: f64,
    out: &mut Vec<NearbyActor>,
) -> Result<()> {
    let eps = 1e-6;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取空间查询结果行", err))?
    {
        let (actor_id, level_name, actor) = read_actor_from_row(row)?;
        let r_min = Vec3::new(
            row.get(18)
                .map_err(|err| DbError::sqlite("读取 actor_rtree.min_x", err))?,
            row.get(20)
                .map_err(|err| DbError::sqlite("读取 actor_rtree.min_y", err))?,
            row.get(22)
                .map_err(|err| DbError::sqlite("读取 actor_rtree.min_z", err))?,
        );
        let r_max = Vec3::new(
            row.get(19)
                .map_err(|err| DbError::sqlite("读取 actor_rtree.max_x", err))?,
            row.get(21)
                .map_err(|err| DbError::sqlite("读取 actor_rtree.max_y", err))?,
            row.get(23)
                .map_err(|err| DbError::sqlite("读取 actor_rtree.max_z", err))?,
        );

        // Prefer exact f64 world_aabb from the Actor over f32-rounded R*Tree bounds for boundary precision.
        let (bounds_min, bounds_max) = actor.world_aabb().unwrap_or((r_min, r_max));
        let distance = point_to_aabb_distance(center, bounds_min, bounds_max);
        if distance <= radius + eps {
            out.push(NearbyActor {
                actor_id,
                level_name,
                actor,
                distance,
                bounds_min,
                bounds_max,
            });
        }
    }
    Ok(())
}

fn read_actor_from_row(row: &Row<'_>) -> Result<(i64, String, Actor)> {
    let actor_id: i64 = row
        .get(0)
        .map_err(|err| DbError::sqlite("读取 actor.id", err))?;
    let level_name: String = row
        .get(1)
        .map_err(|err| DbError::sqlite("读取 level.name", err))?;
    let name: String = row
        .get(2)
        .map_err(|err| DbError::sqlite("读取 actor.name", err))?;
    let label: Option<String> = row
        .get(3)
        .map_err(|err| DbError::sqlite("读取 actor.label", err))?;
    let class: String = row
        .get(4)
        .map_err(|err| DbError::sqlite("读取 actor.class", err))?;
    let is_game_class: i64 = row
        .get(5)
        .map_err(|err| DbError::sqlite("读取 actor.is_game_class", err))?;

    let location = read_opt_vec3(row, 6, "actor.location")?;
    let rotation = read_opt_rot3(row, 9)?;
    let scale = read_opt_vec3(row, 12, "actor.scale")?;

    let props_raw: Option<String> = row
        .get(15)
        .map_err(|err| DbError::sqlite("读取 actor.props", err))?;
    let asset_refs_raw: Option<String> = row
        .get(16)
        .map_err(|err| DbError::sqlite("读取 actor.asset_refs", err))?;
    let subobjects_raw: Option<String> = row
        .get(17)
        .map_err(|err| DbError::sqlite("读取 actor.subobjects", err))?;

    let properties: Option<Map<String, Value>> = props_raw
        .as_deref()
        .map(|s| decode_doc("actor.props", actor_id, s))
        .transpose()?;
    let asset_refs: Option<Vec<String>> = asset_refs_raw
        .as_deref()
        .map(|s| decode_doc("actor.asset_refs", actor_id, s))
        .transpose()?;
    let subobjects: Option<Vec<ActorSubobject>> = subobjects_raw
        .as_deref()
        .map(|s| decode_doc("actor.subobjects", actor_id, s))
        .transpose()?;

    Ok((
        actor_id,
        level_name,
        Actor {
            name,
            label,
            class,
            is_game_class: is_game_class != 0,
            location,
            rotation,
            scale,
            asset_refs,
            properties,
            subobjects,
        },
    ))
}

fn read_opt_vec3(row: &Row<'_>, start_col: usize, label: &str) -> Result<Option<Vec3>> {
    let x: Option<f64> = row
        .get(start_col)
        .map_err(|err| DbError::sqlite(format!("读取 {label}.x"), err))?;
    let y: Option<f64> = row
        .get(start_col + 1)
        .map_err(|err| DbError::sqlite(format!("读取 {label}.y"), err))?;
    let z: Option<f64> = row
        .get(start_col + 2)
        .map_err(|err| DbError::sqlite(format!("读取 {label}.z"), err))?;
    Ok(match (x, y, z) {
        (Some(x), Some(y), Some(z)) => Some(Vec3::new(x, y, z)),
        _ => None,
    })
}

fn read_opt_rot3(row: &Row<'_>, start_col: usize) -> Result<Option<Rot3>> {
    let pitch: Option<f64> = row
        .get(start_col)
        .map_err(|err| DbError::sqlite("读取 actor.pitch", err))?;
    let yaw: Option<f64> = row
        .get(start_col + 1)
        .map_err(|err| DbError::sqlite("读取 actor.yaw", err))?;
    let roll: Option<f64> = row
        .get(start_col + 2)
        .map_err(|err| DbError::sqlite("读取 actor.roll", err))?;
    Ok(match (pitch, yaw, roll) {
        (Some(pitch), Some(yaw), Some(roll)) => Some(Rot3 { pitch, yaw, roll }),
        _ => None,
    })
}

fn lookup_map_unit_names(
    conn: &Connection,
    map_unit_id: i64,
) -> Result<(Option<String>, Option<String>)> {
    let mut stmt = conn
        .prepare_cached(sql::MAP_UNIT_NAMES_BY_ID)
        .map_err(|err| DbError::sqlite("准备查询 map_unit 名称", err))?;
    let pair: Option<(String, Option<String>)> = stmt
        .query_row([map_unit_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询 map_unit {map_unit_id} 名称"), err))?;
    Ok(match pair {
        Some((name, name_zh)) => (Some(name), name_zh),
        None => (None, None),
    })
}

fn point_to_aabb_distance_2d(px: f64, py: f64, min: Vec3, max: Vec3) -> f64 {
    let dx = if px < min.x {
        min.x - px
    } else if px > max.x {
        px - max.x
    } else {
        0.0
    };
    let dy = if py < min.y {
        min.y - py
    } else if py > max.y {
        py - max.y
    } else {
        0.0
    };
    dx.hypot(dy)
}

fn point_to_aabb_distance(p: Vec3, min: Vec3, max: Vec3) -> f64 {
    let dx = if p.x < min.x {
        min.x - p.x
    } else if p.x > max.x {
        p.x - max.x
    } else {
        0.0
    };
    let dy = if p.y < min.y {
        min.y - p.y
    } else if p.y > max.y {
        p.y - max.y
    } else {
        0.0
    };
    let dz = if p.z < min.z {
        min.z - p.z
    } else if p.z > max.z {
        p.z - max.z
    } else {
        0.0
    };
    (dx * dx + dy * dy + dz * dz).sqrt()
}
