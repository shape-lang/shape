# Wave-40D Remote Delivery Semantics

Date: 2026-07-10

Scope: read-only design scout over the distributed transfer design, wire
request/call identities, sender and receiver paths, cancellation, timeout and
retry behavior, `RemoteError`, focused tests, and current book claims. This
report is the only file changed. No build, test, extraction, or book-truth
command was run.

## Recommendation

Describe the current remote-call contract as **single-submission, outcome-aware
RPC**, not at-most-once, at-least-once, or exactly-once delivery.

One source-level invocation creates one logical call. The sender submits one
execution-bearing `Call` attempt and does not automatically retry after a
failure that could have reached the receiver. The one exception is missing-blob
resupply: the receiver explicitly rejects the first attempt before VM
execution, so a second `Call` is a safe protocol continuation, not a general
retry.

The public surfaces should promise:

| Surface | Honest promise |
|---|---|
| `@remote` | Transparent placement with the original return type. Success is an observed remote result. A remote failure terminates the current computation. It must not silently retry an outcome-unknown call, run locally as fallback, or imply exactly-once effects. |
| `remote::call` | Recoverable `Result<R, RemoteError>`. The error must distinguish definitely-not-executed from outcome-unknown. The caller owns retry, fallback, and idempotency policy. |
| `remote::call_async` | The same delivery contract as `remote::call`, plus a caller-local future. Cancelling that future is prompt locally and sends a best-effort remote cancellation request; it does not prove the remote call did not execute. |

Do not add transparent post-send retries to `@remote`. If retries after an
unknown outcome are desired, require either an idempotent operation or a real
idempotency-key/deduplication protocol and expose that policy through an
explicit API or user-defined annotation over `remote::call`.

Before documenting this as implemented, production must preserve sender phase
information through `RemoteError` construction and give async calls a response
deadline. The current code declares the right pre-send/post-send vocabulary but
does not carry it end to end.

## Canonical Terms

Use these terms consistently in code, tests, and documentation:

| Term | Meaning |
|---|---|
| **Logical call** | One source-level invocation of `@remote`, `remote::call`, or `remote::call_async`. |
| **Wire attempt** | One `WireMessage::Call` frame sent for a logical call. Missing-blob resupply can make two attempts. |
| **Correlation token** | An identifier used to associate protocol activity. It does not by itself prevent duplicate execution. |
| **Cancellation token** | The current `RemoteCallId`: an ephemeral correlation token used only by `remote::call_async` and `CancelCall`. |
| **Idempotency key** | A caller-stable semantic identity for an operation whose duplicates the receiver must coalesce. No such key exists today. |
| **Admission** | Receiver validation and scheduling before the user function enters a VM frame. |
| **Execution started** | The receiver has entered user computation. This is later than the current registry's `Running` transition. |
| **Terminal outcome** | The receiver produced success or a structured failure for an attempt. |
| **Observed outcome** | The sender decoded the terminal response. A terminal outcome may exist without being observed if the reply is lost. |
| **Outcome unknown** | The sender cannot know whether execution started or completed because submission may have reached the receiver but no terminal response was observed. |
| **Duplicate execution** | The same logical operation enters receiver user computation more than once. |
| **Deduplication** | Receiver-side suppression or joining of attempts sharing an idempotency key and matching payload fingerprint. |
| **Result replay** | Returning a stored terminal response for a completed idempotency key instead of executing again. |
| **Idempotent operation** | A domain property: repeating the operation has the same externally visible effect. It is not created by assigning a call id. |

Avoid using "request id", "call id", "retry", or "cancelled" without the
qualifier above. In particular, call IDs are not idempotency keys, and local
future cancellation is not proof of remote non-execution.

## Current Protocol Reality

### Identities do not provide deduplication

`RemoteCallRequest.call_id` is optional and documented as a sender-assigned
identity for best-effort cancellation only
(`crates/shape-vm/src/remote.rs:50-64`). `RemoteCallId` explicitly does not
identify a durable remote future (`:125-134`). Synchronous `@remote` and
`remote::call` requests leave it `None`; only `remote::call_async` assigns one
(`crates/shape-vm/src/executor/builtins/remote_builtins.rs:931-975`).

