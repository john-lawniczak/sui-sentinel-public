# sui-sentinel

Rust Suilend position census and reproducible diagnostics for Sui mainnet.

Capture all obligations and visible owner capabilities in selected markets at one
checkpoint. Raw Move JSON, object versions/digests, reserve configurations, chain
Clock, and deployment identity are retained. Buffered NDJSON avoids holding the
whole position book in memory. Requests share a connection pool and bounded
concurrency; pages remain sequential within each collection.

**Current calculations are explicitly unverified diagnostics of cached onchain
valuations. They are not freshly accrued checkpoint health factors.** The deployed
source/oracle research gaps remain open. Reports emit no overall grade,
estimate of actual losses, or assertion that a position is currently liquidatable. See
[research](docs/suilend-notes.md) and [snapshot design](docs/snapshot-design.md).

**Early preview · Apache-2.0.** Useful today for reproducible cached diagnostics
and read-only MCP investigation. Production operations and fresh financial risk
remain under qualification. APIs and schemas may evolve before 1.0; see
[contributing and compatibility](CONTRIBUTING.md), [security reporting](SECURITY.md)
and the [roadmap](docs/roadmap.md).

Try the bundled historical fixture without provider credentials or PostgreSQL
(the first build downloads Cargo dependencies):

```sh
mkdir -p snapshots/oss-demo
cargo run --locked -- report --input tests/data/elixir --out snapshots/oss-demo/analysis
cargo run --locked -- serve --report snapshots/oss-demo/analysis/report.json --listen 127.0.0.1:9108
```

