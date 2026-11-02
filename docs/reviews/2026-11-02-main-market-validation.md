# Full main-market development validation — 2026-11-02

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

The complete main-market census, database bootstrap, bounded sequential recovery,
and independent full reconciliation passed. The continuous production-readiness
gate remains open: the consumer cannot keep pace with checkpoint production.
The tested interval contained clock changes only, so it does not establish
correctness for live position mutations or capability transfers.

Input commit: `f57fd3dddc9252c701b808939d973fcdba050a6f`, followed by the query fix
and timing instrumentation in this change. Exact image IDs, hashes, measurements,
and limitations are in [machine-readable evidence](../evidence/main-market-validation-2026-11-02.json).
Raw data, logs, commands, resource samples and committed frames are retained in
`snapshots/validation-2026-11-02/` locally; that directory is intentionally ignored
by Git. This was a read-only Mainnet observation exercise, with an isolated local
database; no on-chain transactions were submitted.

## Environment and scope

- Docker Desktop 29.2.0, Linux aarch64, 16 VM CPUs and 8,216,473,600 bytes of VM RAM.
- Application and PostgreSQL containers each limited to two CPUs and 2 GiB RAM.
  PostgreSQL 17.11; exporter limited to one CPU and 512 MiB.
- Public GraphQL `https://graphql.mainnet.sui.io/graphql` and public gRPC
  `https://fullnode.mainnet.sui.io:443`, without API keys, for development only.
- Main market `0x84030d26d85eaa7035084a057f2f11f701b7e2e4eda87551becbc7c97505ece1`;
  deployed market version 7, package version 25; 45 reserves.
- Complete inventory retained: 150,164 positions and 145,984 capabilities.
  The incremental projection identified 3,087 positions containing SUI collateral
  and native USDC debt. That is an asset-leg watchlist, not a verified risk score.
- Shared development host, overlapping capture/test compilation and warm caches.
  These are reproducible development measurements, not a production SLO study.

## Measurements

| Operation | Result |
| --- | --- |
| Full capture, checkpoint 340925930 | 496.90 s; published successfully |
| Independent capture, checkpoint 340925943 | 434.77 s; published successfully |
| First capture artifacts | 487.56 MB raw; 641.34 MB including analysis |
| GraphQL requests per capture | At least 5,928; actual attempts/retries uninstrumented |
| Original bootstrap | Still incomplete when killed after 287.66 s |
| Corrected full bootstrap | 119.51 s, including full initial evaluation |
| Sequential checkpoint commits | 340925931 through 340925943, all 13 present |
| Commit duration | Median 6.48 s; sample p95/max 9.60 s, only 13 samples |
| Normalization time | 0.463 s total across the 13 checkpoints |
| Commit-only throughput upper bound | 0.144 checkpoints/s |
| Chain timestamp arrival rate in this interval | 4.10 checkpoints/s |
| Complete reconciliation | Passed in 29.01 s; all 296,151 objects matched |
| Final database size | 3.56 GB, including indexes and unreclaimed row versions |
| Cached report HTTP latency | 4.00 ms p95; 200 requests, eight clients, localhost |
| Metrics HTTP latency | 2.87 ms p95 under the same test |
| Grafana | All 29 provisioned panel queries validated against the report |

The checkpoint sample spans only 3.169 seconds of chain time. Provider availability
latency was not instrumented, so the commit percentile is **not** an availability-
to-diagnosis percentile. Final chain-timestamp lag was 1,291.95 seconds, including
capture/bootstrap time, debugging and deliberate interruptions. It must not be
presented as ordinary steady-state latency.

Ten-second samples saw approximately 40.62 MiB working set for the first capture,
67.84 MiB for the corrected live worker and 943.2 MiB for PostgreSQL. These can miss
short peaks. Recorded cgroup memory peaks were about 808.5 MiB, 475.6 MiB and 2 GiB
respectively; cgroup peaks include filesystem cache, not just process RSS.
Database measurements include the isolated benchmark and test databases.

## Correctness and failure checks

1. Both captures exhausted obligation and capability pagination, matched the
   on-chain obligation table count, passed raw checksum validation and published
   only after validation. No diagnostic calculation failed on the captured inputs.
2. Killing the incomplete original bootstrap left zero committed stream/object
   records. The transaction did not publish a partially loaded market.
3. After three committed checkpoints, a real Docker network disconnect interrupted
   the next worker during its transaction. Checkpoint 340925933 remained committed;
   no position evaluation exposed a different checkpoint.
4. The disconnected worker required explicit termination. The isolated database
   was restarted, retained the same four frames including bootstrap, and the new
   worker resumed at 340925934. Automatic recovery from a stalled database is
   **not** proven by this manual recovery exercise.
5. All 13 subsequent frames have consecutive checkpoint numbers and passed the
   application's predecessor-digest validation. Ingestion and evaluation ended at
   340925943; a separate GraphQL census at that exact checkpoint matched every
   stored live object. Reconciliation advanced its watermark to the same value.
6. Both capability and obligation files were byte-identical across the two census
   checkpoints; the market object version was unchanged. All 13 frames changed
   only the clock. No claim is made about real position/reserve/capability changes.
