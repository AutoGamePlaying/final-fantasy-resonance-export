//! Reflection area (`06_reflection`) read-only `impl GameDb` queries.
//!
//! All queries use `Connection::prepare_cached` and index-backed predicates over `ue_class`,
//! `ue_property`, `ue_function`, and `ue_enum_value`.

use ffr_data::assets::reflection::{UeClass, UeFunction, UeProperty};
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::GameDb;
use crate::error::{DbError, Result};
use crate::record::{decode_doc, query_doc_opt, query_docs};

/// Maximum inheritance recursion depth to guard against malformed cyclic graphs.
const MAX_INHERITANCE_DEPTH: i64 = 64;

pub(crate) const SQL_CLASS_BY_PATH: &str = "SELECT doc FROM ue_class WHERE path = ?1";

pub(crate) const SQL_CLASS_BY_SHORT_NAME: &str =
    "SELECT path, doc FROM ue_class WHERE short_name = ?1 ORDER BY path ASC";

pub(crate) const SQL_CLASS_PATH_BY_PATH: &str = "SELECT path FROM ue_class WHERE path = ?1";

pub(crate) const SQL_CLASS_PATHS_BY_SHORT_NAME: &str =
    "SELECT path FROM ue_class WHERE short_name = ?1 ORDER BY path ASC";

pub(crate) const SQL_ENUM_BY_PATH: &str =
    "SELECT doc FROM ue_class WHERE path = ?1 AND kind = 'enum'";

pub(crate) const SQL_ENUM_BY_SHORT_NAME: &str =
    "SELECT path, doc FROM ue_class WHERE kind = 'enum' AND short_name = ?1 ORDER BY path ASC";

pub(crate) const SQL_ENUM_PATH_BY_PATH: &str =
    "SELECT path FROM ue_class WHERE path = ?1 AND kind = 'enum'";

pub(crate) const SQL_ENUM_PATHS_BY_SHORT_NAME: &str =
    "SELECT path FROM ue_class WHERE kind = 'enum' AND short_name = ?1 ORDER BY path ASC";

pub(crate) const SQL_CLASS_CHAIN: &str = "\
WITH RECURSIVE chain(path, super_class, doc, depth) AS (
    SELECT path, super_class, doc, 0
    FROM ue_class
    WHERE path = ?1
    UNION ALL
    SELECT ue_class.path, ue_class.super_class, ue_class.doc, chain.depth + 1
    FROM chain
    CROSS JOIN ue_class ON ue_class.path = chain.super_class
    WHERE chain.super_class IS NOT NULL
      AND chain.depth < ?2
)
SELECT path, doc FROM chain ORDER BY depth ASC";

pub(crate) const SQL_OWN_PROPERTIES: &str = "\
SELECT owner || '.' || name, doc
FROM ue_property
WHERE owner = ?1
ORDER BY offset ASC, ord ASC";

pub(crate) const SQL_INHERITED_PROPERTIES: &str = "\
WITH RECURSIVE chain(path, super_class, depth) AS (
    SELECT path, super_class, 0
    FROM ue_class
    WHERE path = ?1
    UNION ALL
    SELECT ue_class.path, ue_class.super_class, chain.depth + 1
    FROM chain
    CROSS JOIN ue_class ON ue_class.path = chain.super_class
    WHERE chain.super_class IS NOT NULL
      AND chain.depth < ?2
)
SELECT ue_property.owner || '.' || ue_property.name, ue_property.doc
FROM chain
CROSS JOIN ue_property ON ue_property.owner = chain.path
ORDER BY ue_property.offset ASC, chain.depth DESC, ue_property.ord ASC";

pub(crate) const SQL_DIRECT_SUBCLASSES: &str = "\
SELECT path, doc
FROM ue_class
WHERE super_class = ?1
ORDER BY path ASC";

pub(crate) const SQL_RECURSIVE_SUBCLASSES: &str = "\
WITH RECURSIVE subs(path, doc, depth) AS (
    SELECT path, doc, 1
    FROM ue_class
    WHERE super_class = ?1
    UNION ALL
    SELECT ue_class.path, ue_class.doc, subs.depth + 1
    FROM subs
    CROSS JOIN ue_class ON ue_class.super_class = subs.path
    WHERE subs.depth < ?2
)
SELECT path, doc FROM subs ORDER BY depth ASC, path ASC";

