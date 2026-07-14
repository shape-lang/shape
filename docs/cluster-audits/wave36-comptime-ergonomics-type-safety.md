# Wave-36D Comptime Ergonomics And Type-Safety Gap Scout

Date: 2026-07-10

Worker: Wave-36D comptime ergonomics/type-safety gap scout

Scope: static inspection only. Read sources were the compiler comptime and
annotation-lowering paths, `tools/shape-test/tests/{comptime,annotations_comptime}/`,
the sibling book pages `advanced/comptime.mdx` and `advanced/annotations.mdx`,
stdlib comptime derive examples, and previous cluster-audit reports. I did not
run cargo, just, rustc, nextest, build, tests, extractor, or book-truth gates.

Book authority for this scout: supervisor context says the current verified
Wave-35 state is 707 total / 557 runnable / 150 disabled. The local worktree was
already heavily dirty; this report treats current files as evidence and writes
only this audit document.

## Executive Answer

Comptime works today for real programs, but it is not yet a hygienic, typed macro
system.

What works and feels reasonably ergonomic:

- Plain `comptime { ... }` expressions and top-level side-effect blocks are
  runnable/book-backed (`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:46`,
  `:58`; `tools/shape-test/tests/comptime/blocks.rs:13`, `:24`, `:83`).
- `comptime fn` helpers are usable for string/int/bool computations, chaining,
  multiple params, no params, recursion, and trait checks
  (`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:95`;
  `tools/shape-test/tests/comptime/functions.rs:13`, `:68`, `:107`, `:124`,
  `:139`, `:158`).
- Function/type/module annotation transforms work for the supported targets:
  function wrappers, `extend target`, `replace body`, `replace module`, `remove
  target`, `set param`, and `set return`
  (`../shape-web/book/book-site/src/content/docs/advanced/annotations.mdx:53`,
  `:109`, `:131`; `tools/shape-test/tests/annotations_comptime/code_gen.rs:8`,
  `:56`; `tools/shape-test/tests/annotations_comptime/on_define.rs:8`, `:77`,
  `:154`; `tools/shape-test/tests/annotations_comptime/directives.rs:8`,
  `:146`).
- Read-side reflection is now meaningfully typed. `__ComptimeTypeRef` is exposed
  beside legacy strings on `field.type_ref`, `param.type_ref`,
  `target.return_type_ref`, and `type_info(T).type_ref`
  (`crates/shape-vm/src/compiler/comptime_target.rs:106`, `:210`, `:450`,
  `:463`; `crates/shape-vm/src/compiler/comptime_builtins.rs:988`).
- TypeRef values can drive type directives: `set return (target.params[0].type_ref)`
  and `set param left: (target.params[1].type_ref)` have active tests
  (`tools/shape-test/tests/annotations_comptime/type_mutation.rs:290`, `:313`).

What remains awkward or stringly:

- The directive system is structured after parsing, but source-level directives
  still cross the comptime mini-VM boundary through internal `__emit_*` calls
  whose payloads are strings, JSON AST strings, or source strings
  (`crates/shape-vm/src/compiler/statements.rs:593`, `:626`, `:659`, `:680`;
  `crates/shape-vm/src/compiler/comptime_builtins.rs:233`, `:317`, `:335`).
- Computed code generation is explicitly source-string based. `extend (expr)`
  parses a string of top-level Shape source; `replace module (expr)` parses a
  source or JSON module payload (`../shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:189`,
  `:210`; `crates/shape-vm/src/compiler/comptime_builtins.rs:760`, `:776`).
- Staged outputs are strict after they are parsed. That is good for safety, but
  bad for authoring ergonomics: failures arrive as parser/source-payload errors,
  not typed-fragment construction errors (`tools/shape-test/tests/annotations_comptime/directives.rs:220`).

## Working And Runnable / Book-Backed

### Blocks, helpers, and builtins

The book has runnable examples for a compile-time expression block, top-level
warning block, `comptime fn`, type-level comptime fields, and field annotation
inspection (`advanced/comptime.mdx:46`, `:58`, `:95`, `:311`, `:345`). The scope
violation example at `advanced/comptime.mdx:76` is now an expected-fail snippet,
not an ordinary disabled row.

