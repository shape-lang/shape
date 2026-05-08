# Cluster 1 audit — type_schema slot construction & readback

**Architectural anchor (binding)**: `docs/adr/005-typed-slot-construction.md`
(Accepted 2026-05-08). The single-discriminator discipline and typed-slot-
storage rules below are downstream of that ADR.

**Scope** (expanded by ADR-005, 2026-05-08): establish typed-slot-construction
discipline end-to-end with strings as the load-bearing example, across both
construction (`typed_object_from_pairs`) and slot storage (`ValueSlot`
per-FieldType constructors + NativeKind-driven drop) and the corresponding
readback (`typed_object_to_hashmap_nb`). Touch surface:

- `crates/shape-runtime/src/type_schema/mod.rs` (~330 LoC) — public API.
- `crates/shape-value/src/slot.rs` — per-FieldType constructors land here.
- `crates/shape-value/src/heap_value.rs` — TypedObject Drop refactor (NativeKind
  dispatch instead of, or alongside, `heap_mask` bit dispatch).
- Cross-crate `from_heap` consumers (full enumeration in §"Migrator dispatch
  checklist"): 9 sites in shape-runtime (4 in `stdlib/json.rs`, 2 in
  `type_schema/mod.rs`) and ~21 sites in shape-vm (`exceptions/mod.rs`,
  `control_flow/foreign_marshal.rs`, `builtins/special_ops.rs`,
  `state_builtins/core.rs`, `vm_impl/stack.rs`, `executor/objects/...`).
- Schema-cache wire conversion (`schema_cache.rs`) and multi-table
  (`multi_table/functions.rs`) consumers as previously enumerated.

**Predicted error drop** (revised for ADR-005 scope):

- shape-runtime --lib in isolation: -3 to -8 (the prior -2 to -5 absorbed the
  inline `nb_to_slot` body's stale `ValueWord`/`ValueWordExt` cites; ADR-005
  expansion adds the `from_heap` migration which forces the stale
  `use shape_value::{ValueWord, ValueWordExt}` import deletion and may also
  remove a few cascade cites in `stdlib/json.rs` once that file flips to
  per-FieldType constructors).
- Cross-crate (shape-vm cascade once construction lands): unknown; depends on
  whether the migrator chooses to flip shape-vm's ~21 `from_heap` sites in the
  same cluster (deferred decision per Q3).
- **Calibration warning**: Phase 2c had 8/8 cluster-decomposition predictions
  miscount. ADR-005's "1-2 weeks rather than 4-6 days" estimate is honest only
  if the readback walker (Q2) and the shape-vm `from_heap` cascade (Q3) move
  in this cluster. If both defer, the prediction collapses to ~3 days but
  defers most of the load-bearing migration. **Recommend the supervisor
  resolve Q2/Q3 before dispatching the migrator** so the budget covers the
  full ADR scope.

**Effort estimate**: 1-2 weeks (per ADR-005 §Costs), assuming Q2 (readback
walker) and Q3 (shape-vm cascade) are in scope. ~3-4 days if both deferred,
but that's a Layer 1 partial.

**Audit performed by**: scout-2026-05-07 (initial), scout-2026-05-08
(ADR-005 update).

## Audit 1 — consumer call shape

The cluster surface is the 4-function public API of `type_schema/mod.rs`
plus the inlined `nb_to_slot` body inside `typed_object_from_pairs`.
Public surface (post-N9):

- `pub fn typed_object_from_pairs(fields: &[(&str, ValueWord)]) -> ValueWord`
  (mod.rs:192)
- `pub fn typed_object_to_hashmap_nb(value: &ValueWord) -> Option<HashMap<String, ValueWord>>`
  (mod.rs:287)
- `pub fn lookup_schema_by_id_public(id: SchemaId) -> Option<TypeSchema>`
  (mod.rs:128)
- `pub fn ensure_next_schema_id_above` / `register_predeclared_any_schema`
  (mod.rs:71, mod.rs:84) — schema-id management; not slot-bound.

### Construction-side consumers (typed_object_from_pairs)

All call shapes pass `&[(&str, ValueWord)]`:

- `crates/shape-runtime/src/const_eval.rs:165` — comptime fold producing a
  TypedObject from constant-folded field values; ValueWord pairs already
  present in caller.
- `crates/shape-runtime/src/schema_cache.rs:90, 104, 121, 123` — wire-conversion
  schema entity / table / column object construction; multiple sites in one
  file all build pairs from `serde_json::Value` decoders that already produce
  ValueWord.
- `crates/shape-runtime/src/multi_table/functions.rs:137, 143` —
  `align_tables` / `correlation` legacy `IntrinsicFn`-shaped bodies build
  result pair-list; **interlocks with cluster #6** (these are
  `fn(&mut ExecutionContext, &[ValueWord]) -> Result<ValueWord>` bodies
  that need to migrate together).
- `crates/shape-runtime/src/multiple_testing.rs:89` — test-runner result
  TypedObject; one site.
- `crates/shape-runtime/src/type_schema/builtin_schemas.rs` — module-init
  schema bootstrap; references the function for documentation but doesn't
  invoke it on the hot path.
- `crates/shape-vm/src/executor/state_builtins/core.rs`,
  `executor/objects/object_creation.rs`,
  `executor/objects/datatable_methods/*`,
  `executor/printing.rs`,
  `executor/builtins/special_ops.rs`,
  `executor/state_builtins/introspection.rs` — all shape-vm executor
  consumers that materialize a TypedObject from per-method-result
  ValueWord pairs.
- `crates/shape-vm/src/compiler/comptime.rs`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs` — comptime
  evaluator's TypedObject literal materialization.

Calling convention: every consumer collects ValueWord pairs (already
in ValueWord form on the caller side), then calls
`typed_object_from_pairs`. The ValueWord input shape is *load-bearing*
because callers receive ValueWord-typed return values from prior layers
(comptime evaluator, executor, schema-cache builders).

### Readback-side consumers (typed_object_to_hashmap_nb)

- `crates/shape-runtime/src/const_eval.rs:435, 549` — comptime test
  assertions reading TypedObject fields back as a `HashMap<String, ValueWord>`.
- `crates/shape-runtime/src/schema_cache.rs:135, 151, 156, 174` — wire
  conversion's reverse direction (HashMap-of-objects → schema-cache rebuild).
- `crates/shape-runtime/src/type_schema/mod.rs:273` — local test only.

Readback returns a `HashMap<String, ValueWord>` where each value is
reconstructed via either `slot.as_heap_nb()` (heap-tagged slot) or
`unsafe { ValueWord::clone_from_bits(slots[i].raw()) }` (inline-tagged
slot). The latter is the **cluster #1 readback gap**: raw bits in the
ValueSlot are interpreted as a ValueWord representation. Removing
ValueWord requires this fallback to project to a typed leaf instead.

### Twin parallel-impls in shape-vm (untouched by N9 per supervisor scope)

- `crates/shape-vm/src/executor/objects/object_creation.rs:317`:
  `nb_to_slot_with_field_type` — schema-driven, takes a `FieldType`
  parameter. Does **not** belong to type_schema cluster; it's the
  per-field-type strict-typed primitive that exists today and is
  consumed by the executor's object literal materialization.
- `crates/shape-vm/src/executor/exceptions/mod.rs:26`: a trivial
  `nb_to_slot` clone. Belongs to shape-vm cascade, not cluster #1.

## Audit 2 — marshal-API readiness

### What's already in place

- **`ValueSlot::from_*` typed primitives** (shape-value/src/slot.rs):
  `from_number`, `from_int`, `from_bool`, `from_u64`, `from_heap`,
  `from_raw`, plus `none()`. ADR-005 makes `from_heap` transitional and
  requires per-FieldType heap constructors alongside (`from_string_arc`,
  `from_typed_array`, `from_typed_object`, etc.) — see §"Per-FieldType
  ValueSlot constructors" below.
- **`FieldType` enum** (`type_schema/field_types.rs:35`) — already covers
  i8/u8/.../i64/u64, f64, bool, string, decimal, timestamp, array,
  object, plus `Any` (the strict-typed-violation). Schema layer is
  already strict-typed-aware; ADR-005 leverages this — the schema
  supplies the kind at construction time.
- **N9 inline body preserved** (mod.rs:215-244): the `is_heap` branch +
  unified-heap-bit-47 handling + `ValueSlot::from_raw(value.raw_bits())`
  fallback are all in place. Inlining preserved verbatim per N9
  supervisor binder; this is the body cluster #1 has to replace
  end-to-end (not patch).
- **shape-vm's `nb_to_slot_with_field_type`** (object_creation.rs:317)
  is a **shape-vm-side schema-driven equivalent** of the per-FieldType
  shape ADR-005 codifies. The pattern exists; cluster #1 is the
  shape-runtime parallel landing + ADR-grade discipline.
- **ADR-005 marker comments** already at five load-bearing sites
  (`heap_variants.rs:55,92`, `slot.rs:54`, `native_kind.rs:98`,
  `typed_module_exports.rs:49`, `json_value.rs:17`). Migrator adds
  one more at the `TypedFieldValue` definition (binding from ADR-005
  §Visibility, drift-prevention §1).

### What's missing

- **`TypedFieldValue` enum definition** (12-variant shape fixed by
  ADR-005 §Decision §1-2; see §Architectural shape below). Lives in
  `crates/shape-runtime/src/type_schema/mod.rs` adjacent to the
  `typed_object_from_pairs` rewrite, with an inline `// ADR-005`
  comment justifying the `String` exception per ADR's drift-prevention
  list.
