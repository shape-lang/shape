# ERGO/PERF/POLY workstream publication — closeout record

Status: **published and independently audited** (2026-07-27). The committed
plan ([publication-plan.json](./publication-plan.json), commit `cdb0be5d`,
SHA-256 `89a6c37100c05502443854958f1cae28eeb3d96fd39a776a56c93711a50fe2db`)
executed 2026-07-27 16:04:13Z–16:05:02Z; the independent read-only re-fetch
audit passed with **zero discrepancies** across all eight checks.

Authority: grill rulings Q1 B / Q2 A / Q3 C / Q4 as-recommended / Q5 B +
enactment go-ahead, user, 2026-07-27 ([ratification-grill.md](./ratification-grill.md)).
Authority-set commit: `87f51f61` (rides AUTHORITY-BASELINE #111).

## Published state

- **26 new issues, #178–#203** (contiguous, no collision with the prior
  program's #111–#177): the 25 ratified ERGO/PERF/POLY tickets plus
  `EFFECT-ROW-IN-TYPE` #178, the R21 row-in-type carrier for the Q2 edge
  into #143 (disclosed in the plan).
- **28 native blocker edges** among/from the 26; body Blocked-by sections
  byte-equal the native dependency endpoints on all 26 (audited).
- **Q2 delta applied exactly**: #110 gained blocker #182 (ERGO-QUASIQUOTE);
  #143 gained blocker #178; both bodies amended with the matching line and
  nothing else structural in-window (timeline-audited). Scope-by-reference
  comments on #112, #113, #143, #163 (one comment each, exact plan text).
- **No stray mutations**: full since-midnight sweep accounted for every
  touched issue; edge-set diff over all 86 reachable prior-program nodes
  vs the committed manifest = zero beyond the two authorized edges.
- **Acyclicity**: full blocked_by closure crawled — 113 nodes, 303 edges,
  0 cycles (DFS + independent Kahn sort).
- **Readiness**: `ready-for-agent` applied post-audit to the 7 zero-blocker
  issues: #178, #179, #181, #186, #194, #196, #201.

## Symbolic mapping

| Symbolic ID | Issue |
|---|---|
| EFFECT-ROW-IN-TYPE | [#178](https://github.com/shape-lang/shape/issues/178) |
| ERGO-FIX-CHANNEL | [#179](https://github.com/shape-lang/shape/issues/179) |
| ERGO-CONTRACT-FIXIT | [#180](https://github.com/shape-lang/shape/issues/180) |
| ERGO-VAR-TRUTH | [#181](https://github.com/shape-lang/shape/issues/181) |
| ERGO-QUASIQUOTE | [#182](https://github.com/shape-lang/shape/issues/182) |
| ERGO-SUPERVISOR-SCOPE | [#183](https://github.com/shape-lang/shape/issues/183) |
| ERGO-TEACHING-DIAG | [#184](https://github.com/shape-lang/shape/issues/184) |
| ERGO-CEREMONY-GATE | [#185](https://github.com/shape-lang/shape/issues/185) |
| PERF-SUITE | [#186](https://github.com/shape-lang/shape/issues/186) |
| PERF-DEOPT-GRANULARITY | [#187](https://github.com/shape-lang/shape/issues/187) |
| PERF-CLOSURE-NATIVE | [#188](https://github.com/shape-lang/shape/issues/188) |
| PERF-HOF-CARRIER | [#189](https://github.com/shape-lang/shape/issues/189) |
| PERF-RC-ELISION | [#190](https://github.com/shape-lang/shape/issues/190) |
| PERF-BCE-WIDEN | [#191](https://github.com/shape-lang/shape/issues/191) |
| PERF-DEAD-OPT | [#192](https://github.com/shape-lang/shape/issues/192) |
| PERF-ESCAPE | [#193](https://github.com/shape-lang/shape/issues/193) |
| PERF-ALLOC-SEAM | [#194](https://github.com/shape-lang/shape/issues/194) |
| PERF-ARENA | [#195](https://github.com/shape-lang/shape/issues/195) |
| POLY-STUB-CHANNEL | [#196](https://github.com/shape-lang/shape/issues/196) |
| POLY-FOREIGN-CHECK | [#197](https://github.com/shape-lang/shape/issues/197) |
| POLY-ENV-PIN | [#198](https://github.com/shape-lang/shape/issues/198) |
| POLY-ZERO-COPY | [#199](https://github.com/shape-lang/shape/issues/199) |
| POLY-FOREIGN-REF | [#200](https://github.com/shape-lang/shape/issues/200) |
| POLY-ASYNC-TRUTH | [#201](https://github.com/shape-lang/shape/issues/201) |
| POLY-ASYNC-OFFLOAD | [#202](https://github.com/shape-lang/shape/issues/202) |
| POLY-LSP-FENCE | [#203](https://github.com/shape-lang/shape/issues/203) |

## Recorded decisions and limits

- **Readiness-label convention**: #110 and #143 keep `ready-for-agent`
  while newly blocked. This follows the prior program's published
  convention — "native edges, not that label, determine the runnable
  frontier" ([tracker-publication.md](../adr011-012/tracker-publication.md)).
  This wave's stricter zero-blocker rule governed only which NEW issues
  received the label. Agents selecting work must use native edges.
- **Audit limit (on the record)**: GitHub exposes no body-edit history and
  no committed baseline of #110/#143 body prose exists, so "only one line
  changed" in those two bodies is timeline-and-structure-supported, not
  byte-provable. Blocked-by sets ARE proven against the committed manifest.
- Wave-1 lane assignment (Q1 B) at dispatch: spine = #111 evidence
  completion (then #133/#134/#135 → #136 → #91 — #91 is natively blocked
  by #136); lane A = #186; lane B = #179 then #181; lane C = #201 then
  #196, #202 after #201 gates.

## Audit provenance

Independent auditor, read-only (231 GET requests, rate-limit residual
4669/5000). Raw fetches, closed digraph, and the body/edge verifier script
retained in the session scratchpad (`audit/`); this record is the durable
summary.
