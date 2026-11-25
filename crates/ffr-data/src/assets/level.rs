//! Exported level metadata, placed actors, classified level objects, and map transition graph
//! types for the `08_levels` database area.
//!
//! Parses `export-maps` outputs under `$FFR_DEPS/ffr/export-full/maps/`:
//! - Per-level `<level>.json` (`LevelExportDocument`: `level`, `actors`, `summary`).
//! - Aggregated `_graph.json` (`MapGraphDocument`: per-map nodes, exits, transition points, shops).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{DataError, Result};
use crate::flags::parse_flag_condition_expr;

/// 3D world-space coordinate or extent vector (`{ x, y, z }`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    /// Construct a 3D vector.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Return `true` if all three components are finite numbers.
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

/// 3D Euler rotation in degrees (`{ pitch, yaw, roll }`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rot3 {
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
}

/// One `ULevelStreaming` reference declared inside a persistent `*_PL` level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingLevel {
    pub name: String,
    pub world_asset: String,
    pub streaming_class: String,
}

/// Metadata header (`level`) of one exported `.umap` JSON file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    pub name: String,
    pub output_file: String,
    pub path: String,
    pub package_path: String,
    pub sublevel_type: String,
    #[serde(default)]
    pub persistent_level: Option<String>,
    #[serde(default)]
    pub persistent_levels: Vec<String>,
    #[serde(default)]
    pub map_id: Option<i64>,
    #[serde(default)]
    pub map_ids: Vec<i64>,
    #[serde(default)]
    pub map_row_key: Option<String>,
    #[serde(default)]
    pub map_type: Option<String>,
    #[serde(default)]
    pub streaming_levels: Vec<StreamingLevel>,
    #[serde(default)]
    pub actor_count: usize,
    #[serde(default)]
    pub game_actor_count: usize,
    #[serde(default)]
    pub engine_actor_count: usize,
}

impl Level {
    /// Return the unique database level stem derived from `output_file` (falling back to `name`).
    ///
    /// For 1,543 of 1,545 levels this equals `name`; for the 2 cross-folder basename collisions
    /// disambiguated by `export-maps`, it preserves the `<folder>__<name>` prefix.
    pub fn unique_name(&self) -> &str {
        let stem = self
            .output_file
            .strip_suffix(".json")
            .unwrap_or(&self.output_file);
        if stem.is_empty() { &self.name } else { stem }
    }

    /// Return all distinct map IDs associated with this level in ascending order.
    pub fn effective_map_ids(&self) -> Vec<i64> {
        let mut ids = self.map_ids.clone();
        if let Some(primary) = self.map_id
            && !ids.contains(&primary)
        {
            ids.push(primary);
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

/// One component/subobject attached to a game-class [`Actor`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSubobject {
    pub name: String,
    pub class: String,
    #[serde(default)]
    pub is_game_class: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Rot3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_extent: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sphere_radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capsule_half_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capsule_radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Map<String, Value>>,
}

/// One placed actor inside a level's `actors[]` array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub class: String,
    #[serde(default)]
    pub is_game_class: bool,
    #[serde(default)]
    pub location: Option<Vec3>,
    #[serde(default)]
    pub rotation: Option<Rot3>,
    #[serde(default)]
    pub scale: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subobjects: Option<Vec<ActorSubobject>>,
}

impl Actor {
    /// Extract the actor's box half-extents (`m_BoxComponentBoxExtent` or first subobject
    /// `boxExtent`), if present.
    pub fn box_extent(&self) -> Option<Vec3> {
        if let Some(props) = &self.properties
            && let Some(raw) = props
                .get("m_BoxComponentBoxExtent")
                .and_then(Value::as_object)
            && let Some(vec) = parse_vec3_object(raw)
        {
            return Some(vec);
        }
        if let Some(subs) = &self.subobjects {
            for sub in subs {
                if let Some(ext) = sub.box_extent {
                    return Some(ext);
                }
            }
        }
        None
    }

    /// Extract the actor's first subobject `sphereRadius`, if present.
    pub fn sphere_radius(&self) -> Option<f64> {
        self.subobjects
            .as_ref()?
            .iter()
            .find_map(|sub| sub.sphere_radius)
    }

    /// Extract the actor's first subobject `(capsule_radius, capsule_half_height)`, if present.
    pub fn capsule_dimensions(&self) -> Option<(f64, f64)> {
        self.subobjects.as_ref()?.iter().find_map(|sub| {
            match (sub.capsule_radius, sub.capsule_half_height) {
                (Some(r), Some(hh)) => Some((r, hh)),
                (Some(r), None) => Some((r, r)),
                (None, Some(hh)) => Some((0.0, hh)),
                (None, None) => None,
            }
        })
    }

