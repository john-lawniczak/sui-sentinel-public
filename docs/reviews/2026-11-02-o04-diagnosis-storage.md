# Task O04 — incident/outbox storage and two observational rules

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: O04 (temp.md section 11, depends on O03)
- **Status**: DONE (storage substrate + two observational rules; see
  Limitations for what's explicitly out of this row's scope)
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `0c9b559` (U01, branch `production-readiness-plan`)
- **Dependencies**: O03 (DONE)
- **Files**: `migrations/0001_incidents_outbox.sql` (new), `src/diagnosis/{mod,incident,outbox,rules}.rs`
  (new), `src/lib.rs`, `src/main.rs`, `Cargo.toml`, `README.md`

## Scope taken from temp.md section 11 row O04

> New PostgreSQL incident/outbox storage and `src/diagnosis/`, exporter.
> 1. Define incident/rule IDs and migrations. 2. Persist
> collection/freshness/coverage transitions and outbox. 3. Add bounded
> incident/evidence reads, deduplicated delivery and restart recovery. S02
> extends this database; no fresh HF required.

## What was implemented

### Schema (`migrations/0001_incidents_outbox.sql`)

Two tables, applied via `sqlx::migrate!` (embedded in the binary at compile
time, so no `migrations/` directory needs to exist alongside the deployed
binary):

