# ADR-006: Value & Memory Model

## Status

Accepted (2026-05-08)

Supersedes ADR-005 in scope. ADR-005's "single-discriminator discipline"
principle is preserved verbatim and extended; ADR-005's `from_typed_array(Arc<TypedArrayData>)`-
style typed-pointer examples are corrected to match the actual `HeapValue`
layout this ADR specifies.

Companion to the strict-typing plan
(`~/.claude/plans/stop-native-vs-tagged-tax.md`); the strict-typing plan
removed dynamic dispatch from the runtime, this ADR specifies the typed
runtime that takes its place.

## Context

The strict-typing bulldozer + ADR-005 cluster #1 work surfaced an
architectural gap: ADR-005 §3 specified per-FieldType typed slot
constructors as if the heap layout already supported them — it didn't.
The migrator hit the gap and rationalized a workaround
(`from_heap_arc(Arc<HeapValue>)`), violating Q6 supervisor ruling. Rather
than patch the symptom, the supervisor paused all migration and ordered a
top-down redesign of the value/memory/lifetime/GC/runtime model.

Three parallel research surveys (`docs/research/01-ownership-gc.md`,
`02-layout-runtime.md`, `03-strings-arrays.md`) and two design alternatives
(`docs/adr/006-DRAFT-alternative-B.md`, `006-DRAFT-alternative-C.md`) were
produced to inform this ADR.

This ADR is the architectural anchor for all subsequent runtime work. It
supersedes the open clusters #1 / #5 / #6 / #7 that derived from ADR-005
and reframes them as phases of a coherent runtime rebuild.

## Decision summary

Shape's runtime:

1. **Tag-free typed slots.** No `ValueWord`, no NaN-boxing, no per-value
   tag bits. The strict type system supplies the kind.
2. **Three explicit binding forms with a smart-default.** `let` /
   `let mut` are precise (Rust-shaped, single-owner); `var` is inferred
   among Direct / UniqueHeap / SharedCow / SharedAtomic / SharedAtomicMut
   based on observed usage, surfaced as inlay hints.
3. **Existing Rust-shaped lifetime infrastructure is reused, not
   replaced.** The MIR storage-planning pass, the borrow solver, ref-escape
   analysis, and the `BindingStorageClass`/`BindingSemantics` vocabulary
   are extended (one new `SharedAtomic*` variant, `var`-specific inference)
   rather than rebuilt as a new modal-types subsystem.
4. **Refcounting is opt-in by escape, not by mutability.** `let mut x = 0`
   is a stack-resident mutable scalar, not `Arc<Mutex<int>>`. RC is reached
   only when escape (closure capture, cross-task share, store-into-shared-
   container) requires it.
5. **HeapValue payloads carry typed `Arc<T>` directly.** The slot stores
   typed pointers; `Box<HeapValue>` wrapping is removed except as a
   transitional API marked deprecated.
6. **Cranelift JIT, uniform slot ABI across VM and JIT tiers.** No
   conversion at the boundary; introspection metadata is tier-dependent
   (VM keeps; JIT may drop).
7. **LLM-Structured Diagnostic Schema (LSDS) is the primary compiler
   output.** Renderers (terminal, LSP, MCP) consume LSDS rather than
   produce it. Errors carry expected/found type witnesses, suggested-fix
   diffs, and bounded agent-context windows.
8. **Polyglot Value Lattice (PVL) conditional on audit.** A small audit
   determines whether the existing per-language marshal layers (PyO3,
   deno_core, `extern C`) genuinely share enough structure to justify a
   unified protocol. If yes, implement; if no, keep per-language adapters
   and document the rejection.
9. **Permission-aware JIT speculation (PES) post-JIT, behind feature flag.**
   Tier-2 JIT may specialize on observed permission state; deopt on
   change. Gated until ≥3× measured speedup on permission-heavy I/O loops.
10. **Compile-Time AI Optimization Notes (CT-AION) as v2 capability.**
    Opt-in per package; advisor outputs hashed into content addresses for
    reproducibility. Off by default.

The combination targets best-in-class ergonomics (Python-easy-entry via
`var`; precise control via `let`/`let mut`), best-in-class perf (tag-free
slots, RC-only-on-escape, Cranelift JIT, uniform ABI), and best-in-class
distribution (content-addressed everything, signed manifests, two-tier
permissions) without rebuilding the existing analysis subsystem.

## 1. Bindings — `let`, `let mut`, `var`

### 1.1 `let` and `let mut` — explicit, Rust-shaped

`let` binds an immutable, single-owner value. `let mut` binds a mutable,
single-owner value. Both use the existing borrow-checked aliasing,
ref-escape analysis, and storage-planning pass. **No new analysis is
written for `let`/`let mut`.**

The grammar already supports both (`shape.pest:760-771`,
`var_mut_modifier`); the implementation needs to honor it consistently.

Storage class for `let`/`let mut` follows existing rules:

- Scalar (`int`, `number`, `bool`, ...) → `Direct` (stack).
- Heap-resident (`string`, `Vec<T>`, struct, ...) → `UniqueHeap` (single
  owner, no refcount).
- Captured by non-escaping closure → `LocalMutablePtr` (stack with typed
  capture pointer).
- Borrowed via `&` / `&mut` references → `Reference`.
- Escapes to `Arc<T>` only when the type system demands sharing
  *explicitly* (e.g., the user wrote `Arc<T>`).

The grammar's `move` / `clone` ownership modifiers (line 769-770) remain
the explicit user-facing way to control transfer at the binding RHS.

**Errors for `let`/`let mut`** point at borrow / lifetime / escape
violations using the existing solver vocabulary (`B0013`,
`B0014`, `BorrowError::*`).

### 1.2 `var` — smart inference

`var x = expr` defers storage-class choice to the compiler. The compiler
walks `x`'s use-graph and picks the lightest policy that proves safe:

| Observed usage of `x` | Inferred class | Runtime shape |
|---|---|---|
| Bound, read; never mutated, never escapes | `Direct` (immutable) | Stack, no allocation. Equivalent to `let`. |
| Mutated within owning scope; doesn't escape | `Direct` (mutable) | Stack mutable. Equivalent to `let mut`. |
| Escapes via closure / store / return; immutable | `UniqueHeap` (if last-use detectable) or `SharedCow` | Heap pointer; refcount only when sharing is genuine. |
| Mutated AND shared (single-thread only) | `SharedCow` | Refcounted CoW. Existing class. |
| Read-shared across thread/task boundary | `SharedAtomic` (NEW) | Atomic-refcounted, no lock. `T: Send + Sync` proven. |
| Mutated AND shared across thread/task boundary | `SharedAtomicMut` (NEW) | Atomic-refcounted + lock. `Arc<Mutex<T>>` shape. |

**The two `Shared*Atomic*` variants are the only additions to
`BindingStorageClass`.** Everything else maps to existing classes.

**Inference is conservative.** When usage can't be proven tighter, the
compiler picks the most permissive policy that's still correct (typically
`SharedAtomicMut` for cross-task mutation; `SharedCow` for in-scope
sharing). The chosen class is **always shown as an inlay hint** so users
can see and refactor.

**`var` falls back, never fails.** A `var` binding never produces a
borrow-checker error — it always finds a class that works, even if the
class is heavy. `let`/`let mut` are where the borrow checker is strict.

### 1.3 Visibility — inlay hints + LSDS suggestions

Every `var` binding emits an LSP inlay hint immediately after the binding
keyword:

```shape
var counter = 0          // ⟦Direct (stack-mutable)⟧
var config = parse()     // ⟦UniqueHeap⟧
var shared = Vec.new()   // ⟦SharedCow⟧ ← captured by closure on line 12
var queue = Channel()    // ⟦SharedAtomicMut⟧ ← shared across spawn on line 18
```

When the inferred class is heavier than the user might expect, the
compiler emits an LSDS *suggestion* (not error) with the cause and a
proposed refactor:

```
suggestion: var "shared" inferred as SharedCow due to closure capture at line 12
   if performance-critical, consider:
     - making the closure non-escaping
     - using an explicit `let` with borrow at the call site
   override: `var shared: SharedCow Vec<T> = ...`
```

### 1.4 Override syntax

For cases where the inference is wrong or undesirable, users can pin the
class:

```shape
var x: SharedCow Vec<int> = ...    // explicit class hint (no inference change)
var y: Direct int = 0              // pin to stack even if compiler would heap-allocate
```

The override syntax extends the existing type-annotation grammar
(`shape.pest:760` `(":" ~ type_annotation)?`) — the type-annotation
position accepts an optional class qualifier prefix.

## 2. Value representation

### 2.1 Tag-free typed slots

`ValueSlot` remains an 8-byte raw container (`#[repr(transparent)] struct
ValueSlot(u64)`). The interpretation is supplied by the schema's
`FieldType` and the surrounding analysis's `NativeKind` — never by per-
slot tag bits.

ADR-005 §1 single-discriminator discipline is binding: `HeapValue` is the
canonical heap discriminator. Layers above HeapValue take `Arc<HeapValue>`
and dispatch on `HeapValue::kind()` when kind information is needed at
runtime.

### 2.2 The `String` exception remains

`TypedFieldValue::String(Arc<String>)` is the named, bounded exception
from ADR-005 §2. Justified by measured allocation cost on the most-common
heap type. Preserved here.

### 2.3 HeapValue payloads — typed Arc

This is the layout correction over ADR-005 §3. Each `HeapValue` variant
that previously carried inline payload now carries `Arc<TypedT>`:

```rust
pub enum HeapValue {
    String(Arc<String>),                    // existing — preserved
    TypedArray(Arc<TypedArrayData>),        // CHANGED: was inline
    TypedObject(Arc<TypedObjectStorage>),   // CHANGED: was struct variant
    HashMap(Arc<HashMapData>),              // existing — preserved
    Decimal(Arc<rust_decimal::Decimal>),    // CHANGED: was inline
    BigInt(Arc<i64>),                       // ... etc per Kind
    // ... other variants similarly
}
```

The Arc wrapping is per-variant payload, not over the entire enum. The
slot stores a raw pointer to the inner T (not to HeapValue) — drop
dispatch consults the `NativeKind` from the schema/type-system, not the
HeapValue tag.

`TypedObjectStorage` is a new struct holding `{schema_id, slots,
heap_mask}` — the fields previously inline in `HeapValue::TypedObject`.

### 2.4 ValueSlot per-FieldType constructors

```rust
impl ValueSlot {
    pub fn from_string_arc(s: Arc<String>) -> Self {
        Self(Arc::into_raw(s) as u64)
    }
    pub fn from_typed_array(a: Arc<TypedArrayData>) -> Self {
        Self(Arc::into_raw(a) as u64)
    }
    pub fn from_typed_object(o: Arc<TypedObjectStorage>) -> Self {
        Self(Arc::into_raw(o) as u64)
    }
    // ... per-FieldType, mirroring FieldType variant set
}
```

`from_heap(value: HeapValue)` is `#[deprecated]` transitional; deleted
when the last caller migrates.

### 2.5 Drop discipline

`TypedObjectStorage` carries `schema_id`. Its `Drop` impl looks up the
schema, walks `heap_mask`, and dispatches on each field's `NativeKind`
to call the matching `Arc::decrement_strong_count` or no-op for scalars.

Schema lookup is by id at drop time (per Q8 ruling, ADR-005 follow-up).
HashMap probe; promote to `Arc<TypeSchema>` only if profiling shows
measurable overhead.

### 2.6 The `from_heap_arc` rejection stands

Per Q6 supervisor ruling: no catch-all `from_heap_arc(Arc<HeapValue>)`
constructor. Per-FieldType constructors only. The migrator-cluster-1
commits that introduced `from_heap_arc` will be partially salvaged
(steps 1, 4, 6, 8, 9 keep their core logic; step 2 is rewritten without
`from_heap_arc`).

### 2.7 Caller-side runtime-value abstraction — `KindedSlot`

Phase 1.A established `ValueSlot` as the slot foundation, but the
deletion of `ValueWord` (commit `fdd5205`, before Phase 1.A) left a
caller-side gap: ~95 sites across `crates/shape-runtime/` use values
where the `NativeKind` is not statically available locally. The deleted
`ValueWord` carried its kind in tag bits; `ValueSlot` does not. Phase
1.B's audit (2026-05-08, `/tmp/phase-1b-audit.md`) ground-truthed the
shape of this gap across 60 files and ~658 references.

