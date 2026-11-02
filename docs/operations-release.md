# Operating the R1 cached-monitoring candidate

R1 observes capture freshness, completeness, publication, provider availability,
local storage and notification delivery. It does not provide fresh collateral
valuation or liquidation decisions. See the [release evidence](reviews/2026-11-02-r1-release.md)
and [remaining gates](readiness.md) before activating production.

## Selected deployment

- Fly.io app `sui-sentinel-r1` exists in the authenticated personal organization;
  `fly launch --no-deploy` and CLI validation passed on 2026-11-02. The hostname
  `sui-sentinel-r1.fly.dev` is reserved, but no Machines or service are deployed.
- One continuously running Machine, initially 2 shared CPUs and 2 GiB RAM in `iad`.
  Keep exactly one monitor for this scope. File locks do not coordinate different
  hosts/volumes. Do not horizontally scale until distributed fencing exists.
- PostgreSQL stores checkpoints, incidents, outbox and audit records. Configure a
  Fly Managed Postgres service with private networking and verified TLS. Its
  [provisioning and PITR plan](fly-postgres.md) awaits budget approval.
  There is no SQLite database. The 30 GB Fly volume stores raw captures, reports,
  progress journals and publication pointers; it is not the database backup.
- QuickNode Sui Mainnet GraphQL for R1 snapshots. One complete 150,166-position
  main-market census passed in 1,341.65 seconds with 5,928 requests and no retries
  or throttles. Repeat account-capacity and history-retention checks remain gates. Use the endpoint hostname with `/graphql`, with the
  credential in `SUI_API_KEY` and `SUI_API_KEY_HEADER=x-token`.
- BlockVision gRPC remains the selected future live source:
  `https://sui-mainnet-grpc.blockvision.org:443`. The R1 monitor does not consume it.
- Discord incoming webhook targets the user-selected `#sui-risk-alerts` channel.
  The webhook URL contains a credential: store the entire URL as a secret.
  A single real TEST ONLY notification passed through the durable sender and was
  accepted by Discord. QuickNode and Discord credentials are staged in Fly secrets;
  database-role credentials and the monitoring read token remain unconfigured.

The [Fly configuration](../deploy/fly.toml) has HTTPS enforcement, no autostop,
rolling replacement and `/livez` checks. Liveness permits historical diagnostics
while stale readiness remains visible. External monitoring must separately check
`/readyz` and `/status`; a healthy Fly routing check is not freshness evidence.

## Preparation and activation gates

Complete these steps in order. App creation and staging the supplied secrets were
explicitly authorized and completed. Paid database provisioning awaits budget
approval; production deployment still requires completed prerequisites and approval.

1. Run `python3 deploy/ops/check_fly.py`, formatting, Clippy, PostgreSQL-backed
   tests and the security workflow. The local checker validates TOML/invariants;
   it does not replace `fly config validate --config deploy/fly.toml` with the
   installed Fly CLI. CLI validation has passed for the prepared app.
2. Confirm the Fly app, organization, region, PostgreSQL service, costs and
   independent uptime/alert receiver. Keep Grafana/Prometheus private or use a
   separately authenticated managed service. This Fly app serves diagnostics,
   not Grafana itself. The secured Compose stack is an alternative self-hosted
   reference, not a description of the Fly deployment.
3. Create DB roles using `deploy/ops/init-roles.sql` under the database owner.
   Supply its five password environment variables through the secret manager;
   never commit a populated file. Use `sentinel_migrate` to run `sui-sentinel
   migrate`, then apply `deploy/ops/grants.sql`. Reapply grants after migrations.
   Migrations are a separate maintenance step, not an app-start privilege.
4. Configure secrets: `DIAGNOSIS_DATABASE_URL` (monitor role),
   `SENTINEL_DELIVERY_DATABASE_URL` (delivery role), `SUI_API_KEY`,
   `SENTINEL_WEBHOOK_URL`, and a random `SENTINEL_READ_TOKEN` of at least 32 ASCII
   characters. Configure `SUI_GRAPHQL_URL` independently. Use Fly secret import
   from a protected source, not literal secrets in shell history. The optional
   future `SUI_GRPC_API_KEY` and header name belong to the gRPC client only.
