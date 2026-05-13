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

**HeapKind variant set is gated by Q8 cardinality.** Adding a new
`HeapKind` variant requires an ADR amendment per §2.7.6 (Q8
"Adding a method outside the bound requires either: (a) Adding a
`NativeKind` variant to `shape-value` ..."). Wave-γ G-heap-filter-expr
(§2.7.9, 2026-05-09) added `HeapKind::FilterExpr` (ordinal 18) to fix
a label-collision soundness gap surfaced by Wave-α D-raw-helpers
(commit `a27c0e4`); see §2.7.9 for the full justification + the
mechanical-lockstep dispatch-table updates that fan out from the
addition. Phase 1.B-vm Wave 8 W8-T25 (§2.7.12, 2026-05-10) added
`HeapKind::SharedCell` (ordinal 19) so the §2.7.7 stack parallel-kind
track and §2.7.8 cell-storage parallel-kind track can label
`Arc<SharedCell>` cell-pointer bits with a dedicated discriminator —
the precondition for unblocking `op_alloc_shared_local` /
`op_alloc_shared_module_binding`; see §2.7.12 for the precedent
mirror.

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
constructors and ~5-10 scalar accessors (NativeKind has ~26 variants
total post-Wave-γ, ~7 are scalar; ~19 are `Ptr(HeapKind::*)` which get
constructor-only — see §2.7.9 for the Wave-γ `FilterExpr` addition).
Total carrier surface is ~150 LoC, bounded by the type system's enum
cardinality, not by user demand.

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

#### 2.7.9 `HeapKind::FilterExpr` — Q8 cardinality amendment (Wave-γ G-heap-filter-expr, 2026-05-09)

Phase 1.B-vm Wave-α D-raw-helpers (commit `a27c0e4`, supervisor merge
`5a738f1`) surfaced a label-collision soundness gap in the
filter-expression branch of `executor/logical/mod.rs`:

> `executor/logical/mod.rs` (And/Or/Not heap path) pushes
> `Arc::into_raw(Arc<FilterNode>) as u64` onto the kinded stack with
> the kind label `NativeKind::Ptr(HeapKind::NativeView)` because no
> `HeapKind::FilterExpr` variant exists. The `clone_with_kind` /
> `drop_with_kind` dispatch tables in `vm_impl/stack.rs` (and the
> §2.7.8 cell-storage mirrors `KindedSlot::{drop,clone}`,
> `TypedObjectStorage::drop`, `SharedCell::drop`) interpret
> `HeapKind::NativeView` as `Arc<NativeViewData>`. When the runtime
> retains or releases a FilterExpr-bearing slot, the dispatch fires
> `Arc::increment/decrement_strong_count::<NativeViewData>` against an
> `Arc<FilterNode>` pointer — wrong-type retain/release at every
> retain/drop site.

This is genuinely undefined behavior, not an aesthetic concern: the two
types have different layouts (`FilterNode` is an enum with Box pointers;
`NativeViewData` is a struct with an integer pointer + layout
metadata), so the wrong destructor walks the wrong fields.

**Decision (Q8 cardinality amendment):** add a new HeapKind variant
`FilterExpr` (ordinal 18, immediately after `HashMap`'s ordinal 17 per
§2.3's append-only ordering convention). The amendment is gated by the
§2.7.6 / Q8 cardinality bound's "Adding a method outside the bound
requires either: (a) Adding a `NativeKind` variant to `shape-value`
(gated by ADR-006 / Q-ruling — same gate as ADR-005 §1
single-discriminator additions), OR (b) An ADR amendment justifying the
parallel discrimination" — option (a) applied via this section.

**Why a new variant rather than off-label re-use of `NativeView`:** the
two payload types (`Arc<FilterNode>` vs `Arc<NativeViewData>`) require
different destructors. Per §2.7.7 / §2.7.8, the `clone_with_kind` /
`drop_with_kind` dispatch tables are the **single source of truth** for
typed-Arc retain/release. A label that selects the wrong destructor is
not a "kind error" the type system can recover from — it's UB. The
discriminator must match the payload 1:1 at the dispatch table.

**Why no parallel `HeapValue::FilterExpr` enrichment is required by
ADR-005 §1 single-discriminator:** ADR-005 §1 says HeapValue is the
single discriminator for **heap-resident values** *that flow through
`HeapValue` materialization*. FilterExpr payloads do **not**: they are
emitted to the kinded stack via `Arc::into_raw(Arc<FilterNode>)` and
consumed via `Arc::from_raw(...)` directly on the slot bits, never
wrapped in `Box<HeapValue>` or accessed via `slot.as_heap_value()`.
Adding `HeapValue::FilterExpr(Arc<FilterNode>)` is provided to preserve
the symmetry property "every `HeapKind` variant has a `HeapValue` arm
of the same shape" (matching `HeapValue::ClosureRaw`/`Future`/
`NativeScalar`/`Char`'s discriminator-only role) — but **calling
`slot.as_heap_value()` on a FilterExpr-labeled slot is undefined
behavior** (the slot bits are an `Arc::into_raw::<FilterNode>` pointer,
not a `*const HeapValue`). Heap dispatch on FilterExpr-kinded slots
goes through the kind label, not through `as_heap_value()`.

**Mechanical lockstep updates (the new variant fans out to 6 dispatch
sites — every Q8/Q10 retain/release table):**

1. `crates/shape-value/src/heap_variants.rs` — `HeapKind::FilterExpr`
   ordinal 18 + `HeapValue::FilterExpr(Arc<FilterNode>)` arm +
   `kind()` / `is_truthy()` / `type_name()` / `Clone` / `Display`
   updates.
2. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` dispatch the new arm to
   `Arc::increment/decrement_strong_count::<FilterNode>`.
3. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror the same arm.
4. `crates/shape-value/src/heap_value.rs` — `TypedObjectStorage::drop`
   §2.7.8 mirror.
5. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
   §2.7.8 mirror.
6. `crates/shape-vm/src/executor/logical/mod.rs` — push sites use
   `NativeKind::Ptr(HeapKind::FilterExpr)` instead of `NativeView`.
7. `crates/shape-vm/src/executor/objects/raw_helpers.rs` —
   `extract_filter_expr` matches the new label.

Plus knock-on exhaustive-match additions in `printing.rs`,
`comparison/mod.rs`, `arithmetic/mod.rs`, `objects/typed_access.rs`
(kind→type-name maps); `wire_conversion.rs`, `json_value.rs`
(HeapValue serialization rejection arms — FilterExpr does not cross the
wire boundary). All knock-on sites are mechanical Q8 mirrors of the
same dispatch-table discipline, not new dispatch surfaces.

**Cardinality cost:** `HeapKind` grows from 18 variants to 19; the
§2.7.6 Q8 bound (~25 constructors / ~5-10 scalar accessors max on
`KindedSlot`) is unchanged because FilterExpr does not need a new
constructor or accessor — the existing `Ptr(HeapKind::*)` constructor
generic shape applies. Total dispatch surface grows by one arm per
table, no new dispatch tables.

**Forbidden alternatives this rules out:**

- "Just keep using `NativeView` as a stand-in label." This is the
  pre-amendment shape; the wrong-type retain/release was the gap
  Wave-α surfaced. Refused: dispatch tables must match payloads 1:1.
- "Make `extract_filter_expr` peek at the bits to disambiguate
  FilterNode from NativeViewData." This is exactly the
  `(decode|tag) (bridge|probe|helper|hop|translator|adapter)` family
  defection (CLAUDE.md "Renames to refuse on sight") — re-introducing
  bit-pattern probing as a substitute for a kind discriminator.
  Refused on sight.
- "Add a single `HeapKind::Other` arm and walk a side-table to
  disambiguate." Same defection at a different layer — Q8
  cardinality says one variant per dispatch shape, not a generic
  bucket plus side-table dispatch.
- "Box `FilterNode` inside `HeapValue::NativeView` so the existing
  dispatch works." Forbidden by ADR-005 §1 (HeapValue is the single
  discriminator) and ADR-006 §2.3 (typed-Arc payloads, no
  Box<HeapValue> wrapping in new code).

**Out-of-scope this amendment:** routing FilterExpr through `HeapValue`
materialization. The new `HeapValue::FilterExpr` arm exists for
HeapKind↔HeapValue symmetry only; no caller materializes a
`Box<HeapValue::FilterExpr>` or expects `slot.as_heap_value()` to
return one. If a future caller needs HeapValue materialization of
FilterExpr, the work is a separate ADR amendment with the same
single-discriminator analysis applied to the materialization path.
#### 2.7.10 Method-dispatch ABI kind-awareness — `MethodFnV2` over `&[KindedSlot]` (Q11 ruling)

Phase 1.B-vm Wave-α `D-array-joins` (close commit `2fe4a6b`) and
Wave-β `M-datatable` (close commit `eb78699`) surfaced that the
§2.7.7 / §2.7.8 parallel-kind invariant stops at the method-dispatch
boundary. The `MethodFnV2` type alias defined in
`crates/shape-vm/src/executor/objects/method_registry.rs` is
**kind-blind in both directions**:

```rust
// Pre-§2.7.10 (kind-blind):
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
// §2.7.10 (kinded):
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
  pre-§2.7.10 `args: &mut [u64]` MethodFnV2) or by deletion-fate
  (the kind-blind handler ABI), never by hypothetical role.

**Performance characteristics** (mirror of §2.7.7 / §2.7.8
analyses):

- `KindedSlot` is `repr(C)` `{ slot: ValueSlot (u64), kind:
  NativeKind (1 byte) }`. With natural alignment / padding, the
  carrier is 16 bytes; a `&[KindedSlot]` of N args is `N * 16`
  bytes vs. the pre-§2.7.10 `N * 8` bytes for `&mut [u64]`. **Net
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
  the same as it was pre-§2.7.10). The IC fast-path call site
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
   now flipped (§2.7.10 landed); the body remains a SURFACE stub
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

#### 2.7.11 Value-call ABI kind-awareness — `op_call_value` over `&[KindedSlot]` (Q12 ruling)

Phase 1.B-vm Wave 6.5 close (`e0915f3`) left the value-call dispatch
path — `op_call_value` in `executor/control_flow/mod.rs` and the
`call_value_immediate_*` family in `executor/call_convention.rs` —
as `todo!("phase-2c — see ADR-006 §2.7.4")` stubs. The blocker is
the same shape as §2.7.10/Q11 surfaced for method dispatch: the
pre-§2.7.11 ABI is kind-blind in both directions, and the §2.7.7 /
§2.7.8 parallel-kind invariant stops at the call-frame boundary.

```rust
// Pre-§2.7.11 (kind-blind, currently `todo!()`):
fn call_value_immediate_static(
    &mut self,
    callee_bits: u64,         // raw u64 — no NativeKind for callee
    arg_bits: &[u64],         // raw u64 args — no NativeKind track
) -> Result<u64, VMError>;    // raw u64 result — no NativeKind
```

Across Wave-α and Wave-δ migrations, ~150 PHF handler bodies in
`executor/objects/*_methods.rs` collapsed to `NotImplemented(SURFACE)`
with the surface message "closure-callback path unmigrated" because
no kinded callee dispatch exists. The canonical example is
`array_sort.rs::handle_order_by_v2` (close commit at Wave-δ
`MR-array-sort-sets-joins`): the kinded `MethodFnV2` ABI landed
(§2.7.10/Q11), the receiver kind = `NativeKind::Ptr(HeapKind::TypedArray)`
flows in cleanly, but the comparator-closure callback path needed to
invoke `key_fn(elem)` per element cannot run because
`call_value_immediate_*` is `todo!()`. Without this ruling,
`.map / .filter / .reduce / .orderBy / .thenBy / .find / .some /
.every / .forEach` and every higher-order method body remains
SURFACE — the user-facing language is missing closures-as-values
end-to-end.

**Decision (Q12 ruling):** the value-call ABI extends the §2.7.7 /
§2.7.8 / §2.7.10 parallel-kind invariant by **carrying the kind on
the carrier itself across the call frame**, mirroring Q11 for the
value-call path:

```rust
// §2.7.11 (kinded):
fn call_value_immediate_static(
    &mut self,
    callee: KindedSlot,           // kinded callee (Ptr(HeapKind::Closure) /
                                  //   Ptr(HeapKind::FunctionRef) / etc.)
    args: &[KindedSlot],          // kinded args per §2.7.6 dispatch-slice case
) -> Result<KindedSlot, VMError>; // kinded result
```

`callee.kind` is the callee classification (Closure / FunctionRef /
TraitObjectMethod / ForeignFn / Bound method) — sourced from the
§2.7.7 stack parallel-kind track at the dispatch shell (no
fabrication, no tag decode). `args[i].kind` per arg, same source.
`op_call_value` constructs the `&[KindedSlot]` slice from popped
stack args using the same WB2.4 retain-on-read discipline §2.7.10
established for `op_call_method` — pop one share per arg, hand to
`KindedSlot::new`, slice borrows, dispatch, `mem::forget` the
returned carrier after re-pushing the share.

**Frame setup — closure-capture kind flow.** This is the §2.7.8
cell-storage extension across the call boundary. When the dispatch
classifies `callee.kind == NativeKind::Ptr(HeapKind::Closure)`, the
frame setup:

1. Reads the closure layout: `ClosureLayout.capture_native_kinds`
   (the parallel-kind track on `OwnedClosureBlock` per Wave-α
   `G-owned-closure-block` close).
2. For each capture, calls `OwnedClosureBlock::read_capture_kinded`
   (per §2.7.8 / Q10) to recover `(bits, kind)` and pushes onto the
   new frame's locals via the same `push_kinded` primitive the
   §2.7.7 stack uses.
3. Sets `CallFrame.closure_heap_kind: Option<NativeKind>` (added by
   B9 Wave-α) from the closure's heap kind label, so closure-self
   loads (`LoadOwnedClosureSelf`) recover the parallel-kind track
   without re-decoding.

The closure-call path is the §2.7.8 cell-storage parallel-kind
invariant *transitively closed* across the call boundary: the
producing opcode pushed each capture onto the closure's
`OwnedClosureBlock` with a known kind (Wave-α G-owned-closure-block);
the dispatch shell hands each capture into the new frame with that
kind preserved (this ruling); the new frame's local-load opcodes
recover the kind from the frame's parallel-kind track (already
landed). End-to-end: no fabrication, no tag decode, no `is_heap()`
probe, no Bool-default fallback — the kind flows through every hop.

**Options considered:**

- **Option A: keep `&[u64]` + a parallel `ClosureLayout`-side kind
  table, decode at the call site.** **Rejected.** The
  `OwnedClosureBlock` already carries `capture_native_kinds` per
  §2.7.8/Q10; piping that through a separate side-channel into
  `call_value_immediate_*` would re-introduce the parallel-track
  shape at the dispatch ABI level — exactly the §2.7.6 / Q8
  carrier-API-bound rejection that ruled out the parallel slice
  in §2.7.10. The carrier-on-the-struct shape is canonical at the
  runtime-tier dispatch boundary; the parallel-track shape is
  appropriate at the storage-tier (stack, cells), not at the
  dispatch boundary. Same ruling as §2.7.10 Option A.

