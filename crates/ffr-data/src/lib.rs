//! Offline game data layer for FINAL FANTASY RESONANCE (`ffr-data`).
//!
//! Reads CUE4Parse JSON exports (`FFRS/Content/Datatable/` and `Localization/<lang>.json`) on
//! demand with caching, performs language-aware `FText` resolution (defaulting to `"zh-Hans"`
//! without silently passing Japanese source text off as Chinese), and provides typed queries for
//! Scenario tables (`DT_ChapterData`, `DT_MainScenario`).

pub mod assets;
pub mod battle;
pub mod datatable;
pub mod error;
pub mod flags;
pub mod ftext;
pub mod paths;
pub mod playable;
pub mod scenario;
pub mod store;
pub mod world;

pub use datatable::{
    DataTable, DataTableType, LocalizationCatalog, StringTable, load_data_table, load_localization,
    load_string_table, normalize_row_struct_name, parse_data_table_document,
    parse_localization_document, parse_string_table_document,
};
pub use error::{DataError, Result};
pub use flags::{
    FLAG_CONDITION_TABLE, FlagConditionData, FlagConditionEntry, FlagTerm, load_flag_conditions,
    parse_flag_condition_expr, parse_flag_conditions,
};
pub use ftext::{
    DEFAULT_LOCRES_LANGUAGES, FTextValue, MODEL_LANGUAGE, NATIVE_LANGUAGE, ResolveOptions,
    ResolvedText, TextResolver, TextResolverOptions, TextSource, parse_ftext, resolve_ftext,
    resolve_model_text,
};
pub use paths::{
    FFR_DEPS_ENV, datatable_dir, datatable_relative_path_to_json, export_dir_from_deps,
    export_dir_from_env, export_dir_from_env_value, is_safe_language_tag, localization_dir,
    localization_file, resolve_export_dir, ue_object_path_to_export_json,
};
pub use playable::{
    MAP_UNIT_ASSET_TABLE, MAP_UNIT_PLAYABLE_TABLE, MapUnitPlayable, MapUnitPlayableData,
    ResolvedMapPlayableUnit, load_map_unit_playable, parse_map_unit_playable,
};
pub use scenario::{
    CHAPTER_DATA_TABLE, Chapter, CurrentMainScenario, MAIN_QUEST_ID_BASE, MAIN_SCENARIO_TABLE,
    MainQuest, MainScenarioEntry, QUEST_MENU_STRING_TABLE, ResolvedMainQuest,
    ResolvedMainScenarioTexts, ScenarioData, ScenarioDestination, chapter_no_from_main_quest_id,
    chapter_no_from_progress, find_chapter, get_chapter_main_scenario, load_chapters,
    load_main_scenario, main_quest_id_for_chapter, parse_chapters, parse_main_progress_condition,
    parse_main_scenario,
};
pub use store::DataStore;
