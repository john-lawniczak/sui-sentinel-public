# Task U01 — exporter self-health metrics and the Monitor Operations row

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: U01 (temp.md section 11, depends on F01)
- **Status**: DONE (dashboard generator + metrics scope; see Limitations
  for the parts that need a running stack or a future task's infra)
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `adaf6e1` (O03, branch `production-readiness-plan`)
- **Dependencies**: F01 (DONE)
- **Files**: `deploy/grafana/generate.py`, `deploy/grafana/dashboards/snapshot.json`,
  `docs/dashboard.md`, `src/export/registry/{mod,operational}.rs`

## Scope taken from temp.md section 11 row U01 and section 7's table

> 1. Define view/state/label matrix and query contracts. 2. Implement
> overview/operations layouts and readable identities. 3. Preserve cached
> warnings, distinct unknown states, and scope labels. Use fixtures for
> every state.

Section 7's Grafana view table lists "Monitor operations" as one of four
required views, answering *"Is Sentinel functioning?"* via "per-stage lag,
queue age, provider throttling, database health, last successful
capture/publication, and alert delivery failures."

## What's realistically implementable right now

Most of section 7's listed operations signals need infrastructure that
does not exist yet on this branch: there is no queue (no broker/bounded
queue component built), no database (O04), no provider-throttling
instrumentation in the GraphQL transport, and no alert/incident pipeline
(O04) to report delivery failures from. Adding dashboard panels that query
nonexistent metrics would either silently show "Unknown" forever (not
useful - indistinguishable from "not implemented yet") or, worse, invite
someone to believe the row is wired up.

What *is* real and available after O02/O03: the exporter's own generation
state - whether it has ever loaded a generation, how old the current one
is, and whether the last reload attempt succeeded. That is a genuine,
presently-true answer to "is Sentinel functioning," scoped honestly to what
this branch actually built, rather than a placeholder for signals that
don't exist yet.

## What was implemented

### New metrics (`src/export/registry/operational.rs`, wired from `mod.rs`)

Four new Prometheus metric families, emitted by `export::serve_dir`'s
`/metrics` endpoint only (not by the older single-file `serve`, which has
no reload/registry concept to report on):

- `sui_sentinel_exporter_ready` (0/1) - has any generation ever loaded.
- `sui_sentinel_exporter_generation_loaded_at_seconds` - unix time the
  current generation loaded; the sample is **absent** (not zero) until the
  first load.
- `sui_sentinel_exporter_last_reload_attempt_seconds` - unix time of the
  most recent reload attempt (success, no-op, or failure); absent before
  the first attempt.
- `sui_sentinel_exporter_last_reload_failed` (0/1) - whether that most
  recent attempt failed; absent before the first attempt.

These intentionally use "absent sample" rather than a sentinel value for
the not-yet-observed case, matching temp.md section 2's "Unknown is never
equivalent to safe or zero" and the existing codebase convention (e.g.
`noValue: "Unknown"` mappings already used throughout `generate.py`).

### New dashboard row (`deploy/grafana/generate.py`)

A new `operations(y)` helper adds four stat panels under a "Monitor
operations" row: **Exporter ready**, **Current generation age**, **Last
reload attempt age**, **Last reload attempt** (OK/Failed). Color/threshold
conventions match the existing "SNAPSHOT AGE" panel (amber at 1h, red at
24h) and the existing `mapped()`/`thresholds()` helpers - no new visual
language introduced. These are process-wide metrics (no `market` label),
so they deliberately do not use the `$market` template filter that the
report-content panels use.

### Docs (`docs/dashboard.md`)

Added a paragraph explaining the new row's scope and, explicitly, that it
will read **Unknown** under the currently-deployed `deploy/compose.yaml`
(which still runs `serve --report`, not `serve-dir` - see O03's task
record for why that switch is deferred) - this is documented as the
*correct* state for metrics a one-shot `serve` process never emits, not a
defect to chase.

## Acceptance checks

- `src/export/registry/operational.rs` unit tests (pre-existing from O03,
  retained): `omits_samples_before_any_reload_attempt` and
  `reports_readiness_and_last_reload_outcome_independently` cover every
  state these metrics can be in - before any attempt, after a successful
  load, and after a load followed by a failed reload (readiness must not
  regress).
- `python3 deploy/grafana/generate.py --check` confirms the committed
  dashboard JSON matches the generator source.
- `python3 .github/scripts/check_file_sizes.py` - all files within the
  400-line cap.
- Full Rust test suite (`cargo test --locked --all-targets`), `cargo fmt
  --all -- --check`, and `cargo clippy --locked --all-targets -- -D
  warnings` all pass.

### Unrelated fix found and corrected in this task: flaky scratch-directory naming

While re-running the full suite repeatedly to confirm this task's own
changes were stable, four pre-existing test helpers (`storage::tests::scratch`,
`runtime::tests::scratch_dir`, `runtime::lock::tests::scratch_path`, and
the new `export::registry::tests::scratch`) named their temp directories
using only a nanosecond timestamp, with no per-process sequence counter.
Under `cargo test`'s default thread-parallel execution, two test threads
can observe the same clock tick on a coarser-than-nanosecond system clock,
producing a directory-name collision and an intermittent, hard-to-reproduce
test failure. Fixed by adding a `static AtomicU64` sequence counter to each
helper (matching the pattern `tests/snapshot_report.rs::Scratch` and
`tests/cli.rs`'s new O02 test already used). Confirmed stable across 8+
repeated full-suite runs after the fix, where it had failed within the
first 1-3 runs before.

## Commands run

```
cargo test --locked --all-targets   # x3, plus 8x --lib only, to confirm the flake fix
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
```

All passed.

## Limitations / explicitly deferred

- **`deploy/grafana/check.py` (the running-stack query reconciliation) was
  not run.** It requires a live Grafana + Prometheus stack reachable at
  `localhost:3000`, which this sandboxed session does not have. The new
  panel's PromQL expressions were checked by hand against the exact metric
  names `operational::render` emits (both live in the same commit, so a
  typo would be a same-PR regression, not a silent drift) but this is not
  a substitute for the actual reconciliation check.
- **No UX gate.** Temp.md's UX gate (three operators, timed tasks,
  screenshots at two viewports) is explicit acceptance evidence for U02,
  not U01, and requires human operators and a rendered browser session
  this agent does not have.
- **The row reads Unknown under the current deployment**, as stated above
  and in `docs/dashboard.md` - this is accurate, not a known-but-unfixed
  bug. It becomes populated once `deploy/compose.yaml` is switched to
  `serve-dir` (O03's deferred item) and that switch is validated against a
  running stack.
- **Per-stage lag, queue age, provider throttling, database health, and
  alert delivery failures remain absent from this row**, as explained
  above - each needs infrastructure (O02's eventual scheduler loop metrics,
  a queue component, GraphQL throttling instrumentation, O04's database
  and incident pipeline) that doesn't exist on this branch yet. Extending
  this row is natural follow-up work once those tasks ship real metrics to
  show, not a gap in this task's own scope.
