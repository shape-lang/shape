# ROOT J — singleton binding/dispatch losses (J1 / J2 / J3)

Baseline: strict-flip `@f01e8323` (let-gen landed; ROOT A cleared).
Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape`
Source root: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/crates/`

All three sub-seams are **distinct, independent** type-checker (semantic-pass) bugs.
All three are **FP_fix_checker** — valid programs over-rejected by the strict-flip
checker. None is a runtime/VM-codegen gap: the VM bytecode compiler already
handles all three constructs; the rejections come purely from the type-inference
pass (`crates/shape-runtime/src/type_system/inference/`), confirmed by the
quoted error format `Undefined variable: '<x>'` (`type_system/errors.rs:18`,
quotes) vs the VM format `Undefined variable: <x>` (`dispatch.rs:1095`, no
quotes).

---

## J1 — `pub const` / `pub let` module binding dropped by the type checker

### Reproduces (verbatim, strict-flip binary)
Program (`imports_exports::test_export_pub_const_executes`):
```shape
pub const MAX_SIZE = 1024
MAX_SIZE
```
```
Error: Runtime error: Bytecode compilation failed: Semantic error: Undefined variable: 'MAX_SIZE'
```
Isolation (proves it is the export-wrapper, not const/let itself):
- `const MAX_SIZE = 1024; MAX_SIZE`  → WORKS (prints 1024)
- `pub let  MAX_SIZE = 1024; MAX_SIZE` → SAME rejection
- `pub const MAX_SIZE = 1024; MAX_SIZE` → SAME rejection
So the seam is the `pub` (Export) wrapper, independent of `const` vs `let`.
(`pub fn` works because the Export arm DOES have a `Function` case.)

### Exact seam
`crates/shape-runtime/src/type_system/inference/items.rs:243-281`
— `infer_item`'s `Item::Export(export, _)` arm.

`pub const NAME = expr` parses (parser `crates/shape-ast/src/parser/modules.rs:188-211`)
as `Item::Export(ExportStmt { item: ExportItem::Named([{name:"NAME"}]),
source_decl: Some(VariableDecl) })` (AST `crates/shape-ast/src/ast/modules.rs:35-40`).

The Export arm matches only `Function` / `TypeAlias` / `Struct` / `Trait` and
falls into `_ => {}` (items.rs:280) for `ExportItem::Named`. It **never inspects
`export.source_decl`**, so the carried `VariableDecl` is never inferred and
`NAME` is never `env.define`d. A later reference to `NAME` then fails at
`type_system/inference/expressions.rs:78` (`TypeError::UndefinedVariable`).

The VM side is already correct: `crates/shape-vm/src/compiler/statements.rs:939-942`
and `functions.rs:998-999` lift `export.source_decl` into a real
`Statement::VariableDecl` for codegen — so once the checker defines the binding,
the runtime value exists. J1 is a checker-only FP.

### Minimal fix (FP_fix_checker)
In `inference/items.rs`, the `infer_item` `Item::Export` arm: before/within the
match on `export.item`, when `export.source_decl` is `Some(decl)`, run the same
inference + define the `Item::VariableDecl` arm uses (items.rs:163-177):
```rust
Item::Export(export, _) => {
    // pub const/let/var NAME = expr : the VariableDecl rides in source_decl;
    // infer + bind it exactly like a bare Item::VariableDecl so NAME is in scope.
    if let Some(decl) = &export.source_decl {
        let var_type = self.infer_variable_decl(decl)?;
        self.record_unannotated_let_origin(decl);
        if let Some(name) = decl.pattern.as_identifier() {
            types.insert(name.to_string(), var_type.clone());
        } else {
            for name in decl.pattern.get_identifiers() {
                let scheme = self.env.lookup(&name).cloned();
                let inferred = scheme
                    .map(|s| s.instantiate(&mut self.type_var_gen))
                    .unwrap_or_else(|| var_type.clone());
                types.insert(name, inferred);
            }
        }
    }
    match &export.item {
        /* existing Function / TypeAlias / Struct / Trait arms unchanged */
        _ => {}
    }
}
```
(Equivalent: factor the VariableDecl body of items.rs:163-177 into a helper and
call it from both arms.) `infer_variable_decl` itself defines the binding in
`self.env`; the `types` insert mirrors the bare-decl arm so the binding's type is
exported in the program type map.

### Files touched
`crates/shape-runtime/src/type_system/inference/items.rs` (only).

---

## J2 — for-loop destructuring pattern bindings dropped by the type checker

### Reproduces (verbatim, strict-flip binary)
Program (`loops::cf_05_for_destructuring`):
```shape
let points = [{x: 1, y: 2}, {x: 3, y: 4}]
for {x, y} in points {
  print(f"({x}, {y})")
}
```
```
Error: Runtime error: Bytecode compilation failed: Semantic error: Undefined variable: 'x'
```
Isolation:
- `for [a, b] in pairs { ... }` → SAME rejection (`Undefined variable: 'a'`) — affects array AND object destructure.
- `for n in nums { print(n) }` → WORKS (simple identifier).
- `let {x, y} = p; ...` (object destructure in a `let`, not a `for`) → WORKS.
So the seam is the for-loop pattern-binding step, NOT the destructuring machinery.

