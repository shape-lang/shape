# Value-escape and inbound-reference analysis (#193)

**Authority:** ADR-018 §4 (the two prerequisites and why the sink-blind
heuristic was disqualified), R24, `docs/program/workstreams/perf.md`
PERF-ESCAPE row.
**Artifact:** `crates/shape-vm/src/mir/escape.rs`, published on
`StoragePlan::escape` (`crates/shape-vm/src/mir/storage_planning.rs`).
**Consumers (designed for, not implemented):** PERF-RC-ELISION (#190) breadth,
stack promotion, PERF-ARENA (#195).
**Recorded at:** wave4-infra, on top of `60ef72a8`.

This slice lands the analysis INERT: the facts are computed and published, and
nothing consumes them to change codegen. Blast radius on behaviour is zero by
construction.

## 1. What the two products are

Both are per-allocation-site facts, keyed by `AllocSite { block, statement }`
and returned in site order.

**(a) Outbound value-escape** — `OutboundEscape::{FrameConfined,
Escapes(vector, span), NotProven(reason)}`. "This allocation dies with the
frame." Distinct from the solver's `reference_escape_promotions`
(`solver.rs:1453`), which answer whether a *reference* to a local outlives the
frame; that verdict is consumed here as one of the disproving vectors
(`EscapeVector::ReferenceEscape`), not as a substitute for the product.

**(b) Inbound-reference** — `InboundProof::{NoForeignStores,
ForeignStore(source, span), NotProven(reason)}`. "No outside cycle-capable
value is stored into this allocation." ADR-018 §4 names it a distinct
obligation; it is computed and reported independently, and
`EscapeFacts::region_exemption_candidates()` is the *only* API that hands out
the conjunction, so a consumer cannot reach the arena exemption by filtering
`frame_confined()` and silently dropping the inbound half.

An allocation site is a statement that materializes a fresh heap value:
`ArrayStore`, `ObjectStore`, `EnumStore`, `ClosureCapture`. `Rvalue::Aggregate`
is deliberately **not** a site — MIR lowering emits it as a generic
multi-operand carrier for things that are not allocations (`lowering/
helpers.rs:404` for a binary-op surface, `lowering/expr.rs:452` for match-branch
operand merging). It is honoured as a flow edge instead.

## 2. Why not the sink-blind heuristic

`storage_planning::detect_escape_status` chases `Assign(Place::Local(d), rv)`
edges to `SlotId(0)` and nothing else. Two consequences, both reproduced as
tests:

- It **misses** every escape that leaves through a non-`Local` place or a
  container-store statement. `let a = [0]; let inner = [1,2]; a[0] = inner; a`
  writes through `Assign(Place::Index(..), ..)`, so the heuristic reports the
  escaping `inner` as `Local`
  (`tripwire_b0004_container_referent_class_is_seen_through` asserts both the
  heuristic's blind spot and this analysis's correct verdict).
- It **over-reports** the B0004 container-referent class: `Rvalue::Aggregate
  [&x]` counts as return-flow, which is exactly why the storage planner's
  Rule 3c (`storage_planning.rs:996`) refuses to consume it for promotion.
  `tripwire_b0004_container_referent_stays_confined_when_nothing_escapes`
  pins that this analysis distinguishes the two cases.

## 3. Mechanism

One monotone fixed point over the MIR, two bitset tracks per slot:

- `holds[slot]` — allocations the slot may hold **or contain**. Drives the
  outbound product: an escape of a container escapes its members, which is how
  container membership is *seen through*.
- `aliases[slot]` — allocations the slot may name **itself** (copy/move/clone
  and borrow chains only; `Aggregate` and container stores do not propagate
  into it). Drives the inbound product: writing into a container is not
  writing into that container's members, and each member carries its own
  inbound verdict.

A separate provenance map answers "is this value locally produced" and decides
whether a container store is containment (local container) or an escape
(foreign container), and classifies stored values for the inbound whitelist. It
is a greatest fixed point: everything starts local and is demoted by
parameters, call destinations, and any rvalue shape not on the whitelist, so an
unrecognized shape demotes rather than survives.

Determinism (#205): every structure iterated is positional — `Vec<u64>`
bitsets, slot-indexed `Vec`s, block/statement order. There is no `HashMap`
iteration anywhere in the analysis, and `promoted_referents` is sorted before
use. Asserted by `facts_are_identical_across_repeated_analyses` and
`allocations_are_ordered_by_site_not_by_hash_iteration`.

## 4. Soundness preconditions

- **`had_fallbacks` ⇒ everything `NotProven(MirLoweringIncomplete)`.** Mirrors
  the storage planner's own rule (`storage_planning.rs:275`). MIR is only a
  faithful over-approximation of program dataflow when lowering did not fall
  back; a vector table run over an incomplete MIR proves nothing.
- **Calls are opaque.** Any allocation passed to a call — including as the
  receiver of a method call, which lowers to `args[0]` — escapes, unless the
  callee's own `FunctionBorrowSummary::closure_param_escapes` proves the
  parameter non-escaping. The callee operand escapes too: invoking a closure
  hands it its own environment and the body can stash a capture anywhere.
- **`snapshot()` is opaque** — every argument escapes (taken from the existing
  Phase-B closure analysis).
- **Whitelist, not blacklist, for inbound.** A stored value is clean only if it
  is a scalar (`LocalTypeInfo::Copy`), a leaf literal, or a slot of proven
  local provenance. Notably `MirConstant::Function(name)` is **not** treated as
  an inert literal: MIR lowering emits that variant for any identifier that
  does not resolve to a local (`lowering/expr.rs:2014`), which includes
  module-level bindings.

Cycle-capability is a runtime `NativeKind`/`HeapKind` property
(`shape-value/src/gc.rs::cycle_capable_direct_header`) that MIR does not carry —
`LocalTypeInfo` distinguishes only Copy / NonCopy / Unknown. Every non-`Copy`,
non-local value is therefore treated as potentially cycle-capable. That
over-approximates the offending set, which can only withhold the exemption,
never grant it wrongly.

## 5. Prior art: `shape-jit/src/optimizer/escape_analysis.rs`

Read per the ADR-018 §4 rule. It is a dead, bytecode-level,
single-basic-block, JIT-side scalar-replacement planner (743 lines). Nothing
was wired; this analysis is independent MIR code. Its wire-or-delete
disposition remains #192's, and this table is the input to that decision.

| From the prior art | Taken / rejected | Why |
|---|---|---|
| Escape criteria list: not returned, not stored to a heap object/closure, not passed to a call | **Taken** — reimplemented over the MIR vector table | The criteria are right; the level was wrong |
| `is_escaping_call` — every call opcode captures its arguments | **Taken** in spirit (all calls opaque), **refined** by the existing `closure_param_escapes` callee summaries | Same conservatism, with a live refinement seam |
| "Array is not stored to the heap ⇒ escape" (containment = escape) | **Rejected** | Too coarse for arenas. Containment is tracked as membership, so a value in a non-escaping local container stays confined. This is the precision #195 needs |
| Single-basic-block confinement; any block boundary kills the candidate | **Rejected** | The MIR analysis is whole-function and control-flow-insensitive-but-sound; a loop-local allocation is exactly the arena case and the prior art discards it |
| `MAX_SCALAR_ARRAY_ELEMENTS = 8` size cap; `NewArray`-only candidates | **Rejected** | An artifact of scalar replacement, not of escape. Objects, enums and closure environments are allocation sites here, with no size cap |
| Constant-index `GetProp`/`SetLocalIndex` use pattern matching, `resolve_constant_index`, the `NewArray`+`StoreLocal` adjacency pattern | **Rejected** | Bytecode peephole shapes with no MIR analogue; they encode scalar-replacement eligibility, not escape |
| `ScalarArrayEntry` get/set site maps | **Rejected** | Scalar-replacement bookkeeping, out of scope for an escape fact |
| Jump-target reconstruction from operand offsets (two passes, one of which is a no-op loop) | **Rejected** | MIR has a real CFG |
| Inbound-reference reasoning | **Absent from the prior art entirely** | It never considered references *into* the candidate — ADR-018 §4 prerequisite 2 has no prior art to take |

Net for #192: the module's *criteria* survive in this ticket; its *mechanism*
(bytecode peephole, single-block, size-capped, scalar-replacement-specific) has
no remaining consumer. If #192 deletes it, the evidence a future
re-derivation must produce is a measured scalar-replacement win at the JIT
tier — which is a different optimization from escape analysis and would be
built on this MIR product, not on the deleted module.

## 6. Measured precision (R24: no measurement, no close)

`precision_report_on_charter_workloads` (committed; run with `--nocapture`)
over the 11 committed charter workloads:

| workload | allocation sites | frame-confined | exemption-eligible |
|---|---|---|---|
| alloc_tree | 1 | 0 | 0 |
| alloc_object_graph | 1 | 0 | 0 |
| collections_hashmap | 1 | 0 | 0 |
| closures_dispatch | 1 | 0 | 0 |
| the other 7 | 0 | — | — |
| **suite** | **4** | **0 (0%)** | **0 (0%)** |

A wider corpus (the 255 parsable book-acceptance programs, 807 allocation
sites) gives the more informative distribution:

| verdict | share |
|---|---|
| frame-confined | 1.6% |
| `Escapes(CallArgument)` | 56.3% |
| `Escapes(Return)` | 42.0% |
| `Escapes(ClosureCapture)` | 0.1% |

**The honest number for #195 is that today this analysis proves ~0% of the
charter's allocation-heavy workload allocations frame-confined.** Two reasons,
both structural and both outside this ticket:

1. **Call-argument conservatism dominates (56% of all sites).** The receiver of
   a method call is `args[0]`, so `nodes.push(Node { .. })` escapes both the
   node and the array; even `a.len()` escapes `a`. Refining this needs a
   per-builtin-method escape contract. It must come from an analyzed contract,
   not from matching method names — name-matched semantics is exactly what
   §Forbidden Patterns refuses, and the `closure_param_escapes` seam this
   analysis already consumes (proved live by
   `callee_summary_keeps_a_non_escaping_argument_confined`) is the correct
   extension point.
2. **The hot containers are not visible as allocation sites.** `let mut nodes:
   Array<TreeNode> = []` emits no container-store statement at all — MIR
   lowering early-returns for empty non-object literals (`lowering/
   helpers.rs:112`). The array that an arena would most want to own is
   therefore invisible to any MIR-level allocation analysis.

Cost of running the analysis, measured over the 255-program corpus (release
build): **2 ms total, against 21 ms for MIR lowering of the same corpus** —
about 10% of lowering, and nothing at runtime.

## 7. Open questions left to consumers

- **#195 cannot be planned on these numbers as they stand.** The prerequisite
  is landed and sound, but its yield on the workloads that motivated arenas is
  zero until (1) and (2) above are addressed. That is a sequencing input, not
  a defect in the analysis: an optimistic escape fact would be far worse.
- **Interprocedural region confinement is not attempted.** `build_graph`
  returns its array, so the array escapes *that* frame even though it dies
  inside `churn`'s round. Region-level (as opposed to frame-level) confinement
  needs the ADR-010 region-plan pipeline, which has no implementation.
- **The `non_escaping_closure_slots` overlap is deliberate but temporary.**
  The Phase-B closure analysis in `storage_planning.rs` answers the same
  escape-vector question for closure slots only. This ticket did not re-express
  it in terms of the new product because that would be a behaviour-affecting
  refactor inside a pure-analysis slice; whoever first consumes either fact
  should collapse them, and until then the duplication is recorded here rather
  than left implicit.
