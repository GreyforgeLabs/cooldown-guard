# STARTHERE.md - Coding Client Bootstrap Guide

> This file is designed for coding assistants. If you are a human,
> see [README.md](README.md) for the human-friendly guide.

## Quick Bootstrap

```bash
git clone https://github.com/GreyforgeLabs/cooldown-guard.git && cd cooldown-guard && ./scripts/setup.sh
```

## What This Project Does

`cooldown-guard` is a Rust CLI for minimum-interval enforcement. It records completed command runs in SQLite, tells you whether a named job is ready or still cooling down, and can skip re-execution cleanly when the interval has not elapsed.

## Project Structure

```text
cooldown-guard/
  src/
    main.rs              # process entry point
    cli.rs               # clap command definitions and rendering
    db.rs                # SQLite schema and persistence helpers
    guard.rs             # cooldown logic and command execution
    model.rs             # shared data structures
  tests/
    cli.rs               # integration tests for run/status/clear
  scripts/
    setup.sh             # idempotent build and verification script
  .github/workflows/
    ci.yml               # fmt, clippy, and test workflow
  README.md              # human-facing docs
  STARTHERE.md           # this file
```

## Setup Prerequisites

- Rust 1.88+
- `cargo`
- `rustfmt` component (`rustup component add rustfmt`)

## Installation Steps

1. Clone: `git clone https://github.com/GreyforgeLabs/cooldown-guard.git`
2. Enter directory: `cd cooldown-guard`
3. Run setup: `./scripts/setup.sh`

## Verification

```bash
cargo run --locked -- --version
# Expected output: cooldown-guard 0.1.0
```

## Key Entry Points

- `src/cli.rs` - subcommands and output formatting
- `src/guard.rs` - cooldown evaluation and command execution
- `src/db.rs` - SQLite schema and run history queries

## Configuration

- Default state DB: platform state directory for `cooldown-guard`, usually `~/.local/state/cooldown-guard/runs.sqlite3` on Linux
- Override state DB: `--db /path/to/runs.sqlite3`
- Output mode: add `--json`

## Common Tasks

```bash
# Run tests
cargo test --locked

# Format check
cargo fmt --check

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# Run an example command
cargo run -- run --name backup --min-interval 30m -- ./backup.sh
```
