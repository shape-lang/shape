# E2 #18 slice-0 report — pre-analysis materialization spike + parity denominator

Committed evidence for the two slice-0 obligations (E2-D6 STOP decision, E2-D7
parity gate). Authored at branch `adr009/e2`, base `3a1aa469` (main `52fc13f8`
with C1+C2 merged). Executable pins live in
`crates/shape-vm/src/compiler/functions_annotations/e2_slice0_spike_tests.rs`
(supervisor runs them — this author does not).

---

## Part A (E2-D6) — verdict: FEASIBLE-WITH-EXISTING-MACHINERY

**The directive-produced replacement CAN run through the existing pre-analysis
window with the existing machinery. No analyzer-ordering change is required.**
Wiring point: the directive-materialization loop inside
`materialize_computed_comptime_extends`
(`crates/shape-vm/src/compiler/functions_annotations.rs:2200`) plus a
replacement-edit return channel that `compile_in_place_inner`
(`crates/shape-vm/src/compiler/compiler_impl_reference_model.rs:2132`) applies to
`analysis_program` before `analyze_program_full` (same file, `:2260`).

### What the pre-pass needs vs. what a comptime-post handler needs

The executed declaration-discovery pre-pass (`materialize_computed_comptime_extends`)
already runs BEFORE the analyzer and already executes `targets: [function]`
comptime-post handlers:

- `collect_declaration_discovery_targets`
  (`functions_annotations/declaration_discovery.rs:161-168`) collects every
  annotated non-comptime function as a `DeclarationDiscoveryTarget::Function`.
- The fixed-point loop executes each target's handler via
  `execute_comptime_with_annotation_handler` (`functions_annotations.rs:2121`)
  and iterates the resulting `execution.directives`.
- Its directive-materialization arm handles ONLY `ExtendItems` / `Extend`
  (`functions_annotations.rs:2200-2216`); every other directive — including
  `ReplaceBody` — falls through `_ => continue` and is discarded.

The inputs a handler reads to PRODUCE a `ReplaceBody` directive are all
pre-analysis-available:

- the target function's SIGNATURE — `ComptimeTarget::from_function(func_def)`,
  built straight from the AST (`declaration_discovery.rs:96-99`);
- the semantic-freeze reflection handle — installed by `install_semantic_freeze`
  at `compiler_impl_reference_model.rs:2118`, BEFORE the pre-pass at `:2132`, and
  consumed by the pre-pass at `functions_annotations.rs:2107`;
- comptime helpers — dependency-module functions are already in `function_defs`
  from graph phase 1; unregistered root helpers already fall back to pass-2.

The state the `ReplaceBody` directive APPLICATION reads
(`functions_annotations.rs:3049-3122`) is likewise pre-analysis-available: the
target `func_def` AST, the freeze handle (`build_original_capability`), the
effective pass modes (`effective_function_like_pass_modes`), and the generated
provenance stamp (`stamp_generated_replacement_body` — the same generated-origin
issuer the fresh-extend path uses via `stamp_generated_closure_provenance` at
`:2250`). None of it requires compiled function bodies or a materialized MIR.

### Why this is not a STOP

The E2-D6 STOP condition is "handler execution structurally requires pass-2
context, forcing an analyzer-ordering change." It does not hold:

1. Handler EXECUTION is already pre-analysis (the pre-pass runs it today).
   Proven by pin `t2`: a function-target `extend` handler's method is
   materialized into the executed authority `generated_analysis_items` — which
   ONLY the pre-pass populates, never pass-2.
2. Directive PRODUCTION and APPLICATION read only pre-analysis-available state
   (traced above).
3. The gap is purely WIRING: the pre-pass drops `ReplaceBody` at
   `_ => continue`. Pin `t1` shows a function-target `replace body` contributes
   nothing to the executed authority; pin `t3` shows the replacement still SHIPS
   at pass-2 (compile succeeds) while the executed authority stays empty — the
   exact C2 named finding (`checked_body/mod.rs:139-149`) at the compiler tier.

The analyzer ordering is unchanged: `analyze_program_full` still runs at
`:2260`; the fix only makes the analysis program it already builds (a clone that
already receives prepended generated `extend` items at `:2150-2158`) additionally
carry the replacement edit + its hygienic `ctx.original` shadow. The named E2-D6
machinery — the `InstallTransaction` bracketing `compile_in_place`
(`checked_body/mod.rs`) and the `GeneratedNodeOrigin` issuer — is already present
and already used on both the fresh-extend and replace-body sides.

### Scoping boundary surfaced (not a STOP; a later-slice note)

