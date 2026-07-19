# E1 #17 slice-0 report — FrozenTypeIdentity→TypeAnnotation reconstruction spike

Committed evidence for the E1-D1 obligation: an executable spike proving or
refuting that the expr-form comptime type refs can be resolved off their B7
FrozenTypeIdentity descriptors to a `TypeAnnotation` WITHOUT reparsing
`__ComptimeTypeRef.source`, for exactly the corpus cases at
`tools/shape-test/tests/annotations_comptime/type_mutation.rs:297` and `:323`.
Authored on branch `adr009/e1`. Executable pins live inline in
`crates/shape-vm/src/compiler/comptime_builtins.rs`
(`#[cfg(test)] mod e1_slice0_reconstruction_spike`) — supervisor runs them; this
author does not build.

---

## VERDICT: PROVEN for the corpus cases (with two bounded slice-5 build-outs)

The two named corpus cases both resolve the leaf primitive `string`:

- `:297` — `set return (target.params[0].type_ref)` where param 0 is
  `value: string`.
- `:323` — `set param left: (target.params[1].type_ref)` where param 1 is
  `right: string`.

Both are reconstructable to the byte-identical `TypeAnnotation` the current
reparse consumer produces, via a reparse-free route that starts from the B7
descriptor machinery. The spike demonstrates the route end to end and asserts
equivalence against both the reparse oracle (`parse_type_annotation_payload`)
and the real production consumer
(`type_annotation_from_string_or_type_ref_slot`).

