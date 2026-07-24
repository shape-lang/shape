# E5 CKPT-4 report — producer migration + irreducible-class rulings

**Branch** `adr009/e5` · **base** `f5b46958` (CKPT-3) · **additive — deletes
nothing.** CKPT-4 migrates the reconstructable `__ComptimeTypeRef` producers to
STAMP, rules the INVALID identity surface LOUD, and SURFACES the one irreducible
producer the design's 5-class inventory mis-dispositioned (class E / item_fn).

## What landed

1. **Class A (module targets) — MIGRATED.** `statements.rs::module_target_fields`
   now returns each member's declared-type AST; `ComptimeTarget::from_module` takes
   `&[(String, String, Option<TypeAnnotation>)]` and populates `field_type_asts`;
   the module handler call site acquires the freeze BEFORE `to_nanboxed` and passes
   `Some(overlay)`. Typed members (`let x: int`) STAMP; synthetic members
   (functions/types/modules/annotations) → `kind:"Unresolved"` → LOUD at the
   consumer. The `for_expression` sibling keeps `None` (no stampable fields; its
   sole `type_ref` is the class-C Unresolved return).

2. **Class B (optional fields) — MIGRATED.** `build_field_descriptor_array` drops
   the `!is_optional → None` gate; it stamps the UNWRAPPED-inner AST
   (`TypeAnnotation::option_inner`) for an optional field (matching the emitted
   unwrapped `.source`/`.type`, keeping source↔identity consistent) and the full
   AST otherwise.

3. **Class D (`type_info` reflection) — MIGRATED.** `build_type_info_heap_value`
   threads `Some(freeze)` + the struct field ASTs into `build_field_descriptor_
   array`, and stamps the top-level `type_ref` via a new
   `comptime_target::build_named_type_ref_descriptor` (`Basic(type_name)` →
   `bare_nominal_name_of` / primitive inverse; Unresolved names stay INVALID → LOUD).

4. **Class C + A5 (INVALID identity) — RULED LOUD.** The consumer
   `type_annotation_from_string_or_type_ref_slot` rejects ANY `__ComptimeTypeRef`
   whose `identity == INVALID` as a named surface-and-stop (citing `kind`/`name`),
   BEFORE the (untouched) `.source` arm. Subsumes class C (`kind:"Unresolved"`),
   scoped generic parameters, un-applied heads. The `.source` arm is now UNREACHED
   by any `__ComptimeTypeRef` — kept byte-identical as the runtime-dead net until
   CKPT-5, via a `!= INVALID` guard immediately after the reject.

5. **Records STAMP** (CKPT-3 already shipped record-IN spelling) — migrated in A/B/D
   like any stampable type; not ruled loud.

## What is SURFACED (blocked, not forced) — class E

The design's class E ("reject the bare-string type-payload arm loud") is **BLOCKED**.
That arm is a **SANCTIONED, documented carrier** for `item_fn(name, return_type:
string | TypeRef, value)` + `extend_method`, which have no Int64/TypeRef alternative
today and inherently need `parse_type_annotation_payload` to turn a spelling string
into an AST. Applying the reject broke ~19 item-generation tests (measured). Per the
standing *"SURFACE it, don't force it"* ruling, class E was reverted to its reparse
and surfaced. **Consequence:** the `.source` field + arm are unreached (CKPT-5 can
delete them), but `parse_type_annotation_payload` + `__type_probe` retain the live
item_fn/extend caller → CKPT-5 cannot delete the reparse machinery until item_fn/
extend get a sanctioned carrier. **DECISION REQUIRED** (see e5-decisions.md CKPT-4
§"class-E blocker").

## Pins

- `e1_s5_ckpt4_unstamped_typeref_is_named_surface_and_stop_not_source_reparse`
  (rewrite of the former `..._falls_through_to_source_arm_bytewise`): an unstamped
  (INVALID) `__ComptimeTypeRef` is a NAMED surface-and-stop — even with a valid
  parseable `.source` — never a silent reparse. The design §4 "successor" pin.
- `e1_s5_ckpt4_typeref_producers_stamp_invalid_rejects_loud_string_arm_surfaced`
  (exit criterion): concrete producer STAMPS end-to-end through the real
  `to_nanboxed` → consumer resolves via identity; class-C/INVALID rejects LOUD; the
  bare-string arm STILL reparses (pins the surfaced item_fn/extend residual).
- `e1_s5_ckpt4_recursive_named_record_reconstructs_and_terminates` (deferred CKPT-3
  fold-in): `type Tree { kids: Array<Tree> }` reconstructs + terminates via the
  bare-name self-ref leaf.

## Gate (FAILED-name sets vs baseline — ZERO regressions)

| Gate | Baseline | Post-CKPT-4 | Verdict |
|---|---|---|---|
| `shape-vm --lib` | 3587 / 7 | 3589 / 7 (+2 pins) | same 7 pre-existing; ZERO new |
| `e1_s5` | 29 / 0 | 31 / 0 | green |
| `no_json` | 2 / 0 | 2 / 0 | green |
| `comptime` | 271 / 3 | 271 / 3 | exact 3 pre-existing; ZERO flips |
| `annotations_comptime` | 117 / 10 | 117 / 10 | exact 10 pre-existing; ZERO flips |
| `just check-clean` | exit 0 | exit 0 | green (1 pre-existing warning) |
| `just check-no-dynamic` | success | success | green |

No TP rebaselines were needed: no producer flipped a test (concrete-typed emit
tests still stamp → resolve identically); the ~19 tests the class-E reject would
have flipped were reverted and surfaced, not rebaselined.

`.source`/reparse machinery UNTOUCHED (byte-identical): the `.string_field("source")`
schema field, `parse_type_annotation_payload`, `fn __type_probe(...)`, and the
`.source` reparse arm.

## Verdict

CKPT-4 is **DONE for the `__ComptimeTypeRef` identity surface** (A/B/D migrated;
class-C/INVALID ruled loud; records stamp; `.source` arm unreached — CKPT-5 can
delete it). It is **BLOCKED on class E** (item_fn/extend bare-string carrier) — the
completeness finding — which needs a typed-carrier migration decision before the
full string-arm exit criterion (and the CKPT-5 reparse-fn deletion) can close.
