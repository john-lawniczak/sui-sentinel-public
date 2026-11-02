# Task F02 — report provenance and artifact checksums

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: F02 (temp.md section 11, depends on F01)
- **Status**: DONE (scope below; see Limitations for what is explicitly deferred)
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `2088c9a` (F01, branch `production-readiness-plan`)
- **Dependencies**: F01 (DONE)
- **Files**: `src/storage.rs`, `src/model.rs` (unchanged, read-only reference),
  `src/analysis/report/{mod,types}.rs`, `src/export/mod.rs`, `Cargo.toml`,
  `tests/snapshot_report.rs`, `README.md`

## Scope taken from temp.md section 3 item 2 and item 3, and section 11 row F02

> Expand report provenance with run ID, chain, protocol, deployment/package
> version, configuration and policy hashes, adapter build, observation
> checkpoint, valuation time, and coverage status. Define a schema migration
> and backwards-compatible reading strategy; do not relabel old cached
> reports as verified.
>
> Add artifact checksums and a manifest that binds the complete artifact
> set. Verify checksums on replay and publication.

## Change

### Provenance (`src/analysis/report/types.rs`, `mod.rs`)

Added `Report.provenance: Option<Provenance>` with:

- `adapter_version` — `CARGO_PKG_VERSION` (adapter build identity).
- `chain_identifier`, `original_package_id`, `upgrade_cap_object_version` —
  copied from the manifest's already-validated deployment identity fields
  (no new trust assumptions; `generate_with_policy` already `ensure!`s the
  manifest is mainnet and the package ID is well-formed before this point).
- `stress_policy_sha256` — sha256 of the canonical JSON encoding of the
  `stress::Policy` applied to this report, so two reports can be compared
  for identical policy without diffing the full embedded policy document.
- `generated_at` — RFC3339 wall-clock time the report was generated. This is
  **valuation time**, explicitly distinct from `Report.checkpoint.timestamp`
  (chain observation time); the field's doc comment states this to prevent
  the two from being conflated downstream.

`Provenance` has no `schema_version` of its own; it is additive to the
existing `Report.schema_version = 1`. Backward-compat reading: the field is
`#[serde(default)]`, so a pre-F02 `report.json` (no `provenance` key)
deserializes with `provenance: None` rather than failing, and its
`calculation_status` is untouched — an old report is not "relabeled" as
having provenance it never recorded. `run_id` from the temp.md wish list was
**not** added (see Limitations) since it requires a scheduler/run concept
that does not exist until O02.

### Artifact checksums (`src/storage.rs`)

Added `storage::Checksums { algorithm, files: BTreeMap<String,String> }` and:

- `write_checksums(directory, &[names])` — sha256 of each named sibling
  file, written via the existing `storage::json` atomic-publish helper
  (temp partial file, fsync, hard-link, matching the pattern already used
  for `manifest.json`/`report.json`).
- `verify_checksums(directory, &[expected_names])` — reads
  `checksums.json`, asserts its bound-file set is *exactly* the expected
  set (catches a manifest that silently stopped covering a file, not just
  per-file corruption), then recomputes and compares each digest.

`report::generate_with_policy` calls `write_checksums(out, &["positions.ndjson",
"report.json"])` after both files are flushed and `sync_all`'d, binding the
complete output artifact set for that generation.

`export::serve` calls `verify_checksums` on the report directory before
loading `report.json` for serving:

- Manifest present and mismatched → **hard failure** (corruption; fail
  closed, per temp.md section 2's "treat ... freshness ... as separate
  dimensions" and general fail-closed posture).
- Manifest absent → **warning to stderr, serve anyway**. This is the
  explicit "do not relabel old cached reports" requirement: a report
  generated before this change has no checksums to violate, so absence is
  not treated as corruption.

### Dependency

Added `sha2 = { version = "0.10", default-features = false, features =
["std"] }` to `Cargo.toml` (pulls in `digest`, `block-buffer`,
`crypto-common`, `generic-array`, `typenum`, `cpufeatures`,
`version_check` as transitive deps — all widely-used, no-`unsafe` crates
compatible with the crate's `unsafe_code = "forbid"` lint). Added `time`'s
`formatting` feature (previously only `parsing`) to render `generated_at`.

## Acceptance checks

1. **Determinism preserved.** `complete_mainnet_capture_reports_ownership_gaps_and_replays_deterministically`
   (`tests/snapshot_report.rs`) was updated: `positions.ndjson` is still
   compared byte-for-byte (it carries no generation-time metadata), while
   `report.json` is compared as parsed JSON with `provenance.generated_at`
   explicitly asserted to be a string and then nulled out before the
   equality check — the one field expected to differ between two
   generations of the same input is the one the test now names and skips,
   rather than silently loosening full-file equality. The test also asserts
   `storage::verify_checksums` succeeds against the output of both runs.
2. **Corruption detection.** New unit tests in `src/storage.rs`
   (`verifies_bound_files_and_detects_corruption_and_scope_drift`,
   `missing_checksum_manifest_fails_rather_than_treating_absence_as_valid`)
   cover: matching manifest passes; a tampered bound file is caught;
   asking for a different expected-file set than the manifest covers is
   caught (both a subset and a superset); and a wholly missing manifest
   fails `verify_checksums` directly (the *caller*, `export::serve`, is
   what downgrades "missing" to a warning — the storage-layer function
   itself is strict, which is the correct place to keep the strict
   invariant).
3. All existing regression tests pass unchanged in behavior (only the one
   determinism test's comparison mechanics changed, per #1).

## Commands run

```
cargo test --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
```

All passed.

## Limitations / explicitly deferred

- **No `run_id`.** temp.md asks for a run identifier in provenance, but a
  "run" is a scheduler/orchestration concept that does not exist in this
  repository yet (that's task O02, "Add run lifecycle"). Adding a
  freestanding UUID with no corresponding run record would be provenance
  theater — a field nothing else can join against. Tracked here so O02's
  implementer wires `Provenance.run_id` to the real lifecycle record instead
  of reinventing it.
- **No cryptographic correspondence to provider JSON.** As temp.md states
  explicitly, local sha256 checksums detect corruption of the artifacts
  *this repository wrote*; they say nothing about whether the raw capture
  matches an on-chain object's actual digest. That is unrelated,
  already-tracked work (collection-time validation in
  `src/ingest/snapshot.rs`, untouched by this task).
- **No schema version bump.** The change is additive and backward-compatible
  (`#[serde(default)]`), so `Report.schema_version` stays `1`. A genuine
  schema migration (versioned reader strategy, `schema_version` bump, and
  explicit handling of *removed or renamed* fields) is unneeded for an
  additive change and is deferred until a breaking change is actually
  proposed.
- **`export::serve`'s warning branch is not covered by an automated test.**
  It is a thin wrapper around `storage::verify_checksums`, which is fully
  covered; adding an HTTP-level test would require spawning and tearing
  down a real `axum::serve` listener in the test, which the existing test
  suite avoids (`http_serves_immutable_artifacts_and_health` exercises
  `export::router` directly, not `export::serve`). Noted as a gap rather
  than silently skipped.
