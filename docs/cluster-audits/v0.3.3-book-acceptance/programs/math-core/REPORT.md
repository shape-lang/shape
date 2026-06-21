# Book-Acceptance Report — slice `math-core`

Binary: /home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape (strict-flip-collection-dispatch worktree, HEAD release build)
Book chapters (PRIMARY source):
- stdlib/native/math.mdx  (bare builtins + constants + trig + clamp/lerp/sign)
- stdlib/core/math.mdx    (statistical surface: sum/mean/std/variance/median/percentile/correlation/covariance/spread/zscore/coefficient_of_variation)

Determinism strategy: pure — assert math fn results against book-derived expected values
and mathematical identities (variance-builtin == sum-of-squares formula, std == sqrt(var),
rotation preserves length, cross orthogonal to inputs, Newton sqrt == builtin sqrt, etc.).

## Programs

### small.shape (48 non-blank LOC)
Exercises the chapter core: bare-global builtins (sqrt/abs/floor/ceil/round/min/max/pow),
constants (PI/E/TAU), trig (sin/cos + radians/degrees), helpers (clamp/lerp/sign), and the
working statistical functions (sum/mean/variance/std).

- VM  : EC=0, stdout = `ALL_CHECKS_PASSED`
- JIT : EC=0, stdout = `ALL_CHECKS_PASSED` (JIT bails on the direct `min` call — "Route A
  surface-and-stop: no JIT FuncRef" — and falls through to the interpreter; stderr only)
- stdout VM == JIT byte-for-byte: YES
- Classification: PASS

### large.shape (405 non-blank / 471 total LOC)
A deterministic numerical-computing toolkit across 6 modules, ~170 assertions:
1. Vec3 algebra (dot/cross/norm/normalize/angle) — cross-orthogonality + unit-length identities
2. 2D rotation via trig — rot90/rot180/rot360, length preservation, rotation composition,
   sin^2+cos^2=1 sampled at 13 angles
3. Deterministic LCG PRNG (Numerical Recipes constants, hand-computed first state 1083814273)
4. Statistics — builtin mean/variance/std cross-checked against hand formulas on the
   LCG-derived 64-element series; textbook reference set {2,4,4,4,5,5,7,9} (mean=5,var=4,std=2);
   shift/scale invariances (shift mean by k, scale variance by k^2)
5. Signal smoothing — moving average (windows 1/4/full) and EMA (alpha 0.5/1.0) with
   hand-computed EMA chain e0..e5 = 1, 1.5, 2.25, 3.125, 4.0625, 5.03125
6. Polynomials (Horner), Newton sqrt, floor/ceil/round contract (incl. negatives),
   atan2/atan/asin/tan identities, degrees/radians round-trip, pow/sqrt round-trip

- VM  : EC=0, stdout = `ALL_CHECKS_PASSED`
- JIT : EC=0, stdout = `ALL_CHECKS_PASSED` (JIT bails to interpreter on unverified V2 typed
  opcodes — see below; stderr only)
- stdout VM == JIT byte-for-byte: YES
- Classification: PASS (with a non-fatal stderr diagnostic — see V0.4-DEFER note)

Note on LOC: the slice's *working* book surface is narrow (5 of the 11 statistical functions
are NotImplemented stubs — see BOOK-WRONG below), so the large program is ~470 LOC of densely
asserted code rather than ~1000 LOC of filler. Every line exercises the live surface; padding
to 1000 LOC would mean repeating identities, not adding coverage.

Expected-value rationale (all derived from book semantics BEFORE first run):
- sqrt(3*3+4*4)=5, abs(-7)=7, pow(2,10)=1024 — book native/math.mdx examples
- PI=3.141592653589793, E=2.718281828459045, TAU=6.283185307179586 — book constants table
- sin(radians(90))=1, cos(0)=1, degrees(PI)=180 — book trig section
- clamp(150,0,100)=100, lerp(0,10,0.5)=5, sign(-5)=-1 — book helpers table
- sum([1,2,3,4])=10, mean([10,20,30])=20 — book core/math examples
- variance/std of {2,4,4,4,5,5,7,9}=4/2 — standard textbook population stats (book documents
  variance/std as the population aggregations)
