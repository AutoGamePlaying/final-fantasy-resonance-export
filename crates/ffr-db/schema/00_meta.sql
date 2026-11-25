-- 00_meta: Build metadata, data sources, per-stage reports, and non-fatal build issues.

CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE source (
    stage TEXT NOT NULL,
    path TEXT NOT NULL,
    manifest_sha256 TEXT,
    file_count INTEGER,
    PRIMARY KEY (stage, path)
);

CREATE TABLE stage_report (
    stage TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK (status IN ('ok', 'skipped', 'failed')),
    rows INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT ''
);

CREATE TABLE build_issue (
    stage TEXT NOT NULL,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail TEXT NOT NULL
);
