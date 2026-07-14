# Wave-31A State Carriers/Resume Next Slice

Role: Wave-31A implementation scout.

Scope honored:

- Static inspection only over the requested state/runtime/book files plus one
  narrow dispatch-context read needed to explain why public resume is not yet a
  state-builtin-only slice.
- Wrote only this report.
- Did not edit `AGENTS.md`.
- Did not run cargo, just, nextest, rustc, build, test, extractor, or
  book-truth commands.
- Pre-existing dirty state-file changes were present and left untouched.

## Recommendation

Implement `state.capture_call` first, not public `state.resume` or
`state.resume_frame`.

Smallest honest slice:

1. Return a real schema-backed `CallPayload` for a bare function id or closure
   whose content hash is present in `ctx.function_hashes`.
2. Support `args` only when the second argument is already a real homogeneous
   scalar/string runtime array carrier that can be stored as the `CallPayload`
   `args` field without boxing into fake `Any`.
3. Surface heterogeneous, heap-shaped, missing-hash, and unsupported callable
   cases with the existing W17 surface style.
4. Flip only a new narrow state-book row such as "capture a function hash and
   scalar args into a payload"; keep the remote dispatch examples disabled
   until transport and generic deserialize surfaces are separately true.

This can retire at least one disabled row around `state.capture_call` in
`stdlib/core/state.mdx:190` after the book example is narrowed away from
transport, `Sample`/`Model`, and generic `Any` arguments. It also creates the
missing production primitive used by broader disabled rows in
`advanced/content-addressed-bytecode.mdx:282` and `:321`, but those rows should
remain disabled because they still rely on annotation rewriting, old
`__original__(args)` forwarding, cluster policy objects, and generic
serialization.

## Why Not Resume First

Public `state.resume` has two separate blockers.

First, the public body requires a live callback. `state_resume_stub` explicitly
surfaces when `ctx.set_pending_resume` is absent, then only queues the payload
when that callback exists (`state_builtins/introspection.rs:753-788`). The
current native-module dispatch context still installs `set_pending_resume:
None` and `set_pending_frame_resume: None` (`executor/vm_impl/modules.rs:901-913`),
even though `ModuleContext` has callback slots for both (`module_exports.rs:199-207`).

Second, the lower restore path still cannot round-trip a normal
`state.capture_all()` result with frames. `apply_pending_resume` can consume a
typed-object `VmState` and route it through `decode_vmstate_typed_object` into
`VirtualMachine::from_snapshot` (`resume.rs:110-197`). But the decode path sets
resume `ip` to `0` because the public `VmState` schema has no resume IP field
(`resume.rs:519-545`), and it rejects any non-empty `frames` array because the
read-only `FrameState` schema cannot supply `return_ip`, `locals_base`, or
`locals_count` (`resume.rs:617-705`).

`state.resume_frame` is also not the smallest slice. The body intentionally
errors even if `set_pending_frame_resume` is present because the current public
`FrameState` only carries metadata, not real locals/upvalues plus a validated
resume offset (`state_builtins/introspection.rs:790-820`). The lower
`apply_pending_frame_resume` path is useful, but it needs a new public carrier
before it can be called honestly.

## What Works Today

- Runtime schemas exist for `FunctionRef`, `FrameState`, `VmState`,
  `ModuleState`, `CallPayload`, and `Delta` (`state_builtins/core.rs:42-105`).
- The registered runtime `CallPayload` schema is `{ hash: string, args: any }`
  (`state_builtins/core.rs:82-88`). The stdlib source still declares
  `{ function: FunctionRef, args: Vec<T>, upvalues: Vec<T>? }`
  (`state.shape:50-56`). The next worker should settle that mismatch before
  making book-visible promises.
- `state.fn_hash` already decodes inline function ids and closure function ids,
  then looks up the content hash from `ctx.function_hashes`
  (`state_builtins/core.rs:536-604`). This is the helper shape `capture_call`
  should reuse or extract.
