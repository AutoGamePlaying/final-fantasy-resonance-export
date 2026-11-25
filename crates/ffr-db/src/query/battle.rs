//! Battle area (`03_battle`) `impl GameDb` read queries and shared DB enum codecs.
//!
//! All queries run via `rusqlite::Connection::prepare_cached`, deserialize `doc` JSON columns
//! through [`crate::record`], and use primary or secondary indexes (including FTS5 trigram
//! virtual tables for multilingual substring name search).

use std::collections::{BTreeMap, HashSet};

use ffr_data::MODEL_LANGUAGE;
use ffr_data::battle::{
    BattleGroup, BreakCalcParams, CommandSkill, Element, EncounterPoolEntry, EnemyChapterSource,
    EnemyChapterSourceKind, EnemyProfile, Resistance, Skill, SkillAttributeType, SkillCostType,
    SkillDamageType, SkillTargetType, Unit, UnitAiProfile, UnitCategory, UnitRace,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record;

/// One skill link belonging to a unit, paired with its provenance (`source`, optional `level`)
/// and resolved [`Skill`] document when the target skill exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitSkillEntry {
    pub unit_id: i64,
    pub skill_id: i64,
    pub source: String,
    pub level: Option<i64>,
    /// Resolved `Skill` from `skill`, or `None` when `skill_id` was a dangling reference.
    pub skill: Option<Skill>,
}

/// One positional member slot inside a battle group with its resolved [`Unit`] (if present).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattleGroupMember {
    pub slot: i64,
    pub unit_id: i64,
    /// Resolved `Unit` from `unit`, or `None` when `unit_id` was a dangling reference.
    pub unit: Option<Unit>,
}

/// Battle group record paired with its slot-ordered member units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattleGroupDetail {
    pub group: BattleGroup,
    pub members: Vec<BattleGroupMember>,
}

/// One encounter pool row paired with its source pool path and optional region directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterEntry {
    pub pool_path: String,
    pub region: Option<String>,
    pub entry: EncounterPoolEntry,
}

/// Enemy/boss catalog row with optional AI profile and pre-resolved `zh-Hans` name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnemyCatalogRow {
    pub unit: Unit,
    pub ai: Option<UnitAiProfile>,
    pub name_zh: Option<String>,
}

/// Skill catalog row with pre-resolved `zh-Hans` name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillCatalogRow {
    pub skill: Skill,
    pub name_zh: Option<String>,
}

pub(crate) const SQL_UNIT_BY_ID: &str = "SELECT doc FROM unit WHERE id = ?1";
pub(crate) const SQL_UNITS_EXACT_ZH: &str =
    "SELECT id, doc FROM unit WHERE name_zh = ?1 ORDER BY id";
pub(crate) const SQL_ENEMIES_EXACT_ZH: &str = "SELECT u.id, u.doc, a.doc \
     FROM unit u \
     LEFT JOIN unit_ai a ON a.unit_id = u.id \
     WHERE u.name_zh = ?1 AND u.category IN ('Enemy', 'Boss') \
     ORDER BY u.id";
pub(crate) const SQL_ALL_ENEMIES_CATALOG: &str = "SELECT u.id, u.doc, a.doc, u.name_zh \
     FROM unit u INDEXED BY idx_unit_category \
     LEFT JOIN unit_ai a ON a.unit_id = u.id \
     WHERE u.category IN ('Enemy', 'Boss') \
     ORDER BY u.id";
pub(crate) const SQL_ALL_ENEMY_CHAPTERS: &str = "SELECT DISTINCT unit_id, chapter_no \
     FROM enemy_chapter \
     ORDER BY unit_id, chapter_no";
pub(crate) const SQL_ALL_SKILLS_CATALOG: &str = "SELECT id, name_zh, doc \
     FROM skill \
     WHERE id >= ?1 \
     ORDER BY id";
pub(crate) const SQL_HAS_BATTLE_DATA: &str = "SELECT 1 FROM unit WHERE id >= ?1 LIMIT 1";
pub(crate) const SQL_HAS_COMMAND_SKILLS: &str =
    "SELECT 1 FROM command_skill WHERE id >= ?1 LIMIT 1";
pub(crate) const SQL_MISSING_ENEMY_CHAPTER_ISSUE: &str = "SELECT 1 FROM build_issue \
     WHERE stage = 'battle' AND kind = 'missing_optional_table' AND subject = 'enemy_chapter' \
     LIMIT 1";
pub(crate) const SQL_UNITS_EXACT_LANG: &str = "SELECT u.id, u.doc \
     FROM unit_name n \
     JOIN unit u ON u.id = n.unit_id \
     WHERE n.lang = ?1 AND n.name = ?2 \
     ORDER BY u.id";
pub(crate) const SQL_UNITS_CONTAINS_FTS: &str = "SELECT u.id, f.name, u.doc \
     FROM unit_name_fts f \
     JOIN unit u ON u.id = f.unit_id \
     WHERE f.name LIKE ?1 AND f.lang = ?2 AND f.name != ?3 \
     ORDER BY u.id";
