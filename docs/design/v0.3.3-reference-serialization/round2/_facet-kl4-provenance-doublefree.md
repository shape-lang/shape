# Round 2 facet — KL-4 provenance / double-free (container-escape B0004)

> Scope: the broad-flip extension to round 1. Round 1 flipped **ReturnSlot +
> ModuleBindingStore** escapes to `escape→RC-PROMOTE` with the heap-owning
> `RefTarget::PromotedCell { cell: Arc<SharedCell> }` carrier (round-1 `DESIGN.md`
> §3). It explicitly **left B0004 container/field escapes REJECTED** (round-1
> §6 KL-2). This facet designs the provenance-safe scheme that would let
> **container escapes** (a `&local` stored into an array or object that outlives
> the referent's scope — `LoanSinkKind::ArrayStore`/`ObjectStore` +
> `ArrayAssignment`/`ObjectAssignment` + `EnumStore`) flip to promote, **without**
> a raw-pointer-token double-free.
>
> Every claim below cites source at workspace HEAD (`main`, `67768f17`).

---

## VERDICT (read first)

**SOUND for the OBJECT/ENUM container case — by construction, today, no new
machinery.** A reference stored into a `TypedObject` field (or enum struct
payload) is carried as a real owning `Arc<SharedCell>` (or `Arc<RefTarget>`)
share in the field slot, and `TypedObjectStorage::drop_fields`
(`crates/shape-value/src/heap_value.rs:3670-3916`) **already has the
`HeapKind::Reference` arm** (`:3852-3856`) and the **`HeapKind::SharedCell` arm**
(`:3893-3897`), each retiring exactly one `Arc::decrement_strong_count` share.
The container holds a real owning share with correct allocator provenance (it is
an `Arc<T>` allocation reclaimed by `Arc::decrement_strong_count::<T>`). There is
**no raw-pointer token** anywhere on this path; the field slot bits ARE the
`Arc::into_raw` pointer, dropped through the same `Arc` allocator that produced
them. Double-free is **proven impossible** for the object case (§3 below).

