//! World data layer: maps, regions/islands, save points & world-map teleport landmarks, treasure
//! boxes, items & equipment, side quests & quest board, shops & inns.
//!
//! Provides typed loaders (`load_*` / `parse_*`), an indexed [`WorldData`] facade supporting ID,
//! `zh-Hans` display-name, and runtime `level_name` (`*_PL`) reverse lookups, and model-facing
//! Chinese summary helpers (`map_summary`, `item_summary`, `quest_summary`).

mod items;
mod maps;
mod parse;
mod quests;
mod shops;
mod summary;
pub mod talk;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::Result;
use crate::ftext::{FTextValue, ResolvedText, TextResolver, TextResolverOptions};
use crate::paths::datatable_relative_path_to_json;
use crate::scenario::{
    Chapter, MainScenarioEntry, get_chapter_main_scenario, load_chapters, load_main_scenario,
};
use crate::store::DataStore;

pub use items::{
    ARTIFACT_ITEM_TABLE, CONSUMABLES_ITEM_TABLE, EQUIPMENT_ITEM_TABLE, EquipmentParam, Item,
    ItemCategory, ItemDetail, SEED_ITEM_TABLE, SeedHarvestRecipe, VALUABLE_ITEM_TABLE,
    VISION_ITEM_TABLE, load_all_items, parse_artifacts, parse_consumables, parse_equipment,
    parse_seeds, parse_valuables, parse_visions,
};
pub use maps::{
    MAP_DATA_TABLE, MAP_ISLAND_TABLE, MAP_LANDMARK_ASSET, MAP_REGION_TABLE,
    MAP_RELEASE_CONDITION_TABLE, MapFacility, MapInfo, MapIsland, MapRegion, MapReleaseCondition,
    MapStartupEvent, SAVE_POINT_COMMAND_TABLE, SAVE_POINT_DATA_TABLE, SavePoint, SavePointCommand,
    TREASURE_BOX_TABLE, TeleportPoint, TeleportTarget, TreasureBox, extract_level_name,
    load_map_islands, load_map_regions, load_map_release_conditions, load_maps,
    load_save_point_commands, load_save_points, load_teleport_points, load_treasure_boxes,
    parse_map_islands, parse_map_regions, parse_map_release_conditions, parse_maps,
    parse_save_point_commands, parse_save_points, parse_teleport_points_document,
    parse_treasure_boxes,
};
pub use parse::WorldPoint3;
pub use quests::{
    QUEST_BOARD_TABLE, Quest, QuestBoardEntry, QuestBoardReward, QuestClearCondition,
    QuestObjective, QuestPrize, REQUEST_DATA_TABLE, load_quest_board, load_quests,
    parse_quest_board, parse_quests,
};
pub use shops::{
    INN_LIST_TABLE, Inn, SHOP_LIST_TABLE, Shop, ShopItem, ShopSellItem, VARIOUS_SHOPS_TABLE,
    VariousShop, load_inns, load_shops, load_various_shops, parse_inns, parse_shops,
    parse_various_shops,
};
pub use summary::{
    item_effect_overview, item_summary, main_objective_summary, map_summary, quest_summary,
    shop_summary,
};
pub use talk::{
    OrderedDataTableRows, TALK_EVENT_DATA_TABLE, TALK_EVENT_ROW_STRUCT, TALK_LINE_ROW_STRUCT,
    TalkCustomChoiceDialog, TalkCustomChoiceItemCondition, TalkEvent, TalkEventArg,
    TalkEventChoiceBranch, TalkEventTransition, TalkLine, is_developer_asset_path,
    load_ordered_datatable_rows, load_talk_events, normalize_ue_asset_path,
    parse_ordered_datatable_json, parse_talk_event_row, parse_talk_events,
    parse_talk_lines_ordered, talk_table_relative_path,
};

