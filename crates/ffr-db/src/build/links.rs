//! Cross-area links (`11_links`) import stage.
//!
//! Runs after all domain stages (`01_text` .. `10_snapshots`) and reads already-populated tables
//! (plus exported dialogue table JSONs when available so original JSON row order is preserved)
//! without re-parsing other areas:
//! 1. Talk event chain: `talk_event`, `talk_event_next`, `talk_line`, `level_object_talk`.
//! 2. Map units: `map_unit` (from `Asset/Map/DT_MapUnitAsset`) + validation of `level_object`
//!    map-unit references.
//! 3. Unified class references: `class_ref` collecting raw class strings from `actor`,
//!    `blueprint`, `snapshot_object`, and script `asset_ref` rows and linking them to `ue_class`
//!    and/or `blueprint`.
//!
//! Any sub-pass whose upstream tables are empty is skipped gracefully and recorded in
//! `stage_report.note`.

use std::collections::{BTreeMap, BTreeSet};

use ffr_data::assets::level::{LevelObject, LevelObjectData};
use ffr_data::playable::MAP_UNIT_ASSET_TABLE;
use ffr_data::world::{
    TALK_EVENT_DATA_TABLE, is_developer_asset_path, load_ordered_datatable_rows,
    normalize_ue_asset_path, parse_talk_event_row, parse_talk_lines_ordered,
    talk_table_relative_path,
};
use ffr_data::{datatable_relative_path_to_json, normalize_row_struct_name};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::build::{ImportCtx, StageStats};
use crate::error::{DbError, Result};
use crate::query::links::{MapUnit, MapUnitAnimationAsset};
use crate::record::encode_doc;

/// Populate `11_links` tables by connecting previously loaded domain tables.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let mut total_rows: u64 = 0;
    let mut summary_parts: Vec<String> = Vec::new();
    let mut skipped_parts: Vec<String> = Vec::new();
    let mut any_subpass_ran = false;

    // 1. Talk events, next-event edges, dialogue lines, and level_object -> talk links.
    match import_talk_chain(ctx, &mut skipped_parts)? {
        Some(stats) => {
            any_subpass_ran = true;
            total_rows = total_rows.saturating_add(stats.total_rows());
            summary_parts.push(format!(
                "talk_event={}, talk_event_next={}, talk_line={}, level_object_talk={}",
                stats.talk_events, stats.talk_next, stats.talk_lines, stats.level_object_talk
            ));
        }
        None => {
            skipped_parts.push(format!("对话链（dt_table 缺少 {TALK_EVENT_DATA_TABLE}）"));
        }
    }

    // 2. Map units (`DT_MapUnitAsset`) and `level_object` map-unit reference validation.
    match import_map_units(ctx, &mut skipped_parts)? {
        Some(map_unit_rows) => {
            any_subpass_ran = true;
            total_rows = total_rows.saturating_add(map_unit_rows);
            summary_parts.push(format!("map_unit={map_unit_rows}"));
        }
        None => {
            skipped_parts.push(format!("地图单位（dt_table 缺少 {MAP_UNIT_ASSET_TABLE}）"));
        }
    }

    // 3. Normalized cross-area class references (`class_ref`).
    match import_class_refs(ctx, &mut skipped_parts)? {
        Some((class_rows, resolved_rows)) => {
            any_subpass_ran = true;
            total_rows = total_rows.saturating_add(class_rows);
            summary_parts.push(format!("class_ref={class_rows}（命中={resolved_rows}）"));
        }
        None => {
            skipped_parts.push("类关联（来源表均为空）".to_owned());
        }
    }

    if !any_subpass_ran {
        return Ok(StageStats::skipped("前置区域表均为空，跳过跨区域连接"));
    }

    if !skipped_parts.is_empty() {
        summary_parts.push(format!("跳过：{}", skipped_parts.join("；")));
    }

    Ok(StageStats::ok_with_note(
        total_rows,
        summary_parts.join("；"),
    ))
}

struct TalkImportStats {
    talk_events: u64,
    talk_next: u64,
    talk_lines: u64,
    level_object_talk: u64,
}

impl TalkImportStats {
    fn total_rows(&self) -> u64 {
        self.talk_events + self.talk_next + self.talk_lines + self.level_object_talk
    }
}