5. Initialize the capture volume for UID/GID 65532 and verify writable ownership
   before starting the non-root app. Retain at least 5 GiB free. Never solve
   permission failures by running the steady-state application as root.
6. Build from a clean, committed revision, passing its full 40-character hash as
   `SOURCE_REVISION`. Retain the image digest, executable hash, SBOM and scan
   results. Production startup rejects unrecorded/candidate revision strings.
   Pin the approved image digest when releasing; the local candidate is not a
   source-pinned production artifact.
7. Validate one full QuickNode main-market capture, provider quota and deadline
   margin; send a test notification only to the configured test/approved channel.
   Verify missing, stale, failed and recovered states. Reconfirm activation
   authorization, then provision/deploy with **one Machine**, without HA cloning.
8. Start the seven-day R1 sampler after initial bootstrap. Record excluded
   bootstrap separately; do not trim later bad slots. Fulfil the remaining gates
   in [readiness and roadmap](readiness.md) before declaring the operated service production ready.

Official references: [Fly configuration](https://docs.fly.io/reference/configuration),
[QuickNode GraphQL](https://www.quicknode.com/docs/sui/sui-graphql),
[QuickNode authentication](https://www.quicknode.com/docs/sui/endpoint-security),
[Discord webhook confirmation](https://docs.discord.com/developers/resources/webhook#execute-webhook).

## Policy, telemetry and backpressure

`deploy/config/monitoring.mainnet.toml` pins the main market, a 3,600-second
interval and 2,400-second end-to-end capture deadline. The selected QuickNode census
measured 1,341.65 seconds; a previous 900-second attempt timed out safely. These
margins remain provisional until repeated account-capacity and soak evidence.
The policy warns at 6,000 seconds of observation age and becomes critical at
9,600 seconds. This is hourly cached monitoring, not near-real-time valuation.
`SUI_SNAPSHOT_CHECKPOINT_LAG=200` pins a provider checkpoint behind head once;
that offset adds observation age and never permits changing checkpoint mid-capture.
The older `monitoring.toml` is a short-interval fixture/development policy.

The supervisor runs collection, observation and notification delivery independently.
A capture lock prevents overlapping publication. The GraphQL semaphore bounds
in-flight requests. Checkpoint channels remain bounded and sequential. Slow
consumers make producers wait instead of allocating an unlimited queue: that is
backpressure. The cached adapter now avoids unused clock and unchanged-reserve
recalculation; database transactions still preserve atomic progress. Keep short in-memory locks;
shard only independent markets after measuring contention.

`operations.json` is atomic local telemetry; child progress records distinguish
collecting, captured, analyzing and validating. A dead scheduler, observer or
notification worker expires readiness. Database availability and successful
observation are distinct fields. The final progress counters are actual request
attempts, retries, throttles, failed attempts and bytes consumed from successful
response bodies. They are not billing counters or total network wire bytes.
Terminated jobs may retain only the last periodic progress sample.

Retention preserves current, last-good, unreplayed/incomplete runs and evidence
referenced by incidents, historical notifications or operator audits. Cleanup holds
the capture lock and refuses unsafe generation symlinks. A bounded journal budget
fails visibly when exceeded. Indefinitely pinned evidence and database signal
history still require an archival/storage budget; do not delete them blindly.

## Operator workflows

1. **Is it usable?** Read readiness, policy freshness, coverage and monitor health
   in the top Grafana panels. Then inspect `/status` for the reason, checkpoint,
   exact market, build and policy identity. Historical report access is not proof
   of current data. No price-age or fresh-valuation metric is fabricated.
2. **Capture failed/stalled:** inspect `runs/*.json` and matching `.progress`,
   request/throttle counts, disk space and phase duration. A failed generation must
   not replace `current`; fix the dependency and allow the next scheduled capture.
   Never manually label a partial generation successful.
3. **Notifications delayed:** inspect oldest queue age, attempts and dead letters;
   use `sui-sentinel admin --scope suilend-main-r1 inspect` with a reader DSN.
   Inspect output before retrying. Use an operator DSN for `acknowledge ID --reason
   '...'` or `retry ID --reason '...'`; both are scoped and audited. Retry rejects
   leased, delivered and obsolete messages. Acknowledgment records ownership;
   it does not silence reminder notifications or resolve the underlying condition.
4. **Provider/DB outage:** check source quota and private DB reachability. Never
   switch providers without verifying mainnet/checkpoint identity and history.
   DB calls have statement/lock/acquisition timeouts; durable outcomes replay
   after recovery. The external watchdog must catch complete monitor/DB/host loss.
5. **Storage pressure:** pause expensive work, preserve last-good and incident
   pins, archive verified generations off-host and expand storage deliberately.
   Do not remove a pinned generation to make a dashboard green.

Monitor HTTP protection accepts `Authorization: Bearer <SENTINEL_READ_TOKEN>` on
all routes except `/livez`. The legacy standalone `serve`/`serve-dir` interfaces
remain private-network tools; use an authenticated gateway if exposing them.
Prometheus must supply the token when scraping the combined Fly monitor. Never
put tokens in Grafana links. Import the generated dashboard and provision its
`sentinel-prometheus` datasource in the chosen authenticated Grafana service.

Alerts use 15-second opening and 30-second recovery debounce plus hourly reminders.
Discord messages disable mentions, include checkpoint/rule/scope and a stable
delivery ID, and request `wait=true` for acceptance confirmation. Delivery is
**at least once**. Discord does not guarantee deduplication of our header; lost
acknowledgments can duplicate a message. Use its displayed delivery ID when
investigating, or a durable deduplicating relay before any automated downstream
side effect. Generic receivers must deduplicate `Idempotency-Key`. HTTP 429
`Retry-After` seconds are respected up to the declared one-day retry cap.

## Recovery and release evidence

Desired RTO: restore the declared dataset within 30 minutes. Desired RPO: at most
five minutes for incidents/audit/outbox, plus checkpoint replay bounded by provider
history. These are production objectives, not provisioned guarantees. Configure
managed PostgreSQL PITR and encrypted off-host capture backups before activation.
Fly volume snapshots alone do not meet the database RPO.

For local Docker drills, `deploy/ops/backup.sh CONTAINER NEW_DIRECTORY DATABASE
USER` creates a custom-format dump with checksum. `restore-drill.sh CONTAINER
BACKUP NEW_DATABASE_restore USER` refuses a non-drill/existing database. Compare
checkpoint/digest, object and evaluation hashes, incidents, delivery IDs and audit
rows; measure elapsed restore time. Managed production PostgreSQL requires its
provider-native backup/PITR procedure. Reapply roles/grants to a restored database.

Before upgrade, retain a verified backup and old image digest. Rehearse additive
migrations and both versions against a copy. Roll back only to a schema-compatible
binary; restore incompatible data into a separate database under maintenance.
Stop the old writer before starting a replacement with the same scope/volume.
Do not replay already-delivered notifications after restoring an older outbox
without reviewing receiver deduplication and the recovery point.

Run the sampler from an independent host with the read token held only in
`SENTINEL_STATUS_AUTHORIZATION` (`Bearer ...`):

```sh
python3 deploy/ops/soak.py collect --url https://sui-sentinel-r1.fly.dev/status \
  --out /protected/evidence/r1-soak --seconds 604800 --interval 15
python3 deploy/ops/soak.py assess /protected/evidence/r1-soak
```

Missing/late samples count as failures. Source revisions must be pinned. A short
smoke run is always `INCOMPLETE`; `PASS_SOAK_ONLY` does not waive recovery,
security, provider, operator UX or financial-correctness gates.
