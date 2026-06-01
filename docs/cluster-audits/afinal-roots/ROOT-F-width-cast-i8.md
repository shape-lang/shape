# A-final ROOT F — width-cast assertion refuses i8

**Verdict: FP_fix_checker** (valid Shape over-rejected by the strict-flip type checker).

## Failing test

- `error_handling::diagnostics::width_cast_i8_always_valid`
  (`tools/shape-test/tests/error_handling/diagnostics.rs:264-267`)

```rust
#[test]
fn width_cast_i8_always_valid() {
    ShapeTest::new("let x = 256 as i8\nx").expect_run_ok();
}
```

The surrounding comment block documents the intent:
`// -- Width casts are unaffected by Into validation --`.

## Reproduction on the strict-flip binary (@f01e8323, let-gen landed)

Program `/tmp/root_f.shape`:

```
let x = 256 as i8
x
```

Run:

```
target/release/shape run /tmp/root_f.shape
Error: Runtime error: Bytecode compilation failed: Semantic error: Cannot assert type 'int' as 'i8'
```

Verbatim rejection confirmed. The whole width-cast family is over-rejected, not just `i8`:

| program            | strict-flip result                              |
|--------------------|-------------------------------------------------|
| `5 as int`         | OK (int == int short-circuits)                  |
| `let x: i8 = 5`    | OK (annotation path, not a cast)                |
| `256 as i8`        | REJECT: `Cannot assert type 'int' as 'i8'`      |
| `5 as i32`         | REJECT: `Cannot assert type 'int' as 'i32'`     |
| `5 as i64`         | REJECT: `Cannot assert type 'int' as 'i64'`     |
| `5 as number`      | REJECT: `Cannot assert type 'int' as 'number'`  |

