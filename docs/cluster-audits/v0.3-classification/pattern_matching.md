# pattern_matching classification

**HEAD:** 82f049dd
**Total tests in binary:** 230
**Passed:** 197 / Failed: 33 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test pattern_matching --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 4 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 28 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 1 |

## SURFACE-shape groupings

### Group A — SCOPE-RECLAIM: "Cannot infer types for binary operation" inside destructuring / match-extraction (21 tests)

Tests: `destructuring::pm_18_workaround_array_ascending`, `pm_18_workaround_array_element_access`, `stress_advanced::t115_match_recursive_function`, `t121_top_level_let_array_destructure`, `t144_for_loop_array_destructure`, `stress_destructure::t53_match_array_two_elements`, `t54_match_array_three_elements`, `t57_match_array_length_mismatch`, `t60_match_array_nested_computation`, `t78_match_nested_some`, `t81_let_array_destructure_basic`, `t82_let_array_destructure_three`, `t87_let_destructure_from_function`, `t90_let_destructure_in_loop_body`, `t91_let_nested_object_destructure`, `t93_param_object_destructure`, `t94_param_array_destructure`, `t95_param_nested_destructure`, `t96_lambda_object_destructure`, `t97_lambda_array_destructure`, `t98_for_loop_object_destructure`, `t99_param_destructure_three_fields`, `stress_guards::t80_match_option_with_guard`.

Excerpt (representative):
```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`. Strict typing requires both operands to have a known concrete type at compile time.
```

- **User disposition:** 2026-05-21 — "Object destructuring must fully work." Array destructuring pull-in same date (SCOPE-RECLAIM trigger noted in dispatch).
- **SURFACE cite:** No v0.4 anchor cited; bare strict-typing error with no inference path through the destructured binding. Bindings from `let [a, b] = arr` / `let {x} = obj` / match-extracted payloads are losing their element type — `unknown` then propagates into arithmetic.
- **Mis-cite reason:** Destructuring-bound types should flow from container type; loss is in inference, not in v0.4 territory. 2026-05-21 row applies.
- **Test asserts on:** user-facing semantics (test stays the same after fix).

### Group B — SCOPE-RECLAIM: "op_new_array SURFACE — V3-S5 ckpt-5 consumer-cascade" (3 tests)

Tests: `stress_advanced::t125_top_level_object_rest_destructure`, `stress_destructure::t85_let_object_rest`, `stress_destructure::t92_let_array_destructure_mixed_types`.

Excerpt:
```
Runtime error: Not implemented: op_new_array(N): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. … REFUSED ON SIGHT: TypedArrayData resurrection under any rename (Refusal #1).
```

- **User disposition:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade pull-in. Also 2026-05-21 destructuring pull-in.
- **SURFACE cite:** "V3-S5 ckpt-5 consumer-cascade".
- **Mis-cite reason:** Per TAXONOMY 2026-05-18 row — V3-S5 ckpt-5/ckpt-6 op_new_array work is explicitly v0.3 scope; SURFACE message defaults to SCOPE-RECLAIM.
- **Test asserts on:** user-facing semantics.

### Group C — SCOPE-RECLAIM: range builtin SURFACE — V3-S5 ckpt-3 consumer-cascade (1 test)

Test: `stress_literal::t42_match_in_loop`.

Excerpt:
```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 …
```

- **User disposition:** 2026-05-18 V3-S5 construction-cascade pull-in.
- **SURFACE cite:** "V3-S5 ckpt-3 consumer-cascade", "UNREACHABLE until ckpt-6 STRICT close".
- **Mis-cite reason:** V3-S5 ckpt-3..ckpt-6 work is dated v0.3-gating; routes to SCOPE-RECLAIM.
- **Test asserts on:** user-facing semantics.

### Group D — SCOPE-RECLAIM: WrapTypeAnnotation depends on deleted ValueWord (2 tests)

Tests: `basic::pm_02_union_type_match_string`, `basic::pm_02_union_type_match_int`.

Excerpt:
```
Runtime error: Not implemented: SURFACE: WrapTypeAnnotation depends on the deleted ValueWord wrapper type. Annotation wrapping needs a kinded redesign (ADR-006 §2.7.6 / Q8) — see playbook §8 cross-cluster cascade. D-objects-mod scope does not include the compiler emit site.
```

- **User disposition:** 2026-05-22 "W18 content-rendering rebuild into v0.3 (regressions not an option)" + annotations cluster pull-in (TAXONOMY 2026-05-18 row notes annotation_targets + annotations_comptime IS V3-S5 work).
- **SURFACE cite:** ADR-006 §2.7.6 / Q8, "D-objects-mod scope does not include the compiler emit site".
- **Mis-cite reason:** WrapTypeAnnotation is a compiler emit-site bug in v0.3 scope; the SURFACE punts to a sub-cluster boundary rather than a dated v0.4 disposition.
- **Test asserts on:** user-facing semantics.

### Group E — SCOPE-RECLAIM: array rest-pattern (1 test)

Test: `stress_destructure::t86_let_array_rest`.

Excerpt:
```
Semantic error: array rest-pattern (`[a, ...rest]`) is not supported
```

- **User disposition:** 2026-05-21 "Object destructuring must fully work."
- **SURFACE cite:** Bare "not supported" — no v0.4 anchor.
- **Mis-cite reason:** Array rest-pattern is a natural sibling of object destructuring (which IS pulled-in) and shares the same evaluator path. Routes to SCOPE-RECLAIM under the 2026-05-21 row; if supervisor re-dispositions to v0.4 the SURFACE should be re-cited.
- **Test asserts on:** user-facing semantics.

### Group F — FN-REG-CORRECTNESS: wrong runtime result (4 tests)

Plausibly-correct match programs returning wrong values; no SURFACE; no diagnostic — silent-wrong-output.

#### stress_advanced::t106_match_mixed_literal_types
```
Expected 3, got 2
```
- Match selects wrong arm with mixed-literal-type scrutinee. Affected: bytecode match-arm dispatch / literal-pattern comparison.

#### stress_advanced::t110_match_with_function_call_in_arm
```
Expected 9, got 0
```
- Arm body's function call return value lost. Affected: match-arm body codegen / call-result propagation.

#### stress_destructure::t67_match_object_variable_scrutinee
```
Expected 50, got 0
```
- Object-scrutinee variable destructuring returns 0; binding from destructured field is null/uninitialised. Affected: object pattern binding (similar shape to R8 W7 5669a8ff enum-payload match work).

Bisect hint: `git log --oneline -- crates/shape-vm/src/compiler/pattern_matching.rs` (and adjacent match codegen) post-2026-05-21 SCOPE-RECLAIM.

### Group G — UNKNOWN (1 test)

#### stress_advanced::t115_match_recursive_function
```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `int | number` and `int | number`.
```

- Different shape from Group A: operands are `int | number` union (not bare `unknown`). Could be a recursive-function return-type inference regression (`int | number` join not resolving to concrete type) rather than destructuring-binding loss.
- **Blocks classification:** need to see the fixture source — recursive match-on-self may be hitting a generic-fn instantiation issue (known cluster (a) in CLAUDE.md residuals) rather than the strict-typing destructuring loss in Group A.
- **Next step:** read fixture + compare to `stress_generics::generic_identity_*` cluster; if same shape → FN-REG-CORRECTNESS in generic-instantiation cluster; else SCOPE-RECLAIM.

(Reclassified out of Group A on shape distinction; counts above reflect the split.)
