//! Integration tests for the `07_input_ui` database area (Enhanced Input, DataAssets,
//! Blueprints/components, and UMG WidgetTrees), plus optional real-export verification tests.

use std::path::PathBuf;

use ffr_data::assets::blueprint::{BlueprintKind, ComponentOrigin};
use ffr_db::query::input_ui::{BlueprintPropertyLookup, sql};
use ffr_db::testing::{assert_db_uses_index, build_fixture_db_stages, build_temp_db};
use ffr_db::{BuildInputs, Stage, StageStatus};
use rusqlite::params;
use serde_json::json;

const ALL_INPUT_UI_TABLES: &[&str] = &[
    "input_action",
    "input_context",
    "input_mapping",
    "input_set",
    "input_set_action",
    "data_asset",
    "blueprint",
    "blueprint_component",
    "widget_node",
];

#[test]
fn input_ui_fixture_import_queries_and_issues() {
    let test_db =
        build_fixture_db_stages("input_ui", Some(vec![Stage::InputUi])).expect("build input_ui db");
    let db = test_db.db();

    let stage_rep = test_db
        .report()
        .stages
        .iter()
        .find(|s| s.stage == "input_ui")
        .expect("input_ui stage report");
    assert_eq!(stage_rep.status, StageStatus::Ok);
    assert!(stage_rep.rows > 0);

    // 1. Parse errors & dangling references are recorded in `build_issue` without aborting build.
    let issues = db.build_issues().expect("read build_issue");
    assert!(
        issues.iter().any(|i| i.stage == "input_ui"
            && i.kind == "parse_error"
            && i.subject.contains("BP_Mock_Broken")),
        "expected parse_error for BP_Mock_Broken, got: {issues:?}"
    );
    assert!(
        issues.iter().any(|i| {
            i.stage == "input_ui"
                && i.kind == "dangling_input_action"
                && i.detail.contains("IA_Mock_Dangling")
        }),
        "expected dangling_input_action for IA_Mock_Dangling, got: {issues:?}"
    );

    // 2. Ignored DataTable and LevelSequence (including its director BP) must not pollute tables.
    assert!(
        db.blueprint("LS_Mock_Ignored_DirectorBP_C")
            .expect("query director bp")
            .is_none()
    );
    assert!(
        db.data_asset("/Game/Datatable/DT_Mock_Ignored")
            .expect("query ignored dt")
            .is_none()
    );

    // 3. InputAction metadata & triggers.
    let ia_move = db
        .input_action("IA_Mock_Move")
        .expect("query IA_Mock_Move")
        .expect("IA_Mock_Move exists");
    assert_eq!(ia_move.value_type, "Axis2D");
    assert_eq!(ia_move.consume_input, Some(true));
    assert_eq!(ia_move.triggers.len(), 1);
    assert_eq!(ia_move.triggers[0].class, "InputTriggerDown");

    let ia_confirm = db
        .input_action("/Game/Input/Action/IA_Mock_Confirm")
        .expect("query IA_Mock_Confirm by path")
        .expect("IA_Mock_Confirm exists");
    assert_eq!(ia_confirm.id, "IA_Mock_Confirm");
    assert_eq!(ia_confirm.value_type, "Boolean");

    // 4. `input_bindings(context)` (by ID and by UE path, preserving order, modifiers, triggers).
    let kbm_bindings = db
        .input_bindings("IMC_Mock_Field_KBM")
        .expect("input_bindings KBM");
    assert_eq!(kbm_bindings.len(), 5);
    assert_eq!(kbm_bindings[0].action_id, "IA_Mock_Move");
    assert_eq!(kbm_bindings[0].key, "W");
    assert_eq!(kbm_bindings[0].modifiers.len(), 1);
    assert_eq!(
        kbm_bindings[0].modifiers[0].class,
        "InputModifierSwizzleAxis"
    );
    assert_eq!(kbm_bindings[1].key, "S");
    assert_eq!(kbm_bindings[1].modifiers.len(), 2);
    assert_eq!(kbm_bindings[4].action_id, "IA_Mock_Menu");
    assert_eq!(kbm_bindings[4].key, "Tab");

    let pad_bindings = db
        .input_bindings("/Game/Input/Mapping/IMC_Mock_Field_Pad")
        .expect("input_bindings Pad via DefaultKeyMappings");
    assert_eq!(pad_bindings.len(), 4);
    assert_eq!(pad_bindings[0].key, "Gamepad_Left2D");
    assert_eq!(pad_bindings[0].modifiers[0].class, "InputModifierDeadZone");

    // 5. `keys_for_action(action, device)` across device filters & semantic names.
    let confirm_kbm = db
        .keys_for_action("IA_Mock_Confirm", Some("KeyboardAndMouse"))
        .expect("keys_for_action KBM");
    let confirm_kbm_keys: Vec<&str> = confirm_kbm.iter().map(|b| b.key.as_str()).collect();
    assert_eq!(confirm_kbm_keys, vec!["F", "Enter"]);
    assert!(
        confirm_kbm
            .iter()
            .all(|b| b.device.as_deref() == Some("KeyboardAndMouse"))
    );

    // Short alias ("pad") and semantic name ("FieldConfirm") resolve via `input_set` / `input_set_action`.
    let confirm_pad = db
        .keys_for_action("FieldConfirm", Some("pad"))
        .expect("keys_for_action semantic+pad");
    assert_eq!(confirm_pad.len(), 1);
    assert_eq!(confirm_pad[0].key, "Gamepad_FaceButton_Bottom");
    assert_eq!(confirm_pad[0].device.as_deref(), Some("Gamepad"));

    let confirm_all = db
        .keys_for_action("IA_Mock_Confirm", None)
        .expect("keys_for_action all");
    assert_eq!(confirm_all.len(), 4); // F, Enter (Field_KBM), Gamepad_FaceButton_Bottom (Field_Pad), Enter (Menu_KBM)

    // 6. `contexts_for_action(action)`
    let confirm_ctxs = db
        .contexts_for_action("IA_Mock_Confirm")
        .expect("contexts_for_action");
    let ctx_ids: Vec<&str> = confirm_ctxs.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ctx_ids,
        vec![
            "IMC_Mock_Field_KBM",
            "IMC_Mock_Field_Pad",
            "IMC_Mock_Menu_KBM"
        ]
    );

    // 7. `actions_for_key(key)`
    let enter_actions = db.actions_for_key("Enter").expect("actions_for_key Enter");
    assert_eq!(enter_actions.len(), 2);
    assert_eq!(enter_actions[0].context_id, "IMC_Mock_Field_KBM");
    assert_eq!(enter_actions[0].device.as_deref(), Some("KeyboardAndMouse"));
    assert_eq!(enter_actions[1].context_id, "IMC_Mock_Menu_KBM");
    assert_eq!(enter_actions[1].device, None);

    let shift_actions = db
        .actions_for_key("LeftShift")
        .expect("actions_for_key LeftShift");
    assert_eq!(shift_actions.len(), 1);
    assert_eq!(shift_actions[0].action_id, "IA_Mock_Dash");
    assert_eq!(shift_actions[0].triggers[0].class, "InputTriggerHold");

    // 8. `data_asset(path)` and `data_assets_by_class(class)`
    let cfg = db
        .data_asset("/Game/Field/Data/DA_Mock_Config")
        .expect("data_asset")
        .expect("DA_Mock_Config exists");
    assert_eq!(cfg.class, "CPP_DA_MockConfig");
    assert_eq!(cfg.properties.get("MaxSpeed"), Some(&json!(420.0)));

    let input_das = db
        .data_assets_by_class("CPP_DA_MUCHA_INPUT")
        .expect("data_assets_by_class CPP_DA_MUCHA_INPUT");
    assert_eq!(input_das.len(), 1);
    assert_eq!(input_das[0].name, "DA_Input_MockField");

    let primary_das = db
        .data_assets_by_class("PrimaryDataAsset")
        .expect("data_assets_by_class PrimaryDataAsset");
    assert_eq!(primary_das.len(), 1);
    assert_eq!(primary_das[0].properties.get("Version"), Some(&json!(3)));

    // 9. `blueprint(path_or_class)`, `blueprints_by_parent(class)`, and `blueprint_components`
    let base_bp = db
        .blueprint("BP_Mock_BaseUnit")
        .expect("blueprint without _C")
        .expect("BP_Mock_BaseUnit exists");
    assert_eq!(base_bp.class_name, "BP_Mock_BaseUnit_C");
    assert_eq!(base_bp.parent_class.as_deref(), Some("CPP_Mock_Unit"));
    assert_eq!(base_bp.kind, BlueprintKind::Actor);
    assert_eq!(base_bp.components.len(), 3);
    let trigger_box = base_bp
        .components
        .iter()
        .find(|c| c.name == "TriggerBox")
        .expect("TriggerBox SCS component");
    assert_eq!(
        trigger_box.origin,
        ComponentOrigin::SimpleConstructionScript
    );
    assert_eq!(
        trigger_box.parent_component.as_deref(),
        Some("DefaultSceneRoot")
    );

    let ai_bp = db
        .blueprint("/Game/BP/BP_Mock_AI")
        .expect("blueprint by path")
        .expect("BP_Mock_AI exists");
    assert_eq!(ai_bp.kind, BlueprintKind::Other);

    let children = db
        .blueprints_by_parent("BP_Mock_BaseUnit")
        .expect("blueprints_by_parent");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].class_name, "BP_Mock_FastUnit_C");

    let fast_comps = db
        .blueprint_components("BP_Mock_FastUnit_C")
        .expect("blueprint_components");
    assert_eq!(fast_comps.len(), 2);
    assert!(
        fast_comps.iter().any(|c| {
            c.name == "TriggerBox" && c.origin == ComponentOrigin::InheritableComponent
        })
    );

    // 10. `blueprint_value(class, property_path)` across inheritance chain & C++ default reporting
    assert_eq!(
        db.blueprint_value("BP_Mock_FastUnit", "RunSpeed")
            .expect("blueprint_value child override"),
        BlueprintPropertyLookup::Overridden {
            defined_in_path: "/Game/BP/BP_Mock_FastUnit".to_owned(),
            defined_in_class: "BP_Mock_FastUnit_C".to_owned(),
            value: json!(950.0),
            chain: vec!["BP_Mock_FastUnit_C".to_owned()],
        }
    );
    assert_eq!(
        db.blueprint_value("BP_Mock_FastUnit_C", "WalkSpeed")
            .expect("blueprint_value inherited override"),
        BlueprintPropertyLookup::Overridden {
            defined_in_path: "/Game/BP/BP_Mock_BaseUnit".to_owned(),
            defined_in_class: "BP_Mock_BaseUnit_C".to_owned(),
            value: json!(500.0),
            chain: vec![
                "BP_Mock_FastUnit_C".to_owned(),
                "BP_Mock_BaseUnit_C".to_owned(),
            ],
        }
    );
    assert_eq!(
        db.blueprint_value("BP_Mock_FastUnit", "CharMoveComp.MaxAcceleration")
            .expect("blueprint_value child component override"),
        BlueprintPropertyLookup::Overridden {
            defined_in_path: "/Game/BP/BP_Mock_FastUnit".to_owned(),
            defined_in_class: "BP_Mock_FastUnit_C".to_owned(),
            value: json!(18000.0),
            chain: vec!["BP_Mock_FastUnit_C".to_owned()],
        }
    );
    assert_eq!(
        db.blueprint_value("BP_Mock_FastUnit", "CharMoveComp.MaxStepHeight")
            .expect("blueprint_value parent component override"),
        BlueprintPropertyLookup::Overridden {
            defined_in_path: "/Game/BP/BP_Mock_BaseUnit".to_owned(),
            defined_in_class: "BP_Mock_BaseUnit_C".to_owned(),
            value: json!(45.0),
            chain: vec![
                "BP_Mock_FastUnit_C".to_owned(),
                "BP_Mock_BaseUnit_C".to_owned(),
            ],
        }
    );
    assert_eq!(
        db.blueprint_value("BP_Mock_FastUnit", "MovementComp.MaxStepHeight")
            .expect("blueprint_value via CDO ref alias"),
        BlueprintPropertyLookup::Overridden {
            defined_in_path: "/Game/BP/BP_Mock_BaseUnit".to_owned(),
            defined_in_class: "BP_Mock_BaseUnit_C".to_owned(),
            value: json!(45.0),
            chain: vec![
                "BP_Mock_FastUnit_C".to_owned(),
                "BP_Mock_BaseUnit_C".to_owned(),
            ],
        }
    );
    assert_eq!(
        db.blueprint_value("BP_Mock_FastUnit", "Offset.Z")
            .expect("blueprint_value nested struct"),
        BlueprintPropertyLookup::Overridden {
            defined_in_path: "/Game/BP/BP_Mock_BaseUnit".to_owned(),
            defined_in_class: "BP_Mock_BaseUnit_C".to_owned(),
            value: json!(10.0),
            chain: vec![
                "BP_Mock_FastUnit_C".to_owned(),
                "BP_Mock_BaseUnit_C".to_owned(),
            ],
        }
    );
    match db
        .blueprint_value("BP_Mock_FastUnit", "GravityScale")
        .expect("blueprint_value C++ default")
    {
        BlueprintPropertyLookup::NotOverriddenInBlueprint {
            chain,
            cpp_parent,
            reason,
        } => {
            assert_eq!(chain, vec!["BP_Mock_FastUnit_C", "BP_Mock_BaseUnit_C"]);
            assert_eq!(cpp_parent.as_deref(), Some("CPP_Mock_Unit"));
            assert!(
                reason.contains("蓝图没改，值在 C++ 默认里"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected NotOverriddenInBlueprint, got: {other:?}"),
    }
    assert!(matches!(
        db.blueprint_value("BP_DoesNotExist", "WalkSpeed")
            .expect("missing bp"),
        BlueprintPropertyLookup::BlueprintNotFound { .. }
    ));

    // 11. `widget_tree(blueprint)` nested hierarchy & `widgets_using_class(class)`
    let entry_tree = db
        .widget_tree("WBP_Mock_Entry")
        .expect("widget_tree WBP_Mock_Entry")
        .expect("tree exists");
    assert_eq!(entry_tree.blueprint_class, "WBP_Mock_Entry_C");
    assert_eq!(entry_tree.node_count(), 6);
    let root = entry_tree.root.as_ref().expect("root widget");
    assert_eq!(root.node.name, "RootOverlay");
    assert_eq!(root.node.class, "Overlay");
    assert_eq!(root.children.len(), 2);

    assert_eq!(root.children[0].node.name, "ContentRow");
    assert_eq!(
        root.children[0].node.slot_class.as_deref(),
        Some("OverlaySlot")
    );
    assert_eq!(
        root.children[0].node.slot_properties.get("Padding"),
        Some(&json!({"Left": 8.0, "Right": 8.0}))
    );
    assert_eq!(root.children[0].children.len(), 2);
    assert_eq!(root.children[0].children[0].node.name, "ItemIcon");
    assert_eq!(root.children[0].children[0].node.class, "WBP_Mock_Icon_C");
    assert_eq!(root.children[0].children[1].node.name, "LabelText");
    assert_eq!(root.children[0].children[1].node.class, "CommonTextBlock");

    assert_eq!(root.children[1].node.name, "BadgeIcon");
    assert_eq!(root.children[1].children.len(), 1);
    assert_eq!(root.children[1].children[0].node.name, "BadgeBorder");
    assert_eq!(
        root.children[1].children[0].node.slot_class.as_deref(),
        Some("NamedSlot")
    );
    assert_eq!(
        root.children[1].children[0].node.slot_name.as_deref(),
        Some("Slot_Badge")
    );

    let wrapper_tree = db
        .widget_tree("WBP_Mock_Wrapper_C")
        .expect("widget_tree wrapper")
        .expect("wrapper exists");
    assert_eq!(
        wrapper_tree.root.as_ref().expect("wrapper root").node.class,
        "WBP_Mock_Entry_C"
    );

    let icon_usages = db
        .widgets_using_class("WBP_Mock_Icon")
        .expect("widgets_using_class without _C");
    assert_eq!(icon_usages.len(), 2);
    assert_eq!(icon_usages[0].blueprint_class, "WBP_Mock_Entry_C");
}

#[test]
fn all_input_ui_queries_use_indexes() {
    let test_db =
        build_fixture_db_stages("input_ui", Some(vec![Stage::InputUi])).expect("build input_ui db");
    let db = test_db.db();

    assert_db_uses_index(
        db,
        sql::INPUT_ACTION_BY_ID,
        ["IA_Mock_Move"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::INPUT_ACTION_BY_PATH,
        ["/Game/Input/Action/IA_Mock_Move"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::INPUT_ACTION_ID_BY_PATH,
        ["/Game/Input/Action/IA_Mock_Move"],
        ALL_INPUT_UI_TABLES,
    );

    assert_db_uses_index(
        db,
        sql::INPUT_CONTEXT_BY_ID,
        ["IMC_Mock_Field_KBM"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::INPUT_CONTEXT_BY_PATH,
        ["/Game/Input/Mapping/IMC_Mock_Field_KBM"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::INPUT_CONTEXT_ID_BY_PATH,
        ["/Game/Input/Mapping/IMC_Mock_Field_KBM"],
        ALL_INPUT_UI_TABLES,
    );

    assert_db_uses_index(
        db,
        sql::INPUT_BINDINGS_BY_CONTEXT,
        ["IMC_Mock_Field_KBM"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::KEYS_FOR_ACTION_ALL_DEVICES,
        ["IA_Mock_Confirm"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::KEYS_FOR_ACTION_BY_DEVICE,
        params!["IA_Mock_Confirm", "KeyboardAndMouse"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::CONTEXTS_FOR_ACTION,
        ["IA_Mock_Confirm"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(db, sql::ACTIONS_FOR_KEY, ["Enter"], ALL_INPUT_UI_TABLES);
    assert_db_uses_index(
        db,
        sql::INPUT_SET_ACTIONS_BY_SEMANTIC,
        ["FieldConfirm"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::INPUT_SET_ACTIONS_BY_ACTION,
        ["IA_Mock_Confirm"],
        ALL_INPUT_UI_TABLES,
    );

    assert_db_uses_index(
        db,
        sql::BLUEPRINT_BY_PATH,
        ["/Game/BP/BP_Mock_BaseUnit"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::BLUEPRINT_BY_CLASS,
        ["BP_Mock_BaseUnit_C"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::BLUEPRINTS_BY_PARENT,
        ["BP_Mock_BaseUnit_C"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::BLUEPRINT_META_BY_PATH,
        ["/Game/UI/WBP_Mock_Entry"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::BLUEPRINT_META_BY_CLASS,
        ["WBP_Mock_Entry_C"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::BLUEPRINT_COMPONENTS_BY_BP,
        ["/Game/BP/BP_Mock_BaseUnit"],
        ALL_INPUT_UI_TABLES,
    );

    assert_db_uses_index(
        db,
        sql::WIDGET_NODES_BY_BLUEPRINT,
        ["/Game/UI/WBP_Mock_Entry"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::WIDGETS_USING_CLASS,
        ["WBP_Mock_Icon_C"],
        ALL_INPUT_UI_TABLES,
    );

    assert_db_uses_index(
        db,
        sql::DATA_ASSET_BY_PATH,
        ["/Game/Field/Data/DA_Mock_Config"],
        ALL_INPUT_UI_TABLES,
    );
    assert_db_uses_index(
        db,
        sql::DATA_ASSETS_BY_CLASS,
        ["CPP_DA_MockConfig"],
        ALL_INPUT_UI_TABLES,
    );
}

#[test]
fn real_export_input_ui_spot_checks_if_deps_present() {
    let full_dir = std::env::var_os("FFR_DEPS")
        .filter(|v| !v.is_empty())
        .map(|d| PathBuf::from(d).join("ffr/export-full"))
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.deps/ffr/export-full")
        });
    if !full_dir.join("FFRS/Content").is_dir() {
        eprintln!(
            "skipping real_export_input_ui_spot_checks: {} not present",
            full_dir.display()
        );
        return;
    }

    let test_db = build_temp_db(
        "input_ui_real",
        BuildInputs {
            full: Some(full_dir),
            ..BuildInputs::default()
        },
        Some(vec![Stage::InputUi]),
    )
    .expect("build input_ui from real export-full");
    let db = test_db.db();

    // 1. Battle KBM mappings
    let battle_kbm = db
        .input_bindings("IMC_Battle_Default_KBM")
        .expect("IMC_Battle_Default_KBM");
    assert!(
        battle_kbm
            .iter()
            .any(|m| m.action_id == "IA_Battle_AutoBattle" && m.key == "T")
    );
    assert!(
        battle_kbm
            .iter()
            .any(|m| m.action_id == "IA_Battle_ChangeGameSpeed" && m.key == "Tab")
    );

    // 2. BP_MapUnit_Player CharMoveComp.MaxAcceleration = 15000.0
    assert_eq!(
        db.blueprint_value("BP_MapUnit_Player", "CharMoveComp.MaxAcceleration")
            .expect("BP_MapUnit_Player MaxAcceleration"),
        BlueprintPropertyLookup::Overridden {
            defined_in_path: "/Game/BP/Map/Unit/BP_MapUnit_Player".to_owned(),
            defined_in_class: "BP_MapUnit_Player_C".to_owned(),
            value: json!(15000.0),
            chain: vec!["BP_MapUnit_Player_C".to_owned()],
        }
    );

    // 3. WBP_BtlCmd_ItemEntry root is WBP_InventoryItemEntry_C
    let tree = db
        .widget_tree("WBP_BtlCmd_ItemEntry")
        .expect("WBP_BtlCmd_ItemEntry")
        .expect("widget tree exists");
    let root = tree.root.as_ref().expect("root widget");
    assert_eq!(root.node.class, "WBP_InventoryItemEntry_C");
}

#[test]
fn parsers_handle_widget_tree_named_slots_expandable_area_and_malformed_inputs() {
    use ffr_data::assets::input::{normalize_input_device, parse_input_mapping_context};
    use ffr_data::assets::widget::parse_widget_tree;

    assert_eq!(
        normalize_input_device("eMuchaInputDeviceType::KeyboardAndMouse"),
        "KeyboardAndMouse"
    );
    assert_eq!(normalize_input_device("kbm"), "KeyboardAndMouse");
    assert_eq!(normalize_input_device("pad"), "Gamepad");

    // Malformed IMC (missing Key) returns Err rather than panicking.
    let bad_imc = json!([
        {
            "Type": "InputMappingContext",
            "Name": "IMC_Bad",
            "Package": "/Game/Input/IMC_Bad",
            "Properties": {
                "Mappings": [
                    {
                        "Action": { "ObjectName": "InputAction'IA_X'", "ObjectPath": "/Game/Input/IA_X.0" }
                    }
                ]
            }
        }
    ]);
    assert!(parse_input_mapping_context(&bad_imc, "IMC_Bad.json").is_err());

    // Derived WBP with top-level WidgetTree.NamedSlotBindings + ExpandableArea Header/Body.
    let derived_wbp = json!([
        {
            "Type": "WidgetBlueprintGeneratedClass",
            "Name": "WBP_Derived_C",
            "Package": "/Game/UI/WBP_Derived",
            "Properties": {
                "WidgetTree": { "ObjectName": "WidgetTree'WBP_Derived_C:WidgetTree'", "ObjectPath": "/Game/UI/WBP_Derived.1" }
            }
        },
        {
            "Type": "WidgetTree",
            "Name": "WidgetTree",
            "Outer": { "ObjectName": "WidgetBlueprintGeneratedClass'WBP_Derived_C'", "ObjectPath": "/Game/UI/WBP_Derived.0" },
            "Properties": {
                "NamedSlotBindings": [
                    {
                        "Key": "Slot_Main",
                        "Value": { "ObjectName": "ExpandableArea'WBP_Derived_C:WidgetTree.Area_0'", "ObjectPath": "/Game/UI/WBP_Derived.2" }
                    }
                ]
            }
        },
        {
            "Type": "ExpandableArea",
            "Name": "Area_0",
            "Outer": { "ObjectName": "WidgetTree'WBP_Derived_C:WidgetTree'", "ObjectPath": "/Game/UI/WBP_Derived.1" },
            "Properties": {
                "HeaderContent": { "ObjectName": "TextBlock'WBP_Derived_C:WidgetTree.HeaderText'", "ObjectPath": "/Game/UI/WBP_Derived.3" },
                "BodyContent": { "ObjectName": "TextBlock'WBP_Derived_C:WidgetTree.BodyText'", "ObjectPath": "/Game/UI/WBP_Derived.4" }
            }
        },
        {
            "Type": "TextBlock",
            "Name": "HeaderText",
            "Outer": { "ObjectName": "WidgetTree'WBP_Derived_C:WidgetTree'", "ObjectPath": "/Game/UI/WBP_Derived.1" }
        },
        {
            "Type": "TextBlock",
            "Name": "BodyText",
            "Outer": { "ObjectName": "WidgetTree'WBP_Derived_C:WidgetTree'", "ObjectPath": "/Game/UI/WBP_Derived.1" }
        }
    ]);
    let tree = parse_widget_tree(&derived_wbp, "WBP_Derived.json")
        .expect("parse derived wbp")
        .expect("widget tree present");
    assert!(tree.root.is_none());
    assert_eq!(tree.extra_roots.len(), 1);
    let area = &tree.extra_roots[0];
    assert_eq!(area.node.name, "Area_0");
    assert_eq!(area.node.slot_class.as_deref(), Some("WidgetTreeNamedSlot"));
    assert_eq!(area.node.slot_name.as_deref(), Some("Slot_Main"));
    assert_eq!(area.children.len(), 2);
    assert_eq!(area.children[0].node.name, "HeaderText");
    assert_eq!(area.children[1].node.name, "BodyText");
}