The serve registry stores only `Queued`, `CancelRequested`, `Running`, or
`Finished`, not the request fingerprint or response
(`bin/shape-cli/src/commands/serve_cmd.rs:206-288`). A repeated call ID is not
rejected, joined, or replayed: `register_queued` overwrites every prior state
except `CancelRequested`. The same ID can therefore execute again, including
after `Finished`, and can race a still-running attempt on another connection.
The response is never cached.

Execution-server `request_id` fields are correlation-only and are merely echoed
in responses (`crates/shape-vm/src/remote.rs:403-487`). `remote::execute`
currently sends the constant `1`; the server performs no deduplication by that
field (`remote_builtins.rs:1582-1586`). These IDs must not be cited as an
at-most-once mechanism.

### Receiver scheduling state is not execution certainty

For a call with an ID, `shape serve` registers it, waits for the concurrency
permit, marks it `Running`, and only then starts `handle_call`
(`serve_cmd.rs:646-690`). `handle_call` still has to create a store, verify/link
the request, enforce permissions and ABI, construct a VM, and enter the callee
(`serve_cmd.rs:1230-1277`; `crates/shape-vm/src/remote.rs:960-1350`).

Therefore the registry's `Running` means **worker-owned/not preemptible by this
registry**, not **user code definitely started**. `AlreadyRunning` is an honest
cancellation limitation, but it does not prove side effects occurred.

After the worker returns, the registry is marked `Finished` before the response
is written to the socket (`serve_cmd.rs:677-689`, `:809-819`). If that write
fails, the receiver has a terminal outcome but the sender sees connection loss.
Because only `Finished` is retained, not the result, a later duplicate executes
again rather than replaying the lost response.

### Retry is intentionally narrow

`call_with_resupply` retries only a structured
`MissingModuleFunction` response carrying hashes, at most once
(`crates/shape-vm/src/remote.rs:2184-2297`). The receiver discovers missing
dependencies before user-function execution, making this safe for
side-effecting callees. Both sync and async sender paths preserve this bounded
rule (`remote_builtins.rs:734-845`).

Call this **pre-execution resupply** or a **protocol continuation** in public
explanations. Calling it simply a retry invites the false inference that timeout
or connection-loss retries are also safe. No transport, receiver-runtime, or
post-send failure is automatically retried today.

### Cancellation is best-effort and one-way from Shape's perspective

Cancelling an async future runs a hook, spawns a `CancelCall`, aborts the local
network task, and marks the local scheduler entry cancelled
(`crates/shape-vm/src/executor/task_scheduler.rs:289-305`;
`remote_builtins.rs:1042-1054`). The cancellation send is not awaited by Shape,
and its `RemoteCancelResponse` is not surfaced to the caller.

The receiver can suppress a call that has not passed its queue boundary. A
cancel arriving before the call creates a `CancelRequested` tombstone; a queued
call observes that state and returns a cancelled-before-execution response.
Once marked `Running`, cancellation returns `AlreadyRunning` and does not
interrupt the VM frame (`serve_cmd.rs:231-288`, `:659-690`, `:1293-1319`).

Important consequences:

- local cancellation is deterministic; remote cancellation is best-effort;
- `AcceptedQueued` means that queued instance will not execute, not that the ID
  is permanently suppressed;
- `AlreadyRunning` means the worker is not preemptible, not that completion or
  side effects are known;
- a cancelled-before-execution `CallResponse` currently uses wire
  `RuntimeError`, so it would map to `RemoteError::Remote` if observed rather
  than to a dedicated not-executed cancellation outcome
  (`serve_cmd.rs:1280-1290`); and
- production `shape serve` treats an unknown cancellation as
  `AcceptedQueued` to support cancel-before-arrival, so its `UnknownCall`
  outcome is not the normal production result for an absent ID.

## Sender Failure Classification Gap

The design and stdlib declare a load-bearing split:

- `RemoteError::Transport`: request definitely did not execute and is safe to
  retry;
- `RemoteError::ConnectionLost` and `RemoteError::Timeout`: request may have
  executed and must not be automatically retried
  (`docs/design/distributed-function-transfer.md:143-170`, `:378-421`;
  `crates/shape-runtime/stdlib-src/core/remote.shape:117-163`).

