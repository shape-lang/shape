# Wave 41D: Current Comptime Surface Inventory

## Scope and labels

This is a static inventory of the current parser, AST, compiler, reserved
descriptor schemas, stdlib definitions, focused ShapeTests, and sibling book
pages. No build, test, extraction, or book-truth command was run for this
report.

Status labels are deliberately narrow:

- **CURRENT / VM**: current syntax with a checked-in successful VM ShapeTest.
- **CURRENT / VM+JIT**: checked-in tests cover both modes. This does not by
  itself mean the comptime evaluator ran natively in the JIT.
- **CURRENT / compiler**: parser and compiler application path exist, but no
  successful focused public test was found.
- **CURRENT / parse**: grammar/AST only, or the checked-in test records a
  failure instead of successful execution.
- **LEGACY CURRENT**: executes today but crosses source text, JSON, `Any`, or a
  string-keyed semantic boundary. It is not a strictly typed recommendation.
- **TARGET ONLY**: illustrative design notation; it is not accepted syntax.

“Checked-in test” below means static evidence in the repository. Tests were not
executed during Wave 41D.

## Verdict

Shape has one working comptime engine with several entry points: expression and
top-level blocks, comptime-only helpers, annotation `comptime pre/post` handlers,
comptime fields, limited comptime trait/impl context, reflection builtins, and a
directive stream. Function, struct-type, and module transforms are materially
implemented. Generated free functions and methods have VM/JIT coverage.

The current surface is not strictly typed end to end. `TypeRef` is a reserved
typed object but is reconstructed from `name`/`kind`/`source` strings. Descriptor
arrays still contain `Any` or `unknown`; annotation arguments are stringified;
direct AST directives serialize through JSON; computed bodies/modules and most
computed items reparse Shape source. `ItemFragment` is the sole typed additive
generation slice, limited to one zero-argument free function returning a scalar
literal. Evidence: `crates/shape-vm/src/compiler/comptime_target.rs:48-119`,
`crates/shape-runtime/src/type_schema/builtin_schemas.rs:241-329`, and
`crates/shape-vm/src/compiler/comptime_builtins.rs:461-653`.

## Strictly typed current examples

These are the recommended current examples because they use typed descriptors
or the typed fragment slice and never pass generated Shape source, JSON, or
`Any` values.

### Reflection through `TypeRef`

**CURRENT / VM** (the same reflection program also has a VM+JIT flagship):

```shape
type Profile { id: int, nickname: string? }

let reflected: string = comptime {
  let info = type_info(Profile)
  let field = info.fields[1]
  f"{info.type_ref.kind}|{field.name}|{field.type_ref.kind}|{field.optional}"
}
print(reflected)
```

Semantic types are `TypeInfo`, `Array<FieldDescriptor>`, and `TypeRef`. The
exact field/optional behavior is covered at
`tools/shape-test/tests/comptime/type_info_chained.rs:72-90`; the VM/JIT
reflection matrix is at `tools/shape-test/tests/comptime/flagship_wf3d.rs:68-94`.
Limitation: `TypeRef.kind` and `TypeRef.source` remain strings, and annotation
handler execution currently lacks a user-type reflection snapshot
(`comptime.rs:1270-1274`).

### Signature mutation through `TypeRef`

**CURRENT / VM**:

```shape
annotation return_like_input() {
  targets: [function]
  comptime post(target, ctx) {
    set return (target.params[0].type_ref)
  }
}

@return_like_input()
fn echo(value: string) { value }
```

The payload is a `TypeRef`; the resulting semantic operation is
`set_return(TypeRef)`. The corresponding successful test is
`tools/shape-test/tests/annotations_comptime/type_mutation.rs:291-310`.
Limitation: the compiler reparses `TypeRef.source` into a `TypeAnnotation`.

### Additive generation through `ItemFragment`

**CURRENT / VM**:

```shape
annotation typed_label() {
  targets: [type]
  comptime post(target, ctx) {
    extend (item_fn(
      f"{target.name}_label",
      type_info(string).type_ref,
      "typed fragment"
    ))
  }
}

@typed_label()
type Widget { id: int }
print(Widget_label())
```

