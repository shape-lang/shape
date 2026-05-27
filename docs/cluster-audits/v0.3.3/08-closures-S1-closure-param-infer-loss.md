# v0.3.3 fix-dispatch cluster #8 — closures_hof S1 — closure-param type-inference loss

**HEAD:** `70507224` (post-v0.3.2 — same audit baseline as `82f049dd` classification doc; closure-touch files unchanged since).
**Cluster size:** 77 tests (closures_hof S1, FN-REG-CORRECTNESS).
**Discipline:** AUDIT-ONLY. No source/fixture changes. No commits. Repro run via `cargo run` only.
**Owning files (read-only):**
- `crates/shape-vm/src/compiler/expressions/closures.rs` (compile_expr_closure — let-binding-time param publish)
- `crates/shape-vm/src/compiler/expressions/function_calls.rs` (install_pending_closure_param_types_for_* — HOF call-site publish)

---

## 1. Minimal repro (FN-REG-CORRECTNESS confirmed)

```shape
let f = |a, b| a + b
print(f(1, 2))
```

Run (`cargo run --bin shape -- run /tmp/cluster8_repro.shape`):

```
Error: Runtime error: Bytecode compilation failed:
Semantic error: Cannot infer types for binary operation `Add`:
operand types are `unknown` and `unknown`.
Strict typing requires both operands to have a known concrete type at compile time.
Add a type annotation to disambiguate.
```

Repro reproduces canonically. The classification doc's repro is verbatim correct.

---

## 2. Root cause

The closure compiler at `closures.rs:736-780` consumes a `pending_closure_param_types` hint to seed user-param type annotations. The hint is populated by **three** call-site paths in `function_calls.rs`:

| Site | File:Line | Trigger shape |
|---|---|---|
| HOF method-arg | `function_calls.rs:1754 install_pending_closure_param_types_for_hof` | `arr.map(|x| ...)`, `.filter(|x| ...)`, etc. — receiver generic param infers closure arg type. |
| Any-typed named-fn-arg | `function_calls.rs:1892 install_pending_closure_param_types_for_any_param_hof` (called from `function_calls.rs:1249`) | `apply(|x,y| x+y, 2, 3)` where `apply` has an `any`-annotated callable param + homogeneous concrete-typed remaining args. |
| **(NONE for let-bound)** | — | `let f = |a,b| a+b` has NO call site at let-binding time. Closure compiles before its call is seen. |

For `let f = |a,b| a + b`, `compile_expr_closure` (`closures.rs:592`) runs at let-binding time with **no caller context**:
- `user_param_hints = self.pending_closure_param_types.take()` (`closures.rs:742`) → `None`.
- Body-level fallback `infer_param_type_from_body` (`closures.rs:124-126`, definition above) handles `<param> op <literal>` and `<param> op <outer-ident>` (the `_with_outer_idents` variant at `closures.rs:771`) — but the body `a + b` is **`<param> op <param>`**, which neither helper covers.
- Both `a` and `b` end up unannotated → compile-time strict-typing rejection of the `Add` op inside the body.

The call-site `f(1, 2)` later has full type info (`Int(1)`, `Int(2)`) and DOES re-infer return type via `local_callable_closure_bodies` peek (`function_calls.rs:819-836`, "cluster-2-cw-IB-class-b" — added 2026-05-16 commit `97a91029`) — but that peek runs **after** the closure has already failed to compile. There is no "delay closure compilation until first call site" path. The closure-body peek mechanism re-infers RETURN type only, not param types.

**Why this regressed.** Prior to W17 / W17.2-C typed-closure work (commits `2f64ae83` 2026-05-19, `9818ee44` 2026-05-18, `97a91029` 2026-05-16), the closure body could fall through to a runtime/dynamic-dispatch `Add` opcode (the deleted `*Dynamic-emission shim` referenced verbatim in `closures.rs:753`). Strict-typing removal of that shim turned the unannotated-param case from "runs dynamically" into "compile-time reject". The compensating bidirectional inference was added for HOF (`c9dfb82b sweep phase 1.5 cluster 3`) and any-param-HOF (cite uncertain — likely `87a295ca` D-α.1 closure-param inference for sort), but NOT for let-bound closures.

---

## 3. Bisect anchor (history map)

`git log --oneline -- crates/shape-vm/src/compiler/expressions/closures.rs`:

| Commit | Title | Relevance |
|---|---|---|
| `97a91029` | Phase 3 cluster-2 Round 3 closure-wave-IB-class-b — Class B coverage | Adds value-call return-`ConcreteType` re-inference at the **call site** (`function_calls.rs:776-836`). Covers RETURN-type, NOT PARAM-type. |
| `87a295ca` | D-α.1 closure-param inference for sort + closure-aware spec type-annotation propagation | Adjacent fix-shape — proves the same defect class exists for sort-comparator closures. Per-callsite, not let-binding-generic. |
| `c9dfb82b` | sweep phase 1.5 cluster 3: bidirectional closure inference for HOF dispatch | The HOF-only sibling of the missing let-binding-time path. |
| `ab59f3fc` | sweep phase 1.5 cluster 2: closure-body literal-pairing param inference | Adds `infer_param_type_from_body` — handles `<param> op <literal>` only. |
| `d26c730f` | sweep phase 1.5 cluster 2 ext: param inference from outer-scope ident pairings | Adds `_with_outer_idents` — handles `<param> op <outer-ident>` only. |
| `2f64ae83` | W17.2-C TRANSITIONAL 4-name narrowing | Tightened strict-typing — the regression-amplifier (removed the dynamic fallback the body had been riding on). |

