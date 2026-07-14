# Wave 41B: Comptime Untyped-Path Migration Inventory

## Verdict

The current pipeline often flattens already-parsed or computed compiler data
into source text, JSON, `Any`, or a bare name, then reconstructs it without the
original type, scope, identity, or hygiene proof. The target boundary should be:

```shape
comptime handler(
  target: FrozenCallable<Sig>,
  ctx: ComptimeContext<Sig>
) -> RewritePlan<Sig>
```

`RewritePlan<Sig>` contains only checked fragments and compiler identities. The
compiler validates it atomically against the frozen target before registering
generated items. There is no final source-string or JSON compatibility arm.

The `AFTER` examples use design notation: `quote expr/body/item/module { ... }`
returns a checked fragment; `$x` is a type-checked splice; `#p` resolves to a
parameter identity and is never retained as a runtime string.

Ordinary parsing of user source, runtime JSON/YAML data, and the bounded JSON
diagnostic envelope in `comptime_diagnostics.rs` are not migration targets.

## Type model

| Structure | Required invariant |
|---|---|
| `FrozenCallable<Sig>` | Resolved callable ID, frozen signature, ordered parameters, return `TypeRef`, annotation arguments, captures, effects, and provenance. |
| `ParamDescriptor<Sig, I>` | Stable `ParamId`, hygienic symbol, `TypeRef`, mode, default, and span for parameter `I`. |
| `TypeRef` | Opaque canonical type identity; not constructible from `string` and has no reparsable `source`. |
| `HygienicSymbol` | `SymbolId`, owning scope, and non-semantic display spelling. Equality uses the ID. |
| `CheckedExpr<T>` | Type-, effect-, and scope-checked expression with resolved references. |
| `CheckedStmt` / `CheckedBody<R>` | Checked statements/body with explicit result and effect row. |
| `CheckedItem` / `CheckedModule` | Checked declarations with hygienic IDs and dependency edges. |
| `RewritePlan<Sig>` | Ordered atomic edits keyed by target/item/parameter identities, with phase and conflict validation. |

Missing types found here are `FrozenType`, `FieldDescriptor`,
`AnnotationDescriptor`, `CaptureDescriptor`, a closed `ConstValue<T>` sum,
`TraitRef`, `HookPlan<Sig, State>`, and `CheckedTemplate<Sig>`. `KindedSlot` is
not a compiler-facing substitute for `ConstValue`.

## Path inventory

Path shorthand in this section is exact: compiler files are under
`crates/shape-vm/src/compiler/`, `runtime/...` is under
`crates/shape-runtime/src/`, stdlib files are under
`crates/shape-runtime/stdlib-src/`, and test files are under
`tools/shape-test/tests/`.

### U01. Parsed AST -> JSON -> parsed AST directives

**Paths/callers:** `compiler/statements.rs:128-137`
`serialize_directive_payload`; callers `emit_comptime_extend_directive`
(`:593-603`), `emit_comptime_set_param_type_directive` (`:626-638`),
`emit_comptime_set_return_type_directive` (`:659-670`), and
`emit_comptime_replace_body_directive` (`:680-691`). Decoders are
`__emit_extend` (`comptime_builtins.rs:951-971`), `__emit_set_param_type`
(`:1000-1024`), `__emit_set_return_type` (`:1069-1090`), and
`__emit_replace_body` (`:1092-1105`).

**BEFORE:**
```shape
set return string
replace body { return "generated" }
```
**AFTER:**
```shape
ctx.rewrite.set_return(TypeRef<string>)
ctx.rewrite.replace_body(quote body -> string { return "generated" })
```
**Primary:** `RewritePlan<Sig>`. **Dependencies:** opaque TypeRef, checked quotes,
typed mini-VM plan values. **Delete when:** these emitters pass checked values
directly and no AST serde decoder or malformed-AST-JSON diagnostic remains.

### U02. Source/JSON type reparsing