pub(crate) const SQL_ENEMY_PROFILE: &str = "SELECT u.doc, a.doc \
     FROM unit u \
     LEFT JOIN unit_ai a ON a.unit_id = u.id \
     WHERE u.id = ?1";
pub(crate) const SQL_SKILL_BY_ID: &str = "SELECT doc FROM skill WHERE id = ?1";
pub(crate) const SQL_SKILLS_EXACT_ZH: &str =
    "SELECT id, doc FROM skill WHERE name_zh = ?1 ORDER BY id";
pub(crate) const SQL_SKILLS_EXACT_LANG: &str = "SELECT s.id, s.doc \
     FROM skill_name n \
     JOIN skill s ON s.id = n.skill_id \
     WHERE n.lang = ?1 AND n.name = ?2 \
     ORDER BY s.id";
pub(crate) const SQL_SKILLS_CONTAINS_FTS: &str = "SELECT s.id, f.name, s.doc \
     FROM skill_name_fts f \
     JOIN skill s ON s.id = f.skill_id \
     WHERE f.name LIKE ?1 AND f.lang = ?2 AND f.name != ?3 \
     ORDER BY s.id";
pub(crate) const SQL_SKILLS_OF_UNIT: &str = "SELECT us.unit_id, us.skill_id, us.source, us.level, s.doc \
     FROM unit_skill us \
     LEFT JOIN skill s ON s.id = us.skill_id \
     WHERE us.unit_id = ?1 \
     ORDER BY us.source, us.level, us.skill_id";
pub(crate) const SQL_BATTLE_GROUP_BY_ID: &str = "SELECT doc FROM battle_group WHERE id = ?1";
pub(crate) const SQL_BATTLE_GROUP_MEMBERS: &str = "SELECT m.slot, m.unit_id, u.doc \
     FROM battle_group_member m \
     LEFT JOIN unit u ON u.id = m.unit_id \
     WHERE m.group_id = ?1 \
     ORDER BY m.slot";
pub(crate) const SQL_GROUPS_CONTAINING: &str = "SELECT DISTINCT g.id, g.doc \
     FROM battle_group_member m \
     JOIN battle_group g ON g.id = m.group_id \
     WHERE m.unit_id = ?1 \
     ORDER BY g.id";
pub(crate) const SQL_ENEMIES_IN_CHAPTER: &str = "SELECT u.id, u.doc, a.doc \
     FROM (SELECT DISTINCT unit_id FROM enemy_chapter WHERE chapter_no = ?1) ec \
     JOIN unit u ON u.id = ec.unit_id \
     LEFT JOIN unit_ai a ON a.unit_id = u.id \
     ORDER BY u.id";
pub(crate) const SQL_CHAPTERS_OF_ENEMY: &str = "SELECT DISTINCT chapter_no \
     FROM enemy_chapter \
     WHERE unit_id = ?1 \
     ORDER BY chapter_no";
pub(crate) const SQL_ENEMY_CHAPTER_SOURCES: &str = "SELECT chapter_no, source, battle_group_id, source_ref \
     FROM enemy_chapter \
     WHERE unit_id = ?1 \
     ORDER BY chapter_no, source, battle_group_id, source_ref";
pub(crate) const SQL_UNITS_BY_RESISTANCE: &str = "SELECT u.id, u.doc \
     FROM unit_resistance r \
     JOIN unit u ON u.id = r.unit_id \
     WHERE r.element = ?1 AND r.resistance = ?2 \
     ORDER BY u.id";
pub(crate) const SQL_UNITS_BY_CATEGORY: &str =
    "SELECT id, doc FROM unit INDEXED BY idx_unit_category WHERE category = ?1 ORDER BY id";
pub(crate) const SQL_SKILLS_BY_ELEMENT: &str =
    "SELECT id, doc FROM skill INDEXED BY idx_skill_element WHERE element = ?1 ORDER BY id";
pub(crate) const SQL_COMMAND_SKILL_BY_ID: &str = "SELECT doc FROM command_skill WHERE id = ?1";
pub(crate) const SQL_COMMAND_SKILLS_FOR_UNIT: &str =
    "SELECT id, doc FROM command_skill WHERE unit_id = ?1 ORDER BY id";
pub(crate) const SQL_UNIT_AI_BY_ID: &str = "SELECT doc FROM unit_ai WHERE unit_id = ?1";
pub(crate) const SQL_ENCOUNTERS_FOR_GROUP: &str = "SELECT pool_path, region, row_key, doc \
     FROM encounter \
     WHERE battle_group_id = ?1 \
     ORDER BY pool_path, row_key";
pub(crate) const SQL_ENCOUNTERS_IN_POOL: &str = "SELECT pool_path, region, row_key, doc \
     FROM encounter \
     WHERE pool_path = ?1 \
     ORDER BY row_key";
pub(crate) const SQL_BREAK_CALC: &str = "SELECT doc FROM battle_param WHERE id = 1";

impl GameDb {
    /// Fetch one [`Unit`] by primary key `id`.
    pub fn unit(&self, id: i64) -> Result<Option<Unit>> {
        self.with_conn(|conn| record::query_doc_opt(conn, "unit", id, SQL_UNIT_BY_ID, [id]))
    }

