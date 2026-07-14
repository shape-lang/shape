# Vertical Deep-Dive 09: Async & Concurrency

**Auditor:** 09 of 19 · **Date:** 2026-07-11 · **Tree:** working tree at `ce332ca2` + uncommitted changes (audited as-is)
**Territory:** async fn compilation, `async let`, `await`, `async scope`, `join all|race|any|settle`, `for await`, task scheduler & shared async runtime, cross-task sharing (Channel/Mutex/Atomic, SharedAtomic storage classes), JIT × async, snapshot × async.

All Shape programs referenced as `tNN_*.shape` live in the scratchpad
(`/tmp/claude-1000/.../scratchpad/verticals/async-concurrency/`) and were run against the
prebuilt working-tree binary `target/debug/shape`. Extension-loader warnings and the
`[jit-fallback] function main ...` line (async `main` never JITs; see §2.9) are elided from
transcripts unless load-bearing.

---

## 0. Executive summary

**Overall health verdict: PARTIALLY REAL, SHARPLY BOUNDED, WITH ONE SILENT-WRONG-RESULT HAZARD.**

Shape async is not a stub — there is a genuine multi-threaded overlap lane (WF-2D, decision
D1 2026-07-05): async *module* functions (`time::sleep` + 6 `http.*` functions) are spawned
onto a process-global multi-threaded tokio runtime at call-evaluation time, and
zero-argument user `async fn`s with a *declared scalar return* (`int`/`number`/`bool`) are
deferred onto isolated per-task VMs on the blocking pool. Both lanes measurably overlap
(two 300 ms sleeps ≈ 310 ms wall, 20 tasks ≈ 320 ms — §2.2). Everything outside those two
lanes — arg-bearing calls, heap-returning functions, unannotated returns, closures —
**silently degrades to serial eager execution** with no diagnostic. The single-threaded
interpreter cannot suspend a mid-flight frame (the perpetually-deferred "Phase-2c
snapshot-tier"), so the design buys concurrency with a fresh-VM isolation boundary — and
that boundary leaks: a deferred task reading a module-level binding sees **silent zeros**
instead of the initialized value (P0, §9.1). Join semantics have carrier holes (`join all`
over strings = runtime error) and ordering holes (`join race` returns a slow eager branch
over a fast deferred one). Cancellation of deferred user tasks is cosmetic
(`spawn_blocking` tasks cannot be aborted once started — demonstrated side effect after
"cancellation", §9.3). Channel exists in the VM but is dead at the surface: the type
checker rejects every Channel method. Async code never JITs (all 15 async opcodes are
VM-only by preflight). Snapshot × pending futures is the best-engineered edge here: a
clean, precise, designed refusal.

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P0 | Deferred async task reads module globals as silent zero → wrong results, no error (`GLOBAL=100`; task returns `GLOBAL+1` ⇒ prints `1`) | §9.1, t22 transcript; `async_runtime.rs:91-98` documents the boundary but runtime silently mis-computes |
| 2 | P1 | `join race` returns the *slowest* branch when any branch is non-deferrable: eager branches run serially at spawn time and always "win" in source order (1000 ms loser beats 50 ms winner) | §9.2, t14 transcript; `async_ops/mod.rs:888-908` |
| 3 | P1 | Cancellation of deferred user async-fn tasks does not stop them — `AbortHandle::abort()` on a started `spawn_blocking` task is a no-op; loser's side effects still run after `join race` returns | §9.3, t15 transcript; `async_ops/mod.rs:467-479`, `task_scheduler.rs:290-301` |
| 4 | P1 | `async let` concurrency silently serializes for arg-bearing / heap-returning / unannotated user fns (2×300 ms ⇒ 605 ms) — book says "run concurrently" unconditionally | §9.4, t06+t07 transcripts; deferral gate `advanced.rs:972-1010` |
| 5 | P1 | Channel is dead at the language surface: type checker has zero Channel method seeds — `c.send(1)` / `c.try_recv()` = semantic error; VM PHF handlers are unreachable; `recv()` on empty = `NotImplemented` | §9.5, t23/t24 transcripts; `method_table.rs:1086` (follow-up comment), `channel_methods.rs:154-162` |
| 6 | P1 | `join all` materializes only `int`/`number`/`bool`/TypedObject results; strings, arrays, maps, options ⇒ runtime error | §9.6, t10 transcript; `async_ops/mod.rs:803-816` |
| 7 | P1 | `for await x in futures` fails type-check (element type never `await`-unwrapped: "Type 'Future' does not implement trait 'Numeric'"); `ForExpr.is_async` is ignored by the entire type system | §9.7, t16 transcript; zero `is_async` handling in `type_system/` |
| 8 | P1 | `async let` with a closure RHS fails inference ("Could not infer generic type arguments for 'Future'"), making the entire closure-spawn machinery (`op_spawn_task` Closure arm, `resolve_spawned_task` closure path) unreachable from source; also masks the dedicated `&mut`-across-task-boundary diagnostic | §9.8, t27/t27b/t32 transcripts |
| 9 | P2 | `TaskScheduler::resolve_task_group` is a zero-caller parallel implementation of join semantics that has already drifted from the live `op_join_await` (dead: All→TaskGroup carrier, Race→first-in-source; live: All→typed array, Race→first-settled poll) | §5.1; grep: no production callers |
| 10 | P2 | 9 of 15 async opcodes are never emitted by the compiler (Yield/Suspend/Resume/Poll/AwaitBar/AwaitTick/EmitAlert/EmitEvent/CancelTask); EmitAlert/EmitEvent handlers silently drop their payloads; JIT `jit_cancel_task` is an `extern "C"` `todo!()` that would abort the process if ever reached | §3.4, §2.9; `async_ops/mod.rs:274-281,1013-1024`, `shape-jit/src/ffi/async_ops.rs:197-216` |

Honorable mentions (see §9): `join settle` returns an opaque `[TaskGroup:Settle(2)]`
(book-disclosed as v0.4 preview, so P2); the book's `join all` caution is now stale in the
*other* direction (working tree materializes indexable arrays); busy-wait 1 ms polling in
race/any; ADR-006's `SharedAtomic`/`SharedAtomicMut` storage classes exist nowhere in the
codebase; a **bare function reference** RHS (`async let a = work` — no call parens,
including *sync* fns) bypasses the compiler's scalar-return deferral gate entirely and
spawns onto the isolated VM (§9.15); `join all` over unit-returning tasks materializes
`[false, false]` — the unit sentinel leaking as user-visible bools (§9.16).

**Feature completeness: 55/100** — every syntactic feature parses and compiles, and the
happy path of each works, but real concurrency covers only two narrow lanes, three of four
join strategies have semantic or carrier holes, Channel is dead, `for await` is
sugar-over-sync-iteration that type-fails on its motivating use case, and cancellation is
partly cosmetic.

**Code quality: 70/100** — the live paths show real discipline (kinded `(bits, kind)`
carriers end-to-end, explicit refcount share accounting on every transfer, per-arm
`debug_assert_eq!` stack-depth checks, honest surface-and-stop errors), but the territory
carries a zero-caller parallel join implementation, stale docstrings referencing deleted
machinery, dead opcodes, and a `todo!()` in an `extern "C"` function.

**Biggest risk:** the isolation-boundary wrong-result class (finding #1). The design
choice "run deferred tasks on a fresh VM without running module initializers" converts a
concurrency limitation into a *silent data-corruption* bug for any deferred function that
touches module state — the exact class of bug strict typing was supposed to make
impossible, reachable from a 7-line program with no warnings. Combined with #4 (the
deferral gate is invisible at the surface), users cannot tell which of their `async let`s
actually run concurrently, which serialize, and which compute wrong values.

---

## 1. Architecture & code structure map

### 1.1 Module inventory (working tree, `wc -l`)

| File | LOC | Responsibility |
|------|-----|----------------|
| `crates/shape-vm/src/executor/async_ops/mod.rs` | 1194 | All 15 async opcode handlers (`op_await`, `op_spawn_task`, `op_join_init`, `op_join_await`, scope enter/exit, cancel, plus 9 legacy handlers); join-all typed-array materialization; race/any polling loops; `AsyncExecutionResult`/`SuspensionInfo`/`WaitType` types. Tests at lines 1050-1194. |
| `crates/shape-vm/src/executor/task_scheduler.rs` | 790 | `TaskScheduler`: `callables` / `results` / `external_receivers` / `pending_async` maps; kinded `(u64, NativeKind)` carrier API; refcount share accounting; `FutureSnapshotStatus`; `Drop` releases owned shares. Tests at lines 585-790. |
| `crates/shape-vm/src/executor/async_runtime.rs` | 148 | Process-global multi-threaded tokio runtime (`OnceLock`, `shared_runtime()`, `async_runtime.rs:43-59`); `run_isolated_async_fn` (fresh isolated VM per deferred user async fn, `:102-126`); scalar-only result marshal (`kinded_scalar_to_typed_return`, `:131-148`). |
| `crates/shape-vm/src/executor/call_convention.rs` | 1526 | `resolve_spawned_task` (`:477-636`): cached fast-path, closure/function-id dispatch, drive-to-completion via `execute_until_call_depth`; `execute_with_async` (`:411-423`) — which is just `execute_fast` (see §3.5). |
| `crates/shape-vm/src/executor/vm_impl/modules.rs` | 1726 (async part ≈ `:608-736`) | `spawn_async_module_future` (spawn onto shared runtime, mpsc completion channel, abort handle); `resolve_pending_async_task` (blocking `recv`); `try_resolve_pending_async_task` (non-blocking poll); `project_and_cache_pending_async` (TypedReturn → KindedSlot projection on the interpreter thread). |
| `crates/shape-vm/src/executor/builtins/remote_builtins.rs` | 2002 (async part ≈ `:963-975`, `:1726-1834`) | `remote::call_async` reuses the same `pending_async` infrastructure + cancellation hooks (`set_pending_async_cancellation_hook`). |
| `crates/shape-jit/src/ffi/async_ops.rs` | 316 | JIT FFI trampolines for async ops — mostly stub/dead (see §2.9): `jit_join_init` returns `TAG_NULL`, `jit_cancel_task` is `todo!()`. |
| `crates/shape-vm/src/executor/objects/channel_methods.rs` | 245 | Channel PHF handlers (send/recv/try_recv/close/is_closed/is_sender) — VM-side complete for same-thread queue, but unreachable from source (§9.5). |
| `crates/shape-vm/src/executor/objects/concurrency_methods.rs` | 364 | Mutex/Atomic/Lazy handlers. `Mutex.lock()` is a documented no-op marker (single-threaded VM), `try_lock()` always `true` (`concurrency_methods.rs:140-165`). |
| `crates/shape-runtime/src/stdlib_time.rs` | 279 | `time` module: `sleep` is the canonical async module function (`register_typed_async_function`, `:65-87`); `now`/`sleep_sync`/`benchmark`/`stopwatch`/`millis` are sync. |
| `crates/shape-runtime/src/stdlib_io/async_file_ops.rs` | 23 | **Stub.** `register_async_file_io` body is empty; the 5 `io.*_async` functions are deferred ("next-session path-mass migration", `:19-22`). |

Compiler/front-end async surface:

| Location | What |
|----------|------|
| `crates/shape-ast/src/shape.pest:793,1168,1179-1183` | `await_keyword`, `await_expr` (3 forms: annotated await, `await join`, plain), `for_expr` with optional `await` (`:1240`) |
| `crates/shape-ast/src/ast/expressions.rs:252-274` | `Expr::Await`, `Expr::Join`, `Expr::AsyncLet`, `Expr::AsyncScope` |
| `crates/shape-ast/src/ast/expr_helpers.rs:193-210` | `AsyncLetExpr { name, expr, span }` (no type-annotation field), `JoinExpr { kind, branches }` |
| `crates/shape-vm/src/compiler/expressions/advanced.rs:825-1010` | `compile_async_let` + WF-2D-fu deferral classification (`deferrable_async_call_target`, `is_marshalable_scalar_return`) |
| `crates/shape-vm/src/compiler/expressions/advanced.rs:1012-1073` | `compile_async_scope` + closure-literal heap-ABI forcing |
| `crates/shape-vm/src/compiler/expressions/advanced.rs:1082-1130` | `check_task_boundary_safety` / `walk_expr_for_exclusive_refs` (&mut-across-boundary compile error) |
| `crates/shape-vm/src/compiler/expressions/misc.rs:751-817` | `compile_join_expr` (per-branch deferral or eager compile + `SpawnTask`, packed kind/arity operand) |
| `crates/shape-vm/src/compiler/expressions/mod.rs:1826-1849` | `Expr::Await` compile (async-context check, annotated-await routing) |
| `crates/shape-vm/src/compiler/expressions/mod.rs:1113-1316` | `compile_annotated_await_expr` (user-annotation before/after handlers around await; no builtin `@timeout` — §2.8) |
| `crates/shape-vm/src/compiler/loops.rs:302-513, 775-1056` | `for await` = normal iteration + `OpCode::Await` before element binding |
| `crates/shape-runtime/src/type_system/inference/expressions.rs:2998-3055, 3191-3210` | Await/Join/AsyncLet/AsyncScope inference; `awaited_type_for` Future\<T\> unwrap; join-all → `Array<T>` when homogeneous |
| `crates/shape-vm/src/mir/solver.rs:129, 380-407, 1354-1366` | B0013 `ExclusiveRefAcrossTaskBoundary` loans + B0014 `NonSendableAcrossTaskBoundary` facts for detached tasks |

### 1.2 Data flow (the three spawn lanes)

`async let x = RHS` compiles to one of three shapes, decided at `compile_async_let`
(`advanced.rs:836-948`):

1. **Deferred lane (real concurrency, WF-2D-fu).** RHS is a *zero-argument call to a user
   `async fn` whose declared return annotation is `int`/`number`/`bool`*
   (`deferrable_async_call_target`, `advanced.rs:972-1010`). Compiler emits
   `PushConst(Constant::Function(id))` + `SpawnTask`. At runtime `op_spawn_task`'s
   `NativeKind::UInt64` arm (`async_ops/mod.rs:443-497`) clones the whole
   `BytecodeProgram`, spawns `run_isolated_async_fn` on the shared runtime's
   **blocking pool** (`spawn_blocking`), stores a `PendingAsyncTask { completion: mpsc
   Receiver, abort: Some(AbortHandle) }`, and pushes `Future(id)`. The isolated VM
   (`async_runtime.rs:102-126`) registers stdlib modules and `populate_module_objects()`
   but **does not run module initializers**; the result crosses back only if it is a leaf
   scalar (`kinded_scalar_to_typed_return`, `:131-148`).
2. **Module-async lane (real concurrency, WF-2D).** RHS is `time::sleep(..)` / `http.*`
   (a `QualifiedFunctionCall` to a `ModuleFnEntry::TypedAsync`). The call itself already
   spawned the `Send + 'static` future onto the shared runtime
   (`modules.rs:622-645`) and produced a `Future(id)`; `op_spawn_task`'s
   `Ptr(HeapKind::Future)` passthrough arm (`async_ops/mod.rs:416-442`) re-pushes the
   id unchanged (no double-spawn) and tracks it in the active async scope.
3. **Eager lane (no concurrency).** Everything else: the RHS is compiled inline —
   *the entire async fn body runs to completion on the interpreter thread right there* —
   and `op_spawn_task`'s catch-all arm (`async_ops/mod.rs:502-524`) registers the
   already-computed value as a pre-completed result via `scheduler.complete(id, bits,
   kind)`. `await` later hits the cached fast-path.

`await x` (`op_await`, `async_ops/mod.rs:290-354`): a `Ptr(HeapKind::Future)` slot routes
to `resolve_pending_async_task` (blocking `recv` on the completion channel,
`modules.rs:656-673`) when the id names an in-flight task, else to
`resolve_spawned_task` (`call_convention.rs:477-636`) for cached values / registered
callables. Any non-Future slot is pushed back unchanged (sync shortcut). There is **no
event loop**: awaiting blocks the interpreter thread; overlap exists only because sibling
tasks were spawned earlier onto other OS threads.

`await join K { b1..bn }` (`compile_join_expr` → `JoinInit(packed)` + `JoinAwait`):
`op_join_init` (`async_ops/mod.rs:553-600`) pops n Future ids into an
`Arc<TaskGroupData>`; `op_join_await` (`:608-744`) dispatches per strategy — All
materializes a homogeneous `TypedArray` (working-tree addition, uncommitted; see §1.4),
Race/Any run a 1 ms-sleep polling loop over `try_resolve_pending_async_task`
(`:888-950`), Settle drives all tasks and returns the raw TaskGroup carrier.

`async scope { .. }` = `AsyncScopeEnter` (push empty id vec onto
`vm.async_scope_stack`) + body + `AsyncScopeExit` (pop, `scheduler.cancel(id)` in LIFO
order, `async_ops/mod.rs:981-1011`).

### 1.3 Key types

- `HeapValue::Future(u64)` — the future is *just the task id* stored inline in the slot
  bits (`NativeKind::Ptr(HeapKind::Future)` with no Arc backing; drop/clone are no-ops
  for this kind). No completion state on the value itself.
- `TaskGroupData { kind: u8, task_ids: Vec<u64> }` — Arc-backed join aggregate
  (`shape-value/src/heap_value.rs`).
- `TaskScheduler` (per-VM) — four maps: `callables` (kinded pairs), `results`
  (`TaskStatus::Pending|Completed(Kinded)|Cancelled`), `external_receivers` (tokio
  oneshot, remote calls), `pending_async` (mpsc receivers + abort handles for in-flight
  runtime tasks) + `pending_async_cancel_hooks` (remote receiver-side cancel).
- `AsyncExecutionResult::{Continue, Yielded, Suspended(SuspensionInfo)}` and
  `WaitType::{NextBar, Timer, AnyEvent, Future, TaskGroup}` — the suspension protocol.
  In practice only the snapshot sentinel (`SNAPSHOT_FUTURE_ID = u64::MAX`,
  `executor/mod.rs:50`) flows through it; real awaits resolve inline/blocking (§3.4).
- `FutureSnapshotStatus` (working-tree addition) — snapshot-capture diagnostics for live
  future handles.

### 1.4 Uncommitted working-tree changes in-territory

`git diff` shows this vertical is under active surgery (audited as-is):

- `async_ops/mod.rs` (+154/-x): **join-all typed-array materialization** replaces the old
  "resolve then discard, return TaskGroup-of-ids placeholder" — the four
  `materialize_join_all_*` arms are new.
- `task_scheduler.rs` (+151): `abort` became `Option<AbortHandle>` (detached remote socket
  workers use `None` + companion hook); `FutureSnapshotStatus` added.
- `tools/shape-test/tests/async_concurrency/async_scope.rs` (+42): new timing test
  `async_scope_exit_cancels_unawaited_child_without_waiting`.
- `executor/snapshot.rs`: `guard_future_snapshot_slot` FUTURE_SNAPSHOT_BARRIER.

The book's `join all` caution ("individual branch values cannot yet be unpacked",
`async.mdx:71-73`) documents the *pre*-diff behavior — already stale against this tree
(§8.2).

