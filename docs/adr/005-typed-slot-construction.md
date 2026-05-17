# ADR-005: Typed Slot Construction Discipline

## Status

Accepted (2026-05-08)

Supersedes the implicit "wrap heap data in `HeapValue`" pattern present in
the v1 typed-runtime path. Companion to the strict-typing plan
(`~/.claude/plans/stop-native-vs-tagged-tax.md`); the strict-typing plan
removes the *dynamic* dispatch path, this ADR specifies how the *typed*
path is shaped on the way through.

## Context

The strict-typing plan deletes `ValueWord` and the dynamic-dispatch
fallback. Once that lands, the remaining performance and complexity gap
is **typed wrapping at boundaries** — every layer that takes typed values
in or out has accumulated its own discriminator, even when an
authoritative discriminator already exists below.

### Concrete instances of the drift

- `HeapValue` is a 18-arm sum type with `HeapKind` as canonical
  discriminator (`crates/shape-value/src/heap_variants.rs:56`).
- `ConcreteReturn` (the marshal-layer return type at
  `crates/shape-runtime/src/typed_module_exports.rs:49`) re-encodes a
  subset of HeapKind as top-level variants (`ArrayHeapValue`,
  `HashMapStringHeapValue`, `JsonValue`, `OpaqueTypedObject`, ...).
- `NativeKind` (`crates/shape-value/src/native_kind.rs:32`) carries an
  explicit watchlist comment forbidding parametric variants like
  `NativeKind::Result(...)`/`Option(...)` — the same discipline as this
  ADR, applied at the proof layer.
- The proposed `TypedFieldValue` for cluster #1's
  `typed_object_from_pairs` migration initially had separate
  `Array(Arc<HeapValue>)` and `Object(Arc<HeapValue>)` variants — the same
  parallel-discriminator pattern at a third layer.
- `ValueSlot::from_heap` (`crates/shape-value/src/slot.rs:54`) wraps any
  heap value in `Box<HeapValue>` regardless of the field's
  statically-known type. A string field costs 2 heap allocations:
  `Arc<String>` (already exists) → `HeapValue::String(Arc<String>)`
  (allocation 1) → `Box<HeapValue::String(Arc<String>)>` (allocation 2).
  The slot's NativeKind is known at construction time; the wrapping is
  redundant.

### The defection pattern

Every parallel discriminator we added has eventually drifted. Variants
get added to one layer but not another. Future maintainers face "should
I add this variant to ConcreteReturn or HeapValue?" and the answer is
"both" or "neither" depending on context they don't have. The N9
close-out (`docs/defections.md`, 2026-05-07,
`type_schema-slot-construction-cleanup`) explicitly named
"smaller-subset enum of an existing discriminator" as a defection
attractor on a par with the W-series ValueWord renames.

### What this ADR is about

This ADR establishes **one discriminator per concept**, **typed slot
storage** (no transitional `Box<HeapValue>` wrapping), and **a uniform
slot ABI between VM and JIT**. It also names the one explicit exception
— `String` at the input-carrier layer — and bounds it.

## Decision

### 1. Single-discriminator discipline

`HeapValue` is the single discriminator for heap-resident values. Layers
above HeapValue take `Arc<HeapValue>` and dispatch on `HeapValue::kind()`
when they need kind information. **No layer above HeapValue may
introduce a sum type whose variants project 1:1 to HeapKind variants.**

This forbids:

- `enum X { Array(Arc<HeapValue>), Object(Arc<HeapValue>), HashMap(Arc<HeapValue>), ... }`
- `enum X { ArrayHeapValue(...), HashMapStringHeapValue(...), Json(...), DataTable(...), ... }`
  pattern as currently present in `ConcreteReturn` (cluster #7 cleanup
  target).
- Per-`HeapKind` parametric variants on `NativeKind`
  (already independently forbidden via the existing watchlist comment
  at `native_kind.rs:88-96`).

The principle applies at **every layer above HeapValue**: function-return
ABI, field-input ABI, marshal helpers, JIT FFI carriers, snapshot
serialization. Any layer that needs kind dispatch reads
`HeapValue::kind()`.

### 2. The `String` exception, named and bounded

The input-carrier layer (`TypedFieldValue` for object construction; the
analogous role for any future heap-input ABI) **may** carry a
`String(Arc<String>)` variant alongside the canonical
`Heap(Arc<HeapValue>)` variant.

Justification (the only justification accepted):

- Strings are the most common heap type by an order of magnitude in
  measured stdlib parser output (json/yaml/toml/msgpack).
- Routing strings through `Heap(Arc::new(HeapValue::String(arc_string)))`
  costs one `Arc::new` allocation per string field at construction. For
  parsed JSON documents, this is N allocations per document where N is
  the field count.
- The wrapping in `HeapValue::String(Arc<String>)` is purely a tagging
  mechanism with no semantic value at this layer — the slot's NativeKind
  already says "string."

