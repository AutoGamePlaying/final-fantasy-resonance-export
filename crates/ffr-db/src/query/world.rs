//! World area (`04_world`) `impl GameDb` read queries.
//!
//! All queries run via `Connection::prepare_cached`, deserialize `ffr-data` objects from `doc`
//! columns via [`crate::record`], and use primary or secondary indexes.

use std::collections::{BTreeMap, BTreeSet};

use ffr_data::world::{
    Inn, Item, MapFacility, MapInfo, MapIsland, MapRegion, MapReleaseCondition, Quest,
    QuestBoardEntry, SavePoint, SavePointCommand, Shop, ShopItem, TeleportPoint, TreasureBox,
    VariousShop, extract_level_name,
};
use ffr_data::{
    Chapter, CurrentMainScenario, FlagConditionEntry, MainQuest, MainScenarioEntry,
    ResolvedMapPlayableUnit, chapter_no_from_progress, main_quest_id_for_chapter,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record::{decode_doc, query_doc_opt, query_docs, query_keyed_doc_opt};

/// Compact catalog entry for one map, pre-joined with teleportability and treasure count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapCatalogEntry {
    pub map: MapInfo,
    pub name_zh: Option<String>,
    pub teleportable: bool,
    pub treasure_count: usize,
}

/// Directed map-to-map connection derived from static data tables (`map_link`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapLink {
    pub from_map: i64,
    pub to_map: i64,
    pub kind: String,
    pub via: String,
    pub condition: String,
}

/// Static contents associated with a map according to `04_world` data tables.
///
/// Note: `Datatable/` has no static `map_id -> shop_id` foreign key; exact shop NPC placements live
/// in `*_EV.umap` level objects (`08_levels`). `shop_facilities` reports the shop facility markers
/// declared in `CDT_MapData_Prod.facilityDataList`, while `shops` remains empty for pure DataTable
/// builds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapContents {
    pub map: MapInfo,
    pub island: Option<MapIsland>,
    pub region: Option<MapRegion>,
    pub save_point: Option<SavePoint>,
    pub teleport_points: Vec<TeleportPoint>,
    pub treasure_boxes: Vec<TreasureBox>,
    pub inns: Vec<Inn>,
    pub shop_facilities: Vec<MapFacility>,
    pub shops: Vec<Shop>,
}

/// One stock slot of a shop resolved against the `item` catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedShopEntry {
    pub slot_index: usize,
    pub entry: ShopItem,
    /// Resolved [`Item`] from `item`, or `None` when `entry.item_id` is a dangling reference.
    pub item: Option<Item>,
    /// Effective purchase price (`buying_price * price_ratio`), or `None` when dangling.
    pub price: Option<i64>,
}

/// Shop record paired with its stock entries resolved to [`Item`] objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedShop {
    pub shop: Shop,
    pub name_zh: Option<String>,
    pub stock: Vec<ResolvedShopEntry>,
}

/// Drop or steal channel on a unit (`DT_UnitParameter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemDropKind {
    DropNormal,
    DropRare,
    StealNormal,
    StealRare,
}

impl ItemDropKind {
    fn from_db(raw: &str) -> Result<Self> {
        match raw {
            "drop_normal" => Ok(Self::DropNormal),
            "drop_rare" => Ok(Self::DropRare),
            "steal_normal" => Ok(Self::StealNormal),
            "steal_rare" => Ok(Self::StealRare),
            other => Err(DbError::sqlite(
                format!("解析 item_drop.drop_kind 未知值 {other:?}"),
                rusqlite::Error::InvalidQuery,
            )),
        }
    }
}

/// Static acquisition source for an item across treasure boxes, shops, quests, and enemy drops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemSource {
    /// Static treasure box (`CDT_MapGimmickTreasureBox`).
    TreasureBox {
        map_id: i64,
        map_name_zh: Option<String>,
        box_id: i64,
        count: i64,
        is_trap: bool,
    },
    /// Purchasable shop stock (`DT_ShopList`).
    Shop {
        shop_id: i64,
        shop_name_zh: Option<String>,
        price: Option<i64>,
        price_ratio: f64,
        max_order_num: Option<i64>,
        condition: String,
    },
    /// Side quest completion reward (`DT_RequestData`).
    QuestPrize {
        quest_id: i64,
        quest_name_zh: Option<String>,
        count: i64,
    },
    /// Quest-board delivery reward (`CDT_QuestBoardData`).
    QuestBoardReward {
        quest_id: i64,
        quest_name_zh: Option<String>,
        count: Option<i64>,
    },
    /// Enemy / unit defeat drop or steal (`DT_UnitParameter`).
    EnemyDrop {
        unit_id: i64,
        unit_name_zh: Option<String>,
        drop_kind: ItemDropKind,
        rate: i64,
    },
}

impl GameDb {
    // ---- Maps, islands, regions, save/teleport points, links ----

    /// Look up a map by its numeric ID (`CDT_MapData_Prod.ID`).
    pub fn map(&self, id: i64) -> Result<Option<MapInfo>> {
        self.with_conn(|conn| {
            query_doc_opt(conn, "map", id, "SELECT doc FROM map WHERE id = ?1", [id])
        })
    }

