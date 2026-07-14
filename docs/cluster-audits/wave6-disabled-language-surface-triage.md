# Wave 6 disabled language-surface triage

Date: 2026-07-09

Scope: disabled snippets from `/tmp/shape-async-snippets/manifest.json` for:

- `fundamentals/content.mdx`, `strings.mdx`, `error-handling.mdx`,
  `modules.mdx`, `traits.mdx`, `enums.mdx`, `tables.mdx`,
  `references-borrowing.mdx`, `functions.mdx`, `datetime.mdx`
- `advanced/ownership-deep-dive.mdx`, `annotations.mdx`,
  `native-c-interop.mdx` where the disabled snippet is about language surface
  rather than external OS/native runtime state.

Fresh manifest totals: 756 snippets, 462 runnable, 294 disabled. This report
covers all 77 disabled snippets in the requested pages.

Classification key:

- `stale_disabled_candidate`: likely current implementation exists; next worker
  should try flipping or rewrite into a self-contained smoke.
- `active_feature_gap`: intended language feature but not implemented or not
  fully wired.
- `old_syntax_or_policy_rewrite`: example uses syntax or a surface we should
  replace rather than implement.
- `preview_or_out_of_scope`: future/extension/external-runtime/conceptual or
  diagnostic-only example that should stay non-runnable unless split.

## Summary

| Page | Disabled | stale_disabled_candidate | active_feature_gap | old_syntax_or_policy_rewrite | preview_or_out_of_scope |
|---|---:|---:|---:|---:|---:|
| `advanced/annotations.mdx` | 9 | 7 | 0 | 0 | 2 |
| `advanced/native-c-interop.mdx` | 3 | 1 | 0 | 0 | 2 |
| `advanced/ownership-deep-dive.mdx` | 10 | 6 | 1 | 0 | 3 |
| `fundamentals/content.mdx` | 9 | 4 | 0 | 1 | 4 |
| `fundamentals/datetime.mdx` | 3 | 3 | 0 | 0 | 0 |
| `fundamentals/enums.mdx` | 5 | 4 | 0 | 0 | 1 |
| `fundamentals/error-handling.mdx` | 6 | 2 | 3 | 0 | 1 |
| `fundamentals/functions.mdx` | 3 | 1 | 0 | 0 | 2 |
| `fundamentals/modules.mdx` | 6 | 2 | 0 | 2 | 2 |
| `fundamentals/references-borrowing.mdx` | 5 | 0 | 1 | 1 | 3 |
| `fundamentals/strings.mdx` | 7 | 6 | 0 | 1 | 0 |
| `fundamentals/tables.mdx` | 5 | 1 | 2 | 1 | 1 |
| `fundamentals/traits.mdx` | 6 | 0 | 5 | 0 | 1 |
| **Total** | **77** | **37** | **12** | **6** | **22** |

High-signal observations:

- Content builders, color/chart constants, DateTime constructors/methods, enum
  matching/display, f-string specs, annotation target resolution, and several
  concurrency primitive methods now have source support. Many disabled fences are
  stale fragments rather than real gaps.
- Trait/module syntax gaps are concentrated in named impl call-site dispatch,
  generic trait arg erasure, target-side conversion dispatch, associated type
  substitution, and module examples using unstable `state::capture`.
- Result/Option patterns are split: `!!` examples are stale fragments; custom
  `From`/`TryFrom` and `TryInto` conversion examples are active `Convert`
  opcode/trait-dispatch gaps.
- Typed object/field mutation is not directly exercised by these disabled
  language pages. The relevant nearby surface is Option/Result carrier and
  field-kind handling; no disabled snippet here is a typed object field mutation
  flip candidate.

## Page triage

### `advanced/annotations.mdx`

Counts: 9 disabled, 7 stale, 2 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `stale_disabled_candidate` | `D__advanced__annotations__1__L65.shape` L65, `D__advanced__annotations__2__L74.shape` L74, `D__advanced__annotations__3__L80.shape` L80, `D__advanced__annotations__4__L86.shape` L86, `D__advanced__annotations__5__L95.shape` L95 | Function/expression/await/type/module annotation targets are documented as current target kinds. These snippets need self-contained local annotation definitions and small call sites. |
| `stale_disabled_candidate` | `D__advanced__annotations__7__L355.shape` L355, `D__advanced__annotations__8__L363.shape` L363 | Named-import and namespace-qualified `@remote` resolution are described as working at HEAD in the page prose. There are module-visibility tests covering `from std::core::remote use { @remote }`, namespace import, and qualified `@remote::remote`. |
| `preview_or_out_of_scope` | `D__advanced__annotations__10__L428.shape` L428 | Calls a remote worker at `worker:9527`; this is a transport/server fixture, not a pure annotation-resolution smoke. |
| `preview_or_out_of_scope` | `D__advanced__annotations__11__L456.shape` L456 | `remote::route` is explicitly extension-provided and `fetch_user(id)` is not scaffolded. Keep conceptual unless a distributed-async/extension worker owns it. |

