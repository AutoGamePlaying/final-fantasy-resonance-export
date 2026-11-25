//! Battle area (`03_battle`) import stage.
//!
//! Reuses `ffr_data::battle` parsing via `ctx.require_data_store()` to populate units,
//! multilingual name indexes, elemental resistances, skills, unit-to-skill provenance links,
//! command skills, battle groups, encounter pools, unit AI profiles, break calculation params,
//! and derived enemy-to-chapter mappings.
//!
//! Dangling references (missing unit, skill, battle group, command skill, or level parameter
//! IDs) are recorded in `build_issue` without silently dropping rows or aborting the build.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use ffr_data::battle::{
    BattleData, COMMAND_SKILL_DATA_TABLE, EnemyChapterIndex, UNIT_AI_PARAMETER_TABLE,
    UNIT_PARAMETER_TABLE, parse_encounter_pool,
};
use ffr_data::{
    DEFAULT_LOCRES_LANGUAGES, DataError, DataStore, FTextValue, MODEL_LANGUAGE, ResolveOptions,
    TextResolver, datatable_dir, datatable_relative_path_to_json, is_safe_language_tag,
    localization_dir,
};
use rusqlite::params;
use serde_json::Value;

use crate::build::{ImportCtx, StageStats, relative_label_under};
use crate::error::{DbError, Result};
use crate::query::battle::{
    element_to_db, enemy_chapter_source_kind_to_db, resistance_to_db, skill_attr_to_db,
    skill_cost_to_db, skill_damage_to_db, skill_target_to_db, unit_category_to_db, unit_race_to_db,
};
use crate::record;

const UNIT_LEVEL_PARAM_LIST_TABLE: &str = "Unit/LevelParameter/DT_UnitLevelParameterList";
const UNIT_BATTLE_CMD_TABLE: &str = "Unit/DT_UnitBattleCmd";
const VISION_ITEM_TABLE: &str = "Item/Vision/DT_VisionItemData";
const VISION_AWAKENING_TABLE: &str = "Item/Vision/DT_VisionAwakeningMasteryData";
const VISION_SYNCHRO_TABLE: &str = "Item/Vision/DT_VisionSynchroMasteryData";
const BTL_HIT_EFFECT_TABLE: &str = "Battle/Sequencer/DT_BtlHitEffectData";

/// Import typed battle domain tables and relationships from `ctx.require_data_store()`.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let store = ctx.require_data_store()?;
    let Some(unit_table_path) =
        datatable_relative_path_to_json(UNIT_PARAMETER_TABLE, store.export_root())
    else {
        return Ok(StageStats::skipped("无法定位战斗基础表路径"));
    };
    if !unit_table_path.is_file() {
        return Ok(StageStats::skipped(
            "导出目录缺少 Unit/DT_UnitParameter.json，跳过 battle 阶段",
        ));
    }

    let battle = store.battle()?;
    let resolver = &battle.text_resolver;
    let languages = discover_languages(store.export_root())?;

    let unit_ids: BTreeSet<i64> = battle.units.iter().map(|u| u.id).collect();
    let skill_ids: BTreeSet<i64> = battle.skills.iter().map(|s| s.id).collect();
    let battle_group_ids: BTreeSet<i64> = battle.battle_groups.iter().map(|g| g.id).collect();

    let mut total_rows: u64 = 0;

    // 1. Units, multilingual names (+ FTS5), and elemental resistances.
    total_rows += import_units(ctx, &battle, resolver, &languages)?;

    // 2. Skills and multilingual names (+ FTS5).
    total_rows += import_skills(ctx, &battle, resolver, &languages)?;

    // 3. Command skills (optional if table absent in minimal synthetic fixtures).
    let (cmd_rows, command_skill_ids) = import_command_skills(ctx, store, resolver, &unit_ids)?;
    total_rows += cmd_rows;

    // Validate Skill -> CommandSkill references when DT_CommandSkillData was loaded.
    if let Some(ref cmd_ids) = command_skill_ids {
        let mut sorted_skills: Vec<_> = battle.skills.iter().collect();
        sorted_skills.sort_by_key(|s| s.id);
        for skill in sorted_skills {
            for &cmd_id in &skill.belong_command_ids {
                if !cmd_ids.contains(&cmd_id) {
                    ctx.record_issue(
                        "dangling_command_skill_ref",
                        &format!("skill:{}", skill.id),
                        &format!(
                            "招式 {} ({}) 引用了不存在的指令技能编号 {}",
                            skill.id, skill.row_key, cmd_id
                        ),
                    )?;
                }
            }
        }
    }

    // 4. Battle groups and positional member slots.
    total_rows += import_battle_groups(ctx, &battle, &unit_ids)?;

    // 5. Regional & root encounter pool entries.
    total_rows += import_encounters(ctx, store, &battle_group_ids)?;

    // 6. Unit AI profiles.
    total_rows += import_unit_ai(ctx, &battle, &unit_ids)?;

    // 7. Unit -> Skill provenance links (AI, level-up, Vision mastery/finish blow, battle cmd).
    total_rows += import_unit_skills(
        ctx,
        store,
        &battle,
        &unit_ids,
        &skill_ids,
        command_skill_ids.as_ref(),
    )?;

    // 8. Global break & elemental calculation parameters (single row).
    total_rows += import_battle_param(ctx, &battle)?;

    // 9. Derived Enemy <-> Chapter mapping via EnemyChapterIndex.
    total_rows += import_enemy_chapters(ctx, store, &battle)?;

    ctx.record_dir_source(
        store.export_root(),
        Some(store.cached_data_table_count() as u64),
    )?;

    Ok(StageStats::ok(total_rows))
}

