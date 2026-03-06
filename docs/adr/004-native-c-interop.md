# ADR-004: Native C Interop In Language Core

## Status

Accepted (2026-02-25)

## Context

Shape currently relies on the `cffi` extension path for C library access. That
path is flexible but not ideal for production-grade interop:

- call overhead includes dynamic module dispatch and generic marshaling logic
- layout guarantees for C structs are not first-class in the language core
- dependency resolution for native libraries is not yet lockfile-driven
- ergonomics still depend on helper APIs instead of native syntax

The target is first-class, module-scoped, compile-time generated C wrappers
with near-zero runtime overhead, explicit ABI/layout guarantees, and robust
cross-platform behavior (Linux/macOS/Windows).

## Decision

### 1. Language surface: explicit `extern C` (no annotation magic)

Native bindings use direct syntax:

```shape
extern C fn cos(x: number) -> number from "libm";
extern C fn duckdb_open(path: string) -> ptr from "duckdb" as "duckdb_open";
```

- `extern C` is the native ABI entry point (quoted `extern "C"` remains accepted for compatibility).
- No `@native` annotation is used for call binding.
- Bindings are module-scoped and compiled as normal function symbols.

### 2. Native dependency resolution via `[native-dependencies]`

`shape.toml` provides alias-to-library mapping:

```toml
[native-dependencies]
libm = "libm.so.6"
duckdb = { linux = "libduckdb.so", macos = "libduckdb.dylib", windows = "duckdb.dll" }
```

- `extern ... from "duckdb"` resolves through this table.
- Platform-specific fallback is deterministic.
- `native-dependencies` is treated as a core-owned manifest section.

### 3. Compile-time wrapper generation

For each `extern C` declaration, the compiler emits:

- normal callable Shape function symbol
- `ForeignFunctionEntry.native_abi` metadata:
  - ABI
  - resolved library name/path
  - symbol name
  - canonical C signature

No runtime parsing of source declarations is needed.

### 4. Runtime execution model

At VM load/link time:

- native libraries are loaded once per VM and cached by library key
- symbols are resolved once
- `libffi` call interface (`Cif`) is prepared once per foreign function

At call time:

- args are marshaled directly to C ABI memory layout
- call executes via prepared `Cif` + code pointer
- return is decoded directly to `NanBoxed`
- no msgpack bridge on native path

### 4.1 Out-parameter support in core

To support C APIs that require `T* out` pointers (e.g., DuckDB open/connect/query
handles), core exposes pointer-cell intrinsics:

- `__native_ptr_new_cell() -> ptr`
- `__native_ptr_write_ptr(addr: ptr, value: ptr) -> void`
- `__native_ptr_read_ptr(addr: ptr) -> ptr`
- `__native_ptr_free_cell(cell: ptr) -> void`

This enables pure Shape packages to call out-parameter-heavy C APIs without
extension shims.

### 5. Canonical marshalling rules (v1)

Scalar mappings:

- `i8/u8/i16/u16/i32/i64/u32/u64/isize/usize` -> width-aware native scalars
  (preserved end-to-end through VM/wire/native ABI)
- `byte` -> `u8` (range 0..255)
- `char` -> `i8` (range -128..127)
- `f32/f64` -> `f32`/`number` with native-width preservation for `f32`
- `bool` -> Shape `bool`
- `cstring` -> Shape `string` (NUL-terminated; interior NUL rejected on input)
- `cstring?` -> Shape `Option<string>` (`None` on null pointers)
- `ptr` -> pointer-width scalar (`usize`) via native pointer carrier
- `callback(fn(...)->...)` -> automatic call-scoped callback pointer for callable args
- `void` -> Shape `()`

Rules:

- all narrowing conversions are range-checked
- no implicit object/hashmap conversion on native path
- null `cstring` return is an error in v1; use `Option<string>` (`cstring?`) for nullable
- language-level `int` / `number` stay as convenience aliases for script ergonomics

### 6. Struct layout strategy (production path)

`type C` is the required production model for high-performance C layout
interop:

- compile-time computed size/alignment/offsets (`repr(C)` semantics)
- explicit passing modes:
  - `by_value` (copy)
  - `by_ref` (`*const T`)
  - `out` (`*mut T`, callee writes)
  - `inout` (`*mut T`, read/write)
- pointer-backed field access by offset; no implicit object materialization
- explicit `to_object()` (or equivalent) for copy-on-demand conversion

Companion object conversion is compiler-generated (no annotations) when naming
matches one of:

- `type C FooC` + `type Foo`
- `type C CFoo` + `type Foo`
- `type C FooLayout` + `type Foo`

For compatible field sets/types, the compiler auto-registers `From`/`Into` in
both directions.

### 6.1 Arrow C import contract for `Table<T>`

Core imports Arrow C pointers through:

- `__native_table_from_arrow_c(schema_ptr, array_ptr) -> Result<Table<any>, AnyError>`
- `__native_table_from_arrow_c_typed<T>(schema_ptr, array_ptr, type_name) -> Result<Table<T>, AnyError>`
- `__native_table_bind_type<T>(table, type_name) -> Result<Table<T>, AnyError>`

The typed path validates runtime Arrow schema against the requested row type
name and returns `Result::Err` on mismatch.

### 7. `Vec<byte>` as canonical raw buffer

`Vec<byte>` is the standard contiguous memory carrier for native interop:

- binary payloads
- manual pointer passing
- typed views/casts with alignment checks

This avoids per-element object allocation for byte-oriented APIs.

### 8. Lockfile/caching contract

Native resolution metadata is persisted into `shape.lock` artifacts
(`external.native.library.*`) with:

- alias key
- resolved library candidate/path
- host triple / OS
- fingerprint (content hash for path/vendored libs; alias + declared version for system libs)

`build.external.mode = "frozen"` must reject unresolved/fingerprint-mismatched
native artifacts.

### 9. Package strategy for C-backed dependencies

Two classes are supported:

- **System libraries** (e.g., `libm`, `sqlite3`):
  - package declares required aliases + min version constraints
  - resolver verifies host availability and records lock artifact

- **Vendored/build libraries** (e.g., DuckDB package with bundled source/bin):
  - package declares per-target install/build recipe
  - outputs cached under Shape native cache directory
  - lockfile pins artifact hash and target

Registry metadata must declare which class a package uses.

### 10. JIT contract

`CallForeign` remains VM-managed in v1:

- JIT preflight marks it VM-only
- translator must fail fast if preflight is bypassed
- future phase may add direct call stubs + deopt hooks

This keeps correctness and ABI safety while still allowing JIT on surrounding
pure-Shape regions.

## Consequences

### Positive

- Native interop becomes first-class, explicit, and ergonomic.
- Runtime overhead is minimized (prelinked symbol + prepared CIF path).
- ABI and layout behavior are explicit and auditable.
- Packaging story is deterministic via lockfile artifacts.

### Negative

- Full zero-copy struct UX depends on `type C` view/pointer carriers.
- Cross-platform native packaging requires registry/build tooling work.

### Risks

- ABI drift if user declarations mismatch actual C signatures.
- Platform loader differences (search paths, naming, calling conventions).
- Unsound pointer usage without strong static rules around lifetimes/ownership.

Mitigation: strict compile-time signature checks, explicit pointer carriers,
frozen lockfile mode, and no implicit struct/object conversion in hot path.
