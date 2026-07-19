# E1 #17 slice-5 PROBE report — Shape A feasibility (E1-D5, read-only)

Read-only probe per E1-D5, BEFORE any implementation. Question: is the
`FreezeOverlay` reachable at `comptime_target` build time (where
`build_type_ref_descriptor` constructs the `__ComptimeTypeRef`), so the producer
can stamp a `FrozenTypeIdentity` and the consumer resolve it off reparse?

## VERDICT: Shape A is FEASIBLE (overlay reachable) — but materially bigger than the spec assumed; scope ruling requested before implementing

The overlay is NOT the blocker, so this is not the E1-D5 "Shape B forced (overlay
unreachable)" case. But three findings diverge from the Shape A spec's mental
model and materially change the footprint — reporting before I write code, per
the report-before-implementing rule.

## Findings (anchors, re-verified)

1. **Overlay reachable at the `from_function` production sites.** `from_function`
   is called at `functions_annotations.rs:401` and `:1147`; both enclosing
   methods hold `self` and call `self.comptime_freeze_overlay()?` (`:420`, `:1266`
   in the same methods). `from_type` prod site `statements.rs:4124` likewise has
   `self.comptime_freeze_overlay()?` at `:4145`. So the overlay can be threaded
   into the target builders at these sites.

2. **AST is available at target-build time.** `from_function`
   (`comptime_target.rs:241`) reads `p.type_annotation` (a real `TypeAnnotation`)
   and RENDERS it to a string via `type_annotation_to_string`, discarding the AST;
   `from_type` does the same with `field.type_annotation`. So the AST needed for
   canonicalizing composites IS present — currently thrown away.

3. **LOAD-BEARING DIVERGENCE — `__ComptimeTypeRef` does NOT carry
   `identity_high/low`.** `__ComptimeTypeRef` (`builtin_schemas.rs:408`) declares
   ONLY `{name, kind, source}`. The `identity_high/low` fields the spec cited at
   `:423` belong to a DIFFERENT schema, `COMPTIME_FROZEN_TYPE_REF_SCHEMA`, which
   is the `type_ref(T)` intrinsic's OPAQUE frozen ref (consumed at
   `comptime_builtins.rs:1651+`, `type_reflection.rs:1900`) — not the
   annotation-handler `target.params[].type_ref` the corpus uses. So Shape A must
   ADD `identity_high/low` to `__ComptimeTypeRef`; the "already declares" premise
   is about the other schema.

4. **`ComptimeTarget` stores types as STRINGS, not identities.** `params:
   Vec<(String, String, bool)>`, `return_type: Option<String>`, fields likewise.
   `build_type_ref_descriptor(source: &str, …)` only ever sees the rendered
   string. So stamping a composite identity requires either the AST (finding 2,
   at `from_function`) or a reparse (the avoided thing).

5. **Freeze-timing.** `canonicalize_type_annotation(annotation, overlay)` requires
   a freezable type at build time (post-`install_semantic_freeze`). The corpus
   (leaf `string`) canonicalizes cleanly; a non-freezable type_ref would need a
   per-ref fallback (stamp where canonicalizable, reparse arm stays for the rest
   until E5).

6. **`from_type` in the pre-pass has NO direct overlay.** The pre-pass
   `declaration_discovery.rs:101` calls `from_type` from a
   `DeclarationDiscoveryTarget` method that is NOT compiler-scoped (no `self:
   &BytecodeCompiler`). Full composite coverage on the type-target path needs the
   overlay threaded from the pre-pass caller — an extra gap the function-target
   path doesn't have.

## The scope fork (the decision I need before implementing)

Both variants add `identity_high/low` to `__ComptimeTypeRef` + a total
`FrozenPayloadDescriptor -> TypeAnnotation` reconstruction fn (replacing the
slice-0 spike scaffold), and both cover the corpus (`:297`/`:323`, both leaf
`string`).

- **Shape A-lean (corpus + all leaf / simple-nominal type_refs).** Thread the
  overlay into `to_nanboxed` → `build_type_ref_descriptor` only; stamp identity
  via `overlay.identity_of(source_name)` (slice-0 pin proved `identity_of("string")`
  works with no AST, no reparse). Consumer: identity present → `payload_of` →
  reconstruct; absent → the existing reparse arm (unchanged, dead-but-present for
  leaves, live for composites). Footprint: `build_type_ref_descriptor` signature +
  `__ComptimeTypeRef` schema + consumer + reconstruction fn. NO `from_function` /
  `ComptimeTarget`-representation / `declaration_discovery` changes. Composites
  stay on reparse (a ruled residual — `identity_of("Array<int>")` is `None`, per
  slice-0 pin 4).

- **Shape A-full (all U02 incl. composites).** Canonicalize the AST at
  `from_function`/`from_type` (overlay threaded there + the pre-pass
  `declaration_discovery` gap closed), store the `FrozenTypeIdentity` in
  `ComptimeTarget`, stamp in `build_type_ref_descriptor`. Footprint adds:
  `from_function`/`from_type`/`to_nanboxed` signature changes, a `ComptimeTarget`
  representation change, the `declaration_discovery` overlay threading, and ~8
  test-caller updates.

## Recommendation

Shape A-lean discharges the slice-0 PROVEN corpus verdict (both cases leaf) plus
all primitive/simple-nominal type_refs with a bounded, low-risk change, and leaves
composites as an explicit ruled residual on the reparse arm (a leaf-now /
composite-later split — the Shape-B-flavored outcome slice-0 named, though the
overlay IS reachable so it is not strictly E1-D5 Shape B). Shape A-full delivers
the composite reconstruction the spec wants but at a materially larger footprint
(pre-pass overlay gap, target-representation change, test churn).

Because the spec assumed the identity fields already existed and did not
anticipate the `from_function`/pre-pass threading, and because leaf-vs-composite
is a genuine scope fork, I am PAUSING for the ruling rather than proceeding: pick
Shape A-lean (corpus+leaves now, composites a follow-up/E5 split) or Shape A-full
(composites now, bigger). No implementation code written; probe is read-only.
