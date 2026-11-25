//! Optional live-game snapshot area (`10_snapshots`) import stage.
//!
//! Reads `.deps/r1/*` scene snapshot directories (and `<root>/hotkey/*` hotkey snapshots) when
//! `--snapshots <DIR>` is explicitly supplied. Missing files or an unfinished `done.json` do not
//! abort the stage: every file that can be read is imported and the problems are recorded in
//! `build_issue` plus the snapshot's own `status` / `issues` document.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use ffr_data::assets::snapshot::{
    Marker, Snapshot, SnapshotObject, SnapshotText, SnapshotWidget, class_short_name,
    label_from_snapshot_dir,
};
use rusqlite::params;
use serde_json::{Map, Value};

use crate::build::{ImportCtx, StageStats};
use crate::error::{DbError, Result};
use crate::record::encode_doc;

const META_FILE: &str = "meta.json";
const CONTEXT_FILE: &str = "context.json";
const CLASSES_FILE: &str = "classes.json";
const OBJECTS_FILE: &str = "objects.jsonl";
const WIDGETS_FILE: &str = "widgets.jsonl";
const TEXTS_FILE: &str = "texts.jsonl";
const DONE_FILE: &str = "done.json";

/// One non-fatal import problem, collected before being written to `build_issue`.
struct NewIssue {
    kind: &'static str,
    subject: String,
    detail: String,
}

impl NewIssue {
    fn new(kind: &'static str, subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            subject: subject.into(),
            detail: detail.into(),
        }
    }
}

/// Import optional live-game R1 scene snapshots.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let Some(root) = ctx.inputs().snapshots.clone() else {
        return Ok(StageStats::skipped("未指定 --snapshots 快照目录"));
    };
    if !root.is_dir() {
        return Ok(StageStats::skipped("输入目录不存在（--snapshots）"));
    }

    let directories = collect_snapshot_dirs(&root)?;
    let mut total_rows = 0_u64;
    let mut snapshot_count = 0_u64;
    let mut object_count = 0_u64;
    let mut widget_count = 0_u64;
    let mut text_count = 0_u64;
    let mut issues = Vec::new();
    let mut seen = BTreeSet::new();

    for (directory, hotkey) in &directories {
        let id = directory_name(directory);
        if !seen.insert(id.clone()) {
            issues.push(NewIssue::new(
                "duplicate_snapshot",
                id,
                "同一份快照目录出现了两次，后一份被跳过",
            ));
            continue;
        }

        let mut snapshot_issues = Vec::new();
        let (snapshot, objects, widgets, texts) =
            read_snapshot(ctx, directory, *hotkey, &mut snapshot_issues)?;
        insert_snapshot(ctx, &snapshot)?;
        object_count += objects;
        widget_count += widgets;
        text_count += texts;
        snapshot_count += 1;
        total_rows += 1 + objects + widgets + texts;
        issues.extend(snapshot_issues);
    }

    let marker_count = import_markers(ctx, &root, &mut issues)?;
    total_rows += marker_count;

    for issue in &issues {
        ctx.record_issue(issue.kind, &issue.subject, &issue.detail)?;
    }
    ctx.record_dir_source(&root, None)?;

    let note = format!(
        "快照 {snapshot_count} 份，对象 {object_count} 个，控件 {widget_count} 个，文本 {text_count} 条，标记 {marker_count} 条"
    );
    Ok(StageStats::ok_with_note(total_rows, note))
}

