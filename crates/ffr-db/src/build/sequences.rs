//! Cutscene sequences area (`09_sequences`) import stage.
//!
//! Scans `<full>/FFRS/Content/Sequencer/**/*.json`, parses every `LevelSequence` asset via
//! [`ffr_data::assets::sequence`], and bulk-loads `sequence`, `sequence_binding`,
//! `sequence_track`, and `sequence_event`. Non-`LevelSequence` JSON exports (such as Blueprint or
//! Widget helpers living under `Sequencer/`) are skipped cleanly, while parse errors and
//! structural anomalies are recorded in `build_issue` without aborting the build.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ffr_data::assets::sequence::{normalize_sequence_path, parse_level_sequence_document};
use rusqlite::params;
use serde_json::Value;

use crate::build::{ImportCtx, StageStats, relative_label_under};
use crate::error::{DbError, Result};
use crate::record::encode_doc;
use crate::util::path_to_slash_string;

/// Import all exported `LevelSequence` assets under `<full>/FFRS/Content/Sequencer/`.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let Some(full_dir) = ctx.inputs().full.as_deref() else {
        return Ok(StageStats::skipped("未指定全量导出目录（--full）"));
    };
    let sequencer_dir = full_dir.join("FFRS").join("Content").join("Sequencer");
    if !sequencer_dir.is_dir() {
        return Ok(StageStats::skipped(
            "全量导出目录缺少 FFRS/Content/Sequencer",
        ));
    }

    let mut json_files = Vec::new();
    collect_json_files(&sequencer_dir, &mut json_files)?;
    json_files.sort_by(|a, b| {
        let ra = path_to_slash_string(a.strip_prefix(full_dir).unwrap_or(a));
        let rb = path_to_slash_string(b.strip_prefix(full_dir).unwrap_or(b));
        ra.cmp(&rb)
    });

    ctx.record_dir_source(full_dir, Some(json_files.len() as u64))?;

    let content_root = full_dir.join("FFRS").join("Content");
    let mut seen_paths: BTreeSet<String> = BTreeSet::new();

    let mut next_seq_id: i64 = 1;
    let mut seq_rows: u64 = 0;
    let mut binding_rows: u64 = 0;
    let mut track_rows: u64 = 0;
    let mut event_rows: u64 = 0;
    let mut skipped_non_seq: u64 = 0;
    let mut failed_files: u64 = 0;

    let tx = ctx.tx();
    for file_path in &json_files {
        let path_hint = build_path_hint(file_path, &content_root);
        let rel_label = relative_label_under(full_dir, "export-full", file_path);
        let raw_bytes = match fs::read(file_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                failed_files += 1;
                ctx.record_issue(
                    "parse_error",
                    &path_hint,
                    &format!("读取导出文件失败：{err}"),
                )?;
                continue;
            }
        };
        let value: Value = match serde_json::from_slice(&raw_bytes) {
            Ok(val) => val,
            Err(err) => {
                failed_files += 1;
                ctx.record_issue(
                    "parse_error",
                    &path_hint,
                    &format!("解析导出 JSON 失败：{err}"),
                )?;
                continue;
            }
        };
        let parsed = match parse_level_sequence_document(&path_hint, &value) {
            Ok(Some(parsed)) => parsed,
            Ok(None) => {
                skipped_non_seq += 1;
                continue;
            }
            Err(err) => {
                failed_files += 1;
                ctx.record_issue("parse_error", &path_hint, &err.to_string())?;
                continue;
            }
        };

        let seq_path = &parsed.sequence.path;
        if !seen_paths.insert(seq_path.clone()) {
            ctx.record_issue(
                "duplicate_sequence_path",
                seq_path,
                &format!("发现重复的过场路径（文件：{rel_label}）"),
            )?;
            continue;
        }

        for warning in &parsed.warnings {
            ctx.record_issue(&warning.kind, seq_path, &warning.detail)?;
        }

        let seq_id = next_seq_id;
        next_seq_id += 1;

        let seq_doc = encode_doc("sequence", seq_path, &parsed.sequence)?;
        {
            let mut stmt = tx
                .prepare_cached(
                    "INSERT INTO sequence (
                        id, path, name, category, fps, duration_secs,
                        binding_count, track_count, event_count,
                        director_class, has_skip_markers, doc
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                )
                .map_err(|err| DbError::sqlite("准备写入 sequence 表", err))?;
            stmt.execute(params![
                seq_id,
                parsed.sequence.path,
                parsed.sequence.name,
                parsed.sequence.category.as_str(),
                parsed.sequence.display_rate.as_fps(),
                parsed.sequence.duration_secs,
                i64::from(parsed.sequence.binding_count),
                i64::from(parsed.sequence.track_count),
                i64::from(parsed.sequence.event_count),
                parsed.sequence.director_class,
                i64::from(parsed.sequence.has_skip_markers),
                seq_doc,
            ])
            .map_err(|err| DbError::sqlite(format!("写入 sequence 行 {seq_path}"), err))?;
        }
        seq_rows += 1;

        {
            let mut binding_stmt = tx
                .prepare_cached(
                    "INSERT INTO sequence_binding (
                        sequence_id, guid, name, kind, template_class, map_unit_id, doc
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(|err| DbError::sqlite("准备写入 sequence_binding 表", err))?;
            for binding in &parsed.bindings {
                let doc = encode_doc(
                    "sequence_binding",
                    format!("{seq_path}#{}", binding.guid),
                    binding,
                )?;
                binding_stmt
                    .execute(params![
                        seq_id,
                        binding.guid,
                        binding.name,
                        binding.kind.as_str(),
                        binding.template_class,
                        binding.map_unit_id,
                        doc,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 sequence_binding（过场 {seq_path}）"), err)
                    })?;
                binding_rows += 1;
            }
        }

        {
            let mut track_stmt = tx
                .prepare_cached(
                    "INSERT INTO sequence_track (
                        sequence_id, name, track_type, binding_guid, section_count,
                        start_frame, end_frame, doc
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(|err| DbError::sqlite("准备写入 sequence_track 表", err))?;
            for track in &parsed.tracks {
                let doc = encode_doc(
                    "sequence_track",
                    format!("{seq_path}#{}", track.name),
                    track,
                )?;
                track_stmt
                    .execute(params![
                        seq_id,
                        track.name,
                        track.track_type,
                        track.binding_guid,
                        i64::from(track.section_count),
                        track.start_frame,
                        track.end_frame,
                        doc,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 sequence_track（过场 {seq_path}）"), err)
                    })?;
                track_rows += 1;
            }
        }

        {
            let mut event_stmt = tx
                .prepare_cached(
                    "INSERT INTO sequence_event (
                        sequence_id, time_ticks, frame, time_secs, function,
                        track_name, binding_guid, talk_event_id, flag_id, map_unit_id, doc
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                )
                .map_err(|err| DbError::sqlite("准备写入 sequence_event 表", err))?;
            for (idx, event) in parsed.events.iter().enumerate() {
                let doc = encode_doc("sequence_event", format!("{seq_path}#{idx}"), event)?;
                event_stmt
                    .execute(params![
                        seq_id,
                        event.time_ticks,
                        event.frame,
                        event.time_secs,
                        event.function,
                        event.track_name,
                        event.binding_guid,
                        event.talk_event_id,
                        event.flag_id,
                        event.map_unit_id,
                        doc,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 sequence_event（过场 {seq_path}）"), err)
                    })?;
                event_rows += 1;
            }
        }
    }

    let total_rows = seq_rows + binding_rows + track_rows + event_rows;
    let note = format!(
        "sequence={seq_rows}, binding={binding_rows}, track={track_rows}, event={event_rows}, 非过场跳过={skipped_non_seq}, 解析失败={failed_files}"
    );
    Ok(StageStats::ok_with_note(total_rows, note))
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|err| DbError::io("读取过场目录", dir, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("遍历过场目录项", dir, err))?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out)?;
        } else if path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn build_path_hint(file_path: &Path, content_root: &Path) -> String {
    match file_path.strip_prefix(content_root) {
        Ok(rel) => {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            normalize_sequence_path(&format!("/Game/{rel_str}"))
        }
        Err(_) => normalize_sequence_path(&file_path.to_string_lossy()),
    }
}
