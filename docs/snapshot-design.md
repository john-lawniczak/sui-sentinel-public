# Snapshot architecture and maintenance

The implemented pipeline captures checkpoint-consistent observations and derives
explicitly cached diagnostics. Fresh protocol valuation remains gated on verified
deployed rules and oracle mappings.

## Module boundaries

| Module | Responsibility |
| --- | --- |
| `main` | CLI arguments, environment variables, orchestration |
| `config` | Deployment configuration and local validation |
| `ingest::graphql` | HTTP transport, checkpoint-scoped queries, pagination integrity |
| `model` | Serializable chain observations and structural validation |
| `ingest::snapshot` | Market selection, concurrent collection, completeness manifest |
| `storage` | Buffered artifact I/O and atomic completion markers |
| `analysis::risk` | Pure calculations over supplied observations; no I/O |
| `analysis::stress` | Pure per-position scenario arithmetic, coverage gating and economic score |
| `analysis::report` | Offline ownership joins and diagnostic aggregation |
| `analysis::report::types` | Versioned report schema shared with presentation adapters |
| `export` | Validate and pre-render one immutable report for HTTP/Prometheus |
| `deploy/` | Optional Grafana/Prometheus provisioning and container packaging |
| `deploy/config/` | Deployment identity and versioned stress policy TOML |
| `deploy/grafana/` | Dashboard source, generated JSON, integration check, provisioning |
| `.github/scripts/` | CI maintenance checks |

Shared `config`, `model`, `decimal`, and `storage` modules support these boundaries.
Ingestion is independent of analysis and export; offline analysis does not depend
on the transport module. Export reads the saved report schema.

Keep network and filesystem access outside calculations. Add adapters or exporters
at those boundaries; introduce shared traits when a second implementation needs
them. Preserve raw observations so reports can be regenerated without network
access. Validate artifact schema versions when formats change, and make sampling,
ownership coverage, and valuation confidence explicit in every consumer.

The README owns supported commands and setup. This document owns architecture and
operational guarantees. `suilend-notes.md` owns protocol findings and unresolved
verification questions. Dated notes under `reviews/` and captures under `evidence/`
retain provenance; real fixtures support regression tests. Update documentation with behavior changes,
remove superseded plans and unused configuration, and keep large generated captures
outside git. Historical plans remain available in git history.

Maintained text files have a **400-line maximum**, including tests, documentation,
and configuration. `python3 .github/scripts/check_file_sizes.py` checks tracked and new
nonignored files; CI enforces it. Split by cohesive responsibility, not arbitrary
line ranges. Exceptions are `Cargo.lock`, generated
`deploy/grafana/dashboards/snapshot.json`, and captured artifacts under
`tests/data/` and `docs/evidence/`. Keep those directories for evidence, not source.
The dashboard's maintained source is `deploy/grafana/generate.py`; CI checks
its output is current. Add an exception only for an artifact with a documented
generator or capture provenance.

The exporter consumes `report.json` only, without reading raw positions or making
chain requests. Startup validates schema, status, provenance, counts and label
bounds, then pre-renders JSON and metrics. Requests clone shared byte buffers.
Reports are limited to 8 MiB, 256 markets, and 1,024 assets per exposure map.
Wallet/obligation addresses never become Prometheus labels. Restart the exporter
to load another report. Collection completeness, cached valuation confidence,
and HTTP availability are separate signals; `/healthz` only means a report loaded.
Grafana uses instant queries and source checkpoint time; repeated scrapes do not
represent new snapshots. See [dashboard operations](dashboard.md).

## Consistency and discovery

All reads use `checkpoint(sequenceNumber: C) { query { ... } }`, including market
inventory, UpgradeCap, Clock, obligation table fields, and owner capabilities.
The endpoint must report Sui mainnet's chain identifier. The configured UpgradeCap
package/version and main market schema/type are checked at the chosen checkpoint.
Each response must return that checkpoint number. There is no latest-state fallback.

Discovery filters the original package's generic LendingMarket type. This includes
unendorsed markets; select explicit IDs for an official reporting scope. The default
is the configured main market. An embedded ObjectTable UID is not a top-level object:
its dynamic fields must be queried through `address(address: table)`. Read each
field's MoveObject value, not its wrapper. Every obligation's type, market ID, reserve
index and coin type are checked. Exhausted collections must equal the market table
size. Duplicate IDs/cursor loops, GraphQL errors, and missing child data abort.

Owner caps are scanned separately by exact generic type and joined by obligation ID.
Exactly one visible cap with an immediate AddressOwner/ConsensusAddressOwner is
a direct wallet mapping. Multiple caps are preserved as ambiguous ownership and
excluded from single-wallet debt attribution; public source permits admin recovery
caps through `new_obligation_owner_cap`.
ObjectOwner is retained as a non-wallet owner. Missing/wrapped caps remain unresolved;
the code never guesses from transaction senders. Census completeness is distinct
from ownership resolution and valuation confidence.

Sources verified against the live GraphQL schema on 2026-10-19:
- https://www.docs.sui.io/develop/accessing-data/graphql/query-with-graphql
- https://docs.suilend.fi/ecosystem/suilend-sdk-guide

## Calculation boundary

`analysis::risk::assess` has no I/O. It divides stored unweighted debt by stored collateral
for LTV, stored unhealthy-borrow value by collateral for effective close-LTV, and
stored unhealthy-borrow value by stored weighted debt for HF. Equality at HF=1
is not assigned to the below-1 bucket. Borrow records with zero cached weighted
debt are unknown rather than marked debt-free. Denominator-zero ratios are null.

These are diagnostics of each obligation's last stored valuations. Even if all
objects are read at one checkpoint, those valuations can originate at different
times. Aggregated cached USD totals are explicitly named accordingly. Current
reserve config is exported independently; it is not silently used to relabel older
cached valuations. The implementation does not compound debt or refresh prices using
unverified version-20 rules against the version-25 deployment.

Scaled USD strings are normalized before `rust_decimal::Decimal::from_str_exact`;
raw u256 amounts are retained as strings. Unsupported diagnostic values are not
coerced to zero. Arithmetic overflow fails analysis. Ratios can round at Decimal
precision and do not claim Move execution equivalence.

## Resource use and recovery

One shared reqwest connection pool and semaphore bound active requests. Sequential
cursors are never guessed or split. Independent cap and position streams run
concurrently. Timeouts and bounded exponential retries cover connection/timeouts,
HTTP 429, server errors, and interrupted response bodies. Decompressed responses
are limited to 64 MiB, and numeric Retry-After is honored up to ten seconds. Authorization headers are sensitive and redirects
are disabled. Endpoint credentials are not recorded in artifacts.

Records use 256 KiB buffered files; only a page of raw positions is needed at a
time. Hash sets retain IDs/cursors for completeness checks. Offline reports retain
one market's cap join and deduplication sets, wallet debt totals, and a bounded
top-20 position debt ranking.
Artifacts are create-new only. Manifest/report completion markers are serialized
to a sibling temporary file, flushed/synced, then atomically hard-linked into place;
failures retain raw partial files without claiming completeness. There is no resume
support yet. Choose a fresh output directory for retries.

Future work: validated effective pricing/interest engine, same-valuation simulation
verification, stress scenarios over fresh valuations and executable liquidity,
reserve reconciliation, and calibrated risk grades. Those outputs remain unknown.
