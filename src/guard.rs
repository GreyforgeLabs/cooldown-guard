use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::db;
use crate::model::{ClearResult, GuardState, RunRecord, RunResult, StatusResult};

const MAX_HISTORY_PER_JOB: i64 = 1000;
static CLAIM_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn parse_min_interval(value: &str) -> Result<Duration> {
    parse_positive_duration(value, "min interval")
}

pub fn parse_failure_backoff(value: &str) -> Result<Duration> {
    parse_positive_duration(value, "failure backoff")
}

pub fn parse_lease_duration(value: &str) -> Result<Duration> {
    parse_positive_duration(value, "lease")
}

fn parse_positive_duration(value: &str, label: &str) -> Result<Duration> {
    let duration =
        humantime::parse_duration(value).with_context(|| format!("invalid duration: {value}"))?;

    if duration.is_zero() {
        return Err(anyhow!("{label} must be greater than zero"));
    }
    if duration.as_nanos() % 1_000_000 != 0 {
        return Err(anyhow!("{label} must use whole-millisecond precision"));
    }
    i64::try_from(duration.as_millis())
        .with_context(|| format!("{label} is too large to store"))?;

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

pub fn status(
    connection: &Connection,
    name: &str,
    min_interval: Duration,
    failure_backoff: Duration,
) -> Result<StatusResult> {
    validate_job_name(name)?;
    status_at(connection, name, min_interval, failure_backoff, now_ms())
}

pub fn run_guarded(
    connection: &mut Connection,
    name: &str,
    min_interval: Duration,
    failure_backoff: Duration,
    lease_duration: Duration,
    command: &[String],
) -> Result<RunResult> {
    validate_job_name(name)?;
    if command.is_empty() {
        return Err(anyhow!("missing command to execute"));
    }

    let owner_token = claim_token(name);
    let claim = claim_run(
        connection,
        name,
        min_interval,
        failure_backoff,
        lease_duration,
        &owner_token,
    )?;
    if let ClaimOutcome::CoolingDown(current_status) = claim {
        return Ok(RunResult {
            name: name.to_owned(),
            action: "skip",
            skipped: true,
            exit_code: None,
            last_exit_code: current_status.last_exit_code,
            started_at: None,
            finished_at: None,
            remaining_seconds: current_status.remaining_seconds,
        });
    }

    let started_at = now_ms();
    let status = Command::new(&command[0])
        .args(&command[1..])
        .status()
        .with_context(|| format!("failed to execute {:?}", command));
    let finished_at = now_ms();

    let record = RunRecord {
        name: name.to_owned(),
        started_at,
        finished_at,
        exit_code: status
            .as_ref()
            .ok()
            .and_then(std::process::ExitStatus::code),
        succeeded: status.as_ref().is_ok_and(std::process::ExitStatus::success),
    };
    finalize_run(connection, &owner_token, &record)?;
    status?;

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

#[derive(Debug)]
enum ClaimOutcome {
    Claimed,
    CoolingDown(StatusResult),
}

fn claim_run(
    connection: &mut Connection,
    name: &str,
    min_interval: Duration,
    failure_backoff: Duration,
    lease_duration: Duration,
    owner_token: &str,
) -> Result<ClaimOutcome> {
    let lease_ms = duration_ms_i64(lease_duration, "lease")?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now_ms();
    tx.execute(
        "DELETE FROM run_claims WHERE lease_expires_at_ms <= ?",
        params![now],
    )?;
    let current_status = status_at(&tx, name, min_interval, failure_backoff, now)?;
    if current_status.remaining_seconds.is_some() {
        tx.rollback()?;
        return Ok(ClaimOutcome::CoolingDown(current_status));
    }
    tx.execute(
        "
        INSERT INTO run_claims (name, owner_token, claimed_at_ms, lease_expires_at_ms)
        VALUES (?, ?, ?, ?)
        ",
        params![name, owner_token, now, now.saturating_add(lease_ms)],
    )?;
    tx.commit()?;
    Ok(ClaimOutcome::Claimed)
}

fn finalize_run(connection: &mut Connection, owner_token: &str, record: &RunRecord) -> Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now_ms();
    let updated = tx.execute(
        "
        DELETE FROM run_claims
        WHERE name = ? AND owner_token = ? AND lease_expires_at_ms > ?
        ",
        params![record.name, owner_token, now],
    )?;
    if updated == 0 {
        tx.execute(
            "DELETE FROM run_claims WHERE name = ? AND lease_expires_at_ms <= ?",
            params![record.name, now],
        )?;
        tx.commit()?;
        return Err(anyhow!("run claim expired or was replaced before finalize"));
    }
    tx.execute(
        "
        INSERT INTO runs
            (name, started_at, finished_at, started_at_ms, finished_at_ms, exit_code, succeeded)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ",
        params![
            record.name,
            record.started_at / 1000,
            record.finished_at / 1000,
            record.started_at,
            record.finished_at,
            record.exit_code,
            if record.succeeded { 1 } else { 0 }
        ],
    )?;
    tx.execute(
        "
        DELETE FROM runs
        WHERE name = ?
          AND id NOT IN (
            SELECT id FROM runs WHERE name = ? ORDER BY finished_at_ms DESC, id DESC LIMIT ?
          )
        ",
        params![record.name, record.name, MAX_HISTORY_PER_JOB],
    )?;
    tx.commit()?;
    Ok(())
}

fn status_at(
    connection: &Connection,
    name: &str,
    min_interval: Duration,
    failure_backoff: Duration,
    now: i64,
) -> Result<StatusResult> {
    let last = db::last_run(connection, name)?;
    let active_lease_expiry = connection
        .query_row(
            "SELECT lease_expires_at_ms FROM run_claims WHERE name = ? AND lease_expires_at_ms > ?",
            params![name, now],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;

    let lease_remaining = active_lease_expiry
        .map(|expires_at| remaining_ms_as_seconds(expires_at.saturating_sub(now) as u64));

    let (
        record_remaining,
        last_exit_code,
        last_succeeded,
        last_started_at,
        last_finished_at,
        elapsed_seconds,
    ) = match last {
        None => (None, None, None, None, None, None),
        Some(record) => {
            let elapsed = elapsed_ms(now, record.finished_at);
            let interval = if record.succeeded {
                min_interval
            } else {
                failure_backoff
            };
            (
                remaining_seconds(elapsed, interval),
                record.exit_code,
                Some(record.succeeded),
                Some(format_timestamp(record.started_at)?),
                Some(format_timestamp(record.finished_at)?),
                Some(elapsed / 1000),
            )
        }
    };
    let remaining_seconds = match (record_remaining, lease_remaining) {
        (Some(record), Some(lease)) => Some(record.max(lease)),
        (Some(record), None) => Some(record),
        (None, Some(lease)) => Some(lease),
        (None, None) => None,
    };
    let state = if remaining_seconds.is_some() {
        GuardState::CoolingDown
    } else if last_succeeded.is_none() {
        GuardState::NeverRun
    } else {
        GuardState::Ready
    };

    Ok(StatusResult {
        name: name.to_owned(),
        state,
        last_exit_code,
        last_succeeded,
        last_started_at,
        last_finished_at,
        elapsed_seconds,
        remaining_seconds,
    })
}

pub fn clear(connection: &Connection, name: &str) -> Result<ClearResult> {
    validate_job_name(name)?;
    let deleted_runs = db::clear_runs(connection, name)?;
    Ok(ClearResult {
        name: name.to_owned(),
        deleted_runs,
    })
}

pub fn format_duration(seconds: u64) -> String {
    humantime::format_duration(Duration::from_secs(seconds)).to_string()
}

fn elapsed_ms(now: i64, finished_at: i64) -> u64 {
    now.saturating_sub(finished_at).max(0) as u64
}

fn remaining_seconds(elapsed_ms: u64, min_interval: Duration) -> Option<u64> {
    let min_interval_ms = duration_ms_u64(min_interval);
    if elapsed_ms < min_interval_ms {
        Some(remaining_ms_as_seconds(min_interval_ms - elapsed_ms))
    } else {
        None
    }
}

fn remaining_ms_as_seconds(remaining_ms: u64) -> u64 {
    remaining_ms.div_ceil(1000)
}

fn duration_ms_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn duration_ms_i64(duration: Duration, label: &str) -> Result<i64> {
    i64::try_from(duration.as_millis()).with_context(|| format!("{label} is too large to store"))
}

fn format_timestamp(unix_ms: i64) -> Result<String> {
    let timestamp = OffsetDateTime::from_unix_timestamp(unix_ms / 1000)?;
    Ok(timestamp.format(&Rfc3339)?)
}

fn now_ms() -> i64 {
    now_utc().unix_timestamp_nanos() as i64 / 1_000_000
}

fn now_utc() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

fn validate_job_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(anyhow!("job name must be 1..128 ASCII characters"));
    }
    let mut bytes = name.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || bytes
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(anyhow!(
            "job name must start with an ASCII letter or digit and contain only letters, digits, '.', '_', ':', or '-'"
        ));
    }
    Ok(())
}

