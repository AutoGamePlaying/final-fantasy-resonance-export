//! Enhanced Input asset types exported from the game package (`07_input_ui`).
//!
//! Parses CUE4Parse export arrays for:
//! - `InputAction` (`IA_*`) into [`InputAction`];
//! - `InputMappingContext` (`IMC_*`) into [`InputMappingContext`] + [`InputKeyMapping`];
//! - Input DataAssets (`CPP_DA_MUCHA_INPUT`, e.g. `DA_Input_MapUnit`) into [`InputSet`].

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::assets::blueprint::{
    build_ue_path_index, export_objects, extract_object_type_prefix, extract_properties_map,
    extract_quoted_inner, obj_str, read_export_json, require_non_empty_str, resolve_local_ref,
    resolve_package_path, strip_object_path_suffix,
};
use crate::error::{DataError, Result};

/// Enhanced Input modifier attached to an [`InputAction`] or [`InputKeyMapping`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputModifier {
    /// Subobject name (e.g. `"InputModifierNegate_0"`).
    pub name: String,
    /// Modifier class (e.g. `"InputModifierNegate"`, `"InputModifierSwizzleAxis"`).
    pub class: String,
    /// Overridden properties on the modifier subobject (e.g. `bX`, `bY`, `Order`, `LowerThreshold`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
}

/// Enhanced Input trigger attached to an [`InputAction`] or [`InputKeyMapping`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputTrigger {
    /// Subobject name (e.g. `"InputTriggerPressed_0"`, `"CPP_EIT_AcceleHold_0"`).
    pub name: String,
    /// Trigger class (e.g. `"InputTriggerPressed"`, `"InputTriggerHold"`).
    pub class: String,
    /// Overridden properties on the trigger subobject (e.g. `HoldTimeThreshold`, `ActuationThreshold`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
}

/// Exported Enhanced Input `InputAction` (`IA_*`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputAction {
    /// Action object name (e.g. `"IA_Battle_AutoBattle"`).
    pub id: String,
    /// Canonical Unreal package path (e.g. `"/Game/Input/Battle/Action/IA_Battle_AutoBattle"`).
    pub path: String,
    /// Normalized `EInputActionValueType` (`"Boolean"`, `"Axis1D"`, `"Axis2D"`, `"Axis3D"`).
    pub value_type: String,
    /// Whether `bConsumeInput` was explicitly overridden on the asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consume_input: Option<bool>,
    /// Whether `bTriggerWhenPaused` was explicitly overridden on the asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_when_paused: Option<bool>,
    /// Localization key of `ActionDescription` if present (never substitutes Japanese as Chinese).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_key: Option<String>,
    /// Action-level modifiers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<InputModifier>,
    /// Action-level triggers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<InputTrigger>,
    /// Full exported `Properties` map for forward compatibility.
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

/// One physical key binding inside an [`InputMappingContext`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputKeyMapping {
    /// Referenced [`InputAction::id`] (e.g. `"IA_Battle_AutoBattle"`).
    pub action_id: String,
    /// Canonical package path of the referenced `InputAction`, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_path: Option<String>,
    /// Unreal key name (`Key.KeyName`, e.g. `"T"`, `"Tab"`, `"Gamepad_FaceButton_Bottom"`).
    pub key: String,
    /// Mapping-level modifiers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<InputModifier>,
    /// Mapping-level triggers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<InputTrigger>,
    /// `SettingBehavior` enum string if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting_behavior: Option<String>,
    /// 0-based order inside the context's `Mappings` array.
    pub ord: usize,
}

/// Exported Enhanced Input `InputMappingContext` (`IMC_*`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputMappingContext {
    /// Context object name (e.g. `"IMC_Battle_Default_KBM"`).
    pub id: String,
    /// Canonical Unreal package path (e.g. `"/Game/Input/Battle/Mapping/IMC_Battle_Default_KBM"`).
    pub path: String,
    /// Ordered key bindings defined in this context.
    #[serde(default)]
    pub mappings: Vec<InputKeyMapping>,
}

