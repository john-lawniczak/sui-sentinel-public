# Task O01 — monitoring policy schema and boundary-clock validation

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: O01 (temp.md section 11, depends on F02)
- **Status**: DONE (code/validation scope only - see Limitations for what
  is explicitly blocked and why)
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `edcc15d` (F04, branch `production-readiness-plan`)
- **Dependencies**: F02 (DONE)
- **Files**: `src/config.rs` (unchanged, read-only reference pattern),
  `src/monitoring.rs` (new), `src/lib.rs`, `deploy/config/monitoring.toml`
  (new)

## Scope taken from temp.md section 11 row O01

> 1. Benchmark configured capture scope/provider. 2. Set `T`, `D`,
> freshness/coverage limits and budgets. 3. Validate startup config and
> boundary clocks. Stop live launch if quotas/budget/access are missing;
> continue fixture work.

## What was implemented

`src/monitoring.rs` (new module, registered in `src/lib.rs`) defines
`monitoring::Policy`, following the same `builtin()`/`load()`/`parse()`/
`validate()` shape already used by `analysis::stress::Policy` and
`config::Config`:

- `Policy { schema_version, capture: Capture { interval_seconds,
  deadline_seconds } }` — `interval_seconds` is `T`, `deadline_seconds` is
  `D` from temp.md section 8's degradation-contract table.
- `validate()` rejects an unsupported schema version, a zero or
  unreasonably large interval (capped at 24h), and a deadline that exceeds
  the interval (a capture that is allowed to take longer than its own
  scheduling period cannot describe a meaningful cadence).
- `snapshot_age_status(age_seconds) -> Freshness` implements exactly the
  "Scheduled snapshot age" row of temp.md's table: `Fresh` at or below
  `T + D`, `Warn` above `T + D` and at or below `2T + D`, `Critical` above
  `2T + D`. The function takes an already-computed age in seconds; it does
  not read the system clock itself, so every boundary is deterministic and
  testable without mocking time.
- `deploy/config/monitoring.toml` holds the schema's one section
  (`[capture]`) with `interval_seconds = 300`, `deadline_seconds = 60`.

### Why only one row of the degradation-contract table

temp.md section 8 lists six signals (scheduled snapshot age, live
publication lag, accepted price age, valuation age, coverage, unknown
deployment/schema). Only "scheduled snapshot age" is implemented here.
The other five each require state this repository does not have yet:

- **Live publication lag** needs a running publication/export loop with a
  tracked last-successful-publish timestamp (task O03).
- **Accepted price age** and **valuation age** need per-reserve/per-position
  tracking of when a price or valuation was last accepted — there is no
  fresh-valuation pipeline yet (tasks S01+/V01+); F04 (this branch,
  previous commit) recorded that fresh valuation itself remains BLOCKED.
- **Coverage** needs a per-capability completeness signal distinct from
  the existing per-market `obligations_exhausted`/`owner_caps_exhausted`
  flags (F01, this branch) — a cross-capability coverage policy is request-
  time/query-service work (task S04+).
- **Unknown required deployment/schema** needs an adapter registry of
  supported package/schema versions (tasks S01, P01+) to compare against.

Adding policy fields or enum variants for signals nothing evaluates yet
would be unused surface area pretending to be a contract. Each is deferred
to the task row that actually produces the input it needs.

## Acceptance checks

New unit tests in `src/monitoring.rs`:

1. `builtin_policy_is_valid` — the shipped `deploy/config/monitoring.toml`
   parses and validates.
2. `rejects_invalid_schema_interval_or_deadline` — an unsupported schema
   version, a zero interval, an interval above the 24h cap, and a deadline
   exceeding the interval are all rejected.
3. `classifies_exact_warn_and_critical_boundaries` — with `T=300, D=60`
   (`warn_at=360`, `critical_at=660`): age `0` and `360` are `Fresh`; `361`
   and `660` are `Warn`; `661` and `u64::MAX` are `Critical`. This is the
   "test every boundary with a controlled clock" requirement from temp.md
   section 8 — every input here is a literal integer, not a real clock
   read, so the boundaries are exact and reproducible.

## Commands run

```
cargo test --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
```

All passed.

## Limitations / explicitly blocked

- **Step 1, "benchmark configured capture scope/provider," was not done
  and cannot be done in this session.** It requires live mainnet GraphQL
  provider access, a configured budget, and measuring real capture p95
  latency against the configured scope — none of which this agent has
  authorization or connectivity for. `deploy/config/monitoring.toml`'s
  `interval_seconds = 300` / `deadline_seconds = 60` are **explicitly
  unbenchmarked placeholders**, flagged as such in the file's own header
  comment, not a result of the required measurement. Per temp.md section 0
  item 7 and section 8's own text ("Choose interval `T` only after
  measuring capture p95 and provider limits"), this blocks treating these
  specific numbers as a validated operational policy — they exist only so
  `monitoring::Policy`'s schema and boundary arithmetic have something
  concrete to validate and test against.
- **Nothing calls `monitoring::Policy` outside its own tests yet.** There
  is no capture scheduler (task O02) or publication loop (task O03) to
  evaluate this policy against a real snapshot's age. Wiring it into a
  live `readyz`/status endpoint is explicitly part of those later tasks,
  not this one — O01's row only asks for the policy schema, its
  validation, and its boundary arithmetic, which are now in place for O02
  to consume.
- Per temp.md section 0 item 7, this is exactly the "continue independent
  tasks" path: the schema/validation work proceeds now; the benchmark and
  the "stop live launch if quotas/budget/access are missing" behavior
  (which presupposes an actual live-launch code path that does not exist
  yet) are deferred to when a live deployment, provider budget, and O02's
  scheduler exist.
