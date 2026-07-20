# E1 #17 slice-3 report — U01 literal directive types → typed store+index

ADR-009 E1-D2.3. The U01 literal-form directive types stop flowing through
`serialize_directive_payload` JSON and move onto the per-run typed
carrier-store + opaque-index transport (the proven E2 replace-body pattern).
Branch `adr009/e1`, on top of the closed slice-2 (`e402da29`).

## Territory re-verification (vs scout anchors)

Re-verified after slice-2 churn — surface drift only, no material divergence:

| Directive emitter (`statements.rs`) | Was | Classification | This slice |
|---|---|---|---|
| `emit_comptime_set_param_type_directive` (:646, serialize @:652) | JSON string | **U01 literal** | MIGRATED → store+index |
| `emit_comptime_set_return_type_directive` (:685, serialize @:690) | JSON string | **U01 literal** | MIGRATED → store+index |
| `emit_comptime_set_param_value_directive` (:630) | typed `Expr` (no JSON) | already typed | untouched |
| `emit_comptime_set_param_type_expr_directive` (:663) | `__ComptimeTypeRef` expr | **U02** | untouched (slice 5) |
| `emit_comptime_set_return_expr_directive` (:698) | `__ComptimeTypeRef` expr | **U02** | untouched (slice 5) |
| `emit_comptime_extend_directive` (:618, serialize @:618) | JSON string | **extend** | untouched (slice 4) |
| `emit_comptime_replace_body_directive` (:700) | typed store+index | done (E2) | untouched |

No ambiguous U01/U02 site: the literal and expr forms are DISTINCT emitters, so
the boundary is syntactic, not a per-site judgment.

## What landed

- **`COMPTIME_DIRECTIVE_TYPES`** (`comptime_builtins.rs`) — a per-run
  `RefCell<Vec<TypeAnnotation>>` carrier store, with `push_comptime_directive_type`
  / `comptime_directive_type_at` / `clear_comptime_directive_types`. Identical
  lifecycle to `COMPTIME_REPLACE_BODIES`: **compile-populated**, so cleared at
  `execute_comptime_with_annotation_handler` ENTRY (`comptime.rs`, beside
  `clear_comptime_replace_bodies`), BEFORE the inner compile that stashes into it
  — a pre-execute clear would wipe it. Per-run clear ⇒ the pre-pass/pass-2
  double-compile never leaks a stale type.
- **Emit side** — the two literal emitters push the typed `TypeAnnotation` and
  emit `Literal::Int(index)` instead of `serialize_directive_payload` → a JSON
  string. No `serde_json`, no reparse.
- **Consumer side** — `type_annotation_from_string_or_type_ref_slot` gains a
  leading `NativeKind::Int64` branch that fetches the stored annotation by index.
  The kind is disjoint from the legacy `String` payload and the U02
  `Ptr(TypedObject)` `__ComptimeTypeRef`, so it never shadows those paths. The
  existing `__emit_set_param_type` / `__emit_set_return_type` builtins are REUSED
  (their `type_payload` param is `type_name: "unknown"`, so an int arg needs no
  signature change) — a new builtin would fragment the still-live U02 handling.

Set-param/set-return literal consumers still route through slice-2's
`resolve_param_id` (unchanged). No `CheckedBody` is constructed — these directives
carry a type, not a body (per scope item 3).

## Tests (supervisor runs)

Unit pins (`cargo test -p shape-vm --lib e1_literal_type_carrier`):

| Test | Covers |
|---|---|
| `literal_type_carrier_index_restarts_per_run_no_stale_leak` | store lifecycle — per-run clear, no stale leak (mirrors the replace-body carrier pin) |
| `literal_type_index_resolves_through_the_consumer_without_reparse` | the consumer's `Int64` branch fetches the stored typed annotation |
| `missing_literal_type_index_is_a_named_error_not_a_panic` | out-of-range index → named error, not a panic |

End-to-end pins (`cargo test -p shape-test --test annotations_comptime`):

| Test (`annotations_comptime/directives.rs`) | Covers |
|---|---|
| `set_param_type_literal_applies_via_typed_transport` | `set param x: int` full emit→store→exec→apply, prints the arg |
| `set_return_type_literal_applies_via_typed_transport` | `set return int` full path, runs green |

## Differential expectation

Behavior-preserving: the literal type is applied identically, just carried typed
instead of JSON. Existing literal-form tests (e.g. `reference_provenance_tests`
`set param value: int`, the `extension_integration` `set param uri: string`
fixtures) should stay green. Suite MOVEMENT is expected ONLY if a test asserts the
JSON transport itself — a workspace sweep found none (no test asserts a serialized
`TypeAnnotation` payload string). Any flip: classify, no silent rebaseline.

## Orphaned-inventory for slice 6 (E2-D8 staging — dead-but-present, NOT deleted here)

Slice 3 orphans exactly one arm; it is left byte-unchanged for the slice-6
single-commit deletion:

- **`comptime_builtins.rs:327-329`** — the `serde_json::from_str::<TypeAnnotation>`
  FIRST branch of `parse_type_annotation_payload` (the JSON-deserialize half of
  the U01 round-trip). Post-slice-3 no producer emits a `serde_json`-serialized
  `TypeAnnotation`: the two literal emitters now carry an index; `item_fn` passes
  plain type-name strings (`"int"`, `"string"`) that fail `serde_json` and take
  the source-reparse fallback; the U02 path passes a `__ComptimeTypeRef` object.
  The rest of `parse_type_annotation_payload` (the source-reparse fallback) and
  the function itself STAY LIVE — consumed by `item_fn` plain-name returns and the
  U02 `__ComptimeTypeRef.source` reparse until slices 5/6.

NOT orphaned (still live, so NOT in the inventory):
- `serialize_directive_payload` (`statements.rs:148`) — still used by extend
  (:618, slice 4).
- the `String` payload branch of `type_annotation_from_string_or_type_ref_slot`
  and the source-reparse fallback — still used by `item_fn` plain-name returns.

## Forbidden-patterns check

No re-serialization "just at this one boundary" — the literal type is carried as a
typed `TypeAnnotation` end to end; no JSON, no source string, no reparse on the
U01 path. The orphaned JSON-deserialize arm is staged for deletion, not renamed or
retained as a "compatibility layer". The store is the sanctioned E2 carrier
pattern, not a new dynamic-dispatch surface.
