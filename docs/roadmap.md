# Roadmap: continuous observation, verified risk and composability

These are unfinished capabilities beyond the cached-monitoring implementation.
Use [readiness and R1 gates](readiness.md) for current release blockers and execution
rules, and [the runbook](operations-release.md) for shipped behavior. Task IDs remain
stable so evidence and dependent work can refer to the original plan.

## Incremental correctness and performance

### S01 — Complete the protocol boundary

Files: `src/protocols/`, `src/model.rs`, snapshot discovery and offline report code.

- [ ] Move remaining Suilend-specific discovery/configuration/report decoding
  out of shared infrastructure. Keep capability-specific report models explicit.
- [ ] Expand adapter manifests to supported deployments, versions, dependency
  requirements and confidence. The cached Suilend registry now dispatches runtime adapters;
  remaining shared discovery/report coupling must still be removed.
- [ ] Add conformance fixtures for upgrades, unsupported types, ownership changes,
  dependency cycles and isolation between adapters.

Acceptance: shared orchestration has no Suilend type/field branches; an adapter
cannot silently claim fresh lending-health or liquidity capabilities it lacks.

### S02/S03 — Harden durable state and the live source

Dependencies: main-market evidence from O01. Files: `src/state/`, `src/ingest/grpc/`.

- [ ] Test all lifecycle shapes at real deployed versions: create, mutate, wrap,
  unwrap, permanent delete, capability reassignment and package migration.
- [ ] Validate bootstrap and reconciliation under concurrent upstream changes,
  long captures, expired retention, partial responses and unavailable archives.
- [ ] Schedule reconciliation by elapsed time as well as checkpoint count; expose
  failures durably and define an explicit repair/rebootstrap workflow.
- [ ] Prove upgrade rollback and resumed replay after restoring production-sized
  projections; preserve relevant effects/source overlap before pruning. Local
  full-size restore and content-hash comparison already passed; validate the
  subsequent recovery workflow and hosted environment.
- [ ] Decide whether provider trust is sufficient per capability. If local
  checkpoint/effects signature verification is required, implement and test it;
  matching provider digests alone does not provide that proof.

Acceptance: no skipped checkpoints; divergent replays fail; independent complete
census comparisons succeed; unsupported changes stop affected capability progress.

### S04 — Versioned query and diagnosis publication

Dependencies: S02/S03. New `src/query/` over committed projections and incidents.

- [ ] Add retained historical projections and their retention-expired contract.
  Current scoped queries pin the current generation and reject changed generations;
  checkpoint-frame history is not a historical valuation API.
- [ ] Extend capability-specific source/price/valuation provenance once verified;
  current responses expose observation/recalculation checkpoints, configuration,
  reconciliation lag, confidence and explicit unknown valuation age.
- [ ] Measure sustained query availability under real ingestion load; add scoped
  caching only if profiling justifies it. Bounded pages, cancellation, read-only
  queries and private transport authentication are implemented in [MCP](mcp.md).
- [ ] Publish deterministic configuration/ownership/activity change diagnoses
  with transitions, affected exposure, cause and reproducible inputs.

Acceptance: concurrent readers never mix generations; unknown values remain
unknown; complete responses have reproducible evidence and bounded cost.

### S05 — Benchmark and optimize measured bottlenecks

Dependencies: O01 and S02/S03. Keep actor admission bounded and commits ordered.

- [ ] Validate sustained real mixed-mutation throughput after the cached clock/
  unchanged-reserve invalidation fix. Synthetic full-book clock+market reuse passed
  with unchanged cached evaluation hashes; it does not qualify real ingestion.
  New financial adapters must declare their own clock/price dependencies and may
  not inherit the cached adapter's input-equivalence optimization blindly.
- [ ] Benchmark the real fixture, full main-market book and a documented synthetic
  million-position workload. Include hot reserves, clock changes, 10x bursts,
  full fan-out, catch-up and concurrent dashboard/query traffic.
- [ ] Measure RPC latency/bytes, decode CPU, database queries/commit time,
  evaluations/second, queue wait/age/bytes, lock wait, allocations and peak RSS.
- [ ] Batch bootstrap object/dependency writes, reuse versioned reserve inputs
  and avoid unnecessary whole-market recalculation after unrelated mutations.
