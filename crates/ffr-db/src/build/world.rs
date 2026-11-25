//! World area (`04_world`) import stage.
//!
//! Reuses `ffr_data` typed parsers via `ctx.data_store()` (`WorldData`, `ScenarioData`,
//! `FlagConditionData`, `MapUnitPlayableData`, and unit drops from `DT_UnitParameter`) to populate
//! all `04_world` tables, relationship tables, multilingual name search indexes, and `build_issue`
//! entries for dangling references.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use ffr_data::battle::{UNIT_PARAMETER_TABLE, Unit, parse_units};
use ffr_data::world::{ItemCategory, ItemDetail, MAP_DATA_TABLE, WorldData};
use ffr_data::{
    DataError, DataStore, FTextValue, FlagConditionData, MODEL_LANGUAGE, MapUnitPlayableData,
    NATIVE_LANGUAGE, ResolveOptions, ScenarioData, TextResolver, datatable_relative_path_to_json,
    is_safe_language_tag, localization_dir,
};
use rusqlite::params;

use crate::build::{ImportCtx, StageStats};
use crate::error::{DbError, Result};
use crate::record::encode_doc;

/// Pending row for `world_name` / `world_name_fts`, keyed by natural PK `(entity_kind, entity_id, lang)`.
type WorldNameMap = BTreeMap<(&'static str, i64, String), (String, String)>;

/// Import typed world, item, shop, inn, quest, scenario, flag, playable-unit, and drop tables.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let store = ctx.require_data_store()?;
    let Some(map_table_path) = datatable_relative_path_to_json(MAP_DATA_TABLE, store.export_root())
    else {
        return Ok(StageStats::skipped("无法定位世界地图数据表路径"));
    };
    if !map_table_path.is_file() {
        return Ok(StageStats::skipped(
            "导出目录缺少世界数据表（Map/CDT_MapData_Prod.json）",
        ));
    }

    let world = WorldData::from_store(store)?;
    let scenario = ScenarioData {
        chapters: world.chapters().to_vec(),
        main_scenario: world.main_scenario().to_vec(),
        text_resolver: store.text_resolver().clone(),
    };
    let flags = load_optional_flags(store)?;
    let playable = load_optional_playable(store)?;
    let units = load_optional_units(store)?;

    let languages = discover_languages(store.export_root());
    let resolver = store.text_resolver();

    let map_ids: BTreeSet<i64> = world.maps().iter().map(|m| m.id).collect();
    let island_types: BTreeSet<&str> = world
        .islands()
        .iter()
        .map(|i| i.island_type.as_str())
        .collect();
    let region_ids: BTreeSet<i64> = world.regions().iter().map(|r| r.id).collect();
    let item_prices: BTreeMap<i64, i64> = world
        .items()
        .iter()
        .map(|it| (it.id, it.buying_price))
        .collect();
    let shop_ids: BTreeSet<i64> = world.shops().iter().map(|s| s.id).collect();
    let valid_quest_board_ids: BTreeSet<i64> = world
        .quest_board()
        .iter()
        .filter(|e| !e.is_placeholder())
        .map(|e| e.quest_id)
        .collect();

    let mut total_rows: u64 = 0;
    let mut world_names: WorldNameMap = BTreeMap::new();

    // 1. Maps
    total_rows += import_maps(
        ctx,
        &world,
        resolver,
        &languages,
        &map_ids,
        &island_types,
        &region_ids,
        &mut world_names,
    )?;

    // 2. Islands, regions, release conditions
    total_rows += import_islands_and_regions(ctx, &world, resolver, &island_types)?;

    // 3. Save points & save-point commands
    total_rows += import_save_points(ctx, &world, resolver, &item_prices)?;

    // 4. Teleport points & derived map_link rows
    total_rows += import_teleports_and_links(ctx, &world, &map_ids)?;

    // 5. Items & seed recipe reference checks
    total_rows += import_items(
        ctx,
        &world,
        resolver,
        &languages,
        &item_prices,
        &mut world_names,
    )?;

    // 6. Treasure boxes
    total_rows += import_treasure_boxes(ctx, &world, &map_ids, &item_prices)?;

    // 7. Shops, shop_item, various_shop, various_shop_member, inns
    total_rows += import_shops_and_inns(ctx, &world, resolver, &item_prices, &shop_ids)?;

    // 8. Quests, quest_prize, quest_board, quest_board_reward
    total_rows += import_quests(
        ctx,
        &world,
        resolver,
        &map_ids,
        &item_prices,
        &valid_quest_board_ids,
    )?;

    // 9. Chapters, main_scenario, main_quest
    total_rows += import_scenario(ctx, &scenario, resolver, &map_ids)?;

    // 10. Flag conditions
    if let Some(flag_data) = &flags {
        total_rows += import_flag_conditions(ctx, flag_data)?;
    }

    // 11. Playable map units
    if let Some(playable_data) = &playable {
        total_rows += import_playable_units(ctx, playable_data, resolver)?;
    }

    // 12. Enemy / unit item drops & steals
    if let Some(unit_list) = &units {
        total_rows += import_item_drops(ctx, unit_list, resolver, &item_prices)?;
    }

    // 13. World multilingual name index + FTS5 in natural PK `(entity_kind, entity_id, lang)` order.
    total_rows += flush_world_names(ctx, &world_names)?;

    let loaded_tables = u64::try_from(store.cached_data_table_count()).unwrap_or(0) + 1; // +1 for DA_MapLandmark
    ctx.record_dir_source(store.export_root(), Some(loaded_tables))?;

    let mut notes = Vec::new();
    if flags.is_none() {
        notes.push("缺少 Flag/CDT_FlagCondition.json");
    }
    if playable.is_none() {
        notes.push("缺少 Map/DT_MapUnitPlayableData.json 或 Unit/DT_UnitParameter.json");
    }
    if units.is_none() {
        notes.push("缺少 Unit/DT_UnitParameter.json（未导入敌人掉落）");
    }

    if notes.is_empty() {
        Ok(StageStats::ok(total_rows))
    } else {
        Ok(StageStats::ok_with_note(total_rows, notes.join("；")))
    }
}

