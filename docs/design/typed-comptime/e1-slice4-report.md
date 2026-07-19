# E1 #17 slice-4 report — the `extend` directive → typed store+index carrier

ADR-009 E1-D2.4. The direct-block `extend Type { … }` rewrite directive comes off
the JSON directive protocol (`serialize_directive_payload` → `__emit_extend`
serde_json reparse) onto the per-run typed store+index transport. Branch
`adr009/e1`, on top of the closed slice-3 (`b94b01cc`).

**Milestone:** after this slice NO live producer feeds `serialize_directive_payload`
— it joins the slice-6 orphan inventory (confirmed below).

## Carrier choice + rationale (constraint 1)

**Kept `ComptimeDirective::Extend(ExtendStatement)`; added a COMPILE-populated
`COMPTIME_EXTEND_STATEMENTS` store + a new `__emit_extend_checked(index)` builtin,
mirroring `__emit_replace_body_checked` exactly.**

Why not reuse the E2 computed-extend carrier (`__emit_extend_items` ←
`parse_extend_items_slot` ← a `__CheckedItem` handle)? Different SOURCE and
LIFECYCLE, not a stylistic choice:

- The E2 path is for COMPUTED extends (`extend (item_fn(…) / extend_method(…))`):
  a typed producer mints a `CheckedItem` at VM-EXECUTE time, stashed in
  `COMPTIME_CHECKED_ITEMS` (execute-populated, cleared pre-execute).
- A direct-block `extend Type { … }` is LITERAL AST known at handler-COMPILE
  time — exactly like a `replace body { … }` block. It needs a COMPILE-populated
  store (the `COMPTIME_REPLACE_BODIES` sibling), cleared at run ENTRY; the
  execute-populated `CheckedItem` store would be wiped by its pre-execute clear.

So the correct reuse is the **store+index PATTERN** (the sanctioned slice-3 /
replace-body carrier, constraint 3) applied to the compile-time extend payload,
plus the **`Extend` directive + its downstream `apply_comptime_extend`
materialization** (byte-unchanged). This is NOT the parallel-carrier defection: I
did not mint a second `__CheckedItem`-shaped handle or a second `Vec<Item>`
carrier meeting the E2 path at a conversion layer. The two extend transports stay
distinct by CONSTRUCT — direct-block `Extend` vs computed `ExtendItems` — exactly
as they already were; only the direct-block transport moved off JSON.

Routing the direct block through E2's `ExtendItems` directive was considered and
rejected: it would require a semantic wrap (`Extend` → `Item::Extend` →
`ExtendItems`) and switch materialization (`apply_comptime_extend` →
`apply_comptime_extend_items`) — a behavior change beyond a transport migration,
for no carrier-unification benefit (the compile-populated store is still required
either way).

## Composition invariant (constraint 2) — no double-wrapping

The extend methods carry BODIES, so the slice-1 construction/install split is in
view. Slice 4 is a **transport migration only** and does NOT add `CheckedBody`
construction:

- The install-side half is ALREADY present and unchanged: extend method bodies
  materialize via `apply_comptime_extend`, which runs during directive processing
  INSIDE the C2 `InstallTransaction` bracketing `compile_in_place`. My change
  moves the `ExtendStatement` from a JSON string to a typed store index; it does
  NOT move `apply_comptime_extend` in or out of that transaction.
- The direct-block extend was never on the `CheckedBody` construction path;
  adding it now would be the double-wrap constraint 2 warns against. The typed
  carrier + the existing C2 install transaction discharge the "both halves"
  invariant by composition, stated here explicitly rather than re-wrapped.

## What landed

- `COMPTIME_EXTEND_STATEMENTS` store + `push`/`at`/`clear` (compile-populated,
  cleared at handler-run entry in `comptime.rs` beside the replace-body and
  literal-type clears).
- Emit: `emit_comptime_extend_directive` stashes the `ExtendStatement` and emits
  `__emit_extend_checked(Int(index))`, not `serialize_directive_payload` JSON.
- Consumer: new `__emit_extend_checked(index: int)` builtin fetches by index →
  `ComptimeDirective::Extend` (same directive, typed transport). Mirrors
  `__emit_replace_body_checked`.

## Tests (supervisor runs)

Unit carrier pins (`cargo test -p shape-vm --lib e1_extend_carrier`):
`extend_carrier_index_restarts_per_run_no_stale_leak`,
`missing_extend_index_resolves_to_none`.

End-to-end (`cargo test -p shape-test --test annotations_comptime`):
`direct_block_extend_adds_callable_method_via_typed_transport` (one method),
`direct_block_extend_multiple_methods_via_typed_transport` (two methods).
Both use CAPTURE-FREE method bodies, keeping them off the JITExecutor
empty-capture debt (the green `extend_item_fragment_…` precedent shows capture-free
generated code asserts cleanly with `expect_output`); calling the generated method
is the load-bearing proof it was added (a missing method is a run error).

## Orphaned-inventory for slice 6 (E2-D8 staging — byte-unchanged, NOT deleted)

| Site | What | Note |
|---|---|---|
| `statements.rs:148` | `serialize_directive_payload` | **Now fully caller-less** (extend was its last user). **Will dead-code warn** — the E2-D8 byte-unchanged consequence; warnings are the supervisor-accepted class (`just check-clean` = `cargo check`, warnings don't fail). |
| `comptime_builtins.rs:1340` | `__emit_extend` builtin (`register_typed_fn_1` + `serde_json::from_str::<ExtendStatement>`) | Registered but never invoked (no emitter). Dead-but-present, no warning. |
| `comptime_builtins.rs:327-329` | `serde_json::from_str::<TypeAnnotation>` first branch of `parse_type_annotation_payload` | Carried forward from slice 3 (still orphaned). |

`__emit_extend_items` + `parse_extend_items_slot` + `CheckedItem` + the
`extend_method*` producers STAY LIVE (the computed-extend path) — NOT in the
inventory.

## Differential expectation

Behavior-preserving: the same `ExtendStatement` is carried typed instead of JSON;
`apply_comptime_extend` is unchanged. Zero baseline test movement expected. The
one new signal is the `serialize_directive_payload` **dead-code warning** (a
warning, not a test flip) — flagged so it isn't mistaken for a regression. Any
actual test flip: classify, no silent rebaseline.

## Forbidden-patterns check

No re-serialization "just at this boundary" — the `ExtendStatement` is carried as
a typed AST value end to end; no JSON, no serde_json reparse on the direct-block
path. The store+index is the sanctioned E2 carrier pattern (not a new dynamic
surface, not a parallel extend carrier — see the carrier rationale). The orphaned
JSON machinery is staged for slice-6 deletion, not renamed or retained as a
"compatibility layer". `resolve_param_id` and the slice-3 literal path are
untouched.