/// Collect snapshot directories under the root (including `<root>/hotkey/`), sorted by id.
fn collect_snapshot_dirs(root: &Path) -> Result<Vec<(PathBuf, bool)>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root).map_err(|err| DbError::io("读取快照根目录", root, err))?
    {
        let entry = entry.map_err(|err| DbError::io("遍历快照根目录", root, err))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|err| DbError::io("读取快照目录类型", &path, err))?
            .is_dir()
        {
            continue;
        }
        let name = directory_name(&path);
        if name.ends_with("-snapshot") && looks_like_snapshot(&path) {
            out.push((path, false));
        }
    }

    let hotkey_root = root.join("hotkey");
    if hotkey_root.is_dir() {
        for entry in fs::read_dir(&hotkey_root)
            .map_err(|err| DbError::io("读取热键快照目录", &hotkey_root, err))?
        {
            let entry = entry.map_err(|err| DbError::io("遍历热键快照目录", &hotkey_root, err))?;
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|err| DbError::io("读取热键快照目录类型", &path, err))?
                .is_dir()
            {
                continue;
            }
            let name = directory_name(&path);
            if name.starts_with("hotkey-") && looks_like_snapshot(&path) {
                out.push((path, true));
            }
        }
    }

    out.sort_by(|a, b| {
        directory_name(&a.0)
            .cmp(&directory_name(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    Ok(out)
}

fn looks_like_snapshot(path: &Path) -> bool {
    [
        META_FILE,
        CONTEXT_FILE,
        CLASSES_FILE,
        OBJECTS_FILE,
        WIDGETS_FILE,
        TEXTS_FILE,
        DONE_FILE,
    ]
    .iter()
    .any(|name| path.join(name).is_file())
}

fn directory_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// Read one snapshot directory into its objects and textual records.
fn read_snapshot(
    ctx: &ImportCtx<'_>,
    directory: &Path,
    hotkey: bool,
    issues: &mut Vec<NewIssue>,
) -> Result<(Snapshot, u64, u64, u64)> {
    let id = directory_name(directory);
    let meta = load_json_object(directory, META_FILE, "missing_file", &id, issues);
    let context = load_json_object(directory, CONTEXT_FILE, "missing_file", &id, issues);
    let done = load_json_object(directory, DONE_FILE, "incomplete_snapshot", &id, issues);

    // `classes.json` is not stored as its own table in this area; still report a missing file so a
    // partial snapshot is visible instead of silently accepted.
    if !directory.join(CLASSES_FILE).is_file() {
        issues.push(NewIssue::new(
            "missing_file",
            id.clone(),
            format!("缺少文件 {CLASSES_FILE}"),
        ));
    }

    if let Some(map) = &done
        && map.get("ok").and_then(Value::as_bool) == Some(false)
    {
        let error = map
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("未提供错误说明");
        issues.push(NewIssue::new(
            "failed_snapshot",
            id.clone(),
            format!("done.json 报告 ok=false：{error}"),
        ));
    }

    let label = done
        .as_ref()
        .and_then(|map| map.get("label"))
        .or_else(|| meta.as_ref().and_then(|map| map.get("label")))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| label_from_snapshot_dir(&id));

    let started_at = meta
        .as_ref()
        .and_then(|map| map.get("startedAt"))
        .and_then(Value::as_i64)
        .or_else(|| {
            done.as_ref()
                .and_then(|map| map.get("startedAt"))
                .and_then(Value::as_i64)
        });
    let finished_at = meta
        .as_ref()
        .and_then(|map| map.get("finishedAt"))
        .and_then(Value::as_i64)
        .or_else(|| {
            done.as_ref()
                .and_then(|map| map.get("finishedAt"))
                .and_then(Value::as_i64)
        });
    let time = context
        .as_ref()
        .and_then(|map| map.get("time"))
        .and_then(Value::as_i64)
        .or(started_at);
    let world = context
        .as_ref()
        .and_then(|map| map.get("world"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let done_exists = done.is_some();
    let done_failed = done
        .as_ref()
        .and_then(|map| map.get("ok"))
        .and_then(Value::as_bool)
        == Some(false);
    let completed = done_exists && !done_failed;

    let object_count = import_objects(ctx, directory, &id, issues)?;
    let widget_count = import_widgets(ctx, directory, &id, issues)?;
    let text_count = import_texts(ctx, directory, &id, issues)?;

    let status = if !done_exists {
        "incomplete"
    } else if done_failed {
        "failed"
    } else if issues.iter().any(|issue| issue.subject == id) {
        "partial"
    } else {
        "ok"
    };

    // Keep the per-snapshot issue list readable in the `doc` column as well.
    let snapshot_issues = issues
        .iter()
        .filter(|issue| issue.subject == id)
        .map(|issue| format!("{}：{}", issue.kind, issue.detail))
        .collect();

    let snapshot = Snapshot {
        id,
        label,
        time,
        started_at,
        finished_at,
        world,
        completed,
        hotkey,
        status: status.to_owned(),
        issues: snapshot_issues,
    };
    Ok((snapshot, object_count, widget_count, text_count))
}

fn load_json_object(
    directory: &Path,
    file: &str,
    missing_kind: &'static str,
    subject: &str,
    issues: &mut Vec<NewIssue>,
) -> Option<Map<String, Value>> {
    let path = directory.join(file);
    match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Object(map)) => Some(map),
            Ok(_) => {
                issues.push(NewIssue::new(
                    "invalid_json",
                    subject,
                    format!("{file} 顶层不是 JSON 对象"),
                ));
                None
            }
            Err(err) => {
                issues.push(NewIssue::new(
                    "invalid_json",
                    subject,
                    format!("{file} JSON 格式错误：{err}"),
                ));
                None
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let detail = if missing_kind == "incomplete_snapshot" {
                format!("缺少 {file}，快照未完成")
            } else {
                format!("缺少文件 {file}")
            };
            issues.push(NewIssue::new(missing_kind, subject, detail));
            None
        }
        Err(err) => {
            issues.push(NewIssue::new(
                "read_error",
                subject,
                format!("读取 {file} 失败：{err}"),
            ));
            None
        }
    }
}

fn open_jsonl(
    directory: &Path,
    file: &'static str,
    subject: &str,
    issues: &mut Vec<NewIssue>,
) -> Option<BufReader<File>> {
    let path = directory.join(file);
    match File::open(&path) {
        Ok(file) => Some(BufReader::new(file)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            issues.push(NewIssue::new(
                "missing_file",
                subject,
                format!("缺少文件 {file}"),
            ));
            None
        }
        Err(err) => {
            issues.push(NewIssue::new(
                "read_error",
                subject,
                format!("打开 {file} 失败：{err}"),
            ));
            None
        }
    }
}

fn import_objects(
    ctx: &ImportCtx<'_>,
    directory: &Path,
    snapshot_id: &str,
    issues: &mut Vec<NewIssue>,
) -> Result<u64> {
    let Some(reader) = open_jsonl(directory, OBJECTS_FILE, snapshot_id, issues) else {
        return Ok(0);
    };
    let mut count = 0_u64;
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                issues.push(NewIssue::new(
                    "read_error",
                    snapshot_id,
                    format!("读取 {OBJECTS_FILE} 第 {line_number} 行失败：{err}"),
                ));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<SnapshotObject>(&line) {
            Ok(mut object) => {
                object.class_short = class_short_name(&object.class).to_owned();
                insert_object(ctx, snapshot_id, &object)?;
                count += 1;
            }
            Err(err) => issues.push(NewIssue::new(
                "invalid_line",
                snapshot_id,
                format!("{OBJECTS_FILE} 第 {line_number} 行解析失败：{err}"),
            )),
        }
    }
    Ok(count)
}

fn import_widgets(
    ctx: &ImportCtx<'_>,
    directory: &Path,
    snapshot_id: &str,
    issues: &mut Vec<NewIssue>,
) -> Result<u64> {
    let Some(reader) = open_jsonl(directory, WIDGETS_FILE, snapshot_id, issues) else {
        return Ok(0);
    };
    let mut count = 0_u64;
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                issues.push(NewIssue::new(
                    "read_error",
                    snapshot_id,
                    format!("读取 {WIDGETS_FILE} 第 {line_number} 行失败：{err}"),
                ));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<SnapshotWidget>(&line) {
            Ok(mut widget) => {
                widget.class_short = class_short_name(&widget.class).to_owned();
                insert_widget(ctx, snapshot_id, &widget)?;
                count += 1;
            }
            Err(err) => issues.push(NewIssue::new(
                "invalid_line",
                snapshot_id,
                format!("{WIDGETS_FILE} 第 {line_number} 行解析失败：{err}"),
            )),
        }
    }
    Ok(count)
}

