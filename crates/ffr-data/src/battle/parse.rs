//! Strict schema-validating parsers and file loaders for battle data tables.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value};

use crate::battle::types::{
    ALL_COMBAT_ELEMENTS, ArmorType, AvailableLocation, BattleGroup, BreakCalcParams, CommandSkill,
    Element, EncounterPoolEntry, PlayableUnitProfile, Resistance, Skill, SkillAttributeType,
    SkillCostOverride, SkillCostType, SkillDamageType, SkillTargetType, TargetRelation,
    TargetState, Unit, UnitAiProfile, UnitCategory, UnitRace, WeaponType,
};
use crate::battle::{
    BATTLE_GROUP_TABLE, COMMAND_SKILL_DATA_TABLE, PLAYABLE_UNIT_PARAMETER_TABLE,
    SKILL_ATTRIBUTE_DETAIL_TABLE, SKILL_DATA_TABLE, SKILL_GENERAL_DATA_TABLE,
    UNIT_AI_PARAMETER_TABLE, UNIT_PARAMETER_CALC_TABLE, UNIT_PARAMETER_TABLE,
};
use crate::datatable::{DataTable, load_data_table};
use crate::error::{DataError, Result};
use crate::ftext::{FTextValue, parse_ftext};
use crate::paths::datatable_relative_path_to_json;

/// Parse `Unit/DT_UnitParameter` into typed [`Unit`] records sorted by `id` ascending.
pub fn parse_units(table: &DataTable, source_label: &str) -> Result<Vec<Unit>> {
    expect_row_struct(table, "UNIT_PARAMETER", source_label)?;

    let mut units = Vec::with_capacity(table.rows.len());
    let mut seen_ids = HashSet::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;

        let id = expect_i64(row, "ID", source_label, &row_prefix)?;
        if !seen_ids.insert(id) {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.ID"),
                format!("重复的单位编号 {id}"),
            ));
        }

        let race_raw = expect_string(row, "Race", source_label, &row_prefix)?;
        let race = UnitRace::from_ue_str(&race_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.Race"),
                format!("未知的种族枚举值：{race_raw}"),
            )
        })?;

        let category_raw = expect_string(row, "Category", source_label, &row_prefix)?;
        let category = UnitCategory::from_ue_str(&category_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.Category"),
                format!("未知的单位分类枚举值：{category_raw}"),
            )
        })?;

        let element_resistances = parse_element_resistances(row, source_label, &row_prefix)?;
        let status_resistances = parse_status_resistances(row, source_label, &row_prefix)?;
        let accessory_ids =
            parse_i64_array(row, "AccessoryIdList", source_label, &row_prefix, true)?;
        let passive_skill_ids =
            parse_i64_array(row, "passiveSkillList", source_label, &row_prefix, true)?;
        let traits = parse_ftext_array(row, "Traits", source_label, &row_prefix)?;

        units.push(Unit {
            row_key: row_key.clone(),
            id,
            name: expect_ftext(row, "Name", source_label, &row_prefix)?,
            description: expect_ftext(row, "Description", source_label, &row_prefix)?,
            level: expect_i64(row, "Level", source_label, &row_prefix)?,
            level_param_id: expect_i64(row, "LevelParamId", source_label, &row_prefix)?,
            sex: expect_string(row, "Sex", source_label, &row_prefix)?,
            race,
            category,
            is_boss: expect_bool(row, "IsBoss", source_label, &row_prefix)?,
            is_accent: expect_bool(row, "IsAccent", source_label, &row_prefix)?,
            max_hp: expect_i64(row, "MaxHitPoint", source_label, &row_prefix)?,
            max_mp: expect_i64(row, "MaxMagicPoint", source_label, &row_prefix)?,
            max_break_point: expect_i64(row, "MaxBreakPoint", source_label, &row_prefix)?,
            attack: expect_i64(row, "Attack", source_label, &row_prefix)?,
            defence: expect_i64(row, "Defence", source_label, &row_prefix)?,
            intelligence: expect_i64(row, "Intelligence", source_label, &row_prefix)?,
            mind: expect_i64(row, "Mind", source_label, &row_prefix)?,
            divine: expect_i64(row, "Divine", source_label, &row_prefix)?,
            agility: expect_i64(row, "Agility", source_label, &row_prefix)?,
            element_resistances,
            status_resistances,
            weapon_id: expect_i64(row, "WeaponId", source_label, &row_prefix)?,
            armor_id: expect_i64(row, "ArmorId", source_label, &row_prefix)?,
            accessory_ids,
            vision_id: expect_i64(row, "VisionId", source_label, &row_prefix)?,
            passive_skill_ids,
            experience: expect_i64(row, "Experience", source_label, &row_prefix)?,
            money: expect_i64(row, "Money", source_label, &row_prefix)?,
            drop_normal_item: expect_i64(row, "DropNormalItem", source_label, &row_prefix)?,
            drop_normal_item_rate: expect_i64(
                row,
                "DropNormalItemRate",
                source_label,
                &row_prefix,
            )?,
            drop_rare_item: expect_i64(row, "DropRareItem", source_label, &row_prefix)?,
            drop_rare_item_rate: expect_i64(row, "DropRareItemRate", source_label, &row_prefix)?,
            steal_normal_item: expect_i64(row, "StealNormalItem", source_label, &row_prefix)?,
            steal_rare_item: expect_i64(row, "StealRareItem", source_label, &row_prefix)?,
            steal_item_rate: expect_i64(row, "StealItemRate", source_label, &row_prefix)?,
            traits,
        });
    }

    units.sort_by_key(|unit| unit.id);
    Ok(units)
}

