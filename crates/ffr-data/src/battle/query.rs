//! Indexed queries and model-facing summaries over loaded static battle tables.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::battle::parse::{
    parse_battle_groups, parse_break_calc, parse_command_skills, parse_skill_attribute_costs,
    parse_skill_cost_overrides, parse_skills, parse_unit_ai, parse_units,
};
use crate::battle::types::{
    BattleGroup, BreakCalcParams, CommandSkill, Element, EnemyProfile, Resistance,
    ResolvedBattleGroup, Skill, Unit, UnitAiProfile, UnitCategory,
};
use crate::battle::{
    BATTLE_GROUP_TABLE, COMMAND_SKILL_DATA_TABLE, SKILL_ATTRIBUTE_DETAIL_TABLE, SKILL_DATA_TABLE,
    SKILL_GENERAL_DATA_TABLE, UNIT_AI_PARAMETER_TABLE, UNIT_PARAMETER_CALC_TABLE,
    UNIT_PARAMETER_TABLE,
};
use crate::error::{DataError, Result};
use crate::ftext::{
    MODEL_LANGUAGE, ResolveOptions, ResolvedText, TextResolver, TextResolverOptions, TextSource,
};
use crate::paths::{datatable_relative_path_to_json, is_safe_language_tag};
use crate::store::DataStore;

/// Loaded static battle tables paired with a cached [`TextResolver`].
#[derive(Debug, Clone)]
pub struct BattleData {
    pub units: Vec<Unit>,
    pub battle_groups: Vec<BattleGroup>,
    pub skills: Vec<Skill>,
    pub unit_ai: BTreeMap<i64, UnitAiProfile>,
    pub break_calc: BreakCalcParams,
    pub text_resolver: TextResolver,
    unit_index: BTreeMap<i64, usize>,
    battle_group_index: BTreeMap<i64, usize>,
    skill_index: BTreeMap<i64, usize>,
}

impl BattleData {
    /// Load all core battle tables from `export_root` with default text options
    /// (`language = "zh-Hans"`, strict missing reporting).
    pub fn load(export_root: impl Into<PathBuf>) -> Result<Self> {
        Self::load_with_options(export_root, TextResolverOptions::default())
    }

    /// Load all core battle tables with custom [`TextResolverOptions`].
    pub fn load_with_options(
        export_root: impl Into<PathBuf>,
        options: TextResolverOptions,
    ) -> Result<Self> {
        let store = DataStore::with_text_options(export_root, options);
        Self::from_store(&store)
    }

    /// Build [`BattleData`] through a [`DataStore`], reusing its table and text caches.
    pub fn from_store(store: &DataStore) -> Result<Self> {
        let units = {
            let table = store.data_table(UNIT_PARAMETER_TABLE)?;
            let label = table_label(store, UNIT_PARAMETER_TABLE);
            parse_units(&table, &label)?
        };
        let ai_list = {
            let table = store.data_table(UNIT_AI_PARAMETER_TABLE)?;
            let label = table_label(store, UNIT_AI_PARAMETER_TABLE);
            parse_unit_ai(&table, &label)?
        };
        let break_calc = {
            let table = store.data_table(UNIT_PARAMETER_CALC_TABLE)?;
            let label = table_label(store, UNIT_PARAMETER_CALC_TABLE);
            parse_break_calc(&table, &label)?
        };
        let mut battle_groups = {
            let table = store.data_table(BATTLE_GROUP_TABLE)?;
            let label = table_label(store, BATTLE_GROUP_TABLE);
            parse_battle_groups(&table, &label)?
        };
        let attr_costs = {
            let table = store.data_table(SKILL_ATTRIBUTE_DETAIL_TABLE)?;
            let label = table_label(store, SKILL_ATTRIBUTE_DETAIL_TABLE);
            parse_skill_attribute_costs(&table, &label)?
        };
        let cost_overrides = {
            let table = store.data_table(SKILL_GENERAL_DATA_TABLE)?;
            let label = table_label(store, SKILL_GENERAL_DATA_TABLE);
            parse_skill_cost_overrides(&table, &label)?
        };
        let skills = {
            let table = store.data_table(SKILL_DATA_TABLE)?;
            let label = table_label(store, SKILL_DATA_TABLE);
            parse_skills(&table, &attr_costs, &cost_overrides, &label)?
        };

        let unit_index: BTreeMap<i64, usize> = units
            .iter()
            .enumerate()
            .map(|(index, unit)| (unit.id, index))
            .collect();

        for group in &mut battle_groups {
            let mut all_resolved = !group.unit_ids.is_empty();
            let mut has_boss = false;
            for &unit_id in &group.unit_ids {
                if let Some(&idx) = unit_index.get(&unit_id) {
                    if units[idx].is_boss {
                        has_boss = true;
                    }
                } else {
                    all_resolved = false;
                    break;
                }
            }
            group.is_boss = if all_resolved { Some(has_boss) } else { None };
        }

        let battle_group_index: BTreeMap<i64, usize> = battle_groups
            .iter()
            .enumerate()
            .map(|(index, group)| (group.id, index))
            .collect();
        let skill_index: BTreeMap<i64, usize> = skills
            .iter()
            .enumerate()
            .map(|(index, skill)| (skill.id, index))
            .collect();
        let unit_ai: BTreeMap<i64, UnitAiProfile> = ai_list
            .into_iter()
            .map(|profile| (profile.id, profile))
            .collect();

        Ok(Self {
            units,
            battle_groups,
            skills,
            unit_ai,
            break_calc,
            text_resolver: store.text_resolver().clone(),
            unit_index,
            battle_group_index,
            skill_index,
        })
    }

