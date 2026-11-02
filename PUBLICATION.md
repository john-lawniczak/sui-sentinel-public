# Public source preview

This Apache-2.0 repository starts with a clean source snapshot of the audited
November 2, 2026 implementation. Earlier private Git history, credentials,
ignored snapshots, database backups, build outputs and local configuration are
not included. The original repository retains its existing visibility.

The source matches the audited implementation except for public repository
links, the example Fly app name/check, and publication/security documentation.
The upstream source revision is `022d9d7`; historical commit IDs in review and
evidence documents refer to that earlier development history and may not be
resolvable in this repository. Those documents describe earlier experiments,
not a hosted service supplied with this repository. App/database names and
credential-provisioning statements in historical runbooks refer to the original
operator's environment. Provision your own resources; no accounts or secrets
are supplied. The Fly configuration intentionally requires an app-name change.

The bundled fixture contains public on-chain observations; its provenance is in
`tests/data/elixir/README.txt`. Captured addresses identify on-chain objects and
immediate owners; they do not assert real-world identities.

The audited code passed 139 Rust tests against PostgreSQL 17 plus the checks in
`docs/reviews/2026-11-02-main-post-merge-audit.md`. The exported source tree was
scanned locally with Trivy for secrets. This is an early source preview, not
qualification of fresh financial valuation or production operations. See the
README, contributor guide, security policy and roadmap before use.