fn import_talk_chain(
    ctx: &mut ImportCtx<'_>,
    skipped_parts: &mut Vec<String>,
) -> Result<Option<TalkImportStats>> {
    let tx = ctx.tx();
    let Some(talk_table_id) = lookup_dt_table_id(tx, TALK_EVENT_DATA_TABLE)? else {
        return Ok(None);
    };

    let raw_rows = load_dt_rows(tx, talk_table_id)?;
    let mut events = Vec::with_capacity(raw_rows.len());
    for (row_name, raw_doc) in &raw_rows {
        let val: Value =
            serde_json::from_str(raw_doc).map_err(|source| DbError::DocDeserialize {
                table: "dt_row".to_owned(),
                key: format!("{TALK_EVENT_DATA_TABLE}/{row_name}"),
                source,
            })?;
        events.push(parse_talk_event_row(row_name, &val, TALK_EVENT_DATA_TABLE)?);
    }
    events.sort_by(|a, b| a.id.cmp(&b.id));

    // Load upstream lookup indexes once (secondary indexes are not yet created during build).
    let (seq_by_path, seq_by_lower) = load_sequence_lookup(tx)?;
    let has_sequences = !seq_by_path.is_empty();
    if !has_sequences {
        skipped_parts.push("过场关联（sequence 表为空）".to_owned());
    }

    let battle_group_ids = load_i64_id_set(tx, "battle_group", "id")?;
    let has_battle_groups = !battle_group_ids.is_empty();
    if !has_battle_groups {
        skipped_parts.push("对话战斗组校验（battle_group 表为空）".to_owned());
    }

    let (dt_by_rel, dt_by_lower) = load_dt_table_lookup(tx)?;
    let string_table_ns = load_string_table_ns_map(tx)?;
    if string_table_ns.is_empty() {
        skipped_parts.push("对话字符串表命名空间（string_table 表为空，改用路径推断）".to_owned());
    }

    let known_talk_ids: BTreeSet<String> = events.iter().map(|e| e.id.clone()).collect();
    let mut referenced_talk_tables: BTreeSet<(String, String, i64)> = BTreeSet::new();
    let mut talk_event_rows: u64 = 0;
    let mut talk_next_rows: u64 = 0;

    {
        let mut insert_event = tx
            .prepare_cached(
                "INSERT INTO talk_event \
                 (id, event_type, sequence_path, sequence_id, talk_table_path, is_skip_confirm, encount_group_id, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|err| DbError::sqlite("准备写入 talk_event", err))?;

        let mut insert_next = tx
            .prepare_cached(
                "INSERT INTO talk_event_next \
                 (event, next_event, kind, choice_index, condition, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 talk_event_next", err))?;

        for event in &events {
            let sequence_id = match event.event_sequence_path.as_deref() {
                Some(seq_path) if has_sequences => {
                    let resolved = seq_by_path
                        .get(seq_path)
                        .or_else(|| seq_by_lower.get(&seq_path.to_ascii_lowercase()))
                        .copied();
                    if resolved.is_none() {
                        let detail = if is_developer_asset_path(seq_path) {
                            format!("对话事件引用了开发者测试过场（未导出）：{seq_path}")
                        } else {
                            format!("对话事件引用的过场不在 sequence 表中：{seq_path}")
                        };
                        ctx.record_issue("dangling_talk_sequence", &event.id, &detail)?;
                    }
                    resolved
                }
                _ => None,
            };

            if let Some(table_path) = event.talk_table_path.as_deref() {
                let matched_dt = talk_table_relative_path(table_path).and_then(|rel| {
                    dt_by_rel
                        .get(&rel)
                        .map(|&tid| (rel.clone(), tid))
                        .or_else(|| dt_by_lower.get(&rel.to_ascii_lowercase()).cloned())
                });
                match matched_dt {
                    Some((rel, table_id)) => {
                        referenced_talk_tables.insert((table_path.to_owned(), rel, table_id));
                    }
                    None => {
                        let detail = if is_developer_asset_path(table_path) {
                            format!("对话事件引用了开发者测试对话表（未导出）：{table_path}")
                        } else {
                            format!("对话事件引用的对话表不在 dt_table 中：{table_path}")
                        };
                        ctx.record_issue("dangling_talk_table", &event.id, &detail)?;
                    }
                }
            }

            if has_battle_groups
                && let Some(group_id) = event.encount_group_id
                && !battle_group_ids.contains(&group_id)
            {
                ctx.record_issue(
                    "dangling_talk_battle_group",
                    &event.id,
                    &format!("对话事件引用的战斗组编号 {group_id} 不存在于 battle_group 表"),
                )?;
            }

            let doc = encode_doc("talk_event", &event.id, event)?;
            insert_event
                .execute(params![
                    event.id,
                    event.event_type,
                    event.event_sequence_path,
                    sequence_id,
                    event.talk_table_path,
                    i64::from(event.is_skip_confirm),
                    event.encount_group_id,
                    doc,
                ])
                .map_err(|err| DbError::sqlite(format!("写入 talk_event {}", event.id), err))?;
            talk_event_rows += 1;

            // Insert in natural PRIMARY KEY `(event, kind, ord)` order ("choice" < "continuous").
            let mut choice_ord: usize = 0;
            for branch in &event.choice_branches {
                for tr in &branch.events {
                    if !known_talk_ids.contains(&tr.event_id) {
                        ctx.record_issue(
                            "dangling_talk_next",
                            &event.id,
                            &format!(
                                "分支 {} 事件目标 {} 不存在于 talk_event 表",
                                branch.index, tr.event_id
                            ),
                        )?;
                    }
                    insert_next
                        .execute(params![
                            event.id,
                            tr.event_id,
                            "choice",
                            branch.index,
                            tr.condition,
                            i64::try_from(choice_ord).unwrap_or(i64::MAX),
                        ])
                        .map_err(|err| {
                            DbError::sqlite(
                                format!(
                                    "写入 talk_event_next ({}, choice, {choice_ord})",
                                    event.id
                                ),
                                err,
                            )
                        })?;
                    choice_ord += 1;
                    talk_next_rows += 1;
                }
            }

            for (ord, tr) in event.continuous_events.iter().enumerate() {
                if !known_talk_ids.contains(&tr.event_id) {
                    ctx.record_issue(
                        "dangling_talk_next",
                        &event.id,
                        &format!("连续事件目标 {} 不存在于 talk_event 表", tr.event_id),
                    )?;
                }
                insert_next
                    .execute(params![
                        event.id,
                        tr.event_id,
                        "continuous",
                        Option::<i64>::None,
                        tr.condition,
                        i64::try_from(ord).unwrap_or(i64::MAX),
                    ])
                    .map_err(|err| {
                        DbError::sqlite(
                            format!("写入 talk_event_next ({}, continuous, {ord})", event.id),
                            err,
                        )
                    })?;
                talk_next_rows += 1;
            }
        }
    }

    // Import dialogue lines for every referenced TalkEventRowData table, preserving original JSON
    // row order in `ord` while inserting rows in natural PRIMARY KEY `(table_path, row)` order.
    let mut talk_line_rows: u64 = 0;
    {
        let mut insert_line = tx
            .prepare_cached(
                "INSERT INTO talk_line \
                 (table_path, row, ord, talk_id, speaker, emotion, text_ns, text_key, voice, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(|err| DbError::sqlite("准备写入 talk_line", err))?;

        for (ue_path, rel_path, table_id) in &referenced_talk_tables {
            let ordered_rows = load_talk_table_rows_ordered(ctx, rel_path, *table_id)?;
            let mut lines =
                parse_talk_lines_ordered(ue_path, &ordered_rows, rel_path, |table_id_str| {
                    string_table_ns.get(table_id_str).cloned()
                })?;
            lines.sort_by(|a, b| a.row_key.cmp(&b.row_key));
            for line in lines {
                let doc = encode_doc(
                    "talk_line",
                    format!("{}:{}", line.table_path, line.row_key),
                    &line,
                )?;
                insert_line
                    .execute(params![
                        line.table_path,
                        line.row_key,
                        i64::try_from(line.ord).unwrap_or(i64::MAX),
                        line.talk_id,
                        line.speaker,
                        line.emotion,
                        line.text_ns,
                        line.text_key,
                        line.voice,
                        doc,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(
                            format!("写入 talk_line ({}:{})", line.table_path, line.row_key),
                            err,
                        )
                    })?;
                talk_line_rows += 1;
            }
        }
    }

    // Link level objects -> talk events (`level_object_talk`).
    let level_object_talk_rows = if table_has_rows(tx, "level_object")? {
        import_level_object_talk_links(ctx, &known_talk_ids)?
    } else {
        skipped_parts.push("关卡对象对话关联（level_object 表为空）".to_owned());
        0
    };

    Ok(Some(TalkImportStats {
        talk_events: talk_event_rows,
        talk_next: talk_next_rows,
        talk_lines: talk_line_rows,
        level_object_talk: level_object_talk_rows,
    }))
}

fn load_talk_table_rows_ordered(
    ctx: &ImportCtx<'_>,
    rel_path: &str,
    table_id: i64,
) -> Result<Vec<(String, Value)>> {
    if let Some(store) = ctx.data_store()
        && let Some(json_path) = datatable_relative_path_to_json(rel_path, store.export_root())
        && json_path.is_file()
    {
        let ordered = load_ordered_datatable_rows(&json_path)?;
        return Ok(ordered.rows);
    }

    let raw_rows = load_dt_rows_by_rowid(ctx.tx(), table_id)?;
    let mut out = Vec::with_capacity(raw_rows.len());
    for (row_name, raw_doc) in raw_rows {
        let val: Value =
            serde_json::from_str(&raw_doc).map_err(|source| DbError::DocDeserialize {
                table: "dt_row".to_owned(),
                key: format!("{rel_path}/{row_name}"),
                source,
            })?;
        out.push((row_name, val));
    }
    Ok(out)
}

fn import_level_object_talk_links(
    ctx: &ImportCtx<'_>,
    known_talk_ids: &BTreeSet<String>,
) -> Result<u64> {
    let tx = ctx.tx();
    let mut objects: Vec<(i64, String)> = Vec::new();
    {
        let mut stmt = tx
            .prepare_cached(
                "SELECT id, doc \
                 FROM level_object \
                 WHERE kind IN ('event_trigger', 'map_unit', 'gimmick', 'transition', 'treasure_box') \
                 ORDER BY id",
            )
            .map_err(|err| DbError::sqlite("准备读取 level_object 对话引用", err))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| DbError::sqlite("读取 level_object 对话引用", err))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| DbError::sqlite("迭代 level_object 对话引用", err))?
        {
            objects.push((
                row.get(0)
                    .map_err(|err| DbError::sqlite("读取 level_object.id", err))?,
                row.get(1)
                    .map_err(|err| DbError::sqlite("读取 level_object.doc", err))?,
            ));
        }
    }

    let mut insert_stmt = tx
        .prepare_cached(
            "INSERT INTO level_object_talk (level_object_id, talk_event_id, condition, ord) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|err| DbError::sqlite("准备写入 level_object_talk", err))?;

    let mut inserted_rows: u64 = 0;
    for (lo_id, raw_doc) in objects {
        let obj: LevelObject =
            serde_json::from_str(&raw_doc).map_err(|source| DbError::DocDeserialize {
                table: "level_object".to_owned(),
                key: lo_id.to_string(),
                source,
            })?;
        let extracted = extract_level_object_event_ids(&obj);
        // Deduplicate in extraction order to keep semantic `ord`, then insert in natural PK
        // `(level_object_id, talk_event_id)` order.
        let mut deduped: BTreeMap<String, (Option<String>, i64)> = BTreeMap::new();
        let mut ord: i64 = 0;
        for (event_id, condition) in extracted {
            if deduped.contains_key(&event_id) {
                continue;
            }
            if !known_talk_ids.contains(&event_id) {
                ctx.record_issue(
                    "dangling_talk_event",
                    &format!("{}:{}", obj.level_name, obj.actor_name),
                    &format!(
                        "关卡对象（level_object={}，kind={}）引用的事件编号 {} 不存在于 talk_event 表",
                        lo_id,
                        obj.kind.as_str(),
                        event_id
                    ),
                )?;
            }
            deduped.insert(event_id, (condition, ord));
            ord += 1;
        }
        for (event_id, (condition, ord)) in deduped {
            insert_stmt
                .execute(params![lo_id, event_id, condition, ord])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 level_object_talk ({lo_id}, {event_id})"), err)
                })?;
            inserted_rows += 1;
        }
    }

    Ok(inserted_rows)
}

