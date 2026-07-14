# Wave-40I Shape Destruction and Unwind Audit

Date: 2026-07-10

Scope: static inspection of ownership and borrow design, compiler drop insertion,
VM/JIT execution and teardown, heap/resource carriers, async cancellation,
focused tests, and shipped book claims. No production, test, book, script,
`CONTEXT.md`, or `AGENTS.md` file was changed. No build or test command was run.

## Decision

Shape has a useful but narrow Rust-like guarantee today: on bytecode paths that
reach a compiler-emitted lexical exit, synchronous user `Drop::drop` calls run in
reverse declaration order. This includes normal fallthrough and the compiler's
explicit `return`, `break`, and `continue` paths. A failing drop body is contained
so later drops still run. Native JIT refuses programs that register a user Drop
implementation, so these programs preserve behavior by interpreter fallback.

That is not a general unwind guarantee. Ordinary runtime failure, caught
exceptions crossing frames, Ctrl+C, parent-task cancellation, engine faults, and
abandonment of a suspended VM do not execute pending Shape `Drop` bodies. They
eventually release many Rust-owned heap shares when the affected slots or VM are
dropped, but raw refcount release is not user finalization, graceful close, task
quiescence, or rollback of an external effect. A process abort cannot provide
deterministic cleanup at all.

The book's normal lexical-exit and source `?` propagation examples are supported.
Its claims that async drops are awaited and that pending children are cancelled
deterministically are broader than the implementation.

## Terms and Current Outcome Surface

The following operations must not be conflated:

- **Release** retires one VM/Rust-owned heap share using the slot's proven
  `NativeKind`. It is memory ownership bookkeeping.
- **Finalize** executes source-visible `Drop::drop` or `drop_async` code.
- **Close** performs a protocol or OS resource's explicit graceful shutdown.
- **Cancel request** asks work to stop. It is not proof that work has stopped.
- **Quiesce** observes that child/provider work has stopped and can no longer use
  parent resources.
- **Process reclamation** is what the OS does after process loss; it cannot run
  Shape code, commit/rollback a remote transaction, or stop remote work.

The public VM result is only `ExecutionResult::{Completed, Suspended}`
(`crates/shape-vm/src/executor/mod.rs:121-141`). `VMError` mixes user/runtime
errors with `Suspended`, `Interrupted`, and `ResumeRequested` control signals
(`crates/shape-value/src/context.rs:42-101`). Therefore `Failed`, `Cancelled`,
and `Faulted` below are audit terms, not current Rust enum variants:

```text
Evaluation<R> =
    Completed(R)
  | Failed(RuntimeFailure)
  | Cancelled(CancelReason)
  | Suspended(Continuation<R>)
  | Faulted(EngineFault)
```

`Evaluation<R>` is a host/evaluator concept, not a Shape source return type.

## What Is Implemented

### Ownership and normal lexical exits

The borrow RFC says dropping with active loans is illegal, reverse-order drop is
preserved, and `return`/`break`/`continue` emit drops after borrow validation
(`docs/vision/rfc-borrow-lifetimes-ergonomics-v1.md:149-153`). This is enforced by
explicit bytecode generation, not a runtime unwind mechanism:

ADR-006 separately specifies exact kind-dispatched heap release
(`docs/adr/006-value-and-memory-model.md:509-567`) and defers user Drop when an
escaping reference extends the referent's lifetime
(`docs/adr/006-value-and-memory-model.md:7476-7500`). These are ownership/lifetime
rules; neither clause defines abnormal evaluator unwind.

- `pop_drop_scope` emits closure-capture discharge, ownership/share release, and
  user `DropCall` instructions in reverse order
  (`crates/shape-vm/src/compiler/helpers.rs:5834-5928`).
- `emit_drops_for_early_exit` emits the corresponding instructions for every
  exited scope and skips a returned-by-value local whose ownership moves to the
  caller (`crates/shape-vm/src/compiler/helpers.rs:6264-6403`).
- Source `?` uses `IsTryFailure` to guard that same early-exit drop sequence, so
  only the Err/None branch finalizes pending locals before `TryUnwrap` propagates
  (`crates/shape-vm/src/compiler/expressions/advanced.rs:32-106`). This is
  compiler-covered source control flow, not generic runtime-error unwind.
