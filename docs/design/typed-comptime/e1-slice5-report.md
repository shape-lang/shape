# E1 #17 slice-5 report — U02 expr-form type_refs off `.source` reparse onto FrozenTypeIdentity stamping

ADR-009 E1-D2.5 + **E1-D7** (A-FULL in one slice). The annotation-handler
expr-form comptime type refs — `set return (target.params[N].type_ref)` /
`set param x: (…type_ref)` — used to resolve by REPARSING the
`__ComptimeTypeRef.source` string. Slice 5 moves them onto a `FrozenTypeIdentity`
stamped at the producer and reconstructed at the consumer, **including composite
types**, leaving the reparse arm dead-but-present for stamped refs (slice 6
deletes). Branch `adr009/e1`, on top of closed slice-4 (`e7d0cb71`).

Stages (append-only, per-commit-green):
`389c1940` stage 1 (STOP-shaped boundary pins) →
`f8e772be` stage 2 (schema + total reconstruction fn, INVALID stamps) →
`a58aae7d` stage 3 (producer stamps, one shared overlay Arc) →
`f6b1e327` stage 4 (consumer flip — the behavior flip) →
this stage 5 (route-proof pins + composite e2e + docs, test-only).

## The ruled A-FULL composite scope

E1-D7 charters A-FULL *including composites*, but "composites" is bounded by
what the semantic-freeze descriptor algebra can invert **without a second name
table or a second hasher**. The frontier:

- **Reconstructable (stamped → identity route):** primitive leaves (`int`,
  `string`, `bool`, `number`, …), `Never`, base `any`, and the structural
  composites `Tuple`, `Reference` (`&T`/`&mut T`), `Union`, and `Callable`
  (when it round-trips). Each recurses the `FrozenPayloadDescriptor` algebra
  totally.
- **NOT reconstructable (stamp INVALID → `.source` reparse arm, named error in
  the total fn):** applied-generic nominals (`Array<int>`, `Option<T>`,
  `HashMap<K,V>`), structural `Record`s (field names are one-way-hashed into
  hygienic member identities), bare user-nominals, and un-applied generic heads.
  `Array<int>` reconstruction is **B4/B5, out of E1** — attempting it would
  force either a false failure or the forbidden silent stamped→reparse fallback.

This is the reconciliation of "A-FULL incl. composites" with the proven
`Array<int>` boundary: A-FULL covers *every composite the freeze can invert*,
and the non-invertible remainder stays on the byte-unchanged reparse arm as
E1-D7(a) unstamped fall-through — not a scope cut, a stamp-gated residual.

## The proof chain locking that boundary (stage-1 pins)

Two plain `#[cfg(test)]` pins in `comptime_builtins.rs::e1_s5_boundary` fix the
frontier in code:

- `e1_s5_applied_nominal_is_pending_rejection_not_reconstructable` — `Array<int>`
  canonicalizes CLEANLY to a Nominal identity, but `overlay.payload_of(identity)`
  returns the NAMED `payloads::applied_nominal_pending_rejection()`
  (`substituted_applied_nominal` is `None` for a builtin generic head). The
  boundary is at *payload issuance*, not canonicalization.
- `e1_s5_tuple_int_string_reconstructs_via_descriptor` — the positive frontier:
  `[int, string]` `payload_of` returns a `Tuple` descriptor whose two element
  identities each themselves `payload_of` to `Primitive`.

## The stamp-gate: one predicate, no parallel logic (E1-D7(a)+(b))

The producer stamps an identity **iff** `reconstruct_type_annotation(overlay,
id).is_ok()` — the SAME function the consumer resolves with
(`comptime_target::stamp_for`, `comptime_target.rs:156`). There is no separate
"is this reconstructable" classifier: the gate literally calls the consumer's
inverse and stamps on `Ok`. Consequences:

- A stamped identity is, by construction, one the consumer can reconstruct → the
  consumer's identity-only branch never has to fall back.
- A stamped-but-unresolvable identity is therefore *impossible from the
  producer*; if one is ever presented (a forged/never-frozen identity) the
  consumer returns a NAMED `ShapeError::SemanticError`, NEVER a silent `.source`
  reparse (E1-D7(a)). Route-proof pin (c) is the sentinel for this.
