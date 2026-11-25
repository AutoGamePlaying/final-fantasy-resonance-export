//! Model-facing Chinese (`zh-Hans`) summary formatters for maps, items, quests, and shops.
//!
//! All text resolution strictly queries `"zh-Hans"` via [`crate::TextResolver::resolve_model_text`]
//! and never silently passes Japanese source text off as Chinese.

use crate::ftext::{FTextValue, TextResolver};
use crate::scenario::MainScenarioEntry;
use crate::world::WorldData;
use crate::world::items::{Item, ItemCategory, ItemDetail};
use crate::world::maps::MapInfo;
use crate::world::quests::Quest;
use crate::world::shops::Shop;

/// Build a concise Chinese summary of a [`MapInfo`] for the AI planner.
pub fn map_summary(world: &WorldData, map: &MapInfo) -> String {
    let display_name = resolve_zh_or_placeholder(world.text_resolver(), &map.name, "未命名地图");
    let map_type = map_type_label(&map.map_type);
    let mut parts = vec![format!(
        "地图 #{id}「{display_name}」（关卡 {level}，{map_type}）",
        id = map.id,
        level = map.level_name
    )];

    if let Some(dungeon_sub) = dungeon_type_label(&map.dungeon_type) {
        parts.push(format!("迷宫类型：{dungeon_sub}"));
    }

    if let Some(island_label) = resolve_island_label(world, &map.island) {
        parts.push(format!("区域：{island_label}"));
    }

    if map.danger_level > 0 {
        parts.push(format!("危险度：{}", map.danger_level));
    }

    if map.do_movement_encount {
        if let Some(group_id) = map.encount_group_id {
            parts.push(format!("移动遇敌：开启（遇敌组 #{group_id}）"));
        } else {
            parts.push("移动遇敌：开启".to_owned());
        }
    }

    if !map.facilities.is_empty() {
        let mut names = Vec::new();
        for fac in &map.facilities {
            let label = facility_label(&fac.interact_type);
            if !names.contains(&label) {
                names.push(label);
            }
        }
        parts.push(format!("设施：{}", names.join("、")));
    }

    if world.is_teleport_destination(map.id) {
        parts.push("世界地图可传送".to_owned());
    }

    if let Some(sp) = world.save_point_for_map_type(&map.map_type) {
        let mut caps = Vec::new();
        if sp.can_save {
            caps.push("存档");
        }
        if sp.can_use_tent {
            caps.push("帐篷");
        }
        if sp.can_teleport {
            caps.push("迷宫内传送");
        }
        if !caps.is_empty() {
            parts.push(format!("存档点支持：{}", caps.join("/")));
        }
    }

    let box_count = world.treasure_boxes_in_map(map.id).len();
    if box_count > 0 {
        parts.push(format!("宝箱：{box_count} 个"));
    }

    parts.join("；")
}

/// Build a concise Chinese effect overview of an [`Item`] (params, scene, skills, and description).
pub fn item_effect_overview(resolver: &TextResolver, item: &Item) -> String {
    let mut facts = Vec::new();

    match &item.detail {
        ItemDetail::Consumable {
            usable_scene,
            is_consumed_after_use,
            skill_id,
            is_throwable,
            throw_skill_id,
        } => {
            facts.push(format!("使用场合：{}", usable_scene_label(usable_scene)));
            if !*is_consumed_after_use {
                facts.push("使用后不消耗".to_owned());
            }
            if let Some(sid) = skill_id {
                facts.push(format!("技能 #{sid}"));
            }
            if *is_throwable {
                if let Some(tsid) = throw_skill_id {
                    facts.push(format!("可投掷（技能 #{tsid}）"));
                } else {
                    facts.push("可投掷".to_owned());
                }
            }
        }
        ItemDetail::Seed { harvest_recipes } => {
            facts.push(format!("培育配方 {} 种", harvest_recipes.len()));
        }
        ItemDetail::Equipment {
            params,
            passive_skill_ids,
            ..
        } => {
            if !params.is_empty() {
                let stats: Vec<String> = params
                    .iter()
                    .map(|p| {
                        let sign = if p.value >= 0 { "+" } else { "" };
                        format!("{}{sign}{}", param_type_label(&p.param_type), p.value)
                    })
                    .collect();
                facts.push(format!("属性：{}", stats.join("、")));
            }
            if !passive_skill_ids.is_empty() {
                let ids: Vec<String> = passive_skill_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect();
                facts.push(format!("被动技能：{}", ids.join("、")));
            }
        }
        ItemDetail::Vision {
            finish_blow_skill_id,
            attack_type,
            roles,
            ..
        } => {
            facts.push(format!("攻击类型：{}", attack_type_label(attack_type)));
            if !roles.is_empty() {
                let role_labels: Vec<String> =
                    roles.iter().map(|r| role_label(r).to_owned()).collect();
                facts.push(format!("职责：{}", role_labels.join("/")));
            }
            if let Some(sid) = finish_blow_skill_id {
                facts.push(format!("终结技 #{sid}"));
            }
        }
        ItemDetail::Valuable => {}
        ItemDetail::Artifact {
            item_tag,
            effect_values,
            is_initial_active,
        } => {
            if !item_tag.is_empty() && item_tag != "None" {
                facts.push(format!("标签：{item_tag}"));
            }
            if !effect_values.is_empty() {
                let vals: Vec<String> = effect_values.iter().map(|v| format_number(*v)).collect();
                facts.push(format!("效果值：{}", vals.join("、")));
            }
            if *is_initial_active {
                facts.push("初始生效".to_owned());
            }
        }
    }

    if let Some(desc) = resolve_zh_clean(resolver, &item.description) {
        facts.push(desc);
    }

    if facts.is_empty() {
        "无效果说明".to_owned()
    } else {
        facts.join("；")
    }
}