**NOT SOUND for the ARRAY container case as-is — a real structural gap, not a
fear.** A reference stored into an `Array<&T>` would have to monomorphize to a
`TypedArray<*const T>` whose per-element drop runs through
`TypedArray::<*const T>::drop_array_heap` (`typed_array.rs:296-324`), which
requires `T: HeapElement` (`v2/heap_element.rs:69-79`). `HeapElement` is
**structurally constrained to `#[repr(C)]` types with `HeapHeader` at offset 0**
and **explicitly forbids implementing it for `Arc<>`-wrapped storage**
(`heap_element.rs:41-45`). `RefTarget` and `SharedCell` are `Arc<T>`-wrapped (NOT
`HeapHeader`-at-offset-0 carriers — `reference.rs:11-16`,
`closure_layout.rs:130-159`), so they **cannot** be `TypedArray` heap-elements,
and there is **no `ELEM_TYPE_REFERENCE` / `ELEM_TYPE_SHARED_CELL` discriminant**
in the element-type table (`typed_array.rs:344-374`;
`v2_array_detect.rs:60-115`). The array path therefore has no monomorphization
that retires a per-element `Arc` share — the only landing today is the
unstamped-discriminant default arm (`typed_array.rs:457-467`), which **leaks the
element buffer** (chosen as "strictly preferable to a misaligned dealloc /
use-after-free"). A leak is not a double-free, but it is also not the sound
escape→RC promotion the broad flip requires.

**Bottom line:** the object/enum half of B0004 is provenance-safe and ship-able
in v0.3.3. The array half needs **one new `HeapElement`-shaped carrier for
reference elements** (a `RefCellElem` v2-raw HeapHeader wrapper — §4) which is
net-new XL work and cannot piggyback on the round-1 carrier. **Recommendation:
flip object/enum-container escapes in v0.3.3 (sound); keep array-container
escapes REJECTED (B0004) as a sharpened KL-4-array tripwire, deferred to v0.3.4
behind the §4 carrier.** This split keeps the no-known-incorrectness bar: nothing
ships that can double-free.

---

## 1. The KL-4 hazard restated precisely (what a naive flip would do wrong)

Round 1's `DESIGN.md` §6 KL-2 + the snapshot-identity adversarial review BREAK 4
(`adversarial/snapshot-identity-correctness.md:144-192`) name the double-free
class. Re-stated for the container case:

The **wrong** scheme (the one to refuse) is: store the reference into the
container as a **raw pointer token** — i.e. the container slot holds
`receiver.0 as u64` (a process-local `*const TypedObjectStorage`, the BREAK 4
`referent_token` shape) — and reconstruct the share with a "one `v2_retain` on
`by_token[token]`" on restore. That is unsound in two ways the review proves:

- **(a) double-serialize aliasing break** (BREAK 4a,
  `snapshot-identity-correctness.md:161-178`): the same object is serialized both
  as the container's token entry AND via the ordinary value-copy `TypedObject`
  snapshot arm, producing two restored copies; mutation through one is invisible
  to the other.
- **(b) allocator-provenance double-free** (BREAK 4b, `:180-192`): a token
  reconstructed via `Arc::new(...)` + `Arc::into_raw` flowing through a v2-raw
  `_new`-path `drop_with_kind` release is the cluster-1.5 / W5 SIGABRT class
  (CLAUDE.md Known Constraints; `vm_state_snapshot.rs:295`) — two different
  allocators claim the same bytes.

The provenance-safe scheme below **never produces a raw-pointer token**. The
container slot holds a real `Arc::into_raw` pointer that the container's own
per-element/per-field drop reclaims through the matching `Arc::decrement_strong_count`.
The "token" never exists at runtime; the slot bits ARE the owning share.

---

## 2. The carrier (reused from round 1, unchanged)

Per the hard constraint: reuse the round-1 escape→RC machinery and the
`PromotedCell { cell: Arc<SharedCell> }` carrier. No new `RefTarget` variant for
this facet.

- Round-1 carrier (to be added to `crates/shape-value/src/reference.rs:41-99`):
  `RefTarget::PromotedCell { cell: Arc<SharedCell> }` (round-1 `DESIGN.md`
  §3.1 / §7 item 1). The reference value at the slot tier is
  `Arc::into_raw(Arc<RefTarget>) as u64` with kind
  `NativeKind::Ptr(HeapKind::Reference)` (`reference.rs:11-16`). The inner
  `PromotedCell` owns one `Arc<SharedCell>` share of the promoted referent.
- The referent cell is the **same** `Arc<SharedCell>` the closure-capture /
  `op_alloc_shared_local` path produces (`variables/mod.rs:1459-1535`,
  `closure_layout.rs:130-321`); it carries its own §2.7.8/Q10 `NativeKind`
  companion (`closure_layout.rs:152`), and its `Drop` (`closure_layout.rs:340-...`)
  retires the inner value share when the last `Arc<SharedCell>` is released.

The **identity-map** for snapshot is the round-1 single-source `heap_referents`
`SharedCell` side-table (round-1 §3.5, §4.1–4.3). The container facet adds **no
second table** — that is the CLAUDE.md §Parallel-implementation discharge: the
container slot points at a `SharedCell` already interned in `heap_referents` by
the round-1 path; the container is just another live holder of an `Arc<SharedCell>`
share, exactly like the `PromotedCell` reference itself.

---

## 3. OBJECT / ENUM container case — PROVENANCE-SAFE BY CONSTRUCTION

### 3.1 How the field slot carries the Arc share (refcount accounting)

When `module_obj.field = &local` (or a struct/enum-payload store) escapes, the
compiler promotes `local` to a `SharedCell` (round-1 §3) and stores into the
field slot **the reference value's `Arc::into_raw(Arc<RefTarget>)` pointer** with
kind `NativeKind::Ptr(HeapKind::Reference)` (equivalently, where the design
prefers to store the cell directly, `Arc::into_raw(Arc<SharedCell>)` with
`Ptr(HeapKind::SharedCell)` — both arms exist, see §3.2). This goes into
`TypedObjectStorage.slots[i]` (`heap_value.rs:3507`) with the matching
`heap_mask` bit set (`:3510`) and `field_kinds[i]` = the heap kind
(`:3516`).

Refcount accounting at the store, per the cluster-1.5 explicit-per-owner-share
discipline (round-1 §3.2; `vm_state_snapshot.rs:295`):

- The ObjectStore handler does **one** `clone_with_kind(bits, kind)`
  (`stack.rs:54`) — `HeapKind::Reference` arm bumps one `Arc<RefTarget>` share
  (`stack.rs:281-283`); `HeapKind::SharedCell` arm bumps one `Arc<SharedCell>`
  share (`stack.rs:376-380`) — **before** writing the pointer into the field
  slot. The field now owns exactly one share.
- Every live holder owns its own explicit share: the producing stack slot owns
  one, the field owns one. Each is retired exactly once by its own drop path
  (stack via `truncate_stack`→`drop_with_kind`, `stack.rs` `:925-938` + the
  `:640`/`:88` arms; field via `drop_fields`, below).

### 3.2 How `drop_with_kind` on the container decrements correctly (NO double-free)

The object container drop is `TypedObjectStorage::drop_fields`
(`heap_value.rs:3670-3916`), reached from the object's own retirement
(`Arc::decrement_strong_count::<TypedObjectStorage>` at refcount 0, or the v2-raw
`_drop` walk at `:2987` / `:3637`). It walks `heap_mask` and dispatches per-field
on `field_kinds[i]`. **Both arms the container case needs already exist:**

```
heap_value.rs:3852   HeapKind::Reference => {
heap_value.rs:3853       std::sync::Arc::decrement_strong_count(
heap_value.rs:3854           bits as *const crate::reference::RefTarget,
heap_value.rs:3855       );
heap_value.rs:3856   }
...
heap_value.rs:3893   HeapKind::SharedCell => {
heap_value.rs:3894       std::sync::Arc::decrement_strong_count(
heap_value.rs:3895           bits as *const crate::v2::closure_layout::SharedCell,
heap_value.rs:3896       );
heap_value.rs:3897   }
```

**Double-free proof for the object case.** The field slot bits are
`Arc::into_raw(Arc<T>)` for `T ∈ {RefTarget, SharedCell}` (the construction-side
contract at `heap_value.rs:3525-3529`: "for every set heap_mask bit, the slot's
`u64` must be the raw pointer of an `Arc::into_raw::<T>` for the matching `T`").
The drop reclaims **exactly one** share via `Arc::decrement_strong_count::<T>`
(`:3853`/`:3894`) — the same `Arc` allocator that produced the pointer. This is
*allocator-symmetric*: `Arc::into_raw` ⇄ `Arc::decrement_strong_count` on the
identical `T`. There is no `_new`/`Arc::new` allocator mismatch (the BREAK 4b
class), because no v2-raw `_new` path is involved — `RefTarget` and `SharedCell`
are pure `Arc<T>` allocations, never HeapHeader carriers. The cell's underlying
payload (the referent value) is retired exactly once by the `SharedCell::Drop`
(`closure_layout.rs:340-...`) when the **last** `Arc<SharedCell>` share across ALL
holders (stack slot + `PromotedCell` ref + this field) is released. Each holder
contributed exactly one `clone_with_kind`/`Arc::clone` and retires exactly one
decrement. Net refcount balances to zero with no holder decrementing twice ⇒ **no
double-free, no leak.** ∎

This is the **identical** dispatch shape already proven for every other heap-kind
field (String, TypedObject, HashMap, FilterExpr, ...). Reference and SharedCell
are not special; they are full members of the 4-table lockstep
(`stack.rs` clone `:281`/`:376`, `stack.rs` drop `:640`/`:88`, `heap_value.rs`
drop_fields `:3852`/`:3893`, plus `KindedSlot` / `closure_layout` clone arms).

### 3.3 Why no raw-pointer-token path is reachable

The object case never interns a token. The field holds the `Arc::into_raw`
pointer directly; "identity" is the heap address of the `SharedCell`, which is
already the round-1 `heap_referents` key (§2). On snapshot serialization the field
is walked by the ordinary `TypedObject` serialize arm, but instead of value-copying
a `Ptr(HeapKind::SharedCell)` / `Ptr(HeapKind::Reference)` field, the serializer
**emits a `heap_referents` token referencing the same `SharedCell` entry** the
`PromotedCell` reference uses (round-1 §4.1–4.3, single-source table). On restore
the field re-acquires **one** `Arc<SharedCell>` share of the **same** restored cell
(allocate-all-then-link, round-1 §4.2). N container slots + M references to the
same cell → same token → same restored cell → aliasing preserved; each restored
holder does one share-acquire matched by one drop. This is the BREAK 4a fix:
**one** identity source, deduped, so the object cannot be serialized twice.

### 3.4 Compile-side flip for the object/enum sinks

Mirror the round-1 ReturnSlot/ModuleBindingStore flip at the object/enum sinks in
`mir/solver.rs`:

- `LoanSinkKind::ObjectStore | ObjectAssignment` arm (`solver.rs:1197-1200`):
  currently emits `BorrowErrorKind::ReferenceStoredInObject` (B0004). Replace the
  terminal diagnostic with a **promotion directive** (same shape as round-1
  §5: rewrite the ref's `RefTarget` → `PromotedCell`, force the referent to
  `SharedCell`). Loan generation (`solver.rs:185-241`, the
  `ObjectStore`/`ObjectAssignment` loan pushes at `:308`/`:418-431`) is **invisible
  to promotion** — the loan is still issued, B0001 still detected; only the
  escape-sink diagnostic is replaced (round-1 §5 walk-back hazard / N2 sentinel
  applies verbatim).
- `LoanSinkKind::EnumStore` arm (`solver.rs:1201-1202`): same flip (enum struct
  payloads are `TypedObjectStorage`-shaped at runtime, so `drop_fields` covers
  them by the same §3.2 proof).
- The `sink_is_local` exemptions (`solver.rs:1197`/`:1201`) stay — a non-escaping
  store (`EscapeStatus::Local`) was never an error and is not promoted.

`detect_escape_status` (`storage_planning.rs:1014-1031`) + the escape→RC Rule 3b
(`storage_planning.rs:956-959`) already promote escaped+aliased+mutated bindings
to `SharedCow`; the object-container referent reuses that exact path (round-1
escape-rc facet §2.2).

---

## 4. ARRAY container case — the structural gap + the carrier that closes it

### 4.1 Why the object proof does NOT carry over to arrays

The object case is sound because `TypedObjectStorage` drops per-field via a
`NativeKind`-dispatched `Arc::decrement_strong_count` table that already has
Reference/SharedCell arms (§3.2). Arrays have **no equivalent**:

- `Array<&T>` would monomorphize to `TypedArray<*const E>` for some element
  carrier `E`. Per-element drop is `TypedArray::<*const E>::drop_array_heap`
  (`typed_array.rs:296-324`), which calls `E::release_elem(elem_ptr)` per element
  (`:312`) — **requiring `E: HeapElement`** (`:296`).
- `HeapElement` (`v2/heap_element.rs:69-79`) is **structurally constrained** to
  `#[repr(C)]` types with `HeapHeader` at offset 0 (`:60-68`) and **explicitly
  forbids** implementing it for `Arc<>`-wrapped storage (`:41-45`: *"Implementing
  `HeapElement` for non-HeapHeader-equipped types (e.g. `Arc<>`-wrapped storage).
  The trait is structurally constrained ... implementing it for an `Arc<>`-wrapped
  struct would fail the `(*ptr).header` field access at compile time."*).
- `RefTarget` is `Arc<RefTarget>`-wrapped with no HeapHeader (`reference.rs:11-16`,
  enum def `:40-99` — no `HeapHeader` field). `SharedCell` is
  `Arc<SharedCell>`-wrapped; its first field is `state: AtomicU8` at offset 0, NOT
  a `HeapHeader` (`closure_layout.rs:130-153`, `offset_of!(SharedCell, state) == 0`
  at `:177`). Neither can be a `HeapElement`.
- There is **no `ELEM_TYPE_REFERENCE` / `ELEM_TYPE_SHARED_CELL`** discriminant
  (`typed_array.rs:344-374`: the heap element types stop at `ELEM_TYPE_STRING`/
  `_DECIMAL`/`_TYPED_OBJECT`; `v2_array_detect.rs:60-115` mirrors). So
  `release_v2_typed_array` (`typed_array.rs:416-470`) has no arm for a
  reference-element array and lands in the unstamped default
  (`:457-467`) — a **leak** (debug_assert + free-struct-only), not a release.

So storing a `PromotedCell` reference into an array **today** either (a) never
type-checks to a `TypedArray<*const RefTarget>` monomorphization (no such
instantiation exists), or (b) if forced, leaks the element shares. Flipping
B0004-array to promote without the carrier below would convert the clean
B0004 **reject** into a **leak** — a regression that violates no-known-incorrectness
only in the leak sense, but more dangerously, any half-measure that tried to
stamp a new discriminant onto the existing `drop_array_heap` path WITHOUT a real
HeapElement carrier would reach the unstamped-default misaligned-dealloc hazard
the comment at `typed_array.rs:454-456` warns about — the array double-free /
provenance class. **Refuse any such half-measure.**

### 4.2 The carrier that closes it (net-new, deferred)

The sound array fix is a **v2-raw HeapHeader-equipped element wrapper** —
`RefCellElem` — a `#[repr(C)]` struct with `HeapHeader` at offset 0 holding one
`Arc<SharedCell>` (or the `Arc<RefTarget>` reference value), so it can implement
`HeapElement` honestly:

```rust
// crates/shape-value/src/v2/ref_cell_elem.rs  (NEW)
#[repr(C)]
pub struct RefCellElem {
    header: HeapHeader,                  // offset 0 — required by HeapElement
    cell: std::mem::ManuallyDrop<Arc<SharedCell>>,  // one owning share
}
// SAFETY: header at offset 0; release_elem decrements the HeapHeader refcount,
// and on the last share drops the inner Arc<SharedCell> (one decrement),
// chaining to SharedCell::Drop (closure_layout.rs:340) for the referent value.
unsafe impl HeapElement for RefCellElem {
    unsafe fn release_elem(ptr: *const Self) {
        if v2_release(&(*ptr).header) {
            ManuallyDrop::drop(&mut (*(ptr as *mut Self)).cell); // one Arc<SharedCell> dec
            dealloc(ptr as *mut u8, Layout::new::<Self>());
        }
    }
}
```

Then:
- Add `ELEM_TYPE_REF_CELL` discriminant (`typed_array.rs:344-374`,
  `v2_array_detect.rs`), stamped at array-of-references allocation.
- Add the `ELEM_TYPE_REF_CELL => TypedArray::<*const RefCellElem>::drop_array_heap`
  arm to `release_v2_typed_array` (`typed_array.rs:422-468`).
- The `Array<&T>` element kind in the array clone/drop dispatch (the 4-table
  lockstep `Ptr(HeapKind::TypedArray)` retain/release at `stack.rs:105`,
  `:394`) is unchanged — it touches only the array's own HeapHeader; per-element
  shares are handled by `drop_array_heap`→`RefCellElem::release_elem`.

**Double-free proof for the array case (under §4.2 carrier).** Each array element
is a `*const RefCellElem` produced by a single allocation, owning one
`Arc<SharedCell>` share. `drop_array_heap` (`typed_array.rs:305-323`) walks the
buffer once (`:310-313`) calling `RefCellElem::release_elem` exactly once per
element, which retires one HeapHeader share and, on the last, one
`Arc<SharedCell>` decrement → `SharedCell::Drop` retires the referent value
exactly once. Allocator-symmetric: `RefCellElem` allocated by this module's
`alloc`, freed by this module's `dealloc` (`:320-321`); the inner
`Arc<SharedCell>` reclaimed by `Arc`'s own allocator. No token, no `_new`/`Arc::new`
mismatch. ∎ — **but this is XL net-new work** (new module, new HeapElement impl,
new discriminant, new 4-table lockstep entry, new snapshot arm, verify-merge
HeapKind/elem-type lockstep update, full test matrix).

### 4.3 Recommendation for v0.3.3 array escapes

**Keep array-container reference escapes REJECTED (B0004).** Do NOT flip the
`LoanSinkKind::ArrayStore | ArrayAssignment` arm (`solver.rs:1193-1196`) in
v0.3.3. Defer to v0.3.4 behind the §4.2 `RefCellElem` carrier. Sharpened
tripwire (KL-4-array): **refuse on sight** any array-flip that (a) stamps a new
`ELEM_TYPE_*` onto the existing `drop_array_heap` without a real
`HeapElement`-with-HeapHeader carrier, or (b) stores a raw `*const SharedCell` /
`*const RefTarget` token as an array element (no HeapHeader ⇒ `drop_array_heap`
cannot release it ⇒ the `typed_array.rs:454-456` misaligned-dealloc hazard), or
(c) reuses the object-field `Arc::decrement_strong_count` table for array
elements (arrays do not have a `field_kinds` per-element table — they are a flat
typed buffer keyed by the single `_pad` discriminant).

---

## 5. LIVE-CONTINUATION interaction (why the broad flip raises the bar)

The user chose **live continuation resume** (not replay-only). Round 1's
soundness for `is_mut` exclusivity rested on **resume ≡ bit-identical replay**
(round-1 §4.3 / O3; adversarial BREAK 5,
`snapshot-identity-correctness.md:196-223`). Under live continuation that defense
is gone — a continuation can take a second `&mut` to a restored referent with no
runtime loan record. This does **not** affect the *provenance/double-free*
soundness of §3 (that is pure refcount accounting, independent of how many
continuations run), but it means:

- The §3 object-container flip is provenance-safe **regardless** of replay vs.
  continuation — the Arc share accounting is correct either way. **Provenance is
  not the live-continuation blocker.** (The exclusivity-downgrade is — that is
  the sibling facet's territory, KL on `is_mut`.)
- The §4 array gap is independent of replay vs. continuation; it is a
  carrier-shape gap, not a resume-semantics gap.

So for **this facet specifically** (provenance / double-free), live continuation
introduces **no new double-free hazard** beyond what §3/§4 already dispose. The
container holds owning `Arc` shares; a restored continuation that reads/clones
those shares does its own `clone_with_kind`/`drop_with_kind` balanced pairs. The
hazard live-continuation adds is **exclusivity-coherence** (a different facet),
not provenance.

---

## 6. Disposition summary

| Container sink | `solver.rs` arm | v0.3.3 disposition | Soundness basis |
|---|---|---|---|
| `ObjectStore` / `ObjectAssignment` (B0004 object) | `:1197-1200` | **FLIP to promote** | §3 — `drop_fields` Reference/SharedCell arms exist (`heap_value.rs:3852`/`:3893`); allocator-symmetric `Arc::into_raw`⇄`Arc::decrement`; no token |
| `EnumStore` (B0004 enum) | `:1201-1202` | **FLIP to promote** | §3 — enum payloads are `TypedObjectStorage`-shaped; same `drop_fields` proof |
| `ArrayStore` / `ArrayAssignment` (B0004 array) | `:1193-1196` | **KEEP REJECTED (B0004)** | §4 — no `HeapElement` carrier for `Arc<>`-wrapped Ref/SharedCell; needs §4.2 `RefCellElem` (XL, v0.3.4) |
| `ReturnSlot` / `ModuleBindingStore` | `:1182`, `:1212-1214` | (round 1) FLIP | round-1 `DESIGN.md` §3 |
| `ClosureEnv` | `:1183-1184` | (sibling facet) | round-1 §6 KL-3 |
| `TypedField` (`&p.x` escape) | n/a (rejected upstream) | **KEEP REJECTED** | round-1 §6 KL-2 + BREAK 4 raw-ptr-token unresolved |

### Required-reading invariants (binding for any implementer)

1. **No raw-pointer token, ever.** The container slot/element bits MUST be a real
   `Arc::into_raw` pointer reclaimed by the matching `Arc::decrement_strong_count`
   (object) or a HeapHeader-carrier reclaimed by `v2_release`+`release_elem`
   (array, §4.2). The BREAK 4 `referent_token` shape is refused on sight.
2. **Single-source identity table.** Container slots reference the SAME round-1
   `heap_referents` `SharedCell` entry as the `PromotedCell` reference; no second
   table (CLAUDE.md §Parallel-implementation).
3. **Loan generation untouched.** Promotion is a terminal escape-sink→directive
   remapping; B0001 &mut-exclusivity (`solver.rs:1058-1144`) and genuine-dangling
   rejection stay byte-for-byte (round-1 §5 / N2 sentinel). The flip must not
   suppress the loan ("it's RC'd, borrow is safe" — refused).
4. **No array half-measure.** Per §4.3 (a)/(b)/(c) — refused on sight.
5. **No ValueWord-shape "serialization helper"** for the container element
   (CLAUDE.md §Forbidden). The element is a kinded `Arc` share, dropped via the
   existing parallel-`NativeKind` dispatch.

### Test matrix delta (this facet)

- **P-obj-1:** `module_obj.field = &local` (escaping) → promotes; after the
  declaring frame returns, deref through the field yields the live value (no UAF).
- **P-obj-2 (refcount balance, double-free guard):** drop the object + drop the
  `PromotedCell` ref → `SharedCell` refcount balances to zero, no leak, no
  double-free (the §3.2 proof's runtime check; mirrors round-1 P8).
- **P-obj-3 (aliasing):** two object fields holding refs to one promoted cell
  observe each other's mutation; snapshot→restore preserves the single-cell
  identity (one `heap_referents` token).
- **P-enum-1:** enum struct-payload escape → same as P-obj-1.
- **N-arr-1 (KL-4-array guard, close-gate):** `arr.push(&local)` where `arr`
  escapes → stays a clean **B0004 reject**, never a leak, never a segfault. This
  is the single most important negative test for this facet.
- **N-obj-local:** non-escaping `obj.field = &local` (sink local) → still NOT
  promoted, still legal as before (no spurious promotion).
