# Cluster #7 — Result `!!` context-operator + `?` try-operator broken at runtime

**HEAD:** `b45bcf77` (audit-only; no source/fixture changes)
**Sub-cluster name:** `c7-result-fn-return-kind-clobber`
**Estimated size:** **S** — single bisect target. The 76 FN-REG-CORRECTNESS error_handling tests (Groups A/B/C/D/G/H/I) + 8 enums Group F + 1 regression `!!`-context-dropped finding all reduce to **one** root: the function-return ABI re-stamps the parallel kind track to a hard-coded `NativeKind`, dropping `Ptr(HeapKind::Result)` / `Ptr(HeapKind::Option)` carriers. This is the same root family as Cluster #3 (`{"Bool": false}` for `print(Color::Red)`); both surface ADR-006 §2.7.13 / §2.7.7 kind-drift across a return boundary.

---

## 1. Three minimal repros (run-verified at HEAD `b45bcf77`)

### 1.1 `!!` context-operator on `Err`

```shape
fn divide(a: int, b: int) -> Result<int, string> {
    if b == 0 { Err("base") } else { Ok(a / b) }
}
fn main() {
    let r = divide(10, 0) !! "context"
    print(r)
}
```

Expected: program raises `Uncaught error: context: base` (or similar — `!!` must wrap `Err("base")` with cause `"context"` and throw via `handle_exception`).
Actual: prints `{"Bool": false}`. **Silent-wrong-output.** No throw. `!!` never sees a `Ptr(HeapKind::Result)` slot.

### 1.2 `?` try-operator

```shape
fn maybe(b: int) -> Result<int, string> {
    if b == 0 { Err("oops") } else { Ok(b * 2) }
}
fn run() -> Result<int, string> {
    let v = maybe(0)?
    Ok(v + 1)
}
fn main() {
    let out = run()
    print(out)
}
```

Expected: prints `Err("oops")` (the inner `?` early-returns the `Err` carrier verbatim from `run`).
Actual: prints `{"Bool": false}`. Same root — `maybe(0)` already arrives in `run` with kind `Bool`.

### 1.3 `?` + multi-level propagation chain

```shape
fn level1(n: int) -> Result<int, string> { if n == 0 { Err("zero") } else { Ok(n + 1) } }
fn level2(n: int) -> Result<int, string> { let a = level1(n)?; Ok(a * 10) }
fn level3(n: int) -> Result<int, string> { let b = level2(n)?; Ok(b + 100) }
fn main() {
    let bad = level3(0)
    let good = level3(5)
    print(bad); print(good)
}
```

Expected: `Err("zero")` then `Ok(160)`.
Actual: both print `{"Bool": false}`.

**Isolation control (passes):** Top-level `let r = Err("base"); print(r)` (NO function boundary) prints `Err("base")` correctly. The bug is entry-exit-on-fn-boundary only.

**Isolation control (fails):** `fn make_err() -> Result<int, string> { Err("base") } fn main() { let r = make_err(); print(r) }` already prints `{"Bool": false}` — confirming this is NOT a `!!`/`?` bug per se; **`!!` and `?` never see a Result carrier in the first place** because the caller already received a `Bool`-kinded slot.

---

## 2. Root cause

`op_return_value_bool` at `crates/shape-vm/src/executor/control_flow/mod.rs:874-877` **discards the source kind and hard-codes `NativeKind::Bool`**:

```rust
pub(in crate::executor) fn op_return_value_bool(&mut self) -> Result<(), VMError> {
    let (bits, _src_kind) = self.pop_kinded()?;  // _src_kind = Ptr(HeapKind::Result), DROPPED
    self.return_value_inner(bits, shape_value::NativeKind::Bool)  // re-stamp to Bool
}
```

The same shape exists for `op_return_value_i64` / `_u64` / `_i32` / `_i8` / `_u8` at L832-871 — all discard `_src_kind` and re-stamp. (Sibling `op_return_value_ptr` at L879-887 correctly preserves `src_kind`.)

