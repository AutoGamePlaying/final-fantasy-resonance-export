//! Blueprint class defaults, component templates, and DataAsset types exported from the game
//! package (`07_input_ui`).
//!
//! Parses CUE4Parse export arrays for:
//! - `BlueprintGeneratedClass` / `WidgetBlueprintGeneratedClass` (`Default__*` CDO overrides,
//!   `RF_DefaultSubObject` components, `SimpleConstructionScript` `SCS_Node` templates, and
//!   `InheritableComponentHandler` overrides) into [`Blueprint`];
//! - `DataAsset` / `PrimaryDataAsset` / `CPP_DA_*` subclasses into [`DataAsset`].

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{DataError, Result};

/// High-level Blueprint classification stored in `blueprint.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintKind {
    /// Actor-derived Blueprint (has a `SimpleConstructionScript`, `RootComponent`, or Actor base).
    Actor,
    /// UMG Widget Blueprint (`WidgetBlueprintGeneratedClass`).
    Widget,
    /// Non-Actor, non-Widget Blueprint (e.g. AI controller/object, CameraShake, UI style, ViewModel).
    Other,
}

impl BlueprintKind {
    /// Canonical lowercase string stored in `blueprint.kind`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Actor => "actor",
            Self::Widget => "widget",
            Self::Other => "other",
        }
    }

    /// Parse a stored `blueprint.kind` string.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "actor" => Some(Self::Actor),
            "widget" => Some(Self::Widget),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// Where a [`BlueprintComponent`] came from inside the exported Blueprint package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentOrigin {
    /// C++ default subobject owned by `Default__<Class>` (`RF_DefaultSubObject`).
    DefaultSubObject,
    /// Blueprint-added component defined by a `SimpleConstructionScript` `SCS_Node`.
    SimpleConstructionScript,
    /// Inherited component overridden via `InheritableComponentHandler`.
    InheritableComponent,
}

/// One component template or default subobject attached to a [`Blueprint`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintComponent {
    /// Component variable/object name (e.g. `"CharMoveComp"`, `"DefaultSceneRoot"`).
    pub name: String,
    /// Unreal component class name (e.g. `"CharacterMovementComponent"`, `"BoxComponent"`).
    pub class: String,
    /// How this component was declared or overridden in the Blueprint export.
    pub origin: ComponentOrigin,
    /// Optional parent component name (from SCS hierarchy or `AttachParent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_component: Option<String>,
    /// Overridden properties recorded on this component in the Blueprint export.
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

/// Exported Unreal Engine Blueprint (`BlueprintGeneratedClass` or `WidgetBlueprintGeneratedClass`).
///
/// Only properties overridden in the Blueprint appear in [`Self::defaults`] and
/// [`Self::components`]; untouched C++ defaults are absent from CUE4Parse exports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blueprint {
    /// Canonical Unreal package path (e.g. `"/Game/BP/Map/Unit/BP_MapUnit_Player"`).
    pub path: String,
    /// Generated Blueprint class name (e.g. `"BP_MapUnit_Player_C"`).
    pub class_name: String,
    /// Immediate parent class name (Blueprint `*_C` or C++ class such as `"CPP_MapUnit_Player"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_class: Option<String>,
    /// Immediate parent package/module path (e.g. `"/Script/mucha"` or `"/Game/..."`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_path: Option<String>,
    /// High-level category (`actor`, `widget`, or `other`).
    pub kind: BlueprintKind,
    /// Class Default Object name if present (e.g. `"Default__BP_MapUnit_Player_C"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdo_name: Option<String>,
    /// Overridden Class Default Object properties.
    #[serde(default)]
    pub defaults: BTreeMap<String, Value>,
    /// Default subobjects, SCS component templates, and inheritable component overrides.
    #[serde(default)]
    pub components: Vec<BlueprintComponent>,
}

