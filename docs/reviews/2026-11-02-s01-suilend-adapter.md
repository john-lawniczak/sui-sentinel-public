# Task S01 — extract a Suilend protocol adapter

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: S01 (temp.md section 11, depends on F02)
- **Status**: DONE (adapter extraction scope; see Limitations for what
  remains intentionally protocol-coupled)
- **Owner**: John Lawniczak (lawniczak.john@gmail.com), Claude Code (agent)
- **Input commit**: `2d524b2` (O04, branch `production-readiness-plan`)
- **Dependencies**: F02 (DONE)
- **Files**: `src/model.rs`, `src/ingest/snapshot.rs`, `src/protocols/{mod,suilend}.rs`
  (new), `src/lib.rs`

## Scope taken from temp.md section 11 row S01 and section 9's adapter contract

> 1. Define capability/deployment contract. 2. Extract Suilend parsing
> into an adapter. 3. Preserve old cached replay behavior and test
> unsupported versions; keep infrastructure protocol-neutral.

Section 9 adds: "Give each adapter a manifest: protocol ID, supported
package/type origins and versions, discovery strategy, required
objects/feeds, capabilities, confidence requirements, and adapter
build/version... Start with statically registered Rust adapters... do not
begin with arbitrary dynamic library loading." Its rollout sequence item 1
says exactly: "Extract a Suilend adapter without changing cached replay
outputs" - this task does only that; item 2 ("select NAVI or Scallop...
implement the chosen adapter end-to-end before declaring the abstraction
reusable") is explicitly a *later*, separate task (P01) that this one does
not anticipate.

## What was implemented

### Why no adapter trait yet

With exactly one adapter, a trait abstracting "an adapter" would encode
Suilend's accidental shape as if it were a general contract - guessing at
what a second, structurally different lending protocol (NAVI/Scallop)
actually needs before that adapter exists to tell us. Section 9's own
rollout sequence agrees: generalize only once a second adapter is
implemented end-to-end (P01). So S01 defines the "capability/deployment
contract" item 1 asks for as a plain struct, not a trait:

```rust
// src/protocols/mod.rs
pub struct AdapterManifest {
    pub protocol_id: &'static str,
    pub adapter_version: &'static str,
    pub capabilities: &'static [&'static str],
}
```

Populated with only fields this repository has a real, checkable answer
for today. Other fields section 9 lists (discovery strategy, required
objects/feeds, confidence requirements) are not stubbed in with
placeholder values - adding them with nothing behind them would be
exactly the kind of invented-field "provenance theater" this project's own
prior task records (e.g. F02's) have flagged and avoided.

### What moved (`src/protocols/suilend.rs`, new)

Every Suilend-specific Move type/module-path string construction,
relocated verbatim (not rewritten) from `model.rs` and
`ingest/snapshot.rs`:

- `market_type_filter(package)` — the GraphQL type filter
  `ingest::snapshot::discover` pages over (was an inline `format!` in that
  function).
- `market_type_argument(move_type, package)` — extracts a market object's
  type argument from its fully-instantiated Move type, replacing the
  inline `strip_prefix`/`strip_suffix` pair that was in `Market::new`.
  Returns `None` for a type that doesn't match this adapter's expected
  shape at all (different package, unrelated type) - the caller treats
  that as unsupported, not a retry.
- `obligation_type(package, type_argument)` / `cap_type(package,
  type_argument)` — were `Market::obligation_type`/`Market::cap_type`'s
  method bodies; `Market` now calls into the adapter instead of
  constructing the strings itself.

`model::Market`'s own shape (object + table_id + type_argument +
expected_obligations) is unchanged - it was already fairly
protocol-neutral; only the Move-type-name *construction* moved out.
`ingest::snapshot::discover` now computes `market_type_filter` once via
the adapter instead of inlining a `format!` at its call site.

## Acceptance checks

1. **Preserve old cached replay behavior** — `tests/snapshot_report.rs::complete_mainnet_capture_reports_ownership_gaps_and_replays_deterministically`
   (and every other `snapshot_report.rs`/`export.rs`/`stress.rs` test) pass
   unchanged, with no modification to any test file. Since every moved
   function is a verbatim relocation (same format strings, same
   strip_prefix/strip_suffix logic), this is the expected outcome, not a
   coincidence - confirmed by running the full suite before writing any
   new test.
2. **Test unsupported versions** — three new unit tests:
   - `protocols::suilend::tests::market_type_argument_rejects_an_unsupported_or_unrelated_package`
     — a type string from a *different* package than the one passed in
     returns `None` from the adapter's own pure function.
   - `model::tests::market_new_rejects_an_object_from_an_unsupported_or_different_package`
     — the same scenario through the real `Market::new` integration path
     (constructs a full `Object`, calls `Market::new` with a mismatched
     configured package, asserts the "unexpected market type" error) -
     checking the model/adapter wiring itself, not just the adapter's
     internal logic in isolation.
   - `model::tests::market_new_accepts_a_matching_package_and_extracts_its_type_argument`
     — the positive case, so the rejection test can't pass by accident
     (e.g. if `Market::new` rejected *everything*).
   - Plus `protocols::suilend::tests::market_type_argument_round_trips_through_the_filter_and_prefix`
     and `..._obligation_and_cap_type_embed_the_market_type_argument`
     covering the adapter's functions directly.
3. **Keep infrastructure protocol-neutral** — `model.rs`'s struct
   definitions, `ingest/snapshot.rs`'s GraphQL pagination/collection logic,
   `storage.rs`, and all of `analysis`/`export` are untouched by this
   task; the only Suilend-specific string construction left anywhere in
   those modules was the two sites just extracted, confirmed by grepping
   for `lending_market::`/`obligation::Obligation`/`ObligationOwnerCap`
   across `src/` after the change - every remaining hit is inside
   `src/protocols/suilend.rs`.

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

- **`config::SuilendConfig` and the CLI's capture/report option shapes are
  still Suilend-specific**, referenced directly from `ingest::snapshot`
  (`config.suilend.original_package_id`, `.upgrade_cap_id`, etc.) and
  `src/main.rs`. Generalizing the *deployment configuration* shape (not
  just Move-type parsing) into a protocol-neutral contract is bigger scope
  than S01's own bullet list asks for, and - per section 9's rollout
  sequence - premature before a second adapter's actual configuration
  needs are known. This task extracted exactly what its own text named
  ("extract Suilend parsing into an adapter"); reshaping configuration is
  naturally P01's job once NAVI/Scallop's real requirements are in hand.
- **No adapter registry or trait.** As explained above, this is a
  deliberate rollout-sequence decision (section 9), not an oversight -
  revisit once P01 implements a second adapter end-to-end.
- **`capabilities: &["cached_lending_health"]` is not read by anything
  yet.** No code currently queries `AdapterManifest` to gate behavior; it
  exists as the "capability contract" item 1 asks for, ready for a future
  caller (e.g. a query service deciding what to expose per protocol) to
  consult, rather than as dead weight - but it is, today, unread. Noted
  rather than hidden.
