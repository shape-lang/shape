# Phase 1.B-vm Wave 7 — CC1 closure-callback ABI Playbook

**Branch (parent):** `bulldozer-strictly-typed` HEAD `d389b24`
**Predecessor:** Wave 6.5 close `e0915f3` (Wave-α/β/γ/δ complete; ~150 method bodies SURFACE-blocked on CC1)
**ADR binding:** ADR-006 §2.7.11 / Q12 (added by `d389b24`); cross-references §2.7.7/Q9 (stack parallel-kind track), §2.7.8/Q10 (cell-storage parallel-kind track), §2.7.10/Q11 (method-dispatch ABI), §2.7.4 (Phase-2c deferral discipline)
**CLAUDE.md binding:** "Forbidden Patterns" + "Renames to refuse on sight" — value-call defection-attractor family added 2026-05-09

This playbook is binding for all 6 sub-clusters. Helper signatures, frame-setup
discipline, and dispatch shape are locked here so parallel agents converge
without cross-talk. Same supervisory pattern as the Wave 6.5 playbook §10.

---

## 1. Wave-7 deliverable

The pre-§2.7.11 ABI is `todo!()` for the entire value-call dispatch path. This
wave lands the kinded ABI on every entry-point in `executor/call_convention.rs`
and `executor/control_flow/mod.rs::op_call_value` (+ `op_call_closure`,
`op_call_function_indirect`, `dispatch_call_closure_like`). Result: closures-as-
values flow end-to-end:

- A Shape program calling `arr.map(|x| x + 1)`, `.filter`, `.reduce`,
  `.orderBy`, `.thenBy`, `.find`, `.some`, `.every`, `.forEach` (W9 mechanical
  re-fill unlocks here).
- `comptime fn` callable through `Function<A, R>`.
- Closure capture and reuse across frame boundaries.

W7 ships only the **dispatch ABI**. The ~150 method bodies in
`executor/objects/*.rs` that surface as "closure-callback path unmigrated"
re-fill in W9 per the `array_sort.rs::handle_join_str_v2` recipe.

---

## 2. Architectural ABI (locked per §2.7.11/Q12)

The ABI carries kind on the carrier, never as a side-channel. Every entry-point
in `call_convention.rs` is one of these shapes — refuse any other shape on
sight:

```rust
// Public/internal dispatch:
pub fn call_value_immediate_nb(
    &mut self,
    callee: &KindedSlot,                                       // borrow — caller owns the share
    args: &[KindedSlot],                                       // borrow — caller owns the shares
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<KindedSlot, VMError>;                              // returned share transfers to caller

// Internal frame setup:
pub(crate) fn call_function_with_nb_args(
    &mut self,
    func_id: u16,
    args: &[KindedSlot],
) -> Result<(), VMError>;

pub(crate) fn call_closure_with_nb_args_keepalive(
    &mut self,
    func_id: u16,
    closure_block: &OwnedClosureBlock,                         // canonical capture-kind source
    args: &[KindedSlot],
    closure_heap_bits: Option<u64>,
    closure_heap_kind: Option<NativeKind>,                     // B9 lockstep companion
) -> Result<(), VMError>;
```

**Locked rules:**

- The `&[KindedSlot]` slice is the canonical §2.7.1 case 4 dispatch-slice form.
- Borrow-only — `&[..]`, never `&mut [..]`, never `Vec<KindedSlot>` by-move.
- Result is `Result<KindedSlot, VMError>` — never `(u64, NativeKind)`,
  never raw `u64`.
- Capture kinds are recovered from `OwnedClosureBlock::read_capture_kinded(idx)`
  exclusively. The `Vec<u64>` upvalue payload that pre-§2.7.11 stubs carry as
  `_upvalue_bits: Vec<u64>` is the deleted-ABI shape; W7 replaces it with
  `closure_block: &OwnedClosureBlock`.

**ADR refinement (post-W7 audit):**

