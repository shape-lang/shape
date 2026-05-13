# W17-jit-typed-object-arc-storage-migration — audit

**Branch**: `bulldozer-strictly-typed-w17-jit-typed-object-arc-storage-migration`
**Worktree**: `/home/dev/dev/shape-lang/shape-w17-jit-typed-object-arc-storage-migration`
**Branched from**: `a846ddfa` (Round 14 dispatch metadata on `bulldozer-strictly-typed`)
**Round**: Phase 3 cluster-0 Round 14 W17 (audit-first standalone, parallel with W12-map-chained)
**Author**: Round-14 W17 agent
**Date**: 2026-05-13

## Status

**SURFACE-AND-STOP — ADR-006 §2.3 amendment territory recommended.**

The dispatch text's framing of W17 as "migrate 17+ JIT-internal TypedObject
consumers to `Arc<TypedObjectStorage>` raw bits per the §2.7.5 recipe" cannot
be carried out as a same-shape migration because the JIT-internal `TypedObject`
struct (`crates/shape-jit/src/ffi/typed_object/`) and the VM-side
`Arc<TypedObjectStorage>` (`crates/shape-value/src/heap_value.rs:2356`) have
**structurally divergent in-memory layouts** that the §2.7.5 stamp-at-compile-
time discipline alone cannot bridge. The JIT-internal struct has a `u32`
manual refcount header followed by inline `u64` field cells with a
JIT-private byte-offset addressing convention; `TypedObjectStorage` is an
`Arc`-wrapped Rust struct holding `schema_id`, `slots: Vec<u64>`,
`field_kinds: Vec<NativeKind>`, `heap_mask` — entirely different Rust types
with different control-block placement (offset +4 vs. offset -16), different
field-access shapes (byte offset on inline data vs. `Vec` index on `slots`),
and different ownership semantics (manual refcount vs. `Arc`).

Per the round dispatch text §"Audit-then-proceed workflow" decision 3:

> If §1 surfaces divergent-shape ADR amendment territory: STOP. Return
> audit-only close.

This audit returns audit-only. The remaining sections substantiate the
finding and provide the per-site inventory + ADR-amendment-area proposal so
the supervisor can frame Round 15 disposition.

## §1. ADR-005 §1 / ADR-006 §2.3 fit

### §1.1. Migration target as written in the dispatch

The dispatch states: "the JIT-side consumers receive bits AND kind from the
parallel-kind track, NOT decoded from receiver_bits" and "each consumer
reads field/method from the typed Arc per the existing VM-side handler
shape (`shape-vm/src/executor/objects/typed_object_methods.rs` or similar)".

This is the §2.7.5 "stamp-at-compile-time" recipe — the consumer trusts the
kind on the parallel companion and reads the payload per the producer-side
type. For the kind classification itself (the load-bearing
`receiver_type_name` symptom), the recipe is correct and the fix is local:
read the receiver kind from the §2.7.7 / Q9 parallel track at the dispatch
shell, not from `is_number(receiver_bits)` (see §3 below for that piece).

But the dispatch ALSO frames W17 as "migrate to consuming `Arc<TypedObjectStorage>`
raw bits" — i.e. produce `Arc::into_raw(Arc<TypedObjectStorage>) as u64`
carriers in place of the current `Box::into_raw(UnifiedValue<*const u8>) as u64`
carriers (the JIT-internal allocator's output via `box_typed_object` at
`ffi/value_ffi.rs:516-518`, which under §2.7.5 IS already raw `Box::into_raw`
— there is no NaN-box wrap). That migration changes the **payload type**,
not just the kind classification.

### §1.2. ADR-005 §1 single-discriminator fit

ADR-005 §1: "`HeapValue` is the canonical discriminator for heap-resident
values. Layers above HeapValue ... take `Arc<HeapValue>` and dispatch on
`HeapValue::kind()`. Do not introduce sum types whose variants project 1:1
to `HeapKind`."

The JIT-internal `TypedObject` struct is not itself a sum type and does not
project 1:1 to `HeapKind` — it is a single concrete carrier paired with the
`NativeKind::Ptr(HeapKind::TypedObject)` kind label from the parallel track.
There is no parallel-discriminator violation today.

