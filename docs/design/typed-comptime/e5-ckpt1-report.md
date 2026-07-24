# E5 CKPT-1 report — Piece 2 SPELLING reconstruction (additive; deletes nothing)

Base `9028d2be` (branch `adr009/e5`). Program of record: `e5-decisions.md`.
Design of record: `/tmp/.../scratchpad/e5/e5-design.md` §1a. CKPT-1 makes applied
generics + bare user nominals RECONSTRUCT (spell) so the shared stamp-gate
AUTO-WIDENS and they stop hitting the `.source` reparse arm — the additive
foundation for the CKPT-5 deletion. **CKPT-1 DELETES NOTHING.**

## What landed

Three production files + one e2e file + the durable docs.

1. **`comptime_builtins/semantic_freeze.rs` — two sibling accessors (ruling A1).**
   - `FreezeOverlay::applied_nominal_of(identity) -> Option<RefinedApplication>`
     — reads the already-derived `composites` memo `applied_nominal` field. `Some`
     iff `identity` is an `applied:h<…>` form (applied builtins + applied user
     structs/enums). A READ of derived facts; NOT a new derivation; NEVER a
     reparse. Sibling accessor, not a new sealed `FrozenPayloadDescriptor` variant.
   - `FreezeOverlay::bare_nominal_name_of(identity) -> Option<String>` — reads the
     base `frozen_nominal_descriptors`. `Some(name)` iff `identity` is a RESOLVED
     user nominal (struct/enum with a frozen descriptor). `None` for an un-applied
     generic head (declared param kinds, no descriptor — ruling A3), a primitive,
     or any composite.

2. **`comptime_builtins.rs` — one new early arm in `reconstruct_type_annotation`**
   (BEFORE the `payload_of`-driven arms). An applied nominal spells its head
   (`type_names_for_identity(head).first()`) then RECURSES on `arg_identities`
   into `TypeAnnotation::Generic{name, args}`; a bare user nominal spells
   `Basic(name)`. Else falls through to the existing arms UNCHANGED. The A2
   identity-indirected-recursion invariant is bound in a code comment.

3. **`comptime_builtins/semantic_freeze/projection.rs` — recursive sub-expression
   memoization.** `canonicalize_type_projection` now memoizes the composite
   payload evidence for the top identity AND every composite SUB-expression
   (`intern_composite_memo` + `memoize_composite_subtree`/`_children`), so a
   NESTED applied identity (the inner `Option<int>` of `Array<Option<int>>`)
   answers off the SAME shared `Arc<FreezeOverlay>` memo. Errors swallowed (a
   non-freezable sub-expression leaves no entry); terminates on the finite type
   AST. This is what makes the A2 nesting terminate with real answers, not a hang.

4. **`tools/shape-test/tests/comptime/e5_spelling.rs`** (+ `main.rs` registration)
   — the e2e round-trip suite (7 tests, VM + JIT).

Net: +453 / −56 across the 3 production/test files (comments + tests dominate);
the new file is additive. **No deletion.**

## Executed spelling proof (unit tier — `e1_s5_reconstruction`)

`e1_s5_reconstruct_applied_builtin_generics_spell_head_and_args` — GREEN:

| input | reconstructs to |
|---|---|
| `Array<int>` (built as the `Array(_)` sugar) | `Generic{ Array, [Basic("int")] }` |
| `Option<int>` | `Generic{ Option, [Basic("int")] }` |
| `HashMap<string, int>` | `Generic{ HashMap, [Basic("string"), Basic("int")] }` |
| `Result<int, string>` | `Generic{ Result, [Basic("int"), Basic("string")] }` |

`e1_s5_reconstruct_nested_applied_generic_terminates_and_spells` — GREEN:
`Array<Option<int>>` → `Generic{ Array, [ Generic{ Option, [Basic("int")] } ] }`
(identity-indirected recursion terminates; inner identity resolved off the shared
memo).

`e1_s5_reconstruct_bare_user_nominal_spells_as_basic` — GREEN: bare `User`
(a resolved `type User { id: int }`) → `Basic("User")`.

## Stamp-gate AUTO-WIDEN proof (no `stamp_for` edit)

`stamp_for` (comptime_target.rs) admits an identity iff
`reconstruct_type_annotation(...).is_ok()` — the ONE predicate shared by producer
and consumer (E1-D7(b)). The moment the new arm reconstructs an applied generic,
the SAME predicate stamps it. Proven two ways, both GREEN:

- `e1_s5_stamp_gate_predicate_auto_widens_for_applied_generics` — asserts
  `reconstruct(canonicalize(form)).is_ok()` for `Array<int>`, `Option<int>`,
  `HashMap<string,int>`, `Result<int,string>`, AND `Array<Option<int>>`. (Pre-CKPT-1
  each was `Err` → `INVALID` stamp → `.source` reparse.)