fn import_units(
    ctx: &ImportCtx<'_>,
    battle: &BattleData,
    resolver: &TextResolver,
    languages: &[String],
) -> Result<u64> {
    let tx = ctx.tx();
    let mut unit_stmt = tx
        .prepare_cached(
            "INSERT INTO unit (\
                id, row_key, name_ns, name_key, name_zh, category, race, level, \
                is_boss, max_hp, max_break_point, doc\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(|err| DbError::sqlite("准备写入 unit 表", err))?;
    let mut name_stmt = tx
        .prepare_cached("INSERT INTO unit_name (unit_id, lang, name) VALUES (?1, ?2, ?3)")
        .map_err(|err| DbError::sqlite("准备写入 unit_name 表", err))?;
    let mut fts_stmt = tx
        .prepare_cached("INSERT INTO unit_name_fts (unit_id, lang, name) VALUES (?1, ?2, ?3)")
        .map_err(|err| DbError::sqlite("准备写入 unit_name_fts 表", err))?;
    let mut res_stmt = tx
        .prepare_cached(
            "INSERT INTO unit_resistance (unit_id, element, resistance) VALUES (?1, ?2, ?3)",
        )
        .map_err(|err| DbError::sqlite("准备写入 unit_resistance 表", err))?;

    let mut sorted_units: Vec<_> = battle.units.iter().collect();
    sorted_units.sort_by_key(|u| u.id);

    let mut rows: u64 = 0;
    for unit in sorted_units {
        let (name_ns, name_key) = extract_ftext_key(&unit.name, resolver);
        let name_zh = resolve_strict_non_empty(resolver, &unit.name, MODEL_LANGUAGE);
        let doc = record::encode_doc("unit", unit.id, unit)?;
        unit_stmt
            .execute(params![
                unit.id,
                unit.row_key,
                name_ns,
                name_key,
                name_zh,
                unit_category_to_db(unit.category),
                unit_race_to_db(unit.race),
                unit.level,
                i64::from(unit.is_boss),
                unit.max_hp,
                unit.max_break_point,
                doc,
            ])
            .map_err(|err| DbError::sqlite(format!("写入 unit 主键 {}", unit.id), err))?;
        rows += 1;

        for lang in languages {
            if let Some(text) = resolve_strict_non_empty(resolver, &unit.name, lang) {
                name_stmt
                    .execute(params![unit.id, lang, text])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 unit_name ({}, {lang})", unit.id), err)
                    })?;
                fts_stmt
                    .execute(params![unit.id, lang, text])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 unit_name_fts ({}, {lang})", unit.id), err)
                    })?;
                rows += 1;
            }
        }

        let mut resistances: Vec<(&'static str, &'static str)> = unit
            .element_resistances
            .iter()
            .map(|(&element, &resistance)| (element_to_db(element), resistance_to_db(resistance)))
            .collect();
        resistances.sort_by_key(|(elem, _)| *elem);
        for (elem, res) in resistances {
            res_stmt
                .execute(params![unit.id, elem, res])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 unit_resistance ({}, {elem})", unit.id), err)
                })?;
            rows += 1;
        }
    }
    Ok(rows)
}