### `advanced/native-c-interop.mdx`

Counts: 3 disabled, 1 stale, 2 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `stale_disabled_candidate` | `D__advanced__native-c-interop__2__L139.shape` L139 | The language-side `out` parameter omission path appears wired now: `functions_foreign.rs` filters `out` params from caller-visible arity and wrapper params. The example still depends on DuckDB, so the next worker should replace it with a deterministic local/libc smoke or keep it external. |
| `preview_or_out_of_scope` | `D__advanced__native-c-interop__3__L155.shape` L155 | Manual pointer-cell pattern depends on a real DuckDB shared library. The `ptr_*` helpers exist, but this snippet is external native runtime scope. |
| `preview_or_out_of_scope` | `D__advanced__native-c-interop__5__L286.shape` L286 | Arrow C import needs real `ArrowSchema`/`ArrowArray` pointers plus a registered row type. Not a language-surface flip by itself. |

### `advanced/ownership-deep-dive.mdx`

Counts: 10 disabled, 6 stale, 1 active gap, 3 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `preview_or_out_of_scope` | `A__advanced__ownership-deep-dive__0__L45.shape` L45 | Conceptual storage-class examples use `parse()`, `make_buf()`, and `Channel()` without definitions. Keep as design prose or rewrite into real LSP/storage-class docs. |
| `active_feature_gap` | `A__advanced__ownership-deep-dive__1__L54.shape` L54 | Explicit storage-class pin syntax (`SharedCow Array<int>`, `Direct int`) is explicitly marked v0.4; grammar does not accept the prefixes. |
| `stale_disabled_candidate` | `A__advanced__ownership-deep-dive__2__L81.shape` L81 | Smart `var` move/clone inference is documented as current. Needs a self-contained observable version. |
| `preview_or_out_of_scope` | `A__advanced__ownership-deep-dive__5__L141.shape` L141 | Intentional use-after-move diagnostic example. It should remain disabled or move to a negative-test doc pattern. |
| `preview_or_out_of_scope` | `A__advanced__ownership-deep-dive__11__L259.shape` L259 | Desugaring pseudocode (`x.push(1) -> (&mut x).push(1)`) with undefined `x/read`. Nearby runnable reference examples already cover the surface. |
| `stale_disabled_candidate` | `A__advanced__ownership-deep-dive__13__L399.shape` L399 | Return references have current evidence in `v0.3.3-ref-ser-soundness-recheck.md`; split this into a positive runnable `-> &int` smoke and a disabled ambiguous-return diagnostic. |
| `stale_disabled_candidate` | `A__advanced__ownership-deep-dive__14__L425.shape` L425 | Wave-6 `Future<T>` work makes `async let` current. Split positive owned/shared-reference cases from the intended `&mut` task-boundary diagnostic. |
| `stale_disabled_candidate` | `A__advanced__ownership-deep-dive__15__L459.shape` L459 | Mutex methods exist, but prose is stale: current `lock()` returns the mutex as a marker/no-op and `get()` reads the inner value. Rewrite before flipping. |
| `stale_disabled_candidate` | `A__advanced__ownership-deep-dive__16__L470.shape` L470 | Atomic methods (`load`, `store`, `fetch_add`, `fetch_sub`, `compare_exchange`) are registered in method tables and handlers. Candidate for deterministic prints. |
| `stale_disabled_candidate` | `A__advanced__ownership-deep-dive__17__L483.shape` L483 | Lazy `get`/`is_initialized` handlers exist. Needs a defined initializer and deterministic output. |

### `fundamentals/content.mdx`

