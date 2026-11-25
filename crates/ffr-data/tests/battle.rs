//! Integration tests for `ffr-data::battle`: synthetic fixture tests, strict error validation,
//! and real-export verification (skipped cleanly when `.deps/ffr/export/` is unavailable).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ffr_data::battle::{
    ArmorType, AvailableLocation, BattleData, Element, Resistance, SkillAttributeType,
    SkillCostType, SkillDamageType, SkillTargetType, TargetRelation, TargetState, UnitCategory,
    UnitRace, WeaponType, enemy_summary, load_encounter_pool, load_playable_unit_parameters,
    parse_battle_groups, parse_skills, parse_units,
};
use ffr_data::{
    DataError, DataStore, DataTable, DataTableType, FFR_DEPS_ENV, datatable_dir,
    export_dir_from_deps, localization_file,
};
use serde_json::json;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ffr-data sits two levels below repository root")
        .to_path_buf()
}

fn synthetic_battle_fixture_dir() -> PathBuf {
    repo_root().join("tests/fixtures/rust/battle")
}

fn real_export_dir() -> PathBuf {
    if let Some(deps) = std::env::var_os(FFR_DEPS_ENV)
        && !deps.is_empty()
    {
        return export_dir_from_deps(PathBuf::from(deps));
    }
    export_dir_from_deps(repo_root().join(".deps"))
}

