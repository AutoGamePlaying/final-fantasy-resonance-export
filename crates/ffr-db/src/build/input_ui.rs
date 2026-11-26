//! Input, DataAsset, Blueprint, and UI widget area (`07_input_ui`) import stage.
//!
//! Scans exported CUE4Parse JSON files under `<full>/FFRS/Content` and `<full>/FFRS/Plugins`,
//! classifies each asset by its exported UE `Type`, populates the `07_input_ui` tables, and logs
//! any parse failures or dangling references into `build_issue` without aborting the build.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ffr_data::assets::blueprint::{
    Blueprint, BlueprintKind, DataAsset, parse_blueprint, parse_data_asset,
};
use ffr_data::assets::input::{
    InputAction, InputMappingContext, parse_input_action, parse_input_mapping_context,
    parse_input_set,
};
use ffr_data::assets::widget::{WidgetNode, parse_widget_tree};
use rusqlite::params;
use serde_json::Value;

use crate::build::{ImportCtx, StageStats, UePathLookup};
use crate::error::{DbError, Result};
use crate::record::encode_doc;
use crate::util::path_to_slash_string;

struct PendingMappingRef {
    context_id: String,
    ord: usize,
    action_id: String,
}

struct PendingSetContextRef {
    asset: String,
    device: String,
    context_id: String,
}

struct PendingSetActionRef {
    asset: String,
    semantic: String,
    action_id: String,
}

struct PendingBlueprintParentRef {
    bp_path: String,
    parent_class: String,
    parent_path: String,
}

struct PendingWidgetClassRef {
    bp_path: String,
    widget_name: String,
    widget_class: String,
    class_path: String,
}

#[derive(Default)]
struct AreaCounts {
    input_action: u64,
    input_context: u64,
    input_mapping: u64,
    input_set: u64,
    input_set_action: u64,
    data_asset: u64,
    blueprint: u64,
    blueprint_component: u64,
    widget_node: u64,
    parse_issues: u64,
}

impl AreaCounts {
    fn total_rows(&self) -> u64 {
        self.input_action
            + self.input_context
            + self.input_mapping
            + self.input_set
            + self.input_set_action
            + self.data_asset
            + self.blueprint
            + self.blueprint_component
            + self.widget_node
    }
}

