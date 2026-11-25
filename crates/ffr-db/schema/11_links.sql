-- 11_links: Cross-area linkages covering talk event chains, dialogue lines,
-- level-object-to-talk mappings, map unit definitions, and normalized class references.

CREATE TABLE talk_event (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    sequence_path TEXT,
    sequence_id INTEGER,
    talk_table_path TEXT,
    is_skip_confirm INTEGER NOT NULL CHECK (is_skip_confirm IN (0, 1)),
    encount_group_id INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE talk_event_next (
    event TEXT NOT NULL,
    next_event TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('continuous', 'choice')),
    choice_index INTEGER,
    condition TEXT NOT NULL DEFAULT '',
    ord INTEGER NOT NULL,
    PRIMARY KEY (event, kind, ord)
);

CREATE TABLE talk_line (
    table_path TEXT NOT NULL,
    row TEXT NOT NULL,
    ord INTEGER NOT NULL,
    talk_id INTEGER,
    speaker TEXT NOT NULL,
    emotion TEXT NOT NULL,
    text_ns TEXT,
    text_key TEXT,
    voice TEXT NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (table_path, row)
);

CREATE TABLE level_object_talk (
    level_object_id INTEGER NOT NULL,
    talk_event_id TEXT NOT NULL,
    condition TEXT,
    ord INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (level_object_id, talk_event_id)
);

CREATE TABLE map_unit (
    id INTEGER PRIMARY KEY,
    row_key TEXT NOT NULL,
    name TEXT NOT NULL,
    name_ns TEXT,
    name_key TEXT,
    name_zh TEXT,
    primary_asset TEXT,
    doc TEXT NOT NULL
);

CREATE TABLE class_ref (
    raw_name TEXT PRIMARY KEY,
    normalized TEXT NOT NULL,
    ue_class_id TEXT,
    blueprint_id TEXT
);