The pre-pass passes `&[]` for specialization const bindings
(`functions_annotations.rs:2128`), whereas the pass-2 function handler passes
`specialization_const_bindings` (`:1043-1047`). A `replace body` handler whose
emitted body DEPENDS on a per-call-site const specialization would therefore
materialize differently (or not at all) in the pre-pass. This does not block the
C0911 case (the quarantine fixture `edit_worker` has no const params) — and
const-template functions already skip comptime-handler execution until a
concrete specialization binds (`functions.rs:945-947`), so a single monomorph is
not in the analyzer's single-program view regardless. Flagged for the slice
that designs the replacement-edit return channel: scope pre-analysis
materialization to non-const-template `replace body` edits; const-template
specialization-dependent edits stay a pass-2 concern.

### Pins (supervisor runs; filter `cargo test -p shape-vm --lib e2_slice0_spike_tests`)

| Pin | Asserts (current reality) | Tripwire flip on E2 fix |
|---|---|---|
| `t1_function_target_replace_body_is_not_materialized_by_the_executed_prepass` | function-target `replace body` → `executed_generated_items` empty | becomes non-empty |
| `t2_function_target_extend_is_materialized_by_the_executed_prepass` | function-target `extend` → method present in executed authority (the reusable machinery) | stays green |
| `t3_replace_body_replacement_ships_at_pass2_but_is_analyzer_invisible` | replacement compiles+ships, yet executed authority empty (C0911 timing) | emptiness flips |

---

## Part B (E2-D7) — parity denominator

Every test/fixture/smoke/stdlib file exercising the legacy U03/U07 transport
today (the string/JSON mini-VM directive transport E2 deletes). Construct tags:
**RB** replace-body · **RM** replace-module · **ES** extend-string (`extend (f"…")`)
· **EI** extend-items (direct `extend target { method }` block) · **IF** `item_fn`
· **PB** probe (`__body_probe`/`__module_probe__`). The transport DEFINITION site
(the deletion target) is `crates/shape-vm/src/compiler/comptime_builtins.rs` —
the sole holder of `__body_probe`, `__module_probe__`,
`parse_function_body_payload`, `parse_module_items_payload`,
`parse_extend_items_slot`, `__ComptimeItemFragment`, `__emit_replace_body/module`,
`__emit_extend_items`, `item_fn`.

### Bucket 1 — shape-test integration (`tools/shape-test/tests/`)