Counts: 9 disabled, 4 stale, 1 old/policy rewrite, 4 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `old_syntax_or_policy_rewrite` | `B__fundamentals__content__3__L70.shape` L70 | Uses retired `c"..."` content-string syntax and automatic `Content` trait dispatch. Replace with `Content.text(...).fg(...).toString()` style builders; do not revive `c"..."`. |
| `preview_or_out_of_scope` | `B__fundamentals__content__4__L80.shape` L80 | Custom `impl Content` plus `print(s)` auto-render is explicitly v0.4 preview. |
| `preview_or_out_of_scope` | `B__fundamentals__content__6__L126.shape` L126 | Auto-table rendering for struct collections is explicitly v0.4 preview. |
| `stale_disabled_candidate` | `B__fundamentals__content__7__L161.shape` L161 | `Color.*` constants and `Color.rgb` lower to style strings / RGB constructors. Rewrite into printable builder calls. |
| `stale_disabled_candidate` | `B__fundamentals__content__12__L341.shape` L341 | `Content.chart("...")` and `ChartType.line` are implemented in current content builder code. Add `.toString()` or a stable assertion output. |
| `stale_disabled_candidate` | `B__fundamentals__content__13__L377.shape` L377 | `Content.fragment` constructor exists now. Needs defined data and possibly an explicit content array if inference requires it. |
| `stale_disabled_candidate` | `B__fundamentals__content__14__L423.shape` L423 | Same fragment-builder surface; replace undefined `today()` and ensure array element type is proven. |
| `preview_or_out_of_scope` | `B__fundamentals__content__16__L463.shape` L463 | `ContentFor<Adapter>` and adapter names are explicitly v0.4 preview. |
| `preview_or_out_of_scope` | `B__fundamentals__content__18__L482.shape` L482 | Adapter-specific `ContentFor<Html/Terminal>` impls are preview and use undefined adapter names. |

### `fundamentals/datetime.mdx`

Counts: 3 disabled, 3 stale.

| Classification | Snippets | Notes |
|---|---|---|
| `stale_disabled_candidate` | `B__fundamentals__datetime__1__L23.shape` L23 | `DateTime.now()` and `DateTime.utc()` builtins are wired. Flip only with stable predicates, not raw wall-clock output. |
| `stale_disabled_candidate` | `B__fundamentals__datetime__18__L356.shape` L356 | Timezone conversion/comparison methods exist. Rewrite to avoid nondeterministic `DateTime.now()` or print deterministic predicates. |
| `stale_disabled_candidate` | `B__fundamentals__datetime__20__L392.shape` L392 | Parse/format/iso8601/unix timestamp/timezone methods have source and tests. Candidate for deterministic output. |

### `fundamentals/enums.mdx`

Counts: 5 disabled, 4 stale, 1 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `stale_disabled_candidate` | `A__fundamentals__enums__1__L40.shape` L40 | Unit-variant matching is covered elsewhere; snippet only needs the enum definition inlined. |
| `stale_disabled_candidate` | `A__fundamentals__enums__3__L68.shape` L68 | Tuple-variant matching has current coverage; inline the `Shape` enum and print deterministic values. |
| `stale_disabled_candidate` | `A__fundamentals__enums__5__L98.shape` L98 | Struct-style variant matching is present in executor tests. Inline the enum and call `handle(...)`. |
| `preview_or_out_of_scope` | `A__fundamentals__enums__8__L154.shape` L154 | Conceptual definition of builtin `Option<T>`. Do not run as a user-defined replacement for stdlib `Option`. |
| `stale_disabled_candidate` | `A__fundamentals__enums__12__L220.shape` L220 | Enum variant display is implemented; inline `Direction` and `Shape` definitions. |

### `fundamentals/error-handling.mdx`

Counts: 6 disabled, 2 stale, 3 active gaps, 1 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `preview_or_out_of_scope` | `A__fundamentals__error-handling__4__L90.shape` L90 | Conceptual `AnyError` structure with placeholder tokens like `<original value>`. It is not Shape code. |
| `active_feature_gap` | `A__fundamentals__error-handling__8__L186.shape` L186 | Source-side `impl TryInto<int> for string as int` through `Convert` remains pending trait-dispatch/value-call/AnyError work per page comment. |
| `active_feature_gap` | `A__fundamentals__error-handling__9__L207.shape` L207 | Target-side `From<number>` auto-derived `Into<Celsius>` is the known `Convert` opcode trait-dispatch surface. |
| `active_feature_gap` | `A__fundamentals__error-handling__10__L224.shape` L224 | Target-side `TryFrom<Json>` auto-derived `TryInto<string>` shares the same `Convert`/AnyError builder gap. |
| `stale_disabled_candidate` | `A__fundamentals__error-handling__11__L275.shape` L275 | `!!` itself was previously verified; this is disabled because `find_user()` is unscaffolded. Add a tiny `Result` stub. |
| `stale_disabled_candidate` | `A__fundamentals__error-handling__12__L287.shape` L287 | `!!` plus `?` parsing is current; snippet needs definitions for `value`, `other_call`, and `find_user`. |

