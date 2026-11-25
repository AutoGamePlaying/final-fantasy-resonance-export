//! Unreal Engine reflection metadata types and streaming parser for UE4SS `.jmap` dumps.
//!
//! Uses a custom `serde::de::DeserializeSeed` + `MapAccess` visitor over a buffered reader so
//! 100+ MB `.jmap` files are streamed object-by-object without ever materializing a full-file
//! `serde_json::Value` tree. Non-reflection top-level sections (such as `vtables`) and runtime
//! `Object` instances (`property_values`) are skipped directly in the stream via `IgnoredAny`.

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{DataError, Result};

/// Buffered I/O capacity for streaming 100+ MB `.jmap` files.
const STREAM_BUF_CAPACITY: usize = 256 * 1024;

/// Unreal Engine version tuple stored in `.jmap` `metadata.engine_version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JmapEngineVersion {
    pub major: u32,
    pub minor: u32,
}

/// Top-level `metadata` block from a UE4SS `.jmap` dump.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JmapMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<JmapEngineVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_change_list: Option<String>,
}

/// Kind of reflected Unreal type stored in `ue_class`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UeClassKind {
    #[serde(rename = "class")]
    Class,
    #[serde(rename = "struct")]
    ScriptStruct,
    #[serde(rename = "enum")]
    Enum,
}

impl UeClassKind {
    /// Canonical lowercase string stored in `ue_class.kind`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::ScriptStruct => "struct",
            Self::Enum => "enum",
        }
    }

    /// Parse a stored `ue_class.kind` string.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "class" => Some(Self::Class),
            "struct" => Some(Self::ScriptStruct),
            "enum" => Some(Self::Enum),
            _ => None,
        }
    }
}

/// Interface implementation entry declared on a `UClass`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UeInterfaceImpl {
    pub class: String,
    #[serde(default)]
    pub pointer_offset: u32,
    #[serde(default)]
    pub implemented_by_k2: bool,
}

/// One enumerator entry in a UE enum (`ue_enum_value`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UeEnumValue {
    /// Full enumerator name as dumped by UE4SS (e.g. `"eElement::Ice"` or `"ACLCL_Low"`).
    pub name: String,
    /// Unqualified short enumerator name without any `"EnumType::"` prefix (e.g. `"Ice"`).
    pub short_name: String,
    /// Signed 64-bit integer value of the enumerator.
    pub value: i64,
}

impl UeEnumValue {
    /// Construct a `UeEnumValue`, deriving `short_name` from any `"Enum::Item"` prefix.
    pub fn new(name: String, value: i64) -> Self {
        let short_name = name
            .rsplit_once("::")
            .map(|(_, short)| short.to_owned())
            .unwrap_or_else(|| name.clone());
        Self {
            name,
            short_name,
            value,
        }
    }
}

/// Reflected Unreal property (`FProperty`) on a class, struct, or function parameter list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UeProperty {
    pub name: String,
    #[serde(rename = "type")]
    pub prop_type: String,
    pub offset: u32,
    pub size: u32,
    #[serde(
        default = "default_array_dim",
        skip_serializing_if = "is_default_array_dim"
    )]
    pub array_dim: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flags: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub struct_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_function: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner: Option<Box<UeProperty>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_prop: Option<Box<UeProperty>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_prop: Option<Box<UeProperty>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Box<UeProperty>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_size: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_mask: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_mask: Option<u8>,
}

impl UeProperty {
    /// Returns `true` if `flags` contains `flag` as an exact `|`-separated token.
    pub fn has_flag(&self, flag: &str) -> bool {
        has_pipe_flag(&self.flags, flag)
    }

    /// Returns `true` when this property is a function parameter (`CPF_Parm`).
    pub fn is_param(&self) -> bool {
        self.has_flag("CPF_Parm")
    }

    /// Returns `true` when this property is a function return value (`CPF_ReturnParm`).
    pub fn is_return_param(&self) -> bool {
        self.has_flag("CPF_ReturnParm")
    }

    /// Returns `true` when this property is an output/reference parameter (`CPF_OutParm`).
    pub fn is_out_param(&self) -> bool {
        self.has_flag("CPF_OutParm")
    }