/// Components needed to assemble a [`WorldData`] facade without reading from disk.
#[derive(Debug, Clone)]
pub struct WorldDataParts {
    pub maps: Vec<MapInfo>,
    pub islands: Vec<MapIsland>,
    pub regions: Vec<MapRegion>,
    pub release_conditions: Vec<MapReleaseCondition>,
    pub save_points: Vec<SavePoint>,
    pub save_point_commands: Vec<SavePointCommand>,
    pub teleport_points: Vec<TeleportPoint>,
    pub treasure_boxes: Vec<TreasureBox>,
    pub items: Vec<Item>,
    pub quests: Vec<Quest>,
    pub quest_board: Vec<QuestBoardEntry>,
    pub shops: Vec<Shop>,
    pub various_shops: Vec<VariousShop>,
    pub inns: Vec<Inn>,
    pub chapters: Vec<Chapter>,
    pub main_scenario: Vec<MainScenarioEntry>,
    pub text_resolver: TextResolver,
}

/// Indexed, typed world-data facade backed by a cached [`TextResolver`].
#[derive(Debug, Clone)]
pub struct WorldData {
    maps: Vec<MapInfo>,
    islands: Vec<MapIsland>,
    regions: Vec<MapRegion>,
    release_conditions: Vec<MapReleaseCondition>,
    save_points: Vec<SavePoint>,
    save_point_commands: Vec<SavePointCommand>,
    teleport_points: Vec<TeleportPoint>,
    treasure_boxes: Vec<TreasureBox>,
    items: Vec<Item>,
    quests: Vec<Quest>,
    quest_board: Vec<QuestBoardEntry>,
    shops: Vec<Shop>,
    various_shops: Vec<VariousShop>,
    inns: Vec<Inn>,
    chapters: Vec<Chapter>,
    main_scenario: Vec<MainScenarioEntry>,
    text_resolver: TextResolver,

    maps_by_id: HashMap<i64, usize>,
    maps_by_level_name: HashMap<String, usize>,
    teleport_by_map_id: HashMap<i64, Vec<usize>>,
    treasures_by_map_id: HashMap<i64, Vec<usize>>,
    items_by_id: HashMap<i64, usize>,
    quests_by_id: HashMap<i64, usize>,
    quest_board_by_id: HashMap<i64, usize>,
    shops_by_id: HashMap<i64, usize>,
    various_shops_by_id: HashMap<i64, usize>,
}

impl WorldData {
    /// Load all world tables from `export_root` using default text options (`"zh-Hans"`, strict).
    pub fn load(export_root: impl Into<PathBuf>) -> Result<Self> {
        Self::load_with_options(export_root, TextResolverOptions::default())
    }

    /// Load all world tables from `export_root` with custom [`TextResolverOptions`].
    pub fn load_with_options(
        export_root: impl Into<PathBuf>,
        options: TextResolverOptions,
    ) -> Result<Self> {
        let root = export_root.into();
        let parts = WorldDataParts {
            maps: load_maps(&root)?,
            islands: load_map_islands(&root)?,
            regions: load_map_regions(&root)?,
            release_conditions: load_map_release_conditions(&root)?,
            save_points: load_save_points(&root)?,
            save_point_commands: load_save_point_commands(&root)?,
            teleport_points: load_teleport_points(&root)?,
            treasure_boxes: load_treasure_boxes(&root)?,
            items: load_all_items(&root)?,
            quests: load_quests(&root)?,
            quest_board: load_quest_board(&root)?,
            shops: load_shops(&root)?,
            various_shops: load_various_shops(&root)?,
            inns: load_inns(&root)?,
            chapters: load_chapters(&root)?,
            main_scenario: load_main_scenario(&root)?,
            text_resolver: TextResolver::with_options(root, options),
        };
        Ok(Self::from_parts(parts))
    }

