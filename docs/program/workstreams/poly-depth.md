# POLY workstream — polyglot depth

Authority: ADR-019, R25, ADR-012 §5 (one cell amended, see ADR-012 Status),
ADR-013 §4 (providers); open rulings answered 2026-07-27 (grill Q1/Q3 —
lane C = STUB-CHANNEL + ASYNC-TRUTH, then ASYNC-OFFLOAD fast-tracked per
the user's Q3 = C ruling; the feasibility scout confirmed days-scale, see
POLY-ASYNC-OFFLOAD). Charter: the boundary that is currently declared-and-trusted,
copied, and ambient becomes checked, shareable, and reproducible — and the
result composes with the distributed story.

## Tickets

### POLY-STUB-CHANNEL — make `register_types` load-bearing

Scope: host calls the vtable stub channel
(`crates/shape-runtime/src/plugins/language_runtime.rs:188`, today
caller-less); Python/TypeScript extensions generate real `.pyi`/`.d.ts`
from declared Shape contracts (both sides are explicit stubs today).
Includes the Python invoke fix: cache the compiled module object instead of
re-executing the module per call (`extensions/python/src/runtime.rs:252`).
Blocked by: none. Blocks: POLY-FOREIGN-CHECK.
Tripwires: (1) generated stubs round-trip every type in the marshaling
table, asserted per type; (2) a declared type with no foreign mapping is a
structured diagnostic at the declaration, not a marshal-time error; (3) an
invoke-count fixture proves module setup runs once per handle.

### POLY-FOREIGN-CHECK — compile-time foreign body checking