#[allow(clippy::too_many_arguments)]
fn import_maps(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    resolver: &TextResolver,
    languages: &[String],
    map_ids: &BTreeSet<i64>,
    island_types: &BTreeSet<&str>,
    region_ids: &BTreeSet<i64>,
    world_names: &mut WorldNameMap,
) -> Result<u64> {
    let mut rows: u64 = 0;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO map (
                id, row_key, group_id, island, region_id, same_map_id, level_name,
                name_ns, name_key, name_zh, map_type, dungeon_type, danger_level,
                encount_group_id, doc
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )
        .map_err(|err| DbError::sqlite("准备写入 map 表", err))?;

    let mut sorted_maps: Vec<_> = world.maps().iter().collect();
    sorted_maps.sort_by_key(|m| m.id);

    for map in sorted_maps {
        if let Some(rid) = map.region_id
            && !region_ids.contains(&rid)
        {
            ctx.record_issue(
                "dangling_region",
                &format!("map:{}", map.id),
                &format!("地图 {} 引用了不存在的区域 region_id={rid}", map.row_key),
            )?;
        }
        if let Some(sid) = map.same_map_id
            && !map_ids.contains(&sid)
        {
            ctx.record_issue(
                "dangling_map",
                &format!("map:{}", map.id),
                &format!("地图 {} 的 same_map_id={sid} 不在 map 表中", map.row_key),
            )?;
        }
        if !is_none_island(&map.island) && !island_types.contains(map.island.as_str()) {
            ctx.record_issue(
                "dangling_island",
                &format!("map:{}", map.id),
                &format!("地图 {} 引用了不存在的岛屿 {}", map.row_key, map.island),
            )?;
        }

        let (name_ns, name_key) = extract_ftext_key(resolver, &map.name);
        let name_zh = resolve_zh_strict(resolver, &map.name);
        let doc = encode_doc("map", map.id, map)?;

        stmt.execute(params![
            map.id,
            map.row_key,
            map.group_id,
            map.island,
            map.region_id,
            map.same_map_id,
            map.level_name,
            name_ns,
            name_key,
            name_zh,
            map.map_type,
            map.dungeon_type,
            map.danger_level,
            map.encount_group_id,
            doc,
        ])
        .map_err(|err| DbError::sqlite(format!("写入 map 主键 {}", map.id), err))?;
        rows += 1;

        collect_entity_names("map", map.id, &map.name, resolver, languages, world_names);
    }

    Ok(rows)
}

fn import_islands_and_regions(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    resolver: &TextResolver,
    island_types: &BTreeSet<&str>,
) -> Result<u64> {
    let mut rows: u64 = 0;

    {
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO island (
                    island_type, row_key, name_ns, name_key, name_zh, display_condition, doc
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|err| DbError::sqlite("准备写入 island 表", err))?;

        let mut sorted_islands: Vec<_> = world.islands().iter().collect();
        sorted_islands.sort_by(|a, b| a.island_type.cmp(&b.island_type));
        for island in sorted_islands {
            let (name_ns, name_key) = extract_ftext_key(resolver, &island.name);
            let name_zh = resolve_zh_strict(resolver, &island.name);
            let doc = encode_doc("island", &island.island_type, island)?;
            stmt.execute(params![
                island.island_type,
                island.row_key,
                name_ns,
                name_key,
                name_zh,
                island.display_condition,
                doc,
            ])
            .map_err(|err| {
                DbError::sqlite(format!("写入 island 主键 {}", island.island_type), err)
            })?;
            rows += 1;
        }
    }

    {
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO region (id, row_key, island_type, doc) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 region 表", err))?;

        let mut sorted_regions: Vec<_> = world.regions().iter().collect();
        sorted_regions.sort_by_key(|r| r.id);
        for region in sorted_regions {
            if !is_none_island(&region.island_type)
                && !island_types.contains(region.island_type.as_str())
            {
                ctx.record_issue(
                    "dangling_island",
                    &format!("region:{}", region.id),
                    &format!(
                        "区域 {} 引用了不存在的岛屿 {}",
                        region.row_key, region.island_type
                    ),
                )?;
            }
            let doc = encode_doc("region", region.id, region)?;
            stmt.execute(params![region.id, region.row_key, region.island_type, doc])
                .map_err(|err| DbError::sqlite(format!("写入 region 主键 {}", region.id), err))?;
            rows += 1;
        }
    }

    {
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO map_release_condition (row_key, island_type, condition, doc) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 map_release_condition 表", err))?;

        let mut sorted_rels: Vec<_> = world.release_conditions().iter().collect();
        sorted_rels.sort_by(|a, b| a.island_type.cmp(&b.island_type));
        for rel in sorted_rels {
            if !is_none_island(&rel.island_type) && !island_types.contains(rel.island_type.as_str())
            {
                ctx.record_issue(
                    "dangling_island",
                    &format!("map_release_condition:{}", rel.row_key),
                    &format!("岛屿解锁条件引用了不存在的岛屿 {}", rel.island_type),
                )?;
            }
            let doc = encode_doc("map_release_condition", &rel.island_type, rel)?;
            stmt.execute(params![rel.row_key, rel.island_type, rel.condition, doc])
                .map_err(|err| {
                    DbError::sqlite(
                        format!("写入 map_release_condition 主键 {}", rel.island_type),
                        err,
                    )
                })?;
            rows += 1;
        }
    }

    Ok(rows)
}

