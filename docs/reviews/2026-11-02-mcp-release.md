# MCP, provider qualification and deployment preparation — 2026-11-02

This follow-up closes concrete implementation and local-validation gaps. It does
not qualify the full platform for production. Machine-readable results and artifact
hashes are in [the evidence record](../evidence/2026-11-02-mcp-release.json).
The current activation backlog is [readiness](../readiness.md); future capabilities
are in [the roadmap](../roadmap.md). `temp.md` was retired into those maintained files.

## Delivered behavior

- Eight read-only MCP tools over scoped PostgreSQL queries, using the official
  pinned SDK. Local stdio and private Streamable HTTP support tested `2026-07-28`
  and `2025-11-25` requests. Public OAuth/per-user tenancy is not implemented.
- Repeatable-read answers, generation-pinned state pages, decimal strings,
  read-only DB connections, strict admission/row/byte/time budgets, cancellation,
  redacted errors and evidence hashes. Unknown valuation remains unknown.
- An immutable incident change journal committed with incidents, scope revisions
  and outbox writes. Per-scope cursor serialization prevents out-of-order commits;
  clients can follow updates to existing incident IDs. Rollback, deletion,
  immutable evidence and expired-history behavior are tested. Existing history
  requires an explicit baseline; journal archival/capacity remains a gate.
- A token-protected operator investigation page over the same query service,
  with scope/checkpoint/confidence context, text-only evidence rendering, bounded
  requests, CSP and in-memory credentials. HTTP/security and syntax checks pass;
  rendered/accessibility acceptance remains open because the supported browser
  runtime fails during bootstrap (`Cannot redefine property: process`). A kernel
  reset did not resolve it. No screenshots or human UX results are claimed.
- A runtime adapter registry with honest capabilities. Only Suilend cached
  diagnostics are implemented. Shared discovery/report decoupling and another
  real protocol adapter remain future work.
- Cached Suilend calculations no longer rewrite the full book on clock ticks or
  market mutations whose reserve references are unchanged. Observation advances
  separately from the retained calculation checkpoint. Required schema/deployment
  and reserve-reference checks still run; fresh-price/accrual math is not implied.

## Measured evidence

| Check | Result and boundary |
| --- | --- |
| Full QuickNode main-market census | 150,166 obligations, 145,986 visible capabilities, complete at checkpoint 340955467; 1,341.65 s, 5,928 requests, 503,240,989 response bytes; zero retries/throttles. |
| Provider failure behavior | Head checkpoint scope unavailable in an initial attempt; a 900 s attempt timed out. Neither published. A later explicitly pinned historical checkpoint passed validation/publication. |
| Cached full-book benchmark | 150,164-position restored book, 100 synthetic clock + unchanged-market updates; 41.92 checkpoints/s, p95 commit 26.95 ms. Two concurrent readers served 200 queries, p95 10.87 ms. Cached evaluation hash unchanged. |
| Local schema-8 recovery | Backup 6.51 s, restore 10.97 s; projection fingerprint matched, exact checkpoint replay was a no-op, incident journal test marker survived. Local same-host storage; not hosted PITR. |
| Discord acceptance | One real TEST ONLY incident/outbox message, one send attempt, Discord success response and durable delivered state. Human channel acknowledgement and hosted recovery remain unverified. |
| Fly preparation | App created in `iad` using `fly launch --no-deploy`; config validated, QuickNode and Discord credentials staged. No Machines, database or production service activated. |
| Automated validation | 135 Rust tests with real PostgreSQL, two Python tests, formatting, Clippy, JS syntax, file limits and Grafana generation checks passed. Monitor/operator/reader role and journal grants checked. |
| Security/runtime | Final source-pinned non-root Linux image listed eight tools, queried the full restored book, resumed restored incident events and revalidated the full QuickNode artifacts. Pinned Trivy dependency/runtime scans reported zero high/critical advisories; evidence includes SBOM hashes. This is not a security audit. |

The performance workload excludes provider ingestion, changed financial inputs,
real position/capability mutations and full dependency fan-out. Do not extrapolate
it into a live 5–15-second SLA. The original database had write-amplification bloat;
the restored benchmark copy is compacted (~1.38 GB), and release/debug measurements
are not a controlled before/after comparison. Synthetic checkpoint copies must
never resume mainnet ingestion.

The successful provider capture used the previous R1 candidate image; its precise
build and raw-input provenance are retained. The new binary separately revalidated
those artifacts. The final runtime is built from `0e02205dba0ac6f335a69fc1664e4be82af4058e`; its
image ID is `sha256:38b3d337fd0f033254fe28d9c7237d35a71dba427139373f6de3cd41b1dab866`.
Binary/SBOM/scan hashes and two final cursor regression checks are in the evidence
record. No dirty-tree capture is relabeled as a committed build. The image was
built locally, not published to a production registry or deployed.

## Operational changes and remaining decisions

The selected QuickNode account required more than the previous 900-second budget.
The provisional main-market policy is now a 3,600-second interval and 2,400-second
deadline. Warning/critical observation ages are 6,000/9,600 seconds. This is hourly
cached monitoring; policy freshness is not financial valuation freshness. Repeat
hosted/account-capacity runs before accepting these budgets for unattended use.

`SUI_SNAPSHOT_CHECKPOINT_LAG=200` avoids selecting an immediately unavailable head
checkpoint on the measured account. The choice is pinned once; an explicit
checkpoint takes precedence. It adds observation age, offers no automatic provider
failover and cannot repair missing historical data. Account retention/costs remain
external qualification items.

Fly Managed Postgres preparation and a concrete PITR drill are in
[the database plan](../fly-postgres.md). Provisioning remains paused pending budget
approval, as requested. Hosted database roles/read tokens, encrypted off-host
capture backups, measured PITR/RPO/RTO, independent monitoring and the seven-day
soak are not complete. Production deployment remains paused.

Fresh financial eligibility stays disabled. Remaining evidence includes deployed
rule/oracle verification, exact arithmetic/accrual fixtures, real mixed-stream
throughput/reconciliation, rendered operator acceptance, a second protocol adapter,
actual assistant-client acceptance and supervised LLM jobs/cost/citation evaluations.
A working MCP read interface does not establish those capabilities.

## Migration and verification order

1. Back up the target database; use the migration identity to apply through
   migration 8. Migration 6 removes only cached-Suilend clock dependencies;
   migrations 7/8 add incident generations and immutable changes.
2. Reapply `deploy/ops/grants.sql`, then run `check-query-grants.sql` on an isolated
   acceptance database. Runtime identities must not own migration privileges.
3. Deploy only a committed, source-pinned approved artifact after operational
   gates and authorization. Configure a separate reader identity for MCP.
4. Follow [MCP setup and investigation](../mcp.md), including current-state page
   generations and the distinct durable incident-feed cursor contract.
5. Run the [remaining qualification gates](../readiness.md) with retained failure,
   recovery, provider and time-based evidence; keep unsupported capabilities off.
