# Feasibility Area 1 — `to_annotation()` lowering completeness

Verdict: **GAP** (foundational, not blocked-but-painful).

The compile-cache RESULTS-IDENTICAL binder assumes a resolved prelude signature can be
serialized at the `TypeAnnotation` level via `Type::to_annotation()` and read back losslessly.
That assumption fails for the polymorphic surface of the prelude (generic stdlib fns, the
operator/iterator trait family, and every generic method table entry). Two independent losses:

1. `to_annotation()` itself drops `Type::Variable` and `Type::Constrained` (and `Generic`
   with a non-`Reference` base), substituting the string `"unknown"` inside `Function` params/returns.
2. The carriers that hold the polymorphic information the type-checker actually needs
   (`TypeScheme.quantified` / `.trait_bounds` / `.default_types`; `MethodTable` /
   `GenericMethodSignature` / `TypeParamExpr`) are **not** `TypeAnnotation` and have **no**
   `to_annotation()` lowering at all. `TypeParamExpr` is structurally inexpressible in `TypeAnnotation`.

Caveat narrowing the gap: struct/type/enum/trait/method **definitions** are stored as the raw
serde AST nodes (`StructTypeDef`, `EnumDef`, `TraitDef`, `MethodDef`), not as inferred `Type`.
Those round-trip losslessly already — they never touch `to_annotation()`. The gap is specifically
about *signatures* derived from inference `Type`.

---

## (a) Every `Type` variant `to_annotation()` can encounter

`Type` (`crates/shape-runtime/src/type_system/types/core.rs:92-112`) has exactly 5 variants;
`to_annotation()` (`core.rs:283-322`) has one arm each:

| `Type` variant | `to_annotation()` behavior (`core.rs`) | Lossless? |
|----------------|----------------------------------------|-----------|
| `Concrete(TypeAnnotation)` | `Some(ann.clone())` — verbatim (`core.rs:285`) | YES |
| `Variable(TypeVar)` | `None` (`core.rs:286`) | NO — total loss |
| `Generic { base, args }` | if `base == Concrete(Reference(name))`: `Generic{name, args.map(to_annotation)}`; else `None` (`core.rs:287-300`) | partial |
| `Constrained { var, constraint }` | `None` (`core.rs:301`) | NO — total loss |
| `Function { params, returns }` | each param/return lowered; **unresolvable → `Basic("unknown")`** (`core.rs:302-320`) | NO when any param/return is lossy |

Note: `Type` has **no** `Tuple`/`Union`/`Object`/`Dyn`/`Array`/`Never`/`Void` variant. Those AST
shapes live only inside `Concrete(TypeAnnotation::…)` and round-trip verbatim through the
`Concrete` arm. The 14-variant `TypeAnnotation` (`crates/shape-ast/src/ast/types.rs:9-47`) is
fully serde (`#[derive(... Serialize, Deserialize)]`, `types.rs:9`); the inference `Type` and
`TypeScheme` are **not** serde (`core.rs:92`, `core.rs:115` derive only `Debug, Clone[, PartialEq]`).
So lowering to `TypeAnnotation` is mandatory — there is no shortcut of serializing `Type` directly.

## (b) Exactly what is lost, and where

### b1. TypeVars (the dominant loss)
- `Variable` → `None` (`core.rs:286`). Inside a `Function`, an unresolvable param becomes
  `FunctionParam{ type_annotation: Basic("unknown") }` and an unresolvable return becomes
  `Basic("unknown")` (`core.rs:308-310`, `core.rs:313-315`).
- `BuiltinTypes::function()` (`builtins.rs:141-146`) deliberately keeps `Type::Variable` in
  `Type::Function.params/returns` (unlike `Concrete(TypeAnnotation::Function)`, which forces
  concrete `FunctionParam`s). Regression test `test_function_type_preserves_variables`
  (`constraints.rs:1316-1330`) pins this. So a function-typed signature that mentions a free
  type variable is *designed* to hold a `Variable` that `to_annotation()` cannot represent.

### b2. Trait bounds / quantified params / default args — not in `Type` at all
- A polymorphic signature is a `TypeScheme` (`core.rs:115-125`): `quantified: Vec<TypeVar>`,
  `trait_bounds: HashMap<String, Vec<String>>`, `default_types: HashMap<String, Type>` — held
  **beside** the `Type`, not inside it. `to_annotation()` operates on `Type` only and emits no
  carrier for any of these. `TypeAnnotation::Function` has no quantifier list and no bounds map,
  so even a hand-written lowering has nowhere to put them.
- Generic-param trait bounds (`<U: Comparable>`) and defaults (`T = int`) are collected into
  `TypeScheme.trait_bounds`/`default_types` in `make_function_scheme()`
  (`inference/items.rs:1107-1177`, esp. `:1140`, `:1142-1147`) — separate from the `Type`.

### b3. `Constrained` → `None` (`core.rs:301`)
- `Constrained{var, constraint: ImplementsTrait{..}}` (constructed e.g. `core.rs:204-213`) carries
  a trait obligation on a variable; `to_annotation()` drops it entirely.