`item_fn(string, TypeRef, string) -> ItemFragment`; `extend` consumes the
fragment, not source text. The checked-in proof is
`tools/shape-test/tests/annotations_comptime/directives.rs:220-239`.
Limitations: the name is still an unhygienic string; only zero-argument free
functions and `string`/`int`/finite `number`/`bool` literal bodies are supported;
there is no focused JIT test for the fragment path.

## Implemented constructs

| Construct | Minimal current example | Semantic type and phase | Status, limitation, evidence |
|---|---|---|---|
| Comptime expression | `let x: int = comptime { 1 + 2 }` | `comptime { ... }: T`; compile phase evaluates and embeds an AST literal/structured expression | **CURRENT / VM**. Arrays and objects depend on `KindedSlot -> Expr` projection. `tools/shape-test/tests/comptime/blocks.rs:14-23,219-230`; `compiler/expressions/mod.rs:1866-1931`. |
| Top-level comptime block | `comptime { warning("building") }` | Statement-like, result discarded; compile phase side effects/directives | **CURRENT / VM**. No implicit annotation target, so target-specific directives fail or have no useful target. `shape.pest:45,145-147`; `compiler/statements.rs:1870-1918`. |
| Nested comptime block | `let x = comptime { comptime { 3 } }` | Inner `T` feeds outer mini-program | **CURRENT / VM**. `tools/shape-test/tests/comptime/blocks.rs:56-67`. |
| Comptime helper | `comptime fn twice(x: int) -> int { x * 2 }` | Compile-only `(int) -> int` callable | **CURRENT / VM**. Supports parameters, helper chains, recursion, and string methods; omitted from runtime program and runtime calls are rejected. `shape.pest:333-335`; `compiler/functions.rs:750-768`; `tests/comptime/functions.rs:14-157`. |
| Comptime `for` | `comptime { comptime for f in type_info(T).fields { warning(f.name) } }` | Compile-phase loop over an iterable; loop variable has element type | **CURRENT / compiler** inside a mini-program, where it lowers to an ordinary VM `for`. Standalone `comptime for` outside comptime returns a structured dormant error. `shape.pest:149-152`; `compiler/expressions/misc.rs:819-880`. |
| Ordinary `for` in comptime | `comptime { for f in type_info(T).fields { ... } }` | Compile-phase `for` over `Array<FieldDescriptor>` | **CURRENT / VM+JIT** through stdlib derives. This is the form actually used by shipping annotations. `serde/derive.shape:40-89`; `llm/tools.shape:38-65`. |
| Comptime trait | `comptime trait Label { method label() -> string; }` | Compile-context trait definition | **CURRENT / compiler**. Prepended to the mini-program and absent at runtime; no focused successful ShapeTest for method dispatch was found. Parser comments still say parser-only. `shape.pest:194-197`; `compiler/comptime.rs:613-688`. |
| Comptime impl | `comptime impl Label for Token { method label() { "t" } }` | Compile-context impl/method set | **CURRENT / compiler**. Uses ordinary UFCS/method resolution in the mini-VM and is skipped by outer runtime compilation. No focused successful public proof found. `shape.pest:221-225`; `compiler/statements.rs:875-890,1805-1812`. |
| Comptime field | `type Currency { comptime symbol: string = "$", amount: int }` | Compile-time constant of declared `T`; zero runtime storage | **CURRENT / VM** for static and instance reads. Defaults must be scalar literals and match the declared kind; forbidden on `type C`. `compiler/statements.rs:3685-3751,3773-3780`; `tests/comptime/blocks.rs:258-309,377-428`. |
| Type-alias comptime override | `type USD = Currency { symbol: "$" }` | Intended compile-time field override of `T` | **CURRENT / parse**, semantically a no-op. The compiler explicitly drops overrides and reads the base value. `shape.pest:99-115`; `compiler/statements.rs:1736-1771`. |
| Annotation comptime handler | `comptime post(target, ctx, arg) { ... }` | `(__ComptimeTarget, __ComptimeContext, args...) -> directive stream/value ignored` | **CURRENT / VM** on functions/types/modules. `pre` and `post` are separate phases, positional arguments are validated. `shape.pest:404-428`; `compiler/comptime.rs:1058-1275`. |
| Extension call in comptime | `comptime { db::schema("users") }` | Extension-declared typed signature; sync or async host function result | **CURRENT / compiler unit**. Extension namespaces are registered in the mini-VM and a Tokio runtime is supplied; no general public fixture was found. `compiler/comptime.rs:731-772,3083-3158`. Direct `extern C` execution claimed by the book is unproven. |
| Const-parameter scheduling | `@derive() fn f(const uri: string) { ... }` | Specialization-time constant binding | **CURRENT / compiler**, incomplete for comptime directives. Template handlers defer until bindings exist, but the end-to-end specialization test remains ignored. `compiler/functions.rs:835-862,2890-2943`. |

