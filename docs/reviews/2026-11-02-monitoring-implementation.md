# Continuous-monitoring implementation record

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

Scope: the next priorities after the implementation review, plus the user's
specified free public Mainnet gRPC development source. Financial verification,
additional real protocols and MCP/LLM integration are not promoted by this change.

## Behavior delivered

- Readiness uses checkpoint age, complete diagnostic coverage, configured scope
  and reload health. A stale loaded report becomes unready while remaining readable.
- PostgreSQL transactions atomically persist signal receipts, incident transitions
  and outbox notifications. Replay deduplicates before transitions; older evidence
  cannot override newer outcomes. Unchanged freshness severity does not flood alerts.
- A single-host scheduler runs nonoverlapping captures, replays lifecycle outcomes,
  evaluates freshness independently and delivers leased webhooks with retry budgets.
- Killable capture/validation workers cannot publish. The parent validates under
  its lock and admits pointer replacement only before the deadline. Run-journal
  recovery handles interruption and the pointer/final-record crash window.
- Raw schema-2 checksums bind all object files, manifest and capture configuration.
  Report provenance links back to that binding. New reports cannot omit required
  checksums; legacy captures remain readable without gaining new integrity claims.
- A real incremental adapter owns Suilend decoding, dependency extraction and cached
  interpretation. Shared projections store object versions, tombstones, dependencies,
  raw relevant effects, evaluations and committed/evaluated watermarks atomically.
- Wrapped IDs can return; permanently deleted IDs cannot. Unsupported deployment
  changes and conflicting/gapped checkpoints fail before progress advances.
- `live` uses `sui-rpc` 0.4.0 with configurable endpoint/key/header, checkpoint
  persistence, SDK reconnection/backfill and periodic pinned GraphQL reconciliation.
  The default public endpoint is development-only. Focus uses SDK-resolved main
  market identity and native USDC, distinct from wUSDC; full inventory is retained.
- The writer actor has bounded admission and commit acknowledgments. Immutable
  market inputs are shared with Arc; evaluations persist in batches of 256.
  Exporter reload I/O/rendering stays outside the short presentation-swap lock.
- Compose serves generation directories; an optional overlay adds PostgreSQL and
  monitoring. Grafana readiness labels and CI fixture publication match the new mode.
- `temp.md` now lists only unfinished requirements and release evidence.

## Verification

- Full Rust suite: 111 tests passed, including 21 tests exercising the temporary
  PostgreSQL instance. Database tests were not skipped.
- Regressions cover replay concurrency, outbox-write rollback, old-signal ordering,
  receiver retry/dedup IDs, lease reclaim, scheduler continuation after failures,
  synchronous worker termination, raw corruption, stale readiness, checkpoint gaps,
  capability transfers, wrapping/deletion, unsupported upgrades and backpressure.
- Formatting, Clippy with warnings denied, maintained-file limits, generated
  dashboard verification, whitespace checks and both Compose configurations passed.
- Bounded public-node gRPC checks verified mainnet identity, current market/package
  identifiers, exact-version normalization and sequential historical/resumed frames.
  Evidence: [development probe](../evidence/grpc-development-probe-2026-11-02.json)
  and [official SDK identity](../evidence/sdk-live-scope-2026-11-02.json).

## Practical limits and next evidence

No new complete main-market bootstrap/reconciliation run, full Linux Docker stack,
rendered browser UX test, sustained main-market throughput benchmark, operational
soak or independent financial-rule review was completed here. Docker was unavailable
locally; Compose validation is not a running-container test. The public probe is
bounded and does not establish service reliability or a production provider SLA.

Fresh risk and valuation time remain unavailable, and execution eligibility remains
false. Dependency-driven evaluation recomputes cached diagnostics; it cannot turn
stale stored financial aggregates into verified current valuations. Checkpoint
signatures are not locally verified; this development mode trusts its provider.

The incremental database does not yet have the planned versioned query/MCP surface
or complete Grafana incident workflow. Snapshot readiness and live-state health
must not be conflated. Remaining operating, performance, verification and expansion
requirements are recorded in [the backlog](../readiness.md), without completed tasks
or obsolete readiness percentages.
