use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::db;
use crate::model::{ClearResult, GuardState, RunRecord, RunResult, StatusResult};

pub fn parse_min_interval(value: &str) -> Result<Duration> {
    let duration =
        humantime::parse_duration(value).with_context(|| format!("invalid duration: {value}"))?;

    if duration.is_zero() {
        return Err(anyhow!("min interval must be greater than zero"));
    }

    Ok(duration)
}

pub fn default_db_path() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from("tech", "Greyforge", "cooldown-guard")
        .context("could not resolve a default state directory")?;

    if let Some(state_dir) = project_dirs.state_dir() {
        return Ok(state_dir.join("runs.sqlite3"));
    }

    Ok(project_dirs.data_local_dir().join("runs.sqlite3"))
}

pub fn open_database(path: &Path) -> Result<Connection> {
    db::open(path)
}

pub fn status(connection: &Connection, name: &str, min_interval: Duration) -> Result<StatusResult> {
    let now = now_utc().unix_timestamp();
    let last = db::last_run(connection, name)?;

    Ok(match last {
        None => StatusResult {
            name: name.to_owned(),
            state: GuardState::NeverRun,
            last_exit_code: None,
            last_succeeded: None,
            last_started_at: None,
            last_finished_at: None,
            elapsed_seconds: None,
            remaining_seconds: None,
        },
        Some(record) => {
            let elapsed_seconds = elapsed_seconds(now, record.finished_at);
            let remaining_seconds = remaining_seconds(elapsed_seconds, min_interval);
            let state = if remaining_seconds.is_some() {
                GuardState::CoolingDown
            } else {
                GuardState::Ready
            };

            StatusResult {
                name: name.to_owned(),
                state,
                last_exit_code: record.exit_code,
                last_succeeded: Some(record.succeeded),
                last_started_at: Some(format_timestamp(record.started_at)?),
                last_finished_at: Some(format_timestamp(record.finished_at)?),
                elapsed_seconds: Some(elapsed_seconds),
                remaining_seconds,
            }
        }
    })
}

pub fn run_guarded(
    connection: &mut Connection,
    name: &str,
    min_interval: Duration,
    command: &[String],
) -> Result<RunResult> {
    if command.is_empty() {
        return Err(anyhow!("missing command to execute"));
    }

    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current_status = status_with_transaction(&tx, name, min_interval)?;
    if let Some(remaining_seconds) = current_status.remaining_seconds {
        return Ok(RunResult {
            name: name.to_owned(),
            action: "skip",
            skipped: true,
            exit_code: None,
            last_exit_code: current_status.last_exit_code,
            started_at: None,
            finished_at: None,
            remaining_seconds: Some(remaining_seconds),
        });
    }

    let started_at = now_utc().unix_timestamp();
    let status = Command::new(&command[0])
        .args(&command[1..])
        .status()
        .with_context(|| format!("failed to execute {:?}", command))?;
    let finished_at = now_utc().unix_timestamp();

    let record = RunRecord {
        name: name.to_owned(),
        started_at,
        finished_at,
        exit_code: status.code(),
        succeeded: status.success(),
    };
    tx.execute(
        "
        INSERT INTO runs (name, started_at, finished_at, exit_code, succeeded)
        VALUES (?, ?, ?, ?, ?)
        ",
        params![
            record.name,
            record.started_at,
            record.finished_at,
            record.exit_code,
            if record.succeeded { 1 } else { 0 }
        ],
    )?;
    tx.commit()?;

    Ok(RunResult {
        name: name.to_owned(),
        action: "run",
        skipped: false,
        exit_code: record.exit_code,
        last_exit_code: None,
        started_at: Some(format_timestamp(started_at)?),
        finished_at: Some(format_timestamp(finished_at)?),
        remaining_seconds: None,
    })
}

fn status_with_transaction(
    tx: &rusqlite::Transaction<'_>,
    name: &str,
    min_interval: Duration,
) -> Result<StatusResult> {
    let last = tx
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

    let now = now_utc().unix_timestamp();

    Ok(match last {
        None => StatusResult {
            name: name.to_owned(),
            state: GuardState::NeverRun,
            last_exit_code: None,
            last_succeeded: None,
            last_started_at: None,
            last_finished_at: None,
            elapsed_seconds: None,
            remaining_seconds: None,
        },
        Some(record) => {
            let elapsed_seconds = elapsed_seconds(now, record.finished_at);
            let remaining_seconds = remaining_seconds(elapsed_seconds, min_interval);
            let state = if remaining_seconds.is_some() {
                GuardState::CoolingDown
            } else {
                GuardState::Ready
            };

            StatusResult {
                name: name.to_owned(),
                state,
                last_exit_code: record.exit_code,
                last_succeeded: Some(record.succeeded),
                last_started_at: Some(format_timestamp(record.started_at)?),
                last_finished_at: Some(format_timestamp(record.finished_at)?),
                elapsed_seconds: Some(elapsed_seconds),
                remaining_seconds,
            }
        }
    })
}

pub fn clear(connection: &Connection, name: &str) -> Result<ClearResult> {
    let deleted_runs = db::clear_runs(connection, name)?;
    Ok(ClearResult {
        name: name.to_owned(),
        deleted_runs,
    })
}

pub fn format_duration(seconds: u64) -> String {
    humantime::format_duration(Duration::from_secs(seconds)).to_string()
}

fn elapsed_seconds(now: i64, finished_at: i64) -> u64 {
    now.saturating_sub(finished_at).max(0) as u64
}

fn remaining_seconds(elapsed_seconds: u64, min_interval: Duration) -> Option<u64> {
    let min_interval_seconds = min_interval.as_secs();
    if elapsed_seconds < min_interval_seconds {
        Some(min_interval_seconds - elapsed_seconds)
    } else {
        None
    }
}

fn format_timestamp(unix_seconds: i64) -> Result<String> {
    let timestamp = OffsetDateTime::from_unix_timestamp(unix_seconds)?;
    Ok(timestamp.format(&Rfc3339)?)
}

fn now_utc() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{parse_min_interval, remaining_seconds};

    #[test]
    fn parse_duration_requires_positive_value() {
        assert!(parse_min_interval("15m").is_ok());
        assert!(parse_min_interval("0s").is_err());
    }

    #[test]
    fn cooldown_remaining_is_none_after_interval() {
        assert_eq!(remaining_seconds(120, Duration::from_secs(60)), None);
        assert_eq!(remaining_seconds(30, Duration::from_secs(60)), Some(30));
    }
}
