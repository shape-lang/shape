# Feasibility Area 3 — resolved-interface data model

Scope: what exactly to serialize for a `resolved_interface` so the type-checker
can check user code against a cached prelude/package interface WITHOUT
re-inferring it, expressed at the annotation level
(`crates/shape-ast/src/ast/types.rs`).

All file:line citations are against workspace HEAD (read-only).

---

## (a) Where the resolved interface lives in memory after a full prelude compile

There is **no single "compiled interface" struct** today. The interface is
spread across three derived, in-memory, non-serializable structures, all owned
by `TypeInferenceEngine`, plus a binary-static method dispatch table. The
**source-of-truth in every case is the AST item** (`FunctionDef`,
`StructTypeDef`, `MethodDef`, `ExtendStatement`, `ImplBlock`, `TraitDef`,
`EnumDef`), which derives serde; the in-memory structures below are *rebuilt*
from those AST items during inference.

### Owning struct: `TypeInferenceEngine`
`crates/shape-runtime/src/type_system/inference/mod.rs:56`

Relevant fields:
- `pub env: TypeEnvironment` — `mod.rs:58`
- `pub(crate) method_table: MethodTable` — `mod.rs:86`
- `pub(crate) struct_type_defs: HashMap<String, StructTypeDef>` — `mod.rs:123`
- `pub(crate) callable_param_defaults: HashMap<String, Vec<bool>>` — `mod.rs:95`
  (per-fn "param has default" flags, needed for callsite arity validation)
- `pub callsite_type_args: HashMap<(String,usize,usize), Vec<(String,TypeAnnotation)>>` — `mod.rs:131` (monomorphization driver; callsite-local, NOT interface)

### Fn / module-binding signatures: `TypeEnvironment`
`crates/shape-runtime/src/type_system/environment/mod.rs:31`
- `scopes: Vec<HashMap<String, TypeScheme>>` — `mod.rs:33` (root scope holds
  user/prelude fn signatures, written by `env.define(&func.name, scheme)` at
  `inference/items.rs:63` / `:125` / `:213`)
- `builtins: HashMap<String, TypeScheme>` — `mod.rs:35`
- `type_registry: TypeRegistry` — `mod.rs:37`

`TypeScheme` is the resolved fn signature carrier:
`crates/shape-runtime/src/type_system/types/core.rs:116`
- `quantified: Vec<TypeVar>` (generics)
- `ty: Type` (the function type — `Type::Function { params, returns }`)
- `trait_bounds: HashMap<String, Vec<String>>` (var → trait names)
- `default_types: HashMap<String, Type>` (generic param defaults)

`TypeScheme` is **built from the AST** in
`predeclare_function_signature` (`inference/items.rs:38`) +
`make_function_scheme` (`inference/items.rs:1107`): name from `func.name`,
params from `func.params[].type_annotation`, return from `func.return_type`,
generics + bounds + defaults from `func.type_params` and `func.where_clause`.

### Struct/enum/alias/trait/trait-impl defs: `TypeRegistry`
`crates/shape-runtime/src/type_system/environment/registry.rs:105`
- `type_aliases: HashMap<String, TypeAliasEntry>` — `registry.rs:107`
- `traits: HashMap<String, TraitDef>` — `registry.rs:109` (TraitDef is the AST type, serde-derived)
- `trait_impls: HashMap<String, TraitImplEntry>` — `registry.rs:111`, key `"Trait::Target::ImplName"` (`registry.rs:121`)
- `blanket_impls: HashMap<String, Vec<BlanketImplEntry>>` — `registry.rs:113`
- `enum_defs: HashMap<String, EnumDef>` — `registry.rs:115` (EnumDef is the AST type, serde-derived)
- `record_schemas: HashMap<String, RecordSchema>` — `registry.rs:117`

Struct field types live additionally in `struct_type_defs` (above), populated by
`predeclare_struct_type` (`inference/items.rs:88-93`) which stores the AST
`StructTypeDef` verbatim.