The §2.7.11 ruling specifies the public dispatch ABI shape — `(callee:
KindedSlot, args: &[KindedSlot]) -> Result<KindedSlot, _>` — and is correct.
However, the migration-scope text enumerates "5 dispatch entry-points" which
undercounts the actual surface. The actual `executor/call_convention.rs`
surface is 12 entry-points (public dispatch + internal frame-setup +
JIT-trampoline + `_raw` pair-slice family).

**The `&[(u64, NativeKind)]` pair-slice form is rejected** on §2.7.6/Q8
carrier-API-bound grounds at the runtime tier. The three `_raw` entry-points
that carry it (`call_value_immediate_raw`, `call_function_with_raw_args`,
`call_closure_with_raw_args`) migrate to either `&[KindedSlot]` or are deleted
as redundant with the kinded entry-points. `jit_trampoline_call_closure` is
the **only** `_raw` survivor — the §2.7.5 cross-crate stable-FFI consumer
where raw u64 + parallel kind is the canonical shape (consumers translate from
`&[KindedSlot]` to `&[u64]` at the FFI boundary, single direction).

This refinement keeps §2.7.11/Q12 architecturally consistent: one carrier
shape (`KindedSlot`) at the runtime-tier, one parallel-pair shape (raw u64 +
parallel `NativeKind`) at the storage/FFI-tier (stack, cells, JIT trampoline),
no third hybrid.

---

## 3. Sub-cluster carve-out

Six sub-clusters dispatched in two ordering bands.

### Band 1 — Foundation (sequential — frame-setup blocks cv-static)

| Sub-cluster | Files owned | Sites |
|---|---|---|
| **W7-frame-setup** | `executor/call_convention.rs` frame-setup helpers: `call_function_with_nb_args`, `call_function_from_stack`, `call_closure_with_nb_args_keepalive`. Refines `_upvalue_bits: Vec<u64>` parameter to `closure_block: &OwnedClosureBlock`. Marks `_raw` family for deletion (W7-cv-polymorphic owns the deletion). Cross-link with `executor/mod.rs::CallFrame` (read-only — B9 already landed). | 4 entry-points |
| **W7-cv-static** | `executor/call_convention.rs::call_value_immediate_nb` body (the dispatch entry-point). Match on `callee.kind`: `Ptr(HeapKind::Closure)` → recover `OwnedClosureBlock` via `slot.as_heap_value()` + `HeapValue::ClosureRaw(block)`, route to `call_closure_with_nb_args_keepalive`. `UInt64` → callee bits is the function-id, route to `call_function_with_nb_args`. | 1 dispatch body + ~3 internal helpers |

### Band 2 — Dispatch shells & extensions (4 parallel agents after Band 1 merges)

| Sub-cluster | Files owned | Sites |
|---|---|---|
| **W7-cv-polymorphic** | Delete `call_value_immediate_raw`, `call_function_with_raw_args`, `call_closure_with_raw_args` (the rejected `&[(u64, NativeKind)]` pair-slice family). JIT-side callers in `crates/shape-jit/` are W10 territory; they translate `&[KindedSlot]` → raw u64 at the FFI boundary directly. | 3 deletions |
| **W7-cv-async** | `executor/call_convention.rs::execute_with_async`, `resolve_spawned_task`. Sync-resolution only — suspension state crossing a `call_value_immediate_*` boundary is OUT OF SCOPE per §2.7.11 out-of-scope clause (Phase-2c snapshot-tier). | 2 entry-points |
| **W7-cv-method** | `executor/call_convention.rs::execute_function_by_name`, `_by_id`, `execute_closure`, `execute_function_fast`, `execute_function_with_named_args`, `resume`, `jit_trampoline_call_closure` (the only `_raw` survivor). Public callers in `shape-cli`, `shape-runtime`, `remote.rs`. | 7 entry-points |
| **W7-op-call-value** | `executor/control_flow/mod.rs`: `op_call_value`, `op_call_closure`, `op_call_function_indirect`, `dispatch_call_closure_like`. The JIT-dispatch surface in `op_call` (lines 222–236) stays SURFACE until W10. | 4 dispatch shells |

### Round 2.5 — W7-closure-retain (inserted post-Round-2 audit, 2026-05-09)

