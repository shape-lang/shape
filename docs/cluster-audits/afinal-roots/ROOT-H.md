# A-final ROOT H — nested-function generic-annotation representation gap

**Baseline:** strict-flip `shape-strict-flip-collection-dispatch` @ `f01e8323` (let-gen landed; ROOT A cleared).
**Failing test:** `error_handling::edge_cases::edge_try_in_nested_function`
**Classification:** **FP_fix_checker** — valid Shape code is over-rejected. The fix is a one-line representation-normalization on the nested-function inference path; **not** a checker-relaxation, not a test re-baseline.
**Read-only audit.** No source/fixture/build/commit/stash changes were made.

---

## 1. Verbatim rejection (run-verified on the strict-flip release binary)

`target/release/shape run` on the reconstructed test program:

```shape
fn inner() -> Result<number> { Ok(42) }
fn outer() -> Result<number> {
    fn nested() -> Result<number> {
        let v = inner()?
        Ok(v + 1)
    }
    let r = nested()?
    Ok(r)
}
match outer() {
    Ok(v) => v
    Err(_) => -1
}
```

```
Error: Runtime error: Bytecode compilation failed: Semantic error: Could not solve type constraints:
  Result<number, AnyError> is not compatible with Result<number>
  () -> Result<number> is not compatible with () -> Result<number>
```

Expected (per the test): `expect_number(43.0)`.

The classification note ("a `?`-expanded default-error param type fails to unify with the `Result<T>` shorthand") is the **visible artifact, not the root.** The `?`/`AnyError`/shorthand machinery is sound here; the real trigger is the **nested function definition**.

## 2. Isolation (the `?`/AnyError/shorthand framing is a red herring)

All run on the same binary, filtered for the constraint error:

| # | Program shape | Result |
|---|---------------|--------|
| T1 | single-level `?`, `Result<number>` shorthand, **no nesting** | **PASS** → `43.0` |
| N9 | two **sibling top-level** fns, `?`, `Result<number>` shorthand | **PASS** → `7` |
| N1 | **nested** fn, `?`, `Result<number>` shorthand | FAIL (the ROOT-H repro) |
| N2 | **nested** fn, **no `?` at all**, plain `Ok(7)`/`Ok(8)` | FAIL: `Result<int, AnyError> is not compatible with Result<number>` |
| N5 | **nested** fn, **fully explicit `Result<number, string>` everywhere** | FAIL: `Result<number, string> is not compatible with Result<number, string>` (identical types!) |
| N8 | **nested** fn returning `Option<int>` (no `?`, no AnyError) | FAIL: `Option<int> is not compatible with Option<int>` (identical types!) |
| N6 | **nested** fn returning `int` (non-generic) | **PASS** → `5` |

The decisive facts:
- N5/N8 fail with **identical generic types on both sides** failing to unify → not a shorthand/AnyError problem at all.
- N6 (nested fn, non-generic `int` return) passes → the problem is specific to **generic** return/param annotations.
- N9 (sibling top-level) passes; N1 (lexically nested) fails → the problem is specific to the **nested-function** inference path.

Root = a nested (lexically-inner) function whose return type (or param type) is a **generic** annotation (`Result<…>`, `Option<…>`, `Array<…>`, `HashMap<…>`, …). The `?`/AnyError/shorthand in the original test merely adds the second `Result<number, AnyError>` line; remove them and the bug still fires.

## 3. Root cause — the EXACT seam

A nested `fn name(params) -> Ret { body }` is desugared at parse time into
`let name = fn(params) -> Ret { body }` — a `VariableDecl` whose value is an
`Expr::FunctionExpr` (`crates/shape-ast/src/parser/statements.rs:70-91`). It is
therefore type-inferred through the `Expr::FunctionExpr` arm of `infer_expr`,
**not** through `infer_function`.

**Seam: `crates/shape-runtime/src/type_system/inference/expressions.rs:876` and `:899`** (the `Expr::FunctionExpr` arm, lines 864-907):

```rust
// param types (line 875-876):
let param_type = if let Some(ann) = &param.type_annotation {
    Type::Concrete(ann.clone())            // <-- :876  raw wrap, NOT resolved
...
// return type (line 898-901):
let ret_type = if let Some(ann) = return_type {
    let annotated = Type::Concrete(ann.clone());   // <-- :899  raw wrap, NOT resolved
    self.constraints.push((inferred_return, annotated.clone()));
    annotated
```

Both sites wrap the raw `TypeAnnotation` as `Type::Concrete(ann.clone())`. For a
generic annotation like `Result<number>`, the AST node is
`TypeAnnotation::Generic { name: "Result", args: [number] }`, so the produced
type is **`Type::Concrete(TypeAnnotation::Generic{…})`**.

The canonical top-level path does the opposite: `infer_function`
(`crates/shape-runtime/src/type_system/inference/items.rs:509` for params, `:542`
for return) calls **`self.resolve_type_annotation(ann)`**, which converts a
`TypeAnnotation::Generic` into the structured **`Type::Generic { base, args }`**
(`items.rs:736-745`). This `Type::Generic` shape is the documented invariant —
see the regression test `inference_tests.rs:407-422`, which asserts a top-level
`-> Result<int>` return is `Type::Generic { base: Result, args: [...] }`.

### Why the solver then refuses it

The constraint pushed at `expressions.rs:900` is
`inferred_return ~ Type::Concrete(TypeAnnotation::Generic{…})`. The
`inferred_return` (from `Ok(...)` / the `?`-rewrap via `wrap_result_type`,
`inference/mod.rs:485-492`) is a proper `Type::Generic { base, args }`.