This is **not** "impossible without new machinery" — no escalation. It is
PROVEN, gated on two mechanical build-outs (neither a new subsystem), plus one
named boundary that decides whether E1 carries ALL expr-form U02 or only the
leaf subset. See [slice-5 shape](#recommended-slice-5-shape).

---

## Mechanism found

The non-reparse resolution path, all three hops on machinery that already ships:

```
overlay.identity_of(name)          // FreezeOverlay, semantic_freeze.rs:675
    -> overlay.payload_of(identity) // FreezeOverlay, semantic_freeze.rs:692
    -> reconstruct(FrozenPayloadDescriptor) -> TypeAnnotation   // NEW (slice 5)
```

1. **Name → identity.** `FreezeOverlay::identity_of(&str) -> Option<FrozenTypeIdentity>`
   (`semantic_freeze.rs:675`) is a frozen-name-table lookup (`frozen_type_id`),
   NOT a parse. The corpus type_ref's `name` field carries exactly this leaf
   name (`"string"`), built by `build_type_ref_descriptor`
   (`comptime_target.rs:106`, name via `type_ref_name_from_source` :89).

2. **Identity → complete descriptor.** `FreezeOverlay::payload_of(identity)`
   (`semantic_freeze.rs:692`) returns `FrozenPayloadDescriptor::Primitive(FrozenPrimitive::String)` —
   a COMPLETE typed descriptor (B7 #11 shipped; Primitive is an ENABLED
   category, `payloads.rs:240`). The `FrozenPrimitive` sub-algebra is sealed and
   exhaustive (`comptime_reflection.rs:467`).

3. **Descriptor → TypeAnnotation.** A total reconstruction over the descriptor
   algebra. For a primitive this inverts the family to its canonical Shape
   spelling (`FrozenPrimitive::String` → `TypeAnnotation::Basic("string")`),
   matching what `parse_type_annotation_payload("string")` yields. **This
   function does not exist today** — see [gap 2](#gaps-and-risks).

The equivalence the spike proves: this route yields the SAME `TypeAnnotation`
as the current reparse consumer on the exact `__ComptimeTypeRef` value the corpus
handlers produce.

### Why the descriptor route, not a name echo

`FrozenTypeIdentity::from_canonical_descriptor` is a one-way SHA-256 fold
(`type_reflection.rs:97`) — the identity is deliberately non-invertible, and the
scout's premise ("reconstruct from the identity") cannot mean inverting the hash.
The reconstruction is driven by the COMPLETE structural descriptor `payload_of`
returns (the B7 deliverable), keyed on the sealed `FrozenPrimitive`/
`FrozenPayloadDescriptor` variant — never by re-reading `.source`, never by the
parser. Pin 3 shows the reconstruction tracks the descriptor variant (string vs
bool diverge), not the input name.

---

## Pins (supervisor runs; filter `cargo test -p shape-vm --lib e1_slice0_reconstruction_spike`)

| Pin | Asserts | Meaning |
|---|---|---|
| `e1_s0_string_reconstructs_off_descriptor_without_reparse` | `payload_of(identity_of("string"))` = `Primitive(String)`; reconstruct = `Basic("string")` = `parse_type_annotation_payload("string")` | the descriptor is a sufficient reparse-free source for the corpus type |
| `e1_s0_descriptor_route_matches_current_reparse_consumer` | descriptor route == `type_annotation_from_string_or_type_ref_slot(build_type_ref_descriptor("string"))` | agrees with the REAL production consumer on the exact corpus input |
| `e1_s0_reconstruction_tracks_the_descriptor_not_the_name` | string vs bool reconstructions diverge; each == its own reparse oracle | descriptor-driven, not a name echo |
| `e1_s0_composite_typeref_is_the_named_leaf_boundary_gap` | `identity_of("string")` = Some; `identity_of("Array<int>")` = None | the leaf-vs-composite boundary that sizes slice 5 |

---

## File:line anchors (re-verified at implementation time, HEAD of `adr009/e1`)

- Corpus cases: `tools/shape-test/tests/annotations_comptime/type_mutation.rs:290-311`
  (`set_return_accepts_type_ref_expression`, `set return (…type_ref)` at :297)
  and `:313-337` (`set_param_type_accepts_type_ref_expression`,
  `set param left: (…type_ref)` at :323). Both types are `string`. (Scout's
  `:297`/`:323` line anchors confirmed exact.)
- Current reparse consumers:
  - `type_annotation_from_string_or_type_ref_slot` — `comptime_builtins.rs:338`;
    reads `__ComptimeTypeRef.source` at :373, reparses via
    `parse_type_annotation_payload` at :374.
  - `parse_type_annotation_payload` — `comptime_builtins.rs:286` (`serde_json`
    attempt then `parse_program` fallback :292-303).
  - `__emit_set_param_type` — `comptime_builtins.rs:1279`, calls the resolver at
    :1308.
  - `__emit_set_return_type` — `comptime_builtins.rs:1359`, calls the resolver at
    :1376.
- Producer of the corpus value: `build_type_ref_descriptor` —
  `comptime_target.rs:106`; call sites `comptime_target.rs:210` (fields),
  `:450`/`:466` (params/return). `__ComptimeTypeRef` schema = `{name, kind,
  source}` strings only — `builtin_schemas.rs:408`.
- Descriptor machinery (B7): `FreezeOverlay::identity_of`
  `semantic_freeze.rs:675`; `payload_of` `:692`; `FrozenPayloadDescriptor`
  `type_reflection/payloads.rs:246`; `FrozenPrimitive` sealed algebra
  `comptime_reflection.rs:467`; `PRIMITIVE_SYNONYM_FAMILIES` (the single
  name↔primitive table) `type_reflection.rs:31`; identity hash
  `type_reflection.rs:97`.
- Overlay/registry test surface: `overlay_for_tests` `semantic_freeze.rs:1378`;
  `current_registry` stdlib default fallback `type_schema/current.rs:123`.

---

## Gaps and risks

Two build-outs and one boundary. None is a new subsystem; none is dynamic
fallback.

1. **Consumer threading gap (mechanical wiring).** The `Arc<FreezeOverlay>` is
   NOT in scope where the expr-form consumers are registered.
   `__emit_set_param_type`/`__emit_set_return_type` live in
   `comptime_builtins_module_base` (`comptime_builtins.rs:837`), which does not
   receive `freeze`; only `register_frozen_reflection_builtins`
   (`comptime_builtins.rs:1500`, called from `create_comptime_builtins_module`
   :825) gets the handle. Slice 5 threads the SAME already-existing handle into
   the emit-set registration (or relocates those two builtins beside the other
   freeze-consuming intrinsics). The overlay is created and Arc-cloned per closure
   already — this is local plumbing, not a missing dependency.

2. **Descriptor→TypeAnnotation reconstruction does not exist yet.** The B7
   descriptor machinery lowers descriptors FORWARD to comptime heap values
   (`build_frozen_type_heap_value`, `payloads.rs:358`); there is no
   `FrozenPayloadDescriptor -> TypeAnnotation` (nor `FrozenPrimitive ->
   TypeAnnotation`) reconstruction anywhere. Slice 5 builds it as a total
   function over the sealed algebra. For primitives it inverts
   `PRIMITIVE_SYNONYM_FAMILIES` to the canonical spelling (the ONE table — no
   second name table, per `type_reflection.rs:24-30`). The spike's
   `spike_reconstruct_primitive` is the corpus-scoped stand-in; its `panic!` arm
   marks the slice-5 build-out surface.

3. **Leaf-vs-composite boundary (sizes slice 5 — the E1-D2 decision).** The
   name-route in hop 1 is lossless ONLY for leaf types. `identity_of` is a
   frozen-name lookup: a composite spelling (`Array<int>`, `int?`, `(int,
   string)`) is not a frozen name and resolves to None (pin 4). For composite
   expr-form type_refs the type_ref's stringy `name`/`kind` fields are
   insufficient — only `.source` (or a producer-stamped identity) recovers the
   full type. The **named gap**: general expr-form U02 composites. The corpus is
   entirely inside the leaf-resolvable set, so the corpus verdict is PROVEN
   regardless of how this boundary is disposed.

Non-risks (checked, ruled out): the identity hash being one-way is NOT a blocker
— reconstruction reads the preserved structural descriptor, never inverts the
hash. The registry is available in the pins — `current_registry()` falls back to
the stdlib-populated process default (`current.rs:123`), which carries the
`__ComptimeTypeRef` schema.

---

## Recommended slice-5 shape

Two viable shapes; the choice is the E1-D2 slice-plan decision, driven by gap 3.

- **Shape A — full-E1 U02 (producer-stamped identity).** In
  `build_type_ref_descriptor`, canonicalize the real AST annotation to a
  `FrozenTypeIdentity` at type_ref BUILD time (the producer has the annotation
  before it renders `source`) and stamp it onto the type_ref — the existing
  `COMPTIME_FROZEN_TYPE_REF_SCHEMA` already carries `identity_high`/
  `identity_low` fields (`builtin_schemas.rs:423`). The consumer then resolves
  purely off the stamped identity via `payload_of` + the new reconstruction, for
  BOTH leaf and composite forms — `.source` reparse deletes entirely within E1.
  **Open question:** is the FreezeOverlay available at `comptime_target` build
  time (during handler execution) to canonicalize the annotation? This is the
  one feasibility item slice 5 (or a slice-4→5 hinge) must confirm; if yes,
  Shape A carries all of U02 and is the cleaner terminal state.

- **Shape B — ruled E1↔E5 split (leaf now, composite deferred).** E1 takes the
  leaf/simple-nominal expr-form type_refs off reparse (covers the corpus and its
  family) via the identity_of(name) route; composite expr-forms stay on
  `.source` until E5 deletes the field. Names the exact gap (composite expr-form
  U02) in the E1↔E5 boundary.

**Recommendation:** attempt Shape A. It is the only shape that lets E1 own U02
end to end (consistent with E1's charter: "the transport off reparse"), it reuses
an already-declared schema, and the producer demonstrably has the AST annotation.
Fall back to Shape B only if the overlay is proven unavailable at
`comptime_target` build time. Either way, gaps 1 and 2 are common prerequisites.

### Effect on the E1-D2 slice plan

- Slice 5 gains an explicit **pre-check**: confirm overlay availability at
  `comptime_target` build time (Shape A gate). This is a small read-only probe,
  best run at the top of slice 5 (or folded into slice 2's ParamId work, which
  already touches the frozen-callable resolution surface).
- The reconstruction fn (gap 2) and the consumer threading (gap 1) are slice-5
  scoped and independent of the Shape A/B choice — they can land first.
- No change to slices 1–4. Slice 6 (TOTAL deletion) inherits whichever of
  `.source`-reparse-for-composites (Shape B) or nothing (Shape A) survives; under
  Shape A the `.source` reparse in `type_annotation_from_string_or_type_ref_slot`
  is fully dead and deletes with the rest of the JSON transport.

---

## Forbidden-patterns check

No dynamic fallback introduced or preserved. The spike does not keep a reparse
"for one edge case", does not rename the reparse path, and does not add a
conversion opcode. The reconstruction is a forward total function over a sealed
descriptor algebra — the opposite of a tag-decode bridge. The slice-5 terminal
state (Shape A) DELETES the reparse path rather than gating it. The
leaf-vs-composite boundary is surfaced as a named gap for a user/supervisor
disposition (Shape A vs B), not rationalized into a retained fallback.
