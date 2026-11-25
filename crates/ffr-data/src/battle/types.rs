//! Typed enums and structs for static battle data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ftext::FTextValue;

/// All eight combat elements in canonical game order.
pub const ALL_COMBAT_ELEMENTS: [Element; 8] = [
    Element::Fire,
    Element::Ice,
    Element::Wind,
    Element::Earth,
    Element::Thunder,
    Element::Water,
    Element::Light,
    Element::Dark,
];

/// Combat element (`eElement::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Element {
    Fire,
    Ice,
    Wind,
    Earth,
    Thunder,
    Water,
    Light,
    Dark,
    None,
}

impl Element {
    /// Parse an Unreal Engine `eElement::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eElement::Fire" => Some(Self::Fire),
            "eElement::Ice" => Some(Self::Ice),
            "eElement::Wind" => Some(Self::Wind),
            "eElement::Earth" => Some(Self::Earth),
            "eElement::Thunder" => Some(Self::Thunder),
            "eElement::Water" => Some(Self::Water),
            "eElement::Light" => Some(Self::Light),
            "eElement::Dark" => Some(Self::Dark),
            "eElement::None" => Some(Self::None),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum representation.
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::Fire => "eElement::Fire",
            Self::Ice => "eElement::Ice",
            Self::Wind => "eElement::Wind",
            Self::Earth => "eElement::Earth",
            Self::Thunder => "eElement::Thunder",
            Self::Water => "eElement::Water",
            Self::Light => "eElement::Light",
            Self::Dark => "eElement::Dark",
            Self::None => "eElement::None",
        }
    }

    /// Returns `true` for the eight combat elements (`Fire..=Dark`), `false` for `None`.
    pub const fn is_combat_element(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Localized display name for supported languages (`"zh-Hans"`, `"en"`, `"ja"`).
    ///
    /// Exported `StringTable` assets do not include a standalone element name table; these names
    /// match `ST_BtlCommon/BtlCommon0031..0038` in the exported locres files.
    pub fn localized_name(self, lang: &str) -> Option<&'static str> {
        match lang {
            "zh-Hans" => Some(match self {
                Self::Fire => "火",
                Self::Ice => "冰",
                Self::Wind => "风",
                Self::Earth => "土",
                Self::Thunder => "雷",
                Self::Water => "水",
                Self::Light => "光",
                Self::Dark => "暗",
                Self::None => "无属性",
            }),
            "en" => Some(match self {
                Self::Fire => "Fire",
                Self::Ice => "Ice",
                Self::Wind => "Wind",
                Self::Earth => "Earth",
                Self::Thunder => "Lightning",
                Self::Water => "Water",
                Self::Light => "Light",
                Self::Dark => "Dark",
                Self::None => "None",
            }),
            "ja" => Some(match self {
                Self::Fire => "火",
                Self::Ice => "氷",
                Self::Wind => "風",
                Self::Earth => "土",
                Self::Thunder => "雷",
                Self::Water => "水",
                Self::Light => "光",
                Self::Dark => "闇",
                Self::None => "無属性",
            }),
            _ => None,
        }
    }
}

/// Elemental affinity / resistance level (`eResistance::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Resistance {
    Weakness,
    None,
    Halved,
    Invalid,
    Absorb,
}

impl Resistance {
    /// Parse an Unreal Engine `eResistance::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eResistance::Weakness" => Some(Self::Weakness),
            "eResistance::None" => Some(Self::None),
            "eResistance::Halved" => Some(Self::Halved),
            "eResistance::Invalid" => Some(Self::Invalid),
            "eResistance::Absorb" => Some(Self::Absorb),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum representation.
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::Weakness => "eResistance::Weakness",
            Self::None => "eResistance::None",
            Self::Halved => "eResistance::Halved",
            Self::Invalid => "eResistance::Invalid",
            Self::Absorb => "eResistance::Absorb",
        }
    }

    /// Localized display label for supported languages (`"zh-Hans"`, `"en"`, `"ja"`).
    pub fn localized_name(self, lang: &str) -> Option<&'static str> {
        match lang {
            "zh-Hans" => Some(match self {
                Self::Weakness => "弱点",
                Self::None => "通常",
                Self::Halved => "减半",
                Self::Invalid => "无效",
                Self::Absorb => "吸收",
            }),
            "en" => Some(match self {
                Self::Weakness => "Weak",
                Self::None => "Normal",
                Self::Halved => "Resist",
                Self::Invalid => "Immune",
                Self::Absorb => "Absorb",
            }),
            "ja" => Some(match self {
                Self::Weakness => "弱点",
                Self::None => "通常",
                Self::Halved => "半減",
                Self::Invalid => "無効",
                Self::Absorb => "吸収",
            }),
            _ => None,
        }
    }
}