fn import_texts(
    ctx: &ImportCtx<'_>,
    directory: &Path,
    snapshot_id: &str,
    issues: &mut Vec<NewIssue>,
) -> Result<u64> {
    let Some(reader) = open_jsonl(directory, TEXTS_FILE, snapshot_id, issues) else {
        return Ok(0);
    };
    let mut count = 0_u64;
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                issues.push(NewIssue::new(
                    "read_error",
                    snapshot_id,
                    format!("读取 {TEXTS_FILE} 第 {line_number} 行失败：{err}"),
                ));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<SnapshotText>(&line) {
            Ok(text) => {
                insert_text(ctx, snapshot_id, &text)?;
                count += 1;
            }
            Err(err) => issues.push(NewIssue::new(
                "invalid_line",
                snapshot_id,
                format!("{TEXTS_FILE} 第 {line_number} 行解析失败：{err}"),
            )),
        }
    }
    Ok(count)
}

fn insert_snapshot(ctx: &ImportCtx<'_>, snapshot: &Snapshot) -> Result<()> {
    let doc = encode_doc("snapshot", &snapshot.id, snapshot)?;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO snapshot
                (id, label, time, started_at, finished_at, world, completed, hotkey, status, doc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(|err| DbError::sqlite("准备写入 snapshot", err))?;
    stmt.execute(params![
        snapshot.id,
        snapshot.label,
        snapshot.time,
        snapshot.started_at,
        snapshot.finished_at,
        snapshot.world,
        snapshot.completed as i64,
        snapshot.hotkey as i64,
        snapshot.status,
        doc,
    ])
    .map_err(|err| DbError::sqlite(format!("写入 snapshot {}", snapshot.id), err))?;
    Ok(())
}

fn insert_object(ctx: &ImportCtx<'_>, snapshot_id: &str, object: &SnapshotObject) -> Result<()> {
    let doc = encode_doc(
        "snapshot_object",
        format!("{snapshot_id}:{}", object.name),
        object,
    )?;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO snapshot_object (snapshot_id, class, class_short, name, doc)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|err| DbError::sqlite("准备写入 snapshot_object", err))?;
    stmt.execute(params![
        snapshot_id,
        object.class,
        object.class_short,
        object.name,
        doc,
    ])
    .map_err(|err| {
        DbError::sqlite(
            format!("写入 snapshot_object {snapshot_id}:{}", object.name),
            err,
        )
    })?;
    Ok(())
}

