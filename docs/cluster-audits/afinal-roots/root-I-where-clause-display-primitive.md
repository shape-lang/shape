# A-final ROOT I — where-clause `Display` bound on a builtin primitive

**Date:** 2026-06-01
**Strict-flip baseline:** worktree `shape-strict-flip-collection-dispatch` @ `f01e8323` (let-gen landed; ROOT A cleared).
**Failing test:** `generics::bounds::where_clause_with_function_body`
**Classification source:** `/home/dev/dev/shape-lang/shape/docs/cluster-audits/v0.3.3-a-final-classification.md` §2 Root I (flagged FP/TP-boundary, "called FP because the bound is satisfiable in spirit; flagged for review").

## VERDICT: TP_rebaseline_test

The strict-flip **correctly rejects** the program. `int` genuinely does NOT implement the `Display` trait in Shape. The "satisfiable in spirit" rationale in the classification report conflated *printability* (a separate builtin formatting path) with *trait-bound satisfaction* (the `Display` trait). This is NOT a checker over-rejection — it is correct strict behavior. The test must be re-baselined as a must-reject, NOT softened in the checker.

## The program (reconstructed from test source, bounds.rs:123-134)

```shape
fn identity<T>(x: T) -> T where T: Display {
    return x
}
identity(42)
```

Test expectation today: `.expect_number(42.0)` — i.e. it expects the program to RUN and yield `42`.

## Reproduction on the strict-flip binary (verbatim)

```
$ /home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape run /tmp/root_i_repro.shape
Error: Runtime error: Bytecode compilation failed: Semantic error: Type 'int' does not implement trait 'Display'
```

Non-strict baseline (`/home/dev/dev/shape-lang/shape/target/release/shape`): exit 0, runs cleanly — confirms this is a strict-flip regression-candidate (passes on baseline, fails only under strict flip).

## Why this is a TP (the language-semantics determination)

The task asked: does `int` implement `Display` in Shape? Three independent lines of evidence say **no**:

### 1. The canonical `Display` trait is user-implementable; no builtin primitive impl exists

`crates/shape-runtime/stdlib-src/core/display.shape` defines the only real `Display`:

```shape
trait Display {
    method display() -> content;   // NOTE: method is display() -> content, NOT to_string()
}
```

Its doc-comment is explicit: *"Implement `Display` for user-defined types that should participate in printing..."* — there is no `impl Display for int` / `for number` / `for string`. The prelude (`stdlib-src/core/prelude.shape:9`) only does `from std::core::display use { Display }` — it imports the trait, it does NOT register any primitive impl.

Every `Display::int`-style `register_trait_impl` in the tree is inside `#[cfg(test)]` fixtures in `crates/shape-runtime/src/type_system/environment/registry.rs`, and those fixtures even use the wrong method name (`to_string`), so they're test scaffolding, not the shipped trait.

### 2. `print(42)` works through a SEPARATE path, not the `Display` bound

```
$ shape run  <print(42); print(f"value is {42}")>
42
value is 42
```

`int` is printable via the builtin formatting / content-dispatch path (`enum_support.rs:10` "Drives print()/Display rendering", content_dispatch). That printability is NOT the `Display` *trait bound* — the where-clause solver checks for a registered `impl Display for <type>`, which does not exist for `int`. "int is printable" ≠ "int satisfies `T: Display`". The report's FP rationale conflated the two.

### 3. Control experiment — the solver is correct, not broken

A user type WITH an actual impl satisfies the *exact same* `identity<T> where T: Display` bound and runs:

```shape
type Tag { name: string }
impl Display for Tag { method display() -> content { return Content.text(self.name) } }
fn identity<T>(x: T) -> T where T: Display { return x }
let t = identity(Tag { name: "hi" })
print(t.name)
```
→ prints `hi` on the strict-flip binary.

So the where-clause `Display` solver accepts types that implement `Display` and rejects those that don't. It is behaving correctly; `int` is correctly in the "doesn't implement" set.

### 4. Test-suite intent corroborates rejection

