//! Cross-area links (`11_links`) `impl GameDb` read queries.
//!
//! Connects talk event chains (`talk_event`, `talk_event_next`, `talk_line`, `level_object_talk`),
//! map units & composite NPC profiles (`map_unit`, `npc`, `where_is_map_unit`), and normalized
//! cross-area class resolution (`class_ref`, `class_info`).
//!
//! Every query uses `Connection::prepare_cached` and index-backed predicates verified via
//! `assert_db_uses_index` in `tests/links.rs`.

use std::collections::HashSet;

use ffr_data::assets::blueprint::Blueprint;
use ffr_data::assets::level::{LevelObject, LevelObjectData, Vec3};
use ffr_data::assets::reflection::UeClass;
use ffr_data::assets::sequence::Sequence;
use ffr_data::world::{Inn, TalkEvent, TalkLine};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::build::links::clean_raw_class_name;
use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::query::world::ResolvedShop;
use crate::record::{decode_doc, query_doc_opt, query_docs};

/// Maximum inheritance depth walked by [`GameDb::class_info`].
const MAX_CLASS_CHAIN_DEPTH: usize = 64;

/// Default inn ID used by `INN/DT_InnList.json` when a map inn trigger carries a generic ID (e.g. `0`).
const DEFAULT_INN_ID: i64 = -1;

/// One animation sprite asset entry from `Asset/Map/DT_MapUnitAsset.animationAssetList[]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapUnitAnimationAsset {
    pub is_vision_character: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ss6_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_base_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_normal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_metallic_roughness: Option<String>,
}

/// Map unit appearance/asset record parsed from `Asset/Map/DT_MapUnitAsset` (`map_unit.doc`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapUnit {
    pub id: i64,
    /// Original row key from `DT_MapUnitAsset` (typically a Japanese label such as `"主人公A"` or
    /// `"兵士B"`).
    pub row_key: String,
    /// Internal asset name (same as `row_key`).
    pub name: String,
    /// Localization namespace from `playable_unit` when linked to a playable character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_ns: Option<String>,
    /// Localization key from `playable_unit` when linked to a playable character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_key: Option<String>,
    /// Strictly resolved `"zh-Hans"` name (`None` when unavailable; never falls back to Japanese).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_zh: Option<String>,
    /// Primary SpriteStudio6 project asset path (`animationAssetList[0].Ss6Project`), if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_asset: Option<String>,
    #[serde(default)]
    pub animations: Vec<MapUnitAnimationAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footstep: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specular_scale: Option<f64>,
}

/// One dialogue line resolved into a requested language (`None` when missing, never falling back
/// to Japanese).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTalkLine {
    pub table_path: String,
    pub row_key: String,
    pub ord: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talk_id: Option<i64>,
    pub speaker: String,
    pub emotion: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_ns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_key: Option<String>,
    /// Localized text in `lang`, or `None` if absent in the `text` table for `lang`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub lang: String,
    #[serde(default)]
    pub voice: String,
    pub line: TalkLine,
}

/// One outgoing next-event edge from `talk_event_next`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkEventNextEntry {
    pub next_event: String,
    /// `"continuous"` or `"choice"`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_index: Option<i64>,
    #[serde(default)]
    pub condition: String,
    pub ord: usize,
}

/// Composite NPC profile returned by [`GameDb::npc`], combining map-unit metadata, localized
/// name, level placement, coordinates, talk events, shops, and inn details in one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcInfo {
    pub map_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_name_zh: Option<String>,
    pub unique_id: i64,
    pub level_name: String,
    pub actor_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interact_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_unit_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_unit: Option<MapUnit>,
    /// Internal asset name from `DT_MapUnitAsset` (`map_unit.name`), if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Chinese display name (`map_unit.name_zh`), or `None` when untranslated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_zh: Option<String>,
    #[serde(default)]
    pub talk_events: Vec<TalkEvent>,
    #[serde(default)]
    pub shops: Vec<ResolvedShop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub various_shop_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inn_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inn: Option<Inn>,
}

/// One level placement of a map unit returned by [`GameDb::where_is_map_unit`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapUnitOccurrence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_name_zh: Option<String>,
    pub level_name: String,
    pub actor_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_id: Option<i64>,
    pub unit_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interact_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shop_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inn_id: Option<i64>,
}

/// One node along a class's unified Blueprint + C++ reflection inheritance chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassChainNode {
    /// Canonical UE class path (or short name when unresolved).
    pub name: String,
    pub short_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ue_class_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint_id: Option<String>,
}

/// Unified class information returned by [`GameDb::class_info`], combining reflected `ue_class`,
/// exported `blueprint`, and the full Blueprint → C++ inheritance chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassInfo {
    pub query: String,
    pub normalized: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ue_class_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ue_class: Option<UeClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<Blueprint>,
    /// Inheritance chain from this class (index 0) up to its root ancestor.
    #[serde(default)]
    pub parent_chain: Vec<ClassChainNode>,
}

/// Exposed SQL statements executed by `11_links` queries so integration tests can verify every
/// query plan with `assert_db_uses_index`.
pub mod sql {
    pub const TALK_EVENT_BY_ID: &str = "SELECT doc FROM talk_event WHERE id = ?1";