    /// Find units matching `query` in `lang`: exact matches first (all duplicates included,
    /// ordered by `id`), followed by substring matches (ordered by `id`).
    pub fn units_by_name(&self, query: &str, lang: &str) -> Result<Vec<Unit>> {
        if query.is_empty() || lang.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_named_docs(
                conn,
                "unit",
                query,
                lang,
                SQL_UNITS_EXACT_ZH,
                SQL_UNITS_EXACT_LANG,
                SQL_UNITS_CONTAINS_FTS,
            )
        })
    }

    /// Build a battle-focused [`EnemyProfile`] for unit `id` (joining `unit` and optional
    /// `unit_ai`). Returns `Ok(None)` if no unit with `id` exists.
    pub fn enemy_profile(&self, id: i64) -> Result<Option<EnemyProfile>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_ENEMY_PROFILE)
                .map_err(|err| DbError::sqlite("准备查询敌人资料", err))?;
            let pair: Option<(String, Option<String>)> = stmt
                .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()
                .map_err(|err| DbError::sqlite(format!("查询单位 {id} 的敌人资料"), err))?;
            let Some((unit_raw, ai_raw)) = pair else {
                return Ok(None);
            };
            let unit: Unit = record::decode_doc("unit", id, &unit_raw)?;
            let ai: Option<UnitAiProfile> = ai_raw
                .map(|raw| record::decode_doc("unit_ai", id, &raw))
                .transpose()?;
            Ok(Some(unit.to_enemy_profile(ai.as_ref())))
        })
    }

    /// Fetch one [`Skill`] by primary key `id`.
    pub fn skill(&self, id: i64) -> Result<Option<Skill>> {
        self.with_conn(|conn| record::query_doc_opt(conn, "skill", id, SQL_SKILL_BY_ID, [id]))
    }

    /// Find skills matching `query` in `lang`: exact matches first (all duplicates included,
    /// ordered by `id`), followed by substring matches (ordered by `id`).
    pub fn skills_by_name(&self, query: &str, lang: &str) -> Result<Vec<Skill>> {
        if query.is_empty() || lang.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_named_docs(
                conn,
                "skill",
                query,
                lang,
                SQL_SKILLS_EXACT_ZH,
                SQL_SKILLS_EXACT_LANG,
                SQL_SKILLS_CONTAINS_FTS,
            )
        })
    }

    /// List all skills associated with unit `id`, including provenance (`source`, `level`) and
    /// the resolved [`Skill`] document when present.
    pub fn skills_of_unit(&self, id: i64) -> Result<Vec<UnitSkillEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_SKILLS_OF_UNIT)
                .map_err(|err| DbError::sqlite("准备查询单位招式列表", err))?;
            let mut rows = stmt
                .query([id])
                .map_err(|err| DbError::sqlite(format!("查询单位 {id} 的招式列表"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取单位招式行", err))?
            {
                let unit_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 unit_skill.unit_id", err))?;
                let skill_id: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 unit_skill.skill_id", err))?;
                let source: String = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 unit_skill.source", err))?;
                let level: Option<i64> = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 unit_skill.level", err))?;
                let skill_raw: Option<String> = row
                    .get(4)
                    .map_err(|err| DbError::sqlite("读取 skill.doc", err))?;
                let skill = skill_raw
                    .map(|raw| record::decode_doc("skill", skill_id, &raw))
                    .transpose()?;
                out.push(UnitSkillEntry {
                    unit_id,
                    skill_id,
                    source,
                    level,
                    skill,
                });
            }
            Ok(out)
        })
    }

    /// Fetch one battle group by `id` with its positional member slots resolved into [`Unit`]s.
    pub fn battle_group(&self, id: i64) -> Result<Option<BattleGroupDetail>> {
        self.with_conn(|conn| {
            let Some(group): Option<BattleGroup> =
                record::query_doc_opt(conn, "battle_group", id, SQL_BATTLE_GROUP_BY_ID, [id])?
            else {
                return Ok(None);
            };
            let mut stmt = conn
                .prepare_cached(SQL_BATTLE_GROUP_MEMBERS)
                .map_err(|err| DbError::sqlite("准备查询战斗组成员", err))?;
            let mut rows = stmt
                .query([id])
                .map_err(|err| DbError::sqlite(format!("查询战斗组 {id} 成员"), err))?;
            let mut members = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取战斗组成员行", err))?
            {
                let slot: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 battle_group_member.slot", err))?;
                let unit_id: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 battle_group_member.unit_id", err))?;
                let unit_raw: Option<String> = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取成员单位 doc", err))?;
                let unit = unit_raw
                    .map(|raw| record::decode_doc("unit", unit_id, &raw))
                    .transpose()?;
                members.push(BattleGroupMember {
                    slot,
                    unit_id,
                    unit,
                });
            }
            Ok(Some(BattleGroupDetail { group, members }))
        })
    }

    /// Return all distinct [`BattleGroup`]s that include `unit_id`, ordered by group `id`.
    pub fn groups_containing(&self, unit_id: i64) -> Result<Vec<BattleGroup>> {
        self.with_conn(|conn| {
            record::query_docs(conn, "battle_group", SQL_GROUPS_CONTAINING, [unit_id])
        })
    }

    /// Return all distinct enemy [`EnemyProfile`]s mapped to `chapter_no`, ordered by `unit_id`.
    pub fn enemies_in_chapter(&self, chapter_no: i64) -> Result<Vec<EnemyProfile>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_ENEMIES_IN_CHAPTER)
                .map_err(|err| DbError::sqlite("准备按章节查询敌人", err))?;
            let mut rows = stmt
                .query([chapter_no])
                .map_err(|err| DbError::sqlite(format!("查询第 {chapter_no} 章敌人"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取章节敌人行", err))?
            {
                let unit_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取敌人单位编号", err))?;
                let unit_raw: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取敌人 unit.doc", err))?;
                let ai_raw: Option<String> = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取敌人 unit_ai.doc", err))?;
                let unit: Unit = record::decode_doc("unit", unit_id, &unit_raw)?;
                let ai: Option<UnitAiProfile> = ai_raw
                    .map(|raw| record::decode_doc("unit_ai", unit_id, &raw))
                    .transpose()?;
                out.push(unit.to_enemy_profile(ai.as_ref()));
            }
            Ok(out)
        })
    }

    /// Return the sorted distinct chapter numbers in which enemy `unit_id` appears.
    pub fn chapters_of_enemy(&self, unit_id: i64) -> Result<Vec<i64>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_CHAPTERS_OF_ENEMY)
                .map_err(|err| DbError::sqlite("准备查询敌人所属章节", err))?;
            let mut rows = stmt
                .query([unit_id])
                .map_err(|err| DbError::sqlite(format!("查询敌人 {unit_id} 所属章节"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取敌人所属章节行", err))?
            {
                let chapter_no: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 enemy_chapter.chapter_no", err))?;
                out.push(chapter_no);
            }
            Ok(out)
        })
    }

    /// Return all provenance records explaining why enemy `unit_id` maps to its chapters.
    pub fn enemy_chapter_sources(&self, unit_id: i64) -> Result<Vec<EnemyChapterSource>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_ENEMY_CHAPTER_SOURCES)
                .map_err(|err| DbError::sqlite("准备查询敌人章节来源", err))?;
            let mut rows = stmt
                .query([unit_id])
                .map_err(|err| DbError::sqlite(format!("查询敌人 {unit_id} 的章节来源"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取敌人章节来源行", err))?
            {
                let chapter_no: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 enemy_chapter.chapter_no", err))?;
                let source_str: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 enemy_chapter.source", err))?;
                let kind = parse_enemy_chapter_source_kind(&source_str).ok_or_else(|| {
                    DbError::sqlite(
                        format!("解析 enemy_chapter.source 失败（值 {source_str}）"),
                        rusqlite::Error::InvalidQuery,
                    )
                })?;
                let battle_group_id: i64 = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 enemy_chapter.battle_group_id", err))?;
                let source_ref: String = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 enemy_chapter.source_ref", err))?;
                out.push(EnemyChapterSource {
                    chapter_no,
                    kind,
                    battle_group_id,
                    source_ref,
                });
            }
            Ok(out)
        })
    }

    /// Return all [`Unit`]s whose resistance to `element` is `Resistance::Weakness`, ordered by
    /// `id`.
    pub fn units_weak_to(&self, element: Element) -> Result<Vec<Unit>> {
        self.units_by_resistance(element, Resistance::Weakness)
    }

    /// Return all [`Unit`]s having a specific `(element, resistance)` affinity, ordered by `id`.
    pub fn units_by_resistance(
        &self,
        element: Element,
        resistance: Resistance,
    ) -> Result<Vec<Unit>> {
        self.with_conn(|conn| {
            record::query_docs(
                conn,
                "unit",
                SQL_UNITS_BY_RESISTANCE,
                [element_to_db(element), resistance_to_db(resistance)],
            )
        })
    }

    /// Return all [`Unit`]s in a given [`UnitCategory`], ordered by `id`.
    pub fn units_by_category(&self, category: UnitCategory) -> Result<Vec<Unit>> {
        self.with_conn(|conn| {
            record::query_docs(
                conn,
                "unit",
                SQL_UNITS_BY_CATEGORY,
                [unit_category_to_db(category)],
            )
        })
    }

    /// Return all [`Skill`]s with a given [`Element`], ordered by `id`.
    pub fn skills_by_element(&self, element: Element) -> Result<Vec<Skill>> {
        self.with_conn(|conn| {
            record::query_docs(
                conn,
                "skill",
                SQL_SKILLS_BY_ELEMENT,
                [element_to_db(element)],
            )
        })
    }

    /// Fetch one [`CommandSkill`] by primary key `id`.
    pub fn command_skill(&self, id: i64) -> Result<Option<CommandSkill>> {
        self.with_conn(|conn| {
            record::query_doc_opt(conn, "command_skill", id, SQL_COMMAND_SKILL_BY_ID, [id])
        })
    }

    /// Return all [`CommandSkill`] entries bound to `unit_id`, ordered by `id`.
    pub fn command_skills_for_unit(&self, unit_id: i64) -> Result<Vec<CommandSkill>> {
        self.with_conn(|conn| {
            record::query_docs(
                conn,
                "command_skill",
                SQL_COMMAND_SKILLS_FOR_UNIT,
                [unit_id],
            )
        })
    }

    /// Fetch one [`UnitAiProfile`] by `unit_id`.
    pub fn unit_ai(&self, unit_id: i64) -> Result<Option<UnitAiProfile>> {
        self.with_conn(|conn| {
            record::query_doc_opt(conn, "unit_ai", unit_id, SQL_UNIT_AI_BY_ID, [unit_id])
        })
    }

    /// Return all encounter pool entries referencing `group_id`.
    pub fn encounters_for_group(&self, group_id: i64) -> Result<Vec<EncounterEntry>> {
        self.with_conn(|conn| query_encounters(conn, SQL_ENCOUNTERS_FOR_GROUP, [group_id]))
    }

    /// Return all encounter pool entries belonging to `pool_path` (for example
    /// `"Encount/01Gra/DT_01Gra_40_01_Encount"`).
    pub fn encounters_in_pool(&self, pool_path: &str) -> Result<Vec<EncounterEntry>> {
        self.with_conn(|conn| query_encounters(conn, SQL_ENCOUNTERS_IN_POOL, [pool_path]))
    }

    /// Fetch global break & elemental damage calculation parameters (`battle_param`).
    pub fn break_calc(&self) -> Result<Option<BreakCalcParams>> {
        self.with_conn(|conn| record::query_doc_opt(conn, "battle_param", 1, SQL_BREAK_CALC, []))
    }

    /// Find combat enemy/boss units whose `zh-Hans` display name matches `name` exactly, paired
    /// with their optional [`UnitAiProfile`] and ordered by `id`.
    pub fn enemies_by_exact_name_zh(
        &self,
        name: &str,
    ) -> Result<Vec<(Unit, Option<UnitAiProfile>)>> {
        if name.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_ENEMIES_EXACT_ZH)
                .map_err(|err| DbError::sqlite("准备按中文精确查询敌人", err))?;
            let mut rows = stmt
                .query([name])
                .map_err(|err| DbError::sqlite(format!("按中文精确查询敌人 {name}"), err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取精确敌人查询结果", err))?
            {
                let unit_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 unit.id", err))?;
                let unit_raw: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 unit.doc", err))?;
                let ai_raw: Option<String> = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 unit_ai.doc", err))?;
                let unit: Unit = record::decode_doc("unit", unit_id, &unit_raw)?;
                let ai: Option<UnitAiProfile> = ai_raw
                    .map(|raw| record::decode_doc("unit_ai", unit_id, &raw))
                    .transpose()?;
                out.push((unit, ai));
            }
            Ok(out)
        })
    }

    /// Find all skills whose `zh-Hans` display name matches `name` exactly, ordered by `id`.
    pub fn skills_by_exact_name_zh(&self, name: &str) -> Result<Vec<Skill>> {
        if name.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| record::query_docs(conn, "skill", SQL_SKILLS_EXACT_ZH, [name]))
    }

    /// Return all enemy/boss catalog rows (joined with optional AI profiles), ordered by `id`.
    pub fn all_enemies_catalog(&self) -> Result<Vec<EnemyCatalogRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_ALL_ENEMIES_CATALOG)
                .map_err(|err| DbError::sqlite("准备查询敌人目录", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("查询敌人目录", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取敌人目录行", err))?
            {
                let unit_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 unit.id", err))?;
                let unit_raw: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 unit.doc", err))?;
                let ai_raw: Option<String> = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 unit_ai.doc", err))?;
                let name_zh: Option<String> = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 unit.name_zh", err))?;
                let unit: Unit = record::decode_doc("unit", unit_id, &unit_raw)?;
                let ai: Option<UnitAiProfile> = ai_raw
                    .map(|raw| record::decode_doc("unit_ai", unit_id, &raw))
                    .transpose()?;
                out.push(EnemyCatalogRow { unit, ai, name_zh });
            }
            Ok(out)
        })
    }

    /// Return the full enemy-to-chapters map (`unit_id -> sorted distinct chapters`) using the
    /// covering index `idx_enemy_chapter_unit`.
    pub fn all_enemy_chapters(&self) -> Result<BTreeMap<i64, Vec<i64>>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_ALL_ENEMY_CHAPTERS)
                .map_err(|err| DbError::sqlite("准备查询全部敌人章节映射", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("查询全部敌人章节映射", err))?;
            let mut map: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取敌人章节映射行", err))?
            {
                let unit_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 enemy_chapter.unit_id", err))?;
                let chapter_no: i64 = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 enemy_chapter.chapter_no", err))?;
                map.entry(unit_id).or_default().push(chapter_no);
            }
            Ok(map)
        })
    }

    /// Return all skill catalog rows ordered by `id`.
    pub fn all_skills_catalog(&self) -> Result<Vec<SkillCatalogRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_ALL_SKILLS_CATALOG)
                .map_err(|err| DbError::sqlite("准备查询技能目录", err))?;
            let mut rows = stmt
                .query([i64::MIN])
                .map_err(|err| DbError::sqlite("查询技能目录", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取技能目录行", err))?
            {
                let skill_id: i64 = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 skill.id", err))?;
                let name_zh: Option<String> = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 skill.name_zh", err))?;
                let skill_raw: String = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 skill.doc", err))?;
                let skill: Skill = record::decode_doc("skill", skill_id, &skill_raw)?;
                out.push(SkillCatalogRow { skill, name_zh });
            }
            Ok(out)
        })
    }

    /// Returns `true` if the `unit` table contains at least one row.
    pub fn has_battle_data(&self) -> Result<bool> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_HAS_BATTLE_DATA)
                .map_err(|err| DbError::sqlite("准备检查战斗数据是否就绪", err))?;
            Ok(stmt
                .query_row([i64::MIN], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|err| DbError::sqlite("检查战斗数据是否就绪", err))?
                .is_some())
        })
    }

    /// Returns `true` if the `command_skill` table contains at least one row.
    pub fn has_command_skills(&self) -> Result<bool> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_HAS_COMMAND_SKILLS)
                .map_err(|err| DbError::sqlite("准备检查指令技能数据是否就绪", err))?;
            Ok(stmt
                .query_row([i64::MIN], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|err| DbError::sqlite("检查指令技能数据是否就绪", err))?
                .is_some())
        })
    }

    /// Returns `true` if battle data is present and `EnemyChapterIndex` was built without missing
    /// auxiliary tables.
    pub fn enemy_chapters_available(&self) -> Result<bool> {
        if !self.has_battle_data()? {
            return Ok(false);
        }
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(SQL_MISSING_ENEMY_CHAPTER_ISSUE)
                .map_err(|err| DbError::sqlite("准备检查敌人章节映射状态", err))?;
            let missing = stmt
                .query_row([], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|err| DbError::sqlite("检查敌人章节映射状态", err))?
                .is_some();
            Ok(!missing)
        })
    }
}