The current production sender does not implement that split:

1. Plain synchronous TCP does produce typed `TransportError` values, including
   a 30-second read timeout (`crates/shape-wire/src/transport/tcp.rs:31-79`).
   `wire_roundtrip` immediately formats the error into a string
   (`remote_builtins.rs:1162-1180`).
2. `send_call_classifying_transport` maps every such string to wire-local
   `RemoteErrorKind::Transport`, whether it came from connect, write, timeout,
   reply reset, or decode (`remote_builtins.rs:997-1040`).
3. `build_remote_error_arc` can construct `Transport` and `Timeout`, but
   `RemoteErrorKind` has no `ConnectionLost` case, and no sender path constructs
   `Timeout` for a read timeout (`remote_builtins.rs:525-585`;
   `crates/shape-vm/src/remote.rs:182-228`). The public
   `RemoteError::ConnectionLost` variant is therefore not reachable from this
   production path.
4. `transport_send_phase` and `transport_error_shape_variant` encode the
   intended distinction, but source search finds them used only by their unit
   test, not by the sender (`crates/shape-vm/src/remote.rs:262-309`,
   `:3948-3973`).
5. The raising `@remote` path is weaker still: its sender maps every transport
   string to wire `RuntimeError`, then raises a generic runtime diagnostic
   (`remote_builtins.rs:683-703`, `:878-909`). It loses both cause and execution
   certainty.

There is also a deadline gap. Plain and TLS synchronous calls have connect and
read timeouts. Async TCP has a 10-second connect timeout but no write or reply
timeout; async TLS has connect and handshake timeouts but no post-handshake
write or reply timeout (`remote_builtins.rs:1241-1352`, `:1438-1480`). A
never-replying receiver can therefore leave `remote::call_async` pending until
the caller cancels it. This contradicts the design claim that the sender always
gets `RemoteError::Timeout` at the request deadline for a wedged foreign call
(`docs/design/polyglot-distributed-integration.md:295`, `:319`).

Finally, the design calls all send failures definitely-not-executed. That is
too strong without an application-level admission acknowledgement. DNS,
encoding, payload-cap, and connect failures are definitely pre-submission. A
write or flush failure can be ambiguous from the application caller's point of
view: the receiver may have obtained a complete frame even though the sender
did not observe successful completion of the write. Conservatively classify
ambiguous writes as outcome-unknown, or add an explicit receiver acceptance
acknowledgement before claiming otherwise.

## Delivery State Model

Model **failure cause** and **execution certainty** as separate axes. Current
`RemoteError` variants mostly encode cause and cannot reliably answer whether
retry is safe.

Recommended execution-certainty vocabulary:

| Certainty | Definition | Examples |
|---|---|---|
| `DefinitelyNotExecuted` | The attempt could not have entered receiver user code. | local encode/type/payload failure, DNS/connect failure, explicit auth/permission/missing-function rejection, accepted queued cancellation, missing-blob response |
| `OutcomeUnknown` | Submission may have reached execution, but no terminal outcome was observed. | ambiguous write, reply timeout, reset/EOF/decode failure after submission, caller cancellation after send |
| `ExecutionStarted` | Receiver reported a failure after entering user computation; side effects may already exist. | callee runtime error, resource-limit failure after work began, return-ABI mismatch detected after the call |
| `Completed` | Sender observed a successful terminal result. | decoded `CallResponse(Ok(...))` |

Some causes span certainty classes. `Protocol` includes pre-execution hash or
argument rejection and post-execution return-kind mismatch. `Remote` currently
also carries cancelled-before-execution. Therefore retry logic must not infer
certainty from broad cause variants unless the mapping is narrowed or the
certainty is carried explicitly.

The safest public evolution is either:

1. keep the existing cause variants and add an explicit certainty field or
   accessor to every recoverable error; or
2. split ambiguous mixed variants so every public variant has one certainty.

Whichever shape is chosen, the invariant is more important than the spelling:
only `DefinitelyNotExecuted` is generally retry-safe. `OutcomeUnknown` is safe
to retry only under a caller-supplied idempotency policy or receiver
deduplication guarantee.

