# Audit Remediation Evidence

Release: `0.2.0`

Date: 2026-08-28

## GF-AUD-010

- A short immediate transaction atomically creates a same-name owner-token claim with claim and lease-expiry timestamps, then commits before child process spawn.
- Finalization is a second short transaction and succeeds only for the matching, unexpired owner token.
- Expired crash claims are removed on the next claim. A replacement owner can proceed, while the stale owner cannot record a result.
- Different job names execute concurrently; same-name contenders produce one winner.
- SQLite waits are bounded to five seconds and lock errors remain visible to the caller.

## GF-AUD-031

- Run timestamps and cooldown calculations use milliseconds; accepted inputs require positive whole-millisecond precision.
- `--failure-backoff` applies to both spawn failures and nonzero exits and defaults to `--min-interval` for compatibility.
- Job names use a strict 1–128-character normalized ASCII grammar.
- Finalization retains the newest 1,000 completed attempts per job.
- Cooldown skips intentionally exit `0`; runtime and usage errors exit `2`, and executed commands return their child exit code.
- Existing v0.1 databases receive additive millisecond columns and migrate old second-precision values in place.

## Validation

- Tier 2: `cargo test`
- Static and formatting: `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`
- Packaging: `cargo package --allow-dirty`
- Patch hygiene: `git diff --check`

Tests cover 1ms, 999ms, and 1s duration calculations; unrelated and same-name contenders; crash recovery; expiry while an old owner remains alive; stale finalization; bounded database busy behavior; repeated spawn and execution failure; invalid names; retention pruning; legacy migration; and skip exit semantics.
