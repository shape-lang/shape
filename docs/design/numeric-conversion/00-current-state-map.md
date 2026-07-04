# Numeric Type + Cast/Conversion Machinery — Current-State Map

Baseline: `shape-strict-flip-collection-dispatch` @ `0cfb1b11`.
Binary probed: `target/release/shape run --mode vm <file>`.

## THE RULE (user 2026-06-01, binding)

No "lossless enough." A numeric conversion is **implicit** ONLY if it is TRULY
LOSSLESS — every value of the source type is exactly representable in the target
(strict widening, e.g. `u8 -> u16`, `i32 -> i64`). Everything else —
`int <-> number` BOTH directions, any lossy width narrowing (`u16 -> u8`,
`i64 -> i32`), `number -> int` — REQUIRES an EXPLICIT cast (`x as T`). A silent
lossy conversion is a correctness bug. NUMERIC LITERALS adopt their context type
when the literal value is losslessly representable in it (a small int literal in
a `number` context IS a `number` literal — no conversion). Value-level
`int != number` holds: an int VARIABLE/VALUE is never silently a number.

## Confirmed current behavior (this binary)

| Probe | Source | Expected (RULE) | Actual | Verdict |
|-------|--------|-----------------|--------|---------|
| `let n: number = 5` (literal) | int-lit `5` -> number | accept (literal adoption) | ACCEPTS, prints `5` | accepts, but via lossy `can_numeric_widen` not literal-adoption |
| `val:number > 10` | number vs int-lit `10` | accept (literal adoption) | **REJECTS** "number is not compatible with int" | GAP-D |
| `let n:number = int_value` | int VALUE -> number, no cast | **reject** | ACCEPTS, prints `5` | GAP-C (silent) |
| `let small:u8 = u16_value` (300) | u16 -> u8, no cast | **reject** | ACCEPTS, prints **44** (300&0xFF) | GAP-E (silent data loss) |
| `int as number` (cast) | explicit | accept | **REJECTS** "Cannot assert type 'int' as 'number'" | GAP-A (broken cast) |
| `number as int` (cast) | explicit | accept (truncate) | **REJECTS** "Cannot assert type 'number' as 'int'" | GAP-B (broken cast) |
| `int as string`, `string as int?`, `int as decimal` | explicit | accept | **ALL REJECT** | broader: ALL Into/TryInto casts broken (see §b) |

Probe files: `/tmp/numprobe/*.shape` (top-level form — `fn main` is not auto-invoked in `--mode vm` script mode).

---

## (a) Numeric types + widths

### `int` and `number` (the two script primitives)
- `int` = `i64`; `number` = `f64`. Mapping in
  `crates/shape-runtime/src/type_system/types/builtins.rs:73` `canonical_numeric_runtime_name`:
  `int|Int|integer|i64 -> "i64"`; `number|Number|float|f64 -> "f64"`; `f32 -> "f32"`;
  `byte -> "u8"`; `char -> "i8"`; the seven widths map to themselves; `isize`/`usize` pass through.
- Literals: `Literal::Int -> int`, `Literal::Number -> number`, `Literal::TypedInt(_,w) -> w.type_name()`,
  `Literal::UInt -> u64` — `crates/shape-runtime/src/type_system/inference/operators.rs:21-39` (`infer_literal`).
  **No context-sensitivity**: an `Int` literal is always `int` regardless of expected type. (Root of GAP-D.)

### Width types — `IntWidth` (single source of truth)
`crates/shape-ast/src/int_width.rs`. Enum `IntWidth` (line 16): `I8 U8 I16 U16 I32 U32 U64`.
**Does NOT include `i64`** (that stays the default `int`), and does NOT include `isize`/`usize`.

- Per-width spec via `define_int_width_spec!` macro (`int_width.rs:26-251`): `bits()`, `is_signed()`,
  `mask()`, `sign_shift()`, `min_value()/max_value()` (as i64), `max_unsigned()` (u64), `type_name()`.
  Concrete table at `int_width.rs:180-251` (e.g. U16 mask `0xFFFF`, min 0, max 65535).