- **Per-FieldType heap constructors on `ValueSlot`**:
  `from_string_arc(Arc<String>)`, `from_typed_array(Arc<TypedArrayData>)`,
  `from_typed_object(...)`, `from_decimal(Decimal)`, `from_hashmap(...)`,
  `from_datatable(...)`, etc. — one per `FieldType` heap-resident arm.
  See §"Per-FieldType ValueSlot constructors".
- **NativeKind-driven drop**: ValueSlot or `HeapValue::TypedObject` needs
  a Drop impl that consults the schema's per-slot `FieldType` (project to
  `NativeKind`) to dispatch the right deallocator (Arc::decrement_strong_count
  vs Box::from_raw, etc.). Today, no Drop impl exists on `HeapValue::TypedObject`
  itself; `slot.drop_heap()` is dead code (only called from the slot.rs test).
  This is a **real soundness gap** the cluster closes — heap slots stored via
  `from_heap` today never get freed unless callers rebuild the HeapValue and
  drop it (which is partial).
- **Readback strategy**: `typed_object_to_hashmap_nb`'s
  `clone_from_bits(slots[i].raw())` is the load-bearing inline fallback;
  schema-driven readback (`read_slot_nb` in shape-vm) is the strict-
  typed-correct mirror but doesn't exist on the shape-runtime side.
  Whether this is in cluster #1 or follows is Q2.
- **Stale `use shape_value::{ValueWord, ValueWordExt}`** at mod.rs:22.
  Per N9 supervisor self-correction, this **must be paired** with the
  signature migration — Rule F application binder.

## Architectural shape (fixed by ADR-005)

The architectural shape is **not negotiable** at the cluster-#1 level —
ADR-005 fixes it. The shape is:

```rust
// In crates/shape-runtime/src/type_schema/mod.rs, with `// ADR-005` marker.
pub enum TypedFieldValue {
    F64(f64),
    I64(i64),
    I8(i8), U8(u8), I16(i16), U16(u16), I32(i32), U32(u32), U64(u64),
    Bool(bool),
    String(Arc<String>),     // explicit exception, named in ADR-005 §Decision §2
    Heap(Arc<HeapValue>),    // single discriminator for all other heap types
}
```

12 variants. **No** `Array`/`Object`/`HashMap`/`Decimal`/`Timestamp` /
`TypedArray` / `DataTable` / `IoHandle` / `BigInt` / etc. top-level
variants. All non-string heap types route through `Heap(Arc<HeapValue>)`
and dispatch via `HeapValue::kind()`. Adding any heap-arm variant to
`TypedFieldValue` is forbidden by ADR-005 §Forbidden under this ADR.

The public signature flips to:

```rust
pub fn typed_object_from_pairs(fields: &[(&str, TypedFieldValue)]) -> Arc<HeapValue>;
//                                              ^^^^^^^^^^^^^^^                 ^^^^^^^^^^^^^^^
//                                              (was ValueWord)                 (was ValueWord)
```

Return type is `Arc<HeapValue>` (the heap-resident TypedObject), not
`ValueWord` — strict-typed plan deletes ValueWord.

### Rejected alternatives, recorded for posterity

(For future sessions reading this audit cold; do not expand.)

- **Option α (`&[(&str, Arc<HeapValue>)]`)** — rejected because HeapValue
  has no inline scalar variants (`HeapValue::Int64`, `HeapValue::Bool`,
  etc. don't exist). Forcing scalars through `Arc<HeapValue>` is either
  a heap allocation per scalar field (perf regression) or requires adding
  inline-scalar arms to HeapValue (architectural change conflicting with
  the heap-vs-stack split).
- **Option β (`&[(&str, ValueSlot)]`)** — rejected because callers don't
  have FieldType at the call site (const_eval, comptime receive an
  upstream typed value and the schema is queried inside
  `typed_object_from_pairs`). Forcing per-callsite typed conversion
  inverts the schema lookup.
- **Option γ (`TypedObjectBuilder` fluent API)** — rejected as
  parallel-discriminator risk: the per-FieldType `set_*` setter family
  is itself a parallel-discriminator layer above ValueSlot. Larger scope
  than necessary; ε is the smaller-scope strict-typed shape.
- **Option ε with `TypedFieldValue` mirroring `ConcreteReturn` (initial
  sketch)** — superseded by ADR-005's explicit single-discriminator rule.
  Initial sketch had separate `Array(Arc<HeapValue>)` /
  `Object(Arc<HeapValue>)` / `HashMap(Arc<HeapValue>)` variants —
  exactly the parallel-discriminator pattern ADR-005 forbids.
  Compressed to single `Heap(Arc<HeapValue>)` arm with `String` as the
  one named exception.
- **Reuse `ConcreteReturn` directly as input type** — rejected because
  ConcreteReturn is itself a cluster-#7 cleanup target (it's a parallel
  discriminator at the function-return ABI). Reusing it would land
  cluster #1 on top of code that's slated for revision in cluster #7.

## Per-FieldType ValueSlot constructors (ADR-005 §3 cross-crate work)

ADR-005 §3 requires slot storage to use typed pointers, not
`Box<HeapValue>` wrappers. Cluster #1 must:

### Constructors to add on `ValueSlot`

(Each new constructor is keyed to a `FieldType` heap-resident variant;
each goes in `crates/shape-value/src/slot.rs` with a `// ADR-005`
marker.)

| Constructor | FieldType arm | Storage |
|---|---|---|
| `from_string_arc(s: Arc<String>)` | `FieldType::String` | `Arc::into_raw(s) as u64` |
| `from_typed_array(a: Arc<TypedArrayData>)` | `FieldType::Array(_)` | `Arc::into_raw(a) as u64` |
| `from_typed_object(o: Arc<HeapValue::TypedObject{..}>)` | `FieldType::Object(_)` | `Arc::into_raw(o) as u64` |
| `from_decimal(d: Decimal)` | `FieldType::Decimal` | re-uses `from_int(d.bits)` form |
| `from_timestamp(ts: i64)` | `FieldType::Timestamp` | re-uses `from_int` |
| `from_hashmap(h: Arc<HashMapData>)` | (extension) | `Arc::into_raw(h) as u64` |

The `from_*` family for inline scalars (`from_number`, `from_int`,
`from_bool`, `from_u64`) already exists; cluster #1 does not change them.

`from_heap(value: HeapValue)` is **transitional** per ADR-005 §3
(comment already at slot.rs:54). New code uses per-FieldType
constructors; existing call sites migrate as part of the cluster.

### `from_heap` call-site migration (cross-crate)

Counted from `rg "ValueSlot::from_heap" crates/`:

- **shape-runtime** (5 sites in `stdlib/json.rs`, 2 in
  `type_schema/mod.rs`): all materialize `HeapValue::String(Arc::new(s))`
  or `HeapValue::HashMap(Arc::new(hm))` or `HeapValue::Array(...)` or a
  passthrough cloned `HeapValue`. Most are `String` arms — direct
  migration to `from_string_arc`. Some are nested-HeapValue arms —
  migrate to `from_heap_arc(Arc<HeapValue>)` (a new transitional
  constructor that takes an already-Arc'd HeapValue, avoiding the Box
  wrap; this is the bridge until full per-FieldType migration lands).
- **shape-vm** (~21 sites across `exceptions/mod.rs`,
  `control_flow/foreign_marshal.rs`, `builtins/special_ops.rs`,
  `state_builtins/core.rs`, `vm_impl/stack.rs`,
  `executor/objects/...`): mostly string-arm calls
  (`HeapValue::String(Arc::new(...))`). Migration is mechanical —
  callers materialize `Arc<String>` directly and call `from_string_arc`.
  One site materializes `HeapValue::Array(vmarray_from_vec(...))`;
  migrates to `from_typed_array` once the array data shape is normalized.

### Drop-path refactor (heap_mask bit dispatch → NativeKind dispatch)

**Current state**: `slot.drop_heap()` does `Box::from_raw(ptr as *mut HeapValue)`.
This works for `from_heap`-stored slots but is wrong for the new
`from_string_arc(Arc<String>)`-stored slots — those need
`Arc::from_raw(ptr as *const String)` then `drop`.

`drop_heap()` is also currently dead code: no Drop impl on
`HeapValue::TypedObject` calls it (verified by `rg "\.drop_heap\(\)"
crates/`; only the slot.rs test calls it). This is a pre-existing leak
in the `from_heap` path; cluster #1 closes it as part of the refactor.

**Required refactor**:

1. Add `impl Drop for HeapValue` (or for the TypedObject branch
   specifically) that, for the `TypedObject { schema_id, slots, heap_mask }`
   variant, looks up the schema, walks each heap-tagged slot, and
   dispatches drop based on the slot's `FieldType` projected to
   `NativeKind`:
   - `NativeKind::String` → `Arc::from_raw(slot.0 as *const String)` drop.
   - `NativeKind::Array(_)` → `Arc::from_raw(slot.0 as *const TypedArrayData)` drop.
   - `NativeKind::Object(_)` → `Arc::from_raw(slot.0 as *const HeapValue)` drop.
   - … one arm per heap-resident NativeKind.
2. The `heap_mask` bitmap is preserved as a fast "is this slot heap at
   all?" check (avoids per-non-heap-slot schema lookup); NativeKind
   dispatch happens only when the bit is set. ADR-005 §3 phrases this
   as "consults the slot's NativeKind … not just heap_mask" — both work
   together; mask is the gate, kind is the dispatch.
3. The same dispatch shape applies on `clone_heap` (which today is also
   wrong for Arc-stored slots).

**Predicted error-count delta from drop refactor**: 0 to +5 transient
(adding a Drop impl can surface unsoundness in places that relied on
`from_heap` not dropping). Real fix; not a regression. -2 to -3 net once
the Arc paths are correct (clone_heap and drop_heap stop being incorrect
for the migrated slots).

## Re-evaluated supervisor open questions

### Q1 — Cluster #1 input type for ε

**RESOLVED by ADR-005** (§Decision §1-2). Input type is the 12-variant
`TypedFieldValue` with `String(Arc<String>)` as the named exception and
`Heap(Arc<HeapValue>)` as the canonical heap arm. Not a new sum mirroring
ConcreteReturn; not ConcreteReturn itself. See ADR-005 §Decision §1-2.

### Q2 — Readback walker scope (still open; ADR-005 has implications)

**ADR-005 implications**: ADR's principles apply symmetrically. The
readback walker currently projects `HeapValue → HashMap<String,
ValueWord>`. Once ValueWord is gone, the output type must change to
something — either `HashMap<String, TypedFieldValue>` (mirroring the
input ABI) or `HashMap<String, Arc<HeapValue>>` (collapsing inline
scalars into HeapValue arms, which would require adding scalar arms to
HeapValue — out of scope per §Architectural shape rejection of α). The
**ADR-aligned answer** is `HashMap<String, TypedFieldValue>` — input
and output ABIs share the discriminator.