- **Option B: stack-based calling convention preserved (read args
  from the stack inside `call_value_immediate_*`).** **Rejected.**
  This is the deleted pre-§2.7.10 shape applied to the value-call
  path: a kind-blind handler reading from a kinded stack would
  need to fabricate kinds at the read site (§2.7.7 #9 forbidden),
  or call back through a deleted `pop_raw_u64`-shape primitive
  (§2.7.7 forbidden). The §2.7.10 dispatch-slice ABI is the
  established shape for boundary carriers; value-call extends it.

- **Option C: `callee: KindedSlot, args: &[KindedSlot] → Result<KindedSlot, _>`.**
  **Accepted.** Mirrors §2.7.10/Q11 for the value-call path. The
  carrier is the canonical §2.7.6 / Q8 vocabulary; the slice is
  the canonical §2.7.1 case 4 dispatch-slice form; closure-capture
  kind flow uses the existing §2.7.8 / Q10 cell-storage parallel-
  kind track read via `OwnedClosureBlock::read_capture_kinded`.
  Migration cost: ~5 dispatch entry-point signatures
  (`call_value_immediate_static`, `_polymorphic`, `_async`,
  `_method`, plus `op_call_value` itself); ~30 stub call-sites in
  `executor/objects/*` re-fill from SURFACE to real bodies once
  the dispatch entry-points land.

**Forbidden shapes this rules out (mirror of §2.7.7 / §2.7.8 /
§2.7.10 forbidden lists, applied to value-call ABI):**

- `args: &mut [u64]` with kind decoded from raw bits — same
  §2.7.7 #4 / #7 deleted tag_bits dispatch.
- `callee: u64` with `is_heap()` probe to classify Closure vs.
  FunctionRef — §2.7.7 #7 forbidden, the deleted ValueWord-shape probe.
- `&mut [KindedSlot]` mutable form or `Vec<KindedSlot>` by-move —
  same desynchronized-drop concern as §2.7.10. Borrow-only `&[..]`
  keeps the share-accounting at the dispatch shell.
- Result type `(u64, NativeKind)` — same §2.7.6 / Q8 carrier-API-
  bound rejection as §2.7.10.
- **Bool-default fallback for closure captures with unresolved kind**
  at frame setup — §2.7.8 #4 forbidden. The correct response when a
  capture's kind cannot be sourced from `ClosureLayout.capture_native_kinds`
  is `NotImplemented(SURFACE)` and surface back to the supervisor;
  never silent-leak.
- **Transitional shims preserving deleted ABI-shape names** —
  `call_value_legacy` / `call_value_raw_u64` / `dispatch_value_call_handler_raw`
  / `call_value_with_u64_slice` — same §2.7.7 #1 rule, same
  defection-attractor at the value-call dispatch layer. Migrate
  every entry-point in-wave; do not preserve a kind-blind variant
  as a transitional layer.
- **Defection-attractor descriptors** — "value-call bridge",
  "closure-callback translator", "frame-setup probe", "callee-kind
  helper", "capture-injection adapter". Per the 2026-05-09 user
  ruling broadening the W-series rename family, any descriptor of
  the deleted kind-blind value-call ABI that uses bridge / probe /
  helper / hop / translator / adapter framing belongs to the same
  defection-attractor family CLAUDE.md "Renames to refuse on sight"
  enumerates.

**Performance characteristics** (mirror of §2.7.10):

- Per-call overhead: +N×8 bytes for the `&[KindedSlot]` slice (vs.
  pre-§2.7.11 `N×8` raw-u64 slice), where N is arg count. Slice
  is allocated once per call on the dispatch shell's stack frame;
  no heap allocation, no pointer chase per arg.
- Frame setup: closure-capture loop is M kind reads from
  `ClosureLayout.capture_native_kinds` (1 byte each) + M
  `read_capture_kinded` calls (1 word + 1 byte each). The kind
  reads are linear in capture count; the kind decoding is O(1) per
  capture.
- IC fast path for closure-call (`call_value_immediate_static`
  hot path): callee kind dispatch is a single `match` on
  `NativeKind` (1-byte cmp + jump table); same dispatch table as
  §2.7.7 / §2.7.8 / §2.7.10. No new dispatch surface.
- Strictly the same cost as the pre-§2.7.11 `todo!()` stub once
  filled in — i.e., the cost of doing the work the language
  promises.

**Cross-check on debug builds:** for each capture pushed onto the
new frame, the kind read from `ClosureLayout.capture_native_kinds`
should match the kind the closure-allocation opcode emitted (the
emitter knows what kind it captured). A `debug_assert_eq!` inside
the frame-setup loop catches kind drift during development.

**Migration scope (Wave-7 territory):**

1. Sub-cluster **W7-cv-static**: rewrite
   `call_value_immediate_static` in `executor/call_convention.rs`
   to take `(KindedSlot, &[KindedSlot]) → Result<KindedSlot, _>`.
   Frame setup integrates `OwnedClosureBlock::read_capture_kinded`
   per §2.7.8.
2. Sub-cluster **W7-cv-polymorphic**: same flip for
   `call_value_immediate_polymorphic` (the fall-through for
   undetermined-callee-kind dispatch).
3. Sub-cluster **W7-cv-async**: same flip for the async closure
   path (`call_value_async` / suspension-resumption shape).
4. Sub-cluster **W7-cv-method**: cross-link with §2.7.10 — the
   path where `op_call_value` resolves to a method dispatch (e.g.,
   bound-method calls). Routes through the §2.7.10 `op_call_method`
   path with the kinded carrier preserved.
5. Sub-cluster **W7-op-call-value**: rewrite `op_call_value` in
   `executor/control_flow/mod.rs` to construct the `&[KindedSlot]`
   slice via the §2.7.10 `op_call_method` precedent, dispatch on
   `callee.kind`, route to the correct entry-point.
6. Sub-cluster **W7-frame-setup**: integrate
   `CallFrame.closure_heap_kind` (B9 field) with the new frame's
   parallel-kind track at frame entry, so subsequent local-load
   opcodes recover kinds without re-decoding.
7. Sub-cluster **W7-stub-refill**: the ~150 method bodies in
   `executor/objects/*` that surfaced to `NotImplemented(SURFACE)`
   with "closure-callback path unmigrated" messages re-fill from
   SURFACE to real bodies once 1-6 land. This is mostly mechanical
   per the `array_sort.rs::handle_join_str_v2` recipe — the body
   migrates off SURFACE per §2.7.6 / Q8 heterogeneous-kind body
   pattern, calling into the now-live closure-callback path for
   the `key_fn(elem)` / `predicate(elem)` / `transform(elem)`
   invocation step.

The architectural ABI flip is the deliverable of sub-clusters 1-6;
sub-cluster 7 is the downstream Wave-9 mechanical migration once
the ABI lands.

**Out-of-scope this ruling:** Snapshot/restore of in-flight value
calls (suspension state crossing a `call_value_immediate_*`
boundary). Per §2.7.4, snapshot rebuild is Phase 2c; the kinded-
ABI in-flight value-call suspension shape gets its own follow-up
if/when async value-calls land in the snapshot subset. Same
out-of-scope clause as §2.7.10.

**§2.7.11 Migration scope refinement (post-W7-audit, 2026-05-09):**
The `executor/call_convention.rs` surface is **12 entry-points**,
not 5 as the original migration scope text enumerated: the public
entry-points (`execute_function_by_name`, `execute_function_by_id`,
`execute_closure`, `execute_function_fast`,
`execute_function_with_named_args`, `resume`, `execute_with_async`,
`call_value_immediate_nb`, `jit_trampoline_call_closure`), the
internal frame-setup helpers (`call_function_with_nb_args`,
`call_closure_with_nb_args_keepalive`, `call_function_from_stack`),
and the deleted `_raw` family (`call_value_immediate_raw`,
`call_function_with_raw_args`, `call_closure_with_raw_args`)
which carried a hybrid `&[(u64, NativeKind)]` pair-slice form
pre-§2.7.11. **The `&[(u64, NativeKind)]` pair-slice form is
rejected on §2.7.6/Q8 carrier-API-bound grounds** at the runtime
tier and the three `_raw` entry-points migrate to either
`&[KindedSlot]` or are deleted as redundant with the kinded
entry-points. The JIT-trampoline FFI consumer
(`jit_trampoline_call_closure`) keeps the pair-slice shape because
the §2.7.5 cross-crate stable boundary needs raw u64 + parallel
kind; consumers translate from `&[KindedSlot]` to `&[u64]` at the
FFI boundary, single direction. This refinement keeps §2.7.11/Q12
architecturally consistent: one carrier shape (`KindedSlot`) at
the runtime tier, one parallel-pair shape (raw u64 + parallel
`NativeKind`) at the storage/FFI tier (stack, cells, JIT
trampoline), no third hybrid. The W7 playbook
(`docs/cluster-audits/wave-7-cc1-playbook.md`) carries this
refinement as binding for all 6 sub-clusters.

#### 2.7.12 `HeapKind::SharedCell` — Q8 cardinality amendment (Wave 8 W8-T25, 2026-05-10)

Phase 1.B-vm Wave 8 W8-T25 (`docs/cluster-audits/wave-8-structural-playbook.md`
§1) surfaced a structural gap in the §2.7.7 stack parallel-kind track
and the §2.7.8 cell-storage / module-binding parallel-kind track:

> `op_alloc_shared_local` / `op_alloc_shared_module_binding` in
> `executor/variables/mod.rs` allocate an `Arc<SharedCell>`, take the
> raw `Arc::into_raw` pointer, and need to label that cell-pointer
> slot in the parallel-kind track. The bytecode opcode docstring
> (`bytecode/opcode_defs.rs:1418`) anticipates the variant
> ("`NativeKind::Ptr(HeapKind::SharedCell)` is the parallel-track
> discriminator") but the `HeapKind` enum in `heap_variants.rs` has no
> matching ordinal. Without the variant, the alloc opcodes have no
> sound kind to push and the §2.7.8 #9 forbidden Bool-default fallback
> would be the only option — refused on sight. The opcodes therefore
> surfaced as `NotImplemented(SURFACE)` per §2.7.4.

This is the same cardinality-extension shape as the §2.7.9 / Wave-γ
G-heap-filter-expr amendment: a new HeapKind discriminator labels a
distinct `Arc<T>` payload type whose retain/release dispatch must
match 1:1 against the dispatch tables. Re-using an existing variant
(e.g. labeling `*const SharedCell` as `HeapKind::NativeView`) would
fire `Arc::increment/decrement_strong_count::<NativeViewData>`
against an `Arc<SharedCell>` pointer — wrong-type retain/release at
every retain/drop site. Same UB class as the pre-§2.7.9
`Arc<FilterNode>` mislabel; same fix shape.

**Decision (Q13 cardinality amendment):** add a new HeapKind variant
`SharedCell` (ordinal 19, immediately after `FilterExpr`'s ordinal 18
per §2.3's append-only ordering convention). The amendment is gated
by the §2.7.6 / Q8 cardinality bound's option (a) "Adding a
`NativeKind` variant to `shape-value` (gated by ADR-006 / Q-ruling —
same gate as ADR-005 §1 single-discriminator additions)" — applied
via this section.

**Why a new variant rather than off-label re-use of `NativeView` (or
any other existing label):** the payload type (`Arc<SharedCell>`)
requires `Arc::decrement_strong_count::<SharedCell>` at retire, which
in turn fires `SharedCell::Drop` — and `SharedCell::Drop` itself
dispatches its inner `value` bits through `drop_with_kind` per the
§2.7.8 / Q10 lockstep invariant on the cell's persistent
`kind: NativeKind` field. Mislabeling the cell-pointer slot would
walk a different destructor (e.g. `NativeViewData::Drop`) against the
`SharedCell`'s memory layout — UB at every retire. Per §2.7.7 /
§2.7.8, the `clone_with_kind` / `drop_with_kind` dispatch tables are
the **single source of truth** for typed-Arc retain/release; the
discriminator must match the payload 1:1.

**Why no parallel `HeapValue::SharedCell` enrichment is required by
ADR-005 §1 single-discriminator:** ADR-005 §1 says HeapValue is the
single discriminator for **heap-resident values** *that flow through
`HeapValue` materialization*. `Arc<SharedCell>` cell-pointer slots do
**not**: they are emitted to the kinded stack / module-binding store /
closure-capture cells via `Arc::into_raw(Arc<SharedCell>)` and
consumed via `Arc::from_raw(...)` directly on the slot bits, never
wrapped in `Box<HeapValue>` or accessed via `slot.as_heap_value()`.
Calling `slot.as_heap_value()` on a `SharedCell`-labeled slot is
**undefined behavior** (the slot bits are an `Arc::into_raw::<SharedCell>`
pointer, not a `*const HeapValue`); heap dispatch on
`SharedCell`-kinded slots goes through the kind label, not through
`as_heap_value()`. This matches the §2.7.9 FilterExpr precedent
exactly — same pure-discriminator role, same `as_heap_value()`
unsoundness, same justification for not enriching `HeapValue`.

`HeapKind::SharedCell` is therefore **a pure-discriminator HeapKind
variant without a corresponding `HeapValue` arm** (the second such
variant after FilterExpr; the §2.7.9 amendment explicitly allowed
this shape going forward).

**Mechanical lockstep updates (the new variant fans out to 4 dispatch
tables — every Q8/Q10 retain/release table):**

1. `crates/shape-value/src/heap_variants.rs` — `HeapKind::SharedCell`
   ordinal 19 (no `HeapValue` arm; pure-discriminator label per the
   precedent).
2. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` dispatch the new arm to
   `Arc::increment/decrement_strong_count::<SharedCell>`.
3. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror the same arm.
4. `crates/shape-value/src/heap_value.rs` — `TypedObjectStorage::drop`
   §2.7.8 mirror (a TypedObject field of kind
   `NativeKind::Ptr(HeapKind::SharedCell)` retires one
   `Arc<SharedCell>` strong-count share).
5. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
   §2.7.8 mirror. Yes — a `SharedCell` whose `kind` companion is
   `NativeKind::Ptr(HeapKind::SharedCell)` retires the inner
   `Arc<SharedCell>` share transitively. This is the closure-capture
   shape where one shared-mutable variable is itself captured shared
   into another closure (the inner `SharedCell` wraps an outer
   `SharedCell` cell-pointer).
6. `crates/shape-vm/src/executor/variables/mod.rs::op_alloc_shared_local`
   + `op_alloc_shared_module_binding` — push sites use
   `NativeKind::Ptr(HeapKind::SharedCell)` to label the
   `Arc::into_raw(Arc<SharedCell>) as u64` cell-pointer bits.

There is no fan-out to `printing.rs` / `comparison/mod.rs` /
`arithmetic/mod.rs` / `wire_conversion.rs` / `json_value.rs` because
`SharedCell`-labeled slots are an interior-only cell-pointer shape:
they do not flow through user-visible printing / comparison /
arithmetic / wire-serialisation surfaces. Loaders go through
`op_load_shared_local` / `op_load_shared_capture` which dispatch on
the cell's interior kind (stripping the `SharedCell` outer label
before pushing onto the kinded stack); the `SharedCell` outer label
is only ever observed by the four dispatch tables above + the alloc
sites.

**Cardinality cost:** `HeapKind` grows from 19 variants to 20; the
§2.7.6 Q8 bound (~25 constructors / ~5-10 scalar accessors max on
`KindedSlot`) is unchanged because `SharedCell` does not need a new
constructor or accessor — the existing `Ptr(HeapKind::*)` constructor
generic shape applies. Total dispatch surface grows by one arm per
table, no new dispatch tables.

**Forbidden alternatives this rules out:**

- "Just keep using `HeapKind::NativeView` (or `Closure`, or any other
  Arc-bearing arm) as a stand-in label for `*const SharedCell`."
  Wrong-type retain/release at every retain/drop site (same UB shape
  as the pre-§2.7.9 FilterExpr/NativeView mislabel). Refused:
  dispatch tables must match payloads 1:1.
- "Bool-default the alloc-site kind and let the load opcode peek at
  the cell to recover the kind." This is exactly the §2.7.7 #9 /
  §2.7.8 forbidden-shapes Bool-default fallback — refused on sight.
  The kind discriminator is sourced from the alloc-site (where the
  bits are sourced) and threaded through the parallel-kind track,
  not recovered later.
- "Probe the slot bits at retire to disambiguate
  `Arc<SharedCell>` from other `Arc<T>` shapes." This is the
  `(decode|tag|kind) (bridge|probe|helper|hop|translator|adapter)`
  family defection (CLAUDE.md "Renames to refuse on sight") —
  re-introducing bit-pattern probing as a substitute for a kind
  discriminator. Refused on sight.
- "Add a `HeapValue::SharedCell(Arc<SharedCell>)` arm so
  `slot.as_heap_value()` works." `Arc<SharedCell>` cell-pointer slots
  do not flow through `Box<HeapValue>` materialization; adding the
  arm would create a parallel materialization path the dispatch
  tables would then have to support, contradicting the
  pure-discriminator role. Refused per §2.7.9 precedent.
- "Transitional shim — call it `shared-cell bridge` /
  `shared-cell-pointer probe` / `Arc<SharedCell> hop` / `cell-bits
  translator` / `shared-storage adapter`." These are the W8-T25
  defection-attractor family per the playbook §3 #19 + CLAUDE.md
  "Renames to refuse on sight" `(decode|tag|kind|dispatch|value.call|
  closure.callback|frame.setup|callee|capture) (bridge|probe|helper|
  hop|translator|adapter|shim)` broader-family rule. Refused on sight.

**Out-of-scope this amendment:** routing `SharedCell` cell-pointer
slots through `HeapValue` materialization. No caller materializes a
`Box<HeapValue::SharedCell>` or expects `slot.as_heap_value()` to
return one. If a future caller needs `HeapValue` materialization of a
`SharedCell` cell-pointer (e.g. for a snapshot/wire surface that
crosses the §2.7.4 Phase-2c boundary), the work is a separate ADR
amendment with the same single-discriminator analysis applied to the
materialization path.
#### 2.7.13 `RefTarget` kinded redesign — `HeapValue::Reference(Arc<RefTarget>)` (Q14 ruling)

Phase 1.B-vm Wave 8 sub-cluster W8-T26 surfaced that the `MakeRef` /
`MakeFieldRef` / `MakeIndexRef` / `DerefLoad` / `DerefStore` /
`SetIndexRef` opcode family in
`crates/shape-vm/src/executor/variables/mod.rs` cannot rebuild against
the deleted `nanboxed::RefTarget` / `RefProjection` carrier. The pre-
strict-typing shape was a `ValueWord`-encoded enum:

```rust
// Pre-§2.7.13 (deleted with `ValueWord`):
enum RefTarget {
    Stack { slot: u16 },
    ModuleBinding { idx: u16 },
    Projected { root: Box<RefTarget>, projection: RefProjection },
}
enum RefProjection {
    TypedField { type_id: u16, field_idx: u16, field_type_tag: u16 },
    Index { index: u64 },
    MatrixRow { row: u32 },
}
// Packed into `ValueWord` via TAG_REF and chained tag-bits dispatch.
// Both `ValueWord` and the `nanboxed` / `RefProjection` modules are
// deleted by the strict-typing bulldozer (CLAUDE.md "Forbidden code"
// #1, #4, #6, the §2.7.7 / §2.7.8 forbidden-shapes list).
```

The deleted shape carried no `NativeKind` for the projected slot:
loading and storing through a ref relied on a `ValueWord`-shaped
discriminator (TAG_REF) and on `tag_bits::*` dispatch chained through
`RefProjection` to recover the projected slot's type. Both are
forbidden post-§2.7.7 / §2.7.8 (CLAUDE.md "Forbidden code" #4, #6;
playbook §3 forbidden #20 / #22).

**Decision (Q14 ruling):** the reference-value carrier becomes a typed-
`Arc<RefTarget>`-backed `HeapValue` arm, mirroring the §2.3 typed-Arc
shape every other heap variant uses. `RefTarget` is a kinded enum: each
variant carries the `NativeKind` of the *projected slot* alongside the
identifying place data, threaded from the producing-opcode emit per
§2.7.7. Loading and storing through a ref dispatch on the carried kind
via the same `clone_with_kind` / `drop_with_kind` tables §2.7.7 /
§2.7.8 / §2.7.9 / §2.7.10 / §2.7.11 already established — no new
dispatch surface.

**The kinded carrier (`shape-value/src/heap_variants.rs`):**

```rust
// New HeapKind variant (next free ordinal — 20 if W8-T25 lands SharedCell
// at 19 first, otherwise 19 — per §2.3 append-only ordering, paired with
// W8-T25 to avoid merge collision):
pub enum HeapKind {
    // ... String=0 .. HashMap=17 .. FilterExpr=18 ..
    Reference,    // (Wave 8 W8-T26, 2026-05-10)
}

// New HeapValue arm carrying typed Arc per §2.3 — provided ONLY for the
// HeapKind↔HeapValue symmetry property (mirror of §2.7.9 FilterExpr).
// Reference-labeled slot bits are `Arc::into_raw(Arc<RefTarget>) as u64`
// directly, NOT a `Box::into_raw(Box<HeapValue>)` wrap; calling
// `slot.as_heap_value()` on a Reference-labeled slot is undefined
// behavior, same as FilterExpr per §2.7.9. The `clone_with_kind` /
// `drop_with_kind` dispatch tables retain/release `Arc<RefTarget>`
// directly, NOT through `HeapValue`.
pub enum HeapValue {
    // ... existing arms ...
    Reference(std::sync::Arc<crate::reference::RefTarget>),
}

// New module `shape-value/src/reference.rs`:
pub enum RefTarget {
    /// Reference to a local slot in a specific stack frame.
    /// `kind` is the `NativeKind` of the slot at the time of `MakeRef`,
    /// sourced from `FrameDescriptor.slots[slot_index]` at emit time.
    Local { frame_index: u32, slot_index: u32, kind: NativeKind },

    /// Reference to a module binding.
    /// `kind` is the binding's stored kind from the module-binding
    /// parallel-kind track (§2.7.8 / Q10).
    ModuleBinding { binding_idx: u32, kind: NativeKind },

    /// Projected reference into a typed-object field.
    /// `receiver` is the typed `Arc<TypedObjectStorage>` payload (per
    /// §2.4 `from_typed_object` constructor — slot bits are
    /// `Arc::into_raw(Arc<TypedObjectStorage>)`, never wrapped in
    /// `Box<HeapValue>`); `field_offset` is the slot index inside
    /// `TypedObjectStorage.slots` (the schema-resolved `field_idx` from
    /// `Operand::TypedField`); `kind` is the projected slot's
    /// `NativeKind`, sourced from the emitter's `field_type_tag` via
    /// the existing `field_tag_to_heap_native_kind` mapping
    /// (`executor/typed_object_ops.rs:86`) extended to inline scalars.
    TypedField {
        receiver: std::sync::Arc<crate::heap_value::TypedObjectStorage>,
        field_offset: u32,
        kind: NativeKind,
    },

    /// Projected reference into a typed-array element.
    /// `receiver` is the typed `Arc<TypedArrayData>` payload (per §2.4
    /// `from_typed_array` constructor); `index` is the element index
    /// (post-bounds-check at construction); `elem_kind` is the element-
    /// type `NativeKind` recovered from the receiver `TypedArrayData`'s
    /// variant at emit time (the producing opcode knows what element
    /// kind it pushed).
    TypedIndex {
        receiver: std::sync::Arc<crate::heap_value::TypedArrayData>,
        index: u64,
        elem_kind: NativeKind,
    },
}
```

**Why `HeapValue::Reference(Arc<RefTarget>)` rather than a separate
discriminator:** ADR-005 §1 single-discriminator. Every kind variant
the runtime/VM sees on a ref-bearing slot dispatches through
`HeapValue` exactly like every other heap-resident value. The runtime-
tier carrier at boundaries is `KindedSlot` per §2.7.6 / Q8; for a ref,
`kind == NativeKind::Ptr(HeapKind::Reference)` and
`slot.as_heap_value() => HeapValue::Reference(arc)` recovers the
typed `Arc<RefTarget>`. No parallel sum type, no `Box<RefTarget>`
slot wrapping (forbidden by §2.3), no `Arc<NativeViewData>`-style
type-confusion off-label re-use (the §2.7.9 FilterExpr precedent
applies — the discriminator must match the payload 1:1 at the
dispatch table).

**Why kind is a field on each `RefTarget` variant rather than fabricated
at projection time:** §2.7.7 forbidden-shape #4 (tag-bit chains) and
§2.7.7 / §2.7.8 / §2.7.9 / §2.7.10 / §2.7.11 invariant: the kind
comes from the producing-opcode emit, never fabricated downstream. At
`MakeRef` / `MakeFieldRef` / `MakeIndexRef` time, the compiler knows
which slot it's projecting and what kind that slot has —
`FrameDescriptor.slots[slot_index]` for `Local`, the module-binding's
stored kind for `ModuleBinding`, the operand-encoded
`field_type_tag` (mapped through `field_tag_to_heap_native_kind`) for
`TypedField`, and the receiver `TypedArrayData`'s element-kind for
`TypedIndex`. Every kind threads from a kind-source the executor
already trusts (§2.7.7 stack parallel-kind track, §2.7.8 cell /
module-binding parallel-kind tracks, `TypedObjectStorage.field_kinds`,
`TypedArrayData::variant_kind()`).

**The dispatch shape (`op_load_ref` / `op_store_ref` / `op_set_index_ref`):**

```rust
// op_deref_load — pop a kinded ref slot, recover RefTarget, push the
// projected slot's value with the carried kind:
fn op_deref_load(&mut self, instruction: &Instruction) -> Result<(), VMError> {
    let (ref_bits, ref_kind) = self.pop_kinded()?;
    debug_assert_eq!(ref_kind, NativeKind::Ptr(HeapKind::Reference));
    // SAFETY: kind == Ptr(HeapKind::Reference) is sufficient by
    // ADR-005 §1 single-discriminator + §2.7.9 dispatch-table 1:1
    // payload-discriminator invariant — `ref_bits` is the
    // `Arc::into_raw(Arc<HeapValue::Reference(_)>)` pointer, and the
    // §2.7.7 stack parallel-kind track owns one share for us.
    let hv = unsafe { ValueSlot::from_raw(ref_bits).as_heap_value() }
        .ok_or(VMError::InvalidOperand)?;
    let HeapValue::Reference(rt_arc) = hv else { return Err(...); };
    let (out_bits, out_kind) = match &**rt_arc {
        RefTarget::Local { frame_index, slot_index, kind } => {
            let frame = &self.call_stack[*frame_index as usize];
            let bits = frame.locals[*slot_index as usize];
            (bits, *kind)
        }
        RefTarget::ModuleBinding { binding_idx, kind } => {
            let bits = self.module_binding_read_raw(*binding_idx as usize);
            (bits, *kind)
        }
        RefTarget::TypedField { receiver, field_offset, kind } => {
            // receiver is &Arc<HeapValue::TypedObject(_)>
            let HeapValue::TypedObject(storage) = &**receiver else { return Err(...); };
            (storage.slots[*field_offset as usize].raw(), *kind)
        }
        RefTarget::TypedIndex { receiver, index, elem_kind } => {
            let HeapValue::TypedArray(data) = &**receiver else { return Err(...); };
            (data.read_index_raw(*index as usize)?, *elem_kind)
        }
    };
    // WB2.4 retain-on-read: the projected source keeps its share
    // (the place is borrowed, not consumed); the pushed slot needs
    // an independent share.
    crate::executor::vm_impl::stack::clone_with_kind(out_bits, out_kind);
    self.push_kinded(out_bits, out_kind)?;
    // Drop our share of the ref carrier — the §2.7.7 pop transferred
    // one share to us; release it via the kinded dispatch table.
    crate::executor::vm_impl::stack::drop_with_kind(ref_bits, ref_kind);
    Ok(())
}

// op_deref_store — pop value (kinded), pop ref (kinded), write value
// into the projected slot via the carried kind:
fn op_deref_store(&mut self, instruction: &Instruction) -> Result<(), VMError> {
    let (val_bits, val_kind) = self.pop_kinded()?;
    let (ref_bits, ref_kind) = self.pop_kinded()?;
    debug_assert_eq!(ref_kind, NativeKind::Ptr(HeapKind::Reference));
    // ... recover RefTarget, dispatch on variant ...
    // For each variant, the write path:
    //   1. Read the prior occupant's bits via the projected place.
    //   2. drop_with_kind(prior_bits, target.kind) — the place owned
    //      a share that's about to be replaced.
    //   3. Write val_bits to the place.
    //   4. The pushed val owns a share; the place now owns it
    //      (transfer of ownership; no clone_with_kind, no
    //      drop_with_kind on val_bits).
    //   5. drop_with_kind(ref_bits, ref_kind) — release the ref carrier
    //      share we popped.
    //   6. Cross-check `val_kind == target.kind` (debug_assert) —
    //      the producing opcode pushed a value of the kind the place
    //      expects, by ADR-006 §2.7.5.1 "stack contents are post-proof".
    Ok(())
}
```

**`MakeRef` / `MakeFieldRef` / `MakeIndexRef` construction:**

```rust
// op_make_ref — operand is Operand::Local(slot) or
// Operand::ModuleBinding(idx). Source kind from the corresponding
// parallel-kind track:
fn op_make_ref(&mut self, instruction: &Instruction) -> Result<(), VMError> {
    let rt = match instruction.operand {
        Some(Operand::Local(slot)) => {
            let kind = self.current_frame_descriptor()?
                .slot(slot as usize)
                .ok_or(VMError::InvalidOperand)?;
            let frame_index = (self.call_stack.len() - 1) as u32;
            RefTarget::Local { frame_index, slot_index: slot as u32, kind }
        }
        Some(Operand::ModuleBinding(idx)) => {
            let (_, kind) = self.module_binding_read_kinded_raw(idx as usize);
            RefTarget::ModuleBinding { binding_idx: idx as u32, kind }
        }
        _ => return Err(VMError::InvalidOperand),
    };
    let arc = std::sync::Arc::new(HeapValue::Reference(std::sync::Arc::new(rt)));
    let bits = std::sync::Arc::into_raw(arc) as u64;
    self.push_kinded(bits, NativeKind::Ptr(HeapKind::Reference))
}

// op_make_field_ref — operand is Operand::TypedField{type_id, field_idx,
// field_type_tag}. Pops a base-ref carrier, projects through the field:
fn op_make_field_ref(&mut self, instruction: &Instruction) -> Result<(), VMError> {
    let Some(Operand::TypedField { field_idx, field_type_tag, .. }) =
        instruction.operand else { return Err(VMError::InvalidOperand); };
    let (base_bits, base_kind) = self.pop_kinded()?;
    debug_assert_eq!(base_kind, NativeKind::Ptr(HeapKind::Reference));
    // Recover the base RefTarget; resolve the receiver Arc<HeapValue>
    // (TypedObject) by chasing the base ref through one DerefLoad-equivalent
    // step, then build a TypedField projection. Kind sourced from
    // field_type_tag via field_tag_to_native_kind (extended to scalars).
    let receiver: std::sync::Arc<HeapValue> = ...;  // resolved per base RefTarget
    let kind = field_tag_to_native_kind(field_type_tag)?;
    let rt = RefTarget::TypedField { receiver, field_offset: field_idx as u32, kind };
    let arc = std::sync::Arc::new(HeapValue::Reference(std::sync::Arc::new(rt)));
    let bits = std::sync::Arc::into_raw(arc) as u64;
    // Release the base-ref carrier we popped.
    crate::executor::vm_impl::stack::drop_with_kind(base_bits, base_kind);
    self.push_kinded(bits, NativeKind::Ptr(HeapKind::Reference))
}

// op_make_index_ref — pops [base_ref, index] kinded; index is Int64.
// Resolves receiver to Arc<HeapValue::TypedArray(_)>; reads element
// kind from `TypedArrayData::variant_kind()`:
fn op_make_index_ref(&mut self, instruction: &Instruction) -> Result<(), VMError> {
    let (idx_bits, idx_kind) = self.pop_kinded()?;
    debug_assert_eq!(idx_kind, NativeKind::Int64);
    let (base_bits, base_kind) = self.pop_kinded()?;
    debug_assert_eq!(base_kind, NativeKind::Ptr(HeapKind::Reference));
    // Recover receiver; bounds-check; build TypedIndex projection.
    let receiver: std::sync::Arc<HeapValue> = ...;
    let HeapValue::TypedArray(arr) = &*receiver else { return Err(...); };
    let elem_kind = arr.variant_kind();
    let rt = RefTarget::TypedIndex { receiver: receiver.clone(), index: idx_bits, elem_kind };
    let arc = std::sync::Arc::new(HeapValue::Reference(std::sync::Arc::new(rt)));
    let bits = std::sync::Arc::into_raw(arc) as u64;
    crate::executor::vm_impl::stack::drop_with_kind(base_bits, base_kind);
    self.push_kinded(bits, NativeKind::Ptr(HeapKind::Reference))
}
```

**Lockstep dispatch-table updates (the new variant fans out to the same
6 dispatch sites §2.7.9 enumerated for `FilterExpr` — every Q8/Q10
retain/release table):**

1. `crates/shape-value/src/heap_variants.rs` — `HeapKind::Reference`
   ordinal 19 + `HeapValue::Reference(Arc<RefTarget>)` arm + `kind()`
   / `is_truthy()` / `type_name()` / `Display` updates.
2. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` dispatch the new arm via the standard
   `Box<HeapValue>` path (the slot bits are
   `Arc::into_raw(Arc<HeapValue>)`; the inner
   `Arc<RefTarget>` is owned by the `HeapValue::Reference` arm and
   the standard `Drop` impl on `HeapValue` decrements its
   strong-count). This is the **same dispatch as every other typed-
   `Arc<HeapValue>` arm** — no new arm in the per-`HeapKind`
   dispatch tables, because the discriminator's payload is
   `Arc<HeapValue>` (single ADR-005 §1 discriminator), not a custom
   `Arc<RefTarget>` payload escaping the `HeapValue` shape.
3. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror via the same `HeapValue`-arm path.
4. `crates/shape-value/src/heap_value.rs` — `TypedObjectStorage::drop`
   §2.7.8 mirror handles `HeapKind::Reference`-kinded fields
   (TypedObject can hold a ref-typed field; the destructor
   dispatches `Arc::decrement_strong_count::<HeapValue>` on the
   slot's `Arc<HeapValue>`, same as every other heap-kinded field).
5. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
   §2.7.8 mirror, same `HeapValue`-arm path.
6. `crates/shape-vm/src/executor/variables/mod.rs` — `op_make_ref`
   family bodies (Phase 2 of W8-T26).
7. Knock-on exhaustive-match additions in `printing.rs`,
   `comparison/mod.rs`, `arithmetic/mod.rs`,
   `objects/typed_access.rs` (kind→type-name maps);
   `wire_conversion.rs`, `json_value.rs` (HeapValue serialization
   rejection arms — `Reference` does not cross the wire boundary;
   refs are within-program data, like `FilterExpr`).

**Cardinality cost:** `HeapKind` grows from 19 variants to 20; the
§2.7.6 / Q8 carrier-API bound is unchanged because `Reference` does
not need a new `KindedSlot` constructor or accessor — the existing
`Ptr(HeapKind::*)` constructor generic shape applies, and heap
dispatch is `slot.as_heap_value()` + `HeapValue::Reference` match
per ADR-005 §1.

**Why receiver is `Arc<HeapValue>` rather than a stack-frame pointer
or a raw `*mut TypedObjectStorage`:** the receiver share keeps the
heap object alive while the ref exists (a ref outliving its
receiver would be dangling — same lifetime contract as every other
heap reference). `Arc<HeapValue>` is the §2.3 typed-Arc shape; the
projection variant carries one strong-count for the receiver; the
ref's `Arc<RefTarget>` carries its own strong-count via the
enclosing `HeapValue::Reference` arm. Stack-frame `Local`-flavored
refs do **not** carry an `Arc<HeapValue>` — they carry a
`(frame_index, slot_index, kind)` triple and rely on
**ref-escape analysis** (`mir/lowering/mod.rs`, ADR-006 §3.1)
to prevent a `Local`-shaped ref from escaping its frame. Storing a
`Local`-shaped ref into a closure capture or returning it from a
function is rejected at compile time by the existing escape
analysis (the §3.1 boundary the MIR audit Item #4 names as the
highest-priority follow-up); this ruling does not relax that
boundary. A future ruling that loosens ref-escape analysis would
require either promoting `Local` to a frame-`Arc`-shared cell
(SharedCell variant per §2.7.8 / W8-T25 amendment) or rejecting
the escaping path at the MIR level.

**Forbidden shapes this rules out (mirror of §2.7.9 / §2.7.10 /
§2.7.11 forbidden lists, applied to the ref carrier):**

- **`ValueWord` revival.** The pre-§2.7.13 `nanboxed::RefTarget`
  packed into a TAG_REF `ValueWord` is the deleted shape (CLAUDE.md
  "Forbidden code" #1, "Forbidden rationalizations" #2). No
  resurrection, not as a "serialization helper", not as a
  "compatibility layer".
- **Tag-bit chains for `RefProjection`.** The deleted
  `RefProjection` enum dispatched through chained `tag_bits::*`
  decoding to recover the projected slot's type at deref time.
  Forbidden by §2.7.7 #4 / #7. The kinded redesign carries the
  projected slot's `NativeKind` as a field on each variant,
  threaded from the producing-opcode emit; deref reads the field,
  no decoding.
- **Kind fabrication at projection time** ("the projection knows
  the parent's kind, derive the child's at deref"). Forbidden by
  §2.7.7 / §2.7.8 / §2.7.9 / §2.7.10 / §2.7.11 invariant: kind
  comes from the producing-opcode emit, never fabricated
  downstream. `MakeFieldRef` knows `field_type_tag`;
  `MakeIndexRef` knows the receiver's element kind; `MakeRef`
  knows the slot's kind from the parallel-kind track. The
  emitter is the kind-source.
- **`Box<dyn RefTrait>` or any non-`Arc<HeapValue>` discriminator.**
  ADR-005 §1 single-discriminator. Layers above HeapValue dispatch
  through `HeapValue::kind()`; introducing a parallel sum type
  whose variants project 1:1 to `RefTarget` variants is the
  defection ADR-005 enumerates as "every parallel discriminator
  has eventually drifted".
- **`HeapKind::Reference` as a pure-discriminator variant** (mirror
  of §2.7.9 FilterExpr) — rejected. FilterExpr's pure-discriminator
  shape is justified by the §2.7.9 ruling because filter-expression
  payloads are emitted directly as `Arc::into_raw(Arc<FilterNode>)`
  to the kinded stack and never wrapped in `HeapValue`. Refs do
  not have that justification: refs flow through the same opcodes
  as every other heap value (`LoadLocal` / `StoreLocal` /
  `MakeFieldRef` / closure capture), can be stored in
  `TypedObjectStorage` slots and `TypedArrayData::HeapValue`
  buffers, and need `slot.as_heap_value()` to return a
  `HeapValue::Reference(_)` for downstream deref. Pure-
  discriminator status would re-introduce the §2.7.9 type-confusion
  pattern at a different layer.
- **`RefTarget::Projected { root: Box<RefTarget>, projection: ... }`
  (recursive nesting).** The deleted shape allowed arbitrary
  projection-of-projection chains decoded at deref time. The
  kinded redesign flattens to a single projection level per
  `RefTarget` variant (`TypedField` and `TypedIndex` carry a
  *resolved* `Arc<HeapValue>` receiver, not a nested ref). Chained
  property access (`&a.b.c`) is built as a sequence of
  `MakeFieldRef` opcodes that collapse the chain at construction
  time — the emitter resolves nested `TypedFieldPlace` paths in
  `helpers_reference.rs:collect_property_access_chain`, and each
  `MakeFieldRef` step projects from the resolved parent receiver.
  Recursive `RefTarget` would re-introduce nested-decoding
  dispatch at deref time, the deleted §2.7.7 #4 pattern.
- **`KindedSlot::as_ref_target()` per-heap-variant accessor on
  `KindedSlot`.** §2.7.6 / Q8 carrier-API bound: heap dispatch
  goes through `slot.as_heap_value()` + `HeapValue::Reference`
  match. No parallel discrimination on `HeapKind` at the carrier.
- **Transitional shims preserving deleted RefTarget-shape names**
  (`RefTargetLegacy` / `nanboxed::as_ref_target` /
  `decode_ref_target_bits` / `pack_ref_into_value_word` /
  `read_ref_target` / `write_ref_target`). Same §2.7.7 #1 rule —
  the W-series "borrowed bits with call-pattern invariants"
  defection-attractor at the ref-carrier layer. Migrate every
  `op_make_ref` family entry in-wave; do not preserve a kind-blind
  variant as a transitional layer. The B6 SURFACE messages (the
  current `op_make_ref` body text) cite the deleted shape by name
  (`nanboxed::RefTarget` / `RefProjection`) per CLAUDE.md
  "describe deleted code by name".
- **Defection-attractor descriptors** — "ref-target bridge",
  "ref-projection translator", "deref probe", "place-resolution
  helper", "ref-decode hop", "boundary adapter for ref carrier".
  Per the 2026-05-09 user ruling broadening the W-series rename
  family + playbook §3 forbidden #20, any descriptor of the
  deleted ref carrier that uses bridge / probe / helper / hop /
  translator / adapter framing belongs to the same defection-
  attractor family CLAUDE.md "Renames to refuse on sight"
  enumerates. Describe the deleted carrier by name (the pre-
  §2.7.13 `nanboxed::RefTarget` / `RefProjection` ValueWord
  encoding) or by deletion-fate (the kind-blind `ValueWord`-
  shape ref carrier), never by hypothetical role.

**Performance characteristics:**

- `MakeRef` cost: one `Arc::new(HeapValue::Reference(Arc::new(rt)))`
  allocation pair (two atomic refcount initializations) +
  `push_kinded` (1 word + 1 byte to the parallel tracks). The
  double-Arc shape (outer `Arc<HeapValue>`, inner `Arc<RefTarget>`)
  is canonical for typed-Arc heap variants per §2.3 — every
  `HeapValue::TypedArray(Arc<TypedArrayData>)` /
  `HeapValue::TypedObject(Arc<TypedObjectStorage>)` arm is the same
  shape. **No new dispatch surface; the cost is one allocation
  pair plus the kinded push, the same as constructing any other
  `HeapValue` arm.**
- `MakeFieldRef` / `MakeIndexRef`: pop the base-ref carrier (1
  word + 1 byte read), recover the inner `RefTarget` (one `Arc`
  deref), build a new `RefTarget::TypedField` /
  `RefTarget::TypedIndex` (an `Arc::clone` on the receiver — one
  atomic refcount bump), wrap in `Arc<HeapValue>` (one
  allocation), push (1 word + 1 byte). Dropping the popped
  base-ref releases its share via the standard `HeapValue` Drop.
- `DerefLoad` / `DerefStore`: pop the ref carrier (1 word + 1
  byte read), recover `RefTarget` (one `Arc` deref + match),
  read/write the projected slot's `u64` (one indexed read/write
  on the receiver's slot/element buffer), `clone_with_kind` /
  `drop_with_kind` for the loaded/stored value (1 byte cmpxchg
  target + matching `Arc::increment/decrement_strong_count`),
  push the loaded value or release the stored value's prior
  occupant. **Strictly the same work the deleted `RefProjection`
  dispatch did, minus the chained `tag_bits` decode the strict-
  typing bulldozer removed.**
- Memory: `RefTarget` enum is at most `{ tag: u8, payload: max(u32+u32+kind,
  Arc<HeapValue> + u64 + kind, Arc<HeapValue> + u64 + kind) }`
  ≈ 24-32 bytes. Wrapped in `Arc<RefTarget>` (24 bytes inner +
  8 bytes refcount-prefix), wrapped in `Arc<HeapValue>` (the
  enclosing slot pointer). Total per-ref allocation: ~64 bytes,
  one per `MakeRef`. References are short-lived in typical Shape
  programs (immediate borrow into a builtin's `&` parameter, or
  field-mutation via `&obj.field = value`); the allocation cost
  amortizes against the immediate consumption.
- IC fast path: refs flow through the standard `LoadLocal` /
  `StoreLocal` paths via their `NativeKind::Ptr(HeapKind::Reference)`
  kind label. No special IC entry; the per-receiver-kind IC
  caching in `ic_fast_paths.rs` already handles
  `NativeKind::Ptr(_)` receivers uniformly.

**Cross-check on debug builds:** at every `op_make_ref` /
`op_make_field_ref` / `op_make_index_ref` site, the kind written into
the new `RefTarget` variant should match the kind-source's emitter
intent. `debug_assert_eq!(rt.kind, expected_kind)` at construction
catches kind drift during development; `debug_assert_eq!(val_kind,
target.kind)` at `op_deref_store` catches store-side kind drift; in
release builds these compile out.

**Migration scope (Wave 8 sub-cluster W8-T26 territory, Phase 2):**

1. Add `HeapKind::Reference` ordinal 19 +
   `HeapValue::Reference(Arc<RefTarget>)` arm in
   `crates/shape-value/src/heap_variants.rs`. Update `kind()` /
   `is_truthy()` / `type_name()` / `Display`.
2. Add `crates/shape-value/src/reference.rs` defining
   `RefTarget` enum with the four variants
   (`Local` / `ModuleBinding` / `TypedField` / `TypedIndex`).
3. Lockstep dispatch-table updates (#1-#5 in the lockstep list above)
   — every new `HeapValue` arm follows the same Q8/Q10 mirror
   pattern. The dispatch is the standard `Arc<HeapValue>` path,
   not a new kind-specific arm; the lockstep work is exhaustive-
   match additions, not new dispatch logic.
4. Migrate `op_make_ref` / `op_make_field_ref` / `op_make_index_ref`
   / `op_deref_load` / `op_deref_store` / `op_set_index_ref` in
   `crates/shape-vm/src/executor/variables/mod.rs` from the current
   `NotImplemented(SURFACE)` stubs to real bodies per the dispatch
   shapes above. Bodies use only kinded-API primitives
   (`pop_kinded` / `push_kinded` / `clone_with_kind` /
   `drop_with_kind`) — no fabrication, no tag decode, no
   `is_heap()` probe, no Bool-default fallback.
5. Add `field_tag_to_native_kind` (extension of
   `field_tag_to_heap_native_kind` in
   `executor/typed_object_ops.rs:86` that handles inline scalars
   for `MakeFieldRef`'s kind-source). The function takes a
   `field_type_tag` and returns `NativeKind` — `FIELD_TAG_F64
   => NativeKind::Float64`, `FIELD_TAG_I64 => NativeKind::Int64`,
   etc.
6. JIT codegen (Wave 10) emits the kinded `RefTarget`
   construction at `MakeRef` / `MakeFieldRef` / `MakeIndexRef`
   sites — same lockstep discipline as the stack-side §2.7.7
   codegen.

**Out-of-scope this ruling:**

- Snapshot/restore of in-flight ref-bearing slots crossing a §2.7.4
  Phase-2c snapshot boundary. Refs are short-lived and typically do
  not cross suspension boundaries; if a future async value-call
  suspends with a live ref on the stack, the persisted shape gets
  its own follow-up amendment.
- Cross-task ref sharing. Refs are not `Send` by construction
  (the `Local` variant carries a frame index that's only
  meaningful within the originating task); cross-task ref escape
  is rejected by `B0014 NonSendableAcrossTaskBoundary`, the same
  boundary §3.3 enforces for non-`Send` values.
- Wire-format serialization of ref-bearing values. Refs do not
  cross the wire boundary (same as `FilterExpr` — within-program
  data only); `wire_conversion.rs` / `json_value.rs` reject the
  arm with a clear error per the §2.7.5.1 stable-FFI boundary
  rule.
- Loosening of ref-escape analysis (the MIR audit Item #4 boundary
  CLAUDE.md "MIR Audit Status" names as the highest-priority
  follow-up). This ruling preserves the existing escape boundary;
  loosening it would be a separate ADR amendment with measurement.

#### 2.7.14 JIT array FFI rebuild — `JitArray` deletion and kinded `TypedArray<T>` re-introduction (Q15 deferral)

W10-misc (close `4b978a4`, 2026-05-10) deleted the `JitArray = UnifiedArray`
type alias in `crates/shape-jit/src/jit_array.rs` after
`shape_value::unified_array::UnifiedArray` (1,134 LoC) was bulldozed in
commit `0270dd4`. W10-cascade (close `60f9b7c`, 2026-05-10) followed by
surface-and-stop'ing every `shape-jit` consumer that walked the deleted
heap layout — 19 production sites across `ffi/control/mod.rs` (8),
`ffi/call_method/{array,matrix}.rs` (5), `ffi/object/{object_ops,
property_access}.rs` (3), `ffi/references.rs`, `ffi_symbols/intrinsics/
mod.rs`, `ffi_symbols/data_access/mod.rs` — bringing `shape-jit --lib`
to a clean build (`51 → 0` errors). The full array-FFI registration
surface (`ffi_symbols/array_symbols.rs::register_array_symbols` /
`declare_array_functions`) is a no-op pending this ruling.

W12-jit-array (audit `9bd19f8`, 2026-05-10) confirmed the rebuild
crosses Cranelift codegen, FFI registration, method-dispatch ABI
threading, and consumer-site translation in lockstep — multi-week
work that exceeds any single-wave budget. Q15 formalizes the
deferral and the architectural decisions the rebuild must make
before re-introducing array codegen in the JIT.

**The deleted shape was kind-on-heap.** Pre-strict-typing `UnifiedArray`
packed an `ArrayElementKind` byte and a typed-mirror pointer into the
`#[repr(C)]` heap object alongside the `Vec<u64>` data buffer (offsets
DATA=0, LEN=8, CAP=16, TYPED_DATA=24, ELEMENT_KIND=32, relative to the
post-header data field). Element kind was recovered at runtime by
loading the byte at offset 32. Every JIT-FFI consumer (`jit_new_array`,
`jit_array_get/push/pop`, `jit_array_first/last/min/max`, `jit_slice`,
`jit_range`, `jit_make_range`, `jit_array_filled`, `jit_array_reverse`,
`jit_array_push_*`, `jit_array_zip`, `jit_hof_array_alloc/push`,
`jit_array_info`) consumed this kind byte to dispatch element
operations. This is the §2.7.7 #4 / #7 forbidden pattern — kind
recovered at runtime via heap-byte decode rather than threaded from
the producing call signature. The deletion was correct under §2.7.5
("kind stamped at JIT compile time from the call signature, not on
the heap object").

**The two architecturally-distinct rebuild routes** (no ruling between
them yet — Q15-A is the open question this deferral parks):

1. **Route A — monomorphized `Arc<TypedArrayData>` per element kind.**
   Match `shape_value::v2::typed_array::TypedArray<T>` (24-byte header,
   one allocation per concrete element type, `HEAP_KIND_V2_TYPED_ARRAY`
   discriminator). Each call signature variant of `jit_new_array_*` /
   `jit_array_get_*` / etc. monomorphizes on a single `NativeKind` per
   array (Float64 / Int64 / Int32 / Bool / String / Ptr(_)).
   Cranelift offsets are derived against the `TypedArray<T>` layout
   (data=8, len=16, cap=20). Element-kind threading is per-call-site
   from the JIT's `FrameDescriptor.slots` and the `Operand::TypedArray`
   element type tag at the producing opcode. Compatible with §2.7.6 /
   Q8 cardinality bound: heap dispatch goes through
   `slot.as_heap_value()` → `HeapValue::TypedArray(arc)` →
   `arc.element_kind()`, no parallel-kind side track on the heap
   object.

   *Cost:* ~10–14 monomorphized variants of every array-FFI entry
   point (one per `NativeKind` arm); FFI symbol registration grows
   ~10× by line count but each entry is small. No new heap kind.

   *Compatibility:* Direct match for the existing v2-runtime
   `TypedArray<T>` (`shape_value/src/v2/typed_array.rs`), already
   used by VM-side typed-opcode array handlers
   (`crates/shape-vm/src/executor/objects/array_*.rs`). VM↔JIT slot
   ABI parity preserved (§4.1 uniform slot ABI).

2. **Route B — unified `Vec<u64>` data + parallel `Vec<NativeKind>`
   element-kind track per §2.7.7 / §2.7.8 cell-storage pattern.** A
   single heap kind (`HEAP_KIND_JIT_ARRAY`) carries `(Vec<u64> data,
   Vec<NativeKind> elem_kinds, len, cap)`. Element-kind threading is
   per-element on the heap, but lookup is via the parallel-track
   pattern §2.7.7 stack ABI / §2.7.8 cell ABI established —
   `clone_with_kind` / `drop_with_kind` dispatch the per-element
   retain/release without tag decode.

   *Cost:* No multiplication of FFI entries. Heap object grows by
   ~16 bytes (parallel `Vec<NativeKind>`). Per-element push/pop
   touches two vectors (lockstep §2.7.7 invariant).

   *Incompatibility:* Diverges from `TypedArray<T>` — the JIT and VM
   would carry distinct array shapes, breaking the §4.1 uniform slot
   ABI. Also requires a new `HeapKind` variant (Q8 cardinality
   amendment process per §2.7.6 / Wave-γ G-heap-filter-expr / W8-T25
   SharedCell). The §2.7.7 parallel-`Vec<NativeKind>` pattern was
   designed for the stack and for cells with a small fixed slot
   count — extending to heap arrays with unbounded element count
   keeps the lockstep invariant but doubles the per-element memory
   bandwidth on push/pop.

**Default expectation: Route A.** §2.7.5 cross-crate ABI policy and
§4.1 uniform slot ABI both push toward monomorphization on
`TypedArray<T>`. Route B is preserved here only because the
multiplication cost (~10–14× FFI entries) crosses the boundary
where "small" rebuild work becomes "redesign FFI registration".
A measured comparison is required before committing.

**The four lockstep dependencies** the rebuild must thread (none of
which can land independently — they must close in a single wave to
keep `shape-jit --lib` clean):

1. **`jit_array.rs` layout decision** — Route A (per-element-kind
   monomorphization, no public type alias; `TypedArray<T>` is the
   carrier) or Route B (single `JitArrayKinded` type with parallel
   `Vec<NativeKind>`). Either way the existing
   `pub const {DATA,LEN,CAP,TYPED_DATA,ELEMENT_KIND}_OFFSET` constants
   in `jit_array.rs` are replaced — Route A maps to
   `TypedArray<T>`'s offsets (DATA=8, LEN=16, CAP=20, no
   ELEMENT_KIND on the heap), Route B re-derives all five plus an
   ELEM_KINDS_OFFSET pointer to the parallel `Vec<NativeKind>`
   buffer.

2. **`ffi/array.rs` body re-introduction.** All entry points
   (currently surfaced) re-emit, kind-threaded per the chosen route.
   Route A monomorphizes per-`NativeKind`; Route B threads element
   kind explicitly via a `NativeKind` parameter at each entry.

3. **`ffi_symbols/array_symbols.rs` registration.** No-op stubs
   replaced with the per-route registration. Route A registers
   ~10–14 monomorphized symbols per entry; Route B registers a
   single kinded symbol per entry. The same applies to the cascade
   sites in `ffi_symbols/intrinsics/mod.rs` and
   `ffi_symbols/data_access/mod.rs`.

4. **Method-dispatch ABI integration (§2.7.10 / Q11).** The
   `MethodFnV2` shape `fn(&mut VM, &[KindedSlot], ctx) -> Result<KindedSlot, _>`
   already lands for VM-side method dispatch. The JIT-side method-call
   FFI shims (`ffi/call_method/array.rs::call_array_method` and the
   matrix/string variants) wrap this contract. Route A naturally
   extends — each monomorphized array entry produces a `KindedSlot`
   with the matching arm; Route B requires the parallel-kind track
   to be readable through the §2.7.10 carrier.

**Forbidden under any rebuild** (mirrors §2.7.7 / §2.7.8 / §2.7.10
forbidden lists, applied to the array carrier):

- **`JitArray` revival under any renamed shape.** The deleted
  `UnifiedArray` heap layout with the on-heap `ELEMENT_KIND` byte at
  offset 32 is the §2.7.7 #4 / #7 pattern. CLAUDE.md "Renames to
  refuse on sight" extends to this carrier — "JitArray bridge",
  "UnifiedArray shim", "tag-bit array carrier", "boundary array
  view", "element-kind helper", "array-decode probe", "array
  translator" framing all belong to the broader defection-attractor
  family rule (deleted `(decode|tag|kind|dispatch|...) (bridge|
  probe|helper|hop|translator|adapter|shim)` regex). Describe the
  deleted code by name (`UnifiedArray`, `ArrayElementKind`,
  `JitArray::from_heap_bits`, `JitArray::heap_box`,
  `JitArray::from_vec`) or by deletion-fate (the W10-misc-deleted
  kind-on-heap array carrier), never by hypothetical role.

- **Bool-default fallback for unknown element kinds.** §2.7.7 #9 /
  W10 jit-playbook §3 / §5 surface-and-stop instead. Every consumer
  that lacks a kind source is a kind-source gap, not a Bool-default
  arm.

- **`tag_bits`-based element decoder.** The deleted
  `tag_bits::HEAP_KIND_ARRAY` literal is forbidden by CLAUDE.md
  "Forbidden Patterns" #4 ("Runtime tag_bits dispatch"). The
  discriminator now lives on `HeapKind::TypedArray` (Route A) or on
  the new `HeapKind` variant (Route B), read from the heap header
  through the §2.7.6 / Q8 dispatch tables — never reconstructed from
  a bit pattern.

- **Mixed-route incrementalism.** Closing some array-FFI entries
  Route A and others Route B inside the same wave breaks
  `shape-jit --lib` symbol-resolution coherence: every consumer's
  call signature has to match the route the FFI symbol expects. The
  rebuild is single-route; the route is decided once before the
  wave opens.

**State at deferral (W12-jit-array audit, 2026-05-10):**

- `crates/shape-jit/src/jit_array.rs` — 71 lines, surface-and-stop
  module docs + 5 offset constants kept under `#[allow(dead_code)]`
  pending the route decision (Route A drops them; Route B re-derives
  them).
- `crates/shape-jit/src/ffi/array.rs` — 83 lines, all entry-point
  bodies removed; `ArrayInfo #[repr(C)]` carrier struct and
  `is_inline_bool` helper preserved (downstream symbol-table
  compatibility / non-array consumers).
- `crates/shape-jit/src/ffi_symbols/array_symbols.rs` — 41 lines,
  both `register_array_symbols` and `declare_array_functions`
  no-op'd.
- 19 production surface-and-stop sites named above carry the
  `"phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild"`
  marker.
- `cargo check -p shape-jit --lib` passes clean (215 warnings,
  zero errors).
- `shape-jit` test build is broken in unrelated ways (unresolved
  `shape_value::ValueWord` / `tag_bits` / `NativeKind::Unknown`
  imports across `differential_fuzz` / mir_compiler tests / etc.) —
  out of scope for Q15 deferral; the W10-cascade close gate was
  `--lib` only.

**Hot-path question answer (W12-jit-array charter):** the JIT
*can* compile-and-execute simple programs end-to-end on
`bulldozer-strictly-typed` for **scalar-typed paths** —
`executor.rs::execute_with_jit` decodes typed return tags
(`RETURN_TAG_F64` / `I64` / `I32` / `BOOL`) per §2.7.5.1 stable-FFI
boundary, and the JIT compile + execute pipeline works for any
program whose top-level frame returns a scalar `NativeKind`. Programs
that touch array operations (`Array<T>` construction, indexing,
`.push` / `.pop` / `.map` / `.filter` / etc.) hit one of the 19
surface-and-stop sites and panic with the `"JitArray rebuild"` marker
at JIT-FFI dispatch — not at JIT compile, not at VM-fallback
boundary. The JIT pipeline is alive; the array-FFI surface area is
the deferred work.

**Closure trigger:** Q15 closes when the route decision (Route A
vs. Route B) is made, the `jit_array.rs` layout lands, and a single
wave migrates all 19 surface-and-stop sites + the FFI symbol
registrations + the method-dispatch ABI threading in lockstep.
Estimated scope: 2–4 days for Route A (preferred), 4–7 days for
Route B (FFI registration multiplication and HeapKind cardinality
amendment add overhead). Either way it's an explicit Phase-2c wave,
not a maintenance follow-up; the W12-jit-array audit was the
boundary check.

**Status:** CLOSED. Phase 3 cluster-0 W11-jit-new-array (2026-05-12)
adopted Route A and shipped the unblock surface:

- `jit_array.rs`: 5 offset constants deleted (Route A uses
  `TypedArray<T>` directly: header @ 0, `data` @ 8, `len` @ 16,
  `cap` @ 20). Module retained as documentation anchor.
- `ffi/array.rs`: legacy entry-point bodies remain deleted;
  `ArrayInfo` `#[repr(C)]` carrier struct + `is_inline_bool`
  helper preserved per Q15 contract.
- `ffi_symbols/array_symbols.rs`: kept as no-op with Route-A-close
  documentation. The kinded allocator surface is the existing
  `register_v2_symbols`-registered `v2_array_new_<f64,i64,i32,bool>`
  family plus the size-dispatched `v2_array_push` helper.
- `FFIFuncRefs`: kind-blind `new_array` / `array_push_elem` slots
  deleted. Consumer MIR call sites (`Rvalue::Aggregate`,
  `StatementKind::ArrayStore` fallback, `StatementKind::EnumStore`
  payload, qualified-name enum-constructor call) surface-and-stop
  with a `Route A surface-and-stop:` marker and §-cite when the
  destination place's element kind isn't statically provable.
- `mir_compiler/v2_array.rs::try_emit_v2_array_method` covers
  `length` / `len` / `push` / `sum` / `min` / `max` / `mean` /
  `sumSquares` / `scale` / `addScalar` / `addArray` / `mulArray`
  inline against the `TypedArray<T>` layout. Cascade entries beyond
  that set (slice, reverse, zip, filled, range, info, first, last,
  pop) remain reachable via the generic `jit_call_method` trampoline
  path — per-method Cranelift codegen for them is a §2.7.14
  follow-up.

The §2.7.5 stamp-at-compile-time discipline was extended to two
collateral surfaces uncovered during the close: (a) `RETURN_TAG_I64`
is now stamped at the `TerminatorKind::Return` codegen from the
return slot's `NativeKind` (was always `RETURN_TAG_NANBOXED`); (b) a
new `RETURN_TAG_UNIT = 5` is stamped for `()`-typed returns
(top-level program ending in `print(x)`), mapped by the executor to
`WireValue::Null`. Per-kind `jit_print_<i64,f64,bool>` entries were
added with the MIR emitter dispatching by operand kind — the
kind-blind `jit_print` fallback is reserved for unproven kinds.

Smoke 1 (`let mut sum = 0; for i in 0..100 { sum += i }; print(sum)`)
produces `4950` under both `--mode vm` and `--mode jit` (exit 0).

**Reopen amendment (2026-05-12):** the first close (`b60d3678`) walked
back `jit_arc_retain` / `jit_arc_release` to a silent no-op, justified
in the close report as "the MIR caller side doesn't yet thread kind".
The walk-back hit CLAUDE.md "Forbidden rationalizations" (*"Soft-fail
counter for now, harden later."*) and was refused by the supervisor.

Root-cause from the reopen tracing: the MIR caller side
(`mir_compiler/ownership.rs`) DOES have the proven `NativeKind` for the
firing slots — `mir::types::infer_slot_kinds` proves `Int64` for the
`let mut x = 0` accumulator in the user-frame MIR — but the legacy
`is_native_slot` predicate (`types.rs:46-58`) excluded the
integer-family variants, treating any I64-wide slot as a candidate
heap pointer. The exclusion was correct under the deleted ValueWord
ABI (where I64 bits could be NaN-boxed pointer) and stale under
strict typing.

The reopen landed a principled refactor:

- New `shape_value::NativeKind::is_refcounted()` predicate — returns
  `true` iff the kind is `String` or `Ptr(HeapKind::*)` (the only two
  heap-pointer-carrying kinds). Every numeric / bool / nullable
  variant is `false`. This IS the §2.7.5 kind-discriminator answer
  to "does this slot need refcounting": kind IS the discriminator,
  no tag-bit probe.
- `mir_compiler/ownership.rs::refcount_disposition` centralizes the
  three-way decision (`Refcounted` / `Skip` / `Skip_TypedCellCarrier`)
  consulting `slot_kinds` + the existing typed-cell-carrier guards.
  Unproven kind falls back to `LocalTypeInfo` discrimination: `Copy`
  → skip, `Unknown` → safe-skip for unused-tail / implicit-return
  slots, `NonCopy` + unproven kind → surface-and-stop (the genuine
  §2.7.7 #9 kind-source gap; no W-series Bool-default).
- `Rvalue::Clone` in `rvalues.rs` shares the same disposition path
  via `refcount_disposition_for_place` instead of unconditionally
  retaining.
- `jit_arc_retain` / `jit_arc_release` bodies implement real
  retain/release against the `UnifiedValue<T>` `#[repr(C)]` layout
  (refcount `AtomicU32` at offset 4). The kind-dispatched reclaim on
  refcount-zero lives in `ffi/jit_release.rs::release_unified_value_by_kind`
  — reads the `kind: u16` field at offset 0 of `UnifiedValue` (the
  §2.7.6 / Q8 single-discriminator structural field, NOT a tag-bit
  probe) and dispatches per-kind `Box::from_raw::<UnifiedValue<T>>`
  arms. Unknown kinds surface-and-stop with intentional leak (no
  silent skip).
- Leak-balance verification: `SHAPE_JIT_ARC_COUNTERS=1` opt-in
  surface in `executor.rs` reports `retain_calls` /
  `release_calls` / `release_frees` deltas across the JIT-emitted
  code run. Smoke 1 measures `0/0/0` (no heap allocations at
  runtime — the principled outcome).

Remaining out-of-scope (now properly surfaced, not silently no-op'd):

- v2-map family (`jit_v2_map_*`) slots deleted from `FFIFuncRefs`
  with `try_emit_v2_typed_map_method` surface-and-stopped — these
  are W11-jit-carrier-conversion's territory.
- Top-level `concrete_types` side-table empty at
  `compiler/strategy.rs:205` — typed-array literals at top level
  (e.g. `let xs: Array<int> = [1,2,3]`) hit the `Rvalue::Aggregate`
  surface-and-stop instead of routing to the v2 fast path. This is
  a genuine §2.7.5 architectural gap (the bytecode compiler proves
  `Array<int>` but the side-table conduit from `BytecodeProgram` to
  the JIT MirToIR doesn't exist; the comment at strategy.rs:200-204
  flags it as "in flux upstream, other Phase 3.1 agents are
  refactoring it"). Surfaced for ADR-amendment / supervisor decision
  per CLAUDE.md handover §0 "Surface-and-stop discipline".
- Compile-time-boxed string constants (`box_string` in `MirConstant::Str`
  lowering) leak by design — pre-existing JIT pattern that pre-dates
  W11; flagged here for completeness.
- Per-HeapKind kinded print entries (`jit_print_str` / `jit_print_typed_object`
  / ...) — the kind-blind `jit_print` fallback still uses
  `format_value_word` (NaN-decode-via-tag-bits) for heap arms.
  §2.7.5 follow-up.

#### 2.7.15 `HeapKind::HashSet` — Q16 cardinality amendment (Wave 13 W13-hashset-rebuild, 2026-05-10)

Phase 1.B-vm Wave 13 W13-hashset-rebuild
(`docs/cluster-audits/wave-13-phase2c-playbook.md` Round 2) closes the
W9-set-methods Stage C surface left open by close commit `4c81e54`.
The W9 owner audit (file-level docstring in
`crates/shape-vm/src/executor/objects/set_methods.rs`) recorded:

> 1. `HeapKind` enumeration has no `Set` variant. The Phase-2 ValueWord
>    bulldozer removed the pre-existing `HeapValue::Set { items:
>    Vec<ValueWord> }` payload along with the rest of the heterogeneous-
>    element collections.
> 2. `BuiltinFunction::SetCtor` exists in the bytecode opcode table but
>    the executor body in `vm_impl/builtins.rs:491` is itself a `todo!()`.
>    Set values cannot reach a method handler from any execution path.
> 3. The Wave 9 playbook §1 recipe prescribes `args[0].slot
>    .as_heap_value()` receiver classification; the precondition for both
>    is a surviving `HeapValue::Set` arm.

The audit recommended **Path A — `Arc<HashSetData>` adjacent to Stage C
P1(b) HashMapData**: same insertion-ordered `TypedBuffer<Arc<String>>`
keys + bucket-index hash store, no values buffer. This amendment rules
that path in.

**Decision (Q16 ruling):** the Set carrier becomes a typed-
`Arc<HashSetData>`-backed `HeapValue` arm, mirroring the §2.3 typed-Arc
shape every other heap variant uses, and structurally a one-keyspace
specialization of the Stage C P1(b) `HeapValue::HashMap(Arc<HashMapData>)`
carrier (same `Arc<TypedBuffer<Arc<String>>>` keys, same eager FNV-1a
bucket index, no values buffer). The §2.7.7 / §2.7.8 / §2.7.9 / §2.7.10
/ §2.7.11 / §2.7.12 / §2.7.13 dispatch tables already established gain
one new arm per table — no new dispatch surface.

**The kinded carrier (`shape-value/src/heap_value.rs` +
`shape-value/src/heap_variants.rs`):**

```rust
// New HeapKind variant — next free ordinal (21, after SharedCell at
// ordinal 20 per §2.7.12) per §2.3 append-only ordering:
pub enum HeapKind {
    // ... String=0 .. HashMap=17 .. FilterExpr=18 .. Reference=19
    // .. SharedCell=20 ..
    HashSet,    // 21 (Wave 13 W13-hashset-rebuild, 2026-05-10)
}

// New HeapValue arm carrying typed Arc per §2.3 — full HeapValue arm
// (mirror of HashMap's §2.3 shape, NOT FilterExpr / SharedCell's
// pure-discriminator shape). Set values flow through the same opcodes
// as every other heap value (`LoadLocal` / `StoreLocal` / closure
// capture, store into `TypedObjectStorage` slots and
// `TypedArrayData::HeapValue` buffers); `slot.as_heap_value()` returns
// `HeapValue::HashSet(arc)` for downstream method dispatch.
pub enum HeapValue {
    // ... existing arms ...
    HashSet(std::sync::Arc<HashSetData>),
}

// New HashSetData struct — one-keyspace mirror of HashMapData. No
// values buffer; the `index` bucket map maps FNV-1a key hash to
// indices into the keys buffer, same as HashMapData's index.
pub struct HashSetData {
    pub keys: Arc<crate::typed_buffer::TypedBuffer<Arc<String>>>,
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}
```

**Why a full `HeapValue::HashSet` arm rather than pure-discriminator
(mirror of §2.7.9 FilterExpr / §2.7.12 SharedCell):** §2.7.9 /
§2.7.12's pure-discriminator shape is justified because their payloads
(`Arc<FilterNode>` and `Arc<SharedCell>` cell-pointer slots) are
emitted directly to the kinded stack as `Arc::into_raw(...) as u64`
and never wrapped in `HeapValue` — `as_heap_value()` is unsound on
those slot bits. Set values do not have that justification: Set values
flow through method dispatch (`set.add(...)`, `set.has(...)`,
`set.union(other)`), can be stored in `TypedObjectStorage` slots and
`TypedArrayData::HeapValue` buffers, and the W9 playbook §1 recipe
explicitly prescribes `args[0].slot.as_heap_value()` for receiver
classification. Pure-discriminator status would re-introduce the
§2.7.9 type-confusion pattern at a different layer. **Set is a HashMap
sibling, not a FilterExpr sibling.**

**Why Path A (`Arc<HashSetData>` mirror of HashMap) rather than Path B
(`TypedSet<T>` per element kind):** Path B was the W9 audit's second
coherent option — monomorphized `TypedSet<T>` per element kind, with a
hash-side index for O(1) `has`. Path A wins on three grounds:

1. **String-keyspace coverage.** The 12 W9-set-methods SURFACE'd
   handlers (`add`, `has`, `delete`, `size`, `is_empty`, `to_array`,
   `union`, `intersection`, `difference`, `for_each`, `map`, `filter`)
   plus the smoke target `let s = Set(); s.add("a"); s.add("b");
   print(s.size())` exercise only a string keyspace. Same as the
   Stage C P1(b) HashMap landing — string keys are the immediate
   need; heterogeneous-element kinds are the §2.7.4 follow-up.
2. **Lockstep cost.** Path A reuses the §2.7.4 `Arc<TypedBuffer<Arc<
   String>>>` storage shape verbatim (same keys buffer, same FNV-1a
   hashing, same bucket index); the dispatch-table lockstep is one
   new arm in each of 4 tables. Path B requires a new monomorphized
   variant per element kind across the same 4 dispatch tables —
   ~10× the lockstep surface.
3. **W13-hashmap-mutation precedent.** The mutation API
   (`insert(Arc<String>, _)`, `remove(&str)`, `merge(&other)`) was
   landed for HashMapData in commit `d8ec8c2` with `Arc::make_mut`
   clone-on-write over the inner `Arc<TypedBuffer<Arc<String>>>` plus
   parallel bucket-index rebuild. HashSetData mutation collapses the
   same precedent to a one-keyspace specialization
   (`insert(Arc<String>) -> bool`, `remove(&str) -> bool`,
   `union/intersection/difference` build fresh `HashSetData::from_keys`
   instances).

Path B is a **future optimization** — when measured allocation
pressure on string-keyed Sets exceeds the bucket-index miss cost
(or when non-string keysets land — int-keyed Set, TypedObject-keyed
Set), Path B's monomorphization becomes worth the lockstep cost. This
ruling does not foreclose Path B; it rules in Path A as the
W13-hashset-rebuild close shape.

**Mechanical lockstep updates (the new variant fans out to 4 dispatch
tables — every Q8/Q10 retain/release table — plus the
`HeapValue::kind()` / `is_truthy()` / `type_name()` mirrors and the
`KindedSlot` / `ValueSlot` constructors):**

1. `crates/shape-value/src/heap_variants.rs` — `HeapKind::HashSet`
   ordinal 21 + `HeapValue::HashSet(Arc<HashSetData>)` arm. Update
   `kind()` / `is_truthy()` / `type_name()`.
2. `crates/shape-value/src/heap_value.rs` —
   - `HashSetData` struct + `new()` / `from_keys()` / `len()` /
     `is_empty()` / `contains()` / `insert(Arc<String>) -> bool` /
     `remove(&str) -> bool` mutation API (mirror of HashMapData's
     W13-hashmap-mutation API with the values buffer dropped).
   - `TypedObjectStorage::drop` mirror — a TypedObject field of kind
     `NativeKind::Ptr(HeapKind::HashSet)` retires one
     `Arc<HashSetData>` strong-count share.
3. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` dispatch the new arm to
   `Arc::increment/decrement_strong_count::<HashSetData>`.
4. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror the same arm. Plus
   `KindedSlot::from_hashset(Arc<HashSetData>) -> Self` constructor
   (§2.7.6 / Q8 cardinality bound: one constructor per heap variant
   that needs a `from_*` entry — same as `from_hashmap` /
   `from_typed_object` / `from_typed_array`).
5. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
   §2.7.8 mirror. A `SharedCell` whose single-slot payload is
   `NativeKind::Ptr(HeapKind::HashSet)` retires one `Arc<HashSetData>`
   strong-count share at cell drop.
6. `crates/shape-value/src/slot.rs` — `ValueSlot::from_hashset(
   Arc<HashSetData>)` constructor (mirror of `from_hashmap`).
7. `crates/shape-vm/src/executor/objects/set_methods.rs` — migrate the
   12 SURFACE'd handlers (`v2_add`, `v2_has`, `v2_delete`, `v2_size`,
   `v2_is_empty`, `v2_to_array`, `v2_union`, `v2_intersection`,
   `v2_difference`, `v2_for_each`, `v2_map`, `v2_filter`) from
   `NotImplemented(SURFACE)` to real bodies. Read-only handlers borrow
   the receiver `Arc<HashSetData>` via the §2.7.6 / Q8 `as_heap_value()`
   + match path; mutation handlers `Arc::make_mut` over a cloned
   receiver share for clone-on-write per §2.7.4 (mirror of the
   W13-hashmap-mutation `v2_set` / `v2_delete` / `v2_merge` shape);
   set-operation handlers (`union` / `intersection` / `difference`)
   build a fresh `HashSetData` via `from_keys`; closure-callback
   handlers (`for_each` / `map` / `filter`) route per-element callbacks
   through `vm.call_value_immediate_nb` per §2.7.11 / Q12.
8. `crates/shape-vm/src/executor/vm_impl/builtins.rs` —
   `BuiltinFunction::SetCtor` body builds an empty
   `Arc::new(HashSetData::new())` and pushes a
   `KindedSlot::from_hashset(...)` (mirror of the still-pending
   `HashMapCtor` shape; the cross-link ctor pair is part of the
   wave-13 playbook Round 2).

There is no fan-out to `printing.rs` / `comparison/mod.rs` /
`arithmetic/mod.rs` / `wire_conversion.rs` / `json_value.rs` *in this
amendment*; those surfaces preserve the §2.7.5 stable-FFI rejection
arms by default and will gain `HashSet`-specific entries via the
exhaustive-match compiler errors when the dispatch shell wires through
to the SET_METHODS PHF map — same incremental shape as HashMap's
landing.

**Cardinality cost:** `HeapKind` grows from 20 variants to 21; the
§2.7.6 / Q8 carrier-API bound gains one new constructor
(`KindedSlot::from_hashset`) — total constructor count remains within
the cardinality bound (~25 max) per §2.7.6. No new accessor (heap
dispatch goes through `slot.as_heap_value()` per ADR-005 §1
single-discriminator).

**Forbidden shapes this rules out (mirror of §2.7.9 / §2.7.12 /
§2.7.13 forbidden lists, applied to the Set carrier):**

- **Resurrecting the deleted `HeapValue::Set { items: Vec<ValueWord> }`
  arm under another name.** The pre-bulldozer shape carried
  heterogeneous elements via `ValueWord`-encoded buckets and tag-bit
  hash dispatch (`vw_hash` / `vw_equals`). Both `ValueWord` and the
  heterogeneous-element bucket dispatch are deleted (CLAUDE.md
  "Forbidden code" #1, #4). No `Vec<ValueWord>` revival, no
  rename to `Vec<ValueBits>` / `Vec<RawSlot>` / `Vec<TaggedItem>`.
  The W13-hashset-rebuild keyspace is explicitly string-only at
  landing; non-string keysets are a separate ADR amendment with
  measurement.
- **Reusing an existing `HeapKind` variant as a stand-in label for
  `*const HashSetData`.** Same wrong-type retain/release UB as the
  pre-§2.7.9 FilterExpr/NativeView mislabel. Refused: dispatch
  tables match payloads 1:1.
- **Bool-default fallback for the unknown receiver kind at
  `set_methods.rs::v2_*`.** The receiver kind is sourced from the
  §2.7.7 stack parallel-kind track; a kind-mismatch surfaces as a
  `RuntimeError` per playbook §6, not a silent leak.
- **Pure-discriminator `HeapKind::HashSet` (mirror of §2.7.9
  FilterExpr).** Refused per the analysis above — Set values flow
  through `slot.as_heap_value()` and need a real `HeapValue::HashSet`
  arm. Pure-discriminator status would re-introduce the §2.7.9
  type-confusion pattern.
- **Heterogeneous-element re-introduction by reusing
  `Arc<TypedBuffer<Arc<HeapValue>>>` for keys.** Future work; not in
  this amendment. Path B (`TypedSet<T>` per element kind) is the
  monomorphized rebuild path when a non-string keyspace is
  surfaced; out-of-scope here.
- **Transitional shims preserving deleted Set-shape names**
  (`SetLegacy` / `nanboxed::Set` / `as_set_items` /
  `set_bucket_decode` / `vw_set_hash` / `vw_set_equals`). Same §2.7.7
  #1 rule — the W-series "borrowed bits with call-pattern invariants"
  defection-attractor at the Set-carrier layer. Migrate every
  `set_methods.rs` handler in-wave; do not preserve a kind-blind
  variant as a transitional layer.
- **Defection-attractor descriptors** — "set bridge", "set-element
  translator", "bucket probe", "key-decode helper", "set-projection
  hop", "boundary adapter for set carrier". Per the 2026-05-09 user
  ruling broadening the W-series rename family + playbook §3
  forbidden #20, any descriptor of the deleted Set carrier that uses
  bridge / probe / helper / hop / translator / adapter framing
  belongs to the same defection-attractor family CLAUDE.md "Renames
  to refuse on sight" enumerates. Describe the deleted carrier by
  name (the pre-bulldozer `HeapValue::Set { items: Vec<ValueWord> }`)
  or by deletion-fate (the heterogeneous-element ValueWord-shape Set
  carrier), never by hypothetical role.

**Performance characteristics:**

- `Set()` ctor cost: one `Arc::new(HashSetData::new())` allocation +
  empty `TypedBuffer<Arc<String>>` inner + empty `HashMap<u64,
  Vec<u32>>`. Same as `HashMap()` with the values buffer dropped.
- `set.add(key)` cost: one `Arc::clone` of the receiver, then
  `Arc::make_mut` clone-on-write on the keys buffer (cheap on
  unique receiver, full clone on shared) + bucket-index entry.
  Hash is FNV-1a over the key bytes; collision bucket scan is O(k)
  where k is the bucket size (typically 1–2 for non-pathological
  inputs). Identical cost to `HashMap.set(key, _)` minus the values
  buffer mutation.
- `set.has(key)` cost: one FNV-1a hash + bucket index lookup + O(k)
  collision scan. O(1) amortized — same as `HashMap.has(key)`.
- `set.union(other)` / `set.intersection(other)` / `set.difference(
  other)` cost: O(n+m) build of a fresh `HashSetData` via `from_keys`
  (one allocation pair for the new keys buffer + bucket index).
  No Arc::make_mut on the receivers; both inputs are borrowed
  read-only.
- IC fast path: Sets flow through the standard `LoadLocal` /
  `StoreLocal` paths via their `NativeKind::Ptr(HeapKind::HashSet)`
  kind label. No special IC entry; same as HashMap.

**Out-of-scope this amendment:**

- Heterogeneous-element keysets (int-keyed, TypedObject-keyed,
  Char-keyed). String-only keyspace at landing, per the W9 audit's
  Path A scope and the W13-hashmap-mutation precedent.
- The `iter()` method (`set.iter()` returning a stateful iterator).
  Same §2.7.4 deferral as `HashMap.iter()` — the kinded
  Set-iteration shape (per-element key kind dispatch over the typed
  buffer) is a phase-2c follow-up workstream tracked under
  W13-iterator-state.
- JIT FFI for Set methods. The JIT array FFI rebuild (§2.7.14 / Q15)
  has the same deferral shape and Set inherits it; SetMethod calls
  will deopt to the interpreter until the Q15 ruling lands. No
  separate Q-ruling needed for Set since it follows the same FFI
  rebuild path as HashMap and TypedArray.
- Wire-format serialization of Set-bearing values. `wire_conversion.rs`
  / `json_value.rs` will gain a `HashSet` arm via exhaustive-match
  follow-up; the §2.7.5.1 stable-FFI boundary rule applies (HashSet
  serializes as a JSON array of strings, mirror of HashMap's
  serialization shape).

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

#### 2.7.16 Lazy iterator carrier — `HeapValue::Iterator(Arc<IteratorState>)` (Q17 ruling)

W13-iterator-state (close 2026-05-10) lands the kinded lazy-iterator
pipeline carrier, replacing the deleted `heap_value::IteratorState` /
`IteratorTransform` `ValueWord`-shaped enums (Phase-1.A bulldozed).
Pre-strict-typing the lazy-iterator pipeline pushed
`ValueWord::from_heap_value(HeapValue::Iterator { state, transforms })`
slots and dispatched per-element via `tag_bits` on the per-stage closure
payloads — both forbidden post-§2.7.7 / §2.7.8 (CLAUDE.md "Forbidden
code" #1, #4, #6). The pre-W13-iterator-state scope was the
`ITERATOR_METHODS` PHF in `executor/objects/method_registry.rs` (14
entries) plus the four receiver-bound `iter()` factories
(`Array.iter` / `String.iter` / `HashMap.iter` / `Range.iter`) — 18
distinct handler bodies surfacing as `NotImplemented(SURFACE: §2.7.4)`.

**Decision (Q17 ruling):** the lazy-iterator pipeline rebuilds on a
typed `Arc<IteratorState>` payload carried by a fresh `HeapKind` arm,
mirroring the §2.7.13 `Reference` precedent (typed-Arc dispatch with a
parallel `HeapValue` arm for ADR-005 §1 single-discriminator
recovery).

**The kinded carrier (`shape-value/src/iterator_state.rs`):**

```rust
pub struct IteratorState {
    pub source: IteratorSource,
    pub transforms: Vec<IteratorTransform>,
    pub cursor: usize,
}

pub enum IteratorSource {
    Array(Arc<TypedArrayData>),
    String(Arc<String>),
    Range { start: i64, end: i64, step: i64 },
    HashMap(Arc<HashMapData>),
}

pub enum IteratorTransform {
    Map(Arc<HeapValue>),       // closure carrier per §2.7.11/Q12
    Filter(Arc<HeapValue>),    // closure carrier per §2.7.11/Q12
    Take(usize),
    Skip(usize),
    FlatMap(Arc<HeapValue>),   // closure carrier per §2.7.11/Q12
    Enumerate,
    Chain(Arc<IteratorState>),
}
```

**The new `HeapKind` arm:**

```rust
pub enum HeapKind {
    // ... String=0 .. SharedCell=20 ..
    Iterator,    // 21  (W13-iterator-state, 2026-05-10)
}

pub enum HeapValue {
    // ... existing arms ...
    Iterator(Arc<IteratorState>),
}
```

Slot bits for an `Iterator`-kinded slot are
`Arc::into_raw(Arc<IteratorState>) as u64` directly. Unlike the §2.7.9
FilterExpr / §2.7.13 Reference precedents — where the parallel
`HeapValue` arm exists only for `HeapKind`↔`HeapValue` symmetry and
`as_heap_value()` is undefined behavior on those slot bits —
**`as_heap_value()` IS valid on Iterator-labeled bits**: the iterator
method handlers recover the typed `Arc<IteratorState>` via the
canonical `slot.as_heap_value()` → `HeapValue::Iterator(arc)` match,
preserving ADR-005 §1 single-discriminator. The shape is the same as
existing typed-Arc heap variants (`HeapValue::TypedArray`,
`HeapValue::HashMap`, etc.) — typed `Arc<T>` payload, dispatch goes
through both the kind label (for refcount discipline at the §2.7.7 /
§2.7.8 dispatch tables) and through `HeapValue` (for handler-body
recovery).

**Why kind is on the carrier rather than fabricated at terminal time:**
§2.7.7 forbidden-shape #4 (tag-bit chains) and §2.7.7 / §2.7.8 / §2.7.9
/ §2.7.10 / §2.7.11 invariant — the kind comes from the producing-
opcode emit, never fabricated downstream. Iterator factories
(`Array.iter` / `String.iter` / `HashMap.iter` / `Range.iter`) are the
producing sites; they construct `IteratorState` from a typed receiver
`Arc<T>` and label the resulting slot with
`NativeKind::Ptr(HeapKind::Iterator)` directly. Lazy transforms append
new stages without touching the kind label. Eager terminals walk the
state and dispatch per-stage on the `IteratorTransform` arm — no
runtime kind decode, no `is_heap()` probe.

**Mechanical lockstep updates (4 dispatch tables — every Q8/Q10
retain/release table — plus the knock-on exhaustive matches):**

1. `crates/shape-value/src/heap_variants.rs` — `HeapKind::Iterator`
   ordinal 21 + `HeapValue::Iterator(Arc<IteratorState>)` arm + `kind()`
   / `is_truthy()` / `type_name()` / `Clone` / `Display` updates.
2. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` dispatch the new arm to
   `Arc::increment/decrement_strong_count::<IteratorState>`.
3. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror the same arm; `from_iterator(Arc<IteratorState>)`
   constructor.
4. `crates/shape-value/src/heap_value.rs` — `TypedObjectStorage::drop`
   §2.7.8 mirror.
5. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
   §2.7.8 mirror.
6. `crates/shape-vm/src/executor/objects/iterator_methods.rs` — 18
   handler bodies migrated from `NotImplemented(SURFACE)` to live
   bodies. Source-side factories construct `IteratorState`; lazy
   transforms append via `IteratorState::with_transform`; eager
   terminals walk via the `iterate_to_vec` driver (Vec-collect-then-
   terminate pattern — closure invocations during Map/Filter/FlatMap
   stages happen during the walk, but the terminator's per-element
   side effects (forEach / reduce / any / all / find) iterate the
   collected `Vec<KindedSlot>` after the walk, sidestepping the
   `&mut VirtualMachine` ↔ closure-capture borrow conflict).

Plus knock-on exhaustive-match additions in `printing.rs`,
`comparison/mod.rs`, `arithmetic/mod.rs`, `objects/typed_access.rs`
(kind→type-name maps); `wire_conversion.rs`, `json_value.rs` (Iterator
does not cross the wire boundary — same as FilterExpr / Reference).

**Cardinality cost:** `HeapKind` grows from 20 variants to 21; the
§2.7.6 Q8 bound (~25 constructors / ~5-10 scalar accessors max on
`KindedSlot`) is unchanged — `from_iterator` is the matching constructor
addition.

**Range.iter remains a SURFACE.** The kinded Range receiver carrier
itself is phase-2c (`HeapValue::Range` was deleted; `op_make_range`
surfaces in `executor/objects/mod.rs`). The W13-iterator-state carrier
provides `IteratorSource::Range { start, end, step }` for forward
compatibility, but the `Range.iter` factory cannot construct one
without a live Range receiver kind labeling Range slots. Re-entry once
the upstream Range carrier lands.

**Forbidden alternatives this rules out:**

- "Re-introduce `IteratorState` in the deleted `ValueWord`-encoded
  shape under a less-suspicious name." This is the W-series
  defection-attractor (CLAUDE.md "Renames to refuse on sight"); the
  kind dispatch must go through the `HeapKind::Iterator` arm and the
  payload must be a typed `Arc<T>`, never a `Box<HeapValue>` wrapper
  or a tag-bit-decoded carrier.
- "Store closure transforms as raw `u64` bits in
  `IteratorTransform::Map(u64)` to avoid the extra `Arc<HeapValue>`
  bump." Forbidden by §2.7.11/Q12: closure carriers cross the
  abstraction boundary as `Arc<HeapValue>` shares (the `HeapKind::Closure`
  retain/release dispatch matches that share). Storing raw bits would
  bypass the share-counting discipline at the transform-stash tier.
- "Skip the `HeapValue::Iterator` arm and use a pure-discriminator
  `HeapKind::Iterator` like FilterExpr / SharedCell." `HeapValue::Iterator`
  is required because handler bodies recover the typed
  `Arc<IteratorState>` via `slot.as_heap_value()` (per ADR-005 §1
  single-discriminator) — there is no second recovery path. The
  pure-discriminator pattern (FilterExpr / SharedCell) is reserved for
  variants whose payloads never flow through `HeapValue` materialization;
  iterator handlers do, so the parallel arm is load-bearing here.

**Out-of-scope this amendment:** snapshot/wire serialization of
in-flight iterator state. Iterators are within-program lazy values and
must be materialized via a terminal (`collect` / `forEach` / etc.)
before any cross-process boundary. The wire/JSON arms reject Iterator
slots in the same shape as FilterExpr / Reference.

#### 2.7.19 `HeapKind::Deque` — Q20 cardinality amendment (Wave 15 W15-deque, 2026-05-10)

W15-deque (close 2026-05-10) lands the kinded double-ended-queue
carrier, replacing the deleted `HeapValue::Deque` /
`heap_value::DequeData` `ValueWord`-shaped enums (Phase-1.A
bulldozed). Pre-strict-typing the deque used a
`Vec<ValueWord>`-backed payload and dispatched per-element via the
deleted `tag_bits` machinery — both forbidden post-§2.7.7 / §2.7.8
(CLAUDE.md "Forbidden code" #1, #4, #6). The pre-W15-deque scope was
the `DEQUE_METHODS` PHF in `executor/objects/method_registry.rs` (10
entries) plus the `Deque()` ctor in `vm_impl/builtins.rs` — all
surfacing as `NotImplemented(SURFACE: §2.7.4)`.

**Decision (Q20 ruling):** the deque rebuilds on a typed
`Arc<DequeData>` payload carried by a fresh `HeapKind` arm, mirroring
the §2.7.15 HashSet precedent (full `HeapValue::Deque` arm, NOT
pure-discriminator like FilterExpr / SharedCell — Deque values flow
through `slot.as_heap_value()` for receiver classification at method
dispatch).
#### 2.7.18 `HeapKind::PriorityQueue` — Q19 cardinality amendment (Wave 15 W15-priority-queue, 2026-05-10)

W15-priority-queue (close 2026-05-10) lands the kinded i64-priority
min-heap carrier, replacing the deleted pre-bulldozer
`HeapValue::PriorityQueue { state, ... }` `ValueWord`-shaped struct
variant (Phase-1.A bulldozed alongside the `HashMap` / `Set` / `Deque`
heterogeneous-element collection family). Pre-strict-typing the
PriorityQueue pushed `ValueWord::from_priority_queue(...)` slots with
heterogeneous per-element `ValueWord` payloads and dispatched per-method
via `tag_bits` on the receiver — both forbidden post-§2.7.7 / §2.7.8
(CLAUDE.md "Forbidden code" #1, #4, #6). The pre-W15-priority-queue
scope was the `PRIORITY_QUEUE_METHODS` PHF in
`executor/objects/method_registry.rs` (7 entries — `push` / `pop` /
`peek` / `size` / `isEmpty` / `toArray` / `toSortedArray`) plus the
`BuiltinFunction::PriorityQueueCtor` ctor body in
`vm_impl/builtins.rs` — 7 distinct method handler bodies plus 1 ctor
body surfacing as `NotImplemented(SURFACE: §2.7.4)`.

**Decision (Q19 ruling):** the priority-queue carrier rebuilds on a
typed `Arc<PriorityQueueData>` payload carried by a fresh `HeapKind`
arm, mirroring the §2.7.15 HashSet precedent (typed-Arc dispatch with
a parallel `HeapValue` arm for ADR-005 §1 single-discriminator
recovery — full HeapValue arm, NOT pure-discriminator like FilterExpr
/ SharedCell). Storage at landing is a binary min-heap over a single
`Arc<TypedBuffer<i64>>` priorities buffer — i.e. **i64-priority-only**.
The audit-time alternative — typed-payload `PriorityQueue<T, K>` with
key-extractor (`Arc<HeapValue>` payload + `Arc<HeapValue>` comparator
closure per §2.7.11/Q12) — is rejected at landing for the same
cardinality-cost reason §2.7.15 / Q16 rejected `TypedSet<T>` per
element kind, plus an additional reason: the smoke target
(`pq.push(3); pq.push(1); pq.push(2); pq.pop() == 1`) is exercised
end-to-end on the simpler i64-only shape, and the typed-payload
rebuild is a follow-up Phase-2c amendment with measurement (the same
"future Phase-2c amendment" boundary the §2.7.15 ruling drew for
non-string keysets). The Wave 15 playbook explicitly called out
i64-priority-only as a valid audit choice — this ruling formalises it.

**The kinded carrier (`shape-value/src/heap_value.rs`):**

```rust
pub struct DequeData {
    /// Insertion-ordered double-ended queue of heap-allocated element
    /// payloads. Element kinds are recovered via the canonical
    /// ADR-005 §1 single-discriminator `HeapValue` match at the read
    /// site.
    pub items: std::collections::VecDeque<Arc<HeapValue>>,
}
```

pub struct PriorityQueueData {
    /// Heap-ordered i64 priorities. Index 0 is the current min.
    /// Backed by `Arc<TypedBuffer<i64>>` so a HeapValue clone is a
    /// single atomic refcount bump and `Arc::make_mut` is the
    /// canonical clone-on-write entry.
    pub heap: Arc<TypedBuffer<i64>>,
}
```

Operations: `push(value)` does sift-up after `Vec::push`; `pop()` does
sift-down after the root↔last swap + `Vec::pop`; `peek()` reads index
0 without mutation; `to_vec()` / `to_sorted_vec()` project the heap
contents (heap-array order vs ascending sort) for the `toArray` /
`toSortedArray` methods. Mutation goes through `Arc::make_mut` per the
W13-hashmap-mutation precedent (commit `d8ec8c2`) — clone-on-write
when the receiver `Arc<PriorityQueueData>` has multiple shares;
single-share fast-path otherwise.

**The new `HeapKind` arm:**
#### 2.7.17 Variant carriers — `HeapValue::Result(Arc<ResultData>)` / `HeapValue::Option(Arc<OptionData>)` (Q18 ruling)

W14-variant-codegen (close 2026-05-10) lands the kinded Result/Option
carriers, replacing the deleted pre-bulldozer `Some/Ok/Err`
`ValueWord`-shaped HeapValue arms (Phase-1.A bulldozed). Pre-strict-
typing the variant ctors pushed `ValueWord::from_heap_value(HeapValue::
Some(ValueWord_inner))` etc. and dispatched the variant discriminators
(`op_is_ok`, `op_is_err`, `op_unwrap_ok`, `op_unwrap_err`,
`op_unwrap_option`, `op_try_unwrap`, `op_error_context`,
`op_type_check`) via `tag_bits` on the wrapper bits — both forbidden
post-§2.7.7 (CLAUDE.md "Forbidden code" #1, #4, #6). The
pre-W14-variant-codegen scope was the 8 op_* discriminator handlers in
`executor/exceptions/mod.rs` plus the 3 ctor bodies in
`executor/vm_impl/builtins.rs` (`SomeCtor`, `OkCtor`, `ErrCtor`) — all
surfacing as `NotImplemented(SURFACE: §2.7.4)` at the
W13-result-option-ops audit (close `61d0f49`).

**Decision (Q18 ruling):** Result and Option rebuild on typed
`Arc<ResultData>` / `Arc<OptionData>` payloads carried by fresh
`HeapKind` arms — full HeapValue arms (NOT pure-discriminator like
§2.7.9 FilterExpr / §2.7.12 SharedCell), mirroring the §2.7.16
Iterator typed-Arc dispatch with both kind-label refcount discipline
and `slot.as_heap_value()` recovery for handler bodies. The decision
rejects two alternatives:

- **Path B — single `__Result` / `__Option` schema-keyed
  `Arc<TypedObjectStorage>`:** would re-use the AnyError TypedObject
  precedent (§2.5 / W13-anyerror) with discriminator and payload
  fields. Rejected because the payload field would need a Bool-default
  fallback for non-heap inner values (the schema's per-slot
  `field_kinds` table can't represent "kind varies per instance"
  without a parallel kind track adjacent to the storage); duplicating
  the §2.7.7 stack parallel-kind contract at the schema tier defeats
  the carrier-API-bound (§2.7.6 / Q8). Result/Option payloads are
  inherently kind-polymorphic (the inner T is whatever the producing
  expression emitted), so a typed-Arc carrier with an embedded
  `KindedSlot` payload is the natural fit.

- **Path C — null-coding Option only (`Some(x) ≡ x`, `None ≡ null
  sentinel`):** matches the legacy compiler emit (`op_is_null` test
  in `compiler/patterns/checking.rs:213`), is zero-overhead for the
  common case, and reuses the existing null-sentinel discipline.
  Rejected as the canonical representation (but PRESERVED as a
  fallback for compiler emit sites that haven't migrated) because
  `Some(x)` where `T = T?` (a doubly-nullable type) cannot
  distinguish `Some(None)` from `None` under null-coding. The kinded
  `Arc<OptionData>` carrier with explicit `is_some` discriminator
  resolves this without losing soundness.

**The kinded carriers (`shape-value/src/heap_value.rs`):**

```rust
pub struct ResultData {
    pub is_ok: bool,
    pub payload: KindedSlot,    // owns one share for the inner value
}

pub struct OptionData {
    pub is_some: bool,
    pub payload: KindedSlot,    // None: KindedSlot::none() placeholder
}
```

Both structs implement `Clone` via per-field clone — `KindedSlot::Clone`
bumps the inner share per Q8. Drop is auto-derived from the embedded
`KindedSlot`'s explicit Drop impl (kind-dispatched refcount retire per
§2.7.6 / Q8). The recursion-through-Arc discipline is the same as
§2.7.16 `IteratorTransform::Map(Arc<HeapValue>)` (the iterator
state's stash of a closure carrier).

**The new `HeapKind` arms:**

```rust
pub enum HeapKind {
    // ... String=0 .. Iterator=22 ..
    Deque,    // 26 (Wave 15 W15-deque, 2026-05-10) — ordinal-bumped to
              // 23 at branch landing (W14 / W15-priority-queue not yet
              // merged; merge-time playbook §4 ordering restores 26)
    // ... String=0 .. SharedCell=20, HashSet=21, Iterator=22 ..
    PriorityQueue,    // 25  (W15-priority-queue, 2026-05-10)
    Result,    // 23  (Wave 14 W14-variant-codegen, 2026-05-10)
    Option,    // 24  (Wave 14 W14-variant-codegen, 2026-05-10)
}

pub enum HeapValue {
    // ... existing arms ...
    Deque(Arc<DequeData>),
}
```

Slot bits for a `Deque`-kinded slot are
`Arc::into_raw(Arc<DequeData>) as u64` directly. **Full HeapValue
arm** (NOT pure-discriminator like §2.7.9 FilterExpr / §2.7.13
Reference / §2.7.12 SharedCell): handler bodies recover the typed
`Arc<DequeData>` via `slot.as_heap_value()` →
`HeapValue::Deque(arc)`, preserving ADR-005 §1
single-discriminator. Same shape as §2.7.15 HashSet and §2.7.16
Iterator — typed `Arc<T>` payload, dispatch goes through both the
kind label (refcount discipline at the §2.7.7 / §2.7.8 dispatch
tables) and through `HeapValue` (handler-body recovery).

**Why `VecDeque<Arc<HeapValue>>` rather than the §2.7.7
parallel-kind track shape:**

- **Heterogeneous-element semantics.** Deque is element-kind-agnostic
  at landing, mirroring `HashMapData::values`. Storing
  `Arc<HeapValue>` per element collapses the (bits, kind) pair into a
  single payload at the element tier, matching the Stage C P1(b)
  HashMap precedent verbatim.
- **§2.7.7 parallel-kind (`Vec<u64>` + `Vec<NativeKind>`) is for the
  STACK ABI** — its hot-path role is opcode dispatch where every
  push/pop is on the call site. A heap-resident deque is a
  GENERIC_CARRIER (§2.7.1) where each element pushed/popped already
  pays the construction cost; threading kinds through the deque API
  forces every caller to source kinds from outside the data
  structure.
- **No HashSet-style dedup needed.** Deque is order-preserving
  without deduplication, so the bucket-index that distinguishes
  HashSet from a plain typed buffer doesn't apply.

Path B alternatives considered and rejected at landing:

- **`TypedDeque<T>` per element kind** — same cardinality cost as the
  §2.7.15 Path B rejection: 12+ monomorphized variants × the §2.7.7
  / §2.7.8 / §2.7.10 dispatch tables each. Future amendment with
  measurement.
- **`Vec<u64>` + parallel `Vec<NativeKind>` element track** —
  rejected per the bullet above. Reserved for the case where
  measurement shows `Arc<HeapValue>` per element is the bottleneck on
  scalar-heavy workloads.
    Result(Arc<ResultData>),
    Option(Arc<OptionData>),
}
```

Slot bits for a `Result`-kinded slot are
`Arc::into_raw(Arc<ResultData>) as u64`; Option mirrors. Like §2.7.16
Iterator (and unlike §2.7.9 FilterExpr / §2.7.13 Reference),
**`as_heap_value()` IS valid** on Result/Option-labeled bits: the
discriminator handlers (`op_is_ok` / `op_is_err` / `op_unwrap_ok` /
`op_unwrap_err` / `op_unwrap_option` / `op_try_unwrap` /
`op_error_context`) recover the typed Arc via
`slot.as_heap_value()` → `HeapValue::Result(arc)` /
`HeapValue::Option(arc)` per ADR-005 §1 single-discriminator. The
handler-side helpers (`read_result` / `read_option`) implement this
classifier, mirroring `iterator_methods.rs::as_iterator`.

**Why kind is on the carrier rather than fabricated downstream:**
§2.7.7 forbidden-shape #4 (tag-bit chains) and §2.7.7 / §2.7.8 / §2.7.9
/ §2.7.10 / §2.7.11 invariant — kind comes from the producing-opcode
emit, never fabricated downstream. The variant ctors
(`SomeCtor` / `OkCtor` / `ErrCtor`) are the producing sites; they pop
the input arg's `KindedSlot` carrier, transfer the share into a fresh
`Arc<ResultData>` / `Arc<OptionData>`, and label the resulting slot
with `NativeKind::Ptr(HeapKind::Result)` / `NativeKind::Ptr(HeapKind::
Option)` directly. The discriminator opcodes consume those labels as
their classification input — no runtime kind decode, no `is_heap()`
probe.

**The 8 op_* handler bodies (`executor/exceptions/mod.rs`):**

| op | Behavior |
|---|---|
| `op_is_ok` | classify Result, push Bool(`is_ok`) |
| `op_is_err` | classify Result, push Bool(`!is_ok`) |
| `op_unwrap_ok` | classify Result, retain payload via KindedSlot::Clone, push (Err: throw RuntimeError) |
| `op_unwrap_err` | classify Result, retain payload, push (Ok: throw) |
| `op_unwrap_option` | classify Option (or null-coded), retain payload, push (None/null: throw) |
| `op_try_unwrap` | Ok(v)/Some(v) → push v; Err(e)/None → early-return wrapper to caller via `return_value_inner`; bare non-null → pass-through |
| `op_error_context` | Ok/Some(v) → push v; Err/None → `build_any_error(payload, cause=context, ..)` + `handle_exception` |
| `op_type_check` | match TypeAnnotation against carrier kind (Basic scalars + Generic Result/Option/Array/Map/Set/Iterator); other forms conservatively false |

The retain-on-extract pattern (`KindedSlot::Clone` on the inner
payload, then `KindedSlot::Drop` on the outer wrapper) is the WB2.4
discipline per §2.7.7 — pin-tested by the
`unwrap_refcount_regression_tests` block in the same file (preserved
as `#[cfg(feature = "phase-2c-exception-rebuild")]` for the deeper
match-binding integration tests; the storage-tier round-trip is
covered by the §2.7.17 unit tests in `heap_value.rs`).

**Mechanical lockstep updates (4 dispatch tables — every Q8/Q10
retain/release table — plus the knock-on exhaustive matches):**

1. `crates/shape-value/src/heap_variants.rs` — `HeapKind::Deque`
   ordinal 23 + `HeapValue::Deque(Arc<DequeData>)` arm + `kind()` /
   `is_truthy()` / `type_name()` updates. Display impl renders
   `Deque[elem1, elem2, ...]`.
2. `crates/shape-value/src/heap_value.rs` — `DequeData` struct +
   `new()` / `from_items()` / `len()` / `is_empty()` / `peek_front()`
   / `peek_back()` / `get()` / `push_back()` / `push_front()` /
   `pop_back()` / `pop_front()` API. Plus the Clone arm in
   `HeapValue::clone()` (single Arc bump; no payload copy) and the
   `TypedObjectStorage::Drop` §2.7.8 mirror — a TypedObject field of
   kind `NativeKind::Ptr(HeapKind::Deque)` retires one
   `Arc<DequeData>` strong-count share.
3. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` dispatch the new arm to
   `Arc::increment/decrement_strong_count::<DequeData>`.
4. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror the same arm; new
   `KindedSlot::from_deque(Arc<DequeData>)` constructor.
5. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
   §2.7.8 mirror.
6. `crates/shape-value/src/slot.rs` — `ValueSlot::from_deque(Arc<
   DequeData>)` constructor (mirror of `from_hashset`).
7. `crates/shape-vm/src/executor/objects/deque_methods.rs` — all 10
   handlers (`pushBack` / `pushFront` / `popBack` / `popFront` /
   `peekBack` / `peekFront` / `size` / `isEmpty` / `toArray` /
   `get`) flipped from `NotImplemented(SURFACE)` to live bodies on
   top of `DequeData`. Receiver projects via `as_deque` (kind gate
   + `as_heap_value()` Q8 single-discriminator path); element
   conversion via `arg_slot_to_heap_value_arc` /
   `heap_value_arc_to_slot` mirrors the §2.7.15 hashmap-mutation
   pattern.
8. `crates/shape-vm/src/executor/vm_impl/builtins.rs` —
   `BuiltinFunction::DequeCtor` body builds an empty
   `Arc::new(DequeData::new())` and pushes a
   `KindedSlot::from_deque` slot.

Plus knock-on exhaustive-match additions in `printing.rs`
(`format_deque` helper rendering `Deque[...]` front-to-back),
`comparison/mod.rs`, `arithmetic/mod.rs`,
`objects/typed_access.rs` (kind→type-name maps);
`shape-runtime/json_value.rs` (Deque serializes as a JSON array of
elements, dispatching per element through the canonical
`heap_to_json_value` recursion); `shape-runtime/wire_conversion.rs`
(opaque `<deque:phase-2c>` tag — same phase-2c marshal-rebuild
deferral as HashMap / HashSet).

**Cardinality cost:** `HeapKind` grows from 22 variants to 23 (or to
26 post-merge once W14 / PriorityQueue land per playbook §4); the
§2.7.6 Q8 bound (~25 constructors / ~5-10 scalar accessors max on
`KindedSlot`) is unchanged — `from_deque` is the matching constructor
addition.

**Forbidden alternatives this rules out:**

- "Re-introduce `DequeData` in the deleted `Vec<ValueWord>`-encoded
  shape under a less-suspicious name." This is the W-series
  defection-attractor (CLAUDE.md "Renames to refuse on sight"); the
  kind dispatch must go through the `HeapKind::Deque` arm and the
  payload must be a typed `Arc<T>`, never a `Vec<ValueWord>` or a
  tag-bit-decoded carrier.
- "Off-label re-use of an existing HeapKind variant
  (`HeapKind::HashSet` / `HeapKind::TypedArray`) to label
  `Arc<DequeData>` payloads." This was the Wave-α D-raw-helpers
  type-confusion gap (commit `a27c0e4`) generalized to deque labels
  — wrong-type retain/release at every push site.
- "Bool-default fallback at the receiver-kind mismatch site."
  Forbidden #9 (CLAUDE.md "Forbidden Patterns"). The `as_deque`
  receiver projector returns `RuntimeError` on mismatch.
- "Skip the `HeapValue::Deque` arm and use a pure-discriminator
  `HeapKind::Deque` like FilterExpr / SharedCell."
  `HeapValue::Deque` is required because handler bodies recover the
  typed `Arc<DequeData>` via `slot.as_heap_value()` (per ADR-005 §1
  single-discriminator) — there is no second recovery path. The
  pure-discriminator pattern is reserved for variants whose payloads
  never flow through `HeapValue` materialization; deque handlers do,
  so the parallel arm is load-bearing here.
- "Transitional shims preserving deleted Deque-shape names
  (`deque_legacy_push`, `vw_pop_front`, `extract_deque`)." This is
  the W-series "borrowed-bits with call-pattern invariants"
  defection-attractor at the deque-API layer — refuse on sight,
  migrate every caller to the kinded API in-wave.

**Out-of-scope this amendment:** snapshot/wire serialization of
in-flight Deque state (the §2.7.4 phase-2c marshal rebuild covers
this for HashMap / HashSet / Deque uniformly); element-kind
specialisation (Path B `TypedDeque<T>` per kind, future
amendment).
#### 2.7.22 Matrix and MatrixSlice exit `TypedArrayData` — Q23 amendment (Round 18 S3 W12-matrix-floatslice-heapkind-exit, 2026-05-13)

Round 18 S3 supersedes the prior Q23 audit-only ruling (§2.7.22 below,
W15-matrix 2026-05-10) and the §2.7.22 subsection's "no new HeapKind"
disposition. The Round 17 W12-typed-array-data-deletion audit
(`docs/cluster-audits/w12-typed-array-data-deletion-audit.md` §2.3 /
§2.4) named `TypedArrayData::Matrix(Arc<MatrixData>)` and
`TypedArrayData::FloatSlice { parent, offset, len }` as
**category-error** variants: Matrix is a **single Matrix value**, not
a buffer-of-Matrix; FloatSlice is a **projection-into-a-Matrix**, not
a buffer of floats. Their residency in `TypedArrayData` was a
second-order consequence of the ADR-005 §1 single-discriminator
concern that motivated Q23's parallel-HeapKind-refusal.

Under the Round 17 deletion-audit + cluster-0-transition
strategic-owner authorization (2026-05-13), `TypedArrayData::Matrix`
and `TypedArrayData::FloatSlice` are deleted. Once the second label
is gone, the parallel-HeapKind-discriminator concern that motivated
Q23 evaporates: `HeapKind::Matrix` and `HeapKind::MatrixSlice` are
**not parallel discriminators of `HeapKind::TypedArray`** (which is
the array-buffer carrier with element-typed payload); they are
**separate value categories** — a structured numeric matrix value and
a row/column projection — that share zero structural shape with the
element-typed-array carrier.

**Decision (Q23 amendment, Round 18 S3):**

- `HeapKind::Matrix = 34` (next free after `ModuleFn = 33`).
- `HeapValue::Matrix(Arc<MatrixData>)` — typed-Arc pure-discriminator
  arm, mirror of §2.7.9 FilterExpr. Slot bits are
  `Arc::into_raw(Arc<MatrixData>) as u64`; `as_heap_value()` is
  unsound on Matrix-labeled bits (the slot stores an
  `Arc<MatrixData>` pointer, not a `*const HeapValue`). Heap dispatch
  for retain/release routes through the kind label
  (`clone_with_kind` / `drop_with_kind`), not through HeapValue
  materialization. Receiver classification in
  `op_call_method` dispatches `Ptr(HeapKind::Matrix)` directly to
  `MATRIX_METHODS` (no inner-arm sub-classification two-step).
- `HeapKind::MatrixSlice = 35` (next free).
- `HeapValue::MatrixSlice(Arc<MatrixSliceData>)` — typed-Arc
  pure-discriminator arm with the same dispatch shape as
  `HeapValue::Matrix`. `MatrixSliceData { parent: Arc<MatrixData>,
  offset: u32, len: u32 }` preserves the aliasing-into-parent
  semantics from the pre-amendment `TypedArrayData::FloatSlice`
  payload. Receiver classification dispatches
  `Ptr(HeapKind::MatrixSlice)` to `FLOAT_ARRAY_METHODS` (their
  numeric aggregations apply uniformly over the projection's flat
  f64 region).

**4-table lockstep updates** (post-§2.7.6 / Q8 cardinality rule):

1. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` retain/release at the matching
   `Arc::increment/decrement_strong_count::<MatrixData>` /
   `::<MatrixSliceData>` per kind label.
2. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::Clone` /
   `KindedSlot::Drop` mirror the same arms;
   `KindedSlot::from_matrix(Arc<MatrixData>)` /
   `KindedSlot::from_matrix_slice(Arc<MatrixSliceData>)`
   constructors land (§2.7.6 / Q8 carrier-API-bound preserved: one
   constructor per new heap variant, no per-heap accessor on
   `KindedSlot` itself).
3. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
   dispatches the new arms.
4. `crates/shape-value/src/heap_value.rs` — `TypedObjectStorage::drop`
   dispatches the new arms (matrix/slice payloads can live in
   TypedObject field slots).

Plus knock-on exhaustive-match additions in `printing.rs` /
`arithmetic/mod.rs::kind_type_name` /
`comparison/mod.rs::kind_type_name` /
`objects/typed_access.rs::kind_type_name`;
`shape-jit/src/ffi/call_method/mod.rs::receiver_type_name`
(JIT-side type-name dispatch); `wire_conversion.rs` /
`json_value.rs` arms (HeapValue serialization — same N7
architectural-choice deferral as the pre-amendment `TypedArrayData`
arms had: 2D-layout encoding policy undecided).

**Construction site migration:**

- `executor/objects/object_creation.rs::op_new_matrix` — pushes
  `Arc::into_raw(Arc<MatrixData>) as u64` with kind
  `Ptr(HeapKind::Matrix)` directly (pre-amendment shape:
  `Arc<TypedArrayData::Matrix(Arc<MatrixData>)>` under
  `Ptr(HeapKind::TypedArray)` — retired).
- `executor/objects/datatable_methods/core.rs::handle_toMat` —
  builds an `Arc<MatrixData>` and pushes via
  `KindedSlot::from_matrix`.
- `executor/objects/matrix_methods.rs::as_matrix` — recovers
  `Arc<MatrixData>` via the canonical reconstruct-clone-restore
  pattern at the typed-Arc payload directly (no inner-enum
  projection), kind-gating on `Ptr(HeapKind::Matrix)`. `matrix_slot`
  wraps via `KindedSlot::from_matrix`.
- No MatrixSlice construction site exists in current code (the
  pre-amendment `TypedArrayData::FloatSlice` variant had no
  constructor either — it was a dormant variant pinned to the
  category-error shape). The `KindedSlot::from_matrix_slice`
  constructor is provided for the eventual Matrix.row/col
  projection methods to land their projection-into-parent-buffer
  semantics (currently `mat.row(i)` / `mat.col(i)` materialise to
  fresh `Arc<TypedArrayData::F64>` arrays per the pre-amendment
  matrix_methods bodies).

**Cardinality cost:** `HeapKind` grows from 34 variants (0..33) to 36
(0..35); the §2.7.6 Q8 bound (~25 constructors / ~5-10 scalar
accessors max on `KindedSlot`) is unchanged because Matrix /
MatrixSlice each get one matching constructor and no scalar
accessor — heap dispatch goes through the kind label per the
§2.7.9 FilterExpr precedent. Total dispatch surface grows by two
arms per dispatch table.

**Forbidden alternatives this amendment rules out (defection-attractor
class extends per CLAUDE.md "Renames to refuse on sight"):**

- "Keep Matrix in `TypedArrayData` under documented exception" —
  refused on sight. The deletion is systematic; Q23 is being
  superseded, not preserved with a footnote.
- "Preserve TypedArrayData::Matrix for one variant" —
  parallel-implementation-across-producer/consumer-carrier-shape-
  boundaries defection (CLAUDE.md "Forbidden Patterns" §
  "Parallel-implementation across producer/consumer carrier-shape
  boundaries").
- "Bridge / probe / helper / hop / translator / adapter / shim
  framing for the Matrix-out-of-TypedArrayData migration" —
  broader-family rule. Describe the migration by name (Matrix
  HeapKind exit, FloatSlice HeapKind exit) or by deletion-fate
  (the deleted `TypedArrayData::Matrix` / `FloatSlice` arms),
  never by hypothetical role.

**Provenance:** Round 17 W12-typed-array-data-deletion audit
(`docs/cluster-audits/w12-typed-array-data-deletion-audit.md` §2.3
/ §2.4 — the category-error finding) + cluster-0-transition
strategic-owner authorization (2026-05-13). Round 18 S3
W12-matrix-floatslice-heapkind-exit closes the migration in a
single commit per the supervisor's directive (new HeapKind
allocations + variant removal + dispatch tables + amendment text
co-located).

#### 2.7.22 Matrix lives under `HeapKind::TypedArray` — Q23 audit-only ruling (Wave 15 W15-matrix, 2026-05-10, **SUPERSEDED**)

**Status (2026-05-13):** SUPERSEDED by the §2.7.22 Round 18 S3
amendment above. The text below is preserved for historical
provenance — the Q23 ruling that Matrix continues to live under
`HeapKind::TypedArray` via `TypedArrayData::Matrix` was retired when
the Round 17 deletion-audit named the category-error and Round 18 S3
landed the systematic exit. Read the amendment above for the
current ruling.


W15-matrix (close 2026-05-10) audited the wave-14-15-16 playbook §2
W15-matrix sub-cluster proposal to add `HeapKind::Matrix = 29` +
`HeapValue::Matrix(Arc<MatrixData>)` adjacent to the §2.7.15 HashSet
rebuild precedent. The audit's critical finding: **MatrixData already
exists** as an Arc-backed payload reachable through the existing
`TypedArrayData::Matrix(Arc<MatrixData>)` arm under
`HeapKind::TypedArray = 8`. Adding a parallel `HeapKind::Matrix` would
create the exact failure mode §2.7.9 documents (Wave-γ
G-heap-filter-expr / commit `a27c0e4`) where the same `Arc<T>`
payload was indexed under two different `HeapKind` labels and
`clone_with_kind` / `drop_with_kind` dispatched the wrong-type
retain/release.

**Decision (Q23 ruling):** Matrix continues to live under
`HeapKind::TypedArray` via the existing `TypedArrayData::Matrix(Arc<
MatrixData>)` arm. The HeapKind ordinal 29 reserved by the playbook is
**vacated** — no `HeapKind::Matrix` variant is added. The
`HeapValue::Matrix(...)` arm remains absent (deleted in the Phase 2
ValueWord bulldozer; not reintroduced). The 4 lockstep dispatch tables
(§2.7.7 / §2.7.8 / §2.7.9 / §2.7.10) need NO new arms for Matrix
because `Arc<TypedArrayData>` retain/release already covers
`TypedArrayData::Matrix(Arc<MatrixData>)` through enum-variant
`Clone` / `Drop` (the inner `Arc<MatrixData>` is refcount-managed by
the enum constructor).

**Why not pure-discriminator (mirror of §2.7.9 FilterExpr / §2.7.12
SharedCell):** §2.7.9 / §2.7.12's pure-discriminator shape is
justified because their payloads (`Arc<FilterNode>` / `Arc<SharedCell>`)
are emitted directly to the kinded stack as `Arc::into_raw(...) as u64`
and never wrapped in `HeapValue` — `as_heap_value()` is unsound on
those slot bits. Matrix values **do** flow through `HeapValue`
materialization: `op_new_matrix` (`crates/shape-vm/src/executor/
objects/object_creation.rs`) pushes `Arc<TypedArrayData>` containing
`TypedArrayData::Matrix(Arc<MatrixData>)`, with kind
`Ptr(HeapKind::TypedArray)`; method-handler bodies recover the typed
`Arc<MatrixData>` via the §2.7.10 / Q11 single-discriminator path
(`slot.as_heap_value()` → `HeapValue::TypedArray(arr)` → match
`arr.as_ref()` against `TypedArrayData::Matrix`). Pure-discriminator
status would re-introduce the §2.7.9 type-confusion pattern at a
different layer.

**Why not a separate full `HeapValue::Matrix(Arc<MatrixData>)` arm
(mirror of §2.7.15 HashSet / §2.7.16 Iterator):** §2.7.15 HashSet and
§2.7.16 Iterator are full HeapValue arms because their payloads
(`HashSetData` / `IteratorState`) are **new** typed structs with no
prior carrier — a new HeapValue arm is the canonical landing site for
a new typed Arc payload. Matrix's situation is the inverse: the typed
struct (`MatrixData`) **already has a carrier** via
`TypedArrayData::Matrix`, and `TypedArrayData::FloatSlice { parent:
Arc<MatrixData>, ... }` structurally depends on `Arc<MatrixData>` as
parent through that carrier. Adding `HeapValue::Matrix(Arc<MatrixData>)`
would create two parallel HeapKind labels (`HeapKind::Matrix` direct
+ `HeapKind::TypedArray` via `TypedArrayData::Matrix`) for the same
`Arc<MatrixData>` payload, which is exactly the ADR-005 §1
single-discriminator forbidden pattern.

**Receiver-projection contract:** Matrix method handlers recover the
typed `Arc<MatrixData>` via the canonical reconstruct-clone-restore
pattern established by `array_transform::handle_map_v2` and
`iterator_methods::clone_typed_array_arc`:
`Arc::<TypedArrayData>::from_raw(slot.raw() as *const TypedArrayData)`,
match the inner against `TypedArrayData::Matrix(m)`, `Arc::clone(m)`
to bump the inner share, then `Arc::into_raw(outer)` to restore the
slot's outer share. **`slot.as_heap_value()` is unsound on
`Ptr(HeapKind::TypedArray)`-kinded bits** — `ValueSlot::from_typed_array`
stores `Arc::into_raw(Arc<TypedArrayData>) as u64` directly, NOT
`Box<HeapValue>` (the deleted `from_heap` shape ADR-005 §3 retired);
interpreting those bits as `*const HeapValue` would dereference into
the wrong type. The `slot.as_heap_value()` recovery path is reserved
for variants whose carrier path goes through `Box<HeapValue>` (the
pre-§2.3 deleted shape) or whose construction stores
`Arc::into_raw(Arc<HeapValue>) as u64` (the §2.7.16 Iterator pattern,
where `HeapValue::Iterator(arc)` recovery via `as_heap_value()` IS
sound).

**Method dispatch path:** the `MATRIX_METHODS` PHF table in
`crates/shape-vm/src/executor/objects/method_registry.rs` (18
handlers — `transpose` / `inverse` / `det` / `determinant` / `trace`
/ `shape` / `reshape` / `row` / `col` / `diag` / `flatten` / `map` /
`sum` / `min` / `max` / `mean` / `rowSum` / `colSum`) is filled with
real bodies operating on `Arc<MatrixData>` (post-W15-matrix close,
this commit). The receiver classification cascade routes
`Ptr(HeapKind::TypedArray)`-kinded receivers to either `ARRAY_METHODS`
or `MATRIX_METHODS` based on the inner `TypedArrayData` arm — that
routing is the §2.7.10 / Q11 dispatch shell's territory, owned by
W16-op-call-method. W15-matrix close-out lands the bodies; W16 lands
the routing.

**Mechanical lockstep updates (NONE for Matrix per this ruling):**

1. `crates/shape-value/src/heap_variants.rs` — **no changes**. Ordinal
   29 stays vacated; HashSet (21) and Iterator (22) remain the most
   recent additions.
2. `crates/shape-vm/src/executor/vm_impl/stack.rs` — **no changes** to
   `clone_with_kind` / `drop_with_kind`. The existing
   `Ptr(HeapKind::TypedArray)` arm (which retains/releases
   `Arc<TypedArrayData>`) covers Matrix transparently.
3. `crates/shape-value/src/kinded_slot.rs` — **no changes** to
   `KindedSlot::Drop` / `KindedSlot::Clone`. Same reasoning.
4. `crates/shape-value/src/v2/closure_layout.rs` — **no changes** to
   `SharedCell::drop`. Same reasoning.
5. `crates/shape-value/src/heap_value.rs` — **no changes** to
   `TypedObjectStorage::drop`. The existing `NativeKind::Ptr(HeapKind::
   TypedArray)` arm covers Matrix-bearing slots transparently.
6. `kind_type_name` maps in `printing.rs` / `arithmetic/mod.rs` /
   `comparison/mod.rs` / `objects/typed_access.rs` — **no changes**.
   Matrix slots type-name as "array" via `HeapKind::TypedArray`; the
   inner-arm distinction surfaces in `TypedArrayData::type_name()`
   ("Mat<number>") at user-facing `print` time only.
7. Wire/JSON conversion arms in `wire_conversion.rs` / `json_value.rs`
   — **no changes**. The pre-existing `TypedArrayData::Matrix` rejection
   (commit predating this audit; see `json_value.rs` line ~244 "Matrix
   serialization policy not yet decided (N7 architectural-choice
   deferral)") stands; deciding the encoding is out-of-scope here.

**Anti-pattern call-out:** the playbook §2 W15-matrix proposal of
`HeapKind::Matrix = 29` was a templating error — the
W15-priority-queue / W15-deque / W15-channel sub-clusters are
structurally similar to W15-matrix but have no pre-existing
`Arc<T>`-backed carrier, so for them the §2.7.15 HashSet recipe
(new HeapKind + new HeapValue arm) is the right shape. W15-matrix's
inverse situation is the W15-column-style "audit may reveal this is a
deletion candidate, not a rebuild" outcome the playbook §2 W15-column
section explicitly anticipated; W15-matrix is the second sub-cluster
in this wave to land that outcome (column being the first if its
audit lands the same way).

**Forbidden patterns refused at audit time:**

- A "tag-decode bridge" / "matrix-typed-array adapter" / similar
  CLAUDE.md "Renames to refuse on sight" framing for the proposed
  `HeapKind::Matrix` parallel discriminator. Per the broader-family
  rule the proposal would have shipped under that defection-attractor
  framing.
- Bool-default fallback at the `as_matrix` projection helper — the
  receiver must be `Ptr(HeapKind::TypedArray)` AND the inner
  `TypedArrayData` arm must be `Matrix`; mismatch surfaces a typed
  `RuntimeError` per ADR-006 §2.7.10 / Q11.
- Transitional shims preserving deleted Matrix-shape names —
  `from_matrix` / `from_float_array` / `vw_drop` / `vmarray_from_vec`
  / `extract_matrix` are not reintroduced.
- ValueWord revival in `Vec<ValueWord>` disguise — payload is typed
  `AlignedVec<f64>` end-to-end; closure-callback values flow through
  `KindedSlot` per §2.7.10 / Q11.

**Out-of-scope this amendment:**

- The `matrix(...)` stdlib ctor (Wave 5e per playbook §2 W15-matrix
  smoke-target note); user-facing matrix construction via a
  function-call form is W15-matrix-adjacent but not the body fill
  this amendment lands.
- The W16 `op_call_method` dispatch shell that routes
  `Ptr(HeapKind::TypedArray)`-kinded receivers to `MATRIX_METHODS` vs
  `ARRAY_METHODS` based on inner `TypedArrayData` arm — owned by
  W16-op-call-method per playbook §3.
- Snapshot/wire serialization of Matrix (shape-runtime/snapshot.rs
  `BlobKind::Matrix` exists pre-audit; encoding policy is an N7
  architectural-choice deferral per `json_value.rs` line ~244).
#### 2.7.21 `HeapKind::Column` — formal deletion (Q22 ruling, W15-column, 2026-05-10)

W15-column (close 2026-05-10) is unique among the W15 sub-cluster
agents: the audit-first step ruled the assigned territory **redundant
with `HeapKind::TableView`**, and the close is a deletion rather than a
rebuild.

**The W15-column audit findings.** The W15 playbook
(`docs/cluster-audits/wave-14-15-16-playbook.md` §2 W15-column)
allocated `HeapKind::Column = 28` for a "single typed-buffer column"
HeapValue, with the audit instruction "is this redundant with
`TypedArrayData`? if so, surface the redundancy and propose either
drop or keep with rationale". The audit found:

1. **No `HeapKind::Column` exists.** The Phase-2b §2.3 trim removed
   `HeapValue::ColumnRef { schema_id, table, col_id }` (the v1
   `ValueWord::from_column_ref` payload) along with every other
   `ValueWord`-shaped variant. `crates/shape-value/src/heap_variants.rs`
   shows the surviving 23 ordinals (String=0 through Iterator=22); no
   ordinal labels a `Column` shape.
2. **The Column semantics are absorbed by TableView.** The kinded
   replacement landed inside `HeapKind::TableView = 10`:
   `TableViewData::ColumnRef { schema_id, table: Arc<DataTable>,
   col_id: u32 }` (`crates/shape-value/src/heap_value.rs:1558`). This
   is the projection-into-a-DataTable that the v1 `Column` shape
   modeled — not a standalone typed buffer.
3. **A standalone Column would not be a thin wrapper around
   TypedArrayData.** `TypedArrayData` (heap_value.rs:1266) is the
   *element-typed buffer* primitive (Int64 / Float64 / Bool / String /
   HeapValue / Matrix / typed-int variants). v1 `Column` was a
   *projection into a DataTable* (column-id-keyed view, retains the
   table for cross-column operations). The two are not the same shape:
   a Column needs the parent-table reference. The TableView::ColumnRef
   carrier already holds it.
4. **The dispatch surface is dead.** The pre-W15 worktree has a
   `crates/shape-vm/src/executor/objects/column_methods.rs` file with
   11 `NotImplemented(SURFACE)` handlers and a `COLUMN_METHODS` PHF map
   referencing them, but no dispatch shell classifies a receiver kind
   into `COLUMN_METHODS` — the PHF is unreachable. The compiler-side
   stdlib `extend Column { ... }` declarations
   (`crates/shape-runtime/stdlib-src/core/column_methods.shape`)
   compile-type-check `column.method()` calls but produce only the
   surface error at runtime. The whole substrate is dead code.

**Decision (Q22 ruling):** delete the dead `Column` substrate. Do not
add `HeapKind::Column = 28`; do not introduce
`HeapValue::Column(Arc<ColumnData>)`; do not add a parallel
`ColumnData` typed-buffer struct. Column-shaped APIs (single-column
aggregation across a `DataTable`) belong on the existing
`TableView::ColumnRef` projection and live in
`crates/shape-vm/src/executor/objects/datatable_methods/`; new entries
on `COLUMN_METHODS` would merely duplicate dispatch that the
TypedArray and DataTable PHFs already cover.

**Why deletion is correct, not "keep with rationale":** the W13-hashset
recipe (close `0da1477`) that the playbook prescribes as the canonical
rebuild template adds value when the audited shape has unique payload
semantics not yet expressible (HashSet's one-keyspace dedup index,
Iterator's cursor+transforms, etc.). The W15-column audit found the
opposite: every Column semantic has a kinded home — element buffer →
`TypedArrayData`, projection-into-table → `TableViewData::ColumnRef`,
single-column aggregation → DataTable methods on the parent
`TableViewData::TypedTable` carrier. Adding `HeapKind::Column = 28`
would reintroduce the redundancy that §2.3 trimmed and create a third
discriminator parallel to `TableView::ColumnRef` — exactly the
"parallel discriminator drift" hazard ADR-005 §1 / ADR-006 §2.3 spell
out.

**Mechanical scope of this close (lockstep deletions):**

1. `crates/shape-vm/src/executor/objects/column_methods.rs` — file
   removed. Was 11 surface-only handlers (`v2_len`, `v2_sum`,
   `v2_mean`, `v2_min`, `v2_max`, `v2_std`, `v2_first`, `v2_last`,
   `v2_to_array`, `v2_abs`, `v2_len` aliased to `length`).
2. `crates/shape-vm/src/executor/objects/mod.rs::pub mod
   column_methods;` — removed; replaced with a deletion-pointer
   comment citing this §-number.
3. `crates/shape-vm/src/executor/objects/method_registry.rs::COLUMN_METHODS`
   — removed; replaced with a deletion-pointer comment citing this
   §-number.
4. `crates/shape-runtime/stdlib-src/core/column_methods.shape` — file
   removed.
5. `crates/shape-runtime/stdlib-src/core/prelude.shape::use
   std::core::column_methods` — removed; replaced with a
   deletion-pointer comment.
6. `crates/shape-runtime/src/metadata/methods.rs::column_methods()` —
   reduced to an empty-`Vec` stub. Preserves the LSP-facing API
   surface (`tools/shape-lsp/src/completion/types.rs` calls
   `LanguageMetadata::column_methods()`); the stub returns no entries
   because there is no surviving runtime type named `Column` whose
   methods could populate it. The LSP `is_column_type` heuristic —
   string-matching `series` / `column` / `series<...>` against typed
   completions — is unaffected; the stub returns no completions when
   the heuristic fires, which is correct behavior post-deletion.

**Not deleted, intentionally:**

- `FunctionCategory::Column` enum variant + the four `category:
  "Column"` `BuiltinMetadata` entries
  (`crates/shape-runtime/src/builtin_metadata.rs`). These categorize
  *free* stdlib functions whose signatures take `Table<any>`, not the
  deleted `Column` value-type. The categorization label is for
  documentation (`shift`, `resample`, `map`, `filter` over a
  `Table<any>`); the function bodies operate on the surviving
  TableView shape, not on a Column carrier.
- `crates/shape-abi-v1/src/binary_builder.rs::ColumnData` — a
  serialization-format enum for the binary export format
  (`Float64` / `Timestamp` / `Int64` / `String` / `Bool` per-column
  buffer). Unrelated to runtime HeapValue dispatch; serves the wire
  format only. ADR-005 §2 / ADR-006 §2.7.5.1 wire-format-shapes are a
  separate layer.
- `crates/shape-value/src/column_store/{native_dense_store,arrow_store,mod}.rs`
  + `crates/shape-value/src/datatable.rs::ColumnPtrs` — the
  `DataTable`'s internal column storage (Arrow / native dense). These
  back the surviving `HeapKind::TableView` and are not Column
  HeapValues; they are the TypedTable carrier's typed-buffer payload.

**Forbidden shapes ruled out by this deletion:**

- A `HeapKind::Column = 28` ordinal labeled with thin-wrapper
  semantics — e.g., `HeapValue::Column(Arc<TypedArrayData>)` or
  `HeapValue::Column { name: Arc<String>, data: Arc<TypedArrayData> }`.
  The first is exactly the redundancy the playbook audit-instruction
  flagged; the second is `TableViewData::ColumnRef`'s shape minus the
  parent-table reference, which makes cross-column operations
  ill-typed.
- A revived `HeapValue::ColumnRef { schema_id, table, col_id }` arm.
  The §2.3 trim is binding; recovery goes through
  `slot.as_heap_value()` → `HeapValue::TableView(arc)` →
  `TableViewData::ColumnRef { ... }` match.
- A "Column shim retained for stdlib compat" — same defection-
  attractor family as the W-series ValueWord renames per CLAUDE.md
  "Forbidden rationalizations". The stdlib `extend Column` block is
  compiler-only declarative metadata for `column.method()` syntax;
  with the runtime substrate gone, the metadata has nothing to bind
  to. Restoring it requires a new ADR amendment proposing a Column
  HeapValue with measured justification (the same bar as ADR-005 §2's
  String exception).

**Phase-2c follow-up (not blocking this close):** the
`TableViewData::ColumnRef` projection methods are not yet enumerable
through the LSP metadata API — `column_methods()` returning an empty
Vec is the conservative behavior. When `TableView::ColumnRef` grows a
method registry (datatable_methods/ extension), the LSP completion
path should route through that registry under the `is_column_type`
heuristic; the heuristic itself is preserved. This is symmetric with
the §2.7.16 Iterator close: the kinded carrier exists, the LSP
metadata follows once the dispatch surface stabilizes.

**Provenance.** §-number 2.7.21 / Q22 is from the W14-15-16 playbook
§0 lockstep table; ordinal 28 was pre-assigned for `HeapKind::Column`
but is not consumed by this close (the next W15 sub-cluster taking the
ordinal-28 slot — if any — should bump per the playbook's "if your
ordinal is already taken at edit time, bump to the next free" rule
and cite this §-number as the deletion that freed it).
#### 2.7.20 `HeapKind::Channel` — concurrency-primitive carrier (Wave 15 W15-channel-rebuild, 2026-05-10)

**Question (Q21):** the strict-typing Phase-2 deletion removed the
`HeapValue::Concurrency(ConcurrencyData::{Channel, Mutex, Atomic, Lazy}(_))`
arm because every `ConcurrencyData` variant carried `ValueWord`-shaped
payload fields. The W13 playbook "Out of scope" list called for each
concurrency primitive to rebuild as its own Stage C cluster. **Channel
is the first concurrency primitive to land kinded.** What is the
correct typed-Arc shape for the rebuild, and how does it integrate
with the §2.7.4 task-scheduler boundary?

**Decision:** introduce `HeapKind::Channel` (ordinal 24 — bumped from drafted 23 at merge; W15-deque already took 23) +
`HeapValue::Channel(Arc<ChannelData>)`. Same retain/release dispatch
shape as the §2.7.15 HashSet precedent (full HeapValue arm, NOT
pure-discriminator like FilterExpr / SharedCell — receiver
classification at method dispatch flows through
`slot.as_heap_value()` per ADR-005 §1 single-discriminator).

**Storage shape.** Unlike HashMap / HashSet / Iterator (immutable-on-
clone with `Arc::make_mut` clone-on-write), Channel needs **interior
mutability** so two `Arc<ChannelData>` shares of the same channel
observe each other's `send` / `recv` mutations — the producer/
consumer-endpoints shape. The inner state therefore lives behind a
`Mutex<ChannelInner>`; the outer `Arc` is purely a refcount carrier.

```rust
pub struct ChannelData {
    inner: std::sync::Mutex<ChannelInner>,
}
struct ChannelInner {
    queue:  std::collections::VecDeque<KindedSlot>,
    closed: bool,
}
```

**Element typing.** The buffer stores `KindedSlot` payloads directly
so heterogeneous-element queues are first-class — a channel can carry
ints, strings, or typed objects without a per-element-kind
specialisation. Each queued slot owns one strong-count share for
heap-bearing kinds; the `KindedSlot::Drop` dispatch retires shares
cleanly when the channel itself drops. This is the same shape
`concurrency_methods.rs` (Mutex / Atomic / Lazy) will use when those
primitives rebuild.

**Sync-only path at landing.** The smoke target (`let c = Channel();
c.send(1); c.recv()` returns `1`) exercises the same-thread sync
path. Cross-task blocking `recv()` (the canonical async-channel use
case) requires integration with the §2.7.4 task-scheduler boundary
(`shape-vm/src/executor/task_scheduler.rs`) — the receiver suspends
until a producer `send()`s. Per the W15 playbook surface-and-stop
discipline, `recv()` on an empty queue returns `NotImplemented(SURFACE)`
citing this section + §2.7.4. `try_recv()` is the non-blocking poll
variant and is the canonical surface for sync use.

**Sender/receiver endpoint split.** The pre-bulldozer Channel design
had separate sender / receiver endpoint types (with
`is_sender()` to classify a handle). The rebuild collapses both into
a single `Arc<ChannelData>` carrier — any share is both producer and
consumer. The `is_sender()` method is preserved at the PHF surface
for source-compatibility but always errors with a SURFACE message;
re-introducing typed sender / receiver endpoints is a phase-2c
follow-up.

**Dispatch tables (mirror of §2.7.15 HashSet, lockstep updates):**

1. `clone_with_kind` / `drop_with_kind` (`vm_impl/stack.rs`) dispatch
   the `HeapKind::Channel` arm to
   `Arc::increment/decrement_strong_count::<ChannelData>`.
2. `KindedSlot::Drop` / `KindedSlot::Clone` (`shape-value/src/kinded_slot.rs`)
   mirror the same arm; new `KindedSlot::from_channel` constructor.
3. `SharedCell::drop` (`shape-value/src/v2/closure_layout.rs`) mirrors
   the same arm.
4. `TypedObjectStorage::drop` (`shape-value/src/heap_value.rs`) mirrors
   the same arm — a TypedObject field of kind
   `Ptr(HeapKind::Channel)` retires one `Arc<ChannelData>` strong-count
   share.

**Knock-on `kind_type_name` updates** in `arithmetic/mod.rs`,
`comparison/mod.rs`, `typed_access.rs`, `printing.rs` — Channel
displays as "channel" (formatter renders
`<channel:open:N>` / `<channel:closed:N>`).

**Wire / JSON.** Channels are concurrency primitives with interior
mutable state; no wire serialization at landing — same phase-2c
deferral shape as HashMap / HashSet. `wire_conversion.rs` surfaces
as opaque tag; `json_value.rs` rejects.

**Forbidden alternatives this rules out:**

- "Re-introduce `ConcurrencyData::Channel` in the deleted
  `ValueWord`-encoded shape under a less-suspicious name." This is
  the W-series defection-attractor (CLAUDE.md "Renames to refuse
  on sight"); the kind dispatch must go through the
  `HeapKind::Channel` arm and the payload must be a typed `Arc<T>`,
  never a `Box<HeapValue>` wrapper or a tag-bit-decoded carrier.
- "Skip the `Mutex<ChannelInner>` and use `Arc::make_mut` clone-on-
  write like HashMap / HashSet." Wrong semantics: the
  producer/consumer endpoints must observe each other's
  mutations. `Arc::make_mut` clones the inner state on the first
  mutation past refcount=1, breaking the shared-buffer contract.
- "Skip the `HeapValue::Channel` arm and use a pure-discriminator
  `HeapKind::Channel` like FilterExpr / SharedCell." Wrong fit:
  channel method handlers recover the typed `Arc<ChannelData>` via
  `slot.as_heap_value()` (per ADR-005 §1 single-discriminator) —
  there is no second recovery path. Same justification as the
  §2.7.15 HashSet / §2.7.16 Iterator arm-existence rulings.
- "Bool-default fallback for empty `recv()` on the assumption that
  `null` is the right answer." Forbidden #9 — a sync `recv()` on an
  empty queue is the §2.7.4 task-scheduler suspend point;
  surface-and-stop. Use `try_recv()` for the documented null-on-
  empty poll variant.

**Out-of-scope this amendment:** cross-task blocking `recv()` (the
§2.7.4 task-scheduler integration); typed sender / receiver endpoint
split (the pre-bulldozer two-handle shape); bounded-capacity
backpressure (`Channel(capacity)` ctor signature); snapshot / wire
serialization of in-flight queue state. All four are phase-2c
follow-ups tracked separately.
    PriorityQueue(Arc<PriorityQueueData>),
}
```

Slot bits for a `PriorityQueue`-kinded slot are
`Arc::into_raw(Arc<PriorityQueueData>) as u64` directly (mirror of the
§2.7.15 HashSet shape). Like Iterator, **`as_heap_value()` IS valid on
PriorityQueue-labeled bits**: the priority-queue method handlers
recover the typed `Arc<PriorityQueueData>` via the canonical
`slot.as_heap_value()` → `HeapValue::PriorityQueue(arc)` match,
preserving ADR-005 §1 single-discriminator. The shape is the same as
existing typed-Arc heap variants (`HeapValue::TypedArray`,
`HeapValue::HashMap`, `HeapValue::HashSet`, ...) — typed `Arc<T>`
payload, dispatch goes through both the kind label (for refcount
discipline at the §2.7.7 / §2.7.8 dispatch tables) and through
`HeapValue` (for handler-body recovery).

**Pre-assigned ordinal:** 25 per the wave-14-15-16 playbook §0 table.
No bump needed at landing (W14 took 23/24 for Result/Option; HashSet
=21, Iterator=22 are the §2.7.15 / §2.7.16 amendments; SharedCell=20,
Reference=19, FilterExpr=18 are the prior pure-discriminator
additions). If a concurrent amendment claims 25 first at merge time,
the bump-and-comment rule applies (same precedent as W8-T25/T26
19↔20).

**Mechanical lockstep updates** (4 dispatch tables — every Q8/Q10
retain/release table — plus knock-on exhaustive matches):

1. `shape-value/heap_variants.rs` — `HeapKind::PriorityQueue` ordinal
   25 + `HeapValue::PriorityQueue(Arc<PriorityQueueData>)` arm +
   `kind()` / `is_truthy()` / `type_name()` / Clone / Display.
2. `shape-vm/.../vm_impl/stack.rs` — `clone_with_kind` /
   `drop_with_kind` dispatch the PriorityQueue arm to
   `Arc::increment/decrement_strong_count::<PriorityQueueData>`.
3. `shape-value/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror; `from_priority_queue(Arc<
   PriorityQueueData>)` constructor.
4. `shape-value/heap_value.rs::TypedObjectStorage::drop` —
   PriorityQueue-typed field arm.
5. `shape-value/v2/closure_layout.rs::SharedCell::drop` —
   PriorityQueue-typed cell arm.
6. Knock-on exhaustive matches: `arithmetic/mod.rs`,
   `comparison/mod.rs`, `typed_access.rs` (kind→type-name maps);
   `printing.rs` (HeapKind + HeapValue arms — `format_priority_queue`
   renders as `PriorityQueue[v1, v2, ...]` in heap-array order);
   `wire_conversion.rs`, `json_value.rs` (PriorityQueue projects to a
   wire/JSON array of i64 priorities — i64-priority-only at landing).

**Migration of 7 NotImplemented sites** in
`executor/objects/priority_queue_methods.rs` plus 1 ctor in
`vm_impl/builtins.rs`:

```
Mutation handlers (Arc::make_mut clone-on-write):
  v2_push    -> push(value), returns the post-mutation Arc
  v2_pop     -> pop() unwrap_or(0), returns popped i64

Read-only handlers:
  v2_peek            -> peek() unwrap_or(0)
  v2_size            -> len() as i64
  v2_is_empty        -> is_empty() as bool
  v2_to_array        -> heap-array-order Vec<int>
  v2_to_sorted_array -> ascending Vec<int>

Ctor body (vm_impl/builtins.rs):
  PriorityQueueCtor  -> Arc::new(PriorityQueueData::new())
                        + KindedSlot::from_priority_queue
```

The `peek` / `pop` empty-queue convention is "return 0" at landing,
matching the deleted pre-bulldozer behavior shape; the
`Option<int>`-typed return is W14-variant-codegen territory and will
be re-amended once `OptionCtor` lands (the variant-codegen close is
the upstream §2.7.4 dependency).

**Forbidden alternatives considered and rejected:**

- "`HeapValue::PriorityQueue { values: Vec<KindedSlot>, comparator:
  Arc<HeapValue> }` — heterogeneous-payload + closure comparator from
  day one." Rejected for cardinality-cost reasons (the same shape
  §2.7.15 / Q16 rejected for HashSet's `TypedSet<T>` alternative). The
  i64-only shape lands the smoke target end-to-end and the
  typed-payload rebuild is a measured follow-up.
- "Re-use the existing `HeapValue::TypedArray(Arc<TypedArrayData::I64
  >)` carrier and stash heap-invariant invariant-respecting bits via a
  side-table." Rejected — would conflate two distinct receiver kinds
  (a generic `Vec<int>` vs a min-heap-laid-out `Vec<int>`) at the
  method dispatch layer, breaking §2.7.6/Q8 cardinality and ADR-005
  §1 single-discriminator. The receiver kind classifier MUST be able
  to distinguish a `[3, 1, 2]` `Vec<int>` (which has no `.push`/`.pop`
  heap semantics) from a `pq.heap = [1, 3, 2]` `PriorityQueue` (which
  does).
- "Surface PriorityQueue under the `HeapKind::HashSet` discriminator
  with a side-tag, on the basis that the storage shape is similar."
  Forbidden under §2.3 / Q8: each new HeapKind discriminator is an
  amendment in its own right. Off-label re-use of an existing variant
  is the precise W-series defection-attractor (CLAUDE.md "Forbidden
  rationalizations" #6 / #8: "Rename to a less suspicious name." /
  "Document it as out-of-scope.").
- "Render PriorityQueue as `[1, 2, 3, ...]` (sort order) in Display
  to match user mental model." Rejected — Display reflects storage
  order (heap-array layout); users who want sort-order output use
  `pq.toSortedArray()` (the explicit projection method). Rendering as
  sorted at the Display surface would compute on every print, which
  is both expensive and surprising (reads-have-side-effects in the
  monadic-purity sense).

**Out-of-scope this amendment:** typed-payload `PriorityQueue<T, K>`
(arbitrary `T` payloads + `K`-extractor closure); custom comparator
closures (max-heap or arbitrary ordering — the i64 min-heap is the
fixed shape at landing); stable-priority semantics (insertion-order
preservation among equal priorities). All three are future Phase-2c
amendments with measurement; all three would require an §-level
amendment of their own.
#### 2.7.23 `HeapKind::Range` — Q24 amendment (W15-range, 2026-05-10)

**Trigger.** The `HeapKind::Range` ordinal was deleted by the strict-
typing bulldozer along with the cross-kind `HeapValue::Range { start:
Option<Box<ValueWord>>, end: Option<Box<ValueWord>>, inclusive: bool }`
shape. The kinded equivalent must answer: is `Range` a value type with
identity, or a thin sugar over `IteratorState`? W13-iterator-state
(§2.7.16) added an `IteratorSource::Range { start, end, step }`
forward-compatibility hook, leaving the Range receiver itself as a
phase-2c surface (`executor/objects/iterator_methods.rs::v2_range_iter`,
`executor/objects/mod.rs::op_make_range`). The W14-15-16 playbook §2
W15-range row asks: separate `HeapKind::Range` (Option A) or
deletion-candidate that collapses to `IteratorState` (Option B)?

**Decision (Q24 ruling):** Option A. `HeapKind::Range` is a separate
typed-Arc carrier (`HeapValue::Range(Arc<RangeData>)`) with its own
identity, methods, and conversion path to `IteratorState` via
`Range.iter()`. The boundary is:

- `RangeData` is a value with identity — `r.start`, `r.end`, `r.step`,
  `r.inclusive`, `r.contains(x)`, prints as `0..10` / `0..=10`. It is
  the receiver of the `start..end` / `start..=end` surface syntax.
- `IteratorState` is a stateful pipeline with a cursor, transform
  stages, and a source. Created by `r.iter()` (or `arr.iter()` /
  `s.iter()` / `m.iter()`). It does not have a printable literal form.
- The conversion at `Range.iter()` builds an `IteratorState { source:
  IteratorSource::Range { start: r.start, end: r.end_exclusive(),
  step: r.step }, transforms: empty, cursor: 0 }`. The
  inclusive-bound adjustment is baked into `end_exclusive` (`r.end +
  r.step` for inclusive ranges) so the post-conversion iterator
  preserves the right element count.

**Storage.** `RangeData` is four scalar fields (`i64`, `i64`, `i64`,
`bool`). It does not carry inner Arcs, so `Drop` at refcount=0 is just
`dealloc` of the small heap block. Slot bits are
`Arc::into_raw(Arc<RangeData>) as u64` with kind
`NativeKind::Ptr(HeapKind::Range)` per the §2.3 typed-Arc shape (mirror
of HashMap / HashSet / Iterator).

**Bounds today are i64 only.** Cross-kind range bounds (Decimal, BigInt,
Float64, NativeScalar) — the use case the deleted `Option<Box<
ValueWord>>` cross-kind shape was designed for — surface in
`op_make_range` as `NotImplemented` (with a precise side label
distinguishing open-range placeholders from genuine cross-kind bounds).
Following the playbook's surface-and-stop discipline, this is the
correct landing posture; the cross-kind extension is a follow-up
§2.7.23 amendment with measurement of which bound types are actually
needed (matches the §2.7.6 / Q8 cardinality-cost reasoning that gated
the W13-hashset Path A vs Path B decision).

**Step is implicit at landing.** The surface syntax does not have a step
suffix today (`0..10` step 3 is not expressible). `RangeData::step`
defaults to 1 in `op_make_range`. The field exists so `IteratorSource::
Range::step` (which W13-iterator-state already provides) round-trips
losslessly; explicit step syntax is a follow-up.

**Why not Option B (delete `HeapKind::Range`):**

- `HeapValue::Range` has methods that are not iterator methods —
  `r.contains(x)` is a bound test, `r.start` / `r.end` are accessors,
  `print(r)` produces the surface-syntax literal. Collapsing to
  `IteratorState` would lose all of these.
- `r.iter()` is observably idempotent — `r.iter().collect()` returns
  the same array each call. An `IteratorState` is single-use: cursor
  advances, terminals consume. The two have different identities.
- The pre-bulldozer `HeapValue::Range` was distinct from `HeapValue::
  Iterator`; collapsing them would be a semantic regression, not a
  simplification.

**Dispatch tables (lockstep with §2.7.7 / §2.7.8 / §2.7.10
amendments).** Range arms added to:

- `vm_impl/stack.rs::clone_with_kind` / `drop_with_kind`
- `kinded_slot.rs::KindedSlot::Drop` / `KindedSlot::Clone`
- `v2/closure_layout.rs::SharedCell::drop`
- `heap_value.rs::TypedObjectStorage::drop`

Plus `kind_type_name` -> `"range"` in `arithmetic/mod.rs`,
`comparison/mod.rs`, `objects/typed_access.rs`. `printing.rs` renders
the surface-syntax literal form (`0..10` / `0..=10`).
`json_value.rs` materializes to a JSON array of i64 (mirror of
HashSet's "array of strings" mechanical-yes mapping). `wire_conversion.
rs` emits the literal-form string (deferred structured-wire follow-up,
same shape as HashMap / HashSet).

**Refused alternatives:**

- "`HeapValue::Range` carries `Box<ValueWord>` bounds for cross-kind
  support." `ValueWord` is deleted. Cross-kind range bounds need a
  follow-up §2.7.23 amendment with measured demand, not a ValueWord
  revival under a renamed alias.
- "`Range` is a pure-discriminator like FilterExpr / SharedCell." The
  `HeapValue::Range` arm is required for `slot.as_heap_value()`-based
  receiver classification at method dispatch (the same shape HashMap /
  HashSet / Iterator use). Pure-discriminator is reserved for variants
  whose payloads never flow through `HeapValue` materialization.
- "Skip `HeapKind::Range` and let `IteratorState` carry everything."
  Loses Range identity (see Why-not-Option-B above); regression.

**Out-of-scope this amendment:** open-range syntax (`..n` / `n..` /
`..`), explicit step suffix syntax, cross-kind bounds (Decimal,
BigInt, Float64). All three surface in `op_make_range` /
`range_methods` with precise diagnostics; each is its own follow-up
amendment.
1. `crates/shape-value/src/heap_variants.rs` — `HeapKind::Result`
   ordinal 27 + `HeapKind::Option` ordinal 28 (both renumbered from
   drafted 23/24 at merge — Deque/Channel took those slots first) + matching
   `HeapValue::Result(Arc<ResultData>)` / `HeapValue::Option(Arc<OptionData>)`
   arms + `kind()` / `is_truthy()` / `type_name()` / `Clone` /
   `Display` updates.
2. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind`
   / `drop_with_kind` dispatch the new arms to
   `Arc::increment/decrement_strong_count::<ResultData|OptionData>`.
3. `crates/shape-value/src/kinded_slot.rs` — `KindedSlot::clone` /
   `KindedSlot::drop` mirror the same arms; new `from_result` /
   `from_option` constructors.
4. `crates/shape-value/src/v2/closure_layout.rs::SharedCell::drop` —
   mirror of (2). `crates/shape-value/src/heap_value.rs::TypedObjectStorage::drop`
   — mirror of (2).
5. `kind_type_name` updates in `executor/printing.rs`,
   `executor/arithmetic/mod.rs`, `executor/comparison/mod.rs`,
   `executor/objects/typed_access.rs` — Result/Option display as
   "result" / "option".
6. Wire/JSON arms (`shape-runtime/src/wire_conversion.rs`,
   `shape-runtime/src/json_value.rs`) reject Result/Option as
   within-program control-flow values (deferred to AnyError marshal
   for thrown errors / unwrapped inner for Ok/Some — out-of-scope
   for this amendment, same shape as §2.7.16 Iterator deferral).

**Why typed-Arc dispatch (mirror of Iterator §2.7.16) and not pure-
discriminator (FilterExpr §2.7.9 / SharedCell §2.7.12):**

- The variant discriminator handlers consume Result/Option values via
  `slot.as_heap_value()` → `HeapValue::Result(arc)` /
  `HeapValue::Option(arc)` for handler-body recovery (preserves
  ADR-005 §1 single-discriminator) — there is no second recovery
  path. The pure-discriminator pattern is reserved for variants whose
  payloads never flow through `HeapValue` materialization;
  variant-discriminator handlers do, so the parallel arm is
  load-bearing here.

- Result/Option values can be stored in TypedObject slots and
  `TypedArrayData::HeapValue` buffers (e.g. `Array<Result<int,
  string>>`); the storage-tier `TypedObjectStorage::drop` and
  `SharedCell::drop` dispatch tables retire `Arc<ResultData>` /
  `Arc<OptionData>` shares directly via the kind label, NOT through
  `HeapValue` materialization. This is the same shape as HashMap /
  HashSet / Iterator at the storage tier — full HeapValue arm but
  storage-tier dispatch goes through the kind label.

**Out-of-scope this amendment:** wire/JSON serialization of
in-flight Result/Option carriers (deferred to the AnyError marshal /
unwrapped-inner-value path). The compiler's null-coding emit path for
`Option<T>` (`op_is_null` test in `compiler/patterns/checking.rs:213`)
is **preserved as a fallback** at the consumer side
(`op_unwrap_option` / `op_try_unwrap` / `op_error_context`) — the
canonical `Some(x)` ctor produces an `Arc<OptionData>` carrier, but
legacy-emitted `None` (a null sentinel) and bare-value `Some(x) ≡ x`
flows are recognised by the discriminator handlers via the
`is_null_sentinel` helper. A future cluster migrates the compiler
emit to the kinded `OptionData` carrier exclusively.

#### 2.7.24 Typed-carrier monomorphization bundle — `TypedArrayData::HeapValue` deletion, `HashMapData` parametric value buffer, `HeapKind::TraitObject` re-introduction with universal `dyn Trait` (Phase 2d Q25 ruling, 2026-05-11)

This amendment is the architectural foundation Phase 2d sub-clusters consume.
It bundles three coordinated changes that share design DNA:

1. `TypedArrayData::HeapValue` arm is **deleted** and replaced by monomorphic specialization variants per built-in heap type plus a single `TypedObject` catch-all for user-defined types (Q25.A).
2. `HashMapData` becomes parametric over its value type via a new `HashMapValueBuf` enum (Q25.B).
3. `HeapKind::TraitObject = 29` is **re-introduced** with a fat-pointer `Arc<TraitObjectStorage>` carrier; **all traits are dyn-able** under a runtime auto-boxing rule (Q25.C).

The three changes land as one ADR amendment because they share the same forbidden-pattern (polymorphic catch-all arms with kind-blind raw-bits decode at use site) and the same replacement discipline (kind known at the variant level, runtime dispatch through typed Arcs).

##### Q25.A — `TypedArrayData::HeapValue` deletion + monomorphic specialization

**Decision:** the `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` arm is deleted. `TypedArrayData` gains specialized variants for every built-in heap type:

```rust
pub enum TypedArrayData {
    // Existing scalar / native-buffer variants — kept verbatim:
    I64, F64, Bool, I8, I16, I32, U8, U16, U32, U64, F32, String,
    Matrix(Arc<MatrixData>),
    FloatSlice { parent: Arc<MatrixData>, offset: u32, len: u32 },

    // NEW — specialized variants per built-in heap type:
    Decimal(Arc<TypedBuffer<Decimal>>),
    BigInt(Arc<TypedBuffer<BigInt>>),
    DateTime(Arc<TypedBuffer<DateTimeData>>),
    Timespan(Arc<TypedBuffer<TimespanData>>),
    Duration(Arc<TypedBuffer<DurationData>>),
    Instant(Arc<TypedBuffer<InstantData>>),
    Char(Arc<TypedBuffer<u32>>),     // unicode scalar values

    // NEW — single catch-all for user-defined types (structs, enums):
    TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>),

    // NEW — single catch-all for trait-object element types:
    TraitObject(Arc<TypedBuffer<Arc<TraitObjectStorage>>>),
}
```

**Per-element kind is uniform per variant.** `Array<int>` carries `NativeKind::I64` for every element; `Array<DateTime>` carries `NativeKind::Ptr(HeapKind::Temporal)` for every element; `Array<EnumX>` carries `NativeKind::Ptr(HeapKind::TypedObject)` for every element (with the enum schema discriminating variants inside each `TypedObjectStorage`).

**No parallel `Vec<NativeKind>` track.** The variant tag IS the kind. This is the structural difference from §2.7.7 (stack) and §2.7.8 (cells), which DO carry parallel kind tracks because their *call patterns* admit truly heterogeneous slots. Arrays do not.

**Rust-style enum layout.** Shape enums compile to a fixed-size-per-type layout (discriminator + max-payload-bytes, heap variants stored as pointers). `Array<MyEnum>` is therefore uniform per element — no polymorphism at the array level.

**Forbidden:** re-introducing `TypedArrayData::HeapValue` as a "polymorphic fallback" / "catch-all element buffer" / "any-shaped array carrier" — same defection-attractor family as the deleted ValueWord; refuse on sight. If a hypothetical use case appears, surface and amend.

**Method-handler implications.** Every SURFACE site at `objects/array_transform.rs`, `objects/array_aggregation.rs`, `objects/array_basic.rs`, `objects/iterator_methods.rs`, `objects/string_methods.rs` that today returns `NotImplemented(SURFACE)` on `TypedArrayData::HeapValue` becomes either:

- **Filled** with the corresponding specialized variant's body (e.g. `TypedArrayData::Decimal(arr)` → direct Decimal-aware sum/max/etc.); or
- **Replaced** with a per-variant exhaustive match (no `_` catch-all) where each arm dispatches monomorphically; or
- **Re-routed** through `TypedObject` / `TraitObject` for the user-type cases, where the schema or vtable carries the dispatch decision.

##### Q25.B — `HashMapData` parametric value buffer

**Decision:** `HashMapData` is refactored from a single struct with `values: Arc<TypedBuffer<Arc<HeapValue>>>` to a struct with a parametric `HashMapValueBuf` enum:

```rust
pub struct HashMapData {
    pub keys: Arc<TypedBuffer<Arc<String>>>,
    pub values: HashMapValueBuf,
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}

pub enum HashMapValueBuf {
    // Specialized per common value type — mirrors TypedArrayData variants:
    I64(Arc<TypedBuffer<i64>>),
    F64(Arc<TypedBuffer<f64>>),
    Bool(Arc<TypedBuffer<u8>>),
    String(Arc<TypedBuffer<Arc<String>>>),
    Decimal(Arc<TypedBuffer<Decimal>>),
    BigInt(Arc<TypedBuffer<BigInt>>),
    DateTime(Arc<TypedBuffer<DateTimeData>>),
    Timespan(Arc<TypedBuffer<TimespanData>>),
    Duration(Arc<TypedBuffer<DurationData>>),
    Instant(Arc<TypedBuffer<InstantData>>),

    // User-defined value types:
    TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>),
    TraitObject(Arc<TypedBuffer<Arc<TraitObjectStorage>>>),
}
```

**Same discipline as Q25.A.** Per-value kind known at variant level. No parallel kind track. No `Arc<HeapValue>` catch-all. The `index` field (FNV-1a bucket index) is unchanged — it operates on keys.

**Resolves the `datetime_methods.rs:426` cascade.** `v2_diff` returning `HashMap<String, int>` is now a `HashMapValueBuf::I64` construction — no architectural amendment beyond this one.

**Keys remain string-typed at landing.** `HashMap<K, V>` with non-String K is deferred to a follow-up amendment if/when the use case appears. Today's stdlib + tests only use String keys.

##### Q25.C — `HeapKind::TraitObject = 29` with `Arc<TraitObjectStorage>` carrier

**Decision:** the bulldozer-deleted `HeapValue::TraitObject { value: Box<u64>, vtable: Arc<VTable> }` is replaced by a kinded `HeapValue::TraitObject(Arc<TraitObjectStorage>)` arm, mirroring §2.3 typed-Arc shape:

```rust
pub struct TraitObjectStorage {
    /// The data half of the fat pointer — owned, heap-allocated.
    /// Always heap (no inline-scalar trait objects at landing — scalars
    /// that implement traits are boxed into TypedObject first; see §Q25.C.4).
    pub value: Arc<TypedObjectStorage>,

    /// The vtable half of the fat pointer.
    pub vtable: Arc<VTable>,
}
```

**Ordinal:** `HeapKind::TraitObject = 29` (next free after Option=28). Pre-assigned for the W17-trait-object-rebuild sub-cluster. Bump-on-collision per §0 ordinal-collision rule.

###### Q25.C.1 — Universal `dyn Trait` (no object-safety rule)

**Decision:** every trait in Shape is dyn-able. Rust's object-safety restrictions (no generic methods, no `Self` in non-receiver position, no associated types without bounds, no associated functions) are **lifted**, at the cost of runtime indirection that is recoverable via JIT IC devirtualization.

The substitution operator `Erase_T(τ)` (the auto-boxing rule) determines how a method signature is rewritten when the trait is used as `dyn T`:

| Input `τ` | `Erase_T(τ)` |
|---|---|
| `Self` | `dyn T` (carrier: `Arc<TraitObjectStorage>` with same `trait_id`) |
| `&Self` | `&dyn T` |
| `&mut Self` | `&mut dyn T` |
| `Self::A` where `trait T { type A: Bound; }` | `dyn Bound` |
| `Self::A` where `trait T { type A; }` (no bound) | **compile error ETO-001** |
| `G<τ₁, τ₂, …>` where `G ∈ {Option, Result, Vec, Box, Arc, HashMap, HashSet, tuples, user-#[erasure_safe]}` | `G<Erase_T(τ₁), Erase_T(τ₂), …>` (recursive) |
| `&G<…>` / `&mut G<…>` | reference unchanged, recurse into payload |
| method-generic `G` | `KindedSlot` + runtime `&TypeInfo` parameter |
| any concrete or builtin type | unchanged |

**Worked examples:**

```text
fn clone(&self) -> Self
  → fn clone(&self) -> dyn T

fn try_clone(&self) -> Result<Self, Error>
  → fn try_clone(&self) -> Result<dyn T, Error>

fn iter(&self) -> Self::Iter   // trait declares `type Iter: Iterator`
  → fn iter(&self) -> dyn Iterator

fn split(self) -> (Self, Self)
  → fn split(self) -> (dyn T, dyn T)
```

###### Q25.C.2 — `Self` in argument position: runtime vtable-identity check

A method `fn merge(&mut self, other: Self)` through a `dyn T` receiver is exposed to the caller as `fn merge(&mut self, other: dyn T)`. The thunk performs a runtime check at dispatch time:

```rust
if !Arc::ptr_eq(&self_storage.vtable, &other_storage.vtable) {
    return Err(VMError::TraitObjectIdentityMismatch {
        method: "merge",
        self_impl: self_storage.vtable.concrete_type_id,
        other_impl: other_storage.vtable.concrete_type_id,
    });
}
```

Cost: one pointer comparison per `Self`-typed argument. Same rule applies to multi-argument cases (`fn foo(&self, a: Self, b: Self)` — two checks).

###### Q25.C.3 — Generic method parameters: runtime type-info

A method `fn method<G: Bound>(&self, g: G) -> R` becomes, through `dyn T`:

```text
fn method(&self, g: KindedSlot, g_type_info: &TypeInfo) -> Erase_T(R)
```

`TypeInfo` carries `{ concrete_type_id: u32, vtable_for_bound: Option<Arc<VTable>>, size_align: (u32, u32) }`. The thunk body uses `g_type_info` to dispatch operations on `g`.

The IC at this call site records `(self_vtable_arc_id, g_type_info.concrete_type_id)`. When it stabilizes, the optimizing tier emits a direct call.

###### Q25.C.4 — `#[static_only]` per-method opt-out

A method marked `#[static_only]` is dispatchable only through static call sites (`<X as T>::method(&x)`) and is **excluded** from the vtable entirely. Calling it through `dyn T` is **compile error ETO-002**:

```text
error[ETO-002]: method `cold_path` is marked `#[static_only]` and cannot be
  called through `dyn T`. Use a static call site (`<X as T>::cold_path(&x)`)
  or remove the `#[static_only]` attribute.
```

This is the trait author's cost-control surface. Default is universal-dyn; opt-out is per-method, explicit.

###### Q25.C.5 — `VTable` and `VTableEntry` final shape

```rust
pub struct VTable {
    pub trait_names: Vec<String>,            // multi-trait inheritance support
    pub concrete_type_id: u32,                // enables Self-arg runtime check
    pub methods: HashMap<String, VTableEntry>,
}

pub enum VTableEntry {
    /// No Erase_T rewriting: no Self in non-receiver position, no generics.
    Direct { function_id: u16 },

    /// Pre-existing closure entry (W7 closure trait impls).
    Closure { function_id: u32, type_id: u32 },

    /// Self / Self::Assoc in return position. Thunk wraps return value.
    BoxedReturn {
        thunk_id: u16,
        wrap_targets: SmallVec<[WrapTarget; 2]>,
    },

    /// Self in argument position. Thunk checks vtable identity per Self-arg.
    SelfArg {
        thunk_id: u16,
        self_arg_positions: SmallVec<[u8; 4]>,
    },

    /// Generic method. Thunk consumes TypeInfo per generic param.
    Generic {
        thunk_id: u16,
        type_param_count: u8,
    },

    /// Combinations of the above three. Thunk dispatches per flag set.
    Compound {
        thunk_id: u16,
        flags: VTableEntryFlags,                 // bitfield
        wrap_targets: SmallVec<[WrapTarget; 2]>,
        self_arg_positions: SmallVec<[u8; 4]>,
        type_param_count: u8,
    },
}

pub struct WrapTarget {
    /// Generic-arg-index path into the structural return type.
    /// E.g. for `Result<Self, Error>`, path = `[0]`.
    /// For `HashMap<int, Self>`, path = `[1]`.
    pub path: SmallVec<[u8; 4]>,
    /// Trait to wrap as. Usually `self.trait_id`; for Self::Assoc the bound's trait.
    pub wrap_as_trait_id: u32,
}

bitflags::bitflags! {
    pub struct VTableEntryFlags: u8 {
        const BOXED_RETURN  = 0b0000_0001;
        const SELF_ARG      = 0b0000_0010;
        const GENERIC       = 0b0000_0100;
    }
}
```

###### Q25.C.6 — IC devirtualization

The JIT IC (`feedback.rs:9-128`) at each `dyn T` call site records `(self_vtable_arc_id, g_type_info.concrete_type_id_per_generic)`. State transitions:

- **Uninitialized** → first call records the tuple.
- **Monomorphic** → tuple matches stored. Optimizing tier emits direct call to the impl's method, eliding vtable lookup, auto-boxing on return, SelfArg check, and TypeInfo dispatch. Deopt on mismatch.
- **Polymorphic** (2-4 entries) → first-match dispatch via inlined comparisons.
- **Megamorphic** → fall back to vtable + thunk path.

The cost model degenerates to "zero-cost dyn Trait at hot call sites" — same end state Rust achieves via LLVM devirtualization, but explicit through Shape's IC.

###### Q25.C.7 — LSP cost-class inlay hints

The LSP annotates each `dyn T` call site with its cost class for developer visibility:

| Class | Means |
|---|---|
| `[direct]` | IC stabilized; optimizing tier devirtualizes. Zero overhead. |
| `[vtable]` | Plain dispatch — one indirect call. ~1ns. |
| `[boxed-return]` | Self/Self::Assoc in return; one boxing per call. ~10ns. |
| `[generic-type-info]` | Generic param; TypeInfo lookup. ~5ns per generic arg. |
| `[self-arg-check]` | Self in arg; pointer compare per Self-typed arg. ~1ns. |

Multiple classes combine (`[vtable + boxed-return + generic-type-info]`).

##### Q25.D — Erasure-safe generic types

`Erase_T` recurses through a fixed set of "erasure-safe" generic constructors: `Option`, `Result`, `Vec`/`Array`, `Box`, `Arc`, `HashMap`, `HashSet`, tuples (any arity). User-defined generic types opt in via the `#[erasure_safe]` attribute on the type definition:

```shape
@erasure_safe
type Pair<A, B> { first: A, second: B }
```

Without the attribute, a user-defined `G<Self>` does NOT participate in `Erase_T` recursion — `Erase_T(G<Self>) = G<Self>` (unchanged), which then propagates as a compile error at the trait-object site because `Self` is unbound there.

This is the safety mechanism: erasure-safety is opt-in for user types, automatic for the stdlib.

##### Q25.E — Forbidden patterns (new entries)

The following are added to the codebase's forbidden-pattern list (`docs/check-no-dynamic-baseline.txt`, CLAUDE.md "Forbidden Patterns"):

1. **Resurrection of `TypedArrayData::HeapValue`** as a "polymorphic fallback", "catch-all element buffer", "any-shaped array carrier", or any synonym. Refuse on sight.
2. **`HashMapData::values: Arc<TypedBuffer<Arc<HeapValue>>>`** field shape — replaced by `HashMapValueBuf`; the old shape is forbidden under any alias.
3. **`Box<u64>` data half of trait-object carrier** — kind-blind raw-bits storage, same defection-attractor as the deleted `ValueWord`. The kinded shape is `Arc<TypedObjectStorage>`.
4. **"Object-safety check" emitted as a compile-time rejection** of a trait based on its method shapes — under Q25.C.1 every trait is dyn-able; the rejection path is replaced by the auto-boxing rule + the ETO-001/ETO-002 narrow error cases.
5. **Defection-attractor descriptors** for the deleted polymorphic array carrier: `(array|hashmap|trait.object) (catchall|polymorphic.fallback|any.element|heap.value.element|generic.element) (arm|variant|carrier|buffer)`.

##### Q25.F — Migration cluster scope

This amendment is consumed by Phase 2d sub-cluster **W17-typed-carrier-monomorphization** (merger of the prior W17-array-heap-element-kind + W17-hashmap-typed-buffer + W17-trait-object-rebuild sub-clusters). Estimated 24-32h elapsed; landed as a single coordinated branch because the three changes share carrier shape and dispatch-table updates.

The W17-typed-carrier sub-cluster's gates include:
- `TypedArrayData::HeapValue` arm grep returns zero hits in source trees.
- `HashMapData::values: Arc<TypedBuffer<Arc<HeapValue>>>` grep returns zero hits.
- `HeapKind::TraitObject = 29` arm present in all 4 lockstep dispatch tables (`clone_with_kind` / `drop_with_kind` / `KindedSlot::Clone+Drop` / `TypedObjectStorage::drop`).
- All ~33 SURFACE sites in the three sub-clusters land (full list in `docs/cluster-audits/phase-2d-stub-inventory.md`).
- ADR's `docs/defections.md` gets a fresh entry referencing this section + the W17-typed-carrier close commit.

##### Status

Binding for Phase 2d onward.

#### 2.7.25 Mutex / Atomic / Lazy HeapKinds — concurrency-primitive rebuild trio (Phase 2d Wave 2.5 W17-concurrency, 2026-05-11)

**Question:** the §2.7.20 Channel amendment landed the first concurrency primitive kinded. The W13-out-of-scope list identified three more: `Mutex<T>` (shared-cell-with-exclusion), `Atomic<T>` (atomic load/store/CAS), `Lazy<T>` (initialize-once). All three lost their carriers in the strict-typing Phase-2 deletion of `HeapValue::Concurrency(ConcurrencyData::*)` because every `ConcurrencyData` variant carried `ValueWord`-shaped payload fields. What is the correct typed-Arc shape for each, and what are the runtime semantics at the single-threaded VM landing?

**Decision:** introduce three new `HeapKind` ordinals + their matching `HeapValue` arms in a single coordinated amendment (mirror of the §2.7.20 Channel rebuild structure):

- `HeapKind::Mutex = 30` + `HeapValue::Mutex(Arc<MutexData>)`
- `HeapKind::Atomic = 31` + `HeapValue::Atomic(Arc<AtomicData>)`
- `HeapKind::Lazy = 32` + `HeapValue::Lazy(Arc<LazyData>)`

All three are **full `HeapValue` arms** (NOT pure-discriminator like FilterExpr / SharedCell) — receiver classification at method dispatch flows through `slot.as_heap_value()` per ADR-005 §1 single-discriminator. Same retain/release dispatch shape as the §2.7.20 Channel precedent.

**Storage shape — Mutex.** Like Channel, Mutex needs **interior mutability** so two `Arc<MutexData>` shares of the same mutex observe each other's `set` mutations. The inner state therefore lives behind a `Mutex<MutexInner>`; the outer `Arc` is purely a refcount carrier. At the single-threaded VM landing, `lock()` / `try_lock()` are no-op markers that preserve the user-visible contract ("the inner value is mutated under exclusion") without serializing real concurrency.

```rust
pub struct MutexData {
    inner: std::sync::Mutex<MutexInner>,
}
struct MutexInner {
    value: Option<KindedSlot>,  // wrapped value
}
```

**Storage shape — Atomic.** Wraps `std::sync::atomic::AtomicI64` for the atomic load / store / fetch_add / fetch_sub / compare_exchange operations. **i64-only at landing** per the typed-payload deferral precedent (W15-priority-queue i64-priority-only, W13-hashset string-only). Memory ordering is `SeqCst` throughout — the simplest semantically-correct ordering. A typed-payload `Atomic<T>` and relaxed-ordering optimizations are future Phase-2c amendments with measurement.

```rust
pub struct AtomicData {
    value: std::sync::atomic::AtomicI64,
}
```

**Storage shape — Lazy.** Wraps an initializer closure (`KindedSlot` of kind `Ptr(HeapKind::Closure)`) and a cached value slot. The closure-call path is unlocked by **W17-make-closure** (the Phase 2d Wave 2 partial-gate, merged at `aa47364`); without that closure-call re-entry shape, `Lazy.get()` could not invoke the initializer from a method handler. `LazyData` uses a `Mutex<LazyInner>` for interior mutability so the OnceCell-style "init only happens once" guarantee is preserved when the runtime grows real concurrency. At single-threaded landing the mutex is uncontended.

```rust
pub struct LazyData {
    inner: std::sync::Mutex<LazyInner>,
}
struct LazyInner {
    initializer: Option<KindedSlot>,  // closure; dropped after first get()
    value:       Option<KindedSlot>,  // cached value; populated by first get()
}
```

**Method surface (~11 sites):**

- **Mutex:** `lock()`, `try_lock()`, `set(value)`, `get()` — `get` is the read-accessor for the wrapped value. The playbook smoke target uses `print(m.value)` (property-access form); since GetProp dispatch for `HeapKind::Mutex` is out of scope for W17-concurrency, the `get()` method is the equivalent accessor user code calls.
- **Atomic:** `load()`, `store(v)`, `fetch_add(d)`, `fetch_sub(d)`, `compare_exchange(expected, new)`. Each `fetch_*` returns the prior value; `compare_exchange` returns the prior value regardless of success (callers infer success by comparing to `expected`).
- **Lazy:** `get()` (runs initializer once, caches; closure-call via `vm.call_value_immediate_nb`), `is_initialized()` (bool).

**Construction shape.** Each ctor takes one argument:

- `Mutex(initial_value)` — accepts any `KindedSlot` (the inner value can be any kind; the share moves into the cell).
- `Atomic(initial_int)` — int-only at landing per the i64-only storage shape; non-int args error.
- `Lazy(|| initializer)` — closure-only; kind-validated as `Ptr(HeapKind::Closure)` at the ctor.

**Dispatch tables (mirror of §2.7.20 Channel, lockstep updates for all three new ordinals):**

1. `clone_with_kind` / `drop_with_kind` (`vm_impl/stack.rs`) dispatch the `Mutex` / `Atomic` / `Lazy` arms to `Arc::increment/decrement_strong_count::<MutexData/AtomicData/LazyData>`.
2. `KindedSlot::Drop` / `KindedSlot::Clone` (`shape-value/src/kinded_slot.rs`) mirror the same arms; new `KindedSlot::from_mutex` / `from_atomic` / `from_lazy` constructors.
3. `SharedCell::drop` (`shape-value/src/v2/closure_layout.rs`) mirrors the same arms.
4. `TypedObjectStorage::drop` (`shape-value/src/heap_value.rs`) mirrors the same arms — a TypedObject field of kind `Ptr(HeapKind::Mutex/Atomic/Lazy)` retires one strong-count share.

**Knock-on `kind_type_name` updates** in `arithmetic/mod.rs`, `comparison/mod.rs`, `typed_access.rs`, `printing.rs` — Mutex/Atomic/Lazy display as "mutex"/"atomic"/"lazy" (formatter renders `<mutex>`, `<atomic:N>`, `<lazy:initialized>` / `<lazy:pending>`). PHF classifier in `objects/mod.rs` routes Mutex receivers to `MUTEX_METHODS`, Atomic to `ATOMIC_METHODS`, Lazy to `LAZY_METHODS`.

**Wire / JSON.** Concurrency primitives carry runtime-mutable interior state (Mutex inner value, atomic counter, lazy initializer) and don't have a stable serialized form. `wire_conversion.rs` surfaces as opaque tags (`<mutex:phase-2c>` etc.); `json_value.rs` rejects with "cannot serialize: Mutex/Atomic/Lazy". Same deferral shape as Channel / HashMap / HashSet.

**Refcount discipline.** Storage-tier unit tests (`crates/shape-value/src/heap_value.rs#concurrency_storage`, 18 tests) pin the API contracts and refcount-on-drop invariant:

- `mutex_set_with_heap_payload_retires_shares` — `Mutex.set` drops the prior payload's heap share.
- `mutex_shared_arc_observes_set_mutations` — two `Arc<MutexData>` shares observe each other's mutations.
- `atomic_shared_arc_observes_other_share` — same for Atomic load/store/fetch.
- `lazy_dropping_lazy_with_heap_payload_retires_shares` — dropping the LazyData retires the cached slot's heap share.

**Forbidden alternatives this rules out:**

- **"Re-introduce `ConcurrencyData::{Mutex, Atomic, Lazy}` in the deleted `ValueWord`-encoded shape under a less-suspicious name."** This is the W-series defection-attractor (CLAUDE.md "Renames to refuse on sight"); the kind dispatch must go through the `HeapKind::Mutex` / `Atomic` / `Lazy` arms and the payloads must be typed `Arc<T>`, never `Box<HeapValue>` wrappers or tag-bit-decoded carriers.
- **"Build a generic 'concurrency primitive' wrapper (`HeapValue::Concurrency(ConcurrencyData)`) and dispatch all three primitives through it."** Wrong shape — each primitive has different storage semantics (Mutex carries a `KindedSlot` payload; Atomic carries an atomic-typed scalar; Lazy carries a closure + cached value), and a parent enum would re-introduce the dispatch-on-inner-discriminator pattern §2.3 explicitly forbids.
- **"Inline-scalar Mutex / Atomic carriers."** Wrong shape — the semantic identity of Mutex/Atomic is "this is a shared cell with mutation observable by all holders". Inline scalars cannot share state across `KindedSlot` copies; the typed-Arc shape is structurally required.
- **"Re-use `HeapKind::SharedCell` for Mutex."** Wrong fit — `SharedCell` is the binding-storage interior-mutability carrier for `var` binding-form values (§2.7.12 / Q13), with a pure-discriminator dispatch and no method surface. `MutexData` is a runtime synchronization primitive user code asks for explicitly, with its own method surface and its own refcount discipline.
- **"Skip the inner `Mutex<>` and use `Arc::make_mut` clone-on-write like HashMap / HashSet."** Wrong semantics — the shared-cell contract requires that all `Arc<MutexData>` / `Arc<LazyData>` shares observe each other's mutations. `Arc::make_mut` clones the inner state on the first mutation past refcount=1, breaking the shared-cell contract. Same justification as the §2.7.20 Channel rebuild.

**Out-of-scope this amendment:** typed-payload `Atomic<T>` for non-i64 element kinds (the Phase-2c follow-up with measurement); relaxed memory-ordering variants on Atomic operations (SeqCst-only at landing); GetProp dispatch for `Mutex.value` property-access form (the playbook smoke target's `print(m.value)` shape — currently expressed via the `get()` method); typed sender/receiver / producer-only / consumer-only specializations for Mutex (the way Channel had pre-bulldozer); cross-task await-style `lazy.get()` for async initializers (the §2.7.4 task-scheduler boundary, same as Channel.recv blocking). All five are phase-2c follow-ups tracked separately.

**Status.** Binding for Phase 2d onward.

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

#### 2.7.5.1.A `SerializableVMValue` wire-format extension (Phase 2d Wave 2.6 W17-snapshot-roundtrip, 2026-05-11)

**Question:** the §2.7.5.1 rule pins `FrameDescriptor` as the canonical post-proof wire-format struct and forbids `Option<NativeKind>` wrap / `Unspecialized` placeholders in any `#[derive(Serialize, Deserialize)]` shape that reaches the `FunctionBlob` content hash. W17-snapshot-surfaces (Wave 2.5 close `0db5920`) identified a separate gap: `SerializableVMValue` (`shape-runtime/src/snapshot.rs:296`) — the snapshot wire-format enum — has arms for the pre-bulldozer carriers (`Int`, `Number`, `String`, `Bool`, `Decimal`, `Array`, `TypedObject`, `Range`, `Ok`, `Err`, `Future`, `DataTable`, `HashMap`, `TypedArray`, ...) but no arms for the post-W14/W15/W16/Wave-2.5 HeapKinds added since: `HashSet`, `Iterator`, `Result` (as typed-Arc carrier — `Ok`/`Err` are pre-bulldozer scalar form), `Option` (as typed-Arc carrier — `Some`/`None` are pre-bulldozer scalar form), `Deque`, `Channel`, `PriorityQueue`, `Reference`, `FilterExpr`, `SharedCell`, `Mutex`, `Atomic`, `Lazy`. The §2.7.5.1 forbidden-pattern policy (no Option wrap, no Unspecialized placeholder, no generic Arc<HeapValue> serializer) extends to `SerializableVMValue` — adding the 14 missing arms must follow the same shape as §2.7.5.1 governs for `FrameDescriptor`.

**Decision (Wave 2.6 W17-snapshot-roundtrip):** extend `SerializableVMValue` with 14 new arms — one per missing `HeapKind` — and land the kind-threaded `slot_to_serializable(bits, kind, store)` / `serializable_to_slot(sv, expected_kind, store)` API pair. Each new arm is **post-proof**: the discriminator (variant tag) authoritatively carries the kind, the payload carries per-kind serialized data.

**Arm-by-arm coverage:**

| HeapKind | Wire arm | Coverage shape |
|---|---|---|
| `HashSet` | `HashSet { keys: Vec<String> }` | full payload — string-keyspace storage round-trips verbatim |
| `Iterator` | `IteratorOpaque` | discriminator-only — carries closure-self share + source-buffer refs (§2.7.16 graph) |
| `Result` (typed-Arc) | `ResultData { is_ok, payload: Box<SerializableVMValue> }` | discriminator + inner scalar payload (Int/String/Bool/Number/Unit); deep inner kinds follow-up |
| `Option` (typed-Arc) | `OptionData { is_some, payload: Option<Box<...>> }` | mirror of ResultData |
| `Deque` | `DequeOpaque { len }` | length only — heterogeneous-element `Arc<HeapValue>` payload re-introduces the generic-serializer shape (§2.7.5.1 forbidden) |
| `Channel` | `ChannelOpaque { closed, len }` | closed-flag + length — queue contents follow-up |
| `PriorityQueue` | `PriorityQueueHeap { heap: Vec<i64> }` | full payload — i64-priority-only landing |
| `Reference` | `ReferenceOpaque` | discriminator-only — target identity across snapshot boundaries unspecified at landing |
| `FilterExpr` | `FilterExprOpaque` | discriminator-only — AST tree serializer follow-up |
| `SharedCell` | `SharedCellOpaque` | discriminator-only — binding-identity + per-kind cell payload follow-up |
| `Mutex` | `MutexOpaque { has_value }` | discriminator + has-value flag — inner `Option<KindedSlot>` payload follow-up |
| `Atomic` | `AtomicI64 { value }` | full payload — i64-only landing |
| `Lazy` | `LazyOpaque { is_initialized }` | discriminator + init-flag — inner closure + cached payload follow-up |
| `Char` | `Char(char)` | full payload — `char` serializes via serde |
| `BigInt` | `BigInt(i64)` | full payload — current `Arc<i64>` representation; future typed-payload BigInt updates the wire format |

**Wire-format policy (mirror of §2.7.5.1 for FrameDescriptor):**

- Discriminator is post-proof: every `SerializableVMValue` produced by `slot_to_serializable` carries a definite arm name; no `Option<SerializableVMValue>` wrap (an unknown-kind slot is a `slot_to_serializable` error, not a wrapped-Option success — same shape as §2.7.5.1's `FrameDescriptor.slots` policy).
- No `SerializableVMValue::Unknown` / `Unspecialized` variant — same defection-attractor as `NativeKind::Unknown` (§2.7.5.1 explicit) and `SlotKind::Dynamic` (CLAUDE.md "Forbidden code"). The wire format is bounded by `NativeKind` × `HeapKind` cardinality.
- No `Arc<HeapValue>` generic serializer — heap arms recover the typed `Arc<T>` via the canonical 5-arm receiver-recovery pattern (CLAUDE.md "The 5-arm receiver-recovery soundness rule") and project per-arm. Casting bits to `*const HeapValue` is wrong-type recovery (the bits are `Arc::into_raw(Arc<XData>)`, not `*const HeapValue`).
- No Bool-default fallback at the kind-discriminator-mismatch path. `serializable_to_slot` surfaces a structured error when the discriminator doesn't pair with the expected kind — same rule as §2.7.7 #9 stack-track Bool-default forbid.
- Opaque-stub arms are surface-and-stop on restore. The `XOpaque` arms (Iterator/Deque/Channel/Reference/FilterExpr/SharedCell/Mutex/Lazy) round-trip the discriminator but a `serializable_to_slot` call against them returns a structured `Err(...)` per §2.7.5.1 — not a placeholder slot, not a fabricated Bool-zero. The §2.7.4 invariant — "snapshot reconstruction must not silently corrupt persisted state" — extends to here.

**Kind-threaded API:**

```rust
// shape-runtime/src/snapshot.rs
pub fn slot_to_serializable(
    bits: u64,
    kind: NativeKind,
    store: &SnapshotStore,
) -> Result<SerializableVMValue, String>;

pub fn serializable_to_slot(
    sv: &SerializableVMValue,
    expected_kind: NativeKind,
    store: &SnapshotStore,
) -> Result<(u64, NativeKind), String>;
```

Both functions take/return raw `(u64, NativeKind)` — the §2.7.5 cross-crate ABI policy boundary. The `expected_kind` parameter on `serializable_to_slot` is the post-proof kind the caller has already committed to (from `FrameDescriptor.slots[i]` or the parallel stack-kind track); a discriminator-vs-expected mismatch surfaces a structured error.

**Mechanical effect:** at maximum, `SerializableVMValue` carries one arm per `NativeKind::Ptr(HeapKind::*)` plus one per scalar `NativeKind` family. Total wire-format surface is bounded by HeapKind cardinality (33 ords at HEAD `235256e`), not by user demand. Adding a new HeapKind variant requires extending `SerializableVMValue` in lockstep (the same 4-table lockstep rule §2.7 governs for `HeapKind` extends to the wire-format extension).

**Forbidden shapes this rules out:**

- `SerializableVMValue::HeapValue(Arc<HeapValue>)` — generic carrier that decodes per-arm at runtime. Same defection-attractor as the §2.7.6 / Q8 `from_heap_arc(Arc<HeapValue>)` carrier-API-bound violation; reject on sight.
- `SerializableVMValue::Unknown { kind_tag: u32, bits: u64 }` — Unknown-kind escape hatch. Same defection-attractor as the deleted `SlotKind::Dynamic` / `NativeKind::Unknown`; reject.
- `SerializableVMValue::OpaqueKind { kind: NativeKind, raw_payload: Vec<u8> }` — "we'll deserialize on resume" pattern. Same defection-attractor as the §2.7.5.1 `Option<NativeKind>` wrap forbid; reject.
- `SerializableVMValue::Discriminator(u8)` — bare-tag carrier with no payload at all. The wire format is post-proof; if a payload is missing, the arm shouldn't have been written.
- Bool-default fallback in `serializable_to_slot` when the discriminator doesn't match `expected_kind`. The §2.7.7 #9 stack-track Bool-default forbid extends to here.

**Cross-cluster coordination.** Per-arm coverage expansion lands in follow-up sub-clusters as each `XOpaque` arm's deep payload semantics are nailed down:

- **W17-snapshot-iterator** — Iterator graph walker that traces transform-closure captures and source-buffer refs.
- **W17-snapshot-channel-queue** — Channel queue contents (per-element kinded projection).
- **W17-snapshot-deque** — Deque element-payload kinded projection.
- **W17-snapshot-references** — Reference target identity across snapshot boundaries (entity-id stable handle table).
- **W17-snapshot-filter-expr** — FilterExpr AST tree serializer (mirror of pest's serde-aware AST landing).
- **W17-snapshot-sharedcell** — SharedCell per-kind cell payload + binding-identity table.
- **W17-snapshot-mutex-payload** — Mutex inner KindedSlot deep projection.
- **W17-snapshot-lazy-closure** — Lazy initializer closure + cached value (shares the W17-snapshot-closure follow-up's ClosureLayout reconstruction).
- **W17-snapshot-callstack-upvalues** — non-empty call-stack frames (deep upvalue restoration).
- **W17-snapshot-nullable** — nullable-scalar kind wire-format with explicit sentinel-rule amendment.
- **W17-snapshot-callback-invoker** — `ModuleContext.invoke_callable` / `raw_invoker` hooks for `@ai`-annotation callbacks back into the VM during module-fn dispatch.

Each follow-up extends `SerializableVMValue` in lockstep with its target arm, lands the per-kind serializer / deserializer body, and updates this table.

**Smoke targets at landing (six unit tests in `executor/snapshot.rs::tests`):**

1. `test_w17_vm_snapshot_empty_ok` — empty VM snapshots cleanly.
2. `test_w17_snapshot_roundtrip_scalar_state` — Int / Float / Bool scalar stack round-trip end-to-end.
3. `test_w17_snapshot_result_option_roundtrip` — `Ok(42)`, `Some("hello")`, `None` round-trip end-to-end.
4. `test_w17_snapshot_hashset_roundtrip` — HashSet with string keys round-trips end-to-end.
5. `test_w17_snapshot_resume_incompatible_surfaces_error` — corrupted snapshot (IteratorOpaque on resume) surfaces structured error, not panic.
6. `test_w17_state_bodies_return_structured_errors` — `state.*` bodies still surface as Err per W17-snapshot-surfaces close (pre-Wave-2.6 invariant preserved).

Binding for Phase 2d onward.

#### 2.7.6.A `KindedSlot::from_temporal` / `from_instant` constructor pair (Phase 2d Wave 3 W17-from-temporal-instant-constructors, 2026-05-12)

**Question:** Bundle-A's close (`13d63ed7`) noted that `KindedSlot` lacked per-`FieldType`-style constructors for `NativeKind::Ptr(HeapKind::Temporal)` and `NativeKind::Ptr(HeapKind::Instant)`. Bundle-A migrated the `TypedArrayData::{DateTime, Timespan, Duration}` and `TypedArrayData::Instant` element-readback paths in `array_transform.rs`, `array_aggregation.rs`, `iterator_methods.rs`, and `compiler/comptime.rs` by introducing **local helper mirrors** (`kinded_from_temporal_arc` / `kinded_from_instant_arc`) that inlined the canonical `Arc::into_raw(arc) as u64` + `KindedSlot::new(ValueSlot::from_raw(bits), NativeKind::Ptr(HeapKind::X))` shape. The mirrors were noted as "kept local so the comptime layer doesn't pull in executor-tier visibility" and explicitly flagged as a §2.7.6 / Q8 cardinality amendment for separate work.

**Decision (Wave 3 W17-from-temporal-instant-constructors):** add the two missing constructors to `KindedSlot` and delete the four local helper mirrors. The new pair is:

```rust
// crates/shape-value/src/kinded_slot.rs
impl KindedSlot {
    #[inline]
    pub fn from_temporal(arc: Arc<crate::heap_value::TemporalData>) -> Self {
        let bits = Arc::into_raw(arc) as u64;
        Self::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Temporal),
        )
    }

    #[inline]
    pub fn from_instant(arc: Arc<std::time::Instant>) -> Self {
        let bits = Arc::into_raw(arc) as u64;
        Self::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Instant),
        )
    }
}
```

**Carrier-API-bound fit (§2.7.6 / Q8).** This pair sits squarely inside the §2.7.6 bound: one constructor per existing `NativeKind::Ptr(HeapKind::X)` heap variant. The amendment **does not introduce a new `HeapKind` variant** — `HeapKind::Temporal` and `HeapKind::Instant` already exist; they already have arms in the 4-table lockstep dispatch (`clone_with_kind` / `drop_with_kind` in `vm_impl/stack.rs`, `Drop` / `Clone` in `kinded_slot.rs`, `SharedCell::drop` in `v2/closure_layout.rs`, `TypedObjectStorage::drop` in `heap_value.rs`); and `TypedArrayData::{DateTime, Timespan, Duration, Instant}` already store the matching typed-Arc payloads (`Arc<TemporalData>` / `Arc<std::time::Instant>`). The constructors are the **missing matching half** of an already-complete §2.7.6 dispatch surface, not a cardinality expansion. No new heap-variant accessor is added (the §2.7.6 forbidden-shape rule against per-heap-variant `as_X()` accessors is preserved — receivers still recover the typed Arc via the 5-arm receiver-recovery pattern when needed, not via a `KindedSlot::as_temporal()`).

**Storage-shape parity with siblings.** Both constructors follow the typed-Arc-via-`from_raw` shape (`KindedSlot::new(ValueSlot::from_raw(Arc::into_raw(arc) as u64), NativeKind::Ptr(HeapKind::X))`), matching the existing `from_iterator` (§2.7.16 / Q17), `from_range` (§2.7.23 / Q24), `from_result` / `from_option` (§2.7.17 / Q18) precedents where `ValueSlot` does not carry a sibling `from_temporal` / `from_instant` typed-Arc constructor and the `KindedSlot` constructor builds the slot bits directly. This is identical to the `from_iterator` shape — `ValueSlot::from_raw(Arc::into_raw(it) as u64)` inside the `KindedSlot::from_iterator` body.

**Sibling-of-Temporal note.** `TemporalData` is the consolidated DateTime / Duration / TimeSpan / Timeframe / TimeReference / DateTimeExpr / DataDateTimeRef carrier (`crates/shape-value/src/heap_value.rs:3492`). `TypedArrayData::{DateTime, Timespan, Duration}` all carry `Arc<TemporalData>` element payloads; all three feed into the single `from_temporal` constructor at the element-readback path (the three TypedArrayData arms dispatch on storage variant, not on TemporalData variant — that's a downstream `TemporalData::*` body match, not a constructor-side cardinality concern).

**Migration sites (four local helper mirrors deleted):**

1. `crates/shape-vm/src/executor/objects/array_transform.rs::kinded_from_temporal_arc` / `kinded_from_instant_arc` — deleted; four call sites (`DateTime` / `Timespan` / `Duration` / `Instant` arms in `element_kinded`) now call `KindedSlot::from_temporal` / `KindedSlot::from_instant` directly.
2. `crates/shape-vm/src/executor/objects/array_aggregation.rs::kinded_from_temporal_arc` / `kinded_from_instant_arc` — deleted; four call sites in the per-element `element_kinded` body migrated.
3. `crates/shape-vm/src/executor/objects/iterator_methods.rs::kinded_from_temporal_arc` / `kinded_from_instant_arc` — deleted; four call sites in the iterator element-readback body migrated.
4. `crates/shape-vm/src/compiler/comptime.rs::typed_array_element_kinded` — the `DateTime | Timespan | Duration` and `Instant` arms used **inline** `KindedSlot::new(ValueSlot::from_raw(bits), NativeKind::Ptr(HeapKind::X))` construction (not a named helper); both migrated to call `KindedSlot::from_temporal` / `KindedSlot::from_instant` directly, collapsing the inline `Arc::into_raw` + slot-build dance into the named constructor.

**Refcount discipline preserved.** Each call site previously built the slot bits by `Arc::into_raw(Arc::clone(&buf.data[idx]))`. The new constructors take ownership of the cloned `Arc<T>` and do the `Arc::into_raw` internally; the strong-count semantics are identical (one share owned by the resulting `KindedSlot`, paired with one Drop / Clone arm retire/bump under the existing 4-table lockstep). Storage-tier unit tests added in `kinded_slot.rs::tests` (`kinded_slot_from_temporal_sets_kind_and_retires_arc`, `kinded_slot_from_temporal_clone_then_double_drop_balances`, `kinded_slot_from_instant_sets_kind_and_retires_arc`, `kinded_slot_from_instant_clone_then_double_drop_balances`) pin the refcount-on-drop and clone-then-double-drop invariants for both kinds.

**Forbidden shapes this rules out:**

- **Per-heap-variant `KindedSlot::as_temporal()` / `KindedSlot::as_instant()` accessors.** The §2.7.6 / Q8 forbidden-shape rule against per-heap-variant accessors stands — heap dispatch goes through `slot.slot.as_heap_value()` + `HeapValue` match (for `Box<HeapValue>` arms) or the 5-arm receiver-recovery pattern (for typed-Arc arms like Temporal / Instant). Adding only the constructor half preserves the carrier-API bound.
- **`from_heap_arc(Arc<HeapValue>)` catch-all.** Q6 ruling reaffirmed. The constructors take typed `Arc<TemporalData>` / `Arc<std::time::Instant>`, never `Arc<HeapValue>`.
- **Local helper-mirror retention "for separation of concerns".** Bundle-A's noted "kept local so the comptime layer doesn't pull in executor-tier visibility" rationale dissolves once the constructor lives on `KindedSlot` directly — `shape-value` is already a dependency of both `shape-vm/executor/` and `shape-vm/compiler/`, so the constructor has lower visibility burden than the local helpers it replaces (zero versus three duplicated function definitions).

**Out-of-scope this amendment:** any further `KindedSlot` constructor for heap variants whose slot-bits shape is not `Arc::into_raw(Arc<T>) as u64` — those are §2.7.6 forbidden by the carrier-API bound and require a separate ADR amendment if a new heap variant is ever introduced with a different storage shape; deletion of `HeapKind::Temporal` / `HeapKind::Instant` as discriminator labels (they remain, with all 4-table lockstep arms intact); migration of `executor/stack_ops/mod.rs:153`'s `op_push_const` Temporal / Instant paths to the new constructors (those use the same inline `Arc::into_raw` + `KindedSlot::new` shape; constructor migration is a mechanical follow-up but not in this sub-cluster's territory because the bundle-A close did not name `stack_ops` as one of the helper-mirror sites).

Binding for Phase 2d onward.

#### 2.7.27 Method-mutation semantics — `&mut self` opt-in with compile-time Arc-COW write-back (Item 4 ruling, W17-mutation-writeback, 2026-05-12)

Phase 2d Item 4 (handover §1 Item 4) surfaced that `let s = HashSet();
s.add("a"); s.size()` returns 0 — the post-W16 method-call dispatch is
functional-shaped (handlers return a new `Arc<HashSetData>` after
`Arc::make_mut`, but the dispatch shell pops the receiver and pushes
the result; with no consumer, the new Arc is dropped and the binding
slot retains the old Arc). The user ruling adopts Rust-style `&mut self`
opt-in: mutating handlers stay in the existing `MethodFnV2` ABI
(`fn(&mut VM, &[KindedSlot], ctx) -> Result<KindedSlot, VMError>`), but
the compiler now emits a `Dup; StoreLocal recv` / `Dup;
StoreModuleBinding recv` write-back after the standard `CallMethod`
opcode when the call site matches a `&mut self` opt-in pattern.

**Decision (Item 4 ruling):**

1. **Mutation semantics — `&mut self` opt-in with write-back at the
   call site.** Methods that mutate the receiver Arc (via
   `Arc::make_mut` and returning a possibly-new Arc) opt in by
   registration name in the per-receiver-kind
   `MUT_SELF_*` `phf::Set` exposed from
   `crates/shape-vm/src/executor/objects/method_registry.rs`. The
   compiler reads these sets via
   `crate::compiler::mutation_writeback::ContainerKind::is_mut_self_method`
   to decide whether to emit a post-`CallMethod` write-back at the
   identifier-receiver method-call site:

   ```rust
   // §2.7.27 emit shape, post-CallMethod, for mut-self methods on
   // identifier receivers tracked as recognised COW container kinds.
   self.emit(Instruction::new(OpCode::CallMethod, Some(operand)));
   if let Some(target) = mut_self_writeback_target {
       self.emit(Instruction::simple(OpCode::Dup));
       match target {
           MutSelfWriteBackTarget::Local(idx) =>
               self.emit(Instruction::new(OpCode::StoreLocal,
                   Some(Operand::Local(idx)))),
           MutSelfWriteBackTarget::ModuleBinding(idx) =>
               self.emit(Instruction::new(OpCode::StoreModuleBinding,
                   Some(Operand::ModuleBinding(idx)))),
       }
   }
   ```

   `Dup` bumps the heap refcount via `clone_with_kind` (§2.7.7 WB2.4
   retain-on-read), so the binding slot and the method-call expression
   result each own an independent share of the new Arc. `StoreLocal` /
   `StoreModuleBinding` pops one share, releases the old slot
   occupant's share via `drop_with_kind` (existing
   `stack_write_kinded` invariant), and stores the new bits. The
   result share stays on the stack for the expression value.

2. **R-value receiver rule.** `compute_set().add(x)` — receiver is not
   an `Identifier(name, _)`; `resolve_mut_self_writeback_target`
   returns `None`; no write-back is emitted. The new (mutated) Arc is
   the expression value of the call; if the statement has no consumer
   (`compute_set().add(x);`), it drops on statement end. This matches
   the dispatch text's decision-call: silent drop, not an error,
   because erroring would break composition patterns.

3. **`let` (immutable binding) check.** When the compiler resolves a
   write-back target, it invokes the existing
   `check_named_binding_write_allowed` to enforce that the receiver
   binding is `let mut` / `var`. `let s = HashSet(); s.add("x")` fails
   to compile with the diagnostic "Cannot reassign immutable variable
   's'. Use `let mut` or `var` for mutable bindings", flowing through
   the same path as direct reassignment.

4. **Receiver-kind narrowing — `ContainerKind` tracking.** The
   compiler maintains
   `mut_self_container_locals: HashMap<u16, ContainerKind>` and
   `mut_self_container_bindings: HashMap<u16, ContainerKind>`
   populated at let-binding time when the initializer is a recognised
   container constructor (`Set()` / `HashMap()` / `Deque()` /
   `PriorityQueue()`). The constructor-call emit path (in
   `compile_expr_function_call`) sets
   `pending_variable_container_kind` after emitting `BuiltinCall(...
   Ctor)`; the let-binding completion in
   `compile_statement(Statement::Var)` transfers the pending kind onto
   the target local-slot or module-binding index. Method-call
   dispatch's `resolve_mut_self_writeback_target` consults this side
   table to narrow `add` / `set` / `delete` — names that overlap
   between mutating containers (HashSet / HashMap) and pure-functional
   types (`DateTime.add` is the operator-trait backing for `+`,
   `Mutex.set` is interior mutability). Without a recognised container
   kind, the writeback path is not taken and the call falls through
   to the standard `CallMethod` shape (the pre-ruling functional
   behaviour).

5. **Interior-mutability primitives — explicitly NOT registered.**
   `Mutex.set`, `Atomic.store` / `fetch_add` / etc., `Lazy.get`,
   `Channel.send` / `close` mutate through `Cell` / `AtomicI64` /
   `OnceCell` / channel-buffer; the receiver Arc's identity is
   preserved, no write-back is needed, and `let m = Mutex(0);
   m.set(5)` stays valid (the binding itself is immutable; what
   changes is the shared interior — mirrors Rust's `let m =
   Mutex::new(0); *m.lock().unwrap() = 5;` shape). These primitives
   do not register a `ContainerKind` in
   `mut_self_container_locals`; their mutating method names
   (`set`, `store`, `send`, ...) are NOT in any `MUT_SELF_*` set.

6. **`pop`-shaped methods — tuple-return ABI variant.**
   `Array.pop`, `Deque.popBack` / `popFront`, `PriorityQueue.pop`,
   `HashMap.remove` extract an element AND mutate the container's
   structure. They opt into the **tuple-return ABI variant**
   (W17-pop-mutation amendment, 2026-05-12, below in this §2.7.27).
   Conceptual dispatch signature `(&mut self) -> (Option<T>, Self)`:
   the user-facing return is the popped element (`Option<T>`); the
   new container Arc is published as a side effect on the VM stack
   and the compiler emits a post-`CallMethod`
   `Swap; Store*(receiver)` to write it back to the binding slot
   (r-value receivers get `Swap; Pop` for silent drop, mirroring the
   §2.7.27 r-value silent-drop rule). The runtime `MethodFnV2` ABI is
   unchanged — the convention is that tuple-return handlers
   `vm.push_kinded` the new container before returning the popped
   element. See the tuple-return amendment below for the binding
   spec.

7. **Primitive operator sugar — `s += x` already covered by existing
   compound-assignment lowering.** The parser (`shape-ast/src/parser/expressions/binary_ops.rs::parse_assignment_impl`)
   desugars `s += x` into `Expr::Assign { target: s, value: s + x }` at
   parse time. The compound-assignment lowering already emits
   `LoadLocal s; <load x>; AddInt; StoreLocal s` for primitive
   receivers — typed opcode (`AddInt` / `AddNumber` / `SubInt` /
   etc.) + write-back to s's slot. The non-assignment form `s + x`
   produces a value without write-back (`s` stays unchanged). This
   sub-cluster's ruling does NOT change operator lowering; the
   desugaring path was already correct. Coverage: `i8 / i16 / i32 /
   i64 / u8 / u16 / u32 / u64 / number / string` for `+=` (string is
   concatenation); same set minus `string` for `-= / *= / /= / %=`
   (string subtraction is undefined).

8. **Informal naming convention — documented, not tool-enforced.**
   Mutating methods take simple names (`add`, `push`, `set`,
   `pushBack`); functional siblings, if needed, take participial names
   (`added`, `pushed`, ...). No LSP / clippy / verify-merge.sh hook
   enforces this — adopting the convention is on the stdlib /
   user-code author. The LSP "naming convention warning" hook
   originally listed in the dispatch text is deferred to the
   hardening backlog (phase-2d-hardening:(j) when added).

**Forbidden shapes this rules out (mirror of §2.7.7 / §2.7.10
forbidden lists, applied to method mutation semantics):**

- **Silent mutation without write-back.** A handler returns a new Arc
  via `Arc::make_mut` but the binding slot doesn't update. This is
  the footgun the ruling fixes. Any `MUT_SELF_*`-registered method
  whose dispatch path doesn't write back is a §2.7.27 defection.

- **Reintroducing runtime numeric coercion** to dodge the lossless-
  widening lattice (`IntToNumber` / `NumberToInt` opcodes added "for
  one assignment"). CLAUDE.md "Forbidden Patterns" #5 (`Convert<X>To<Y>`
  opcodes papered over a kind-tracker gap) stands verbatim. The
  Commit-2 widening lattice is compile-time only; the smaller type's
  value is reinterpreted into the wider slot at compile-time-resolved
  binding/call sites (typed slots already carry `NativeKind` so the
  compiler can emit the target slot directly).

- **Implicit narrowing without explicit `as T`.** `let n: i8 =
  some_i32_value` without `as i8` must be a compile error. The
  typer's existing unify pass plus the §2.7.27 widening lattice
  (Commit 2) cover this — narrowing requires explicit `as T`.

- **Defection-attractor descriptors.** "MethodMut bridge",
  "writeback hop", "Arc-mutation translator", "let-mut adapter".
  Per the 2026-05-09 user ruling broadening the W-series rename
  family, any descriptor of the writeback emission that uses bridge /
  probe / helper / hop / translator / adapter framing belongs to the
  defection-attractor family CLAUDE.md "Renames to refuse on sight"
  enumerates. Describe the writeback emission by name (the §2.7.27
  `Dup; StoreLocal` emission) or by mechanism (compile-time
  write-back at the identifier-receiver site), never by hypothetical
  role.

**Out-of-scope this amendment:**

- **Trait-based operator overloading for user types** (e.g. `impl Add
  for Vec3` so `v1 + v2` calls `v1.add(v2)`). Phase 3 work; not in
  scope for Item 4.
- **Tuple-return ABI for `pop`-shaped methods.** Pop methods return
  the popped element; a future amendment can add a mutate-and-return-
  X handler shape so write-back fires on the receiver while the
  element is the call's expression value.
- **Runtime-driven write-back via opcode operand extension.** The
  compile-time approach in this amendment is sufficient for all
  Identifier-receiver call sites. A future hardening pass could move
  the write-back into the `op_call_method` dispatch shell via an
  operand extension on `TypedMethodCall`; the `mutation_writeback`
  module ships a `WriteBackTarget` / `writeback_result` stub for
  that future surface but neither is wired in this ruling.
- **LSP naming-convention warning hook.** Deferred to
  phase-2d-hardening backlog (the dispatch text's deferral).
- **Width-narrowing operator sugar** (`let n: i32 = 100; n += 1i64;`
  — would require explicit narrowing of the i64 rhs; Commit 2's
  lossless-widening lattice covers the opposite direction and rejects
  narrowing).

**Widening lattice (W17-mutation-writeback Commit 2 deliverable):**

The Item 4 ruling also specifies a **lossless numeric widening lattice**
for narrow integers, hardening what `unify_annotations` /
`can_numeric_widen` already partially implement. The lattice:

- `i8 → i16 → i32 → i64` (signed sign-extension).
- `u8 → u16 → u32 → u64` (unsigned zero-extension).
- Safe signed↔unsigned crossings only when the source range is fully
  representable in the target. `u8 → i16` is safe (0..255 ⊂ -32768..32767);
  `i8 → u16` is unsafe (negative i8 ≠ representable u16); `u32 → i64`
  is safe (0..2^32-1 ⊂ -2^63..2^63-1).
- **No widening across the int/number boundary.** `int → number` is
  lossy beyond 2^53 (f64 mantissa); `number → int` is always lossy
  (rounding). Existing arithmetic-result inference (`5 * 2.0 → number`)
  is a separate inference, not part of this lattice — the lattice
  governs *assignment-side widening*, not arithmetic-result type
  promotion.
- **Narrowing requires explicit `as T`.** `let n: i8 =
  some_i32_value` without `as i8` is a compile error. The typer's
  existing unify pass plus the widening lattice cover this.
- **Compile-time only.** Widening produces no runtime opcode; the
  narrower type's bits are reinterpreted into the wider slot at
  compile-time-resolved binding / call sites. Typed slots already
  carry `NativeKind` in the §2.7.7 parallel-kind track, so the
  compiler emits the target slot directly (no `IntToNumber` /
  `NumberToInt` coercion opcodes — CLAUDE.md "Forbidden Patterns" #5
  stands verbatim).

Concretely, the existing `can_numeric_widen` in
`crates/shape-runtime/src/type_system/constraints.rs:298` covers the
integer-family ↔ integer-family lattice (any integer name to any
wider integer name); the typed-opcode lowering in
`crates/shape-vm/src/compiler/expressions/binary_ops.rs` already emits
the target-typed opcode (`AddI32` / `AddI64` / etc.) when the
producing context proves a widening source. The Commit-2 deliverable
is the **verification scope**: the smoke target
`let mut n: i32 = 0; let x: i8 = 5; n += x` lowers to `LoadLocal n;
LoadLocal x; AddI32; StoreLocal n` (i8 → i32 widening, typed AddI32
opcode, write-back to n's i32 slot) and prints `5`. Tests in
`mutation_writeback.rs::widening_*` pin the lattice across (i8→i32 via
compound assign), (i16→i64 in binding), (u8→u16 in binding), and
(u8→u32 in binding).

**Per-method-handler sweep (W17-mutation-writeback Commit 2 deliverable):**

Audit confirmed (`grep Arc::make_mut crates/shape-vm/src/executor/objects/`):

- **HashSet** (`set_methods.rs`): `v2_add` (line 235), `v2_delete`
  (line 263). Both return `KindedSlot::from_hashset(hs)` after
  `Arc::make_mut(&mut hs).{insert,remove}`. **Registered** in
  `MUT_SELF_HASHSET_METHODS`.
- **HashMap** (`hashmap_methods.rs`): `v2_set` (line 411), `v2_delete`
  (line 445), `v2_merge` (line 467). All return
  `KindedSlot::from_hashmap(hm)` after `Arc::make_mut`. **Registered**
  in `MUT_SELF_HASHMAP_METHODS`.
- **Deque** (`deque_methods.rs`): `v2_push_back` (line 281),
  `v2_push_front` (line 300) return
  `KindedSlot::from_deque(dq)` — **registered** in
  `MUT_SELF_DEQUE_METHODS`. `v2_pop_back` (line 321), `v2_pop_front`
  (line 340) return the popped element via `heap_value_arc_to_slot` —
  **NOT registered** (pop-shape deferral).
- **PriorityQueue** (`priority_queue_methods.rs`): `v2_push` (line
  208) returns `KindedSlot::from_priority_queue(owned)` —
  **registered** in `MUT_SELF_PRIORITY_QUEUE_METHODS`. `v2_pop` (line
  227) returns the popped i64 — **NOT registered** (pop-shape
  deferral).
- **Array** (`array_basic.rs`): `handle_push_v2` (line 288) returns
  `KindedSlot::from_typed_array(arc)` — **registered** in
  `MUT_SELF_ARRAY_METHODS`. `handle_pop_v2` (line 378) returns the
  popped element — **NOT registered**. `handle_reverse_v2` (line
  239) returns a **fresh** Arc per the documented pre-Wave-6.5
  "produces a fresh array" contract — **NOT registered** (functional
  semantics preserved, matches `arr.sort()`).
- **Mutex / Atomic / Lazy / Channel** (`concurrency_methods.rs` /
  `channel_methods.rs`): all use interior mutability (`Cell` /
  `AtomicI64` / `OnceCell` / channel-buffer); the receiver Arc's
  identity is preserved. **NOT registered** — `let m = Mutex(0);
  m.set(5)` stays valid on `let` (immutable) bindings.
- **TypedArray<i64> / TypedArray<f64>** (`typed_int_array_methods.rs`
  / `typed_number_array_methods.rs`): `push`, `set` mutate the
  underlying TypedBuffer via Arc::make_mut and return the (mutated)
  array — **registered** in `MUT_SELF_TYPED_ARRAY_METHODS`. `pop`
  returns the popped element — **NOT registered**. (Note: these go
  through the existing `ArrayPushLocal` / `TypedArrayPush*` fast
  paths for identifier receivers, which have their own
  compile-time write-back via the operand-encoded local-slot index;
  the `MUT_SELF_TYPED_ARRAY_METHODS` set covers the generic-dispatch
  fallback when the bespoke path isn't taken.)
- **DataTable** (`datatable_methods/*`): `groupBy`, `aggregate`,
  `filter`, `orderBy`, etc. all return new DataTables (builder
  pattern). **NOT registered** — functional semantics, no write-back
  needed.

The sweep confirms `MUT_SELF_*` sets are correct and complete for the
methods that return the mutated receiver. Pop-shape methods are the
only mutating methods left out; a future tuple-return ABI amendment
(out of scope this ruling) will pick them up.

**Migration scope (W17-mutation-writeback Commit 1 deliverable):**

1. `crates/shape-vm/src/executor/objects/method_registry.rs` — per-
   receiver-kind `MUT_SELF_*` `phf::Set` constants
   (`MUT_SELF_HASHSET_METHODS` / `MUT_SELF_HASHMAP_METHODS` /
   `MUT_SELF_ARRAY_METHODS` / `MUT_SELF_DEQUE_METHODS` /
   `MUT_SELF_PRIORITY_QUEUE_METHODS` /
   `MUT_SELF_TYPED_ARRAY_METHODS`) — registered names of
   `&mut self` opt-in methods that return the (mutated) receiver
   Arc.
2. `crates/shape-vm/src/compiler/mutation_writeback.rs` (new module) —
   `ContainerKind` enum (HashSet / HashMap / Deque / PriorityQueue /
   Array), `is_mut_self_method(method)` classifier,
   `from_ctor_name(name)` recogniser, `MutSelfWriteBackTarget` enum
   (Local / ModuleBinding).
3. `crates/shape-vm/src/compiler/mod.rs` —
   `mut_self_container_locals` / `mut_self_container_bindings`
   side tables on `BytecodeCompiler` + the
   `pending_variable_container_kind` signal mirror of
   `pending_variable_typed_array_kind`.
4. `crates/shape-vm/src/compiler/expressions/function_calls.rs` —
   `resolve_mut_self_writeback_target(receiver, method)` helper +
   the post-`CallMethod` `Dup; Store{Local,ModuleBinding}` emit gate +
   the let-immutability guard (calls existing
   `check_named_binding_write_allowed`).
5. `crates/shape-vm/src/compiler/expressions/function_calls.rs` /
   `statements.rs` — the existing bespoke `ArrayPushLocal` fast paths
   for `arr.push(x)` (call site + standalone statement site) are
   gated to NOT fire when the receiver is a recognised non-Array
   container kind; those fall through to the standard `CallMethod`
   path which then receives the §2.7.27 write-back emission.
6. Smoke tests in `crates/shape-vm/src/executor/tests/mutation_writeback.rs`
   — 23 tests covering HashSet / HashMap / Deque / PriorityQueue
   write-back, let-immutability compile errors, r-value receiver
   silent-drop, compound-assignment for primitives, and
   interior-mutability primitives staying on `let` bindings.

**Tuple-return ABI variant (W17-pop-mutation amendment, 2026-05-12):**

Pop-shaped methods extract an element from a collection AND mutate
the collection's structure. The conceptual dispatch signature is
`(&mut self) -> (Option<T>, Self)`: the user-facing call return is
the popped element (`Option<T>`); the `Self` slot is invisible to
user code and consumed by compile-time codegen to write the new
container Arc back to the receiver binding. The runtime
`MethodFnV2` ABI is unchanged — handlers still return a single
`Result<KindedSlot, VMError>`. The convention is that tuple-return
handlers `vm.push_kinded(new_container_bits, kind)` before
returning the popped element; the dispatch shell pushes the
returned popped element on top, so the post-call stack is
`[..., NewContainer, popped_element]`. The compiler emits the
post-`CallMethod` opcode pair that consumes `NewContainer` per the
receiver-rooting rule.

```rust
// §2.7.27 amendment emit shape, post-CallMethod, for tuple-return
// pop-shape methods on identifier receivers tracked as recognised
// COW container kinds.
self.emit(Instruction::new(OpCode::CallMethod, Some(operand)));
if let Some(target) = mut_self_tuple_return_target {
    self.emit(Instruction::simple(OpCode::Swap));
    match target {
        MutSelfWriteBackTarget::Local(idx) =>
            self.emit(Instruction::new(OpCode::StoreLocal,
                Some(Operand::Local(idx)))),
        MutSelfWriteBackTarget::ModuleBinding(idx) =>
            self.emit(Instruction::new(OpCode::StoreModuleBinding,
                Some(Operand::ModuleBinding(idx)))),
    }
} else if is_rvalue_tuple_return {
    self.emit(Instruction::simple(OpCode::Swap));
    self.emit(Instruction::simple(OpCode::Pop));
}
```

`Swap` flips the top two slots into
`[..., popped_element, NewContainer]`. `Store*(target)` then pops
`NewContainer` and writes it to the receiver binding (the existing
`stack_write_kinded` invariant releases the prior occupant's share
via `drop_with_kind`); `popped_element` remains as the call's
expression value. The r-value variant `Swap; Pop` drops
`NewContainer` cleanly (the `Pop` opcode's `drop_with_kind`
discipline releases the heap share) and leaves `popped_element` on
top — mirror of the §2.7.27 self-returning r-value silent-drop
rule.

**Canonical-shape rule — binding scope of the tuple-return ABI variant.**
Tuple-return is used ONLY for methods that satisfy BOTH:

- **(a) canonical-extraction-from-collection** — the call returns an
  element drawn from the container's payload (a popped element, a
  removed value-at-key, the minimum-priority entry from a heap, etc.);
- **(b) structural-mutation-of-collection** — the call mutates the
  container's spine (decrements the length, removes an entry, drains
  a slot, etc.), and the post-mutation receiver is observably
  different from the pre-mutation receiver in length / membership.

Methods that satisfy only one half do NOT use the tuple-return ABI.
Forbidden examples (refused on sight):

- **`find` / `findIndex`** — extraction without structural mutation.
  Use the self-return ABI variant (or stay as `&self` if non-mutating).
- **`entry_or_default`** — returns a reference-into-collection. Wrong
  shape for tuple-return; needs its own ABI (deferred to a future
  amendment, not this one).
- **`peek_first` / `peek_back` / `peekFront`** — extraction without
  mutation. Stay `&self`.
- **`iter().next()` and iteration cursor advancement** — mutates the
  cursor state, not the container's spine. Out of scope; iterators
  have their own §2.7.16 dispatch shape.
- **`HashSet.remove(x) -> bool`** — returns "was present" rather than
  the removed element. Self-return ABI (extraction half not
  satisfied — the result isn't a payload element).
- **`HashMap.delete(k) -> HashMap`** — returns the (new) map for
  chaining; consumed by `stdlib-src/core/set.shape::remove` which
  wraps it. Stays self-return; the canonical pop-shape sibling is
  the new `HashMap.remove(k)` method (which returns `Option<V>`)
  introduced in this amendment.

**Composition with the existing self-returning `&mut self` pattern.**
Self-return (§2.7.27 base) and tuple-return (this amendment) share
the receiver-rooting machinery
(`mut_self_container_locals` / `mut_self_container_bindings`,
`resolve_mut_self_writeback_target` / `resolve_mut_self_tuple_return_target`)
and the let-vs-let-mut immutability guard
(`check_named_binding_write_allowed`). The difference is purely in
the post-`CallMethod` emit shape: `Dup; Store*` (self-return) vs
`Swap; Store*` / `Swap; Pop` (tuple-return). A method is registered
in at most one ABI partition — the registries are mutually
exclusive at the `method_registry` level.

**Per-method-handler sweep (W17-pop-mutation amendment):**

The pop-shape audit identifies the following five canonical methods.
Each has its handler updated to side-channel-publish the new
container Arc before returning the popped element, and is registered
in the matching `MUT_SELF_TUPLE_RETURN_*` `phf::Set`:

| Method | Receiver kind | Handler | Returns | Tuple-return set |
|---|---|---|---|---|
| `Array.pop` | `Ptr(TypedArray)` | `array_basic::handle_pop_v2` | `KindedSlot` (popped element) | `MUT_SELF_TUPLE_RETURN_ARRAY_METHODS` |
| `Deque.popBack` | `Ptr(Deque)` | `deque_methods::v2_pop_back` | `KindedSlot` (popped element, or `KindedSlot::none()` if empty) | `MUT_SELF_TUPLE_RETURN_DEQUE_METHODS` |
| `Deque.popFront` | `Ptr(Deque)` | `deque_methods::v2_pop_front` | `KindedSlot` (popped element, or `KindedSlot::none()` if empty) | `MUT_SELF_TUPLE_RETURN_DEQUE_METHODS` |
| `PriorityQueue.pop` | `Ptr(PriorityQueue)` | `priority_queue_methods::v2_pop` | `KindedSlot::from_int(min)` (`0` if empty per landing) | `MUT_SELF_TUPLE_RETURN_PRIORITY_QUEUE_METHODS` |
| `HashMap.remove(k)` | `Ptr(HashMap)` | `hashmap_methods::v2_remove` (new) | `KindedSlot` (popped value, or `KindedSlot::none()` for missing key) | `MUT_SELF_TUPLE_RETURN_HASHMAP_METHODS` |

`TypedArray<i64>` / `TypedArray<f64>` v2 fast-path `pop`
(`typed_int_array_methods::pop` / `typed_number_array_methods::pop`)
stays in-place on the raw `*mut TypedArray<T>` pointer — there is
no `Arc<T>` identity to write back, the underlying buffer mutation
is visible through the binding's stable pointer. The tuple-return
ABI does not apply to the v2 fast path.

Audit-and-include disposition for the additional candidate list in
the dispatch text (`Array.remove(i)` / `Array.swap_remove(i)` /
`HashSet.take(x)` / `HashSet.pop_first` / `Channel.recv`): none of
these exist in the current stdlib. Queued as natural follow-up
sites for a future amendment if they're introduced; they all match
the canonical-shape rule and would slot into the same ABI variant
with no further machinery.

**Forbidden shapes the tuple-return amendment rules out (in
addition to the §2.7.27 base list):**

- **Bool-default for the tuple-return ABI variant.** If a kind
  source is genuinely missing, surface-and-stop with a §2.7.27
  cite; never silently leak a `Ptr(Bool)` slot as the new container.
- **Reintroducing the runtime dispatch-shell write-back path** that
  W17-mutation-writeback specifically avoided. The tuple-return
  amendment extends the compile-time codegen, not the runtime ABI.
  The new `MUT_SELF_TUPLE_RETURN_*` flag is a compile-time marker
  only; the runtime `MethodFnV2` ABI is unchanged.
- **Tuple-return for non-canonical pop-shape methods.** A method
  not satisfying BOTH (a) and (b) of the canonical-shape rule
  belongs in the self-return ABI variant, in the iterator §2.7.16
  shape, or in `&self`. Adding a tuple-return entry for `find` /
  `entry_or_default` / `peek_first` / iteration cursors is a
  §2.7.27-amendment defection.
- **Silent fallback to peek-like behaviour** if the receiver root
  can't be identified. R-value receivers DROP both slots
  (`Swap; Pop`); the popped element stays as the expression value,
  the new container is released cleanly via `drop_with_kind`. No
  fabrication, no kind-blind probe on the receiver bits.
- **Adding new HeapKind ordinals or runtime variants** to support
  the tuple-return path. None are required; the amendment is pure
  codegen + handler convention.
- **Defection-attractor descriptors** for the tuple-return emission:
  "MethodTuple bridge", "popback hop", "container-publish translator",
  "tuple-result adapter". Per the 2026-05-09 user ruling broadening
  the W-series rename family, any descriptor of the tuple-return
  emission that uses bridge / probe / helper / hop / translator /
  adapter framing belongs to the defection-attractor family
  CLAUDE.md "Renames to refuse on sight" enumerates. Describe the
  emission by name (the §2.7.27 amendment `Swap; Store*` /
  `Swap; Pop` emission) or by mechanism (compile-time write-back
  of the side-channel-published new container), never by hypothetical
  role.

**Migration scope (W17-pop-mutation amendment deliverables):**

1. `crates/shape-vm/src/executor/objects/method_registry.rs` —
   per-receiver-kind `MUT_SELF_TUPLE_RETURN_*` `phf::Set` constants
   (`MUT_SELF_TUPLE_RETURN_ARRAY_METHODS` /
   `MUT_SELF_TUPLE_RETURN_DEQUE_METHODS` /
   `MUT_SELF_TUPLE_RETURN_PRIORITY_QUEUE_METHODS` /
   `MUT_SELF_TUPLE_RETURN_HASHMAP_METHODS`) +
   `is_mut_self_tuple_return_method_name` predicate. New `remove`
   entry in `HASHMAP_METHODS` pointing at the new `v2_remove`
   handler.
2. `crates/shape-vm/src/compiler/mutation_writeback.rs` —
   `ContainerKind::is_mut_self_tuple_return_method` classifier +
   `MutSelfWriteBackMode` enum (`SelfReturn` vs `TupleReturn`) as
   documentation of the two ABI partitions.
3. `crates/shape-vm/src/compiler/expressions/function_calls.rs` —
   `resolve_mut_self_tuple_return_target` resolver +
   `is_known_tuple_return_method` predicate + the post-`CallMethod`
   `Swap; Store{Local,ModuleBinding}` / `Swap; Pop` emit gate +
   integration with the let-immutability guard.
4. Pop-shape handler updates:
   - `executor/objects/array_basic.rs::handle_pop_v2` — pushes the
     new `Arc<TypedArrayData>` via `vm.push_kinded` before returning
     the popped element.
   - `executor/objects/deque_methods.rs::{v2_pop_back, v2_pop_front}` —
     pushes the new `Arc<DequeData>`.
   - `executor/objects/priority_queue_methods.rs::v2_pop` — pushes
     the new `Arc<PriorityQueueData>`.
   - `executor/objects/hashmap_methods.rs::v2_remove` (new) —
     extracts the value-at-key BEFORE mutation (so post-mutation
     buffer cloning doesn't strand the value), then pushes the new
     `Arc<HashMapData>` and returns the popped value.
5. Smoke tests in `crates/shape-vm/src/executor/tests/pop_mutation.rs`
   — 17 tests covering Deque popBack/popFront, PriorityQueue pop,
   HashMap remove, let-immutability compile errors, r-value receiver
   silent-drop, regression guard that `HashMap.delete` stays
   self-returning for the stdlib `set.shape::remove` wrapper, and
   bytecode-level checks that the `Swap; Store*` / `Swap; Pop`
   sequence emits correctly.

Binding for Phase 2d onward.

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