/// Weapon detail type (`eEquipmentDetailType::*` weapon variants).
///
/// Note: UE's enum literally spells greatsword as `eEquipmentDetailType::GreatSowrd`.
/// Exported data has no weapon-type enemy weakness; this enum governs equippable weapon classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WeaponType {
    GreatSword,
    Katana,
    Bow,
    Hammer,
    Grimoire,
    Whip,
    Gun,
    Rod,
}

impl WeaponType {
    /// Parse an Unreal Engine `eEquipmentDetailType::*` weapon string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eEquipmentDetailType::GreatSowrd" => Some(Self::GreatSword),
            "eEquipmentDetailType::Katana" => Some(Self::Katana),
            "eEquipmentDetailType::Bow" => Some(Self::Bow),
            "eEquipmentDetailType::Hammer" => Some(Self::Hammer),
            "eEquipmentDetailType::Grimoire" => Some(Self::Grimoire),
            "eEquipmentDetailType::Whip" => Some(Self::Whip),
            "eEquipmentDetailType::Gun" => Some(Self::Gun),
            "eEquipmentDetailType::Rod" => Some(Self::Rod),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum string (preserving UE's `GreatSowrd` spelling).
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::GreatSword => "eEquipmentDetailType::GreatSowrd",
            Self::Katana => "eEquipmentDetailType::Katana",
            Self::Bow => "eEquipmentDetailType::Bow",
            Self::Hammer => "eEquipmentDetailType::Hammer",
            Self::Grimoire => "eEquipmentDetailType::Grimoire",
            Self::Whip => "eEquipmentDetailType::Whip",
            Self::Gun => "eEquipmentDetailType::Gun",
            Self::Rod => "eEquipmentDetailType::Rod",
        }
    }

    /// Localized display name for supported languages (`"zh-Hans"`, `"en"`, `"ja"`).
    pub fn localized_name(self, lang: &str) -> Option<&'static str> {
        match lang {
            "zh-Hans" => Some(match self {
                Self::GreatSword => "大剑",
                Self::Katana => "刀",
                Self::Bow => "弓",
                Self::Hammer => "锤",
                Self::Grimoire => "魔导书",
                Self::Whip => "鞭",
                Self::Gun => "火枪",
                Self::Rod => "魔棒",
            }),
            "en" => Some(match self {
                Self::GreatSword => "Greatsword",
                Self::Katana => "Katana",
                Self::Bow => "Bow",
                Self::Hammer => "Hammer",
                Self::Grimoire => "Grimoire",
                Self::Whip => "Whip",
                Self::Gun => "Gun",
                Self::Rod => "Rod",
            }),
            "ja" => Some(match self {
                Self::GreatSword => "大剣",
                Self::Katana => "刀",
                Self::Bow => "弓",
                Self::Hammer => "ハンマー",
                Self::Grimoire => "魔導書",
                Self::Whip => "鞭",
                Self::Gun => "銃",
                Self::Rod => "ロッド",
            }),
            _ => None,
        }
    }
}

/// Armor detail type (`eEquipmentDetailType::*` armor variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ArmorType {
    HeavyArmor,
    LightArmor,
    Robe,
}

impl ArmorType {
    /// Parse an Unreal Engine `eEquipmentDetailType::*` armor string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eEquipmentDetailType::HeavyArmor" => Some(Self::HeavyArmor),
            "eEquipmentDetailType::LightArmor" => Some(Self::LightArmor),
            "eEquipmentDetailType::Robe" => Some(Self::Robe),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum string.
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::HeavyArmor => "eEquipmentDetailType::HeavyArmor",
            Self::LightArmor => "eEquipmentDetailType::LightArmor",
            Self::Robe => "eEquipmentDetailType::Robe",
        }
    }
}