| File (under `tools/shape-test/tests/`) | Tests / fixtures | Tag | Filter |
|---|---|---|---|
| `annotations_comptime/directives.rs` | `replace_module_from_source_string_replaces_items`, `replace_module_generated_source_type_errors_are_reported`, `replace_module_rejected_on_function_target`, `replace_module_payload_is_still_source_text`; `extend_item_fragment_generates_free_function_without_source_string` | RM; IF | `cargo test -p shape-test annotations_comptime::directives` |
| `annotations_comptime/code_gen.rs` | `annotation_replace_body_generates_constant_function` (RB); `annotation_generates_display_method`, `annotation_generates_getter_method`, `annotation_extends_type_with_equality_check`, `stacked_annotations_both_extend_type`, `annotation_generates_to_string_method`, `annotation_generated_extend_method_runs_under_jit`, `annotation_generates_predicate_method` (ES) | RB; ES | `cargo test -p shape-test annotations_comptime::code_gen` |
| `annotations_comptime/executed_extend_authority.rs` | `d7_direct_extend_target_method_materializes_via_executed_prepass` (EI); `d12_computed_snippet_extend_only_materializes_via_executed_prepass`, `d8_stacked_annotations_both_extend_via_executed_prepass` (ES); `s3_ctx_original_invokes_pre_annotation_body_typed_path`, `s3_user_original_spelling_does_not_collide_reaches_body_via_ctx_original`, `s3_former_original_spelling_does_not_resolve_the_shadow` (RB); `function_target_extend_explicit_type_materializes_via_executed_prepass`, `r6_*`, `u10_*`, `s4_*`, `false_guarded_*` (ES) | EI; ES; RB | `cargo test -p shape-test annotations_comptime::executed_extend_authority` |
| `annotations_comptime/on_define.rs` | `comptime_post_replace_body_overrides_function` (RB); `comptime_post_extend_adds_method_to_type`, `comptime_post_extend_adds_computed_method`, `comptime_post_extend_adds_multiple_methods`, `comptime_post_with_annotation_param_in_method` (ES) | RB; ES | `cargo test -p shape-test annotations_comptime::on_define` |
| `annotations_comptime/type_mutation.rs` | `replace_body_on_function_target` (RB); `extend_target_adds_derived_method`, `extend_target_adds_method_using_multiple_fields`, `extend_target_method_with_parameters`, `annotation_removes_and_replaces_type`, `annotation_extends_type_with_boolean_method`, `annotation_extends_type_with_string_method`, `annotation_with_param_used_in_generated_method` (ES) | RB; ES | `cargo test -p shape-test annotations_comptime::type_mutation` |
| `annotations_comptime/generated_capture.rs` | `generated_function_rejects_implicit_closure_capture`, `generated_function_rejects_parameter_capture`, `generated_capture_diagnostic_is_deterministically_sorted`, `generated_method_rejects_implicit_self_capture`, `generated_closure_parameters_are_not_captures`, `generated_method_allows_capture_free_closure_in_both_tiers`, `ordinary_source_closures_keep_implicit_capture_in_both_tiers` | ES | `cargo test -p shape-test annotations_comptime::generated_capture` |
| `annotations_comptime/generated_capture/gate_totality.rs` | `replace_body_expansion_rejects_implicit_capture`, `ctx_original_shadow_body_keeps_implicit_capture` (RB); `generated_function_allows_capture_free_closure`, `generated_nested_closure_rejects_implicit_capture`, `generated_generic_body_rejects_implicit_capture_through_monomorphization`, `annotation_hook_impl_body_keeps_implicit_capture` (ES) | RB; ES | `cargo test -p shape-test gate_totality` |
| `annotations_comptime/generated_method_runtime.rs` | `generated_snippet_extend_method_behaves_identically_in_vm_and_jit`, `generated_extend_target_method_behaves_identically_in_vm_and_jit`, `generated_extend_target_arithmetic_method_behaves_identically_in_vm_and_jit`, `generated_free_function_behaves_identically_in_vm_and_jit` | ES | `cargo test -p shape-test generated_method_runtime` |
| `annotations_comptime/frozen_reflection.rs` | `annotation_handler_reflection_reaches_generated_fn_called_from_fn_body`, `..._called_top_level`, `function_target_annotation_handler_asserts_frozen_category`, `annotation_handler_composite_type_expression_reaches_generated_fn`, `function_target_annotation_handler_error_branch_fires_on_wrong_category`, `annotation_handler_reflect_payload_reaches_generated_fn`, `annotation_handler_reflect_composite_payload_reaches_generated_fn`, `annotation_handler_reflect_r1_rejection_fires_in_hooks` | ES | `cargo test -p shape-test frozen_reflection` |
| `comptime/flagship_wf3d.rs` | `wf3d_f1_generated_free_fn_vm`, `wf3d_f1_generated_free_fn_jit`, `wf3d_f4_method_emission_dispatch_vm`, `wf3d_f4_method_emission_dispatch_jit` | ES | `cargo test -p shape-test flagship_wf3d` |
| `comptime/nominal.rs` | `reflect_repr_with_authority_exposes_complete_shape_on_vm_and_jit`, `ordinary_reflect_is_not_a_filtered_representation_on_vm_and_jit` | ES | `cargo test -p shape-test comptime::nominal` |
| `comptime/annotations.rs` | `ct_46_annotation_replace_body` (RB); `ct_41_extend_target` (EI/ES) | RB; ES | `cargo test -p shape-test comptime::annotations` |
| `comptime/reflect.rs` | reflect-focused; program builders use `extend (f"…")` incidentally | ES | `cargo test -p shape-test comptime::reflect` |

### Bucket 2 — crate unit tests (`#[cfg(test)]`)