**Decision (Q7 ruling):** Introduce `KindedSlot` in `shape-value`:

```rust
// crates/shape-value/src/slot.rs (new addition; ValueSlot stays 8 bytes)
#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct KindedSlot {
    pub slot: ValueSlot,
    pub kind: NativeKind,
}
```

**This is a carrier, not a discriminator.** It does not violate
ADR-005 §1 (single-discriminator discipline) because:

- `KindedSlot` is a `struct`, not a sum type. ADR-005 §1 forbids
  parallel *sum types* whose variants project 1:1 to `HeapKind`.
- `NativeKind` is a broader taxonomy than `HeapKind` — it includes
  raw scalars (`Int64`, `Float64`, `Bool`) with no `HeapValue` arm.
  The kind→heap mapping is many-to-one (heap arms only), not 1:1.
- The struct introduces no new dispatch surface; `KindedSlot::kind`
  is the *same* `NativeKind` already tracked elsewhere in the type
  system. It co-locates information already present in the data
  model.

`KindedSlot` carries explicit `Drop` and `Clone` impls that dispatch
on `kind` to handle heap retain/release. Without these, `Vec<KindedSlot>`
push/pop/clone would alias-copy heap pointers — the WB2.4 / WB2.5 bug
class the typed-slot ABI was designed to prevent. The reference
discipline pattern lives at `module_exports.rs:42-88` (`FrameInfo`)
and `event_queue.rs:226-243` (`Cache::set/remove`); both must preserve
their refcount semantics across the `Vec<ValueWord> → Vec<KindedSlot>`
migration.

#### 2.7.1 Per-site usage policy

Three site shapes, applied per call site (audit-grounded counts in
parentheses):

1. **STATIC_KIND** (~30 files dominated by this shape, ~400 sites).
   Use `ValueSlot` directly. `NativeKind` is statically determined by
   the surrounding `FieldType` / schema / typed dispatch. Per-FieldType
   `ValueSlot::from_*` constructors give kind by construction. **No
   `KindedSlot`.** Examples: `content_builders.rs`, `content_methods.rs`,
   `stdlib/{msgpack,toml,yaml}_module.rs`, `multiple_testing.rs`,
   `module_exports_tests.rs`, `intrinsics/math.rs` (typed entries).

2. **GENERIC_CARRIER — single value** (~6 files, ~15 sites). Use
   `KindedSlot`. Examples: `Variable.value: KindedSlot`,
   `Export::Value(KindedSlot)`, `OutputAdapter::print -> KindedSlot`,
   `const_eval::eval -> Result<KindedSlot>`. The static-kind from
   `Literal::*` arms is preserved by construction at the boundary.

3. **GENERIC_CARRIER — vector storage** (~3 files, ~25 sites driving
   ~80% of the cluster). Use `Vec<KindedSlot>`. Examples:
   `ModuleBindingRegistry::values: Vec<KindedSlot>`,
   `FrameInfo::{locals,upvalues,args}: Vec<KindedSlot>`,
   `SuspensionState::{saved_locals,saved_stack}: Vec<KindedSlot>`.
   Pre-existing parallel arrays (`is_const: Vec<bool>`,
   `index_to_name: Vec<String>`) stay — those track unrelated
   metadata.

4. **Dispatch slices** (~3 files: `intrinsics/mod.rs`, `module_exports.rs`,
   `stdlib_time.rs`). Use `&[KindedSlot]`. Examples:
   `IntrinsicFn = fn(&[KindedSlot], &mut ExecutionContext) -> Result<KindedSlot>`,
   `ModuleFn = Arc<dyn for<'ctx> Fn(&[KindedSlot], &ModuleContext<'ctx>) -> Result<KindedSlot, String> + Send + Sync>`.

#### 2.7.2 Forbidden uses

- Do not use `KindedSlot` where `NativeKind` is statically known
  (would re-introduce kind-tag latency the slot ABI just removed).
- Do not introduce `KindedSlot` *variants* (sum-type form).
  `KindedSlot` is a carrier, not a discriminator.
- Do not migrate `ValueSlot` itself to a 16-byte form. ADR-006 §2.1
  fixes the slot at 8 bytes; the runtime-value carrier is a separate
  type.
- Do not let `KindedSlot` leak into the typed VM↔JIT slot ABI
  (`docs/runtime-v2-spec.md`). The hot stack/JIT path stays
  `ValueSlot`-only with kind threaded through opcode operands and
  per-frame slot-kind metadata. `KindedSlot` is a *runtime-tier*
  carrier (`shape-runtime` module bindings, frame snapshots,
  intrinsic dispatch) — not a VM stack carrier.

#### 2.7.3 Migration roadmap interaction

Phase 1.B's caller migration (per §12) targets:
- 9 cleanup-only files (pure `use` removal, zero non-trivial uses).
- 16 DEPRECATED-comment files (no functional change, comment cleanup).
- ~30 STATIC_KIND-dominated files (mechanical sed-shape rewrite).
- ~9 files with real GENERIC_CARRIER sites needing `KindedSlot`
  introduction. Top 3: `module_bindings.rs`,
  `event_queue.rs`, `context/variables.rs` — resolving these three
  closes ~80% of the carrier cluster.
- 2 files with cross-crate ABI surface (`module_exports.rs`
  `RawCallableInvoker`/`ModuleFn` extension contract;
  `multi_table/functions.rs` shape-jit consumer at
  `crates/shape-jit/src/ffi_symbols/data_access/mod.rs:95`).
  Coordinate with shape-vm/shape-jit/extensions migrations rather
  than unilaterally changing the trait-object signatures.

The N9 cleanup hotspot (`type_schema/mod.rs:255-290` calling the
deleted `value.as_heap_ref()` / `value.raw_bits()` tag_bits dispatch)
is in-scope for Phase 1.B and pre-flagged as needing audit-grounded
cleanup, not pure-mechanical.

#### 2.7.4 API rebuild scope clarification

Phase 1.B's first work session (`6ae58c4`, partial close at 57/62
errors) surfaced that the ValueWord-deletion bulldozer cascaded into
helper-API surface beyond the audit's call-site classification. The
audit documents per-cluster *recipe shapes*, not a literal helper-API
catalog. Phase 1.B's scope clarifications:

- **Snapshot serialization** — `nanboxed_to_serializable` /
  `serializable_to_nanboxed` (and `enum_*` / `print_result_*` adapters)
  were deleted. The replacement — kind-threaded
  `slot_to_serializable(slot: &KindedSlot, store) -> Result<SerializableVMValue>`
  plus inverse — is **deferred to a Phase 2c snapshot rebuild
  session**. Phase 1.B replaces the deleted-API call sites with
  `todo!("phase-2c snapshot rebuild — see snapshot.rs:648 deferral")`
  to let `shape-runtime` compile. Snapshot/restore is a known-broken
  capability; do not paper over it with placeholder serializers that
  silently corrupt persisted state.

- **Stdlib registration** — `register_typed_function` /
  `register_typed_async_function` (variadic-arg helpers) were deleted
  in favor of per-arity helpers (`register_typed_fn_N`) in
  `crates/shape-runtime/src/marshal.rs`. Phase 1.B **re-introduces the
  variadic helpers at the KindedSlot shape** — body signature
  `Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>` —
  because (a) variadic dispatch is exactly the §2.7.1.4 dispatch-slice
  case, (b) the 5 stdlib consumers (json/msgpack/toml/yaml/stdlib_time)
  genuinely need variadic shape for functions with optional arguments,
  and (c) per-arity stdlib mass migration is Phase 2c scope, not Phase
  1.B's caller migration. The new variadic helpers live alongside the
  per-arity ones in `marshal.rs`. Both are valid registration paths;
  per-arity is preferred when the function arity is fixed.

- **Output adapter** — `PrintResult` and `PrintSpan` (output-formatting
  carriers) were inline references to the deprecated
  `RareHeapData::PrintResult`. Phase 1.B **moves `PrintResult` /
  `PrintSpan` to `shape-runtime`** (they are runtime-tier formatting
  concerns with no value-tier dependency). Trait signature becomes
  `fn print(&mut self, result: PrintResult) -> KindedSlot`. The
  `RareHeapData::PrintResult` arm is deleted.

- **Display / utility helpers** (`ValueWordDisplay`, `vmarray_from_vec`,
  `ArgVec`, `ValueMap`) — these were thin wrappers around `ValueWord`.
  Their post-`KindedSlot` shapes are call-site-local (DETAIL):
  - `ValueWordDisplay(slot)` → `format!("{:?}", kinded_slot)`, or add
    `KindedSlot::display()` if multi-line formatting is needed.
  - `vmarray_from_vec(...)` → direct `TypedArrayData::from_*`
    constructor matching the array's element FieldType.
  - `ArgVec` typedef → `Vec<KindedSlot>` at call sites.
  - `ValueMap` typedef → `HashMap<String, KindedSlot>` at call sites.

- **Audit accuracy** — the audit's site lists are *recipe instances*,
  not literal site catalogs. Where catalogued sites do not exist in
  the current source (e.g. `event_queue.rs` no longer has the
  Cache/State/Registry structs the audit listed), apply the recipe
  pattern to whatever sites actually exist. This is DETAIL, not
  architectural surface.

#### 2.7.5 Cross-crate ABI policy

`KindedSlot` is a `shape-runtime`-tier carrier. It does **not**
propagate into stable cross-crate ABI surfaces. The split:

- **Extension contract (FFI via `*mut c_void`)** — keeps the raw-bits
  ABI. The canonical site is `RawCallableInvoker.invoke` at
  `module_exports.rs:21`:
  ```rust
  unsafe fn(*mut c_void, &u64, &[u64]) -> Result<u64, String>
  ```
  Extensions store this signature in their CFFI userdata; changing it
  requires extension recompilation. The conversion to/from `KindedSlot`
  happens **inside `shape-runtime` at the boundary** —
  `invoke_callable` reads bits + parallel `NativeKind` from the typed
  registry, constructs `KindedSlot` for runtime-tier dispatch, then
  unpacks back to `u64` for the extension call. Extensions stay on the
  stable raw-bits ABI.

- **Internal Rust trait objects / function pointers** — migrate to
  `KindedSlot`. `ModuleFn` (`module_exports.rs:248`) becomes
  `Arc<dyn for<'ctx> Fn(&[KindedSlot], &ModuleContext<'ctx>) -> Result<KindedSlot, String> + Send + Sync>`.
  `IntrinsicFn` (`intrinsics/mod.rs:32`) becomes
  `fn(&[KindedSlot], &mut ExecutionContext) -> Result<KindedSlot>`.
  These trait objects live entirely inside `shape-runtime` with no
  recompilation concern.

- **shape-vm / shape-jit consumers** — migrate the `shape-runtime`
  side to `KindedSlot`; break the consumer side. shape-jit's
  `ffi_symbols/data_access/mod.rs:95` calls `align_tables` with the
  legacy `(ctx, &[ValueWord])` signature; the consumer-side migration
  is the next session's scope. shape-jit is already non-compiling
  from the broader cascade — there is no value in preserving the
  legacy shape on the runtime side just to keep shape-jit compiling
  through this session.

General policy: **stable ABI surfaces (extension contracts, persisted
formats, FFI handoffs to non-Rust callers) stay on raw bits + parallel
`NativeKind`. Internal Rust dispatch (trait objects, function
pointers, structs, enums) uses `KindedSlot`.** A new internal Rust
API surface that mirrors a stable raw-bits ABI on the runtime side is
acceptable and expected.

#### 2.7.5.1 Wire-format structs are post-proof shapes

`FrameDescriptor` (`crates/shape-vm/src/type_tracking.rs`) is
`#[derive(Serialize, Deserialize)]` and lives inside `FunctionBlob`
(`crates/shape-vm/src/bytecode/content_addressed.rs`), which is the
content-hash unit for distributed bytecode. Per the general §2.7.5
policy, it falls under "stable wire format": the `slots` field stays
`Vec<NativeKind>` — **no `Option<NativeKind>` wrapping, no
`Unspecialized` / `Unknown` placeholder variant**.