**Sequential, blocks Round 3.** Surfaced at W7-cv-static close: `clone_with_kind`
and `drop_with_kind` in `crates/shape-vm/src/executor/vm_impl/stack.rs:130/243`
are `debug_assert!(false)` for `HeapKind::Closure` (and `Future` /
`NativeScalar`). The comment-design says "no Arc<T> slot payload routed through
this path" — closures were modeled as single-ownership `Box<HeapValue>` outside
the standard Arc-counted retain-on-read path.

This conflicts with the §2.7.11 dispatch-shell shape: `op_call_value`
(Round 3) constructs `KindedSlot` args from popped stack values, and those
carriers' `Drop` calls `drop_with_kind` keyed on each `kind` — including
`HeapKind::Closure` when a closure is passed as an arg or as the callee. The
debug_assert WILL trip in error paths and on Gate 5 smoke (which is built
around `arr.map(|x| ...)` where the closure value flows through the dispatch
shell).

**W7-closure-retain sub-cluster** wires the share semantics for
`HeapKind::Closure` (and `Future`, `NativeScalar`) per the FilterExpr precedent
(`Arc::increment_strong_count(bits as *const FilterNode)` at the matching
typed-Arc target). Investigation needed to determine the exact share carrier:

- `HeapValue::ClosureRaw(OwnedClosureBlock)` — the variant carries an
  `OwnedClosureBlock` directly (not `Arc<OwnedClosureBlock>`).
- `ValueSlot::as_heap_value()` reinterprets `slot.0 as *const HeapValue` —
  meaning the slot bits ARE a `*const HeapValue` pointer.
- The `*const HeapValue` is presumably an `Arc<HeapValue>` (or `Box<HeapValue>`,
  but Box has no clone path — and the slot does need clone-on-share semantics).
- Determine which it is by reading `from_typed_closure` / closure-push call
  sites (probably in `executor/control_flow/mod.rs::op_make_closure` or
  `executor/variables/mod.rs` closure-allocation paths).

**Body shape (forecast — verify in audit):**

```rust
HeapKind::Closure => {
    Arc::increment_strong_count(bits as *const HeapValue);  // or *const OwnedClosureBlock
}
// drop side:
HeapKind::Closure => {
    Arc::decrement_strong_count(bits as *const HeapValue);
}
```

Same dispatch for `Future` and `NativeScalar` if they share the same
`Box<HeapValue>` / `Arc<HeapValue>` shape; otherwise they get their own arms
per their actual share carrier.

**Gates:** `cargo build -p shape-vm --lib` succeeds; `bash scripts/check-no-dynamic.sh`
exit 0; AGENTS.md row flipped to idle.

**Time budget:** 1-2 hours. Single-file edit. If the audit reveals the share
semantics are more complex than Arc<HeapValue> (e.g., OwnedClosureBlock has its
own Arc), the sub-cluster forks into a deeper investigation and surfaces back.

### Out-of-band (W9, NOT this wave)

**W7-stub-refill** (the §2.7.11 forecast names this) is **NOT this wave's
deliverable.** It is W9 mechanical migration per the plan in
`docs/cluster-audits/post-wave-6-5-rebuild-plan.md` §W9.

---

## 4. Body shapes per sub-cluster

### API name corrections (post-W7-cv-static audit, 2026-05-09)

Three minor playbook typos were surfaced by Round 2 and corrected in code at
W7-cv-static close (commit `06cdfce`). Round 3 agents must use the corrected
names:

- **`run_until_return(ctx)` does NOT exist.** The actual return-driver is
  `pub(crate) fn execute_until_call_depth(target_depth: usize, ctx: ...) -> Result<(), VMError>`
  at `executor/dispatch.rs:470`. Pattern: capture
  `let saved_depth = self.call_stack.len();` *before* frame setup, drive
  `self.execute_until_call_depth(saved_depth, ctx)?`, then `pop_kinded()` for
  the return value as `(u64, NativeKind)` → wrap as `KindedSlot`.