fn extract_level_object_event_ids(obj: &LevelObject) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    match &obj.data {
        LevelObjectData::EventTrigger(et) => {
            if let Some(eid) = et.interact_event_id.as_deref().map(str::trim)
                && !eid.is_empty()
                && !eid.eq_ignore_ascii_case("None")
            {
                out.push((eid.to_owned(), None));
            }
            for ev in &et.events {
                let eid = ev.event_id.trim();
                if !eid.is_empty() && !eid.eq_ignore_ascii_case("None") {
                    out.push((eid.to_owned(), ev.condition.clone()));
                }
            }
        }
        LevelObjectData::MapUnit(mu) => {
            for ev in &mu.events {
                let eid = ev.event_id.trim();
                if !eid.is_empty() && !eid.eq_ignore_ascii_case("None") {
                    out.push((eid.to_owned(), ev.condition.clone()));
                }
            }
        }
        LevelObjectData::Gimmick(g) => {
            for ev in &g.events {
                let eid = ev.event_id.trim();
                if !eid.is_empty() && !eid.eq_ignore_ascii_case("None") {
                    out.push((eid.to_owned(), ev.condition.clone()));
                }
            }
        }
        LevelObjectData::Transition(tr) => {
            for tgt in &tr.targets {
                for ev in tgt.events.iter().chain(&tgt.after_transition_events) {
                    let eid = ev.event_id.trim();
                    if !eid.is_empty() && !eid.eq_ignore_ascii_case("None") {
                        out.push((eid.to_owned(), ev.condition.clone()));
                    }
                }
            }
        }
        LevelObjectData::TreasureBox(tb) => {
            if let Some(eid) = tb.battle_finish_event_id.as_deref().map(str::trim)
                && !eid.is_empty()
                && !eid.eq_ignore_ascii_case("None")
            {
                out.push((eid.to_owned(), None));
            }
        }
        _ => {}
    }
    out
}

