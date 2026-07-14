# Wave 30D Disabled Comptime/Extension Book Triage

Manifest authority:
`/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`,
generated `2026-07-09T23:40:40.617Z`.

Global manifest state verified by static JSON read: 707 total / 541 runnable /
166 disabled / 0 deferred.

Owned scope after supervisor correction:

- `advanced/annotations.mdx`
- `advanced/comptime.mdx`
- `advanced/comptime-annotations-cookbook.mdx`
- `advanced/comptime-llm-patterns.mdx`
- `advanced/native-c-interop.mdx`
- `tooling/extensions.mdx`
- `tooling/polyglot.mdx`
- `tooling/python-extension.mdx`
- `tooling/typescript-extension.mdx`

Explicit exclusion: `advanced/polyglot-distributed.mdx` disabled rows are not
counted here. The current disabled rows at L74, L149, and L212 are distributed
composition rows owned by Wave-30B. This report only uses their extension
runtime implications when they overlap the tooling/polyglot pages.

## Bucket Counts

| Bucket | Count |
|---|---:|
| active implementation gap | 6 |
| external/manual/fixture/server/env/permission dependent | 22 |
| proof/design gap | 0 |
| preview/out-of-scope | 0 |
| intentional diagnostic | 1 |
| stale-green/count-reduction candidate | 0 |
| old syntax/book rewrite | 1 |
| **Total owned disabled snippets** | **30** |

## Classifications

