# Wave 2 Audit: Remaining Generic Opcode Emission Sites

Audit date: 2026-04-09
Auditor: Stage 2.6 JIT v2 audit

## Purpose

Catalog every remaining site in `crates/shape-vm/src/compiler/` that
emits a generic arithmetic or comparison opcode (`Add`, `Sub`, `Mul`,
`Div`, `Mod`, `Pow`, `Neg`, `Eq`, `Neq`, `Gt`, `Lt`, `Gte`, `Lte`),
classify each site, and determine what must happen before the generic
opcodes can be deleted from the opcode table and executor.

---

## 1. Emission Site Inventory

### 1a. `helpers.rs` -- Runtime fallback wrappers (category d)

| File:Line | Opcode | Category | Notes |
|-----------|--------|----------|-------|
| `helpers.rs:24` | `Add` | (d) helper | `emit_runtime_add()` -- centralized fallback for `+` when compiler cannot prove operand types |
| `helpers.rs:40` | `Eq` | (d) helper | `emit_runtime_eq()` -- centralized fallback for `==` when types unresolvable |
| `helpers.rs:55` | `Neq` | (d) helper | `emit_runtime_neq()` -- centralized fallback for `!=` when types unresolvable |

These three helper functions are the canonical runtime-dispatch emit
points. Eliminating them requires eliminating all callers first.

**Callers of `emit_runtime_add`:**
- `binary_ops.rs:718` -- DateTime/Duration addition (temporal types)
- `binary_ops.rs:827` -- NoPlan/CoercedNeedsGeneric fallback for `+` when both operand types are unknown

**Callers of `emit_runtime_eq`:**
- `binary_ops.rs:443` -- equality fallback when `compile_typed_equality` resolves neither operand type

**Callers of `emit_runtime_neq`:**
- `binary_ops.rs:441` -- inequality fallback when `compile_typed_equality` resolves neither operand type

### 1b. `expressions/binary_ops.rs` -- `generic_opcode_for()` dispatch table (category b)

| File:Line | Opcode | Category | Notes |
|-----------|--------|----------|-------|
| `binary_ops.rs:74` | `Sub` | (b) runtime dispatch | Returned by `generic_opcode_for()` for unresolvable Sub operands |
| `binary_ops.rs:75` | `Mul` | (b) runtime dispatch | Same, Mul |
| `binary_ops.rs:76` | `Div` | (b) runtime dispatch | Same, Div |
| `binary_ops.rs:77` | `Mod` | (b) runtime dispatch | Same, Mod |
| `binary_ops.rs:78` | `Pow` | (b) runtime dispatch | Same, Pow |
| `binary_ops.rs:84` | `Gt` | (b) runtime dispatch | Same, ordered comparison |
| `binary_ops.rs:85` | `Lt` | (b) runtime dispatch | Same, ordered comparison |
| `binary_ops.rs:86` | `Gte` | (b) runtime dispatch | Same, ordered comparison |
| `binary_ops.rs:87` | `Lte` | (b) runtime dispatch | Same, ordered comparison |

The `generic_opcode_for()` function is called from two sites in the `_`
(catch-all) arm of `compile_expr_binary_op`:
- Line 1043: `CoercedNeedsGeneric` -- coercion plan produced a type
  with no typed opcode variant
- Line 1066: `NoPlan` -- both slot tracker and inference engine failed
  to resolve operand types

Note: `Add`, `Equal`, `NotEqual` return `None` from `generic_opcode_for()`
-- they are already fully handled by typed dispatch + `emit_runtime_*`
helpers.

### 1c. `literals.rs` -- `compile_binary_op()` and `compile_unary_op()` (category a/b)

| File:Line | Opcode | Category | Notes |
|-----------|--------|----------|-------|
| `literals.rs:64` | `Add` | (b) runtime dispatch | Only reachable when `generic_opcode_for()` returns `None` and fallback calls `compile_binary_op(op)` (line 1046/1069 of binary_ops.rs). The `Add` arm is the only non-`unreachable!()` arithmetic opcode remaining. |
| `literals.rs:111` | `Neg` | (b) runtime dispatch | `compile_unary_op()` emits generic `Neg` unconditionally. The compiler does not currently specialize unary negation based on operand type at the expression level. |

