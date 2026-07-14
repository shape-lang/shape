# Wave 40B: VM Runtime Failure Channel Model

Date: 2026-07-10

## Decision

A Shape function's declared return type describes only normal completion. For a
function `f: P -> R`, `R` is the type of the value produced when `f` returns; it
is not the type of every way an evaluator invocation can end.

Use one structured evaluator outcome model:

```text
Evaluation<R> =
    Completed(R)
  | Suspended(Suspension)
  | Failed(RuntimeFailure)
  | Cancelled(Cancellation)
  | Faulted(EngineFault)
```

`Completed`, `Failed`, `Cancelled`, and `Faulted` are terminal for that
invocation. `Suspended` is resumable and must retain enough state to continue.
An uncontainable process abort is outside this in-process type; isolation is
needed to turn it into `Faulted`.

This is an evaluator/host type, not a Shape source type. In particular:

```text
@remote fn f(p: P) -> R

source call type:      P -> R
evaluator observation: Evaluation<R>

remote success:        Completed(r)
remote failure:        Failed(RuntimeFailure {
                           kind: RuntimeFailureKind::Remote(remote_kind),
                           diagnostic,
                       })
```

No `R` exists on the failure path. Therefore `@remote` can preserve `R` and
still fail non-returningly without a typed exception system and without
changing the function to `Result<R, RemoteError>`.

The explicit recoverable primitive remains different:

```text
remote::call(...)       evaluates as Evaluation<Result<R, RemoteError>>
remote::call_async(...) evaluates as Evaluation<Future<Result<R, RemoteError>>>
```

Here `Err(RemoteError)` is an ordinary value inside `Completed(...)`. The
evaluator may still independently produce `Failed`, `Cancelled`, or `Faulted`
for machinery that cannot construct the promised Shape value.

## Canonical Terms

Use these terms consistently:

* **Return type `R`**: the Shape type declared by a function. It constrains only
  a normal return.
* **Completion value**: the value `R` carried by `Completed(R)`. A Shape
  `Result::Err(e)` is a completion value when `R` itself is `Result<T, E>`.
* **Evaluation**: the complete sum of observable evaluator outcomes for one
  invocation.
* **Runtime failure**: a structured, non-returning failure encountered while
  executing valid code, such as division by zero, permission refusal, resource
  exhaustion, failed builtin dispatch, or remote transport/receiver failure.
  It is not a Shape value and is not catchable by implication.
* **Suspension**: a non-terminal evaluator outcome with a continuation and wait
  reason. It is not an error.
* **Cancellation**: terminal withdrawal of the invocation by its owner or
  structured-concurrency scope. Root cancellation is an evaluator outcome;
  child cancellation may be consumed by a concurrency operator.
* **Engine fault**: an invariant violation or contained panic in Shape's
  implementation, JIT, marshal layer, or extension adapter. It must not be
  presented as a user program error.
* **Diagnostic**: structured presentation data attached to a failure or fault:
  stable code, message, source location, origin, and typed details.
* **Host projection**: an adapter's mapping from `Evaluation<R>` to its external
  interface, such as CLI text and exit status, a server response, or an
  embedding result.

Avoid the phrases "error result" and "typed raise" without qualification.
They conflate a completed Shape `Result` value with a non-returning evaluator
failure. In current `@remote` documentation, "raise" should be read only as
"the evaluator returns `Failed`", not as a catchable Shape exception.

## Current Evaluator Path

### VM outcomes

The VM already contains most of the desired sum, but splits it across two
types. `shape-vm`'s `ExecutionResult` has `Completed(KindedSlot)` and
`Suspended { future_id, resume_ip }` at
`crates/shape-vm/src/executor/mod.rs:121-141`.
`VirtualMachine::execute_with_suspend` returns
`Result<ExecutionResult, VMError>` at
`crates/shape-vm/src/executor/dispatch.rs:84-105`. The simpler `execute`
interface immediately converts `Suspended` back into `Err(VMError::Suspended)`
at `:20-43`.

`VMError` then mixes at least three categories in one enum at
`crates/shape-value/src/context.rs:42-101`:

* user/runtime failures such as division by zero, bounds errors, invalid
  arguments, and `RuntimeError(String)`;
* VM integrity or implementation states such as stack underflow,
  `InvalidOperand`, and `NotImplemented`; and
* control outcomes explicitly documented as not real errors: `Suspended`,
  `Interrupted`, and `ResumeRequested`.