fn import_save_points(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    resolver: &TextResolver,
    item_prices: &BTreeMap<i64, i64>,
) -> Result<u64> {
    let mut rows: u64 = 0;

    {
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO save_point (
                    map_type, row_key, can_save, can_use_tent, can_teleport, doc
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 save_point 表", err))?;

        let mut sorted_sps: Vec<_> = world.save_points().iter().collect();
        sorted_sps.sort_by(|a, b| a.map_type.cmp(&b.map_type));
        for sp in sorted_sps {
            let doc = encode_doc("save_point", &sp.map_type, sp)?;
            stmt.execute(params![
                sp.map_type,
                sp.row_key,
                i64::from(sp.can_save),
                i64::from(sp.can_use_tent),
                i64::from(sp.can_teleport),
                doc,
            ])
            .map_err(|err| DbError::sqlite(format!("写入 save_point 主键 {}", sp.map_type), err))?;
            rows += 1;
        }
    }

    {
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO save_point_command (
                    command_type, row_key, label_ns, label_key, label_zh, item_id, is_enable, doc
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|err| DbError::sqlite("准备写入 save_point_command 表", err))?;

        let mut sorted_cmds: Vec<_> = world.save_point_commands().iter().collect();
        sorted_cmds.sort_by(|a, b| a.command_type.cmp(&b.command_type));
        for cmd in sorted_cmds {
            if let Some(iid) = cmd.item_id
                && !item_prices.contains_key(&iid)
            {
                ctx.record_issue(
                    "dangling_item",
                    &format!("save_point_command:{}", cmd.command_type),
                    &format!("存档点指令引用了不存在的道具 item_id={iid}"),
                )?;
            }
            let (label_ns, label_key) = extract_ftext_key(resolver, &cmd.label);
            let label_zh = resolve_zh_strict(resolver, &cmd.label);
            let doc = encode_doc("save_point_command", &cmd.command_type, cmd)?;
            stmt.execute(params![
                cmd.command_type,
                cmd.row_key,
                label_ns,
                label_key,
                label_zh,
                cmd.item_id,
                i64::from(cmd.is_enable),
                doc,
            ])
            .map_err(|err| {
                DbError::sqlite(
                    format!("写入 save_point_command 主键 {}", cmd.command_type),
                    err,
                )
            })?;
            rows += 1;
        }
    }

    Ok(rows)
}

fn import_teleports_and_links(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    map_ids: &BTreeSet<i64>,
) -> Result<u64> {
    let mut rows: u64 = 0;

    let mut field_map_ids: Vec<i64> = world
        .maps()
        .iter()
        .filter(|m| m.map_type == "eMapType::Field")
        .map(|m| m.id)
        .collect();
    field_map_ids.sort_unstable();

    let mut tp_stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO teleport_point (
                id, airship_landing_point_id, loc_x, loc_y, loc_z, doc
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|err| DbError::sqlite("准备写入 teleport_point 表", err))?;

    let mut target_stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO teleport_point_target (teleport_id, target_index, map_id, condition) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|err| DbError::sqlite("准备写入 teleport_point_target 表", err))?;

    let mut link_stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT OR IGNORE INTO map_link (from_map, to_map, kind, via, condition) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|err| DbError::sqlite("准备写入 map_link 表", err))?;

    let mut map_links: BTreeSet<(i64, i64, &'static str, String, String)> = BTreeSet::new();

    let mut sorted_tps: Vec<_> = world.teleport_points().iter().collect();
    sorted_tps.sort_by_key(|tp| tp.index);
    for tp in sorted_tps {
        let tp_id = i64::try_from(tp.index).unwrap_or(i64::MAX);
        let doc = encode_doc("teleport_point", tp_id, tp)?;
        tp_stmt
            .execute(params![
                tp_id,
                tp.airship_landing_point_id,
                tp.location.x,
                tp.location.y,
                tp.location.z,
                doc,
            ])
            .map_err(|err| DbError::sqlite(format!("写入 teleport_point 主键 {tp_id}"), err))?;
        rows += 1;

        let via = format!("teleport_point:{tp_id}");
        for (target_idx, target) in tp.targets.iter().enumerate() {
            let Some(target_map_id) = target.map_id else {
                continue;
            };
            if !map_ids.contains(&target_map_id) {
                ctx.record_issue(
                    "dangling_map",
                    &format!("teleport_point:{tp_id}"),
                    &format!("传送地标 #{tp_id} 指向不存在的地图 map_id={target_map_id}"),
                )?;
            }
            let t_idx = i64::try_from(target_idx).unwrap_or(i64::MAX);
            target_stmt
                .execute(params![tp_id, t_idx, target_map_id, target.condition])
                .map_err(|err| {
                    DbError::sqlite(
                        format!("写入 teleport_point_target ({tp_id}, {t_idx})"),
                        err,
                    )
                })?;
            rows += 1;

            for &field_id in &field_map_ids {
                if field_id == target_map_id {
                    continue;
                }
                map_links.insert((
                    field_id,
                    target_map_id,
                    "teleport_landmark",
                    via.clone(),
                    target.condition.clone(),
                ));
                map_links.insert((
                    target_map_id,
                    field_id,
                    "world_map_exit",
                    via.clone(),
                    target.condition.clone(),
                ));
            }
        }
    }

    // Derive `same_map` links declared directly in `CDT_MapData_Prod.sameMapId`.
    for map in world.maps() {
        let Some(same_id) = map.same_map_id else {
            continue;
        };
        if same_id == map.id {
            continue;
        }
        map_links.insert((
            map.id,
            same_id,
            "same_map",
            "map.same_map_id".to_owned(),
            String::new(),
        ));
        if map_ids.contains(&same_id) {
            map_links.insert((
                same_id,
                map.id,
                "same_map",
                "map.same_map_id".to_owned(),
                String::new(),
            ));
        }
    }

    for (from_map, to_map, kind, via, condition) in map_links {
        let inserted = link_stmt
            .execute(params![from_map, to_map, kind, via, condition])
            .map_err(|err| DbError::sqlite("写入 map_link", err))?;
        rows += u64::try_from(inserted).unwrap_or(0);
    }

    Ok(rows)
}

