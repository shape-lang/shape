# E1 #17 slice-6 orphan inventory — the `.source` reparse machinery (dead-but-present)

ADR-009 E1-D2.6. Slice 6 is the TOTAL-deletion slice (one commit,
review-mandatory, E2-D8 sequencing: per-commit-green A → pure-deletion → B).
This doc is the running source-of-truth for what slices 3–5 orphaned but left
**byte-unchanged** for slice-6 deletion. Branch `adr009/e1`, current through
stage-5 of slice 5 (`f6b1e327` + the stage-5 test/docs commit).

> **Load-bearing ordering constraint (read first).** Every row below is
> dead-but-present **only for STAMPED refs**. The `.source` string field and its
> reparse machinery are STILL LIVE for **unstamped (`identity == INVALID`)
> refs** — and those still exist by design after slice 5:
>
> - **module / expression annotation targets** — `to_nanboxed(None)` stamps
>   INVALID (no overlay at those sites);
> - the **`"unknown"` / `Unresolved` fallback** type_ref
>   (`comptime_target.rs`, unresolved return);
> - **optional fields** — the emitted `.source` is the unwrapped inner while the
>   AST is the full `Option<…>`, so the field stamps INVALID by design;
> - every **non-reconstructable type_ref** — applied-generic nominals
>   (`Array<int>`, `Option<T>`, `HashMap`), structural records, bare
>   user-nominals, un-applied generic heads — which the stamp-gate rejects to
>   INVALID (reconstruction of these is **B4/B5**, out of E1).
>
> Therefore **slice 6 must delete the `.source` machinery TOGETHER with the
> unstamped-legacy resolution path (and, for the applied-nominal/record cases,
> only once B4/B5 makes them reconstructable)** — NOT before. Deleting the
> reparse arm while any INVALID-stamping producer survives would break those
> targets. Slice 6's A-phase must first migrate/retire every INVALID producer
> (or gate them under B4/B5), then the pure-deletion phase can remove the rows
> below.

## Primary orphan targets (U02 `.source` reparse — new this slice)

| # | Site (current line @ HEAD) | What | Status for STAMPED refs | Still LIVE for |
|---|---|---|---|---|
| 1 | `crates/shape-runtime/src/type_schema/builtin_schemas.rs:418` | `.string_field("source")` on the `__ComptimeTypeRef` schema | Dead — stamped refs resolve via `identity_high`/`identity_low` (`:419/:420`, which SURVIVE) | Unstamped refs read `.source` |
| 2 | `crates/shape-vm/src/compiler/comptime_builtins.rs:493-494` | The reparse arm in `type_annotation_from_string_or_type_ref_slot`: `string_field_from_typed_object(…, "source")` + `parse_type_annotation_payload(&source)` | Dead — reached only when `identity == INVALID` | All unstamped refs (module/expression/unknown/optional-field/non-reconstructable) |
| 3 | `crates/shape-vm/src/compiler/comptime_builtins.rs:364` | `fn parse_type_annotation_payload` | Dead for stamped refs | Called by the reparse arm (#2) for unstamped refs |
| 4 | `crates/shape-vm/src/compiler/comptime_builtins.rs:370` | The `__type_probe` textual-source fallback inside `parse_type_annotation_payload` (`fn __type_probe(value: {payload}) { value }` + `parse_program`) | Dead for stamped refs | The unstamped reparse path |

`identity_high` / `identity_low` on `__ComptimeTypeRef`
(`builtin_schemas.rs:419-420`) are the STAMPED carrier and **survive** — they
are the replacement, not orphans. `name` / `kind` stay (read by
`target_params_and_return_expose_type_refs` and general reflection); only
`source` and its reparse consumers are orphaned.

## Carried-forward orphans (slices 3–4, still awaiting slice-6 deletion)

| Site (current line @ HEAD) | What | Note |
|---|---|---|
| `crates/shape-vm/src/compiler/statements.rs:148` | `fn serialize_directive_payload` | Fully caller-less since slice 4 (extend was its last user). Dead-code warns — the E2-D8 byte-unchanged consequence; a warning, not a test flip. |
| `crates/shape-vm/src/compiler/comptime_builtins.rs` (`__emit_extend` builtin) | `register_typed_fn_1` + `serde_json::from_str::<ExtendStatement>` | Registered but never emitted (slice 4 moved extend to the typed store+index). Dead-but-present, no warning. |
| `crates/shape-vm/src/compiler/comptime_builtins.rs:365` | `serde_json::from_str::<TypeAnnotation>` first branch of `parse_type_annotation_payload` | Carried from slice 3. Orphaned once #2 above is deleted (it is only reachable through the reparse arm). |

## Already retired (do NOT re-list as orphans)

- The **slice-0 reconstruction spike module** (`e1_slice0_reconstruction_spike`
  + `spike_reconstruct_primitive`/`panic!`) — DELETED in slice-5 stage 2
  (`f8e772be`). E1-D2 chartered slice 5 to retire it; done.

## Explicitly NOT orphaned (stays LIVE past slice 6)

- `reconstruct_type_annotation` + `canonical_primitive_spelling` — the stamped
  identity route (the replacement for the reparse arm).
- `FreezeOverlay::canonicalize_type_projection` / `payload_of` / the
  `composites` memo — the one identity computation.
- `__emit_extend_items` + `parse_extend_items_slot` + `CheckedItem` + the
  `extend_method*` producers — the computed-extend path (E2), a different
  construct, never on the `.source` reparse path.
- The slice-3 `COMPTIME_DIRECTIVE_TYPES` store + the Int64 literal-carrier arm —
  the U01 literal transport, kind-disjoint from the U02 type_ref path.

## Slice-6 close checklist (for the deletion agent)

1. Migrate or gate EVERY INVALID-stamping producer (module/expression targets,
   the `"unknown"` fallback, optional fields) so no live path depends on the
   `.source` reparse — else deleting rows 1–4 breaks those targets.
   Applied-nominal / record / bare-nominal type_refs stay INVALID until B4/B5;
   coordinate the deletion of their reparse dependence with that work.
2. Delete rows 1–4 (the `.source` field + reparse arm + `parse_type_annotation_payload`
   + `__type_probe`) together, in the pure-deletion phase.
3. Delete the carried-forward slice-3/4 rows (`serialize_directive_payload`,
   `__emit_extend`, the `serde_json::<TypeAnnotation>` branch) in the same
   pure-deletion phase.
4. Workspace-wide both-spelling closure sweep (E2-D8 discipline); `just
   check-clean` green; the recorded baselines (st-annotations 10-name, vmlib
   7-name + `nested_exact` flapper, st-comptime 3-name @ `-j1`) UNMOVED.