Active tests cover the same surface:

- Literal, nested, conditional, complex-expression, warning, and `build_config`
  blocks: `tools/shape-test/tests/comptime/blocks.rs:13`, `:24`, `:39`, `:55`,
  `:83`, `:94`, `:107`.
- `comptime fn` helper calls, string operations, recursion, and runtime-call
  rejection: `tools/shape-test/tests/comptime/functions.rs:13`, `:68`, `:139`,
  `:187`.
- LSDS-routed `error()` diagnostics under VM and JIT:
  `tools/shape-test/tests/comptime/flagship_wf3d.rs:96`.

### Typed reflection and TypeRef

`ComptimeTarget` builds typed target descriptors, not loose maps. Field
descriptor rows include `name`, legacy `type`, `annotations`, `optional`, and
typed `type_ref`; param rows include `type_ref`; function targets include
`return_type_ref` (`crates/shape-vm/src/compiler/comptime_target.rs:178`,
`:202`, `:439`, `:463`). `type_info(T)` uses the same field row builder and adds
`type_ref` to `TypeInfo` (`crates/shape-vm/src/compiler/comptime_builtins.rs:581`,
`:968`, `:988`).

Active tests prove the public shape:

- `type_info(Profile).fields[i].type_ref` plus legacy `.type` compatibility:
  `tools/shape-test/tests/comptime/type_info_chained.rs:71`.
- Function `target.params[i].type_ref` and `target.return_type_ref`:
  `tools/shape-test/tests/annotations_comptime/type_mutation.rs:262`.
- TypeRef-driven `set return` and `set param` expressions:
  `tools/shape-test/tests/annotations_comptime/type_mutation.rs:290`, `:313`.

### Annotation-driven transforms

The function/type/module examples in `advanced/annotations.mdx` are runnable at
lines 53, 109, and 131. The tests cover more than the book examples:

- `extend target` method generation and stacked type annotations:
  `tools/shape-test/tests/annotations_comptime/code_gen.rs:8`, `:32`, `:83`,
  `:111`.
- `replace body` for functions:
  `tools/shape-test/tests/annotations_comptime/code_gen.rs:56` and
  `tools/shape-test/tests/annotations_comptime/on_define.rs:77`.
- `set param` defaults now update the public call surface for omitted args,
  explicit overrides, and scalar defaults:
  `tools/shape-test/tests/annotations_comptime/directives.rs:8`, `:31`, `:77`,
  `:100`, `:123`.
- `replace module` is live and re-enters ordinary typing for generated items:
  `tools/shape-test/tests/annotations_comptime/directives.rs:146`, `:169`,
  `:196`, `:220`; compiler application path at
  `crates/shape-vm/src/compiler/statements.rs:5470`.

### Stdlib/userland derive patterns

The stdlib already ships real comptime patterns:

- `@json_schema` uses `target.fields`, `field.optional`, field annotations, and
  `field.type_ref.kind`, then emits a generated function through `extend (expr)`
  (`crates/shape-runtime/stdlib-src/serde/derive.shape:33`, `:40`, `:43`, `:91`).
- `@to_json`, `@llm_tool`, and `@prompt` are covered under VM/JIT or ordinary
  ShapeTest expectations (`tools/shape-test/tests/annotations_comptime/showcases.rs:42`,
  `:89`, `:118`, `:155`, `:174`).

## Working But Awkward / Stringly / Directive-Based

This is the important type-safety boundary.

- `extend (expr)` is string-generated source. The book says it takes a computed
  string, and the compiler builtin receives `items_payload: string`
  (`advanced/comptime.mdx:212`; `crates/shape-vm/src/compiler/comptime_builtins.rs:776`).
  WF-3D flagship F1 and F4 still use f-string source generation even though they
  are VM/JIT green (`tools/shape-test/tests/comptime/flagship_wf3d.rs:41`,
  `:125`).
