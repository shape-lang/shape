# THE RULE — shape-test corpus blast-radius scan

**Date:** 2026-06-01
**Baseline:** strict-flip-collection-dispatch converged @ `0cfb1b11`
**Binary probed:** `target/release/shape run --mode vm <file>`
**Corpus root:** `tools/shape-test/tests/` (325 `.rs` files, ~11.8k tests; tests embed Shape source as Rust string literals)

## THE RULE (user 2026-06-01, binding)

A numeric conversion is **implicit** only if **truly lossless** (every value of the source
type is exactly representable in the target — strict widening, e.g. `u8->u16`, `i32->i64`).
Everything else requires an **explicit cast** (`x as T`):

- `int <-> number` **both** directions — explicit cast.
- any lossy width narrowing (`u16->u8`, `i64->i32`, `number->int`) — explicit cast.
- A **silent lossy** conversion is FORBIDDEN (a correctness bug).
- **NUMERIC LITERALS adopt their context type** when the literal value is losslessly
  representable in it (a small int literal in a `number` context IS a number literal —
  no conversion). Out-of-range literals into a sized type do NOT adopt — they reject.
- **Value-level `int != number`** holds: an int VARIABLE/VALUE is never silently a number.

## Current-binary gaps confirmed (probes)

| Probe | Program | Current binary | THE RULE | Delta |
|---|---|---|---|---|
| P1 | `let val:number=5.0; val > 10` | REJECT (`number not compatible with int`) | ACCEPT (literal `10` adopts number) | over-strict on literal |
| P2 | `let i:int=7; let n:number=i` | **ACCEPT** (prints 7) | REJECT (value-level int->number) | silent acceptance bug |
| P3 | `i as number` | REJECT (`Cannot assert type 'int' as 'number'`) | ACCEPT (explicit cast) | cast machinery broken |
| P4 | `n as int` | REJECT (`Cannot assert type 'number' as 'int'`) | ACCEPT (explicit cast) | cast machinery broken |
| P5 | `let n:number=2.5; n + 3` | ACCEPT (5.5) | ACCEPT (literal `3` adopts) | OK |
| P6 | `let i:int=4; let n:number=2.5; n + i` | **ACCEPT** (6.5) | REJECT (int var in number arith) | silent acceptance bug |
| FLD1 | `type P{x:number}; P{x:1}` | REJECT (`cannot construct field x ... with int literal`) | ACCEPT (literal adopts) | over-strict on literal |
| FLD2 | `let i:int=7; P{x:i}` | **ACCEPT** (7.0) | REJECT (int var into number field) | silent acceptance bug |
| T1 | `fn f()->number{ let i:int=5; i }` | **ACCEPT** (5) | REJECT (return int var as number) | silent acceptance bug |
| T2 | `let n:number=3.0; let i:int=n` | REJECT (already) | REJECT | OK (no change) |
| LEN1 | `floatsum / arr.len()` | **ACCEPT** (2.0) | REJECT (number / int mix) | silent acceptance bug |
| P10 | `let big:u16=300; let small:u8=big` | **ACCEPT** (44 — wraps) | REJECT (lossy narrowing) | silent data loss |

**Key asymmetry:** the current binary is *over-strict on int LITERALS* into number context
(rejects what THE RULE accepts) and *under-strict on int VALUES/VARIABLES* into number
context (silently accepts what THE RULE rejects, with real corruption — int bits read as f64).

## Crucial corpus context: the strict-flip already migrated most int->number sites

During the strict-flip, the corpus was extensively migrated under the `c2-A` / `c2a-cluster`
fixes (search `c2a-cluster sub-fix`). Those fixes made BOTH the literal form (`Point{x:1}`)
AND the int-var form (`p.x = v`) compile-errors, and migrated callers to `1.0`/`as`-style.
Consequence for THIS task:

- The **int-VAR -> number** rebaselines were *already done* by the strict-flip — those sites
  now use number literals or are already negative-assertion tests. They are NOT "newly
  rejected by THE RULE"; the baseline already rejects them.
- What remains genuinely affected by THE RULE in the corpus splits into the four buckets below.

## Buckets

### Bucket A — Over-strict-literal negative tests that THE RULE FLIPS to accept (4 tests)

