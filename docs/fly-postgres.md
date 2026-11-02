# Prepared Fly Managed Postgres and recovery plan

Status, 2026-11-02: preparation is authorized; **paid provisioning awaits budget
approval**. The Fly app exists without Machines. No managed database, backup,
PITR recovery point or seven-day hosted evidence exists yet.

## Proposed configuration and budget decision

- Region: `iad`, colocated with `sui-sentinel-r1`, subject to MPG availability.
- Cluster name: `sui-sentinel-r1-db`; PostgreSQL 17 to match local tests.
- Initial candidate: Starter, 2 GiB/shared 2 CPUs, 30 GB database storage. This
  matches the local benchmark memory envelope but does not establish hosted capacity.
- As checked on 2026-11-02, Fly lists Starter at $72/month and storage at
  $0.28/GB-month: **$80.40/month for this database candidate alone**. App compute,
  capture volume/snapshots, transfer, provider account, monitoring and temporary
  restore clusters are additional. Recheck the quote before approval.
  [Official MPG pricing](https://fly.io/docs/mpg/)
- Include a separately billed temporary restore cluster in the approved drill
  budget and a cap/owner for teardown. No automatic scale-up or spend approval
  is implied. The CLI reported no payment method during app preparation.

The following is a **reviewable future command**, not an executed step:

```sh
fly mpg create --name sui-sentinel-r1-db --org personal --region iad \
  --pg-major-version 17 --plan Starter --volume-size 30
```

After budget approval, verify the quoted plan, available region, cluster version,
PITR support, actual retention window, backup encryption and maintenance policy.
Do not substitute a legacy/unmanaged database if MPG or PITR is unavailable.

## Connection and role qualification

1. Use a direct/session-preserving database connection for the writer and
   migrations. The ingestion coordinator owns a session advisory lock; transaction
   pooling must not silently break this fencing. Connection startup settings and
   read-only transactions must also behave as tested. Use the provider's documented
   direct endpoint, then verify TLS and the expected cluster/database identity.
   [PgBouncer feature compatibility](https://www.pgbouncer.org/features.html)
2. Inject five separate role passwords into the migration environment and run
   `deploy/ops/init-roles.sql`. Run migrations through 8 with `sentinel_migrate`,
   apply `grants.sql`, then run `check-query-grants.sql` on an isolated acceptance
   database. Reader writes must fail; monitor/operator changes must atomically
   advance query generations. Restore the grants after each additive migration.
3. Stage role-specific DSNs in Fly secrets: `DIAGNOSIS_DATABASE_URL`,
   `SENTINEL_DELIVERY_DATABASE_URL`; use `SENTINEL_QUERY_DATABASE_URL` only in a
   separately scoped MCP process. Stage `SENTINEL_READ_TOKEN` for monitor reads.
   Migration and operator credentials must not be in the steady-state app.
4. Confirm least-privilege network access and database connection limits. The
   monitor/delivery pools and four MCP connections count separately; account for
   migrations, recovery and probes before accepting a cluster connection budget.
5. Measure disk growth, query/commit latency, DB CPU/memory and backup lag under
   the actual workload. A 3.56 GB restored projection is a baseline dataset, not
   a seven-day storage forecast. Retention of frames/incidents/evidence needs its
   own capacity limit and archival policy before unattended operation.

## PITR acceptance drill

Fly's restore command creates a **new, separately billed cluster** and leaves the
source unchanged. PITR needs a v2 cluster and a timestamp within its actual
recovery window. This is provider capability, not proof of our RPO/RTO.
[Official PITR command and constraints](https://fly.io/changelog/managed-postgres-point-in-time-restore/)

1. On an isolated hosted acceptance database, commit an incident, its outbox row,
   an operator annotation and a checkpoint marker; record server timestamps,
   source checkpoint/digest, row IDs and payload hashes. Observe backup/WAL progress.
2. Record a recovery target after those commits. Commit a second marker after the
   target so the drill can prove it restored the requested boundary, not merely
   any working database. Keep the monitor and delivery workers stopped on the
   restored copy to prevent notifications or duplicate writers.
3. After confirming the target lies within the real PITR window, execute only
   under the approved drill budget:

   ```sh
   fly mpg restore CLUSTER_ID --pitr-time RFC3339_TARGET \
     --name sui-sentinel-r1-pitr-drill
   ```

4. Time from the simulated loss/restore decision until an authenticated application
   can read validated state. Verify the first marker exists, the second is absent,
   and incident/outbox/audit relationships and projection hashes agree. Reapply
   role grants and run read-only MCP and monitoring checks against the restored copy.
5. Measure effective lost committed data, including non-chain annotations. Require
   RPO <=300 seconds and RTO <=1800 seconds on the declared dataset; record actual
   values, denominator, failures, logs and hashes. Fast local pg_dump restoration
   does not establish hosted PITR or network transfer time.
6. Rehearse reconnect/replay and independent census reconciliation before any
   cutover. Preserve delivered outbox state; never replay external notifications
   merely because a drill restored them. Fence the old writer before activation.
7. Retain evidence, then remove the temporary cluster only under the approved
   teardown instruction. A rollback plan must include both schema and binary.

## Capture backup and seven-day observation

Database PITR does not back up the Fly capture volume. Select an encrypted,
off-host object-storage destination, key custody, retention and spend limit. Back
up complete immutable generations plus manifests/checksums, publication pointer
and run journals; verify restored artifact hashes before publishing. Coordinate
capture retention with incident evidence references. Fly volume snapshots alone
are not independent off-host recovery evidence.

After provisioning, capture qualification, recovery and activation approval,
run the [independent seven-day sampler](operations-release.md#recovery-and-release-evidence)
from a different host. Retain
all missing/late samples as failures. The clock starts after declared bootstrap,
not when code was written. Keep hosted financial and live-throughput claims
separate from the cached R1 availability result.
