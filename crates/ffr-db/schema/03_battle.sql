-- 03_battle: Typed battle domain tables, relationship tables, and FTS5 name indexes.

CREATE TABLE unit (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    category TEXT NOT NULL,
    race TEXT NOT NULL,
    level INTEGER NOT NULL,
    is_boss INTEGER NOT NULL CHECK (is_boss IN (0, 1)),
    max_hp INTEGER NOT NULL,
    max_break_point INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE unit_name (
    unit_id INTEGER NOT NULL,
    lang TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (unit_id, lang)
);

CREATE VIRTUAL TABLE unit_name_fts USING fts5(
    unit_id UNINDEXED,
    lang UNINDEXED,
    name,
    tokenize='trigram'
);

CREATE TABLE unit_resistance (
    unit_id INTEGER NOT NULL,
    element TEXT NOT NULL,
    resistance TEXT NOT NULL,
    PRIMARY KEY (unit_id, element)
);

CREATE TABLE skill (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    attr_type TEXT NOT NULL,
    damage_type TEXT NOT NULL,
    element TEXT NOT NULL,
    target_type TEXT NOT NULL,
    cost INTEGER NOT NULL,
    cost_type TEXT NOT NULL,
    power INTEGER NOT NULL,
    break_damage INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE skill_name (
    skill_id INTEGER NOT NULL,
    lang TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (skill_id, lang)
);

CREATE VIRTUAL TABLE skill_name_fts USING fts5(
    skill_id UNINDEXED,
    lang UNINDEXED,
    name,
    tokenize='trigram'
);

CREATE TABLE unit_skill (
    unit_id INTEGER NOT NULL,
    skill_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    level INTEGER
);

CREATE TABLE command_skill (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    sort_id INTEGER NOT NULL,
    unit_id INTEGER,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    is_removable INTEGER NOT NULL CHECK (is_removable IN (0, 1)),
    equip_cost INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE battle_group (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    is_boss INTEGER CHECK (is_boss IS NULL OR is_boss IN (0, 1)),
    can_escape INTEGER NOT NULL CHECK (can_escape IN (0, 1)),
    is_bounty INTEGER NOT NULL CHECK (is_bounty IN (0, 1)),
    ap INTEGER NOT NULL,
    battle_event_id INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE battle_group_member (
    group_id INTEGER NOT NULL,
    slot INTEGER NOT NULL,
    unit_id INTEGER NOT NULL,
    PRIMARY KEY (group_id, slot)
);

CREATE TABLE encounter (
    pool_path TEXT NOT NULL,
    region TEXT,
    row_key TEXT NOT NULL,
    battle_group_id INTEGER NOT NULL,
    encount_ratio INTEGER NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (pool_path, row_key)
);

CREATE TABLE unit_ai (
    unit_id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    ai_id INTEGER NOT NULL,
    fight_skill_id INTEGER NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE battle_param (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    row_key TEXT NOT NULL,
    break_damage_magnification REAL NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE enemy_chapter (
    unit_id INTEGER NOT NULL,
    chapter_no INTEGER NOT NULL,
    source TEXT NOT NULL,
    battle_group_id INTEGER NOT NULL,
    source_ref TEXT NOT NULL,
    PRIMARY KEY (unit_id, chapter_no, source, battle_group_id, source_ref)
);
