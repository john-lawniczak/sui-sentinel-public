# Task F03 — publish stress scenario asset coverage

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: F03 (temp.md section 11, depends on F02)
- **Status**: DONE
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `a45e484` (F02, branch `production-readiness-plan`)
- **Dependencies**: F02 (DONE)
- **Files**: `src/analysis/stress/mod.rs`, `tests/stress.rs`,
  `docs/economic-stress.md`

## Problem (temp.md section 3 item 4, section 11 row F03)

> Fail or explicitly warn when configured scenario assets match no
> exposures. Publish matched/excluded assets and position counts. Keep
> current score gating until valuation and recovery assumptions are
> verified.

Before this change, `analysis::stress::reprice` silently fell through to
`scenario.default_price_factor` whenever an asset in a position's
collateral or debt legs was not a key in `scenario.price_factors` - correct
behavior for assets intentionally left at the default. But the reverse case
was invisible: a `price_factors` entry that never matched *any* exposure in
the whole capture (a typo'd coin type, or an asset the operator expected to
see but that is genuinely absent from this market) produced no signal
anywhere. The override was configured and silently had zero effect.

## Change

`analysis::stress::Outcome` gained two fields:

- `matched_price_overrides: BTreeSet<String>` - configured `price_factors`
  keys for that scenario that matched at least one evaluated collateral or
  debt leg.
- `unmatched_price_overrides: BTreeSet<String>` - configured keys that
  matched none.

`Accumulator` tracks a `Vec<BTreeSet<String>>` (one set per scenario,
parallel to `policy.scenarios`) populated by a new `track_matches` helper
called from `add()` for both the debt and collateral legs of every
evaluated position. `finish()` diffs each scenario's configured
`price_factors` keys against its matched set to populate both new
`Outcome` fields.

**Warn, not fail.** Per temp.md's "fail or explicitly warn," this
implementation warns (publishes the gap in the report) rather than failing
report generation. Hard-failing on an unmatched override would block the
entire report - including every other scenario and market - over what is
very often a benign asset absent from one particular market's current
book, not necessarily a misconfiguration. Publishing the gap in
`report.json` makes it visible to an operator or dashboard without making
a single-asset mismatch destructive to the rest of the report. Score
gating is unchanged: this is purely additional disclosure, matching "keep
current score gating until valuation and recovery assumptions are
verified."

Excluded positions (`Accumulator::exclude()`) do not contribute to
matching, which is correct: a position excluded from stress evaluation
(unknown cached debt) cannot demonstrate that a configured asset applies
to it either.

## Acceptance checks

New test `reports_matched_and_unmatched_price_override_assets`
(`tests/stress.rs`): configures two `price_factors` overrides in one
scenario, evaluates a single position whose assets match only one of
them, and asserts the matched key appears in `matched_price_overrides`
(and not in `unmatched_price_overrides`) while the never-observed key
appears in `unmatched_price_overrides` (and not in
`matched_price_overrides`).

Existing tests updated for the field additions:
- `tests/stress.rs::selects_worst_scenario_rate_not_largest_dollar_loss`
  constructs `Outcome` literals directly and needed the two new fields
  added (`Default::default()` - empty sets, since that test only exercises
  `score()`, not coverage tracking).

All existing stress/report/export regressions pass unchanged otherwise -
this is a purely additive field on `Outcome`, serialized like every other
field, with no schema-version bump needed (same reasoning as F02: additive
fields don't require migration handling, only a `#[serde(default)]` guard
where old data must still deserialize - not needed here since `Outcome` is
never read back from old artifacts, only written fresh each report run).

## Commands run

```
cargo test --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
```

All passed.

## Limitations / not done here

- No dashboard panel surfaces `unmatched_price_overrides` yet. temp.md's
  Grafana work (U01-U03) covers dashboard panel additions as its own task;
  adding a panel here would be scope creep into a UX-gated task with its
  own acceptance criteria (operator timing tests, three-operator UX gate)
  that this code-only task did not run.
- This does not address F04 (deployed-source/oracle verification) or the
  broader stress-model gaps in temp.md section 6 ("Economic stress and
  decision eligibility": correlated exposures, depegs, liquidity
  constraints, calibrated recovery assumptions) - those remain tracked
  under V04, which depends on V01's verification gate.