fn import_map_units(
    ctx: &mut ImportCtx<'_>,
    skipped_parts: &mut Vec<String>,
) -> Result<Option<u64>> {
    let tx = ctx.tx();
    let Some(table_id) = lookup_dt_table_id(tx, MAP_UNIT_ASSET_TABLE)? else {
        return Ok(None);
    };

    let playable_names = load_playable_unit_names(tx)?;
    let raw_rows = load_dt_rows(tx, table_id)?;

    let mut units_by_id: BTreeMap<i64, MapUnit> = BTreeMap::new();
    for (row_key, raw_doc) in &raw_rows {
        let val: Value =
            serde_json::from_str(raw_doc).map_err(|source| DbError::DocDeserialize {
                table: "dt_row".to_owned(),
                key: format!("{MAP_UNIT_ASSET_TABLE}/{row_key}"),
                source,
            })?;
        let Some(obj) = val.as_object() else {
            continue;
        };
        let Some(id) = obj.get("ID").and_then(Value::as_i64) else {
            ctx.record_issue(
                "invalid_map_unit_row",
                row_key,
                "DT_MapUnitAsset 行缺少整数 ID，已跳过",
            )?;
            continue;
        };

        let animations = parse_map_unit_animations(obj.get("animationAssetList"));
        let primary_asset = animations.iter().find_map(|a| a.ss6_project.clone());
        let footstep = obj
            .get("Footstep")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let shadow_scale = obj.get("shadowScale").and_then(Value::as_f64);
        let specular_scale = obj.get("specularScale").and_then(Value::as_f64);

        let (name_ns, name_key, name_zh) = playable_names
            .get(&id)
            .cloned()
            .unwrap_or((None, None, None));

        units_by_id.insert(
            id,
            MapUnit {
                id,
                row_key: row_key.clone(),
                name: row_key.clone(),
                name_ns,
                name_key,
                name_zh,
                primary_asset,
                animations,
                footstep,
                shadow_scale,
                specular_scale,
            },
        );
    }

    let mut known_map_unit_ids: BTreeSet<i64> = BTreeSet::new();
    let mut inserted_rows: u64 = 0;

    {
        let mut insert_stmt = tx
            .prepare_cached(
                "INSERT INTO map_unit \
                 (id, row_key, name, name_ns, name_key, name_zh, primary_asset, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|err| DbError::sqlite("准备写入 map_unit", err))?;

        for unit in units_by_id.values() {
            let doc = encode_doc("map_unit", unit.id, unit)?;
            insert_stmt
                .execute(params![
                    unit.id,
                    unit.row_key,
                    unit.name,
                    unit.name_ns,
                    unit.name_key,
                    unit.name_zh,
                    unit.primary_asset,
                    doc,
                ])
                .map_err(|err| DbError::sqlite(format!("写入 map_unit {}", unit.id), err))?;
            known_map_unit_ids.insert(unit.id);
            inserted_rows += 1;
        }
    }

    // Re-validate `level_object` mapUnitId references against `map_unit` (`DT_MapUnitAsset`).
    if table_has_rows(tx, "level_object")? {
        let mut to_check: Vec<(i64, String)> = Vec::new();
        {
            let mut stmt = tx
                .prepare_cached(
                    "SELECT id, doc \
                     FROM level_object \
                     WHERE kind IN ('map_unit', 'shop', 'inn') \
                     ORDER BY id",
                )
                .map_err(|err| DbError::sqlite("准备校验 level_object 地图单位引用", err))?;
            let mut rows = stmt
                .query([])
                .map_err(|err| DbError::sqlite("读取 level_object 地图单位引用", err))?;
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("迭代 level_object 地图单位引用", err))?
            {
                to_check.push((
                    row.get(0)
                        .map_err(|err| DbError::sqlite("读取 level_object.id", err))?,
                    row.get(1)
                        .map_err(|err| DbError::sqlite("读取 level_object.doc", err))?,
                ));
            }
        }
        let mut seen_issues: BTreeSet<(String, String, i64)> = BTreeSet::new();
        for (lo_id, raw_doc) in to_check {
            let obj: LevelObject =
                serde_json::from_str(&raw_doc).map_err(|source| DbError::DocDeserialize {
                    table: "level_object".to_owned(),
                    key: lo_id.to_string(),
                    source,
                })?;
            let map_unit_id = match &obj.data {
                LevelObjectData::MapUnit(u) => u.map_unit_id,
                LevelObjectData::Shop(s) => s.map_unit_id,
                LevelObjectData::Inn(i) => i.map_unit_id,
                _ => None,
            };
            if let Some(mu_id) = map_unit_id
                && !known_map_unit_ids.contains(&mu_id)
                && seen_issues.insert((obj.level_name.clone(), obj.actor_name.clone(), mu_id))
            {
                ctx.record_issue(
                    "dangling_map_unit",
                    &format!("{}:{}", obj.level_name, obj.actor_name),
                    &format!(
                        "关卡对象（kind={}）引用的地图单位编号 {mu_id} 不存在于 map_unit（DT_MapUnitAsset）",
                        obj.kind.as_str()
                    ),
                )?;
            }
        }
    } else {
        skipped_parts.push("地图单位引用校验（level_object 表为空）".to_owned());
    }

    Ok(Some(inserted_rows))
}

