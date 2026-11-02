# Post-merge audit — 2026-11-02

Ship rating: **7/10 for an early source preview**. This is not approval of a
production financial-risk service. Hosted recovery, sustained load and fresh
valuation gates remain open.

## Scope and method

Updated local `main` with `git pull --ff-only origin main` to `e153865` and
preserved the existing README, contributor, security and PR-template edits.
The review scope is all 22 commits dated November 2 in America/New_York,
including merge `e153865`, beginning with `66f73f6` (base `66f73f6^`).
The aggregate change spans 166 files, 18,754 insertions and 427 deletions.

This was a risk-focused source review, not a claim of equal line-by-line depth
across generated dashboards and historical review documents. Commit groups:

| Commits | Reviewed behavior |
| --- | --- |
| `66f73f6`, `0808735`, `edcc15d` | Plans, capability boundaries and research gates |
| `2088c9a`, `a45e484`, `802f8d2` | Ownership completeness, checksums/provenance, stress coverage |
| `78882ff`, `7574859`, `adaf6e1`, `0c9b559` | Policy, supervised capture, generation loading and health |
| `2d524b2`, `ab8f5fa`, `e2eb7d2`, `7025cb2`, `8c1ca56` | Incident/outbox behavior, adapter boundaries, earlier fixes and CI |
| `f57fd3d`, `4f72f03` | Workers, checkpoint transactions, normalization, bootstrap and evaluation costs |
| `31c10d2` | Roles, delivery/administration, retention, deployment and release evidence |
| `ef0dffb`, `0e02205`, `f36d540`, `e153865` | MCP/query authorization, generations, incident cursors and merge state |

Traced publication/cancellation, replay and database transaction boundaries;
read nearby code for each finding; ran the complete suite against a fresh local
PostgreSQL 17 database. No paid provider calls, production database changes,
deployment or live Discord notifications were needed.

## Confirmed findings and fixes

### 1. High: mutable files share temporary paths

`storage::write_pointer_before` used `path.with_extension("tmp")`.
`runs/run-X.json`, `runs/run-X.progress` and `runs/run-X.applied` therefore shared
`runs/run-X.tmp`. Concurrent progress reporters also write the same destination.
One writer could truncate another's bytes, publish the wrong document, or fail
its rename after the other writer consumed the temporary file. The result can
be unreadable lifecycle evidence, interrupted captures or stalled observation.

Fix: [pointer.rs](../../src/storage/pointer.rs) creates a sibling temporary file
with the complete destination filename, process ID and atomic sequence, using
exclusive creation and collision retry. Each writer owns its file through fsync
and rename; failed writes/deadlines clean up their own temporary file. The
existing public API and directory fsync remain intact.

Regression: eight concurrent writers repeatedly replace four paths, including
same-stem/different-extension paths and shared destinations. Every observed
document must be complete and belong to that destination. The test also checks
deadline rejection preserves the old value and leaves no temporary files.

### 2. High: exiting workers can orphan descendants

`runtime::process::run_until` disarmed process-group cleanup after `child.wait()`
returned, including unsuccessful exits. If a capture supervisor crashed while a
snapshot child remained alive, the child could continue consuming resources or
writing artifacts after the supervisor released its capture lock.

Fix: [process.rs](../../src/runtime/process.rs) cleans up the owned process group
on normal and unsuccessful exits as well as timeout and cancellation. Nested
workers still inherit the outer supervisor's group.

Regression: workers exit with status 0 and 1 while a background descendant is
scheduled to write later. Neither descendant may survive. Existing timeout and
cancellation tests also pass.

### 3. Medium: failed stress calculations mutate reported coverage

`Accumulator::add` recorded matched price overrides before all scenario arithmetic
succeeded. Overflow in a later scenario left coverage mutated even though no
scenario totals or evaluated-position count were committed. Reports could claim
an override was applied when its only matching position was excluded.

Fix: [stress/mod.rs](../../src/analysis/stress/mod.rs) updates matched sets only
after every scenario and the position-count check succeed. No calculation or
policy changes.

Regression: a second scenario overflows for an asset with an explicit override;
a subsequent unrelated position succeeds. Both scenarios must still report that
override as unmatched, with only the successful position in totals.

### 4. Medium: provider error text can expose credentials

