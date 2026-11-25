//! Input, DataAsset, Blueprint, and UI widget area (`07_input_ui`) `impl GameDb` queries.
//!
//! Every query uses `Connection::prepare_cached` and executes over explicit primary/secondary
//! indexes (verified in `tests/input_ui.rs` via `assert_db_uses_index`).

use std::collections::HashSet;

use ffr_data::assets::blueprint::{Blueprint, BlueprintComponent, DataAsset};
use ffr_data::assets::input::{
    InputAction, InputKeyMapping, InputMappingContext, InputModifier, InputSetAction, InputTrigger,
    normalize_input_device,
};
use ffr_data::assets::widget::{WidgetNode, WidgetTree};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record::{decode_doc, query_doc_opt, query_docs, query_keyed_doc_opt};

/// Maximum Blueprint inheritance depth walked by [`GameDb::blueprint_value`].
const MAX_BLUEPRINT_CHAIN_DEPTH: usize = 64;

/// Indexed SQL statements executed by `07_input_ui` query methods.
///
/// Exposed so integration tests can verify the exact production queries via `assert_db_uses_index`.
pub mod sql {
    pub const INPUT_ACTION_BY_ID: &str = "SELECT doc FROM input_action WHERE id = ?1";
    pub const INPUT_ACTION_BY_PATH: &str = "SELECT id, doc FROM input_action WHERE path = ?1";
    pub const INPUT_ACTION_ID_BY_PATH: &str = "SELECT id FROM input_action WHERE path = ?1";

    pub const INPUT_CONTEXT_BY_ID: &str = "SELECT doc FROM input_context WHERE id = ?1";
    pub const INPUT_CONTEXT_BY_PATH: &str = "SELECT id, doc FROM input_context WHERE path = ?1";
    pub const INPUT_CONTEXT_ID_BY_PATH: &str = "SELECT id FROM input_context WHERE path = ?1";

    pub const INPUT_BINDINGS_BY_CONTEXT: &str = "\
        SELECT m.action_id, a.path, m.key, m.modifiers, m.triggers, m.ord \
        FROM input_mapping m \
        LEFT JOIN input_action a ON a.id = m.action_id \
        WHERE m.context_id = ?1 \
        ORDER BY m.ord";

    pub const KEYS_FOR_ACTION_ALL_DEVICES: &str = "\
        SELECT DISTINCT m.context_id, s.device, m.action_id, m.key, m.modifiers, m.triggers, m.ord \
        FROM input_mapping m \
        LEFT JOIN input_set s ON s.context_id = m.context_id \
        WHERE m.action_id = ?1 \
        ORDER BY m.context_id, m.ord";

    pub const KEYS_FOR_ACTION_BY_DEVICE: &str = "\
        SELECT DISTINCT m.context_id, s.device, m.action_id, m.key, m.modifiers, m.triggers, m.ord \
        FROM input_mapping m \
        INNER JOIN input_set s ON s.context_id = m.context_id \
        WHERE m.action_id = ?1 AND s.device = ?2 \
        ORDER BY m.context_id, m.ord";

    pub const CONTEXTS_FOR_ACTION: &str = "\
        SELECT DISTINCT c.id, c.doc \
        FROM input_mapping m \
        INNER JOIN input_context c ON c.id = m.context_id \
        WHERE m.action_id = ?1 \
        ORDER BY c.id";

    pub const ACTIONS_FOR_KEY: &str = "\
        SELECT DISTINCT m.context_id, s.device, m.action_id, m.key, m.modifiers, m.triggers, m.ord \
        FROM input_mapping m \
        LEFT JOIN input_set s ON s.context_id = m.context_id \
        WHERE m.key = ?1 \
        ORDER BY m.context_id, m.ord";

    pub const INPUT_SET_ACTIONS_BY_SEMANTIC: &str = "\
        SELECT asset, semantic, action_id \
        FROM input_set_action \
        WHERE semantic = ?1 \
        ORDER BY asset, action_id";

    pub const INPUT_SET_ACTIONS_BY_ACTION: &str = "\
        SELECT asset, semantic, action_id \
        FROM input_set_action \
        WHERE action_id = ?1 \
        ORDER BY asset, semantic";

