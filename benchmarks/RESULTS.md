# Shape JIT Benchmark Results

## Current Results (Post-Refactor Phase 3.5)

Date: 2026-02-19
Branch: main (shape-refactor)
Changes: FFI-to-inline optimizations (Task #19), shared NaN tags (Task #17), JIT module splits

| Benchmark       | Rust    | Node    | Shape JIT | JIT/Node |
|-----------------|---------|---------|-----------|----------|
| 01_fib          | 176ms   | 700ms   | 735ms     | 1.1x     |
| 02_fib_iter     | 24ms    | 115ms   | 331ms     | 2.9x     |
| 03_sieve        | 15ms    | 47ms    | 240ms     | 5.1x     |
| 04_mandelbrot   | 665ms   | 673ms   | 2.48s     | 3.7x     |
| 05_spectral     | 394ms   | 486ms   | 2.46s     | 5.1x     |
| 06_ackermann    | 55ms    | 128ms   | 278ms     | 2.2x     |
| 07_sum_loop     | 1ms     | 559ms   | 2.42s     | 4.3x     |
| 08_collatz      | 178ms   | 2.10s   | 1.80s     | 0.9x     |
| 09_matrix_mul   | 236ms   | 395ms   | 4.27s     | 10.8x    |
| 10_primes_count | 1.07s   | 1.09s   | 3.68s     | 3.4x     |

**Geometric mean JIT/Node: 3.11x** (JIT is 3.11x slower than Node on average)
**JIT vs Node: 1 win, 9 losses**

## Targets vs Actuals

| Benchmark     | Baseline | Target | Actual | Status |
|---------------|----------|--------|--------|--------|
| 03_sieve      | 6.8x     | 3.0x   | 5.1x   | Improved (from 6.8x), not at target |
| 09_matrix_mul | 11.0x    | 5.0x   | 10.8x  | Barely improved, needs array work   |
| 08_collatz    | 0.9x     | 0.9x   | 0.9x   | Maintained -- JIT wins!             |

## Analysis

### Wins
- **08_collatz (0.9x)**: JIT beats Node. This is a tight loop with simple arithmetic
  and branching -- exactly what the JIT's inline typed arithmetic excels at.

### Near-parity (1-2x)
- **01_fib (1.1x)**: Recursive function calls. JIT is nearly at Node parity.
  The function call overhead (ctx.locals save/restore) is the remaining gap.

### Moderate gap (2-4x)
- **02_fib_iter (2.9x)**: Iterative loops. The gap is from loop overhead
  (LoopStart/LoopEnd prologue/epilogue) and branch prediction.
- **06_ackermann (2.2x)**: Deep recursion. Same call overhead as fib.
- **10_primes_count (3.4x)**: Array iteration + modulo. Array access through
  FFI for get_index is the main bottleneck.
- **04_mandelbrot (3.7x)**: Nested loops with floating-point arithmetic.
  The typed arithmetic path (nullable_float64_binary_op) should help here
  once all operands are properly type-tracked through the loop.

### Large gap (>4x)
- **03_sieve (5.1x)**: Array mutation in a loop. SetIndexRef improvements
  should help but the main cost is array allocation and initialization.
- **05_spectral (5.1x)**: Nested loops with array access. Similar to mandelbrot.
- **07_sum_loop (4.3x)**: Simple accumulation loop. The 4.3x gap suggests
  significant loop iteration overhead that native loops avoid.
- **09_matrix_mul (10.8x)**: Triple-nested loop with array access. This is
  the worst case because every inner iteration does 2 array reads + 1 array write,
  all through bounds-checked paths. Needs unified array representation (Task #18)
  to bring this down.

## Key Optimization Opportunities

1. **Array representation unification (Task #18)**: The biggest remaining gap.
   JitArray uses #[repr(C)] but element access still goes through inline IR
   with bounds checks. Direct pointer arithmetic with hoisted bounds checks
   in loops would close the gap significantly for matrix_mul and sieve.

2. **Loop optimization**: Loop iteration overhead (LoopStart/LoopEnd, induction
   variable tracking) accounts for most of the gap in sum_loop, fib_iter, and
   sieve. Cranelift's register allocation handles this well but the JIT adds
   compilation overhead per iteration.

3. **Type propagation through loops**: Many benchmarks lose type information
   at loop merge points (PHI nodes). Carrying StorageHint through loop headers
   would allow more operations to use the NaN-sentinel fast path.

## Wave 2 (jit-v2-phase1) Additions

### Bounds-Check Elision Wireup

A MIR-level analyzer (`crates/shape-jit/src/mir_compiler/bounds_elision.rs`)
detects the canonical `for i in 0..arr.length { use arr[i] }` pattern at
compile time and emits `inline_array_get_unchecked` / `inline_array_set_unchecked`
on the active NaN-boxed `Place::Index` codegen path
(`crates/shape-jit/src/mir_compiler/places.rs`). The `optimizer/` crate's
`build_function_plan` infrastructure remains independent (1054 lines of
bytecode-level analysis, dead in this configuration except for its own
unit tests) — bridging bytecode-instruction-keyed plans to
MIR-statement-keyed codegen is an open problem; the new MIR-level
analyzer is the pragmatic substitute for the matmul/dot-product class.

The analyzer rejects any pattern that:
  - Reassigns the array slot (covering `a = a.push(...)` patterns).
  - Uses an iv that is not initialized to a non-negative integer constant.
  - Allows a non-monotone or negative-step iv increment.
  - Lacks a back-edge predecessor on the loop header.

The default empty plan keeps every access on the bounds-checked path,
preserving the v2_array_tests OOB zero-default semantics. Soundness of
the elided path relies on the existing loop-conditional branch
(`SwitchBool(iv < bnd, body, after)`) acting as a preheader guard for
each iteration: if `iv < bnd` and `bnd == arr.length` and `iv >= 0`,
then `arr[iv]` is in bounds with no further runtime check.

### New benchmark fixture: `07b_dot_product`

`benchmarks/shape/07b_dot_product.shape` and the matching
`benchmarks/node/07b_dot_product.mjs` exercise the canonical
elision-eligible pattern: `for k in 0..n { sum += a[k] * b[k] }`. Wired
into `benchmarks/run_all.sh` as a new measurement point. Note: the
existing 07_sum_loop benchmark is scalar-only (no array indexing) — 07b
is a separate benchmark, not a rewrite. Per CLAUDE.md, no existing
benchmark fixture is altered.

### Bench delta status

The Wave 2 plan calls for a measured 09_matrix_mul delta from 10.8x → ≤4x
slower than Node. That measurement is not produced in this commit
because `JITExecutor::execute_program` on `jit-v2-phase1` currently
fails JIT execution on programs that touch stdlib (matmul triggers the
known `JumpIfFalseTrusted slot has Unknown kind` bytecode-verifier
failure on `std::core::math::clamp`/`sign`/`coefficient_of_variation`).
The same failure reproduces on the pre-elision baseline (verified via
`git stash` + rebuild + run), so it is unrelated to this change.

The wireup itself is complete: `optimizer::build_function_plan` is
called per function and per top-level compile, the bounds-elision plan
is installed on `MirToIR` before `compile_body`, and the unchecked
load/store variants are emitted at `Place::Index` sites whose
`(arr_slot, iv_slot)` pair appears in the plan. End-to-end perf
measurement awaits a fix to the pre-existing JIT execution bug.