#[test]
fn synthetic_fixture_units_profiles_groups_skills_and_summary() {
    let fixture_root = synthetic_battle_fixture_dir();
    let store = DataStore::new(&fixture_root);
    let battle = store.battle().expect("load synthetic BattleData");

    // 7 core tables loaded through DataStore cache.
    assert_eq!(store.cached_data_table_count(), 7);
    assert_eq!(battle.units.len(), 4);
    assert_eq!(battle.battle_groups.len(), 3);
    assert_eq!(battle.skills.len(), 4);

    // Lookup by ID: Unit & EnemyProfile.
    let hero = battle.unit_by_id(9001).expect("hero unit exists");
    assert_eq!(hero.category, UnitCategory::Playable);
    assert!(!hero.is_boss);
    assert_eq!(hero.max_hp, 320);

    let mob = battle.enemy_profile(9101).expect("mob enemy profile");
    assert_eq!(mob.max_hp, 140);
    assert_eq!(mob.max_break_point, 60);
    assert!(!mob.is_boss);
    assert_eq!(mob.race, UnitRace::Avian);
    assert_eq!(mob.weaknesses, vec![Element::Fire, Element::Thunder]);
    assert_eq!(mob.halved, vec![Element::Ice]);
    assert_eq!(mob.invalid, vec![Element::Water]);
    assert!(mob.absorb.is_empty());
    assert!(mob.is_weak_to(Element::Fire));
    assert!(!mob.is_weak_to(Element::Ice));
    assert_eq!(mob.resistance_to(Element::Water), Some(Resistance::Invalid));

    let boss = battle.enemy_profile(9201).expect("boss enemy profile");
    assert!(boss.is_boss);
    assert_eq!(boss.category, UnitCategory::Boss);
    assert_eq!(boss.max_hp, 4200);
    assert_eq!(boss.max_break_point, 500);
    assert_eq!(boss.weaknesses, vec![Element::Ice, Element::Water]);
    assert_eq!(boss.halved, vec![Element::Earth]);
    assert_eq!(boss.absorb, vec![Element::Fire]);
    assert_eq!(boss.high_risk_skill_ids, vec![95003]);

    // Lookup by zh-Hans name (including duplicate enemy names & excluding playable units).
    let mobs_by_name = battle
        .enemies_by_name("霜翼试作兽")
        .expect("lookup enemies by zh-Hans name");
    let mob_ids: Vec<i64> = mobs_by_name.iter().map(|enemy| enemy.unit_id).collect();
    assert_eq!(mob_ids, vec![9101, 9202]);

    let heroes_as_units = battle
        .units_by_name("合成英雄")
        .expect("lookup playable unit by zh-Hans name");
    assert_eq!(heroes_as_units.len(), 1);
    assert_eq!(heroes_as_units[0].id, 9001);

    let heroes_as_enemies = battle
        .enemies_by_name("合成英雄")
        .expect("enemies_by_name filters out playable units");
    assert!(heroes_as_enemies.is_empty());

    // Multilingual name lookup.
    let boss_en = battle
        .enemies_by_name_in("Core Guardian", "en")
        .expect("lookup enemy by English name");
    assert_eq!(boss_en.len(), 1);
    assert_eq!(boss_en[0].unit_id, 9201);

    // Skills: element, power, break damage, cost & override, targeting, lookup by name.
    let ice_skill = battle.skill_by_id(95001).expect("ice skill exists");
    assert_eq!(ice_skill.element, Element::Ice);
    assert_eq!(ice_skill.damage_type, SkillDamageType::Magic);
    assert_eq!(ice_skill.skill_attr_type, SkillAttributeType::Magic);
    assert_eq!(ice_skill.cost, 8);
    assert_eq!(ice_skill.cost_type, SkillCostType::Mp);
    assert_eq!(ice_skill.power, 24);
    assert_eq!(ice_skill.break_damage, 30);
    assert_eq!(ice_skill.target_type, SkillTargetType::Single);
    assert_eq!(ice_skill.default_target_relation, TargetRelation::Enemies);
    assert_eq!(ice_skill.default_target_state, TargetState::Alive);
    assert_eq!(ice_skill.available_location, AvailableLocation::BattleOnly);
    assert!(ice_skill.effect_ids.is_empty());

    // Estimated break damage uses DT_UnitParameterCalc multipliers (3.0x on weakness, 0.0x on absorb).
    let fire_skill = battle.skill_by_id(95002).expect("fire skill exists");
    assert_eq!(fire_skill.effect_ids, vec![7001]);
    assert_eq!(
        boss.estimated_break_damage(ice_skill, &battle.break_calc),
        Some(90.0)
    );
    assert_eq!(
        boss.estimated_break_damage(fire_skill, &battle.break_calc),
        Some(0.0)
    );

    // Cost override from DT_SkillGeneralData takes precedence over DT_SkillData.Cost=0 / MP.
    let hp_skill = battle.skill_by_id(95004).expect("hp-cost skill exists");
    assert_eq!(hp_skill.cost, 10);
    assert_eq!(hp_skill.cost_type, SkillCostType::HpRatio);
    assert_eq!(hp_skill.element, Element::None);

    let skills_zh = battle
        .skills_by_name("试作冰刺")
        .expect("lookup skill by zh-Hans name");
    assert_eq!(skills_zh.len(), 1);
    assert_eq!(skills_zh[0].id, 95001);

    // Battle groups: normal, boss, and broken member reference.
    let normal_group = battle.battle_group_by_id(9801).expect("normal group");
    assert_eq!(normal_group.is_boss, Some(false));
    assert!(normal_group.can_escape);
    let resolved_normal = battle
        .resolve_battle_group(9801)
        .expect("resolve normal battle group");
    assert!(!resolved_normal.is_boss);
    assert_eq!(resolved_normal.members.len(), 2);

    let boss_group = battle.battle_group_by_id(9802).expect("boss group");
    assert_eq!(boss_group.is_boss, Some(true));
    assert!(!boss_group.can_escape);
    assert!(boss_group.is_bounty);
    let resolved_boss = battle
        .resolve_battle_group(9802)
        .expect("resolve boss battle group");
    assert!(resolved_boss.is_boss);
    assert_eq!(resolved_boss.members[0].unit_id, 9201);

    let broken_group = battle.battle_group_by_id(9803).expect("broken group");
    assert_eq!(broken_group.is_boss, None);
    let broken_err = battle
        .resolve_battle_group(9803)
        .expect_err("broken battle group must fail honestly");
    assert!(broken_err.to_string().contains("99999"));

    // Companion loaders: playable unit weapon/armor types and encounter pool.
    let playable = load_playable_unit_parameters(&fixture_root).expect("load playable params");
    assert_eq!(playable.len(), 1);
    assert_eq!(playable[0].equipable_weapon_type, WeaponType::GreatSword);
    assert_eq!(
        playable[0].equipable_weapon_type.localized_name("zh-Hans"),
        Some("大剑")
    );
    assert_eq!(
        playable[0].equipable_armor_types,
        vec![ArmorType::HeavyArmor, ArmorType::LightArmor]
    );

    let pool = load_encounter_pool(&fixture_root, "Encount/00Test/DT_00Test_Encount")
        .expect("load encounter pool");
    assert_eq!(pool.len(), 2);
    assert_eq!(pool[0].battle_group_id, 9801);
    assert_eq!(pool[0].encount_ratio, 70);

    // Model-facing enemy_summary(id, lang).
    let boss_summary = enemy_summary(&battle, 9201, "zh-Hans").expect("boss enemy_summary");
    assert!(boss_summary.contains("熔核守护者"));
    assert!(boss_summary.contains("Boss"));
    assert!(boss_summary.contains("4200"));
    assert!(boss_summary.contains("500"));
    assert!(boss_summary.contains("冰、水"));
    assert!(boss_summary.contains("吸收(火)"));
    assert!(boss_summary.contains("炽核爆裂"));

    let mob_summary = battle
        .enemy_summary(9101, "zh-Hans")
        .expect("mob enemy_summary");
    assert!(mob_summary.contains("霜翼试作兽"));
    assert!(mob_summary.contains("普通敌人"));
    assert!(mob_summary.contains("火、雷"));
    assert!(mob_summary.contains("60"));
}

