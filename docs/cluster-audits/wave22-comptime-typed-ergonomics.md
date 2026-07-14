# Wave-22D Comptime Typed Ergonomics Scout

Date: 2026-07-09

Worker: Wave-22D comptime typed-ergonomics scout

Scope: static inspection only over comptime/annotation lowering, comptime
builtins, annotation tests, sibling comptime book snippets, and prior comptime
proof docs. I did not run cargo, just, nextest, rustc, build, test, or
book-truth commands. The worktree was already heavily dirty; this report treats
current files as the truth and does not revert or normalize any unrelated edit.

## Executive Truth

Comptime is now much more typed than the older Wave-12/WF-1B reports described,
but it is not a typed macro system.

Current typed surfaces:

- `ComptimeExecutionResult.value` is a `KindedSlot`, not the deleted untyped
  value carrier (`crates/shape-vm/src/compiler/comptime.rs:55`).
- Annotation handler `target` and `ctx` params are assigned object
  `TypeAnnotation`s before analysis, with typed `target.fields`,
  `target.params`, and `{ module_path, file }` context fields
  (`crates/shape-vm/src/compiler/comptime.rs:94`,
  `crates/shape-vm/src/compiler/comptime.rs:155`,
  `crates/shape-vm/src/compiler/comptime.rs:1034`,
  `crates/shape-vm/src/compiler/comptime.rs:1120`).
- `ComptimeTarget` builds named-schema typed objects and typed arrays for field
  and param descriptor rows. `target.fields` and `type_info(T).fields` share
  the same `build_field_descriptor_array` implementation
  (`crates/shape-vm/src/compiler/comptime_target.rs:106`,
  `crates/shape-vm/src/compiler/comptime_target.rs:291`,
  `crates/shape-vm/src/compiler/comptime_target.rs:364`).
- `build_config()` and `type_info()` return typed objects built through reserved
  named schemas, and current WF-3D tests assert `type_info(T).fields[i].name`,
  generated free functions, LSDS diagnostics, and generated methods under VM
  and JIT (`crates/shape-vm/src/compiler/comptime_builtins.rs:399`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:488`,
  `tools/shape-test/tests/comptime/flagship_wf3d.rs:20`).
- Stdlib `@json_schema`, `@to_json`, `@llm_tool`, and `@prompt` now exercise
  real userland/stdlib comptime patterns against the public contract
  (`tools/shape-test/tests/annotations_comptime/showcases.rs:18`,
  `crates/shape-runtime/stdlib-src/serde/derive.shape:30`,
  `crates/shape-runtime/stdlib-src/llm/tools.shape:24`).

Current string/directive leaks:

- Reflection still exposes type identity as strings. `ComptimeTarget` stores
  field and param type strings, stringifies annotation args lossily, and
  `type_info` returns string `name`, string `kind`, and field rows whose `type`
  is a string (`crates/shape-vm/src/compiler/comptime_target.rs:157`,
  `crates/shape-vm/src/compiler/comptime_target.rs:414`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:851`).
- Bare `type_info(Point)` and `implements(Dog, Speak)` are compiler-rewritten
  to string literals at the builtin boundary
  (`crates/shape-vm/src/compiler/comptime.rs:323`,
  `crates/shape-vm/src/compiler/comptime.rs:402`).
- Directive payloads still cross the mini-VM boundary as strings. Even already
  parsed `set return Type`, `set param x: Type`, `replace body { ... }`, and
  `extend target { ... }` are serialized to JSON strings before the internal
  `__emit_*` builtins parse them back (`crates/shape-vm/src/compiler/statements.rs:593`,
  `crates/shape-vm/src/compiler/statements.rs:626`,
  `crates/shape-vm/src/compiler/statements.rs:643`,
  `crates/shape-vm/src/compiler/statements.rs:664`).
- Computed generation remains explicitly source-string based: `replace module
  (expr)` and `extend (expr)` evaluate to source/JSON strings, then parse into
  `Vec<Item>` (`crates/shape-vm/src/compiler/comptime_builtins.rs:271`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:661`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:677`).

So the honest current statement is: comptime execution and descriptor carriers
are typed; comptime authoring and code generation are still directive and
source-string oriented.

## Set Param And Replace Module

The Wave-12 claim that `set param` cannot update the public function call
surface is stale.

What is current:

- `SetParamValue` carries the computed default as a `KindedSlot`
  (`crates/shape-vm/src/compiler/comptime_builtins.rs:136`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:591`).
- The compiler converts directive defaults back to scalar AST literals for
  `int`, `number`, `bool`, `string`, and `none`
  (`crates/shape-vm/src/compiler/functions_annotations.rs:257`).
- During function compilation, handlers run before body compilation; if params
  or return type changed, the ordinary type analyzer is re-run with the mutated
  signature (`crates/shape-vm/src/compiler/functions.rs:840`,
  `crates/shape-vm/src/compiler/functions_annotations.rs:2230`).
- Public metadata is refreshed after directives, including function definitions,
  arity bounds, const param indexes, and default-aware required-param counts
  (`crates/shape-vm/src/compiler/functions.rs:864`,
  `crates/shape-vm/src/compiler/statements.rs:1232`).
- Active tests cover omitted-arg defaults, explicit-arg override, unknown-param
  diagnostics, and non-int scalar defaults
  (`tools/shape-test/tests/annotations_comptime/directives.rs:8`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:31`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:55`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:77`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:100`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:123`).

