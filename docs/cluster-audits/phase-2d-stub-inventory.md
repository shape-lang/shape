# Phase 2d — Stub Inventory

**Audit date:** 2026-05-11
**Branch:** `bulldozer-strictly-typed` at `45bd827` (one commit past W14/W15/W16 close `89d5f5f`).
**Trigger:** Phase-2d handover §1 Item 1 — fresh inventory before allocating sub-cluster work.
**Methodology:** ripgrep multi-line scan of `Err(VMError::NotImplemented(...))` blocks + per-file `todo!("phase-2c…")` rollup + ADR-006 §-cite verification (range §2.7.1..§2.7.24 + §2.3, §2.4).
**Architectural decisions:** ADR-006 §2.7.24 (Q25.A/B/C — typed-carrier monomorphization bundle) is the binding amendment for this inventory's sub-cluster shapes. Updated 2026-05-11 with locked decisions; see §6 below.

---

## §0 — Audit corrections to the handover framing

The handover's "439 hits across ~50 files" figure mixed three different emission styles into one grep. The cleaner taxonomy:

| Class | Count | Where | Remediation shape |
|---|---|---|---|
| **§1.A** Production `Err(NotImplemented(... SURFACE ...))` | **104** | 35 files in `shape-vm/src/executor/` | Sub-cluster body-fill (handover Item 2 territory) |
| **§1.B** Production `todo!("phase-2c…")` panic stubs | **~55** | Snapshot, state, transport, FFI, JIT FFI | Sub-cluster body-fill (same class as A; cruder syntactic shape) |
| **§3** Test `todo!("phase-2c…")` placeholders | **~300** | 16 test files in `executor/tests/` | Validation downstream — flip when production sub-cluster lands |
| **§4** Compile-tier `todo!("phase-2c…")` | **~35** | `compiler/expressions/*`, `compiler/comptime*`, `compiler/loops/helpers` | Compile-tier sub-clusters (C1/C2/C3) |
| Non-SURFACE `Err(NotImplemented(...))` | 24 | various | Intentional runtime constraint errors; **not** arch gaps |

**Storage tier (`crates/shape-value/`) is clean.** The 3 grep hits the handover called out in `closure_raw.rs` are all docstrings pointing at *consumer-side* SURFACE handlers in `executor/variables/mod.rs:3113,3676`. `ClosureCell` (closure_raw.rs:~1500) is the §2.7.8 / Q10 cell-storage foundation Wave-β B6 needs; consumer migration belongs to the W17-references-mutation sub-cluster (§2 below). **No separate W17-closure-raw-storage sub-cluster needed.**

**§-cite verification:** 75/104 SURFACE sites cite an ADR-006 § (mostly §2.7.4); 29 are `[CITE-MISSING]` and tagged inline below. Every cited § is present in ADR-006 — zero invalid cites. Per user ruling, sub-cluster owners fix `[CITE-MISSING]` rows in passing during their first commit.

---

## §1.A — Production SURFACE emission sites

Aggregated by file. Per-line file:line references in this section; see `/tmp/phase2d-audit/surface-structured.tsv` for the full per-line TSV with extracted blocker text.

| File | Count | Cited §s | Recommended sub-cluster |
|---|---|---|---|
| `executor/objects/concurrency_methods.rs` | 10 | §2.3 (×10) | **W17-concurrency** |
| `executor/objects/array_transform.rs` | 9 | §2.7.4 (×7) + [CITE-MISSING] (×2) | **W17-array-heap-element-kind** (8) + **W17-array-coerce** (1) |
| `executor/loops/mod.rs` | 7 | §2.7.4 (×4) + [CITE-MISSING] (×3, ADR-005 §1 violations) | **W17-iterator-tableview** |
| `executor/objects/array_operations.rs` | 7 | [CITE-MISSING] (×7, all §2.7.7/§2.7.8 implicit) | **W17-array-typed-receiver** |
| `executor/objects/array_query.rs` | 6 | (TYPED_ARRAY_BUILDER_SURFACE constant) | **W17-array-closure-callback** |
| `executor/objects/property_access.rs` | 5 | §2.7.4 (×2) + §2.7.7/§2.7.8 (×3) | **W17-property-access** |
| `executor/objects/array_aggregation.rs` | 5 | §2.7.4 (×5) | **W17-array-heap-element-kind** |
| `executor/variables/mod.rs` | 4 | §2.7.13 (×2) + [CITE-MISSING] (×2) | **W17-references-mutation** (Wave-β B6 cont.) |
| `executor/typed_object_ops.rs` | 4 | §2.7.7 (×3) + [CITE-MISSING] (×1, ADR-005 §1) | **W17-typed-object-mutation** |
| `executor/trait_object_ops.rs` | 4 | [CITE-MISSING] (×4) | **W17-trait-object-rebuild** |
| `executor/vm_impl/builtins.rs` | 3 | §2.7.4 (×2) + §2.7.6/Q8 (×1) | **W17-concurrency** + **W17-deque-ctor** |
| `executor/objects/typed_access.rs` | 3 | §2.7.4 (×3) | **W17-hashmap-typed-buffer** |
| `executor/objects/array_joins.rs` | 3 | §2.7.10 / §2.7.11 (×3) | **W17-array-closure-callback** |
| `executor/window_join.rs` | 2 | §2.7.6 (×1) + [CITE-MISSING] (×1) | **W17-window-join** |
| `executor/resume.rs` | 2 | (PHASE_2C_SNAPSHOT_SURFACE constant) | **W17-snapshot-resume** |
| `executor/objects/object_creation.rs` | 2 | [CITE-MISSING] (×2) | **W17-typed-object-mutation** |
| `executor/objects/mod.rs` | 2 | §2.7.6 (×1) + [CITE-MISSING] (×1) | **W17-trait-object-rebuild** |
| `executor/objects/datatable_methods/query.rs` | 2 | §2.7.4 (×2) | **W17-datatable-results** |
| `executor/objects/datatable_methods/joins.rs` | 2 | §2.7.4 (×2) | **W17-datatable-results** |
| `executor/objects/datatable_methods/aggregation.rs` | 2 | [CITE-MISSING] (×2, depends on D-prop-access) | **W17-datatable-results** |
| `executor/objects/channel_methods.rs` | 2 | §2.7.20 (×1) + §2.7.4 (×1) | **W17-channel-blocking** |
| `executor/objects/array_sort.rs` | 2 | §2.7.10 + §2.7.4/§2.7.8 | **W17-array-closure-callback** |
| `executor/control_flow/mod.rs` | 1 | §2.7.4 / §2.7.5 | **W17-foreign-ffi** |
| `executor/control_flow/foreign_marshal.rs` | 2 | (PHASE_2C_FFI_REBUILD_SURFACE constant) | **W17-foreign-ffi** |
| `executor/builtins/type_ops.rs` | 2 | §2.7.4 (×2) | **W17-builtin-coercions** |
| `executor/objects/string_methods.rs` | 1 | §2.7.4 | **W17-array-heap-element-kind** |
| `executor/objects/iterator_methods.rs` | 1 | §2.7.4 | **W17-iterator-tableview** |
| `executor/objects/datetime_methods.rs` | 1 | §2.7.4 (cascade) | **W17-hashmap-typed-buffer** (cross-cluster) |
| `executor/objects/datatable_methods/simulation.rs` | 1 | §2.7.4 | **W17-datatable-results** |
| `executor/objects/concat.rs` | 1 | [CITE-MISSING] | **W17-array-heap-element-kind** |
| `executor/objects/bool_methods.rs` | 1 | §2.7.6 | **W17-method-bodies-misc** |
| `executor/objects/array_sets.rs` | 1 | §2.7.10 / §2.7.11 | **W17-array-closure-callback** |
| `executor/objects/array_basic.rs` | 1 | §2.7.6 / Q8 | **W17-array-heap-element-kind** |
| `executor/vm_impl/schemas.rs` | 1 | [CITE-MISSING] | **W17-method-bodies-misc** |
| `executor/vm_impl/modules.rs` | 1 | [CITE-MISSING] | **W17-method-bodies-misc** |

