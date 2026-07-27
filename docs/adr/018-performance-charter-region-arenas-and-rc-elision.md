# ADR-018: Performance Charter, Region Arenas, and Retain/Release Elision

## Status

Proposed 2026-07-27 (pending ratification).

Composes with ADR-006 (value and memory model), ADR-010 (verified region
teardown — accepted, currently design-only), ADR-016 (evidence honesty), and
the R15 native-witness ruling. This ADR makes performance a chartered,
measured program instead of an assumed property of strict typing.

## Context

Shape's structural position against speculative-JIT runtimes is strong: no
deopt cliffs to speculate around, no hidden-class machinery, no tag checks,
8-byte typed slots, monomorphized generics, and contiguous native arrays. Its
structural disadvantages are equally concrete: Cranelift optimizes less than
mature speculative tiers, and atomic refcounting plus a cycle collector loses
to generational bump allocation on allocation-heavy code.

The ground truth at 2026-07-27 makes the gap between design ceiling and
shipped behavior explicit:

- deopt granularity is whole-program: roughly a dozen bail sites abandon
  native execution of the entire program when any one construct is
  unsupported, and the CLI compiles whole-program up front rather than using
  the implemented tier machinery;
- only closures originating from stack-closure direct dispatch run natively;
  every other closure call re-enters the bytecode VM through a trampoline,
  and any method call on an iterator receiver bails the whole function
  because a JIT-format closure argument cannot cross the VM's typed carrier
  ABI. One array-method FFI entry is an aborting `todo!()`;
- a real bounds-check-elision pass is live but narrow: it removes checks
  only for bare `local[local]` in canonical counted loops;
- thirteen optimizer components totaling ~5,350 lines (affine bounds
  guards, LICM, vectorization, loop lowering, escape analysis, call-path,
  correctness, cross-function, HOF inlining, numeric arrays,
  table-queryable, typed-MIR, and their shared plan type) exist with zero
  external callers — dead code that invites "we already have BCE/LICM"
  over-claims;
- retain/release traffic is emitted at ~837 scattered call sites; the only
  elision is the solver's move-vs-clone decision, `var` bindings are pinned
  off the move path, and no dominated retain/release pair cancellation
  exists anywhere;
- no live value-escape analysis exists ("this allocation dies with this
  frame or region" is not provable today; the only implementation is a dead
  bytecode-level, single-basic-block pass inside the uncalled optimizer
  module), there is no central allocation seam (twelve files call
  `std::alloc` directly; `TypedArray` performs two allocations per array),
  and the cycle collector tracks header-less kinds in an address-keyed side
  table that wholesale freeing would silently corrupt.

None of these are design defects of ADR-006/010 — they are unscheduled work.
This ADR schedules it and fixes the decisions that are hard to reverse.

## Decision

### 1. The performance charter is a measured, falsifiable contract

Shape maintains one committed comparison suite of typed workloads with
pinned harness, environment, and reference-runtime versions (the exact Node
LTS current at ratification, pinned to a specific V8 build in the suite
manifest; other runtimes may appear as informational, non-gating columns).

The charter target (user-ratified 2026-07-27) is **per-category bars**, not
a single geomean — each PERF lane landing ratchets its own category, and no
category hides behind another:

| Category | Example workloads | Bar vs pinned Node |
|---|---|---|
| Numeric kernels | bspline (the historical 69×-vs-Rust case), matrix ops | ≥ 1.5× |
| Collection pipelines | map/filter/reduce chains with closures | ≥ 1.0× (post closure-nativity) |
| Strings / JSON | parse–transform–serialize | ≥ 1.0× |
| Allocation-heavy | object-graph churn | ≥ 0.8× pre-arena, ratcheting to ≥ 1.0× when arenas land |
| Startup | hello-world, CLI-tool cold start | ≥ 5× |

The multipliers receive exactly one recorded calibration after the first
measured baseline run; any later change is a dated decision, not a quiet
edit. No individual workload regresses release-over-release without a
recorded reason.

