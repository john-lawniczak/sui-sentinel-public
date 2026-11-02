# Task O03 — atomic generation swap, `/livez`/`/readyz` split, directory mount

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: O03 (temp.md section 11, depends on O02)
- **Status**: DONE (binary/code scope; the Docker Compose default deployment
  switch is explicitly deferred - see Limitations)
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `7574859` (O02, branch `production-readiness-plan`)
- **Dependencies**: O02 (DONE)
- **Files**: `src/export/{mod,registry}.rs`, `src/storage.rs`,
  `src/main.rs`, `tests/export.rs`, `README.md`

## Scope taken from temp.md section 11 row O03 and section 4

> 1. Validate candidate generation. 2. Swap immutable presentation
> atomically. 3. Add liveness/readiness/status and directory mount. Pass
> concurrent-reader, invalid-reload, stale-last-good, and restart tests.

## What was implemented

### The pointer: `storage::write_pointer`

A new `storage::write_pointer(path, contents)` (`src/storage.rs`) does an
atomic write-temp-then-`rename` replace - distinct from the existing
`storage::json`, which is a one-shot completion marker that refuses to
overwrite (hard-link, `create_new`). A mutable "which generation is
current" pointer needs the opposite property: it is expected to be
overwritten every successful capture. `rename` within one filesystem is
atomic on this repository's target platforms (Unix/Linux containers), so a
concurrent reader of the pointer never observes a half-written value.

`Command::CaptureOnce` (`src/main.rs`) now writes `<out>/current` with the
successful attempt's generation ID whenever (and only when) the run reaches
`State::Published` - a failed or skipped attempt never touches the
pointer, which is how "last good generation" is preserved automatically
without any extra bookkeeping.

### The registry: `export::registry`

A new `Registry` (`src/export/registry.rs`) holds the currently-served
`Presentation` (the existing report+metrics bytes struct, now also used by
the original single-file `router`/`serve`, refactored to share it) behind a
`RwLock`, plus the current generation's ID, load time, and the last reload
attempt's outcome.

- **Validate candidate generation**: `try_reload` reads `base/current`,
  and if it names a generation different from the one already serving,
  calls the shared `export::load_report` helper (checksum-verified, size-
  bounded, JSON-parsed - the same logic `serve` already used, extracted
  so both paths apply identical integrity checks) and then
  `Presentation::render`, which runs the existing `validate::report` checks
  as part of building the Prometheus text output.
- **Atomic swap**: the new `Presentation` and generation metadata are built
  *before* the write lock is taken; the swap itself is a single lock
  acquisition that either fully replaces the served state or doesn't run
  at all. A request arriving mid-reload sees either the old or the new
  generation whole, never a partial one - there is no window where
  `/report` and `/metrics` could disagree with each other.
- **Keep the last good generation on failure**: any error building the
  candidate (missing pointer target, missing/corrupt report, checksum
  mismatch, schema violation) is recorded in `last_reload_error` and the
  function returns without touching `presentation`/`generation_id`. The
  previously-served generation keeps being served, unchanged.
- **`/livez` vs `/readyz` vs `/status`**: `/livez` is unconditional (process
  is responding). `/readyz` returns `200` once any generation has ever
  loaded, `503` otherwise - it does **not** flip to not-ready just because
  the *most recent* poll failed, matching temp.md's instruction to keep
  serving the last good evidence. `/status` always returns `200` with the
  same JSON body (`ready`, `current_generation`, `current_generation_loaded_at`,
  `last_reload_attempted_at`, `last_reload_error`) for an operator or
  dashboard that wants the detail `/readyz`'s status code alone can't carry.
- **Directory mount**: `export::serve_dir(base, listen, poll_interval)`
  takes a base *directory* (not one file path), matching temp.md's "mount a
  report directory rather than relying on replacement of a single
  bind-mounted file." `Command::ServeDir` (`src/main.rs`) exposes it.

The original `router`/`serve`/`Command::Serve` (one fixed file, explicit
restart required) are preserved unchanged in behavior - `export::mod.rs`
was refactored to extract `load_report` and `Presentation::render` as
shared helpers, but `serve`'s observable behavior (including its own
existing checksum-warning/failure logic) is identical to before this task.

## Acceptance checks

**Unit tests** (`src/export/registry.rs`, 3 new):

1. `missing_pointer_is_not_ready_but_not_an_error` — no `current` file yet
   (fresh deployment before any `capture-once`): `try_reload` returns
   `Ok(false)`, `/readyz`-equivalent state is not-ready, and no error is
   recorded (a missing pointer at cold start is the expected steady state,
   not a failure).
2. `loads_and_swaps_to_a_new_generation_keeping_the_last_good_one_on_failure`
   — the **invalid-reload** and **stale-last-good** cases together: loads
   `gen-a`; points `current` at a generation that was never published
   (`gen-missing`) and confirms the reload fails *without* losing `gen-a`
   (`current_generation` stays `"gen-a"`, `last_reload_error` becomes
   `Some`); then publishes a real `gen-b` and confirms a subsequent reload
   swaps to it cleanly.
3. `repeated_reload_of_the_same_generation_is_a_no_op` — reloading while
   already serving the current generation returns `Ok(false)` (no
   spurious swap, no wasted render work).

**Integration test** (`tests/export.rs`, 1 new):

4. `serve_dir_reloads_without_restart_and_keeps_last_good_on_a_bad_pointer`
   — the **restart** and **concurrent-reader** cases: starts a real
   `export::serve_dir` HTTP server, confirms it reaches `/readyz` without
   any restart once `gen-a` is published (covering "restart": the server
   process itself never restarts across this whole test), writes a bad
   pointer and confirms `/status`/`​/readyz` show the failure while still
   serving `gen-a` (stale-last-good, over real HTTP), then publishes
   `gen-b` and polls until the server swaps to it - all while the same
   `reqwest::Client` keeps issuing requests to the same running server
   (concurrent-reader: nothing about issuing requests during a reload ever
   errors or blocks).

All existing regressions (`tests/export.rs`'s prior 6 tests, plus the full
lib/cli/snapshot_report/stress suites) pass unchanged.

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

- **`deploy/compose.yaml` was not switched to `serve-dir`.** Temp.md's own
  instruction for this bullet is "mount a report directory rather than
  relying on replacement of a single bind-mounted file... **preserve the
  current explicit restart mechanism until safe reload exists**." Safe
  reload now exists in the binary, but switching the live Docker Compose
  deployment's default command is an operational change this session
  cannot validate: it requires actually starting the Docker stack and
  confirming Grafana renders correctly against it (temp.md section 0 item
  5: "a successful query check alone does not complete a UX task"), which
  needs a running-stack check this sandboxed session doesn't have. Rather
  than switch the default and claim it works, `compose.yaml` keeps running
  `serve --report /data/report.json` unchanged; `serve-dir` is documented
  in `README.md` as available to run manually. Switching the Compose
  default is tracked as follow-up work for whoever can run and observe the
  actual stack (this is the same category of gap as O01's unbenchmarked
  interval/deadline and F04's blocked verification - recorded rather than
  asserted).
- **No disk-based "watch" (inotify/fsnotify).** `serve_dir` polls
  `base/current` on a fixed interval (`--poll-interval-seconds`) rather
  than reacting to filesystem events. This matches temp.md's own framing
  ("poll that pointer") and avoids a platform-specific file-watching
  dependency; a poll interval of a few seconds is adequate given captures
  are not expected to land faster than that cadence (O01's placeholder
  `T = 300s`).
- **No authentication, TLS, or structured status-endpoint rate limiting.**
  Those are O05's scope (shared-service operating model), not O03's.
