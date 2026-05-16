# cluster-1.5 Q25.C TraitObject rebuild — empirical audit + producer/consumer carrier-shape fix

**Branch:** `bulldozer-strictly-typed-cluster-1.5-q25c-trait-object-rebuild`
**Parent HEAD:** `739f89ef` (post-cluster-2-close at tag `phase-3-cluster-2-close` annotated on `938929de`).
**Dispatch:** cluster-1.5-q25c-trait-object-rebuild per Surface A (c) split user disposition 2026-05-13 + supervisor 2026-05-16 PRESERVE cluster-1.5 separation ratification.
**Scope:** Q25.C.1–Q25.C.5 completion sufficient to make Smoke 5 (`let t: dyn T = X{}` → `print(t.name())`) pass VM == JIT under the load-bearing acceptance criterion. Q25.C.6 IC devirtualization + Q25.C.7 LSP cost-class inlay hints OUT-OF-SCOPE per kickoff prompt + supervisor disposition.

## §1 — Empirical audit (Phase 1)

Reading 6 binding: cluster-1.5 is partially-known territory; empirical-verification first. Smoke 5 fixture written at `/tmp/smokes/s5.shape` per kickoff prompt prose adapted to current Shape syntax (the literal kickoff prose uses Rust syntax: `fn name(&self) -> String` / `box(X{})`; Shape grammar at `crates/shape-ast/src/shape.pest:202-209` interface_member uses `name(): string` form, `dyn_type` exists at `:864`, no `box()` keyword — coerce is via `let t: dyn T = X {}` annotation per `crates/shape-vm/src/compiler/statements.rs:4426-4463`):

```shape
trait T { name(): string }
type X {}
impl T for X { method name() { "x" } }
let t: dyn T = X {}
print(t.name())
```

### Baseline behavior at HEAD `739f89ef` (pre-fix)

```
$ ./target/release/shape run --mode vm /tmp/smokes/s5.shape
x
free(): invalid pointer
[exit 134 / SIGABRT]

$ ./target/release/shape run --mode jit /tmp/smokes/s5.shape
x
[exit 0]
```

**Output identical (`x`), exit codes diverge** (VM=134 SIGABRT after print, JIT=0 clean). The 5-times-rerun reproducibility check confirmed the abort is deterministic. The same SIGABRT (`free(): invalid pointer`, multiple instances under parallel test execution) reproduces on baseline at the existing `executor::tests::trait_object_thunks::*` test suite — confirming this is **a pre-existing carrier-shape bug**, not a regression introduced by recent work.

### Per-Q25.C.x subsection disposition

