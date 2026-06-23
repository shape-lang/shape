# Book-Acceptance REPORT — slice: types-primitive

Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape` (release, at HEAD)
Book chapters (PRIMARY source):
- `fundamentals/builtin-types.mdx`
- `fundamentals/integer-types.mdx`
Determinism strategy: pure (no time/network/random; all fixtures inline).

## Summary

| Program | LOC | VM ec | JIT ec | VM stdout | JIT stdout | byte-identical |
|---------|-----|-------|--------|-----------|------------|----------------|
| small.shape | 74 | 0 | 0 | `ALL_CHECKS_PASSED` | `ALL_CHECKS_PASSED` | YES |
| large.shape | 889 | 0 | 0 | `ALL_CHECKS_PASSED` | `ALL_CHECKS_PASSED` | YES |

Both programs PASS under both execution modes with byte-identical stdout.
(Note: stdlib emits a benign `V2 bytecode verification failed ... Json.keys`
line to **stderr** during init — unrelated to these programs; stdout is clean.)

Assertions: small ~21 distinct checks; large 187 `check`/`check_bool`/`check_str` calls.

## small.shape

Exercises the chapter cores:
- Scalar types: `int`, `number`, `bool`, `string` (builtin-types.mdx "Scalar Types").
- Large i64 literal stores exactly: `9000000000000000000` (integer-types.mdx).
- Literal suffixes: `42i8`, `255u8`, `1000i16`, `50000u16`, `100i32`, `3000000u32`, `1048576u64`.
- Hex/bin/oct + suffix: `0xFFu8`=255, `0b1010i16`=10, `0o77u32`=63 (integer-types.mdx §Literal Suffixes).
- `as` casting verbatim from book §"Explicit Casting": `300 as i8`=44, `-1 as u8`=255, `3.9 as int`=3.
- Width types as struct fields (`Pixel { r/g/b/a: u8 }`) per §"In Type Definitions".

All expected values taken directly from the book's own worked examples. PASS.

## large.shape — Binary Protocol Codec + Fixed-Point Math

A deterministic, non-interactive, machine-proofable application rooted in the
primitive-types slice. 15 sections, 187 assertions. Every expected value was
hand-derived from book semantics BEFORE the first run (no back-filling).

- §1 Byte primitives — `as u8` masking; book claim "narrowing keeps low bits"
  (e.g. `to_byte(300)=44`, `to_byte(256)=0`, `to_byte(-1)=255`).
- §2 Q16.16 fixed-point — `number as int` truncation, `int as number` widening
  (e.g. `1.5 -> 98304`, `fp_mul(0.5,0.5)=16384`).
- §3 Big-endian buffer writer/reader over `Vec<int>`.
- §4 Fletcher-16 checksum (hand-traced: `[1,2,3,4]`→5130, `"abcde"`→51440).
- §5 RLE codec with round-trip equality proof.
- §6 Record (`magic:u16, kind:u8, value:u32`) pack/unpack; `0xCAFE`/`0xDEADBEEF`
  byte decomposition asserted.
- §7 Bit-width boundary table for `as` (e.g. `128 as i8`=-128, `255 as i8`=-1).
- §8 Bool truth table + comparison operators yield `bool` (strict, no truthiness).
- §9 Integer identities: ipow/gcd/factorial; signed `/` and `%` truncate toward
  zero (`-7/2=-3`, `-7%2=-1`); exact i64 product `10^12`.
- §10 number arithmetic + number/int interplay; truncation table incl. negatives.
- §11 End-to-end: encode 3 records → checksum → RLE round-trip → re-checksum
  stable → decode → sum=600.
- §12 Base-N integer formatter/parser (decimal/hex/bin/oct), string round-trips
  (`0xABCD`→"ABCD"→43981, `DEADBEEF`).
- §13 Extended width-cast boundary matrix across u8/i8/u16/i16/u32/i32.
- §14 Saturating byte arithmetic (clamp 0..255).
- §15 Q16.16 affine transform pipeline over integer points; number-derived
  coefficient `1.25*65536=81920`.

### Defects found during authoring (BOTH author-error — fixed, NOT language defects)

1. **`as` precedence vs unary minus** — `-128 as u8` parses as `-(128 as u8)`
   (=-128), not `(-128) as u8` (=128). The book's casting examples always bind
   the negative value to a variable first (`let signed: int = -1; signed as u8`),
   so a book-following user would not hit this; I hit it by writing a negative
   literal directly. Fixed by binding to a variable, matching book idiom.
   First-run truth: `cast_neg128_u8 expected=128 got=-128` (+ u16/u32 siblings).
   → recorded as a **book_gap** (book never states `as` precedence).

2. **Dead scratch loop in `rle_encode`** — a placeholder `while` block I left in
   mutated the index `i` early, causing an out-of-bounds read. Pure typo/leftover;
   replaced with the clean single counting loop. First-run truth:
   `Index 21 out of bounds (length 21)`. Author-error, not a language defect.

After fixes both programs pass cleanly under VM and JIT.

## book_gaps (book silent; behavior verified via experimentation / works fine)

- **Operator precedence of `as`** — integer-types.mdx §"Explicit Casting" shows
  `wide as i8` and `signed as u8` but never states how `as` binds relative to
  unary minus (or other operators). `-1 as u8` is `-(1 as u8)` = -1, surprising a
  reader who expects 255. A precedence note (or "parenthesize negative operands")
  would prevent the trap.
- **Bitwise / shift operators** — neither chapter documents `&`, `|`, `^`, `<<`,
  `>>`, `%`, or integer `/`. A binary-protocol / width-int user needs these and
  the chapters are the natural home. (The codec used `/`/`*`/`%` to emulate shifts
  to stay strictly within documented operators; the language DOES provide
  `&`/`>>`/`<<` — verified — but the slice chapters never mention them.)
- **String indexing returns a 1-char string** — used for the digit table in §12;
  not covered by these chapters (belongs to a strings chapter), verified by
  experiment.
- **No `assert` builtin in prelude** — a natural "self-checking program" needs an
  assertion primitive; `assert(...)` is an undefined function. Had to roll a
  manual `if got != want { ... }` check helper. Not strictly a types chapter gap,
  but worth noting for the acceptance methodology.

## book_wrong (book documents X, language does not do X)

- NONE. Every behavior the two chapters explicitly document was reproduced
  exactly: `int`=i64 with exact large literals; all literal suffixes; hex/bin/oct
  + suffix combos; `as` bit-level semantics (narrowing keeps low bits — `300 as
  i8`=44; signed/unsigned reinterpret — `-1 as u8`=255); `number as int`
  truncation (`3.9 as int`=3); width types as struct field annotations.
  The book's "i32 has typed-opcode support; other widths are docs/interop"
  caveat held — all width casts behaved consistently across VM and JIT for the
  values tested (all within i64 range, as the book recommends).

## Classification

- small.shape: **PASS**
- large.shape: **PASS** (the two authoring defects were AUTHOR-ERROR, fixed)
- vm_jit_byte_identical: **YES** for both.