The dispatch loop partially repairs that mixture. It converts a suspension to
`ExecutionResult::Suspended`, consumes snapshot suspension internally, consumes
`ResumeRequested` internally, and propagates interruption at
`crates/shape-vm/src/executor/dispatch.rs:248-301`. However, the public
`execute()` projection and several host callers collapse suspension back into
the error channel.

Error enrichment also destroys useful structure. Every located VM error except
the three control signals is formatted and replaced by
`VMError::RuntimeError(String)` at
`crates/shape-vm/src/executor/dispatch.rs:1177-1249`. The original variant and
typed location are no longer available to later adapters.

### Builtin and Shape `Result` propagation

Native module bodies expose exactly the semantic distinction this model needs,
but only by convention. Their outer Rust return is
`Result<TypedReturn, String>`, while `TypedReturn::Ok` and
`TypedReturn::Err` are Shape values at
`crates/shape-runtime/src/typed_module_exports.rs:184-223`.

`invoke_module_fn_id_stub` maps an outer body `Err(String)` to
`VMError::RuntimeError`, then projects a successful `TypedReturn`, including
its `Err` value arm, to a `KindedSlot` at
`crates/shape-vm/src/executor/vm_impl/modules.rs:797-929`. Thus:

```text
Ok(TypedReturn::Err(payload)) = Completed(Shape Result::Err(payload))
Err(message)                  = Failed(runtime failure)
```

The distinction is correct. The defect is that the outer failure is only a
string, so its category, origin, and structured details are lost.