fn insert_widget(ctx: &ImportCtx<'_>, snapshot_id: &str, widget: &SnapshotWidget) -> Result<()> {
    let doc = encode_doc(
        "snapshot_widget",
        format!("{snapshot_id}:{}", widget.name),
        widget,
    )?;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO snapshot_widget
                (snapshot_id, class, class_short, name, is_in_viewport, visibility, doc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|err| DbError::sqlite("准备写入 snapshot_widget", err))?;
    stmt.execute(params![
        snapshot_id,
        widget.class,
        widget.class_short,
        widget.name,
        widget.is_in_viewport as i64,
        widget.visibility,
        doc,
    ])
    .map_err(|err| {
        DbError::sqlite(
            format!("写入 snapshot_widget {snapshot_id}:{}", widget.name),
            err,
        )
    })?;
    Ok(())
}

fn insert_text(ctx: &ImportCtx<'_>, snapshot_id: &str, text: &SnapshotText) -> Result<()> {
    let doc = encode_doc(
        "snapshot_text",
        format!("{snapshot_id}:{}", text.name),
        text,
    )?;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO snapshot_text
                (snapshot_id, class, name, text, visibility, doc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|err| DbError::sqlite("准备写入 snapshot_text", err))?;
    stmt.execute(params![
        snapshot_id,
        text.class,
        text.name,
        text.text,
        text.visibility,
        doc,
    ])
    .map_err(|err| {
        DbError::sqlite(
            format!("写入 snapshot_text {snapshot_id}:{}", text.name),
            err,
        )
    })?;
    Ok(())
}