### Method table (compile-time method-call resolution): `MethodTable`
`crates/shape-runtime/src/type_system/checking/method_table.rs:119`
- `methods: HashMap<(String,String), Vec<MethodSignature>>` — `method_table.rs:121`
  keyed by `(receiver_type_name, method_name)`
- `generic_methods: HashMap<(String,String), GenericMethodSignature>` — `method_table.rs:123`
- `comptime_methods: HashSet<(String,String)>` — `method_table.rs:127`

`MethodSignature` (`method_table.rs:97`): `name`, `param_types: Vec<Type>`,
`return_type: Type`, `is_fallible: bool`.
`GenericMethodSignature` (`method_table.rs:79`): `name`, `method_type_params:
usize`, `param_types: Vec<TypeParamExpr>`, `return_type: TypeParamExpr`,
`is_fallible`, `receiver_param_bounds: Vec<(usize, Vec<String>)>`.
`TypeParamExpr` (`method_table.rs:52`): `Concrete(Type)` | `ReceiverParam(usize)`
| `MethodParam(usize)` | `Function{..}` | `GenericContainer{name,args}` | `SelfType`.

The table is **built from the AST** in `register_extend`
(`inference/items.rs:807`) and the impl-block path — each `MethodDef`'s
`params[].type_annotation` and `return_type` (both `TypeAnnotation`) are mapped
to `Type` / `TypeParamExpr` via `resolve_type_annotation` (`items.rs:894/899`)
and `annotation_to_type_param_expr` (`items.rs:925`).

### Builtin method *implementations*: PHF static maps (NOT data, not serialized)
`crates/shape-vm/src/executor/objects/method_registry.rs`
- `ARRAY_METHODS`, `HASHMAP_METHODS`, `STRING_METHODS`, etc. —
  `phf::Map<&'static str, MethodHandler>` (e.g. `method_registry.rs:260`,
  `:445`, `:899`). `MethodFnV2 = fn(...)` — `method_registry.rs:48`.

These are compile-time-static Rust function pointers baked into the binary. They
are identical across any process running the same binary, so they require **no
serialization** — the cache and the running compiler share the same `&'static`
table. Only the *type-checker's view* of method signatures
(`MethodTable`, above) is interface state.

---

## (b) Exact serializable data model for `resolved_interface`

Because every in-memory carrier (`TypeScheme`, `Type`, `MethodTable`,
`MethodSignature`, `GenericMethodSignature`, `TypeParamExpr`) is a *derived*
form rebuilt cheaply from serde-capable AST items, the smallest sound
serializable model is **the resolved AST items themselves, all at the
`TypeAnnotation` level**. Two layout options:

### Option A — annotation-level resolved model (recommended; lossless for annotated code)

```
ResolvedInterface {
    // fn / module-export signatures
    functions: Vec<FunctionDef>,          // shape-ast functions.rs:13 (serde ✓)
    foreign_functions: Vec<ForeignFunctionDef>, // functions.rs:42 (serde ✓)

    // struct / type defs (field types + comptime fields + field annotations)
    structs: Vec<StructTypeDef>,          // ast/types.rs:638 (serde ✓)
    type_aliases: Vec<TypeAliasDef>,      // ast/types.rs:344 (serde ✓)
    enums: Vec<EnumDef>,                  // ast/types.rs:415 (serde ✓)

    // trait system
    traits: Vec<TraitDef>,                // ast/types.rs:507 (serde ✓)
    impls: Vec<ImplBlock>,                // ast/types.rs:539 (serde ✓)
    extends: Vec<ExtendStatement>,        // ast/types.rs:563 (serde ✓)

    // module exports surface (names + visibility)
    exports: Vec<(String, ExportVisibility)>,
}
```

Each field/annotation-level type breaks down to:

- **fn signature**: `FunctionDef` already carries `name` (`functions.rs:14`),
  `type_params: Option<Vec<TypeParam>>` (generics; `functions.rs:24`),
  `params: Vec<FunctionParameter>` (`functions.rs:25`, each with
  `type_annotation: Option<TypeAnnotation>` `functions.rs:172`),
  `return_type: Option<TypeAnnotation>` (`functions.rs:26`),
  `where_clause: Option<Vec<WherePredicate>>` (bounds; `functions.rs:27`).
  `TypeParam::Type` carries `trait_bounds: Vec<TypePath>` + `default_type`
  (`ast/types.rs:212/210`). `WherePredicate` = `{type_name, bounds:
  Vec<TypePath>}` (`ast/types.rs:298`).
  → params, ret, generics, bounds all expressible.

- **struct/type def**: `StructTypeDef.fields: Vec<StructField>`
  (`ast/types.rs:643`). `StructField` (`ast/types.rs:675`) carries
  `type_annotation: TypeAnnotation`, `is_comptime: bool` (comptime fields),
  `annotations: Vec<Annotation>` (field annotations `@description`/`@range`/
  `@example`/`@alias`), `default_value: Option<Expr>`, `type_params`.
  → field types + comptime fields + field annotations all expressible.

- **method table** keyed by receiver+method → signature: carried implicitly by
  `impls`/`extends`/`structs[].methods`. Each `MethodDef` (`ast/types.rs:571`)
  has `name`, `type_params`, `params: Vec<FunctionParameter>`, `return_type:
  Option<TypeAnnotation>`, `is_async`, `when_clause`, `annotations`,
  `declaring_module_path`. The cache loader replays the existing
  `register_extend` / impl-registration path (`inference/items.rs:807`) to
  rebuild `MethodTable` — exactly what a fresh compile does, but skipping body
  inference. Receiver-type key derives from `ExtendStatement.type_name` /
  `ImplBlock.target_type` (`ast/types.rs:565` / `:543`).

- **module exports**: name list + visibility. Runtime `ModuleExports`
  (`crates/shape-runtime/src/module_exports.rs:342`) is a live-function-pointer
  registry (`ModuleFnEntry::Typed` — `module_exports.rs:271`) and is NOT
  serializable; only the *names* + visibility (`ModuleExportVisibility`,
  `module_exports.rs:278`) belong in the interface. Signatures of exported fns
  are already in `functions`.

Loader cost: parse-free; runs the predeclare passes
(`predeclare_function_signature`, `predeclare_struct_type`, `register_extend`,
`define_trait`, `register_trait_impl`, `register_enum`) over the deserialized
items. No body inference, no `infer_function` calls. This rebuilds
`TypeEnvironment` + `TypeRegistry` + `MethodTable` + `struct_type_defs`
identically to a from-source prelude compile.

### Option B — pre-resolved `TypeScheme`/`MethodSignature` model (needs new serde types)

Serialize the *already-resolved* `Type`-level signatures directly, skipping even
the predeclare passes. This requires either:
1. adding `Serialize/Deserialize` to `Type`, `TypeScheme`, `TypeVar`,
   `TypeConstraint`, `MethodSignature`, `GenericMethodSignature`,
   `TypeParamExpr` (currently none derive serde — see (c)), OR
2. a parallel `#[derive(Serialize)]` annotation-level mirror that
   `Type::to_annotation()` (`core.rs:283`) projects into.

Option B is only worthwhile if the predeclare passes prove hot; Option A reuses
existing, tested registration code and is the lower-risk path.

---

## (c) Do the `TypeAnnotation` serde types already derive serde, and do they
capture what the checker needs?