fn query_named_docs<T>(
    conn: &Connection,
    table: &str,
    query: &str,
    lang: &str,
    sql_exact_zh: &str,
    sql_exact_lang: &str,
    sql_contains_fts: &str,
) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    let mut results = Vec::new();
    let mut seen_ids = HashSet::new();

    // 1. Exact matches first (ordered by ID), using indexed name column / exact lookup table.
    let exact_rows: Vec<(i64, String)> = if lang == MODEL_LANGUAGE {
        let mut stmt = conn
            .prepare_cached(sql_exact_zh)
            .map_err(|err| DbError::sqlite(format!("准备按中文精确查询 {table}"), err))?;
        collect_id_doc_rows(&mut stmt, [query], table)?
    } else {
        let mut stmt = conn
            .prepare_cached(sql_exact_lang)
            .map_err(|err| DbError::sqlite(format!("准备按语言精确查询 {table}"), err))?;
        collect_id_doc_rows(&mut stmt, [lang, query], table)?
    };
    for (id, raw) in exact_rows {
        if seen_ids.insert(id) {
            results.push(record::decode_doc(table, id, &raw)?);
        }
    }

    // 2. Substring matches via FTS5 trigram index (ordered by ID), deduplicated against exact hits.
    let like_pattern = format!("%{query}%");
    let has_like_wildcard = query.contains('%') || query.contains('_');
    let query_lower = has_like_wildcard.then(|| query.to_lowercase());

    let mut stmt = conn
        .prepare_cached(sql_contains_fts)
        .map_err(|err| DbError::sqlite(format!("准备按子串查询 {table}"), err))?;
    let mut rows = stmt
        .query(params![like_pattern, lang, query])
        .map_err(|err| DbError::sqlite(format!("按子串查询 {table}"), err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取 {table} 子串查询结果"), err))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite(format!("读取 {table}.id"), err))?;
        if seen_ids.contains(&id) {
            continue;
        }
        let name: String = row
            .get(1)
            .map_err(|err| DbError::sqlite(format!("读取 {table} 名称"), err))?;
        if let Some(ref needle) = query_lower
            && !name.to_lowercase().contains(needle)
        {
            continue;
        }
        let raw: String = row
            .get(2)
            .map_err(|err| DbError::sqlite(format!("读取 {table}.doc"), err))?;
        if seen_ids.insert(id) {
            results.push(record::decode_doc(table, id, &raw)?);
        }
    }

    Ok(results)
}