A migration to consume `Arc<TypedObjectStorage>` bits would preserve §1:
the slot kind `NativeKind::Ptr(HeapKind::TypedObject)` already labels the
carrier; the JIT consumer would dispatch on the kind from the parallel
track (same as the VM-side `KindedSlot` consumers) and read the payload via
`Arc::from_raw(bits as *const TypedObjectStorage)`. **§1 is not the
obstacle.**

### §1.3. ADR-006 §2.3 typed-Arc fit — THE GAP

ADR-006 §2.3: "HeapValue payloads carry typed `Arc<T>` directly.
`HeapValue::TypedObject(Arc<TypedObjectStorage>)`, etc. The slot stores
typed pointers; `Box<HeapValue>` wrapping is forbidden in new code."

`TypedObjectStorage` (`shape-value/src/heap_value.rs:2356`) carries:

```rust
pub struct TypedObjectStorage {
    pub schema_id: u32,
    pub slots: Vec<u64>,            // boxed Vec → heap-allocated indirection
    pub field_kinds: Vec<NativeKind>,  // parallel-kind track per §2.7.8
    pub heap_mask: BitVec,          // or similar for Drop dispatch
}
```

The JIT-internal `TypedObject` (`shape-jit/src/ffi/typed_object/mod.rs:67-75`)
carries:

```rust
#[repr(C)]
pub struct TypedObject {
    pub schema_id: u32,
    pub ref_count: u32,         // manual, NOT Arc control block
    // Field data follows inline (NOT through a Vec)
    // Access via get_field/set_field with byte offset on the inline buffer
}
```

The two layouts are **fundamentally incompatible**:

1. **Refcount placement.** `Arc<TypedObjectStorage>` refcount is at offset
   `-16` (Rust standard Arc control block). The JIT-internal `TypedObject`
   refcount is at offset `+4` (manually placed as the second `u32` of the
   `#[repr(C)]` header). The JIT-emitted retain/release inline ops (e.g.
   the `arc_retain` FuncRef dispatched via `retain_func_for_place` at
   `mir_compiler/ownership.rs:290` for the legacy "_ =>" arm — TypedObject
   falls through to it) bump the `+4` offset; switching to Arc would
   scribble on the wrong word. The `arc_string_retain` precedent (Round
   12 T2/T3) was exactly an isolated retain/release-callable migration
   because String has **no field-access addressing** — TypedObject's
   field-access codegen is the major scope multiplier.

2. **Field-access addressing.** JIT-emitted code reads/writes TypedObject
   fields via the byte-offset pattern at `places.rs:702-737`
   (`emit_typed_object_ptr` does `bits & UNIFIED_PTR_MASK` to recover the
   `UnifiedValue<*const u8>*`, then loads the `data` pointer at offset
   `UNIFIED_VALUE_DATA_OFFSET`, then loads field at `[data + HEADER + byte_off]`).
   Migrating to `Arc<TypedObjectStorage>` means JIT-emitted field-access
   codegen must lower to `Vec`-index addressing: `arc_ptr → storage.slots.as_ptr() + idx*8`
   — a different address-computation shape, since `Vec`'s data pointer
   lives at `arc_ptr + offset_of!(TypedObjectStorage, slots)` and then
   dereferences through a *second* pointer to reach the slot cells.

   This is not a §2.7.5 stamp-at-compile-time issue (the byte offsets are
   already known at compile time today); it is a different memory-layout
   shape that requires a different Cranelift IR pattern at every typed-
   field-load and typed-field-store site. The codegen is in
   `places.rs:702-737` (helpers) plus every caller in `field_access` /
   `places.rs:1006-1199` (the inline typed-struct field-access fast paths).