**Paths/callers:** `parse_type_annotation_payload`
(`comptime_builtins.rs:233-253`) accepts JSON or parses synthetic
`fn __type_probe(value: ...)`. It is reached by
`type_annotation_from_string_or_type_ref_slot` (`:285-322`),
`type_source_from_string_or_type_ref_slot` (`:325-358`), `item_fn`
(`:541-574`), `function_item_from_fragment` (`:583-625`), and both set-type
builtins.

**BEFORE:**
```shape
set return ("Array<Result<int, string>>")
set param value: (target.params[0].type_ref) // contains source text
```
**AFTER:**
```shape
ctx.rewrite.set_return(TypeRef<Array<Result<int, string>>>)
ctx.rewrite.set_param_type(target.param(#value), target.params[0].type_ref)
```
**Primary:** `TypeRef`. **Dependencies:** non-forgeable interned type handles
across the comptime VM. **Delete when:** `__ComptimeTypeRef.source`, string
overloads, the parser function, and both adapter functions are gone.

### U03. Source/JSON body and module reparsing

**Paths/callers:** `parse_function_body_payload`
(`comptime_builtins.rs:361-376`) accepts JSON or synthetic `fn __body_probe()`;
`parse_module_items_payload` (`:379-397`) accepts JSON or source. They feed
`__emit_replace_body` (`:1092-1105`), `__emit_replace_module` (`:1107-1121`),
and string `parse_extend_items_slot` (`:627-652`). Regression callers include
`annotations_comptime/directives.rs:153,176,203,248` and
`compiler/statements.rs:8077-8151`.

**BEFORE:**
```shape
replace module ("fn answer() -> int { 42 }")
replace body (f"return decode({target.name})")
```
**AFTER:**
```shape
ctx.rewrite.replace_module(quote module { fn answer() -> int { 42 } })
ctx.rewrite.replace_body(quote body -> R { return decode($target_value) })
```
**Primary:** `CheckedModule` (with `CheckedBody<R>` for body edits).
**Dependencies:** typed quote/splice, effect checks, module-target validation.
**Delete when:** both payload parsers and their synthetic probe names are gone.

### U04. String-backed TypeRef and `Any` descriptors

**Paths/callers:** `comptime_target.rs:106-120` derives TypeRef `name`/`kind`
from source and retains `source`. `ComptimeTarget` (`:222-310`) stores fields,
params, return, annotations, and captures as strings; `to_nanboxed` (`:430-493`)
emits them. `expr_to_string_lossy` (`:495-505`) even falls back to debug text;
`type_annotation_to_string` (`:507-568`) renders type source.
`runtime/type_schema/builtin_schemas.rs:241-318` defines the magic descriptor
objects with string fields and `Array<Any>` members.

**BEFORE:**
```shape
if target.params[0].type == "Array<int>" &&
   target.params[0].type_ref.kind == "Array" { print(target.annotations[0].args[0]) }
```
**AFTER:**
```shape
let p: ParamDescriptor<Sig, 0> = target.params[0]
if p.type_ref == TypeRef<Array<int>> {
  let d: ConstValue<string> = target.annotation<description>().value
}
```
**Primary:** `FrozenCallable<Sig>`. **Dependencies:** typed field, annotation,
capture, and const descriptors. **Delete when:** magic descriptor schemas,
semantic `.type`/`.kind` strings, `expr_to_string_lossy`, and descriptor `Any`
fields have no users.

### U05. String-keyed reflection and bare-type rewriting

**Paths/callers:** `TypeReflectionSnapshot` (`comptime_builtins.rs:29-125`)
indexes structs, enums, aliases, fields, and type parameters by `String`; alias
targets degrade to `TypeAnnotation::Basic(name)`. The full AST walker
`rewrite_comptime_type_symbol_args` (`comptime.rs:390-575`) rewrites bare
identifiers passed to `type_info`/`implements` into strings. The forwarder table
at `comptime.rs:44-65` is also name/arity based.