/// Unit category (`eUnitCategory::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UnitCategory {
    Playable,
    Enemy,
    Boss,
    Beast,
    Vision,
}

impl UnitCategory {
    /// Parse an Unreal Engine `eUnitCategory::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eUnitCategory::Playable" => Some(Self::Playable),
            "eUnitCategory::Enemy" => Some(Self::Enemy),
            "eUnitCategory::Boss" => Some(Self::Boss),
            "eUnitCategory::Beast" => Some(Self::Beast),
            "eUnitCategory::Vision" => Some(Self::Vision),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum string.
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::Playable => "eUnitCategory::Playable",
            Self::Enemy => "eUnitCategory::Enemy",
            Self::Boss => "eUnitCategory::Boss",
            Self::Beast => "eUnitCategory::Beast",
            Self::Vision => "eUnitCategory::Vision",
        }
    }

    /// Returns `true` for combat opponent categories (`Enemy` or `Boss`).
    pub const fn is_enemy_category(self) -> bool {
        matches!(self, Self::Enemy | Self::Boss)
    }
}

/// Unit race (`eRace::*`). Official multilingual names live in `ST_UI_Libra`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UnitRace {
    Animal,
    Aquatic,
    Avian,
    Beast,
    Demon,
    Dragon,
    Fairy,
    Human,
    Insect,
    Machina,
    Plantoid,
    Reaper,
    Stone,
    Vision,
    None,
}

impl UnitRace {
    /// Parse an Unreal Engine `eRace::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eRace::Animal" => Some(Self::Animal),
            "eRace::Aquatic" => Some(Self::Aquatic),
            "eRace::Avian" => Some(Self::Avian),
            "eRace::Beast" => Some(Self::Beast),
            "eRace::Demon" => Some(Self::Demon),
            "eRace::Dragon" => Some(Self::Dragon),
            "eRace::Fairy" => Some(Self::Fairy),
            "eRace::Human" => Some(Self::Human),
            "eRace::Insect" => Some(Self::Insect),
            "eRace::Machina" => Some(Self::Machina),
            "eRace::Plantoid" => Some(Self::Plantoid),
            "eRace::Reaper" => Some(Self::Reaper),
            "eRace::Stone" => Some(Self::Stone),
            "eRace::Vision" => Some(Self::Vision),
            "eRace::None" => Some(Self::None),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum string.
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::Animal => "eRace::Animal",
            Self::Aquatic => "eRace::Aquatic",
            Self::Avian => "eRace::Avian",
            Self::Beast => "eRace::Beast",
            Self::Demon => "eRace::Demon",
            Self::Dragon => "eRace::Dragon",
            Self::Fairy => "eRace::Fairy",
            Self::Human => "eRace::Human",
            Self::Insect => "eRace::Insect",
            Self::Machina => "eRace::Machina",
            Self::Plantoid => "eRace::Plantoid",
            Self::Reaper => "eRace::Reaper",
            Self::Stone => "eRace::Stone",
            Self::Vision => "eRace::Vision",
            Self::None => "eRace::None",
        }
    }

    /// Key inside `Strings/UI/UI_Common/ST_UI_Libra` for this race's localized name.
    pub const fn libra_string_key(self) -> &'static str {
        match self {
            Self::Animal => "UI_Libra_Race_Animal",
            Self::Aquatic => "UI_Libra_Race_Aquatic",
            Self::Avian => "UI_Libra_Race_Avian",
            Self::Beast => "UI_Libra_Race_Beast",
            Self::Demon => "UI_Libra_Race_Demon",
            Self::Dragon => "UI_Libra_Race_Dragon",
            Self::Fairy => "UI_Libra_Race_Fairy",
            Self::Human => "UI_Libra_Race_Human",
            Self::Insect => "UI_Libra_Race_Insect",
            Self::Machina => "UI_Libra_Race_Machina",
            Self::Plantoid => "UI_Libra_Race_Plantoid",
            Self::Reaper => "UI_Libra_Race_Reaper",
            Self::Stone => "UI_Libra_Race_Stone",
            Self::Vision => "UI_Libra_Race_Vision",
            Self::None => "UI_Libra_Race_None",
        }
    }
}