impl Blueprint {
    /// Look up an overridden property on this Blueprint alone (without walking ancestors).
    ///
    /// Supports:
    /// - Direct CDO properties (`"m_MaxWalkingSpeed"`);
    /// - Nested CDO struct/object paths (`"m_SsPlayerOffset.Z"`);
    /// - Component properties by component name (`"CharMoveComp.MaxAcceleration"`) or nested
    ///   component structs (`"CineCamera.LensSettings.DiaphragmBladeCount"`);
    /// - Component properties via a CDO reference field pointing to a component (e.g.
    ///   `"CharacterMovement.MaxAcceleration"` when CDO `CharacterMovement` references
    ///   `CharMoveComp`).
    pub fn lookup_property(&self, property_path: &str) -> Option<&Value> {
        let segments: Vec<&str> = property_path
            .split('.')
            .filter(|seg| !seg.is_empty())
            .collect();
        if segments.is_empty() {
            return None;
        }

        // 1. Check component by exact component name when a sub-property path is provided.
        if segments.len() >= 2
            && let Some(comp) = self.components.iter().find(|c| c.name == segments[0])
            && let Some(val) = lookup_in_map(&comp.properties, &segments[1..])
        {
            return Some(val);
        }

        // 2. Check CDO defaults directly (including nested struct/object fields).
        if let Some(val) = lookup_in_map(&self.defaults, &segments) {
            return Some(val);
        }

        // 3. Follow a CDO component reference property (e.g. `CharacterMovement` -> `CharMoveComp`).
        if segments.len() >= 2
            && let Some(ref_val) = self.defaults.get(segments[0])
            && let Some(target_name) = extract_subobject_leaf_name(ref_val)
            && let Some(comp) = self.components.iter().find(|c| c.name == target_name)
            && let Some(val) = lookup_in_map(&comp.properties, &segments[1..])
        {
            return Some(val);
        }

        None
    }
}

/// Exported Unreal Engine `DataAsset` / `PrimaryDataAsset` (and game-specific `CPP_DA_*` subclasses).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataAsset {
    /// Canonical Unreal package path (e.g. `"/Game/Field/Airship/Data/DA_Airship"`).
    pub path: String,
    /// Asset object name (e.g. `"DA_Airship"`).
    pub name: String,
    /// Concrete Unreal DataAsset class name (e.g. `"CPP_DA_Airship"`).
    pub class: String,
    /// Exported properties on the DataAsset object.
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