/// Load `Unit/DT_UnitParameter.json` from `export_root`.
pub fn load_units(export_root: impl AsRef<Path>) -> Result<Vec<Unit>> {
    let (table, label) = load_relative_table(export_root.as_ref(), UNIT_PARAMETER_TABLE)?;
    parse_units(&table, &label)
}

/// Parse `Unit/DT_PlayableUnitParameter` into typed [`PlayableUnitProfile`] records sorted by `id`.
pub fn parse_playable_unit_parameters(
    table: &DataTable,
    source_label: &str,
) -> Result<Vec<PlayableUnitProfile>> {
    expect_row_struct(table, "UNIT_PLAYABLE_PARAMETER", source_label)?;

    let mut profiles = Vec::with_capacity(table.rows.len());
    let mut seen_ids = HashSet::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;

        let id = expect_i64(row, "ID", source_label, &row_prefix)?;
        if !seen_ids.insert(id) {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.ID"),
                format!("重复的可操控角色编号 {id}"),
            ));
        }

        let attack_raw = expect_string(row, "attackType", source_label, &row_prefix)?;
        let attack_type = SkillDamageType::from_ue_str(&attack_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.attackType"),
                format!("未知的攻击类型枚举值：{attack_raw}"),
            )
        })?;

        let weapon_raw = expect_string(row, "equipableWeaponType", source_label, &row_prefix)?;
        let equipable_weapon_type = WeaponType::from_ue_str(&weapon_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.equipableWeaponType"),
                format!("未知的武器类型枚举值：{weapon_raw}"),
            )
        })?;

        let roles = parse_string_array(row, "roles", source_label, &row_prefix)?;
        let raw_armors =
            parse_string_array(row, "equipableArmorTypeList", source_label, &row_prefix)?;
        let mut equipable_armor_types = Vec::with_capacity(raw_armors.len());
        for (index, raw) in raw_armors.iter().enumerate() {
            let armor = ArmorType::from_ue_str(raw).ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.equipableArmorTypeList[{index}]"),
                    format!("未知的防具类型枚举值：{raw}"),
                )
            })?;
            equipable_armor_types.push(armor);
        }

        profiles.push(PlayableUnitProfile {
            row_key: row_key.clone(),
            id,
            attack_type,
            roles,
            equipable_weapon_type,
            equipable_armor_types,
        });
    }

    profiles.sort_by_key(|profile| profile.id);
    Ok(profiles)
}

/// Load `Unit/DT_PlayableUnitParameter.json` from `export_root`.
pub fn load_playable_unit_parameters(
    export_root: impl AsRef<Path>,
) -> Result<Vec<PlayableUnitProfile>> {
    let (table, label) = load_relative_table(export_root.as_ref(), PLAYABLE_UNIT_PARAMETER_TABLE)?;
    parse_playable_unit_parameters(&table, &label)
}