fn import_skills(
    ctx: &ImportCtx<'_>,
    battle: &BattleData,
    resolver: &TextResolver,
    languages: &[String],
) -> Result<u64> {
    let tx = ctx.tx();
    let mut skill_stmt = tx
        .prepare_cached(
            "INSERT INTO skill (\
                id, row_key, name_ns, name_key, name_zh, attr_type, damage_type, \
                element, target_type, cost, cost_type, power, break_damage, doc\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .map_err(|err| DbError::sqlite("准备写入 skill 表", err))?;
    let mut name_stmt = tx
        .prepare_cached("INSERT INTO skill_name (skill_id, lang, name) VALUES (?1, ?2, ?3)")
        .map_err(|err| DbError::sqlite("准备写入 skill_name 表", err))?;
    let mut fts_stmt = tx
        .prepare_cached("INSERT INTO skill_name_fts (skill_id, lang, name) VALUES (?1, ?2, ?3)")
        .map_err(|err| DbError::sqlite("准备写入 skill_name_fts 表", err))?;

    let mut sorted_skills: Vec<_> = battle.skills.iter().collect();
    sorted_skills.sort_by_key(|s| s.id);

    let mut rows: u64 = 0;
    for skill in sorted_skills {
        let (name_ns, name_key) = extract_ftext_key(&skill.name, resolver);
        let name_zh = resolve_strict_non_empty(resolver, &skill.name, MODEL_LANGUAGE);
        let doc = record::encode_doc("skill", skill.id, skill)?;
        skill_stmt
            .execute(params![
                skill.id,
                skill.row_key,
                name_ns,
                name_key,
                name_zh,
                skill_attr_to_db(skill.skill_attr_type),
                skill_damage_to_db(skill.damage_type),
                element_to_db(skill.element),
                skill_target_to_db(skill.target_type),
                skill.cost,
                skill_cost_to_db(skill.cost_type),
                skill.power,
                skill.break_damage,
                doc,
            ])
            .map_err(|err| DbError::sqlite(format!("写入 skill 主键 {}", skill.id), err))?;
        rows += 1;

        for lang in languages {
            if let Some(text) = resolve_strict_non_empty(resolver, &skill.name, lang) {
                name_stmt
                    .execute(params![skill.id, lang, text])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 skill_name ({}, {lang})", skill.id), err)
                    })?;
                fts_stmt
                    .execute(params![skill.id, lang, text])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 skill_name_fts ({}, {lang})", skill.id), err)
                    })?;
                rows += 1;
            }
        }
    }
    Ok(rows)
}

fn import_command_skills(
    ctx: &ImportCtx<'_>,
    store: &DataStore,
    resolver: &TextResolver,
    unit_ids: &BTreeSet<i64>,
) -> Result<(u64, Option<BTreeSet<i64>>)> {
    let command_data = match store.command_skills() {
        Ok(data) => data,
        Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
            ctx.record_issue(
                "missing_optional_table",
                COMMAND_SKILL_DATA_TABLE,
                "导出目录缺少 Skill/DT_CommandSkillData.json，command_skill 表为空",
            )?;
            return Ok((0, None));
        }
        Err(err) => return Err(err.into()),
    };

    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO command_skill (\
                id, row_key, sort_id, unit_id, name_ns, name_key, name_zh, \
                is_removable, equip_cost, doc\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(|err| DbError::sqlite("准备写入 command_skill 表", err))?;

    let mut sorted_cmds: Vec<_> = command_data.commands.iter().collect();
    sorted_cmds.sort_by_key(|c| c.id);

    let mut ids = BTreeSet::new();
    let mut rows: u64 = 0;
    for cmd in sorted_cmds {
        ids.insert(cmd.id);
        if let Some(uid) = cmd.unit_id_to_use_skill
            && !unit_ids.contains(&uid)
        {
            ctx.record_issue(
                "dangling_unit_ref",
                &format!("command_skill:{}", cmd.id),
                &format!(
                    "指令技能 {} ({}) 引用了不存在的单位编号 {}",
                    cmd.id, cmd.row_key, uid
                ),
            )?;
        }
        let (name_ns, name_key) = extract_ftext_key(&cmd.name, resolver);
        let name_zh = resolve_strict_non_empty(resolver, &cmd.name, MODEL_LANGUAGE);
        let doc = record::encode_doc("command_skill", cmd.id, cmd)?;
        stmt.execute(params![
            cmd.id,
            cmd.row_key,
            cmd.sort_id,
            cmd.unit_id_to_use_skill,
            name_ns,
            name_key,
            name_zh,
            i64::from(cmd.is_removable),
            cmd.equip_cost,
            doc,
        ])
        .map_err(|err| DbError::sqlite(format!("写入 command_skill 主键 {}", cmd.id), err))?;
        rows += 1;
    }
    Ok((rows, Some(ids)))
}

