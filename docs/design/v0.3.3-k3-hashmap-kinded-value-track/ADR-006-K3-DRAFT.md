# ADR-006 K3 DRAFT — `HashMapData` kinded value-track (parallel `Vec<NativeKind>` over the values buffer)

**Status:** PROPOSAL for supervisor ratification. NOT yet folded into the
canonical `docs/adr/006-value-and-memory-model.md`. Ratification = the
supervisor copies the §2.7.25 body below (near-verbatim) into the canonical
ADR under the §2.7.24 Q25-family numbering and stamps the authority line.

**Proposed canonical home:** `docs/adr/006-value-and-memory-model.md`
§2.7.25 (next sibling after §2.7.24 Q25 typed-carrier-monomorphization
bundle), Q25-family numbering **Q25.F**.

**Scope:** v0.3.3. Unblocks the sole remaining keystone-surfaced
`HashMapStringHeapValue` arm + the `datetime_methods.rs:427` `v2_diff`
cascade + the snapshot non-string-value HashMap round-trip + JIT HashMap
FFI for polymorphic-value maps.

**Authored from worktree:** `shape-strict-flip-collection-dispatch`
(cumulative strict-flip @ `f2e364a0`). Preservation gates held at authoring
(`numeric_conversions` 104/0, smoke 5/5, `sf-NEW`=0). Doc-only change.

---

## 0. Reconciliation note for the supervisor (read first)

The dispatch prompt named "the **PRESERVED** Q25.B `HashMapValueBuf`". The
canonical ADR-006 text is more recent than that framing: **Q25.B's
`HashMapValueBuf` enum is SUPERSEDED** (§2.7.24 Q25.B SUPERSEDED, Wave 2
Agent C partial close, 2026-05-15). The live carrier today is **not**
`HashMapValueBuf`; it is the per-V monomorphized `HashMapData<V:
HashMapValueElem>` flat struct (`*mut TypedArray<V>` values pointer) reached
through the `HashMapKindedRef` enum carrier
(`crates/shape-value/src/heap_value.rs:1716`). `HashMapValueBuf` is a future
`grep -rn 'HashMapValueBuf::' crates/` close-gate deletion target, not a
preserved alternative.

This draft is written against the **actual current canonical state** (per-V
`HashMapData<V>` + `HashMapKindedRef`), not against the superseded
`HashMapValueBuf` shape. The relation to the *preserved-by-history-only*
Q25.B body is stated explicitly in §3 below so the supervisor is not misled
by the prompt's framing. What is genuinely *preserved* and load-bearing here
is the **per-V monomorphization discipline** Q25.B established and Q25.B
SUPERSEDED carried forward: per-value kind known at the carrier level, no
`Arc<HeapValue>` catch-all, no inline tag byte on the parent struct. The K3
value-track is the **complement** that handles the one shape that per-V
monomorphization cannot express — a single map whose values are
heterogeneous (`HashMap<string, T>` where `T` is the open polymorphic
`Arc<HeapValue>` set), without a forbidden Bool-default.

---

## §2.7.25 (proposed) — `HashMapData` kinded value-track — K3 polymorphic-value HashMap (Q25.F ruling)

K3 surface work (the strict-flip collection-dispatch keystone audits +
STAGE-K1 module-return projection) surfaced that exactly **one** value
shape has no typed-Arc-direct carrier at any of the four HashMap value
boundaries: a `HashMap<string, T>` whose values are the open polymorphic
`Arc<HeapValue>` set rather than one monomorphic value type. Three of the
four boundaries already round-trip the monomorphic cases through the
post-Q25.B-supersession `HashMapData<V>` + `HashMapKindedRef` carrier
(`I64` / `F64` / `Bool` / `Char` / `String` / `Decimal` / `TypedObject` /
`TraitObject` / `HashMap`-recursive arms). Only `HashMap<string, string>`
(the `String` arm) round-trips deep-restore today; every non-string
*homogeneous* value type has its own `HashMapKindedRef` arm, but a
*heterogeneous* / open-polymorphic value map has none.

### 2.7.25.1 The PROBLEM

The keystone-surfaced arm is `ConcreteReturn::HashMapStringHeapValue(Vec<(String,
Arc<HeapValue>)>)` (`crates/shape-runtime/src/typed_module_exports.rs:110`).
It is the sole `ConcreteReturn` arm with no typed-Arc-direct projection at
the module-return boundary (`project_concrete_return`,
`crates/shape-vm/src/executor/vm_impl/modules.rs:191`). The surfaced reason
is exact:

> `HashMapStringHeapValue` is K3 territory: the polymorphic-value HashMap
> needs the ADR-006 `HashMapData` kinded-value-track amendment (a parallel
> `Vec<NativeKind>` over the values) before it can carry `Arc<HeapValue>`
> payloads **without a Bool-default kind**.

The same gap was independently surfaced from the temporal cluster.
`DateTime.v2_diff` (`crates/shape-vm/src/executor/objects/datetime_methods.rs:421`)
returns a `HashMap<string, int>`, and surfaces at line 427:

> The diff result is a HashMap with int values, but `HashMapData` stores
> values as `Arc<HeapValue>` and `HeapValue` has no integer arm — packing
> the diff components as `HeapValue::BigInt` or `HeapValue::Decimal` would
> silently change their semantic type. Clean migration needs an ADR-006
> amendment giving `HashMapData` a kinded value buffer (parallel
> `Vec<NativeKind>` track per §2.7.7).

(Note: `v2_diff`'s `HashMap<string, int>` is *homogeneous* `int`; it can in
principle be served by a `HashMapKindedRef::I64` monomorphic arm — but its
producer constructs the map from `Arc<HeapValue>`-shaped intermediates, and
the clean cross-cluster fix is the same value-track that closes the
genuinely-polymorphic `HashMapStringHeapValue` case. Both are folded under
K3 so the carrier is built once.)

**Why a Bool-default is the forbidden temptation.** The carrier-level kind
in the `HashMapKindedRef` design lives on the *variant* (one variant per
homogeneous value type). A polymorphic map has no single variant kind to
name. The deleted-pattern move is to add a `HashMapKindedRef::HeapValue(Arc<HashMapData<Arc<HeapValue>>>)`
arm and stamp every slot's drop/clone dispatch with `NativeKind::Bool` (a
no-op Drop/Clone), relying on `Arc<HeapValue>`'s own Drop to balance. That
is precisely the §2.7.7 #9 / W-series rationalization ("the shim's apparent
leak-freeness is an accident of `Bool`'s no-op Drop/Clone, not WB2.4
retain-on-read"). It is refused on sight. The right fix is a per-value
parallel `Vec<NativeKind>` track that names each value's real kind, mirroring
the §2.7.7 stack track and the §2.7.8 cell-storage track.

### 2.7.25.2 The DESIGN

Introduce a **single new polymorphic value carrier** — a `HashMapData<V>`
specialization whose value element is the runtime-tier typed-Arc carrier and
whose per-element kind lives in a **parallel `Vec<NativeKind>` value-track**,
exactly mirroring the §2.7.7 stack-side `Vec<u64>` + `Vec<NativeKind>` pair
and the §2.7.8 cell-storage `Vec<u64>` + `Vec<NativeKind>` pair.

This is **additive**: every existing `HashMapKindedRef` monomorphic arm is
unchanged. The new arm handles only the heterogeneous / open-polymorphic
case the monomorphic arms cannot express.

```rust
// crates/shape-value/src/heap_value.rs — NEW polymorphic value carrier.
//
// Mirrors §2.7.7 stack track + §2.7.8 cell-storage track: a flat values
// buffer of raw 8-byte slots, paired 1:1 with a parallel kind track.
pub struct HashMapKindedValues {
    /// Keys are string-typed at landing (same as every monomorphic arm).
    pub keys: *mut TypedArray<*const StringObj>,

    /// Raw 8-byte value slots. For heap values, each is
    /// `Arc::into_raw(Arc<HeapValue>) as u64`; for inline scalars (int /
    /// number / bool / char), the raw bits. NEVER `Arc<HeapValue>` boxed
    /// per element — the slot is raw, the kind names it.
    pub values: *mut TypedArray<u64>,

    /// PARALLEL kind track — `values.len() == value_kinds.len()` at every
    /// observable boundary. Drives `clone_with_kind` / `drop_with_kind`
    /// per the §2.7.7 / §2.7.8 dispatch tables. NO Bool-default; every
    /// element's kind is concrete at insert time (the producing opcode /
    /// builder emitted it).
    pub value_kinds: *mut TypedArray<NativeKind>,

    /// FNV-1a bucket index — unchanged; operates on KEYS only.
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}

// HashMapKindedRef gains ONE new arm — the polymorphic carrier.
pub enum HashMapKindedRef {
    I64(Arc<HashMapData<i64>>),
    // ... all existing monomorphic arms UNCHANGED ...
    HashMap(Arc<HashMapData<HashMapKindedRef>>),

    /// NEW (K3 / Q25.F) — heterogeneous / open-polymorphic value map.
    /// The value buffer is raw `u64`; per-element kind lives in the
    /// parallel `value_kinds` track. The map-level `NativeKind` reported
    /// by `HashMapKindedRef::value_native_kind()` for THIS arm is the
    /// carrier label `Ptr(HeapKind::HashMap)` (the *map* is heap); the
    /// per-VALUE kinds are read element-wise from `value_kinds`, never
    /// fabricated.
    Polymorphic(Arc<HashMapKindedValues>),
}
```