- `replace module (expr)` is working but source-text backed. The malformed-source
  test intentionally expects `invalid replacement module payload`
  (`tools/shape-test/tests/annotations_comptime/directives.rs:220`).
- `replace body { ... }` is author-friendly inline syntax, but internally the
  body is serialized and re-parsed as an internal directive payload
  (`crates/shape-vm/src/compiler/statements.rs:680`;
  `crates/shape-vm/src/compiler/comptime_builtins.rs:744`).
- `type_info(User)` and `implements(Dog, Speak)` accept ergonomic bare type
  identifiers, but the compiler rewrites them to string literals before builtin
  dispatch (`crates/shape-vm/src/compiler/comptime.rs:370`).
- Annotation metadata remains partially lossy. Field annotation args are stored
  as `Vec<String>` / `args` arrays of strings
  (`crates/shape-vm/src/compiler/comptime_target.rs:48`, `:185`), and
  `@json_schema` reads `ann.args[0]` as a string
  (`crates/shape-runtime/stdlib-src/serde/derive.shape:58`).
- Legacy type strings remain part of the public contract for compatibility.
  `@json_schema` has moved to `field.type_ref.kind`, but `@to_json` and
  `@llm_tool` still branch on `field.type` / `p.type`
  (`crates/shape-runtime/stdlib-src/serde/serialize.shape:35`;
  `crates/shape-runtime/stdlib-src/llm/tools.shape:38`).

## Disabled Because Active Implementation Gap

Current comptime/annotation rows still disabled for implementation reasons:

| Row | Classification | Evidence |
|---|---|---|
| `advanced/annotations.mdx:73` | expression-target annotations | Book row is `runnable=false`; active test still expects `@log_expr(...)` to be rejected with `cannot be applied` (`tools/shape-test/tests/comptime/annotations.rs:475`). |
| `advanced/annotations.mdx:89` | await-expression annotations | Book row is `runnable=false`; previous triage calls out async-lowering proof/semantics as the gap (`docs/cluster-audits/wave30-disabled-comptime-extension-triage.md:45`). |
| `advanced/comptime.mdx:266` | connector-driven generated return types | The example uses DuckDB native calls and returns textual type source to `set return` (`advanced/comptime.mdx:275`, `:285`, `:300`). Prior triage classifies this as an active gap (`docs/cluster-audits/wave34-disabled-current-triage.md:124`). |
| `advanced/comptime-annotations-cookbook.mdx:31` | connector schema generation | Prior triage says it still depends on textual type/source payloads plus DuckDB probing (`docs/cluster-audits/wave30-disabled-comptime-extension-triage.md:48`). |
| `advanced/comptime-llm-patterns.mdx:170` | source-fragment generation | Prior triage classifies the row as an active gap because TypeRef did not remove the `extend (expr)` source-fragment surface (`docs/cluster-audits/wave30-disabled-comptime-extension-triage.md:52`). |

Not book-disabled, but still a surface bound: `set param extra: int` does not add
a new parameter. The active test expects `extra` to remain undefined
(`tools/shape-test/tests/comptime/annotations.rs:675`).

## Disabled Because Old Syntax / Book Rewrite

There are no old-syntax disabled rows in the two inspected sibling pages
`advanced/comptime.mdx` and `advanced/annotations.mdx`.

The current comptime/extension old-syntax row from prior triage is
`advanced/comptime-annotations-cookbook.mdx:308`: a policy-stack fragment with
undefined policies, `fetch_order`, and `id`, better converted to prose or a real
fixture (`docs/cluster-audits/wave30-disabled-comptime-extension-triage.md:50`;
`docs/cluster-audits/wave34-disabled-current-triage.md:138`).

There is also book drift, not a disabled snippet: `advanced/comptime.mdx:113-120`
still warns that applying `comptime pre/post` directives is planned for v0.4,
while current book rows and tests prove supported apply paths are live.

## Preview / Out-Of-Scope

Prior comptime/extension triage counted zero preview/out-of-scope disabled rows
in its owned comptime scope (`docs/cluster-audits/wave30-disabled-comptime-extension-triage.md:34`).
The real preview surface is design-level, not a current runnable row:

- full typed quote/quasiquote syntax,
- hygiene marks for generated identifiers,
- typed `ExprFragment` / `StmtFragment` / `ItemFragment` / `ModuleFragment`
  values across every directive,
- live DuckDB/Arrow/native connector fixtures,
- expression/await annotation runtime policy examples that depend on extension
  awaitables or live routing.

Those should not be conflated with the working annotation transform surface.

## Smallest Next Implementation Lane

Recommendation: implement typed additive item fragments for `extend (expr)`,
not a whole macro language.

The TypeRef-first lane recommended in Wave-22 has mostly landed: TypeRef
descriptors exist, `set return (expr)` consumes them, source-level
`set param name: (expr)` is wired, and `@json_schema` uses `field.type_ref.kind`.
The next smallest lane should attack the highest-frequency remaining string
chokepoint: generated functions/items emitted through `extend (expr)`.

Agent-ready scope:

1. Keep the existing compatibility path: `extend ("fn ...")` must continue to
   work.
2. Add a typed additive item carrier that decodes to the existing
   `ComptimeDirective::ExtendItems { items: Vec<Item> }`. Start with one narrow
   public shape sufficient for current real users:
   - a typed `ItemFragment` for a generated zero-arg function returning a literal
     scalar/string, with a `TypeRef` return type;
   - optionally the method equivalent for `extend {Type} { method ... }` if it
     falls out cleanly.
3. Teach `__emit_extend_items` to accept `string | ItemFragment | Array<ItemFragment>`
   instead of only `string`.
4. Migrate one current proof path off source strings. Best first target:
   WF-3D F1 generated free function, then stdlib `@json_schema` or `@llm_tool`.
   Leave `@to_json` for the second slice because it generates a runtime f-string
   method body and exercises more expression holes.
5. Keep generated items entering the same strict compile pipeline. The goal is
   not to weaken parsing/type checking; it is to make the authoring boundary
   typed before the compiler sees generated AST.

Proposed owned files for the implementation lane:

- `crates/shape-vm/src/compiler/comptime_builtins.rs`: define fragment schemas /
  constructors, decode fragment slots, and widen `__emit_extend_items`.
- `crates/shape-vm/src/compiler/statements.rs`: only if source emission needs a
  new directive arm; otherwise the existing `ExtendItemsExpr` path can continue
  to pass the expression through.
- `crates/shape-vm/src/compiler/functions_annotations.rs`: only if generated-item
  registration needs target-specific metadata fixes.
- `crates/shape-runtime/stdlib-src/serde/derive.shape`,
  `crates/shape-runtime/stdlib-src/llm/tools.shape`: migrate one real stdlib
  generator after the compiler path is green.

Proposed acceptance tests:

- `tools/shape-test/tests/comptime/flagship_wf3d.rs`: add typed-fragment variants
  of F1 and, if included, F4. Preserve VM and JIT expectations.
- `tools/shape-test/tests/annotations_comptime/code_gen.rs`: add a typed
  generated free-function test that does not assemble `fn ...` with an f-string.
- `tools/shape-test/tests/annotations_comptime/directives.rs`: keep string
  compatibility and add negative coverage where malformed identifiers/types fail
  at fragment construction, not as `invalid replacement module payload`.
- `tools/shape-test/tests/annotations_comptime/showcases.rs`: migrate one stdlib
  showcase only after the focused fragment tests pass.

Non-goals for this lane:

- no expression/await annotation target work,
- no live DuckDB/Arrow/native connector fixture,
- no full quote/splice syntax,
- no full hygiene system,
- no typed `replace module` until additive item fragments are proven.

Why this lane is the smallest useful one:

- It reuses the existing post-fragment strict compile path and the current
  `ExtendItems` directive.
- It removes the source-string API from the most visible success stories without
  redesigning every directive.
- It gives future quasiquote/hygiene work a typed carrier to build on.

## Static Checks

Planned static check after writing: `git diff --check -- docs/cluster-audits/wave36-comptime-ergonomics-type-safety.md`.
