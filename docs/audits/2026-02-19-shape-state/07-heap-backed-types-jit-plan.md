# Heap-Backed Types: JIT-First Implementation Guideline

## Goal
Design and implement `Vec<T>`, `HashMap<T>`, `Matrix<T>`, and `DataTable<T>` so the default JIT path can beat V8-class runtimes on numeric/data workloads.

## Why this must change now
Current state shows structural blockers for a JIT-default future:
- VM and JIT still use different value-tagging models (`shape/shape-value/src/nanboxed.rs:61` vs `shape/shape-jit/src/nan_boxing.rs:14`).
- JIT array path is still clone-heavy and allocation-heavy in FFI (`shape/shape-jit/src/ffi/array.rs:13`, `shape/shape-jit/src/ffi/array.rs:39`, `shape/shape-jit/src/ffi/array.rs:115`).
- Some JIT data access paths are stubs (`shape/shape-jit/src/ffi/data.rs:319`).
- DataTable runtime is Arrow-backed and already columnar (`shape/shape-value/src/datatable.rs:1`), but many APIs still surface array-heavy intermediate values in query/group flows (`shape/shape-vm/src/executor/objects/datatable_methods/query.rs:285`).

## Performance principles
1. One runtime value ABI for interpreter and JIT.
2. Hot-path operations must be direct memory access, not helper-call orchestration.
3. Specialize by element type aggressively (`f64`, `i64`, `u32`, `bool`), with generic fallback.
4. Keep FFI only for cold paths and complex fallback behavior.
5. Prefer columnar and block-contiguous layouts that can be auto-promoted to matrix views.

## Ground truth on Rust HashMap
- `std::collections::HashMap` is not fundamentally poor; it is SwissTable-based (`hashbrown`) in modern Rust.
- Real performance choice is usually the hasher and key representation.
- For trusted, performance-critical runtime internals, `SipHash` defaults are often slower than needed.

## Target heap object model (new baseline)
Introduce a single internal heap object header usable by VM and JIT:

```rust
#[repr(C, align(16))]
struct HeapHeader {
    kind: u16,      // Vec, Map, Matrix, Table, String, etc.
    elem_type: u8,  // F64, I64, U32, Bool, Any, ...
    flags: u8,      // mutability, nullability, ownership bits
    len: u32,
    cap: u32,
    aux: u64,       // schema id / stride / pointer to metadata
}
```

All new container kinds should use this ABI so JIT can inline shape checks and pointer math without bridging through ad-hoc FFI structs.

## 1) `Vec<T>` implementation plan

## Runtime representation
Add heap kinds:
- `VecAny` -> contiguous `NanBoxed` buffer.
- `VecF64`, `VecI64`, `VecU32`, `VecBool` -> contiguous typed buffers.

Use aligned allocation (>=32-byte alignment) for SIMD-friendly loads.

## JIT path
Add direct-lowering intrinsics:
- `vec_len`, `vec_get`, `vec_set`, `vec_push`, `vec_pop`, `vec_slice`.
- For typed vectors, JIT emits direct typed loads/stores.
- Bounds checks are one branch, then direct pointer arithmetic.
- Remove `SetIndexRef` FFI mutation in favor of inline write-through lowering
  (today: `shape/shape-jit/src/translator/opcodes/references.rs:97` calls `jit_set_index_ref` in `shape/shape-jit/src/ffi/references.rs:19`).

No per-op cloning and no `Vec<u64>` rebuild loops.

## Language semantics
- Preserve language-level array ergonomics.
- Internally, auto-specialize literal arrays and inferred homogeneous arrays.
- Deopt to `VecAny` when mixed types appear.

## Acceptance criteria
- `push/pop/get/set/len` on typed vectors perform with zero heap allocation in steady state.
- JIT array microbenchmarks show <20ns element get/set for hot loops.

## 2) `HashMap<T>` implementation plan

## Runtime representation
Add map kinds:
- `MapSym<V>` keyed by interned symbol IDs (`u32`).
- `MapStr<V>` keyed by interned string IDs (`u32`).
- `MapAny` fallback for dynamic keys.

Implementation choice:
- Backing table: `hashbrown::HashMap` or `hashbrown::raw::RawTable`.
- Hasher choice per map kind:
- `MapSym` and `MapStr`: nohash-style integer hashing (fastest).
- `MapAny`: `ahash` for trusted/internal fast mode.
- Optional secure mode: keep `SipHash` for untrusted sandbox contexts.

## JIT path
- Property access lowers to symbol-id lookup, not raw string hashing.
- Add inline fast path for `MapSym` with integer key probes.
- Fallback helper only for polymorphic or slow-path map kinds.

## Key optimization requirement
Introduce global string interning in compiler/runtime boundary so method/property names become stable symbol IDs in bytecode constants.

## Acceptance criteria
- Eliminate repeated runtime string hashing for hot method/property paths.
- Map lookup benchmarks: 2x+ faster than current string-key dynamic path on representative workloads.

## 3) `Matrix<T>` implementation plan

## Runtime representation
Add matrix kinds:
- `MatF32`, `MatF64`, `MatI32` (minimum).

Matrix header fields (in `aux` / side metadata):
- rows, cols.
- row_stride, col_stride.
- layout flag: row-major / column-major / tiled.
- device flag: CPU / GPU.

Use contiguous aligned buffers by default; support non-owning views for slices/transposes.