/// Device-to-context entry inside an [`InputSet`] (`pInputMappingContextList`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSetContext {
    /// Canonical device identifier (e.g. `"KeyboardAndMouse"`, `"Gamepad"`).
    pub device: String,
    /// Referenced [`InputMappingContext::id`] (e.g. `"IMC_Map_Unit_KBM"`).
    pub context_id: String,
    /// Canonical package path of the referenced context, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_path: Option<String>,
}

/// Semantic-name-to-action entry inside an [`InputSet`] (`InputActionList`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSetAction {
    /// Logical/semantic action key (e.g. `"UnitInteract"`, `"SystemMenu_Open"`).
    pub semantic: String,
    /// Referenced [`InputAction::id`] (e.g. `"IA_Map_UnitInteract"`).
    pub action_id: String,
    /// Canonical package path of the referenced `InputAction`, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_path: Option<String>,
}

/// Exported input DataAsset (`CPP_DA_MUCHA_INPUT`, e.g. `DA_Input_MapUnit`).
///
/// Connects physical input devices (`KeyboardAndMouse`, `Gamepad`) to [`InputMappingContext`]s and
/// semantic gameplay action names to [`InputAction`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSet {
    /// DataAsset object name (e.g. `"DA_Input_MapUnit"`).
    pub asset: String,
    /// Canonical Unreal package path (e.g. `"/Game/Input/DataAsset/Map/DA_Input_MapUnit"`).
    pub path: String,
    /// Unreal DataAsset class (e.g. `"CPP_DA_MUCHA_INPUT"`).
    pub class: String,
    /// Device -> `InputMappingContext` list (`pInputMappingContextList`).
    #[serde(default)]
    pub contexts: Vec<InputSetContext>,
    /// Semantic name -> `InputAction` list (`InputActionList`).
    #[serde(default)]
    pub actions: Vec<InputSetAction>,
}

/// Normalize a device string from CUE4Parse or a caller into canonical form.
///
/// Examples:
/// - `"eMuchaInputDeviceType::KeyboardAndMouse"`, `"kbm"`, `"keyboard"` -> `"KeyboardAndMouse"`
/// - `"eMuchaInputDeviceType::Gamepad"`, `"pad"`, `"controller"` -> `"Gamepad"`
/// - `"CustomEnum::Foo"` -> `"Foo"`
pub fn normalize_input_device(raw: &str) -> String {
    let trimmed = raw.trim();
    let stripped = trimmed
        .rsplit_once("::")
        .map_or(trimmed, |(_, suffix)| suffix);
    match stripped.to_ascii_lowercase().as_str() {
        "keyboardandmouse" | "kbm" | "keyboard" | "kb" | "mouse" => "KeyboardAndMouse".to_owned(),
        "gamepad" | "pad" | "controller" => "Gamepad".to_owned(),
        _ => stripped.to_owned(),
    }
}