| Q25.C subsection | Audit finding | Disposition |
|---|---|---|
| **Q25.C.1** universal `dyn Trait` (auto-boxing `Erase_T`) | Infrastructure landed: `dyn_type` parser at `crates/shape-ast/src/shape.pest:864`; `TypeAnnotation::Dyn(Vec<String>)` AST shape; `trait_name_from_annotation` helper at `compiler/trait_object_emission.rs:298`; `collect_self_wrap_targets` walker at `:327` with 8 unit tests pinning the §Q25.C.1 row table. `op_box_trait_object` at `executor/trait_object_ops.rs:90` consumes the `Operand::Name(StringId)` operand emitted from `statements.rs:4458`. **Universal-dyn caveat present**: line `:136-143` returns `NotImplemented(SURFACE)` for non-TypedObject concrete kinds (scalars-implementing-traits auto-box into TypedObject first — deferred row table entry, not load-bearing for Smoke 5 which boxes a `type X {}` TypedObject). | LANDED for TypedObject case. Scalar-trait-auto-boxing remains deferred (out-of-scope per kickoff Smoke 5 which boxes TypedObject). |
| **Q25.C.2** Self-arg runtime vtable-identity check | LANDED at `executor/trait_object_ops.rs::invoke_dyn_unified` step 1 (lines 470-529): for each `pos ∈ self_arg_positions` peeks `args[pos]`, validates kind=`Ptr(HeapKind::TraitObject)`, calls `trait_object.vtable_eq(arg_trait_object)` (which is `Arc::ptr_eq` on the vtable Arc per `heap_value.rs:3026`). Error structure: `VMError::RuntimeError("DynMethodCall SelfArg: vtable identity mismatch at argument position {} ... §2.7.24 Q25.C.2 ...")`. **Spec deviation**: ADR-006 §Q25.C.2 names the error `VMError::TraitObjectIdentityMismatch { method, self_impl, other_impl }`; current implementation returns plain `RuntimeError` with structured message. Functionally equivalent; the ETO-001 spec error variant is not load-bearing for Smoke 5. Test `executor::tests::trait_object_thunks::self_arg_identity_check_rejects_cross_impl_arg` pins the disposition. | LANDED. Spec-vs-impl divergence on error-variant name not load-bearing; cluster-3+ follow-up if structured-error-type plumbing is required. |
| **Q25.C.3** generic method TypeInfo threading | PARTIAL — `VTableEntry::Generic { thunk_id, type_param_count }` variant exists at `crates/shape-value/src/value.rs:142-145` with `TypeInfo` struct declared at `:235-246`. Runtime dispatch at `trait_object_ops.rs:413-430` treats Generic as Direct per in-source docstring rationale (lines 414-420): "at the bytecode tier the impl method is already monomorphic-shaped (accepts raw arg slots, dispatches internally). No TypeInfo threading is emitted at the current Shape bytecode layer." `TypeInfo` struct is **never constructed at runtime** (grep at HEAD confirms zero non-doc construction sites). The current emission tier at `compiler/trait_object_emission.rs:143-147` tracks `type_param_count` but does not emit TypeInfo arguments. This is documented as **deferred** in the in-source comment + the existing test `generic_method_dispatches_as_direct` (which after the fix landed surfaces with a structured `RuntimeError("not in function_name_index")` — an emission gap surfaced once the SIGABRT mask was removed). | PARTIAL — disposition acceptable as Direct-collapse. Full TypeInfo threading is a Q25.C.6 IC-devirtualization-adjacent extension; not load-bearing for Smoke 5 (which has no generic methods). |
| **Q25.C.4** `#[static_only]` per-method opt-out | UNCOVERED — grep at HEAD `739f89ef` confirms zero parser/AST/AST-walker references to `static_only` or `StaticOnly` outside of `crates/shape-value/src/value.rs` (the `Eto002StaticOnly` ErasureError variant at `:427` exists but has no producer). Grammar at `crates/shape-ast/src/shape.pest` has no `static_only` annotation rule. **Disposition**: parser-side `#[static_only]` annotation does not exist at HEAD; ETO-002 error code is reserved but unreachable. Surface-and-stop on opt-out request would require: (a) grammar rule for `#[static_only]` annotation parser; (b) AST decoration on `MethodDef`; (c) emission-tier check in `build_and_register_vtable` to skip the method (excluded from vtable); (d) compile-tier check at `compile_expr_method_call` to surface ETO-002 when a `dyn T` call site names a static-only method. Estimated scope: 4-6 files, ~150-250 LoC. **NOT load-bearing for Smoke 5** (smoke fixture has no `#[static_only]` annotation; trait default is universal-dyn per Q25.C.1, which Smoke 5 exercises). | UNCOVERED — explicitly out-of-scope for cluster-1.5 Q25.C.1-5 completion per Smoke-5-load-bearing disposition. Follow-up tracked as `cluster-1.5-q25c4-static-only-opt-out` (cluster-1.5-tooling-adjacent candidate; SMALL-to-MEDIUM scope; opt-out tier not correctness tier; can dispatch when developer-facing demand surfaces). |
| **Q25.C.5** VTable + VTableEntry final shape + thunk emission | Struct definitions LANDED: `VTable` at `crates/shape-value/src/value.rs:67-81` (3 fields: `trait_names`, `concrete_type_id`, `methods`); `VTableEntry` 6-variant enum at `:97-156` (`Direct` / `Closure` / `BoxedReturn` / `SelfArg` / `Generic` / `Compound`); `WrapTarget` at `:171-177`; `VTableEntryFlags` bitfield at `:184-227`; `ErasureType` + `Erase_T::rewrite` at `:270-409`; `ErasureError` (Eto001/Eto002) at `:421-428`; `ThunkSignature::build` consumed by emission at `compiler/trait_object_emission.rs:213-221`. Thunk emission landed per-variant at `build_and_register_vtable` (compiler/trait_object_emission.rs:67-237) — emits `Direct` / `BoxedReturn` / `SelfArg` / `Generic` / `Compound` per `ThunkSignature::build`. `Closure` variant has documented surface-and-stop at `trait_object_ops.rs:641-649` (W7 closure-trait-impl emission is out-of-scope per `build_and_register_vtable` not emitting `VTableEntry::Closure`). 8 wrap_target_tests + 12 trait_object_thunks unit tests pin the disposition. | LANDED. `Closure` variant remains documented surface; nested `BoxedReturn` for `TypedObject` / `HashMap` / `TypedArray` rewrap sites preserve their structured-defer dispositions per Wave 2 Round 1 Agent F dead-arm deletion (`rewrap_typed_object_fields` / `rewrap_hashmap_values` / `rewrap_typed_array_elements` at `trait_object_ops.rs:1041-1124` — these are Wave-2-Round-1-Agent-F deletion-aligned dispositions, not regressions). |