pub(crate) const SQL_ENUM_VALUE_BY_FULL_NAME: &str = "\
SELECT value FROM ue_enum_value WHERE enum = ?1 AND name = ?2";

pub(crate) const SQL_ENUM_VALUE_BY_SHORT_NAME: &str = "\
SELECT value FROM ue_enum_value WHERE enum = ?1 AND short_name = ?2 ORDER BY ord ASC LIMIT 1";

pub(crate) const SQL_ENUM_NAME_BY_VALUE: &str = "\
SELECT name FROM ue_enum_value WHERE enum = ?1 AND value = ?2 ORDER BY ord ASC LIMIT 1";

pub(crate) const SQL_ENUM_NAMES_BY_VALUE: &str = "\
SELECT name FROM ue_enum_value WHERE enum = ?1 AND value = ?2 ORDER BY ord ASC";

pub(crate) const SQL_CLASSES_WITH_PROPERTY: &str = "\
SELECT ue_class.path, ue_class.doc
FROM ue_property
CROSS JOIN ue_class ON ue_class.path = ue_property.owner
WHERE ue_property.name = ?1
ORDER BY ue_class.path ASC";

pub(crate) const SQL_OWN_FUNCTIONS: &str = "\
SELECT owner || ':' || name, doc
FROM ue_function
WHERE owner = ?1
ORDER BY name ASC";

pub(crate) const SQL_INHERITED_FUNCTIONS: &str = "\
WITH RECURSIVE chain(path, super_class, depth) AS (
    SELECT path, super_class, 0
    FROM ue_class
    WHERE path = ?1
    UNION ALL
    SELECT ue_class.path, ue_class.super_class, chain.depth + 1
    FROM chain
    CROSS JOIN ue_class ON ue_class.path = chain.super_class
    WHERE chain.super_class IS NOT NULL
      AND chain.depth < ?2
)
SELECT ue_function.owner || ':' || ue_function.name, ue_function.doc
FROM chain
CROSS JOIN ue_function ON ue_function.owner = chain.path
ORDER BY chain.depth ASC, ue_function.name ASC";

impl GameDb {
    /// Look up a reflected class, struct, or enum by full UE path (e.g.
    /// `"/Script/mucha.CPP_UI_BattleSubsystem"`) or short name (e.g.
    /// `"CPP_UI_BattleSubsystem"`).
    ///
    /// Returns an error listing all candidate full paths when a short name is ambiguous.
    pub fn ue_class(&self, name: &str) -> Result<Option<UeClass>> {
        self.with_conn(|conn| query_class_by_name(conn, name))
    }

    /// Return the inheritance chain starting from `name` (`depth = 0`) up to its root base class,
    /// computed via a recursive SQLite CTE.
    ///
    /// Returns an empty `Vec` if `name` does not exist; errors if a short name is ambiguous.
    pub fn class_chain(&self, name: &str) -> Result<Vec<UeClass>> {
        self.with_conn(|conn| {
            let Some(root_path) = resolve_class_path(conn, name)? else {
                return Ok(Vec::new());
            };
            query_docs(
                conn,
                "ue_class",
                SQL_CLASS_CHAIN,
                params![root_path, MAX_INHERITANCE_DEPTH],
            )
        })
    }

    /// Return properties of `name` ordered by byte `offset` ascending (then declaration order).
    ///
    /// When `inherited` is `true`, includes properties declared on all ancestor classes/structs.
    pub fn class_properties(&self, name: &str, inherited: bool) -> Result<Vec<UeProperty>> {
        self.with_conn(|conn| {
            let Some(root_path) = resolve_class_path(conn, name)? else {
                return Ok(Vec::new());
            };
            if inherited {
                query_docs(
                    conn,
                    "ue_property",
                    SQL_INHERITED_PROPERTIES,
                    params![root_path, MAX_INHERITANCE_DEPTH],
                )
            } else {
                query_docs(conn, "ue_property", SQL_OWN_PROPERTIES, params![root_path])
            }
        })
    }

    /// Return subclasses of `name` (excluding `name` itself).
    ///
    /// When `recursive` is `false`, returns only direct subclasses; when `true`, traverses the
    /// full subclass tree via a recursive CTE.
    pub fn subclasses(&self, name: &str, recursive: bool) -> Result<Vec<UeClass>> {
        self.with_conn(|conn| {
            let Some(root_path) = resolve_class_path(conn, name)? else {
                return Ok(Vec::new());
            };
            if recursive {
                query_docs(
                    conn,
                    "ue_class",
                    SQL_RECURSIVE_SUBCLASSES,
                    params![root_path, MAX_INHERITANCE_DEPTH],
                )
            } else {
                query_docs(conn, "ue_class", SQL_DIRECT_SUBCLASSES, params![root_path])
            }
        })
    }