#[test]
fn strict_errors_on_missing_ids_languages_translations_and_invalid_schemas() {
    let fixture_root = synthetic_battle_fixture_dir();
    let battle = BattleData::load(&fixture_root).expect("load synthetic fixture");

    // Missing unit ID and missing battle group ID must fail with Chinese errors.
    let missing_unit_err = battle
        .enemy_summary(404_404, "zh-Hans")
        .expect_err("unknown enemy ID must fail");
    assert!(missing_unit_err.to_string().contains("404404"));

    let missing_group_err = battle
        .resolve_battle_group(404_404)
        .expect_err("unknown battle group ID must fail");
    assert!(missing_group_err.to_string().contains("404404"));

    // Unsafe or unexported language must fail instead of falling back silently.
    let bad_lang_err = battle
        .enemies_by_name_in("霜翼试作兽", "../bad")
        .expect_err("unsafe lang must fail");
    assert!(matches!(bad_lang_err, DataError::InvalidLanguage { .. }));

    let missing_lang_err = battle
        .enemy_summary(9101, "xx")
        .expect_err("unexported lang must fail");
    assert!(matches!(missing_lang_err, DataError::Io { .. }));

    // Missing zh-Hans translation key must error rather than faking Japanese as Chinese.
    let temp = tempfile::tempdir().expect("create tempdir");
    copy_dir_recursive(&fixture_root, temp.path());
    fs::write(
        temp.path().join("Localization/zh-Hans.json"),
        r#"{ "ST_UnitName": {}, "ST_SkillName": {} }"#,
    )
    .expect("overwrite zh-Hans localization with empty keys");
    let battle_missing_tr = BattleData::load(temp.path()).expect("tables themselves still load");
    let tr_err = battle_missing_tr
        .enemy_summary(9101, "zh-Hans")
        .expect_err("missing translation must error");
    assert!(tr_err.to_string().contains("9101"));

    // Wrong RowStruct must yield UnexpectedRowStruct.
    let wrong_struct_table = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_Wrong".to_owned(),
        package_path: "/Game/Test/DT_Wrong".to_owned(),
        row_struct: "OTHER_STRUCT".to_owned(),
        rows: BTreeMap::new(),
    };
    assert!(matches!(
        parse_units(&wrong_struct_table, "DT_Wrong"),
        Err(DataError::UnexpectedRowStruct { .. })
    ));
    assert!(matches!(
        parse_battle_groups(&wrong_struct_table, "DT_Wrong"),
        Err(DataError::UnexpectedRowStruct { .. })
    ));
    assert!(matches!(
        parse_skills(
            &wrong_struct_table,
            &BTreeMap::new(),
            &BTreeMap::new(),
            "DT_Wrong"
        ),
        Err(DataError::UnexpectedRowStruct { .. })
    ));

    // Missing required field (`MaxBreakPoint`) and incomplete `ElementResistanceList` must fail.
    let bad_unit_table = DataTable {
        table_type: DataTableType::DataTable,
        name: "DT_UnitParameter".to_owned(),
        package_path: "/Game/Datatable/Unit/DT_UnitParameter".to_owned(),
        row_struct: "UNIT_PARAMETER".to_owned(),
        rows: BTreeMap::from([(
            "BrokenRow".to_owned(),
            json!({
                "ID": 1,
                "Name": { "CultureInvariantString": "Broken" },
                "Description": { "CultureInvariantString": null },
                "Level": 1,
                "LevelParamId": -1,
                "Sex": "eUnitSex::None",
                "Race": "eRace::Avian",
                "Category": "eUnitCategory::Enemy",
                "IsBoss": false,
                "IsAccent": false,
                "MaxHitPoint": 50,
                "MaxMagicPoint": 10,
                "Attack": 10,
                "Defence": 10,
                "Intelligence": 10,
                "Mind": 10,
                "Divine": 0,
                "Agility": 10,
                "ElementResistanceList": [
                    { "Key": "eElement::Fire", "Value": "eResistance::None" }
                ],
                "StatusConditionResistanceList": [],
                "WeaponId": -1,
                "ArmorId": -1,
                "AccessoryIdList": [],
                "VisionId": -1,
                "passiveSkillList": [],
                "Experience": 1,
                "Money": 1,
                "DropNormalItem": -1,
                "DropNormalItemRate": 0,
                "DropRareItem": -1,
                "DropRareItemRate": 0,
                "StealNormalItem": -1,
                "StealRareItem": -1,
                "StealItemRate": 0,
                "Traits": []
            }),
        )]),
    };
    let elem_err = parse_units(&bad_unit_table, "DT_UnitParameter")
        .expect_err("incomplete ElementResistanceList must fail");
    assert!(elem_err.to_string().contains("ElementResistanceList"));
}

