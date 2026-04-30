# cooldown-guard

> Minimum-interval guard for cron jobs and recurring commands.

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

<p align="center">
  <img src="docs/assets/openforge-cooldown-guard.webp" alt="cooldown-guard OpenForge project artwork" width="720">
</p>

## Why This Exists

`flock` solves overlap. It does not solve cadence.

Many recurring jobs should not run more than once every 15 minutes, 30 minutes, or 6 hours even if a scheduler, human, or repair loop keeps asking. `cooldown-guard` is a small Rust CLI that keeps a SQLite ledger of past runs and decides whether the next invocation should execute or skip.

The v0.1 rule is intentionally simple: cooldown is based on the last completed attempt, regardless of whether that attempt exited successfully.

## Quick Start

```bash
git clone https://github.com/GreyforgeLabs/cooldown-guard.git
cd cooldown-guard
./scripts/setup.sh
```

Or run it directly with Cargo:

```bash
cargo run -- run --name backup --min-interval 30m -- ./backup.sh
```

## Features

- **Minimum interval enforcement** - run a command only when its cooldown window has elapsed
- **SQLite state ledger** - durable run history with no daemon and no background service
- **Human and JSON output** - useful in shells, cron logs, and automation wrappers
- **Explicit subcommands** - `run`, `status`, and `clear`
- **Direct execution** - commands are spawned directly, not interpolated through an internal shell

## Usage

```bash
# Run a job if 30 minutes have elapsed since the last completed attempt
cooldown-guard run --name backup --min-interval 30m -- ./backup.sh

# Inspect current cooldown state
cooldown-guard status --name backup --min-interval 30m

# Machine-readable output
cooldown-guard --json status --name backup --min-interval 30m

# Clear stored history for a key
cooldown-guard clear --name backup
```

Example output:

```text
name=backup action=run exit_code=0 finished_at=2026-04-07T15:16:39Z
name=backup action=status state=cooling-down last_exit_code=0 last_finished_at=2026-04-07T15:16:39Z remaining=29m 58s
name=backup action=skip reason=cooldown last_exit_code=0 remaining=29m 41s
```

## Exit Codes

- `0` when a run is skipped because the cooldown is active
- Child process exit code when a command is executed
- `2` on `cooldown-guard` usage or runtime errors

## Documentation

- [STARTHERE.md](STARTHERE.md) - coding client bootstrap
- [CONTRIBUTING.md](CONTRIBUTING.md) - contribution workflow
- [CHANGELOG.md](CHANGELOG.md) - version history

## License

AGPL-3.0. See [LICENSE](LICENSE) for details.

---

Built by [Greyforge](https://greyforge.tech)