/// Skill attribute/command kind (`eSkillAttributeType::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkillAttributeType {
    Fight,
    ChaseAttack,
    Ability,
    Magic,
    LimitBurst,
    Item,
    Summon,
    MixItem,
    Throw,
    ThrowMoney,
    Dualcast,
    Mimic,
    MagicSword,
    Escape,
    Mechabo,
    FinishBlow,
}

impl SkillAttributeType {
    /// Parse an Unreal Engine `eSkillAttributeType::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eSkillAttributeType::Fight" => Some(Self::Fight),
            "eSkillAttributeType::ChaseAttack" => Some(Self::ChaseAttack),
            "eSkillAttributeType::Ability" => Some(Self::Ability),
            "eSkillAttributeType::Magic" => Some(Self::Magic),
            "eSkillAttributeType::LimitBurst" => Some(Self::LimitBurst),
            "eSkillAttributeType::Item" => Some(Self::Item),
            "eSkillAttributeType::Summon" => Some(Self::Summon),
            "eSkillAttributeType::MixItem" => Some(Self::MixItem),
            "eSkillAttributeType::Throw" => Some(Self::Throw),
            "eSkillAttributeType::ThrowMoney" => Some(Self::ThrowMoney),
            "eSkillAttributeType::Dualcast" => Some(Self::Dualcast),
            "eSkillAttributeType::Mimic" => Some(Self::Mimic),
            "eSkillAttributeType::MagicSword" => Some(Self::MagicSword),
            "eSkillAttributeType::Escape" => Some(Self::Escape),
            "eSkillAttributeType::Mechabo" => Some(Self::Mechabo),
            "eSkillAttributeType::FinishBlow" => Some(Self::FinishBlow),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum string.
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::Fight => "eSkillAttributeType::Fight",
            Self::ChaseAttack => "eSkillAttributeType::ChaseAttack",
            Self::Ability => "eSkillAttributeType::Ability",
            Self::Magic => "eSkillAttributeType::Magic",
            Self::LimitBurst => "eSkillAttributeType::LimitBurst",
            Self::Item => "eSkillAttributeType::Item",
            Self::Summon => "eSkillAttributeType::Summon",
            Self::MixItem => "eSkillAttributeType::MixItem",
            Self::Throw => "eSkillAttributeType::Throw",
            Self::ThrowMoney => "eSkillAttributeType::ThrowMoney",
            Self::Dualcast => "eSkillAttributeType::Dualcast",
            Self::Mimic => "eSkillAttributeType::Mimic",
            Self::MagicSword => "eSkillAttributeType::MagicSword",
            Self::Escape => "eSkillAttributeType::Escape",
            Self::Mechabo => "eSkillAttributeType::Mechabo",
            Self::FinishBlow => "eSkillAttributeType::FinishBlow",
        }
    }
}

/// Skill resource cost type (`eSkillCostType::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkillCostType {
    Mp,
    Lb,
    Item,
    ThrowItem,
    ThrowMoney,
    HpRatio,
    GoodLuck,
    OverClock,
}

impl SkillCostType {
    /// Parse an Unreal Engine `eSkillCostType::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eSkillCostType::MP" => Some(Self::Mp),
            "eSkillCostType::LB" => Some(Self::Lb),
            "eSkillCostType::Item" => Some(Self::Item),
            "eSkillCostType::ThrowItem" => Some(Self::ThrowItem),
            "eSkillCostType::ThrowMoney" => Some(Self::ThrowMoney),
            "eSkillCostType::HPRatio" => Some(Self::HpRatio),
            "eSkillCostType::GoodLuck" => Some(Self::GoodLuck),
            "eSkillCostType::OverClock" => Some(Self::OverClock),
            _ => None,
        }
    }

    /// Canonical Unreal Engine enum string.
    pub const fn as_ue_str(self) -> &'static str {
        match self {
            Self::Mp => "eSkillCostType::MP",
            Self::Lb => "eSkillCostType::LB",
            Self::Item => "eSkillCostType::Item",
            Self::ThrowItem => "eSkillCostType::ThrowItem",
            Self::ThrowMoney => "eSkillCostType::ThrowMoney",
            Self::HpRatio => "eSkillCostType::HPRatio",
            Self::GoodLuck => "eSkillCostType::GoodLuck",
            Self::OverClock => "eSkillCostType::OverClock",
        }
    }
}