- A normal function return pops the frame, truncates its stack window, and
  releases the frame's closure keepalive
  (`crates/shape-vm/src/executor/control_flow/mod.rs:1211-1321` and
  `crates/shape-vm/src/executor/vm_impl/stack.rs:1314-1335`).
- `DropCall` invokes a registered user method or otherwise performs only kinded
  release. A drop-body error triggers a focused frame unwind, is recorded, and
  returns `Ok(())`, preserving the original value and later drops
  (`crates/shape-vm/src/executor/trait_object_ops.rs:682-876`).

There is no `defer`, `finally`, `using`-resource, or finalizer statement in the
language grammar. The only source cleanup mechanism found is ordinary bindings
plus builtin-trait syntax such as:

```shape
impl Drop for Handle {
  method drop() { close(self) }
}
```

`using` in the grammar is an implementation selector/join term, not a resource
scope (`crates/shape-ast/src/shape.pest:750,1112-1113`).

### Raw heap release

`KindedSlot::Drop` dispatches on exact `NativeKind`; scalars are no-ops and each
heap kind retires its matching typed share. It has dedicated arms for IoHandle,
Closure, Future, and SharedCell (`crates/shape-value/src/kinded_slot.rs:947-1335`).
The VM mirrors this in `drop_with_kind` and uses it on stack truncation, frame
return, catch-stack truncation, and VM teardown.

`VirtualMachine::drop` releases shared module bindings, every live stack slot,
and every module-binding slot whose bits/kind tracks remain lockstep
(`crates/shape-vm/src/executor/mod.rs:690-848`). This is an important eventual
memory-release backstop. It explicitly leaks an unmatched module-binding tail in
release builds rather than inventing a kind
(`crates/shape-vm/src/executor/mod.rs:806-847`), so even raw release is
conditional on the metadata invariant.

Reference counting does not finalize cycles. With the `gc` feature, cycle
collection is deliberately memory-only and skips user Drop
(`docs/design/real-gc-cycle-collection.md:14-26,778-816`; implementation proof at
`crates/shape-value/src/gc.rs:1812-1885`). Without collection, such cycles remain
retained. Neither mode gives Rust-like finalization for a cyclic resource graph.

### JIT behavior

Native JIT `emit_drop` only releases heap shares and nulls slots. The JIT rejects
the whole program when `trait_method_symbols` contains a user Drop implementation
(`crates/shape-jit/src/executor.rs:612-654`; raw lowering at
`crates/shape-jit/src/mir_compiler/ownership.rs:1012-1077`). The normal user-Drop
contract is therefore interpreter behavior reached through deoptimization, not
native JIT cleanup support.

JIT runtime errors are negative signals. The pending-call-error path explicitly
abandons the JIT frame and surfaces the VM error without rerunning side effects
(`crates/shape-jit/src/executor.rs:960-1043`). There are no JIT cleanup landing
pads or an unwind plan for heap locals on those early signal returns. Native JIT
failure cleanup is therefore not proven equivalent even for types without user
Drop.

### Exceptions and ordinary failure

The top-level dispatch loop returns an uncaught instruction error immediately
(`crates/shape-vm/src/executor/dispatch.rs:248-302`). It does not walk compiler
drop scopes or execute pending `DropCall`s. `unwind_call_frames_to` exists, but
repository callers are limited to drop-body containment and one immediate
value-call failure (`crates/shape-vm/src/executor/call_convention.rs:994-1015,
1159-1164`); it is not the evaluator's failure path.

A catch handler raw-drops stack slots to the saved stack depth, truncates the
call-frame vector, and jumps to the catch block
(`crates/shape-vm/src/executor/exceptions/mod.rs:132-162`). It does not execute
user drops. `CallFrame` owns a separate closure keepalive share that normal
return releases, but has no Rust `Drop` implementation
(`crates/shape-vm/src/executor/mod.rs:202-257`). Consequently, cross-frame
`call_stack.truncate` also omits that keepalive release. This is a concrete raw
share leak in addition to the missing source finalization.

