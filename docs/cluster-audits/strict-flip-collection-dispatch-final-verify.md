# strict-flip collection-dispatch — FINAL VERIFY

Worktree: `shape-strict-flip-collection-dispatch` @ `42a9365d` (cumulative strict-flip).
Base: `f20f9f98` (collection-dispatch migration-debt close).
Date: 2026-06-14. All integration binaries run `--test-threads=1` (deterministic).

## Gates (all green)

| Gate | Result |
|------|--------|
| `just check-clean` | EXIT 0 |
| `just check-no-dynamic` | EXIT 0 |
| `bash scripts/verify-merge.sh` | 13/13 PASSED, EXIT 0 |
| `no_dynamic` sentinel | 1/1 ok |

## Preserved invariants

| Invariant | Base | HEAD |
|-----------|------|------|
| numeric_conversions | 104/0 | 104/0 |
| borrow_refs | 215/4 | 215/4 (same 4, pre-existing) |
| smoke_test | 8/8 | 8/8 |
| s1–s5 VM==JIT | 5/5 | 5/5 IDENTICAL |
| sf-NEW (new strict-flip regressions) | — | 0 |

s1–s5 VM==JIT (each output byte-identical under `--mode vm` and `--mode jit`):
- s1 R5a `let x: Array<number>=[1,2.5,3]` → `[1.0, 2.5, 3.0]`
- s2 R2 immutable HashMap builder `m.set("a",1).set("b",2)` → `1`/`2`
- s3 R3 `[5,2,8].sort().map(|x| x*x)` → `[4, 25, 64]`
- s4 R4 `words.reduce(|a,w| a+w, "")` → `foobarbaz`
- s5 R5c `user.name = "Bob"` → `Bob`

## F7 soundness (no broad-suppression / no unsoundness introduced)

- `int != number`: `let x:int=1; let y:number=2.0; x+y` → REJECT (int not compatible with number). HOLDS.
- lossy literal into int array: `let a:Array<int>=[1,2.5,3]` → REJECT (number not compatible with int). HOLDS.
- `"x" + 3` → `x3`: PRE-EXISTING on base (verified identical base vs HEAD); NOT introduced by R4. R4 only widened the accepted *string carrier* (String / Ptr(HeapKind::String) / StringV2) on the `string+string` path; it did not touch string-concat coercion policy. Separate pre-existing concern, out of scope here.
- No skeptic flagged any fix UNSOUND. R1-pushreassign was correctly SURFACED + reverted (heap corruption at module scope); tree clean.

## Per-root pass-delta (integration, base → HEAD)

| Group | Base | HEAD | Δ | NEW regr |
|-------|------|------|---|----------|
| arrays_vectors | 324/61 | 327/58 | +3 | 0 |
| objects | 21/3 | 22/2 | +1 | 0 |
| objects_arrays | 71/22 | 79/14 | +8 | 0 |
| type_inference | 268/27 | 270/25 | +2 | 0 |
| complex_integration | 50/50 | 52/48 | +2 | 0 |
| structs_types | 248/33 | 251/30 | +3 | 0 |
| **Total red→green** | | | **+19** | **0** |

Fixed test names: arrays_vectors {stress_chained::test_pipeline_top_3_squares, stress_creation::test_array_first_last_same_on_single, stress_mutation::test_array_for_in_strings}; objects {operations::object_computed_key}; objects_arrays {arrays::array_reduce_string_concat, objects::hashmap_basic_creation_and_get, objects::hashmap_delete, objects::hashmap_has_key, objects::hashmap_has_missing_key, objects::hashmap_immutability, objects::hashmap_len, objects::object_property_assignment}; type_inference {collections::test_hashmap_delete_key, collections::test_hashmap_delete_preserves_other_keys}; complex_integration {pattern_based::test_complex_pipeline_transform_filter_reduce, real_world::test_program_config_merger}; structs_types {stress_fields::anon_object_field_mutation_string, stress_methods::anon_object_multiple_string_fields, stress_methods::closure_captures_struct}.

## Per-root unit-test confirmation

| Root | Commit | Status | Unit tests |
|------|--------|--------|-----------|
| R5a-literal | cd8f84a6 | FIXED | array_emission 28/0, no_dynamic 1/1 |
| R1-pushreassign | (reverted) | SURFACED — needs design (empty-array element-type inference / let-gen layer) | n/a, tree clean |
| R2-chainbuilder | 234fbfb0 | FIXED | 7 rebaselined immutable + 2 MIR (builder_on*) all ok |
| R3-elemerasure | aeaf1916 | FIXED | r3_elemerasure 5/5 |
| R4-stringiter | f079435f | FIXED | typed_access 5/5 |
| R5c-objfield | 42a9365d | FIXED | (verified via integration: object_computed_key, anon_object_*) |

## Pre-existing failures (NOT regressions — identical name-for-name base vs HEAD)

- mutation_writeback unit: 6 failed (atomic_store/mutex_set/writeback_hashset_*/writeback_emits_dup_storelocal) — base also 6, identical names.
- pop_mutation unit: 6 failed (pop_mutation_hashmap_remove_*/pop_mutation_rvalue_*) — base also 6, identical names.
- borrow_refs: 4 failed (unchanged).
- Residual reds in the integration groups above are the pre-existing shape-test-residuals collection-dispatch family (inference-loss / monomorphization / v2-raw-heap); none NEW.

## Honest residual

- R1-pushreassign genuinely needs the empty-array element-type-inference / let-generalization type-system design; the bytecode push-resolver the prompt cited never runs (inference rejects `.push` on `Array<TypeVar>` first). SURFACED, not forced. Module-scope self-push hit a separate module-binding storage/snapshot SIGSEGV → reverted rather than ship heap corruption.
- R3 SURFACED sub-case: object-element HOF closures (`users.filter(|u| u.score>85)`) not fixed — struct identity erased at array-of-structs binding; needs a distinct struct-array-recording fix.
- R5a downstream empty-NESTED typed-array carrier (`op_new_array(0)`) is the pre-SURFACED V3-S5 ckpt-6 root, distinct from R5a-literal.

## Verdict

5/6 roots FIXED-and-clean (R5a, R2, R3, R4, R5c), 1 SURFACED (R1 — needs type-system design). +19 integration tests cleared, 0 NEW regressions, all invariants preserved, all gates green. No unsound fix shipped (F7 discipline upheld).