/// Build a concise Chinese summary of an [`Item`] for the AI planner.
pub fn item_summary(world: &WorldData, item: &Item) -> String {
    let display_name = resolve_zh_or_placeholder(world.text_resolver(), &item.name, "无中文译名");
    let category = item_category_label(item);
    let price = if item.is_available_sale {
        format!("买价 {} / 卖价 {}", item.buying_price, item.selling_price)
    } else {
        format!("买价 {} / 不可出售", item.buying_price)
    };
    let effect = item_effect_overview(world.text_resolver(), item);

    format!(
        "道具 #{id}「{display_name}」（{category}）；{price}；效果：{effect}",
        id = item.id
    )
}

/// Build a concise Chinese summary of a side [`Quest`] (`Request/DT_RequestData`) for the AI planner.
pub fn quest_summary(world: &WorldData, quest: &Quest) -> String {
    let title = resolve_zh_or_placeholder(world.text_resolver(), &quest.title, "无中文标题");
    let chapter = match quest.chapter_no {
        Some(ch) => {
            let chapter_title = world
                .chapters()
                .iter()
                .find(|c| c.chapter_no == ch)
                .and_then(|c| resolve_zh_clean(world.text_resolver(), &c.name));
            match chapter_title {
                Some(name) => format!("第 {ch} 章「{name}」"),
                None => format!("第 {ch} 章"),
            }
        }
        None => "所属章节未定（前置支线条件）".to_owned(),
    };

    let mut parts = vec![format!(
        "任务 #{id}「{title}」（{chapter}，地区：{region}）",
        id = quest.id,
        region = request_category_label(&quest.category)
    )];

    if let Some(requester) = resolve_zh_clean(world.text_resolver(), &quest.requester) {
        if let Some(map_id) = quest.requester_map_id {
            let map_name = world
                .map_by_id(map_id)
                .and_then(|m| resolve_zh_clean(world.text_resolver(), &m.name));
            match map_name {
                Some(name) => {
                    parts.push(format!("委托人：{requester}（地图 #{map_id}「{name}」）"))
                }
                None => parts.push(format!("委托人：{requester}（地图 #{map_id}）")),
            }
        } else {
            parts.push(format!("委托人：{requester}"));
        }
    }

    if !quest.accept_condition.is_empty() {
        parts.push(format!("接取条件：{}", quest.accept_condition));
    }

    let goals: Vec<String> = quest
        .objectives
        .iter()
        .enumerate()
        .filter_map(|(idx, stage)| {
            resolve_zh_clean(world.text_resolver(), &stage.goal_text)
                .map(|text| format!("{}. {text}", idx + 1))
        })
        .collect();
    if !goals.is_empty() {
        parts.push(format!("目标：{}", goals.join(" -> ")));
    }

    let prizes: Vec<String> = quest
        .prizes
        .iter()
        .filter_map(|prize| {
            if let Some(item_id) = prize.item_id {
                let item_name = world
                    .item_by_id(item_id)
                    .and_then(|it| resolve_zh_clean(world.text_resolver(), &it.name))
                    .unwrap_or_else(|| format!("道具 #{item_id}"));
                Some(format!("{item_name}×{}", prize.count))
            } else {
                resolve_zh_clean(world.text_resolver(), &prize.text)
            }
        })
        .collect();
    if !prizes.is_empty() {
        parts.push(format!("奖励：{}", prizes.join("、")));
    }

    parts.join("；")
}