---

## 2. Feature completeness

Legend: ✅ works end-to-end (empirically verified) · ⚠️ partial (works in a bounded lane)
· ❌ broken/stubbed · ⬛ code exists but unreachable from source.

### 2.1 `async fn` + `await` — ✅ (with weak coloring)

```
=== t01_basic_async ===          basic await: 3
=== t04_await_heap_direct ===    direct await heap: direct heap
=== t38_await_in_sync ===        Semantic error: 'await' can only be used inside an async function
=== t37_sync_calls_async ===     sync calling async compiled+ran: 1
```

`await f()` inside async fns works for scalar and heap returns. `await` in a sync fn is a
compile error (`expressions/mod.rs:1828-1834`); top-level `await` is allowed (the check is
gated on `current_function.is_some()`). But async coloring is weak: a **sync fn can call
an async fn bare** (t37) — the body just runs eagerly. There is no "unawaited future"
diagnostic anywhere.

### 2.2 `async let` — ⚠️ real concurrency in two lanes only

Measured (300 ms sleeps, `time::now().elapsed_ms()`):

```
=== t05_overlap_scalar ===        sum=3 elapsed_ms=309.886829       (zero-arg, -> int : OVERLAPS)
=== t06_serialize_args ===        sum=2 elapsed_ms=605.2900500000001 (arg-bearing      : SERIAL)
=== t08_module_sleep_overlap ===  module sleep overlap elapsed_ms=303.31356700000003 (module async: OVERLAPS)
=== t30_mass_tasks ===            20 tasks len=20 elapsed=320.47328200000004 (join all × 20: OVERLAPS)
```