    /// Look up one unit by `ID`.
    pub fn unit_by_id(&self, id: i64) -> Option<&Unit> {
        self.unit_index.get(&id).map(|&idx| &self.units[idx])
    }

    /// Look up one unit by `ID`, returning a Chinese [`DataError`] if absent.
    pub fn require_unit(&self, id: i64) -> Result<&Unit> {
        self.unit_by_id(id).ok_or_else(|| {
            DataError::invalid_schema(
                UNIT_PARAMETER_TABLE,
                format!("ID={id}"),
                format!("单位表中找不到编号为 {id} 的单位"),
            )
        })
    }

    /// Look up a unit's AI record by `ID`.
    pub fn unit_ai_by_id(&self, id: i64) -> Option<&UnitAiProfile> {
        self.unit_ai.get(&id)
    }

    /// Build a battle-focused [`EnemyProfile`] for unit `id`, failing if `id` does not exist.
    pub fn enemy_profile(&self, id: i64) -> Result<EnemyProfile> {
        let unit = self.require_unit(id)?;
        Ok(unit.to_enemy_profile(self.unit_ai_by_id(id)))
    }

    /// Look up one battle group by `ID`.
    pub fn battle_group_by_id(&self, id: i64) -> Option<&BattleGroup> {
        self.battle_group_index
            .get(&id)
            .map(|&idx| &self.battle_groups[idx])
    }

    /// Resolve a battle group and all of its member units into [`EnemyProfile`] records.
    ///
    /// Returns an error if the battle group ID does not exist, has no members, or references any
    /// unit ID missing from `DT_UnitParameter`.
    pub fn resolve_battle_group(&self, id: i64) -> Result<ResolvedBattleGroup> {
        let group = self.battle_group_by_id(id).ok_or_else(|| {
            DataError::invalid_schema(
                BATTLE_GROUP_TABLE,
                format!("ID={id}"),
                format!("战斗组表中找不到编号为 {id} 的战斗组"),
            )
        })?;
        if group.unit_ids.is_empty() {
            return Err(DataError::invalid_schema(
                BATTLE_GROUP_TABLE,
                format!("Rows.{}.UnitIdList", group.row_key),
                format!("战斗组 {id} 没有配置任何成员单位"),
            ));
        }

        let mut members = Vec::with_capacity(group.unit_ids.len());
        let mut is_boss = false;
        for &unit_id in &group.unit_ids {
            let unit = self.unit_by_id(unit_id).ok_or_else(|| {
                DataError::invalid_schema(
                    BATTLE_GROUP_TABLE,
                    format!("Rows.{}.UnitIdList", group.row_key),
                    format!("战斗组 {id} 引用的单位编号 {unit_id} 在单位表中不存在"),
                )
            })?;
            if unit.is_boss {
                is_boss = true;
            }
            members.push(unit.to_enemy_profile(self.unit_ai_by_id(unit_id)));
        }

        Ok(ResolvedBattleGroup {
            group: group.clone(),
            members,
            is_boss,
        })
    }