### Exact seam
`crates/shape-runtime/src/type_system/inference/expressions.rs:728-744`
— `infer_expr`'s `Expr::For(for_expr, _)` arm (a top-level for-loop is an
`Expr::For` whose `pattern` is `ast::Pattern`, not `DestructurePattern`).

Line 735:
```rust
if let Some(name) = for_expr.pattern.as_simple_name() {
    self.env.define(name, TypeScheme::mono(element_type));
}
```
`Pattern::as_simple_name()` (`crates/shape-ast/src/ast/patterns.rs:113-119`)
returns `Some` only for `Identifier` / `Typed`. For `Pattern::Object(...)` /
`Pattern::Array(...)` it returns `None`, so **no bindings are defined at all** —
neither `x` nor `y` (nor `a`/`b`). The body's reference to `x` then fails at
`expressions.rs:78` → `UndefinedVariable`.

The VM side is already correct: `crates/shape-vm/src/compiler/loops.rs:710-826`
fully implements for-loop object-destructure (`Pattern::Object`, codegen 805-823)
and array-destructure (`Pattern::Array`, codegen 825-836). J2 is a checker-only FP.

Note: `ast::Pattern` (the match/for pattern enum) has NO bound-identifier
collector — only `as_simple_name()`. `DestructurePattern` (used by `let`) has
`get_identifiers()`/`get_bindings()` (`patterns.rs:179-206`), which is why the
`let` destructure works and the `for` destructure does not.

### Minimal fix (FP_fix_checker)
In `inference/expressions.rs`, the `Expr::For` arm: replace the single
`as_simple_name` define with a walk over all identifiers the pattern binds, each
defined with `element_type`. Minimal local helper (recurse over
`Pattern::Identifier`/`Typed`/`Object`/`Array`):
```rust
// Bind every identifier the loop pattern introduces (simple, object {x,y},
// or array [a,b]) — not just the simple-identifier case.
fn collect_pattern_names(p: &shape_ast::ast::Pattern, out: &mut Vec<String>) {
    use shape_ast::ast::Pattern::*;
    match p {
        Identifier(n) => out.push(n.clone()),
        Typed { name, .. } => out.push(name.clone()),
        Object(fields) => for (_k, sub) in fields { collect_pattern_names(sub, out) },
        Array(items)    => for sub in items     { collect_pattern_names(sub, out) },
        Constructor { .. } | Literal(_) | Wildcard => {}
    }
}
let mut names = Vec::new();
collect_pattern_names(&for_expr.pattern, &mut names);
for name in names {
    self.env.define(&name, TypeScheme::mono(element_type.clone()));
}
```
(Cleaner: add a `Pattern::get_identifiers()` method to
`crates/shape-ast/src/ast/patterns.rs` mirroring `DestructurePattern`'s, and call
it here. If that route is taken, the fix touches `patterns.rs` as well — see
files-touched note. The element-type granularity matches the existing
`as_simple_name` behavior, which bound the WHOLE element type; object/array
destructure precision is not required to clear the test, only the bindings'
existence.)

### Files touched
`crates/shape-runtime/src/type_system/inference/expressions.rs` (primary).
(If implemented via a shared `Pattern::get_identifiers()` helper, also
`crates/shape-ast/src/ast/patterns.rs`.)

---

## J3 — unannotated param over-constrained to Numeric by overloaded `+`

### Reproduces (verbatim, strict-flip binary)
Program (`multi_function::test_complex_string_pad_left`):
```shape
fn pad_left(s, total_len, pad_char) {
    let mut result = s
    while result.length < total_len {
        result = pad_char + result
    }
    result
}
pad_left("42", 5, "0")
```
```
Error: Runtime error: Bytecode compilation failed: Semantic error: Type constraint violation: parameter at position 0 of 'pad_left' must be numeric (its body requires a Numeric operand), but a call site passes the non-numeric type 'string'
```
Minimal isolation (the `result.length < total_len` compare is a red herring —
isolation test A shows it triggers only a separate JIT V2-verifier fallback, NOT
the over-constraint). The over-constraint reduces to `+`:
```shape
fn f(s, pad) { let mut r = s; r = pad + r; r }
f("42", "0")           // FP: rejected "param 0 ... must be numeric ... passes 'string'"
```
Scoping (what the fix MUST preserve vs MUST flip):
| case | program | strict-flip now | correct |
|---|---|---|---|
| D | `fn f(c){c+1}; f("hello")` | reject | reject (TP — `+1` forces numeric) — **keep** |
| E | `fn f(a,b){a+b}; f(3,4)` | accept | accept — keep |
| F | `fn f(a,b){a+b}; f("x","y")` | **reject (FP)** | accept — **flip** |
| G | `fn f(a,b){a*b}; f("x","y")` | reject | reject (TP — `*` is numeric-only) — **keep** |
| H | `fn f(a){a+"_suffix"}; f("base")` | accept | accept — keep (concrete-string guard already fires) |

