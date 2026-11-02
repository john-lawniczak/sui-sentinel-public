# Suilend protocol findings and verification gaps

Research date: 2026-10-19. **Fresh valuation gate: BLOCKED.**

Checkpoint collection and cached diagnostics are implemented; see
[snapshot architecture](snapshot-design.md). Protocol observations below are dated
evidence. Fresh valuation must wait for verified deployed rules and effective price
paths; cached diagnostic output does not establish that verification.

## Blocking findings

1. **The available public Move source has not been tied to the deployed code.**
   Both public `devel` and `mainnet` branches resolved to
   `d5ba83a617bb0778b48b0c3b1e77a87be81258ca` (commit date 2026-03-06).
   Its [Published.toml][published] records package version 20 at
   `0x3d4353f3bd3565329655e6b77bc2abfd31e558b86662ebd078ae453d416bc10f`.
   The live UpgradeCap instead reports version 25 at the address below.
   A metadata difference alone would not prove a semantic difference, but
   [SDK 13.6.0's Pyth integration][pro] calls
   `lending_market::refresh_reserve_price_pro_compatible`, absent from the public
   Move source. [Its client][sdk] also describes a forward-time oracle bound
   absent from that source. Deployed oracle validation and price-update behavior
   therefore remain unverified from Move source. The npm package's repository URL,
   `fireflyprotocol/lending-mono`, returned GitHub HTTP 404 with available access.
2. **Complete current reserve-to-oracle-object mappings are unverified.**
   A reserve stores a feed identifier, not a `PriceInfoObject` address. Several
   distinct assets share a feed identifier, including SUI derivatives and assets
   whose identifier equals USDC's. It is unsafe to interpret every raw identifier
   as a direct USD price for that reserve. Current wrapper, temporary-feed, and
   exchange-rate paths need verification. The current SDK explicitly distinguishes
   legacy and upgraded Pyth objects. No per-reserve object address is guessed.
3. **The requested verification needs a valuation-aligned reference.**
   Obligations store cached USD aggregates, not a continuously updated HF field.
   Fresh debt accrual and today's reserve prices cannot be expected to match an
   obligation's older stored values. Establish an independent reference using
   historical state at its actual valuation point or simulation of the deployed
   refresh path. Do not make verification pass by merely dividing cached totals
   on both sides, selecting only zero-debt accounts, or widening the tolerance.

To unblock: obtain a public source revision with established correspondence to
the version-25 package; validate every configured reserve's current pricing path
and Pyth object; then implement a reference-aligned verification procedure.

## Deployment identity at research checkpoint 335784468

The [official SDK guide][guide] identifies the `@suilend/sdk` package. Its
[version-pinned client][sdk] identifies the main market, original package,
market type, UpgradeCap, and the package associated with market version 7.
The market and UpgradeCap were independently queried from mainnet.

| Item | Observed value |
| --- | --- |
| Original package / type origin | `0xf95b06141ed4a174f239417323bde3f209b972f5930d8521ea38a52aff3a6ddf` |
| Current published package from UpgradeCap | `0x59672975e15c58ffc9450f5236b6aa9fa2407d94b8081e634778aa01957d8c6f` |
| UpgradeCap | `0x3d4ef1859c3ee9fc72858f588b56a09da5466e64f8cc4e90a7b3b909fba8a7ae` |
| Package version | `25` |
| Main LendingMarket | `0x84030d26d85eaa7035084a057f2f11f701b7e2e4eda87551becbc7c97505ece1` |
| Market schema version | `7` (distinct from package version) |
| Market type parameter | `0xf95b06141ed4a174f239417323bde3f209b972f5930d8521ea38a52aff3a6ddf::suilend::MAIN_POOL` |
| Obligation table, discovered from market | `0xcffebc0eb9d701d7843346dab87a043a60e28e595278db5764b87a4b3919a00a` |
| Obligation table size | `149922` |
| Reserve count | `45` |

These identifiers are recorded in [deploy/config/suilend.toml](../deploy/config/suilend.toml); the table ID must be
discovered at runtime. This table describes the initial main-market observation.
The implemented collector discovers all market type instances and defaults to the
configured main market. The later [full census](evidence/snapshot-2026-10-19.json)
captured 26 instances at checkpoint 335839129; discovery does not establish official
endorsement of a market.

[The raw GraphQL response](evidence/mainnet-market-2026-10-19.json) includes
checkpoint `335784468`, timestamp `2026-10-19T14:56:27.474Z`, market object version
`1054644385`, and UpgradeCap object version `852336188`. A separate gRPC GetObject
call confirmed the same UpgradeCap package/version. The raw response is evidence
of the queries performed, not a cryptographic proof of source equivalence or a
complete, normalized State snapshot. See [reproduction instructions](evidence/README.md).

## Reserve and obligation storage; exhaustive discovery

[LendingMarket][market] embeds a `vector<Reserve<P>>` and an
`ObjectTable<ID, Obligation<P>>`. The table's UID is available in the market's
`obligations.id`; its `size` is a completeness check. A reserve has a UID but its
main record is embedded in the market, so reading a reserve ID as if it were an
independent top-level Reserve object is incorrect. Reserve balance and other
dynamic fields need separate reads where relevant.

An obligation is a child object of a dynamic-object-field wrapper beneath the
table. The implemented collector uses GraphQL checkpoint scopes and reads each
table through `address(address: table_id)`; see [snapshot architecture](snapshot-design.md).
The initial research also verified this gRPC discovery path:

1. Read the market, validate its type/version, and extract the table UID and size.
2. Page `sui.rpc.v2.StateService/ListDynamicFields` on that UID until
   `next_page_token` is absent. Use the returned `child_id`, not `field_id`.
   Validate `kind = OBJECT` and the exact obligation type including market type.
3. Deduplicate the child IDs and batch-read them with
   `sui.rpc.v2.LedgerService/BatchGetObjects`. Set the top-level field mask;
   individual failed objects must fail completeness rather than silently disappear.
4. Confirm every obligation's `lending_market_id`, resolve all reserve indexes,
   and compare the unique count with the table size at a compatible checkpoint.
   Concurrent mutations require a pinned snapshot or reconciliation; counting
   pages against a separately timed table read does not prove completeness.

The public fullnode supports this listing directly. GraphQL is an alternative
for snapshot-consistent listing/filtering and later history, not a mandatory
event-history workaround for current obligation discovery. Event-only discovery
must establish historical completeness and handle deletions, so the table is the
preferred authoritative inventory. [Sui gRPC documentation][grpc] describes the
services; the observed live schema uses `child_id`.

[Discovery evidence](evidence/grpc-discovery-2026-10-19.json) contains two child
IDs and a continuation token. [Batch-read evidence](evidence/grpc-obligations-2026-10-19.json)
contains those two obligations. **That initial probe fetched one page; it is not a
full census or a verification sample. Both probe obligations have zero weighted debt.**
No JSON-RPC calls were used.

The implemented collector bounds concurrency, applies deadlines and bounded retries,
and records pagination exhaustion and table-count reconciliation. Sampled captures
are explicitly incomplete. Source timestamps and object versions are retained;
see the [full-census evidence](evidence/snapshot-2026-10-19.json).

## Scaling, exchange rates, and interest in the available source

The following describes the pinned public source, **not verified version-25
equivalence**. Sources: [decimal.move][decimal], [reserve.move][reserve],
[reserve_config.move][config], and [obligation.move][obligation].

* `Decimal.value` is a `u256` scaled by `10^18`. Token amounts are in atomic units;
  token conversion divides by `10^mint_decimals`. These two scales are distinct.
* Fixed-point multiplication and division truncate after each operation.
  Exponentiation uses repeated squaring with those truncations. Reordering
  algebraically equivalent expressions can change the final result.
* `total_supply = available_amount + borrowed_amount - unclaimed_spread_fees`.
  `ctoken_ratio = total_supply / ctoken_supply`, or 1 if cToken supply is zero.
  Deposited underlying atomic units equal `deposited_ctoken_amount * ctoken_ratio`.
  The implementation multiplies by the ratio; some nearby comments are misleading.
* Utilization is borrowed amount divided by the above supply (zero for empty
  supply). APR interpolates the configured utilization percentage breakpoints
  and APR basis points. Use the actual reserve configuration, not fixed APRs.
* With elapsed whole chain-clock seconds `dt`, growth is
  `(1 + APR / 31_536_000)^dt`. Multiply the reserve cumulative index by growth,
  add `old_borrows * (growth - 1)` to reserve debt, and add the configured spread
  fee on new interest to unclaimed fees. Recompute the cToken exchange rate too.
* User debt is `old_user_debt * (new_reserve_index / user_recorded_index)`, with
  division rounded before multiplication as in `compound_debt`. Accruing only
  reserve totals or only user debt gives inconsistent health and solvency.

Use the Sui Clock time associated with the snapshot. Reject future interest
timestamps or zero borrow indexes; do not replace invalid input with zero debt.
Keep amounts as decimal strings in captured data. Live raw scaled debt values
already exceed the 96-bit coefficient of `rust_decimal`; directly parsing every
raw `u256` as a Decimal integer is not safe. An exact normalization and rounding
strategy must be established before implementing fresh valuation. The current
collector retains raw amounts as strings; cached USD diagnostics normalize the
scale before checked Decimal parsing and reject unsupported diagnostic ranges.

The supply equality alone is tautological if supply was derived from its right
side. A meaningful solvency check also needs independently reconstructed user
debts at one timestamp and, where available, independently checked balances.
Deposits held outside obligations mean summed obligation collateral is not the
entire reserve supply. Inspect [reserve staking behavior][reserve] before treating
an underlying cash Balance as identical to the reserve's accounting availability.

## Prices, health, and liquidation in the available source

`obligation::refresh` compounds the participating reserves and user debts, then:

* Deposited value and liquidation-threshold value use spot `reserve.price`.
* Allowed borrow value uses `min(spot, smoothed_price)` times open LTV.
* Liquidation-weighted debt uses spot price times `borrow_weight_bps / 10_000`.
* Borrow/withdraw-limit debt uses `max(spot, smoothed_price)` times borrow weight.

Thus the liquidation HF derived from this source is
`unhealthy_borrow_value_usd / weighted_borrowed_value_usd`, when debt is nonzero.
No debt means unbounded HF, which needs an explicit representation rather than
a misleading finite score. `is_liquidatable` uses **strict greater-than** on debt
versus threshold; HF exactly 1 is not liquidatable in this source. The cached
diagnostic buckets preserve that boundary without claiming fresh liquidation status.

[The old oracle parser][oracles] requires nonnegative Pyth prices, rejects
`confidence * 10 > price`, and rejects publish times more than 60 seconds behind
the chain Clock. It accepts forward timestamps without the newer SDK-described
bound. Price-update time is written from the chain Clock, not copied from Pyth's
publish timestamp; preserve both timestamps when measuring freshness.

Liquidation parameters are reserve-specific. The captured SUI reserve has close
LTV 75%, liquidator bonus 300 bps, and protocol liquidation fee 399 bps. Do not
substitute the documentation's illustrative 5% bonus for those values.

The public [liquidate implementation][obligation] additionally:

* Restricts seized collateral to one of the highest borrow-weight deposits.
* Caps total bonus (liquidator bonus plus protocol fee) by remaining collateral
  surplus over unweighted debt; a bonus cap can enable full liquidation.
* Allows full liquidation of a selected borrow worth at most $100.
* Otherwise uses 20% of total weighted debt, capped by the selected borrow's
  unweighted value, to bound repayment. This is not universally 20% of each loan.
* Caps seizure by available collateral; protocol fee is deducted from the bonus.

Any future `collateral < debt * (1 + liq_bonus)` counter should be labeled as a
scenario indicator. The actual capped bonus, protocol fee, collateral selection,
gas, and executable liquidity determine profitability. [Suilend's liquidation
guide][liquidations] is introductory; the current deployed implementation must
settle these details before production calculations.

## Pyth feeds and objects

The full 45-reserve response preserves each coin type, decimals, feed bytes,
configured parameters, and cached prices. A decoded, evidence-only inventory is
in [reserve-feeds.json](evidence/reserve-feeds.json). It is not the runtime asset
mapping: `pyth_price_object_id` is explicitly unknown for every row.

For example, SUI's raw feed identifier is
`23d7315113f5b1d3ba7a83604c44b94d79f4fd69af77f804fc7f920a6dc65744`;
native USDC's is
`eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a`.
These are **feed IDs, not Sui object IDs**. Reserve `SPRING_SUI` shares SUI's ID;
reserve `FUD` shares USDC's ID in the captured state. This observation does not
establish identical effective prices or stablecoin status.

The [SDK client][sdk] uses Pyth state
`0x03719fae774ddab3cfcaa53bbc046f0cbe21410019b6280811bf3f9f4b05839d`
and resolves feed objects through `SuiPythClient.getPriceFeedObjectId`.
[The upgraded integration][pro] warns that legacy objects abort the current path.
To complete research, resolve the current feed table, validate each resulting
object's type and embedded feed identifier, and verify how derivative and
temporary feeds are handled by the deployed package. External latest Pyth prices
cannot silently replace the protocol's cached or transformed prices.

The eventual single config asset table must include coin type, decimals, verified
feed and object, optional DeepBook pool, and independently justified `is_stable`.
No DeepBook mapping means UNKNOWN liquidity. DeepBook bids must be converted from
their quote denomination into USD using fresh prices; arbitrary pool names or
an assumption that every quote token is $1 would be incorrect.

## Verification status

The [full-census evidence](evidence/snapshot-2026-10-19.json) records 159,717
obligations across 26 market instances at checkpoint 335839129. The collector,
offline reports, cached diagnostics, and regression tests are implemented.

Current deployed Move-source correspondence and full effective-price mapping remain
unresolved. `verify` is unimplemented; no fresh health-factor verification is claimed.
Exporters, operational alerts, Docker Compose and Grafana are implemented;
see [current readiness](readiness.md). Their availability does not verify fresh financial math.
Neither collection nor offline reporting submits chain transactions.

[published]: https://github.com/suilend/suilend/blob/d5ba83a617bb0778b48b0c3b1e77a87be81258ca/contracts/suilend/Published.toml
[market]: https://github.com/suilend/suilend/blob/d5ba83a617bb0778b48b0c3b1e77a87be81258ca/contracts/suilend/sources/lending_market.move
[reserve]: https://github.com/suilend/suilend/blob/d5ba83a617bb0778b48b0c3b1e77a87be81258ca/contracts/suilend/sources/reserve.move
[obligation]: https://github.com/suilend/suilend/blob/d5ba83a617bb0778b48b0c3b1e77a87be81258ca/contracts/suilend/sources/obligation.move
[config]: https://github.com/suilend/suilend/blob/d5ba83a617bb0778b48b0c3b1e77a87be81258ca/contracts/suilend/sources/reserve_config.move
[decimal]: https://github.com/suilend/suilend/blob/d5ba83a617bb0778b48b0c3b1e77a87be81258ca/contracts/suilend/sources/decimal.move
[oracles]: https://github.com/suilend/suilend/blob/d5ba83a617bb0778b48b0c3b1e77a87be81258ca/contracts/suilend/sources/oracles.move
[sdk]: https://unpkg.com/@suilend/sdk@13.6.0/client.js
[pro]: https://unpkg.com/@suilend/sdk@13.6.0/lib/proCompatible.js
[guide]: https://docs.suilend.fi/ecosystem/suilend-sdk-guide
[liquidations]: https://docs.suilend.fi/security/liquidations
[grpc]: https://docs.sui.io/develop/accessing-data/grpc/using-grpc
