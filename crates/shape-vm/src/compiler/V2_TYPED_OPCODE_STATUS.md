# Typed Opcode Emission Status

Initial audit date: 2026-04-01
Last updated: 2026-04-09 (Wave 1 complete)

## Summary

The compiler has a well-structured typed opcode dispatch system in
`expressions/numeric_ops.rs` with `typed_opcode_for()` and a two-tier
fallback (slot tracking -> inference engine -> generic opcode). This
audit catalogs every site that emits a generic arithmetic/comparison
opcode and classifies whether conversion to typed is feasible.

### Wave 1 Completion Status (Stage 2.6)

Wave 1 is complete. All items below were landed in the `jit-v2-phase1`
branch. Test count increased from 1768 to 1825 (monomorphization tests
added).

| Item | Status |
|------|--------|
| Typed loop counters (iterator `__idx` as `Int(0)` + `AddInt`) | DONE |
| Typed arith/cmp in `literals.rs` (`Sub`/`Mul`/`Div`/`Mod`/`Pow`/`Gt`/`Lt`/`Gte`/`Lte`/`Eq`/`Neq` arms marked `unreachable!()`) | DONE |
| Fuzzy comparison desugaring (`*Number` variants) | DONE |
| Typed `Neg` dispatch (`NegInt`/`NegNumber`/`NegDecimal` opcodes added + executor handlers) | DONE |
| Typed `Eq` dispatch (`compile_typed_equality` with inference-driven resolution) | DONE |
| `Add` fallback structured dispatch (`StringConcat`, `ArrayConcat`, `CallMethod` for operator traits) | DONE |
| `Eq`/`Neq` unreachable in `literals.rs` | DONE |
| `IsNull` opcode replacing sentinel `PushNull + Eq` pattern (16 sites) | DONE |
| `EqString` opcode for string equality comparison | DONE |
| `EqDecimal` opcode for decimal equality comparison | DONE |
| `NegDecimal` opcode for decimal negation | DONE |
| JIT consumer cleanup (new opcodes handled in `shape-jit`) | DONE |
| Monomorphization module integrated (was orphaned; now wired into method call dispatch) | DONE |
| `impl Iterable` cleanup (untyped `includes` removed, typed `Vec<T>.includes` exists) | DONE |

### New Opcodes Added in Wave 1

| Opcode | Byte | Category | Description |
|--------|------|----------|-------------|
| `NegInt` | `0xCA` | Arithmetic | Typed negation for `int` values |
| `NegNumber` | `0xCB` | Arithmetic | Typed negation for `number` values |
| `NegDecimal` | `0xCC` | Arithmetic | Typed negation for `decimal` values |
| `IsNull` | `0xF2` | Comparison | Replaces `PushNull + Eq` pattern, tests single operand for null |
| `EqString` | `0xFE` | Comparison | Typed equality for non-null `StringObj` pointers |
| `EqDecimal` | `0xFF` | Comparison | Typed equality for non-null `DecimalObj` pointers |

### Remaining Generic Emission (Wave 2 scope)

A full inventory of remaining generic opcode emission sites is in
`V2_AUDIT_WAVE2.md`. Key remaining areas:

1. **`generic_opcode_for()` in `binary_ops.rs`** -- Returns generic
   `Sub`/`Mul`/`Div`/`Mod`/`Pow`/`Gt`/`Lt`/`Gte`/`Lte` when both
   operand types are unresolvable. Also covers DateTime/operator trait
   fallbacks.

2. **`helpers.rs` runtime dispatch** -- `emit_runtime_add()`,
   `emit_runtime_eq()`, `emit_runtime_neq()` are centralized fallback
   helpers still emitting generic `Add`/`Eq`/`Neq` when the compiler
   cannot prove operand types.

3. **`compile_binary_op` `Add` arm in `literals.rs`** -- The only
   non-`unreachable!()` arithmetic opcode remaining in the legacy
   dispatch function. Reached when `generic_opcode_for()` returns `None`
   for `Add`.

Wave 2 plan: replace `generic_opcode_for()` with `CallMethod` dispatch
for operator traits, add string comparison opcodes, extend inference
coverage, then delete all generic opcodes from the enum and executor
(estimated ~2100 lines of executor code removable).

---

## 1. Expressions That ALWAYS Emit Typed Opcodes

These paths exclusively emit typed opcodes when type information is
available, with generic fallback only when types cannot be proven:

| Expression | Typed path | Generic fallback trigger |
|---|---|---|
| `Sub`, `Mul`, `Div`, `Mod`, `Pow` (strict arithmetic) | `emit_numeric_binary_with_coercion_trusted` -> `typed_opcode_for` | Both operands lack type info from slot tracker AND inference engine |
| `Gt`, `Lt`, `Gte`, `Lte` (ordered comparisons) | Same as above | Same as above |
| `Equal`, `NotEqual` on numeric operands | `typed_opcode_for` returns `EqInt`/`EqNumber`/`EqDecimal`/`NeqInt`/`NeqNumber` | Non-numeric operands without typed opcode (Wave 1 added `EqDecimal`, `EqString`, `IsNull`) |
| Width-typed arithmetic (`u8 + u8`, `i16 * i16`) | `AddTyped`/`SubTyped` with `Width` operand | Never falls back — IncompatibleWidths is a compile error |
| Range counter loops (typed path) | `LtInt`/`LteInt`, `AddInt` | Only when `use_typed=false` |
| Mat * Vec / Mat * Mat | Lowers to `IntrinsicMatMulVec`/`IntrinsicMatMulMat` | N/A (not generic arithmetic) |

### Literal arithmetic proof

- **`1 + 2`** -> Both literals set `last_expr_numeric_type = Some(Int)`.
  `is_expr_confirmed_numeric` returns true for both. Result: **`AddInt`**.
- **`1.0 + 2.0`** -> Both literals set `last_expr_numeric_type = Some(Number)`.
  Result: **`AddNumber`**.
- **`let x: int = 5; x + 1`** -> `x` gets `StorageHint::Int64` from type
  annotation. `storage_hint_for_expr` returns `Int64`. Literal `1` is confirmed
  numeric. Result: **`AddInt`**.

### Annotated variable arithmetic proof

- **`let x: int = 5; x + 1`** -> `storage_hint_for_expr(x)` = `Int64` (from
  annotation). `is_expr_confirmed_numeric(1)` = true. Both confirmed.
  Result: **`AddInt`**.
- **`let x: number = 5.0; x + 1.0`** -> Same mechanism. Result: **`AddNumber`**.

---

## 2. Expressions That SOMETIMES Fall Back to Generic

### 2a. `BinaryOp::Add` (line 395-491 of binary_ops.rs)

Add is overloaded: numeric add, string concat, array concat, object merge.
The compiler requires **double confirmation** that both operands are numeric
before emitting `AddInt`/`AddNumber`:

1. Syntactic literal check (`is_expr_confirmed_numeric`)
2. Storage hint check (`storage_hint_for_expr`)
3. Non-identifier expression with inferred type

If neither operand passes these gates, generic `Add` is emitted. This is
**correct by design** since `Add` genuinely handles non-numeric types.

**Root causes of generic `Add` fallback:**
- String concatenation: `"a" + "b"` -> must use generic `Add`
- Array concatenation: `[1] + [2]` -> must use generic `Add`
- Object merge: `obj1 + obj2` -> must use generic `Add`
- Operator trait dispatch: type with `impl Add` -> generic `Add`
- Untyped function parameters: `param_locals` guard forces `None` numeric type
- Variables inferred but not annotation-confirmed (see section 3)

### 2b. Range counter loops (lines 104-158 of loops.rs)

Two paths: `use_typed=true` emits `LtInt/LteInt` + `AddInt`,
`use_typed=false` emits `Lt/Lte` + `Add`.

`use_typed` is false when EITHER range endpoint has
`last_expr_numeric_type = None` (unresolvable type). This happens when
the range start/end is a complex expression whose type cannot be tracked.

### 2c. Generic iterator loops -- RESOLVED (Wave 1)

~~The generic `for x in iterable` path used an internal `__idx` counter
initialized to `Constant::Number(0.0)` and incremented with generic
`OpCode::Add`.~~

**Fixed in Stage 2.6**: All iterator index counters now use typed
`Int(0)` initialization and `AddInt` increment. Range counter loops
use `LtInt`/`LteInt` for all resolved cases.

### 2d. Fuzzy comparisons -- RESOLVED (Wave 1)

~~All fuzzy comparison desugaring emitted generic opcodes (`Sub`, `Lt`,
`Neg`, `Add`, `Div`, `Lte`, `Gt`). ~30 generic opcode emissions.~~

**Fixed in Stage 2.6**: All fuzzy comparison opcodes replaced with
typed `*Number` variants (`SubNumber`, `LtNumber`, `GtNumber`,
`LteNumber`, `AddNumber`, `DivNumber`, `NegNumber`).

### 2e. String interpolation -- RESOLVED (Wave 1)

~~One generic `Add` for string concatenation in interpolated strings.~~