The benchmark-integrity rule is preserved verbatim: benchmarks measure the
compiler; the compiler never gets to rewrite the benchmarks, and adding
hints to help the JIT remains forbidden. Performance claims follow ADR-016
discipline — a number without a committed harness, exact revisions, and
environment identity is not evidence, and any native-execution claim carries
the R15 `NativeExecutionWitness`. Non-goal: LLVM-peak scalar parity with
Rust; conceded explicitly rather than chased.

### 2. Deopt granularity becomes per-function; tiering becomes the default

An unsupported construct costs its enclosing function native execution,
never the program. The whole-program bail set is a generated shrink-only
baseline ratcheted to zero; a new whole-program bail cannot be added. The
CLI's default execution mode becomes the tiered interpreter-first pipeline
the VM already implements (T1@100/T2@10k with OSR); explicit whole-program
compilation remains available as an opt-in mode, not the default. Fallback
remains loud and structured — the silent-divergence class stays closed.

### 3. Retain/release elision is a MIR-level pass over existing solver facts

The borrow solver already computes per-point loan liveness and
move-vs-clone ownership decisions; the existing `LoadLocalMove` path is the
proof that elision composes with the value model. This ADR extends that
seam, in order:

- unpin `var` destinations from the move path by retiring the forced
  `SharedCow` storage flag (jointly owned with ADR-017's script-tier
  truthfulness) so the refined aliased-and-mutated rule decides storage;
- add a MIR pass canceling dominated retain/release pairs, using the
  solver's existing liveness and loan intervals — no new analysis
  infrastructure. A pair is elidable only when one of two conditions is
  proven for the entire elided interval: a covering owning reference
  provably keeps the value live across it, or the interval contains no
  allocation or other collection safepoint. Without that condition, an
  elided retain leaves a live frame reference uncounted, and a cycle
  collection triggered inside the interval can find the value's subgraph
  internally balanced and free it under the frame — the exact hazard the
  increment barrier's coloring exists to prevent;
- decouple the collector's barrier side effects from the refcount
  arithmetic for cycle-capable kinds: an elided pair may drop the atomic
  operations, but the candidate-production semantics of the release path
  must be preserved (or the value proven unable to be a cycle root within
  the interval), so that elision cannot starve the candidate buffer and
  postpone cycle collection unboundedly;
- express all elision at the MIR/opcode level the compiler controls.
  Executor-internal refcount operations are invisible to this pass and out
  of scope; centralizing them is not required for the win and not attempted
  here.

Correctness gate: refcount-balance differential tests (identical final
counts and finalization order with the pass on and off) on every slice, a
forced-collection differential that triggers cycle collection **inside**
every elided interval class (end-state comparison alone cannot catch a
mid-interval free), and a cycle-collection completeness assertion — known
cycle fixtures still collect within a bounded number of safepoints with the
pass on.

### 4. Region arenas are sequenced behind their three real prerequisites

ADR-010 §4 already licenses the semantics this ADR needs: only transitively
proven unobservable memory retirement may be reordered, batched, or elided —
finalization order is untouched; arenas batch memory retirement only. But
ADR-010 has zero implementation today, and three prerequisites are missing
and independently valuable:

1. **Value-escape analysis** (outbound): a proof that every allocation in a
   region dies with the region. The existing reference-escape promotions
   cover references only, and the sink-blind `detect_escape_status`
   heuristic admits false positives — the storage planner's own promotion
   rule explicitly refuses to consume it for exactly this reason. Prior
   art exists and must be dispositioned first: the dead
   `optimizer/escape_analysis.rs` implements exactly these criteria but at
   bytecode level, single-basic-block, JIT-side — it is §6 inventory, and
   its wire-or-delete disposition precedes this prerequisite. The live
   analysis is a MIR solver product, and it independently benefits RC
   elision and stack promotion before any arena exists.