| Bucket | Page / line / snippet id | Reason |
|---|---|---|
| active implementation gap | `advanced/annotations.mdx:L73` `D__advanced__annotations__2__L73.shape` | Expression-target runtime annotations remain disabled; parser/model knows `expression`, but this needs current compile/lowering proof and book-gate coverage. |
| active implementation gap | `advanced/annotations.mdx:L89` `D__advanced__annotations__3__L89.shape` | Await-expression annotation target crosses async lowering; needs current `await_expr` before/after semantics proven, not just target parsing. |
| external/manual/fixture/server/env/permission dependent | `advanced/annotations.mdx:L480` `D__advanced__annotations__10__L480.shape` | `@remote("worker:9527")` requires a live `shape serve` receiver and remote execution fixture. |
| external/manual/fixture/server/env/permission dependent | `advanced/annotations.mdx:L508` `D__advanced__annotations__11__L508.shape` | `@host` example depends on extension-provided `remote::route`, an awaited function, and runtime routing fixture. |
| active implementation gap | `advanced/comptime-annotations-cookbook.mdx:L31` `D__advanced__comptime-annotations-cookbook__0__L31.shape` | Connector-driven type generation still relies on textual type/source payloads plus DuckDB schema probing; needs a real typed connector schema lane. |
| external/manual/fixture/server/env/permission dependent | `advanced/comptime-annotations-cookbook.mdx:L183` `D__advanced__comptime-annotations-cookbook__3__L183.shape` | Await host routing depends on an extension `route` primitive and service fixture. |
| old syntax/book rewrite | `advanced/comptime-annotations-cookbook.mdx:L308` `D__advanced__comptime-annotations-cookbook__9__L308.shape` | This is a policy-stack fragment with undefined policies, `fetch_order`, and `id`; it is better as prose or a full fixture, not a standalone Shape snippet. |
| external/manual/fixture/server/env/permission dependent | `advanced/comptime-annotations-cookbook.mdx:L329` `D__advanced__comptime-annotations-cookbook__10__L329.shape` | Checkpoint workflow needs snapshot/resume process orchestration and real `ingest`/`normalize`/`publish` workflow functions. |
| active implementation gap | `advanced/comptime-llm-patterns.mdx:L170` `E__advanced__comptime-llm-patterns__4__L170.shape` | Demonstrates `extend (expr)` source-string generation; current TypeRef work does not remove the string/source-fragment surface. |
| intentional diagnostic | `advanced/comptime.mdx:L76` `D__advanced__comptime__2__L76.shape` | Negative example: comptime must not capture runtime local `marker`. Keep disabled unless the book gate gains expected-fail snippets. |
| active implementation gap | `advanced/comptime.mdx:L266` `D__advanced__comptime__6__L266.shape` | DuckDB connector return typing uses comptime native calls and textual `set return` payloads; not a current self-contained typed Shape example. |
| external/manual/fixture/server/env/permission dependent | `advanced/native-c-interop.mdx:L139` `D__advanced__native-c-interop__2__L139.shape` | Out-param sugar likely has deterministic stub coverage elsewhere, but this exact row requires DuckDB and `pricing_data.duckdb`. |
| external/manual/fixture/server/env/permission dependent | `advanced/native-c-interop.mdx:L155` `D__advanced__native-c-interop__3__L155.shape` | Manual pointer-cell pattern depends on real DuckDB shared library and database fixture. |
| external/manual/fixture/server/env/permission dependent | `advanced/native-c-interop.mdx:L286` `D__advanced__native-c-interop__5__L286.shape` | Arrow C import needs live `ArrowSchema`/`ArrowArray` pointers and a registered `MyRow` schema. |
| external/manual/fixture/server/env/permission dependent | `tooling/extensions.mdx:L120` `D__tooling__extensions__0__L120.shape` | DuckDB package query builder requires package, native dependency, and `pricing_data.duckdb` fixture. |
| external/manual/fixture/server/env/permission dependent | `tooling/polyglot.mdx:L14` `D__tooling__polyglot__0__L14.shape` | Scalar `fn python` example needs the Python extension loaded in the book gate. Existing FFI tier evidence suggests this is fixture-gated, not dead. |
| active implementation gap | `tooling/polyglot.mdx:L96` `D__tooling__polyglot__2__L96.shape` | `Result<Vec<Element>>` is the known Vec-of-struct return gap in foreign marshalling; prior audit calls it broken rather than merely ungated. |
| external/manual/fixture/server/env/permission dependent | `tooling/polyglot.mdx:L126` `D__tooling__polyglot__3__L126.shape` | Async Python example needs `aiohttp`, network access, and a real endpoint. |
| external/manual/fixture/server/env/permission dependent | `tooling/polyglot.mdx:L186` `D__tooling__polyglot__7__L186.shape` | NumPy examples need Python extension and NumPy in the active environment. |
| external/manual/fixture/server/env/permission dependent | `tooling/python-extension.mdx:L68` `D__tooling__python-extension__1__L68.shape` | Basic Python call requires built/loaded `libshape_ext_python.so`; ignored FFI e2e covers the scalar path. |
| external/manual/fixture/server/env/permission dependent | `tooling/python-extension.mdx:L117` `D__tooling__python-extension__3__L117.shape` | Inline object return requires Python extension fixture and should gain direct book-gate or FFI-tier coverage. |
| external/manual/fixture/server/env/permission dependent | `tooling/python-extension.mdx:L142` `D__tooling__python-extension__4__L142.shape` | Named alias plus async fetch needs Python extension, `aiohttp`, network, and endpoint fixture. |
| external/manual/fixture/server/env/permission dependent | `tooling/python-extension.mdx:L163` `D__tooling__python-extension__5__L163.shape` | Inline-object alias definition needs extension parsing/runtime fixture; it is definition-only and has no deterministic output. |
| external/manual/fixture/server/env/permission dependent | `tooling/python-extension.mdx:L184` `D__tooling__python-extension__6__L184.shape` | Nonconforming Python return is covered by ignored FFI e2e but still needs extension loading in the book gate. |
| external/manual/fixture/server/env/permission dependent | `tooling/python-extension.mdx:L197` `D__tooling__python-extension__7__L197.shape` | Async Python fetch hits a live external API and needs `aiohttp` plus extension runtime. |
| external/manual/fixture/server/env/permission dependent | `tooling/typescript-extension.mdx:L74` `D__tooling__typescript-extension__1__L74.shape` | Basic TypeScript call requires built/loaded `libshape_ext_typescript.so`; ignored FFI e2e covers scalar VM/JIT parity. |
| external/manual/fixture/server/env/permission dependent | `tooling/typescript-extension.mdx:L134` `D__tooling__typescript-extension__3__L134.shape` | Object return requires TypeScript extension fixture and direct marshalling coverage. |
| external/manual/fixture/server/env/permission dependent | `tooling/typescript-extension.mdx:L163` `D__tooling__typescript-extension__4__L163.shape` | Bad-return error model requires TypeScript extension fixture; analogous Python path has ignored FFI coverage. |
| external/manual/fixture/server/env/permission dependent | `tooling/typescript-extension.mdx:L180` `D__tooling__typescript-extension__5__L180.shape` | Async TypeScript fetch needs V8 extension runtime plus network/fetch fixture. |
| external/manual/fixture/server/env/permission dependent | `tooling/typescript-extension.mdx:L238` `D__tooling__typescript-extension__6__L238.shape` | Bundled namespace import needs TypeScript extension loaded and a `helpers.ts` module fixture. |