## Mini-VM operation set

| Operation family | Minimal example | Semantic type | Status and limitation |
|---|---|---|---|
| Scalar literals | `comptime { 1 }`, `2.5`, `true`, `"x"`, `None` | `int`, `number`, `bool`, `string`, optional/null carrier | **CURRENT / VM**. Scalar embedding is the strongest-covered path. `tests/comptime/blocks.rs:40-54,324-339`. |
| Bindings and mutation | `comptime { let mut n = 1; n = n + 1; n }` | Binding retains inferred/declared type | **CURRENT / VM** through block and stdlib handler tests. Runtime outer locals are intentionally unavailable. `tests/comptime/blocks.rs:95-106,341-354`. |
| Arithmetic/comparison/boolean | `comptime { if 2 * 3 > 5 { 1 } else { 0 } }` | Numeric operators plus `bool` condition produce branch `T` | **CURRENT / VM**. `tests/comptime/blocks.rs:95-137,169-205`. |
| Arrays/indexing/iteration | `comptime { let xs = [1, 2]; xs[0] }` | `Array<T>` and `T` | **CURRENT / VM** for arrays; typed-object arrays back descriptor iteration. Heterogeneous arrays remain a weak/unknown carrier. `tests/comptime/blocks.rs:219-230`; `comptime_target.rs:121-217`. |
| Strings/f-strings/methods | `comptime { " x ".trim() }` | `string` | **CURRENT / VM**. `tests/comptime/functions.rs:69-84`. |
| Helper calls/recursion | `comptime fn fact(n: int) { if n <= 1 { 1 } else { n * fact(n-1) } }` | Declared/inferred function result | **CURRENT / VM**. Only comptime helpers are imported into the mini-program; ordinary runtime functions are unavailable. `tests/comptime/functions.rs:86-157,188-222`. |
| Diagnostics/output | `warning("x")`, `error("x")`, `print("x")` | `warning`/`print -> ()`; `error` aborts compilation | **CURRENT / VM+JIT** for `error`; warning is spanned and routed. Top-level `print` is observable and exactly-once deopt logic exists. `comptime_builtins.rs:716-762`; `flagship_wf3d.rs:97-118`; `compiler/mod.rs:1856-1873`. |
| Execution budget | `comptime { while true {} }` | No value before interruption | **CURRENT / compiler** with a five-second interrupt watchdog. This is time-based, not a deterministic effect/resource type. `compiler/comptime.rs:1328-1337`. |

The mini-program does not inherit runtime locals or the runtime prelude. Its
inputs are comptime helpers, comptime context trait/struct/impl items, reserved
builtins, target/context bindings, const-specialization bindings, and registered
extensions (`compiler/comptime.rs:613-772,1213-1269`).

## Current semantic descriptors