The `compile_binary_op` function marks `Sub`, `Mul`, `Div`, `Mod`, `Pow`,
`Greater`, `Less`, `GreaterEq`, `LessEq`, `Equal`, `NotEqual` as
`unreachable!()`. These arms are dead code and confirm those ops should
never flow through `compile_binary_op` anymore.

**However**: The `Add` arm is still live because `binary_ops.rs:1046/1069`
calls `self.compile_binary_op(op)?` when `generic_opcode_for()` returns
`None`. For `BinaryOp::Add`, this code path is reachable (it's the same
as calling `emit_runtime_add`, just via a different codepath).

**`Neg` is live**: `compile_unary_op()` is called for all unary negation.
No typed negation dispatch exists at the expression level (the executor
has `NegInt`/`NegNumber`/`NegDecimal` handlers but the compiler always
emits generic `Neg`).

### 1d. `compiler_tests.rs` -- Test assertions (category c)

| File:Line | Opcode | Category | Notes |
|-----------|--------|----------|-------|
| `compiler_tests.rs:1568` | `Eq` | (c) test-only | Asserts `!opcodes.contains(&OpCode::Eq)` -- verifies typed EqInt is emitted instead |
| `compiler_tests.rs:1593` | `Neq` | (c) test-only | Asserts `!opcodes.contains(&OpCode::Neq)` -- verifies typed NeqNumber is emitted instead |
| `compiler_tests.rs:1670` | `Add` | (c) test-only | Asserts `!opcodes.contains(&OpCode::Add)` -- verifies no generic Add in numeric accumulation |
| `compiler_tests.rs:1748` | `Mul` | (c) test-only | Asserts `!opcodes.contains(&OpCode::Mul)` -- verifies no generic Mul for Mat*Vec path |

All four are negative assertions ("the generic opcode should NOT appear").
When generic opcodes are deleted from the enum, these assertions become
compile errors and should be removed.

### 1e. `patterns/helpers.rs` -- Documentation reference only (category: informational)

| File:Line | Opcode | Category | Notes |
|-----------|--------|----------|-------|
| `patterns/helpers.rs:14` | `Eq` (comment) | N/A | Doc comment mentioning the old `OpCode::Eq` pattern. No emission. |
| `patterns/helpers.rs:18` | `Eq` (comment) | N/A | Same. |

### 1f. `V2_TYPED_OPCODE_STATUS.md` -- Documentation reference only (category: informational)

| File:Line | Opcode | Category | Notes |
|-----------|--------|----------|-------|
| `V2_TYPED_OPCODE_STATUS.md:84` | `Add` (doc) | N/A | Reference in prior audit document |

---

## 2. Executor Handler Line Counts

### 2a. `executor/arithmetic/mod.rs` (3950 lines total)

Generic opcode handlers within `exec_arithmetic()`:

| Opcode | Start Line | End Line | Lines | Key dispatch responsibilities |
|--------|-----------|----------|-------|-------------------------------|
| `Add` | 1178 | 1673 | 496 | Numeric add, string concat, array concat, object merge, DateTime+Duration, Vec SIMD, Matrix add, BigInt, operator trait fallback |
| `Sub` | 1674 | 1913 | 240 | Numeric sub, DateTime-DateTime, DateTime-Duration, Vec SIMD, Matrix sub, BigInt, operator trait fallback |
| `Mul` | 1914 | 2182 | 269 | Numeric mul, Vec SIMD, Matrix matmul/matvec/scale, BigInt, operator trait fallback |
| `Div` | 2183 | 2442 | 260 | Numeric div, Vec SIMD, BigInt, zero-check, operator trait fallback |
| `Mod` | 2443 | 2560 | 118 | Numeric mod, BigInt, Decimal, zero-check |
| `Neg` | 2577 | 2610 | 34 | Unary negation: i48/f64/BigInt/Decimal, operator trait fallback |
| `Pow` | 2612 | 2715 | 104 | Numeric pow, BigInt, Decimal via f64 approximation |
| **Total** | | | **1521** | |