- `e1_s5_applied_generic_identity_route_resolves_past_garbage_source` (route
  tier) — a `__ComptimeTypeRef` stamped with `Array<Option<int>>`'s identity plus
  an UNPARSEABLE `.source` ("###unparseable###") resolves through the FULL
  consumer (`type_annotation_from_string_or_type_ref_slot`) to the nested
  `Generic{Array,[Generic{Option,[int]}]}`. A green result can ONLY have come from
  the identity route (a reparse of the garbage source would error) — proving
  (i) the stamp-gate auto-widen, (ii) the identity route fired, (iii) the nested
  identity resolves off the shared memo.

## e2e round-trip (VM + JIT — `e5_spelling.rs`, 7/7 GREEN)

`Array<int>` / `Option<int>` / `HashMap<string,int>` / `Result<int,string>` /
`Array<Option<int>>` (nesting termination canary) / bare `User` / applied user
generic `Box<int>` — each `type_ref(...)` consumed via the exhaustive
`type_category` match resolves to `Nominal` on BOTH engines through the REAL
compiler path. (Stamp-vs-reparse is observationally equivalent at the Shape level
for a VALID `.source`; the definitive stamp witness is the unit-tier
garbage-source route pin above.)

## Pin flips

- **`e1_s5_reconstruct_covers_frozen_payload_descriptor_totally`** (totality pin)
  — case (1) `Array<int>` FLIPPED from the applied-nominal-pending `Err` to
  `Ok(Generic{Array,[int]})`; cases (2) record + (3) bare head STAY named
  rejections. GREEN.
- **`e1_s5_applied_nominal_is_pending_rejection_not_reconstructable`** — COMMENT
  reworded (it tests `payload_of` DESCRIPTOR substitution, which STILL PENDS —
  that is CKPT-2 — not `reconstruct` SPELLING which landed in CKPT-1). Assertion
  UNCHANGED (still asserts the `applied_nominal_pending_rejection`). GREEN.
- **`e1_s5_leaf/composite_identity_route_resolves_past_garbage_source`** — the
  `.source` reparse arm they exercise is UNTOUCHED; both stay GREEN.

## Gate table (judged by FAILED-NAME SETS vs the Step-0 baseline)

| gate | baseline @ 9028d2be | CKPT-1 | verdict |
|---|---|---|---|
| `cargo test -p shape-vm --lib` | 6 stable failures + `nested_exact_calls…` FLAP | same 6 + same flap | **FAILED-name set IDENTICAL** |
| e1_s5 (`-- e1_s5`) | 10 pass / 0 fail | **15 pass / 0 fail** (10 + 5 new) | green, +5 |
| no_json (`-- no_json`) | 2 pass / 0 fail | 2 pass / 0 fail | green |
| `cargo test -p shape-test --test comptime` | 260 pass / 3 fail | 267 pass / 3 fail (+7 new) | **FAILED-name set IDENTICAL** |
| `just check-clean` | exit 0 (1 pre-existing `super::*` warn) | exit 0 (same pre-existing warn) | green |
| `just check-no-dynamic` | exit 0 | exit 0 | green, no dynamic path added |

The stable shape-vm-lib FAILED set (unchanged): `test_async_let_binding_is_immutable`,
`test_match_arm_empty_array_unprovable_element_is_clean_compile_error`,
`route_tests::inlined_closure_keeps_outer_authored_type_ref_in_its_parameter_scope`,
`route_tests::unavailable_and_missing_callsite_evidence_execute_only_in_legacy_domain`,
`ws6_generic_id_ok_arg`, `ws6b_inferred_result_variable_arg`.
The comptime FAILED set (unchanged, pre-existing JIT/callable):
`annotations::b6_annotation_iterates_callable_parameters_on_vm_and_jit`,
`annotations::b6_annotation_reads_callable_param_modes_on_vm_and_jit`,
`callable::hash_tracer_does_not_disturb_formatted_strings`.

**The FLAP (`route_tests::nested_exact_calls_close_outer_arguments_before_inner_compilation`)**
was verified PRE-EXISTING via `git stash` → clean baseline: it fails under
full-suite parallelism nondeterministically on `9028d2be` itself (run1=7 fail,
runs 2–3=6 fail; PASSES in isolation) — the `project_bulk_test_hang`
resource-accumulation family, NOT a CKPT-1 regression.

## `.source` untouched (anti-walk-back)

`git diff` over the reparse machinery: ZERO code changes to the `.source` schema
field, the `.source` reparse arm, `parse_type_annotation_payload`, `__type_probe`,
or `stamp_for`. Every `.source`/`reparse` token in the diff is a reworded COMMENT
or error-message string, never the code path. `just check-no-dynamic` stays exit 0
— CKPT-1 adds NO dynamic/reparse path; the new arm reads frozen identities and
spells from them.

## Verdict

**CKPT-1 FINISHED.** Applied generics + bare user nominals now reconstruct
(spell) and, via the auto-widened shared stamp-gate, STAMP — stopping the reparse
for those forms. `.source` and the reparse machinery are byte-untouched; the
deletion is CKPT-5. Next: CKPT-2 (Piece 1 descriptor substitution, B1 in-scope).
