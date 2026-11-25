//! Optional live-game snapshot area (`10_snapshots`) `impl GameDb` queries.
//!
//! All queries use [`rusqlite::Connection::prepare_cached`] and hit the indexes created by
//! `schema/10_snapshots.idx.sql`.

use ffr_data::assets::snapshot::{Snapshot, SnapshotObject, SnapshotText};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record::query_docs;

/// One observed field value from [`GameDb::field_history`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldHistoryEntry {
    /// Snapshot directory name.
    pub snapshot_id: String,
    /// Scene label of that snapshot.
    pub label: String,
    /// Capture time in Unix seconds, when known.
    pub time: Option<i64>,
    /// Full object name inside the snapshot.
    pub object_name: String,
    /// Extracted property value; `None` means the field was absent (JSON `null` remains
    /// `Some(Value::Null)`).
    pub value: Option<Value>,
}

const SNAPSHOTS_BY_LABEL_SQL: &str =
    "SELECT id, doc FROM snapshot WHERE label = ?1 ORDER BY time, id";
const SNAPSHOTS_ALL_SQL: &str = "SELECT id, doc FROM snapshot ORDER BY time, id";
const SNAPSHOT_OBJECTS_BY_CLASS_SQL: &str = "SELECT id, doc FROM snapshot_object WHERE snapshot_id = ?1 AND class_short = ?2 \
     ORDER BY name, id";
const SNAPSHOT_OBJECTS_SQL: &str =
    "SELECT id, doc FROM snapshot_object WHERE snapshot_id = ?1 ORDER BY name, id";
const FIELD_HISTORY_SQL: &str = "SELECT s.id, s.label, s.time, o.name, \
        json_quote(json_extract(o.doc, ?2)), json_type(o.doc, ?2) \
     FROM snapshot_object o JOIN snapshot s ON s.id = o.snapshot_id \
     WHERE o.class_short = ?1 \
     ORDER BY s.time, s.id, o.name";
const SNAPSHOTS_WITH_CLASS_SQL: &str = "SELECT s.id, s.doc \
     FROM (SELECT DISTINCT snapshot_id FROM snapshot_object WHERE class_short = ?1) hits \
     JOIN snapshot s ON s.id = hits.snapshot_id \
     ORDER BY s.time, s.id";
const TEXTS_IN_SNAPSHOT_SQL: &str =
    "SELECT id, doc FROM snapshot_text WHERE snapshot_id = ?1 ORDER BY id";

impl GameDb {
    /// List snapshots, optionally filtered to one scene label (`battle-command`, `field-idle`, …).
    ///
    /// The label-filtered form uses `idx_snapshot_label`; without a filter the table is small
    /// enough to read in capture order.
    pub fn snapshots(&self, label: Option<&str>) -> Result<Vec<Snapshot>> {
        self.with_conn(|conn| match label {
            Some(label) => query_docs(conn, "snapshot", SNAPSHOTS_BY_LABEL_SQL, [label]),
            None => query_docs(conn, "snapshot", SNAPSHOTS_ALL_SQL, []),
        })
    }

    /// Read captured objects of one snapshot, optionally restricted to one short class name.
    pub fn snapshot_objects(
        &self,
        snapshot: &str,
        class_short: Option<&str>,
    ) -> Result<Vec<SnapshotObject>> {
        self.with_conn(|conn| match class_short {
            Some(class_short) => query_docs(
                conn,
                "snapshot_object",
                SNAPSHOT_OBJECTS_BY_CLASS_SQL,
                [snapshot, class_short],
            ),
            None => query_docs(conn, "snapshot_object", SNAPSHOT_OBJECTS_SQL, [snapshot]),
        })
    }