    pub const TALK_EVENT_NEXT_BY_EVENT: &str = "\
        SELECT next_event, kind, choice_index, condition, ord \
        FROM talk_event_next \
        WHERE event = ?1 \
        ORDER BY kind, ord";

    pub const TALK_LINES_BY_TABLE: &str = "\
        SELECT \
            tl.table_path, tl.row, tl.ord, tl.talk_id, tl.speaker, tl.emotion, \
            tl.text_ns, tl.text_key, tl.voice, tl.doc, t.value \
        FROM talk_line tl \
        LEFT JOIN text t \
            ON t.ns = tl.text_ns AND t.key = tl.text_key AND t.lang = ?2 \
        WHERE tl.table_path = ?1 \
        ORDER BY tl.ord";

    pub const TALK_LINES_BY_TABLE_AND_TALK_ID: &str = "\
        SELECT \
            tl.table_path, tl.row, tl.ord, tl.talk_id, tl.speaker, tl.emotion, \
            tl.text_ns, tl.text_key, tl.voice, tl.doc, t.value \
        FROM talk_line tl \
        LEFT JOIN text t \
            ON t.ns = tl.text_ns AND t.key = tl.text_key AND t.lang = ?3 \
        WHERE tl.table_path = ?1 AND tl.talk_id = ?2 \
        ORDER BY tl.ord";

    pub const TALK_EVENTS_OF_OBJECT: &str = "\
        SELECT te.id, te.doc \
        FROM level_object lo \
        CROSS JOIN level_object_talk lot ON lot.level_object_id = lo.id \
        CROSS JOIN talk_event te ON te.id = lot.talk_event_id \
        WHERE lo.map_id = ?1 AND lo.unique_id = ?2 \
        ORDER BY lo.id, lot.ord, te.id";

    pub const SEQUENCE_OF_TALK_EVENT: &str = "\
        SELECT s.id, s.doc \
        FROM talk_event te \
        CROSS JOIN sequence s ON s.id = te.sequence_id \
        WHERE te.id = ?1";

    pub const TALK_EVENTS_WITH_BATTLE: &str = "\
        SELECT id, doc \
        FROM talk_event \
        WHERE encount_group_id >= 0 \
        ORDER BY encount_group_id, id";

    pub const MAP_UNIT_BY_ID: &str = "SELECT doc FROM map_unit WHERE id = ?1";

    pub const MAP_UNITS_BY_NAME_ZH: &str = "\
        SELECT id, doc \
        FROM map_unit INDEXED BY idx_map_unit_name_zh \
        WHERE name_zh IS NOT NULL AND instr(name_zh, ?1) > 0 \
        ORDER BY id";

    pub const MAP_UNITS_BY_ROW_KEY: &str = "\
        SELECT id, doc \
        FROM map_unit INDEXED BY idx_map_unit_row_key \
        WHERE instr(row_key, ?1) > 0 \
        ORDER BY id";

    pub const LEVEL_OBJECTS_FOR_NPC: &str = "\
        SELECT id, kind, doc \
        FROM level_object \
        WHERE map_id = ?1 AND unique_id = ?2 \
        ORDER BY id";

    pub const MAP_NAME_ZH_BY_ID: &str = "SELECT name_zh FROM map WHERE id = ?1";

    pub const INN_BY_ID: &str = "SELECT doc FROM inn WHERE id = ?1";

    pub const VARIOUS_SHOP_MEMBER_IDS: &str = "\
        SELECT shop_id \
        FROM various_shop_member \
        WHERE various_shop_id = ?1 \
        ORDER BY slot";

    pub const WHERE_IS_MAP_UNIT: &str = "\
        SELECT lo.id, lo.map_id, m.name_zh, lo.doc \
        FROM level_object lo \
        LEFT JOIN map m ON m.id = lo.map_id \
        WHERE lo.kind = 'map_unit' AND lo.ref_id = ?1 \
        ORDER BY lo.map_id, lo.id";

    pub const CLASS_REF_BY_RAW: &str = "\
        SELECT normalized, ue_class_id, blueprint_id \
        FROM class_ref \
        WHERE raw_name = ?1";

    pub const CLASS_REF_BY_NORMALIZED: &str = "\
        SELECT normalized, ue_class_id, blueprint_id \
        FROM class_ref \
        WHERE normalized = ?1 \
        ORDER BY raw_name \
        LIMIT 1";

    pub const UE_CLASS_BY_PATH: &str = "SELECT doc FROM ue_class WHERE path = ?1";

    pub const UE_CLASS_BY_SHORT_NAME: &str = "\
        SELECT path, doc \
        FROM ue_class \
        WHERE short_name = ?1 \
        ORDER BY path";

    pub const BLUEPRINT_BY_PATH: &str = "SELECT doc FROM blueprint WHERE path = ?1";

    pub const BLUEPRINT_BY_CLASS_NAME: &str = "\
        SELECT path, doc \
        FROM blueprint \
        WHERE class_name = ?1 \
        ORDER BY path";
}

