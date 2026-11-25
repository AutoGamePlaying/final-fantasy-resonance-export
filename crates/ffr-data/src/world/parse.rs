//! Shared internal JSON parsing helpers for `world` loaders.

use serde_json::{Map, Value};

use crate::datatable::DataTable;
use crate::error::{DataError, Result};
use crate::ftext::{FTextValue, parse_ftext};
use crate::scenario::ScenarioDestination;

/// 3D world-space coordinate exported by Unreal Engine (`{ X, Y, Z }`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPoint3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub(crate) fn expect_row_struct(
    table: &DataTable,
    expected: &str,
    source_label: &str,
) -> Result<()> {
    if table.row_struct != expected {
        return Err(DataError::UnexpectedRowStruct {
            path: source_label.to_owned(),
            expected: expected.to_owned(),
            actual: table.row_struct.clone(),
        });
    }
    Ok(())
}

pub(crate) fn expect_row_object<'a>(
    raw_row: &'a Value,
    source_label: &str,
    row_prefix: &str,
) -> Result<&'a Map<String, Value>> {
    raw_row
        .as_object()
        .ok_or_else(|| DataError::invalid_schema(source_label, row_prefix, "行值不是对象"))
}

pub(crate) fn expect_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<&'a Map<String, Value>> {
    object.get(field).and_then(Value::as_object).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "缺少对象")
    })
}

pub(crate) fn expect_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<&'a Vec<Value>> {
    object.get(field).and_then(Value::as_array).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是数组")
    })
}

pub(crate) fn expect_i64(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<i64> {
    object.get(field).and_then(Value::as_i64).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是整数")
    })
}

pub(crate) fn expect_f64(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<f64> {
    object.get(field).and_then(Value::as_f64).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是数值")
    })
}

pub(crate) fn expect_string(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是字符串")
        })
}

pub(crate) fn expect_bool(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<bool> {
    object.get(field).and_then(Value::as_bool).ok_or_else(|| {
        DataError::invalid_schema(source_label, format!("{prefix}.{field}"), "不是布尔值")
    })
}

pub(crate) fn expect_ftext(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<FTextValue> {
    object.get(field).and_then(parse_ftext).ok_or_else(|| {
        DataError::invalid_schema(
            source_label,
            format!("{prefix}.{field}"),
            "不是合法的 FText",
        )
    })
}

pub(crate) fn non_negative_id(value: i64) -> Option<i64> {
    (value >= 0).then_some(value)
}

pub(crate) fn positive_id(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

pub(crate) fn parse_point3(
    object: &Map<String, Value>,
    field: &str,
    source_label: &str,
    prefix: &str,
) -> Result<WorldPoint3> {
    let vec_prefix = format!("{prefix}.{field}");
    let vec = expect_object(object, field, source_label, prefix)?;
    Ok(WorldPoint3 {
        x: expect_f64(vec, "X", source_label, &vec_prefix)?,
        y: expect_f64(vec, "Y", source_label, &vec_prefix)?,
        z: expect_f64(vec, "Z", source_label, &vec_prefix)?,
    })
}

pub(crate) fn parse_scenario_destination(
    value: &Value,
    source_label: &str,
    prefix: &str,
) -> Result<ScenarioDestination> {
    let obj = value
        .as_object()
        .ok_or_else(|| DataError::invalid_schema(source_label, prefix, "不是对象"))?;

    let dest_key_prefix = format!("{prefix}.DestinationKey");
    let dest_key = expect_object(obj, "DestinationKey", source_label, prefix)?;

    let object_prefix = format!("{prefix}.Object");
    let object_ref = expect_object(obj, "Object", source_label, prefix)?;

    Ok(ScenarioDestination {
        point_type: expect_string(obj, "PointType", source_label, prefix)?,
        destination_type: expect_string(dest_key, "Type", source_label, &dest_key_prefix)?,
        destination_id: expect_i64(dest_key, "DestinationId", source_label, &dest_key_prefix)?,
        is_point_to_object: expect_bool(obj, "IsPointToObject", source_label, prefix)?,
        object_type: expect_string(object_ref, "Type", source_label, &object_prefix)?,
        object_id: expect_i64(object_ref, "ID", source_label, &object_prefix)?,
        condition: expect_string(obj, "Condition", source_label, prefix)?,
    })
}

/// Extract the numeric threshold `N` from the first `{progress:0}>=<N>` clause inside a
/// potentially compound condition expression (for example `"{progress:6}==0&{progress:0}>=5310"`).
pub(crate) fn extract_main_progress_threshold(condition: &str) -> Option<i64> {
    let needle = "{progress:0}>=";
    let start = condition.find(needle)? + needle.len();
    let rest = &condition[start..];
    let digit_len = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digit_len == 0 {
        return None;
    }
    rest[..digit_len].parse::<i64>().ok()
}
