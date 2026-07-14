# Vertical Deep-Dive 06 — Value Representation, Memory Model & GC

Auditor: 06/19 · Date: 2026-07-11 · Territory: `crates/shape-value/` (all), the GC
implementation (shape-value `gc.rs`/`gc_visit.rs`/`gc_coordinator.rs`, shape-vm
barriers/safepoint/teardown, shape-jit barrier/safepoint FFI), Drop/RAII semantics,
`docs/adr/005` + `docs/adr/006`. Audited against the DIRTY working tree (HEAD
`ce332ca2` + uncommitted changes). Empirical runs used the prebuilt working-tree
binary `target/debug/shape`.

## 0. Executive summary

**Overall health verdict: STRONG core, honest edges, with a documented-but-real
completeness tail.** This vertical is the best-engineered part of the codebase I
have seen evidence of: the value representation (typed 8-byte slots + per-slot
`NativeKind`, `HeapValue` single-discriminator, v2-raw `HeapHeader` carriers) is
coherent and aggressively documented at every load-bearing site; the brand-new
Bacon–Rajan cycle collector is a faithful, test-hardened implementation of its
RATIFIED design (memory-only, no root scan, non-moving, MT rendezvous built in
v1 as ruled) and I reproduced its headline claim empirically — the Finding #31
closure-in-array leak is RSS-bounded end-to-end (74,008 KB at 200k iterations vs
74,004 KB at 1M). The main risks are (a) a deliberately-scoped GC completeness
tail (header-less `Arc`-backed kinds — HashMap/HashSet/Deque/`SharedCell`-rooted
cycles — are barrier-invisible and leak-safe-deferred; `HK_JIT_OBJECT` barrier
tag is still hardcoded 0), (b) a live dual-allocation-discipline footgun on
`TypedObjectStorage`/`TraitObjectStorage` (`Arc::new(new(...))` vs raw `_new()`,
provenance-discriminated by call-site convention only — the exact class that
produced the W5 SIGABRTs), and (c) doc drift: CLAUDE.md's NativeKind variant
list is wrong, Cargo feature comments still say "Default OFF" for a
default-ON feature, and the book documents RAII but says nothing about the
shipped cycle collector.

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P1 | The canonical Finding #31 source form (`var arr = []` / `var arr: Array<() -> int> = []` at statement scope) cannot run at all — `op_new_array(0)` V3-S5 ckpt-5 SURFACE at runtime; only the `fn`-body `let mut arr: Array<() -> int> = []` form reaches the GC | Run transcript §2.2; readiness report caveat §7.2 acknowledges it |
| 2 | P1 | GC completeness tail: cycles rooted only through header-less Arc-backed kinds (HashMap/HashSet/Deque/Channel/Mutex/SharedCell-as-root) are never buffered — `cycle_capable_direct_header` admits only TypedObject/TypedArray/TraitObject (`gc.rs:344-354`); TraitObject is buffered but its children are never enumerated (`gc_visit.rs:107-112`) — such cycles still leak | `gc.rs:344`, `gc.rs:669-677`, `FreeKind::Leak` `gc.rs:553-566` |
| 3 | P1 | `HK_JIT_OBJECT` write-barrier tag still hardcoded `0` (design 3c-iii unresolved) — JIT-object-map cycles invisible to the collector | `shape-jit/src/ffi/object/object_ops.rs:98` |
| 4 | P1 | Dual allocation discipline on the same type: `TypedObjectStorage::new` (Arc lifecycle, header refcount "sits unused") vs `_new` (raw v2 lifecycle) coexist, discriminated only by call-site provenance; same for `TraitObjectStorage` ("both carrier shapes coexist") — the allocator-pair mismatch class that caused the W5 SIGABRTs is still structurally open | `heap_value.rs:3984-4090`, `heap_value.rs:4971-4984`, `heap_value.rs:3560-3576` |
| 5 | P2 | The JIT tier never actually executes the flagship #31 workload — the repro's closure JIT-compile fails (`TypedArrayPushCallable ... no FrameDescriptor`) and whole-program-deopts to the interpreter, so the 3c JIT-barrier path is exercised only by Rust unit tests, not by any end-to-end Shape program | Run transcript §2.2; `ffi/gc.rs` tests |
| 6 | P2 | Split-brain: two `HeapHeader` types (`heap_header.rs` v1 — zero runtime-carrier uses — vs `v2/heap_header.rs` — the real carrier) and two "GarbageCollector"s (inert `memory.rs::GarbageCollector` + trait `gc_integration.rs` no-ops vs the real `shape_value::gc`) — `force_gc()` returns an empty result even with GC on | `heap_header.rs:1-60`, `gc_integration.rs:35-42`, design doc §3.1 CORRECTED note |
| 7 | P2 | Doc drift: CLAUDE.md says NativeKind variants are "Float64 / Int64 / Int32 / Int8 / Bool / Unit / Null / Ptr / String / StringV2 / DecimalV2" — the real enum has 29 variants (all widths + nullables + Float32 + Char, and NO `Unit` variant); Cargo.toml comments in shape-value/shape-vm/shape-jit still say GC "Default OFF" while `default = ["jit","gc"]` | `native_kind.rs:32-225` vs CLAUDE.md; `shape-vm/Cargo.toml:57,72` |
| 8 | P2 | ADR-006 §2.7.6/Q8 drift: `KindedSlot::as_typed_object_storage` is a per-heap-variant accessor, and the "heap dispatch via `as_heap_value()`" rule is unsatisfiable for v2-raw carriers (TypedObject/TypedArray bits are not `Arc<HeapValue>`) — the rule text no longer matches the post-Q25 carrier reality | `kinded_slot.rs:762-790` vs the Q8 comment at `kinded_slot.rs:791-798` |
| 9 | P2 | `collect_cycles`' closure-cascade special case is a bespoke 5-step topology-specific path (restore/reachable-DFS/neuter/gather/cascade) rather than the uniform algorithm — correct for the tested topologies but a fragility hotspot for novel closure-cycle shapes | `gc.rs:1120-1235` |
| 10 | P2 | `mark_gray`/`scan`/`scan_black`/`collect_white` are unbounded recursion over the candidate subgraph — a pathological deep structure (e.g. long linked list buffered as candidate) can overflow the Rust stack at a safepoint | `gc.rs:921-985` |
| 11 | P2 | ADR-005 Layer-2 cleanup (cluster #7) still pending: `ConcreteReturn` retains its parallel per-shape heap arms (`ArrayI64`, `ArrayF64`, `ArrayString`, …) | `typed_module_exports.rs:55-75` |
| 12 | P2 | Book gap: `resource-management.mdx` + `ownership-deep-dive.mdx` document RAII and "Arc everywhere" but never mention the shipped-by-default cycle collector or the memory-only (no-Drop-on-cycles) semantics | `book-site/src/content/docs/fundamentals/resource-management.mdx:8`, `advanced/ownership-deep-dive.mdx:414` |
| 13 | **P1** | *(second-pass, Appendix B.2)* `var`-bound `Option<Node>` object cycles — the most ordinary user-expressible cycle shape — DO form at runtime and leak linearly under the shipped gc-on default binary (325 MB @500k iter → 576 MB @1M vs 78 MB acyclic control): the `Arc<OptionData>` intermediary is header-less and un-enumerated, so the cycle is never buffered and never traced. Corrects A.4's "no RC cycle forms at the language level" (that held only for `let mut` CoW stores) | Appendix B.2 transcripts; `gc.rs:344-354` (`cycle_capable_direct_header` excludes `Ptr(Option)`), `gc.rs:669-677` (`child_heapkind` traces only TypedObject/TypedArray/SharedCell) |
| 14 | **P1** | *(second-pass, Appendix B.3)* §2.7.30 ReturnSlot reference-escape promotion is broken for user-defined types: `fn make() -> &P` fails to compile with the self-contradictory diagnostic "`&P is not compatible with &P`" (works for `&int` / `&string`) — §6's "CONFORMS (empirical)" holds for primitive referents only | Appendix B.3 transcript |
| 15 | P2 | *(second-pass, Appendix B.4)* `SharedCell::Drop` (`closure_layout.rs:376-395`) and `KindedSlot::Drop` (`kinded_slot.rs:947+`) retire heap shares with NO decrement barrier — decrements flowing through these Drop impls never buffer a surviving cycle-capable carrier (leak-only, not unsound; the wrapping `drop_with_kind` helpers and `MutexData::set` do carry the barrier) | `closure_layout.rs:376`, `kinded_slot.rs:947`, contrast `stack.rs:638/976` |

**Feature-completeness score: 82/100.** The value model, RC, Drop/RAII,
escape-promotion (§2.7.30), and the GC Phases 0–5 (metadata, shared edge
primitive, barriers, single-thread CollectCycles, cross-worker STW rendezvous,
teardown sweep, snapshot v7) are all landed and empirically working; the tail
(header-less-kind collection, TraitObject child enumeration, HK_JIT_OBJECT tag,
OwnedMutable capture edges) is real but is documented, leak-safe, and scoped.

**Code-quality score: 88/100.** Exceptional inline documentation with
design-section cross-references at nearly every function; disciplined
`#[cfg(feature="gc")]` gating (feature-off compiles to nothing, verified by the
readiness lane); test fixtures use production allocators and `Weak`-witness
assertions. Docked points: the 8,142-line `heap_value.rs` monolith, the
comment-to-code ratio hiding logic, the bespoke cascade special case, and
unbounded recursion in the collector.

**Biggest risk.** The dual `Arc::new(TypedObjectStorage::new(...))` /
`TypedObjectStorage::_new(...)` allocation discipline (finding #4). Every other
risk in this vertical fails *safe* (leaks, never frees a live object — the
Bacon–Rajan candidate buffer is only a worklist, and the collector's `Leak`
disposition is explicit). But an Arc-allocated storage flowing into a
`v2_release`-then-`_drop` path (or vice versa) is undefined behavior TODAY, is
prevented only by call-site convention, and has already produced real SIGABRTs
(the W5 `length_typed_object_empty` class). The type system does not
distinguish the two carriers; one wrong constructor choice in new code
re-opens it silently.

## 1. Architecture & code structure map

### 1.1 Module inventory (`crates/shape-value/src/`, LOC via `wc -l`)

| Module | LOC | Responsibility |
|---|---:|---|
| `heap_value.rs` | 8,142 | `TypedObjectStorage`, `TraitObjectStorage`, all `*Data` payload structs (HashMap/HashSet/Deque/Channel/Mutex/Atomic/Lazy/PriorityQueue/Range/Result/Option/Matrix/…), `TypedObjectPtr` newtype, HeapElement impls, Drop glue |
| `v2/closure_raw.rs` | 2,290 | Raw `TypedClosureHeader` closure blocks: alloc/write/read/release, `OwnedClosureBlock`, capture-edge accessors shared with GC |
| `kinded_slot.rs` | 2,143 | `KindedSlot { slot, kind }` — the §2.7/Q7 GENERIC_CARRIER; per-FieldType constructors, scalar accessors, Clone/Drop dispatch |
| `gc.rs` | 2,111 | GC Phase 0 metadata (`GcMeta`, `GcSideTable`), Phase 2 barriers + candidate buffer, Phase 3a `collect_cycles` (Bacon–Rajan), `maybe_collect` safepoint entry, `gc_note_object_freed` UAF guard |
| `v2/closure_layout.rs` | 1,918 | `ClosureLayout` (capture offsets/kinds), `SharedCell` (spinlock + `UnsafeCell<u64>` + kind companion), layout registry |
| `v2/typed_array.rs` | 1,660 | v2-raw `TypedArray<T>` flat struct (header + ptr + len/cap), element-type `_pad` stamp, per-T monomorphized push/get/drop, callable-element carrier, GC memory-only free + neuter helpers |
| `heap_variants.rs` | 951 | `define_heap_types!` macro — the single source of truth for `HeapValue` (26 arms) + `HeapKind` (36 ordinals, 0–35, ordinal 8 vacated) |
| `content.rs` | 778 | `ContentNode` rich-content tree |
| `gc_coordinator.rs` | 730 | Phase 3b cross-worker stop-the-world rendezvous: registry, ack/resume condvars, `GcSafeRegion`, bounded-abort |
| `value.rs` | 723 | `FilterNode`, `VTable`, residual value types |
| `datatable.rs` | 644 | DataTable columnar storage |
| `v2_struct_layout.rs` | 615 | TypedStruct compile-time field-offset layout |
| `gc_visit.rs` | 591 | Read-only `for_each_heap_child` edge visitor (Phase 1), shared with destructive Drop walks |
| `slot.rs` | 536 | `ValueSlot(u64)` `repr(transparent)`; deprecated transitional `from_heap`; `as_heap_value` |
| `method_id.rs` | 518 | Method-id interning |
| `native_kind.rs` | 453 | `NativeKind` — 29-variant slot-kind discriminator + predicates (`is_refcounted`, width/sign/nullability algebra) |
| `shape_graph.rs` / `shape_graph_current.rs` | 445/265 | Graph value types |
| `iterator_state.rs` | 416 | `IteratorState` lazy pipeline |
| `scalar.rs` | 414 | `NativeScalar` inline value |
| `v2/heap_header.rs` | 398 | **The real** 8-byte `HeapHeader` (refcount@0 AtomicU32, kind@4 u16, flags@6, `_pad`@7) + GC color/buffered bit constants + `GcColor` |
| `heap_header.rs` | 367 | v1 header — zero runtime-carrier uses (design doc: "GC metadata must NOT live there"); kept for `DATA_OFFSET` docs |
| `context.rs` | 334 | Value-side context |
| `v2/typed_result.rs` / `v2/typed_option.rs` | 315/225 | Typed Result/Option carriers |
| `v2/struct_layout.rs` | 287 | v2 struct layout |
| `aligned_vec.rs` | 284 | Aligned buffer allocation |
| `ids.rs` | 278 | `StringId`, interning ids |
| `v2/concrete_type.rs` | 805 | `ConcreteType` monomorphization type |
| `reference.rs` | 242 | `RefTarget` (Local / ModuleBinding / projected / §2.7.30 PromotedCell) |
| `v2/string_obj.rs` / `v2/decimal_obj.rs` | 239/230 | v2-raw leaf carriers (`StringObj`, `DecimalObj`) |
| `vm_closure_handle.rs` | 232 | Stable closure read API |
| `v2/alloc_budget.rs` | 220 | Allocation budgeting |
| `v2/function_type_registry.rs` | 211 | Function-type registry |
| `string_intern.rs` | 184 | String interning |
| `v2/refcount.rs` | 147 | `v2_retain` (Relaxed fetch_add) / `v2_release` (Release fetch_sub + Acquire fence) / `v2_get_refcount` |
| `v2/heap_element.rs` | 79 | `HeapElement` trait (retain_elem/release_elem contract) |
| `lib.rs` | 119 | Crate root, `gc`/`gc_visit`/`gc_coordinator` gated `#[cfg(feature="gc")]` |
| **Total** | **31,555** | |

GC-adjacent code outside shape-value:

| Location | Role |
|---|---|
| `shape-vm/src/executor/dispatch.rs:18,203-205,370-372` | Same-thread safepoint: `maybe_collect(GC_CANDIDATE_THRESHOLD=256)` on the existing `& 0x3FF` instruction gate |
| `shape-vm/src/executor/vm_impl/stack.rs:618,638,976` | `clone_with_kind` increment barrier; `drop_with_kind` decrement precheck + buffer |
| `shape-vm/src/executor/mod.rs:740-885` | VM `Drop`: shared-module-binding release + module-binding kinded release + **Phase 4 teardown sweep** (`collect_under_stop(collect_cycles)` + buffer clear) |
| `shape-vm/src/executor/v2_handlers/v2_array_detect.rs:806-868` | Array-element overwrite decrement barriers (3 sites) |
| `shape-vm/src/executor/gc_integration.rs` + `memory.rs` | **Legacy inert stubs** (`force_gc` returns empty; "Arc reference counting handles memory") |
| `shape-vm/src/executor/tests/gc_teardown.rs` (405 LOC) | Finding #82 teardown-sweep tests with production allocators |
| `shape-jit/src/ffi/gc.rs` (267 LOC) | `jit_gc_safepoint` (parks via coordinator when flag raised), `jit_write_barrier` (threads old-kind tag), 3 gc-gated integration tests |
| `shape-jit/src/context.rs:646,718` | `gc_safepoint_flag_ptr` wired to `gc_coordinator::stop_flag_ptr()` under `gc`; null otherwise |
| `shape-jit/src/mir_compiler/places.rs:781-833,1361-1387` | `inline_typed_field_set` emits compile-time-constant-tag `jit_write_barrier` for cycle-capable fields (3c-ii); dynamic-kind fields route to the FFI setter which reads `field_kinds[idx]` at runtime |
| `shape-jit/src/ffi/typed_object/field_access.rs:94` + `ffi/data.rs:472` | 3c-i FFI sites: tag from the object's stamped `field_kinds[idx]` |
| `shape-runtime/src/snapshot.rs:116` | `SNAPSHOT_VERSION = 7`; `SerializeIdentityCtx` (line 1250) generalized identity map |

### 1.2 Data flow

**Value representation.** A runtime value is 8 bytes (`ValueSlot`,
`repr(transparent)` over `u64`, `slot.rs`) plus a compile-time-proven
`NativeKind` that lives *outside* the bits: in the opcode (STATIC_KIND sites),
in the VM stack's parallel `Vec<NativeKind>` track (§2.7.7/Q9), in
per-container kind tracks (`TypedObjectStorage.field_kinds`,
`ClosureLayout.capture_native_kind`, `SharedCell.kind`, `TypedArray`'s `_pad`
element stamp), or bundled as `KindedSlot` at GENERIC_CARRIER sites. Heap
payloads are either std-`Arc<T>` raw pointers (`Arc::into_raw`) or v2-raw
carriers with an embedded `HeapHeader` at offset 0
(TypedObject/TypedArray/Closure blocks/StringObj/DecimalObj/TraitObject).
Clone/drop dispatch on the kind — `stack.rs::clone_with_kind/drop_with_kind`,
`KindedSlot::clone/drop`, `TypedObjectStorage::drop_fields` (heap_mask +
field_kinds), `SharedCell::drop`, container Drops — the "4-table lockstep"
family.

**GC.** RC is the fast path (free at rc==0, unchanged). The decrement barrier
inside `drop_with_kind` prechecks rc>1 on cycle-capable direct-header carriers
and buffers survivors Purple in a `thread_local!` `CandidateBuffer` (`gc.rs:390-395`).
The dispatch-loop safepoint fires `maybe_collect(256)`; the initiator takes the
global `collector_lock`, raises `stop_requested`, waits (bounded 500 ms) for
every registered mutator to park, and runs the three-pass trial-deletion
(`mark_gray`/`scan`+`scan_black`/`collect_white`) using
`gc_visit::for_each_heap_child` — the same enumeration primitive the
destructive Drop walks consume, so the read set and release set cannot drift.
White garbage is freed **memory-only** (no user Drop, no child-release walk);
the closure-in-array topology takes a special cascade path (§9.3 below). VM
`Drop` runs one final teardown collection (Phase 4, Finding #82).

### 1.3 Key types

- `NativeKind` (`native_kind.rs:32`) — 29 variants; no `Dynamic`/`Unknown`/`Default` (deleted, comment at 220-225).
- `HeapKind` (`heap_variants.rs:63`) — 36 ordinals 0–35; ordinal 8 (TypedArray-as-HeapValue-arm) vacated do-not-reuse; pure-discriminator variants (FilterExpr, SharedCell, Reference, ModuleFn, Matrix, MatrixSlice) documented as `as_heap_value()`-unsound.
- `HeapValue` (`heap_variants.rs:430`) — 26 arms, all typed-`Arc<T>` or inline-scalar payloads; `TypedObject(TypedObjectPtr)` is the one raw-pointer newtype (v2-raw share).
- `HeapHeader` (v2) — 8 bytes, refcount AtomicU32 @0; GC color bits 4–5 + buffered bit 6 in flags (constants gated `#[cfg(feature="gc")]`, `v2/heap_header.rs:100-107`).
- `KindedSlot` (`kinded_slot.rs`) — `{ slot: ValueSlot, kind: NativeKind }` (+ Miri provenance sidecar under `cfg(miri)`).
- `RefTarget` (`reference.rs:127`) — Local / ModuleBinding / field-projection / §2.7.30 PromotedCell.
- `GcColor` (Black/Gray/White/Purple), `GcMeta` (Header flags-ptr vs SideTable addr), `GcSideTable` (addr-keyed `{color, buffered, shadow_trial_count, shadow_seeded}`), `CandidateBuffer`, `GcCoordinator`.

### 1.4 Entry points

- Barriers: `gc_increment_barrier` / `gc_decrement_precheck` + `gc_buffer_possible_root` / `gc_jit_write_barrier` (`gc.rs:401-487`).
- Collection: `collect_cycles()` (`gc.rs:1071`), `maybe_collect(threshold)` (`gc.rs:1257`), teardown sweep (`executor/mod.rs:850-885`).
- Rendezvous: `ensure_registered` / `park_at_safepoint` / `collect_under_stop` / `GcSafeRegion` (`gc_coordinator.rs`).
- Feature wiring: `shape-vm` `default = ["jit","gc"]`, `gc = ["shape-value/gc"]`; `shape-cli` `default = ["jit","gc"]`, `gc = ["shape-vm/gc", "shape-jit?/gc"]` — the two-tier flip is handled at the CLI (the shipped binary), NOT by shape-vm forwarding to shape-jit (per the known two-tier-flip hazard; shape-vm cannot forward since it does not depend on shape-jit).

## 2. Feature completeness

Legend: ✅ works end-to-end (empirically verified or covered by production-allocator
tests), 🟡 code exists / partially works, 🔴 stubbed or missing.

### 2.1 Value representation & memory model

| Feature | Status | Evidence |
|---|---|---|
| Typed 8-byte slots, per-slot NativeKind, no tag decode | ✅ | `slot.rs`, `native_kind.rs`; `NativeKind::Dynamic/Unknown` deleted (`native_kind.rs:220-225`); no `value_word.rs` anywhere in `shape/crates` |
| `HeapValue` single discriminator + typed `Arc<T>` payloads | ✅ | `heap_variants.rs:430-560`; every arm typed-Arc or inline scalar; ADR markers present |
| v2-raw `HeapHeader` carriers (refcount@0) | ✅ | `v2/heap_header.rs:148-160,236-238` (compile-time size assert; offset tests at 250-268) |
| `TypedArray<T>` contiguous native buffer | ✅ | `v2/typed_array.rs` flat struct (header + `*mut T` + len/cap), per-T monomorphized ops; element stamp in `_pad` |
| TypedStruct compile-time field offsets | ✅ (code+tests) | `v2_struct_layout.rs` (615 LOC), `heap_value.rs` `JIT_OFFSET_SCHEMA_ID=8` / `JIT_OFFSET_SLOT_DATA=16` pinned by `jit_offset_constants_hold` |
| RC on escape (storage-class lattice) | ✅ | `BindingStorageClass` (shape-vm `type_tracking.rs`), `storage_planning.rs`; exercised by every closure test |
| Drop trait / RAII at scope exit | ✅ empirical | §2.4 transcript: `drop in-fn` printed at fn scope exit before `after fn call` |
| Escaping-binding Drop deferral (§2.7.30) | ✅ empirical | §2.4 transcript: module-scope `Res` drop printed after `end of program` |
| `return &local` / module-scope `let r = &x` escape→RC promotion (§2.7.30 narrow floor) | ✅ empirical | §2.4 transcript: both forms compile and read correctly (no B0003) |
| KindedSlot GENERIC_CARRIER (§2.7/Q7) | ✅ | `kinded_slot.rs`; VM stack parallel-kind track `Vec<NativeKind>` (`executor/mod.rs` module-binding twin at 826-846) |
| StringV2/DecimalV2 v2-raw leaf carriers | ✅ | `v2/string_obj.rs`, `v2/decimal_obj.rs`; retain/release via `v2_retain/v2_release` |

### 2.2 GC — phase-by-phase against the RATIFIED design (`docs/design/real-gc-cycle-collection.md`)

| Design phase | Status | Evidence |
|---|---|---|
| Phase 0 — metadata + `gc_meta` + side table + `gc` flag | ✅ | `gc.rs:39-309`; color bits 4–5 / buffered bit 6, bit 3 closure flag untouched (test `gc_meta_header_carrier_reads_writes_flags_byte` asserts low-4-bits + `_pad` untouched, `gc.rs:1339-1370`) |
| Phase 1 — shared edge-enumeration primitive | ✅ | `gc_visit.rs`; TypedObject via `for_each_heap_child_edge` (same fn `drop_fields` walks); TypedArray via `for_each_typed_array_elem_ptr` (same as `drop_array_heap`); closure via `closure_immutable_heap_capture_edge` / `closure_shared_capture_edge` (same as `release_typed_closure` / `drop_shared_capture`); parity tests assert visited-set == released-set by witness refcounts (`gc_visit.rs:325-376,451-500,507-550,559-590`) |
| Phase 2 — barriers + candidate buffer | ✅ | Increment barrier after retain (`stack.rs:618`, `closure_raw.rs:1705`); decrement precheck-before-release + buffer-after (`stack.rs:638/976`); interior sinks: SharedCell store via `drop_with_kind` inside `op_store_shared_capture` (`variables/mod.rs:938-975`), MutexData::set explicit barrier (`heap_value.rs:3092-3110`), array-element overwrite (`v2_array_detect.rs:806-868`); JIT `jit_write_barrier` real body (`ffi/gc.rs:67-74`) |
| Phase 3a — CollectCycles (Bacon–Rajan) | ✅ | `gc.rs:513-1238`; 12 collector tests incl. object↔object, array↔object, 3-node, live-cycle-not-collected (refcounts restored exactly, `gc.rs:1692-1726`), acyclic-not-collected, side-table shadow (real `Arc<String>` strong count untouched, `gc.rs:1756-1810`), memory-only Drop-skip head-to-head vs `_drop` (`gc.rs:1820-1886`) |
| Phase 3b — cross-worker STW rendezvous (RATIFIED #2: required in v1) | ✅ | `gc_coordinator.rs`; built as ruled (R1-RESOLVED, full STW not the per-VM tripwire); 6 rendezvous tests: N-workers-frozen, lone-thread trivial, deserter-deregister, two-initiators-one-collect, bounded-abort, GcSafeRegion-counts-parked |
| Phase 3c-i/ii — JIT barrier tags | ✅ | FFI sites thread `field_kinds[idx]` (`field_access.rs:94`, `data.rs:472`); `inline_typed_field_set` emits constant-tag barrier for cycle-capable fields (`places.rs:796-833`) with dynamic-kind fields routed to the FFI setter (`places.rs:1368-1387`) — the design's "primary hot path bypasses the barrier" gap is CLOSED |
| Phase 3c-iii — HK_JIT_OBJECT | 🔴 | Tag still literal `0` at `object_ops.rs:98` — design decision (lower-to-TypedObject vs kind track) never made |
| Phase 4 — teardown sweep (Finding #82) | ✅ | `executor/mod.rs:850-885` (runs `collect_under_stop(collect_cycles)` after releasing module-binding roots + clears thread-local buffer); `gc_teardown.rs` tests with weak witnesses |
| Phase 5 — snapshot identity map v6→v7 | ✅ (code) | `snapshot.rs:116` `SNAPSHOT_VERSION = 7`; `SerializeIdentityCtx` at 1250 generalized (BODY/backref emission comments at 875-897); merged as `wave7/gc-ph5-snapshot-v7` per git log |
| gc-on by default (both tiers) | ✅ | `shape-cli/Cargo.toml:13,26` (`default = ["jit","gc"]`, `gc = ["shape-vm/gc", "shape-jit?/gc"]`), commit `ce332ca2` |
| Header-less kinds actually collected (§3.5 option A end state) | 🔴 deferred | Barrier admits only TypedObject/TypedArray/TraitObject (`gc.rs:344-354`); `FreeKind::Leak` for everything Arc-backed (`gc.rs:553-566,1053-1057`); documented as "§3.5 option-A migration is fast-follow" |
| TraitObject child enumeration | 🔴 deferred | Buffered as candidate but `for_each_heap_child` has no TraitObject arm (`gc_visit.rs:107-112` falls to `_ => {}`); `child_heapkind` excludes it (`gc.rs:669-677`) — a TraitObject-membered cycle is ScanBlack'd or leak-deferred, never freed |
| `OwnedMutable` (Box-cell) capture edges | 🔴 deferred | `gc_visit.rs:218-224`: "not enumerated here" — documented §3.5 deferral |

**Empirical verification of the headline claim (Finding #31 bounded).** Working-tree
binary, annotated repro (the only form that runs — see below):

```
fn leak() {
    let mut arr: Array<() -> int> = []
    arr.push(|| arr.len())
}
for i in 0..N { leak() }
```

RSS polled at 100 ms via `/proc/<pid>/status` (no `/usr/bin/time` on this NixOS host):

```
f31_fn_small (N=200,000):   exit=0  MAX_RSS_KB=74008
f31_fn_large (N=1,000,000): exit=0  MAX_RSS_KB=74004
```

Flat at 5× the iterations — the per-iteration 4-node cycle is being reclaimed.
This independently reproduces the readiness report's gc-on column
(`docs/cluster-audits/gc-on-readiness-report.md` §2: 37,724 KB @200k → 36,552 KB
@1M on a release build; my numbers are a debug build, hence higher but equally flat).

**Two negative results from the same run:**

1. The canonical design-doc repro form does NOT run. `var arr: Array<() -> int> = []`
   inside a top-level `for` body fails at runtime:
   `Error: Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
   consumer-cascade tier 3 surface … (line 4)` — even WITH the element-type
   annotation. Only the `fn`-body `let mut` form reaches the GC. The readiness
   report's caveat §7.2 discloses the bare-form failure but describes the
   annotated form as working — that is true only in function-body position.
2. The JIT tier never executes this workload: `[jit-fallback] … V2 typed opcode
   TypedArrayPushCallable at offset 2010 in function '__closure_0' has no
   FrameDescriptor … falling through to bytecode interpreter`. So the flagship
   cycle workload exercises the **interpreter** barriers only; the JIT barrier
   path (3c) is proven by Rust-level integration tests
   (`jit_produced_typed_object_cycle_is_collected`,
   `jit_set_field_overwrite_barrier_buffers_and_collects_object_cycle`,
   `ffi/gc.rs:134-266`) but by no end-to-end Shape program I could construct.

### 2.3 What cycles are actually collectable today (measured boundary)

Collectable: cycles whose every edge runs through TypedObject fields,
TypedArray heap elements (String/Decimal/TypedObject/TraitObject/nested-array
element stamps + the CALLABLE closure carrier), closure immutable heap captures,
closure `Shared` (`SharedCell`) captures, and the specific
array→closure→block→SharedCell→array topology. NOT collectable (leak-safe
deferred): cycles rooted or exclusively pinned through HashMap / HashSet /
Deque / Channel / Mutex / Lazy / Iterator / module-fn maps / JIT `HK_JIT_OBJECT`
objects; TraitObject-interior edges; `OwnedMutable` box-cell captures. All of
these fail toward leak, never toward premature free — verified by reading every
`FreeKind::Leak` and `_ => {}` visitor arm cited above.

### 2.4 Drop/RAII + reference escape — empirical transcripts

`drop_raii.shape` (impl Drop with implicit receiver — note `fn drop(self)` is
rejected with a good diagnostic, "method receivers are implicit"):

```
before fn end
drop in-fn            ← RAII at fn scope exit
after fn call
end of program
drop module-scope     ← escaping module binding deferred to program end (§2.7.30)
```

`ref_escape.shape`:

```
fn make_ref() -> &int {
    let mut local = 42
    return &local        ← §2.7.30 flipped sink #1 (ReturnSlot)
}
let r = make_ref()       → "escaped ref reads 42"
let mut x = 7
let rx = &x              ← flipped sink #2 (ModuleBindingStore)
                         → "module-scope ref reads 7"
```

Both §2.7.30 narrow-floor sinks work; no B0003, correct values. (Caveat: any
program containing a trait/impl block whole-program-deopts to the interpreter —
`Wave-20A user-trait-method JIT SURFACE` printed on every Drop test — so Drop
semantics are effectively interpreter-only in practice today.)

`drop_cycle.shape` (object graphs with Drop): an acyclic object drops at scope
exit (`drop solo`); an attempted `a.next = Some(b)` / `b.next = Some(a)` cycle
printed `drop b` / `drop a` at scope exit — i.e. plain TypedObject field stores
under `let mut` did NOT form an RC cycle at the language level (value/CoW
semantics intervene). The only language-level cycle constructor I could produce
is the closure-captured-mutable-array form. This matters: the GC's
cycle-collection surface is currently reachable from user code almost
exclusively through closures, which matches where the barriers are strongest.

### 2.5 Stubbed / legacy surfaces

- `gc_integration.rs` `force_gc()` → empty `GCResult`, `maybe_collect_garbage()` → no-op comment "Arc reference counting handles memory" — even in a gc-on build (`gc_integration.rs:35-42`). The real collector is invisible to this trait. Any host/tooling calling `force_gc` gets a lie.
- `memory.rs::GarbageCollector` — bookkeeping-only stub struct (`memory.rs:53-80`).
- `heap_header.rs` (v1) — parallel header type with its own FLAG_* constants; zero runtime carriers (only `v2/refcount.rs` and `closure_layout.rs` use the v2 one; design doc §3.1 CORRECTED note confirms).

### 2.6 TypedArray / TypedStruct layout claims — verified against source

CLAUDE.md claims: "TypedArray\<T\>: Contiguous native buffer (HeapHeader +
`*mut T` + len/cap). `Array<number>` → `TypedArray<f64>` with `arr[i]` =
`load f64 [data + i*8]`" and "TypedStruct: C-compatible fixed layout with
compile-time field offsets."

Verified for TypedArray (`v2/typed_array.rs`):

```rust
#[repr(C)]
pub struct TypedArray<T> {
    pub header: HeapHeader,   // 8 bytes, refcount @0
    pub data: *mut T,         // @8 — contiguous element buffer
    pub len: u32,             // @16
    pub cap: u32,             // @20
}
const _: () = { assert!(size_of::<TypedArray<f64>>() == 24); /* + i32, u8, f32, char */ };
```

(`typed_array.rs:28-48`) — 24-byte header struct, one fixed layout regardless
of `T` ("repr(C): header @0, data @8 = one 8-byte pointer regardless of T",
`typed_array.rs:325`), so JIT codegen can address `data`/`len` at constant
offsets for every monomorphization. The element-type discriminant is stamped
into the header `_pad` byte (offset 7) — 20 `ELEM_TYPE_*` ordinals
(`typed_array.rs:445-506`: UNKNOWN=0 through CONTENT=19, including the heap
elements STRING=13 / DECIMAL=14 / TYPED_OBJECT=15 / TYPED_ARRAY=16 /
TRAIT_OBJECT=17 / CALLABLE=18), written by `stamp_elem_type`
(`typed_array.rs:533`) and read by `read_elem_type` (`typed_array.rs:513`).
The stamp drives the monomorphized free dispatch (`typed_array.rs:780-800`:
per-`ELEM_TYPE` `TypedArray::<T>::drop_array` arms) — this is producer-side
kind stamping, not a runtime bits probe, consistent with the strict-typing
rules. The GC additions live beside it: `free_v2_typed_array_memory_only`
(`typed_array.rs:877` — frees header + buffer, releases no element share) and
`gc_neuter_callable_closure_edges` (`typed_array.rs:936` — zeroes closure
element bits so the cascade cannot double-drop, §9.5/§2.2).

The 16-byte `CallableArrayElem` record (`typed_array.rs:549-560`,
`{bits: u64, kind: CallableArrayElemKind}`) is the one non-uniform element
carrier; its `Closure` variant owns an `Arc<HeapValue::ClosureRaw>` share and
is enumerated by the shared callable-element primitive for both Drop and GC
(`gc_visit.rs:172-181`).

TypedStruct: `v2_struct_layout.rs` (615 LOC) + `v2/struct_layout.rs` (287 LOC)
compute compile-time field offsets; the TypedObject analog is pinned for JIT
consumption by `JIT_OFFSET_SCHEMA_ID = 8` / `JIT_OFFSET_SLOT_DATA = 16`
(`heap_value.rs:4106-4118`) with a dedicated offset-stability test
(`jit_offset_constants_hold`). The CLAUDE.md claim "point.x = load f64
[ptr + 8]" is directionally right but imprecise for TypedObject: field reads
are TWO loads (slot-buffer base at `[ptr+16]`, then `[slot_data + i*8]`)
because `slot_cells` is an out-of-line `Box<[UnsafeCell<ValueSlot>]>` — the
places.rs comment says exactly this ("hot path stays two loads",
`places.rs:793-794`). Fixed-inline-layout single-load structs exist only on
the `v2_struct_layout` path.

### 2.7 Refcount primitive semantics — verified

`v2_retain` = `fetch_add(1, Relaxed)`; `v2_release` = `fetch_sub(1, Release)`
+ `Acquire` fence when the count hits zero (`v2/refcount.rs:15-46`,
duplicated on the header methods `v2/heap_header.rs:204-227`) — the standard
Boost/Arc protocol, correct for cross-thread release even though today's heaps
are thread-confined. An 8-thread retain/release stress test exists
(`v2/heap_header.rs:349-377`). The GC's trial arithmetic uses `Relaxed`
fetch_sub/fetch_add directly on the refcount (`gc.rs:812-845`) — sound only
under STW quiescence, which `collect_under_stop` guarantees and the SAFETY
comments state.

## 3. Code quality

### 3.1 Idiom & naming

Consistently strong. Names carry their design lineage (`gc_decrement_precheck`
vs `gc_buffer_possible_root` split so the RC fast path stays byte-identical;
`cycle_capable_direct_header`; `slot_cells` deliberately renamed from `slots`
so the `UnsafeCell` migration surfaced compile errors at every stale reader —
`heap_value.rs:4014-4020`). Constructors follow the per-FieldType convention
uniformly (39 `from_*` constructors on `KindedSlot`, one per carrier — counted
via grep, `kinded_slot.rs:127-719`). The `_new`/`_drop` underscore convention
marks the raw-lifecycle allocator pair, though see §9.2 for why convention is
not enough.

### 3.2 Documentation density

Unusually high — most functions in `gc.rs`/`gc_visit.rs`/`gc_coordinator.rs`
carry a design-doc section reference (`§3.2`, `R1-RESOLVED`, `Q10`), and every
GC-relevant site is grep-able by design-section. The cost: files read at
roughly 50–70% comment; `heap_variants.rs` spends ~340 lines of its 951 on
ordinal-history comments. This is the correct trade for a codebase with a
documented multi-agent-drift history, but it buries logic (the entire
`collect_cycles` special-case control flow is 165 lines of which ~90 are
prose).

### 3.3 Error handling

The crate follows surface-and-stop: kind-gaps produce
`VMError::NotImplemented(SURFACE …)` (observed live in the `op_new_array(0)`
transcript, §2.2) rather than a Bool-default or silent fallback. GC code paths
prefer leak-safe dispositions (`FreeKind::Leak`, `arc_strong_count_seed`'s
`u32::MAX / 2` conservative sentinel for unknown Arc kinds, `gc.rs:801-803`)
over guessing. The bounded-abort STW policy (`STW_ACK_TIMEOUT` 500 ms, abort =
clear-stop + return 0, `gc_coordinator.rs:78-84,263-284`) is the right
fail-safe direction: a missed collection defers, never deadlocks or corrupts.
`Mutex` lock poisoning is handled by `.expect("mutex poisoned")` throughout the
coordinator — acceptable for an abort-on-poison policy but each panics at a
safepoint held under `collector_lock`; a poisoned coordinator would wedge all
future collections (low likelihood; noted, not rated).

### 3.4 Unsafe usage

Occurrence counts (`unsafe fn` + `unsafe {`, grep): `heap_value.rs` 252,
`closure_raw.rs` 192, `typed_array.rs` 104, `gc.rs` 103, `gc_visit.rs` ~20 —
this is intrinsically a raw-pointer runtime, so volume is expected. Quality
observations:

- **Justified and documented**: virtually every `unsafe` block in the GC files
  carries a specific SAFETY comment naming the invariant (e.g. "buffered roots
  are kept alive by the cycle … the RC free path clears any entry it reclaims",
  `gc.rs:1097-1101`; "reconstruct-then-forget" strong-count reads,
  `gc.rs:775-804`).
- **The one deliberate laxity**: `gc_meta()` is NOT marked `unsafe` despite
  computing an interior pointer, with an explicit justification (computes but
  does not dereference, `gc.rs:82-89`). Defensible.
- **Weakest invariant**: `GcNode` operations assume every reachable node
  address is live for the entire collection. This holds because (i) collection
  runs under STW, (ii) `gc_note_object_freed` removes RC-freed addresses from
  the buffer (`typed_array.rs:778`, `heap_value.rs:3581` TraitObject release,
  `heap_value.rs:4978` TypedObject release), and (iii) White frees are deferred
  to after the full traversal (`collect_white` pushes into `freed`,
  `gc.rs:977-985`). I checked all three legs; the reasoning is sound for the
  kinds that can be buffered. The invariant is however **distributed across
  three files** with no single assertion enforcing it.
- Miri support is real: `field_provenance` sidecar under `cfg(miri)`
  (`heap_value.rs:4032-4036`), Miri-specific tests
  (`miri_write_slot_in_place_replaces_typed_object_field_and_preserves_metadata`
  etc.), and the `UnsafeCell` slot-cells migration was explicitly done to fix a
  Stacked/Tree-Borrows violation (`heap_value.rs:3995-4008`). This is rare
  discipline.

### 3.5 Complexity hotspots

- `heap_value.rs` at **8,142 lines** is the crate's god-file: 15+ payload
  structs, two storage types, Drop glue, equality, display. Longest items
  (awk over fn bodies): `fmt` 238 lines, `release_one_field` 225 lines
  (per-kind release dispatch), `equals` 141, `clone` 132.
- `collect_cycles` (`gc.rs:1071-1238`) — the two-phase borrow dance (Phase A
  inside the `CANDIDATES` borrow, Phase B outside to avoid the RefCell
  double-borrow via `gc_note_object_freed` re-entry) plus the 5-step cascade
  special case is the single most intricate function in the territory.
- `kinded_slot.rs` Clone/Drop dispatch — a full 36-ordinal match duplicated
  against `stack.rs::clone_with_kind/drop_with_kind` (see §4.1).

### 3.6 Dead code in-territory

Minimal and annotated: one `#[allow(dead_code)]` in shape-value
(`int_float_array_eq`, `heap_value.rs:5306`, explicitly retained for the v2-raw
rebuild with a dated note). Zero `#[ignore]`d tests in shape-value (grep). The
v1 `heap_header.rs` is the largest de-facto-dead surface (see §5.1). The legacy
`GCIntegration` trait + `memory.rs::GarbageCollector` are live-but-inert API
surface (§5.2).

## 4. Duplication & DRY violations

### 4.1 The kind-dispatch table family (accepted, fenced duplication)

The per-`NativeKind`/`HeapKind` retain/release dispatch exists in at least four
places (the codebase's own "4-table lockstep"):

1. `shape-vm/executor/vm_impl/stack.rs::clone_with_kind` / `drop_with_kind` (~36-arm matches);
2. `shape-value/kinded_slot.rs` `impl Clone/Drop for KindedSlot` (declared mirror: "Mirror of `KindedSlot::drop`" comment at `stack.rs:625`);
3. `shape-value/heap_value.rs::TypedObjectStorage::release_one_field` (225-line per-kind release);
4. `SharedCell::drop` + container Drops (`closure_layout.rs`, `heap_value.rs`).

Divergence here is the historically-feared failure mode. Mitigations that
exist: `verify-merge.sh` checks 4-table lockstep; the GC edge visitor was
*deliberately built on shared primitives instead of adding a fifth table*
(`gc_visit.rs:14-31`) — the right call. Residual risk: tables 1 and 2 are still
hand-mirrored (a new HeapKind needs both), and nothing mechanical diffs them;
the FilterExpr type-confusion incident (`heap_variants.rs:101-111`) is the
canonical example of what a one-table miss does.

### 4.2 `gc_meta` vs `GcNode::meta` (real, subtle)

Two placement functions decide where GC metadata lives: `gc_meta(ptr, HeapKind)`
(`gc.rs:90-98`, keyed on HeapKind alone) and `GcNode::meta()`
(`gc.rs:653-662`, keyed on NativeKind because "gc_meta … would mis-place
`Arc<String>` vs the `StringObj` carrier"). The comment is honest, but the two
classifiers CAN give different answers for the same HeapKind (e.g.
`HeapKind::String`: `is_header_carrier` says header (`gc.rs:45-55`, meaning the
StringObj carrier), `GcNode::is_arc_backed` routes `Ptr(HeapKind::String)` to
the side table (`gc.rs:619-645`, meaning `Arc<String>`)). Same for
`HeapKind::Decimal` and `Closure`. The ambiguity is inherent (one HeapKind
label covers two carrier shapes — see §5.4), but having both functions live
invites a future caller picking the wrong one. The barrier path
(`gc_buffer_possible_root → gc_meta`) is only safe because the buffer can only
ever contain the three direct-header kinds.

### 4.3 `build_finding31_cycle` duplicated in two test files

`gc.rs:1913-1966` and `executor/tests/gc_teardown.rs:60-101` construct the same
four-node production-allocator cycle with near-identical code. Divergence risk
low (tests), but a shared fixture would keep the topology definition single.

### 4.4 Flag constants duplicated across the two headers

`FLAG_MARKED/PINNED/READONLY` are defined independently in `heap_header.rs:27-32`
(v1) and `v2/heap_header.rs:65-67` — same values today, nothing ties them. The
GC color constants exist only on the v2 side (correct per the design's
CORRECTED-carrier note), which at least prevents the worst confusion.

## 5. Split-brain analysis

### 5.1 Two `HeapHeader` types (v1 vs v2) — MEDIUM risk

`crate::heap_header::HeapHeader` (367 LOC, imports `HeapKind` for its kind
field, exports `DATA_OFFSET=8`) vs `crate::v2::heap_header::HeapHeader` (the
real carrier — all six v2-raw carriers embed it; `v2/refcount.rs` and
`closure_layout.rs:854` reference it). Same name, same layout, different flag
namespaces, different kind-constant spaces (`HeapKind as u16` vs
`HEAP_KIND_V2_* ≥ 80` "to avoid collision with v1 HeapKind variants",
`v2/heap_header.rs:20`). The GC design doc itself records that Phase 0 almost
put the metadata on the wrong one ("CORRECTED carrier (impl finding
2026-07-07)"). As long as both exist, every new header consumer faces the same
trap. The v1 header should be deleted or reduced to a doc alias.

### 5.2 Two garbage collectors — LOW risk, HIGH confusion

The inert `memory.rs::GarbageCollector` + `GCIntegration` trait
(`gc_integration.rs`) predate the real collector and still present a
GC-shaped API: `vm.force_gc()` returns `GCResult::new(0, 0, ZERO)`
unconditionally (`gc_integration.rs:39-42`) even when the real
`shape_value::gc::collect_cycles` is compiled in and running. Nothing routes
between them. Any REPL/tooling/metrics surface built on `gc_stats()` reports a
fiction. Either wire the trait to the real collector (a 20-line change:
`force_gc` → `collect_under_stop(collect_cycles)` count) or delete it.

### 5.3 VM vs JIT barrier duality — LOW risk (by construction)

The JIT half re-derives the barrier from a `u64` kind tag
(`gc_jit_kind_tag`/`gc_jit_write_barrier`, `gc.rs:459-487`) because Rust enums
can't cross `extern "C"`. Both halves funnel into the same
`gc_decrement_precheck` + `gc_buffer_possible_root`, so the semantics can't
drift — only the tag table (3 entries) can, and it's defined once in
shape-value next to its decoder. Well-contained.

### 5.4 One `HeapKind` label, two carrier shapes — MEDIUM risk (the deepest one)

`HeapKind::String` labels both `Arc<String>` slots and v2-raw `StringObj`
element carriers; `HeapKind::Decimal` labels both `Arc<rust_decimal::Decimal>`
and `DecimalObj`; `HeapKind::Closure` labels both the `Arc<HeapValue::ClosureRaw>`
value and (inside the GC only) the raw block C. `NativeKind` disambiguates at
the slot tier (`String` vs `StringV2`, `Ptr(Decimal)` vs `DecimalV2`) — the H-c
per-carrier-variant decision, `native_kind.rs:156-198` — but any API keyed on
`HeapKind` alone (like `gc_meta`, `is_header_carrier`) inherits the ambiguity.
The GC handles it by classifying on `NativeKind` inside the collector
(`GcNode.nk`), which is correct but had to be *re-derived* (§4.2). The
parallel-discriminator note on `NativeKind::Char` vs `Ptr(HeapKind::Char)`
(`native_kind.rs:55-70`) documents a third instance with a promised fold-in
that has not happened.

### 5.5 Doc-vs-code drift (concrete instances)

| Claim | Reality |
|---|---|
| CLAUDE.md: NativeKind "Variants: Float64 / Int64 / Int32 / Int8 / Bool / Unit / Null / Ptr(HeapKind) / String / StringV2 / DecimalV2" | 29 variants incl. every unsigned/nullable width, `Float32`, `Char`, `IntSize`; **no `Unit` variant exists** (`native_kind.rs:32-225`) |
| CLAUDE.md: "HeapHeader: … kind `u16`, flags `u8`" (single header implied) | Two header types (§5.1) |
| `shape-value/Cargo.toml:28`, `shape-vm/Cargo.toml:72`, `shape-jit/Cargo.toml:65` comments: gc "Default OFF" | `default = ["jit", "gc"]` on shape-vm (`Cargo.toml:57`) and shape-cli (`Cargo.toml:13`) since `ce332ca2` |
| `gc.rs:4` module doc: "entirely gated behind the **default-off** `gc` Cargo feature" | Default-on via shape-vm/shape-cli defaults |
| Design doc §1: "The existing `gc_integration.rs` / `memory.rs::GarbageCollector` are inert no-op stubs" — accurate then, but the impl never repurposed the "dead `GCConfig` knobs" §8 promised | Stubs still inert next to the live collector (§5.2) |
| ADR-006 §2.7.7 (VM stack `Vec<u64>` + `Vec<NativeKind>`) | Conforms — `executor/mod.rs` `self.stack`/`self.kinds` walked in lockstep at teardown (`mod.rs:784-792`) |

## 6. ADR & spec conformance (rule by rule)

Marker density: 36 `ADR-005` + 383 `ADR-006` references in shape-value source
(grep) — the mirroring mechanism ADR-005 §Visibility prescribed is real.

### 6.1 ADR-005 (typed slot construction)

| Rule | Verdict | Evidence |
|---|---|---|
| §1 Single discriminator — no sum type projecting 1:1 to HeapKind above HeapValue | **CONFORMS (with the sanctioned legacy exception)** | `GcMeta` is a 2-arm placement locator, not a HeapKind projection (`gc.rs:57-74`); `FreeKind` (6 arms: 4 free-strategies + ClosureBlock + Leak) is a free-strategy enum, not a HeapKind mirror — defensible; `HashMapKindedRef` is a per-V monomorphization carrier authorized by ADR-006 §2.7.24 Q25.B. The standing violation is `ConcreteReturn`'s per-shape heap arms (`typed_module_exports.rs:55-75`) — the cluster #7 target ADR-005 itself names as pending; I found no evidence of NEW arms added beyond the ADR-era set, so the "do not extend while pending" rule holds, but the cleanup itself has not happened ~2 months on |
| §2 String exception bounded to the input carrier | **CONFORMS** | `NativeKind::String` (`Arc<String>`) is the only special-cased heap scalar besides the ADR-006-authorized StringV2/DecimalV2 v2-raw carriers (each carrying its own H-c ADR justification inline, `native_kind.rs:156-198`) |
| §3 Typed slot storage — per-FieldType constructors, no `Box<HeapValue>` in new code | **CONFORMS** | 39 per-FieldType `KindedSlot::from_*` constructors; `ValueSlot::from_heap` is `#[deprecated]` with an explicit note (`slot.rs:77-104`); grep shows exactly 3 remaining non-definition callers: `stdlib/json.rs:383` (documented legacy JSON path) and two in `slot.rs` itself (one is the deprecated API's own test, `slot.rs:521-524`) — a shrinking, tracked tail, not drift |
| §3 Drop dispatch by NativeKind (not heap_mask alone) | **CONFORMS** | `TypedObjectStorage::drop_fields` walks `heap_mask` + `field_kinds` together; `release_one_field` dispatches per-kind (`heap_value.rs`, 225-line match) |
| §4 Uniform slot ABI VM↔JIT, no boundary conversion | **CONFORMS (spot-checked)** | JIT reads slots via pinned offsets (`JIT_OFFSET_SLOT_DATA=16` + `jit_offset_constants_hold` test); `places.rs` inline field get/set are raw loads/stores at `[slot_data + byte_off]`; KindedSlot explicitly barred from the slot ABI (§2.7 note in CLAUDE.md; I found no KindedSlot in JIT codegen signatures) |
| §Forbidden — "RawBits/ValueBits/shim/bridge" renames | **CONFORMS** | No `value_word.rs` under `shape/crates`; `no_dynamic.rs` sentinel exists (`shape-vm/src/executor/tests/no_dynamic.rs`); the stale orphan copy holding `value_word.rs` is outside the repo (scope-excluded per audit brief) |

### 6.2 ADR-006 (value & memory model) — rules binding this territory

| Rule | Verdict | Evidence |
|---|---|---|
| §2.3 Typed `Arc<T>` payloads on HeapValue | **CONFORMS** | Every `HeapValue` arm is `Arc<T>`, inline scalar, `OwnedClosureBlock` (self-refcounting), or `TypedObjectPtr` (v2-raw share, sanctioned by the §2.3 D4 amendment noted inline at `heap_variants.rs:465-474`) |
| §2.3/Q6 No `from_heap_arc(Arc<HeapValue>)` catch-all | **CONFORMS** | Explicitly refused in comments (`slot.rs:81,104`); no such constructor exists |
| §2.7.6/Q8 KindedSlot API bounded — one constructor + ≤1 scalar accessor per variant; NO per-heap-variant accessors | **DEVIATES (documented tension)** | Scalar accessors conform (`as_i64/f64/bool/char/f32/str`, `kinded_slot.rs:791-900`, with the rule restated verbatim at 791-798). But `as_typed_object_storage` (`kinded_slot.rs:762-790`) IS a per-heap-variant accessor. Root cause: the Q8 dispatch rule ("heap dispatch via `kinded_slot.slot.as_heap_value()`") is **unsatisfiable** for v2-raw carriers — TypedObject slot bits are `*const TypedObjectStorage`, not `Arc<HeapValue>`, so `as_heap_value()` is unsound on them (the same unsoundness ADR-006 itself documents for FilterExpr/SharedCell/Matrix labels). The accessor is the sound replacement, but Q8's text was never amended to bless it. Needs an ADR-006 §2.7.24-family amendment, not a code change |
| §2.7.7/Q9 VM stack parallel `Vec<NativeKind>`; no `Vec<KindedSlot>` stack, no packed tags, no `Option<NativeKind>` | **CONFORMS** | `self.stack: Vec<u64>` + `self.kinds: Vec<NativeKind>` walked in lockstep (`executor/mod.rs:784-792`); `NativeKind::Bool` used as *dead-slot sentinel* only with an explicit "not a Bool-default fallback in the §2.7.7 #9 sense" justification (`mod.rs:905-918`) |
| §2.7.8/Q10 Cell-storage parallel kinds (closure cells, SharedCell, module bindings) | **CONFORMS** | `SharedCell { value, kind }` with mid-life kind-change refusal enforced at `op_store_shared_capture` (`variables/mod.rs:955-963`); `module_bindings` + `module_binding_kinds` with debug-assert lockstep + defensive `min()` walk + explicitly-NOT-Bool-default unwalked-tail disposition (`mod.rs:806-846`) |
| §2.7.9 FilterExpr pure-discriminator dispatch | **CONFORMS** | `heap_variants.rs:101-112,531-544`; clone/drop tables dispatch `Arc<FilterNode>` |
| §2.7.30 Reference escape→RC promotion (narrow floor: ReturnSlot + ModuleBindingStore only) + escaping-Drop deferral | **CONFORMS (empirical)** | Both sinks work (§2.4 transcripts); module-scope Drop deferred to program end observed; `RefTarget` carries the PromotedCell family (`reference.rs:127+`) |
| §2.7.30.4/Finding #82 teardown finalization | **CONFORMS (as re-ruled)** | The Phase-4 teardown sweep reclaims module-scope cycles at VM Drop (`executor/mod.rs:850-885`); note the GC ratification §0 #1 SUPERSEDED the original "Drop observably runs at teardown" gate — cycles are now reclaimed memory-only, Drop does NOT run (matching Rust), and the design doc records Finding #82's finalizer as "still won't run … accepted" |
| §Single-discriminator "no new modal-types subsystem" | **CONFORMS** | `SharedAtomic`/`SharedAtomicMut` remain absent from `BindingStorageClass` (design R0 verified this; grep confirms zero code hits outside prose) |

### 6.3 GC ratified design (`real-gc-cycle-collection.md` §0) — ruling by ruling

| Ratified ruling | Verdict | Evidence |
|---|---|---|
| #1 Memory-only collector; GC never runs Drop; CollectWhite frees with NO finalize pass | **CONFORMS, test-proven** | `free_white_node` per-kind memory-only frees (`_free_memory_only`, `free_v2_typed_array_memory_only`, `dealloc_typed_closure_no_drop` — `gc.rs:996-1058`); head-to-head test `cycle_member_skips_drop_of_owned_field` proves the same object shape releases its field share via `_drop` but NOT via cycle reclaim (`gc.rs:1820-1886`) |
| #2 MT cross-worker STW rendezvous REQUIRED in v1 (R1-RESOLVED: full STW, not tripwire-only) | **CONFORMS** | `gc_coordinator.rs` implements register/park/ack/resume/serialize exactly as R1-RESOLVED specifies; 6 concurrency tests including frozen-progress assertion under the stop (`gc_coordinator.rs:441-498`). Deviation from the letter: the R4 "tripwire (mandatory)" — a build-failing assertion on `SharedAtomic` appearing or `Arc<HeapValue>` crossing a thread — was specified as defense-in-depth alongside the full STW; I found no such tripwire (grep for it returns nothing). Minor, since the full machinery it was guarding the absence of was built |
| #3 Snapshot as-is + post-resume collect (no force-collect at snapshot) | **CONFORMS (code-read)** | No `collect_cycles` call in `snapshot()` paths; GC state documented transient/never serialized (`gc.rs:208-209,390-394`) |
| #4 Snapshot v6→v7 + generalized identity map | **LANDED** | `SNAPSHOT_VERSION = 7` (`snapshot.rs:116`), `SerializeIdentityCtx` generalized (merge `8e585bcb`); not independently exercised by this audit (snapshot vertical's territory) |
| OQ#2 default: allocation/instruction-count quantum trigger | **CONFORMS** | `maybe_collect(256)` on the 1024-instruction dispatch gate (`dispatch.rs:18,203`); threshold documented as free-to-tune since collection is unobservable |
| OQ#5 default: side-table (option A) for header-less kinds | **PARTIAL by design** | `GcSideTable` exists and shadow-counts are exercised (`collect_cycle_with_arc_string_field_uses_side_table_shadow`), but header-less kinds are never *buffered as roots* and never *freed* (`FreeKind::Leak`) — option A is implemented as scaffolding + read-path, not as end-to-end collection; the fast-follow remains open |
| §3.4 lockstep: shared edge primitive both paths consume | **CONFORMS** | §2.2 Phase-1 row; parity tests assert read-set == release-set via witness refcounts |
| §3.2 barrier placement (inc→Black, dec-to-nonzero→Purple+buffer) | **CONFORMS** | `stack.rs:618/638/976`; precheck-before-release keeps rc==1 fast path untouched (`gc.rs:423-431`) |
| S3/R3: JIT barrier — 3c-i, 3c-ii, 3c-iii | **2 of 3** | 3c-i ✅ (`field_access.rs:94`, `data.rs:472` — runtime `field_kinds[idx]` read, which is a kind-track read, not a tag-bit decode); 3c-ii ✅ (`places.rs:796-833` compile-time constant tag); 3c-iii 🔴 (`object_ops.rs:98` literal 0). Note R3 declared 3c a "hard gate on Phase 5 gc-on-by-default" — gc went default with 3c-iii open; the risk is bounded (completeness, not soundness) but this is a ruling deviation that should be logged |
| §4 No root scan / no `is_heap()` / no tag decode / no ValueWord in the collector | **CONFORMS** | The collector never inspects raw slot bits for pointer-ness; all classification via NativeKind/HeapKind labels carried on nodes; `gc_visit.rs:33-39` restates and the code matches; the `no_dynamic.rs` sentinel + `check-no-dynamic` gate remain in place |

### 6.4 Runtime v2 spec (`docs/runtime-v2-spec.md`) touchpoints

Spot-checked the value-representation claims that bind this territory: 8-byte
slots ✅; refcount at header offset 0 ✅ (compile-time assert
`v2/heap_header.rs:236-238`); `Array<number>` → contiguous `TypedArray<f64>`
✅ (`typed_array.rs` + POD no-edge Drop parity test); TypedObject field =
`load [ptr + offset]` ✅ (two-load hot path documented and JIT-pinned,
`heap_value.rs` JIT_OFFSET constants). The spec has no GC section — it predates
the collector and should gain a pointer to the design doc.

## 7. Test coverage in-territory

### 7.1 Counts

- **shape-value: 480 `#[test]` functions** across 33 files (grep). Densest:
  `heap_value.rs` 106, `closure_layout.rs` 38, `typed_array.rs` 35,
  `gc.rs` 21 (+7 gc_visit, +6 coordinator = **34 GC-specific tests in
  shape-value**), `kinded_slot.rs` 22, `content.rs` 22.
- GC tests outside shape-value: `executor/tests/gc_teardown.rs` (Finding #82
  suite, 405 LOC), `stack.rs:1459-1467` (rc→0 removes stale candidate),
  `ffi/gc.rs` 3 JIT-integration tests, dispatch-loop safepoint coverage via
  the readiness lane's suite runs.
- Zero `#[ignore]` in shape-value (grep) — no parked debt in-territory. (The
  ~23 ignored shape-jit tests are JIT-vertical territory; the 4 ignored
  simulation tests are CLI territory.)

### 7.2 Assertion quality — high

The GC tests are the strongest in the codebase by assertion discipline:

- Production allocators only (`TypedObjectStorage::_new`,
  `TypedArray::with_capacity`, `alloc_typed_closure`,
  `Arc::new(SharedCell...)`) — the exact discipline the W5 postmortem demanded
  after the test-fixture allocator-pair mismatch.
- Refcount witnesses at every step (e.g. `live_cycle_with_external_ref_is_not_collected`
  asserts post-ScanBlack counts EXACTLY: `a==2, b==1`, `gc.rs:1710-1716`).
- `Weak`-handle witnesses to assert Arc payloads reached strong-count 0
  *without* touching freed memory (`weak_of` reconstruct-downgrade-forget,
  `gc.rs:1984-1991`) — a genuinely careful UAF-free way to assert full reclaim.
- Negative tests for every soundness edge: live-cycle-not-collected (×2
  topologies), acyclic-buffered-not-collected, decrement-to-zero-not-buffered,
  tag-0-inert, buffered-bit dedup.
- The memory-only semantics are tested head-to-head against the RC path on the
  same object shape (`cycle_member_skips_drop_of_owned_field`).
- Fixture-teardown honesty: comments even record where an earlier fixture
  leaked and why it was fixed (`gc.rs:1502-1509`).

### 7.3 Gaps

1. **No end-to-end Shape-language GC test** in the repo's test tree (the
   readiness lane ran external scratch programs). The `f31_fn_*.shape` shape
   belongs in `shape-test` with an RSS or iteration-count bound so regressions
   are caught by `just test-all`, not by a one-off audit.
2. **No JIT-tier end-to-end cycle workload** — blocked on
   `TypedArrayPushCallable` FrameDescriptor (§2.2); when that lands, the 3c
   gate test ("forced-tier #31 repro bounded RSS") must be added as designed.
3. **No deep-graph stress** — nothing exercises `mark_gray` recursion depth
   (§9.4).
4. **No concurrent barrier+collect fuzz** — coordinator tests use synthetic
   workers; no test runs real VMs on N threads each building cycles while one
   collects (defensible today given disjoint heaps, but it is the Phase-6
   promise).
5. **Rendezvous tests share a process-global coordinator** and serialize via a
   static mutex + `test_reset` (`gc_coordinator.rs:360-366,380`) — correct but
   means `cargo test` parallelism gives no extra interleaving coverage.

## 8. Book/docs vs reality for this vertical

- **`fundamentals/resource-management.mdx`** — RAII/Drop documentation is
  accurate and matches my empirical runs (scope-exit drop, reverse drop order,
  early-return/break coverage; `method drop()` receiver-implicit syntax — the
  compiler's diagnostic for `fn drop(self)` even points to it). **What is
  missing:** no page anywhere mentions the cycle collector, that cyclic
  garbage is reclaimed memory-only, or that a `Drop` impl on a cycle member
  will NOT run (Rust-`Rc`-like). That is a user-visible semantic (their
  finalizer silently doesn't fire) now shipped ON by default. Per the
  project's own book-gate rule ("every implemented feature must be in the book
  + covered by a gate-runnable example") the GC is un-booked.
- **`advanced/ownership-deep-dive.mdx:414`** — "Arc everywhere — all heap
  values use atomic reference counting" — now incomplete: RC + cycle
  collection. Also nothing on escape→RC promotion for references (§2.7.30),
  which is user-visible (`return &local` works; the book should say when).
- **CLAUDE.md** — NativeKind variant list wrong (§5.5); "GC became a default
  feature" is recorded in git history and memory but CLAUDE.md's Architecture
  section does not mention the collector at all.
- **`docs/runtime-v2-spec.md`** — no GC section (§6.4).
- **Cargo.toml comments** — three crates still say "Default OFF" (§5.5).
- **The GC design doc** is the standout positive: ratification stamps, an
  impl-corrected carrier note, and per-phase gates that the implementation
  demonstrably followed. `docs/cluster-audits/gc-on-readiness-report.md` is a
  model verification artifact (its two caveats were both honest and I
  reproduced both).

## 9. Bugs & correctness risks found

Severity scale: P0 unsound/wrong-results/security · P1 broken feature · P2 paper cut.
No P0 was found in this vertical: every identified gap fails toward *leak*, never
toward premature free — a direct consequence of the Bacon–Rajan candidate-buffer
design (an omitted barrier entry can only under-populate the possible-root
worklist) and of the explicit `FreeKind::Leak` disposition. The findings below
are ordered by risk.

### 9.1 [P1] The canonical Finding #31 program shape cannot run

Repro (working-tree binary):

```
$ shape run f31_small.shape        # var arr: Array<() -> int> = [] inside a top-level for-body
Error: Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. … (line 4)
```

Even with the element-type annotation, an empty array literal at statement
scope inside a top-level loop hits the V3-S5 ckpt-5 construction surface; only
the `fn`-body `let mut arr: Array<() -> int> = []` form constructs. The root
cause is the arrays vertical (op_new_array rebuild), but the memory-vertical
consequence is direct: the exact program the GC was commissioned to fix
(design §1, Finding #31, `var arr = []; arr.push(|| arr.len())`) still cannot
be written as documented. Cross-reference: readiness report caveat §7.2
discloses the bare form; the annotated-at-statement-scope failure above is a
narrower additional case it did not distinguish.

### 9.2 [P1] Dual allocation discipline on `TypedObjectStorage` / `TraitObjectStorage`

Two live lifecycles for the same struct:

- `TypedObjectStorage::new(...)` → by-value, wrapped `Arc::new(...)`; the
  embedded `HeapHeader` refcount "sits at refcount=1 unused (the enclosing Arc
  owns the lifecycle)" (`heap_value.rs:3987-3990`).
- `TypedObjectStorage::_new(...)` → raw `*mut`, header refcount IS the
  lifecycle, freed via `release_elem` → `v2_release` → `_drop`
  (`heap_value.rs:4971-4984`).

`TraitObjectStorage` documents the same duality verbatim: "both carrier shapes
coexist at the struct level during the Wave 2 dispatch transition; the slot ABI
discriminates them by allocation provenance" (`heap_value.rs:3560-3576`).
Nothing in the type system distinguishes an Arc-provenance pointer from a
`_new`-provenance pointer; a `&TypedObjectStorage` or `*const TypedObjectStorage`
from either side type-checks into the other's release path. This is precisely
the class behind the resolved W5 SIGABRTs (test fixtures allocating
`Arc::new(TypedObjectStorage::new(...))` + `Arc::into_raw` and flowing into the
v2-raw `drop_with_kind` dispatch). Production callers of the Arc form are now
rare (grep: one production-adjacent site at `heap_value.rs:6459` plus the
transitional `slot.rs` doc-examples), and the migration note at
`shape-runtime/src/type_schema/mod.rs:405` records the direction — but until
the Arc form is deleted (or newtyped apart), every new consumer re-rolls the
dice. **Recommendation is structural**: make `_new`-provenance pointers a
distinct newtype (as `TypedObjectPtr` already does for the HeapValue arm) and
delete the by-value `new` from the public surface.

### 9.3 [P1→P2] GC completeness tail (documented, leak-direction, but user-visible)

Three concrete sub-gaps, all cited in §2.2/§2.3:

1. **Header-less-rooted cycles leak**: `cycle_capable_direct_header` admits
   only TypedObject/TypedArray/TraitObject (`gc.rs:344-354`); a cycle whose
   only decrement-surviving members are HashMap/HashSet/Deque/Channel/Mutex/
   SharedCell/Closure-value nodes is never buffered, hence never collected.
   E.g. two HashMaps referencing each other (if constructible at the language
   level) or a SharedCell→HashMap→SharedCell loop.
2. **TraitObject is buffered but opaque**: it can enter the candidate buffer,
   but `for_each_heap_child` yields nothing for it (`gc_visit.rs:107-112`) and
   `child_heapkind` excludes it (`gc.rs:669-677`), so a cycle passing through a
   TraitObject's inner TypedObject is invisible → leaks.
3. **`HK_JIT_OBJECT` overwrite barrier inert** (`object_ops.rs:98`, tag 0) —
   design 3c-iii. Note the design's R3 declared 3c a **hard gate on Phase 5
   (gc-on-by-default)**; gc went default (`ce332ca2`) with 3c-iii unresolved.
   The deviation is bounded (completeness, and `HK_JIT_OBJECT` objects are a
   legacy JIT map form) but it contradicts the ruling as written and belongs in
   `docs/defections.md` or an explicit re-scope note.

### 9.4 [P2] Unbounded recursion in the collector

`mark_gray`, `scan`, `scan_black`, `collect_white` (`gc.rs:921-985`) recurse
per edge with no depth bound and no explicit-stack fallback. A buffered
candidate that roots a long chain (e.g. a 10⁶-node linked list of TypedObjects
whose head survived a decrement) recurses 10⁶ frames deep at a safepoint —
Rust's default 8 MB main-thread stack gives roughly 10⁵–10⁶ frames for small
frames; a stack overflow here aborts the process. CPython's implementation of
the same algorithm is iterative for this reason. Likelihood today is low
(deep single-chains rarely enter the buffer since interior nodes are rc==1),
but the failure is an abort, not a leak, making this the collector's only
non-leak-direction defect. Fix: worklist-style iteration.

### 9.5 [P2] The closure-cascade special case over-counts and under-frees on untested topologies

`collect_cycles`' Phase A steps (1)–(5) (`gc.rs:1120-1207`) special-case the
{A array, B closure-value, C block, D cell} topology. Analysis of a plausible
variant — ONE closure value held by TWO callable arrays (both garbage): B's
strong count is 2; `freed` contains one B node, so Phase B drops exactly one
share → B survives at strong 1 → `OwnedClosureBlock::Drop` never runs → C, D
and the cell-referenced array are never freed, yet step (5) counts every
cascade-reachable node as reclaimed (`gc.rs:1194-1201`). The non-reachable
second array is raw-freed memory-only without releasing its element share
(consistent). Net effect: leak + wrong `reclaimed` count; **no UAF and no
double free** (I traced both directions — the neuter step zeroes A→B edges
before any free, and raw-free never touches Arc-backed nodes). This is
analysis, not a reproduced failure — but the cascade's correctness argument is
topology-shaped rather than algorithmic, and each new closure-cycle shape
(two arrays, two cells, closure-in-closure) needs its own proof. A uniform
treatment (side-table trial counts driving per-kind release of ALL White
Arc-backed nodes) would retire the special case.

### 9.6 [P2] `force_gc()` / `gc_stats()` report fiction with GC on

`gc_integration.rs:35-42`: `force_gc` returns an empty result, `maybe_collect_garbage`
is a no-op, in all builds. Any embedder or REPL command calling this trait
concludes "no GC activity" while the real collector runs. Wire or delete (§5.2).

### 9.7 [P2] `gc_meta` mis-places dual-carrier HeapKinds

`gc_meta(ptr, HeapKind::String)` returns `Header` (StringObj assumption,
`gc.rs:45-55`) — but `Arc<String>` payloads carry the same HeapKind label
via `Ptr(HeapKind::String)` slots and have NO header; calling
`gc_meta(...).set_color` on one would write into the `String`'s data bytes.
Today no caller can do this (the barrier only feeds the three direct-header
kinds into `gc_buffer_possible_root`, and the collector classifies on
`NativeKind` via `GcNode::meta`), so it is latent — but the function's
signature invites the bug and its doc does not warn. Same for
`HeapKind::Decimal`/`Closure`. Guard: debug-assert the header's `kind` field
(`HEAP_KIND_V2_*` stamp) inside `GcMeta::Header` accessors.

### 9.8 [P2] Thread-local candidate buffer relies on `VirtualMachine::Drop` for hygiene

The buffer is `thread_local!` (`gc.rs:390-395`); isolated async task VMs run on
pooled `spawn_blocking` threads, and the teardown sweep clears the buffer at VM
Drop (`executor/mod.rs:880-884`). If a VM is leaked (`mem::forget`, abort-only
path) or a future refactor drops the clear, a later VM reusing the thread
inherits stale candidate addresses → `collect_cycles` dereferences freed
memory. Currently safe (Drop is reliable under unwinding; the clear is
present); the invariant deserves a debug assertion at VM construction
("candidate buffer empty on VM start") rather than convention.

### 9.9 [P2] Observed adjacent-vertical defects (logged for the owning auditors)

These surfaced in my empirical runs; roots are outside this territory:

- `null` fails to parse in object-literal field position (`Node { next: null }`
  → "unexpected `}`"), while `None` parses; and a `Node?` field annotation then
  rejects `None` with "Option\<Node\> is not compatible with Node" — the `T?`
  sugar and `Option<T>` do not unify at the object-literal boundary
  (types/parser verticals). Transcript in §2.4 harness runs.
- Every program containing any trait/impl block whole-program-deopts to the
  interpreter (Wave-20A SURFACE) — makes Drop/RAII interpreter-only in
  practice (JIT vertical).
- `fn drop(self)` inside `impl Drop` is rejected (receivers are implicit) —
  correct behavior, good diagnostic, but the book's `method drop()` vs the
  also-accepted `fn drop()` duality is worth one doc line (docs vertical).

### 9.10 Soundness arguments verified (negative findings — things I checked and could NOT break)

- **Rendezvous wake/park race**: a mutator can only exit its park while holding
  the state mutex AND observing `stop_requested == false`; the initiator sets
  the flag SeqCst *before* reading `parked` under the same mutex — a stale
  parked count therefore implies the flag is already up and the "stale" thread
  re-waits instead of exiting (`gc_coordinator.rs:204-217,252-284`). No
  lost-wakeup window: `resume.notify_all` is issued while holding the state
  lock (`gc_coordinator.rs:289-293`).
- **Deferred White free**: `collect_white` collects nodes into `freed` and
  frees only after the full traversal, so a White child shared by two White
  parents is never freed-then-re-read (`gc.rs:968-985` + comment).
- **RC-free vs stale buffer entry**: all three bufferable kinds remove
  themselves on rc→0 (`typed_array.rs:778`, `heap_value.rs:3577-3583`,
  `heap_value.rs:4971-4984`); the RefCell re-entry hazard
  (`release_v2_typed_array` → `gc_note_object_freed` → `CANDIDATES.borrow_mut`)
  is dodged by running the cascade outside the Phase-A borrow
  (`gc.rs:1072-1080` comment + structure).
- **Trial-decrement on the real refcount** is restored exactly by ScanBlack for
  survivors; test asserts post-restore counts to the unit (`gc.rs:1710-1716`).
- **The GC never fabricates a kind from bits**: every `GcNode.nk` originates
  from a producer-side stamp (buffer entries from `cycle_capable_direct_header`
  kind labels; children from `field_kinds`/`_pad`/capture layout/cell
  companions). I found no `is_heap()` probe, no tag decode, and no
  forbidden-family symbol in any GC file.

## 10. What is done well

1. **The shared edge-enumeration primitive** (`gc_visit.rs`) — refusing to add
   a fifth kind-dispatch table and instead threading the GC visitor through the
   exact functions the destructive Drop path calls
   (`for_each_heap_child_edge`, `for_each_typed_array_elem_ptr`,
   `closure_immutable_heap_capture_edge`, `gc_payload_edge`) is the single best
   structural decision in this vertical: it converts the historically-feared
   lockstep hazard into a can't-drift property, and the parity tests prove
   read-set == release-set by witness refcounts rather than by assertion of
   intent.
2. **Precheck-before-release barrier shape** (`gc_decrement_precheck` returning
   the survivor *before* the decrement, `gc.rs:413-431`) — keeps the rc==1 fast
   path byte-identical and makes "the barrier never touches a freed object"
   locally provable at the call site.
3. **No-root-scan collector choice.** Bacon–Rajan was chosen specifically
   because external roots surface as refcount residue — the collector never
   walks the stack, so the zero-tag slot design and the Forbidden-Patterns
   discipline survive GC intact. The design doc's constraint table (§2) shows
   the reasoning; the code honors it (§9.10).
4. **Leak-safe failure direction everywhere** — `FreeKind::Leak`, conservative
   `u32::MAX/2` seeds, bounded-abort STW, buffered-entry removal on RC free.
   The system's every unknown degrades to "collect it later or never", not to
   UB.
5. **Memory-only Drop semantics tested head-to-head** against the RC path on
   the same object shape (`cycle_member_skips_drop_of_owned_field`) — the
   ratified §0 #1 semantic is pinned by a test, not a comment.
6. **`Weak`-witness assertions** for full-reclaim without UAF in tests
   (`weak_of`, `gc.rs:1984-1991`) — a pattern worth propagating repo-wide.
7. **Feature hygiene**: the entire GC is `#[cfg(feature="gc")]`-gated to
   nothing when off (module level, `lib.rs`), and the flip to default-on was
   done at the shipped-binary crate with an explicit two-tier forward
   (`shape-cli/Cargo.toml:26`) — the known vm-vs-jit flip hazard was handled.
8. **The `UnsafeCell` slot-cells migration** (`heap_value.rs:3995-4020`) — a
   Miri-driven aliasing fix executed with a deliberate rename so every stale
   reader broke at compile time, plus a provenance sidecar to keep Miri runs
   meaningful.
9. **Design-doc ↔ code traceability**: ratification stamps, per-phase gates,
   and section-numbered code comments make this the most auditable subsystem
   in the repo; my conformance table (§6.3) was checkable in hours rather than
   days because of it.
10. **The readiness report's honesty** — it disclosed its own two caveats
    (teardown panic since resolved; bare-form runtime failure) that a
    less-adversarial lane would have buried; both reproduced exactly as
    described.

## 11. What is done poorly / tech debt

1. **`heap_value.rs` (8,142 LOC)** — TypedObjectStorage, TraitObjectStorage and
   a dozen unrelated `*Data` payload types in one file; the Drop glue for the
   whole heap family lives 4,000 lines from the types it manages. Splitting
   along carrier families would cut review risk on every future kind addition.
2. **The dual-lifecycle allocator pair** (§9.2) — transitional since Wave 2
   (2026-05-14) and still open; the longer it lives the more provenance-only
   contracts accrete on top.
3. **Legacy GC API stubs** (`GCIntegration`, `memory.rs::GarbageCollector`,
   dead `GCConfig` knobs the design §8 planned to repurpose) — inert but
   report-fiction-producing (§9.6).
4. **v1 `heap_header.rs`** — a same-named parallel header type with zero
   runtime carriers, kept alive as doc surface; already caused one near-miss
   (design §3.1 CORRECTED note).
5. **Comment-mass vs mechanism**: ordinal-history essays inside
   `heap_variants.rs` and per-amendment archaeology inside `native_kind.rs` do
   drift-prevention work that sentinel tests would do better (ADR-005
   §Visibility item 4 — the "optional" sentinel variant-count test — appears
   not to exist; adding it would let half the prose retire).
6. **Cascade special case** (§9.5) — bespoke topology handling inside the one
   function that most needs to stay boring.
7. **Stale feature-comment drift** ("Default OFF" ×3, `gc.rs:4` module doc) and
   the CLAUDE.md NativeKind list — cheap fixes, real confusion for every new
   agent (this audit initially trusted the CLAUDE.md list).
8. **`Ptr(HeapKind::Char)` vs `NativeKind::Char` parallel carriers** — the
   promised fold-in (`native_kind.rs:66-70`) has no owner or date.
9. **No end-to-end GC regression in the test tree** (§7.3.1) — the flagship
   guarantee is currently protected by Rust unit tests + an unversioned
   scratch-program procedure.

## 12. Prioritized recommendations

**P0 (do before the next release tag)**

1. Add the annotated Finding-#31 Shape program to `shape-test` with an RSS or
   candidate-count bound (effort: S — the program exists in the readiness
   report; the harness needs one RSS probe helper). Protects the vertical's
   headline guarantee mechanically.
2. Fix the three "Default OFF" Cargo comments + `gc.rs:4` module doc + the
   CLAUDE.md NativeKind variant list (effort: XS). Doc-only but
   agent-misleading at the project's stated multi-agent operating model.
3. Log the 3c-iii/Phase-5 ruling deviation (gc default-on with `HK_JIT_OBJECT`
   tag unresolved) in `docs/defections.md` or get an explicit re-scope from the
   owner (effort: XS). Process debt, not code.

**P1 (next wave)**

4. Retire the dual allocation discipline: newtype `_new`-provenance pointers
   end-to-end (extend the `TypedObjectPtr` pattern), delete or privatize the
   by-value `TypedObjectStorage::new`/Arc lifecycle, same for
   `TraitObjectStorage` (effort: M — call-site migration is mostly mechanical;
   the W5 history shows the payoff).
5. Convert the collector's four recursive walks to explicit-worklist iteration
   (effort: S–M; `collect_white`'s deferred-free vector is already half the
   pattern).
6. Enumerate TraitObject children in `for_each_heap_child` (its data half is an
   `Arc<TypedObjectStorage>` — one edge) and add the corresponding parity test
   (effort: S). This closes the cheapest completeness gap.
7. Wire `GCIntegration::force_gc`/`gc_stats` to the real collector or delete
   the trait (effort: S).
8. Resolve 3c-iii by lowering `HK_JIT_OBJECT` stores to TypedObject (the
   design's own recommendation) rather than adding a per-value kind track
   (effort: M, JIT-vertical coordination).

**P2 (opportunistic)**

9. Header-less collection (design §3.5 end state): buffer SharedCell/container
   survivors through the side table and release White Arc-backed nodes
   per-kind, retiring the closure-cascade special case in the same stroke
   (effort: L — this is the natural "GC v1.1" workstream and removes findings
   9.3.1 and 9.5 together).
10. Add a `HEAP_KIND_V2_*` stamp debug-assert inside `GcMeta::Header` accessors
    (effort: XS) to defuse the `gc_meta` dual-carrier latency (§9.7).
11. Add the ADR-005 sentinel variant-count test; amend ADR-006 §2.7.6/Q8 to
    bless the v2-raw per-carrier accessor (`as_typed_object_storage`) so code
    and rule agree (effort: XS–S).
12. Book chapter: "Reference cycles and the collector" — memory-only semantics,
    Drop-on-cycles doesn't run, escape-promoted references (effort: S, docs
    vertical, gate-runnable example required by the project's own book gate).
13. Debug-assert "candidate buffer empty at VM start" (effort: XS) for the
    pooled-thread hygiene invariant (§9.8).
14. Split `heap_value.rs` along carrier families (effort: M, mechanical,
    high review-leverage).

---

## Appendix A — Full empirical transcripts

All runs: `target/debug/shape run <file>` on the working tree, 2026-07-11.
The two `libshape_ext_*` extension-load lines are filtered per the audit brief.

### A.1 Finding #31 — bare/statement-scope annotated form (FAILS)

```shape
// f31_small.shape
var total = 0
for i in 0..100000 {
    var arr: Array<() -> int> = []
    arr.push(|| arr.len())
    total = total + 1
}
print(f"done {total}")
```

```
[jit-fallback] function main failed JIT compile: … Rvalue::Aggregate reached the
kind-blind fallback … Tracked as W11-jit-new-array …; running under interpreter
Error: Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data enum + … DELETED
across V3-S5 ckpt-1..ckpt-4 … Construction-site rebuild lands at ckpt-6 STRICT
close … (line 4)
MAX_RSS_KB=1924   (process died before the loop ran)
```

### A.2 Finding #31 — fn-body annotated form (WORKS, RSS BOUNDED)

```shape
// f31_fn_{small,large}.shape
fn leak() {
    let mut arr: Array<() -> int> = []
    arr.push(|| arr.len())
}
for i in 0..N { leak() }      // N = 200_000 / 1_000_000
print("done")
```

```
f31_fn_small (N=200,000):   exit=0  MAX_RSS_KB=74008
f31_fn_large (N=1,000,000): exit=0  MAX_RSS_KB=74004
```

Both runs also printed the JIT deopt line:
`[jit-fallback] … V2 typed opcode TypedArrayPushCallable at offset 2010 in
function '__closure_0' has no FrameDescriptor … falling through to bytecode
interpreter` — i.e. the bounded-RSS proof exercises the interpreter tier's
barriers + safepoint only (§2.2 negative result 2).

Interpretation: 5× iterations, ΔRSS = −4 KB. With gc-off the readiness lane
measured ~190 bytes/iteration growth (69→781 MB across 200k→4M); a debug-build
leak at that rate over the extra 800k iterations would have added ~150 MB here.
The collector is reclaiming the per-iteration cycle.

### A.3 Drop / RAII / deferral

```shape
type Res { name: string }
impl Drop for Res {
    fn drop() { print(f"drop {self.name}") }
}
fn scoped() {
    let a = Res { name: "in-fn" }
    print("before fn end")
}
scoped()
print("after fn call")
let m = Res { name: "module-scope" }
print("end of program")
```

```
[jit-fallback] … Wave-20A user-trait-method JIT SURFACE … Whole-program
deopting to the bytecode interpreter …; running under interpreter
before fn end
drop in-fn
after fn call
end of program
drop module-scope
```

Also observed: `fn drop(self)` rejected with `Semantic error: Method 'drop'
has an explicit `self` parameter, but method receivers are implicit.`

### A.4 Object-graph cycle attempt (no RC cycle forms at the language level)

```shape
type Node { name: string, next: Option<Node> }
impl Drop for Node { fn drop() { print(f"drop {self.name}") } }
fn acyclic() { let a = Node { name: "solo", next: None } ... }
fn cyc() {
    let mut a = Node { name: "a", next: None }
    let b = Node { name: "b", next: Some(a) }
    a.next = Some(b)
    print("cycle built")
}
```

```
acyclic scope ending
drop solo
after acyclic
cycle built
drop b
drop a
after cyc
```

Both Drops fire at scope exit ⇒ the field stores did not alias into an RC
cycle (CoW/value semantics on the object store) — evidence for §2.4's
conclusion that closures are effectively the only user-reachable cycle
constructor today. Adjacent-vertical observations from the same session:
`next: null` inside an object literal is a parse error (`None` required), and
a `Node?` field annotation rejects `None` ("Option\<Node\> is not compatible
with Node") while `Option<Node>` accepts it.

### A.5 Reference escape (§2.7.30 narrow floor)

```shape
fn make_ref() -> &int {
    let mut local = 42
    return &local
}
let r = make_ref()
print(f"escaped ref reads {r}")
let mut x = 7
let rx = &x
print(f"module-scope ref reads {rx}")
```

```
escaped ref reads 42
module-scope ref reads 7
```

(Note: `*r` deref syntax is rejected — references auto-deref in interpolation.)

## Appendix B — Second-pass independent verification (same day, independent session)

A second auditor pass re-derived this vertical from scratch (code reading +
fresh empirical runs with the same prebuilt `target/debug/shape`, RSS via GNU
`time -v`) before reading this report, then reconciled. Confirmations and
three NEW findings follow. All programs live in the session scratchpad
(`verticals/value-memory-gc/`).

### B.1 Confirmations of the first-pass load-bearing claims

- **Finding #31 fn-scoped form is RSS-bounded** — independently reproduced at
  three scales (debug build, `Maximum resident set size` from `time -v`):

  | N (iterations) | peak RSS (KB) |
  |---:|---:|
  | 250,000 | 75,420 |
  | 1,000,000 | 74,416 |
  | 2,000,000 | 75,284 |

  Flat within noise across an 8× span. The same run reconfirmed both negative
  results: the statement-scope `var arr = []` / `var arr: Array<() -> int> = []`
  forms die at `op_new_array(0)` V3-S5 SURFACE (even with the annotation, even
  as `let mut`, when at top level), and the workload's closure fails JIT
  verification (`TypedArrayPushCallable … no FrameDescriptor`) so all measured
  collection ran in the interpreter tier.
- **Drop/RAII order** — reconfirmed: reverse-declaration-order drops at scope
  exit (`drop b` then `drop a`), module-scope binding's Drop deferred to
  program end (`drop module` printed after `end`).
- **Phase 2/3a/3b/4 wiring** — re-traced from scratch to the same sites:
  dispatch-loop safepoint `maybe_collect(256)` at `dispatch.rs:205/372` under
  the `& 0x3FF` gate; barriers in `stack.rs:618/638/976`,
  `closure_raw.rs:1705/1731/1742`, `heap_value.rs:3092-3110` (`MutexData::set`),
  `v2_array_detect.rs:806-868`; rc→0 buffer-hygiene hooks at
  `typed_array.rs:778`, `heap_value.rs:3581`, `heap_value.rs:4978`; teardown
  sweep + thread-local buffer clear at `executor/mod.rs:874-878`; JIT flag
  aliasing the coordinator's `stop_requested` byte at `context.rs:718`.
- **Lockstep discipline is real, not aspirational** — `drop_fields` and the GC
  visitor both consume the single `for_each_heap_child_edge` enumeration
  (`heap_value.rs:4476-4545`); `SharedCell::gc_payload_edge` returns the same
  `(bits, kind)` pair `Drop for SharedCell` dispatches on
  (`closure_layout.rs:273-278`).
- **Stale-doc confirmations** — `shape-jit/src/ffi/gc.rs:9-10` still opens with
  "No-op stubs … no tracing collector exists" above a real barrier + park
  implementation; `shape-value/Cargo.toml:28` still says "Default OFF" under a
  workspace where `shape-vm` and `shape-cli` both ship `default = ["jit","gc"]`.

### B.2 NEW P1 — ordinary `var`-bound object cycles leak linearly (Option-mediated)

The first pass's A.4 concluded that `let mut` + field stores do not alias into
an RC cycle (CoW value semantics — both Drops fired) and inferred closures are
effectively the only user-reachable cycle constructor. That inference does not
survive `var` bindings, which take the interior-mutation storage class:

```shape
type Node {
    next: Option<Node>
}
var i = 0
while i < 1000000 {
    var a = Node { next: None }
    var b = Node { next: None }
    a.next = Some(b)
    b.next = Some(a)      // back-edge — 4-node cycle a → OptionData → b → OptionData → a
    i = i + 1
}
print("done")
```

Measured peak RSS (debug binary, gc ON by default):

| variant | 500k iter | 1M iter |
|---|---:|---:|
| cyclic (both edges) | 324,852 KB | 576,100 KB |
| acyclic control (`a.next = Some(b)` only) | — | 77,656 KB |

Linear ≈ 500 B/iteration growth with the back-edge; flat without it. The leak
is exactly the header-less completeness tail (first-pass finding #2)
manifesting through the *most ordinary* cycle shape a user can write: the
`next` field's kind is `Ptr(HeapKind::Option)` (an `Arc<OptionData>`), which
(a) `cycle_capable_direct_header` refuses to buffer (`gc.rs:344-354` admits
only TypedObject/TypedArray/TraitObject), and (b) `GcNode::child_heapkind`
never traces through (`gc.rs:669-677` — TypedObject/TypedArray/SharedCell
only), so even the TypedObject members that DO get buffered by their own
field-overwrite decrements can never see across the OptionData hop —
`arc_strong_count_seed` seeds `Ptr(Option)` at `u32::MAX / 2` ("never White",
`gc.rs:802`). Since `next: Node?` does not parse in field position and
`next: Node` cannot bootstrap a cycle, **every user-expressible object-graph
cycle routes through `Option<T>` and therefore leaks** under the shipped
collector. Severity P1: the GC's flagship purpose (Finding #31 class) is met
for the closure topology but not for the plain data-structure topology
(doubly-linked list, parent/child back-pointers — the shapes users actually
build). Fix path: add `HeapKind::Option`/`Result` payload edges to
`for_each_heap_child` (they carry a `KindedSlot` payload, same shape as the
`SharedCell` arm) and admit buffering through them, or fold Option into a
header carrier.

### B.3 NEW P1 — §2.7.30 ReturnSlot promotion broken for user-defined referents

First-pass §2.4/A.5 validated the reference-escape narrow floor on `&int`.
Re-testing with a user type (with or without `impl Drop`):

```shape
type P { x: int }
fn make() -> &P {
    let p = P { x: 1 }
    return &p
}
let r = make()
```

```
error[RUNTIME]: Bytecode compilation failed: Semantic error: Could not solve type constraints:
  &P is not compatible with &P
  --> <input>:2:4
```

`-> &int` and `-> &string` both work; any user TypedObject referent fails, with
a self-contradictory diagnostic ("`&P is not compatible with &P`"). §6's
"§2.7.30 CONFORMS (empirical)" should read: **conforms for primitive
referents; the ReturnSlot flip is unusable for user types** (likely a
type-identity comparison bug where two distinct `Type` instances for `P` fail
to unify under `&`). P1 — the promotion feature exists precisely so references
to *resources* (user objects with Drop) can escape.

### B.4 NEW P2 — two Drop impls retire heap shares without the decrement barrier

The Phase-2 decrement barrier lives in the `drop_with_kind` wrappers
(`stack.rs:638/976`, `closure_raw.rs:1731-1742`) and in `MutexData::set`
(`heap_value.rs:3092-3110`). But the two canonical Drop impls that retire heap
shares directly bypass it:

- `impl Drop for SharedCell` (`closure_layout.rs:376-395`) — reads the payload
  bits and calls `Arc::decrement_strong_count` / `release_elem` per kind, with
  no `gc_decrement_precheck`/`gc_buffer_possible_root` pair.
- `impl Drop for KindedSlot` (`kinded_slot.rs:947+`) — the per-kind release
  table (which `ResultData`/`OptionData`/`MutexInner` payload drops and every
  owned-`KindedSlot` container flow through) likewise has no barrier.

Consequence: a decrement-to-nonzero delivered through either path never
buffers the surviving carrier as a Purple candidate. This is leak-only (a
missed possible-root defers/loses a collection; it can never cause a premature
free), and for OptionData it is currently masked by B.2's larger gap — but it
becomes load-bearing the moment header-less kinds are made traceable, so it
belongs on the same fix ticket. The asymmetry also contradicts the design's
"§3.2 barriers wire into existing hooks — the general `drop_with_kind`
decrement" framing: `KindedSlot::Drop` IS a general decrement hook, and it is
unbarriered.

### B.5 Reconciliation notes

- First-pass findings #1-#12 all re-verified as stated (no retractions).
- A.4's conclusion is narrowed by B.2 (holds for `let`-CoW stores only); §2.3's
  "NOT collectable" list should explicitly include `Option`/`Result`-mediated
  cycles — with the note that these are the *default* object-cycle shape, which
  raises the tail from "documented deferral" to P1 user-facing leak.
- §6's §2.7.30 row is downgraded per B.3.

---

*Methodology note: all `file:line` cites are against the dirty working tree as
of 2026-07-11. Empirical transcripts were produced with
`target/debug/shape` (prebuilt from this tree); RSS measured by polling
`/proc/<pid>/status` VmRSS at 100–200 ms (no GNU time on host). No project
files were modified; scratch programs live under the session scratchpad
(`verticals/value-memory-gc/`). Zero cargo invocations were needed — the
existing binary plus source reading covered all claims.*

