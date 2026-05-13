# W12-jit-collection-method-dispatch-abi — audit

**Sub-cluster:** Phase 3 cluster-0 Round 8B (AUDIT-FIRST).
**Branch:** `bulldozer-strictly-typed-w12-jit-collection-method-dispatch-abi` (parent `267b1ca2`).
**Worktree:** `/home/dev/dev/shape-lang/shape-w12-jit-collection-method-dispatch-abi`.
**Audited:** 2026-05-13.
**Disposition:** option **(iii)** — STOP after audit commit; surface architectural insight to supervisor.

---

## §0. TL;DR — Audit refines Round 7B's scope, but the trinity is still multi-week

Round 7B classified this as `§2.7.10/Q11 kinded MethodFnV2 ABI rebuild` and
declined to land partial code. This audit confirms the architectural gap
but **discovers a load-bearing scope refinement**: the original "trinity item
(iii)" — *per-HeapKind kinded MethodFnV2 entries on the JIT side* — is
**unnecessary**. The VM-side handlers in
`crates/shape-vm/src/executor/objects/method_registry.rs` are already
`MethodFnV2`-shaped (kinded `&[KindedSlot]` → `Result<KindedSlot, VMError>`
per ADR-006 §2.7.10/Q11). The JIT-side dispatch shell does not need to
mirror those ~50+ handler entries; it needs to **delegate to the VM's
existing kinded dispatch via a new public `jit_trampoline_call_method` API
on `VirtualMachine`**, structurally identical to the
`jit_trampoline_call_closure` precedent at
`crates/shape-vm/src/executor/call_convention.rs:953`.

This refinement changes the scope from "trinity with ~50-entry mirror" to
"quadrant" where the trinity's third item is replaced by a single new VM
public-API entry point. **Even with that refinement applied, the remaining
work is still a multi-piece co-design** (typed-Arc allocation FFI × 8 +
retain/release kinded entries × 8 + new VM public API + `jit_call_method`
shell rebuild + EnumStore consumer arm + tests). The honest scope estimate
is **~600-900 LoC across ~7-9 files plus a shape-vm crate-boundary API
addition** — at the high end of a single sub-cluster's budget, and the
shape-vm public-API change crosses a crate boundary that wasn't anticipated
in the dispatch.

Per the dispatch instruction *"if the integrated trinity scope exceeds a
single round's reasonable budget, surface as audit-only close with
structured option (iii) cite, like Round 7B did"*, this audit closes
audit-only. **No code changes that regress Round 6C/7B's clean SURFACE-at-
EnumStore-consumer state are landed.** The audit document is the
deliverable; the §2.7.10/Q11 implementation territory is surfaced for
supervisor scoping with the corrected scope estimate.

---

## §1. Surface — exact gap analysis with file:line cites

### §1.1 The `jit_call_method` shell — current state

`crates/shape-jit/src/ffi/call_method/mod.rs:201` is the dispatch shell
called by the MIR emitter (`crates/shape-jit/src/mir_compiler/terminators.rs:271-274`):

```rust
pub extern "C" fn jit_call_method(ctx: *mut JITContext, stack_count: usize) -> u64 {
    // ... pop arg_count (raw i64), method_name (NaN-boxed string), args, receiver ...
    // Then dispatches by `heap_kind(receiver_bits)`:
    let builtin_result = if is_ok_tag(receiver_bits) || is_err_tag(receiver_bits) {
        call_result_method(...)
    } else if is_number(receiver_bits) {
        call_number_method(...)
    } else {
        match heap_kind(receiver_bits) {
            Some(HK_ARRAY) => call_array_method(...),
            Some(HK_STRING) => call_string_method(...),
            Some(HK_JIT_OBJECT) => call_object_method(...),
            // ... no arms for HK_HASHSET, HK_HASHMAP, HK_DEQUE, HK_PRIORITYQUEUE,
            //     HK_CHANNEL, HK_MUTEX, HK_ATOMIC, HK_LAZY ...
            _ => TAG_NULL,
        }
    };

    if builtin_result == TAG_NULL {
        // Tries try_call_user_method ...
        // Then falls through to:
        dispatch_method_via_trampoline(receiver_bits, &method_name, &args, ctx)
        // ^^ This is extern-C todo!() — aborts process at runtime.
    }
}
```

The `heap_kind(receiver_bits)` call at `value_ffi.rs:330-336` decodes a
NaN-box tag. Typed-Arc collection slots store `Arc::into_raw(arc) as u64`
raw pointers with no NaN-box tag. The probe returns `None`, dispatch falls
through to `dispatch_method_via_trampoline` (`call_method/mod.rs:179`),
which is `todo!()` and aborts the process at runtime.

