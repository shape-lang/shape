# enums classification

**HEAD:** 82f049dd
**Total tests in binary:** 424
**Passed:** 334 / Failed: 90 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test enums --no-fail-fast 2>&1`
**Source log:** `/tmp/audit_logs/enums.log`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 44 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 19 |
| V0.4-DEFER         | 27 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## SURFACE-shape groups

### Group A — `op_new_array` SURFACE (SCOPE-RECLAIM, 19)

**SURFACE text (verbatim, recurring):**
`Runtime error: Not implemented: op_new_array(N): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. ... Construction-site rebuild lands at ckpt-6 STRICT close ...`

**Cite analysis:** SURFACE cites "V3-S5 ckpt-5 consumer-cascade". The 2026-05-18 dated user pull-in (TAXONOMY.md row 1) explicitly names "V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade" as in-scope v0.3 work. There is no later dated re-disposition to v0.4. Therefore the SURFACE is a mis-cite — these failures route to SCOPE-RECLAIM, not V0.4-DEFER. Tests assert on user-facing semantics (array of enums iterating/matching), so tests stay the same once construction-cascade lands.

**Tests (19):**
- basics::enum_values_in_array
- basics::enum_values_in_array_iterate_and_match
- basics_decl::enum_used_in_for_loop
- basics_decl::test_enum_in_array
- basics_decl::test_enum_in_array_access
- basics_programs::test_complex_enum_multi_step_matching
- matching::test_match_inside_loop
- matching::test_match_on_array_elements
- matching_patterns::match_enum_in_loop_body
- option::test_option_in_array
- result::test_result_in_array
- stress_advanced::test_enum_filter_via_match_in_loop
- stress_advanced::test_enum_from_array_index_match
- stress_advanced::test_enum_payload_sum_in_loop
- stress_advanced::test_enum_variant_in_complex_expression
- stress_match::test_enum_array_length
- stress_match::test_enum_from_array_index_match
- stress_match::test_enum_in_array_access
- stress_match::test_enum_match_in_loop

### Group B — enum `Equal` / `NotEqual` rejected at compile time (FN-REG-CORRECTNESS, 32)

**Failure text (two shapes, same root):**
- `Semantic error: Cannot infer types for binary operation 'Equal': operand types are 'unknown' and 'unknown'. Strict typing requires both operands to have a known concrete type at compile time.`
- `Semantic error: Cannot infer types for binary operation 'Equal': operand types are 'Concrete(Reference(TypePath { segments: ["Color"], qualified: "Color" }))' and 'Concrete(Reference(TypePath { ... }))'. Strict typing requires both operands ...`

**Why FN-REG-CORRECTNESS:** Enum value equality (`Color::Red == Color::Red`, `c1 != c2`) is the canonical Shape user-facing pattern. The compiler now rejects it even when both operands are statically resolved to the same enum type. Affected subsystem: type-inference / binop dispatch for `Type::Reference` enum operands (need `Eq`/`PartialEq` trait wiring for declared enums). NOT in B2 EnumPayload preflight scope (B2 is payload binding, not variant equality).

**Minimal repro:**
```shape
enum Color { Red, Green, Blue }
fn main() {
    let a = Color::Red
    let b = Color::Red
    print(a == b)  // Semantic error: Cannot infer types for `Equal`
}
```

**Affected symbol:** binop typing for `Type::Reference(enum-name)` in `crates/shape-runtime/src/type_system/` (Equal/NotEqual not recognized as defined for enum types).

**Tests (32):**
- basics::enum_equality_different_data_same_variant
- basics::enum_equality_different_unit_variants
- basics::enum_equality_same_data_variant
- basics::enum_equality_same_unit_variant
- basics_decl::test_enum_equality_different_variants
- basics_decl::test_enum_equality_same_variant
- basics_decl::test_enum_inequality_different_variants
- basics_decl::test_enum_inequality_same_variant
- basics_programs::enum_as_function_return
- basics_programs::enum_unit_variant_equality_different
- basics_programs::enum_unit_variant_equality_same
- basics_programs::enum_unit_variant_inequality
- basics_programs::enum_variant_in_let_binding
- stress_advanced::test_enum_different_types_not_equal
- stress_advanced::test_enum_eq_after_fn_roundtrip
- stress_advanced::test_enum_neq_after_fn_roundtrip
- stress_decl::test_enum_10_variant_eq
- stress_decl::test_enum_10_variant_neq
- stress_decl::test_enum_eq_different_variant
- stress_decl::test_enum_eq_same_variant
- stress_decl::test_enum_eq_three_variants_aa
- stress_decl::test_enum_eq_three_variants_ab
- stress_decl::test_enum_eq_three_variants_ac
- stress_decl::test_enum_eq_three_variants_bb
- stress_decl::test_enum_eq_three_variants_bc
- stress_decl::test_enum_eq_three_variants_cc
- stress_decl::test_enum_eq_unit_variants_different
- stress_decl::test_enum_eq_unit_variants_same
- stress_decl::test_enum_eq_via_variables
- stress_decl::test_enum_neq_different_variant
- stress_decl::test_enum_neq_same_variant
- stress_decl::test_enum_neq_via_variables

### Group C — match-arm payload binding has `unknown` type in arithmetic (V0.4-DEFER, 27)

**Failure text (varied operators, common shape):**
`Semantic error: Cannot infer types for binary operation 'Add' / 'Mul' / 'Greater' / 'Less' / 'Div' / 'GreaterEq': operand types are 'unknown' and 'int'. Strict typing requires both operands to have a known concrete type at compile time.`

**Why V0.4-DEFER:** All 27 tests bind a payload from a match arm (`Some(x)`, `Ok(v)`, `Err(e)`, custom-enum payload variants) and then use it in arithmetic / comparison / string concat. The binding's type is lost (`unknown`) — this is precisely the **B2 EnumPayload preflight** territory in the §5.16 JIT-lowering followup workstream named by supervisor 2026-05-25 (TAXONOMY.md line 74: "§5.16 ... actual scope: aliased-CoW SEGFAULT + imported-const ident-eval + W17-marshal + Drop codegen + B2 EnumPayload"). Surface-and-stop is clean (structured semantic error, no panic / SEGFAULT / silent-wrong).

**Recommended v0.4 issue ID:** `TBD-v0.4-b2-enum-payload-preflight` (per task brief).

**Tests (27):**
- matching::test_match_constructor_some
- matching::test_match_nested_if_in_arm
- matching_patterns::constructor_ok_err_function_err_path
- matching_patterns::constructor_ok_err_in_function
- matching_patterns::constructor_payload_used_in_string_concat
- matching_patterns::match_constructor_some_guard_matches
- matching_patterns::match_constructor_some_with_guard
- option::option_from_function_return_none
- option::option_from_function_return_some
- option::option_in_variable_then_match
- option::option_match_in_multiple_functions
- option::option_match_used_as_expression
- option::option_some_match_returns_string
- option::test_complex_accumulate_with_option
- option::test_option_in_conditional
- option::test_option_match_with_computation
- option::test_option_some_with_zero
- result::result_chained_function_calls
- result::result_chained_function_err_propagation
- result::result_from_function_err_path
- result::result_from_function_ok_path
- result::result_match_as_expression
- result::result_ok_takes_success_branch
- result::test_result_as_function_return_err
- result::test_result_as_function_return_ok
- result::test_result_ok_with_computation
- result::test_result_ok_zero_value

### Group D — wire_conversion runtime panic (FN-REG-CORRECTNESS, 2)

**Failure text:**
```
panicked at crates/shape-runtime/src/wire_conversion.rs:201:5:
assertion `left == right` failed: slot kind TypedObject does not match HeapValue::Decimal
  left: Decimal
 right: TypedObject