**Serde derives: YES, complete on the annotation layer.**
- `TypeAnnotation` derives `Serialize, Deserialize` — `ast/types.rs:9`. Captures
  generics (`Generic{name,args}` `:30`), `Array(Box<_>)` `:14`, function types
  (`Function{params,returns}` `:20`), `Reference(TypePath)` `:35`, `Union`/
  `Intersection`/`Tuple`/`Object`/`Dyn`. `Option<T>`/`Result<T,E>`/
  `HashMap<K,V>`/`Array<T>` are all `Generic{name, args}` or `Array` (helpers
  `option`/`option_inner` `:50/:57`).
- `ObjectTypeField` `:171`, `FunctionParam` `:181`, `TypeParam` `:200`,
  `WherePredicate` `:297`, `TypeAliasDef` `:343`, `TraitMemberSignature` `:359`,
  `EnumDef`/`EnumMember`/`EnumMemberKind`/`EnumValue` `:415/:426/:436/:446`,
  `TraitMember` `:453`, `AssociatedTypeBinding` `:490`, `TraitDef` `:507`,
  `ImplBlock` `:538`, `ExtendStatement` `:562`, `MethodDef` `:570`,
  `TypeName` `:616`, `StructTypeDef` `:637`, `StructField` `:674`,
  `NativeLayoutBinding` `:655` — all `#[derive(... Serialize, Deserialize)]`.
- `FunctionDef` `functions.rs:12`, `ForeignFunctionDef` `:41`,
  `FunctionParameter` `:156`, `Annotation` `:201`, `NativeAbiBinding` `:70` —
  all serde-derived.
- `Program` (`ast/program.rs:22`), `Item` (`:30`), `Expr`
  (`ast/expressions.rs:33`), `Span` (`ast/span.rs:7`, also `Default`) — all
  serde-derived. The *entire AST is serializable*, so any subset of items is.

**Coverage of resolution needs: YES for annotated code.**
- generics/bounds: `TypeParam::Type.trait_bounds` (`:212`) + `default_type`
  (`:210`) + `WherePredicate.bounds` (`:300`).
- Option/Result/Array/HashMap: `TypeAnnotation::Generic` / `Array`.
- field access: `StructField.type_annotation` + `is_comptime` + `annotations`.
- method-call: `MethodDef`/`ExtendStatement`/`ImplBlock` params + return as
  `TypeAnnotation`, replayed through `register_extend`.

Stdlib discipline makes this lossless: stdlib fns/methods are fully annotated
(e.g. `crates/shape-runtime/stdlib-src/core/math.shape:183` `pub fn clamp<T:
Ord>(x: T, lo: T, hi: T) -> T`; `:195` `pub fn sign(x: int) -> int`). Foreign
functions are *required* to be fully annotated and validated
(`functions.rs:110 validate_type_annotations`). So no prelude/package interface
signature falls into the fresh-type-var hole described in (d).

---

## (d) Things the checker reads that are NOT expressible as `TypeAnnotation`
(non-annotation escape hatches)

The in-memory inference types are a **strict superset** of `TypeAnnotation`.
`Type::to_annotation()` (`core.rs:283`) is `-> Option<TypeAnnotation>` and
returns `None`/lossy for:

1. **`Type::Variable(TypeVar)`** — `core.rs:97`, `to_annotation` returns `None`
   at `core.rs:286`. Unannotated fn params / return types become
   `self.fresh_type_var()` (`inference/items.rs:52`, `:58`; method path `:860`,
   `:869`, `:895`, `:900`). For a fully-annotated prelude these never appear in
   a signature, but a *user package* with unannotated public fns would.
   `TypeVar` itself (`core.rs:46`) does **not** derive serde.

2. **`Type::Constrained { var, constraint }`** — `core.rs:101`, `to_annotation`
   returns `None` at `core.rs:301`. `TypeConstraint`
   (`type_system/types/constraints.rs:9`) does not derive serde. Carries
   `ImplementsTrait{trait_name}` etc. — a bound on an inference variable, not an
   annotation.