The baseline asserts that an int LITERAL into a number field/slot is a compile error.
THE RULE says the literal adopts context -> these must be re-baselined (invert the
`expect_run_err_contains(...)` to a success assertion). These are the *opposite-direction*
rebaselines — listed because THE RULE directly contradicts the assertion.

- `structs_types/structs.rs:209` `struct_literal_int_to_number_rejected_at_compile_time` — `Point{x:1,y:2}` asserts `"with int literal"` reject.
- `structs_types/structs.rs:161` `struct_field_mutation_int_to_number_rejected_at_compile_time` — `p.x = 10` (literal) asserts reject.
- `structs_types/stress_fields.rs:33` `object_creation_int_to_number_rejected_at_compile_time` — `Point{x:1,y:2}` asserts `"with int literal"` reject.
- (counterpart `structs_types/structs.rs:184` `struct_field_mutation_int_var_to_number_rejected_at_compile_time` — `p.x = v` with int VAR — STAYS rejecting; RULE-aligned, NOT a rebaseline.)

NOTE: this bucket is a TP-rebaseline only in the sense that the test's *assertion* must
change. The migrated callers (many `0`->`0.0`, `5`->`5.0` edits across structs_types/,
complex_integration/, extend_blocks/, native_interop/) need no change — `.0` literals still
type as number.

### Bucket B — Silent width-narrowing on out-of-range literal reassignment (3 tests)

These assert silent wrap/truncate as CORRECT — exactly the "silent lossy conversion is
FORBIDDEN" the rule bans. The out-of-range literal does NOT adopt the sized type; it must
reject. `expect_number(<wrapped>)` must become an error expectation.

- `variables_bindings/stress_let_basic.rs:224` `test_width_var_reassign_truncates_u8` — `let mut x:u8=10; x=300` asserts `44.0`.
- `variables_bindings/stress_let_basic.rs:237` `test_width_var_reassign_truncates_i8` — `x=200` asserts `-56.0`.
- `variables_bindings/stress_let_basic.rs:250` `test_width_var_reassign_truncates_u16` — `x=70000` asserts `4464.0`.

(Sibling negative tests already exist and STAY: `test_width_i8_overflow_compile_error`
:180, `test_width_u16_overflow_compile_error` :192 — `let x:u8=128`/`u16=65536` already
`expect_run_err`. The truncate tests are the inconsistency.)

### Bucket C — Silent int-arithmetic-overflow promote-to-f64 (6 tests)

`int + int` that overflows i64 silently promotes the RESULT to f64 (number). THE RULE:
"an int VALUE is never silently a number"; int->number is an explicit cast. These assert
the forbidden silent promotion. Likely the runtime behavior changes (overflow should
error or stay int with explicit `as number` opt-in), and these tests rebaseline.

- `operators/stress_add_sub.rs:200` `overflow_add_promotes_to_float` — `9007199254740990 + 10` asserts `9007199254741000.0`.
- `operators/stress_add_sub.rs:211` `overflow_mul_promotes_to_float`.
- `operators/stress_add_sub.rs:222` `overflow_sub_promotes_to_float`.
- `operators/arithmetic.rs:152` `integer_overflow_promotes_to_float`.
- `literals/stress_integers.rs:259` `test_int_overflow_promotes_to_f64` — `let a:int=140737488355327; a+1` asserts `140737488355328.0`.
- `type_inference/stress_inference_complex.rs:561` `int_overflow_promotes_to_f64`.

### Bucket D — Existing `int as number` / `number as int` cast tests (FIX-validation, not rebaseline)

The cast path is currently broken (P3/P4). THE RULE makes these casts the canonical
mechanism, so fixing the cast machinery RE-ENABLES these tests. They are not new
rejections — they should start passing once `as` works. They are listed so they are not
mistaken for rebaselines.

- `error_handling/diagnostics.rs:230` `infallible_cast_int_to_number_no_semantic_diagnostics` — `let n = 42 as number`.
- `error_handling/diagnostics.rs:272` `option_int_as_number_no_semantic_diagnostics` — `opt as number`.
- `error_handling/diagnostics.rs:284` `result_int_as_number_no_semantic_diagnostics` — `res as number`.
- ~41 total `as number`/`as int` occurrences corpus-wide (mostly in error_handling/, many inside `impl Into/TryInto` bodies).

