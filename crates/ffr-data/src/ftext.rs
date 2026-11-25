//! Parsing and language-aware resolution for Unreal Engine `FText` values exported by CUE4Parse.
//!
//! Lookup follows UE's localization order: look up `(namespace, key)` in the requested language's
//! exported locres JSON (`<export_root>/Localization/<lang>.json`) first; only when absent do we
//! consider the source text (exported `StringTable` or inline `SourceString`).
//!
//! Non-native lookups never silently return Japanese source text labeled as the target language:
//! by default they return `text: None, lang: None, source: TextSource::Missing` with a Chinese
//! reason; when `fallback_to_native` is explicitly enabled they return `lang: Some("ja")` and
//! record the fallback in `reason`. Model-facing helpers default to `"zh-Hans"`.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::datatable::{LocalizationCatalog, StringTable, load_localization, load_string_table};
use crate::error::{DataError, Result};
use crate::paths::{is_safe_language_tag, localization_file, ue_object_path_to_export_json};

/// Native source language of `StringTable` assets and inline `SourceString` fields.
pub const NATIVE_LANGUAGE: &str = "ja";
/// Default target language used when presenting text to the AI model.
pub const MODEL_LANGUAGE: &str = "zh-Hans";
/// Standard exported locres language set.
pub const DEFAULT_LOCRES_LANGUAGES: [&str; 3] = ["zh-Hans", "en", "ja"];

/// Parsed CUE4Parse `FText` JSON value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FTextValue {
    /// Reference into a `StringTable` asset (`{ TableId, Key, SourceString?, LocalizedString? }`).
    #[serde(rename_all = "camelCase")]
    StringTable {
        table_id: String,
        key: String,
        source_string: Option<String>,
        localized_string: Option<String>,
    },
    /// Inline localized text without a `TableId` (`{ Namespace, Key, SourceString?, LocalizedString? }`).
    #[serde(rename_all = "camelCase")]
    Inline {
        namespace: String,
        key: String,
        source_string: Option<String>,
        localized_string: Option<String>,
    },
    /// Culture-invariant or empty text (`{ CultureInvariantString: string | null }`).
    #[serde(rename_all = "camelCase")]
    CultureInvariant { value: Option<String> },
}

/// Origin of a resolved text value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextSource {
    Localization,
    StringTable,
    CultureInvariant,
    InlineSource,
    Missing,
}

/// Result of resolving one `FText` in a requested language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedText {
    /// Resolved string, or `None` when missing.
    pub text: Option<String>,
    /// True language of `text`, or `None` when `text` is `None`.
    pub lang: Option<String>,
    /// Language requested by the caller.
    pub requested_language: String,
    /// Chinese explanation when missing or when an explicit fallback occurred.
    pub reason: Option<String>,
    /// Where `text` came from.
    pub source: TextSource,
}

impl ResolvedText {
    /// Returns `true` when text was resolved (directly or via explicit fallback).
    pub fn is_found(&self) -> bool {
        self.text.is_some()
    }

    /// Returns `true` when text came from a fallback language different from `requested_language`.
    pub fn is_fallback(&self) -> bool {
        self.text.is_some() && self.lang.as_deref() != Some(self.requested_language.as_str())
    }
}

/// Configuration for [`TextResolver`].
#[derive(Debug, Clone)]
pub struct TextResolverOptions {
    /// Default target language when resolving without an explicit language (default `"zh-Hans"`).
    pub language: String,
    /// Source language of `StringTable` and inline `SourceString` (default `"ja"`).
    pub native_language: String,
    /// When `true`, missing non-native translations fall back to source text while reporting
    /// `lang = Some(native_language)` and noting the fallback in `reason`. Defaults to `false`.
    pub fallback_to_native: bool,
    /// Optional preloaded localization catalogs injected by language code.
    pub localizations: HashMap<String, Arc<LocalizationCatalog>>,
}

impl Default for TextResolverOptions {
    fn default() -> Self {
        Self {
            language: MODEL_LANGUAGE.to_owned(),
            native_language: NATIVE_LANGUAGE.to_owned(),
            fallback_to_native: false,
            localizations: HashMap::new(),
        }
    }
}