**Forbidden** per §2.7.7 #4/#7: even reading kind from `receiver_bits` is
the wrong shape. Receiver kind MUST come from the parallel `stack_kinds`
track (§2.7.7/Q9) — already maintained at the JIT context level
(`crates/shape-jit/src/context.rs:515` — `pub stack_kinds: [u8; 512]`),
already written in lockstep at the MIR-emit method-call site
(`mir_compiler/terminators.rs:230-262`).

### §1.2 The MIR emit-side method-call lockstep — current state

`crates/shape-jit/src/mir_compiler/terminators.rs:202-262` emits the JIT
call to `jit_call_method`. Critically, it ALREADY writes the parallel-
kind track via `emit_kind_track_write` at every push:

```rust
for (i, arg) in args.iter().enumerate() {
    let arg_kind = self.operand_slot_kind_or_carrier(arg);
    let val = self.compile_operand(arg)?;
    // ... widen val to I64 ...
    let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
    let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
    let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base_offset as i64);
    let store_addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
    self.builder.ins().store(MemFlags::new(), boxed, store_addr, 0);
    // §2.7.7 / Q9 lockstep parallel-kind write.
    self.emit_kind_track_write(slot_idx, arg_kind);
}
```

So at the dispatch-shell receive side (`jit_call_method`), reading
`ctx.stack_kinds[receiver_slot_idx]` recovers the receiver's `NativeKind`
without any tag decode. The infrastructure is **already in place**. The
gap is purely on the consumer side — `jit_call_method` doesn't read it.

### §1.3 The EnumStore consumer for collection_ctor — current state

`crates/shape-jit/src/mir_compiler/statements.rs:239-268` surfaces
collection-ctor `EnumStore` shapes with a structured error:

```rust
let is_collection_ctor = variant_name.as_deref().map(is_collection_ctor_name).unwrap_or(false);
if is_collection_ctor {
    let variant_label = variant_name.as_deref().unwrap_or("<missing>");
    return Err(format!(
        "EnumStore: SURFACE — primitive-collection constructor '{}' \
         (operands.len()={}) requires the typed-Arc allocation FFI \
         for its `HeapKind` ... not yet landed at \
         W12-collection-constructor-mir-lowering ...",
        variant_label, operands.len(), variant_label,
    ));
}
```

This is the Round 6C clean surface state. Round 7B confirmed it is the
honest equivalence-ratchet position — better than allocating successfully
and then SIGABRT-ing at the first method call. The Round-8B trinity
requires this arm to be replaced with a typed-Arc allocation FFI call.

### §1.4 The 8 HeapKinds and their VM-side ctors

Per Round 7B audit §1, the VM-side `BuiltinFunction::*` ctors at
`crates/shape-vm/src/executor/vm_impl/builtins.rs:587-749` produce
`KindedSlot::from_X(Arc<XData>)` where `X ∈ {HashSet, HashMap, Deque,
PriorityQueue, Channel, Mutex, Atomic, Lazy}`. The bits are
`Arc::into_raw(arc) as u64`. **Refcount lives in Arc's internal control
block at offset -16 from the data pointer**, NOT in a HeapHeader at
offset 0. This is Round 7B audit §8's load-bearing observation.

---

## §2. ADR-006 §2.7.10/Q11 binding constraints

Verbatim from the ADR (§2.7.10 / lines 1003-1326):

1. **`MethodFnV2` signature** (post-Q11):
   ```rust
   pub type MethodFnV2 = fn(
       &mut VirtualMachine,
       args: &[KindedSlot],
       Option<&mut ExecutionContext>,
   ) -> Result<KindedSlot, VMError>;
   ```
2. **Args[0]** is the receiver; **args[1..]** are call args.
3. **Every entry's `kind`** comes from the §2.7.7 stack parallel-kind track at
   the dispatch shell — no fabrication, no tag-bit decode.
4. **Heterogeneous-kind body pattern** (§2.7.6/Q8): handlers dispatch on
   `args[i].kind`; heap arms go through `args[i].slot.as_heap_value()` +
   `HeapValue` match (preserves ADR-005 §1 single-discriminator).
5. **Carrier-API-bound** (§2.7.6/Q8): kind goes on the carrier struct, not as
   a parallel slice parameter; no `(u64, NativeKind)` result type — only
   `KindedSlot` results.

