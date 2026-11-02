# Review of the production-readiness implementation

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

Date: 2026-11-02. Branch: `production-readiness-plan`.
Reviewed implementation range: `0808735..7025cb2` (12 commits).
The changes in this review are deliberately bounded fixes, not completion of
the remaining monitoring platform. No release, deployment, or merge to `main`
is implied by pushing this branch.

## Assessment

**Implementation quality: 6.50/10.00.** Good module separation, checked
calculations, meaningful regression tests, explicit cached-value limitations,
and actual PostgreSQL CI coverage. The largest weaknesses are incomplete
integration, acceptance criteria being deferred while tasks are labeled DONE,
and regressions outside the original tests.

Approximately **15–20% of the core plan's scope** is implemented. This is a
scope-based engineering judgment, not a measured effort forecast. More
objectively, 10 of the 31 non-optional task/release rows have implementation
or blocker documentation; only two meet functional acceptance after this
review's fixes. None of R1–R4 has its required evidence. P03–P05 are three
additional optional extensions and are not started.

### Task-by-task reconciliation

| Task | Assessment | Evidence and outstanding acceptance |
| --- | --- | --- |
| F01 | Functionally complete after review | Provisional ownership is propagated through reports/metrics; this review adds the missing non-wallet ownership case and legacy report compatibility. No fresh-risk claim is made. |
| F02 | Partial | Output checksums and some provenance exist. Raw capture files/manifest are not checksum-bound or verified on offline replay. Run ID, configuration hash, protocol/build identity beyond a constant crate version, and complete deployment/valuation context remain missing. |
| F03 | Functionally complete after review | Matched/unmatched overrides are reported and score gating persists; legacy stress results now deserialize when coverage fields are absent. Old artifacts do not gain retrospectively measured override coverage. |
| F04 | Blocked, not a completed research spike | Its record repackages dated findings, explicitly leaves owner/reviewer unassigned, performs no fresh verification, and supplies no next-review date. Recording that blocker is useful but does not satisfy the original investigation gate. |
| O01 | Partial | Pure policy parsing and age-boundary tests exist. No provider benchmark, live budget/quota contract, or runtime wiring; `monitoring::Policy` and `snapshot_age_status` have no production callers. |
| O02 | Partial | Single-run CLI, lock, directory naming, deadline wrapper, and final lifecycle record exist. No configured scheduler/missed-tick integration or persisted intermediate lifecycle transitions. The synchronous report work cannot be forcibly preempted by the async timeout. |
| O03 | Partial | Immutable generation reload and HTTP endpoints exist. Readiness means a report loaded, not fresh/complete/supported scope. Compose still serves a fixed file. This review fixes publication locking, candidate validation, and pointer error handling. |
| O04 | Partial | PostgreSQL schema, incident updates, bounded store reads, two rule functions, and outbox APIs exist. No sender, durable retry of failed lifecycle ingestion, incident HTTP API, or running freshness evaluator. Incident and outbox writes are separate transactions. |
| U01 | Partial | A Monitor Operations row and self-health metrics exist. No complete four-view workflow/state matrix. Default Compose uses `serve`, so `serve-dir` operations metrics are unavailable in that deployment. |
| S01 | Partial | Type-string helpers and a small manifest moved under `protocols/suilend`. Model validation, reserve interpretation, risk math, and ingestion configuration still depend directly on Suilend. No used capability/deployment dispatch contract exists. |

No implementation or gate evidence was found for O05, U02–U03, S02–S05,
V01–V05, P01–P02, A01–A03, or R1–R4. The incident PostgreSQL database is
not the incremental chain-state database specified in S02. A helper function
with no runtime caller is credited as a component, not as an operated feature.

## Findings fixed in this review

### 1. Container build was missing compile-time inputs

`deploy/Dockerfile` copied only the stress policy and source. New
`include_str!` and `sqlx::migrate!` calls also require the monitoring policy
and migrations. `.dockerignore` excluded both. Reconstructing the old COPY
inputs in a scratch directory and running `cargo check --locked --offline`
failed on both missing paths. Added COPY instructions and context allowlist
entries; the corrected reconstructed inputs compile.

The Docker daemon is unavailable locally, so this does not establish a
successful Linux image build or a passing Grafana/Prometheus stack. CI must
still validate those environments.

### 2. Additive report changes broke saved schema-1 reports

`provisional_owner_positions`, `matched_price_overrides`, and
`unmatched_price_overrides` were required by deserialization, despite being
absent in older saved reports. Added serde defaults and a legacy-shaped
JSON regression. Existing coverage invariants still validate the report;
defaulted fields do not refresh old valuations or establish new evidence.

Also corrected comments that called wall-clock report generation time the
valuation time. Cached position valuations may come from different times.

### 3. Publication occurred after the capture lock was released

`run_supervised_capture` returned Published before the caller wrote `current`.
The caller then waited on optional diagnosis database work before publication.
Another run could acquire the lock and publish first; the older caller could
subsequently overwrite its pointer. Pointer failures were also recorded after
a supposedly successful lifecycle/incident outcome.

