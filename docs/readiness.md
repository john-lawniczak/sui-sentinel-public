# Production readiness and R1 release gates

Assessment as of 2026-11-02, based on release `31c10d2` and its
[recorded evidence](reviews/2026-11-02-r1-release.md): **7.00/10.00** for the
current cached Suilend diagnostics application. This is an engineering judgment,
not a reliability measurement or production certification. The intended real-time,
multi-protocol/MCP platform is approximately **4.00/10.00** implemented/validated.

The strongest areas are reproducible captures, explicit coverage and freshness,
atomic incident/outbox writes, last-good retention, bounded concurrency, and
failure-path testing. The recorded release passed 123 Rust tests with PostgreSQL,
41 Grafana queries, local full-market captures and a full-size restore drill.
Production operation, sustained live throughput, verified fresh valuation, and
multiple adapters remain unqualified or unfinished. The newer [MCP implementation](mcp.md)
and [release follow-up](reviews/2026-11-02-mcp-release.md) extend this dated baseline;
the baseline rating is not a claim of current production certification.

This file contains current R1 blockers. Future R2–R4 capabilities and their
acceptance criteria are in [the roadmap](roadmap.md); they are not all required
to ship a narrowly scoped cached-monitoring release. Follow the
[operating runbook](operations-release.md). No production deployment has occurred.

## Execution rules

1. Choose the first task with satisfied dependencies. Split it into reviewable
   subtasks; record owner, input commit, acceptance checks and evidence location.
2. Inspect the implementation before editing. Preserve existing behavior tests
   and unrelated work. Keep maintained files at or below 400 lines.
3. Test failure paths, not only successful execution. Run formatting, Clippy,
   all targets with real PostgreSQL, the file-size check and dashboard generator.
4. For UI work, test the running stack and rendered failure states. For provider
   work, retain dated endpoint/version/data evidence with credentials removed.
5. Remove completed requirements from this file; retain their evidence under
   `docs/reviews/`. Do not remove an external-validation gate after passing unit tests.
6. If external evidence blocks one capability, keep that capability unavailable
   and proceed with independent work. Never substitute cached math for fresh risk.

## R1 activation and remaining operational evidence

Owner for each item must be assigned before activation. Implementation and local
measurements are recorded in [the R1 review](reviews/2026-11-02-r1-release.md);
follow [the release runbook](operations-release.md). Do not redo completed
telemetry, heartbeat, debounce, retention or administration implementations.

### O01 — Validate the selected production providers

Inputs: QuickNode Sui Mainnet account, BlockVision gRPC, exact main-market policy.

- [ ] Repeat the successful selected-account QuickNode full census under the
  declared quota and hosted network. The first passed with 150,166 positions in
  1,341.65 seconds, 5,928 requests and no retries/throttles. Qualify the revised
  provisional 3,600 s interval/2,400 s deadline and record provider limits/costs.
- [ ] Confirm checkpoint/object history retention, archival/fallback behavior,
  costs and support expectations. Verify provider identity before failover.
- [ ] For R2, observe real mutations, capability transfers and SUI/native-USDC
  positions; compare exact versions with an independent GraphQL census. Clock-only
  checkpoints and three development censuses do not complete this gate.

Acceptance: dated provider/account-capacity evidence with credentials redacted;
no public-provider production fallback or silent partial coverage.

### O02–O04 — Finish operational failure and scale evidence

Inputs: source-pinned candidate image and isolated fault-injection environment.

- [ ] Exercise supervisor/host death in collecting, analyzing, validating and
  publication, plus stalled filesystem and DB failover under load. Existing
  deadline/cancellation, failed-capture replay, full-disk and lost-ack tests are
  baselines; retain evidence that a replaced writer cannot publish late.
- [ ] Establish database signal/outbox/audit archival policy and capacity budget.
  Retained incident evidence can pin generations indefinitely. Measure directory
  scan and DB queue-query costs at the declared seven-/30-day retention scale.
- [ ] Obtain human channel acknowledgement and test hosted Discord recovery.
  One real TEST ONLY outbox notification was HTTP-accepted on 2026-11-02; this
  proves sender acceptance, not human delivery or hosted recovery. Direct Discord
  is at least once; review the duplicate-message acceptance policy or deploy a
  durable deduplicating relay before downstream automated side effects.
- [ ] For R2, expose ingestion/evaluation lag, reconciliation age and supported
  scope independently of snapshot readiness. Prove stale live state fails closed.

Acceptance: failure matrix with observed outcomes and hashes, queue/disk budgets,
no lost non-chain annotations, and documented receiver semantics.

### O05/F02 — Provision and qualify the Fly deployment

Inputs: user approval after Fly app, provider credentials and Discord configured.

- [ ] Approve the [Fly Managed Postgres budget and prepared recovery plan](fly-postgres.md),
  then provision the database, network/TLS and external watchdog. The Fly app now
  exists in `iad`; `fly launch --no-deploy` and CLI config validation passed.
  No Machines, database or capture volume were provisioned. Initialize the future
  capture volume for UID/GID 65532 and keep one writer.
- [ ] Configure database-role credentials and the monitoring read token. QuickNode
  GraphQL/API and Discord webhook secrets are already staged in Fly; never paste
  values here. Verify all role separation after provisioning.
- [ ] Build the committed 40-character revision, retain immutable image and binary
  digests, SBOM and scan reports. CI builds source-pinned images; the measured
  local candidate deliberately remains marked as a candidate.
- [ ] Configure managed PostgreSQL PITR and encrypted off-host capture backups;
  prove production RPO <=5 min for incidents/audit/outbox and RTO <=30 min for
  the declared dataset. Local full-projection and notification restores passed,
  but are not evidence of hosted PITR, cross-host recovery or network throughput.
- [ ] Rehearse upgrade/rollback of schema and application against a restored copy;
  restore legacy capture artifacts and retain their unbound provenance labels.
- [ ] Provision private/authenticated Grafana and Prometheus or a managed service;
  configure the read token for scraping and an independent host/DB failure alert.
- [ ] Obtain production activation authorization only after prerequisites are
  configured. No production deployment has been performed in this work.

Acceptance: tested host-specific runbook, pinned release evidence, externally
verified authentication and recovery. One Fly Machine is not a high-availability
architecture; do not claim host failover until single-writer fencing is designed.

### R1 — Seven-day qualification

- [ ] Start `deploy/ops/soak.py` from an independent host after bootstrap; sample
  `/status` every 15 seconds for at least 604,800 seconds. Use a protected evidence
  directory and source-pinned build; missing/late slots count as failures.
- [ ] Meet >=99.9% query availability and >=99.9% fresh/complete availability,
  with denominators, bootstrap exclusions, fault/recovery results and costs.
- [ ] Complete U02 rendered/operator evidence in [the roadmap](roadmap.md#grafana-operator-experience). A short smoke test or
  `PASS_SOAK_ONLY` does not close other release gates.

Acceptance: reviewed seven-day evidence bundle and all declared R1 capability
criteria passed. Do not convert elapsed engineering work into a readiness score.