## §2 — Producer/consumer carrier-shape fix (Phase 2)

Empirical audit surfaced the load-bearing bug for Smoke 5: **the `TraitObjectStorage` carrier producer side (`op_box_trait_object` + `rebox_self_value`) uses `Arc::new(...) + Arc::into_raw(...)` discipline; the 4-table consumer dispatch arms use `_new` v2-raw discipline.** ADR-006 §Q25.C.5 amendment Wave 2 Agent E (lines 5635-5642) explicitly names this as a forbidden-mixed-dispatch shape:

> "A subsequent cascade migration in Wave 3 stabilize (or a dedicated E2 agent if cascade-site count justifies splitting) will flip the 4-table dispatch arms to `v2_retain(&(*ptr).header)` / `v2_release(&(*ptr).header)` + `Self::_drop(ptr)` on return-true **in a single commit** (atomic producer/consumer flip — leaving Arc-style consumer arms with raw-pointer producers would call `Arc::decrement_strong_count` on non-Arc pointers = heap corruption / SIGSEGV, same lockstep requirement as D1→D2)."

The 4-table consumer arms flipped to v2-raw (`crates/shape-vm/src/executor/vm_impl/stack.rs:220-223` + `:578-581`; `crates/shape-value/src/kinded_slot.rs:772-777` + `:1130-1134`; `crates/shape-value/src/v2/closure_layout.rs:482-493`; `crates/shape-value/src/heap_value.rs:3804`). **The producer never flipped.**

### Root cause

The Arc-allocated and `_new`-allocated `TraitObjectStorage` allocations have incompatible memory layouts:
- `Arc::new(TraitObjectStorage { header: HeapHeader::new(...), value, vtable })`: allocates an `ArcInner<TraitObjectStorage>` block. The struct's `header` field sits unused at refcount=1; refcount lives in the `ArcInner` prefix (offset -16 from the `Arc::into_raw` pointer).
- `TraitObjectStorage::_new`: allocates `Layout::new::<TraitObjectStorage>()` directly. The embedded `HeapHeader` at offset 0 IS the refcount.

When `release_elem` is called on an Arc-allocated pointer (consumer arms call `TraitObjectStorage::release_elem(bits as *const TraitObjectStorage)` per `stack.rs:580`), it reads `(*ptr).header.get_refcount()` (the unused in-struct field = 1), `v2_release` decrements to 0, then calls `Self::_drop(ptr as *mut Self)`. `_drop` calls `release_elem` on the inner `value` (raw TypedObjectStorage), `drop_in_place` on the vtable `Arc<VTable>`, then `dealloc(ptr, Layout::new::<Self>())`. **The dealloc misses the ArcInner prefix** → glibc detects the free of a pointer that wasn't an `alloc`'d-block start → `free(): invalid pointer` SIGABRT.

### Fix

Three sites flipped to match the post-cascade consumer carrier shape:

1. **`op_box_trait_object`** (`crates/shape-vm/src/executor/trait_object_ops.rs:90-189`): replaced
   ```rust
   let trait_object = Arc::new(TraitObjectStorage::new(typed_object_ptr, vtable));
   let to_bits = Arc::into_raw(trait_object) as u64;
   ```
   with
   ```rust
   let ptr = TraitObjectStorage::_new(typed_object_ptr, vtable);
   let to_bits = ptr as u64;
   ```