/// Parse an [`InputAction`] from a CUE4Parse export JSON array.
///
/// Returns `Ok(None)` if the export array does not contain an `InputAction` object.
pub fn parse_input_action(document: &Value, source_label: &str) -> Result<Option<InputAction>> {
    let objects = export_objects(document, source_label)?;
    let Some(action_obj) = objects
        .iter()
        .copied()
        .find(|obj| obj_str(obj, "Type") == Some("InputAction"))
    else {
        return Ok(None);
    };

    let id = require_non_empty_str(action_obj, "Name", source_label, "Name")?.to_owned();
    let path = resolve_package_path(action_obj, &objects, source_label)?;
    let properties = extract_properties_map(action_obj);
    let path_index = build_ue_path_index(&objects);

    let value_type = match properties.get("ValueType").and_then(Value::as_str) {
        Some(raw) => raw
            .rsplit_once("::")
            .map_or(raw, |(_, suffix)| suffix)
            .to_owned(),
        None => "Boolean".to_owned(),
    };
    let consume_input = properties.get("bConsumeInput").and_then(Value::as_bool);
    let trigger_when_paused = properties
        .get("bTriggerWhenPaused")
        .and_then(Value::as_bool);
    let description_key = properties
        .get("ActionDescription")
        .and_then(Value::as_object)
        .and_then(|ft| ft.get("Key"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let modifiers = extract_modifiers(properties.get("Modifiers"), &objects, &path_index);
    let triggers = extract_triggers(properties.get("Triggers"), &objects, &path_index);

    Ok(Some(InputAction {
        id,
        path,
        value_type,
        consume_input,
        trigger_when_paused,
        description_key,
        modifiers,
        triggers,
        properties,
    }))
}

/// Load and parse an [`InputAction`] from a JSON file on disk.
pub fn load_input_action(path: impl AsRef<Path>) -> Result<Option<InputAction>> {
    let path = path.as_ref();
    let doc = read_export_json(path, "输入动作")?;
    let label = path.display().to_string();
    parse_input_action(&doc, &label)
}

/// Parse an [`InputMappingContext`] from a CUE4Parse export JSON array.
///
/// Supports both UE 5.6 `Properties.Mappings[]` and `Properties.DefaultKeyMappings.Mappings[]`.
/// Returns `Ok(None)` if the export array does not contain an `InputMappingContext` object.
pub fn parse_input_mapping_context(
    document: &Value,
    source_label: &str,
) -> Result<Option<InputMappingContext>> {
    let objects = export_objects(document, source_label)?;
    let Some(ctx_obj) = objects
        .iter()
        .copied()
        .find(|obj| obj_str(obj, "Type") == Some("InputMappingContext"))
    else {
        return Ok(None);
    };

    let id = require_non_empty_str(ctx_obj, "Name", source_label, "Name")?.to_owned();
    let path = resolve_package_path(ctx_obj, &objects, source_label)?;
    let path_index = build_ue_path_index(&objects);

    let props = ctx_obj.get("Properties").and_then(Value::as_object);
    let mappings_val = props.and_then(|p| {
        p.get("Mappings").or_else(|| {
            p.get("DefaultKeyMappings")
                .and_then(Value::as_object)
                .and_then(|dkm| dkm.get("Mappings"))
        })
    });

    let mut mappings = Vec::new();
    if let Some(raw_mappings) = mappings_val {
        let Some(entries) = raw_mappings.as_array() else {
            return Err(DataError::invalid_schema(
                source_label,
                "Properties.Mappings",
                "Mappings 不是 JSON 数组",
            ));
        };
        for (ord, entry) in entries.iter().enumerate() {
            let Some(entry_obj) = entry.as_object() else {
                return Err(DataError::invalid_schema(
                    source_label,
                    format!("Properties.Mappings[{ord}]"),
                    "映射条目不是 JSON 对象",
                ));
            };
            let action_ref = entry_obj
                .get("Action")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.Mappings[{ord}].Action"),
                        "映射条目缺少 Action 对象引用",
                    )
                })?;
            let action_id = action_ref
                .get("ObjectName")
                .and_then(Value::as_str)
                .and_then(extract_quoted_inner)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.Mappings[{ord}].Action.ObjectName"),
                        "无法从 Action.ObjectName 提取动作名",
                    )
                })?
                .to_owned();
            let action_path = action_ref
                .get("ObjectPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(strip_object_path_suffix);

            let key = extract_key_name(entry_obj.get("Key")).ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("Properties.Mappings[{ord}].Key"),
                    "映射条目缺少有效的 Key.KeyName",
                )
            })?;

            let modifiers = extract_modifiers(entry_obj.get("Modifiers"), &objects, &path_index);
            let triggers = extract_triggers(entry_obj.get("Triggers"), &objects, &path_index);
            let setting_behavior = entry_obj
                .get("SettingBehavior")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            mappings.push(InputKeyMapping {
                action_id,
                action_path,
                key,
                modifiers,
                triggers,
                setting_behavior,
                ord,
            });
        }
    }

    Ok(Some(InputMappingContext { id, path, mappings }))
}