/// Per-call override options for resolving a single `FText`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolveOptions {
    pub fallback_to_native: Option<bool>,
}

impl ResolveOptions {
    /// Convenience constructor that enables explicit fallback to the native source language.
    pub const fn with_fallback() -> Self {
        Self {
            fallback_to_native: Some(true),
        }
    }

    /// Convenience constructor that enforces strict target-language resolution.
    pub const fn strict() -> Self {
        Self {
            fallback_to_native: Some(false),
        }
    }
}

#[derive(Debug, Clone)]
enum CachedStringTable {
    Loaded(Arc<StringTable>),
    Failed(String),
}

#[derive(Debug, Clone)]
enum CachedLocalization {
    Loaded(Arc<LocalizationCatalog>),
    NotFound(String),
    Error(String),
}

/// Cached, thread-safe, language-aware `FText` resolver over one export tree.
///
/// Cloning a `TextResolver` shares its underlying `StringTable` and `LocalizationCatalog` caches.
#[derive(Debug, Clone)]
pub struct TextResolver {
    export_root: PathBuf,
    language: String,
    native_language: String,
    fallback_to_native: bool,
    string_tables: Arc<RwLock<HashMap<PathBuf, CachedStringTable>>>,
    localizations: Arc<RwLock<HashMap<String, CachedLocalization>>>,
}

impl TextResolver {
    /// Create a resolver with default options (`language = "zh-Hans"`, `native_language = "ja"`,
    /// `fallback_to_native = false`). Performs no disk I/O until first lookup.
    pub fn new(export_root: impl Into<PathBuf>) -> Self {
        Self::with_options(export_root, TextResolverOptions::default())
    }

    /// Create a resolver with custom options.
    pub fn with_options(export_root: impl Into<PathBuf>, options: TextResolverOptions) -> Self {
        let mut initial_localizations = HashMap::with_capacity(options.localizations.len());
        for (language, catalog) in options.localizations {
            initial_localizations.insert(language, CachedLocalization::Loaded(catalog));
        }
        Self {
            export_root: export_root.into(),
            language: options.language,
            native_language: options.native_language,
            fallback_to_native: options.fallback_to_native,
            string_tables: Arc::new(RwLock::new(HashMap::new())),
            localizations: Arc::new(RwLock::new(initial_localizations)),
        }
    }