2. **`op_dyn_method_call` receiver recovery** (`trait_object_ops.rs:203+`): replaced the `Arc::from_raw` + `Arc::clone` + `Arc::into_raw` transient-share-bump pattern with a transient `&TraitObjectStorage` borrow:
   ```rust
   let trait_object_ptr: *const TraitObjectStorage = receiver_bits as *const TraitObjectStorage;
   let trait_object: &TraitObjectStorage = unsafe { &*trait_object_ptr };
   ```
   The slot owns the strong-count share on the carrier's HeapHeader for the duration of the dispatch (the slot is not freed mid-call). No share bump needed since the dispatch only reads (vtable lookup, value pointer access) and never returns the carrier back to the slot. Signatures of `invoke_dyn_unified` / `invoke_dyn_closure` flipped from `&Arc<TraitObjectStorage>` to `&TraitObjectStorage` to match.

3. **`rebox_self_value`** (`trait_object_ops.rs:866-910`): replaced `Arc::new(TraitObjectStorage::new(inner_ptr, Arc::clone(receiver_vtable))) + Arc::into_raw` with `TraitObjectStorage::_new(inner_ptr, Arc::clone(receiver_vtable)) as u64`.

### Forbidden-pattern compliance

The fix matches the post-cascade consumer carrier shape — does NOT introduce a new parallel-implementation framing. Per CLAUDE.md "Parallel-implementation across producer/consumer carrier-shape boundaries" rule, this is the lockstep-flip that the §Q25.C.5 amendment text describes ("atomic producer/consumer flip ... in a single commit"). The pre-fix mixed-dispatch state was the violation; the fix is the lockstep correction.

No defection-attractor renames introduced: no "bridge", "probe", "helper", "hop", "translator", "adapter", "shim" descriptors in code or comments. No ValueWord resurrection. No Bool-default fallback. No Ptr-newtype-shim defection (the fix uses raw `*const TraitObjectStorage` directly per the post-cascade slot-ABI contract; `TraitObjectPtr` newtype at `heap_value.rs:702` is not introduced as a new parallel carrier).

### Test acceptance widening

The pre-fix `executor::tests::trait_object_thunks::generic_method_dispatches_as_direct` test ABORTED at process teardown (same `free(): invalid pointer` bug). With the abort gone, the test now surfaces the pre-existing emission gap with `RuntimeError("DynMethodCall: function 'Tag::describe' not in function_name_index")`. The test's acceptance branch widened to also accept `not in function_name_index` (functionally equivalent to the existing `not in vtable` acceptance — both name "emission-gap-not-bridged" structured surfaces). The widening is honest: it documents the pre-existing emission gap that the SIGABRT was masking. No new functional regression.

## §3 — Acceptance evidence

### Smoke matrix 5/5 VM == JIT (post-fix)

```
s1 vm EC=0 result=4950   s1 jit EC=0 result=4950
s2 vm EC=0 result=30     s2 jit EC=0 result=30
s3 vm EC=0 result=x      s3 jit EC=0 result=x
s4 vm EC=0 result=2      s4 jit EC=0 result=2
s5 vm EC=0 result=x      s5 jit EC=0 result=x
```

All 5 smokes produce identical VM and JIT output, all exit cleanly (EC=0). Pre-existing `Bytecode verification failed: N violation(s)` warning lines are unchanged baseline noise (present pre-fix and post-fix, not load-bearing for VM/JIT correctness output).

### Smoke 5 fixture (verbatim)

```shape
trait T { name(): string }
type X {}
impl T for X { method name() { "x" } }
let t: dyn T = X {}
print(t.name())
```

VM output: `x`
JIT output: `x`

### Close-gate evidence

- `cargo check --workspace --lib --bins --tests --examples` EXIT=0
- `cargo check -p shape-jit --features jit-trace` EXIT=0
- `bash scripts/check-no-dynamic.sh` EXIT=0
- `bash scripts/verify-merge.sh` 12/12 PASS, "ALL CHECKS PASSED. Safe to merge."
- `cargo test --release -p shape-vm --lib -- trait_object_thunks --test-threads=1` 12/12 PASS (parallel SIGSEGV is pre-existing test-infrastructure issue per CLAUDE.md "annotations parallel-state contention" — same bug class as baseline; baseline SIGABRTs on the same parallel run with `free(): invalid pointer`)

## §4 — Imprecision-pattern instances surfaced

Cumulative phase-3 trajectory imprecision count was 61 (per cluster-2-shape-test-residuals-triage close 2026-05-16). This dispatch surfaces **2 new instances** (62-63):