Typed arithmetic handlers (`exec_typed_arithmetic`, line 435) and compact
typed handlers (`exec_compact_typed_arithmetic`) are separate and will be
retained.

NegInt/NegNumber/NegDecimal handlers (lines 2561-2576) are in exec_arithmetic
but are typed -- they will be retained.

### 2b. `executor/comparison/mod.rs` (790 lines total)

Generic opcode handlers within `exec_comparison()`:

| Opcode | Start Line | End Line | Lines | Key dispatch responsibilities |
|--------|-----------|----------|-------|-------------------------------|
| `Gt` | 384 | 403 | 20 | ExprProxy (SQL pushdown), numeric compare, string compare |
| `Lt` | 405 | 422 | 18 | Same |
| `Gte` | 424 | 443 | 20 | Same |
| `Lte` | 445 | 464 | 20 | Same |
| `Eq` | 466 | 475 | 10 | ExprProxy, `vw_equals()` full-type dispatch |
| `Neq` | 477 | 484 | 8 | ExprProxy, `!vw_equals()` |
| **Total** | | | **96** | |

Typed comparison handlers (`exec_typed_comparison`, line 122) handle all
the `GtInt`/`GtNumber`/`GtDecimal`/etc. variants and will be retained.

### 2c. Combined savings estimate

Deleting all generic opcode handlers from the executor would save
approximately **1521 + 96 = 1617 lines** from the two executor modules.
Additionally, the helper infrastructure (`numeric_binary_result`,
`numeric_div_result`, `numeric_mod_result`, `numeric_pow_result`,
`materialize_float_slice`, `unwrap_annotated`, `numeric_domain`,
`dispatch_numeric_binary_with_zero_check`, `nb_compare_numeric`,
`try_nb_expr_proxy_compare`, operator trait helpers) accounts for an
additional ~500 lines that become dead code once all generic opcode
callers are removed.

**Estimated total executor savings: ~2100 lines.**

---

## 3. What Blocks Deletion of Each Generic Opcode

### 3a. `Add` -- the hardest to remove

The generic `Add` handler is the executor's Swiss Army knife. It handles:

1. **DateTime + Duration / Duration + DateTime / Duration + Duration** --
   Temporal arithmetic. The compiler already detects temporal types at line
   717 of binary_ops.rs and emits generic `Add` via `emit_runtime_add`.
   **Fix**: Emit `CallMethod("add")` for temporal types, with the executor
   routing through the DateTime method dispatch.

2. **String concatenation** -- Now handled by `StringConcat` opcode (Phase
   2.3). Generic `Add` is only reached when the compiler cannot prove both
   operands are strings. **Fix**: Improve inference or emit `StringConcat`
   more aggressively.

3. **Array concatenation** -- Now handled by `ArrayConcat` opcode (Phase
   2.4). Same situation as strings. **Fix**: Same approach.

4. **Object merge** -- Handled by `compile_typed_merge` when both schemas
   are known. Generic `Add` reached when schemas are unknown. **Fix**: Emit
   `CallMethod("add")` for unresolved objects.

5. **Vec SIMD (element-wise add)** -- Reached when `Vec<number>` or
   `Vec<int>` operands are not proven by the compiler. **Fix**: Dedicated
   `VecAdd` opcode or `CallMethod` dispatch.

6. **Operator trait dispatch (`impl Add for T`)** -- Already handled at
   compile time by `emit_operator_trait_call` when the type is known.
   Generic `Add` is the fallback when the type is unknown. **Fix**: If
   the compiler can't resolve the type, `CallMethod("add")` is the
   correct fallback anyway.

7. **Untyped function parameters** -- `param_locals` guard forces
   `left_numeric = None`. The parameter could be any type at runtime.
   **Fix**: This is the fundamental blocker. Without whole-program type
   analysis or specialization, untyped params must use runtime dispatch.
   The correct replacement is `CallMethod` for all operator overloading.

### 3b. `Sub`, `Mul`, `Div` -- operator traits + temporal types