    /// Build [`WorldData`] through an existing [`DataStore`], reusing its DataTable and text caches.
    pub fn from_store(store: &DataStore) -> Result<Self> {
        let root = store.export_root();

        let maps = parse_from_store(store, MAP_DATA_TABLE, parse_maps)?;
        let islands = parse_from_store(store, MAP_ISLAND_TABLE, parse_map_islands)?;
        let regions = parse_from_store(store, MAP_REGION_TABLE, parse_map_regions)?;
        let release_conditions = parse_from_store(
            store,
            MAP_RELEASE_CONDITION_TABLE,
            parse_map_release_conditions,
        )?;
        let save_points = parse_from_store(store, SAVE_POINT_DATA_TABLE, parse_save_points)?;
        let save_point_commands =
            parse_from_store(store, SAVE_POINT_COMMAND_TABLE, parse_save_point_commands)?;
        let teleport_points = load_teleport_points(root)?;
        let treasure_boxes = parse_from_store(store, TREASURE_BOX_TABLE, parse_treasure_boxes)?;

        let mut items = Vec::new();
        items.extend(parse_from_store(
            store,
            CONSUMABLES_ITEM_TABLE,
            parse_consumables,
        )?);
        items.extend(parse_from_store(store, SEED_ITEM_TABLE, parse_seeds)?);
        items.extend(parse_from_store(
            store,
            EQUIPMENT_ITEM_TABLE,
            parse_equipment,
        )?);
        items.extend(parse_from_store(store, VISION_ITEM_TABLE, parse_visions)?);
        items.extend(parse_from_store(
            store,
            VALUABLE_ITEM_TABLE,
            parse_valuables,
        )?);
        items.extend(parse_from_store(
            store,
            ARTIFACT_ITEM_TABLE,
            parse_artifacts,
        )?);
        items.sort_by_key(|item| item.id);

        let quests = parse_from_store(store, REQUEST_DATA_TABLE, parse_quests)?;
        let quest_board = parse_from_store(store, QUEST_BOARD_TABLE, parse_quest_board)?;
        let shops = parse_from_store(store, SHOP_LIST_TABLE, parse_shops)?;
        let various_shops = parse_from_store(store, VARIOUS_SHOPS_TABLE, parse_various_shops)?;
        let inns = parse_from_store(store, INN_LIST_TABLE, parse_inns)?;

        let chapters = store.chapters()?;
        let main_scenario = store.main_scenario()?;

        Ok(Self::from_parts(WorldDataParts {
            maps,
            islands,
            regions,
            release_conditions,
            save_points,
            save_point_commands,
            teleport_points,
            treasure_boxes,
            items,
            quests,
            quest_board,
            shops,
            various_shops,
            inns,
            chapters,
            main_scenario,
            text_resolver: store.text_resolver().clone(),
        }))
    }

