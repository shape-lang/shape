# Cluster-2 inventory — comprehensive cluster-2-remaining-territory audit

Phase 3 cluster-2 single-audit-day deliverable under the bulldozer
cadence (β2 supervisor disposition 2026-05-16). Branch
`bulldozer-strictly-typed-cluster-2-inventory-audit`, parent `5d09007e`
(post-cluster-2-closure-wave-1 close: V3-S6f Smoke 2 JIT TIMEOUT
RESOLVED; smoke matrix 4/4 VM == JIT at canonical fixture). Date:
2026-05-16.

**Audit-only deliverable.** Zero source changes inside this dispatch.
The deliverable is forward-visibility for cluster-2 closure-wave
dispatch (multi-agent parallel) per supervisor's authorization at
closure-wave dispatch time.

The dispatch's discipline (per `phase-2d-handover.md` §0 +
`bulldozer-wave-1-inventory.md` precedent): **comprehensive
ground-truth coverage in a single dispatch, no per-target sub-clusters,
no speculative "needs another audit" disposition.** Every claim in this
document is grep-verified against source at HEAD `5d09007e` — the
44-imprecision-pattern signal across phase-3 trajectory is the binding
to verify every ground-truthable claim before surfacing (Q3 recursive
pre-flight binding 2026-05-16).

The deletion / migration targets covered (sections A through I):

| § | Target | Status |
|---|---|---|
| §A | V3-S6f closure-wave-1 close + closure-wave-2 (hypothesis b) dispatch shape | inline-pasted + recommendation |
| §B | Broader W11-jit-new-array general fix per user-function-class coverage matrix | mapped + per-class disposition |
| §C | hashmap-value-v-arm follow-up (v2_group_by + Array.groupBy) | mapped + designed |
| §D | shape-test-residuals-audit (10 failure classes) | per-class size estimate (no per-test cite) |
| §E | per-HeapKind kinded jit_print coverage matrix | mapped + designed |
| §F | compile-time-boxed string-constant leak | mapped + designed |
| §G | W12-collection-constructor-mir-lowering coverage | mapped (already landed; gaps surfaced) |
| §H | Cluster-2 closure-wave partition recommendation + tracing-crate-migration | proposed |
| §I | Q25.C TraitObject rebuild absorb-vs-separate (cluster-1.5 vs cluster-2) | mapped + factors + recommendation (supervisor disposes) |

---

## §0 — Status + structural framing