These are emitted via `generic_opcode_for()` when:
- Both operand types are unresolvable (NoPlan from inference)
- Coercion produced a type with no typed opcode (CoercedNeedsGeneric)
- DateTime subtraction (Sub only)
- Vector/Matrix operations on unproven types

**Fix for all three**: Replace `generic_opcode_for()` with `CallMethod`
dispatch for the operator trait method (`sub`, `mul`, `div`). The
executor's `try_binary_operator_trait` already knows how to find
`TypeName::method_name` in the function index.

### 3c. `Mod`, `Pow` -- no operator traits, pure numeric

`Mod` and `Pow` have no operator trait variants. They are emitted when:
- Both operand types are unresolvable
- CoercedNeedsGeneric (rare)

**Fix**: For proven-numeric cases, ensure inference resolves the type.
For truly unknown cases, a `CallMethod("mod"/"pow")` would work but
requires adding `Mod`/`Pow` operator traits to the language. Alternatively,
a runtime numeric-only dispatch intrinsic could replace the generic opcode.

### 3d. `Neg` -- no typed dispatch at expression level

The compiler unconditionally emits generic `Neg` via `compile_unary_op()`.
The executor has `NegInt`/`NegNumber`/`NegDecimal` handlers, but no
compiler path routes to them for general expressions.

**Fix**: Add typed negation dispatch in `compile_unary_op()` or in the
unary expression compilation. Check `last_expr_numeric_type` after
compiling the operand, and emit `NegInt`/`NegNumber`/`NegDecimal`
accordingly. This is a straightforward 10-20 line change.

### 3e. `Eq`, `Neq` -- almost eliminated

`compile_typed_equality` handles the vast majority of cases. Generic
`Eq`/`Neq` is only emitted when:
- Both operand types are completely unresolvable by inference AND
  literal analysis

**Fix**: Extend `resolve_eq_type` to handle more cases (boolean equality,
enum equality, etc.). The remaining unresolvable cases can fall back to
`CallMethod("eq")` if an `Eq` trait is added, or keep a minimal runtime
`vw_equals` dispatch.

### 3f. `Gt`, `Lt`, `Gte`, `Lte` -- same as Eq/Neq

Generic ordered comparisons are emitted via `generic_opcode_for()` when:
- Both operand types are unresolvable
- String comparisons (no typed string comparison opcode exists)
- ExprProxy comparisons (SQL pushdown)

**Fix**: Add `GtString`/`LtString`/`GteString`/`LteString` opcodes for
proven string comparisons. ExprProxy comparisons are rare and could use
a dedicated opcode or `CallMethod` dispatch. Unresolvable types need
`CallMethod("cmp")` or similar.

---

## 4. Recommended Order of Operations

### Phase 1: Quick wins (low risk, high impact)

1. **Typed `Neg` dispatch** -- Add `NegInt`/`NegNumber`/`NegDecimal`
   emission in `compile_unary_op()` based on `last_expr_numeric_type`.
   Eliminates almost all generic `Neg` emission.
   *Estimated: 15 lines changed, saves 34 executor lines.*

2. **Temporal `Add` to `CallMethod("add")`** -- Change line 718 of
   binary_ops.rs from `emit_runtime_add` to `emit_operator_trait_call`.
   Requires DateTime/Duration to have an `add` method registered.
   *Estimated: 5 lines changed.*

3. **Delete test-only assertions (category c)** -- Remove the four
   negative assertions in compiler_tests.rs. These can be deleted now
   as they only verify the absence of generic opcodes.
   *Estimated: 16 lines removed.*

### Phase 2: Replace `generic_opcode_for()` with `CallMethod` (medium risk)

4. **Operator trait `CallMethod` fallback for Sub/Mul/Div** -- When
   `CoercedNeedsGeneric` or `NoPlan`, emit `CallMethod("sub"/"mul"/"div")`
   instead of the generic opcode. Requires ensuring all numeric types
   have these methods registered in the method dispatch table.
   *Estimated: 30 lines changed.*