    /// Look up one skill by `ID`.
    pub fn skill_by_id(&self, id: i64) -> Option<&Skill> {
        self.skill_index.get(&id).map(|&idx| &self.skills[idx])
    }

    /// Look up one skill by `ID`, returning a Chinese [`DataError`] if absent.
    pub fn require_skill(&self, id: i64) -> Result<&Skill> {
        self.skill_by_id(id).ok_or_else(|| {
            DataError::invalid_schema(
                SKILL_DATA_TABLE,
                format!("ID={id}"),
                format!("技能表中找不到编号为 {id} 的技能"),
            )
        })
    }

    /// Return all active skills belonging to battle command category `command_id`
    /// (`DT_SkillData.belongCommandList` contains `command_id`).
    pub fn skills_in_command(&self, command_id: i64) -> Vec<&Skill> {
        self.skills
            .iter()
            .filter(|skill| skill.belong_command_ids.contains(&command_id))
            .collect()
    }

    /// Resolve a unit's display name in the default language (`"zh-Hans"`).
    pub fn unit_name(&self, id: i64) -> ResolvedText {
        self.unit_name_in(id, self.text_resolver.language())
    }

    /// Resolve a unit's display name in an explicit language.
    pub fn unit_name_in(&self, id: i64, lang: &str) -> ResolvedText {
        let Some(unit) = self.unit_by_id(id) else {
            return ResolvedText {
                text: None,
                lang: None,
                requested_language: lang.to_owned(),
                reason: Some(format!("单位表中找不到编号为 {id} 的单位")),
                source: TextSource::Missing,
            };
        };
        self.text_resolver
            .resolve_with(&unit.name, Some(lang), ResolveOptions::strict())
    }

    /// Resolve a skill's display name in the default language (`"zh-Hans"`).
    pub fn skill_name(&self, id: i64) -> ResolvedText {
        self.skill_name_in(id, self.text_resolver.language())
    }

    /// Resolve a skill's display name in an explicit language.
    pub fn skill_name_in(&self, id: i64, lang: &str) -> ResolvedText {
        let Some(skill) = self.skill_by_id(id) else {
            return ResolvedText {
                text: None,
                lang: None,
                requested_language: lang.to_owned(),
                reason: Some(format!("技能表中找不到编号为 {id} 的技能")),
                source: TextSource::Missing,
            };
        };
        self.text_resolver
            .resolve_with(&skill.name, Some(lang), ResolveOptions::strict())
    }

    /// Find all units matching exact display name `name` in `"zh-Hans"`.
    pub fn units_by_name(&self, name: &str) -> Result<Vec<&Unit>> {
        self.units_by_name_in(name, MODEL_LANGUAGE)
    }

    /// Find all units matching exact display name `name` in `lang`.
    ///
    /// Fails loudly if `lang` is unsafe or its localization file cannot be read.
    pub fn units_by_name_in(&self, name: &str, lang: &str) -> Result<Vec<&Unit>> {
        self.ensure_language_ready(lang)?;
        let mut matched = Vec::new();
        for unit in &self.units {
            let resolved =
                self.text_resolver
                    .resolve_with(&unit.name, Some(lang), ResolveOptions::strict());
            if resolved.text.as_deref() == Some(name) {
                matched.push(unit);
            }
        }
        Ok(matched)
    }

    /// Find all combat enemy/boss profiles matching exact display name `name` in `"zh-Hans"`.
    pub fn enemies_by_name(&self, name: &str) -> Result<Vec<EnemyProfile>> {
        self.enemies_by_name_in(name, MODEL_LANGUAGE)
    }