impl GameDb {
    /// Look up one [`TalkEvent`] by its row key in `CDT_TalkEventData_Prod`.
    pub fn talk_event(&self, id: &str) -> Result<Option<TalkEvent>> {
        self.with_conn(|conn| query_doc_opt(conn, "talk_event", id, sql::TALK_EVENT_BY_ID, [id]))
    }

    /// Return outgoing continuous and choice-branch transitions from `talk_event` `id`.
    pub fn talk_event_next(&self, id: &str) -> Result<Vec<TalkEventNextEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::TALK_EVENT_NEXT_BY_EVENT)
                .map_err(|err| DbError::sqlite("准备查询 talk_event_next", err))?;
            let mut rows = stmt
                .query([id])
                .map_err(|err| DbError::sqlite(format!("查询对话事件 {id} 的后续事件"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 talk_event_next 行", err))?
            {
                let ord_i64: i64 = row
                    .get(4)
                    .map_err(|err| DbError::sqlite("读取 talk_event_next.ord", err))?;
                out.push(TalkEventNextEntry {
                    next_event: row
                        .get(0)
                        .map_err(|err| DbError::sqlite("读取 talk_event_next.next_event", err))?,
                    kind: row
                        .get(1)
                        .map_err(|err| DbError::sqlite("读取 talk_event_next.kind", err))?,
                    choice_index: row
                        .get(2)
                        .map_err(|err| DbError::sqlite("读取 talk_event_next.choice_index", err))?,
                    condition: row
                        .get(3)
                        .map_err(|err| DbError::sqlite("读取 talk_event_next.condition", err))?,
                    ord: usize::try_from(ord_i64).unwrap_or(0),
                });
            }
            Ok(out)
        })
    }

    /// Resolve dialogue lines for `event_id` in the requested language `lang`.
    ///
    /// Missing translations strictly return `text: None` and never fall back to Japanese.
    /// When `event_id`'s `ArgList` specifies a numeric `id` (shared free-talk tables), only the
    /// matching `talk_id` section is returned; otherwise all lines of the table are returned in
    /// order.
    pub fn talk_lines(&self, event_id: &str, lang: &str) -> Result<Vec<ResolvedTalkLine>> {
        self.with_conn(|conn| {
            let Some(event): Option<TalkEvent> = query_doc_opt(
                conn,
                "talk_event",
                event_id,
                sql::TALK_EVENT_BY_ID,
                [event_id],
            )?
            else {
                return Ok(Vec::new());
            };
            let Some(table_path) = event.talk_table_path.as_deref() else {
                return Ok(Vec::new());
            };
            if let Some(talk_id) = event.talk_id_arg() {
                let mut stmt = conn
                    .prepare_cached(sql::TALK_LINES_BY_TABLE_AND_TALK_ID)
                    .map_err(|err| DbError::sqlite("准备按段落查询 talk_line", err))?;
                let mut rows = stmt
                    .query(params![table_path, talk_id, lang])
                    .map_err(|err| {
                        DbError::sqlite(format!("查询对话 {event_id} 段落 {talk_id} 台词"), err)
                    })?;
                collect_resolved_talk_lines(&mut rows, lang)
            } else {
                let mut stmt = conn
                    .prepare_cached(sql::TALK_LINES_BY_TABLE)
                    .map_err(|err| DbError::sqlite("准备查询 talk_line", err))?;
                let mut rows = stmt
                    .query(params![table_path, lang])
                    .map_err(|err| DbError::sqlite(format!("查询对话 {event_id} 台词"), err))?;
                collect_resolved_talk_lines(&mut rows, lang)
            }
        })
    }

    /// Return all [`TalkEvent`]s attached to level objects matching `(map_id, unique_id)`.
    pub fn talk_events_of_object(&self, map_id: i64, unique_id: i64) -> Result<Vec<TalkEvent>> {
        self.with_conn(|conn| query_talk_events_of_object_on_conn(conn, map_id, unique_id))
    }

    /// Return the [`Sequence`] linked to `talk_event` `id`, if resolved in the `sequence` table.
    pub fn sequence_of_talk_event(&self, id: &str) -> Result<Option<Sequence>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::SEQUENCE_OF_TALK_EVENT)
                .map_err(|err| DbError::sqlite("准备查询对话关联过场", err))?;
            let pair: Option<(i64, String)> = stmt
                .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()
                .map_err(|err| DbError::sqlite(format!("查询对话 {id} 关联过场"), err))?;
            pair.map(|(seq_id, raw)| decode_doc("sequence", seq_id, &raw))
                .transpose()
        })
    }

    /// Return all [`TalkEvent`]s that trigger a battle (`encount_group_id >= 0`), ordered by
    /// `(encount_group_id, id)`.
    pub fn talk_events_with_battle(&self) -> Result<Vec<TalkEvent>> {
        self.with_conn(|conn| query_docs(conn, "talk_event", sql::TALK_EVENTS_WITH_BATTLE, []))
    }

    /// Look up one [`MapUnit`] record from `map_unit` by its numeric ID (`DT_MapUnitAsset.ID`).
    pub fn map_unit(&self, id: i64) -> Result<Option<MapUnit>> {
        self.with_conn(|conn| query_doc_opt(conn, "map_unit", id, sql::MAP_UNIT_BY_ID, [id]))
    }

    /// Search [`MapUnit`] records whose `name_zh` or `row_key` contains `query`, ordered by `id`.
    ///
    /// Uses `idx_map_unit_name_zh` and `idx_map_unit_row_key` indexes and deduplicates matches.
    pub fn search_map_units_by_name(&self, query: &str) -> Result<Vec<MapUnit>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            let mut seen: HashSet<i64> = HashSet::new();
            let mut out: Vec<MapUnit> = Vec::new();
            for unit in
                query_docs::<MapUnit, _>(conn, "map_unit", sql::MAP_UNITS_BY_NAME_ZH, [needle])?
            {
                if seen.insert(unit.id) {
                    out.push(unit);
                }
            }
            for unit in
                query_docs::<MapUnit, _>(conn, "map_unit", sql::MAP_UNITS_BY_ROW_KEY, [needle])?
            {
                if seen.insert(unit.id) {
                    out.push(unit);
                }
            }
            out.sort_by_key(|u| u.id);
            Ok(out)
        })
    }

    /// Fetch the full composite [`NpcInfo`] for `(map_id, unique_id)`: map unit asset, name,
    /// level placement & coordinates, attached talk events, expanded shops, and inn data.
    pub fn npc(&self, map_id: i64, unique_id: i64) -> Result<Option<NpcInfo>> {
        let Some((
            level_name,
            actor_name,
            location,
            direction,
            interact_type,
            visible_condition,
            map_unit_id,
            map_unit,
            map_name_zh,
            talk_events,
            shop_ids,
            various_shop_id,
            inn_id,
            inn,
        )) = self.with_conn(|conn| {
            let objects = query_level_objects_for_npc(conn, map_id, unique_id)?;
            if objects.is_empty() {
                return Ok(None);
            }

            let mut map_unit_obj = None;
            let mut shop_obj = None;
            let mut inn_obj = None;
            for obj in &objects {
                match &obj.data {
                    LevelObjectData::MapUnit(u) if map_unit_obj.is_none() => {
                        map_unit_obj = Some((obj, u));
                    }
                    LevelObjectData::Shop(s) if shop_obj.is_none() => {
                        shop_obj = Some((obj, s));
                    }
                    LevelObjectData::Inn(i) if inn_obj.is_none() => {
                        inn_obj = Some((obj, i));
                    }
                    _ => {}
                }
            }

            let primary = map_unit_obj
                .map(|(o, _)| o)
                .or_else(|| shop_obj.map(|(o, _)| o))
                .or_else(|| inn_obj.map(|(o, _)| o))
                .unwrap_or(&objects[0]);

            let map_unit_id = map_unit_obj
                .and_then(|(_, u)| u.map_unit_id)
                .or_else(|| shop_obj.and_then(|(_, s)| s.map_unit_id))
                .or_else(|| inn_obj.and_then(|(_, i)| i.map_unit_id));

            let direction = map_unit_obj.and_then(|(_, u)| u.direction.clone());
            let interact_type = map_unit_obj
                .and_then(|(_, u)| u.interact_type.clone())
                .or_else(|| shop_obj.and_then(|(_, s)| s.interact_type.clone()))
                .or_else(|| inn_obj.and_then(|(_, i)| i.interact_type.clone()));

            let map_unit: Option<MapUnit> = match map_unit_id {
                Some(mu_id) => {
                    query_doc_opt(conn, "map_unit", mu_id, sql::MAP_UNIT_BY_ID, [mu_id])?
                }
                None => None,
            };

            let map_name_zh: Option<String> = {
                let mut stmt = conn
                    .prepare_cached(sql::MAP_NAME_ZH_BY_ID)
                    .map_err(|err| DbError::sqlite("准备查询 map.name_zh", err))?;
                stmt.query_row([map_id], |row| row.get::<_, Option<String>>(0))
                    .optional()
                    .map_err(|err| DbError::sqlite(format!("查询地图 {map_id} 中文名"), err))?
                    .flatten()
            };

            let talk_events = query_talk_events_of_object_on_conn(conn, map_id, unique_id)?;

            let mut shop_ids = Vec::new();
            let mut various_shop_id = None;
            if let Some((_, sh)) = shop_obj {
                if sh.is_various_shop {
                    various_shop_id = Some(sh.shop_id);
                    let mut stmt = conn
                        .prepare_cached(sql::VARIOUS_SHOP_MEMBER_IDS)
                        .map_err(|err| DbError::sqlite("准备查询综合商店子店", err))?;
                    let mut rows = stmt.query([sh.shop_id]).map_err(|err| {
                        DbError::sqlite(format!("查询综合商店 {} 子店", sh.shop_id), err)
                    })?;
                    while let Some(row) = rows
                        .next()
                        .map_err(|err| DbError::sqlite("读取综合商店子店行", err))?
                    {
                        let sid: i64 = row.get(0).map_err(|err| {
                            DbError::sqlite("读取 various_shop_member.shop_id", err)
                        })?;
                        if !shop_ids.contains(&sid) {
                            shop_ids.push(sid);
                        }
                    }
                    if shop_ids.is_empty() {
                        shop_ids.push(sh.shop_id);
                    }
                } else {
                    shop_ids.push(sh.shop_id);
                }
            } else if let Some((_, u)) = map_unit_obj
                && let Some(sid) = u.shop_id
            {
                shop_ids.push(sid);
            }

            let inn_id = inn_obj
                .map(|(_, i)| i.inn_id)
                .or_else(|| map_unit_obj.and_then(|(_, u)| u.inn_id));
            let inn: Option<Inn> = match inn_id {
                Some(iid) => match query_doc_opt(conn, "inn", iid, sql::INN_BY_ID, [iid])? {
                    Some(found) => Some(found),
                    None => query_doc_opt(
                        conn,
                        "inn",
                        DEFAULT_INN_ID,
                        sql::INN_BY_ID,
                        [DEFAULT_INN_ID],
                    )?,
                },
                None => None,
            };

            Ok(Some((
                primary.level_name.clone(),
                primary.actor_name.clone(),
                primary.location,
                direction,
                interact_type,
                primary.visible_condition.clone(),
                map_unit_id,
                map_unit,
                map_name_zh,
                talk_events,
                shop_ids,
                various_shop_id,
                inn_id,
                inn,
            )))
        })?
        else {
            return Ok(None);
        };

        let mut shops = Vec::with_capacity(shop_ids.len());
        for sid in shop_ids {
            if let Some(resolved) = self.shop(sid)? {
                shops.push(resolved);
            }
        }

        let name = map_unit.as_ref().map(|u| u.name.clone());
        let name_zh = map_unit.as_ref().and_then(|u| u.name_zh.clone());

        Ok(Some(NpcInfo {
            map_id,
            map_name_zh,
            unique_id,
            level_name,
            actor_name,
            location,
            direction,
            interact_type,
            visible_condition,
            map_unit_id,
            map_unit,
            name,
            name_zh,
            talk_events,
            shops,
            various_shop_id,
            inn_id,
            inn,
        }))
    }

    /// Return all level placements referencing `map_unit_id` (`DT_MapUnitAsset.ID`).
    pub fn where_is_map_unit(&self, map_unit_id: i64) -> Result<Vec<MapUnitOccurrence>> {
        let ref_key = map_unit_id.to_string();
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::WHERE_IS_MAP_UNIT)
                .map_err(|err| DbError::sqlite("准备查询地图单位出现位置", err))?;
            let mut rows = stmt.query([ref_key.as_str()]).map_err(|err| {
                DbError::sqlite(format!("查询地图单位 {map_unit_id} 出现位置"), err)
            })?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取地图单位位置行", err))?
            {
                let lo_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 level_object.id", err))?;
                let map_id: Option<i64> = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 level_object.map_id", err))?;
                let map_name_zh: Option<String> = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 map.name_zh", err))?;
                let raw_doc: String = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 level_object.doc", err))?;
                let obj: LevelObject = decode_doc("level_object", lo_id, &raw_doc)?;
                let (unit_kind, interact_type, shop_id, inn_id) = match &obj.data {
                    LevelObjectData::MapUnit(u) => {
                        (u.kind.clone(), u.interact_type.clone(), u.shop_id, u.inn_id)
                    }
                    _ => (obj.kind.as_str().to_owned(), None, None, None),
                };
                out.push(MapUnitOccurrence {
                    map_id,
                    map_name_zh,
                    level_name: obj.level_name,
                    actor_name: obj.actor_name,
                    unique_id: obj.unique_id,
                    unit_kind,
                    interact_type,
                    visible_condition: obj.visible_condition,
                    location: obj.location,
                    shop_id,
                    inn_id,
                });
            }
            Ok(out)
        })
    }

    /// Resolve a class name (raw UE format with prefix like `"Class /Script/..."`, full path,
    /// Blueprint path, or short name with/without `_C`) to its reflected [`UeClass`], exported
    /// [`Blueprint`], and full Blueprint → C++ inheritance chain.
    pub fn class_info(&self, name: &str) -> Result<Option<ClassInfo>> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let cleaned = clean_raw_class_name(trimmed);
        if cleaned.is_empty() {
            return Ok(None);
        }

        self.with_conn(|conn| {
            let mut normalized = cleaned.clone();
            let mut ue_class_id: Option<String> = None;
            let mut blueprint_id: Option<String> = None;

            // 1. Check pre-resolved `class_ref` by raw name or cleaned normalized name.
            if let Some((norm, uc, bp)) = lookup_class_ref(conn, trimmed, &cleaned)? {
                normalized = norm;
                ue_class_id = uc;
                blueprint_id = bp;
            }

            // 2. Load or directly resolve `ue_class` and `blueprint` via indexed queries.
            let mut ue_class = match &ue_class_id {
                Some(path) => query_doc_opt::<UeClass, _>(
                    conn,
                    "ue_class",
                    path,
                    sql::UE_CLASS_BY_PATH,
                    [path.as_str()],
                )?,
                None => resolve_ue_class_direct(conn, &cleaned)?,
            };
            let mut blueprint = match &blueprint_id {
                Some(path) => query_doc_opt::<Blueprint, _>(
                    conn,
                    "blueprint",
                    path,
                    sql::BLUEPRINT_BY_PATH,
                    [path.as_str()],
                )?,
                None => resolve_blueprint_direct(conn, &cleaned)?,
            };

            // 3. Cross-link the two sides when only one was initially resolved.
            if ue_class.is_none()
                && let Some(bp) = &blueprint
            {
                let full_bp_class = format!("{}.{}", bp.path, bp.class_name);
                ue_class = query_doc_opt::<UeClass, _>(
                    conn,
                    "ue_class",
                    &full_bp_class,
                    sql::UE_CLASS_BY_PATH,
                    [full_bp_class.as_str()],
                )?;
            }
            if blueprint.is_none()
                && let Some(uc) = &ue_class
                && let Some(pkg) = game_package_from_class_path(&uc.path)
            {
                blueprint = query_doc_opt::<Blueprint, _>(
                    conn,
                    "blueprint",
                    pkg,
                    sql::BLUEPRINT_BY_PATH,
                    [pkg],
                )?;
            }

            if let Some(uc) = &ue_class {
                ue_class_id = Some(uc.path.clone());
                normalized = uc.path.clone();
            }
            if let Some(bp) = &blueprint {
                blueprint_id = Some(bp.path.clone());
                if ue_class.is_none() {
                    normalized = format!("{}.{}", bp.path, bp.class_name);
                }
            }

            if ue_class.is_none()
                && blueprint.is_none()
                && ue_class_id.is_none()
                && blueprint_id.is_none()
            {
                return Ok(None);
            }

            let parent_chain =
                build_unified_parent_chain(conn, ue_class.as_ref(), blueprint.as_ref())?;

            Ok(Some(ClassInfo {
                query: trimmed.to_owned(),
                normalized,
                ue_class_id,
                blueprint_id,
                ue_class,
                blueprint,
                parent_chain,
            }))
        })
    }
}