`Ok(...)` / `Err(...)` ctors at `crates/shape-vm/src/executor/vm_impl/builtins.rs:623-658` correctly emit `push_kinded_slot(KindedSlot::from_result(res))` with kind `Ptr(HeapKind::Result)` (verified at top-level — control repro passes). The kind is **clobbered at the function-return boundary**.

The compile-time gate at `crates/shape-vm/src/compiler/helpers_binding.rs:369-410::emit_return_value_with_ownership` chooses the typed `ReturnValueBool` opcode via `last_emitted_native_kind() == StorageHint::Bool`. The walk-back at `helpers.rs:2177-2229` skips past `DropCall`/`ReturnOwned` chatter looking for "the actual producer". For a body like `Err("base")`, the producer is `CallBuiltin(ErrCtor)` — the walk-back's `_ => break` arm at `helpers.rs:2223` lands on `CallBuiltin`, and `CallBuiltin` is NOT listed in the raw-i64/Bool/F64 producer match below, so `last_emitted_native_kind` should return `None` for the CtxOp. Either (a) `CallBuiltin(OkCtor|ErrCtor|SomeCtor)` is silently triggering a Bool match through an unexpected arm, OR (b) `last_expr_numeric_type` is being set to `NumericType::Bool` by a sibling branch (the `if b == 0 { Err(...) } else { Ok(...) }` if-as-expression — Bool condition leaking into the if-result type stamp), OR (c) the walk-back is past the `Err`/`Ok` producer and landing on a `Constant::Bool` / `EqInt` upstream comparison.

