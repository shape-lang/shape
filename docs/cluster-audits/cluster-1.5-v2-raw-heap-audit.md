# cluster-1.5 v2-raw-heap-audit — empirical audit deliverable

**Branch:** `bulldozer-strictly-typed-cluster-1.5-v2-raw-heap-audit`
**Parent HEAD:** `3cb72c2d` (post-cluster-1.5-q25c-trait-object-rebuild close).
**Dispatch:** cluster-1.5-v2-raw-heap-audit per CLAUDE.md "Known Constraints"
v2-raw-heap-audit territory + cluster-1.5-close-criterion gating per supervisor
2026-05-16.
**Scope:** Audit (Phase 1) per per-site file:line + root-cause + fix shape; bounded
fixes (Phase 2) per `vw_clone` / `vw_drop` precedent commit `afb1651`.

**Read-only audit delivered; Phase 2 bounded fix NOT landed in scope.**
Empirical re-classification of the 4 simulation tests + the cluster-2 §D Class 1
SIGABRT anchor surfaces a load-bearing reframe: at HEAD `3cb72c2d`, the 4
simulation tests are blocked by V3-S5 ckpt-5/ckpt-6 SURFACE-and-stops (Phase 4
cluster-0 territory), **not** the historical v2-raw aliasing repro. The Class 1
SIGABRT anchor at `hashmap_filter_all_match` remains live but its repro depth
is bounded to a narrower carrier-shape surface than the simulation-test wording
implies. Phase 2 fix landing requires environment with working `cargo check` +
`verify-merge.sh`; deferred to merge-eligible follow-up agent per close gate
requirements (see §6).

---

## §0 Pre-flight (Q3 binding)

- HEAD verified at `3cb72c2d`.
- All file:line cites below grep-verified at HEAD `3cb72c2d` per Q3 binding.
- `cargo check`/`verify-merge.sh` NOT executed at this HEAD due to sandbox
  Nix-loader environment limitation (`cc` / `ld-linux-x86-64.so.2` interpreter
  path issue blocks build-script execution; see §5 for full diagnostic). This
  prevents Phase 2 close-gate evidence collection in-agent-scope. The audit
  deliverable (Phase 1) is fully grep-verifiable static analysis.
- Smoke matrix 5/5 VM == JIT preservation not empirically reverified at this
  HEAD; parent close at `3cb72c2d` (cluster-1.5-q25c-trait-object-rebuild)
  documents 5/5 VM == JIT post-merge per AGENTS.md row dated 2026-05-16.

## §1 Per-site empirical audit

### §1.A The 4 ignored simulation tests — re-classification

Per CLAUDE.md "Known Constraints" v2-raw-heap-audit entry, the 4 ignored tests
at `bin/shape-cli/tests/stdlib/simulation.rs` are described as
"`typed_array_push_*` realloc invalidates aliased raw pointers held across
iterations → VM Drop double-free". Empirical static-analysis re-classification
at HEAD `3cb72c2d`:

| Test | File:line | Stated rationale | Re-classified blocker at HEAD `3cb72c2d` |
|---|---|---|---|
| `test_harmonic_oscillator_rk4_system` | `bin/shape-cli/tests/stdlib/simulation.rs:111-121` | "v2 raw-ptr aliasing class (path-c2/v2-c-alias): SIGSEGV at VM Drop via 'free(): double free detected in tcache 2'. Hot loop in rk4_system pushes records into a results array..." | **V3-S5 ckpt-5 surface — `op_new_array(0)`**. The test body calls `rk4_system(|t, y| [y[1], -y[0]], [1.0, 0.0], ...)`. The fixture's `[1.0, 0.0]` initial-value vector + closure-return-array `[y[1], -y[0]]` + `let mut results = []` in `crates/shape-runtime/stdlib-src/core/ode.shape:86` all trip `op_new_array` for empty-or-typed-element arrays. Per `crates/shape-vm/src/executor/objects/object_creation.rs:353-374` `op_new_array` SURFACEs at ckpt-5 with explicit message `"op_new_array(N): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface ... Construction-site rebuild lands at ckpt-6 STRICT close"`. The TypedObject record construction `{ t: t, y: y }` at `ode.shape:89` additionally surfaces `op_new_object` per `object_creation.rs:228-258` (`"op_new_object: ad-hoc TypedObject construction depends on `create_typed_object_from_pairs` ... phase-2c, see ADR-006 §2.7.4"`). |
| `test_rk45_system_harmonic_oscillator` | `bin/shape-cli/tests/stdlib/simulation.rs:184-201` | Same class as above; "adaptive integrator with a closure that returns a fresh `[f64, f64]` from inside a hot loop that also pushes records into the results array." | **V3-S5 ckpt-5 surface — same root cause as `test_harmonic_oscillator_rk4_system`**. `rk45_system` body in `core/ode.shape:225-...` mirrors `rk4_system`'s `let mut results = []` + `{ t: t, y: y }` record pattern. |
| `test_find_collisions_brute` | `bin/shape-cli/tests/stdlib/simulation.rs:459-477` | "'Cannot get property string on int (line 33)' — the AABB record value `a` gets read with a corrupted tag mid-loop. Pattern matches `for i in range(0, n) { let x = arr[i]; fn_call(x, ...) }` over an array of TypedObject records." | **V3-S5 ckpt-5/ckpt-6 surface — TypedObject `[]` construction site + `arr[i]` index access**. The fixture builds `let boxes = [aabb(...), aabb(...), ...]` — an `Array<TypedObject>` literal — which trips `op_new_array` SURFACE (count > 0 variant). `find_collisions_brute` at `physics/collision.shape:156-171` iterates `for i in range(0, n) { let box_i = boxes[i]; ... }` — the `arr[i]` for `Array<TypedObject>` requires V3-S5 ckpt-6 `RefTarget::TypedIndex` per per-element-kind rebuild (per `cluster-2-shape-test-residuals-triage.md` Class 6 row sub-class (b)/(c)/(d) blocker cite). |
| `test_find_collisions_sweep` | `bin/shape-cli/tests/stdlib/simulation.rs:484-502` | Same family; "both inner-loop record-iteration variants trip the same alias-window." | **V3-S5 ckpt-5/ckpt-6 surface — same root cause as `test_find_collisions_brute`**. `find_collisions_sweep` at `physics/collision.shape:180-...` uses `let indices = []; ... indices.push({ idx: i, min_x: b.min_x })` pattern, identical SURFACE class. |