    pub const BLUEPRINT_BY_PATH: &str = "SELECT doc FROM blueprint WHERE path = ?1";
    pub const BLUEPRINT_BY_CLASS: &str = "\
        SELECT path, doc \
        FROM blueprint \
        WHERE class_name = ?1 \
        ORDER BY path \
        LIMIT 1";
    pub const BLUEPRINTS_BY_PARENT: &str = "\
        SELECT path, doc \
        FROM blueprint \
        WHERE parent_class = ?1 \
        ORDER BY path";
    pub const BLUEPRINT_META_BY_PATH: &str =
        "SELECT path, class_name, kind FROM blueprint WHERE path = ?1";
    pub const BLUEPRINT_META_BY_CLASS: &str = "\
        SELECT path, class_name, kind \
        FROM blueprint \
        WHERE class_name = ?1 \
        ORDER BY path \
        LIMIT 1";

    pub const BLUEPRINT_COMPONENTS_BY_BP: &str = "\
        SELECT name, doc \
        FROM blueprint_component \
        WHERE blueprint_id = ?1 \
        ORDER BY name";

    pub const WIDGET_NODES_BY_BLUEPRINT: &str = "\
        SELECT name, doc \
        FROM widget_node \
        WHERE blueprint_id = ?1 \
        ORDER BY ord, name";

    pub const WIDGETS_USING_CLASS: &str = "\
        SELECT w.blueprint_id, b.class_name, w.name, w.doc \
        FROM widget_node w \
        INNER JOIN blueprint b ON b.path = w.blueprint_id \
        WHERE w.class = ?1 \
        ORDER BY w.blueprint_id, w.ord, w.name";

    pub const DATA_ASSET_BY_PATH: &str = "SELECT doc FROM data_asset WHERE path = ?1";
    pub const DATA_ASSETS_BY_CLASS: &str = "\
        SELECT path, doc \
        FROM data_asset \
        WHERE class = ?1 \
        ORDER BY path";
}

/// One resolved key binding returned by [`GameDb::keys_for_action`] or [`GameDb::actions_for_key`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionKeyBinding {
    pub context_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub action_id: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<InputModifier>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<InputTrigger>,
    pub ord: usize,
}

/// One `widget_node` occurrence returned by [`GameDb::widgets_using_class`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetClassUsage {
    /// Package path of the Widget Blueprint that instantiates this widget.
    pub blueprint_id: String,
    /// Class name of the Widget Blueprint that instantiates this widget.
    pub blueprint_class: String,
    /// The widget node itself.
    pub node: WidgetNode,
}

/// Result of resolving a property along a Blueprint's inheritance chain via
/// [`GameDb::blueprint_value`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BlueprintPropertyLookup {
    /// Found the first Blueprint in the inheritance chain that explicitly overrode the property.
    Overridden {
        defined_in_path: String,
        defined_in_class: String,
        value: Value,
        /// Walked Blueprint class names from the queried class up to `defined_in_class`.
        chain: Vec<String>,
    },
    /// No Blueprint in the inheritance chain overrode this property; its effective value lives in
    /// C++ defaults (not exported in Blueprint JSON).
    NotOverriddenInBlueprint {
        /// Walked Blueprint class names before reaching the C++ base class.
        chain: Vec<String>,
        /// Terminal C++ parent class where the C++ default lives, if known.
        cpp_parent: Option<String>,
        /// Explicit Chinese explanation (`"蓝图没改，值在 C++ 默认里..."`).
        reason: String,
    },
    /// The requested Blueprint path or class name was not found in the database.
    BlueprintNotFound { query: String },
}

impl GameDb {
    /// Fetch one [`InputAction`] by action ID (e.g. `"IA_Battle_AutoBattle"`) or UE package path.
    pub fn input_action(&self, id_or_path: &str) -> Result<Option<InputAction>> {
        self.with_conn(|conn| {
            if id_or_path.starts_with('/') {
                query_keyed_doc_opt(
                    conn,
                    "input_action",
                    sql::INPUT_ACTION_BY_PATH,
                    [id_or_path],
                )
            } else {
                query_doc_opt(
                    conn,
                    "input_action",
                    id_or_path,
                    sql::INPUT_ACTION_BY_ID,
                    [id_or_path],
                )
            }
        })
    }

