-- 06_reflection indexes executed after bulk import.

CREATE INDEX idx_ue_class_short_name ON ue_class (short_name);
CREATE INDEX idx_ue_class_super_class ON ue_class (super_class);
CREATE INDEX idx_ue_class_kind_short_name ON ue_class (kind, short_name);
CREATE INDEX idx_ue_class_package ON ue_class (package);

CREATE INDEX idx_ue_property_owner ON ue_property (owner, offset, ord);
CREATE INDEX idx_ue_property_name ON ue_property (name);

CREATE INDEX idx_ue_function_owner ON ue_function (owner);
CREATE INDEX idx_ue_function_name ON ue_function (name);

CREATE INDEX idx_ue_enum_value_enum_value ON ue_enum_value (enum, value, ord);
CREATE INDEX idx_ue_enum_value_name ON ue_enum_value (name);
CREATE INDEX idx_ue_enum_value_enum_short_name ON ue_enum_value (enum, short_name);