| Descriptor | Current semantic fields | Minimal use | Strict-typing limitation |
|---|---|---|---|
| `ComptimeContext` | `{ module_path: string, file: string }` | `ctx.module_path` | Reserved typed object. `ctx.build` does not exist; use `build_config()`. `compiler/comptime.rs:87-105,1178-1194`. |
| `TypeRef` | `{ name: string, kind: string, source: string }` | `target.params[0].type_ref` | Typed shell, but forgeable/string-backed; `kind` is heuristic and `source` is reparsed. `comptime_target.rs:56-119`. |
| `AnnotationDescriptor` | `{ name: string, args: Array<Any> }` | `field.annotations[0].name` | Arguments are stringified, including debug fallback for complex expressions. `builtin_schemas.rs:274-277`; `comptime_target.rs:183-195,491-503`. |
| `FieldDescriptor` | `{ name: string, type: string, annotations: Array<Any>, optional: bool, type_ref: TypeRef }` | `type_info(T).fields[0].type_ref` | Runtime rows are typed objects, but handler static annotation declares `annotations: Array<string>`; this mismatch is an explicit unknown for broad annotation use. `builtin_schemas.rs:259-265`; `comptime.rs:107-147`. |
| `ParamDescriptor` | `{ name: string, type: string, const: bool, type_ref: TypeRef }` | `target.params[1].type_ref` | Parameter identity is spelling only; unannotated parameters become `"any"`. `comptime_target.rs:239-273,439-455`. |
| `ComptimeTarget` | `{ kind: string, name: string, fields: Array<Any>, params: Array<Any>, return_type: string?, return_type_ref: TypeRef, annotations: Array<Any>, captures: Array<Any> }` | `target.name` | Function/type/module data is useful, but arrays are not closed typed contracts; annotations/captures are names, not IDs/descriptors. `builtin_schemas.rs:291-300`; `comptime_target.rs:219-343,434-487`. |
| `TypeInfo` | `{ name: string, kind: string, fields: Array<Any>, type_ref: TypeRef }` | `type_info(Profile)` | Struct fields are populated; non-struct kinds have empty fields. Enum and struct both report `TypedObject`; unknown/generic collapse to `Unresolved`. `builtin_schemas.rs:302-318`; `comptime_builtins.rs:1221-1357`. |
| `BuildConfig` | `{ debug: bool, version: string, target_os: string, target_arch: string, comptime_api: int }` | `build_config().comptime_api` | Concrete reserved schema; host configuration only. `builtin_schemas.rs:228-239`; `comptime_builtins.rs:764-851`. |
| `ItemFragment` | `{ kind, name, return_type, return_type_ref, literal_kind, literal_* sentinels }` | `item_fn(...)` | Typed object with string discriminants and parallel sentinel fields, not a general checked AST. `builtin_schemas.rs:247-257`; `comptime_builtins.rs:557-625`. |

`ComptimeExecutionResult` is an internal typed carrier containing a `KindedSlot`
value, `Vec<ComptimeDirective>`, and diagnostics. It is not a Shape-visible plan
type (`compiler/comptime.rs:70-85`). The dormant `ConstantValue` model in
`compiler/comptime_concrete.rs` is test/internal dead-code scaffolding, not a
public implemented comptime value API.

## Comptime builtins

| Builtin | Minimal example | Semantic type | Status and limitation |
|---|---|---|---|
| `implements` | `implements(User, Display)` | `(string-backed TypeRef name, string-backed TraitRef name) -> bool` | **CURRENT / VM**. Bare type symbols are rewritten to strings; supports legacy/canonical impl keys and integer-to-number widening. Not strict identity. `comptime_builtins.rs:681-714`; `tests/comptime/functions.rs:159-186,224-244`. |
| `warning` | `warning("deprecated")` | `string -> ()` | **CURRENT / VM**. Collected and surfaced with the driving span. `comptime_builtins.rs:716-736`. |
| `error` | `error("bad schema")` | Surface `string -> never`; registered implementation returns an error from a Unit-typed host function | **CURRENT / VM+JIT** diagnostics. `comptime_builtins.rs:738-762`; `flagship_wf3d.rs:99-118`. |
| `build_config` | `build_config().target_arch` | `() -> BuildConfig` | **CURRENT / compiler unit**, with weaker ShapeTests that often discard fields. Exact field-value public proof is not established here. `comptime_builtins.rs:764-851`; `tests/comptime/blocks.rs:84-93,436-465`. |
| `type_info` | `type_info(Profile)` | `TypeRef-like argument -> TypeInfo` | **CURRENT / VM+JIT** for exact basic reflection. Generic/enum tests often assert only non-panic sentinels, so deep generic semantics remain unknown. `comptime_builtins.rs:853-907,1221-1357`; `flagship_wf3d.rs:68-94`. |
| `item_fn` | `item_fn("label", type_info(string).type_ref, "x")` | `(string, string | TypeRef, scalar literal) -> ItemFragment` | **CURRENT / VM**. The string return-type overload is legacy; use `TypeRef`. Only one zero-arg function fragment. `comptime_builtins.rs:909-953`. |
| `string_lit` | `string_lit(schema)` | `string -> string` containing escaped Shape source literal text | **LEGACY CURRENT / VM+JIT** through source generators. It exists only for textual code generation and is not recommended. `comptime_builtins.rs:1149-1175`; `flagship_wf3d.rs:41-63,125-148`. |