The missing case is **`<param> op <param>`** — neither sweep-phase-1.5 nor the W17 / W17.2-C narrowing addressed it.

---

## 4. Affected subsystem (fix surface)

**Primary file:line for the publish-gap:**
- `crates/shape-vm/src/compiler/expressions/closures.rs:742` — `user_param_hints` is `None` at let-binding time.
- `crates/shape-vm/src/compiler/expressions/closures.rs:754-780` — fallback chain has no case for `<param> op <param>` with both params unannotated.

**Two viable fix shapes** (audit-only — no implementation here):

**Fix A (simpler — most-likely closes the family in one commit):**
Defer compilation of `let f = |..|` closures until the first call site is observed (lazy compilation). At first `f(...)` site, build a `pending_closure_param_types` hint from the concrete arg types and compile the closure body with that hint. Mirrors how generic functions are monomorphized at first call. **Risk:** changes ordering of error reporting + interacts with mutual recursion / closures-in-closures / closures returned from functions (S5 family).

**Fix B (narrower — won't close all 77):**
Extend `infer_param_type_from_body` / `_with_outer_idents` to propagate via a **2-pass fixed-point** when `<param> op <param>` appears: if one of them gets inferred via another path (e.g. from a separate `<other-param> op <literal>` elsewhere in body, or from a capture-pair), propagate. **Risk:** doesn't help the canonical `|a, b| a + b` case (both params equally unknown in body).

**Recommended:** Fix A. The closure body peek (`ClosureBodyPeek`) machinery already exists for return-type re-inference at the call site (`function_calls.rs:819`); extending the same peek to drive **param-type publish** before re-running the closure body compile is the surgical extension. The retained body is already there; we'd add a `pending_closure_param_types` install before the value-call compile and re-run the (deferred) inner-body compilation. This mirrors the existing `cluster-2-cw-IB-class-b` shape exactly.

---

## 5. Sub-cluster size estimate — **M (medium)**

Single fix expected to close the canonical S1 shape (77 tests) for the `<param> op <param>` family. Estimate breakdown:
- Lazy-closure-compile prototype: ~1 day.
- Wire deferred body into `local_callable_closure_bodies` map + reuse `ClosureBodyPeek`: ~1 day.
- Cascade triage on the ~9 other shapes (S5/S7/S8/S10 may share root): ~1 day.
- Risk buffer for snapshot/JIT interaction (lazy compile changes when bytecode is emitted, which may interact with FunctionBlob content-hashing): ~1 day.

Total: **~3-4 days**. Sized M.

---

## 6. Dependencies — overlap with other v0.3.3 clusters

**Cluster #9 (closures_hof S2 — 23 var-capture upvalue):** *Independent root cause.* S2 is a closure-frame-setup / upvalue-allocation defect (CreateClosure emit path missing the upvalue operand when capture is `var`). S1 is a type-inference-publish defect. Both touch closures, but the failure stages are different (S1 fails at body-compile, S2 fails at run-time call-frame setup). However, a successful Fix A (lazy compile + call-site hint) is **prerequisite** for S2 testing — until S1 is fixed, S2's mutable-capture closures often can't even compile their bodies (they trip S1 first). Recommended ordering: **#8 → #9**.

**Cluster #11 (variables_bindings width-types):** *Likely independent.* width-types touches `i8`/`i16`/`i32` carrier-typing in the type-tracker, not the closure-binding type-publish path. They share `type_tracking.rs` as a file, but at different layers. No overlap predicted unless empirical evidence emerges during fix work.

**Cluster #5 (closures_hof S5 `call_value_immediate_nb: ... got Ptr(NativeView)`):** *Possibly shared root.* S5's NativeView leakage in the callee slot for `compose`-style HOFs is a kind-carrier defect at the value-call ABI (§2.7.11/Q12). Fix A's lazy compile may incidentally surface S5 cases (the deferred body now sees a concrete-typed callee at call time) but is unlikely to close it. Recommend treating S5 separately.

**Cluster #6 (closures_hof S7/S8 silent-wrong / kind mislabel):** *Possibly shared root with S1+S5 family.* The `0` / `0.0000…0208` denormal outputs in S8 suggest closure-emit-path packs a kind-blind carrier when called. If Fix A drives the closure body through a kinded compile path with concrete types, some S7/S8 tests may incidentally green. Triage during Fix A work.

---

## 7. Forbidden-pattern hygiene check (per CLAUDE.md)

No defection-attractor framings emerged during this audit. The fix surface is:
- Type-inference publish path (compile-time tracker extension).
- Lazy compilation ordering (precedent: generic monomorphization).

No "value-call bridge" / "closure-callback translator" / "tag-decode hop" / "decoder pattern" / "bridge / probe / helper / hop / translator / adapter / shim" framings encountered or required. The proposed fix extends an existing typed publish path (`pending_closure_param_types`) and reuses an existing peek (`ClosureBodyPeek`) — both are first-class compiler machinery, not bridges across deleted dispatch.

---

## 8. Audit close

**Classification:** FN-REG-CORRECTNESS confirmed (matches taxonomy + classification doc).
**Repro:** verified at HEAD.
**Root cause:** localized to `closures.rs:742-780` (param-publish gap for let-bound closures at compile-time vs HOF-method-arg) + cascade-amplified by W17.2-C strict-typing narrowing (`2f64ae83`) which removed the dynamic fallback the no-param-hint case had been riding on.
**Recommended fix shape:** Fix A — extend the closure body peek + `pending_closure_param_types` install to a let-bound-closure call site (lazy-compile-on-first-call). Sized M (~3-4 days).
**Sequencing:** unblocks #9 (closures_hof S2 testability); independent of #11; possibly shares root with #5/#6/#7/#8.