    /// Fetch one [`InputMappingContext`] by context ID (e.g. `"IMC_Battle_Default_KBM"`) or path.
    pub fn input_context(&self, id_or_path: &str) -> Result<Option<InputMappingContext>> {
        self.with_conn(|conn| {
            if id_or_path.starts_with('/') {
                query_keyed_doc_opt(
                    conn,
                    "input_context",
                    sql::INPUT_CONTEXT_BY_PATH,
                    [id_or_path],
                )
            } else {
                query_doc_opt(
                    conn,
                    "input_context",
                    id_or_path,
                    sql::INPUT_CONTEXT_BY_ID,
                    [id_or_path],
                )
            }
        })
    }

    /// Return all ordered key bindings defined in an `InputMappingContext` (by ID or package path).
    pub fn input_bindings(&self, context: &str) -> Result<Vec<InputKeyMapping>> {
        self.with_conn(|conn| {
            let Some(context_id) = resolve_context_id(conn, context)? else {
                return Ok(Vec::new());
            };
            let mut stmt = conn
                .prepare_cached(sql::INPUT_BINDINGS_BY_CONTEXT)
                .map_err(|err| DbError::sqlite("准备查询 input_mapping", err))?;
            let mut rows = stmt
                .query([context_id.as_str()])
                .map_err(|err| DbError::sqlite("查询 input_mapping", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 input_mapping 行", err))?
            {
                let action_id: String = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 input_mapping.action_id", err))?;
                let action_path: Option<String> = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 input_action.path", err))?;
                let key: String = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 input_mapping.key", err))?;
                let mods_raw: String = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取 input_mapping.modifiers", err))?;
                let trigs_raw: String = row
                    .get(4)
                    .map_err(|err| DbError::sqlite("读取 input_mapping.triggers", err))?;
                let ord_i64: i64 = row
                    .get(5)
                    .map_err(|err| DbError::sqlite("读取 input_mapping.ord", err))?;
                let ord = usize::try_from(ord_i64).unwrap_or(0);
                let row_key = format!("{context_id}:{ord}");
                let modifiers: Vec<InputModifier> =
                    decode_doc("input_mapping.modifiers", &row_key, &mods_raw)?;
                let triggers: Vec<InputTrigger> =
                    decode_doc("input_mapping.triggers", &row_key, &trigs_raw)?;
                out.push(InputKeyMapping {
                    action_id,
                    action_path,
                    key,
                    modifiers,
                    triggers,
                    setting_behavior: None,
                    ord,
                });
            }
            Ok(out)
        })
    }

    /// Find physical keys bound to `action` (action ID, package path, or semantic name in
    /// `input_set_action`), optionally filtered by `device` via `input_set`.
    ///
    /// Device names are normalized: `"KeyboardAndMouse"` / `"kbm"` /
    /// `"eMuchaInputDeviceType::KeyboardAndMouse"` all match `"KeyboardAndMouse"`;
    /// `"Gamepad"` / `"pad"` / `"eMuchaInputDeviceType::Gamepad"` all match `"Gamepad"`.
    pub fn keys_for_action(
        &self,
        action: &str,
        device: Option<&str>,
    ) -> Result<Vec<ActionKeyBinding>> {
        let normalized_device = device.map(normalize_input_device);
        self.with_conn(|conn| {
            let action_ids = resolve_action_ids(conn, action)?;
            let mut out = Vec::new();
            for action_id in action_ids {
                match normalized_device.as_deref() {
                    Some(dev) => {
                        let mut stmt = conn
                            .prepare_cached(sql::KEYS_FOR_ACTION_BY_DEVICE)
                            .map_err(|err| DbError::sqlite("准备按设备查询动作按键", err))?;
                        let mut rows = stmt
                            .query(params![action_id.as_str(), dev])
                            .map_err(|err| DbError::sqlite("按设备查询动作按键", err))?;
                        collect_binding_rows(&mut rows, &mut out)?;
                    }
                    None => {
                        let mut stmt = conn
                            .prepare_cached(sql::KEYS_FOR_ACTION_ALL_DEVICES)
                            .map_err(|err| DbError::sqlite("准备查询动作按键", err))?;
                        let mut rows = stmt
                            .query([action_id.as_str()])
                            .map_err(|err| DbError::sqlite("查询动作按键", err))?;
                        collect_binding_rows(&mut rows, &mut out)?;
                    }
                }
            }
            Ok(out)
        })
    }

    /// Return all [`InputMappingContext`]s that bind `action` (ID, package path, or semantic name).
    pub fn contexts_for_action(&self, action: &str) -> Result<Vec<InputMappingContext>> {
        self.with_conn(|conn| {
            let action_ids = resolve_action_ids(conn, action)?;
            let mut seen: HashSet<String> = HashSet::new();
            let mut out = Vec::new();
            for action_id in action_ids {
                let contexts: Vec<InputMappingContext> = query_docs(
                    conn,
                    "input_context",
                    sql::CONTEXTS_FOR_ACTION,
                    [action_id.as_str()],
                )?;
                for ctx in contexts {
                    if seen.insert(ctx.id.clone()) {
                        out.push(ctx);
                    }
                }
            }
            Ok(out)
        })
    }

    /// Return all input bindings across every context that listen to the given physical `key`
    /// (e.g. `"F"`, `"Enter"`, `"Tab"`, `"Gamepad_FaceButton_Bottom"`).
    pub fn actions_for_key(&self, key: &str) -> Result<Vec<ActionKeyBinding>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::ACTIONS_FOR_KEY)
                .map_err(|err| DbError::sqlite("准备按按键反查动作", err))?;
            let mut rows = stmt
                .query([key])
                .map_err(|err| DbError::sqlite("按按键反查动作", err))?;
            let mut out = Vec::new();
            collect_binding_rows(&mut rows, &mut out)?;
            Ok(out)
        })
    }

    /// Look up semantic input action entries by semantic name or action ID in `input_set_action`.
    pub fn input_set_actions_by_semantic(&self, semantic: &str) -> Result<Vec<InputSetAction>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(sql::INPUT_SET_ACTIONS_BY_SEMANTIC)
                .map_err(|err| DbError::sqlite("准备按语义名查询 input_set_action", err))?;
            let mut rows = stmt
                .query([semantic])
                .map_err(|err| DbError::sqlite("按语义名查询 input_set_action", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 input_set_action 行", err))?
            {
                let _asset: String = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取 input_set_action.asset", err))?;
                let sem: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取 input_set_action.semantic", err))?;
                let action_id: String = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取 input_set_action.action_id", err))?;
                out.push(InputSetAction {
                    semantic: sem,
                    action_id,
                    action_path: None,
                });
            }
            Ok(out)
        })
    }

    /// Fetch one [`Blueprint`] by package path (e.g. `"/Game/BP/Map/Unit/BP_MapUnit_Player"`),
    /// generated class name (`"BP_MapUnit_Player_C"`), or Blueprint asset name
    /// (`"BP_MapUnit_Player"`).
    pub fn blueprint(&self, path_or_class: &str) -> Result<Option<Blueprint>> {
        self.with_conn(|conn| fetch_blueprint_on_conn(conn, path_or_class))
    }

    /// Return all [`Blueprint`]s whose immediate `parent_class` matches `class` (accepts both
    /// C++ parent names such as `"CPP_MapUnit_Player"` and Blueprint parent names with or without
    /// `_C`).
    pub fn blueprints_by_parent(&self, class: &str) -> Result<Vec<Blueprint>> {
        self.with_conn(|conn| {
            let exact: Vec<Blueprint> =
                query_docs(conn, "blueprint", sql::BLUEPRINTS_BY_PARENT, [class])?;
            if !exact.is_empty() || class.ends_with("_C") {
                return Ok(exact);
            }
            let with_c = format!("{class}_C");
            query_docs(
                conn,
                "blueprint",
                sql::BLUEPRINTS_BY_PARENT,
                [with_c.as_str()],
            )
        })
    }

    /// Return all [`BlueprintComponent`] rows belonging to one Blueprint (by path or class name).
    pub fn blueprint_components(&self, path_or_class: &str) -> Result<Vec<BlueprintComponent>> {
        self.with_conn(|conn| {
            let Some((bp_path, _, _)) = resolve_blueprint_meta(conn, path_or_class)? else {
                return Ok(Vec::new());
            };
            query_docs(
                conn,
                "blueprint_component",
                sql::BLUEPRINT_COMPONENTS_BY_BP,
                [bp_path.as_str()],
            )
        })
    }

    /// Resolve `property_path` along a Blueprint's inheritance chain, returning the first ancestor
    /// that explicitly overrode the property.
    ///
    /// If no Blueprint in the inheritance chain overrode `property_path`, explicitly returns
    /// [`BlueprintPropertyLookup::NotOverriddenInBlueprint`] (`"蓝图没改，值在 C++ 默认里"`)
    /// rather than guessing a default value.
    pub fn blueprint_value(
        &self,
        class: &str,
        property_path: &str,
    ) -> Result<BlueprintPropertyLookup> {
        self.with_conn(|conn| {
            let Some(mut current) = fetch_blueprint_on_conn(conn, class)? else {
                return Ok(BlueprintPropertyLookup::BlueprintNotFound {
                    query: class.to_owned(),
                });
            };

            let mut visited: HashSet<String> = HashSet::new();
            let mut chain: Vec<String> = Vec::new();
            let mut terminal_cpp_parent: Option<String> = None;

            for _ in 0..MAX_BLUEPRINT_CHAIN_DEPTH {
                if !visited.insert(current.path.clone()) {
                    break;
                }
                chain.push(current.class_name.clone());

                if let Some(value) = current.lookup_property(property_path).cloned() {
                    return Ok(BlueprintPropertyLookup::Overridden {
                        defined_in_path: current.path,
                        defined_in_class: current.class_name,
                        value,
                        chain,
                    });
                }

                // Prefer exact `parent_path` when it is a Blueprint asset path (avoids ambiguity
                // if two Blueprints share a class name); fall back to `parent_class`.
                let next_bp = if let Some(parent_path) = current.parent_path.as_deref()
                    && !parent_path.starts_with("/Script/")
                {
                    match fetch_blueprint_on_conn(conn, parent_path)? {
                        Some(bp) => Some(bp),
                        None => match current.parent_class.as_deref() {
                            Some(pc) => fetch_blueprint_on_conn(conn, pc)?,
                            None => None,
                        },
                    }
                } else if let Some(parent_class) = current.parent_class.as_deref() {
                    fetch_blueprint_on_conn(conn, parent_class)?
                } else {
                    None
                };

                match next_bp {
                    Some(parent_bp) => current = parent_bp,
                    None => {
                        terminal_cpp_parent = current.parent_class.clone();
                        break;
                    }
                }
            }

            let reason = match &terminal_cpp_parent {
                Some(cpp) => format!("蓝图没改，值在 C++ 默认里（终极父类：{cpp}）"),
                None => "蓝图没改，值在 C++ 默认里".to_owned(),
            };
            Ok(BlueprintPropertyLookup::NotOverriddenInBlueprint {
                chain,
                cpp_parent: terminal_cpp_parent,
                reason,
            })
        })
    }

    /// Reconstruct the nested [`WidgetTree`] for a Widget Blueprint (by package path,
    /// `WBP_*_C` class name, or `WBP_*` asset name).
    pub fn widget_tree(&self, blueprint: &str) -> Result<Option<WidgetTree>> {
        self.with_conn(|conn| {
            let Some((bp_path, bp_class, bp_kind)) = resolve_blueprint_meta(conn, blueprint)?
            else {
                return Ok(None);
            };
            let nodes: Vec<WidgetNode> = query_docs(
                conn,
                "widget_node",
                sql::WIDGET_NODES_BY_BLUEPRINT,
                [bp_path.as_str()],
            )?;
            if bp_kind != "widget" && nodes.is_empty() {
                return Ok(None);
            }
            Ok(Some(WidgetTree::from_flat_nodes(bp_path, bp_class, nodes)))
        })
    }

    /// Find every Widget Blueprint node that instantiates widget `class` (e.g.
    /// `"WBP_InventoryItemEntry_C"`, `"WBP_InventoryItemEntry"`, or `"CommonTextBlock"`).
    pub fn widgets_using_class(&self, class: &str) -> Result<Vec<WidgetClassUsage>> {
        self.with_conn(|conn| {
            let exact = query_widget_usages(conn, class)?;
            if !exact.is_empty() || class.ends_with("_C") {
                return Ok(exact);
            }
            let with_c = format!("{class}_C");
            query_widget_usages(conn, &with_c)
        })
    }

    /// Fetch one [`DataAsset`] by its canonical UE package path.
    pub fn data_asset(&self, path: &str) -> Result<Option<DataAsset>> {
        self.with_conn(|conn| {
            query_doc_opt(conn, "data_asset", path, sql::DATA_ASSET_BY_PATH, [path])
        })
    }

    /// Return all [`DataAsset`]s of a given UE class (e.g. `"CPP_DA_MUCHA_INPUT"`,
    /// `"CPP_DA_RegularIdAsset"`).
    pub fn data_assets_by_class(&self, class: &str) -> Result<Vec<DataAsset>> {
        self.with_conn(|conn| query_docs(conn, "data_asset", sql::DATA_ASSETS_BY_CLASS, [class]))
    }
}