    /// Look up a reflected Unreal enum (`kind = 'enum'`) by full path or short name.
    ///
    /// Returns an error listing candidate full paths when a short name matches multiple enums.
    pub fn ue_enum(&self, name: &str) -> Result<Option<UeClass>> {
        self.with_conn(|conn| query_enum_by_name(conn, name))
    }

    /// Look up the integer value of an enumerator in `enum_name`.
    ///
    /// Accepts both full enumerator names (e.g. `"eElement::Ice"`) and unqualified short names
    /// (e.g. `"Ice"`).
    pub fn enum_value(&self, enum_name: &str, entry_name: &str) -> Result<Option<i64>> {
        self.with_conn(|conn| {
            let Some(enum_path) = resolve_enum_path(conn, enum_name)? else {
                return Ok(None);
            };

            let mut exact_stmt = conn
                .prepare_cached(SQL_ENUM_VALUE_BY_FULL_NAME)
                .map_err(|err| DbError::sqlite("准备按完整项名查询 ue_enum_value", err))?;
            let exact: Option<i64> = exact_stmt
                .query_row(params![&enum_path, entry_name], |row| row.get(0))
                .optional()
                .map_err(|err| {
                    DbError::sqlite(format!("查询枚举 {enum_path} 项 {entry_name}"), err)
                })?;
            if exact.is_some() {
                return Ok(exact);
            }

            let mut short_stmt = conn
                .prepare_cached(SQL_ENUM_VALUE_BY_SHORT_NAME)
                .map_err(|err| DbError::sqlite("准备按短项名查询 ue_enum_value", err))?;
            short_stmt
                .query_row(params![&enum_path, entry_name], |row| row.get(0))
                .optional()
                .map_err(|err| {
                    DbError::sqlite(format!("按短项名查询枚举 {enum_path} 项 {entry_name}"), err)
                })
        })
    }

    /// Look up the primary enumerator name for integer `value` in `enum_name` (in declaration order).
    pub fn enum_name(&self, enum_name: &str, value: i64) -> Result<Option<String>> {
        self.with_conn(|conn| {
            let Some(enum_path) = resolve_enum_path(conn, enum_name)? else {
                return Ok(None);
            };
            let mut stmt = conn
                .prepare_cached(SQL_ENUM_NAME_BY_VALUE)
                .map_err(|err| DbError::sqlite("准备按数值查询 ue_enum_value", err))?;
            stmt.query_row(params![&enum_path, value], |row| row.get(0))
                .optional()
                .map_err(|err| DbError::sqlite(format!("查询枚举 {enum_path} 数值 {value}"), err))
        })
    }

    /// Look up all enumerator names (including aliases) matching integer `value` in `enum_name`.
    pub fn enum_names(&self, enum_name: &str, value: i64) -> Result<Vec<String>> {
        self.with_conn(|conn| {
            let Some(enum_path) = resolve_enum_path(conn, enum_name)? else {
                return Ok(Vec::new());
            };
            let mut stmt = conn
                .prepare_cached(SQL_ENUM_NAMES_BY_VALUE)
                .map_err(|err| DbError::sqlite("准备按数值批量查询 ue_enum_value", err))?;
            let mut rows = stmt.query(params![&enum_path, value]).map_err(|err| {
                DbError::sqlite(format!("批量查询枚举 {enum_path} 数值 {value}"), err)
            })?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| DbError::sqlite("读取 ue_enum_value 行", err))?
            {
                out.push(
                    row.get(0)
                        .map_err(|err| DbError::sqlite("读取 ue_enum_value.name", err))?,
                );
            }
            Ok(out)
        })
    }

    /// Return all reflected classes/structs that directly declare a property named `prop_name`.
    pub fn classes_with_property(&self, prop_name: &str) -> Result<Vec<UeClass>> {
        self.with_conn(|conn| query_docs(conn, "ue_class", SQL_CLASSES_WITH_PROPERTY, [prop_name]))
    }

    /// Return reflected functions declared on `name` (optionally including inherited functions).
    pub fn class_functions(&self, name: &str, inherited: bool) -> Result<Vec<UeFunction>> {
        self.with_conn(|conn| {
            let Some(root_path) = resolve_class_path(conn, name)? else {
                return Ok(Vec::new());
            };
            if inherited {
                query_docs(
                    conn,
                    "ue_function",
                    SQL_INHERITED_FUNCTIONS,
                    params![root_path, MAX_INHERITANCE_DEPTH],
                )
            } else {
                query_docs(conn, "ue_function", SQL_OWN_FUNCTIONS, params![root_path])
            }
        })
    }
}

