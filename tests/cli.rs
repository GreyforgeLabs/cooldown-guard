use std::fs;
use std::thread;
use std::time::Duration;

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
