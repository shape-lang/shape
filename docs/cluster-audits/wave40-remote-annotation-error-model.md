# Wave 40A: Remote Annotation Error Model

Date: 2026-07-10

## Decision

Keep `@remote` transparent to the annotated function's declared return type
and keep its transport/protocol failure path non-returning. In current Shape
terms, that means:

```text
@remote fn f(...) -> R   has call-site type (...)->R
remote::call(addr, f, ...) -> Result<R, RemoteError>
remote::call_async(addr, f, ...) -> Future<Result<R, RemoteError>>
```

Do not change `@remote` into an implicit `Result<R, RemoteError>` surface.
Do not describe `@remote` as raising a catchable typed `RemoteError`: Shape's
error doctrine is Result types, with no try/catch/throw mechanism. The current
raising path is an ordinary VM/runtime failure carrying a user-legible
message. A future internal VM diagnostic may preserve the `RemoteErrorKind`
alongside that failure, but that is not a Shape composition contract.

The typed, matchable error surface is explicitly `remote::call` and its async
sibling. This preserves the existing separation between a transparent
placement annotation and an explicit recoverable RPC primitive.

## Current Behavior

### `@remote`

`crates/shape-runtime/stdlib-src/core/remote.shape:72-108` declares two
internal siblings:

* `__call_raising` returns the callee value directly and maps transport,
  protocol, and remote failures to an ordinary runtime error.
* `__call_result` returns `Result<R, RemoteError>` and is reserved for the
  public `remote::call` elaboration.

The `remote` annotation itself is at
`crates/shape-runtime/stdlib-src/core/remote.shape:165-190`. Its `before`
hook calls `__call_raising(addr, ctx.target, args)` and returns
`{ result: result }`. The object `result` field is the annotation contract's
short-circuit: the wrapper substitutes the remote value and skips the local
implementation.

The annotation compiler makes this contract concrete:

* Annotation application compiles the original body as `name___impl`, then
  wraps it at `crates/shape-vm/src/compiler/functions_annotations.rs:2355-2450`.
* `ctx.target` is a typed function value with the original function's exact
  parameters and return type at
  `crates/shape-vm/src/compiler/functions_annotations.rs:2513-2557`.
* Runtime handlers are specialized per application site before the wrapper is
  emitted at `crates/shape-vm/src/compiler/functions_annotations.rs:2762-2791`.
* The wrapper invokes the before hook, interprets `{args, result, state}` and
  short-circuits on a non-null `result` at
  `crates/shape-vm/src/compiler/functions_annotations.rs:2916-3151`.
* The wrapper invokes the implementation and optional after hook, then returns
  the stored result at `crates/shape-vm/src/compiler/functions_annotations.rs:3153-3287`.

Therefore the current `@remote` success path preserves `R`. If the annotated
function's own declared return is `Result<T, E>`, the remote callee's domain
`Ok` or `Err` is just the value `R` and passes through unchanged. A transport,
protocol, permission, or receiver failure never reaches that domain Result;
the raising builtin returns `Err(String)` at the native boundary.

### `remote::call` and `remote::call_async`

The compiler recognizes the public calls at
`crates/shape-vm/src/compiler/expressions/function_calls.rs:5767-5783` and
elaborates them at `:6184-6410`:

* it resolves a statically-known named function or retained closure signature;
* it checks positional argument count and known types before network I/O;
* it lowers arguments into the typed positional carrier;
* it dispatches to `__call_result` or `__call_async_result`; and
* it types the expression as `Result<R, RemoteError>` or
  `Future<Result<R, RemoteError>>`.

The one `Future<T>` payload-unwrapping rule is also explicit in
`remote_result_payload_annotation` at `:6411-6435`; it does not change the
error model.

The runtime implementation keeps the two paths separate:

* `RemoteDispatcher::call_remote` returns `Result<TypedReturn, String>` for
  the raising annotation path, while `call_remote_result` returns a typed
  Result carrier at `crates/shape-runtime/src/module_exports.rs:100-161`.
