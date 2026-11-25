-- 08_levels indexes (created after bulk data import).

CREATE INDEX idx_level_map_id ON level(map_id);
CREATE INDEX idx_level_sublevel_type ON level(sublevel_type);
CREATE INDEX idx_level_persistent_level ON level(persistent_level);
CREATE INDEX idx_level_raw_name ON level(raw_name);

CREATE INDEX idx_level_map_map_id ON level_map(map_id, level_id);

CREATE INDEX idx_actor_level_id ON actor(level_id);
CREATE INDEX idx_actor_class ON actor(class);

CREATE INDEX idx_level_object_map_kind ON level_object(map_id, kind);
CREATE INDEX idx_level_object_kind_ref ON level_object(kind, ref_id);
CREATE INDEX idx_level_object_map_unique ON level_object(map_id, unique_id);
CREATE INDEX idx_level_object_actor_kind ON level_object(actor_id, kind);
CREATE INDEX idx_level_object_level_id ON level_object(level_id);

CREATE INDEX idx_map_edge_from_map ON map_edge(from_map);
CREATE INDEX idx_map_edge_to_map ON map_edge(to_map);

CREATE INDEX idx_map_shop_map_id ON map_shop(map_id);
CREATE INDEX idx_map_shop_shop_id ON map_shop(shop_id);
CREATE INDEX idx_map_shop_source_shop_id ON map_shop(source_shop_id);
