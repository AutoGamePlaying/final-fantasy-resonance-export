//! Path resolution inside an unpacked CUE4Parse export directory (`<deps>/ffr/export/`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{DataError, Result};

/// Environment variable pointing to the runtime `.deps` directory.
pub const FFR_DEPS_ENV: &str = "FFR_DEPS";

const GAME_MOUNT_PREFIX: &str = "/Game/";

/// Compute `<deps_dir>/ffr/export`.
pub fn export_dir_from_deps(deps_dir: impl AsRef<Path>) -> PathBuf {
    deps_dir.as_ref().join("ffr").join("export")
}

/// Resolve `<FFR_DEPS>/ffr/export` from the process environment.
pub fn export_dir_from_env() -> Result<PathBuf> {
    export_dir_from_env_value(std::env::var_os(FFR_DEPS_ENV))
}

/// Resolve `<deps>/ffr/export` from an optional `FFR_DEPS` value.
pub fn export_dir_from_env_value(value: Option<OsString>) -> Result<PathBuf> {
    match value {
        Some(dir) if !dir.is_empty() => Ok(export_dir_from_deps(PathBuf::from(dir))),
        _ => Err(DataError::MissingEnvVar { var: FFR_DEPS_ENV }),
    }
}

/// Resolve the export root from an explicit path, falling back to `$FFR_DEPS/ffr/export`.
pub fn resolve_export_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    match explicit {
        Some(path) => Ok(path.to_path_buf()),
        None => export_dir_from_env(),
    }
}

/// `<export_root>/FFRS/Content/Datatable`: exported DataTable, StringTable, and DataAsset JSONs.
pub fn datatable_dir(export_root: impl AsRef<Path>) -> PathBuf {
    export_root
        .as_ref()
        .join("FFRS")
        .join("Content")
        .join("Datatable")
}

/// `<export_root>/Localization`: per-language locres JSON exports.
pub fn localization_dir(export_root: impl AsRef<Path>) -> PathBuf {
    export_root.as_ref().join("Localization")
}

/// `<export_root>/Localization/<language>.json`: exported `Game.locres` JSON for one language.
pub fn localization_file(export_root: impl AsRef<Path>, language: &str) -> PathBuf {
    localization_dir(export_root).join(format!("{language}.json"))
}

/// Validate a BCP-47-like language tag (`[A-Za-z0-9-]+`) before using it in a file path.
pub fn is_safe_language_tag(language: &str) -> bool {
    !language.is_empty()
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Map a UE object or package path (e.g.
/// `"/Game/Datatable/Strings/ChapterTitle/ST_ChapterTitle.ST_ChapterTitle"` or
/// `"/Game/Datatable/Scenario/DT_ChapterData"`) to its JSON path under `export_root`.
///
/// Returns `None` if the path does not sit cleanly inside `/Game/`.
pub fn ue_object_path_to_export_json(
    ue_path: &str,
    export_root: impl AsRef<Path>,
) -> Option<PathBuf> {
    let without_mount = ue_path.strip_prefix(GAME_MOUNT_PREFIX)?;
    if without_mount.is_empty() {
        return None;
    }

    let segments: Vec<&str> = without_mount.split('/').collect();
    let last = *segments.last()?;
    if last.is_empty() {
        return None;
    }

    let asset_base = match last.find('.') {
        Some(dot) => &last[..dot],
        None => last,
    };
    if asset_base.is_empty() {
        return None;
    }

    let mut path = export_root.as_ref().join("FFRS").join("Content");
    for segment in &segments[..segments.len() - 1] {
        if !is_safe_path_segment(segment) {
            return None;
        }
        path.push(segment);
    }
    if !is_safe_path_segment(asset_base) {
        return None;
    }
    path.push(format!("{asset_base}.json"));
    Some(path)
}

/// Map a relative path inside `FFRS/Content/Datatable/` (with or without `.json`, such as
/// `"Scenario/DT_ChapterData"` or `"Map/CDT_MapData_Prod.json"`) to its export JSON path.
///
/// Returns `None` if the path is empty, absolute, or attempts directory traversal.
pub fn datatable_relative_path_to_json(
    relative_path: &str,
    export_root: impl AsRef<Path>,
) -> Option<PathBuf> {
    if relative_path.is_empty() || relative_path.starts_with('/') || relative_path.contains('\\') {
        return None;
    }
    let trimmed = relative_path.strip_suffix(".json").unwrap_or(relative_path);
    if trimmed.is_empty() {
        return None;
    }

    let segments: Vec<&str> = trimmed.split('/').collect();
    let file_stem = *segments.last()?;
    if !is_safe_path_segment(file_stem) {
        return None;
    }

    let mut path = datatable_dir(export_root);
    for segment in &segments[..segments.len() - 1] {
        if !is_safe_path_segment(segment) {
            return None;
        }
        path.push(segment);
    }
    path.push(format!("{file_stem}.json"));
    Some(path)
}

fn is_safe_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('\\')
        && !segment.contains(':')
}