    /// Compute the world-space axis-aligned bounding box `(min, max)` for R*Tree indexing.
    ///
    /// Uses `boxExtent` (with scale and rotation), `sphereRadius`, or capsule dimensions when
    /// available; falls back to a point AABB at `location`. Returns `None` if `location` is
    /// missing or contains non-finite coordinates.
    pub fn world_aabb(&self) -> Option<(Vec3, Vec3)> {
        let loc = self.location?;
        if !loc.is_finite() {
            return None;
        }
        let scale = self.scale.unwrap_or(Vec3::new(1.0, 1.0, 1.0));
        let sx = if scale.x.is_finite() {
            scale.x.abs()
        } else {
            1.0
        };
        let sy = if scale.y.is_finite() {
            scale.y.abs()
        } else {
            1.0
        };
        let sz = if scale.z.is_finite() {
            scale.z.abs()
        } else {
            1.0
        };

        let (hx, hy, hz) = if let Some(ext) = self.box_extent()
            && ext.is_finite()
        {
            let ex = ext.x.abs() * sx;
            let ey = ext.y.abs() * sy;
            let ez = ext.z.abs() * sz;
            rotated_box_half_extents(ex, ey, ez, self.rotation)
        } else if let Some(radius) = self.sphere_radius()
            && radius.is_finite()
        {
            let r = radius.abs() * sx.max(sy).max(sz);
            (r, r, r)
        } else if let Some((cap_r, cap_hh)) = self.capsule_dimensions()
            && cap_r.is_finite()
            && cap_hh.is_finite()
        {
            let r = cap_r.abs() * sx.max(sy);
            let hh = (cap_hh.abs() * sz).max(r);
            (r, r, hh)
        } else {
            (0.0, 0.0, 0.0)
        };

        if !hx.is_finite() || !hy.is_finite() || !hz.is_finite() {
            return None;
        }

        Some((
            Vec3::new(loc.x - hx, loc.y - hy, loc.z - hz),
            Vec3::new(loc.x + hx, loc.y + hy, loc.z + hz),
        ))
    }
}

/// Conditional talk/script event entry attached to a level object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEntry {
    #[serde(default)]
    pub condition: Option<String>,
    pub event_id: String,
}

