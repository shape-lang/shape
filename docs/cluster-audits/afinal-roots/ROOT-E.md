# A-final ROOT E — Result/Option match-arm payload binder loses its element type (int → number drift)

- **Baseline:** strict-flip `@f01e8323` worktree
  (`/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch`),
  release binary `target/release/shape` (let-gen landed, ROOT A cleared).
- **Verdict:** **FP_fix_checker** — the code is genuinely `int`-throughout; the
  checker over-rejects by widening the matched payload to `number`.
- **Failing tests (2):**
  - `error_handling::stress_ok_err::two_ok_results_matched`
  - `error_handling::stress_ok_err::one_ok_one_err_results_matched`
- **Files the fix touches:**
  `crates/shape-runtime/src/type_system/inference/expressions.rs` (only).

---

## 1. The two test programs (reconstructed from the test source)

`tools/shape-test/tests/error_handling/stress_ok_err.rs:389` and `:398`.

`two_ok_results_matched` (`.expect_number(30.0)`):

```shape
fn test() -> int { let a = Ok(10)
let b = Ok(20)
let va = match a { Ok(v) => v, Err(e) => 0 }
let vb = match b { Ok(v) => v, Err(e) => 0 }
va + vb }
test()
```

`one_ok_one_err_results_matched` (`.expect_number(10.0)`):

```shape
fn test() -> int {
    let a = Ok(10)
    let b = Err("fail")
    let va = match a { Ok(v) => v, Err(e) => 0 }
    let vb = match b { Ok(v) => v, Err(e) => 0 }
    va + vb
}
test()
```

The harness assertion is not the problem: `expect_number` →
`extract_number` (`tools/shape-test/src/shape_test.rs:256`) accepts an
`Integer`-tagged result. The program is valid: every literal is an `int`
(`10`, `20`, `0`), the `Err` payload is a `string`, and `va + vb` is
`int + int`. The function is declared `-> int`. There is **no** int/number mix
in the source. → not a TP.

## 2. Verbatim strict-flip rejection (reproduced)

Both programs, run with `target/release/shape run <file>`:

```
Error: Runtime error: Bytecode compilation failed: Semantic error: Could not solve type constraints:
  number is not compatible with int
```

## 3. Minimization — where the kind drifts

| Program (all `-> int`) | Result |
|---|---|
| `match a { Ok(v) => v, Err(e) => 0 }` returned **alone** | OK → `Integer 10` |
| `let va = match …; va` (binding, no arithmetic) | OK |
| `let va = match …; va + vb` (two matches) | **REJECT** |
| `let va = match …; va + 20` (match + plain int) | **REJECT** |
| `let va = match …; va + va` (single match, self-add) | **REJECT** |
| `let va = match …; va * 2` (single match) | **REJECT** |
| `let va: int = match …; va + vb` (binder annotated) | OK |
| `let va = match a { Ok(v) => 5, Err(e) => 0 }; va + 1` (both arms **literal int**) | OK |
| `enum MyR { Ok(int), Bad }` user enum, same shape, `va + 1` | OK |
| **no** `-> int` annotation, `va + vb` / `va * 2` | OK (yields `Integer`) |

Reading: the rejection needs **(payload-binder return) + (arithmetic) +
(`-> int` annotation)** together. A single matched value returned directly
resolves to `int` and satisfies `-> int`; feeding it through `+`/`*` and then
the `-> int` return triggers the conflict. Annotating the binder (`let va: int`)
or making **both** match arms concrete-int literals removes the failure. A user
enum with a typed payload (`MyR::Ok(int)`) never fails.

## 4. Root cause — exact seam

`match { Ok(v) => v, Err(e) => 0 }` infers as follows
(`crates/shape-runtime/src/type_system/inference/expressions.rs`):

1. Match arms are inferred independently (`Expr::Match`, lines 634–699). The
   arm result types are collected, then:
   - `all_types_equal(&arm_types)` (no constraint pushed; structural compare
     only, `inference/mod.rs:839` → `types_equal`) → if equal, take `arm[0]`;
   - else `create_nominal_union(&arm_types)` (`inference/mod.rs:859`).
   **The two arms are never unified with a same-type constraint.**

