//! Typed loaders for shops (`Shop/DT_ShopList`, `Shop/DT_VariousShopsList`) and inns
//! (`INN/DT_InnList`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::datatable::{DataTable, load_data_table};
use crate::error::Result;
use crate::ftext::FTextValue;
use crate::paths::datatable_dir;
use crate::world::parse::{
    expect_array, expect_f64, expect_ftext, expect_i64, expect_row_object, expect_row_struct,
    expect_string, non_negative_id,
};

/// Relative path of `DT_ShopList` under `FFRS/Content/Datatable/`.
pub const SHOP_LIST_TABLE: &str = "Shop/DT_ShopList";
/// Relative path of `DT_VariousShopsList` under `FFRS/Content/Datatable/`.
pub const VARIOUS_SHOPS_TABLE: &str = "Shop/DT_VariousShopsList";
/// Relative path of `DT_InnList` under `FFRS/Content/Datatable/`.
pub const INN_LIST_TABLE: &str = "INN/DT_InnList";

/// One purchasable item slot inside a shop (`SHOP_ShopList1.ItemList[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopItem {
    pub item_id: i64,
    pub condition: String,
    /// `None` when unlimited (`MaxOrderNum == -1`).
    pub max_order_num: Option<i64>,
    pub price_ratio: f64,
}

impl ShopItem {
    /// Compute the effective purchase price from an item's base `buying_price`.
    pub fn effective_price(&self, base_buying_price: i64) -> i64 {
        ((base_buying_price as f64) * self.price_ratio).round() as i64
    }
}

/// One special buy-back entry inside a shop (`SHOP_ShopList1.SellItemList[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopSellItem {
    pub item_id: i64,
    pub price_ratio: f64,
}

/// One single-category or combined shop from `Shop/DT_ShopList.json` (`SHOP_ShopList1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shop {
    pub id: i64,
    pub row_key: String,
    pub name: FTextValue,
    pub items: Vec<ShopItem>,
    pub sell_items: Vec<ShopSellItem>,
}

/// One multi-shop counter bundle from `Shop/DT_VariousShopsList.json`
/// (`SHOP_VariousShopsEntryInfo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariousShop {
    pub id: i64,
    pub row_key: String,
    pub tool_shop_id: Option<i64>,
    pub weapon_shop_id: Option<i64>,
    pub armor_shop_id: Option<i64>,
    pub accessory_shop_id: Option<i64>,
}

