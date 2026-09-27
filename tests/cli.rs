use std::fs;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("cooldown-guard").expect("binary should build")
}

fn temp_paths() -> (TempDir, String, String) {
    let temp = TempDir::new().expect("tempdir");
    let db = temp.path().join("runs.sqlite3");
    let marker = temp.path().join("marker.txt");

    (
        temp,
        db.to_string_lossy().into_owned(),
        marker.to_string_lossy().into_owned(),
    )
}

#[test]
fn run_executes_command_on_first_invocation() {
    let (_temp, db, marker) = temp_paths();

    let mut command = bin();
    command.args([
        "--db",
        &db,
        "run",
        "--name",
        "backup",
        "--min-interval",
        "30m",
        "--",
        "sh",
        "-c",
        &format!("printf first >> {marker}"),
    ]);
    command.assert().success().stdout(contains("action=run"));

    assert_eq!(fs::read_to_string(marker).unwrap(), "first");
}

#[test]
fn run_skips_when_cooldown_window_is_active() {
    let (_temp, db, marker) = temp_paths();

    let mut first = bin();
    first.args([
        "--db",
        &db,
        "run",
        "--name",
        "backup",
        "--min-interval",
        "30m",
        "--",
        "sh",
        "-c",
        &format!("printf first >> {marker}"),
    ]);
    first.assert().success();

    let mut second = bin();
    second.args([
        "--db",
        &db,
        "run",
        "--name",
        "backup",
        "--min-interval",
        "30m",
        "--",
        "sh",
        "-c",
        &format!("printf second >> {marker}"),
    ]);
    second.assert().success().stdout(contains("action=skip"));

    assert_eq!(fs::read_to_string(marker).unwrap(), "first");
}

#[test]
fn clear_resets_saved_state() {
    let (_temp, db, marker) = temp_paths();

    let mut run = bin();
    run.args([
        "--db",
        &db,
        "run",
        "--name",
        "backup",
        "--min-interval",
        "30m",
        "--",
        "sh",
        "-c",
        &format!("printf first >> {marker}"),
    ]);
    run.assert().success();

    let mut clear = bin();
    clear.args(["--db", &db, "clear", "--name", "backup"]);
    clear
        .assert()
        .success()
        .stdout(contains("action=clear").and(contains("deleted_runs=1")));

    let mut status = bin();
    status.args([
        "--db",
        &db,
        "status",
        "--name",
        "backup",
        "--min-interval",
        "30m",
    ]);
    status
        .assert()
        .success()
        .stdout(contains("state=never-run"));
}

