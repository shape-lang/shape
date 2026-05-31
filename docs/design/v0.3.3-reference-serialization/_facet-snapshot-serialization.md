# v0.3.3 reference-serialization — Facet: snapshot-serialization

**Facet owner question:** Design the `SerializableVMValue` reference arm + the
restore identity-map. A reference serializes as an identity-handle to its (now
heap/RC'd) referent; on restore, an identity-map (`original-ptr → restored-ptr`)
dedupes so multiple references re-point at the **same** restored referent.
Handle cycles / self-reference / multiple-refs-to-one-referent / `&mut` vs `&`
mode. Integrate with W17 deep-frame-stack restore. Reuse the SharedCell
serialize-with-shared-identity mechanism.

---

## 0. The one structural fact that reshapes this facet

Read the actual reference carrier before designing the wire arm. `RefTarget`
(`crates/shape-value/src/reference.rs:41-99`) has exactly three live variants
post-V3-S5-ckpt-4:

```rust
// crates/shape-value/src/reference.rs:41
pub enum RefTarget {
    Local        { frame_index: u32, slot_index: u32, kind: NativeKind },  // :54
    ModuleBinding{ binding_idx: u32,                  kind: NativeKind },  // :66
    TypedField   { receiver: TypedObjectPtr, field_offset: u32, kind: NativeKind }, // :84
}
```

Two of the three arms are **already symbolic / location-relative, not
pointer-relative**:

- `Local { frame_index, slot_index, kind }` — `frame_index` is an index into
  `VirtualMachine.call_stack`, `slot_index` is an offset from that frame's
  `base_pointer` (construction at `executor/variables/mod.rs:2541-2545`;
  resolution at `:2888-2901` computes `base_pointer + slot_index`). It contains
  **no heap pointer**. The `u32::MAX` sentinel (`:2522-2526`, `:2888`) marks a
  top-level (frameless) ref → `base_pointer = 0`.
- `ModuleBinding { binding_idx, kind }` — `binding_idx` indexes
  `VirtualMachine.module_bindings` (construction `:2552-2555`; resolution
  `:2921-2922` via `module_binding_read_kinded_raw`). Also **no heap pointer**.

Only the third arm carries a raw pointer:

- `TypedField { receiver: TypedObjectPtr, field_offset, kind }` —
  `TypedObjectPtr(pub *const TypedObjectStorage)`
  (`crates/shape-value/src/heap_value.rs:553`). `receiver` is a v2-raw struct
  pointer (HeapHeader at offset 0), retained via `v2_retain`
  (`reference.rs:74-77`, resolution `variables/mod.rs:2908-2912`).

**Consequence for the identity-map.** The classic "serialize a pointer as an
identity-handle, dedupe on restore" problem applies to **exactly one arm**
(`TypedField`). The other two arms already *are* identity-handles — symbolic
addresses into VM-state structures that the W17 deep-frame-stack / deep-module-
binding restore reconstructs in order. They serialize as their integer fields
verbatim; on restore they re-point at the reconstructed frame/binding by
**re-indexing**, not by pointer-dedupe. This is the cleanest possible reuse of
the SharedCell "serialize-with-shared-identity" idea: the identity *is* the
symbolic index, and the index survives the snapshot trivially.

This collapses the facet into two distinct mechanisms, not one:

| RefTarget arm   | Identity carrier            | Restore mechanism                        | Needs ptr identity-map? |
|-----------------|-----------------------------|------------------------------------------|-------------------------|
| `Local`         | `(frame_index, slot_index)` | re-index into restored `call_stack`      | **No** — symbolic       |
| `ModuleBinding` | `binding_idx`               | re-index into restored `module_bindings` | **No** — symbolic       |
| `TypedField`    | `receiver: *const TOS`      | dedupe via ptr identity-map              | **Yes**                 |

---

## 1. Ground truth: where references are opaque today

Today `HeapKind::Reference` round-trips as a content-free `ReferenceOpaque`
stub. Three call sites establish this:

1. **Wire arm** — `SerializableVMValue::ReferenceOpaque`
   (`crates/shape-runtime/src/snapshot.rs:507-512`), explicitly deferred:
   > "round-tripping requires tracking target identity across snapshot
   > boundaries which is unspecified by ADR-006 §2.7. The
   > W17-snapshot-references follow-up answers the identity question."

2. **Serialize projection** — `slot_heap_to_serializable`
   (`snapshot.rs:1104`): `HeapKind::Reference => Ok(SV::ReferenceOpaque)`. Note
   the comment at `:1100-1103`: "we don't even need to touch the Arc on the way
   out for the opaque round-trip path." This facet **changes that** — we now
   recover `Arc<RefTarget>` and project its arm.

3. **Deserialize projection** — `serializable_to_heap_slot`
   (`snapshot.rs:1223` onward) has no `(SV::ReferenceOpaque, HeapKind::Reference)`
   arm; it surfaces structured-error today.

The diagnostic name table also lists it: `serializable_arm_name`
(`snapshot.rs:1437`) and the resume-side mirror `arm_name_for_diag`
(`crates/shape-vm/src/executor/resume.rs:547`).

This facet is the **W17-snapshot-references follow-up** named at
`snapshot.rs:511`.

---

## 2. The SharedCell "serialize-with-shared-identity" mechanism we reuse

The task names the SharedCell decision as the reuse target. Ground truth check:
the SharedCell identity-table does **not exist in code yet** — it is the
**W17-snapshot-sharedcell follow-up** (`docs/adr/006-value-and-memory-model.md:5977`):

> **W17-snapshot-sharedcell** — SharedCell per-kind cell payload +
> binding-identity table.

The `SharedCellOpaque` wire arm (`snapshot.rs:522-529`) names the same
requirement in prose:

> "Round-tripping also bumps into the binding-identity question (two `var x`
> bindings that share a cell observe each other's mutations, so **cell identity
> must survive the snapshot**)."