Discipline:

- The exception is named explicitly in this ADR. It does NOT generalize
  to any other heap variant by analogy.
- A second exception requires its own ADR-level justification with
  measurement. "X is also common" is not sufficient — strings are common
  *and* the wrapping is purely tagging *and* the unwrap path is
  one-line.
- The `String` variant carries `Arc<String>` (refcounted shared
  ownership). Not `String` (owned), not `&str` (borrowed), not
  `StringId` (interned). `Arc<String>` is the runtime carrier; interning
  layers (Layer 3 below) coexist by deduplicating the Arc-inner.

### 3. Typed slot storage

`ValueSlot` storage of heap data must store typed pointers directly, not
`Box<HeapValue>` wrappers.

Concrete shape (incremental, additive — `ValueSlot::from_heap` remains
as a transitional API during cluster #1 migration):

```rust
impl ValueSlot {
    pub fn from_string_arc(s: Arc<String>) -> Self {
        Self(Arc::into_raw(s) as u64)
    }
    pub fn from_typed_array(a: Arc<TypedArrayData>) -> Self {
        Self(Arc::into_raw(a) as u64)
    }
    pub fn from_typed_object(o: Arc<TypedObject>) -> Self {
        Self(Arc::into_raw(o) as u64)
    }
    // ... per-FieldType, mirroring FieldType's variant set
}
```

Drop dispatch consults the slot's NativeKind (derived from the schema's
FieldType), not just `heap_mask`. The current `heap_mask: u64` (one bit
per slot, "is this slot heap?") is insufficient when different heap
slots need different drop paths (`Arc::decrement_strong_count` vs
`Box::from_raw`). Cluster #1 migrates the drop path to NativeKind
dispatch.

### 4. Uniform slot ABI between VM and JIT

A slot's bit pattern is interpreted identically in VM and JIT.

- VM and JIT read/write slots via the same NativeKind interpretation.
- No conversion happens at the VM↔JIT boundary, including OSR entries
  and deopt exits. `Cranelift` emits `load`/`store` of the appropriate
  width for each NativeKind directly.
- Inline caches in the JIT key on NativeKind, not on
  HeapKind-via-runtime-probe. The schema/type system supplies the kind
  at compile time; runtime probes are debug-only sanity checks.

This is the principle the strict-typing plan deletes `ValueWord` for.
Reintroducing wrapping at any layer (slot, marshal, snapshot, FFI)
resurrects the cost in a different place. It is forbidden by the same
list of patterns CLAUDE.md "Forbidden Patterns" enumerates.

### 5. Future optimization layers preserve the ABI

Each of the following ships independently without rework on the layers
below. The ABI established here is designed to support them:

**Layer 3 — runtime interning.** A global `InternPool` maps
`StringId(u32)` ↔ `Arc<String>`. Hot strings (literals, repeated
identifiers, dict keys) deduplicate to a single `Arc<String>`. Slot ABI
unchanged: the slot still stores an `Arc<String>` raw pointer; it just
points to a pool entry. Equality of interned strings is pointer
equality. The `StringId` already used by bytecode operands
(`crates/shape-value/src/ids.rs:60-79`) becomes the runtime pool key
under this layer.

**Layer 4 — Small String Optimization (SSO).** Strings ≤7 bytes stored
inline in the slot. A tag (low bit, high bit, or NativeKind variant —
to be decided when this lands) discriminates inline vs pointer. The
existing `Arc<String>` path remains valid for strings >7 bytes. Cranelift
emits a tag-check + branch per read/write; inline-cache specialization
collapses the branch on monomorphic call sites.

**Layer 5 — multiple representations.** `ConsString` (lazy concat),
`SlicedString` (substring view), `ExternalString` (FFI-owned).
Speculative; introduce only when measured concat/substring workloads
justify the dispatch cost.

Each layer is independently cancelable. None changes the slot's basic
size (8 bytes) or the VM/JIT shared interpretation.

## Consequences

### Enables

- Cluster #1's `TypedFieldValue` lands with the minimal-discriminator
  shape: 12 variants (10 width-typed primitive scalars + `Bool`,
  `String(Arc<String>)`, `Heap(Arc<HeapValue>)`).
- Cluster #7 (named here for the first time) folds `ConcreteReturn`'s
  heap-arm variants into a single `Heap(Arc<HeapValue>)` arm. The
  function-return ABI gets the same discipline as the field-input ABI.
- Future `InternPool` (Layer 3) work has a stable target. Slot ABI is
  Arc<String> raw pointer; interning deduplicates the inner.
- Future SSO (Layer 4) work has a stable target. Tag bit extension; slot
  ABI extends without breaking Layer 1 callers.