3. **Allocation surface.** `TypedObject::alloc_raw` (`allocation.rs:39-54`)
   uses `alloc_zeroed` directly with a `Layout::from_size_align(total_size,
   TYPED_OBJECT_ALIGNMENT=64)` for L1 cache-line alignment + SIMD-friendly
   field reads — JIT-side documented `O(1)` field access at 2ns vs. HashMap
   25ns (the `mod.rs:30-34` performance table). `Arc<TypedObjectStorage>`
   uses the standard `Arc::new(...)` allocator with no 64-byte alignment
   guarantee; the per-field cells live indirected through `Vec<u64>`'s
   heap buffer (a separate allocation pointed to by `slots.as_ptr()`).
   The JIT's performance argument for the inline-data layout is real and
   ADR-tracked.

4. **JIT-private allocation lifecycle.** The current JIT-internal
   `TypedObject` is allocated, refcount-tracked, and freed by a
   JIT-private code path: `jit_typed_object_alloc` → `jit_typed_object_inc_ref`
   → `jit_typed_object_dec_ref` (manual `dealloc` after `dec_ref` hits 0,
   `allocation.rs:179-198`). Migrating to `Arc<TypedObjectStorage>` puts
   this lifecycle in the Rust Arc machinery — the JIT-emitted retain/
   release must dispatch through `Arc::increment_strong_count::
   <TypedObjectStorage>` / `Arc::decrement_strong_count::
   <TypedObjectStorage>` (the standard contract). The 17+ consumer sites
   (§2 below) all currently call into the JIT-private inc/dec_ref helpers;
   they'd need to migrate to the Arc-contract helpers. This is the part
   of the dispatch that IS same-shape with W17-jit-string-carrier-
   unification (Round 12 T2/T3 commit `0324a969`) — the
   `jit_arc_string_retain` / `_release` pattern generalizes.

5. **Schema model.** Both shapes carry `schema_id: u32` and resolve through
   `TypeSchema` via `type_schema_registry`. **§5 is the only piece that
   IS same-shape.** Schema lookup itself does not require an ADR change.

