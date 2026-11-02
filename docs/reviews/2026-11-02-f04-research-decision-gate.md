# Task F04 — deployed-source/oracle verification research decision gate

> Historical review of the implementation at that stage. Its open-task and
> default-setting statements may be superseded; use [current readiness](../readiness.md)
> and the [latest release evidence](2026-11-02-r1-release.md) for current status.

- **Task ID**: F04 (temp.md section 11, no dependencies)
- **Status**: DONE (per temp.md section 11: "The F04 research spike can be
  DONE with a documented blocked outcome, while the affected financial
  capability remains BLOCKED"). The underlying verification work is
  **BLOCKED**, unchanged from the prior review.
- **Owner**: unassigned (see Missing inputs below)
- **Independent reviewer**: unassigned (see Missing inputs below)
- **Input**: `docs/suilend-notes.md` (dated 2026-10-19), this session's branch
  `production-readiness-plan`
- **Dependencies**: none

## What this task record is, and is not

temp.md section 3's "research decision gate" calls for assigning an owner
and an independent reviewer, timeboxing a five-working-day evidence spike,
and recording a dated `verified` / `partially_verified` / `blocked`
decision per deployment/reserve path. This agent session has no mainnet
RPC/GraphQL access, no ability to independently fetch and diff a candidate
public-source revision against the live package, and no second human
reviewer available to it - so it cannot itself run that spike. Fabricating
an owner, a reviewer, or new verification evidence to make this task
appear more complete than it is would violate temp.md section 0 item 7
("Do not invent ... benchmark results, or approval") and section 7's
explicit prohibition on inventing package correspondence.

What this record does instead, consistent with "the F04 research spike can
be DONE with a documented blocked outcome": it (1) confirms the existing
evidence in `docs/suilend-notes.md` is the current, still-accurate decision
for every reserve/deployment path in scope, (2) records the exact missing
inputs blocking a real spike, and (3) keeps every financial capability that
depends on verified valuation explicitly ineligible (unchanged - no code in
this branch introduces or claims verified valuation).

## Decision per deployment/reserve path

| Path | Decision | Evidence |
| --- | --- | --- |
| Main Suilend market, all 45 reserves, package version 25 at `0x59672975e15c58ffc9450f5236b6aa9fa2407d94b8081e634778aa01957d8c6f` | **blocked** | `docs/suilend-notes.md` "Blocking findings" 1-3: public source (commit `d5ba83a617bb0778b48b0c3b1e77a87be81258ca`, package version 20) does not correspond to the deployed version-25 package; SDK calls a Pyth-integration function (`refresh_reserve_price_pro_compatible`) absent from that public source; reserve-to-`PriceInfoObject` mappings are unverified; no valuation-aligned independent reference exists yet. |

No other deployment or reserve path has been investigated to a different
conclusion. This table has one row because the repository targets one
deployment (`deploy/config/suilend.toml`'s main market); the 26 discovered
market-type instances from the full census are explicitly not verified or
endorsed (`docs/suilend-notes.md` and `README.md` both already state this).

## Missing inputs (temp.md section 0 item 7)

Recorded exactly, per the instruction to name the missing input rather than
proceeding past it:

1. **Owner.** No individual has been assigned accountability for Suilend
   deployed-source verification. Required before a real five-day spike can
   start (temp.md section 3's research decision gate).
2. **Independent reviewer.** No second person is assigned to review the
   owner's evidence matrix. Required by the same gate.
3. **A public source revision with established correspondence to package
   version 25.** The currently available public branches (`devel`,
   `mainnet`) both resolve to a commit recording package version 20. Finding
   or obtaining a version-25-correspondent revision (from the maintainer, a
   verified block explorer decompilation, or an authenticated private
   repository) is an external dependency this session cannot satisfy.
4. **Per-reserve oracle object addresses and acceptance rules.** Needs
   either maintainer-provided documentation or live on-chain inspection of
   each reserve's current `PriceInfoObject` wiring, cross-checked against
   the SDK's legacy/upgraded Pyth object distinction.
5. **A valuation-aligned independent reference** (deployed refresh-path
   simulation at matching object versions/price inputs/chain time, or
   another aligned source) - needed to validate any future fresh-pricing
   implementation once 3 and 4 are resolved.

## Next review date

Not scheduled. Per temp.md section 3: "At the deadline, unresolved paths
remain ineligible, R1/R2 continue, and the owner records ... the next
review date." Since no owner is yet assigned, no next-review date can be
set by this agent; the person assigning an owner should set it then.

## Effect on this branch's other work

No code in F01-F03 (this branch) claims, computes, or depends on verified
Suilend valuation. `calculation_status` remains
`"unverified_cached_diagnostics"`, `overall_grade` remains `None`, and the
stress module's score gating (F03) is explicitly unchanged pending this
gate. This record does not unblock V01 or any task depending on
"F04 verified scope."
