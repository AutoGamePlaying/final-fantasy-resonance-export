//! Default path resolution under `$FFR_DEPS/ffr/` for database inputs and output.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ffr_data::FFR_DEPS_ENV;

use crate::build::BuildInputs;
use crate::error::{DbError, Result};

/// Default paths derived from a single `FFR_DEPS` root directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbPaths {
    /// `<deps>/ffr/export`
    pub export_dir: PathBuf,
    /// `<deps>/ffr/export-full`
    pub full_dir: PathBuf,
    /// `<deps>/ffr/inventory`
    pub inventory_dir: PathBuf,
    /// `<deps>/ffr/export-full/maps`
    pub maps_dir: PathBuf,
    /// `<deps>/ffr/db/ffr.sqlite`
    pub db_file: PathBuf,
}

impl DbPaths {
    /// Compute all default input/output paths from a `deps` root directory.
    pub fn from_deps(deps_dir: impl AsRef<Path>) -> Self {
        let ffr = deps_dir.as_ref().join("ffr");
        let full_dir = ffr.join("export-full");
        let maps_dir = full_dir.join("maps");
        Self {
            export_dir: ffr.join("export"),
            full_dir,
            inventory_dir: ffr.join("inventory"),
            maps_dir,
            db_file: ffr.join("db").join("ffr.sqlite"),
        }
    }

    /// Derive default paths from the process `FFR_DEPS` environment variable, if set and non-empty.
    pub fn from_env_opt() -> Option<Self> {
        Self::from_env_value(std::env::var_os(FFR_DEPS_ENV))
    }

    /// Derive default paths from an optional `FFR_DEPS` environment variable value.
    pub fn from_env_value(value: Option<OsString>) -> Option<Self> {
        match value {
            Some(dir) if !dir.is_empty() => Some(Self::from_deps(PathBuf::from(dir))),
            _ => None,
        }
    }
}

/// Resolved input paths and output database path ready for [`crate::build::build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBuildPaths {
    pub inputs: BuildInputs,
    pub output: PathBuf,
}

impl ResolvedBuildPaths {
    /// Merge explicit CLI overrides with optional `FFR_DEPS` defaults.
    ///
    /// `--jmap` and `--snapshots` are never populated from defaults and must be given explicitly.
    pub fn resolve(
        explicit_inputs: BuildInputs,
        explicit_out: Option<PathBuf>,
        deps_env: Option<OsString>,
    ) -> Result<Self> {
        let defaults = DbPaths::from_env_value(deps_env);
        let output = match explicit_out {
            Some(path) => path,
            None => defaults
                .as_ref()
                .map(|paths| paths.db_file.clone())
                .ok_or(DbError::MissingEnvVar { var: FFR_DEPS_ENV })?,
        };
        let inputs = BuildInputs {
            export: explicit_inputs
                .export
                .or_else(|| defaults.as_ref().map(|p| p.export_dir.clone())),
            full: explicit_inputs
                .full
                .or_else(|| defaults.as_ref().map(|p| p.full_dir.clone())),
            inventory: explicit_inputs
                .inventory
                .or_else(|| defaults.as_ref().map(|p| p.inventory_dir.clone())),
            maps: explicit_inputs
                .maps
                .or_else(|| defaults.as_ref().map(|p| p.maps_dir.clone())),
            // `--jmap` and `--snapshots` must be provided explicitly.
            jmaps: explicit_inputs.jmaps,
            snapshots: explicit_inputs.snapshots,
        };
        Ok(Self { inputs, output })
    }
}

/// Compute `<deps_dir>/ffr/db/ffr.sqlite`.
pub fn default_db_path_from_deps(deps_dir: impl AsRef<Path>) -> PathBuf {
    DbPaths::from_deps(deps_dir).db_file
}

/// Resolve `$FFR_DEPS/ffr/db/ffr.sqlite` from the process environment.
pub fn default_db_path_from_env() -> Result<PathBuf> {
    DbPaths::from_env_opt()
        .map(|paths| paths.db_file)
        .ok_or(DbError::MissingEnvVar { var: FFR_DEPS_ENV })
}

/// Resolve a database file path from an explicit override or `$FFR_DEPS/ffr/db/ffr.sqlite`.
pub fn resolve_db_path(explicit: Option<&Path>) -> Result<PathBuf> {
    match explicit {
        Some(path) => Ok(path.to_path_buf()),
        None => default_db_path_from_env(),
    }
}