Eagerness of the non-deferred lane is directly observable in print order (t07):

```
1: before async let
2: inside work (eager path)      <-- body ran BEFORE async let returned
3: after async let, before await
4: awaited done
```

The deferral gate (`advanced.rs:972-1010`) requires: `Expr::FunctionCall` + zero args +
`func.is_async` + declared return annotation ∈ {`int`,`number`,`bool`}. An absent
annotation is rejected ("an inferred return could be a heap type"). The gate is correct
*as a regression fix* (the WF-2D-fu heap-return regression from project memory is indeed
fixed — t03/t04 heap returns work, via the eager path), but it makes concurrency an
invisible function of the callee's signature (§9.4).

`async let` outside an async fn is a clean compile error (t34), matching the book.

### 2.3 `join all` — ⚠️ works for 4 carrier kinds, runtime-errors otherwise

```
=== t09_join_all_ints ===    join all: 10 20 elapsed=209.216981   (2×200ms overlapped, indexable)
=== t35_join_all_number ===  1.5 2.5
=== t39_join_typed_obj ===   points: 1 4                          (TypedObject carrier)
=== t10_join_all_strings === Error: Runtime error: join all cannot materialize result carrier String;
                             supported homogeneous carriers are int, number, bool, and typed object (line 4)
```

The materialization (`async_ops/mod.rs:781-877`, uncommitted) covers
`Int64`/`Float64`/`Bool`/`Ptr(TypedObject)`; strings and every other heap carrier error
loudly at runtime (§9.6). Mixed carriers also error (`:791-801`). Type inference has a
matching lane: homogeneous proven joins infer `Array<T>`
(`inference/expressions.rs:3011-3030`), which is why `results[0]` type-checks.

### 2.4 `join race` / `join any` — ⚠️ correct only when all branches are deferred

```
=== t11_join_race ===       race winner=1 elapsed=109.123081       (fast deferred branch wins, real race)
=== t12_join_any_err ===    any winner=7                           (failure skipped)
=== t36_race_all_fail ===   Error: Runtime error: Index out of bounds: 8 (length: 1) (line 14)  (any: last error surfaced)
=== t14_race_eager_heap === race winner=slow-heap elapsed=1003.747071   <-- WRONG: 1000ms heap branch beats 50ms scalar branch
```

