use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::model::RunRecord;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            finished_at INTEGER NOT NULL,
            started_at_ms INTEGER NOT NULL DEFAULT 0,
            finished_at_ms INTEGER NOT NULL DEFAULT 0,
            exit_code INTEGER,
            succeeded INTEGER NOT NULL CHECK (succeeded IN (0, 1))
        );
        CREATE TABLE IF NOT EXISTS run_claims (
            name TEXT PRIMARY KEY,
            owner_token TEXT NOT NULL,
            claimed_at_ms INTEGER NOT NULL,
            lease_expires_at_ms INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_runs_name_finished_at
            ON runs(name, finished_at DESC);
        ",
    )?;
    ensure_run_millisecond_columns(&connection)?;
    migrate_run_milliseconds(&connection)?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_runs_name_finished_at_ms ON runs(name, finished_at_ms DESC)",
        [],
    )?;

    Ok(connection)
}

fn ensure_run_millisecond_columns(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(runs)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if !columns.iter().any(|name| name == "started_at_ms") {
        connection.execute(
            "ALTER TABLE runs ADD COLUMN started_at_ms INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !columns.iter().any(|name| name == "finished_at_ms") {
        connection.execute(
            "ALTER TABLE runs ADD COLUMN finished_at_ms INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn migrate_run_milliseconds(connection: &Connection) -> Result<()> {
    connection.execute(
        "UPDATE runs SET started_at_ms = started_at * 1000 WHERE started_at_ms = 0",
        [],
    )?;
    connection.execute(
        "UPDATE runs SET finished_at_ms = finished_at * 1000 WHERE finished_at_ms = 0",
        [],
    )?;
    Ok(())
}

pub fn last_run(connection: &Connection, name: &str) -> Result<Option<RunRecord>> {
    let row = connection
        .query_row(
            "
            SELECT name, started_at_ms, finished_at_ms, exit_code, succeeded
            FROM runs
            WHERE name = ?
            ORDER BY finished_at_ms DESC, id DESC
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

pub fn clear_runs(
    connection: &mut Connection,
    name: &str,
    force: bool,
    now_ms: i64,
) -> Result<usize> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let active = tx
        .query_row(
            "SELECT 1 FROM run_claims WHERE name = ? AND lease_expires_at_ms > ?",
            params![name, now_ms],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if active && !force {
        return Err(anyhow!(
            "job has an active claim; use clear --force only if overlap is acceptable"
        ));
    }
    let deleted = tx.execute("DELETE FROM runs WHERE name = ?", params![name])?;
    tx.execute("DELETE FROM run_claims WHERE name = ?", params![name])?;
    tx.commit()?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::{last_run, open};

    #[test]
    fn open_migrates_v0_1_second_precision_rows() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("legacy.sqlite3");
        let legacy = Connection::open(&path).expect("legacy database");
        legacy
            .execute_batch(
                "
                CREATE TABLE runs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    finished_at INTEGER NOT NULL,
                    exit_code INTEGER,
                    succeeded INTEGER NOT NULL CHECK (succeeded IN (0, 1))
                );
                INSERT INTO runs
                    (name, started_at, finished_at, exit_code, succeeded)
                VALUES ('legacy-job', 10, 20, 0, 1);
                ",
            )
            .expect("legacy schema");
        drop(legacy);

        let migrated = open(&path).expect("migrated database");
        let record = last_run(&migrated, "legacy-job")
            .expect("last run")
            .expect("legacy row");
        assert_eq!(record.started_at, 10_000);
        assert_eq!(record.finished_at, 20_000);

        let columns = migrated
            .prepare("PRAGMA table_info(runs)")
            .expect("table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("column names");
        assert!(columns.iter().any(|column| column == "started_at_ms"));
        assert!(columns.iter().any(|column| column == "finished_at_ms"));
    }
}
