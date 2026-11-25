-- 09_sequences indexes: lookup by category/name, bindings by sequence/template_class/unit,
-- tracks by sequence/type, and events by sequence/function/talk_event_id/flag_id/map_unit_id.

CREATE INDEX idx_sequence_category ON sequence(category);
CREATE INDEX idx_sequence_name ON sequence(name);

CREATE INDEX idx_sequence_binding_sequence_id ON sequence_binding(sequence_id);
CREATE INDEX idx_sequence_binding_template_class ON sequence_binding(template_class);
CREATE INDEX idx_sequence_binding_map_unit_id ON sequence_binding(map_unit_id);

CREATE INDEX idx_sequence_track_sequence_id ON sequence_track(sequence_id);
CREATE INDEX idx_sequence_track_type ON sequence_track(track_type);

CREATE INDEX idx_sequence_event_sequence_id ON sequence_event(sequence_id, time_ticks);
CREATE INDEX idx_sequence_event_function ON sequence_event(function);
CREATE INDEX idx_sequence_event_talk_event_id ON sequence_event(talk_event_id);
CREATE INDEX idx_sequence_event_flag_id ON sequence_event(flag_id);
CREATE INDEX idx_sequence_event_map_unit_id ON sequence_event(map_unit_id);
