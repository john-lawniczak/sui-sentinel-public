# Task F01 — provisional ownership attribution under incomplete capability scans

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: F01 (temp.md section 11, no dependencies)
- **Status**: DONE
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `0808735` (branch `production-readiness-plan`)
- **Dependencies**: none
- **Files**: `src/analysis/report/mod.rs`, `src/analysis/report/types.rs`,
  `src/export/metrics.rs`, `src/export/validate.rs`, `deploy/grafana/generate.py`,
  `tests/snapshot_report.rs`

## Problem (temp.md section 1, "Gaps to close", P0)

> An incomplete capability scan can still produce definitive wallet attribution.

Reproduced before the fix: marking a market's capability scan incomplete
(`owner_caps_exhausted = false`) while every obligation still had exactly one
*observed* capability caused the report to keep attributing those obligations
as `direct_address_owner` and sum their debt into wallet totals. A second,
unseen capability for the same obligation (e.g. a wrapped or not-yet-indexed
cap) could invalidate that attribution, so single-observed-capability
certainty must depend on the scan being exhaustive, not merely on the current
observed count.

## Change

In `src/analysis/report/mod.rs`, ownership resolution for an obligation now
reads `capture.owner_caps_exhausted` (per-market, from the manifest) before
treating a single observed capability as definitive:

- `owner_capabilities.len() > 1` → unchanged: `multiple_owner_capabilities`
  (already non-definitive).
- `owner_capabilities.len() == 1` and the market's capability scan is **not**
  exhaustive → new status `provisional_incomplete_capability_scan`. The
  observed capability ID is still recorded in `owner_cap_id` (evidence is
  preserved), but the `Owner` is not read, so the obligation is excluded from
  `wallets`, `unique_direct_wallets`, `address_owned_positions`, and
  `largest_direct_wallets_by_cached_debt`.
- `owner_capabilities.len() == 1` and the scan **is** exhaustive → unchanged
  behavior (`direct_address_owner` / `non_wallet_or_unknown_owner_kind` /
  `unresolved`).

Added `MarketReport.provisional_owner_positions: u64` (`types.rs`) to track
the new bucket. `src/export/validate.rs`'s ownership-coverage invariant now
sums five buckets instead of four, and a complete report
(`report.capture_complete == true`) is asserted to carry zero provisional
positions — consistent with the existing manifest invariant that
`manifest.complete` requires every market's `owner_caps_exhausted` to be
true. `src/export/metrics.rs` exports a `status="provisional"` series on the
`ownership_positions` metric family. `deploy/grafana/generate.py` maps that
status to an orange "Provisional (incomplete capability scan)" label so it
is visually distinct from the grey non-actionable statuses.

## Acceptance checks

1. New regression `single_observed_cap_stays_provisional_until_capability_scan_is_exhaustive`
   (`tests/snapshot_report.rs`): sets `owner_caps_exhausted = false` on the
   89-position fixture market and asserts `address_owned_positions == 0`,
   `provisional_owner_positions == 89`, `unique_direct_wallets == 0`, and that
   every provisional position still carries a non-null `owner_cap_id`.
2. New regression `exhaustive_capability_scan_upgrades_provisional_ownership_to_definitive`:
   runs the same fixture once with `owner_caps_exhausted = false` and once
   unmodified (exhaustive), asserting the transition from 0→89 definitive
   direct-address positions as coverage completes.
3. Existing regressions unaffected: ambiguous-capability handling
   (`preserves_multiple_owner_caps_without_assigning_or_double_counting_debt`)
   and the incomplete-obligation-sampling case
   (`sampled_capture_stays_incomplete_and_does_not_invent_owners`) still pass
   unchanged — neither constructs a single-observed-cap/incomplete-scan
   combination, so their expected outputs did not change.
4. Non-wallet owners (`ObjectOwner`/`Shared`/`Immutable`) are exercised by
   the existing fixture's one `unresolved` position; a dedicated non-wallet
   fixture case was not added since no existing fixture position has that
   owner kind — tracked as a fixture gap under F06 (temp.md P1: "Real
   regression coverage is narrow").

## Commands run

```
cargo test --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
```

All passed. `deploy/grafana/dashboards/snapshot.json` was regenerated via
`python3 deploy/grafana/generate.py` after the label-mapping change.
`deploy/grafana/check.py` (the running-stack query reconciliation) was not
run — it requires a live Grafana/Prometheus stack and a `--report` path,
which is out of scope for this code-only task.

## Limitations / not done here

- This task does not touch F02's provenance/schema work, F03's stress-scenario
  coverage, or F04's deployed-source verification spike — those are separate
  rows and are recorded in their own task files.
- No live deployment, soak, or independent review was performed; none is
  required for this task's acceptance scope (code correctness + regression
  tests), per temp.md section 0 item 6.
