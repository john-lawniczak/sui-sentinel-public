# CI Postgres service + composability pass (branch `production-readiness-plan`)

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

Follow-up to the post-implementation audit
(`2026-11-02-post-implementation-audit.md`), whose top-leverage item #1
was: the diagnosis module's tests never actually ran in CI, only when a
human remembered to run them locally against a real database first.

## 1. CI now runs the diagnosis tests for real

- **File**: `.github/workflows/ci.yml`
- **Change**: added a `postgres:17` service container to the `rust` job,
  with a standard health-check (`pg_isready`) gating the job's steps, and
  `SENTINEL_TEST_DATABASE_URL: postgres://postgres:postgres@localhost:5432/sentinel_test`
  set as a job-level env var. `cargo test --locked --all-targets` picks
  this up automatically (`diagnosis::test_support::store()` already reads
  this exact variable name) - no test code changed.
- **Verified locally, not just read for syntax**: stood up a local
  Postgres 17 cluster with password auth matching the service container's
  config exactly (`POSTGRES_PASSWORD=postgres`, same connection URL
  shape), and ran the full suite against it - all 11 diagnosis tests
  execute for real (not skipped) and pass.
- **Effect**: the 11 tests in `diagnosis::{incident,outbox,rules}` that
  previously only ran when a human set `SENTINEL_TEST_DATABASE_URL`
  locally now run on every push and PR. A future change to the
  incident/outbox SQL that breaks one of them will fail CI, not wait for
  someone to remember to test it manually (exactly the gap the audit
  flagged).

## 2. Composability: no file left doing too many unrelated things

Reviewed every file touched or created today for whether it mixes
concerns a reader would expect to find separately. Found three real
offenders (all from today's own work, not pre-existing files) and split
each along its natural seam - state/logic from HTTP wiring, and code from
its own tests, matching the one convention already established in this
codebase (`src/ingest/graphql/mod.rs` + sibling `tests.rs`, pre-existing).

| File | Before | After |
| --- | --- | --- |
| `src/export/registry/mod.rs` | 398 lines: reload state machine, HTTP route handlers, and tests all in one file | `mod.rs` (211, state machine only) + `http.rs` (99, route wiring) + `tests.rs` (98) + the pre-existing `operational.rs` (133, self-health metrics) |
| `src/diagnosis/rules.rs` | 362 lines: both rules + their tests | `rules.rs` (167, the two rule functions) + `rules/tests.rs` (194) |
| `src/diagnosis/incident.rs` | 327 lines: `Store` incident methods + their tests | `incident.rs` (209) + `incident/tests.rs` (117) |
| `src/main.rs` | 329 lines: CLI arg definitions AND every command's full handler body inline in one `match` | `main.rs` (204, CLI definitions + a thin dispatching `match`) + `src/commands.rs` (270, one function per command) |

None of these splits changed behavior - each is a pure code move (verified
by running the full test suite, including a live-database run and a
manual `capture-once` smoke test against an unreachable endpoint, both
before and after, with identical output). `main.rs`'s refactor additionally
introduces `CaptureScope`/`CaptureOnceArgs` structs so `commands::capture_once`
doesn't need an 11-positional-parameter function signature, and splits
`run_name`'s two call sites back into their original distinct failure
behaviors (the lifecycle-record file name tolerates a missing path
component with a harmless `"run"` fallback; the `current` pointer write
does not - it hard-fails, matching the pre-refactor behavior exactly,
since writing a wrong pointer is a correctness issue a log file name
isn't).

### What was deliberately left alone

- `src/analysis/report/mod.rs` (350 lines), `src/ingest/graphql/mod.rs`
  (326, already split from its own tests), `src/model.rs` (271),
  `src/analysis/stress/mod.rs` (246): all predate today's session's major
  additions and are single-purpose, cohesive modules already under the
  400-line cap with headroom. Splitting a file that isn't actually
  confusing to navigate, just because it's long, would be exactly the
  "unnecessary refactor" this review was told to avoid.
- `src/model.rs`'s small inline test module (added in S01, ~52 lines) and
  similarly-sized inline test modules elsewhere (`storage.rs`,
  `monitoring.rs`, `runtime/lifecycle.rs`) were left inline rather than
  extracted - the split-into-`tests.rs` convention is for files that are
  *large enough that the test module is itself a reason the main file is
  hard to navigate*, not a blanket rule applied regardless of size.

## Commands run

```
cargo build --locked
cargo test --locked --all-targets                      # without a database (CI's normal mode)
SENTINEL_TEST_DATABASE_URL=... cargo test --locked --all-targets   # against a real local Postgres, matching the CI service's exact connection shape
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
# manual: capture-once against an unreachable GraphQL endpoint, before and
# after the main.rs/commands.rs split, output byte-identical.
```

All passed.