/// Skill damage category (`eSkillDamageType::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkillDamageType {
    Physic,
    Magic,
    None,
}

impl SkillDamageType {
    /// Parse an Unreal Engine `eSkillDamageType::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eSkillDamageType::Physic" => Some(Self::Physic),
            "eSkillDamageType::Magic" => Some(Self::Magic),
            "eSkillDamageType::None" => Some(Self::None),
            _ => None,
        }
    }
}

/// Skill targeting scope (`eSkillTargetType::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkillTargetType {
    Single,
    Group,
    All,
    Random,
    SelfTarget,
    SingleAndGroup,
}

impl SkillTargetType {
    /// Parse an Unreal Engine `eSkillTargetType::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eSkillTargetType::Single" => Some(Self::Single),
            "eSkillTargetType::Group" => Some(Self::Group),
            "eSkillTargetType::All" => Some(Self::All),
            "eSkillTargetType::Random" => Some(Self::Random),
            "eSkillTargetType::Self" => Some(Self::SelfTarget),
            "eSkillTargetType::SingleAndGroup" => Some(Self::SingleAndGroup),
            _ => None,
        }
    }
}

/// Default targeting faction (`eRelation::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TargetRelation {
    Enemies,
    Friendlies,
}

impl TargetRelation {
    /// Parse an Unreal Engine `eRelation::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eRelation::Enemies" => Some(Self::Enemies),
            "eRelation::Friendlies" => Some(Self::Friendlies),
            _ => None,
        }
    }
}

/// Default target life state (`eSkillTargetState::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TargetState {
    Alive,
    Dead,
}

impl TargetState {
    /// Parse an Unreal Engine `eSkillTargetState::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eSkillTargetState::Alive" => Some(Self::Alive),
            "eSkillTargetState::Dead" => Some(Self::Dead),
            _ => None,
        }
    }
}

/// Where a skill may be used (`eAvailableLocation::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AvailableLocation {
    Anywhere,
    BattleOnly,
}

impl AvailableLocation {
    /// Parse an Unreal Engine `eAvailableLocation::*` string strictly.
    pub fn from_ue_str(raw: &str) -> Option<Self> {
        match raw {
            "eAvailableLocation::Anywhere" => Some(Self::Anywhere),
            "eAvailableLocation::BattleOnly" => Some(Self::BattleOnly),
            _ => None,
        }
    }
}

/// One typed unit row from `Unit/DT_UnitParameter.json` (`UNIT_PARAMETER`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unit {
    pub row_key: String,
    pub id: i64,
    pub name: FTextValue,
    pub description: FTextValue,
    pub level: i64,
    pub level_param_id: i64,
    pub sex: String,
    pub race: UnitRace,
    pub category: UnitCategory,
    pub is_boss: bool,
    pub is_accent: bool,
    pub max_hp: i64,
    pub max_mp: i64,
    pub max_break_point: i64,
    pub attack: i64,
    pub defence: i64,
    pub intelligence: i64,
    pub mind: i64,
    pub divine: i64,
    pub agility: i64,
    pub element_resistances: BTreeMap<Element, Resistance>,
    pub status_resistances: BTreeMap<String, i64>,
    pub weapon_id: i64,
    pub armor_id: i64,
    pub accessory_ids: Vec<i64>,
    pub vision_id: i64,
    pub passive_skill_ids: Vec<i64>,
    pub experience: i64,
    pub money: i64,
    pub drop_normal_item: i64,
    pub drop_normal_item_rate: i64,
    pub drop_rare_item: i64,
    pub drop_rare_item_rate: i64,
    pub steal_normal_item: i64,
    pub steal_rare_item: i64,
    pub steal_item_rate: i64,
    pub traits: Vec<FTextValue>,
}