### b4. `Generic` with a non-`Reference` base → `None` (`core.rs:297-299`)
- e.g. `Generic{ base: Variable(T), args: [...] }` (a higher-kinded/var-headed application) lowers
  to `None`. Resolved prelude `Generic`s usually have a `Concrete(Reference(name))` base
  (`Option`/`Result`/`Vec`, built that way at `environment/mod.rs:937-942`, `:948-952`, `:971-979`),
  so this arm is mostly safe — but it is still a latent total-loss arm.

### b5. Comptime fields / field annotations (`@range`, `@description`, `@alias`) — NOT lost
- These live on `StructField` (`crates/shape-ast/src/ast/types.rs:674-685`: `is_comptime`,
  `annotations: Vec<Annotation>`, `default_value`) and are stored as the raw serde
  `StructTypeDef` in `struct_type_defs` (`inference/mod.rs:123`, populated `inference/items.rs:92`,
  `:177-194`). They never round-trip through `to_annotation()`, so they survive **iff** the cache
  serializes the AST defs rather than reconstructing them from `Type`.
- Caveat: `resolve_type_annotation`'s `Object` arm *does* re-emit fields with
  `annotations: vec![]` (`inference/items.rs:491-504`) and an `"unknown"` fallback on each field
  type — so any path that turns a struct into a `Concrete(Object(...))` and back through
  inference strips field annotations. The cache must read defs from the AST side to avoid this.

## (c) Do RESOLVED prelude sigs actually contain the lossy shapes? — YES, pervasively

The prelude (`crates/shape-runtime/stdlib-src/core/prelude.shape`) auto-imports the entire
operator/iterator/conversion trait family plus the generic method modules (`use std::core::vec`,
`option_methods`, `result_methods`, `hashmap_methods`, `table_methods`, …). The resolved interface
of that surface is saturated with the lossy shapes:

1. **Rust-registered builtins store `Type::Variable` directly** (`environment/mod.rs:888-996`):
   - `print: <T>(Variable(T)) -> void` (`:891-899`)
   - `len: <T>(Variable(T)) -> int` (`:901-910`)
   - `fold: <T,U>(Array<Variable(T)>, Variable(U), Function([Variable(U),Variable(T)] -> Variable(U))) -> Variable(U)` (`:912-925`)
   - `Some/Ok/Err` — `Type::Generic{Reference("Option"/"Result"), [Variable(T), …]}` plus a
     `default_types` entry for `E = AnyError` (`:935-988`). `to_annotation()` of the inner
     `Variable(T)` args → `None` ⇒ the whole `Generic` arm returns `None` (`core.rs:290-296`,
     because `arg_annotations` is `None`). Total loss, plus the `E` default is in `default_types`.
   These are exactly the `Function`-with-`Variable` and `Generic`-with-`Variable`-args shapes from (b1)/(b4).

2. **Shape-source generic exports resolve their type params to `Type::Variable`.**
   `infer_function` pushes each `<T>`/`<U>` into scope as `TypeScheme::mono(Type::Variable(var))`
   (`inference/items.rs:294-305`), and `resolve_type_annotation` returns that `Variable` for any
   reference to the param name (`inference/items.rs:463-469`). The scheme registered for the
   function (`inference/items.rs:122-126`, `:211-214` for exports) therefore holds
   `Type::Function{ params: […Variable…], returns: …Variable… }`. Any exported `fn f<T>(x: T) -> T`
   round-trips to `(unknown) -> unknown`.
   - (A divergent path: `predeclare_function_signature` at `inference/items.rs:38-65` does *not*
     push type params into scope first, so a bare `Reference("U")` there resolves to
     `Concrete(Reference("U"))` (`inference/items.rs:474`) which *would* round-trip — but the
     **final registered scheme** comes from `infer_function`, the `Variable` form. So the
     authoritative resolved sig is the lossy one.)

3. **Generic method tables are not `TypeAnnotation` at all.** Vec/Option/Result/HashMap methods
   are stored as `GenericMethodSignature`/`TypeParamExpr` keyed by `(receiver_type_name, method_name)`
   (`checking/method_table.rs:78-93`, `:117-123`; registered for `extend Vec<T>` at
   `inference/items.rs:807-852+`). Concrete method overloads are `MethodSignature{ param_types: Vec<Type>, return_type: Type }` (`method_table.rs:96-106`). Examples that must survive:
   `extend Vec<T> { method map<U>(f: (T) => U) -> Vec<U> }`, `reduce<U>(f: (U,T)=>U, init: U) -> U`,
   `filter(predicate: (T) => bool) -> Vec<T>` (`stdlib-src/core/vec.shape:46-64`), and the
   `trait Iterable<T>` member sigs (`stdlib-src/core/iterable.shape:24-48`).
   - `TypeParamExpr` (`method_table.rs:51-74`) has variants `ReceiverParam(i)`, `MethodParam(i)`,
     `SelfType`, `Function{…}`, `GenericContainer{…}`, `Concrete(Type)`. There is **no**
     `TypeParamExpr::to_annotation()` and **no** `TypeAnnotation` variant that can carry a
     positional receiver/method type-param reference. It is resolved into `Type` *at each call site*
     by `resolve_type_param_expr` allocating fresh vars (`method_table.rs:288-306`). It cannot be
     lowered to the annotation layer at all — only re-derived from the AST `MethodDef`+`extend`/`impl`
     headers (which are serde and *do* round-trip).
   - None of `MethodTable`/`MethodSignature`/`GenericMethodSignature`/`TypeParamExpr` derive
     `Serialize`/`Deserialize` (`method_table.rs:51`, `:78`, `:96`, `:118` derive only
     `Debug, Clone`).