fn looks_like_full_path(name: &str) -> bool {
    name.contains('/')
}

fn query_class_by_name(conn: &Connection, name: &str) -> Result<Option<UeClass>> {
    if looks_like_full_path(name) {
        return query_doc_opt(conn, "ue_class", name, SQL_CLASS_BY_PATH, [name]);
    }

    // If caller passed a dotted path without a leading slash, try exact path lookup first.
    if name.contains('.')
        && let Some(found) = query_doc_opt(conn, "ue_class", name, SQL_CLASS_BY_PATH, [name])?
    {
        return Ok(Some(found));
    }

    let matches = fetch_path_doc_pairs(conn, SQL_CLASS_BY_SHORT_NAME, name, "反射类型")?;
    take_unique_doc(matches, name, "反射类型")
}

fn resolve_class_path(conn: &Connection, name: &str) -> Result<Option<String>> {
    if looks_like_full_path(name) {
        return fetch_single_path(conn, SQL_CLASS_PATH_BY_PATH, name);
    }

    if name.contains('.')
        && let Some(found) = fetch_single_path(conn, SQL_CLASS_PATH_BY_PATH, name)?
    {
        return Ok(Some(found));
    }

    let matches = fetch_paths(conn, SQL_CLASS_PATHS_BY_SHORT_NAME, name, "反射类型")?;
    take_unique_path(matches, name, "反射类型")
}

fn query_enum_by_name(conn: &Connection, name: &str) -> Result<Option<UeClass>> {
    if looks_like_full_path(name) {
        return query_doc_opt(conn, "ue_class", name, SQL_ENUM_BY_PATH, [name]);
    }

    if name.contains('.')
        && let Some(found) = query_doc_opt(conn, "ue_class", name, SQL_ENUM_BY_PATH, [name])?
    {
        return Ok(Some(found));
    }

    let matches = fetch_path_doc_pairs(conn, SQL_ENUM_BY_SHORT_NAME, name, "枚举")?;
    take_unique_doc(matches, name, "枚举")
}

fn resolve_enum_path(conn: &Connection, name: &str) -> Result<Option<String>> {
    if looks_like_full_path(name) {
        return fetch_single_path(conn, SQL_ENUM_PATH_BY_PATH, name);
    }

    if name.contains('.')
        && let Some(found) = fetch_single_path(conn, SQL_ENUM_PATH_BY_PATH, name)?
    {
        return Ok(Some(found));
    }

    let matches = fetch_paths(conn, SQL_ENUM_PATHS_BY_SHORT_NAME, name, "枚举")?;
    take_unique_path(matches, name, "枚举")
}

fn fetch_single_path(conn: &Connection, sql: &str, name: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite("准备按路径查询 ue_class", err))?;
    stmt.query_row([name], |row| row.get(0))
        .optional()
        .map_err(|err| DbError::sqlite(format!("按路径查询 ue_class ({name})"), err))
}

fn fetch_paths(conn: &Connection, sql: &str, short_name: &str, label: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备按短名查询{label}"), err))?;
    let mut rows = stmt
        .query([short_name])
        .map_err(|err| DbError::sqlite(format!("按短名查询{label} ({short_name})"), err))?;
    let mut paths = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取{label}短名候选"), err))?
    {
        paths.push(
            row.get(0)
                .map_err(|err| DbError::sqlite("读取 ue_class.path", err))?,
        );
    }
    Ok(paths)
}

