-- 07_input_ui: Enhanced Input, DataAssets, Blueprint defaults/components, and UMG widget trees.

CREATE TABLE input_action (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    value_type TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE input_context (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    doc TEXT NOT NULL
);

CREATE TABLE input_mapping (
    context_id TEXT NOT NULL,
    action_id TEXT NOT NULL,
    key TEXT NOT NULL,
    modifiers TEXT NOT NULL,
    triggers TEXT NOT NULL,
    ord INTEGER NOT NULL,
    PRIMARY KEY (context_id, ord)
);

CREATE TABLE input_set (
    asset TEXT NOT NULL,
    device TEXT NOT NULL,
    context_id TEXT NOT NULL,
    PRIMARY KEY (asset, device, context_id)
);

CREATE TABLE input_set_action (
    asset TEXT NOT NULL,
    semantic TEXT NOT NULL,
    action_id TEXT NOT NULL,
    PRIMARY KEY (asset, semantic)
);

CREATE TABLE data_asset (
    path TEXT PRIMARY KEY,
    class TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE blueprint (
    path TEXT PRIMARY KEY,
    class_name TEXT NOT NULL,
    parent_class TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('actor', 'widget', 'other')),
    doc TEXT NOT NULL
);

CREATE TABLE blueprint_component (
    blueprint_id TEXT NOT NULL,
    name TEXT NOT NULL,
    class TEXT NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (blueprint_id, name)
);

CREATE TABLE widget_node (
    blueprint_id TEXT NOT NULL,
    name TEXT NOT NULL,
    class TEXT NOT NULL,
    parent_name TEXT,
    ord INTEGER NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (blueprint_id, name)
);
