//! Lazy, cached facade over an exported FFR data directory.
//!
//! Constructing a [`DataStore`] performs no disk I/O: `DataTable` / `CompositeDataTable`,
//! `StringTable`, and `Localization/<lang>.json` files are loaded on first access and cached in
//! memory for subsequent queries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde_json::Value;

use crate::datatable::{DataTable, LocalizationCatalog, StringTable, load_data_table};
use crate::error::{DataError, Result};
use crate::ftext::{FTextValue, ResolveOptions, ResolvedText, TextResolver, TextResolverOptions};
use crate::paths::{
    datatable_relative_path_to_json, export_dir_from_env, resolve_export_dir,
    ue_object_path_to_export_json,
};
use crate::scenario::{
    CHAPTER_DATA_TABLE, Chapter, MAIN_SCENARIO_TABLE, MainScenarioEntry, ScenarioData,
    parse_chapters, parse_main_scenario,
};

/// Lazy, thread-safe, cached reader over an exported FFR data directory.
#[derive(Debug, Clone)]
pub struct DataStore {
    export_root: PathBuf,
    data_tables: Arc<RwLock<HashMap<PathBuf, Arc<DataTable>>>>,
    text_resolver: TextResolver,
}

impl DataStore {
    /// Create a store rooted at an explicit export directory. No files are read eagerly.
    pub fn new(export_root: impl Into<PathBuf>) -> Self {
        Self::with_text_options(export_root, TextResolverOptions::default())
    }

    /// Create a store rooted at an explicit export directory with custom text resolver options.
    pub fn with_text_options(
        export_root: impl Into<PathBuf>,
        options: TextResolverOptions,
    ) -> Self {
        let export_root = export_root.into();
        let text_resolver = TextResolver::with_options(export_root.clone(), options);
        Self {
            export_root,
            data_tables: Arc::new(RwLock::new(HashMap::new())),
            text_resolver,
        }
    }

    /// Create a store rooted at `$FFR_DEPS/ffr/export`.
    pub fn from_env() -> Result<Self> {
        Ok(Self::new(export_dir_from_env()?))
    }

    /// Create a store rooted at `$FFR_DEPS/ffr/export` with custom text resolver options.
    pub fn from_env_with_text_options(options: TextResolverOptions) -> Result<Self> {
        Ok(Self::with_text_options(export_dir_from_env()?, options))
    }

    /// Open a store using an explicit export directory when given, otherwise `$FFR_DEPS/ffr/export`.
    pub fn open(explicit_export_root: Option<&Path>) -> Result<Self> {
        Ok(Self::new(resolve_export_dir(explicit_export_root)?))
    }

    /// Root directory of the CUE4Parse export tree.
    pub fn export_root(&self) -> &Path {
        &self.export_root
    }

    /// Shared cached [`TextResolver`] backing this store.
    pub fn text_resolver(&self) -> &TextResolver {
        &self.text_resolver
    }

    /// Load and cache a `DataTable` or `CompositeDataTable` by relative path under
    /// `FFRS/Content/Datatable/` (for example `"Scenario/DT_ChapterData"` or
    /// `"Map/CDT_MapData_Prod.json"`).
    pub fn data_table(&self, relative_path: &str) -> Result<Arc<DataTable>> {
        let json_path = datatable_relative_path_to_json(relative_path, &self.export_root)
            .ok_or_else(|| DataError::InvalidPath {
                kind: "数据表",
                input: relative_path.to_owned(),
            })?;
        self.load_data_table_by_path(&json_path)
    }

    /// Load and cache a `DataTable` or `CompositeDataTable` by its UE object/package path
    /// (for example `"/Game/Datatable/Map/CDT_MapData_Prod"`).
    pub fn data_table_by_ue_path(&self, ue_path: &str) -> Result<Arc<DataTable>> {
        let json_path =
            ue_object_path_to_export_json(ue_path, &self.export_root).ok_or_else(|| {
                DataError::InvalidPath {
                    kind: "数据表",
                    input: ue_path.to_owned(),
                }
            })?;
        self.load_data_table_by_path(&json_path)
    }

    /// Load and cache a `StringTable` by relative path under `FFRS/Content/Datatable/`
    /// (for example `"Strings/ChapterTitle/ST_ChapterTitle"`).
    pub fn string_table(&self, relative_path: &str) -> Result<Arc<StringTable>> {
        let json_path = datatable_relative_path_to_json(relative_path, &self.export_root)
            .ok_or_else(|| DataError::InvalidPath {
                kind: "字符串表",
                input: relative_path.to_owned(),
            })?;
        self.text_resolver.load_string_table_strict(&json_path)
    }