fn parse_map_unit_animations(raw: Option<&Value>) -> Vec<MapUnitAnimationAsset> {
    let Some(items) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let extract_path = |field: &str| {
            obj.get(field)
                .and_then(Value::as_object)
                .and_then(|sub| sub.get("AssetPathName"))
                .and_then(Value::as_str)
                .and_then(normalize_ue_asset_path)
        };
        out.push(MapUnitAnimationAsset {
            is_vision_character: obj
                .get("isVisionCharacter")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ss6_project: extract_path("Ss6Project"),
            material: extract_path("Material"),
            texture_base_color: extract_path("textureBaseColor"),
            texture_normal: extract_path("textureNormal"),
            texture_metallic_roughness: extract_path("textureMetallicRoughness"),
        });
    }
    out
}

fn import_class_refs(
    ctx: &mut ImportCtx<'_>,
    skipped_parts: &mut Vec<String>,
) -> Result<Option<(u64, u64)>> {
    let tx = ctx.tx();
    let ue_index = UeClassLookup::load(tx)?;
    let bp_index = BlueprintLookup::load(tx)?;

    if ue_index.is_empty() {
        skipped_parts.push("反射类连接（ue_class 表为空）".to_owned());
    }
    if bp_index.is_empty() {
        skipped_parts.push("蓝图类连接（blueprint 表为空）".to_owned());
    }

    let mut raw_names: BTreeSet<String> = BTreeSet::new();
    collect_distinct_strings(
        tx,
        "SELECT DISTINCT class FROM actor WHERE class IS NOT NULL AND class != ''",
        &mut raw_names,
    )?;
    collect_distinct_strings(
        tx,
        "SELECT DISTINCT parent_class FROM blueprint WHERE parent_class IS NOT NULL AND parent_class != ''",
        &mut raw_names,
    )?;
    collect_distinct_strings(
        tx,
        "SELECT DISTINCT class_name FROM blueprint WHERE class_name IS NOT NULL AND class_name != ''",
        &mut raw_names,
    )?;
    collect_distinct_strings(
        tx,
        "SELECT DISTINCT class FROM snapshot_object WHERE class IS NOT NULL AND class != ''",
        &mut raw_names,
    )?;
    // Only class-level script refs (containing `.`), skipping bare `/Script/<Module>` packages.
    collect_distinct_strings(
        tx,
        "SELECT DISTINCT to_package FROM asset_ref WHERE kind = 'script' AND to_package LIKE '%.%'",
        &mut raw_names,
    )?;

    if raw_names.is_empty() {
        return Ok(None);
    }

    let check_resolution = !ue_index.is_empty() || !bp_index.is_empty();
    let mut inserted_rows: u64 = 0;
    let mut resolved_rows: u64 = 0;

    {
        let mut insert_stmt = tx
            .prepare_cached(
                "INSERT INTO class_ref (raw_name, normalized, ue_class_id, blueprint_id) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 class_ref", err))?;

        for raw_name in &raw_names {
            let cleaned = clean_raw_class_name(raw_name);
            let (normalized, ue_class_id, blueprint_id) =
                resolve_class_ref(&cleaned, &ue_index, &bp_index);

            let is_resolved = ue_class_id.is_some() || blueprint_id.is_some();
            if is_resolved {
                resolved_rows += 1;
            } else if check_resolution {
                let detail = if is_likely_level_blueprint(&normalized) {
                    format!(
                        "类名未在 ue_class 或 blueprint 中找到（属 .umap 内部关卡蓝图，非独立蓝图资产）：{normalized}"
                    )
                } else {
                    format!("类名未在 ue_class 或 blueprint 中找到：{normalized}")
                };
                ctx.record_issue("unresolved_class_ref", raw_name, &detail)?;
            }

            insert_stmt
                .execute(params![raw_name, normalized, ue_class_id, blueprint_id])
                .map_err(|err| DbError::sqlite(format!("写入 class_ref {raw_name}"), err))?;
            inserted_rows += 1;
        }
    }

    Ok(Some((inserted_rows, resolved_rows)))
}