/// Import Enhanced Input, DataAssets, Blueprints/components, and UMG widget trees.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let Some(full_dir) = ctx.inputs().full.as_deref() else {
        return Ok(StageStats::skipped("未指定全量导出目录（--full）"));
    };

    let ffrs_dir = full_dir.join("FFRS");
    let content_dir = ffrs_dir.join("Content");
    let plugins_dir = ffrs_dir.join("Plugins");

    let mut scan_roots = Vec::new();
    if content_dir.is_dir() {
        scan_roots.push(content_dir);
    }
    if plugins_dir.is_dir() {
        scan_roots.push(plugins_dir);
    }
    if scan_roots.is_empty() {
        return Ok(StageStats::skipped(
            "全量导出目录中缺少 FFRS/Content 或 FFRS/Plugins 目录",
        ));
    }

    let mut json_files = Vec::new();
    for root in &scan_roots {
        collect_json_files(root, &mut json_files)?;
    }
    json_files.sort_by(|a, b| {
        let ra = path_to_slash_string(a.strip_prefix(full_dir).unwrap_or(a));
        let rb = path_to_slash_string(b.strip_prefix(full_dir).unwrap_or(b));
        ra.cmp(&rb)
    });
    ctx.record_dir_source(full_dir, Some(json_files.len() as u64))?;

    let mut counts = AreaCounts::default();
    let mut known_action_ids: BTreeSet<String> = BTreeSet::new();
    let mut known_action_paths: BTreeSet<String> = BTreeSet::new();
    let mut known_context_ids: BTreeSet<String> = BTreeSet::new();
    let mut known_context_paths: BTreeSet<String> = BTreeSet::new();
    let mut known_da_paths: BTreeSet<String> = BTreeSet::new();
    let mut known_bp_paths: BTreeSet<String> = BTreeSet::new();
    let mut bp_path_lookup: UePathLookup<()> = UePathLookup::new();
    let mut known_bp_classes: BTreeSet<String> = BTreeSet::new();

    // Collect rows into BTreeMap/BTreeSet keyed by natural PRIMARY KEY before writing so
    // database page layout is independent of directory traversal or file order.
    let mut actions_by_id: BTreeMap<String, InputAction> = BTreeMap::new();
    let mut contexts_by_id: BTreeMap<String, InputMappingContext> = BTreeMap::new();
    let mut input_sets: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut input_set_actions: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut data_assets_by_path: BTreeMap<String, DataAsset> = BTreeMap::new();
    let mut blueprints_by_path: BTreeMap<String, (Blueprint, BTreeMap<String, WidgetNode>)> =
        BTreeMap::new();

    let mut pending_mappings: Vec<PendingMappingRef> = Vec::new();
    let mut pending_set_contexts: Vec<PendingSetContextRef> = Vec::new();
    let mut pending_set_actions: Vec<PendingSetActionRef> = Vec::new();
    let mut pending_bp_parents: Vec<PendingBlueprintParentRef> = Vec::new();
    let mut pending_widget_classes: Vec<PendingWidgetClassRef> = Vec::new();

    for path in &json_files {
        let rel_label = path_to_slash_string(path.strip_prefix(full_dir).unwrap_or(path));

        let raw_bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                counts.parse_issues += 1;
                ctx.record_issue("io_error", &rel_label, &format!("读取导出文件失败：{err}"))?;
                continue;
            }
        };

        if can_skip_raw_export(&raw_bytes) {
            continue;
        }

        let doc: Value = match serde_json::from_slice(&raw_bytes) {
            Ok(val) => val,
            Err(err) => {
                counts.parse_issues += 1;
                ctx.record_issue(
                    "parse_error",
                    &rel_label,
                    &format!("解析导出 JSON 失败：{err}"),
                )?;
                continue;
            }
        };

        // 1. Enhanced Input: InputAction
        match parse_input_action(&doc, &rel_label) {
            Ok(Some(ia)) => {
                if !known_action_ids.insert(ia.id.clone())
                    || !known_action_paths.insert(ia.path.clone())
                {
                    ctx.record_issue(
                        "duplicate_input_action",
                        &rel_label,
                        &format!("重复的 InputAction：{} ({})", ia.id, ia.path),
                    )?;
                } else {
                    actions_by_id.insert(ia.id.clone(), ia);
                }
                continue;
            }
            Ok(None) => {}
            Err(err) => {
                counts.parse_issues += 1;
                ctx.record_issue("parse_error", &rel_label, &err.to_string())?;
                continue;
            }
        }

        // 2. Enhanced Input: InputMappingContext
        match parse_input_mapping_context(&doc, &rel_label) {
            Ok(Some(imc)) => {
                if !known_context_ids.insert(imc.id.clone())
                    || !known_context_paths.insert(imc.path.clone())
                {
                    ctx.record_issue(
                        "duplicate_input_context",
                        &rel_label,
                        &format!("重复的 InputMappingContext：{} ({})", imc.id, imc.path),
                    )?;
                } else {
                    for mapping in &imc.mappings {
                        pending_mappings.push(PendingMappingRef {
                            context_id: imc.id.clone(),
                            ord: mapping.ord,
                            action_id: mapping.action_id.clone(),
                        });
                    }
                    contexts_by_id.insert(imc.id.clone(), imc);
                }
                continue;
            }
            Ok(None) => {}
            Err(err) => {
                counts.parse_issues += 1;
                ctx.record_issue("parse_error", &rel_label, &err.to_string())?;
                continue;
            }
        }

        // 3. Blueprint / WidgetBlueprint
        match parse_blueprint(&doc, &rel_label) {
            Ok(Some(bp)) => {
                if !known_bp_paths.insert(bp.path.clone()) {
                    ctx.record_issue(
                        "duplicate_blueprint",
                        &rel_label,
                        &format!("重复的蓝图路径：{}", bp.path),
                    )?;
                    continue;
                }
                bp_path_lookup.insert(bp.path.clone(), ());
                known_bp_classes.insert(bp.class_name.clone());

                if let (Some(parent_class), Some(parent_path)) = (&bp.parent_class, &bp.parent_path)
                    && !parent_path
                        .get(..8)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("/Script/"))
                {
                    pending_bp_parents.push(PendingBlueprintParentRef {
                        bp_path: bp.path.clone(),
                        parent_class: parent_class.clone(),
                        parent_path: parent_path.clone(),
                    });
                }

                let mut widget_nodes: BTreeMap<String, WidgetNode> = BTreeMap::new();
                if bp.kind == BlueprintKind::Widget {
                    match parse_widget_tree(&doc, &rel_label) {
                        Ok(Some(tree)) => {
                            for node in tree.flatten() {
                                if widget_nodes.contains_key(&node.name) {
                                    ctx.record_issue(
                                        "duplicate_widget_node",
                                        &format!("{}:{}", bp.path, node.name),
                                        "同一 WidgetTree 中存在重名控件，已跳过重复节点",
                                    )?;
                                    continue;
                                }
                                if let Some(class_path) = &node.class_path
                                    && !class_path.get(..8).is_some_and(|prefix| {
                                        prefix.eq_ignore_ascii_case("/Script/")
                                    })
                                {
                                    pending_widget_classes.push(PendingWidgetClassRef {
                                        bp_path: bp.path.clone(),
                                        widget_name: node.name.clone(),
                                        widget_class: node.class.clone(),
                                        class_path: class_path.clone(),
                                    });
                                }
                                widget_nodes.insert(node.name.clone(), node.clone());
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            counts.parse_issues += 1;
                            ctx.record_issue("parse_error", &rel_label, &err.to_string())?;
                        }
                    }
                }
                blueprints_by_path.insert(bp.path.clone(), (bp, widget_nodes));
                continue;
            }
            Ok(None) => {}
            Err(err) => {
                counts.parse_issues += 1;
                ctx.record_issue("parse_error", &rel_label, &err.to_string())?;
                continue;
            }
        }

        // 4. InputSet DataAsset (`CPP_DA_MUCHA_INPUT`, etc.) + generic DataAsset table
        match parse_input_set(&doc, &rel_label) {
            Ok(Some(set)) => {
                for entry in &set.contexts {
                    input_sets.insert((
                        set.asset.clone(),
                        entry.device.clone(),
                        entry.context_id.clone(),
                    ));
                    pending_set_contexts.push(PendingSetContextRef {
                        asset: set.asset.clone(),
                        device: entry.device.clone(),
                        context_id: entry.context_id.clone(),
                    });
                }
                for entry in &set.actions {
                    input_set_actions
                        .entry((set.asset.clone(), entry.semantic.clone()))
                        .or_insert_with(|| entry.action_id.clone());
                    pending_set_actions.push(PendingSetActionRef {
                        asset: set.asset.clone(),
                        semantic: entry.semantic.clone(),
                        action_id: entry.action_id.clone(),
                    });
                }
            }
            Ok(None) => {}
            Err(err) => {
                counts.parse_issues += 1;
                ctx.record_issue("parse_error", &rel_label, &err.to_string())?;
            }
        }

        match parse_data_asset(&doc, &rel_label) {
            Ok(Some(da)) => {
                if !known_da_paths.insert(da.path.clone()) {
                    ctx.record_issue(
                        "duplicate_data_asset",
                        &rel_label,
                        &format!("重复的 DataAsset 路径：{}", da.path),
                    )?;
                } else {
                    data_assets_by_path.insert(da.path.clone(), da);
                }
            }
            Ok(None) => {}
            Err(err) => {
                counts.parse_issues += 1;
                ctx.record_issue("parse_error", &rel_label, &err.to_string())?;
            }
        }
    }

    // Write every table in deterministic natural primary-key order.
    {
        let tx = ctx.tx();
        let mut stmt_ia = tx
            .prepare_cached(
                "INSERT INTO input_action (id, path, value_type, doc) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 input_action", err))?;
        for ia in actions_by_id.values() {
            let json_doc = encode_doc("input_action", &ia.id, ia)?;
            stmt_ia
                .execute(params![ia.id, ia.path, ia.value_type, json_doc])
                .map_err(|err| DbError::sqlite(format!("写入 input_action {}", ia.id), err))?;
            counts.input_action += 1;
        }

        let mut stmt_imc = tx
            .prepare_cached("INSERT INTO input_context (id, path, doc) VALUES (?1, ?2, ?3)")
            .map_err(|err| DbError::sqlite("准备写入 input_context", err))?;
        let mut stmt_mapping = tx
            .prepare_cached(
                "INSERT INTO input_mapping (context_id, action_id, key, modifiers, triggers, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 input_mapping", err))?;
        for imc in contexts_by_id.values() {
            let json_doc = encode_doc("input_context", &imc.id, imc)?;
            stmt_imc
                .execute(params![imc.id, imc.path, json_doc])
                .map_err(|err| DbError::sqlite(format!("写入 input_context {}", imc.id), err))?;
            counts.input_context += 1;

            let mut sorted_mappings: Vec<_> = imc.mappings.iter().collect();
            sorted_mappings.sort_by_key(|m| m.ord);
            for mapping in sorted_mappings {
                let mods_json = encode_doc(
                    "input_mapping.modifiers",
                    format!("{}:{}", imc.id, mapping.ord),
                    &mapping.modifiers,
                )?;
                let trigs_json = encode_doc(
                    "input_mapping.triggers",
                    format!("{}:{}", imc.id, mapping.ord),
                    &mapping.triggers,
                )?;
                stmt_mapping
                    .execute(params![
                        imc.id,
                        mapping.action_id,
                        mapping.key,
                        mods_json,
                        trigs_json,
                        i64::try_from(mapping.ord).unwrap_or(i64::MAX),
                    ])
                    .map_err(|err| {
                        DbError::sqlite(
                            format!("写入 input_mapping {}:{}", imc.id, mapping.ord),
                            err,
                        )
                    })?;
                counts.input_mapping += 1;
            }
        }

        let mut stmt_set = tx
            .prepare_cached(
                "INSERT OR IGNORE INTO input_set (asset, device, context_id) VALUES (?1, ?2, ?3)",
            )
            .map_err(|err| DbError::sqlite("准备写入 input_set", err))?;
        for (asset, device, context_id) in &input_sets {
            let inserted = stmt_set
                .execute(params![asset, device, context_id])
                .map_err(|err| DbError::sqlite(format!("写入 input_set {asset}:{device}"), err))?;
            counts.input_set += u64::try_from(inserted).unwrap_or(0);
        }

        let mut stmt_set_action = tx
            .prepare_cached(
                "INSERT OR IGNORE INTO input_set_action (asset, semantic, action_id) \
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|err| DbError::sqlite("准备写入 input_set_action", err))?;
        for ((asset, semantic), action_id) in &input_set_actions {
            let inserted = stmt_set_action
                .execute(params![asset, semantic, action_id])
                .map_err(|err| {
                    DbError::sqlite(format!("写入 input_set_action {asset}:{semantic}"), err)
                })?;
            counts.input_set_action += u64::try_from(inserted).unwrap_or(0);
        }

        let mut stmt_da = tx
            .prepare_cached("INSERT INTO data_asset (path, class, doc) VALUES (?1, ?2, ?3)")
            .map_err(|err| DbError::sqlite("准备写入 data_asset", err))?;
        for da in data_assets_by_path.values() {
            let json_doc = encode_doc("data_asset", &da.path, da)?;
            stmt_da
                .execute(params![da.path, da.class, json_doc])
                .map_err(|err| DbError::sqlite(format!("写入 data_asset {}", da.path), err))?;
            counts.data_asset += 1;
        }

        let mut stmt_bp = tx
            .prepare_cached(
                "INSERT INTO blueprint (path, class_name, parent_class, kind, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| DbError::sqlite("准备写入 blueprint", err))?;
        let mut stmt_comp = tx
            .prepare_cached(
                "INSERT INTO blueprint_component (blueprint_id, name, class, doc) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| DbError::sqlite("准备写入 blueprint_component", err))?;
        let mut stmt_widget = tx
            .prepare_cached(
                "INSERT INTO widget_node (blueprint_id, name, class, parent_name, ord, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| DbError::sqlite("准备写入 widget_node", err))?;

        for (bp, widget_nodes) in blueprints_by_path.values() {
            let json_doc = encode_doc("blueprint", &bp.path, bp)?;
            stmt_bp
                .execute(params![
                    bp.path,
                    bp.class_name,
                    bp.parent_class,
                    bp.kind.as_str(),
                    json_doc
                ])
                .map_err(|err| DbError::sqlite(format!("写入 blueprint {}", bp.path), err))?;
            counts.blueprint += 1;

            let mut sorted_comps: Vec<_> = bp.components.iter().collect();
            sorted_comps.sort_by(|a, b| a.name.cmp(&b.name));
            for comp in sorted_comps {
                let comp_doc = encode_doc(
                    "blueprint_component",
                    format!("{}:{}", bp.path, comp.name),
                    comp,
                )?;
                stmt_comp
                    .execute(params![bp.path, comp.name, comp.class, comp_doc])
                    .map_err(|err| {
                        DbError::sqlite(
                            format!("写入 blueprint_component {}:{}", bp.path, comp.name),
                            err,
                        )
                    })?;
                counts.blueprint_component += 1;
            }

            for node in widget_nodes.values() {
                let node_doc =
                    encode_doc("widget_node", format!("{}:{}", bp.path, node.name), node)?;
                stmt_widget
                    .execute(params![
                        bp.path,
                        node.name,
                        node.class,
                        node.parent_name,
                        i64::try_from(node.ord).unwrap_or(i64::MAX),
                        node_doc,
                    ])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 widget_node {}:{}", bp.path, node.name), err)
                    })?;
                counts.widget_node += 1;
            }
        }
    }

    // Post-pass dangling reference verification (recorded in deterministic order).
    pending_mappings.sort_by(|a, b| {
        a.context_id
            .cmp(&b.context_id)
            .then_with(|| a.ord.cmp(&b.ord))
            .then_with(|| a.action_id.cmp(&b.action_id))
    });
    for m in pending_mappings {
        if !known_action_ids.contains(&m.action_id) {
            ctx.record_issue(
                "dangling_input_action",
                &format!("{}[{}]", m.context_id, m.ord),
                &format!("映射引用了不存在的 InputAction：{}", m.action_id),
            )?;
        }
    }
    pending_set_contexts.sort_by(|a, b| {
        a.asset
            .cmp(&b.asset)
            .then_with(|| a.device.cmp(&b.device))
            .then_with(|| a.context_id.cmp(&b.context_id))
    });
    for s in pending_set_contexts {
        if !known_context_ids.contains(&s.context_id) {
            ctx.record_issue(
                "dangling_input_context",
                &format!("{}:{}", s.asset, s.device),
                &format!(
                    "InputSet 引用了不存在的 InputMappingContext：{}",
                    s.context_id
                ),
            )?;
        }
    }
    pending_set_actions.sort_by(|a, b| {
        a.asset
            .cmp(&b.asset)
            .then_with(|| a.semantic.cmp(&b.semantic))
            .then_with(|| a.action_id.cmp(&b.action_id))
    });
    for a in pending_set_actions {
        if !known_action_ids.contains(&a.action_id) {
            ctx.record_issue(
                "dangling_input_action",
                &format!("{}:{}", a.asset, a.semantic),
                &format!(
                    "InputSet 语义动作引用了不存在的 InputAction：{}",
                    a.action_id
                ),
            )?;
        }
    }
    bp_path_lookup.record_collisions(ctx)?;
    pending_bp_parents.sort_by(|a, b| {
        a.bp_path
            .cmp(&b.bp_path)
            .then_with(|| a.parent_path.cmp(&b.parent_path))
    });
    for p in pending_bp_parents {
        if !bp_path_lookup.contains(&p.parent_path) && !known_bp_classes.contains(&p.parent_class) {
            ctx.record_issue(
                "dangling_blueprint_parent",
                &p.bp_path,
                &format!(
                    "蓝图父类未在导出中找到：{} ({})",
                    p.parent_class, p.parent_path
                ),
            )?;
        }
    }
    pending_widget_classes.sort_by(|a, b| {
        a.bp_path
            .cmp(&b.bp_path)
            .then_with(|| a.widget_name.cmp(&b.widget_name))
            .then_with(|| a.class_path.cmp(&b.class_path))
    });
    for w in pending_widget_classes {
        if !bp_path_lookup.contains(&w.class_path) && !known_bp_classes.contains(&w.widget_class) {
            ctx.record_issue(
                "dangling_widget_class",
                &format!("{}:{}", w.bp_path, w.widget_name),
                &format!(
                    "控件引用的 UserWidget 蓝图未在导出中找到：{} ({})",
                    w.widget_class, w.class_path
                ),
            )?;
        }
    }

    let note = format!(
        "IA={}, IMC={}, mapping={}, set={}, set_action={}, DA={}, BP={}, comp={}, widget={}, parse_err={}",
        counts.input_action,
        counts.input_context,
        counts.input_mapping,
        counts.input_set,
        counts.input_set_action,
        counts.data_asset,
        counts.blueprint,
        counts.blueprint_component,
        counts.widget_node,
        counts.parse_issues
    );
    Ok(StageStats::ok_with_note(counts.total_rows(), note))
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|err| DbError::io("读取导出目录", dir, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| DbError::io("遍历导出目录", dir, err))?;
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

/// Fast byte-level pre-filter skipping domains unrelated to `07_input_ui` (such as 1.16 GiB of
/// `LevelSequence` exports and 93 MiB of `DataTable`/`StringTable` exports) without building a
/// full `serde_json::Value` DOM tree.
fn can_skip_raw_export(bytes: &[u8]) -> bool {
    if contains_subslice(bytes, b"\"Type\": \"LevelSequence\"")
        || contains_subslice(bytes, b"\"Type\":\"LevelSequence\"")
    {
        return true;
    }

    let head = &bytes[..bytes.len().min(512)];
    let is_skip_head = contains_subslice(head, b"\"Type\": \"DataTable\"")
        || contains_subslice(head, b"\"Type\":\"DataTable\"")
        || contains_subslice(head, b"\"Type\": \"CompositeDataTable\"")
        || contains_subslice(head, b"\"Type\":\"CompositeDataTable\"")
        || contains_subslice(head, b"\"Type\": \"StringTable\"")
        || contains_subslice(head, b"\"Type\":\"StringTable\"")
        || contains_subslice(head, b"\"Type\": \"Curve")
        || contains_subslice(head, b"\"Type\":\"Curve")
        || contains_subslice(head, b"\"Type\": \"UserDefinedEnum\"")
        || contains_subslice(head, b"\"Type\":\"UserDefinedEnum\"")
        || contains_subslice(head, b"\"Type\": \"UserDefinedStruct\"")
        || contains_subslice(head, b"\"Type\":\"UserDefinedStruct\"");

    if !is_skip_head {
        return false;
    }

    // Guard against multi-object files that also embed an input/blueprint asset.
    !contains_subslice(bytes, b"BlueprintGeneratedClass")
        && !contains_subslice(bytes, b"\"InputAction\"")
        && !contains_subslice(bytes, b"\"InputMappingContext\"")
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    let first = needle[0];
    let mut i = 0;
    let limit = haystack.len() - needle.len();
    while i <= limit {
        if haystack[i] == first && &haystack[i..i + needle.len()] == needle {
            return true;
        }
        i += 1;
    }
    false
}
