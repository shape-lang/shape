{
  "summary": "U4 structural-research: map the L3 type split-brain (fallback engine + 4 type reps + ~13 side-tables) vs HEAD; synthesize a deletion-shaped roadmap",
  "agentCount": 6,
  "logs": [
    "Enumeration complete; synthesizing U4 deletion-shaped roadmap"
  ],
  "result": {
    "roadmap": "The live U4 bug (`f8`: closure body is a field-read → `unknown`) is confirmed reproducing exactly as Report D describes. All load-bearing claims verified. I have everything needed to synthesize the roadmap. The one contradiction across reports I'll flag: the STAGE-F1 line citations (`constraints.rs:1108`/`:1137`) are wrong in all four reports — the real site is `inference/access.rs` field-access arms.

---

# U4 Unification Roadmap — Collapse L3 Type Layer to ONE Source of Truth

**Root R2.** Verified against worktree `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch` @ HEAD `18991310`. All sites below independently confirmed (not just transcribed from the reports).

**Central finding (all 5 reports agree):** the convergence target — the engine span-keyed `expr_type_table: HashMap<Span, Type>` — **already exists and is already consulted first**. It landed in the 2026-06-22 "T1 KEYSTONE" commit. U4 is therefore **NOT** "build the span-table." U4 is the **deletion half**: the keystone was layered *on top of* the old split-brain "as a fallback," which is the exact W-series walk-back shape CLAUDE.md forbids. U4 finishes the deletion.

---

## 1. CANONICAL SOURCE OF TRUTH

**What survives — exactly two carriers, one derivation:**

1. **Inference authority (L3):** the engine span-keyed table `expr_type_table: HashMap<Span, Type>` (`crates/shape-runtime/src/type_system/inference/mod.rs:268`), finalized post-solve by `finalize_expr_type_table` (`mod.rs:361` — drops any entry still carrying a free `Type::Variable`, **no Unknown-default**), handed to the compiler via `take_expr_type_table` (`mod.rs:352`) into `resolved_expr_types: HashMap<Span, Type>` (`compiler/mod.rs:891`). This is the **single L3 inference source**. `Type` (`types/core.rs:93`) is the canonical type. Confirmed: U1 already collapsed `Type::Generic` to one encoding + one equality.

2. **Emit structural carrier:** `ConcreteType` (`shape-value/src/v2/concrete_type.rs:84`) survives **only downstream of inference** (monomorphizer, bytecode emit, FrameDescriptor), always **derived from** the one resolved `Type`, never as a parallel inference source.

**The single emit derivation:**
```
engine Type  →  ConcreteType  →  native_kind_from_concrete_type()  →  NativeKind  →  (opcode)
```
`native_kind_from_concrete_type` (`shape-value/src/v2/closure_layout.rs:944`) is the **one** `ConcreteType→NativeKind` map. `NumericType` survives **only** as an emit-time enum *derived from the one Type* at the opcode-selection point (via `inferred_type_to_numeric`, `numeric_ops.rs:78`) — never as a standalone register.

**The invariant U4 establishes:**

> A span-table MISS is a **surface-and-stop compile error**, never a re-derivation. There is no fallback engine, no patch-ladder arm, no side-table, and no closure mini-inferencer that "recovers" a type the table lacks. If `resolved_expr_types[span]` has no fully-resolved entry, the operand is genuinely un-inferable and the strict-typing checker rejects it loudly.

This is the U4 analogue of U1's "one equality" and U3's "one carrier": **one inference answer per expression span; everything else is a deletion target.**

---

## 2. DELETION TARGETS, ORDERED

Ratings: blast-radius (readers to re-point) × difficulty.

### T0 — The fallback engine `self.type_inference.infer_expr(expr)` — `expressions/mod.rs:2187` — **KEYSTONE DELETION, HIGH**
The terminal arm of `infer_expr_type`. Confirmed at `:2187`. It is **Engine B** — a *persistent struct field* `TypeInferenceEngine::new()` that is **never given `infer_program/infer_function`** (verified: zero `type_inference.infer_program*` hits), so its env has only type *definitions*, never function-body locals. On any in-body local/param it returns `UndefinedVariable` → fresh var → `unknown`. It is a stale, module-scope-only re-derivation of what **Engine A** (the transient full-program walk in `infer_reference_model`) already proved and recorded. **Deletion replaces it with surface-and-stop.** Re-point first: nothing reads its *result* except this one arm; but its preconditions (§3) must be met first.

### T1 — `type_name: Option<String>` stringly rep + re-parse sites — `type_tracking.rs:340` — **HIGH**
The `VariableTypeInfo.type_name` field (+ siblings `concrete_numeric_type: Option<String>` `:355`, `function_return_types: HashMap<String,String>` `:668`). Readers to re-point at a span-table / `ConcreteType` query:
- `tracker_type_name_for_identifier` (`expressions/mod.rs:2245`) — ladder arms #2.
- `tracked_array_element_type` (`expressions/mod.rs:2208`) — **both re-parse sites die here**: `.strip_suffix("[]")` (`:1632`) and `.strip_prefix("Array<")/"Vec<"` (`:2229-2234`), confirmed.
- The producing projection `tracked_type_name_from_annotation` (`helpers.rs:1881-1888`) that renders `TypeAnnotation::Array` → `"Vec<...>"` — the round-trip's *write* side. Deleting both sides closes the round-trip.
- `numeric_operand_proof_gap` (`binary_ops.rs:366`), schema-id lookups (`mod.rs:2267`).
~62 `.type_name` reads + ~65 writers + ~70 set-sites. **Blast-radius: HIGH.**

### T2 — `NumericType` standalone register `last_expr_numeric_type` — `compiler/mod.rs:726` — **HIGH (writers), MEDIUM (semantics)**
Keep the `NumericType` *enum* (`type_tracking.rs:54`) as the emit-time opcode index; **delete the standalone per-expression mutable register** (115 write sites, 65 read sites — confirmed counts). Re-point: derive `NumericType` from the one `Type` at the opcode-selection point only, via `inferred_type_to_numeric` (`numeric_ops.rs:78`), which becomes the **sole** Type→NumericType derivation (no longer a competing source). Readers: `typed_opcode_for` (`numeric_ops.rs:282`), the binop writeback (`binary_ops.rs:796`), `last_expr_numeric_type_to_storage_hint` (`helpers_binding.rs:549` — survives as the sole emit stamp, re-sourced from the one Type), for-loop element kind (`loops.rs:1120-1273`). The proof-gap guard `numeric_operand_proof_gap` (`binary_ops.rs:347`) exists *because* the register and the table disagree — it dissolves when there is one source.

### T3 — The closure-body mini-inferencer `expr_type` — `closures.rs:936-1131` — **MEDIUM, this is the LIVE BUG's root**
A **fourth, stringly inference engine** (`Option<String>` of `"int"`/`"Vec<int>"`, own `strip_prefix("Vec<")` re-parse at `:1104`). Confirmed: its last arm is `_ => None` at `:1130` — **no PropertyAccess arm** → closure body `p.salary` erases to `None`. This is the single live U4 failure class (§6). Delete entirely; re-point the closure-return type to the engine span-table (the engine already walks closure bodies — the fix is that `finalize_expr_type_table` currently *drops* the still-free closure-field entry; §3/§7).

### T4 — The ~22-arm patch ladder in `infer_expr_type` — `expressions/mod.rs:1558-2185` — collapses as T1/T2/side-tables go
Once T1/T2 + the side-tables are gone and PropertyAccess is consulted, the ladder collapses to: **keystone consult → (if needed) ConcreteType projection for opcode-stamp → surface-and-stop**. Arms #13/#14 (PropertyAccess field-read re-derivation) die with the §3 exclusion removal.

### T5 — The ~16 hint side-tables (Report C: GROWN from 13, not shrunk)

**None are dead post-U1/U3** — confirmed. U3 deleted the *runtime* `TypedMap` carrier, NOT these compiler-side inference hints. Map tables survived.

| Tier | Tables | Readers to re-point | Rating |
|---|---|---|---|
| **Tier 1 — nearly dead, delete first** | `array_element_types` (`mod.rs:1237`, 1W+1R both in `v2_map_emission.rs`); `inferred_param_concrete_types` (`:1383`, 1R); `inferred_param_fn_param_types` (`:1406`, 1R); `inferred_param_object_fields` / `inferred_return_object_fields` (1R each); `binding_object_element_fields` (`:1254`, 1R `loops.rs:1813`) | each → per-slot engine query at the binding's defining-expr span | **LOW** |
| **Tier 1b — soundness-coupled** | `binding_collection_carrier_kinds` (`:1724`, 1W+1R) | feeds §2.7.8 capture-kind at `type_resolution.rs:1605` — migrate to engine-Type-derived capture kind, do NOT just delete | **LOW count, MEDIUM care** |
| **Tier 2 — single-purpose in a ladder** | `local_array_element_types`/`module_binding_array_element_types` (`:1240/:1243`, + the lockstep-upgrade hack `helpers.rs:4135-4160`); `map_key_value_types`/`local_map_key_value_types`/`module_binding_map_key_value_types` (`:1222/:1228/:1232` — **survived U3, must delete**); `function_return_schema_ids` (`:1444`, 4 narrow `f().field` readers); `local_array_callable_return_types`/`module_binding_array_callable_return_types` (`:784/:788`, `__call__` arm) | Ladder-1 (`type_resolution.rs:2675`) + Ladder-2 (`infer_expr_type`) | **MEDIUM** |
| **Tier 3 — load-bearing, delete LAST** | `current_function_local_concrete_types`/`module_binding_concrete_types` (`:1684/:1710`, head of Ladder-1, 4 reader paths each, 6 writers, snapshot/restore in `monomorphization/cache.rs`); `inferred_param_type_hints` (`:1371`, stamps `type_name`, SB-7 keystone); `function_return_types` (`type_tracking.rs:668`)/`local_callable_return_types`/`module_binding_callable_return_types` (`:771/:775`, read by strict-typing binop dispatch) | the strict binop dispatch + the whole-binding ConcreteType ladder head | **HIGH** |

**Mark dead-post-U1/U3:** none. The reports converge that U1/U3 *added* projection tables (the closure-body peeks `local_callable_closure_bodies` `:811`, `binding_collection_carrier_kinds`, the pass-modes pair `:753/:760`) rather than retiring any. **Audit's `inferred_return_type_hints` is NOT a struct field** — it is a transient local folded into the stringly tracker `function_return_types`; treat it as part of T1's stringly class.

### T6 — Reconcile the TWO divergent `ConcreteType→NativeKind` maps — **MEDIUM, a HARD PREREQUISITE for any prove_native_kind wiring**
Confirmed they disagree on **four arms**:

| ConcreteType | closure_layout.rs:944 (total) | mir_compiler/types.rs:151 (Option) |
|---|---|---|
| `Option(_)` | `Ptr(TypedObject)` | `Ptr(Option)` |
| `Result(_,_)` | `Ptr(TypedObject)` | `Ptr(Result)` |
| `Pointer(_)` | `Ptr(NativeView)` | `UInt64` |
| `Void` | **panics** | `None` |

The proof gate would project through the closure_layout copy; the JIT proves against its own copy. Until these agree, VM and JIT prove different kinds. **This must land before prove_native_kind is wired (U2).**

---

## 3. THE PREREQUISITE — make the engine export EVERY expression BEFORE deleting the fallback

The fallback (T0) cannot be deleted until a table MISS genuinely means "un-inferable." Four preconditions, from Report A §6 + Report D §6:

**P1 — PropertyAccess consult exclusion must be removed (`expressions/mod.rs:1558`), but STAGE-F1 must keep surfacing.** The engine **already records** PropertyAccess (`infer_expr` at `expressions.rs:172` is exhaustive — verified no top-level `_ =>` in `infer_expr_inner`). The *compiler consumer* deliberately skips the table for PropertyAccess to avoid masking the STAGE-F1 error. STAGE-F1 is **already a real engine error** — `TypeError::ConstraintViolation` in the field-access constraint arms of `crates/shape-runtime/src/type_system/inference/access.rs` (`:821/:884/:920/:1142`). **⚠️ CONTRADICTION FLAG:** all four reports (A, B, C, D) cite this as `constraints.rs:1108` / `:1137` — **that file/line is wrong**; the actual site is `inference/access.rs`. Implementers must use `access.rs`. The deletion shape: make STAGE-F1 the *sole* gate of field-read strictness, then drop the consult exclusion so resolvable field reads hit the table and ladder arms #13/#14 disappear.

**P2 — Engine must export closure-body field-reads (the live bug).** The engine walks closure bodies, but `finalize_expr_type_table` (`mod.rs:361`) **drops** the closure-field call-result entry because it stays a free `Type::Variable` post-solve for the `|p: Emp| { p.salary }` shape. **Expression kinds needing reliable engine export added/fixed (from Report A §3.2 + Report D §3):**
- `PropertyAccess` *inside closure bodies* — currently dropped as free var (the `f8`/`h1`/`h2`/`h4b` root). The engine must bind the closure param's field projection to a concrete type at that span so the entry survives finalization.
- The genuinely-un-inferable tail that **should stay absent → surface-and-stop** (do NOT force these to resolve): `TableRows` (always fresh), `QualifiedFunctionCall` to a real module fn (deliberately fresh, `expressions.rs:446`), `SimulationCall`, `Comptime`/`ComptimeFor`, `Join`, empty `Array` with un-pinned element, `Range` over non-numeric, `Let`-with-no-init.

**P3 — Reference-typed entries must be projected at record (or consult) time.** The keystone currently skips `Borrow`-typed hits (`:1571-1575`, confirmed) so GapA `&T→T` projection runs in the ladder. To delete the fallback cleanly, record/serve the *projected referent* so reference reads hit the table.

**P4 — Module-scope top-level exprs must be recorded by Engine A, and span-collision safety.** Engine A walks module scope, so this is recording-completeness, not re-derivation. Synthetic/desugared nodes with **dummy spans** are never in the table (`expressions.rs:171`, `mod.rs:342`) → they'd become permanent misses → must be guaranteed genuine surface-and-stop or given real spans. **This is a correctness precondition for "miss == error."** The `self.type_inference.env.*` *lookup* uses (trait dispatch, enum/alias resolution at `binary_ops.rs:1553`, `helpers.rs:2002`, `type_ops.rs:107`) are independent of `infer_expr` and **survive** fallback deletion — only the `infer_expr(expr)` call at `:2187` is the target.

---

## 4. WAVE DECOMPOSITION

Each wave is one deletion with a green `just check-clean` + `just test-fast` gate, in dependency order, sized like U1/U3 stages.

### Wave U4-0 — Engine export completeness (PREREQUISITE, no deletion yet)
- **Territory:** `crates/shape-runtime/src/type_system/inference/` (`expressions.rs`, `access.rs`, `mod.rs`).
- **Does:** add NOTHING to the compiler. Makes the engine bind closure-body PropertyAccess to concrete types (P2), project Borrow→referent at record time (P3), and audit `finalize_expr_type_table` so legitimate field-read/closure-return entries survive while the un-inferable tail (P2 list) stays dropped.
- **Re-point first:** n/a (additive engine work).
- **Isolation test:** engine-standalone span-table completeness assertion (§5) over the `f8`/`h1`/`h2`/`h4b` ASTs — `resolved_expr_type(span)` returns `int` for the closure-call sites; returns `None` for the genuinely-un-inferable tail.
- **Surface-and-stop installed:** none yet (the fallback still exists; this wave just makes the table complete enough that the next wave can delete it).

### Wave U4-1 — Remove PropertyAccess consult exclusion
- **Territory:** `compiler/expressions/mod.rs:1558` + ladder arms #13/#14 (`:1905-1963`).
- **Deletes:** the `if !matches!(expr, Expr::PropertyAccess { .. })` guard; arms #13/#14.
- **Re-point first:** confirm STAGE-F1 (`access.rs`) fires for every un-annotatable field read (P1). Confirm resolvable field reads now hit the table (U4-0 guarantees it).
- **Isolation test:** `f2`/`f4`/`f5`/`f3c` stay green (resolvable field reads); `f1` (unannotated empty `[]` push) stays a **deliberate error** (STAGE-F1 not masked).
- **Surface-and-stop:** STAGE-F1 becomes the sole field-read strictness gate.

### Wave U4-2 — Delete the closure mini-inferencer (T3)
- **Territory:** `closures.rs:830-1131` (`infer_closure_body_return_type_name_with_caller_context` + `expr_type`).
- **Deletes:** the fourth stringly inference engine.
- **Re-point first:** closure-return type now read from `resolved_expr_types` at the closure-body span (enabled by U4-0/P2). The callable-return side-tables (E1-E4, `local_callable_return_types` etc.) become engine-served — staged for deletion in U4-6.
- **Isolation test:** `f8`/`h1`/`h2`/`h4b` go **red→green**; `c1`/`g1` stay green.
- **Surface-and-stop:** a closure body whose type the engine genuinely can't resolve → the call site MISSes the table → strict binop error (correct).

### Wave U4-3 — Delete the fallback engine (T0)
- **Territory:** `expressions/mod.rs:2187`.
- **Deletes:** `self.type_inference.infer_expr(expr)` terminal arm → replace with `Err(surface-and-stop)`.
- **Re-point first:** U4-0/U4-1/U4-2 complete; P4 span-collision audit done (dummy-span nodes are genuine errors).
- **Isolation test:** full `just test-fast` + the entire §6 regression set; the deliberate-error anchors (`f1`, `c5`) stay errors.
- **Surface-and-stop:** **THE keystone invariant** — a table miss is now a compile error. Engine B is gone.

### Wave U4-4 — Delete `NumericType` standalone register (T2)
- **Territory:** `compiler/mod.rs:726`, `binary_ops.rs`, `numeric_ops.rs`, `loops.rs`, `helpers_binding.rs`.
- **Deletes:** `last_expr_numeric_type` register (115 writers); the `numeric_operand_proof_gap` ad-hoc guard.
- **Re-point first:** derive `NumericType` at opcode-selection via `inferred_type_to_numeric(resolved Type)`; `last_expr_numeric_type_to_storage_hint` re-sourced from the one Type.
- **Isolation test:** numeric opcode-selection golden set (int vs number vs decimal vs width-typed arithmetic emits the same opcodes as before); `c1`/`c1n` (int/number through closures).
- **Surface-and-stop:** no separate register to drift; the single-derivation agreement test (§5) must pass.

### Wave U4-5 — Delete `type_name: Option<String>` stringly rep + re-parse sites (T1)
- **Territory:** `type_tracking.rs:340/355/668`, `expressions/mod.rs:1632/2208/2229`, `helpers.rs:1881`.
- **Deletes:** `type_name`/`concrete_numeric_type` string fields; `function_return_types` string map; both `.strip_*` re-parse sites; the `tracked_type_name_from_annotation` `"Vec<...>"` projection.
- **Re-point first:** all `.type_name` readers → `ConcreteType`/engine-Type query.
- **Isolation test:** array-element-typed programs (`g2`, `f2`) — element type now flows structurally, no string round-trip.
- **Surface-and-stop:** no string boundary to lose structure across.

### Wave U4-6 — Delete the side-tables, Tier 1 → Tier 2 → Tier 3 (T5)
Staged in three sub-waves matching the difficulty tiers (each its own green gate):
- **U4-6a (Tier 1):** `array_element_types`, `inferred_param_*` (concrete/fn_param/object_fields), `inferred_return_object_fields`, `binding_object_element_fields`, `binding_collection_carrier_kinds` (migrate capture-kind to engine-Type-derived).
- **U4-6b (Tier 2):** array-element + map-kv tables (delete the lockstep-upgrade hack `helpers.rs:4135-4160`), `function_return_schema_ids`, array-callable tables.
- **U4-6c (Tier 3):** `current_function_local_concrete_types`/`module_binding_concrete_types` + snapshot/restore plumbing; `inferred_param_type_hints`; `function_return_types`/callable-return tables.
- **Re-point first (all):** each table → `resolved_expr_types[binding-defining-expr.span]`.
- **Isolation test:** the two consult ladders (`identifier_concrete_type` `type_resolution.rs:2675`; `infer_expr_type`) collapse to a single span-table lookup with no disagreement (the U4-0 completeness test guarantees no MISS for these slots).
- **Surface-and-stop:** Ladder-1 and Ladder-2 no longer exist as multi-step fallbacks.

### Wave U4-7 — Reconcile the two `ConcreteType→NativeKind` maps (T6)
- **Territory:** `closure_layout.rs:944`, `mir_compiler/types.rs:151`.
- **Deletes:** one of the two copies; the four divergent arms reconciled to one canonical answer (requires a decision — §7).
- **Isolation test:** the single-derivation agreement test (§5) — VM emit and JIT emit project identical NativeKind for Option/Result/Pointer/Void.
- **Surface-and-stop:** one map; `prove_native_kind` now has a single trustworthy projection to assert against (enabling U2, **separable**).

---

## 5. ISOLATION-TEST PLAN

**(A) Engine standalone — span-table completeness (no MISS).** New `#[cfg(test)]` in `inference/`: parse a program, run `infer_program_best_effort`, `finalize_expr_type_table`, then assert `resolved_expr_type(span)` is `Some(fully-resolved)` for **every** expr span the strict binop checker would query (every operand of every `BinaryOp`, every `FunctionCall`/`MethodCall` result feeding an arithmetic/comparison/concat site), AND `None` for the deliberate un-inferable tail (P2 list) + the STAGE-F1 reject (`f1`). Drive it with the §6 corpus. This is the precondition gate for U4-3.

**(B) Single Type→kind derivation agreement (the U4 analogue of U1's one-equality test).** New cross-crate test: for a representative `ConcreteType` set covering all arms (esp. Option/Result/Pointer/Void/widths), assert `closure_layout::native_kind_from_concrete_type(ct)` and `mir_compiler::types::native_kind_from_concrete_type(ct)` **agree** (after U4-7: that there is only one). Plus: for each emit site (binop terminal, return stamp, store/load, v2 Ptr-stamp), assert the stamped NativeKind equals `native_kind_from_concrete_type(concrete_from(resolved Type))` — i.e. **no opcode-inspection recovery** (`last_emitted_native_kind`, `helpers.rs:2209`) disagrees with the projected kind. This replaces "recover kind from bytecode and hope it agrees" with "project from proven Type and assert."

**(C) Ladder-collapse property test.** After U4-6: assert `identifier_concrete_type` and `infer_expr_type` return answers that agree with `resolved_expr_types[span]` for every binding in the corpus — proving the two ladders are now one lookup.

---

## 6. REGRESSION SET (the U4 acceptance gate)

**MUST go red→green (the live U4 bug — Class 2+5, closure-body field-read):**
| File | Shape | Currently |
|---|---|---|
| `f8` | `let get = \\|p: Emp\\| { p.salary }; get(e) + 1` | ❌ `unknown + int` (re-confirmed live at HEAD) |
| `f8let` | `let s = get(e); s + 1` | ❌ |
| `h1` | `\\|w: Outer\\| { w.inner.x }; getx(o) + 1` (nested) | ❌ |
| `h2` | `\\|p: Emp\\| { return p.salary }; get(e) + 1` (explicit return) | ❌ |
| `h4b` | `get(a) == get(b)` (both closure-field, no sibling-literal recovery) | ❌ `unknown == unknown` |

**MUST stay green (regression guards — the working paths the deletion must not break):**
`f2` (`rs[0].n + 1`→4), `f4` (`make().v + 1`→8), `f5` (`o.inner.x + 1`→6), `f3c` (`self.count + 1` in trait method→11), `g2` (`.map(\\|p\\| p.x)` then `xs[0]+xs[1]`→4), `c6` (`.map(\\|e\\| e.salary)` then `for v in sals`→200), `h5` (named `fn get(p: Emp) -> int { p.salary }`→51), `c1`/`c1n` (int/number through closures→24/24.0), `c3inline`/`c3let`→10, `c4` (`s.toUpperCase()+"!"`→`HELLO!`).

**MUST stay a deliberate error (STAGE-F1 / carrier-gap — deletion must NOT mask these):**
- `f1` — `let mut rs = []; rs = rs.push(Run{..}); for r in rs { r.n + 1 }` → must error "annotate `let rs: Array<Run> = []`" via the engine STAGE-F1 (`access.rs`). **The whole point of P1.**
- `c5` — array-of-closures `arr[0](1)+arr[1](1)` → deliberate refusal "Arrays of function values not yet supported" (a carrier gap, **not** type-erasure; reports correct the stale audit that listed it as a live R2 repro).

---

## 7. RISKS / OPEN QUESTIONS — surface, do not paper over

1. **`prove_native_kind` wiring: in-scope or separable? (NEEDS DECISION.)** Confirmed: `prove_native_kind` (`type_tracking.rs:1261`) is a **real** exact-equality check but has **ZERO production callers** (all 8 callers are `#[cfg(test)]`). The one production proof-gate that fires (`numeric_operand_proof_gap` → `proof_gap_unresolved_operand`, `binary_ops.rs:383`) **bypasses** it. **Recommendation:** wiring it is **U2, separable** — but the *enabling* work is U4. It cannot be wired meaningfully before U4 (you'd be checking one re-derivation against another). The doc-comment at `helpers_binding.rs:468-469` already **lies** that the return kind "comes from `prove_native_kind`" — it never does. Decide: does U4 fix the lying comment in passing, or leave it for U2?

2. **The two `ConcreteType→NativeKind` maps disagree on Option/Result/Pointer/Void (NEEDS DECISION on canonical answer).** This is a genuine VM/JIT correctness drift, not a clone. Which is canonical — `Ptr(TypedObject)` (VM/closure_layout) or `Ptr(Option)`/`Ptr(Result)` (JIT)? `Void`: panic or `None`? `Pointer`: `Ptr(NativeView)` or `UInt64`? This must be resolved (U4-7) before any proof gate, and it is **outside the L3 inference layer** — it may belong to a JIT-owner decision. **Surface to supervisor.**

3. **Citation contradiction across all four reports: STAGE-F1 is NOT at `constraints.rs:1108`/`:1137`.** Verified: the real site is the field-access constraint arms in `crates/shape-runtime/src/type_system/inference/access.rs` (`:821/:884/:920/:1142`). The file `crates/shape-runtime/src/type_system/inference/constraints.rs` **does not exist**. Implementers of U4-1 must not waste time at the cited line.

4. **Genuine missing engine capability (the P2 blocker, BLOCKS U4-2/U4-3):** the engine *records* closure-body PropertyAccess but `finalize_expr_type_table` *drops* it because the call-result var stays free post-solve for `|p: Emp| { p.salary }`. Is making the engine bind the closure param's field projection at module-scope-walk time a **bounded inference fix**, or does it need a deeper change to how closure bodies are solved against their declared param types? This is the one place where deletion is **blocked by a real missing engine capability**, not just plumbing. **U4-0 must prove this is tractable before committing to the U4-2/U4-3 deletions** — if the engine genuinely cannot resolve the closure-field return without the side-table, that is a surface-to-user moment, not a rationalized fallback retention.

5. **Span-collision / dummy-span safety (P4):** desugared/synthetic nodes carry dummy spans and are never in the table. Converting "miss == error" requires auditing that no legitimate desugared expr the strict checker queries lands on a dummy span. If some do, they either need real spans or the desugarer must record their type — surface any found before U4-3.

6. **Snapshot/restore plumbing for Tier-3 ConcreteType tables** (`monomorphization/cache.rs:432/438/623/626/840/843`) — deleting `current_function_local_concrete_types`/`module_binding_concrete_types` (U4-6c) requires unwinding monomorphization save/restore. Highest-blast-radius deletion; keep it last and isolated.

**No contradictions between the five reports on the core thesis** — all five agree the span-table is the already-built target, the fallback is the deletion keystone, the four reps are intact (Reports B/E note it's closer to **six** with FieldKind + StorageType), and the side-tables grew rather than shrank. The only cross-report error is the shared wrong STAGE-F1 citation (#3 above).",
    "reports": {
      "aEngine": "I have complete, verified material. Here is the report.

---

# U4 Facet Report: The Fallback Engine + Keystone Export Completeness

**Worktree:** `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch` @ HEAD `18991310`
**Verified against HEAD.** Audit line numbers are substantially stale (the codebase moved through U1, U3, the L5 Miri pass, AND a major **post-audit T1-keystone landing on 2026-06-22** that the audit predates entirely). Corrections are flagged inline.

---

## 0. HEADLINE: the audit's central premise is PARTIALLY ALREADY-IMPLEMENTED

The audit treats "the span-keyed inference table" as the *target to build*. **It already exists at HEAD.** A 2026-06-22 commit ("T1 KEYSTONE") landed exactly the structure U4 asks for:

- The engine records every expression's resolved type into `expr_type_table: HashMap<Span, Type>` (`inference/mod.rs:268`).
- It is finalized post-solve (free vars dropped, no Unknown-default) by `finalize_expr_type_table` (`inference/mod.rs:361`).
- The compiler harvests it into `resolved_expr_types` (`compiler/mod.rs:891`) and **consults it FIRST** in `infer_expr_type` (`expressions/mod.rs:1558-1578`).

So U4 is **NOT** "build the span-table." U4 is now: **(a) delete the fallback engine, (b) delete the ~26-arm patch ladder that the keystone has made largely redundant, (c) remove the PropertyAccess exclusion, (d) collapse the FOUR type representations.** The keystone is in place but is layered ON TOP OF the old split-brain rather than replacing it — classic "kept as fallback" shape that CLAUDE.md Forbidden Patterns warns against. This is the deletion that U4 must finish.

---

## 1. THE FALLBACK ENGINE ANATOMY

### 1.1 Location (audit was right, line shifted)
- **Fallback call:** `self.type_inference.infer_expr(expr)` at **`crates/shape-vm/src/compiler/expressions/mod.rs:2187`** (audit said `:2188` — off by 1; it is the FINAL statement of `infer_expr_type`, after the entire ladder). It maps any engine error into `ShapeError::SemanticError { "Type inference failed: {:?}" }`.

### 1.2 What the fallback engine IS — and the critical split-brain finding
There are **TWO distinct `TypeInferenceEngine` instances** in play, and this is the heart of the bug:

| | Engine A (the keystone source) | Engine B (the fallback) |
|---|---|---|
| Identity | `inference` — a **transient local** in `Self::infer_reference_model` (`compiler_impl_reference_model.rs:814`) | `self.type_inference` — a **persistent struct field** (`compiler/mod.rs:877`, init `compiler_impl_initialization.rs:85`) |
| What it walks | The **FULL program** via `infer_program_best_effort` (`compiler_impl_reference_model.rs:815`), including **every function body** | **Nothing.** It is `TypeInferenceEngine::new()` and is **never** given `infer_program/infer_item/infer_function` (verified: `grep type_inference.infer_program*` returns ZERO hits) |
| Its env | Fully populated by the program walk | Only ever receives **type DEFINITIONS** (traits/enums/structs/aliases/impls) via incremental `self.type_inference.env.define_*`/`register_trait_impl` calls in `statements.rs`. **Never** value bindings for locals/params (verified: no `define_variable`/`insert`/`bind` calls on `type_inference.env`) |
| Output consumed | `take_expr_type_table()` → `self.resolved_expr_types` (the span-table) | A single `infer_expr(expr)` call at the END of `infer_expr_type` |

**Consequence:** When the fallback `self.type_inference.infer_expr(expr)` runs on an `Expr::Identifier(local)`, `Expr::IndexAccess{Identifier(param)}`, `Expr::MethodCall{...}` inside a function body, **its env has no binding for that local/param** → `env.lookup` misses → engine returns `UndefinedVariable` (`expressions.rs:245-248`) → falls to a fresh type var → resolves to `unknown`. **This is exactly the "empty fallback engine" the audit names.** It is a *module-scope-only* re-derivation that structurally cannot see what Engine A already proved.

### 1.3 Is it genuinely a re-derivation of what the engine already knows?
**Yes, unambiguously.** Engine A *already computed* the type of `expr` while walking the function body (`infer_item → infer_function → infer_expr`), and recorded it in the span-table. The fallback (Engine B) tries to recompute the same type from an empty module-scope env and **gets it wrong (`unknown`) for precisely the function-body-local cases that matter.** The keystone consult at the TOP of `infer_expr_type` now serves Engine A's recorded answer; the fallback at the BOTTOM is the stale Engine B re-derivation that the keystone was supposed to obsolete but which was left in place "as fallback."

### 1.4 The MISS semantics (what falls through to the fallback)
`infer_expr_type` is called primarily by the **strict-typing binop operand check** in `binary_ops.rs` (e.g. `strict_typing_binop_error` at `binary_ops.rs:214-237`; the Add/Sub/Mul/Eq dispatch arms at `binary_ops.rs:1592-1593`, `:1518/:1522`). A "MISS" = the keystone span-table has no entry for the expr's span AND no ladder arm fires. The fallback then returns `unknown`/`Err`, and the binop emitter produces `strict_typing_binop_error` ("Cannot infer types for binary operation… operand types are `unknown`…"). So **a MISS already surfaces as a loud compile error today** — but only because the ladder + keystone catch the legitimate cases first. The ladder exists to prevent **false** rejects (real types the fallback engine can't see).

### 1.5 What deleting the fallback requires (the surface-and-stop target)
To delete `mod.rs:2187`, the keystone table must be **complete enough** that every expr the strict checker asks about either (a) hits the table, or (b) is a genuine un-inferable that *should* error. Today the fallback's only unique value is: **module-scope expressions the persistent `self.type_inference.env` happens to know about via incremental type-definition registration** — chiefly enum/struct *name* references and trait-dispatch lookups that the ladder routes through `self.type_inference.env.*` (e.g. `binary_ops.rs:1553`, `helpers.rs:2002`, `type_ops.rs:107`). Those `env`-lookup uses are **separate from `infer_expr`** and survive fallback deletion. The `infer_expr(expr)` call itself contributes nothing the table+ladder don't already cover for in-body expressions; for module-scope top-level expressions it can still re-derive — **those must instead be recorded by Engine A into the table** (Engine A already walks module scope, so this is a recording-completeness fix, not a re-derivation need).

---

## 2. THE ~26-ARM LADDER INVENTORY (`infer_expr_type`, `expressions/mod.rs:1522-2193`)

Full enumeration in source order. Classification: **(KS)** keystone-table read · **(ST)** side-table read · **(RD)** structural re-derivation via `concrete_type_for_expr`/etc. · **(RP)** string re-parse.

| # | Line | Guard / Expr shape | Reads | Notes |
|---|------|--------------------|-------|-------|
| 0 | 1558-1578 | **Keystone consult** (all exprs except `PropertyAccess`) | **KS** | NEW (2026-06-22). Returns recorded `Type` on hit. **Excludes reference-typed results** (falls through so GapA projection runs) and **excludes `PropertyAccess`** (SB-1, see §3). |
| 1 | 1594-1601 | `Identifier` bound to a reference (`reference_referent_type_name`) | ST | GapA referent projection `&T`→`T`. |
| 2 | 1602-1621 | `Identifier`, tracker `type_name` is temporal or primitive scalar | **ST + RP** | Reads `tracker_type_name_for_identifier` (the `type_name: Option<String>` field). |
| 3 | 1632-1636 | `Identifier`, tracker name ends `"[]"` | **ST + RP** | **`strip_suffix("[]")`** — audit's `:1635` re-parse, **CONFIRMED at `:1632`**. |
| 4 | 1650-1686 | `Identifier`, recorded `ConcreteType` in `current_function_local_concrete_types`/`module_binding_concrete_types` | **ST + RD** | Projects `ConcreteType`→annotation. |
| 5 | 1696-1709 | `FunctionCall` to `-> &T` fn (`function_defs`) | RD | GapA return-deref projection. |
| 6 | 1710-1712 | `FunctionCall`, tracker `get_function_return_type` | **ST + RP** | Stringly return name. |
| 7 | 1719-1723 | `FunctionCall` to local closure (`local_callable_return_types`) | **ST + RP** | |
| 8 | 1724-1743 | `FunctionCall` to module-binding closure (`module_binding_callable_return_types`) | **ST + RP** | Two lookups (scoped + bare). |
| 9 | 1762-1767 | `MethodCall` `to_string`/`toString` | hard-coded | Universal-method shortcut. |
| 10 | 1787-1818 | `MethodCall` string-returning builtin (`charAt`/`slice`/…) | RD (recurses on receiver) | STAGE-S5; proves receiver is `string` via self-recursion. |
| 11 | 1829-1867 | `MethodCall` `__call__` on `arr[i]` (`local_array_callable_return_types`/`module_binding_array_callable_return_types`) | **ST + RP** | Callable-array-element. |
| 12 | 1875-1895 | `BinaryOp::Add` both-string → `string` | RD (recurses on both operands) | String-concat propagation. |
| 13 | 1905-1938 | `PropertyAccess` (non-optional), object has tracker schema_id → field type, then object-field-contract | **ST** | `field_type_to_annotation` (`mod.rs:2338`). **Runs DESPITE the keystone PropertyAccess exclusion** — this is the patch the exclusion comment points to. |
| 14 | 1939-1963 | `PropertyAccess` derived object (`rs[0].len`) → `concrete_type_for_expr` | **RD** | T1 sub-case (a); routes un-annotatable case to STAGE-F1. |
| 15 | 1993-2027 | `IndexAccess` const-int into `ConcreteType::Tuple` (concrete-types side-tables) | **ST + RD** | Tuple element. |
| 16 | 2044-2049 | `IndexAccess`, receiver is `string` → `string` | RD (recurses on object) | STAGE-S4 char model. |
| 17 | 2050-2052 | `IndexAccess` → `tracked_array_element_type` | **ST + RP** | Calls helper at `:2208` which does **`strip_prefix("Array<")`/`strip_prefix("Vec<")`/`strip_suffix("[]")`** — audit's `:2230` re-parse, **CONFIRMED at `:2229-2234`**. |
| 18 | 2062-2072 | `IndexAccess` nested (`m[r][c]`) → `concrete_type_for_expr` | **RD** | |
| 19 | 2089-2102 | `MethodCall` receiver-derived element (`first`/`last`/`pop`/`sort`…) → `method_call_receiver_derived_concrete_type` | **RD** | |
| 20 | 2119-2131 | `MethodCall` inline result → `concrete_type_for_expr` | **RD** | ROOT-2. |
| 21 | 2147-2159 | `FunctionCall` inline result → `concrete_type_for_expr` | **RD** | ROOT-2. |
| 22 | 2171-2185 | `Array(..)` literal → `concrete_type_for_expr` array projection | **RD** | |
| 23 | **2187** | **FALLBACK** `self.type_inference.infer_expr` | **RD (Engine B, module-scope)** | The deletion target. |

**Net:** 22 patch arms + the keystone consult + the fallback ≈ the audit's "~26-arm ladder" (the audit counted sub-arms). Of these, **the string-re-parse arms are #3 (`:1632`), #6/#7/#8/#11 (stringly return names via `TypeAnnotation::Basic`), and #17 (`tracked_array_element_type` :2229-2234)**. The bulk of the rest are `concrete_type_for_expr`/`method_call_receiver_derived_concrete_type` structural re-derivations — these duplicate work Engine A *also* did, and are candidates for deletion once the table records method/call/index results (it largely does now — see §3).

---

## 3. KEYSTONE COMPLETENESS (SB-1): WHICH EXPRESSIONS THE ENGINE DOES vs DOES NOT EXPORT

### 3.1 The engine's recording is EXHAUSTIVE over `Expr`
`infer_expr` wraps `infer_expr_inner` and records **every** non-dummy-span expr into the table (`expressions.rs:165-175`). `infer_expr_inner` (`expressions.rs:181`) has an **explicit arm for every `Expr` variant — NO catch-all `_ =>`** (verified: zero top-level `_ =>` between L181 and the function end). Variants covered include: `Literal`, `Identifier`, `BinaryOp`, `UnaryOp`, `PropertyAccess`, `IndexAccess`, `FunctionCall`, `QualifiedFunctionCall`, `EnumConstructor`, `Array`, `TableRows`, `Object`, `Conditional`, `TypeAssertion`, `InstanceOf`, `MethodCall`, `Match`, `If`, `While`, `For`, `Loop`, `Let`, `Assign`, `Block`, `FunctionExpr`, `ListComprehension`, `DataRef`, `TimeRef`, `DateTime`, `Duration`, `PatternRef`, `Spread`, `Range`, `Break/Continue/Return`, `Unit`, `TryOperator`, `UsingImpl`, `SimulationCall`, `WindowExpr`, `FuzzyComparison`, `FromQuery`, `StructLiteral`, `Await`, `Join`, `Annotated`, `AsyncLet`, `AsyncScope`, `Comptime`, `ComptimeFor`, `Reference`.

### 3.2 So "non-export" is NOT about missing arms — it is about TWO drop mechanisms
The engine records every expr **pre-solve**, then `finalize_expr_type_table` (`mod.rs:361-378`) **drops** entries that are not fully resolved post-substitution (`type_is_fully_resolved`, `mod.rs:385`: any embedded free `Type::Variable` → dropped, no Unknown-default). An expr is therefore **absent from the final table** iff:

1. **Its arm returns `Err`** → never recorded (e.g. STAGE-F1 field reject — see §3.4).
2. **Its arm returns a type that stays a free var post-solve** → recorded then dropped. The arms most prone to this (return `fresh_type_var()` with no constraint that the solver can pin from module context): `TableRows` (always fresh), `QualifiedFunctionCall` to a real module fn (deliberately fresh, `expressions.rs:446`), `SimulationCall`, `Comptime`/`ComptimeFor`, `Join`, empty `Array` whose element never gets pinned, `Range` over non-numeric, `Let`-with-no-init. These are the genuinely-un-inferable tail and SHOULD stay absent → surface-and-stop.

### 3.3 The ONE deliberate consult exclusion: `PropertyAccess` (SB-1's field-erasure)
**The engine DOES export PropertyAccess types** (its `PropertyAccess` arm at `expressions.rs:306-358` calls `infer_property_access`, returns concrete field types, and `infer_expr` records them). **But the COMPILER deliberately refuses to consult the table for `PropertyAccess`** at `expressions/mod.rs:1558` (`if !matches!(expr, Expr::PropertyAccess { .. })`). The documented reason (`mod.rs:1546-1557`): serving the table entry would **MASK the STAGE-F1 compile error** for `rs[0].n` where `rs`'s element type is known only from a `push` into an unannotated `[]`. Field-type recovery for legitimate cases is instead routed through ladder arms #13/#14, which deliberately re-derive structurally and let the un-annotatable case hit STAGE-F1.

**This is the SB-1 field-erasure root, and the audit's framing is now INVERTED by the post-audit code:** the audit said "the engine excludes PropertyAccess; move STAGE-F1 strictness INTO the engine." **STAGE-F1 is ALREADY a real engine error** (§3.4). The remaining problem is that the *compiler consult* still skips the table for PropertyAccess to avoid masking that error — but it ALSO skips the table for the *legitimately-resolvable* field reads, forcing them through ladder arms #13/#14. **The U4 deletion shape here:** make the engine's STAGE-F1 reject the SINGLE source of field-read strictness (it already is), then DELETE the compiler's PropertyAccess exclusion so resolvable field reads hit the table directly and arms #13/#14 disappear. The exclusion is the "kept-as-fallback" hedge.

### 3.4 STAGE-F1 is a REAL engine error (audit task ALREADY DONE)
`constraints.rs:1108-1149` (audit said `:1050` — shifted to `:1108`). This is **inside the constraint solver's field-access constraint arm** — a genuine `TypeError::ConstraintViolation` returned from the engine when a field is read off a value whose element type was back-propagated to a bare named-struct `Reference` (the unannotated-empty-`[]`-grown-by-`push` case). It is **not** a downstream compiler patch. The audit's "move STAGE-F1 strictness into the engine" is satisfied; what remains is removing the *compiler-side workaround* (the consult exclusion + arms #13/#14).

### 3.5 Second keystone consumer (not in audit)
Beyond `infer_expr_type`, the table is also read by **`keystone_scrutinee_concrete_type`** (`advanced.rs:392-405`) for `match` scrutinee types (`match g() { Ok(p) => p.x }`). This is a clean keystone use and a model for what the collapsed design looks like.

---

## 4. SB-7: THE FOUR STATIC-TYPE REPRESENTATIONS — verified at HEAD

| Rep | Type | Location (HEAD) | Audit said | Status |
|-----|------|-----------------|-----------|--------|
| 1 | `Type` / `TypeAnnotation` (structural, shape-runtime) | `shape-runtime/.../type_system/`; table is `HashMap<Span, Type>` | (structural) | The canonical one (U1 landed "one equality"). |
| 2 | `type_name: Option<String>` (stringly) | **`type_tracking.rs:340`** | `:338` | **STILL PRESENT.** Read via `tracker_type_name_for_identifier` (`mod.rs:2245`). The source of the `strip_suffix("[]")` (`mod.rs:1632`) and `strip_prefix("Array<")` (`mod.rs:2229-2234`) re-parses. |
| 3 | `ConcreteType` (shape-value v2) | `shape_value::v2::ConcreteType` | (v2) | **STILL PRESENT.** Side-tables store it; ladder projects it via `concrete_type_to_type_annotation` (`closures.rs:681`). |
| 4 | `NumericType` (per-last-expr mutable register) | enum **`type_tracking.rs:54`**; register **`compiler/mod.rs:726`** (`last_expr_numeric_type`) | enum `:52`, reg `mod.rs:726` | **STILL PRESENT.** `mod.rs:726` exact. |

**The drift the audit names is CONFIRMED and active:** `binary_ops` selects opcodes from `NumericType` (`last_expr_numeric_type`, set as a side-effect of `compile_expr` — e.g. consumed at `binary_ops.rs:1518/1522`, planned via `plan_coercion` in `numeric_ops.rs:153`, `EqOperandType` mapping at `binary_ops.rs:1053-1055`), while the **same binop** independently calls `infer_expr_type` for the strict-proof/heap-dispatch decision (`binary_ops.rs:1592-1593`). Two representations, two code paths, same operands, computed separately. There is also a **`Type → String → re-parse` round-trip**: `type_display_name` (`numeric_ops.rs:102`) renders `Type::Concrete(Array(inner))` as `"{inner}[]"` (`:119`), which `is_arrayish` (`binary_ops.rs:1669-1673`) and `tracked_array_element_type` then re-parse with `strip_suffix("[]")`.

---

## 5. SB-8: THE HINT SIDE-TABLES — census at HEAD (line numbers all shifted)

**All 14 of the audit's side-tables still exist** (none deleted by U1/U3). Declaration sites in `compiler/mod.rs`:

| Side-table | Decl (HEAD) | Audit | Type |
|------------|-------------|-------|------|
| `current_function_local_concrete_types` | `:1684` | `:1711` | `HashMap<u16, ConcreteType>` |
| `module_binding_concrete_types` | `:1710` | `:1737` | `HashMap<u16, ConcreteType>` |
| `inferred_param_concrete_types` | `:1383` | `:1410` | per-fn `Vec<Option<ConcreteType>>` |
| `inferred_param_type_hints` | `:1371` | `:1410` | `HashMap<String, Vec<Option<String>>>` (stringly) |
| `inferred_return_type_hints` | **consumed locally, NOT a field** | `:1398` | Built at `compiler_impl_reference_model.rs:826`, drained into tracker `function_return_types` at `:2085` — **already half-retired** (not stored). |
| `local_callable_return_types` | `:771` | `:771` | `HashMap<u16, String>` (stringly) |
| `module_binding_callable_return_types` | `:775` | `:775` | `HashMap<u16, String>` |
| `local_array_callable_return_types` | `:784` | `:784` | `HashMap<u16, String>` |
| `module_binding_array_callable_return_types` | `:788` | `:788` | `HashMap<u16, String>` |
| `array_element_types` | `:1237` | `:1263` | `HashMap<Span, ConcreteType>` |
| `local_array_element_types` | `:1240` | `:1266` | `HashMap<u16, ConcreteType>` |
| `module_binding_array_element_types` | `:1243` | `:1269` | `HashMap<u16, ConcreteType>` |
| `inferred_param_object_fields` | `:1423` | (listed) | per-fn `Vec<Option<Vec<(String,FieldType)>>>` |
| `inferred_return_object_fields` | `:1435` | (listed) | per-fn field lists |

**NEW since audit (U3-era, NOT in SB-8 list):** `local_map_key_value_types` (`:1228`), `module_binding_map_key_value_types` (`:1232`) — U3's HashMap-carrier unification ADDED these rather than removing tables. So the side-table count GREW, not shrank. The comment at `mod.rs:1707` references them as already covering the map case.

Each side-table is a **frozen projection of Engine A's output** captured at `infer_reference_model` time (`compiler_impl_reference_model.rs:824-848`) — exactly SB-8's "frozen projection of L2." Once the keystone table is consulted unconditionally (PropertyAccess included) and records method/call/index/array results (it largely does already — arms #4,#13-22 prove the table CAN serve these), these projections become redundant and are the deletion target.

---

## 6. WHAT BLOCKS DELETING THE FALLBACK (the surface-and-stop gap)

The fallback (`mod.rs:2187`) can be deleted once these are true:

1. **PropertyAccess consult exclusion removed** (`mod.rs:1558`). Blocker: must first confirm the engine's STAGE-F1 reject (`constraints.rs:1108`) fires for EVERY un-annotatable field read that the ladder arms #13/#14 currently route to it — i.e. the engine error must be the SOLE gate, with no resolvable field read depending on arms #13/#14 to recover a type the table lacks. (The table HAS the resolvable ones; the risk is a field read the engine left as a free var → dropped → table miss → needs the structural re-derivation. Those must instead be recorded by the engine or be genuine errors.)

2. **Reference-typed table entries projected at record time, not skipped.** Today the keystone consult skips `Borrow`-typed table hits (`mod.rs:1571-1575`) so the GapA `&T→T` projection (arms #1/#5) runs. To delete the fallback cleanly, the engine should record the *projected referent* (or the consult should project inline) so reference reads hit the table.

3. **Module-scope top-level expressions must be recorded by Engine A.** The fallback's only non-stale value is module-scope exprs the persistent `self.type_inference.env` knows via incremental type-def registration. Engine A walks module scope too, so this is a **recording-completeness** task (ensure `infer_program_best_effort` records top-level expr spans), not a re-derivation need. The `self.type_inference.env.*` LOOKUP uses (trait dispatch, enum/alias resolution at `binary_ops.rs:1553`, `helpers.rs:2002`, `type_ops.rs:107-109`) are **independent of `infer_expr`** and survive fallback deletion — only the `infer_expr(expr)` call at `:2187` is the target.

4. **Span-collision safety.** The table keys on `Span` and skips dummy spans (`expressions.rs:171`, `mod.rs:342`). Desugared/synthetic nodes with dummy spans are never in the table → they'd become permanent misses → must be guaranteed to be genuine surface-and-stop cases or given real spans. This is a correctness precondition for "miss == error."

**Bottom line for the synthesis agent:** the keystone table + finalize-drop + first-consult are ALREADY built and working (probe `/tmp/probe_keystone.shape` prints `7`, proving `r.n`'s `int` flows through the for-binder via the table). U4 is the **deletion half**: remove the PropertyAccess consult exclusion (arms #13/#14 with it), retire the 14+2 SB-8 projections as the table subsumes them, replace `type_name: Option<String>` (`type_tracking.rs:340`) + `NumericType` register (`mod.rs:726`) reads with `Type`/`ConcreteType` so the `strip_suffix("[]")`/`strip_prefix("Array<")` re-parses (`mod.rs:1632`, `:2229-2234`) and the `type_display_name` round-trip (`numeric_ops.rs:119`) vanish, and finally delete `self.type_inference.infer_expr` at `mod.rs:2187` so a table miss is a loud surface-and-stop. The ladder collapses to: **keystone consult → ConcreteType projection (for opcode-stamp NumericType derivation only) → surface-and-stop.**",
      "bReps": "I have everything needed. Here is the complete report.

---

# U4 / SB-7 Facet Report: The Four Static-Type Representations in L3 (verified vs HEAD `18991310`)

## 0. Worktree + audit-drift summary

- Worktree `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch`, branch `strict-flip-collection-dispatch`, HEAD `18991310`. Read-only; nothing edited.
- **`type_tracking.rs` moved.** The audit cites `type_tracking.rs:338` / `:52` / `:726`. The file is now at `crates/shape-vm/src/type_tracking.rs` (NOT under `compiler/`). All line numbers below are HEAD-verified.
- **The single biggest change vs the audit: a FIFTH carrier has been ADDED, not removed.** The strict-flip T1 keystone (2026-06-22) added the engine span-keyed `expr_type_table: HashMap<Span, Type>` (`crates/shape-runtime/src/type_system/inference/mod.rs:268`) plus a compiler-side copy `resolved_expr_types: HashMap<Span, Type>` (`crates/shape-vm/src/compiler/mod.rs:891`). This is *exactly the canonical "one source of truth" the U4 roadmap wants to converge on* — it already exists and is already consulted FIRST in the ladder, but as an additive layer on top of the 4 reps, not a replacement. The U4 deletion target is now well-defined: collapse the 4 legacy reps onto this span table.
- **U1 (commit `9fb34e9a`) landed:** ONE canonical `Type::Generic { base, args }` encoding + one equivalence. Visible in `type_display_name` (numeric_ops.rs:122-132) which now special-cases the canonical `Type::Generic` form.
- **U3 (commit `ffea6ade`) landed:** TypedMap carrier DELETED, all HashMap routed to HashMapData. Map-keyed dual carriers are gone; the surviving map side-tables are now stringly/ConcreteType key-value tables (`local_map_key_value_types` mod.rs:1228, `module_binding_map_key_value_types` mod.rs:1232) — they are SB-8 hint tables, not the SB-7 carrier duality.
- The audit's "expressions/mod.rs:2188 empty fallback engine" is confirmed live at `crates/shape-vm/src/compiler/expressions/mod.rs:2187` — the terminal `self.type_inference.infer_expr(expr)` arm.

---

## 1. The four representations — definition, contents, writers, readers, lifetime

### REP A — Engine `Type` (+ `TypeAnnotation`) — shape-runtime — **CANONICAL KEEP**

- **Definition:** `crates/shape-runtime/src/type_system/types/core.rs:93` — `pub enum Type { Concrete(TypeAnnotation), Variable(TypeVar), Generic{base,args}, Constrained{var,constraint}, Function{params,returns} }`.
- **Stores:** the inference-level type. `Type::Concrete` *wraps the AST `TypeAnnotation`* (`crates/shape-ast/src/ast/types.rs:10` — Basic/Array/Tuple/Object/Function/Union/Intersection/Generic/Reference/Borrow/Void/Never/Null/Dyn). So "structural" is qualified: `Type` is structural for type variables/generics/functions, but a concrete type is an AST-`TypeAnnotation` payload, not a distinct structural enum. This matters for U4: the canonical rep is `Type`, but `ConcreteType` (Rep C) is structurally *richer for monomorphized scalars* (carries i8/i16/u-widths/f32/char that `TypeAnnotation::Basic("...")` only encodes as a string).
- **Span-table form (the keystone):** `expr_type_table: HashMap<Span, Type>` at `inference/mod.rs:268`.
  - **Writers (engine-side):** the single recording point `inference/expressions.rs:172` (`self.expr_type_table.insert(span, ty.clone())` inside `infer_expr`, wrapping `infer_expr_inner`). Records EVERY non-dummy-span expression. Post-solve finalized by `finalize_expr_type_table` (`mod.rs:361`), called at `mod.rs:1681` and `mod.rs:1908`; cleared at `mod.rs:1629`/`:1716`. Finalization re-applies the unifier substitution and DROPS any entry still containing a free `Type::Variable` (no Unknown-default — strict).
  - **Handoff to compiler:** `take_expr_type_table` (`mod.rs:352`) → consumed at `compiler/compiler_impl_reference_model.rs:823` and stored into `self.resolved_expr_types` at `:2062`.
  - **Readers:** `resolved_expr_type(span)` (`mod.rs:341`); compiler reads its copy `resolved_expr_types` FIRST in the ladder at `compiler/expressions/mod.rs:1561`.
- **`Type` as the ladder return:** `infer_expr_type` returns `Result<Type>` (`expressions/mod.rs:1522`). **56 caller sites** across 14 compiler files (binary_ops, function_calls, property_access, loops, advanced, closures, assignment, unary_ops, matrix_ops, type_ops, statements, v2_typed_emission, helpers_reference, mod). This is the dominant L3 read interface; it is the right thing to keep.
- **Lifetime/scope:** engine table is per-inference-run; compiler copy lives for the whole compile. The non-table `Type` values are ephemeral (returned by `infer_expr_type`, immediately projected to NumericType or a name).

### REP B — Tracker `type_name: Option<String>` — shape-vm — **DELETION TARGET (stringly)**

- **Definition:** `crates/shape-vm/src/type_tracking.rs:340` — field of `pub struct VariableTypeInfo` (struct at `:336`). Audit said `:338`; it is `:340` at HEAD.
- **Stores:** a *stringly* type name, e.g. `"Candle"`, `"int"`, `"Vec<int>"`, `"int[]"`, `"Option<Number>"`. Sibling stringly fields in the same struct: `concrete_numeric_type: Option<String>` (`:355`), and `VariableKind`-carried strings (`element_type`, `column_type` at `:463/:479/:496/:512`).
- **Writers:** via `VariableTypeInfo` constructors `known`/`named`/`with_storage`/`row_view`/`datatable`/`column`/`indexed` (type_tracking.rs:366-512) and direct `.type_name =`. **~65 construction sites** in `compiler/` (counted via `VariableTypeInfo::named|known|with_storage|.type_name =`), plus **~70** `set_local_type`/`set_binding_type` flow-in sites. The decisive *Type→String* projection that feeds it: `tracked_type_name_from_annotation` (`compiler/helpers.rs:1881`) — maps `TypeAnnotation::Array(inner)` → the literal string `format!("Vec<{}>", inner.to_type_string())` (`:1885`), and `Generic{Vec,...}` → `"Vec<...>"` (`:1888`). This is where the structural array Type is *destroyed into a string*.
- **Readers:** **62** `.type_name` reads in `compiler/`. The load-bearing ones for SB-7:
  - `tracker_type_name_for_identifier` (`expressions/mod.rs:2245`) — the ladder's primitive-name + array-name recovery (`:1602`, `:1616`, `:1632`).
  - `tracked_array_element_type` (`expressions/mod.rs:2208`) — **the re-parse site** (`:2225-2234`).
  - `numeric_operand_proof_gap` (`binary_ops.rs:366` — `info.type_name.is_some()`), `safe_adopt_numeric_hint` (reads `info.storage_hint`), the STAGE-F1 guard path.
- **Lifetime/scope:** per-slot, persists for the function/module frame in `TypeTracker`. Compiler-tier only — NOT serialized into `FunctionBlob` (documented at type_tracking.rs:346-351).
- **Sibling stringly side-table:** `function_return_types: HashMap<String,String>` (`type_tracking.rs:668`, writer `set_function_return_type` `:829`, reader `get_function_return_type` `:834`; consumed in ladder at `expressions/mod.rs:1710`). Same string-typed deletion class as `type_name`.

### REP C — `ConcreteType` — shape-value v2 — **KEEP (the emit-time structural truth), but currently a 3rd parallel inference source**

- **Definition:** `crates/shape-value/src/v2/concrete_type.rs:84` — `pub enum ConcreteType { F64,F32,Char,I64,I32,I16,I8,U64,U32,U16,U8,Bool,String,Struct(NamedTypeId),Array(Box),HashMap(Box,Box),Option(Box),Result(Box,Box),Enum(NamedTypeId),Closure,Function,Pointer(Box),Tuple(Vec),Void,Decimal,BigInt,DateTime,HashSet,Deque,PriorityQueue,Channel,Mutex,Atomic,... }`.
- **Stores:** the fully-monomorphized, structural, no-type-variable type. Richer than `Type`/`TypeAnnotation` for scalars (explicit i8/i16/i32/u-widths/f32/char). This is the correct emit-time structural carrier — it must survive as the thing the bytecode emitter and monomorphizer consume.
- **Writers (in the L3 inference role):** the per-slot side-tables (Rep-C *as a hint table* = SB-8): `current_function_local_concrete_types` (mod.rs:1684), `module_binding_concrete_types` (mod.rs:1710), `inferred_param_concrete_types` (mod.rs:1383), `array_element_types`/`local_array_element_types`/`module_binding_array_element_types` (mod.rs:1237/1240/1243), `local_map_key_value_types`/`module_binding_map_key_value_types` (mod.rs:1228/1232). Structural producer: `concrete_type_for_expr` (`compiler/monomorphization/type_resolution.rs:1621`) and `declared_annotation_concrete_type` (`type_resolution.rs:1286`).
- **Readers (in the ladder):** **66** `concrete_type_for_expr(` calls in `compiler/`. The ladder consults Rep C heavily: scalar local recovery (`expressions/mod.rs:1650-1686`), tuple index (`:2009`), nested-index (`:2062`), PropertyAccess struct-field (`:1951`), method-call receiver-derived (`:2089-2102`), inline method/function call (`:2119`/`:2147`), array-literal element (`:2171`). Every one of these immediately converts ConcreteType→TypeAnnotation via `concrete_type_to_type_annotation` (see §2).
- **Lifetime/scope:** per-slot (the `_concrete_types` maps) and per-expression (computed on demand by `concrete_type_for_expr`). Survives into bytecode emission as the real type.

### REP D — `NumericType` — shape-vm — **DELETION TARGET as a separate per-expression register**

- **Definition:** `crates/shape-vm/src/type_tracking.rs:54` — `pub enum NumericType { Int, IntWidth(IntWidth), Number, Decimal }`. Audit said `:52`; it is `:54` at HEAD.
- **Per-expression mutable register:** `last_expr_numeric_type: Option<NumericType>` at `crates/shape-vm/src/compiler/mod.rs:726` (audit's `:726` — CONFIRMED exact). Initialized `None` at `compiler_impl_initialization.rs:61`.
- **Stores:** the numeric subtype of *the last compiled expression*, used to pick a typed arithmetic/comparison opcode.
- **Writers:** **99 assignment sites** (`last_expr_numeric_type = ...`) across literals.rs, identifiers.rs, property_access.rs, function_calls.rs (the largest cluster, ~30 sites), binary_ops.rs, type_ops.rs, unary_ops.rs, advanced.rs, helpers.rs, destructure.rs, and ~40 `= None` clears in collections/loops/matrix_ops/closures. It is set as a side effect of compiling almost every expression kind.
- **Readers:** `last_expr_numeric_type_to_storage_hint` (`helpers_binding.rs:549` — NumericType→NativeKind/StorageHint, the emit-time stamp), the binop emitter writeback at `binary_ops.rs:796`, the for-loop element-kind path (`loops.rs:1120-1273`), v2_typed_emission.rs:557, helpers.rs:3796. Plus the `NumericType` *value* (not the register) is the index into the `TYPED_ARITH`/`TYPED_CMP` opcode tables (numeric_ops.rs:228-244, indexed via `numeric_type_index` `:271`).
- **Lifetime/scope:** **per-last-compiled-expression mutable register** — CONFIRMED. It is overwritten on every expression compile and read immediately by the enclosing binop/return/let. This is precisely the "mutable register that drifts from the Type table" the audit flags.

---

## 2. The conversion-arm map (the ~26 arms — where one rep converts to another)

The audit's "~26 arms" is split across two locations: the `infer_expr_type` ladder (Rep-recovery arms) and the numeric/projection helpers (Rep↔Rep converters). Verified inventory:

**Rep A → Rep D (Type → NumericType), for opcode selection — the SB-7 DRIFT CORE:**
- `inferred_type_to_numeric(ty: &Type) -> Option<NumericType>` — `compiler/expressions/numeric_ops.rs:78`. Pattern-matches `Type::Concrete(TypeAnnotation::Basic(name))`/`Reference(name)`, then re-classifies the STRING `name` via `IntWidth::from_name` / `BuiltinTypes::is_integer_type_name` / `is_number_type_name` / `"decimal"`. **This is a Type→string-name→NumericType hop.**
- Used by `infer_numeric_pair` (`binary_ops.rs:428-442`) which calls `infer_expr_type` (Rep A) then `inferred_type_to_numeric` (→ Rep D) for both operands.
- The EqOperand path (`binary_ops.rs:1050-1055`) does the same Type→NumericType match for typed-equality dispatch.

**THE TWO-SOURCES-FOR-"IS-THIS-INT-OR-NUMBER" DRIFT (audit's headline):**
- `infer_expr_type` reads the **Rep-A `Type` table** (`expressions/mod.rs:1561`, the keystone) to answer "what type is this operand."
- `binary_ops` separately reads the **Rep-D `last_expr_numeric_type` register** (written by 99 sites as a side effect of compilation) and writes it BACK at `binary_ops.rs:796` after emitting a typed opcode. So the int-vs-number answer flows through BOTH the post-solve `Type` table AND a per-expression mutable register that is set ad hoc per expression kind. When a writer site stamps the register but the `Type` table says otherwise (or vice versa), they disagree — exactly the SB-7 drift. The proof-gap guard `numeric_operand_proof_gap` (`binary_ops.rs:347`) exists *specifically because* a NumericType hint can be `Some` while `infer_expr_type` returns `Type::Variable` (a fabricated claim) — direct evidence of the two sources disagreeing (`binary_ops.rs:374-381`).

**Rep C → Rep A (ConcreteType → TypeAnnotation/Type) — the ladder's dominant recovery conversion:**
- `concrete_type_to_type_annotation(ct) -> Option<TypeAnnotation>` — `compiler/expressions/closures.rs:681`. Called **~14×** in the ladder (every `concrete_type_for_expr` consult wraps its result through this). Note the `Array(inner)` arm (closures.rs:697-706) renders to `TypeAnnotation::Generic{ name: "Vec", args:[inner] }` — i.e. ConcreteType→annotation→(later)→`"Vec<...>"` string. **This round-trips straight into the Rep-B re-parse.**
- `concrete_to_annotation(ct) -> TypeAnnotation` — `compiler/monomorphization/substitution.rs:75` (total version, monomorphizer-side).

**Rep A → Rep B/string projections (Type/Annotation → String):**
- `tracked_type_name_from_annotation` — `helpers.rs:1881` (Annotation→`"Vec<...>"`/name string; feeds tracker `type_name`).
- `type_display_name(ty)` — `numeric_ops.rs:102` (Type→display string, used by operator-trait dispatch and `expr_is_proven_string` at binary_ops.rs:519).
- `type_display_name_for_closure_inference` — `closures.rs:737`.

**Rep D → Rep NativeKind (NumericType → StorageHint/NativeKind, the emit stamp):**
- `last_expr_numeric_type_to_storage_hint` — `helpers_binding.rs:549` (Int→Int64, Number→Float64, IntWidth(w)→Int8/UInt8/.../UInt64). This is the legitimate "keep NumericType/NativeKind only as the emit-time stamp" role — but it currently derives from the standalone register (Rep D), not from the one `Type`.

**Rep C ↔ Rep D adjacent:** `field_type_to_numeric` (property_access.rs:405) and `builtin_return_numeric_type`/`method_return_numeric_type` (function_calls.rs:1590/3608/3757) produce Rep D directly from FieldType/method tables, bypassing Rep A — another independent source feeding the same register.

**Rep recovery arms in the ladder (the "~26 arm ladder" itself):** `expressions/mod.rs:1558-2185`, in order — span-table consult (A), reference-referent (B-string), tracker primitive-name (B), tracker array-name `.strip_suffix("[]")` (B, re-parse), recorded-ConcreteType scalar (C→A), function-return tracker (B-string), closure-binding return (B-string), `to_string` (literal), string-method return (literal), callable-array-element (B-string), string-Add (recursive A), PropertyAccess schema/contract (FieldType→A), PropertyAccess derived ConcreteType (C→A), tuple-index ConcreteType (C→A), string-index (literal), tracked array element (B re-parse), nested-index ConcreteType (C→A), method-receiver-derived ConcreteType (C→A), inline method-call ConcreteType (C→A), inline function-call ConcreteType (C→A), array-literal element ConcreteType (C→A), terminal engine fallback (A). That is **~22 arms** at HEAD (the audit's ~26 was pre-keystone; the keystone arm was prepended and a couple of map arms were folded into U3).

---

## 3. The stringly re-derivation / re-parse sites (audit :1635 and :2230) — CONFIRMED, relocated

**Re-parse site #1 — `.strip_suffix("[]")` (audit :1635):**
- HEAD location: `compiler/expressions/mod.rs:1632` (the `if let Some(elem) = type_name.strip_suffix("[]")` arm, inside the `Expr::Identifier` branch of `infer_expr_type`). It takes the tracker `type_name` STRING (Rep B), strips `"[]"`, and rebuilds `Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(elem))))` (`:1633-1635`). Pure string→structure re-parse. Vanishes if Rep B were a real `ConcreteType`/`Type`.

**Re-parse site #2 — `Array<`/`Vec<` re-parse (audit :2230):**
- HEAD location: `compiler/expressions/mod.rs:2229-2234`, inside `tracked_array_element_type` (fn at `:2208`). It takes the tracker `type_name` STRING (or the reference-referent string), `.strip_prefix("Array<").or strip_prefix("Vec<").and_then strip_suffix('>')`, else `.strip_suffix("[]")`, then rebuilds `Type::Concrete(TypeAnnotation::Basic(inner))` (`:2238`). Rejects `""`/`"unknown"` inner (`:2235`).
- **The round-trip is closed:** `tracked_type_name_from_annotation` (helpers.rs:1885) *produces* the `"Vec<...>"` string from a structural `TypeAnnotation::Array`, and `tracked_array_element_type` (mod.rs:2230) *re-parses* it back. The structural element type is destroyed and reconstructed across the tracker string boundary — the canonical SB-7 stringly-defect. Both sites disappear when `type_name: Option<String>` becomes `Option<ConcreteType>` (or the slot's element type is read from the span-table / `_concrete_types` map directly).

---

## 4. Verdict per rep + blast radius

| Rep | Site | Verdict | What deletion breaks (readers to migrate) |
|-----|------|---------|--------------------------------------------|
| **A — Engine `Type` + span `expr_type_table`** | core.rs:93; inference/mod.rs:268; compiler resolved_expr_types mod.rs:891 | **CANONICAL KEEP** — make it the ONE source. Already consulted first. | Nothing to delete; instead the terminal fallback `self.type_inference.infer_expr` (expressions/mod.rs:2187) must become surface-and-stop, and the engine must record PropertyAccess (it already does at expressions.rs:172 — the *compiler* excludes it at mod.rs:1558; that exclusion + the STAGE-F1 masking concern at constraints.rs:1108 must move so a field-read MISS surfaces). |
| **B — Tracker `type_name: Option<String>`** (+ `function_return_types: HashMap<String,String>`, `concrete_numeric_type: Option<String>`) | type_tracking.rs:340 / :668 / :355 | **DELETE the string form** — replace with `Type`/`ConcreteType`. | 62 `.type_name` reads + ~65 writers + 70 set-sites. Specifically: `tracker_type_name_for_identifier` (mod.rs:2245), `tracked_array_element_type` (mod.rs:2208 — both re-parse sites die here), `numeric_operand_proof_gap` (binary_ops.rs:366), the schema-id lookups (mod.rs:2267). The two `.strip_*` re-parses (mod.rs:1632, :2230) and `tracked_type_name_from_annotation`'s `"Vec<...>"` projection (helpers.rs:1885) all DELETE when this is structural. `function_return_types` string table → fold into the span-table / return-`ConcreteType`. |
| **C — `ConcreteType`** | concrete_value.rs:84; side-tables mod.rs:1684/1710/1383/1237-1243/1228-1232; producer type_resolution.rs:1621 | **KEEP as the emit-time structural truth**, but RETIRE its *parallel-inference* role. The ~13 SB-8 `_concrete_types`/`_element_types`/`_callable_return_types` side-tables are frozen projections that exist only because the engine table didn't cover function bodies — the keystone now does. | 66 `concrete_type_for_expr` consults + 14 `concrete_type_to_type_annotation` conversions in the ladder. As the span-table becomes complete, each ladder arm that does `concrete_type_for_expr → concrete_type_to_type_annotation → Type` collapses to "span-table hit." `ConcreteType` survives ONLY downstream of inference (monomorphizer, bytecode emit, FrameDescriptor), keyed from the one resolved `Type`. |
| **D — `NumericType` + `last_expr_numeric_type` register** | type_tracking.rs:54; register mod.rs:726 | **KEEP `NumericType` as an emit-time enum; DELETE the standalone per-expression register as a *type source*.** It must be DERIVED from the one `Type` at the opcode-selection point, not maintained as a 99-writer mutable register that drifts. | `last_expr_numeric_type_to_storage_hint` (helpers_binding.rs:549 — the legit NumericType→NativeKind stamp, keep but re-source), the binop emitter (binary_ops.rs:796 writeback + the `infer_numeric_pair`→`inferred_type_to_numeric` Type→NumericType arm at binary_ops.rs:428/numeric_ops.rs:78), `TYPED_ARITH`/`TYPED_CMP` table indexing (numeric_ops.rs:228-277), for-loop element kind (loops.rs:1120-1273), v2_typed_emission.rs:557. ~99 writer sites collapse to one derivation `Type → inferred_type_to_numeric → opcode-table index`. |

**Net U4 shape (this facet):** Keep **Rep A (engine `Type` / span table)** as the single L3 inference source and **Rep C (`ConcreteType`)** as the single emit/monomorphization structural carrier derived from it. Delete **Rep B** (the stringly `type_name` + `function_return_types` + the two `.strip_*` re-parses + the `"Vec<...>"` projection) and delete **Rep D's standalone register** (derive `NumericType` from the one `Type` at the opcode-selection point only). The `inferred_type_to_numeric` (numeric_ops.rs:78) arm survives as the sole Type→NumericType *derivation* (no longer a competing source); `last_expr_numeric_type_to_storage_hint` survives as the sole emit stamp. The ~22-arm ladder collapses to: **span-table hit → (if needed) `ConcreteType` projection → surface-and-stop** once the engine records PropertyAccess field-reads and the terminal `infer_expr` fallback (expressions/mod.rs:2187) is deleted.

**Key audit corrections:** (1) the span `expr_type_table` (the convergence target) ALREADY EXISTS — it is a 5th additive carrier, not yet a replacement; (2) `type_tracking.rs` is at `crates/shape-vm/src/type_tracking.rs`, lines `:54`/`:340`/`mod.rs:726`; (3) SB-1's PropertyAccess exclusion now lives in the COMPILER consumer (mod.rs:1558) + the STAGE-F1 constraint (constraints.rs:1108), NOT the engine recorder (engine records everything at expressions.rs:172); (4) U3 already removed the map dual-carrier, so SB-8's map tables are now plain ConcreteType key-value hint tables; (5) both re-parse sites confirmed live at mod.rs:1632 and mod.rs:2230.",
      "cTables": "I now have complete, verified raw material. Let me compile the exhaustive report.

# U4 / SB-8 Facet Report — The ~13 Hint Side-Tables (Verified Against HEAD `18991310`)

## 0. Scope corrections vs the STRUCTURAL-AUDIT (read this first)

The audit was written at an earlier commit. Confirmed structural drift since then:

- **`type_tracking.rs` MOVED.** The audit cites `type_tracking.rs:52/338/726` as if inside `compiler/`. The file is at `crates/shape-vm/src/type_tracking.rs` (NOT under `compiler/`). The compiler struct itself lives in `crates/shape-vm/src/compiler/mod.rs:581` (`pub struct BytecodeCompiler`), 1829 lines.
- **`inferred_return_type_hints` is NOT a struct field.** The audit lists it as an SB-8 side-table. At HEAD it is a **transient local** in `compiler_impl_reference_model.rs` (built at :826, returned at :916/:2052, consumed at :2085) that is folded into the TypeTracker's stringly map `TypeTracker.function_return_types: HashMap<String,String>` (`type_tracking.rs:668`) via `register_function_return_type` (`type_tracking.rs:828`). So this "table" is really an **SB-7 stringly projection inside TypeTracker**, not a standalone compiler hint table. Its single inference reader is `expressions/mod.rs:1710` (`get_function_return_type`).
- **The HashMap element side-tables SURVIVED U3.** The prompt expected "the HashMap-related ones may be gone." They are NOT gone. U3 (`ffea6ade feat(U3/SB-9): DELETE TypedMap carrier — route all HashMap to HashMapData`) deleted the **runtime value carrier** `TypedMap`, not the **compiler-side inference hint tables** `map_key_value_types` / `local_map_key_value_types` / `module_binding_map_key_value_types`. All three are live (`v2_map_emission.rs` populators + readers; consumed in `identifier_concrete_type` `type_resolution.rs:2688/2723`). **The roadmap must still delete these three.**
- **SB-1 producer-vs-consumer correction.** The audit says the engine "currently excludes" PropertyAccess from its export. At HEAD the **engine DOES record PropertyAccess** — `infer_expr` wraps `infer_expr_inner` and unconditionally inserts every non-dummy-span expr (incl. the PropertyAccess arm at `expressions.rs:306`) into `expr_type_table` (`expressions.rs:165-174`). The exclusion is on the **CONSUMER** side: `BytecodeCompiler::infer_expr_type` deliberately skips the span-table consult for `Expr::PropertyAccess` (`expressions/mod.rs:1558`) to preserve the STAGE-F1 strictness ruling (unannotated empty-`[]` field read). This matters for the SB-8 grouping below.
- **The T1 keystone span-table already exists and is consulted FIRST.** `BytecodeCompiler.resolved_expr_types: HashMap<Span, Type>` (`mod.rs:891`), harvested via `inference.take_expr_type_table()` (`compiler_impl_reference_model.rs:823`), assigned at `:2062`, consulted at the TOP of `infer_expr_type` (`expressions/mod.rs:1558-1577`). The engine-fallback U4-deletion target is now at **`expressions/mod.rs:2187`** (`self.type_inference.infer_expr(expr)`), audit said `:2188` — off by one. The `.strip_suffix("[]")` is at **`:1632/:2234`** (audit `:1635`); the `strip_prefix("Array<")/Vec<` re-parse is at **`:2230-2231`** (audit `:2230`).
- **New SB-8-class table not in the audit's named list:** `inferred_param_fn_param_types` (`mod.rs:1406`, Wave-1a PART B) — a 14th projection.

---

## 1. Per-table inventory

For every field, format is: **field def site** → key→value · **populator(s)** · **reader(s)** · **engine fact duplicated** · **STATUS** · **deletion difficulty**.

### Group A — whole-binding ConcreteType tables (the `identifier_concrete_type` head of the ladder)

**A1. `current_function_local_concrete_types`** — `mod.rs:1684` · `HashMap<u16, shape_value::v2::ConcreteType>`
- **Populators (writes):** `statements.rs:5959` (annotated `let`, via `declared_annotation_concrete_type`), `statements.rs:6011` (inferred `let`, via `concrete_type_for_expr`), `functions.rs:1618` (param seeding), `helpers.rs:4158` (array-upgrade in `record_*_array_element`), `v2_typed_emission.rs:1155` (empty-array accumulator), `patterns/binding.rs:297/587`, `loops.rs:442/884`. Snapshot/restore (mem::take + restore) at `monomorphization/cache.rs:432/438/623/626/840/843`; cleared `functions.rs:1494`.
- **Readers:** `type_resolution.rs:2679` (`identifier_concrete_type`, FIRST consult), `expressions/mod.rs:1652` (identifier value-type), `expressions/mod.rs:2003` (tuple-index `arr[k]`), `expressions/binary_ops.rs:1200` (`?? `-Option carrier detect).
- **Engine fact duplicated:** the resolved `Type` of the binding's initializer expression — i.e. the engine's `resolved_expr_types[init_expr.span]` projected to `ConcreteType`. Frozen at let-compile time because the engine ran at module scope and never saw the function-body local.
- **STATUS:** LIVE, load-bearing (most-read of the concrete tables). U1 changed nothing here.
- **Difficulty:** HIGH — 4 distinct reader call paths, 6 writers, snapshot/restore plumbing.

**A2. `module_binding_concrete_types`** — `mod.rs:1710` · `HashMap<u16, ConcreteType>`
- **Populators:** `statements.rs:5551/5597` (module-binding `let`, annotated + inferred), `helpers.rs:4182` (array-upgrade), `v2_typed_emission.rs:1163` (empty-array accumulator).
- **Readers:** `type_resolution.rs:2710` (`identifier_concrete_type`, module branch FIRST consult), `expressions/mod.rs:1656`, `expressions/mod.rs:2007`, `expressions/binary_ops.rs:1204`. Mirror of A1 for module-binding slots.
- **Engine fact:** same as A1 (top-level binding initializer `Type`).
- **STATUS:** LIVE. **Difficulty:** HIGH (symmetric to A1).

### Group B — array-element ConcreteType tables

**B1. `array_element_types`** (span-keyed) — `mod.rs:1237` · `HashMap<Span, ConcreteType>`
- **Populator:** `v2_map_emission.rs:155` (`record_array_element_type_for_span`). **Reader:** `v2_map_emission.rs:160` (`array_element_type_for_span`). NOTE the audit-cited grep count of 72 was substring noise; the actual `self.array_element_types` (span) sites are exactly these 2.
- **Engine fact:** element `Type` of an array-producing expression keyed by AST span (the engine's `expr_type_table[span]` would carry `Array<elem>`).
- **STATUS:** LIVE but NEARLY DEAD — exactly one writer + one reader, both inside `v2_map_emission.rs`. **Difficulty:** LOW.

**B2. `local_array_element_types`** — `mod.rs:1240` · `HashMap<u16, ConcreteType>`
- **Populators:** `helpers.rs:4149`, `v2_typed_emission.rs:1157`, `statements.rs:6085`, `patterns/binding.rs:591`. **Readers:** `helpers.rs:4146` (upgrade-guard), `type_resolution.rs:2685` (`identifier_concrete_type`, consulted AFTER `current_function_local_concrete_types`).
- **Engine fact:** element `Type` of an array local (subset of A1's `Array<elem>` — a frozen projection of just the element).
- **STATUS:** LIVE. **Difficulty:** MEDIUM (one cross-module reader in the priority ladder).

**B3. `module_binding_array_element_types`** — `mod.rs:1243` · `HashMap<u16, ConcreteType>`
- **Populators:** `helpers.rs:4173`, `v2_typed_emission.rs:1165`, `statements.rs:5657`. **Readers:** `helpers.rs:4170`, `type_resolution.rs:2716` (after `module_binding_concrete_types`).
- **STATUS:** LIVE. Mirror of B2. **Difficulty:** MEDIUM.

### Group C — HashMap key/value ConcreteType tables (SURVIVED U3 — flag)

**C1. `map_key_value_types`** (span) — `mod.rs:1222` · `HashMap<Span,(ConcreteType,ConcreteType)>` · pop `v2_map_emission.rs:30` · read `v2_map_emission.rs:75`.
**C2. `local_map_key_value_types`** — `mod.rs:1228` · `HashMap<u16,(CT,CT)>` · pop `v2_map_emission.rs:40`, `patterns/binding.rs:595` · read `v2_map_emission.rs:59`, `type_resolution.rs:2688`.
**C3. `module_binding_map_key_value_types`** — `mod.rs:1232` · pop `v2_map_emission.rs:50` · read `v2_map_emission.rs:67`, `type_resolution.rs:2723`.
- **Engine fact:** the `HashMap<K,V>` `Type` of a map binding/expression (key+value projection).
- **STATUS:** LIVE — **NOT deleted by U3** (U3 deleted the runtime `TypedMap` carrier only). **Difficulty:** LOW-MEDIUM (each has ≤2 readers; one reader is the `identifier_concrete_type` ladder).

### Group D — per-function param/return projections (stringly + structural)

**D1. `inferred_param_type_hints`** — `mod.rs:1371` · `HashMap<String, Vec<Option<String>>>` (STRINGLY — SB-7)
- **Populator:** `compiler_impl_reference_model.rs:825/2067` (from `infer_param_type_hints_from_types(program,&types)`). **Readers:** `functions.rs:312` (`compile_function_body` param seeding), `functions.rs:1697`. Referenced in rationale-comments at `binary_ops.rs:679/1584/2323`, `expressions/mod.rs:1978` (the `arr[i]` element-recovery patch reads the tracker `type_name` this stamps).
- **Engine fact:** the engine's per-param resolved `Type`, rendered to a **string** then re-parsed downstream (`strip_suffix("[]")` at `expressions/mod.rs:2234`). Pure SB-7 stringly drift.
- **STATUS:** LIVE. **Difficulty:** MEDIUM (stamps the tracker `type_name`, which is read indirectly all over the `[]`/`Array<` re-parse paths).

**D2. `inferred_param_concrete_types`** — `mod.rs:1383` · `HashMap<String, Vec<Option<ConcreteType>>>`
- **Populator:** `compiler_impl_reference_model.rs:831/2068` (`infer_param_concrete_types_from_types`). **Reader:** `compiler_impl_reference_model.rs:2705` (MIR param-slot seeding when slot is `ConcreteType::Void`).
- **Engine fact:** per-unannotated-param resolved `Type` → `ConcreteType` (the JIT typed-array-fastpath proof). Strict subset of the engine's param solve.
- **STATUS:** LIVE. **Difficulty:** LOW (single reader).

**D3. `inferred_param_fn_param_types`** (NOT in audit) — `mod.rs:1406` · `HashMap<String, Vec<Option<Vec<TypeAnnotation>>>>`
- **Populator:** `compiler_impl_reference_model.rs:836/2069` (`infer_param_fn_param_types_from_types`). **Reader:** `function_calls.rs:2236` (`install_pending_closure_param_types_for_inferred_fn_param`).
- **Engine fact:** per-param inferred `Function<A,R>` arg annotations (fn-typed unannotated params). Projection of the engine's function-type solve.
- **STATUS:** LIVE. **Difficulty:** LOW (single reader).

**D4. `inferred_param_object_fields`** — `mod.rs:1423` · `HashMap<String, Vec<Option<Vec<(String,FieldType)>>>>`
- **Populator:** `compiler_impl_reference_model.rs:842/2070` (`infer_param_object_fields_from_types`). **Reader:** `functions.rs:313` (inline anon-schema registration for anon-object params).
- **Engine fact:** per-param anonymous-object structural `Type` → field list. Projection of param solve where the resolved type is an anon object.
- **STATUS:** LIVE. **Difficulty:** LOW-MEDIUM (single reader, but feeds schema registration plumbing).

**D5. `inferred_return_object_fields`** — `mod.rs:1435` · `HashMap<String, Vec<(String,FieldType)>>`
- **Populator:** `compiler_impl_reference_model.rs:847/2071` (`infer_return_object_fields_from_types`). **Readers:** `compiler_impl_reference_model.rs:1046` (`register_inferred_return_object_schemas`, the consumer that builds `function_return_schema_ids`).
- **Engine fact:** per-function inferred anon-object RETURN `Type` → field list.
- **STATUS:** LIVE. **Difficulty:** LOW (one reader; feeds D6).

**D6. `function_return_schema_ids`** — `mod.rs:1444` · `HashMap<String, u32>` (derived from D5)
- **Populator:** `compiler_impl_reference_model.rs:1072` (`register_inferred_return_object_schemas`). **Readers:** `v2_typed_emission.rs:626`, `expressions/mod.rs:2320/2324` (`f(...).field` resolution), `function_calls.rs:466`.
- **Engine fact:** a registered-schema handle for the engine's anon-object return type. Second-order projection of D5.
- **STATUS:** LIVE. **Difficulty:** MEDIUM (4 readers, but all narrow `f(...).field` lookups).

**D7. (TypeTracker) `function_return_types`** — `type_tracking.rs:668` · `HashMap<String,String>` (STRINGLY — SB-7; the audit's "`inferred_return_type_hints`")
- **Populator:** `compiler_impl_reference_model.rs:2085-2087` (loop over the transient `inferred_return_type_hints` local → `register_function_return_type`, `type_tracking.rs:828`). **Readers:** `expressions/mod.rs:1710` (`get_function_return_type` in `infer_expr_type` FunctionCall arm), `functions_annotations.rs:1102`, `expressions/collections.rs:390` (comment). Getter `type_tracking.rs:834`.
- **Engine fact:** the engine's per-function return `Type`, rendered to a **string**. SB-7 stringly drift.
- **STATUS:** LIVE. **Difficulty:** MEDIUM (read directly by the strict-typing binop dispatch via `infer_expr_type`).

### Group E — callable-binding return-type tables (closures via slots, STRINGLY)

All four are `HashMap<u16, String>` (SB-7 stringly). Populated/cleared together by `update_callable_binding_from_expr` / `clear_callable_binding` in `helpers_reference.rs`. Snapshot/restore in `functions.rs` (per-function compile save/restore).

**E1. `local_callable_return_types`** — `mod.rs:771`
- **Populator:** `helpers_reference.rs:1103` (insert) / `:1106`,`:1172` (remove). Snapshot `functions.rs:1373/1484/1947/2078`. **Reader:** `expressions/mod.rs:1720` (`f(...)` call-result type in `infer_expr_type`), `function_calls.rs:795`.
- **Engine fact:** return `Type` of a `let f = |…| …` local closure (string-rendered). The engine's closure-body solve.
- **STATUS:** LIVE. **Difficulty:** MEDIUM.

**E2. `module_binding_callable_return_types`** — `mod.rs:775` · pop `helpers_reference.rs:1144` · read `expressions/mod.rs:1727/1737`, `function_calls.rs:799/803`. Mirror of E1 for module-binding closures. **STATUS:** LIVE. **Difficulty:** MEDIUM.

**E3. `local_array_callable_return_types`** — `mod.rs:784` · pop `helpers_reference.rs:952/1109` · read **`expressions/mod.rs:1838`** (the `arr[i](args)` = `MethodCall{method:"__call__"}` arm). Snapshot `functions.rs:1375/1949/2079`.
- **Engine fact:** homogeneous element return `Type` of an array-of-closures local.
- **STATUS:** LIVE (I initially thought dead — the reader is the non-obvious `__call__` arm). **Difficulty:** LOW-MEDIUM (single narrow reader).

**E4. `module_binding_array_callable_return_types`** — `mod.rs:788` · pop `helpers_reference.rs:960/969/1150/1153` · read `expressions/mod.rs:1848/1858`. Mirror of E3. **STATUS:** LIVE. **Difficulty:** LOW-MEDIUM.

### Group F — name-keyed structural side-channels (not in audit's named list)

**F1. `binding_object_element_fields`** — `mod.rs:1254` · `HashMap<String, Vec<ObjectTypeField>>` · pop `helpers.rs:4237` · read `loops.rs:1813` (for-in `for {x,y} in points` destructuring over an array-of-anon-object-literals binding).
- **Engine fact:** element anon-object field `Type`s of an array binding; frozen at let-time because the engine's scope is popped.
- **STATUS:** LIVE. **Difficulty:** LOW (single reader).

**F2. `binding_collection_carrier_kinds`** — `mod.rs:1724` · `HashMap<String, ConcreteType>` · pop `statements.rs:5196` (single writer) · read `type_resolution.rs:1605` (`binding_collection_ctor_capture_type`, consumed by `resolve_capture_concrete_type` for §2.7.8 closure-capture kind).
- **Engine fact:** collection-carrier ConcreteType of a bare-ctor binding (`let mut m = HashMap()`), **deliberately separated** from `module_binding_concrete_types` (its comment, `mod.rs:1720-1723`, explains the `HasField` constraint collision that forced the split — a documented drift).
- **STATUS:** LIVE. **Difficulty:** LOW (1 writer, 1 reader) but **semantically load-bearing for capture-kind soundness** — must be migrated, not just deleted.

---

## 2. Consult-priority drift map (where two tables can disagree about the same binding)

There are **two** independent priority ladders. Both must collapse to a single span-table lookup.

### Ladder 1 — `identifier_concrete_type` (`type_resolution.rs:2675-2741`), local then module:
```
LOCAL slot:
  1. current_function_local_concrete_types[idx]        (A1)   ← wins
  2. local_array_element_types[idx]  → Array(elem)     (B2)
  3. local_map_key_value_types[idx]  → HashMap(k,v)    (C2)
  4. type_tracker.get_local_type(idx).type_name        (SB-7 stringly)
        → concrete_type_from_type_name  (re-parse "Vec<int>")
MODULE binding:
  1. module_binding_concrete_types[idx]                (A2)   ← wins
  2. module_binding_array_element_types[idx]           (B3)
  3. module_binding_map_key_value_types[idx]           (C3)
  4. type_tracker.get_binding_type(idx).type_name      (SB-7 stringly re-parse)
```
**Drift point (documented in code):** `helpers.rs:4135-4160` — the array-element writer must "upgrade in lockstep" a stale placeholder `Array(_)` sitting in A1/A2 because "`identifier_concrete_type` consults the whole-binding `*_concrete_types` table FIRST (before the element side-table), so a stale placeholder array there shadows our named element unless we upgrade it." This is the canonical SB-8 disagreement: **A1 vs B2 can hold different element types for the same slot, and a manual lockstep-upgrade hack patches it.** The level-4 tracker-stringly fallback can also disagree with levels 1-3 (re-parse of `type_name` vs the stored ConcreteType).

### Ladder 2 — `infer_expr_type` (`expressions/mod.rs:1522-2192`), top-to-bottom:
```
 0. resolved_expr_types[span]               (T1 KEYSTONE span-table)  :1561  ← already FIRST; skipped for PropertyAccess (:1558) & Borrow (:1573)
 1. reference_referent_type_name(name)       (GapA)                    :1594
 2. tracker_type_name_for_identifier(name)   (SB-7 stringly + strip_suffix "[]")  :1602/:1632
 3. current_function_local_concrete_types / module_binding_concrete_types (A1/A2)  :1652/:1656
 4. type_tracker.get_function_return_type     (D7 stringly)            :1710
 5. local_callable_return_types               (E1 stringly)           :1720
 6. module_binding_callable_return_types       (E2 stringly)          :1727/:1737
 7. local/module_binding_array_callable_return_types (E3/E4 __call__) :1838/:1848/:1858
 8. current_function_local/module_binding_concrete_types (tuple-index) (A1/A2)  :2003/:2007
 9. concrete_type_for_expr → identifier_concrete_type (Ladder 1)      :2150/:2173
10. self.type_inference.infer_expr(expr)      ← U4 DELETION TARGET     :2187
```
**Drift point:** step 0 (engine span-table) and steps 2-9 (the 13 tables) are **independent frozen views of the same fact.** A binding whose engine span-table entry resolved correctly but whose tracker `type_name`/ConcreteType froze a stale or coarser value will get a DIFFERENT answer depending on which step fires first. Today step 0 wins when it has a fully-resolved entry; the patches only run on a step-0 MISS — that miss is exactly what U4 wants to convert into a surface-and-stop at step 10.

---

## 3. Replace-by-engine-query grouping (which engine fact each table becomes once the span-table is complete)

Once the engine span-table is complete (every expr recorded, PropertyAccess strictness moved INTO the engine as a real error), all 14 tables collapse to **one query**: `resolved_expr_types[span]` (or, for slot-keyed cases, `resolved_expr_types[binding-defining-expr.span]`). Grouped by the engine sub-fact:

| Engine query that replaces it | Tables | Notes |
|---|---|---|
| **`expr_type_table[init_expr.span]`** (binding initializer resolved Type) | A1, A2, B1, B2, B3, C1, C2, C3, F1, F2 | All whole-binding / element / map / element-object tables are frozen projections of the binding's initializer expr type. Once the span-table carries the function-body local's init type (the reason it was added in T1), these are pure duplication. F2 needs the §2.7.8 capture-kind derived from the same Type. |
| **engine per-param resolved Type** (`infer_param_*_from_types` already projects this — delete the projection, query the engine slot directly) | D1, D2, D3, D4 | D1 is the stringly version (drives the `strip_suffix("[]")` re-parse — delete with the re-parse). D2/D3/D4 are structural projections of the same param solve. |
| **engine per-function return Type** | D5, D6, D7, E1, E2, E3, E4 | D7/E1-E4 are the stringly `HashMap<_,String>` versions (the SB-7 drift). D5/D6 are the anon-object-return structural projection + its registered-schema handle. All are the engine's function/closure-body return solve, rendered/frozen. |

**SB-7 collapse coupling:** D1, D7, E1-E4 are exactly the `type_name: Option<String>` (`type_tracking.rs:340`) stringly representation the U4 plan replaces with the engine `Type`. Deleting them removes the `.strip_suffix("[]")` (`expressions/mod.rs:1632/2234`) and the `strip_prefix("Array<")/Vec<` re-parse (`:2230-2231`) — the two re-parse sites the audit named. `NumericType` (`type_tracking.rs:52`, register `last_expr_numeric_type` at `mod.rs:726`, read for opcode selection at `binary_ops.rs:796` etc.) stays as the **emit-time stamp derived from the one Type**, per the U4 target — it is NOT one of these hint tables.

---

## 4. Deletion-difficulty ranking (nearly-dead → load-bearing)

**Tier 1 — nearly dead / single narrow reader (delete first):**
- `array_element_types` (B1) — 1 writer + 1 reader, both in `v2_map_emission.rs`.
- `inferred_param_concrete_types` (D2), `inferred_param_fn_param_types` (D3), `inferred_param_object_fields` (D4), `inferred_return_object_fields` (D5) — each exactly 1 reader; all are direct `infer_*_from_types` projections you can replace with a per-slot engine query.
- `binding_object_element_fields` (F1) — 1 writer, 1 reader.
- `binding_collection_carrier_kinds` (F2) — 1 writer, 1 reader, BUT soundness-coupled (capture kind); migrate carefully.

**Tier 2 — single-purpose but in a priority ladder:**
- `local_array_element_types` (B2), `module_binding_array_element_types` (B3) — in Ladder-1 + the lockstep-upgrade hack (`helpers.rs:4141-4160`).
- `local_map_key_value_types` (C2), `module_binding_map_key_value_types` (C3), `map_key_value_types` (C1) — survived U3; in Ladder-1.
- `function_return_schema_ids` (D6) — 4 narrow `f(...).field` readers, depends on D5.
- `local_array_callable_return_types` (E3), `module_binding_array_callable_return_types` (E4) — single `__call__`-arm reader each.

**Tier 3 — load-bearing (delete last, after the span-table demonstrably covers function bodies):**
- `current_function_local_concrete_types` (A1), `module_binding_concrete_types` (A2) — head of Ladder-1, 4 reader paths each, 6 writers, snapshot/restore plumbing in `monomorphization/cache.rs`.
- `inferred_param_type_hints` (D1) — stamps tracker `type_name`, read indirectly across all the `[]`/`Array<` re-parse sites; SB-7 keystone.
- `function_return_types` (D7, in TypeTracker), `local_callable_return_types` (E1), `module_binding_callable_return_types` (E2) — read by the strict-typing binop dispatch via `infer_expr_type` FunctionCall arm; closure-body return solve; SB-7 stringly.

**Load-bearing structural caveat for the roadmap:** the SB-8 tables exist because the engine ran ONCE at module scope and could not see function-body locals; the T1 keystone span-table (`resolved_expr_types`) was the fix that lets the engine's full-program walk reach those locals. Therefore the precondition for deleting ALL of Group A/B/C/F is **the span-table must carry every function-body binding's defining-expr type** (verify the engine's `infer_program_best_effort` walks into function bodies and `finalize_expr_type_table` keeps those entries). The deliberate PropertyAccess consumer-exclusion (`expressions/mod.rs:1558`) and the STAGE-F1 strictness ruling must be **moved INTO the engine as a real error** before that exclusion can be dropped — exactly the U4 "move STAGE-F1 strictness INTO the engine" sub-goal.",
      "dRepros": "I have everything needed. The side-tables exist and have actually GROWN (now ~16+ including `local_callable_closure_bodies`, `binding_collection_carrier_kinds`, the pass-modes pair). The object-fields tables live in `compiler_impl_initialization.rs` (declared elsewhere in mod.rs as non-HashMap or in a sub-struct). Let me compile the final report.

# U4 Concrete Failure Repros — Verified Against HEAD `18991310`

## 0. Environment & verification baseline

- **Worktree:** `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch`, branch `strict-flip-collection-dispatch`, HEAD `18991310` ("harden(R6 guard)…").
- **Binary:** rebuilt at HEAD via `direnv exec /home/dev/dev/shape-lang cargo build --release -p shape-cli` (finished clean). All runs are `bash -c "ulimit -v 12582912; timeout 20 ./target/release/shape run --mode vm <file>"`.
- **Cosmetic noise filtered:** every program prints a `V2 bytecode verification failed: … 'Json.keys' … no FrameDescriptor` warning. That is **SB-22** (the universal JIT-verifier off-switch), NOT a U4 symptom — the VM still runs and produces output. I filter it from the captures below.
- **Closure syntax note (affects repro authoring, not U4):** Shape closures are `|p: T| { body }`. A `-> T` return annotation on a closure literal is a **parse error** (`H3` below). This matters for U4 because it means the strict-typing error's own suggested fix ("add a type annotation") is *unavailable at the closure boundary* — you can only annotate the consuming `let` (see `h2cast`).

---

## 1. Audit-citation drift (what CHANGED vs STRUCTURAL-AUDIT.md)

The audit was written at HEAD `bea569cc`. Verified deltas at `18991310`:

| Audit claim | Audit cite | At HEAD `18991310` | Status |
|---|---|---|---|
| Empty fallback engine `self.type_inference.infer_expr` | `expressions/mod.rs:2188` | `expressions/mod.rs:2187` | **CONFIRMED** (off by 1) |
| `infer_expr_type` ~26-arm ladder | — | `expressions/mod.rs:1522`–`2193`; ladder has **GROWN** (now T1-keystone span-table consult at `:1558`–`1578` first, then ~30 fall-through arms) | **CONFIRMED, larger** |
| `.strip_suffix("[]")` re-parse | `:1635` | `expressions/mod.rs:1632` | **CONFIRMED** (off by 3) |
| `Array<`/`Vec<` strip re-parse | `:2230` | `expressions/mod.rs:2230` (in `tracked_array_element_type`) | **CONFIRMED exactly** |
| tracker `type_name: Option<String>` | `type_tracking.rs:338` | `type_tracking.rs:340` (`:338` is now `schema_id`) | **CONFIRMED** (off by 2) |
| `NumericType` enum | `type_tracking.rs:52` | `type_tracking.rs:54` | **CONFIRMED** |
| Engine span-table `resolved_expr_types` | (post-audit feature) | `mod.rs:891`; populated `compiler_impl_reference_model.rs:823,2057`; engine side `inference/mod.rs:268` (`expr_type_table`), recorded `inference/expressions.rs:172`, finalized `inference/mod.rs:361` | **NEW since audit** — the T1 keystone landed 2026-06-22 |
| **SB-8b `prove_native_kind` is a no-op stub** | `type_tracking.rs:1251` | `type_tracking.rs:1261`: now a **REAL exact-equality check** (`native_kind_from_concrete_type` → `kinds_consistent` exact-eq, `:1290`), with regression tests `_rejects_sb10_uint64_for_hashmap`, `_rejects_sb12_bool_for_null`, `_does_not_unify_int_and_number` (`:1360`–`1385`) | **CHANGED — SB-8b FIXED** (U2 landed). Not a U4 concern but corrects the audit's headline. |
| **SB-1 "PropertyAccess excluded from keystone"** | engine excludes field-reads | **MOVED**: the engine `infer_expr` (`inference/expressions.rs:165`) records **EVERY** expression including PropertyAccess. The exclusion is now on the **consumer side** — bytecode `infer_expr_type` deliberately skips the span-table for `PropertyAccess` (`expressions/mod.rs:1558`) to avoid masking the STAGE-F1 error. | **CHANGED** — the exclusion relocated; see §3/Class 2. |
| **STAGE-F1 strictness "move INTO the engine as a real error"** | audit target (TODO) | **ALREADY DONE**: it is a real `TypeError::ConstraintViolation` in the engine at `constraints.rs:1137` | **CHANGED — partly done** |
| SB-8 ~13 hint side-tables | `mod.rs:771-788/1263-1269/1410/1398/1711/1737` | tables **GROWN, not shrunk**; some renamed (see §5). U3 did NOT delete these (it deleted map carriers). | **CONFIRMED present, count up** |

**Bottom line for the roadmap author:** U1/U2 cleared the *L4 proof gate* (SB-8b) and added the *engine span-table* (the U4 target authority already exists and is consulted first). What U4 must still do is (a) DELETE the fallback engine + the ~30-arm ladder + the side-tables, and (b) resolve the PropertyAccess consumer-side exclusion + the closure-body mini-inferencer. The structural split-brain is intact; the repro surface has *shrunk* to a sharp residue around field-reads-through-closures.

---

## 2. Per-class repro results (the 6 claimed classes)

| Class | Minimal program | Result at HEAD | Verdict |
|---|---|---|---|
| **1. int/number through closures** | `/tmp/c1.shape`, `/tmp/c1n.shape` | `24`, `24.0` (correct) | **FIXED** (T1 keystone + callable-return side-tables) |
| **2. field-read erasure** (`rs[0].n`, `self.field`, `f(...).field`) | `/tmp/f2,f3c,f4,f5.shape` | all correct; the un-annotated-push form is a **deliberate** error | **MOSTLY FIXED** — residue = closure-returns-field (see Class 5) |
| **3. inline-vs-let asymmetry** | `/tmp/c3inline.shape`, `/tmp/c3let.shape` | both `10` (identical) | **FIXED** |
| **4. string-method return loss** | `/tmp/c4.shape` | `HELLO!` (correct) | **FIXED** (hard-coded STAGE-S5 arm `expressions/mod.rs:1787`) |
| **5. closure-call return loss** | `/tmp/c5.shape` / `/tmp/f8.shape` | array-of-closures = deliberate refusal; **closure-returns-field = LIVE BUG** | **PARTIALLY LIVE** — see §3 |
| **6. over-broad / lost inferred types** | `/tmp/c6.shape` | `200` (correct) | **FIXED** (`.map(|e|…)` → keystone span-table) |

### Class 1 — int/number through closures (FIXED)
```shape
fn main() {
  let f = |x: int| { x * 2 }
  let r = f(5) + f(7)
  print(r)
}
main()
```
→ `24`. The `number` variant (`|x: number| { x * 2.0 }`, `g(5.0)+g(7.0)`) → `24.0`. Both operands resolve via `local_callable_return_types` (`expressions/mod.rs:1719`) and the engine span-table. **No longer repros.** What changed: the T1 keystone span-table consult (`expressions/mod.rs:1561`) + the closure-return side-table.

### Class 3 — inline-vs-let asymmetry (FIXED)
```shape
fn hour() -> int { 9 }
fn main() { print(hour() + 1) }   // inline
```
Both inline (`hour() + 1`) and let (`let h = hour(); h + 1`) → `10`. Root historically: inline operand never got the let-reconciliation pass. **Fixed** by the ROOT-2 inline-FunctionCall arm (`expressions/mod.rs:2147`) + inline-MethodCall arm (`:2119`) calling `concrete_type_for_expr`. No longer repros.

### Class 4 — string-method return loss (FIXED)
```shape
fn main() { let s = "hello"; let r = s.toUpperCase() + "!"; print(r) }
```
→ `HELLO!`. Fixed by the hard-coded STAGE-S5 string-method arm (`expressions/mod.rs:1787`–`1819`, an explicit method-name allowlist `charAt|slice|…|toUpperCase|…`). **Note for roadmap:** this is itself a hard-coded side-ladder arm that U4 should subsume into the engine — it is a *frozen projection* of the method registry's return types, exactly the SB-8 pattern.

### Class 6 — over-broad / lost inferred types (FIXED)
```shape
type Emp { salary: int }
fn main() {
  let roster = [Emp { salary: 100 }, Emp { salary: 200 }]
  let sals = roster.map(|e| e.salary)
  let mut mx = 0
  for v in sals { if v > mx { mx = v } }
  print(mx)
}
main()
```
→ `200`. The `.map(|e| e.salary)` result element type reaches `for v in sals { if v > mx }` via the keystone span-table (the for-binder `v` is not a PropertyAccess, so it is served). No longer repros.

---

## 3. Class 2 + Class 5 — the LIVE U4 bug (root R2), fully isolated

**The single surviving U4 failure class is: a closure whose body is a field-read (`PropertyAccess`), whose return type then erases to `unknown`.** Isolated below.

### Minimal repro (copy-pasteable)
`/tmp/f8.shape`:
```shape
type Emp { salary: int }
fn main() {
  let e = Emp { salary: 50 }
  let get = |p: Emp| { p.salary }
  print(get(e) + 1)
}
main()
```
**Observed:**
```
error[SEMANTIC]: Cannot infer types for binary operation `Add`: operand types are `unknown` and `int`.
Strict typing requires both operands to have a known concrete type at compile time.
  --> <input>:5:12
 5 |   print(get(e) + 1)
```

### Discriminating probes (which axis is responsible)
| Probe | Program shape | Result | Conclusion |
|---|---|---|---|
| `f8scalar` | `let get = |p: int| { p }; get(5)+1` | `6` ✅ | closure-call return works when body is a scalar |
| `f8scalar2` | `|p: int| { p * 2 }; get(5)+1` | `11` ✅ | …and for scalar expressions |
| `g1` | `|a: Array<int>| { a.sum() }; f(xs)+1` | `7` ✅ | …and for MethodCall bodies (mini-inferencer has a MethodCall arm) |
| **`f8`** | **`|p: Emp| { p.salary }; get(e)+1`** | **❌ `unknown + int`** | **field-read body → return type lost** |
| **`f8let`** | `let s = get(e); s + 1` | **❌** | not fixed by a `let` (the let records the unproven type) |
| **`h1`** | `|w: Outer| { w.inner.x }; getx(o)+1` | **❌** | nested field too |
| **`h2`** | `|p: Emp| { return p.salary }; get(e)+1` | **❌** | explicit `return` too (Return arm recurses into PropertyAccess → None) |
| `h3` | `|p: Emp| -> int { p.salary }` | **parse error** | you cannot annotate the closure to escape it |
| `h2cast` | `let s: int = get(e); s + 1` | `51` ✅ | annotating the *consuming let* is the only workaround |
| `h5` | named `fn get(p: Emp) -> int { p.salary }` | `51` ✅ | named fns work — declared `-> int` flows via `function_return_types` |
| `h4` | `get(e) == 50` (one literal sibling) | `yes` ✅ | `==` recovers from the *sibling* literal operand |
| **`h4b`** | `get(a) == get(b)` (both closure-field) | **❌ `unknown == unknown`** | confirms the type really is lost; `h4` only "passed" via sibling recovery |

### Root attribution (read the code)

Three structures conspire — all three are U4's named targets:

1. **The closure-body return type is computed by a hand-written mini-inferencer**, NOT the engine: `infer_closure_body_return_type_name_with_caller_context` (`crates/shape-vm/src/compiler/expressions/closures.rs:830`). Its inner `expr_type` walker (`closures.rs:936`–`1131`) has arms for `Literal`, `Identifier`, `BinaryOp`, `UnaryOp`, `Return`, `Block`, `MethodCall` — and **NO `PropertyAccess`/`FieldAccess` arm**, so a field-read body falls to `_ => None` at `closures.rs:1130`. This walker is a *fourth* re-implementation of inference (on top of SB-1's three) and a *stringly-typed* one (`Option<String>` of `"int"`/`"number"`/`"Vec<int>"…`, with its own `strip_prefix("Vec<")` re-parse at `closures.rs:1104`). **This IS SB-1 + SB-7 in microcosm.**

2. **The result is stored in the side-table** `local_callable_return_types: HashMap<u16, String>` (`mod.rs:771`), populated by `update_callable_binding_from_expr` (`helpers_reference.rs:910`–`921`, the `FunctionExpr` arm). When the mini-inferencer returns `None`, the slot gets `.remove(&slot)` (`helpers_reference.rs:1106`) — the binding has *no* recorded return type. **This is SB-8.**

3. **At the use site**, `infer_expr_type`'s `FunctionCall` arm consults `local_callable_return_types` (`expressions/mod.rs:1719`), misses, and falls through. The engine span-table *would* hold the call's type — but the call `get(e)` is a `FunctionCall`, which IS consulted (not the excluded PropertyAccess), so why does it still miss? Because `finalize_expr_type_table` (`inference/mod.rs:361`) **drops any entry that stayed a free variable**, and the module-scope engine never bound the closure-local param `p: Emp`'s field projection to a concrete type at that span (the engine walks the closure body but the call-result var stays free post-solve for this shape). So the span-table entry is *dropped*, the ladder's callable-return side-table is *empty*, and the operand erases. **This is SB-1 (fallback engine returns unknown for the body local) made visible.**

### Why `==` differs from `+` (a real asymmetry worth a regression case)
`h4` (`get(e) == 50`) passes but `h4b` (`get(a) == get(b)`) fails. The `==` emitter (`binary_ops.rs:1006`, `has_eq_impl`) can pick a typed `Eq*` opcode from *either* operand; one literal sibling (`50`) suffices. With both operands closure-field (`h4b`), neither is proven → `unknown == unknown`. `+` has no such sibling-recovery, so `f8` fails on the first field operand. Both should be in the U4 acceptance set — they prove the type is genuinely lost, independent of opcode-selection luck.

---

## 4. Additional field-read / int-number repros (varied shapes)

All self-contained and copy-pasteable. These extend the regression set:

**R-extra-1 — closure returns nested field** (`/tmp/h1.shape`) — **FAILS**:
```shape
type Inner { x: int }
type Outer { inner: Inner }
fn main() {
  let o = Outer { inner: Inner { x: 5 } }
  let getx = |w: Outer| { w.inner.x }
  print(getx(o) + 1)
}
main()
```
→ `unknown + int`. Same root (`closures.rs:1130` no PropertyAccess arm); deeper nesting confirms it isn't a one-level miss.

**R-extra-2 — closure returns field via explicit `return`** (`/tmp/h2.shape`) — **FAILS**:
```shape
type Emp { salary: int }
fn main() {
  let e = Emp { salary: 50 }
  let get = |p: Emp| { return p.salary }
  print(get(e) + 1)
}
main()
```
→ `unknown + int`. The `Return(Some(inner))` arm (`closures.rs:1038`) recurses into `expr_type(p.salary)` which hits the missing PropertyAccess arm.

**R-extra-3 — both operands are closure-field reads** (`/tmp/h4b.shape`) — **FAILS**:
```shape
type Emp { salary: int }
fn main() {
  let a = Emp { salary: 50 }
  let b = Emp { salary: 60 }
  let get = |p: Emp| { p.salary }
  if get(a) == get(b) { print("eq") } else { print("ne") }
}
main()
```
→ `unknown == unknown`. Proves the value type is lost, not merely opcode-selection.

**Acceptance anchors that must KEEP PASSING (regression guards against re-breaking the working paths):**
- `/tmp/f2.shape` — `let rs: Array<Run> = [Run{n:3}]; rs[0].n + 1` → `4`
- `/tmp/f4.shape` — `make().v + 1` (named-fn-result field) → `8`
- `/tmp/f5.shape` — `o.inner.x + 1` (nested field on identifier) → `6`
- `/tmp/f3c.shape` — `self.count + 1` in a trait `method` → `11`
- `/tmp/g2.shape` — `.map(|p| p.x)` then `xs[0]+xs[1]` → `4`
- `/tmp/c6.shape` — `.map(|e| e.salary)` then `for v in sals` → `200`
- `/tmp/h5.shape` — named `fn get(p: Emp) -> int { p.salary }` → `51`

**Deliberate-error anchors (must stay errors, NOT silently accept — the STAGE-F1 ruling):**
- `/tmp/f1.shape` — `let mut rs = []; rs = rs.push(Run{..}); for r in rs { r.n + 1 }` → must error "annotate the array `let rs: Array<Run> = []`" (engine `constraints.rs:1137`). U4 must keep this as a *real engine error*, not mask it via the span-table. This is precisely why `infer_expr_type` excludes PropertyAccess from the span-table consult (`expressions/mod.rs:1558`) — **the roadmap must preserve this exclusion semantics when it deletes the ladder, or the STAGE-F1 error gets masked.**
- `/tmp/c5.shape` — array-of-closures `[|x:int|{x+1}, …]; arr[0](1)+arr[1](1)` → deliberate refusal "Arrays of function values not yet supported" (a *carrier* gap, SB-14-adjacent, NOT a type-erasure bug). The audit listed this as a live R2 repro; **it is now blocked earlier by an explicit refusal** — correct this in the roadmap.

---

## 5. SB-7 / SB-8 structural confirmations (raw material for the deletion roadmap)

**SB-7 (FOUR representations) — CONFIRMED at HEAD:**
- engine `Type`/`TypeAnnotation` (structural) — consumed via `infer_expr_type` returning `Type`.
- tracker `type_name: Option<String>` (stringly) — `type_tracking.rs:340`; re-parsed by `.strip_suffix("[]")` (`expressions/mod.rs:1632`) and `strip_prefix("Array<"/"Vec<")` (`expressions/mod.rs:2230`).
- `ConcreteType` (shape-value v2) — the `current_function_local_concrete_types` family.
- `NumericType` (`type_tracking.rs:54`) — per-last-expr mutable register `last_expr_numeric_type` (set `binary_ops.rs:195,423`).
- **Dual-read drift CONFIRMED:** `binary_ops.rs` reads `NumericType` for opcode selection (`:530,548,650,769`) AND calls `infer_expr_type` for the Type table (`:221,225,434,438,517,574,1006`) — two sources for "is this operand int or number," exactly as the audit states.

**SB-8 (hint side-tables) — present and GROWN, not shrunk** (`crates/shape-vm/src/compiler/mod.rs`):
- `local_callable_return_types` `:771`, `module_binding_callable_return_types` `:775`, `local_array_callable_return_types` `:784`, `module_binding_array_callable_return_types` `:788`
- `local_callable_pass_modes` `:753`, `module_binding_callable_pass_modes` `:760` (audit didn't list these)
- `local_callable_closure_bodies` `:811`, `module_binding_callable_closure_bodies` `:819` (**NEW** — `ClosureBodyPeek` caches; SB-8 growth)
- `array_element_types` `:1237`, `local_array_element_types` `:1240`, `module_binding_array_element_types` `:1243`
- `inferred_param_type_hints` `:1371`
- `function_return_schema_ids` `:1444`
- `current_function_local_concrete_types` `:1684`, `module_binding_concrete_types` `:1710`
- `binding_collection_carrier_kinds` `:1724` (**NEW** since audit)
- `inferred_param_object_fields` / `inferred_return_object_fields` — still constructed (`compiler_impl_initialization.rs:176-177`), backed by `type_tracker.get_object_field_contract` (`type_tracking.rs:848`); audit's `inferred_param_concrete_types` / `inferred_return_type_hints` names are GONE (renamed/folded).
- `function_return_types` lives in the **tracker** (`type_tracking.rs:668`), read at `expressions/mod.rs:1710` via `get_function_return_type`.

So the SB-8 count is now **~16+ tables**, not 13 — U1/U3/strict-flip *added* projection tables (the closure-body peeks, carrier-kinds, pass-modes) rather than retiring them. **U4's "retire the ~13 hint side-tables" is now a ~16-table deletion**, and the `closures.rs` `expr_type` mini-inferencer (`closures.rs:936`–`1131`) is a *fourth inference engine* the audit didn't separately call out — it must be deleted with the ladder.

**The empty fallback engine** (`self.type_inference.infer_expr`, `expressions/mod.rs:2187`) is still the tail of `infer_expr_type` and is still consulted only after the ~30-arm ladder. Deleting it (→ surface-and-stop) is the U4 keystone; the repros in §3/§4 are the acceptance gate that the span-table + engine-export covers what the ladder/mini-inferencer currently patches.

---

## 6. Summary for the synthesis agent

- **5 of 6 claimed classes no longer repro** (1, 3, 4, 6 fully; 2 mostly) — U1 (canonical Type), U2 (`prove_native_kind` now real), and the 2026-06-22 T1 keystone span-table did most of the work. The audit's repro list is stale on these; verify-pass them as *regression guards*.
- **The one live U4 class is "closure body is a field-read":** `f8`, `f8let`, `h1`, `h2`, `h4b` all fail with `unknown` operand. Root = the hand-written `expr_type` mini-inferencer in `closures.rs:936-1131` lacks a `PropertyAccess` arm → `local_callable_return_types` side-table empty → `infer_expr_type` FunctionCall arm misses → fallback engine returns unknown for the body-local field projection. This is SB-1 + SB-7 + SB-8 in one bug.
- **Two consumer-side subtleties the deletion must preserve:** (a) the PropertyAccess span-table exclusion (`expressions/mod.rs:1558`) exists to NOT mask the real STAGE-F1 engine error (`constraints.rs:1137`) — deleting the ladder must keep STAGE-F1 surfacing; (b) named-fn field-returns already work via declared annotations — the fix is making the engine export the *closure* return type the same way (the engine already walks closure bodies; `finalize_expr_type_table` currently drops the still-free entry — that drop is where the closure-field result dies).
- **SB-8b is FIXED** (not a U4 item) and **SB-22/`Json.keys`** is the cosmetic noise on every run (not a U4 item).

All `/tmp/*.shape` files referenced are written and reproduce as described against `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape`.",
      "eEmit": "I now have all the verified material. Let me compile the final report.

# U4 Facet Report: Emit-time Type → NativeKind path + prove_native_kind status

Verified against HEAD `18991310` (branch `strict-flip-collection-dispatch`). All file:line references confirmed at this commit. The audit predates U1/U3/Miri; deltas vs the audit are flagged inline.

---

## PART 1 — `prove_native_kind`: real-vs-stub, wired-vs-inert, caller list

### 1a. Real or stub? **REAL.** (audit said `:1251`; now at `type_tracking.rs:1261`)

`prove_native_kind(site, proven: &ConcreteType, claimed_kind: NativeKind) -> Result<NativeKind, ProofGap>` (`crates/shape-vm/src/type_tracking.rs:1261`) is a genuine check, NOT a pass-through `Ok(claimed_kind)` stub. Body (`:1266-1278`):
- projects `proven` through the single canonical total map `native_kind_from_concrete_type` (`:1266`),
- requires EXACT equality (`kinds_consistent` is literally `expected == claimed`, `:1289-1292`),
- no relaxation: no int↔number unify, no width-narrow, no Bool-default, no UInt64-for-Ptr.

`ProofGap`'s constructor is module-private via `ProofGapSeal(())` (`:1205-1208`) — emit code cannot fabricate a pass. The mechanical-enforcement claim in CLAUDE.md ("ProofGap's constructor is private to the type-tracking module") holds at HEAD.

So the prior-work note ("body made real but reportedly left with zero production callers") is **CONFIRMED at HEAD** — see 1b.

### 1b. Wired or inert? **INERT — zero production callers.**

`prove_native_kind` is invoked ONLY from its own test module (`type_tracking.rs:1333, 1337, 1341, 1354, 1367, 1380, 1388, 1390`). Every other occurrence in the tree is a doc-comment or module-doc mention. Confirmed by:
- grep across `crates/`, `tools/`, `bin/` — no call outside `#[cfg(test)] mod tests`.
- grep for `prove_native_kind` in `crates/shape-jit/`, `crates/shape-runtime/`, `crates/shape-vm/src/executor/` — **empty**.

No emit path gates on it. The function is dead weight w.r.t. production codegen.

### 1c. The ONE production proof-gate that DOES fire — but bypasses `prove_native_kind`

`proof_gap_unresolved_operand(site, detail) -> ProofGap` (`type_tracking.rs:1316`) — a sibling that mints a `ProofGap` for an UNRESOLVED operand — has exactly **one** production caller:

- `crates/shape-vm/src/compiler/expressions/binary_ops.rs:383`, inside `numeric_operand_proof_gap` (`binary_ops.rs:347`), called from the binop terminal at `binary_ops.rs:2493` and `:2500`.

Critically, this gate does NOT call `prove_native_kind`. It re-derives independently: resolve the operand to a local (`:359`), require it be an untyped param (`:361`), then ask `infer_expr_type` and only fire if the result is `Type::Variable`/`Type::Constrained` (`:374-381`). It is a narrow ad-hoc "untyped-param-with-unresolved-type" guard, NOT the canonical projection check. The real `ConcreteType→NativeKind` equality test in `prove_native_kind` is never reached in production.

### 1d. Full caller inventory

| Symbol | Def | Production callers | Test callers |
|---|---|---|---|
| `prove_native_kind` | `type_tracking.rs:1261` | **NONE** | `type_tracking.rs:1333/1337/1341/1354/1367/1380/1388/1390` |
| `proof_gap_unresolved_operand` | `type_tracking.rs:1316` | `binary_ops.rs:383` (1) | `type_tracking.rs:1895` |
| `numeric_operand_proof_gap` | `binary_ops.rs:347` | `binary_ops.rs:2493`, `:2500` | — |
| `native_kind_from_concrete_type` (the map `prove_native_kind` uses) | `shape-value/src/v2/closure_layout.rs:944` | only `prove_native_kind` (`:1266`) + closure capture-kind derivation `from_capture_types` (`:1049`) | many |

---

## PART 2 — The emit-time Type → NativeKind path(s), with DUPLICATION flagged

There is **no single derivation**. At least **five** distinct routes produce a NativeKind/NumericType at emit time, and the canonical `ConcreteType→NativeKind` map is NOT on any of the compiler's hot emit paths.

### Route A — Numeric binop opcode selection: via `NumericType`, never NativeKind, never the proof gate

The dominant arithmetic path keys entirely off `NumericType` (`type_tracking.rs:53` enum: `Int / IntWidth / Number / Decimal`), not NativeKind:
- operands' `left_numeric`/`right_numeric: Option<NumericType>` are seeded from the mutable per-last-expression register `self.last_expr_numeric_type` (`binary_ops.rs:1518`, `:1890`, `:2316`), then patched from inference (`infer_numeric_pair` → `inferred_type_to_numeric`, `binary_ops.rs:428-440`), local tracking, etc.
- emission: `emit_numeric_binary_with_coercion_inner` (`binary_ops.rs:739`) → `plan_coercion` → `apply_coercion` → `typed_opcode_for(op, result_type: NumericType)` (`numeric_ops.rs:282`). The opcode is chosen from `NumericType` alone.
- **The StorageHint (NativeKind) operands are computed and then THROWN AWAY**: `emit_numeric_binary_with_coercion_trusted` (`binary_ops.rs:718`) computes `lhs_hint`/`rhs_hint` via `storage_hint_for_expr` (`:727-728`) but passes them as `_lhs_hint`/`_rhs_hint` underscore-unused params (`binary_ops.rs:745-746`). So for arithmetic, the NativeKind route and the NumericType route are fully disjoint — concrete SB-7 drift.

`self.last_expr_numeric_type` is the SB-7 "per-last-expression mutable register": **115 write sites** and **65 read sites** across `crates/shape-vm/src/compiler/`. (audit cited `mod.rs:726` as the register; it is a `Compiler` field, heavily mutated.)

### Route B — `infer_storage_hint(type_name: &str)` — stringly NativeKind, the central re-derivation

`VariableTypeInfo::infer_storage_hint(&str) -> Option<StorageHint>` (`type_tracking.rs:555`) derives a NativeKind by **string-matching the type NAME** — via `BuiltinTypes::canonical_numeric_runtime_name` + `storage_hint_for_runtime_numeric` (`:594`) which is a second hardcoded `"i8"=>Int8 … "u64"=>UInt64 …` table. It also string-strips `Option<…>` (`option_inner_type`, `:588-592`).

This is invoked from `VariableTypeInfo::named()` (`type_tracking.rs:397`), so **every binding constructed from a type name re-derives its `storage_hint: NativeKind` from the string**, never from `ConcreteType`. `concrete_numeric_type: Option<String>` (a fourth, stringly type rep on `VariableTypeInfo`, `:355`) is co-derived via `infer_numeric_runtime_name` (`:612`).

`storage_hint_for_expr` (`binary_ops.rs:674`) reads back `info.storage_hint` (`:690`), or hardcodes literal kinds (`Int64`/`Float64`, `:692-695`) — again no ConcreteType.

### Route C — Return-value emission: `NumericType → NativeKind` via a third hardcoded table

`last_expr_numeric_type_to_storage_hint` (`helpers_binding.rs:549`) maps `NumericType → StorageHint`(NativeKind) with its OWN match arms (`Number→Float64`, `Int→Int64`, `IntWidth(w)→…`, `:553-571`). Used by `emit_return_value_with_ownership` (`helpers_binding.rs:480`) and `let_decl_storage_hint` (`:532`). **NOTE the doc-comment lie at `helpers_binding.rs:468-469`**: it claims the host-boundary return kind "comes from … `top_level_frame.return_kind` set by `prove_native_kind`" — but `prove_native_kind` is never called; the kind comes from this hardcoded table cross-checked against Route D.

### Route D — `last_emitted_native_kind()` — opcode→NativeKind reverse-derivation (a FIFTH source)

`last_emitted_native_kind() -> Option<StorageHint>` (`helpers.rs:2209`) recovers a NativeKind by **inspecting the last-emitted OPCODE** (huge match: `AddInt/LoadLocalI64/… → Int64` `:2269-2336`; `AddNumber/… → Float64` `:2339-2347`; `EqInt/Not/… → Bool` `:2350-2382`; `PushConst → push_const_native_kind`; `LoadLocalTrusted/LoadModuleBinding/GetFieldTyped → slot-tag lookups`). It walks back past drop chatter (`:2230-2257`). This is disconnected from any static Type — it's a proof-by-inspection of the bytecode.

Used as the **gate cross-check** in `emit_return_value_with_ownership` (`helpers_binding.rs:494`: the Route-C hint is only accepted if `last_emitted_native_kind()` agrees) and as the primary kind for MethodCall/Match top-level returns (`helpers.rs:3612`, `:3700`, `:3842`).

### Route E — Store/Load opcode selection: `StorageHint → FieldKind → OpCode` (FieldKind = a SIXTH type rep)

`typed_store_local_opcode(hint: StorageHint)` (`helpers.rs:6299`) routes through `storage_hint_to_field_kind` (`helpers.rs:6010`) — a `StorageHint→FieldKind` map — then `FieldKind→OpCode`. Same shape for `typed_load_module_binding_opcode`/`typed_store_module_binding_opcode` (`:6320`, `:6341`). So a SEVENTH conversion layer (`FieldKind`, `shape-value::v2::struct_layout`) sits between NativeKind and the opcode.

### DUPLICATION: TWO disagreeing `ConcreteType → NativeKind` maps

The canonical map exists in TWO copies that **DISAGREE on three arms** — a real correctness drift, not just a clone:

| `ConcreteType` arm | `shape-value/v2/closure_layout.rs:944` | `shape-jit/mir_compiler/types.rs:151` |
|---|---|---|
| `Option(_)` | `Ptr(HeapKind::TypedObject)` (`:970`) | `Ptr(HeapKind::Option)` (`:171`) |
| `Result(_,_)` | `Ptr(HeapKind::TypedObject)` (`:971`) | `Ptr(HeapKind::Result)` (`:170`) |
| `Pointer(_)` | `Ptr(HeapKind::NativeView)` (`:963`) | `UInt64` (`:186`) |
| `Void` | **panics** (`:1003`) | returns `None` (`:214`) |
| return type | `NativeKind` (total) | `Option<NativeKind>` |

The closure_layout copy is the one `prove_native_kind` projects through (`type_tracking.rs:1266`). The JIT copy is used by JIT MIR (`mir_compiler/mod.rs:887`, `types.rs:1143/1147/1179/…`). U4's "one derivation" must reconcile these two before either can be the single source.

---

## PART 3 — Independently-re-derived kind sites (NOT derived from the engine Type)

Every emit-time kind today is independently re-derived; NONE flow from the engine `Type`/`ConcreteType` through the canonical map. The re-derivation entry points to re-point at the single derivation:

1. **`infer_storage_hint(&str)`** `type_tracking.rs:555` — stringly NativeKind from type name. Called by `VariableTypeInfo::named` (`:397`). **The central one.**
2. **`storage_hint_for_runtime_numeric(&str, nullable)`** `type_tracking.rs:594` — second hardcoded name→NativeKind table.
3. **`infer_numeric_runtime_name(&str)`** `type_tracking.rs:612` — stringly `concrete_numeric_type` co-derivation.
4. **`storage_hint_for_expr`** `binary_ops.rs:674` — reads `info.storage_hint` / hardcodes literal `Int64`/`Float64`.
5. **`inferred_type_to_numeric(ty: &Type)`** `numeric_ops.rs:78` — engine `Type` → `NumericType` by string name-matching (`name.as_str()`, `is_integer_type_name`, …), NOT via ConcreteType. 17 call sites (`binary_ops.rs`, `unary_ops.rs`, `numeric_ops.rs`).
6. **`typed_opcode_for(op, NumericType)`** `numeric_ops.rs:282` — opcode from NumericType.
7. **`last_expr_numeric_type_to_storage_hint`** `helpers_binding.rs:549` — third hardcoded NumericType→NativeKind table.
8. **`last_emitted_native_kind`** `helpers.rs:2209` + helpers `push_const_native_kind`, `load_local_trusted_native_kind` (`:2398`), `load_module_binding_native_kind` (`:2417`), `call_native_kind`/`match_arms_uniform_literal_kind` — opcode/literal-inspection NativeKind recovery.
9. **`storage_hint_to_field_kind` / `native_kind_from_storage_type`** `helpers.rs:6010` / `type_tracking.rs:90` — `StorageType`→NativeKind is yet another source-type→kind map (StorageType is a runtime enum, distinct from ConcreteType).
10. **~151 direct `NativeKind::` literal construction sites** across 19 compiler files (`v2_array_emission.rs`, `v2_typed_emission.rs`, `v2_map_emission.rs`, `typed_emission.rs`, `mutation_writeback.rs`, `loops.rs`, `closures.rs`, etc.) — many stamp `Ptr(HeapKind::…)` directly rather than projecting from a Type.

**Decisive structural fact:** grep for any `ConcreteType → NativeKind/FieldKind` derivation **inside `crates/shape-vm/src/compiler/` returns EMPTY**. The SB-8 `ConcreteType` side-tables exist (`current_function_local_concrete_types` `mod.rs:1684`, `module_binding_concrete_types` `:1710`, `inferred_param_concrete_types` `:1383`) but are consumed ONLY by `infer_expr_type` for inference (`expressions/mod.rs:1652`, `:2003`; `loops.rs:442/884`) and by monomorphization save/restore (`monomorphization/cache.rs:432/438/623/626/840/843`) — **never to derive an emit-time NativeKind**. The one ConcreteType→kind map that IS reachable in shape-vm is via capture-kind layout (`closures.rs` → `from_capture_types` → closure_layout map) and the JIT copy; the VM emit path itself goes stringly.

So U4's claim "keep NumericType/NativeKind only as a stamp DERIVED from the one Type" requires **redirecting all 10 entry points above to `native_kind_from_concrete_type(concrete_type_from_engine_Type)`**, and the SB-8 ConcreteType tables (which already hold the right structural data) become the natural input — but today they feed inference, not stamping.

**Delta vs audit:** the audit's `NumericType` register and the `.strip_suffix("[]")` / `Array<`/`Vec<` re-parse it flagged are all still present (`expressions/mod.rs:1632`, `:2232-2234`; plus stringly `strip_suffix('>')` re-parses at `typed_emission.rs:146`, `v2_map_emission.rs:178`, `v2_typed_emission.rs:473`, `property_access.rs:111`, `function_calls.rs:1102`, `loops.rs:2019`, `matrix_ops.rs:58`, `closures.rs:1106`). U1/U3 did NOT remove the stringly emit path; they touched the inference/HashMap-carrier layer. The SB-7 four-representation split is intact, plus FieldKind and StorageType as additional reps (so closer to SIX static-type reps in L3, not four).

---

## PART 4 — Proof-gate placement (U4 ↔ U2 interface)

**Where `prove_native_kind` would sit after U4:** the single derivation `engine Type → ConcreteType → native_kind_from_concrete_type → NativeKind` makes the projection deterministic and total. `prove_native_kind` becomes the **consistency assertion** at each typed-opcode emission site: "the kind I'm about to stamp on this opcode equals the canonical projection of this expression's proven ConcreteType." Concretely it belongs:
- at the numeric binop terminal (`binary_ops.rs:2507` region, replacing/subsuming `numeric_operand_proof_gap`),
- at the return-value stamp (`emit_return_value_with_ownership`, replacing the Route-C-vs-Route-D agreement dance at `helpers_binding.rs:493-514`),
- at typed store/load emission (`emit_store_local_for_hint` / `typed_store_local_opcode`),
- at the v2 typed-array/object/map Ptr-stamp sites.

In the U4 end state, `last_emitted_native_kind` (Route D, the opcode-inspection recovery) is exactly what `prove_native_kind` replaces: instead of recovering the kind from already-emitted bytecode and hoping it agrees with a hardcoded table, the kind is projected from the proven Type and `prove_native_kind` asserts the opcode's stamp matches it.

**Is wiring it part of U4 or separable?** **Separable, but the enabling work is U4.** The gate cannot be wired meaningfully *before* U4, because today there is no single proven `ConcreteType` at the emit sites to project from — the kind arrives via the stringly/NumericType/opcode-inspection routes, so `prove_native_kind` would have nothing trustworthy to check against (you'd be checking a re-derivation against another re-derivation). The sequence is:
1. **U4 (this root):** collapse to one derivation — make the engine `Type` (→`ConcreteType`) the sole input at every emit site; delete Routes B/C/D/E's independent re-derivations; keep `native_kind_from_concrete_type` as the only `Type→kind` map (and unify the two divergent copies first).
2. **U2 follow-up (separable):** flip `prove_native_kind` from inert to wired — make each typed-opcode emission call it and propagate `ProofGap` as `E_TYPED_OPCODE_WITHOUT_PROOF`. This is the "Phase 2 wires call sites" step described in the dormant doc-comment at `type_tracking.rs:1188`, which has never executed.

The two divergent `ConcreteType→NativeKind` maps (closure_layout vs mir_compiler, §Part 2) are a **U4 prerequisite**: the proof gate projects through the closure_layout copy, so until Option/Result/Pointer/Void arms agree, wiring the gate would make the VM and JIT prove against different expected kinds."
    }
  }
}