### `fundamentals/functions.mdx`

Counts: 3 disabled, 1 stale, 2 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `stale_disabled_candidate` | `A__fundamentals__functions__12__L185.shape` L185 | `where` clause grammar is present. A runnable smoke should define `Display` and a concrete impl or keep this as parse-only prose. |
| `preview_or_out_of_scope` | `A__fundamentals__functions__15__L237.shape` L237 | Intentional negative named-argument diagnostics. Keep disabled or move to diagnostic docs. |
| `preview_or_out_of_scope` | `A__fundamentals__functions__27__L421.shape` L421 | Python extension runtime install/load is external to book-truth and not a pure language snippet. |

### `fundamentals/modules.mdx`

Counts: 6 disabled, 2 stale, 2 old/policy rewrite, 2 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `old_syntax_or_policy_rewrite` | `A__fundamentals__modules__0__L20.shape` L20, `A__fundamentals__modules__1__L28.shape` L28 | `use std::core::state` / `state::capture()` is a bad module-system example: state capture has frame/snapshot constraints and historical VM/JIT divergence. Replace with a stable stdlib namespace example. |
| `stale_disabled_candidate` | `A__fundamentals__modules__4__L48.shape` L48, `A__fundamentals__modules__5__L57.shape` L57 | Annotation imports and namespace-qualified annotation resolution have current tests. Keep calls out unless a remote server fixture is owned. |
| `preview_or_out_of_scope` | `A__fundamentals__modules__7__L74.shape` L74 | External `mylib` namespace alias plus undefined `input`; this is project-layout prose, not a standalone snippet. |
| `preview_or_out_of_scope` | `A__fundamentals__modules__12__L185.shape` L185 | Multi-file library example depends on sibling files outside the extracted snippet. Keep disabled unless book-truth gains multi-file fixtures. |

### `fundamentals/references-borrowing.mdx`

Counts: 5 disabled, 1 active gap, 1 old/policy rewrite, 3 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `preview_or_out_of_scope` | `A__fundamentals__references-borrowing__1__L30.shape` L30 | Intentional use-after-move diagnostic. Current behavior should be covered by negative tests, not runnable book-truth. |
| `active_feature_gap` | `A__fundamentals__references-borrowing__4__L73.shape` L73 | `var` alias plus `.push` copy-on-write path has known VM/JIT issues in prior G.1 audit. This remains a real ownership/CoW gap. |
| `preview_or_out_of_scope` | `A__fundamentals__references-borrowing__11__L192.shape` L192 | Mixed negative/positive reference-escape example. Keep negative disabled; split `read_val(&x)` if a positive smoke is useful. |
| `preview_or_out_of_scope` | `A__fundamentals__references-borrowing__13__L253.shape` L253 | Mixed async task-boundary rules plus undefined functions. Split positive owned/shared-ref cases from the intended `&mut` diagnostic. |
| `old_syntax_or_policy_rewrite` | `A__fundamentals__references-borrowing__14__L269.shape` L269 | Quick-reference table uses placeholders and literal `...`. Keep as prose/table, not a runnable Shape block. |

### `fundamentals/strings.mdx`

Counts: 7 disabled, 6 stale, 1 old/policy rewrite.

| Classification | Snippets | Notes |
|---|---|---|
| `stale_disabled_candidate` | `A__fundamentals__strings__0__L30.shape` L30 | Simple, triple, `f`, `f$`, and `f#` string forms are in grammar/parser tests. Add concrete variables and output if flipping. |
| `stale_disabled_candidate` | `A__fundamentals__strings__8__L164.shape` L164, `A__fundamentals__strings__9__L174.shape` L174 | Braces-mode f-strings are current; snippets are disabled because variables are undefined. |
| `stale_disabled_candidate` | `A__fundamentals__strings__11__L199.shape` L199 | Sigil modes are in grammar/parser tests. Needs concrete `user`/`path`. |
| `stale_disabled_candidate` | `A__fundamentals__strings__13__L231.shape` L231, `A__fundamentals__strings__14__L247.shape` L247 | `table(...)` interpolation format specs are parsed and compiled. Provide real rows/results and deterministic rendered checks. |
| `old_syntax_or_policy_rewrite` | `A__fundamentals__strings__18__L333.shape` L333 | Explicitly retired `c"..."` inline styling syntax. Replace with f-string specs or `Content.*` builders; do not implement old syntax. |