/// Parse a [`Blueprint`] from a CUE4Parse export JSON array.
///
/// Returns `Ok(None)` if the document does not represent a standalone Blueprint (for example,
/// `LevelSequence` packages that embed a sequence director class, or non-Blueprint assets).
pub fn parse_blueprint(document: &Value, source_label: &str) -> Result<Option<Blueprint>> {
    let objects = export_objects(document, source_label)?;
    if objects.is_empty() {
        return Ok(None);
    }

    // LevelSequence assets often include a generated director BlueprintGeneratedClass;
    // those belong to the sequences domain, not standalone Blueprints.
    if objects
        .iter()
        .any(|obj| obj_str(obj, "Type") == Some("LevelSequence"))
    {
        return Ok(None);
    }

    let Some((cls_idx, cls_obj)) = objects.iter().enumerate().find(|(_, obj)| {
        matches!(
            obj_str(obj, "Type"),
            Some("BlueprintGeneratedClass" | "WidgetBlueprintGeneratedClass")
        )
    }) else {
        return Ok(None);
    };

    let cls_type = obj_str(cls_obj, "Type").unwrap_or("BlueprintGeneratedClass");
    let class_name = require_non_empty_str(cls_obj, "Name", source_label, "Name")?.to_owned();
    let path = resolve_package_path(cls_obj, &objects, source_label)?;

    let super_ref = cls_obj
        .get("Super")
        .filter(|v| v.is_object())
        .or_else(|| cls_obj.get("SuperStruct").filter(|v| v.is_object()));
    let parent_class = super_ref
        .and_then(|v| v.get("ObjectName"))
        .and_then(Value::as_str)
        .and_then(extract_quoted_inner)
        .map(ToOwned::to_owned);
    let parent_path = super_ref
        .and_then(|v| v.get("ObjectPath"))
        .and_then(Value::as_str)
        .map(strip_object_path_suffix);

    let path_index = build_ue_path_index(&objects);

    // Locate Class Default Object (CDO).
    let cdo_idx = cls_obj
        .get("ClassDefaultObject")
        .and_then(|r| resolve_local_ref(r, &objects, &path_index))
        .or_else(|| {
            objects.iter().position(|obj| {
                obj_str(obj, "Name").is_some_and(|n| n.starts_with("Default__"))
                    || obj_str(obj, "Flags").is_some_and(|f| f.contains("RF_ClassDefaultObject"))
            })
        });

    let (cdo_name, defaults) = match cdo_idx {
        Some(idx) => {
            let cdo = objects[idx];
            (
                obj_str(cdo, "Name").map(ToOwned::to_owned),
                extract_properties_map(cdo),
            )
        }
        None => (None, BTreeMap::new()),
    };

    let has_scs = objects
        .iter()
        .any(|obj| obj_str(obj, "Type") == Some("SimpleConstructionScript"));

    let kind = if cls_type == "WidgetBlueprintGeneratedClass" {
        BlueprintKind::Widget
    } else if has_scs
        || defaults.contains_key("RootComponent")
        || parent_class
            .as_deref()
            .is_some_and(is_known_actor_base_class)
    {
        BlueprintKind::Actor
    } else {
        BlueprintKind::Other
    };

    let components = extract_blueprint_components(&objects, cls_idx, cdo_idx, &path_index);

    Ok(Some(Blueprint {
        path,
        class_name,
        parent_class,
        parent_path,
        kind,
        cdo_name,
        defaults,
        components,
    }))
}

/// Load and parse a [`Blueprint`] from a JSON file on disk.
pub fn load_blueprint(path: impl AsRef<Path>) -> Result<Option<Blueprint>> {
    let path = path.as_ref();
    let doc = read_export_json(path, "蓝图")?;
    let label = path.display().to_string();
    parse_blueprint(&doc, &label)
}

/// Parse a [`DataAsset`] from a CUE4Parse export JSON array.
///
/// Returns `Ok(None)` when the file is not a DataAsset (e.g. DataTables, StringTables,
/// Blueprints, WidgetBlueprints, InputActions, InputMappingContexts, LevelSequences, Curves, or
/// user-defined schemas).
pub fn parse_data_asset(document: &Value, source_label: &str) -> Result<Option<DataAsset>> {
    let objects = export_objects(document, source_label)?;
    if objects.is_empty() {
        return Ok(None);
    }

    // Reject packages belonging to non-DataAsset domains.
    for obj in &objects {
        if let Some(t) = obj_str(obj, "Type")
            && matches!(
                t,
                "LevelSequence"
                    | "DataTable"
                    | "CompositeDataTable"
                    | "StringTable"
                    | "InputAction"
                    | "InputMappingContext"
                    | "BlueprintGeneratedClass"
                    | "WidgetBlueprintGeneratedClass"
                    | "UserDefinedEnum"
                    | "UserDefinedStruct"
            )
        {
            return Ok(None);
        }
    }

    // Primary export is the top-level object (no `Outer`), falling back to index 0.
    let primary = objects
        .iter()
        .copied()
        .find(|obj| !obj.get("Outer").is_some_and(Value::is_object))
        .unwrap_or(objects[0]);

    let Some(class) = obj_str(primary, "Type") else {
        return Ok(None);
    };
    let name = obj_str(primary, "Name").unwrap_or("");
    if class.starts_with("Curve") || !is_data_asset_class(class, name) {
        return Ok(None);
    }

    let name = require_non_empty_str(primary, "Name", source_label, "Name")?.to_owned();
    let path = resolve_package_path(primary, &objects, source_label)?;
    let properties = extract_properties_map(primary);

    Ok(Some(DataAsset {
        path,
        name,
        class: class.to_owned(),
        properties,
    }))
}

