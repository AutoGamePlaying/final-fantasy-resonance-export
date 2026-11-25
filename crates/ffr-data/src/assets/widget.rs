//! UMG Widget Blueprint hierarchy types exported from the game package (`07_input_ui`).
//!
//! Parses CUE4Parse `WidgetBlueprintGeneratedClass` + `WidgetTree` exports into flat [`WidgetNode`]
//! records and nested [`WidgetTree`] / [`WidgetTreeNode`] hierarchies, resolving panel `Slots`,
//! `UserWidget` / `WidgetTree` `NamedSlotBindings`, and `ExpandableArea` header/body content.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::assets::blueprint::{
    build_ue_path_index, export_objects, extract_properties_map, extract_quoted_inner, obj_str,
    read_export_json, require_non_empty_str, resolve_local_ref, resolve_package_path,
    strip_object_path_suffix,
};
use crate::error::Result;

/// One widget node inside a UMG `WidgetTree`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetNode {
    /// Widget object name (unique within its `WidgetTree`, e.g. `"PathButton"`, `"Overlay_0"`).
    pub name: String,
    /// Widget class / `Type` (e.g. `"WBP_InventoryItemEntry_C"`, `"CommonTextBlock"`, `"Overlay"`).
    pub class: String,
    /// Package path of the nested Widget Blueprint class when this node is a UserWidget instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_path: Option<String>,
    /// Parent widget name inside the same `WidgetTree` (`None` for the root or top-level named slot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
    /// 0-based order among siblings under the same parent.
    pub ord: usize,
    /// Slot class (`"OverlaySlot"`, `"CanvasPanelSlot"`, `"NamedSlot"`, `"WidgetTreeNamedSlot"`,
    /// `"ExpandableAreaHeader"`, `"ExpandableAreaBody"`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_class: Option<String>,
    /// Slot object name or named-slot identifier (e.g. `"OverlaySlot_1"`, `"Slot_32_Icon"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_name: Option<String>,
    /// Overridden layout properties on the slot object (excluding `Parent` and `Content` refs).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slot_properties: BTreeMap<String, Value>,
    /// Overridden properties on the widget object itself.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
}

/// Recursive nested node inside a [`WidgetTree`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetTreeNode {
    pub node: WidgetNode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WidgetTreeNode>,
}

/// Complete nested UMG `WidgetTree` belonging to one `WidgetBlueprintGeneratedClass`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetTree {
    /// Canonical Unreal package path of the owning Widget Blueprint.
    pub blueprint_path: String,
    /// Class name of the owning Widget Blueprint (e.g. `"WBP_BtlCmd_ItemEntry_C"`).
    pub blueprint_class: String,
    /// Primary `RootWidget` subtree, if the Widget Blueprint defines one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<WidgetTreeNode>,
    /// Additional top-level widget roots (such as inherited `WidgetTree.NamedSlotBindings` or
    /// unparented widgets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_roots: Vec<WidgetTreeNode>,
}

impl WidgetTree {
    /// Reconstruct a nested [`WidgetTree`] from flat [`WidgetNode`] rows (for example, loaded from
    /// the `widget_node` SQLite table).
    pub fn from_flat_nodes(
        blueprint_path: String,
        blueprint_class: String,
        nodes: Vec<WidgetNode>,
    ) -> Self {
        let mut by_name: HashMap<String, WidgetNode> = HashMap::with_capacity(nodes.len());
        let mut children_map: HashMap<Option<String>, Vec<String>> = HashMap::new();

        for node in nodes {
            let parent_key = node.parent_name.clone();
            let name = node.name.clone();
            children_map
                .entry(parent_key)
                .or_default()
                .push(name.clone());
            by_name.insert(name, node);
        }

        // Sort each sibling list by `(ord, name)` for deterministic tree assembly.
        for siblings in children_map.values_mut() {
            siblings.sort_by(|a, b| {
                let ord_a = by_name.get(a).map_or(usize::MAX, |n| n.ord);
                let ord_b = by_name.get(b).map_or(usize::MAX, |n| n.ord);
                ord_a.cmp(&ord_b).then_with(|| a.cmp(b))
            });
        }

        let mut visited: HashSet<String> = HashSet::new();
        let top_level_names = children_map.get(&None).cloned().unwrap_or_default();
        let mut root: Option<WidgetTreeNode> = None;
        let mut extra_roots: Vec<WidgetTreeNode> = Vec::new();

        for name in top_level_names {
            if let Some(tree_node) =
                build_tree_node_from_map(&name, &by_name, &children_map, &mut visited)
            {
                let is_named_slot_root = tree_node
                    .node
                    .slot_class
                    .as_deref()
                    .is_some_and(|s| s == "WidgetTreeNamedSlot");
                if root.is_none() && !is_named_slot_root {
                    root = Some(tree_node);
                } else {
                    extra_roots.push(tree_node);
                }
            }
        }

        // Attach any remaining nodes whose parent was missing or cyclic so nothing is dropped.
        let mut remaining: Vec<String> = by_name
            .keys()
            .filter(|name| !visited.contains(*name))
            .cloned()
            .collect();
        remaining.sort();
        for name in remaining {
            if let Some(tree_node) =
                build_tree_node_from_map(&name, &by_name, &children_map, &mut visited)
            {
                extra_roots.push(tree_node);
            }
        }

        Self {
            blueprint_path,
            blueprint_class,
            root,
            extra_roots,
        }
    }