2. The `Ok(v)` payload binder `v` is bound in
   **`bind_pattern_vars_typed`, `Pattern::Constructor` → `Tuple` arm
   (`expressions.rs:1503–1552`)**. Payload types are pulled **only** from a
   *user* enum via `enum_name_of_type(ty)` + `env.get_enum(name)`
   (`expressions.rs:1514–1523`; `enum_name_of_type` at `items.rs:1888`).
   `Result`/`Option` are **not** user enums — `Ok`/`Err`/`Some`/`None` are
   built-in polymorphic constructor functions (`environment/mod.rs:943–981`),
   and the scrutinee `Ok(10)` infers as
   `Type::Generic { base: Reference("Result"), args: [int, AnyError] }`
   (`inference/mod.rs:485`). `get_enum("Result")` → `None`, so
   `enum_kind = None`, `payload_tys = None`, and **`v` binds to an
   unconstrained fresh type var** (`bind_pattern_vars_typed`,
   `Pattern::Identifier` arm, `expressions.rs:1464–1467`) that is never linked
   to the scrutinee's `args[0] = int`.

3. Consequence: the `Ok` arm yields a bare `Type::Variable` while the `Err`
   arm yields concrete `int`. `all_types_equal` is **false** (a Variable is not
   structurally equal to `int`), so the match result becomes a **nominal union**
   `Union<unknown, int>` (`create_nominal_union`, `inference/mod.rs:859`;
   `type_name_for_union` maps the Variable to `"unknown"`,
   `inference/mod.rs:920`).

4. `va` therefore has a non-`Basic` concrete type (the union). In
   `va + vb` / `va + 20`, `infer_numeric_arithmetic_op` →
   `numeric_result_type` (`inference/operators.rs:139`) hits the catch-all
   `_ => BuiltinTypes::number()` arm (line 185): neither operand is a
   `Concrete(Basic(numeric))` nor a `Variable`, so it **defaults to `number`**.
   That `number` then unifies against the `-> int` return → `number is not
   compatible with int`.

This is exactly the cluster the pre-strict-flip classification already named:
`docs/cluster-audits/v0.3-classification/error_handling.md:123` "Group E —
Match-arm enum-payload `unknown` in arithmetic" and
`.../ALLOWLIST.md:54` "Result/Option match-arm payload `unknown` in
arithmetic". (Those two A-final tests use `Ok(v) => v` rather than the
`Ok(v) => v + N` of the 7 listed Group-E tests, but the binder-typing root is
identical.)

**Why it is an FP and not a TP:** the binder *should* be `int` because
`Ok(10)`'s success element is `int`. The checker has the information
(`scrutinee.args[0] == int`) and simply fails to wire the binder to it for the
built-in `Result`/`Option` generics. With the binder correctly typed `int`,
both arms are `int`, the union collapses to `int`, and `va + vb` stays `int`.

## 5. Minimal fix (the exact edit + seam)

**Seam:** `crates/shape-runtime/src/type_system/inference/expressions.rs`, the
`PatternConstructorFields::Tuple(patterns)` block inside
`bind_pattern_vars_typed`'s `Pattern::Constructor` arm (lines 1529–1552).

**Edit:** before the `for (idx, p) …` loop, when `enum_kind` is `None`, derive
the payload `Type` for built-in `Result`/`Option` scrutinees directly from the
scrutinee's generic args, keyed by the `variant` name. Concretely, compute a
`builtin_payload: Option<Type>` and prefer it over the user-enum
`payload_tys`:

```rust
PatternConstructorFields::Tuple(patterns) => {
    let payload_tys: Option<Vec<TypeAnnotation>> = match &enum_kind {
        Some(shape_ast::ast::EnumMemberKind::Tuple(types)) => Some(types.clone()),
        _ => None,
    };

    // Built-in Result<T,E> / Option<T> payload binders. `Ok`/`Some` →
    // args[0] (success/inner element); `Err` → args[1] (error element).
    // Single-element tuple payload (the only shape Ok/Err/Some take), so
    // it maps to pattern index 0. Uses the scrutinee's already-resolved
    // generic-arg `Type` directly (no re-resolution).
    let builtin_payload: Option<Type> = if payload_tys.is_none() {
        match scrutinee {
            Some(Type::Generic { base, args })
                if matches!(
                    base.as_ref(),
                    Type::Concrete(ann)
                        if matches!(ann.as_type_name_str(), Some("Result") | Some("Option"))
                ) =>
            {
                match variant.as_str() {
                    "Ok" | "Some" => args.get(0).cloned(),
                    "Err"         => args.get(1).cloned(),
                    _             => None,
                }
            }
            _ => None,
        }
    } else {
        None
    };

    for (idx, p) in patterns.iter().enumerate() {
        let field_ty = payload_tys
            .as_ref()
            .and_then(|tys| tys.get(idx).map(|ann| self.resolve_type_annotation(ann)))
            .or_else(|| if idx == 0 { builtin_payload.clone() } else { None });
        self.bind_pattern_vars_typed(p, field_ty.as_ref())?;
        if let (Pattern::Identifier(bind_name), Some(ft)) = (p, &field_ty) {
            self.env.define(bind_name, TypeScheme::mono(ft.clone()));
        }
    }
}
```

(Implementer note: `variant` is the `Pattern::Constructor { variant, .. }`
field already in scope at line 1504; `as_type_name_str()` is the same accessor
used by the sibling `Result`/`Option` matches in `operators.rs:104` and
`inference/mod.rs:414`. Match `scrutinee` may also arrive as
`Type::Concrete(TypeAnnotation::Generic { name, args })` for some construction
paths — mirror the dual shape already handled in
`operators.rs::unwrap_result_or_option_type` (lines 102–117) if the
`Type::Generic` arm alone proves insufficient in testing; for `Ok(10)` the
scrutinee is the `Type::Generic { Reference("Result"), .. }` shape.)

This keeps the change inside the one binder-typing seam, touches no opcodes,
no runtime, no `numeric_result_type` (the arithmetic defaulting is correct
*given* a proper binder type). It is a pure inference-completeness fix on the
forbidden-pattern-free type-inference layer.

### Why the fix is sound (does not mask real mismatches)

Verified on the strict-flip binary that these MUST keep rejecting / passing,
and the fix preserves each:

- `Ok(1.5)` (success = `number`) with `-> int`, `va + 1` → still **REJECT**
  after fix: `v` becomes `number`, `number != int`.
- `Ok(10)` + `Err(e) => 0.5` (genuinely mixed arms) with `-> int` → still
  **REJECT**: Ok arm `int`, Err arm `number` → real union, `int`/`number`
  distinct.
- `Ok(1.5)` with `-> number`, `va + 1.0` → already passes; still passes.

## 6. Classification

**FP_fix_checker.** Valid `int`-throughout code is over-rejected because the
checker fails to type the built-in `Result`/`Option` match-payload binder from
the scrutinee's generic element, producing a `Union<unknown, int>` that the
arithmetic path widens to `number` and then conflicts with the `-> int`
return. The fix is the exact inference edit in §5; no test re-baseline.

- **files_touched:** `crates/shape-runtime/src/type_system/inference/expressions.rs`
- **clears:** `error_handling::stress_ok_err::two_ok_results_matched`,
  `error_handling::stress_ok_err::one_ok_one_err_results_matched`
  (both reproduce verbatim on the strict-flip binary as
  `number is not compatible with int`).
- **Likely also cleared (same root, verify in the run):** the Group-E /
  ALLOWLIST:54 cluster — `stress_ok_err::match_ok_with_computation_in_arm`,
  `match_ok_with_multiply`, `result_creation::result_match_ok_extracts_value`,
  `result_match_with_computation_in_arm`, `result_in_array`, and the
  `Some(v)`/Option arithmetic siblings.