**Recommendation**: include the readback walker rewrite in cluster #1
scope. Otherwise the migration is asymmetric (construction is
TypedFieldValue, readback emits ValueWord-bits-from-raw — strict-typing
plan can't delete ValueWord while this remains). **Supervisor decision
needed**: in cluster #1 (1-2 weeks), or follow-up cluster (defers
ValueWord deletion gate).

### Q3 — shape-vm twin parallel-impls (still open; ADR-005 changes the answer)

**Updated**: ADR-005 expands the migration to ~21 `from_heap` sites in
shape-vm. The choice now is:

- **Move with cluster #1**: budget grows to 1-2 weeks (per ADR estimate);
  shape-vm cascade lands atomically; ADR-005's "uniform slot ABI between
  VM and JIT" §4 rule is cluster-#1 enforcing.
- **Move with shape-vm cascade**: cluster #1 lands shape-runtime side
  only, defers ~21 sites; shape-vm cascade includes them. Risk: the
  `from_heap` API has to remain pub on shape-value for the cascade
  duration.

**Recommendation**: **move with cluster #1**. ADR-005 §4 explicitly
requires uniform slot ABI between VM and JIT — splitting the migration
defers the uniformity guarantee.

### Q4 — multi_table/functions.rs (still open; ADR-005 doesn't change it)

`align_tables` / `correlation` use `typed_object_from_pairs` to build
their result; the IntrinsicFn signature is the cluster-#6 axis.
ADR-005 doesn't speak to IntrinsicFn shape. Recommendation unchanged:
migrate at the call-site as part of cluster #1 (signature flip), defer
the IntrinsicFn shape change to cluster #6.

### Q5 — Stale `use shape_value::{ValueWord, ValueWordExt}` (still open; ADR-005 doesn't change it)

Mechanical question; lands in same commit as signature flip per Rule F
application binder. ADR-005 doesn't speak to it.

## New questions surfaced by ADR-005

### Q6 — `from_heap_arc(Arc<HeapValue>)` transitional constructor?

ADR-005 mandates per-FieldType constructors but allows a transitional
period (slot.rs:54 comment). For sites that pass an already-`Arc<HeapValue>`
(e.g., `state_builtins/core.rs` cloning an existing heap value), should
cluster #1 add an `Arc<HeapValue>`-taking transitional constructor, or
force every site to project Arc → typed pointer at the call site?

The `Arc::into_raw(arc) as u64` form works for any Arc-stored heap value;
the dispatch at drop time still needs NativeKind. So a transitional
`from_heap_arc(Arc<HeapValue>)` is mechanically identical to
`from_string_arc` for the String case — the only difference is whether
the schema's NativeKind dispatch tag is "Heap" (catch-all) or "String"
(specific). **Supervisor decision needed**: does the catch-all "Heap"
NativeKind exist as a permanent slot-storage shape, or does ADR-005
forbid it (forcing every heap slot to know its specific NativeKind)?

ADR-005 §3 examples list `from_string_arc`, `from_typed_array`,
`from_typed_object` — implying the per-FieldType discipline. A catch-all
`from_heap_arc` would re-introduce a parallel-discriminator-shaped slot
storage. Recommend the supervisor say no; force per-FieldType migration.

### Q7 — Drop impl placement: HeapValue, TypedObject-only, or ValueSlot?

ADR-005 §3 says "Drop dispatch consults the slot's NativeKind". Three
plausible placements:

- **`impl Drop for HeapValue`** (whole sum). Needs to handle every
  variant; `TypedObject` arm walks `slots` + `heap_mask` + schema lookup.
- **`impl Drop for TypedObjectStorage`** (a new struct that owns the
  slots+heap_mask+schema_id, embedded in `HeapValue::TypedObject`'s
  variant). Tighter scope; ADR-005-clean.
- **`impl Drop for ValueSlot`** (per-slot). Requires the slot to
  self-describe its NativeKind, which contradicts the `#[repr(transparent)]
  struct ValueSlot(u64)` model (only 8 bytes; no room for a tag).
  Rejected.