fn collect_id_doc_rows<P: rusqlite::Params>(
    stmt: &mut rusqlite::CachedStatement<'_>,
    params: P,
    table: &str,
) -> Result<Vec<(i64, String)>> {
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite(format!("查询表 {table}"), err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取表 {table} 行"), err))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite(format!("读取 {table}.id"), err))?;
        let doc: String = row
            .get(1)
            .map_err(|err| DbError::sqlite(format!("读取 {table}.doc"), err))?;
        out.push((id, doc));
    }
    Ok(out)
}

fn query_encounters<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<EncounterEntry>> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite("准备查询 encounter 表", err))?;
    let mut rows = stmt
        .query(params)
        .map_err(|err| DbError::sqlite("查询 encounter 表", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 encounter 行", err))?
    {
        let pool_path: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 encounter.pool_path", err))?;
        let region: Option<String> = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 encounter.region", err))?;
        let row_key: String = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 encounter.row_key", err))?;
        let raw: String = row
            .get(3)
            .map_err(|err| DbError::sqlite("读取 encounter.doc", err))?;
        let entry: EncounterPoolEntry =
            record::decode_doc("encounter", format!("{pool_path}:{row_key}"), &raw)?;
        out.push(EncounterEntry {
            pool_path,
            region,
            entry,
        });
    }
    Ok(out)
}

pub(crate) const fn element_to_db(element: Element) -> &'static str {
    match element {
        Element::Fire => "Fire",
        Element::Ice => "Ice",
        Element::Wind => "Wind",
        Element::Earth => "Earth",
        Element::Thunder => "Thunder",
        Element::Water => "Water",
        Element::Light => "Light",
        Element::Dark => "Dark",
        Element::None => "None",
    }
}

