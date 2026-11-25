-- 04_world secondary and unique indexes (applied after bulk import).

CREATE UNIQUE INDEX idx_map_level_name ON map(level_name);
CREATE INDEX idx_map_group_id ON map(group_id);
CREATE INDEX idx_map_island ON map(island);
CREATE INDEX idx_map_region_id ON map(region_id);
CREATE INDEX idx_map_name_zh ON map(name_zh);
CREATE INDEX idx_map_name_key ON map(name_ns, name_key);
CREATE INDEX idx_map_map_type ON map(map_type);
CREATE INDEX idx_map_danger_level ON map(danger_level);

CREATE INDEX idx_island_name_zh ON island(name_zh);
CREATE INDEX idx_island_name_key ON island(name_ns, name_key);

CREATE INDEX idx_region_island_type ON region(island_type);

CREATE INDEX idx_save_point_command_item_id ON save_point_command(item_id);

CREATE INDEX idx_teleport_point_target_map_id ON teleport_point_target(map_id);
CREATE INDEX idx_teleport_point_target_teleport_id ON teleport_point_target(teleport_id);

CREATE INDEX idx_map_link_from ON map_link(from_map);
CREATE INDEX idx_map_link_to ON map_link(to_map);
CREATE INDEX idx_map_link_kind ON map_link(kind);

CREATE INDEX idx_treasure_box_map_id ON treasure_box(map_id);
CREATE INDEX idx_treasure_box_item_id ON treasure_box(item_id, map_id, id);

CREATE INDEX idx_item_category ON item(category);
CREATE INDEX idx_item_name_zh ON item(name_zh);
CREATE INDEX idx_item_name_key ON item(name_ns, name_key);
CREATE INDEX idx_item_buying_price ON item(buying_price);

CREATE INDEX idx_shop_name_zh ON shop(name_zh);
CREATE INDEX idx_shop_name_key ON shop(name_ns, name_key);

CREATE INDEX idx_shop_item_shop_id ON shop_item(shop_id);
CREATE INDEX idx_shop_item_item_id ON shop_item(item_id, shop_id, slot_index);

CREATE INDEX idx_various_shop_member_various_shop_id ON various_shop_member(various_shop_id);
CREATE INDEX idx_various_shop_member_shop_id ON various_shop_member(shop_id);

CREATE INDEX idx_quest_name_zh ON quest(name_zh);
CREATE INDEX idx_quest_chapter_no ON quest(chapter_no);
CREATE INDEX idx_quest_category ON quest(category);
CREATE INDEX idx_quest_requester_map_id ON quest(requester_map_id);
CREATE INDEX idx_quest_report_map_id ON quest(report_map_id);

CREATE INDEX idx_quest_prize_quest_id ON quest_prize(quest_id);
CREATE INDEX idx_quest_prize_item_id ON quest_prize(item_id, quest_id, prize_index);

CREATE UNIQUE INDEX idx_quest_board_valid_quest_id ON quest_board(quest_id) WHERE is_placeholder = 0;
CREATE INDEX idx_quest_board_name_zh ON quest_board(name_zh);

CREATE INDEX idx_quest_board_reward_item_id ON quest_board_reward(item_id, quest_id, reward_index);
CREATE INDEX idx_quest_board_reward_quest_id ON quest_board_reward(quest_id);

CREATE INDEX idx_main_scenario_chapter_progress ON main_scenario(chapter_no, min_progress);
CREATE INDEX idx_main_scenario_progress_range ON main_scenario(min_progress, progress_end, row_key);

CREATE UNIQUE INDEX idx_main_quest_chapter_no ON main_quest(chapter_no);

CREATE INDEX idx_flag_condition_term_flag_id ON flag_condition_term(flag_id);
CREATE INDEX idx_flag_condition_term_key ON flag_condition_term(condition_key);

CREATE INDEX idx_playable_unit_unit_id ON playable_unit(unit_id);
CREATE INDEX idx_playable_unit_name_zh ON playable_unit(name_zh);

CREATE INDEX idx_item_drop_item_id ON item_drop(item_id, unit_id, drop_kind);
CREATE INDEX idx_item_drop_unit_id ON item_drop(unit_id);

CREATE INDEX idx_world_name_lookup ON world_name(entity_kind, lang, name_lower, entity_id);
