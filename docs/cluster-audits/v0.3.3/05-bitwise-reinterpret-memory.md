# v0.3.3 Cluster #5 — Bitwise ops silently reinterpret memory

**HEAD at audit:** workspace tip on `main` (latest: `70507224`).
**Audit-only.** No source / fixture / commit changes.
**Class:** FN-REG-CORRECTNESS (silent-wrong-result, type-safety regression).
**Source taxonomy doc:** `docs/cluster-audits/v0.3-classification/operators.md:61-139`.

---

## 1. The 8 fixtures

All in `tools/shape-test/tests/operators/`. Each is one-line, asserts `expect_run_err()`, and now silently succeeds with garbage integer output:

| # | Fixture file (`tools/shape-test/tests/operators/`) | Test | Source under test |
|---|---|---|---|
| 1 | `stress_bitwise_and_or.rs:390` | `test_and_on_float_fails` | `1.5 & 3` |
| 2 | `stress_bitwise_and_or.rs:396` | `test_or_on_float_fails`  | `1.5 \| 3` |
| 3 | `stress_bitwise_and_or.rs:402` | `test_and_on_string_fails` | `"hello" & 3` |
| 4 | `stress_bitwise_and_or.rs:408` | `test_or_on_string_fails`  | `"hello" \| 3` |
| 5 | `stress_bitwise_shift.rs:327`  | `test_shl_on_float_fails` | `1.5 << 2` |
| 6 | `stress_bitwise_shift.rs:333`  | `test_shr_on_float_fails` | `1.5 >> 2` |
| 7 | `stress_bitwise_xor_not.rs:308`| `test_xor_on_float_fails` | `1.5 ^ 3` |
| 8 | `stress_bitwise_xor_not.rs:314`| `test_not_on_float_fails` | `~1.5` |

All 8 are the **same regression**: typed-int bitwise path is taken only when `both_int` is proven; otherwise the compiler emits the dynamic bitwise opcode which reinterprets the slot bits as `i64`. **No fixture asserts on a specific diagnostic string** — only `expect_run_err()` — so once the compile-time gate is added, all 8 pass with no fixture edits.

---

## 2. Minimal repro (verified)

```shape
print(1.5 | 3)
```
Run: `direnv exec /home/dev/dev/shape-lang cargo run --bin shape -- run /tmp/bitwise-repro.shape`

Actual VM output: `4609434218613702659`  (= `f64::to_bits(1.5)` reinterpreted as `i64`, OR-ed with 3).

```shape
print("hello" | 3)
```
Actual VM output: `111404952129875` (= raw `Arc<String>` pointer bits OR-ed with 3 — heap-pointer disclosed to user code).

JIT path correctly surfaces ("kind-untyped BitOr reached the JIT — SURFACE per W10 playbook §5") and falls back to interpreter — interpreter is the silent path.

---

## 3. Root cause

`crates/shape-vm/src/compiler/expressions/binary_ops.rs:1403-1576` — the bitwise arm of `compile_binary_op_inner`:

- L1489-1492: `let emit_typed = both_int && typed_bitwise_enabled();` — typed-int emission gate requires *proof* both operands are `int`.
- L1539-1554: typed path emits `BitAndInt`/`BitOrInt`/`BitXorInt`/`BitShlInt`/`BitShrInt`. Correct.
- L1555-1575: **else branch** falls through to `self.compile_binary_op(op)` ("dynamic fallback"). No proof check. No compile-time rejection of float/string/heap operands.

The dynamic opcodes land at `crates/shape-vm/src/executor/arithmetic/mod.rs:637-658`:

```rust
fn exec_dyn_bit_binary(&mut self, op: OpCode) -> Result<(), VMError> {
    let (b_bits, _b_kind) = self.pop_kinded()?;   // <-- kind discarded
    let (a_bits, _a_kind) = self.pop_kinded()?;
    let b_int = b_bits as i64;                    // <-- raw bits as i64
    let a_int = a_bits as i64;
    let result = match op { BitXor => a_int ^ b_int, ... };
    self.push_kinded(result as u64, NativeKind::Int64)
}
```

This is a direct violation of CLAUDE.md §Forbidden Code: "Runtime `tag_bits` dispatch (deleted)" + ADR-006 §2.7.7/Q9 "kind IS the discriminator". The `_b_kind` / `_a_kind` silent-discard is the same defection-attractor pattern the W-series IC retirement closed for the typed arithmetic ABI.

**Per the contrast at `binary_ops.rs:1680` and `:1795`:** `+`, `-`, `*`, `/`, `%`, `**` are protected by `is_strict_arithmetic(op)` + `is_type_numeric` (rejects non-numeric at compile time). Bitwise ops are **NOT** in `is_strict_arithmetic`'s match (`numeric_ops.rs:12-17` covers only `Sub | Mul | Div | Mod | Pow`), so they bypass the strict-type rejection and fall straight into the unguarded dynamic emit path.

---

## 4. Bisect anchor

`git log --oneline -- crates/shape-vm/src/executor/arithmetic/mod.rs` returns the `exec_dyn_bit_binary` author at:

- `cce5303a` — "executor: collapse dynamic fallback to minimal typed dispatch (V4.2+V4.3)" (2026-04-18) — body acknowledges `BitAnd/Or/Xor/Shl/Shr/Not` each kept ~2 fallback hits; the audit concluded "zero class-(c) compiler bugs" — that conclusion was wrong: the 8 fixtures here are exactly class-(c) (typed Shape code reaching the fallback because the compiler accepted non-int operands without proof).
- `f221d1f8` / `63eede8f` — "preserve Int numeric hint on dynamic bitwise ops" — cemented the dynamic fallback as the silent-correctness path.

R5.1C (typed-bitwise-enabled gate) inverted the polarity correctly for the **typed** path but left the else branch unguarded.

---

## 5. Affected subsystem (fix location)

Compile-time type-check gate must be added at one of:

- **Primary fix:** `crates/shape-vm/src/compiler/expressions/binary_ops.rs:1403-1407` (bitwise arm entry). Before reaching `emit_typed`/dynamic-fallback fork, prove both operands are `int` (or `int`-implementing user trait). On proof gap → `prove_native_kind() → ProofGap` → compile error per CLAUDE.md mechanical-enforcement rule. No runtime coercion (forbidden by CLAUDE.md §Type-System-Rules: "NO runtime coercion").

- **Supporting fix:** `crates/shape-vm/src/compiler/expressions/numeric_ops.rs:12-17` — extend `is_strict_arithmetic` (or add `is_strict_bitwise`) to cover `BitAnd | BitOr | BitXor | BitShl | BitShr` (plus `is_strict_unary_bitwise` for `BitNot`), then reuse the existing rejection-emit at `binary_ops.rs:1680`. This mirrors the existing strict-arithmetic pattern symmetrically.

- **Deletion target:** `crates/shape-vm/src/executor/arithmetic/mod.rs:637-658` — `exec_dyn_bit_binary` + `exec_dyn_bit_unary` discard kinds + reinterpret bits; with the compile-time gate added, the dynamic `BitAnd`/`BitOr`/`BitXor`/`BitShl`/`BitShr`/`BitNot` opcodes have no producer for non-int operands and should be deleted (matches the existing operator-trait dispatch path at `binary_ops.rs:1430-1436` for user `impl BitAnd for T` which already returns before the fallback). Surface-and-stop per ADR-006 §2.7.14 instead of bits-reinterpret.

Unary `~` lives at `crates/shape-vm/src/compiler/expressions/unary_ops.rs:28-66` — same shape (typed `BitNotInt` emitted only when operand proven int; else falls through to dynamic `BitNot`).

---

## 6. Sub-cluster

- **Name:** `v0.3.3-bitwise-strict-typing-gate`
- **Size:** **S** (one type-check gate + 6-opcode deletion). Approx scope:
  - Extend `is_strict_arithmetic` (or add sibling helper) — `numeric_ops.rs` ~5 lines.
  - Add bitwise-arm entry gate at `binary_ops.rs:1403` reusing the existing rejection-emit shape — ~10-15 lines.
  - Add unary-arm entry gate at `unary_ops.rs:28` — ~5-10 lines.
  - Delete `exec_dyn_bit_binary` + `exec_dyn_bit_unary` + their dispatch table entries — ~30 lines.
  - All 8 fixtures pass unchanged (they already `expect_run_err()`).
- **Smoke regressions to check:** user-type `impl BitAnd / BitOr / BitXor / Shl / Shr for T` (W1.9 + W1.10) — these dispatch BEFORE the dynamic fallback at `binary_ops.rs:1430-1536` so they should be unaffected; confirm via existing `test_return_bitwise_and` / `test_return_bitwise_shift` family + operator-trait fixtures.

---

## 7. Dependencies / overlap with cluster #4 (pointer-as-float)

**Same root family.** Cluster #4 (pointer-as-float, per the small-batch agent flag) and cluster #5 (this one) are both instances of the deleted-but-resurrected "raw-bits dispatch" pattern: kind-discarding `pop_kinded()` + `bits as <target_scalar>` reinterpret. The compile-time gate added here is independent of cluster #4's gate (different opcodes, different emit paths), but **both fixes must enforce the same discipline**: any opcode whose executor calls `pop_kinded()` and discards the kind is a CLAUDE.md §Forbidden-Code violation. Recommend a single audit pass over all `pop_kinded()` callers in `crates/shape-vm/src/executor/` after both clusters land, to catch sibling instances (DivDynamic, ModDynamic, GtDynamic etc. enumerated in `cce5303a`'s commit body still exist).

No ordering dependency — clusters #4 and #5 can be fixed in parallel.

---

## Disposition

8 / 8 fixtures: **FN-REG-CORRECTNESS — RELEASE-BLOCKING**. None route to V0.4-DEFER, SCOPE-RECLAIM, or FN-REG-DIAGNOSTIC. Fix is a single compile-time gate + dead-code deletion (~50 lines net), zero fixture changes.