- **`slot.bits()` does NOT exist on `ValueSlot` / `KindedSlot`.** The accessor
  is `slot.raw()` returning `u64`. Use `callee.slot.raw()` and
  `slot.slot.raw()`.
- **`VMError::TypeError(format!(...))` does NOT compile.** The variant is
  `TypeError { expected: &'static str, got: &'static str }` (named struct
  fields). For dispatch-error surfaces use `VMError::RuntimeError(format!(...))`
  per the existing `op_call_value` precedent.

### W7-frame-setup

`call_function_with_nb_args(&mut self, func_id: u16, args: &[KindedSlot]) -> Result<(), VMError>`:

Push new `CallFrame { return_ip, base_pointer = self.sp, locals_count =
descriptor.locals_count, function_id: Some(func_id), upvalues: None,
blob_hash: descriptor.blob_hash, closure_heap_bits: None, closure_heap_kind:
None }`. Walk `args` and `stack_write_kinded(base_pointer + i, slot.bits(),
slot.kind)` per playbook 6.5 §3. The dispatch shell in `op_call_value` uses
`mem::forget` on each slot of its args vec after it's been consumed by this
function — same pattern as §2.7.10 dispatch shell.

`call_closure_with_nb_args_keepalive(&mut self, func_id: u16, closure_block:
&OwnedClosureBlock, args: &[KindedSlot], closure_heap_bits: Option<u64>,
closure_heap_kind: Option<NativeKind>) -> Result<(), VMError>`:

Same `CallFrame` push as `call_function_with_nb_args`, but threads
`closure_heap_bits` + `closure_heap_kind` through (B9 lockstep — both `is_some`
or both `is_none`, enforced by `debug_assert_eq!`). Walks
`closure_block.layout().capture_count()` calling `read_capture_kinded(idx)` per
capture, writes each `(bits, kind)` into the new frame's reserved capture-
locals via `stack_write_kinded`. Then walks `args` per the non-closure path.

`call_function_from_stack(&mut self, func_id: u16, arg_count: usize) -> Result<(), VMError>`:

Fast path — pops `arg_count` slots via `pop_kinded()` directly into the new
frame's local slots (no intermediate `Vec`). Per playbook 6.5 §3, the pop
transfers the share to the local-write directly via `stack_write_kinded`. No
clone, no drop. Sentinel-fill omitted-arg slots with `(0u64, NativeKind::Bool)`
per playbook 6.5 §2 Null/Unit row.

### W7-cv-static

`call_value_immediate_nb(&mut self, callee: &KindedSlot, args: &[KindedSlot],
ctx: Option<&mut ExecutionContext>) -> Result<KindedSlot, VMError>`:

```text
match callee.kind {
    NativeKind::Ptr(HeapKind::Closure) => {
        let block = match callee.slot.as_heap_value() {
            HeapValue::ClosureRaw(block) => block,
            other => /* debug_assert + RuntimeError("HeapKind::Closure label but {:?} payload") */
        };
        let function_id = unsafe { typed_closure_function_id(block.as_ptr()) };
        self.call_closure_with_nb_args_keepalive(
            function_id, block, args,
            Some(callee.slot.bits()),
            Some(callee.kind),
        )?;
        let result = self.run_until_return(ctx)?;
        Ok(result)
    }
    NativeKind::UInt64 => {
        let function_id = callee.slot.bits() as u16;
        self.call_function_with_nb_args(function_id, args)?;
        let result = self.run_until_return(ctx)?;
        Ok(result)
    }
    other => Err(VMError::TypeError(format!(
        "value-call: callee must be Closure or function-ref, got {:?}", other
    ))),
}
```

Forbidden: tag_bits decode, `is_heap()` probe, fabricated kind, polymorphic
fall-through that fabricates kinds.

### W7-cv-polymorphic

Delete `call_value_immediate_raw`, `call_function_with_raw_args`,
`call_closure_with_raw_args`. Surface as `compile_error!("deleted by W7 — use
call_function_with_nb_args + JIT-side &[KindedSlot] translation per §2.7.5")`
if any caller remains inside `crates/shape-vm/`. (JIT-side callers in
`crates/shape-jit/` are W10 territory; their breakage is expected — W7's gate
is `cargo build -p shape-vm --lib`, not `cargo build`.)

