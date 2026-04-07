use std::fs;
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::model::RunRecord;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let connection = Connection::open(path)?;
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            finished_at INTEGER NOT NULL,
            exit_code INTEGER,
            succeeded INTEGER NOT NULL CHECK (succeeded IN (0, 1))
        );

        CREATE INDEX IF NOT EXISTS idx_runs_name_finished_at
            ON runs(name, finished_at DESC);
        ",
    )?;

    Ok(connection)
}

pub fn last_run(connection: &Connection, name: &str) -> Result<Option<RunRecord>> {
    let row = connection
        .query_row(
            "
            SELECT name, started_at, finished_at, exit_code, succeeded
            FROM runs
            WHERE name = ?
            ORDER BY finished_at DESC, id DESC
            LIMIT 1
            ",
            params![name],
            |row| {
                Ok(RunRecord {
                    name: row.get(0)?,
                    started_at: row.get(1)?,
                    finished_at: row.get(2)?,
                    exit_code: row.get(3)?,
                    succeeded: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()?;

    Ok(row)
}

pub fn clear_runs(connection: &Connection, name: &str) -> Result<usize> {
    let deleted = connection.execute("DELETE FROM runs WHERE name = ?", params![name])?;
    Ok(deleted)
}