- [ ] Add bounded parallel evaluation only with a completed-checkpoint barrier;
  never publish partial fan-out as a fully evaluated checkpoint.
- [ ] Shard writers by independent protocol/market if contention is demonstrated.
  Keep cross-scope reads pinned to a common completed checkpoint.
- [ ] Consider lock-free publication only if short immutable-generation swaps
  materially affect measured tail latency. Do not replace database transactions
  or checkpoint sequencing with uncoordinated atomic counters.

Acceptance targets, measured from provider checkpoint availability: p95 <=5 s
normal, <=15 s declared burst, p99 <=30 s. Measure chain timestamp lag separately.
Catch-up should sustain >=3x measured arrival rate, draining a 10-minute outage
within five minutes when provider capacity permits. Target summary-query p95
<=250 ms and indexed-page p95 <=1 s at declared concurrency. Record hardware,
cost, workload hashes and failures; these are targets, not current results.

## Grafana operator experience

### U01/U02 — Complete overview and operations workflows

Dependencies: O03/O04; use `deploy/grafana/` and `docs/dashboard.md`.

- [ ] Add scoped incident/evidence drill-down links and readable identities using
  the bounded query service. The operations overview and policy-derived freshness
  are implemented; distinct fresh-price/valuation indicators depend on verified
  financial capabilities, not renamed snapshot age.
- [ ] Build a state matrix: startup, healthy cached, stale, partial, unsupported
  schema, collector failure, reload failure, delivery failure and recovery.
- [ ] Run rendered browser checks at desktop/mobile sizes, keyboard navigation
  and empty/error states. Keep checkpoint/scope context in every drill-down link.
- [ ] Have operators identify staleness, find a failed capture and explain an
  incident using evidence; retain timings, mistakes and screenshots.

Acceptance: operators can distinguish monitor health from protocol risk without
reading source code. Generated dashboard/query checks alone do not complete UX.

### U03 — Protocol and incident investigation views

Dependencies: U02 and S04. Add actual generation history, incident timelines,
condition changes and evidence drill-downs. Test aggregate semantics, scope filters,
common checkpoints and query budgets. Repeated scrapes are not market history.

## Verified financial capabilities

### F04/V01 — Deployed-rule research and exact valuation

Dependencies: assigned protocol owner and independent reviewer.

- [ ] Timebox an initial five-working-day verification spike. Produce a dated
  deployment/reserve matrix with verified, partial or blocked decisions, evidence
  links, exact missing inputs and a next-review date.
- [ ] Establish deployed source/package correspondence, oracle object mappings,
  accepted-price rules, wrapper rates, staleness/future bounds and accrual rules.
- [ ] Implement exact arithmetic, interest, exchange rates and risk thresholds
  only for verified paths. Keep external price scenarios separately identified.
- [ ] Compare against independent checkpoint-aligned execution/reference fixtures;
  cover zero/extreme values, rounding, boundary health and upgrades.

Acceptance: every unexplained difference blocks the affected financial capability.
The initial financial focus is SUI collateral and native USDC debt in the main
market; an obligation may contain other assets, whose dependencies must still
be accounted for before making a whole-position financial claim.

### V02/V03 — Fresh dependency evaluation and financial diagnoses

Dependencies: V01 and S03/S04.

- [ ] Build verified asset/reserve/oracle and transitive wrapper dependencies;
  invalidate on prices, accrual time, exchange rates and parameter changes.
- [ ] Reuse shared reserve computations by input version; compare incremental
  results and aggregates with full recomputation at every test checkpoint.
- [ ] Enforce per-reserve accepted-price and per-capability valuation-age limits;
  unknown/stale/incomplete dependencies suppress affected financial conclusions.
- [ ] Add financial rules with hysteresis, affected exposure, supporting inputs
  and confidence. Evaluate held-out incidents for false negatives/positives.

Acceptance: unchanged obligations receive correct fresh results when dependencies
change; stale data cannot remain decision-eligible. Establish label counts and
reviewed recall/precision thresholds before evaluating the held-out suite.

### V04/V05 — Stress realism and decision-input eligibility

Dependencies: V01–V03 and S05/U03.

- [ ] Add correlated shocks/depegs, wrapper exposure, measured liquidity limits
  and calibrated recovery assumptions; preserve scenario coverage/exclusions.