The remote module deliberately uses both forms. `RemoteDispatcher::call_remote`
uses the outer failure for `@remote`, while `call_remote_result` builds a Shape
Result value at `crates/shape-runtime/src/module_exports.rs:100-161`.
The raising adapter maps a failed `RemoteCallResponse` to `Err(String)` at
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:878-909`; the
recoverable adapter maps it to `TypedReturn::Err(RemoteError)` at `:847-867`.
The public native bodies preserve that split at `:1749-1841`.

### JIT and FFI propagation

The JIT has a second out-of-band failure implementation. A VM trampoline writes
an error message to thread-local `JIT_RUNTIME_ERROR`, sets
`JITContext.pending_call_error`, and makes generated code return a negative
signal at `crates/shape-jit/src/ffi/control/mod.rs:29-72` and
`crates/shape-jit/src/context.rs:95-108,660-669`. The executor takes and clears
the message, then constructs `ShapeError::RuntimeError` at
`crates/shape-jit/src/executor.rs:1015-1043`.

This design has two sound properties worth preserving: placeholder value bits
never become a completion value, and the error is drained exactly once. The
focused tests at `crates/shape-jit/src/ffi/control/mod.rs:1095-1123` prove the
second property. The channel nevertheless stores only `String`, and the flag,
thread-local payload, negative signal, nested Rust `Result`, and final
`ShapeError` are parallel representations of one semantic outcome.

`JITExecutor::execute_with_jit` uses
`Result<Result<ProgramExecutorResult, ShapeError>, ShapeError>` to distinguish
JIT pipeline failure from a runtime failure after side effects at
`crates/shape-jit/src/executor.rs:251-262,305-327`. That distinction prevents an
unsafe interpreter rerun, but the nested type is local to one adapter rather
than the common evaluator interface.

Panic treatment is also local. JIT compilation catches a panic and formats it
as a runtime error at `crates/shape-jit/src/executor.rs:698-720`. Dynamic
foreign dispatch catches host-side marshal panics and also formats them as
`VMError::RuntimeError` at
`crates/shape-vm/src/executor/control_flow/mod.rs:1042-1065`. General VM
execution has no corresponding panic projection, and invariant `unreachable!`
or `todo!` paths remain process-level faults if reached. A contained panic is
an `EngineFault`; classifying it as the user's `RuntimeFailure` is misleading.

### Host and CLI projection

The cross-crate `ProgramExecutor` interface returns
`Result<ProgramExecutorResult, ShapeError>` at
`crates/shape-runtime/src/engine/mod.rs:59-76`. `ShapeEngine` uses `?` for parse,
prefetch, compile, and execute phases, then constructs a success-only runtime
`ExecutionResult` at
`crates/shape-runtime/src/engine/execution.rs:58-122`. There are also two
different public types named `ExecutionResult`: the VM completed/suspended sum
and the runtime success envelope (`crates/shape-runtime/src/engine/types.rs:17-42`).

`BytecodeExecutor` converts every VM error except interruption into
`ShapeError::RuntimeError { message, location: None }` at
`crates/shape-vm/src/execution.rs:883-932`. A structured uncaught `AnyError`
payload is stored separately in `Runtime::last_runtime_error`, then consumed by
the CLI (`crates/shape-runtime/src/lib.rs:184-187,286-303`). This side channel
can drift from the returned error and is not available through the ordinary
executor result.

The CLI does preserve one distinction: `ShapeError::Interrupted` exits 130,
while generic execution failures render a diagnostic and exit 1 at
`bin/shape-cli/src/commands/script_cmd.rs:460-487,1498-1528`. JSON diagnostics
otherwise flatten every `ShapeError::RuntimeError` to the `RUNTIME` id at
`bin/shape-cli/src/diagnostics_json.rs:59-79`.

### Cancellation

Cancellation exists as a real scheduler state. `TaskStatus::Cancelled` and
`FutureSnapshotStatus::Cancelled` are explicit at
`crates/shape-vm/src/executor/task_scheduler.rs:85-121`, and cancellation aborts
pending async work and runs a best-effort remote hook at `:289-309,531-559`.
But resolving a cancelled task returns `VMError::RuntimeError("Task ... was
cancelled")`, and a closed remote completion channel returns another runtime
error string at `:340-421`.

Remote cancellation is similarly structured on the wire
(`RemoteCancelOutcome` at `crates/shape-vm/src/remote.rs:136-162`) but a queued
call cancelled before execution is returned as wire `RuntimeError` at
`bin/shape-cli/src/commands/serve_cmd.rs:1280-1290`. Cancellation is therefore
modeled accurately inside the scheduler and protocol, then erased at the
evaluator seam.

## Current Inconsistencies

1. `Result` names both a completed Shape value and multiple Rust transport
   types; nested Rust Results encode undocumented local distinctions.
2. `VMError` combines runtime failure, evaluator control, unsupported features,
   and probable engine faults.
3. Source enrichment replaces structured VM variants with one formatted
   `RuntimeError(String)` instead of attaching location data.
4. Native builtins correctly separate outer failure from returned Shape `Err`,
   but the outer `String` erases the reason and origin.
5. JIT signals, a pending flag, a thread-local string, and `ShapeError` duplicate
   one semantic failure channel.
6. The host result carries only success while uncaught structured error data is
   stored in mutable runtime state as a side channel.
7. Scheduler and wire cancellation discriminants collapse to runtime strings
   when they reach evaluation.
8. Some contained implementation panics are labeled user runtime errors, while
   other panics or aborts bypass the evaluator interface entirely.
9. Dormant VM handlers can unwind any `VMError`, although the source doctrine
   exposes no catchable runtime-failure contract.
10. Two unrelated types named `ExecutionResult` describe a VM step outcome and
    a host success envelope, respectively.

## Doctrine and ADR Fit

The source-language doctrine is explicit: "Result types for errors: No
try/catch/throw" in `docs/vision/implementation-plan.md:18-26` and
`docs/vision/distributed-comptime-async-vision.md:242-248`. `@remote`'s Q26
ruling preserves `R`, uses a non-returning runtime failure, and reserves
recoverable `RemoteError` values for `remote::call` in
`docs/design/00-priority-spine-overview.md:170-178` and
`docs/design/distributed-function-transfer.md:198-202`.

ADR-006 defines the typed carrier for completed values and ratifies builtin
interfaces such as `Result<KindedSlot, VMError>` at
`docs/adr/006-value-and-memory-model.md:1916-1927`. It does not define what
`VMError` means, distinguish cancellation from failure, or specify panic and
host projection. None of the accepted ADRs currently owns that model. A small
execution-outcome ADR should therefore precede a public Rust interface change;
it would complement ADR-006 rather than amend its value-carrier rules.

The VM still contains latent exception-handler machinery. If a handler exists,
dispatch currently converts any `VMError` into `AnyError` and unwinds at
`crates/shape-vm/src/executor/dispatch.rs:280-301,471-485`. No compiler path
currently emits `SetupTry`; the opcode has only executor consumers. Internal
`Throw` is used for failed pattern matches and top-level `?`, but this does not
establish a source-level catch contract. The evaluator model must not make
runtime failures catchable merely because this legacy machinery exists.

## Required Invariants

1. `R` constrains only `Completed(R)`. No other outcome fabricates a value of
   `R`, a null sentinel, or a default kind.
2. A completed Shape `Result::Err(e)` remains `Completed(Result::Err(e))`; it is
   never promoted to `Failed` merely because its constructor is named `Err`.
3. A builtin's outer failure becomes `Failed`; its returned
   `TypedReturn::Err` remains a completion value. This rule is uniform in VM,
   JIT, FFI, remote, and async adapters.
4. `Suspended` and `Cancelled` are not failures. `ResumeRequested` is private
   VM control flow and must never escape the dispatch implementation.
5. `RuntimeFailure` is structured before formatting. Source enrichment adds a
   location without replacing its discriminant or code.
6. A caught panic or invariant violation becomes `Faulted(EngineFault)`, not a
   user runtime failure. Hosts may sanitize its display, but must retain an
   operational code or incident identity.
7. JIT and VM adapters produce equivalent semantic outcomes. A JIT failure
   after observable side effects never triggers interpreter re-execution.
8. Side channels may carry telemetry, but never the authoritative failure. The
   outcome owns any structured `AnyError` payload needed by a host renderer.
9. Root interruption/cancellation has an explicit host projection. CLI exit
   130 for user interrupt and exit 1 for runtime failure remain stable unless a
   separately versioned CLI decision changes them.
10. `@remote` maps its structured wire failure into a `RuntimeFailure` whose
    kind is `Remote`; it does not construct a Shape `RemoteError` value.
    `remote::call` performs the opposite projection inside
    `Completed(Result<...>)`.

## Deep Module and Seam

The evaluator outcome should be one deep module with a small interface, not a
new wrapper at every call site. The natural seam is the execution interface
shared by VM and JIT adapters. Its conceptual Rust shape is:

```rust
pub enum Evaluation<R> {
    Completed(R),
    Suspended(Suspension),
    Failed(RuntimeFailure),
    Cancelled(Cancellation),
    Faulted(EngineFault),
}

pub struct RuntimeFailure {
    pub kind: RuntimeFailureKind,
    pub diagnostic: RuntimeDiagnostic,
}

pub enum RuntimeFailureKind {
    Program(ProgramFailureCode),
    Builtin { module: String, function: String },
    Permission(PermissionFailure),
    ResourceLimit(ResourceLimitFailure),
    Remote(RemoteFailureKind),
    Foreign(ForeignFailureKind),
    Unsupported(UnsupportedFeature),
}
```

`RuntimeFailureKind` and its payloads must be enums, not strings parsed by
adapters. The remote kind may retain the existing `RemoteErrorKind` plus
delivery facts without making the Shape `RemoteError` enum an exception
payload. VM and JIT are already two real adapters, so this is a real seam. CLI,
server, and embedding code are host projections over the same interface. The
interface is also the test surface: parity assertions should inspect the same
outcome rather than scrape stderr separately for each tier.

Do not layer this beside `VMError`, `JIT_RUNTIME_ERROR`,
`Runtime::last_runtime_error`, and `ShapeError::RuntimeError` indefinitely.
Migrate producers to the common outcome and delete superseded semantic side
channels; retain formatting adapters only at external interfaces.

## Compatibility Consequences

This model requires no Shape source change. Function types, higher-order
composition, annotation specialization, and `@remote`'s declared `R` all stay
unchanged. A function whose own `R` is `Result<T, E>` still returns its remote
domain `Ok` or `Err` unchanged.

The Rust embedding interface would change if `ProgramExecutor` directly adopts
`Evaluation<ProgramExecutorResult>`. That should be treated as an intentional
internal/interface version change, not hidden behind nested `anyhow::Error`.
CLI text and exit statuses can remain compatible while JSON diagnostics gain
additive `code`, `origin`, and structured-detail fields.

No FunctionBlob hash, bytecode version, snapshot version, or Shape value schema
change follows from an in-process evaluator outcome. A new serialized remote
engine-fault or cancellation kind would be a separate wire-version decision;
it is not required to preserve `@remote`'s `R` now.

## Focused Proofs

Existing tests prove pieces but not the full invariant:

* Runtime diagnostic tests assert `expect_run_err` and message fragments for
  division, bounds, stack, and arity failures at
  `tools/shape-test/tests/error_handling/diagnostics.rs:394-528`.
* Source `Result` propagation and top-level `?` behavior are covered at
  `tools/shape-test/tests/error_handling/try_operator.rs:587-617`.
* The structured uncaught payload side channel is covered at
  `crates/shape-vm/src/lib_tests_parts/runtime_error_payload_tests.rs:7-29`.
* Resource-limit tests prove clean failure rather than panic at
  `crates/shape-vm/src/executor/tests/resource_limit_enforcement.rs:47-110`.
* Interrupt snapshot/resume is covered at
  `crates/shape-vm/src/lib_tests_parts/interrupt_resume_tests.rs:38-132`.
* Async scope cancellation is covered at
  `tools/shape-test/tests/async_concurrency/async_scope.rs:104-142` and
  real-socket cancellation has ignored, timing-sensitive supervisor-lane proofs
  at `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs:347-490`.
* Explicit `remote::call` live `Ok` plus dead-endpoint `Err` is covered at
  `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs:85-122`.
* `@remote` receiver refusal currently proves nonzero host termination at
  `bin/shape-cli/tests/distributed_matrix_e2e.rs:102-147`, but it does not assert
  a structured evaluator outcome.

A focused implementation lane should add these interface-level regressions:

1. A function returning Shape `Err("domain")` yields
   `Completed(Result::Err(...))`; division by zero in the same declared return
   type yields `Failed`, proving the channels cannot be confused.
2. A native body returning `Ok(TypedReturn::Err(...))` completes, while an
   outer body failure preserves a typed builtin origin and code.
3. The same division, bounds, permission, resource, and trampoline failures
   produce equal VM/JIT outcome codes, locations, and user messages. The JIT
   side-effect-before-failure probe proves no interpreter rerun.
4. Suspension is returned as `Suspended`, root interrupt as `Cancelled`, and an
   awaited cancelled task does not degrade to an unclassified runtime string.
5. Injected contained JIT/FFI adapter panics produce `Faulted`, never
   `Failed(Program)` and never a completion sentinel.
6. `@remote fn f(...) -> int` completes with `int` over a live socket and
   produces structured `Failed(kind = Remote)` for a dead endpoint. No Shape
   Result is materialized on either path.
7. `@remote fn f(...) -> Result<int, string>` passes remote domain `Ok` and
   domain `Err` through as completions; transport failure remains outside that
   Result. The companion explicit `remote::call` failure is
   `Completed(Result::Err(RemoteError::...))`.
8. CLI projections preserve exit 0 for completion, 1 for runtime failure, and
   130 for user interruption; JSON retains the same message while exposing the
   structured code and origin.

## Bounded Follow-Ups

1. Record the `Evaluation<R>` terms and invariants in a focused ADR. Do not
   change `@remote`, Shape `Result`, or exception syntax in that decision.
2. Replace the VM's split `ExecutionResult`/control-`VMError` representation at
   `VirtualMachine::execute_with_suspend` with the common outcome. Keep
   `ResumeRequested` private to dispatch.
3. Preserve VM failure variants and source locations through enrichment. Stop
   converting every located failure to `RuntimeError(String)`.
4. Replace native module bodies' outer `String` with a structured failure type;
   leave `TypedReturn::Err` untouched. Migrate remote raising/result bodies as
   the focused proof of the distinction.
5. Make the JIT pending slot carry a structured failure payload behind its
   existing deopt signal, drain it exactly once, and map known native signals
   to the same codes as the VM.
6. Move uncaught `AnyError` data into `RuntimeFailure` and retire
   `Runtime::last_runtime_error` as an authoritative side channel.
7. Update `ProgramExecutor`, CLI, serve, and embedding adapters to project the
   common outcome. Preserve current CLI text/exit behavior first; additive
   diagnostics can follow.
8. Normalize scheduler cancellation into `Cancelled` and separately decide
   whether a future's cancellation is consumed by an async combinator or
   propagated to the root evaluation. Do not silently call both cases a
   runtime error.
9. Audit latent `SetupTry`/generic `VMError` unwinding. Keep internal `Throw`
   semantics needed by pattern failure and top-level `?`, but do not let a
   dormant catch mechanism redefine runtime failures as source exceptions.
10. Defer remote delivery guarantees, serialized engine-fault variants, and
    process isolation to their own designs. They are not prerequisites for the
    evaluator model or transparent `@remote` typing.

No cargo, test, build, extraction, or book command was run for this read-only
scout.

## Changed File

`docs/cluster-audits/wave40-runtime-failure-channel-model.md`