The nearby GraphQL error path interpolated the provider's full `errors` JSON
into an error chain. Providers can reflect request URLs or authentication data
in messages/extensions. These strings reach CLI logs; the previous HTTP-only
credential-redaction test did not cover HTTP 200 GraphQL errors. Large error
bodies could also produce excessive logs.

Fix: [graphql/mod.rs](../../src/ingest/graphql/mod.rs) emits a fixed diagnostic
and continues rejecting all partial data. This deliberately removes provider
error detail from routine logs. Transport counters remain available.

Regression: a mock provider reflects an API key, private URL and large debug
extension. None appears in the bounded error chain, and the request is not retried.

### 5. Medium: new contributor quickstart fails on a clean checkout

The pre-existing uncommitted README/contributor instructions used
`--out snapshots/oss-demo/analysis` without creating its parent directory.
`report` intentionally uses `create_dir` to avoid overwriting artifacts.

Fix: add `mkdir -p snapshots/oss-demo` to both examples. Keep the application's
existing output ownership semantics. Executed the documented report command
against the checked-in 90-position fixture.

## Additional implementation

- Add a separate, pinned Trivy source-secret check to the security workflow.
  A finding fails the job; scans remain local to the runner.
- Include the existing contributor/security guidance and PR template in the
  requested commits. Preserve the Apache-2.0 license and preview limitations.
- Publish a separate public source snapshot with fresh history. Export only
  reviewed repository files; exclude `.git`, ignored captures, local credentials,
  build outputs and database backups. Adjust public repository links in the export.

## Validation

- `cargo test --locked --all-targets`: **139 passed**, using a fresh PostgreSQL 17
  database through `SENTINEL_TEST_DATABASE_URL`; no database-test skip path.
- `cargo clippy --locked --all-targets -- -D warnings` and formatting checks pass.
- Repository file-size check, generated dashboard check, two Python tests,
  operator JavaScript syntax check and Fly local invariants pass.
- Applied `init-roles.sql` and `grants.sql`; `check-query-grants.sql` passes with
  the actual monitor, operator and reader identities.
- Trivy 0.76.1 reports **zero secret findings** in the prepared source snapshot.
  This checks the published tree, not the private history, and is not a guarantee
  that every possible secret format is detected.
- The new Rust tests ran natively on macOS. A fresh Linux image build, fresh
  dependency/image vulnerability scan and hosted acceptance run were not performed
  in this audit; earlier release evidence must not be relabeled as this build's.

## Remaining risks and exact follow-up

1. **Database growth is unbounded.** Operational observations insert signal
   receipts every five seconds, and active incidents generate immutable change
   entries. Filesystem retention does not archive `diagnosis_signals`,
   `incident_changes`, `chain_frames` or delivery history. Long deployments will
   accumulate storage and query/backup costs. Define a supported replay window,
   persist its rejection floor, archive evidence before deletion, and add bounded
   batched cleanup. Do not simply delete idempotency receipts: an old replay could
   otherwise be accepted again. Retention duration needs an explicit product choice.
2. **Possible retry/delivery ordering race (not reproduced here).** Administrative
   dead-letter retry locks the incident, but delivery claims lock only outbox rows.
   The retry's check for later deliveries may race with another worker claiming a
   later notification. Before claiming strict ordering under concurrent manual
   retries, add a deterministic interleaving test and share a transaction-level
   per-incident advisory lock between retry and delivery admission. The existing
   ordinary concurrent-delivery test does not establish this behavior.
3. **Qualification is incomplete.** Existing throughput evidence uses synthetic
   clock/unchanged-market updates; it does not establish sustained mixed mutation
   throughput, hosted PITR, seven-day availability or fresh valuation. Rendered
   operator/accessibility acceptance also remains absent. Run these against a
   source-pinned candidate before making production claims.

## Top three highest-leverage improvements

1. Implement bounded database archival with explicit replay/cursor floors and
   tests for reads/replays across the floor.
2. Reproduce and close the administrative retry versus delivery-claim race;
   include a role-restricted concurrency test in CI.
3. Automate a source-pinned mixed-mutation and recovery soak, including provider
   disconnects, process crashes, disk pressure and rendered operator acceptance.

Summary: fixed temporary-file races, orphaned processes, failed-calculation
coverage leakage and provider-error disclosure; repaired onboarding and added
four focused regressions plus a standing source-secret gate.
