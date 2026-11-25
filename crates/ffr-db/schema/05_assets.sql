-- 05_assets: AssetRegistry entries, tags, exported package references, and INI entries.

CREATE TABLE asset (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    package_name TEXT NOT NULL UNIQUE,
    primary_class TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    size INTEGER NOT NULL,
    package_size INTEGER NOT NULL,
    bulk_size INTEGER NOT NULL,
    category TEXT NOT NULL,
    decision TEXT NOT NULL,
    has_export INTEGER NOT NULL CHECK (has_export IN (0, 1)),
    export_path TEXT,
    doc TEXT NOT NULL
);

CREATE TABLE asset_tag (
    asset_id INTEGER NOT NULL REFERENCES asset(id),
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    value_short TEXT,
    PRIMARY KEY (asset_id, key)
);

CREATE TABLE asset_ref (
    from_asset_id INTEGER NOT NULL REFERENCES asset(id),
    to_package TEXT NOT NULL,
    to_asset_id INTEGER REFERENCES asset(id),
    kind TEXT NOT NULL CHECK (kind IN ('hard', 'soft', 'script')),
    PRIMARY KEY (from_asset_id, to_package, kind)
);

CREATE TABLE config_entry (
    file TEXT NOT NULL,
    section TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    op TEXT NOT NULL CHECK (op IN ('=', '+', '-', '.', '!')),
    ord INTEGER NOT NULL,
    PRIMARY KEY (file, ord)
);