So we are not *reusing* an existing identity-table — we are **establishing the
shared identity-table pattern** that both `Reference::TypedField` and the future
`SharedCell` follow-up will share. The pattern, distilled from the SharedCell
requirement, is:

> **serialize-with-shared-identity:** when N carriers point at one heap
> allocation, assign that allocation a stable *identity token* once, serialize
> each carrier as `{ token }`, and emit the referent's payload exactly once into
> an identity-keyed side-table. On restore, materialize each side-table entry
> exactly once into a real allocation, record `token → restored-ptr` in a
> dedupe map, and patch every carrier's `token` to the single restored pointer.

This is the **same shape** as `bincode`/serde's lack of pointer-graph support
worked around by an explicit object-table — and it is exactly what
`SharedCell` will need for "two `var x` bindings share a cell". By landing it
for `Reference::TypedField` first, with the identity-map keyed on the v2-raw
`*const TypedObjectStorage` and the referent payload routed through the
**existing** `TypedObject` snapshot arm (`SerializableVMValue::TypedObject`,
`snapshot.rs:343-348`), we reuse all of the typed-object serialization
machinery and add only the dedupe table.

**Constraint honored:** No new `ValueWord`-shape carrier; no `Bool`-default; the
identity token is a plain `u64` index into a `Vec`, not a tagged word. The
referent payload reuses `slot_to_serializable` (`snapshot.rs:843`) for the
object body. CLAUDE.md §Forbidden has nothing to refuse here — there is no
dynamic dispatch, no tag decode, no `Arc<HeapValue>` generic serializer (the
TypedObject arm is kind-threaded per §2.7.5.1).

---

## 3. Wire-format design

### 3.1 Replace `ReferenceOpaque` with a structured `Reference` arm