(The `as i64` / `as number` rejections are sibling over-rejections from the same
seam but are NOT in ROOT F's named test scope — see "Adjacent, out of ROOT-F
scope" below.)

## Is the cast valid Shape? YES — width casts are a designed, wrapping feature

This is not a range check that should fire. `i8`/`u8`/`i16`/`u16`/`i32`/`u32`/`u64`
are first-class width types (`crates/shape-ast/src/int_width.rs:16`, `IntWidth`
enum; note `i64` is deliberately excluded — it is the default `int`). Width casts
do **Rust-style bit truncation**, never reject:

- `crates/shape-ast/src/int_width.rs:329`: `IntWidth::I8.truncate(256) == 0`.
- So `256 as i8` is *defined* to evaluate to `0`. `expect_run_ok()` is correct.

The bytecode compiler ALREADY treats this exact form as a valid width cast:

`crates/shape-vm/src/compiler/expressions/type_ops.rs:876-888`

```rust
// ── Width integer cast: `expr as i8`, `expr as u16`, etc. ──
// Emits CastWidth which does bit-truncation (Rust-style). Not Into-based.
if let TypeAnnotation::Basic(name) = type_annotation {
    if let Some(w) = shape_ast::IntWidth::from_name(name) {
        self.compile_expr(expr)?;
        self.emit(Instruction::new(
            OpCode::CastWidth,
            Some(Operand::Width(NumericWidth::from_int_width(w))),
        ));
        self.last_expr_numeric_type = Some(crate::type_tracking::NumericType::IntWidth(w));
        return Ok(());
    }
}
```

The `CastWidth` opcode (`0xF7`) is fully wired end to end — emitted by the
compiler, dispatched at `crates/shape-vm/src/executor/dispatch.rs:585-586`,
implemented (truncation) at `crates/shape-vm/src/executor/arithmetic/mod.rs:641`.
Nothing about the runtime is missing; only the *type checker* refuses the program
before the working compiler path is reached.

## Root cause / exact seam

`crates/shape-runtime/src/type_system/inference/expressions.rs` — the
`Expr::TypeAssertion` arm (lines 347-377). The checker has **no width-cast
carve-out** that mirrors the compiler's lines 876-888. The decisive line:

`crates/shape-runtime/src/type_system/inference/expressions.rs:368`

```rust
// Plain `as Type` is trait-dispatched conversion when Type is a
// concrete named target supported by Into<Target>.
if self.try_into_selector(&asserted_type).is_some() {
    self.validate_infallible_conversion(&expr_type, &asserted_type)?;  // <-- rejects here
    return Ok(asserted_type);
}
```

`try_into_selector` (same file, line 1763) returns `Some("i8")` for the target
`TypeAnnotation::Basic("i8")` — it just extracts the basic name and canonicalizes
it (`canonical_try_into_name`, line 1786, does not special-case width types). That
routes the width cast into `validate_infallible_conversion` (line 1668), where
`source_name == "int"`, `target_selector == "i8"`, `types_equal(int, i8) == false`,
and there is no `Into<i8>` impl for `int`. So it falls through to the final
`Err(TypeError::InvalidAssertion(...))` at line 1706-1709 → the
`Cannot assert type 'int' as 'i8'` message
(`crates/shape-runtime/src/type_system/errors.rs:54-55`).

The asymmetry: the compiler does Option-fallible → **width** → Into-infallible;
the checker does Option-fallible → Into-infallible (width arm missing).

## Minimal fix (FP — fix the checker)

Add the width-cast bypass to the inference `Expr::TypeAssertion` arm, in the same
ordering the compiler uses (after the Option-fallible check, before the
`try_into_selector` Into gate). A width target is statically valid — it always
succeeds with truncation — so it must not be subjected to `Into<Target>`
validation.

File: `crates/shape-runtime/src/type_system/inference/expressions.rs`
Insert between line 364 (`let asserted_type = self.resolve_type_annotation(...)`)
and line 366/368 (the `try_into_selector` Into gate):

```rust
let asserted_type = self.resolve_type_annotation(type_annotation);

// Width integer cast: `expr as i8`, `expr as u16`, etc. is a Rust-style
// bit-truncating conversion (compiler emits `OpCode::CastWidth`, NOT an
// Into dispatch — see compiler/expressions/type_ops.rs:876-888). It is
// statically infallible (truncates, never rejects), so it must bypass the
// Into<Target> validation below. Mirrors the compiler's cast ordering.
if let TypeAnnotation::Basic(name) = type_annotation {
    if shape_ast::IntWidth::from_name(name).is_some() {
        return Ok(asserted_type);
    }
}

// Plain `as Type` is trait-dispatched conversion when Type is a
// concrete named target supported by Into<Target>.
if self.try_into_selector(&asserted_type).is_some() {
    ...
```

Notes:
- `shape_ast` is already in scope (used at `expressions.rs:9`); `IntWidth` is the
  same path the compiler uses (`shape_ast::IntWidth::from_name`). No new import
  strictly required (`shape_ast::IntWidth::from_name(...)` works as a full path).
- `IntWidth::from_name` covers exactly the 7 real width names: `i8, u8, i16, u16,
  i32, u32, u64`. `i64`/`int`/`number` are intentionally NOT width names, so this
  guard does not change their behavior (see below).
- Returning `asserted_type` (the resolved `i8` type) matches the compiler stamping
  `NumericType::IntWidth(w)` for the cast result. No constraint is pushed — a
  width cast is unconditional, so unifying the source with the target (as the
  `as Type` strict-assertion arm at line 374 does) would be wrong.
- This is a checker carve-out only — it does NOT touch the runtime/compiler, does
  NOT add or rename any dispatch path, and does NOT introduce dynamic fallback. It
  removes a spurious rejection so the already-correct `CastWidth` path runs.

## Adjacent, out of ROOT-F scope (do NOT fold in without a ruling)

- `5 as i64` and `5 as number` are rejected by the same line-368 seam but are
  NOT width-from_name targets (`i64` is the default int; `number` is f64). Whether
  `int as i64` should be an identity-accept and whether `int as number` should be
  an accepted widening are separate language-semantics questions tracked by their
  own A-final roots (likely the int/number-widening + i64-alias cluster). ROOT F's
  fix deliberately scopes to `IntWidth::from_name` so it touches ONLY the 7 real
  width types and clears exactly the named test.

## Classification

**FP_fix_checker.** Valid Shape (`256 as i8` → `0` by defined truncation) is
over-rejected by a missing width-cast carve-out in the type-inference
`Expr::TypeAssertion` arm. The runtime/compiler already implement the cast
correctly; the checker rejects before reaching it. Fix is a localized checker
carve-out. NOT a re-baseline (no test anywhere expects width casts to reject —
verified by grep across `tools/shape-test/tests/`, `crates/shape-vm/src/`,
`crates/shape-runtime/src/`).

## Files the fix touches (conflict-grouping)

- `crates/shape-runtime/src/type_system/inference/expressions.rs` (single arm,
  `Expr::TypeAssertion`, ~lines 364-368). One file.