    /// Returns `true` when this property is marked `CPF_ConstParm`.
    pub fn is_const_param(&self) -> bool {
        self.has_flag("CPF_ConstParm")
    }

    /// Returns `true` when this property is marked `CPF_ReferenceParm`.
    pub fn is_reference_param(&self) -> bool {
        self.has_flag("CPF_ReferenceParm")
    }

    /// Concise summary of the inner or target type stored in `ue_property.inner`.
    ///
    /// Returns `None` for plain scalar properties (e.g. `IntProperty`, `FloatProperty`,
    /// `BoolProperty`, `StrProperty`).
    pub fn inner_summary(&self) -> Option<String> {
        if let Some(struct_path) = &self.struct_path {
            return Some(struct_path.clone());
        }
        if let Some(enum_path) = &self.enum_path {
            return Some(enum_path.clone());
        }
        if let (Some(key_prop), Some(value_prop)) = (&self.key_prop, &self.value_prop) {
            return Some(format!(
                "{} => {}",
                key_prop.type_display(),
                value_prop.type_display()
            ));
        }
        if let Some(key_prop) = &self.key_prop {
            return Some(key_prop.type_display());
        }
        if let Some(inner) = &self.inner {
            return Some(inner.type_display());
        }
        if let Some(meta_class) = &self.meta_class {
            return Some(meta_class.clone());
        }
        if let Some(property_class) = &self.property_class {
            return Some(property_class.clone());
        }
        if let Some(interface_class) = &self.interface_class {
            return Some(interface_class.clone());
        }
        if let Some(signature_function) = &self.signature_function {
            return Some(signature_function.clone());
        }
        None
    }

    /// Human-readable representation of this property's type, including nested container types.
    pub fn type_display(&self) -> String {
        if let Some(struct_path) = &self.struct_path {
            return struct_path.clone();
        }
        if let Some(enum_path) = &self.enum_path {
            return enum_path.clone();
        }
        if let (Some(key_prop), Some(value_prop)) = (&self.key_prop, &self.value_prop) {
            return format!(
                "{}<{}, {}>",
                self.prop_type,
                key_prop.type_display(),
                value_prop.type_display()
            );
        }
        if let Some(key_prop) = &self.key_prop {
            return format!("{}<{}>", self.prop_type, key_prop.type_display());
        }
        if let Some(inner) = &self.inner {
            return format!("{}<{}>", self.prop_type, inner.type_display());
        }
        if let Some(meta_class) = &self.meta_class {
            return format!("{}<{}>", self.prop_type, meta_class);
        }
        if let Some(property_class) = &self.property_class {
            return format!("{}<{}>", self.prop_type, property_class);
        }
        if let Some(interface_class) = &self.interface_class {
            return format!("{}<{}>", self.prop_type, interface_class);
        }
        if let Some(signature_function) = &self.signature_function {
            return format!("{}<{}>", self.prop_type, signature_function);
        }
        self.prop_type.clone()
    }

    /// Collect all `ue_class` paths referenced by this property and any nested sub-properties.
    pub fn collect_referenced_classes<'a>(&'a self, out: &mut Vec<(&'static str, &'a str)>) {
        if let Some(p) = self.struct_path.as_deref() {
            out.push(("struct", p));
        }
        if let Some(p) = self.enum_path.as_deref() {
            out.push(("enum", p));
        }
        if let Some(p) = self.property_class.as_deref() {
            out.push(("property_class", p));
        }
        if let Some(p) = self.meta_class.as_deref() {
            out.push(("meta_class", p));
        }
        if let Some(p) = self.interface_class.as_deref() {
            out.push(("interface_class", p));
        }
        if let Some(inner) = &self.inner {
            inner.collect_referenced_classes(out);
        }
        if let Some(key_prop) = &self.key_prop {
            key_prop.collect_referenced_classes(out);
        }
        if let Some(value_prop) = &self.value_prop {
            value_prop.collect_referenced_classes(out);
        }
        if let Some(container) = &self.container {
            container.collect_referenced_classes(out);
        }
    }

    /// Collect all `ue_function` signature paths referenced by this property and nested sub-properties.
    pub fn collect_referenced_functions<'a>(&'a self, out: &mut Vec<&'a str>) {
        if let Some(sig) = self.signature_function.as_deref() {
            out.push(sig);
        }
        if let Some(inner) = &self.inner {
            inner.collect_referenced_functions(out);
        }
        if let Some(key_prop) = &self.key_prop {
            key_prop.collect_referenced_functions(out);
        }
        if let Some(value_prop) = &self.value_prop {
            value_prop.collect_referenced_functions(out);
        }
        if let Some(container) = &self.container {
            container.collect_referenced_functions(out);
        }
    }
}

