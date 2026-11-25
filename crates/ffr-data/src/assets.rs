//! Typed Rust representations for exported Unreal Engine assets beyond core DataTables.
//!
//! Each submodule corresponds to one asset domain that will be populated into `ffr-db`
//! (`asset` registry, `.ini` configs, `.jmap` reflection, Enhanced Input, Blueprints, UMG
//! widgets, exported levels, LevelSequences, and optional R1 runtime snapshots).

pub mod blueprint;
pub mod config;
pub mod input;
pub mod level;
pub mod reflection;
pub mod registry;
pub mod sequence;
pub mod snapshot;
pub mod widget;
