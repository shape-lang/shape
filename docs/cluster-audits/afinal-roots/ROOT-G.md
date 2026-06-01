# A-final ROOT G — single-element Union not collapsed for `Comparable`

**Verdict: `FP_fix_checker`** (valid code over-rejected → fix the checker).

## Failing test

`error_handling::edge_cases::edge_result_as_if_condition_value`
(`tools/shape-test/tests/error_handling/edge_cases.rs:603`)

Reconstructed program:

```shape
fn get() -> Result<number> { Ok(42) }
let r = get()
let val = match r {
    Ok(v) => v
    Err(_) => 0
}
if val > 10 { "big" } else { "small" }
```

Expected result: `"big"`.

## Reproduction on the strict-flip binary (@f01e8323)

```
$ target/release/shape run /tmp/root_g.shape
Error: Runtime error: Bytecode compilation failed: Semantic error:
Type constraint violation: Concrete(Union([Basic("int")])) is not comparable
```

Reproduces verbatim. The rejection text `Concrete(Union([Basic("int")])) is not comparable`
matches the classification note exactly (singleton `Union([int])` not collapsed for the
Comparable/Ord solve).

## Root cause (exact seam)

`crates/shape-runtime/src/type_system/constraints.rs:771-783` — the
`TypeConstraint::Comparable` arm of `check_constraint`.

The arm matches **only** `Type::Concrete(TypeAnnotation::Basic(name))`:

```rust
TypeConstraint::Comparable => match ty {
    Type::Concrete(TypeAnnotation::Basic(name))
        if BuiltinTypes::is_numeric_type_name(name)
            || name == "string"
            || name == "bool" =>
    {
        Ok(())
    }
    _ => Err(TypeError::ConstraintViolation(format!(
        "{:?} is not comparable",
        ty
    ))),
},
```

`val` resolves to `Type::Concrete(TypeAnnotation::Union([Basic("int")]))`. This is a
**degenerate** union: a match expression whose arms all yield the same type
(`Ok(v) => v` is `int`, `Err(_) => 0` is `int`) accumulates into `Union([int])`
during inference. `Union([int])` is semantically just `int`, but the `Comparable`
arm never collapses it, so it falls through to the `_ =>` error arm.

The `>` operator emits this constraint at
`crates/shape-runtime/src/type_system/inference/operators.rs:415-438`
(`BinaryOp::Less | Greater | LessEq | GreaterEq` push `TypeConstraint::Comparable`
onto the operand type).

## Why this is a checker bug (FP), not a TP

The blessed collapse helper **already exists** and was introduced for exactly this
class of degenerate match-arm unions:

`crates/shape-runtime/src/type_system/constraints.rs:46-64`
`fn collapse_degenerate_union(ann: &TypeAnnotation) -> &TypeAnnotation` — collapses
`Union([T, T, ...])` (incl. the single-element `Union([T])`) to `T` when **all**
members are structurally equal; leaves a genuinely heterogeneous union
(`Union([int, string])`) intact.