**Total: 104 SURFACE emissions across 35 files.**

---

## §1.B — Production `todo!("phase-2c…")` panic stubs

These are panic-style architectural gaps (cruder than SURFACE — they panic the VM on first hit instead of returning structured `VMError`). Same remediation shape as §1.A.

| File | Count | Cited § | Recommended sub-cluster |
|---|---|---|---|
| `executor/state_builtins/introspection.rs` | 9 | §2.7.4 | **W17-snapshot-resume** |
| `executor/state_builtins/core.rs` | 8 | §2.7.4 | **W17-snapshot-resume** |
| `executor/vm_state_snapshot.rs` | 8 | §2.7.4 | **W17-snapshot-resume** |
| `executor/snapshot.rs` | 2 | §2.7.4 | **W17-snapshot-resume** |
| `runtime/data/cache.rs` | 2 | (snapshot.rs:648 deferral) | **W17-snapshot-resume** |
| `runtime/context/mod.rs` | 2 | (snapshot.rs:648 deferral) | **W17-snapshot-resume** |
| `shape-vm/src/remote.rs` | 2 | §2.7.4 + addendum | **W17-typed-module-exports** |
| `executor/builtins/transport_builtins.rs` | 1 (+ doc) | §2.7.4 + addendum | **W17-typed-module-exports** |
| `executor/builtins/remote_builtins.rs` | 1 (+ doc) | §2.7.4 + addendum | **W17-typed-module-exports** |
| `executor/v2_stack_tests.rs` | 3 | §2.7.4 (NativeScalar) | **W17-native-scalar-carrier** |
| `executor/loops/mod.rs` | 2 | (cross-ref to tests) | absorb in **W17-iterator-tableview** |
| `executor/mod.rs` | 1 | (`apply_pending_resume` stub) | **W17-snapshot-resume** |
| `executor/builtins/type_ops.rs` | 1 (doc/comment) | (gap surface) | absorb in **W17-builtin-coercions** |
| `executor/printing.rs` | 1 (doc/comment) | §2.7.4 | absorb in **W17-builtin-coercions** |
| `shape-jit/src/ffi_symbols/vector/mod.rs` | 1 | §2.7.4 | **W17-jit-stubs** |
| `shape-jit/src/ffi_symbols/data_access/mod.rs` | 1 | §2.7.4 | **W17-jit-stubs** |
| `shape-jit/src/ffi/object/conversion.rs` | 1 (doc) | (surface notes) | absorb in **W17-jit-stubs** (or defer to Item 3 JIT-verify) |

**Total: ~45 production todo!() stubs across 17 files.**

---

## §2 — Sub-cluster recommendations

Synthesized from §1.A + §1.B. Format mirrors `wave-14-15-16-playbook.md` §2. Sub-clusters are ordered by suggested execution priority (see §5 for rationale). Each is sized to fit a single agent ~3-8h.

### W17-array-closure-callback **(upstream gate; do this first within the array family)**

**Territory.** Array methods whose body depends on the `op_make_closure` upstream gate (still SURFACE at `executor/control_flow/mod.rs` per `PHASE_2C_CALL_REBUILD_SURFACE`). The kinded `MethodFnV2` (§2.7.10 / Q11) and `op_call_value` (§2.7.11 / Q12) ABIs already landed in W7; this sub-cluster fills the closure-callback array-method bodies that consume them.

**Sites (~14):**
- `array_joins.rs:76,117,148` — innerJoin, leftJoin, crossJoin
- `array_sets.rs:539` — distinctBy
- `array_sort.rs:163,189` — orderBy, thenBy
- `array_query.rs:271,285,482,498` + 2 more — where, select, takeWhile, skipWhile, find, forEach (uses `TYPED_ARRAY_BUILDER_SURFACE`)