    pub fn export_root(&self) -> &Path {
        &self.export_root
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn native_language(&self) -> &str {
        &self.native_language
    }

    pub fn fallback_to_native(&self) -> bool {
        self.fallback_to_native
    }

    /// Resolve a parsed `FTextValue` in the resolver's default language (`"zh-Hans"` by default).
    pub fn resolve(&self, value: &FTextValue) -> ResolvedText {
        self.resolve_with(value, None, ResolveOptions::default())
    }

    /// Resolve a parsed `FTextValue` in an explicit target language.
    pub fn resolve_in(&self, value: &FTextValue, language: &str) -> ResolvedText {
        self.resolve_with(value, Some(language), ResolveOptions::default())
    }

    /// Resolve a parsed `FTextValue` with optional language and per-call fallback overrides.
    pub fn resolve_with(
        &self,
        value: &FTextValue,
        language: Option<&str>,
        options: ResolveOptions,
    ) -> ResolvedText {
        let language = language.unwrap_or(&self.language);
        let fallback_to_native = options
            .fallback_to_native
            .unwrap_or(self.fallback_to_native);
        match value {
            FTextValue::CultureInvariant { value } => match value {
                None => missing(language, "FText 为空（CultureInvariantString 为 null）"),
                Some(text) => ResolvedText {
                    text: Some(text.clone()),
                    lang: Some(language.to_owned()),
                    requested_language: language.to_owned(),
                    reason: None,
                    source: TextSource::CultureInvariant,
                },
            },
            FTextValue::StringTable { table_id, key, .. } => {
                self.resolve_string_table_text(table_id, key, language, fallback_to_native)
            }
            FTextValue::Inline {
                namespace,
                key,
                source_string,
                ..
            } => self.resolve_inline_text(
                namespace,
                key,
                source_string.as_deref(),
                language,
                fallback_to_native,
            ),
        }
    }

    /// Parse and resolve a raw JSON `FText` value using the resolver's default language.
    pub fn resolve_value(&self, value: &Value) -> ResolvedText {
        self.resolve_value_with(value, None, ResolveOptions::default())
    }

    /// Parse and resolve a raw JSON `FText` value with optional language and fallback overrides.
    pub fn resolve_value_with(
        &self,
        value: &Value,
        language: Option<&str>,
        options: ResolveOptions,
    ) -> ResolvedText {
        let target_language = language.unwrap_or(&self.language);
        let Some(parsed) = parse_ftext(value) else {
            return missing(target_language, "不是合法的 FText 对象");
        };
        self.resolve_with(&parsed, Some(target_language), options)
    }

    /// Convenience helper for model-facing text: always resolves in `"zh-Hans"`.
    pub fn resolve_model_text(&self, value: &FTextValue) -> ResolvedText {
        self.resolve_with(value, Some(MODEL_LANGUAGE), ResolveOptions::default())
    }

    /// Convenience helper for model-facing text with explicit fallback override.
    pub fn resolve_model_text_with(
        &self,
        value: &FTextValue,
        options: ResolveOptions,
    ) -> ResolvedText {
        self.resolve_with(value, Some(MODEL_LANGUAGE), options)
    }

    /// Load and cache a `StringTable` by its UE `TableId`, returning `None` if unavailable.
    pub fn load_string_table_by_id(&self, table_id: &str) -> Option<Arc<StringTable>> {
        match self.load_table_cached(table_id) {
            CachedStringTable::Loaded(table) => Some(table),
            CachedStringTable::Failed(_) => None,
        }
    }

    /// Load and cache a `LocalizationCatalog` by language code, returning `None` if unavailable.
    pub fn load_localization_catalog(&self, language: &str) -> Option<Arc<LocalizationCatalog>> {
        match self.load_catalog_cached(language) {
            CachedLocalization::Loaded(catalog) => Some(catalog),
            CachedLocalization::NotFound(_) | CachedLocalization::Error(_) => None,
        }
    }

    pub(crate) fn load_string_table_strict(&self, json_path: &Path) -> Result<Arc<StringTable>> {
        {
            let cache = self
                .string_tables
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(CachedStringTable::Loaded(table)) = cache.get(json_path) {
                return Ok(Arc::clone(table));
            }
        }
        let table = Arc::new(load_string_table(json_path)?);
        let mut cache = self
            .string_tables
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            json_path.to_path_buf(),
            CachedStringTable::Loaded(Arc::clone(&table)),
        );
        Ok(table)
    }

