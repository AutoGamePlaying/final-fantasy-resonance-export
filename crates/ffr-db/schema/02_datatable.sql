-- 02_datatable: Raw exported DataTable metadata and rows.

CREATE TABLE dt_table (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('DataTable', 'CompositeDataTable')),
    row_struct TEXT NOT NULL,
    row_count INTEGER NOT NULL CHECK (row_count >= 0)
);

CREATE TABLE dt_row (
    table_id INTEGER NOT NULL,
    row_name TEXT NOT NULL,
    doc TEXT NOT NULL,
    PRIMARY KEY (table_id, row_name),
    FOREIGN KEY (table_id) REFERENCES dt_table(id)
);