## Outcome Matrix

| Audit outcome | User `Drop` today | Heap/share behavior | Tasks and external effects | Precise guarantee |
|---|---|---|---|---|
| **Completed** | Yes for reached lexical exits in VM bytecode, reverse order; drop errors are contained. User-Drop programs deopt from native JIT. | Frame/stack/module shares are kind-released; last-owner Rust destructors can run. Aliases delay destruction; cycles do not run user Drop. | Normal `AsyncScopeExit` issues LIFO cancellation after body-local drops. It neither joins children nor proves remote/provider completion. | Deterministic synchronous finalization only along compiler-covered control flow. |
| **Failed** | No general unwind. Compiler-lowered source `?` runs guarded DropCalls, but an arbitrary uncaught `VMError`, and even a caught exception crossing scopes, bypasses them. | The still-live VM retains state until host teardown; VM Drop later releases most raw shares, subject to kind metadata and frame-keepalive gaps. | No automatic graceful close, cancellation acknowledgement, or rollback. | One explicit propagation path has cleanup; evaluator failures do not have a total guarantee. |
| **Cancelled** | No evaluator-level Cancelled outcome and no user-Drop unwind. Ctrl+C is `VMError::Interrupted`. | Eventual VM destruction releases raw shares; an interrupt snapshot may intentionally preserve a resumable state. | Explicit task/scope cancellation invokes hooks and abort handles, but does not await quiescence. | Cancellation is a request/status transition, not deterministic cleanup. |
| **Suspended** | None, intentionally: live frames must remain resumable. | Stack, closure cells, handles, and scheduler state stay owned by the continuation/VM. Dropping the VM later performs raw teardown, not pending user Drop. | Work may remain pending. Live scheduler futures are not snapshot-restorable. | Preservation while live; no cleanup until completion or abandonment. |
| **Faulted** | No `EngineFault` channel and no Shape unwind. | A Rust panic that unwinds through the VM may run Rust destructors, including VM raw teardown. This is not source Drop and is not guaranteed across FFI UB or abort. | External work may continue. | No semantic cleanup guarantee. |

## Value and Resource Classes

### Shape-owned heap values

Strings, arrays, objects, maps, and other typed heap carriers have exact
retain/release dispatch. Their guarantee is lifetime of a share, not lexical
destruction of the allocation: aliases can outlive a binding, and cyclic graphs
either leak or are reclaimed without user finalizers. This is enough for many
memory-safety paths, but not enough for a type whose Drop body represents an
external obligation.

### Closures and shared cells

Closure blocks and SharedCells have matching kind-aware release paths.
Drop-bearing captures that escape are specially deferred and discharged once by
`DropClosureCaptures`
(`crates/shape-vm/src/compiler/helpers.rs:5837-5848,5914-5926` and
`crates/shape-vm/src/executor/trait_object_ops.rs:879-945`). This handles focused
normal escape patterns. It is not a substitute for evaluator unwind: failure can
skip the consumer-side opcode, exception frame truncation omits closure
keepalive cleanup, and closure/shared-cell cycles are memory-only GC territory.

### FFI and OS resources

`IoHandleData` wraps files, sockets, child processes, pipes, and custom payloads
in `Arc<Mutex<Option<IoResource>>>`; explicit `close()` takes the resource and
last-owner Rust Drop otherwise drops it
(`crates/shape-value/src/heap_value.rs:330-394,495-517`). Therefore:

- Explicit close is prompt and shared across aliases.
- Automatic OS-handle close occurs only when the last Arc-backed owner is raw-
  released and the Rust value is actually dropped.
- User Drop skipped by failure does not necessarily leak the file descriptor if
  the final Rust owner is later released, but it can skip flush, protocol close,
  transaction rollback, pool return, child termination/wait, or other semantic
  work.
- A custom payload receives only its Rust `Box` destructor. Shape cannot infer a
  provider-specific asynchronous close protocol.

The native C ABI intentionally exposes explicit pointer carriers/manual pointer
passing and identifies lifetime/ownership as a risk
(`docs/adr/004-native-c-interop.md:149-155,212-219`). It has no general owned-
handle descriptor with clone/drop/async-close callbacks.