fn import_battle_groups(
    ctx: &ImportCtx<'_>,
    battle: &BattleData,
    unit_ids: &BTreeSet<i64>,
) -> Result<u64> {
    let tx = ctx.tx();
    let mut group_stmt = tx
        .prepare_cached(
            "INSERT INTO battle_group (\
                id, row_key, is_boss, can_escape, is_bounty, ap, battle_event_id, doc\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(|err| DbError::sqlite("准备写入 battle_group 表", err))?;
    let mut member_stmt = tx
        .prepare_cached(
            "INSERT INTO battle_group_member (group_id, slot, unit_id) VALUES (?1, ?2, ?3)",
        )
        .map_err(|err| DbError::sqlite("准备写入 battle_group_member 表", err))?;

    let mut sorted_groups: Vec<_> = battle.battle_groups.iter().collect();
    sorted_groups.sort_by_key(|g| g.id);

    let mut rows: u64 = 0;
    for group in sorted_groups {
        let doc = record::encode_doc("battle_group", group.id, group)?;
        let is_boss_int = group.is_boss.map(i64::from);
        group_stmt
            .execute(params![
                group.id,
                group.row_key,
                is_boss_int,
                i64::from(group.can_escape),
                i64::from(group.is_bounty),
                group.ap,
                group.battle_event_id,
                doc,
            ])
            .map_err(|err| DbError::sqlite(format!("写入 battle_group 主键 {}", group.id), err))?;
        rows += 1;

        for (slot, &unit_id) in group.unit_ids.iter().enumerate() {
            if !unit_ids.contains(&unit_id) {
                ctx.record_issue(
                    "dangling_unit_ref",
                    &format!("battle_group:{}", group.id),
                    &format!(
                        "战斗组 {} ({}) 槽位 {} 引用了不存在的单位编号 {}",
                        group.id, group.row_key, slot, unit_id
                    ),
                )?;
            }
            member_stmt
                .execute(params![group.id, slot as i64, unit_id])
                .map_err(|err| {
                    DbError::sqlite(
                        format!("写入 battle_group_member ({}, slot={slot})", group.id),
                        err,
                    )
                })?;
            rows += 1;
        }
    }
    Ok(rows)
}

fn import_encounters(
    ctx: &ImportCtx<'_>,
    store: &DataStore,
    battle_group_ids: &BTreeSet<i64>,
) -> Result<u64> {
    let encount_root = datatable_dir(store.export_root()).join("Encount");
    if !encount_root.is_dir() {
        return Ok(0);
    }

    let mut pool_files: Vec<(Option<String>, String, PathBuf)> = Vec::new();
    for file_path in sorted_json_files(&encount_root)? {
        let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        pool_files.push((None, format!("Encount/{stem}"), file_path));
    }
    for region_dir in sorted_subdirs(&encount_root)? {
        let Some(region) = region_dir.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if region.eq_ignore_ascii_case("Staging") {
            continue;
        }
        for file_path in sorted_json_files(&region_dir)? {
            let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            pool_files.push((
                Some(region.to_owned()),
                format!("Encount/{region}/{stem}"),
                file_path,
            ));
        }
    }
    pool_files.sort_by(|a, b| a.1.cmp(&b.1));

    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO encounter (\
                pool_path, region, row_key, battle_group_id, encount_ratio, doc\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|err| DbError::sqlite("准备写入 encounter 表", err))?;

    let mut rows: u64 = 0;
    for (region, pool_path, file_path) in pool_files {
        let table = store.data_table(&pool_path)?;
        if table.row_struct != "ENCOUNT_GROUP_DATA" {
            continue;
        }
        let label = file_path.display().to_string();
        let mut entries = parse_encounter_pool(&table, &label)?;
        entries.sort_by(|a, b| a.row_key.cmp(&b.row_key));
        for entry in entries {
            if !battle_group_ids.contains(&entry.battle_group_id) {
                ctx.record_issue(
                    "dangling_battle_group_ref",
                    &format!("encounter:{pool_path}:{}", entry.row_key),
                    &format!(
                        "遭遇池 {pool_path} 行 {} 引用了不存在的战斗组编号 {}",
                        entry.row_key, entry.battle_group_id
                    ),
                )?;
            }
            let doc = record::encode_doc(
                "encounter",
                format!("{pool_path}:{}", entry.row_key),
                &entry,
            )?;
            stmt.execute(params![
                pool_path,
                region,
                entry.row_key,
                entry.battle_group_id,
                entry.encount_ratio,
                doc,
            ])
            .map_err(|err| {
                DbError::sqlite(
                    format!("写入 encounter ({pool_path}:{})", entry.row_key),
                    err,
                )
            })?;
            rows += 1;
        }
    }

    Ok(rows)
}

fn import_unit_ai(
    ctx: &ImportCtx<'_>,
    battle: &BattleData,
    unit_ids: &BTreeSet<i64>,
) -> Result<u64> {
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO unit_ai (unit_id, row_key, ai_id, fight_skill_id, doc) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|err| DbError::sqlite("准备写入 unit_ai 表", err))?;

    let mut rows: u64 = 0;
    for ai in battle.unit_ai.values() {
        if !unit_ids.contains(&ai.id) {
            ctx.record_issue(
                "dangling_unit_ref",
                &format!("unit_ai:{}", ai.id),
                &format!(
                    "单位 AI {} ({}) 引用了不存在的单位编号 {}",
                    ai.id, ai.row_key, ai.id
                ),
            )?;
        }
        let doc = record::encode_doc("unit_ai", ai.id, ai)?;
        stmt.execute(params![ai.id, ai.row_key, ai.ai_id, ai.fight_skill_id, doc])
            .map_err(|err| DbError::sqlite(format!("写入 unit_ai 主键 {}", ai.id), err))?;
        rows += 1;
    }
    Ok(rows)
}

