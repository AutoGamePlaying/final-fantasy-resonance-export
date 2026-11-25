-- 07_input_ui indexes: executed after bulk import.

CREATE INDEX idx_input_mapping_action_id
    ON input_mapping (action_id, context_id, ord);

CREATE INDEX idx_input_mapping_key
    ON input_mapping (key, context_id, ord);

CREATE INDEX idx_input_set_context_id
    ON input_set (context_id, device);

CREATE INDEX idx_input_set_device
    ON input_set (device, context_id);

CREATE INDEX idx_input_set_action_action_id
    ON input_set_action (action_id, asset);

CREATE INDEX idx_input_set_action_semantic
    ON input_set_action (semantic, action_id);

CREATE INDEX idx_data_asset_class
    ON data_asset (class, path);

CREATE INDEX idx_blueprint_class_name
    ON blueprint (class_name, path);

CREATE INDEX idx_blueprint_parent_class
    ON blueprint (parent_class, path);

CREATE INDEX idx_blueprint_kind
    ON blueprint (kind, path);

CREATE INDEX idx_blueprint_component_class
    ON blueprint_component (class, blueprint_id);

CREATE INDEX idx_widget_node_blueprint_ord
    ON widget_node (blueprint_id, ord, name);

CREATE INDEX idx_widget_node_class
    ON widget_node (class, blueprint_id);