Compile-time analysis state where a slot's kind is "not yet known"
during inference is held **locally** in the analysis tracker as
`Option<NativeKind>` or `Result<NativeKind, ProofGap>`. Such
intermediate states must NOT propagate into `FrameDescriptor` — by
the time `FunctionBlob` is constructed, every slot's `NativeKind` is
proven. A slot whose kind genuinely cannot be proven by that point
is a compile error per CLAUDE.md type-system rules ("If the type
can't be proven, it is a compile error. There is no generic-opcode
fallback path."), not a runtime "we don't know" marker.

**Forbidden patterns this rules out:**
- `FrameDescriptor.slots: Vec<Option<NativeKind>>` — the `Option`
  wrap is a wire-format-visible defection-attractor, identical in
  shape to the deleted `SlotKind::Unknown` / `SlotKind::Dynamic`
  variants and the deleted W-series `tag_bits` dispatch sites. Don't
  migrate the in-memory state into the wire format.
- Adding `NativeKind::Unspecialized` / `NativeKind::Unknown` /
  `NativeKind::Pending` — same defection-attractor with a different
  spelling. CLAUDE.md "Renames to refuse on sight" applies in
  spirit; if you find yourself drafting any of these, stop.
- Splitting `FrameDescriptor` into "wire-stable" + "compile-time
  intermediate" twin types where the intermediate type leaks back
  into wire-stable surfaces. The right shape is one wire-stable
  `FrameDescriptor` plus *whatever* local analysis structure the
  compile-time pass needs (a `Vec<Option<NativeKind>>` is fine
  inside the analysis pass, just don't serialize it).

The same rule generalizes to any `#[derive(Serialize, Deserialize)]`
struct that reaches the `FunctionBlob` content hash: its kind fields
are post-proof, no Option wrapping. If a future struct needs to
carry "not yet known" kind state across sessions or wire boundaries,
that's an ADR-level decision (probably wrong-shape).

#### 2.7.6 Carrier API bound (Q8 ruling)

The `KindedSlot` accessor + constructor surface is **bounded by
`NativeKind` variant cardinality**. This is mechanical, code-review-
enforceable, and matches the §2.7 / Q7 carrier-not-discriminator
framing.

**For each `NativeKind` scalar variant V** (`Int64`, `Float64`, `Bool`,
`Char`, `String`, etc. — see `crates/shape-value/src/native_kind.rs`
for the complete list):
- At most one constructor: `KindedSlot::from_<v>(payload) -> KindedSlot`
  that wraps a concrete payload and sets `kind = NativeKind::V`.
- At most one scalar accessor: `KindedSlot::as_<v>() -> Option<T>` that
  matches on `self.kind`; returns `Some(payload)` if `self.kind ==
  NativeKind::V`, else `None`.

**For heap kinds (`NativeKind::Ptr(HeapKind::*)`)**:
- One constructor per `HeapKind` variant
  (`KindedSlot::from_typed_array`, `KindedSlot::from_typed_object`,
  `KindedSlot::from_hashmap`, etc.) — all already in place from
  Phase 1.B.
- **NO per-heap-variant accessor on `KindedSlot`.** Dispatch on
  heap-side payload goes through
  `kinded_slot.slot.as_heap_value() -> Option<&HeapValue>` (already
  on `ValueSlot`) plus pattern-match on `&HeapValue`. `HeapValue`
  stays the **single discriminator** per ADR-005 §1.

**Forbidden shapes the bound rules out:**

- `KindedSlot::as_typed_array()`, `KindedSlot::as_typed_object()`,
  `KindedSlot::as_hashmap()`, `KindedSlot::as_decimal()`,
  `KindedSlot::as_function_id()`, etc. — every per-heap-variant
  accessor would re-create parallel `HeapKind` discrimination on a
  non-`HeapValue` type. Use `slot.as_heap_value()` + `HeapValue::*`
  match.
- `KindedSlot::as_X()` where X is not a `NativeKind` variant
  (e.g. `as_number_or_int_coerced()`) — coercion is the caller's
  job at the body site, not a carrier concern.
- Convenience accessors bundling multiple kinds into one return
  (e.g. `as_any_numeric() -> Option<f64>` covering both `Int64`
  and `Float64`). Bodies that accept heterogeneous-kind input
  dispatch on `kind` explicitly at the body site.
- `KindedSlot::as_value_word()`, `KindedSlot::raw_bits()` — same
  defection-attractor as the deleted `ValueWord::raw_bits()` /
  `ValueWordExt::*` surface, just renamed. CLAUDE.md "Renames to
  refuse on sight" applies in spirit.

**Adding a method outside the bound requires either:**

- (a) Adding a `NativeKind` variant to `shape-value` (gated by
  ADR-006 / Q-ruling — same gate as ADR-005 §1 single-discriminator
  additions), OR
- (b) An ADR amendment justifying the parallel discrimination
  (would need to overcome ADR-005 §1).

**Mechanical effect:** at maximum, `KindedSlot` carries ~25
constructors and ~5-10 scalar accessors (NativeKind has ~25 variants
total, ~7 are scalar; ~18 are `Ptr(HeapKind::*)` which get
constructor-only). Total carrier surface is ~150 LoC, bounded by the
type system's enum cardinality, not by user demand.

**Code-review rule:** "Does this proposed accessor pair 1:1 with a
`NativeKind` variant, with no parallel discrimination on `HeapKind`?
If no, refuse."

**Heterogeneous-kind body pattern.** Builtin bodies that genuinely
accept heterogeneous-kind input (e.g. `abs(x: int|float)`,
`format(value: any)`) dispatch on `kind: NativeKind` explicitly at
the body site:

```rust
fn builtin_abs(arg: &KindedSlot) -> Result<KindedSlot, VMError> {
    match arg.kind {
        NativeKind::Int64 =>
            Ok(KindedSlot::from_int(arg.as_i64().unwrap().abs())),
        NativeKind::Float64 =>
            Ok(KindedSlot::from_number(arg.as_f64().unwrap().abs())),
        _ => Err(type_error("abs requires int or float")),
    }
}
```

This is **runtime-tier dispatch on a carrier** at a builtin
boundary, not a resurrection of the deleted hot-path `tag_bits`
dispatch. It does not violate the strict-typing rules — the
alternative (Option 2: per-kind body variants) pushes the same
dispatch into the central wrapper and costs the same total work.

#### 2.7.7 Stack ABI kind-awareness — parallel `Vec<NativeKind>` (Q9 ruling)

Phase 1.B-vm Wave 5b (commit `fa2bafc`) surfaced that `pop_builtin_args`
cannot recover per-arg `NativeKind` from the typed VM stack: the
compiler emits typed pushes (`PushNativeInt64`, `PushNativeF64`,
etc.) and the kind is consumed by the producing opcode and
discarded. `FrameDescriptor.slots` tracks per-LOCAL kind, not
per-stack-position kind, so the existing infrastructure doesn't
close the gap.

**Decision (Q9 ruling):** the VM stack ABI extends to carry a
**parallel `Vec<NativeKind>` track** alongside the existing
`Vec<u64>` data:

```rust
pub struct VmStack {
    data: Vec<u64>,           // 8-byte raw slots (existing, unchanged)
    kinds: Vec<NativeKind>,   // parallel kind track (NEW — 1 byte per slot)
}
```

Every push records the kind in lockstep with the bits; every pop
reads both. Index invariant: `data.len() == kinds.len()` at every
opcode boundary.

**WB2.4 retain-on-read discipline** uses the parallel track for
kind-aware clone/drop dispatch:

```rust
impl VmStack {
    fn push(&mut self, bits: u64, kind: NativeKind) { ... }
    fn pop(&mut self) -> (u64, NativeKind) { ... }
    fn read_owned(&self, idx: usize) -> KindedSlot {
        // For retain-on-read sites that hand a share to a runtime-tier carrier
        let bits = self.data[idx];
        let kind = self.kinds[idx];
        clone_with_kind(bits, kind);  // increment Arc strong-count if heap
        KindedSlot::new(ValueSlot::from_raw(bits), kind)
    }
}
```

The `clone_with_kind(bits, kind)` and `drop_with_kind(bits, kind)`
helpers replace the deleted `vw_clone` / `vw_drop` (which dispatched
on `tag_bits` internally). Post-Wave-6, the kind is locally available
at every retain/release site — **the deleted `tag_bits` dispatch,
`is_heap()`, and `as_heap_ref()` call sites do not return**.

**Forbidden shapes this rules out:**

- `Vec<KindedSlot>` for the stack — § 2.7.5 explicitly forbids
  `KindedSlot` in the typed VM↔JIT slot ABI.
- 16-byte stack slots (e.g. `Vec<TypedSlot>` where `TypedSlot = {
  bits: u64, kind: NativeKind }`) — would conflict with §2.1's
  8-byte slot invariant and double the stack memory.
- Tag bits packed into the u64 — would re-introduce the deleted
  ValueWord `tag_bits` dispatch (CLAUDE.md "Forbidden code").
- Stack-side kind track typed as `Vec<Option<NativeKind>>` — same
  defection-attractor as §2.7.5.1's wire-format rule. Stack
  contents are post-proof; every pushed slot has a known kind by
  construction (the producing opcode emitted it).
- `Vec<NativeKind>` track holding `NativeKind::Unknown` /
  `NativeKind::Dynamic` placeholders — both deleted; per-stack-position
  kinds are always concrete.
- **Transitional shims preserving deleted ValueWord-shape names**
  (`push_raw_u64`, `pop_raw_u64`, `push_native_i64`,
  `stack_read_owned`, `stack_peek_raw`, etc.) **backed by kinded
  primitives with `NativeKind::Bool` default**. The shim's
  apparent "leak-freeness" is an accident of `Bool`'s no-op
  Drop/Clone, not WB2.4 retain-on-read — semantically these are
  **"borrowed slot" with call-pattern invariants**, exactly the
  W-series bug class (heap pointer pushed via shim → no Arc
  increment → relies on source binding outliving stack push, a
  fragile call-pattern invariant the type system can't verify).
  **Migrate every caller to the kinded API in-wave; do not
  preserve legacy names as a transitional layer.** "Just keep
  the shim until Wave N" is the rationalization CLAUDE.md
  "Renames to refuse on sight" applies to verbatim. If a wave
  cannot complete its scope without shims, surface the cascade
  cost; do not introduce them.

**Performance characteristics:**

- Push: 1 word write to `data` + 1 byte write to `kinds`. Sequential
  cache lines.
- Pop: 1 word read + 1 byte read. Same.
- WB2.4 clone/drop: dispatch on `kind` (1 byte cmpxchg target),
  call matching `Arc::increment_strong_count::<T>` / `decrement`.
  **Strictly faster than the deleted `vw_clone(bits)`, which
  dispatched on `tag_bits` before performing the same Arc work.**
- Memory overhead: 1 byte per stack slot (vs. 8 bytes data) =
  +12.5% stack memory. For typical frame sizes (≤256 slots), this
  is ≤256 bytes per frame — negligible.
- Cache line behavior: `data` and `kinds` are separate allocations.
  Hot opcode dispatch reads `data[idx]` and `kinds[idx]` together —
  branch predictor + prefetch handles the parallel access well.

**Cross-check on debug builds:** the parallel track's per-position
kind should match `FrameDescriptor.slots[corresponding_local]` for
locals, and the producing opcode's emitted kind for stack-temporary
positions. A `debug_assert_eq!` at every push/pop catches kind drift
during development; in release builds the assertions compile out.

**Migration scope:** Wave 6's territory per the audit
(`docs/cluster-audits/phase-1b-vm-valueword-callers.md` §D1, §D4):
`vm_impl/stack.rs` (94 refs), `bytecode/opcode_defs.rs` (39 refs),
`executor/objects/raw_helpers.rs`, all `executor/{stack_ops,
arithmetic, comparison, logical, loops, call_convention}/mod.rs`,
`executor/control_flow/mod.rs`. The migration:

1. Extend `VmStack` with `kinds: Vec<NativeKind>` field +
   push/pop signature changes.
2. Replace `vw_clone(bits)` / `vw_drop(bits)` call sites with
   `clone_with_kind(bits, kind)` / `drop_with_kind(bits, kind)` —
   kind from the local context (FrameDescriptor or stack track).
3. `pop_builtin_args` (Wave 5b's `NativeKind::Bool` transitional
   sentinel) reads the parallel-track kind directly. Transitional
   tagging removed.
4. JIT codegen (Wave 10) emits both data and kind writes in
   lockstep — `mir_compiler` generates the `kinds.push(NativeKind::*)`
   alongside the existing `data.push(bits)`.

#### 2.7.8 Cell-storage kind-awareness — parallel `Vec<NativeKind>` extended to cells (Q10 ruling)

Phase 1.B-vm Wave 6.5 substep-2 cluster B (commits 28de706..727143e
landed at supervisor merge `62513e3`) surfaced that the §2.7.7
parallel-kind-track invariant stops at the stack boundary.
Cell-bearing storage structs that hold `Vec<u64>`-shaped raw slots —
closure cell layout (`closure_raw::read_owned_mutable_ptr`),
shared-cell payload, module-binding storage, and the
`CallFrame.closure_heap_bits: Option<u64>` field at
`executor/mod.rs:188` — carry **no parallel `NativeKind`** alongside
the heap pointer. `Load*Ptr` handlers cannot reconstruct the kind
locally, and `vw_drop(bits)` (forbidden #8 per §2.7.7) cannot be
rewritten as `drop_with_kind(bits, kind)` without an extension.

The agent correctly refused to introduce a `NativeKind::Bool`-default
fallback (§2.7.7 #9 — the W-series rationalization). Cluster B
partial-closed (110 of 278 mandatory sites migrated; -123 errors) and
surfaced the gap as architectural.

**Decision (Q10 ruling):** the §2.7.7 parallel-`Vec<NativeKind>`
invariant **extends to every cell-storage struct** that holds raw
heap-pointer bits in the runtime/VM tier. Each `Vec<u64>`-like cell
store grows a parallel `Vec<NativeKind>`; `Option<u64>` heap-bit
fields gain an `Option<NativeKind>` companion. `clone_with_kind` /
`drop_with_kind` are reused — same dispatch tables as §2.7.7.

Concretely, the targets are (non-exhaustive — extend per discovered
cell-bearing struct):

```rust
// crates/shape-vm/src/executor/closure_raw.rs — closure cell layout
pub struct ClosureCell {
    pub bits: Vec<u64>,          // EXISTING — raw payload
    pub kinds: Vec<NativeKind>,  // NEW — per-cell kind, lockstep with bits
}

// shared-cell payload (Arc<...> wrapper currently bits-only)
pub struct SharedCell {
    bits: AtomicU64,             // EXISTING
    kind: NativeKind,            // NEW — set at construction, read at drop
}

// module-binding storage (Vec<u64> form)
pub struct ModuleBindingStorage {
    bits: Vec<u64>,              // EXISTING
    kinds: Vec<NativeKind>,      // NEW — lockstep with bits
}

// CallFrame.closure_heap_bits (Option<u64> form)
pub struct CallFrame {
    // ...
    pub closure_heap_bits: Option<u64>,        // EXISTING
    pub closure_heap_kind: Option<NativeKind>, // NEW — lockstep with closure_heap_bits
}
```

**Index invariant:** for `Vec<u64>` + `Vec<NativeKind>` companion
pairs, `bits.len() == kinds.len()` at every observable boundary
(method entry/exit, opcode boundaries). For `Option<u64>` +
`Option<NativeKind>` companion pairs, both are `Some` or both are
`None` at every observable boundary; mixed states are a bug.

**Drop discipline.** Every release path (cell-array truncate,
shared-cell unique-drop, CallFrame teardown) calls
`drop_with_kind(bits[i], kinds[i])` — never bare `vw_drop` (forbidden
#8) or "drop only if heap-shaped" probes (forbidden #7). Read paths
into runtime-tier `KindedSlot` carriers bump the heap refcount via
`clone_with_kind(bits[i], kinds[i])` per WB2.4.

**Forbidden shapes this rules out (mirror of §2.7.7's stack-side list,
applied to cell storage):**

- Cell store as `Vec<KindedSlot>` — same §2.7.5 rule as for the stack:
  `KindedSlot` is a runtime-tier carrier, not the storage-tier shape.
  Cells store raw `u64` + parallel `NativeKind`; runtime-tier consumers
  can construct a `KindedSlot` at the read boundary.
- 16-byte cell slots (`Vec<{ bits: u64, kind: NativeKind }>` packed) —
  same §2.1 8-byte slot invariant; cell stores stay 8-byte raw payload
  with a separate kind track.
- Tag bits packed in the `u64` — deleted ValueWord pattern.
- `Vec<Option<NativeKind>>` for the kind track of a `Vec<u64>` cell
  store — cell contents are post-proof per the same §2.7.5.1 rule:
  every cell write carries a known kind by construction. (The
  `Option<NativeKind>` companion to an `Option<u64>` field is a
  *single-slot* presence indicator paired 1:1 with the bits Option;
  the two are populated and cleared together. Different shape from
  "we don't know yet" wrappers.)
- `NativeKind::Unknown` / `NativeKind::Pending` / `NativeKind::Dynamic`
  in the kind track — all deleted; per-cell kinds are always concrete.
- **Transitional Bool-default fallbacks** — same §2.7.7 #9 rule. Refuse
  on sight; surface to supervisor instead. The `NotImplemented(SURFACE)`
  pattern cluster B used for `Load*Ptr` handlers is the correct
  refusal shape — it surfaces the gap as a compile error rather than
  silently leaking shares.
- Cell store carrying its kind via a parallel `Vec<u8>` tag-byte that
  decodes to a custom enum — same defection-attractor as the deleted
  ValueWord `tag_bits` dispatch, just at a different layer.

**Performance characteristics** (mirror of §2.7.7's stack-side
analysis):

- Cell store push/pop: 1 word + 1 byte. Sequential cache lines.
  Frames are short-lived; closures are typically single-digit cells.
- Memory overhead: 1 byte per cell (vs. 8 bytes data) = +12.5% per
  cell, ≤16 bytes per typical closure — negligible.
- WB2.4 clone/drop: dispatch on `kind` (1 byte cmpxchg target),
  call matching `Arc::increment_strong_count::<T>` / `decrement`. Same
  helpers as the stack — no new dispatch surface.

**Cross-check on debug builds:** for closure cells whose binding source
is a typed local, the cell's `kind` should match the local's
`FrameDescriptor.slots[binding_idx]`. A `debug_assert_eq!` at the
closure-creation site catches kind drift during development.

**Migration scope (Wave 6.5 cluster B-round-2 territory):**

1. Extend `closure_raw::ClosureCell` (or current closure-layout struct)
   with `kinds: Vec<NativeKind>` — every constructor + push/pop
   signature accepts/returns `(bits, kind)`.
2. Extend `SharedCell` with `kind: NativeKind` — single-slot, set at
   construction.
3. Extend module-binding storage with `kinds: Vec<NativeKind>`.
4. Extend `CallFrame.closure_heap_bits: Option<u64>` (executor/mod.rs:188)
   with companion `closure_heap_kind: Option<NativeKind>`. The teardown
   path replaces forbidden `vw_drop(bits)` with `drop_with_kind(bits, kind)`.
5. Migrate `Load*Ptr` / `Store*Ptr` handlers in cluster B's
   `variables/mod.rs` to thread the kind through. Cluster B-round-2
   closes the remaining 168 mandatory shim sites once §2.7.8 lands.
6. JIT codegen (Wave 10) emits the parallel kind writes at every cell
   construction site — same lockstep discipline as the stack-side
   §2.7.7 codegen.

**Out-of-scope this ruling:** Snapshot/restore serialization of cell
stores. Per §2.7.4, snapshot rebuild is Phase 2c. The Phase-1.B-vm
work updates in-memory cell layouts; the persisted/wire shapes get
their parallel-kind extension at Phase 2c entry.

#### 2.7.9 Method-dispatch ABI kind-awareness — `MethodFnV2` over `&[KindedSlot]` (Q11 ruling)

Phase 1.B-vm Wave-α `D-array-joins` (close commit `2fe4a6b`) and
Wave-β `M-datatable` (close commit `eb78699`) surfaced that the
§2.7.7 / §2.7.8 parallel-kind invariant stops at the method-dispatch
boundary. The `MethodFnV2` type alias defined in
`crates/shape-vm/src/executor/objects/method_registry.rs` is
**kind-blind in both directions**:

```rust
// Pre-§2.7.9 (kind-blind):
pub type MethodFnV2 = fn(
    &mut VirtualMachine,
    args: &mut [u64],            // raw u64 only — no NativeKind track
    Option<&mut ExecutionContext>,
) -> Result<u64, VMError>;        // raw u64 result — no NativeKind
```

Every PHF entry in `method_registry.rs` (~280 method handlers spread
across `executor/objects/*_methods.rs`, `executor/objects/array_*.rs`,
`executor/objects/datatable_methods/*.rs`, `executor/objects/concat.rs`,
etc.) takes its receiver-and-args as a kind-blind `&mut [u64]` and
returns a kind-blind `u64`. The dispatch shell `op_call_method` in
`executor/objects/mod.rs` would have to fabricate a kind on the
result push (the W-series "Bool-default because Drop is a no-op"
rationalization §2.7.7 #9 forbids verbatim) and the handler bodies
have no way to dispatch on per-arg `NativeKind` for receiver
classification (heap-vs-scalar split, `HeapKind::TypedArray` vs
`HeapKind::DataTable` vs `HeapKind::String`, etc.) without falling
back to the deleted `tag_bits` dispatch (forbidden #4 / #7) or an
`is_heap()` probe (forbidden #7) on the receiver bits.

Across Wave-α and Wave-β migrations roughly 150 handler bodies
collapsed to `NotImplemented(SURFACE)` — the playbook §7.4 REVISED
correct refusal shape — waiting for the architectural ABI flip
this ruling specifies.

**Decision (Q11 ruling):** the method-dispatch ABI extends the
§2.7.7 / §2.7.8 parallel-kind invariant by **carrying the kind on
the carrier itself at the boundary**. `MethodFnV2` becomes:

```rust
// §2.7.9 (kinded):
pub type MethodFnV2 = fn(
    &mut VirtualMachine,
    args: &[KindedSlot],         // kinded carrier per §2.7.6 dispatch-slice case
    Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError>; // kinded result
```

`args[0]` is the receiver (kind = `NativeKind::Ptr(HeapKind::*)`
for heap receivers, `NativeKind::String` / `Float64` / `Int64` /
`Bool` for inline-scalar receivers, etc.); `args[1..]` are the call
arguments in order. Handler bodies dispatch on `args[0].kind`
(receiver classification) and on each `args[i].kind` (per-arg
classification) per the §2.7.6 / Q8 heterogeneous-kind body
pattern, going through `args[i].slot.as_heap_value()` + `HeapValue`
match for heap arms (preserves ADR-005 §1 single-discriminator).

**The `&[KindedSlot]` shape is exactly §2.7.1 case 4 — the
dispatch-slice carrier.** The PHF map in `method_registry.rs` is a
**heterogeneous-kind body** in §2.7.6 vocabulary: each handler
expects a specific kind shape for its receiver-and-args, dispatches
on slot kinds at entry, and returns a specific kinded result. The
slice form is the exact carrier §2.7.1 case 4 names:

> *Case 4 — dispatch slice. A function takes `&[KindedSlot]`
> heterogeneous-kind args; the body dispatches on `slot.kind` per
> arg. Use sites: `op_call_value` arg list, intrinsic dispatch.*

`MethodFnV2` is the ~280-entry generalization of `op_call_value`'s
heterogeneous-kind dispatch slice.

**WB2.4 retain-on-read discipline at the dispatch boundary.** The
dispatch shell `op_call_method` constructs the `&[KindedSlot]` from
popped stack args. Per playbook §2 kind-sourcing rules + §3 pop
pattern:

```rust
// Kind-sourcing (per playbook §2):
//   - Receiver kind: from pop_kinded() (the producing opcode emitted
//     it; the parallel-Vec<NativeKind> track on the stack carries it
//     into op_call_method per §2.7.7).
//   - Per-arg kind: from pop_kinded() (same).
let arg_count = /* from instruction operand */;
let mut args: Vec<KindedSlot> = Vec::with_capacity(arg_count + 1);
for _ in 0..(arg_count + 1) {
    let (bits, kind) = self.pop_kinded()?;
    // SAFETY: pop_kinded transfers one share to us; we hand it to
    // KindedSlot::new which now owns the share. The handler reads
    // (borrows) it via the &[KindedSlot] slice; on drop, the
    // dispatch shell (or the returned-result re-push path) releases
    // each share through KindedSlot's Drop dispatch.
    args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
}
args.reverse();  // pop order is reverse of push order
let result: KindedSlot = handler(self, &args, ctx)?;
// args drops here — each KindedSlot's Drop releases the share via
// drop_with_kind dispatch. No bare vw_drop(bits) (forbidden #8).
self.push_kinded(result.slot.into_raw(), result.kind)?;
std::mem::forget(result);  // we transferred the share onto the stack;
                           // skip the carrier-drop to balance refcounts.
```

The dispatch shell never fabricates a kind; every kind in the
slice and every kind on the result come from the §2.7.7 /
§2.7.8 parallel-kind tracks. No Bool-default fallback (§2.7.7 #9),
no tag_bits decode (§2.7.7 #4 / #7), no heap probe via deleted
ValueWord accessors (§2.7.7 #7).

**Options considered:**

- **Option A: `args: &[(u64, NativeKind)]` parallel-tuple slice.**
  Mirrors the §2.7.7 stack-side `Vec<u64>` + `Vec<NativeKind>`
  *parallel-track* shape one level closer (a slice of (bits, kind)
  pairs is morally a single packed buffer, not two parallel ones,
  but it preserves the "kind alongside data, not bundled in a
  carrier" bias). **Rejected.** §2.7.6 / Q8 named the
  `KindedSlot` carrier as the ADR-006 vocabulary for boundary-
  carrier shapes specifically to avoid proliferating pair / tuple
  /shape variants across crates. Method dispatch is a boundary
  (the single-most-common GENERIC_CARRIER site per ADR-006 §2.7.1
  case 4 enumeration); using `&[KindedSlot]` rather than
  `&[(u64, NativeKind)]` keeps the project's vocabulary consistent.
  The §2.7.7 stack track is a *storage-tier* choice (8-byte slot
  invariant matters there, see §2.7.7 forbidden shapes #2 / #3);
  the method-dispatch carrier is a *runtime-tier* choice (no
  storage-shape constraints) where the carrier struct is the
  natural fit. Adopting Option A would reintroduce two ways to
  spell the same boundary, the §2.7.6 / Q8 ruling forbids on
  carrier-API-bound grounds.

- **Option B: keep `&mut [u64]` + Bool-default fallback at the
  dispatch shell.** Push `args[0]` with a fabricated
  `NativeKind::Bool` because "the dispatcher already owns the
  share, Drop is a no-op". **Rejected — forbidden by §2.7.7 #9.**
  This is the W-series defection-attractor verbatim: the apparent
  leak-freeness is an accident of `Bool`'s no-op Drop, not of any
  refcount discipline. The first heap-pointer receiver pushed
  via the shim leaks an `Arc::into_raw`'d strong count (or, on
  the result side, mis-Drops a heap pointer as a `Bool` no-op).
  CLAUDE.md "Renames to refuse on sight" applies verbatim; this
  option is not a real option.

- **Option C: `args: &[KindedSlot]` dispatch-slice carrier.
  Result: `Result<KindedSlot, VMError>`.** **Accepted.** The
  carrier is the canonical §2.7.6 / Q8 vocabulary; the slice
  shape is the canonical §2.7.1 case 4 dispatch-slice form; every
  handler body uses `args[i].kind` at the §2.7.6 heterogeneous-
  kind dispatch site without indirection. The dispatch shell
  sources every kind from the §2.7.7 stack parallel-kind track
  (no fabricated kind); the result-push path takes the kind from
  the handler-returned `KindedSlot.kind` (no fabricated kind).
  The migration cost (~280 PHF handler signature flips) is the
  cross-cluster cascade Wave-α / Wave-β surfaced; the bodies
  themselves migrate in Wave-γ-followup once the ABI flip lands.

**Forbidden shapes this rules out (mirror of §2.7.7 / §2.7.8
forbidden lists, applied to method-dispatch ABI):**

- `args: &mut [u64]` with kind decoded from the high bits of each
  `u64` — same deleted tag_bits dispatch as §2.7.7 #4 / #7. Method
  dispatch is post-proof: the producing opcode pushed each arg with
  a known kind onto the §2.7.7 parallel-kind track; the dispatch
  shell already has the kind, fabrication is forbidden.
- `args: &mut [u64]` with an `is_heap()` probe on each entry to
  classify heap-vs-scalar receivers — §2.7.7 #7 forbidden, the
  deleted ValueWord-shape probe.
- `args: &mut [u64]` + a *parallel* `&[NativeKind]` second slice
  parameter on `MethodFnV2`. **Rejected on §2.7.6 / Q8 grounds:**
  the carrier API bound says "kind on the carrier struct, not as
  a parallel side-channel on the function signature". The §2.7.7
  parallel-`Vec<NativeKind>` shape is appropriate at the
  *storage-tier* boundary (8-byte slot constraint, two
  allocations); at the *runtime-tier dispatch boundary* the
  carrier-struct shape is canonical.
- `args: &mut [KindedSlot]` (mutable). **Rejected** — handlers
  borrow the args; the dispatch shell owns the shares. Mutability
  invites a body to swap a `KindedSlot` in-place, which would
  desynchronize the dispatch shell's drop accounting. `&[KindedSlot]`
  is borrow-only, matching the dispatch contract.
- `Vec<KindedSlot>` by-move into the handler. **Rejected** —
  same desynchronized-drop concern. By-move would transfer
  ownership of every share to the handler, which then has to
  unconditionally drop or push everything. Borrow-only `&[..]` keeps
  the share-accounting at the dispatch shell where the §2.7.7
  invariants live.
- Result type `(u64, NativeKind)` rather than `KindedSlot`. Same
  Option-A rejection rationale: §2.7.6 / Q8 carrier-API-bound says
  the project speaks `KindedSlot` at boundaries, not parallel-pair
  variants. `KindedSlot` already has the WB2.4-correct `Drop`
  dispatch (`drop_with_kind` keyed on `kind`); a `(u64, NativeKind)`
  result would force every handler to call the helper explicitly.
- **Transitional shims preserving deleted ABI-shape names** —
  `MethodFn` / `MethodFnLegacy` / `dispatch_method_handler_raw` /
  `call_handler_with_u64_slice` — same §2.7.7 #1 rule, the
  W-series "borrowed bits with call-pattern invariants" defection-
  attractor at the dispatch-shell layer. **Migrate every PHF entry
  in-wave; do not preserve a legacy ABI as a transitional layer.**
  The cross-cluster cascade closure is the deliverable; "just keep
  the kindless variant for the methods that already work" is the
  rationalization §2.7.7 forbids verbatim.
- **Defection-attractor descriptors** — "MethodFnV2 bridge",
  "MethodFn translator", "dispatch-slice probe", "boundary
  adapter for handler ABI", "kind-injection helper". Per the
  2026-05-09 user ruling broadening the W-series rename family,
  any descriptor of the deleted kind-blind ABI that uses bridge /
  probe / helper / hop / translator / adapter framing belongs to
  the same defection-attractor family CLAUDE.md "Renames to refuse
  on sight" enumerates. Describe the deleted ABI by name (the
  pre-§2.7.9 `args: &mut [u64]` MethodFnV2) or by deletion-fate
  (the kind-blind handler ABI), never by hypothetical role.

**Performance characteristics** (mirror of §2.7.7 / §2.7.8
analyses):

- `KindedSlot` is `repr(C)` `{ slot: ValueSlot (u64), kind:
  NativeKind (1 byte) }`. With natural alignment / padding, the
  carrier is 16 bytes; a `&[KindedSlot]` of N args is `N * 16`
  bytes vs. the pre-§2.7.9 `N * 8` bytes for `&mut [u64]`. **Net
  cost:** +8 bytes per arg at the dispatch boundary. For typical
  call patterns (1–3 args per method call), this is +8 to +24
  bytes per dispatch — negligible. The slice itself is allocated
  once per method call on the dispatch shell's stack frame; no
  heap allocation, no pointer chase per arg.
- Pop+construct: `pop_kinded()` (1 word read + 1 byte read from
  the parallel tracks) + `KindedSlot::new(ValueSlot::from_raw,
  kind)` (struct construction, no Drop work). One per arg.
  Strictly the same work the §2.7.7 stack pop already does; the
  carrier struct is just a different shape over the same bits.
- Result push: `push_kinded(result.slot.into_raw(), result.kind)`
  (1 word write + 1 byte write to the parallel tracks) +
  `mem::forget(result)` to balance the carrier-drop accounting.
  Strictly the same work the §2.7.7 stack push already does.
- WB2.4 clone/drop within the slice: `KindedSlot::Drop` dispatches
  on `kind` (1 byte cmpxchg target) and calls `drop_with_kind`.
  Same dispatch table as §2.7.7 / §2.7.8; **no new dispatch
  surface.** **Strictly faster than the deleted W-series shape**
  (which dispatched on tag_bits before performing the same Arc
  work).
- IC fast path: `MethodIcHit` stores a `MethodFnV2` function
  pointer keyed on `(receiver_kind, method_name_id)`. Pointer
  shape is unchanged (the function-pointer-as-`usize` storage is
  ABI-opaque); the IC keying is unchanged (`receiver_kind: u8` —
  the lower 8 bits of `NativeKind::Ptr(HeapKind::*) as u8` — is
  the same as it was pre-§2.7.9). The IC fast-path call site
  constructs the `&[KindedSlot]` once per dispatch from popped
  args; the fast-path skip is the same number of cycles it was.

**Cross-check on debug builds:** for each `args[i]` constructed in
the dispatch shell from `pop_kinded`, the kind read from the
§2.7.7 parallel-kind track should match the producing opcode's
emitted kind (the call-site emitter knows what kind it pushed). A
`debug_assert_eq!` inside `op_call_method`'s arg-construction loop
catches kind drift during development; in release builds the
assertions compile out.

**Migration scope (Wave-γ G-method-fn-v2-abi territory plus
follow-up):**

1. Type alias `MethodFnV2` in
   `crates/shape-vm/src/executor/objects/method_registry.rs` flips
   from `(&mut VM, &mut [u64], _) -> Result<u64, VMError>` to
   `(&mut VM, &[KindedSlot], _) -> Result<KindedSlot, VMError>`.
   Same flip on `MethodHandler` (which is a type alias to
   `MethodFnV2`).
2. Dispatch shell `op_call_method` in
   `crates/shape-vm/src/executor/objects/mod.rs` is currently a
   `NotImplemented(SURFACE)` stub from D-objects-mod Wave-α. Its
   doc-comment surface text is updated to reflect that the ABI is
   now flipped (§2.7.9 landed); the body remains a SURFACE stub
   because the receiver-classification cascade and IC fast-path
   wiring are downstream Wave-γ-followup territory (the kinded
   bodies for `handle_typed_object_method_v2`, the
   `v2_array_detect` PHF-fast-path receiver kind unwrap, and the
   legacy stack-based calling convention all need their own
   sub-cluster work).
3. Every PHF handler signature (~280 across ~33 files) re-aligns:
   `args: &mut [u64]` → `args: &[KindedSlot]`; `Result<u64,
   VMError>` → `Result<KindedSlot, VMError>`. Bodies that were
   already `NotImplemented(SURFACE)` keep that body; bodies that
   had real implementations (~150 of the ~280) become
   `NotImplemented(SURFACE)` with the migration contract
   documented (per the M-datatable Wave-β `joins.rs` precedent at
   close commit `eb78699`). Wave-γ-followup migrates each body
   off SURFACE per the §2.7.6 / Q8 heterogeneous-kind body
   pattern.
4. IC fast-path consumer (`crates/shape-vm/src/executor/ic_fast_paths.rs`)
   imports `MethodFnV2` for IC entry pointer storage. The
   signature change is internal to the function-pointer type; the
   storage shape (transmute through `usize`) is unchanged. The
   `test_method_ic_handler_roundtrip` unit test's `dummy_handler`
   constant signature realigns to the new ABI — minor follow-up
   in the IC sub-cluster.
5. Receiver classification + sub-dispatch cascade in
   `op_call_method` (`receiver_is_numeric` / `receiver_is_bool` /
   `receiver_is_heap` + `HeapKind` match + sub-dispatch on
   `Concurrency` / `TypedArray` / `Temporal` / `TableView` inner
   variants) rewrites from the deleted `ValueWord::is_*` /
   `as_heap_ref` (forbidden) to `match args[0].kind { NativeKind::*
   => ..., NativeKind::Ptr(HeapKind::*) => args[0].slot
   .as_heap_value() match { HeapValue::* => ... } }` per
   ADR-006 §2.7.6 / Q8. Wave-γ-followup territory.
6. v2-typed-array PHF fast-path detector
   (`v2_array_detect::as_v2_typed_array`) currently relied on
   `as_vw_ref` reinterpreting `&u64` as `&ValueWord`. With
   `ValueWord` deleted the detector takes raw bits + kind directly
   — a Wave-γ-followup `D-v2-array-detect` cluster row.
7. Legacy stack-based calling convention (the legacy `_` arm
   reading `arg_count` and `method_name` from the stack via
   `pop_raw_u64` + `ValueWord::as_str`) either becomes the kinded
   equivalent (`pop_kinded` + `String` arm match) or is deleted
   as legacy bytecode the compiler no longer emits. Wave-γ-
   followup territory.

**Cross-cluster surfaces (~280 handler signatures realigned in this
ruling-implementation cluster, ~150 working bodies become SURFACE,
each surfaces back into Wave-γ-followup body migration territory).**
The architectural ABI flip is the deliverable of this ruling; the
~150 body migrations are downstream waves (the same shape as
M-datatable Wave-β surfaced from D-array-joins Wave-α — close
commit `eb78699` set the per-handler precedent at one PHF entry
pair, this ruling generalizes the same flip across the full PHF
registry).

**Out-of-scope this ruling:** Snapshot/restore of in-flight method
calls (suspension state crossing a `MethodFnV2` boundary). Per
§2.7.4, snapshot rebuild is Phase 2c; the kinded-ABI in-flight
suspension shape gets its own follow-up if/when async method calls
land in the snapshot subset.

## 3. Lifetime, ownership, and storage planning

### 3.1 Reuse the existing infrastructure

The MIR storage-planning pass (`crates/shape-vm/src/mir/storage_planning.rs`),
the borrow solver (`mir/solver.rs`), `BindingStorageClass` and
`BindingSemantics` (`type_tracking.rs:286-310`), `B0013` /`B0014`,
ref-escape analysis (`mir/lowering/mod.rs`) — all preserved.

### 3.2 Extensions

Two new `BindingStorageClass` variants for cross-thread sharing:

```rust
pub enum BindingStorageClass {
    Deferred,
    Direct,
    UniqueHeap,
    SharedCow,
    Reference,
    LocalMutablePtr,
    SharedAtomic,        // NEW: Arc<T>, T: Send + Sync, read-shared across threads
    SharedAtomicMut,     // NEW: Arc<Mutex<T>>, mutable-shared across threads
}
```

`var` inference adds a new pass after storage-planning that picks the
class for `var` bindings based on observed usage. The pass consumes the
existing borrow-solver output, escape-analysis output, and
`B0014 NonSendableAcrossTaskBoundary` results — it does not reimplement
them.

### 3.3 RC is opt-in by escape

The default for owned heap-resident bindings is `UniqueHeap`. The
compiler escalates to `SharedCow` / `SharedAtomic` / `SharedAtomicMut`
**only when escape requires it**:

- Closure capture where the closure escapes the binding's scope.
- Storage into a container that itself shares.
- Send across a task / thread boundary.
- Explicit user opt-in via `Arc<T>` / `Mutex<T>` syntax.

`let mut x = 0` does NOT allocate a refcounted heap value. It allocates
zero bytes (or a stack slot).

## 4. VM / JIT / FFI boundary

### 4.1 Uniform slot ABI

VM and JIT read/write typed slots via the same NativeKind interpretation.
No conversion at the VM↔JIT boundary, including OSR and deopt. JIT emits
typed `load`/`store` of the appropriate width (Cranelift `iconst` of the
pointer + `store i64`).

### 4.2 Tier-dependent introspection metadata

VM mode preserves frame metadata, slot kinds, source positions —
enables full introspection for debugging, time-travel, breakpoints, LLM-
inspection. JIT may drop these as an optimization for tier-2 hot
functions. Deopt re-materializes the abstract VM state from compiled-
frame metadata (HotSpot precedent — survey 02 §4.1).

### 4.3 Cranelift backend

Cranelift remains the JIT backend (10× faster than LLVM at comparable
runtime perf — survey 02 §8.1). Tier-1 baseline @ 100 calls; tier-2
optimizing @ 10k. Inline caches with monomorphic→polymorphic→megamorphic
state machine (survey 02 §6.1).

ISLE patterns for typed slot ops are added incrementally as needed; the
existing `MirToIR` lowering is preserved and extended, not replaced.

### 4.4 `extern C` FFI

`extern C fn` calls go through a typed C ABI bridge (existing — see
ADR-004). `repr(C)` discipline at the boundary; no `ValueWord`-style
packing. Project Panama precedent (survey 02 §5.1).

### 4.5 Polyglot — see PVL audit (§7)

## 5. Strings

### 5.1 Carrier

`String` value is 16-byte tagged: 15-byte UTF-8 SSO inline OR pointer to
`Arc<[u8]>` UTF-8 buffer. Bit-packed length+flags following Swift /
Mojo / ecow precedent (survey 03 §1.4, §1.6, §1.8).

Tag bit distinguishes inline-vs-pointer; not a per-value runtime tag in
the same sense as NaN-boxing — the carrier type itself encodes the choice
and the type system knows it's a `String`.

### 5.2 Heap form

Refcounted CoW (`Arc<[u8]>` shape). Mutation triggers copy when refcount
> 1 (Swift / Mojo precedent — survey 03 §1.4, §1.8).

### 5.3 Interning

Compile-time interning for string literals via existing `StringId(u32)`
in opcodes (`crates/shape-value/src/ids.rs:60-79`). Runtime InternPool
deferred to a post-v1 phase if profiling shows demand. **No global
runtime interning in v1.**

### 5.4 Concat and slice

Concat eagerly allocates. Reuse analysis covers the build-then-write
pattern (Roc precedent — survey 03 §1.12). **No ConsString lazy
concat** in v1.

Slice produces a `Str` view (not a sub-buffer reference that retains the
parent — Erlang's sub-binary leak risk — survey 03 §1.11). View is a
lifetime-bounded reference.

### 5.5 Encoding

UTF-8 throughout. Survey 03 §8.1 — UTF-8 has won (Swift, Mojo, Rust, Go).

## 6. Arrays and direct memory access

### 6.1 Element-typed buffers

`Array<T>` where `T` is a primitive (int, number, bool, byte) maps to
`HeapValue::TypedArray(Arc<TypedArrayData>)` with the matching inner
buffer (`TypedBuffer<T>`). Existing `TypedArrayData` enum preserved.

### 6.2 Direct memory access

Element access compiles to direct typed loads/stores when the bound
check elides (existing JIT optimization). SIMD via Cranelift vector
intrinsics (survey 03 §4.2).

### 6.3 Multi-dim and SoA

Multi-dim arrays carry shape+strides à la NumPy (survey 03 §3.2). SoA
opt-in via `@layout(soa)` annotation if/when demand surfaces; not v1.

### 6.4 Arrow C Data Interface

Arrow CDI is the FFI contract for zero-copy export of typed arrays
(survey 03 §6.1). Implemented as part of the polyglot boundary work.

## 7. Polyglot — PVL audit, then conditional implementation

### 7.1 Audit (Phase 4 of migration)

A scoped audit (~2 weeks) examines whether the existing per-language
marshal layers (PyO3 for Python, deno_core for TypeScript, `extern C`
bridges) genuinely share enough structure to unify under a 7-shape PVL
protocol.

Audit deliverable: an addendum to this ADR (`006-addendum-pvl-audit.md`)
recommending PVL-implementation or per-language-status-quo. Both
outcomes are valid; the audit is to determine which.

### 7.2 If PVL adopted

Single boundary protocol with seven shapes (`Scalar`, `Frozen`, `Native`,
`OpaqueHandle`, `Buffer`, `Stream`, `UserMarshal`). 3-bit kind shadow in
side table (per-binding, not per-value — single-discriminator preserved).
Each language's native value model unchanged; PVL describes only what
crosses the boundary.

### 7.3 If PVL rejected

Per-language adapters preserved. Each Shape↔Foreign boundary remains
its own thin marshal layer. The existing ADR-004 `extern C` design is
the C-side reference.

## 8. Distribution and dependency model

### 8.1 Content-addressed everything

Existing `FunctionBlob::content_hash` (SHA-256) extended:
- Native dependencies (`shape.toml [native-dependencies]`) hashed by
  resolved library version + file hash.
- Foreign-language deps hashed by venv-equivalent lock entries.
- Permissions baked into manifest hash (existing).

### 8.2 Lockfile

`shape.lock` (new) records resolved dependency tree with content hashes
for every transitive dep — Shape, native, Python (venv), TypeScript,
etc. Reproducible builds: matching `shape.lock` on different machines
produces byte-identical bytecode artifacts.

### 8.3 Foreign-language dep handling

- **Python:** `shape build` resolves Python deps via `requirements.txt`-
  equivalent in `shape.toml`, generates and freezes a venv, hashes the
  venv contents into `shape.lock`. Activated at runtime when polyglot
  Python code executes.
- **TypeScript:** `package-lock.json`-equivalent; deno_core consumes
  the lock.
- **Native C libraries:** existing `[native-dependencies]` in
  `shape.toml` extended with file-hash field.

### 8.4 Nix integration — optional

If the user's environment has Nix, the foreign-dep resolution can defer
to a Nix flake. Otherwise Shape's own resolver. Not load-bearing on Nix.

### 8.5 Distribution units

Functions, modules, or whole programs. Each has a content hash; the
hash is the trust statement (signed via Ed25519 — existing).

## 9. Error system — LLM-Structured Diagnostic Schema (LSDS)

### 9.1 Primary output

Compiler diagnostics emit LSDS (JSON) as the primary format. Terminal,
LSP, and MCP renderers consume LSDS and produce human-readable output.
LSDS is the source of truth.

### 9.2 LSDS schema (sketch)

```json
{
  "diagnostic_id": "B0013",
  "severity": "error",
  "location": { "file": "src/main.shape", "line": 12, "col": 4, "span": [102, 145] },
  "expected": { "type": "int", "witness": 42 },
  "found": { "type": "string", "witness": "hello" },
  "message": "expected int, found string",
  "fixes": [
    {
      "label": "convert string to int",
      "diff": "let x: int = parse_int(value)?",
      "confidence": 0.85
    }
  ],
  "context_window": {
    "tokens": 312,
    "spans": [ { "file": "...", "lines": [10, 14] } ]
  },
  "rule": "ADR-006-§1.1"
}
```

### 9.3 Type witnesses

Where the compiler can synthesize a concrete value satisfying or
violating the type constraint, it includes the value as a witness.
Reduces the LLM's need to construct examples.

### 9.4 Suggested-fix diffs

Compiler emits ranked code-diffs for the most common error classes
(missing import, type coercion, borrow violation, escape). Ranked by
confidence. LSP code actions consume them.

### 9.5 Token-budgeted context windows

For each error, LSDS includes a "context window" — the smallest set of
source spans needed to understand the error, with a token count. LLMs
consuming LSDS get exactly the context they need.

### 9.6 Inference recovery

Errors are locally-bounded — they don't poison the type environment
globally. Inlay hints continue to work in the rest of the file even
when one binding has an error.

## 10. Permission system

### 10.1 Two tiers

- **Tier 0 (zero-cost, static):** Capabilities enumerated by opcode
  scan. The linker computes the transitive permission set from blob
  opcodes; no runtime cost. Existing design (CLAUDE.md "Security Model
  Tier 1").
- **Tier 1 (runtime, ~5ns per call):** Path-based access checks at
  stdlib syscall boundaries. Existing design (`check_permission()` per
  CLAUDE.md).

### 10.2 Permission propagation

Permissions are baked into `FunctionBlob::content_hash`. Two functions
with identical code but different permissions hash differently. The
linker computes transitive union at link time.

### 10.3 Granularity

Function, module, or whole-program. The permission boundary is wherever
the user declares one (function-level via attribute, module-level via
manifest, program-level via top-level).

### 10.4 PES — Phase 5, behind feature flag

Permission-aware JIT speculation (NOVEL): the tier-2 JIT may specialize
on observed permission state and prune dead permission branches. Feature
flag default off; promote to default when ≥3× speedup demonstrated on
permission-heavy I/O loops with deopt rate <1%.

## 11. CT-AION — v2 capability

`@ai`-tagged optimization advisor consultation at compile time, for
decisions where heuristics are weak (layout choice, tile size, region
merge). Reproducibility:

- Advisor prompt + model + seed are part of the content hash.
- Two builds with the same advisor pin produce byte-identical output.
- Off by default; opt-in per package via `shape.toml` flag.

Not required for v1. Phase 6 of migration; can be deferred indefinitely
without affecting the rest of the design.

## 12. Migration roadmap

| Phase | Scope | Duration | Deps |
|---|---|---|---|
| **1.A** | HeapValue layout refactor (variant payloads → `Arc<TypedT>`); per-FieldType ValueSlot constructors; `from_heap` deprecated; `TypedObjectStorage` struct extracted; drop discipline. Cluster #1 partial commits salvaged where compatible. | 2-3 months | — |
| **1.B** | Migrate slot construction sites (was cluster #1 scope). `typed_object_from_pairs` flipped. Caller migration. shape-vm twin parallel-impls migrated. | 1 month | 1.A |
| **1.C** | `var`-inference pass extending storage-planning. Two new `BindingStorageClass` variants (`SharedAtomic`, `SharedAtomicMut`). Inlay hints. | 1-1.5 months | 1.A; parallel-safe with 1.B |
| **2** | LSDS — primary diagnostic format. Renderers. Type witnesses. Fix-diff generation. | 1.5-2 months | parallel with 1.A-C |
| **3** | Cranelift JIT modernization against new slot ABI. Tier-1 baseline + tier-2 optimizing. Uniform frame format. | 3-4 months | 1.A-C complete |
| **4** | PVL audit (~2 weeks) + (conditional) PVL implementation (~6-8 weeks). | 2 weeks audit + maybe 6-8 weeks impl | parallel with 3 |
| **5** | PES — permission-aware JIT speculation, behind feature flag. | 6 weeks | 3 complete |
| **6** | CT-AION — opt-in compile-time AI advisor. | 4 weeks | any time after 1 |

**Total: ~10-14 months wall-clock at 2 FTE, or ~7-10 months at 3 FTE.**

### 12.1 Migrator-cluster-1 commits — disposition

`bulldozer-strictly-typed-intrinsics-dev1` carries 5 commits
(`263e372`–`dd02c8e`) from the prior migrator. Disposition:

- `263e372` (Step 1, define `TypedFieldValue`): **keep**. The 12-variant
  shape is correct.
- `681557f` (Step 2, slot constructors **including `from_heap_arc`**):
  **rewrite**. Drop `from_heap_arc`; keep the per-FieldType
  constructors. This is the Q6-violation commit.
- `2260310` (Steps 4+6+8 bundled): **partial keep**. The signature flip
  + readback rewrite + import deletion is correct. The `from_heap_arc`
  call sites need adjustment.
- `7cbff57` (Step 9, ADR-005 forward-pointer comments): **keep verbatim**.
- `dd02c8e` (AGENTS.md): **regenerate** as part of phase 1 close.

Phase 1.A starts by cherry-picking `263e372` and `7cbff57` clean, then
re-doing 681557f / 2260310 against the new layout.

### 12.2 ADR-005 supersession

ADR-005 §1 (single-discriminator), §2 (String exception), §4 (uniform
slot ABI), §Forbidden (no `Box<HeapValue>` slot wrapping in new code) —
**preserved verbatim** here.

ADR-005 §3 typed-pointer constructor examples — **corrected** in this
ADR §2.4 to match the actual layout.

ADR-005 §5 future optimizations roadmap — folded into this ADR's §12.

The `// ADR-005` marker comments at five source sites stay; new code may
add `// ADR-006` markers for v3-specific concerns.

## 13. Forbidden patterns (extends ADR-005 §Forbidden)

- **No `from_heap_arc(Arc<HeapValue>)` catch-all slot constructor.** Per-
  FieldType constructors only. (Q6 ruling, reaffirmed.)
- **No refcount-by-default for `var`.** Default is `Direct` (stack);
  refcount only on escape. (§3.3)
- **No new modal-types subsystem.** Reuse the existing borrow solver and
  storage planner. (§3.1)
- **No `let`/`let mut` inference of policy class.** The policy is fixed
  by the keyword. (`var` is the only inferred form.)
- **No global runtime string interning in v1.** Compile-time only.
  (§5.3)
- **No NaN-box or low-bit-tag reintroduction** anywhere. (ADR-005 §1.)
- **No conversion at VM↔JIT boundary.** (§4.1, ADR-005 §4.)

Plus all existing CLAUDE.md "Forbidden Patterns" remain binding.

## 14. Success metrics

Defined upfront so we measure rather than rationalize:

- **var inference convergence:** ≥80% of `var` bindings on a corpus of
  50 Shape programs are inferred to `Direct` or `UniqueHeap` (i.e., no
  refcount). Compile-time overhead of inference ≤15%.
- **`from_heap` callers:** 0 (deleted) at end of Phase 1.B.
- **shape-runtime --lib errors:** 0 at end of Phase 1.C.
- **Slot ABI uniformity:** zero conversion ops at VM↔JIT boundary
  (verified by JIT codegen audit).
- **String fast path:** SSO threshold ≥15 bytes. Allocation rate on
  parsed-JSON workload reduced by ≥40% vs current.
- **LSDS adoption:** ≥95% of compiler errors emit LSDS with witness +
  fix-diff fields populated. Average error LSDS payload ≤500 cl100k
  tokens.
- **Cranelift JIT compile time:** baseline ≤10ms per function (Pulley /
  Cranelift target).
- **Distribution reproducibility:** same `shape.lock` on two machines →
  byte-identical bytecode artifacts.

If any metric misses by >2×, surface and re-audit before proceeding to
the next phase.

## 15. Visibility

Following ADR-005's convention:

- This ADR file (`docs/adr/006-value-and-memory-model.md`) is canonical.
- CLAUDE.md "Forbidden Patterns" gets a new subsection "ADR-006 patterns"
  pointing here.
- Code comments at load-bearing sites carry `// ADR-006` markers
  (extends the existing `// ADR-005` set):
  - `BindingStorageClass` definition (`type_tracking.rs:286`).
  - `ValueSlot` per-FieldType constructors (`slot.rs`).
  - HeapValue payload definitions (`heap_variants.rs`).
  - The `var` inference pass entry point (TBD post-Phase-1.C).
  - LSDS schema definition (TBD post-Phase-2).
- defections.md gets an append-only entry at Phase 1.A start naming
  the supersession of ADR-005 §3 by this ADR.

## 16. References

### Research base
- `docs/research/01-ownership-gc.md`
- `docs/research/02-layout-runtime.md`
- `docs/research/03-strings-arrays.md`

### Design alternatives
- `docs/adr/006-DRAFT-alternative-B.md`
- `docs/adr/006-DRAFT-alternative-C.md`

### Cluster audits
- `docs/cluster-audits/cluster-1-type-schema.md` (now superseded by §1, §2 of this ADR)
- `docs/cluster-audits/cluster-{4,5,6}-*.md` (preserved as historical context)

### Code anchors
- Pest grammar: `crates/shape-ast/src/shape.pest:760-771` (`variable_decl`, `var_mut_modifier`, `ownership_modifier`)
- BindingStorageClass: `crates/shape-vm/src/type_tracking.rs:286-310`
- Storage planning: `crates/shape-vm/src/mir/storage_planning.rs`
- Borrow solver: `crates/shape-vm/src/mir/solver.rs`
- ValueSlot: `crates/shape-value/src/slot.rs`
- HeapValue: `crates/shape-value/src/heap_variants.rs`

### External
- ADR-005: `docs/adr/005-typed-slot-construction.md` (this supersedes its §3)
- Strict-typing plan: `~/.claude/plans/stop-native-vs-tagged-tax.md`
- Strict-typed baseline: `docs/strictly-typed-baseline.md`
- Forbidden patterns: `CLAUDE.md` "Forbidden Patterns" section

## 17. Resolved questions

Answers below were reached during the ADR-006 review on 2026-05-08
(Q1-Q6), the Phase 1.B carrier-shape decision on 2026-05-08 (Q7),
the Phase 1.B-vm Wave 5 carrier-API-bound decision on 2026-05-08
(Q8), and the Phase 1.B-vm Wave 6 stack-kind-track decision on
2026-05-09 (Q9). Q5 remains predicted-pending-audit; the rest are
decisions binding for Phase 1 onward.

### Q1 — `var` × `B0014 NonSendableAcrossTaskBoundary` coordination

**Decision:** B0014 fires as an error for `let` / `let mut`. For `var`,
the same condition triggers a class upgrade to `SharedAtomicMut` (or
`SharedAtomic` if read-only) instead of an error.

**Rationale:** Consistent with the let/let mut/var philosophy — explicit
forms have contracts, `var` is forgiving. The inlay hint shows the
upgrade so users see the cost of the cross-task share. Concrete
example:

```shape
let counter = 0
spawn { counter += 1 }   // B0014 ERROR (user wrote `let`)

var counter = 0
spawn { counter += 1 }   // OK; ⟦SharedAtomicMut⟧ inlay hint
```

### Q2 — Schema-pointer vs schema-id at drop

**Decision:** Default to schema-id with HashMap lookup. Profile in Phase
1.A; switch to `Arc<TypeSchema>` only if drop-path is >1% of CPU on a
representative workload.

**Rationale:** Schema-id is 8 bytes per `TypedObject`; schema-pointer is
16 bytes plus an Arc bump on every object construction. Drops are
typically batched at scope-end (one HashMap probe per object) — amortized
cost favors the id+lookup approach. Switch only if profiling justifies
moving the cost from the (frequent) alloc path to the (less frequent)
drop path.

### Q3 — `@ai` × `var` inference ordering

**Decision:** `@ai` annotation rewriting runs first at comptime. `var`
inference runs on the rewritten body. No special-casing in the inference
layer.

**Rationale:** `@ai` expands `@ai fn name(args) -> ReturnType {}` into a
function body that constructs an LLM prompt and parses the structured
response. By the time type-inference + storage-planning passes run, the
AST is fully expanded — the generated body uses regular `let` /
`let mut` / `var` internally. Add a Phase 1.C test for an `@ai` body that
uses `var` to validate.

### Q4 — JIT introspection drop strategy

**Decision:** Three-layer:

- **Tier 1 (baseline) always keeps** introspection metadata (frame info,
  slot kinds, source positions).
- **Tier 2 (optimizing) drops by default**, gated by a per-function
  `introspection_needed` flag. Flag is set true when a debugger or
  profiler attaches to the function, otherwise false.
- **Deopt always works** via stack maps + side tables (HotSpot
  precedent). Stack-map cost is amortized; side tables aren't on the hot
  path.

**Rationale:** Tier-1 metadata is small relative to interpreter dispatch;
keeping it everywhere costs little. Tier-2 metadata is bloat that hurts
cache + codegen on hot loops; dropping is the win. The
`introspection_needed` flag is the escape hatch for active debugging.
Matches V8/JSC/HotSpot production patterns (survey 02 §4.1, §4.4, §8.1).

### Q5 — PVL audit outcome (predicted)

**Predicted decision:** Partial PVL — unify scalars + frozen-immutable
values + buffer crossings (where structural overlap is real); keep
per-language adapters for object-handle crossings (where it isn't).

**Rationale:** Python's PyObject (refcount + attributes + methods),
TypeScript's JSValueRef (prototypes + dynamic shape), and C (no value
model) genuinely diverge at the object-handle layer. Forcing a unified
protocol there would invent escape hatches that erode the unification.
But scalars, frozen values, and Arrow buffers are already moved as
opaque bit-blobs at all four boundaries — unification captures real
shared structure.

**Status:** Phase 4 audit (~2 weeks) is the actual decision-maker. This
is a prediction.

### Q9 — Stack ABI kind-awareness (Phase 1.B-vm Wave 6 surface)

**Decision:** the VM stack ABI extends with a **parallel
`Vec<NativeKind>` track** alongside the existing `Vec<u64>` data
slots. Per-stack-position kind is stored explicitly; WB2.4
retain-on-read uses the parallel track for kind-aware clone/drop
dispatch (`clone_with_kind` / `drop_with_kind`). Spec lives at §2.7.7.

**Rationale:** Phase 1.B-vm Wave 5b surfaced the gap —
`pop_builtin_args` cannot recover per-arg `NativeKind` from the
typed VM stack because the compiler emits typed pushes and the
kind is consumed by the producing opcode. Three options were
considered:

- **Option A (kind from `FrameDescriptor.slots`)** — rejected:
  `FrameDescriptor` is per-LOCAL, not per-stack-position. Doesn't
  fit the actual data flow.

- **Option C (opcode operands carry kind, e.g. `Call(builtin_id,
  arity, packed_arg_kinds)`)** — rejected: works for
  fixed-signature builtins but doesn't generalize to variadic
  (`println(...)`, `format(...)`) or higher-order calls
  (`fn.apply(args)`).

- **Option B (parallel `Vec<NativeKind>` stack track)** —
  accepted. Generalizes the FrameDescriptor pattern (slots →
  kinds parallel) at the stack level. Leaves no surface for the
  deleted `tag_bits` dispatch sites — kind is locally available at
  every retain/release.

**Performance characteristics:** push/pop overhead is +1 byte per
slot (negligible). WB2.4 clone/drop is **strictly faster** than the
deleted `vw_clone(bits)` (which dispatched on `tag_bits` before
performing the same Arc work).
Cache-line behavior: `data` and `kinds` are separate allocations
but accessed in lockstep — prefetch/branch-predictor handles well.
Memory overhead: +12.5% stack memory (e.g. ≤256 bytes per typical
frame).

**Status:** Binding for Wave 6 onward. Wave 5b's `NativeKind::Bool`
transitional sentinel in `pop_builtin_args` is removed by Wave 6.
Heap-bearing builtins (`len(array)`, `string_concat`, etc.) become
runtime-correct after Wave 6 lands.

**Anti-pattern call-out (post-Wave-6.0 supervisor ruling 2026-05-09):**
the transitional-shim layer (legacy push/pop names backed by Bool
default) introduced by Wave 6.0 (`d782401`) was rejected as a
W-series-shape defection-attractor. The pattern is now explicitly
forbidden in §2.7.7. Wave 6.5 deletes the shims and migrates every
legacy ValueWord caller in arithmetic/comparison/loops/call_convention
/raw_helpers/control_flow to the kinded API as part of the wave.
Wave 6 close gate now includes a grep-fail: zero `push_raw_u64` /
`pop_raw_u64` / `push_native_i64` / `stack_read_owned` /
`stack_peek_raw` callers in the codebase.

### Q10 — Cell-storage kind-awareness (Phase 1.B-vm Wave 6.5 cluster B surface)

**Decision:** the §2.7.7 parallel-`Vec<NativeKind>` invariant
**extends to every cell-storage struct** that holds raw heap-pointer
bits in the runtime/VM tier. Each `Vec<u64>`-like cell store grows a
parallel `Vec<NativeKind>`; `Option<u64>` heap-bit fields gain an
`Option<NativeKind>` companion. Targets: closure cell layout
(`closure_raw::ClosureCell`), shared-cell payload (`SharedCell`),
module-binding storage, and `CallFrame.closure_heap_bits` at
`executor/mod.rs:188`. `clone_with_kind` / `drop_with_kind` reused
verbatim. Spec lives at §2.7.8.

**Rationale:** Phase 1.B-vm Wave 6.5 substep-2 cluster B partial-close
(commits 28de706..727143e merged at supervisor `62513e3`) surfaced the
gap. Three options considered:

- **Option A (Bool-default fallback for `Load*Ptr` handlers)** —
  rejected: this is the §2.7.7 #9 W-series rationalization the cluster
  B agent correctly refused. "Drop is a no-op for Bool" is the same
  borrowed-slot-with-call-pattern-invariants defection-attractor.

- **Option B (Phase-2c deferral via `todo!()` stubs)** — rejected for
  closure cells / module bindings: these are core hot-path runtime
  surfaces, not snapshot/restore wire formats. Deferral would block
  every `Load*Ptr` handler indefinitely.

- **Option C (parallel `Vec<NativeKind>` extended to cells)** —
  accepted. Generalizes the §2.7.7 stack-side pattern to the
  cell-storage tier. No new dispatch surface (reuses
  `clone_with_kind` / `drop_with_kind`), no defection-attractor
  variant introduced, mechanical to verify (lockstep
  `bits.len() == kinds.len()` invariant).

**Performance characteristics:** mirror of §2.7.7. Per-cell push/pop
+1 byte; +12.5% memory overhead per cell. WB2.4 clone/drop reuses the
same dispatch as the stack side. Closures are typically single-digit
cells, frames are short-lived — cumulative overhead is negligible.

**Status:** Binding for Phase 1.B-vm Wave 6.5 cluster B-round-2
onward. Cluster B-round-2 closes the remaining 168 mandatory shim
sites in `variables/mod.rs` / `loops/mod.rs` / `control_flow/mod.rs`
/ `call_convention.rs` once §2.7.8 lands. Snapshot/restore wire
extension is Phase 2c per §2.7.4 (out of scope here).

**Anti-pattern call-out:** the cluster B agent's correct response to
the gap was `NotImplemented(SURFACE)` returns from `Load*Ptr`
handlers — a compile-error surface that escalates to supervisor,
*not* a runtime fallback that silently leaks shares. This is the
canonical surface-and-stop pattern under §2.7.7's prohibition; future
cluster agents who hit a kind-source gap should mirror it.

### Q8 — Carrier API bound for `KindedSlot` accessors/constructors

**Decision:** `KindedSlot`'s accessor and constructor surface is
**bounded by `NativeKind` variant cardinality** (one constructor +
at most one scalar accessor per variant; **no per-heap-variant
accessors** — heap dispatch via `slot.as_heap_value()` +
`HeapValue` match). Adding a method outside this bound requires
adding a `NativeKind` variant first (itself gated) or an ADR
amendment overcoming ADR-005 §1. Spec lives at §2.7.6.

**Rationale:** Phase 1.B-vm Wave 5 surfaced that the audit's
"STATIC_KIND once dispatch flips" claim was wrong for heterogeneous-
kind builtin bodies (~12 accessors + ~30 constructors needed).
Three options were considered:

- **Option 1 (full ValueWordExt-equivalent on `KindedSlot`)** —
  rejected: same defection-attractor surface as the deleted
  2,497-LoC `ValueWordExt` module, just renamed (CLAUDE.md
  "Renames to refuse on sight" pattern). Surface unbounded by
  type-system structure.

- **Option 2 (per-kind dispatch tables in `BuiltinFunction`
  enum)** — rejected: massively bigger refactor (every
  `BuiltinFunction` arm × per-kind dispatch). Pushes the same
  dispatch into the central wrapper without architectural win;
  total work same.

- **Option 4 (refined Option 3 — bounded carrier API +
  HeapValue-via-slot for heap dispatch)** — accepted. Surface
  bounded by `NativeKind` cardinality; heap-side dispatch
  preserves ADR-005 §1 single-discriminator (HeapValue is the
  canonical heap discriminator); ~150 LoC carrier total.

**Performance characteristics:** KindedSlot is shape-runtime tier
(§2.7.5); not in opcode dispatch / VM stack ABI / JIT codegen.
Accessor calls (`match self.kind` per call) run at builtin-boundary
cost, where function-call overhead already dominates by orders of
magnitude. Hot path stays raw `u64` + opcode-encoded kind, unchanged.

**Status:** Binding for Wave 5a onward. Bound is mechanically
enforceable in code review — "Does this accessor pair 1:1 with a
`NativeKind` variant, with no parallel discrimination on `HeapKind`?
If no, refuse."

### Q7 — Carrier shape for kind-erased call sites (Phase 1.B surface)

**Decision:** Introduce `KindedSlot { slot: ValueSlot, kind: NativeKind }`
carrier struct in `shape-value` (Option B). Used for the GENERIC_CARRIER
call sites identified by the Phase 1.B audit (2026-05-08); not used for
STATIC_KIND sites where `NativeKind` is locally available. Spec lives
at §2.7.

**Rationale:** The Phase 1.B audit found three call-site patterns —
STATIC_KIND (mechanical, no carrier needed), GENERIC_CARRIER
single-value, GENERIC_CARRIER vector storage. All three are served by
one `KindedSlot` shape. Alternatives considered and rejected:

- **Option A (raw `(ValueSlot, NativeKind)` tuples)** — rejected for
  vector sites: `Vec<(ValueSlot, NativeKind)>` and
  `Vec<ValueSlot>` + `Vec<NativeKind>` both lose the lockstep guarantee.
  One indexing bug separates them and the type system stops catching it.

- **Option C (parallel `Vec<NativeKind>` track)** — rejected for the
  same reason: adds one more slot to every storage struct that must be
  hand-maintained on every push/pop/swap. The `WB2.4` / `WB2.5`
  retain-on-read pattern already had to be hand-maintained on `Vec<u64>`;
  doubling that surface area is exactly where bugs hide.

- **Re-extend `ValueSlot` to 16 bytes with embedded kind** — rejected:
  breaks the slot ABI invariant in §2.1 (the typed-VM↔JIT slot is 8 bytes,
  dispatching on external kind). A 16-byte `ValueSlot` would also expand
  the `TypedObjectStorage::slots: Vec<ValueSlot>` storage by 2× and force
  the JIT codegen to load/store 16 bytes per slot.

- **New `RuntimeValue` enum with HeapKind-aligned variants** — rejected:
  parallel-discriminator anti-pattern explicitly named in ADR-005 §1 and
  the N9 close-out as a defection-attractor.

**Status:** Binding for Phase 1.B onward. Audit-grounded site catalog
at `docs/cluster-audits/phase-1b-valueword-callers.md` (2026-05-08).
Cluster of 60 files / 658 references / ~95 GENERIC_CARRIER sites.

Working-session refinements (Phase 1.B partial close `6ae58c4`,
2026-05-08): API rebuild scope (snapshot defer to Phase 2c, variadic
register_typed_function re-introduction at KindedSlot shape, PrintResult
move to shape-runtime, display/utility helper replacements) is spelled
out at §2.7.4. Cross-crate ABI policy (extension contracts stay on
raw bits, internal Rust dispatch uses KindedSlot) at §2.7.5.

### Q6 — String SSO threshold

**Decision:** Default 15 bytes (Swift / ecow precedent — survey 03 §1.4,
§1.6, §1.8). Exposed as a tunable constant:

```rust
// crates/shape-value/src/lib.rs
pub const SSO_THRESHOLD_BYTES: usize = 15;
```

All SSO-aware code paths (carrier load/store, bit-packing, comparison,
hash) reference the const. Profile-driven adjustment is a one-symbol
change; never hardcoded at call sites.

**Rationale:** 15 bytes balances inline capacity with carrier size for
Shape's 16-byte tagged `String` value. Phase 1.A profiles a Shape stdlib
parser workload; if measurement shows a meaningfully higher threshold
performs better (e.g., 23 bytes per Mojo / smol_str), increment the
const and re-profile.
