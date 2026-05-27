# lsp classification

**HEAD:** 82f049dd
**Total tests in binary:** 385
**Passed:** 361 / Failed: 24 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test lsp --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 8 |
| FN-REG-DIAGNOSTIC  | 4 |
| SCOPE-RECLAIM      | 12 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### completions::completion_impl_block_suggests_default_methods

Class: **SCOPE-RECLAIM**

```
Expected completion 'execute' in []
```

- Dated user pull-in: 2026-05-26 LSP-parity-with-rust-analyzer.
- Completion engine returns empty list inside `impl Trait for Type { }`. LSP territory. Same root cause as `trait_system::completion_inside_impl_suggests_methods`.

### code_lens::trait_definition_shows_lens

Class: **SCOPE-RECLAIM**

```
Expected at least one code lens
```

- 2026-05-26 LSP-parity. Code-lens not emitted on trait definitions.

### completions::completion_impl_block_suggests_unimplemented_methods

Class: **SCOPE-RECLAIM**

```
Expected completion 'select' in []
```

- 2026-05-26 LSP-parity. Twin of `completion_impl_block_suggests_default_methods`.

### combined::test_lsp_combined_diagnostics_and_error

Class: **FN-REG-DIAGNOSTIC**

```
Expected semantic diagnostic containing 'Could not solve type constraints', found: ["Type constraint violation: parameter at position 0 of 'bad' must be numeric (its body requires a Numeric operand), but a call site passes the non-numeric type 'Union(...)'", "Cannot infer types for binary operation `Add`: operand types are `unknown` and `int`. ..."]
```

- Old expected substring: `Could not solve type constraints`.
- New actual: `Type constraint violation: parameter at position 0 ... must be numeric ...`.
- Language change: numeric-constraint diagnostic was rewritten to be more specific. Same shape applied across 5 sibling tests below.

### diagnostics::function_param_numeric_constraint_rejects_object_callsites
### diagnostics::empty_match_arm_does_not_suppress_following_numeric_diagnostic
### diagnostics::function_param_numeric_constraint_with_print_reports_on_numeric_line
### diagnostics::function_param_numeric_constraint_with_typed_match_reports_assignment_line
### diagnostics::test_lsp_diagnostic_type_constraint_error
### hover::hover_signature_preserves_callsite_union_under_numeric_conflict

Class: **FN-REG-DIAGNOSTIC** (6 tests — same shape as `test_lsp_combined_diagnostics_and_error`)

- All expect `Could not solve type constraints` substring; all actuals are the new `Type constraint violation: parameter ... must be numeric` form. Same diagnostic-rewrite. Fixtures stale.

### diagnostics::generated_method_call_from_comptime_extend_has_no_semantic_diagnostics

Class: **SCOPE-RECLAIM**

```
Expected no semantic diagnostics, found: [("comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per docs/v0.3-close-summary.md §5.16 JIT-lowering followup workstream).", ...)]
```

- Dated user pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 + 2026-05-22 comptime trait into v0.3.
- SURFACE cites `v0.4 / planned per §5.16 JIT-lowering followup workstream` for comptime-extend method generation over a `nb_object_array` carrier. Per CLAUDE.md / TAXONOMY: §5.16 scope = aliased-CoW + imported-const ident-eval + W17-marshal + Drop codegen + B2 EnumPayload only. comptime-extend + nb_object_array is V3-S5 ckpt-5 construction-cascade territory (2026-05-18 pull-in) + comptime trait (2026-05-22) — NOT §5.16. MIS-CITE → SCOPE-RECLAIM.

### hover::definition_from_impl_method_name, definition_from_impl_trait_name, definition_from_trait_name

Class: **SCOPE-RECLAIM** (3 tests)

```
Expected definition at ...
```

- 2026-05-26 LSP-parity. Trait/impl go-to-definition not wired.

### completions::completions_after_dot_on_array_via_resilient_parse

Class: **FN-REG-CORRECTNESS**

```
Expected completion 'len' in ["first", "last", "push", "pop", "reverse", "clone", "filter", "map", "reduce", "find", "forEach", "some", "every", "join", "slice", ...]
```

- Dated user pull-in: 2026-05-21 "Len trait" must work. Array completion list omits `len`. The Len-trait pull-in 2026-05-21 explicitly named this. Classed CORRECTNESS not SCOPE-RECLAIM because the test fails by absence (Len not yet attached to Array PHF) — the underlying user pull-in says `len` must work; not having it in completion is a direct failure of that disposition, not a SURFACE-and-stop. (Either classification routes to release-blocking; CORRECTNESS more accurately frames the user-visible miss.)

### hover::hover_default_method_shows_default_indicator

Class: **FN-REG-DIAGNOSTIC**

```
Hover should contain 'default', got: **Function**: `execute` ... **Signature:** execute(addr, code) -> Result<...>
```

- Hover renders the method correctly but omits the "default" tag marker for default trait methods. LSP fixture-text expectation; behavior is otherwise correct. Diagnostic-class fixture update OR LSP feature work — chose DIAGNOSTIC because hover content is structurally present and only the textual annotation is missing.

### presentation::code_lens_on_trait_at_correct_line, code_lens_on_trait_shows_implementations

Class: **SCOPE-RECLAIM** (2 tests)

- 2026-05-26 LSP-parity. Trait code-lens not emitted.

### presentation::inlay_hint_closure_param_is_refined_from_body_constraints

Class: **FN-REG-CORRECTNESS**

```
Expected type hint ': ({ x: number }) -> number', found: [": { x: int }", ": fn(_) -> number"]
```

- Closure param-inference picks `int` for `{ x: int }` instead of `{ x: number }` based on body constraints. Bidirectional closure inference regression — body constraints don't propagate back to param. Plausibly-correct code; this is the kind of inference the 2026-05-21 + 2026-05-26 LSP-parity dispositions assumed.

### comptime::generated_method_call_from_comptime_extend_executes

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).
```

- MIS-CITE of §5.16 (not in scope per TAXONOMY). Underlying work = V3-S5 ckpt-5 + comptime trait. Both in-scope 2026-05-18 / 2026-05-22. SCOPE-RECLAIM. Twin of `generated_method_call_from_comptime_extend_has_no_semantic_diagnostics` above.

### hover::hover_on_bounded_type_param_shows_bound_names
### hover::hover_on_impl_method_shows_trait_signature
### hover::hover_on_bounded_type_param_shows_traits
### hover::test_lsp_hover_bounded_type_param

Class: **SCOPE-RECLAIM** (4 tests)

```
Expected hover at (3, 7)  /  (4, 11)
```

- All 2026-05-26 LSP-parity territory. Bounded-type-param hover + impl-method-shows-trait-signature hover all not wired.