/// Classified map unit placement (`summary.mapUnits[]`: NPC, object, vehicle, player, unit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapUnitPlacement {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    pub kind: String,
    #[serde(default)]
    pub map_unit_id: Option<i64>,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub interact_type: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub shop_id: Option<i64>,
    #[serde(default)]
    pub inn_id: Option<i64>,
    #[serde(default)]
    pub events: Vec<EventEntry>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Classified treasure box placement (`summary.treasureBoxes[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreasurePlacement {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub chest_id: Option<i64>,
    #[serde(default)]
    pub map_id: Option<i64>,
    #[serde(default)]
    pub item_id: Option<i64>,
    #[serde(default)]
    pub item_num: Option<i64>,
    #[serde(default)]
    pub money: Option<i64>,
    #[serde(default)]
    pub is_trap: Option<bool>,
    #[serde(default)]
    pub battle_group_id: Option<i64>,
    #[serde(default)]
    pub battle_finish_event_id: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Classified event/chat/interact trigger (`summary.eventTriggers[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventTrigger {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub interact_type: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub flag_condition: Option<String>,
    #[serde(default)]
    pub interact_event_id: Option<String>,
    #[serde(default)]
    pub events: Vec<EventEntry>,
    #[serde(default)]
    pub box_extent: Option<Vec3>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// One conditional destination branch of a [`Transition`] trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionTarget {
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub target_map_id: Option<i64>,
    #[serde(default)]
    pub target_point_id: Option<i64>,
    #[serde(default)]
    pub is_event_only: bool,
    #[serde(default)]
    pub events: Vec<EventEntry>,
    #[serde(default)]
    pub after_transition_events: Vec<EventEntry>,
}

/// Classified map transition trigger (`summary.transitions[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub is_same_map: bool,
    #[serde(default)]
    pub is_auto_transition: bool,
    #[serde(default)]
    pub is_event_only: bool,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub targets: Vec<TransitionTarget>,
    #[serde(default)]
    pub box_extent: Option<Vec3>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Classified map transition landing point (`summary.transitionPoints[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPoint {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    pub point_id: i64,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub point_name: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Classified save point placement (`summary.savePoints[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePointPlacement {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub save_point_type: Option<String>,
    #[serde(default)]
    pub is_teleport: bool,
    #[serde(default)]
    pub transition_map_id: Option<i64>,
    #[serde(default)]
    pub transition_point_id: Option<i64>,
    #[serde(default)]
    pub parent_map_id: Option<i64>,
    #[serde(default)]
    pub parent_unique_id: Option<i64>,
    #[serde(default)]
    pub target_map_id: Option<i64>,
    #[serde(default)]
    pub target_point_id: Option<i64>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Classified shop NPC placement (`summary.shops[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopPlacement {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub map_unit_id: Option<i64>,
    pub shop_id: i64,
    #[serde(default)]
    pub is_various_shop: bool,
    #[serde(default)]
    pub is_test: bool,
    #[serde(default)]
    pub interact_type: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Classified inn NPC placement (`summary.inns[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InnPlacement {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub map_unit_id: Option<i64>,
    pub inn_id: i64,
    #[serde(default)]
    pub interact_type: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Classified misc map gimmick placement (`summary.gimmicks[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gimmick {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    pub category: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub interact_type: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub flag_condition: Option<String>,
    #[serde(default)]
    pub events: Vec<EventEntry>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// One terrain-to-encounter-table mapping inside an [`EncounterZone`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncounterTerrain {
    #[serde(default)]
    pub terrain: Option<String>,
    #[serde(default)]
    pub datatable: Option<String>,
}

/// Classified enemy encounter zone (`summary.encounterZones[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncounterZone {
    pub actor_name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub class: String,
    pub kind: String,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub terrain_id: Option<i64>,
    #[serde(default)]
    pub battle_stage_id: Option<i64>,
    #[serde(default)]
    pub overwrite_ratio: Option<f64>,
    #[serde(default)]
    pub is_reset_distance: Option<bool>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub flag_condition: Option<String>,
    #[serde(default)]
    pub encount_datatable: Option<String>,
    #[serde(default)]
    pub terrain_datatables: Vec<EncounterTerrain>,
    #[serde(default)]
    pub box_extent: Option<Vec3>,
    #[serde(default)]
    pub sphere_radius: Option<f64>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Aggregated classified object lists inside one exported level JSON (`summary`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelSummary {
    #[serde(default)]
    pub map_units: Vec<MapUnitPlacement>,
    #[serde(default)]
    pub treasure_boxes: Vec<TreasurePlacement>,
    #[serde(default)]
    pub event_triggers: Vec<EventTrigger>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub transition_points: Vec<TransitionPoint>,
    #[serde(default)]
    pub save_points: Vec<SavePointPlacement>,
    #[serde(default)]
    pub shops: Vec<ShopPlacement>,
    #[serde(default)]
    pub inns: Vec<InnPlacement>,
    #[serde(default)]
    pub gimmicks: Vec<Gimmick>,
    #[serde(default)]
    pub encounter_zones: Vec<EncounterZone>,
}

/// Full parsed document of a per-level `<level>.json` export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelExportDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub level: Level,
    #[serde(default)]
    pub actors: Vec<Actor>,
    #[serde(default)]
    pub summary: LevelSummary,
}

/// Discriminant for the unified `level_object` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelObjectKind {
    MapUnit,
    TreasureBox,
    EventTrigger,
    Transition,
    TransitionPoint,
    SavePoint,
    Shop,
    Inn,
    Gimmick,
    EncounterZone,
}

impl LevelObjectKind {
    /// Canonical string stored in `level_object.kind`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MapUnit => "map_unit",
            Self::TreasureBox => "treasure_box",
            Self::EventTrigger => "event_trigger",
            Self::Transition => "transition",
            Self::TransitionPoint => "transition_point",
            Self::SavePoint => "save_point",
            Self::Shop => "shop",
            Self::Inn => "inn",
            Self::Gimmick => "gimmick",
            Self::EncounterZone => "encounter_zone",
        }
    }

    /// Parse a stored `level_object.kind` string.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "map_unit" => Some(Self::MapUnit),
            "treasure_box" => Some(Self::TreasureBox),
            "event_trigger" => Some(Self::EventTrigger),
            "transition" => Some(Self::Transition),
            "transition_point" => Some(Self::TransitionPoint),
            "save_point" => Some(Self::SavePoint),
            "shop" => Some(Self::Shop),
            "inn" => Some(Self::Inn),
            "gimmick" => Some(Self::Gimmick),
            "encounter_zone" => Some(Self::EncounterZone),
            _ => None,
        }
    }
}

/// Typed payload stored inside [`LevelObject`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum LevelObjectData {
    MapUnit(MapUnitPlacement),
    TreasureBox(TreasurePlacement),
    EventTrigger(EventTrigger),
    Transition(Transition),
    TransitionPoint(TransitionPoint),
    SavePoint(SavePointPlacement),
    Shop(ShopPlacement),
    Inn(InnPlacement),
    Gimmick(Gimmick),
    EncounterZone(EncounterZone),
}

/// Unified classified level object row stored in `level_object.doc`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelObject {
    pub kind: LevelObjectKind,
    #[serde(default)]
    pub map_id: Option<i64>,
    pub level_name: String,
    pub actor_name: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
    pub data: LevelObjectData,
}

/// Normalized kind of a directed edge in `map_edge` / [`MapExit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapEdgeKind {
    CrossMap,
    SameMap,
    SavePointTeleport,
}

impl MapEdgeKind {
    /// Canonical string stored in `map_edge.kind`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CrossMap => "cross_map",
            Self::SameMap => "same_map",
            Self::SavePointTeleport => "savepoint_teleport",
        }
    }

    /// Parse a stored `map_edge.kind` string.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "cross_map" => Some(Self::CrossMap),
            "same_map" => Some(Self::SameMap),
            "savepoint_teleport" => Some(Self::SavePointTeleport),
            _ => None,
        }
    }

    /// Classify a raw `_graph.json` exit entry into a [`MapEdgeKind`].
    pub fn classify(raw_kind: &str, from_map_id: i64, exit: &GraphExit) -> Self {
        if raw_kind == "savepoint_teleport" {
            Self::SavePointTeleport
        } else if exit.is_same_map || exit.target_map_id == Some(from_map_id) {
            Self::SameMap
        } else {
            Self::CrossMap
        }
    }
}

