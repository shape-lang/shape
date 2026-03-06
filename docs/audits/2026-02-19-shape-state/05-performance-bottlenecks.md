# Performance Bottlenecks and Competitiveness vs Node/V8

## Current Baseline (re-check on 2026-02-19)
Command: `bash shape/benchmarks/run_all.sh`

Result summary:
- Shape JIT vs Node: **1 win, 9 losses**
- Geometric mean JIT/Node: **3.31x slower**
- Previously failing JIT benchmarks now execute: `03_sieve`, `05_spectral`, `09_matrix_mul`

### Benchmark Snapshot

| Benchmark | Node | Shape VM | Shape JIT | JIT/Node |
|---|---:|---:|---:|---:|
| 01_fib | 706ms | 41.93s | 788ms | 1.1x |
| 02_fib_iter | 111ms | ERR | 321ms | 2.9x |
| 03_sieve | 40ms | 4.43s | 273ms | 6.8x |
| 04_mandelbrot | 673ms | ERR | 2.66s | 3.9x |
| 05_spectral | 489ms | 109.86s | 2.65s | 5.4x |
| 06_ackermann | 123ms | ERR | 269ms | 2.2x |
| 07_sum_loop | 552ms | 79.07s | 2.39s | 4.3x |
| 08_collatz | 2.05s | 39.02s | 1.93s | 0.9x |
| 09_matrix_mul | 384ms | 100.14s | 4.23s | 11.0x |
| 10_primes_count | 1.08s | 113.28s | 3.67s | 3.4x |

## Crash Status

Direct reproductions now succeed (`rc=0`):
- `./target/release/shape -m jit shape/benchmarks/shape/03_sieve.shape`
- `./target/release/shape -m jit shape/benchmarks/shape/05_spectral.shape`
- `./target/release/shape -m jit shape/benchmarks/shape/09_matrix_mul.shape`

## Bottleneck Classes

### 1. `SetIndexRef` is now mixed-mode: inline fast path + FFI fallback
- JIT now emits an inline path for array/index writes through references (`shape/shape-jit/src/translator/opcodes/references.rs:139`, `shape/shape-jit/src/translator/opcodes/references.rs:142`).
- Fallback still calls `jit_set_index_ref` for edge cases (`shape/shape-jit/src/translator/opcodes/references.rs:145`, `shape/shape-jit/src/ffi/references.rs:19`).
- Non-reference local index writes also use inline mutation (`shape/shape-jit/src/translator/opcodes/data.rs:245`, `shape/shape-jit/src/translator/opcodes/data.rs:268`).

Impact:
- This issue is improved but not fully eliminated; remaining fallback frequency still matters for worst-case loops.

### 2. Numeric hot loops still pay NaN-boxing/type-guard overhead
- Generic numeric ops run tag guards and box/unbox flow in the JIT helpers (`shape/shape-jit/src/translator/helpers.rs:16`, `shape/shape-jit/src/translator/helpers.rs:48`, `shape/shape-jit/src/translator/helpers.rs:51`).
- Typed fast paths exist but are marked as reserved/limited use (`shape/shape-jit/src/translator/helpers.rs:68`, `shape/shape-jit/src/translator/helpers.rs:77`).
- Even with bitcast conversions, repeated tag checks and boxed representation handling remain in the critical path (`shape/shape-jit/src/translator/helpers.rs:517`, `shape/shape-jit/src/translator/helpers.rs:527`).

Impact:
- This is a core source of the remaining 3x+ gap in numeric-heavy kernels versus specialized engines.

### 3. Strong typing not fully carried to execution strategy
- Type checker still treats many `Table` methods as loose `any` contracts (`shape/shape-runtime/src/type_system/checking/method_table.rs:166`).
- Runtime method calls still dispatch by method-name string (`shape/shape-vm/src/executor/objects/mod.rs:127`).

Impact:
- Missed specialization opportunities and avoidable dynamic overhead.

### 4. Row/column access still uses repeated runtime downcasting in hot access paths
- RowView property access does runtime Arrow type downcast checks per access (`shape/shape-vm/src/executor/objects/property_access.rs:161`).

Impact:
- Per-field access overhead accumulates in data-heavy loops.

### 5. Representation boundary overhead remains non-trivial
- VM still crosses between `NanBoxed` and `VMValue` at multiple execution boundaries (call convention, snapshots, debugger, etc.).
- Evidence examples: `shape/shape-vm/src/executor/call_convention.rs:24`, `shape/shape-vm/src/executor/mod.rs:988`, `shape/shape-runtime/src/snapshot.rs:721`.

Impact:
- Extra conversion and allocation pressure.

## Why “Strongly Typed” Has Not Yet Converted into V8-Level Wins

Strong typing helps only if it reaches:
1. call lowering,
2. memory layout,
3. dispatch strategy,
4. JIT codegen specialization,
5. elimination of dynamic guards.

Current state has strong typing at compile level in many places, but runtime/JIT still executes major paths with dynamic machinery or unstable implementations.

## Priority Plan to Close the Gap

1. **Stabilize JIT first (non-negotiable).**
   - Remove pathological slow paths and replace placeholder FFI returns.
   - Keep targeted regression benches for former crash cases to prevent recurrence.

2. **Create typed method dispatch fast paths.**
   - Avoid string-dispatch for known method IDs on known receiver kinds.

3. **Specialize row field extraction by schema/column type at compile time.**
   - Pre-bind field extractors to avoid repeated Arrow downcast chains.

4. **Tighten method table typing for Table pipelines.**
   - Replace `any` contracts with concrete generic result propagation where possible.

5. **Define VMValue boundary policy.**
   - Keep at explicit subsystem boundaries only, not in hot execution loops.

## Suggested KPIs
- JIT crash rate: `0` on benchmark suite.
- JIT/Node geometric mean target: `<1.5x` first milestone, then `<1.0x`.
- Hot-path conversions (`to_vmvalue`/`from_vmvalue`) reduced by >50% outside debugger/snapshot/interop boundaries.
- Table method typing coverage: >=80% non-`any` signatures for core query methods.