- `IntWidth::from_name(name)` (`int_width.rs:170-175`): parses exactly the 7 width names (`i8`/`u8`/.../`u64`);
  returns `None` for `i64`/`int`/`number`/anything else. This is the gate that distinguishes a
  "width cast" from an Into-dispatch cast at both the compiler and the type-checker.
- **Bit-truncation semantics (ROOT F)**: `IntWidth::truncate(self, value: i64) -> i64` (`int_width.rs:122-143`):
  signed → mask then sign-extend; U64 → identity; other unsigned → mask only.
  `truncate_u64` (`int_width.rs:150-167`) is the u64-input sibling. This is **the** wrap function —
  `IntWidth::U8.truncate(300) == 44`. Tests at `int_width.rs:322-383` pin every boundary.
- `IntWidth::join(a, b)` (`int_width.rs:261-297`): mixed-width arithmetic join (same width → same;
  same sign → wider; mixed sign → next signed; `u64+signed -> Err`; `u32+signed -> Err` = "promote to i64").
- Range checks `in_range_i64` / `in_range_u64` (`int_width.rs:301-309`) — exactly the
  lossless-representability predicate the RULE needs for literal-adoption and strict-widening checks.

### `NumericWidth` (bytecode/runtime mirror of widths, +float +i64)
`crates/shape-vm/src/bytecode/opcode_defs.rs:2099`. Variants `I8 I16 I32 I64 U8 U16 U32 U64 F32 F64`
(superset of `IntWidth`: adds I64, F32, F64). Helpers: `is_integer/is_float/is_signed/is_unsigned/bits/mask`
(lines 2126-2178). Bridges to `IntWidth`: `from_int_width` (`:2182`) and `to_int_width` (`:2196`, returns
`None` for I64/F32/F64). This is the operand type carried by the `CastWidth` opcode.

---

## (b) `as` / `TypeAssertion` handling + why `int as number`/`number as int` REJECT

### Two layers run on a cast: type-checker (rejects first) then compiler (emits).

**Type-checker arm** — `crates/shape-runtime/src/type_system/inference/expressions.rs:356-401`
(`Expr::TypeAssertion`):
1. `as Type?` (parsed `Generic{Option,[T]}`) → `validate_fallible_conversion` (`:368`).
2. Width cast: if `IntWidth::from_name(name).is_some()` → return target type, BYPASS validation
   (`:384-388`). So `expr as u8` always type-checks (it is statically infallible/truncating).
   `i64`/`int`/`number` are deliberately NOT width names, so they DO NOT take this bypass.
3. Otherwise, if `try_into_selector(&asserted_type).is_some()` (true for `number`, `int`, `string`,
   `decimal`, `bool`, and any named type) → `validate_infallible_conversion(&expr_type, &asserted_type)`
   (`:392-394`).
4. Else: strict assertion constraint `expr_type ~ asserted_type` (`:398`).

**The gate that REJECTS `int as number`** — `validate_infallible_conversion`
(`expressions.rs:1886-1928`):
- early-out Ok if either side has unresolved vars (`:1887`) or `types_equal` (`:1892`);
- else requires `has_into_impl(source_name, target_selector)` (`:1909`) — i.e. a registered
  `Into<Target> for Source` trait impl. `has_into_impl` (`:1951`) →
  `env.lookup_trait_impl_named("Into", source, target)` (key format `Into::int::number`, built by
  `registry.rs:121` `trait_impl_key`);
- Option/Result lifting check (`:1914`);
- otherwise `Err(TypeError::InvalidAssertion(source, target))` (`:1924`) →
  message `"Cannot assert type '{0}' as '{1}'"` (`errors.rs:54`).
- Fallible sibling `validate_fallible_conversion` (`:1842-1884`) is identical but keyed on
  `has_try_into_impl` (`:1944`).