**Smoke:** `[1,2,3].orderBy(|x| -x)` returns `[3,2,1]`. `[{a:1},{a:2},{a:1}].distinctBy(|x| x.a)` returns 2-elem.

**Cited §:** §2.7.4, §2.7.10, §2.7.11. `op_make_closure` itself is **NOT in this sub-cluster's scope** — that's W17-make-closure (next).

**Time:** 4-6h.

**Risk:** medium. The `TYPED_ARRAY_BUILDER_SURFACE` constant indicates a missing output-buffer factory — may force a small ADR-006 §2.7.6 amendment if heterogeneous result kinds surface.

---

### W17-make-closure **(true upstream gate; precedes W17-array-closure-callback)**

**Territory.** `op_make_closure` at `executor/control_flow/mod.rs:~447` returning `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)`. Several other sub-clusters mention this as their gate; this is the actual blocker.

**Sites (~1-2):** `executor/control_flow/mod.rs:~447`, plus knock-on at the capture-cell construction layer (`closure_raw.rs` already has `ClosureCell` per §2.7.8 / Q10; the gate is the *make* path, not the cell layout).

**Smoke:** `let f = |x| x + 1; f(5)` returns `6` via VM. `let xs = [1,2,3]; xs.map(|x| x * 2)` returns `[2,4,6]`.

**Cited §:** §2.7.4 / §2.7.5 / §2.7.8 (ClosureCell capture flow); §2.7.11 / Q12 (value-call entry).

**Time:** 4-6h. **Highest priority** — gates W17-array-closure-callback, W17-datatable-results, W17-array-heap-element-kind partially.

**Risk:** medium-high. Cross-cluster gate; if it requires capture-cell ABI work it could spill into ClosureCell layout adjustments. Surface-and-stop if the gap turns out architectural (§2.7.8 amendment needed).

---

### W17-array-typed-receiver

**Territory.** `array_operations.rs` push/pop/slice on non-`Ptr(HeapKind::TypedArray)` receivers (`HeapValue::Array` family deleted by bulldozer; today the legacy receiver path is SURFACE).

**Sites (7):** `array_operations.rs:108,148,189,260,270,381,419,484` (ArrayPush, ArrayPushLocal, ArrayPop, SliceAccess × 2, plus per-variant fallbacks).

**Smoke:** `let xs = [1,2,3]; xs.push(4)` works; `let xs:Array<String> = ["a"]; xs.push("b")` works; `xs[0..2]` returns slice.

**Cited §:** §2.7.7 / §2.7.8 (all [CITE-MISSING], fix in passing).

**Time:** 3-5h. Mechanical bodies; the receiver-kind classification is one shape repeated 7 times.

**Risk:** low. Pure kind-narrowing work; no new HeapKinds.

---

### W17-typed-carrier-monomorphization **(merged sub-cluster; consumes ADR-006 §2.7.24)**

**Territory.** Single coordinated sub-cluster covering the storage-tier rebuild ADR-006 §2.7.24 (Q25.A/B/C) ruled in:

- **Q25.A** — delete `TypedArrayData::HeapValue` arm; add specialized variants (`Decimal`, `BigInt`, `DateTime`, `Timespan`, `Duration`, `Instant`, `Char`); add `TypedObject` and `TraitObject` catch-all variants for user types.
- **Q25.B** — refactor `HashMapData` to parametric `HashMapValueBuf` enum (same shape per-value-type as Q25.A).
- **Q25.C** — re-introduce `HeapKind::TraitObject = 29` with `Arc<TraitObjectStorage>` carrier (fat-pointer shape: `value: Arc<TypedObjectStorage>` + `vtable: Arc<VTable>`); universal `dyn` via auto-boxing rule + richer `VTableEntry` enum (6 variants).

**Why merged.** All three Q25 sub-rulings share carrier-shape DNA (typed Arc, kind known at variant level, no polymorphic catch-all arms), share dispatch-table updates (4-table lockstep applies to TraitObject; the new TypedArrayData/HashMapValueBuf variants need exhaustive-match expansion in the same files), and share migration discipline. Splitting them would force three round-trips through the same dispatch-table files.

**Pre-assigned HeapKind ordinal:** `TraitObject = 29` (next free after Option=28). Bump-on-collision per §0 ordinal rule.

**Sites (~33):**

- *Array specialization (Q25.A):* `array_transform.rs:486,547,589,693,722,775,1140,1310` (8); `array_aggregation.rs:194,205,316,362,392` (5); `array_basic.rs:447` (1); `iterator_methods.rs:224` (1); `string_methods.rs:573,579` (2); `objects/concat.rs:1` (1) — total 18.
- *HashMap parametric (Q25.B):* `objects/typed_access.rs:172,192,212` (3); `datetime_methods.rs:426` cross-cluster cascade (1) — total 4.
- *Trait object rebuild (Q25.C):* `trait_object_ops.rs:77,99,124,142` (4); `objects/mod.rs:251` (1); `objects/concat.rs` overlap absorbed above — total 5.
- *Test placeholders unblocked downstream:* tests/matrix_ops.rs (38), tests/decimal_ops.rs (7), tests/typed_array_ops.rs (31).

**Smoke targets:**

```shape
// Q25.A
[1n, 2n, 3n].sum()                                    // BigInt array sum works
[date(2026,1,1), date(2026,2,1)].forEach(|d| ...)     // DateTime array iter works

// Q25.B
let m = HashMap<String, int>(); m["a"] = 1; m["a"]    // returns 1 via specialized I64 buffer

// Q25.C
trait Animal { fn name(&self) -> String; fn clone_me(&self) -> Self; }
impl Animal for Dog { ... }
let a: dyn Animal = box(Dog { ... })
print(a.name())                                       // [vtable] dispatch
let b = a.clone_me()                                  // [vtable + boxed-return]; b: dyn Animal
```