```rust
// crates/shape-runtime/src/snapshot.rs — replaces ReferenceOpaque (line 507-512)

/// `HeapKind::Reference` — `&expr` / `&mut expr` reference handle (Wave 8;
/// W17-snapshot-references, v0.3.3). The reference's *projected kind*
/// (`RefTarget::projected_kind`, reference.rs:105) and its target arm
/// round-trip; the referent itself is reached either symbolically (Local /
/// ModuleBinding re-index into the restored frame stack / module bindings) or
/// via the heap-identity side-table (TypedField — see `heap_referents` below).
Reference {
    /// `&mut` (true) vs `&` (false). Sourced at MakeRef time from the
    /// borrow-solver mode; see §3.4. `&mut` exclusivity is preserved across
    /// snapshot→resume because the *whole VM* moves as one unit (no
    /// concurrent live handle exists at the snapshot instant).
    is_mut: bool,
    /// Which RefTarget variant this handle projects.
    target: SerializableRefTarget,
},
```

```rust
// New sibling enum in snapshot.rs (next to EnumPayloadSnapshot, ~line 580)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializableRefTarget {
    /// Symbolic — re-index into restored `call_stack`. `frame_index ==
    /// u32::MAX` is the top-level (frameless) sentinel (variables/mod.rs:2522).
    Local {
        frame_index: u32,
        slot_index: u32,
        /// `NativeKind` ordinal of the projected slot (RefTarget.kind).
        /// Serialized as the kind's stable wire ordinal, NOT raw bits.
        projected_kind: SerializableNativeKind,
    },
    /// Symbolic — re-index into restored `module_bindings`.
    ModuleBinding {
        binding_idx: u32,
        projected_kind: SerializableNativeKind,
    },
    /// Heap-identity — `referent_token` indexes the snapshot's `heap_referents`
    /// side-table (§3.3). Multiple TypedField refs to the same object share a
    /// token; the restore dedupe map (§4.2) re-points them all at one object.
    TypedField {
        referent_token: u64,
        field_offset: u32,
        projected_kind: SerializableNativeKind,
    },
}
```

`SerializableNativeKind` is a thin serde-stable mirror of `NativeKind` (it must
not serialize as raw discriminant bits — `NativeKind::Ptr(HeapKind)` carries a
nested ordinal). A `From`/`TryFrom` pair lives next to it. (If a stable
`Serialize` for `NativeKind` already exists in `shape-value`, reuse it; grep
showed kind ordinals are already wire-stable for the §2.7.7 parallel-kind track,
so prefer that path and drop `SerializableNativeKind`.)

### 3.2 No `Box<HeapValue>` and no inline referent in the ref arm

The reference arm carries **no referent payload inline** — neither a
`Box<SerializableVMValue>` nor an `Arc`. This is deliberate and load-bearing:

- For `Local` / `ModuleBinding`, the referent already lives in the frame stack /
  module-binding vectors that W17 deep-restore reconstructs. Inlining a copy
  would *duplicate* the value and break the aliasing contract (a `&mut x`
  write must be visible through `x`). The symbolic index guarantees the ref and
  the binding observe the **same** restored slot.
- For `TypedField`, the referent is emitted once into the `heap_referents`
  side-table (§3.3), keyed by token. Inlining it per-ref would (a) duplicate
  shared objects, (b) reintroduce the `Arc<HeapValue>` generic-serializer shape
  §2.7.5.1 forbids.

This is precisely why `ReferenceOpaque` carried no payload — we keep that
property and add only the symbolic/token identity.

### 3.3 The heap-referent side-table (the SharedCell identity-table, instantiated)

`VmSnapshot` (`snapshot.rs:238-261`) gains one field:

```rust
// crates/shape-runtime/src/snapshot.rs — VmSnapshot, add after exception_handlers
    /// W17-snapshot-references / W17-snapshot-sharedcell shared identity-table.
    /// `heap_referents[token]` is the once-serialized payload of a heap object
    /// that one-or-more references (and, later, SharedCells) point at. Empty
    /// when the program holds no escaping references. Each entry is a full
    /// `SerializableVMValue` (in practice `TypedObject`), projected via the
    /// existing kind-threaded `slot_to_serializable` path (snapshot.rs:843).
    #[serde(default)]
    pub heap_referents: Vec<SerializableHeapReferent>,
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableHeapReferent {
    /// The heap kind of the referent (today: always TypedObject; the table is
    /// kind-tagged so the SharedCell follow-up can store cell payloads here).
    pub kind: SerializableHeapReferentKind,
    /// The once-serialized payload. For TypedObject this is the existing
    /// `SerializableVMValue::TypedObject { schema_id, slot_data, heap_mask }`.
    pub payload: SerializableVMValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializableHeapReferentKind {
    TypedObject,
    // SharedCell,  // reserved — W17-snapshot-sharedcell lands here
}
```

The token is the index into `heap_referents`. Identity = index. This is the
literal instantiation of "serialize-with-shared-identity": the dedupe happens
because *the serializer assigns one token per distinct `*const TypedObjectStorage`*
(§4.1), and every ref carrying that pointer serializes the same token.

### 3.4 `&mut` vs `&` mode metadata

`RefTarget` does **not** carry mutability today — it's erased at the carrier
level (all three arms are mode-agnostic; the borrow solver enforces `&mut`
exclusivity statically at B0001, `mir/solver.rs:1073-1079`, before any runtime
ref exists). So `is_mut` is not recoverable from `Arc<RefTarget>` alone at
serialize time.

Two options, surface both:

- **(a) Drop the mode from the wire format.** Since the whole VM moves as one
  unit and `&mut` exclusivity was already proven at compile time, the resumed VM
  re-executes from the same MIR with the same solver guarantees. The mode is not
  needed at runtime for deref correctness (`op_deref_load`/`op_deref_store`
  resolve via `projected_kind`, not mode). **Recommended** — it keeps the wire
  arm minimal and avoids inventing a mode field the carrier never had.
- **(b) Thread mode through `RefTarget`.** Add `is_mut: bool` to each arm,
  stamp it at MakeRef from the operand. Larger blast radius (touches
  construction at `variables/mod.rs:2541/2552/2652`, the resolution arms, and
  the `clone_with_kind`/`drop_with_kind` paths are unaffected since mode is
  inert). Only worth it if a future feature reads mode at runtime.

The wire arm above includes `is_mut` for forward-compat, but under option (a)
the serializer always writes `false` and the deserializer ignores it; the field
is reserved. **Open question O1** flags this for the borrow-solver facet to
ratify (it owns the escape→RC promotion that decides whether mode even survives
promotion).

---

## 4. Serialize path (VM → wire)

### 4.1 Identity assignment — the dedupe-on-write table

Snapshot serialization needs a per-snapshot mutable `IdentityWriter` threaded
through the slot projection. Today `slot_to_serializable` / `slot_heap_to_serializable`
take `(bits, kind)` and a `&SnapshotStore` (`snapshot.rs:843`, `:975`); they are
pure-ish (no graph state). We extend the heap path with an identity context:

```rust
// New, lives in snapshot.rs alongside slot_heap_to_serializable

#[derive(Default)]
pub struct IdentityWriter {
    /// original `*const TypedObjectStorage as u64` → assigned token.
    seen: std::collections::HashMap<u64, u64>,
    /// token-indexed referent payloads, emitted exactly once each.
    referents: Vec<SerializableHeapReferent>,
}

impl IdentityWriter {
    /// Assign (or reuse) a token for a referent pointer. Emits the payload on
    /// first sight only — this is the dedupe that makes N refs to one object
    /// share one token (multiple-refs-to-one-referent requirement).
    fn intern_typed_object(
        &mut self,
        ptr: u64,
        store: &SnapshotStore,
    ) -> Result<u64, String> {
        if let Some(&tok) = self.seen.get(&ptr) {
            return Ok(tok);                       // already emitted — dedupe
        }
        let tok = self.referents.len() as u64;
        // Reserve the slot BEFORE recursing, so a cycle (object → ref → same
        // object) finds the token already present and terminates. See §5.
        self.seen.insert(ptr, tok);
        self.referents.push(SerializableHeapReferent {
            kind: SerializableHeapReferentKind::TypedObject,
            payload: SerializableVMValue::Unit,   // placeholder; patched below
        });
        // Project the object body via the EXISTING TypedObject arm. This
        // reuses slot_to_serializable for each field slot (kind-threaded).
        let payload = slot_heap_to_serializable_with_identity(
            ptr,
            HeapKind::TypedObject,
            store,
            self,                                  // recursion carries the table
        )?;
        self.referents[tok as usize].payload = payload;
        Ok(tok)
    }
}
```

Then the Reference arm of the (identity-aware) heap projection:

```rust
// In slot_heap_to_serializable_with_identity — the Reference arm REPLACES the
// stub at snapshot.rs:1104 (HeapKind::Reference => Ok(SV::ReferenceOpaque)):

HeapKind::Reference => {
    // SAFETY: kind == Ptr(HeapKind::Reference) ⇒ bits = Arc::into_raw(
    // Arc<RefTarget>) per reference.rs:11-12. Recover, read, restore share.
    let arc = unsafe { Arc::<RefTarget>::from_raw(bits as *const RefTarget) };
    let result = serialize_ref_target(&arc, store, identity);
    let _ = Arc::into_raw(arc);                   // restore the original share
    result
}
```

```rust
fn serialize_ref_target(
    rt: &RefTarget,
    store: &SnapshotStore,
    identity: &mut IdentityWriter,
) -> Result<SerializableVMValue, String> {
    let target = match rt {
        RefTarget::Local { frame_index, slot_index, kind } =>
            SerializableRefTarget::Local {
                frame_index: *frame_index,        // u32::MAX sentinel preserved
                slot_index: *slot_index,
                projected_kind: serialize_kind(*kind),
            },
        RefTarget::ModuleBinding { binding_idx, kind } =>
            SerializableRefTarget::ModuleBinding {
                binding_idx: *binding_idx,
                projected_kind: serialize_kind(*kind),
            },
        RefTarget::TypedField { receiver, field_offset, kind } => {
            // The ONE arm that needs the identity-map. `receiver.0` is the
            // raw *const TypedObjectStorage (heap_value.rs:553).
            let ptr = receiver.0 as u64;
            let token = identity.intern_typed_object(ptr, store)?;
            SerializableRefTarget::TypedField {
                referent_token: token,
                field_offset: *field_offset,
                projected_kind: serialize_kind(*kind),
            }
        }
    };
    Ok(SerializableVMValue::Reference { is_mut: false, target }) // mode: §3.4(a)
}
```

**Sharing the table.** The `IdentityWriter` is constructed once per
`VmSnapshot` and threaded through *every* slot in stack / locals /
module_bindings / frame upvalues during `capture_vm_state`
(`vm_state_snapshot.rs:64`) and the top-level snapshot builder. Because the same
`*const TypedObjectStorage` interns to the same token regardless of which slot
referenced it, this is where **multiple refs to one referent** dedupe and where
a `TypedField` ref and the original binding holding that object both point at
one side-table entry. After the walk, `snapshot.heap_referents =
identity.referents`.

### 4.2 Why this is sound under the escape→RC thesis

The design thesis (borrow-solver facet) flips B0003-REJECT
(`mir/solver.rs:1146-1160`) to escape→RC-PROMOTE: a referent that is referenced
across an escape boundary is forced to an RC'd heap object (`TypedObjectStorage`
via the v2-raw `_new` path) with lifetime extended to cover the reference. By
the time a `RefTarget::TypedField` exists at snapshot time, its `receiver` is
**already** a live RC'd `TypedObjectStorage` (it has to be — `MakeFieldRef`
took a `v2_retain` share, `reference.rs:74-77`). So `intern_typed_object`
serializing the object body is always reading a live, owned allocation. No
dangling read. This is the snapshot facet leaning entirely on the borrow-solver
facet's promotion — we serialize what is already heap-resident.

---

## 5. Cycles, self-reference, multiple-refs

- **Multiple refs → one referent.** Solved by `IdentityWriter.seen`: the first
  ref interns and emits; subsequent refs hit the `if let Some(&tok)` fast path
  (§4.1) and reuse the token. On restore the dedupe map (§6.2) hands all of them
  the same restored pointer.

- **Cycles** (object A contains a ref to object B which contains a ref back to
  A). Handled by **reserve-token-before-recurse** (§4.1: `seen.insert` and the
  placeholder `push` happen *before* `slot_heap_to_serializable_with_identity`
  recurses into the body). When the recursion reaches the back-edge ref to A,
  `intern_typed_object(ptr_A)` finds the token already in `seen` and returns it
  without re-recursing. The placeholder payload at `referents[tok]` is patched
  in once the body finishes. Standard serde-object-table cycle break.

- **Self-reference** (object A contains a `TypedField` ref into its own field).
  Degenerate cycle; same reserve-before-recurse handles it — the self-ref interns
  to A's own token.

- **Local / ModuleBinding refs into a cyclic frame.** No cycle hazard: these are
  symbolic indices, not recursive payloads. They serialize as integers.

---

## 6. Deserialize / restore path (wire → VM)

### 6.1 Two-phase restore, ordering constraint

Restore MUST run in this order so symbolic and pointer identities resolve:

1. **Phase A — materialize `heap_referents`.** Walk `snapshot.heap_referents`;
   for each entry, reconstruct the `TypedObjectStorage` via the existing
   `serializable_to_slot` / TypedObject reconstruction path
   (`snapshot.rs:1174`, `serializable_to_heap_slot` `:1223`). Record
   `token → restored *const TypedObjectStorage` in a `RestoreIdentityMap`. This
   must precede any `TypedField` ref reconstruction. **Cycles:** allocate all
   referent objects first with their scalar fields, then in a second sub-pass
   patch the ref-typed fields (which point at tokens) — i.e. two-phase even
   within Phase A, because a ref field inside referent[i] may point at
   referent[j] with j > i. This is the standard "allocate then link" graph
   restore.

2. **Phase B — restore frame stack + module bindings (W17 deep-frame-stack).**
   This is where `RefTarget::Local` / `ModuleBinding` get their referents.
   Today this lands empty (`resume.rs:503-515` returns empty `call_stack` /
   `module_bindings`); the W17 deep-frame-stack facet fills it. **Integration
   contract:** Local/ModuleBinding refs are reconstructed *after* the frame
   stack and module bindings are populated, so re-indexing
   (`call_stack[frame_index].base_pointer + slot_index`) lands on real slots.

3. **Phase C — reconstruct Reference slots.** Each
   `SerializableVMValue::Reference { is_mut, target }` rebuilds an
   `Arc<RefTarget>` (§6.2) and pushes it with kind
   `NativeKind::Ptr(HeapKind::Reference)` via the slot ABI.

### 6.2 The restore dedupe map (re-pointing)

```rust
// snapshot-restore side (resume.rs or a new restore module)

#[derive(Default)]
struct RestoreIdentityMap {
    /// referent_token → restored *const TypedObjectStorage (as u64).
    /// One entry per heap_referents slot; populated in Phase A. Every
    /// TypedField ref with this token re-points HERE — the dedupe that makes
    /// N refs share one restored object.
    by_token: Vec<u64>,
}

fn restore_ref_target(
    target: &SerializableRefTarget,
    idmap: &RestoreIdentityMap,
) -> Result<RefTarget, String> {
    Ok(match target {
        SerializableRefTarget::Local { frame_index, slot_index, projected_kind } =>
            RefTarget::Local {
                frame_index: *frame_index,        // u32::MAX sentinel preserved
                slot_index: *slot_index,
                kind: restore_kind(projected_kind)?,
            },
        SerializableRefTarget::ModuleBinding { binding_idx, projected_kind } =>
            RefTarget::ModuleBinding {
                binding_idx: *binding_idx,
                kind: restore_kind(projected_kind)?,
            },
        SerializableRefTarget::TypedField { referent_token, field_offset, projected_kind } => {
            let ptr = *idmap.by_token.get(*referent_token as usize).ok_or_else(|| {
                format!("restore_ref_target: TypedField token {referent_token} \
                         out of bounds (heap_referents.len()={})", idmap.by_token.len())
            })?;
            // Bump the restored object's refcount — the new RefTarget owns one
            // share, mirroring the MakeFieldRef v2_retain (variables/mod.rs:2910).
            let tos = ptr as *const TypedObjectStorage;
            unsafe { shape_value::v2::refcount::v2_retain(&(*tos).header); }
            RefTarget::TypedField {
                receiver: TypedObjectPtr::new(tos),
                field_offset: *field_offset,
                kind: restore_kind(projected_kind)?,
            }
        }
    })
}
```

`restore_ref_target` is the inverse of `serialize_ref_target` and the place
where **re-pointing** happens: every `TypedField` ref carrying the same token
calls `idmap.by_token[token]` and gets the **one** restored pointer — they all
alias the same object, exactly as before the snapshot. Refcount discipline:
each restored ref does one `v2_retain` (matching the per-ref share the original
held); the side-table's own first materialization holds the base share.

### 6.3 Wire it into `serializable_to_heap_slot`

Add the missing inverse arm (today there is none for Reference;
`serializable_to_heap_slot` at `snapshot.rs:1223` falls through to surface):

```rust
(SV::Reference { is_mut: _, target }, HeapKind::Reference) => {
    // idmap must be threaded into serializable_to_heap_slot for this arm; see
    // §7 signature change. Phase A has already populated it.
    let rt = restore_ref_target(target, idmap)?;
    let arc = Arc::new(rt);
    let raw = Arc::into_raw(arc) as u64;
    Ok((raw, NativeKind::Ptr(HeapKind::Reference)))
}
```

---

## 7. Concrete change recipe

1. **`crates/shape-runtime/src/snapshot.rs`**
   - Replace `ReferenceOpaque` (`:507-512`) with the structured `Reference {
     is_mut, target }` arm (§3.1).
   - Add `SerializableRefTarget` enum (§3.1), `SerializableHeapReferent` +
     `SerializableHeapReferentKind` (§3.3). If a wire-stable `NativeKind` serde
     already exists, reuse it and drop `SerializableNativeKind`; else add it
     with `From`/`TryFrom`.
   - Add `heap_referents: Vec<SerializableHeapReferent>` to `VmSnapshot`
     (`:238-261`) with `#[serde(default)]` (back-compat: old snapshots restore
     with empty table).
   - Add `IdentityWriter` (§4.1) and a `slot_heap_to_serializable_with_identity`
     variant (or thread `Option<&mut IdentityWriter>` into the existing
     `slot_heap_to_serializable` — prefer a dedicated entry to avoid churning
     the §2.7.5.1 signature for non-ref kinds).
   - Add the `HeapKind::Reference` serialize arm (§4.1) and the
     `(SV::Reference, HeapKind::Reference)` deserialize arm (§6.3) — thread the
     `RestoreIdentityMap` into the inverse path.
   - Update `serializable_arm_name` (`:1437`) `SharedCellOpaque`-adjacent table:
     `Reference { .. } => "Reference"`.
   - Bump `SNAPSHOT_VERSION` (`:37`) v5 → v6; document the `heap_referents`
     addition. `#[serde(default)]` keeps v5 readable.

