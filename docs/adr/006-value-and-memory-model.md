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

Answers below were reached during the ADR-006 review on 2026-05-08.
Q5 remains predicted-pending-audit; the rest are decisions binding for
Phase 1 onward.

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