/// Reflected Unreal function (`UFunction` / `UDelegateFunction`) with parameters and return value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UeFunction {
    pub path: String,
    pub owner: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub super_function: Option<String>,
    #[serde(default)]
    pub properties_size: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flags: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<UeProperty>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_value: Option<UeProperty>,
}

impl UeFunction {
    /// Returns `true` if `flags` contains `flag` as an exact `|`-separated token.
    pub fn has_flag(&self, flag: &str) -> bool {
        has_pipe_flag(&self.flags, flag)
    }
}

/// Reflected Unreal class, struct, or enum (`ue_class`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UeClass {
    pub path: String,
    pub short_name: String,
    pub kind: UeClassKind,
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub super_class: Option<String>,
    #[serde(default)]
    pub size: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_alignment: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flags: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object_flags: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<UeInterfaceImpl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpp_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpp_form: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<UeProperty>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<UeEnumValue>,
}

/// One typed entry yielded while streaming a `.jmap` file's `objects` map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JmapEntry {
    /// A `Class`, `ScriptStruct`, or `Enum` reflection definition.
    Class(UeClass),
    /// A `Function` (including delegate signatures) reflection definition.
    Function(UeFunction),
    /// A `Package` path (retained so package-owned delegate functions do not look dangling).
    Package(String),
    /// A non-reflection runtime object (e.g. `Object` CDO/subobject) skipped during streaming.
    Ignored,
}

/// Summary counters returned after streaming a single `.jmap` file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JmapStreamSummary {
    pub metadata: Option<JmapMetadata>,
    pub total_objects: u64,
    pub classes: u64,
    pub structs: u64,
    pub enums: u64,
    pub functions: u64,
    pub packages: u64,
    pub ignored: u64,
}

/// In-memory collection of parsed `.jmap` entries (convenience wrapper over [`stream_jmap_reader`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JmapDump {
    pub summary: JmapStreamSummary,
    pub classes: Vec<UeClass>,
    pub functions: Vec<UeFunction>,
    pub packages: Vec<String>,
}

/// Stream a `.jmap` JSON document from any [`Read`] source, invoking `on_entry` for each object
/// without loading the whole file into a `serde_json::Value` tree.
pub fn stream_jmap_reader<R, F>(
    reader: R,
    label: &str,
    mut on_entry: F,
) -> Result<JmapStreamSummary>
where
    R: Read,
    F: FnMut(JmapEntry),
{
    let mut de = serde_json::Deserializer::from_reader(reader);
    let mut summary = JmapStreamSummary::default();
    let mut saw_objects = false;

    let seed = JmapDocumentSeed {
        summary: &mut summary,
        saw_objects: &mut saw_objects,
        on_entry: &mut on_entry,
    };

    seed.deserialize(&mut de)
        .map_err(|source| DataError::Json {
            kind: "反射转储文件",
            path: PathBuf::from(label),
            source,
        })?;

    de.end().map_err(|source| DataError::Json {
        kind: "反射转储文件",
        path: PathBuf::from(label),
        source,
    })?;

    if !saw_objects {
        return Err(DataError::invalid_schema(
            label,
            "objects",
            "缺少顶层 objects 字段",
        ));
    }

    Ok(summary)
}

/// Stream a `.jmap` file from disk using a 256 KiB buffered reader.
pub fn stream_jmap_file<F>(path: impl AsRef<Path>, on_entry: F) -> Result<JmapStreamSummary>
where
    F: FnMut(JmapEntry),
{
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| DataError::Io {
        kind: "反射转储文件",
        path: path.to_path_buf(),
        source,
    })?;
    let reader = BufReader::with_capacity(STREAM_BUF_CAPACITY, file);
    let label = path.display().to_string();
    stream_jmap_reader(reader, &label, on_entry)
}