### W7-cv-async

`execute_with_async(&mut self, ctx: ...) -> Result<KindedSlot, VMError>`:

Drive the task scheduler loop calling `resolve_spawned_task` per ready task.

`resolve_spawned_task(&mut self, task_id: u64) -> Result<KindedSlot, VMError>`:

Look up the task's `OwnedClosureBlock` from `task_scheduler::Task`, call
`call_closure_with_nb_args_keepalive(function_id, block, &[], Some(bits),
Some(NativeKind::Ptr(HeapKind::Closure)))`, drive `run_until_return`, return
result as `KindedSlot`.

**Surface-and-stop trigger:** if `task_scheduler::Task` carries a `Vec<u64>`
capture payload that hasn't been §2.7.8 cell-extended, surface to supervisor
for a W7-task-scheduler-cells follow-up sub-cluster (B7-style cell extension).

### W7-cv-method

Each public entry-point routes through frame-setup + `run_until_return`:

- `execute_function_by_name(name, args, ctx)` → resolve name→id via the
  function table, route to `execute_function_by_id`.
- `execute_function_by_id(func_id, args, ctx)` → call
  `call_function_with_nb_args(func_id, &args)`, `run_until_return`, return.
- `execute_closure(closure_block, args, ctx)` → call
  `call_closure_with_nb_args_keepalive(closure_block.function_id(),
  closure_block, &args, Some(bits), Some(NativeKind::Ptr(HeapKind::Closure)))`,
  `run_until_return`, return. **Replace pre-existing `_upvalue_bits: Vec<u64>`
  parameter with `closure_block: &OwnedClosureBlock`.**
- `execute_function_fast(...)` → fast-path entry; same shape with omitted-arg
  sentinel handling.
- `execute_function_with_named_args(...)` → map `&[(String, KindedSlot)]` to a
  positional `Vec<KindedSlot>` via `descriptor.param_names` lookup, then route
  through `call_function_with_nb_args`.
- `resume(...)` → resume from suspension state; **§2.7.4 Phase-2c stays
  `todo!()`** if the suspension shape requires snapshot-tier work.
- `jit_trampoline_call_closure(func_id, upvalue_bits, args, ctx)` —
  `&[(u64, NativeKind)]` form is the §2.7.5 stable-FFI consumer. Body wraps
  `args` as transient `&[KindedSlot]` via `KindedSlot::new` per slot (no Arc
  bump — the JIT pre-incremented), calls `call_closure_with_nb_args_keepalive`,
  returns `result.slot.bits()` as raw u64 (kind discarded — the JIT caller
  knows the static return kind).

### W7-op-call-value

`op_call_value(&mut self) -> Result<(), VMError>`:

```text
let arg_count = /* operand or popped */;
let mut args: Vec<KindedSlot> = Vec::with_capacity(arg_count);
for _ in 0..arg_count {
    let (bits, kind) = self.pop_kinded()?;
    args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
}
args.reverse();
let (callee_bits, callee_kind) = self.pop_kinded()?;
let callee = KindedSlot::new(ValueSlot::from_raw(callee_bits), callee_kind);
let result = self.call_value_immediate_nb(&callee, &args, ctx)?;
self.push_kinded(result.slot.into_raw(), result.kind)?;
std::mem::forget(result);
// args + callee drop at end of scope, releasing shares via drop_with_kind
Ok(())
```

Same precedent as §2.7.10 op_call_method dispatch shell pseudocode in
`executor/objects/mod.rs:267-296`.

`op_call_closure`, `op_call_function_indirect`, `dispatch_call_closure_like`
follow the same body shape; arg-count comes from the opcode operand rather
than a popped value where applicable.

---

## 5. Cross-cluster surfaces and ordering