/// Directed exit edge in the map graph, optionally enriched with its resolved target landing
/// coordinates from `transition_point`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapExit {
    pub from_map_id: i64,
    pub kind: MapEdgeKind,
    pub source_level: String,
    pub actor_name: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub target_map_id: Option<i64>,
    #[serde(default)]
    pub target_point_id: Option<i64>,
    /// Resolved landing coordinate in `target_map_id` (`transition_point`), when available.
    #[serde(default)]
    pub target_location: Option<Vec3>,
    /// Resolved facing direction at the landing point, when available.
    #[serde(default)]
    pub target_direction: Option<String>,
    #[serde(default)]
    pub is_same_map: bool,
    #[serde(default)]
    pub is_auto_transition: bool,
    #[serde(default)]
    pub is_event_only: bool,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

impl MapExit {
    /// Convert a raw [`GraphExit`] belonging to `from_map_id` into a [`MapExit`].
    pub fn from_graph_exit(from_map_id: i64, exit: &GraphExit) -> Self {
        let kind = MapEdgeKind::classify(&exit.kind, from_map_id, exit);
        Self {
            from_map_id,
            kind,
            source_level: exit.source_level.clone(),
            actor_name: exit.actor_name.clone(),
            unique_id: exit.unique_id,
            target_map_id: exit.target_map_id,
            target_point_id: exit.target_point_id,
            target_location: None,
            target_direction: None,
            is_same_map: exit.is_same_map || exit.target_map_id == Some(from_map_id),
            is_auto_transition: exit.is_auto_transition,
            is_event_only: exit.is_event_only,
            condition: exit.condition.clone(),
            location: exit.location,
        }
    }
}

// ---- `_graph.json` DTOs ----

/// Raw exit entry inside `_graph.json`'s `maps[<id>].exits[]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphExit {
    pub kind: String,
    pub source_level: String,
    pub actor_name: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub target_map_id: Option<i64>,
    #[serde(default)]
    pub target_point_id: Option<i64>,
    #[serde(default)]
    pub is_same_map: bool,
    #[serde(default)]
    pub is_auto_transition: bool,
    #[serde(default)]
    pub is_event_only: bool,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Sublevel reference inside `_graph.json`'s `maps[<id>].sublevels[]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSublevelRef {
    pub name: String,
    #[serde(rename = "type")]
    pub sublevel_type: String,
    pub output_file: String,
}

/// Per-map summary counters inside `_graph.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMapCounts {
    #[serde(default)]
    pub npcs: usize,
    #[serde(default)]
    pub map_units: usize,
    #[serde(default)]
    pub treasure_boxes: usize,
    #[serde(default)]
    pub shops: usize,
    #[serde(default)]
    pub inns: usize,
    #[serde(default)]
    pub save_points: usize,
    #[serde(default)]
    pub event_triggers: usize,
    #[serde(default)]
    pub gimmicks: usize,
    #[serde(default)]
    pub encounter_zones: usize,
    #[serde(default)]
    pub transitions: usize,
    #[serde(default)]
    pub transition_points: usize,
}