    /// Find all combat enemy/boss profiles matching exact display name `name` in `lang`.
    pub fn enemies_by_name_in(&self, name: &str, lang: &str) -> Result<Vec<EnemyProfile>> {
        let units = self.units_by_name_in(name, lang)?;
        Ok(units
            .into_iter()
            .filter(|unit| unit.category.is_enemy_category())
            .map(|unit| unit.to_enemy_profile(self.unit_ai_by_id(unit.id)))
            .collect())
    }

    /// Find all skills matching exact display name `name` in `"zh-Hans"`.
    pub fn skills_by_name(&self, name: &str) -> Result<Vec<&Skill>> {
        self.skills_by_name_in(name, MODEL_LANGUAGE)
    }

    /// Find all skills matching exact display name `name` in `lang`.
    ///
    /// Fails loudly if `lang` is unsafe or its localization file cannot be read.
    pub fn skills_by_name_in(&self, name: &str, lang: &str) -> Result<Vec<&Skill>> {
        self.ensure_language_ready(lang)?;
        let mut matched = Vec::new();
        for skill in &self.skills {
            let resolved =
                self.text_resolver
                    .resolve_with(&skill.name, Some(lang), ResolveOptions::strict());
            if resolved.text.as_deref() == Some(name) {
                matched.push(skill);
            }
        }
        Ok(matched)
    }

    /// Build a concise Chinese summary paragraph for the AI model describing one enemy's
    /// weaknesses, resistances, break gauge (shield), boss status, and high-risk skills.
    ///
    /// Resolves unit and skill display names in `lang` strictly: missing unit IDs, unsafe language
    /// tags, missing localization files, or missing translations return a Chinese [`DataError`].
    pub fn enemy_summary(&self, id: i64, lang: &str) -> Result<String> {
        self.ensure_language_ready(lang)?;
        let unit = self.require_unit(id)?;
        let resolved_name =
            self.text_resolver
                .resolve_with(&unit.name, Some(lang), ResolveOptions::strict());
        let Some(display_name) = resolved_name.text else {
            let reason = resolved_name
                .reason
                .unwrap_or_else(|| "缺少对应语言译名".to_owned());
            return Err(DataError::invalid_schema(
                UNIT_PARAMETER_TABLE,
                format!("Rows.{}.Name", unit.row_key),
                format!("无法读取单位 {id} 的 {lang} 名称：{reason}"),
            ));
        };

        let profile = unit.to_enemy_profile(self.unit_ai_by_id(id));
        let boss_label = if profile.is_boss {
            "Boss"
        } else {
            match profile.category {
                UnitCategory::Boss => "非Boss（大型/稀有分类）",
                UnitCategory::Enemy => "普通敌人",
                UnitCategory::Playable => "可操控角色",
                UnitCategory::Beast => "召唤兽",
                UnitCategory::Vision => "幻兵",
            }
        };

        let weak_text = format_elements_zh(&profile.weaknesses);
        let resist_text = format_resistances_zh(&profile);
        let weak_break_mult = self
            .break_calc
            .break_element_multipliers
            .get(&Resistance::Weakness)
            .copied()
            .ok_or_else(|| {
                DataError::invalid_schema(
                    UNIT_PARAMETER_CALC_TABLE,
                    "breakElementResistanceValues",
                    "缺少 Weakness 破防倍率",
                )
            })?;

        let mut summary = format!(
            "{display_name}（编号 {id}，Lv.{}，{boss_label}）：HP {}，破防槽（盾值）{}（弱点破防倍率 {:.1}x，破防增伤 {:.0}%）；弱点：{}；抗性：{}。",
            profile.level,
            profile.max_hp,
            profile.max_break_point,
            weak_break_mult,
            self.break_calc.break_damage_magnification,
            weak_text,
            resist_text,
        );

        if !profile.high_risk_skill_ids.is_empty() {
            let mut skill_labels = Vec::with_capacity(profile.high_risk_skill_ids.len());
            for &skill_id in &profile.high_risk_skill_ids {
                if let Some(skill) = self.skill_by_id(skill_id) {
                    let skill_name = self.text_resolver.resolve_with(
                        &skill.name,
                        Some(lang),
                        ResolveOptions::strict(),
                    );
                    if let Some(text) = skill_name.text {
                        skill_labels.push(format!("{text}（编号 {skill_id}）"));
                    } else {
                        skill_labels.push(format!("编号 {skill_id}（缺少 {lang} 译名）"));
                    }
                } else {
                    skill_labels.push(format!("编号 {skill_id}（技能表未收录）"));
                }
            }
            summary.push_str(&format!(
                "高危/斗气技能：{}（可用破防打断）。",
                skill_labels.join("、")
            ));
        }

        Ok(summary)
    }