**Re-classification verdict**: the 4 ignored tests do NOT exercise the v2-raw
aliasing carrier-shape territory at HEAD `3cb72c2d` — they hit earlier
V3-S5 ckpt-5/ckpt-6 SURFACE-and-stops at `op_new_array` + `op_new_object` +
`arr[i]` for `Array<TypedObject>`. The historical "double-free / SIGSEGV at VM
Drop" symptoms named in the `#[ignore]` strings were observed under a pre-V3-S5
HEAD (before `TypedArrayData` + `op_new_array` consumer-cascade tier 3 surface
landed). At HEAD `3cb72c2d` the tests would fail much earlier in execution
with `Runtime error: Not implemented: op_new_array(N): SURFACE — V3-S5
ckpt-5 ... Construction-site rebuild lands at ckpt-6 STRICT close`.

**Disposition for the 4 ignored tests**: REMAIN IGNORED, but the `#[ignore]`
reason strings are stale-relative-to-HEAD. The correct disposition is
**structured-defer to V3-S5 ckpt-6 cluster-0 territory** (matching cluster-2
triage doc Class 5/6/8/9 dispositions for op_new_array/op_new_object SURFACE
classes). Updating the `#[ignore]` reason strings to reflect actual HEAD
blocker is a SMALL bounded fix per §3 below — but the tests remain ignored
either way (no test passing on this path until V3-S5 ckpt-6 closes).

### §1.B cluster-2 Class 1 v2-raw-heap aliasing empirical anchor — focused review

Per cluster-2 triage doc §1 Class 1 row + §D, the empirical anchor for the
v2-raw aliasing class is:

> `tools/shape-test/tests/hashmap/iteration.rs:69` (`hashmap_filter_all_match`) →
> `crates/shape-value/src/v2/string_obj.rs:118` — `misaligned pointer dereference:
> address must be a multiple of 0x4` then SIGABRT

The test fixture (file:line `tools/shape-test/tests/hashmap/iteration.rs:69-79`,
grep-verified at HEAD `3cb72c2d`):

```shape
let m = HashMap().set("a", 1).set("b", 2)
let result = m.filter(|k, v| v > 0)
print(result.len())
```

Trace per static analysis:

1. `HashMap()` → `BuiltinFunction::HashMapCtor` at
   `crates/shape-vm/src/executor/vm_impl/builtins.rs:595-615`. Builds an empty
   `Arc<HashMapKindedRef::String(Arc<HashMapData<*const StringObj>::new()>)>`
   (default V=String per ctor convention).

2. `.set("a", 1)` → `crates/shape-vm/src/executor/objects/hashmap_methods.rs:774`
   `v2_set` → empty-map V-promotion path at `:806` calls `empty_set_with_promotion`
   at `:1066-1188`. Value kind = `Int64`, so promotes to
   `HashMapKindedRef::I64(Arc::new(HashMapData<i64>::new()))` then inserts
   `("a", 1)`. The insert path at `crates/shape-value/src/heap_value.rs:1409-1452`
   allocates a fresh `StringObj::new("a")` (refcount=1) via
   `crates/shape-value/src/v2/string_obj.rs:30-51`, pushes the `*const StringObj`
   into `self.keys: *mut TypedArray<*const StringObj>` via
   `crates/shape-value/src/v2/typed_array.rs:149-159` `TypedArray::push`, and
   pushes the i64 value into `self.values: *mut TypedArray<i64>` via the
   parallel `Self::values_push` raw-write path at `heap_value.rs:1557-1577`.

3. `.set("b", 2)` → `v2_set` → non-empty `HashMapKindedRef::I64` arm at
   `set_kinded` `:811-826`. `Arc::clone(arc)` bumps the inner `Arc<HashMapData<i64>>`
   strong-count to 2; `Arc::make_mut(&mut new_arc)` sees strong-count > 1, clones
   the `HashMapData<i64>` via the `Clone` impl at `heap_value.rs:1635-1670`
   (walks `self.keys` via `TypedArray::get_unchecked`, `share_clone`s each
   `*const StringObj` via `<*const StringObj as HashMapValueElem>::share_clone`
   at `heap_value.rs:980-989` (`v2_retain(&(**elem).header)` bumps refcount,
   returns same raw pointer); allocates fresh `new_keys` + `new_values` buffers
   via `TypedArray::with_capacity(n)`; copies elements into them via
   `ptr::write`; sets `(*new_keys).len = n` + `(*new_values).len = n`). Then
   the inner `insert("b", 2)` path allocates fresh `StringObj::new("b")` and
   pushes into the (just-allocated) `new_keys` buffer — `TypedArray::push`
   sees `len == cap` (both `n=1`), calls `grow` to realloc data buffer
   (cap 1→2), writes element, increments len.