fn fetch_path_doc_pairs(
    conn: &Connection,
    sql: &str,
    short_name: &str,
    label: &str,
) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|err| DbError::sqlite(format!("准备按短名查询{label}"), err))?;
    let mut rows = stmt
        .query([short_name])
        .map_err(|err| DbError::sqlite(format!("按短名查询{label} ({short_name})"), err))?;
    let mut pairs = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| DbError::sqlite(format!("读取{label}短名候选"), err))?
    {
        let path: String = row
            .get(0)
            .map_err(|err| DbError::sqlite("读取 ue_class.path", err))?;
        let doc: String = row
            .get(1)
            .map_err(|err| DbError::sqlite(format!("读取 ue_class ({path}) 的 doc 列"), err))?;
        pairs.push((path, doc));
    }
    Ok(pairs)
}

fn take_unique_path(
    mut paths: Vec<String>,
    short_name: &str,
    label: &str,
) -> Result<Option<String>> {
    match paths.len() {
        0 => Ok(None),
        1 => Ok(paths.pop()),
        _ => Err(ambiguous_short_name_error(short_name, label, &paths)),
    }
}

fn take_unique_doc(
    mut pairs: Vec<(String, String)>,
    short_name: &str,
    label: &str,
) -> Result<Option<UeClass>> {
    match pairs.len() {
        0 => Ok(None),
        1 => {
            let (path, doc) = pairs.pop().expect("len == 1");
            Ok(Some(decode_doc("ue_class", &path, &doc)?))
        }
        _ => {
            let candidates: Vec<String> = pairs.into_iter().map(|(path, _)| path).collect();
            Err(ambiguous_short_name_error(short_name, label, &candidates))
        }
    }
}

fn ambiguous_short_name_error(short_name: &str, label: &str, candidates: &[String]) -> DbError {
    DbError::Sqlite {
        action: format!(
            "{label}短名 \"{short_name}\" 存在重名，请改用完整路径；候选：{}",
            candidates.join("、")
        ),
        source: rusqlite::Error::QueryReturnedMoreThanOneRow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::Stage;
    use crate::testing::{assert_db_uses_index, build_fixture_db_stages};

    const REFLECTION_TABLES: &[&str] = &["ue_class", "ue_property", "ue_function", "ue_enum_value"];

    #[test]
    fn every_reflection_query_sql_uses_index() {
        let test_db = build_fixture_db_stages("reflection", Some(vec![Stage::Reflection]))
            .expect("build reflection fixture db");
        let db = test_db.db();

        assert_db_uses_index(
            db,
            SQL_CLASS_BY_PATH,
            ["/Script/mucha.CPP_TestChild"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_CLASS_BY_SHORT_NAME,
            ["CPP_TestChild"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_CLASS_PATH_BY_PATH,
            ["/Script/mucha.CPP_TestChild"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_CLASS_PATHS_BY_SHORT_NAME,
            ["CPP_TestChild"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_BY_PATH,
            ["/Script/mucha.eMockElement"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_BY_SHORT_NAME,
            ["eMockElement"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_PATH_BY_PATH,
            ["/Script/mucha.eMockElement"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_PATHS_BY_SHORT_NAME,
            ["eMockElement"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_CLASS_CHAIN,
            params!["/Script/mucha.CPP_TestGrandChild", MAX_INHERITANCE_DEPTH],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_OWN_PROPERTIES,
            ["/Script/mucha.CPP_TestGrandChild"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_INHERITED_PROPERTIES,
            params!["/Script/mucha.CPP_TestGrandChild", MAX_INHERITANCE_DEPTH],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_DIRECT_SUBCLASSES,
            ["/Script/CoreUObject.Object"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_RECURSIVE_SUBCLASSES,
            params!["/Script/CoreUObject.Object", MAX_INHERITANCE_DEPTH],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_VALUE_BY_FULL_NAME,
            ["/Script/mucha.eMockElement", "eMockElement::Ice"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_VALUE_BY_SHORT_NAME,
            ["/Script/mucha.eMockElement", "Ice"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_NAME_BY_VALUE,
            params!["/Script/mucha.eMockElement", 2_i64],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_ENUM_NAMES_BY_VALUE,
            params!["/Script/mucha.eMockElement", 2_i64],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_CLASSES_WITH_PROPERTY,
            ["SharedProp"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_OWN_FUNCTIONS,
            ["/Script/mucha.CPP_TestChild"],
            REFLECTION_TABLES,
        );
        assert_db_uses_index(
            db,
            SQL_INHERITED_FUNCTIONS,
            params!["/Script/mucha.CPP_TestGrandChild", MAX_INHERITANCE_DEPTH],
            REFLECTION_TABLES,
        );
    }
}