impl VariousShop {
    /// Return all active sub-shop IDs (`DT_ShopList.ShopID`) grouped by this counter.
    pub fn shop_ids(&self) -> Vec<i64> {
        [
            self.tool_shop_id,
            self.weapon_shop_id,
            self.armor_shop_id,
            self.accessory_shop_id,
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// Inn lodging price record from `INN/DT_InnList.json` (`INN_Data`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inn {
    pub id: i64,
    pub row_key: String,
    pub price: i64,
}

/// Parse `Shop/DT_ShopList` into typed [`Shop`] records sorted by `id` ascending.
pub fn parse_shops(table: &DataTable, source_label: &str) -> Result<Vec<Shop>> {
    expect_row_struct(table, "SHOP_ShopList1", source_label)?;
    let mut shops = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;

        let raw_items = expect_array(row, "ItemList", source_label, &row_prefix)?;
        let mut items = Vec::new();
        for (idx, item) in raw_items.iter().enumerate() {
            let item_prefix = format!("{row_prefix}.ItemList[{idx}]");
            let obj = expect_row_object(item, source_label, &item_prefix)?;
            let item_id = expect_i64(obj, "ItemId", source_label, &item_prefix)?;
            let condition = expect_string(obj, "Condition", source_label, &item_prefix)?;
            let max_order_num =
                non_negative_id(expect_i64(obj, "MaxOrderNum", source_label, &item_prefix)?);
            let price_ratio = expect_f64(obj, "PriceRatio", source_label, &item_prefix)?;
            if item_id >= 0 {
                items.push(ShopItem {
                    item_id,
                    condition,
                    max_order_num,
                    price_ratio,
                });
            }
        }

        let raw_sell_items = expect_array(row, "SellItemList", source_label, &row_prefix)?;
        let mut sell_items = Vec::new();
        for (idx, item) in raw_sell_items.iter().enumerate() {
            let item_prefix = format!("{row_prefix}.SellItemList[{idx}]");
            let obj = expect_row_object(item, source_label, &item_prefix)?;
            let item_id = expect_i64(obj, "ItemId", source_label, &item_prefix)?;
            let price_ratio = expect_f64(obj, "PriceRatio", source_label, &item_prefix)?;
            if item_id >= 0 {
                sell_items.push(ShopSellItem {
                    item_id,
                    price_ratio,
                });
            }
        }

        shops.push(Shop {
            id: expect_i64(row, "ShopID", source_label, &row_prefix)?,
            row_key: row_key.clone(),
            name: expect_ftext(row, "ShopName", source_label, &row_prefix)?,
            items,
            sell_items,
        });
    }

    shops.sort_by_key(|shop| shop.id);
    Ok(shops)
}

/// Load `Shop/DT_ShopList.json` from `export_root`.
pub fn load_shops(export_root: impl AsRef<Path>) -> Result<Vec<Shop>> {
    let path = datatable_dir(export_root)
        .join("Shop")
        .join("DT_ShopList.json");
    let table = load_data_table(&path)?;
    parse_shops(&table, &path.display().to_string())
}

/// Parse `Shop/DT_VariousShopsList` into typed [`VariousShop`] records sorted by `id` ascending.
pub fn parse_various_shops(table: &DataTable, source_label: &str) -> Result<Vec<VariousShop>> {
    expect_row_struct(table, "SHOP_VariousShopsEntryInfo", source_label)?;
    let mut bundles = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        bundles.push(VariousShop {
            id: expect_i64(row, "ID", source_label, &row_prefix)?,
            row_key: row_key.clone(),
            tool_shop_id: non_negative_id(expect_i64(
                row,
                "ToolShopID",
                source_label,
                &row_prefix,
            )?),
            weapon_shop_id: non_negative_id(expect_i64(
                row,
                "WeaponShopID",
                source_label,
                &row_prefix,
            )?),
            armor_shop_id: non_negative_id(expect_i64(
                row,
                "ArmorShopID",
                source_label,
                &row_prefix,
            )?),
            accessory_shop_id: non_negative_id(expect_i64(
                row,
                "AccessoryShopID",
                source_label,
                &row_prefix,
            )?),
        });
    }

    bundles.sort_by_key(|bundle| bundle.id);
    Ok(bundles)
}

/// Load `Shop/DT_VariousShopsList.json` from `export_root`.
pub fn load_various_shops(export_root: impl AsRef<Path>) -> Result<Vec<VariousShop>> {
    let path = datatable_dir(export_root)
        .join("Shop")
        .join("DT_VariousShopsList.json");
    let table = load_data_table(&path)?;
    parse_various_shops(&table, &path.display().to_string())
}

/// Parse `INN/DT_InnList` into typed [`Inn`] records sorted by `id` ascending.
pub fn parse_inns(table: &DataTable, source_label: &str) -> Result<Vec<Inn>> {
    expect_row_struct(table, "INN_Data", source_label)?;
    let mut inns = Vec::with_capacity(table.rows.len());

    for (row_key, raw_row) in &table.rows {
        let row_prefix = format!("Rows.{row_key}");
        let row = expect_row_object(raw_row, source_label, &row_prefix)?;
        inns.push(Inn {
            id: expect_i64(row, "ID", source_label, &row_prefix)?,
            row_key: row_key.clone(),
            price: expect_i64(row, "Price", source_label, &row_prefix)?,
        });
    }

    inns.sort_by_key(|inn| inn.id);
    Ok(inns)
}

/// Load `INN/DT_InnList.json` from `export_root`.
pub fn load_inns(export_root: impl AsRef<Path>) -> Result<Vec<Inn>> {
    let path = datatable_dir(export_root)
        .join("INN")
        .join("DT_InnList.json");
    let table = load_data_table(&path)?;
    parse_inns(&table, &path.display().to_string())
}
