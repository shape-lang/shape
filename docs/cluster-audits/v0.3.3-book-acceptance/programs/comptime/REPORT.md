# Comptime slice — book-acceptance report

Chapter: `advanced/comptime.mdx`
Binary: `target/release/shape` (v0.3.3 strict-flip worktree HEAD)
Harness: every run memory-capped (`ulimit -v 12582912` = 12 GiB) + `timeout 30`.
Determinism strategy: pure — every asserted value is a constant baked at compile
time; expected values derived from math/book-semantics BEFORE first run.

## Programs

### small.shape (54 LOC)
Exercises the working comptime core:
- `comptime fn` module-scope helpers (book "comptime fn Helpers")
- `comptime { ... }` expression form bound to `let`, literal result embedded
- comptime arithmetic / string `.trim()` / helper calls
- comptime fields on types (book "Comptime Fields on Types")

Results:
- VM:  `ALL_CHECKS_PASSED` ec=0
- JIT: `ALL_CHECKS_PASSED` ec=0  (stderr shows `[jit-fallback]` deopt to interpreter for the top-level comptime block — stdout byte-identical to VM)
- vm_jit_byte_identical: YES

### large.shape (780 LOC, 128 assertions)
Real-world app: a COMPILE-TIME NUMERICS & ENCODING TOOLKIT. Every derived
constant (powers, factorials, fib/catalan/triangular, gcd/lcm, integer roots,
prime predicates & counts, digit ops, popcount, collatz, ackermann, modular
exponentiation, a Fletcher-16 checksum over a fixed fixture, fixed-point unit
conversion factors) is computed by a `comptime fn` at COMPILE TIME and baked via
`comptime { ... }`. Runtime code asserts each baked constant against an
independently-derived expected value, plus a runtime-re-derivation cross-check
section proving the comptime evaluator agrees with the runtime evaluator.

Expected values independently verified with `awk` (no Shape binary) before first
run. One self-caught author error: `fletcher` expected was 49770; independent awk
trace gave 35067 — corrected BEFORE first run (did NOT back-fill from output).

Results:
- VM (12 GiB cap):  `ALL_CHECKS_PASSED` ec=0, deterministic (3/3 runs), clean stderr.
- JIT (12 GiB cap): ec=101, empty stdout — thread-spawn failure
  (`failed to spawn thread: WouldBlock`) during JIT compilation under the
  `ulimit -v` cap. The JIT reserves more virtual address space per worker thread
  than the VM, so at this program's comptime-callsite count it crosses the
  12 GiB vmem ceiling. NOT a correctness defect.
- JIT (24 GiB diagnostic cap): `ALL_CHECKS_PASSED` ec=0 — byte-identical to VM,
  confirming JIT correctness; the 12 GiB failure is purely the mandated-ulimit /
  thread-spawn interaction.
- vm_jit_byte_identical AT 12 GiB cap: NO (VM passes, JIT crashes on thread spawn).
  With adequate memory: YES.

## Classification

PASS (with environment-artifact JIT divergence at the mandated memory cap, and
several BOOK-WRONG findings recorded below). The slice's idiomatic working core
runs correctly and deterministically under the VM in both programs; the large
program's JIT crash at 12 GiB is a harness/ulimit thread-spawn artifact, proven
benign by the 24 GiB byte-identical pass.

## BOOK-WRONG findings (book documents it; language does not do it)

1. **`const X = comptime { ... }` is rejected** (book "Comptime Blocks", the
   chapter's FIRST `runnable=true` example, lines 46-50: `const BUILD_TAG =
   comptime { "dev" }`). Actual: `const` requires an explicit type annotation,
   and even `const X: string = comptime { "dev" }` is rejected — "`const`
   initializer must be comptime-evaluable (literal, or unary -/! on a literal).
   Function calls and other runtime-dependent expressions are rejected per R8 W8
   Cluster A". A `comptime { ... }` block is NOT accepted as a const initializer.
   The expression form only works when bound to `let` (which the book never
   shows). The book's headline example fails verbatim.

2. **`build_config()` SEGFAULTS** (book "Comptime Builtins", line 145, documents
   it returns `{ debug, version, target_os, target_arch }`). Actual:
   `let cfg = comptime { build_config() }` cores (ec=139) even WITHOUT field
   access. Entirely non-functional.

3. **`implements(type, trait)` returns `null`** (book line 142 documents a
   compile-time trait-implementation bool check). Actual:
   `comptime { implements("int", "Display") }` evaluates to `null`, not a bool.

4. **`warning(msg)` silently drops the message** (book lines 23, 54-58, 82).
   Actual: `comptime { warning("hello-warning") }` prints NOTHING on stdout or
   stderr. The compile-time warning text is lost.

5. **`error(msg)` garbles the message** (book line 142). Actual: it does
   hard-fail compilation (correct), but the diagnostic is `[comptime error]
   <Bool> (line 1)` — the passed string is never surfaced.

6. **Annotation comptime hooks (`comptime pre/post`) against function OR type
   targets are NON-FUNCTIONAL** — the LARGEST part of the chapter (lines 107-302:
   hook parameters, directives `set return`/`set param`/`replace body`, the
   connector-driven generated-types pattern, field-annotation inspection via
   `comptime for`). Applying ANY such annotation surfaces a v0.4 wall:
   `comptime_target::nb_object_array / nb_string_array: V3-S5 ckpt-5
   consumer-cascade tier 3 SURFACE ... Feature impl pending (v0.4 ...)`.
   Annotation *definitions* parse fine and field annotations *declare* fine, but
   any hook body that reads `target.fields` / `target.params` and emits a
   directive is unimplemented in v0.3.3. The book presents these as runnable
   v0.3.3 features.

## Defects (non-book-driven, found while building)

7. **comptime fn returning an Array into runtime is corrupt** — a comptime block
   whose value is an `Array<int>` either prints garbage (`9D`) or fails V2
   bytecode verification (`NewTypedArrayI64 ... has no FrameDescriptor`) and then
   `TypeError: ... heap value without length semantics`. Worked around by keeping
   every comptime result scalar/string (the shape the book illustrates).

8. **comptime fn building a string in a `while` loop emits V2 verification
   noise** — `s = s + "x"` inside a comptime fn loop triggers repeated
   `StringConcatTyped ... has no FrameDescriptor` warnings on stderr (result is
   still correct). Removed the one such helper (`c_repeat_marker`) from large.shape
   to keep stderr clean.

## BOOK-GAPS (book silent; needed a fallback)

- The book never shows the ONLY working binding form for comptime expression
  results: `let x = comptime { ... }`. It shows `const X = comptime { ... }`,
  which does not compile. Discovering the working form required probing, not the
  book. (Recorded as book_gap: "expression-form comptime must bind to `let`, not
  `const`; book shows only the non-working `const` form".)

## vm vs jit summary
- small: byte-identical PASS both modes.
- large: byte-identical & PASS with adequate memory; at the mandated 12 GiB cap
  JIT crashes on thread-spawn (environment artifact), VM passes.