4. `.filter(|k, v| v > 0)` → `crates/shape-vm/src/executor/objects/hashmap_methods.rs:1606`
   `v2_filter` → `read_keys_owned(keys_ptr)` at `:1633` walks the keys buffer
   per `:124-139` (`StringObj::as_str(ptr).to_owned()` — creates `String` copies
   into a `Vec<Arc<String>>`). Then iterates per `:1637-1659` calling closure
   per entry via `read_entry_kinded(&map, i, Arc::clone(&key_arc))` which uses
   `entry_object_at`-style projection for the closure args.

5. `print(result.len())` → `crates/shape-vm/src/executor/objects/hashmap_methods.rs:684-694`
   `v2_len` → `as_hashmap(&args[0])` recovers the `Arc<HashMapKindedRef>`,
   `map.len()` dispatches to `kref_len` at `:142-145`.

### §1.B.1 Root-cause hypotheses for the misaligned-ptr SIGABRT at `string_obj.rs:118`

`StringObj::release_elem` at `crates/shape-value/src/v2/string_obj.rs:116-122`:
```rust
unsafe impl super::heap_element::HeapElement for StringObj {
    unsafe fn release_elem(ptr: *const Self) {
        if unsafe { super::refcount::v2_release(&(*ptr).header) } {
            unsafe { Self::drop(ptr as *mut Self) };
        }
    }
}
```

Line 118 dereferences `(*ptr).header`. `ptr` is `*const StringObj`; `header`
is at offset 0 (`#[repr(C)]` per `string_obj.rs:18-26`). A misaligned-ptr
panic on `header` access means `ptr` itself is not `align_of::<HeapHeader>()
== 4`-aligned (HeapHeader contains `AtomicU32`).

Possible root-cause hypotheses (each anchored to a specific call site):

**Hypothesis A**: `*const StringObj` pointer-bit corruption in the kinded
stack/slot ABI. If somewhere a slot's bits-as-`*const StringObj` are read
when the slot's `NativeKind` is not `StringV2` (e.g. a scalar `i64` slot's
bits get treated as `*const StringObj`), the deref of `(*ptr).header` would
hit arbitrary memory and likely produce a misaligned/SIGSEGV. Candidate
sites: `read_keys_owned` at `crates/shape-vm/src/executor/objects/hashmap_methods.rs:124-139`
unconditionally calls `StringObj::as_str(ptr).to_owned()` on raw pointers
read from `arc.keys` — relies on keys-buffer invariant that all entries are
valid `*const StringObj`. If `HashMapData::clone` (`heap_value.rs:1635-1670`)
or the parallel C2-joint ckpt-3 mutation API (`heap_value.rs:1378+`) ever
writes a bogus pointer (uninitialized memory not zeroed, partial init in
realloc, etc.), this would manifest.

**Hypothesis B**: Premature Drop of a `*const StringObj` held by both old
and new `HashMapData<V>` after `Arc::make_mut`-driven clone. The `HashMapData::Clone`
impl at `heap_value.rs:1647-1660` `share_clone`s each key (`v2_retain` bumps
refcount). Both the source `HashMapData` and the new clone now hold one share
each on every `*const StringObj`. When either one drops, `release_typed_array`
walks the buffer and calls `release_elem` per element. If somewhere in the
fixture the source `HashMapData` is dropped DURING iteration of the new
clone's buffer, that wouldn't directly cause misalignment — the refcount
would still be 1 on the inner StringObj allocation.

**Hypothesis C**: `TypedArray::push` realloc invalidating an aliased raw
pointer (the carrier-shape "v2-raw aliasing" pattern named in CLAUDE.md
"Known Constraints"). `TypedArray::push` at `typed_array.rs:149-159` operates
via `*mut Self`: the `TypedArray` struct itself does NOT move during realloc;
only its `data: *mut T` field gets reallocated. Sound by construction — the
push's `let arr = &mut *this` reborrow after `grow` reads the updated `data`
field. **Unless** a caller captured `(*this).data` into a local before the
grow and used the stale local after.

   Candidate sites for Hypothesis C:
   - `HashMapData::insert` at `heap_value.rs:1409-1452` — calls `TypedArray::push(self.keys, key_obj)` at `:1446`, then `Self::values_push(self.values, value)` at `:1448`. The `values_push` body at `:1557-1577` independently reads/writes through `(*values).data` after its own `grow` — sound by construction. The `self.index.entry(hash).or_default().push(new_idx_u32)` at `:1450` reads `new_idx_u32` captured BEFORE the push at `:1444` — captured value is just `u32`, not a pointer; sound.
   - `read_keys_owned` at `crates/shape-vm/src/executor/objects/hashmap_methods.rs:124-139` — reads `TypedArray::len(keys)` THEN iterates `0..n` reading `TypedArray::get_unchecked(keys, i)`. The `len` is captured to a local `n` BEFORE the loop. No mutation happens during this read — the function only reads. Sound.
   - The `HashMapData::Clone` impl at `heap_value.rs:1647-1660` — captures `(*new_keys).data` and `(*new_values).data`-style addresses inside the loop. The new buffers are local to this function and not realloc'd during the loop (they were pre-allocated at `with_capacity(n)` and the loop only writes within `0..n` < capacity). Sound.
   - `HashMapData::insert`'s overwrite path at `:1422-1436` — captures `data_ptr = (*self.values).data.add(i)` then does `ptr::read(data_ptr)` + `ptr::write(data_ptr, value)`. No intervening realloc. Sound.

