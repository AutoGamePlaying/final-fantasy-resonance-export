//! Typed loaders for the 6 item tables sharing the global `ItemId` namespace.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::datatable::{DataTable, load_data_table};
use crate::error::{DataError, Result};
use crate::ftext::FTextValue;
use crate::paths::datatable_dir;
use crate::world::parse::{
    expect_array, expect_bool, expect_ftext, expect_i64, expect_object, expect_row_object,
    expect_row_struct, expect_string, non_negative_id,
};

/// Relative path of `DT_ConsumablesItemData` under `FFRS/Content/Datatable/`.
pub const CONSUMABLES_ITEM_TABLE: &str = "Item/DT_ConsumablesItemData";
/// Relative path of `DT_SeedItemData` under `FFRS/Content/Datatable/`.
pub const SEED_ITEM_TABLE: &str = "Item/DT_SeedItemData";
/// Relative path of `DT_EquipmentItemData` under `FFRS/Content/Datatable/`.
pub const EQUIPMENT_ITEM_TABLE: &str = "Item/DT_EquipmentItemData";
/// Relative path of `DT_VisionItemData` under `FFRS/Content/Datatable/`.
pub const VISION_ITEM_TABLE: &str = "Item/Vision/DT_VisionItemData";
/// Relative path of `DT_ValuableItemData` under `FFRS/Content/Datatable/`.
pub const VALUABLE_ITEM_TABLE: &str = "Item/DT_ValuableItemData";
/// Relative path of `DT_ArtifactItemData` under `FFRS/Content/Datatable/`.
pub const ARTIFACT_ITEM_TABLE: &str = "Item/DT_ArtifactItemData";

/// High-level item table category across the 6 disjoint `ItemId` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemCategory {
    Consumable,
    Seed,
    Equipment,
    Vision,
    Valuable,
    Artifact,
}

/// One stat modifier on an equipment item (`EQUIPMENT_ITEM_DATA.params[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentParam {
    pub param_type: String,
    pub value: i64,
}

/// One seed harvest recipe entry (`SEED_ITEM_DATA.harvestList[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedHarvestRecipe {
    pub combine_item_id: i64,
    pub create_item_id: i64,
}

/// Category-specific effect and parameter fields of an [`Item`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ItemDetail {
    #[serde(rename_all = "camelCase")]
    Consumable {
        usable_scene: String,
        is_consumed_after_use: bool,
        skill_id: Option<i64>,
        is_throwable: bool,
        throw_skill_id: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    Seed {
        harvest_recipes: Vec<SeedHarvestRecipe>,
    },
    #[serde(rename_all = "camelCase")]
    Equipment {
        equipment_type: String,
        detail_type: String,
        limit_sex: String,
        params: Vec<EquipmentParam>,
        passive_skill_ids: Vec<i64>,
    },
    #[serde(rename_all = "camelCase")]
    Vision {
        finish_blow_skill_id: Option<i64>,
        command_id: Option<i64>,
        attack_type: String,
        roles: Vec<String>,
    },
    Valuable,
    #[serde(rename_all = "camelCase")]
    Artifact {
        item_tag: String,
        effect_values: Vec<f64>,
        is_initial_active: bool,
    },
}

/// Unified item record across all 6 item tables (`Consumables`, `Seed`, `Equipment`, `Vision`,
/// `Valuable`, `Artifact`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: i64,
    pub row_key: String,
    pub sort_id: i64,
    pub category: ItemCategory,
    pub raw_item_type: String,
    pub name: FTextValue,
    pub description: FTextValue,
    pub buying_price: i64,
    pub selling_price: i64,
    pub is_available_sale: bool,
    pub detail: ItemDetail,
}

struct CommonItemFields {
    id: i64,
    sort_id: i64,
    raw_item_type: String,
    name: FTextValue,
    description: FTextValue,
    buying_price: i64,
    selling_price: i64,
    is_available_sale: bool,
}

fn parse_common_fields(
    row: &Map<String, Value>,
    source_label: &str,
    row_prefix: &str,
) -> Result<CommonItemFields> {
    Ok(CommonItemFields {
        id: expect_i64(row, "ID", source_label, row_prefix)?,
        sort_id: expect_i64(row, "SortId", source_label, row_prefix)?,
        raw_item_type: expect_string(row, "ItemType", source_label, row_prefix)?,
        name: expect_ftext(row, "Name", source_label, row_prefix)?,
        description: expect_ftext(row, "Description", source_label, row_prefix)?,
        buying_price: expect_i64(row, "buyingPrice", source_label, row_prefix)?,
        selling_price: expect_i64(row, "sellingPrice", source_label, row_prefix)?,
        is_available_sale: expect_bool(row, "isAvailableSale", source_label, row_prefix)?,
    })
}