**Forbidden (verbatim from ADR §2.7.10 forbidden list, lines 1160-1214)**:
- `&mut [u64]` args with kind decoded from high bits.
- `is_heap()` probe on each entry.
- Parallel `&[NativeKind]` second-slice parameter on `MethodFnV2`.
- `&mut [KindedSlot]` mutable form / `Vec<KindedSlot>` by-move.
- Result type `(u64, NativeKind)` rather than `KindedSlot`.
- Transitional shims preserving deleted ABI-shape names (`MethodFn` /
  `MethodFnLegacy` / `dispatch_method_handler_raw` /
  `call_handler_with_u64_slice`).
- Defection-attractor descriptors ("MethodFnV2 bridge", "MethodFn translator",
  "dispatch-slice probe", "boundary adapter for handler ABI",
  "kind-injection helper").

### §2.1 What does *not* need to be migrated on the JIT side

**This is the audit's load-bearing insight, missed by the dispatch:**

The VM-side handlers in
`crates/shape-vm/src/executor/objects/method_registry.rs` are **already
kinded** per the ADR-006 §2.7.10/Q11 ABI. Specifically:

- `SET_METHODS` PHF map (line 463-480): 14 entries, each pointing to a
  `v2_*` function with the signature `fn(&mut VirtualMachine,
  &[KindedSlot], Option<&mut ExecutionContext>) -> Result<KindedSlot,
  VMError>`. Verified by reading `crates/shape-vm/src/executor/objects/
  set_methods.rs:179-202` — `v2_size`, `v2_add`, `v2_is_empty`,
  `v2_to_array` all conform.
- `HASHMAP_METHODS` (line 428-456): 22 entries, same signature shape.
- `DEQUE_METHODS` (line 488-502): 11 entries.
- `PRIORITY_QUEUE_METHODS` (line 510-521): 9 entries.
- `MUTEX_METHODS` (line 822-827): 4 entries.
- `ATOMIC_METHODS` (line 830-836): 5 entries.
- `LAZY_METHODS` (line 839-842): 2 entries.
- `CHANNEL_METHODS` (line 845-852): 6 entries.

**Total: ~73 method entries across 8 HeapKinds, all already kinded.**

The dispatch shell `op_call_method` at `crates/shape-vm/src/executor/objects/
mod.rs:333` is already kinded too — it does receiver classification, looks
up the handler via `resolve_method_handler` (line 462), and dispatches
with kinded args. The kinded dispatch path is **complete on the VM side**.

The JIT-side dispatch shell `jit_call_method` should therefore **delegate
to the VM-side kinded dispatch**, not mirror those 73 handlers. The
mechanical shape is identical to `jit_trampoline_call_closure` at
`crates/shape-vm/src/executor/call_convention.rs:953` — the JIT pops
`(bits, kind)` tuples from `ctx.stack` + `ctx.stack_kinds`, packages them
into `KindedSlot` carriers, calls a new public `VirtualMachine` method,
and receives a kinded result.

**This obviates the dispatch's described "Per-HeapKind kinded MethodFnV2
entries" — there are no JIT-side handlers to write. The trinity is
actually a triplet of (i) typed-Arc allocators, (ii) kinded dispatch shell
rebuild, (iii) one new public VM trampoline API.**

---

## §3. Per-HeapKind method roster (exhaustive, from VM PHF maps)

For completeness, the VM-side methods that the JIT-delegated dispatch
covers — these are NOT JIT-side entries to write; this roster confirms
the smoke-target methods exist:

| HeapKind | Method roster (from `method_registry.rs`) | Smoke-target methods |
|---|---|---|
| HashSet (21) | add, delete, has, size, len, length, isEmpty, toArray, union, intersection, difference, forEach, map, filter | `add`, `size` |
| HashMap (17) | get, set, has, delete, remove, keys, values, entries, len, length, isEmpty, merge, getOrDefault, toArray, map, filter, forEach, reduce, groupBy, iter | `set`, `get`, `size` (alias `len`) |
| Deque (23) | pushBack, pushFront, popBack, popFront, peekBack, peekFront, size, len, length, isEmpty, toArray, get | (not in smoke matrix) |
| PriorityQueue (25) | push, pop, peek, size, len, length, isEmpty, toArray, toSortedArray | (not in smoke matrix) |
| Channel (24) | send, recv, try_recv, close, is_closed, is_sender | (not in smoke matrix) |
| Mutex (30) | lock, try_lock, set, get | `get` (and `lock` for explicit smoke) |
| Atomic (31) | load, store, fetch_add, fetch_sub, compare_exchange | `load` |
| Lazy (32) | get, is_initialized | `get` |