## CPU kernels
- Baseline scalar kernels.
- SIMD kernels using `std::simd` for add/mul/axpy/reduction.
- Blocked GEMM kernels with cache tiling.
- Optional backend hook for `matrixmultiply`/BLAS where available.

## GPU roadmap
- Backend abstraction now (`MatrixBackend` trait).
- Phase 1 CPU-only default.
- Phase 2 optional `wgpu` compute backend for large dense ops.
- JIT emits backend dispatch stubs based on matrix size and backend availability.

## JIT path
Add typed opcodes (or typed intrinsic IDs):
- `MatMulF64`, `MatAddF64`, `MatSubF64`, `MatScaleF64`, `MatTransposeF64`.

Lowering rule:
- Monomorphic matrix types -> direct kernel call with prevalidated dims.
- Polymorphic path -> runtime shape checks + fallback helper.

## Acceptance criteria
- Dense `f64` GEMM throughput significantly above current array loops.
- No conversion to generic arrays inside matrix kernels.

## 4) `DataTable<T>` migration plan (away from array-oriented flows)

Note: DataTable is already columnar Arrow-backed today (`shape/shape-value/src/datatable.rs:1`).
The migration target is to remove array-heavy intermediate APIs and make table internals matrix/pivot-friendly.

## DataTable v2 core
Replace ad-hoc row/array intermediate structures with:
- `ColumnBlock` storage enum:
- `F64Block`, `I64Block`, `BoolBlock`, `Utf8DictBlock`, `TimeBlock`, `AnyBlock`.
- `TableSchema` with stable column IDs and logical/physical types.
- `TableIndex` abstraction (time index, key index, composite index).
- `MatrixViewRegistry` for dense numeric subsets.

## Auto-pivot design
Add a pivot planner:
- Detect candidate dimension columns and metric columns.
- Compute cardinalities and sparsity estimate.
- Choose dense matrix, sparse matrix, or keep columnar.

Expose pivot artifacts as first-class table/matrix views, not `Array<Array<...>>` results.

## JIT integration
Extend `JITContext` with column block descriptors instead of raw `*const f64` only (`shape/shape-jit/src/context.rs:19`).

Add typed loaders:
- `load_col_f64`, `load_col_i64`, `load_col_bool`, dictionary decode helpers.

For matrix-backed views, JIT reads contiguous matrix blocks directly.

## Interop policy
- Keep Arrow as ingestion/egress boundary.
- Internally normalize to `ColumnBlock` representation for execution.
- Zero-copy where Arrow buffer layout matches target blocks.

## Acceptance criteria
- Group/pivot/query pipelines stop returning array-heavy intermediate structures in hot paths.
- Auto-pivot can materialize dense metric matrices without generic array conversions.

## 5) JIT-first migration phases

## Phase 0: ABI unification
- Unify VM/JIT value tagging and heap object ABI.
- Remove representation mismatches between `shape-value` and `shape-jit`.

## Phase 1: `Vec<T>`
- Implement typed vector heap kinds and JIT direct ops.
- Remove clone-based JIT array helpers for hot ops.

## Phase 2: `HashMap<T>`
- Add symbol interning and map specializations.
- Switch property/method lookup to symbol IDs.

## Phase 3: `Matrix<T>`
- Add matrix runtime kinds and SIMD kernels.
- Introduce matrix op intrinsics/opcodes in JIT.

## Phase 4: `DataTable<T>` v2 + auto-pivot
- Introduce `ColumnBlock` and matrix views.
- Migrate query/group/pivot internals to non-array intermediate model.

## Phase 5: default-JIT hardening
- Remove remaining JIT stubs/placeholders.
- Gate default-JIT promotion on crash-free parity and perf KPIs.

## 6) KPI gate to become default JIT

Hard gates:
- Zero segfaults in benchmark suite and parity tests.
- JIT/Node geometric mean <= 1.2x (first gate), then <= 1.0x.
- >=80% of hot operations avoid FFI fallback.

Container-specific gates:
- `VecF64` loops: no per-iteration allocations.
- `MapSym` lookup: no string hashing in hot loops.
- `MatF64` GEMM: stable SIMD path with measurable multi-core scaling.
- DataTable auto-pivot: dense matrix materialization without array conversion churn.

## 7) Risks and mitigations

Risk: complexity explosion from too many container variants.
Mitigation: start with minimum useful specializations (`Any`, `F64`, `I64`, `U32`, `Bool`).

Risk: secure hashing tradeoff.
Mitigation: dual mode (secure by default in untrusted mode, fast hasher in trusted JIT mode).

Risk: migration regressions from dual old/new representations.
Mitigation: feature-gated rollout, conformance tests, and phased deprecation.

## 8) Immediate implementation backlog (next PR sequence)
1. Define unified heap header and kind IDs used by VM and JIT.
2. Inline `SetIndexRef` write-through mutation path in JIT (remove `jit_set_index_ref` from hot loops).
3. Implement `VecF64` + JIT `len/get/set` direct lowering.
4. Add symbol interning table and convert method/property call sites to symbol IDs.
5. Implement `MapSym` with integer-key fast hasher.
6. Add `MatF64` with aligned contiguous storage + SIMD add/mul/reduce.
7. Introduce `ColumnBlock::F64Block` and one DataTable path that auto-exposes `MatrixView`.
8. Replace one query/group path returning array intermediates with table/matrix-native outputs.

This sequence yields early perf wins while building the full architecture needed to surpass V8-class performance.
