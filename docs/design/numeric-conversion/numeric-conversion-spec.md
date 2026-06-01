# Numeric Conversion Conformance Spec

**Status:** Design spec (release-blocking, v0.3.3)
**Binding rule source:** User, 2026-06-01
**Baseline:** `shape-strict-flip-collection-dispatch` @ `0cfb1b11`
**Probe binary:** `target/release/shape run --mode vm <file>`
**Scope:** Defines the conformance model for all numeric type conversions —
which conversions are implicit, which require an explicit `as` cast, the cast
semantics, and the literal-adoption rule. This spec is normative; the
"Conformance gap" callouts mark where the shipped binary diverges and must be
fixed.

---

## 0. The Rule (binding, verbatim intent)

> No "lossless enough." A numeric conversion is **implicit ONLY if it is TRULY
> LOSSLESS** — every value of the source type is exactly representable in the
> target (strict widening, e.g. `u8 -> u16`, `i32 -> i64`). Everything else —
> `int <-> number` **both directions**, any lossy width narrowing
> (`u16 -> u8`, `i64 -> i32`), `number -> int` — **REQUIRES an EXPLICIT cast**
> (`x as T`). A silent lossy conversion is **FORBIDDEN** (a correctness bug).
>
> **Numeric literals** adopt their context type when the literal value is
> losslessly representable in it (a small int literal in a `number` context IS
> a number literal — no conversion).
>
> **Value-level `int != number` holds**: an int *variable / value* is never
> silently a number.