#[test]
fn real_export_battle_data_and_steel_bat_ice_weakness() {
    let export_root = real_export_dir();
    let unit_file = datatable_dir(&export_root).join("Unit/DT_UnitParameter.json");
    let has_real_data = unit_file.is_file() && localization_file(&export_root, "zh-Hans").is_file();
    if !has_real_data {
        eprintln!(
            "跳过真实导出战斗数据测试：未找到 {} 或 zh-Hans.json",
            unit_file.display()
        );
        return;
    }

    let store = DataStore::new(&export_root);
    let battle = store.battle().expect("load real BattleData");

    assert_eq!(battle.units.len(), 643);
    assert_eq!(battle.battle_groups.len(), 678);
    assert_eq!(battle.skills.len(), 1167);
    assert_eq!(battle.unit_ai.len(), 626);
    assert!((battle.break_calc.break_damage_magnification - 150.0).abs() < f64::EPSILON);
    assert_eq!(
        battle
            .break_calc
            .break_element_multipliers
            .get(&Resistance::Weakness)
            .copied(),
        Some(3.0)
    );

    let playable =
        load_playable_unit_parameters(&export_root).expect("load real DT_PlayableUnitParameter");
    assert_eq!(playable.len(), 8);

    // Spot-check 钢蝠 (user verified on real game machine that it is weak to Ice).
    let steel_bats = battle
        .enemies_by_name("钢蝠")
        .expect("lookup 钢蝠 by zh-Hans name");
    assert_eq!(steel_bats.len(), 1);
    let steel_bat = &steel_bats[0];
    assert_eq!(steel_bat.unit_id, 2);
    assert!(!steel_bat.is_boss);
    assert!(steel_bat.max_hp > 0);
    assert!(steel_bat.max_break_point > 0);
    assert!(
        steel_bat.is_weak_to(Element::Ice),
        "钢蝠（ID={}）弱点列表中应包含冰属性，实际弱点：{:?}",
        steel_bat.unit_id,
        steel_bat.weaknesses
    );

    let summary = enemy_summary(&battle, steel_bat.unit_id, "zh-Hans")
        .expect("generate zh-Hans enemy_summary for 钢蝠");
    assert!(summary.contains("钢蝠"));
    assert!(summary.contains("冰"));
    assert!(summary.contains("破防槽"));
    assert!(summary.contains("普通敌人"));

    // Verify battle group boss classification across normal & boss encounters.
    let normal_groups = battle
        .battle_groups
        .iter()
        .filter(|group| group.is_boss == Some(false))
        .count();
    let boss_groups = battle
        .battle_groups
        .iter()
        .filter(|group| group.is_boss == Some(true))
        .count();
    let unresolved_groups = battle
        .battle_groups
        .iter()
        .filter(|group| group.is_boss.is_none())
        .count();
    assert!(normal_groups > 400);
    assert!(boss_groups > 100);
    assert!(unresolved_groups > 0);

    // Resolve a real encounter containing 钢蝠 (ID=2) and verify non-boss resolution.
    let bat_group = battle
        .battle_groups
        .iter()
        .find(|group| group.unit_ids.contains(&steel_bat.unit_id) && group.is_boss == Some(false))
        .expect("find normal battle group containing 钢蝠");
    let resolved_bat_group = battle
        .resolve_battle_group(bat_group.id)
        .expect("resolve battle group containing 钢蝠");
    assert!(!resolved_bat_group.is_boss);
    assert!(
        resolved_bat_group
            .members
            .iter()
            .any(|member| member.unit_id == steel_bat.unit_id)
    );
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create destination dir");
    for entry in fs::read_dir(src).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let target = dst.join(entry.file_name());
        if entry.file_type().expect("entry file type").is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}
