# Numeric-Conversion CONFORMANCE Regression Suite

**Status:** PERMANENT regression suite (release-blocking, v0.3.3 strict-typing)
**Rule source:** THE RULE — user 2026-06-01 (binding)
**Spec:** [`numeric-conversion-spec.md`](numeric-conversion-spec.md)
**Baseline:** `shape-strict-flip-collection-dispatch` @ `0cfb1b11`
**Suite location:** `tools/shape-test/tests/numeric_conversions/`
**Run:** `cargo test -p shape-test --test numeric_conversions --no-fail-fast -- --test-threads=1`

This is the **permanent** executable encoding of THE RULE for numeric
conversions. It is the source of truth: it is never relaxed to match a
regressed binary. It was authored as the **TDD red phase** — tests assert THE
RULE's *target* behavior, so the conformance gaps (spec §6) show up as RED
(failing) tests that the GREEN fix workstream must turn green. No checker
source was changed to author the suite (red phase = tests only).

## THE RULE (verbatim intent)

> No "lossless enough." A numeric conversion is implicit ONLY if it is TRULY
> LOSSLESS — every value of the source type is exactly representable in the
> target (strict widening, e.g. `u8 -> u16`, `i32 -> i64`). Everything else —
> `int <-> number` BOTH directions, any lossy width narrowing (`u16 -> u8`,
> `i64 -> i32`), `number -> int` — REQUIRES an EXPLICIT cast (`x as T`). A
> silent lossy conversion is FORBIDDEN (a correctness bug). NUMERIC LITERALS
> adopt their context type when the literal value is losslessly representable
> in it (a small int literal in a `number` context IS a number literal — no
> conversion). Value-level `int != number` holds: an int VARIABLE/VALUE is
> never silently a number.

## Suite layout

| File | Category | What it pins |
|------|----------|--------------|
| `category_a_lossless_widening.rs` | A — LOSSLESS-WIDENING | The §2 lattice IMPL cells ACCEPT and round-trip (u8→u16, i32→int, i32→number, u32→u64, cross-sign widening, identity). |
| `category_b_lossy_implicit_rejected.rs` | B — LOSSY/NON-SUBSET implicit | Every CAST cell WITHOUT an `as` cast must COMPILE-REJECT (int↔number value both ways, number→int, width-narrowing, sign reinterpretation, int64→number, u64→number). |
| `category_c_explicit_casts.rs` | C — EXPLICIT CASTS | `x as T` ACCEPTS with §3 semantics (int as number = f64; number as int = trunc-toward-zero; width casts = two's-complement wrap, 300 as u8 = 44). |
| `category_d_literal_adoption.rs` | D — LITERAL ADOPTION | In-range literal adopts context (number ctx, comparison, match arm, struct field, sized int); out-of-range literal REJECTS (no silent wrap). |
| `category_e_silent_lossy_forbidden.rs` | E — SILENT-LOSSY FORBIDDEN | Data-loss witnesses: each silent conversion the baseline performs is documented with its corrupt value and asserted to require a reject (300→44, -5→65531, 200→-56, int-bits→f64). |

## Assertion strategy

- **ACCEPT cases** (A, C, D-accept) use `expect_number` / `expect_bool` and
  assert the round-tripped/computed value, so a future regression that returns
  a wrong value (e.g. a truncating cast that drops precision) is caught, not
  just a crash.
- **REJECT cases** (B, D-reject, E) use `expect_run_err()` — wording-agnostic.
  The compile/type error surfaces through `ShapeEngine::execute` exactly as the
  sibling `test_width_*_overflow_compile_error` corpus tests assert. This keeps
  the permanent suite decoupled from the not-yet-finalized strict-typing
  diagnostic text; the RED signal today is that the program *runs* with no
  error (silent acceptance).

## RED baseline (captured `0cfb1b11`, `--test-threads=1`, `--no-fail-fast`)

```
test result: FAILED. 55 passed; 48 failed; 1 ignored; 0 measured
```

- **Total cases:** 104 (55 conform now / 48 violate now / 1 ignored).
- **Ignored (1):** `category_e ... e_int_overflow_silent_float_promotion_forbidden`
  — i64-overflow *replacement* semantics are an unresolved open decision
  (blast-radius Bucket C: error vs wrap-to-int vs explicit `as number`).
  `expect_run_err()` only holds under the error-replacement choice, so the
  witness is documented but not asserted until the user rules. The value-level
  invariant it guards is covered unambiguously by E.4.

### Violation breakdown (48 RED = the TDD targets)

| RED category | Count | Spec gap | Example |
|--------------|------:|----------|---------|
| **lossy-implicit-accepted** (B + the E witnesses of the same root) | 20 (B) + 10 (E) | G2/G5/G6/G8 | `b_u16_to_u8_rejected`: `let big:u16=300; let small:u8=big` runs (→44/300) instead of rejecting. `e_i16_neg_to_u16_silent_corruption_forbidden`: `-5` silently → 65531. |
| **casts-rejected** (C int↔number + int-family `as int`/`as number`) | 8 | G3/G4 | `c_int_as_number`: `i as number` → "Cannot assert type 'int' as 'number'". `c_number_as_int_truncates_positive`: `3.7 as int` rejected (should trunc → 3). |
| **literal-not-adopted** (D over-strict) | 4 | G1/FLD1 | `d_number_var_gt_int_literal`: `val:number > 10` → "number is not compatible with int". `d_int_literal_into_number_field`: `P{x:1}` rejected. |
| **out-of-range-literal-silently-wraps** (D under-strict) | 6 | G7 | `d_out_of_range_literal_u8_rejected`: `let x:u8=300` runs (→44) instead of compile-error. `d_out_of_range_reassign_u16_rejected`: `x=70000` → 4464. |

(The width-target casts — `300 as u8 = 44`, `i16 as u16`, `as i16` widening —
already PASS today via `CastWidth`; category A widening and the literal-adoption
ACCEPT cases that lean on the to-be-removed `can_numeric_widen` path also pass
today by value, and are pinned so the GREEN fix preserves them.)

### Cross-reference to the baseline corpus

The corpus siblings `variables_bindings/stress_let_basic.rs::test_width_{i8,u16}_overflow_compile_error`
and `test_width_u8_negative_compile_error` are **also RED on `0cfb1b11`** (the
literal range-check is dormant — spec §6 G7). This suite is the canonical home
for the strict numeric-conversion conformance; the corpus rebaselines
(blast-radius Buckets A/B/C) are tracked separately.

## GREEN exit criterion

When the fix workstream lands (spec §5/§8 touchpoints), this suite must reach
`104 passed; 0 failed` (the 1 ignored test un-ignored once the i64-overflow
replacement is ruled). The suite must NOT be edited to match a partial fix —
only the implementation changes.