`NativeViewData` is likewise a non-owning pointer-backed view: it stores a raw
address, an owned layout descriptor, and mutability, but no pointee owner or
destructor (`crates/shape-value/src/heap_value.rs:322-328`). Dropping its slot
releases the view/layout Arc, not the referenced native allocation.

Plugin coverage is inconsistent. `LanguageRuntimeState::drop` calls the plugin
instance destructor and `PluginOutputSink::drop` flushes then calls its vtable
drop (`crates/shape-runtime/src/plugins/language_runtime.rs:24-39` and
`crates/shape-runtime/src/plugins/output_sink.rs:156-163`). By contrast,
`CompiledForeignFunction` is a cloneable raw handle with no Drop; disposal is an
explicit method and repository search found no caller
(`crates/shape-runtime/src/plugins/language_runtime.rs:11-22,360-367`).

### Provider and transport handles

The current transport provider is a global `Arc<dyn WireTransportProvider>`
(`crates/shape-vm/src/executor/builtins/transport_provider.rs:11-36`). Transport
and connection objects are type-erased into `IoResource::Custom`; `Connection`
offers explicit `close`, and the builtin calls it only when source requests
`transport.connection_close`
(`crates/shape-vm/src/executor/builtins/transport_builtins.rs:53-66,422-469` and
`crates/shape-wire/src/transport/mod.rs:43-52`). Dropping the box can close the
underlying socket through its Rust implementation, but graceful provider/session
shutdown is not a host-enforced lifecycle.

Opaque typed destinations or placements should remain inert capabilities, not
addresses with an assumed encoding and not cleanup tokens. Live provider/session
handles are separate host-owned resources. The proposed asynchronous
`provider.shutdown(deadline)` and `session.close()` boundary in
`docs/cluster-audits/wave40-remoting-provider-interface.md:283-316` is design,
not current implementation. It will need explicit close/drain semantics, bounded
deadlines, cancellation acknowledgement where supported, and leases for process
loss; generic annotation failure hooks must remain provider-neutral.

### Process abort and host loss

No language can promise destructors after `SIGKILL`, OOM kill, power loss,
segfault, abort-mode panic, or host disappearance. The OS reclaims local address
space and closes process-owned descriptors, but does not guarantee child-process
termination, peer-observed protocol close, remote cancellation, lock release
outside kernel ownership, or transaction resolution. Those obligations require
leases, fencing, idempotency/deduplication, and transactional protocols above the
process, not a stronger Drop claim.

## Async and Structured Cancellation

`Future` is an inline task ID; cloning or dropping it is a no-op. Cancellation
requires `CancelTask` or scope tracking
(`crates/shape-value/src/kinded_slot.rs:1293-1300,1716-1720` and
`crates/shape-vm/src/executor/async_ops/mod.rs:954-965`).

On a normal `async scope` path, the compiler emits Enter, body, Exit, and the
runtime cancels tracked tasks in LIFO order
(`crates/shape-vm/src/compiler/expressions/advanced.rs:1012-1049` and
`crates/shape-vm/src/executor/async_ops/mod.rs:977-1010`). Return, runtime
failure, interruption, or fault before `AsyncScopeExit` skips that opcode. VM
teardown's scheduler Drop is only a fallback: it releases stored heap shares,
runs best-effort hooks, and calls abort handles without joining
(`crates/shape-vm/src/executor/task_scheduler.rs:531-563`).

There are two further hard limits:

1. User async functions run with Tokio `spawn_blocking`
   (`crates/shape-vm/src/executor/async_ops/mod.rs:443-480`). Tokio cannot abort a
   running blocking task; only a task not yet started can be prevented. The
   isolated VM and its effects can continue after the parent reports
   cancellation.
2. Remote cancellation hooks spawn an unawaited best-effort `CancelCall`
   (`crates/shape-vm/src/executor/builtins/remote_builtins.rs:965-974,1042-1054`).
   Real-socket tests explicitly distinguish queued cancellation from a running
   receiver's `AlreadyRunning` result
   (`bin/shape-cli/tests/distributed_async_cancellation_e2e.rs:277-345`).