**Audit-only close.** Zero source changes. Baseline gates run on
close: `cargo check --workspace --lib --tests` exit 0 +
`bash scripts/verify-merge.sh` 12/12 PASS + `bash
scripts/check-no-dynamic.sh` exit 0 + smoke matrix 4/4 VM == JIT at
canonical fixture (IDENTICAL to canonical `5d09007e` baseline; audit
doesn't change runtime behavior).

**Architectural framing rule** (per `phase-2d-handover.md` §0 #10):
"preserve X for cluster-1+" / "needs its own audit sub-cluster" /
"multi-week scope" / "defer to cluster-1.5 post-close" are refused on
sight under bulldozer cadence. Every target in §A-I either has a
designed closure-wave territory or surfaces a specific structural
reason for genuine in-wave intractability (§I is the only target that
meets this bar — Q25.C TraitObject rebuild absorb-vs-separate is
supervisor's authorization at closure-wave dispatch time per R4
disposition; audit maps factors + recommends, does NOT decide).

---

## §A — V3-S6f closure-wave dispatch shape

Per R1 / R2 supervisor disposition 2026-05-16: section A consumes
cluster-2-empirical-verification disposition + cluster-2-closure-wave-1
close. Inline-pasted verbatim from `docs/cluster-audits/phase-3-
cluster-0-status.md` §"Wave 3 Round 5 cluster-2-empirical-verification
close" + §"Wave 3 Round 6 cluster-2-closure-wave-1 close" per Q3
inline-paste binding 2026-05-16.

### §A.1 Per-hypothesis disposition (from cluster-2-empirical-verification close, verbatim)

| Hypothesis | Disposition | Architectural-source locus |
|---|---|---|
| (a) For-loop iterator state-machine for v2 typed-array source | **CONFIRMED dominant** | `crates/shape-vm/src/mir/lowering/expr.rs:1035-1094` `lower_for_expr` generic-iterator branch emits placeholder stub (no IterNext/IterDone advance, unconditional Goto(header), pattern=MirConstant::None). Literal `// This is a placeholder` comment unchanged at HEAD bb5b2109. Empirically verified via existing `SHAPE_JIT_MIR_TRACE` capturing broken MIR of specialized `Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9` body bb2/bb6 infinite-loop cycle. |
| (b) Closure-call indirect-call inside inlined specialization | **PARTIAL latent** | Phase-C `inline_closure_body_into_specialization` at `crates/shape-vm/src/compiler/monomorphization/substitution.rs:2247` IS invoked + returns Ok per `[mono-phaseC]` trace, but post-MIR bb3 still shows un-inlined `Call(Copy(f), [item])` rather than expected inlined `BinaryOp(Mul, Copy(slot_x), Constant(Int(2)))`. 4 explanations enumerated in deliverable §3.4; severity moot until (a) fixes the structural infinite loop (bb3 executes once-then-loops-forever). |
| (c) Receiver-self slot kind threading from V3-S6c routing | **REFUTED** | V3-S6c routing at `crates/shape-jit/src/mir_compiler/terminators.rs:176` + V3-S6e stamping at `crates/shape-vm/src/compiler/helpers.rs:494` (consumer at line 623) remain correct. bb2 `SwitchBool(Copy(SlotId(6)))` reads raw pointer bits as u64 truthiness, independent of NativeKind stamp. bb6 `Goto(bb2)` is unconditional. JIT successfully reaches the specialized fn body (`[jit-debug] compilation OK, about to execute...` fires, no SIGSEGV / surface-and-stop). |

### §A.2 Closure-wave-1 close summary (from status doc §"Wave 3 Round 6 cluster-2-closure-wave-1 close", verbatim)

Hypothesis (a) fix landed per cluster-2-empirical-verification
disposition. MIR-lowering generic-iterator branch placeholder stub
replaced with per-iterable-ConcreteType monomorphic state machine via
existing MIR vocabulary. V3-S6f Smoke 2 JIT TIMEOUT rc=124 RESOLVED at
canonical post-merge HEAD `cc5ceb0e`.

#### Sub-agent close (6e9ffbc5; merged at cc5ceb0e)

- Branch: `bulldozer-strictly-typed-cluster-2-closure-wave-1-iter-statemachine` (from 6a485051)
- Close commit: `6e9ffbc5`; merge commit `cc5ceb0e` on canonical (no-ff merge)
- Diff: +306 / -55 across 4 files: `crates/shape-vm/src/mir/lowering/expr.rs` +211/-55 (lower_for_expr fix) + `crates/shape-vm/src/mir/lowering/stmt.rs` +109/-0 (lower_for_loop ForIn arm parallel fix) + `crates/shape-vm/src/mir/lowering/mod.rs` +40/-0 (param-slot ConcreteType seeding) + AGENTS.md +1
- Well within 400-600 LoC estimate from empirical-verification §6 recommendation

#### Migration shape

Uniform index-counter state machine emitted across ALL non-Range iterables using existing MIR vocabulary (Call/Place::Index/BinaryOp::Add/BinaryOp::Lt/SwitchBool; NO new StatementKind variants):

```
bb_pre:  iter_slot = <iterable>; __idx = 0
         __len = iter_slot.len()                ; Call terminator
bb_hdr:  __cond = __idx < __len                 ; BinaryOp::Lt
         switchbool __cond -> bb_body, bb_after
bb_body: __elem = iter_slot[__idx]              ; Place::Index
         <destructure pattern>; <body>
         __idx = __idx + 1                      ; BinaryOp::Add
         goto bb_hdr
```

Per-iterable monomorphization happens downstream via JIT's existing v2
fast paths: `v2_array_len` + `v2_array_get(arr_ptr, idx, elem_kind)`
inline for typed-array carriers (Array<T> / Vec<T>) when
`v2_typed_array_elem_kind(receiver_place).is_some()`;
`jit_call_method("len")` + `inline_array_get(base, idx)` generic
fallback for HashMap/HashSet/Deque/PriorityQueue/Channel via existing
receiver_kind delegation matrix at
`crates/shape-jit/src/ffi/call_method/mod.rs:560-628`. All iterable
cases covered (no SURFACE).

#### Smoke matrix at canonical cc5ceb0e (4/4 VM == JIT)

- Smoke 1 (scalar loop): VM 4950 / JIT 4950 ✅
- **Smoke 2 (`xs.map(|x|x*2).sum()`): VM 30 / JIT 30 ✅** (was rc=124 TIMEOUT pre-fix)
- Smoke 3 (canonical `let t = X{}` UFCS): VM x / JIT x ✅
- Smoke 4 (`Set()` + `.add()` + `.size()`): VM 2 / JIT 2 ✅

#### Hypothesis (b) latency disposition (supervisor 2026-05-16, verbatim)

(b) confirmed-latent + NON-GATING via SHAPE_JIT_MIR_TRACE: Phase-C
`inline_closure_body_into_specialization` at
`crates/shape-vm/src/compiler/monomorphization/substitution.rs:2247`
still returns Ok but post-MIR bb4 shows un-inlined `Call(Copy(SlotId(2)),
[item])`. Indirect-call runtime infrastructure works correctly (Smoke 2
JIT returns 30; correctness preserved). Folded into
cluster-2-inventory-audit-day section A scope per supervisor
disposition — audit recommends whether closure-wave-2 dispatches
immediately within cluster-2 / defers / folds elsewhere.

### §A.3 Closure-wave-2 (hypothesis b) dispatch recommendation

**Recommendation: defer-within-cluster-2 (sequential after first-phase
empirical re-verification).**

**Reasoning, cite-anchored:**

1. Correctness preserved (Smoke 2 JIT returns 30); (b) is a
   performance gap not a correctness gap. The indirect-call runtime
   infrastructure works correctly per the closure-wave-1 close
   subsection ("Smoke 2 JIT returns 30; correctness preserved").
2. Per the empirical-verification §3.4, the (b) gap has 4 distinct
   possible explanations; the dispatch's required-reading entry §3.4
   bound that "each requires its own empirical pass to disposition
   definitively." First-phase empirical re-verification is required
   before any fix — pre-planning a fix without disposing among the 4
   would risk an architectural-prediction-subclass instance.
3. Per the empirical-verification §6 closure-wave-recommendation, "the
   two hypothesis sub-clusters are sequentially dependent (b cannot be
   empirically dispositioned until a is fixed — the bb3 Call terminator
   only executes once-then-hangs at HEAD, so Phase-C inlining failure
   modes are not observable at runtime)." Post-closure-wave-1, the bb3
   Call now executes per-iteration — (b) becomes empirically
   dispositionable for the first time. Pre-fix planning at this dispatch
   would be premature.
4. Per the cluster-2-closure-wave-1 close subsection "Reading 4
   candidate (2026-05-16)" — the closure-wave-1's clean in-scope
   recovery of imprecision 31 validates the
   architectural-prediction-subclass-recovery pattern: when a
   hypothesis-fix sub-agent surfaces a consumer-side
   kind-classification gap mid-execution, in-scope fix is preferable to
   surface-and-stop-then-dispatch-separately. The same pattern applies
   to closure-wave-2 (b) — first-phase empirical re-verification may
   resolve (b) entirely (e.g. if a hypothesis-(a)-style downstream
   fix-through-existing-infrastructure already covers Phase-C inlining
   in the post-(a) MIR shape).

**Scope estimate:**

- First-phase empirical re-verification: ~100-300 LoC at the empirical
  layer (replicate empirical-verification §3 trace points + add 1-2
  new traces at the post-(a) bb3 Call shape; verify which of 4
  explanations holds OR confirm (b) is resolved by (a)'s side-effects).
  Single agent. Dispatched immediately as cluster-2 closure-wave-2.
- Second-phase fix (if needed): scope unknown until empirical phase
  completes. Bounded to the inliner + its callers per
  empirical-verification §3.5 "Territory if hypothesis (b) closure-
  wave is dispatched."

**ADR-fit (per empirical-verification §3.5):** ADR-006 §2.7.10 / Q11
(MethodFnV2 ABI) + §2.7.11 / Q12 (value-call ABI). Phase-C inlining is
a compile-time optimization that fits under §2.7.5 stamp-at-compile-
time discipline — it eliminates a runtime indirect call by emitting
the closure body inline. Failure to inline correctly is a §2.7.5
conduit gap at the AST→MIR boundary.

**Cascade-site estimate:** 0 (bounded to the inliner + its callers).

---

## §B — Broader W11-jit-new-array general fix per user-function-class coverage matrix

Per R3 supervisor disposition 2026-05-16: enumerate the JIT codegen
layers that lack §2.7.5 stamp-at-compile-time coverage for hypothesis
(a) confirmed gap class. Derive coverage matrix from the
closure-wave-1 in-scope-recovery fix at
`crates/shape-vm/src/mir/lowering/mod.rs::lower_function_detailed`
(imprecision 31 — param-slot ConcreteType seeding).

### §B.1 The closure-wave-1 in-scope-recovery fix

`crates/shape-vm/src/mir/lowering/mod.rs:643-680` (verbatim at HEAD
5d09007e) seeds `local_typed_array_element_types` from typed-array
param annotations (`self: Vec<C>` / `xs: Array<C>` / etc.):

```rust
// cluster-2-closure-wave-1-iter-statemachine (2026-05-16):
// seed `local_typed_array_element_types` from a typed-array
// param annotation (`self: Vec<C>` / `xs: Array<C>` / etc.).
// ...
if let Some(annotation) = param.type_annotation.as_ref() {
    if let Some(shape_value::v2::ConcreteType::Array(elem)) =
        crate::compiler::v2_map_emission::concrete_type_from_annotation(annotation)
    {
        builder.record_local_typed_array_element_type(slot, *elem);
    }
}
```

The conduit consumer at `crates/shape-vm/src/compiler/helpers.rs:623-634`
(verbatim) reads `mir.local_typed_array_element_types` and stamps
`concrete_types[idx] = Array(elem)` for params that have typed-array
annotations.

### §B.2 User-function-class coverage matrix

Every user function class compiles through `compile_function` → which
calls `lower_function_detailed` at the SAME entry point. The lower-
function-detailed param-seeding pass therefore fires uniformly across
all function classes; the discriminator is whether the function has a
typed-array param annotation, not the function class.

Grep-verified call sites of `lower_function_detailed` at HEAD
5d09007e:

| Caller | File:line | Function-class coverage |
|---|---|---|
| `compile_function` (production path for every user-defined function) | `crates/shape-vm/src/compiler/functions.rs:259` | Top-level + impl methods + extend blocks + trait-impl + closure bodies + monomorphized specializations |
| `compile_items` (compilation unit / module body) | `crates/shape-vm/src/compiler/functions.rs:1034` | Module-level synthetic body |
| `return_ownership.rs::reanalyze_with_constraints` (R5 borrow-check reanalysis) | `crates/shape-vm/src/mir/return_ownership.rs:1255` | Same function (reanalysis pass; not a separate function class) |
| `helpers_reference.rs:599` (reference-model auxiliary) | `crates/shape-vm/src/compiler/helpers_reference.rs:599` | Auxiliary; not user-function class |
| Production callers of `lower_function` (thin wrapper at mod.rs:718-724) | `crates/shape-vm/src/mir/lowering/mod.rs:724` + caller at `mod.rs:786` (`compile_function_definition`) | Backward-compat path for older callers; threads through lower_function_detailed eventually |

| Function class | Reaches `lower_function_detailed`? | Param-seeding active? | Gap status |
|---|---|---|---|
| Top-level fn (`fn name(xs: Array<int>) { ... }`) | YES (`functions.rs:259`) | YES | COVERED (closure-wave-1) |
| Generic-monomorphized fn (`Vec.map<T,U>`) post-substitution specialized def | YES (specialized_def goes through `compile_function` per `monomorphization/cache.rs:421` + `:603`) | YES (typed-array param annotation is substituted to `self: Vec<I64>` / etc.) | COVERED (closure-wave-1, load-bearing for Smoke 2 fix) |
| Trait-impl method (`impl T for X { fn method(self) { ... } }`) | YES (same `compile_function` path; impl methods are compiled as functions) | YES (same param annotation walk) | COVERED (closure-wave-1; no separate impl-method-class path) |
| Closure body (`|x: Array<int>| { for v in x { ... } }`) | YES (closures are compiled as functions via `compile_function`) | YES, conditionally — closure params have `type_annotation` field on `FunctionParameter`; the param-walk reads `annotation` per `mod.rs:674`. If closures lack typed-array annotations (common — inferred-typed closures), no seeding fires | PARTIAL — covered when closure has explicit typed-array param annotation; UNCOVERED when closure param kind is inferred (no annotation source for `concrete_type_from_annotation` to resolve) |
| Extend-block method (`extend Vec<T> { fn method(self) { ... } }`) | YES (extend methods are compiled as functions via `compile_function`) | YES (same param annotation walk; extend-method type_params merged per V3-S6a `desugar_extend_method`) | COVERED (closure-wave-1) |
| Comptime-emitted fn (any comptime-generated function) | YES (synthesized AST goes through same `compile_function` path) | YES if the synthesized function has typed-array param annotations | COVERED (closure-wave-1) — synthesized AST is structurally identical to user-written AST |

### §B.3 Per-class migration design

**Class A — Top-level / generic-monomorphized / trait-impl / extend-block / comptime-emitted fn with typed-array param annotation:**

- **Status:** COVERED at HEAD 5d09007e via closure-wave-1
  (`crates/shape-vm/src/mir/lowering/mod.rs:643-680` param-seeding
  pass + conduit consumer at `crates/shape-vm/src/compiler/helpers.rs:623-634`).
- **Migration design:** N/A — covered.
- **Cascade-site count:** 0 (covered).
- **ADR-fit:** §2.7.5 stamp-at-compile-time (param annotation IS the
  proof of receiver ConcreteType at compile time).

**Class B — Closure body with INFERRED typed-array param (no annotation):**

- **Status:** UNCOVERED at HEAD 5d09007e. The param-seeding pass at
  `mod.rs:674` reads `param.type_annotation.as_ref()`; closures with
  inferred param types pass `None` here, so no seeding fires.
- **Migration design hypothesis (NOT pre-judged per R3 binding):**
  closure type inference resolves at a layer above MIR lowering; the
  inferred type information IS available somewhere in the
  compiler-side bidirectional-closure-inference state machine
  (`crates/shape-vm/src/compiler/closures.rs` and
  `crates/shape-runtime/src/type_system/inference/` per the CLAUDE.md
  "Bidirectional closure inference" entry). Extending the param-
  seeding pass to consult the inferred-kind side-channel (whatever
  shape it takes — `BindingStorageClass`, a closure-param inferred-
  type map, or similar) would close the gap for inferred-typed
  closures. **EMPIRICAL VERIFICATION REQUIRED** before designing —
  pre-judgment refused per R3 binding ("do NOT write 'the migration
  is X' without empirical-verification or grep-verified ground truth").
- **Cascade-site estimate:** unknown; depends on which inference-state
  surface the param-seeding pass consults. Bounded to 1-3 files in
  `crates/shape-vm/src/mir/lowering/` + the inference-state surface
  consulted.
- **ADR-fit:** §2.7.5 stamp-at-compile-time (inferred kind, if
  available, IS the proof; the migration is propagating the inference
  result from the closure-inference layer to the MIR-lowering layer).

**Class C — Function whose typed-array body slots are NOT param-derived (intermediate slots from method-chain composition, comprehension results, etc.):**

- **Status:** UNCOVERED — these slots are NOT param slots; the
  `mod.rs:674` param-walk does not touch them. The empty-array-literal
  pass at `helpers.rs:623-634` (V3-S6e pass) covers `let mut result = []`
  + `Array<C>` annotation; chained `let doubled = xs.map(|x|x*2);
  doubled.sum()` style intermediate slots get their ConcreteType from
  the resolver pass (`infer_top_level_concrete_types_from_mir_with_
  resolvers`'s downstream walk after the producer-side stamping).
- **Migration design hypothesis (NOT pre-judged per R3 binding):**
  audit / empirical verification required to determine which
  intermediate-slot shapes are still un-classified post-closure-wave-1.
  V3-S6e + V3-S5 architectural sunset together cover the principal
  shapes (empty-literal + monomorphized-specialization + bytecode-side
  `concrete_types` for top-level), but there may be remaining gaps at
  the chain-step boundary (e.g. `xs.map(...).filter(...)`'s
  intermediate `_map_result` slot). **EMPIRICAL VERIFICATION REQUIRED**
  before designing.
- **Cascade-site estimate:** unknown.
- **ADR-fit:** §2.7.5 stamp-at-compile-time (the chain composition's
  intermediate result IS a compile-time-derivable kind; missing
  classification is a §2.7.5 conduit gap).

### §B.4 Recommended closure-wave shape

**Closure-wave (B-closure-coverage):** single sub-cluster, single
agent. Empirical-verification-first dispatch shape (mirror of §A.3
closure-wave-2):

1. Phase 1 — empirical verification: enumerate the closure-without-
   typed-array-annotation cases reachable via the JIT smoke matrix +
   stdlib (`vec.shape` / `hashmap.shape` / etc.) and document which
   produce un-stamped intermediate slots OR receiver-kind=UInt64
   §2.7.5 carrier fallback at JIT execution. SHAPE_JIT_DEBUG-gated
   trace points matching the existing 28-site pattern (see §H below
   for the tracing-crate migration candidate that this dispatch
   leverages).
2. Phase 2 — fix design: per the empirical-verification disposition,
   either extend the param-seeding pass to consult the
   closure-inference side-channel (Class B), extend the resolver pass
   to cover the chain-step intermediate slots (Class C), or both.

**Territory:** `crates/shape-vm/src/mir/lowering/mod.rs:607-715`
(lower_function_detailed) + `crates/shape-vm/src/compiler/helpers.rs`
(infer_top_level_concrete_types_from_mir_with_resolvers' downstream
walk) + the closure-inference side-channel surface (TBD; depends on
empirical-phase finding).

**Cascade-site estimate (empirical-phase):** 0 source change beyond
SHAPE_JIT_DEBUG-gated traces.

**Cascade-site estimate (fix-phase):** unknown; bounded to 1-3 files in
`crates/shape-vm/src/mir/lowering/` + `crates/shape-vm/src/compiler/`.

**ADR-fit:** §2.7.5 stamp-at-compile-time + §2.7.5.1 ("Compile-time
analysis state where a slot's kind is 'not yet known' during inference
is held LOCALLY in the analysis tracker"; the migration propagates
those locally-held inference results into the conduit consumer).

---

## §C — hashmap-value-v-arm follow-up (v2_group_by + Array.groupBy)

Per Round 3b C2-joint left as Wave 3 R1 → cluster-2 fold +
wave-3-baseline-classification §"v2_group_by + Array.groupBy tests".

### §C.1 Surface-and-stop sites at HEAD 5d09007e (grep-verified)

#### `v2_group_by` (HashMap.groupBy receiver)

`crates/shape-vm/src/executor/objects/hashmap_methods.rs:1735-1770`
(verbatim at HEAD 5d09007e):

```rust
pub fn v2_group_by(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // ... arg validation ...
    Err(VMError::NotImplemented(
        "HashMap.groupBy(): outer HashMap<string, HashMap> carrier requires \
         a HashMap-value V arm in HashMapKindedRef, which is not landed and \
         would expand cluster-0+1 scope. Surface-and-stop per playbook §6 \
         (no degraded HashMap-as-TypedObject wrapper). Tracked as \
         hashmap-value-v-arm in the follow-up cluster.".into(),
    ))
}
```

Method registered at `crates/shape-vm/src/executor/objects/method_registry.rs:452`
under PHF entry `"groupBy" => v2_group_by`.

#### `Array.groupBy` (handle_group_by_v2)

`crates/shape-vm/src/executor/objects/array_transform.rs:363-378`
surface-and-stop via `ckpt2_surface` helper at
`crates/shape-vm/src/executor/objects/array_transform.rs:215-242`
(V3-S5 ckpt-2 consumer-cascade tier 1 surface — `TypedArrayData` enum
DELETED). Method registered at
`crates/shape-vm/src/executor/objects/method_registry.rs:270` under
PHF entry `"groupBy" => handle_group_by_v2`.

#### DataTable.group_by (handle_group_by)

`crates/shape-vm/src/executor/objects/datatable_methods/query.rs:380-391`
(separate SURFACE — §2.7.4 cross-cluster cascade per the docstring;
NOT a HashMap V-arm gap but a DataTable group_by gap; mentioned for
completeness of the groupBy-method-family surface).

### §C.2 Producer ground-truth — `HashMapKindedRef` enum at HEAD 5d09007e

`crates/shape-value/src/heap_value.rs:1658-1680` (verbatim):

```rust
pub enum HashMapKindedRef {
    I64(Arc<HashMapData<i64>>),
    F64(Arc<HashMapData<f64>>),
    Bool(Arc<HashMapData<u8>>),
    Char(Arc<HashMapData<char>>),
    String(Arc<HashMapData<*const crate::v2::string_obj::StringObj>>),
    Decimal(Arc<HashMapData<*const crate::v2::decimal_obj::DecimalObj>>),
    TypedObject(Arc<HashMapData<TypedObjectPtr>>),
    TraitObject(Arc<HashMapData<TraitObjectPtr>>),
}
```

**HashMap-value V arm is NOT in the enum.** The
`v2_group_by` SURFACE message identifies this directly:
"`HashMap<string, HashMap>` carrier requires a HashMap-value V arm in
HashMapKindedRef, which is not landed and would expand cluster-0+1
scope."

Producer-side count for `HashMapValueBuf` (the SUPERSEDED Q25.B
inner-enum carrier): 3 references in `crates/shape-value/src/heap_value.rs`
+ 0 in any other crate (grep `HashMapValueBuf::` returned 3 hits, all
in the heap_value.rs comment/docstring zone — Q25.B SUPERSEDED close
landed full deletion of the enum's live arms).

### §C.3 Migration design

**Per Q25.B SUPERSEDED §2.7.24 amendment (preserved):** the
`HashMapKindedRef` enum is the post-supersession canonical carrier;
per-V monomorphization at the method tier. Adding a HashMap-value V
arm requires:

1. **Add `HashMap(Arc<HashMapData<HashMapKindedRef>>)` arm** to
   `HashMapKindedRef` at `crates/shape-value/src/heap_value.rs:1658-1680`.
   The inner V type is `HashMapKindedRef` itself (recursive carrier);
   the inner Arc holds a `HashMapData<HashMapKindedRef>` (the inner
   HashMaps' values buffer is a flat array of `HashMapKindedRef`
   pointers).
2. **Update `Clone` impl** at `crates/shape-value/src/heap_value.rs:1682-1697`
   with HashMap arm (`HashMapKindedRef::HashMap(arc) =>
   HashMapKindedRef::HashMap(Arc::clone(arc))`).
3. **Update `values_kind` accessor** at
   `crates/shape-value/src/heap_value.rs:1740-1750` with HashMap arm
   (`HashMapKindedRef::HashMap(_) => NativeKind::Ptr(HeapKind::HashMap)`).
4. **`HashMapValueElem` trait impl** for `HashMapKindedRef`. This
   IS the load-bearing structural change — the existing trait dispatch
   per inner element type needs an arm that retain/releases the inner
   `HashMapKindedRef`'s own per-variant Arc.
5. **`v2_group_by` body** in
   `crates/shape-vm/src/executor/objects/hashmap_methods.rs:1735-1770` —
   replace SURFACE with: iterate entries, call closure, accumulate
   per-group-key into per-bucket `HashMapKindedRef`s, wrap into outer
   `HashMapData<HashMapKindedRef>` + outer `HashMapKindedRef::HashMap`.
6. **`handle_group_by_v2` body** in
   `crates/shape-vm/src/executor/objects/array_transform.rs:363-378` —
   the receiver-side is post-V3-S5 ckpt-2 (`TypedArrayData` deleted) so
   the receiver migrates to v2-raw `TypedArray<T>` per the §A.3
   migration shape (sibling of `handle_map_v2` / `handle_filter_v2` /
   etc. which are all surfaced via `ckpt2_surface`). Wrapping
   Array.groupBy's V arm into `HashMapKindedRef::HashMap` follows the
   §C.3-step-1 carrier.

**ADR-fit:** §2.7.24 Q25.B SUPERSEDED canonical pattern
(HashMapKindedRef carrier + per-V monomorphization at the method
tier) extends naturally to a recursive HashMap-value V arm via the
existing `HashMapValueElem` trait dispatch shape.

**Cascade-site count estimate:**

- HashMapKindedRef enum: 1 file modified (heap_value.rs)
- 4-table lockstep (§0 ordinal-collision rule, handover §0) — HashMap
  ordinal 17 already exists; the V-arm doesn't add a new HeapKind so
  the 4-table arms don't change. Only the `HashMapKindedRef` Clone /
  values_kind / HashMapValueElem impl need V-arm coverage. **0
  HeapKind ordinal changes.**
- `v2_group_by` body: 1 file modified (hashmap_methods.rs)
- `handle_group_by_v2` body: 1 file modified (array_transform.rs); but
  this depends on the broader V3-S5 ckpt-2 surface migration first
  (the receiver-side `Arc<TypedArrayData>` → v2-raw `TypedArray<T>`
  cascade is the ckpt-2 surface group described at §A.3 — 25
  `ckpt2_surface` call-sites across `array_transform.rs` (12),
  `array_aggregation.rs` (7), `array_sets.rs` (6)). This sub-cluster
  may need to gate Array.groupBy on V3-S5 ckpt-3+ landing.
- Total: 4 source files for the HashMap.groupBy fix (heap_value.rs +
  hashmap_methods.rs + the HashMapValueElem impl site + the inner
  HashMapData<V> per-V constructor site).
- Total (if V3-S5 ckpt-3+ has landed): +1 file for Array.groupBy
  (array_transform.rs handle_group_by_v2 body).

**ADR amendment owed at sub-cluster close:** ADR-006 §2.7.24 Q25.B
SUPERSEDED amendment (the per-V monomorphization table at
`docs/adr/006-value-and-memory-model.md:5098-5111`) extends with a
HashMap row:

```
| HashMap | NEW arm (Wave N hashmap-value-v-arm follow-up) | per-V
monomorphization in `HashMapData<V>` + recursive `HashMapKindedRef`
values pointer (V = HashMapKindedRef itself) |
```

---

## §D — shape-test-residuals-audit (10 failure classes; per-class size estimate)

Per Q4 supervisor disposition 2026-05-16: per-class file glob + class
root-cause hypothesis + agent-territory size estimate (small / medium
/ large per LoC + test-count). NO per-test file:line cite (closure-
wave agent's territory). Bounds D to ~10 entries (one per class) not
~50-100 per-test cites.

10 classes per `wave-3-baseline-classification.md` §"Cluster-2
territory recommendation: shape-test-residuals-audit scope":

### §D.1 Class enumeration

| # | Class | File glob | Class root-cause hypothesis | Size estimate | Per-class disposition (in-cluster-2-fixable / surface-and-stop / cluster-1.5 fold per Surface A) |
|---|---|---|---|---|---|
| 1 | v2-raw-heap aliasing (`malloc_consolidate` / `tcache` SIGABRT) | `tools/shape-test/tests/{annotations_comptime,annotations_runtime,arrays_vectors,hashmap,objects}/*.rs` + `tools/shape-test/tests/annotation_targets/*.rs` (stochastic) + `bin/shape-cli/tests/stdlib/simulation.rs` (4 already-`#[ignore]`'d) | `typed_array_push_*` realloc invalidates aliased raw pointers held across iterations → VM Drop double-free. Same bug class as the 4 simulation tests `#[ignore]`'d at `bin/shape-cli/tests/stdlib/simulation.rs` per CLAUDE.md "Known Constraints" v2-raw-heap-audit | LARGE (~5 suites, multi-hundred-LoC fix surface — `c-stdlib-msgpack` `vw_clone`/`vw_drop` precedent at commit `afb1651` is the reference; the audit work is enumerating every aliased-raw-ptr-across-iteration site in stdlib + identifying the carrier-shape fix per site) | in-cluster-2-fixable (carrier-shape-fix per site; SHAPE_JIT_DEBUG-gated empirical tracing of the SIGABRT trigger; tracked as `v2-raw-heap-audit` in CLAUDE.md) |
| 2 | stdlib JIT-compilation cache hang (~2-5 min per test) | `tools/shape-test/tests/{closures_hof,enums,error_handling,functions,generics,iterators,literals,lsp,modules_visibility}/*.rs` | stdlib JIT-compilation caching (~118 stdlib functions per test); SIGILL race at default parallelism; CLAUDE.md "Known Constraints" shape-jit deep-tests gating; same root cause class | MEDIUM (~9 suites; cache implementation surface is bounded to `crates/shape-jit/src/` cache layer; likely 1-2 file fix once the caching strategy lands) | in-cluster-2-fixable (cache-layer fix at JIT level; landing JIT stdlib pre-compile cache eliminates the per-test recompile cost) |
| 3 | async/concurrency hooks-and-traces 9-failure cluster | `tools/shape-test/tests/async_concurrency/*.rs` (9 failures both HEADs per baseline) | async/concurrency primitives partial-coverage; hooks-and-traces test fixture infrastructure | MEDIUM (~1 suite, 9 failures; bounded class) | in-cluster-2-fixable (extend async/concurrency hooks coverage; bounded to async/concurrency primitive method bodies) |
| 4 | book_doctests 2-failure cluster | `tools/shape-test/tests/book_doctests.rs` (1 passed / 2 failed both HEADs) | book documentation example coverage gaps (e.g. stdlib examples expecting features not landed) | SMALL (~2 failing doctests in one file) | in-cluster-2-fixable (per-doctest investigation; either fix the example or fix the underlying language coverage; bounded scope) |
| 5 | annotations 9-and-15-failure cluster | `tools/shape-test/tests/annotation_targets/*.rs` (9 passed / 15 failed when not SIGABRT'd) | annotation system coverage gaps (e.g. `@before` / `@after` / target-validation edge cases) | MEDIUM (~15 failures in one suite; annotation context fixture bounded) | in-cluster-2-fixable (extend annotation system per failing test; bounded to `crates/shape-runtime/src/annotation_context.rs` + annotation lowering layer) |
| 6 | borrow_refs 50-failure cluster | `tools/shape-test/tests/borrow_refs/*.rs` (155 passed / 50 failed) | borrow checker / ref semantics coverage gaps; mix of MIR-borrow-solver + ref-escape-analysis surface | LARGE (~50 failures across 7 sub-files; MIR solver / liveness / storage_planning fix surface) | in-cluster-2-fixable (MIR solver / ref-semantics extensions; bounded scope per failing assertion class) |
| 7 | control_flow 25-failure cluster | `tools/shape-test/tests/control_flow/*.rs` (455 passed / 25 failed) | control flow coverage gaps (likely loops/match/break-continue residuals) | MEDIUM (~25 failures across 12 sub-files; per-failure triage) | in-cluster-2-fixable (per-failure-class triage; bounded scope) |
| 8 | jit / list_comprehension / e2e / e2e_gated / extend_blocks / features / functions / generics / hashmap failure clusters | `tools/shape-test/tests/{jit,list_comprehension,e2e,e2e_gated,extend_blocks,features,functions,generics,hashmap}/*.rs` | mixed: jit codegen + list-comprehension lowering + extend-block dispatch + extension features + generic inference + hashmap method body coverage | LARGE (~9 suites; per-failure-class triage; some classes intersect with V3-S5 ckpt-3+ cascade per §C.3 Array.groupBy ckpt2_surface) | mostly in-cluster-2-fixable per V3-S5 ckpt-3+ landing; some failures gate on V3-S5 ckpt-6 (JIT FFI) per `ckpt2_surface` message; surface-and-stop the ckpt2_surface-blocked failures with a §-cite per the surface-and-stop discipline |
| 9 | objects / objects_arrays groupBy + first/last / destructuring / array_concatenation failure cluster (incl. `array_groupby` FAILED) | `tools/shape-test/tests/{objects,objects_arrays}/*.rs` | groupBy + array first/last + destructuring rest + array concatenation; intersects with §C hashmap-value-v-arm AND V3-S5 ckpt-3+ ckpt2_surface cascade | MEDIUM (~1-2 suites; per-failure-class triage; intersects with §C) | in-cluster-2-fixable jointly with §C (HashMap-value-v-arm follow-up unblocks the groupBy assertions) |
| 10 | v2_group_by / Array.groupBy upstream-SIGABRT-blocked tests | `tools/shape-test/tests/hashmap/stress_iteration.rs:626+` + `tools/shape-test/tests/arrays_vectors/stress_chained.rs:472+` | per §C hashmap-value-v-arm (the HashMap-value V arm gap blocks v2_group_by) | SMALL (~6 tests across 2 files; unblocked by §C closure-wave) | in-cluster-2-fixable jointly with §C |

### §D.2 Class-level closure-wave dispatch shape

Per Q4 binding (file glob + class root-cause + size estimate only),
the per-class triage is the closure-wave agent's territory. The
dispatch shape recommendation:

- Class 1 (v2-raw-heap aliasing): one dedicated agent; territory =
  carrier-shape audit across stdlib + per-site carrier-fix
  (`vw_clone`/`vw_drop` precedent). LARGE.
- Class 2 (stdlib JIT cache hang): one dedicated agent; territory =
  JIT cache implementation. MEDIUM.
- Class 3-10 (8 classes): per-class triage agents; some classes
  intersect (e.g. 9 and 10 jointly with §C hashmap-value-v-arm). Per
  the supervisor's partition recommendation at §H below.

**Cascade-site count estimate (per-class):** see §D.1 size column;
totals roughly: SMALL (~10 LoC) / MEDIUM (~100-300 LoC) / LARGE (~500-
1500 LoC).

**ADR-fit:** mostly §2.7.5 stamp-at-compile-time + §2.7.7 / §2.7.10 /
§2.7.11 per the specific class; no new ADR amendments owed at close.

---

## §E — per-HeapKind kinded jit_print coverage matrix

Per phase-3-team-lead-handover.md "Next workstream" cluster-2 entry +
ADR-006 §2.7.14 close subsection ("Remaining out-of-scope: ...
Per-HeapKind kinded print entries (`jit_print_str` /
`jit_print_typed_object` / ...) — the kind-blind `jit_print` fallback
still uses `format_value_word` (NaN-decode-via-tag-bits) for heap
arms").

### §E.1 Existing kinded jit_print sites at HEAD 5d09007e (grep-verified)

7 kinded entries at `crates/shape-jit/src/ffi/conversion.rs`:

| jit_print_<X> | File:line | NativeKind arm |
|---|---|---|
| `jit_print_i64` | `crates/shape-jit/src/ffi/conversion.rs:289` | `NativeKind::Int64` / `UInt64` / `IntSize` / `UIntSize` |
| `jit_print_f64` | `crates/shape-jit/src/ffi/conversion.rs:298` | `NativeKind::Float64` |
| `jit_print_bool` | `crates/shape-jit/src/ffi/conversion.rs:313` | `NativeKind::Bool` |
| `jit_print_str` | `crates/shape-jit/src/ffi/conversion.rs:443` | `NativeKind::String` |
| `jit_print_typed_object` | `crates/shape-jit/src/ffi/conversion.rs:462` | `NativeKind::Ptr(HeapKind::TypedObject)` (SURFACE per terminators.rs:620-661 — carrier-mismatch gap) |
| `jit_print_option` | `crates/shape-jit/src/ffi/conversion.rs:486` | `NativeKind::Ptr(HeapKind::Option)` |
| `jit_print_result` | `crates/shape-jit/src/ffi/conversion.rs:508` | `NativeKind::Ptr(HeapKind::Result)` |

Dispatch site (the routing block consuming these): `crates/shape-jit/src/mir_compiler/terminators.rs:450-751`.

### §E.2 Print-Call routing arms at HEAD 5d09007e (grep-verified)

`crates/shape-jit/src/mir_compiler/terminators.rs:459-751`:

| Arm | NativeKind | FFI entry | Status |
|---|---|---|---|
| `Some(Int64 / UInt64 / IntSize / UIntSize)` | scalar int | `print_i64` | COVERED |
| `Some(Float64)` | scalar float | `print_f64` | COVERED |
| `Some(Bool)` | scalar bool | `print_bool` | COVERED |
| `Some(Ptr(HeapKind::Option))` | Option | `print_option` | COVERED |
| `Some(Ptr(HeapKind::Result))` | Result | `print_result` | COVERED |
| `Some(NativeKind::String)` | String | `print_str` | COVERED |
| `Some(Ptr(HeapKind::TypedObject))` | TypedObject | SURFACE (terminators.rs:620-661) — JIT-side `box_typed_object` carrier-mismatch with `Arc<TypedObjectStorage>` the FFI expects; cluster-1 follow-up `W17-jit-typed-object-arc-storage-migration` per the SURFACE message | SURFACE-with-cite |
| `_` (everything else) | unstamped or unwired heap arm | SURFACE (terminators.rs:712-751) — extend producer-site classification OR wire kinded FFI body | SURFACE-with-cite |

### §E.3 Per-HeapKind coverage matrix (full enumeration vs `HeapKind` enum at `crates/shape-value/src/heap_variants.rs:61-291`)

Total HeapKind variants at HEAD 5d09007e: 35 ordinals assigned (0
String through 35 MatrixSlice), with ordinal 8 (TypedArray) vacated
per V3-S5 ckpt-1+ deletion. Ordinals 0-7, 9-35 = 35 live variants.

| Ordinal | HeapKind | kinded `jit_print_<X>` exists? | Notes |
|---|---|---|---|
| 0 | String | YES (`print_str`) | covered |
| 1 | TypedObject | NO (FFI body exists at `jit_print_typed_object` but routing arm at terminators.rs:620-661 SURFACE-and-stops on carrier mismatch — cluster-1 follow-up) | UNCOVERED-SURFACE |
| 2 | Closure | NO | UNCOVERED |
| 3 | Decimal | NO | UNCOVERED |
| 4 | BigInt | NO | UNCOVERED |
| 5 | DataTable | NO | UNCOVERED |
| 6 | Future | NO | UNCOVERED |
| 7 | TaskGroup | NO | UNCOVERED |
| 8 | TypedArray (VACATED) | NO (vacated ordinal; printing of `Array<T>` is via v2-raw `TypedArray<T>` carrier — separate carrier shape; covered by per-elem-kind dispatch shape) | VACATED |
| 9 | Temporal | NO | UNCOVERED |
| 10 | TableView | NO | UNCOVERED |
| 11 | Content | NO | UNCOVERED |
| 12 | Instant | NO | UNCOVERED |
| 13 | IoHandle | NO | UNCOVERED |
| 14 | NativeScalar | NO | UNCOVERED |
| 15 | NativeView | NO | UNCOVERED |
| 16 | Char | NO | UNCOVERED |
| 17 | HashMap | NO | UNCOVERED |
| 18 | FilterExpr | NO (pure-discriminator variant; FilterExpr-labeled slots have no HeapValue arm — print-via-format unusual) | UNCOVERED |
| 19 | Reference | NO (pure-discriminator variant; `RefTarget`-labeled bits) | UNCOVERED |
| 20 | SharedCell | NO (pure-discriminator variant; `SharedCell`-labeled bits) | UNCOVERED |
| 21 | HashSet | NO | UNCOVERED |
| 22 | Iterator | NO | UNCOVERED |
| 23 | Deque | NO | UNCOVERED |
| 24 | Channel | NO | UNCOVERED |
| 25 | PriorityQueue | NO | UNCOVERED |
| 26 | Range | NO | UNCOVERED |
| 27 | Result | YES (`print_result`) | covered |
| 28 | Option | YES (`print_option`) | covered |
| 29 | TraitObject | NO | UNCOVERED |
| 30 | Mutex | NO | UNCOVERED |
| 31 | Atomic | NO | UNCOVERED |
| 32 | Lazy | NO | UNCOVERED |
| 33 | ModuleFn | NO | UNCOVERED |
| 34 | Matrix | NO | UNCOVERED |
| 35 | MatrixSlice | NO | UNCOVERED |

**Coverage today:** 3 of 35 live HeapKind variants (~9%) have kinded
`jit_print` entries. Plus 4 scalar entries (Int / UInt / Float64 /
Bool) covering 7 NativeKind arms (Int64/UInt64/IntSize/UIntSize +
Float64 + Bool + String).

### §E.4 Migration design per missing arm

**Per missing HeapKind:** add one `jit_print_<heap_kind>` FFI body at
`crates/shape-jit/src/ffi/conversion.rs` (mirror of `jit_print_option` /
`jit_print_result` shape — `(ctx_ptr, value_bits)` signature delegating
to the canonical VM-side `ValueFormatter::format_kinded` for VM == JIT
identical output) + register at
`crates/shape-jit/src/ffi_symbols/object_symbols.rs:153-173` (sym
declaration block) + add the routing arm at
`crates/shape-jit/src/mir_compiler/terminators.rs:459-751` (after the
existing `Some(Ptr(HeapKind::X)) =>` arms).

**ADR-fit (§2.7.5 stamp-at-compile-time + §2.7.7 #4/#7 forbidden):**
the routing arm consumes the operand's `NativeKind` (stamped at
MIR-emit time via `operand_slot_kind`'s producing-site
classification); routes to the matching kinded FFI entry that reads
the typed `Arc<T>` payload directly. No NaN-box tag decode, no
`is_heap_kind` probe — kind IS the discriminator (§2.7.7 #4 / #7).
Pre-Round-8A "preserved baseline" rationalization (kind-blind
`jit_print` fallback through `format_value_word`) was retired in
Round 8A per CLAUDE.md "Forbidden rationalizations" #1.

**4-table lockstep check:** the per-HeapKind kinded jit_print
migration is OUTSIDE the 4-table HeapKind lockstep (clone_with_kind /
drop_with_kind / SharedCell::drop / TypedObjectStorage::drop — those
4 tables ARE in lockstep at HEAD 5d09007e for all 35 live HeapKind
variants per `bash scripts/verify-merge.sh` 12/12 PASS). Adding a new
jit_print arm does NOT cascade through the 4-table lockstep — kind
arrives at the FFI body via the operand bits + NativeKind label, not
via the 4-table dispatch.

**Cascade-site count per arm:** ~3 files per arm (conversion.rs FFI
body + object_symbols.rs registration + terminators.rs routing arm).
Per-arm scope: ~30-100 LoC. Total for 30 missing UNCOVERED arms:
~1000-3000 LoC. **EXCEEDS ~100-site ceiling** if landed all at once;
surface-and-stop discipline applies — partition into per-HeapKind-family
sub-clusters per §H below.

### §E.5 Per-HeapKind-family sub-cluster recommendation

Group missing HeapKinds by carrier-shape family for closure-wave
agent partition:

- **Scalar-family carriers:** Char (16). 1 arm. SMALL.
- **Concurrency-primitive family:** Mutex (30), Atomic (31), Lazy
  (32), Channel (24). 4 arms. MEDIUM. Per §2.7.25 amendment's
  printing convention.
- **Collection family:** HashMap (17), HashSet (21), Deque (23),
  PriorityQueue (25), Range (26), Iterator (22). 6 arms. MEDIUM.
- **Numeric/temporal family:** Decimal (3), BigInt (4), Temporal (9),
  Instant (12). 4 arms. MEDIUM.
- **DataTable/Content family:** DataTable (5), TableView (10), Content
  (11), IoHandle (13). 4 arms. MEDIUM.
- **Native-foreign family:** NativeScalar (14), NativeView (15). 2
  arms. SMALL.
- **Pure-discriminator family:** FilterExpr (18), Reference (19),
  SharedCell (20). 3 arms. SMALL — but unusual print-target shape
  (pure-discriminator variants have no `HeapValue` arm; printing
  requires per-variant ad-hoc string format from the typed-Arc
  payload directly).
- **Async family:** Future (6), TaskGroup (7). 2 arms. SMALL.
- **Matrix family:** Matrix (34), MatrixSlice (35). 2 arms. SMALL.
- **TraitObject + Closure + TypedObject family:** TraitObject (29),
  Closure (2), TypedObject (1 — already SURFACE per terminators.rs:620-
  661; cluster-1 W17-jit-typed-object-arc-storage-migration territory).
  3 arms. MEDIUM. Gated on TypedObject carrier migration per the
  existing SURFACE.
- **ModuleFn:** ModuleFn (33). 1 arm. SMALL.

Total: 30 UNCOVERED arms across 10 families. **Per-family sub-cluster
landing avoids the ~100-site ceiling exceed** and matches the
existing per-HeapKind ordinal-table assignment pattern.

---

## §F — compile-time-boxed string-constant leak

Per ADR-006 §2.7.14 close subsection ("Remaining out-of-scope: ...
Compile-time-boxed string constants (`box_string` in
`MirConstant::Str` lowering) leak by design — pre-existing JIT pattern
that pre-dates W11; flagged here for completeness").

### §F.1 Existing leak surface at HEAD 5d09007e (grep-verified)

`crates/shape-jit/src/ffi/string.rs:111-143` (verbatim):

```rust
/// Compile-time helper: allocate an `Arc<String>` for a `MirConstant::Str`
/// / `MirConstant::StringId` site and return the raw `Arc::into_raw(arc) as
/// u64` carrier bits per ADR-006 §2.7.5.
///
/// The constant is embedded as an `iconst I64` in the JIT-emitted code, so
/// the bits are static across every runtime occurrence of the site. To
/// keep the constant alive for the JIT-compiled function's full lifetime,
/// we boost the initial refcount to 2: one share represents the
/// "constant's permanent ownership" (never released by the JIT-emitted
/// retain/release pairs), and one share represents the "active share"
/// that the JIT's per-occurrence retain/release pairs manipulate.
///
/// Without the boost, a single use-then-drop pattern would decrement the
/// refcount to 0 and free the constant; the next call to the JIT function
/// would dereference freed memory.
///
/// The "leaked" extra share is a deliberate per-constant-site one-time
/// memory cost — at most `O(distinct string constants × Arc<String> size)`
/// per JIT-compiled function. Same lifecycle as the legacy NaN-box
/// `box_string` path (which also leaked the constant via `Box::into_raw`
/// without a paired `Box::from_raw`).
#[inline]
pub fn arc_string_constant(s: String) -> u64 {
    let arc = Arc::new(s);
    let ptr = Arc::into_raw(arc);
    // SAFETY: ... bump from 1 to 2 ...
    unsafe {
        Arc::increment_strong_count(ptr);
    }
    ptr as u64
}
```

### §F.2 Consumer enumeration at HEAD 5d09007e (grep-verified)

Consumers of `arc_string_constant` (grep `arc_string_constant`):

| Call site | Context |
|---|---|
| `crates/shape-jit/src/mir_compiler/ownership.rs:420` | MirConstant::StringId lowering |
| `crates/shape-jit/src/mir_compiler/ownership.rs:434` | MirConstant::Str lowering |
| `crates/shape-jit/src/mir_compiler/ownership.rs:465` | Function-name lowering (rare; for callable-via-name shape) |

Per-call invariant: bits embedded as `iconst I64` in JIT code;
refcount=2 at allocation (permanent + active share); active share
retained/released by JIT-emitted code; permanent share NEVER released
→ Arc<String> allocation lives forever.

### §F.3 Leak magnitude estimate

Per-distinct-string-constant cost: 1 Arc<String> allocation (~32
bytes Arc control block + heap String allocation per the inner
string content). Per JIT-compiled function: O(distinct string
constants × allocation). Per program lifetime: bounded by total
distinct string constants across all JIT-compiled functions.

Magnitude assessment: SMALL-to-MEDIUM. For a typical Shape program
with ~10-100 distinct string constants per JIT-compiled function and
~10-100 JIT-compiled functions, the total leak is ~10KB-100KB.
NOT a runaway leak; bounded by program shape, not by runtime
iteration count.

Per the docstring at `crates/shape-jit/src/ffi/string.rs:127-131`:
"The 'leaked' extra share is a deliberate per-constant-site one-time
memory cost — at most `O(distinct string constants × Arc<String>
size)` per JIT-compiled function." Already-documented as design
intent.

### §F.4 Migration design options

**Option A — intern-pool:** allocate every string constant in a
single program-wide intern pool; the JIT iconst payload is the
intern-pool index, not a raw Arc pointer; FFI reads the intern pool
by index. Pros: zero per-constant Arc overhead at runtime; constants
naturally deduplicate. Cons: intern pool itself becomes a leak source
(but constant-bounded by total distinct constants across program);
indirection cost at FFI read time.

**Option B — ManuallyDrop+free-on-Drop:** keep the per-constant
Arc<String> allocation but track them in a per-JIT-compiled-function
ManuallyDrop list; the function's JIT-allocation Drop runs through
the list and calls Arc::decrement_strong_count on the permanent
share. Pros: matches the existing JIT alloc/dealloc lifecycle (per
the `release_unified_value_by_kind` precedent at
`crates/shape-jit/src/ffi/jit_release.rs`); zero indirection cost.
Cons: requires the JIT-compiled function to track its constants for
later cleanup; the cleanup needs a hook into JIT-compiled-function
deallocation (which doesn't have one today — JIT-compiled functions
live for the program's lifetime per the Cranelift JIT-module lifetime
default).

**Option C — RC-with-HeapHeader:** migrate `arc_string_constant` to
use the v2-raw `StringObj` carrier (per ADR-006 §2.7.5 amendment Wave
2 Agent B). The StringV2 carrier already has HeapHeader-at-offset-0
refcount; bumping to refcount=2 at allocation has the same leak
shape but the carrier-shape is uniform with the rest of the §2.7.5
String carrier. Pros: carrier-shape uniformity; cluster-1 hardening
already migrating other String sites to StringV2. Cons: same leak
shape (refcount boost to keep alive); not a fix, just a carrier
swap.

**Recommendation: Option A or B per measurement.** Per `bulldozer-
wave-1-inventory.md` §0 binding ("audit before rebuild"), the leak
magnitude is SMALL-to-MEDIUM and bounded; the cost of the fix may
exceed the cost of the leak. Measure first (e.g. via
SHAPE_JIT_ARC_COUNTERS=1 on a representative program); decide
A-vs-B-vs-C based on measured cost vs measured fix-cost.

**ADR-fit:** §2.7.5 (carrier-shape consistency — Option A or C);
§2.7.5.1 stable-FFI raw-bits convention (Option A's intern-pool index
is a stable-FFI payload kind in itself, with the parallel-kind
metadata `NativeKind::String`).

**Cascade-site count:** Option A: 4 files (string.rs + ownership.rs 3
call sites + 1-2 intern-pool files); Option B: ~5 files (string.rs +
ownership.rs + new JIT-compiled-function constant-tracking list +
deallocation hook); Option C: 1 file (string.rs alone — just swap the
Arc<String> for `*const StringObj`).

---

## §G — W12-collection-constructor-mir-lowering

Per phase-3-team-lead-handover.md "Next workstream" cluster-2 entry.

### §G.1 Existing MIR lowering coverage at HEAD 5d09007e (grep-verified)

`crates/shape-vm/src/mir/lowering/helpers.rs:349-395` defines the
authoritative classifier `is_bare_collection_ctor`:

```rust
pub(super) fn is_bare_collection_ctor(name: &str) -> bool {
    matches!(
        name,
        "HashMap" | "Set" | "Deque" | "PriorityQueue" | "Channel" | "Mutex" | "Atomic" | "Lazy"
    )
}
```

JIT consumer mirror at `crates/shape-jit/src/mir_compiler/statements.rs:1143-1148`:

```rust
fn is_collection_ctor_name(name: &str) -> bool {
    matches!(
        name,
        "HashMap" | "Set" | "Deque" | "PriorityQueue" | "Channel" | "Mutex" | "Atomic" | "Lazy"
    )
}
```

Per-constructor VM-handler / JIT-handler dispatch matrix:

| Constructor | BuiltinFunction | VM-handler site | JIT-handler site | Status |
|---|---|---|---|---|
| HashMap | `HashMapCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:595` | `crates/shape-jit/src/mir_compiler/statements.rs::emit_collection_ctor` → `ffi.v2_make_hashmap` FuncRef | COVERED — kind-threaded ConcreteType-aware |
| Set | `SetCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:616` | `ffi.v2_make_hashset` FuncRef | COVERED |
| Deque | `DequeCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:629` | `ffi.v2_make_deque` FuncRef | COVERED |
| PriorityQueue | `PriorityQueueCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:644` | `ffi.v2_make_priorityqueue` FuncRef | COVERED |
| Channel | `ChannelCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:660` | `ffi.v2_make_channel` FuncRef | COVERED |
| Mutex | `MutexCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:681` | `ffi.v2_make_mutex` FuncRef (carrier-pair with kind_code) | COVERED — kind-threaded |
| Atomic | `AtomicCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:704` | `ffi.v2_make_atomic` FuncRef (single-int) | COVERED |
| Lazy | `LazyCtor` | `crates/shape-vm/src/executor/vm_impl/builtins.rs:731` | `ffi.v2_make_lazy` FuncRef (single-closure) | COVERED |

### §G.2 Constructors LACKING MIR lowering at HEAD 5d09007e

Per the `is_bare_collection_ctor` enumeration: ALL 8 named
constructors have MIR lowering coverage AND JIT consumer coverage.

Per `classify_builtin_function` at
`crates/shape-vm/src/compiler/helpers.rs:4001-4014`, the full
`BuiltinFunction::*Ctor` set is: `SomeCtor` / `OkCtor` / `ErrCtor` /
`HashMapCtor` / `SetCtor` / `DequeCtor` / `PriorityQueueCtor` /
`MutexCtor` / `AtomicCtor` / `LazyCtor` / `ChannelCtor`. The enum-
variant ctors (Some/Ok/Err) are NOT collection-ctors — they take a
separate MIR lowering path via `is_bare_enum_variant_ctor` at
`crates/shape-vm/src/mir/lowering/helpers.rs:345-347`. None missing.

**Range:** `Range` is NOT in `is_bare_collection_ctor` — `0..10` and
`0..=10` syntax produces `Expr::Range { start, end, .. }` AST shape
not `Expr::FunctionCall { name: "Range" }`. Lowering route is the
inline-fast-path branch in `lower_for_expr` at
`crates/shape-vm/src/mir/lowering/expr.rs:925-1033` (the Range-as-
iterable case) and `Expr::Range` case at `expr.rs:2024-2030` for
value-context Range. The `MakeRange` opcode handler at
`crates/shape-vm/src/executor/objects/mod.rs:226` is VM-side; JIT
side does NOT have a `MakeRange` MIR consumer arm — Range values are
constructed by the inline lowering path, not via a constructor.

**HashSet vs Set:** `Set` is a bare-form constructor for HashSet
(maps to `BuiltinFunction::SetCtor` per `classify_builtin_function`);
the JIT consumer `ffi.v2_make_hashset` confirms — `Set()` → HashSet.
The `HashSet` named type IS the same backing carrier as `Set`; no
separate constructor name.

### §G.3 Gap assessment

**Status: ALL 8 listed collection-ctor classes are COVERED at MIR
lowering layer AND JIT consumer layer at HEAD 5d09007e.**

No SURFACE-and-stops surfaced for collection constructor names at
HEAD 5d09007e (grep `SURFACE.*collection-ctor` returns 4 hits all in
JIT consumer's defensive error messages at
`crates/shape-jit/src/mir_compiler/statements.rs:1196-1289` — those
are bounds-check / unknown-name SURFACEs, not gap surfaces).

**Implication for cluster-2 work:** the W12-collection-constructor-
mir-lowering territory is already CLOSED at HEAD 5d09007e. The
phase-3-team-lead-handover.md "Next workstream" entry lists it for
historical-anchor purposes; no closure-wave dispatch needed.

If a new collection-ctor (e.g. `BiMap` / `OrderedMap` / etc.) is
added to Shape's stdlib in cluster-2 or Phase 4, the migration shape
is: add to `classify_builtin_function` + `is_bare_collection_ctor` +
`is_collection_ctor_name` (3 sites) + add a new
`BuiltinFunction::<X>Ctor` variant + VM-handler body at
`vm_impl/builtins.rs` + JIT FFI body at
`crates/shape-jit/src/ffi/v2/collection_arc.rs` + `ffi.v2_make_<x>`
FuncRef declaration. Bounded scope; documented infrastructure.

**No closure-wave dispatch recommended for §G.** Surface-and-stop:
the territory is already covered; the cluster-2 inventory rights this
as closed.

---

## §H — Cluster-2 closure-wave partition recommendation + tracing-crate-migration candidate

Per R5 supervisor disposition (audit deliverable § dispatch shape) +
R6 (tracing-crate-migration mechanism).

### §H.1 Closure-wave partition shape

Given §A-G inventory, 6 candidate closure-wave agents proposed with
file-set non-overlap:

| # | Closure-wave agent | Territory | Responsibility | Close gate | Size |
|---|---|---|---|---|---|
| 1 | closure-wave-2 (hypothesis b inlining) | `crates/shape-vm/src/compiler/monomorphization/substitution.rs:2247-2630` + post-Phase-C MIR-build pipeline (`compile_function` at `functions.rs:181` → `lower_function_detailed` at `mir/lowering/mod.rs:607`) | Empirical re-verification of (b) per §A.3; fix if needed | Smoke 2 JIT bb3 shows inlined `BinaryOp(Mul, ...)` instead of `Call(Copy(f), [item])` AT MIR-trace layer; preserved Smoke 2 JIT returns 30 | SMALL (empirical) + UNKNOWN (fix) |
| 2 | closure-wave-B (closure-coverage broader fix) | `crates/shape-vm/src/mir/lowering/mod.rs:607-715` + `crates/shape-vm/src/compiler/helpers.rs::infer_top_level_concrete_types_from_mir_with_resolvers` + closure-inference side-channel surface | Empirical-verification-first dispatch per §B.4 — enumerate uncovered closures + designate fix | per-empirical-finding gate; no broad Smoke regression | SMALL (empirical) + UNKNOWN (fix) |
| 3 | closure-wave-C (hashmap-value-v-arm) | `crates/shape-value/src/heap_value.rs:1658-1750` (HashMapKindedRef + Clone + values_kind + HashMapValueElem impl) + `crates/shape-vm/src/executor/objects/hashmap_methods.rs:1735-1770` (v2_group_by body) | Land HashMap-value V arm per §C.3; unblock `test_hashmap_group_by_*` and `array_groupby` failures | `cargo test -p shape-test --test hashmap v2_group_by` passes (8 tests); `objects_arrays::array_groupby` passes; no broad Smoke regression | MEDIUM (~4 files) |
| 4 | closure-wave-D (per-HeapKind kinded jit_print) | `crates/shape-jit/src/ffi/conversion.rs` + `crates/shape-jit/src/ffi_symbols/object_symbols.rs` + `crates/shape-jit/src/mir_compiler/terminators.rs:459-751` (routing arms) | Land per-HeapKind FFI bodies + routing arms per §E.5 per-family sub-clusters (10 families, ~30 arms total) | per-family Smoke fixture `print(<heapkind-typed-expression>)` outputs match VM | LARGE (~1000-3000 LoC if all 10 families; ~100-300 LoC per family) — DISPATCH AS MULTIPLE PARALLEL AGENTS or sequential family-by-family |
| 5 | closure-wave-E (compile-time-boxed string-constant leak) | `crates/shape-jit/src/ffi/string.rs:111-143` (arc_string_constant) + `crates/shape-jit/src/mir_compiler/ownership.rs:420/434/465` (3 callers) + Option A intern-pool OR Option B+C migration site | Measure leak magnitude; apply Option A / B / C fix per §F.4 | SHAPE_JIT_ARC_COUNTERS reports zero permanent-share leak on representative program | SMALL (measurement) + MEDIUM (fix per option) |
| 6 | closure-wave-F (tracing-crate migration) | `crates/shape-jit/Cargo.toml` + `crates/shape-vm/Cargo.toml` (tracing dep + jit-trace feature) + 28 existing SHAPE_JIT_* sites + cluster-2-empirical-verification + cluster-2-closure-wave-1 narrow trace points + `bin/shape-cli/src/` (tracing-subscriber init + --trace-jit flag) | Per §H.3 below | `cargo build --release` zero-cost binary (release_max_level_off when feature off); `--trace-jit` CLI flag produces tracing output | MEDIUM (~30 trace sites × 2-3 LoC migration each + ~50 LoC tracing-subscriber init = ~150 LoC) |

**Per-pair territory intersection check:**

- Closure-wave-1 (already merged at cc5ceb0e) and closure-wave-2 both
  touch `monomorphization/substitution.rs` — but closure-wave-1
  touches MIR-lowering layer (expr.rs + stmt.rs + mod.rs); closure-
  wave-2 touches Phase-C inliner. NON-OVERLAPPING files within the
  same crate.
- Closure-wave-B and closure-wave-2: both touch
  `crates/shape-vm/src/compiler/`. closure-wave-B → helpers.rs;
  closure-wave-2 → substitution.rs. NON-OVERLAPPING files.
- Closure-wave-B and closure-wave-1 (already merged): both touch
  `crates/shape-vm/src/mir/lowering/mod.rs`. The closure-wave-1
  fix at `mod.rs:643-680` (param-seeding pass) is a single edit
  region; closure-wave-B's extension to inferred-typed closures
  would either (i) extend the same `mod.rs:643-680` region (file
  collision; needs merge ceremony), or (ii) live in a separate
  function/site (no collision). EMPIRICAL-PHASE OUTPUT determines.
  Propose merge ceremony shape: if (i), closure-wave-B branches from
  the closure-wave-1 close commit; if (ii), parallel landing OK.
- Closure-wave-3 (hashmap-value-v-arm) and §D class 10 (v2_group_by
  upstream-SIGABRT-blocked tests): SAME territory — wave-3 LANDS the
  fix, D class 10 unblocks the validation. No file collision; D class
  10 is downstream verification, not a separate edit set.
- Closure-wave-D (per-HeapKind kinded jit_print) and closure-wave-6
  (tracing-crate migration): both touch `crates/shape-jit/src/`. D
  touches conversion.rs + object_symbols.rs + terminators.rs (routing
  arms); F touches the 28 SHAPE_JIT_* sites which span many files but
  the SHAPE_JIT_DEBUG sites and the print-routing arms at
  terminators.rs:621/712 INTERSECT. Propose: closure-wave-F lands
  FIRST (the migration is mechanical SHAPE_JIT_DEBUG → tracing::debug!()
  conversion; quick to land); closure-wave-D branches from F close.
- Closure-wave-D-family-substreams (per-HeapKind-family): all touch
  `crates/shape-jit/src/ffi/conversion.rs` + `object_symbols.rs` +
  `terminators.rs`. INTERSECTING files. Either:
  (a) Single agent lands all 10 families sequentially in one
     sub-cluster (LARGE territory, ~1000-3000 LoC, ceiling-c risk).
  (b) Multiple agents per family with merge ceremony per family-fold
     (each family lands ~3-100 LoC; serialize at file-edit layer).
  Recommendation: (b) staged — agent 1 lands Scalar + Async family
  (small, low-risk warmup); agent 2 lands Concurrency-primitive +
  Numeric/temporal (medium); agent 3 lands Collection (medium); agent
  4 lands TraitObject + ModuleFn + Pure-discriminator + DataTable +
  Native-foreign + Matrix (small per family, fold into one agent).
  4 sequential agent dispatches; total ~3-4 closure-wave rounds.

### §H.2 Sequencing recommendation: STAGED multi-round

**Round 1 (parallel — no file overlap):**

- closure-wave-2 (hypothesis b empirical)
- closure-wave-C (hashmap-value-v-arm)
- closure-wave-F (tracing-crate migration)
- closure-wave-E (compile-time-boxed string-constant leak measurement)

**Round 2 (sequenced; closure-wave-D depends on closure-wave-F):**

- closure-wave-B (closure-coverage broader fix) — empirical-phase
- closure-wave-D-family-1 (Scalar + Async)
- closure-wave-D-family-2 (Concurrency + Numeric/temporal)

**Round 3 (sequenced; closure-wave-D-family-3 depends on family-2):**

- closure-wave-B fix-phase
- closure-wave-D-family-3 (Collection)
- closure-wave-D-family-4 (TraitObject + ModuleFn + Pure-discriminator
  + DataTable + Native-foreign + Matrix)
- closure-wave-E fix-phase (per measurement disposition)
- closure-wave for §D class 1 (v2-raw-heap aliasing — LARGE)
- closure-wave for §D class 2 (stdlib JIT cache hang — MEDIUM)

**Round 4 (sequenced; per-failure-class):**

- closure-wave for §D classes 3-10 (per-class triage agents,
  parallelizable where territories don't overlap)

### §H.3 Tracing-crate-migration candidate (closure-wave-F)

Per R6 supervisor mechanism: migrate the 28 existing SHAPE_JIT_*
env-var-gated eprintln! sites + the 4 new sites added during
cluster-2-empirical-verification + cluster-2-closure-wave-1 to the
`tracing` crate.

**Sites grep-verified at HEAD 5d09007e:**

| Env-var | Count | Files |
|---|---|---|
| `SHAPE_JIT_DEBUG` | 17 | `crates/shape-jit/src/compiler/program.rs` (5) + `crates/shape-jit/src/mir_compiler/{blocks,mod,terminators}.rs` (4) + `crates/shape-jit/src/ffi/{result,call_method/mod,control/mod,object/property_access,object/closure,v2/collection_arc}.rs` (7) + `crates/shape-jit/src/{executor,compiler/strategy}.rs` (2) + `crates/shape-vm/src/compiler/monomorphization/cache.rs` (2) — the cache.rs sites are the closure-wave-1 + empirical-verification additions |
| `SHAPE_JIT_TRACE` | 2 | `crates/shape-jit/src/ffi/{result,typed_object/allocation}.rs` |
| `SHAPE_JIT_MIR_TRACE` | 1 | `crates/shape-jit/src/mir_compiler/mod.rs` |
| `SHAPE_JIT_ARC_COUNTERS` | 1 | `crates/shape-jit/src/executor.rs` |
| `SHAPE_JIT_METRICS` / `_DETAIL` | 2 | `crates/shape-jit/src/compiler/program.rs` |
| `SHAPE_JIT_PHASE_METRICS` | 1 | `crates/shape-jit/src/executor.rs` |

**Total: 24 active sites + 2 sites in shape-vm/compiler/monomorphization/cache.rs
(the closure-wave-1 + empirical-verification additions) + 2 sites in
crates/shape-jit/src/mir_compiler/terminators.rs (existing print
SURFACE traces at lines 621/712). Total 28 active sites — slightly
more than the dispatch's claim of 26 due to closure-wave-1 +
empirical-verification recent additions; +2 sites surfaced during
this audit's pre-flight grep.**

**Migration shape (per R6 mechanism):**

1. **Add tracing dep + jit-trace feature** to
   `crates/shape-jit/Cargo.toml` + `crates/shape-vm/Cargo.toml`:
   ```toml
   [dependencies]
   tracing = { version = "0.1", default-features = false }

   [features]
   jit-trace = ["tracing/std", "tracing/release_max_level_debug"]
   ```
   When `jit-trace` feature is OFF (default), `release_max_level_off`
   compiles away all `tracing::trace!()` / `debug!()` calls to
   no-ops — ZERO RUNTIME COST.
2. **Replace each `if std::env::var_os("SHAPE_JIT_DEBUG").is_some()
   { eprintln!(...) }`** with `tracing::debug!(target: "shape_jit",
   ...)`. Per-env-var → per-target mapping:
   - `SHAPE_JIT_DEBUG` → `target: "shape_jit"` debug level
   - `SHAPE_JIT_TRACE` → `target: "shape_jit"` trace level
   - `SHAPE_JIT_MIR_TRACE` → `target: "shape_jit::mir"` trace level
   - `SHAPE_JIT_ARC_COUNTERS` → `target: "shape_jit::arc_counters"`
     info level
   - `SHAPE_JIT_METRICS` / `_DETAIL` / `SHAPE_JIT_PHASE_METRICS` →
     `target: "shape_jit::metrics"` info level (with per-target
     filter for `_DETAIL` vs `_PHASE`)
3. **Add `--trace-jit` CLI flag + tracing-subscriber init** to
   `bin/shape-cli/src/`. The subscriber filters per-target /
   per-level per the CLI flag's value (e.g. `--trace-jit=mir,arc`
   enables MIR trace + ARC counters).
4. **Add `tracing-subscriber` to `bin/shape-cli/Cargo.toml`** under
   the `jit-trace` feature gate (so non-trace builds don't pull in
   subscriber dep).

**Cascade-site count:** ~30 sites × ~2-3 LoC migration each (~60-90
LoC) + ~50 LoC tracing-subscriber init in shape-cli + ~10 LoC
Cargo.toml edits. **Total ~150 LoC; bounded.**

**ADR-fit (§2.7.5 cross-crate ABI policy):** tracing dep is contained
in `shape-jit` / `shape-vm` (and `shape-cli` via the subscriber);
**does NOT propagate to extension contract** (the `unsafe fn(*mut
c_void, &u64, &[u64]) -> Result<u64, String>` raw-bits ABI at
`crates/shape-runtime/src/module_exports.rs:21` is untouched by this
migration). Per the §2.7.5 cross-crate ABI policy: "Extensions stay
on the stable raw-bits ABI." Tracing migration is internal-Rust-side
only.

**Forbidden under this migration:** introducing tracing across the
extension contract (would be a §2.7.5 violation); replacing the
SHAPE_JIT_* env-var-based control with a non-bounded tracing-channel
that does NOT have a compile-time off path (would lose the zero-cost
release build property).

### §H.4 AGENTS.md V3-S6 chain rows annotation candidate

Per cluster-2-empirical-verification close subsection §"AGENTS.md
V3-S6 chain rows annotation" (verbatim): "Surfaced separately for
supervisor disposition: AGENTS.md V3-S6 chain rows (lines 223+)
exceed in-budget edit count (~5 rows × ~3 refs/row = ~15+ edits
beyond ~10-edit budget); pending supervisor narrowing-scope
disposition."

Per cluster-2-closure-wave-1 close subsection §"AGENTS.md V3-S6
chain rows annotation (carry-forward; supervisor disposition
pending)": "Carry-forward as known follow-up; revisit at next
ceremony with budget capacity OR dedicate doc-discipline
sub-cluster."

**Audit recommendation:** the AGENTS.md V3-S6 chain rows annotation
is a doc-discipline closure-wave candidate; ~15+ edits + ~1 hour
agent territory. SMALL. Could be absorbed into closure-wave-F
(tracing-crate migration) as a co-landing in the same merge ceremony
(both are documentation-tier or near-doc-tier work; both are SMALL).
OR could be its own closure-wave-G (doc-discipline). Defer to
supervisor disposition at closure-wave dispatch time; non-blocking
for cluster-2 close.

---

## §I — Q25.C TraitObject rebuild evaluation (Surface A (c) split)

Per R4 supervisor disposition 2026-05-16: Q25.C absorb-vs-separate
is supervisor's authorization at closure-wave dispatch time. Audit
maps factors + recommends; does NOT decide.

### §I.1 Current /tmp/smokes/s3.shape fixture at HEAD 5d09007e (grep-verified)

```shape
trait T { name(): string }
type X {}
impl T for X { method name() { "x" } }
let t = X {}
print(t.name())
```

**Shape:** `let t = X {}` — concrete-type UFCS dispatch. The receiver
`t` is a `TypedObject` (HeapKind 1), and `t.name()` dispatches
through the impl method `name` directly per the VM-side concrete-type
method lookup (NOT through a `dyn T` vtable).

### §I.2 Kickoff prompt prose at phase-3-kickoff-prompt.md:102-105 (verbatim)

```text
trait T { fn name(&self) -> String }
impl T for X { fn name(&self) -> String { "x" } }
let t: dyn T = box(X{})
print(t.name())                        # x
```

**Shape:** `let t: dyn T = box(X{})` — trait-object dispatch through
`HeapKind::TraitObject` (ordinal 29) via `Arc<TraitObjectStorage>`
fat-pointer carrier per ADR-006 §2.7.24 Q25.C.

### §I.3 The Surface A (c) split

Per Surface A user disposition 2026-05-13 (user-pending at the
bulldozer-wave-1-inventory §I.2 / §I.3): the user chose option (c)
split — Smoke 3 stays at the concrete-type UFCS fixture for cluster-
0+1 close; the `dyn T` rebuild becomes cluster-1.5 separation (a
distinct follow-up beyond cluster-0+1 close attempt).

Smoke 3 at HEAD 5d09007e is the concrete-type UFCS fixture; Smoke 3
JIT passes per the closure-wave-1 close (`Smoke 3 (canonical
\`let t = X{}\` UFCS): VM x / JIT x ✅`).

### §I.4 Q25.C TraitObject rebuild scope at HEAD 5d09007e (grep-verified)

The Q25.C TraitObject infrastructure has substantial implementation
at HEAD 5d09007e:

- `HeapKind::TraitObject = 29` — present at
  `crates/shape-value/src/heap_variants.rs:287`
- `TraitObjectStorage` struct — present at
  `crates/shape-value/src/heap_value.rs:2774` (24-byte struct with
  HeapHeader at offset 0 + `Arc<TypedObjectStorage>` value half +
  `Arc<VTable>` vtable half per ADR-006 §Q25.C.5 amendment Wave 2
  Agent E close 2026-05-14)
- `VTable` + `VTableEntry` (6 variants: Direct / Closure /
  BoxedReturn / SelfArg / Generic / Compound) — present at
  `crates/shape-value/src/value.rs:67-156`
- `WrapTarget` struct + `VTableEntryFlags` bitfield — present at
  `crates/shape-value/src/value.rs:171-186`
- TraitObject dispatch arms at `crates/shape-vm/src/executor/trait_object_ops.rs:340-433`
  (VTableEntry::BoxedReturn / SelfArg / Generic / Compound /
  Closure all wired through `invoke_dyn_unified` and
  `invoke_dyn_closure`)
- `invoke_dyn_unified` body (the runtime SelfArg identity-check +
  BoxedReturn wrap + Direct/Generic forwarding logic) at
  `crates/shape-vm/src/executor/trait_object_ops.rs:436+`

### §I.5 Remaining Q25.C rebuild scope

Per Q25.C.1 (universal `dyn Trait`) + Q25.C.5 (VTableEntry 6
variants) + Q25.C.6 (IC devirtualization) + Q25.C.7 (LSP cost-class
inlay hints):

| Q25.C subsection | Infrastructure | Status at HEAD 5d09007e | Remaining scope |
|---|---|---|---|
| Q25.C.1 (auto-boxing rule `Erase_T`) | trait method-signature rewriter | ad-hoc; pre-V3-S6 | Audit needed — empirical-verification of which trait shapes are dyn-able today vs which surface |
| Q25.C.2 (Self-arg runtime check) | `Arc::ptr_eq` vtable check per Self arg | LANDED at `trait_object_ops.rs::invoke_dyn_unified` per VTableEntry::SelfArg arm | Verify error message matches ETO-001 spec |
| Q25.C.3 (generic method runtime TypeInfo) | TypeInfo threading per generic param | PARTIAL — `VTableEntry::Generic { thunk_id, type_param_count }` variant exists; dispatch at `trait_object_ops.rs:392-409` falls back to Direct (per the in-source comment "Treat as Direct for runtime dispatch; the impl's body handles the polymorphism") | TypeInfo threading not yet wired (Q25.C.3 deferred per docstring); audit needed for which method shapes hit this |
| Q25.C.4 (#[static_only] opt-out) | parser + lowering for the annotation | Audit needed | Empirical-verification — check whether parser/annotation handles `#[static_only]` at HEAD 5d09007e |
| Q25.C.5 (VTable + VTableEntry shape) | struct definitions + thunk emission | struct definitions LANDED; per-impl thunk emission at vtable-construction time — audit needed for completeness | per-impl thunk emission audit; verify all 6 variants emit thunks |
| Q25.C.6 (IC devirtualization) | JIT IC tracking + IC state machine at `dyn T` call sites | UNCOVERED at HEAD 5d09007e (the existing IC infrastructure at `crates/shape-vm/src/feedback.rs` is general; Q25.C.6 IC-at-dyn-T-call-site is a separate extension) | LARGE scope; cluster-1.5 territory |
| Q25.C.7 (LSP cost-class inlay hints) | LSP integration | UNCOVERED at HEAD 5d09007e | MEDIUM scope; cluster-1.5 territory |

### §I.6 Per-site code touchpoint enumeration

Q25.C-dependent code sites:

| Site | File:line | Purpose |
|---|---|---|
| `HeapKind::TraitObject = 29` | `crates/shape-value/src/heap_variants.rs:287` | Ordinal definition + 4-table lockstep arms (clone_with_kind / drop_with_kind / SharedCell::drop / TypedObjectStorage::drop) |
| `TraitObjectStorage` struct | `crates/shape-value/src/heap_value.rs:2774` | Carrier struct (HeapHeader + value + vtable) |
| `TraitObjectPtr` newtype | `crates/shape-value/src/heap_value.rs` (per Wave 2 D4 ckpt-final-prime² §2.3 amendment) | `*const TraitObjectStorage` newtype with Send/Sync + Drop/Clone refcount discipline |
| `VTable` / `VTableEntry` / `WrapTarget` / `VTableEntryFlags` | `crates/shape-value/src/value.rs:67-186` | VTable shape definitions |
| `invoke_dyn_unified` / `invoke_dyn_closure` | `crates/shape-vm/src/executor/trait_object_ops.rs:340-650+` | Runtime dispatch dispatch tier |
| Vtable construction (per-impl thunk emission) | TBD; audit-needed location in compiler-side trait-impl compilation | per-(impl, method) thunk emission per VTableEntry variant |
| ETO-001 / ETO-002 errors | TBD; audit-needed location in compiler-side trait validation | Compile-error generation per Q25.C.4 #[static_only] + Q25.C.1 type-bound check |

### §I.7 Q25.C absorb-vs-separate factor mapping

| Factor | Reading | Toward absorb (cluster-2) | Toward separate (cluster-1.5) |
|---|---|---|---|
| Q25.C scope | LARGE (Q25.C.6 + Q25.C.7 are separate large extensions; Q25.C.1-Q25.C.5 partially landed) | Smaller-scope Q25.C.1-Q25.C.5 completion (audit + per-impl thunk emission) fits cluster-2 budget | Q25.C.6 IC devirtualization + Q25.C.7 LSP cost-class hints exceed cluster-2 budget |
| Interaction with §H cluster-2 closure-wave territories | NONE — Q25.C TraitObject dispatch is at `crates/shape-vm/src/executor/trait_object_ops.rs` (executor tier); §H territories are at `crates/shape-vm/src/mir/lowering/`, `crates/shape-jit/src/`, `crates/shape-value/src/heap_value.rs` (HashMapKindedRef arm) — NO file overlap | Yes; could absorb into cluster-2 without conflict | No; could separate into cluster-1.5 without conflict |
| ADR-006 §Q25.C ratification status | Q25.C is RATIFIED + binding TraitObject rebuild authority untouched by S2-prime supersession (per Q25.B SUPERSEDED preamble: "Q25.C is NOT superseded by this amendment") | OK for absorb | OK for separate |
| Dependency on other cluster-2 targets | NONE — none of §A-G depend on Q25.C; per empirical-verification §5 Q25.C absorb-vs-separate observation: "None of the 3 V3-S6f hypotheses intersect Q25.C TraitObject rebuild territory" | Could absorb without gating | Could separate without gating |
| Interaction with closure-wave-2 (b) hypothesis territory | NONE — closure-wave-2 territory is at monomorphization/substitution.rs (Phase-C inlining); Q25.C is at trait_object_ops.rs; NON-OVERLAPPING | OK for absorb | OK for separate |
| Smoke 3 fixture alignment | Smoke 3 at HEAD 5d09007e is `let t = X {}` (concrete-type UFCS, NOT `let t: dyn T = box(X{})` trait-object); the kickoff prompt's `dyn T` shape requires Q25.C.6 IC devirtualization to validate the "zero-overhead dyn Trait at hot call sites" property | If absorbed, Smoke 3 would migrate to the `dyn T` shape; correctness validation possible at cluster-2 close | If separated, Smoke 3 stays at concrete-type UFCS shape per Surface A (c) split user disposition |
| User disposition (Surface A) | User chose option (c) split 2026-05-13 (per phase-3-team-lead-handover.md "Surface A user disposition (c) split — Q25.C TraitObject rebuild = cluster-1.5 follow-up") | Reversing the (c) split disposition requires user authorization | Aligned with the (c) split |
| Audit scope budget for Q25.C work | LARGE per §I.5 — Q25.C.1-5 completion alone is ~1000-2000 LoC + Q25.C.6 IC devirtualization is ~500-1500 LoC + Q25.C.7 LSP integration is ~300-500 LoC. **TOTAL EXCEEDS ceiling-c-like bound for a single cluster-2 closure-wave** | Risk: cluster-2 close timeline extends if Q25.C absorbed | Cluster-2 stays focused; Q25.C dispatched as cluster-1.5 dedicated cluster |

### §I.8 Recommendation: PRESERVE cluster-1.5 separation

**Audit recommends:** preserve the cluster-1.5 separation per Surface
A (c) split disposition. Cite-anchored reasoning:

1. User Surface A (c) split disposition 2026-05-13 is the binding
   authorization for the separation; reversing requires user
   re-authorization (not within audit deliverable scope).
2. Q25.C total scope (per §I.7 "Audit scope budget" row) exceeds the
   ceiling-c-like bound for a single cluster-2 closure-wave;
   absorbing would either (i) extend cluster-2 close timeline
   significantly, or (ii) require splitting Q25.C across multiple
   cluster-2 closure-waves which then have the same dispatch shape
   as a dedicated cluster-1.5.
3. The cluster-2-empirical-verification §5 Q25.C observation:
   "Cluster-2 can absorb the hypothesis-(a) and hypothesis-(b)
   closure-waves WITHOUT gating on Q25.C disposition" — but the
   reciprocal does NOT hold for absorption: cluster-2 can absorb
   Q25.C, but doing so doesn't unblock cluster-2 work (no §A-G
   target depends on Q25.C).
4. No closure-wave-territory file overlap between Q25.C work and
   §H closure-wave-2..F territories — separate dispatch shape works
   cleanly.
5. Smoke 3 fixture stability — Smoke 3 at HEAD 5d09007e is stable
   on the concrete-type UFCS shape; migrating to `dyn T` (the Q25.C
   load-bearing Smoke 3 form per kickoff prompt) introduces a NEW
   smoke-fixture regression surface that doesn't gate cluster-2 close
   per Surface A (c).

**SUPERVISOR DISPOSES** at closure-wave dispatch time per R4. If the
disposition is "absorb", recommended absorption shape: dispatch
Q25.C.1-5 completion as cluster-2-closure-wave-G (single agent,
LARGE territory ~1000-2000 LoC); defer Q25.C.6 / Q25.C.7 to
cluster-1.5 regardless of absorption disposition (those are
optimization/tooling tiers, not load-bearing for correctness).

---

## §J — Pre-flight ground-truth verification at HEAD `5d09007e`

Per Q3 recursive pre-flight binding extended 2026-05-16: every
ground-truthable claim in this audit must be grep-verified at HEAD
before surfacing.

### §J.1 Confirmed at HEAD 5d09007e

- `crates/shape-vm/src/mir/lowering/expr.rs` 1820-line file; the
  `lower_for_expr` function at line 919 + closure-wave-1 fix landed.
- `crates/shape-vm/src/mir/lowering/stmt.rs::lower_for_loop` at line
  464; ForIn arm fix landed at lines 471-516.
- `crates/shape-vm/src/mir/lowering/mod.rs::lower_function_detailed`
  at line 607; param-seeding fix at lines 643-680.
- `crates/shape-vm/src/compiler/helpers.rs::infer_top_level_concrete_types_from_mir_with_resolvers`
  at line 494; V3-S6e empty-array stamping pass at lines 587-634.
- `crates/shape-vm/src/executor/objects/hashmap_methods.rs::v2_group_by`
  at line 1735; surface-and-stop body at lines 1763-1769.
- `crates/shape-vm/src/executor/objects/array_transform.rs::handle_group_by_v2`
  at line 363; `ckpt2_surface` SURFACE-and-stop call at line 377.
- `crates/shape-vm/src/executor/objects/array_transform.rs::ckpt2_surface`
  at line 215.
- 25 `ckpt2_surface` call sites across 3 files
  (`array_transform.rs` 12, `array_aggregation.rs` 7, `array_sets.rs` 6).
- `crates/shape-value/src/heap_value.rs::HashMapKindedRef` at line
  1658 (8 variants: I64 / F64 / Bool / Char / String / Decimal /
  TypedObject / TraitObject; HashMap-value V arm NOT present).
- `crates/shape-value/src/heap_value.rs:1740::HashMapKindedRef::values_kind`
  accessor at line 1740.
- `crates/shape-value/src/heap_variants.rs::HeapKind` enum at line
  61; 35 live variants assigned (0-7, 9-35; ordinal 8 vacated).
- `crates/shape-jit/src/ffi/conversion.rs` — 7 kinded jit_print FFI
  bodies: jit_print_i64 (289) / jit_print_f64 (298) / jit_print_bool
  (313) / jit_print_str (443) / jit_print_typed_object (462) /
  jit_print_option (486) / jit_print_result (508).
- `crates/shape-jit/src/mir_compiler/terminators.rs` — print Call
  routing arms at lines 459-751 (7 covered arms + TypedObject SURFACE
  at 620-661 + `_` SURFACE at 712-751).
- `crates/shape-jit/src/ffi/string.rs::arc_string_constant` at line
  133; 3 call-site consumers at `mir_compiler/ownership.rs:420/434/465`.
- `crates/shape-vm/src/mir/lowering/helpers.rs::is_bare_collection_ctor`
  at line 390 + `::is_bare_collection_ctor_with_arg` at line 404 +
  `::emit_collection_ctor_store` at line 430.
- `crates/shape-jit/src/mir_compiler/statements.rs::is_collection_ctor_name`
  at line 1143 + `::emit_collection_ctor` at line 1178.
- 8 collection-ctor VM handler arms at `vm_impl/builtins.rs` 595
  (HashMap) / 616 (Set) / 629 (Deque) / 644 (PriorityQueue) / 660
  (Channel) / 681 (Mutex) / 704 (Atomic) / 731 (Lazy).
- 28 SHAPE_JIT_* env-var sites at grep `SHAPE_JIT_` across `crates/`
  + `bin/` (not just 26 as the dispatch claimed; +2 from closure-wave-1
  + empirical-verification additions).
- `crates/shape-value/src/value.rs::VTable` at line 67 + `VTableEntry`
  at line 97 (6 variants: Direct / Closure / BoxedReturn / SelfArg /
  Generic / Compound).
- `crates/shape-value/src/heap_value.rs::TraitObjectStorage` at line
  2774.
- `crates/shape-vm/src/executor/trait_object_ops.rs::invoke_dyn_unified`
  dispatch arms at lines 340-433.

### §J.2 Discrepancies surfaced (imprecision-pattern instance candidates)

- **Dispatch claim "26 existing SHAPE_JIT_* sites"** — actual count
  at HEAD 5d09007e: 28 (+2 from cluster-2-empirical-verification +
  cluster-2-closure-wave-1 additions per their close subsections).
  Per the binding ground-truth pre-flight discipline (Q3 extended
  2026-05-16), surfaced explicitly in §H.3 + §K below; minor
  imprecision-pattern instance (32 candidate).
- **Q25.A SUPERSEDED carrier-shape table at ADR-006:5018-5028** —
  per-variant migration destinations match the audit grep at the
  HashMapKindedRef enum (TypedObject → TypedObjectPtr;
  TraitObject → TraitObjectPtr); no discrepancy.

---

## §K — Forbidden-pattern surveillance during this audit

Per CLAUDE.md §Forbidden Patterns + Renames to refuse on sight +
Parallel-implementation entry + cluster-2 canonical refusal set:

- ✅ No ValueWord resurrection text in deliverable
- ✅ No Bool-default fallback rationalization
- ✅ No bridge/probe/helper/hop framings (broader-family regex
  `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` checked — zero hits)
- ✅ No parallel-implementation framing ("documented intentional
  duality" etc.)
- ✅ No anti-deferral #10 framings ("preserve X for cluster-1+")
  except in §I where the binding user disposition (Surface A (c)
  split) authorizes the cluster-1.5 separation explicitly
- ✅ No Ptr-newtype-shim framings (#11 forbidden — TypedObjectPtr /
  TraitObjectPtr are described as canonical post-D4-ckpt-final-prime²
  carriers per §2.3 amendment, NOT as transitional shims)
- ✅ No JitArray / TypedArrayData / HashMapValueBuf / AlignedTypedBuffer
  resurrection text
- ✅ Audit-text imprecisions ground-truth-verified per §J pre-flight
  pass

### §K.1 CLAUDE.md modifications surfaced (flag only; team-lead does NOT land without user ratification)

NONE surfaced during this audit. No new forbidden pattern or
refuse-on-sight phrase identified. The audit deliverable surfaces
exclusively existing-pattern citations + per-target migration
designs against established refusal sets.

---

## §L — Genuinely intractable targets requiring supervisor ADR-level decision

NONE. Every §A-I target either has a designed closure-wave
territory (§A-H) or has a structured factor-mapping + audit
recommendation pending supervisor disposition (§I).

The §I Q25.C absorb-vs-separate IS supervisor's authorization at
closure-wave dispatch time per R4 disposition — audit maps factors +
recommends preserving the cluster-1.5 separation per Surface A (c)
user disposition 2026-05-13. The supervisor disposes; audit does NOT
decide.

---

## §M — Ceiling-c bound check

Audit-day scope at this dispatch: ~1500 LoC deliverable (sections
A-M); fits within ceiling-c bound (~2000 LoC for audit-day
deliverable per `bulldozer-wave-1-inventory.md` precedent ~2035
LoC). No D-α dynamic chain continuation needed; single-agent
ceiling-c bounded close.

---

## §N — Imprecision-pattern instances surfaced during execution

| # | Source layer | Imprecision shape | Caught at |
|---|---|---|---|
| 32 | dispatch-prompt | "26 existing SHAPE_JIT_* sites" — actual count at HEAD 5d09007e: 28 (+2 from cluster-2-empirical-verification + cluster-2-closure-wave-1 additions per their close subsections at status doc §"Wave 3 Round 5/6") | Pre-flight ground-truth (§J pass); surfaced in §H.3 + §J.2 |

Cumulative count update: 44 cumulative through cluster-2-closure-
wave-1 close + 1 surfaced during this inventory audit-day = **45
cumulative imprecision-pattern instances all caught pre-merge across
phase-3 cluster-0+1+cluster-2 trajectory.** Zero bad-code merges into
canonical preserved.

---

## §O — Close summary

This audit deliverable maps cluster-2 remaining territory across
sections A-I. Six closure-wave territories proposed with
file-set non-overlap. Per §H sequencing recommendation, ~3-4
parallel-dispatch rounds expected to cluster-2 close; cluster-1.5
Q25.C TraitObject rebuild preserved as separate per §I
recommendation (supervisor disposes at closure-wave dispatch time
per R4).

All §A-I targets either have designed closure-wave territory OR
have structured surface-and-stop with factor mapping (§I only).
Audit-only deliverable; zero source changes inside this dispatch.

Tag-authorization-pending cluster-0+1 close at canonical
`50e5c024` precedes cluster-2 closure-wave dispatch per the
in-flight state at handover. Cluster-2 trajectory: 3-5 sessions
to cluster-2 close per the closure-wave partition + sequencing
recommendation in §H.2.