/// Strip UE runtime object prefixes (`"Class "`, `"BlueprintGeneratedClass "`,
/// `"WidgetBlueprintGeneratedClass "`, `"ScriptStruct "`, `"Enum "`, etc.) and CUE4Parse
/// `Type'Inner'` wrappers from a raw class name string.
pub(crate) fn clean_raw_class_name(raw: &str) -> String {
    let mut s = raw.trim();
    if s.is_empty() {
        return String::new();
    }

    // Strip runtime prefix like `Class /Script/...` or `BlueprintGeneratedClass /Game/...`.
    if let Some((prefix, rest)) = s.split_once(' ')
        && !prefix.is_empty()
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        s = rest.trim();
    }

    // Strip CUE4Parse wrapper like `Class'CPP_MapUnit_Player'` or
    // `BlueprintGeneratedClass'/Game/BP/Foo.Foo_C'`.
    let unwrapped = normalize_row_struct_name(s);
    unwrapped.trim().replace('\\', "/")
}

struct UeClassLookup {
    by_path: BTreeMap<String, String>,
    by_path_lower: BTreeMap<String, String>,
    by_short: BTreeMap<String, Vec<String>>,
}

impl UeClassLookup {
    fn load(conn: &Connection) -> Result<Self> {
        let mut by_path = BTreeMap::new();
        let mut by_path_lower = BTreeMap::new();
        let mut by_short: BTreeMap<String, Vec<String>> = BTreeMap::new();

        let mut stmt = conn
            .prepare_cached("SELECT path, short_name FROM ue_class ORDER BY path")
            .map_err(|err| DbError::sqlite("准备读取 ue_class 索引", err))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| DbError::sqlite("读取 ue_class 索引", err))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| DbError::sqlite("迭代 ue_class 索引", err))?
        {
            let path: String = row
                .get(0)
                .map_err(|err| DbError::sqlite("读取 ue_class.path", err))?;
            let short_name: String = row
                .get(1)
                .map_err(|err| DbError::sqlite("读取 ue_class.short_name", err))?;
            by_path_lower
                .entry(path.to_ascii_lowercase())
                .or_insert_with(|| path.clone());
            by_short.entry(short_name).or_default().push(path.clone());
            by_path.insert(path.clone(), path);
        }

        Ok(Self {
            by_path,
            by_path_lower,
            by_short,
        })
    }

    fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    fn resolve(&self, cleaned: &str) -> Option<String> {
        if let Some(p) = self.by_path.get(cleaned) {
            return Some(p.clone());
        }
        let lower = cleaned.to_ascii_lowercase();
        if let Some(p) = self.by_path_lower.get(&lower) {
            return Some(p.clone());
        }
        if cleaned.starts_with('/')
            && !cleaned.contains('.')
            && let Some(stem) = cleaned.rsplit('/').next()
            && !stem.is_empty()
        {
            let with_c = format!("{cleaned}.{stem}_C");
            if let Some(p) = self
                .by_path
                .get(&with_c)
                .or_else(|| self.by_path_lower.get(&with_c.to_ascii_lowercase()))
            {
                return Some(p.clone());
            }
        }
        // Only match by short name when `cleaned` is NOT a concrete `/Game/...` path whose asset
        // wasn't loaded into reflection (avoid false matches against a different package).
        if !cleaned.starts_with('/') {
            if let Some(p) = self.pick_by_short(cleaned) {
                return Some(p);
            }
            if !cleaned.ends_with("_C")
                && let Some(p) = self.pick_by_short(&format!("{cleaned}_C"))
            {
                return Some(p);
            }
            if let Some(stripped) = cleaned.strip_suffix("_C")
                && let Some(p) = self.pick_by_short(stripped)
            {
                return Some(p);
            }
        }
        None
    }

    fn pick_by_short(&self, short: &str) -> Option<String> {
        let list = self.by_short.get(short)?;
        if list.len() == 1 {
            return Some(list[0].clone());
        }
        list.iter()
            .min_by_key(|p| {
                let rank = if p.starts_with("/Script/mucha.") {
                    0
                } else if p.starts_with("/Script/Engine.") {
                    1
                } else if p.starts_with("/Script/") {
                    2
                } else {
                    3
                };
                (rank, p.as_str())
            })
            .cloned()
    }
}