Remaining bound: `set param x: Type` only updates the type of an existing
parameter. It does not create a new parameter; the active test still expects
`extra` to be undefined (`tools/shape-test/tests/comptime/annotations.rs:675`).

`replace module` re-analysis is also current.

- Module-target handlers can replace `module_items`; the compiler records that
  replacement happened (`crates/shape-vm/src/compiler/statements.rs:5449`).
- Before compiling replaced items, `recheck_replaced_module_items` patches a
  clone of the analysis program and calls `analyze_program_full`
  (`crates/shape-vm/src/compiler/statements.rs:5665`).
- Module compilation invokes that recheck before registering/compiling the
  replacement items (`crates/shape-vm/src/compiler/statements.rs:5993`).
- Active tests cover source-string replacement, generated-source type errors,
  wrong-target rejection, and malformed source payload diagnostics
  (`tools/shape-test/tests/annotations_comptime/directives.rs:146`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:169`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:196`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:220`).

Remaining bound: `replace module` is still a source/JSON payload surface, not a
typed `ModuleFragment` value.

## Book And Doc Drift

The sibling book documents the source-string surface honestly in places:
`replace module (expr)` is a module-source payload, `set return (expr)` accepts
serialized `TypeAnnotation` JSON or textual type source, and `extend (expr)`
takes a computed string of Shape source
(`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:168`,
`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:191`,
`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:210`).

But it is stale or incomplete in several comptime-specific ways:

- The builtins table omits `type_info`, even though source and WF-3D tests treat
  it as current (`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:151`,
  `tools/shape-test/tests/comptime/flagship_wf3d.rs:66`).
- The target-field table omits `optional`, while source and stdlib derives use
  it (`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:138`,
  `crates/shape-vm/src/compiler/comptime_target.rs:121`,
  `crates/shape-runtime/stdlib-src/serde/derive.shape:79`).
- The examples chapter still warns that typed target-field iteration hits the
  old `comptime_target::nb_object_array` blocker
  (`../shape-web/book/book-site/src/content/docs/examples/comptime-codegen.mdx:12`).
  Current source builds typed object arrays for target descriptors, and stdlib
  showcases use ordinary `for field in target.fields` inside comptime handlers.
- Connector examples remain disabled and string-return oriented: a comptime
  schema helper returns textual type source for `set return`
  (`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:266`,
  `../shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:31`).

`comptime for` nuance: inside the comptime mini-VM, `compile_comptime_for`
rewrites it to a normal runtime `for` expression so handler-scope bindings like
`target` are visible. Outside comptime mode, the unroll path is still dormant
and returns a structured semantic error
(`crates/shape-vm/src/compiler/expressions/misc.rs:821`,
`crates/shape-vm/src/compiler/expressions/misc.rs:854`).

## Recommended First Implementation Lane

First lane: TypeRef-first typed reflection and type-directive payloads.

Do not start with a full quasiquote/hygiene syntax. The smallest useful slice is
to replace the type-string seam that all current derives already depend on,
while leaving `extend (expr)` source strings as compatibility.

Deliverables:

1. Add a reserved `__ComptimeTypeRef` typed descriptor and expose it alongside
   existing string fields: `field.type_ref`, `param.type_ref`,
   `target.return_type_ref`, and `type_info(T).type_ref` or equivalent. Keep
   `.type` / `.return_type` string fields for compatibility and diagnostics.
2. Teach `set return (expr)` and `set param name: expr` to accept a TypeRef
   value directly, not only serialized `TypeAnnotation` JSON or textual type
   source. Internally, this should still end in the existing structured
   `ComptimeDirective::SetReturnType` / `SetParamType`.
3. Preserve annotation args as typed values in descriptor rows, at least for
   scalar literals, while retaining a `.display`/`.source` string for old code.
4. Rewrite one real stdlib derive path to use `field.type_ref.kind` or an
   equivalent typed discriminator instead of `field.type == "int"` string
   comparisons. `@json_schema` is the best first acceptance target because it is
   already gate-covered under VM and JIT.
5. Add focused tests proving `type_info(Point).fields[0].type_ref`, optional
   fields, `set return (some_type_ref)`, and old `.type` string compatibility.

Why this lane first:

- It reuses the typed descriptor and named-schema machinery that is already
  current.
- It removes the most common author-visible string leak without designing a
  macro language.
- It gives a clean stepping stone to `ItemFragment` / `ModuleFragment` values:
  once TypeRef is a real comptime value, typed fragments can use TypeRef holes
  instead of interpolated strings.

Next lane after that: introduce typed `ExprFragment`, `StmtFragment`,
`ItemFragment`, and `ModuleFragment` carriers for directive payloads, route
existing inline directives away from JSON string round-trips, then add a small
hygienic quote/splice surface. That should follow TypeRef, not precede it,
because hygienic generated code needs typed holes before it needs quote syntax.

## Residual Risks

- This report did not execute the active tests. It relies on current source and
  active test definitions only.
- Some comments in older test/docs still describe pre-WF-3D behavior. I used
  current source plus active test bodies when comments contradicted code.
- `ConstantValue` exists as a typed comptime constant sketch, but it is not a
  current public comptime carrier: the module is marked `allow(dead_code)` and
  `rg` only found TODO references outside the file itself.