/// Parse `Unit/AI/DT_BtlUnitAIParameter` into typed [`UnitAiProfile`] records sorted by `id`.
pub fn parse_unit_ai(table: &DataTable, source_label: &str) -> Result<Vec<UnitAiProfile>> {
    expect_row_struct(table, "BTL_UNIT_AI_PARAMETER", source_label)?;

    let mut profiles = Vec::with_capacity(table.rows.len());
    let mut seen_ids = HashSet::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;

        let id = expect_i64(row, "ID", source_label, &row_prefix)?;
        if !seen_ids.insert(id) {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.ID"),
                format!("重复的 AI 单位编号 {id}"),
            ));
        }

        profiles.push(UnitAiProfile {
            row_key: row_key.clone(),
            id,
            fight_skill_id: expect_i64(row, "fightSkillId", source_label, &row_prefix)?,
            chase_skill_id: expect_i64(row, "chaseSkillId", source_label, &row_prefix)?,
            ai_id: expect_i64(row, "aiId", source_label, &row_prefix)?,
            counter_skill_ids: parse_i64_array(
                row,
                "counterSkillIdList",
                source_label,
                &row_prefix,
                true,
            )?,
            silence_fight_id: expect_i64(row, "silenceFightId", source_label, &row_prefix)?,
            high_risk_skill_ids: parse_i64_array(
                row,
                "highRiskSkillIdList",
                source_label,
                &row_prefix,
                true,
            )?,
        });
    }

    profiles.sort_by_key(|profile| profile.id);
    Ok(profiles)
}

/// Load `Unit/AI/DT_BtlUnitAIParameter.json` from `export_root`.
pub fn load_unit_ai(export_root: impl AsRef<Path>) -> Result<Vec<UnitAiProfile>> {
    let (table, label) = load_relative_table(export_root.as_ref(), UNIT_AI_PARAMETER_TABLE)?;
    parse_unit_ai(&table, &label)
}

/// Parse `Unit/DT_UnitParameterCalc` into [`BreakCalcParams`].
pub fn parse_break_calc(table: &DataTable, source_label: &str) -> Result<BreakCalcParams> {
    expect_row_struct(table, "UNIT_PARAMETER_CALC_DATA", source_label)?;

    if table.rows.len() != 1 {
        return Err(DataError::invalid_schema(
            source_label,
            "Rows",
            format!(
                "应包含且仅包含 1 行计算参数，实际有 {} 行",
                table.rows.len()
            ),
        ));
    }
    let (row_key, raw_row) = table
        .rows
        .iter()
        .next()
        .ok_or_else(|| DataError::invalid_schema(source_label, "Rows", "缺少计算参数行"))?;
    let row_prefix = format!("Rows.{row_key}");
    let row = expect_object(raw_row, source_label, &row_prefix)?;

    let break_damage_magnification =
        expect_f64(row, "breakDamageMagnification", source_label, &row_prefix)?;
    let element_damage_multipliers =
        parse_resistance_multiplier_map(row, "elementResistanceValues", source_label, &row_prefix)?;
    let break_element_multipliers = parse_resistance_multiplier_map(
        row,
        "breakElementResistanceValues",
        source_label,
        &row_prefix,
    )?;

    Ok(BreakCalcParams {
        row_key: row_key.clone(),
        break_damage_magnification,
        element_damage_multipliers,
        break_element_multipliers,
    })
}

/// Load `Unit/DT_UnitParameterCalc.json` from `export_root`.
pub fn load_break_calc(export_root: impl AsRef<Path>) -> Result<BreakCalcParams> {
    let (table, label) = load_relative_table(export_root.as_ref(), UNIT_PARAMETER_CALC_TABLE)?;
    parse_break_calc(&table, &label)
}

/// Parse `Encount/DT_BattleGroup` into typed [`BattleGroup`] records sorted by `id` ascending.
///
/// Standalone parsing leaves `is_boss = None`; [`crate::battle::BattleData`] joins member IDs
/// against `DT_UnitParameter` to populate `is_boss`.
pub fn parse_battle_groups(table: &DataTable, source_label: &str) -> Result<Vec<BattleGroup>> {
    expect_row_struct(table, "BATTLE_GROUP_DATA", source_label)?;

    let mut groups = Vec::with_capacity(table.rows.len());
    let mut seen_ids = HashSet::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;

        let id = expect_i64(row, "ID", source_label, &row_prefix)?;
        if !seen_ids.insert(id) {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.ID"),
                format!("重复的战斗组编号 {id}"),
            ));
        }

        let unit_ids = parse_i64_array(row, "UnitIdList", source_label, &row_prefix, false)?;
        let location_ids =
            parse_i64_array(row, "locationIdList", source_label, &row_prefix, false)?;
        let added_passive_skill_ids = parse_added_passive_list(row, source_label, &row_prefix)?;

        groups.push(BattleGroup {
            row_key: row_key.clone(),
            id,
            unit_ids,
            location_ids,
            added_passive_skill_ids,
            can_escape: expect_bool(row, "CanEscape", source_label, &row_prefix)?,
            escape_probability: expect_f64(
                row,
                "probabilityOfSuccessfulEscape",
                source_label,
                &row_prefix,
            )?,
            is_bounty: expect_bool(row, "isBounty", source_label, &row_prefix)?,
            ap: expect_i64(row, "ap", source_label, &row_prefix)?,
            bgm_cue_name: expect_string(row, "bgmCueName", source_label, &row_prefix)?,
            preemptive_relation: expect_string(
                row,
                "preemtiveRelation",
                source_label,
                &row_prefix,
            )?,
            battle_event_id: expect_i64(row, "battleEventId", source_label, &row_prefix)?,
            battle_finish_event_id: expect_string(
                row,
                "battleFinishEventId",
                source_label,
                &row_prefix,
            )?,
            battle_finish_escape_event_id: expect_string(
                row,
                "battleFinishEscapeEventId",
                source_label,
                &row_prefix,
            )?,
            battle_finish_lose_event_id: expect_string(
                row,
                "battleFinishLoseEventId",
                source_label,
                &row_prefix,
            )?,
            is_boss: None,
        });
    }

    groups.sort_by_key(|group| group.id);
    Ok(groups)
}