struct BlueprintLookup {
    by_path: BTreeMap<String, (String, String)>,
    by_path_lower: BTreeMap<String, (String, String)>,
    by_class: BTreeMap<String, Vec<(String, String)>>,
    by_class_lower: BTreeMap<String, Vec<(String, String)>>,
}

impl BlueprintLookup {
    fn load(conn: &Connection) -> Result<Self> {
        let mut by_path = BTreeMap::new();
        let mut by_path_lower = BTreeMap::new();
        let mut by_class: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        let mut by_class_lower: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

        let mut stmt = conn
            .prepare_cached("SELECT path, class_name FROM blueprint ORDER BY path")
            .map_err(|err| DbError::sqlite("准备读取 blueprint 索引", err))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| DbError::sqlite("读取 blueprint 索引", err))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| DbError::sqlite("迭代 blueprint 索引", err))?
        {
            let path: String = row
                .get(0)
                .map_err(|err| DbError::sqlite("读取 blueprint.path", err))?;
            let class_name: String = row
                .get(1)
                .map_err(|err| DbError::sqlite("读取 blueprint.class_name", err))?;
            let pair = (path.clone(), class_name.clone());
            by_path_lower
                .entry(path.to_ascii_lowercase())
                .or_insert_with(|| pair.clone());
            by_class
                .entry(class_name.clone())
                .or_default()
                .push(pair.clone());
            by_class_lower
                .entry(class_name.to_ascii_lowercase())
                .or_default()
                .push(pair.clone());
            by_path.insert(path, pair);
        }

        Ok(Self {
            by_path,
            by_path_lower,
            by_class,
            by_class_lower,
        })
    }

    fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    fn resolve(&self, cleaned: &str) -> Option<(String, String)> {
        if cleaned.starts_with('/') {
            if cleaned.starts_with("/Script/") {
                return None;
            }
            let pkg = cleaned.split('.').next().unwrap_or(cleaned);
            if let Some(hit) = self.by_path.get(pkg) {
                return Some(hit.clone());
            }
            if let Some(hit) = self.by_path_lower.get(&pkg.to_ascii_lowercase()) {
                return Some(hit.clone());
            }
            return None;
        }

        if let Some(list) = self.by_class.get(cleaned) {
            return list.first().cloned();
        }
        if !cleaned.ends_with("_C") {
            let with_c = format!("{cleaned}_C");
            if let Some(list) = self.by_class.get(&with_c) {
                return list.first().cloned();
            }
            if let Some(list) = self.by_class_lower.get(&with_c.to_ascii_lowercase()) {
                return list.first().cloned();
            }
        }
        if let Some(list) = self.by_class_lower.get(&cleaned.to_ascii_lowercase()) {
            return list.first().cloned();
        }
        None
    }
}

fn resolve_class_ref(
    cleaned: &str,
    ue: &UeClassLookup,
    bp: &BlueprintLookup,
) -> (String, Option<String>, Option<String>) {
    let mut ue_id = ue.resolve(cleaned);
    let mut bp_pair = bp.resolve(cleaned);

    // Cross-link when a Blueprint appears in both `ue_class` and `blueprint`.
    if ue_id.is_none()
        && let Some((ref bp_path, ref bp_cls)) = bp_pair
    {
        let candidate = format!("{bp_path}.{bp_cls}");
        ue_id = ue.resolve(&candidate);
    }
    if bp_pair.is_none()
        && let Some(ref uc_path) = ue_id
        && uc_path.starts_with('/')
        && !uc_path.starts_with("/Script/")
    {
        bp_pair = bp.resolve(uc_path);
    }

    let normalized = if let Some(ref uc) = ue_id {
        uc.clone()
    } else if let Some((ref bp_path, ref bp_cls)) = bp_pair {
        format!("{bp_path}.{bp_cls}")
    } else {
        cleaned.to_owned()
    };

    (normalized, ue_id, bp_pair.map(|(p, _)| p))
}

fn is_likely_level_blueprint(normalized: &str) -> bool {
    if normalized.starts_with("/Game/Map/") || normalized.starts_with("/Game/Debug/") {
        return true;
    }
    let short = normalized.rsplit('.').next().unwrap_or(normalized);
    short.ends_with("_PL_C")
        || short.ends_with("_GD_C")
        || short.ends_with("_BG_C")
        || short.ends_with("_KD_C")
        || short.ends_with("_LT_C")
        || short.ends_with("_EV_C")
}

// ---- SQLite bulk-read helpers ----

fn lookup_dt_table_id(conn: &Connection, rel_path: &str) -> Result<Option<i64>> {
    let mut stmt = conn
        .prepare_cached("SELECT id FROM dt_table WHERE path = ?1")
        .map_err(|err| DbError::sqlite("准备查询 dt_table.id", err))?;
    stmt.query_row([rel_path], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("查询 dt_table ({rel_path})"), err))
}

