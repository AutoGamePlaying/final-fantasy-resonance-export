-- 10_snapshots indexes (created after bulk data import).

CREATE INDEX idx_snapshot_label ON snapshot(label);
CREATE INDEX idx_snapshot_time ON snapshot(time);

CREATE INDEX idx_snapshot_object_snapshot_class_short ON snapshot_object(snapshot_id, class_short);
CREATE INDEX idx_snapshot_object_class_short ON snapshot_object(class_short);
CREATE INDEX idx_snapshot_object_class ON snapshot_object(class);

CREATE INDEX idx_snapshot_widget_snapshot ON snapshot_widget(snapshot_id);
CREATE INDEX idx_snapshot_widget_class_short ON snapshot_widget(class_short);

CREATE INDEX idx_snapshot_text_snapshot ON snapshot_text(snapshot_id);
CREATE INDEX idx_snapshot_text_text ON snapshot_text(text);

CREATE INDEX idx_marker_source_seq ON marker(source, seq);
CREATE INDEX idx_marker_time ON marker(time);