2. **Inbound-reference analysis**: the collector-exemption proof constrains
   references **into** the region as well — an outside cycle-capable value
   stored into a region-allocated object creates a boundary-crossing edge
   that outbound escape analysis says nothing about. This is a distinct
   proof obligation and a named prerequisite, not a corollary of (1).
3. **One allocation seam**: a single allocator API through which typed heap
   carriers allocate, replacing the twelve files making direct `std::alloc` calls, with
   the `TypedArray` header+buffer double allocation collapsed. The seam
   lands with system-allocator semantics first — a pure refactor gated by
   allocation-count and layout-identity tests — and only then gains a
   region-arena backend.

Arena semantics, once the prerequisites exist and the ADR-010 region-plan
pipeline is live: allocations proven region-confined come from a per-region
bump arena; region teardown runs declared finalization in ADR-010's proven
order, then retires the arena's memory in bulk. The cycle collector is
co-designed, not layered: region-confined allocations are exempt from
candidate buffering only under the combined outbound + inbound proof that
no cycle-capable reference crosses the region boundary in either direction.
Bulk retirement must be genuinely O(1) plus finalization: the collector's
per-address freed-object notification exists to defeat address-reuse
aliasing, and a bump arena reusing its range is exactly that case — so the
co-design includes a range-based invalidation primitive for the candidate
buffer and side table, invalidating the arena's whole address range in one
operation instead of enumerating retired objects. An arena without these
proofs, or a "temporary" arena behind a feature flag, is the exact
walk-back shape §Forbidden Patterns exists to refuse.

### 5. Bounds-check elimination widens the live matcher

The shipped elision pass is sound and wired; its ceiling is the index-shape
matcher, which admits only bare `local[local]`. The matcher widens to
constant indices with proven length bounds, `iv ± constant` offsets with
adjusted range proofs, and field-projected array receivers proven
unreassigned — each widening preserving the non-negativity proof
independently, because the unchecked path skips index normalization as well
as the check. Interpreter-side checks are untouched: BCE is a native-tier
optimization, never a semantic change. Speculative guards with runtime
deoptimization are rejected; every elision is a static proof, in keeping
with the no-dynamic-fallback discipline.

### 6. Dead analyses are wired or deleted; there is no third state

The uncalled optimizer module is a parallel implementation held in reserve,
which this codebase's own discipline forbids. The disposition inventory is
**all thirteen** components — bounds, LICM, vectorization, loop lowering,
escape analysis, call-path, correctness, cross-function, HOF inlining,
numeric arrays, table-queryable, typed-MIR, and the shared plan/cache
types — not a highlight subset; enumerating four and leaving nine would
land the nine in precisely the third state this section forbids. Each
component gets exactly one disposition in the workstream: wired into the
live MIR pipeline behind a measured ticket, or deleted with its tests.
"Keep it for later" is not a disposition. The affine bounds analysis is
evaluated first as the engine for §5's widened matcher, and
`escape_analysis.rs` is dispositioned before §4's prerequisite (1) starts;
LICM and vectorization are expected deletions with re-derivation later from
the live pipeline, unless their wiring tickets can demonstrate a measured
win at acceptable risk.

### 7. Closure and HOF nativity is re-scoped to the current defect

The historical description (Cranelift arity bug, `todo!()` on `var`
capture) is fixed and must not be re-litigated; R15's fresh-repro rule
applies. The live work is:

- widen stack-closure direct dispatch to first-class and escaping closures
  so ordinary closure calls stop trampolining into the VM;
- unify the closure-argument carrier across the JIT/VM boundary so iterator
  and array HOF receivers stop bailing whole functions;
- replace the aborting array-method FFI `todo!()` with a structured
  per-function bail until the carrier work lands.

`.map`/`.filter` chains executing natively is the acceptance bar, witnessed
per R15.

## Grounding (2026-07-27)

- Whole-program bails: ~12 sites in `crates/shape-jit/src/executor.rs`
  (:121–:621) plus `compiler/strategy.rs:40,:226`; CLI whole-program
  entry `bin/shape-cli/src/commands/script_cmd.rs:1589`; tier machinery
  `crates/shape-vm/src/tier.rs:29`, OSR `executor/osr.rs:538`,
  worker `shape-jit/src/worker.rs:147`.