fn load_dt_rows(conn: &Connection, table_id: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare_cached("SELECT row_name, doc FROM dt_row WHERE table_id = ?1 ORDER BY row_name")
        .map_err(|err| DbError::sqlite("准备读取 dt_row", err))?;
    let mut rows = stmt
        .query([table_id])
        .map_err(|err| DbError::sqlite("读取 dt_row", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 dt_row", err))?
    {
        out.push((
            row.get(0)
                .map_err(|err| DbError::sqlite("读取 dt_row.row_name", err))?,
            row.get(1)
                .map_err(|err| DbError::sqlite("读取 dt_row.doc", err))?,
        ));
    }
    Ok(out)
}

fn load_dt_rows_by_rowid(conn: &Connection, table_id: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare_cached("SELECT row_name, doc FROM dt_row WHERE table_id = ?1 ORDER BY rowid")
        .map_err(|err| DbError::sqlite("准备按行序读取 dt_row", err))?;
    let mut rows = stmt
        .query([table_id])
        .map_err(|err| DbError::sqlite("按行序读取 dt_row", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 dt_row", err))?
    {
        out.push((
            row.get(0)
                .map_err(|err| DbError::sqlite("读取 dt_row.row_name", err))?,
            row.get(1)
                .map_err(|err| DbError::sqlite("读取 dt_row.doc", err))?,
        ));
    }
    Ok(out)
}

type DtTableLookup = (BTreeMap<String, i64>, BTreeMap<String, (String, i64)>);

fn load_dt_table_lookup(conn: &Connection) -> Result<DtTableLookup> {
    let mut by_rel = BTreeMap::new();
    let mut by_lower = BTreeMap::new();
    let mut stmt = conn
        .prepare_cached("SELECT id, path FROM dt_table ORDER BY path")
        .map_err(|err| DbError::sqlite("准备读取 dt_table 目录", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("读取 dt_table 目录", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 dt_table 目录", err))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 dt_table.id", err))?;
        let path: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 dt_table.path", err))?;
        by_lower
            .entry(path.to_ascii_lowercase())
            .or_insert_with(|| (path.clone(), id));
        by_rel.insert(path, id);
    }
    Ok((by_rel, by_lower))
}

fn load_sequence_lookup(
    conn: &Connection,
) -> Result<(BTreeMap<String, i64>, BTreeMap<String, i64>)> {
    let mut by_path = BTreeMap::new();
    let mut by_lower = BTreeMap::new();
    let mut stmt = conn
        .prepare_cached("SELECT id, path FROM sequence ORDER BY path")
        .map_err(|err| DbError::sqlite("准备读取 sequence 路径表", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("读取 sequence 路径表", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 sequence 路径表", err))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 sequence.id", err))?;
        let path: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 sequence.path", err))?;
        by_lower.entry(path.to_ascii_lowercase()).or_insert(id);
        by_path.insert(path, id);
    }
    Ok((by_path, by_lower))
}

fn load_string_table_ns_map(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    let mut stmt = conn
        .prepare_cached("SELECT table_id, ns FROM string_table ORDER BY table_id")
        .map_err(|err| DbError::sqlite("准备读取 string_table", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("读取 string_table", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 string_table", err))?
    {
        let tid: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 string_table.table_id", err))?;
        let ns: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 string_table.ns", err))?;
        map.insert(tid, ns);
    }
    Ok(map)
}

type PlayableNameTuple = (Option<String>, Option<String>, Option<String>);

fn load_playable_unit_names(conn: &Connection) -> Result<BTreeMap<i64, PlayableNameTuple>> {
    let mut map = BTreeMap::new();
    let mut stmt = conn
        .prepare_cached(
            "SELECT map_unit_id, name_ns, name_key, name_zh FROM playable_unit ORDER BY map_unit_id",
        )
        .map_err(|err| DbError::sqlite("准备读取 playable_unit 名称", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("读取 playable_unit 名称", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("迭代 playable_unit 名称", err))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 playable_unit.map_unit_id", err))?;
        let ns: Option<String> = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 playable_unit.name_ns", err))?;
        let key: Option<String> = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 playable_unit.name_key", err))?;
        let zh: Option<String> = row
            .get(3)
            .map_err(|err| DbError::sqlite("读取 playable_unit.name_zh", err))?;
        map.insert(id, (ns, key, zh));
    }
    Ok(map)
}

fn load_i64_id_set(conn: &Connection, table: &str, col: &str) -> Result<BTreeSet<i64>> {
    let sql = format!("SELECT \"{col}\" FROM \"{table}\"");
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|err| DbError::sqlite(format!("准备读取表 {table} 编号集"), err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite(format!("读取表 {table} 编号集"), err))?;
    let mut set = BTreeSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("迭代表 {table} 编号集"), err))?
    {
        set.insert(
            row.get(0)
                .map_err(|err| DbError::sqlite(format!("读取 {table}.{col}"), err))?,
        );
    }
    Ok(set)
}

fn table_has_rows(conn: &Connection, table: &str) -> Result<bool> {
    let sql = format!("SELECT EXISTS(SELECT 1 FROM \"{table}\" LIMIT 1)");
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(|err| DbError::sqlite(format!("检查表 {table} 是否非空"), err))
}

fn collect_distinct_strings(
    conn: &Connection,
    sql: &str,
    out: &mut BTreeSet<String>,
) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite("准备收集类名", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| DbError::sqlite("执行类名收集查询", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取类名收集行", err))?
    {
        let raw: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取类名字符串", err))?;
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            out.insert(trimmed.to_owned());
        }
    }
    Ok(())
}
