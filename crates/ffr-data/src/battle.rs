//! Typed static battle data for FINAL FANTASY RESONANCE (`ffr-data::battle`).
//!
//! Covers units and enemy profiles (`DT_UnitParameter`, `DT_PlayableUnitParameter`,
//! `DT_BtlUnitAIParameter`, `DT_UnitParameterCalc`), battle groups and encounter pools
//! (`DT_BattleGroup`, `ENCOUNT_GROUP_DATA`), and skills (`DT_SkillData`,
//! `DT_SkillAttributeDetailData`, `DT_SkillGeneralData`).
//!
//! Break/shield values come directly from `DT_UnitParameter.MaxBreakPoint` (numeric break gauge,
//! not hit-count shields) together with `DT_SkillData.breakDamageValue` and
//! `DT_UnitParameterCalc.breakElementResistanceValues`. Weapon-type weaknesses do not exist in
//! the exported data; weaknesses are strictly elemental.

mod chapters;
mod parse;
mod query;
mod types;

pub use chapters::{EnemyChapterIndex, EnemyChapterSource, EnemyChapterSourceKind};
pub use parse::{
    load_battle_groups, load_break_calc, load_command_skills, load_encounter_pool,
    load_playable_unit_parameters, load_skill_attribute_costs, load_skill_cost_overrides,
    load_skills, load_unit_ai, load_units, parse_battle_groups, parse_break_calc,
    parse_command_skills, parse_encounter_pool, parse_playable_unit_parameters,
    parse_skill_attribute_costs, parse_skill_cost_overrides, parse_skills, parse_unit_ai,
    parse_units,
};
pub use query::{BattleData, CommandSkillData, enemy_summary};
pub use types::{
    ALL_COMBAT_ELEMENTS, ArmorType, AvailableLocation, BattleGroup, BreakCalcParams, CommandSkill,
    Element, EncounterPoolEntry, EnemyProfile, PlayableUnitProfile, Resistance,
    ResolvedBattleGroup, Skill, SkillAttributeType, SkillCostOverride, SkillCostType,
    SkillDamageType, SkillTargetType, TargetRelation, TargetState, Unit, UnitAiProfile,
    UnitCategory, UnitRace, WeaponType,
};

/// Relative path of `DT_UnitParameter` under `FFRS/Content/Datatable/`.
pub const UNIT_PARAMETER_TABLE: &str = "Unit/DT_UnitParameter";
/// Relative path of `DT_PlayableUnitParameter` under `FFRS/Content/Datatable/`.
pub const PLAYABLE_UNIT_PARAMETER_TABLE: &str = "Unit/DT_PlayableUnitParameter";
/// Relative path of `DT_BtlUnitAIParameter` under `FFRS/Content/Datatable/`.
pub const UNIT_AI_PARAMETER_TABLE: &str = "Unit/AI/DT_BtlUnitAIParameter";
/// Relative path of `DT_UnitParameterCalc` under `FFRS/Content/Datatable/`.
pub const UNIT_PARAMETER_CALC_TABLE: &str = "Unit/DT_UnitParameterCalc";
/// Relative path of `DT_BattleGroup` under `FFRS/Content/Datatable/`.
pub const BATTLE_GROUP_TABLE: &str = "Encount/DT_BattleGroup";
/// Relative path of `DT_BattleUnitLayout` under `FFRS/Content/Datatable/`.
pub const BATTLE_UNIT_LAYOUT_TABLE: &str = "Encount/DT_BattleUnitLayout";
/// Relative path of `DT_SkillData` under `FFRS/Content/Datatable/`.
pub const SKILL_DATA_TABLE: &str = "Skill/DT_SkillData";
/// Relative path of `DT_CommandSkillData` under `FFRS/Content/Datatable/`.
pub const COMMAND_SKILL_DATA_TABLE: &str = "Skill/DT_CommandSkillData";
/// Relative path of `DT_SkillAttributeDetailData` under `FFRS/Content/Datatable/`.
pub const SKILL_ATTRIBUTE_DETAIL_TABLE: &str = "Skill/Other/DT_SkillAttributeDetailData";
/// Relative path of `DT_SkillGeneralData` under `FFRS/Content/Datatable/`.
pub const SKILL_GENERAL_DATA_TABLE: &str = "Skill/Other/DT_SkillGeneralData";