```
┌─────────────────────────┐
│  W7-frame-setup         │  Round 1 (sequential)
│  (call_function_with_   │
│   nb_args, call_closure │
│   _with_nb_args_keep,   │
│   call_function_from_   │
│   stack)                │
└────────┬────────────────┘
         v
┌─────────────────────────┐
│  W7-cv-static           │  Round 2 (sequential)
│  (call_value_immediate_ │
│   nb)                   │
└────────┬────────────────┘
         │
   ┌─────┼─────────┬──────────────┬────────────────┐
   v     v         v              v                v
W7-cv-  W7-cv-  W7-cv-         W7-op-call-       (W9 stub-
poly-   async   method         value (control_   refill,
morphic         (5 entry-      flow shells)      OUT OF
(delete         points + jit_                    SCOPE)
_raw)           trampoline)
              ─── Round 3 (4 parallel) ───
```

**Cross-cluster cascade triggers:**

- `task_scheduler::Task` does not carry an `OwnedClosureBlock` — surface to
  supervisor for a W7-task-scheduler-cells follow-up sub-cluster.
- The `Vec<u64>` upvalue payload in `CallFrame.upvalues` cannot be dropped in
  favor of `OwnedClosureBlock` reference — surface; this is a B7+ residual
  not anticipated in §2.7.11.
- The JIT dispatch table expects raw `u64` callee bits — **W10 territory by
  design**; W7 keeps the JIT fast-path in `op_call:222-236` as SURFACE.

---

## 6. Forbidden patterns (this wave)

Wave 6.5 playbook §4 list applies verbatim. ADR-006 §2.7.11 forbidden list
applies verbatim. **Additions specific to W7:**

| # | Forbidden | Why |
|---|---|---|
| 12 | `Vec<KindedSlot>` by-move into a frame-setup helper | §2.7.11 — caller owns shares; by-move desynchronizes drop accounting |
| 13 | `&[(u64, NativeKind)]` pair-slice as a runtime-tier dispatch ABI | §2.7.6/Q8 carrier-API-bound — JIT trampoline is the only §2.7.5 stable-FFI exception |
| 14 | Dispatching on `callee.slot.bits()` to classify Closure vs FunctionRef | §2.7.7 #4 / #7 — deleted tag_bits dispatch |
| 15 | `ClosureLayout.capture_kind(i)` (FieldKind one) for `drop_with_kind` dispatch | §2.7.8 — needs NativeKind via `capture_native_kind(i)` or `read_capture_kinded(idx).1` |
| 16 | Bool-default fallback for unresolved-kind capture at frame setup | §2.7.8 #4 / §2.7.11 — surface-and-stop instead |
| 17 | Re-introducing `call_value_legacy` / `call_value_raw_u64` / `dispatch_value_call_handler_raw` / `call_value_with_u64_slice` | §2.7.11 forbidden list — W-series defection-attractor at value-call layer |
| 18 | Defection-attractor descriptors: "value-call bridge" / "closure-callback translator" / "frame-setup probe" / "callee-kind helper" / "capture-injection adapter" / "value-call shim" | CLAUDE.md "Renames to refuse on sight" — 2026-05-09 broadening |

**On encountering a forbidden shape:** surface to supervisor. Do not paper over.

---

## 7. Per-sub-cluster definition of done

A sub-cluster closes when **all** of the following hold:

1. **Zero `todo!()` in the sub-cluster's owned files** (cumulative across
   sub-clusters: `call_convention.rs` reaches 0 only after the wave closes).
2. **Zero `VMError::NotImplemented` referencing `PHASE_2C_CALL_REBUILD_SURFACE`
   or "closure-callback path unmigrated" or "call_value_immediate_nb rebuild
   pending"** in the sub-cluster's owned files.
3. **No new forbidden-pattern introductions** (Wave 6.5 §4 + W7 §6 above).
4. **`bash scripts/check-no-dynamic.sh` exits 0** (defection guard).
5. **shape-vm builds:** `cargo build -p shape-vm --lib` succeeds.
6. **AGENTS.md updated:** sub-cluster's row flipped to `idle` with close hash.
   (Supervisor maintains the W7 row family; agents do not edit AGENTS.md
   themselves — Wave 6.5 §10 protocol.)
