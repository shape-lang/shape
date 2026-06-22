# Book-Acceptance Report — slice: types-primitive

Binary: /home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape (HEAD, prebuilt)
Chapters (book-PRIMARY): fundamentals/builtin-types.mdx, fundamentals/integer-types.mdx
Determinism: pure (no time/random/network; the LCG section is seeded integer-only).
All runs memory-capped (ulimit -v 12 GiB) + timeout 30s.

## Programs

### small.shape (84 LOC)
Exercises the chapter core: int/number/bool/string scalars, width-typed literal
suffixes (i8/u8/i16/i32/u32), hex/bin/oct+suffix combos, and `as` bit-level casts
(300 as i8 -> 44 ; -1 as u8 -> 255), each asserted against a book-derived value.

- VM:  EC=0, stdout = "ALL_CHECKS_PASSED"
- JIT: EC=0, stdout = "ALL_CHECKS_PASSED" (stderr: one documented `[jit-fallback]`
  line — `ConvertToInt` from `as int` is a vm_only opcode; JIT falls through to the
  interpreter exactly as `run --help` documents; NOT silent-no-output).
- vm_jit_byte_identical (stdout): YES.
- Classification: PASS.

### large.shape (519 LOC, 121 asserted checks)
Real-world app: a deterministic binary sensor-telemetry codec + fixed-point
arithmetic engine, rooted entirely in primitive integer types. Sections:
 1. Byte primitives (hi8/lo8/join16/join32/sign_extend16) — bit manipulation on int.
 2. `as`-cast bit-level conformance (the chapter's headline rules: narrow=low bits,
    signed<->unsigned=reinterpret). 300 as i8 -> 44; -1 as u8/u16/u32 -> 255/65535/
    4294967295; 65535 as i16 -> -1; 128 as i8 -> -128; etc.
 3. Frame encode/decode round-trip with mod-256 checksum (Frame A positive temp,
    Frame B negative temp via two's-complement low-16, Frame C corruption detection).
 4. Multi-frame streaming ramp + min/max/sum/mean aggregate (integer-exact).
 5. Fixed-point Q16.16 scaled-integer arithmetic (to_fixed/mul/div/whole/frac_milli).
 6. Deterministic LCG (a=1664525,c=1013904223,m=2^32) — seeded, integer-only.
 7. Integer division/modulo identities + power-of-two shifts (1<<31, 1<<32 exact in i64).
 8. Population count + 8-bit reversal (table-free bit loops).
 9. Width-typed literal edge table (i8/u8/i16/u16/i32/u32 min/max; hex/bin/oct suffixes).

- VM:  EC=0, stdout = "checks_run=121\nALL_CHECKS_PASSED"
- JIT: EC=0, stdout = "checks_run=121\nALL_CHECKS_PASSED" (stderr: one documented
  `[jit-fallback]` — function-local typed-array opcodes lack a FrameDescriptor, so
  the JIT refuses the unverified V2 opcodes and falls through to the interpreter so
  its surface agrees with VM; R8/W7 SURFACE, ADR-006 §2.7.14, tracked for v0.4).
- vm_jit_byte_identical (stdout): YES.
- Classification: PASS.

## Expected-value rationale (all derived from BOOK SEMANTICS before first run)

- `as` is a BIT-LEVEL conversion (integer-types.mdx "Explicit Casting with as"):
  narrowing keeps the LOW bits (book's stated `300 as i8 -> 44`); signed/unsigned
  changes reinterpret the same bits (book's stated `-1 as u8 -> 255`). No range-check,
  no saturate. Every cast assertion in Sections 2/9 follows this rule.
- `int` is signed i64; literals within range store exactly and arithmetic below 2^53
  is exact in both VM and JIT (integer-types.mdx "The int Type" + the large-integer
  Aside). All div/mod, shift, checksum, LCG, and fixed-point constants rely on this.
- Width-typed literals are in-range, so their value equals the plain integer
  (integer-types.mdx "Literal Suffixes"); hex/bin/oct prefixes combine with suffixes.
- Two's-complement byte/word semantics for &, |, ^, <<, >> (bitwise ops are available
  in the language; the book's bit-level cast story is the conceptual anchor).

DISCIPLINE NOTE: during pre-run hand-derivation, five of my own constants were wrong
(Frame A timestamp bytes/checksum, Frame B low byte, LCG s1/s2). They were caught and
corrected by independent exact-arithmetic re-derivation (perl Math::BigInt) BEFORE the
first run — never back-filled from program output. The asserted values encode book
semantics, not observed behavior.

## Author-errors fixed during development (a real user would hit + fix these)
- `assert(...)` is NOT a prelude builtin (RUNTIME: Undefined function: assert).
  Switched to a `check_int/check_bool` helper that prints CHECK_FAILED + counts
  failures, reaching ALL_CHECKS_PASSED only on zero failures.
- Top-level `let SCALE = 65536` is not visible inside top-level `fn` bodies
  (RUNTIME: Undefined variable: SCALE). Top-level functions do not capture module
  locals; inlined the literal. (Consistent, well-diagnosed; not a defect.)
- Operator-precedence subtlety: `-2 as u8` parses as `-(2 as u8)` = -2, not 254.
  Binding first (`let v: int = -2; v as u8` -> 254) matches the book's own idiom
  (`let signed: int = -1; let unsigned = signed as u8`). Not book-wrong; the book
  always binds first. Used the bind-first idiom throughout.

## Language defects encountered (recorded, NOT worked around silently)

These are all in COLLECTION/CODEGEN territory (the v0.3.3 strict-flip + W17 WIP), NOT
in the primitive-types semantics my chapters teach. I restructured the large program
to stay rooted in the primitive slice and recorded each defect here. None changed an
asserted primitive value.

D1. Nested empty-array annotation NOT honored.
    `let mut a: Vec<Vec<int>> = []` -> SEMANTIC error "cannot determine the element
    type of empty array" DESPITE the explicit concrete annotation. Single-level
    `let mut a: Vec<int> = []` works. 3-line repro. (Outside slice: containers.)

D2. Function-local typed-array opcodes lack FrameDescriptor.
    Any `[...]`/`.push()`/`[i]=` inside a `fn` body emits "V2 typed opcode
    NewTypedArrayI64/TypedArrayPushI64/SetElemI64 ... has no FrameDescriptor"
    verification warnings to stderr (result still correct under VM; this is what
    forces the JIT `[jit-fallback]`). (Outside slice: codegen.)

D3. Function-returned Vec<int> bound to a module `let mut` then index-mutated hits an
    unimplemented SURFACE stub (HARD error):
      `Not implemented: SURFACE: SetModuleBindingIndex requires the W17-typed-carrier-
       monomorphization replacement ... (ADR-006 §2.7.24 Q25.A). Key kind: Int64`
    5-line repro:
      fn mk() -> Vec<int> { return [10,20,30] }
      let mut a = mk(); a[1] = 99; print(a[1])   // -> Not implemented SURFACE
    This is the known release-blocking W17 unfinished work. (Outside slice; recorded.)

## book_gaps
- builtin-types.mdx "Notes" states "`[]` literals infer as `Vec<T>`" but neither
  chapter mentions that a DEFERRED-PUSH empty array (`let mut a = []` then `.push`)
  requires an explicit `Vec<T>` / `Array<T>` annotation under strict typing, and that
  a NESTED empty array `Vec<Vec<int>> = []` is currently rejected even WITH the
  annotation (D1). A reader following the Notes verbatim hits a SEMANTIC error.
- Neither chapter teaches an assertion/test mechanism, so self-checking programs must
  invent one (`assert` is not in the prelude). Minor — out of these chapters' scope —
  but every machine-proofable program in this slice needs it.
- Neither chapter documents bitwise operators (&, |, ^, <<, >>) on `int`. The
  integer-types chapter leans entirely on `as` for the "bit-level" story; a reader
  doing real integer/protocol work (the natural application of width types, which the
  chapter explicitly motivates with "binary protocol work") has to discover &/<</>>
  elsewhere. They DO work; the chapter is silent. (Used MCP/reference-free probing.)
- integer-types.mdx "Explicit Casting" shows `300 as i8` directly in prose but does
  not warn that unary-minus binds looser than `as` (`-2 as u8` = -2, not 254). A
  reader writing `-2 as u8` literally gets a surprising result; the book's worked
  examples happen to bind to a variable first, sidestepping it.

## book_wrong
- (none) Every documented primitive behavior I followed verbatim produced exactly the
  book's stated result: 300 as i8 = 44, -1 as u8 = 255, the suffix table, hex/bin/oct
  combos, int exactness within i64. No case where the book documents something the
  language does not do for the PRIMITIVE-type semantics these chapters cover.
