//! Per-domain read query modules.
//!
//! Each submodule adds `impl GameDb { ... }` methods for its own area without modifying `db.rs`.

pub mod assets;
pub mod battle;
pub mod datatable;
pub mod input_ui;
pub mod levels;
pub mod links;
pub mod reflection;
pub mod sequences;
pub mod snapshots;
pub mod text;
pub mod world;