/// Parse a `.jmap` JSON document from a reader into a [`JmapDump`] via [`stream_jmap_reader`].
pub fn parse_jmap_reader<R: Read>(reader: R, label: &str) -> Result<JmapDump> {
    let mut dump = JmapDump::default();
    let summary = stream_jmap_reader(reader, label, |entry| match entry {
        JmapEntry::Class(cls) => dump.classes.push(cls),
        JmapEntry::Function(func) => dump.functions.push(func),
        JmapEntry::Package(pkg) => dump.packages.push(pkg),
        JmapEntry::Ignored => {}
    })?;
    dump.summary = summary;
    Ok(dump)
}

/// Load a `.jmap` file from disk into a [`JmapDump`] via [`stream_jmap_file`].
pub fn load_jmap_file(path: impl AsRef<Path>) -> Result<JmapDump> {
    let mut dump = JmapDump::default();
    let summary = stream_jmap_file(path, |entry| match entry {
        JmapEntry::Class(cls) => dump.classes.push(cls),
        JmapEntry::Function(func) => dump.functions.push(func),
        JmapEntry::Package(pkg) => dump.packages.push(pkg),
        JmapEntry::Ignored => {}
    })?;
    dump.summary = summary;
    Ok(dump)
}

fn default_array_dim() -> u32 {
    1
}

fn is_default_array_dim(dim: &u32) -> bool {
    *dim == 1
}

fn has_pipe_flag(flags: &str, needle: &str) -> bool {
    flags.split('|').any(|part| part.trim() == needle)
}

fn split_class_path(path: &str, outer: Option<String>) -> (String, String) {
    let (derived_pkg, short_name) = match path.rsplit_once('.') {
        Some((pkg, short)) => (pkg.to_owned(), short.to_owned()),
        None => (String::new(), path.to_owned()),
    };
    let package = match outer {
        Some(pkg) if !pkg.is_empty() => pkg,
        _ => derived_pkg,
    };
    (package, short_name)
}

fn split_function_path(path: &str, outer: Option<String>) -> (String, String) {
    let (derived_owner, name) = match path.rsplit_once(':') {
        Some((owner, name)) => (owner.to_owned(), name.to_owned()),
        None => match path.rsplit_once('.') {
            Some((owner, name)) => (owner.to_owned(), name.to_owned()),
            None => (String::new(), path.to_owned()),
        },
    };
    let owner = match outer {
        Some(owner) if !owner.is_empty() => owner,
        _ => derived_owner,
    };
    (owner, name)
}

// ---------------------------------------------------------------------------
// Streaming serde DeserializeSeed / Visitor implementation
// ---------------------------------------------------------------------------

struct JmapDocumentSeed<'a, F> {
    summary: &'a mut JmapStreamSummary,
    saw_objects: &'a mut bool,
    on_entry: &'a mut F,
}

impl<'de, F> DeserializeSeed<'de> for JmapDocumentSeed<'_, F>
where
    F: FnMut(JmapEntry),
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(self)
    }
}

impl<'de, F> Visitor<'de> for JmapDocumentSeed<'_, F>
where
    F: FnMut(JmapEntry),
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a top-level .jmap JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "metadata" => {
                    let meta: Option<JmapMetadata> = map.next_value()?;
                    self.summary.metadata = meta;
                }
                "objects" => {
                    *self.saw_objects = true;
                    map.next_value_seed(JmapObjectsSeed {
                        summary: self.summary,
                        on_entry: self.on_entry,
                    })?;
                }
                _ => {
                    let _: IgnoredAny = map.next_value()?;
                }
            }
        }
        Ok(())
    }
}

struct JmapObjectsSeed<'a, F> {
    summary: &'a mut JmapStreamSummary,
    on_entry: &'a mut F,
}

impl<'de, F> DeserializeSeed<'de> for JmapObjectsSeed<'_, F>
where
    F: FnMut(JmapEntry),
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(self)
    }
}

impl<'de, F> Visitor<'de> for JmapObjectsSeed<'_, F>
where
    F: FnMut(JmapEntry),
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a map of UE object paths to reflection objects")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        while let Some(path) = map.next_key::<String>()? {
            let raw: RawJmapObject = map.next_value()?;
            self.summary.total_objects += 1;
            let entry = raw.into_entry(path, self.summary);
            (self.on_entry)(entry);
        }
        Ok(())
    }
}