### `fundamentals/tables.mdx`

Counts: 5 disabled, 1 stale, 2 active gaps, 1 old/policy rewrite, 1 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `stale_disabled_candidate` | `B__fundamentals__tables__2__L31.shape` L31 | Compiler code has `Table<T>` row-literal lowering. Candidate for a focused table-row smoke with inline `Event` type/imports. |
| `preview_or_out_of_scope` | `B__fundamentals__tables__3__L48.shape` L48 | `load_events("/tmp/events.csv")` is an app-specific external loader plus filesystem input. |
| `active_feature_gap` | `B__fundamentals__tables__4__L68.shape` L68 | `from row in events where ... select ...` query syntax is explicitly v0.4 preview in the page. |
| `active_feature_gap` | `B__fundamentals__tables__5__L101.shape` L101 | Table method chaining is still a partial surface; the documented `orderBy(|row| row.value, "desc")` shape does not match the current core `table_methods.shape` one-arg signature. |
| `old_syntax_or_policy_rewrite` | `B__fundamentals__tables__6__L117.shape` L117 | Trait body uses old pseudo-signature syntax (`filter(predicate): any,`). Current `Queryable<T>` uses `method filter(predicate) -> Self;`. Rewrite rather than implement this form. |

### `fundamentals/traits.mdx`

Counts: 6 disabled, 5 active gaps, 1 preview/out-of-scope.

| Classification | Snippets | Notes |
|---|---|---|
| `active_feature_gap` | `A__fundamentals__traits__3__L71.shape` L71 | Named impl declarations parse, but `using ImplName` call-site dispatch still hits the `WrapTypeAnnotation`/deleted-ValueWord surface per page note. |
| `active_feature_gap` | `A__fundamentals__traits__8__L172.shape` L172 | Generic trait impl-side dispatch is blocked by generic trait arg erasure. |
| `active_feature_gap` | `A__fundamentals__traits__11__L249.shape` L249, `A__fundamentals__traits__12__L265.shape` L265 | User-defined `From`/`TryFrom` auto-derived conversion through `as`/`as Type?` shares the `Convert` opcode trait-dispatch/AnyError gap. |
| `preview_or_out_of_scope` | `A__fundamentals__traits__14__L330.shape` L330 | Illustrative `extend Table<Row>` pseudo-code needs row-spread, row indexing, rolling windows, and a real table source. |
| `active_feature_gap` | `A__fundamentals__traits__17__L387.shape` L387 | Associated type declarations/bindings parse, but end-to-end associated-type substitution in impl method return positions shares the generic-impl resolution gap. |

## Next recommended wave

1. Content/DateTime stale flips.
   Own only `fundamentals/content.mdx` and `fundamentals/datetime.mdx`.
   Convert Color/ChartType/fragment examples to deterministic builder smokes,
   and flip DateTime examples using fixed timestamps or stable predicates.

2. Enum/string stale flips.
   Own only `fundamentals/enums.mdx` and `fundamentals/strings.mdx`.
   Inline missing enum/type/value scaffolding, keep builtin `Option<T>` and
   retired `c"..."` blocks disabled, and use deterministic f-string outputs.

3. Annotation/module import smoke.
   Own only `advanced/annotations.mdx` and `fundamentals/modules.mdx`.
   Build self-contained local annotation target examples and remote annotation
   import-resolution examples without calling a remote worker. Replace
   `state::capture` module examples with a stable stdlib namespace import.

4. Ownership/concurrency doc refresh.
   Own only `advanced/ownership-deep-dive.mdx` and
   `fundamentals/references-borrowing.mdx`.
   Update Mutex docs to current `lock`/`get` semantics, add positive smokes for
   Atomic/Lazy and split mixed diagnostic examples. Leave `var` alias CoW as an
   active implementation gap.

5. Trait/conversion gap tickets.
   Own `fundamentals/traits.mdx` and `fundamentals/error-handling.mdx` only.
   Do not flip the conversion/named-impl/generic-trait/associated-type examples
   until `Convert`, kinded trait dispatch, and generic impl resolution are fixed.
   Add explicit disabled reasons if the prose is not already clear.

6. Tables/native C bounded follow-up.
   Own `fundamentals/tables.mdx` plus only the language-side `out` snippet in
   `advanced/native-c-interop.mdx`.
   Try a focused `Table<T>` row-literal smoke and rewrite `Queryable` pseudo
   syntax. Keep external loaders, DuckDB, and Arrow pointer examples disabled
   unless a fixture story exists.