- [ ] Validate historical/adversarial cases and shared-liquidity exhaustion.
- [ ] Specify decision-time object/price revalidation and simulation contracts.
  Run 30-day shadow evaluation and obtain independent correctness review.

Acceptance: unsupported liquidity or valuation paths remain ineligible. No live
transaction executor or autonomous capital decisions are implied by this backlog.

## Additional protocols and AI integration

### P01/P02 — A second real adapter

Dependencies: S01/S03/S04. Select NAVI or Scallop using current deployment,
complete-discovery and independent-reference evidence. Implement it end to end;
run shared conformance, recovery and two-adapter isolation tests. Verify its math
separately before enabling financial capabilities. Resolve canonical assets,
ownership and backing without cross-protocol double counting.

### A01/A02 — Bounded read-only MCP

Dependencies: S04; reuse the query service, not direct raw database/file access.

- [ ] Qualify the actual assistant clients and remote gateway. Local stdio and
  private shared-token HTTP have SDK and independent JSON-RPC tests for pinned
  protocol revisions; public OAuth discovery/per-user authorization is not shipped.
- [ ] Improve capability-specific output schemas and versioned historical valuation cursors. The ordered incident journal now has resumable
  update/delete cursors and explicit history-expired/bootstrap semantics.
  Eight read-only tools, current-generation pagination and bounded evidence lookup
  are implemented; use [the MCP guide](mcp.md) before extending them.
- [ ] Add bounded scenario evaluation with cancellation, scope/cost limits,
  reproducible inputs and explicit confidence/assumptions.
- [ ] Validate cancellation, hostile clients and ingestion isolation at sustained
  production concurrency; add retention-expired semantics when historical state
  is implemented. Current server enforces scope, admission, row/byte/time limits,
  secret isolation, private bearer authentication and read-only database access.

Acceptance: decimal amounts stay strings; tools cannot mix checkpoints, fetch
arbitrary paths/URLs, mutate incidents or interfere with ingestion/mandatory alerts.

### A03 — Supervised LLM investigation

Dependencies: A01/A02 and durable incidents.

- [ ] Persist incident cursors, job IDs, retries and evidence-backed annotations.
- [ ] Bound each investigation initially to 30 s, 8 tools, 100 rows/page,
  256 KiB/response, 12,000 tokens and 4 concurrent jobs; enforce priced cost caps.
- [ ] Test prompt injection, unavailable models/tools, stale/partial evidence,
  fabricated citations and ambiguous causes. Models cannot promote confidence,
  change deterministic severity or acknowledge incidents implicitly.
- [ ] Have independent reviewers score usefulness and evidence correctness;
  target median usefulness >=4/5 and measure investigation-time improvement.

Acceptance: deterministic monitoring remains available during model failure;
unsupported claims fail evaluation rather than being saved as trusted findings.

## Release evidence still required

| Gate | Required evidence |
| --- | --- |
| R1 — Operated cached monitoring | O01–O05 and U02; seven-day unattended soak, correct freshness/coverage behavior, delivery/recovery drills and operator UX evidence. |
| R2 — Continuous observation | R1 plus S01–S05/U03; 30-day soak, no unexplained gaps, independent census reconciliation and measured latency/recovery. |
| R3 — Verified Suilend scope | R2 plus V01–V05; independent aligned financial correctness, reviewed eligibility and shadow decision-input evidence. |
| R4 — Composable platform | R2 plus P01/P02/A01–A03; two adapters and MCP/AI gates at explicitly declared confidence levels. |

For enabled capabilities, target >=99.9% fresh-and-completely-evaluated wall-clock
time and >=99.9% query-serving availability, measured separately. Count missing
telemetry/outages as failures; publish denominators and exclusions. Do not average
away a weak capability to claim 8.00/10.00 across the board.

## Optional post-R4 extensions

- [ ] P03: verified DEX depth/route adapter with trade-size-dependent quotes,
  simulation comparisons and shared-liquidity accounting.
- [ ] P04: staking/vault adapters with verified backing, exchange rates,
  withdrawal delays, cycle detection and unsupported-upgrade behavior.
- [ ] P05: cross-protocol exposure and correlated recovery at a common completed
  checkpoint, without double-counting backing assets or route liquidity.