## Scenario Matrix

| Scenario | Current behavior | Honest caller interpretation |
|---|---|---|
| Address/serialization/permission check fails before socket use | No `Call` reaches the receiver. Some failures raise before a `RemoteError` value is built. | Definitely not executed. |
| DNS/connect/payload-cap failure | Sender returns/raises transport failure. | Definitely not executed. |
| Write fails | Currently classified as `Transport`. | Conservatively outcome unknown unless the transport proves no complete frame was delivered. |
| Receiver explicitly rejects auth, permissions, function, ABI, hash, or missing blobs before VM entry | Structured `CallResponse(Err(...))`; missing blobs may trigger one resupply attempt. | Definitely not executed for that attempt. |
| Receiver user code fails | Structured `RuntimeError`; sender maps to `Remote`. | Execution started; side effects are not rolled back. |
| Receiver completes, then response write/connection fails | Receiver marks `Finished`; sender sees a transport string, currently mapped to `Transport`. No result replay. | Outcome unknown; the operation may have completed. |
| Sender read timeout | Sync TCP times out but maps to `Transport`; async has no reply deadline. | Outcome unknown; never auto-retry a non-idempotent call. |
| Queued async call is cancelled | Receiver can return `AcceptedQueued` and skip the call. Sender locally aborts without observing that outcome. | Local future cancelled; remote non-execution is likely but not known to Shape code. |
| Running async call is cancelled | Receiver returns/logs `AlreadyRunning`; execution continues. | Outcome unknown until separately observed; cancellation is not rollback. |
| Same call ID is submitted twice | Registry overwrites prior state and both attempts may execute. | Duplicate execution is allowed. |
| Missing blob resupply | Two `Call` attempts; first is a pre-execution rejection, second may execute once. | Safe protocol continuation, not general retry. |

## Idempotency And Duplicate Suppression

Do not repurpose `RemoteCallId` as an idempotency key. Its generation is
process-local/time-based, it is absent from sync calls, the receiver does not
bind it to a payload, and completed results are not stored.

A future deduplicating protocol needs all of the following:

- a separate `IdempotencyKey` supplied or derived at the logical-call level;
- a canonical request fingerprint covering principal/tenant, target function
  hash, arguments, captures, and semantics-affecting options;
- atomic receiver insertion before admission;
- states such as `InFlight`, `Completed(response)`, and terminal rejection;
- duplicate behavior: join an in-flight attempt or replay the completed result;
- rejection when one key is reused with a different fingerprint;
- an explicit retention window, capacity policy, receiver epoch, and crash
  semantics; and
- a security scope preventing one caller from probing or replaying another
  caller's result.

An in-memory result cache could promise deduplication only within one receiver
process epoch and retention window. Durable at-most-once execution across
receiver crashes requires durable admission and completion records. Exactly-once
**effects** cannot be promised for arbitrary external I/O without transactional
integration with those effects. Even with deduplication, use the narrower term
"deduplicated execution within the stated window" rather than "exactly once."

## Surface-Specific Invariants

### `@remote`

- Preserve the annotated function's declared return type, as decided by the
  Wave-40 annotation error-model scout.
- On success, return only the observed receiver value; never execute the local
  body as a hidden fallback.
- On pre-execution resupply, continuation is transparent.
- On outcome-unknown failure, stop with a diagnostic that says the call may
  have executed. Do not auto-retry.
- Retry/fallback/circuit policy belongs in an explicit annotation built over
  `remote::call`, where the typed error and certainty are inspectable.

### `remote::call`

- Return `Ok(R)` only after a decoded terminal success.
- Return a receiver rejection or receiver execution failure as a typed cause
  with correct execution certainty.
- Preserve transport phase; do not collapse timeout/reply loss into
  definitely-not-executed `Transport`.
- Make no at-most-once or exactly-once claim without a separate deduplication
  protocol.
- Never retry `OutcomeUnknown` automatically.

### `remote::call_async`

- Use the same outcome mapping and deadline as synchronous `remote::call`.
- Treat the Future ID and `RemoteCallId` as local scheduling/cancellation
  identities, not remote future or idempotency identities.