7. **Sub-cluster commit message** cites this playbook + ADR-006 §2.7.11 + the
   per-cluster pattern from §4 above.

**Wave-level gates (mirror Wave 6.5 §7 REVISED — binary, not strictly-
decreased):**

- **Gate 1:** `grep -n 'todo!(' crates/shape-vm/src/executor/call_convention.rs`
  returns zero matches.
- **Gate 2:** `grep -rn "closure-callback path unmigrated\|call_value_immediate_nb rebuild pending\|PHASE_2C_CALL_REBUILD_SURFACE" crates/shape-vm/src/executor/control_flow/`
  returns zero matches.
- **Gate 3:** `bash scripts/check-no-dynamic.sh` exits 0.
- **Gate 4:** `cargo build -p shape-vm --lib` succeeds.
- **Gate 5 (end-to-end smoke):** a one-line Shape program — e.g.,
  `let xs = [1, 2, 3]; let ys = xs.map(|x| x + 1); print(ys[0])` — runs to
  completion under `cargo run --bin shape -- run smoke.shape` and prints `2`.
  Requires: W7 dispatch ABI live + `array_transform::handle_map_v2` body
  re-fill (one of the W9 stubs). Supervisor pre-migrates `handle_map_v2` only
  as part of W7 wave-close (mechanical migration per `array_sort.rs::handle_join_str_v2`
  recipe — single PHF entry, ~30 lines). Other ~149 W9 bodies stay SURFACE.

**`cargo check -p shape-vm --lib` error count is INFORMATIONAL, NOT GATING.**
Same Wave 6.5 §7 REVISED rule.

---

## 8. Surface-and-stop triggers

Stop, stash WIP, flip AGENTS.md row to `blocked`, and surface to supervisor on:

- **Capture kind unsourceable** — `OwnedClosureBlock::read_capture_kinded`
  panics or returns a kind that doesn't match the expected closure layout.
  Likely B7 cell-extension regression — surface for cluster B-round-3.
- **`task_scheduler::Task` shape mismatch** — async closure resolution finds
  `Vec<u64>` captures with no parallel `Vec<NativeKind>` track. Surface for a
  W7-task-scheduler-cells sub-cluster (B7-style cell extension).
- **JIT dispatch fast-path required** — `op_call:222-236` JIT path needed by
  a test program before W10 lands. Surface; W10 boundary is hard.
- **Async closure suspension shape** — a `call_value_immediate_*` invocation
  must yield mid-execution and resume later. Surface; §2.7.4 Phase-2c /
  §2.7.11 out-of-scope (snapshot-tier).
- **Trait-object closure dispatch** — `op_call_value` with a callee of kind
  `Ptr(HeapKind::TypedObject)` carrying a `dyn Trait` vtable. Surface;
  trait-object dispatch is W9 TR territory. The dispatch on `callee.kind`
  should fall through to `Err(VMError::TypeError(...))` for non-Closure
  non-UInt64 callee kinds.
- **`UInt64` callee bits don't match a real function-id** — likely a
  compile-time bug at the producing opcode. Surface as `RuntimeError`.

**Do not surface for:** drop-discipline ambiguity at a single call site (read
playbook 6.5 §3 — re-push or `drop_with_kind`); helper signature questions
answered in §2 above; pattern-recognition at heap-kind dispatch (use
`slot.as_heap_value()` + `HeapValue::ClosureRaw` per Q8).

---

## 9. Risk surface