2. **`crates/shape-vm/src/executor/resume.rs` / `vm_state_snapshot.rs`**
   - Thread one `IdentityWriter` per snapshot through `capture_vm_state`
     (`vm_state_snapshot.rs:64`) and the top-level snapshot builder so all slot
     walks share the dedupe table; assign `snapshot.heap_referents` after.
   - Add Phase A/B/C ordering (§6.1) to the restore entry. The empty-restore
     stub (`resume.rs:503-515`) is replaced by the W17 deep-frame-stack facet;
     this facet adds Phase A (referent materialization) **before** it and Phase C
     (ref reconstruction) **after** it.
   - Update `arm_name_for_diag` (`resume.rs:547`): `Reference { .. } =>
     "Reference"`.

3. **No `shape-value` change required for `Local`/`ModuleBinding`.** They round-
   trip via their integer fields. `TypedField` needs only the existing
   `TypedObjectPtr::new` + `v2_retain` (`reference.rs`, `heap_value.rs:553`).

4. **Mode metadata (§3.4):** default to option (a) — `is_mut` reserved, always
   `false`. Defer to the borrow-solver facet's ruling (O1).

5. **Tests** (unit, `#[cfg(test)]` in `snapshot.rs` / `resume.rs`):
   - round-trip a `RefTarget::Local` ref → re-index lands on same slot value.
   - round-trip a `RefTarget::ModuleBinding` ref.
   - round-trip a `TypedField` ref → object body in `heap_referents`, restored
     ref derefs to same field.
   - **two refs to one object** → one `heap_referents` entry, both restored refs
     alias (write via one visible through the other).
   - **cycle** (A→ref→B→ref→A) → terminates, both restore.
   - back-compat: a v5 snapshot (no `heap_referents`) restores with empty table.

