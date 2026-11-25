-- 01_text: Localization text, FTS5 trigram index, and StringTable entries.

CREATE TABLE text (
    id INTEGER NOT NULL UNIQUE,
    ns TEXT NOT NULL,
    key TEXT NOT NULL,
    lang TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (ns, key, lang)
);

CREATE TABLE text_language (
    lang TEXT PRIMARY KEY
);

CREATE TABLE string_table (
    table_id TEXT PRIMARY KEY,
    ns TEXT NOT NULL
);

CREATE TABLE string_table_entry (
    table_id TEXT NOT NULL,
    key TEXT NOT NULL,
    source TEXT NOT NULL,
    PRIMARY KEY (table_id, key),
    FOREIGN KEY (table_id) REFERENCES string_table(table_id)
);

CREATE VIRTUAL TABLE text_fts USING fts5(
    value,
    content='text',
    content_rowid='id',
    tokenize='trigram'
);
