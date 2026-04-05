# Typed Opcode Emission Status

Audit date: 2026-04-01

## Summary

The compiler has a well-structured typed opcode dispatch system in
`expressions/numeric_ops.rs` with `typed_opcode_for()` and a two-tier
fallback (slot tracking -> inference engine -> generic opcode). This
audit catalogs every site that emits a generic arithmetic/comparison
opcode and classifies whether conversion to typed is feasible.

---

## 1. Expressions That ALWAYS Emit Typed Opcodes

These paths exclusively emit typed opcodes when type information is
available, with generic fallback only when types cannot be proven:

| Expression | Typed path | Generic fallback trigger |
|---|---|---|
| `Sub`, `Mul`, `Div`, `Mod`, `Pow` (strict arithmetic) | `emit_numeric_binary_with_coercion_trusted` -> `typed_opcode_for` | Both operands lack type info from slot tracker AND inference engine |
| `Gt`, `Lt`, `Gte`, `Lte` (ordered comparisons) | Same as above | Same as above |
| `Equal`, `NotEqual` on numeric operands | `typed_opcode_for` returns `EqInt`/`EqNumber`/`NeqInt`/`NeqNumber` | Decimal equality not covered; non-numeric operands |
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

### 2c. Generic iterator loops (lines 450-460, 830-840 of loops.rs)

The generic `for x in iterable` path uses an internal `__idx` counter
initialized to `Constant::Number(0.0)` and incremented with generic
`OpCode::Add`. This is an **internal iteration counter**, not user
arithmetic — the index type is always `number` but the compiler does not
prove this because the counter is initialized in an opaque way.

**Affected sites (6 total):**
- `loops.rs:158` — range counter loop, `use_typed=false` arm
- `loops.rs:460` — generic for-in loop index increment
- `loops.rs:840` — generic for-expr loop index increment
- `loops.rs:1091` — comprehension clause iterator index increment
- `loops.rs:1224` — spread-over-range, `use_typed=false` arm
- `loops.rs:1303` — spread generic iterator index increment

### 2d. Fuzzy comparisons (lines 702-988 of binary_ops.rs)

All fuzzy comparison desugaring emits generic opcodes (`Sub`, `Lt`, `Neg`,
`Add`, `Div`, `Lte`, `Gt`). There are ~30 generic opcode emissions in the
fuzzy comparison lowering.

**Root cause:** Fuzzy comparisons always operate on `number`-typed values
(tolerance is `f64`) but the desugaring code does not consult or propagate
numeric type information. It unconditionally emits generic opcodes.

### 2e. String interpolation (string_interpolation.rs:209)

One generic `Add` for string concatenation in interpolated strings. This is
**correct** — string concat requires the generic `Add` path.

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

### 3d. Equality/inequality on non-numeric types

`Eq`/`Neq` are used for:
- Null checks (`PushNull` + `Eq`) — 15+ sites
- Unit sentinel checks — 2+ sites  
- Pattern matching value checks — 5+ sites
- Optional chaining null checks — 2 sites

These cannot be converted to `EqInt`/`EqNumber` because the operands are
not numeric. They compare against `null`, `unit`, strings, enum tags, etc.

### 3e. Missing typed `Eq`/`Neq` for Decimal

`typed_opcode_for` returns `None` for `(Equal, Decimal)` and
`(NotEqual, Decimal)`. The `EqDecimal`/`NeqDecimal` opcodes do not exist
in the opcode table.

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

## 4. Complete List of Generic Opcode Emission Sites

### Structurally required (cannot be eliminated)

| File | Line | Opcode | Reason |
|---|---|---|---|
| `literals.rs` | 62-104 | All generic opcodes | Fallback entry point called from binary_ops.rs when types unknown |
| `string_interpolation.rs` | 209 | `Add` | String concatenation |
| `binary_ops.rs` | 321 | `Eq` | Null-coalescing null check |
| `binary_ops.rs` | 429 | `Add` | Operator trait dispatch (type implements `Add`) |
| `binary_ops.rs` | 485 | `Add` | Generic Add fallback (unproven types or non-numeric) |
| `expressions/mod.rs` | 386, 405, 423 | `Eq` | Null checks in annotation `@before` dispatch |
| `property_access.rs` | 335, 340 | `Eq` | Optional chaining null/unit checks |
| `functions.rs` | 1078 | `Eq` | Default parameter sentinel check |
| `functions_annotations.rs` | 1601, 1619, 1637, 1722 | `Eq` | Annotation pipeline null checks |
| `patterns/checking.rs` | 103, 122, 196, 214, 350 | `Eq` | Pattern match value equality |
| `patterns/binding.rs` | 107 | `Eq` | Enum variant tag check |
| `patterns/binding.rs` | 132 | `Lt` | Array length check |

### Improvable (could emit typed opcodes with work)

