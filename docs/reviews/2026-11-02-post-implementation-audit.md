# Post-implementation audit — F01 through S01 (branch `production-readiness-plan`)

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Scope**: every commit made today on this branch (`0808735`..`ab8f5fa`:
  F01-F04, O01-O04, U01, S01) - roughly 5,600 added lines across the
  runtime scheduler, exporter reload path, Postgres diagnosis storage, and
  the Suilend adapter extraction.
- **Method**: re-read every changed file with fresh skepticism (not the
  implementer's assumptions), traced concurrency/ordering across process
  boundaries (lock acquisition vs. release vs. reacquisition; reload vs.
  concurrent HTTP reads; capture outcome vs. repeated scheduler invocations),
  and verified every fix against a real local Postgres 17 instance
  (Homebrew `initdb`/`pg_ctl`, no Docker), not just the DB-skip path CI
  actually exercises.
- **Outcome**: 2 real bugs found and fixed, both with regression tests
  that fail without the fix (verified by temporarily reintroducing each
  bug and confirming the new test catches it); 1 misleading-but-not-wrong
  error message fixed with a regression test; remaining areas reviewed
  with no further issues found at this effort level.

## Findings

### 1. [CONFIRMED] `HostLock::drop` unlinking the lock file reintroduces the exact race the lock exists to prevent

- **File**: `src/runtime/lock.rs`
- **What was wrong**: `Drop for HostLock` called `fs::remove_file(&self.path)` after unlocking. `flock`-style advisory locks are bound to an **inode**, not a path. If process A drops its lock (unlock, then unlink) while process B is mid-`open()` on the same path, B can end up locking a *freshly recreated* inode at that path - one process C opening the path later creates yet another new inode. Each "holder" then locks a *different* inode at the same path, so none of them actually exclude each other: the single-host lock silently stops providing mutual exclusion.
- **Why it matters**: this is the one mechanism `capture-once` relies on to guarantee "overlapping runs cannot both publish" (temp.md section 4). A broken lock means two scheduler-triggered runs close in time could both write into `<out>/current` and both believe they're the sole writer - exactly the failure mode the lock was built to close off.
- **Impact**: low probability (needs a narrow timing window between two closely-spaced scheduler invocations) but high severity (defeats the entire purpose of the lock when it happens) - the kind of bug that passes every normal test run and then causes a confusing double-publish incident in production months later.
- **Fix**: removed the `remove_file` call entirely. The lock file is now a permanent artifact, like a `Cargo.lock` - every acquirer opens and locks the *same* inode for the file's whole lifetime, which is what makes `flock` exclusion correct. Added
  `runtime::lock::tests::dropping_the_holder_does_not_unlink_the_lock_file`
  asserting the file survives a drop.

### 2. [CONFIRMED] Diagnosis outbox dedup_id collapsed repeated real incidents into silence after the second occurrence

- **File**: `src/diagnosis/rules.rs`
- **What was wrong**: `apply_capture_outcome`'s dedup_id was
  `format!("{key}:{transition}:{:?}", run.error_category)`, and
  `apply_snapshot_freshness`'s was `format!("{key}:{transition}:{severity}")`.
  Both used a value that **repeats across every call while the underlying
  condition is unchanged** (the same error category keeps failing; the
  freshness check stays at the same severity). Since
  `enqueue_notification` dedups on exactly this string via
  `ON CONFLICT (dedup_id) DO NOTHING`, the **third and every later**
  occurrence of the same ongoing failure produced a dedup_id identical to
  the second occurrence's, and the enqueue silently became a no-op.
- **Why it matters**: this is a notification-delivery bug, not a
  storage-correctness bug - the incident row itself stayed correctly
  updated, but the outbox (the only path to an actual human notification,
  once a sender exists) would go silent after the second update for any
  persistent failure. A capture job stuck failing the same way for a week
  would have produced exactly two outbox entries total, no matter how
  many scheduler ticks ran in between.
- **Impact**: high - this is precisely the "a failure that keeps happening
  stops generating alerts after the second time" failure mode that makes
  monitoring silently useless, and it would not have been caught by the
  existing tests (`capture_failure_opens_then_resolves_across_runs` only
  exercised exactly one failure followed by one resolution - never a third
  occurrence).
- **Fix**: `apply_capture_outcome`'s dedup_id now uses `run.started_at`
  (unique per run) instead of the error category.
  `apply_snapshot_freshness`'s now uses `age_seconds` (which increases
  monotonically while unresolved) instead of severity - matching what the
  "resolved" transition in both functions was already doing correctly.
  Added two regression tests
  (`each_repeated_failure_with_the_same_error_category_gets_its_own_notification`,
  `each_freshness_check_at_a_growing_age_gets_its_own_notification`), each
  driving 3 occurrences and asserting 3 distinct outbox entries. **Verified
  the fix is real, not just a plausible-looking test**: temporarily
  reverted the two dedup_id lines to their buggy form and reran both new
  tests against a live Postgres - both failed with `left: 2, right: 3`,
  exactly the predicted collapse; restored the fix and reran - both pass.

