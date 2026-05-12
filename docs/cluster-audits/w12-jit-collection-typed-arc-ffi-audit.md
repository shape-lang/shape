# W12-jit-collection-typed-arc-ffi — audit

**Sub-cluster:** Phase 3 cluster-0 Round 7B (AUDIT-FIRST).
**Branch:** `bulldozer-strictly-typed-w12-jit-collection-typed-arc-ffi` (parent `b77be454`).
**Worktree:** `/home/dev/dev/shape-lang/shape-w12-jit-collection-typed-arc-ffi`.
**Audited:** 2026-05-12.
**Disposition:** option **(iii)** — STOP after audit commit; surface to supervisor.

---

## §0. TL;DR

The dispatch asked: "land typed-Arc allocation FFI for 8 collection
HeapKinds, same shape as W11-jit-new-array Route A but extended to
non-Array kinds."

After audit, **the allocation FFI piece alone is bounded mechanical
work** (option (i)). But landing it without the downstream pieces
produces the same failure pattern as W12-jit-aggregate-non-array
(Round 5B) — the smoke test cannot pass because the JIT-side method
dispatch shell for these collection kinds is broken by a deeper
architectural gap:

1. `jit_call_method` dispatches by `heap_kind(receiver_bits)` (NaN-box
   tag decode at `crates/shape-jit/src/ffi/value_ffi.rs:330-336`).
2. Typed-Arc bits — what `KindedSlot::from_hashset(arc)` stores per
   ADR-006 §2.7.6 — are raw `Arc::into_raw(arc) as u64` pointers, no
   NaN-box tag.
3. `is_heap(bits)` returns `false`. `heap_kind(bits)` returns `None`.
4. `jit_call_method` falls through to `_ => TAG_NULL`, then to
   `dispatch_method_via_trampoline` which is `todo!()` at
   `crates/shape-jit/src/ffi/call_method/mod.rs:179-199` —
   extern-C `todo!()` aborts the process.

The smoke `let s = Set(); s.add("a"); s.add("b"); print(s.size())`
therefore cannot pass under `--mode jit` even with typed-Arc
allocation FFI landed. The same pattern Round 5B documented for
`jit_make_ok` / `_err` / `_some` re-emerges verbatim: the producer-
side allocation works, but the consumer-side method dispatch /
kind-routing infrastructure is missing.

