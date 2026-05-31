# Adversarial Review — lens: container-double-free

> Target: `docs/design/v0.3.3-reference-serialization/round2/DESIGN-ROUND2-DRAFT.md`
> Every claim re-verified against source at workspace HEAD (`main`, `67768f17`).
> VERDICT: **HOLES-FOUND.**

## What I verified holds (the draft's true parts)

- `TypedObjectStorage::drop_fields` HAS a live `HeapKind::Reference` arm
  (`heap_value.rs:3852-3856`, `Arc::decrement_strong_count::<RefTarget>`) and a
  `HeapKind::SharedCell` arm (`:3893-3897`, `::<SharedCell>`). Allocator-symmetric
  with `Arc::into_raw`. The draft's per-field "exactly one decrement" runtime drop
  proof for OBJECT/ENUM is structurally correct **for the fresh-literal
  construction path** (`op_new_typed_object` → `_new`, `object_creation.rs:143-222`;
  `kinded_to_slot` sets `is_heap` for `Ptr(_)` and `field_kinds[i]=Ptr(Reference)`,
  `:495-498`).
- Enum payloads ARE `TypedObjectStorage`-shaped: `compile_expr_enum_constructor`
  emits `OpCode::NewTypedObject` (`collections.rs:1377-1383`). The draft's "EnumStore
  rides the same proof" is correct (the separate v2 `typed_enum.rs` raw layout is not
  the constructor path).
- `HeapElement` structurally forbids `Arc<>`-wrapped element carriers
  (`v2/heap_element.rs:41-45`). So the array-exclusion (P1 fails) is real.
- The MIR solver IS the gate for the escaping container case. EMPIRICAL: a function
  returning `{ r: &x }` is rejected `[B0004] ReferenceStoredInObject` today
  (`solver.rs:1199` → `functions.rs:621`); a non-escaping `{ r: &x }` runs (EXIT 0,
  the `sink_is_local` exemption `solver.rs:1197`).

## BREAK C1 (silent-wrong → UAF) — sibling non-escaping `Local` ref desyncs when the broad flip promotes its referent to SharedCow

This is the container-lens analogue of the round-1 BREAK-1 the draft believes it
closed, RE-OPENED by the broad scope.

The draft §4.3 Delta 1 forces the **referent** slot to `SharedCow` whenever a
reference to it escapes via a container/closure. Round-1's soundness rests on
DESIGN.md §3.3: *"the `Local`/`ModuleBinding` arms are unchanged ... sound because
those refs never escape and **the slot kind never changes under them**."*

That invariant is **false under the broad flip.** In one frame you can have:

```
fn make() {
    let x = 5
    let r_local = &x          // NON-escaping  → stays RefTarget::Local{kind:Int64}
    let obj = { r: &x }       // ESCAPING      → &x→PromotedCell, x FORCED to SharedCow
    use(r_local)              // reads via the Local coordinate
    return obj
}
```

Promoting `x` to SharedCow rewrites its slot to `Arc::into_raw(Arc<SharedCell>)`
with kind `Ptr(HeapKind::SharedCell)` (`op_alloc_shared_local`,
`variables/mod.rs:1530-1534`). But `r_local` is still `RefTarget::Local{slot_index,
kind:Int64}` — and `read_ref_target`'s `Local` arm does a raw
`stack_read_kinded_raw(slot)` returning **`(cell_pointer_bits, frozen Int64)`**,
explicitly discarding the live `_stored_kind` (`variables/mod.rs:2997-2998`,
verified). The reader gets the `*const SharedCell` reinterpreted as an `int`.

- Best case: **silent-wrong** — `use(r_local)` observes the cell's heap address, not
  `5`.
- Worse: if that mis-typed value reaches any heap-typed slot (assignment into an
  inferred-heap binding, a later promotion, a generic container), the `Int64`→heap
  reinterpretation drives a wrong-type `Arc::*_strong_count` → **UAF / double-free**.

The draft NEVER addresses sibling-`Local`-ref reconciliation. Round-1's narrow
scope (ReturnSlot + ModuleBinding) avoided this because promotion only ever targeted
a value already leaving the frame; the broad flip is the first place a referent gets
SharedCow-promoted *while a non-escaping sibling `Local` ref to it is still live*.
SURFACE TO USER — this is an ADR-level soundness gap in Delta 1, not an
implementation detail.

## BREAK C2 (foundational dependency mislabeled) — TypedObject snapshot serialization SURFACES-AND-STOPS; "no new wire arm, Effort M" is wrong

The draft §2.3 / §5.3 / §6 claim the container field "is walked by the ordinary
`TypedObject` serialize arm" and that a `Reference`/`SharedCell` field "emits a
`heap_referents` token," billed as inherited-from-round-1, Effort M, "no new wire
arm." Verified FALSE:

- `slot_to_serializable` has **no `HeapKind::TypedObject` arm** — it falls into the
  `other =>` catch-all that returns `Err("W17-snapshot-TypedObject follow-up")`
  (`snapshot.rs:1120-1127`). A TypedObject cannot be serialized at all today.
- There is **no `heap_referents` table anywhere in the tree.** `SnapshotStore`
  is a content-addressed on-disk blob store (`snapshot.rs:44-56`), not a runtime
  heap-identity map. `slot_to_serializable`/`serializable_to_slot` take `_store`
  underscore-ignored (`:846`, `:1177`).
