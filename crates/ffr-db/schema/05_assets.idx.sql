-- 05_assets secondary indexes, created after bulk import.

CREATE INDEX idx_asset_primary_class ON asset(primary_class);
CREATE INDEX idx_asset_category ON asset(category);
CREATE INDEX idx_asset_tag_key_value_short ON asset_tag(key, value_short);
CREATE INDEX idx_asset_ref_from ON asset_ref(from_asset_id);
CREATE INDEX idx_asset_ref_to_id ON asset_ref(to_asset_id);
CREATE INDEX idx_asset_ref_to_package ON asset_ref(to_package);
CREATE INDEX idx_config_entry_section_key ON config_entry(section, key);