| File | Line | Opcode | What's needed |
|---|---|---|---|
| `loops.rs` | 112, 114 | `Lte`/`Lt` | Range counter: propagate numeric proof when `use_typed=false` |
| `loops.rs` | 158 | `Add` | Range counter increment: same |
| `loops.rs` | 460 | `Add` | For-in loop `__idx` counter: init as `Int(0)`, use `AddInt` |
| `loops.rs` | 840 | `Add` | For-expr loop `__idx` counter: same |
| `loops.rs` | 1091 | `Add` | Comprehension iterator index: same |
| `loops.rs` | 1184, 1186 | `Lte`/`Lt` | Spread-range: same as range counter |
| `loops.rs` | 1224 | `Add` | Spread-range increment: same |
| `loops.rs` | 1303 | `Add` | Spread generic iterator index: same |
| `binary_ops.rs` | 739-979 | `Sub`, `Lt`, `Neg`, `Add`, `Div`, `Lte`, `Gt` | Fuzzy comparisons: ~30 sites, all on `number` constants |

---

## 5. Changes Needed to Eliminate ALL Improvable Generic Opcodes

### P1: Generic iterator index counters (8 sites, easy)

Change the internal `__idx` counter from `Constant::Number(0.0)` to
`Constant::Int(0)` and use `AddInt` for increment. These are internal
counters that are always integer-valued. Affected:
- `loops.rs` lines 394, 455-460 (for-in loop)
- `loops.rs` lines 697, 835-840 (for-expr loop)
- `loops.rs` lines 1086-1091 (comprehension)
- `loops.rs` lines 1252, 1298-1303 (spread generic iterator)

**Risk**: Low. The iterator index is compared via `IterDone` which
accepts both int and number. The `IterNext` opcode uses the index
for array indexing, which also works with integers.

### P2: Fuzzy comparison desugaring (~30 sites, medium)

All fuzzy comparison constants are `Number` literals and the temp locals
are always `number`-valued. Replace every generic opcode in
`compile_expr_fuzzy_comparison` with its `*Number` variant:
- `Sub` -> `SubNumber`
- `Lt` -> `LtNumber`
- `Gt` -> `GtNumber`
- `Lte` -> `LteNumber`
- `Add` -> `AddNumber`
- `Div` -> `DivNumber`
- `Neg` -> already generic (no typed Neg variant exists)

**Risk**: Low. Fuzzy comparisons are defined over floating-point
tolerance values, so all operands are provably `number`.

### P3: Range counter `use_typed=false` fallback (6 sites, medium)

When range endpoints come from expressions that don't set
`last_expr_numeric_type`, the range counter falls back to generic. Fix
by consulting the inference engine as a second-pass fallback (similar to
the binary_ops `NoPlan` -> `infer_numeric_pair` pattern).

**Risk**: Medium. Need to ensure the inference engine can resolve the
endpoint types. Non-numeric ranges (e.g., date ranges) should not
get typed int opcodes.

### Not eliminable

- Generic `Eq`/`Neq` for null/unit checks (15+ sites): These compare
  against sentinel values, not numeric operands. Would need a dedicated
  `EqNull`/`IsNull` opcode to eliminate.
- Generic `Add` for string/array/object concat: Requires overloaded
  semantics at runtime. Could be split into `StringConcat`/`ArrayConcat`
  opcodes but that's a different optimization.
- Generic `Add` for operator trait dispatch: The type implements a
  user-defined `Add` trait; the executor resolves at runtime.
- `patterns/binding.rs:132` `Lt` for array length: Length is always int
  but the constant is `Number(patterns.len() as f64)`. Could change
  to `Int` constant + `LtInt`.

---

## 6. Opcode Coverage Matrix

| Operation | Int | Number | Decimal | IntWidth | Non-numeric |
|---|---|---|---|---|---|
| Add | AddInt | AddNumber | AddDecimal | AddTyped | Add (generic) |
| Sub | SubInt | SubNumber | SubDecimal | SubTyped | -- |
| Mul | MulInt | MulNumber | MulDecimal | MulTyped | -- |
| Div | DivInt | DivNumber | DivDecimal | DivTyped | -- |
| Mod | ModInt | ModNumber | ModDecimal | ModTyped | -- |
| Pow | PowInt | PowNumber | PowDecimal | -- | -- |
| Gt | GtInt | GtNumber | GtDecimal | GtInt | Gt (generic) |
| Lt | LtInt | LtNumber | LtDecimal | LtInt | Lt (generic) |
| Gte | GteInt | GteNumber | GteDecimal | GteInt | Gte (generic) |
| Lte | LteInt | LteNumber | LteDecimal | LteInt | Lte (generic) |
| Eq | EqInt | EqNumber | **MISSING** | EqInt | Eq (generic) |
| Neq | NeqInt | NeqNumber | **MISSING** | NeqInt | Neq (generic) |
| Neg | -- | -- | -- | -- | Neg (generic) |

**Gaps**: `EqDecimal`, `NeqDecimal`, `NegInt`, `NegNumber`, `NegDecimal`
do not exist. `Pow` has no `IntWidth` variant (`PowTyped`).