* `remote_call_raising_body` delegates to `call_remote` at
  `crates/shape-vm/src/executor/builtins/remote_builtins.rs:1749-1778`.
* `remote_call_result_body` delegates to `call_remote_result` at
  `:1780-1810`; the async body registers the same recoverable result path at
  `:1812-1841`.
* `remote_call_response_to_typed_return` wraps success in `Ok(R)` and maps a
  `RemoteCallError` into the registered `RemoteError` enum's `Err` arm at
  `crates/shape-vm/src/executor/builtins/remote_builtins.rs:847-867`.
* The mapping is variant-aware at `:525-585`, including the load-bearing
  pre-send `Transport` versus post-send `ConnectionLost`/`Timeout` split.

This is a source-level current-state finding. No cargo or test command was
run in this scout. Historical audits that describe `remote::call` as lowering
to a bare return are stale relative to the current compiler and builtin paths;
they remain useful as regression history, not as the present contract.

## Why `@remote` Should Not Expose `Result`

### Type compatibility

Changing the annotation to return `Result<R, RemoteError>` changes every
annotated function's public function type. Existing code such as
`let value: int = compute(input)` becomes a type error, and passing `compute`
to a typed higher-order function no longer matches its declared signature.
The current `ctx.target` contract also describes the original target as
returning `R`; changing the wrapper result requires a second target/wrapper
signature model and changes how specialized hook handlers are typed.

An annotated function whose domain type is already `Result<T, E>` exposes a
further ambiguity. Automatic wrapping produces
`Result<Result<T, E>, RemoteError>`. Flattening avoids nesting but silently
couples transport failure semantics to the user's domain error type and is
impossible for arbitrary `E`. Requiring `E == RemoteError` is narrower than
the current pass-through behavior and breaks ordinary domain Results.

### Annotation composition

The wrapper contract is value-oriented: before hooks may replace arguments or
short-circuit with `{ result: v }`; after hooks receive and may transform the
implementation result. Making `@remote` return a Result changes what every
outer/inner annotation sees, changes after-hook behavior, and makes annotation
order observable through error wrapping. Existing runtime hook tests exercise
bare values and result transformation in
`tools/shape-test/tests/annotations_runtime/before_after.rs:9-170`.

The current design gives retry, fallback, load balancing, and idempotency
annotations a clean choice:

* transparent placement via `@remote`, where an unrecoverable call failure
  stops the current computation; or
* an explicit `remote::call` inside a user-defined annotation, where matching
  on `RemoteError` is possible and retry can respect the pre-send/post-send
  distinction.

Implicit Result conversion would force every annotation and every caller to
understand transport errors, even when the function is used as a normal
typed computation. It would also make a transparent placement annotation an
effectful signature transformation.

### Error-model compatibility

The project doctrine states “Result types for errors: No try/catch/throw” in
`docs/vision/implementation-plan.md:18-26`. The distributed design records
the same bounded choice at
`docs/design/distributed-function-transfer.md:198-202` and ratifies it in
OQ-1/Q26 at `:499-505` and
`docs/design/00-priority-spine-overview.md:178`.

Under that doctrine, “raise a typed RemoteError” is not a usable user-level
alternative to returning a Result. A raised value cannot be matched by Shape
code without a catch/unwind contract. The existing `RemoteError` enum is
therefore correctly constructed only by the recoverable `remote::call` path.

ADR-006's related carrier rules reinforce this boundary: typed values must
carry an authoritative kind rather than being reconstructed from raw bits
(`docs/adr/006-value-and-memory-model.md:1535-1583` and `:1876-1880`). A
typed `RemoteError` value is appropriate as a Result payload; fabricating one
as an opaque runtime exception would need a separate lifetime, schema, and
unwind contract.

## Recommended Contract

Adopt and preserve this bounded contract:

| Surface | Success type | Remote failure | Intended composition |
|---|---|---|---|
| `@remote` function call | Original declared `R` | Non-returning ordinary runtime failure with a legible diagnostic | Transparent placement; domain `Result<T,E>` passes through unchanged |
| `remote::call` | `Result<R, RemoteError>` | `Err(RemoteError::...)` | `match`, retry, fallback, load balancing, explicit policy |
| `remote::call_async` | `Future<Result<R, RemoteError>>` | Future resolves to `Err(RemoteError::...)` | `await`, `join all`, ordered recoverable composition |

The term “typed RemoteError” should be reserved for the second and third rows.
If host diagnostics need stronger structure, a follow-up may carry the
`RemoteErrorKind` alongside the internal VM error without changing the Shape
signature. It should not make `RemoteError` catchable by implication.

The two surfaces must continue sharing request construction, blob transfer,
return-kind validation, permission enforcement, and receiver execution. They
should differ only at the result projection boundary: raising maps failure to
the builtin error channel; recoverable dispatch maps failure to the Shape
`RemoteError` enum.

## Compatibility and Documentation Consequences

No production API change is recommended for this wave. The bounded follow-up
is contract alignment:

1. Keep `remote.shape`'s `__call_raising` and `__call_result` split and make
   the comments continue to state that `@remote` preserves `R`.
2. Keep `ctx.target` typed to the original implementation signature and keep
   `{ result: v }` as the before-hook short-circuit contract.
3. Teach recoverable transport handling only with `remote::call` and
   `remote::call_async`, never with `@remote` examples that match `Ok`/`Err`.
4. Keep the annotated function's own `Result<T,E>` examples as domain-value
   pass-through examples, clearly distinct from `RemoteError`.
5. Remove or rewrite stale book rows that show `@remote` returning `Ok`/`Err`.
   The existing triage already identifies those rows as policy rewrites in
   `docs/cluster-audits/wave6-disabled-distributed-comptime-proof-triage.md:351-354`.
6. Keep current direct-call tests as the error-composition proof: live
   `remote::call` returns `Ok(42)`, dead-port calls reach `Err`, and async
   calls preserve `Future<Result<...>>` in
   `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs:85-122` and
   `bin/shape-cli/tests/distributed_async_e2e.rs:7-67`.

## Focused Proofs

The next verification lane should add or retain these bounded assertions:

* A scalar `@remote fn f(...) -> int` remains assignable to `int` on success;
  a dead endpoint exits through the ordinary runtime error path rather than
  producing a matchable value.
* An `@remote fn f(...) -> Result<int, string>` returns the receiver's domain
  `Ok` and domain `Err` unchanged. A transport failure does not become the
  domain `Err` arm.
* `remote::call` returns `Ok(R)` on success and `Err(RemoteError::Transport)`
  for a pre-send failure; `remote::call_async` resolves the same distinction.
* A before hook's `{ result: v }` still short-circuits the original body, and
  an after hook still receives the bare target result for `@remote`.
* Stacked annotations preserve declared target signatures and do not silently
  add or remove a Result layer.
* If an internal typed diagnostic is later added, test that it preserves the
  `RemoteErrorKind` and message for host logs while remaining explicitly
  non-catchable in Shape.

## Bounded Implementation Order

1. Treat this note and the existing distributed-function-transfer OQ-1/Q26
   ruling as the source-of-truth contract.
2. Audit and correct stale `@remote` documentation/book rows; do not change
   the wrapper or builtin return types.
3. Keep `remote::call` and `remote::call_async`'s typed Result projection
   aligned with the raising path's shared request/receiver machinery.
4. Add only the narrow success/domain-Result/raising-vs-recoverable tests
   above. Do not introduce a typed exception carrier or implicit Result
   flattening in this lane.
5. Consider preserving `RemoteErrorKind` in internal host diagnostics as a
   separate design if operational observability requires it. That follow-up
   must specify VM error ownership and serialization without changing Shape
   function signatures.

## Changed File

`docs/cluster-audits/wave40-remote-annotation-error-model.md`