### Exact seam
`crates/shape-runtime/src/type_system/inference/operators.rs:357-371`
— `infer_binary_op`'s `BinaryOp::Add` arm.

```rust
BinaryOp::Add => {
    if let Some(merged) = Self::infer_object_add_type(left, right) { return Ok(merged); }
    // String concatenation guard — fires only on CONCRETE string operands:
    if Self::is_string_like(left) || Self::is_string_like(right) { return Ok(BuiltinTypes::string()); }
    if let Some(rt) = self.check_operator_trait(left, "Add") { return Ok(rt); }
    self.infer_numeric_arithmetic_op(left, right, span)   // <-- pushes hard Numeric bound
}
```
`is_string_like` (operators.rs:189-205) matches only CONCRETE `string` (or
`Option<string>` / string-bearing union). In `r = pad + r`, both `pad` and `r`
are still unresolved unannotated-param `Type::Variable`s at body-inference time,
so the guard is skipped and it falls into `infer_numeric_arithmetic_op`
(operators.rs:294-344), which pushes `ImplementsTrait{ "Numeric" }` constraints
on both operands (lines 314-334). That records param 0 in
`callable_numeric_param_indices` via
`refine_callable_param_types_from_local_constraints` (inference/mod.rs:578-586,
588-590). Then callsite-union propagation resolves param 0 to `string`, and
`refine_numeric_params_post_callsite` (inference/mod.rs:1672-1732) hits the
`concrete` arm (1697) where `type_satisfies_numeric_bound` is false (1698) and
emits the `ConstraintViolation` (1701-1708) — the verbatim error.

`+` is overloaded (numeric add OR string concat); committing to Numeric when BOTH
operands are unresolved is the defect. `-`/`*`/`/`/`%` are numeric-only and must
keep forcing Numeric (case G stays rejected), so the fix is `Add`-only.

### Minimal fix (FP_fix_checker)
In `inference/operators.rs`, the `BinaryOp::Add` arm: when **both** operands are
unresolved type variables (i.e. neither `is_string_like` NOR a concrete numeric —
no concrete operand on either side to disambiguate), do NOT push the hard
`Numeric` constraint. Defer to callsite resolution and yield a fresh result var
(or the left operand var) instead of routing through
`infer_numeric_arithmetic_op`. Concretely, add a pre-check before line 370:
```rust
// `+` is overloaded (numeric add OR string concat). When BOTH operands are
// still unresolved type variables there is nothing to disambiguate at body
// time — committing to a Numeric bound here is the J3 over-constraint (it
// later rejects a string call site). Defer: yield a fresh result var and let
// callsite-union propagation pin the operands. A CONCRETE numeric on either
// side (e.g. `c + 1`) still flows to infer_numeric_arithmetic_op below and
// keeps the genuine Numeric requirement (case D / G unaffected — G is `*`).
if Self::is_unresolved_var(left) && Self::is_unresolved_var(right) {
    // result aliases the left operand var so callsite resolution unifies it.
    return Ok(left.clone());
}
self.infer_numeric_arithmetic_op(left, right, span)
```
where `is_unresolved_var(ty)` is `matches!(ty, Type::Variable(_) | Type::Constrained { .. })`
(both-vars guard; a single concrete operand still disambiguates and keeps current
behavior). This is the narrowest correct edit: it touches only the
ambiguous-Add path, leaves `-`/`*`/`/`/`%` (G), concrete-numeric `+` (D), and
concrete-string `+` (E/H) untouched, and lets F/pad_left resolve to `string`
via the existing callsite-union machinery.

Alternative considered & rejected (too broad / wrong layer): suppressing the
`errors.push` in `refine_numeric_params_post_callsite` when the resolution is
`string` — this would silently swallow the genuine TP rejection of case D-style
bodies and loses the `+`-vs-`*` distinction.

### Files touched
`crates/shape-runtime/src/type_system/inference/operators.rs` (only).

---

## Classification summary

| sub | verdict | seam (file:line) | files touched |
|---|---|---|---|
| J1 | FP_fix_checker | `type_system/inference/items.rs:243-281` (Export arm ignores `source_decl`) | `inference/items.rs` |
| J2 | FP_fix_checker | `type_system/inference/expressions.rs:735` (`Expr::For` only binds `as_simple_name`) | `inference/expressions.rs` (+`shape-ast/src/ast/patterns.rs` if helper route) |
| J3 | FP_fix_checker | `type_system/inference/operators.rs:357-371` (`Add` forces Numeric on two unresolved vars) | `inference/operators.rs` |

All three confirmed reproducing verbatim on strict-flip `@f01e8323`. No TP, no
needs_ruling, no cannot_reproduce among J1/J2/J3. (ROOT J's 4th member
`trig::pi_constant_approximation` is out of this task's scope — it is the
untyped-stdlib-fn family, same as ROOT A.)