/// Load and parse a [`DataAsset`] from a JSON file on disk.
pub fn load_data_asset(path: impl AsRef<Path>) -> Result<Option<DataAsset>> {
    let path = path.as_ref();
    let doc = read_export_json(path, "数据资产")?;
    let label = path.display().to_string();
    parse_data_asset(&doc, &label)
}

/// Return `true` if `class` (or asset `name`) identifies a UE `DataAsset` / `PrimaryDataAsset`
/// subclass in the exported game package.
pub fn is_data_asset_class(class: &str, name: &str) -> bool {
    matches!(
        class,
        "DataAsset" | "PrimaryDataAsset" | "PrimaryAssetLabel" | "CPP_DM_EventShortcutData"
    ) || class.starts_with("CPP_DA_")
        || class.starts_with("CPP_DMI_DA_")
        || class.starts_with("DA_")
        || class.ends_with("DataAsset")
        || name.starts_with("DA_")
}

fn is_known_actor_base_class(class: &str) -> bool {
    matches!(
        class,
        "Actor"
            | "Pawn"
            | "Character"
            | "PlayerController"
            | "AIController"
            | "GameModeBase"
            | "GameMode"
            | "GameStateBase"
            | "GameState"
            | "HUD"
            | "Info"
            | "Volume"
            | "TriggerBase"
            | "CameraActor"
            | "CineCameraActor"
            | "Light"
            | "DirectionalLight"
            | "PointLight"
            | "SpotLight"
            | "SkeletalMeshActor"
            | "StaticMeshActor"
            | "SceneCapture2D"
    )
}