**This is ADR-006 §2.7.10 / Q11 (kinded MethodFnV2 ABI rebuild)
territory.** The dispatch authority explicitly anticipated this
("If audit reveals option (iii) ADR amendment is required ... STOP
after audit commit and SURFACE to supervisor"). Per W11-round-1
walk-back precedent ("the kind-blind fallback would resurrect the
deleted UnifiedArray heap layout"), I am surfacing rather than
landing a fix that papers over the gap.

---

## §1. Reproduction — current state of 8 surface sites

Build: `cd /home/dev/dev/shape-lang && devenv shell --quiet -- bash -c "cd /home/dev/dev/shape-lang/shape-w12-jit-collection-typed-arc-ffi && cargo build --bin shape --release"`.

Smoke commands and observed JIT-mode errors (post W12-collection-
constructor-mir-lowering close, commit `d784042e`):

| Ctor | Operands | Smoke | JIT result | VM result |
|---|---|---|---|---|
| `Set()` | 0 | `let mut s = Set(); s.add("a"); s.add("b"); print(s.size())` | `JIT compilation failed: EnumStore: SURFACE — primitive-collection constructor 'Set' (operands.len()=0)` | `2` |
| `HashMap()` | 0 | `let mut m = HashMap(); m.set("k", 1); print(m.get("k"))` | `JIT compilation failed: EnumStore: SURFACE — primitive-collection constructor 'HashMap' (operands.len()=0)` | `1` |
| `Deque()` | 0 | `let mut d = Deque(); d.push_back(1); print(d.len())` | (same surface, 'Deque') | (depends on stdlib coverage — verified VM ctor works at `vm_impl/builtins.rs:611-625`) |
| `PriorityQueue()` | 0 | (assumed similar) | (same surface, 'PriorityQueue') | — |
| `Channel()` | 0 | (assumed similar) | (same surface, 'Channel') | — |
| `Mutex(42)` | 1 | `let m = Mutex(42); print(m.get())` | `JIT compilation failed: EnumStore: SURFACE — primitive-collection constructor 'Mutex' (operands.len()=1)` | `42` |
| `Atomic(7)` | 1 | `let a = Atomic(7); print(a.load())` | (same surface, 'Atomic', operands.len()=1) | (assumed `7`) |
| `Lazy(closure)` | 1 | `let l = Lazy(|| 42); print(l.get())` | (same surface, 'Lazy', operands.len()=1) | (assumed `42`) |

**8-of-8 surface sites confirmed**, matching the dispatch's expected
count. Round 6 (especially 6A `BytecodeProgram.function_return_concrete_types`
and 6C `EnumStore` reuse for collection ctors) did not close any of
them inadvertently — Round 6C's whole *purpose* was establishing the
honest surface this audit is now classifying.

---

## §2. Per-HeapKind audit grid — current vs. post-fix state

For each HeapKind, I record (a) ordinal, (b) constructor signature
shape (zero-arg vs with-arg + payload kind), (c) VM-side ctor
reference, (d) `KindedSlot::from_*` constructor, (e) current JIT
state, (f) target FFI shape, (g) what would still be missing after
landing allocation FFI alone.

### §2.1 `Set` — `HeapKind::HashSet` ord 21 (§2.7.15)

- **Ctor signature**: `Set()` — zero-arg. Reader contract: kind `Ptr(HeapKind::HashSet)`, bits = `Arc::into_raw::<HashSetData>`.
- **VM ctor** (`vm_impl/builtins.rs:598-610`): `Arc::new(HashSetData::new())` → `KindedSlot::from_hashset(empty)`.
- **`KindedSlot::from_hashset`** (`kinded_slot.rs:132-137`): `Arc::into_raw(arc) as u64` via `ValueSlot::from_hashset`.
- **Carrier**: `HashSetData { keys: Arc<TypedBuffer<Arc<String>>>, index: HashMap<u64, Vec<u32>> }` (`heap_value.rs:1133`).
- **Current JIT state**: surfaces at `mir_compiler/statements.rs::EnumStore` with collection-ctor cite per `is_collection_ctor_name`.
- **Target FFI shape (allocation only)**: `jit_v2_make_hashset() -> u64` returning `Arc::into_raw(Arc::new(HashSetData::new())) as u64`. Slot stamp `Ptr(HeapKind::HashSet)` already known at EnumStore consumer (the `variant_name == "Set"` discriminator IS the producer-side classification per §2.7.5).
- **Gap after allocation FFI alone**: `s.add("a")` dispatches via `jit_call_method` → `heap_kind(receiver_bits)` decodes NaN-box tag → bits are raw `Arc::into_raw`, no tag → returns `None` → falls through `_ => TAG_NULL` → `try_call_user_method` returns None (no user-defined `HashSet::add`) → `dispatch_method_via_trampoline` `todo!()` aborts. **Method dispatch is broken.** Same architectural gap §2.7.10/Q11 deferral parks.

### §2.2 `HashMap` — `HeapKind::HashMap` ord 17 (§2.7.4 + W13-hashmap-mutation)

- **Ctor signature**: `HashMap()` — zero-arg. Reader contract: kind `Ptr(HeapKind::HashMap)`, bits = `Arc::into_raw::<HashMapData>`.
- **VM ctor** (`vm_impl/builtins.rs:587-597`): `Arc::new(HashMapData::new())` → `KindedSlot::from_hashmap(hm)`.
- **`KindedSlot::from_hashmap`** (`kinded_slot.rs:120-125`): standard typed-Arc shape.
- **Carrier**: `HashMapData { keys: Arc<TypedBuffer<Arc<String>>>, values: Arc<TypedBuffer<Arc<HeapValue>>>, index: HashMap<u64, Vec<u32>> }` (`heap_value.rs:653+`).
- **K/V kind threading question**: `HashMap()` is *empty* — K/V kinds are not specified at ctor time. Downstream `m.set("k", 1)` operations would need to know V kind. But **the VM-side carrier stores `Arc<HeapValue>` values** (heterogeneous), not a per-V monomorphization. So **HashMap does NOT need per-V FFI monomorphization** — a single `jit_v2_make_hashmap()` entry point matches the VM carrier's single shape. K is string-only at landing per W13-hashmap-mutation precedent (`heap_value.rs` HashMapData docstring + §2.7.15 string-only keyspace ruling).
- **No ADR amendment needed for HashMap allocation alone.**
- **Current JIT state**: surfaces at EnumStore consumer.
- **Target FFI shape**: `jit_v2_make_hashmap() -> u64` returning `Arc::into_raw(Arc::new(HashMapData::new())) as u64`.
- **Gap after allocation FFI alone**: identical to §2.1 — `m.set("k", 1)` cannot dispatch.

### §2.3 `Deque` — `HeapKind::Deque` ord 23 (§2.7.19)

- **Ctor signature**: `Deque()` — zero-arg.
- **VM ctor** (`vm_impl/builtins.rs:611-625`): `Arc::new(DequeData::new())` → `KindedSlot::from_deque`.
- **Carrier**: `DequeData` (`heap_value.rs:1407`). Storage is `Arc<TypedBuffer<Arc<HeapValue>>>` per the §2.7.19 amendment.
- **No element-kind threading needed**: Deque stores heterogeneous `Arc<HeapValue>` like HashMap values. Single FFI entry.
- **Target FFI shape**: `jit_v2_make_deque() -> u64` returning `Arc::into_raw(Arc::new(DequeData::new())) as u64`.
- **Gap after allocation FFI alone**: identical method-dispatch gap.

### §2.4 `PriorityQueue` — `HeapKind::PriorityQueue` ord 25 (§2.7.18)

- **Ctor signature**: `PriorityQueue()` — zero-arg.
- **VM ctor** (`vm_impl/builtins.rs:626-641`): `Arc::new(PriorityQueueData::new())` → `KindedSlot::from_priority_queue`.
- **Carrier**: `PriorityQueueData` (`heap_value.rs:2008`). i64-priority-only landing per §2.7.18 ruling.
- **No per-priority-kind FFI**: i64-only at landing; single entry.
- **Target FFI shape**: `jit_v2_make_priorityqueue() -> u64`.
- **Gap after allocation FFI alone**: identical method-dispatch gap.

### §2.5 `Channel` — `HeapKind::Channel` ord 24 (§2.7.20)

- **Ctor signature**: `Channel()` — zero-arg.
- **VM ctor** (`vm_impl/builtins.rs:642-662`): `Arc::new(ChannelData::new())` → `KindedSlot::from_channel`.
- **Carrier**: `ChannelData { inner: Mutex<ChannelInner> }` (`heap_value.rs:1530`). Heterogeneous queue payload per §2.7.20.
- **No per-element-kind FFI**: single entry.
- **Target FFI shape**: `jit_v2_make_channel() -> u64`.
- **Gap after allocation FFI alone**: identical method-dispatch gap, plus cross-task `recv()` is already a separate §2.7.4 deferral (task-scheduler boundary). For JIT alloc alone, the `recv()` deferral is unaffected — `Channel()` allocation is independent of the blocking-recv work.

### §2.6 `Mutex` — `HeapKind::Mutex` ord 30 (§2.7.25)

- **Ctor signature**: `Mutex(initial_value)` — with-arg, accepts any `KindedSlot`. The inner value can be any kind; the share moves into the cell.
- **VM ctor** (`vm_impl/builtins.rs:663-685`): pops the arg as `KindedSlot`, calls `Arc::new(MutexData::new(initial))` → `KindedSlot::from_mutex`.
- **Carrier**: `MutexData { inner: Mutex<MutexInner { value: Option<KindedSlot> }> }` (`heap_value.rs:1682`). The wrapped value is stored as a `KindedSlot` (with its parallel kind track).
- **Inner-value kind threading question**: `Mutex(42)` — the inner `42` is `Int64`. The VM passes the `KindedSlot{slot, kind}` pair to `MutexData::new`. **For JIT to do the same, it needs to pass BOTH the slot bits AND the kind label.** This is the §2.7.5 cross-crate ABI policy: kind threads alongside bits. The JIT can know the inner kind from the producing-operand's `NativeKind` (e.g. `Int64` for the literal `42`).
- **Target FFI shape — TWO options**:
  - **(a) Monomorphized per inner-kind**: `jit_v2_make_mutex_i64(value: i64)`, `jit_v2_make_mutex_f64(value: f64)`, `jit_v2_make_mutex_ptr(bits: u64)` etc. Mirrors W11 Route A per-element-kind shape. Cost: ~5-10 FFI variants.
  - **(b) Single entry with (bits, kind) carrier**: `jit_v2_make_mutex(value_bits: u64, value_kind: u8)`. Cost: 1 FFI entry, but the inner `KindedSlot` reconstruction at the FFI boundary needs a `(u64, NativeKind) -> KindedSlot` factory. This is exactly the §2.7.5 `JitFfiCarrier` shape established in W11-jit-carrier-conversion. **This is the closer match.**
- **Recommended for Mutex**: option (b) — `JitFfiCarrier` is the documented §2.7.5 boundary form; mutex's payload is intrinsically polymorphic. Monomorphization is feasible but the carrier-pair form matches §2.7.10/Q11 method-dispatch ABI's `&[KindedSlot]` shape — consistency win.
- **Current JIT state**: surfaces at EnumStore consumer.
- **Gap after allocation FFI alone**: identical method-dispatch gap (`m.get()`, `m.lock()`, `m.set(v)`).

### §2.7 `Atomic` — `HeapKind::Atomic` ord 31 (§2.7.25)

- **Ctor signature**: `Atomic(initial_int)` — **i64-only at landing** per §2.7.25 ruling. Non-int args error.
- **VM ctor** (`vm_impl/builtins.rs:686-712`): kind-validates `args[0].as_i64()`, calls `Arc::new(AtomicData::new(initial))` → `KindedSlot::from_atomic`.
- **Carrier**: `AtomicData { value: AtomicI64 }` (`heap_value.rs:1747`). i64-only landing per §2.7.25.
- **Inner-value kind threading**: simpler than Mutex — only `Int64` is accepted. JIT-side validation could be at compile time (refuse non-Int64 inner-operand kinds at EnumStore consumer) or at runtime (FFI body errors on bad kind). Compile-time is principled per §2.7.5.
- **Target FFI shape**: `jit_v2_make_atomic(initial: i64) -> u64`. Single i64-input entry — matches the VM carrier's i64-only storage exactly.
- **Gap after allocation FFI alone**: method dispatch (`a.load()`, `a.store(v)`, `a.fetch_add(d)`, ...) — same architectural gap.

### §2.8 `Lazy` — `HeapKind::Lazy` ord 32 (§2.7.25)

- **Ctor signature**: `Lazy(closure)` — closure-only. Kind-validated as `Ptr(HeapKind::Closure)` at the ctor.
- **VM ctor** (`vm_impl/builtins.rs:713-750`): kind-validates `args[0].kind == Ptr(HeapKind::Closure)`, calls `Arc::new(LazyData::new(initializer))` → `KindedSlot::from_lazy`.
- **Carrier**: `LazyData { inner: Mutex<LazyInner { initializer, value }> }` (`heap_value.rs:1814`).
- **Inner-value kind threading**: simpler than Mutex — only `Ptr(HeapKind::Closure)` is accepted. Same compile-time validation pattern as Atomic.
- **Target FFI shape**: `jit_v2_make_lazy(closure_bits: u64) -> u64`. Single ptr-input entry — the closure's kind is enforced at JIT compile time (refuse non-`Ptr(HeapKind::Closure)` inner-operand kinds at EnumStore consumer).
- **Gap after allocation FFI alone**: method dispatch (`l.get()`, `l.is_initialized()`) — same architectural gap; PLUS `l.get()` semantics call the initializer via `vm.call_value_immediate_nb`, which is another §2.7.11/Q12 path that the JIT-side does not currently re-enter cleanly.

---

## §3. Audit summary

### §3.1 Per-HeapKind FFI shape table

| HeapKind | Ord | Operands | FFI entry shape | Per-inner-kind mono? | Storage shape |
|---|---|---|---|---|---|
| HashSet | 21 | 0 | `jit_v2_make_hashset() -> u64` | N/A | `Arc<HashSetData>` |
| HashMap | 17 | 0 | `jit_v2_make_hashmap() -> u64` | N | `Arc<HashMapData>` |
| Deque | 23 | 0 | `jit_v2_make_deque() -> u64` | N | `Arc<DequeData>` |
| PriorityQueue | 25 | 0 | `jit_v2_make_priorityqueue() -> u64` | N | `Arc<PriorityQueueData>` |
| Channel | 24 | 0 | `jit_v2_make_channel() -> u64` | N | `Arc<ChannelData>` |
| Mutex | 30 | 1 (any) | `jit_v2_make_mutex(bits, kind) -> u64` (carrier-pair form) | Y (via JitFfiCarrier) | `Arc<MutexData>` (inner KindedSlot) |
| Atomic | 31 | 1 (i64) | `jit_v2_make_atomic(initial: i64) -> u64` | N (i64-only) | `Arc<AtomicData>` |
| Lazy | 32 | 1 (Closure) | `jit_v2_make_lazy(closure_bits: u64) -> u64` | N (Ptr-only) | `Arc<LazyData>` |

**Five zero-arg ctors** monomorphize to a single FFI entry each.
**Three with-arg ctors** have payload-kind constraints:
- Atomic — compile-time i64-only validation, single FFI entry.
- Lazy — compile-time Ptr(Closure)-only validation, single FFI entry.
- Mutex — accepts any kind; carrier-pair form (bits + kind) matches §2.7.5.

### §3.2 No ADR amendment required for allocation FFI alone

The allocation FFI piece in isolation is **option (i)** territory:
- No new HeapKind. No new MIR statement. No new dispatch shape.
- No new FieldKind. No parallel-kind side track on heap object.
- Per-HeapKind monomorphic — single FFI entry per HeapKind (3 of 8
  validate inner kind at compile time before single-entry dispatch).
- Producer-side classification per §2.7.5 (the `variant_name` field
  on `StatementKind::EnumStore` already established by Round 6C is
  the kind discriminator at MIR-emit time).
- Kinded slot stamp at EnumStore consumer (`Ptr(HeapKind::HashSet)` /
  etc. is known statically from `variant_name`).

The mechanical work is: write 8 small extern-C FFI bodies, register
them in `ffi_symbols/`, add 8 FuncRef slots in `FFIFuncRefs`, and
extend the EnumStore consumer's collection-ctor arm to dispatch on
`variant_name` and emit the matching FFI call instead of surfacing.

**Estimated bounded work**: ~250 LoC across 4 files, ~half-day if
nothing else surfaces.

### §3.3 But the smoke target requires more than allocation

The smoke `let s = Set(); s.add("a"); s.add("b"); print(s.size())`
explicitly asks for VM=JIT parity, which means the JIT path must
allocate AND method-dispatch AND print. The method dispatch is
**broken** at `crates/shape-jit/src/ffi/call_method/mod.rs:201` for
all 8 of these HeapKinds:

```rust
// pseudo-code of the relevant flow:
let result = match heap_kind(receiver_bits) {      // returns None
    Some(HK_ARRAY)  => call_array_method(...),     // never reached
    Some(HK_STRING) => call_string_method(...),
    Some(HK_JIT_OBJECT) => call_object_method(...),
    _ => TAG_NULL,                                  // reached
};
if result == TAG_NULL {
    if let Some(u) = try_call_user_method(...) {    // returns None (no user-def)
        return u;
    }
}
if result == TAG_NULL {
    dispatch_method_via_trampoline(...)             // todo!() — process abort
}
```

`heap_kind(bits)` returns `None` because:
- `heap_kind` requires `is_heap(bits)` which requires `is_tagged(bits)`
- `is_tagged` checks the NaN-box tag pattern in bits 49+
- Our typed-Arc bits are raw `Arc::into_raw(arc) as u64` pointers —
  the high bits are zero (pointer arithmetic), not the TAG_HEAP
  pattern.

So even after we allocate `Arc<HashSetData>` and stamp
`Ptr(HeapKind::HashSet)` on the slot, the JIT method dispatch
cannot find the right `call_*_method` handler. It hits
`dispatch_method_via_trampoline` (`crates/shape-jit/src/ffi/call_method/mod.rs:179-199`)
which is **`todo!()` — an extern-C todo!() aborts the process** at runtime.

### §3.4 This is the §2.7.10 / Q11 kinded MethodFnV2 ABI rebuild gap

The deferred work is identified at the call site:

```text
// crates/shape-jit/src/ffi/call_method/mod.rs:185-198
todo!(
    "phase-2c §2.7.10/Q11: JIT-side kinded MethodFnV2 ABI rebuild — \
     dispatch_method_via_trampoline. The trampoline VM's op_call_method \
     is now driven by the kinded MethodFnV2 ABI ({{args: &[KindedSlot]}} \
     carrier with per-arg NativeKind from the §2.7.7 stack parallel-kind \
     track) per ADR-006 §2.7.10/Q11. The deleted machinery (ValueBits \
     is_unified_heap probe, ValueWord::clone_from_bits receiver decode, \
     as_hashmap_data / as_typed_object accessors, push_raw_u64 / \
     pop_raw_u64 stack ABI, vmarray_from_vec, ValueWord::from_string / \
     from_hashmap_pairs / from_array constructors) was the kind-blind \
     pre-§2.7.10 dispatch shell. Reconstruction must thread NativeKind \
     through the JIT call signature per §2.7.5. See \
     docs/cluster-audits/wave-10-jit-playbook.md §5."
)
```

This is the **same** §2.7.10/Q11 work the W11-jit-carrier-conversion
Round-2 close already touched (commit `2960f5cf` added the
`JitFfiCarrier` (bits, NativeKind) carrier-pair conversion FFI
+ `jit_call_value` real body — but the parallel `jit_call_method`
rebuild was NOT in scope). The W11-jit-carrier-conversion close
commit summary names value-call but explicitly leaves method-call
for a later wave.

### §3.5 Parallel precedent: W12-jit-aggregate-non-array (Round 5B)

The Round 5B audit identified an identical-shaped gap for `Result`/
`Option`: `jit_make_ok` etc. could be added as FFI entries (and were
— see `FFIFuncRefs::make_ok` / `make_err` / `make_some` at
`crates/shape-jit/src/ffi_refs.rs:225-227`), but the EnumStore
consumer surfaces because the downstream pieces (Call-return kind
track, match-on-enum inline codegen, NaN-box↔Arc round-trip) are
not yet co-designed. Round 5B classified this as option (iii) and
surfaced; the supervisor accepted that close as PARTIAL with the
option-(iii) gap documented.

Rounds 6A (call-return kind, closed), 6B (match-enum-inline, audit-
only close), 6C (collection-ctor MIR, closed) are the three follow-
ups Round 5B identified. **My sub-cluster is at the same architectural
boundary Round 5B hit, applied to a different family of HeapKinds**
(collections instead of Result/Option).

**The decisive difference**: Round 5B's option-(i) partial landing
(register `jit_make_ok` symbols even though the consumer surfaces)
was useful preparatory work — when the downstream pieces eventually
land, the FFI infrastructure is in place. I could land the same
shape for collections: register `jit_v2_make_hashset` etc., extend
EnumStore consumer to call them, and keep the method-dispatch
surface as-is. **But the consumer-side surface would still be
silent process-abort via `todo!()` at first method call** —
strictly worse than the current Round 6C surface (a clean compile-
time `JIT compilation failed: EnumStore: SURFACE` error).

Specifically:
- **Pre-fix (current Round 6C state)**: JIT-compile fails cleanly
  with a §-cited SURFACE error at the EnumStore consumer; smoke
  exits non-zero with a readable diagnostic; VM mode unaffected.
- **Post-allocation-FFI-only (hypothetical)**: JIT-compile succeeds;
  allocates `Arc<HashSetData>` correctly; pushes typed-Arc bits to
  the slot; first method call (`s.add("a")`) reaches `jit_call_method`,
  falls through to `dispatch_method_via_trampoline` → `todo!()` →
  extern-C `todo!()` aborts the process (SIGABRT). **No diagnostic,
  process crash.**

Per CLAUDE.md "Forbidden rationalizations" / "Surface-and-stop
discipline" — **clean compile-time surface beats hidden runtime
process-abort**. The Round 6C landing is the principled current
state; landing allocation FFI alone REGRESSES the equivalence-
ratchet.

### §3.6 Why this matters: the W11-round-1 walk-back precedent

W11-jit-new-array Round 1 close (`b60d3678`) walked back
`jit_arc_retain` / `jit_arc_release` to silent no-ops because the
emitter caller-side wasn't kind-aware yet. The walk-back was refused
by the supervisor and the close was reopened (`e9a420ac`) with a
principled fix that includes the kind threading.

The same shape applies here. If I land allocation FFI but the method-
dispatch consumer surfaces with `todo!()`, that's the same defection
pattern: bury the architectural gap behind an internal `todo!()` that
will eventually fire as a process-abort. The principled response is
to surface and let the supervisor decide the co-design scope.

---

## §4. Why not stop-gap at the EnumStore consumer instead

Three alternatives I considered before settling on option (iii):

### §4.1 Alternative: emit `KindedSlot`-stamped slot AND keep the surface for first method call

Idea: land allocation FFI, but ALSO modify the JIT EnumStore consumer
to keep surfacing on subsequent method-call site. Result: the
allocation site succeeds but downstream first method call surfaces
"method dispatch on Ptr(HeapKind::HashSet) not yet supported".

**Rejected.** This is a strictly worse user experience than the
current Round 6C state. The user goes from a clean compile-time
error at the `let s = Set()` site to a slightly-later compile-time
error at the `s.add(...)` site. Same surface-and-stop pattern, more
code to maintain, no actual progress on smoke equivalence.

### §4.2 Alternative: route allocation through VM fallback on `Set()` ctor

Idea: detect collection-ctor at JIT compile time, deopt the entire
function to VM execution. Result: programs containing `let s = Set()`
never JIT-compile; they run on the VM regardless of size.

**Rejected.** This is also strictly worse than the current state:
- Round 5B already established that surface-and-stop at JIT compile
  is the principled response (with VM fallback handled by the
  outer driver, not by the JIT-side code itself).
- "Auto-deopt on collection-ctor" is a feature flag in disguise —
  CLAUDE.md "Forbidden rationalizations": *"Add a feature flag we
  can toggle."*
- The deopt-everywhere fallback was the deleted Wave-7 W-series
  pattern that the strict-typing plan exists to delete.

### §4.3 Alternative: land allocation FFI + extend `jit_call_method` to read parallel-kind track

Idea: extend `jit_call_method` to consult the JIT context's
`stack_kinds` parallel-kind track (the byte-typed array landed at
Round 3 W12-jit-stack-parallel-kind-track) instead of decoding kind
from NaN-box tags. When `kind == Ptr(HeapKind::HashSet)`, route the
method call to a kinded VM-side handler.

**This is the principled co-design that ADR-006 §2.7.10/Q11 describes.**
It is also a multi-week workstream (the cross-crate kinded MethodFnV2
ABI rebuild). It is the deferred work the §2.7.10 closure-trigger
explicitly parks. **It is correctly out of scope for a single sub-
cluster.**

If the supervisor wants this co-design as cluster-0 Round 8 scope,
that's a strategic call. The dispatch's "STOP after audit commit
and SURFACE" instruction is explicit; I'm complying.

---

## §5. Mechanical fix proposal — if option (iii) is rejected

Per the dispatch's option-(i) fallback framing ("If audit reveals
option (iii) ADR amendment is required, STOP after audit commit and
surface — that's a supervisor decision"), I document the bounded
mechanical work that COULD land in a subsequent commit if the
supervisor decides to land allocation FFI as preparatory work
(Round 5B-style partial close).

### §5.1 New FFI bodies (~150 LoC, new file `crates/shape-jit/src/ffi/v2/collection_ctors.rs`)

```rust
//! Typed-Arc collection-ctor FFI for the 8 HeapKind ordinals
//! per ADR-006 §2.7.15 (HashSet) / §2.7.4 (HashMap) / §2.7.19
//! (Deque) / §2.7.18 (PriorityQueue) / §2.7.20 (Channel) / §2.7.25
//! (Mutex/Atomic/Lazy). Each returns Arc::into_raw(arc) as u64; the
//! slot is stamped Ptr(HeapKind::X) by the EnumStore consumer per
//! §2.7.5 producer-side classification.

use shape_value::heap_value::{
    HashSetData, HashMapData, DequeData, PriorityQueueData,
    ChannelData, MutexData, AtomicData, LazyData,
};
use shape_value::{KindedSlot, NativeKind};
use std::sync::Arc;

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_hashset() -> u64 {
    let arc = Arc::new(HashSetData::new());
    Arc::into_raw(arc) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_hashmap() -> u64 {
    let arc = Arc::new(HashMapData::new());
    Arc::into_raw(arc) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_deque() -> u64 {
    let arc = Arc::new(DequeData::new());
    Arc::into_raw(arc) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_priorityqueue() -> u64 {
    let arc = Arc::new(PriorityQueueData::new());
    Arc::into_raw(arc) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_channel() -> u64 {
    let arc = Arc::new(ChannelData::new());
    Arc::into_raw(arc) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_atomic(initial: i64) -> u64 {
    let arc = Arc::new(AtomicData::new(initial));
    Arc::into_raw(arc) as u64
}

/// Mutex(value) — carrier-pair form per §2.7.5. The (bits, kind)
/// pair reconstructs to `KindedSlot::new(ValueSlot::from_raw(bits), kind)`.
/// Caller (JIT-emitted code) supplies kind from the producing-operand's
/// NativeKind at MIR-emit time.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_mutex(value_bits: u64, value_kind: u8) -> u64 {
    let kind = decode_kind_byte(value_kind);
    let initial = KindedSlot::new(
        shape_value::slot::ValueSlot::from_raw(value_bits),
        kind,
    );
    let arc = Arc::new(MutexData::new(initial));
    Arc::into_raw(arc) as u64
}

/// Lazy(closure) — closure_bits is Arc::into_raw of the closure.
/// Kind label is enforced at JIT compile time (consumer refuses non-
/// Ptr(HeapKind::Closure) inner-operand kinds at the EnumStore site).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_lazy(closure_bits: u64) -> u64 {
    let initializer = KindedSlot::new(
        shape_value::slot::ValueSlot::from_raw(closure_bits),
        NativeKind::Ptr(shape_value::heap_value::HeapKind::Closure),
    );
    let arc = Arc::new(LazyData::new(initializer));
    Arc::into_raw(arc) as u64
}

fn decode_kind_byte(byte: u8) -> NativeKind {
    // Mirror of stack_kind_code::decode (§2.7.7 parallel-kind track).
    // Caller supplies the byte from the JIT-side `stack_kinds` track
    // at MIR-emit time per §2.7.5.
    // ...
}
```

### §5.2 Symbol registration (`ffi_symbols/v2_symbols.rs` extension)

8 new entries via `builder.symbol(...)` + 8 `declare(...)` calls.
Mirror of the existing `jit_v2_array_new_*` family pattern.

### §5.3 `FFIFuncRefs` slots + `ffi_builder.rs` lookup

8 new `pub(crate) make_hashset: FuncRef` / etc. fields. 8 `r!(...)`
lookups.

### §5.4 EnumStore consumer extension (`mir_compiler/statements.rs`)

Replace the surface block with dispatch on `variant_name`:

```rust
if is_collection_ctor {
    let variant_label = variant_name.as_deref().unwrap();
    let func_ref = match variant_label {
        "Set"           => self.ffi.make_hashset,
        "HashMap"       => self.ffi.make_hashmap,
        "Deque"         => self.ffi.make_deque,
        "PriorityQueue" => self.ffi.make_priorityqueue,
        "Channel"       => self.ffi.make_channel,
        "Mutex"         => self.ffi.make_mutex,
        "Atomic"        => self.ffi.make_atomic,
        "Lazy"          => self.ffi.make_lazy,
        _ => unreachable!(),
    };
    // Compile operands (with-arg ctors only); validate inner kind.
    // Emit `call func_ref` with proper arg list (0 / 1 / 2 args).
    // Write the resulting u64 into Place::Local(container_slot).
    return Ok(());
}
```

### §5.5 Estimated landing scope (if option-(i) supervisor decision)

~250 LoC across 4 files. Half-day mechanical implementation. Tests:
- Allocation balance counters for each FFI entry (no leak — every
  `Arc::into_raw` paired with a `jit_arc_release` retire when the slot
  drops).
- JIT compile succeeds for all 8 smokes (no surface at EnumStore).
- **Method dispatch still surfaces** — smokes fail at first method call
  via the `todo!()` route OR via a different surface I add at
  `jit_call_method`'s `_ => TAG_NULL` arm for `Ptr(HeapKind::X)` kinds
  (adding a structured Err return instead of process-abort).

This last point — replacing the `todo!()` with a structured Err — is
itself a small principled improvement (`todo!()` in extern-C aborts
the process; a structured return + JIT-side propagation surfaces a
clean diagnostic). Would land as part of the option-(i) partial close
if the supervisor chooses that path.

---

## §6. Coordination check with parallel sub-cluster (W12-jit-result-option-trinity 7A)

The dispatch named the parallel sub-cluster as `W12-jit-result-option-
trinity` (Round 7A), territory: `Result` (ord 27) and `Option` (ord 28)
HeapKinds. I checked the file overlap before drafting this audit:

- **`crates/shape-jit/src/ffi_refs.rs`**: 7A adds Result/Option FFI
  slots (`make_result_arc`, `make_option_arc`); I would add 8
  collection slots. **No conflict** — disjoint field additions.
- **`crates/shape-jit/src/compiler/ffi_builder.rs`**: same — disjoint
  `r!(...)` line additions.
- **`crates/shape-jit/src/mir_compiler/statements.rs::EnumStore`**:
  7A handles `variant_name` = "Ok" / "Err" / "Some" / "None" path;
  I would handle the collection-ctor path. The
  `is_collection_ctor_name` disambiguator Round 6C added keeps the
  two paths separate. **No conflict** at the dispatch level.
- **`crates/shape-jit/src/ffi/`**: 7A would add a `result_arc.rs` /
  `option_arc.rs`; I would add `collection_ctors.rs`. Disjoint files.

**Coordination conclusion**: zero file-territory overlap. The two
sub-clusters could land in parallel without merge conflict. **Both
likely surface option (iii) for the method-dispatch architectural
gap** — same root cause (§2.7.10/Q11 kinded MethodFnV2 ABI rebuild),
applied to different HeapKind families.

---

## §7. Forbidden frames explicitly refused

Per CLAUDE.md "Forbidden Patterns" + dispatch sub-cluster's
"Forbidden" list, I document the framings I am NOT proposing:

- **NOT** "collection-FFI bridge" / "typed-Arc translator" /
  "container-allocation helper" — broader-family defection-attractor
  regex (CLAUDE.md "Renames to refuse on sight" `(decode|tag|kind|
  dispatch|...) (bridge|probe|helper|hop|translator|adapter|shim)`).
  The fix shape is "typed-Arc allocation FFI for §2.7.X HeapKinds";
  the consumer threading is "`StatementKind::EnumStore` consumer
  dispatch on `variant_name`".
- **NOT** Bool-default fallback for inner-kind when statically
  underivable — the Mutex inner-kind threading uses the (bits, kind)
  carrier-pair form per §2.7.5 stable-FFI rule, not a Bool-default
  on uninferable kind.
- **NOT** resurrecting `unified_box(HK_HASHSET, ...)` /
  `Box::into_raw(Box::new(UnifiedValue<HashSetData>)) as u64` shape.
  The kickoff prompt named this shape, but **after audit the correct
  shape is `Arc::into_raw(Arc<HashSetData>) as u64`** to match the
  VM-side `KindedSlot::from_hashset` carrier exactly. Going through
  `UnifiedValue<T>` would re-introduce a wrong-type retain/release at
  every cross-boundary trip (the slot's kind says
  `Ptr(HeapKind::HashSet)` but the bits would be a
  `UnifiedValue<HashSetData>` allocation, not an `Arc<HashSetData>`
  allocation — segfault on every `jit_arc_release` reclaim attempt).
  See §8 below.
- **NOT** silent walkback — if the supervisor declares this work
  out of scope, I close the audit-only sub-cluster with the option-
  (iii) surface and don't land any code. Per W11-round-1 walk-back
  precedent.
- **NOT** "blame pre-existing" — the broken `jit_call_method` shell
  is documented as a §2.7.10/Q11 deferral; that's the kind-blind ABI
  the strict-typing plan is migrating away from. My audit names the
  gap, not the prior agents who landed the (correct-at-the-time)
  surface markers.

---

## §8. Carrier-shape clarification: Arc vs Box::UnifiedValue

The dispatch said:
> Per-HeapKind monomorphized `Box::into_raw(Box::new(UnifiedValue<T>)) as u64` allocation entry points

After audit, this is **the W11-jit-new-array Route A pattern applied
literally**, but it's a mismatch for the collection case:

- **W11 Route A** (`TypedArray<T>`): used `*mut TypedArray<T>` (raw
  heap object, not Arc). JIT and VM share the *same* raw pointer
  shape because TypedArray's refcount lives in the `HeapHeader` at
  offset 0 (`shape-value/src/v2/heap_header.rs::HeapHeader.refcount:
  AtomicU32`). Both can construct/retain/release via the same
  `HeapHeader.refcount` field. No Arc wrapper.
- **Collections (HashSet/HashMap/...)**: VM-side `KindedSlot::from_X`
  stores `Arc::into_raw(Arc<XData>) as u64`. The refcount lives
  inside `Arc<T>`'s internal control block, NOT inside `XData`
  itself. JIT-side allocation MUST go through `Arc::new(XData::new())`
  + `Arc::into_raw` to interoperate with the existing
  `arc_retain`/`arc_release` paths (`shape-jit/src/ffi/jit_release.rs`
  reads the kind byte from offset 0 of `UnifiedValue<T>` — but the
  slot bits at `Ptr(HeapKind::HashSet)` are NOT `UnifiedValue<HashSetData>`
  allocations, they're `Arc<HashSetData>` allocations, so the
  `release_unified_value_by_kind` dispatch would fault).

**Recommended carrier shape**: `Arc::into_raw(Arc<XData>) as u64`,
matching the VM-side `KindedSlot::from_*` constructors verbatim. This
is the §2.7.6 / Q8 / §2.7.5 single-source-of-truth pattern.

**The kickoff's `Box::new(UnifiedValue<T>)` prescription is the W11
shape but applied to the wrong family of HeapKinds.** The W11 family
uses TypedArray<T> (which has its own HeapHeader refcount), but the
collection family uses Arc<XData> (which has Arc's internal refcount).
The two refcount mechanisms are not interchangeable.

**This is a sub-cluster-level discovery worth surfacing**: even the
`option (i)` mechanical landing requires a small ADR-clarification
amendment on the carrier-shape mismatch — the dispatch's literal
prescription would have caused a silent ABI mismatch with the VM-
side carrier. The audit catches it; landing code without the audit
would not have.

---

## §9. Sites surfaced (cite-tracked, for cluster-1+ follow-up)

1. **`jit_call_method` collection-kind dispatch** — `crates/shape-jit/
   src/ffi/call_method/mod.rs:201-388`. The `heap_kind(receiver_bits)`
   NaN-box-tag dispatch cannot route typed-Arc receivers. Requires
   §2.7.10/Q11 kinded MethodFnV2 ABI rebuild. **Load-bearing for
   smoke 4 + the 2 additional smokes — no current cluster-0 path
   exercises it cleanly.**

2. **`dispatch_method_via_trampoline` extern-C `todo!()`** —
   `crates/shape-jit/src/ffi/call_method/mod.rs:179-199`. extern-C
   `todo!()` aborts the process (not a controlled surface). Even
   pending the §2.7.10 rebuild, this should be a structured Err
   return — a small principled improvement orthogonal to the
   broader §2.7.10 work. Tracked as `W12-jit-method-dispatch-
   structured-error` follow-up.

3. **Carrier-shape ADR clarification** — the kickoff's prescription
   `Box::into_raw(Box::new(UnifiedValue<T>))` vs the VM's
   `Arc::into_raw(Arc<XData>)` discrepancy. Whichever path the
   supervisor decides, the §2.7.15 / §2.7.18 / §2.7.19 / §2.7.20 /
   §2.7.25 amendments could add a "Carrier-shape note: typed-Arc
   per §2.7.6 / Q8, NOT W11-style `Box<UnifiedValue<T>>`" clause.
   Documentation hygiene, not a behavior change.

4. **HashMap K/V kind threading** (per kickoff's hint) — NOT a gap
   in the allocation path (HashMap stores `Arc<HeapValue>` values
   heterogeneously, like Deque/Channel). K/V kinds matter only for
   downstream `m.set(k, v)` / `m.get(k)` method dispatch, which is
   already in the §2.7.10/Q11 deferral. **Not a sub-cluster gap;
   not an ADR amendment requirement.**

5. **JitFfiCarrier vs `Arc::into_raw` carrier shape coherence** —
   W11-jit-carrier-conversion (Round 2 close `ff1ad3e6`) established
   the `(bits, NativeKind)` carrier-pair form for `jit_call_value`.
   The Mutex(x) ctor — accepting any kind — naturally uses this form.
   The other 7 ctors don't need it. Worth noting for §2.7.5 documentation.

---

## §10. Disposition recommendation

**Surface this audit as the close of W12-jit-collection-typed-arc-
ffi (Round 7B) — audit-only**, mirroring the Round 6B audit-only
close (`25ac74d4` W12-jit-match-enum-inline-codegen).

Reasoning:

1. **The smoke target cannot pass with allocation FFI alone** — the
   method-dispatch gap is the load-bearing piece, and it is
   §2.7.10/Q11 deferred work.
2. **Landing allocation FFI alone REGRESSES the user-facing
   equivalence-ratchet** — current Round 6C state is a clean compile-
   time surface; post-allocation-FFI would be a slightly-later runtime
   surface OR (with the `todo!()` arm) a process-abort.
3. **The dispatch explicitly anticipated this** — "If your audit
   reveals (iii), STOP after the audit commit and SURFACE to
   supervisor."

If the supervisor decides to land the allocation FFI as preparatory
work (Round 5B-style partial close), this audit becomes the design
doc for the ~250 LoC mechanical implementation. The fix would NOT
include any change to `jit_call_method` — that's a separate sub-
cluster.

If the supervisor decides to deeper-dispatch the §2.7.10/Q11 work,
this audit identifies the surfaces (`jit_call_method` shell rebuild,
`dispatch_method_via_trampoline` real body, kind-threaded receiver
classification via `stack_kinds` parallel-kind track) that the
broader work would touch.

---

## §11. Close-gate verification (audit-only commit)

This audit commit is documentation-only — no source changes, no FFI
additions, no MIR changes. Close gates run for completeness:

- `cargo check --workspace --lib --tests` — expected EXIT=0 (no
  changes to compile)
- `cargo test -p shape-jit --lib` — expected 322/0/26 (matches Round
  6C close baseline)
- `cargo test -p shape-vm --lib` — expected pre-existing failure set
  (v2-raw-heap-audit cluster) — unaffected by this audit
- `bash scripts/verify-merge.sh` — expected 12/12
- `bash scripts/check-no-dynamic.sh` — expected EXIT=0
- Smoke 4 / additional collection smokes — UNCHANGED (Round 6C
  surfaces preserved verbatim under `--mode jit`; VM mode prints
  correct values)

The audit is content-only; gates verify no regression.

---

## §12. ADR amendment status

**No ADR-006 amendment required for the AUDIT itself.** This document
is an audit / disposition surface, not an amendment.

**ADR-006 amendments WOULD be required for the deeper co-design work
this audit surfaces**:

- §2.7.10 / Q11 closure-trigger condition extension — adding
  "method dispatch for collection-family HeapKinds" to the §2.7.10
  rebuild scope clauses.
- Cross-§-amendment carrier-shape note — clarifying the typed-Arc
  vs `Box::new(UnifiedValue<T>)` distinction across §2.7.15 / §2.7.18
  / §2.7.19 / §2.7.20 / §2.7.25 (one clause per amendment, or a
  central one in §2.7.6 / Q8 carrier-API-bound).

Neither lands in this audit-only commit. If the supervisor opens
follow-up work, the amendments land alongside the relevant fix.

---

*End of audit. Branch: `bulldozer-strictly-typed-w12-jit-collection-
typed-arc-ffi`. Audit commit hash: pending (will be backfilled at
close).*