- `state.capture_all` returns a schema-backed `VmState` typed object with
  `frames`, homogeneous scalar/string `module_bindings`, and
  `instruction_count` (`state_builtins/introspection.rs:688-716`). Tests cover
  the happy path and the heterogeneous-binding surface
  (`state_builtins_tests.rs:680-744`).
- `state.capture_module` returns a schema-backed `ModuleState` typed object with
  bindings and schema hashes (`state_builtins/introspection.rs:718-739`), with
  tests for schema hash shape and heterogeneous binding surfaces
  (`state_builtins_tests.rs:746-832`).
- `VmStateSnapshot` clones live frame locals, current args/current locals, module
  bindings, and instruction count into `KindedSlot` carriers
  (`vm_state_snapshot.rs:64-75`, `:178-230`, `:337-372`). It still records
  per-frame `local_ip` as `0` and per-frame args as empty because those are
  explicit follow-ups (`vm_state_snapshot.rs:118-130`).
- `state.serialize` can encode a `KindedSlot` through the snapshot
  `SerializableVMValue` codec into bytes (`state_builtins/core.rs:418-432`,
  `:668-682`). `state.deserialize` currently projects only scalar/string/bool
  returns; arrays, objects, heap values, and `None` remain surfaced
  (`state_builtins/core.rs:499-516`, `:684-705`).

## Disabled Rows Inspected

Current manifest rows in scope:

| Row | Status after this scout |
|---|---|
| `stdlib/core/state.mdx:163` / `B__stdlib__core__state__9__L163.shape` | Keep disabled. Current `capture_all` is a bounded typed object, not full portable all-frame state; page also mentions nonexistent `timestamp`. |
| `stdlib/core/state.mdx:190` / `B__stdlib__core__state__11__L190.shape` | Best next target after narrowing. Existing example depends on transport, generic `Any`, `Sample`/`Model`, and `payload.function.hash`; first runnable row should check the bounded runtime shape only. |
| `stdlib/core/state.mdx:220` / `B__stdlib__core__state__12__L220.shape` | Keep disabled. Needs callback wiring plus true frame/resume-IP schema. |
| `stdlib/core/state.mdx:238` / `B__stdlib__core__state__13__L238.shape` | Keep disabled. Current `FrameState` is metadata-only. |
| `stdlib/core/state.mdx:331` / `B__stdlib__core__state__19__L331.shape` | Keep disabled. Assumes arbitrary-value deserialize plus external cache API. |
| `stdlib/core/state.mdx:398` / `B__stdlib__core__state__22__L398.shape` | Keep disabled. Needs object/module diff and patch, beyond scalar root replacement. |
| `stdlib/core/state.mdx:481` / `B__stdlib__core__state__26__L481.shape` | Keep disabled after `capture_call`; it also depends on transport, generic `Any` args/results, generic deserialize, and spread invocation. |
| `stdlib/core/state.mdx:511` / `B__stdlib__core__state__27__L511.shape` | Keep disabled. Needs ModuleState diffing plus live transport. |
| `stdlib/core/state.mdx:538` / `B__stdlib__core__state__28__L538.shape` | Keep disabled. Needs stable function+argument payload hashing, arbitrary serialize/deserialize, spread calls, and store API. |
| `advanced/content-addressed-bytecode.mdx:154` | Keep disabled. Claims full frame tuples with `function_hash`, `local_ip`, and locals. |
| `advanced/content-addressed-bytecode.mdx:168` | Keep disabled. Full public resume not ready. |
| `advanced/content-addressed-bytecode.mdx:282`, `:321` | `capture_call` helps, but rows remain disabled due annotation/cluster/transport/generic-serialization assumptions. |
| `advanced/content-addressed-bytecode.mdx:396` | Keep disabled. Live migration needs full `capture_all`, transport, scheduler/coroutine policy, and old forwarding rewrite. |
| `advanced/content-addressed-bytecode.mdx:541` | Separate lane. Wave-28 made `caller`, homogeneous `args`, and string-only `locals` honest; this row still claims full `Array<any>` and `Map<string, any>`. |
| `advanced/resumability.mdx:21`, `:105` | Book/fixture lane, not this state-builtin slice. The page itself says dynamic hash/resume flow is non-runnable in the truth gate (`resumability.mdx:10-16`, `:95-118`). |