`ConstraintSolver::solve_constraint`
(`crates/shape-runtime/src/type_system/constraints.rs:193-349`) has arms for:
- `(Type::Concrete, Type::Concrete)` → `unify_annotations` (line 227)
- `(Type::Generic, Type::Generic)` → **the Result-shorthand `(1,2)|(2,1)` + AnyError lattice logic** (lines 249-279)
- `(Type::Generic, Type::Concrete(Array))` / vice-versa (line 338)

There is **no `(Type::Generic, Type::Concrete(TypeAnnotation::Generic))`
cross-representation arm**, so the mixed pair falls to the wildcard
`_ => Err(TypeError::TypeMismatch)` at **`constraints.rs:345`**, stays unsolved,
and is reported as `UnsolvedConstraints` (`errors.rs:160-177`,
`format_unsolved_constraints`).

Crucially, even when the pair is `(Type::Concrete(Generic), Type::Concrete(Generic))`
(N5/N8, where both sides came through a `Type::Concrete` path — e.g. the
function-type-vs-function-type comparison at the call site recursing through
`unify_annotations`), it still fails: `unify_annotations`
(`constraints.rs:453-620`) has **no general `(Generic, Generic)` arm** either —
only the `Array`/`Generic` cross-compat case (line 593) — so two identical
`Result<number,string>` annotations fall to `_ => Ok(false)` (line 619). That is
the `Result<number, string> is not compatible with Result<number, string>` line.

The second reported line, `() -> Result<number> is not compatible with
() -> Result<number>`, is the same root surfacing through the
`Type::Function ~ Type::Function` arm (`constraints.rs:301`), which recurses into
the return types — the nested fn's scheme carries the un-normalized
`Type::Concrete(Generic)` return.

## 4. Minimal fix (the seam edit)

Align the nested-function path with the top-level path: resolve the annotations
through `resolve_type_annotation` instead of raw-wrapping. `resolve_type_annotation`
takes `&self` and is already used identically in `infer_function`, so it is
callable in this `&mut self` arm.

In `crates/shape-runtime/src/type_system/inference/expressions.rs`, the
`Expr::FunctionExpr` arm:

- **Line 876** — params:
  ```rust
  //   let param_type = if let Some(ann) = &param.type_annotation {
  //       Type::Concrete(ann.clone())
  // becomes:
        Type::Concrete(ann.clone())  ->  self.resolve_type_annotation(ann)
  ```
- **Line 899** — return type:
  ```rust
  //   let annotated = Type::Concrete(ann.clone());
  // becomes:
        let annotated = self.resolve_type_annotation(ann);
  ```

This produces `Type::Generic { base, args }` for generic annotations, so the
constraint `inferred_return ~ annotated` becomes `Type::Generic ~ Type::Generic`
and routes through the existing Result-shorthand + AnyError-lattice arm at
`constraints.rs:249-279`, exactly as the working top-level case does. Plain
non-generic annotations (`int`, `string`, `AnyError`) resolve to
`Type::Concrete(Basic/Reference)` unchanged, so closures `|x| ...` (which usually
omit `return_type`) and non-generic nested fns are unaffected.

Both lines should change together — the return-type edit (`:899`) clears the
two reported constraint lines for the ROOT-H test; the param edit (`:876`) is the
sibling normalization that prevents the same class re-appearing for a nested fn
with a generic **parameter** annotation (e.g. `fn f(m: HashMap<string,int>) {...}`
nested). Both mirror `infer_function`'s `:509` / `:542`.

### Why this is the producer-side fix, not a solver-side patch

The defect is that the nested-function path produces a *non-canonical type
representation* (`Type::Concrete(Generic)`) for something every other path
represents as `Type::Generic`. The correct fix normalizes at the producer to
restore the single-representation invariant the regression test
`inference_tests.rs:407-422` already encodes. Adding a
`(Generic, Concrete(Generic))` bridge arm to the solver would instead *bless* the
dual representation — a parallel-representation patch of exactly the shape the
project guidelines refuse. Normalize at the source.

## 5. Classification

**FP_fix_checker.** Valid Shape (a nested function with a generic return type,
identical in spirit to the working top-level form) is rejected at type-inference
time. The fix is the two-line normalization above; it relaxes nothing and
re-baselines no test as must-reject.

## 6. Files the fix touches (for conflict-grouping)

- `crates/shape-runtime/src/type_system/inference/expressions.rs` (the only edit —
  `Expr::FunctionExpr` arm, lines 876 + 899).

No solver edit, no error-formatting edit, no fixture edit.

## 7. Tests this clears

- `error_handling::edge_cases::edge_try_in_nested_function` (the named ROOT-H test;
  reproduces verbatim on the strict-flip binary @ f01e8323 with the two-line
  `Could not solve type constraints` rejection shown in §1).

Sibling programs that share this exact root and would also clear (audit probes,
not named tests): any nested fn returning `Result<…>` / `Option<…>` / `Array<…>` /
`HashMap<…>` (N1/N2/N3/N5/N8 above). Blast radius across the shape-test corpus is
narrow — most "nested-looking" fns in the test sources (e.g. `error_handling/
propagation.rs`, `enums/result.rs`) are actually **sibling top-level** fns at the
same raw-string indentation and already pass (N9).

## 8. Discipline

Audit-only. No defection-attractor framing used: the fix deletes a non-canonical
representation at its producer to restore the single `Type::Generic` invariant —
it is not a bridge/shim/adapter between two carriers and does not add a
cross-representation solver arm.
