# Wave 17 Content/Container Parity Scope

Date: 2026-07-09

Role: Wave-17D content/container parity scout

Constraints honored:

- Read-only investigation except this report.
- No edits to `AGENTS.md`.
- No `cargo`, `just`, `rustc`, build, test, or book-truth commands run.

## Executive finding

The smallest truthful implementation lane is **Content array parity**:

1. Make `Content.*` namespace builders produce a concrete `content` result in bytecode compiler type metadata.
2. Teach v2 typed-array/container inference and runtime detection to carry `Array<content>` as a typed heap-pointer array.
3. Add focused runtime/compiler coverage for `Content.fragment([Content.text(...), ...])` and mixed content-builder arrays.

This lane should unlock the disabled `Content.fragment` snippets without touching table query APIs, database loaders, optimizer internals, matrix arithmetic intrinsics, or v0.4 preview content syntax.

Do **not** start by flipping table, optimizer, physics, or rotation/interpolation docs. Those are either mixed/stale, external preview material, or separate implementation gaps.

## Relevant disabled snippets

### Current implementation gaps

#### `Content.fragment` with builder arrays

Source: sibling book file `book/book-site/src/content/docs/fundamentals/content.mdx`.

Relevant disabled snippets:

- `Content.fragment([fast, slow])`
- `Content.fragment([summary, table, chart])`

The docs already identify the blocker: `Content` builders exist at runtime, but static checking cannot infer a shared `Array<content>` for builder arrays. This matches the implementation shape:

- `crates/shape-vm/src/compiler/expressions/function_calls.rs` lowers `Content.chart`, `Content.text`, `Content.table`, `Content.code`, `Content.kv`, and `Content.fragment` namespace calls to builtins.
- The same lowering path only stamps `last_expr_type_info` for `Color.rgb`; Content namespace calls are not stamped as `content`.
- `crates/shape-vm/src/executor/vm_impl/builtins.rs` already implements `ContentFragmentCtor`, but it requires exactly one `Array<content>` argument backed by a v2 typed array of `Ptr(HeapKind::Content)`.
- `crates/shape-vm/src/compiler/v2_typed_emission.rs` and `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs` have no content element kind/tag, so array literals of content values cannot be truthfully materialized as `Array<content>`.

Classification: implementation gap, good first lane.

#### Public `mat(rows, cols, flat: Array<number>)`

Sources:

- `book/book-site/src/content/docs/stdlib/math/interpolation.mdx`
- `book/book-site/src/content/docs/stdlib/math/rotation.mdx`
- `crates/shape-runtime/stdlib-src/math/rotation.shape`

Disabled snippets in interpolation and rotation depend on public construction of `Mat<number>`. The current global `mat` helper resolves to `BuiltinFunction::MatFromFlat`, but the runtime implementation expects spread scalar elements as `mat(rows, cols, ...values)`. The shipped stdlib and docs call `mat(rows, cols, flat_array)`.

That means `mat(1, 2, [0.5, 0.5])` is currently interpreted as one provided element for a two-element matrix, not as a flat element array.

Classification: implementation gap, but second lane. It is smaller than optimizer work, but wider than content parity because the stdlib uses the flat-array contract in multiple matrix constructors and matrix arithmetic intrinsics still have separate `NotImplemented` surfaces.

#### Optimizer typed-array/container migration

Source: `book/book-site/src/content/docs/stdlib/math/optimize.mdx`.

The disabled optimizer snippets exercise nested `Array<Array<number>>`, mutation, and vector helper usage in `crates/shape-runtime/stdlib-src/math/optimize.shape`. This is a real implementation gap, but it is not the same as public `Mat<number>` construction and should not be bundled with content parity.

Classification: implementation gap, separate later lane.

#### Physics strict-module inference

Source: `book/book-site/src/content/docs/stdlib/domain/physics.mdx`.

The disabled physics snippets are blocked on strict-type acceptance of the physics module in VM/JIT paths, not on content/table/Mat parity alone.

Classification: implementation gap, separate later lane.

### Stale or narrowly flippable docs, but not first implementation work

#### Basic `Table<T>` row literals

Source: `book/book-site/src/content/docs/fundamentals/tables.mdx`.

The introductory `Table<Person>` row-literal snippet is disabled, but current compiler/runtime support exists:

- `crates/shape-vm/src/compiler/statements.rs` special-cases `Table<T>` variable declarations with row literal arrays and emits table schema binding.
- `crates/shape-vm/src/executor/tests/type_system_integration.rs` has focused coverage for table row literal count/filter/select/projection and chart projection.

Classification: likely stale or doc-gate candidate. Do not use it as an implementation lane. If flipped later, it needs a focused book gate because the same page also contains preview/external table APIs.

### External, preview, or pseudo-code snippets

Keep these disabled even after content parity:

- `c"..."` content-string syntax, `Content` trait dispatch, and `ContentFor<Adapter>` examples in `fundamentals/content.mdx`.
- Automatic collection-to-table rendering in `fundamentals/content.mdx`.
- App/database-backed table loading, queryable connectors, and generic query examples in `fundamentals/tables.mdx`.
- Content-string table/chart format-specifier examples in `fundamentals/tables.mdx`.
- Comment-only or explanatory data-table snippets in `getting-started/basic-concepts.mdx` unless a later book-truth pass proves they should be treated as runnable examples.

Classification: external/pseudo-code or v0.4 preview, not current implementation gaps.

## First patch wave ownership

Recommended worker role: content-array parity implementation worker.

Primary owned files:

- `crates/shape-vm/src/compiler/expressions/function_calls.rs`
  - Stamp `last_expr_type_info` as builtin `content` for `Content.text`, `Content.table`, `Content.chart`, `Content.code`, `Content.kv`, and `Content.fragment` namespace constructors after their builtin call emission.
  - If `Table.new`, `Code.new`, or `KeyValue.new` builder paths produce content nodes through the same surface, stamp those too.
- `crates/shape-vm/src/compiler/expressions/collections.rs`
  - Ensure array literals whose elements have concrete `content` type metadata infer `Array<content>` and select v2 typed-array emission instead of falling back to an untyped/dynamic array.
- `crates/shape-vm/src/compiler/v2_typed_emission.rs`
  - Add a content typed-array kind, storage mapping, and emission path for `NativeKind::Ptr(HeapKind::Content)`.
- `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs`
  - Add the runtime content element tag/kind mapping so the VM can detect and validate `Array<content>` at call boundaries.
- `crates/shape-value/src/v2/typed_array.rs`
  - Add content heap-pointer element support only if the v2 tag/element table lives here for the existing pointer carriers.

Focused test ownership:

- Prefer a new focused executor/compiler test file if the existing content/table integration tests are already large.
- Otherwise add narrow cases under `crates/shape-vm/src/executor/tests/type_system_integration.rs`.
- Required cases:
  - `Content.fragment([Content.text("fast"), Content.text("slow")])`
  - `let summary = Content.text(...); let table = Content.table(...); let chart = Content.chart(...); Content.fragment([summary, table, chart])`
  - A negative case where non-content array elements still fail honestly instead of being coerced to `Any`.

Out of scope for this first patch:

- Generic content-string syntax and formatter dispatch.
- `ContentFor<Adapter>` and trait-based content rendering.
- Database/app table loading or queryable connectors.
- Optimizer typed-array migration.
- Physics strict-module fixes.
- Matrix multiplication or matrix-vector intrinsics.

## Later lanes

### Public matrix constructor lane

Own:

- `crates/shape-vm/src/executor/builtins/datetime_builtins.rs`
- Focused compiler call-lowering only if `mat(rows, cols, flat_array)` needs a special argument shape.
- `crates/shape-runtime/stdlib-src/math/rotation.shape` only if the project decides the public contract should remain spread scalars instead of flat arrays.

Goal:

- Make `mat(rows, cols, flat: Array<number>)` work because the stdlib already uses that public shape.
- Keep `rotation_apply` and `rotation_compose` disabled until matrix-vector/matrix-matrix intrinsics are implemented.

### Table docs lane

Own:

- Sibling book `fundamentals/tables.mdx` only, after focused book verification.

Goal:

- Flip only the basic `Table<T>` row literal examples that are already covered by compiler/runtime tests.
- Keep app loaders, queryable connectors, and content-string table/chart formatting disabled.

### Optimizer lane

Own:

- `crates/shape-runtime/stdlib-src/math/optimize.shape`
- Typed-array/container mutation compiler/runtime files identified by the optimizer failure mode.

Goal:

- Fix nested numeric array construction and mutation in optimizer code without changing public math docs first.

## Changed paths

This scout changed one file:

- `docs/cluster-audits/wave17-content-container-parity-scope.md`

Line count: 181.