    /// List one field's value across every snapshot, ordered by capture time.
    ///
    /// `json_path` is relative to the object's `properties` map, e.g. `$.bDuringBattle` or
    /// `$.FocusCmdData.EntryIndex` (stored internally as `$.properties.<path>`). Missing fields
    /// yield `None`; JSON `null` yields `Some(Value::Null)`.
    pub fn field_history(
        &self,
        class_short: &str,
        json_path: &str,
    ) -> Result<Vec<FieldHistoryEntry>> {
        self.with_conn(|conn| {
            let path = object_property_path(json_path);
            let mut stmt = conn
                .prepare_cached(FIELD_HISTORY_SQL)
                .map_err(|err| DbError::sqlite("准备查询 snapshot 字段历史", err))?;
            let mut rows = stmt
                .query([class_short, path.as_str()])
                .map_err(|err| DbError::sqlite("查询 snapshot 字段历史", err))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 snapshot 字段历史结果", err))?
            {
                let snapshot_id: String = row
                    .get(0)
                    .map_err(|err| DbError::sqlite("读取字段历史 snapshot_id", err))?;
                let label: String = row
                    .get(1)
                    .map_err(|err| DbError::sqlite("读取字段历史 label", err))?;
                let time: Option<i64> = row
                    .get(2)
                    .map_err(|err| DbError::sqlite("读取字段历史 time", err))?;
                let object_name: String = row
                    .get(3)
                    .map_err(|err| DbError::sqlite("读取字段历史 object_name", err))?;
                let raw: Option<String> = row
                    .get(4)
                    .map_err(|err| DbError::sqlite("读取字段历史 JSON 值", err))?;
                let json_type: Option<String> = row
                    .get(5)
                    .map_err(|err| DbError::sqlite("读取字段历史 JSON 类型", err))?;
                let value = match json_type.as_deref() {
                    None => None,
                    Some("true") => Some(Value::Bool(true)),
                    Some("false") => Some(Value::Bool(false)),
                    _ => raw
                        .map(|json| serde_json::from_str::<Value>(&json))
                        .transpose()
                        .map_err(|source| DbError::DocDeserialize {
                            table: "snapshot_object".to_owned(),
                            key: format!("{snapshot_id}:{object_name}"),
                            source,
                        })?,
                };
                out.push(FieldHistoryEntry {
                    snapshot_id,
                    label,
                    time,
                    object_name,
                    value,
                });
            }
            Ok(out)
        })
    }

    /// List snapshots whose `objects.jsonl` contains at least one object of `class_short`.
    pub fn snapshots_with_class(&self, class_short: &str) -> Result<Vec<Snapshot>> {
        self.with_conn(|conn| query_docs(conn, "snapshot", SNAPSHOTS_WITH_CLASS_SQL, [class_short]))
    }

    /// Read all captured texts of one snapshot.
    pub fn texts_in_snapshot(&self, snapshot: &str) -> Result<Vec<SnapshotText>> {
        self.with_conn(|conn| query_docs(conn, "snapshot_text", TEXTS_IN_SNAPSHOT_SQL, [snapshot]))
    }
}

/// Map a user-facing property path (`$.bDuringBattle`) onto the stored object document path
/// (`$.properties.bDuringBattle`).
fn object_property_path(json_path: &str) -> String {
    let trimmed = json_path.trim();
    if trimmed.is_empty() || trimmed == "$" {
        return "$.properties".to_owned();
    }
    if let Some(rest) = trimmed.strip_prefix("$.") {
        if rest == "properties" || rest.starts_with("properties.") {
            return format!("$.{rest}");
        }
        return format!("$.properties.{rest}");
    }
    format!("$.properties.{}", trimmed.trim_start_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::object_property_path;

    #[test]
    fn property_paths_are_relative_to_the_object_properties_map() {
        assert_eq!(
            object_property_path("$.bDuringBattle"),
            "$.properties.bDuringBattle"
        );
        assert_eq!(
            object_property_path("$.FocusCmdData.EntryIndex"),
            "$.properties.FocusCmdData.EntryIndex"
        );
        assert_eq!(
            object_property_path("$.properties.bDuringBattle"),
            "$.properties.bDuringBattle"
        );
        assert_eq!(object_property_path("$"), "$.properties");
        assert_eq!(
            object_property_path("bDuringBattle"),
            "$.properties.bDuringBattle"
        );
    }
}