pub(crate) const fn resistance_to_db(resistance: Resistance) -> &'static str {
    match resistance {
        Resistance::Weakness => "Weakness",
        Resistance::None => "None",
        Resistance::Halved => "Halved",
        Resistance::Invalid => "Invalid",
        Resistance::Absorb => "Absorb",
    }
}

pub(crate) const fn unit_category_to_db(category: UnitCategory) -> &'static str {
    match category {
        UnitCategory::Playable => "Playable",
        UnitCategory::Enemy => "Enemy",
        UnitCategory::Boss => "Boss",
        UnitCategory::Beast => "Beast",
        UnitCategory::Vision => "Vision",
    }
}

pub(crate) const fn unit_race_to_db(race: UnitRace) -> &'static str {
    match race {
        UnitRace::Animal => "Animal",
        UnitRace::Aquatic => "Aquatic",
        UnitRace::Avian => "Avian",
        UnitRace::Beast => "Beast",
        UnitRace::Demon => "Demon",
        UnitRace::Dragon => "Dragon",
        UnitRace::Fairy => "Fairy",
        UnitRace::Human => "Human",
        UnitRace::Insect => "Insect",
        UnitRace::Machina => "Machina",
        UnitRace::Plantoid => "Plantoid",
        UnitRace::Reaper => "Reaper",
        UnitRace::Stone => "Stone",
        UnitRace::Vision => "Vision",
        UnitRace::None => "None",
    }
}