- State cancellation in two parts: local wait cancelled; remote cancellation
  requested. Do not collapse them into one success claim.
- Keep queued suppression and running non-preemption explicit.

## Test And Proof Gaps

Current tests prove happy-path sync/async results, dead-port `Err`, missing-blob
resupply, and queued/running cancellation behavior. The transport phase unit
test proves a pure classifier, not its production integration. The dead-port
e2es match `Err(_)`, so they do not verify the `Transport` variant. Cancellation
proofs inspect server logs and are intentionally timing-sensitive/ignored.

A delivery-semantics lane should add deterministic socket fixtures for:

1. pre-connect failure: assert `DefinitelyNotExecuted` / `Transport`;
2. full request accepted, response deliberately dropped: assert
   `ConnectionLost` / outcome unknown and one receiver side effect;
3. full request accepted, reply deliberately delayed past a bounded sync and
   async deadline: assert `Timeout` / outcome unknown;
4. explicit pre-execution rejection: assert zero side effects and
   definitely-not-executed certainty;
5. callee writes a side effect then fails: assert execution-started certainty;
6. missing-blob resupply: assert two attempts and one user-code execution;
7. duplicate identical call IDs: first expose the current double execution,
   then pin the chosen future dedup behavior;
8. same idempotency key with a different fingerprint: reject if/when that
   protocol exists;
9. queued cancellation versus running cancellation: assert receiver state and
   local outcome separately; and
10. receiver completes but sender disconnects, followed by a repeated logical
    call: demonstrate current duplicate execution or future result replay.

## Book And Design Alignment

The current remote book correctly distinguishes transparent `@remote` failure
from recoverable `remote::call` (`shape-web/.../stdlib/core/remote.mdx:53-55`),
but it does not explain outcome-unknown failures or retry safety. Its error
section only lists connection refusal, timeout, server compilation, and runtime
errors (`:220-236`). Add the certainty distinction before teaching retries.

The same page says persistent connections cache repeated public calls
(`:206-216`), while `wire_roundtrip` creates a fresh transport per attempt and
the production blob cache is connection-local. Wave-39A proves same-connection
cache reuse, not that `remote::call` currently reuses a persistent connection.
That claim should be narrowed independently of delivery semantics.

The async book says scope exit cancels pending children deterministically
(`shape-web/.../fundamentals/async.mdx:153`). For a remote future, this is true
only of the caller-local scheduler entry. It should not imply deterministic
remote non-execution.

The annotation/cookbook material advertises timeout and retry wrappers without
an idempotency warning. Generic retry is fine, but applying it around an
outcome-unknown remote call can duplicate side effects. Remote retry examples
must state the operation's idempotency or use an idempotency key once one
exists.

The normative distributed design already says no automatic retries after
timeout or connection loss. Preserve that decision, but correct its claim that
the production pre-send/post-send variants are wired and soften its assertion
that every write failure proves non-execution. The polyglot design's guaranteed
async timeout is also ahead of the current sender implementation.

## Follow-On Order

1. Preserve typed transport phase through sync and async sender code; construct
   `Transport`, `ConnectionLost`, and `Timeout` honestly, and add a reply
   deadline to both async transports.
2. Decide whether execution certainty is a separate public field/accessor or is
   encoded by narrower `RemoteError` variants. Fix mixed cancellation and
   protocol mappings accordingly.
3. Align book claims and add the deterministic unknown-outcome proof fixtures.
4. Keep v1 at single-submission with no outcome-unknown auto-retry.
5. Design a separate idempotency-key/result-replay protocol only if product use
   cases require transparent retries. Specify retention and crash boundaries
   before claiming at-most-once behavior.

## Bottom Line

Shape currently has one safe retry-like behavior: missing-blob resupply after a
receiver-proven pre-execution rejection. Everything after possible submission
is single-shot and may end with an unknown outcome. `RemoteCallId` enables
best-effort queued cancellation but provides no deduplication or result replay.
Transparent `@remote` must remain conservative; explicit `remote::call` is the
surface where callers can inspect uncertainty and apply idempotency-aware
policy. The code must first make that uncertainty observable as declared.