It is already wired into the `ImplementsTrait` arm at
`constraints.rs:991` ("A match-accumulate union whose arms all yield the same
type (`Union([int])`) is just that type — collapse it before extracting the name
so `Numeric`/etc. resolve."), with two regression tests:

- `test_implements_trait_single_member_union_collapses` (constraints.rs:1816) —
  `Union([number])` collapses and satisfies `Numeric`.
- `test_implements_trait_heterogeneous_union_still_violates` (constraints.rs:1845,
  marked `NOT-BROAD-SUPPRESSION`) — `Union([number, string])` still violates.

The `Comparable` arm simply never received the same treatment. It is an oversight
in the established pattern, not a deliberate semantic boundary. The program is
valid strict-typing Shape (`val: int`, `int > int` is well-typed), so the strict
flip is over-rejecting it. This is FP — fix the checker.

This is NOT a forbidden-pattern concern: collapsing a structurally-uniform union to
its single member is a pure type-level simplification, not runtime dynamic dispatch,
not a coercion opcode, and not a tag/value-word path. It mirrors the existing,
in-tree `ImplementsTrait` collapse exactly.

## Minimal fix (exact edit, the seam)

`crates/shape-runtime/src/type_system/constraints.rs:771-783`

Restructure the `Comparable` arm to match `Type::Concrete(ann)`, collapse a
degenerate union, then run the existing basic-name check on the collapsed
annotation. Replace:

```rust
            TypeConstraint::Comparable => match ty {
                Type::Concrete(TypeAnnotation::Basic(name))
                    if BuiltinTypes::is_numeric_type_name(name)
                        || name == "string"
                        || name == "bool" =>
                {
                    Ok(())
                }
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} is not comparable",
                    ty
                ))),
            },
```

with:

```rust
            TypeConstraint::Comparable => match ty {
                Type::Concrete(ann) => {
                    // A match-accumulate union whose arms all yield the same
                    // type (`Union([int])`) is just that type — collapse it
                    // before the comparability check (mirrors the
                    // ImplementsTrait arm at constraints.rs:991). A genuinely
                    // heterogeneous union is left intact and still fails below.
                    let ann = collapse_degenerate_union(ann);
                    match ann {
                        TypeAnnotation::Basic(name)
                            if BuiltinTypes::is_numeric_type_name(name)
                                || name == "string"
                                || name == "bool" =>
                        {
                            Ok(())
                        }
                        _ => Err(TypeError::ConstraintViolation(format!(
                            "{:?} is not comparable",
                            ty
                        ))),
                    }
                }
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} is not comparable",
                    ty
                ))),
            },
```

Notes:
- `collapse_degenerate_union` returns `&TypeAnnotation`, so binding `let ann = ...`
  shadows the inner `ann` reference with no clone.
- The heterogeneous-union case (`Union([int, string])`) does NOT collapse (members
  differ) and correctly falls into the inner `_ =>` error arm — comparing mixed
  types stays a compile error.
- A non-degenerate non-Basic concrete type (Array, Object, etc.) still fails as
  before.

### Suggested regression test (add alongside the existing collapse tests)

Mirror `test_comparable_accepts_int` + the `ImplementsTrait` collapse pair:

```rust
#[test]
fn test_comparable_single_member_union_collapses() {
    let mut solver = ConstraintSolver::new();
    let mut tvgen = TypeVarGen::new();
    let bound_var = fresh_var(&mut tvgen);
    let mut constraints = vec![(
        Type::Concrete(TypeAnnotation::Union(vec![TypeAnnotation::Basic(
            "int".to_string(),
        )])),
        Type::Constrained {
            var: bound_var,
            constraint: Box::new(TypeConstraint::Comparable),
        },
    )];
    assert!(
        solver.solve(&mut constraints).is_ok(),
        "Union([int]) should collapse to int and satisfy Comparable"
    );
}

#[test]
fn test_comparable_heterogeneous_union_still_violates() {
    let mut solver = ConstraintSolver::new();
    let mut tvgen = TypeVarGen::new();
    let bound_var = fresh_var(&mut tvgen);
    let mut constraints = vec![(
        Type::Concrete(TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ])),
        Type::Constrained {
            var: bound_var,
            constraint: Box::new(TypeConstraint::Comparable),
        },
    )];
    assert!(
        solver.solve(&mut constraints).is_err(),
        "Union([int, string]) is heterogeneous and must NOT satisfy Comparable"
    );
}
```

## Files the fix touches (conflict-grouping)

- `crates/shape-runtime/src/type_system/constraints.rs` (single file — the
  `Comparable` arm of `check_constraint`, plus optional inline regression tests in
  the same file's `#[cfg(test)]` module).

## Tests this clears

- `error_handling::edge_cases::edge_result_as_if_condition_value` — directly cleared
  (the `if val > 10` Comparable check on `Union([int])` now passes; program runs and
  returns `"big"`).

Likely also clears any sibling A-final tests in the same root where a degenerate
`Union([T])` flows into a relational comparison (`<`, `>`, `<=`, `>=`) — same seam,
same collapse.
