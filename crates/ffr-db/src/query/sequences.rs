//! Cutscene sequences area (`09_sequences`) read queries on [`GameDb`].
//!
//! Every query uses `prepare_cached` (via [`crate::record`]) and executes through an index on
//! `sequence`, `sequence_binding`, `sequence_track`, or `sequence_event`.

use ffr_data::assets::sequence::{
    Sequence, SequenceBinding, SequenceEvent, SequenceTrack, normalize_sequence_path,
};

use crate::db::GameDb;
use crate::error::Result;
use crate::record::{query_doc_opt, query_docs};

/// SQL for [`GameDb::sequence`].
pub const SQL_SEQUENCE_BY_PATH: &str = "SELECT doc FROM sequence WHERE path = ?1";

/// SQL for [`GameDb::sequences_in_dir`] (range scan over the UNIQUE `sequence(path)` index).
pub const SQL_SEQUENCES_IN_DIR: &str =
    "SELECT id, doc FROM sequence WHERE path >= ?1 AND path < ?2 ORDER BY path";

/// SQL for [`GameDb::sequence_bindings`].
pub const SQL_SEQUENCE_BINDINGS_BY_PATH: &str = "SELECT b.id, b.doc
     FROM sequence s
     JOIN sequence_binding b ON b.sequence_id = s.id
     WHERE s.path = ?1
     ORDER BY b.id";

/// SQL for [`GameDb::sequence_tracks`].
pub const SQL_SEQUENCE_TRACKS_BY_PATH: &str = "SELECT t.id, t.doc
     FROM sequence s
     JOIN sequence_track t ON t.sequence_id = s.id
     WHERE s.path = ?1
     ORDER BY t.id";

/// SQL for [`GameDb::sequence_events`].
pub const SQL_SEQUENCE_EVENTS_BY_PATH: &str = "SELECT e.id, e.doc
     FROM sequence s
     JOIN sequence_event e ON e.sequence_id = s.id
     WHERE s.path = ?1
     ORDER BY e.time_ticks, e.id";

/// SQL for [`GameDb::sequences_calling`].
pub const SQL_SEQUENCES_CALLING: &str = "SELECT DISTINCT s.id, s.doc
     FROM sequence_event e
     JOIN sequence s ON s.id = e.sequence_id
     WHERE e.function = ?1
     ORDER BY s.path";

/// SQL for [`GameDb::sequences_with_unit_id`].
pub const SQL_SEQUENCES_WITH_UNIT_ID: &str = "SELECT DISTINCT s.id, s.doc
     FROM sequence_binding b
     JOIN sequence s ON s.id = b.sequence_id
     WHERE b.map_unit_id = ?1
     ORDER BY s.path";

/// SQL for [`GameDb::sequences_with_unit_class`].
pub const SQL_SEQUENCES_WITH_UNIT_CLASS: &str = "SELECT DISTINCT s.id, s.doc
     FROM sequence_binding b
     JOIN sequence s ON s.id = b.sequence_id
     WHERE b.template_class = ?1
     ORDER BY s.path";

/// SQL for [`GameDb::sequences_by_event_id`].
pub const SQL_SEQUENCES_BY_EVENT_ID: &str = "SELECT DISTINCT s.id, s.doc
     FROM sequence_event e
     JOIN sequence s ON s.id = e.sequence_id
     WHERE e.talk_event_id = ?1
     ORDER BY s.path";

/// SQL for [`GameDb::sequences_by_flag`].
pub const SQL_SEQUENCES_BY_FLAG: &str = "SELECT DISTINCT s.id, s.doc
     FROM sequence_event e
     JOIN sequence s ON s.id = e.sequence_id
     WHERE e.flag_id = ?1
     ORDER BY s.path";

/// Filter for [`GameDb::sequences_with_unit`]: look up sequences by `m_MapUnitId` or actor class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceUnitFilter<'a> {
    /// Spawned map unit identifier (`m_MapUnitId`, e.g. `60`).
    MapUnitId(i64),
    /// Bound actor template or possessed class name (e.g. `"CPP_MapUnit"`, `"CineCameraActor"`).
    TemplateClass(&'a str),
}

impl GameDb {
    /// Fetch one `LevelSequence` descriptor by canonical or relative asset path.
    pub fn sequence(&self, path: &str) -> Result<Option<Sequence>> {
        let normalized = normalize_sequence_path(path);
        if normalized.is_empty() {
            return Ok(None);
        }
        self.with_conn(|conn| {
            query_doc_opt(
                conn,
                "sequence",
                &normalized,
                SQL_SEQUENCE_BY_PATH,
                [normalized.as_str()],
            )
        })
    }

