# Book-Acceptance Report — slice: types-primitive

Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape` (HEAD release, not rebuilt).
Book chapters (PRIMARY): `fundamentals/builtin-types.mdx`, `fundamentals/integer-types.mdx`.
Determinism strategy: pure (no I/O, no time, no randomness).
Run harness: `ulimit -v 12582912; timeout 30 ... run --mode {vm,jit}`.

## Programs

### small.shape (75 LOC)
Exercises the chapter core: scalar `int`/`number`/`bool`/`string`; width-typed
integers (`i8/u8/i16/u16/i32/u32/u64`); literal suffixes; hex/bin/oct + suffix
combos (`0xFFu8`, `0b1010i16`, `0o77u32`); `as` bit-level casts using the book's
verbatim examples (`300 as i8 == 44`, `-1 as u8 == 255` via a variable);
width-typed struct fields (`Pixel { r:u8, ... }`).

- VM:  ec=0, stdout = `ALL_CHECKS_PASSED`
- JIT: ec=0, stdout = `ALL_CHECKS_PASSED`
- vm_jit_byte_identical: YES
- Result: **PASS**

First-run note: an early draft used an empty `if cond { /* comment */ } else { ... }`
block, which is a parse error ("expected a block, found keyword else"). That was
an AUTHOR-ERROR (empty block) — fixed by flipping to `if !cond { ... }`. A real
user would hit and fix this immediately; not a language defect.

### large.shape (704 LOC, 108 asserted checks)
REAL-WORLD APP: a deterministic 8x8 RGBA / luminance image-processing pipeline
built entirely on primitive types. Sections: channel clamp/saturation; Color
type with `u8` fields + pack/unpack via int bit-arithmetic; bit-level `as` cast
matrix; synthetic image generation; histogram; brightness adjust; 3x3 box blur
(edge-replicate); number(f64) luminance ratios with explicit int<->number casts;
threshold/binarize; min/max/range; palette pack round-trip; posterization LUT;
Rec.601 grayscale; run-length encoding (parallel int arrays); Fletcher-16
checksum; full u8/i32 cast round-trip matrix.

Every expected value was hand-derived from BOOK SEMANTICS before the first run
(arithmetic kept in `int` per the book's "general-purpose integer work"
recommendation; `u8` only at boundaries because the book WARNS width-typed
overflow arithmetic is not portable across VM/JIT).

- VM:  ec=0, stdout = `checks_run=108\nALL_CHECKS_PASSED`
- JIT: ec=0, stdout = `checks_run=108\nALL_CHECKS_PASSED`
- vm_jit_byte_identical: YES
- Result: **PASS** (after refactoring around one confirmed defect, see below)

## Expected-value rationale (representative, cite book)

- `300 as i8 == 44`, `-1 as u8 == 255` — integer-types.mdx "Explicit Casting with `as`":
  "narrowing keeps the low bits ... signed/unsigned changes reinterpret the same bits.
  It does not range-check or saturate." Extended matrix (`256 as u8 == 0`,
  `511 as u8 == 255`, `128 as i8 == -128`, `200 as i8 == -56`, `70000 as u16 == 4464`)
  derived from the same low-bits / two's-complement rule.
- `int` division floors; `number` is f64 — builtin-types.mdx "Scalar Types".
  Average-luminance check (`8064 / 64 == 126`) uses int floor; `255.0/2.0 == 127.5`
  and `127.5 as int == 127` uses number then explicit number->int cast.
- Width-typed struct fields (`Pixel { r:u8,g:u8,b:u8,a:u8 }`) — integer-types.mdx
  "In Type Definitions".
- Hex/bin/oct + suffix (`0xFFu8==255`, `0b1010i16==10`, `0o77u32==63`) —
  integer-types.mdx "Literal Suffixes".

## Failure classifications

1. **RE-VALIDATION 2026-06-21 — the earlier "FN-REG-CORRECTNESS defect" is FULLY
   RESOLVED at HEAD: the program compiles AND runs correctly.** Minimal repro:
   `defect_struct_array_field_arith.shape`. At HEAD, run via the exact prescribed
   harness (`bash -c 'ulimit ...; timeout 30 ... run --mode {vm,jit} <file>'`),
   the program returns ec=0 and prints `4` then `5` under BOTH VM and JIT. Field
   read off an indexed element of a `Vec<StructType>` flowing into an arithmetic
   operator (`rs[0].len + 1`) now works. The unannotated `let mut rs = []` +
   `.push(Run{...})` element-type inference succeeds — the strict-flip does NOT
   reject it here.

   CORRECTION of the prior 2026-06-20 note: that note claimed HEAD now rejects
   this at COMPILE time with a strict-typing diagnostic demanding
   `let rs: Array<Run> = []`. That claim is WRONG — it was an artifact of a
   broken heredoc (fish/bash single-quote collision in the test harness), not
   real language behavior. Re-run via the exact env-var harness form, the program
   compiles and runs clean. **Classification: PASS.** No annotation is required;
   large.shape's struct-of-arrays design is correct and in-chapter.

2. **AUTHOR-ERROR** — empty `if` block (small.shape draft); negative-literal cast
   precedence (`-1 as u8` parses as `-(1 as u8) == -1`, not `(-1) as u8`). The book's
   own example uses the variable form (`let signed: int = -1; signed as u8 == 255`),
   which is correct; my literal form was the error. Fixed by binding negatives to a
   variable first (matching the book). Documented inline in large.shape Section 3.

3. **AUTHOR-ERROR** — `checks >= N` self-proof threshold: the argument is evaluated
   at the call site BEFORE `checkb` increments `checks`, so the gate sees the prior
   count. Adjusted threshold; documented inline.

## book_gaps

- **Array TYPE annotations are undocumented.** builtin-types.mdx says `[]` literals
  infer as `Vec<T>` and lists `Vec<T>` as the container type, but NEVER shows how to
  SPELL an array type in a `fn` return / parameter / field annotation. A naive reader
  writes `[int]` (matching the literal syntax). That parses as a **1-tuple `(int)`**
  (per the tuple common-mistake note `[T1,T2,T3]` is tuple syntax) and fails:
  `Generic { Array<T> } is not compatible with (int)`. The working spelling is
  `Vec<int>` (also `Array<int>`), discovered via the llm_summary, not the prose.
  The chapter should show at least one `fn f(xs: Vec<int>) -> Vec<int>` example.

- **No `assert` builtin / self-check idiom is documented.** The deliverable requires
  machine-proof assertions; the book shows no `assert`. I built a `check(name, cond)`
  helper from `if`/`print` (taught elsewhere). The builtin-types chapter could note
  there is no assert and that conditional `print` is the idiom.

- **`.length` / `.push` / indexing on `Vec<T>` are used but not introduced here.**
  builtin-types.mdx lists `Vec<T>` as "ordered homogeneous sequence" but does not
  show element access (`v[i]`), `.length`, or `.push` (the immutable-append needed to
  build a Vec). I relied on prior knowledge / objects-arrays cross-reference. (These
  may be covered in the linked `fundamentals/objects-arrays` chapter — flagged as a
  gap relative to THIS slice's two chapters.)

- **`V2 bytecode verification failed: ... has no FrameDescriptor` warning on stderr**
  for any function that builds a `Vec` via `[] + .push`. Non-fatal (stdout stays
  correct and VM/JIT byte-identical), but undocumented and alarming to a real user.
  The book gives no guidance that this stderr noise is benign.

- **`none` scalar type has no usable value form.** builtin-types.mdx "Scalar Types"
  lists `none` as a scalar type meaning "Explicit absence value", but the lowercase
  `none` value is an **undefined variable** at every position tried: `let x: none = none`
  and `print(none)` both fail with `error[E0101]: Undefined variable: 'none'`; `none()`
  fails as an undefined function. Only the capitalized `None` (Option variant) works, and
  a function that returns nothing is typed `void`, not `none` (`fn f() -> none {...}` fails
  `void is not compatible with none`). The chapter presents `none` as a first-class scalar
  but never shows how to PRODUCE a value of it; a reader cannot use the type the table
  advertises. (Re-verified at HEAD on re-validation run.)

## book_wrong

- None. Every behavior the two chapters DOCUMENT was reproduced correctly
  (scalar types, width types, literal suffixes, hex/bin/oct combos, `as` bit-level
  casts in the book's variable form, width-typed struct fields, `int` floor division,
  `number` f64). The cast "discrepancies" I initially saw were my own
  negative-literal-precedence author error, not a book-wrong: the book's variable-form
  example is correct and reproduces exactly.

## Files written (all under the slice dir)

- small.shape
- large.shape
- defect_struct_array_field_arith.shape
- REPORT.md
