# A-final ROOT B — bare `HashMap()` / `HashMap().set()` cannot infer `<K,V>` under strict mode

**Verdict: `FP_fix_checker`** — valid code is over-rejected. The `HashMap()`
constructor is registered with a non-polymorphic bare-reference return type, so
the key/value types provided by `.set(k, v)` can never flow into `<K,V>`. The
fix is in the checker (constructor registration), NOT a test re-baseline.

Baseline: strict-flip @ `f01e83232933bac70b2103d5eed4706411ea9831`
(`shape-strict-flip-collection-dispatch`, let-gen already landed → ROOT A
cleared). Binary: `target/release/shape`.

---

## Failing tests (all 4 reproduce verbatim on the strict-flip binary)

| Test | Program | strict-flip rejection (verbatim, HOF/DEBUG filtered) |
|------|---------|------|
| `stress_creation::test_hashmap_created_in_function` | `fn make_map() { HashMap().set("x", 42) } make_map().get("x")` | `Generic type error: Could not infer generic type arguments for 'HashMap'` |
| `stress_operations::test_hashmap_returned_from_function` | `fn build() { HashMap().set("result", 42) } fn query(m) { m.get("result") } query(build())` | `Generic type error: Could not infer generic type arguments for 'HashMap'` |
| `stress_operations::test_hashmap_set_nested_hashmap_value` | `let inner = HashMap().set("nested", true); let outer = HashMap().set("child", inner); outer.get("child").get("nested")` | `Type constraint violation: Generic { base: Concrete(Reference("Option")), args: [Variable(TypeVar("_oob"))] } cannot have fields` |
| `stress_operations::test_hashmap_var_reassignment` | `let mut m = HashMap(); m = m.set("a", 1); m = m.set("b", 2); m = m.set("c", 3); m.len()` | `Could not solve type constraints: HashMap is not compatible with Generic { base: ..."HashMap"..., args: [Variable(TypeVar("_oob")), Variable(TypeVar("_oob"))] }` (×3) |

All four root in the same defect; the three different error surfaces are three
downstream consequences of one cause (see below).

---

## Root cause — the `HashMap()` constructor return type is a bare reference, not `HashMap<fresh, fresh>`

`crates/shape-runtime/src/type_system/environment/mod.rs:927-932`:

```rust
// HashMap constructor: HashMap() -> HashMap<any, any>   <-- comment LIES
self.define_builtin(
    "HashMap",
    vec![],
    Type::Concrete(TypeAnnotation::Reference("HashMap".into())),  // bare, no <K,V>, monomorphic
);
```

`define_builtin` (mod.rs:861) installs a **monomorphic** `TypeScheme::mono` with
a bare `Reference("HashMap")` return — NO type-parameter args, NO quantified
vars. Contrast the `Some`/`Ok`/`Err` constructors directly below it
(mod.rs:934-…), which use `define_polymorphic` / `poly_bounded_with_defaults`
with fresh quantified vars and return `Option<T>` / `Result<T,E>`. Those work
(control case `fn build() { Some(42) }` compiles clean); `HashMap()` does not.

### The dispatch chain that drops K,V on the floor

1. `HashMap()` is typed as bare `Type::Concrete(Reference("HashMap"))`.
2. `.set("x", 42)` resolves via `MethodTable`. `extract_receiver_info`
   (`crates/shape-runtime/src/type_system/checking/method_table.rs:721`):
   ```rust
   Type::Concrete(TypeAnnotation::Reference(name)) => (Some(name.to_string()), vec![]),
   ```
   A bare-`Reference` receiver yields **empty `receiver_params`**.
3. `set` is registered as `set(ReceiverParam(0), ReceiverParam(1)) -> HashMap<ReceiverParam(0), ReceiverParam(1)>`
   (method_table.rs:451-457). With empty `receiver_params`,
   `resolve_type_param_expr` (method_table.rs:653-656) falls to the
   out-of-bounds branch:
   ```rust
   let placeholder = || Type::Variable(TypeVar::new("_oob".to_string()));
   TypeParamExpr::ReceiverParam(idx) => receiver_params.get(*idx).cloned().unwrap_or_else(placeholder),
   ```
   So K → `_oob`, V → `_oob`. **The args `string`, `int` are never unified into
   K,V** — the receiver had no slots for them to flow into. `.set()` returns
   `HashMap<_oob, _oob>`.

That single `_oob` carrier produces all three error surfaces:
- **`test_hashmap_var_reassignment`**: `m: bare HashMap` reassigned to
  `HashMap<_oob,_oob>` → `HashMap` not compatible with `HashMap<_oob,_oob>`.
- **`test_hashmap_set_nested_hashmap_value`**: `.get("child")` on
  `HashMap<_oob,_oob>` → `Option<_oob>`; `.get("nested")` on `Option<_oob>` →
  "cannot have fields".
- **created/returned-from-function**: see the second seam below.

### Second (downstream) seam — function-boundary non-expansiveness gate

For `fn make_map() { HashMap().set("x",42) }` the rejection surfaces earlier, at
the let-gen function-return gate:

`crates/shape-runtime/src/type_system/inference/items.rs:561-562`:
```rust
let allow_unresolved_return =
    func.return_type.is_some() || Self::fn_body_is_non_expansive(func);
```
`fn_body_is_non_expansive` → `expr_is_nonexpansive` (items.rs:1731-1774): a
`MethodCall` is non-expansive iff its receiver is (items.rs:1761-1763); the
receiver `HashMap()` is an `Expr::FunctionCall` whose name is not
`Some`/`Ok`/`Err`, so it hits `_ => false` (items.rs:1773). Body is judged
EXPANSIVE → `allow_unresolved_return = false` → `combine_return_types` (strict)
→ `ensure_no_unresolved_generic_args` (inference/mod.rs:1017-1051) rejects with
"Could not infer generic type arguments for 'HashMap'".

