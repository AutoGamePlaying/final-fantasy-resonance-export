-- 10_snapshots: Optional live-game R1 scene snapshots, captured objects/widgets/texts, and markers.

CREATE TABLE snapshot (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    time INTEGER,
    started_at INTEGER,
    finished_at INTEGER,
    world TEXT,
    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
    hotkey INTEGER NOT NULL DEFAULT 0 CHECK (hotkey IN (0, 1)),
    status TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE snapshot_object (
    id INTEGER PRIMARY KEY,
    snapshot_id TEXT NOT NULL,
    class TEXT NOT NULL,
    class_short TEXT NOT NULL,
    name TEXT NOT NULL,
    doc TEXT NOT NULL
);

CREATE TABLE snapshot_widget (
    id INTEGER PRIMARY KEY,
    snapshot_id TEXT NOT NULL,
    class TEXT NOT NULL,
    class_short TEXT NOT NULL,
    name TEXT NOT NULL,
    is_in_viewport INTEGER NOT NULL DEFAULT 0 CHECK (is_in_viewport IN (0, 1)),
    visibility INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE snapshot_text (
    id INTEGER PRIMARY KEY,
    snapshot_id TEXT NOT NULL,
    class TEXT NOT NULL,
    name TEXT NOT NULL,
    text TEXT NOT NULL DEFAULT '',
    visibility INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE marker (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    seq INTEGER,
    time INTEGER,
    local_time TEXT,
    world TEXT,
    text TEXT NOT NULL DEFAULT '',
    clock_ms INTEGER,
    doc TEXT NOT NULL
);
