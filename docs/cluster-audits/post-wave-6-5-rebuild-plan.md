# Post-Wave-6.5 Rebuild Plan — VM + JIT to Working

**Status:** drafted 2026-05-09 by post-Wave-6.5 supervisor.
**Branch:** `bulldozer-strictly-typed` HEAD `e0915f3`.
**Goal:** drive `cargo run --bin shape -- run program.shape` back to working,
end-to-end, with no Bool-default fallbacks and no defection-attractor framing.

---

## Stub coverage (audit at `e0915f3`)

`rg '\btodo!\(|VMError::NotImplemented\(' crates/shape-vm/src/` returns 536 sites;
shape-jit fails to compile with 213 errors. Every site is tracked under one of
the architectural surfaces below — no orphans.

| Surface | Count | Description | Wave |
|---|---|---|---|
| **CC1** closure-callback ABI | ~30 explicit + ~150 downstream | Kinded `op_call_value` / `call_value_immediate_*` → unblocks `arr.map/filter/reduce/orderBy/find/some/every/forEach/...` and custom-iterator method bodies | **W7** keystone |
| **T25** `HeapKind::SharedCell` variant | 2 | §2.7.9 mirror amendment for shared-cell allocation | W8 |
| **T26** `RefTarget`/`RefProjection` kinded | ~10 | `op_make_ref` family in `variables/mod.rs` | W8 |
| **EX** exception handler rebuild | ~11 | `exceptions/mod.rs` + `vm_impl/builtins.rs:315/613` | W8 |
| **WJ** window_join rebuild | ~7 | `executor/window_join.rs` element-kind materialization | W8 |
| **HM** HashMap typed-buffer mutation | ~12 | `hashmap_methods.rs` insert/remove/get with kind tracking | W9 (after CC1) |
| **IT** IteratorState rebuild | ~9 | `loops/mod.rs` for-await + custom iterators | W9 (after CC1) |
| **DT** datatable methods | ~12 | `objects/datatable_methods/{joins,query,simulation,aggregation}.rs` | W9 (after CC1) |
| **TR** trait-object dispatch | ~5 | `trait_object_ops.rs` BoxTraitObject/DynMethodCall/DropCall/DropCallAsync | W9 |
| **CT** comptime / @ai | ~7 | `compiler/comptime.rs` + scattered call-sites | W9 (parallelizable) |
| **AS** async / transport / remote | ~14 | `async_ops/`, `remote.rs`, `transport_builtins`, `vm_impl/modules.rs` | W9 |
| **OBJ** array/object method bodies | ~152 | Mostly CC1-blocked re-fill; small share blocked on T25/T26 | W9 mechanical migration |
| **TYP** typed_handlers / v2_handlers | ~15 | Some are dead dispatch (cluster-C audit); some live | W9 |
| **PRT** printing / debug | ~8 | `printing.rs` etc. | W9 |
| **JIT-W10** shape-jit consumer-side | 213 compile errors, 0 stubs | Whole `crates/shape-jit/src/ffi/` + `mir_compiler/` migration to kinded API | **W10** |
| **TST** test stubs | ~131 | Re-enable as their handlers come back online | W11 |
| **CLI** shape-cli polish | 2 | `repl_cmd.rs:62` `bail!` macro + match-arm | W11 |

**Total covered: 536/536 + 213 jit errors. Zero orphans.**

---

## Wave sequence

### W7 — CC1 closure-callback ABI (keystone)

**ADR amendment:** §2.7.11 / Q12 — kinded value-call dispatch ABI.
Mirror of §2.7.10/Q11 but for the value-call path:

- `call_value_immediate_*` family in `executor/call_convention.rs` → take
  `&[KindedSlot]`, return `Result<KindedSlot, VMError>`.
- `op_call_value` in `executor/control_flow/mod.rs` → kind-aware callee
  classification via `args[0].kind` (Closure / FunctionRef / TraitObjectMethod /
  ForeignFn), no tag decode, no `is_heap()` probe.
- Closure-call dispatch in `executor/closures/` extends the §2.7.8 cell-storage
  parallel-kind invariant: capture-kind from `OwnedClosureBlock` /
  `ClosureLayout::capture_native_kinds` flows into the new frame's parallel-kind
  track at frame setup.