    /// Return all widget nodes in depth-first traversal order.
    pub fn flatten(&self) -> Vec<&WidgetNode> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            collect_flat(root, &mut out);
        }
        for extra in &self.extra_roots {
            collect_flat(extra, &mut out);
        }
        out
    }

    /// Total number of widget nodes across `root` and `extra_roots`.
    pub fn node_count(&self) -> usize {
        self.flatten().len()
    }
}

fn build_tree_node_from_map(
    name: &str,
    by_name: &HashMap<String, WidgetNode>,
    children_map: &HashMap<Option<String>, Vec<String>>,
    visited: &mut HashSet<String>,
) -> Option<WidgetTreeNode> {
    if !visited.insert(name.to_owned()) {
        return None;
    }
    let node = by_name.get(name)?.clone();
    let mut children = Vec::new();
    if let Some(child_names) = children_map.get(&Some(name.to_owned())) {
        for child_name in child_names {
            if let Some(child_node) =
                build_tree_node_from_map(child_name, by_name, children_map, visited)
            {
                children.push(child_node);
            }
        }
    }
    Some(WidgetTreeNode { node, children })
}

fn collect_flat<'a>(tree: &'a WidgetTreeNode, out: &mut Vec<&'a WidgetNode>) {
    out.push(&tree.node);
    for child in &tree.children {
        collect_flat(child, out);
    }
}

struct ParentLink {
    parent_idx: Option<usize>,
    ord: usize,
    slot_class: Option<String>,
    slot_name: Option<String>,
    slot_properties: BTreeMap<String, Value>,
}