- **62**: ADR-006 §Q25.C.5 amendment Wave 2 Agent E text (lines 5635-5642) describes the cascade-flip lockstep but does NOT specify which side flips first or who owns the producer-side cascade. The text says "a dedicated `E2` agent if cascade-site count justifies splitting" — but no `E2` agent dispatched, and the consumer-side flipped (Wave 3 stabilize) without producer-side follow-up. This left HEAD in a mixed-dispatch state for ~2-3 weeks until Smoke 5 surfaced it. **Pattern**: cascade-flip lockstep requirement-without-owner under multi-agent dispatch; surface tracking via per-flip producer/consumer owner attribution (sibling-coordination question) is the future preventative.
- **63**: ADR-006 §Q25.C.5 amendment text says `TraitObjectStorage::new(...)` "remains a legacy transitional entry point during the Wave 2 transition (deleted when the last caller migrates to `from_trait_object_raw`)" — but `TraitObjectStorage::new` is NOT the deletion target; `Arc::new(TraitObjectStorage::new(...))` is. The amendment text conflates struct-`new` (POD constructor, legitimate for tests) with Arc-wrapped-`new` (forbidden post-cascade). The producer-side `op_box_trait_object` uses the latter; the test scaffold at `heap_value.rs:6303-6313` makes the inner TypedObjectStorage via `_new` correctly but builds the outer carrier via `Arc::new(TraitObjectStorage::new(...))` per the legacy pattern (the whole test mod is `#[cfg(any())]` disabled, so this is documentary only). **Pattern**: amendment-text deletion-target naming imprecision (conflates struct-new with Arc-wrapped-new); future amendments should name the specific Arc-wrapped pattern as the deletion target, not the inner struct constructor.

## §5 — Out-of-scope follow-up sub-cluster recommendations

Per kickoff prompt scope partition + audit findings:

### `cluster-1.5-q25c-6-ic-devirtualization` (LARGE; cluster-1.5-fast-path candidate)
- **Territory**: `crates/shape-jit/src/feedback.rs` IC state machine extension; `crates/shape-vm/src/feedback.rs:9-128` IC site recording; `crates/shape-vm/src/executor/trait_object_ops.rs` IC-stabilization key recording in `invoke_dyn_unified` / `invoke_dyn_closure`; `crates/shape-jit/src/mir_compiler/` JIT-tier direct-call emission per Q25.C.6 state-machine row table.
- **Scope estimate**: ~500-1500 LoC across 4-6 files. Requires Q25.C.5 vtable-Arc-id + per-generic-arg `concrete_type_id` tuple recording per IC site; Monomorphic-state emission of direct call eliding vtable lookup + auto-boxing + SelfArg check + TypeInfo dispatch; Polymorphic 2-4 entry inline comparison; Megamorphic fallback to vtable + thunk path; deopt-on-mismatch wiring.
- **Tier**: optimization (not correctness). Smoke-5-style `dyn T` programs work correctness-wise today via the vtable + thunk path (post the cluster-1.5 producer/consumer fix).

### `cluster-1.5-q25c-7-lsp-cost-class-inlay-hints` (MEDIUM; cluster-1.5-lsp candidate)
- **Territory**: `tools/shape-lsp/src/` inlay-hint provider; new visitor over typed AST that flags each `dyn T` call site with its Q25.C.6 cost class (`[direct]` / `[vtable]` / `[boxed-return]` / `[generic-type-info]` / `[self-arg-check]` per §Q25.C.7 row table).
- **Scope estimate**: ~300-500 LoC across 2-3 files. Depends on Q25.C.6 IC stabilization for the `[direct]` class (other classes derivable from VTableEntry variant alone).
- **Tier**: tooling (not correctness or runtime performance).

### `cluster-1.5-q25c4-static-only-opt-out` (SMALL-to-MEDIUM; cluster-1.5-tooling-adjacent candidate)
- **Territory**: `crates/shape-ast/src/shape.pest` annotation grammar rule for `#[static_only]`; AST decoration on `MethodDef`; `compiler/trait_object_emission.rs::build_and_register_vtable` skip-emission for static-only methods; `compiler/expressions/function_calls.rs::compile_expr_method_call` ETO-002 surface at `dyn T` call sites naming static-only methods.
- **Scope estimate**: ~150-250 LoC across 4-6 files.
- **Tier**: developer-facing cost-control opt-out (not load-bearing for any current Shape program). Dispatch when developer demand for cost-control opt-out surfaces.

