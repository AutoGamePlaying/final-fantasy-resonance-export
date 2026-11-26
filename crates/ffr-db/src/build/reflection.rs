//! Reflection area (`06_reflection`) import stage.
//!
//! Streams one or more UE4SS `.jmap` dumps via [`ffr_data::assets::reflection::stream_jmap_file`],
//! merges them in CLI order (later files override conflicting definitions while recording each
//! difference in `build_issue`), records per-file SHA-256 digests in `source`, checks dangling
//! reflection references, and bulk-inserts `ue_class`, `ue_property`, `ue_function`, and
//! `ue_enum_value`.

use std::collections::{BTreeMap, BTreeSet};

use ffr_data::assets::reflection::{JmapEntry, UeClass, UeFunction, UeProperty, stream_jmap_file};
use rusqlite::params;
use serde::Serialize;

use crate::build::{ImportCtx, StageStats, UePathLookup, jmap_display_labels, jmap_logical_paths};
use crate::error::{DbError, Result};
use crate::record::encode_doc;
use crate::util::sha256_file;

struct MergeConflict {
    subject: String,
    detail: String,
}

/// Compact parameter summary stored in `ue_function.params` for quick SQL/JSON inspection.
#[derive(Serialize)]
struct FunctionParamSummary<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    prop_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    inner: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    out: bool,
    #[serde(rename = "return", skip_serializing_if = "is_false")]
    is_return: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn summarize_function_params(func: &UeFunction) -> Vec<FunctionParamSummary<'_>> {
    let mut out = Vec::with_capacity(func.params.len() + usize::from(func.return_value.is_some()));
    for p in &func.params {
        out.push(FunctionParamSummary {
            name: &p.name,
            prop_type: &p.prop_type,
            inner: p.inner_summary(),
            out: p.is_out_param(),
            is_return: false,
        });
    }
    if let Some(ret) = &func.return_value {
        out.push(FunctionParamSummary {
            name: &ret.name,
            prop_type: &ret.prop_type,
            inner: ret.inner_summary(),
            out: true,
            is_return: true,
        });
    }
    out
}