`join_race_first_settled` (`async_ops/mod.rs:888-908`) checks non-pending-async branches
first *in source order* — but non-deferrable branches already ran to completion serially
at spawn time, so a slow eager branch always "settles first" (§9.2). When every branch is
in-flight, the 1 ms polling loop returns genuinely first-settled results. `join any`
skips failures and surfaces the last error when all fail (message does not say "all
branches failed" — P2 papercut).

### 2.5 `join settle` — ❌ opaque result (book-disclosed)

```
=== t13_join_settle ===  settle result: [TaskGroup:Settle(2)]
```

All tasks are driven to completion (`async_ops/mod.rs:705-720`) but per-branch
{status, value/error} is not constructible — the carrier is the raw TaskGroup and no
method/field surface exists to inspect it. The book flags this as a v0.4 preview
(`async.mdx:117-119`), so this is a known, disclosed hole rather than an over-claim.

### 2.6 `for await` — ⚠️ sync sugar; type-broken on its motivating case

```
=== t19_for_await_simple ===  got 1 / got 2 / got 3        (plain array: works)
=== t16_for_await ===         Semantic error: trait bound not satisfied:
                              Type 'Future' does not implement trait 'Numeric'
```

Runtime lowering is honest sugar: each element gets an `Await` opcode before binding
(`loops.rs:511-513`, `:1054-1056`), so an array of futures *would* work at the VM level.
But the type system contains **zero** handling of `ForExpr.is_async` (grep across
`type_system/` finds no use), so the element binding keeps type `Future` and any use of
it fails (§9.7). The book is honest here: "`for await` currently works over ordinary
collections. A real async stream protocol is still a future extension" (`async.mdx:155`).
There are no streams, no async iterators, no backpressure — nothing to audit beyond this.

### 2.7 `async scope` — ⚠️ tracking/cancel bookkeeping works; abort is lane-dependent

```
=== t17_scope_cancel ===   scope body done / scope exited elapsed=103.245756 / grace period over
=== t28_scope_escape ===   Error: Runtime error: Task 1 was cancelled (line 12)
```

Scope exit does not wait for pending children (103 ms exit with a 5000 ms child pending)
and marks them cancelled — a future escaping the scope and awaited afterwards errors
"Task 1 was cancelled" (defensible structured-concurrency semantics, though the type
system happily types the escape, `inference/expressions.rs:3055`). **However** actual
termination of the child's work is lane-dependent: module-async tasks (`tokio::spawn`)
are genuinely aborted; deferred user async-fn tasks (`spawn_blocking`) keep running to
completion in the background (§9.3 proves the side effect fires). The new working-tree
test only asserts *timing* (exit doesn't wait), not termination.

### 2.8 Annotated await — ⚠️ machinery exists, no builtin policies

`await @ann expr` compiles a full before/after handler protocol with short-circuit
support (`expressions/mod.rs:1113-1316`). But there is no builtin `@timeout`/`@retry`:

```
=== t31_timeout_annotation ===  Semantic error: Unknown annotation '@timeout'
```

The grammar comment `await @timeout(5s) fetch()` (`shape.pest:1181`) and the book's
"Annotated Await" section both point at user-defined annotations (cookbook); no timeout
primitive exists anywhere in the async runtime — there is **no way to bound an await**
(a hung task hangs the program; `resolve_pending_async_task` is an unbounded `recv()`,
`modules.rs:666`).

### 2.9 JIT × async — ❌ by design (interpreter-only), with a landmine

All 15 async opcodes are classified VM-only by JIT preflight
(`shape-jit/src/compiler/accessors.rs:595-610`; test
`async_opcodes_are_vm_only_until_jit_async_is_kinded`, `:1195-1224`) — any function
containing one is routed to the interpreter before compilation. Empirically every async
`main` prints `[jit-fallback] ... running under interpreter`. The FFI trampoline layer
(`ffi/async_ops.rs`) is therefore dead code: `jit_join_init` returns `TAG_NULL`
(`:122-153`, SURFACE), `jit_cancel_task` is `todo!()` inside `extern "C"` (`:197-216`) —
unwinding across `extern "C"` aborts the process, which the ignored test at `:296`
documents (SIGABRT). Unreachable today, but a process-abort landmine if the preflight
list and this file ever drift (§11).

### 2.10 Async stdlib surface — ⚠️ 7 functions total

Genuinely-async module functions in the whole stdlib: `time::sleep`
(`stdlib_time.rs:65-87`) + 6 http functions (`stdlib/http.rs:167,206,283,340,387,442` —
get/delete via `register_typed_async_fn_2_full`, post/put/patch/request via `_3_full`).
The async file-ops module is an **empty stub** (`stdlib_io/async_file_ops.rs:19-22` —
"registrations DEFERRED"). Every other I/O call (file ops, process, network_ops) blocks
the interpreter thread and can never overlap.

### 2.11 Channels / Mutex / Atomic / cross-task sharing — ❌ / ✅ / ✅ / ❌

```
=== t23/t24 ===  Semantic error: Method 'try_recv' not found on type 'Channel'
                 Semantic error: Method 'send' not found on type 'Channel'
=== t25_mutex_atomic ===  mutex get: 20 / atomic: old=5 now=8
```

Channel: constructor type-checks (`environment/mod.rs:1179-1182`) but zero method seeds
exist in the checker (`method_table.rs:1086` acknowledges: "FOLLOW-UP (remaining
concurrency-method seeds): `Lazy` / `Channel`"), so every use is a semantic error — the
245-line VM handler file is unreachable (§9.5). Even if reachable: `recv()` on empty =
`NotImplemented` (blocking cross-task recv needs the Phase-2c scheduler boundary,
`channel_methods.rs:154-162`) and `is_sender()` = `NotImplemented` (`:228-245`).
Mutex/Atomic work same-thread (lock is a no-op marker). **Cross-task sharing is
structurally impossible in the real-concurrency lanes**: deferred tasks run on isolated
VMs that share no heap (`async_runtime.rs:89-93`), so a Channel/Mutex/Atomic can never
be seen by two concurrent tasks. ADR-006's `SharedAtomic`/`SharedAtomicMut` storage
classes do not exist in the code (§6.4).

### 2.12 Snapshot × async — ✅ clean designed refusal

```
=== t26c_snapshot_result ===
snapshot returned: Err(Barrier("cannot checkpoint while Future(1) is still live at stack[0]:
pending async task. Await it, cancel it, or move snapshot() after it resolves; resumable
futures are not implemented yet."))
r=1
```

`guard_future_snapshot_slot` (`executor/snapshot.rs:132-150`) consults
`future_snapshot_status` for a precise diagnostic; the generic codec also refuses nested
Future fields (`shape-runtime/src/snapshot.rs:1970-1976`). No silent state loss, the
program continues, and the pending task still completes (r=1). This is the
best-engineered async edge in the territory.

### 2.13 Error propagation across await — ✅

```
=== t20_err_prop ===      Error: Runtime error: Index out of bounds: 42 (length: 1) (line 10)
                          (error raised inside an ISOLATED deferred task; correct message + line)
=== t21_result_qmark ===  err boom     (Result + `?` across await works)
=== t18_double_await ===  double await: 5 5   (cached completion; second await returns clone)
```

---

## 3. Code quality

### 3.1 Idiom & discipline (good)

The live async paths are among the more disciplined code in the executor:

- **Kinded carriers end-to-end.** The scheduler API was fully migrated off the deleted
  legacy tagged-word representation to `(bits: u64, kind: NativeKind)` pairs
  (`task_scheduler.rs:12-40` documents the migration; type alias `Kinded`, `:59`). No
  kind fabrication from raw bits anywhere in-territory.
- **Explicit share accounting.** Every ownership transfer is commented and balanced:
  `register` replaces prior callables with `drop_with_kind` (`task_scheduler.rs:256-264`),
  `complete` releases a double-completion's prior share (`:278-287`), cached fast-paths
  `clone_with_kind` before handing out (`:349-354`, `call_convention.rs:484-489`),
  and `Drop for TaskScheduler` drains both maps (`:531-563`). The
  `push_kinded(result.raw(), result.kind()); std::mem::forget(result)` transfer pattern
  is used consistently (`async_ops/mod.rs:332-333, 677-678, 695-696`).
- **Stack-depth postconditions.** `op_await`/`op_spawn_task`/`op_join_await` each carry
  `debug_assert_eq!(self.sp, sp_before, ...)` on every arm.
- **Surface-and-stop instead of fabrication.** Heap-return marshal failure is a loud
  `NotImplemented` with a remediation hint (`async_runtime.rs:137-144`); join-all
  unsupported carriers likewise (`async_ops/mod.rs:810-815`).

### 3.2 unsafe usage

| File | `unsafe` occurrences | Assessment |
|------|------|------------|
| `async_ops/mod.rs` | 9 | Justified: `Arc::from_raw` reclaim in `op_join_await` (with SAFETY comment citing the construction contract, `:618-623`); `TypedArray::push` + `stamp_elem_type` in the 4 materialization arms (`:821-877`). All follow the construction-side invariant pattern. |
| `channel_methods.rs` | 1 | Canonical typed-Arc recovery (`as_channel`, `:75-81`), cites the 5-arm receiver-recovery soundness rule. |
| `shape-jit/ffi/async_ops.rs` | 12 | `transmute` of trampoline pointers + `#[unsafe(no_mangle)]`. Currently dead code (preflight routes async away from JIT). The transmutes are the usual trampoline pattern but live in a file whose functions are stubs — dead unsafe. |
| `task_scheduler.rs`, `async_runtime.rs` | 0 | Clean. |

No unjustified unsafe found in-territory. The materialization arms `mem::forget` each
`KindedSlot` after pushing its raw bits into the TypedArray — correct (share transfers
into the array) but fragile if a future edit adds an early `?` between push and forget.

### 3.3 Complexity hotspots

- `op_join_await` (`async_ops/mod.rs:608-744`, ~136 lines, 4-way strategy match with
  per-arm share choreography) is the densest function; each arm duplicates the
  cancel-losers loop.
- `compile_async_let` (`advanced.rs:836-948`) interleaves three lanes + type stamping +
  escape planning; the deferral early-return duplicates the local-declaration block of
  the eager path verbatim (~15 lines, §4.2).
- `resolve_spawned_task` (`call_convention.rs:477-636`) is ~160 lines of which ~90 are
  comments — comment-to-code ratio is extreme throughout this territory (see §10/§11:
  double-edged).

### 3.4 Dead code in-territory

- **9 of 15 async opcodes are never emitted** by any compiler path: `Yield`, `Suspend`,
  `Resume`, `Poll`, `AwaitBar`, `AwaitTick`, `EmitAlert`, `EmitEvent`, `CancelTask`
  (grep over `crates/shape-vm/src/compiler/` finds zero emission sites; only `Await`,
  `SpawnTask`, `JoinInit`, `JoinAwait`, `AsyncScopeEnter`, `AsyncScopeExit` are live).
  Their handlers are placeholder-grade: `op_suspend` ignores its operand and always
  waits `AnyEvent` (`async_ops/mod.rs:182-197`), `op_poll` always pushes Null
  (`:213-222`), `op_emit_alert`/`op_emit_event` pop and **silently drop** the payload
  ("alert pipeline integration is deferred", `:274-281`, `:1017-1024`).
- **`TaskScheduler::resolve_task` + `resolve_task_group` have zero production callers**
  (grep: only the scheduler's own tests + one stale docstring reference at
  `async_ops/mod.rs:605`). ~130 LOC of parallel join logic (§5.1).
- **`execute_with_async` is `execute_fast`** (`call_convention.rs:411-423`) — a
  misleadingly-named alias kept for a rebuild that hasn't happened.
- The JIT FFI trampoline registration (`register_async_task_fns`,
  `ffi/async_ops.rs:70-88`) guards paths no JIT code can reach.
- `WaitType::NextBar` / `Timer` / `AnyEvent` variants are constructed only by
  never-emitted opcodes.

### 3.5 Naming / error-message quality

Error messages on live paths are good (actionable, cite alternatives — e.g. the
FUTURE_SNAPSHOT_BARRIER and the isolation-marshal NotImplemented). Weak spots:
`join any` all-fail surfaces only the last branch's raw error (t36); named-join field
access produces "Generic { base: ... } cannot have fields" (t33) instead of "named join
branches are not implemented"; `&mut`-across-boundary produces a generic Future-inference
failure instead of the purpose-built B0013 message (t32, §9.8).

---

## 4. Duplication & DRY violations

### 4.1 Join-kind encoding duplicated in 4 places

The `0=All / 1=Race / 2=Any / 3=Settle` u8 encoding is independently maintained at:
`compile_join_expr` (`misc.rs:799-804`), `op_join_await` match (`async_ops/mod.rs:644-726`),
`TaskScheduler::resolve_task_group` (`task_scheduler.rs:465-527`, dead), and the
codebase-index doc (`02-runtime.md:677`). No shared enum at the VM tier — `JoinKind`
exists in the AST but is erased to a raw u8 in the packed operand. A renumbering would
be caught only by integration tests.

### 4.2 Deferral-lane local-binding block duplicated

`compile_async_let`'s deferred arm (`advanced.rs:872-891`) and eager arm (`:926-945`)
duplicate declare-local/store/immutable-mark/semantics/stamp/load — 15 lines each,
already divergent in one argument (`payload hint` vs `None` to
`stamp_future_handle_type_for_async_let`). A future fix applied to one arm will miss the
other.

### 4.3 Scalar-marshalability list duplicated across compile/runtime

The set of "kinds that survive the isolation boundary" exists twice:
`is_marshalable_scalar_return` (compiler, `advanced.rs:1008-1010`: `int|number|bool` by
*type name*) and `kinded_scalar_to_typed_return` (runtime, `async_runtime.rs:131-148`:
`Int64|Float64|Bool|Null` by *NativeKind*). They are deliberately coupled ("These are
exactly the kinds handled by...", `:1004`) but not mechanically linked. Divergence
directions: runtime grows a kind without the compiler gate → benign (eager path keeps
covering it); compiler gate grows a name the runtime can't marshal → runtime
`NotImplemented` on previously-working programs. Note the existing asymmetry: runtime
marshals `Null`/unit but the compiler gate rejects unannotated fns, so a `-> unit`-ish
async fn never defers even though the runtime could carry it.

### 4.4 Four near-identical materialization arms

`materialize_join_all_{i64,f64,bool,typed_object}` (`async_ops/mod.rs:818-877`) are
copy-paste with only the element type and `ELEM_TYPE_*` stamp varying. Adding the missing
String arm (§9.6) will make a fifth copy unless generalized.

### 4.5 Cancel-losers loop duplicated

Race and Any arms of `op_join_await` each contain the identical
`for &other in &task_ids { if other != winner_id { self.task_scheduler.cancel(other); } }`
loop (`:672-676`, `:690-694`).

---

## 5. Split-brain analysis

### 5.1 Live join implementation vs dead scheduler join implementation — ALREADY DRIFTED

`op_join_await` (live) and `TaskScheduler::resolve_task_group` (dead, zero callers,
`task_scheduler.rs:456-528`) implement the same four join strategies. They have already
diverged:

| Strategy | Live (`op_join_await`) | Dead (`resolve_task_group`) |
|---|---|---|
| All | typed-array materialization of ordered results | drops results, returns TaskGroup-of-ids |
| Race | polls in-flight tasks, first-settled wins, losers cancelled | returns first id in source order, no cancellation |
| Any | poll loop, skip failures, cancel losers | sequential resolve, no cancellation |
| Settle | TaskGroup carrier + errors preserved | same, but no pending_async awareness |

The dead one still passes its unit tests (verified: all 20 scheduler/async_ops lib tests
green — transcript in §7), which gives false comfort. The scheduler docstring even
documents the dead API as the design ("resolve_task_group ... Since we execute
synchronously...", `:439-455`). This is precisely the parallel-implementation
defection-attractor CLAUDE.md warns about, in miniature. Delete or wire it.

### 5.2 Compiler docstring vs compiler code (`compile_join_expr`)

Docstring: "For each branch, wrap the expression in a closure and emit SpawnTask"
(`misc.rs:742`). Code: compiles the branch expression **eagerly inline** (no closure
wrap) unless deferrable (`:787-796`). The closure-wrapping design would have given
correct race semantics for all branch shapes; the doc describes an implementation that
does not exist, and the gap is exactly the root of §9.2.

### 5.3 Scheduler docstring vs codebase reality

`task_scheduler.rs:36-40` claims "Out-of-territory callers (`call_convention.rs::
resolve_spawned_task`, `async_ops/mod.rs::op_await` ... `gc_integration.rs::scan_roots`)
still reference deleted ValueWord-shape APIs". False on both counts today:
`resolve_spawned_task`/`op_await` are fully kinded, and `gc_integration.rs` is a 63-line
no-op trait impl with **no `scan_roots` at all** (the real GC is trial-deletion in
`shape-value/src/gc.rs:518` — "no root scan" by design). Stale doc pointing at
nonexistent code.

### 5.4 Codebase index vs code

`docs/codebase-index/02-runtime.md:712-…` documents
`external_receivers: HashMap<u64, oneshot::Receiver<Result<ValueWord, String>>>` — the
type is `Result<Kinded, String>` since Wave 6.5 (`task_scheduler.rs:146`). The index also
describes the scheduler as "Inline execution model: tasks run synchronously at
await-time" without the WF-2D pending_async lane, and documents `JoinInit` at stale line
numbers. Anyone navigating by the index will reason about a pre-WF-2D scheduler.

### 5.5 Type system vs runtime semantics (three instances)

1. `async scope` types the escaping future as usable
   (`inference/expressions.rs:3055` — body's type), runtime cancels it at scope exit
   (t28 errors at await).
2. `for await` element: runtime awaits each element; checker never unwraps Future (t16).
3. Channel: value env knows the constructor, method table knows nothing (t23/t24).

### 5.6 VM vs JIT

No semantic split-brain risk today because the JIT refuses all async opcodes (preflight,
`accessors.rs:595-610`) — one-sided rather than divergent. The risk is latent: the stub
FFI file (`ffi/async_ops.rs`) still models NaN-boxed semantics ("NaN-boxed Future(task_id)",
`:100`) from the deleted value model; if async JIT is ever lit up from these stubs
without a rewrite, it resurrects deleted-carrier semantics. The file's own comments
acknowledge this (`:23-32`).

---

## 6. ADR & spec conformance

### 6.1 ADR-005 §1 single-discriminator — CONFORMS

Heap dispatch in-territory goes through `slot.as_heap_value()` + `HeapValue` match:
closure recovery in `resolve_spawned_task` (`call_convention.rs:526-550`, explicitly
citing ADR-005 §1), channel/mutex/atomic receivers (`channel_methods.rs:64-82`,
`concurrency_methods.rs:57-...`). No parallel discriminator sum types found. `TaskGroup`
and `Future` are `HeapValue` arms / HeapKind labels, not parallel enums.

### 6.2 ADR-006 §2.3 typed-Arc payloads — CONFORMS

`TaskGroupData` flows as `Arc::into_raw(Arc<TaskGroupData>)` bits with
`NativeKind::Ptr(HeapKind::TaskGroup)` (`async_ops/mod.rs:593-598`), reclaimed with a
SAFETY-commented `Arc::from_raw` (`:618-623`). `ChannelData`/`MutexData`/`AtomicData`
likewise. No `Box<HeapValue>` in-territory.

### 6.3 ADR-006 §2.7.7 / Q9 parallel-kind carriers — CONFORMS

All async pushes/pops use `push_kinded`/`pop_kinded`; `Future` is an inline-scalar kind
(bits = id) with no-op clone/drop, documented per-arm. The scheduler holds one strong
count per stored heap-bearing kinded pair and releases on Drop (verified by reading all
transfer sites, §3.1). `op_poll`'s empty-queue result was migrated from the forbidden
`(0, Bool)` collision to `NativeKind::Null` (`async_ops/mod.rs:216-221`) — the R5b-2
disposition applied correctly.

### 6.4 ADR-006 bindings model (`var` smart-default, SharedAtomic/SharedAtomicMut) — **NOT IMPLEMENTED**

ADR-006 requires extending `BindingStorageClass` with `SharedAtomic`/`SharedAtomicMut`
for cross-task escape ("No new modal-types subsystem. Existing ... `BindingStorageClass`
(`type_tracking.rs:286`) extended with `SharedAtomic`, `SharedAtomicMut`"). Working-tree
`BindingStorageClass` (`type_tracking.rs:359-368`) = `{Deferred, Direct, UniqueHeap,
SharedCow, Reference, LocalMutablePtr}` — **no SharedAtomic variants; grep across all
crates finds zero occurrences of `SharedAtomic`**. The B0014 `var`-upgrade rule (ADR-006
Q1) is consequently also unimplemented, as `02-runtime.md:733-736` admits. Since the
real-concurrency lanes share no heap, nothing is unsound *today*, but the ADR's
cross-task sharing model simply doesn't exist.

### 6.5 ADR-006 §3.2 / Q1 task-boundary borrow rules — PARTIAL

Machinery exists at two layers: AST-level `walk_expr_for_exclusive_refs` (clear
diagnostic text, `advanced.rs:1096-1110`) and MIR-level B0013/B0014 facts
(`solver.rs:380-407`, error emission `:1354-1366`, `TaskBoundaryKind::Detached`
sendability check `:394-406`). But the checker's Future-generic inference failure fires
*first* for the natural test case (t32), so the dedicated diagnostics are unreachable in
practice for `async let t = &mut x` — masked, not absent.

### 6.6 Forbidden Patterns (CLAUDE.md) — NO LIVE VIOLATIONS FOUND

- No `ValueWord` at runtime in-territory: scheduler fully migrated; remaining mentions
  are historical docstrings describing deleted shapes by name (permitted) — plus the
  **stale index doc** (§5.4) which *describes* a ValueWord-typed field as current
  (doc bug, not code).
- No kind fabrication from raw bits; no `is_heap()` probes; no Bool-defaults (the
  `op_poll` Null fix and the isolation-marshal NotImplemented are the pattern applied
  correctly).
- No transitional ABI shims: `resolve_spawned_task` routes through the §2.7.11 kinded
  value-call family (`call_closure_with_nb_args_keepalive` / `call_function_with_nb_args`).
- The JIT stubs *describe* the deleted pipeline they refuse to resurrect
  (`ffi/async_ops.rs:132-148`), which is the documented surface-and-stop posture.

### 6.7 runtime-v2-spec — CONFORMS (trivially)

Async opcodes never enter the typed VM↔JIT slot ABI (preflight-excluded), so the
KindedSlot-must-not-leak rule (§2.7 / spec) is satisfied by exclusion.

### 6.8 §2.7.4 Phase-2c boundary (suspension across frames) — DOCUMENTED GAP, HONORED

Every site that would need coroutine-style suspension (await inside spawned closure body,
suspension crossing `resolve_spawned_task`, blocking Channel.recv, resumable futures in
snapshots) declares the same Phase-2c boundary and errors rather than faking it
(`async_ops/mod.rs:78-84`, `call_convention.rs:468-476`, `channel_methods.rs:131-138`,
`executor/snapshot.rs:132-150`). Consistent; but note Phase-2c has been "deferred" across
many waves — this is the load-bearing TODO of the whole vertical.

---

## 7. Test coverage in-territory

### 7.1 Unit tests (shape-vm lib) — 20 tests, all green, but shallow on the hot paths

Ran narrowly against the working tree:

```
$ cargo test -p shape-vm --lib -- task_scheduler async_op
running 20 tests
test executor::async_ops::tests::test_is_async_opcode ... ok            (+9 siblings)
test executor::task_scheduler::tests::test_register_and_take_callable ... ok  (+9 siblings)
test result: ok. 20 passed; 0 failed; 0 ignored; ... finished in 0.01s
```

Assessment of what they actually assert:

- `async_ops::tests` (10 tests, `async_ops/mod.rs:1050-1194`): **enum-shape checks only**
  — `is_async_opcode` classification, `WaitType`/`SuspensionInfo` construction. **Zero
  execution coverage** of `op_await`, `op_spawn_task`, `op_join_await`, scope enter/exit,
  the materialization arms, or the race/any polling loops. The entire behavioral surface
  of the 1194-line file is untested at unit level.
- `task_scheduler::tests` (10 tests, `:585-790`): decent API coverage
  (register/take/resolve/cancel/external/snapshot-status), including refcount-relevant
  paths — but they exercise `resolve_task`/`resolve_task_group`, i.e. **the dead parallel
  implementation** (§5.1), not the live `resolve_spawned_task` path. The green suite
  actively tests code production never runs.

### 7.2 Integration tests (`tools/shape-test/tests/async_concurrency/`) — 28 tests / 828 LOC

`async_let.rs` (7), `async_scope.rs` (6, +1 new timing test uncommitted), `for_await.rs`
(3), `future_handles.rs` (3), `join_strategies.rs` (9). Quality is mixed:

- The join tests measure wall-clock deltas (`join_all_two_ms`, `join_race_two_ms`) —
  genuinely assert concurrency, good.
- `async_let.rs:11-16` carries a stale header comment: "Known limitation: The semantic
  analyzer does not track variable bindings from `async let` ... These tests currently
  fail (TDD)" — the analyzer registration was since implemented
  (`inference/expressions.rs:3041-3050`) and t02/t18 pass; the comment misleads.
- The new `async_scope_exit_cancels_unawaited_child_without_waiting` asserts *timing
  only* — it would pass even though the child (in the spawn_blocking lane) keeps running
  (§9.3). No test anywhere asserts actual termination of cancelled work.

### 7.3 Gaps (things this audit tested that no suite tests)

No existing test covers: module-global reads from deferred tasks (§9.1 — the P0),
race-with-eager-branch ordering (§9.2), post-cancellation side effects (§9.3), `join all`
non-scalar carriers (t10), `join settle` result usability (t13), `for await` over
futures (t16), closure-RHS `async let` (t27), snapshot-with-pending-future via the
`snapshot()` builtin (t26c), double-await (t18 — passes, worth pinning), `join any`
all-fail messaging (t36). The one ignored in-territory JIT test
(`test_cancel_task_null_trampoline`, `ffi/async_ops.rs:296`) has an accurate,
well-reasoned ignore string (extern-C `todo!()` aborts the process — reason holds).

---

## 8. Book/docs vs reality

Source: `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/fundamentals/async.mdx`
(10 gate snippets `A__fundamentals__async__0-9` exist in `.book-truth-gate/snippets/`).

| Book claim | Reality |
|---|---|
| "Top-level `await` is supported in scripts and REPL" (`:21`) | ✅ confirmed (t01-style programs; check gated on `current_function.is_some()`). |
| "`async let` spawns an expression as a concurrent task" (`:25`); "Multiple `async let` bindings run concurrently" (`:38`) | ⚠️ **Over-claims.** True only for the two real lanes. Arg-bearing / heap-returning / unannotated user fns run **serially at spawn point** (t06: 605 ms for 2×300 ms; t07 print-order). The book's own examples use plain values (pre-resolved — trivially "concurrent"), so no example is false, but the general claim is. No mention of the deferral gate or the isolation boundary anywhere in the book. |
| `join all` caution: "the `join all` block ... returns a `TaskGroup` summary, but the individual branch values cannot yet be unpacked" (`:71-73`) | ❌ **Stale in the conservative direction** vs. this working tree: `join all` returns an indexable `Array<T>` for int/number/bool/TypedObject (t09/t35/t39); still true for strings/other carriers, which instead **runtime-error** (t10) — a behavior the book doesn't mention either. |
| `join race` / `join any` examples with plain strings (`:75-101`) | ✅ those exact examples work (pre-resolved values settle in source order) — but that same mechanism produces wrong winners with mixed real branches (§9.2), undocumented. |
| `join settle` caution: v0.4 preview, returns TaskGroup summary (`:117-119`) | ✅ accurate (t13). |
| Named branches caution: "surfaces an unimplemented error at runtime" (`:132-134`) | ⚠️ Half-right: it surfaces a *confusing semantic error at compile time* ("Array\<int\> cannot have fields", t33), not a runtime unimplemented error. |
| "When scope exits, pending child tasks are cancelled deterministically" (`:153`) | ⚠️ Bookkeeping yes (t17/t28); actual work termination is lane-dependent — spawn_blocking children run to completion (§9.3). "Cancelled deterministically" overstates. |
| "`for await` currently works over ordinary collections. A real async stream protocol is still a future extension" (`:155`) | ✅ honest — and the type-level failure over future arrays (t16) makes "ordinary collections" the *only* thing it works over. |
| Annotated await → cookbook (`:169-171`) | ✅ machinery exists; no builtin policies (t31). |
| CLAUDE.md: "**Async**: `async let`, `await`, `async scope`, `for await x in stream { }`, `join all|race|any|settle { }`" | ⚠️ feature-list framing; "stream" does not exist as a concept anywhere in the runtime. |

Net: the book is *more honest than most priors suggested* (three explicit v0.4 cautions),
but it never mentions the three facts a user must know to write correct async Shape: the
deferral gate (what actually runs concurrently), the isolation boundary (module globals,
§9.1), and the join-race ordering hazard (§9.2).

---

## 9. Bugs & correctness risks found

### 9.1 [P0] Deferred async tasks read module globals as silent zeros — wrong results

```shape
use std::core::time
let GLOBAL = 100
async fn reads_global() -> int {
    await time::sleep(10.0)
    return GLOBAL + 1
}
async fn main() {
    async let t = reads_global()
    let r = await t
    print(f"global read from deferred task: {r}")
}
main()
```
```
=== t22_module_global ===
global read from deferred task: 1        <-- correct answer is 101
```

Root cause: `run_isolated_async_fn` builds a fresh VM and calls
`populate_module_objects()` but deliberately does not run the program's top-level code
("Module-binding initializers are NOT re-run ... A deferred async task therefore sees
module globals in their default (uninitialised) state", `async_runtime.rs:91-98`). An
uninitialized module binding slot reads as raw `0` bits with an Int64 kind, so
`GLOBAL + 1` computes `1` — **no error, no warning, wrong arithmetic**. The design doc
frames this as "the documented boundary of this lane", but the failure mode is silent
data corruption, not a boundary error. For a heap-typed global the same read would
produce Null/0-pointer bits with worse potential outcomes. The gate that decides
deferral (`advanced.rs:972-1010`) looks only at arity + return annotation; it cannot see
whether the body reads module state.

**Fix directions:** (a) compile-time: reject deferral when the callee's body references
any module-scope binding (the compiler already walks bodies for capture analysis);
(b) runtime: initialize module bindings in the isolated VM by running top-level
statements in a side-effect-free replay mode; (c) minimum: surface-and-stop on
uninitialized module-binding reads inside isolated VMs instead of returning zero bits.

### 9.2 [P1] `join race` returns the wrong winner with any non-deferred branch

```shape
async fn slow_heap() -> string { await time::sleep(1000.0)  return "slow-heap" }
async fn fast() -> int { await time::sleep(50.0)  return 1 }
...
let winner = await join race { slow_heap(), fast() }
```
```
=== t14_race_eager_heap ===
race winner=slow-heap elapsed=1003.747071    <-- 1000ms branch "beats" 50ms branch
```

Two compounding causes: (1) `compile_join_expr` evaluates non-deferrable branches
eagerly at spawn time (`misc.rs:791-795`) — `slow_heap()` runs its full 1000 ms body
before `fast()` is even spawned; (2) `join_race_first_settled` treats any
non-pending-async id as "settled" and returns the first such id in source order
(`async_ops/mod.rs:892-897`). The result is not merely "slower than expected" — it is
**semantically wrong** (`race` returns the losing value) and the failure is silent.
`join any` has the same structure (`:919-925`): an eagerly-failing branch's error can be
skipped fine, but an eager slow success in source-position 1 pre-empts a faster deferred
success. The book's own race example (two string literals) only works *because* of this
source-order rule.

### 9.3 [P1] Cancellation of deferred user tasks is cosmetic (spawn_blocking can't abort)

```shape
async fn loser() -> int { await time::sleep(500.0)  print("LOSER SIDE EFFECT RAN")  return 2 }
async fn fast()  -> int { await time::sleep(50.0)   return 1 }
...
let winner = await join race { loser(), fast() }   // then sleep 1500ms
```
```
=== t15_race_loser_abort ===
winner=1
LOSER SIDE EFFECT RAN        <-- printed AFTER the race returned and the loser was "cancelled"
after grace period
```

`op_spawn_task`'s UInt64 arm runs deferred user fns via
`shared_runtime().spawn_blocking(...)` and stores `handle.abort_handle()`
(`async_ops/mod.rs:466-481`). Tokio's `AbortHandle::abort()` on a **blocking** task is a
no-op once the closure has started running — so `scheduler.cancel()`
(`task_scheduler.rs:290-301`), race/any loser cancellation (`async_ops/mod.rs:672-676`),
`async scope` exit, and VM-teardown Drop (`:551-558`) all fail to stop in-flight deferred
tasks. Their side effects (prints, file writes — the isolated VM inherits the parent's
permission envelope, `async_ops/mod.rs:463`) land after cancellation. The code's own
comments claim the opposite: "abort lets race/any losers and async scope exit genuinely
cancel the underlying tokio task" (`task_scheduler.rs:69-71`), "WF-2D real cancellation"
(`:291-293`). Module-async tasks (`tokio::spawn`, `modules.rs:628-638`) *are* genuinely
abortable — the guarantee is silently lane-dependent.

### 9.4 [P1] Silent serialization outside the deferral gate

t06 (605 ms for two "concurrent" 300 ms tasks) + t07 (print order) — see §2.2. The
severity is not the serialization itself (a legitimate v1 restriction) but that it is
**indistinguishable at the surface** from the concurrent lane: same syntax, same types,
no diagnostic, and the book asserts concurrency. A `comptime warning` at the async-let
site when the RHS is non-deferrable would eliminate the trap cheaply.

### 9.5 [P1] Channel unusable end-to-end

```
=== t24_channel_send ===  Semantic error: Method 'send' not found on type 'Channel'
```

Constructor is registered polymorphically (`environment/mod.rs:1179-1182`); zero method
seeds in `method_table.rs` (the file's own FOLLOW-UP comment at `:1086` names the gap);
therefore every Channel program dies at semantic analysis and the entire VM-side
surface — `CHANNEL_METHODS` PHF (`method_registry.rs:618-625`), 245-line handler file,
`ChannelData` with interior mutability (`heap_value.rs:2862-2896`) — is dead. Even
post-fix, `recv()` on an empty channel is `NotImplemented` (`channel_methods.rs:154-162`)
and cross-task use is impossible (isolated heaps, §2.11), so the primitive's only
reachable semantics is a same-thread queue via `try_recv`.

### 9.6 [P1] `join all` carrier holes

t10: strings error at runtime ("supported homogeneous carriers are int, number, bool,
and typed object", `async_ops/mod.rs:810-815`). Arrays/maps/options/results likewise.
Because the deferral gate (§2.2) already forces heap-returning branches down the eager
path, the values *exist* at join time — only the aggregate carrier is missing. Users
combining `join all` with anything but numbers/bools/objects hit a runtime error the
type system did not predict (inference falls back to a fresh var for mixed/unproven
joins, `inference/expressions.rs:3029`, so it compiles).

### 9.7 [P1] `for await` type-level failure over futures

t16: `for await x in [future1, future2, ...]` — checker binds `x: Future`, arithmetic
fails ("Type 'Future' does not implement trait 'Numeric'"). Runtime would have awaited
each element correctly (`loops.rs:511-513`). One-line-concept fix: apply
`awaited_type_for` to the element type when `for_expr.is_async` in the inference walk
(no `is_async` handling exists in `type_system/` at all — grep). Until then the feature
only iterates plain collections, where `await` per element is a no-op.

### 9.8 [P1] Closure RHS in `async let` fails inference; masks &mut diagnostics

```
=== t27_closure_spawn ===  Semantic error: Generic type error: Could not infer generic type arguments for 'Future'
=== t27b (|| -> int {..}) === parse error (closure return-type annotation not accepted here)
=== t32_mut_boundary ===   same Future inference error (NOT the B0013 message)
```

`async let t = || expr` cannot be typed, so the scheduler's entire closure lane —
`op_spawn_task`'s `Ptr(HeapKind::Closure)` arm (`async_ops/mod.rs:498-501`),
`resolve_spawned_task`'s closure recovery + keepalive frame setup
(`call_convention.rs:517-572`), the `MakeClosureHeap` escape forcing
(`advanced.rs:915-917`) — is **unreachable from source** (⬛). Deferred-lazy execution
semantics (closure runs at await time, on the interpreter thread) are also arguably
surprising, but nobody can observe them today. Side effect: the purpose-built
`&mut`-across-task-boundary error (`advanced.rs:1103-1109`) and MIR B0013/B0014 are
masked by the earlier inference failure for the natural repro (t32).

### 9.9 [P2] `join settle` opaque carrier — book-disclosed (t13, §2.5)

### 9.10 [P2] Busy-wait polling in race/any

`join_race_first_settled` / `join_any_first_success` spin with
`thread::sleep(1ms)` between full scans (`async_ops/mod.rs:900-907`, `:934-947`), with
no timeout and no backoff. A hung branch (e.g. `time::sleep(very_large)`, or a deferred
task deadlocked on the blocking pool) spins the interpreter thread forever at ~1 kHz
polling. Combined with §2.8 (no await timeout), there is no way to bound any wait.

### 9.11 [P2] Blocking-pool exhaustion is a theoretical deadlock

Each deferred task occupies a blocking-pool worker for its entire body (inner awaits
block that worker). Tokio's default blocking pool caps at 512 threads; >512
simultaneously-pending deferred tasks (or deep nesting of deferred fns spawning deferred
fns, each holding a worker while awaiting a child) would deadlock/starve. Not reachable
in normal programs today (nested `async let` inside a deferred body works — the isolated
VM supports it), but unguarded.

### 9.12 [P2] `jit_cancel_task` extern-C `todo!()` process-abort landmine (§2.9)

Unreachable today only because `vm_only_opcode_reason` lists all async opcodes. Any
drift (e.g. someone removing `CancelTask` from the preflight list while wiring JIT
async) converts a missing-feature error into a SIGABRT with no Shape-level diagnostics.

### 9.13 [P2] EmitAlert/EmitEvent silently drop payloads

`op_emit_alert`/`op_emit_event` pop and release ("integration is deferred",
`async_ops/mod.rs:274-281`, `:1017-1024`). Unreachable from the compiler today; if any
surface ever emits them, events vanish without error — should be
`NotImplemented(SURFACE)` per house style instead of silent success.

### 9.14 [P2] Diagnostics papercuts

`join any` all-fail returns only the last error (t36); named-join access yields
"Generic {...} cannot have fields" (t33); top-level `async let` produces a correct error
followed by a cascade ("Undefined variable: x", t34).

### 9.15 [P1] Bare function references bypass the deferral gate — including sync fns

The compiler's deferral gate (`deferrable_async_call_target`, `advanced.rs:972-1010`)
only classifies `Expr::FunctionCall` RHS shapes. A **bare function reference** (no call
parens) compiles to a `Constant::Function` push whose slot kind is `UInt64`, and
`op_spawn_task`'s UInt64 arm (`async_ops/mod.rs:443-497`) unconditionally spawns it onto
the isolated VM — with none of the gate's protections:

```shape
async fn work() -> string { "heap-result" }
async fn main_task() {
  async let a = work        // bare ref, heap return — gate never consulted
  let v = await a
  print(v)
}
await main_task()
```
```
=== t35_bare_fnref ===
Error: Runtime error: Not implemented: WF-2D-fu isolated async task returned a non-scalar
result kind String; concurrent user async-fn tasks currently marshal only leaf scalars
(int/number/bool/unit) across the isolation boundary. Return a scalar or await the call
eagerly. (line 4)
```

The gate's stated purpose — "makes the isolation boundary's hard `NotImplemented`
unreachable for any previously-working program" (`advanced.rs:964-971`) — is therefore
false for the bare-ref spelling. Worse, a bare reference to a **sync** fn is also
spawned onto the isolated VM (the arm checks neither `is_async` nor arity at runtime):

```
=== t36_bare_sync_fnref ===   (fn sync_work() -> int { print("sync-body-ran") 9 })
sync-body-ran
9
=== t38_global_sync_fnref ===  (let factor = 7 ; fn scale() -> int { factor * 3 })
0                              <-- correct answer is 21: §9.1's silent-zero hole, sync-fn edition
```

So `async let a = f` and `async let a = f()` have wildly different semantics (isolated-VM
spawn vs. gated eager/deferred), and the bare-ref spelling re-opens both the heap-return
`NotImplemented` (loud, tolerable) and the §9.1 module-global silent-wrong-results hole
(silent, P0-class) for *any* zero-arg function, async or not. Fix: apply the same
deferral classification (or an outright compile error) to bare-fn-ref RHS shapes; the
runtime UInt64 arm should not be reachable for shapes the gate never examined.

### 9.16 [P2] `join all` over unit-returning tasks yields `[false, false]`

```shape
let r = await join all { time::sleep(50.0), time::sleep(60.0) }
print(r)
```
```
=== t30b ===
[false, false]
```

`time::sleep` returns unit (`ConcreteReturn::Unit`), which projects to the Bool-shaped
none-sentinel; `materialize_join_all_slots` (`async_ops/mod.rs:803-815`) then sees
`NativeKind::Bool` and happily builds a bool array — the internal unit sentinel leaks to
user space as `false` values. Harmless for typical fire-and-forget joins, but it is a
user-visible instance of the Bool-default shape the house style bans, and it makes
`join all` of unit tasks indistinguishable from a join of genuinely-false bools. Unit
results should materialize as unit/null (or the aggregate should be `unit`), not `false`.

---

## 10. What is done well

1. **The WF-2D architecture is an honest, sound answer to a hard constraint.** Given a
   `!Sync`, non-suspendable interpreter, overlap-at-the-wait-points via a dedicated
   multi-threaded runtime (`async_runtime.rs:26-35` — including the correct reasoning
   about why the ambient current-thread runtime can't be used, and mpsc-not-tokio for
   the completion channel so blocking `recv` never touches the ambient reactor) is a
   defensible v1. The isolation contract is explicitly documented with its exact
   soundness argument (`:85-101`).
2. **Refcount share discipline.** Every transfer between stack ↔ scheduler ↔ caller is
   accounted and commented; `Drop for TaskScheduler` releases retained shares; the
   double-completion defensive release (`task_scheduler.rs:278-287`) shows care for
   should-not-happen paths. I found no leak or double-free by reading all transfer sites.
3. **The snapshot × async boundary** (§2.12) is exemplary: a scheduler-aware preflight
   with a per-status diagnostic (`FutureSnapshotStatus`), a structured `Err(Barrier(..))`
   the program can observe and recover from, and a nested-field refusal in the generic
   codec — no silent state loss anywhere.
4. **The WF-2D-fu regression repair is the right shape**: rather than shipping heap
   returns that die at the marshal boundary, the compiler gate
   (`advanced.rs:956-971`) makes the hard `NotImplemented` unreachable while keeping the
   scalar win — a compile-time-proof-driven decision consistent with the language's
   philosophy (even though its invisibility creates §9.4).
5. **Surface-and-stop compliance under pressure.** Channel.recv-on-empty,
   is_sender, isolation marshal, jit stubs — all fail loudly with remediation text
   instead of fabricating defaults; `op_poll`'s Bool→Null sentinel fix is documented
   with its collision rationale (`async_ops/mod.rs:216-221`).
6. **Real timing-based integration tests** for overlap (join_strategies.rs measures
   wall-clock and asserts thresholds) — rare and valuable, and they would catch a
   regression to full serialization of the module-async lane.
7. **Remote async reuses the same scheduler lane** (`remote_builtins.rs:963-975`
   pending_async + cancellation hooks) instead of growing a parallel path — one
   completion/cancellation model across local and distributed futures.

---

## 11. What is done poorly / tech debt

1. **The dead parallel join implementation** (`TaskScheduler::resolve_task/resolve_task_group`,
   ~130 LOC + 4 tests) — zero callers, already semantically divergent from the live
   path, still green in CI, still cited by a docstring (`async_ops/mod.rs:605`). This is
   the exact drift pattern the project's own Forbidden-Patterns doctrine exists to
   prevent, sitting inside the scheduler.
2. **Comment/code divergence as a systemic style risk.** This territory's
   comment-to-code ratio is enormous, and at least four load-bearing comments are false
   on the working tree: "genuinely cancel the underlying tokio task"
   (`task_scheduler.rs:69-71` — false for spawn_blocking), "wrap the expression in a
   closure" (`misc.rs:742`), the `gc_integration.rs::scan_roots` reference
   (`task_scheduler.rs:38`), and the stale TDD header in `async_let.rs:11-16`. Wave-log
   archaeology (W7/W8/W10/W15/W17 citations) dominates files to the point where the
   current behavior is harder to see than its history.
3. **Concurrency semantics are signature-coupled and invisible** (§9.4): whether
   `async let` is concurrent depends on arity + return-annotation of the callee, with no
   surface marker, no diagnostic, and no book coverage. This will generate a steady
   stream of "async is slow/wrong" reports.
4. **The eager lane is load-bearing for join semantics** (§9.2): `race`/`any`
   correctness currently *requires* every branch to be deferrable. The design needs
   either closure-wrapping of non-deferrable branches (what the docstring already
   promises) or a compile-time refusal to mix lanes inside `join race`/`any`.
5. **Dead opcode surface** (9 of 15) with placeholder handlers that silently succeed
   (Poll/EmitAlert/EmitEvent) — should be deleted or converted to loud SURFACE errors.
   The suspension protocol (`AsyncExecutionResult::Suspended` → host) is effectively
   dead scaffolding for everything except the snapshot sentinel; `execution.rs:388-400`
   turns any real suspension reaching the host into a generic runtime error.
6. **No timeout/bounding primitive anywhere** (§2.8 + §9.10): unbounded `recv()` in
   `resolve_pending_async_task`, unbounded 1 ms spin in race/any, no builtin `@timeout`.
7. **Cross-task sharing story is absent while primitives suggest otherwise**: Mutex/
   Atomic/Channel exist (two of three usable same-thread) but no two concurrent Shape
   tasks can ever share them; ADR-006's SharedAtomic classes are unimplemented (§6.4).
   The surface implies a concurrency model the runtime cannot deliver — either implement
   the rendezvous or document the primitives as same-thread-only.
8. **Unit-test theater on the hot path** (§7.1): 10 enum-shape tests give the 1194-line
   opcode file a green checkmark while covering none of its behavior; the scheduler
   tests validate the dead API.

---

## 12. Prioritized recommendations

### P0 (correctness, do first)

1. **Close the module-global hole in deferred tasks** (§9.1, and its bare-fn-ref
   edition §9.15). Cheapest sound fix: compile-time — reject deferral (fall back to
   eager) when the callee body references any module-scope binding; the
   capture-analysis walk already visits these nodes. Apply the same classification to
   bare-fn-ref RHS shapes so the runtime UInt64 spawn arm is unreachable for ungated
   functions. Effort: S (1-2 days incl. tests). Alternative loud-fail at the
   uninitialized read: M.
2. **Fix `join race`/`any` winner semantics** (§9.2). Either (a) refuse to compile
   `join race|any` containing non-deferrable branches (surface-and-stop, matches house
   style), or (b) implement the docstring's closure-wrapping so every branch defers.
   (a): S. (b): M-L (depends on closure-spawn inference, rec 4).

### P1 (broken features)

3. **Make cancellation truthful** (§9.3). Options: cooperative cancellation flag checked
   at await points inside the isolated VM (M); or document lane-dependent behavior and
   fix the false comments + add a termination-asserting test (S, stopgap).
4. **Seed Channel methods in the checker** (§9.5) — mechanical, mirrors the Deque/Mutex
   seeding right above the FOLLOW-UP comment (`method_table.rs:990-1060`). S. Decide
   Channel's honest scope (same-thread queue) and document it.
5. **`for await` element typing** (§9.7): apply `awaited_type_for` under
   `for_expr.is_async`. S.
6. **`join all` string carrier** (§9.6): add the String materialization arm (and a
   generalized fallback erroring only on genuinely mixed carriers). S-M.
7. **Async-let closure inference** (§9.8): give `async let t = || e` the
   `Future<typeof e>` rule (the AsyncLet inference already exists — the failure is in
   the generic-arg resolution for the closure case). M. Unblocks the &mut diagnostics
   (t32) and the scheduler's closure lane.
8. **Warn on silent serialization** (§9.4): comptime warning when an `async let` RHS is
   non-deferrable. S. Update the book's async page in the same change (deferral gate,
   isolation boundary, race caveat, join-all carriers — §8).

### P2 (debt & hardening)

9. Delete `TaskScheduler::resolve_task/resolve_task_group` (+ retarget its 4 tests at
   `resolve_spawned_task`/`op_join_await` through eval-style harnesses). S.
10. Delete or SURFACE the 9 never-emitted opcodes and their placeholder handlers;
    replace `jit_cancel_task`'s `todo!()` with a returning error (extern-C-safe). S.
11. Add an await/join timeout primitive (builtin `@timeout` annotation lowering to a
    bounded recv/poll). M.
12. Fix the four false comments (§11.2), the stale codebase-index scheduler entry
    (§5.4), and the stale TDD header (§7.2). S.
12b. Materialize unit join-all results as unit/null instead of `false` (§9.16). S.
13. Behavioral unit tests for `op_await`/`op_spawn_task`/`op_join_await` arms (spawn →
    await → cached; join kinds; scope cancel; the three spawn lanes). M.
14. Decide the ADR-006 SharedAtomic question explicitly: either schedule the
    implementation behind a real shared-heap rendezvous design, or amend the ADR to
    scope v1 concurrency as shared-nothing. Doc-level: S.

---

*End of report. All findings verified against the working tree on 2026-07-11; all
transcripts reproduced from `target/debug/shape run` executions during this audit.*