fn build_item(
    row_key: String,
    category: ItemCategory,
    common: CommonItemFields,
    detail: ItemDetail,
) -> Item {
    Item {
        id: common.id,
        row_key,
        sort_id: common.sort_id,
        category,
        raw_item_type: common.raw_item_type,
        name: common.name,
        description: common.description,
        buying_price: common.buying_price,
        selling_price: common.selling_price,
        is_available_sale: common.is_available_sale,
        detail,
    }
}

/// Parse `Item/DT_ConsumablesItemData` (`CONSUMABLES_ITEM_DATA`).
pub fn parse_consumables(table: &DataTable, source_label: &str) -> Result<Vec<Item>> {
    expect_row_struct(table, "CONSUMABLES_ITEM_DATA", source_label)?;
    let mut items = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let common = parse_common_fields(row, source_label, &row_prefix)?;

        let throw_prefix = format!("{row_prefix}.throwSkillData");
        let throw_obj = expect_object(row, "throwSkillData", source_label, &row_prefix)?;
        let detail = ItemDetail::Consumable {
            usable_scene: expect_string(row, "usableScene", source_label, &row_prefix)?,
            is_consumed_after_use: expect_bool(
                row,
                "isConsumedAfterUse",
                source_label,
                &row_prefix,
            )?,
            skill_id: non_negative_id(expect_i64(row, "SkillId", source_label, &row_prefix)?),
            is_throwable: expect_bool(throw_obj, "isThrow", source_label, &throw_prefix)?,
            throw_skill_id: non_negative_id(expect_i64(
                throw_obj,
                "SkillId",
                source_label,
                &throw_prefix,
            )?),
        };
        items.push(build_item(
            row_key.clone(),
            ItemCategory::Consumable,
            common,
            detail,
        ));
    }

    items.sort_by_key(|item| item.id);
    Ok(items)
}

/// Parse `Item/DT_SeedItemData` (`SEED_ITEM_DATA`).
pub fn parse_seeds(table: &DataTable, source_label: &str) -> Result<Vec<Item>> {
    expect_row_struct(table, "SEED_ITEM_DATA", source_label)?;
    let mut items = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let common = parse_common_fields(row, source_label, &row_prefix)?;
        let raw_harvests = expect_array(row, "harvestList", source_label, &row_prefix)?;
        let mut harvest_recipes = Vec::with_capacity(raw_harvests.len());
        for (idx, entry) in raw_harvests.iter().enumerate() {
            let entry_prefix = format!("{row_prefix}.harvestList[{idx}]");
            let obj = expect_row_object(entry, source_label, &entry_prefix)?;
            harvest_recipes.push(SeedHarvestRecipe {
                combine_item_id: expect_i64(obj, "combineItemId", source_label, &entry_prefix)?,
                create_item_id: expect_i64(obj, "createItemId", source_label, &entry_prefix)?,
            });
        }
        items.push(build_item(
            row_key.clone(),
            ItemCategory::Seed,
            common,
            ItemDetail::Seed { harvest_recipes },
        ));
    }

    items.sort_by_key(|item| item.id);
    Ok(items)
}

/// Parse `Item/DT_EquipmentItemData` (`EQUIPMENT_ITEM_DATA`).
pub fn parse_equipment(table: &DataTable, source_label: &str) -> Result<Vec<Item>> {
    expect_row_struct(table, "EQUIPMENT_ITEM_DATA", source_label)?;
    let mut items = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let common = parse_common_fields(row, source_label, &row_prefix)?;

        let raw_params = expect_array(row, "params", source_label, &row_prefix)?;
        let mut params = Vec::with_capacity(raw_params.len());
        for (idx, entry) in raw_params.iter().enumerate() {
            let entry_prefix = format!("{row_prefix}.params[{idx}]");
            let obj = expect_row_object(entry, source_label, &entry_prefix)?;
            params.push(EquipmentParam {
                param_type: expect_string(obj, "Key", source_label, &entry_prefix)?,
                value: expect_i64(obj, "Value", source_label, &entry_prefix)?,
            });
        }

        let raw_passives = expect_array(row, "passiveList", source_label, &row_prefix)?;
        let mut passive_skill_ids = Vec::new();
        for (idx, entry) in raw_passives.iter().enumerate() {
            let pid = entry.as_i64().ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.passiveList[{idx}]"),
                    "不是整数",
                )
            })?;
            if let Some(valid_id) = non_negative_id(pid) {
                passive_skill_ids.push(valid_id);
            }
        }

        let detail = ItemDetail::Equipment {
            equipment_type: expect_string(row, "EquipmentType", source_label, &row_prefix)?,
            detail_type: expect_string(row, "equipmentDetailType", source_label, &row_prefix)?,
            limit_sex: expect_string(row, "equipmentLimitSex", source_label, &row_prefix)?,
            params,
            passive_skill_ids,
        };
        items.push(build_item(
            row_key.clone(),
            ItemCategory::Equipment,
            common,
            detail,
        ));
    }

    items.sort_by_key(|item| item.id);
    Ok(items)
}