/// Parse a [`WidgetTree`] from a CUE4Parse `WidgetBlueprintGeneratedClass` export JSON array.
///
/// Returns `Ok(None)` if the document is not a Widget Blueprint.
pub fn parse_widget_tree(document: &Value, source_label: &str) -> Result<Option<WidgetTree>> {
    let objects = export_objects(document, source_label)?;
    if objects.is_empty() {
        return Ok(None);
    }

    let Some(cls_obj) = objects
        .iter()
        .copied()
        .find(|obj| obj_str(obj, "Type") == Some("WidgetBlueprintGeneratedClass"))
    else {
        return Ok(None);
    };

    let blueprint_class = require_non_empty_str(cls_obj, "Name", source_label, "Name")?.to_owned();
    let blueprint_path = resolve_package_path(cls_obj, &objects, source_label)?;
    let path_index = build_ue_path_index(&objects);

    let tree_obj = cls_obj
        .get("Properties")
        .and_then(Value::as_object)
        .and_then(|p| p.get("WidgetTree"))
        .and_then(|r| resolve_local_ref(r, &objects, &path_index))
        .map(|idx| objects[idx])
        .or_else(|| {
            objects
                .iter()
                .copied()
                .find(|obj| obj_str(obj, "Type") == Some("WidgetTree"))
        });

    let Some(tree_obj) = tree_obj else {
        return Ok(Some(WidgetTree {
            blueprint_path,
            blueprint_class,
            root: None,
            extra_roots: Vec::new(),
        }));
    };

    // Identify every widget whose `Outer` is `WidgetTree'...'`.
    let mut widget_indices: Vec<usize> = Vec::new();
    let mut widget_set: HashSet<usize> = HashSet::new();
    for (idx, obj) in objects.iter().enumerate() {
        let outer_is_tree = obj
            .get("Outer")
            .and_then(|o| o.get("ObjectName"))
            .and_then(Value::as_str)
            .is_some_and(|on| on.starts_with("WidgetTree'"));
        if outer_is_tree {
            widget_indices.push(idx);
            widget_set.insert(idx);
        }
    }

    let mut parent_links: HashMap<usize, ParentLink> = HashMap::new();
    let mut next_child_ord: HashMap<Option<usize>, usize> = HashMap::new();

    let tree_props = tree_obj.get("Properties").and_then(Value::as_object);

    // 1. Primary `RootWidget`.
    let root_widget_idx = tree_props
        .and_then(|p| p.get("RootWidget"))
        .and_then(|r| resolve_local_ref(r, &objects, &path_index))
        .filter(|idx| widget_set.contains(idx));
    if let Some(ridx) = root_widget_idx {
        parent_links.insert(
            ridx,
            ParentLink {
                parent_idx: None,
                ord: 0,
                slot_class: None,
                slot_name: None,
                slot_properties: BTreeMap::new(),
            },
        );
        next_child_ord.insert(None, 1);
    }

    // 2. Top-level `WidgetTree.NamedSlotBindings` (inherited named slots on derived widgets).
    if let Some(nsbs) = tree_props
        .and_then(|p| p.get("NamedSlotBindings"))
        .and_then(Value::as_array)
    {
        for nsb in nsbs {
            let Some(nsb_obj) = nsb.as_object() else {
                continue;
            };
            let content_ref = nsb_obj.get("Value").or_else(|| nsb_obj.get("Content"));
            let Some(cidx) = content_ref
                .and_then(|r| resolve_local_ref(r, &objects, &path_index))
                .filter(|idx| widget_set.contains(idx))
            else {
                continue;
            };
            let slot_name = nsb_obj
                .get("Key")
                .or_else(|| nsb_obj.get("Name"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let ord = allocate_ord(&mut next_child_ord, None);
            parent_links.entry(cidx).or_insert(ParentLink {
                parent_idx: None,
                ord,
                slot_class: Some("WidgetTreeNamedSlot".to_owned()),
                slot_name,
                slot_properties: BTreeMap::new(),
            });
        }
    }

    // 3. Per-widget children: `Slots[]`, `NamedSlotBindings[]`, and `ExpandableArea` Header/Body.
    for &widx in &widget_indices {
        let Some(props) = objects[widx].get("Properties").and_then(Value::as_object) else {
            continue;
        };

        // 3a. Panel `Slots[]`.
        if let Some(slots) = props.get("Slots").and_then(Value::as_array) {
            for slot_ref in slots {
                let Some(sidx) = resolve_local_ref(slot_ref, &objects, &path_index) else {
                    continue;
                };
                let slot_obj = objects[sidx];
                let slot_props_raw = slot_obj.get("Properties").and_then(Value::as_object);
                let Some(cidx) = slot_props_raw
                    .and_then(|sp| sp.get("Content"))
                    .and_then(|r| resolve_local_ref(r, &objects, &path_index))
                    .filter(|idx| widget_set.contains(idx))
                else {
                    continue;
                };
                let ord = allocate_ord(&mut next_child_ord, Some(widx));
                parent_links.entry(cidx).or_insert(ParentLink {
                    parent_idx: Some(widx),
                    ord,
                    slot_class: obj_str(slot_obj, "Type").map(ToOwned::to_owned),
                    slot_name: obj_str(slot_obj, "Name").map(ToOwned::to_owned),
                    slot_properties: extract_slot_properties(slot_obj),
                });
            }
        }

        // 3b. UserWidget `NamedSlotBindings[]`.
        if let Some(nsbs) = props.get("NamedSlotBindings").and_then(Value::as_array) {
            for nsb in nsbs {
                let Some(nsb_obj) = nsb.as_object() else {
                    continue;
                };
                let content_ref = nsb_obj.get("Content").or_else(|| nsb_obj.get("Value"));
                let Some(cidx) = content_ref
                    .and_then(|r| resolve_local_ref(r, &objects, &path_index))
                    .filter(|idx| widget_set.contains(idx))
                else {
                    continue;
                };
                let slot_name = nsb_obj
                    .get("Name")
                    .or_else(|| nsb_obj.get("Key"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let ord = allocate_ord(&mut next_child_ord, Some(widx));
                parent_links.entry(cidx).or_insert(ParentLink {
                    parent_idx: Some(widx),
                    ord,
                    slot_class: Some("NamedSlot".to_owned()),
                    slot_name,
                    slot_properties: BTreeMap::new(),
                });
            }
        }

        // 3c. ExpandableArea `HeaderContent` and `BodyContent`.
        for (field, slot_cls) in [
            ("HeaderContent", "ExpandableAreaHeader"),
            ("BodyContent", "ExpandableAreaBody"),
        ] {
            if let Some(cidx) = props
                .get(field)
                .and_then(|r| resolve_local_ref(r, &objects, &path_index))
                .filter(|idx| widget_set.contains(idx))
            {
                let ord = allocate_ord(&mut next_child_ord, Some(widx));
                parent_links.entry(cidx).or_insert(ParentLink {
                    parent_idx: Some(widx),
                    ord,
                    slot_class: Some(slot_cls.to_owned()),
                    slot_name: Some(field.to_owned()),
                    slot_properties: BTreeMap::new(),
                });
            }
        }
    }

    // 4. Fallback: any Slot object specifying both `Parent` and `Content`.
    for obj in &objects {
        let Some(sp) = obj.get("Properties").and_then(Value::as_object) else {
            continue;
        };
        if !sp.contains_key("Parent") || !sp.contains_key("Content") {
            continue;
        }
        let Some(pidx) = sp
            .get("Parent")
            .and_then(|r| resolve_local_ref(r, &objects, &path_index))
            .filter(|idx| widget_set.contains(idx))
        else {
            continue;
        };
        let Some(cidx) = sp
            .get("Content")
            .and_then(|r| resolve_local_ref(r, &objects, &path_index))
            .filter(|idx| widget_set.contains(idx))
        else {
            continue;
        };
        parent_links.entry(cidx).or_insert_with(|| {
            let ord = allocate_ord(&mut next_child_ord, Some(pidx));
            ParentLink {
                parent_idx: Some(pidx),
                ord,
                slot_class: obj_str(obj, "Type").map(ToOwned::to_owned),
                slot_name: obj_str(obj, "Name").map(ToOwned::to_owned),
                slot_properties: extract_slot_properties(obj),
            }
        });
    }

    // Assemble flat `WidgetNode` list, then build the nested tree.
    let mut flat_nodes = Vec::with_capacity(widget_indices.len());
    for &widx in &widget_indices {
        let obj = objects[widx];
        let Some(name) = obj_str(obj, "Name") else {
            continue;
        };
        let class = obj_str(obj, "Type").unwrap_or("Widget").to_owned();
        let class_path = obj_str(obj, "Class")
            .and_then(extract_quoted_inner)
            .filter(|inner| inner.starts_with('/'))
            .map(strip_object_path_suffix);

        let (parent_name, ord, slot_class, slot_name, slot_properties) =
            if let Some(link) = parent_links.remove(&widx) {
                let parent_name = link
                    .parent_idx
                    .and_then(|pidx| obj_str(objects[pidx], "Name"))
                    .map(ToOwned::to_owned);
                (
                    parent_name,
                    link.ord,
                    link.slot_class,
                    link.slot_name,
                    link.slot_properties,
                )
            } else {
                let ord = allocate_ord(&mut next_child_ord, None);
                (None, ord, None, None, BTreeMap::new())
            };

        flat_nodes.push(WidgetNode {
            name: name.to_owned(),
            class,
            class_path,
            parent_name,
            ord,
            slot_class,
            slot_name,
            slot_properties,
            properties: extract_properties_map(obj),
        });
    }

    Ok(Some(WidgetTree::from_flat_nodes(
        blueprint_path,
        blueprint_class,
        flat_nodes,
    )))
}

/// Load and parse a [`WidgetTree`] from a JSON file on disk.
pub fn load_widget_tree(path: impl AsRef<Path>) -> Result<Option<WidgetTree>> {
    let path = path.as_ref();
    let doc = read_export_json(path, "控件蓝图")?;
    let label = path.display().to_string();
    parse_widget_tree(&doc, &label)
}

fn allocate_ord(counters: &mut HashMap<Option<usize>, usize>, parent: Option<usize>) -> usize {
    let slot = counters.entry(parent).or_insert(0);
    let current = *slot;
    *slot += 1;
    current
}

fn extract_slot_properties(slot_obj: &Map<String, Value>) -> BTreeMap<String, Value> {
    let mut map = extract_properties_map(slot_obj);
    map.remove("Parent");
    map.remove("Content");
    map
}