**Supervisor decision needed**: Drop on HeapValue (option A) or on a
TypedObjectStorage struct (option B)? B is cleaner; A is fewer files
touched. Recommend B; it's more aligned with the long-term cluster-#7
work (where TypedObjectStorage may also become a typed pointer in its
own right).

### Q8 — Schema lookup at drop time: by id, or schema-pointer?

The current `HeapValue::TypedObject { schema_id: u64, slots, heap_mask }`
stores a schema-id, not a schema-pointer. NativeKind dispatch at drop
time needs the schema to project FieldType → NativeKind. Two options:

- **Schema lookup by id at drop time**: `lookup_schema_by_id(schema_id)`
  returns `Option<TypeSchema>`. Drop iterates and looks up — extra
  hashmap probe per drop; bounded.
- **Store `Arc<TypeSchema>` in the variant**: `TypedObject { schema:
  Arc<TypeSchema>, slots, heap_mask }`. Drop has direct access; no
  lookup. Costs 8 bytes per TypedObject + an Arc bump on construction.

ADR-005 doesn't specify. Recommend the hashmap-lookup option for the
initial cluster; promote to schema-pointer only if measurement shows
drop-path overhead (unlikely; drops are typically batched at end-of-scope).

### Q9 — `JIT FFI carrier` ABI: cluster #1 or cluster #7?

