# PERF workstream — performance charter

Authority: ADR-018, R24, R15. Charter target (user-ratified 2026-07-27,
Q5 = B): per-category bars vs the pinned Node LTS — numeric ≥ 1.5×,
collections ≥ 1.0× post-closure-nativity, strings/JSON ≥ 1.0×,
allocation-heavy ≥ 0.8× ratcheting to ≥ 1.0× post-arena, startup ≥ 5× —
one recorded calibration allowed after the first baseline; per-ticket
before/after measurements; immutable benchmarks.

## Tickets

### PERF-SUITE — the comparison suite and harness

Scope: committed suite (numeric kernels, collection pipelines, strings,
allocation-heavy graphs, closure-heavy code) with pinned Node LTS reference,
pinned environment manifest, and a one-command harness emitting a
machine-readable report (revisions, binary hash, environment, per-workload
results). Extends `shape/benchmarks/` without modifying existing files.
Blocked by: none. Blocks: every other PERF ticket's close (no measurement,
no close).
Tripwires: (1) two consecutive runs on the same revision agree within a
declared noise bound, asserted in CI; (2) the report refuses to render a
comparison when environment identities differ; (3) benchmark-file hashes are
asserted (integrity rule made mechanical).

### PERF-DEOPT-GRANULARITY — per-function bails, tiered CLI default

Scope: convert whole-program bail sites (~12 in `shape-jit/src/executor.rs`,
plus `compiler/strategy.rs:40,:226`) to per-function interpreter fallback;
generate the whole-program-bail baseline and ratchet to zero; make tiered
execution (T1@100/T2@10k + OSR, already implemented in
`crates/shape-vm/src/tier.rs`) the CLI default with whole-program compile as
opt-in.
Blocked by: PERF-SUITE.
Tripwires: (1) a program with one unsupported construct keeps every other
hot function native (asserted via NativeExecutionWitness on a two-function
fixture); (2) the bail baseline increasing fails CI; (3) suite geomean does
not regress under the tiered default.

### PERF-CLOSURE-NATIVE — first-class closure dispatch