/// Transition point entry inside `_graph.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphTransitionPoint {
    pub source_level: String,
    pub actor_name: String,
    pub point_id: i64,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub point_name: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Shop entry inside `_graph.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphShop {
    pub source_level: String,
    pub actor_name: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub map_unit_id: Option<i64>,
    pub shop_id: i64,
    #[serde(default)]
    pub is_various_shop: bool,
    #[serde(default)]
    pub is_test: bool,
    #[serde(default)]
    pub interact_type: Option<String>,
    #[serde(default)]
    pub visible_condition: Option<String>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Save point entry inside `_graph.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSavePoint {
    pub source_level: String,
    pub actor_name: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub save_point_type: Option<String>,
    #[serde(default)]
    pub is_teleport: bool,
    #[serde(default)]
    pub target_map_id: Option<i64>,
    #[serde(default)]
    pub target_point_id: Option<i64>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// Treasure box entry inside `_graph.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphTreasureBox {
    pub source_level: String,
    pub actor_name: String,
    #[serde(default)]
    pub unique_id: Option<i64>,
    #[serde(default)]
    pub chest_id: Option<i64>,
    #[serde(default)]
    pub item_id: Option<i64>,
    #[serde(default)]
    pub money: Option<i64>,
    #[serde(default)]
    pub is_trap: Option<bool>,
    #[serde(default)]
    pub location: Option<Vec3>,
}

/// One map node inside `_graph.json`'s `maps` dictionary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMapNode {
    pub map_id: i64,
    #[serde(default)]
    pub map_row_key: Option<String>,
    #[serde(default)]
    pub map_type: Option<String>,
    #[serde(default)]
    pub persistent_level: Option<String>,
    #[serde(default)]
    pub sublevels: Vec<GraphSublevelRef>,
    #[serde(default)]
    pub counts: GraphMapCounts,
    #[serde(default)]
    pub exits: Vec<GraphExit>,
    #[serde(default)]
    pub transition_points: Vec<GraphTransitionPoint>,
    #[serde(default)]
    pub shops: Vec<GraphShop>,
    #[serde(default)]
    pub save_points: Vec<GraphSavePoint>,
    #[serde(default)]
    pub treasure_boxes: Vec<GraphTreasureBox>,
}

/// One unmapped level entry inside `_graph.json`'s `unmappedLevels[]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphUnmappedLevel {
    pub name: String,
    #[serde(rename = "type")]
    pub sublevel_type: String,
    pub path: String,
    pub output_file: String,
}

/// Top-level `_graph.json` aggregate statistics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStats {
    #[serde(default)]
    pub total_levels: usize,
    #[serde(default)]
    pub exported_levels: usize,
    #[serde(default)]
    pub failed_levels: usize,
    #[serde(default)]
    pub sublevel_type_counts: BTreeMap<String, usize>,
    #[serde(default)]
    pub mapped_map_count: usize,
    #[serde(default)]
    pub unmapped_level_count: usize,
    #[serde(default)]
    pub total_exits: usize,
    #[serde(default)]
    pub cross_map_exits: usize,
    #[serde(default)]
    pub same_map_exits: usize,
    #[serde(default)]
    pub save_point_teleport_exits: usize,
    #[serde(default)]
    pub total_transition_points: usize,
    #[serde(default)]
    pub total_npcs: usize,
    #[serde(default)]
    pub total_map_units: usize,
    #[serde(default)]
    pub total_treasure_boxes: usize,
    #[serde(default)]
    pub total_save_points: usize,
    #[serde(default)]
    pub total_shops: usize,
    #[serde(default)]
    pub unique_shop_ids: usize,
    #[serde(default)]
    pub maps_with_shops: usize,
    #[serde(default)]
    pub total_inns: usize,
    #[serde(default)]
    pub total_event_triggers: usize,
    #[serde(default)]
    pub total_gimmicks: usize,
    #[serde(default)]
    pub total_encounter_zones: usize,
}

/// Top-level `_graph.json` document generated by `export-maps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapGraphDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub stats: GraphStats,
    #[serde(default)]
    pub maps: BTreeMap<String, GraphMapNode>,
    #[serde(default)]
    pub shops_by_map: BTreeMap<String, Vec<i64>>,
    #[serde(default)]
    pub shop_to_maps: BTreeMap<String, Vec<i64>>,
    #[serde(default)]
    pub unmapped_levels: Vec<GraphUnmappedLevel>,
}