| File | Tests / note | Tag | Filter |
|---|---|---|---|
| `crates/shape-vm/src/compiler/comptime_builtins.rs` | TRANSPORT DEFINITION (deletion target); inline probe tests | PB/RB/RM/ES/IF | `cargo test -p shape-vm --lib compiler::comptime_builtins` |
| `crates/shape-vm/src/compiler/comptime_builtins/expansion_provenance.rs` | expansion identity/provenance module | RB | `cargo test -p shape-vm --lib comptime_builtins::expansion_provenance` |
| `crates/shape-vm/src/compiler/functions_annotations/c2_slice4_edit_tests.rs` | `replace_body_edit_drop_plus_await_in_replacement_rejects_c0922`, `replace_body_edit_drop_plus_await_only_in_pre_edit_body_installs`, `replace_body_edit_suspension_without_drop_in_replacement_installs`, `failed_replace_body_edit_leaves_no_half_edited_hybrid`, `successful_replace_body_edit_supersedes_pre_edit_body_cleanly`, `replace_body_edit_capture_set_and_body_commit_together`, `replace_body_edit_capture_set_and_body_roll_back_together` | RB | `cargo test -p shape-vm --lib c2_slice4_edit_tests` |
| `crates/shape-vm/src/compiler/functions_annotations/c2_slice2_battery_tests.rs` | `battery_row1/3/4/5/6/7/9_and_10a_*`, `battery_row10b_d6_*` (D6 matrix), `battery_row10b_generic_monomorphization_uncovered_installs` | ES | `cargo test -p shape-vm --lib c2_slice2_battery_tests` |
| `crates/shape-vm/src/compiler/functions_annotations/c2_slice0_preflight_tests.rs` | install-atomicity pins | ES | `cargo test -p shape-vm --lib c2_slice0_preflight_tests` |
| `crates/shape-vm/src/compiler/functions_annotations.rs` (`e3_function_target_discovery_tests`) | `function_target_extend_explicit_type_enters_discovery`, `function_target_extend_user_type_enters_discovery`, `unapplied_function_target_annotation_generates_nothing` | EI/ES | `cargo test -p shape-vm --lib e3_function_target_discovery_tests` |
| `crates/shape-vm/src/compiler/functions_annotations/original_body_shadow_tests.rs` | `remove_target_discards_a_staged_original_body_shadow`, `repeated_replace_body_is_rejected_before_shadow_publication`, `replacement_mir_uses_only_its_own_distinct_closure_identity`, `failed_shadow_emission_restores_body_analysis_authority`, `pending_shadow_rejects_misaligned_reference_provenance`, `authentic_capability_*`, `staged_emission_*` | RB | `cargo test -p shape-vm --lib original_body_shadow_tests` |
| `crates/shape-vm/src/compiler/functions_annotations/generated_closure_provenance.rs` | provenance module | RB | `cargo test -p shape-vm --lib generated_closure_provenance` |
| `crates/shape-vm/src/compiler/functions/reference_provenance_tests.rs` | `replace_body_ctx_original_preserves_inferred_reference_provenance`, `single_runtime_annotation_*`, `chained_runtime_annotations_*` | RB | `cargo test -p shape-vm --lib reference_provenance_tests` |
| `crates/shape-vm/src/compiler/original_body_rewrite/generated_origin_tests.rs` | `stamp_survives_the_ctx_original_rewrite_including_nested_closures` | RB | `cargo test -p shape-vm --lib original_body_rewrite::generated_origin_tests` |
| `crates/shape-vm/src/compiler/checked_body/battery.rs` | inline battery tests | RB | `cargo test -p shape-vm --lib checked_body::battery` |
| `crates/shape-vm/src/compiler/checked_body/edit_transaction_guards.rs` | `c0924_message_is_well_formed_and_marker_free`, `c0925_message_is_well_formed_and_marker_free` | RB | `cargo test -p shape-vm --lib edit_transaction_guards` |
| `crates/shape-vm/src/compiler/checked_body/async_drop_context.rs` | inline async-drop-context tests | RB | `cargo test -p shape-vm --lib checked_body::async_drop_context` |
| `crates/shape-vm/src/compiler/comptime_builtins/capture_plan/declared_tests.rs` | `flagship_declared_move_over_read_only_let_mut_emits_owned_mutable`, `declared_share_over_local_var_emits_shared`, `declared_mode_survives_monomorphization`, + siblings | ES | `cargo test -p shape-vm --lib capture_plan::declared_tests` |
| `crates/shape-vm/src/compiler/statements/annotation_import_pipeline_tests.rs` | `local_annotation_shadows_two_imports_through_the_full_pipeline`, + siblings | ES | `cargo test -p shape-vm --lib annotation_import_pipeline_tests` |
| `crates/shape-vm/src/compiler/statements/annotation_declarations/tests/phase.rs` | `direct_and_exported_forward_declarations_share_one_phase`, `transformed_nested_module_prepares_final_effective_items`, + siblings | RM | `cargo test -p shape-vm --lib annotation_declarations::tests::phase` |
| `crates/shape-ast/src/parser/tests/advanced.rs` | `test_annotation_typed_comptime_directives_parse` (RB), `test_annotation_replace_body_expr_directive_parse` (RB, `ReplaceBodyExpr`), `test_annotation_replace_module_expr_directive_parse` (RM, `ReplaceModuleExpr`), `test_annotation_comptime_directives_parse_in_block` | RB; RM | `cargo test -p shape-ast --lib parser::tests::advanced` |

