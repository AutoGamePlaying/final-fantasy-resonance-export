-- 02_datatable indexes. UNIQUE(path) and PRIMARY KEY(table_id, row_name) already provide lookup
-- indexes; this named path index makes the prefix-range plan explicit.

CREATE INDEX idx_dt_table_path ON dt_table(path);