impl MapGraphDocument {
    /// Collect all directed [`MapExit`] edges across every map node in deterministic map ID order.
    pub fn all_exits(&self) -> Vec<MapExit> {
        let mut nodes: Vec<&GraphMapNode> = self.maps.values().collect();
        nodes.sort_by_key(|n| n.map_id);
        let mut out = Vec::new();
        for node in nodes {
            for exit in &node.exits {
                out.push(MapExit::from_graph_exit(node.map_id, exit));
            }
        }
        out
    }
}

// ---- Parsing helpers ----

/// Parse one exported level JSON string into a [`LevelExportDocument`].
pub fn parse_level_document(raw: &str, source_label: &str) -> Result<LevelExportDocument> {
    let doc: LevelExportDocument = serde_json::from_str(raw).map_err(|err| {
        DataError::invalid_schema(source_label, "$", format!("解析关卡 JSON 失败：{err}"))
    })?;
    if doc.level.name.trim().is_empty() {
        return Err(DataError::invalid_schema(
            source_label,
            "level.name",
            "关卡名不能为空",
        ));
    }
    if doc.level.path.trim().is_empty() {
        return Err(DataError::invalid_schema(
            source_label,
            "level.path",
            "关卡路径不能为空",
        ));
    }
    Ok(doc)
}

/// Read and parse one exported level JSON file from disk.
pub fn load_level_document(path: impl AsRef<Path>) -> Result<LevelExportDocument> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|source| DataError::Io {
        kind: "关卡 JSON",
        path: path.to_path_buf(),
        source,
    })?;
    let label = path.display().to_string();
    serde_json::from_str::<LevelExportDocument>(&raw)
        .map_err(|source| DataError::Json {
            kind: "关卡",
            path: path.to_path_buf(),
            source,
        })
        .and_then(|doc| {
            if doc.level.name.trim().is_empty() {
                return Err(DataError::invalid_schema(
                    &label,
                    "level.name",
                    "关卡名不能为空",
                ));
            }
            if doc.level.path.trim().is_empty() {
                return Err(DataError::invalid_schema(
                    &label,
                    "level.path",
                    "关卡路径不能为空",
                ));
            }
            Ok(doc)
        })
}

/// Parse a `_graph.json` string into a [`MapGraphDocument`].
pub fn parse_map_graph(raw: &str, source_label: &str) -> Result<MapGraphDocument> {
    serde_json::from_str(raw).map_err(|err| {
        DataError::invalid_schema(source_label, "$", format!("解析地图图谱 JSON 失败：{err}"))
    })
}

/// Read and parse `_graph.json` from disk.
pub fn load_map_graph(path: impl AsRef<Path>) -> Result<MapGraphDocument> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|source| DataError::Io {
        kind: "地图图谱 JSON",
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| DataError::Json {
        kind: "地图图谱",
        path: path.to_path_buf(),
        source,
    })
}

/// Validate a level/event condition expression string (such as `"!{flag:1170}"`,
/// `"{progress:0}>=1020&{progress:0}<=1040"`, `"!100&!102"`, or `"C01_001F"`).
///
/// Reuses [`crate::flags::parse_flag_condition_expr`] for `CDT_FlagCondition` numeric syntax and
/// runs a recursive-descent validator for general game condition expressions. Returns `Ok(())` if
/// empty or syntactically valid, or `Err(reason)` describing the syntax error.
pub fn validate_condition_expr(expr: &str) -> std::result::Result<(), String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if parse_flag_condition_expr(trimmed).is_some() {
        return Ok(());
    }
    let tokens = tokenize_condition(trimmed)?;
    let mut parser = ConditionParser {
        tokens: &tokens,
        pos: 0,
    };
    parser.parse_or()?;
    if let Some(extra) = parser.peek() {
        return Err(format!("存在多余词法单元：{extra:?}"));
    }
    Ok(())
}

// ---- Internal helpers ----

const fn default_schema_version() -> u32 {
    1
}

fn parse_vec3_object(obj: &Map<String, Value>) -> Option<Vec3> {
    let x = obj
        .get("X")
        .or_else(|| obj.get("x"))
        .and_then(Value::as_f64)?;
    let y = obj
        .get("Y")
        .or_else(|| obj.get("y"))
        .and_then(Value::as_f64)?;
    let z = obj
        .get("Z")
        .or_else(|| obj.get("z"))
        .and_then(Value::as_f64)?;
    Some(Vec3::new(x, y, z))
}

