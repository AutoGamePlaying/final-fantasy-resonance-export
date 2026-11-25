-- 06_reflection: Unreal Engine reflection metadata parsed from .jmap dumps.
-- Covers classes, structs, enums (ue_class), properties (ue_property),
-- functions (ue_function), and enum values (ue_enum_value).

CREATE TABLE ue_class (
    path TEXT PRIMARY KEY,
    short_name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('class', 'struct', 'enum')),
    package TEXT NOT NULL,
    super_class TEXT,
    size INTEGER NOT NULL DEFAULT 0,
    flags TEXT NOT NULL DEFAULT '',
    doc TEXT NOT NULL
);

CREATE TABLE ue_property (
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    offset INTEGER NOT NULL,
    size INTEGER NOT NULL,
    inner TEXT,
    ord INTEGER NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (owner, name)
);

CREATE TABLE ue_function (
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    flags TEXT NOT NULL DEFAULT '',
    params TEXT NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (owner, name)
);

CREATE TABLE ue_enum_value (
    enum TEXT NOT NULL,
    name TEXT NOT NULL,
    short_name TEXT NOT NULL,
    value INTEGER NOT NULL,
    ord INTEGER NOT NULL,
    PRIMARY KEY (enum, name)
);