**Fixed in Stage 2.3**: String interpolation now uses `StringConcat`
opcode.

---

## 3. What Blocks 100% Typed Opcode Emission for Arithmetic

### 3a. Variables WITHOUT annotations but with inferable types

**`let x = 5; x + 1`**:
- The literal `5` sets `last_expr_numeric_type = Some(Int)` during RHS compilation.
- The `declare_local` + `StoreLocal` step propagates this to the type tracker
  as `StorageHint::Int64`.
- When `x + 1` is compiled, `storage_hint_for_expr(x)` returns `Int64`.
- `is_expr_confirmed_numeric(1)` returns true.
- Result: **`AddInt`** — this DOES get typed opcodes.

**`let x = some_fn(); x + 1`**:
- If `some_fn()` return type is not tracked, `x` gets `StorageHint::Unknown`.
- `storage_hint_for_expr(x)` returns `None`.
- Falls through to inference engine (`infer_numeric_pair`).
- If inference succeeds, typed opcode; if not, generic fallback.

### 3b. Function parameters without type annotations

`param_locals` guard (binary_ops.rs:579-592) explicitly zeroes out numeric
type info for parameters that are function locals. This is **intentional** —
`inferred_param_type_hints` can be wrong (B19 bug class). Only parameters
with explicit `: int`/`: number` type annotations get typed opcodes.

### 3c. `Add` overloading

`Add` is the only arithmetic op that is genuinely polymorphic (strings,
arrays, objects, numbers). The other five (`Sub`, `Mul`, `Div`, `Mod`, `Pow`)
are numeric-only and can always use typed opcodes when types are proven.

### 3d. Equality/inequality on non-numeric types -- MOSTLY RESOLVED (Wave 1)

~~`Eq`/`Neq` were used for null checks, unit sentinel checks, pattern
matching, and optional chaining.~~

**Fixed in Stage 2.6.5**: Null checks (16 sites) replaced with `IsNull`
opcode. Pattern matching uses `typed_eq_for_literal()` to emit typed
`EqInt`/`EqNumber`/`EqString` based on the matched literal type.
String equality uses `EqString`. Only truly unresolvable cases still
fall through to generic `Eq`/`Neq` via `emit_runtime_eq`/`emit_runtime_neq`.

### 3e. Missing typed `Eq`/`Neq` for Decimal -- RESOLVED (Wave 1)

~~`typed_opcode_for` returned `None` for `(Equal, Decimal)` and
`(NotEqual, Decimal)`. The `EqDecimal`/`NeqDecimal` opcodes did not exist.~~

**Fixed in Stage 2.6.3**: `EqDecimal` opcode added (`0xFF`). `NeqDecimal`
handled by `EqDecimal` + `Not`.

### 3f. Decimal storage hints dropped entirely

In `helpers.rs:885-893`, `propagate_assignment_type_to_slot` deliberately
stores `VariableTypeInfo::unknown()` for `NumericType::Decimal`, meaning
`let x: decimal = 1.5m; x + 1.0m` will NOT get typed opcodes because the
slot tracker loses the decimal type after assignment. Comment says
"Decimal typed opcodes are not JIT-compiled yet."

### 3g. `Number` hint gated by `allow_number_hint`

In `helpers.rs:871-883`, the `Number` storage hint is only stored when
`allow_number_hint = true`. All current call sites pass `true`, so this
is not currently blocking anything, but it's a guard rail that could
block typed emission if callers change.

---

## 4. Generic Opcode Emission Sites (post-Wave 1)

The "improvable" sites from the original audit (sections 4 old) are now
resolved. The following remain as the complete set of generic emission
sites. See `V2_AUDIT_WAVE2.md` for the detailed inventory.

### Still emitted (runtime dispatch fallbacks)

| File | Line | Opcode | Reason | Wave 2 fix |
|---|---|---|---|---|
| `helpers.rs` | 24 | `Add` | `emit_runtime_add()` fallback | Replace with `CallMethod("add")` |
| `helpers.rs` | 40 | `Eq` | `emit_runtime_eq()` fallback | Extend `resolve_eq_type` coverage |
| `helpers.rs` | 55 | `Neq` | `emit_runtime_neq()` fallback | Same |
| `binary_ops.rs` | 74-87 | `Sub`/`Mul`/`Div`/`Mod`/`Pow`/`Gt`/`Lt`/`Gte`/`Lte` | `generic_opcode_for()` table | Replace with `CallMethod` dispatch |
| `literals.rs` | 64 | `Add` | Last non-unreachable arm in `compile_binary_op` | Delete when `emit_runtime_add` is gone |
| `literals.rs` | 111 | `Neg` | `compile_unary_op()` generic dispatch | Add typed negation dispatch at expression level |