/// Load and parse an [`InputMappingContext`] from a JSON file on disk.
pub fn load_input_mapping_context(path: impl AsRef<Path>) -> Result<Option<InputMappingContext>> {
    let path = path.as_ref();
    let doc = read_export_json(path, "输入映射上下文")?;
    let label = path.display().to_string();
    parse_input_mapping_context(&doc, &label)
}

/// Parse an [`InputSet`] from a CUE4Parse export JSON array (`CPP_DA_MUCHA_INPUT` or any DataAsset
/// defining `pInputMappingContextList` / `InputActionList`).
///
/// Returns `Ok(None)` if the export array does not contain an input set DataAsset.
pub fn parse_input_set(document: &Value, source_label: &str) -> Result<Option<InputSet>> {
    let objects = export_objects(document, source_label)?;
    let Some(set_obj) = objects.iter().copied().find(|obj| {
        if obj_str(obj, "Type") == Some("CPP_DA_MUCHA_INPUT") {
            return true;
        }
        obj.get("Properties")
            .and_then(Value::as_object)
            .is_some_and(|p| {
                p.contains_key("pInputMappingContextList") || p.contains_key("InputActionList")
            })
    }) else {
        return Ok(None);
    };

    let asset = require_non_empty_str(set_obj, "Name", source_label, "Name")?.to_owned();
    let class = require_non_empty_str(set_obj, "Type", source_label, "Type")?.to_owned();
    let path = resolve_package_path(set_obj, &objects, source_label)?;
    let props = set_obj.get("Properties").and_then(Value::as_object);

    let mut contexts = Vec::new();
    if let Some(raw_ctx_list) = props.and_then(|p| p.get("pInputMappingContextList")) {
        let Some(entries) = raw_ctx_list.as_array() else {
            return Err(DataError::invalid_schema(
                source_label,
                "Properties.pInputMappingContextList",
                "pInputMappingContextList 不是 JSON 数组",
            ));
        };
        for (idx, entry) in entries.iter().enumerate() {
            let Some(entry_obj) = entry.as_object() else {
                return Err(DataError::invalid_schema(
                    source_label,
                    format!("Properties.pInputMappingContextList[{idx}]"),
                    "设备上下文条目不是 JSON 对象",
                ));
            };
            let raw_device = entry_obj
                .get("Key")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.pInputMappingContextList[{idx}].Key"),
                        "设备键为空",
                    )
                })?;
            let val_obj = entry_obj
                .get("Value")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.pInputMappingContextList[{idx}].Value"),
                        "缺少上下文引用对象",
                    )
                })?;
            let context_id = val_obj
                .get("ObjectName")
                .and_then(Value::as_str)
                .and_then(extract_quoted_inner)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.pInputMappingContextList[{idx}].Value.ObjectName"),
                        "无法提取上下文名称",
                    )
                })?
                .to_owned();
            let context_path = val_obj
                .get("ObjectPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(strip_object_path_suffix);
            contexts.push(InputSetContext {
                device: normalize_input_device(raw_device),
                context_id,
                context_path,
            });
        }
    }

    let mut actions = Vec::new();
    if let Some(raw_action_list) = props.and_then(|p| p.get("InputActionList")) {
        let Some(entries) = raw_action_list.as_array() else {
            return Err(DataError::invalid_schema(
                source_label,
                "Properties.InputActionList",
                "InputActionList 不是 JSON 数组",
            ));
        };
        for (idx, entry) in entries.iter().enumerate() {
            let Some(entry_obj) = entry.as_object() else {
                return Err(DataError::invalid_schema(
                    source_label,
                    format!("Properties.InputActionList[{idx}]"),
                    "动作表条目不是 JSON 对象",
                ));
            };
            let semantic = entry_obj
                .get("Key")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.InputActionList[{idx}].Key"),
                        "语义动作名为空",
                    )
                })?
                .to_owned();
            let val_obj = entry_obj
                .get("Value")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.InputActionList[{idx}].Value"),
                        "缺少输入动作引用对象",
                    )
                })?;
            let action_id = val_obj
                .get("ObjectName")
                .and_then(Value::as_str)
                .and_then(extract_quoted_inner)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    DataError::invalid_schema(
                        source_label,
                        format!("Properties.InputActionList[{idx}].Value.ObjectName"),
                        "无法提取输入动作名",
                    )
                })?
                .to_owned();
            let action_path = val_obj
                .get("ObjectPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(strip_object_path_suffix);
            actions.push(InputSetAction {
                semantic,
                action_id,
                action_path,
            });
        }
    }

    Ok(Some(InputSet {
        asset,
        path,
        class,
        contexts,
        actions,
    }))
}

