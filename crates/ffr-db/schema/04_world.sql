-- 04_world: World domain tables (maps, islands, regions, save/teleport points, map links,
-- treasure boxes, items, shops, inns, quests, chapters, main scenario, flag conditions,
-- playable units, item drops, and multilingual name lookup index).

CREATE TABLE map (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    group_id INTEGER NOT NULL,
    island TEXT NOT NULL,
    region_id INTEGER,
    same_map_id INTEGER,
    level_name TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    map_type TEXT NOT NULL,
    dungeon_type TEXT NOT NULL,
    danger_level INTEGER NOT NULL,
    encount_group_id INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE island (
    island_type TEXT PRIMARY KEY,
    row_key TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    display_condition TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE region (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    island_type TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE map_release_condition (
    island_type TEXT PRIMARY KEY,
    row_key TEXT NOT NULL,
    condition TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE save_point (
    map_type TEXT PRIMARY KEY,
    row_key TEXT NOT NULL,
    can_save INTEGER NOT NULL,
    can_use_tent INTEGER NOT NULL,
    can_teleport INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE save_point_command (
    command_type TEXT PRIMARY KEY,
    row_key TEXT NOT NULL,
    label_ns TEXT,
    label_key TEXT,
    label_zh TEXT,
    item_id INTEGER,
    is_enable INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE teleport_point (
    id INTEGER PRIMARY KEY,
    airship_landing_point_id INTEGER,
    loc_x REAL NOT NULL,
    loc_y REAL NOT NULL,
    loc_z REAL NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE teleport_point_target (
    teleport_id INTEGER NOT NULL,
    target_index INTEGER NOT NULL,
    map_id INTEGER NOT NULL,
    condition TEXT NOT NULL,
    PRIMARY KEY (teleport_id, target_index)
);

CREATE TABLE map_link (
    from_map INTEGER NOT NULL,
    to_map INTEGER NOT NULL,
    kind TEXT NOT NULL,
    via TEXT NOT NULL,
    condition TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (from_map, to_map, kind, via, condition)
);

CREATE TABLE treasure_box (
    map_id INTEGER NOT NULL,
    id INTEGER NOT NULL,
    row_key TEXT NOT NULL,
    item_id INTEGER,
    item_num INTEGER NOT NULL,
    money INTEGER,
    is_trap INTEGER NOT NULL,
    battle_group_id INTEGER,
    doc TEXT NOT NULL,
    PRIMARY KEY (map_id, id)
);

CREATE TABLE item (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    sort_id INTEGER NOT NULL,
    category TEXT NOT NULL,
    raw_item_type TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    buying_price INTEGER NOT NULL,
    selling_price INTEGER NOT NULL,
    is_available_sale INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE shop (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    doc TEXT NOT NULL
);

CREATE TABLE shop_item (
    shop_id INTEGER NOT NULL,
    slot_index INTEGER NOT NULL,
    item_id INTEGER NOT NULL,
    price INTEGER,
    price_ratio REAL NOT NULL,
    max_order_num INTEGER,
    condition TEXT NOT NULL,
    PRIMARY KEY (shop_id, slot_index)
);

CREATE TABLE various_shop (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE various_shop_member (
    various_shop_id INTEGER NOT NULL,
    slot TEXT NOT NULL,
    shop_id INTEGER NOT NULL,
    PRIMARY KEY (various_shop_id, slot)
);

CREATE TABLE inn (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    price INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE quest (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    title_ns TEXT,
    title_key TEXT,
    name_zh TEXT,
    category TEXT NOT NULL,
    chapter_no INTEGER,
    min_main_progress INTEGER,
    requester_map_id INTEGER,
    report_map_id INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE quest_prize (
    quest_id INTEGER NOT NULL,
    prize_index INTEGER NOT NULL,
    item_id INTEGER,
    count INTEGER NOT NULL,
    text_ns TEXT,
    text_key TEXT,
    text_zh TEXT,
    PRIMARY KEY (quest_id, prize_index)
);

CREATE TABLE quest_board (
    row_key TEXT PRIMARY KEY,
    quest_id INTEGER NOT NULL,
    quest_board_id INTEGER NOT NULL,
    quest_type TEXT NOT NULL,
    is_placeholder INTEGER NOT NULL,
    title_ns TEXT,
    title_key TEXT,
    name_zh TEXT,
    doc TEXT NOT NULL
);

CREATE TABLE quest_board_reward (
    quest_board_row_key TEXT NOT NULL,
    reward_index INTEGER NOT NULL,
    quest_id INTEGER NOT NULL,
    item_id INTEGER,
    item_num INTEGER,
    money INTEGER,
    PRIMARY KEY (quest_board_row_key, reward_index)
);

CREATE TABLE chapter (
    chapter_no INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    doc TEXT NOT NULL
);

CREATE TABLE main_scenario (
    row_key TEXT PRIMARY KEY,
    chapter_no INTEGER,
    min_progress INTEGER,
    progress_end INTEGER,
    condition TEXT NOT NULL,
    text_ns TEXT,
    text_key TEXT,
    text_zh TEXT,
    doc TEXT NOT NULL
);

CREATE TABLE main_quest (
    quest_id INTEGER PRIMARY KEY,
    chapter_no INTEGER NOT NULL,
    chapter_row_key TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    doc TEXT NOT NULL
);

CREATE TABLE flag_condition (
    row_key TEXT PRIMARY KEY,
    condition TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE flag_condition_term (
    condition_key TEXT NOT NULL,
    term_index INTEGER NOT NULL,
    flag_id INTEGER NOT NULL,
    expected INTEGER NOT NULL,
    PRIMARY KEY (condition_key, term_index)
);

CREATE TABLE playable_unit (
    map_unit_id INTEGER PRIMARY KEY,
    unit_id INTEGER NOT NULL,
    row_key TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    is_idle_by_four_direction INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE item_drop (
    unit_id INTEGER NOT NULL,
    drop_kind TEXT NOT NULL,
    item_id INTEGER NOT NULL,
    rate INTEGER NOT NULL,
    unit_name_ns TEXT,
    unit_name_key TEXT,
    unit_name_zh TEXT,
    PRIMARY KEY (unit_id, drop_kind)
);

CREATE TABLE world_name (
    entity_kind TEXT NOT NULL,
    entity_id INTEGER NOT NULL,
    lang TEXT NOT NULL,
    name TEXT NOT NULL,
    name_lower TEXT NOT NULL,
    PRIMARY KEY (entity_kind, entity_id, lang)
);

CREATE VIRTUAL TABLE world_name_fts USING fts5(
    entity_kind UNINDEXED,
    entity_id UNINDEXED,
    lang UNINDEXED,
    name,
    tokenize='trigram'
);