/// Raw property deserialized directly from `.jmap` (ignores volatile `address`).
#[derive(Debug, Deserialize)]
struct RawJmapProperty {
    name: String,
    #[serde(rename = "type")]
    prop_type: String,
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    size: u32,
    #[serde(default = "default_array_dim")]
    array_dim: u32,
    #[serde(default)]
    flags: String,
    #[serde(rename = "struct", default)]
    struct_path: Option<String>,
    #[serde(rename = "enum", default)]
    enum_path: Option<String>,
    #[serde(default)]
    property_class: Option<String>,
    #[serde(default)]
    meta_class: Option<String>,
    #[serde(default)]
    interface_class: Option<String>,
    #[serde(default)]
    signature_function: Option<String>,
    #[serde(default)]
    inner: Option<Box<RawJmapProperty>>,
    #[serde(default)]
    key_prop: Option<Box<RawJmapProperty>>,
    #[serde(default)]
    value_prop: Option<Box<RawJmapProperty>>,
    #[serde(default)]
    container: Option<Box<RawJmapProperty>>,
    #[serde(default)]
    field_size: Option<u8>,
    #[serde(default)]
    byte_offset: Option<u8>,
    #[serde(default)]
    byte_mask: Option<u8>,
    #[serde(default)]
    field_mask: Option<u8>,
}

impl From<RawJmapProperty> for UeProperty {
    fn from(raw: RawJmapProperty) -> Self {
        Self {
            name: raw.name,
            prop_type: raw.prop_type,
            offset: raw.offset,
            size: raw.size,
            array_dim: raw.array_dim,
            flags: raw.flags,
            struct_path: raw.struct_path,
            enum_path: raw.enum_path,
            property_class: raw.property_class,
            meta_class: raw.meta_class,
            interface_class: raw.interface_class,
            signature_function: raw.signature_function,
            inner: raw.inner.map(|p| Box::new(UeProperty::from(*p))),
            key_prop: raw.key_prop.map(|p| Box::new(UeProperty::from(*p))),
            value_prop: raw.value_prop.map(|p| Box::new(UeProperty::from(*p))),
            container: raw.container.map(|p| Box::new(UeProperty::from(*p))),
            field_size: raw.field_size,
            byte_offset: raw.byte_offset,
            byte_mask: raw.byte_mask,
            field_mask: raw.field_mask,
        }
    }
}

/// Raw object deserialized directly from `.jmap` `objects[path]`.
///
/// Unknown fields (`address`, `vtable`, `instance_vtable`, `func`, `script`, `property_values`,
/// `children`, `class_default_object`, `class_cast_flags`, etc.) are automatically skipped by
/// serde via `IgnoredAny` with zero heap allocation.
#[derive(Debug, Deserialize)]
struct RawJmapObject {
    #[serde(rename = "type", default)]
    obj_type: String,
    #[serde(default)]
    outer: Option<String>,
    #[serde(rename = "class", default)]
    class_type: Option<String>,
    #[serde(default)]
    super_struct: Option<String>,
    #[serde(default)]
    properties_size: u32,
    #[serde(default)]
    min_alignment: Option<u32>,
    #[serde(default)]
    class_flags: Option<String>,
    #[serde(default)]
    struct_flags: Option<String>,
    #[serde(default)]
    enum_flags: Option<String>,
    #[serde(default)]
    function_flags: Option<String>,
    #[serde(default)]
    object_flags: Option<String>,
    #[serde(default)]
    interfaces: Option<Vec<UeInterfaceImpl>>,
    #[serde(default)]
    properties: Option<Vec<RawJmapProperty>>,
    #[serde(default)]
    cpp_type: Option<String>,
    #[serde(default)]
    cpp_form: Option<String>,
    #[serde(default, deserialize_with = "deserialize_enum_names")]
    names: Option<Vec<(String, i64)>>,
}

