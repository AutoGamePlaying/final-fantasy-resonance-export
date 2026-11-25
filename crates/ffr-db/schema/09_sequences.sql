-- 09_sequences: LevelSequence cutscene metadata, bindings, tracks, and event triggers.

CREATE TABLE sequence (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    category TEXT NOT NULL CHECK (category IN ('battle', 'event', 'other')),
    fps REAL NOT NULL,
    duration_secs REAL NOT NULL,
    binding_count INTEGER NOT NULL DEFAULT 0,
    track_count INTEGER NOT NULL DEFAULT 0,
    event_count INTEGER NOT NULL DEFAULT 0,
    director_class TEXT,
    has_skip_markers INTEGER NOT NULL DEFAULT 0,
    doc TEXT NOT NULL
);

CREATE TABLE sequence_binding (
    id INTEGER PRIMARY KEY,
    sequence_id INTEGER NOT NULL REFERENCES sequence(id),
    guid TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('spawnable', 'possessable')),
    template_class TEXT,
    map_unit_id INTEGER,
    doc TEXT NOT NULL
);

CREATE TABLE sequence_track (
    id INTEGER PRIMARY KEY,
    sequence_id INTEGER NOT NULL REFERENCES sequence(id),
    name TEXT NOT NULL,
    track_type TEXT NOT NULL,
    binding_guid TEXT,
    section_count INTEGER NOT NULL DEFAULT 0,
    start_frame REAL,
    end_frame REAL,
    doc TEXT NOT NULL
);

CREATE TABLE sequence_event (
    id INTEGER PRIMARY KEY,
    sequence_id INTEGER NOT NULL REFERENCES sequence(id),
    time_ticks INTEGER NOT NULL,
    frame REAL NOT NULL,
    time_secs REAL NOT NULL,
    function TEXT NOT NULL,
    track_name TEXT NOT NULL,
    binding_guid TEXT,
    talk_event_id TEXT,
    flag_id INTEGER,
    map_unit_id INTEGER,
    doc TEXT NOT NULL
);