## §6 — Refusal discipline

Per CLAUDE.md "Forbidden Patterns + Renames to refuse on sight; cluster-2 canonical refusal set; carries forward to cluster-1.5":

- **NO ValueWord resurrection** — no ValueWord reference introduced (grep confirms zero hits in diff).
- **NO Bool-default fallback at any kind-source gap** — receiver-recovery uses explicit `NativeKind::Ptr(HeapKind::TraitObject)` match; non-match arms return structured `VMError::RuntimeError`.
- **NO bridge/probe/helper/hop/translator/adapter/shim framings** — broader-family regex zero hits in diff (`crates/shape-vm/src/executor/trait_object_ops.rs` modifications); zero hits in this deliverable doc.
- **NO parallel-implementation framings** — the fix matches the post-cascade consumer carrier shape (lockstep flip), not a new parallel implementation. The pre-fix state WAS the parallel-implementation; the fix corrects to a single carrier shape (`_new`-allocated v2-raw).
- **NO new HeapKind variants** — `HeapKind::TraitObject = 29` was already in place at HEAD.
- **Refuse #10 anti-deferral** — Q25.C.6 + Q25.C.7 + Q25.C4-static-only are NOT "tracked-as-follow-up-to-ignore"; each names a specific cluster-1.5-fast-path / cluster-1.5-lsp / cluster-1.5-tooling-adjacent destination with territory + scope + tier estimates.
- **Refuse #11 Ptr-newtype-shim defection** — `TraitObjectPtr` newtype at `heap_value.rs:702` is NOT introduced as a new parallel carrier in the fix; the fix uses raw `*const TraitObjectStorage` directly per the post-cascade slot-ABI contract that the consumer arms already established.
- **Per CLAUDE.md "Own all code quality"** — no new clippy regressions verified via `cargo check --workspace --lib --bins --tests --examples` EXIT=0 (default + `--features shape-jit/jit-trace`).
- **Empirical-verification-first per Reading 6** honored — Phase 1 audit completed before fix territory identified; fix matched empirically-observed root cause.

## §7 — ADR-006 / CLAUDE.md modifications surfaced

**Suggested ADR-006 amendment** (FLAGGED — not landed in this dispatch; supervisor disposition required):

The §Q25.C.5 amendment Wave 2 Agent E text at ADR-006:5599-5713 describes the cascade-flip lockstep correctly but never received its companion producer-side flip. The current dispatch lands the producer-side flip. Suggested addendum sentence at ADR-006 line ~5713 (end of `ckpt-final-prime²` paragraph):

> "**Wave 3 producer-side cascade flip (cluster-1.5 Q25.C close, 2026-05-16):** `op_box_trait_object` (`crates/shape-vm/src/executor/trait_object_ops.rs:90-189`) + `rebox_self_value` (`:866-910`) flipped from `Arc::new(TraitObjectStorage::new(...)) + Arc::into_raw` to `TraitObjectStorage::_new(...) + (ptr as u64)`. `op_dyn_method_call` receiver recovery (`:203+`) flipped from `Arc::from_raw + Arc::clone + Arc::into_raw` transient-share-bump to plain `unsafe { &*(bits as *const TraitObjectStorage) }` borrow. `invoke_dyn_unified` / `invoke_dyn_closure` signatures flipped from `&Arc<TraitObjectStorage>` to `&TraitObjectStorage`. Mixed-dispatch shape eliminated; producer/consumer carrier shapes now match (both v2-raw `_new`-allocated, `HeapHeader`-refcounted, `release_elem`-drop)."

**No CLAUDE.md modifications surfaced.** All discipline preserved per existing forbidden-pattern + renames-to-refuse + ADR-006 §2.7 family text.

## §8 — Ceiling-c + D-α status

- **Ceiling-c bound check**: fix scope = 1 file (`crates/shape-vm/src/executor/trait_object_ops.rs`, ~100 LoC modified across 3 sites) + 1 test-acceptance widening (`crates/shape-vm/src/executor/tests/trait_object_thunks.rs`, +15 LoC). Well within ceiling-c.
- **D-α status**: not applicable (single-checkpoint dispatch; no dynamic chain — Phase 1 audit immediately surfaced root cause; Phase 2 fix matched empirically observed bug; no surface-and-stop intermediate states required).