**This second seam is NOT the right fix target.** Once the primary fix lands,
`.set("x",42)` resolves to `HashMap<string,int>` (fully concrete, no free vars),
so `ensure_no_unresolved_generic_args` finds nothing to reject — the gate is
moot. Widening `expr_is_nonexpansive` to bless `HashMap()` would be papering
over the constructor defect and risks the value-restriction soundness binder
(§3.2 / value-restriction, let-gen spec §5). Do not touch it.

---

## Why this is FP_fix_checker, not TP_rebaseline

The `project_generic_types_require_args` ruling rejects a bare generic name
*when the type is not inferable*. Here the type **is** inferable from usage:
`.set("x", 42)` supplies K=`string`, V=`int`. The classification note is exact —
"the type IS inferable from usage so it should infer, not reject." Proof the
inference machinery already works once K,V have slots to flow into:

- Control: `let m = HashMap().set("result", 42); m.get("result")` at **module
  scope COMPILES AND RUNS** on the strict-flip binary. Same `.set()` chain; the
  only difference is the absence of the function-return gate AND the module-scope
  path tolerating the residual `_oob`. The chain's K,V resolution is not the
  blocker — the constructor's missing `<K,V>` slots are.
- Control: `fn build() { Some(42) }` compiles clean — the polymorphic-constructor
  pattern this fix copies is already proven for `Some`/`Ok`/`Err`.
- Even the user-facing workaround `let mut m: HashMap<string,int> = HashMap()`
  FAILS today (`HashMap` not compatible with `HashMap<string,int>`), which means
  mutable HashMap locals are unusable under strict mode at all — unambiguously a
  checker bug, not intended strict semantics.

---

## Fix recipe (exact minimal edit — the seam)

**File:** `crates/shape-runtime/src/type_system/environment/mod.rs`
**Lines:** 927-932 (inside `define_builtin_functions`)

Replace the monomorphic bare-reference registration with a polymorphic one that
returns `HashMap<K, V>` over two fresh quantified vars — mirroring the `Some`
constructor 11 lines below it (mod.rs:934-943):

```rust
// HashMap constructor: HashMap() -> HashMap<K, V> (K,V inferred from first .set/usage)
let hm_k = TypeVar::new("K".to_string());
let hm_v = TypeVar::new("V".to_string());
let hashmap_result = Type::Generic {
    base: Box::new(Type::Concrete(TypeAnnotation::Reference("HashMap".into()))),
    args: vec![Type::Variable(hm_k.clone()), Type::Variable(hm_v.clone())],
};
self.define_polymorphic("HashMap", vec![hm_k, hm_v], vec![], hashmap_result);
```

Mechanism: each `HashMap()` callsite instantiates fresh K,V (the scheme is
quantified, so `instantiate` mints distinct vars per use — no cross-callsite
aliasing). `extract_receiver_info` on the resulting
`Type::Generic{base: HashMap, args: [K, V]}` returns `receiver_params = [K, V]`
(method_table.rs:704-712), so `.set("x", 42)`'s `ReceiverParam(0/1)` resolve to
K,V and the args `string`,`int` unify into them. `_oob` is never produced. The
`.set()` chain returns a concrete `HashMap<string,int>`; the function-return
gate finds no unresolved args and accepts.

**No second edit required.** Do NOT widen `expr_is_nonexpansive`
(items.rs:1761) or relax `ensure_no_unresolved_generic_args` — both become
no-ops once the constructor is polymorphic, and touching either risks the
let-gen value-restriction soundness binder.

### Scope check
`HashMap` is the only collection constructor registered via `define_builtin`
with a bare collection reference (grep of `define_builtin(` in mod.rs: the rest
are math/`exit`). `Map`/`Set`/`Deque`/`PriorityQueue` are type-annotation
aliases in `concrete_conv.rs`, not constructor builtins, so they need no change
for these four tests. The fix is precisely one constructor.

---

## Files the fix touches (for conflict-grouping)

- `crates/shape-runtime/src/type_system/environment/mod.rs` (sole edit; lines 927-932)

Adjacent / read-only (NOT edited — listed for conflict awareness because
sibling A-final roots that touch the let-gen gate or method_table dispatch may
collide):
- `crates/shape-runtime/src/type_system/checking/method_table.rs` (the `_oob`
  placeholder at :650 and `extract_receiver_info` at :702 are the *symptom*
  surface, not the fix)
- `crates/shape-runtime/src/type_system/inference/items.rs` (the
  `fn_body_is_non_expansive` gate at :561/:1761 — explicitly DO NOT touch)
- `crates/shape-runtime/src/type_system/inference/mod.rs`
  (`ensure_no_unresolved_generic_args` at :1017 — explicitly DO NOT touch)

---

## Clears (post-fix expectation)

- `stress_creation::test_hashmap_created_in_function`
- `stress_operations::test_hashmap_returned_from_function`
- `stress_operations::test_hashmap_set_nested_hashmap_value`
- `stress_operations::test_hashmap_var_reassignment`

All four reproduce on the strict-flip binary @ `f01e8323` today; after the
single polymorphic-constructor edit, `.set()` flows K,V concretely and every one
should pass. (Fix not applied — task is READ-ONLY diagnosis.)