/// Build a concise Chinese summary of a [`MainScenarioEntry`] (main story objective).
pub fn main_objective_summary(world: &WorldData, entry: &MainScenarioEntry) -> String {
    let chapter = match entry.chapter_no {
        Some(ch) => format!("第 {ch} 章"),
        None => "未分章".to_owned(),
    };
    let progress = match entry.min_progress {
        Some(p) => format!("进度>={p}"),
        None => "无进度阈值".to_owned(),
    };
    let goal = resolve_zh_or_placeholder(world.text_resolver(), &entry.text, "无中文目标");
    let dest_names: Vec<String> = entry
        .destinations
        .iter()
        .map(|dest| {
            world
                .map_by_id(dest.destination_id)
                .and_then(|m| resolve_zh_clean(world.text_resolver(), &m.name))
                .map_or_else(
                    || format!("地图 #{}", dest.destination_id),
                    |name| format!("地图 #{}「{name}」", dest.destination_id),
                )
        })
        .collect();

    if dest_names.is_empty() {
        format!("主线（{chapter}，{progress}）：{goal}")
    } else {
        format!(
            "主线（{chapter}，{progress}）：{goal}；目的地：{}",
            dest_names.join("、")
        )
    }
}

/// Build a concise Chinese summary of a [`Shop`] and its stock list for the AI planner.
pub fn shop_summary(world: &WorldData, shop: &Shop) -> String {
    let shop_title = match resolve_zh_clean(world.text_resolver(), &shop.name) {
        Some(name) => format!("商店 #{}「{name}」", shop.id),
        None => format!("商店 #{}（无中文店名）", shop.id),
    };
    let mut goods = Vec::with_capacity(shop.items.len());
    for entry in &shop.items {
        let (name, price_str) = match world.item_by_id(entry.item_id) {
            Some(item) => {
                let item_name =
                    resolve_zh_or_placeholder(world.text_resolver(), &item.name, "无中文译名");
                let price = entry.effective_price(item.buying_price);
                (item_name, format!("{price}G"))
            }
            None => (
                format!("未知道具 #{}", entry.item_id),
                "价格未知".to_owned(),
            ),
        };
        let limit = match entry.max_order_num {
            Some(max) => format!("，限购 {max}"),
            None => String::new(),
        };
        goods.push(format!(
            "{name}（#{id}，{price_str}{limit}）",
            id = entry.item_id
        ));
    }

    if goods.is_empty() {
        format!("{shop_title}：暂无商品")
    } else {
        format!(
            "{shop_title}（共 {} 件）：{}",
            goods.len(),
            goods.join("、")
        )
    }
}

fn resolve_zh_clean(resolver: &TextResolver, value: &FTextValue) -> Option<String> {
    let resolved = resolver.resolve_model_text(value);
    let text = resolved.text?;
    let collapsed: Vec<&str> = text.split_whitespace().collect();
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed.join(" "))
    }
}

fn resolve_zh_or_placeholder(
    resolver: &TextResolver,
    value: &FTextValue,
    placeholder: &str,
) -> String {
    resolve_zh_clean(resolver, value).unwrap_or_else(|| placeholder.to_owned())
}

fn resolve_island_label(world: &WorldData, island_type: &str) -> Option<String> {
    if island_type.is_empty() || island_type == "eMapIslandType::None" {
        return None;
    }
    if let Some(island) = world.island_by_type(island_type)
        && let Some(zh) = resolve_zh_clean(world.text_resolver(), &island.name)
    {
        return Some(zh);
    }
    Some(strip_enum_prefix(island_type).to_owned())
}

fn item_category_label(item: &Item) -> String {
    match (&item.category, &item.detail) {
        (ItemCategory::Consumable, _) => "消耗品".to_owned(),
        (ItemCategory::Seed, _) => "种子".to_owned(),
        (
            ItemCategory::Equipment,
            ItemDetail::Equipment {
                equipment_type,
                detail_type,
                ..
            },
        ) => {
            let main = equipment_type_label(equipment_type);
            match equipment_detail_label(detail_type) {
                Some(sub) => format!("{main}/{sub}"),
                None => main.to_owned(),
            }
        }
        (ItemCategory::Equipment, _) => "装备".to_owned(),
        (ItemCategory::Vision, _) => "幻兵".to_owned(),
        (ItemCategory::Valuable, _) => "贵重品".to_owned(),
        (ItemCategory::Artifact, _) => "圣遗物".to_owned(),
    }
}