Items 1–4 jointly mean the migration target as written ("consume
`Arc<TypedObjectStorage>` directly via `Arc::from_raw(bits as *const
TypedObjectStorage)` per the existing VM-side handler shape") would require
either:

- (a) **Replace** the JIT-internal `TypedObject` struct with
  `TypedObjectStorage` end-to-end. Drop JIT-private alloc/inc_ref/dec_ref/
  field-access helpers; route all object operations through the
  `Arc<TypedObjectStorage>` shape; JIT-emitted field-access codegen
  changes from inline-byte-offset to `Vec`-indirect. **Loses the documented
  64-byte alignment + inline-data performance** (`typed_object/mod.rs:28-34`)
  unless `TypedObjectStorage` is itself redesigned. This is an ADR-006 §2.3
  scope change — the canonical `TypedObjectStorage` shape needs to match
  the JIT's performance contract.

- (b) **Keep** both shapes, with `TypedObjectStorage` on the VM side and
  the JIT-internal `TypedObject` on the JIT side, and translate at the
  cross-crate boundary (§2.7.5 stable FFI). The kind label
  `NativeKind::Ptr(HeapKind::TypedObject)` would mean different concrete
  payloads in different crates — a per-crate carrier-shape decision that
  defeats §2.7.5's single-shape-per-kind-label discipline. **Defection-
  attractor territory** — this is exactly the "carrier-shape drift at the
  crate boundary" pattern §2.7.10 / Q11 spent the entire dispatch-ABI
  rebuild eliminating.

- (c) **Redesign** `TypedObjectStorage` to be layout-compatible with the
  JIT-internal `TypedObject` struct (or vice versa) — give
  `TypedObjectStorage` `#[repr(C)]`, inline `u64` cells (not `Vec<u64>`),
  matching alignment + refcount placement. An ADR-006 §2.3 amendment
  effectively, since the canonical typed-Arc shape would have non-Rust
  Arc semantics (`Arc::new(TypedObjectStorage::new(...))` no longer
  works because the storage is variably-sized; would need custom
  `Arc`-like with `Layout::from_size_align`). This is fundamentally a
  redesign of `TypedObjectStorage`.

### §1.4. The actual load-bearing surface — receiver_type_name

The Round 13 T1' close report's surface description ("`receiver_type_name`'s
legacy NaN-box tag-decode reads `is_number(receiver_bits) = true` for raw
`Arc::into_raw(Arc<TypedObjectStorage>)` pointer bits") is accurate **as a
classification gap**, but the framing "the Round-12 T2/T3 producer
migration's carrier" is misleading: Round 12 T2/T3 migrated **String**
producers to raw `Arc::into_raw(Arc<String>) as u64` carriers; the
TypedObject producer migration was NOT in T2/T3 scope and explicitly
surface-and-stopped at `terminators.rs:481-522`. The TypedObject producer
TODAY (`box_typed_object` at `ffi/value_ffi.rs:516-518`) emits raw
`Box::into_raw(UnifiedValue<*const u8>) as u64` — already raw bits without
NaN-box wrap per §2.7.5 (verified empirically via `SHAPE_JIT_TRACE=1` —
debug-instrumented allocation prints `result=0x56c5488796a0 kind=None`).

The classification gap is real regardless of payload shape: `is_number()`,
`is_heap()`, `heap_kind()`, `is_typed_object()`, `is_inline_function()` and
their siblings all return wrong answers on raw `Box::into_raw` / `Arc::
into_raw` carriers because they were tag-bit predicates on the deleted
NaN-box dispatch. The 5-arm cascade in `receiver_type_name`
(`call_method/mod.rs:51-81`) is one site; there are MANY others (see §2
inventory).

The **correct fix for the load-bearing surface** is:

> Migrate `receiver_type_name` (and the broader legacy JIT-format dispatch
> in `jit_call_method` at `call_method/mod.rs:572-612`) to read the receiver
> kind from the §2.7.7 / Q9 parallel-kind track AND the schema id via the
> JIT-internal `(*ptr).schema_id` direct field read (NOT via the deleted
> tag-bit `heap_kind()` predicate). This is a CLASSIFICATION-LAYER fix and
> requires NO change to the underlying TypedObject payload shape.

This is fundamentally a **smaller, in-crate fix** than the dispatch text
implies. It does NOT require migrating the JIT-internal `TypedObject` to
`Arc<TypedObjectStorage>`.

### §1.5. ADR amendment area

The dispatch text frames W17 as a typed-Arc carrier migration. The audit
finds:

- The **classification gap** (`receiver_type_name` + cascade) is real and
  has a clean §2.7.5-discipline fix that doesn't touch the TypedObject
  payload shape.
- The **typed-Arc carrier migration** (replacing JIT-internal `TypedObject`
  with `Arc<TypedObjectStorage>`) is ADR-006 §2.3 amendment territory because
  the two payload shapes are structurally divergent. Items 1–4 of §1.3 each
  individually need supervisor-level disposition; jointly they amount to a
  redesign decision on `TypedObjectStorage` (or a permanent per-crate
  carrier-shape divergence under §2.7.5, which has its own discipline issues).

**Recommendation**: scope-narrow W17 to the **classification gap** (the
load-bearing surface for kickoff Smoke 3 JIT), and dispatch the **typed-Arc
carrier migration** as a separate sub-cluster after supervisor disposition
on the ADR-006 §2.3 amendment. The carrier migration is not load-bearing
for kickoff Smoke 3 JIT — it is load-bearing for a different (and larger)
goal, namely "JIT and VM share one TypedObject representation". That goal
is real, but a different scope.

## §2. 17+ consumer inventory

### §2.1. JIT-private heap operations on `TypedObject` (NOT `Arc<TypedObjectStorage>`)

These sites manipulate the JIT-internal `#[repr(C)]` struct directly. They
DO NOT cross the crate boundary into shape-vm. They are all in shape-jit.

| # | File:line | Symbol | Call shape | Migration target |
|---|---|---|---|---|
| 1 | `ffi/typed_object/allocation.rs:83` | `jit_typed_object_alloc` | Allocates `TypedObject*` via `alloc_zeroed`, wraps in `UnifiedValue<*const u8>` via `box_typed_object` | Same-shape (no change) — already returns raw `Box::into_raw(...) as u64` per §2.7.5 |
| 2 | `ffi/typed_object/allocation.rs:126` | `jit_new_typed_object` | Allocates + initialises fields; same wrap as #1 | Same-shape (no change) |
| 3 | `ffi/typed_object/allocation.rs:159` | `jit_typed_object_inc_ref` | Gates on `is_typed_object(obj_bits)` (broken on raw-bits carrier — always returns false); reads `*ptr.inc_ref()` after unbox | **Drop `is_typed_object` gate** — same recipe as `field_access.rs:124-137` already applied to `jit_typed_object_get/set_field`. Read kind from parallel companion at caller site; trust the kind label, deref via `unbox_typed_object` (which is now just a `bits & UNIFIED_PTR_MASK` mask + `data` field-load — see `value_ffi.rs:521-523`). |
| 4 | `ffi/typed_object/allocation.rs:179` | `jit_typed_object_dec_ref` | Same broken `is_typed_object` gate as #3 | Same fix as #3 |
| 5 | `ffi/typed_object/field_access.rs:139` | `jit_typed_object_get_field` | Already migrated to drop the `is_typed_object` gate (W12-jit-binop-after-heap-read-kind-tracker close, 2026-05-12 — see `field_access.rs:122-137` comment). | **Already correct.** Reads the inline field via `(*ptr).get_field(offset)`. |
| 6 | `ffi/typed_object/field_access.rs:176` | `jit_typed_object_set_field` | Same as #5 — already correct. | **Already correct.** |
| 7 | `ffi/typed_object/field_access.rs:209` | `jit_typed_object_schema_id` | Gates on `is_typed_object(obj_bits)` (broken) before reading `(*ptr).schema_id` | Drop the `is_typed_object` gate; trust the parallel-kind companion. Same as #3. |
| 8 | `ffi/typed_object/merge_ops.rs:37` | `jit_typed_merge_object` | Gates on `is_typed_object(left_obj) && is_typed_object(right_obj)` (both broken); unboxes both, allocates merge, copies fields | Drop both `is_typed_object` gates; trust the parallel-kind companion on both arg slots. |
| 9 | `ffi/typed_object/merge_ops.rs:89` | `jit_typed_object_from_hashmap` | Gates on `is_typed_object(obj_bits)` (broken); rebuilds as typed object | Same fix |
| 10 | `ffi/data.rs:381` | `jit_get_field_typed` / wrapper | `if is_typed_object(obj) { let ptr = unbox_typed_object(obj); ... }` — gated read | Drop the gate; consume the kind from the parallel companion (current caller signature doesn't carry it; needs producer-side stamping at the call site OR a kinded call signature for these JIT-FFI bodies) |
| 11 | `ffi/data.rs:451` | sibling `jit_set_field_typed` | Same shape as #10 | Same fix |
| 12 | `ffi/object/property_access.rs:209` | property-access dispatch | Same gated `unbox_typed_object` pattern | Same fix |

### §2.2. Method dispatch + classification surface (the load-bearing site)

| # | File:line | Symbol | Call shape | Migration target |
|---|---|---|---|---|
| 13 | `ffi/call_method/mod.rs:51-81` | `receiver_type_name` | 5-arm tag-bit cascade: `is_number(receiver_bits)` → "number"; `TAG_BOOL_*` → "bool"; `TAG_NULL/TAG_NONE` → None; `heap_kind` match for HK_STRING/HK_ARRAY/HK_TYPED_OBJECT/HK_JIT_OBJECT/HK_DURATION/HK_TIME. **All five tag-bit predicates broken on raw `Box::into_raw` carriers per §2.7.5.** | Receive the `NativeKind` from the parallel-kind track; dispatch on the kind directly; for `Ptr(HeapKind::TypedObject)` read `(*ptr).schema_id` via direct field load and resolve via `type_schema_registry`. Signature changes from `(receiver_bits: u64, exec_ctx: &ExecutionContext) -> Option<String>` to `(receiver_bits: u64, receiver_kind: NativeKind, exec_ctx: &ExecutionContext) -> Option<String>`. |
| 14 | `ffi/call_method/mod.rs:111-167` | `try_call_user_method` | Calls #13; passes `receiver_bits` through to the JIT-emitted function via `ctx_mut.stack` push | Pass `receiver_kind` through to #13; otherwise unchanged. |
| 15 | `ffi/call_method/mod.rs:572-592` | builtin JIT-format method dispatch cascade (`is_ok_tag` / `is_err_tag` / `is_number` / `is_inline_function` / `heap_kind` cascade for HK_ARRAY/HK_STRING/etc.) | All 6+ tag-bit predicates broken on raw-bits carriers per §2.7.5. | Migrate to kind-from-parallel-track dispatch. The receiver_kind is **already popped from the parallel track at line 332-349** — just use it instead of re-classifying via tag bits. Each `is_*` predicate gets replaced by a `matches!(receiver_kind, ...)` check. |

### §2.3. Producer-side correctness

| # | File:line | Symbol | Call shape | Migration target |
|---|---|---|---|---|
| 16 | `mir_compiler/places.rs:702-737` | `emit_typed_object_ptr` / `inline_typed_field_{get,set}` | Reads field via `bits & UNIFIED_PTR_MASK` then double-load through `UnifiedValue.data` then byte-offset on `TypedObject` inline data | **No change needed** — this is the JIT-format inline-byte-offset addressing on the JIT-internal `TypedObject` struct. Works correctly when payload is `UnifiedValue<*const u8>`. Only needs migration if the JIT-internal TypedObject is replaced with `Arc<TypedObjectStorage>` — which is the §1.3 ADR-amendment territory. |
| 17 | `mir_compiler/terminators.rs:481-522` | print `Ptr(HeapKind::TypedObject)` surface-and-stop | Returns `Err(...)` because the JIT-side payload (`UnifiedValue<*const u8>`) doesn't match what `jit_print_typed_object` (`conversion.rs:426`) expects (`Arc::into_raw(Arc<TypedObjectStorage>) as u64`). | Either (a) bridge the two payload shapes at the print call (a structurally distinct fix), (b) migrate the producer (§1.3 ADR amendment), or (c) **add `jit_print_typed_object_jit_format`** that handles the JIT-internal `UnifiedValue<TypedObject*>` shape — a parallel printer body for the JIT carrier. Option (c) defeats §2.7.5 single-shape-per-kind discipline; (a) requires ad-hoc translation that bleeds carrier-shape concerns into the print site; (b) is the ADR amendment. **None of these is in the W17 dispatch's claimed scope.** |

### §2.4. Refcount-retain/release dispatch

| # | File:line | Symbol | Call shape | Migration target |
|---|---|---|---|---|
| 18 | `mir_compiler/ownership.rs:290` | `retain_func_for_place` "_" arm | `TypedObject`-kinded slots fall through to `self.ffi.arc_retain` (operates on `UnifiedValue<T>` HeapHeader at offset +4) | Same-shape — works correctly on the current JIT-internal `TypedObject` payload (which DOES live behind `UnifiedValue<*const u8>`). Only needs migration if §1.3 ADR amendment proceeds. |
| 19 | `mir_compiler/ownership.rs:329` | `release_func_for_place` mirror | Same | Same |

### §2.5. LoC estimate

**Classification-layer fix (§2.2 + drop-the-gate sites #3/#4/#7/#8/#9/#10/#11/#12)**:
~150–250 LoC including docstring updates + tests. Straightforward, no
new ABI shape, no ADR amendment.

**Typed-Arc carrier migration (full §1.3 (a))**: ~1500–3000 LoC. Touches:

- `crates/shape-value/src/heap_value.rs::TypedObjectStorage` redesign or
  layout-compatibility ADR amendment.
- All 17+ consumer sites listed in §2.1–§2.4 (re-route through Arc-
  contract retain/release; re-shape inline field-access codegen for
  `Vec`-indirect addressing OR redesign `TypedObjectStorage` to match
  JIT inline-data shape).
- VM-side `op_new_object_*` allocator (would need to drop or rebuild for
  cross-crate carrier-shape compatibility).
- VM-side `KindedSlot::from_typed_object` constructor + `Drop` (which
  currently calls `Arc::decrement_strong_count::<TypedObjectStorage>`).
- ALL tests under `crates/shape-jit/src/ffi/typed_object/tests/` +
  `value_ffi::tests::test_typed_object_*` + integration round-trip tests.
- `jit_print_typed_object` already expects the Arc shape (so this part
  becomes consistent), but the print MIR Call-terminator surface (`terminators.rs:481-522`)
  changes from Err-surface to `jit_print_typed_object` dispatch only if the
  carrier shape unifies.

This scope blows past the ~400 LoC dispatch estimate by 4–7×.

## §3. Cross-crate boundary check

The classification-layer fix (§1.4 / §2.2) does NOT cross the shape-jit →
shape-vm boundary. The fix is in-crate: `receiver_type_name` and the
`jit_call_method` legacy-fallback cascade live in
`crates/shape-jit/src/ffi/call_method/mod.rs`. Schema lookup goes through
`exec_ctx.type_schema_registry()` (which is shape-runtime, not shape-vm),
but that's the existing call shape — no new boundary crossing.

No new `VirtualMachine::jit_trampoline_*` API needed for the
classification-layer fix.

The typed-Arc carrier migration (§1.3 ADR-amendment territory) DOES cross
the boundary — it changes the payload shape consumed by both shape-jit
and shape-vm. That's not a §2.7.5-stable-FFI question; it's an ADR-006
§2.3 question.

## §4. Refuse-on-sight discipline

The audit explicitly refuses:

- **"Preserve NaN-box decode for one edge case"** — refused. The
  classification-layer fix replaces ALL five tag-bit predicates in
  `receiver_type_name` (and the broader cascade) with kind-from-parallel-
  track classification. Zero NaN-box predicates remain.

- **"Bool-default for unproven receiver_type kind"** — refused per
  §2.7.7 #4 / #9. When the kind on the parallel track is absent
  (`stack_kind_code::decode(receiver_code) == None`), the existing
  surface-and-stop discipline at `call_method/mod.rs:336-349` is preserved
  (returns `TAG_NULL` with structured `eprintln!` cite under
  `SHAPE_JIT_DEBUG=1`).

- **"bridge / probe / helper / hop / translator / adapter / shim"
  descriptors** — refused per CLAUDE.md "Renames to refuse on sight"
  (broader-family regex). The classification-layer fix is described as
  "kind-from-parallel-track classification at the dispatch shell"; no
  defection-attractor framing.

- **"Tracked as a follow-up for any individual consumer site"** —
  refused. The classification-layer fix migrates ALL of #3, #4, #7, #8, #9,
  #10, #11, #12, #13, #14, #15 in one round (~13 sites). No site-by-site
  deferral.

- **"Migrate the typed-Arc carrier shape in this round per the dispatch
  text"** — REFUSED on §1.3 grounds. The supervisor's authority is
  required to amend ADR-006 §2.3 OR to accept the §1.3(b) per-crate
  carrier-shape divergence (which has its own discipline issues —
  defection-attractor territory under §2.7.10 / Q11). This audit
  surfaces-and-stops.

## §5. Disposition — supervisor-bound questions

The supervisor's Round-15 dispatch options (the audit recommendations):

### Option α — narrow-scope close (recommended)

Dispatch W17 as a **classification-layer migration only**: ~13 consumer
sites at §2.1 #3/#4/#7/#8/#9/#10/#11/#12 + §2.2 #13/#14/#15, ~150-250 LoC.
Drop the tag-bit predicates on raw-bits carriers; consume kind from the
parallel-kind track at every dispatch shell. The current JIT-internal
`TypedObject` payload shape is preserved.

Acceptance: kickoff Smoke 3 JIT `t.name()` → `x` matches VM.
`jit_print_typed_object` surface-and-stop at `terminators.rs:481-522`
remains UNCHANGED (separate scope, separate sub-cluster — the print path's
carrier-shape mismatch with the VM-side `jit_print_typed_object` is a
distinct surface from the classification path).

### Option β — full ADR-006 §2.3 amendment (supervisor disposition required)

Amend ADR-006 §2.3 to address one of:

- **Option β.1**: redesign `TypedObjectStorage` to use inline-data
  layout with 64-byte alignment + manual refcount, matching the JIT-
  internal `TypedObject` performance contract. Requires custom Arc-like
  semantics (variably-sized + non-standard refcount placement).
  Cross-crate code share via the redesigned struct.

- **Option β.2**: accept per-crate carrier-shape divergence under
  `NativeKind::Ptr(HeapKind::TypedObject)` — JIT side keeps inline-data
  shape, VM side keeps `Arc<TypedObjectStorage>` with Vec-indirect.
  Stable-FFI boundary (§2.7.5) translates between the two when crossing
  shape-jit↔shape-vm. **Defection-attractor risk**: this is the
  carrier-shape-drift pattern §2.7.10 / Q11 eliminated for method
  dispatch; reintroducing it for a single HeapKind variant needs explicit
  ADR justification.

- **Option β.3**: keep `Arc<TypedObjectStorage>` as the canonical shape;
  delete the JIT-internal `TypedObject` struct; rebuild JIT-emitted
  TypedObject codegen against the Arc-indirect-Vec shape. Loses the
  documented inline-data performance contract; performance regression
  measurement is part of the amendment.

The audit makes no recommendation between β.1 / β.2 / β.3 — that is
supervisor authority.

### Option γ — scope split

Dispatch Option α immediately (Round 15 W17-narrow), and queue
**W17-typed-object-carrier-shape-decision** as a CLUSTER-1 follow-up
that drives the ADR-006 §2.3 amendment via either β.1 / β.2 / β.3.
Cluster-0 close (post Smoke matrix verification) does NOT block on β —
the classification-layer fix is sufficient for kickoff Smoke 3 JIT.

## §6. Empirical evidence

Direct verification from the worktree at the audit head
(`a846ddfa`, Round 14 dispatch metadata):

```
$ SHAPE_JIT_TRACE=1 ./target/release/shape run --mode jit /tmp/smoke3.shape
[alloc] schema=54 result=0x56c5488796a0 kind=None HK_TYPED_OBJECT=1
```

The producer `box_typed_object` returns `0x56c5488796a0` — raw
`Box::into_raw(UnifiedValue<*const u8>) as u64`. The `kind=None` comes
from `heap_kind(0x56c5...)` because `is_heap()` returns false on
non-TAG_BASE bits. So:

- Producer ALREADY emits raw `Box::into_raw` carriers (per §2.7.5);
  there is no T2/T3-style producer migration to do for TypedObject.
- Consumer tag-bit predicates (`is_heap`, `is_typed_object`, `heap_kind`,
  `is_number`, `is_inline_function`, `is_ok_tag`, `is_err_tag`) ALL
  return wrong answers on these carriers.
- The §2.7.7 / Q9 parallel-kind track DOES carry the correct kind at the
  pop site:
  ```
  [jit-call-method] method='name' receiver_kind=Ptr(TypedObject)
    receiver_code=129 receiver_bits=0x56d0fa0f49f0
  ```
  The classification gap is at the legacy fallback cascade that re-
  classifies via the broken tag-bit predicates after the parallel-track
  kind has already been correctly read.

This is consistent with the audit's recommendation: the load-bearing fix
is the **classification layer**, not the **payload shape**.

## §7. Close

This audit returns **audit-only** per §1.3 / §1.5 / §5 — the dispatch
text's framing of W17 conflates two distinct migrations:

1. A small, in-crate, §2.7.5-discipline-pure classification-layer fix
   (~13 sites, ~150–250 LoC, no ADR amendment) that closes kickoff
   Smoke 3 JIT.

2. An ADR-006 §2.3 amendment-territory typed-Arc carrier migration
   (~1500–3000 LoC, requires supervisor-level decision on β.1/β.2/β.3)
   that is **NOT load-bearing for kickoff Smoke 3 JIT**.

Recommendation: supervisor authorises Option γ (split). Round 15 dispatches
W17-narrow (Option α scope) to close the kickoff Smoke 3 surface. The
typed-Arc carrier amendment becomes a separate cluster-1 follow-up after
β.1/β.2/β.3 disposition.

No source changes that regress Round 13 state are made in this audit-only
close.