    /// Reverse-lookup a map by runtime level name (`*_PL`) or full UE level path.
    pub fn map_by_level(&self, name: &str) -> Result<Option<MapInfo>> {
        let Some(stem) = extract_level_name(name) else {
            return Ok(None);
        };
        self.with_conn(|conn| {
            query_keyed_doc_opt(
                conn,
                "map",
                "SELECT id, doc FROM map WHERE level_name = ?1",
                [stem],
            )
        })
    }

    /// Search maps whose localized name in `lang` contains `query` (ordered by map ID).
    ///
    /// Uses `world_name_fts` (trigram index) when `query` has at least 3 characters and the
    /// covering index on `world_name` for 1-2 character queries. Never falls back to Japanese when
    /// `lang != "ja"`.
    pub fn maps_by_name(&self, query: &str, lang: &str) -> Result<Vec<MapInfo>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            if needle.chars().count() >= 3 {
                let fts_query = fts_literal_phrase(needle);
                query_docs(
                    conn,
                    "map",
                    "SELECT m.id, m.doc \
                     FROM world_name_fts \
                     JOIN map m ON m.id = world_name_fts.entity_id \
                     WHERE world_name_fts MATCH ?1 \
                       AND world_name_fts.entity_kind = 'map' \
                       AND world_name_fts.lang = ?2 \
                     ORDER BY m.id",
                    params![fts_query, lang],
                )
            } else {
                let lower = needle.to_lowercase();
                query_docs(
                    conn,
                    "map",
                    "SELECT m.id, m.doc \
                     FROM world_name w \
                     JOIN map m ON m.id = w.entity_id \
                     WHERE w.entity_kind = 'map' \
                       AND w.lang = ?1 \
                       AND instr(w.name_lower, ?2) > 0 \
                     ORDER BY m.id",
                    params![lang, lower],
                )
            }
        })
    }

    /// Return save-point rules, world-map teleport landmarks, treasure boxes, inns, and data-table
    /// shop facility markers associated with map `id`.
    pub fn map_contents(&self, id: i64) -> Result<Option<MapContents>> {
        self.with_conn(|conn| {
            let Some(map): Option<MapInfo> =
                query_doc_opt(conn, "map", id, "SELECT doc FROM map WHERE id = ?1", [id])?
            else {
                return Ok(None);
            };

            let island: Option<MapIsland> =
                if map.island.is_empty() || map.island == "eMapIslandType::None" {
                    None
                } else {
                    query_doc_opt(
                        conn,
                        "island",
                        &map.island,
                        "SELECT doc FROM island WHERE island_type = ?1",
                        [&map.island],
                    )?
                };

            let region: Option<MapRegion> = match map.region_id {
                Some(rid) => query_doc_opt(
                    conn,
                    "region",
                    rid,
                    "SELECT doc FROM region WHERE id = ?1",
                    [rid],
                )?,
                None => None,
            };

            let save_point: Option<SavePoint> = query_doc_opt(
                conn,
                "save_point",
                &map.map_type,
                "SELECT doc FROM save_point WHERE map_type = ?1",
                [&map.map_type],
            )?;

            let teleport_points: Vec<TeleportPoint> = query_docs(
                conn,
                "teleport_point",
                "SELECT DISTINCT tp.id, tp.doc \
                 FROM teleport_point_target t \
                 JOIN teleport_point tp ON tp.id = t.teleport_id \
                 WHERE t.map_id = ?1 \
                 ORDER BY tp.id",
                [id],
            )?;

            let treasure_boxes: Vec<TreasureBox> = query_docs(
                conn,
                "treasure_box",
                "SELECT id, doc FROM treasure_box WHERE map_id = ?1 ORDER BY id",
                [id],
            )?;

            let has_inn = map
                .facilities
                .iter()
                .any(|f| f.interact_type == "eMapInteractType::Inn");
            let inns: Vec<Inn> = if has_inn {
                query_doc_opt(
                    conn,
                    "inn",
                    -1_i64,
                    "SELECT doc FROM inn WHERE id = ?1",
                    [-1_i64],
                )?
                .into_iter()
                .collect()
            } else {
                Vec::new()
            };

            let shop_facilities: Vec<MapFacility> = map
                .facilities
                .iter()
                .filter(|f| f.interact_type.starts_with("eMapInteractType::Shop"))
                .cloned()
                .collect();

            Ok(Some(MapContents {
                map,
                island,
                region,
                save_point,
                teleport_points,
                treasure_boxes,
                inns,
                shop_facilities,
                shops: Vec::new(),
            }))
        })
    }

    /// Return all data-table derived outgoing map links from `id`.
    pub fn map_links_from(&self, id: i64) -> Result<Vec<MapLink>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT from_map, to_map, kind, via, condition \
                     FROM map_link \
                     WHERE from_map = ?1 \
                     ORDER BY to_map, kind, via, condition",
                )
                .map_err(|err| DbError::sqlite("准备查询 map_link（出边）", err))?;
            let mut rows = stmt
                .query([id])
                .map_err(|err| DbError::sqlite("查询 map_link（出边）", err))?;
            collect_map_links(&mut rows)
        })
    }

    /// Return all data-table derived incoming map links to `id`.
    pub fn map_links_to(&self, id: i64) -> Result<Vec<MapLink>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT from_map, to_map, kind, via, condition \
                     FROM map_link \
                     WHERE to_map = ?1 \
                     ORDER BY from_map, kind, via, condition",
                )
                .map_err(|err| DbError::sqlite("准备查询 map_link（入边）", err))?;
            let mut rows = stmt
                .query([id])
                .map_err(|err| DbError::sqlite("查询 map_link（入边）", err))?;
            collect_map_links(&mut rows)
        })
    }

    /// Look up island metadata by `island_type` (e.g. `"eMapIslandType::Grandshelt"`).
    pub fn island(&self, island_type: &str) -> Result<Option<MapIsland>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "island",
                island_type,
                "SELECT doc FROM island WHERE island_type = ?1",
                [island_type],
            )
        })
    }

    /// Look up region metadata by `DT_MapRegionData.ID`.
    pub fn region(&self, id: i64) -> Result<Option<MapRegion>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "region",
                id,
                "SELECT doc FROM region WHERE id = ?1",
                [id],
            )
        })
    }

    /// Look up the island unlock condition row for `island_type`.
    pub fn map_release_condition(&self, island_type: &str) -> Result<Option<MapReleaseCondition>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "map_release_condition",
                island_type,
                "SELECT doc FROM map_release_condition WHERE island_type = ?1",
                [island_type],
            )
        })
    }

    /// Look up save-point capabilities for a map type (`eMapType::Town` / `eMapType::Dungeon`).
    pub fn save_point(&self, map_type: &str) -> Result<Option<SavePoint>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "save_point",
                map_type,
                "SELECT doc FROM save_point WHERE map_type = ?1",
                [map_type],
            )
        })
    }

    /// Look up a save-point menu command by `ESavePointCommandType::*`.
    pub fn save_point_command(&self, command_type: &str) -> Result<Option<SavePointCommand>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "save_point_command",
                command_type,
                "SELECT doc FROM save_point_command WHERE command_type = ?1",
                [command_type],
            )
        })
    }

    /// Look up a world-map teleport landmark by index (`0..N`).
    pub fn teleport_point(&self, id: usize) -> Result<Option<TeleportPoint>> {
        let Ok(id_i64) = i64::try_from(id) else {
            return Ok(None);
        };
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "teleport_point",
                id_i64,
                "SELECT doc FROM teleport_point WHERE id = ?1",
                [id_i64],
            )
        })
    }

    /// Look up a single treasure box by its composite key `(map_id, id)`.
    pub fn treasure_box(&self, map_id: i64, id: i64) -> Result<Option<TreasureBox>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "treasure_box",
                format!("{map_id}:{id}"),
                "SELECT doc FROM treasure_box WHERE map_id = ?1 AND id = ?2",
                params![map_id, id],
            )
        })
    }

    // ---- Items & acquisition sources ----

    /// Look up an item by its global `ItemId`.
    pub fn item(&self, id: i64) -> Result<Option<Item>> {
        self.with_conn(|conn| {
            query_doc_opt(conn, "item", id, "SELECT doc FROM item WHERE id = ?1", [id])
        })
    }

    /// Search items whose localized name in `lang` contains `query` (ordered by item ID).
    ///
    /// Uses `world_name_fts` (trigram index) when `query` has at least 3 characters and the
    /// covering index on `world_name` for 1-2 character queries. Never falls back to Japanese when
    /// `lang != "ja"`.
    pub fn items_by_name(&self, query: &str, lang: &str) -> Result<Vec<Item>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            if needle.chars().count() >= 3 {
                let fts_query = fts_literal_phrase(needle);
                query_docs(
                    conn,
                    "item",
                    "SELECT i.id, i.doc \
                     FROM world_name_fts \
                     JOIN item i ON i.id = world_name_fts.entity_id \
                     WHERE world_name_fts MATCH ?1 \
                       AND world_name_fts.entity_kind = 'item' \
                       AND world_name_fts.lang = ?2 \
                     ORDER BY i.id",
                    params![fts_query, lang],
                )
            } else {
                let lower = needle.to_lowercase();
                query_docs(
                    conn,
                    "item",
                    "SELECT i.id, i.doc \
                     FROM world_name w \
                     JOIN item i ON i.id = w.entity_id \
                     WHERE w.entity_kind = 'item' \
                       AND w.lang = ?1 \
                       AND instr(w.name_lower, ?2) > 0 \
                     ORDER BY i.id",
                    params![lang, lower],
                )
            }
        })
    }

    /// Return all static sources for `item_id`: treasure boxes, shops, side-quest prizes,
    /// quest-board rewards, and enemy drops / steals.
    pub fn item_sources(&self, item_id: i64) -> Result<Vec<ItemSource>> {
        self.with_conn(|conn| {
            let mut sources = Vec::new();

            // 1. Treasure boxes
            {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT tb.map_id, m.name_zh, tb.id, tb.item_num, tb.is_trap \
                         FROM treasure_box tb \
                         LEFT JOIN map m ON m.id = tb.map_id \
                         WHERE tb.item_id = ?1 \
                         ORDER BY tb.map_id, tb.id",
                    )
                    .map_err(|err| DbError::sqlite("准备查询道具宝箱来源", err))?;
                let mut rows = stmt
                    .query([item_id])
                    .map_err(|err| DbError::sqlite("查询道具宝箱来源", err))?;
                while let Some(row) = rows
                    .next()
                    .map_err(|err| DbError::sqlite("读取道具宝箱来源行", err))?
                {
                    let is_trap_i64: i64 = row
                        .get(4)
                        .map_err(|err| DbError::sqlite("读取 treasure_box.is_trap", err))?;
                    sources.push(ItemSource::TreasureBox {
                        map_id: row
                            .get(0)
                            .map_err(|err| DbError::sqlite("读取 treasure_box.map_id", err))?,
                        map_name_zh: row
                            .get(1)
                            .map_err(|err| DbError::sqlite("读取 map.name_zh", err))?,
                        box_id: row
                            .get(2)
                            .map_err(|err| DbError::sqlite("读取 treasure_box.id", err))?,
                        count: row
                            .get(3)
                            .map_err(|err| DbError::sqlite("读取 treasure_box.item_num", err))?,
                        is_trap: is_trap_i64 != 0,
                    });
                }
            }

            // 2. Shops
            {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT si.shop_id, s.name_zh, si.price, si.price_ratio, \
                                si.max_order_num, si.condition \
                         FROM shop_item si \
                         JOIN shop s ON s.id = si.shop_id \
                         WHERE si.item_id = ?1 \
                         ORDER BY si.shop_id, si.slot_index",
                    )
                    .map_err(|err| DbError::sqlite("准备查询道具商店来源", err))?;
                let mut rows = stmt
                    .query([item_id])
                    .map_err(|err| DbError::sqlite("查询道具商店来源", err))?;
                while let Some(row) = rows
                    .next()
                    .map_err(|err| DbError::sqlite("读取道具商店来源行", err))?
                {
                    sources.push(ItemSource::Shop {
                        shop_id: row
                            .get(0)
                            .map_err(|err| DbError::sqlite("读取 shop_item.shop_id", err))?,
                        shop_name_zh: row
                            .get(1)
                            .map_err(|err| DbError::sqlite("读取 shop.name_zh", err))?,
                        price: row
                            .get(2)
                            .map_err(|err| DbError::sqlite("读取 shop_item.price", err))?,
                        price_ratio: row
                            .get(3)
                            .map_err(|err| DbError::sqlite("读取 shop_item.price_ratio", err))?,
                        max_order_num: row
                            .get(4)
                            .map_err(|err| DbError::sqlite("读取 shop_item.max_order_num", err))?,
                        condition: row
                            .get(5)
                            .map_err(|err| DbError::sqlite("读取 shop_item.condition", err))?,
                    });
                }
            }

            // 3. Side quest prizes
            {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT qp.quest_id, q.name_zh, qp.count \
                         FROM quest_prize qp \
                         JOIN quest q ON q.id = qp.quest_id \
                         WHERE qp.item_id = ?1 \
                         ORDER BY qp.quest_id, qp.prize_index",
                    )
                    .map_err(|err| DbError::sqlite("准备查询道具支线奖励来源", err))?;
                let mut rows = stmt
                    .query([item_id])
                    .map_err(|err| DbError::sqlite("查询道具支线奖励来源", err))?;
                while let Some(row) = rows
                    .next()
                    .map_err(|err| DbError::sqlite("读取道具支线奖励来源行", err))?
                {
                    sources.push(ItemSource::QuestPrize {
                        quest_id: row
                            .get(0)
                            .map_err(|err| DbError::sqlite("读取 quest_prize.quest_id", err))?,
                        quest_name_zh: row
                            .get(1)
                            .map_err(|err| DbError::sqlite("读取 quest.name_zh", err))?,
                        count: row
                            .get(2)
                            .map_err(|err| DbError::sqlite("读取 quest_prize.count", err))?,
                    });
                }
            }

            // 4. Quest-board rewards
            {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT qbr.quest_id, qb.name_zh, qbr.item_num \
                         FROM quest_board_reward qbr \
                         JOIN quest_board qb ON qb.row_key = qbr.quest_board_row_key \
                         WHERE qbr.item_id = ?1 \
                         ORDER BY qbr.quest_id, qbr.reward_index",
                    )
                    .map_err(|err| DbError::sqlite("准备查询道具任务板奖励来源", err))?;
                let mut rows = stmt
                    .query([item_id])
                    .map_err(|err| DbError::sqlite("查询道具任务板奖励来源", err))?;
                while let Some(row) = rows
                    .next()
                    .map_err(|err| DbError::sqlite("读取道具任务板奖励来源行", err))?
                {
                    sources.push(ItemSource::QuestBoardReward {
                        quest_id: row.get(0).map_err(|err| {
                            DbError::sqlite("读取 quest_board_reward.quest_id", err)
                        })?,
                        quest_name_zh: row
                            .get(1)
                            .map_err(|err| DbError::sqlite("读取 quest_board.name_zh", err))?,
                        count: row.get(2).map_err(|err| {
                            DbError::sqlite("读取 quest_board_reward.item_num", err)
                        })?,
                    });
                }
            }

            // 5. Enemy drops & steals
            {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT unit_id, unit_name_zh, drop_kind, rate \
                         FROM item_drop \
                         WHERE item_id = ?1 \
                         ORDER BY unit_id, drop_kind",
                    )
                    .map_err(|err| DbError::sqlite("准备查询道具敌人掉落来源", err))?;
                let mut rows = stmt
                    .query([item_id])
                    .map_err(|err| DbError::sqlite("查询道具敌人掉落来源", err))?;
                while let Some(row) = rows
                    .next()
                    .map_err(|err| DbError::sqlite("读取道具敌人掉落来源行", err))?
                {
                    let kind_raw: String = row
                        .get(2)
                        .map_err(|err| DbError::sqlite("读取 item_drop.drop_kind", err))?;
                    sources.push(ItemSource::EnemyDrop {
                        unit_id: row
                            .get(0)
                            .map_err(|err| DbError::sqlite("读取 item_drop.unit_id", err))?,
                        unit_name_zh: row
                            .get(1)
                            .map_err(|err| DbError::sqlite("读取 item_drop.unit_name_zh", err))?,
                        drop_kind: ItemDropKind::from_db(&kind_raw)?,
                        rate: row
                            .get(3)
                            .map_err(|err| DbError::sqlite("读取 item_drop.rate", err))?,
                    });
                }
            }

            Ok(sources)
        })
    }

    // ---- Shops & inns ----

    /// Look up a shop by `DT_ShopList.ShopID`, resolving its stock entries into [`Item`] objects.
    pub fn shop(&self, id: i64) -> Result<Option<ResolvedShop>> {
        self.with_conn(|conn| {
            let shop_row: Option<(Option<String>, String)> = {
                let mut stmt = conn
                    .prepare_cached("SELECT name_zh, doc FROM shop WHERE id = ?1")
                    .map_err(|err| DbError::sqlite("准备查询 shop 表", err))?;
                stmt.query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()
                    .map_err(|err| DbError::sqlite(format!("查询 shop 主键 {id}"), err))?
            };

            let Some((name_zh, shop_doc)) = shop_row else {
                return Ok(None);
            };
            let shop: Shop = decode_doc("shop", id, &shop_doc)?;

            let mut stock = Vec::with_capacity(shop.items.len());
            let mut stmt = conn
                .prepare_cached(
                    "SELECT si.slot_index, si.item_id, si.price, si.price_ratio, \
                            si.max_order_num, si.condition, i.doc \
                     FROM shop_item si \
                     LEFT JOIN item i ON i.id = si.item_id \
                     WHERE si.shop_id = ?1 \
                     ORDER BY si.slot_index",
                )
                .map_err(|err| DbError::sqlite("准备查询 shop_item 货单", err))?;
            let mut rows = stmt
                .query([id])
                .map_err(|err| DbError::sqlite(format!("查询商店 {id} 货单"), err))?;

            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite(format!("读取商店 {id} 货单行"), err))?
            {
                let slot_i64: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 shop_item.slot_index", err))?;
                let item_id: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 shop_item.item_id", err))?;
                let price: Option<i64> = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 shop_item.price", err))?;
                let price_ratio: f64 = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 shop_item.price_ratio", err))?;
                let max_order_num: Option<i64> = row
                    .get(4)
                    .map_err(|err| DbError::sqlite("读取 shop_item.max_order_num", err))?;
                let condition: String = row
                    .get(5)
                    .map_err(|err| DbError::sqlite("读取 shop_item.condition", err))?;
                let item_doc: Option<String> = row
                    .get(6)
                    .map_err(|err| DbError::sqlite("读取关联 item.doc", err))?;
                let item = item_doc
                    .map(|raw| decode_doc::<Item>("item", item_id, &raw))
                    .transpose()?;

                stock.push(ResolvedShopEntry {
                    slot_index: usize::try_from(slot_i64).unwrap_or(0),
                    entry: ShopItem {
                        item_id,
                        condition,
                        max_order_num,
                        price_ratio,
                    },
                    item,
                    price,
                });
            }

            Ok(Some(ResolvedShop {
                shop,
                name_zh,
                stock,
            }))
        })
    }

    /// Return all shops that sell `item_id`, ordered by shop ID.
    pub fn shops_selling(&self, item_id: i64) -> Result<Vec<Shop>> {
        self.with_conn(|conn| {
            query_docs(
                conn,
                "shop",
                "SELECT DISTINCT s.id, s.doc \
                 FROM shop_item si \
                 JOIN shop s ON s.id = si.shop_id \
                 WHERE si.item_id = ?1 \
                 ORDER BY s.id",
                [item_id],
            )
        })
    }

    /// Look up a multi-shop bundle (`DT_VariousShopsList.ID`).
    pub fn various_shop(&self, id: i64) -> Result<Option<VariousShop>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "various_shop",
                id,
                "SELECT doc FROM various_shop WHERE id = ?1",
                [id],
            )
        })
    }

    /// Return `(slot, Shop)` member records for a multi-shop bundle `id`.
    pub fn various_shop_members(&self, id: i64) -> Result<Vec<(String, Shop)>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT m.slot, s.id, s.doc \
                     FROM various_shop_member m \
                     JOIN shop s ON s.id = m.shop_id \
                     WHERE m.various_shop_id = ?1 \
                     ORDER BY m.slot",
                )
                .map_err(|err| DbError::sqlite("准备查询 various_shop_member", err))?;
            let mut rows = stmt
                .query([id])
                .map_err(|err| DbError::sqlite("查询 various_shop_member", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 various_shop_member 行", err))?
            {
                let slot: String = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 various_shop_member.slot", err))?;
                let shop_id: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 shop.id", err))?;
                let raw_doc: String = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 shop.doc", err))?;
                let shop = decode_doc("shop", shop_id, &raw_doc)?;
                out.push((slot, shop));
            }
            Ok(out)
        })
    }

    /// Look up an inn record by `DT_InnList.ID`.
    pub fn inn(&self, id: i64) -> Result<Option<Inn>> {
        self.with_conn(|conn| {
            query_doc_opt(conn, "inn", id, "SELECT doc FROM inn WHERE id = ?1", [id])
        })
    }

    // ---- Quests, chapters, main scenario, flags, playable units ----

    /// Look up a side quest by `DT_RequestData.ID`.
    pub fn quest(&self, id: i64) -> Result<Option<Quest>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "quest",
                id,
                "SELECT doc FROM quest WHERE id = ?1",
                [id],
            )
        })
    }

    /// Look up a non-placeholder quest-board entry by `CDT_QuestBoardData.QuestId`.
    pub fn quest_board(&self, quest_id: i64) -> Result<Option<QuestBoardEntry>> {
        if quest_id < 0 {
            return Ok(None);
        }
        self.with_conn(|conn| {
            query_keyed_doc_opt(
                conn,
                "quest_board",
                "SELECT row_key, doc FROM quest_board \
                 WHERE quest_id = ?1 AND is_placeholder = 0 \
                 LIMIT 1",
                [quest_id],
            )
        })
    }

    /// Look up a chapter by `DT_ChapterData.ChapterNo`.
    pub fn chapter(&self, chapter_no: i64) -> Result<Option<Chapter>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "chapter",
                chapter_no,
                "SELECT doc FROM chapter WHERE chapter_no = ?1",
                [chapter_no],
            )
        })
    }

    /// Look up a synthesized main quest by `MainQuestId` (`1000 + ChapterNo`).
    pub fn main_quest(&self, quest_id: i64) -> Result<Option<MainQuest>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "main_quest",
                quest_id,
                "SELECT doc FROM main_quest WHERE quest_id = ?1",
                [quest_id],
            )
        })
    }

    /// Return all main-scenario entries belonging to `chapter_no`, sorted by `min_progress`.
    pub fn main_scenario_by_chapter(&self, chapter_no: i64) -> Result<Vec<MainScenarioEntry>> {
        self.with_conn(|conn| {
            query_docs(
                conn,
                "main_scenario",
                "SELECT row_key, doc FROM main_scenario \
                 WHERE chapter_no = ?1 \
                 ORDER BY min_progress ASC",
                [chapter_no],
            )
        })
    }

    /// Resolve the active main-story stage for a save's `{progress:0}` value.
    ///
    /// Matches [`ffr_data::ScenarioData::current_main_scenario`] while querying `main_scenario`
    /// and `chapter` strictly via indexes.
    pub fn main_scenario_for_progress(&self, progress: i64) -> Result<Option<CurrentMainScenario>> {
        let Some(raw_chapter) = chapter_no_from_progress(progress) else {
            return Ok(None);
        };

        self.with_conn(|conn| {
            if raw_chapter == 1 {
                let first_ch1: Option<i64> = {
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT min_progress FROM main_scenario \
                             WHERE chapter_no = 1 AND min_progress IS NOT NULL \
                             ORDER BY min_progress ASC LIMIT 1",
                        )
                        .map_err(|err| DbError::sqlite("准备查询第 1 章起始主线进度", err))?;
                    stmt.query_row([], |row| row.get(0))
                        .optional()
                        .map_err(|err| DbError::sqlite("查询第 1 章起始主线进度", err))?
                };
                if let Some(first_start) = first_ch1
                    && progress < first_start
                {
                    return Ok(Some(CurrentMainScenario {
                        progress,
                        chapter_no: Some(0),
                        is_prologue: true,
                        main_quest_id: None,
                        entry: None,
                        progress_start: Some(1000),
                        next_progress: Some(first_start),
                        destination_map_ids: Vec::new(),
                    }));
                }
            }

            let active_row: Option<(String, Option<i64>, Option<i64>, String)> = {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT row_key, min_progress, progress_end, doc \
                         FROM main_scenario \
                         WHERE chapter_no = ?1 AND min_progress <= ?2 \
                         ORDER BY min_progress DESC LIMIT 1",
                    )
                    .map_err(|err| DbError::sqlite("准备按进度查询 main_scenario", err))?;
                stmt.query_row(params![raw_chapter, progress], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .optional()
                .map_err(|err| DbError::sqlite("按进度查询 main_scenario", err))?
            };

            let (entry, progress_start, next_progress) = match active_row {
                Some((row_key, start, end, raw_doc)) => {
                    let parsed: MainScenarioEntry =
                        decode_doc("main_scenario", &row_key, &raw_doc)?;
                    (Some(parsed), start, end)
                }
                None => (None, None, None),
            };

            let mut destination_map_ids = Vec::new();
            if let Some(active) = &entry {
                for dest in &active.destinations {
                    if !destination_map_ids.contains(&dest.destination_id) {
                        destination_map_ids.push(dest.destination_id);
                    }
                }
            }

            let has_chapter: bool = {
                let mut stmt = conn
                    .prepare_cached("SELECT 1 FROM chapter WHERE chapter_no = ?1")
                    .map_err(|err| DbError::sqlite("准备检查 chapter 是否存在", err))?;
                stmt.query_row([raw_chapter], |row| row.get::<_, i64>(0))
                    .optional()
                    .map_err(|err| DbError::sqlite("检查 chapter 是否存在", err))?
                    .is_some()
            };
            let main_quest_id = has_chapter
                .then(|| main_quest_id_for_chapter(raw_chapter))
                .flatten();

            Ok(Some(CurrentMainScenario {
                progress,
                chapter_no: Some(raw_chapter),
                is_prologue: false,
                main_quest_id,
                entry,
                progress_start,
                next_progress,
                destination_map_ids,
            }))
        })
    }

    /// Look up a named flag condition by row key (e.g. `"PlayerRideChocobo"`).
    pub fn flag_condition(&self, row_key: &str) -> Result<Option<FlagConditionEntry>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "flag_condition",
                row_key,
                "SELECT doc FROM flag_condition WHERE row_key = ?1",
                [row_key],
            )
        })
    }

    /// Return all named flag conditions referencing event flag `flag_id`, ordered by `row_key`.
    pub fn flag_conditions_for_flag(&self, flag_id: i64) -> Result<Vec<FlagConditionEntry>> {
        self.with_conn(|conn| {
            query_docs(
                conn,
                "flag_condition",
                "SELECT DISTINCT fc.row_key, fc.doc \
                 FROM flag_condition_term t \
                 JOIN flag_condition fc ON fc.row_key = t.condition_key \
                 WHERE t.flag_id = ?1 \
                 ORDER BY fc.row_key",
                [flag_id],
            )
        })
    }

    /// Resolve a field player actor `m_MapUnitId` (e.g. `10` or `60`) to its playable unit.
    pub fn playable_by_map_unit_id(&self, id: i64) -> Result<Option<ResolvedMapPlayableUnit>> {
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "playable_unit",
                id,
                "SELECT doc FROM playable_unit WHERE map_unit_id = ?1",
                [id],
            )
        })
    }

    // ---- Catalog & filtered queries for module tools/resources ----

    /// Return all chapters ordered by `chapter_no` ascending.
    pub fn chapters(&self) -> Result<Vec<Chapter>> {
        self.with_conn(|conn| {
            query_docs(
                conn,
                "chapter",
                "SELECT chapter_no, doc FROM chapter WHERE chapter_no >= ?1 ORDER BY chapter_no",
                [i64::MIN],
            )
        })
    }

    /// Return the number of main-scenario rows per chapter using `idx_main_scenario_chapter_progress`.
    pub fn main_scenario_counts_by_chapter(&self) -> Result<BTreeMap<i64, usize>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT chapter_no, COUNT(*) \
                     FROM main_scenario \
                     WHERE chapter_no IS NOT NULL \
                     GROUP BY chapter_no \
                     ORDER BY chapter_no",
                )
                .map_err(|err| DbError::sqlite("准备统计各章主线条数", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("统计各章主线条数", err))?;
            let mut counts = BTreeMap::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取各章主线条数", err))?
            {
                let chapter_no: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 main_scenario.chapter_no", err))?;
                let count: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取主线条数", err))?;
                counts.insert(chapter_no, usize::try_from(count).unwrap_or(0));
            }
            Ok(counts)
        })
    }

    /// Return the full map catalog ordered by `id`, enriched with teleportability and treasure counts.
    pub fn maps_catalog(&self) -> Result<Vec<MapCatalogEntry>> {
        self.with_conn(|conn| {
            let mut teleportable_ids = BTreeSet::new();
            {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT DISTINCT map_id FROM teleport_point_target ORDER BY map_id",
                    )
                    .map_err(|err| DbError::sqlite("准备查询可传送地图编号", err))?;
                let mut rows = stmt
                    .query([])
                    .map_err(|err| DbError::sqlite("查询可传送地图编号", err))?;
                while let Some(row) = rows
                    .next()
                    .map_err(|err| DbError::sqlite("读取可传送地图编号", err))?
                {
                    let map_id: i64 = row
                        .get(0)
                        .map_err(|err| DbError::sqlite("读取 teleport_point_target.map_id", err))?;
                    teleportable_ids.insert(map_id);
                }
            }

            let mut treasure_counts: BTreeMap<i64, usize> = BTreeMap::new();
            {
                let mut stmt = conn
                    .prepare_cached("SELECT map_id, COUNT(*) FROM treasure_box GROUP BY map_id")
                    .map_err(|err| DbError::sqlite("准备统计各地图宝箱数", err))?;
                let mut rows = stmt
                    .query([])
                    .map_err(|err| DbError::sqlite("统计各地图宝箱数", err))?;
                while let Some(row) = rows
                    .next()
                    .map_err(|err| DbError::sqlite("读取各地图宝箱数", err))?
                {
                    let map_id: i64 = row
                        .get(0)
                        .map_err(|err| DbError::sqlite("读取 treasure_box.map_id", err))?;
                    let count: i64 = row
                        .get(1)
                        .map_err(|err| DbError::sqlite("读取宝箱数量", err))?;
                    treasure_counts.insert(map_id, usize::try_from(count).unwrap_or(0));
                }
            }

            let mut stmt = conn
                .prepare_cached("SELECT id, name_zh, doc FROM map WHERE id >= ?1 ORDER BY id")
                .map_err(|err| DbError::sqlite("准备查询地图目录", err))?;
            let mut rows = stmt
                .query([i64::MIN])
                .map_err(|err| DbError::sqlite("查询地图目录", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取地图目录行", err))?
            {
                let id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 map.id", err))?;
                let name_zh: Option<String> = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 map.name_zh", err))?;
                let raw_doc: String = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 map.doc", err))?;
                let map: MapInfo = decode_doc("map", id, &raw_doc)?;
                out.push(MapCatalogEntry {
                    teleportable: teleportable_ids.contains(&id),
                    treasure_count: treasure_counts.get(&id).copied().unwrap_or(0),
                    map,
                    name_zh,
                });
            }
            Ok(out)
        })
    }

    /// Return all side quests derived as belonging to `chapter_no`, ordered by `id`.
    pub fn quests_by_chapter(&self, chapter_no: i64) -> Result<Vec<Quest>> {
        self.with_conn(|conn| {
            query_docs(
                conn,
                "quest",
                "SELECT id, doc FROM quest WHERE chapter_no = ?1 ORDER BY id",
                [chapter_no],
            )
        })
    }

    /// Return all side quests whose `zh-Hans` title contains `query`, ordered by `id`.
    pub fn quests_by_name_zh(&self, query: &str) -> Result<Vec<Quest>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_docs(
                conn,
                "quest",
                "SELECT id, doc FROM quest INDEXED BY idx_quest_name_zh \
                 WHERE name_zh IS NOT NULL AND instr(name_zh, ?1) > 0 \
                 ORDER BY id",
                [needle],
            )
        })
    }

    /// Return all shops whose `zh-Hans` name contains `query`, ordered by `id`.
    pub fn shops_by_name_zh(&self, query: &str) -> Result<Vec<Shop>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_docs(
                conn,
                "shop",
                "SELECT id, doc FROM shop INDEXED BY idx_shop_name_zh \
                 WHERE name_zh IS NOT NULL AND instr(name_zh, ?1) > 0 \
                 ORDER BY id",
                [needle],
            )
        })
    }

    /// Returns `true` if the `map` table contains at least one row.
    pub fn has_world_data(&self) -> Result<bool> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached("SELECT 1 FROM map WHERE id >= ?1 LIMIT 1")
                .map_err(|err| DbError::sqlite("准备检查世界数据是否就绪", err))?;
            Ok(stmt
                .query_row([i64::MIN], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|err| DbError::sqlite("检查世界数据是否就绪", err))?
                .is_some())
        })
    }

    /// Returns `true` if the `chapter` table contains at least one row.
    pub fn has_scenario_data(&self) -> Result<bool> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached("SELECT 1 FROM chapter WHERE chapter_no >= ?1 LIMIT 1")
                .map_err(|err| DbError::sqlite("准备检查章节数据是否就绪", err))?;
            Ok(stmt
                .query_row([i64::MIN], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|err| DbError::sqlite("检查章节数据是否就绪", err))?
                .is_some())
        })
    }

    /// Returns `true` if the `playable_unit` table contains at least one row.
    pub fn has_playable_units(&self) -> Result<bool> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached("SELECT 1 FROM playable_unit WHERE map_unit_id >= ?1 LIMIT 1")
                .map_err(|err| DbError::sqlite("准备检查可操作角色数据是否就绪", err))?;
            Ok(stmt
                .query_row([i64::MIN], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|err| DbError::sqlite("检查可操作角色数据是否就绪", err))?
                .is_some())
        })
    }

    /// Returns `true` if the `flag_condition` table contains at least one row.
    pub fn has_flag_conditions(&self) -> Result<bool> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached("SELECT 1 FROM flag_condition WHERE row_key >= '' LIMIT 1")
                .map_err(|err| DbError::sqlite("准备检查旗标条件数据是否就绪", err))?;
            Ok(stmt
                .query_row([], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|err| DbError::sqlite("检查旗标条件数据是否就绪", err))?
                .is_some())
        })
    }
}

fn fts_literal_phrase(query: &str) -> String {
    let escaped = query.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn collect_map_links(rows: &mut rusqlite::Rows<'_>) -> Result<Vec<MapLink>> {
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 map_link 行", err))?
    {
        out.push(MapLink {
            from_map: row
                .get(0)
                .map_err(|err| DbError::sqlite("读取 map_link.from_map", err))?,
            to_map: row
                .get(1)
                .map_err(|err| DbError::sqlite("读取 map_link.to_map", err))?,
            kind: row
                .get(2)
                .map_err(|err| DbError::sqlite("读取 map_link.kind", err))?,
            via: row
                .get(3)
                .map_err(|err| DbError::sqlite("读取 map_link.via", err))?,
            condition: row
                .get(4)
                .map_err(|err| DbError::sqlite("读取 map_link.condition", err))?,
        });
    }
    Ok(out)
}