impl Unit {
    /// Build a battle-focused [`EnemyProfile`] from this unit and optional AI profile.
    pub fn to_enemy_profile(&self, ai: Option<&UnitAiProfile>) -> EnemyProfile {
        let mut weaknesses = Vec::new();
        let mut halved = Vec::new();
        let mut invalid = Vec::new();
        let mut absorb = Vec::new();
        for element in ALL_COMBAT_ELEMENTS {
            match self.element_resistances.get(&element).copied() {
                Some(Resistance::Weakness) => weaknesses.push(element),
                Some(Resistance::Halved) => halved.push(element),
                Some(Resistance::Invalid) => invalid.push(element),
                Some(Resistance::Absorb) => absorb.push(element),
                Some(Resistance::None) | None => {}
            }
        }
        EnemyProfile {
            unit_id: self.id,
            row_key: self.row_key.clone(),
            name: self.name.clone(),
            category: self.category,
            race: self.race,
            level: self.level,
            is_boss: self.is_boss,
            is_accent: self.is_accent,
            max_hp: self.max_hp,
            max_mp: self.max_mp,
            max_break_point: self.max_break_point,
            attack: self.attack,
            defence: self.defence,
            intelligence: self.intelligence,
            mind: self.mind,
            divine: self.divine,
            agility: self.agility,
            weaknesses,
            halved,
            invalid,
            absorb,
            element_resistances: self.element_resistances.clone(),
            status_resistances: self.status_resistances.clone(),
            high_risk_skill_ids: ai
                .map(|profile| profile.high_risk_skill_ids.clone())
                .unwrap_or_default(),
            traits: self.traits.clone(),
        }
    }
}

/// Battle-focused enemy summary record with HP, elemental weaknesses/resistances, and break gauge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnemyProfile {
    pub unit_id: i64,
    pub row_key: String,
    pub name: FTextValue,
    pub category: UnitCategory,
    pub race: UnitRace,
    pub level: i64,
    pub is_boss: bool,
    pub is_accent: bool,
    pub max_hp: i64,
    pub max_mp: i64,
    /// Maximum break gauge (`DT_UnitParameter.MaxBreakPoint`, equivalent to shield capacity).
    pub max_break_point: i64,
    pub attack: i64,
    pub defence: i64,
    pub intelligence: i64,
    pub mind: i64,
    pub divine: i64,
    pub agility: i64,
    pub weaknesses: Vec<Element>,
    pub halved: Vec<Element>,
    pub invalid: Vec<Element>,
    pub absorb: Vec<Element>,
    pub element_resistances: BTreeMap<Element, Resistance>,
    pub status_resistances: BTreeMap<String, i64>,
    pub high_risk_skill_ids: Vec<i64>,
    pub traits: Vec<FTextValue>,
}

impl EnemyProfile {
    /// Returns `true` if `element` is listed as a weakness of this enemy.
    pub fn is_weak_to(&self, element: Element) -> bool {
        self.weaknesses.contains(&element)
    }

    /// Look up resistance for `element` (`Element::None` yields `Resistance::None`).
    pub fn resistance_to(&self, element: Element) -> Option<Resistance> {
        if element == Element::None {
            return Some(Resistance::None);
        }
        self.element_resistances.get(&element).copied()
    }

    /// Estimate break gauge reduction dealt by `skill` using `calc` multipliers.
    ///
    /// Returns `None` if `skill.element` is missing from this enemy's resistance table or if the
    /// corresponding resistance multiplier is missing from `calc`.
    pub fn estimated_break_damage(&self, skill: &Skill, calc: &BreakCalcParams) -> Option<f64> {
        let resistance = self.resistance_to(skill.element)?;
        let multiplier = calc.break_element_multipliers.get(&resistance)?;
        Some((skill.break_damage as f64) * multiplier)
    }
}

/// Playable character equipment & role row from `Unit/DT_PlayableUnitParameter.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayableUnitProfile {
    pub row_key: String,
    pub id: i64,
    pub attack_type: SkillDamageType,
    pub roles: Vec<String>,
    pub equipable_weapon_type: WeaponType,
    pub equipable_armor_types: Vec<ArmorType>,
}

/// Unit AI skill references from `Unit/AI/DT_BtlUnitAIParameter.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitAiProfile {
    pub row_key: String,
    pub id: i64,
    pub fight_skill_id: i64,
    pub chase_skill_id: i64,
    pub ai_id: i64,
    pub counter_skill_ids: Vec<i64>,
    pub silence_fight_id: i64,
    /// Dangerous/telegraphed high-risk skills (interruptible by Break).
    pub high_risk_skill_ids: Vec<i64>,
}