fn import_unit_skills(
    ctx: &ImportCtx<'_>,
    store: &DataStore,
    battle: &BattleData,
    unit_ids: &BTreeSet<i64>,
    skill_ids: &BTreeSet<i64>,
    command_skill_ids: Option<&BTreeSet<i64>>,
) -> Result<u64> {
    // Deduplicate `(unit_id, skill_id, source, level)` deterministically before inserting.
    let mut links: BTreeSet<(i64, i64, &'static str, Option<i64>)> = BTreeSet::new();

    // 1. Structured AI skill links from BattleData.unit_ai.
    for ai in battle.unit_ai.values() {
        if ai.fight_skill_id >= 0 {
            links.insert((ai.id, ai.fight_skill_id, "ai_fight", None));
        }
        if ai.chase_skill_id >= 0 {
            links.insert((ai.id, ai.chase_skill_id, "ai_chase", None));
        }
        if ai.silence_fight_id >= 0 {
            links.insert((ai.id, ai.silence_fight_id, "ai_silence_fight", None));
        }
        for &sid in &ai.counter_skill_ids {
            if sid >= 0 {
                links.insert((ai.id, sid, "ai_counter", None));
            }
        }
        for &sid in &ai.high_risk_skill_ids {
            if sid >= 0 {
                links.insert((ai.id, sid, "ai_high_risk", None));
            }
        }
    }

    // 2. Random AI skill pools (`skillRandData1`, `skillRandData2`) from cached DT_BtlUnitAIParameter.
    collect_ai_rand_skills(store, unit_ids, skill_ids, &mut links)?;

    // 3. Level-up learned skills (`DT_UnitLevelParameterList` -> `DT_UnitLevelParameter*`).
    collect_level_up_skills(ctx, store, battle, &mut links)?;

    // 4. Vision mastery active skills and finish-blow skills.
    collect_vision_mastery_skills(
        store,
        VISION_AWAKENING_TABLE,
        "vision_awakening",
        &mut links,
    )?;
    collect_vision_mastery_skills(store, VISION_SYNCHRO_TABLE, "vision_synchro", &mut links)?;
    collect_vision_finish_blow_skills(store, &mut links)?;

    // 5. Direct active skills from `Unit/DT_UnitBattleCmd`.
    collect_battle_cmd_skills(ctx, store, skill_ids, command_skill_ids, &mut links)?;

    let hit_effect_names = load_hit_effect_names(store)?;

    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO unit_skill (unit_id, skill_id, source, level) VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|err| DbError::sqlite("准备写入 unit_skill 表", err))?;

    let mut rows: u64 = 0;
    for (unit_id, skill_id, source, level) in links {
        if !unit_ids.contains(&unit_id) {
            ctx.record_issue(
                "dangling_unit_ref",
                &format!("unit_skill:{unit_id}:{source}:{skill_id}"),
                &format!(
                    "单位招式关联（来源 {source}，技能 {skill_id}）引用了不存在的单位编号 {unit_id}"
                ),
            )?;
        }
        if !skill_ids.contains(&skill_id) {
            let detail = if let Some(hit_row) = hit_effect_names.get(&skill_id) {
                format!(
                    "单位 {unit_id}（来源 {source}）引用了 DT_SkillData 中不存在的招式编号 {skill_id}（见于 DT_BtlHitEffectData：{hit_row}，属未入招式表的幻兵/特殊普攻编号）"
                )
            } else {
                format!(
                    "单位 {unit_id}（来源 {source}）引用了 DT_SkillData 与命中特效表中均不存在的招式编号 {skill_id}（疑为已删除或占位编号）"
                )
            };
            ctx.record_issue(
                "dangling_skill_ref",
                &format!("unit_skill:{unit_id}:{source}:{skill_id}"),
                &detail,
            )?;
        }
        stmt.execute(params![unit_id, skill_id, source, level])
            .map_err(|err| {
                DbError::sqlite(
                    format!("写入 unit_skill ({unit_id}, {skill_id}, {source})"),
                    err,
                )
            })?;
        rows += 1;
    }

    Ok(rows)
}