fn extract_blueprint_components(
    objects: &[&Map<String, Value>],
    cls_idx: usize,
    cdo_idx: Option<usize>,
    path_index: &HashMap<String, usize>,
) -> Vec<BlueprintComponent> {
    let mut by_name: BTreeMap<String, BlueprintComponent> = BTreeMap::new();

    // 1. Default subobjects attached to the CDO (`Outer = Default__X_C` or `RF_DefaultSubObject`).
    let cdo_name = cdo_idx.and_then(|idx| obj_str(objects[idx], "Name"));
    for (idx, obj) in objects.iter().enumerate() {
        if idx == cls_idx || Some(idx) == cdo_idx {
            continue;
        }
        let is_dso_flag =
            obj_str(obj, "Flags").is_some_and(|flags| flags.contains("RF_DefaultSubObject"));
        let outer_is_cdo = cdo_name.is_some_and(|cdo| {
            obj.get("Outer")
                .and_then(|o| o.get("ObjectName"))
                .and_then(Value::as_str)
                .and_then(extract_quoted_inner)
                == Some(cdo)
        });
        if !is_dso_flag && !outer_is_cdo {
            continue;
        }
        let Some(name) = obj_str(obj, "Name") else {
            continue;
        };
        let Some(class) = obj_str(obj, "Type") else {
            continue;
        };
        let properties = extract_properties_map(obj);
        let parent_component = properties
            .get("AttachParent")
            .and_then(extract_subobject_leaf_name)
            .map(ToOwned::to_owned);
        by_name.insert(
            name.to_owned(),
            BlueprintComponent {
                name: name.to_owned(),
                class: class.to_owned(),
                origin: ComponentOrigin::DefaultSubObject,
                parent_component,
                properties,
            },
        );
    }

    // 2. SimpleConstructionScript (`SCS_Node` -> `ComponentTemplate`).
    let mut scs_parent_by_node_idx: HashMap<usize, String> = HashMap::new();
    for obj in objects {
        if obj_str(obj, "Type") != Some("SCS_Node") {
            continue;
        }
        let Some(props) = obj.get("Properties").and_then(Value::as_object) else {
            continue;
        };
        let parent_var = scs_node_var_name(props, objects, path_index);
        if let Some(parent_name) = parent_var
            && let Some(children) = props.get("ChildNodes").and_then(Value::as_array)
        {
            for child_ref in children {
                if let Some(child_idx) = resolve_local_ref(child_ref, objects, path_index) {
                    scs_parent_by_node_idx.insert(child_idx, parent_name.clone());
                }
            }
        }
    }

    for (idx, obj) in objects.iter().enumerate() {
        if obj_str(obj, "Type") != Some("SCS_Node") {
            continue;
        }
        let Some(props) = obj.get("Properties").and_then(Value::as_object) else {
            continue;
        };
        let Some(name) = scs_node_var_name(props, objects, path_index) else {
            continue;
        };
        let template = props
            .get("ComponentTemplate")
            .and_then(|r| resolve_local_ref(r, objects, path_index))
            .map(|tidx| objects[tidx]);
        let class = props
            .get("ComponentClass")
            .and_then(|r| r.get("ObjectName"))
            .and_then(Value::as_str)
            .and_then(extract_quoted_inner)
            .or_else(|| template.and_then(|t| obj_str(t, "Type")))
            .unwrap_or("ActorComponent")
            .to_owned();
        let properties = template.map(extract_properties_map).unwrap_or_default();
        let parent_component = scs_parent_by_node_idx.get(&idx).cloned().or_else(|| {
            props
                .get("ParentComponentOrVariableName")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && *s != "None")
                .map(ToOwned::to_owned)
        });
        by_name.entry(name.clone()).or_insert(BlueprintComponent {
            name,
            class,
            origin: ComponentOrigin::SimpleConstructionScript,
            parent_component,
            properties,
        });
    }

    // 3. InheritableComponentHandler (`Records[]` overriding parent SCS components).
    for obj in objects {
        if obj_str(obj, "Type") != Some("InheritableComponentHandler") {
            continue;
        }
        let Some(records) = obj
            .get("Properties")
            .and_then(Value::as_object)
            .and_then(|p| p.get("Records"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for record in records {
            let Some(rec_obj) = record.as_object() else {
                continue;
            };
            let template = rec_obj
                .get("ComponentTemplate")
                .and_then(|r| resolve_local_ref(r, objects, path_index))
                .map(|tidx| objects[tidx]);
            let name = rec_obj
                .get("ComponentKey")
                .and_then(Value::as_object)
                .and_then(|k| k.get("SCSVariableName"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    template
                        .and_then(|t| obj_str(t, "Name"))
                        .map(strip_gen_variable_suffix)
                });
            let Some(name) = name else {
                continue;
            };
            let class = rec_obj
                .get("ComponentClass")
                .and_then(|r| r.get("ObjectName"))
                .and_then(Value::as_str)
                .and_then(extract_quoted_inner)
                .or_else(|| template.and_then(|t| obj_str(t, "Type")))
                .unwrap_or("ActorComponent")
                .to_owned();
            let properties = template.map(extract_properties_map).unwrap_or_default();
            let parent_component = properties
                .get("AttachParent")
                .and_then(extract_subobject_leaf_name)
                .map(ToOwned::to_owned);
            by_name.entry(name.clone()).or_insert(BlueprintComponent {
                name,
                class,
                origin: ComponentOrigin::InheritableComponent,
                parent_component,
                properties,
            });
        }
    }

    by_name.into_values().collect()
}

fn scs_node_var_name(
    props: &Map<String, Value>,
    objects: &[&Map<String, Value>],
    path_index: &HashMap<String, usize>,
) -> Option<String> {
    if let Some(var) = props
        .get("InternalVariableName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(var.to_owned());
    }
    let tidx = props
        .get("ComponentTemplate")
        .and_then(|r| resolve_local_ref(r, objects, path_index))?;
    obj_str(objects[tidx], "Name").map(strip_gen_variable_suffix)
}

fn strip_gen_variable_suffix(name: &str) -> String {
    name.strip_suffix("_GEN_VARIABLE")
        .unwrap_or(name)
        .to_owned()
}

fn lookup_in_map<'a>(map: &'a BTreeMap<String, Value>, segments: &[&str]) -> Option<&'a Value> {
    let first = *segments.first()?;
    let mut current = map.get(first)?;
    for &seg in &segments[1..] {
        match current {
            Value::Object(obj) => {
                current = obj.get(seg)?;
            }
            Value::Array(arr) => {
                let idx: usize = seg.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn extract_subobject_leaf_name(value: &Value) -> Option<&str> {
    let inner = value
        .as_object()?
        .get("ObjectName")?
        .as_str()
        .and_then(extract_quoted_inner)?;
    inner
        .rsplit_once('.')
        .or_else(|| inner.rsplit_once(':'))
        .map(|(_, leaf)| leaf)
        .filter(|leaf| !leaf.is_empty())
}

// ---------------------------------------------------------------------------
// Shared internal CUE4Parse JSON export helpers (used by input.rs & widget.rs)
// ---------------------------------------------------------------------------

pub(crate) fn read_export_json(path: &Path, kind: &'static str) -> Result<Value> {
    let raw = fs::read_to_string(path).map_err(|source| DataError::Io {
        kind,
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| DataError::Json {
        kind,
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn export_objects<'a>(
    document: &'a Value,
    source_label: &str,
) -> Result<Vec<&'a Map<String, Value>>> {
    let Value::Array(items) = document else {
        return Err(DataError::invalid_schema(
            source_label,
            "<root>",
            "导出 JSON 顶层不是数组",
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(DataError::invalid_schema(
                source_label,
                format!("[{idx}]"),
                "导出数组元素不是 JSON 对象",
            ));
        };
        out.push(obj);
    }
    Ok(out)
}

pub(crate) fn obj_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

pub(crate) fn require_non_empty_str<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
    source_label: &str,
    field_label: &str,
) -> Result<&'a str> {
    match obj_str(obj, key).map(str::trim) {
        Some(val) if !val.is_empty() => Ok(val),
        _ => Err(DataError::invalid_schema(
            source_label,
            field_label,
            "缺少非空字符串字段",
        )),
    }
}

pub(crate) fn extract_properties_map(obj: &Map<String, Value>) -> BTreeMap<String, Value> {
    let Some(props) = obj.get("Properties").and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    props.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Extract the text inside single quotes from a CUE4Parse `ObjectName`, e.g.
/// `"InputAction'IA_Battle_AutoBattle'"` -> `Some("IA_Battle_AutoBattle")`.
pub(crate) fn extract_quoted_inner(object_name: &str) -> Option<&str> {
    let start = object_name.find('\'')?;
    let end = object_name.rfind('\'')?;
    if end > start + 1 {
        Some(&object_name[start + 1..end])
    } else {
        None
    }
}

/// Extract the class prefix before single quotes from a CUE4Parse `ObjectName`, e.g.
/// `"InputModifierNegate'IMC_Foo:InputModifierNegate_0'"` -> `Some("InputModifierNegate")`.
pub(crate) fn extract_object_type_prefix(object_name: &str) -> Option<&str> {
    let start = object_name.find('\'')?;
    let prefix = object_name[..start].trim();
    if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    }
}

/// Strip an export-index (`.0`, `.12`) or object-name (`.Foo_C`) suffix from a UE `ObjectPath`,
/// yielding the canonical package path (e.g. `"/Game/BP/Map/Unit/BP_MapUnit_Player"`).
pub(crate) fn strip_object_path_suffix(object_path: &str) -> String {
    let trimmed = object_path.trim();
    if let Some(last_slash) = trimmed.rfind('/') {
        let tail = &trimmed[last_slash + 1..];
        if let Some(dot) = tail.find('.') {
            return trimmed[..last_slash + 1 + dot].to_owned();
        }
    } else if let Some(dot) = trimmed.find('.') {
        return trimmed[..dot].to_owned();
    }
    trimmed.to_owned()
}

/// Resolve a package path from an exported object's `Package` field, falling back to any other
/// object's `Package` or `Class` path in the same export file.
pub(crate) fn resolve_package_path(
    primary: &Map<String, Value>,
    objects: &[&Map<String, Value>],
    source_label: &str,
) -> Result<String> {
    if let Some(pkg) = obj_str(primary, "Package").map(str::trim)
        && !pkg.is_empty()
    {
        return Ok(pkg.to_owned());
    }
    for obj in objects {
        if let Some(pkg) = obj_str(obj, "Package").map(str::trim)
            && !pkg.is_empty()
        {
            return Ok(pkg.to_owned());
        }
    }
    if let Some(cls) = obj_str(primary, "Class").and_then(extract_quoted_inner)
        && cls.starts_with('/')
    {
        return Ok(strip_object_path_suffix(cls));
    }
    Err(DataError::invalid_schema(
        source_label,
        "Package",
        "缺少有效的 Package 路径字段",
    ))
}

/// Build an index from canonical UE subobject path (the quoted portion of `ObjectName`) to the
/// element's position in `objects`.
///
/// Exported Blueprint/Widget files strip `Function` and `EdGraph*` objects, so the numeric `.N`
/// export index in `ObjectPath` no longer matches array positions; canonical UE paths remain exact.
pub(crate) fn build_ue_path_index(objects: &[&Map<String, Value>]) -> HashMap<String, usize> {
    let mut index = HashMap::with_capacity(objects.len());
    for (idx, obj) in objects.iter().enumerate() {
        let Some(name) = obj_str(obj, "Name") else {
            continue;
        };
        let full_path = if let Some(outer_inner) = obj
            .get("Outer")
            .and_then(|o| o.get("ObjectName"))
            .and_then(Value::as_str)
            .and_then(extract_quoted_inner)
        {
            let sep = if outer_inner.contains(':') { '.' } else { ':' };
            format!("{outer_inner}{sep}{name}")
        } else {
            name.to_owned()
        };
        index.entry(full_path).or_insert(idx);
    }
    index
}

/// Resolve an intra-file `{ "ObjectName", "ObjectPath" }` reference to an index into `objects`.
pub(crate) fn resolve_local_ref(
    reference: &Value,
    objects: &[&Map<String, Value>],
    path_index: &HashMap<String, usize>,
) -> Option<usize> {
    let ref_obj = reference.as_object()?;
    if let Some(inner) = ref_obj
        .get("ObjectName")
        .and_then(Value::as_str)
        .and_then(extract_quoted_inner)
        && let Some(&idx) = path_index.get(inner)
    {
        return Some(idx);
    }

    // Fallback for unfiltered/synthetic exports where `.N` matches the array index directly.
    let expected_leaf = ref_obj
        .get("ObjectName")
        .and_then(Value::as_str)
        .and_then(extract_quoted_inner)
        .map(|inner| {
            inner
                .rsplit_once('.')
                .or_else(|| inner.rsplit_once(':'))
                .map_or(inner, |(_, leaf)| leaf)
        });
    if let Some(op) = ref_obj.get("ObjectPath").and_then(Value::as_str)
        && let Some((_, tail)) = op.rsplit_once('.')
        && let Ok(idx) = tail.parse::<usize>()
        && let Some(candidate) = objects.get(idx)
    {
        let name_matches = match expected_leaf {
            Some(leaf) => obj_str(candidate, "Name") == Some(leaf),
            None => true,
        };
        if name_matches {
            return Some(idx);
        }
    }

    // Final unique-name fallback if only one object shares the leaf name.
    if let Some(leaf) = expected_leaf {
        let mut matches = objects
            .iter()
            .enumerate()
            .filter(|(_, obj)| obj_str(obj, "Name") == Some(leaf));
        if let Some((first_idx, _)) = matches.next()
            && matches.next().is_none()
        {
            return Some(first_idx);
        }
    }
    None
}