/// Global elemental & break calculation multipliers from `Unit/DT_UnitParameterCalc.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakCalcParams {
    pub row_key: String,
    pub break_damage_magnification: f64,
    pub element_damage_multipliers: BTreeMap<Resistance, f64>,
    pub break_element_multipliers: BTreeMap<Resistance, f64>,
}

/// Skill cost override from `Skill/Other/DT_SkillGeneralData.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCostOverride {
    pub skill_id: i64,
    pub cost_type: SkillCostType,
    pub cost: i64,
}

/// One battle encounter group from `Encount/DT_BattleGroup.json` (`BATTLE_GROUP_DATA`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BattleGroup {
    pub row_key: String,
    pub id: i64,
    pub unit_ids: Vec<i64>,
    pub location_ids: Vec<i64>,
    pub added_passive_skill_ids: Vec<Vec<i64>>,
    pub can_escape: bool,
    pub escape_probability: f64,
    pub is_bounty: bool,
    pub ap: i64,
    pub bgm_cue_name: String,
    pub preemptive_relation: String,
    pub battle_event_id: i64,
    pub battle_finish_event_id: String,
    pub battle_finish_escape_event_id: String,
    pub battle_finish_lose_event_id: String,
    /// `Some(true)` if any member is a boss and all members resolve in `DT_UnitParameter`;
    /// `Some(false)` if all members resolve and none is a boss; `None` if unresolved.
    pub is_boss: Option<bool>,
}

/// A [`BattleGroup`] whose member unit IDs have all been verified and resolved into [`EnemyProfile`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedBattleGroup {
    pub group: BattleGroup,
    pub members: Vec<EnemyProfile>,
    pub is_boss: bool,
}

/// One weighted encounter entry from `Encount/*/DT_*_Encount.json` (`ENCOUNT_GROUP_DATA`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncounterPoolEntry {
    pub row_key: String,
    pub battle_group_id: i64,
    pub encount_ratio: i64,
}

/// One active skill or spell from `Skill/DT_SkillData.json` (`SKILL_DATA`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub row_key: String,
    pub id: i64,
    pub sort_id: i64,
    pub name: FTextValue,
    pub description: FTextValue,
    pub has_unit: String,
    pub belong_command_ids: Vec<i64>,
    pub skill_attr_type: SkillAttributeType,
    pub damage_type: SkillDamageType,
    pub damage_calc_type: String,
    pub element: Element,
    pub target_type: SkillTargetType,
    pub default_target_relation: TargetRelation,
    pub default_target_state: TargetState,
    pub cost: i64,
    pub cost_type: SkillCostType,
    /// Power multiplier (`DT_SkillData.magnification`).
    pub power: i64,
    /// Break gauge reduction power (`DT_SkillData.breakDamageValue`).
    pub break_damage: i64,
    pub accuracy: i64,
    pub critical_hit_rate: i64,
    pub hit_count: i64,
    pub hit_damage_ratios: Vec<f64>,
    pub equip_cost: i64,
    pub available_location: AvailableLocation,
    pub effect_ids: Vec<i64>,
}

/// One battle command category/entry from `Skill/DT_CommandSkillData.json` (`COMMAND_SKILL_DATA`).
///
/// Represents top-level/submenu commands in the battle command wheel (such as `1` 招式, `5` 幻兵召唤,
/// `6` 幻兽召唤, `7` 道具, `10` 极限技, and Vision-specific command groups `100..=209`).
/// Direct single-action entries in `DT_UnitBattleCmd` (such as Attack `401010..408010`, Defend
/// `400010`, and Escape `400020`) live in `Skill/DT_SkillData` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSkill {
    pub row_key: String,
    pub id: i64,
    pub sort_id: i64,
    /// Vision or unit ID required to unlock/use this command (`None` when `unitIdToUseSkill < 0`).
    pub unit_id_to_use_skill: Option<i64>,
    pub name: FTextValue,
    pub description: FTextValue,
    pub skill_icon: String,
    pub is_removable: bool,
    pub equip_cost: i64,
    pub filters: Vec<String>,
}

impl CommandSkill {
    /// Returns `true` for placeholder rows (`ID < 0`, such as `ＮＯＮＥ`).
    pub const fn is_placeholder(&self) -> bool {
        self.id < 0
    }
}
