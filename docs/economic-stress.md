# Economic stress model

The `analysis::stress` module implements checked-decimal, per-position scenario arithmetic.
Offline reporting feeds it individual asset exposures; exporters only present
saved results. No position book or chain connection is needed to serve Grafana.

## Calculation

For each position `i` and scenario `s`:

```
collateral(i,s) = sum(cached collateral USD by asset × asset price factor)
debt(i,s)       = sum(cached debt USD by asset × asset price factor)
recovery(i,s)   = collateral(i,s) × (1 − recovery discount)
shortfall(i,s)  = max(debt(i,s) − recovery(i,s), 0)
loss_rate(s)   = sum(shortfall(i,s)) / sum(debt(i,s))
score          = 100 × min(max(loss_rate(s)) / loss_tolerance, 1)
```

A price factor applies identically to an asset on both sides. Shortfalls are
calculated before aggregation: surplus in one account cannot offset another
account's deficit. Scenario debt, not current debt or collateral, is the denominator.
Zero-debt scenarios have no loss rate; no debt in any scenario yields no score.
The worst loss *rate* determines the score, not the largest dollar shortfall.
Higher scores mean greater modeled stress. This is not a probability of failure,
a protocol credit rating, or a measure of smart-contract security.

The default loss tolerance is 5%: a worst loss rate of 1% maps to 20/100, and 5%
or more maps to 100/100. These are explicit, illustrative policy choices, not
empirically calibrated thresholds. Backtest scenarios and calibrate risk tolerance
before using a score in production decisions.

## Current availability

The current collector supplies cached valuations. **Every published economic
score therefore remains null**, and Prometheus omits the numeric sample. Grafana
shows a gray Unavailable state, separate from zero risk. There is no CLI option
to mark cached input verified. The pure model's `Basis::Verified` contract is for
a future adapter that establishes fresh prices/accrual and validated recoveries.

Cached scenario dollar results remain available as **illustrative sensitivities**.
Positions with unavailable diagnostics or borrow records with unknown zero cached
weighted debt are excluded and counted. An incomplete capture or any exclusions
also blocks the market score. These results cover only evaluated positions and
must not be represented as complete losses for a partially evaluated market.

Recovery discount is a fixed assumption, not observed DEX liquidity. The model
does not execute liquidation rules, close factors, bonuses, price impact, interest
accrual, recovery timing, insurance waterfalls, or cross-asset correlations inferred
from history. It reprices existing cached USD legs, which may have different ages.
Fresh deployed-source and oracle-path verification remain required; see
[protocol research](suilend-notes.md). A zero hypothetical shortfall does not
establish safety or liquidatability.

## Scenario policy

`deploy/config/stress.toml` is embedded in the binary and retained in each new report. It defines:

- A 10% recovery discount at unchanged prices.
- Native SUI down 30%, with a 5% recovery discount.
- Native SUI up 30%, with a 5% recovery discount (also stresses SUI borrowers).

Only the exact native SUI coin type receives those price shocks. SUI derivatives,
stablecoins and other assets are unchanged unless explicitly listed. This is a
small sensitivity suite, not an exhaustive crisis model. Coin-type overrides use
canonical 64-digit lowercase package IDs without `0x`, matching capture artifacts.
Unknown override keys do not match any exposure; review IDs against the report.

To choose another policy, use a new output directory:

```sh
cargo run --release --locked -- report --input snapshots/mainnet \
  --out snapshots/stress-report --stress-config deploy/config/stress.toml
```

Policies reject unknown fields, duplicate/invalid scenario names, invalid ratios,
and more than 16 scenarios. Price factors range from zero to ten; recovery
discounts from zero to one; loss tolerance must be greater than zero and at most
one. All arithmetic is checked; overflow aborts before publishing a report.

Results remain separate per market. Grafana's score panel is the worst selected
market score and is unavailable if any selected market lacks a score. Scenario
dollar panels sum the selected markets, preserving per-position losses. Discovery
is not an endorsement registry, so choose the intended scope explicitly. A pooled
shortfall ratio is not evidence that liquidity or loss absorption can move between
isolated markets.

Older reports remain readable and show Not computed for stress. Regenerate from
the retained raw capture to add scenarios; no recrawl is necessary. Report schema
1 gains optional `stress_policy` and per-market `stress` fields; consumers that
require stress results must check their presence and status.

Each scenario outcome also publishes `matched_price_overrides` and
`unmatched_price_overrides`: the configured `price_factors` asset keys that
did, and did not, match any evaluated collateral or debt leg in that market.
An unmatched override is typically a typo'd coin type or an asset that is
configured but genuinely absent from the current capture; it is published
rather than silently falling through to `default_price_factor`, but does not
by itself fail report generation or change the stress score - current score
gating stays as described above until valuation and recovery assumptions are
independently verified (see the financial-verification tasks in [the roadmap](roadmap.md)).