/// Load `Encount/DT_BattleGroup.json` from `export_root`.
pub fn load_battle_groups(export_root: impl AsRef<Path>) -> Result<Vec<BattleGroup>> {
    let (table, label) = load_relative_table(export_root.as_ref(), BATTLE_GROUP_TABLE)?;
    parse_battle_groups(&table, &label)
}

/// Parse one regional encounter pool table (`ENCOUNT_GROUP_DATA`).
pub fn parse_encounter_pool(
    table: &DataTable,
    source_label: &str,
) -> Result<Vec<EncounterPoolEntry>> {
    expect_row_struct(table, "ENCOUNT_GROUP_DATA", source_label)?;

    let mut entries = Vec::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;
        entries.push(EncounterPoolEntry {
            row_key: row_key.clone(),
            battle_group_id: expect_i64(row, "battleGroupId", source_label, &row_prefix)?,
            encount_ratio: expect_i64(row, "encountRatio", source_label, &row_prefix)?,
        });
    }
    Ok(entries)
}

/// Load one encounter pool table by relative path under `FFRS/Content/Datatable/`
/// (for example `"Encount/01Gra/DT_01Gra_40_01_Encount"`).
pub fn load_encounter_pool(
    export_root: impl AsRef<Path>,
    relative_path: &str,
) -> Result<Vec<EncounterPoolEntry>> {
    let (table, label) = load_relative_table(export_root.as_ref(), relative_path)?;
    parse_encounter_pool(&table, &label)
}

/// Parse `Skill/Other/DT_SkillAttributeDetailData` into a `skillAttrType -> costType` map.
pub fn parse_skill_attribute_costs(
    table: &DataTable,
    source_label: &str,
) -> Result<BTreeMap<SkillAttributeType, SkillCostType>> {
    expect_row_struct(table, "SKILL_ATTRIBUTE_DETAIL_DATA", source_label)?;

    let mut map = BTreeMap::new();
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;

        let attr_raw = expect_string(row, "skillAttrType", source_label, &row_prefix)?;
        let attr = SkillAttributeType::from_ue_str(&attr_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.skillAttrType"),
                format!("未知的技能类型枚举值：{attr_raw}"),
            )
        })?;

        let cost_raw = expect_string(row, "costType", source_label, &row_prefix)?;
        let cost = SkillCostType::from_ue_str(&cost_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.costType"),
                format!("未知的消耗类型枚举值：{cost_raw}"),
            )
        })?;

        if map.insert(attr, cost).is_some() {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.skillAttrType"),
                format!("重复的技能类型配置：{attr_raw}"),
            ));
        }
    }
    Ok(map)
}

/// Load `Skill/Other/DT_SkillAttributeDetailData.json` from `export_root`.
pub fn load_skill_attribute_costs(
    export_root: impl AsRef<Path>,
) -> Result<BTreeMap<SkillAttributeType, SkillCostType>> {
    let (table, label) = load_relative_table(export_root.as_ref(), SKILL_ATTRIBUTE_DETAIL_TABLE)?;
    parse_skill_attribute_costs(&table, &label)
}