/// Load and parse an [`InputSet`] from a JSON file on disk.
pub fn load_input_set(path: impl AsRef<Path>) -> Result<Option<InputSet>> {
    let path = path.as_ref();
    let doc = read_export_json(path, "输入集合资产")?;
    let label = path.display().to_string();
    parse_input_set(&doc, &label)
}

fn extract_key_name(key_val: Option<&Value>) -> Option<String> {
    let val = key_val?;
    if let Some(s) = val.as_str()
        && !s.trim().is_empty()
    {
        return Some(s.trim().to_owned());
    }
    val.as_object()?
        .get("KeyName")?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_modifiers(
    raw: Option<&Value>,
    objects: &[&Map<String, Value>],
    path_index: &HashMap<String, usize>,
) -> Vec<InputModifier> {
    let Some(items) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Some(idx) = resolve_local_ref(item, objects, path_index) {
            let sub = objects[idx];
            out.push(InputModifier {
                name: obj_str(sub, "Name").unwrap_or("").to_owned(),
                class: obj_str(sub, "Type").unwrap_or("InputModifier").to_owned(),
                properties: extract_properties_map(sub),
            });
        } else if let Some(obj_name) = item
            .as_object()
            .and_then(|o| o.get("ObjectName"))
            .and_then(Value::as_str)
        {
            let class = extract_object_type_prefix(obj_name)
                .unwrap_or("InputModifier")
                .to_owned();
            let name = extract_quoted_inner(obj_name)
                .map(|inner| {
                    inner
                        .rsplit_once('.')
                        .or_else(|| inner.rsplit_once(':'))
                        .map_or(inner, |(_, leaf)| leaf)
                })
                .unwrap_or(obj_name)
                .to_owned();
            out.push(InputModifier {
                name,
                class,
                properties: BTreeMap::new(),
            });
        }
    }
    out
}

fn extract_triggers(
    raw: Option<&Value>,
    objects: &[&Map<String, Value>],
    path_index: &HashMap<String, usize>,
) -> Vec<InputTrigger> {
    let Some(items) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Some(idx) = resolve_local_ref(item, objects, path_index) {
            let sub = objects[idx];
            out.push(InputTrigger {
                name: obj_str(sub, "Name").unwrap_or("").to_owned(),
                class: obj_str(sub, "Type").unwrap_or("InputTrigger").to_owned(),
                properties: extract_properties_map(sub),
            });
        } else if let Some(obj_name) = item
            .as_object()
            .and_then(|o| o.get("ObjectName"))
            .and_then(Value::as_str)
        {
            let class = extract_object_type_prefix(obj_name)
                .unwrap_or("InputTrigger")
                .to_owned();
            let name = extract_quoted_inner(obj_name)
                .map(|inner| {
                    inner
                        .rsplit_once('.')
                        .or_else(|| inner.rsplit_once(':'))
                        .map_or(inner, |(_, leaf)| leaf)
                })
                .unwrap_or(obj_name)
                .to_owned();
            out.push(InputTrigger {
                name,
                class,
                properties: BTreeMap::new(),
            });
        }
    }
    out
}