**Cited §:** ADR-006 §2.7.24 (Q25.A/B/C). §2.3 (typed-Arc carrier shape). §2.7.7 / §2.7.8 (lockstep dispatch tables for HeapKind::TraitObject).

**Time:** **24-32h** elapsed. Split into 3 internal phases for sequencing within one branch: P1 (Q25.A storage tier, 8-10h), P2 (Q25.B HashMap refactor + cascade, 4-6h), P3 (Q25.C TraitObject + universal-dyn + thunks, 12-16h). One agent owns the whole branch; phases are commits within it, not separate worktrees, because the dispatch tables touched in P1 are also touched in P3.

**Risk:** **high**. Carrier shape changes affect ~30 files. Apply 4-table lockstep grep verification after every commit. The Q25.C auto-boxing thunks are the most novel territory — keep the implementation faithful to ADR-006 §2.7.24 Q25.C.5 (final VTableEntry enum); do not add transitional shims preserving the old VTable shape.

**Required reading** (per §7 below): handover §0; CLAUDE.md "Forbidden Patterns" + "Renames to refuse on sight"; ADR-006 §2.7.3 (typed Arc), §2.7.6 (Q8 carrier-API bound), §2.7.7 / §2.7.8 (parallel-kind invariant), §2.7.10 (Q11 MethodFnV2), §2.7.15 (W13-hashset recipe for HeapKind add), §2.7.24 (this amendment, in full); canonical commits `0da1477` (HeapKind sibling recipe), `52c8ef5` (typed-Arc dispatch label), `3ac2f11` (5-arm receiver-recovery soundness).

---

### W17-typed-object-mutation

**Territory.** `TypedObjectStorage` field write paths + property-access for non-`Ptr(TypedObject)` receivers + heterogeneous receiver-kind GetProp/SetProp/SetLocalIndex/SetModuleBindingIndex.

**Sites (~11):**
- `typed_object_ops.rs:164,179,248,429` — push_field_value × 2, op_get_field_typed, op_set_field_typed
- `property_access.rs:193,302,396,418,444` — GetProp on heap kinds, GetProp on TypedArray variant, SetProp, SetLocalIndex, SetModuleBindingIndex
- `objects/object_creation.rs` × 2

**Smoke:** `type P { x: int }; let p = P { x: 1 }; p.x = 2; p.x` returns 2. `m["key"] = value` works for HashMap.

**Cited §:** §2.7.4 / §2.7.7 / §2.7.8.

**Time:** 6-8h. Cross-cluster cascade with W17-references-mutation.

**Risk:** medium. The write-path is `Arc<TypedObjectStorage>` rebuild via `clone_slots_with_update`; needs Wave-β B6 patterns from `closure_raw::ClosureCell`.

---

### W17-references-mutation (Wave-β B6 continuation)

**Territory.** `executor/variables/mod.rs` DerefStore/SetIndexRef and DerefLoad through TypedIndex — the Wave-β B6 cluster cited in `closure_raw.rs:1617` (3 hits there are docstrings pointing at *these* sites).

**Sites (4):**
- `variables/mod.rs:3113` — DerefStore / SetIndexRef SURFACE
- `variables/mod.rs:3676` — DerefLoad through TypedIndex SURFACE
- 2 more cite-missing in same file

**Smoke:** `let mut p = P{x:1}; let r = &mut p.x; *r = 2; p.x` returns 2.

**Cited §:** §2.7.13 / Q14 (kinded `RefTarget` redesign — already landed). The handler-side migration is what remains.

**Time:** 4-6h.

**Risk:** medium. Uses the §2.7.8 `ClosureCell` pattern as recipe; well-trodden territory after Wave-β.

---

### W17-iterator-tableview

**Territory.** `executor/loops/mod.rs` op_iter_done / op_iter_next for TableView/DataTable rows, HashMap key/value pair iteration, TypedArrayData::HeapValue iteration.

**Sites (~9):**
- `loops/mod.rs:229,239,253,379,392,399,410,522` — 7 SURFACE + 2 cross-ref todo!()s
- `iterator_methods.rs` partial overlap

**Smoke:** `for row in table.rows() { print(row) }` works. `for (k, v) in map { print(k, v) }` works. `for x in heterogeneous_array { ... }` works.

**Cited §:** §2.7.4 (the `[CITE-MISSING]` ones are ADR-005 §1 violations — those are bugs to fix, not SURFACEs to fill).

**Time:** 4-6h. Note: 2 sites are diagnostic ADR-005 §1 violations (iter_kind mismatches heap arm) — fix the kind-tracking bug, don't fill the body.

**Risk:** medium.

---

### W17-concurrency

**Territory.** Mutex / Atomic / Lazy as new HeapKinds. Today the constructors panic in `vm_impl/builtins.rs:678` and the methods all return SURFACE. ADR-006 §2.3 cites — typed-Arc redesign.

**Sites (~11):**
- `concurrency_methods.rs:40-157` — 10 emissions (Mutex.lock/try_lock/set, Atomic.{load,store,fetch_add,fetch_sub,compare_exchange}, Lazy.{get,is_initialized})
- `vm_impl/builtins.rs:678` — constructor SURFACE

**Smoke:** `let m = Mutex(0); m.lock(); m.set(5)` works. `let a = Atomic(0); a.fetch_add(1)` works.

**Cited §:** §2.3.

**Time:** 8-12h. **Full 4-table HeapKind rebuild** (3 new HeapKinds: Mutex, Atomic, Lazy — or one Concurrency variant with sub-tag). Plus cross-task semantics work for Mutex (interacts with §2.7.4 task-scheduler boundary).

**Risk:** **high**. Largest single sub-cluster. New HeapKind territory — apply the 4-table lockstep rule (handover §0). Mirror `W15-channel` recipe (commit `0da1477` derivative). Lazy.get() additionally needs closure-call dispatch (gates on W17-make-closure).