**Why it currently rejects even though the stdlib impl exists.** The stdlib DOES declare
`impl Into<number> for int as number` (`crates/shape-runtime/stdlib-src/core/into.shape:16-18`) and the
full int/number/string/bool/decimal matrix (`into.shape:16-58`, `try_into.shape:15-93`), each registered
via `inference/items.rs:893` (`register_trait_impl_with_assoc_types_named`, impl_name = the `as Target`
selector). **But in this binary the validation does not find them**: EVERY Into/TryInto-dispatched cast
rejects — not just numerics. Probed: `int as number`, `number as int`, `int as string`, `int as decimal`,
`string as int?` all return `InvalidAssertion`. So `has_into_impl` is returning false at user-program
validation time — the prelude `impl Into`/`impl TryInto` blocks are not present in the inference engine's
`env.trait_impls` when the user program is checked (`checker.rs:78` `check_program` →
`inference_engine.infer_program`). Not a compile-cache effect (probed with `SHAPE_NO_CACHE=1` /
`SHAPE_DISABLE_COMPILE_CACHE=1`: identical reject).

**Implication for GREEN.** For the RULE, `int as number` and `number as int` are explicit casts that must
SUCCEED unconditionally. They should NOT depend on stdlib `Into`-impl discovery. The compiler already has a
direct primitive path for them (`convert_opcode_for_primitive`, see §d) — the right lever is the
type-checker: make the TypeAssertion arm accept the primitive numeric cast targets (`int`/`number`/`decimal`/
`bool`/`string`/`char`) directly, parallel to the existing `IntWidth::from_name` width-cast bypass at
`expressions.rs:384-388`, rather than routing them through `validate_infallible_conversion`'s impl lookup.

### Compiler-side emission (what runs after the checker passes)
`crates/shape-vm/src/compiler/expressions/type_ops.rs` `compile_expr_type_assertion` (`:768-995`):
- Fallible `as T?` (`:773-874`): user-impl dispatch (`:803`) else `TryConvertTo*` opcode
  (`try_convert_opcode_for_primitive` `:752`) else `Convert` + TryInto dispatch metadata.
- **Width cast** `as i8..u64` (`:878-888`): `IntWidth::from_name` → emit `OpCode::CastWidth` with
  `Operand::Width(NumericWidth::from_int_width(w))`, stamp `last_expr_numeric_type =
  NumericType::IntWidth(w)`. **Pure bit-truncation, NOT Into-based** — no validation. (This is exactly
  why `u16 -> u8` would truncate cleanly under an explicit cast; the GAP-E problem is that NO cast is
  required today.)
- Infallible `as T` (`:891-977`): user-impl dispatch (`:899`) else `ConvertTo*`
  (`convert_opcode_for_primitive` `:701`: `int->ConvertToInt`, `number->ConvertToNumber`,
  `string/bool/decimal/char`) then `record_cast_result_kind` (`:723`) else `Convert` + Into dispatch
  metadata.
- `has_any_conversion_impls()` early-out in `validate_infallible_cast`/`validate_fallible_cast`
  (`:351`,`:438`) skips compiler-side cast validation in stripped test mode.

---

## (c) Where IMPLICIT numeric conversions are silently ACCEPTED (the constraint-solver sites)

Structural equality alone is strict: `annotations_equal` (`unification/structural_equality.rs:65`) has
`Basic(n1)==Basic(n2)` iff `n1==n2`, so `int`/`number` and `u16`/`u8` are NOT equal there. The silent
acceptance comes from RELAXATIONS layered in the constraint solver
(`crates/shape-runtime/src/type_system/constraints.rs`):

### Relaxation 1 — `can_numeric_widen` (int -> number, directional) → GAP-C
- `constraints.rs:373-391`. `true` iff `is_integer_type_name(from) && is_number_type_name(to)`.
  Used at `solve_constraint` (`:230`, "Implicit numeric promotion (int → number/float)") and inside
  `unify_annotations` (`:464`, `:481`).
