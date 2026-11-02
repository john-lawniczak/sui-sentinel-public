# Continuous monitoring and development gRPC ingestion

Sentinel supports two complementary paths: scheduled complete snapshots for
cached dashboards/operational alerts, and checkpoint ingestion into durable state.
Neither path claims verified fresh liquidation values. See [R1 release gates](readiness.md)
for production qualification and [the roadmap](roadmap.md) for financial
verification and additional adapters. The implemented read-only tools and incident
change feed are described in [MCP setup](mcp.md).

## Scheduled captures and alerts

```sh
export DIAGNOSIS_DATABASE_URL='postgres://sentinel:YOUR_PASSWORD@localhost:5432/sentinel'
export SENTINEL_WEBHOOK_URL='https://YOUR_RECEIVER/incidents'
# Optional receiver authentication; keep secrets outside git.
export SENTINEL_WEBHOOK_TOKEN='YOUR_TOKEN'
cargo run --release --locked -- monitor --out snapshots/operated --scope suilend-main
```

Configure interval `T`, deadline `D` and exact expected markets in
`deploy/config/monitoring.mainnet.toml`, the `monitor` default. It pins the main
market with provisional T=3,600 seconds and D=2,400 seconds based on selected-provider
census. The generic `monitoring.toml` remains for fixture/local use.
Only one monitor per output directory can run on a single host. Missed ticks are
skipped; one capture is admitted at a time. This is not a multi-host fenced scheduler.

Capture and validation run in child processes with a shared deadline. Children
can create immutable artifacts but cannot publish them. The parent owns the lock
and pointer; it checks the deadline again after flushing the pointer's temporary
file. Kernel I/O completion is not a hard real-time guarantee. Killing a worker
prevents synchronous analysis from escaping an async timeout and publishing late.

Run records are journaled before work and after completion. A crash after pointer
publication but before final lifecycle persistence is recovered as published;
an interrupted unpublished run becomes failed. Finished outcomes are replayed in
bounded batches until PostgreSQL accepts them, then marked with `.applied` receipts.
Do not delete these records before replay or while retained evidence references them.

Incident identity, original signal time, processed-signal receipt, incident
transition and outbox enqueue are committed in one transaction. Duplicate signals
are no-ops; conflicting duplicates fail. Older signals cannot reopen a condition
resolved by newer evidence. Freshness emits notifications on opening, severity
change and recovery. Operational rules have persisted opening/recovery debounce
and periodic reminders; unchanged conditions update evidence between reminders.