This is the strict-typing posture applied to the numeric lattice. It is the
same discipline as CLAUDE.md §Type System Rules ("NO runtime coercion", "`int`
and `number` are separate") — extended to the full width lattice and made
exhaustive.

---

## 1. The Numeric Types

The type universe for this spec, with the runtime carrier and the
exactly-representable value range:

| Script type | Width / carrier | Signed | Exact value range | `NativeKind` | `IntWidth` |
|-------------|-----------------|--------|-------------------|--------------|------------|
| `i8` / `char`† | 8-bit int | yes | −128 .. 127 | `Int8` | `I8` |
| `u8` / `byte` | 8-bit int | no | 0 .. 255 | `UInt8` | `U8` |
| `i16` | 16-bit int | yes | −32 768 .. 32 767 | `Int16` | `I16` |
| `u16` | 16-bit int | no | 0 .. 65 535 | `UInt16` | `U16` |
| `i32` | 32-bit int | yes | −2 147 483 648 .. 2 147 483 647 | `Int32` | `I32` |
| `u32` | 32-bit int | no | 0 .. 4 294 967 295 | `UInt32` | `U32` |
| `int` / `i64` | 64-bit int (default integer) | yes | −2⁶³ .. 2⁶³−1 | `Int64` | — (not an `IntWidth`) |
| `u64` | 64-bit int | no | 0 .. 2⁶⁴−1 | `UInt64` | `U64` |
| `number` / `f64` | IEEE-754 double | — | exact integers in −2⁵³ .. 2⁵³; all f64 reals | `Float64` | — |

† `char` canonicalizes to `i8` in the current runtime
(`builtins.rs:80,168`). This is a pre-existing quirk; `char`'s conversion
membership is an **open decision** (§7, OD-5) — the lattice below treats the
nine numeric types and does not enumerate `char`.

Notes:
- `int` is the default integer and is **not** an `IntWidth` variant
  (`int_width.rs:7-24` — the enum is `{I8,U8,I16,U16,I32,U32,U64}`, no `I64`).
  `int` widening to/from the `IntWidth` set is therefore a distinct code path
  from `IntWidth`-to-`IntWidth`.
- `isize`/`usize` exist as `NativeKind` variants and as canonical aliases
  (`builtins.rs:85-86`) but are platform-width and out of scope for the
  fixed-range lattice below; see §7 OD-4.
- `decimal` and `bigint` are arbitrary/exact-precision heap types, not part of
  the fixed-width lossless lattice; conversions to/from them are always
  `as`-cast (trait-dispatched `Into`/`TryInto`) and unchanged by this spec.

---

## 2. The Lossless Lattice (general rule)

**General rule.** An ordered pair `(src, dst)` is **LOSSLESS-IMPLICIT** iff the
entire value range of `src` is a subset of the values **exactly representable**
in `dst`. Otherwise the pair is **CAST-REQUIRED**.

"Exactly representable" is the strict criterion — for `number`/`f64` the
representable-integer set is `[-2⁵³, 2⁵³]`, so an integer type is
lossless→`number` iff its **whole** range fits in `[-2⁵³, 2⁵³]`.

Formally, with `range(T) = [lo_T, hi_T]` over the value set (and the f64
exact-integer set treated as `[-2⁵³, 2⁵³]`):

```
lossless_implicit(src, dst)  ⇔  lo_dst ≤ lo_src  ∧  hi_src ≤ hi_dst
                                  (over the exactly-representable set of dst)
```

Identity (`src == dst`) is trivially lossless-implicit.

### 2.1 Full ordered-pair table

Rows = source, columns = destination. `≡` = identity (implicit, no-op).
`IMPL` = LOSSLESS-IMPLICIT. `CAST` = CAST-REQUIRED. Cells are read
**src → dst**.

| src \ dst | i8 | u8 | i16 | u16 | i32 | u32 | int(i64) | u64 | number(f64) |
|-----------|----|----|-----|-----|-----|-----|----------|-----|-------------|
| **i8**    | ≡  |CAST|IMPL |CAST |IMPL |CAST |IMPL      |CAST |IMPL |
| **u8**    |CAST| ≡  |IMPL |IMPL |IMPL |IMPL |IMPL      |IMPL |IMPL |
| **i16**   |CAST|CAST| ≡   |CAST |IMPL |CAST |IMPL      |CAST |IMPL |
| **u16**   |CAST|CAST|CAST | ≡  |IMPL |IMPL |IMPL      |IMPL |IMPL |
| **i32**   |CAST|CAST|CAST |CAST | ≡  |CAST |IMPL      |CAST |IMPL |
| **u32**   |CAST|CAST|CAST |CAST |CAST | ≡  |IMPL      |IMPL |IMPL |
| **int**   |CAST|CAST|CAST |CAST |CAST |CAST | ≡        |CAST |CAST |
| **u64**   |CAST|CAST|CAST |CAST |CAST |CAST |CAST      | ≡  |CAST |
| **number**|CAST|CAST|CAST |CAST |CAST |CAST |CAST      |CAST | ≡   |

### 2.2 Derivation of the subtle cells (why each is what it is)

**Signed widening (same signedness, wider).** `i8→i16→i32→int(i64)` are all
IMPL: each signed range is a strict subset of the next wider signed range.
Likewise `u8→u16→u32→u64` are all IMPL.

**Unsigned → wider signed.** An unsigned range `[0, 2ⁿ−1]` fits a signed range
`[−2ᵐ⁻¹, 2ᵐ⁻¹−1]` iff `2ⁿ−1 ≤ 2ᵐ⁻¹−1`, i.e. `m ≥ n+1`.
  - `u8 (0..255) → i16 (..32767)`: **IMPL** (255 ≤ 32767).
  - `u8 → i32`, `u8 → int`: **IMPL**.
  - `u16 (0..65535) → i32 (..2147483647)`: **IMPL**.
  - `u16 → int`: **IMPL**.
  - `u32 (0..4294967295) → int(i64) (..9.2e18)`: **IMPL** (the requested
    `u32 → i64 ok` case).
  - `u8 → i8`, `u16 → i16`, `u32 → i32`: **CAST** — the unsigned high half
    (e.g. u8's 128..255) does not fit the same-width signed range.
  - `u32 → i16`, `u16 → i8`, etc. (narrower signed): **CAST**.

**Unsigned → wider unsigned.** `u8→u16`, `u8→u32`, `u8→u64`, `u16→u32`,
`u16→u64`, `u32→u64` (the requested `u32 → u64 ok`): all **IMPL**.

**Signed → unsigned (any).** Always **CAST** — every signed type includes
negatives, which no unsigned type represents. `i8→u8`, `i8→u16`, `i16→u16`,
`i32→u32`, `int→u64`, etc. are all CAST. (This is the
`i16(-5) → u16 = 65531` silent-loss bug today; see §6.)

**Narrowing (wider → narrower, same or any signedness).** Always **CAST**:
`u16→u8`, `i64(int)→i32`, `i32→i16`, `u32→u16`, etc. The source range
overflows the destination. (This is the `u16(300) → u8 = 44` silent-loss bug
today.)

**To `number` (f64).** f64 exactly represents every integer in `[−2⁵³, 2⁵³]`.
  - `i8, u8, i16, u16, i32, u32`: whole ranges fit in `[−2⁵³, 2⁵³]` → **IMPL**.
    (`u32` max 4 294 967 295 ≈ 2³² ≪ 2⁵³.)
  - `int (i64)`: range reaches 2⁶³ ≫ 2⁵³ → **CAST**. Some i64 values (e.g.
    `2⁵³+1`) are not exactly representable as f64.
  - `u64`: range reaches 2⁶⁴ ≫ 2⁵³ → **CAST**.
  - This is the load-bearing distinction the rule calls out: **`int → number`
    is CAST-REQUIRED** even though "most" ints fit, because *not every* int
    value fits. `i32 → number` is IMPL because *every* i32 fits.

**From `number` (f64) to any integer.** Always **CAST**: f64 holds
fractional and out-of-range values that no integer type represents. This is
`number → int` and every `number → iN/uN`.

### 2.3 Same-signedness widening chains (quick reference)

- Signed widen-implicit: `i8 ⊂ i16 ⊂ i32 ⊂ int(i64)`.
- Unsigned widen-implicit: `u8 ⊂ u16 ⊂ u32 ⊂ u64`.
- Cross-sign widen-implicit (unsigned into next-or-wider signed):
  `u8 ⊂ {i16,i32,int}`, `u16 ⊂ {i32,int}`, `u32 ⊂ {int}`.
- Integer→float widen-implicit: `{i8,u8,i16,u16,i32,u32} ⊂ number`.

Everything not in these chains (and not identity) is CAST-REQUIRED.

---

## 3. Cast Semantics (the CAST-REQUIRED pairs)

When a pair is CAST-REQUIRED, the program must write `x as T`. The semantics
below adopt the **existing** established runtime behavior where it exists; new
conventions and genuinely-ambiguous choices are flagged.

### 3.1 Integer width narrowing & sign reinterpretation — `as iN` / `as uN`

**Semantics: two's-complement bit-truncation / wrap (Rust `as`).** This is the
**established** behavior of `OpCode::CastWidth`
(`executor/arithmetic/mod.rs:641-659`), which truncates via
`IntWidth::truncate` (`int_width.rs:122-143`): mask to width, then
sign-extend for signed targets.

Confirmed against the binary and against the in-crate unit tests
(`int_width.rs` `truncate_*` tests, `arithmetic/mod.rs:1151-1165`):

| Cast | Result | Source |
|------|--------|--------|
| `300 as u8` | `44` (300 mod 256) | probe `300 as u8` → `44` |
| `300 as i8` | `44` | `cast_width_i8_truncation` (`arithmetic/mod.rs:1153`) |
| `-1 as u8` | `255` | `cast_width_i8_negative` (`arithmetic/mod.rs:1159`) |
| `u64::MAX as i8` | `-1` | `cast_width_u64_max_to_i8` (`arithmetic/mod.rs:1165`) |
| `-5 as u16` | `65531` | probe `i16(-5) → u16` (today implicit; must require cast) |

This is the canonical "**`300 as u8 = 44 = 300 mod 256`**" the rule asks to
confirm. **Confirmed.**

Compiler emission for width targets is already correct: `expr as i8/u8/i16/
u16/i32/u32/u64` emits `CastWidth` (`type_ops.rs:876-887`); the inference side
bypasses Into-validation for the 7 width names (`expressions.rs:384-388`).
Width-target casts therefore already work; the gap (§6) is that the *implicit*
path also silently does this truncation.

> **Note on `as int` (i64) and `as u64`.** `int`/`i64` is **not** an
> `IntWidth` name, so `x as int` does **not** route through `CastWidth`; it
> routes through the primitive `Into` path (`ConvertToInt`,
> `type_ops.rs:703`), which currently rejects for primitive integer sources
> (§6). The fix (§5) must make `as int` from a narrower-or-equal integer a
> reinterpret-to-i64 (sign/zero-extend), and from `number`/`u64` a defined
> narrowing per §3.2 / §3.3.

### 3.2 `number → int` (and `number → iN/uN`) — truncate toward zero

**Stated convention (user 2026-06-01):** `number as int` = **truncate toward
zero** (drop the fractional part). E.g. `3.7 as int = 3`, `-3.7 as int = -3`.

**Conformance / established-semantics conflict — OPEN DECISION (OD-1).** The
existing runtime helper `read_as_i64` (`executor/builtins/type_ops.rs:63-77`),
which backs `ConvertToInt`/`op_convert_to_number`, does **NOT** truncate — it
**rejects** a non-integer float at runtime:

```rust
let i = n as i64;
if (i as f64 - n).abs() > f64::EPSILON {
    return Err(VMError::RuntimeError(
        format!("cannot convert non-integer number '{n}' to int")));
}
```

So the established behavior for `number → int` is *fail-on-fractional*, while
the user's stated cast convention is *truncate-toward-zero*. These are
incompatible. The user's binding rule says "Adopt the EXISTING established
semantics where they exist; state conventions where new; FLAG any
genuinely-ambiguous choice as an open_decision." This is genuinely ambiguous —
see §7 OD-1. **This spec adopts the user's explicit convention
(truncate-toward-zero) as the intended target** and flags the existing
reject-behavior as the thing to change, because the user stated the convention
directly and as part of the binding rule. Implementation must replace the
`EPSILON` reject in `read_as_i64` with `Ok(n.trunc() as i64)` for the
`number→int` cast path (non-finite still errors; out-of-i64-range behavior is
OD-2).

Out-of-range / non-finite `number → int`:
- Non-finite (`NaN`, `±inf`): the existing code errors
  (`type_ops.rs:65-69`). Keep as a runtime error — **OPEN DECISION OD-2**
  (error vs saturate vs wrap; Rust `as` saturates, but Shape has no exceptions
  and `as` is statically infallible elsewhere — see OD-2).

`number → iN/uN` (e.g. `number as u8`) = truncate-toward-zero **then**
width-wrap per §3.1 (compose `ConvertToInt`-trunc with `CastWidth`), matching
Rust's `f64 as u8` chain.

### 3.3 `int → number` (and `u64 → number`) — value as f64

**Semantics (established + stated):** the i64 (resp. u64) value reinterpreted
as the nearest f64. This is exactly `read_as_f64`
(`type_ops.rs:124-143`): `slot.as_i64() as f64` / `slot.as_u64() as f64`.
For magnitudes above 2⁵³ this rounds to the nearest representable double —
**which is precisely why the conversion is CAST-REQUIRED rather than implicit.**
The explicit `as number` is the programmer acknowledging the possible rounding.

No range error: every i64/u64 maps to *some* finite f64 (rounding, never
overflow). This is unambiguous; adopt as-is.

### 3.4 Integer widening done explicitly

A LOSSLESS-IMPLICIT pair may also be written explicitly (`x as int` where
`x: i32`); the cast is then a no-op reinterpret (sign/zero-extend to i64).
Explicit casts are always permitted for any pair in the lattice; the
classification only governs whether the cast is *required*.

### 3.5 Summary of cast lowering

| Cast class | Opcode today | Semantics | Status |
|------------|--------------|-----------|--------|
| `as iN/uN` (N a width) | `CastWidth` (`type_ops.rs:882`) | two's-complement wrap | works; correct |
| `as int` from int-family | `ConvertToInt` (`type_ops.rs:703`) | reinterpret/extend to i64 | rejected by inference gate — fix §5 |
| `as number` from int-family | `ConvertToNumber` (`type_ops.rs:704`) | `value as f64` | rejected by inference gate — fix §5 |
| `number as int` | `ConvertToInt` → `read_as_i64` | **trunc-toward-zero (target)** vs reject (today) | OD-1, §6 |
| `as u64` from narrower | `ConvertToInt`/`CastWidth(U64)` | zero-extend | fix §5 |

---

## 4. Literal Adoption Rule

**Rule.** An **untyped integer literal** (`Literal::Int(i64)` /
`Literal::UInt(u64)`, `literals.rs:36-38`) adopts the numeric type required by
its context **iff its value is losslessly representable in that type**. A
literal whose value does not fit the target is a **compile error** (not a
silent wrap).

- `let n: number = 5` ⇒ `5` is the f64 literal `5.0` (no `int→number`
  conversion occurs; the literal *is* a number). Probe confirms the runtime
  treats it as f64: `let n: number = 5; n / 2` prints `2.5` (probe `lit1b`),
  `n + 0.5` prints `5.5` (probe `lit1c`).
- `let x: u8 = 200` ⇒ `200` is a `u8` literal (200 ∈ 0..255). ✔
- `let x: u8 = 300` ⇒ **compile error** ("literal `300` out of range for
  `u8`"). Today this **silently wraps to 44** (probe `lit2`) — the conformance
  gap (§6).
- `val: number > 10` ⇒ the literal `10` adopts `number` (10 ∈ exact-f64) and
  the comparison is `f64 > f64`. Today this **REJECTS** ("number is not
  compatible with int", probe `probe1`) — the conformance gap (§6).
- `a: number * 3` ⇒ `3` adopts `number`; result `number`. (Already works,
  probe `arith` → `6.0` — but via the to-be-removed widen path, §6.)

**Mechanism (target).** A bare integer literal must be **context-polymorphic**,
not pre-typed as `int`. Today `infer_literal` (`operators.rs:23`) types
`Literal::Int(_)` as concrete `int`, and `let n: number = 5` only typechecks
because `can_numeric_widen(int, number)` permits the int→number widen
(`constraints.rs:373-391`). Once that widen is removed (§5), the literal must
adopt the context type directly (bidirectional: the annotation/expected type
flows into the literal), with a range check against the target. A literal with
**no** numeric context defaults to `int` (its natural type), as today.

**Explicitly-typed literals** (`Literal::TypedInt(v, w)`, e.g. `42u8`,
`literals.rs:39-40`) do **not** adopt context — they are already that width;
assigning `42u8` into a `u16` follows the value-level lattice (`u8 → u16` is
IMPL, so ok; `300u16` into `u8` is CAST-required).

**`Literal::UInt(u64)`** (value > i64::MAX, `literals.rs:37-38`) is naturally
`u64`; it adopts only types whose range contains it (i.e. `u64`, or `number`
with possible rounding — and `→ number` is CAST-required per §2 for `u64`, so
a bare `Literal::UInt` in a `number` context past 2⁵³ is an **open decision**,
OD-3: error vs implicit-with-rounding-since-it's-a-literal).

---

## 5. The value-level `int != number` invariant

**Invariant.** An `int` *variable, parameter, field, or expression result* is
**never** silently a `number`, and vice versa. Cross-family flow requires an
explicit `as` cast at every site:

- **Binding:** `let n: number = int_value` ⇒ **error** (must be
  `int_value as number`). Today **ACCEPTS** (probe `probe2`) via
  `can_numeric_widen` — gap §6.
- **Comparison / binary op:** `int_var > number_var`, `int_var + number_var`
  ⇒ **error**. Today `int_var + number_var` **ACCEPTS** → `7.0` (probe
  `mix2`) — gap §6. Both operands must already be the same family; mixing
  requires an explicit cast on one side.
- **Argument passing:** an `int` argument to a `number` parameter ⇒ error.
- **Return:** returning an `int` expression from a `-> number` function ⇒
  error.

The invariant is **only** relaxed for **untyped literals** (§4): a literal is
not yet a "value of type int" — it is an untyped numeral that adopts the
context type. This is the precise line between "literal adoption is fine" and
"value-level coercion is forbidden."

### The single mechanical change that enforces both directions

The cross-family int↔number leak has exactly one root: **`can_numeric_widen`**
(`constraints.rs:373-391`) returns `true` for *any* integer-family → any
number-family pair, and it is consulted in two places:
1. `solve_constraint` (`constraints.rs:230-232`) — binding/return/arg/op
   unification.
2. `unify_annotations` basic/reference arms (`constraints.rs:463-464`,
   `480-481`).

Removing the `can_numeric_widen` disjunct from those sites enforces value-level
`int != number` and all CAST-REQUIRED cross-family pairs. The literal-adoption
rule (§4) must land **simultaneously**, otherwise `let n: number = 5` and
`val:number > 10` break (they currently lean on this same widen path).

### The width-collapse leak (separate root)

The intra-integer silent-loss bugs (`u16→u8 = 44`, `i16(-5)→u16 = 65531`,
`u8→i16` etc. all implicit) have a **separate** root:
`annotations_same_numeric` (`constraints.rs:445-450`) →
`same_canonical_numeric_type` (`constraints.rs:409-417`) →
`canonical_script_alias` (`builtins.rs:116-133`), which collapses **every**
integer width to the single alias `"int"`. So `i8 ~ int ~ u16 ~ u8` all unify
with no cast and no range check. The fix must replace this single-alias
collapse with the per-pair lattice of §2: two integer-width annotations unify
implicitly **iff** the `(src,dst)` cell is IMPL (range-subset), else
CAST-required. `IntWidth::in_range_i64`/`in_range_u64`
(`int_width.rs:299-309`) and `min_value`/`max_value`/`max_unsigned` already
provide the range primitives to compute the lattice.

---

## 6. Confirmed conformance gaps (this binary, `--mode vm`)

All probed against `target/release/shape run --mode vm` @ `0cfb1b11`.

| # | Program | Today | Spec-required | Root |
|---|---------|-------|---------------|------|
| G1 | `let val:number=5.0; val > 10` | **REJECT** ("number not compatible with int") | accept (`10` adopts `number`) | literal not context-polymorphic (`operators.rs:23`); comparison unify lacks literal adoption |
| G2 | `let i:int=42; let n:number=i` | **ACCEPT** (prints 42) | **REJECT** (need `i as number`) | `can_numeric_widen` (`constraints.rs:230,463`) |
| G3 | `let i:int=42; i as number` | **REJECT** ("Cannot assert type 'int' as 'number'") | accept (`ConvertToNumber`, may round) | `validate_infallible_conversion` requires user `Into` impl (`expressions.rs:1909`); primitives have none |
| G4 | `let n:number=3.7; n as int` | **REJECT** ("Cannot assert type 'number' as 'int'") | accept, trunc → `3` (OD-1) | same gate (`expressions.rs:1909`) + `read_as_i64` rejects fractional (`type_ops.rs:71-75`) |
| G5 | `let big:u16=300; let small:u8=big` | **ACCEPT silently → 44** | **REJECT** (need `big as u8`) | width-collapse: `canonical_script_alias` → `"int"` (`builtins.rs:123`) |
| G6 | `let x:i16=-5; let y:u16=x` | **ACCEPT silently → 65531** | **REJECT** (need `x as u16`) | width-collapse (same as G5) |
| G7 | `let x:u8=300` | **ACCEPT silently → 44** (StoreLocalTyped wrap, `statements.rs:5128-5137`) | **compile error** (literal out of range) | literal range-check dormant (`statements.rs:4691-4716`, ADR-006 §2.4 `ConstFoldValue` uninhabited) |
| G8 | `let i:int=5; let n:number=2.0; i + n` | **ACCEPT → 7.0** | **REJECT** (need `i as number` or `n as int`) | `can_numeric_widen` in op unify (`constraints.rs:230`) |

Note G3/G4: the compiler *back end* is already wired — `ConvertToInt`
(`type_ops.rs:703`) / `ConvertToNumber` (`type_ops.rs:704`) exist with kinded
bodies (`type_ops.rs:591-608`). The block is purely the **inference-front-end
gate** at `expressions.rs:390-395`, which routes `int`/`number` targets
through `validate_infallible_conversion` (Into-impl required) instead of
recognizing them as built-in primitive numeric casts. The fix: in
`Expr::TypeAssertion` inference (`expressions.rs:356-401`), treat a
primitive-numeric target (`int`/`number` and the width names) as a built-in
cast whose legality is governed by §2/§3 (always permitted for any numeric
src→dst, with §3 semantics), bypassing the user-`Into` requirement.

---

## 7. Open decisions for the user

- **OD-1 — `number → int` cast semantics: truncate vs reject.** The user's
  stated convention is **truncate-toward-zero** (`3.7 as int = 3`). The
  **existing** runtime (`read_as_i64`, `type_ops.rs:63-77`) **rejects**
  non-integer floats at runtime. These conflict. This spec adopts the user's
  stated convention (truncate) as the target and flags the existing reject as
  the behavior to change. **Confirm:** `as int` truncates (Rust-like), or does
  `as int` reject fractional values and a separate `round`/`floor`/`trunc`
  stdlib API is the only way to drop the fraction? (Truncate is the §0-stated
  rule; confirming so the `EPSILON` reject can be removed.)

- **OD-2 — out-of-range / non-finite `number → int`.** With truncation
  adopted (OD-1), what is `1e30 as int` (exceeds i64) and `NaN as int` /
  `inf as int`? Options: (a) runtime error (current code errors on non-finite);
  (b) Rust-style saturate (`i64::MAX`/`MIN`, `0` for NaN); (c) wrap. Shape has
  no exceptions and `as` is statically infallible for the width casts, which
  argues against a runtime error. **Convention adopted pending ruling:**
  non-finite → runtime error (matches existing `type_ops.rs:65-69`);
  out-of-i64-range → runtime error. Confirm or pick saturate.

- **OD-3 — large `Literal::UInt` in a `number` context.** `u64 → number` is
  CAST-required (§2, value-level). But a *literal* `18446744073709551615` in a
  `number` context: error (require `as number`), or implicit-with-rounding
  because it is a literal not a value? The literal-adoption rule (§4) says
  "losslessly representable"; past 2⁵³ a u64 literal is **not** losslessly
  representable in f64. **Convention adopted:** treat as the strict rule — a
  literal not losslessly representable in the target `number` is a **compile
  error**, requiring `as number`. Confirm.

- **OD-4 — `isize`/`usize`.** Platform-width. Excluded from the fixed-range
  lattice (§1). **Convention adopted:** `isize` behaves as `int(i64)` and
  `usize` as `u64` for lattice purposes on 64-bit targets (their canonical
  aliases keep them distinct, `builtins.rs:85-86`). Confirm, or rule them
  out-of-scope until a portable-width policy exists.

- **OD-5 — `char` ↔ integer.** `char` canonicalizes to `i8`
  (`builtins.rs:80,168`), which is a pre-existing oddity (a Unicode scalar is
  not an 8-bit int). This spec **excludes** `char` from the numeric lattice.
  **Convention adopted:** `char ↔ int` conversions stay explicit `as`-casts
  via the dedicated `ConvertToChar`/`as int` paths and are out of scope here.
  Confirm `char` should not participate in numeric widening at all.

---

## 8. Implementation touchpoints (for the fix workstream)

| Concern | File:line | Change |
|---------|-----------|--------|
| int↔number implicit widen (remove) | `constraints.rs:230-232`, `463-464`, `480-481` | drop `can_numeric_widen` disjunct |
| `can_numeric_widen` (delete or repurpose) | `constraints.rs:373-391` | remove; replaced by §2 lattice |
| width-collapse to `"int"` | `builtins.rs:116-133` (`canonical_script_alias`) via `same_canonical_numeric_type` (`constraints.rs:409-417`) / `annotations_same_numeric` (`constraints.rs:445-450`) | replace single-alias unify with per-pair range-subset lattice |
| lattice range primitives | `int_width.rs:84-105`, `299-309` | reuse `min_value`/`max_value`/`max_unsigned`/`in_range_*` |
| primitive numeric cast inference gate | `expressions.rs:384-395` (`Expr::TypeAssertion`) | recognize `int`/`number`/width targets as built-in casts; bypass `Into`-impl requirement |
| `number→int` trunc vs reject | `executor/builtins/type_ops.rs:63-77` (`read_as_i64`) | replace `EPSILON` reject with `n.trunc() as i64` (OD-1) |
| literal context adoption | `operators.rs:21-39` (`infer_literal`) + bidirectional flow | make `Literal::Int`/`UInt` context-polymorphic with range check |
| literal range check (dormant) | `statements.rs:4691-4716` | activate compile-time literal range check (ADR-006 §2.4 `ConstFoldValue` rebuild) |
| width store-truncation (literals) | `statements.rs:5121-5141`, `4879-4892` (`StoreLocalTyped`/`StoreModuleBindingTyped`) | gate so it only fires after a literal range check or an explicit cast — not as a silent implicit-narrow |

**Forbidden-pattern note.** None of these fixes introduce a runtime coercion
opcode or a dynamic fallback — they *remove* implicit conversions and route the
remaining (explicit) ones through the **already-existing** typed `ConvertTo*` /
`CastWidth` opcodes. There is no new `Convert<X>To<Y>` opcode and no widening
of the slot ABI; this is consistent with CLAUDE.md §Type System Rules ("NO
runtime coercion", "NO dynamic fallback") and §Forbidden Patterns. The
`int→number` rounding is *acknowledged* by the explicit cast, not silently
inserted.