All user builtins are forwarded into an internal `__comptime__` extension. The
`__emit_*` functions are compiler plumbing, not public semantics: they collect
the directive variants below, and several direct-AST forms serialize and decode
JSON internally (`compiler/statements.rs:128-137,7926-7984` and
`compiler/comptime_builtins.rs:955-1146`).

## Annotation phases

| Phase | Minimal handler | Semantic contract | Current status |
|---|---|---|---|
| `comptime pre` | `comptime pre(target, ctx) { error("x") }` | Compile-time target validation/rewrite | **CURRENT / VM** for function/type/module. For stacked annotations, all `pre` handlers run in source order. |
| `comptime post` | `comptime post(target, ctx) { extend target { ... } }` | Compile-time rewrite after pre | **CURRENT / VM** for function/type/module. All `post` handlers run in source order. `functions_annotations.rs:771-816`. |
| Analysis materialization | same source handlers | Speculative signature/generated-item discovery before ordinary analysis | **CURRENT / compiler**. Handlers can execute more than once with output suppressed, then execute authoritatively; handlers therefore must be deterministic and side effects are risky. `functions_annotations.rs:24-205,1427-1743`. |
| `on_define` | `on_define(target, ctx) { ... }` | Definition-emission runtime call | **CURRENT / VM** for function/type/module only. Function first argument is a function-id value while type/module receive `{name,kind,id}`; this is not one typed target contract. `functions_annotations.rs:488-665`. |
| `metadata` | `metadata(target, ctx) { { tag: "x" } }` | Definition-emission runtime call whose result should be metadata | **CURRENT / compiler**, but emitted result is immediately popped; no durable metadata retrieval API was found. `functions_annotations.rs:547-665`. |
| `before` | `before(args, ctx) { args }` | Runtime argument wrapper, currently `Array<Any>` plus `{state: Any,event_log:Array<Any>}` | **CURRENT / VM** for functions. Not a strictly typed comptime surface. |
| `after` | `after(args, result, ctx) { result }` | Runtime result wrapper | **CURRENT / VM** for functions; source-order nesting is before outer-to-inner, after inner-to-outer. `tests/comptime/annotations.rs:13-134,238-265`. |

Handler argument binding is positional: parameter 0 is typed target, parameter
1 typed compile context, then annotation arguments. If only `(target, ctx)` is
declared, annotation-definition parameters are injected by name. Missing and
extra arguments are diagnosed. One final variadic parameter is parsed and
validated, but heterogeneous variadic execution coverage is ignored; broad
variadic behavior is unknown (`compiler/comptime.rs:1072-1176` and
`compiler/functions.rs:3017-3037`). Duplicate application of the same annotation
to one item is rejected (`type_mutation.rs:339-361`).

## Annotation targets

| Target | Minimal current/illustrative application | Comptime status | Runtime/JIT status and limitation |
|---|---|---|---|
| `function` | `@a() fn f(x: int) -> int { x }` | **CURRENT / VM**, full pre/post directive path | Runtime before/after and lifecycle work in VM. Generated functions can be native JIT. |
| `type` (struct) | `@a() type T { x: int }` | **CURRENT / VM**, full pre/post extend/remove path | Generated methods have VM+JIT tests. Enum/trait declarations parse annotations but do not share the proven struct handler path. |
| `module` | `@a() mod m { fn f() { 1 } }` | **CURRENT / VM** for pre/post, remove, extend, and replacement | `replace module` proof uses source text. Runtime module lifecycle supports only `on_define`/`metadata`; before/after do not wrap module execution. |
| `expression` | `@a() (1 + 2)` | **CURRENT / compiler** comptime descriptor/directive path | Runtime before/after currently reaches `op_new_array(0): SURFACE`; no successful public expression proof. `regression/language_surface.rs:75-97`. |
| `await_expr` | `await @a() future()` | **CURRENT / compiler** specialized lowering exists | Book examples are `runnable=false`; no successful focused proof found. Treat as unknown, including JIT. `shape.pest:1181`; `compiler/expressions/mod.rs:1113-1208`. |
| `block` | `targets: [block]` | **CURRENT / parse/enum only** | No direct attachment syntax or successful execution proof found. |
| `binding` | `targets: [binding]` | **CURRENT / parse/enum only** | No direct `@a let ...` attachment path or successful proof found. |
| Method/impl member | `@a() method m() { ... }` | Not a completed target surface | Checked-in tests record parser failures for impl and inline methods. `tests/comptime/annotations.rs:561-629`. |