/// Parse `Skill/Other/DT_SkillGeneralData` into a `skill_id -> SkillCostOverride` map.
pub fn parse_skill_cost_overrides(
    table: &DataTable,
    source_label: &str,
) -> Result<BTreeMap<i64, SkillCostOverride>> {
    expect_row_struct(table, "SKILL_GENERAL_DATA", source_label)?;

    if table.rows.len() != 1 {
        return Err(DataError::invalid_schema(
            source_label,
            "Rows",
            format!(
                "应包含且仅包含 1 行技能通用配置，实际有 {} 行",
                table.rows.len()
            ),
        ));
    }
    let (row_key, raw_row) = table
        .rows
        .iter()
        .next()
        .ok_or_else(|| DataError::invalid_schema(source_label, "Rows", "缺少技能通用配置行"))?;
    let row_prefix = format!("Rows.{row_key}");
    let row = expect_object(raw_row, source_label, &row_prefix)?;

    let field_path = format!("{row_prefix}.costOverrideData");
    let raw_items = row
        .get("costOverrideData")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;

    let mut overrides = BTreeMap::new();
    for (index, item) in raw_items.iter().enumerate() {
        let item_prefix = format!("{field_path}[{index}]");
        let obj = expect_object(item, source_label, &item_prefix)?;
        let skill_id = expect_i64(obj, "SkillId", source_label, &item_prefix)?;
        let cost_raw = expect_string(obj, "costType", source_label, &item_prefix)?;
        let cost_type = SkillCostType::from_ue_str(&cost_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.costType"),
                format!("未知的消耗类型枚举值：{cost_raw}"),
            )
        })?;
        let cost = expect_i64(obj, "Cost", source_label, &item_prefix)?;
        overrides.insert(
            skill_id,
            SkillCostOverride {
                skill_id,
                cost_type,
                cost,
            },
        );
    }
    Ok(overrides)
}

/// Load `Skill/Other/DT_SkillGeneralData.json` from `export_root`.
pub fn load_skill_cost_overrides(
    export_root: impl AsRef<Path>,
) -> Result<BTreeMap<i64, SkillCostOverride>> {
    let (table, label) = load_relative_table(export_root.as_ref(), SKILL_GENERAL_DATA_TABLE)?;
    parse_skill_cost_overrides(&table, &label)
}

/// Parse `Skill/DT_SkillData` into typed [`Skill`] records sorted by `id` ascending.
///
/// Requires `attr_costs` from `DT_SkillAttributeDetailData` so every skill's `cost_type` is derived
/// strictly from exported game data without guessing.
pub fn parse_skills(
    table: &DataTable,
    attr_costs: &BTreeMap<SkillAttributeType, SkillCostType>,
    cost_overrides: &BTreeMap<i64, SkillCostOverride>,
    source_label: &str,
) -> Result<Vec<Skill>> {
    expect_row_struct(table, "SKILL_DATA", source_label)?;

    let mut skills = Vec::with_capacity(table.rows.len());
    let mut seen_ids = HashSet::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;

        let id = expect_i64(row, "ID", source_label, &row_prefix)?;
        if !seen_ids.insert(id) {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.ID"),
                format!("重复的技能编号 {id}"),
            ));
        }

        let attr_raw = expect_string(row, "skillAttrType", source_label, &row_prefix)?;
        let skill_attr_type = SkillAttributeType::from_ue_str(&attr_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.skillAttrType"),
                format!("未知的技能类型枚举值：{attr_raw}"),
            )
        })?;

        let damage_type_raw = expect_string(row, "DamageType", source_label, &row_prefix)?;
        let damage_type = SkillDamageType::from_ue_str(&damage_type_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.DamageType"),
                format!("未知的伤害类型枚举值：{damage_type_raw}"),
            )
        })?;

        let element_raw = expect_string(row, "element", source_label, &row_prefix)?;
        let element = Element::from_ue_str(&element_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.element"),
                format!("未知的元素属性枚举值：{element_raw}"),
            )
        })?;

        let target_type_raw = expect_string(row, "TargetType", source_label, &row_prefix)?;
        let target_type = SkillTargetType::from_ue_str(&target_type_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.TargetType"),
                format!("未知的目标范围枚举值：{target_type_raw}"),
            )
        })?;

        let target_relation_raw =
            expect_string(row, "defaultTargetRelation", source_label, &row_prefix)?;
        let default_target_relation = TargetRelation::from_ue_str(&target_relation_raw)
            .ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.defaultTargetRelation"),
                    format!("未知的目标阵营枚举值：{target_relation_raw}"),
                )
            })?;

        let target_state_raw = expect_string(row, "defaultTargetState", source_label, &row_prefix)?;
        let default_target_state =
            TargetState::from_ue_str(&target_state_raw).ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.defaultTargetState"),
                    format!("未知的目标状态枚举值：{target_state_raw}"),
                )
            })?;

        let avail_raw = expect_string(row, "availableLocation", source_label, &row_prefix)?;
        let available_location = AvailableLocation::from_ue_str(&avail_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.availableLocation"),
                format!("未知的可用场合枚举值：{avail_raw}"),
            )
        })?;

        let raw_cost = expect_i64(row, "Cost", source_label, &row_prefix)?;
        let (cost, cost_type) = if let Some(override_entry) = cost_overrides.get(&id) {
            (override_entry.cost, override_entry.cost_type)
        } else {
            let mapped = attr_costs.get(&skill_attr_type).copied().ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.skillAttrType"),
                    format!("DT_SkillAttributeDetailData 中缺少技能类型 {attr_raw} 的消耗配置"),
                )
            })?;
            (raw_cost, mapped)
        };

        let belong_command_ids =
            parse_i64_array(row, "belongCommandList", source_label, &row_prefix, true)?;
        let hit_damage_ratios =
            parse_f64_array(row, "hitDamageRatioList", source_label, &row_prefix)?;
        let effect_ids = parse_skill_effect_ids(row, source_label, &row_prefix)?;

        skills.push(Skill {
            row_key: row_key.clone(),
            id,
            sort_id: expect_i64(row, "SortId", source_label, &row_prefix)?,
            name: expect_ftext(row, "Name", source_label, &row_prefix)?,
            description: expect_ftext(row, "Description", source_label, &row_prefix)?,
            has_unit: expect_string(row, "hasUnit", source_label, &row_prefix)?,
            belong_command_ids,
            skill_attr_type,
            damage_type,
            damage_calc_type: expect_string(row, "damageCalcType", source_label, &row_prefix)?,
            element,
            target_type,
            default_target_relation,
            default_target_state,
            cost,
            cost_type,
            power: expect_i64(row, "magnification", source_label, &row_prefix)?,
            break_damage: expect_i64(row, "breakDamageValue", source_label, &row_prefix)?,
            accuracy: expect_i64(row, "accuracy", source_label, &row_prefix)?,
            critical_hit_rate: expect_i64(row, "criticalHitRate", source_label, &row_prefix)?,
            hit_count: expect_i64(row, "hitCount", source_label, &row_prefix)?,
            hit_damage_ratios,
            equip_cost: expect_i64(row, "equipCost", source_label, &row_prefix)?,
            available_location,
            effect_ids,
        });
    }

    skills.sort_by_key(|skill| skill.id);
    Ok(skills)
}