**Forbidden** (refuse on sight in W7):
- `Vec<KindedSlot>` for the call frame (§2.7.7 #1).
- `&[NativeKind]` second-slice arg (§2.7.6 / Q8 carrier-API-bound).
- Bool-default fallbacks for unresolved-kind capture (§2.7.8 #4).
- "kind-injection bridge / call-frame translator / value-call adapter" naming
  (defection-attractor family — refuse on sight).

**Dispatch shape:** 5-8 parallel agents (one per dispatch shape):
1. `call_value_immediate_static` — direct closure call, kinded
2. `call_value_immediate_polymorphic` — fall-through path with kind-source from frame
3. `call_value_async` — async closure invocation
4. `call_value_method` (cross-link with §2.7.10) — method via closure callback
5. `comparator-closure pattern` — orderBy/thenBy/sort comparator
6. `predicate-closure pattern` — filter/find/some/every predicate
7. `transform-closure pattern` — map/reduce/forEach transform
8. `frame setup` — `CallFrame.closure_heap_kind` integration with new frame's parallel-kind track

**Gates:** zero `todo!()` in `call_convention.rs`; zero CC1 SURFACE messages
remain across `executor/objects/`; check-no-dynamic exit 0; binary `cargo
build -p shape-cli --bin shape --no-default-features` succeeds (modulo the 2
pre-existing repl_cmd issues, fixed in W11).

**ETA:** 1 supervised session (~3-5 hours agent fan-out).

### W8 — Structural amendments + small surfaces (parallel)

Parallel waves (independent, no cross-dependency):
- **T25** HeapKind::SharedCell variant amendment (§2.7.9 mirror, ~1 agent).
- **T26** RefTarget/RefProjection kinded redesign (`op_make_ref` family, ~2 agents).
- **EX** exception handler rebuild (`exceptions/mod.rs` + foreign_marshal, ~1 agent).
- **WJ** window_join rebuild (~1 agent).
- **AS** async / transport / remote — preparatory ABI work that's CC1-orthogonal (~2 agents).

**Total: ~7 parallel agents.** ETA: 1 supervised session.

### W9 — Mechanical method-body re-fill

Once CC1 + T25 + T26 land, ~150 method bodies in `executor/objects/*.rs` follow
the `array_sort.rs::handle_join_str_v2` recipe. Massive parallelism — assign
agents per-file.

**Sub-clusters:**
- **OBJ-array-transform** (18 stubs)
- **OBJ-array-query** (16)
- **OBJ-iterator-methods** (19)
- **OBJ-hashmap-methods** (12) — depends on HM ABI (W8)
- **OBJ-array-aggregation** (11)
- **OBJ-concurrency-methods** (10)
- **OBJ-array-operations** (8)
- **OBJ-array-basic + property_access + object_creation** (~16)
- **OBJ-datatable** (~12)
- **OBJ-misc** (range/priority_queue/deque/datetime/string/char/bool/set/matrix etc., ~15)
- **CT** comptime (~7) — parallelizable, no CC1 dep
- **TR** trait-object — depends on CC1 (5)
- **IT** iterator state — depends on CC1 (9)

**Total: ~12-15 parallel agents.** ETA: 1-2 supervised sessions.

### W10 — shape-jit consumer-side migration

Per ADR-006 §2.7.5 the JIT FFI is stable raw-bits; consumer-side translation
to kinded carriers happens at the VM↔JIT boundary in shape-jit. 213 compile
errors concentrate in:

- `ffi/value_ffi.rs` (33 errors) — value-bits ↔ kinded translation
- `ffi/object/conversion.rs` (31)
- `ffi/control/mod.rs` (23)
- `ffi/call_method/mod.rs` (18) — picks up new MethodFnV2 ABI
- `mir_compiler/rvalues.rs` (17)
- `ffi_symbols/vector/mod.rs` (17)
- `ffi/jit_kinds.rs` (12)
- `mir_compiler/places.rs`, `ownership.rs`, `v2_int.rs`, `blocks.rs` (~20 combined)
- Misc tail (~40)

**Sub-clusters:** 1 agent per ffi/ subsystem family. **Total: ~8-10 parallel
agents.** ETA: 2-3 supervised sessions.

### W11 — Tests + CLI polish (parallel)

- Re-enable ~131 test stubs whose handlers came back online in W7-W9-W10.
  Some bodies need re-thinking when the API shape changed; most are mechanical.
- shape-cli `repl_cmd.rs:62` (`bail!` macro + match-arm) — single-file fix.
- `just test-fast` baseline — capture which tests now pass / which still need fixture updates.

**Total: ~5-8 parallel agents.** ETA: 1 supervised session.

---

## Timeline summary

| Wave | What | ETA (supervised sessions) | Cumulative |
|---|---|---|---|
| W7 | CC1 closure-callback ABI | 1 | 1 |
| W8 | Structural + small surfaces | 1 | 2 |
| W9 | Method-body re-fill | 1-2 | 3-4 |
| W10 | shape-jit consumer migration | 2-3 | 5-7 |
| W11 | Tests + CLI polish | 1 | 6-8 |

**Realistic range: 6-8 supervised sessions to "shape run program.shape works
end-to-end on typical Shape programs (closures, iterators, async, snapshot
not-yet, JIT live)."**

Two notable exclusions held out of this plan (kept Phase-2c per ADR-006 §2.7.4):
- **Snapshot/restore** (~26 stubs) — niche feature; resumable distributed
  execution. Returns when an explicit user task asks for it.
- **Some async paths** that route through the closure-callback path beyond
  what W7 covers — likely a small follow-up to W9.

---

## Dispatch protocol per wave

1. Supervisor writes the wave's playbook in `docs/cluster-audits/wave-N-…-playbook.md`
   with cluster split, gates, forbidden patterns, surface-and-stop pattern.
2. Worktrees per agent under `../shape-wN-cX/` (mirror Wave 6.5 pattern).
3. AGENTS.md row updated at every state transition.
4. Cluster close: agent flips status to idle, supervisor merges into
   `bulldozer-strictly-typed`, captures Δ in commit message.
5. Wave close: supervisor verifies gates and commits a wave-close merge with
   an audit summary in the message.

Same supervisory pattern as Wave 6.5. Same gate machinery. Same forbidden lists.

---

## ADR-006 amendments expected

- **§2.7.11 / Q12** — kinded value-call dispatch ABI (W7).
- **§2.7.12 / Q13** — `HeapKind::SharedCell` variant (W8 / T25), Q8-amendment
  in the FilterExpr style.
- **§2.7.13 / Q14** — RefTarget/RefProjection kinded redesign (W8 / T26).

These are all incremental amendments in the same shape as §2.7.7-§2.7.10; no
load-bearing model-change is anticipated.