fn collect_ai_rand_skills(
    store: &DataStore,
    unit_ids: &BTreeSet<i64>,
    skill_ids: &BTreeSet<i64>,
    links: &mut BTreeSet<(i64, i64, &'static str, Option<i64>)>,
) -> Result<()> {
    let table = store.data_table(UNIT_AI_PARAMETER_TABLE)?;
    for raw_row in table.rows.values() {
        let Some(obj) = raw_row.as_object() else {
            continue;
        };
        let Some(unit_id) = obj.get("ID").and_then(Value::as_i64) else {
            continue;
        };
        for (field, source) in [
            ("skillRandData1", "ai_rand1"),
            ("skillRandData2", "ai_rand2"),
        ] {
            let Some(entries) = obj.get(field).and_then(Value::as_array) else {
                continue;
            };
            for item in entries {
                let Some(entry_obj) = item.as_object() else {
                    continue;
                };
                if let Some(sid) = entry_obj.get("SkillId").and_then(Value::as_i64)
                    && sid >= 0
                {
                    // Boss AI tables repurpose 0%-probability `skillRandData` slots to pass
                    // linked/summoned/form-change Unit IDs (`DT_UnitParameter.ID`) rather than
                    // skills. Skip those non-skill unit parameter references.
                    let prob = entry_obj.get("Probability").and_then(Value::as_f64);
                    if prob == Some(0.0) && !skill_ids.contains(&sid) && unit_ids.contains(&sid) {
                        continue;
                    }
                    links.insert((unit_id, sid, source, None));
                }
            }
        }
    }
    Ok(())
}

fn load_hit_effect_names(store: &DataStore) -> Result<BTreeMap<i64, String>> {
    let Some(table) = load_optional_table(store, BTL_HIT_EFFECT_TABLE)? else {
        return Ok(BTreeMap::new());
    };
    let mut names = BTreeMap::new();
    for (row_key, raw_row) in &table.rows {
        if let Some(id) = raw_row
            .as_object()
            .and_then(|obj| obj.get("ID"))
            .and_then(Value::as_i64)
        {
            names.entry(id).or_insert_with(|| row_key.clone());
        }
    }
    Ok(names)
}

fn collect_level_up_skills(
    ctx: &ImportCtx<'_>,
    store: &DataStore,
    battle: &BattleData,
    links: &mut BTreeSet<(i64, i64, &'static str, Option<i64>)>,
) -> Result<()> {
    let list_table = match load_optional_table(store, UNIT_LEVEL_PARAM_LIST_TABLE)? {
        Some(table) => table,
        None => return Ok(()),
    };

    let mut param_paths: BTreeMap<i64, String> = BTreeMap::new();
    for raw_row in list_table.rows.values() {
        let Some(obj) = raw_row.as_object() else {
            continue;
        };
        let Some(id) = obj.get("ID").and_then(Value::as_i64) else {
            continue;
        };
        if let Some(ue_path) = obj
            .get("DataTable")
            .and_then(Value::as_object)
            .and_then(|dt| dt.get("ObjectPath"))
            .and_then(Value::as_str)
        {
            param_paths.insert(id, ue_path.to_owned());
        }
    }

    let mut sorted_units: Vec<_> = battle.units.iter().collect();
    sorted_units.sort_by_key(|u| u.id);
    for unit in sorted_units {
        if unit.level_param_id < 0 {
            continue;
        }
        let Some(ue_path) = param_paths.get(&unit.level_param_id) else {
            ctx.record_issue(
                "dangling_level_param",
                &format!("unit:{}", unit.id),
                &format!(
                    "单位 {} ({}) 引用的成长表编号 {} 在 {} 中不存在",
                    unit.id, unit.row_key, unit.level_param_id, UNIT_LEVEL_PARAM_LIST_TABLE
                ),
            )?;
            continue;
        };

        let lvl_table = match store.data_table_by_ue_path(ue_path) {
            Ok(table) => table,
            Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
                ctx.record_issue(
                    "dangling_level_param_table",
                    &format!("unit:{}", unit.id),
                    &format!(
                        "单位 {} ({}) 对应的成长表文件不存在：{ue_path}",
                        unit.id, unit.row_key
                    ),
                )?;
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        for raw_row in lvl_table.rows.values() {
            let Some(obj) = raw_row.as_object() else {
                continue;
            };
            let level = obj.get("Level").and_then(Value::as_i64);
            let Some(add_skills) = obj.get("AddSkills").and_then(Value::as_array) else {
                continue;
            };
            for item in add_skills {
                if let Some(sid) = item.as_i64()
                    && sid >= 0
                {
                    links.insert((unit.id, sid, "level_up", level));
                }
            }
        }
    }

    Ok(())
}

fn collect_vision_mastery_skills(
    store: &DataStore,
    table_path: &str,
    source_name: &'static str,
    links: &mut BTreeSet<(i64, i64, &'static str, Option<i64>)>,
) -> Result<()> {
    let table = match load_optional_table(store, table_path)? {
        Some(table) => table,
        None => return Ok(()),
    };
    for raw_row in table.rows.values() {
        let Some(obj) = raw_row.as_object() else {
            continue;
        };
        let Some(unit_id) = obj.get("ID").and_then(Value::as_i64) else {
            continue;
        };
        let level = obj.get("Level").and_then(Value::as_i64);
        let Some(details) = obj.get("detailData").and_then(Value::as_array) else {
            continue;
        };
        for detail in details {
            let Some(detail_obj) = detail.as_object() else {
                continue;
            };
            let param_type = detail_obj
                .get("parameterType")
                .and_then(Value::as_str)
                .unwrap_or("");
            if param_type != "eVisionMasteryParameterType::ActiveSkill" {
                continue;
            }
            if let Some(sid) = detail_obj
                .get("params")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(Value::as_i64)
                && sid >= 0
            {
                links.insert((unit_id, sid, source_name, level));
            }
        }
    }
    Ok(())
}

fn collect_vision_finish_blow_skills(
    store: &DataStore,
    links: &mut BTreeSet<(i64, i64, &'static str, Option<i64>)>,
) -> Result<()> {
    let table = match load_optional_table(store, VISION_ITEM_TABLE)? {
        Some(table) => table,
        None => return Ok(()),
    };
    for raw_row in table.rows.values() {
        let Some(obj) = raw_row.as_object() else {
            continue;
        };
        let Some(unit_id) = obj.get("ID").and_then(Value::as_i64) else {
            continue;
        };
        if let Some(sid) = obj.get("finishBlowSkill").and_then(Value::as_i64)
            && sid >= 0
        {
            links.insert((unit_id, sid, "vision_finish_blow", None));
        }
    }
    Ok(())
}

fn collect_battle_cmd_skills(
    ctx: &ImportCtx<'_>,
    store: &DataStore,
    skill_ids: &BTreeSet<i64>,
    command_skill_ids: Option<&BTreeSet<i64>>,
    links: &mut BTreeSet<(i64, i64, &'static str, Option<i64>)>,
) -> Result<()> {
    let table = match load_optional_table(store, UNIT_BATTLE_CMD_TABLE)? {
        Some(table) => table,
        None => return Ok(()),
    };
    for raw_row in table.rows.values() {
        let Some(obj) = raw_row.as_object() else {
            continue;
        };
        let Some(unit_id) = obj.get("ID").and_then(Value::as_i64) else {
            continue;
        };
        let Some(details) = obj.get("detailData").and_then(Value::as_array) else {
            continue;
        };
        for detail in details {
            let Some(sid) = detail
                .as_object()
                .and_then(|m| m.get("SkillId"))
                .and_then(Value::as_i64)
            else {
                continue;
            };
            if sid < 0 {
                continue;
            }
            if skill_ids.contains(&sid) {
                links.insert((unit_id, sid, "battle_cmd", None));
            } else if let Some(cmd_ids) = command_skill_ids
                && !cmd_ids.contains(&sid)
            {
                ctx.record_issue(
                    "dangling_battle_cmd_ref",
                    &format!("unit_battle_cmd:{unit_id}"),
                    &format!("角色战斗指令 {unit_id} 引用了未知的技能或指令编号 {sid}"),
                )?;
            }
        }
    }
    Ok(())
}

fn import_battle_param(ctx: &ImportCtx<'_>, battle: &BattleData) -> Result<u64> {
    let doc = record::encode_doc("battle_param", 1, &battle.break_calc)?;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO battle_param (id, row_key, break_damage_magnification, doc) \
             VALUES (1, ?1, ?2, ?3)",
        )
        .map_err(|err| DbError::sqlite("准备写入 battle_param 表", err))?;
    stmt.execute(params![
        battle.break_calc.row_key,
        battle.break_calc.break_damage_magnification,
        doc,
    ])
    .map_err(|err| DbError::sqlite("写入 battle_param", err))?;
    Ok(1)
}

fn import_enemy_chapters(
    ctx: &ImportCtx<'_>,
    store: &DataStore,
    battle: &BattleData,
) -> Result<u64> {
    let index = match EnemyChapterIndex::from_store_with_battle(store, battle) {
        Ok(index) => index,
        Err(DataError::Io { source, path, .. }) if source.kind() == ErrorKind::NotFound => {
            let rel = relative_label_under(store.export_root(), "export", &path);
            ctx.record_issue(
                "missing_optional_table",
                "enemy_chapter",
                &format!("缺少推导敌人章节所需的辅助表（{rel}），enemy_chapter 表为空"),
            )?;
            return Ok(0);
        }
        Err(err) => return Err(err.into()),
    };

    let mut chapter_rows: BTreeSet<(i64, i64, &'static str, i64, String)> = BTreeSet::new();
    for ch in index.chapters() {
        for &uid in index.enemies_in_chapter(ch) {
            for src in index.sources_for_enemy(uid) {
                chapter_rows.insert((
                    uid,
                    src.chapter_no,
                    enemy_chapter_source_kind_to_db(src.kind),
                    src.battle_group_id,
                    src.source_ref.clone(),
                ));
            }
        }
    }

    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO enemy_chapter (unit_id, chapter_no, source, battle_group_id, source_ref) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|err| DbError::sqlite("准备写入 enemy_chapter 表", err))?;

    let mut rows: u64 = 0;
    for (unit_id, chapter_no, source_kind, battle_group_id, source_ref) in chapter_rows {
        stmt.execute(params![
            unit_id,
            chapter_no,
            source_kind,
            battle_group_id,
            source_ref,
        ])
        .map_err(|err| {
            DbError::sqlite(
                format!("写入 enemy_chapter (unit={unit_id}, ch={chapter_no})"),
                err,
            )
        })?;
        rows += 1;
    }

    Ok(rows)
}

fn load_optional_table(
    store: &DataStore,
    relative_path: &str,
) -> Result<Option<std::sync::Arc<ffr_data::DataTable>>> {
    match store.data_table(relative_path) {
        Ok(table) => Ok(Some(table)),
        Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn discover_languages(export_root: &Path) -> Result<Vec<String>> {
    let mut langs: BTreeSet<String> = DEFAULT_LOCRES_LANGUAGES
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let loc_dir = localization_dir(export_root);
    if loc_dir.is_dir() {
        let entries =
            fs::read_dir(&loc_dir).map_err(|err| DbError::io("读取本地化目录", &loc_dir, err))?;
        for entry in entries {
            let entry = entry.map_err(|err| DbError::io("遍历本地化目录", &loc_dir, err))?;
            let path = entry.path();
            if path.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                && stem != "manifest"
                && is_safe_language_tag(stem)
            {
                langs.insert(stem.to_owned());
            }
        }
    }
    Ok(langs.into_iter().collect())
}

fn extract_ftext_key(
    value: &FTextValue,
    resolver: &TextResolver,
) -> (Option<String>, Option<String>) {
    match value {
        FTextValue::StringTable { table_id, key, .. } => {
            let ns = resolver
                .load_string_table_by_id(table_id)
                .map(|table| table.table_namespace.clone())
                .unwrap_or_else(|| fallback_namespace_from_table_id(table_id));
            (Some(ns), Some(key.clone()))
        }
        FTextValue::Inline { namespace, key, .. } => (Some(namespace.clone()), Some(key.clone())),
        FTextValue::CultureInvariant { .. } => (None, None),
    }
}

fn fallback_namespace_from_table_id(table_id: &str) -> String {
    let after_slash = table_id.rsplit('/').next().unwrap_or(table_id);
    after_slash
        .split('.')
        .next()
        .unwrap_or(after_slash)
        .to_owned()
}

fn resolve_strict_non_empty(
    resolver: &TextResolver,
    value: &FTextValue,
    lang: &str,
) -> Option<String> {
    resolver
        .resolve_with(value, Some(lang), ResolveOptions::strict())
        .text
        .filter(|s| !s.is_empty())
}

fn sorted_subdirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).map_err(|err| DbError::io("读取遇敌子目录", dir, err))?;
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("遍历遇敌子目录", dir, err))?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(dirs)
}

fn sorted_json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).map_err(|err| DbError::io("读取遇敌数据表目录", dir, err))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("遍历遇敌数据表目录", dir, err))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}