    /// Assemble [`WorldData`] from already-parsed components and build all lookup indices.
    pub fn from_parts(parts: WorldDataParts) -> Self {
        let mut maps_by_id = HashMap::with_capacity(parts.maps.len());
        let mut maps_by_level_name = HashMap::with_capacity(parts.maps.len());
        for (idx, map) in parts.maps.iter().enumerate() {
            maps_by_id.insert(map.id, idx);
            maps_by_level_name.insert(map.level_name.clone(), idx);
        }

        let mut teleport_by_map_id: HashMap<i64, Vec<usize>> = HashMap::new();
        for (idx, point) in parts.teleport_points.iter().enumerate() {
            for map_id in point.target_map_ids() {
                let bucket = teleport_by_map_id.entry(map_id).or_default();
                if !bucket.contains(&idx) {
                    bucket.push(idx);
                }
            }
        }

        let mut treasures_by_map_id: HashMap<i64, Vec<usize>> = HashMap::new();
        for (idx, chest) in parts.treasure_boxes.iter().enumerate() {
            treasures_by_map_id
                .entry(chest.map_id)
                .or_default()
                .push(idx);
        }

        let mut items_by_id = HashMap::with_capacity(parts.items.len());
        for (idx, item) in parts.items.iter().enumerate() {
            items_by_id.insert(item.id, idx);
        }

        let mut quests_by_id = HashMap::with_capacity(parts.quests.len());
        for (idx, quest) in parts.quests.iter().enumerate() {
            quests_by_id.insert(quest.id, idx);
        }

        let mut quest_board_by_id = HashMap::with_capacity(parts.quest_board.len());
        for (idx, entry) in parts.quest_board.iter().enumerate() {
            if !entry.is_placeholder() {
                quest_board_by_id.insert(entry.quest_id, idx);
            }
        }

        let mut shops_by_id = HashMap::with_capacity(parts.shops.len());
        for (idx, shop) in parts.shops.iter().enumerate() {
            shops_by_id.insert(shop.id, idx);
        }

        let mut various_shops_by_id = HashMap::with_capacity(parts.various_shops.len());
        for (idx, bundle) in parts.various_shops.iter().enumerate() {
            various_shops_by_id.insert(bundle.id, idx);
        }

        Self {
            maps: parts.maps,
            islands: parts.islands,
            regions: parts.regions,
            release_conditions: parts.release_conditions,
            save_points: parts.save_points,
            save_point_commands: parts.save_point_commands,
            teleport_points: parts.teleport_points,
            treasure_boxes: parts.treasure_boxes,
            items: parts.items,
            quests: parts.quests,
            quest_board: parts.quest_board,
            shops: parts.shops,
            various_shops: parts.various_shops,
            inns: parts.inns,
            chapters: parts.chapters,
            main_scenario: parts.main_scenario,
            text_resolver: parts.text_resolver,
            maps_by_id,
            maps_by_level_name,
            teleport_by_map_id,
            treasures_by_map_id,
            items_by_id,
            quests_by_id,
            quest_board_by_id,
            shops_by_id,
            various_shops_by_id,
        }
    }

    pub fn text_resolver(&self) -> &TextResolver {
        &self.text_resolver
    }

    pub fn maps(&self) -> &[MapInfo] {
        &self.maps
    }

    pub fn islands(&self) -> &[MapIsland] {
        &self.islands
    }

    pub fn regions(&self) -> &[MapRegion] {
        &self.regions
    }

    pub fn release_conditions(&self) -> &[MapReleaseCondition] {
        &self.release_conditions
    }

    pub fn save_points(&self) -> &[SavePoint] {
        &self.save_points
    }

    pub fn save_point_commands(&self) -> &[SavePointCommand] {
        &self.save_point_commands
    }

    pub fn teleport_points(&self) -> &[TeleportPoint] {
        &self.teleport_points
    }