Smoke 4 (`Set()` + `add("a")` + `add("b")` + `size()`) exercises HashSet
arm. Additional smokes specified in dispatch (HashMap + Mutex) exercise
HashMap and Mutex arms. The delegation path covers all 8 HeapKinds
uniformly — no per-method JIT-side wiring.

---

## §4. Per-HeapKind FFI shape table

### §4.1 Typed-Arc allocation FFI (required)

| HeapKind | FFI entry | Args | Return |
|---|---|---|---|
| HashSet | `jit_v2_make_hashset` | none | `Arc::into_raw(Arc::new(HashSetData::new())) as u64` |
| HashMap | `jit_v2_make_hashmap` | none | `Arc::into_raw(Arc::new(HashMapData::new())) as u64` |
| Deque | `jit_v2_make_deque` | none | `Arc::into_raw(Arc::new(DequeData::new())) as u64` |
| PriorityQueue | `jit_v2_make_priorityqueue` | none | `Arc::into_raw(Arc::new(PriorityQueueData::new())) as u64` |
| Channel | `jit_v2_make_channel` | none | `Arc::into_raw(Arc::new(ChannelData::new())) as u64` |
| Atomic | `jit_v2_make_atomic` | `(i64)` | `Arc::into_raw(Arc::new(AtomicData::new(i))) as u64` |
| Lazy | `jit_v2_make_lazy` | `(closure_bits: u64)` | `Arc::into_raw(Arc::new(LazyData::new(...))) as u64` |
| Mutex | `jit_v2_make_mutex` | `(value_bits: u64, value_kind: u8)` | `Arc::into_raw(Arc::new(MutexData::new(...))) as u64` |

8 entries. Per-allocator inner-kind validation:
- Atomic enforces `Int64` (§2.7.25). Inner-operand kind validated at JIT
  compile-time (`mir_compiler/statements.rs` EnumStore consumer rejects
  non-Int64 with structured surface).
- Lazy enforces `Ptr(HeapKind::Closure)` (§2.7.25). Same compile-time
  validation pattern.
- Mutex accepts any kind — passes `(bits, kind_code)` carrier-pair per
  §2.7.5 stable-FFI rule.

### §4.2 Retain/release kinded entries (required for refcount discipline)

Following Round 7A's precedent at `ffi/result.rs:425-470` (where
`jit_arc_result_retain`/`_release` / `jit_arc_option_retain`/`_release`
were added because the legacy `jit_arc_retain`/`jit_arc_release` operate
on `UnifiedValue<T>` refcount at offset 4 — corrupting Arc's
internal-control-block refcount at offset -16). 8 retain + 8 release
entries:

| HeapKind | Retain FFI | Release FFI | Body shape |
|---|---|---|---|
| HashSet | `jit_arc_hashset_retain` | `jit_arc_hashset_release` | `Arc::increment/decrement_strong_count::<HashSetData>(bits as *const HashSetData)` |
| HashMap | `jit_arc_hashmap_retain` | `jit_arc_hashmap_release` | `Arc::increment/decrement_strong_count::<HashMapData>` |
| Deque | `jit_arc_deque_retain` | `jit_arc_deque_release` | `Arc::increment/decrement_strong_count::<DequeData>` |
| PriorityQueue | `jit_arc_priorityqueue_retain` | `jit_arc_priorityqueue_release` | `Arc::increment/decrement_strong_count::<PriorityQueueData>` |
| Channel | `jit_arc_channel_retain` | `jit_arc_channel_release` | `Arc::increment/decrement_strong_count::<ChannelData>` |
| Mutex | `jit_arc_mutex_retain` | `jit_arc_mutex_release` | `Arc::increment/decrement_strong_count::<MutexData>` |
| Atomic | `jit_arc_atomic_retain` | `jit_arc_atomic_release` | `Arc::increment/decrement_strong_count::<AtomicData>` |
| Lazy | `jit_arc_lazy_retain` | `jit_arc_lazy_release` | `Arc::increment/decrement_strong_count::<LazyData>` |

The `retain_func_for_place` / `release_func_for_place` helpers at
`crates/shape-jit/src/mir_compiler/` need extension to match each
`NativeKind::Ptr(HeapKind::*)` arm to the matching retain/release FuncRef.

### §4.3 Dispatch shell — new shape