**BEFORE:**
```shape
let info = type_info(User) // User becomes "User"
if implements(User, "Display") { ... }
```
**AFTER:**
```shape
let user: TypeRef = TypeRef<User>
let info: FrozenType<User> = type_info(user)
if implements(user, TraitRef<Display>) { ... }
```
**Primary:** `TypeRef`. **Dependencies:** `FrozenType`, `TraitRef`, resolver IDs
in the reflection snapshot. **Delete when:** the rewrite walker, bare-name
classifier, string overloads, and string-keyed snapshot API are absent.

### U06. Parameters selected by spelling

**Paths/callers:** `ComptimeDirective::{SetParamType,SetParamValue}` stores
`param_name: String` (`comptime_builtins.rs:129-158`). Pre-analysis in
`functions_annotations.rs:208-254` uses `simple_name()` and silently skips a
miss; application repeats the lookup at `:2097-2169`. Emitters take a string at
`comptime_builtins.rs:995-1061`; current use is
`annotations_comptime/type_mutation.rs:297-323`.

**BEFORE:**
```shape
set param left: (target.params[1].type_ref)
set param value: 10
```
**AFTER:**
```shape
ctx.rewrite.set_param_type(target.param(#left), target.params[1].type_ref)
ctx.rewrite.set_param_default(target.param(#value), quote expr<int> { 10 })
```
**Primary:** `ParamDescriptor`. **Dependencies:** `ParamId`, checked defaults,
one shared validation pass. **Delete when:** directives carry no parameter
strings, no `simple_name()` search remains, and stale IDs fail before apply.

### U07. String/sentinel `ItemFragment`

**Paths/callers:** `literal_fragment_fields_from_slot`,
`build_function_item_fragment`, `function_item_from_fragment`, and
`parse_extend_items_slot` (`comptime_builtins.rs:430-652`) encode only a
zero-argument literal-returning function using string `kind`, `name`, return
type, `literal_kind`, and parallel sentinel fields. Its schema is
`builtin_schemas.rs:247-257`; `item_fn` is registered at
`comptime_builtins.rs:909-950`; caller `directives.rs:227`.

**BEFORE:**
```shape
extend (item_fn(f"{target.name}_label", type_info("string").type_ref, "value"))
```
**AFTER:**
```shape
let label = ctx.symbol.sibling(target.symbol, "label")
ctx.rewrite.add_item(quote item { fn $label() -> string { return "value" } })
```
**Primary:** `CheckedItem`. **Dependencies:** hygienic symbols, item quotes,
checked literal splices. **Delete when:** the magic schema, `item_fn`, sentinels,
schema-name probing, and fragment reconstruction are removed.

### U08. `string_lit` source escaping

**Paths/callers:** forwarder `comptime.rs:63-65`, implementation
`comptime_builtins.rs:1150-1164`; production callers
`serde/derive.shape:95`, `llm/tools.shape:67`; test callers
`comptime/flagship_wf3d.rs:45,129`.

**BEFORE:**
```shape
extend (f"fn {target.name}_json_schema() -> string \{ {string_lit(schema)} \}")
```
**AFTER:**
```shape
let name = ctx.symbol.sibling(target.symbol, "json_schema")
let value: CheckedExpr<string> = CheckedExpr.literal(schema)
ctx.rewrite.add_item(quote item { fn $name() -> string { return $value } })
```
**Primary:** `CheckedExpr<T>`. **Dependencies:** checked literal and symbol
splices. **Delete when:** no Shape caller uses `string_lit` and generated code
never requires textual escaping. The schema string remains valid output data.

### U09. Helpers and annotations resolved by names

**Paths/callers:** `collect_scoped_helpers_for_expr` plus manual AST walkers
(`functions_annotations.rs:934-1369`) collect call spellings, query
`function_defs` by string (`:947-956`), and deduplicate by name. Callers are
`:157`, `:886`, and `:1556`. `collect_comptime_annotation_handlers`
(`:1678-1743`) keys local/imported definitions by bare annotation name, so
traversal order resolves collisions. Generated-item prepass/application also
keys and deduplicates functions/methods by spelling at `:1388-1424` and
`:1585-1665`.

