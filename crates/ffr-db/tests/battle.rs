//! Integration and alignment tests for the `03_battle` area of `ffr-db`.
//!
//! Covers:
//! - Synthetic fixture build, dangling-reference `build_issue` recording, and strict `name_zh`
//!   (missing translations remain `NULL` and never fall back to Japanese).
//! - All battle query APIs (`unit`, `units_by_name`, `enemy_profile`, `skill`, `skills_by_name`,
//!   `skills_of_unit`, `battle_group`, `groups_containing`, `enemies_in_chapter`,
//!   `chapters_of_enemy`, `units_weak_to`, `break_calc`, plus companion queries).
//! - `assert_db_uses_index` verification that no battle query triggers a full table scan.
//! - Exact `==` alignment against `ffr_data::battle::{BattleData, CommandSkillData, EnemyChapterIndex}`
//!   on synthetic data and (when `.deps/ffr/export` exists) across all real units, skills,
//!   battle groups, enemy profiles, AI profiles, command skills, and chapter mappings.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ffr_data::battle::{
    CommandSkillData, Element, EnemyChapterIndex, EnemyChapterSourceKind, Resistance, UnitCategory,
};
use ffr_data::{DataStore, FFR_DEPS_ENV, datatable_dir, export_dir_from_deps, localization_file};
use ffr_db::testing::{
    assert_db_uses_index, build_fixture_db_stages, build_temp_db, fixture_area_dir,
};
use ffr_db::{BuildInputs, Stage, StageStatus};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ffr-db sits two levels below repository root")
        .to_path_buf()
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
fn synthetic_battle_queries_alignment_issues_and_indexes() {
    let fixture_export = fixture_area_dir("battle").join("export");
    let test_db = build_fixture_db_stages("battle", Some(vec![Stage::Battle]))
        .expect("build battle stage on synthetic fixture");
    let db = test_db.db();

    // Stage completed cleanly with rows.
    let battle_stage = test_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "battle")
        .expect("battle stage report");
    assert_eq!(battle_stage.status, StageStatus::Ok);
    assert!(battle_stage.rows > 0);

    // Build expected reference objects directly from ffr-data.
    let store = DataStore::new(&fixture_export);
    let battle = store.battle().expect("load BattleData");
    let commands = CommandSkillData::from_store(&store).expect("load CommandSkillData");
    let chapters = EnemyChapterIndex::from_store_with_battle(&store, &battle)
        .expect("build EnemyChapterIndex");

    // 1. Exact alignment (`==`) for every Unit, EnemyProfile, UnitAiProfile, Skill, BattleGroup,
    //    CommandSkill, and BreakCalcParams.
    for expected in &battle.units {
        let actual = db
            .unit(expected.id)
            .expect("query unit")
            .expect("unit must exist in DB");
        assert_eq!(&actual, expected, "Unit {} mismatch", expected.id);

        let expected_profile = battle
            .enemy_profile(expected.id)
            .expect("reference enemy_profile");
        let actual_profile = db
            .enemy_profile(expected.id)
            .expect("query enemy_profile")
            .expect("enemy_profile must exist in DB");
        assert_eq!(
            actual_profile, expected_profile,
            "EnemyProfile {} mismatch",
            expected.id
        );
    }
    assert_eq!(db.unit(404_404).expect("missing unit"), None);
    assert_eq!(
        db.enemy_profile(404_404).expect("missing enemy_profile"),
        None
    );

    for (unit_id, expected_ai) in &battle.unit_ai {
        let actual_ai = db
            .unit_ai(*unit_id)
            .expect("query unit_ai")
            .expect("unit_ai must exist");
        assert_eq!(&actual_ai, expected_ai);
    }

    for expected in &battle.skills {
        let actual = db
            .skill(expected.id)
            .expect("query skill")
            .expect("skill must exist in DB");
        assert_eq!(&actual, expected, "Skill {} mismatch", expected.id);
    }
    assert_eq!(db.skill(404_404).expect("missing skill"), None);

    for expected in &battle.battle_groups {
        let detail = db
            .battle_group(expected.id)
            .expect("query battle_group")
            .expect("battle_group must exist in DB");
        assert_eq!(
            &detail.group, expected,
            "BattleGroup {} mismatch",
            expected.id
        );
        assert_eq!(detail.members.len(), expected.unit_ids.len());
        for (slot, (&expected_uid, member)) in
            expected.unit_ids.iter().zip(&detail.members).enumerate()
        {
            assert_eq!(member.slot, slot as i64);
            assert_eq!(member.unit_id, expected_uid);
            assert_eq!(member.unit.as_ref(), battle.unit_by_id(expected_uid));
        }
    }
    assert_eq!(
        db.battle_group(404_404).expect("missing battle_group"),
        None
    );

    // Dangling member in GroupBrokenRef (9803) is preserved with unit = None.
    let broken_group = db
        .battle_group(9803)
        .expect("query broken group")
        .expect("group 9803 exists");
    assert_eq!(broken_group.members.len(), 2);
    assert!(broken_group.members[0].unit.is_some());
    assert_eq!(broken_group.members[1].unit_id, 99999);
    assert_eq!(broken_group.members[1].unit, None);

    for expected in &commands.commands {
        let actual = db
            .command_skill(expected.id)
            .expect("query command_skill")
            .expect("command_skill must exist");
        assert_eq!(&actual, expected);
    }
    assert_eq!(
        db.command_skills_for_unit(13017)
            .expect("command_skills_for_unit")
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![100]
    );

    assert_eq!(
        db.break_calc().expect("query break_calc"),
        Some(battle.break_calc.clone())
    );

    // 2. Name queries: exact-first, then substring contains, duplicates all listed, multilingual,
    //    and missing zh-Hans stays NULL without Japanese fallback.
    let zh_frost = db
        .units_by_name("霜翼试作兽", "zh-Hans")
        .expect("units_by_name zh exact+contains");
    let zh_frost_ids: Vec<i64> = zh_frost.iter().map(|u| u.id).collect();
    assert_eq!(zh_frost_ids, vec![9101, 9202, 9203]);

    // Substring-only query (2 chars) matches all three Frostwing units in ID order.
    let zh_sub = db
        .units_by_name("霜翼", "zh-Hans")
        .expect("units_by_name zh 2-char substring");
    assert_eq!(
        zh_sub.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![9101, 9202, 9203]
    );

    // English exact-first + contains + case-insensitive substring.
    let en_frost = db
        .units_by_name("Frostwing Prototype", "en")
        .expect("units_by_name en");
    assert_eq!(
        en_frost.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![9101, 9202, 9203]
    );
    let en_guardian = db
        .units_by_name("guardian", "en")
        .expect("units_by_name en lowercase substring");
    assert_eq!(
        en_guardian.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![9201]
    );

    // UntranslatedMob (9204): name_zh is NULL and Japanese text must NOT match zh-Hans query.
    let untrans_zh_row = db
        .query_readonly_json("SELECT name_zh FROM unit WHERE id = 9204")
        .expect("query unit 9204 name_zh");
    assert_eq!(untrans_zh_row.len(), 1);
    assert!(untrans_zh_row[0]["name_zh"].is_null());
    assert!(
        db.units_by_name("テスト未訳敵", "zh-Hans")
            .expect("no ja fallback in zh-Hans")
            .is_empty()
    );
    assert_eq!(
        db.units_by_name("テスト未訳敵", "ja")
            .expect("ja lookup finds untranslated unit")
            .iter()
            .map(|u| u.id)
            .collect::<Vec<_>>(),
        vec![9204]
    );
    assert_eq!(
        db.units_by_name("Untranslated Foe", "en")
            .expect("en lookup finds untranslated unit")
            .iter()
            .map(|u| u.id)
            .collect::<Vec<_>>(),
        vec![9204]
    );

    // Skills by name: exact duplicates (95001, 95006) before contains (95005).
    let zh_ice = db
        .skills_by_name("试作冰刺", "zh-Hans")
        .expect("skills_by_name zh");
    assert_eq!(
        zh_ice.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![95001, 95006, 95005]
    );
    let en_ice = db
        .skills_by_name("Synth Ice Spike", "en")
        .expect("skills_by_name en");
    assert_eq!(
        en_ice.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![95001, 95006, 95005]
    );
    assert!(db.units_by_name("", "zh-Hans").expect("empty").is_empty());
    assert!(db.skills_by_name("", "zh-Hans").expect("empty").is_empty());

    // 3. Unit skills with provenance (level_up, battle_cmd, ai_*, vision_* + dangling skill ref).
    let hero_skills = db.skills_of_unit(9001).expect("skills_of_unit 9001");
    let hero_summary: Vec<(i64, &str, Option<i64>, bool)> = hero_skills
        .iter()
        .map(|e| (e.skill_id, e.source.as_str(), e.level, e.skill.is_some()))
        .collect();
    assert_eq!(
        hero_summary,
        vec![
            (95004, "ai_fight", None, true),
            (95004, "battle_cmd", None, true),
            (95001, "level_up", Some(1), true),
            (95002, "level_up", Some(3), true),
        ]
    );

    let vision_skills = db.skills_of_unit(13017).expect("skills_of_unit 13017");
    let vision_summary: Vec<(i64, &str, Option<i64>)> = vision_skills
        .iter()
        .map(|e| (e.skill_id, e.source.as_str(), e.level))
        .collect();
    assert_eq!(
        vision_summary,
        vec![
            (95001, "vision_awakening", Some(0)),
            (95003, "vision_finish_blow", None),
            (95005, "vision_synchro", Some(2)),
        ]
    );

    let boss_skills = db.skills_of_unit(9201).expect("skills_of_unit 9201");
    assert!(
        boss_skills
            .iter()
            .any(|e| e.skill_id == 95003 && e.source == "ai_high_risk" && e.skill.is_some())
    );
    // Dangling counter skill 99988 is preserved with skill = None.
    assert!(
        boss_skills
            .iter()
            .any(|e| e.skill_id == 99988 && e.source == "ai_counter" && e.skill.is_none())
    );

    let mob_skills = db.skills_of_unit(9101).expect("skills_of_unit 9101");
    assert!(
        mob_skills
            .iter()
            .any(|e| e.skill_id == 95005 && e.source == "ai_rand1" && e.skill.is_some())
    );

    // 4. groups_containing deduplicates duplicate slots in group 9801 ([9101, 9101]).
    let groups_9101 = db.groups_containing(9101).expect("groups_containing 9101");
    assert_eq!(
        groups_9101.iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![9801, 9802, 9803]
    );
    let groups_9203 = db.groups_containing(9203).expect("groups_containing 9203");
    assert_eq!(
        groups_9203.iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![9804]
    );

    // 5. Encounters linked to battle groups.
    let enc_9801 = db
        .encounters_for_group(9801)
        .expect("encounters_for_group 9801");
    assert_eq!(enc_9801.len(), 2);
    let pool_entries = db
        .encounters_in_pool("Encount/00Test/DT_00Test_Encount")
        .expect("encounters_in_pool");
    assert_eq!(pool_entries.len(), 3);

    // 6. Enemy <-> Chapter alignment against EnemyChapterIndex.
    for ch in chapters.chapters() {
        let expected_profiles = chapters.enemy_profiles_in_chapter(&battle, ch);
        let actual_profiles = db.enemies_in_chapter(ch).expect("enemies_in_chapter");
        assert_eq!(
            actual_profiles, expected_profiles,
            "Chapter {ch} enemy profiles mismatch"
        );
    }
    for unit in &battle.units {
        assert_eq!(
            db.chapters_of_enemy(unit.id).expect("chapters_of_enemy"),
            chapters.chapters_for_enemy(unit.id),
            "Chapters for unit {} mismatch",
            unit.id
        );
        assert_eq!(
            db.enemy_chapter_sources(unit.id)
                .expect("enemy_chapter_sources"),
            chapters.sources_for_enemy(unit.id),
            "Chapter sources for unit {} mismatch",
            unit.id
        );
    }
    assert_eq!(db.chapters_of_enemy(9101).expect("ch 9101"), vec![1]);
    assert_eq!(db.chapters_of_enemy(9201).expect("ch 9201"), vec![1]);
    assert_eq!(db.chapters_of_enemy(9202).expect("ch 9202"), vec![2]);
    assert_eq!(db.chapters_of_enemy(9203).expect("ch 9203"), vec![2]);
    assert!(
        db.enemy_chapter_sources(9101)
            .expect("sources 9101")
            .iter()
            .any(|s| s.kind == EnemyChapterSourceKind::EncounterPool)
    );

    // 7. Weaknesses, resistances, categories, and skill elements.
    let weak_ice = db.units_weak_to(Element::Ice).expect("units_weak_to Ice");
    assert_eq!(
        weak_ice.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![9201, 9203]
    );
    let absorb_fire = db
        .units_by_resistance(Element::Fire, Resistance::Absorb)
        .expect("units absorbing Fire");
    assert_eq!(
        absorb_fire.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![9201]
    );
    let bosses = db
        .units_by_category(UnitCategory::Boss)
        .expect("units_by_category Boss");
    assert_eq!(bosses.iter().map(|u| u.id).collect::<Vec<_>>(), vec![9201]);
    let ice_skills = db
        .skills_by_element(Element::Ice)
        .expect("skills_by_element Ice");
    assert_eq!(
        ice_skills.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![95001, 95005, 95006]
    );

    // 8. Dangling references recorded in build_issue (never dropped, never aborted).
    let issues = db.build_issues().expect("read build_issues");
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "dangling_unit_ref" && i.subject == "battle_group:9803"),
        "expected dangling_unit_ref for battle_group:9803, got {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "dangling_unit_ref" && i.subject == "command_skill:101"),
        "expected dangling_unit_ref for command_skill:101, got {issues:?}"
    );
    assert!(
        issues.iter().any(|i| {
            i.kind == "dangling_battle_group_ref"
                && i.subject == "encounter:Encount/00Test/DT_00Test_Encount:PoolDangling"
        }),
        "expected dangling_battle_group_ref for PoolDangling, got {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == "dangling_skill_ref"
                && i.subject == "unit_skill:9201:ai_counter:99988"),
        "expected dangling_skill_ref for unit_skill:9201:ai_counter:99988, got {issues:?}"
    );

    // 9. Query-plan checks via assert_db_uses_index across all battle tables.
    let tables = &[
        "unit",
        "unit_name",
        "unit_name_fts",
        "unit_resistance",
        "skill",
        "skill_name",
        "skill_name_fts",
        "unit_skill",
        "command_skill",
        "battle_group",
        "battle_group_member",
        "encounter",
        "unit_ai",
        "battle_param",
        "enemy_chapter",
        "unit_untranslated",
        "skill_untranslated",
        "command_skill_untranslated",
    ];
    assert_db_uses_index(db, "SELECT doc FROM unit WHERE id = ?1", [9101_i64], tables);
    assert_db_uses_index(
        db,
        "SELECT id, doc FROM unit WHERE name_zh = ?1 ORDER BY id",
        ["霜翼试作兽"],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT u.id, f.name, u.doc FROM unit_name_fts f JOIN unit u ON u.id = f.unit_id WHERE f.name LIKE ?1 AND f.lang = ?2 AND f.name != ?3 ORDER BY u.id",
        ["%霜翼%", "zh-Hans", "霜翼"],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT u.id, u.doc FROM unit_resistance r JOIN unit u ON u.id = r.unit_id WHERE r.element = ?1 AND r.resistance = ?2 ORDER BY u.id",
        ["Ice", "Weakness"],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT DISTINCT g.id, g.doc FROM battle_group_member m JOIN battle_group g ON g.id = m.group_id WHERE m.unit_id = ?1 ORDER BY g.id",
        [9101_i64],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT u.id, u.doc, a.doc FROM (SELECT DISTINCT unit_id FROM enemy_chapter WHERE chapter_no = ?1) ec JOIN unit u ON u.id = ec.unit_id LEFT JOIN unit_ai a ON a.unit_id = u.id ORDER BY u.id",
        [1_i64],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT DISTINCT chapter_no FROM enemy_chapter WHERE unit_id = ?1 ORDER BY chapter_no",
        [9101_i64],
        tables,
    );

    // 10. Catalog, exact-name, and availability queries.
    let exact_frost = db
        .enemies_by_exact_name_zh("霜翼试作兽")
        .expect("enemies_by_exact_name_zh");
    assert_eq!(
        exact_frost.iter().map(|(u, _)| u.id).collect::<Vec<_>>(),
        vec![9101, 9202]
    );
    let exact_ice = db
        .skills_by_exact_name_zh("试作冰刺")
        .expect("skills_by_exact_name_zh");
    assert_eq!(
        exact_ice.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![95001, 95006]
    );
    let all_enemies = db.all_enemies_catalog().expect("all_enemies_catalog");
    let expected_enemy_ids: Vec<i64> = battle
        .units
        .iter()
        .filter(|u| u.category.is_enemy_category())
        .map(|u| u.id)
        .collect();
    assert_eq!(
        all_enemies.iter().map(|r| r.unit.id).collect::<Vec<_>>(),
        expected_enemy_ids
    );
    let all_skills = db.all_skills_catalog().expect("all_skills_catalog");
    assert_eq!(all_skills.len(), battle.skills.len());
    let all_enemy_chapters = db.all_enemy_chapters().expect("all_enemy_chapters");
    assert_eq!(all_enemy_chapters.get(&9101), Some(&vec![1]));
    assert!(db.has_battle_data().expect("has_battle_data"));
    assert!(db.has_command_skills().expect("has_command_skills"));
    assert!(
        db.enemy_chapters_available()
            .expect("enemy_chapters_available")
    );

    assert_db_uses_index(
        db,
        "SELECT u.id, u.doc, a.doc FROM unit u LEFT JOIN unit_ai a ON a.unit_id = u.id \
         WHERE u.name_zh = ?1 AND u.category IN ('Enemy', 'Boss') ORDER BY u.id",
        ["霜翼试作兽"],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT u.id, u.doc, a.doc, u.name_zh FROM unit u INDEXED BY idx_unit_category \
         LEFT JOIN unit_ai a ON a.unit_id = u.id \
         WHERE u.category IN ('Enemy', 'Boss') ORDER BY u.id",
        [],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT DISTINCT unit_id, chapter_no FROM enemy_chapter ORDER BY unit_id, chapter_no",
        [],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT id, name_zh, doc FROM skill WHERE id >= ?1 ORDER BY id",
        [i64::MIN],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT 1 FROM build_issue WHERE stage = 'battle' AND kind = 'missing_optional_table' \
         AND subject = 'enemy_chapter' LIMIT 1",
        [],
        &["build_issue"],
    );

    // 11. Companion `*_untranslated` queries & index verification.
    assert!(db.untranslated_units("zh-Hans").unwrap().is_empty());
    assert!(db.unit_untranslated_fields(9101).unwrap().is_empty());
    assert!(db.untranslated_skills("zh-Hans").unwrap().is_empty());
    assert!(db.skill_untranslated_fields(95001).unwrap().is_empty());
    assert!(
        db.untranslated_command_skills("zh-Hans")
            .unwrap()
            .is_empty()
    );
    assert!(db.command_skill_untranslated_fields(1).unwrap().is_empty());

    assert_db_uses_index(
        db,
        "SELECT unit_id, field, lang, text_ns, text_key, value \
         FROM unit_untranslated WHERE lang = ?1 ORDER BY unit_id, field",
        ["zh-Hans"],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT unit_id, field, lang, text_ns, text_key, value \
         FROM unit_untranslated WHERE unit_id = ?1 ORDER BY field, lang",
        [9101_i64],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT skill_id, field, lang, text_ns, text_key, value \
         FROM skill_untranslated WHERE lang = ?1 ORDER BY skill_id, field",
        ["zh-Hans"],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT skill_id, field, lang, text_ns, text_key, value \
         FROM skill_untranslated WHERE skill_id = ?1 ORDER BY field, lang",
        [95001_i64],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT command_skill_id, field, lang, text_ns, text_key, value \
         FROM command_skill_untranslated WHERE lang = ?1 ORDER BY command_skill_id, field",
        ["zh-Hans"],
        tables,
    );
    assert_db_uses_index(
        db,
        "SELECT command_skill_id, field, lang, text_ns, text_key, value \
         FROM command_skill_untranslated WHERE command_skill_id = ?1 ORDER BY field, lang",
        [1_i64],
        tables,
    );
}