    /// Load and cache a `StringTable` by its UE `TableId`
    /// (for example `"/Game/Datatable/Strings/ChapterTitle/ST_ChapterTitle.ST_ChapterTitle"`).
    pub fn string_table_by_id(&self, table_id: &str) -> Result<Arc<StringTable>> {
        let json_path =
            ue_object_path_to_export_json(table_id, &self.export_root).ok_or_else(|| {
                DataError::InvalidPath {
                    kind: "字符串表",
                    input: table_id.to_owned(),
                }
            })?;
        self.text_resolver.load_string_table_strict(&json_path)
    }

    /// Load and cache an exported `Localization/<language>.json` catalog.
    pub fn localization(&self, language: &str) -> Result<Arc<LocalizationCatalog>> {
        self.text_resolver.load_localization_strict(language)
    }

    /// Resolve a parsed `FTextValue` in the store's default language (`"zh-Hans"` by default).
    pub fn resolve_text(&self, value: &FTextValue) -> ResolvedText {
        self.text_resolver.resolve(value)
    }

    /// Resolve a parsed `FTextValue` in an explicit target language.
    pub fn resolve_text_in(&self, value: &FTextValue, language: &str) -> ResolvedText {
        self.text_resolver.resolve_in(value, language)
    }

    /// Resolve a parsed `FTextValue` with optional language and fallback overrides.
    pub fn resolve_text_with(
        &self,
        value: &FTextValue,
        language: Option<&str>,
        options: ResolveOptions,
    ) -> ResolvedText {
        self.text_resolver.resolve_with(value, language, options)
    }

    /// Parse and resolve a raw JSON `FText` value in the store's default language.
    pub fn resolve_value(&self, value: &Value) -> ResolvedText {
        self.text_resolver.resolve_value(value)
    }

    /// Load typed chapter records via the cached `Scenario/DT_ChapterData` table.
    pub fn chapters(&self) -> Result<Vec<Chapter>> {
        let table = self.data_table(CHAPTER_DATA_TABLE)?;
        let label = datatable_relative_path_to_json(CHAPTER_DATA_TABLE, &self.export_root)
            .map_or_else(
                || CHAPTER_DATA_TABLE.to_owned(),
                |path| path.display().to_string(),
            );
        parse_chapters(&table, &label)
    }

    /// Load typed main scenario records via the cached `Scenario/DT_MainScenario` table.
    pub fn main_scenario(&self) -> Result<Vec<MainScenarioEntry>> {
        let table = self.data_table(MAIN_SCENARIO_TABLE)?;
        let label = datatable_relative_path_to_json(MAIN_SCENARIO_TABLE, &self.export_root)
            .map_or_else(
                || MAIN_SCENARIO_TABLE.to_owned(),
                |path| path.display().to_string(),
            );
        parse_main_scenario(&table, &label)
    }

    /// Build a [`ScenarioData`] handle backed by this store's cached tables and [`TextResolver`].
    pub fn scenario(&self) -> Result<ScenarioData> {
        let chapters = self.chapters()?;
        let main_scenario = self.main_scenario()?;
        Ok(ScenarioData {
            chapters,
            main_scenario,
            text_resolver: self.text_resolver.clone(),
        })
    }

    /// Number of `DataTable` / `CompositeDataTable` entries currently cached in memory.
    pub fn cached_data_table_count(&self) -> usize {
        self.data_tables
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// Number of `StringTable` entries currently cached in memory.
    pub fn cached_string_table_count(&self) -> usize {
        self.text_resolver.cached_string_table_count()
    }

    /// Number of `LocalizationCatalog` entries currently cached in memory.
    pub fn cached_localization_count(&self) -> usize {
        self.text_resolver.cached_localization_count()
    }

    fn load_data_table_by_path(&self, json_path: &Path) -> Result<Arc<DataTable>> {
        {
            let cache = self
                .data_tables
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(table) = cache.get(json_path) {
                return Ok(Arc::clone(table));
            }
        }
        let table = Arc::new(load_data_table(json_path)?);
        let mut cache = self
            .data_tables
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(json_path.to_path_buf(), Arc::clone(&table));
        Ok(table)
    }
}