/// Load `Skill/DT_SkillData.json` together with its cost tables from `export_root`.
pub fn load_skills(export_root: impl AsRef<Path>) -> Result<Vec<Skill>> {
    let root = export_root.as_ref();
    let attr_costs = load_skill_attribute_costs(root)?;
    let cost_overrides = load_skill_cost_overrides(root)?;
    let (table, label) = load_relative_table(root, SKILL_DATA_TABLE)?;
    parse_skills(&table, &attr_costs, &cost_overrides, &label)
}

/// Parse `Skill/DT_CommandSkillData` into typed [`CommandSkill`] records sorted by `id` ascending.
pub fn parse_command_skills(table: &DataTable, source_label: &str) -> Result<Vec<CommandSkill>> {
    expect_row_struct(table, "COMMAND_SKILL_DATA", source_label)?;

    let mut commands = Vec::with_capacity(table.rows.len());
    let mut seen_ids = HashSet::with_capacity(table.rows.len());
    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_object(raw_row, source_label, &row_prefix)?;

        let id = expect_i64(row, "ID", source_label, &row_prefix)?;
        if !seen_ids.insert(id) {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{row_prefix}.ID"),
                format!("重复的指令技能编号 {id}"),
            ));
        }

        let raw_unit = expect_i64(row, "unitIdToUseSkill", source_label, &row_prefix)?;
        let unit_id_to_use_skill = (raw_unit >= 0).then_some(raw_unit);

        let icon_prefix = format!("{row_prefix}.SkillIcon");
        let icon_obj = row
            .get("SkillIcon")
            .and_then(Value::as_object)
            .ok_or_else(|| DataError::invalid_schema(source_label, &icon_prefix, "缺少对象"))?;
        let skill_icon = expect_string(icon_obj, "TagName", source_label, &icon_prefix)?;
        let filters = parse_string_array(row, "filters", source_label, &row_prefix)?;

        commands.push(CommandSkill {
            row_key: row_key.clone(),
            id,
            sort_id: expect_i64(row, "SortId", source_label, &row_prefix)?,
            unit_id_to_use_skill,
            name: expect_ftext(row, "Name", source_label, &row_prefix)?,
            description: expect_ftext(row, "Description", source_label, &row_prefix)?,
            skill_icon,
            is_removable: expect_bool(row, "isRemovable", source_label, &row_prefix)?,
            equip_cost: expect_i64(row, "equipCost", source_label, &row_prefix)?,
            filters,
        });
    }

    commands.sort_by_key(|cmd| cmd.id);
    Ok(commands)
}