fn resolve_context_id(conn: &Connection, context: &str) -> Result<Option<String>> {
    if context.starts_with('/') {
        let mut stmt = conn
            .prepare_cached(sql::INPUT_CONTEXT_ID_BY_PATH)
            .map_err(|err| DbError::sqlite("准备按路径查找 input_context", err))?;
        stmt.query_row([context], |row| row.get(0))
            .optional()
            .map_err(|err| DbError::sqlite("按路径查找 input_context", err))
    } else {
        Ok(Some(context.to_owned()))
    }
}

fn resolve_action_ids(conn: &Connection, action: &str) -> Result<Vec<String>> {
    if action.starts_with('/') {
        let mut stmt = conn
            .prepare_cached(sql::INPUT_ACTION_ID_BY_PATH)
            .map_err(|err| DbError::sqlite("准备按路径查找 input_action", err))?;
        let found: Option<String> = stmt
            .query_row([action], |row| row.get(0))
            .optional()
            .map_err(|err| DbError::sqlite("按路径查找 input_action", err))?;
        return Ok(found.into_iter().collect());
    }

    // If `action` matches a known InputAction ID directly, use it; otherwise check whether it is
    // a semantic action name in `input_set_action` (e.g. `"UnitInteract"`).
    let mut exists_stmt = conn
        .prepare_cached(sql::INPUT_ACTION_BY_ID)
        .map_err(|err| DbError::sqlite("准备检查 input_action 是否存在", err))?;
    let is_direct: Option<String> = exists_stmt
        .query_row([action], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite("检查 input_action 是否存在", err))?;
    if is_direct.is_some() {
        return Ok(vec![action.to_owned()]);
    }

    let mut sem_stmt = conn
        .prepare_cached(sql::INPUT_SET_ACTIONS_BY_SEMANTIC)
        .map_err(|err| DbError::sqlite("准备按语义名解析动作", err))?;
    let mut rows = sem_stmt
        .query([action])
        .map_err(|err| DbError::sqlite("按语义名解析动作", err))?;
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取语义动作解析行", err))?
    {
        let action_id: String = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 input_set_action.action_id", err))?;
        if seen.insert(action_id.clone()) {
            resolved.push(action_id);
        }
    }
    if resolved.is_empty() {
        // Still allow querying dangling action IDs present in `input_mapping`.
        resolved.push(action.to_owned());
    }
    Ok(resolved)
}