// ---- Internal helpers ----

fn collect_resolved_talk_lines(
    rows: &mut rusqlite::Rows<'_>,
    lang: &str,
) -> Result<Vec<ResolvedTalkLine>> {
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 talk_line 行", err))?
    {
        let table_path: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 talk_line.table_path", err))?;
        let row_key: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 talk_line.row", err))?;
        let ord_i64: i64 = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 talk_line.ord", err))?;
        let talk_id: Option<i64> = row
            .get(3)
            .map_err(|err| DbError::sqlite("读取 talk_line.talk_id", err))?;
        let speaker: String = row
            .get(4)
            .map_err(|err| DbError::sqlite("读取 talk_line.speaker", err))?;
        let emotion: String = row
            .get(5)
            .map_err(|err| DbError::sqlite("读取 talk_line.emotion", err))?;
        let text_ns: Option<String> = row
            .get(6)
            .map_err(|err| DbError::sqlite("读取 talk_line.text_ns", err))?;
        let text_key: Option<String> = row
            .get(7)
            .map_err(|err| DbError::sqlite("读取 talk_line.text_key", err))?;
        let voice: String = row
            .get(8)
            .map_err(|err| DbError::sqlite("读取 talk_line.voice", err))?;
        let raw_doc: String = row
            .get(9)
            .map_err(|err| DbError::sqlite("读取 talk_line.doc", err))?;
        let text: Option<String> = row
            .get(10)
            .map_err(|err| DbError::sqlite("读取 text.value", err))?;

        let line: TalkLine = decode_doc("talk_line", format!("{table_path}:{row_key}"), &raw_doc)?;
        out.push(ResolvedTalkLine {
            table_path,
            row_key,
            ord: usize::try_from(ord_i64).unwrap_or(0),
            talk_id,
            speaker,
            emotion,
            text_ns,
            text_key,
            text,
            lang: lang.to_owned(),
            voice,
            line,
        });
    }
    Ok(out)
}