Whole-crate fallbacks: `cargo test -p shape-vm --lib`, `cargo test -p shape-ast --lib`.

### Bucket 3 — smoke fixtures + CLI native harness

| Fixture | CLI test (`bin/shape-cli/tests/cli/jit_c2_install_native.rs`) | Tag |
|---|---|---|
| `tests/smokes-jit-closure/c2-replace-body-edit.shape` (`replace body { return 42 }`) | `c2_replace_body_edit_runs_natively_both_tiers` | RB |
| `tests/smokes-jit-closure/c2-async-clean-generated-method.shape` | `c2_async_clean_generated_method_installs_and_runs_named_fallback` | ES |
| `tests/smokes-fallback/c1-generated-extend-capture-free.shape` | `c2_generated_move_capture_still_native_post_c2` | ES |

Filter: `cargo test -p shape-cli jit_c2_install_native` (or `... c2_replace_body_edit`).

### Bucket 4 — stdlib consumers (the slice-4 Q1-A migration set)

All three use **ES** (`extend (f"…")`); NONE use replace-body/replace-module/`item_fn`.
Migration-relevant construct per file:

| File (`crates/shape-runtime/stdlib-src/`) | Annotation | Construct emitted | Line |
|---|---|---|---|
| `serde/derive.shape` | `@json_schema` comptime post | ES → FREE FUNCTION (`{Type}_json_schema() -> string`) | :95 |
| `serde/serialize.shape` | `@to_json` comptime post | ES → EXTEND METHOD (`extend {Type} { method to_json() -> string }`) | :55 |
| `llm/tools.shape` | `@tool_def` comptime post | ES → FREE FUNCTION (`{Type}_tool_def() -> string`) | :67 |

(`llm/tools.shape` `@prompt` at :75 is validation-only — no emit; not a transport
exerciser.) stdlib .shape source lives at `crates/shape-runtime/stdlib-src/`,
NOT `src/stdlib/`.

### Bucket 5 — book / example corpus

No `book/` dir in this worktree (book acceptance lives under `docs/`). Executable
book-derived corpus fixtures using the transport:

| Fixture | Tag |
|---|---|
| `tools/vmjit-diff/corpus/E__advanced__content-addressed-bytecode__11__L347.shape` | RB |
| `tools/vmjit-diff/corpus/E__advanced__content-addressed-bytecode__12__L371.shape` | RB |

The `D__advanced__comptime-annotations-cookbook__*.shape` corpus files do NOT use
the transport constructs. Per E2-D7 the K3 book snippet is included in the gate
iff currently green — to be confirmed by the supervisor's run of the two RB
corpus fixtures above.

### Bucket 6 — D6 / D10 matrix fixtures

- **D6** rows: `crates/shape-vm/src/compiler/functions_annotations/c2_slice2_battery_tests.rs`,
  helper `d6_program` (ES), consumed by `battery_row10b_d6_drop_obligated_across_suspension_rejects_atomically`,
  `battery_row10b_d6_inferred_method_call_drop_local_across_suspension_rejects_atomically`,
  `battery_row10b_d6_headline_case_live_across_await_rejects_with_c0922`,
  `battery_row10b_d6_control_suspension_without_drop_local_installs`,
  `battery_row10b_d6_control_drop_local_without_suspension_installs`.
  Filter: `cargo test -p shape-vm --lib battery_row10b_d6`.
- **D7/D8/D12** rows: `tools/shape-test/tests/annotations_comptime/executed_extend_authority.rs`
  (`d7_*` EI, `d8_*`/`d12_*` ES) — see Bucket 1.
- **D10**: no test carries an explicit `D10` token tied to a comptime directive;
  any D10 obligation is subsumed by `battery_row9_and_10a_*` / `battery_row10b_*`
  in the D6 file above. (Flagged for supervisor confirmation against the phase-1
  D6/D10 matrix.)

### Consolidated candidate green-set filters

```
cargo test -p shape-test annotations_comptime
cargo test -p shape-test comptime
cargo test -p shape-vm --lib comptime_builtins
cargo test -p shape-vm --lib c2_slice
cargo test -p shape-vm --lib original_body_shadow
cargo test -p shape-vm --lib checked_body
cargo test -p shape-vm --lib e3_function_target_discovery_tests
cargo test -p shape-ast --lib parser::tests::advanced
cargo test -p shape-cli jit_c2_install_native
```

Caveat (unresolved read-only): the exact `--test <binary>` name for shape-test /
shape-cli integration harnesses was not enumerated; the `-p <crate> <substring>`
name filters above still select the right functions.