/// Load `Skill/DT_CommandSkillData.json` from `export_root`.
pub fn load_command_skills(export_root: impl AsRef<Path>) -> Result<Vec<CommandSkill>> {
    let (table, label) = load_relative_table(export_root.as_ref(), COMMAND_SKILL_DATA_TABLE)?;
    parse_command_skills(&table, &label)
}

fn load_relative_table(export_root: &Path, relative_path: &str) -> Result<(DataTable, String)> {
    let path = datatable_relative_path_to_json(relative_path, export_root).ok_or_else(|| {
        DataError::InvalidPath {
            kind: "数据表",
            input: relative_path.to_owned(),
        }
    })?;
    let table = load_data_table(&path)?;
    let label = path.display().to_string();
    Ok((table, label))
}

fn expect_row_struct(table: &DataTable, expected: &str, source_label: &str) -> Result<()> {
    if table.row_struct != expected {
        return Err(DataError::UnexpectedRowStruct {
            path: source_label.to_owned(),
            expected: expected.to_owned(),
            actual: table.row_struct.clone(),
        });
    }
    Ok(())
}

fn expect_object<'a>(
    value: &'a Value,
    source_label: &str,
    prefix: &str,
) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| DataError::invalid_schema(source_label, prefix, "不是对象"))
}

fn expect_i64(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<i64> {
    object.get(field).and_then(Value::as_i64).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是整数")
    })
}

fn expect_f64(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<f64> {
    object.get(field).and_then(Value::as_f64).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是数字")
    })
}

fn expect_string(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是字符串")
        })
}

fn expect_bool(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<bool> {
    object.get(field).and_then(Value::as_bool).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是布尔值")
    })
}

fn expect_ftext(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<FTextValue> {
    object.get(field).and_then(parse_ftext).ok_or_else(|| {
        DataError::invalid_schema(
            source_label,
            format!("{prefix}.{field}"),
            "不是合法的 FText",
        )
    })
}

fn parse_i64_array(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
    skip_negative_sentinel: bool,
) -> Result<Vec<i64>> {
    let field_path = format!("{prefix}.{field}");
    let raw = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;
    let mut out = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let number = item.as_i64().ok_or_else(|| {
            DataError::invalid_schema(source_label, format!("{field_path}[{index}]"), "不是整数")
        })?;
        if skip_negative_sentinel && number < 0 {
            continue;
        }
        out.push(number);
    }
    Ok(out)
}

fn parse_f64_array(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<Vec<f64>> {
    let field_path = format!("{prefix}.{field}");
    let raw = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;
    let mut out = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let number = item.as_f64().ok_or_else(|| {
            DataError::invalid_schema(source_label, format!("{field_path}[{index}]"), "不是数字")
        })?;
        out.push(number);
    }
    Ok(out)
}

fn parse_string_array(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<Vec<String>> {
    let field_path = format!("{prefix}.{field}");
    let raw = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;
    let mut out = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let text = item.as_str().ok_or_else(|| {
            DataError::invalid_schema(source_label, format!("{field_path}[{index}]"), "不是字符串")
        })?;
        out.push(text.to_owned());
    }
    Ok(out)
}

fn parse_ftext_array(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<Vec<FTextValue>> {
    let field_path = format!("{prefix}.{field}");
    let raw = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;
    let mut out = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let ftext = parse_ftext(item).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{field_path}[{index}]"),
                "不是合法的 FText",
            )
        })?;
        out.push(ftext);
    }
    Ok(out)
}