**BEFORE:**
```shape
use alpha::schema_for
@derive
comptime post(target, ctx) { extend schema_for(target) }
```
**AFTER:**
```shape
use alpha::schema_for
@alpha::derive
comptime post(target: FrozenCallable<Sig>, ctx: ComptimeContext<Sig>) {
  ctx.rewrite.merge(schema_for(target)) // checked edge already has CallableId
}
```
**Primary:** `FrozenCallable`. **Dependencies:** resolution before freezing,
checked dependency graph, resolved annotation IDs. **Delete when:** no recursive
name collector exists and same-spelled imports cannot depend on iteration order.

### U10. Unhygienic synthetic names and magic roles

**Paths/callers:** mini-program identifiers `argN` (`comptime.rs:270-289`),
`__comptime_block__` (`:648`), `__comptime_handler_fn__` (`:1197`), and
`__target_arg__`/`__ctx_arg__` (`:1116-1120`, `:1260-1280`). Annotation lowering
creates `annotation___handler` (`statements.rs:3280-3395`),
`__ann_*_wrapper_*`/`__ann_arg_*` (`functions_annotations.rs:2637-2729`), and
infers roles from `ctx`, `fn`/`target`, `args`, `result`. Expression lowering
declares ordinary `__ann_*` locals (`expressions/mod.rs:949-1208`).

**BEFORE:**
```shape
before(fn, args, ctx) { args }
after(fn, result, ctx) { result }
```
**AFTER:**
```shape
before(target: FrozenCallable<Sig>, args: ArgumentPack<Sig>, ctx: BeforeContext<S>)
    -> HookDecision<Sig, S> { Proceed(args, ctx.state) }
```
**Primary:** `HygienicSymbol`. **Dependencies:** typed `HookPlan` roles and local
allocation by `SymbolId`. **Delete when:** synthetic names cannot collide,
roles are type/position based, and no magic spelling enters a symbol table.

### U11. Original/wrapper/target capabilities as aliases

**Paths/callers:** replacement creates `__original__{function}` plus global
`function_aliases["__original__"]` (`functions_annotations.rs:2170-2225`;
cleanup `functions.rs:1079-1233`). Annotation chains create `{name}___impl` and
`{name}___{annotation}` (`functions_annotations.rs:2338-2530`). Extend compares
the target name with magic `target` (`:1388-1404`). Foreign wrappers use
`___ann_wrapper` (`functions_foreign.rs:364`).

**BEFORE:**
```shape
replace body { return __original__(value) + 1 }
extend target { method label() -> string { "x" } }
```
**AFTER:**
```shape
ctx.rewrite.replace_body(quote body -> int { return $ctx.original(value) + 1 })
ctx.rewrite.extend_type(target.owner_type,
  quote item { method label() -> string { "x" } })
```
**Primary:** `HygienicSymbol`. **Dependencies:** capability fields in context,
stable generated `CallableId`, checked callable splices. **Delete when:** aliases,
magic `__original__`, semantic `___impl` lookup, and string `target` substitution
are gone. Debug display names may remain non-unique metadata.

### U12. Parallel static comptime-extend collector

**Paths/callers:** `shape-ast/src/transform/comptime_extends.rs:27-180` indexes
definitions/targets by bare names, substitutes magic `target`, walks handler AST
without evaluating conditions, collects both branches, and deduplicates methods
by spelling. It duplicates execution in `functions_annotations.rs`.

**BEFORE:**
```shape
if ctx.enabled { extend target { method enabled() -> bool { true } } }
```
The static collector can observe this edit when `ctx.enabled` is false.

**AFTER:**
```shape
if ctx.enabled {
  ctx.rewrite.extend_type(target.owner_type,
    quote item { method enabled() -> bool { true } })
}
```
**Primary:** `RewritePlan<Sig>`. **Dependencies:** one authoritative prepass and
identity-based conflicts. **Delete when:** this transform has no production
caller and condition/stack/import tests pass only through executed plans. It
must not survive as fallback because duplicate processing changes order.