/// Parse `Item/Vision/DT_VisionItemData` (`VISION_ITEM_DATA`).
pub fn parse_visions(table: &DataTable, source_label: &str) -> Result<Vec<Item>> {
    expect_row_struct(table, "VISION_ITEM_DATA", source_label)?;
    let mut items = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let common = parse_common_fields(row, source_label, &row_prefix)?;

        let raw_roles = expect_array(row, "roles", source_label, &row_prefix)?;
        let mut roles = Vec::with_capacity(raw_roles.len());
        for (idx, entry) in raw_roles.iter().enumerate() {
            let role = entry.as_str().ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.roles[{idx}]"),
                    "不是字符串",
                )
            })?;
            roles.push(role.to_owned());
        }

        let detail = ItemDetail::Vision {
            finish_blow_skill_id: non_negative_id(expect_i64(
                row,
                "finishBlowSkill",
                source_label,
                &row_prefix,
            )?),
            command_id: non_negative_id(expect_i64(row, "CommandId", source_label, &row_prefix)?),
            attack_type: expect_string(row, "attackType", source_label, &row_prefix)?,
            roles,
        };
        items.push(build_item(
            row_key.clone(),
            ItemCategory::Vision,
            common,
            detail,
        ));
    }

    items.sort_by_key(|item| item.id);
    Ok(items)
}

/// Parse `Item/DT_ValuableItemData` (`VALUABLE_ITEM_DATA`).
pub fn parse_valuables(table: &DataTable, source_label: &str) -> Result<Vec<Item>> {
    expect_row_struct(table, "VALUABLE_ITEM_DATA", source_label)?;
    let mut items = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let common = parse_common_fields(row, source_label, &row_prefix)?;
        items.push(build_item(
            row_key.clone(),
            ItemCategory::Valuable,
            common,
            ItemDetail::Valuable,
        ));
    }

    items.sort_by_key(|item| item.id);
    Ok(items)
}

/// Parse `Item/DT_ArtifactItemData` (`ARTIFACT_ITEM_DATA`).
pub fn parse_artifacts(table: &DataTable, source_label: &str) -> Result<Vec<Item>> {
    expect_row_struct(table, "ARTIFACT_ITEM_DATA", source_label)?;
    let mut items = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        let common = parse_common_fields(row, source_label, &row_prefix)?;

        let tag_prefix = format!("{row_prefix}.ItemTag");
        let tag_obj = expect_object(row, "ItemTag", source_label, &row_prefix)?;
        let item_tag = expect_string(tag_obj, "TagName", source_label, &tag_prefix)?;

        let raw_values = expect_array(row, "EffectValueList", source_label, &row_prefix)?;
        let mut effect_values = Vec::with_capacity(raw_values.len());
        for (idx, val) in raw_values.iter().enumerate() {
            let num = val.as_f64().ok_or_else(|| {
                DataError::invalid_schema(
                    source_label,
                    format!("{row_prefix}.EffectValueList[{idx}]"),
                    "不是数值",
                )
            })?;
            effect_values.push(num);
        }

        let detail = ItemDetail::Artifact {
            item_tag,
            effect_values,
            is_initial_active: expect_bool(row, "isInitialActive", source_label, &row_prefix)?,
        };
        items.push(build_item(
            row_key.clone(),
            ItemCategory::Artifact,
            common,
            detail,
        ));
    }

    items.sort_by_key(|item| item.id);
    Ok(items)
}

/// Load and merge all 6 global `ItemId` tables from `export_root`, sorted by `id` ascending.
pub fn load_all_items(export_root: impl AsRef<Path>) -> Result<Vec<Item>> {
    let base = datatable_dir(export_root).join("Item");
    let consumables_path = base.join("DT_ConsumablesItemData.json");
    let seeds_path = base.join("DT_SeedItemData.json");
    let equipment_path = base.join("DT_EquipmentItemData.json");
    let visions_path = base.join("Vision").join("DT_VisionItemData.json");
    let valuables_path = base.join("DT_ValuableItemData.json");
    let artifacts_path = base.join("DT_ArtifactItemData.json");

    let mut all = Vec::new();
    all.extend(parse_consumables(
        &load_data_table(&consumables_path)?,
        &consumables_path.display().to_string(),
    )?);
    all.extend(parse_seeds(
        &load_data_table(&seeds_path)?,
        &seeds_path.display().to_string(),
    )?);
    all.extend(parse_equipment(
        &load_data_table(&equipment_path)?,
        &equipment_path.display().to_string(),
    )?);
    all.extend(parse_visions(
        &load_data_table(&visions_path)?,
        &visions_path.display().to_string(),
    )?);
    all.extend(parse_valuables(
        &load_data_table(&valuables_path)?,
        &valuables_path.display().to_string(),
    )?);
    all.extend(parse_artifacts(
        &load_data_table(&artifacts_path)?,
        &artifacts_path.display().to_string(),
    )?);
    all.sort_by_key(|item| item.id);
    Ok(all)
}