Added a supervisor finalization callback that runs under the same capture
lock. It verifies the persisted checksums, completeness, and export validation
before changing `current`; only success becomes Published. Publication failure
becomes `publication_error`, and optional diagnosis work happens afterward.
Regression tests assert lock ownership during publication, correct failure
classification, and retention of the last-good pointer.

### 4. Incomplete samples could become the production generation

Successful sampling was treated as successful publication, even when the
manifest/report declared incomplete coverage. Finalization now rejects such
candidates. The raw sample and report remain in their run directory; the
complete-channel pointer does not advance. Tests also reject corrupt and
unsupported-schema candidates before publication.

### 5. Generation pointer failures were hidden or escaped the base

Missing, empty, and unreadable pointers could clear reload errors as no-ops;
absolute/parent-relative values could select reports outside the configured
base. Now only a missing pointer before the first load is a normal startup
condition. Other read failures are reported without discarding last-good data.
Generation IDs must be single directory names and canonical report paths
must remain beneath the base, including symlink resolution. Tests cover lost,
empty, directory-valued, absolute, parent-relative, and escaping-symlink cases.
Candidate rendering now occurs outside the registry write lock as documented.

### 6. Repeated freshness incidents could lose every notification

Freshness outbox IDs used scope, transition, and age, but not incident identity.
After recovery, a new incident reaching the same ages collided with the first
incident's globally unique outbox keys. A PostgreSQL regression reproduced a
second incident with zero notifications. Keys now include the incident ID;
both lifecycles retain their opening and resolution notifications.

## Important remaining findings

1. **Freshness-aware readiness is still missing.** `/readyz` can stay 200 for
   arbitrarily old data. Connect observation age and declared coverage policy
   to readiness while continuing to serve explicitly labeled historical data.
   `apply_snapshot_freshness` currently has test callers only.
2. **Incident/outbox transitions are not atomic or replay-safe as a unit.**
   An incident commit followed by enqueue failure can lose an opening or
   resolution notification. Replaying an identical failed run can change its
   transition from opened to updated and create a different dedup key. Introduce
   transactional transition/outbox APIs and a durable processed-signal ID;
   preserve original signal ordering when lifecycle records are retried.
3. **Diagnosis ingestion is best effort.** Database errors log a warning;
   lifecycle files are retained but no recovery worker replays them. Optional
   DB work no longer delays pointer publication after this fix, but can still
   delay command completion. Define explicit DB budgets and recovery semantics.
4. **Checksums do not yet protect the whole replay input.** Only derived report
   files are bound. Missing checksum manifests are accepted as legacy, even
   for a newly generated report whose checksum write failed. Introduce explicit
   legacy/new artifact requirements and bind raw capture inputs before treating
   F02 as complete. Do not mistake SHA-256 self-consistency for provider proof.
5. **Timeout and run accounting remain incomplete.** Synchronous analysis and
   final validation can exceed the async timeout budget. A hard deadline needs
   a cancellable/process-isolated design that cannot publish after cancellation.
   `RunRecord.retries` currently says zero although GraphQL already retries;
   use unavailable until actual transport counters are wired.
6. **No operated monitoring release exists yet.** No configured schedule,
   notification sender, full readiness policy, production Compose wiring,
   authenticated shared deployment, backup/restore evidence, UX trial, or
   seven-/30-day soak. New APIs and panels are useful but not an R1 release.
7. **Core expansion work remains.** Incremental checkpoint ingestion,
   durable chain projections, dependency-based fresh valuation, real adapters
   beyond Suilend, query/MCP services, and LLM investigations are unimplemented.

## Recommended next implementation order

1. Finish O01/O03: measured cadence, startup policy validation, age/coverage
   status, and consistent readiness/metrics with controlled-clock tests.
2. Finish O02/O04 together: deploy an actual schedule, persist replayable run
   outcomes, commit incident/outbox transitions atomically, and add a tested
   bounded sender with receiver-side deduplication.
3. Finish O05/U01/U02: deploy the generation-directory mode, run the container
   stack, build the operator workflow, and test degraded states and restoration.
4. Collect R1's real seven-day evidence. Do not call a code-only run a soak.
5. In parallel, assign F04's research owner/reviewer and finish S01's concrete
   adapter boundary; then proceed to S02–S05. Keep verified financial
   capabilities blocked until aligned deployment/oracle evidence exists.

## Validation

- Baseline: 86 tests passed against an isolated local PostgreSQL 17 cluster,
  including all 11 existing database tests (not their no-database early return).
- Four new regression tests were observed failing before their fixes: legacy
  report loading, lost/invalid pointer handling, pointer escape, and recurring
  freshness incident notifications. Additional tests cover publication under
  lock, failure classification, candidate validation, and non-wallet owners.
- Final verification: **93 tests passed**, including all **12 database tests**
  against the temporary PostgreSQL instance. Formatting, Clippy with warnings
  denied, maintained-file size checks, generated-dashboard verification, and
  `git diff --check` passed. No existing application database was modified.
- Container inputs were compiled in a scratch COPY reconstruction; a full
  Docker build, browser UX test, live census, independent protocol verification,
  and production soak were not performed.
