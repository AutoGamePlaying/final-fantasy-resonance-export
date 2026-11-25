//! Optional live-game R1 scene snapshot types (`.deps/r1/*`).
//!
//! One directory under the snapshot root (or under `<root>/hotkey/`) is one captured scene. The
//! directory name is the snapshot id; `meta.json`, `context.json`, and `done.json` describe when
//! and where the snapshot was taken. `objects.jsonl`, `widgets.jsonl`, and `texts.jsonl` hold the
//! captured runtime objects, one JSON object per line.
//!
//! These types are deliberately permissive: a partial or unfinished snapshot still yields the
//! files that could be read. [`Snapshot::issues`] carries the per-snapshot problems so the build
//! stage can record them in `build_issue`.

use serde::{Deserialize, Serialize};
use serde_json::Map;

/// One captured live-game scene snapshot directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Directory name (stable snapshot id, e.g. `20261124-162006-battle-command-snapshot`).
    pub id: String,
    /// Scene label parsed from the directory name or from `meta.json` / `done.json`.
    pub label: String,
    /// Capture time in Unix seconds (`context.json.time`, falling back to `startedAt`).
    pub time: Option<i64>,
    /// Job start time in Unix seconds (`meta.json.startedAt`).
    pub started_at: Option<i64>,
    /// Job finish time in Unix seconds (`meta.json.finishedAt`).
    pub finished_at: Option<i64>,
    /// Full Unreal world name from `context.json.world`.
    pub world: Option<String>,
    /// Whether `done.json` exists and does not report `ok: false`.
    pub completed: bool,
    /// Whether this snapshot came from the in-game hotkey directory.
    pub hotkey: bool,
    /// `ok`, `partial`, `incomplete`, or `failed`.
    pub status: String,
    /// Human-readable problems found while importing this snapshot.
    #[serde(default)]
    pub issues: Vec<String>,
}

/// One object line from `objects.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotObject {
    /// Full Unreal class name (e.g. `Class /Script/mucha.CPP_UI_BattleSubsystem`).
    pub class: String,
    /// Short class name after the final `.` / `/` (e.g. `CPP_UI_BattleSubsystem`).
    #[serde(default)]
    pub class_short: String,
    /// Full Unreal object name.
    pub name: String,
    /// Properties declared by the game class, exactly as dumped.
    #[serde(default)]
    pub properties: Map<String, serde_json::Value>,
}

/// One `UserWidget` line from `widgets.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotWidget {
    /// Full Unreal class name.
    pub class: String,
    /// Short class name after the final `.` / `/`.
    #[serde(default)]
    pub class_short: String,
    /// Full Unreal object name.
    pub name: String,
    /// `IsInViewport()` at capture time.
    #[serde(rename = "isInViewport", default)]
    pub is_in_viewport: bool,
    /// `Visibility` enum value (0..4), when the dump could read it.
    #[serde(default)]
    pub visibility: Option<i64>,
}

/// One `TextBlock` / `RichTextBlock` line from `texts.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotText {
    /// Full Unreal class name.
    pub class: String,
    /// Full Unreal object name (shows which widget tree it belongs to).
    pub name: String,
    /// Text value at capture time.
    #[serde(default)]
    pub text: String,
    /// `Visibility` enum value (0..4), when the dump could read it.
    #[serde(default)]
    pub visibility: Option<i64>,
}

/// One timestamp marker from `auto/markers.log` or `hotkey/markers.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// `auto` for the manual `markers.log`, `hotkey` for the F10 JSONL file.
    pub source: String,
    /// Marker sequence number from the hotkey protocol.
    pub seq: Option<u64>,
    /// Unix seconds (hotkey markers only).
    pub time: Option<i64>,
    /// Local wall-clock string (`YYYY-MM-DD HH:MM:SS` or `HH:MM:SS`).
    pub local_time: Option<String>,
    /// Full Unreal world name, when known.
    pub world: Option<String>,
    /// User-facing marker text.
    pub text: String,
    /// Milliseconds since the dump mod loaded (hotkey markers only).
    pub clock_ms: Option<i64>,
}

/// Return the short class name used as the indexed `class_short` column.
///
/// `Class /Script/mucha.CPP_UI_BattleSubsystem` -> `CPP_UI_BattleSubsystem`;
/// `BlueprintGeneratedClass /Game/UI/.../VM_BtlCmd.VM_BtlCmd_C` -> `VM_BtlCmd_C`.
pub fn class_short_name(class: &str) -> &str {
    class
        .rsplit(['.', '/', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(class)
}

/// Parse the scene label out of a snapshot directory name.
///
/// `20261124-162006-battle-command-snapshot` -> `battle-command`;
/// `hotkey-0003-20261125-023045` -> `hotkey-0003`. `meta.json` / `done.json` labels take
/// precedence at import time because they are authoritative for renamed or unusual directories.
pub fn label_from_snapshot_dir(dir_name: &str) -> String {
    let trimmed = dir_name.strip_suffix("-snapshot").unwrap_or(dir_name);
    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() >= 3
        && parts[0].len() == 8
        && parts[0].bytes().all(|b| b.is_ascii_digit())
        && parts[1].len() == 6
        && parts[1].bytes().all(|b| b.is_ascii_digit())
    {
        return parts[2..].join("-");
    }
    if dir_name.starts_with("hotkey-") && parts.len() >= 2 {
        return parts[..2].join("-");
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{class_short_name, label_from_snapshot_dir};

    #[test]
    fn class_short_names_drop_the_package_path() {
        assert_eq!(
            class_short_name("Class /Script/mucha.CPP_UI_BattleSubsystem"),
            "CPP_UI_BattleSubsystem"
        );
        assert_eq!(
            class_short_name("WidgetBlueprintGeneratedClass /Game/UI/WBP_Test.WBP_Test_C"),
            "WBP_Test_C"
        );
        assert_eq!(class_short_name("CPP_NoPackage"), "CPP_NoPackage");
    }

    #[test]
    fn labels_follow_the_snapshot_directory_convention() {
        assert_eq!(
            label_from_snapshot_dir("20261124-162006-battle-command-snapshot"),
            "battle-command"
        );
        assert_eq!(
            label_from_snapshot_dir("hotkey-0003-20261125-023045"),
            "hotkey-0003"
        );
        assert_eq!(label_from_snapshot_dir("custom-scene"), "custom-scene");
    }
}