- DIRECTIONAL: `int -> number` widens, `number -> int` does not. For `let n: number = int_value` the
  constraint is `(int, number)` → `can_numeric_widen` accepts → **silent int->number**, the GAP-C bug.
- (`number -> int` is correctly rejected by this function — which is exactly why GAP-D's `(number, int)`
  comparison constraint fails.)

### Relaxation 2 — `same_canonical_numeric_type` (all int widths interchangeable) → GAP-E
- `constraints.rs:409-417` → `BuiltinTypes::canonical_script_alias` (`builtins.rs:116-133`): collapses
  ALL integer widths (`i8 u8 i16 u16 i32 u32 i64 u64 isize usize byte char`) → `"int"`, both float
  widths → `"number"`.
- Wired in via `annotations_same_numeric` (`constraints.rs:445`) → used in `unify_annotations`
  Basic/Basic (`:463`), Reference/Reference (`:466`), Basic/Reference (`:480`).
- Effect: `u16 ~ u8` both canonicalize to `int` → constraint `(u16, u8)` succeeds with NO cast →
  `let small: u8 = u16_300` compiles; at runtime the bits flow unchanged then are READ as u8.
  (The 300->44 wrap actually happens at the slot-kind/print layer, see §d note; the type-checker's
  job — rejecting the assignment — is what's missing.)

### Relaxation 3 — `numeric_result_type` (arithmetic result collapse)
`inference/operators.rs:139-187`. Mixed widths same script-family → collapse to `int`/`number`
(`:152-163`); mixed cross-family (int+number) → widen to `number` (`:164-170`). This is why
`n:number + 1` works (`number + int -> number`) while `n > 10` does not (see §GAP-D). Arithmetic
constrains each operand only to the `Numeric` trait bound INDEPENDENTLY
(`infer_numeric_arithmetic_op` `:319-342`) — it never pushes a `left ~ right` same-type constraint.

### The GAP-D mechanism (literal rejected, asymmetry with arithmetic)
Comparison `< > <= >=` (`inference/operators.rs:436-467`) DOES push a same-type constraint
`effective_left ~ effective_right` (`:449`) plus a `Comparable` bound (`:452`). For `val:number > 10`
the literal `10` is `int` (no adoption), so the constraint is `(number, int)`. In `solve_constraint`:
`unify_annotations(number, int)` → not `==`, `annotations_same_numeric` false (number-alias != int-alias),
`can_numeric_widen(number,int)` false (number is not an integer name) → returns `Ok(false)` → then top-level
`can_numeric_widen(number,int)` (`:230`) also false → `Err(TypeMismatch)` → surfaces as
`"number is not compatible with int"` (`errors.rs:168` constraint-render). RULE fix lever: literal
context-adoption so `10` becomes a `number` literal in number context (`infer_literal` /
check-mode plumbing), making the constraint `(number, number)`.

### Equality `== !=`
`inference/operators.rs:417-434`: pushes same-type constraint (`:432`) except when exactly one side is a
null sentinel. Subject to the same `can_numeric_widen`/`same_canonical_numeric_type` relaxations during
solving.

### `try_unify` (soft, bidirectional)
`unification/unifier.rs:203-277`: Concrete/Concrete uses `annotations_equal` + `AnyError` only — it does
NOT apply the numeric relaxations. It is the soft path; the hard `solve_constraint` path above is where the
numeric gaps live.

---

## (d) Bytecode / opcode support for the actual runtime conversions

The runtime is NOT the gap — every conversion opcode exists and works. The gaps are all in the
type-checker (accept-when-should-reject for C/E, reject-when-should-accept for A/B/D).

### `CastWidth` (width truncation)
- Opcode operand `Operand::Width(NumericWidth)` — `opcode_defs.rs:2293`.
- Handler `op_cast_width` — `crates/shape-vm/src/executor/arithmetic/mod.rs:644-659`: pop kinded,
  `bits as i64`, `width.to_int_width().map(|w| w.truncate(raw))` (the `IntWidth::truncate` wrap), push as
  `NativeKind::Int64` (U64 → `result_kind_for_width` stamps `UInt64`, `:689`). Dispatch at
  `executor/dispatch.rs:586`.
- JIT lowering: `crates/shape-jit/src/mir_compiler/rvalues.rs` does the same width truncation natively
  (`ireduce`/extend, doc at `:684`,`:1089-1108`,`:1232`) — "operate at the width" matches `IntWidth::truncate`.

### `ConvertToInt` / `ConvertToNumber` (int<->float + general primitive casts)
- `op_convert_to_int` — `crates/shape-vm/src/executor/builtins/type_ops.rs:591-598`: pop, `read_as_i64`,
  push `NativeKind::Int64`. (number->int truncation lives in `read_as_i64`.)
- `op_convert_to_number` — `:603-608`: pop, `read_as_f64`, push `NativeKind::Float64` (int->float).
- Siblings: `op_convert_to_string/bool/decimal/char` (`:618-659`). Dispatch `executor/dispatch.rs:761-762`.
- Fallible `TryConvertTo*` (`:710-...`) wrap the infallible body via `try_convert_or_none` (`:694-708`):
  conversion `RuntimeError` → `None` sentinel; other errors propagate. Dispatch `:767-768`.
- Compiler picks these via `convert_opcode_for_primitive` (`type_ops.rs:701`) /
  `try_convert_opcode_for_primitive` (`:752`).

### `Convert` (trait-dispatched, for user Into/TryInto + bare wrapper lift)
- `OpCode::Convert` with `Operand::Const(TypeAnnotation)` carrying `__IntoDispatch`/`__TryIntoDispatch`
  metadata (`type_ops.rs:11-12`,`:156`,`:861`,`:951`).

### Compiler kind-tracking note (GAP-E runtime mechanism)
`op_cast_width` re-stamps the result `NativeKind`; `record_cast_result_kind` (`type_ops.rs:723`) updates
`last_expr_numeric_type`. With NO cast required (GAP-E), `let small:u8 = u16_300` flows the raw bits, and
the read/format at u8 width produces 44 — the data loss is observable precisely because the type-checker
accepted an assignment that should have demanded an explicit `as u8`.

---

## Summary of edit surfaces for GREEN

| Gap | RULE verdict | Edit surface (file:line) |
|-----|--------------|--------------------------|
| A: `int as number` rejects | should accept | `inference/expressions.rs:384-394` — add primitive numeric-cast bypass parallel to the `IntWidth::from_name` width bypass, OR fix prelude-impl registration into the validation env |
| B: `number as int` rejects | should accept (truncate) | same site |
| C: `let n:number = int_value` accepts | should reject | `constraints.rs:373-391` `can_numeric_widen` (+ call sites `:230`,`:464`,`:481`) — remove/narrow int->number implicit widening |
| D: `number > 10` literal rejects | should accept | `inference/operators.rs:21-39` `infer_literal` + check-mode plumbing — literal context-adoption (lossless-representable int-lit in number ctx becomes number) |
| E: `u16 -> u8` accepts (300->44) | should reject | `constraints.rs:409-417` `same_canonical_numeric_type` / `builtins.rs:116-133` `canonical_script_alias` — stop collapsing distinct widths to one alias for assignment unification; require strict-widening-only implicit, else explicit cast. `IntWidth::in_range_*` / `bits()` are the lossless-widening predicate |

Lossless-widening predicate available for "truly lossless implicit" decisions:
`IntWidth::bits()` + `is_signed()` (strict widening = target bits > source bits and sign-compatible) and
`IntWidth::in_range_i64/u64` (literal representability) — `crates/shape-ast/src/int_width.rs`.