fn claim_token(_name: &str) -> String {
    let sequence = CLAIM_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}:{}:{}",
        std::process::id(),
        now_utc().unix_timestamp_nanos(),
        sequence
    )
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;

    use rusqlite::params;
    use tempfile::TempDir;

    use crate::db;
    use crate::model::RunRecord;

    use super::{
        ClaimOutcome, MAX_HISTORY_PER_JOB, claim_run, finalize_run, parse_min_interval,
        remaining_seconds, run_guarded, validate_job_name,
    };

    #[test]
    fn parse_duration_requires_positive_value() {
        assert!(parse_min_interval("15m").is_ok());
        assert!(parse_min_interval("1ms").is_ok());
        assert!(parse_min_interval("999ms").is_ok());
        assert!(parse_min_interval("1s").is_ok());
        assert!(parse_min_interval("1us").is_err());
        assert!(parse_min_interval("0s").is_err());
    }

    #[test]
    fn cooldown_remaining_is_none_after_interval() {
        assert_eq!(remaining_seconds(120_000, Duration::from_secs(60)), None);
        assert_eq!(remaining_seconds(30_000, Duration::from_secs(60)), Some(30));
        assert_eq!(remaining_seconds(0, Duration::from_millis(1)), Some(1));
        assert_eq!(remaining_seconds(1, Duration::from_millis(999)), Some(1));
        assert_eq!(remaining_seconds(999, Duration::from_millis(999)), None);
        assert_eq!(remaining_seconds(999, Duration::from_secs(1)), Some(1));
    }

    #[test]
    fn job_names_are_normalized_and_bounded() {
        for name in [
            "backup",
            "backup.daily",
            "backup_daily",
            "backup:daily",
            "9-job",
        ] {
            assert!(validate_job_name(name).is_ok(), "{name}");
        }
        for name in [
            "",
            " backup",
            "backup job",
            "backup/job",
            "éclair",
            "-backup",
        ] {
            assert!(validate_job_name(name).is_err(), "{name}");
        }
        assert!(validate_job_name(&"a".repeat(128)).is_ok());
        assert!(validate_job_name(&"a".repeat(129)).is_err());
    }

    #[test]
    fn expired_claim_is_recoverable_and_stale_owner_cannot_finalize() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("runs.sqlite3");
        let mut connection = db::open(&path).expect("database");

        assert!(matches!(
            claim_run(
                &mut connection,
                "backup",
                Duration::from_secs(60),
                Duration::from_secs(5),
                Duration::from_millis(1),
                "old-owner",
            )
            .expect("first claim"),
            ClaimOutcome::Claimed
        ));
        thread::sleep(Duration::from_millis(5));
        assert!(matches!(
            claim_run(
                &mut connection,
                "backup",
                Duration::from_secs(60),
                Duration::from_secs(5),
                Duration::from_secs(60),
                "new-owner",
            )
            .expect("replacement claim"),
            ClaimOutcome::Claimed
        ));

        let record = RunRecord {
            name: "backup".to_owned(),
            started_at: super::now_ms(),
            finished_at: super::now_ms(),
            exit_code: Some(0),
            succeeded: true,
        };
        assert!(finalize_run(&mut connection, "old-owner", &record).is_err());
        finalize_run(&mut connection, "new-owner", &record).expect("current owner finalizes");
    }

    #[test]
    fn expired_crash_claim_recovers_after_database_reopen() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("runs.sqlite3");
        {
            let mut connection = db::open(&path).expect("database");
            assert!(matches!(
                claim_run(
                    &mut connection,
                    "backup",
                    Duration::from_secs(60),
                    Duration::from_secs(5),
                    Duration::from_millis(1),
                    "crashed-owner",
                )
                .expect("claim"),
                ClaimOutcome::Claimed
            ));
        }
        thread::sleep(Duration::from_millis(5));

        let mut reopened = db::open(&path).expect("reopened database");
        assert!(matches!(
            claim_run(
                &mut reopened,
                "backup",
                Duration::from_secs(60),
                Duration::from_secs(5),
                Duration::from_secs(60),
                "recovery-owner",
            )
            .expect("recovery claim"),
            ClaimOutcome::Claimed
        ));
    }

    #[test]
    fn finalized_runs_prune_history_to_policy_limit() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("runs.sqlite3");
        let mut connection = db::open(&path).expect("database");
        for id in 0..(MAX_HISTORY_PER_JOB + 5) {
            connection
                .execute(
                    "
                    INSERT INTO runs
                        (name, started_at, finished_at, started_at_ms, finished_at_ms, exit_code, succeeded)
                    VALUES ('retained', 0, 0, ?, ?, 0, 1)
                    ",
                    params![id, id],
                )
                .expect("history row");
        }

        let command = vec!["true".to_owned()];
        run_guarded(
            &mut connection,
            "retained",
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_secs(30),
            &command,
        )
        .expect("guarded run");
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM runs WHERE name = 'retained'",
                [],
                |row| row.get(0),
            )
            .expect("history count");
        assert_eq!(count, MAX_HISTORY_PER_JOB);
    }

    #[test]
    fn database_busy_wait_is_bounded_and_reports_lock_error() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("runs.sqlite3");
        let blocker = db::open(&path).expect("blocker database");
        let mut contender = db::open(&path).expect("contender database");
        contender
            .busy_timeout(Duration::from_millis(50))
            .expect("test busy timeout");
        blocker
            .execute_batch("BEGIN IMMEDIATE")
            .expect("write lock");

        let started = Instant::now();
        let error = claim_run(
            &mut contender,
            "busy-job",
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            "contender",
        )
        .expect_err("claim should report the lock");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("database is locked"));
        blocker.execute_batch("ROLLBACK").expect("unlock database");
    }
}