### 3. [FIXED] Misleading "predates checksumming" warning for a report directory that doesn't exist at all

- **File**: `src/export/mod.rs`
- **What was wrong**: `load_report`'s checksum-verification fallback
  treated "checksums.json is missing" as a single case, printing "report
  predates artifact checksumming; serving without integrity verification"
  regardless of *why* `verify_checksums` failed. A `current` pointer
  naming a generation that was **never published at all** (a typo, a
  pointer written before the generation finished, a deleted generation)
  hits this same branch and gets the same reassuring-sounding message,
  even though nothing is actually being "served" - the subsequent
  `File::open` fails immediately after.
- **Why it matters**: readability/operational-accuracy issue, not a
  correctness bug (the request still correctly fails either way) - but a
  misleading log line during an actual incident (bad pointer, failed
  reload) wastes an operator's time chasing "why does this old report
  have no checksums" instead of "why does this pointer name a generation
  that doesn't exist."
- **Fix**: split the fallback into two arms - `directory.exists()` (real
  pre-checksum report: warn and continue, unchanged behavior) vs. does
  not (hard error naming the actual problem: "report directory ... does
  not exist"). Added two tests:
  `missing_report_directory_is_distinguished_from_a_pre_checksum_report`
  and `a_report_directory_with_no_checksums_file_warns_but_still_loads`,
  the first of which asserts the misleading phrase is specifically absent.

## Areas reviewed with no issues found

- **F01 ownership attribution** (`analysis/report/mod.rs`): re-traced the
  `scan_exhaustive`/`owner`/`ownership_status` branching by hand against
  all four cap-count × exhaustiveness combinations; matches the intended
  truth table and the existing regression tests cover every branch.
- **F02 checksums/provenance**: `write_pointer`'s rename-based atomicity,
  `verify_checksums`'s exact-set comparison, and the ordering guarantee
  that `checksums.json` is always written after `report.json`/`positions.ndjson`
  are fsynced (so a concurrent reader can never observe report.json
  without a matching checksum manifest) - all hold up under inspection.
- **O03 registry reload**: confirmed the candidate `Presentation` is
  fully built *before* the write lock is acquired (true atomic swap, no
  partially-applied state visible to a concurrent reader); confirmed a
  persistently bad pointer is retried every poll tick rather than only
  once (correct, if quietly repeating file I/O - acceptable at O01's
  placeholder poll cadence).
- **O04 SQL**: the `ON CONFLICT (dedup_key) WHERE resolved_at IS NULL DO
  UPDATE` upsert and `ON CONFLICT (dedup_id) DO NOTHING RETURNING id`
  dedup pattern were checked against Postgres's actual documented
  semantics and, more importantly, against a real running Postgres
  instance for every test in `incident.rs`/`outbox.rs`/`rules.rs` - not
  just reasoned about. All parameters are bound (`$1`, `$2`, ...), no
  string-interpolated SQL anywhere - no injection surface.
- **S01 adapter extraction**: confirmed every moved function
  (`market_type_filter`/`market_type_argument`/`obligation_type`/`cap_type`)
  is a byte-for-byte relocation of the pre-existing inline logic, and that
  the deterministic-replay regression in `tests/snapshot_report.rs`
  required no changes to keep passing.
- **Panic surface**: grepped every non-test `.unwrap()`/`.expect()` across
  the new modules; the only ones outside test code are `write!`/`writeln!`
  to an in-memory `String` (cannot fail) and `RwLock::read/write().expect("poisoned")`
  (standard, already-established pattern elsewhere in this codebase - only
  reachable after an unrelated panic already occurred while holding the
  lock).

## Ship rating: 8/10

Both fixed bugs were real and would have caused production-visible
problems (a silently-ineffective lock under rare timing, and silently
dead-after-the-second-time alerting - precisely the "monitoring you can't
trust" failure mode this whole plan exists to prevent). Both are now
fixed with tests that were verified to fail without the fix, against a
real database, not just plausible-sounding assertions. The two points
below the ceiling: (1) this audit covered the files most central to
today's work at reasonable depth, not an exhaustive line-by-line pass over
every file touched since the plan started (O01/O02 from earlier in this
session were reviewed less intensively than O04/S01's newer, riskier
code); (2) the dedup_id design (now fixed) suggests notification-path
logic deserves a second, independent look once a real sender exists and
genuinely-concurrent/retried calls become possible.

## Top remaining risks

1. **No CI coverage for the diagnosis module's actual logic.** Every
   `src/diagnosis/` test gates on `SENTINEL_TEST_DATABASE_URL`, which CI
   never sets - so `cargo test` passing in CI proves the module *compiles*
   and *skips cleanly*, not that its SQL is correct. This audit's
   verification against a real local Postgres is a one-time, manual check
   recorded in this document and in O04's task record; it does not
   persist as a standing guarantee for the next change to this code.
2. **`try_reload`'s read-check-then-write pattern isn't safe against truly
   concurrent callers of itself** (two threads calling `try_reload()` on
   the same `Registry` at once could both decide to load the same
   candidate, doing redundant work before one write wins harmlessly). Not
   reachable in the shipped code (the poller and the initial load never
   overlap), but worth a note for whoever next touches `serve_dir`'s
   startup sequence.
3. **The dedup_id fix is correct for the two rules that exist today, but
   the *pattern* - "pick something that varies per real occurrence" - is
   easy to get wrong again** the next time a rule is added, since nothing
   enforces it structurally (no shared helper, no test that would fail
   for a *new* rule with the same mistake).

## Top 3 highest-leverage improvements

1. **Add a Postgres service container to CI** (`.github/workflows/ci.yml`)
   and set `SENTINEL_TEST_DATABASE_URL` there, so the 11 diagnosis tests
   actually run on every PR instead of relying on a human running them
   locally before merge, as this audit had to do.
2. **Extract a small shared helper for "build a dedup_id from an event
   that must vary per real occurrence"** (e.g. requiring a
   caller-supplied "occurrence" value, like `run.started_at` or
   `age_seconds`, as a mandatory parameter rather than letting each rule
   improvise its own string) so the class of bug fixed in this audit
   cannot recur silently when a third rule is added later.
3. **Write an explicit concurrency test for `Registry::try_reload`** (two
   tasks calling it concurrently against the same candidate) to turn "not
   reachable in the shipped code today" into "verified safe even if it
   becomes reachable," since `serve_dir`'s startup sequence is exactly the
   kind of code that tends to get refactored without re-deriving every
   invariant from scratch.

## Summary of what changed

- `src/runtime/lock.rs`: removed the lock file's self-deletion on drop
  (the actual race fix); added a regression test; `path` field removed
  since it's no longer needed.
- `src/diagnosis/rules.rs`: fixed both rules' outbox `dedup_id`
  construction to vary per real occurrence (`run.started_at` /
  `age_seconds`) instead of per condition-type (`error_category` /
  `severity`); added 2 regression tests, each verified to fail against
  the pre-fix code on a live database.
- `src/export/mod.rs`: distinguished "report directory doesn't exist" from
  "report predates checksumming" in `load_report`'s error/warning path;
  added 2 regression tests.
- All fixes verified with `cargo test --locked --all-targets` (with and
  without a live test database), `cargo fmt --all -- --check`,
  `cargo clippy --locked --all-targets -- -D warnings`,
  `python3 .github/scripts/check_file_sizes.py`, and
  `python3 deploy/grafana/generate.py --check`.