---

### W17-channel-blocking

**Territory.** Channel.recv() blocking-on-empty path + Channel.is_sender role check (sender/receiver collapsed to single Arc<ChannelData> in W15; role-check has no semantic answer).

**Sites (2):** `channel_methods.rs:156,245`.

**Smoke:** Channel.recv() blocks until send arrives (cross-task await-style suspend/resume).

**Cited §:** §2.7.20 / §2.7.4.

**Time:** 6-10h. Genuinely needs the §2.7.4 task-scheduler boundary integration; **defer until Item 3 (JIT) and Item 4 (mutation ADR) close** since cross-task semantics interact with both.

**Risk:** **high**. Defer.

---

### W17-trait-object-rebuild — **MERGED into W17-typed-carrier-monomorphization**

Per ADR-006 §2.7.24 Q25.C ruling (2026-05-11): trait-object rebuild lands as P3 of the bundled W17-typed-carrier-monomorphization sub-cluster, not as a standalone agent. The richer VTableEntry enum + universal-dyn auto-boxing rule + Self-arg vtable-identity check shape are specified in ADR-006 §2.7.24 Q25.C.1–C.7.

---

### W17-snapshot-resume

**Territory.** Snapshot/resume rebuild — currently 27 production `todo!("phase-2c snapshot rebuild")` sites that panic the VM on first call. All cite ADR-006 §2.7.4 with "see snapshot.rs:648 deferral" reference.

**Sites (~25):**
- `executor/state_builtins/{introspection,core}.rs` × 17 (introspection: 9, core: 8)
- `executor/vm_state_snapshot.rs` × 8
- `executor/snapshot.rs` × 2
- `executor/resume.rs` × 2 (SURFACE)
- `executor/mod.rs:476` — `apply_pending_resume` stub
- `runtime/data/cache.rs` × 2 + `runtime/context/mod.rs` × 2

**Smoke:** `snapshot()` captures state; `resume(s)` restores. State introspection (`type_info`, `implements`, `warning`, `error`, `build_config`) works.

**Cited §:** §2.7.4 (+ snapshot.rs:648 deferral marker).

**Time:** **8-16h** (largest single sub-cluster by site count). Requires kinded state-snapshot serialization (every NativeKind must round-trip through wire format).

**Risk:** medium-high. Cross-cuts state-introspection, comptime, and snapshot — but all use the same §2.7.4 KindedSlot snapshot/restore protocol. One coherent design.

---

### W17-typed-module-exports

**Territory.** `transport_builtins` / `remote_builtins` / `shape-vm/remote.rs` typed-module-exports rebuild. Cites ADR-006 §2.7.4 + addendum.

**Sites (~5):** transport_builtins.rs:81, remote_builtins.rs:84, remote.rs:640,655, + 2 docstrings.

**Smoke:** Module exports across QUIC wire protocol round-trip with typed kinds preserved.

**Cited §:** §2.7.4 + addendum.

**Time:** 6-8h.

**Risk:** medium. Depends on wire-format §2.7.5.1 post-proof shapes; the addendum reference suggests a §2.7.4 extension already drafted somewhere — check before designing.

---

### W17-foreign-ffi

**Territory.** `extern C` FFI rebuild — `foreign_marshal` (marshal_args / unmarshal_result) + `op_call_foreign`.

**Sites (3):**
- `control_flow/foreign_marshal.rs:57,74` — marshal_args, unmarshal_result (both use `PHASE_2C_FFI_REBUILD_SURFACE`)
- `control_flow/mod.rs:680` — op_call_foreign

**Smoke:** `extern C fn libm_sin(x: number) -> number; libm_sin(0.0)` returns 0.0.

**Cited §:** §2.7.4 / §2.7.5.

**Time:** 6-8h.

**Risk:** medium. The msgpack-based pre-rebuild body is referenced in commit `afb1651` (cited in CLAUDE.md `v2-raw-heap` notes). Use as starting point but switch to kinded carrier per §2.7.5 cross-crate ABI policy.

---

### W17-builtin-coercions

**Territory.** `as string` cast for heap kinds + `Convert` opcode (TryInto/Into trait dispatch + AnyError TypedObject builder).

**Sites (2):** `executor/builtins/type_ops.rs:451,503`.

**Smoke:** `(123).toString() == "123"`. `let r: Result<int, MyErr> = (10 as int).tryInto()`.

**Cited §:** §2.7.4. Depends on `executor/printing.rs::format_heap_kind` Phase-2c rebuild.

**Time:** 3-5h.

**Risk:** low. Mechanical once `format_heap_kind` is migrated.

---

### W17-hashmap-typed-buffer — **MERGED into W17-typed-carrier-monomorphization**

Per ADR-006 §2.7.24 Q25.B ruling (2026-05-11): HashMap parametric value buffer lands as P2 of the bundled W17-typed-carrier-monomorphization sub-cluster.

---

### W17-datatable-results

**Territory.** DataTable methods that need property-access cluster territory to land first — group_by, map, innerJoin, leftJoin, simulate, aggregate.

**Sites (7):**
- `datatable_methods/joins.rs:43,61` — innerJoin, leftJoin
- `datatable_methods/query.rs:396,460` — group_by, map
- `datatable_methods/aggregation.rs:573,591` — aggregate, parse_agg_spec
- `datatable_methods/simulation.rs:51` — simulate

**Smoke:** `dt.groupBy("col").aggregate({sum:"x"})` works.

**Cited §:** §2.7.4 throughout.

**Time:** 6-10h. **Blocked on W17-property-access (W17-typed-object-mutation) + W17-array-closure-callback + W17-hashmap-typed-buffer**. Schedule LAST in the W17 lineup.

**Risk:** medium. Coordination risk — three upstream gates.

---