- `from_snapshot` (`executor/snapshot.rs:235-321`) restores stack, then module
  bindings, then call_stack in **one forward pass**, each value independently via
  `serializable_to_slot`. There is no allocate-all-cells-then-link pass. The draft's
  aliasing-preservation (N container slots + M refs → one restored cell) requires a
  two-pass restore that does not exist.

So Sub-feature A's *live-continuation* leg depends on (i) full recursive TypedObject
deep serialization (an open W17 workstream), (ii) a net-new runtime identity table,
and (iii) a restructured two-pass `from_snapshot`. None is "inherited from round 1."
This is not a double-free by itself, but it makes the §6 KL-4-resolved-by-construction
table's BREAK-4a row ("RESOLVED by single-source table") rest on a table that does
not exist — the aliasing-break is *unresolved*, not resolved.

## BREAK C3 (latent corruption the flip makes reachable) — stale `heap_mask`/`field_kinds` after a scalar overwrites a reference in an `Any` field

`field_kinds: Arc<[NativeKind]>` and `heap_mask` are **immutable post-construction**
(`heap_value.rs:3516`, set once in `_new`, never mutated — verified no write sites).
`write_field_at_idx` releases the prior occupant using the schema-fixed
`stored_kind = field_kinds[idx]` (`typed_object_ops.rs:889,964`), and its
kind-invariance guard is **skipped entirely for `FIELD_TAG_ANY`** (`:897-899`, the
`&& field_type_tag != FIELD_TAG_ANY` short-circuit).

Construction puts `field_kinds[idx]=Ptr(HeapKind::Reference)` + heap_mask-bit-set for
a reference stored in an `Any` field (`kinded_to_slot:495-498`). If that field is
later overwritten by a scalar (`obj.f = 42`), `write_slot_in_place` lays `42` into the
slot but **cannot update `heap_mask`/`field_kinds`**. On the next `drop_fields`, the
heap_mask bit is still set and `field_kinds[idx]` still says Reference →
`Arc::decrement_strong_count(42 as *const RefTarget)` — a **wild-pointer free**.

Reachability caveat (verified empirically): object-literal fields are type-inferred
concretely, so `obj.f = 42` over a `string` field is rejected by type inference
(`type mismatch: cannot assign int to field of type string`), and bare dynamic
property assignment is disabled ("Generic runtime property lookup is disabled"). So
this requires a genuinely `FieldType::Any` field that the flip lets hold a
reference. Today no reference can reach any container field (B0004); the flip is what
first makes a Reference-kinded `Any` slot constructible, so the flip is what arms
this. The draft's KL-4 "RESOLVED by construction" table does not consider
field-kind-vs-slot-content drift on the `Any` path. Tripwire needed: an `Any` field
that ever held a Reference must reject (or re-tag) a later scalar store.

## Why the array exclusion and cycle-leak claims survive (for completeness)

- ARRAY: P1-fails-by-construction is real (`HeapElement` forbids `Arc<>` carriers,
  `heap_element.rs:41-45`); array stays B0004. SOUND.
- CYCLE-as-leak-not-double-free: defensible. A genuine strong Arc cycle keeps every
  member's refcount ≥ 1, so `drop_fields`/`SharedCell::drop` is never *reached*, let
  alone twice (re-entrancy needs an external release-to-0 that the cycle prevents).
  The draft's "leak, not UAF/double-free" holds — but ONLY because the P2 acyclicity
  gate does not yet exist as code, so I cannot exhibit a *wrongly-promoted* cycle
  that double-frees; see C4.

## BREAK C4 (the safety mechanism is vapor) — P2 acyclicity gate has no machinery

The draft's entire cycle-double-free containment is P2: *"the storage planner proves
the stored ref's referent is not the container nor a transitive owner of it."* The
storage planner has **no such analysis.** `detect_escape_status`
(`storage_planning.rs:1014-1031`) yields only `Escaped` (flows to return SlotId(0)) /
`Captured` / `Local`; `decide_slot_storage` (`:905-1006`) has no aliasing-graph,
no referent↔container ownership relation, no transitive-owner check. The draft's
"reuses the existing `sink_is_local` exemption pattern" (§7) is wrong:
`sink_is_local` is `slot_escape_status.get(slot)==Some(Local)`
(`solver.rs:1176-1179`) — return-flow escape, NOT acyclicity. P2 is net-new
non-trivial analysis presented as a one-liner. Until P2 is actually specified +
implemented conservatively, the broad flip has **no containment for cycles**, and
"ACCEPT-DOCUMENTED-LEAK gated to near-zero surface" is unbacked. If P2 is implemented
imprecisely (over-approximates a true cycle as acyclic), the promoted cycle is still
only a leak — but the draft's claim of a *bounded* residual cannot be evaluated
because the bound is unimplemented.

## Bottom line

The OBJECT/ENUM runtime drop proof (no-snapshot) is sound *in isolation*, but the
broad flip as drafted breaks on:
- **C1**: sibling non-escaping `Local` ref desync when its referent is SharedCow-
  promoted (silent-wrong → UAF). Re-opens round-1 BREAK-1 in the broad scope.
  Load-bearing ADR-level hole.
- **C2**: live-continuation leg depends on unimplemented TypedObject deep
  serialization + a non-existent `heap_referents` identity table + a non-existent
  two-pass restore; the BREAK-4a aliasing-break is unresolved, not resolved.
- **C3**: flip-armed stale-heap_mask wild-free on the `Any`-field scalar-overwrite
  path; no tripwire in the design.
- **C4**: the P2 acyclicity gate that contains every cycle hazard does not exist and
  is mis-described as a reuse of `sink_is_local`.