- JIT codegen simplifies: typed slot ops become direct
  `load`/`store` of the correct width. No wrap/unwrap. Inline caches key
  on NativeKind. Deopt boundaries cost zero ABI conversion.

### Costs

- Cluster #1's scope grows from "change input carrier for
  `typed_object_from_pairs`" to "establish typed-slot-construction
  discipline end-to-end with strings as the load-bearing example." 1-2
  weeks rather than 4-6 days.
- Existing call sites that materialize `HeapValue::String(arc)` and
  pass through `ValueSlot::from_heap(...)` migrate to the typed
  per-FieldType path. ~10 cross-crate caller sites for the
  type_schema/object_creation surface.
- The slot-level drop path moves from `heap_mask` bit dispatch to
  NativeKind dispatch. This is a real refactor of TypedObject's
  Drop impl. Bounded — same cluster.
- `ValueSlot::from_heap(value: HeapValue)` becomes a transitional API.
  Marked deprecated in cluster #1's commit; deleted when the last caller
  migrates.

### Forbidden under this ADR

- Adding new heap-arm variants to `ConcreteReturn` while the cluster #7
  cleanup is pending. Stop and surface; do not extend the parallel
  discriminator further.
- New slot constructors that accept `HeapValue` by value. Typed
  per-FieldType constructors only.
- Per-`HeapKind` variants on input/output carriers (`TypedFieldValue`,
  any future `TypedFieldValueOut`, marshal helpers). The `String`
  exception is the only allowed exception.
- "RawBits", "ValueBits", "shim", "bridge", "boundary helper",
  "compatibility layer" renames of these patterns. CLAUDE.md "Renames
  to refuse on sight" extends to slot-construction layer.
- Re-introducing `Box<HeapValue>` slot wrapping in any new code path
  (e.g., snapshot/wire). Snapshot serializes the typed slot bits + the
  schema; deserialization reconstructs the typed pointer. No
  intermediate `HeapValue` materialization.

## Implementation roadmap

Layered, each independently shippable:

| Layer | Cluster | Scope | Status |
|---|---|---|---|
| 1 | Cluster #1 | `TypedFieldValue` API + per-FieldType `ValueSlot` constructors + NativeKind-driven drop | Active (this ADR is the architectural anchor) |
| 2 | Cluster #7 | `ConcreteReturn` heap-arm folding into single `Heap` variant | Named here; surface-and-decide round-trip required before dispatch |
| 3 | Future cluster | Runtime `InternPool` (`StringId(u32)` ↔ `Arc<String>`) | Not started |
| 4 | Future cluster | Small String Optimization (≤7-byte inline) | Not started |
| 5 | Speculative | ConsString / SlicedString / ExternalString | Speculative; do not start without measured justification |

## Visibility / drift prevention

This ADR is the canonical source. To prevent drift, four mirroring
mechanisms point back at it:

1. **Code comments at load-bearing sites.** Each file that defines or
   primarily edits a discriminator at a layer above HeapValue carries
   a short comment block referencing this ADR by ID. Files in scope:
   - `crates/shape-value/src/heap_variants.rs` — at the `HeapValue` /
     `HeapKind` definitions, naming HeapValue as the single
     discriminator.
   - `crates/shape-value/src/slot.rs` — at `ValueSlot::from_heap`,
     marking it transitional.
   - `crates/shape-value/src/native_kind.rs` — extends the existing
     watchlist comment to point at this ADR.
   - `crates/shape-runtime/src/typed_module_exports.rs` — at the
     `ConcreteReturn` definition, naming the cluster #7 cleanup target.
   - `crates/shape-runtime/src/json_value.rs` — clarifies the
     parser-intermediate role; not a runtime storage type.
   - Future: `TypedFieldValue` definition, with the `String` exception
     justified inline.
2. **CLAUDE.md "Forbidden Patterns" addition.** A short
   "Single-discriminator discipline" subsection referencing this ADR.
3. **`docs/defections.md` cross-reference.** A new entry recording the
   N9-cluster-#1 derivation that surfaced this principle.
4. **Sentinel test (optional).** A Rust test that asserts certain enums
   have a known variant count. Adding a variant requires updating the
   test, which forces the editor to land in the test file and read the
   ADR pointer.

## References

- Strict-typing plan: `~/.claude/plans/stop-native-vs-tagged-tax.md`
- Strict-typed baseline: `docs/strictly-typed-baseline.md`
- N9 cluster close (parallel-discriminator pattern named):
  `docs/defections.md`, 2026-05-07,
  `type_schema-slot-construction-cleanup workstream`
- Cluster #1 audit: `docs/cluster-audits/cluster-1-type-schema.md`
- Forbidden patterns: `CLAUDE.md` "Forbidden Patterns" section
- NativeKind watchlist: `crates/shape-value/src/native_kind.rs:88-96`
- `StringId` pool TODO: `crates/shape-value/src/ids.rs:74-79`