fn import_items(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    resolver: &TextResolver,
    languages: &[String],
    item_prices: &BTreeMap<i64, i64>,
    world_names: &mut WorldNameMap,
) -> Result<u64> {
    let mut rows: u64 = 0;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO item (
                id, row_key, sort_id, category, raw_item_type,
                name_ns, name_key, name_zh, buying_price, selling_price,
                is_available_sale, doc
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(|err| DbError::sqlite("准备写入 item 表", err))?;

    let mut sorted_items: Vec<_> = world.items().iter().collect();
    sorted_items.sort_by_key(|it| it.id);

    for item in sorted_items {
        if let ItemDetail::Seed { harvest_recipes } = &item.detail {
            let mut reported: BTreeSet<i64> = BTreeSet::new();
            for recipe in harvest_recipes {
                for (field, target_id) in [
                    ("combine_item_id", recipe.combine_item_id),
                    ("create_item_id", recipe.create_item_id),
                ] {
                    if target_id >= 0
                        && !item_prices.contains_key(&target_id)
                        && reported.insert(target_id)
                    {
                        ctx.record_issue(
                            "dangling_item",
                            &format!("seed:{}", item.id),
                            &format!(
                                "种子道具 {} 的配方字段 {field}={target_id} 不在 item 表中",
                                item.id
                            ),
                        )?;
                    }
                }
            }
        }

        let (name_ns, name_key) = extract_ftext_key(resolver, &item.name);
        let name_zh = resolve_zh_strict(resolver, &item.name);
        let doc = encode_doc("item", item.id, item)?;
        stmt.execute(params![
            item.id,
            item.row_key,
            item.sort_id,
            item_category_str(item.category),
            item.raw_item_type,
            name_ns,
            name_key,
            name_zh,
            item.buying_price,
            item.selling_price,
            i64::from(item.is_available_sale),
            doc,
        ])
        .map_err(|err| DbError::sqlite(format!("写入 item 主键 {}", item.id), err))?;
        rows += 1;

        collect_entity_names(
            "item",
            item.id,
            &item.name,
            resolver,
            languages,
            world_names,
        );
    }

    Ok(rows)
}

fn import_treasure_boxes(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    map_ids: &BTreeSet<i64>,
    item_prices: &BTreeMap<i64, i64>,
) -> Result<u64> {
    let mut rows: u64 = 0;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO treasure_box (
                map_id, id, row_key, item_id, item_num, money, is_trap, battle_group_id, doc
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(|err| DbError::sqlite("准备写入 treasure_box 表", err))?;

    let mut sorted_chests: Vec<_> = world.treasure_boxes().iter().collect();
    sorted_chests.sort_by_key(|c| (c.map_id, c.id));

    for chest in sorted_chests {
        if !map_ids.contains(&chest.map_id) {
            ctx.record_issue(
                "dangling_map",
                &format!("treasure_box:{}", chest.row_key),
                &format!(
                    "宝箱 ({}, {}) 引用了不存在的地图 map_id={}",
                    chest.map_id, chest.id, chest.map_id
                ),
            )?;
        }
        if let Some(iid) = chest.item_id
            && !item_prices.contains_key(&iid)
        {
            ctx.record_issue(
                "dangling_item",
                &format!("treasure_box:{}", chest.row_key),
                &format!(
                    "宝箱 ({}, {}) 引用了不存在的道具 item_id={iid}",
                    chest.map_id, chest.id
                ),
            )?;
        }
        let doc_key = format!("{}:{}", chest.map_id, chest.id);
        let doc = encode_doc("treasure_box", &doc_key, chest)?;
        stmt.execute(params![
            chest.map_id,
            chest.id,
            chest.row_key,
            chest.item_id,
            chest.item_num,
            chest.money,
            i64::from(chest.is_trap),
            chest.battle_group_id,
            doc,
        ])
        .map_err(|err| DbError::sqlite(format!("写入 treasure_box 主键 {doc_key}"), err))?;
        rows += 1;
    }

    Ok(rows)
}

fn import_shops_and_inns(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    resolver: &TextResolver,
    item_prices: &BTreeMap<i64, i64>,
    shop_ids: &BTreeSet<i64>,
) -> Result<u64> {
    let mut rows: u64 = 0;

    {
        let mut shop_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO shop (id, row_key, name_ns, name_key, name_zh, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 shop 表", err))?;

        let mut shop_item_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO shop_item (
                    shop_id, slot_index, item_id, price, price_ratio, max_order_num, condition
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|err| DbError::sqlite("准备写入 shop_item 表", err))?;

        let mut sorted_shops: Vec<_> = world.shops().iter().collect();
        sorted_shops.sort_by_key(|s| s.id);
        for shop in sorted_shops {
            let (name_ns, name_key) = extract_ftext_key(resolver, &shop.name);
            let name_zh = resolve_zh_strict(resolver, &shop.name);
            let doc = encode_doc("shop", shop.id, shop)?;
            shop_stmt
                .execute(params![
                    shop.id,
                    shop.row_key,
                    name_ns,
                    name_key,
                    name_zh,
                    doc,
                ])
                .map_err(|err| DbError::sqlite(format!("写入 shop 主键 {}", shop.id), err))?;
            rows += 1;

            for (slot_idx, entry) in shop.items.iter().enumerate() {
                let effective_price = match item_prices.get(&entry.item_id) {
                    Some(&base_price) => Some(entry.effective_price(base_price)),
                    None => {
                        ctx.record_issue(
                            "dangling_item",
                            &format!("shop:{}", shop.id),
                            &format!(
                                "商店 {} ({}) 货单槽位 {} 引用了不存在的道具 item_id={}",
                                shop.id, shop.row_key, slot_idx, entry.item_id
                            ),
                        )?;
                        None
                    }
                };
                let slot_i64 = i64::try_from(slot_idx).unwrap_or(i64::MAX);
                shop_item_stmt
                    .execute(params![
                        shop.id,
                        slot_i64,
                        entry.item_id,
                        effective_price,
                        entry.price_ratio,
                        entry.max_order_num,
                        entry.condition,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 shop_item ({}, {slot_i64})", shop.id), err)
                    })?;
                rows += 1;
            }

            for sell_entry in &shop.sell_items {
                if !item_prices.contains_key(&sell_entry.item_id) {
                    ctx.record_issue(
                        "dangling_item",
                        &format!("shop:{}", shop.id),
                        &format!(
                            "商店 {} 收购表引用了不存在的道具 item_id={}",
                            shop.id, sell_entry.item_id
                        ),
                    )?;
                }
            }
        }
    }

    {
        let mut vs_stmt = ctx
            .tx()
            .prepare_cached("INSERT INTO various_shop (id, row_key, doc) VALUES (?1, ?2, ?3)")
            .map_err(|err| DbError::sqlite("准备写入 various_shop 表", err))?;

        let mut member_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO various_shop_member (various_shop_id, slot, shop_id) \
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|err| DbError::sqlite("准备写入 various_shop_member 表", err))?;

        let mut sorted_vs: Vec<_> = world.various_shops().iter().collect();
        sorted_vs.sort_by_key(|v| v.id);
        for vs in sorted_vs {
            let doc = encode_doc("various_shop", vs.id, vs)?;
            vs_stmt
                .execute(params![vs.id, vs.row_key, doc])
                .map_err(|err| DbError::sqlite(format!("写入 various_shop 主键 {}", vs.id), err))?;
            rows += 1;

            // Alphabetical slot order matches natural PRIMARY KEY `(various_shop_id, slot)`.
            for (slot, maybe_shop_id) in [
                ("accessory", vs.accessory_shop_id),
                ("armor", vs.armor_shop_id),
                ("tool", vs.tool_shop_id),
                ("weapon", vs.weapon_shop_id),
            ] {
                let Some(sub_shop_id) = maybe_shop_id else {
                    continue;
                };
                if !shop_ids.contains(&sub_shop_id) {
                    ctx.record_issue(
                        "dangling_shop",
                        &format!("various_shop:{}", vs.id),
                        &format!(
                            "综合商店 {} 槽位 {slot} 引用了不存在的商店 shop_id={sub_shop_id}",
                            vs.id
                        ),
                    )?;
                }
                member_stmt
                    .execute(params![vs.id, slot, sub_shop_id])
                    .map_err(|err| {
                        DbError::sqlite(
                            format!("写入 various_shop_member ({}, {slot})", vs.id),
                            err,
                        )
                    })?;
                rows += 1;
            }
        }
    }

    {
        let mut inn_stmt = ctx
            .tx()
            .prepare_cached("INSERT INTO inn (id, row_key, price, doc) VALUES (?1, ?2, ?3, ?4)")
            .map_err(|err| DbError::sqlite("准备写入 inn 表", err))?;

        let mut sorted_inns: Vec<_> = world.inns().iter().collect();
        sorted_inns.sort_by_key(|inn| inn.id);
        for inn in sorted_inns {
            let doc = encode_doc("inn", inn.id, inn)?;
            inn_stmt
                .execute(params![inn.id, inn.row_key, inn.price, doc])
                .map_err(|err| DbError::sqlite(format!("写入 inn 主键 {}", inn.id), err))?;
            rows += 1;
        }
    }

    Ok(rows)
}

fn import_quests(
    ctx: &ImportCtx<'_>,
    world: &WorldData,
    resolver: &TextResolver,
    map_ids: &BTreeSet<i64>,
    item_prices: &BTreeMap<i64, i64>,
    valid_quest_board_ids: &BTreeSet<i64>,
) -> Result<u64> {
    let mut rows: u64 = 0;

    {
        let mut quest_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO quest (
                    id, row_key, title_ns, title_key, name_zh, category,
                    chapter_no, min_main_progress, requester_map_id, report_map_id, doc
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .map_err(|err| DbError::sqlite("准备写入 quest 表", err))?;

        let mut prize_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO quest_prize (
                    quest_id, prize_index, item_id, count, text_ns, text_key, text_zh
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|err| DbError::sqlite("准备写入 quest_prize 表", err))?;

        let mut sorted_quests: Vec<_> = world.quests().iter().collect();
        sorted_quests.sort_by_key(|q| q.id);
        for quest in sorted_quests {
            if let Some(mid) = quest.requester_map_id
                && !map_ids.contains(&mid)
            {
                ctx.record_issue(
                    "dangling_map",
                    &format!("quest:{}", quest.id),
                    &format!("支线任务 {} 的委托人地图 map_id={mid} 不存在", quest.id),
                )?;
            }
            if let Some(mid) = quest.report_map_id
                && !map_ids.contains(&mid)
            {
                ctx.record_issue(
                    "dangling_map",
                    &format!("quest:{}", quest.id),
                    &format!("支线任务 {} 的回报地图 map_id={mid} 不存在", quest.id),
                )?;
            }
            for obj in &quest.objectives {
                for dest in &obj.destinations {
                    if dest.destination_id >= 0 && !map_ids.contains(&dest.destination_id) {
                        ctx.record_issue(
                            "dangling_map",
                            &format!("quest:{}", quest.id),
                            &format!(
                                "支线任务 {} 的阶段目标引用了不存在的地图 map_id={}",
                                quest.id, dest.destination_id
                            ),
                        )?;
                    }
                }
                for &qb_id in &obj.quest_board_ids {
                    if !valid_quest_board_ids.contains(&qb_id) {
                        ctx.record_issue(
                            "dangling_quest_board",
                            &format!("quest:{}", quest.id),
                            &format!(
                                "支线任务 {} 引用了不存在的任务板条目 quest_board_id={qb_id}",
                                quest.id
                            ),
                        )?;
                    }
                }
            }

            let (title_ns, title_key) = extract_ftext_key(resolver, &quest.title);
            let name_zh = resolve_zh_strict(resolver, &quest.title);
            let doc = encode_doc("quest", quest.id, quest)?;
            quest_stmt
                .execute(params![
                    quest.id,
                    quest.row_key,
                    title_ns,
                    title_key,
                    name_zh,
                    quest.category,
                    quest.chapter_no,
                    quest.min_main_progress,
                    quest.requester_map_id,
                    quest.report_map_id,
                    doc,
                ])
                .map_err(|err| DbError::sqlite(format!("写入 quest 主键 {}", quest.id), err))?;
            rows += 1;

            for (prize_idx, prize) in quest.prizes.iter().enumerate() {
                if let Some(iid) = prize.item_id
                    && !item_prices.contains_key(&iid)
                {
                    ctx.record_issue(
                        "dangling_item",
                        &format!("quest:{}", quest.id),
                        &format!("支线任务 {} 奖励引用了不存在的道具 item_id={iid}", quest.id),
                    )?;
                }
                let (text_ns, text_key) = extract_ftext_key(resolver, &prize.text);
                let text_zh = resolve_zh_strict(resolver, &prize.text);
                let p_idx = i64::try_from(prize_idx).unwrap_or(i64::MAX);
                prize_stmt
                    .execute(params![
                        quest.id,
                        p_idx,
                        prize.item_id,
                        prize.count,
                        text_ns,
                        text_key,
                        text_zh,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 quest_prize ({}, {p_idx})", quest.id), err)
                    })?;
                rows += 1;
            }
        }
    }

    {
        let mut qb_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO quest_board (
                    row_key, quest_id, quest_board_id, quest_type, is_placeholder,
                    title_ns, title_key, name_zh, doc
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(|err| DbError::sqlite("准备写入 quest_board 表", err))?;

        let mut reward_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO quest_board_reward (
                    quest_board_row_key, reward_index, quest_id, item_id, item_num, money
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 quest_board_reward 表", err))?;

        let mut sorted_qbs: Vec<_> = world.quest_board().iter().collect();
        sorted_qbs.sort_by(|a, b| a.row_key.cmp(&b.row_key));
        for entry in sorted_qbs {
            let (title_ns, title_key) = extract_ftext_key(resolver, &entry.title);
            let name_zh = resolve_zh_strict(resolver, &entry.title);
            let doc = encode_doc("quest_board", &entry.row_key, entry)?;
            qb_stmt
                .execute(params![
                    entry.row_key,
                    entry.quest_id,
                    entry.quest_board_id,
                    entry.quest_type,
                    i64::from(entry.is_placeholder()),
                    title_ns,
                    title_key,
                    name_zh,
                    doc,
                ])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 quest_board 主键 {}", entry.row_key), err)
                })?;
            rows += 1;

            for (r_idx, reward) in entry.rewards.iter().enumerate() {
                if let Some(iid) = reward.item_id
                    && !item_prices.contains_key(&iid)
                {
                    ctx.record_issue(
                        "dangling_item",
                        &format!("quest_board:{}", entry.row_key),
                        &format!(
                            "任务板条目 {} 奖励引用了不存在的道具 item_id={iid}",
                            entry.row_key
                        ),
                    )?;
                }
                let r_i64 = i64::try_from(r_idx).unwrap_or(i64::MAX);
                reward_stmt
                    .execute(params![
                        entry.row_key,
                        r_i64,
                        entry.quest_id,
                        reward.item_id,
                        reward.item_num,
                        reward.money,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(
                            format!("写入 quest_board_reward ({}, {r_i64})", entry.row_key),
                            err,
                        )
                    })?;
                rows += 1;
            }
        }
    }

    Ok(rows)
}

fn import_scenario(
    ctx: &ImportCtx<'_>,
    scenario: &ScenarioData,
    resolver: &TextResolver,
    map_ids: &BTreeSet<i64>,
) -> Result<u64> {
    let mut rows: u64 = 0;

    {
        let mut ch_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO chapter (chapter_no, row_key, name_ns, name_key, name_zh, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 chapter 表", err))?;

        let mut sorted_chapters: Vec<_> = scenario.chapters.iter().collect();
        sorted_chapters.sort_by_key(|c| c.chapter_no);
        for ch in sorted_chapters {
            let (name_ns, name_key) = extract_ftext_key(resolver, &ch.name);
            let name_zh = resolve_zh_strict(resolver, &ch.name);
            let doc = encode_doc("chapter", ch.chapter_no, ch)?;
            ch_stmt
                .execute(params![
                    ch.chapter_no,
                    ch.row_key,
                    name_ns,
                    name_key,
                    name_zh,
                    doc,
                ])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 chapter 主键 {}", ch.chapter_no), err)
                })?;
            rows += 1;
        }
    }

    // Compute exclusive upper progress bound `progress_end` for each entry with `min_progress`.
    let distinct_starts: BTreeSet<i64> = scenario
        .main_scenario
        .iter()
        .filter_map(|e| e.min_progress)
        .collect();
    let sorted_starts: Vec<i64> = distinct_starts.into_iter().collect();
    let mut next_progress_map: BTreeMap<i64, i64> = BTreeMap::new();
    for pair in sorted_starts.windows(2) {
        next_progress_map.insert(pair[0], pair[1]);
    }

    {
        let mut ms_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO main_scenario (
                    row_key, chapter_no, min_progress, progress_end, condition,
                    text_ns, text_key, text_zh, doc
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(|err| DbError::sqlite("准备写入 main_scenario 表", err))?;

        let mut sorted_ms: Vec<_> = scenario.main_scenario.iter().collect();
        sorted_ms.sort_by(|a, b| a.row_key.cmp(&b.row_key));
        for entry in sorted_ms {
            for dest in &entry.destinations {
                if dest.destination_id >= 0 && !map_ids.contains(&dest.destination_id) {
                    ctx.record_issue(
                        "dangling_map",
                        &format!("main_scenario:{}", entry.row_key),
                        &format!(
                            "主线阶段 {} 导航目的地引用了不存在的地图 map_id={}",
                            entry.row_key, dest.destination_id
                        ),
                    )?;
                }
            }
            let progress_end = entry
                .min_progress
                .and_then(|start| next_progress_map.get(&start).copied());
            let (text_ns, text_key) = extract_ftext_key(resolver, &entry.text);
            let text_zh = resolve_zh_strict(resolver, &entry.text);
            let doc = encode_doc("main_scenario", &entry.row_key, entry)?;
            ms_stmt
                .execute(params![
                    entry.row_key,
                    entry.chapter_no,
                    entry.min_progress,
                    progress_end,
                    entry.condition,
                    text_ns,
                    text_key,
                    text_zh,
                    doc,
                ])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 main_scenario 主键 {}", entry.row_key), err)
                })?;
            rows += 1;
        }
    }

    {
        let mut mq_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO main_quest (
                    quest_id, chapter_no, chapter_row_key, name_ns, name_key, name_zh, doc
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|err| DbError::sqlite("准备写入 main_quest 表", err))?;

        let mut main_quests = scenario.main_quests();
        main_quests.sort_by_key(|mq| mq.quest_id);
        for mq in main_quests {
            let (name_ns, name_key) = extract_ftext_key(resolver, &mq.chapter_name);
            let name_zh = resolve_zh_strict(resolver, &mq.chapter_name);
            let doc = encode_doc("main_quest", mq.quest_id, &mq)?;
            mq_stmt
                .execute(params![
                    mq.quest_id,
                    mq.chapter_no,
                    mq.chapter_row_key,
                    name_ns,
                    name_key,
                    name_zh,
                    doc,
                ])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 main_quest 主键 {}", mq.quest_id), err)
                })?;
            rows += 1;
        }
    }

    Ok(rows)
}