    pub(crate) fn load_localization_strict(
        &self,
        language: &str,
    ) -> Result<Arc<LocalizationCatalog>> {
        {
            let cache = self
                .localizations
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(CachedLocalization::Loaded(catalog)) = cache.get(language) {
                return Ok(Arc::clone(catalog));
            }
        }
        if !is_safe_language_tag(language) {
            return Err(DataError::InvalidLanguage {
                language: language.to_owned(),
            });
        }
        let json_path = localization_file(&self.export_root, language);
        let catalog = Arc::new(load_localization(&json_path)?);
        let mut cache = self
            .localizations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            language.to_owned(),
            CachedLocalization::Loaded(Arc::clone(&catalog)),
        );
        Ok(catalog)
    }

    pub(crate) fn cached_string_table_count(&self) -> usize {
        let cache = self
            .string_tables
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache
            .values()
            .filter(|entry| matches!(entry, CachedStringTable::Loaded(_)))
            .count()
    }

    pub(crate) fn cached_localization_count(&self) -> usize {
        let cache = self
            .localizations
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache
            .values()
            .filter(|entry| matches!(entry, CachedLocalization::Loaded(_)))
            .count()
    }

    fn load_table_cached(&self, table_id: &str) -> CachedStringTable {
        let Some(json_path) = ue_object_path_to_export_json(table_id, &self.export_root) else {
            return CachedStringTable::Failed(format!(
                "无法把字符串表路径映射到导出目录：{table_id}"
            ));
        };
        {
            let cache = self
                .string_tables
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cached) = cache.get(&json_path) {
                return cached.clone();
            }
        }
        let result = match load_string_table(&json_path) {
            Ok(table) => CachedStringTable::Loaded(Arc::new(table)),
            Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
                CachedStringTable::Failed(format!("找不到字符串表文件：{table_id}"))
            }
            Err(error) => {
                CachedStringTable::Failed(format!("加载字符串表 {table_id} 失败：{error}"))
            }
        };
        let mut cache = self
            .string_tables
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(json_path, result.clone());
        result
    }

    fn load_catalog_cached(&self, language: &str) -> CachedLocalization {
        {
            let cache = self
                .localizations
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cached) = cache.get(language) {
                return cached.clone();
            }
        }
        let result = if !is_safe_language_tag(language) {
            CachedLocalization::Error(format!("语言代码不合法：{language}"))
        } else {
            let json_path = localization_file(&self.export_root, language);
            match load_localization(&json_path) {
                Ok(catalog) => CachedLocalization::Loaded(Arc::new(catalog)),
                Err(DataError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
                    CachedLocalization::NotFound(format!(
                        "缺少语言 {language} 的本地化表（可运行 ../final-fantasy-resonance-export/scripts/run.sh locres --lang {language} 导出到 Localization/{language}.json）"
                    ))
                }
                Err(error) => CachedLocalization::Error(format!(
                    "加载语言 {language} 的本地化表失败：{error}"
                )),
            }
        };
        let mut cache = self
            .localizations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(language.to_owned(), result.clone());
        result
    }

    fn resolve_string_table_text(
        &self,
        table_id: &str,
        key: &str,
        language: &str,
        fallback_to_native: bool,
    ) -> ResolvedText {
        let table = match self.load_table_cached(table_id) {
            CachedStringTable::Loaded(table) => table,
            CachedStringTable::Failed(reason) => return missing(language, reason),
        };

        let ns = &table.table_namespace;
        let loc = self.load_catalog_cached(language);
        match &loc {
            CachedLocalization::Error(reason) => return missing(language, reason.clone()),
            CachedLocalization::Loaded(catalog) => {
                if let Some(translated) = catalog.get(ns, key) {
                    return ResolvedText {
                        text: Some(translated.to_owned()),
                        lang: Some(language.to_owned()),
                        requested_language: language.to_owned(),
                        reason: None,
                        source: TextSource::Localization,
                    };
                }
            }
            CachedLocalization::NotFound(_) => {}
        }

        let missing_reason = match loc {
            CachedLocalization::NotFound(reason) => reason,
            _ => format!("语言 {language} 的本地化表中缺少 {ns}/{key}"),
        };

        if language == self.native_language {
            let Some(found) = table.get(key) else {
                return missing(
                    language,
                    format!("字符串表 {ns}（{table_id}）中不存在键 {key}"),
                );
            };
            return ResolvedText {
                text: Some(found.to_owned()),
                lang: Some(self.native_language.clone()),
                requested_language: language.to_owned(),
                reason: None,
                source: TextSource::StringTable,
            };
        }

        if !fallback_to_native {
            return missing(language, missing_reason);
        }

        let Some(fallback_text) = table.get(key) else {
            return missing(
                language,
                format!("{missing_reason}；且源语言字符串表 {ns} 中不存在键 {key}"),
            );
        };
        ResolvedText {
            text: Some(fallback_text.to_owned()),
            lang: Some(self.native_language.clone()),
            requested_language: language.to_owned(),
            reason: Some(format!(
                "{missing_reason}，已回落到源语言 {}",
                self.native_language
            )),
            source: TextSource::StringTable,
        }
    }

    fn resolve_inline_text(
        &self,
        namespace: &str,
        key: &str,
        source_string: Option<&str>,
        language: &str,
        fallback_to_native: bool,
    ) -> ResolvedText {
        let ns_label = if namespace.is_empty() {
            "(空命名空间)"
        } else {
            namespace
        };
        let loc = self.load_catalog_cached(language);
        match &loc {
            CachedLocalization::Error(reason) => return missing(language, reason.clone()),
            CachedLocalization::Loaded(catalog) => {
                if let Some(translated) = catalog.get(namespace, key) {
                    return ResolvedText {
                        text: Some(translated.to_owned()),
                        lang: Some(language.to_owned()),
                        requested_language: language.to_owned(),
                        reason: None,
                        source: TextSource::Localization,
                    };
                }
            }
            CachedLocalization::NotFound(_) => {}
        }

        let missing_reason = match loc {
            CachedLocalization::NotFound(reason) => reason,
            _ => format!("语言 {language} 的本地化表中缺少内联文本 {ns_label}/{key}"),
        };

        if language == self.native_language {
            let Some(source_text) = source_string else {
                return missing(
                    language,
                    format!("内联 FText（{ns_label}/{key}）缺少 SourceString"),
                );
            };
            return ResolvedText {
                text: Some(source_text.to_owned()),
                lang: Some(self.native_language.clone()),
                requested_language: language.to_owned(),
                reason: None,
                source: TextSource::InlineSource,
            };
        }

        if !fallback_to_native {
            return missing(language, missing_reason);
        }

        let Some(source_text) = source_string else {
            return missing(
                language,
                format!("{missing_reason}；且内联 FText 缺少 SourceString"),
            );
        };
        ResolvedText {
            text: Some(source_text.to_owned()),
            lang: Some(self.native_language.clone()),
            requested_language: language.to_owned(),
            reason: Some(format!(
                "{missing_reason}，已回落到源语言 {}",
                self.native_language
            )),
            source: TextSource::InlineSource,
        }
    }
}