fn import_markers(ctx: &mut ImportCtx<'_>, root: &Path, issues: &mut Vec<NewIssue>) -> Result<u64> {
    let mut total = 0_u64;

    let auto_path = root.join("auto").join("markers.log");
    if auto_path.is_file() {
        match fs::read_to_string(&auto_path) {
            Ok(raw) => {
                for line in raw.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let marker = parse_auto_marker(line);
                    insert_marker(ctx, &marker)?;
                    total += 1;
                }
            }
            Err(err) => issues.push(NewIssue::new(
                "read_error",
                "auto/markers.log",
                format!("读取失败：{err}"),
            )),
        }
    }

    let hotkey_path = root.join("hotkey").join("markers.jsonl");
    if hotkey_path.is_file() {
        match File::open(&hotkey_path) {
            Ok(file) => {
                for (index, line) in BufReader::new(file).lines().enumerate() {
                    let line_number = index + 1;
                    let line = match line {
                        Ok(line) => line,
                        Err(err) => {
                            issues.push(NewIssue::new(
                                "read_error",
                                "hotkey/markers.jsonl",
                                format!("第 {line_number} 行读取失败：{err}"),
                            ));
                            break;
                        }
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match parse_hotkey_marker(&line) {
                        Ok(marker) => {
                            insert_marker(ctx, &marker)?;
                            total += 1;
                        }
                        Err(err) => issues.push(NewIssue::new(
                            "invalid_line",
                            "hotkey/markers.jsonl",
                            format!("第 {line_number} 行解析失败：{err}"),
                        )),
                    }
                }
            }
            Err(err) => issues.push(NewIssue::new(
                "read_error",
                "hotkey/markers.jsonl",
                format!("打开失败：{err}"),
            )),
        }
    }

    Ok(total)
}

fn parse_auto_marker(line: &str) -> Marker {
    let mut local_time = None;
    let mut text = line;
    if let Some((prefix, rest)) = line.split_once(char::is_whitespace)
        && is_clock_time(prefix)
    {
        local_time = Some(prefix.to_owned());
        text = rest.trim_start();
    }
    Marker {
        source: "auto".to_owned(),
        seq: None,
        time: None,
        local_time,
        world: None,
        text: text.to_owned(),
        clock_ms: None,
    }
}

fn is_clock_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 8
        && bytes[2] == b':'
        && bytes[5] == b':'
        && bytes[0..2].iter().all(u8::is_ascii_digit)
        && bytes[3..5].iter().all(u8::is_ascii_digit)
        && bytes[6..8].iter().all(u8::is_ascii_digit)
}

fn parse_hotkey_marker(line: &str) -> std::result::Result<Marker, String> {
    let value: Value = serde_json::from_str(line).map_err(|err| err.to_string())?;
    let map = value
        .as_object()
        .ok_or_else(|| "标记不是 JSON 对象".to_owned())?;
    Ok(Marker {
        source: "hotkey".to_owned(),
        seq: map.get("seq").and_then(Value::as_u64),
        time: map.get("time").and_then(Value::as_i64),
        local_time: map
            .get("localTime")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        world: map
            .get("world")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        text: map
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        clock_ms: map.get("clock_ms").and_then(Value::as_i64),
    })
}

fn insert_marker(ctx: &ImportCtx<'_>, marker: &Marker) -> Result<()> {
    let doc = encode_doc(
        "marker",
        format!("{:?}:{:?}", marker.source, marker.seq),
        marker,
    )?;
    let mut stmt = ctx
        .tx()
        .prepare_cached(
            "INSERT INTO marker
                (source, seq, time, local_time, world, text, clock_ms, doc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(|err| DbError::sqlite("准备写入 marker", err))?;
    let seq = marker.seq.and_then(|v| i64::try_from(v).ok());
    stmt.execute(params![
        marker.source,
        seq,
        marker.time,
        marker.local_time,
        marker.world,
        marker.text,
        marker.clock_ms,
        doc,
    ])
    .map_err(|err| DbError::sqlite("写入 marker", err))?;
    Ok(())
}