**Constructor shape.** One constructor; takes `(key, raw_bits, kind)` per
insert and pushes all three buffers in lockstep. Retain-on-read of a value
goes through `clone_with_kind(bits, kind)` (the §2.7.7 helper). There is at
most **one** scalar accessor returning `(u64, NativeKind)` per the §2.7.6 /
Q8 carrier-API-bound rule — heap value dispatch is `slot.as_heap_value()`
on the recovered bits gated by the recovered kind, NOT a per-heap-variant
accessor:

```rust
impl HashMapKindedValues {
    /// Insert `(key, raw value bits, value kind)`. Pushes keys / values /
    /// value_kinds in lockstep. If a value bit-pattern is a heap pointer,
    /// the caller has already transferred one share (`Arc::into_raw`);
    /// this takes ownership of that single share per the insert contract.
    pub unsafe fn insert(&mut self, key: &str, value_bits: u64, kind: NativeKind);

    /// Read element `i` as a runtime-tier carrier, bumping the heap
    /// refcount per WB2.4 (`clone_with_kind(bits, kind)`).
    pub fn value_owned(&self, i: usize) -> KindedSlot {
        let bits = unsafe { *(*self.values).data.add(i) };
        let kind = unsafe { *(*self.value_kinds).data.add(i) };
        clone_with_kind(bits, kind);          // §2.7.7 helper — Arc bump iff heap
        KindedSlot::new(ValueSlot::from_raw(bits), kind)
    }
}
```

**Drop discipline.** `HashMapKindedValues` carries a manual `Drop` (the
`HashMapValueElem`-equivalent release path for the raw value buffer): for
each element, `drop_with_kind(values[i], value_kinds[i])` — the same
§2.7.7 / §2.7.8 helper, never a bare `Arc::from_raw` guess and never a
"drop only if heap-shaped" probe. Keys release exactly as the monomorphic
`String`-keyed arms already do.

**Index invariant.** `values.len() == value_kinds.len()` at every
observable boundary (insert/remove return, snapshot serialize entry, FFI
boundary). A mismatch is a bug, not a recoverable state — `debug_assert_eq!`
on debug builds, mirroring §2.7.7's stack-track cross-check.

### 2.7.25.3 Relation to the per-V monomorphization carrier (the Q25.B-SUPERSEDED-preserved discipline)

The K3 value-track does **not** replace `HashMapKindedRef`'s monomorphic
arms and does **not** resurrect `HashMapValueBuf`. The two coexist with a
**compile-time classification rule** that selects between them (satisfying
the §Forbidden "explicit ADR amendment naming the duality + a compile-time
classification rule" bar — this is NOT parallel-implementation dressed as a
feature):

- **Homogeneous value type proven at compile time** → the existing
  monomorphic `HashMapKindedRef::<V>` arm (`*mut TypedArray<V>`, kind on the
  variant). This is the Q25.B-SUPERSEDED per-V monomorphization path,
  unchanged. Zero new cost; zero per-element kind byte.
- **Heterogeneous / open-polymorphic value type** (`Arc<HeapValue>` set, no
  single proven `V`) → the new `HashMapKindedRef::Polymorphic` arm
  (raw `u64` values + parallel `value_kinds` track).