```rust
pub extern "C" fn jit_call_method(ctx: *mut JITContext, stack_count: usize) -> u64 {
    unsafe {
        let ctx_ref = &mut *ctx;

        // ABI: arg_count (raw i64), method_name (NaN-boxed string), args, receiver.
        // POP arg_count
        ctx_ref.stack_ptr -= 1;
        let arg_count = ctx_ref.stack[ctx_ref.stack_ptr] as usize;
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;

        // POP method_name (NaN-boxed string, JIT-internal NaN-box; preserved per §2.7.5 JIT-internal)
        ctx_ref.stack_ptr -= 1;
        let method_bits = ctx_ref.stack[ctx_ref.stack_ptr];
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
        let method_name = unbox_string(method_bits).to_string();

        // POP args (reverse pop, then reverse to source order) with kinds from stack_kinds.
        let mut arg_pairs: Vec<(u64, NativeKind)> = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            ctx_ref.stack_ptr -= 1;
            let bits = ctx_ref.stack[ctx_ref.stack_ptr];
            let code = ctx_ref.stack_kinds[ctx_ref.stack_ptr];
            ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
            let kind = stack_kind_code::decode(code)
                .ok_or_else(|| /* surface §2.7.7 #9 */)?;
            arg_pairs.push((bits, kind));
        }
        arg_pairs.reverse();

        // POP receiver
        ctx_ref.stack_ptr -= 1;
        let recv_bits = ctx_ref.stack[ctx_ref.stack_ptr];
        let recv_code = ctx_ref.stack_kinds[ctx_ref.stack_ptr];
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
        let recv_kind = stack_kind_code::decode(recv_code)
            .ok_or_else(|| /* surface §2.7.7 #9 */)?;

        // Build kinded carriers and delegate to the VM-side kinded dispatch.
        let receiver = KindedSlot::new(ValueSlot::from_raw(recv_bits), recv_kind);
        let args: Vec<KindedSlot> = arg_pairs.iter().map(|(b, k)|
            KindedSlot::new(ValueSlot::from_raw(*b), *k)).collect();

        with_trampoline_vm_mut(|vm| {
            match vm.jit_trampoline_call_method(&receiver, &args, &method_name, None) {
                Ok(result) => {
                    let bits = result.slot.raw();
                    std::mem::forget(result);          // transfer to JIT slot
                    std::mem::forget(receiver);        // VM owns the share
                    std::mem::forget(args);            // VM owns the shares
                    bits
                }
                Err(_) => TAG_NULL,
            }
        }).unwrap_or(TAG_NULL)
    }
}
```

The new VM-side public API `VirtualMachine::jit_trampoline_call_method`
mirrors `jit_trampoline_call_closure`. Its body is essentially what
`op_call_method` does after instruction-operand decode — it can either
inline the logic (preferred — avoids stack push/pop) or call a new
shared helper.

---

## §5. Carrier-shape table (audit §8 from Round 7B — preserved)

Per Round 7B audit §8, the typed-Arc collection allocation shape is:

| HeapKind family | Carrier shape | Refcount location | Retain/release ABI |
|---|---|---|---|
| **W11 TypedArray family** | `Box::into_raw(Box::new(UnifiedValue<TypedArrayData<T>>))` | HeapHeader at offset 4 (`UnifiedValue<T>.refcount`) | `jit_arc_retain` / `jit_arc_release` (legacy unified) |
| **W12 Collection family (this audit)** | `Arc::into_raw(Arc<XData>) as u64` | Arc internal control block at offset -16 from data ptr | **New per-HeapKind kinded retain/release** (§4.2 above) |
| **W12 Result/Option family (Round 7A precedent)** | `Arc::into_raw(Arc<ResultData/OptionData>) as u64` | Same as W12 Collection | `jit_arc_result_retain` / `_release` / `jit_arc_option_retain` / `_release` |

**Mixing carrier shapes would segfault**: the W11 legacy `jit_arc_release`
reads `*(bits as *const u32)` at offset 4 expecting a HeapHeader refcount.
For an `Arc::into_raw(Arc<HashSetData>) as u64` slot, offset 4 points into
the `HashSetData` payload (not a refcount), and decrementing it scribbles
on the data. The correct release for an Arc slot is
`Arc::decrement_strong_count::<HashSetData>(bits as *const HashSetData)`,
which decrements the Arc control block at offset -16.

---

## §6. Coordination with Round 8A — verified zero file-territory overlap

Round 8A territory:
- `crates/shape-jit/src/mir_compiler/terminators.rs` Call-terminator
  `print` arm — different MIR terminator path from method-call.
- `crates/shape-jit/src/ffi/` new per-HeapKind kinded `jit_print_*`
  entries — different FFI module surface from `ffi/call_method/` and
  `ffi/v2/collection_*`.

Round 8B territory:
- `crates/shape-jit/src/ffi/call_method/mod.rs` — `jit_call_method` shell.
- `crates/shape-jit/src/ffi/v2/collection_ctors.rs` (NEW) — typed-Arc ctors.
- `crates/shape-jit/src/ffi/v2/collection_arc_refcount.rs` (NEW) —
  per-HeapKind retain/release.