4. **`Queryable<T>`** — known constraint (CLAUDE.md): parses but inference erases type args back to
   simple names; shipped stdlib uses concrete `impl Queryable for Table`. So its *resolved* form is
   concrete and would round-trip, but only because the generic form is already not preserved — not a
   counterexample to the gap.

## Consequence for the RESULTS-IDENTICAL binder

A round-trip through `to_annotation()` cannot reproduce the resolved prelude interface for the
polymorphic surface: it would turn `map<U>(f: (T)=>U) -> Vec<U>` into `map(f: (unknown)=>unknown) -> Vec<unknown>`,
drop every `<T: Bound>` and `E = AnyError` default, and has no representation for the
`TypeParamExpr` method-resolution trees at all. A type-check reading those back would lose
bidirectional closure inference (`arr.map(|x| …)` infers `x` from the element type via the
generic method signature — CLAUDE.md Type System Rules), trait-bound checking, and default-arg
resolution. That is observable divergence, not a benign re-encoding.

## What a clean cache would have to serialize instead (out of scope to design here, but implied)

Serialize the **raw serde AST defs** that already round-trip — `FunctionDef` (params/return as
`TypeAnnotation` *before* the `infer_function` var-substitution), `StructTypeDef`, `EnumDef`,
`TraitDef`, `MethodDef`, `ExtendStatement`, `ImplBlock` — i.e. cache the pre-inference *interface
AST*, and re-run the (cheap, signature-only) predeclaration registration on load, rather than
lowering the post-inference `Type`/`TypeScheme`/`MethodTable` through `to_annotation()`. That side
is fully serde (`crates/shape-ast/src/ast/types.rs`) and carries bounds, defaults, comptime fields,
and field annotations natively. The `to_annotation()` path is the wrong serialization seam for the
resolved interface.

## File:line index of every load-bearing claim

- `Type` enum (5 variants): `crates/shape-runtime/src/type_system/types/core.rs:92-112`
- `to_annotation()`: `core.rs:283-322` (Variable→None `:286`; Generic non-Ref→None `:297-299`;
  Constrained→None `:301`; Function "unknown" fallback `:308-310`, `:313-315`)
- `Type`/`TypeScheme` derive (no serde): `core.rs:92`, `core.rs:115`
- `TypeScheme` quantified/trait_bounds/default_types: `core.rs:115-125`
- `BuiltinTypes::function` keeps Variable: `crates/shape-runtime/src/type_system/types/builtins.rs:141-146`
- regression pin (Function preserves vars): `crates/shape-runtime/src/type_system/constraints.rs:1316-1330`
- `TypeAnnotation` 14 variants + serde: `crates/shape-ast/src/ast/types.rs:9-47` (`Serialize`/`Deserialize` `:9`)
- builtin `print/len/fold/Some/Ok/Err` store Variable/Generic+Variable+defaults: `crates/shape-runtime/src/type_system/environment/mod.rs:888-996`
- generic param → Variable in scope: `crates/shape-runtime/src/type_system/inference/items.rs:294-305`, ref resolves to Variable `:463-469`
- function scheme registered from infer_function (lossy form): `inference/items.rs:122-126`, `:211-214`
- make_function_scheme bounds/defaults beside Type: `inference/items.rs:1107-1177`
- predeclare (divergent, non-authoritative) path: `inference/items.rs:38-65`
- resolve_type_annotation Object/Tuple/Union "unknown" fallback + annotations dropped: `inference/items.rs:480-540` (esp. `:491-504`)
- MethodTable/MethodSignature/GenericMethodSignature/TypeParamExpr (inference Type, no serde, no to_annotation): `crates/shape-runtime/src/type_system/checking/method_table.rs:51-128`
- TypeParamExpr resolved at call site via fresh vars: `method_table.rs:288-306`
- generic extend method registration: `inference/items.rs:807-852+`
- struct defs stored as raw AST (annotations/comptime/defaults survive): `inference/mod.rs:123`, `inference/items.rs:92`, `:177-194`; `StructField` shape `crates/shape-ast/src/ast/types.rs:674-685`
- concrete Vec/Iterable generic sigs in source: `crates/shape-runtime/stdlib-src/core/vec.shape:46-64`, `iterable.shape:24-48`
- prelude composition: `crates/shape-runtime/stdlib-src/core/prelude.shape`