---

## 8. Boundaries / what this facet does NOT do

- **Does not** implement the W17 deep-frame-stack restore itself (`resume.rs:503-515`
  empty stub) — it depends on it for Local/ModuleBinding referents and defines
  the Phase-A-before / Phase-C-after ordering contract (§6.1).
- **Does not** land the cross-node live-coherence / move-on-send path — out of
  scope per the thesis (v0.4 live-distributed-sharing). Snapshot→resume moves the
  whole VM as one unit, so `&mut` exclusivity is preserved trivially (no
  concurrent live handle at the snapshot instant; §3.4).
- **Does not** serialize `RefTarget::TypedIndex` — that variant is deleted
  (`reference.rs:90-98`, V3-S5 ckpt-4); references into typed-array elements
  cascade-break until the per-element-kind receiver rebuild lands downstream.
  A `TypedField`-only ref serializer is complete for the current carrier.
- **Does not** implement `SharedCellOpaque` deep round-trip — but it
  **establishes the shared identity-table** (`heap_referents`, kind-tagged) that
  W17-snapshot-sharedcell plugs into (§3.3 reserved `SharedCell` kind).

---

## 9. CLAUDE.md §Forbidden compliance check

- No `ValueWord` / `tag_bits` / `synthesize_*` — the ref carrier is
  `Arc<RefTarget>` recovered by name (`reference.rs`), projected by arm.