ADR-005 §1 explicitly names "JIT FFI carriers" as a layer subject to the
single-discriminator rule. Cluster #1 doesn't currently touch JIT — the
JIT FFI carrier work appears to be a separate cluster. Confirm:
JIT FFI carrier is **out of scope** for cluster #1, queued for a future
cluster (cluster #N where N > 7)?

### Q10 — Snapshot/wire reconstruction (ADR-005 §Forbidden)

ADR-005 §Forbidden ("Re-introducing `Box<HeapValue>` slot wrapping in any
new code path (e.g., snapshot/wire). Snapshot serializes the typed slot
bits + the schema; deserialization reconstructs the typed pointer. No
intermediate `HeapValue` materialization.") — does cluster #1 audit
the snapshot path (`crates/shape-runtime/src/snapshot.rs`,
`crates/shape-runtime/src/wire_conversion.rs`) for ADR-conformance, or
is that a separate cluster? `snapshot.rs:341` references `heap_mask`;
`wire_conversion.rs:162` destructures it. Recommend: out of scope for
cluster #1 (separate audit), but ADR-005 marker comment added at both
sites pointing forward to that audit.

## Migrator dispatch checklist

### Files the migrator will edit

(Line ranges are approximate; see file contents at branch tip
`bulldozer-strictly-typed @ ed72442` for exact positions.)

**shape-runtime crate**:

1. `crates/shape-runtime/src/type_schema/mod.rs`
   - Define `TypedFieldValue` (12-variant per ADR-005). Add `// ADR-005`
     marker on the enum, with the `String` exception inline-justified.
   - Rewrite `typed_object_from_pairs` body: schema lookup → for each
     `FieldDef`, read matching `TypedFieldValue` arm → project to
     `ValueSlot` via per-FieldType constructor → compute heap_mask.
   - Flip return type: `Arc<HeapValue>` (not `ValueWord`).
   - Rewrite `typed_object_to_hashmap_nb`: return type
     `HashMap<String, TypedFieldValue>` (Q2 resolution) or **defer**
     to follow-up cluster (Q2 deferral).
   - Delete `use shape_value::{ValueWord, ValueWordExt}` import (Q5).
2. `crates/shape-runtime/src/stdlib/json.rs`
   - 5 `from_heap` sites → migrate to `from_string_arc` or
     `from_typed_array` per arm.
3. `crates/shape-runtime/src/const_eval.rs`
   - Caller-side: build `TypedFieldValue` instead of `ValueWord` pairs.
   - Readback callers (lines 435, 549) match new return type if Q2 in scope.
4. `crates/shape-runtime/src/schema_cache.rs`, `multiple_testing.rs`,
   `multi_table/functions.rs` — caller-side migration.

**shape-value crate**:

5. `crates/shape-value/src/slot.rs`
   - Add `from_string_arc`, `from_typed_array`, `from_typed_object`,
     `from_decimal`, `from_timestamp`, `from_hashmap` (one per
     `FieldType` heap-resident arm).
   - `// ADR-005` marker on each new constructor.
   - Mark `from_heap` deprecated; do not delete (transitional).
6. `crates/shape-value/src/heap_value.rs`
   - Add `impl Drop for HeapValue` (or for `TypedObjectStorage` —
     resolves Q7) that walks heap_mask + schema-NativeKind for
     `TypedObject` arm.
   - The `Clone` impl (heap_value.rs:989-997) similarly needs NativeKind-
     dispatch on slot clone.

**shape-vm crate** (Q3 = "in scope"):

7. `crates/shape-vm/src/executor/exceptions/mod.rs` (~7 sites)
8. `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs` (~5 sites)
9. `crates/shape-vm/src/executor/builtins/special_ops.rs` (~6 sites)
10. `crates/shape-vm/src/executor/state_builtins/core.rs` (2 sites)
11. `crates/shape-vm/src/executor/vm_impl/stack.rs` (1 site)

### Order of edits

1. Define `TypedFieldValue` in `type_schema/mod.rs` (no callers yet —
   doesn't break anything).
2. Add per-FieldType constructors on `ValueSlot` in `slot.rs` (purely
   additive — doesn't break anything).
3. Add `Drop` impl resolving Q7 (NativeKind-dispatch) — surfaces ~0-5
   transient errors at sites that relied on no-drop behavior; fix in place.
4. Migrate `typed_object_from_pairs` body to use `TypedFieldValue` input,
   per-FieldType constructors, NativeKind-tagged storage. Flip return type
   to `Arc<HeapValue>`.
5. Migrate caller sites in shape-runtime (const_eval, schema_cache,
   multiple_testing, multi_table — ~6 files).
6. (If Q2 in scope) Rewrite `typed_object_to_hashmap_nb` to schema-driven
   readback returning `HashMap<String, TypedFieldValue>`.
7. (If Q3 in scope) Migrate shape-vm `from_heap` sites to per-FieldType
   constructors (~21 sites across 5 files).
8. Delete stale `use shape_value::{ValueWord, ValueWordExt}` import.
9. (Optional, ADR-005 hygiene) Add `// ADR-005` marker comments at new
   call sites: `TypedFieldValue` definition, every new
   `ValueSlot::from_*` constructor, the `Drop` impl, and every migrated
   caller site (one comment per file is sufficient).

### Per-edit error-count delta predictions (bounded)

(In shape-runtime --lib unless noted.)

| Step | Predicted delta | Notes |
|---|---|---|
| 1. Define TypedFieldValue | 0 | additive |
| 2. Add per-FieldType constructors | 0 | additive |
| 3. Add Drop impl (Q7) | 0 to +5 | may surface latent unsoundness |
| 4. Migrate typed_object_from_pairs body | -3 to -8 | clears stale ValueWord cites |
| 5. Migrate shape-runtime callers | 0 to +2 | transient if some lag |
| 6. Readback rewrite (Q2) | -2 to -4 | clears clone_from_bits cite |
| 7. Migrate shape-vm callers (Q3) | -5 to -15 (in shape-vm --lib) | many sites; high uncertainty |
| 8. Delete stale imports | 0 | mechanical |
| 9. ADR-005 markers | 0 | doc-only |

**Net**: -10 to -27 across both crates if Q2 + Q3 in scope; -3 to -8 if
both deferred (cluster #1 partial).

### Supervisor decisions still pending (block which steps)

| Question | Blocks | Default if no decision |
|---|---|---|
| Q2 (readback walker) | step 6 | defer; cluster #1 ships construction-only |
| Q3 (shape-vm cascade) | step 7 | defer; cluster #1 ships shape-runtime-only |
| Q6 (from_heap_arc transitional?) | step 4 | reject; force per-FieldType |
| Q7 (Drop placement) | step 3 | use HeapValue Drop (option A) |
| Q8 (schema lookup at drop) | step 3 | hashmap lookup by id |
| Q9 (JIT FFI carrier) | none (cluster boundary) | out of scope |
| Q10 (snapshot audit) | none (cluster boundary) | out of scope; mark forward |