The webhook sender makes one request at a time, with a 10-second HTTP timeout,
30-second lease, exponential bounded retry delay and ten-attempt budget. Exhausted
items remain in `outbox` with `dead_lettered_at`; inspect them before an explicit
manual retry through the scoped, audited `admin retry` command. A crashed
worker's expired lease is reclaimable. Generic receivers must deduplicate the stable
`Idempotency-Key`; HTTP acceptance followed by a sender crash can cause a retry.
Discord mode sends bounded, mention-free messages but cannot guarantee receiver
deduplication. See [administration and delivery](operations-release.md#operator-workflows). A 2xx response is receiver acceptance, not human acknowledgment.
Redirects are disabled; delivery errors stored in PostgreSQL omit URL credentials.

## Exporter and readiness

```sh
cargo run --release --locked -- serve-dir --base snapshots/operated \
  --policy deploy/config/monitoring.mainnet.toml --poll-interval-seconds 5
```

- `/livez`: process responsiveness, independent of collector health.
- `/readyz`: 200 only for fresh, complete, supported report scope with a healthy
  reload path and, when policy requires it, recent healthy monitor/observer/delivery
  heartbeats. Otherwise 503 with a reason.
- `/status`: readiness reason, checkpoint observation age/freshness, coverage,
  current generation and reload errors. Cached valuation age is null.
- `/report` and `/metrics`: historical last-good evidence remains readable while
  readiness is degraded. Reload work runs outside the request lock.

Fresh means checkpoint age <=`T+D`; warning begins above that boundary and critical
above `2T+D`. Both warning and critical make readiness false. Future timestamps,
invalid timestamps, incomplete diagnostic coverage, scope mismatch and pointer
failures also degrade readiness. A recent report-generation/load time cannot make
an old observation fresh. An empty `expected_market_ids` accepts any nonempty
explicit-market set; production must pin the intended exact set.

## Live development consumer

The default source is the user's free public Mainnet endpoint:

```sh
export SUI_GRPC_URL='https://fullnode.mainnet.sui.io:443'
# Optional for a future provider; no key is needed for the public endpoint.
export SUI_GRPC_API_KEY=''
export SUI_GRPC_API_KEY_HEADER='authorization'
export DIAGNOSIS_DATABASE_URL='postgres://sentinel:YOUR_PASSWORD@localhost:5432/sentinel'
cargo run --release --locked -- probe-grpc
cargo run --release --locked -- live \
  --bootstrap snapshots/live-bootstrap \
  --reconciliation-dir snapshots/reconciliation \
  --scope suilend-main-live --reconcile-every 10000
```

`live` creates a complete pinned GraphQL bootstrap if that directory does not
exist. An existing directory must contain a valid complete schema-2 census.
Set `SUI_GRAPHQL_URL` and optional `SUI_API_KEY` independently of gRPC. A main-market
census is large; the 600-second default `--census-deadline-seconds` must be measured
and adjusted for provider capacity. A failed partial directory is retained as
evidence; use a new directory for another bootstrap attempt.

Use `--max-checkpoints N` for a bounded development run. Restart with the same
scope, config and bootstrap to resume after the last database commit. A dedicated
PostgreSQL session lock prevents competing live coordinators while connected.
Development startup can run migrations. With `SENTINEL_AUTO_MIGRATE=false`,
startup instead checks the required schema; production migrations use a separate role.

The command emits JSON `bootstrap_ready` and `checkpoint_committed` log events.
Checkpoint events separate normalization and commit duration from chain-timestamp
lag; these are not provider-availability latency measurements. The
[full-market validation](reviews/2026-11-02-main-market-validation.md) passed bounded
recovery and reconciliation but found that clock-triggered whole-market evaluation
cannot keep pace with Mainnet. Do not treat this consumer as production real-time
monitoring. That run measured 435–497-second captures; the later R1 census took
432.57 seconds. The selected QuickNode full census took 1,341.65 seconds.
The main-market monitoring policy now allows 2,400 seconds; remeasure
provider capacity before changing the interval or deadline.

The official SDK identifies main market
`0x84030d26d85eaa7035084a057f2f11f701b7e2e4eda87551becbc7c97505ece1`.
The focus uses SUI (`0x2::sui::SUI`) and **native** USDC
(`0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC`),
not bridged wUSDC. Full main-market inventory is retained; each evaluation carries
`sui_collateral_native_usdc_debt_focus`, based on the presence of those asset legs.
Other assets in a matching obligation are not discarded. Identity sources and
package hashes are in [SDK evidence](evidence/sdk-live-scope-2026-11-02.json).

The source uses official `sui-rpc` 0.4.0 with unfiltered checkpoints, small read
masks, two buffered live items and one-checkpoint backfill pages. The SDK handles
subscription reconnection and historical backfill. The application checks sequence
and predecessor digest, then commits objects, indexes, evaluations, raw relevant
effects and watermarks together. Progress-only subscription messages never advance
the database. Requested object contents are pinned to effects output versions and
checked against their digests; missing type metadata cannot silently hide creations.

Sui subscriptions start at the tip and need historical backfill to resume. The
[official gRPC documentation](https://www.docs.sui.io/develop/accessing-data/grpc/what-is-grpc)
and [Rust SDK](https://github.com/MystenLabs/sui-rust-sdk) describe this contract.
The application uses the persisted checkpoint as its authority after restart.
If source history has expired, it stops; configure an archive-capable provider or
explicitly establish a new bootstrap scope. It never silently jumps to latest.

Reconciliation pauses advancement at a committed checkpoint, captures an
independent complete GraphQL census at that checkpoint, and compares live objects,
versions, contents and counts. Failures stop the live command for investigation.
Backpressure during the census may disconnect the subscription; restart/backfill
recovers from the database watermark. `chain_streams.reconciled_checkpoint` records
success. Time-based scheduling and automatic repair remain backlog items.

To move to BlockVision or another provider, change `SUI_GRPC_URL` and its key/header
settings. `authorization` adds a Bearer prefix; another configured header receives
the raw key. Provider URL path conventions, historical availability and quotas
must be checked against that provider's current contract. Never print credentials.

## State, correctness and performance

`follow-checkpoints` also accepts normalized `<sequence>.json` files for offline
replay/testing, using the same adapter and writer. Its input is capped at 32 MiB;
missing sequence files wait rather than skip. `--once` stops at the first missing
file, and `--reconcile PATH` compares a pinned complete census and exits.

`chain_streams` holds committed/evaluated checkpoints; `chain_frames` retains input
identity and relevant source evidence; `chain_objects` keeps versions and tombstones;
`position_dependencies` indexes reverse dependencies; `position_evaluations` holds
cached results. Wrapped objects may return; permanently deleted IDs cannot.
Package or unsupported-schema changes stop the affected stream before committing.

Market/reserve, clock, deployment and capability changes invalidate dependent
positions even when an obligation itself has not changed. Evaluations reuse shared
immutable market inputs and write in batches of 256. The result still has
`fresh_risk: null`, unknown valuation time and `execution_eligible: false`.
Reevaluating cached totals does not implement verified prices, interest or liquidation.
Checkpoint/provider authenticity is trusted in development; local signature
verification is not implemented. The bounded full-market benchmark found a
6.48-second median commit time and inability to keep pace; sustained catch-up
qualification remains open.

### Why bounded queues rather than making everything lock-free?

An actor is one task owning ordered work; other tasks send it messages. The state
writer uses a bounded Tokio channel and acknowledges a checkpoint only after its
database transaction commits. This keeps progress and dependent state consistent.

**Backpressure means slowing the producer when the consumer cannot keep up.**
When the queue is full, sending waits instead of allocating more memory. If the
upstream stream disconnects while waiting, persisted progress tells the consumer
where to backfill. Checkpoints are not dropped to keep the queue short. The HTTP
sender likewise drains a durable PostgreSQL queue at a bounded rate.

Lock-free structures do not remove provider latency, database contention, or the
need for ordered commits. The exporter uses a short lock to swap immutable `Arc`
presentations; file I/O, validation and rendering happen outside that lock. Sharding
by independent protocol/market is useful if benchmarks show contention. Cross-scope
queries would still need a common completed-checkpoint barrier. Do not shard a
single checkpoint's progress counter independently of its state transaction.

## Deployment and validation limits

`deploy/compose.yaml` mounts a generation directory read-only and runs `serve-dir`.
The optional monitor overlay adds PostgreSQL and the scheduler:

```sh
export SENTINEL_DATA_PATH="$PWD/snapshots/operated"
export SENTINEL_POLICY_PATH=./config/monitoring.mainnet.toml
export SENTINEL_DB_PASSWORD='YOUR_PASSWORD'
# URL-encode the password; use the Compose service hostname from the monitor.
export DIAGNOSIS_DATABASE_URL='postgres://sentinel:YOUR_ENCODED_PASSWORD@postgres:5432/sentinel'
docker compose -f deploy/compose.yaml -f deploy/compose.monitor.yaml up --build -d
```

Create the directory first and make it writable by container UID 65532. Set the
webhook variables above. The base stack is local-only; it has anonymous Grafana
viewing and no shared-service TLS/authentication. Do not treat it as a secured
production deployment. Live gRPC can run as a separately supervised CLI process;
it is not automatically launched by the snapshot-monitor overlay.

Tests use an isolated PostgreSQL database through `SENTINEL_TEST_DATABASE_URL`,
including atomic rollback, replay, leases/retries, scheduling, checkpoint gaps,
wrapping/deletion, dependency invalidation and queue saturation. Source probes are
read-only and bounded. Complete development main-market censuses and local restore
drills are recorded in [release evidence](reviews/2026-11-02-r1-release.md). They
do not establish a production SLA, full QuickNode account-capacity qualification,
verified financial math or a sustained operational soak. Follow the
[Fly release runbook](operations-release.md) for production preparation.
