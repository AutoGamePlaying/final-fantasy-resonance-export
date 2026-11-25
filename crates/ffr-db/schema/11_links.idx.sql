-- 11_links indexes: executed after bulk import.

CREATE INDEX idx_talk_event_sequence_id ON talk_event(sequence_id);
CREATE INDEX idx_talk_event_sequence_path ON talk_event(sequence_path);
CREATE INDEX idx_talk_event_talk_table_path ON talk_event(talk_table_path);
CREATE INDEX idx_talk_event_encount_group_id ON talk_event(encount_group_id, id);
CREATE INDEX idx_talk_event_type ON talk_event(event_type);

CREATE INDEX idx_talk_event_next_event ON talk_event_next(event, kind, ord);
CREATE INDEX idx_talk_event_next_next_event ON talk_event_next(next_event);

CREATE INDEX idx_talk_line_table_ord ON talk_line(table_path, ord);
CREATE INDEX idx_talk_line_table_talk_ord ON talk_line(table_path, talk_id, ord);
CREATE INDEX idx_talk_line_speaker ON talk_line(speaker);
CREATE INDEX idx_talk_line_text_key ON talk_line(text_ns, text_key);

CREATE INDEX idx_level_object_talk_object ON level_object_talk(level_object_id, ord);
CREATE INDEX idx_level_object_talk_event ON level_object_talk(talk_event_id, level_object_id);

CREATE INDEX idx_map_unit_row_key ON map_unit(row_key);
CREATE INDEX idx_map_unit_name_zh ON map_unit(name_zh);

CREATE INDEX idx_class_ref_normalized ON class_ref(normalized);
CREATE INDEX idx_class_ref_ue_class_id ON class_ref(ue_class_id);
CREATE INDEX idx_class_ref_blueprint_id ON class_ref(blueprint_id);