5. **Temporal `Sub` to `CallMethod("sub")`** -- DateTime-DateTime and
   DateTime-Duration subtraction need method dispatch.
   *Estimated: 10 lines changed.*

6. **String comparison opcodes** -- Add `GtString`/`LtString`/
   `GteString`/`LteString` to handle proven string comparisons.
   *Estimated: 60 lines added (executor), 20 lines changed (compiler).*

### Phase 3: Eliminate unresolvable-type fallbacks (high risk)

7. **Extend inference coverage** -- Improve `infer_expr_type` and
   `resolve_eq_type` to handle more cases, reducing the frequency
   of "both types unknown" scenarios.

8. **`CallMethod` universal fallback** -- Replace all remaining
   `generic_opcode_for()` returns with `CallMethod` dispatch. This
   is the endgame: every binary operation becomes either a typed
   opcode or a method call.

9. **Delete generic opcodes from enum** -- Once no compiler path emits
   them, remove `Add`/`Sub`/`Mul`/`Div`/`Mod`/`Pow`/`Neg`/`Eq`/`Neq`/
   `Gt`/`Lt`/`Gte`/`Lte` from the `OpCode` enum.

10. **Delete executor handlers** -- Remove `exec_arithmetic` and
    `exec_comparison` methods, along with all helper infrastructure.
    *Estimated savings: ~2100 lines.*

### Phase 4: Cleanup

11. **Delete `generic_opcode_for()`** -- No longer needed.
12. **Delete `emit_runtime_add/eq/neq`** -- No longer needed.
13. **Delete `compile_binary_op()`** -- All arms are `unreachable!()`.
14. **Update V2_TYPED_OPCODE_STATUS.md** -- Mark audit as complete.
15. **Update compiler_tests.rs** -- Remove or rewrite tests that
    reference deleted opcodes.

---

## 5. Summary Table

| Opcode | Emission sites | Category breakdown | Blocking issue |
|--------|---------------|-------------------|----------------|
| `Add` | 3 (helpers.rs:24, binary_ops.rs:718/827, literals.rs:64) | (b) runtime dispatch, (d) helper | DateTime, strings, arrays, objects, untyped params, operator traits |
| `Sub` | 1 (binary_ops.rs:74 via generic_opcode_for) | (b) runtime dispatch | DateTime subtraction, operator traits, unresolvable types |
| `Mul` | 1 (binary_ops.rs:75 via generic_opcode_for) | (b) runtime dispatch | Vec/Matrix on unproven types, operator traits, unresolvable types |
| `Div` | 1 (binary_ops.rs:76 via generic_opcode_for) | (b) runtime dispatch | Operator traits, unresolvable types |
| `Mod` | 1 (binary_ops.rs:77 via generic_opcode_for) | (b) runtime dispatch | No operator trait, unresolvable types only |
| `Pow` | 1 (binary_ops.rs:78 via generic_opcode_for) | (b) runtime dispatch | No operator trait, unresolvable types only |
| `Neg` | 1 (literals.rs:111) | (b) runtime dispatch | No typed dispatch at expression level (quick fix) |
| `Eq` | 1 (helpers.rs:40) | (d) helper | Both operands unresolvable by inference |
| `Neq` | 1 (helpers.rs:55) | (d) helper | Both operands unresolvable by inference |
| `Gt` | 1 (binary_ops.rs:84 via generic_opcode_for) | (b) runtime dispatch | String comparison, ExprProxy, unresolvable types |
| `Lt` | 1 (binary_ops.rs:85 via generic_opcode_for) | (b) runtime dispatch | Same |
| `Gte` | 1 (binary_ops.rs:86 via generic_opcode_for) | (b) runtime dispatch | Same |
| `Lte` | 1 (binary_ops.rs:87 via generic_opcode_for) | (b) runtime dispatch | Same |

**Test assertions (category c):** 4 sites in compiler_tests.rs
(lines 1568, 1593, 1670, 1748) -- all negative assertions, safe to
remove when opcodes are deleted.

**Documentation references:** 3 sites (patterns/helpers.rs:14/18,
V2_TYPED_OPCODE_STATUS.md:84) -- comments only, no emission.
