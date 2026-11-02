# Task O02 — supervised single-run capture: lock, lifecycle, deadline

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: O02 (temp.md section 11, depends on O01)
- **Status**: DONE (single-run scope; see Limitations for the always-on
  scheduler loop this does not implement)
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `78882ff` (O01, branch `production-readiness-plan`)
- **Dependencies**: O01 (DONE)
- **Files**: `src/runtime/{mod,lock,lifecycle,schedule}.rs` (new),
  `src/lib.rs`, `src/main.rs`, `tests/cli.rs`, `README.md`, `Cargo.toml`

## Scope taken from temp.md section 11 row O02 and section 4

> 1. Add run lifecycle and single-host lock. 2. Enforce unique
> directories/deadlines and skip overdue ticks. 3. Test overlapping runs and
> crashes. Add distributed lease fencing only if multiple schedulers are
> deployed.

## What was implemented

A new `runtime` module, split into four files to stay under the 400-line
cap:

- **`runtime::lock::HostLock`** — a single-host advisory lock using
  `std::fs::File::try_lock`/`unlock` (stabilized in this toolchain's Rust
  1.89+; no external locking crate needed). `try_acquire` returns `Ok(None)`
  on contention rather than an error, so a caller treats "another run holds
  this" as a normal skip, not a failure. The lock is tied to the file
  descriptor's lifetime: dropping the holder (including process exit/crash)
  releases it, so there is no lease to expire and no fencing-token scheme
  needed for the single-host case this task is scoped to.
- **`runtime::lifecycle::{State, RunRecord}`** — the states listed in
  temp.md section 4 (`scheduled`, `collecting`, `captured`, `analyzing`,
  `validating`, `published`, `failed`), plus a `skipped` state (not in
  temp.md's list) to distinguish "another run held the lock" from an actual
  failure. `RunRecord` carries checkpoint, position/market counts,
  duration, error category/message, and a `retries` field - the last is
  always `0` today and documented as reserved rather than invented, since
  no retry loop exists yet to populate it (see Limitations).
- **`runtime::schedule`** — two pure, clock-free functions:
  `unique_run_directory(base, now)` (filesystem-safe, collision-resistant
  per-attempt directory name) and `should_skip_overdue_tick(scheduled_at,
  now, interval)` (temp.md: "skip/coalesce missed schedule ticks rather
  than accumulating jobs"). Nothing calls the latter yet - see Limitations.
- **`runtime::run_supervised_capture`** — the orchestrator: computes a
  unique output directory, tries the lock, and on success runs a
  caller-supplied async capture closure under a `tokio::time::timeout`
  deadline, recording the outcome into a `RunRecord` either way. It takes
  the capture logic as a generic closure specifically so its own tests
  never touch the network - `src/main.rs` is the only place that wires it
  to the real `ingest::snapshot::capture` + `analysis::report::generate`.
- **`Command::CaptureOnce`** (`src/main.rs`) — a new CLI command with the
  same capture knobs as `Command::Snapshot`, plus `--deadline-seconds`. It
  creates `<out>/.capture.lock` and `<out>/runs/`, runs one supervised
  attempt, prints the `RunRecord` as JSON, writes it to
  `<out>/runs/<attempt>.json` via the existing `storage::json` atomic
  publish helper, and exits nonzero only on an actual `Failed` state (not
  `Skipped`).

## Acceptance checks

**Unit tests** (`src/runtime/{lock,lifecycle,schedule,mod}.rs`, 11 new):

1. `lock::tests::second_acquirer_is_rejected_while_first_holds_the_lock` —
   overlapping-run rejection.
2. `lock::tests::dropping_the_holder_releases_the_lock_like_a_crashed_worker`
   — simulates a crash via drop; a replacement can then acquire immediately.
3. `lifecycle::tests::*` (3 tests) — success/failure transitions, duration
   recording, and that the internal `Instant` timer never leaks into the
   serialized JSON.
4. `schedule::tests::*` (3 tests) — directory uniqueness for the same
   instant, the exact boundary between "one interval late" (not yet
   overdue) and "past one interval" (overdue), and that a tick that hasn't
   arrived yet is never skipped.
5. `runtime::tests::*` (4 tests) — successful capture records counts and
   publishes; a failed capture closure is recorded as `Failed` rather than
   propagated as an `Err` from `run_supervised_capture` itself; a capture
   that exceeds the deadline is recorded `Failed` with
   `error_category = "deadline_exceeded"`; a run attempted while another
   holds the lock is recorded `Skipped`, and the capture closure
   `panic!`s if it is ever invoked in that case, enforcing in the test
   itself that no capture work ran.

**Integration tests** (`tests/cli.rs`, 2 new):

6. `capture_once_rejects_invalid_limits_before_network_access` — the same
   four invalid-argument cases as the existing `snapshot` command, now
   covering `capture-once`.
7. `capture_once_skips_without_touching_the_network_while_another_run_holds_the_lock`
   — acquires the real lock file via `runtime::lock::HostLock` in-process
   (the same lock path the binary itself will try), then spawns the actual
   `sui-sentinel capture-once` binary pointed at that directory. The
   process succeeds (exit 0) and its stdout JSON contains `"skipped"`.
   This is the test that proves lock contention is checked *before* any
   network attempt: a real capture would fail in this sandbox (no mainnet
   access), so a clean success is only possible if the capture closure was
   never invoked.

All existing regressions (lib + `tests/cli.rs` + `tests/export.rs` +
`tests/snapshot_report.rs` + `tests/stress.rs`) pass unchanged.

## Commands run

```
cargo test --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
```

All passed.

## Limitations / explicitly deferred

- **No always-running scheduler loop.** `capture-once` runs exactly one
  attempt and exits; it is meant to be invoked by an external scheduler
  (cron, a systemd timer), not to loop internally. `schedule::should_skip_overdue_tick`
  is implemented and unit-tested, but nothing calls it yet - there is no
  persistent "last scheduled tick" state to compare against without a loop
  to own it. Building that loop (and deciding whether it lives in this
  process or stays external) is explicitly deferred; temp.md's exit
  evidence for this stage ("seven days of unattended captures meet the
  agreed cadence") requires that loop to exist first, and a live
  deployment to run it against, neither of which this session has.
- **`--deadline-seconds` is a raw CLI flag, not `monitoring::Policy`-derived.**
  O01's policy models `T`/`D` for *freshness classification* (how stale is
  an already-published snapshot), not the deadline budget for a single
  in-progress attempt. Wiring `capture-once`'s deadline to
  `monitoring.toml` would conflate two different knobs; this task keeps
  them separate rather than forcing a coupling temp.md doesn't ask for.
- **`retries` is always `0`.** No retry loop exists in `ingest::graphql`'s
  transport to report attempts through; see the module doc comment in
  `src/runtime/lifecycle.rs`. If/when that transport gains retry counting,
  it should be threaded through here rather than this field being removed
  or faked.
- **No distributed lease fencing**, per temp.md's own instruction ("add
  distributed lease fencing only if multiple schedulers are deployed") -
  only one host/one scheduler is assumed. A second host pointed at the
  same output base directory is explicitly out of scope until a
  distributed scheduler is actually deployed (tracked for whenever that
  becomes true, not implemented speculatively now).
- **No disk-space preflight, retention, compression, or backup
  verification** (the remaining bullets in temp.md section 4's
  implementation list). Those depend on a real multi-run deployment
  history to size against and are deferred to the task(s) that actually
  run `capture-once` repeatedly against live infrastructure.