/// Narrow a raw CUE4Parse `FText` JSON value into [`FTextValue`].
pub fn parse_ftext(value: &Value) -> Option<FTextValue> {
    let obj = value.as_object()?;
    if let (Some(table_id), Some(key)) = (
        obj.get("TableId").and_then(Value::as_str),
        obj.get("Key").and_then(Value::as_str),
    ) {
        return Some(FTextValue::StringTable {
            table_id: table_id.to_owned(),
            key: key.to_owned(),
            source_string: optional_string(obj.get("SourceString")),
            localized_string: optional_string(obj.get("LocalizedString")),
        });
    }
    if let Some(raw) = obj.get("CultureInvariantString") {
        return match raw {
            Value::Null => Some(FTextValue::CultureInvariant { value: None }),
            Value::String(text) => Some(FTextValue::CultureInvariant {
                value: Some(text.clone()),
            }),
            _ => None,
        };
    }
    if let (Some(namespace), Some(key)) = (
        obj.get("Namespace").and_then(Value::as_str),
        obj.get("Key").and_then(Value::as_str),
    ) {
        return Some(FTextValue::Inline {
            namespace: namespace.to_owned(),
            key: key.to_owned(),
            source_string: optional_string(obj.get("SourceString")),
            localized_string: optional_string(obj.get("LocalizedString")),
        });
    }
    None
}

/// One-shot helper for resolving a single raw `FText` JSON value (defaults to `"zh-Hans"`).
/// Prefer [`TextResolver`] or [`crate::DataStore`] for batch lookups.
pub fn resolve_ftext(
    value: &Value,
    export_root: impl Into<PathBuf>,
    language: Option<&str>,
    options: ResolveOptions,
) -> ResolvedText {
    let resolver = TextResolver::new(export_root);
    resolver.resolve_value_with(value, language, options)
}

/// One-shot convenience helper for model-facing text: always resolves in `"zh-Hans"`.
pub fn resolve_model_text(
    value: &Value,
    export_root: impl Into<PathBuf>,
    options: ResolveOptions,
) -> ResolvedText {
    resolve_ftext(value, export_root, Some(MODEL_LANGUAGE), options)
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn missing(requested_language: &str, reason: impl Into<String>) -> ResolvedText {
    ResolvedText {
        text: None,
        lang: None,
        requested_language: requested_language.to_owned(),
        reason: Some(reason.into()),
        source: TextSource::Missing,
    }
}