    pub fn treasure_boxes(&self) -> &[TreasureBox] {
        &self.treasure_boxes
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn quests(&self) -> &[Quest] {
        &self.quests
    }

    pub fn quest_board(&self) -> &[QuestBoardEntry] {
        &self.quest_board
    }

    pub fn shops(&self) -> &[Shop] {
        &self.shops
    }

    pub fn various_shops(&self) -> &[VariousShop] {
        &self.various_shops
    }

    pub fn inns(&self) -> &[Inn] {
        &self.inns
    }

    pub fn chapters(&self) -> &[Chapter] {
        &self.chapters
    }

    pub fn main_scenario(&self) -> &[MainScenarioEntry] {
        &self.main_scenario
    }

    // ---- Map queries ----

    /// Look up a map by its numeric ID (`CDT_MapData_Prod.ID`).
    pub fn map_by_id(&self, map_id: i64) -> Option<&MapInfo> {
        self.maps_by_id.get(&map_id).map(|&idx| &self.maps[idx])
    }

    /// Reverse-lookup a map from a runtime level name (e.g. `"Com_00Com_45_PL"`) or UE level path.
    pub fn map_by_level_name(&self, level_name: &str) -> Option<&MapInfo> {
        let stem = extract_level_name(level_name)?;
        self.maps_by_level_name
            .get(&stem)
            .map(|&idx| &self.maps[idx])
    }

    /// Reverse-lookup a numeric map ID from a runtime level name or UE level path.
    pub fn map_id_by_level_name(&self, level_name: &str) -> Option<i64> {
        self.map_by_level_name(level_name).map(|map| map.id)
    }

    /// Resolve a map's display name in `"zh-Hans"` (never silently falls back to Japanese).
    pub fn map_name_zh(&self, map: &MapInfo) -> ResolvedText {
        self.text_resolver.resolve_model_text(&map.name)
    }

    /// Find all maps whose `"zh-Hans"` display name matches `zh_name` exactly.
    pub fn find_maps_by_name(&self, zh_name: &str) -> Vec<&MapInfo> {
        let needle = zh_name.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.maps
            .iter()
            .filter(|map| self.matches_zh_exact(&map.name, needle))
            .collect()
    }

    /// Search maps whose `"zh-Hans"` display name contains `query`.
    pub fn search_maps_by_name(&self, query: &str) -> Vec<&MapInfo> {
        let needle = query.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.maps
            .iter()
            .filter(|map| self.matches_zh_contains(&map.name, needle))
            .collect()
    }

    /// Return all maps belonging to `island_type` (e.g. `"eMapIslandType::Grandshelt"`).
    pub fn maps_by_island(&self, island_type: &str) -> Vec<&MapInfo> {
        self.maps
            .iter()
            .filter(|map| map.island == island_type)
            .collect()
    }

    /// Look up island metadata by `IslandType`.
    pub fn island_by_type(&self, island_type: &str) -> Option<&MapIsland> {
        self.islands
            .iter()
            .find(|island| island.island_type == island_type)
    }

    /// Look up a region by `DT_MapRegionData.ID`.
    pub fn region_by_id(&self, region_id: i64) -> Option<&MapRegion> {
        self.regions.iter().find(|region| region.id == region_id)
    }

    /// Return all regions belonging to `island_type`.
    pub fn regions_by_island(&self, island_type: &str) -> Vec<&MapRegion> {
        self.regions
            .iter()
            .filter(|region| region.island_type == island_type)
            .collect()
    }

    /// Return the unlock condition row for `island_type`, if any.
    pub fn release_condition_for_island(&self, island_type: &str) -> Option<&MapReleaseCondition> {
        self.release_conditions
            .iter()
            .find(|entry| entry.island_type == island_type)
    }

    // ---- SavePoint & TeleportPoint queries ----

    /// Look up the save-point capability profile for a map type (`eMapType::Town` / `eMapType::Dungeon`).
    pub fn save_point_for_map_type(&self, map_type: &str) -> Option<&SavePoint> {
        self.save_points
            .iter()
            .find(|point| point.map_type == map_type)
    }

    /// Look up a save-point menu command by `ESavePointCommandType::*`.
    pub fn save_point_command(&self, command_type: &str) -> Option<&SavePointCommand> {
        self.save_point_commands
            .iter()
            .find(|cmd| cmd.command_type == command_type)
    }

    /// Return all world-map teleport/airship landmarks targeting `map_id`.
    pub fn teleport_points_for_map(&self, map_id: i64) -> Vec<&TeleportPoint> {
        self.teleport_by_map_id
            .get(&map_id)
            .map(|indices| indices.iter().map(|&i| &self.teleport_points[i]).collect())
            .unwrap_or_default()
    }

    /// Returns `true` if `map_id` appears as a world-map landmark / teleport destination.
    pub fn is_teleport_destination(&self, map_id: i64) -> bool {
        self.teleport_by_map_id.contains_key(&map_id)
    }

    /// Return all known maps reachable via `Map/DA_MapLandmark.json`, sorted by map ID.
    pub fn teleportable_maps(&self) -> Vec<&MapInfo> {
        self.maps
            .iter()
            .filter(|map| self.is_teleport_destination(map.id))
            .collect()
    }

    // ---- TreasureBox queries ----

    /// Return all static treasure boxes located in `map_id`, sorted by box ID.
    pub fn treasure_boxes_in_map(&self, map_id: i64) -> Vec<&TreasureBox> {
        self.treasures_by_map_id
            .get(&map_id)
            .map(|indices| indices.iter().map(|&i| &self.treasure_boxes[i]).collect())
            .unwrap_or_default()
    }

    /// Look up one treasure box by its composite key `(map_id, box_id)`.
    pub fn treasure_box(&self, map_id: i64, box_id: i64) -> Option<&TreasureBox> {
        self.treasure_boxes_in_map(map_id)
            .into_iter()
            .find(|chest| chest.id == box_id)
    }

    // ---- Item queries ----

    /// Look up an item by its global `ItemId` across all 6 item tables.
    pub fn item_by_id(&self, item_id: i64) -> Option<&Item> {
        self.items_by_id.get(&item_id).map(|&idx| &self.items[idx])
    }

    /// Resolve an item's display name in `"zh-Hans"`.
    pub fn item_name_zh(&self, item: &Item) -> ResolvedText {
        self.text_resolver.resolve_model_text(&item.name)
    }

    /// Resolve an item's description in `"zh-Hans"`.
    pub fn item_description_zh(&self, item: &Item) -> ResolvedText {
        self.text_resolver.resolve_model_text(&item.description)
    }

    /// Find all items whose `"zh-Hans"` name matches `zh_name` exactly.
    pub fn find_items_by_name(&self, zh_name: &str) -> Vec<&Item> {
        let needle = zh_name.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.items
            .iter()
            .filter(|item| self.matches_zh_exact(&item.name, needle))
            .collect()
    }

    /// Search items whose `"zh-Hans"` name contains `query`.
    pub fn search_items_by_name(&self, query: &str) -> Vec<&Item> {
        let needle = query.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.items
            .iter()
            .filter(|item| self.matches_zh_contains(&item.name, needle))
            .collect()
    }

    /// Return all items belonging to `category`.
    pub fn items_by_category(&self, category: ItemCategory) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| item.category == category)
            .collect()
    }

    // ---- Quest & main objective queries ----

    /// Look up a side quest by `DT_RequestData.ID`.
    pub fn quest_by_id(&self, quest_id: i64) -> Option<&Quest> {
        self.quests_by_id
            .get(&quest_id)
            .map(|&idx| &self.quests[idx])
    }

    /// Resolve a quest's title in `"zh-Hans"`.
    pub fn quest_title_zh(&self, quest: &Quest) -> ResolvedText {
        self.text_resolver.resolve_model_text(&quest.title)
    }

    /// Find all side quests whose `"zh-Hans"` title matches `zh_name` exactly.
    pub fn find_quests_by_name(&self, zh_name: &str) -> Vec<&Quest> {
        let needle = zh_name.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.quests
            .iter()
            .filter(|quest| self.matches_zh_exact(&quest.title, needle))
            .collect()
    }

    /// Search side quests whose `"zh-Hans"` title contains `query`.
    pub fn search_quests_by_name(&self, query: &str) -> Vec<&Quest> {
        let needle = query.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.quests
            .iter()
            .filter(|quest| self.matches_zh_contains(&quest.title, needle))
            .collect()
    }

    /// Return all side quests derived as belonging to `chapter_no`.
    pub fn quests_by_chapter(&self, chapter_no: i64) -> Vec<&Quest> {
        self.quests
            .iter()
            .filter(|quest| quest.chapter_no == Some(chapter_no))
            .collect()
    }

    /// Look up a quest-board delivery sub-quest by `CDT_QuestBoardData.QuestId`.
    pub fn quest_board_by_id(&self, quest_id: i64) -> Option<&QuestBoardEntry> {
        self.quest_board_by_id
            .get(&quest_id)
            .map(|&idx| &self.quest_board[idx])
    }

    /// Find non-placeholder quest-board entries whose `"zh-Hans"` title matches `zh_name` exactly.
    pub fn find_quest_board_by_name(&self, zh_name: &str) -> Vec<&QuestBoardEntry> {
        let needle = zh_name.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.quest_board
            .iter()
            .filter(|entry| !entry.is_placeholder() && self.matches_zh_exact(&entry.title, needle))
            .collect()
    }

    /// Return all main-story objectives belonging to `chapter_no`, sorted by progress threshold.
    pub fn main_objectives_by_chapter(&self, chapter_no: i64) -> Vec<&MainScenarioEntry> {
        get_chapter_main_scenario(&self.main_scenario, chapter_no)
    }

    // ---- Shop queries ----

    /// Look up a shop by `DT_ShopList.ShopID`.
    pub fn shop_by_id(&self, shop_id: i64) -> Option<&Shop> {
        self.shops_by_id.get(&shop_id).map(|&idx| &self.shops[idx])
    }

    /// Look up a multi-shop bundle by `DT_VariousShopsList.ID`.
    pub fn various_shop_by_id(&self, id: i64) -> Option<&VariousShop> {
        self.various_shops_by_id
            .get(&id)
            .map(|&idx| &self.various_shops[idx])
    }

    /// Find all shops whose `"zh-Hans"` localized name matches `zh_name` exactly.
    pub fn find_shops_by_name(&self, zh_name: &str) -> Vec<&Shop> {
        let needle = zh_name.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        self.shops
            .iter()
            .filter(|shop| self.matches_zh_exact(&shop.name, needle))
            .collect()
    }

    /// Return all shops that carry `item_id`.
    pub fn shops_selling_item(&self, item_id: i64) -> Vec<&Shop> {
        self.shops
            .iter()
            .filter(|shop| shop.items.iter().any(|entry| entry.item_id == item_id))
            .collect()
    }

    /// Compute the effective purchase price of a [`ShopItem`] using the global item catalog.
    pub fn shop_item_price(&self, entry: &ShopItem) -> Option<i64> {
        self.item_by_id(entry.item_id)
            .map(|item| entry.effective_price(item.buying_price))
    }

    // ---- Convenience model-facing Chinese summaries ----

    /// Produce a Chinese summary for map `map_id`, or `None` if absent.
    pub fn map_summary(&self, map_id: i64) -> Option<String> {
        self.map_by_id(map_id).map(|map| map_summary(self, map))
    }

    /// Produce a Chinese summary for item `item_id`, or `None` if absent.
    pub fn item_summary(&self, item_id: i64) -> Option<String> {
        self.item_by_id(item_id)
            .map(|item| item_summary(self, item))
    }

    /// Produce a Chinese summary for side quest `quest_id`, or `None` if absent.
    pub fn quest_summary(&self, quest_id: i64) -> Option<String> {
        self.quest_by_id(quest_id)
            .map(|quest| quest_summary(self, quest))
    }

    /// Produce a Chinese summary for shop `shop_id`, or `None` if absent.
    pub fn shop_summary(&self, shop_id: i64) -> Option<String> {
        self.shop_by_id(shop_id)
            .map(|shop| shop_summary(self, shop))
    }

    fn matches_zh_exact(&self, value: &FTextValue, needle: &str) -> bool {
        self.text_resolver
            .resolve_model_text(value)
            .text
            .as_deref()
            .is_some_and(|text| text.trim() == needle)
    }

    fn matches_zh_contains(&self, value: &FTextValue, needle: &str) -> bool {
        self.text_resolver
            .resolve_model_text(value)
            .text
            .as_deref()
            .is_some_and(|text| text.contains(needle))
    }
}

fn parse_from_store<T>(
    store: &DataStore,
    relative_path: &str,
    parser: fn(&crate::datatable::DataTable, &str) -> Result<T>,
) -> Result<T> {
    let table = store.data_table(relative_path)?;
    let label = datatable_relative_path_to_json(relative_path, store.export_root()).map_or_else(
        || relative_path.to_owned(),
        |path| path.display().to_string(),
    );
    parser(table.as_ref(), &label)
}