fn collect_binding_rows(
    rows: &mut rusqlite::Rows<'_>,
    out: &mut Vec<ActionKeyBinding>,
) -> Result<()> {
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取动作按键映射行", err))?
    {
        let context_id: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 context_id", err))?;
        let device: Option<String> = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 device", err))?;
        let action_id: String = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 action_id", err))?;
        let key: String = row.get(3).map_err(|err| DbError::sqlite("读取 key", err))?;
        let mods_raw: String = row
            .get(4)
            .map_err(|err| DbError::sqlite("读取 modifiers", err))?;
        let trigs_raw: String = row
            .get(5)
            .map_err(|err| DbError::sqlite("读取 triggers", err))?;
        let ord_i64: i64 = row.get(6).map_err(|err| DbError::sqlite("读取 ord", err))?;
        let ord = usize::try_from(ord_i64).unwrap_or(0);
        let row_key = format!("{context_id}:{ord}");
        let modifiers: Vec<InputModifier> =
            decode_doc("input_mapping.modifiers", &row_key, &mods_raw)?;
        let triggers: Vec<InputTrigger> =
            decode_doc("input_mapping.triggers", &row_key, &trigs_raw)?;
        out.push(ActionKeyBinding {
            context_id,
            device,
            action_id,
            key,
            modifiers,
            triggers,
            ord,
        });
    }
    Ok(())
}