(c) is most likely — `last_emitted_native_kind` at `helpers.rs:2361` already maps `GteI32 => Bool` and a sibling like `Eq*` would return Bool. If the walk-back skips through `CallBuiltin` (it doesn't per the listing, but `if`-as-expression bodies have jump targets the walk-back may stumble through), it lands on the most recent comparison — which for `if b == 0 { ... }` is the `b == 0` Bool result.

Even if the compiler-side mis-pick is fixed, **the executor-side `op_return_value_bool` body MUST preserve `src_kind` for heap-carrier slots** (parallel to how `op_return_value_ptr` does). Hard-coding `NativeKind::Bool` is forbidden under ADR-006 §2.7.7 — every kind on the parallel track must come from the actual producer, not a fabrication.

Cited shared family: Cluster #3 `c3-wire-conversion-v2-raw-carrier-projection` documents the SAME `{"Bool": false}` wrong-output for `print(Color::Red)` post-typed-return-pipeline. Both are kind-drift at a value-flow boundary.

## 3. Bisect anchors

```
$ git log --oneline -- crates/shape-vm/src/executor/exceptions/mod.rs
3b5debfa W8-EX: kinded exception handler rebuild (close)
691d8c3c merge: W8-EX close — kinded exception handler rebuild
5b028b40 W14-variant-codegen: HeapKind::Result + HeapKind::Option + variant codegen + 8 op_* bodies (close)
61d0f496 W13-result-option-ops: variant-discriminator opcode bodies (close)
e9c7260d W13-anyerror: AnyError TypedObject builder rebuild (close)
ca32cebc fix(v0.3): WS-3 — Result / AnyError / error-path machinery (7 fixes)

$ git log --oneline -- crates/shape-vm/src/executor/control_flow/mod.rs
# search for the commit that added `op_return_value_bool` with hard-coded NativeKind::Bool
# this is the Wave E+3 typed-return-value family — opcodes 0x198..=0x1A2
```

**Primary bisect target:** the Wave E+3 typed-return-value family commit (`op_return_value_bool` introduction). The bug is intrinsic to that handler shape — `_src_kind` discarded by design at the point of introduction. The CLAUDE.md docstring at L355-368 calls the typed-return handlers "transport-neutral (same body, same bits-through behavior)" — that claim is FALSE when the source kind is `Ptr(HeapKind::Result)` and the compile-time hint mis-classifies as Bool, because the bits-through behavior re-labels the kind on the parallel track.

**Secondary bisect target:** `last_emitted_native_kind` walk-back at `helpers.rs:2177` — verify whether it correctly returns `None` for the post-`CallBuiltin(OkCtor|ErrCtor|SomeCtor)` position, or if it walks past and lands on a sibling Bool producer in `if`-as-expression bodies.

## 4. Affected subsystem (file:line citations)

- **Executor-side kind clobber (primary):** `crates/shape-vm/src/executor/control_flow/mod.rs:874-877` (`op_return_value_bool` hard-codes `NativeKind::Bool`). Same shape at L830 (`_i64`), L836 (`_u64`), L843 (`_i32`), L849 (`_u8`), L866 (`_u8`). **Correct sibling for comparison:** L879-887 (`op_return_value_ptr` preserves `src_kind`).
- **Compile-time mis-pick (secondary):** `crates/shape-vm/src/compiler/helpers_binding.rs:369-410` (`emit_return_value_with_ownership` picks typed return via `last_emitted_native_kind`).
- **Walk-back logic:** `crates/shape-vm/src/compiler/helpers.rs:2177-2400` (`last_emitted_native_kind`).
- **Ctors (working — control):** `crates/shape-vm/src/executor/vm_impl/builtins.rs:601-658` (`SomeCtor`/`OkCtor`/`ErrCtor` — correctly push `Ptr(HeapKind::Option|Result)`).
- **`!!` handler (works given correct input kind):** `crates/shape-vm/src/executor/exceptions/mod.rs:521-583` (`op_error_context` — dispatches via `read_result`/`read_option` which check `slot.kind == Ptr(HeapKind::Result|Option)`; fails-closed to the bare-value pass-through arm when kind is Bool, which is WHY the `!!` user-visible failure shape is "context dropped" not "panic").
- **`?` handler (works given correct input kind):** `crates/shape-vm/src/executor/exceptions/mod.rs:617-678` (`op_try_unwrap` — same kind-gated dispatch; falls to bare-value pass-through when kind is Bool).

## 5. Sub-cluster name + size estimate

**Sub-cluster:** `c7-result-fn-return-kind-clobber`
**Estimated size:** **S (single bisect target, mechanical fix).** The fix is two parts:
1. **Executor:** `op_return_value_<scalar>` family must preserve `src_kind` when the source kind is a heap-carrier (`Ptr(HeapKind::*)`), or — preferable — every typed `op_return_value_*` should preserve `src_kind` like `op_return_value_ptr` already does, with the typed `<Kind>` suffix being a JIT static-annotation only (matching the docstring's stated contract).
2. **Compiler:** `last_emitted_native_kind` post-`CallBuiltin(OkCtor|ErrCtor|SomeCtor)` and post-`if`-as-expression-returning-Result must return `None` (or `Ptr(HeapKind::Result|Option)`), not `Bool`. Once the compiler doesn't mis-pick `ReturnValueBool` for a Result-typed body, the executor-side bug stops firing — but the executor body is the deeper soundness fix per ADR-006 §2.7.7.

**Risk of M-class:** if the compiler-side walk-back has multiple latent mis-picks (e.g. Some/Ok/Err ctors when body ends in comparison-as-discriminator), each additional shape adds a small fix — but all share the executor-side guard, which is the single mechanical line change that flips all 85 tests green.

## 6. Dependencies

- **Cluster #3 (`c3-wire-conversion-v2-raw-carrier-projection`)** — SAME ROOT FAMILY. `print(Color::Red)` produces `{"Bool": false}` for the identical reason: enum-unit-variant carrier flows through a typed-return / typed-print boundary that re-stamps the kind to Bool. Both clusters reduce to the same ADR-006 §2.7.7 / §2.7.13 violation — kind on the parallel track fabricated from a sibling rather than the actual producer. Fixing the `op_return_value_<scalar>` family to preserve `src_kind` for heap-carrier slots resolves both clusters simultaneously. **Recommend joint fix.**
- **Cluster #12 (`enums-equality-rejected`)** — DIFFERENT ROOT. That cluster is a compile-time semantic error (`Cannot infer types for binary operation Equal: 'unknown' and 'unknown'`) because `register_enum` does not auto-derive `impl Eq for <EnumName>`. Cluster #7 is a runtime kind-drift bug. No shared root, no shared fix.
- **Cluster #2 (`ADR-006 §2.7.13 kind drift`)** — likely SHARED ROOT FAMILY but at a different boundary (per audit doc filename). If #2's repro flow involves the same `op_return_value_<scalar>` clobber, the fix is shared. Recommend cross-check before fix lands.

## 7. Discipline notes

- Audit-only run-verified — no source / fixture changes, no commits, no `git stash`.
- 3 minimal repros + 2 isolation controls executed via `direnv exec /home/dev/dev/shape-lang cargo run --bin shape -- run <file>.shape` at HEAD `b45bcf77`.
- The 85 test rosters (76 error_handling FN-REG + 8 enums Group F + 1 regression) are NOT re-listed here — see `docs/cluster-audits/v0.3-classification/error_handling.md` Groups A/B/C/D/G/H/I + `enums.md` Group F + `regression.md` line 346-352 for the canonical lists.
- No defection-attractor framings used. The fix is preserve-src_kind in the typed-return ABI — not a new opcode, not a shim, not a bridge.

## §8 Empirical correction post JOINT-FIX-#1 + JOINT-FIX-1a (2026-05-28)

The audit framing of "single root for 84 fails" was empirically refined to **three sub-fixes** under the supervisor 2026-05-28 c7-cluster REFINEMENT-COMPLETION dispatch. JOINT-FIX #1 + JOINT-FIX-1a confirm the following correction; JOINT-FIX-1b's `?` Ok-payload work is still in flight as of this commit. Shape matches the precedent of the `ff978be4` aliased-CoW audit §7 post-fix correction.

(a) **§2 compiler walk-back hypothesis FALSIFIED.** The audit at §2 speculated `last_emitted_native_kind` might walk past `CallBuiltin(OkCtor|ErrCtor|SomeCtor)` and land on a sibling Bool comparison (variant "(c) is most likely"). JOINT-FIX #1's Q3 ground-truth investigation (per `c6226b18` commit message) confirmed the walk-back logic is sound — it correctly does NOT mis-pick Bool for Result-typed bodies. The single-line executor-side fix at `op_return_value_<scalar>` family (preserve `src_kind`, drop hard-coded suffix-kind) was the entire root cause.

(b) **Typed-return-clobber root CONFIRMED and FIXED at JOINT-FIX #1 (`c6226b18`).** `crates/shape-vm/src/executor/control_flow/mod.rs` typed `op_return_value_i64` / `_u64` / `_f64` / `_i32` / `_i8` / `_u8` / `_bool` handlers now preserve `src_kind` from the producer per ADR-006 §2.7.7. Audit §2 / §4 lines L74-L77 (the "hard-codes `NativeKind::Bool`" repro) — exactly that diff. Companion fix at `2f0c9218` ("script-completion sentinel uses NativeKind::Null not Bool") covered the related sentinel mis-stamp.

(c) **Audit §1.1 "Expected" entry was wrong; canonical contract is WRAP.** The audit at §1.1 line 23 wrote `Expected: program raises 'Uncaught error: context: base'` (THROW semantics). JOINT-FIX-1a's investigation of `shape-web/book/book-site/src/content/docs/fundamentals/error-handling.mdx` L232-248 + the 26 Group A test corpus (all WRAP-shaped: `expect_string("err")`, `expect_number(-1.0)`, `expect_output_contains("Ok"|"Err")` via `match r { Ok(_)/Err(_) }`) established that `!!` is unambiguously a **wrap operator**, never a throw. Post JOINT-FIX-1a the canonical §1.1 result is:

```
Err({category: "RuntimeError", payload: "context", cause: "base", trace_info: None, message: "context", code: None})
```

— i.e. `print(r)` prints the `Err(AnyError { payload: rhs (context), cause: lhs-inner (base), … })` value per the book table at L236-242. The audit's pre-fix THROW expectation was the visible artifact of `op_error_context` body (`crates/shape-vm/src/executor/exceptions/mod.rs:521-583`) being internally inconsistent with the book + tests — the docstring at the now-replaced L530-536 explicitly claimed "`!!` is a runtime assertion, not a fallible function return", contradicting the book. The doc-coherence rewrite (docstring + impl + audit doc) lands at the JOINT-FIX-1a impl commit.

(d) **Residual α + β at exceptions/mod.rs operator-semantics layer (NOT typed-return-clobber family).**
- **α (JOINT-FIX-1a — this commit):** `op_error_context` body redesigned per book contract. 5 branches: Ok(v)→Ok(v); Some(v)→Ok(v) (lift); Err(e)→Err(AnyError{payload:rhs, cause:e, …}); None→Err(AnyError{payload:rhs, cause:"Value was None", …}); bare-v→Ok(v). Companion fix: `is_null_sentinel` at `crates/shape-vm/src/executor/exceptions/mod.rs:960` gains the missing `NativeKind::Null => true` arm to mirror `comparison/mod.rs::is_null_kinded` (R5b-2-disposition coherence). Probe-verified at HEAD post-fix; 32 tests drop from baseline-failed in the joint-fix-1 + 1a window (25 of 28 Group A WRAP-shaped tests + 7 edge_cases tests).
- **β (JOINT-FIX-1b — in flight):** `op_try_unwrap` Ok-payload-recovery via `?` (Group B 17 tests) + top-level-`?`-on-Err semantics (whether script-level `Err(...)?` throws to host vs silently pushes the Err carrier as script value). Owned by parallel sub-agent 1b.

(e) **Prediction-count was wrong.** Audit §1 line 5 claimed "85 fails reduce to one root"; empirically:
- JOINT-FIX #1 dropped baseline 1147→1062 = **−85 from the typed-return-clobber family** (matches the §1 numerical prediction, but distributed across c7 + c3 + other heap-carrier-flow sites, not the single shape the audit framed).
- JOINT-FIX-1a drops error_handling baseline 90→58 (32 fewer fails). The fix is in the `!!` semantic-contract family, NOT the typed-return-clobber family the audit framed as the single root.
- JOINT-FIX-1b is expected to handle Group B (17 tests) + the 6 `expect_run_err_contains` tests in Group A (which trigger a pre-existing wire_conversion.rs `Ptr(HeapKind::TypedObject)` SEGV at the host-boundary projection — see §8(f) below; whether 1b's fix unblocks them depends on whether `?` at script-top-level throws vs returns).

(f) **Pre-existing wire_conversion bug surfaced by JOINT-FIX-1a (NOT 1a-introduced).** At `crates/shape-runtime/src/wire_conversion.rs:198` the `Ptr(HeapKind::TypedObject)` arm casts `bits as *const HeapValue` — but v2-raw TypedObjectStorage (the AnyError carrier per ADR-006 §2.3 + Wave 2 D4 ckpt-2 v2-raw `_new` allocator) is NOT an `Arc<HeapValue>`. Type confusion → SIGSEGV at script-end host projection. Repro at HEAD (independent of 1a): `type P { x: int, y: int }; P { x: 1, y: 2 }` (raw TypedObject as script terminal value) — 10/10 SEGV. The WS-3 F2b fix at L177-187 / L188-197 added Result/Option special-case projection BEFORE the bad cast, but the analogous TypedObject arm was not added. **Tracked as a follow-up** distinct from c7 — `c7-wire-conversion-typedobject-arm-followup`. Affects the 4 Group A `expect_run_err_contains` tests + 2 `test()?` tests (6 tests total) post-1a. Out of 1a + 1b scope per the dispatch prompt's territory bounds.