impl RawJmapObject {
    fn into_entry(self, path: String, summary: &mut JmapStreamSummary) -> JmapEntry {
        match self.obj_type.as_str() {
            "Class" => {
                summary.classes += 1;
                let (package, short_name) = split_class_path(&path, self.outer);
                let properties = self
                    .properties
                    .unwrap_or_default()
                    .into_iter()
                    .map(UeProperty::from)
                    .collect();
                JmapEntry::Class(UeClass {
                    path,
                    short_name,
                    kind: UeClassKind::Class,
                    package,
                    class_type: self.class_type,
                    super_class: self.super_struct,
                    size: self.properties_size,
                    min_alignment: self.min_alignment,
                    flags: self.class_flags.unwrap_or_default(),
                    object_flags: self.object_flags.unwrap_or_default(),
                    interfaces: self.interfaces.unwrap_or_default(),
                    cpp_type: None,
                    cpp_form: None,
                    properties,
                    enum_values: Vec::new(),
                })
            }
            "ScriptStruct" => {
                summary.structs += 1;
                let (package, short_name) = split_class_path(&path, self.outer);
                let properties = self
                    .properties
                    .unwrap_or_default()
                    .into_iter()
                    .map(UeProperty::from)
                    .collect();
                JmapEntry::Class(UeClass {
                    path,
                    short_name,
                    kind: UeClassKind::ScriptStruct,
                    package,
                    class_type: self.class_type,
                    super_class: self.super_struct,
                    size: self.properties_size,
                    min_alignment: self.min_alignment,
                    flags: self.struct_flags.unwrap_or_default(),
                    object_flags: self.object_flags.unwrap_or_default(),
                    interfaces: Vec::new(),
                    cpp_type: None,
                    cpp_form: None,
                    properties,
                    enum_values: Vec::new(),
                })
            }
            "Enum" => {
                summary.enums += 1;
                let (package, short_name) = split_class_path(&path, self.outer);
                let enum_values = self
                    .names
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, value)| UeEnumValue::new(name, value))
                    .collect();
                JmapEntry::Class(UeClass {
                    path,
                    short_name,
                    kind: UeClassKind::Enum,
                    package,
                    class_type: self.class_type,
                    super_class: None,
                    size: 0,
                    min_alignment: None,
                    flags: self.enum_flags.unwrap_or_default(),
                    object_flags: self.object_flags.unwrap_or_default(),
                    interfaces: Vec::new(),
                    cpp_type: self.cpp_type,
                    cpp_form: self.cpp_form,
                    properties: Vec::new(),
                    enum_values,
                })
            }
            "Function" => {
                summary.functions += 1;
                let (owner, name) = split_function_path(&path, self.outer);
                let mut params = Vec::new();
                let mut return_value = None;
                for raw_prop in self.properties.unwrap_or_default() {
                    let prop = UeProperty::from(raw_prop);
                    // Exclude Blueprint local variables; retain only actual signature parameters
                    // (`CPF_Parm`) and the return value (`CPF_ReturnParm`).
                    if !prop.is_param() && !prop.is_return_param() {
                        continue;
                    }
                    if prop.is_return_param() && return_value.is_none() {
                        return_value = Some(prop);
                    } else {
                        params.push(prop);
                    }
                }
                JmapEntry::Function(UeFunction {
                    path,
                    owner,
                    name,
                    class_type: self.class_type,
                    super_function: self.super_struct,
                    properties_size: self.properties_size,
                    flags: self.function_flags.unwrap_or_default(),
                    params,
                    return_value,
                })
            }
            "Package" => {
                summary.packages += 1;
                JmapEntry::Package(path)
            }
            _ => {
                summary.ignored += 1;
                JmapEntry::Ignored
            }
        }
    }
}