Open [the report](http://127.0.0.1:9108/report). This demo uses historical cached
data; choose a new output directory if you run it again. See the
[contributor guide](CONTRIBUTING.md) for database-backed tests and
[Grafana setup](docs/dashboard.md) for a visual demo.

```text
src/                     Rust implementation
  ingest/                GraphQL transport and snapshot collection
  analysis/              Risk math, stress models, and offline reports
  export/                HTTP endpoints and Prometheus metrics
  runtime/               Scheduling supervision, child deadlines, run recovery
  diagnosis/             Atomic incidents and leased webhook delivery
  protocols/             Protocol decoding and dependency contracts
  state/                 Durable checkpoint projections and bounded writer actor
  query/                 Scoped, bounded reads over committed state
  mcp/                   Read-only MCP tools and operator investigation page
  *.rs                   CLI, shared types, configuration, and artifact I/O
tests/                   Regression tests and captured fixtures
deploy/                  Runtime configuration and dashboard stack
  config/                Deployment identity, stress policy, environment example
  grafana/               Dashboard source, checker, and provisioning
docs/                    Architecture, operations, and research
  reviews/               Dated code-quality reviews
  evidence/              Captured research and validation evidence
.github/                 CI workflows and maintenance scripts
```

Run commands from the repository root. Deployment configuration defaults to
`deploy/config/suilend.toml`; the built-in stress policy is embedded from `deploy/config/stress.toml`.
`monitor` defaults to `deploy/config/monitoring.mainnet.toml`: a measured provisional
60-minute interval and 40-minute deadline for the exact main market. `serve-dir`
and local Compose default to the generic `monitoring.toml`; pass the mainnet
policy explicitly when serving an operated main-market capture. Qualify the
selected provider account before unattended production deployment. See
[continuous monitoring](docs/continuous-monitoring.md) for live gRPC setup,
recovery, notification delivery and concurrency design.
Ignored local directories `target/` (Cargo build output) and `snapshots/` (retained
captures and reports) are separate: `cargo clean` must never remove snapshot data.

```sh
cargo build --release --locked
cargo run --locked -- check-config
cargo run --release --locked -- markets
mkdir -p snapshots
cargo run --release --locked -- snapshot --out snapshots/mainnet
cargo run --release --locked -- report --input snapshots/mainnet --out snapshots/replay
```

`capture-once` runs capture and validation in killable child processes under a
single-host lock. It rejects expired, incomplete or corrupt candidates and
atomically replaces `<out>/current` only after validation. Lifecycle outcomes are
saved under `<out>/runs`; interrupted runs are recovered on the next attempt.
The last good generation remains readable after failures.

```sh
cargo run --release --locked -- capture-once --out snapshots/mainnet --deadline-seconds 2400
cargo run --release --locked -- serve-dir --base snapshots/mainnet --poll-interval-seconds 5
```

`serve-dir` uses `/livez` for process responsiveness and `/readyz` for checkpoint
freshness, complete diagnostic coverage, expected scope and reload health.
`/status` explains degraded readiness. Historical `/report` and `/metrics` remain
readable when readiness returns 503. Observation age is independent of load time;
cached valuation age remains unavailable. `serve --report FILE` remains a fixed
artifact viewer without this operational readiness contract.

`monitor --out DIR --scope NAME` reads monitoring policy and schedules captures
without overlapping or accumulating missed jobs. It independently evaluates
freshness, replays persisted run outcomes and sends incident webhooks. Configure
`DIAGNOSIS_DATABASE_URL`, `SENTINEL_WEBHOOK_URL` and optionally
`SENTINEL_WEBHOOK_TOKEN`. Incident transitions, signal receipts and outbox records
commit together. Delivery is at least once; generic receivers must deduplicate
`Idempotency-Key`. Direct Discord can duplicate messages after lost acknowledgments;
see [delivery operations](docs/operations-release.md#operator-workflows).

`live` consumes Sui gRPC checkpoints into PostgreSQL from a complete GraphQL
bootstrap, resumes from committed progress, repairs gaps through SDK backfill,
and periodically reconciles an independent pinned GraphQL census. It monitors
Suilend's Main Lending Market with a SUI-collateral/native-USDC-debt focus label.
It retains the full market inventory so positions entering the focus aren't lost.
Dependency changes reevaluate **cached diagnostics**; fresh financial risk stays
unsupported. The public endpoint is a development source, not a production SLA.
See [live setup and limits](docs/continuous-monitoring.md#live-development-consumer).

`mcp --scope NAME` exposes read-only committed state through stdio or private
Streamable HTTP, with generation-pinned pages, evidence hashes and explicit
confidence. Its operator page supports incident investigation over the same bounded
query service. See [MCP setup and trust boundaries](docs/mcp.md). Only the Suilend
cached adapter is implemented; LLM job automation and other adapters remain open.

`snapshot` defaults to the configured main market. Repeat `--market 0x...` to
select specific markets. `--all-markets` captures **all discovered instances of
the configured package's LendingMarket type**, including third-party and
experimental markets; this is not an official market registry. Reports keep
markets separate. A census covers obligations, not every depositor holding
cTokens outside an obligation.

Use `--checkpoint NUMBER` for a retained historical checkpoint, `--concurrency 4`
(default; 1–32), and `--page-size 50` (default; 1–50). The public endpoint limits
pages to 50. `--max-pages 2` explicitly requests a sample of each collection;
truncated collections produce `complete: false`. A successful sample command is
not a complete census. Partial ownership joins are labeled unresolved even when
the visible capability scan is exhausted. Multiple capabilities are retained and
marked ambiguous; their debt is not attributed to a single wallet. A single
observed capability is attributed to a direct wallet only once that market's
capability scan is itself exhausted; under an incomplete scan it is labeled
provisional, preserving the observed capability while excluding it from wallet
totals until an unseen second capability can be ruled out.

Set `SUI_GRAPHQL_URL` to use a provider endpoint and optionally `SUI_API_KEY` for
Bearer authentication, or set `SUI_API_KEY_HEADER=x-token` for QuickNode.
Use `SUI_SNAPSHOT_CHECKPOINT_LAG=200` to select a checkpoint behind provider head
when indexing lags. It is selected once; explicit `--checkpoint` takes precedence.
The offset adds observation age and must be included in the freshness budget.
Keep credentials separate from URLs. The development endpoint default is `https://graphql.mainnet.sui.io/graphql`.
See `deploy/config/env.example` for environment settings. Variables are read
directly from the shell; environment files are not loaded automatically.
Public endpoints have rate limits and finite checkpoint retention. A failed or
expired crawl never falls back to latest state. Use a provider with sufficient
retention for large captures. No wallet or transaction submission is needed.

The output directory must not exist. Outputs:

- `manifest.json`: written after successful raw capture; records checkpoint,
  scope, per-market expected/observed counts, and pagination exhaustion.
- `<market>.obligations.ndjson` / `<market>.caps.ndjson`: lossless observations.
- `analysis/positions.ndjson`: one row per obligation, immediate ownership and
  cached LTV, effective close-LTV, health factor, and liquidation buffer.
- `analysis/report.json`: cached health buckets and debt exposure, asset
  concentrations, largest debt positions/wallets, reserve parameters, data
  gaps, and provenance (adapter version, chain/package identity, stress
  policy hash, generation time).
- `capture-config.json` and raw `checksums.json`: schema-2 captures bind the
  manifest, all raw object files and deployment configuration. Replay verifies
  the full set before analysis. Schema-1 captures remain explicit legacy inputs.
- `analysis/checksums.json`: sha256 of `positions.ndjson` and `report.json`,
  written after both are flushed and synced. Detects local corruption; it
  does not establish a cryptographic link to any upstream provider's own
  object digests. Newly generated reports require this file; only legacy reports
  without the new artifact revision may load unbound with a warning.

Missing `manifest.json` or required schema-2 `checksums.json` means the capture
is incomplete and cannot be replayed or published.
Missing `report.json` means analysis did not finish. Existing outputs are never
reused or overwritten. Completion markers are published atomically on filesystems
that support hard links. Offline `report` streams raw records and reproduces the
same output without network access. Raw balances retain protocol scales as decimal
strings; USD diagnostics use checked decimals and reject unsupported ranges.
Current report comparisons exclude `provenance.generated_at`, which records
wall-clock report generation, not the age of cached valuations. Position
outputs remain byte-reproducible; report checksums can change with generation time.

A full live census at checkpoint 335839129 captured **159,717 obligations across
26 market type instances**, including 149,923 in the main market. The reviewed
report, including illustrative stress scenarios, replays deterministically; one
local release run took 2.020 seconds and about 179 MiB peak resident memory.
See [capture evidence](docs/evidence/snapshot-2026-10-19.json)
and the [code review](docs/reviews/2026-10-19.md). This does not establish fresh
liquidation values or an overall protocol grade.

A real 90-obligation capture, including a nonzero-debt position and an unresolved
owner, is committed under `tests/data/elixir`. It supports transport, accounting
boundary, ownership, and deterministic replay tests—not fresh onchain HF verification.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
```

Architecture: `ingest → raw artifacts → analysis::report` calls pure
`analysis::risk` and `analysis::stress` calculations. The independent `export` adapter
serves saved reports to Prometheus and Grafana without invoking collection or calculations.
See [module boundaries and maintenance](docs/snapshot-design.md#module-boundaries)
for how to extend the pipeline and keep its documentation current.

Grafana is provisioned with a snapshot dashboard, Prometheus, and a read-only Rust
exporter. Start Docker, then point the stack at a generation directory containing `current`:

```sh
export SENTINEL_DATA_PATH="$PWD/snapshots/mainnet"
docker compose -f deploy/compose.yaml up --build -d
```

Open [Grafana](http://localhost:3000/d/sui-sentinel-snapshot). No login is needed for
the local viewer. See [dashboard setup](docs/dashboard.md) for the bundled fixture,
standalone exporter, updating a snapshot, and shutdown. Source files have a
**400-line limit enforced in CI**; split modules by responsibility before exceeding it.

Fresh risk calculations require verified
current source/price paths and an independent valuation-aligned reference first.

New reports also run the versioned [economic stress model](docs/economic-stress.md)
over individual cached asset exposures. `report --stress-config FILE` selects a
scenario policy; the default is embedded from `deploy/config/stress.toml`. Illustrative shortfalls
are labeled separately from economic scores, which remain unavailable with cached
inputs. The dashboard uses consistent HF colors, prominent snapshot age, explicit
coverage sections, and coin-type names with package identifiers.

Release candidate operations: [Fly/Discord/QuickNode runbook](docs/operations-release.md),
[dated R1 evidence](docs/reviews/2026-11-02-r1-release.md), and
[remaining release gates](docs/readiness.md), and [future capabilities](docs/roadmap.md).
Production activation remains pending. Earlier reviews are [historical evidence](docs/reviews/README.md).

This public repository starts with a reviewed source snapshot and clean history.
See [publication scope](PUBLICATION.md) for provenance and historical evidence limits.