```

**Why FN-REG-CORRECTNESS:** Hard panic in the wire-conversion path (no SURFACE wrapper, no surface-and-stop). Enum unit-variant declaration triggers a slot-kind / HeapValue mismatch — kind tracker stamped `TypedObject` but the heap payload arrived as `Decimal`. This is a strict-typing slot-kind soundness violation in `crates/shape-runtime/src/wire_conversion.rs:201`.

**Minimal repro:** enum declaration with unit variant that flows through wire-conversion (e.g. printing a unit-variant value).

**Tests (2):**
- basics_decl::test_enum_unit_variant_definition
- basics_programs::enum_unit_variants_declaration

### Group E — Option/Result printing not unwrapping (FN-REG-CORRECTNESS, 2)

**Failure text:**
- `option::option_print_some_unwraps`: expected `42`, got `Some(42)`
- `option::option_print_none`: expected `None`, got `null`

**Why FN-REG-CORRECTNESS:** Silent-wrong-output for the canonical Option print form. Affected subsystem: Option formatter — Some payload not unwrapped, None printing internal `null` instead of variant name.

**Tests (2):**
- option::option_print_none
- option::option_print_some_unwraps

### Group F — Result `!!` context / `?` try-operator broken at runtime (FN-REG-CORRECTNESS, 8)

**Failure shapes:**
- `Error should contain 'context', got: Runtime error: Uncaught error: base` (`!!` not attaching context)
- `Expected run error, but got: Some(Object {"Result": Object {"ok": Bool(false), ...}})` (`?` not propagating Err — returns Result-shaped object instead of erroring)
- `Expected run ok, got error: Runtime error: Uncaught error: ... (line N)` (Err path uncaught when wrapped in Result-returning fn)
- `Expected run ok, got error: No match arm matched the value` (match-arm exhaustiveness regression on Result patterns)
- `Expected 26, got 1` (silent-wrong-output from try-operator chain)

**Why FN-REG-CORRECTNESS:** Result-operator semantics (`!!` error context, `?` Err propagation) broken in user-visible ways. Multiple variants — at least one silent-wrong-output, plus uncaught-error escapes, plus `?` returning success-shape on Err. Affected symbol: Result `?` propagation + `!!` context-attach codegen / runtime in error-handling pipeline.

**Minimal repro:**
```shape
fn divide(a: int, b: int) -> Result<int, string> {
    if b == 0 { Err("base") } else { Ok(a / b) }
}
fn main() {
    let r = divide(10, 0) !! "context"  // expected error message to contain "context", got "base"
}
```

**Tests (8):**
- basics_programs::test_complex_result_error_context_chain
- result::result_err_context_with_bang_bang
- result::result_try_operator_propagates_err
- result::test_combined_context_and_try
- result::test_context_operator_on_err
- result::test_context_operator_on_ok_passes_through
- result::test_try_in_function_returning_result_ok
- result::try_operator_on_err_result