### W17-deque-ctor — **DROPPED as standalone sub-cluster**

Per 2026-05-11 audit: the single SURFACE in `vm_impl/builtins.rs:636` is **stale** — `HeapKind::Deque = 23` and `KindedSlot::from_deque` already exist (landed in W15 W15-deque, 2026-05-10). The fix is 5 lines mirroring the adjacent ChannelCtor body. Rolled into **W17-method-bodies-misc** as a sub-task; no separate sub-cluster needed.

---

### W17-window-join

**Territory.** `executor/window_join.rs` W8-WJ datetime expr eval + join_execute — depends on Temporal heap arm dispatch (§2.7.6 / Q8) and datatable_methods::joins ABI flip.

**Sites (2):** `window_join.rs:385,480`.

**Smoke:** `dt.windowJoin(other, "ts", duration("1h"))` works for time-windowed joins.

**Cited §:** §2.7.6 (Temporal carrier).

**Time:** 4-6h. **Blocked on W17-datatable-results joins** + Temporal heap dispatch.

**Risk:** medium. Coordination risk.

---

### W17-native-scalar-carrier

**Territory.** `executor/v2_stack_tests.rs` three production todo!()s citing "NativeScalar carrier pending kinded redesign" (§2.7.4). These are NOT test placeholders — they're production helpers in a test-named file that other code depends on.

**Sites (3):** `v2_stack_tests.rs:163,169,212`.

**Cited §:** §2.7.4.

**Time:** 2-4h.

**Risk:** low. Verify these are actually production helpers (not test fixtures) before scheduling.

---

### W17-jit-stubs

**Territory.** JIT FFI stubs that panic on hot paths — vector intrinsics + align_tables.

**Sites (3):** `shape-jit/src/ffi_symbols/vector/mod.rs:30`, `data_access/mod.rs:65`, `ffi/object/conversion.rs` (docs).

**Cited §:** §2.7.4 (vector intrinsics + align_tables kind-threaded rebuild).

**Time:** 4-6h. **Defer to Item 3 (JIT verification)** per handover §2 ordering — these are JIT-tier and the JIT FFI surface depends on which kinds flow through hot paths, which depends on the W17 method-handler outcomes.

**Risk:** Defer.

---

### W17-method-bodies-misc (catch-all)

**Territory.** Small singleton SURFACEs not absorbed elsewhere.

**Sites (~4):**
- `objects/bool_methods.rs:29` — bool.toString
- `vm_impl/schemas.rs`, `vm_impl/modules.rs` — 1 each, [CITE-MISSING]
- minor stragglers

**Time:** 2-3h. Trailing cleanup.

**Risk:** low.

---

### Sub-cluster summary (post ADR-006 §2.7.24 lock-in, 2026-05-11)

| # | Sub-cluster | Sites | HeapKinds | Estimated h | Risk | Blocking? |
|---|---|---|---|---|---|---|
| 1 | **W17-make-closure** | 2 | — | 4-6 | medium | **Gates 4 others** |
| 2 | **W17-snapshot-resume** | 25 | — | 8-16 | medium-high | independent |
| 3 | **W17-array-typed-receiver** | 7 | — | 3-5 | low | independent |
| 4 | **W17-references-mutation** | 4 | — | 4-6 | medium | gates #6 |
| 5 | **W17-iterator-tableview** | 9 | — | 4-6 | medium | independent |
| 6 | **W17-typed-object-mutation** | 11 | — | 6-8 | medium | blocked by #4 |
| 7 | **W17-typed-carrier-monomorphization** *(merged: Q25.A + Q25.B + Q25.C)* | 33 | **TraitObject=29** | 24-32 | **high** | gates #15 (datatable) |
| 8 | **W17-concurrency** | 11 | **Mutex=30, Atomic=31, Lazy=32** | 8-12 | high | partial gate #1 for Lazy.get |
| 9 | **W17-typed-module-exports** | 5 | — | 6-8 | medium | independent |
| 10 | **W17-foreign-ffi** | 3 | — | 6-8 | medium | independent |
| 11 | **W17-builtin-coercions** | 2 | — | 3-5 | low | independent |
| 12 | **W17-array-closure-callback** | 14 | — | 4-6 | medium | blocked by #1 |
| 13 | **W17-datatable-results** | 7 | — | 6-10 | medium | blocked by #6, #7, #12 |
| 14 | **W17-window-join** | 2 | — | 4-6 | medium | blocked by #13 |
| 15 | **W17-channel-blocking** | 2 | — | 6-10 | high | defer (cross-task §2.7.4) |
| 16 | **W17-jit-stubs** | 3 | — | 4-6 | defer | defer to Item 3 |
| 17 | **W17-native-scalar-carrier** | 3 | — | 2-4 | low | verify scope |
| 18 | **W17-method-bodies-misc** *(absorbs deque-ctor cleanup)* | 5 | — | 2-3 | low | independent |
| T1 | **T1-host-tier-marshal-rebuild** | (~300 test todos unblocked) | — | 6-8 | low | **Wave-1 leverage** |
| C1 | **C1-temporal-lowering** | 14 | — | 4-6 | low | blocked by T1 |
| C2 | **C2-comptime-rebuild** | 7 | — | 6-10 | medium | independent |
| C3 | **C3-expr-lowering-misc** | ~10 | — | 4-6 | low | independent |

**Total: ~150 production sites across 18 W17 + 1 T1 + 3 compile-tier sub-clusters (22 in all).** Cumulative effort 130-180 agent-hours (~16-22 agent-days serial; ~4-6 days with 4-6 parallel agents). **New HeapKind ordinals pre-assigned:** TraitObject=29, Mutex=30, Atomic=31, Lazy=32 (free ords 33+).

---

## §3 — Test-placeholder per-file rollup

~300 `todo!("phase-2c")` calls across 16 test files. CRITICAL FINDING: nearly all cite **§2.7.4 "host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier"** — they're all blocked on the same root cause.