## Concrete Next Lane

Branch role: Wave-31B `capture_call` carrier worker.

Owned files:

- Primary: `crates/shape-vm/src/executor/state_builtins/introspection.rs`
- Primary: `crates/shape-vm/src/executor/state_builtins/core.rs`
- Primary: `crates/shape-vm/src/executor/state_builtins_tests.rs`
- Primary: `crates/shape-runtime/stdlib-src/core/state.shape`
- Book after code is verified: `../shape-web/book/book-site/src/content/docs/stdlib/core/state.mdx`

Do not own `resume.rs` or `vm_state_snapshot.rs` for this first lane except for
read-only context.

Implementation details:

1. Decide the public `CallPayload` shape. The runtime schema currently says
   `hash`; the stdlib source and docs say `function.hash`. The smallest code
   path is `{ hash: string, args: any }`; if product semantics require
   `FunctionRef`, update the runtime schema and tests deliberately.
2. Extract a small helper for function-id/hash decoding from `state_fn_hash`
   instead of coupling `introspection.rs` back to `core.rs`, because `core.rs`
   already imports the introspection bodies.
3. In `state_capture_call_stub`, require arity 2 and decode the first arg as the
   same callable subset `state.fn_hash` already supports: inline function id and
   closure. Keep `FunctionRef`, trait object, and module function handles
   surfaced unless a real decode path is added.
4. Preserve the second arg as a real carrier, not an invented `Array<any>`.
   Narrow accepted values to the currently projectable homogeneous array kinds
   or a single scalar/string demo shape; surface heterogeneity and unsupported
   heap arrays.
5. Construct an opaque typed object using the existing
   `typed_object_slot_for_schema` / `opaque_typed_object_return` helpers.
6. Add a book-runnable example that only asserts the bounded payload, for
   example `payload.hash.length == 64` plus a scalar argument count/value if the
   chosen carrier exposes one. Do not include transport.

Focused tests to add:

- `state_capture_call_returns_payload_for_inline_function_id_with_string_args`
  or equivalent using `ctx_with_hashes`, a registered `CallPayload` schema, and a
  homogeneous arg carrier.
- `state_capture_call_returns_payload_for_closure_with_hash` if a safe closure
  fixture already exists in state tests; otherwise skip closure until a real
  fixture can be borrowed without growing this lane.
- `state_capture_call_surfaces_missing_function_hash`.
- `state_capture_call_surfaces_non_callable`.
- `state_capture_call_surfaces_unsupported_or_heterogeneous_args`.
- Keep `test_w17_state_bodies_return_structured_errors` valid by passing empty
  args for the missing-argument surface, not by expecting `capture_call` always
  to be unsupported.

Allowed verification for that worker should include focused state-builtin tests
under the supervisor's cargo lane. Static-only agents should only run
`git diff --check` on touched files.

## Follow-On Lanes

1. Public `state.resume` wiring lane:
   own `executor/vm_impl/modules.rs`, `state_builtins/introspection.rs`,
   `executor/resume.rs`, and focused dispatch tests. First wire
   `set_pending_resume`; then prove an explicitly empty-frame `VmState` restore
   if that is a truthful public behavior. Do not claim full continuation.
2. Resumable frame schema lane:
   grow or split `FrameState` so a resumable carrier includes real locals,
   upvalues, validated `ip_offset`, and structural frame fields needed by
   `SerializableCallFrame`. Then connect `state.resume_frame` to
   `set_pending_frame_resume`.
3. Full `VmState` resume lane:
   recover per-frame local IP and args in `VmStateSnapshot`, add resume IP to
   the public carrier, and remove the non-empty-frames decode surface only when
   `return_ip`, `locals_base`, and `locals_count` are supplied honestly.
4. Arbitrary serialize/deserialize lane:
   extend public `state.deserialize` beyond scalar/string/bool only after the
   `Any` return boundary can materialize arrays, typed objects, and heap values.

## Static Check

Run after writing this report:

```bash
git diff --check -- docs/cluster-audits/wave31-state-carriers-resume-next-slice.md
```
