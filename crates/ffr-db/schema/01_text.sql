-- 01_text: Localization text (one row per (ns, key) with ja/en/zh columns),
-- sparse per-language untranslated entries, language-to-column mapping,
-- 3-column FTS5 trigram index over `text`, and StringTable entries.

CREATE TABLE text (
    id INTEGER PRIMARY KEY,
    ns TEXT NOT NULL,
    key TEXT NOT NULL,
    ja TEXT,
    en TEXT,
    zh TEXT,
    UNIQUE (ns, key)
);

CREATE TABLE text_untranslated (
    ns TEXT NOT NULL,
    key TEXT NOT NULL,
    lang TEXT NOT NULL,
    value TEXT NOT NULL,
    reason TEXT NOT NULL,
    PRIMARY KEY (ns, key, lang)
);

CREATE TABLE text_language (
    lang TEXT PRIMARY KEY,
    column_name TEXT NOT NULL UNIQUE
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
    ja,
    en,
    zh,
    content='text',
    content_rowid='id',
    tokenize='trigram'
);