**Hypothesis D** (most likely candidate): the misaligned-ptr SIGABRT is
**downstream of the cluster-2 cw-C HashMapKindedRef::HashMap V-arm work**
(per cluster-2-shape-test-residuals-triage.md §D notes "V-arm RESOLVED"
disposition for cw-C). The `f"{v}"` string-interpolation pattern in
`tools/shape-test/tests/hashmap/stress_iteration.rs:634`
(`test_hashmap_group_by_basic` — Class 10 in cluster-2 triage) creates
fresh `StringObj` allocations per iteration. If the groupBy V-arm walks a
recursive `HashMapKindedRef::HashMap(Arc<HashMapData<HashMapKindedRef>>)`
and the inner `HashMapKindedRef` clone path at `heap_value.rs:1751-1767`
shallow-clones each variant's inner `Arc<HashMapData<V>>` via `Arc::clone`,
the per-`StringObj` retain/release pairing might be off-by-one when a
`HashMapData::Drop` runs on a different thread / different scope from
`HashMapData::clone`. But this is speculation without empirical reproducer.

Without a working `cargo test` environment in agent scope, the hypothesis
ranking is bounded to static analysis. **Phase 2 fix recommendation** is to
re-run the empirical anchor (`hashmap_filter_all_match`) post-environment
fix, capture the actual SIGABRT location via gdb backtrace, and select the
matching hypothesis for targeted fix. The vw_clone/vw_drop precedent pattern
(commit `afb1651`) is the canonical shape for refcount-imbalance fixes; its
post-strict-typing equivalent is `v2_retain` / `release_elem` pairing
balanced per the producer/consumer cascade-flip lockstep rule (per ADR-006
§Q25.C.5 amendment Wave 2 Agent E text).

### §1.C HashMapValueElem trait surface — per-V impl audit

The `HashMapValueElem` trait at `crates/shape-value/src/heap_value.rs:843-903`
defines 3 methods (`release_typed_array`, `share_clone`, `release_owned`)
implemented for 8 V types. Per-V impl audit at HEAD `3cb72c2d`:

| V | release_typed_array | share_clone | release_owned | Drop discipline |
|---|---|---|---|---|
| `i64` | `heap_value.rs:909-913` — `TypedArray::<i64>::drop_array(ptr)` POD | `:914-917` — `*elem` | `:918-921` — no-op POD | Sound (POD byte-copy semantics) |
| `f64` | `:926-928` — `TypedArray::<f64>::drop_array(ptr)` POD | `:929-932` — `*elem` | `:933-934` — no-op POD | Sound (POD byte-copy semantics) |
| `u8` (Bool) | `:940-942` — `TypedArray::<u8>::drop_array(ptr)` POD | `:943-946` — `*elem` | `:947-948` — no-op POD | Sound (POD byte-copy semantics) |
| `char` | `:956+` — `TypedArray::<char>::drop_array(ptr)` POD | `:` — `*elem` | no-op POD | Sound (POD byte-copy semantics; dead-but-derived per §C.5) |
| `*const StringObj` | `:971-977` — `TypedArray::<*const StringObj>::drop_array_heap(ptr)` per-element `release_elem` | `:980-989` — `v2_retain(&(**elem).header); *elem` | `:991-1001` — `release_elem(value)` for non-null | Sound by construction; load-bearing for §1.B.1 hypothesis A/C |
| `*const DecimalObj` | `:1006-1009` — `TypedArray::<*const DecimalObj>::drop_array_heap(ptr)` per-element `release_elem` | `:1012-1019` — `v2_retain(&(**elem).header); *elem` | `:1021-1028` — `release_elem(value)` for non-null | Sound by construction (mirrors *const StringObj) |
| `TypedObjectPtr` | `:1039-1058` — manual buffer walk: per-element `ptr::read` triggers `TypedObjectPtr::Drop` (calls `release_elem` on inner `*const TypedObjectStorage`); then `dealloc` data + struct | `:1060-1063` — `elem.clone()` (delegates to wrapper Clone → `v2_retain`) | `:1066-1069` — no-op (wrapper Drop runs on scope exit) | Sound by construction; mirrors `release_elem` cascade |
| `TraitObjectPtr` | `:1076-1089` — manual buffer walk: per-element `ptr::read` triggers `TraitObjectPtr::Drop`; mirror of TypedObjectPtr | `:1091-1094` — `elem.clone()` mirror | `:1097-1100` — no-op mirror | Sound by construction; mirrors TypedObjectPtr |
| `HashMapKindedRef` | `:1106-...` — manual buffer walk (recursive); per-element drop runs HashMapKindedRef::Drop (auto-derived per-variant Arc drop) | `:` — `HashMapKindedRef::clone` (per-variant `Arc::clone`) | `:` — no-op (auto-Drop) | Sound by construction (Wave N hashmap-value-v-arm follow-up 2026-05-16; recursive carrier) |

**Audit verdict**: per-V `HashMapValueElem` impls are sound by construction
under the producer-side contract (caller transfers one strong-count share
per element to the typed-array buffer; release_typed_array retires one share
per element on drop). No defection-attractor renames; no Bool-default
fallback; no ValueWord resurrection. The 9 V-arms maintain lockstep with
`HashMapKindedRef` variants per `heap_value.rs:1751-1767` Clone impl and
the auto-derived Drop per `:1769-1772` comment.