#[test]
fn minimal_battle_fixture_without_optional_tables_builds_cleanly() {
    let minimal_dir = repo_root().join("tests/fixtures/rust/battle");
    let test_db = build_temp_db(
        "battle-minimal",
        BuildInputs {
            export: Some(minimal_dir.clone()),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Battle]),
    )
    .expect("build battle stage on minimal tests/fixtures/rust/battle");

    let db = test_db.db();
    let battle = DataStore::new(&minimal_dir)
        .battle()
        .expect("load minimal BattleData");
    for expected in &battle.units {
        assert_eq!(db.unit(expected.id).expect("unit").as_ref(), Some(expected));
    }
    assert!(
        db.enemies_in_chapter(1)
            .expect("empty enemy_chapter")
            .is_empty()
    );
    assert_eq!(
        db.break_calc().expect("break_calc"),
        Some(battle.break_calc)
    );
}

#[test]
fn real_export_full_battle_alignment_and_spot_checks() {
    let export_root = real_export_dir();
    let unit_file = datatable_dir(&export_root).join("Unit/DT_UnitParameter.json");
    if !unit_file.is_file() || !localization_file(&export_root, "zh-Hans").is_file() {
        eprintln!(
            "跳过真实导出战斗数据库对齐测试：未找到 {} 或 zh-Hans.json",
            unit_file.display()
        );
        return;
    }

    let test_db = build_temp_db(
        "battle-real",
        BuildInputs {
            export: Some(export_root.clone()),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Battle]),
    )
    .expect("build battle stage on real export");
    let db = test_db.db();

    let store = DataStore::new(&export_root);
    let battle = store.battle().expect("load real BattleData");
    let commands = CommandSkillData::from_store(&store).expect("load real CommandSkillData");
    let chapters = EnemyChapterIndex::from_store_with_battle(&store, &battle)
        .expect("build real EnemyChapterIndex");

    // Full exact alignment across all units, enemy profiles, AI profiles, skills, battle groups,
    // command skills, and break calc params.
    for expected in &battle.units {
        let actual = db
            .unit(expected.id)
            .expect("query real unit")
            .expect("real unit must exist");
        assert_eq!(&actual, expected, "Real Unit {} mismatch", expected.id);

        let expected_profile = battle
            .enemy_profile(expected.id)
            .expect("real reference enemy_profile");
        let actual_profile = db
            .enemy_profile(expected.id)
            .expect("query real enemy_profile")
            .expect("real enemy_profile must exist");
        assert_eq!(
            actual_profile, expected_profile,
            "Real EnemyProfile {} mismatch",
            expected.id
        );
    }

    for (unit_id, expected_ai) in &battle.unit_ai {
        let actual_ai = db
            .unit_ai(*unit_id)
            .expect("query real unit_ai")
            .expect("real unit_ai must exist");
        assert_eq!(&actual_ai, expected_ai);
    }

    for expected in &battle.skills {
        let actual = db
            .skill(expected.id)
            .expect("query real skill")
            .expect("real skill must exist");
        assert_eq!(&actual, expected, "Real Skill {} mismatch", expected.id);
    }

    for expected in &battle.battle_groups {
        let detail = db
            .battle_group(expected.id)
            .expect("query real battle_group")
            .expect("real battle_group must exist");
        assert_eq!(
            &detail.group, expected,
            "Real BattleGroup {} mismatch",
            expected.id
        );
        assert_eq!(detail.members.len(), expected.unit_ids.len());
    }

    for expected in &commands.commands {
        let actual = db
            .command_skill(expected.id)
            .expect("query real command_skill")
            .expect("real command_skill must exist");
        assert_eq!(&actual, expected);
    }

    assert_eq!(
        db.break_calc().expect("query real break_calc"),
        Some(battle.break_calc.clone())
    );

    // Full Enemy <-> Chapter alignment.
    for ch in chapters.chapters() {
        let expected_ids = chapters.enemies_in_chapter(ch);
        let actual_ids: Vec<i64> = db
            .enemies_in_chapter(ch)
            .expect("real enemies_in_chapter")
            .into_iter()
            .map(|p| p.unit_id)
            .collect();
        assert_eq!(actual_ids, expected_ids, "Real chapter {ch} mismatch");
    }
    for unit in &battle.units {
        assert_eq!(
            db.chapters_of_enemy(unit.id)
                .expect("real chapters_of_enemy"),
            chapters.chapters_for_enemy(unit.id)
        );
    }

    // Spot-check 1: 钢蝠 (ID = 2).
    let bat_hits = db
        .units_by_name("钢蝠", "zh-Hans")
        .expect("lookup 钢蝠 by zh-Hans");
    assert_eq!(bat_hits.first().map(|u| u.id), Some(2));
    let bat_profile = db
        .enemy_profile(2)
        .expect("enemy_profile 2")
        .expect("钢蝠 exists");
    assert_eq!(bat_profile.max_hp, 68);
    assert_eq!(bat_profile.max_break_point, 50);
    assert_eq!(bat_profile.weaknesses, vec![Element::Ice, Element::Wind]);
    assert_eq!(db.chapters_of_enemy(2).expect("chapters of 2"), vec![1]);
    let bat_groups: Vec<i64> = db
        .groups_containing(2)
        .expect("groups containing 2")
        .into_iter()
        .map(|g| g.id)
        .collect();
    assert_eq!(bat_groups, vec![1, 2, 3, 4, 1000, 1001, 1064]);

    // Spot-check 2: units weak to Ice matches direct scan of BattleData.
    let expected_weak_ice: BTreeSet<i64> = battle
        .units
        .iter()
        .filter(|u| u.element_resistances.get(&Element::Ice) == Some(&Resistance::Weakness))
        .map(|u| u.id)
        .collect();
    let actual_weak_ice: BTreeSet<i64> = db
        .units_weak_to(Element::Ice)
        .expect("units_weak_to Ice")
        .into_iter()
        .map(|u| u.id)
        .collect();
    assert_eq!(actual_weak_ice, expected_weak_ice);
    assert!(actual_weak_ice.contains(&2));
}
