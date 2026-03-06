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