fn import_flag_conditions(ctx: &ImportCtx<'_>, flags: &FlagConditionData) -> Result<u64> {
    let mut rows: u64 = 0;

    let mut fc_stmt = ctx
        .tx()
        .prepare_cached("INSERT INTO flag_condition (row_key, condition, doc) VALUES (?1, ?2, ?3)")
        .map_err(|err| DbError::sqlite("准备写入 flag_condition 表", err))?;

    let mut term_stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO flag_condition_term (condition_key, term_index, flag_id, expected) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|err| DbError::sqlite("准备写入 flag_condition_term 表", err))?;

    let mut sorted_flags: Vec<_> = flags.entries().iter().collect();
    sorted_flags.sort_by(|a, b| a.row_key.cmp(&b.row_key));
    for entry in sorted_flags {
        let doc = encode_doc("flag_condition", &entry.row_key, entry)?;
        fc_stmt
            .execute(params![entry.row_key, entry.condition, doc])
            .map_err(|err| {
                DbError::sqlite(format!("写入 flag_condition 主键 {}", entry.row_key), err)
            })?;
        rows += 1;

        for (t_idx, term) in entry.terms.iter().enumerate() {
            let idx_i64 = i64::try_from(t_idx).unwrap_or(i64::MAX);
            term_stmt
                .execute(params![
                    entry.row_key,
                    idx_i64,
                    term.flag_id,
                    i64::from(term.expected),
                ])
                .map_err(|err| {
                    DbError::sqlite(
                        format!("写入 flag_condition_term ({}, {idx_i64})", entry.row_key),
                        err,
                    )
                })?;
            rows += 1;
        }
    }

    Ok(rows)
}