### U13. Runtime hook `Any` carrier and shape inspection

**Paths/callers:** `functions_annotations.rs:681-717` types state/event log as
`Any`/`Array<Any>`. Wrapper construction (`:2872-3151`) interprets before output
by array/object shape; `args`, `result`, and `state` are `Any`, while `null`
controls flow. Argument extraction continues at `:3153-3232`; after output is
accepted without the frozen return contract at `:3235-3279`.

**BEFORE:**
```shape
before(target, args, ctx) {
  { args: [args[0] + 1], result: null, state: { retries: 1 } }
}
```
**AFTER:**
```shape
before<Sig>(args: ArgumentPack<Sig>, ctx: BeforeContext<RetryState>)
    -> HookDecision<Sig, RetryState> {
  Proceed(args.with(#value, args.get(#value) + 1), RetryState { retries: 1 })
}
```
**Primary:** missing `HookPlan<Sig, State>`. **Dependencies:** accepted
`ArgumentPack`, `HookDecision`, typed state, signature specialization.
**Delete when:** no hook field is `Any`, no shape test selects semantics, `null`
is data only, and VM/JIT consume the same typed plan.

### U14. Stdlib source generation and template name matching

**Paths/callers:** `serde/serialize.shape:25-56` compares field type strings and
interpolates field/target names, member access, and a method body into source.
`serde/derive.shape:33-96` compares type/annotation strings, reads untyped args,
and generates `{target.name}_json_schema`. `llm/tools.shape:29-68` reads
parameter name/type strings and generates `{target.name}_tool_def`;
`llm/tools.shape:75-93` splits prompts at braces and matches placeholder spelling
to parameter spelling.

**BEFORE:**
```shape
extend (f"extend {target.name} \{ method to_json() -> string \{ {body} \} \}")
for p in target.params { if placeholder == p.name { ... } }
```
**AFTER:**
```shape
let members = target.owner_type.fields.map(|f| {
  json.member(CheckedExpr.literal(f.display_name), quote expr { self.$f.symbol.to_json() })
})
ctx.rewrite.extend_type(target.owner_type,
  quote item { method to_json() -> string { return $json.object(members) } })
let prompt: CheckedTemplate<Sig> = ctx.template(template).bind(target.params)
```
**Primary:** `CheckedItem`; the template caller needs missing
`CheckedTemplate<Sig>`. **Dependencies:** U04/U07/U08, checked field access, JSON
expression builders, hygienic sibling symbols, placeholder-to-`ParamId`
resolution. **Delete when:** these files contain no generated `fn`/`extend`
source, semantic type/name comparisons, or placeholder string matching. Domain
schema JSON remains output data.

## Ordered migration

1. Add non-forgeable type, symbol, callable, item, and parameter IDs plus the
   missing frozen descriptors. Display strings remain diagnostic only.
2. Add checked quote/splice construction. Reject scope escape, type/effect
   mismatch, duplicate items, and cross-arena symbols at construction.
3. Make `RewritePlan<Sig>` the only annotation-prepass result, keyed by IDs and
   validated atomically before any registration.
4. Lower literal directives directly into plans; remove AST JSON and parser
   probes.
5. Migrate reflection, parameter edits, helper closure, and annotation lookup to
   frozen descriptors and resolved IDs.
6. Migrate `ItemFragment`, serde, LLM tools, and prompt templates to checked
   fragments; delete `string_lit` and all source overloads.
7. Specialize hooks to `HookPlan<Sig, State>` using accepted `ArgumentPack` and
   decision types; VM and JIT consume the same plan.
8. Remove the parallel AST collector, name walkers, magic schemas, aliases, and
   compatibility decoders. Add a static guard against new comptime AST JSON or
   generated-source builtins.

## Completion boundary

Migration is complete when comptime code cannot manufacture a type, symbol,
parameter, callable, field access, body, item, or module from `string` or `Any`;
all replacements are checked against the frozen signature; plan application is
atomic and single-pass; and generated code follows the same resolved metadata
path as handwritten code.