fn query_talk_events_of_object_on_conn(
    conn: &Connection,
    map_id: i64,
    unique_id: i64,
) -> Result<Vec<TalkEvent>> {
    let mut stmt = conn
        .prepare_cached(sql::TALK_EVENTS_OF_OBJECT)
        .map_err(|err| DbError::sqlite("准备查询对象关联对话事件", err))?;
    let mut rows = stmt.query(params![map_id, unique_id]).map_err(|err| {
        DbError::sqlite(format!("查询对象 ({map_id}, {unique_id}) 对话事件"), err)
    })?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取对象对话事件行", err))?
    {
        let id: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 talk_event.id", err))?;
        if !seen.insert(id.clone()) {
            continue;
        }
        let raw_doc: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 talk_event.doc", err))?;
        out.push(decode_doc("talk_event", &id, &raw_doc)?);
    }
    Ok(out)
}

fn query_level_objects_for_npc(
    conn: &Connection,
    map_id: i64,
    unique_id: i64,
) -> Result<Vec<LevelObject>> {
    let mut stmt = conn
        .prepare_cached(sql::LEVEL_OBJECTS_FOR_NPC)
        .map_err(|err| DbError::sqlite("准备查询 NPC 关卡对象", err))?;
    let mut rows = stmt.query(params![map_id, unique_id]).map_err(|err| {
        DbError::sqlite(format!("查询 NPC ({map_id}, {unique_id}) 关卡对象"), err)
    })?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 NPC 关卡对象行", err))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 level_object.id", err))?;
        let raw_doc: String = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 level_object.doc", err))?;
        out.push(decode_doc("level_object", id, &raw_doc)?);
    }
    Ok(out)
}

