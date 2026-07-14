# Vertical Deep-Dive 04 — VM Interpreter & Dispatch

Auditor 04 of 19 · 2026-07-11 · working tree (dirty) at `main` (HEAD ce332ca2 + uncommitted work)

Territory: `crates/shape-vm/src/executor/` (all subdirs), `feedback.rs`, `tier.rs`,
`resource_limits.rs`, the PHF method registry (`executor/objects/method_registry.rs`),
the stack parallel-kind track (`executor/vm_impl/stack.rs`), `executor/call_convention.rs`,
`executor/vm_state_snapshot.rs` (execution side).

All file:line references are against the dirty working tree on 2026-07-11. Empirical
transcripts were produced with the prebuilt binary `target/debug/shape` from this tree
(extension-load warnings for stale `~/.shape/extensions` are elided from transcripts).
Scratch programs live under the session scratchpad `verticals/vm-interpreter/` (t1..t25).

---

## 0. Executive summary

**Overall health verdict: structurally strong core, with one serious carrier-shape
split-brain and a ring of user-reachable panics/surfaces at the edges.**

The interpreter core — the typed 8-byte slot stack with its parallel `Vec<NativeKind>`
track, the kind-dispatched retain/release primitives (`clone_with_kind` /
`drop_with_kind`), the frame setup/teardown protocol, and the PHF method registry — is
disciplined, heavily documented, and conforms to ADR-005/ADR-006 in live code. Every
forbidden-pattern sentinel grep comes back clean (only doc-comments describing deleted
code match). Empirically the interpreter executes the mainstream language surface
correctly: closures, HOF array methods, enums/match, checked integer arithmetic,
break-with-value, graceful stack-overflow and division-by-zero errors, and enforced
instruction/wall-time/output resource caps.

The main defect is a **three-way carrier-shape split-brain on
`CallFrame.closure_heap_bits`**: the live call path stores `Arc<HeapValue>` pointer bits,
the snapshot-restore path stores a raw `TypedClosureHeader` block pointer under the *same*
`NativeKind::Ptr(HeapKind::Closure)` label, and two introspection consumers assume the
raw-block shape while the teardown consumer assumes the Arc shape. Every combination
except (live-producer → teardown) is wrong-type pointer reinterpretation — exactly the
"parallel-implementation across producer/consumer carrier-shape boundaries"
defection-attractor CLAUDE.md names. One consumer runs unconditionally on **every stdlib
module-function dispatch** (`capture_vm_state`, `vm_impl/modules.rs:848`).

Around that core, a set of `todo!()`/`NotImplemented` surfaces are reachable from
ordinary user code (`print(closure)` panics the process; every Range method except `iter`
is rejected by the compiler despite a fully-built PHF registry; mutating a collection
bound with plain `let` silently loses the mutation), and error-path frame unwinding is
inconsistent (the closure arm of the value-call ABI releases keep-alive shares on error,
but the function-id arm, `resolve_spawned_task`, and `handle_exception` do not).

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P0 | `CallFrame.closure_heap_bits` carrier split-brain: live producer stores `Arc<HeapValue>` bits (`call_convention.rs:1126-1133`), snapshot-restore stores raw `TypedClosureHeader` ptr (`snapshot.rs:527`) under the same kind; teardown treats all as Arc (`control_flow/mod.rs:1236-1250`), introspection/capture treat all as raw block (`vm_state_snapshot.rs:156-165`, `snapshot.rs:1025-1043`) | §9.1 |
| 2 | P0/P1 | `capture_vm_state` runs on every Typed module-fn dispatch and walks the misread path for any live closure frame — silent wrong data today, wrong-offset `retain_typed_closure` corruption when a garbage fn_id collides with a registered layout | `vm_impl/modules.rs:848`; t21 transcript §9.1 |
| 3 | P1 | `print(closure)` panics the whole process via `todo!()` — user-reachable from 2-line program | `printing.rs:430`; t25 transcript §9.2 |
| 4 | P1 | `handle_exception` truncates `call_stack` without releasing per-frame `closure_heap_bits` keep-alive shares → leak per unwound closure frame | `exceptions/mod.rs:151` §9.3 |
| 5 | P1 | Error-unwind asymmetry in the value-call ABI: closure arm unwinds (`call_convention.rs:1159-1162`), `UInt64` arm (`:1187`) and `resolve_spawned_task` (`:622`) do not → stale frames + leaked shares on error | §9.4 |
| 6 | P1 | `RANGE_METHODS` PHF (toArray/contains/start/end/step/len) is dead: compiler rejects every Range method — "Method 'toArray' not found on type 'Range'" | `method_registry.rs:1062-1073`; t8/t9 §9.5 |
| 7 | P1 | Mut-self method on immutable `let` binding silently loses the mutation: `let m = HashMap(); m.set("a",1); m.get("a")` → `None`, no compile error | t4b/t6 transcript §9.6 |
| 8 | P1 | Default `--mode jit` turns Shape recursion into a **host** stack overflow (process crash); `--mode vm` correctly errors `Stack overflow` — interpreter guard exists, JIT tier lacks one (cross-vertical, JIT) | t2 transcript §9.7 |
| 9 | P2 | `VirtualMachine::resume()` is `todo!()` (host-visible pub API); `apply_pending_resume` carries 5 `NotImplemented` surfaces — yet CLI `--resume` works via a separate `from_snapshot` driver (split restore paths) | `call_convention.rs:374`, `resume.rs:129-151`; t14 §2.4 |
| 10 | P2 | IC fast paths are ~80% dead scaffolding (`#![allow(dead_code)]` module-wide); only property IC + call feedback are live; arithmetic feedback never recorded | `ic_fast_paths.rs:15,167-174` §2.6 |

Additional notable P2s: `capture_vm_state` clones **all** module bindings + frame locals
(with refcount bumps) on every stdlib module-fn call (perf tax, §11); `record_allocation`
in `resource_limits.rs:138` is dead code (memory ceiling went to `alloc_budget` instead);
4 dispatch-loop bodies with copy-pasted preamble blocks and 11 byte-identical
`op_return_value_*` handlers (§4).

### Scores

**Feature completeness: 78/100.** The mainstream execution surface (calls, closures,
collections, method dispatch, arithmetic, exceptions-as-Result, resource limits, tiering
scaffolding, snapshot capture/restore for closure-free frames) works end-to-end and is
empirically verified; the deductions are the reachable `todo!()` surfaces (closure/
NativeScalar printing, `resume()` entry), the dead Range/i-array method registries, the
unfinished IC fast paths, and the snapshot/closure interaction being barrier-refused.