- `crates/shape-vm/src/executor/call_convention.rs` (CROSS-CRATE) —
  new public `jit_trampoline_call_method` method on `VirtualMachine`.
- `crates/shape-jit/src/ffi_symbols/v2_symbols.rs` — symbol registration.
- `crates/shape-jit/src/ffi_refs.rs` — FuncRef slots.
- `crates/shape-jit/src/compiler/ffi_builder.rs` — `r!(...)` lookups.
- `crates/shape-jit/src/mir_compiler/statements.rs` — EnumStore consumer
  collection_ctor arm.
- `crates/shape-jit/src/mir_compiler/{place_movement.rs, ...}` —
  retain/release dispatch by HeapKind arm.

**One shared touchpoint not in 8B's main territory**: 8A might extend
`retain_func_for_place` / `release_func_for_place` for HeapKinds it
touches (e.g. Result/Option for `print(Some(3))`). 8B extends the same
helpers for the 8 collection HeapKinds. If both 8A and 8B touch the same
match expression in those helpers, that's a merge conflict surface —
flagged here for supervisor awareness; resolution is mechanical (both
sub-clusters add disjoint arms to the same match).

---

## §7. Forbidden frames explicitly refused on sight

Per CLAUDE.md "Renames to refuse on sight" and the dispatch's forbidden-
frames list:

- **NOT** "MethodFnV2 bridge" / "MethodFn translator" / "dispatch-slice
  probe" / "boundary adapter for handler ABI" / "kind-injection helper"
  — §2.7.10/Q11 dispatch-ABI defection-attractor family. The new VM
  public API is named `jit_trampoline_call_method`, parallel to the
  existing `jit_trampoline_call_closure` (which is NOT a defection-
  attractor — it's a parallel-named utility for JIT trampoline calls).
  Its body is a kinded dispatch entry point, not a "bridge" between
  ABIs.
- **NOT** "collection-method bridge" / "method-dispatch translator" /
  "kind-injection adapter" / "value-call bridge" / "callee-kind helper"
  / "capture-injection adapter" — Round 8B dispatch's explicit forbidden
  list. The work shape is "typed-Arc allocation FFI for §2.7.X HeapKinds"
  + "delegate to VM-side kinded dispatch via new public API" + "EnumStore
  consumer arm" + "per-HeapKind retain/release entries".
- **NOT** "collection-FFI bridge" / "typed-Arc translator" / "container-
  allocation helper" — Round 7B audit §7 broader-family defection-
  attractor.
- **NOT** Bool-default fallback for inner-kind when statically
  underivable — Mutex inner-kind uses the (bits, kind) carrier-pair form
  per §2.7.5; Atomic / Lazy enforce inner-kind at JIT compile-time.
- **NOT** resurrecting `unified_box(HK_HASHSET, ...)` — wrong-type
  retain/release vs Arc per §5 above.
- **NOT** silent walkback. **NOT** "blame pre-existing". The broken
  `jit_call_method` shell is a §2.7.10/Q11 documented deferral; own the
  code quality.

---

## §8. Sites surfaced (cite-tracked, for cluster-1+ follow-up)

1. **`jit_call_method` collection-kind dispatch** — `crates/shape-jit/
   src/ffi/call_method/mod.rs:201-388`. The `heap_kind(receiver_bits)`
   NaN-box-tag dispatch cannot route typed-Arc receivers. **Load-bearing
   for Smoke 4 + HashMap smoke + Mutex smoke.**

2. **`dispatch_method_via_trampoline` extern-C `todo!()`** —
   `crates/shape-jit/src/ffi/call_method/mod.rs:179-199`. Replaced by
   the kinded-shell rebuild + VM-delegate path; the orthogonal structured-
   error fix from Round 7B audit §9(b) becomes unnecessary if the new
   shell never reaches this trampoline.

3. **Missing `VirtualMachine::jit_trampoline_call_method` public API** —
   `crates/shape-vm/src/executor/call_convention.rs` near line 953
   (alongside the existing `jit_trampoline_call_closure`). Cross-crate
   surface — Round 8B work crosses the shape-jit / shape-vm boundary.
   The dispatch instruction's "Touch points" list did not anticipate
   this cross-crate API addition.

4. **EnumStore consumer arm in `mir_compiler/statements.rs:239-268`** —
   Round 6C established the SURFACE; the typed-Arc allocator dispatch
   replaces the SURFACE arm with `is_collection_ctor` match on
   `variant_name` → 8 FuncRef call sites (5 zero-arg, 2 single-kind, 1
   carrier-pair Mutex).

5. **`retain_func_for_place` / `release_func_for_place` extension** —
   `crates/shape-jit/src/mir_compiler/` (Round 7A established the pattern
   for Result/Option). Round 8B extends with 8 new match arms.

6. **HashMap K/V kind threading is NOT an ADR-amendment trigger** (per
   Round 7B audit §9(d), confirmed by this audit). HashMapData stores
   `Arc<TypedBuffer<Arc<String>>>` keys + `Arc<TypedBuffer<Arc<HeapValue>>>`
   values — heterogeneous, single-shape carrier. Method dispatch on
   `m.set(k, v)` / `m.get(k)` uses the same kinded `&[KindedSlot]`
   carrier where K and V kinds come from per-arg slot kinds at the
   dispatch shell. No Q15 Route A monomorphization conflict for
   non-Array HeapKinds.

7. **Lazy's `l.get()` closure-call path** — `Lazy::get` semantics call
   the captured initializer closure. This re-enters the value-call path
   §2.7.11/Q12. **The JIT-side delegation to `vm.jit_trampoline_call_method`
   inherits the VM's value-call path** — the VM's `v2_lazy_get` handler
   at `concurrency_methods.rs` already invokes the closure through the
   VM's own `call_value_immediate_nb`. **No additional JIT-side closure-
   call wiring needed for Lazy.get** — the delegation chain handles it.
   This is another consequence of the §2.1 architectural insight.

---

## §9. ADR-006 amendment status

**No ADR-006 amendment required for the audit itself.** This document is
an audit / disposition surface, not an amendment.

**No ADR-006 amendment required for the migration work, either** — the
§2.1 architectural insight (delegate to VM-side kinded dispatch via a new
trampoline API mirroring `jit_trampoline_call_closure`) means the JIT
side does NOT need any new ABI shape. The §2.7.10/Q11 ABI is already
correctly specified; the JIT crosses into the VM's existing kinded
dispatch entry, no JIT-side parallel ABI.

The previously-mentioned candidate amendments from Round 7B audit §12
(§2.7.10 closure-trigger extension + cross-§ carrier-shape note) are
**not necessary** under the delegation-based design. The carrier-shape
distinction (Arc vs UnifiedValue) is documented HERE (§5), not in the
ADR — the ADR's §2.7.6/Q8 already binds the carrier-API-bound rule;
mixing carrier shapes is a CHECK-12-style mechanical rule that
`verify-merge.sh` could enforce rather than a textual ADR clause.

---

## §10. Estimated landing scope

### §10.1 LoC estimate

| Item | LoC | Files |
|---|---|---|
| 8 typed-Arc ctor FFI bodies | ~150 | `ffi/v2/collection_ctors.rs` (NEW) |
| 16 retain/release FFI bodies | ~200 | `ffi/v2/collection_arc_refcount.rs` (NEW) |
| `jit_call_method` shell rebuild | ~150 | `ffi/call_method/mod.rs` (edit) |
| New `VirtualMachine::jit_trampoline_call_method` API | ~120 | `executor/call_convention.rs` (edit) — CROSS-CRATE |
| EnumStore consumer collection_ctor arm | ~80 | `mir_compiler/statements.rs` (edit) |
| Symbol registration (24 symbols: 8 ctors + 16 ret/rel) | ~80 | `ffi_symbols/v2_symbols.rs` (edit) |
| FuncRef slots (24 fields) | ~60 | `ffi_refs.rs` (edit) |
| `r!(...)` lookups (24 entries) | ~40 | `compiler/ffi_builder.rs` (edit) |
| `retain_func_for_place` / `release_func_for_place` extension | ~30 | `mir_compiler/` (edit) |
| Tests — 8 ctor round-trips + 8 retain/release pairs + 4 smoke tests | ~400 | new test modules |
| **Total** | **~1310 LoC** | **~9 files** |

### §10.2 Sub-cluster split possibility

The scope CAN be split into two sub-clusters with clean handoff:

- **8B.1 — Typed-Arc allocation FFI + kinded retain/release**
  (~580 LoC, ~5 files):
  - 8 typed-Arc ctor FFI bodies
  - 16 retain/release FFI bodies
  - 24 symbol registrations
  - 24 FuncRef slots
  - 24 `r!(...)` lookups
  - `retain_func_for_place` / `release_func_for_place` extension
  - Close criterion: `cargo check --workspace --lib --tests` EXIT=0 +
    FFI round-trip tests for the 24 new entries. **Does NOT close
    Smoke 4** — the EnumStore consumer still surfaces. This is the
    "Round 5B-style preparatory close" Round 7B audit §5 described.

- **8B.2 — `jit_call_method` shell rebuild + VM trampoline API +
  EnumStore consumer arm**
  (~730 LoC, ~4 files):
  - New `VirtualMachine::jit_trampoline_call_method` public API
  - `jit_call_method` shell rebuild reading from `stack_kinds`
  - EnumStore consumer collection_ctor arm dispatching to 8B.1's ctors
  - Smoke tests: Smoke 4 (`Set`), HashMap, Mutex VM == JIT
  - Close criterion: Smoke 4 + HashMap + Mutex VM == JIT.

### §10.3 Single-round budget assessment

The dispatch instruction asks: *"If your audit also finds that the
integrated trinity scope exceeds a single round's reasonable budget,
surface as audit-only close..."*.

**Assessment**: 1310 LoC + cross-crate API + ~24 symbol registrations +
~400 LoC of tests, all touching ~9 files including a load-bearing public
API on `VirtualMachine`, is **at the high end of a single-round budget**.
For reference:
- W11-jit-carrier-conversion Round 2 (close `2960f5cf`): ~600 LoC,
  single-crate.
- W12-jit-result-option-trinity Round 7A (close `63d59b84`): ~800 LoC,
  single-crate.

This sub-cluster would be **1.5×-2× the largest closed sub-cluster's
scope**, with a cross-crate API addition. It is **executable in a
single round** but high-effort, with a meaningful risk of mid-round
budget pressure tempting the walk-back pattern CLAUDE.md "Forbidden
rationalizations" enumerates.

The safer disposition is **audit-only close + recommend dispatching as
8B.1 + 8B.2 sub-clusters**. Each sub-cluster is then well-sized
(~600-700 LoC, comparable to Round 7A) and the 8B.2 close gate directly
exercises Smoke 4 / HashMap / Mutex equivalence.

---

## §11. Disposition recommendation

**Close audit-only**, recommend split into **W12-jit-collection-arc-ffi-
ctors-and-refcount (8B.1)** + **W12-jit-call-method-shell-rebuild (8B.2)**
sub-clusters.

Reasoning:

1. **Architectural insight is load-bearing**: Round 7B's audit identified
   the trinity but mis-estimated item (iii) as a 50-entry mirror. This
   audit corrects: the JIT side does NOT mirror the VM's PHF maps;
   it delegates to the VM-side kinded dispatch via a new public API.
   This is **smaller** scope but still substantial — ~1310 LoC across
   ~9 files including a cross-crate public API.

2. **Cross-crate API was not in the dispatch's touch-points list**.
   Adding `VirtualMachine::jit_trampoline_call_method` is principled
   work (parallel to existing `jit_trampoline_call_closure`), but it
   was not anticipated as Round 8B scope. Surfacing this for
   supervisor sizing decision.

3. **Single-round budget pressure**: 1.5×-2× the largest closed sub-
   cluster. Even with the §2.1 simplification, this is high-effort
   work. The W11-round-1 walk-back precedent applies: under budget
   pressure, the temptation to "just land the allocators and skip the
   shell rebuild" → SIGABRT regression. **Splitting into 8B.1 + 8B.2
   keeps each sub-cluster well-sized and the 8B.2 close gate is the
   equivalence-ratchet anchor**.

4. **No code changes that regress Round 6C/7B's clean SURFACE state**
   landed by this audit. Audit doc only.

If the supervisor opts to land 8B as a single sub-cluster despite the
size estimate, this audit becomes the design doc — the §10.1 LoC table
maps each line item to its file destination.

---

## §12. Close-gate verification (audit-only commit)

This audit commit is documentation-only — no source changes, no FFI
additions, no MIR changes. Close gates run for completeness:

- `cargo check --workspace --lib --tests` — expected EXIT=0 (no
  changes to compile).
- `cargo test -p shape-jit --lib` — expected unchanged from Round 7A
  baseline (after Round 7B audit-only landed at `7dc0ce5d`).
- `bash scripts/verify-merge.sh` — expected 12/12.
- `bash scripts/check-no-dynamic.sh` — expected EXIT=0.
- Smoke 4 / HashMap / Mutex smokes — UNCHANGED (Round 6C SURFACE
  state preserved verbatim under `--mode jit`; VM mode prints correct
  values).

The audit is content-only; gates verify no regression.

---

*End of audit. Branch: `bulldozer-strictly-typed-w12-jit-collection-
method-dispatch-abi`. Audit commit hash: pending (will be backfilled at
close).*