Scope: widen stack-closure direct dispatch (`terminators.rs:1646` side
table) to first-class and escaping closures so ordinary closure calls stop
trampolining through `jit_call_value` (`terminators.rs:2016`). Fresh-repro
rule per R15; the historical arity/`todo!()` framing is fixed and must not
be re-litigated. Role disclosure: this ticket IS the "separate
post-callable-carrier ticket" R15 mandates for the fresh capturing-HOF
repro and, if still red, the general first-class-callable repair — it
assumes that R15 role explicitly rather than inverting it silently; #97's
native close criterion and HOF-NATIVE-TRACER (#146) consume its witness,
exactly as R15 specifies.
Blocked by: PERF-SUITE; NATIVE-WITNESS (#117).
Tripwires: (1) `NativeExecutionWitness` for a capturing closure called
through a variable, a parameter, and a return value; (2) zero
`PENDING_CALL_ERROR` deopt checks on covered paths; (3) VM/JIT differential
on the closure-dispatch regression suite stays green.

### PERF-HOF-CARRIER — closure-argument carrier unification

Scope: unify the closure-argument carrier across the JIT/VM boundary so
`Ptr(HeapKind::Iterator)` receivers stop whole-function bailing
(`terminators.rs:198`); immediate sub-slice: replace the process-aborting
`ffi/call_method/array.rs:25` `todo!()` with a structured per-function bail.
Blocked by: PERF-CLOSURE-NATIVE.
Tripwires: (1) `arr.map(|x| x * 2.0)` executes natively with a witness;
(2) the aborting-`todo!()` fixture becomes a loud fallback line, not a
SIGABRT; (3) the ~17 stale deleted-path `#[ignore]` tests in this territory
are re-pointed or deleted (no silent ignore inheritance).

### PERF-RC-ELISION — retain/release pair cancellation

Scope: MIR pass canceling dominated retain/release pairs using existing
`loans_at_point`/liveness facts (`solver.rs:1010,:1788`); consume
ERGO-VAR-TRUTH's unpinning so qualifying `var` bindings take the move path
(`solver.rs:1810` caveat removed).
Blocked by: PERF-SUITE; joint with ERGO-VAR-TRUTH.
Elision legality per ADR-018 §3: covering owning reference across the whole
interval, or a safepoint-free interval; barrier side effects decoupled from
the refcount arithmetic for cycle-capable kinds.
Tripwires: (1) refcount-balance differential — identical final counts and
finalization order with the pass on/off across the auto-drop and
declared-capture-teardown suites; (2) forced-collection differential:
cycle collection triggered INSIDE each elided interval class must not free
the value (end-state comparison cannot catch a mid-interval free);
(3) cycle-collection completeness: known cycle fixtures still collect
within a bounded number of safepoints with the pass on; (4) measured
retain/release dynamic count reduction on the allocation-heavy suite
reported in the ticket.

### PERF-BCE-WIDEN — widen the live bounds-elision matcher

Scope: extend `resolve_simple_index_pair` (`places.rs:725`) to constant
indices with proven bounds, `iv ± constant`, and field-projected receivers
proven unreassigned; every widening preserves non-negativity independently
(the unchecked path skips normalization, `places.rs:694`). Evaluate
`optimizer/bounds.rs`'s affine-guard analysis as the engine (see
PERF-DEAD-OPT).
Blocked by: PERF-SUITE.
Tripwires: (1) a deliberately out-of-range constant index still traps via
the checked path (negative control per widened shape); (2) elision plans are
asserted per fixture (which accesses elide), not inferred from timing;
(3) bspline-class kernel time reported before/after.

### PERF-DEAD-OPT — wire-or-delete the dead optimizer module

Scope: one disposition per component of `shape-jit/src/optimizer/` — the
FULL thirteen-component inventory (~5,350 lines): bounds, licm,
vectorization, loop_lowering, escape_analysis, call_path, correctness,
cross_function, hof_inline, numeric_arrays, table_queryable, typed_mir,
and the shared plan/cache types — wired behind a measured ticket or
deleted with tests. No third state; a highlight-subset disposition leaves
the remainder in exactly the forbidden reserve. `escape_analysis.rs`'s
disposition precedes PERF-ESCAPE (ADR-018 §4 prior-art rule).
Blocked by: PERF-BCE-WIDEN (for the bounds decision); PERF-SUITE.
Tripwires: (1) after close, a workspace grep finds zero uncalled optimizer
entry points; (2) each deletion names what evidence a future re-derivation
must produce (so deletion is not silent capability loss); (3) the
disposition table enumerates all thirteen with none marked "later".

### PERF-ESCAPE — value-escape analysis

Scope: two solver products — (a) outbound value-escape: "allocation dies
with frame/region", distinct from reference-escape promotions
(`solver.rs:1453`) and from the sink-blind `detect_escape_status` heuristic
that the promotion rule at `storage_planning.rs:996` explicitly refuses to
consume; (b) inbound-reference analysis: no outside cycle-capable value is
stored into region-confined objects (ADR-018 §4 prerequisite 2 — a distinct
obligation, not a corollary).
Consumers: RC elision breadth, stack promotion, later arenas.
Blocked by: PERF-SUITE.
Tripwires: (1) known-escaping fixtures (return, capture, module store,
container insert, task spawn) are each negative controls; (2) the B0004
container-referent false-positive class that disqualified the heuristic is
an explicit test; (3) inbound negative controls: outside-object-stored-into-
region fixtures must fail the exemption proof.

### PERF-ALLOC-SEAM — one allocation seam

Scope: single allocator API for typed heap carriers replacing the twelve
files making direct `std::alloc` calls; collapse `TypedArray`'s
header+buffer double allocation (`typed_array.rs:63`); system-allocator
semantics initially (pure refactor).
Blocked by: none (independent).
Tripwires: (1) allocation-count and layout-identity differential vs main
across the v2 carrier test suites; (2) Miri provenance tests stay green;
(3) `alloc_budget` behavior preserved and now enforced at the seam.

### PERF-ARENA — region arenas

Scope: per-region bump allocation for proven region-confined values with
bulk retirement, finalization order untouched (ADR-010 §4), GC co-design
per ADR-018 §4: candidate-buffer exemption only under the combined
outbound + inbound proof, and a NEW range-based invalidation primitive for
candidate buffer + side table so retirement is O(1) plus finalization (the
per-address `gc_note_object_freed` contract exists to defeat address-reuse
aliasing, and a reused bump range is exactly that case).
Blocked by: PERF-ESCAPE (both products), PERF-ALLOC-SEAM, and the ADR-010
region-plan pipeline (currently zero implementation — this ticket does not
start it).
Tripwires: (1) finalization-order differential vs non-arena execution is
byte-identical on Drop-observing fixtures; (2) GC stress with cross-region
references detects zero stale side-table entries after range invalidation,
including address-reuse (ABA) fixtures that re-allocate inside a retired
range; (3) allocation-heavy suite improvement reported with the charter
harness.

## Sequencing

PERF-SUITE first and alone. Then DEOPT-GRANULARITY (largest single lever),
CLOSURE-NATIVE → HOF-CARRIER, and the independent pair ESCAPE + ALLOC-SEAM
in parallel with RC-ELISION and BCE-WIDEN. ARENA is last and remains a
sequenced bet — its prerequisites pay for themselves regardless.