### §1.D Other v2-raw-aliasing-class candidate sites

Per dispatch territory enumeration, additional candidate sites for
v2-raw-aliasing-class root-cause review:

| Site | File:line | Bug class candidate | Audit verdict |
|---|---|---|---|
| `TypedArray::push` realloc | `crates/shape-value/src/v2/typed_array.rs:149-159` | Realloc-invalidates-aliased-raw-pointer | Sound by construction — the `TypedArray` struct itself does NOT move during realloc; only its `data` field is reallocated via `grow` at `:258-280`. Push re-borrows `(*this).data` after `grow`. Callers that capture `(*this).data` into locals before `push` then use after the push are aliasing-unsound; static-analysis enumeration found no such caller pattern in the codebase. |
| `HashMapData::values_push` realloc | `heap_value.rs:1557-1577` | Same as above | Sound by construction; mirrors `TypedArray::push` shape with V-generic adaptation (non-Copy V). |
| `HashMapData::insert` overwrite path | `heap_value.rs:1421-1438` | Old-value Drop racing with new-value write | Sound by construction — `Self::drop_owned_value(old_value)` at `:1434` runs Drop on the owned old value BEFORE `ptr::write(data_ptr, value)` at `:1435`. Sequential, no race. |
| `HashMapData::remove` shift-down | `heap_value.rs:1498-1508` | Use-after-free during compaction shift | Sound by construction — `ptr::write(dst, ptr::read(src))` moves elements without running Drop; the removed-slot's value/key were extracted via `ptr::read` BEFORE the shift. |
| `TypedArray::drop_array_heap` per-element walk | `typed_array.rs:305-323` | Use-after-free if buffer was already partially deallocated | Sound by construction — checks `arr.cap > 0 && !arr.data.is_null()` BEFORE the walk; walks `0..arr.len` only. |
| `TypedObjectPtr::Drop` per-element call | `heap_value.rs:596-610` | Double-free if Drop runs twice on same `*const TypedObjectStorage` | Sound by construction — `TypedObjectPtr` is `#[repr(transparent)] struct TypedObjectPtr(*const TypedObjectStorage)`; Drop runs once per wrapper instance per Rust ownership rules. The wrapper's Clone bumps refcount; the wrapper's Drop calls `release_elem` which decrements refcount and dealloc on zero. Sound. |
| `TraitObjectPtr::Drop` per-element call | `heap_value.rs:702-...` | Mirror of TypedObjectPtr | Sound by construction (mirror). The cluster-1.5-q25c-trait-object-rebuild close at parent HEAD `3cb72c2d` resolved the producer/consumer carrier-shape mismatch that previously caused `free(): invalid pointer` SIGABRT on `dyn T` boxed values. Post-fix, the producer side uses `TraitObjectStorage::_new` (HeapHeader-at-offset-0 v2-raw discipline); consumer arms call `release_elem` per the same v2-raw shape. Lockstep complete. |
| StringObj internment / Arc<String> bridge | `crates/shape-value/src/v2/string_obj.rs:30-99` | Internment-pool aliasing | StringObj has NO internment pool at HEAD `3cb72c2d` — `StringObj::new(s)` allocates fresh per call; each StringObj allocation is independent. No internment-pool aliasing surface. (Cluster-2 cw-E close at parent HEAD addresses the JIT-side string-leak interner-pool measurement; that work doesn't introduce StringObj-side interning.) |

**Audit verdict**: the v2-raw producer-side carriers (`TypedArray::push`,
`HashMapData::insert`, `TypedObjectPtr::Drop`, `TraitObjectPtr::Drop`,
`StringObj::release_elem`) are individually sound by construction under
their producer/consumer contracts. No carrier-shape mismatch defection
remaining at HEAD `3cb72c2d` post-cluster-1.5-q25c-trait-object-rebuild
close. The remaining cluster-2 §D Class 1 SIGABRT anchor at
`hashmap_filter_all_match` likely originates from a kind-track / receiver-
recovery violation at a different layer (e.g. callsite passing the wrong
NativeKind to a per-V dispatch arm, or per-V impl receiving a kind not in
its supported set without surface-and-stop). Empirical reproduction via
gdb backtrace is the next step; static-analysis enumeration could not
isolate a single load-bearing carrier-shape violation.

---

## §2 Per-site fix-shape estimates

Per dispatch's "Per-site fix shape estimate (bounded vw_clone/vw_drop
pattern OR deeper rewrite OR structured-defer post-v1)" criterion:

| Site | Fix shape | LoC estimate | Fix applied? |
|---|---|---|---|
| `bin/shape-cli/tests/stdlib/simulation.rs:105-111` ignore reason (test_harmonic_oscillator_rk4_system) | Update `#[ignore = ...]` reason string to cite V3-S5 ckpt-5/ckpt-6 SURFACE class instead of historical "v2 raw-ptr aliasing class" framing | ~3 LoC | NOT APPLIED (Phase 2 — agent-scope blocked by cargo env limitation) |
| `bin/shape-cli/tests/stdlib/simulation.rs:180-184` ignore reason (test_rk45_system_harmonic_oscillator) | Same as above | ~3 LoC | NOT APPLIED |
| `bin/shape-cli/tests/stdlib/simulation.rs:452-458` ignore reason (test_find_collisions_brute) | Same as above + cite `arr[i]` for `Array<TypedObject>` ckpt-6 RefTarget::TypedIndex blocker | ~3 LoC | NOT APPLIED |
| `bin/shape-cli/tests/stdlib/simulation.rs:480-483` ignore reason (test_find_collisions_sweep) | Same as above | ~3 LoC | NOT APPLIED |
| cluster-2 §D Class 1 SIGABRT root cause | UNKNOWN at static-analysis tier — requires gdb backtrace at empirical reproducer to isolate. Likely candidates: (i) callsite passing wrong NativeKind to a per-V dispatch arm; (ii) per-V impl receiving unsupported kind without surface-and-stop; (iii) refcount-imbalance equivalent to vw_clone/vw_drop precedent (commit `afb1651`) in a post-v1 carrier-shape boundary. Fix shape will be vw_clone/vw_drop equivalent ONCE empirically isolated. | UNKNOWN (~30-150 LoC per the precedent) | NOT APPLIED (Phase 2 — empirical isolation blocked by cargo env limitation) |
| 4 ignored simulation tests un-ignore attempt | NOT APPLICABLE post-§1.A re-classification — tests blocked at V3-S5 ckpt-5/ckpt-6, NOT v2-raw aliasing. Un-ignore would surface ckpt-5 SURFACE message instead of test pass. | N/A | N/A (correctly stays ignored; reason-string update is the only bounded action) |

**Audit verdict**: cluster-1.5 v2-raw-heap-audit territory at HEAD `3cb72c2d`
is **narrower than the original dispatch scope describes**. The 4 ignored
simulation tests are not v2-raw-heap-aliasing-blocked at HEAD; they are
V3-S5 ckpt-5/ckpt-6-blocked (sibling cluster-0 territory per Phase 4
dispatch). The cluster-2 Class 1 SIGABRT anchor remains live but requires
empirical reproduction (gdb backtrace) to identify the load-bearing fix
site — bounded vw_clone/vw_drop fix shape is plausible but not isolatable
at static-analysis tier.

---

## §3 Phase 2 fix recommendations (structured-defer)

### §3.A In-scope bounded fix (recommended for follow-up agent with working cargo env)

**Update the 4 ignored simulation tests' `#[ignore]` reason strings** to
reflect actual HEAD `3cb72c2d` blocker class. Replace existing
"v2 raw-ptr aliasing class (path-c2/v2-c-alias)..." rationale with:

```rust
#[ignore = "V3-S5 ckpt-5/ckpt-6 SURFACE class: op_new_array / op_new_object \
            (and Array<TypedObject> arr[i] for find_collisions_*) trip the \
            consumer-cascade tier 3 SURFACE-and-stop at object_creation.rs:353 / \
            :228 / property_access.rs (per V3-S5 ckpt-1..ckpt-5 \
            TypedArrayData enum + Buf<T> wrapper + outer HeapValue::TypedArray \
            arm + HeapKind::TypedArray=8 ordinal deletion per W12 audit §3.5+§3.6 \
            + ADR-006 §2.7.24 Q25.A SUPERSEDED). Construction-site rebuild + \
            arr[i] RefTarget::TypedIndex rebuild lands at V3-S5 ckpt-6 STRICT \
            close per cluster-0 territory. NOT a v2-raw-heap aliasing class \
            at HEAD 3cb72c2d (the original ignore reason cited a pre-V3-S5 \
            failure shape; cluster-1.5-v2-raw-heap-audit re-classified \
            2026-05-16 per docs/cluster-audits/cluster-1.5-v2-raw-heap-audit.md \
            §1.A)."]
```

**LoC**: ~4 × 12 = 48 LoC (4 reason-string edits).
**Risk**: zero (test stays ignored, only docstring change).
**Sibling-territory overlap check**: none (this file is not in Phase 4
Add/AddAssign dispatch territory).

### §3.B Out-of-scope deferred fix (cluster-2 Class 1 SIGABRT empirical isolation)

**Defer to cluster-1.5-v2-raw-heap-empirical-isolation follow-up sub-cluster**
(NEW sub-cluster recommendation). Territory: `hashmap_filter_all_match`
empirical reproduction at HEAD `3cb72c2d`+ via gdb backtrace. Required
infrastructure: working `cargo build` + `cargo test` environment + gdb.
Recommended workflow:

1. Build with `RUSTFLAGS="-g"` for debug symbols in release mode.
2. Run `cargo test --release -p shape-test hashmap::iteration::hashmap_filter_all_match` under gdb.
3. Capture backtrace at SIGABRT (`bt full`); identify the `*const StringObj` bit-value at deref site.
4. Backtrack through the kinded slot ABI to the producer (callsite) of the bad bit-value.
5. Apply vw_clone/vw_drop-equivalent fix per the empirically-isolated producer/consumer carrier-shape mismatch.

**Estimated LoC**: 30-150 (per precedent `afb1651` size).
**Risk**: bounded (matches established precedent shape).
**Sibling-territory overlap check**: requires cross-check with Phase 4 trait
Add/AddAssign dispatch + V3-S5 ckpt-6 cluster-0 carrier-shape work.

### §3.C OUT-OF-SCOPE structured-defer per refusal #10 disposition

**The historical "v2-raw-heap aliasing → typed_array_push_* realloc invalidates
aliased raw pointers" framing in CLAUDE.md "Known Constraints" is stale-
relative-to-HEAD `3cb72c2d`**. At post-V3-S5 / post-cluster-1.5-q25c HEAD:

- `TypedArray::push` realloc is sound by construction per §1.D audit;
- `TypedArrayData` enum + `TypedBuffer<T>` wrapper layer were deleted at V3-S5
  ckpt-1..ckpt-4 per W12 audit §3.5+§3.6 (Refusal #1 binding: TypedArrayData
  resurrection refused on sight);
