# Cached diagnostics dashboard

Grafana reads Prometheus metrics from `serve-dir`, which watches an immutable
generation directory and loads new validated reports without restarting. It
performs no collection or risk calculations. Scheduled capture and live ingestion
are described in [continuous monitoring](continuous-monitoring.md).

The dashboard shows census coverage, ownership gaps, cached health buckets,
asset exposure, top directly attributable wallets, and illustrative stress
scenarios. Values remain cached and unverified. Snapshot age measures chain
observation time; refreshing Grafana does not refresh the underlying valuations.

The operations overview shows policy readiness/freshness, monitor and worker
health, disk space, capture duration/failures, delivery queue age/dead letters,
spool backlog and reload health. Freshness colors follow the deployed policy;
observation age and reload age remain separate. Missing telemetry is unavailable,
not zero. Readiness is false for stale/incomplete observations, scope mismatch,
reload failure or required unhealthy monitor workers. Historical report panels
continue to show last-good evidence. Rendered/operator UX validation remains open.

## Start a local viewing stack

With Docker running, generate a fixture report in a new generation directory:

```sh
mkdir -p snapshots/dashboard-demo/demo
cargo run --release --locked -- report --input tests/data/elixir \
  --out snapshots/dashboard-demo/demo/analysis
printf demo > snapshots/dashboard-demo/current
export SENTINEL_DATA_PATH="$PWD/snapshots/dashboard-demo"
docker compose -f deploy/compose.yaml up --build -d
```

The fixture is an old experimental-market census with 90 positions, so readiness
correctly reports stale. It remains useful for viewing cached report panels.
The output directory must be new; the absolute generation base must exist.
Compose mounts the entire directory read-only and refuses missing host paths.
Allow a 15-second scrape after startup.

For scheduled captures, point `SENTINEL_DATA_PATH` at the `monitor --out` directory.
Use the optional monitor overlay only after configuring the database and receiver
in [operating instructions](continuous-monitoring.md#deployment-and-validation-limits).

- [Grafana dashboard](http://localhost:3000/d/sui-sentinel-snapshot)
- [Prometheus](http://localhost:9090)
- [Metrics](http://localhost:9108/metrics) and [report](http://localhost:9108/report)
- [Liveness](http://localhost:9108/livez), [readiness](http://localhost:9108/readyz)
  and [status](http://localhost:9108/status)

All host ports bind to loopback. Grafana allows anonymous Viewer access in this
local setup; shared deployments require authentication and TLS. Provisioned
configuration is authoritative: edit the generator to maintain dashboard panels.

## Incident investigation

Use [the bounded operator page](mcp.md#private-http-and-the-operator-page) to read
incidents and evidence from the configured database scope. It accepts the same
read-only token and query limits as MCP. Start with status, copy an incident's
`evidence_id`, then inspect that evidence and retain its generation/hash in notes.
Grafana shows the latest published capture; the query service can show a different
incremental checkpoint. Compare the explicit checkpoints before combining them.
Scoped Grafana links, full timelines and rendered operator acceptance remain open.

## Interpretation

- A complete census covers its declared market scope, not all depositors or an
  official registry. Deposits outside obligations are not included in collateral.
- Cached health below 1 does not establish current liquidation eligibility.
  Cached debt is not expected loss, and scenario recovery assumptions are explicit.
- Missing owners, unsupported diagnostics and unavailable scores stay unknown.
  Object-owner addresses are not direct wallet addresses.
- Distinct wallets and top-20 aggregates are per market and must not be summed
  into a global distinct-wallet count or concentration ranking.
- USD metrics use Prometheus floating-point samples. Use JSON/NDJSON artifacts
  for exact decimal amounts and per-position analysis.
- Asset labels are display hints, not price inputs or endorsements. Full coin
  types preserve identity; a package suffix distinguishes duplicate symbols.
- Market filters apply to position/exposure panels. Provenance and operational
  readiness describe the whole report and configured serving scope.
- Repeated scrapes of one saved report do not create fresh market history.
  Source failure, exporter failure and Prometheus query errors are distinct states.

## Update, maintenance and shutdown

A successful `capture-once` atomically updates `current`; `serve-dir` reloads it
on the poll interval. A missing/invalid candidate leaves the previous report
readable and readiness degraded. No exporter restart is required for publication.
Do not modify files inside a published generation.

The mounted generation directory must remain available. Grafana and Prometheus
use named volumes; containers continue after a terminal closes and restart when
Docker returns unless explicitly stopped. They still require an available host.

```sh
docker compose -f deploy/compose.yaml logs --tail=50 exporter prometheus grafana
docker compose -f deploy/compose.yaml down
```

Keep `SENTINEL_DATA_PATH` set for Compose commands. `down --volumes` deletes
local dashboard/Prometheus storage and is not required for normal shutdown.
Back up raw evidence and PostgreSQL independently before production use.

| Script | Purpose |
| --- | --- |
| `deploy/grafana/generate.py` | Reproducible panel definitions and JSON |
| `deploy/grafana/check.py` | Source-report/query checks against a running stack |
| `.github/scripts/check_file_sizes.py` | 400-line maintenance limit |

For local development without Docker:

```sh
cargo run --release --locked -- serve-dir --base "$SENTINEL_DATA_PATH"
python3 deploy/grafana/generate.py --check
python3 .github/scripts/check_file_sizes.py
```

For running-stack verification:

```sh
curl --fail --silent http://localhost:9108/metrics > /tmp/sentinel-metrics.txt
docker compose -f deploy/compose.yaml exec -T prometheus \
  promtool check metrics < /tmp/sentinel-metrics.txt
python3 deploy/grafana/check.py \
  --report "$SENTINEL_DATA_PATH/$(cat "$SENTINEL_DATA_PATH/current")/analysis/report.json"
```

CI builds the stack and fixture generation. Query correctness does not replace
rendered UI checks for degraded states, keyboard access, viewport layout and
operator investigation workflows; see [release gates](readiness.md) and
[the UX roadmap](roadmap.md#grafana-operator-experience).