The classification key is the **same `prove_native_kind()` proof** the rest
of the compiler uses (`compiler/type_tracking.rs`): if every value position
proves to one concrete `NativeKind`, the monomorphic arm is selected at
build time; otherwise the polymorphic arm. The decision is made by the
producer (builder / opcode / marshal projection), not by a runtime probe.
There is **no inline runtime discriminator byte on the parent
`HashMapData`** (refused per §2.7.24 Q25.B SUPERSEDED forbidden #3 — the
deleted UnifiedArray ELEMENT_KIND byte family).

The K3 track is the **value-side** analogue of the relation §2.7.9 drew for
`HeapKind::FilterExpr`: a label/track that names the real per-element
destructor so `clone_with_kind` / `drop_with_kind` dispatch to the correct
`Arc::increment/decrement_strong_count::<T>`. A wrong-kind value-track entry
is UB (wrong destructor walks the wrong fields), not a recoverable kind
error — which is exactly why the kind must be concrete per element and never
Bool-defaulted.

### 2.7.25.4 FORBIDDEN shapes this rules out

Mirror of §2.7.7 / §2.7.8 / §2.7.24 Q25.B-SUPERSEDED forbidden lists,
applied to the HashMap value-track:

1. **`NativeKind::Bool`-default for heap values.** The keystone temptation.
   A `HashMapKindedRef::HeapValue(Arc<HashMapData<Arc<HeapValue>>>)` arm
   stamping every slot drop/clone with `Bool` (no-op Drop/Clone) is the
   §2.7.7 #9 / W-series rationalization. Refused on sight. Every value's
   kind is concrete at insert time; surface (`NotImplemented(SURFACE)`) any
   producer site that cannot supply the kind, do not fabricate it.
2. **`Vec<KindedSlot>` for the value track.** Same §2.7.5 rule as the stack
   and cell stores: `KindedSlot` is the runtime-tier read-boundary carrier,
   not the storage-tier shape. The value buffer stays raw `u64` (`*mut
   TypedArray<u64>`) paired with a separate `*mut TypedArray<NativeKind>`
   track; consumers construct a `KindedSlot` at `value_owned`.
3. **16-byte value slots** (`TypedArray<{ bits: u64, kind: NativeKind }>`
   packed) — conflicts with §2.1's 8-byte slot invariant and doubles the
   buffer. Two parallel arrays, not one packed array.
4. **Packed tag bits in the value `u64`** — reintroduces the deleted
   ValueWord `tag_bits` dispatch (CLAUDE.md "Forbidden code"). The kind
   lives in the parallel track, never in the value bits.
5. **`Vec<Option<NativeKind>>` / `NativeKind::Unknown` / `_::Dynamic` /
   `_::Pending` in the value track** — value contents are post-proof per the
   §2.7.5.1 rule; every inserted value carries a known kind by construction.
6. **`Arc<HeapValue>` boxed per element** (`*mut TypedArray<Arc<HeapValue>>`
   or a `Box<HeapValue>` wrapper) — forbidden by ADR-006 §2.3 / §Forbidden;
   the value is raw bits + a kind that names it, not a re-boxed
   discriminator. This is also the §2.7.24 Q25.B-SUPERSEDED "no
   `Arc<HeapValue>` catch-all" rule.
7. **Resurrection of `HashMapValueBuf` / `Arc<TypedArrayData>`-style enum
   carriers** under any rename ("documented intentional duality between the
   polymorphic value-track and a value-buffer enum"). The §2.7.24 Q25.A /
   Q25.B SUPERSEDED deletion targets stay deleted; the K3 track is a new
   value-side parallel-kind track, not a re-tagged value-buffer enum.
8. **Inline HashMap-wide runtime kind discriminator byte on the parent
   `HashMapData` / `HashMapKindedValues`** naming the value type — §2.7.24
   Q25.B-SUPERSEDED forbidden #3 (the deleted UnifiedArray ELEMENT_KIND byte
   family). The per-element kind lives in the parallel track; the
   monomorphic-vs-polymorphic carrier selection is a compile-time arm
   choice, not a runtime byte.
9. **Per-routing-arm bridge / probe / helper / hop / translator / adapter /
   shim** framings for the value-track read/write path (CLAUDE.md
   broader-family regex). The read boundary is `value_owned` →
   `clone_with_kind`; the write boundary is `insert` → lockstep push. No
   intermediary "value-track decode bridge".

### 2.7.25.5 TOUCHPOINTS this unblocks

The value-track lands once in `shape-value`; the four surfaced boundaries
then build/consume it:

1. **`project_concrete_return::HashMapStringHeapValue`**
   (`crates/shape-vm/src/executor/vm_impl/modules.rs:191`, surfaced
   `other =>` arm). The K3 builder constructs a
   `HashMapKindedRef::Polymorphic(Arc<HashMapKindedValues>)` from the
   `Vec<(String, Arc<HeapValue>)>` payload — each value's `NativeKind`
   read from its `HeapValue::kind()` (concrete, never Bool-defaulted) —
   and returns `KindedSlot::from_hashmap(kref)`. The surfaced
   `NotImplemented` is replaced by the real projection. This is the **sole
   keystone arm** still surfaced at this boundary; K3 closes it.

2. **`DateTime.v2_diff`** (`datetime_methods.rs:421`, surfaced at line 427).
   The `HashMap<string, int>` result is constructed through the K3 builder
   (each `int` value's kind = `NativeKind::Int64`), or — once the producer
   is migrated to emit a proven-homogeneous `int` map — the monomorphic
   `HashMapKindedRef::I64` arm per the §2.7.25.3 classification. Either way
   the line-427 surface is removed; the in-code flag explicitly names "a
   parallel `Vec<NativeKind>` track per §2.7.7" as the clean fix, which this
   ratifies.

3. **Snapshot serialize/restore non-string-value HashMap arm.**
   - Serialize: `slot_to_serializable`
     (`crates/shape-runtime/src/snapshot.rs:1468`, the
     `HashMapKindedRef::String` match + `other_v =>` surface at
     line 1497) gains a `HashMapKindedRef::Polymorphic` arm that walks
     `value_owned(i)` and projects each value to its `SerializableVMValue`
     per the per-element kind. The "only `HashMap<string,string>`
     round-trips" surface lifts to "`HashMap<string, T>` round-trips for
     every K3-serializable `T`".
   - Restore: `serializable_to_slot`
     (`crates/shape-runtime/src/snapshot.rs:2145`, the
     `(SV::HashMap, HeapKind::HashMap)` arm + its non-String surface at
     line 2164) gains the inverse: rebuild a
     `HashMapKindedValues` from the serialized `(key, value)` pairs,
     stamping each value's kind from its `SerializableVMValue`
     discriminator (`expected_kind_from_serializable`,
     `crates/shape-vm/src/executor/snapshot.rs:670` — the existing
     discriminator→kind map, NOT a Bool fallback for value slots).

4. **JIT HashMap FFI for polymorphic-value maps.** The §2.7.5.B Family-3
   `jit_print_hashmap` / `format_hashmap` per-V dispatch
   (`printing.rs`, the `HashMapKindedRef` variant tag) gains the
   `Polymorphic` arm: it iterates `value_owned(i)` and delegates each value
   to `print_kinded_inner` by its per-element kind. **No new `NativeKind`
   per-V cardinality at the FFI boundary** — the outer carrier label stays
   `NativeKind::Ptr(HeapKind::HashMap)` (Q8 carrier-API-bound discipline,
   per §2.7.5.B's existing HashMap-carrier note). The map-construction /
   map-access JIT FFI carriers (`crates/shape-jit/src/ffi/v2/typed_map.rs`,
   `mir_compiler/v2_typed_map.rs`) thread the `Polymorphic` arm through the
   same per-V dispatch shape already used for the monomorphic arms.

### 2.7.25.6 CLOSE-GATE

The amendment is closed when ALL of:

1. **`HashMap<string, T>` round-trips deep-restore with VM == JIT** for the
   K3-serializable `T` set (at minimum `int` / `number` / `bool` / `string`
   / heap-`Arc<HeapValue>` mixed). The snapshot round-trip test
   (`crates/shape-vm/src/executor/snapshot.rs:1469`, currently the
   `HashMap<string,string>`-only `(3)` fixture) extends to a polymorphic /
   heterogeneous-value fixture; `from_snapshot → re-snapshot` yields
   identical per-element kinds AND values. Differential VM-vs-JIT execution
   of the same `HashMap<string, T>` program is RESULTS-IDENTICAL.

2. **4-table HeapKind lockstep holds.** The value-track does NOT add a
   `HeapKind` variant (the map is still `HeapKind::HashMap`); the
   `clone_with_kind` / `drop_with_kind` / `KindedSlot::{clone,drop}` /
   `TypedObjectStorage::drop` / `SharedCell::drop` dispatch tables are
   *reused* for per-element value dispatch — no new dispatch surface. The
   `verify-merge` 4-table lockstep + HeapKind-ordinal-collision checks pass
   unchanged.

3. **`just verify-merge` / `bash scripts/verify-merge.sh` green** (11
   checks, exit-code-based): no Bool-default for value slots, no
   `Vec<KindedSlot>` value buffer, no packed-tag value bits, no resurrected
   `HashMapValueBuf` / `Arc<TypedArrayData>` value carrier, no broader-family
   bridge/probe/shim descriptor on the value-track read/write path.

4. **`just check-no-dynamic` + the `no_dynamic.rs` sentinel green** — the
   value-track introduces no forbidden symbol.

5. **Preservation gates HELD**: `numeric_conversions` 104/0, smoke 5/5,
   `sf-NEW`=0 unchanged across the implementing wave.

6. **The four §2.7.25.5 surfaced sites no longer return
   `NotImplemented(SURFACE)`** for the K3 value shape:
   `modules.rs:191` (HashMapStringHeapValue), `datetime_methods.rs:427`
   (v2_diff), `snapshot.rs:1497` (serialize `other_v`), `snapshot.rs:2164`
   (restore non-String). Each surfaces only genuinely-unsupported shapes
   after K3, not the polymorphic-value HashMap.

---

## Appendix A — current-state anchors (for the implementing agent)

| Concept | Location (worktree `shape-strict-flip-collection-dispatch`) |
|---|---|
| `HashMapData<V: HashMapValueElem>` (current per-V carrier) | `crates/shape-value/src/heap_value.rs:1199` |
| `HashMapKindedRef` enum (current arms) | `crates/shape-value/src/heap_value.rs:1716` |
| `HashMapKindedRef::value_native_kind()` (carrier→NativeKind) | `crates/shape-value/src/heap_value.rs:1816` |
| `HashMapValueElem` recursive-`HashMapKindedRef` impl | `crates/shape-value/src/heap_value.rs:1119` |
| `ConcreteReturn::HashMapStringHeapValue` | `crates/shape-runtime/src/typed_module_exports.rs:110` |
| `project_concrete_return` (keystone surface) | `crates/shape-vm/src/executor/vm_impl/modules.rs:191` |
| `DateTime.v2_diff` surfaced flag | `crates/shape-vm/src/executor/objects/datetime_methods.rs:421-428` |
| snapshot serialize HashMap arm | `crates/shape-runtime/src/snapshot.rs:1468-1504` |
| snapshot restore HashMap arm | `crates/shape-runtime/src/snapshot.rs:2145-2184` |
| `expected_kind_from_serializable` (discriminator→kind) | `crates/shape-vm/src/executor/snapshot.rs:670` |
| snapshot round-trip container-arm test (HashMap fixture) | `crates/shape-vm/src/executor/snapshot.rs:1469-1548` |
| `clone_with_kind` / `drop_with_kind` (§2.7.7 helpers) | `crates/shape-vm/src/executor/vm_impl/stack.rs` |
| JIT HashMap FFI carriers | `crates/shape-jit/src/ffi/v2/typed_map.rs`, `mir_compiler/v2_typed_map.rs` |

## Appendix B — canonical ADR sections this draft mirrors

- §2.7.7 Stack ABI kind-awareness — parallel `Vec<NativeKind>` (Q9). Source
  pattern for the value-track shape, forbidden-list, perf characteristics,
  debug cross-check.
- §2.7.8 Cell-storage kind-awareness — parallel `Vec<NativeKind>` extended
  to cells (Q10). Source pattern for "extend the parallel-kind invariant to
  a new storage struct"; `Option<u64>`+`Option<NativeKind>` 1:1 pairing
  rule; `NotImplemented(SURFACE)` refusal shape.
- §2.7.9 `HeapKind::FilterExpr` (Q8 cardinality amendment). Source for "the
  kind label IS the destructor selector; a wrong label is UB, not a
  recoverable kind error" — applied per-VALUE-element here.
- §2.7.24 Q25.B SUPERSEDED — `HashMapValueBuf` deletion + the
  `HashMapData<V>` / `HashMapKindedRef` per-V monomorphization replacement.
  The discipline K3 complements (homogeneous → monomorphic arm; heterogeneous
  → K3 value-track) and the forbidden-list (#3 inline ELEMENT_KIND byte) K3
  inherits.