## Populations explicitly EXCLUDED from rebaseline (large benign sets)

- **Literal-vs-literal mixed arithmetic / comparison** — `1 + 2.0`, `2.0 * 3`, `1 == 1.0`,
  `1 < 2.5`, `2 <= 2.0`, etc. (`operators/stress_compound_mixed.rs:204,211`;
  `operators/stress_equality.rs:84,90,260,266,272,278`). Both operands are literals; the
  int literal adopts number context -> ACCEPT under THE RULE. Current binary already
  accepts (probed). No change.
- **`.len()` results** (~50 sites) — used directly as int (printed/returned) or compared
  with int (`i < arr.len()`). None feed a number context. No average-idiom (`/ .len()`)
  found in the corpus.
- **Pre-migrated number-field constructions** (`0.0`/`5.0`/`100.0`, the many `c2a-cluster
  sub-fix (i)` edits) — already number literals. No change.
- **Sized-int LITERAL bindings in range** (`let a:u8=200`, `let a:u16=50000`,
  `variables_bindings/stress_let_basic.rs:102-174`) — losslessly representable literal
  adoption. No change.

## Sizing

| Bucket | Tests | Module distribution |
|---|---:|---|
| A — over-strict-literal negatives (assertion flip) | 4 | `structs_types/` (3), `structs_types/stress_fields.rs` shared |
| B — silent width-narrowing truncate | 3 | `variables_bindings/stress_let_basic.rs` |
| C — silent overflow promote-to-f64 | 6 | `operators/` (4), `literals/` (1), `type_inference/` (1) |
| D — `as` cast fix-validation (NOT rebaseline; should start passing) | ~3 named + ~41 sites | `error_handling/` |

**Precise rebaseline count (Buckets A+B+C): 12 corpus tests** (A=3 + B=3 + C=6; the
`struct_field_mutation_int_var_to_number_rejected` int-VAR test is RULE-aligned and is NOT
counted) that need their assertions changed under THE RULE, plus the cast machinery fix
(P3/P4) that un-breaks the ~3 named Bucket-D tests. (Confirmed: no sized-int
VAR-to-narrower-VAR assignments exist anywhere in the corpus, so the only width-narrowing
rebaselines are the 3 out-of-range-literal-reassignment "truncate" tests in Bucket B.)

This is SMALL relative to the strict-flip's own ~1105-FP blast radius, because the
strict-flip already chased the value-level int->number flows during the `c2-A`/`c2a-cluster`
migration. The dominant residual is the inconsistency between the strict-flip's
over-strict literal handling (rejects what THE RULE accepts) and its silent-truncation /
silent-overflow-promotion leftovers (accepts what THE RULE forbids).

## Concrete examples (5-8)

1. `structs_types/structs.rs:209` `Point{x:1,y:2}` -> baseline rejects "with int literal"; THE RULE accepts (literal adopts). Flip assertion.
2. `structs_types/structs.rs:161` `p.x = 10` (number field) -> baseline rejects; THE RULE accepts. Flip.
3. `variables_bindings/stress_let_basic.rs:224` `let mut x:u8=10; x=300` -> baseline returns `44`; THE RULE rejects (300 not representable in u8). Change to error.
4. `variables_bindings/stress_let_basic.rs:250` `let mut x:u16=0; x=70000` -> baseline returns `4464`; THE RULE rejects.
5. `operators/stress_add_sub.rs:200` `9007199254740990 + 10` -> baseline returns `...741000.0` (silent int->f64); THE RULE: int overflow is not silently a number.
6. `literals/stress_integers.rs:259` `let a:int=140737488355327; a+1` -> baseline `...355328.0`; THE RULE: silent promotion forbidden.
7. `error_handling/diagnostics.rs:230` `let n = 42 as number` -> baseline REJECTS (broken cast); THE RULE accepts. Fix-validation (starts passing).
8. `structs_types/structs.rs:184` `p.x = v` (int VAR into number field) -> already rejects; RULE-aligned; NOT a rebaseline (shown for contrast with example 2's literal form).