    /// List all `LevelSequence` descriptors stored under directory `prefix` (recursive).
    ///
    /// Accepts canonical prefixes (`"/Game/Sequencer/Event/Main_Event/C01"`) as well as relative
    /// prefixes (`"Event/Main_Event/C01"`), with or without a trailing slash.
    pub fn sequences_in_dir(&self, prefix: &str) -> Result<Vec<Sequence>> {
        let Some((lower, upper)) = directory_prefix_bounds(prefix) else {
            return Ok(Vec::new());
        };
        self.with_conn(|conn| {
            query_docs(
                conn,
                "sequence",
                SQL_SEQUENCES_IN_DIR,
                [lower.as_str(), upper.as_str()],
            )
        })
    }

    /// Fetch all object bindings belonging to the `LevelSequence` at `path`.
    pub fn sequence_bindings(&self, path: &str) -> Result<Vec<SequenceBinding>> {
        let normalized = normalize_sequence_path(path);
        if normalized.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_docs(
                conn,
                "sequence_binding",
                SQL_SEQUENCE_BINDINGS_BY_PATH,
                [normalized.as_str()],
            )
        })
    }

    /// Fetch all tracks belonging to the `LevelSequence` at `path`.
    pub fn sequence_tracks(&self, path: &str) -> Result<Vec<SequenceTrack>> {
        let normalized = normalize_sequence_path(path);
        if normalized.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_docs(
                conn,
                "sequence_track",
                SQL_SEQUENCE_TRACKS_BY_PATH,
                [normalized.as_str()],
            )
        })
    }

    /// Fetch all event trigger keyframes belonging to the `LevelSequence` at `path`, ordered by time.
    pub fn sequence_events(&self, path: &str) -> Result<Vec<SequenceEvent>> {
        let normalized = normalize_sequence_path(path);
        if normalized.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_docs(
                conn,
                "sequence_event",
                SQL_SEQUENCE_EVENTS_BY_PATH,
                [normalized.as_str()],
            )
        })
    }

    /// Find all `LevelSequence` assets that invoke `function` in at least one event trigger.
    pub fn sequences_calling(&self, function: &str) -> Result<Vec<Sequence>> {
        if function.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| query_docs(conn, "sequence", SQL_SEQUENCES_CALLING, [function]))
    }

    /// Find all `LevelSequence` assets containing a binding matching `filter` (`m_MapUnitId` or
    /// template/actor class name).
    pub fn sequences_with_unit(&self, filter: SequenceUnitFilter<'_>) -> Result<Vec<Sequence>> {
        match filter {
            SequenceUnitFilter::MapUnitId(id) => self.sequences_with_unit_id(id),
            SequenceUnitFilter::TemplateClass(cls) => self.sequences_with_unit_class(cls),
        }
    }

    /// Find all `LevelSequence` assets that bind a map unit with the given `m_MapUnitId`.
    pub fn sequences_with_unit_id(&self, map_unit_id: i64) -> Result<Vec<Sequence>> {
        self.with_conn(|conn| {
            query_docs(conn, "sequence", SQL_SEQUENCES_WITH_UNIT_ID, [map_unit_id])
        })
    }

    /// Find all `LevelSequence` assets that bind an actor/component of `template_class`.
    pub fn sequences_with_unit_class(&self, template_class: &str) -> Result<Vec<Sequence>> {
        if template_class.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            query_docs(
                conn,
                "sequence",
                SQL_SEQUENCES_WITH_UNIT_CLASS,
                [template_class],
            )
        })
    }

    /// Reverse-lookup `LevelSequence` assets referencing `event_id` in event trigger parameters.
    pub fn sequences_by_event_id(&self, event_id: &str) -> Result<Vec<Sequence>> {
        let trimmed = event_id.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| query_docs(conn, "sequence", SQL_SEQUENCES_BY_EVENT_ID, [trimmed]))
    }

    /// Reverse-lookup `LevelSequence` assets whose events reference `flag_id`.
    pub fn sequences_by_flag(&self, flag_id: i64) -> Result<Vec<Sequence>> {
        self.with_conn(|conn| query_docs(conn, "sequence", SQL_SEQUENCES_BY_FLAG, [flag_id]))
    }
}

/// Compute half-open `[lower, upper)` bounds over SQLite `BINARY` collation for directory prefix
/// matching (`lower = "<base>/"`, `upper = "<base>0"` since ASCII `'/' + 1 == '0'`).
pub fn directory_prefix_bounds(prefix: &str) -> Option<(String, String)> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "/" {
        return Some(("/".to_owned(), "0".to_owned()));
    }
    let normalized = normalize_sequence_path(trimmed);
    let base = normalized.trim_end_matches('/');
    if base.is_empty() {
        return Some(("/".to_owned(), "0".to_owned()));
    }
    Some((format!("{base}/"), format!("{base}0")))
}
