-- 03_battle indexes (created after bulk data import).

CREATE INDEX idx_unit_name_zh ON unit(name_zh);
CREATE INDEX idx_unit_name_key ON unit(name_ns, name_key);
CREATE INDEX idx_unit_category ON unit(category, id);
CREATE INDEX idx_unit_is_boss ON unit(is_boss, id);
CREATE INDEX idx_unit_level ON unit(level, id);

CREATE INDEX idx_unit_name_lang_name ON unit_name(lang, name, unit_id);

CREATE INDEX idx_unit_resistance_elem_res ON unit_resistance(element, resistance, unit_id);

CREATE INDEX idx_skill_name_zh ON skill(name_zh);
CREATE INDEX idx_skill_name_key ON skill(name_ns, name_key);
CREATE INDEX idx_skill_attr_type ON skill(attr_type, id);
CREATE INDEX idx_skill_element ON skill(element, id);
CREATE INDEX idx_skill_target_type ON skill(target_type, id);

CREATE INDEX idx_skill_name_lang_name ON skill_name(lang, name, skill_id);

CREATE INDEX idx_unit_skill_unit ON unit_skill(unit_id, source, level, skill_id);
CREATE INDEX idx_unit_skill_skill ON unit_skill(skill_id, unit_id);
CREATE INDEX idx_unit_skill_source ON unit_skill(source, unit_id);

CREATE INDEX idx_command_skill_name_zh ON command_skill(name_zh);
CREATE INDEX idx_command_skill_unit_id ON command_skill(unit_id, id);

CREATE INDEX idx_battle_group_is_boss ON battle_group(is_boss, id);
CREATE INDEX idx_battle_group_is_bounty ON battle_group(is_bounty, id);

CREATE INDEX idx_battle_group_member_unit ON battle_group_member(unit_id, group_id);

CREATE INDEX idx_encounter_battle_group ON encounter(battle_group_id, pool_path);
CREATE INDEX idx_encounter_region ON encounter(region, pool_path);

CREATE INDEX idx_unit_ai_ai_id ON unit_ai(ai_id);

CREATE INDEX idx_enemy_chapter_unit ON enemy_chapter(unit_id, chapter_no);
CREATE INDEX idx_enemy_chapter_chapter ON enemy_chapter(chapter_no, unit_id);
CREATE INDEX idx_enemy_chapter_bg ON enemy_chapter(battle_group_id, unit_id);