7. Ownership attribution remained unresolved for 4,191 positions and ambiguous for
   10. These are explicitly represented limitations, despite complete enumeration
   and cached diagnostic coverage.
8. Exporter startup returned 503 with `no_generation`. After publication it served
   the report and metrics but returned 503 with `stale_observation`, correctly
   enforcing the current 360-second freshness threshold against a 497-second run.

## Fixes made during validation

The original evaluation query joined the entire object table to the affected-ID
table before applying `LIMIT 256`. During bootstrap, PostgreSQL lacked statistics
for uncommitted rows. The inspected plan estimated only 43 matching positions and
chose a repeated bitmap scan of the large object table followed by sorting.

The evaluator now pages the affected IDs first, then fetches object bodies by their
keys. It advances across empty batches, preserving behavior when dependencies
refer to absent/deleted positions. A real-PostgreSQL regression test puts 256 absent
IDs ahead of a valid position and verifies that the later position is evaluated.
This changes query cost without weakening checkpoint transaction boundaries.

The live command now emits JSON `bootstrap_ready` and `checkpoint_committed` events,
including normalization/commit durations and chain-timestamp lag. The monitor test
uses the POSIX shell's `kill` builtin, allowing graceful-shutdown verification in
minimal Linux images that lack a standalone `kill` executable.

Validation: 112 Rust tests passed with real PostgreSQL; formatting, Clippy with
warnings denied, maintained-file size checks and dashboard generation checks passed.
The running Grafana query smoke test passed. Rendered browser/UX testing was not
performed.

## Remaining priorities established by this run

1. Fix clock-triggered whole-market evaluation and write amplification. Every clock
   checkpoint currently rewrites all 150,164 cached evaluations. Even the commit-
   only upper bound is about 29 times slower than the measured chain timestamp
   rate. Lock-free memory structures do not remove this database/algorithm cost.
   Preserve ordered atomic commits and explicit valuation provenance when choosing
   dependency granularity, incremental calculations or shared generation metadata.
2. Batch initial object/dependency inserts. The query fix removes repeated scans;
   bootstrap still performs many individual SQL writes. Benchmark separately from
   initial evaluation and independent reconciliation.
3. Replace the unbenchmarked 60-second capture deadline for main-market operation.
   A conservative **development starting point**, subject to repeated measurement,
   is a 900-second deadline and 1,800-second interval with the exact market pinned.
   This intentionally provides slow cached snapshots; it does not meet real-time
   diagnosis goals. Two captures cannot establish p95 duration or provider quotas.
4. Add actual request/retry/throttle/byte counters and phase timings; measure
   provider checkpoint availability separately from chain timestamps. Bound database
   stalls and demonstrate automatic reconnect/recovery rather than manual restart.
5. Extend the real-data window to position changes, native-USDC/SUI activity,
   ownership transfers and lifecycle changes, then repeat complete reconciliation.
   Prove sustained catch-up and storage stability before an unattended deployment.

## Reproduce and inspect

Build the current source with `docker build -f deploy/Dockerfile -t sentinel-validation .`.
Use an isolated PostgreSQL database and private Docker network. The test database
used trust authentication only on that network, with no host database port; this
is not a shared-service deployment configuration.

The retained JSON command files in the local evidence directory record exact Docker
arguments. The equivalent application sequence is:

```sh
sui-sentinel capture-once --out /data/captures \
  --config /config/suilend.toml --concurrency 2 --deadline-seconds 1800
# Resolve /data/captures/current to an immutable bootstrap directory.
sui-sentinel live --bootstrap /data/captures/BOOTSTRAP_RUN \
  --config /config/suilend.toml --scope validation-main \
  --reconciliation-dir /data/reconciliation \
  --reconcile-every 18446744073709551615 --max-checkpoints 3
# Restart the same scope/bootstrap, then process ten more checkpoints.
# Independently capture at the final committed checkpoint.
sui-sentinel capture-once --out /data/reconcile-captures \
  --config /config/suilend.toml --checkpoint FINAL_CHECKPOINT \
  --concurrency 2 --deadline-seconds 1800
sui-sentinel follow-checkpoints --bootstrap /data/captures/BOOTSTRAP_RUN \
  --config /config/suilend.toml --scope validation-main \
  --input-dir /data/unused --reconcile /data/reconcile-captures/RECONCILE_RUN
```

Set `DIAGNOSIS_DATABASE_URL` for the isolated database. Never perform fault injection
against an existing shared database. Historical checkpoints may eventually fall
outside public-provider retention; capture fresh data or configure retained history.

The validation database is stopped with its named volume retained. The isolated
read-only dashboard remains at <http://127.0.0.1:13000>, Prometheus at port 19090,
and exporter at port 19108. It displays the retained snapshot and correctly reports
its age. Your original dashboard stack was left running unchanged. Stop only these
three validation containers when finished:

```sh
docker stop sentinel-validation-grafana-20261102 \
  sentinel-validation-prometheus-20261102 sentinel-validation-exporter-20261102
```