type ClassRefRow = (String, Option<String>, Option<String>);

fn lookup_class_ref(conn: &Connection, raw: &str, cleaned: &str) -> Result<Option<ClassRefRow>> {
    let mut raw_stmt = conn
        .prepare_cached(sql::CLASS_REF_BY_RAW)
        .map_err(|err| DbError::sqlite("准备按 raw_name 查询 class_ref", err))?;
    if let Some(hit) = raw_stmt
        .query_row([raw], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()
        .map_err(|err| DbError::sqlite("按 raw_name 查询 class_ref", err))?
    {
        return Ok(Some(hit));
    }
    if cleaned != raw
        && let Some(hit) = raw_stmt
            .query_row([cleaned], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .optional()
            .map_err(|err| DbError::sqlite("按规范名查询 class_ref.raw_name", err))?
    {
        return Ok(Some(hit));
    }

    let mut norm_stmt = conn
        .prepare_cached(sql::CLASS_REF_BY_NORMALIZED)
        .map_err(|err| DbError::sqlite("准备按 normalized 查询 class_ref", err))?;
    norm_stmt
        .query_row([cleaned], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()
        .map_err(|err| DbError::sqlite("按 normalized 查询 class_ref", err))
}

fn resolve_ue_class_direct(conn: &Connection, cleaned: &str) -> Result<Option<UeClass>> {
    if cleaned.contains('/') {
        if let Some(found) = query_doc_opt::<UeClass, _>(
            conn,
            "ue_class",
            cleaned,
            sql::UE_CLASS_BY_PATH,
            [cleaned],
        )? {
            return Ok(Some(found));
        }
        // If given `/Game/Path/BP_Foo` without `.BP_Foo_C`, try adding `.BP_Foo_C`.
        if !cleaned.contains('.')
            && let Some(stem) = cleaned.rsplit('/').next()
            && !stem.is_empty()
        {
            let with_c = format!("{cleaned}.{stem}_C");
            if let Some(found) = query_doc_opt::<UeClass, _>(
                conn,
                "ue_class",
                &with_c,
                sql::UE_CLASS_BY_PATH,
                [with_c.as_str()],
            )? {
                return Ok(Some(found));
            }
        }
    }

    let short = extract_short_class_name(cleaned);
    let mut candidates =
        query_docs::<UeClass, _>(conn, "ue_class", sql::UE_CLASS_BY_SHORT_NAME, [short])?;
    if candidates.is_empty() && !short.ends_with("_C") {
        let with_c = format!("{short}_C");
        candidates = query_docs::<UeClass, _>(
            conn,
            "ue_class",
            sql::UE_CLASS_BY_SHORT_NAME,
            [with_c.as_str()],
        )?;
    }
    if candidates.is_empty()
        && let Some(stripped) = short.strip_suffix("_C")
    {
        candidates =
            query_docs::<UeClass, _>(conn, "ue_class", sql::UE_CLASS_BY_SHORT_NAME, [stripped])?;
    }

    Ok(pick_preferred_ue_class(candidates))
}

fn resolve_blueprint_direct(conn: &Connection, cleaned: &str) -> Result<Option<Blueprint>> {
    if cleaned.starts_with('/') {
        let pkg = cleaned.split('.').next().unwrap_or(cleaned);
        if let Some(found) =
            query_doc_opt::<Blueprint, _>(conn, "blueprint", pkg, sql::BLUEPRINT_BY_PATH, [pkg])?
        {
            return Ok(Some(found));
        }
    }

    let short = extract_short_class_name(cleaned);
    let mut candidates =
        query_docs::<Blueprint, _>(conn, "blueprint", sql::BLUEPRINT_BY_CLASS_NAME, [short])?;
    if candidates.is_empty() && !short.ends_with("_C") {
        let with_c = format!("{short}_C");
        candidates = query_docs::<Blueprint, _>(
            conn,
            "blueprint",
            sql::BLUEPRINT_BY_CLASS_NAME,
            [with_c.as_str()],
        )?;
    }
    Ok(candidates.into_iter().next())
}

fn build_unified_parent_chain(
    conn: &Connection,
    initial_ue: Option<&UeClass>,
    initial_bp: Option<&Blueprint>,
) -> Result<Vec<ClassChainNode>> {
    let mut chain = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    let mut cur_ue = initial_ue.cloned();
    let mut cur_bp = initial_bp.cloned();

    for _ in 0..MAX_CLASS_CHAIN_DEPTH {
        let node = match (&cur_ue, &cur_bp) {
            (Some(uc), Some(bp)) => ClassChainNode {
                name: uc.path.clone(),
                short_name: uc.short_name.clone(),
                ue_class_id: Some(uc.path.clone()),
                blueprint_id: Some(bp.path.clone()),
            },
            (Some(uc), None) => ClassChainNode {
                name: uc.path.clone(),
                short_name: uc.short_name.clone(),
                ue_class_id: Some(uc.path.clone()),
                blueprint_id: None,
            },
            (None, Some(bp)) => ClassChainNode {
                name: format!("{}.{}", bp.path, bp.class_name),
                short_name: bp.class_name.clone(),
                ue_class_id: None,
                blueprint_id: Some(bp.path.clone()),
            },
            (None, None) => break,
        };

        if !visited.insert(node.name.clone()) {
            break;
        }
        chain.push(node);

        // Advance to parent: prefer reflected `ue_class.super_class` when available; otherwise
        // walk `blueprint.parent_path` / `blueprint.parent_class` (seamlessly bridging into
        // `ue_class` at the C++ boundary).
        let (next_ue, next_bp) = if let Some(uc) = &cur_ue {
            match uc.super_class.as_deref() {
                Some(sup_path) => {
                    let sup_ue = query_doc_opt::<UeClass, _>(
                        conn,
                        "ue_class",
                        sup_path,
                        sql::UE_CLASS_BY_PATH,
                        [sup_path],
                    )?;
                    let sup_bp = match game_package_from_class_path(sup_path) {
                        Some(pkg) => query_doc_opt::<Blueprint, _>(
                            conn,
                            "blueprint",
                            pkg,
                            sql::BLUEPRINT_BY_PATH,
                            [pkg],
                        )?,
                        None => None,
                    };
                    if sup_ue.is_none() && sup_bp.is_none() {
                        // Record unresolved super_class leaf once and stop.
                        if visited.insert(sup_path.to_owned()) {
                            chain.push(ClassChainNode {
                                name: sup_path.to_owned(),
                                short_name: extract_short_class_name(sup_path).to_owned(),
                                ue_class_id: None,
                                blueprint_id: None,
                            });
                        }
                        break;
                    }
                    (sup_ue, sup_bp)
                }
                None => break,
            }
        } else if let Some(bp) = &cur_bp {
            let mut sup_bp = None;
            let mut sup_ue = None;

            if let Some(parent_path) = bp.parent_path.as_deref() {
                if parent_path.starts_with("/Script/") {
                    if let Some(parent_class) = bp.parent_class.as_deref() {
                        let full = if parent_path.contains('.') {
                            parent_path.to_owned()
                        } else {
                            format!("{parent_path}.{parent_class}")
                        };
                        sup_ue = query_doc_opt::<UeClass, _>(
                            conn,
                            "ue_class",
                            &full,
                            sql::UE_CLASS_BY_PATH,
                            [full.as_str()],
                        )?;
                    }
                } else {
                    sup_bp = resolve_blueprint_direct(conn, parent_path)?;
                }
            }

            if sup_bp.is_none()
                && sup_ue.is_none()
                && let Some(parent_class) = bp.parent_class.as_deref()
            {
                sup_bp = resolve_blueprint_direct(conn, parent_class)?;
                if sup_bp.is_none() {
                    sup_ue = resolve_ue_class_direct(conn, parent_class)?;
                }
            }

            if sup_ue.is_none()
                && let Some(ref sbp) = sup_bp
            {
                let full_bp = format!("{}.{}", sbp.path, sbp.class_name);
                sup_ue = query_doc_opt::<UeClass, _>(
                    conn,
                    "ue_class",
                    &full_bp,
                    sql::UE_CLASS_BY_PATH,
                    [full_bp.as_str()],
                )?;
            }

            if sup_ue.is_none() && sup_bp.is_none() {
                if let Some(parent_class) = bp.parent_class.as_deref() {
                    let leaf_name = match bp.parent_path.as_deref() {
                        Some(pp) if pp.starts_with("/Script/") && !pp.contains('.') => {
                            format!("{pp}.{parent_class}")
                        }
                        _ => parent_class.to_owned(),
                    };
                    if visited.insert(leaf_name.clone()) {
                        chain.push(ClassChainNode {
                            name: leaf_name,
                            short_name: parent_class.to_owned(),
                            ue_class_id: None,
                            blueprint_id: None,
                        });
                    }
                }
                break;
            }
            (sup_ue, sup_bp)
        } else {
            break;
        };

        cur_ue = next_ue;
        cur_bp = next_bp;
    }

    Ok(chain)
}

fn game_package_from_class_path(path: &str) -> Option<&str> {
    if !path.starts_with('/') || path.starts_with("/Script/") {
        return None;
    }
    let pkg = path.split('.').next().unwrap_or(path);
    (!pkg.is_empty()).then_some(pkg)
}

fn extract_short_class_name(cleaned: &str) -> &str {
    if let Some((_, tail)) = cleaned.rsplit_once('.')
        && !tail.is_empty()
    {
        return tail;
    }
    if let Some((_, tail)) = cleaned.rsplit_once('/')
        && !tail.is_empty()
    {
        return tail;
    }
    cleaned
}

fn pick_preferred_ue_class(mut candidates: Vec<UeClass>) -> Option<UeClass> {
    if candidates.len() <= 1 {
        return candidates.pop();
    }
    candidates.sort_by_key(|c| {
        let rank = if c.path.starts_with("/Script/mucha.") {
            0
        } else if c.path.starts_with("/Script/Engine.") {
            1
        } else if c.path.starts_with("/Script/") {
            2
        } else {
            3
        };
        (rank, c.path.clone())
    });
    candidates.into_iter().next()
}