**Code quality: 72/100.** Exceptional invariant documentation and disciplined unsafe
usage at the slot layer, offset by 1,243 `unsafe` occurrences across the executor,
massive per-kind handler duplication (4,240-line variables/mod.rs, 5,355-line
v2_array_detect.rs), four copy-pasted dispatch loops, and comment-vs-code drift
(stale `ValueWord` vocabulary in field docs, a `vm_state_snapshot.rs` SAFETY comment that
asserts the wrong carrier shape — and is load-bearing wrong, see finding #1).

### Biggest risk

The biggest risk is that the `closure_heap_bits` split-brain is *currently masked* by two
accidents: (a) the snapshot closure-barrier refuses checkpoints while closure values are
reachable, so the restore-side wrong-type release (`Arc::decrement_strong_count` against
a malloc'd closure block) cannot fire today; and (b) garbage function-ids read from
misinterpreted `HeapValue` bytes usually miss the `closure_function_layouts` table, so
the live-path misread degrades silently instead of corrupting. Both maskings are
incidental, not designed: the moment closure snapshotting is enabled (it is a stated
v0.3.3 goal — W17 snapshot completion is release-blocking per project memory) the
restore→teardown path becomes a deterministic heap-corruption bug, and any program whose
closure-block pointer low bits happen to index a registered layout corrupts memory
*today* on an ordinary stdlib call. This is exactly the class of latent UB that survives
test suites and detonates after a release.

---

## 1. Architecture & code structure map

### 1.1 Territory inventory

The executor is 158 `.rs` files, **103,760 LOC total** (21,665 of which are test files
under `executor/tests/`), plus the three root-level territory files:

| Module | LOC | Responsibility |
|--------|-----|----------------|
| `executor/` root files | 19,575 | mod.rs (VM struct, Drop, module-binding kinded API), dispatch.rs (4 run loops), call_convention.rs (frame setup, value-call ABI), snapshot.rs (capture/restore), resume.rs, osr.rs, printing.rs, typed_object_ops.rs, trait_object_ops.rs, time_travel.rs, task_scheduler.rs, window_join.rs, ic_fast_paths.rs, vm_state_snapshot.rs, feature tests |
| `executor/objects/` | 25,031 | PHF method registry + per-receiver-kind method handler modules (array_*, hashmap, set, deque, priority_queue, datetime, string, number, matrix, content, datatable, iterator) |
| `executor/vm_impl/` | 7,893 | stack.rs (kinded stack + clone/drop_with_kind), builtins.rs, modules.rs (module-fn dispatch), program.rs, init, output |
| `executor/builtins/` | 7,231 | BuiltinCall dispatch, math, type_ops, datetime, remote/transport builtins, intrinsics |
| `executor/v2_handlers/` | 7,228 | v2 typed-array opcode family (per-element-type monomorphized) + `v2_array_detect.rs` (5,355 LOC carrier recognition/dispatch) |
| `executor/variables/` | 4,240 | Load/Store local/module-binding/capture opcode families (per-width monomorphized), SharedCell alloc/load/store |
| `executor/control_flow/` | 4,193 | jumps, calls, returns, foreign-call core (`invoke_foreign_kinded`), native_abi (libffi), foreign_marshal, jit_abi |
| `executor/state_builtins/` | 2,125 | `std::core::state` module bodies (introspection, capture, serialize) |
| `executor/exceptions/` | 1,990 | SetupTry/Throw/handler unwind, Result/Option opcode ops, `?` propagation |
| `executor/tests/` | 18,975 | unit/integration tests incl. drop_deep_tests.rs (3,497), typed_array_ops, try_operator, auto_drop |
| `executor/arithmetic/`, `comparison/`, `logical/`, `loops/`, `async_ops/`, `stack_ops/`, `additional/`, `jit_ops/`, `utils/` | ~5,200 | typed opcode handler families |
| `feedback.rs` (root) | 689 | FeedbackVector, IC state machine (Uninitialized→Mono→Poly→Mega) |
| `tier.rs` (root) | 1,264 | TierManager: call counters, T1@100/T2@10k thresholds, OSR counters/blacklist, deopt tables, shape invalidation |
| `resource_limits.rs` (root) | 184 | ResourceLimits/ResourceUsage (instruction, memory, wall-time, output caps) |

### 1.2 Key types

- **`VirtualMachine`** (`executor/mod.rs:275-642`): owns `stack: Vec<u64>` + parallel
  `kinds: Vec<NativeKind>` (ADR-006 §2.7.7), `module_bindings: Vec<u64>` + parallel
  `module_binding_kinds` (§2.7.8), `call_stack: Vec<CallFrame>`, exception handlers,
  tier manager, feedback vectors, megamorphic cache, foreign-fn handles, snapshot store
  hooks, task scheduler, `granted_permissions`/`scope_constraints` (WF-1D security).
  Fifty-plus fields; the struct is the god object of the crate by design.
- **`CallFrame`** (`executor/mod.rs:204-257`): return_ip, base_pointer, locals_count,
  `upvalues: Option<Vec<u64>>` (raw-bits indirection table for OwnedMutable/Shared
  capture cells), `closure_heap_bits: Option<u64>` + lockstep
  `closure_heap_kind: Option<NativeKind>` (the closure-self keep-alive share).
- **`ExecutionResult`** (`executor/mod.rs:131-141`): `Completed(KindedSlot)` |
  `Suspended { future_id, resume_ip }`.
- **`MethodFnV2`** (`objects/method_registry.rs:48-52`):
  `fn(&mut VirtualMachine, &[KindedSlot], Option<&mut ExecutionContext>) -> Result<KindedSlot, VMError>`
  — the §2.7.10/Q11 dispatch ABI, exactly as ADR-006 specifies.
- **`FeedbackVector`** (`feedback.rs:102-106`): per-function `HashMap<ip, FeedbackSlot>`
  with Call/Property/Arithmetic/Method slot variants.
- **`TierManager`** (`tier.rs:135-166`): per-function `FunctionTierState`, mpsc channel
  pair to the background JIT thread, `native_code_table`, `osr_table`, `osr_blacklist`,
  `deopt_tables`, `DeoptTracker` for shape-guard invalidation.

### 1.3 Data flow

1. **Entry**: `execute()` / `execute_with_suspend()` (`dispatch.rs:33/88`) installs the
   ambient `ShapeTableScope`, then selects the fast loop
   (`execute_fast_with_exceptions`, `dispatch.rs:346`) when no debugger/tracing.
2. **Dispatch**: `execute_instruction` (`dispatch.rs:655-1175`) is a single
   category-level match over the 420-opcode enum (`bytecode/opcode_defs.rs`), forwarding
   to `exec_*` family handlers. Typed opcodes (AddInt, EqString, LoadLocalF64, ...) read
   raw slot bits with zero runtime type dispatch (`arithmetic/mod.rs:130-165` ignores
   the popped kinds entirely — the compiler proof is trusted).
3. **Calls**: `op_call` → `call_function_from_stack` (in-place frame over pushed args,
   zero copies, `call_convention.rs:1466-1525`); `CallValue`/`CallClosure`/
   `CallFunctionIndirect` → `dispatch_call_value_immediate` (`control_flow/mod.rs:426`)
   → `call_value_immediate_nb` (§2.7.11 kinded value-call ABI) → per-callee-kind frame
   setup → `execute_until_call_depth` drives the callee to completion.
4. **Method calls**: `CallMethod` → `op_call_method` (`objects/mod.rs:385`) → trait-object
   reroute check → pop receiver+args as `KindedSlot`s → `dispatch_method_kinded` →
   UFCS-first for TypedObject receivers, then PHF registry per receiver kind
   (`resolve_method_handler`, `objects/mod.rs:663`).
5. **Returns**: `return_value_inner` (`control_flow/mod.rs:1273`) truncates the frame's
   stack window (releasing shares via the parallel kind track), releases the closure
   keep-alive via `drop_with_kind`, pushes the return value kinded.
6. **Periodic work** (every 1024 instructions): interrupt check, tier promotion polling,
   GC safepoint (`gc::maybe_collect`, threshold 256 candidates), resource-limit tick +
   alloc-budget breach drain (every instruction when sandboxed).
7. **Suspension**: `VMError::Suspended` with `SNAPSHOT_FUTURE_ID` is consumed in-loop
   (capture → persist → push `Result<Snapshot,_>` marker → continue); real future ids
   propagate to the host.

### 1.4 Entry points (host-visible)

`execute` / `execute_raw` / `execute_with_suspend` (dispatch.rs), `execute_function_by_name`
/ `_by_id` / `_by_id_at_host_boundary` / `execute_closure` / `_at_host_boundary` /
`execute_function_fast` / `execute_function_with_named_args` / `call_value_immediate_nb` /
`jit_trampoline_call_closure` / `jit_trampoline_call_method` (call_convention.rs),
`resume` (todo!()), `from_snapshot` + `snapshot` (snapshot.rs), `capture_vm_state`
(vm_state_snapshot.rs), `set_transport_provider` / QUIC config (mod.rs:1129-1158).

---

## 2. Feature completeness

Legend: **WORKS** = verified end-to-end with a run transcript; **CODE EXISTS** = read but
not independently executed; **PARTIAL** / **STUBBED** / **DEAD** as stated.

### 2.1 Core execution — WORKS

Verified end-to-end under `--mode vm` (t1, t23, t24):

```
$ shape run --mode vm t1_closure.shape       # closures as values, loop accumulation
5
20
$ shape run --mode vm t23_break.shape        # loop { break 42 }, for+break, while false
42
10
done
$ shape run --mode vm t24_match.shape        # enum unit/tuple/struct payloads + match
3.14159
6.0
0.0
```

Checked integer arithmetic per the 2026-06-01 numeric ruling — overflow is a structured
runtime error, not a wrap (t22):

```
$ shape run --mode vm t22_overflow.shape
Error: Runtime error: integer addition overflow: result of 9223372036854775807 and 1
exceeds the int (i64) range; widen explicitly with `as number` or `as bigint` (line 2)
```

Handler: `binop_int_checked` via `i64::checked_add/sub/mul` (`arithmetic/mod.rs:150-152`);
`DivInt` uses `wrapping_div` for the single `i64::MIN / -1` case per documented ruling
(`arithmetic/mod.rs:161-164`). Division by zero → clean `VMError::DivisionByZero` (t3).

### 2.2 Method dispatch — WORKS (with dead registries)

HOF array methods, string methods, HashMap (t4b):

```
$ shape run --mode vm t4b_methods.shape
[2, 4, 6, 8, 10]
[3, 4, 5]
15
["HELLO", "WORLD"]
None          <-- see finding §9.6: m.set on immutable `let` silently lost
```

Iterator lazy pipeline + terminal materialization WORKS end-to-end (t7) despite 20
`#[ignore]`d host-tier tests claiming "Phase-2c surface" (the ignores gate the *host-tier
eval/marshal API* tests, not the VM path — see §7.3):

```
$ shape run --mode vm t7_iter1.shape
[2, 4, 6]
```

DEAD registries: `RANGE_METHODS` (10 entries, `method_registry.rs:1062-1073`) is
unreachable — the compiler's method-name table rejects every entry (t8/t9, §9.5).
`INT_ARRAY_METHODS` (`method_registry.rs:751-772`) and `INDEXED_TABLE_METHODS` (`:703-707`)
are `#[allow(dead_code)]`-annotated — registered but not routed by any receiver
classification arm.

### 2.3 Frame protocol & value-call ABI — WORKS

Deep recursion under the interpreter hits the `max_call_depth` guard
(`call_convention.rs:93-98`) and surfaces cleanly (t2):

```
$ shape run --mode vm t2_recursion.shape
Error: Runtime error: Stack overflow (line 1)
```

(Default `--mode jit` crashes the host thread instead — §9.7.) Errors inside closures
invoked through the value-call ABI unwind gracefully with correct source location (t10):

```
$ shape run --mode vm t10_err_in_closure.shape
10
5
Error: Runtime error: Division by zero (line 3)
```

This exercises the 2026-06-18 `unwind_call_frames_to` fix (`call_convention.rs:994-1020`,
invoked at `:1159-1162`) — but only the closure arm has it (§9.4).

### 2.4 Snapshot / resume (execution side) — PARTIAL

`snapshot()` in a plain function + CLI `--resume` works end-to-end (t14):

```
$ shape run --mode vm t14_snap_toplevel.shape
HASH:fadecb3723ad0348...44259d8b
$ shape run --mode vm --resume fadecb3723ad0348...44259d8b
Resuming from snapshot: fadecb3723ad0348...
RESUMED:105
```

But: (a) `snapshot()` with any live closure value reachable is barrier-refused (t15:
`ERR:Barrier("cannot checkpoint here: a live closure value is reachable and this build
cannot yet save closures...")` — honest surface-and-stop, and it currently masks the
restore-side carrier bug §9.1); (b) `snapshot()` lexically inside a closure body doesn't
even compile a closure — `op_make_closure` rejects the capture classification (t12:
"capture 1 is stamped a heap carrier (Ptr(NativeView)) but the captured value arrived
with scalar kind Bool", `control_flow/mod.rs:737-753`); (c) the *in-VM* resume entry
points are stubs: `VirtualMachine::resume()` is `todo!()` (`call_convention.rs:369-379`)
and `apply_pending_resume` carries 5 `NotImplemented(PHASE_2C_SNAPSHOT_SURFACE)` returns
(`resume.rs:129-151`) — the working CLI path goes through `from_snapshot` instead, a
second restore implementation (split-brain risk, §5.4). Resume-with-edited-source is
correctly refused with a clear message (t14 transcript).

### 2.5 Resource limits — WORKS

All four caps are wired through both the CLI and all four dispatch loops
(`dispatch.rs:175-190, 381-396, 537-552, 597-612`); verified (t18):

```
$ shape run --mode vm --max-instructions 100000 t18_spin.shape
Error: Runtime error: Instruction limit exceeded: 100000 >= 100000
$ shape run --mode vm --max-time-ms 300 t18_spin.shape
Error: Runtime error: Wall time limit exceeded: 300.01234ms >= 300ms
```

Memory ceiling goes through `shape_value::v2::alloc_budget` (BudgetGuard install at
`execution.rs:862`, breach drained each tick) rather than
`ResourceUsage::record_allocation` — the latter is dead code (only caller: none;
`resource_limits.rs:138`). Output cap is live via `record_output`
(`vm_impl/builtins.rs:1707`). The WF-1D fix comments confirm the fast path previously
skipped enforcement; all loops now tick.

### 2.6 Feedback vectors / IC — PARTIAL (scaffolding-heavy)

Live: call-site feedback (`control_flow/mod.rs:299-304` records into
`FeedbackSlot::Call`), property feedback + megamorphic cache
(`typed_object_ops.rs:593,918`; `ic_fast_paths.rs:90-154`), and TierManager promotion
reads `monomorphic_ratio()` for T2 requests. DEAD/scaffolding: the whole method IC,
arithmetic IC, dyn-method IC, and closure-call IC fast paths — `ic_fast_paths.rs` is
`#![allow(dead_code)]` module-wide with an "Intentional-future" header (`:10-15`);
`record_arithmetic` has zero live call sites ("no live recordings exist", `:167-174`);
`method_ic_check`'s `transmute`-based handler cache is never consulted by
`dispatch_method_kinded`. The IC state machine itself (`feedback.rs`) is complete,
correct, and well-tested (11 tests).

### 2.7 Tier manager / OSR — CODE EXISTS (mechanics sound)

`record_call` (`tier.rs:214-255`) implements the T1@100/T2@10k thresholds exactly as
CLAUDE.md documents, attaches feedback snapshots on T2 requests, and `poll_completions`
(`tier.rs:300-349`) installs native code/OSR entries/deopt tables and drains shape
transitions for invalidation. OSR back-edge counting is triggered from both `op_jump`
backward-edges (`control_flow/mod.rs:179-190`) and LoopStart. Blacklisting prevents
recompile storms (`tier.rs:429-437`). The empirical default-mode caveat: whole-program
JIT falls back to the interpreter for any program that calls imported stdlib functions
(loud `[jit-fallback]` diagnostic naming the W17-marshal SURFACE — observed on t1), so
the interpreter is the de-facto execution engine for most real programs today.

### 2.8 Foreign-call core — CODE EXISTS (shared-core design verified by reading)

`invoke_foreign_kinded` (`control_flow/mod.rs:891-1076`) is the single shared
implementation for both tiers: permission phase-1 check → link-now (native: scope check
BEFORE `dlopen`; dynamic: extension vtable compile) → dispatch bracketed by
`foreign_reentry_depth` for the snapshot barrier, with host-side `catch_unwind`.
Deterministic-mode call-time backstop refuses foreign calls even when `Ffi` is granted
(`:1109-1115`). Not exercised empirically here (polyglot is vertical 07's territory).

### 2.9 Async ops / task scheduler — PARTIAL

`exec_async_op` handles Yield/Await/SpawnTask/Join*/AsyncScope*; `resolve_spawned_task`
(`call_convention.rs:477-636`) synchronously drives spawned callables with cached-result
share discipline. Suspension crossing a value-call frame boundary is explicitly
out-of-scope Phase-2c (`:466-476`) — a mid-callee suspension propagates as an error and
the task stays Pending. Non-future suspensions (NextBar/Timer/AnyEvent) silently drain
all async scopes and continue (`dispatch.rs:987-998`) — a questionable silent-cancel.

### 2.10 State introspection (`std::core::state`) — PARTIAL

Registration is complete (`state_builtins/core.rs:328-364`: caller/args/locals/snapshot/
resume/resume_frame/serialize/...). The backing `capture_vm_state` snapshot threads kinds
correctly for stack/module-binding slots but: frame `local_ip` is hardcoded 0 and
per-frame `args` empty (documented follow-ups, `vm_state_snapshot.rs:122-131`), and the
closure-upvalue recovery is built on the wrong carrier shape (§9.1).

---

## 3. Code quality

### 3.1 Idiom & documentation

The executor's distinguishing quality is **contract-first documentation**: nearly every
ownership transfer is annotated with who owns which strong-count share and which
teardown releases it (e.g. the 40-line ownership essay on
`call_closure_with_nb_args_keepalive`, `call_convention.rs:749-780`; the share-accounting
history on `clone_slot_kinded`, `vm_state_snapshot.rs:306-335`). SAFETY comments cite the
construction-side invariant they rely on. This is far above typical VM code. The cost is
enormous comment mass (many files are >50% comments) and — critically — comments that
assert invariants the code elsewhere violates (the `vm_state_snapshot.rs:149-155` SAFETY
comment asserts `closure_heap_bits` is "`OwnedClosureBlock::into_raw(...)`", which is
false for the live producer — the comment made the bug look verified; §9.1).

### 3.2 Unsafe usage

1,243 `unsafe` occurrences across the executor (grep count incl. comments; the top
files: `v2_handlers/v2_array_detect.rs` 339, `objects/hashmap_methods.rs` 143,
`variables/mod.rs` 76, `v2_handlers/array.rs` 49, `control_flow/native_abi.rs` 39).
Almost all fall into 4 audited patterns: (a) `Arc::increment/decrement_strong_count`
keyed on the parallel kind track; (b) v2-raw `HeapHeader` retain/release; (c) typed
pointer reads at compile-proven offsets; (d) libffi marshalling. The patterns are sound
*given* the construction-side kind contract — which is precisely why the one place the
contract is violated (finding #1) is dangerous: the entire unsafe economy rests on
"kind labels never lie about carrier shape".

Unjustified/fragile instances worth calling out:

- `vm_state_snapshot.rs:156-165` + `snapshot.rs:1025-1043`: unsafe reads justified by a
  SAFETY comment that mis-states the producer contract (finding #1).
- `dispatch.rs:234-235/429-430`: `store as *const SnapshotStore` + `unsafe { &*store_ptr }`
  to dodge a borrow conflict in the time-travel capture path — a raw-pointer aliasing
  workaround where a restructure would do.
- `ic_fast_paths.rs:50`: `transmute(entry.handler_ptr)` reconstructing a fn pointer from
  a `usize` stored in feedback (currently dead code, but a booby trap if generation
  invalidation is ever skipped).

### 3.3 Error handling

Consistent `Result<_, VMError>`; `#[cold]`/`#[inline(never)]` on slow paths
(`push_kinded_slow`, `vm_impl/stack.rs:998-1000`). Error enrichment attaches file/line +
source excerpt (`dispatch.rs:1181-1251`) — verified working in t10/t22 transcripts.
Two deviations: (a) `VMError::NotImplemented` doubles as both "reserved opcode" and
"surface-and-stop marker", so user-visible messages leak internal ADR jargon
("W17-snapshot-resume residual surface — ..."); (b) `todo!()` panics remain reachable
from user input in printing.rs (§9.2) — panics are not an error-handling channel this
codebase is otherwise disciplined about.

### 3.4 Complexity hotspots

- `execute_instruction` (`dispatch.rs:655-1175`): 520-line match — flat and readable,
  but the variables arm alone enumerates ~110 opcodes.
- `v2_array_detect.rs` (5,355 LOC): per-element-type monomorphized read/write/sum/avg/
  min/max/push/pop over 14 element types — the single largest handler file.
- `variables/mod.rs` (4,240 LOC): 3 opcode families × 11 widths, each handler 15-20
  near-identical lines (§4.2).
- `op_make_closure` (`control_flow/mod.rs:484-829`): 345 lines, 3-level nested capture
  classification with two distinct surface-and-stop guards and an intentional-leak
  error path (documented at `:672-682` — leak-over-null-deref on fatal exit).
- `VirtualMachine::Drop` (`executor/mod.rs:703-880`): 177 lines mixing shared-cell Arc
  reclamation, GC candidate buffering, kinded teardown, and lockstep-drift defense.

### 3.5 Dead code in-territory

- `ResourceUsage::record_allocation` (`resource_limits.rs:138-149`) — zero callers.
- `ic_fast_paths.rs` — module-wide `allow(dead_code)`; only property-IC half is live.
- `INT_ARRAY_METHODS`, `INDEXED_TABLE_METHODS`, `MUT_SELF_TYPED_ARRAY_METHODS` PHF maps —
  `#[allow(dead_code)]` (`method_registry.rs:703,751,135`).
- `RANGE_METHODS` — live-registered but unreachable through the compiler (§9.5).
- Polymorphic `op_load_owned_mutable_capture` / `op_store_owned_mutable_capture`
  (`variables/mod.rs:370-396`) — deliberate SURFACE markers awaiting bytecode-level
  removal.
- `module_binding_take_kinded`, `read_owned_kinded`, `stack_peek_kinded` carry
  `#[allow(dead_code)]` — kinded-API surface kept for symmetry.

### 3.6 Naming

Post-bulldozer naming is consistent (`*_kinded`, `*_v2`, `handle_*_v2`, `op_*`). Residual
stale vocabulary in *comments only*: `CallFrame.closure_heap_bits` doc says "optional
owning `ValueWord` bits" (`executor/mod.rs:232`), `shared_module_bindings` doc mentions
`Arc<parking_lot::Mutex<ValueWord>>` (`executor/mod.rs:337-339`), `module_fn_table` doc
references `ValueWord::ModuleFunction(id)` (`executor/mod.rs:406`). All three describe
deleted types as if live — pure comment rot, but in a codebase whose safety story leans
on doc-contracts, rot is a hazard (see 3.1).

---

## 4. Duplication & DRY violations

### 4.1 Four dispatch loops with copy-pasted preambles — `dispatch.rs`

`execute_with_suspend` (:112-342), `execute_fast_with_exceptions` (:346-510),
`execute_fast` (:516-579), and `execute_until_call_depth` (:581-652) each replicate the
interrupt check, the resource-limit tick + `alloc_budget::take_breach()` drain (4
verbatim 14-line copies at :175-190, :381-396, :537-552, :597-612), the tier-poll block,
the time-travel capture block (2 copies incl. the 16-opcode `is_call_or_return` match
duplicated verbatim at :213-231 and :407-426), the SNAPSHOT_FUTURE_ID consumer (3
copies), and the top-of-stack/Null-sentinel epilogue (3 copies at :334-341, :502-509,
:571-578). **Divergence risk is real and has already materialized once**: the WF-1D fix
comment (:376-380) records that the fast path *previously skipped* resource enforcement
that the slow path had. Any future per-tick concern must be added in four places.
`execute_fast` additionally lacks the exception-handler dispatch the other two
program-level loops have — intentional (docstring :512-514) but easy to misuse.

### 4.2 Eleven byte-identical typed return handlers — `control_flow/mod.rs:1342-1400`

`op_return_value_i64/u64/f64/i32/u32/i16/u16/i8/u8/bool/ptr` all have the same two-line
body (`pop_kinded` → `return_value_inner(bits, src_kind)`); the opcode suffix is
deliberately ignored ("kind from producer" rule, :1324-1340). Eleven functions where one
shared handler (or a macro) suffices; the doc comment itself concedes the suffix is a
JIT-side static annotation with no VM semantics.

### 4.3 Per-width capture/local opcode families — `variables/mod.rs`

12 `op_load_owned_mutable_capture_<K>` + 12 stores + 12 shared-capture loads/stores + 11
`LoadLocal<K>`/`StoreLocal<K>`/`LoadModuleBinding<K>`/`StoreModuleBinding<K>` handlers,
each differing only in the cast and `NativeKind` constant (e.g. :398-417 vs :419-435 vs
:437-453). ~1,800 LOC that a `macro_rules!` table would compress 10x, with the benefit
that a fix (like the null-pointer guard) provably lands in every arm. Today each arm
hand-repeats the guard.

### 4.4 `clone_with_kind`/`drop_with_kind` vs `KindedSlot::Clone/Drop` — 4-table lockstep

The retain and release dispatch tables exist twice each: `vm_impl/stack.rs:229-619`
(clone) / `:627-978` (drop) and the mirrored impls in
`crates/shape-value/src/kinded_slot.rs`. The header comment (:44-49) acknowledges "The
dispatch tables MUST stay in lockstep — divergence is a refcount bug", and history shows
exactly that failure mode (the `HeapKind::TypedArray` arm was retired then re-instated
after live slots were found still carrying it — r5c-2-β-δ-(α) comment at :264-282).
`verify-merge.sh` reportedly checks 4-table lockstep, which mitigates but does not
remove the structural duplication.

### 4.5 Receiver classification double-arm for `Ptr(TypedArray)` — `objects/mod.rs`

`resolve_method_handler` matches `Ptr(HeapKind::TypedArray)` in the scalar-handler match
(:736-746, elem-typed registry → ARRAY_METHODS fallback) and again in the heap match
(:813-823, ARRAY_METHODS only). The second arm is reachable only when the first returned
None (unknown method) and re-queries the same PHF — harmless today, but two arms for one
kind invites drift.

### 4.6 Two snapshot-side closure-upvalue walkers

`snapshot_frame_upvalues` (`vm_state_snapshot.rs:143-174`) and
`snapshot_frame_upvalues_serializable` (`snapshot.rs:1003-1078`) implement the same
recover-block→walk-captures loop against the same (wrong, §9.1) carrier assumption, one
producing `KindedSlot`s, one producing `SerializableVMValue`s. When the carrier bug is
fixed, both must change — and only one of them has the FUTURE_SNAPSHOT_BARRIER logic.

---

## 5. Split-brain analysis

### 5.1 `closure_heap_bits` carrier shape (producer/consumer split) — CRITICAL

The single worst split-brain in the territory; full analysis in §9.1. Summary matrix:

| Site | Role | Assumed carrier for `Some(bits)` |
|------|------|----------------------------------|
| `call_value_immediate_nb` (`call_convention.rs:1126-1133`) | producer (live calls) | `Arc::into_raw(Arc<HeapValue::ClosureRaw>)` |
| `resolve_spawned_task` (`call_convention.rs:565-571`) | producer (spawned tasks) | `Arc<HeapValue>` (same) |
| `restore_call_stack` (`snapshot.rs:505-529`) | producer (resume) | **raw `TypedClosureHeader` block ptr** |
| `op_return`/`return_value_inner` (`control_flow/mod.rs:1236-1250, 1298-1312`) | consumer (teardown) | `Arc<HeapValue>` (via `drop_with_kind` → `Arc::decrement_strong_count::<HeapValue>`, `vm_impl/stack.rs:892-894`) |
| `unwind_call_frames_to` (`call_convention.rs:1003-1018`) | consumer (error unwind) | `Arc<HeapValue>` |
| `snapshot_frame_upvalues_serializable` (`snapshot.rs:1025-1043`) | consumer (capture) | **raw block ptr** (`typed_closure_function_id(ptr)` reads u32 at offset 8; `retain_typed_closure(ptr)` bumps refcount at offset 0) |
| `try_borrow_closure_block` (`vm_state_snapshot.rs:269-300`) | consumer (state introspection) | **raw block ptr** |

Both carrier families share the *same* kind label `NativeKind::Ptr(HeapKind::Closure)`,
so nothing in the type system or kind track can distinguish them. This is a live
instance of the CLAUDE.md §Forbidden "parallel-implementation across producer/consumer
carrier-shape boundaries" attractor — two carriers for one field, selected by which code
path produced the frame, with no classification rule.

### 5.2 Compiler method-name tables vs VM PHF registries

The VM's PHF registries and the compiler's receiver-type method tables are maintained
separately and have drifted: `RANGE_METHODS` has 10 runtime entries
(`method_registry.rs:1062-1073`) but the compiler rejects `r.toArray()` / `r.contains(3)`
at type-check ("Method 'toArray' not found on type 'Range'", t8/t9 transcripts §9.5) —
so runtime capability exists with no reachable syntax. The reverse direction also exists:
`m.set(...)` on an immutable binding type-checks and dispatches, but the compiler's
mut-self write-back gate (driven by `MUT_SELF_HASHMAP_METHODS`,
`method_registry.rs:85-89`) silently skips the write-back (§9.6). There is no
cross-check test tying the two tables together (contrast: the JIT return-kind
cross-check deliberately iterates the PHF maps — `executor/mod.rs:62-69` — proving the
pattern is available).

### 5.3 VM-vs-JIT semantic surface

Handled better than most: the foreign-call core is deliberately single-implementation
(`invoke_foreign_kinded`, "Divergence between vm and jit modes is impossible for
foreign-call semantics by construction", `control_flow/mod.rs:867-870`), and JIT method
dispatch trampolines route back into `dispatch_method_kinded` (`call_convention.rs:1429`)
so the PHF is consulted once. Two asymmetries remain: (a) recursion depth — interpreter
enforces `max_call_depth`, JIT native frames don't (t2, §9.7); (b) whole-program JIT
deopts to the interpreter for any stdlib-calling program (`[jit-fallback]` observed on
t1) — VM==JIT semantics preserved by *not running* the JIT, which is honest but makes
`--mode jit` largely aspirational for real programs.

### 5.4 Two snapshot-restore implementations

The CLI `--resume` path goes through `from_snapshot` / `restore_call_stack`
(`snapshot.rs:465-584`, works — t14), while the in-VM `state.resume()` path
(`VirtualMachine::resume`, `apply_pending_resume`) is `todo!()`/`NotImplemented`
(`call_convention.rs:374`, `resume.rs:129-151`). Two restore drivers, one finished, one
stub; the finished one embeds the wrong-carrier closure frames (§9.1) that the stub
will presumably inherit when built.

### 5.5 Doc-comment vs code drift

Three field-doc comments describe deleted `ValueWord` machinery as live
(`executor/mod.rs:232, 337-339, 406`); the `vm_state_snapshot.rs:149-155` SAFETY comment
asserts the wrong producer contract; the `call_value_immediate_nb` comment calls the
closure slot bits a "`Box<HeapValue>` pointer" (`call_convention.rs:1087`) when every
other site (correctly) says `Arc<HeapValue>`. In a contract-comment-driven unsafe
economy, each of these is a seed for the next wrong "fix".

### 5.6 `upvalues: Vec<u64>` raw table vs the kinded capture track

`CallFrame.upvalues` is a kind-less `Vec<u64>` indirection table
(`call_convention.rs:852-859`) feeding the `Load/StoreOwnedMutableCapture*` opcodes,
while the same captures' kinds live in the `ClosureLayout` side-table. The design is
coherent (raw bits are cell *pointers*, kinds live on the layout) but it means capture
state exists in two parallel structures with the layout as the only kind source; the
field's own doc admits the consumers were "pre-existing Wave-α-broken and migrate
together" (`executor/mod.rs:213-227`). The typed per-width opcodes work today; the
polymorphic shells are surfaced stubs (`variables/mod.rs:370-396`).

---

## 6. ADR & spec conformance

Verdict format: **CONFORMS** / **VIOLATION** / **PARTIAL**, with evidence.

### 6.1 ADR-006 §2.7.7 / Q9 — stack parallel-kind track

**CONFORMS.** `stack: Vec<u64>` + `kinds: Vec<NativeKind>` in lockstep
(`executor/mod.rs:291-304`); `debug_assert_kinds_in_sync` at API boundaries
(`vm_impl/stack.rs:986-992`); push/pop/write/take/truncate all move both tracks
(`vm_impl/stack.rs:1069-1335`). No `Vec<KindedSlot>` stack, no 16-byte slots, no packed
tag bits, no `Option<NativeKind>`/`Unknown` placeholders (grep-verified). Dead-space
sentinel is `(0, Bool)` per convention. `clone_with_kind`/`drop_with_kind` dispatch on
kind with zero tag decode and no `is_heap()` probe; heap arms map 1:1 to typed
`Arc<T>`/`HeapHeader` retain/release (`vm_impl/stack.rs:229-978`). Deleted-shim names
(`push_raw_u64` etc.) exist only in comments describing their deletion (§grep in
audit prep; `objects/mod.rs:10` is a doc reference). The `NativeScalar` arm is a
`debug_assert!(false)` surface, not a Bool-default (`:566-572, 923-929`) — conforming
surface-and-stop. **Caveat**: in release builds that debug_assert compiles out and a
non-zero NativeScalar-kinded pointer would be silently leaked (no-op) rather than
surfaced — acceptable but worth noting.

### 6.2 ADR-006 §2.7.8 / Q10 — cell-storage parallel kinds

**CONFORMS with one bounded exception.** Module bindings carry the parallel
`module_binding_kinds` with lockstep pad/write/read/take accessors
(`executor/mod.rs:896-1126`) and kinded teardown in `Drop` (`:829-848`), including the
documented "leak rather than misdispatch" disposition for any lockstep drift plus a
debug assert. Closure cells: `OwnedClosureBlock::read_capture_kinded` sources kinds from
the layout track at frame setup (`call_convention.rs:858, 914`), with the crucial
subtlety handled correctly — OwnedMutable/Shared captures are *excluded* from the
immutable-capture clone loop because their `read_capture_kinded` kind classifies the
cell interior, not the cell pointer (`:882-917`). `CallFrame.closure_heap_kind` is the
lockstep companion, enforced by debug_asserts at every construction/teardown site
(`:790-795`, `control_flow/mod.rs:1241-1249, 1303-1311`). The **exception**:
`CallFrame.upvalues` remains a kind-less `Vec<u64>` (§5.6) — documented as a §2.7.4
deferral on the field itself, so PARTIAL by the ADR's own accounting rather than a
violation. Sentinel-kind uses of `Bool` (pad, dead slots, omitted args) are the
documented no-op-on-drop convention, not §2.7.7-#9 Bool-default fallbacks — each site
carries the distinction in comments and none masks a real kind-source gap that I could
find.

### 6.3 ADR-006 §2.7.10 / Q11 — MethodFnV2 dispatch ABI

**CONFORMS.** `MethodFnV2 = fn(&mut VirtualMachine, &[KindedSlot], Option<&mut
ExecutionContext>) -> Result<KindedSlot, VMError>` (`method_registry.rs:48-52`) —
exactly the ADR signature. Receiver is `args[0]`; kinds come from `pop_kinded` off the
stack parallel track (`objects/mod.rs:443-454`) — no fabrication; heap dispatch in
handlers goes through `slot.as_heap_value()` + `HeapValue` match, with the two documented
pure-discriminator exceptions (Temporal at `objects/mod.rs:835-879` and FilterExpr)
borrowing the typed Arc directly per the 5-arm receiver-recovery soundness rule. No
parallel `&[NativeKind]` side-slice, no `&mut [KindedSlot]` by-move, no `(u64,
NativeKind)` result. Forbidden transitional names (`MethodFn`, `MethodFnLegacy`,
`dispatch_method_handler_raw`, `call_handler_with_u64_slice`) appear only in
deletion-documenting comments.

### 6.4 ADR-006 §2.7.11 / Q12 — value-call ABI

**CONFORMS structurally; PARTIAL behaviorally.** `call_value_immediate_nb(callee:
&KindedSlot, args: &[KindedSlot], ...)` classifies strictly on `callee.kind`
(`Ptr(Closure)` → block recovery via `as_heap_value()`+`ClosureRaw` match; `UInt64` →
function-id; `Ptr(ModuleFn)` → module-fn stub; everything else → RuntimeError, no
polymorphic fall-through) (`call_convention.rs:1022-1229`). Frame setup via
`read_capture_kinded`; `closure_heap_kind` preserved on the frame. The one `_raw`
survivor (`jit_trampoline_call_closure`, pair-slice at the §2.7.5 FFI boundary only) is
exactly the survivor the ADR names. Behavioral gap: error unwinding is implemented only
in the closure arm (§9.4), and suspension across the boundary is out-of-scope Phase-2c
(documented). No `call_value_legacy`/`call_value_raw_u64` (grep: 0 live).

### 6.5 ADR-005 §1 — single discriminator

**CONFORMS in dispatch; the split-brain of §5.1 is the one soundness exception.**
Heap-resident dispatch throughout the method/value-call shells goes
`ValueSlot::as_heap_value()` → `HeapValue` match. Pure-discriminator HeapKinds
(FilterExpr, SharedCell, Reference, Iterator, Range, Matrix, Temporal...) never call
`as_heap_value()` on their typed-Arc bits — each has its matching typed retain/release
arm in `clone_with_kind`/`drop_with_kind` and direct-borrow recovery where consumed.
I found no new sum type projecting 1:1 onto HeapKind. The `closure_heap_bits` dual
carrier (§5.1) is, functionally, a second undeclared discriminator — the exact drift
ADR-005 §Forbidden predicts.

### 6.6 ADR-006 §2.7.9 — FilterExpr typed-Arc label

**CONFORMS.** `clone_with_kind`/`drop_with_kind` FilterExpr arms retain/release
`Arc<FilterNode>` (`vm_impl/stack.rs:446-448, 817-819`); `heap_ptr_is_truthy`
enumerates it; no `as_heap_value()` on FilterExpr bits found in the executor.

### 6.7 ADR-006 §2.7.30 — Drop containment

**CONFORMS (by reading).** `drop_errors: Vec<String>` sink on the VM
(`executor/mod.rs:389-398`): errors in user `Drop::drop` bodies are contained, logged,
and remaining drops still run; host-queryable via `take_drop_errors`. Deep coverage
exists in `executor/tests/drop_deep_tests.rs` (3,497 LOC).

### 6.8 CLAUDE.md Forbidden Patterns — live-code sweep

**CLEAN.** Sentinel greps over `executor/` for `synthesize_value_word_from_raw`,
`last_program_return_kind`, `normalize_persisted_for_slot`, `SlotKind::Dynamic`,
`exec_*_dynamic_fallback`, `call_value_legacy`, `call_value_raw_u64`, `push_raw_u64`,
`vw_clone`/`vw_drop`, `MethodFnLegacy`, `tag_bits::is_tagged`: **every** hit is a
comment describing the deletion (verified by inspecting all matches). Generic opcodes
(`Add`/`Sub`/`Lt` without kind suffix) are absent from the dispatch match — bitwise
dynamic ops documented as deleted in c5 Phase B (`dispatch.rs:671-675`). The sentinel
test `executor/tests/no_dynamic.rs` exists. `ValueWord` survives only in stale doc
comments (§5.5) — recommend scrubbing, since CLAUDE.md's own enforcement is grep-based
and comment matches create noise that trains reviewers to ignore hits.

### 6.9 runtime-v2-spec — typed opcodes / zero-tag execution

**CONFORMS.** Typed arithmetic reads raw bits and ignores popped kinds entirely
(`arithmetic/mod.rs:150-170`) — the compile-time proof is trusted, there is no runtime
type check and no coercion emission. 420 opcodes, typed variants only. KindedSlot does
not leak into the VM↔JIT slot ABI: the JIT trampolines translate pair-slices at the
boundary in one direction (`call_convention.rs:1232-1443`).

---

## 7. Test coverage in-territory

### 7.1 Counts

- **1,576 `#[test]` functions** under `executor/` (18,975 LOC in `executor/tests/` plus
  substantial `#[cfg(test)]` modules inside source files — e.g. `vm_impl/stack.rs`
  carries 15 kinded-stack tests incl. 4 Miri-provenance probes and 2 GC-barrier tests).
- **37 tests** across `feedback.rs` (11), `tier.rs`, `resource_limits.rs`-adjacent
  (`executor/tests/resource_limit_enforcement.rs` exists specifically to pin the two
  historic enforcement defects — its header documents "Defect A — `--max-output-bytes`
  was inert").
- **61 `#[ignore]`** attributes in-territory.

### 7.2 Assertion quality

High where it matters most: refcount tests assert exact `Weak::strong_count` transitions
around push/read/pop/truncate (`vm_impl/stack.rs:1539-1569`); the GC-barrier test
asserts candidate-buffer contents and the stale-entry removal on rc→0
(`:1438-1471`); Miri-gated provenance tests exist for String/TypedObject/TypedArray
carriers through read/pop/overwrite/truncate (`:1575-1800`) — a rare and valuable
UB-proof surface, honestly scoped ("evidence for this sidecar path only, not a full VM
UB proof"). `drop_deep_tests.rs` (3,497 LOC) covers RAII ordering incl. escape-deferred
drops. Feedback state-machine tests cover all four slot types through
Mono→Poly→Mega including the no-new-entries-after-megamorphic property
(`feedback.rs:466-491`).

### 7.3 Ignored tests — do the reasons hold?

Mostly yes, with one important staleness nuance:

- 20 ignores in `executor/tests/iterator_ops.rs` claim "Phase-2c surface: iterator
  terminal materialization requires the host-tier eval/marshal API rebuild". The
  *end-to-end* iterator path works (t7 transcript: `xs.iter().map(...).collect()` →
  `[2, 4, 6]`), so these gate the deleted host-tier *test harness* API, not the feature.
  The ignore reasons are accurate about the API but a naive reader (or coverage
  dashboard) would wrongly conclude iterators are broken — and conversely, nothing
  currently *unit*-tests iterator terminals; coverage rides on shape-test integration.
- `executor/v2_handlers/integration_tests.rs:140,154` ignore `Array<int>.map/filter`
  citing the deleted `TypedArrayData` enum cascade — yet t4b shows `map`/`filter` on int
  arrays working end-to-end. Stale reason; the tests should be rewritten against the
  v2-raw carrier rather than kept ignored.
- `executor/tests/mod.rs:2052` ignores a v1 alias-preservation test pending a v2
  rewrite — reason holds.

### 7.4 Gaps

- **No test pins the `closure_heap_bits` carrier shape.** Neither producer family nor
  any consumer has a test asserting what the bits *are* — which is how the §9.1
  split-brain survived. A 20-line test (make a closure via `op_make_closure`, call it,
  snapshot `capture_vm_state`, assert upvalues non-empty) would have caught it.
- **No cross-check between compiler method tables and PHF registries** (would have
  caught dead RANGE_METHODS, §9.5). The JIT return-kind cross-check
  (`executor/mod.rs:62-69`) shows the pattern; it needs a compiler-side sibling.
- **Error-path share accounting is under-tested**: `unwind_call_frames_to` has no
  direct unit test; `handle_exception`'s frame truncate (§9.3) has none asserting
  keep-alive release.
- **`execute_until_call_depth` suspension edge**: the SNAPSHOT_FUTURE_ID in-loop
  consumer added for WF-2F (`dispatch.rs:620-639`) is only covered transitively by CLI
  e2e tests outside the crate.
- Arithmetic/method IC fast paths are dead code and correspondingly untested beyond the
  state machine itself.

---

## 8. Book/docs vs reality for this vertical

Sources: `shape-web/book/book-site/public/llms-full.txt` (rendered book),
CLAUDE.md, `crates/shape-vm/README.md`.

| Claim | Reality |
|-------|---------|
| CLAUDE.md: "Stack-based execution with typed 8-byte slots driven by per-slot NativeKind metadata (parallel Vec<u64> + Vec<NativeKind>)" | **TRUE** — implemented exactly (`executor/mod.rs:291-304`, `vm_impl/stack.rs`). |
| CLAUDE.md: "Tier 1 baseline @ 100 calls, Tier 2 optimizing @ 10k", OSR for hot loops | **TRUE** in code (`tier.rs:30-36`, OSR threshold 1000 @ `tier.rs:132`); *but* whole-program JIT deopts to the interpreter for any program calling imported stdlib fns (observed `[jit-fallback]` on t1), so tiering rarely engages end-to-end today. |
| CLAUDE.md: "Feedback-guided JIT: IC state machine ... drives speculative optimization" | **PARTIAL** — call + property feedback recorded and shipped with T2 requests; arithmetic/method/dyn IC recording is dead scaffolding (`ic_fast_paths.rs:15`). |
| CLAUDE.md: "Resource sandboxing: caps instruction count, memory, wall time, output" | **TRUE** — all four verified or wired (t18 transcripts; output cap via `vm_impl/builtins.rs:1707`; memory via alloc_budget). |
| Book: "`math.sqrt(3.0*3.0+4.0*4.0)` ... " (bare `math.` namespace, no import shown) | **FALSE as written** — `math` is undefined without `use std::core::math::sqrt`; bare `math.sqrt` and `use std::core::math` + `math.sqrt` both fail (t17/t19 transcripts). Book examples in the stdlib chapter don't run as printed. |
| Book: `import transport from "std:transport"` style imports | **FALSE** — parser rejects `import x from "..."` (t11b transcript: "unexpected string"). |
| Book: `state.locals()` "Get the current scope's local variables" | **PARTIAL** — registered and dispatchable, but frame `local_ip` is 0, per-frame args are empty (`vm_state_snapshot.rs:122-131`), and closure-frame upvalue introspection reads the wrong carrier (§9.1). |
| Book: snapshot/resume narrative ("execution continues here and returns Snapshot::Resumed") | **TRUE for closure-free frames** (t14 round-trip verified); refused with an honest barrier when closures are reachable (t15). |
| CLAUDE.md Known Constraints: "`just test-all` ... pre-existing #[ignore]'s stay ignored" | Consistent with the 61 in-territory ignores; but two of the ignore *reasons* are stale (§7.3). |
| `shape-vm/README.md` + `V2_*.md` audit files at crate root | Historical migration docs; largely accurate as history, not current-state docs. |

The book's execution-model claims are broadly honest about the VM; the *stdlib-usage
examples* are the part that doesn't survive contact (import forms, bare module
namespaces) — that is vertical 08/10 territory but directly affects what users can
execute on this VM.

---

## 9. Bugs & correctness risks found

### 9.1 P0 — `CallFrame.closure_heap_bits` three-way carrier split-brain

**The defect.** Two producer families write incompatible pointer families into the same
field under the same `NativeKind::Ptr(HeapKind::Closure)` label, and consumers are split
on which family they expect (full matrix in §5.1):

- Live-call producers store `Arc<HeapValue>` data-pointer bits:
  `call_value_immediate_nb` clones the callee share then installs
  `Some(callee.slot.raw())` (`call_convention.rs:1126-1133`); `op_make_closure` produced
  those bits as `Arc::into_raw(Arc::new(HeapValue::ClosureRaw(owned)))`
  (`control_flow/mod.rs:826-828`). The matching teardown release is
  `Arc::decrement_strong_count(bits as *const HeapValue)`
  (`vm_impl/stack.rs:892-894`). Consistent.
- Snapshot-restore produces a **raw `TypedClosureHeader` block pointer**:
  `restore_call_stack` allocates via `alloc_typed_closure` and stores
  `block.as_ptr() as u64` with kind `Ptr(Closure)` (`snapshot.rs:505-529`).
- The introspection/capture consumers assume the raw-block shape:
  `try_borrow_closure_block` does `typed_closure_function_id(bits as *mut u8)` (reads a
  u32 at offset 8 — see the layout doc `closure_raw.rs:20-28`: HeapHeader@0,
  function_id@8) then `retain_typed_closure(ptr)` (atomic increment at offset 0)
  (`vm_state_snapshot.rs:269-300`); `snapshot_frame_upvalues_serializable` does the same
  (`snapshot.rs:1025-1043`).

**Failure mode A (live today, silent + latent corruption).** `capture_vm_state` runs on
*every* `ModuleFnEntry::Typed` dispatch (`vm_impl/modules.rs:848`). If any closure frame
is live on the call stack at that moment, `snapshot_frame_upvalues` reads offset 8 of a
`HeapValue` enum (for `ClosureRaw{ptr, Arc<layout>}` that is the low half of an interior
pointer) as a "function id", then indexes `closure_function_layouts` with it. Usual
outcome: miss → `None` → upvalues silently absent from introspection (wrong data). If
the garbage id *hits* a registered layout, `retain_typed_closure` increments a
"refcount" inside the `HeapValue` allocation's first 8 bytes — corrupting the enum
discriminant/pointer — and the constructed `OwnedClosureBlock` then walks "captures" at
fabricated offsets. Empirical reachability of the path (not the collision): t21 runs a
closure → helper → `sqrt` module-fn chain successfully:

```
$ shape run --mode vm t21_math4.shape       # use std::core::math::sqrt
2.0
3.0                                          # closure frame live during module-fn dispatch
```

**Failure mode B (masked today, detonates when closure snapshots land).** A resumed
program whose snapshot contained a closure frame gets `closure_heap_bits = raw block
ptr` from `restore_call_stack`; when that frame returns, `return_value_inner` calls
`drop_with_kind(bits, Ptr(Closure))` → `Arc::decrement_strong_count` against a pointer
that was **never** an Arc — decrementing whatever word sits 16 bytes before the malloc'd
block. Heap corruption / arbitrary free. Currently unreachable only because the capture
side refuses closures (t15 barrier transcript: "cannot checkpoint here: a live closure
value is reachable and this build cannot yet save closures") — but `restore_call_stack`
happily *produces* the shape already, and closure snapshotting is a stated release goal
(W17 completion is release-blocking per project memory).

**Why it survived.** The SAFETY comment at the misreading site asserts the wrong
contract as if verified ("the bits are `OwnedClosureBlock::into_raw(...)`",
`vm_state_snapshot.rs:151-153`), and no test pins the carrier shape (§7.4).

**Fix direction.** One carrier, one classification: make `restore_call_stack` wrap its
rebuilt block in `Arc<HeapValue::ClosureRaw>` (matching the live producer + teardown),
and rewrite both upvalue walkers to recover the block via
`ValueSlot::from_raw(bits).as_heap_value()` → `ClosureRaw(block)` — the same ADR-005 §1
path `call_value_immediate_nb` uses. Delete `try_borrow_closure_block`.

### 9.2 P1 — `print(closure)` / `print` of NativeScalar panics the process

`printing.rs:430` and `:447` are `todo!()` arms reachable from safe user code. Empirical
(t25):

```
$ shape run --mode vm t25_print_closure.shape
thread 'main' (1352883) panicked at crates/shape-vm/src/executor/printing.rs:430:17:
not yet implemented: phase-2c — ... closure formatting needs kinded ClosureRaw read
```

A two-line program (`let f = |x: int| x + 1` / `print(f)`) aborts the VM host. Every
other unfinished surface in the executor returns `VMError::NotImplemented`; these two
should too (or better: format `[Closure fn=<id>]` — the function id is one
`as_heap_value()` match away).

### 9.3 P1 — `handle_exception` leaks closure keep-alive shares on unwind

`exceptions/mod.rs:144-151` releases every unwound *stack slot* via `drop_with_kind`,
then does `self.call_stack.truncate(handler.call_depth)` — dropping `CallFrame`s without
releasing their `closure_heap_bits` companions. Every closure frame between the throw
point and the handler leaks one `Arc<HeapValue::ClosureRaw>` share (the exact leak class
the 2026-06-18 `unwind_call_frames_to` fix addressed for the value-call path —
`call_convention.rs:1142-1158` documents "valgrind 40 bytes definitely lost on ANY
runtime error inside a forEach/map closure"). The fix exists 300 lines away; this site
was missed. Also unreleased on this path: nothing else (upvalues is a plain Vec; locals
were covered by the sp-loop since frame windows sit above `handler.stack_size`).

### 9.4 P1 — error-unwind asymmetry in the value-call ABI

Only the `Ptr(Closure)` arm of `call_value_immediate_nb` wraps its drive loop in the
unwind guard (`call_convention.rs:1159-1162`). The `UInt64` arm (`:1187`) and
`resolve_spawned_task` (`:622`) call `execute_until_call_depth(...)?` bare: a runtime
error inside a *function-value* call or a spawned task leaves every callee frame on
`call_stack` (with any nested closure frames' keep-alive shares unreleased) and the
callee's stack window live. If the host swallows the error and reuses the VM (REPL,
`execute_function_by_id` embedding), subsequent depth-relative logic
(`execute_until_call_depth` targets, `current_locals_base`) operates against phantom
frames. Same one-line fix as the closure arm.

### 9.5 P1 — dead `RANGE_METHODS`: runtime capability with no reachable syntax

`method_registry.rs:1062-1073` registers iter/toArray/contains/start/end/step/length/
size/len/isEmpty for Range receivers and `resolve_method_handler` routes
`HeapKind::Range` to it (`objects/mod.rs:923`). The compiler rejects them all (t8/t9):

```
$ shape run --mode vm t8_range.shape        # let r = 0..5; print(r.toArray())
error[SEMANTIC]: Method 'toArray' not found on type 'Range'
$ shape run --mode vm t9_range2.shape       # r.contains(3)
error[SEMANTIC]: Method 'contains' not found on type 'Range'
```

(Also note both diagnostics point at line 1, the binding, not the method-call line —
a location-attribution paper cut.) Root cause is compiler-side (its Range method table),
but the split-brain is registry-vs-checker (§5.2); either wire the checker to the PHF
name sets or generate both from one table.

### 9.6 P1 — silent lost mutation: mut-self methods on immutable `let` bindings

Empirical (t4b vs t6):

```
let m = HashMap()      →  m.set("a", 1); m.get("a")  ⇒  None      (silently lost)
let mut m = HashMap()  →  m.set("a", 1); m.get("a")  ⇒  1
```

`HashMap.set` is Arc-copy-on-write; the binding write-back is emitted only per the
compiler's mut-self gate (driven by `MUT_SELF_HASHMAP_METHODS`,
`method_registry.rs:85-89`), and for an immutable binding the write-back is skipped
*silently* — no compile error, no runtime error, the mutation just vanishes. In a
strict-typing language this must be a compile error ("cannot call mutating method `set`
on immutable binding `m`"). The registry sets that drive the gate live in this vertical;
the enforcement gap is compiler-side. Array `push` on `let` shows the same class.

### 9.7 P1 (cross-vertical, JIT) — default-mode recursion crashes the host

t2: `fn boom(n: int) -> int { boom(n + 1) }` under default `--mode jit` →
`thread 'main' has overflowed its stack` (process abort); under `--mode vm` → clean
`Error: Runtime error: Stack overflow` via `ensure_call_stack_capacity`
(`call_convention.rs:93-98`). The interpreter's guard is correct; the JIT tier emits
native recursion with no depth/guard-page handling. Filed here because the *default* CLI
mode exposes it and the interpreter proves the correct behavior.

### 9.8 P2 — non-future suspensions silently cancel all async scopes

`dispatch.rs:987-998`: NextBar/Timer/AnyEvent suspensions "cannot be resumed by the
host", so the handler pops *every* async scope and cancels all tracked tasks, then
continues execution as if nothing happened. Defensible as leak prevention, but it
converts a suspension into a silent mass-cancel with no diagnostic; a
`NotImplemented`-style surface would match the codebase's own discipline.

### 9.9 P2 — `capture_vm_state` cost on every module-fn dispatch

`vm_impl/modules.rs:848` unconditionally builds a full `VmStateSnapshot` — cloning (with
refcount bumps) every module binding, every frame's locals window, and walking closure
blocks — before *every* Typed module-function call, whether or not the body consults
`ctx.vm_state`. For binding-heavy programs each stdlib call pays O(bindings+locals).
Make it lazy (closure-capture a thunk) or gate it on the callee's declared
`needs_vm_state`.

### 9.10 P2 — `execute_function_by_id_at_host_boundary` IP restore gap

`call_convention.rs:170-187`: on frame-setup failure the saved IP is restored, but if
`execute_until_call_depth` errors mid-body the function returns with `self.ip` still
pointing at end-of-program (the host-boundary stamp), not the saved caller IP. For the
remote/async-isolated hosts that own these VMs the next action is usually teardown, but
a host that retries on error resumes from a poisoned IP. Symmetric issue in
`execute_closure_at_host_boundary` (`:239-260`).

### 9.11 P2 — release-build silence of `NativeScalar` retain/release surface

`clone_with_kind`/`drop_with_kind` NativeScalar arms are `debug_assert!(false)`
(`vm_impl/stack.rs:566-572, 923-929`): in release builds a mislabeled non-zero
NativeScalar pointer silently no-ops (leak) instead of surfacing. Consistent with
"leak rather than misdispatch" but unlike the module-binding tail case it is not
documented as a deliberate disposition at the site.

### 9.12 P2 — stale-comment hazards (see §5.5)

Not runtime bugs, but two of them (the `vm_state_snapshot.rs` SAFETY comment and the
`Box<HeapValue>`/`Arc<HeapValue>` confusion at `call_convention.rs:1087`) sit directly
on the §9.1 fault line and actively misled the code that was written against them.

---

## 10. What is done well

1. **The parallel-kind stack as a single load-bearing idea.** One invariant ("kind
   travels beside bits, and every retain/release dispatches on it") is enforced
   uniformly across stack, module bindings, closure cells, frames, and teardown — with
   debug asserts at every boundary (`vm_impl/stack.rs:986-992`,
   `executor/mod.rs:818-828`) and Miri provenance probes where integer-pointer
   round-trips would otherwise be unverifiable (`vm_impl/stack.rs:1575-1800`). This is a
   coherent, checkable memory-safety architecture, not folklore.

2. **Share-accounting archaeology in comments.** The cluster-1.5 / Round-13-T5 fixes
   left behind precise ownership essays at the exact sites that were wrong
   (`call_convention.rs:1091-1124`, `:892-944`; `vm_state_snapshot.rs:306-335`),
   including *why the old shape double-released and what empirical surface it produced*.
   Future maintainers can reconstruct the reasoning without the git log.

3. **`unwind_call_frames_to`** (`call_convention.rs:994-1020`): the error-unwind
   teardown mirrors the happy path exactly (truncate to bp + kinded keep-alive release),
   with the lockstep debug assert preserved. It just needs to be *called* from the two
   missing sites (§9.4) and `handle_exception` (§9.3).

4. **Checked integer semantics with actionable errors** (`arithmetic/mod.rs:139-165`):
   exact i64, structured overflow errors naming the widening escape hatches, and a
   documented deliberate choice on the single `MIN/-1` wrap case. The t22 error message
   is genuinely user-helpful.

5. **The shared foreign-call core** (`invoke_foreign_kinded`,
   `control_flow/mod.rs:881-1076`): one implementation for both tiers by construction,
   pinned check ordering (scope before `dlopen` so ELF constructors can't run
   pre-refusal), reentry-depth bracketing for the snapshot barrier, host-side
   `catch_unwind`. This is how you prevent VM/JIT semantic drift.

6. **Surface-and-stop discipline held (almost) everywhere.** 184 `NotImplemented`
   returns and 198 SURFACE markers instead of silent fallbacks; `op_make_closure`'s
   carrier-mismatch guards (`control_flow/mod.rs:683-753`) refuse to write a scalar into
   a heap-drop-masked slot and even document the deliberate one-shot leak on the fatal
   path rather than risk a null deref. The forbidden-pattern sweep coming back clean in
   live code (§6.8) after this many migration waves is a real institutional achievement.

7. **In-place frame windows** (`call_function_from_stack`,
   `call_convention.rs:1466-1525`): args pushed by the caller *become* the callee's
   locals with zero copies and zero refcount churn — the share "lives once" across the
   frame transition. Clean and fast.

8. **Resource-limit regression pinning**: after the WF-1D audit found the fast path
   skipped enforcement, the fix landed in all four loops *and* a dedicated regression
   test file documents the two defects by name
   (`executor/tests/resource_limit_enforcement.rs:6`). Verified live (t18).

9. **PHF method registry design** (`objects/method_registry.rs`): O(1) compile-time
   maps, kind-generic header handlers (one `handle_len_v2` across all element types
   instead of per-kind duplicates — the W16.2-J.1 deletion of two per-kind PHF maps is
   documented in place at `:774-787`), and the mut-self / tuple-return opt-in sets are
   explicit about semantics rather than encoding them in handler side effects.

10. **Honest barriers over wrong answers** in snapshot territory: the closure barrier
    (t15), the edited-source refusal (t14), and the foreign-frame barrier all refuse
    with instructive messages instead of producing corrupt snapshots.

---

## 11. What is done poorly / tech debt

1. **Comment-as-contract without verification.** The territory's safety model is
   "SAFETY comments cite the producer contract" — but nothing checks the citations.
   §9.1 exists because a SAFETY comment asserted a producer shape that was never true.
   Debt: contracts pinned only in prose need at least one test each (§7.4).

2. **Macro-less monomorphization.** ~1,800 LOC of hand-copied per-width handlers in
   `variables/mod.rs`, 5,355 LOC in `v2_array_detect.rs`, 11 identical return handlers —
   the cost is not aesthetics but *fix propagation*: every guard/bugfix must be
   hand-applied N times (and the TypedArray clone/drop-arm retirement/re-instatement
   incident shows arms do get treated individually).

3. **Four dispatch loops** (§4.1) — already caused one real enforcement gap (WF-1D). The
   per-tick concerns (interrupt, limits, tier poll, GC safepoint, time-travel) belong in
   one inlined `tick()` helper.

4. **`capture_vm_state` eagerness** (§9.9): a full VM-state clone as a fixed tax on
   every stdlib module-fn call is the kind of decision that will dominate profiles once
   anyone benchmarks stdlib-heavy code, and it multiplies the §9.1 exposure by running
   the closure-block walk constantly.

5. **Split restore drivers** (§5.4): `from_snapshot`/`restore_call_stack` (working)
   vs `resume()`/`apply_pending_resume` (todo!/'NotImplemented') — when the in-VM resume
   is finally built, it will either duplicate or diverge from the CLI path unless the
   two are unified first.

6. **Dead scaffolding kept warm**: module-wide `allow(dead_code)` on `ic_fast_paths.rs`
   hides genuine rot (a `transmute`-based cache nothing consults) alongside intentional
   future surface; dead PHF registries (`RANGE_METHODS` reachable-never,
   `INT_ARRAY_METHODS` routed-never) sit next to live ones with nothing marking the
   difference at the definition site.

7. **User-visible internal jargon**: `NotImplemented` surfaces leak ADR/wave vocabulary
   ("W17-marshal-return-arms SURFACE (ADR-006 §2.7.14)...") into stderr on ordinary
   programs (observed on t1's `[jit-fallback]` banner). Honest, but hostile to users
   and it teaches them to ignore diagnostics.

8. **Diagnostic location drift**: method-not-found and closure-capture errors point at
   the *binding* line rather than the offending expression (t8/t9, t12 all report
   line 1). Small, but it compounds with 7.

9. **Stale ignore reasons** (§7.3) — two ignore strings describe already-fixed
   surfaces; in a repo that (correctly) treats `#[ignore]` reasons as tracking state,
   stale reasons are misinformation.

10. **God-object `VirtualMachine`**: 50+ fields spanning execution, security, snapshot,
    foreign linking, metrics, debugging. Every subsystem takes `&mut VirtualMachine`,
    which is why re-entrancy needs the RefCell-parking contortion in
    `vm_impl/modules.rs:873-899`.

---

## 12. Prioritized recommendations

### P0 — do before any closure-snapshot work ships

1. **Unify the `closure_heap_bits` carrier** (§9.1). Make `restore_call_stack` produce
   `Arc<HeapValue::ClosureRaw>` bits; rewrite `try_borrow_closure_block` and
   `snapshot_frame_upvalues_serializable` to recover via `as_heap_value()`; delete the
   raw-block interpretation; fix the two wrong SAFETY comments; add a carrier-shape
   pinning test (producer → capture_vm_state → assert upvalues + refcounts). Effort:
   ~1 day incl. tests. This is a hard prerequisite for the release-blocking W17 closure
   snapshot work — landing that work on top of the current split turns a silent bug
   into deterministic heap corruption.

### P1 — correctness fixes, each small

2. **Replace the two printing `todo!()`s** with `NotImplemented` or a minimal formatter
   (`printing.rs:430,447`). Effort: <1 hour. (§9.2)
3. **Release `closure_heap_bits` in `handle_exception`** — reuse `unwind_call_frames_to`
   for the truncate at `exceptions/mod.rs:151`. Effort: <1 hour + test. (§9.3)
4. **Extend error-unwind to the `UInt64` arm and `resolve_spawned_task`**
   (`call_convention.rs:1187, :622`). Effort: <1 hour + test. (§9.4)
5. **Compile-error for mut-self methods on immutable bindings** (compiler-side gate
   reading the existing `MUT_SELF_*` sets; coordinate with vertical 02). Effort: 1-2
   days incl. test rebaselines. (§9.6)
6. **Wire Range methods through the checker or delete the registry** — one source of
   truth for method-name tables, plus a cross-check test iterating every PHF map against
   the compiler's tables. Effort: 1 day. (§9.5, §5.2)
7. **JIT recursion depth guard** (hand to vertical 05; the interpreter behavior at
   `call_convention.rs:93-98` is the spec). (§9.7)

### P2 — debt reduction, ordered by leverage

8. **Single `tick()` for the dispatch loops** + collapse the four loops' shared blocks.
   Effort: ~1 day; removes the recurring enforcement-gap class. (§4.1)
9. **Lazy `capture_vm_state`** (thunk or `needs_vm_state` flag on module-fn entries).
   Effort: ~1 day; removes a per-stdlib-call tax and shrinks §9.1 exposure. (§9.9)
10. **Macro-generate the per-width opcode families** in `variables/mod.rs`. Effort: 1-2
    days; -1,500 LOC. (§4.3)
11. **Merge the 11 typed return handlers** into one (keep the opcode variants). Effort:
    <1 hour. (§4.2)
12. **Scrub stale `ValueWord` doc comments** (`executor/mod.rs:232,337,406`;
    `call_convention.rs:1087`) and the two stale ignore reasons
    (`v2_handlers/integration_tests.rs:140,154`). Effort: <1 hour; keeps the grep-based
    forbidden-pattern enforcement high-signal. (§5.5, §7.3)
13. **Remove or wire `record_allocation`** (`resource_limits.rs:138`) — either the
    alloc-budget path is the design (delete this) or it isn't (call it). Effort: trivial.
14. **Surface (don't silently mass-cancel) non-resumable suspensions**
    (`dispatch.rs:987-998`). Effort: <1 hour. (§9.8)
15. **Fix host-boundary IP restore on mid-body error**
    (`call_convention.rs:170-187, 239-260`). Effort: <1 hour. (§9.10)

---

## Appendix A — empirical test matrix

All programs run with `target/debug/shape run` from this working tree; extension-load
banner lines elided. `vm` = `--mode vm`; `jit` = default mode. Scratch sources under the
session scratchpad `verticals/vm-interpreter/`.

| # | Program (essence) | Mode | Result | Referenced in |
|---|-------------------|------|--------|---------------|
| t1 | closure value `add(2,3)`, loop accumulation | jit | `5` / `20` + loud `[jit-fallback]` whole-program deopt banner | §2.7, §8, §11.7 |
| t2 | infinite recursion `boom(n+1)` | jit | **host thread stack overflow (process crash)** | §9.7 |
| t2 | same | vm | `Error: Runtime error: Stack overflow (line 1)` | §2.3 |
| t3 | `10 / 0` | vm | `Error: Runtime error: Division by zero` | §2.1 |
| t4 | `xs.reduce(0, |a,b| a+b)` (wrong arg order) | vm | compile error with corrective hint "signature is `reduce(f, init)`" | §2.2 |
| t4b | map/filter/reduce/toUpperCase/split + `let m = HashMap(); m.set; m.get` | vm | collections correct; **`m.get("a")` → `None`** (silent lost mutation) | §2.2, §9.6 |
| t5 | `.iter().map().collect()` + `(0..5).toArray()` | vm | semantic error (Range), misleading location | §9.5 |
| t6 | `let mut m = HashMap(); m.set; m.get` | vm | `1` (correct with `mut`) | §9.6 |
| t7 | `xs.iter().map(|x| x*2).collect()` | vm | `[2, 4, 6]` — iterator terminals work despite ignored tests | §2.2, §7.3 |
| t8 | `(0..5).toArray()` isolated | vm | `Method 'toArray' not found on type 'Range'` @ line 1 | §9.5 |
| t9 | `r.contains(3)` | vm | `Method 'contains' not found on type 'Range'` @ line 1 | §9.5 |
| t10 | `10/x` with `x=0` inside `forEach` closure | vm | two prints then clean `Division by zero (line 3)` — closure-arm unwind works | §2.3 |
| t11/b/c | `state.locals()` under `import state` variants | vm | all import forms rejected (incl. book's `import x from "std:..."`) | §8 |
| t12 | `snapshot()` lexically inside closure w/ mutable capture | vm | `op_make_closure: capture 1 is stamped a heap carrier (Ptr(NativeView)) but ... scalar kind Bool` — surface-and-stop | §2.4 |
| t13 | `use std::core::math` + `math.sqrt` in closure | vm | `Undefined variable: 'math'` | §8 |
| t14 | `snapshot()` in plain fn; then CLI `--resume <hash>` | vm | `HASH:fadecb37...`; resume → `RESUMED:105`; resume-with-source correctly refused | §2.4 |
| t15 | `snapshot()` called below a live closure frame | vm | `ERR:Barrier("... a live closure value is reachable and this build cannot yet save closures ...")` — masks §9.1 mode B | §9.1 |
| t16/t17/t19/t20 | `math` namespace visibility probes | vm | bare `math.` and `use std::core::math` + `math.` all undefined; fully-qualified call refused | §8 |
| t18 | 100M-iteration spin under `--max-instructions` / `--max-time-ms` | vm | `Instruction limit exceeded: 100000 >= 100000`; `Wall time limit exceeded: 300.01234ms >= 300ms` | §2.5 |
| t21 | `use std::core::math::sqrt`; closure → helper → `sqrt` | vm | `2.0` / `3.0` — module-fn dispatch **with a live closure frame** succeeds ⇒ `capture_vm_state` misread path (§9.1 mode A) is reachable and silent | §9.1 |
| t22 | `i64::MAX + 1` | vm | structured overflow error naming `as number` / `as bigint` | §2.1 |
| t23 | `loop { break 42 }`, `for`+`break`, `while false` | vm | `42` / `10` / `done` | §2.1 |
| t24 | enum unit/tuple/struct payload match | vm | `3.14159` / `6.0` / `0.0` | §2.1 |
| t25 | `print(f)` where `f` is a closure | vm | **process panic**: `panicked at crates/shape-vm/src/executor/printing.rs:430:17: not yet implemented: phase-2c ...` | §9.2 |

## Appendix B — quantitative inventory

| Metric | Value | Source |
|--------|-------|--------|
| Executor total LOC (158 files) | 103,760 | `find ... | xargs wc -l` |
| Executor test LOC (`executor/tests/`) | 21,665 | same |
| `#[test]` fns in executor | 1,576 | grep |
| `#[test]` fns in feedback/tier/resource_limits | 37 | grep |
| `#[ignore]` in executor | 61 | grep |
| `unsafe` occurrences in executor | 1,243 | grep (incl. comments) |
| `todo!(` in executor | 82 (60 in test files; live non-test: printing ×2, resume() ×1, vm_impl/output ×1, vm_impl/program ×2) | grep |
| `NotImplemented` returns (non-test) | 184 | grep |
| `SURFACE` markers (non-test) | 198 | grep |
| OpCode variants | 420 | `bytecode/opcode_defs.rs` |
| PHF method registries | 20 maps + 10 opt-in sets | `method_registry.rs` |
| Dead PHF registries | 3 (`RANGE_METHODS` unreachable; `INT_ARRAY_METHODS`, `INDEXED_TABLE_METHODS` unrouted) | §2.2 |
| Forbidden-pattern live-code hits | 0 (all matches are deletion-documenting comments) | §6.8 |
| Dispatch loops sharing copy-pasted preambles | 4 | `dispatch.rs` |
| `closure_heap_bits` producers / consumers with mismatched carrier assumptions | 3 producers, 5 consumers, 2 incompatible shapes | §5.1 |

---

*End of report — auditor 04 (VM Interpreter & Dispatch), 2026-07-11.*