- No `Arc<HeapValue>` generic serializer — the referent body routes through the
  existing kind-threaded `TypedObject` arm (`slot_to_serializable`,
  `snapshot.rs:843`).
- No `Bool`-default — kind mismatches surface structured errors (mirrors the
  existing §2.7.5.1 discipline at `snapshot.rs:900-906`, `:1208-1215`).
- No new modal-types subsystem; no `SlotKind::Dynamic`; no `Convert<X>To<Y>`
  opcode. The identity token is a plain `u64` index, not a tagged carrier.
- The parallel-implementation attractor (CLAUDE.md §Parallel-implementation)
  does not bite: there is exactly one identity-table, shared by Reference now
  and SharedCell later — not two carriers meeting at a structural-equivalence
  layer.

---

## 10. Open questions (for sibling facets / strategic owner)

- **O1 (mode metadata):** Should `&mut` vs `&` survive the wire format, or is
  the static B0001 exclusivity proof (`mir/solver.rs:1073-1079`) plus
  whole-VM-atomic-move sufficient to drop it (§3.4 option a)? Owned by the
  borrow-solver facet (escape→RC promotion may itself alter mode).
- **O2 (NativeKind wire form):** Is there an existing wire-stable `NativeKind`
  serde used by the §2.7.7 parallel-kind track that the frame stack already
  serializes? If yes, reuse it and drop `SerializableNativeKind`. Owned by the
  W17 deep-frame-stack facet (it must serialize kinds for the stack regardless).
- **O3 (referent body for non-TypedObject):** escape→RC promotion forces the
  referent to a heap object — is it *always* `TypedObjectStorage`, or can a
  promoted scalar/array referent land a different `HeapKind`? If so,
  `SerializableHeapReferentKind` needs more arms. Owned by the borrow-solver +
  storage-planning facets (they decide the promotion target carrier).
- **O4 (Phase-A cycle sub-pass):** confirm the W17 deep-frame-stack restore can
  expose an "allocate-then-link" hook so referent-internal ref fields patch
  after all referents exist (§6.1 Phase A). Owned by W17 deep-frame-stack facet.