- `TraitObjectStorage` producer/consumer lockstep flip completed at
  cluster-1.5-q25c-trait-object-rebuild close (parent HEAD `3cb72c2d`);
- The 4 simulation tests' historical SIGSEGV / double-free symptoms originated
  in pre-V3-S5 carrier-shape layouts that no longer exist at HEAD.

**Recommended CLAUDE.md "Known Constraints" v2-raw-heap-audit entry update**
(NOT landed in scope — flagged per dispatch's `CLAUDE.md modifications
surfaced (flag only)` close gate item):

```markdown
- **v2-raw-heap-audit — RE-CLASSIFIED 2026-05-16** per
  `docs/cluster-audits/cluster-1.5-v2-raw-heap-audit.md`: the 4 simulation
  tests at `bin/shape-cli/tests/stdlib/simulation.rs` (`test_harmonic_oscillator_rk4_system`,
  `test_rk45_system_harmonic_oscillator`, `test_find_collisions_brute`,
  `test_find_collisions_sweep`) are at HEAD blocked by V3-S5 ckpt-5/ckpt-6
  SURFACE classes (`op_new_array` + `op_new_object` + `arr[i]` for
  `Array<TypedObject>`), NOT the historical v2-raw-heap aliasing repro.
  The cluster-2 §D Class 1 SIGABRT anchor at `hashmap_filter_all_match`
  remains live (empirical reproducer at
  `crates/shape-value/src/v2/string_obj.rs:118` misaligned ptr → SIGABRT)
  but its root cause requires gdb-backtrace empirical isolation; static-
  analysis tier could not isolate the load-bearing producer/consumer
  carrier-shape mismatch. Tracked as `cluster-1.5-v2-raw-heap-empirical-
  isolation` follow-up sub-cluster.
```

**Disposition**: FLAG for supervisor disposition; do NOT land without user
ratification per dispatch's CLAUDE.md modifications discipline.

---

## §4 Refusal discipline self-audit (cluster-2 canonical refusal set carried forward)

Per CLAUDE.md "Forbidden Patterns + Renames to refuse on sight; cluster-2
canonical refusal set; carries forward to cluster-1.5":

- **NO ValueWord resurrection** — audit does not propose any `ValueWord`,
  `ValueBits`, `tag_bits::*`, or post-strict-typing-deleted-shape introduction.
  The Phase 2 fix recommendation for the empirically-isolated cluster-2 Class 1
  root cause uses `v2_retain` / `release_elem` v2-raw discipline per the
  HEAD `3cb72c2d` carrier-shape contract, NOT the pre-strict-typing
  `vw_clone` / `vw_drop` (deleted along with `value_word_drop` module). The
  vw_clone/vw_drop SHAPE (retain inner / release outer / pair the share
  transfer) is preserved; the SYMBOLS are post-strict-typing equivalents.
- **NO Bool-default fallback** — `entry_object_at` / `read_entry_kinded` /
  `set_kinded` per-V dispatch arms all use explicit `NativeKind::X` match arms
  with structured `type_error` on unsupported kinds; no Bool-default escape
  hatch.
- **NO broader-family bridge/probe/helper/hop/translator/adapter/shim
  framings** — audit deliverable + commit message + AGENTS.md row will use
  carrier-shape names (`TypedArray::push`, `HashMapData::insert`,
  `TypedObjectPtr::Drop`, `release_typed_array`, `v2_retain`, `release_elem`)
  per CLAUDE.md "Describe deleted code by name or by deletion-fate" rule.
  Zero hits on `(decode|tag|kind|dispatch|value.call|closure.callback|frame.
  setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)`
  regex.
- **NO parallel-implementation framings** — audit does not propose new
  carrier shapes parallel to existing ones; the bounded-fix recommendation
  is empirical isolation + matching-precedent fix at the producer/consumer
  boundary per existing carrier shape.
- **NO new HeapKind variants** — audit identifies no need for new HeapKind
  variants. The existing `HeapKind::String` / `HeapKind::TypedArray` /
  `HeapKind::HashMap` / `HeapKind::TypedObject` / `HeapKind::TraitObject` /
  `HeapKind::Decimal` set is sufficient.
- **Refuse #10 anti-deferral** — every deferred work item cites a specific
  destination: cluster-1.5-v2-raw-heap-empirical-isolation (§3.B),
  V3-S5 ckpt-6 cluster-0 territory (§3.A, §1.A), supervisor disposition
  required for CLAUDE.md update (§3.C). No "tracked as follow-up to ignore"
  framing.
- **Per CLAUDE.md "Own all code quality"** — no clippy regressions
  introduced (zero source modifications in agent scope; audit-only
  deliverable).
- **vw_clone/vw_drop precedent (commit `afb1651`) shape mirroring** —
  the precedent SHAPE (retain inner before push / release outer wrapper /
  balanced pair) is preserved in §3.B fix-shape recommendation, mapped to
  post-strict-typing v2-raw equivalents (`v2_retain(&(*ptr).header)` /
  `release_elem(ptr)` per HeapElement trait dispatch). The precedent
  SYMBOLS (`vw_clone` / `vw_drop` / `ValueWord`) are NOT resurrected.
- **IF audit surfaces a NEW forbidden pattern or refuse-on-sight phrase**
  that should land in CLAUDE.md: NONE surfaced. The 4 ignored tests'
  `#[ignore]` reason strings cite "v2 raw-ptr aliasing class
  (path-c2/v2-c-alias)" — that label was historical/honest at the time of
  the ignore, not a new forbidden framing. The re-classification §1.A
  honors the carrier-shape changes since then without retroactively
  re-labeling the ignore as a forbidden-pattern instance.

---

## §5 Environment-limitation diagnostic

Agent scope build verification blocked by sandbox Nix-loader environment.
Specifics for follow-up agent (NixOS-style):

- `cc` not in `$PATH`; nix-store paths (`/nix/store/8v97ngkcpfzgghwnnr7fsz33p2x22gy9-gcc-wrapper-14.3.0/bin`) exist but produce wrong-ELF-interpreter binaries.
- `/lib64/ld-linux-x86-64.so.2` (system loader) successfully runs nix-built executables when invoked explicitly.
- The cluster-1.5-q25c-trait-object-rebuild close note (per AGENTS.md row 357) cites the canonical devenv-wrapper invocation: `env CC=/nix/store/8v97ngkcpfzgghwnnr7fsz33p2x22gy9-gcc-wrapper-14.3.0/bin/gcc PATH=/nix/store/8v97ngkcpfzgghwnnr7fsz33p2x22gy9-gcc-wrapper-14.3.0/bin:/nix/store/6gip8zn8scpzl64gxmm9bvj0cm8rpsjp-python3-3.13.12-env/bin:$PATH cargo check --workspace --lib --bins --tests --examples`. Adapted PATH may be required for follow-up agent depending on nix-store-paths at next-session HEAD.
- Per cluster-1.5-q25c-trait-object-rebuild close gate, the canonical close-gate suite is: `cargo check --workspace --lib --bins --tests --examples` EXIT=0 (default); `cargo check -p shape-jit --features jit-trace` EXIT=0; `bash scripts/verify-merge.sh` 12/12 PASS EXIT=0; `bash scripts/check-no-dynamic.sh` EXIT=0; smoke matrix 5/5 VM == JIT all EC=0.

Without working build, the Phase 2 fixes cannot satisfy the close-gate
discipline. Phase 1 audit deliverable (this doc) is grep-verified at HEAD
`3cb72c2d` and ready for merge once Phase 2 close-gate prerequisites are
met by a follow-up agent or environment fix.

---

## §6 Close gate disposition

Per dispatch's LOAD-BEARING ACCEPTANCE + CLOSE GATE criteria:

| Gate | Status | Disposition |
|---|---|---|
| Audit deliverable lands per-site file:line + root-cause + fix shape | MET — this doc | ✓ |
| Per-site fixes landed reduce shape-test SIGABRT count materially (cite pre-fix vs post-fix counts) | NOT MET — Phase 2 fixes not landed (cargo env blocker) | Defer to follow-up |
| Smoke matrix 5/5 VM == JIT preserved | NOT EMPIRICALLY REVERIFIED at this HEAD; parent close at `3cb72c2d` documents 5/5 VM == JIT preserved per cluster-1.5-q25c-trait-object-rebuild AGENTS.md row | Inherit parent disposition |
| 4 ignored simulation tests un-ignore + pass | NOT APPLICABLE per §1.A re-classification (tests blocked at V3-S5 ckpt-5/ckpt-6, sibling cluster-0 territory; un-ignore would surface ckpt-5 SURFACE message, not test pass) | Documented per-test in §1.A (remain ignored; reason-string update is the bounded action) |
| cargo check workspace EXIT=0 (default + `--features shape-jit/jit-trace`) | NOT REVERIFIED at this HEAD (env blocker) | Defer to follow-up |
| verify-merge.sh 12/12 PASS + check-no-dynamic EXIT=0 | NOT REVERIFIED at this HEAD (env blocker) | Defer to follow-up |
| Audit deliverable exists per §A-N shape (mirror inventory deliverable) | MET — this doc | ✓ |
| AGENTS.md row appended | PENDING (after audit doc landing) | Will append before commit |
| NO Co-Authored-By: Claude trailer | OBSERVED | Will preserve at commit |

**Net disposition**: cluster-1.5-v2-raw-heap-audit Phase 1 (audit) close-gate
criteria MET (audit deliverable + cite-grep-verification + refusal-discipline
self-audit + structured-defer recommendations). Phase 2 (bounded fix)
close-gate criteria NOT MET in agent scope due to environment limitation;
recommendations structured for handoff to follow-up agent with working
cargo build env.

Cite for follow-up: `cluster-1.5-v2-raw-heap-empirical-isolation` sub-cluster
(§3.B) + 4 simulation test reason-string update (§3.A) + CLAUDE.md "Known
Constraints" v2-raw-heap-audit entry refresh (§3.C, supervisor disposition
required).

---

## §7 Ceiling-c + D-α status

**Ceiling-c bound check**: this audit is read-only (zero `.rs` modifications);
the deliverable doc is a single `.md` file. Well within ceiling-c (~100-site
ceiling). Phase 2 fix recommendations are bounded: §3.A reason-string update
~48 LoC across 1 file; §3.B empirical-isolation fix ~30-150 LoC across 1-3
files per vw_clone/vw_drop precedent. Both fit within ceiling-c independently;
combined work fits in a single follow-up agent dispatch.

**D-α status**: surface-and-stop intermediate states not required. The audit
proceeded as single-checkpoint static analysis; the empirical anchor for
cluster-2 Class 1 SIGABRT requires gdb-backtrace at runtime (cargo env
prerequisite) for next-checkpoint isolation. No multi-session chain required
for the audit deliverable itself.