pub(crate) const fn skill_attr_to_db(attr: SkillAttributeType) -> &'static str {
    match attr {
        SkillAttributeType::Fight => "Fight",
        SkillAttributeType::ChaseAttack => "ChaseAttack",
        SkillAttributeType::Ability => "Ability",
        SkillAttributeType::Magic => "Magic",
        SkillAttributeType::LimitBurst => "LimitBurst",
        SkillAttributeType::Item => "Item",
        SkillAttributeType::Summon => "Summon",
        SkillAttributeType::MixItem => "MixItem",
        SkillAttributeType::Throw => "Throw",
        SkillAttributeType::ThrowMoney => "ThrowMoney",
        SkillAttributeType::Dualcast => "Dualcast",
        SkillAttributeType::Mimic => "Mimic",
        SkillAttributeType::MagicSword => "MagicSword",
        SkillAttributeType::Escape => "Escape",
        SkillAttributeType::Mechabo => "Mechabo",
        SkillAttributeType::FinishBlow => "FinishBlow",
    }
}

pub(crate) const fn skill_damage_to_db(damage: SkillDamageType) -> &'static str {
    match damage {
        SkillDamageType::Physic => "Physic",
        SkillDamageType::Magic => "Magic",
        SkillDamageType::None => "None",
    }
}

pub(crate) const fn skill_target_to_db(target: SkillTargetType) -> &'static str {
    match target {
        SkillTargetType::Single => "Single",
        SkillTargetType::Group => "Group",
        SkillTargetType::All => "All",
        SkillTargetType::Random => "Random",
        SkillTargetType::SelfTarget => "SelfTarget",
        SkillTargetType::SingleAndGroup => "SingleAndGroup",
    }
}

pub(crate) const fn skill_cost_to_db(cost: SkillCostType) -> &'static str {
    match cost {
        SkillCostType::Mp => "Mp",
        SkillCostType::Lb => "Lb",
        SkillCostType::Item => "Item",
        SkillCostType::ThrowItem => "ThrowItem",
        SkillCostType::ThrowMoney => "ThrowMoney",
        SkillCostType::HpRatio => "HpRatio",
        SkillCostType::GoodLuck => "GoodLuck",
        SkillCostType::OverClock => "OverClock",
    }
}

pub(crate) const fn enemy_chapter_source_kind_to_db(kind: EnemyChapterSourceKind) -> &'static str {
    match kind {
        EnemyChapterSourceKind::EncounterPool => "encounter-pool",
        EnemyChapterSourceKind::MainStoryEvent => "main-story-event",
        EnemyChapterSourceKind::SubStoryEvent => "sub-story-event",
        EnemyChapterSourceKind::TrapTreasureBox => "trap-treasure-box",
    }
}