fn map_type_label(raw: &str) -> &str {
    match raw {
        "eMapType::Field" => "世界地图",
        "eMapType::Town" => "城镇",
        "eMapType::Dungeon" => "迷宫",
        "eMapType::VisionSmallShrine" => "幻兵小祠",
        other => strip_enum_prefix(other),
    }
}

fn dungeon_type_label(raw: &str) -> Option<&'static str> {
    match raw {
        "eMapDungeonType::Castle" => Some("城堡"),
        "eMapDungeonType::Cave" => Some("洞窟"),
        _ => None,
    }
}

fn facility_label(raw: &str) -> &str {
    match raw {
        "eMapInteractType::Inn" => "旅店",
        "eMapInteractType::Shop" => "道具店",
        "eMapInteractType::Shop_Weapon" => "武器店",
        "eMapInteractType::Shop_Armor" => "防具店",
        "eMapInteractType::Shop_Accessory" => "饰品店",
        "eMapInteractType::Shop_Vision" => "幻兵店",
        other => strip_enum_prefix(other),
    }
}

fn usable_scene_label(raw: &str) -> &str {
    match raw {
        "eItemUsableScene::Any" => "战斗/地图通用",
        "eItemUsableScene::BattleOnly" => "仅战斗",
        "eItemUsableScene::MapOnly" => "仅地图",
        other => strip_enum_prefix(other),
    }
}

fn equipment_type_label(raw: &str) -> &str {
    match raw {
        "eEquipmentType::Weapon" => "武器",
        "eEquipmentType::Armor" => "防具",
        "eEquipmentType::Accessory" => "饰品",
        other => strip_enum_prefix(other),
    }
}

fn equipment_detail_label(raw: &str) -> Option<&'static str> {
    match raw {
        "eEquipmentDetailType::GreatSowrd" | "eEquipmentDetailType::GreatSword" => Some("大剑"),
        "eEquipmentDetailType::Katana" => Some("太刀"),
        "eEquipmentDetailType::Bow" => Some("弓"),
        "eEquipmentDetailType::Hammer" => Some("锤"),
        "eEquipmentDetailType::Grimoire" => Some("魔导书"),
        "eEquipmentDetailType::Whip" => Some("鞭"),
        "eEquipmentDetailType::Gun" => Some("枪"),
        "eEquipmentDetailType::Rod" => Some("法杖"),
        "eEquipmentDetailType::LightArmor" => Some("轻甲"),
        "eEquipmentDetailType::HeavyArmor" => Some("重甲"),
        "eEquipmentDetailType::Robe" => Some("长袍"),
        _ => None,
    }
}

fn param_type_label(raw: &str) -> &str {
    match raw {
        "eUnitParameterType::PhysicalAttack" => "物攻",
        "eUnitParameterType::MagicalAttack" => "魔攻",
        "eUnitParameterType::PhysicalDefense" => "物防",
        "eUnitParameterType::MagicalDefense" => "魔防",
        "eUnitParameterType::Defence" => "防御",
        "eUnitParameterType::Divine" => "精神",
        "eUnitParameterType::Agility" => "敏捷",
        "eUnitParameterType::HitPoint" => "HP",
        "eUnitParameterType::MagicPoint" => "MP",
        other => strip_enum_prefix(other),
    }
}

fn attack_type_label(raw: &str) -> &str {
    match raw {
        "eSkillDamageType::Physic" => "物理",
        "eSkillDamageType::Magic" => "魔法",
        "eSkillDamageType::None" => "无",
        other => strip_enum_prefix(other),
    }
}

fn role_label(raw: &str) -> &str {
    match raw {
        "eUnitRole::Attacker" => "攻击手",
        "eUnitRole::Breaker" => "破防手",
        "eUnitRole::Defender" | "eUnitRole::Tank" => "防御手",
        "eUnitRole::Enhancer" => "强化手",
        "eUnitRole::Healer" => "治疗手",
        "eUnitRole::Supporter" | "eUnitRole::Buffer" => "支援手",
        "eUnitRole::Jammer" | "eUnitRole::Debuffer" => "妨碍手",
        other => strip_enum_prefix(other),
    }
}

fn request_category_label(raw: &str) -> &str {
    strip_enum_prefix(raw)
}

fn strip_enum_prefix(raw: &str) -> &str {
    raw.rsplit_once("::").map_or(raw, |(_, suffix)| suffix)
}

fn format_number(value: f64) -> String {
    if (value - value.round()).abs() <= f64::EPSILON {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.2}")
    }
}