fn fetch_blueprint_on_conn(conn: &Connection, path_or_class: &str) -> Result<Option<Blueprint>> {
    if path_or_class.starts_with('/') {
        return query_doc_opt(
            conn,
            "blueprint",
            path_or_class,
            sql::BLUEPRINT_BY_PATH,
            [path_or_class],
        );
    }
    if let Some(bp) =
        query_keyed_doc_opt(conn, "blueprint", sql::BLUEPRINT_BY_CLASS, [path_or_class])?
    {
        return Ok(Some(bp));
    }
    if !path_or_class.ends_with("_C") {
        let with_c = format!("{path_or_class}_C");
        return query_keyed_doc_opt(
            conn,
            "blueprint",
            sql::BLUEPRINT_BY_CLASS,
            [with_c.as_str()],
        );
    }
    Ok(None)
}

fn resolve_blueprint_meta(
    conn: &Connection,
    path_or_class: &str,
) -> Result<Option<(String, String, String)>> {
    if path_or_class.starts_with('/') {
        let mut stmt = conn
            .prepare_cached(sql::BLUEPRINT_META_BY_PATH)
            .map_err(|err| DbError::sqlite("准备按路径查询蓝图元信息", err))?;
        return stmt
            .query_row([path_or_class], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()
            .map_err(|err| DbError::sqlite("按路径查询蓝图元信息", err));
    }

    let mut stmt = conn
        .prepare_cached(sql::BLUEPRINT_META_BY_CLASS)
        .map_err(|err| DbError::sqlite("准备按类名查询蓝图元信息", err))?;
    if let Some(found) = stmt
        .query_row([path_or_class], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()
        .map_err(|err| DbError::sqlite("按类名查询蓝图元信息", err))?
    {
        return Ok(Some(found));
    }
    if !path_or_class.ends_with("_C") {
        let with_c = format!("{path_or_class}_C");
        return stmt
            .query_row([with_c.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()
            .map_err(|err| DbError::sqlite("按类名查询蓝图元信息", err));
    }
    Ok(None)
}

fn query_widget_usages(conn: &Connection, class: &str) -> Result<Vec<WidgetClassUsage>> {
    let mut stmt = conn
        .prepare_cached(sql::WIDGETS_USING_CLASS)
        .map_err(|err| DbError::sqlite("准备按控件类查询 widget_node", err))?;
    let mut rows = stmt
        .query([class])
        .map_err(|err| DbError::sqlite("按控件类查询 widget_node", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite("读取 widget_node 使用记录", err))?
    {
        let blueprint_id: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 widget_node.blueprint_id", err))?;
        let blueprint_class: String = row
            .get(1)
            .map_err(|err| DbError::sqlite("读取 blueprint.class_name", err))?;
        let name: String = row
            .get(2)
            .map_err(|err| DbError::sqlite("读取 widget_node.name", err))?;
        let doc_raw: String = row
            .get(3)
            .map_err(|err| DbError::sqlite("读取 widget_node.doc", err))?;
        let node: WidgetNode =
            decode_doc("widget_node", format!("{blueprint_id}:{name}"), &doc_raw)?;
        out.push(WidgetClassUsage {
            blueprint_id,
            blueprint_class,
            node,
        });
    }
    Ok(out)
}