/// Import Unreal reflection metadata from one or more `.jmap` files.
pub fn import(ctx: &mut ImportCtx<'_>) -> Result<StageStats> {
    let jmaps = &ctx.inputs().jmaps;
    if jmaps.is_empty() {
        return Ok(StageStats::skipped("未指定 --jmap 反射文件"));
    }

    let mut classes: BTreeMap<String, (UeClass, usize)> = BTreeMap::new();
    let mut functions: BTreeMap<(String, String), (UeFunction, usize)> = BTreeMap::new();
    let mut packages: BTreeSet<String> = BTreeSet::new();
    let mut conflicts: Vec<MergeConflict> = Vec::new();

    let mut added_classes_later: u64 = 0;
    let mut added_functions_later: u64 = 0;
    let mut class_conflicts: u64 = 0;
    let mut function_conflicts: u64 = 0;

    let source_labels = jmap_display_labels(jmaps);
    let source_logical_paths = jmap_logical_paths(jmaps);
    let mut recorded_sources: BTreeMap<String, (String, u64)> = BTreeMap::new();

    for (file_idx, path) in jmaps.iter().enumerate() {
        let sha256 = sha256_file(path)?;
        let cur_label = &source_labels[file_idx];

        let summary = stream_jmap_file(path, |entry| match entry {
            JmapEntry::Class(cls) => {
                let key = cls.path.clone();
                if let Some((prev, prev_idx)) = classes.get_mut(&key) {
                    if *prev != cls {
                        let detail =
                            describe_class_diff(prev, &cls, &source_labels[*prev_idx], cur_label);
                        conflicts.push(MergeConflict {
                            subject: key,
                            detail,
                        });
                        class_conflicts += 1;
                        *prev = cls;
                        *prev_idx = file_idx;
                    }
                } else {
                    if file_idx > 0 {
                        added_classes_later += 1;
                    }
                    classes.insert(key, (cls, file_idx));
                }
            }
            JmapEntry::Function(func) => {
                let key = (func.owner.clone(), func.name.clone());
                if let Some((prev, prev_idx)) = functions.get_mut(&key) {
                    if *prev != func {
                        let detail = describe_function_diff(
                            prev,
                            &func,
                            &source_labels[*prev_idx],
                            cur_label,
                        );
                        conflicts.push(MergeConflict {
                            subject: func.path.clone(),
                            detail,
                        });
                        function_conflicts += 1;
                        *prev = func;
                        *prev_idx = file_idx;
                    }
                } else {
                    if file_idx > 0 {
                        added_functions_later += 1;
                    }
                    functions.insert(key, (func, file_idx));
                }
            }
            JmapEntry::Package(pkg) => {
                packages.insert(pkg);
            }
            JmapEntry::Ignored => {}
        })?;

        recorded_sources.insert(
            source_logical_paths[file_idx].clone(),
            (sha256, summary.total_objects),
        );
    }

    for (logical_path, (sha256, total_objects)) in &recorded_sources {
        ctx.record_logical_source(logical_path, Some(sha256), Some(*total_objects))?;
    }

    conflicts.sort_by(|a, b| {
        a.subject
            .cmp(&b.subject)
            .then_with(|| a.detail.cmp(&b.detail))
    });
    for conflict in &conflicts {
        ctx.record_issue("merge_conflict", &conflict.subject, &conflict.detail)?;
    }

    let mut class_lookup: UePathLookup<()> = UePathLookup::new();
    for path in classes.keys() {
        class_lookup.insert(path.clone(), ());
    }
    class_lookup.record_collisions(ctx)?;

    let mut package_lookup: UePathLookup<()> = UePathLookup::new();
    for pkg in &packages {
        package_lookup.insert(pkg.clone(), ());
    }
    package_lookup.record_collisions(ctx)?;

    let mut function_lookup: UePathLookup<()> = UePathLookup::new();
    for (func, _) in functions.values() {
        function_lookup.insert(func.path.clone(), ());
    }
    function_lookup.record_collisions(ctx)?;

    let mut class_rows: u64 = 0;
    let mut prop_rows: u64 = 0;
    let mut func_rows: u64 = 0;
    let mut enum_val_rows: u64 = 0;

    {
        let mut class_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO ue_class \
                 (path, short_name, kind, package, super_class, size, flags, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|err| DbError::sqlite("准备写入 ue_class", err))?;

        let mut prop_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT OR REPLACE INTO ue_property \
                 (owner, name, type, offset, size, inner, ord, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|err| DbError::sqlite("准备写入 ue_property", err))?;

        let mut enum_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT OR REPLACE INTO ue_enum_value \
                 (enum, name, short_name, value, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| DbError::sqlite("准备写入 ue_enum_value", err))?;

        let mut ref_classes: Vec<(&'static str, &str)> = Vec::new();
        let mut ref_funcs: Vec<&str> = Vec::new();

        for (cls, _) in classes.values() {
            // Check dangling inheritance and interface references without aborting.
            if let Some(sup) = &cls.super_class
                && !class_lookup.contains(sup)
            {
                ctx.record_issue(
                    "dangling_super_class",
                    &cls.path,
                    &format!("父类 {sup} 不在反射数据中"),
                )?;
            }
            for itf in &cls.interfaces {
                if !class_lookup.contains(&itf.class) {
                    ctx.record_issue(
                        "dangling_interface",
                        &cls.path,
                        &format!("实现的接口 {} 不在反射数据中", itf.class),
                    )?;
                }
            }

            let doc = encode_doc("ue_class", &cls.path, cls)?;
            class_stmt
                .execute(params![
                    cls.path,
                    cls.short_name,
                    cls.kind.as_str(),
                    cls.package,
                    cls.super_class,
                    i64::from(cls.size),
                    cls.flags,
                    doc,
                ])
                .map_err(|err| DbError::sqlite(format!("写入 ue_class {}", cls.path), err))?;
            class_rows += 1;

            // Resolve duplicate property names in declaration order (later wins), then insert in
            // natural PRIMARY KEY `(owner, name)` order.
            let mut props_by_name: BTreeMap<&str, (usize, &UeProperty)> = BTreeMap::new();
            for (ord, prop) in cls.properties.iter().enumerate() {
                if props_by_name
                    .insert(prop.name.as_str(), (ord, prop))
                    .is_some()
                {
                    let prop_key = format!("{}.{}", cls.path, prop.name);
                    ctx.record_issue(
                        "duplicate_property",
                        &prop_key,
                        "同一类下出现同名属性，以后者为准",
                    )?;
                }
            }
            for (ord, prop) in props_by_name.values() {
                let prop_key = format!("{}.{}", cls.path, prop.name);
                check_property_refs(
                    ctx,
                    &prop_key,
                    prop,
                    &class_lookup,
                    &function_lookup,
                    &mut ref_classes,
                    &mut ref_funcs,
                )?;

                let inner = prop.inner_summary();
                let prop_doc = encode_doc("ue_property", &prop_key, prop)?;
                prop_stmt
                    .execute(params![
                        cls.path,
                        prop.name,
                        prop.prop_type,
                        i64::from(prop.offset),
                        i64::from(prop.size),
                        inner,
                        i64::try_from(*ord).unwrap_or(i64::MAX),
                        prop_doc,
                    ])
                    .map_err(|err| DbError::sqlite(format!("写入 ue_property {prop_key}"), err))?;
                prop_rows += 1;
            }

            // Resolve duplicate enum values in declaration order (later wins), then insert in
            // natural PRIMARY KEY `(enum, name)` order.
            let mut enums_by_name: BTreeMap<
                &str,
                (usize, &ffr_data::assets::reflection::UeEnumValue),
            > = BTreeMap::new();
            for (ord, ev) in cls.enum_values.iter().enumerate() {
                if enums_by_name.insert(ev.name.as_str(), (ord, ev)).is_some() {
                    ctx.record_issue(
                        "duplicate_enum_value",
                        &format!("{}::{}", cls.path, ev.name),
                        "同一枚举下出现同名枚举项，以后者为准",
                    )?;
                }
            }
            for (ord, ev) in enums_by_name.values() {
                enum_stmt
                    .execute(params![
                        cls.path,
                        ev.name,
                        ev.short_name,
                        ev.value,
                        i64::try_from(*ord).unwrap_or(i64::MAX),
                    ])
                    .map_err(|err| {
                        DbError::sqlite(format!("写入 ue_enum_value {} {}", cls.path, ev.name), err)
                    })?;
                enum_val_rows += 1;
            }
        }

        let mut func_stmt = ctx
            .tx()
            .prepare_cached(
                "INSERT INTO ue_function (owner, name, flags, params, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| DbError::sqlite("准备写入 ue_function", err))?;

        for (func, _) in functions.values() {
            if !class_lookup.contains(&func.owner) && !package_lookup.contains(&func.owner) {
                ctx.record_issue(
                    "dangling_function_owner",
                    &func.path,
                    &format!("函数所属对象 {} 不在反射类或包中", func.owner),
                )?;
            }
            if let Some(sup) = &func.super_function
                && !function_lookup.contains(sup)
            {
                ctx.record_issue(
                    "dangling_super_function",
                    &func.path,
                    &format!("父函数 {sup} 不在反射数据中"),
                )?;
            }
            for param in &func.params {
                let param_key = format!("{}({})", func.path, param.name);
                check_property_refs(
                    ctx,
                    &param_key,
                    param,
                    &class_lookup,
                    &function_lookup,
                    &mut ref_classes,
                    &mut ref_funcs,
                )?;
            }
            if let Some(ret) = &func.return_value {
                let ret_key = format!("{}({})", func.path, ret.name);
                check_property_refs(
                    ctx,
                    &ret_key,
                    ret,
                    &class_lookup,
                    &function_lookup,
                    &mut ref_classes,
                    &mut ref_funcs,
                )?;
            }

            let param_summaries = summarize_function_params(func);
            let params_json = encode_doc("ue_function.params", &func.path, &param_summaries)?;
            let func_doc = encode_doc("ue_function", &func.path, func)?;
            func_stmt
                .execute(params![
                    func.owner,
                    func.name,
                    func.flags,
                    params_json,
                    func_doc,
                ])
                .map_err(|err| DbError::sqlite(format!("写入 ue_function {}", func.path), err))?;
            func_rows += 1;
        }
    }

    let total_rows = class_rows + prop_rows + func_rows + enum_val_rows;
    let total_conflicts = class_conflicts + function_conflicts;
    let note = format!(
        "files={}, classes={}, properties={}, functions={}, enum_values={}, added_classes={}, added_functions={}, conflicts={}",
        jmaps.len(),
        class_rows,
        prop_rows,
        func_rows,
        enum_val_rows,
        added_classes_later,
        added_functions_later,
        total_conflicts,
    );
    Ok(StageStats::ok_with_note(total_rows, note))
}

fn check_property_refs<'a>(
    ctx: &ImportCtx<'_>,
    subject: &str,
    prop: &'a UeProperty,
    class_lookup: &UePathLookup<()>,
    function_lookup: &UePathLookup<()>,
    ref_classes: &mut Vec<(&'static str, &'a str)>,
    ref_funcs: &mut Vec<&'a str>,
) -> Result<()> {
    ref_classes.clear();
    prop.collect_referenced_classes(ref_classes);
    for &(ref_kind, target) in ref_classes.iter() {
        if !class_lookup.contains(target) {
            ctx.record_issue(
                "dangling_property_type",
                subject,
                &format!("属性引用 {ref_kind}={target} 不在反射数据中"),
            )?;
        }
    }

    ref_funcs.clear();
    prop.collect_referenced_functions(ref_funcs);
    for &sig in ref_funcs.iter() {
        if !function_lookup.contains(sig) {
            ctx.record_issue(
                "dangling_signature_function",
                subject,
                &format!("委托签名函数 {sig} 不在反射数据中"),
            )?;
        }
    }
    Ok(())
}

fn describe_class_diff(prev: &UeClass, next: &UeClass, prev_src: &str, next_src: &str) -> String {
    let mut diffs = Vec::new();
    if prev.kind != next.kind {
        diffs.push(format!(
            "kind: {} -> {}",
            prev.kind.as_str(),
            next.kind.as_str()
        ));
    }
    if prev.package != next.package {
        diffs.push(format!("package: {} -> {}", prev.package, next.package));
    }
    if prev.class_type != next.class_type {
        diffs.push(format!(
            "class_type: {:?} -> {:?}",
            prev.class_type, next.class_type
        ));
    }
    if prev.super_class != next.super_class {
        diffs.push(format!(
            "super_class: {:?} -> {:?}",
            prev.super_class, next.super_class
        ));
    }
    if prev.size != next.size {
        diffs.push(format!("size: {} -> {}", prev.size, next.size));
    }
    if prev.min_alignment != next.min_alignment {
        diffs.push(format!(
            "min_alignment: {:?} -> {:?}",
            prev.min_alignment, next.min_alignment
        ));
    }
    if prev.flags != next.flags {
        diffs.push(format!("flags: \"{}\" -> \"{}\"", prev.flags, next.flags));
    }
    if prev.object_flags != next.object_flags {
        diffs.push(format!(
            "object_flags: \"{}\" -> \"{}\"",
            prev.object_flags, next.object_flags
        ));
    }
    if prev.interfaces != next.interfaces {
        diffs.push(format!(
            "interfaces: {} 项 -> {} 项",
            prev.interfaces.len(),
            next.interfaces.len()
        ));
    }
    if prev.cpp_type != next.cpp_type || prev.cpp_form != next.cpp_form {
        diffs.push(format!(
            "cpp: ({:?}, {:?}) -> ({:?}, {:?})",
            prev.cpp_type, prev.cpp_form, next.cpp_type, next.cpp_form
        ));
    }
    if prev.properties != next.properties {
        diffs.push(format!(
            "properties: {} 项 -> {} 项",
            prev.properties.len(),
            next.properties.len()
        ));
    }
    if prev.enum_values != next.enum_values {
        diffs.push(format!(
            "enum_values: {} 项 -> {} 项",
            prev.enum_values.len(),
            next.enum_values.len()
        ));
    }
    format!(
        "{prev_src} 与 {next_src} 定义不一致，以后者为准：{}",
        diffs.join("；")
    )
}

fn describe_function_diff(
    prev: &UeFunction,
    next: &UeFunction,
    prev_src: &str,
    next_src: &str,
) -> String {
    let mut diffs = Vec::new();
    if prev.class_type != next.class_type {
        diffs.push(format!(
            "class_type: {:?} -> {:?}",
            prev.class_type, next.class_type
        ));
    }
    if prev.super_function != next.super_function {
        diffs.push(format!(
            "super_function: {:?} -> {:?}",
            prev.super_function, next.super_function
        ));
    }
    if prev.properties_size != next.properties_size {
        diffs.push(format!(
            "properties_size: {} -> {}",
            prev.properties_size, next.properties_size
        ));
    }
    if prev.flags != next.flags {
        diffs.push(format!("flags: \"{}\" -> \"{}\"", prev.flags, next.flags));
    }
    if prev.params != next.params {
        diffs.push(format!(
            "params: {} 项 -> {} 项",
            prev.params.len(),
            next.params.len()
        ));
    }
    if prev.return_value != next.return_value {
        diffs.push("return_value 变更".to_owned());
    }
    format!(
        "{prev_src} 与 {next_src} 函数定义不一致，以后者为准：{}",
        diffs.join("；")
    )
}