| Risk | Disposition |
|---|---|
| **Closure-capture kind drift** (§2.7.8 invariant violated; `read_capture_kinded` returns kind ≠ producing-opcode emit) | Surface to supervisor with closure-construction call-site. Likely B7 regression; cluster B-round-3 audit. `debug_assert_eq!` in §2.7.11 ADR catches in debug builds. |
| **Async closure resumption shape** (suspension crossing `call_value_immediate_*`) | OUT OF SCOPE per §2.7.11 out-of-scope clause. Stays `todo!("phase-2c")`. |
| **Tail-call optimization interaction** (`TailCall` opcode needs different frame-setup) | Audit `crates/shape-vm/src/bytecode/` for `TailCall`. If present, surface for separate sub-cluster. Not anticipated. |
| **Trait-object method dispatch through closure** | Closure dispatch in-scope; trait-object method invocation inside closure body is W9 TR. Surface only if smoke crosses both surfaces. |
| **`HeapValue::HostClosure` arm referenced in stale doc comments** | Variant deleted; only `ClosureRaw` survives. Doc-comment cleanup is part of W7 close. Documentation hygiene; not code risk. |
| **Function-id encoded as `UInt64` instead of dedicated `NativeKind::FunctionRef`** | Locked by W7. Future ADR-006 §2.7.x amendment if needed for richer callee classification. Not W7 scope. |

**Top 2 supervisor concerns:**

1. **Closure-capture kind drift.** B7/B8/B9 landed cell-storage parallel-kind
   in Wave-α, but no end-to-end smoke has yet driven a real closure through
   both `op_make_closure` (push) and `call_value_immediate_nb` (pop+invoke).
   W7 is first wave exercising both ends together. **Mitigation:** pre-flight
   `op_make_closure` audit before W7-cv-static dispatches.
2. **Pair-slice deletion cascade.** Three `_raw` entry-points use the
   §2.7.11-rejected `&[(u64, NativeKind)]`. The deletion may surface JIT-side
   callers in `crates/shape-jit/` that are out-of-W7-scope (W10).
   **Mitigation:** `cargo build -p shape-vm --lib` is the W7 gate (Gate 4),
   not `cargo build`; shape-jit consumer breakage is W10's gate.

---

## 10. What's NOT in W7

Explicit out-of-scope list:

- **W7-stub-refill** (~150 method bodies in `executor/objects/*.rs`). **W9.**
  Exception: `array_transform::handle_map_v2` migrates as part of Gate 5.
- **`HeapKind::SharedCell` variant amendment** — **W8 / T25.**
- **`RefTarget` / `RefProjection` kinded redesign** — **W8 / T26.**
- **Exception handler rebuild** (`exceptions/mod.rs`) — **W8 / EX.**
- **`window_join` rebuild** — **W8 / WJ.**
- **HashMap typed-buffer mutation** — **W9 / HM.**
- **IteratorState rebuild** — **W9 / IT.**
- **DataTable methods** — **W9 / DT.**
- **Trait-object dispatch** — **W9 / TR.**
- **comptime / @ai paths** — **W9 / CT.**
- **Async / transport / remote** — **W9 / AS.**
- **shape-jit consumer-side migration** (213 errors) — **W10 by design.**
- **Test stubs** (~131) — **W11.**
- **shape-cli polish** (`repl_cmd.rs:62`) — **W11.**
- **Snapshot/restore of in-flight value calls** — §2.7.4 Phase-2c. Stays
  `todo!()` indefinitely until snapshot-tier work begins.

---

## 11. Pointers

- ADR-006: `docs/adr/006-value-and-memory-model.md` (§2.7.11 / Q12)
- Forbidden patterns: `CLAUDE.md` "Forbidden Patterns" + "Renames to refuse
  on sight"
- Wave 6.5 playbook (template for §7 REVISED gate machinery):
  `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md`
- Post-Wave-6.5 rebuild plan: `docs/cluster-audits/post-wave-6-5-rebuild-plan.md`
- Defection log (append-only): `docs/defections.md`
- §2.7.10 dispatch-shell precedent (read before W7-op-call-value):
  `executor/objects/mod.rs:267-296`
- Closure capture-kind source API:
  `crates/shape-value/src/v2/closure_raw.rs::OwnedClosureBlock::read_capture_kinded`
- Closure layout parallel-kind track:
  `crates/shape-value/src/v2/closure_layout.rs::ClosureLayout::capture_native_kind`
- B9 lockstep field on `CallFrame`: `executor/mod.rs:216-228`

---

*Playbook closed for edits during sub-cluster fan-out. Amendments require
supervisor sign-off and a fresh dispatch round.*