| File | Count | Dominant sub-cluster validated | Smoke-readiness |
|---|---|---|---|
| `tests/mod.rs` | 52 | T1-host-tier-marshal-rebuild (general) | gated on T1 |
| `tests/matrix_ops.rs` | 38 | W17-array-heap-element-kind (Matrix variant) | gated on T1 + #13 |
| `tests/typed_array_ops.rs` | 31 | W17-array-typed-receiver | gated on T1 + #3 |
| `tests/v2_opcode_tests.rs` | 29 | W17-make-closure + general | gated on T1 + #1 |
| `tests/iterator_ops.rs` | 28 | W17-iterator-tableview | gated on T1 + #5 |
| `tests/set_ops.rs` | 19 | (HashSet — W13 closed; tests still gated on T1) | gated on T1 only |
| `v2_handlers/integration_tests.rs` | 18 | various | gated on T1 |
| `tests/channel_ops.rs` | 14 | W17-channel-blocking | gated on T1 + #17 |
| `tests/try_operator.rs` | 12 | (W14 Result/Option closed; gated on T1) | gated on T1 only |
| `tests/deque_ops.rs` | 12 | (W15 Deque closed; gated on T1 + #19) | gated on T1 + #19 |
| `tests/priority_queue_ops.rs` | 11 | (W15 PQ closed; gated on T1) | gated on T1 only |
| `tests/io_integration.rs` | 11 | W17-typed-module-exports | gated on T1 + #9 |
| `tests/v2_struct_integration.rs` | 10 | W17-typed-object-mutation | gated on T1 + #6 |
| `tests/table_iteration.rs` | 10 | W17-iterator-tableview + W17-datatable-results | gated on T1 + #5 + #15 |
| `tests/type_system_integration.rs` | 8 | W17-builtin-coercions | gated on T1 + #11 |
| `tests/decimal_ops.rs` | 7 | W17-array-heap-element-kind | gated on T1 + #13 |

### **T1-host-tier-marshal-rebuild** — the validation-unblocking sub-cluster

**Territory.** Rebuild `eval()` / `eval_int()` / `eval_float()` / `eval_string()` / `eval_bool()` test helpers (and `Constant::Value(ValueWord)` carrier they depended on) against the post-strict-typing KindedSlot API.

**Effort:** ~6-8h, single agent.

**Risk:** low (test infrastructure, but high *leverage* — landing T1 unblocks 200-300 test bodies for parallel re-fill).

**Recommendation:** schedule T1 **first or in parallel with W17-make-closure** since the two together unblock 80% of downstream test validation.

---

## §4 — Compile-tier `phase-2c` todos

~35 sites in `crates/shape-vm/src/compiler/`. Three coherent sub-clusters.

### C1-temporal-lowering

**Territory:** `compiler/expressions/temporal.rs` — 14 todos all citing §2.7.4 "KindedSlot heap accessors `as_datetime`/`as_timespan` pending kinded host-tier marshal layer". The bytecode emit site for `dt + duration`, `dt - dt`, etc. needs the kinded heap accessors that are still gated on the marshal layer rebuild.

**Sites:** `compiler/expressions/temporal.rs:108,113,118,123,128,...` (14 sites).

**Cited §:** §2.7.4.

**Time:** 4-6h. **Blocked on T1-host-tier-marshal-rebuild** (same marshal layer).

**Risk:** low once T1 lands.

---

### C2-comptime-rebuild

**Territory:** `compiler/comptime.rs` + `compiler/comptime_target.rs` — comptime evaluator rebuild against typed-Arc HeapValue layout. §2.4 cites.

**Sites:** `compiler/comptime.rs:392,520,557,580,592,...` (6) + `compiler/comptime_target.rs:207` (1).

**Cited §:** §2.4.

**Time:** 6-10h.

**Risk:** medium. Comptime evaluator runs at compile time but uses the runtime HeapValue representation; the rebuild has to keep both sides in sync.

---

### C3-expr-lowering-misc

**Territory:** `compiler/expressions/{function_calls,property_access,mod,misc}.rs` + `compiler/loops.rs` + `compiler/helpers.rs`. Misc bytecode-emit paths citing §2.4 (typed-Arc HeapValue) or §2.7.4 (kinded marshal).

**Sites:** `function_calls.rs:152,157,163,431` (4) + `property_access.rs:201,326` (2) + `expressions/mod.rs:1079` (1) + `expressions/misc.rs:751` (1) + `loops.rs:1410` (1) + `helpers.rs:4836` (1). **~10 sites total.**

**Cited §:** §2.4 / §2.7.4.

**Time:** 4-6h.

**Risk:** low. Mechanical lowering paths.

---

### Compile-tier compiler_tests / monomorphization tests

`compiler_tests.rs:23` and `monomorphization/integration_tests.rs:19` (1 each) absorb into §3 test-placeholder rollup — they're test files, not production compile-tier work.

---

## §5 — Execution order / leverage analysis (post lock-in)

ADR-006 §2.7.24 landed 2026-05-11 — the carrier-shape ADR work is done; sub-clusters can dispatch in parallel waves.

**Phase 2d-Wave-1 (gates):** parallel, 2 agents
- **T1-host-tier-marshal-rebuild** (6-8h test infra — unblocks 200+ tests)
- **W17-make-closure** (4-6h production upstream gate — unblocks 4 array sub-clusters)

**Phase 2d-Wave-2 (independent body-fills):** parallel, up to 8 agents after Wave-1 lands
- W17-array-typed-receiver (3-5h)
- W17-iterator-tableview (4-6h)
- W17-references-mutation (4-6h) → gates W17-typed-object-mutation
- W17-builtin-coercions (3-5h)
- W17-foreign-ffi (6-8h)
- W17-typed-module-exports (6-8h)
- W17-array-closure-callback (4-6h) [needs Wave-1 W17-make-closure]
- W17-native-scalar-carrier (2-4h, verify scope first)
- W17-method-bodies-misc (2-3h, includes deque-ctor cleanup)
- **C1-temporal-lowering** (4-6h) [needs T1]

**Phase 2d-Wave-2.5 (large independent):** parallel, 4 agents
- W17-typed-object-mutation (6-8h) [needs Wave-2 W17-references-mutation]
- W17-snapshot-resume (8-16h) — largest single sub-cluster, independent
- W17-typed-carrier-monomorphization (24-32h) — consumes ADR-006 §2.7.24; new HeapKind TraitObject=29
- W17-concurrency (8-12h) — new HeapKinds Mutex=30, Atomic=31, Lazy=32; partial gate W17-make-closure for Lazy.get
- **C2-comptime-rebuild** (6-10h)
- **C3-expr-lowering-misc** (4-6h)

**Phase 2d-Wave-3 (dependent on Wave-2 + Wave-2.5):** serial
- W17-datatable-results (6-10h) [needs W17-typed-object-mutation + W17-typed-carrier-monomorphization + W17-array-closure-callback]
- W17-window-join (4-6h) [needs W17-datatable-results]

**Deferred to later handover items:**
- W17-jit-stubs → Item 3 (JIT verification) — depends on which kinds flow through hot paths post-W17
- W17-channel-blocking → Item 4 (mutation ADR) + cross-task §2.7.4 work — semantic boundary not yet ruled

**Parallel-dispatch capacity:** with 4-6 agents running concurrently, Phase 2d closes in ~4-6 elapsed days. With one agent serial, ~16-22 days. The verify-merge.sh discipline (`scripts/verify-merge.sh`, Item 7) gates every merge; collisions on dispatch-table arms are caught at verify time.

---

## §6 — Resolved decisions (2026-05-11)

All six original open questions resolved:

1. **Array + HashMap polymorphic catch-all** → **Option B (monomorphic specialization)**. `TypedArrayData::HeapValue` deleted; `HashMapData` becomes parametric via `HashMapValueBuf`. Per ADR-006 §2.7.24 Q25.A + Q25.B.

2. **Trait object carrier shape** → **Option A (kinded `TraitObjectStorage`) + universal `dyn`**. `HeapKind::TraitObject = 29` re-introduced; all traits dyn-able via the `Erase_T` auto-boxing rule + richer `VTableEntry` enum. Per ADR-006 §2.7.24 Q25.C.

3. **W17-make-closure scope** → "make" only (op_make_closure). The "call" side (op_call_closure / call_value_immediate_*) already landed in W7 (per AGENTS.md `w7-cv-static` / `w7-cv-method` rows). Sub-cluster scope: fill the op_make_closure SURFACE per §2.7.8 ClosureCell layout.

4. **W17-deque-ctor** → **stale, dropped as sub-cluster**. 5-line cleanup folded into W17-method-bodies-misc. `HeapKind::Deque = 23` already exists (W15-deque, 2026-05-10).

5. **W17-channel-blocking** → **defer to post-Item-4 (mutation ADR) + cross-task §2.7.4 work**. Confirmed.

6. **W17-jit-stubs** → **defer to Item 3 (JIT verification)**. Confirmed. The JIT FFI surface depends on which kinds flow through hot paths, which depends on the W17 method-handler outcomes.

**Three merged sub-clusters** (Q25.A array specialization + Q25.B HashMap parametric + Q25.C trait object rebuild) consolidate into **W17-typed-carrier-monomorphization** as a single 24-32h branch.

**HeapKind ordinals pre-assigned** for the new variants (ords 29-32 free at HEAD `45bd827`):

| Ord | Name | Sub-cluster | Source |
|---|---|---|---|
| 29 | TraitObject | W17-typed-carrier-monomorphization (P3) | ADR-006 §2.7.24 Q25.C |
| 30 | Mutex | W17-concurrency | (new — ADR amendment lands in W17-concurrency commit) |
| 31 | Atomic | W17-concurrency | (new — ditto) |
| 32 | Lazy | W17-concurrency | (new — ditto) |

Bump-on-collision per handover §0 ordinal-collision rule.

---

## §7 — How to use this inventory

- **Sub-cluster dispatch:** consult **`docs/cluster-audits/phase-2d-playbook.md`** for the per-sub-cluster agent prompts (required reading + forbidden patterns + smoke + gates) rather than this inventory's §2 brief. This inventory is the source-of-truth for sites and grouping; the playbook is the agent-facing dispatch contract.
- **Sites lookup:** for any sub-cluster, the per-file site list in §1.A / §1.B is the agent's worklist. The full per-line TSV with extracted blocker text lives at `/tmp/phase2d-audit/surface-structured.tsv` (will need regeneration if branch advances).
- **§-cite hygiene:** `[CITE-MISSING]` rows are pre-marked; sub-cluster owner fixes in their first commit per the agent prompt.
- **Test validation:** §3 maps every test file to the sub-cluster(s) it validates. After a sub-cluster lands, flip the corresponding test todos to actual bodies as part of the close commit.
- **Merge discipline:** every sub-cluster close runs through `bash scripts/verify-merge.sh` (Item 7) before merge. Gate is exit-code-based; the script catches the 4 take-both merge-survival patterns from W14/W15/W16.

---

*End of inventory.* Companion documents:

- **`docs/adr/006-value-and-memory-model.md` §2.7.24** — the bundled ADR amendment (Q25.A/B/C) this inventory's sub-cluster shapes consume.
- **`docs/cluster-audits/phase-2d-playbook.md`** — per-sub-cluster agent prompts ready for dispatch.
- **`scripts/verify-merge.sh`** — exit-code-based merge verification, run before every sub-cluster merge.
- **`AGENTS.md`** — roster rows for every Phase-2d sub-cluster (branch + worktree + status).