fn parse_element_resistances(
    row: &Map<String, Value>,
    source_label: &str,
    row_prefix: &str,
) -> Result<BTreeMap<Element, Resistance>> {
    let field_path = format!("{row_prefix}.ElementResistanceList");
    let raw = row
        .get("ElementResistanceList")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;

    let mut map = BTreeMap::new();
    for (index, entry) in raw.iter().enumerate() {
        let item_prefix = format!("{field_path}[{index}]");
        let obj = expect_object(entry, source_label, &item_prefix)?;
        let key_raw = expect_string(obj, "Key", source_label, &item_prefix)?;
        let element = Element::from_ue_str(&key_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.Key"),
                format!("未知的元素属性枚举值：{key_raw}"),
            )
        })?;
        if !element.is_combat_element() {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.Key"),
                format!("属性抗性表中出现非战斗属性：{key_raw}"),
            ));
        }

        let val_raw = expect_string(obj, "Value", source_label, &item_prefix)?;
        let resistance = Resistance::from_ue_str(&val_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.Value"),
                format!("未知的抗性枚举值：{val_raw}"),
            )
        })?;
        if map.insert(element, resistance).is_some() {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.Key"),
                format!("属性抗性表中重复出现属性：{key_raw}"),
            ));
        }
    }

    for required in ALL_COMBAT_ELEMENTS {
        if !map.contains_key(&required) {
            return Err(DataError::invalid_schema(
                source_label,
                &field_path,
                format!("属性抗性表缺少属性 {}", required.as_ue_str()),
            ));
        }
    }

    Ok(map)
}

fn parse_status_resistances(
    row: &Map<String, Value>,
    source_label: &str,
    row_prefix: &str,
) -> Result<BTreeMap<String, i64>> {
    let field_path = format!("{row_prefix}.StatusConditionResistanceList");
    let raw = row
        .get("StatusConditionResistanceList")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;

    let mut map = BTreeMap::new();
    for (index, entry) in raw.iter().enumerate() {
        let item_prefix = format!("{field_path}[{index}]");
        let obj = expect_object(entry, source_label, &item_prefix)?;
        let key = expect_string(obj, "Key", source_label, &item_prefix)?;
        let val = expect_i64(obj, "Value", source_label, &item_prefix)?;
        if map.insert(key.clone(), val).is_some() {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.Key"),
                format!("异常状态抗性表中重复出现键：{key}"),
            ));
        }
    }
    Ok(map)
}

fn parse_resistance_multiplier_map(
    row: &Map<String, Value>,
    field: &str,
    source_label: &str,
    row_prefix: &str,
) -> Result<BTreeMap<Resistance, f64>> {
    let field_path = format!("{row_prefix}.{field}");
    let raw = row
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;

    let mut map = BTreeMap::new();
    for (index, entry) in raw.iter().enumerate() {
        let item_prefix = format!("{field_path}[{index}]");
        let obj = expect_object(entry, source_label, &item_prefix)?;
        let key_raw = expect_string(obj, "Key", source_label, &item_prefix)?;
        let resistance = Resistance::from_ue_str(&key_raw).ok_or_else(|| {
            DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.Key"),
                format!("未知的抗性枚举值：{key_raw}"),
            )
        })?;
        let val = expect_f64(obj, "Value", source_label, &item_prefix)?;
        if map.insert(resistance, val).is_some() {
            return Err(DataError::invalid_schema(
                source_label,
                format!("{item_prefix}.Key"),
                format!("重复的抗性倍率键：{key_raw}"),
            ));
        }
    }

    for required in [
        Resistance::Weakness,
        Resistance::None,
        Resistance::Halved,
        Resistance::Invalid,
        Resistance::Absorb,
    ] {
        if !map.contains_key(&required) {
            return Err(DataError::invalid_schema(
                source_label,
                &field_path,
                format!("缺少抗性 {} 的倍率定义", required.as_ue_str()),
            ));
        }
    }

    Ok(map)
}

fn parse_added_passive_list(
    row: &Map<String, Value>,
    source_label: &str,
    row_prefix: &str,
) -> Result<Vec<Vec<i64>>> {
    let field_path = format!("{row_prefix}.addPassiveDataList");
    let raw = row
        .get("addPassiveDataList")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;
    let mut out = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let item_prefix = format!("{field_path}[{index}]");
        let obj = expect_object(item, source_label, &item_prefix)?;
        let passives = parse_i64_array(obj, "passiveSkillList", source_label, &item_prefix, true)?;
        out.push(passives);
    }
    Ok(out)
}

fn parse_skill_effect_ids(
    row: &Map<String, Value>,
    source_label: &str,
    row_prefix: &str,
) -> Result<Vec<i64>> {
    let field_path = format!("{row_prefix}.effectBundleList");
    let raw = row
        .get("effectBundleList")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::invalid_schema(source_label, &field_path, "不是数组"))?;
    let mut effect_ids = Vec::new();
    for (index, item) in raw.iter().enumerate() {
        let item_prefix = format!("{field_path}[{index}]");
        let obj = expect_object(item, source_label, &item_prefix)?;
        let effect_id = expect_i64(obj, "effectId", source_label, &item_prefix)?;
        if effect_id >= 0 {
            effect_ids.push(effect_id);
        }
    }
    Ok(effect_ids)
}