    fn ensure_language_ready(&self, lang: &str) -> Result<()> {
        if !is_safe_language_tag(lang) {
            return Err(DataError::InvalidLanguage {
                language: lang.to_owned(),
            });
        }
        if lang != self.text_resolver.native_language() {
            self.text_resolver.load_localization_strict(lang)?;
        }
        Ok(())
    }
}

/// Indexed battle command-skill catalog (`Skill/DT_CommandSkillData`) backed by [`TextResolver`].
#[derive(Debug, Clone)]
pub struct CommandSkillData {
    pub commands: Vec<CommandSkill>,
    pub text_resolver: TextResolver,
    command_index: BTreeMap<i64, usize>,
}

impl CommandSkillData {
    /// Load `Skill/DT_CommandSkillData.json` from `export_root` with default `"zh-Hans"` options.
    pub fn load(export_root: impl Into<PathBuf>) -> Result<Self> {
        Self::load_with_options(export_root, TextResolverOptions::default())
    }

    /// Load `Skill/DT_CommandSkillData.json` with custom [`TextResolverOptions`].
    pub fn load_with_options(
        export_root: impl Into<PathBuf>,
        options: TextResolverOptions,
    ) -> Result<Self> {
        let store = DataStore::with_text_options(export_root, options);
        Self::from_store(&store)
    }

    /// Build [`CommandSkillData`] through an existing [`DataStore`], reusing its caches.
    pub fn from_store(store: &DataStore) -> Result<Self> {
        let table = store.data_table(COMMAND_SKILL_DATA_TABLE)?;
        let label = table_label(store, COMMAND_SKILL_DATA_TABLE);
        let commands = parse_command_skills(&table, &label)?;
        let command_index: BTreeMap<i64, usize> = commands
            .iter()
            .enumerate()
            .filter(|(_, cmd)| !cmd.is_placeholder())
            .map(|(index, cmd)| (cmd.id, index))
            .collect();
        Ok(Self {
            commands,
            text_resolver: store.text_resolver().clone(),
            command_index,
        })
    }

    /// Look up one non-placeholder command skill by `ID` (`ID >= 0`).
    pub fn command_by_id(&self, id: i64) -> Option<&CommandSkill> {
        self.command_index.get(&id).map(|&idx| &self.commands[idx])
    }

    /// Look up one non-placeholder command skill by `ID`, returning a Chinese [`DataError`] if absent.
    pub fn require_command(&self, id: i64) -> Result<&CommandSkill> {
        self.command_by_id(id).ok_or_else(|| {
            DataError::invalid_schema(
                COMMAND_SKILL_DATA_TABLE,
                format!("ID={id}"),
                format!("指令技能表中找不到编号为 {id} 的指令技能"),
            )
        })
    }

    /// Resolve a command skill's display name in the resolver's default language (`"zh-Hans"`).
    pub fn command_name(&self, id: i64) -> ResolvedText {
        self.command_name_in(id, self.text_resolver.language())
    }

    /// Resolve a command skill's display name in an explicit language (strict mode).
    pub fn command_name_in(&self, id: i64, lang: &str) -> ResolvedText {
        let Some(cmd) = self.command_by_id(id) else {
            return ResolvedText {
                text: None,
                lang: None,
                requested_language: lang.to_owned(),
                reason: Some(format!("指令技能表中找不到编号为 {id} 的指令技能")),
                source: TextSource::Missing,
            };
        };
        self.text_resolver
            .resolve_with(&cmd.name, Some(lang), ResolveOptions::strict())
    }