## Comptime Ergonomics And Type Safety

Typed now:

- `__ComptimeTypeRef` exists as a typed descriptor, exposed beside legacy
  string fields through `field.type_ref`, `param.type_ref`,
  `target.return_type_ref`, and `type_info(T).type_ref`.
- `set return (expr)` can consume a TypeRef expression; the internal
  `__emit_set_param_type` path also accepts a string or TypeRef.
- `serde::derive` has moved to `field.type_ref.kind` for type decisions while
  preserving legacy string compatibility.

Still string/source-fragment based:

- `extend (expr)` emits parsed Shape source strings; `string_lit` makes this
  safer but not typed or hygienic.
- `replace module (expr)` and connector examples still operate on source text.
- Source-level `set param name: (expr)` is still called out in compiler comments
  as not forwarded like `set return (expr)`.
- DuckDB/connector rows still compute textual type source such as
  `Result<Table<...>, AnyError>` rather than producing a typed schema/TypeRef
  value.

Rows reflecting this gap: `advanced/comptime-annotations-cookbook.mdx:L31`,
`advanced/comptime.mdx:L266`, and
`advanced/comptime-llm-patterns.mdx:L170`.

## Priority Lanes

1. Annotation expression/await target lane.
   Suggested files/tests: `crates/shape-vm/src/compiler/functions_annotations.rs`,
   `crates/shape-vm/src/compiler/statements.rs`,
   `tools/shape-test/tests/annotation_targets/other_targets.rs`,
   `tools/shape-test/tests/comptime/annotations.rs`.

2. Comptime typed generation lane.
   Suggested files/tests: `crates/shape-vm/src/compiler/comptime.rs`,
   `crates/shape-vm/src/compiler/comptime_builtins.rs`,
   `crates/shape-vm/src/compiler/comptime_target.rs`,
   `crates/shape-vm/src/compiler/statements.rs`,
   `crates/shape-vm/src/compiler/functions_annotations.rs`,
   `tools/shape-test/tests/annotations_comptime/type_mutation.rs`,
   `tools/shape-test/tests/comptime/flagship_wf3d.rs`,
   `crates/shape-runtime/stdlib-src/serde/derive.shape`,
   `crates/shape-runtime/stdlib-src/llm/tools.shape`.

3. Polyglot extension book-gate fixture lane.
   Suggested files/tests: `bin/shape-cli/tests/ffi_e2e.rs`,
   `bin/shape-cli/src/extension_loading.rs`,
   `extensions/python/src/runtime.rs`,
   `extensions/typescript/src/runtime.rs`,
   `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs`.
   Promote deterministic scalar/error rows into a book-gate-supported fixture
   before trying network, NumPy, or async endpoint examples.

4. Foreign marshalling completeness lane.
   Suggested files/tests: `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs`,
   `extensions/python/src/marshaling.rs`,
   `extensions/typescript/src/marshaling.rs`,
   `bin/shape-cli/tests/ffi_e2e.rs`.
   First concrete gap is Python `Vec<struct>` return from
   `tooling/polyglot.mdx:L96`; add object-return and alias coverage while there.

5. Native C/DuckDB/Arrow fixture split.
   Suggested files/tests: `crates/shape-vm/src/compiler/functions_foreign.rs`,
   `crates/shape-vm/src/executor/control_flow/native_abi.rs`,
   `crates/shape-runtime/stdlib-src/core/native.shape`,
   `bin/shape-cli/tests/ffi_e2e.rs`.
   Keep DuckDB/Arrow rows manual unless the book gate provisions native
   libraries and data; use deterministic libc/stub examples for count reduction.

## Book-Only Candidates

- Convert `advanced/comptime-llm-patterns.mdx:L170` to `text` or inline prose if
  the goal is count reduction without implementing typed fragments.
- Convert `advanced/comptime-annotations-cookbook.mdx:L308` to prose unless a
  full policy fixture is added.
- Keep `advanced/comptime.mdx:L76` disabled as an intentional diagnostic unless
  the snippet extractor supports expected-fail rows.
- Do not flip Python/TypeScript scalar snippets as plain book-only changes;
  they need the extension-runtime fixture in the gate.

## Static Checks

Static-only audit. No cargo, just, nextest, rustc, build, test, book-truth gate,
or extractor commands were run.