fn import_playable_units(
    ctx: &ImportCtx<'_>,
    playable: &MapUnitPlayableData,
    resolver: &TextResolver,
) -> Result<u64> {
    let mut rows: u64 = 0;

    // Report any `DT_MapUnitPlayableData` row whose `map_unit_id` failed to match a playable unit.
    let mut sorted_raw: Vec<_> = playable.entries().iter().collect();
    sorted_raw.sort_by(|a, b| {
        a.map_unit_id
            .cmp(&b.map_unit_id)
            .then_with(|| a.row_key.cmp(&b.row_key))
    });
    let mut reported_missing: BTreeSet<i64> = BTreeSet::new();
    for raw in sorted_raw {
        if playable.resolve_by_map_unit_id(raw.map_unit_id).is_none()
            && reported_missing.insert(raw.map_unit_id)
        {
            ctx.record_issue(
                "unresolved_playable_unit",
                &format!("playable_unit:{}", raw.map_unit_id),
                &format!(
                    "地图可操作单位 {} (map_unit_id={}) 未能在 DT_UnitParameter 中匹配到可操作角色",
                    raw.row_key, raw.map_unit_id
                ),
            )?;
        }
    }

    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO playable_unit (
                map_unit_id, unit_id, row_key, name_ns, name_key, name_zh,
                is_idle_by_four_direction, doc
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(|err| DbError::sqlite("准备写入 playable_unit 表", err))?;

    let mut sorted_mappings = playable.all_mappings();
    sorted_mappings.sort_by_key(|u| u.map_unit_id);
    for unit in sorted_mappings {
        let (name_ns, name_key) = extract_ftext_key(resolver, &unit.name);
        let name_zh = resolve_zh_strict(resolver, &unit.name);
        let doc = encode_doc("playable_unit", unit.map_unit_id, &unit)?;
        let idle_int = unit.is_idle_by_four_direction.map(i64::from);
        stmt.execute(params![
            unit.map_unit_id,
            unit.unit_id,
            unit.row_key,
            name_ns,
            name_key,
            name_zh,
            idle_int,
            doc,
        ])
        .map_err(|err| {
            DbError::sqlite(format!("写入 playable_unit 主键 {}", unit.map_unit_id), err)
        })?;
        rows += 1;
    }

    Ok(rows)
}

fn import_item_drops(
    ctx: &ImportCtx<'_>,
    units: &[Unit],
    resolver: &TextResolver,
    item_prices: &BTreeMap<i64, i64>,
) -> Result<u64> {
    let mut rows: u64 = 0;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO item_drop (
                unit_id, drop_kind, item_id, rate, unit_name_ns, unit_name_key, unit_name_zh
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|err| DbError::sqlite("准备写入 item_drop 表", err))?;

    let mut sorted_units: Vec<_> = units.iter().collect();
    sorted_units.sort_by_key(|u| u.id);
    for unit in sorted_units {
        // Alphabetical `drop_kind` order matches natural PRIMARY KEY `(unit_id, drop_kind)`.
        let candidates = [
            (
                "drop_normal",
                unit.drop_normal_item,
                unit.drop_normal_item_rate,
            ),
            ("drop_rare", unit.drop_rare_item, unit.drop_rare_item_rate),
            ("steal_normal", unit.steal_normal_item, unit.steal_item_rate),
            ("steal_rare", unit.steal_rare_item, unit.steal_item_rate),
        ];

        struct ResolvedUnitName {
            ns: Option<String>,
            key: Option<String>,
            zh: Option<String>,
        }
        let mut resolved_name: Option<ResolvedUnitName> = None;

        for (drop_kind, item_id, rate) in candidates {
            if item_id < 0 {
                continue;
            }
            if !item_prices.contains_key(&item_id) {
                ctx.record_issue(
                    "dangling_item",
                    &format!("unit_drop:{}:{drop_kind}", unit.id),
                    &format!(
                        "单位 {} ({}) 的 {drop_kind} 引用了不存在的道具 item_id={item_id}",
                        unit.id, unit.row_key
                    ),
                )?;
            }

            let unit_name = resolved_name.get_or_insert_with(|| {
                let (ns, key) = extract_ftext_key(resolver, &unit.name);
                let zh = resolve_zh_strict(resolver, &unit.name);
                ResolvedUnitName { ns, key, zh }
            });

            stmt.execute(params![
                unit.id,
                drop_kind,
                item_id,
                rate,
                unit_name.ns.as_deref(),
                unit_name.key.as_deref(),
                unit_name.zh.as_deref(),
            ])
            .map_err(|err| {
                DbError::sqlite(format!("写入 item_drop ({}, {drop_kind})", unit.id), err)
            })?;
            rows += 1;
        }
    }

    Ok(rows)
}

fn collect_entity_names(
    entity_kind: &'static str,
    entity_id: i64,
    ftext: &FTextValue,
    resolver: &TextResolver,
    languages: &[String],
    world_names: &mut WorldNameMap,
) {
    for lang in languages {
        let resolved = resolver.resolve_with(ftext, Some(lang), ResolveOptions::strict());
        let Some(text) = resolved.text.filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let lower = text.to_lowercase();
        world_names.insert((entity_kind, entity_id, lang.clone()), (text, lower));
    }
}

fn flush_world_names(ctx: &ImportCtx<'_>, world_names: &WorldNameMap) -> Result<u64> {
    let mut rows: u64 = 0;

    let mut name_stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO world_name (entity_kind, entity_id, lang, name, name_lower) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|err| DbError::sqlite("准备写入 world_name 表", err))?;

    let mut fts_stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO world_name_fts (entity_kind, entity_id, lang, name) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|err| DbError::sqlite("准备写入 world_name_fts 表", err))?;

    for ((entity_kind, entity_id, lang), (text, lower)) in world_names {
        name_stmt
            .execute(params![entity_kind, entity_id, lang, text, lower])
            .map_err(|err| {
                DbError::sqlite(
                    format!("写入 world_name ({entity_kind}, {entity_id}, {lang})"),
                    err,
                )
            })?;
        fts_stmt
            .execute(params![entity_kind, entity_id, lang, text])
            .map_err(|err| {
                DbError::sqlite(
                    format!("写入 world_name_fts ({entity_kind}, {entity_id}, {lang})"),
                    err,
                )
            })?;
        rows += 1;
    }

    Ok(rows)
}