#[test]
fn clear_refuses_live_claim_without_force() {
    let (_temp, db, marker) = temp_paths();
    let started = format!("{marker}.started");
    let mut running = std::process::Command::new(env!("CARGO_BIN_EXE_cooldown-guard"))
        .args([
            "--db",
            &db,
            "run",
            "--name",
            "backup",
            "--min-interval",
            "10m",
            "--lease",
            "2s",
            "--",
            "sh",
            "-c",
            &format!("printf started > {started}; sleep 0.5; printf A >> {marker}"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("long command should start");
    let wait_started = Instant::now();
    while !std::path::Path::new(&started).exists() {
        assert!(wait_started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(10));
    }

    let mut guarded_clear = bin();
    guarded_clear.args(["--db", &db, "clear", "--name", "backup"]);
    guarded_clear
        .assert()
        .failure()
        .stderr(contains("active claim"));

    let mut status = bin();
    status.args([
        "--db",
        &db,
        "status",
        "--name",
        "backup",
        "--min-interval",
        "10m",
    ]);
    status
        .assert()
        .success()
        .stdout(contains("state=cooling-down"));

    let mut force_clear = bin();
    force_clear.args(["--db", &db, "clear", "--name", "backup", "--force"]);
    force_clear
        .assert()
        .success()
        .stdout(contains("action=clear"));
    assert!(!running.wait().expect("child finishes").success());
}

#[test]
fn expired_lease_allows_overlap_and_stale_owner_cannot_finalize() {
    let (_temp, db, marker) = temp_paths();
    let started = format!("{marker}.started");
    let mut first = std::process::Command::new(env!("CARGO_BIN_EXE_cooldown-guard"))
        .args([
            "--db",
            &db,
            "run",
            "--name",
            "backup",
            "--min-interval",
            "10m",
            "--lease",
            "100ms",
            "--",
            "sh",
            "-c",
            &format!("printf started > {started}; sleep 0.5; printf A >> {marker}"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("first command should start");
    let wait_started = Instant::now();
    while !std::path::Path::new(&started).exists() {
        assert!(wait_started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(180));
    assert!(
        first.try_wait().unwrap().is_none(),
        "first child should still be running"
    );

    let mut second = bin();
    second.args([
        "--db",
        &db,
        "run",
        "--name",
        "backup",
        "--min-interval",
        "10m",
        "--",
        "sh",
        "-c",
        &format!("printf B >> {marker}"),
    ]);
    second.assert().success().stdout(contains("action=run"));
    assert!(!first.wait().expect("first command finishes").success());
    let text = fs::read_to_string(marker).unwrap();
    assert!(text.contains('A') && text.contains('B'));
}

#[test]
fn concurrent_runs_respect_cooldown_when_invoked_together() {
    let (_temp, db, marker) = temp_paths();

    let first_db = db.clone();
    let first_marker = marker.clone();
    let first = thread::spawn(move || {
        let mut command = bin();
        command.args([
            "--db",
            &first_db,
            "run",
            "--name",
            "backup",
            "--min-interval",
            "10m",
            "--",
            "sh",
            "-c",
            &format!("sleep 0.3; printf A >> {first_marker}"),
        ]);
        let output = command.output().expect("first command should execute");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).into_owned()
    });

    thread::sleep(Duration::from_millis(25));

    let second_db = db.clone();
    let second_marker = marker.clone();
    let second = thread::spawn(move || {
        let mut command = bin();
        command.args([
            "--db",
            &second_db,
            "run",
            "--name",
            "backup",
            "--min-interval",
            "10m",
            "--",
            "sh",
            "-c",
            &format!("printf B >> {second_marker}"),
        ]);
        let output = command.output().expect("second command should execute");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).into_owned()
    });

    let first_output = first.join().unwrap();
    let second_output = second.join().unwrap();

    assert!(first_output.contains("action=run"));
    assert!(second_output.contains("action=skip"));

    assert_eq!(fs::read_to_string(marker).unwrap(), "A");
}

#[test]
fn unrelated_jobs_are_not_blocked_by_a_running_command() {
    let (_temp, db, marker) = temp_paths();
    let started_marker = format!("{marker}.started");
    let first_db = db.clone();
    let first_marker = marker.clone();
    let first_started_marker = started_marker.clone();
    let first = thread::spawn(move || {
        let mut command = bin();
        command.args([
            "--db",
            &first_db,
            "run",
            "--name",
            "long-job",
            "--min-interval",
            "10m",
            "--",
            "sh",
            "-c",
            &format!(
                "printf started > {first_started_marker}; sleep 0.6; printf A >> {first_marker}"
            ),
        ]);
        command.output().expect("long command should execute")
    });

    let wait_started = Instant::now();
    while !std::path::Path::new(&started_marker).exists() {
        assert!(wait_started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(10));
    }

    let started = Instant::now();
    let mut second = bin();
    second.args([
        "--db",
        &db,
        "run",
        "--name",
        "short-job",
        "--min-interval",
        "10m",
        "--",
        "sh",
        "-c",
        &format!("printf B >> {marker}"),
    ]);
    second.assert().success().stdout(contains("action=run"));
    assert!(started.elapsed() < Duration::from_millis(400));

    assert!(first.join().unwrap().status.success());
    let contents = fs::read_to_string(marker).unwrap();
    assert!(contents.contains('A'));
    assert!(contents.contains('B'));
}

#[test]
fn spawn_failure_is_recorded_and_uses_configured_backoff() {
    let (_temp, db, _marker) = temp_paths();
    let missing = "/definitely/not/a/cooldown-guard-command";

    let mut first = bin();
    first.args([
        "--db",
        &db,
        "run",
        "--name",
        "missing-command",
        "--min-interval",
        "1ms",
        "--failure-backoff",
        "10m",
        "--",
        missing,
    ]);
    first.assert().code(2).stderr(contains("failed to execute"));

    let mut second = bin();
    second.args([
        "--db",
        &db,
        "run",
        "--name",
        "missing-command",
        "--min-interval",
        "1ms",
        "--failure-backoff",
        "10m",
        "--",
        missing,
    ]);
    second.assert().success().stdout(contains("action=skip"));
}

#[test]
fn nonzero_execution_uses_configured_failure_backoff() {
    let (_temp, db, marker) = temp_paths();

    let mut first = bin();
    first.args([
        "--db",
        &db,
        "run",
        "--name",
        "failing-command",
        "--min-interval",
        "1ms",
        "--failure-backoff",
        "10m",
        "--",
        "sh",
        "-c",
        &format!("printf A >> {marker}; exit 7"),
    ]);
    first.assert().code(7).stdout(contains("action=run"));

    let mut second = bin();
    second.args([
        "--db",
        &db,
        "run",
        "--name",
        "failing-command",
        "--min-interval",
        "1ms",
        "--failure-backoff",
        "10m",
        "--",
        "sh",
        "-c",
        &format!("printf B >> {marker}; exit 7"),
    ]);
    second.assert().success().stdout(contains("action=skip"));
    assert_eq!(fs::read_to_string(marker).unwrap(), "A");
}

#[test]
fn invalid_job_names_are_rejected() {
    let (_temp, db, _marker) = temp_paths();
    let mut command = bin();
    command.args([
        "--db",
        &db,
        "status",
        "--name",
        "not normalized",
        "--min-interval",
        "1s",
    ]);
    command
        .assert()
        .code(2)
        .stderr(contains("job name must start"));
}