- Identity is computed ONLY via `FreezeOverlay::canonicalize_type_projection`
  (`projection.rs:55`) — the one canonicalizer, which also interns the composite
  payload. No second hasher/canonicalizer/name-derivation exists anywhere
  (E1-D7(b)).

Reconstruction (`reconstruct_type_annotation`, `comptime_builtins.rs:524`)
inverts the ONE `PRIMITIVE_SYNONYM_FAMILIES` table (via
`type_reflection::canonical_primitive_spelling`, `names[0]` canonical) and
matches ALL 10 `FrozenPayloadDescriptor` variants — each reconstructs OR returns
a named `SemanticError`; no catch-all silent arm (E1-D7(c)).

## Shared-overlay plumbing, and WHY

Primitive-leaf identities are **base-resident** in the frozen index, so the
consumer's `payload_of` answers them from any overlay — the corpus (leaf
`string`) is immune to which overlay is used. **Composite** payloads are NOT
base-resident: `canonicalize_type_projection` interns the `Tuple`/`Union`/etc.
descriptor into the *per-`Arc` `FreezeOverlay.composites` memo*. So a composite
identity minted on overlay A cannot be `payload_of`-resolved on a different
overlay B — its evidence lives only in A's memo.

Therefore the identity-minting overlay (producer stamp time) MUST be the SAME
`Arc<FreezeOverlay>` handed to the consumer builtins. Stage 3 reorders producer
sites 1–4 to acquire ONE overlay before `to_nanboxed` and threads that Arc into
`execute_comptime_with_annotation_handler` (`comptime.rs`), which hands it
straight to `comptime_builtins_module_base`; stage 4 makes the consumer read
composites off that same Arc. The unit route-proof pin (b) and the Tuple e2e
canary this: a broken shared-overlay would make the composite `payload_of` fail
→ named error → loud compile failure, while the leaf corpus stays green.

## Scout overrides (carried from the probe, re-confirmed)

1. **`declaration_discovery` needs NO signature change.** Under store-AST-in-
   `ComptimeTarget` (R2+H1: index-parallel `param_type_asts`/`field_type_asts`/
   `return_type_ast`), identity is computed inside `to_nanboxed` at the
   compiler-scoped driver where the overlay is already reachable — including the
   pre-pass. The constructors (`from_function`/`from_type`/`comptime_target`)
   take no overlay param and the pre-pass gap the probe flagged (finding 6)
   dissolves. D1 ExpansionIdentity is unperturbed: the string tuples
   params/fields/return are byte-identical, so
   `comptime_target_dependency_descriptors` reads the same descriptor string.
2. **Use the `pub(crate)` `canonicalize_type_projection` method**, NOT the
   `pub(super)` free fn `canonicalize_type_annotation`. The method ALSO interns
   the composite payload (required for the round-trip), so no visibility
   widening of the free fn is needed.
3. **Record → named error, stamp-gated to reparse.** The tests scout listed
   Record as reconstructable; the consumer-freeze scout showed record field
   names are one-way-hashed. Reconciled in favor of the freeze: Record is a
   distinct named rejection, so it never stamps (falls to reparse).

## E1-D7 compliance summary

| Rule | Discharge |
|---|---|
| (a) STAMPED→IDENTITY-ONLY, no silent fallback | Consumer branch: `identity != INVALID` ⇒ `reconstruct_type_annotation(...).map_err(to_string)` — never `.source`. Sentinel pin (c). |
| (b) ONE identity computation | Stamp-gate reuses `reconstruct_type_annotation`; identity only via `canonicalize_type_projection`. No second hasher. |
| (c) TOTAL reconstruction off ONE table | `canonical_primitive_spelling` inverts `PRIMITIVE_SYNONYM_FAMILIES`; all 10 descriptor variants handled or named error. Totality pin. |

## Stage-5 tests (supervisor runs)

Unit route-proof pins (`cargo test -p shape-vm --lib e1_s5_`,
`comptime_builtins.rs::e1_s5_route_proof`, plain `#[cfg(test)]` so the standard
gate runs them):

- `e1_s5_leaf_identity_route_resolves_past_garbage_source` — leaf `string`
  stamped onto an UNPARSEABLE source resolves via identity to `Basic("string")`.
- `e1_s5_composite_identity_route_resolves_past_garbage_source` — `[int, string]`
  stamped onto a garbage source resolves via the composite route (SAME overlay)
  to the byte-identical Tuple. Unit-tier shared-overlay proof.