Scope: `ForeignToolchainProvider` per language via `ComptimeHost` running
pyright/tsc over body+stubs; diagnostics mapped through the existing
bidirectional position mapping (`tools/shape-lsp/src/foreign_lsp.rs:71-152`),
promoted into the shared semantic query per ADR-011 §6 — the LSP-only
foreign diagnostics path is deleted as its consumer migrates. Tracked
inputs: toolchain identity/version, body digest, stub digest, environment
digest. Mismatch = compile error; absent toolchain = structured local
degradation; release/remote requires evidence.
Blocked by: POLY-STUB-CHANNEL; COMPTIME-HOST-TRACER (#138).
Tripwires: (1) a type error inside an inline Python body fails `shape run`
with the fence-local span; (2) comment-only body edits hit the memoized
result (invalidation trace); (3) canary secrets in provider config never
appear in query dumps or diagnostics (ADR-013 acceptance item applied
here); (4) compiler and LSP surface the identical structured diagnostic
(one-source assertion).

### POLY-ENV-PIN — declared, locked, content-addressed environments

Scope: `shape.toml` per-language dependency tables + lockfiles, including
the CHECKER toolchain (identity, version, settings digest — ADR-019 §1
pinning rule); `ForeignEnvironmentDigest` (interpreter/runtime identity +
resolved set + lock hash + checker pin) as `TrackedBuildInput`, derived
from declared locked inputs only, never ambient host inspection; digest
joins `ForeignFunctionEntry.content_hash` (`core_types.rs:64`) — a
deliberate, versioned bytecode-format bump sequenced with
VERIFIED-ARTIFACT-PERSISTENCE (#160), with load-time refusal on digest
mismatch;
digest enters the portable artifact's foreign-dependency manifest and
receiver admission (TARGET-ADMISSION-DYNAMIC #167 consumes). Delete the
runtime venv-sniffing silent fallback
(`extensions/python/src/runtime.rs:73`); add the lockfile-backed TS module
loader (none exists today — `import` cannot resolve).
Blocked by: POLY-STUB-CHANNEL. Coordinates with: #160, #167.
Tripwires: (1) the same source with a different lockfile produces a
different content hash and a different tracked digest; (2) a declared-but-
unavailable environment is a structured pre-entry failure, with the silent
ambient fallback proven gone (negative control: a stray ambient
site-packages must NOT satisfy the import); (3) remote placement of a
foreign-calling continuation is refused on digest mismatch with a
version-refusal-class diagnostic.

### POLY-ZERO-COPY — negotiated buffer sharing

Scope: versioned vtable buffer capability in the reserved ABI tail
(`shape-abi-v1/src/lib.rs:884`), negotiated at load; `TypedArray<T>`
call-scoped views (buffer protocol / ArrayBuffer) with shared-immutable and
exclusive-mutable modes per ADR-006 borrows; opt-in per parameter; bulk
fast path replacing the element-wise msgpack walk
(`foreign_marshal.rs:145`) for both modes.
Blocked by: POLY-STUB-CHANNEL (contract surface declares the mode;
per-parameter `shared`/`shared mut` spelling per ADR-019 §2).
Tripwires: (1) a mutation through a shared-immutable view is prevented or
detected (per-language mechanism documented); (2) Python export-count
verification: a fence that stashes a view (`numpy.asarray` into a global)
fails with a structured boundary error on return — the named corruption
class is a negative control, not documentation; a language without release
accounting has the mode refused, asserted; (3) large-array round-trip
benchmark in the PERF suite shows the copy tax gone; (4) refcount/pin
balance asserted across panic-in-extension paths.

### POLY-FOREIGN-REF — the opaque foreign-reference carrier

Scope: new pure-discriminator `HeapKind` with typed `Arc` payload
(instance, handle, disposer) per ADR-005/006 rules (FilterExpr precedent;
2026-07-05 ratification); drop via the owning instance's dispose under
ADR-010 lexical teardown; `Ffi` on every operation; snapshot refusal with
provenance (`STATE_MODEL_STATEFUL_OPAQUE`); remote rejection at artifact
construction absent matching admission.
Blocked by: POLY-STUB-CHANNEL. Coordinates with: TARGET-PYTHON (#163),
TARGET-TYPESCRIPT (#164).
Tripwires: (1) Q8/Q10 dispatch-table lockstep tests extended for the new
kind (4-table HeapKind lockstep per the merge gate); (2) drop-order and
double-drop balance fixtures with a finalization-observing fake extension;
(3) snapshot of a live foreign ref refuses naming value and origin;
(4) `as_heap_value()` soundness rules asserted for the new label.

### POLY-ASYNC-TRUTH — reject untruthful foreign async (transitional, days)

Scope: ADR-019 §5 — async foreign declarations reject with a structured
diagnostic until POLY-ASYNC-OFFLOAD lands; the rejection's negative test
becomes OFFLOAD's flip-to-green control. Lands within the existing scope
of #163/#164; no edge changes. Supervisor discretion: if OFFLOAD is ready
within the same review window, the two land as one branch with the
rejection commit first for bisectability.
Blocked by: none (a truthfulness fix).
Tripwires: (1) `async fn python` is a compile error naming the owning
issue; (2) sync foreign functions are unaffected (differential); (3) the
Book documents the rejection as the current truthful state per ADR-016's
unsupported-surface rule.

### POLY-ASYNC-OFFLOAD — real foreign async at parity with Shape's async (Q3 = C)

Scope: "offload and resolve at await", the exact pattern of Shape's own
async module calls (`spawn_async_module_future`,
`crates/shape-vm/src/executor/vm_impl/modules.rs:611-654`, +
`resolve_pending_async_task` `:665-682`): an async foreign stub returns
`Future(id)` (new `CallForeignAsync` or `is_async` branch in the stub
emitter, `compiler/functions_foreign.rs:320`), the invoke runs off-thread,
`await` resolves via the existing `PendingAsyncTask` completion channel.
Two `async fn python` calls overlap instead of serializing. Work items per
the 2026-07-27 feasibility scout:
Python via `spawn_blocking` + per-call GIL attach (SMALL); TypeScript via
a dedicated worker thread owning the V8 isolate through `fresh_instance()`
(the documented `serve_cmd.rs:35` precedent) with a command/reply channel
(MEDIUM); msgpack `Vec<u8>` crosses the channel, unmarshal on the
interpreter thread (SMALL); cancellation through the existing abort handle
+ `set_pending_async_cancellation_hook` with run-to-completion-then-
discard semantics (SMALL-MEDIUM); serialize per-worker instance access,
curing the latent `&mut`-through-`Send+Sync` aliasing hazard
(`extensions/typescript/src/runtime.rs:357+`) (MEDIUM, riskiest item);
thread `language_runtimes` into `run_isolated_async_fn`
(`async_runtime.rs:102-126`) — fixing the LIVE bug that foreign calls
inside spawned user async fns fail with "no extension provides language"
(SMALL). Estimate: Python alone 3–5 days; both languages 1.5–2.5 weeks.
Honest contract note: this is parity with Shape's shipped await (blocking
receive at the await point), which satisfies the ADR-012 matrix cell at
the same fidelity as native Shape async; true interpreter suspension is a
runtime-wide later item (`resume()` is `todo!()`, Phase-2c) and changes no
contract.
Blocked by: POLY-ASYNC-TRUTH (flip-to-green control). Related: #163/#164
(matrix-cell owners); JIT untouched (functions containing `CallForeign`
are already interpreter-only, `shape-jit/src/compiler/accessors.rs:706`).
Tripwires: (1) overlap fixture: two `async fn python` sleeps of 200ms
complete in ~200ms wall, not ~400ms (and the equivalent TS pair);
(2) `await` on a sync foreign call is unchanged (the `op_await`
pass-through differential); (3) foreign call inside a spawned user
`async fn` succeeds (regression for the live language_runtimes bug);
(4) cancellation of an in-flight foreign await settles per the declared
run-to-completion-then-discard semantics with no double-completion — and a
cancellation request is never reported as confirmed foreign termination;
(5) concurrent-invoke stress on one extension instance shows serialized
access (the aliasing hazard fixture); (6) VM/JIT differential green (JIT
bails on CallForeign functions loudly, as today).

### POLY-LSP-FENCE — full embedded-language editing

Scope: extend the existing foreign-LSP pipeline (diagnostics relay works
today) to completions and hover inside fences, driven by the same stubs as
POLY-FOREIGN-CHECK so editor feedback and compiler verdicts agree.
Blocked by: POLY-FOREIGN-CHECK.
Tripwires: (1) completion inside a `fn python` body offers stub-declared
parameter names/types; (2) LSP diagnostics and compiler diagnostics for the
same defect carry the same structured identity (no re-fork of the promoted
pipeline).

## Sequencing

POLY-STUB-CHANNEL and POLY-ASYNC-TRUTH are immediately startable after
ratification. POLY-FOREIGN-CHECK follows the ComptimeHost tracer (#138).
POLY-ENV-PIN sequences its hash break with #160. ZERO-COPY and FOREIGN-REF
are independent of each other after the stub channel. The workstream's
composition test — remote placement of a checked, environment-pinned
`fn python` with digest-verified admission — is the acceptance capstone and
feeds GATE-DISTRIBUTED (#122) evidence.