- `incidents` — `dedup_key` identifies *what* the incident is about (stable
  across its whole lifecycle); `state` is `opened`/`updated`/`acknowledged`/`resolved`
  (temp.md section 7's exact list). A **partial unique index** on
  `(dedup_key) WHERE resolved_at IS NULL` enforces at most one *open*
  incident per key while letting the same key be reused by a later,
  separate incident once the first resolves - full history is retained,
  never overwritten.
- `outbox` — one row per notification, with a `UNIQUE dedup_id` so a
  retried enqueue of the same logical transition cannot insert twice.
  `delivered_at`/`attempts`/`last_error` track delivery state for a future
  sender to drain.

### Storage API (`src/diagnosis/{incident,outbox}.rs`)

- `Store::open_or_update_incident(candidate) -> (Incident, bool)` — an
  `INSERT ... ON CONFLICT (dedup_key) WHERE resolved_at IS NULL DO UPDATE`
  upsert; the `bool` tells the caller whether this opened a new incident or
  updated the one already open, using Postgres's `xmax`-free approach of
  comparing the returned `state` (`'opened'` vs the `DO UPDATE` branch's
  `'updated'`) rather than a separate round-trip.
- `Store::resolve_incident(dedup_key) -> Option<Incident>` — `None` when
  nothing was open (not an error: a rule resolving an already-resolved or
  never-opened condition is the normal steady state).
- `Store::list_incidents(limit)` / `Store::pending_notifications(limit)` —
  both clamp `limit` to `1..=500` regardless of what the caller asks for,
  so nothing can accidentally request an unbounded read (temp.md: "bounded
  incident/evidence reads").
- `Store::enqueue_notification(incident_id, dedup_id, payload) -> bool` —
  the `bool` reports whether this call actually inserted (vs. a dedup
  no-op), backed by `INSERT ... ON CONFLICT (dedup_id) DO NOTHING RETURNING id`.
- `Store::mark_delivered` / `Store::record_delivery_attempt_failed` — the
  write side a future sender would call; no sender is implemented (see
  Limitations).

### Two observational rules (`src/diagnosis/rules.rs`)

Exactly two rules, both needing no fresh valuation (temp.md's own note on
this row: "no fresh HF required"):

- `apply_capture_outcome(store, scope, run: &runtime::RunRecord)` —
  `Failed` opens/updates a `capture_failure` incident; `Published` resolves
  it; `Skipped` (another attempt held the lock) does nothing, so a benign
  lock-contention skip never resolves a real failure still open from an
  earlier attempt.
- `apply_snapshot_freshness(store, scope, age_seconds, freshness: monitoring::Freshness)`
  — `Warn`/`Critical` (from O01's `monitoring::Policy::snapshot_age_status`)
  open/update a `snapshot_freshness` incident at `medium`/`high` severity;
  `Fresh` resolves it.

Both are pure translations from an already-computed signal to a store
call - no new signal computation, no hysteresis/debounce timers (temp.md
section 7 lists those as a separate requirement; see Limitations).

### Wiring (`src/main.rs`)

`capture-once` gained `--scope` (defaults to `--out`'s directory name) and
`--diagnosis-database-url` (or `DIAGNOSIS_DATABASE_URL` env, via clap's
`env` feature). When set, after a run completes it connects, migrates, and
calls `apply_capture_outcome`. **Entirely optional**: with no URL,
`capture-once` behaves exactly as it did before this task (verified by
running it with no diagnosis flag and confirming identical output to
O02/O03's existing behavior). A database connection or migration failure
only prints a warning and never changes the run's own exit status -
temp.md section 4's "keep collector failure alerts independent of a
working report-export path" generalizes here to "a broken diagnosis
database must not break capture," which this implements by construction
(the `Err` from the diagnosis path is caught and logged, not propagated).

## Acceptance checks

All `src/diagnosis/` tests are gated on the `SENTINEL_TEST_DATABASE_URL`
environment variable (see `test_support::store`): unset, they return early
and report as passed-but-skipped, so `cargo test --locked --all-targets`
stays green with no database configured (confirmed against this
repository's actual CI, which sets no such variable). This session
additionally verified the real logic by standing up a local, throwaway
Postgres 17 cluster (Homebrew's `initdb`/`pg_ctl`, Unix-socket only, no
Docker) and running the full suite against it:

1. `incident::tests::opening_twice_updates_in_place_instead_of_duplicating`
   — idempotent upsert; second call updates the same row, not a new one.
2. `incident::tests::resolving_an_incident_allows_a_fresh_one_to_open_for_the_same_key`
   — the partial unique index's exact purpose: resolve, confirm a second
   resolve is a no-op, then confirm a brand new incident can open for the
   same `dedup_key`.
3. `incident::tests::an_incident_persists_across_a_fresh_store_connection`
   — **restart recovery**: a second `Store::connect` (standing in for a
   restarted process) sees exactly what the first connection wrote.
4. `incident::tests::list_incidents_is_bounded_and_most_recent_first` —
   `limit=0` still returns rows (clamped to minimum 1); `limit=100_000`
   returns at most 500.
5. `outbox::tests::duplicate_dedup_id_does_not_enqueue_twice` /
   `delivery_attempts_and_delivered_state_are_tracked` — **deduplicated
   delivery**: a repeated enqueue with the same `dedup_id` is a no-op;
   attempts/errors/delivered-state all persist and `pending_notifications`
   stops returning a delivered entry.
6. `rules::tests::capture_failure_opens_then_resolves_across_runs` /
   `a_skipped_run_neither_opens_nor_resolves_anything` /
   `snapshot_freshness_warn_then_fresh_resolves` — the two rules' full
   open→resolve lifecycle, and the explicit non-effect of a skipped run.

**Manual end-to-end verification** (not an automated test, recorded here
per temp.md section 0 item 6's "record commands, results"): ran
`capture-once --diagnosis-database-url <local test db> --scope cli-smoke-test`
against an unreachable `SUI_GRAPHQL_URL` (guaranteeing a fast, deterministic
`capture_error` failure with no real network access needed), then queried
the database directly and confirmed exactly one `capture_failure` incident
(`state=opened`, `severity=high`) and one corresponding outbox row. Ran
`cargo test --locked --all-targets` three times with the test database
configured (47→48 lib tests, all passing) and confirmed `cargo fmt --all
-- --check`, `cargo clippy --locked --all-targets -- -D warnings`,
`python3 .github/scripts/check_file_sizes.py`, and
`python3 deploy/grafana/generate.py --check` all still pass with the
database both configured and unconfigured.

## Commands run

```
cargo build --locked
cargo test --locked --all-targets                      # without a database (CI's actual mode)
SENTINEL_TEST_DATABASE_URL=... cargo test --locked --all-targets   # x3, with a real local Postgres
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
# manual: capture-once against an unreachable GraphQL endpoint + a local
# Postgres, then psql to confirm the incident/outbox rows directly.
```

All passed.

## Limitations / explicitly deferred

- **No actual notification sender.** `pending_notifications`/`mark_delivered`/
  `record_delivery_attempt_failed` are the API a sender would call; no
  email/webhook/Slack integration exists. Building one with no configured
  destination would be exactly the kind of invented capability temp.md
  section 0 item 7 warns against ("do not invent ... approval").
- **No hysteresis/debounce windows.** Temp.md section 7: "Use rule-specific
  hysteresis and debounce windows; urgent breaches must not wait behind a
  generic delay intended for noisy advisory alerts." Both rules here act
  immediately on every call. Debounce logic needs a scheduler loop calling
  rules on a cadence to debounce against (O02's deferred always-on loop),
  which doesn't exist yet - a debounce window with nothing periodic to
  evaluate it would be inert code.
- **CI does not run a Postgres service.** `.github/workflows/ci.yml`'s
  `cargo test` job sets no `SENTINEL_TEST_DATABASE_URL`, so the database
  tests are skipped there, not run - this session verified them manually
  against a local instance instead (see above). Adding a `postgres:`
  service container to the CI workflow is a reasonable near-term follow-up
  but is an operational CI change this session cannot itself validate
  against GitHub Actions' actual runner environment, so it was not made -
  consistent with this branch's existing practice of not changing
  live/CI infrastructure without being able to observe the result (see
  O01's and O03's task records for the same reasoning).
- **No `/incidents` or `/outbox` HTTP endpoint on the exporter.** Section 7
  lists dashboards for "incident history" and a versioned query service is
  explicitly S04/A01's job (a later task building on this storage, per the
  row's own "S02 extends this database" note). Adding ad hoc HTTP exposure
  here would duplicate work that task is scoped to do properly (pagination,
  generation pinning, authorization).
- **Only two rules exist.** Temp.md section 7's full rule catalog
  ("threshold crossings, rapidly shrinking buffers, concentration changes,
  accepted-price divergence/staleness, utilization/liquidity stress,
  configuration changes, upgrades") needs fresh valuation (V01+) or state
  history (S02+/S03+) that doesn't exist on this branch. The two
  implemented here are exactly the ones this branch's own prior tasks
  (O01's freshness policy, O02's run lifecycle) already compute signals
  for.
