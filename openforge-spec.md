# cooldown-guard - Implementation Spec

**Status:** In development
**Pipeline:** `forge openforge cooldown-guard`
**License:** AGPL-3.0
**Repo:** `github.com/GreyforgeLabs/cooldown-guard`
**Version:** v0.1.0
**Language:** Rust

---

## 1. What It Is

A small Rust CLI for minimum-interval enforcement.

It solves the gap between:

- overlap locks like `flock`, which stop concurrent execution but do not enforce cadence
- full scheduler platforms, which are far heavier than needed when the only rule is "do not run this again yet"

**Audience:** operators, homelab users, cron-heavy environments, repair loops, and maintenance pipelines.

## 2. v0.1 Scope

The initial release stays narrow:

- `run` executes a command only if the cooldown has elapsed
- `status` reports whether a named key is ready or still cooling down
- `clear` removes saved state for a key
- SQLite-backed run history
- human-readable and JSON output

The cooldown rule in v0.1 is based on the last completed attempt regardless of exit status.

## 3. Architecture

### 3.1 State Model

- one SQLite database
- one `runs` table keyed by `name`
- each row stores:
  - `name`
  - `started_at`
  - `finished_at`
  - `exit_code`
  - `succeeded`

### 3.2 Execution Model

1. resolve DB path
2. read the most recent row for `name`
3. compare `finished_at` to `now`
4. if the remaining cooldown is greater than zero:
   - print skip result
   - exit `0`
5. otherwise:
   - execute the provided command directly
   - record the completed run
   - return the child exit code

### 3.3 Security Boundary

- `cooldown-guard` does not interpolate user input through an internal shell
- it executes the exact program and arguments supplied by the caller
- SQLite is local-only state, not a network service
- no internal Greyforge paths, recipe names, or host identifiers are carried into the public repo

## 4. Public CLI

```bash
cooldown-guard run --name backup --min-interval 30m -- ./backup.sh
cooldown-guard status --name backup --min-interval 30m
cooldown-guard --json status --name backup --min-interval 30m
cooldown-guard clear --name backup
```

## 5. Release Surface

- `README.md`
- `STARTHERE.md`
- `scripts/setup.sh`
- GitHub Actions CI
- integration tests for `run`, `status`, and `clear`

## 6. Deferred Work

Deliberately not in v0.1:

- success-only cooldown policies
- labels/tags per key
- pruning and retention
- subcommands for listing all tracked keys
- shell-completion generation