- Closure state: native fast path `mir_compiler/terminators.rs:1646`
  (stack-closure direct dispatch); VM trampoline fallback `:2016`; iterator
  whole-function bail `:198`; aborting FFI
  `ffi/call_method/array.rs:25`; regression suite
  `mir_compiler/closure_dispatch_regression_tests.rs`.
- Live BCE: `mir_compiler/bounds_elision.rs` wired at
  `compiler/program.rs:331`; matcher ceiling
  `mir_compiler/places.rs:725`; unchecked accessors `places.rs:694,:709`.
- Dead analyses: `crates/shape-jit/src/optimizer/`
  ({bounds,licm,vectorization,loop_lowering}.rs), zero external callers
  (sole live import: `Tier2CacheKey`).
- RC elision seam: `compute_ownership_decisions`
  (`crates/shape-vm/src/mir/solver.rs:1788`) → `LoadLocalMove`/`Clone`
  (`compiler/helpers_binding.rs:360`); `var` pinned non-consuming
  (`solver.rs:1810`); ~837 `clone_with_kind`/`drop_with_kind` sites
  workspace-wide; no pair-cancellation pass exists.
- Arena prerequisites: reference-only escape promotions
  (`solver.rs:1453`); the sink-blind `detect_escape_status` heuristic that
  the promotion rule explicitly refuses for value-escape purposes
  (`mir/storage_planning.rs:996`); twelve files with direct `std::alloc`
  files; `TypedArray` double allocation
  (`crates/shape-value/src/v2/typed_array.rs:63`); only cross-cutting seam
  today is the advisory `alloc_budget.rs`; GC side-table/freed-address
  contract (`gc.rs:1284 gc_note_object_freed`, side table for header-less
  kinds); ADR-010 nouns have zero code hits.
- `var` force-`SharedCow` flag: `mir/storage_planning.rs:37-42`.

## Consequences

- Performance becomes a gated program with a falsifiable target instead of a
  hoped-for by-product; every claim is bound to a committed harness.
- The largest single win (per-function deopt granularity) is scheduled ahead
  of exotic optimizations, because one unsupported construct currently
  cancels every other optimization's benefit.
- Escape analysis and the allocation seam land as independent, testable
  slices whose value does not depend on arenas ever shipping — the arena
  bet is sequenced, not load-bearing.
- ~5,350 lines of dead analysis across thirteen components get an explicit disposition, closing a
  standing over-claim surface.
- The GC/arena co-design constraint is recorded before either side
  entrenches further.

## Rejected alternatives

- **Speculative guards with runtime deopt (V8-style).** Reintroduces dynamic
  fallback and silent performance cliffs; every elision here is a static
  proof.
- **Keep the dead optimizer module as a future asset.** A parallel
  implementation in reserve is the defection-attractor shape this repo
  refuses elsewhere; wire it or delete it.
- **Centralize all executor-internal RC calls before eliding.** An 837-site
  refactor as a prerequisite would stall the win; MIR-level elision needs
  none of it.
- **Arena allocation behind a feature flag without the escape proof.** That
  is the W-series walk-back shape: a "temporary" unsound fast path that
  becomes permanent.
- **Chasing Rust-level scalar peak.** Wrong target; the charter's reference
  is the speculative-JIT class Shape's typing advantage actually applies
  to.
- **Modifying benchmarks to demonstrate wins.** Already forbidden; restated
  because performance programs are where the pressure appears.

## Related decisions

- ADR-006: Value and Memory Model
- ADR-010: Verified Region Teardown and Callable Lifecycle (§4, §7)
- ADR-014: Closed Effects and Static Capability Ownership
- ADR-016: Executable Public Feature Documentation
- ADR-017: Ergonomic Parity and Progressive Disclosure
- R15 — Native claims require a fresh repro and `NativeExecutionWitness`