- `e1_s5_stamped_unresolvable_identity_is_named_semantic_error_no_fallback` —
  a fabricated never-frozen identity is a NAMED `SemanticError`, not `Ok`, not a
  reparse. The anti-walk-back sentinel.
- `e1_s5_unstamped_typeref_falls_through_to_source_arm_bytewise` — INVALID +
  valid source reparses to `Basic("string")`; INVALID + garbage source still
  `Err`s (the reparse arm is genuinely reached, not shadowed).

Composite e2e (`cargo test -p shape-test --test annotations_comptime
type_mutation`): `set_return_accepts_composite_type_ref_expression` and
`set_param_type_accepts_composite_type_ref_expression` — inline Tuple type_refs
through the real handler path (return + param producer sites). The regression
guard `target_params_and_return_expose_type_refs` (reads `.type_ref.kind` /
`.return_type_ref.source`) stays green: A-FULL only APPENDED identity fields;
`name`/`kind`/`source` are byte-unchanged.

Retained from earlier stages: 2 boundary pins + 3 reconstruction pins (all in the
`e1_s5_` prefix → 9 green total).

## Recorded-baseline differential (all UNMOVED)

Test+docs only this stage; production code byte-unchanged since stage 4. Measured
at `--test-threads=1` (the suites have documented parallel-state contention;
default parallelism is non-deterministic noise — a default run of
annotations_comptime ranged 39–55 FAILED across runs, contradictory in both
directions, and is NOT a valid gate):

| Baseline | Recorded | Measured @ `-j1` with stage-5 changes | Verdict |
|---|---|---|---|
| st-annotations | 10-name | exactly 10 (all `executed_extend_authority::*` + `generated_method_runtime::*`) | UNMOVED |
| vmlib (shape-vm lib) | 7-name + `nested_exact` flapper | exactly 8 (7 + `nested_exact_calls_close_outer_arguments_before_inner_compilation`) | UNMOVED |
| st-comptime | 3-name | exactly 3 (`b6_annotation_*` ×2 + `hash_tracer_does_not_disturb_formatted_strings`) | UNMOVED |

None of the new pins/e2e appear in any FAILED set; the 16-case `type_mutation`
target is fully green. `just check-clean` exit 0; `cargo check -p shape-vm
--all-targets` success.

## Findings disclosed (within binding rules)

- **Heterogeneous tuple VALUES are homogeneous-only.** The TYPE `[int, string]`
  canonicalizes to a Tuple identity and routes the composite identity path
  cleanly (proven by pin (b) and by the stage-4 first-cut e2e, whose set-return
  produced an `((int, string)) -> (int, string)` signature). But constructing a
  heterogeneous tuple *value* is a hard semantic error ("bracket types
  `[T, T, …]` are homogeneous-only"). The e2e therefore uses a **homogeneous
  `[int, int]`** tuple — treated identically by the freeze canonicalizer
  (`FrozenTypeCategory::Tuple` regardless of element homogeneity,
  `type_reflection.rs:998`), so it exercises the same composite identity route,
  but is a constructible, indexable runtime value. This is a test-fixture choice,
  not a scope change; the rigorous garbage-source composite proof is unit pin (b)
  with heterogeneous `[int, string]`.
- **Call-site bracket-literal inference.** A bare `[7, 9]` at a call site infers
  as a homogeneous array, not a tuple, unless an annotation drives it; the e2e
  binds the argument through an annotated `let a: [int, int] = …` first. Unrelated
  to E1 — a general tuple-literal inference property.

## Forbidden-patterns check

No silent stamped→reparse fallback (E1-D7(a) upheld — the sentinel pin fails the
build if it regresses). No second hasher/canonicalizer/name-table (E1-D7(b)/(c)).
Identity halves read via the existing sanctioned `get_field` →
`clone_field_kinded` → `as_i64` shape (the same read `frozen_identity_from_ref`
already uses for the sibling schema), not a new decode path. No JSON at the typed
boundary, no `ValueWord` shapes, no bridge/probe/helper/shim/adapter rename.
`resolve_param_id`/C0930, the slice-3 Int64 literal carrier, and the slice-4
extend carrier are untouched; the `.source` reparse arm,
`parse_type_annotation_payload`, and `__type_probe` are BYTE-UNCHANGED (dead for
stamped refs, live for unstamped) — see the slice-6 orphan inventory.
