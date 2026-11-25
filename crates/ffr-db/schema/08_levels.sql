-- 08_levels: Exported levels, multi-map mapping, placed actors, R*Tree spatial index,
-- classified level objects, map transition graph edges, landing points, and expanded map shops.

CREATE TABLE level (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    raw_name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    package_path TEXT NOT NULL,
    sublevel_type TEXT NOT NULL,
    persistent_level TEXT,
    map_id INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE level_map (
    level_id INTEGER NOT NULL,
    map_id INTEGER NOT NULL,
    PRIMARY KEY (level_id, map_id)
);

CREATE TABLE actor (
    id INTEGER PRIMARY KEY,
    level_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    label TEXT,
    class TEXT NOT NULL,
    is_game_class INTEGER NOT NULL CHECK (is_game_class IN (0, 1)),
    x REAL,
    y REAL,
    z REAL,
    pitch REAL,
    yaw REAL,
    roll REAL,
    scale_x REAL,
    scale_y REAL,
    scale_z REAL,
    props TEXT,
    asset_refs TEXT,
    subobjects TEXT,
    UNIQUE (level_id, name)
);

CREATE VIRTUAL TABLE actor_rtree USING rtree(
    id,
    min_x, max_x,
    min_y, max_y,
    min_z, max_z
);

CREATE TABLE level_object (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    map_id INTEGER,
    level_id INTEGER NOT NULL,
    actor_id INTEGER,
    unique_id INTEGER,
    ref_id TEXT,
    visible_condition TEXT,
    x REAL,
    y REAL,
    z REAL,
    doc TEXT NOT NULL
);

CREATE TABLE map_edge (
    id INTEGER PRIMARY KEY,
    from_map INTEGER NOT NULL,
    level_id INTEGER,
    level_name TEXT NOT NULL,
    actor_id INTEGER,
    actor_name TEXT NOT NULL,
    unique_id INTEGER,
    to_map INTEGER,
    to_point INTEGER,
    kind TEXT NOT NULL CHECK (kind IN ('cross_map', 'same_map', 'savepoint_teleport')),
    is_auto INTEGER NOT NULL CHECK (is_auto IN (0, 1)),
    is_event_only INTEGER NOT NULL CHECK (is_event_only IN (0, 1)),
    condition TEXT,
    x REAL,
    y REAL,
    z REAL
);

CREATE TABLE transition_point (
    map_id INTEGER NOT NULL,
    point_id INTEGER NOT NULL,
    level_id INTEGER NOT NULL,
    level_name TEXT NOT NULL,
    actor_id INTEGER,
    actor_name TEXT NOT NULL,
    direction TEXT,
    point_name TEXT,
    x REAL,
    y REAL,
    z REAL,
    PRIMARY KEY (map_id, point_id)
);

CREATE TABLE map_shop (
    id INTEGER PRIMARY KEY,
    map_id INTEGER NOT NULL,
    shop_id INTEGER NOT NULL,
    source_shop_id INTEGER NOT NULL,
    is_various_shop INTEGER NOT NULL CHECK (is_various_shop IN (0, 1)),
    level_object_id INTEGER NOT NULL
);