3. **`Type::Function` with unresolved params/returns** — `to_annotation`
   substitutes `TypeAnnotation::Basic("unknown")` for any `None`-projecting
   param/return (`core.rs:310`, `:315`). This is the documented `core.rs:218`
   `Type::to_annotation()` TypeVar-loss constraint (CLAUDE.md "Known
   Constraints"). Lossy, not a hard fail.

4. **The `\u{1}tyvar:` marker hack** — `TYVAR_ANNOTATION_PREFIX` (`core.rs:73`),
   `tyvar_to_annotation` (`core.rs:76`). A type variable smuggled inside
   `TypeAnnotation::Basic("\u{1}tyvar:T7")` for object-literal fields whose
   value is an unresolved param. This *is* serde-roundtrippable (it is a
   `Basic` string) but is semantically a non-annotation escape hatch: a marker
   surviving to the interface is an honestly-unresolved type that resolves to
   `unknown`. Should never appear in a fully-annotated prelude signature.

5. **`SemanticType`** (`type_system/semantic.rs`) — the user-facing projection
   produced by `Type::to_semantic` (`core.rs:328`). Has `SemanticType::TypeVar`
   too; same variable-loss class. Not needed for the interface if Option A
   (annotation-level) is used.

6. **`callsite_type_args`** (`inference/mod.rs:131`) and the various
   `callsite_param_types` / `callable_numeric_param_indices` /
   `pending_return_unions` maps (`mod.rs:88-115`) — these are
   *callsite-local inference scratch*, not interface; they are rebuilt per
   compile and should NOT be in the cache.

7. **`HoistedField`** (`environment/mod.rs:23`) — `field_type: Type` (no serde);
   optimistic property-hoisting scratch, not interface.

None of (1)–(3) force a non-annotation escape hatch *for a fully-annotated
prelude/package interface* — that is exactly the precondition stdlib already
satisfies. They DO force one for an interface that includes **unannotated public
function signatures** (a user package without return/param annotations). For
those, Option A would need either (i) a serde-capable inference-variable
encoding (the `\u{1}tyvar:` marker generalized, with `trait_bounds` carried
alongside), or (ii) a hard requirement that public/exported signatures be fully
annotated before caching (mirrors the existing `ForeignFunctionDef`
annotation-required rule, `functions.rs:110`).

---

## Verdict: GAP (bounded, with a clear path)

- **Clean for the prelude + fully-annotated packages.** The entire annotation
  layer (`TypeAnnotation` + all item types) derives serde and captures
  generics, trait bounds, Option/Result/Array<T>/HashMap<K,V>, field types,
  comptime fields, and field annotations. Option A (serialize resolved AST
  items, replay the predeclare/register passes) is lossless and reuses tested
  registration code (`inference/items.rs:38/88/807`).

- **Gaps:**
  1. There is no existing serializable "interface" struct — it must be defined
     (Option A above). The live in-memory carriers (`TypeScheme` `core.rs:116`,
     `Type` `core.rs:93`, `MethodTable` `method_table.rs:119`,
     `MethodSignature`/`GenericMethodSignature`/`TypeParamExpr`) **do not derive
     serde** and contain non-annotation `Type::Variable`/`Type::Constrained`.
  2. Unannotated public fn/method signatures resolve to `Type::Variable`
     (`inference/items.rs:52/58/895/900`) which `to_annotation()` cannot
     express (`core.rs:286`) — they degrade to `Basic("unknown")`
     (`core.rs:310/315`). Caching such an interface requires either a
     serde-capable type-variable+bounds encoding or a "public signatures must be
     annotated" precondition (precedent: `ForeignFunctionDef`
     `functions.rs:110`).
  3. `TypeConstraint` / `TypeVar` (`constraints.rs:9` / `core.rs:46`) have no
     serde; only relevant if Option B (pre-resolved `Type`-level model) is
     chosen instead of Option A.

- **Not blocked:** every checker read for an annotated interface is expressible
  as `TypeAnnotation`, and the rebuild path already exists.