    /// Resolve a command skill's description in the resolver's default language (`"zh-Hans"`).
    pub fn command_description(&self, id: i64) -> ResolvedText {
        self.command_description_in(id, self.text_resolver.language())
    }

    /// Resolve a command skill's description in an explicit language (strict mode).
    pub fn command_description_in(&self, id: i64, lang: &str) -> ResolvedText {
        let Some(cmd) = self.command_by_id(id) else {
            return ResolvedText {
                text: None,
                lang: None,
                requested_language: lang.to_owned(),
                reason: Some(format!("指令技能表中找不到编号为 {id} 的指令技能")),
                source: TextSource::Missing,
            };
        };
        self.text_resolver
            .resolve_with(&cmd.description, Some(lang), ResolveOptions::strict())
    }

    /// Find all non-placeholder command skills matching exact display name in `"zh-Hans"`.
    pub fn commands_by_name(&self, name: &str) -> Result<Vec<&CommandSkill>> {
        self.commands_by_name_in(name, MODEL_LANGUAGE)
    }

    /// Find all non-placeholder command skills matching exact display name in `lang`.
    pub fn commands_by_name_in(&self, name: &str, lang: &str) -> Result<Vec<&CommandSkill>> {
        if !is_safe_language_tag(lang) {
            return Err(DataError::InvalidLanguage {
                language: lang.to_owned(),
            });
        }
        if lang != self.text_resolver.native_language() {
            self.text_resolver.load_localization_strict(lang)?;
        }
        let mut matched = Vec::new();
        for cmd in &self.commands {
            if cmd.is_placeholder() {
                continue;
            }
            let resolved =
                self.text_resolver
                    .resolve_with(&cmd.name, Some(lang), ResolveOptions::strict());
            if resolved.text.as_deref() == Some(name) {
                matched.push(cmd);
            }
        }
        Ok(matched)
    }

    /// Return all command skills tied to a specific vision/unit ID (`unitIdToUseSkill == Some(id)`).
    pub fn commands_for_unit(&self, unit_id: i64) -> Vec<&CommandSkill> {
        self.commands
            .iter()
            .filter(|cmd| !cmd.is_placeholder() && cmd.unit_id_to_use_skill == Some(unit_id))
            .collect()
    }
}

impl DataStore {
    /// Build a [`BattleData`] handle backed by this store's cached tables and [`TextResolver`].
    pub fn battle(&self) -> Result<BattleData> {
        BattleData::from_store(self)
    }

    /// Build a [`CommandSkillData`] handle backed by this store's cached `Skill/DT_CommandSkillData`.
    pub fn command_skills(&self) -> Result<CommandSkillData> {
        CommandSkillData::from_store(self)
    }
}

/// Top-level convenience function returning a concise Chinese enemy summary for the AI model.
///
/// Equivalent to [`BattleData::enemy_summary`].
pub fn enemy_summary(data: &BattleData, id: i64, lang: &str) -> Result<String> {
    data.enemy_summary(id, lang)
}

fn table_label(store: &DataStore, relative_path: &str) -> String {
    datatable_relative_path_to_json(relative_path, store.export_root()).map_or_else(
        || relative_path.to_owned(),
        |path| path.display().to_string(),
    )
}

fn format_elements_zh(elements: &[Element]) -> String {
    if elements.is_empty() {
        return "无".to_owned();
    }
    elements
        .iter()
        .map(|element| element.localized_name("zh-Hans").unwrap_or("未知属性"))
        .collect::<Vec<_>>()
        .join("、")
}

fn format_resistances_zh(profile: &EnemyProfile) -> String {
    let mut parts = Vec::new();
    if !profile.halved.is_empty() {
        parts.push(format!("减半({})", format_elements_zh(&profile.halved)));
    }
    if !profile.invalid.is_empty() {
        parts.push(format!("无效({})", format_elements_zh(&profile.invalid)));
    }
    if !profile.absorb.is_empty() {
        parts.push(format!("吸收({})", format_elements_zh(&profile.absorb)));
    }
    if parts.is_empty() {
        "无".to_owned()
    } else {
        parts.join("；")
    }
}
