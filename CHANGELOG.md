# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.3.0] - 2026-09-27

### Fixed

- `clear` now checks and deletes state in one transaction, refusing an active claim unless `--force` is supplied.

### Changed

- Explain explicitly that a fixed lease limits overlap protection to its lifetime; a running child can outlive it.
- Document that forced clear abandons a claim without stopping its child.

## [0.2.0] - 2026-08-28

### Added

- Short owner-token claim leases with configurable expiry and crash recovery
- Configurable failure backoff for spawn failures and nonzero command exits
- Strict normalized job names and a 1,000-row per-job retention policy
- Regression coverage for contention, stale owners, crash recovery, bounded database waits, failure retries, subsecond durations, and legacy migration

### Changed

- Run child commands outside SQLite write transactions so unrelated jobs can execute concurrently
- Store timestamps at whole-millisecond precision while migrating existing v0.1 rows in place
- Report active leases through status and reject stale owners during finalization

## [0.1.0] - 2026-04-07

### Added

- Initial Rust CLI for minimum-interval command enforcement
- SQLite-backed run history with `run`, `status`, and `clear` subcommands
- JSON output mode plus human-readable shell output
- Integration tests and GitHub Actions CI
- README, STARTHERE bootstrap, and setup script