The seven target labels are defined in the grammar and AST
(`shape.pest:390-402`; `shape-ast/src/ast/functions.rs:261-279`). Target
restrictions are enforced; an empty `targets` list behaves broadly, except
`on_define`/`metadata` are restricted to definition targets function/type/module
(`compiler/compiler_impl_reference_model.rs:1689-1763`).

## Directive inventory

| Directive | Minimal current example | Semantic operation and valid target | Status and limitation |
|---|---|---|---|
| `remove target` | `comptime post(target,ctx) { remove target }` | Remove current function/type/module/expression | **CURRENT / VM** for type and compiler paths for others. Stops later handlers. Removed function calls receive diagnostics; expression removal yields null. |
| `set param p = expr` | `set param limit = 10` | Set scalar default on existing function parameter | **CURRENT / VM**. Supports `int`, finite `number`, `bool`, `string`, and `None`; explicit call argument wins. Cannot add parameters. `directives.rs:8-144`; `functions_annotations.rs:2143-2159`. |
| `set param p: Type` | `set param left: string` | Concretize existing untyped parameter | **CURRENT / VM** for compatible paths. Cannot override a different explicit type or add a parameter. Direct AST crosses internal JSON. |
| `set param p: (expr)` | `set param left: (target.params[1].type_ref)` | Concretize existing parameter from `TypeRef` | **CURRENT / VM** typed input. Still reparses `TypeRef.source`. `type_mutation.rs:313-336`. |
| `set return Type` | `set return int` | Set absent function return type | **CURRENT / VM**. Cannot override a different explicit return; final body is rechecked. Direct AST crosses internal JSON. `type_mutation.rs:211-259`. |
| `set return (expr)` | `set return (target.params[0].type_ref)` | Set absent return type from `TypeRef` or legacy text/JSON | **CURRENT / VM** with `TypeRef`; source/JSON overloads are legacy. `type_mutation.rs:291-310`. |
| `replace body { ... }` | `replace body { 42 }` | Replace function body with parsed AST statements | **CURRENT / VM**. Creates typed shadow `__original__<fn>` and alias `__original__`; forwarding uses real parameters. Internal AST JSON roundtrip remains. `type_mutation.rs:186-209`; `functions_annotations.rs:2170-2225`. |
| `replace body (expr)` | `replace body (body_source)` | Replace function body from source or statement-AST JSON | **LEGACY CURRENT / compiler**. Decoder exists, but the focused execution unit is ignored and no typed `CheckedBody` carrier exists. `compiler/functions.rs:3098-3123`. |
| `replace module (expr)` | `replace module ("fn answer() -> int { 42 }")` | Replace module items from source or item-AST JSON | **LEGACY CURRENT / VM**. Type errors in generated module are rechecked; no typed `CheckedModule`. `directives.rs:146-194,241-260`. |
| `extend target { ... }` / `extend Type { ... }` | `extend target { method label() { "x" } }` | Add statically parsed methods/impl content | **CURRENT / VM+JIT** for generated methods. `target` is textually substituted by current target name; no hygienic symbol. `code_gen.rs:9-220`. |
| `extend (expr)` | `extend (item_fn(...))` | Add an `ItemFragment`, or legacy source containing free functions/extend blocks | **CURRENT / VM** typed fragment; **LEGACY CURRENT / VM+JIT** source path. Other item kinds are rejected. Generated functions/methods pass the full compile/MIR pipeline. `comptime_builtins.rs:627-653`; `functions_annotations.rs:1745-1799`. |

All directives are accepted syntactically only in comptime mode
(`compiler/statements.rs:7926-7984`). Function-only directives are rejected on
type/expression/module targets; module replacement is rejected outside module
compilation (`compiler/functions_annotations.rs:2047-2235`).

## Typed `ItemFragment` slice

The complete current slice is:

```text
item_fn(name: string, return_type: string | TypeRef,
        value: string | int | finite number | bool) -> ItemFragment
extend(ItemFragment) -> one generated zero-argument free function
```

The function name must be a valid non-keyword Shape identifier. The fragment is
validated, reconstructed as a `FunctionDef` with no parameters/type parameters,
one literal expression body, no annotations/effects/async, then registered and
compiled through the ordinary strict function driver. Unsupported fragment
kinds and malformed schema identities fail cleanly
(`compiler/comptime_builtins.rs:409-625`; `functions_annotations.rs:1745-1799`).

Explicit unknowns: no typed method, parameter, generic, effect, async, body,
module, type, enum, trait, impl, multi-item, or hygienic-symbol fragment exists;
no fragment composition API exists; no focused JIT test exercises `item_fn`.

## Stdlib comptime definitions

| Definition | Current operation | Evidence/status | Strict limitation |
|---|---|---|---|
| `std::serde::derive.@json_schema` | Reflects `target.fields`, optionality, `type_ref.kind`, and field descriptions; emits free schema function | **CURRENT / VM+JIT** showcase. `stdlib-src/serde/derive.shape:30-96`; `showcases.rs:27-74`. | Final function is source text via `extend(f"...")`/`string_lit`; annotation args are `Any`/stringified. |
| `std::serde::serialize.@to_json` | Reflects fields and emits a `to_json` method | **CURRENT / VM+JIT** showcase. `stdlib-src/serde/serialize.shape:21-56`; `showcases.rs:77-101`. | Uses legacy `.type` strings and constructs method source text. Only primitive fields. |
| `std::llm::tools.@llm_tool` | Validates signature and emits `{fn}_tool_def` | **CURRENT / VM+JIT** showcase. `stdlib-src/llm/tools.shape:24-69`; `showcases.rs:104-151`. | Uses legacy type strings, `string_lit`, and generated source. |
| `std::llm::tools.@prompt` | Validates `{placeholder}` names against function params | **CURRENT / VM** positive/error tests. `stdlib-src/llm/tools.shape:71-94`; `showcases.rs:154-190`. | String parser, parameter identity by spelling; no JIT-specific test. |

Other stdlib annotations such as remote/indicator lifecycle hooks are runtime
annotation definitions, not additional comptime operations. The four rows above
are the complete comptime stdlib definitions found in the inspected sources.

## VM and JIT interpretation

- Comptime execution itself always uses a fresh bytecode `VirtualMachine`; it is
  not JIT-compiled (`compiler/comptime.rs:1287-1366`).
- A program with top-level comptime is intentionally deoptimized before JIT
  compilation so compile-time side effects execute exactly once
  (`compiler/mod.rs:1856-1873`). Thus a JIT-mode reflection/error test can prove
  mode parity while still evaluating comptime in the VM.
- Generated free functions and generated methods are compiled through
  `compile_function`, receive MIR, and have explicit native VM/JIT tests for the
  source-generation path (`functions_annotations.rs:1388-1424,1745-1799`;
  `flagship_wf3d.rs:37-63,121-148`; `code_gen.rs:172-192`).
- The stdlib `@json_schema`, `@to_json`, and `@llm_tool` have explicit VM/JIT
  return-value tests. Most block, directive, field, lifecycle, and typed
  `ItemFragment` tests are VM-only.
- No successful JIT proof was found for comptime trait/impl dispatch,
  expression/await targets, module replacement, const-specialized handlers, or
  variadic handlers.

## Illustrative strictly typed target syntax

The following is **TARGET ONLY** design notation. It must not be presented as
runnable current Shape:

```shape
comptime post(
  target: FrozenType<T>,
  ctx: ComptimeContext<T>
) -> RewritePlan<T> {
  let label: HygienicSymbol = ctx.symbol.sibling(target.symbol, "label")
  let item: CheckedItem = quote item {
    fn $label() -> string { "typed" }
  }
  ctx.rewrite.add_item(item)
}
```

```shape
comptime post(
  target: FrozenCallable<Sig>,
  ctx: ComptimeContext<Sig>
) -> RewritePlan<Sig> {
  let source: ParamDescriptor<Sig, 0> = target.params[0]
  ctx.rewrite.set_return(source.type_ref)
}
```