fn rotated_box_half_extents(hx: f64, hy: f64, hz: f64, rot: Option<Rot3>) -> (f64, f64, f64) {
    let Some(r) = rot else {
        return (hx, hy, hz);
    };
    if !r.pitch.is_finite() || !r.yaw.is_finite() || !r.roll.is_finite() {
        return (hx, hy, hz);
    }
    if r.pitch == 0.0 && r.yaw == 0.0 && r.roll == 0.0 {
        return (hx, hy, hz);
    }

    // UE FRotator convention: Yaw (Z), Pitch (Y), Roll (X) in degrees.
    let (sp, cp) = r.pitch.to_radians().sin_cos();
    let (sy, cy) = r.yaw.to_radians().sin_cos();
    let (sr, cr) = r.roll.to_radians().sin_cos();

    // Basis vectors of UE FRotationMatrix (rows = rotated local X, Y, Z axes):
    let r00 = cp * cy;
    let r01 = cp * sy;
    let r02 = sp;

    let r10 = sr * sp * cy - cr * sy;
    let r11 = sr * sp * sy + cr * cy;
    let r12 = -sr * cp;

    let r20 = -(cr * sp * cy + sr * sy);
    let r21 = cy * sr - cr * sp * sy;
    let r22 = cr * cp;

    let ax = r00.abs() * hx + r10.abs() * hy + r20.abs() * hz;
    let ay = r01.abs() * hx + r11.abs() * hy + r21.abs() * hz;
    let az = r02.abs() * hx + r12.abs() * hy + r22.abs() * hz;
    (ax, ay, az)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CondToken<'a> {
    LParen,
    RParen,
    Not,
    And,
    Or,
    Cmp(&'a str),
    BraceVar(&'a str),
    Number(&'a str),
    Ident(&'a str),
}

fn tokenize_condition(input: &str) -> std::result::Result<Vec<CondToken<'_>>, String> {
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut tokens = Vec::new();

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        match b {
            b'(' => {
                tokens.push(CondToken::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(CondToken::RParen);
                i += 1;
            }
            b'!' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    tokens.push(CondToken::Cmp("!="));
                    i += 2;
                } else {
                    tokens.push(CondToken::Not);
                    i += 1;
                }
            }
            b'&' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                    i += 2;
                } else {
                    i += 1;
                }
                tokens.push(CondToken::And);
            }
            b'|' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    i += 2;
                } else {
                    i += 1;
                }
                tokens.push(CondToken::Or);
            }
            b'=' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    tokens.push(CondToken::Cmp("=="));
                    i += 2;
                } else {
                    return Err(format!("位置 {i} 出现非法运算符：{:?}", &input[i..]));
                }
            }
            b'>' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    tokens.push(CondToken::Cmp(">="));
                    i += 2;
                } else {
                    tokens.push(CondToken::Cmp(">"));
                    i += 1;
                }
            }
            b'<' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    tokens.push(CondToken::Cmp("<="));
                    i += 2;
                } else {
                    tokens.push(CondToken::Cmp("<"));
                    i += 1;
                }
            }
            b'{' => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'}' {
                    if bytes[end] == b'{' {
                        return Err(format!("位置 {end} 出现嵌套花括号"));
                    }
                    end += 1;
                }
                if end >= bytes.len() {
                    return Err(format!("位置 {i} 的花括号未闭合"));
                }
                let inner = input[start..end].trim();
                if inner.is_empty() {
                    return Err(format!("位置 {i} 出现空变量 {{}}"));
                }
                tokens.push(CondToken::BraceVar(inner));
                i = end + 1;
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                if b == b'-' {
                    i += 1;
                    if i >= bytes.len() || !bytes[i].is_ascii_digit() {
                        return Err(format!("位置 {start} 出现孤立负号"));
                    }
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                tokens.push(CondToken::Number(&input[start..i]));
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                tokens.push(CondToken::Ident(&input[start..i]));
            }
            _ => {
                return Err(format!("位置 {i} 出现无法识别的字符：{:?}", &input[i..]));
            }
        }
    }

    if tokens.is_empty() {
        return Err("条件表达式为空".to_owned());
    }
    Ok(tokens)
}

struct ConditionParser<'a> {
    tokens: &'a [CondToken<'a>],
    pos: usize,
}

impl<'a> ConditionParser<'a> {
    fn peek(&self) -> Option<&'a CondToken<'a>> {
        self.tokens.get(self.pos)
    }

    fn parse_or(&mut self) -> std::result::Result<(), String> {
        self.parse_and()?;
        while matches!(self.peek(), Some(CondToken::Or)) {
            self.pos += 1;
            self.parse_and()?;
        }
        Ok(())
    }

