//! Opening one of the app's own SQLite files in the format this dotori writes.

use std::path::Path;

use rusqlite::Connection;

/// Opens the SQLite file at `path`, makes its tables with `schema`, and marks the file with
/// `version` in `PRAGMA user_version`. A file a newer dotori wrote is refused. A file from before
/// formats were marked holds the first format. Raising a version needs its upgrade here first.
pub(crate) fn open(path: &Path, schema: &str, version: i64) -> Result<Connection, String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let fail = |error: rusqlite::Error| format!("{}: {error}", path.display());
    let connection = Connection::open(path).map_err(fail)?;
    let found: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(fail)?;
    if found > version {
        return Err(format!(
            "{} is from a newer dotori (format {found})",
            path.display()
        ));
    }
    connection.execute_batch(schema).map_err(fail)?;
    if found != version {
        connection
            .pragma_update(None, "user_version", version)
            .map_err(fail)?;
    }
    Ok(connection)
}

#[cfg(test)]
#[path = "versioned.test.rs"]
mod tests;