pub(crate) fn parse_enemy_chapter_source_kind(raw: &str) -> Option<EnemyChapterSourceKind> {
    match raw {
        "encounter-pool" => Some(EnemyChapterSourceKind::EncounterPool),
        "main-story-event" => Some(EnemyChapterSourceKind::MainStoryEvent),
        "sub-story-event" => Some(EnemyChapterSourceKind::SubStoryEvent),
        "trap-treasure-box" => Some(EnemyChapterSourceKind::TrapTreasureBox),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::Stage;
    use crate::testing::{assert_db_uses_index, build_fixture_db_stages};

    #[test]
    fn every_battle_query_sql_uses_index() {
        let test_db = build_fixture_db_stages("battle", Some(vec![Stage::Battle]))
            .expect("build battle fixture db");
        let db = test_db.db();
        let battle_tables = &[
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
        ];

        assert_db_uses_index(db, SQL_UNIT_BY_ID, [9101_i64], battle_tables);
        assert_db_uses_index(db, SQL_UNITS_EXACT_ZH, ["霜翼试作兽"], battle_tables);
        assert_db_uses_index(
            db,
            SQL_UNITS_EXACT_LANG,
            ["en", "Core Guardian"],
            battle_tables,
        );
        assert_db_uses_index(
            db,
            SQL_UNITS_CONTAINS_FTS,
            ["%霜翼%", "zh-Hans", "霜翼"],
            battle_tables,
        );
        assert_db_uses_index(db, SQL_ENEMY_PROFILE, [9201_i64], battle_tables);
        assert_db_uses_index(db, SQL_SKILL_BY_ID, [95001_i64], battle_tables);
        assert_db_uses_index(db, SQL_SKILLS_EXACT_ZH, ["试作冰刺"], battle_tables);
        assert_db_uses_index(
            db,
            SQL_SKILLS_EXACT_LANG,
            ["en", "Synth Ice Spike"],
            battle_tables,
        );
        assert_db_uses_index(
            db,
            SQL_SKILLS_CONTAINS_FTS,
            ["%冰刺%", "zh-Hans", "冰刺"],
            battle_tables,
        );
        assert_db_uses_index(db, SQL_SKILLS_OF_UNIT, [9001_i64], battle_tables);
        assert_db_uses_index(db, SQL_BATTLE_GROUP_BY_ID, [9801_i64], battle_tables);
        assert_db_uses_index(db, SQL_BATTLE_GROUP_MEMBERS, [9801_i64], battle_tables);
        assert_db_uses_index(db, SQL_GROUPS_CONTAINING, [9101_i64], battle_tables);
        assert_db_uses_index(db, SQL_ENEMIES_IN_CHAPTER, [1_i64], battle_tables);
        assert_db_uses_index(db, SQL_CHAPTERS_OF_ENEMY, [9101_i64], battle_tables);
        assert_db_uses_index(db, SQL_ENEMY_CHAPTER_SOURCES, [9101_i64], battle_tables);
        assert_db_uses_index(
            db,
            SQL_UNITS_BY_RESISTANCE,
            ["Ice", "Weakness"],
            battle_tables,
        );
        assert_db_uses_index(db, SQL_UNITS_BY_CATEGORY, ["Enemy"], battle_tables);
        assert_db_uses_index(db, SQL_SKILLS_BY_ELEMENT, ["Ice"], battle_tables);
        assert_db_uses_index(db, SQL_COMMAND_SKILL_BY_ID, [1_i64], battle_tables);
        assert_db_uses_index(db, SQL_COMMAND_SKILLS_FOR_UNIT, [13017_i64], battle_tables);
        assert_db_uses_index(db, SQL_UNIT_AI_BY_ID, [9201_i64], battle_tables);
        assert_db_uses_index(db, SQL_ENCOUNTERS_FOR_GROUP, [9801_i64], battle_tables);
        assert_db_uses_index(
            db,
            SQL_ENCOUNTERS_IN_POOL,
            ["Encount/00Test/DT_00Test_Encount"],
            battle_tables,
        );
        assert_db_uses_index(db, SQL_BREAK_CALC, [], battle_tables);
        assert_db_uses_index(db, SQL_ENEMIES_EXACT_ZH, ["霜翼试作兽"], battle_tables);
        assert_db_uses_index(db, SQL_ALL_ENEMIES_CATALOG, [], battle_tables);
        assert_db_uses_index(db, SQL_ALL_ENEMY_CHAPTERS, [], battle_tables);
        assert_db_uses_index(db, SQL_ALL_SKILLS_CATALOG, [i64::MIN], battle_tables);
        assert_db_uses_index(db, SQL_HAS_BATTLE_DATA, [i64::MIN], battle_tables);
        assert_db_uses_index(db, SQL_HAS_COMMAND_SKILLS, [i64::MIN], battle_tables);
        assert_db_uses_index(db, SQL_MISSING_ENEMY_CHAPTER_ISSUE, [], &["build_issue"]);
    }
}