### Eliminated in Wave 1

| Former site | What replaced it |
|---|---|
| `loops.rs` (8 sites) — iterator index counters | `Int(0)` + `AddInt` |
| `binary_ops.rs` (30 sites) — fuzzy comparisons | `*Number` typed variants |
| `string_interpolation.rs` — string concat | `StringConcat` opcode |
| `binary_ops.rs` — null-coalescing null check | `IsNull` opcode |
| `expressions/mod.rs` — annotation null checks | `IsNull` opcode |
| `property_access.rs` — optional chaining null checks | `IsNull` opcode |
| `functions.rs` — default param sentinel check | `IsNull` opcode |
| `functions_annotations.rs` — annotation pipeline null checks | `IsNull` opcode |
| `patterns/checking.rs` — pattern match equality | `typed_eq_for_literal()` |
| `patterns/binding.rs:107` — enum variant tag check | `typed_eq_for_literal()` |
| `binary_ops.rs` — operator trait dispatch | `CallMethod` via `emit_operator_trait_call` |

---

## 5. Remaining Work (Wave 2 scope)

The P1/P2/P3 items from the original audit are all resolved. Remaining
work to eliminate ALL generic opcodes is documented in `V2_AUDIT_WAVE2.md`.

### Quick wins

1. **Typed `Neg` at expression level** -- The executor has `NegInt`/
   `NegNumber`/`NegDecimal` handlers but `compile_unary_op()` still
   emits generic `Neg`. Fix: check `last_expr_numeric_type` after
   compiling operand. ~15 lines.

2. **Temporal `Add`/`Sub` to `CallMethod`** -- DateTime/Duration
   arithmetic still calls `emit_runtime_add`. Route through method
   dispatch instead.

### Medium effort

3. **Replace `generic_opcode_for()` with `CallMethod` fallback** --
   For `Sub`/`Mul`/`Div` and ordered comparisons when types are
   unresolvable.

4. **String comparison opcodes** -- `GtString`/`LtString`/etc. for
   proven string comparisons.

### Endgame

5. **Delete generic opcodes from `OpCode` enum** -- Once no compiler
   path emits them.

6. **Delete executor handlers** -- Remove `exec_arithmetic` and
   `exec_comparison` generic dispatch (~2100 lines).

---

## 6. Opcode Coverage Matrix

| Operation | Int | Number | Decimal | IntWidth | String | Null | Non-numeric |
|---|---|---|---|---|---|---|---|
| Add | AddInt | AddNumber | AddDecimal | AddTyped | StringConcat | -- | Add (generic) |
| Sub | SubInt | SubNumber | SubDecimal | SubTyped | -- | -- | -- |
| Mul | MulInt | MulNumber | MulDecimal | MulTyped | -- | -- | -- |
| Div | DivInt | DivNumber | DivDecimal | DivTyped | -- | -- | -- |
| Mod | ModInt | ModNumber | ModDecimal | ModTyped | -- | -- | -- |
| Pow | PowInt | PowNumber | PowDecimal | -- | -- | -- | -- |
| Gt | GtInt | GtNumber | GtDecimal | GtInt | -- | -- | Gt (generic) |
| Lt | LtInt | LtNumber | LtDecimal | LtInt | -- | -- | Lt (generic) |
| Gte | GteInt | GteNumber | GteDecimal | GteInt | -- | -- | Gte (generic) |
| Lte | LteInt | LteNumber | LteDecimal | LteInt | -- | -- | Lte (generic) |
| Eq | EqInt | EqNumber | EqDecimal | EqInt | EqString | IsNull | Eq (generic) |
| Neq | NeqInt | NeqNumber | *(via EqDecimal+Not)* | NeqInt | *(via EqString+Not)* | *(via IsNull+Not)* | Neq (generic) |
| Neg | NegInt | NegNumber | NegDecimal | -- | -- | -- | Neg (generic) |

**Closed gaps (Wave 1)**: `EqDecimal`, `EqString`, `NegInt`, `NegNumber`,
`NegDecimal`, `IsNull` all added. `NeqDecimal`/`NeqString` handled by
`EqDecimal`/`EqString` + `Not`.

**Remaining gaps**: `Pow` has no `IntWidth` variant (`PowTyped`). No typed
string comparison opcodes (`GtString`/`LtString`/`GteString`/`LteString`).
Generic fallbacks still exist for all operations when both operand types
are unresolvable (Wave 2 scope).