fn load_optional_flags(store: &DataStore) -> Result<Option<FlagConditionData>> {
    match store.flag_conditions() {
        Ok(data) => Ok(Some(data)),
        Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn load_optional_playable(store: &DataStore) -> Result<Option<MapUnitPlayableData>> {
    match store.map_unit_playable() {
        Ok(data) => Ok(Some(data)),
        Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn load_optional_units(store: &DataStore) -> Result<Option<Vec<Unit>>> {
    let table = match store.data_table(UNIT_PARAMETER_TABLE) {
        Ok(table) => table,
        Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    };
    let label = datatable_relative_path_to_json(UNIT_PARAMETER_TABLE, store.export_root())
        .map_or_else(
            || UNIT_PARAMETER_TABLE.to_owned(),
            |path| path.display().to_string(),
        );
    Ok(Some(parse_units(&table, &label)?))
}

fn discover_languages(export_root: &Path) -> Vec<String> {
    let mut langs: BTreeSet<String> = BTreeSet::new();
    langs.insert(MODEL_LANGUAGE.to_owned());
    langs.insert(NATIVE_LANGUAGE.to_owned());

    let loc_dir = localization_dir(export_root);
    if let Ok(entries) = fs::read_dir(&loc_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem.eq_ignore_ascii_case("manifest") || !is_safe_language_tag(stem) {
                continue;
            }
            langs.insert(stem.to_owned());
        }
    }

    langs.into_iter().collect()
}

fn resolve_zh_strict(resolver: &TextResolver, value: &FTextValue) -> Option<String> {
    resolver
        .resolve_with(value, Some(MODEL_LANGUAGE), ResolveOptions::strict())
        .text
        .filter(|text| !text.is_empty())
}

fn extract_ftext_key(
    resolver: &TextResolver,
    value: &FTextValue,
) -> (Option<String>, Option<String>) {
    match value {
        FTextValue::StringTable { table_id, key, .. } => {
            let ns = resolver
                .load_string_table_by_id(table_id)
                .map(|table| table.table_namespace.clone())
                .unwrap_or_else(|| fallback_table_namespace(table_id));
            (Some(ns), Some(key.clone()))
        }
        FTextValue::Inline { namespace, key, .. } => (Some(namespace.clone()), Some(key.clone())),
        FTextValue::CultureInvariant { .. } => (None, None),
    }
}

fn fallback_table_namespace(table_id: &str) -> String {
    let trimmed = table_id.trim();
    if let Some((_, after_dot)) = trimmed.rsplit_once('.')
        && !after_dot.is_empty()
    {
        return after_dot.to_owned();
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_owned()
}

fn is_none_island(island_type: &str) -> bool {
    island_type.is_empty() || island_type == "eMapIslandType::None"
}

const fn item_category_str(category: ItemCategory) -> &'static str {
    match category {
        ItemCategory::Consumable => "consumable",
        ItemCategory::Seed => "seed",
        ItemCategory::Equipment => "equipment",
        ItemCategory::Vision => "vision",
        ItemCategory::Valuable => "valuable",
        ItemCategory::Artifact => "artifact",
    }
}