/// Deserialize `.jmap` enum `names: [[name, value], ...]`, accepting signed `i64` as well as
/// unsigned 64-bit bitmask values (`u64` cast to `i64`).
fn deserialize_enum_names<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<(String, i64)>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptEnumNamesVisitor;

    impl<'de> Visitor<'de> for OptEnumNamesVisitor {
        type Value = Option<Vec<(String, i64)>>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("null or an array of [name, integer] enum pairs")
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_seq(EnumNamesSeqVisitor).map(Some)
        }

        fn visit_seq<A>(self, seq: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            EnumNamesSeqVisitor.visit_seq(seq).map(Some)
        }
    }

    struct EnumNamesSeqVisitor;

    impl<'de> Visitor<'de> for EnumNamesSeqVisitor {
        type Value = Vec<(String, i64)>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a sequence of [name, integer] enum entries")
        }

        fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(pair) = seq.next_element::<EnumPair>()? {
                out.push((pair.0, pair.1));
            }
            Ok(out)
        }
    }

    struct EnumPair(String, i64);

    impl<'de> Deserialize<'de> for EnumPair {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct PairVisitor;

            impl<'de> Visitor<'de> for PairVisitor {
                type Value = EnumPair;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a 2-element [name, value] enum tuple")
                }

                fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
                where
                    A: SeqAccess<'de>,
                {
                    let name: String = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                    let value: FlexI64 = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                    Ok(EnumPair(name, value.0))
                }
            }

            deserializer.deserialize_seq(PairVisitor)
        }
    }

    struct FlexI64(i64);

    impl<'de> Deserialize<'de> for FlexI64 {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct IntVisitor;

            impl Visitor<'_> for IntVisitor {
                type Value = FlexI64;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("an integer enum value")
                }

                fn visit_i64<E>(self, v: i64) -> std::result::Result<Self::Value, E> {
                    Ok(FlexI64(v))
                }

                fn visit_u64<E>(self, v: u64) -> std::result::Result<Self::Value, E> {
                    Ok(FlexI64(v as i64))
                }
            }

            deserializer.deserialize_any(IntVisitor)
        }
    }

    deserializer.deserialize_option(OptEnumNamesVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JMAP: &str = r#"{
      "metadata": {
        "tool": "UE4SS JMapGenerator",
        "timestamp": "2026-11-25T00:00:00Z",
        "source": "FFRS-Win64-Shipping.exe",
        "engine_version": { "major": 5, "minor": 6 },
        "build_change_list": "0"
      },
      "image_base_address": "0x140000000",
      "vtables": { "0x1": ["ignored"] },
      "objects": {
        "/Script/mucha": {
          "type": "Package",
          "address": "0x10",
          "outer": null,
          "class": "/Script/CoreUObject.Package",
          "property_values": {}
        },
        "/Script/mucha.eElement": {
          "type": "Enum",
          "address": "0x20",
          "outer": "/Script/mucha",
          "class": "/Script/CoreUObject.Enum",
          "cpp_type": "eElement",
          "enum_flags": "",
          "cpp_form": "EnumClass",
          "names": [
            ["eElement::None", 0],
            ["eElement::Fire", 1],
            ["eElement::Ice", 2]
          ]
        },
        "/Script/mucha.FTestData": {
          "type": "ScriptStruct",
          "address": "0x30",
          "outer": "/Script/mucha",
          "class": "/Script/CoreUObject.ScriptStruct",
          "super_struct": null,
          "properties_size": 24,
          "min_alignment": 8,
          "struct_flags": "STRUCT_Native",
          "properties": [
            {
              "address": "0x31",
              "name": "Element",
              "offset": 0,
              "array_dim": 1,
              "size": 1,
              "type": "EnumProperty",
              "enum": "/Script/mucha.eElement",
              "container": {
                "address": "0x32",
                "name": "UnderlyingType",
                "offset": 0,
                "array_dim": 1,
                "size": 1,
                "type": "ByteProperty",
                "enum": null,
                "flags": "CPF_ZeroConstructor"
              },
              "flags": "CPF_Edit"
            },
            {
              "address": "0x33",
              "name": "Tags",
              "offset": 8,
              "array_dim": 1,
              "size": 16,
              "type": "ArrayProperty",
              "inner": {
                "address": "0x34",
                "name": "Tags",
                "offset": 0,
                "array_dim": 1,
                "size": 4,
                "type": "IntProperty",
                "flags": ""
              },
              "flags": "CPF_Edit"
            }
          ]
        },
        "/Script/mucha.CPP_Test": {
          "type": "Class",
          "address": "0x40",
          "outer": "/Script/mucha",
          "class": "/Script/CoreUObject.Class",
          "super_struct": "/Script/CoreUObject.Object",
          "properties_size": 64,
          "min_alignment": 8,
          "class_flags": "CLASS_Native",
          "object_flags": "RF_Public",
          "interfaces": [],
          "properties": [
            {
              "address": "0x41",
              "name": "bActive",
              "offset": 40,
              "array_dim": 1,
              "size": 1,
              "type": "BoolProperty",
              "field_size": 1,
              "byte_offset": 0,
              "byte_mask": 1,
              "field_mask": 255,
              "flags": "CPF_BlueprintReadOnly"
            }
          ]
        },
        "/Script/mucha.CPP_Test:Compute": {
          "type": "Function",
          "address": "0x50",
          "outer": "/Script/mucha.CPP_Test",
          "class": "/Script/CoreUObject.Function",
          "super_struct": null,
          "properties_size": 12,
          "function_flags": "FUNC_Native | FUNC_Public",
          "properties": [
            {
              "address": "0x51",
              "name": "Input",
              "offset": 0,
              "array_dim": 1,
              "size": 4,
              "type": "IntProperty",
              "flags": "CPF_Parm"
            },
            {
              "address": "0x52",
              "name": "LocalTemp",
              "offset": 4,
              "array_dim": 1,
              "size": 4,
              "type": "IntProperty",
              "flags": "CPF_ZeroConstructor"
            },
            {
              "address": "0x53",
              "name": "ReturnValue",
              "offset": 8,
              "array_dim": 1,
              "size": 1,
              "type": "BoolProperty",
              "flags": "CPF_Parm | CPF_OutParm | CPF_ReturnParm"
            }
          ]
        },
        "/Script/mucha.Default__CPP_Test": {
          "type": "Object",
          "address": "0x60",
          "outer": "/Script/mucha",
          "class": "/Script/mucha.CPP_Test",
          "property_values": { "nested": [1, 2, 3], "deep": { "a": true } }
        }
      }
    }"#;

    #[test]
    fn streams_classes_structs_enums_and_functions_while_ignoring_runtime_objects() {
        let dump = parse_jmap_reader(SAMPLE_JMAP.as_bytes(), "sample.jmap")
            .expect("sample jmap should parse");

        assert_eq!(dump.summary.total_objects, 6);
        assert_eq!(dump.summary.packages, 1);
        assert_eq!(dump.summary.enums, 1);
        assert_eq!(dump.summary.structs, 1);
        assert_eq!(dump.summary.classes, 1);
        assert_eq!(dump.summary.functions, 1);
        assert_eq!(dump.summary.ignored, 1);
        assert_eq!(
            dump.summary
                .metadata
                .as_ref()
                .and_then(|m| m.engine_version),
            Some(JmapEngineVersion { major: 5, minor: 6 })
        );

        let enum_def = dump
            .classes
            .iter()
            .find(|c| c.short_name == "eElement")
            .expect("eElement present");
        assert_eq!(enum_def.kind, UeClassKind::Enum);
        assert_eq!(enum_def.enum_values.len(), 3);
        assert_eq!(enum_def.enum_values[2].name, "eElement::Ice");
        assert_eq!(enum_def.enum_values[2].short_name, "Ice");
        assert_eq!(enum_def.enum_values[2].value, 2);

        let struct_def = dump
            .classes
            .iter()
            .find(|c| c.short_name == "FTestData")
            .expect("FTestData present");
        assert_eq!(struct_def.kind, UeClassKind::ScriptStruct);
        assert_eq!(
            struct_def.properties[0].inner_summary().as_deref(),
            Some("/Script/mucha.eElement")
        );
        assert_eq!(
            struct_def.properties[1].inner_summary().as_deref(),
            Some("IntProperty")
        );

        let func = &dump.functions[0];
        assert_eq!(func.owner, "/Script/mucha.CPP_Test");
        assert_eq!(func.name, "Compute");
        assert_eq!(func.params.len(), 1);
        assert_eq!(func.params[0].name, "Input");
        assert_eq!(
            func.return_value.as_ref().map(|r| r.name.as_str()),
            Some("ReturnValue")
        );
    }

    #[test]
    fn rejects_jmap_missing_objects_section() {
        let err = parse_jmap_reader(br#"{"metadata":{}}"#.as_slice(), "broken.jmap")
            .expect_err("missing objects must fail");
        assert!(
            matches!(err, DataError::InvalidSchema { ref field, .. } if field == "objects"),
            "unexpected error: {err:?}"
        );
    }
}