    fn parse_and(&mut self) -> std::result::Result<(), String> {
        self.parse_unary()?;
        while matches!(self.peek(), Some(CondToken::And)) {
            self.pos += 1;
            self.parse_unary()?;
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> std::result::Result<(), String> {
        while matches!(self.peek(), Some(CondToken::Not)) {
            self.pos += 1;
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> std::result::Result<(), String> {
        match self.peek() {
            Some(CondToken::LParen) => {
                self.pos += 1;
                self.parse_or()?;
                if matches!(self.peek(), Some(CondToken::RParen)) {
                    self.pos += 1;
                    Ok(())
                } else {
                    Err("括号未闭合".to_owned())
                }
            }
            Some(CondToken::BraceVar(_)) => {
                self.pos += 1;
                if matches!(self.peek(), Some(CondToken::Cmp(_))) {
                    self.pos += 1;
                    match self.peek() {
                        Some(CondToken::Number(_)) | Some(CondToken::BraceVar(_)) => {
                            self.pos += 1;
                            Ok(())
                        }
                        other => Err(format!("比较运算符右侧缺少数值或变量：{other:?}")),
                    }
                } else {
                    Ok(())
                }
            }
            Some(CondToken::Ident(_)) | Some(CondToken::Number(_)) => {
                self.pos += 1;
                Ok(())
            }
            other => Err(format!("期望条件项，但遇到：{other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_validator_accepts_valid_and_rejects_malformed() {
        for ok in [
            "",
            "!100&!102",
            "1300",
            "!{flag:1170}",
            "{progress:0}>=1020&{progress:0}<=1040",
            "!({progress:0}>=5108 | {progress:0}<5090)",
            "({equip:1001, Vision}|{equip:1002, Vision})&{progress:0}==1090",
            "{progress:0}>=5060 & ({progress:54}>=20&{progress:54}<=30&({flag:955}|{gilgamesh}) | {progress:54}==30)",
            "!{ride}&!{flag:113}",
            "C01_001F",
            "!C03_305_01",
        ] {
            assert!(
                validate_condition_expr(ok).is_ok(),
                "expected valid condition: {ok}"
            );
        }

        for bad in [
            "{progress:0}>=4170 & {progress:0}=<4250",
            "{progress:22}==0&{progress:0}==1205&(flag:10000)",
            "{flag:100",
            "{}",
            "{progress:0}>=",
            "({flag:1}",
            "{flag:1} &",
        ] {
            assert!(
                validate_condition_expr(bad).is_err(),
                "expected invalid condition: {bad}"
            );
        }
    }

    #[test]
    fn actor_world_aabb_handles_point_box_rotated_box_and_sphere() {
        let point_actor = Actor {
            name: "Pt".to_owned(),
            label: None,
            class: "/Script/Engine.Actor".to_owned(),
            is_game_class: false,
            location: Some(Vec3::new(10.0, 20.0, 30.0)),
            rotation: None,
            scale: None,
            asset_refs: None,
            properties: None,
            subobjects: None,
        };
        assert_eq!(
            point_actor.world_aabb(),
            Some((Vec3::new(10.0, 20.0, 30.0), Vec3::new(10.0, 20.0, 30.0)))
        );

        let rotated_box = Actor {
            name: "Box".to_owned(),
            label: None,
            class: "/Game/BP_Trigger".to_owned(),
            is_game_class: true,
            location: Some(Vec3::new(100.0, 0.0, 0.0)),
            rotation: Some(Rot3 {
                pitch: 0.0,
                yaw: 90.0,
                roll: 0.0,
            }),
            scale: Some(Vec3::new(2.0, 1.0, 1.0)),
            asset_refs: None,
            properties: None,
            subobjects: Some(vec![ActorSubobject {
                name: "Collision".to_owned(),
                class: "/Script/Engine.BoxComponent".to_owned(),
                is_game_class: false,
                location: None,
                rotation: None,
                scale: None,
                box_extent: Some(Vec3::new(50.0, 10.0, 20.0)),
                sphere_radius: None,
                capsule_half_height: None,
                capsule_radius: None,
                asset_refs: None,
                properties: None,
            }]),
        };
        let (min, max) = rotated_box.world_aabb().expect("box aabb");
        // hx = 50*2 = 100 along local X -> yaw 90 deg rotates to world Y; hy = 10 along local Y -> world X.
        assert!((min.x - 90.0).abs() < 1e-6);
        assert!((max.x - 110.0).abs() < 1e-6);
        assert!((min.y - -100.0).abs() < 1e-6);
        assert!((max.y - 100.0).abs() < 1e-6);
        assert!((min.z - -20.0).abs() < 1e-6);
        assert!((max.z - 20.0).abs() < 1e-6);
    }
}