Required invariants are opaque canonical `TypeRef`/`TraitRef` identities,
stable `ParamId`/`SymbolId`, closed typed annotation and const values,
`FrozenType`/`FrozenCallable`, checked expression/body/item/module fragments,
typed effect/ownership descriptors, and one atomic `RewritePlan`. There is no
source-string, JSON, `Any`, debug-stringification, reparsing, or magic-name
selection. This target model is consistent with
`docs/cluster-audits/wave41-comptime-untyped-paths.md`.

## Book and proof corrections

- `advanced/comptime.mdx:113-120` says applied hooks are planned; current
  function/type/module application and directives are implemented, so that
  caution is stale.
- The same page accurately documents current source/JSON payloads at
  `:168-243`, but those are legacy compatibility surfaces, not strict examples.
- Its connector specialization/`extern C` example is `runnable=false`; direct
  native calls from comptime and end-to-end const-specialized annotation
  directives are not proven (`:264-301`).
- `examples/comptime-codegen.mdx:12-20` labels its broad field-generation example
  illustrative. Parts now exist, but schema-driven type generation, body/module
  checked fragments, external schema I/O, and several shown combinations remain
  unproven or legacy.
- `advanced/annotations.mdx:157-162` lists all seven target labels. That is a
  grammar/descriptor list, not evidence that block/binding/await/expression
  execution is complete; its expression and await examples are correctly
  `runnable=false` at `:71-105`.
- Older parser comments call comptime trait/impl parser-only, while the current
  compiler prepends them to the mini-VM. Conversely, there is no focused public
  execution proof, so **CURRENT / compiler** is the honest status.
- Several `type_info_chained.rs` tests assert only a sentinel after discarding
  reflected fields. Exact claims should rely on `:72-90` and the flagship
  VM/JIT test, not the non-panic generic/enum rows.

## Explicit unknowns and boundaries

1. No focused successful proof establishes comptime trait/impl user method
   dispatch, though the compiler path is present.
2. Expression comptime directives have a compiler path, but runtime wrappers
   fail on the deleted empty-array surface; await has no successful proof;
   block and binding have no attachment/execution path found.
3. Annotation-handler `type_info(UserType)` cannot see user definitions because
   its reflection snapshot is currently empty.
4. Descriptor static types and runtime schemas disagree around field annotation
   rows, and several arrays remain `Any`/`unknown`.
5. Const-specialized comptime handlers and heterogeneous variadics have ignored
   tests, so they are not runnable recommendations.
6. `build_config` construction has unit coverage, but exact public field-value
   coverage is weaker than the reflection flagship.
7. Type-alias comptime field overrides are explicitly discarded.
8. Source-generated free functions/methods have native JIT evidence; typed
   `ItemFragment` generation does not yet have equivalent focused JIT evidence.
9. No public typed checked-fragment API exists beyond `item_fn`; effects,
   ownership, hygiene, stable identities, conflict handling, and atomic rewrite
   validation are absent from the current public contract.

## Primary source map

- Grammar/AST: `crates/shape-ast/src/shape.pest:30-52,98-225,333-428,530-556`;
  `crates/shape-ast/src/ast/program.rs:30-82`;
  `crates/shape-ast/src/ast/functions.rs:214-279`;
  `crates/shape-ast/src/ast/statements.rs:10-76`.
- Mini-VM and handler binding: `crates/shape-vm/src/compiler/comptime.rs:70-275,
  613-772,1058-1366`.
- Targets/reflection: `crates/shape-vm/src/compiler/comptime_target.rs:32-119,
  172-217,219-504`; `comptime_builtins.rs:853-953,1221-1357`.
- Schemas: `crates/shape-runtime/src/type_schema/builtin_schemas.rs:220-329`.
- Directive application/JIT materialization:
  `crates/shape-vm/src/compiler/functions_annotations.rs:771-850,1388-1799,
  2047-2265`; `crates/shape-vm/src/compiler/statements.rs:7926-7984`.
- Focused public tests: `tools/shape-test/tests/comptime/**` and
  `tools/shape-test/tests/annotations_comptime/**`, with the exact high-signal
  rows cited above.
- Sibling docs: `../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx`,
  `advanced/annotations.mdx`, `advanced/comptime-llm-patterns.mdx`, and
  `examples/comptime-codegen.mdx`.