`DropCallAsync` chooses `drop_async`, but executes it through
`execute_until_call_depth`. A genuine suspension is returned as an error by that
driver (`crates/shape-vm/src/executor/dispatch.rs:581-651`) and then contained as
a failed drop. Thus an async drop that finishes synchronously can run; there is
no general guarantee that an async cleanup future is awaited to completion.

The normal compiler ordering is also unsafe as a general structured-concurrency
rule: body-local drops are emitted while compiling the body, before the trailing
`AsyncScopeExit`. Children are not first quiesced, so they can still observe a
resource while its cleanup runs. The book describes exactly this order but calls
it a guarantee; it is at most current normal-path ordering.

## Tests and Book Truth

Strong focused coverage exists for the normal VM lane:

- `tools/shape-test/tests/drop_raii/{scope_drop,control_flow,ordering}.rs`
  covers block/function exits, return, break, continue, loops, and LIFO order.
- `crates/shape-vm/src/executor/tests/auto_drop.rs:284-759` runs user-drop
  receivers, reverse order, escape deferral, sync/async variant selection, and
  contained drop errors.
- `crates/shape-vm/src/executor/tests/drop_deep_tests.rs:2151-2395` is mostly
  opcode-selection/emission coverage for async drop, not proof that a suspending
  cleanup is awaited.
- `crates/shape-vm/src/executor/vm_impl/stack.rs:1560-1568` proves raw stack
  truncation releases a share.

Missing focused regressions are more important than additional normal-exit
examples: a user-Drop local followed by uncaught failure; caught failure crossing
one and several call frames; closure keepalive counts across catch; interrupt and
parent cancellation; an actually suspending `drop_async`; native-JIT heap locals
on every negative signal; FFI owned-handle disposal; provider close/drain; and
process-loss lease behavior.

The shipped book is accurate about ordinary lexical drop, reverse order, and
source `?` propagation cleanup, and drop-error containment
(`../shape-web/book/book-site/src/content/docs/fundamentals/resource-management.mdx:8-12,58-99,219-253`).
It overstates implementation at these rows:

- Async drops are said to be awaited
  (`../shape-web/book/book-site/src/content/docs/fundamentals/resource-management.mdx:312-380`).
- Async-scope children are said to be cancelled deterministically
  (`../shape-web/book/book-site/src/content/docs/fundamentals/async.mdx:138-153`).

Those claims should not be used as implementation contracts until the unwind and
quiescence work below lands.

## Smallest Coherent Follow-up

1. Introduce one internal `Evaluation<R>` boundary with distinct
   `RuntimeFailure`, `Cancelled`, `Suspended`, and `EngineFault`; do not expose it
   as a Shape `Result` or alter source return type `R`.
2. Build one VM unwind stack from initialized/live/moved compiler facts. On
   `Completed`, recoverable `Failed`, and `Cancelled`, walk it once in LIFO order,
   release every raw share, and run eligible synchronous Shape drops while
   preserving the primary outcome and collecting cleanup failures.
3. Keep `Suspended` state intact. On explicit abandonment, use the same
   cancellation/unwind driver. On `Faulted`, run only host-level cleanup proven
   safe under damaged engine invariants; do not execute arbitrary Shape code.
4. Make structured scope exit total: request child cancellation, await bounded
   quiescence, then finalize resources. Represent unknown remote outcome and
   non-preemptible work honestly rather than reporting successful cancellation.
5. Treat asynchronous close as a separate host-owned protocol with a deadline
   and cancellation shield. Do not pretend ordinary Rust/Shape Drop can await.
6. Give FFI/provider owned handles explicit clone/release/close contracts.
   Provider routing, destination encoding, transport, auth, codec, negotiation,
   deadlines, cancellation, and observability remain behind provider interfaces;
   signature, certainty, idempotency, and deduplication policy stay above them.
7. Add the failure/cancellation tests listed above, then narrow the two book
   overclaims until their corresponding gates pass.

This is the minimum architectural unit. Adding `defer` syntax or more DropCall
insertion before a total evaluator unwind would only create another cleanup form
that abnormal exits can skip.