- LCG first state from seed 42: 1664525*42+1013904223 = 1083814273 (< 2^32, mod identity)
- EMA chain & MA windows — hand-computed from the recurrence
- Newton sqrt(2)=1.4142135623730951 — IEEE-754 double sqrt

## Findings

### BOOK-WRONG (book documents functions the shipped binary does NOT implement)

The core/math.mdx "Function Reference" table and prose document these as working, but the
release binary raises a runtime `Not implemented` surface (phase-1b-vm-wave-5d-intrinsic,
"v0.4 / planned") when they are called:

- `median(series)`        -> NotImplemented: IntrinsicMedian body migration pending (v0.4)
- `percentile(series, p)` -> NotImplemented: IntrinsicPercentile (v0.4)
- `spread(series)`        -> NotImplemented: IntrinsicMax (spread is max-min internally) (v0.4)
- `correlation(a, b)`     -> NotImplemented: IntrinsicCorrelation (v0.4)
- `covariance(a, b)`      -> NotImplemented: IntrinsicCovariance (v0.4)

Additionally:
- `zscore(series)` -> NOT a NotImplemented stub but a hard `TypeError: expected number, got
  array`. The book (core/math.mdx) shows `zscore([2.0,4.0,6.0])` returning a `Vec<number>`;
  the shipped function rejects an array argument. BOOK-WRONG (documented signature does not
  match shipped behavior).

Working statistical functions (book-accurate): `sum`, `mean`, `variance`, `std`,
`coefficient_of_variation`. (`coefficient_of_variation([2,4,6])` = 0.408248290463863, matches
std/mean = 1.632993.../4 .)

The book gives NO indication that median/percentile/spread/correlation/covariance/zscore are
unimplemented. A user following core/math.mdx verbatim hits a runtime error on the first call.
Recommend either implementing them for v0.3.3 or annotating them in the book as "v0.4".

### BOOK-GAP

- core/math.mdx documents `variance`/`std` without stating whether they are POPULATION or
  SAMPLE statistics. The shipped binary uses the population formula (variance of
  {2,4,4,4,5,5,7,9} = 4.0, i.e. divide by N, not N-1). The book should state the convention;
  a user could reasonably expect the sample (N-1) variant and silently get wrong magnitudes.
  (No MCP/reference fallback was needed to author the programs — this gap is documentary, not
  blocking; recorded as the book failing to specify the convention.)
- native/math.mdx llm_common_mistakes notes trig is in radians and constants are functions,
  which is accurate and helped avoid `PI` vs `PI()` errors — good. No gap there.

### V0.4-DEFER (non-fatal infra diagnostic, not a book issue)

Any function that locally constructs a typed `Array<number>` via `[]` + `.push()` emits a
non-fatal stderr diagnostic: `V2 bytecode verification failed: N violation(s) — ... typed
opcode NewTypedArrayF64 ... has no FrameDescriptor`. Confirmed on a 5-line minimal repro.
Under `--mode jit` this same surface causes a `[jit-fallback]` to the bytecode interpreter
(R8 W7 G.5, ADR-006 §2.7.14, explicitly "v0.4 / planned: full V2 type soundness for every
JIT-emitted opcode"). It is COSMETIC: stdout is correct, EC=0, and VM==JIT byte-identical in
both programs. Recorded for completeness; outside the math-core book scope.

## Classification summary
- small.shape: PASS (VM==JIT byte-identical)
- large.shape: PASS (VM==JIT byte-identical; non-fatal V0.4 stderr diagnostic)
- Slice-level book findings: BOOK-WRONG (5 NotImplemented stat fns + zscore TypeError),
  BOOK-GAP (population-vs-sample convention unspecified).