Every OTHER `Display`-bound test in the suite (`generics/bounds.rs:13` `render<T: Display>`, `traits/bounds.rs:48` `process<T: Display + Serializable>`, `traits/bounds.rs:103` `transform<T> where T: Display`, `:115` multi-bound) is `.expect_parse_ok()` only — they assert the *definition compiles*, never that it is *called with a primitive*. The single test that actually calls a `Display`-bounded fn with a real value (`traits/bounds.rs:25` `function_with_trait_bound_dispatches`) supplies a user `Item` type WITH `impl Displayable for Item`. `where_clause_with_function_body` is the **only** test that calls a `Display`-bounded fn with a primitive that has no impl — i.e. it is uniquely the one that exercises the (correct) rejection. No other test regresses from this rejection.

## The seam (where the rejection is decided)

`crates/shape-runtime/src/type_system/constraints.rs:997` — inside `TypeConstraint::ImplementsTrait` for `Type::Concrete`:

```rust
if self.has_trait_impl(trait_name, &type_name) {   // type_name == "int", trait_name == "Display"
    Ok(())
} else {
    Err(TypeError::TraitBoundViolation {           // constraints.rs:1000 — emits "Type 'int' does not implement trait 'Display'"
        type_name, trait_name: trait_name.clone(),
    })
}
```

`has_trait_impl` (`constraints.rs:1088`) consults the solver's `trait_impls` set, populated from `env.trait_impl_keys()` (`inference/mod.rs:1337,1403`) — the registry's `impl Trait for Type` declarations plus numeric-alias/widening fallbacks (`constraints.rs:1094-1115`). No `Display::int` (nor `Display::number`/`Display::f64` that `int` would widen to) is ever inserted, because no such impl exists in stdlib or prelude. The rejection at constraints.rs:1000 is therefore correct and well-founded.

**Do NOT "fix" the checker.** Registering a phantom builtin `impl Display for int` to make this one test pass would be a checker weakening that breaks the trait's contract (`Display::display() -> content` is not satisfiable by a bare `int` without a real impl) and would silently accept genuinely under-constrained code elsewhere. The bound is unsatisfied by design.

## FIX RECIPE (re-baseline the test as must-reject)

Edit `tools/shape-test/tests/generics/bounds.rs`, function `where_clause_with_function_body` (lines 123-134). Change the assertion from `expect_number` to a must-reject assertion and update the intent comment.

Replace line 133:
```rust
    .expect_number(42.0);
```
with:
```rust
    .expect_run_err_contains("does not implement trait 'Display'");
```

(`expect_run_err_contains` is defined at `tools/shape-test/src/shape_test.rs:1272`; it asserts `eval()` returns `Err` containing the substring — the compile-stage `TraitBoundViolation` surfaces through `eval()`'s `Result`, verified verbatim above.)

Recommended: also update the comment at bounds.rs:124 to note the rebaseline, e.g.:
```rust
    // Strict typing: `int` has no `impl Display`, so `identity(42)` is correctly
    // rejected (T: Display bound unsatisfied). Re-baselined to must-reject — A-final ROOT I.
```

Secondary note (not load-bearing): the old `.expect_number(42.0)` was itself loose — `identity(42)` is an `int`, not a `number` (`42.0`). The rebaseline moots this; no separate fix needed.

## Files the fix touches (for conflict-grouping)

- `tools/shape-test/tests/generics/bounds.rs` (single test fn, lines 123-134; assertion + comment only)

No compiler/runtime source changes. No conflict with other A-final roots (none of roots A–J touch this test or constraints.rs:997 in a way that overlaps — root G touches a *different* arm of `ImplementsTrait`/`Comparable` at constraints.rs:770/991 collapse, but this fix changes only the test file, so zero source-level conflict).

## Clears

- `generics::bounds::where_clause_with_function_body` — reproduces verbatim on strict-flip @ f01e8323:
  `Type 'int' does not implement trait 'Display'`. After rebaseline to `expect_run_err_contains("does not implement trait 'Display'")`, the test passes (asserts the correct rejection).
