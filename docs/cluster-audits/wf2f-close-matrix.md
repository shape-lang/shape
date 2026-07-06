# Polyglot × distributed composition — authoritative matrix

> **2026-07-06 REWRITE (WF-3E, branch `wave3/distributed-composition-fix`).**
> The earlier WF-2F/WF-2G version of this file reported a fully-green 9-cell
> matrix that independent Fable verification **refuted**: on merged `main` the
> composition was BROKEN at the receiver (any `@remote` foreign transfer died
> "frame_descriptor has 0 slots but arity is N"; a transferred fn calling a
> stdlib module fn died "callee must be … got Null"; `remote::call`'s
> `Result` surface was a fiction that type-checked then crashed). WF-3E fixed
> the receiver/sender composition and this file is now the **genuine,
> from-scratch reproduced** matrix — every cell below was run on real `shape
> serve` loopback nodes at HEAD, not asserted from unit tests. Cells that do
> NOT fully resume are recorded honestly rather than rounded up to green.

Goal (user TOP priority): *"polyglot works with distributed computing together."*

Design: `docs/design/polyglot-distributed-integration.md` (ratified 2026-07-05).
Branch: `wave3/distributed-composition-fix`. HEAD at reproduction: `eabb960d`.

## Gates (finisher, this branch)

- `just check-clean` — EXIT 0 (workspace compiles clean; only the pre-existing
  `test_array_indexOf` snake_case warn).
- `just check-no-dynamic` — EXIT 0 (no forbidden dispatch symbols; the D1b
  receiver Bool-default sentinel was DELETED, not renamed).
- `scripts/verify-merge.sh` — 15/15 PASSED.
- `just test` (Tier 2, unit+deep) — only the 4 pinned pre-existing
  `shape-jit … mir_compiler::typedarray_ptr_regression_tests::jit_closure_capture_*`
  / `jit_two_closures_capture_distinct_arrays` failures; every other workspace
  crate green (0 new failures) with `--no-fail-fast`.
- `just diff-vmjit --fresh` — MATCH=466 (≥466), unexpected=0, 1 known-red
  pre-existing (`ACC__functions__finding_s1_unknown_hof_return_kind_confusion`).
  The serve-dependent matrix cells below are correctly NOT in the vm/jit corpus.

## How the matrix was produced (reproducible)

Real `shape serve` loopback nodes, extensions loaded from `./extensions/`:

```
# opted-in executor node (executes transferred/resumed foreign python+ts):
shape serve --address 127.0.0.1:22001 --sandbox none \
      --ffi-languages python,typescript --extension-dir ./extensions
# strict-empty node (refuse side — no dynamic language opted in):
shape serve --address 127.0.0.1:22002 --sandbox none --extension-dir ./extensions
```

Client (sender) runs `shape run --mode {vm,jit} <cell>.shape`. `extern C` uses
libc `labs`; python/ts foreign fns return `Result<int>` (dynamic runtimes can
fail on every call — a compile-time requirement). Each transfer cell was
confirmed genuine by the server-side log line
`[serve] inbound Call fn="…___impl" blobs=N foreign_entries=1` (N≥2 → the
foreign stub blob travelled alongside the `@remote` wrapper blob; it was NOT a
client-side local fallback).

## Authoritative 9-cell matrix (reproduced 2026-07-06 @ `eabb960d`)

| foreign | transfer (`@remote`) | snapshot→resume | combined (`@remote` + `snapshot()` mid-exec) |
|---------|----------------------|-----------------|-----------------------------------------------|
| **C**          | `42` vm+jit — `blobs=2 foreign_entries=1` | `99` → resume `RESUMED`/`99` | `105` (foreign both sides); `snapshot()`-in-remote = clean barrier `Err` |
| **python**     | `105` vm+jit — `blobs=2 foreign_entries=1` | `120` → resume `RESUMED`/`120` | `105` (foreign both sides); `snapshot()`-in-remote = clean barrier `Err` |
| **typescript** | `21` vm+jit — `blobs=2 foreign_entries=1` | `30` → resume `RESUMED`/`30` | `21` (foreign both sides); `snapshot()`-in-remote = clean barrier `Err` |

- **transfer** (D1 fix — CRITICAL) — the foreign entry travels as a dependency
  of the `@remote` wrapper's blob and executes on the serve node; the correct
  value returns to the sender. VM and JIT senders produce identical values.
  Server log `blobs=2 foreign_entries=1` on every cell.
- **snapshot→resume** — `snapshot()` between two foreign calls returns
  `Ok(Snapshot::Hash(id))` on the first run and `Ok(Snapshot::Resumed)` from a
  fresh `shape --resume <hash>` process; the pre-snapshot foreign result is
  restored and the tail value is byte-equal across the barrier. Sound on merged
  main (Fable-confirmed) and re-verified here — no regression.
- **combined** (honest status) — an `@remote` fn whose body calls a foreign fn,
  then calls `snapshot()` mid-execution **on the receiver**, then calls the
  foreign fn again. The foreign body executes on **both** sides of the barrier
  on the receiver (server log `blobs=3 foreign_entries=1`) and the arithmetic
  result is correct. `snapshot()` taken **inside the transferred remote frame**
  returns a **clean, surfaced barrier `Err`** — literally *"checkpoint could not
  be written: no execution context to snapshot. Nothing was saved; the program
  continues."* — because the receiver's transient per-call execution context is
  not a persistable top-level snapshot target (design §4.5: barrier refusals are
  surfaced, never silent corruption). This is **not** a persistable/resumable
  combined snapshot; it is a correct-computation + clean-refusal, recorded here
  as such rather than rounded up to green. A persistable snapshot INSIDE a
  remote-transferred frame is routed to the follow-up lane
  `wf3e-remote-inframe-snapshot-persist`.

## Receiver enforcement (axis C, §4.6 / OQ-6) — reproduced

- **Refuse cell** (strict-empty node, python NOT opted in): a transferred
  `fn python` is refused with a clean, distinguishable message — *"foreign call
  'padd': the server has not opted into the 'python' language runtime (opted-in
  ffi_languages: []); the operator must start `shape serve` with
  `--ffi-languages python` to allow it"*. This is DISTINGUISHABLE from the D1
  transfer bug (which was a `frame_descriptor` shape error), satisfying D6(a).
- **Opt-in cell** (node started `--ffi-languages python,typescript`): the same
  transfer EXECUTES the foreign body server-side (the transfer row above),
  satisfying D6(b) — the loaded language runtimes are wired into the executing
  engine.
- `--ffi-languages` / `--sandbox none` genuinely grants `ffi.call` (the opt-in
  flag grants the permission it gates on), satisfying D6(c).
- `extern C` is NOT language-gated (`Ffi` + `ffi_libraries`/`ffi_symbols` only).
- **Permission-over-wire (D5)**: a strict node refuses a transferred fn that
  calls `file::write_text` at LOAD with
  `Err(RemoteError::PermissionDenied { missing: [..fs.write..] })`; the write
  never hits disk. Transferred per-function blobs now carry their real derived
  `required_permissions` (namespace imports + callee-body permissions included).
- Zero sender trust: the receiver installs its own scope + flips
  `ffi_receiver_strict`; `Ffi` is unioned by the linker and enforced at
  load/call. All marshal rides typed `KindedSlot`/`NativeKind` carriers
  (ADR-006) — no ValueWord/tag-decode/bridge/Bool-default; `check-no-dynamic`
  EXIT 0.

## Committed regression coverage (added WF-3E)

`bin/shape-cli/src/commands/serve_cmd.rs` (in-process real serve nodes):

- `test_remote_foreign_extern_c_transfer_over_tcp` — the `blobs>=2` /
  foreign-non-empty transfer path (the audit-untested D1 regression), extern C
  runs server-side, returns 42.
- `test_remote_call_result_ok_and_err_over_tcp` — `remote::call` yields a real
  `Result`: live node → `Ok(42)`, dead port → recoverable `Err`, both arms
  reachable (D4).
- `test_remote_permission_refusal_over_wire` — strict node refuses a transferred
  `fs.write` fn at load; no file on disk (D5).

`crates/shape-vm/src/compiler/functions_foreign.rs`:

- `receiver_strict_refuses_dynamic_language_not_opted_in` /
  `receiver_strict_admits_dynamic_language_when_opted_in` /
  `receiver_strict_does_not_gate_extern_c_by_language` (D6 opt-in/refuse).

## Residuals routed to named follow-up lanes (pre-existing, NOT regressions)

- `wf3e-annotation-args-heap-carrier` — `@remote` with an `Array<T>`/object
  param is a compile error (`emit_annotation_args_array` packs all params into
  one homogeneous scalar `TypedArray`; a heap param has no scalar element
  carrier). Needs a heterogeneous annotation-args ABI.
- `wf3e-remote-global-capture` — an `@remote` fn reading a module-global
  returns 0: `build_minimal_blobs_by_hash` transfers FUNCTION blobs only;
  module-global DATA is never serialized/initialized on the receiver.
- `wf3e-remote-execute-projection` — `remote::execute` renders
  `WireValue::Integer(42)` as `{bindings, schemas}` via the JsonValue
  polymorphic projection (`vm_impl/modules.rs`); shared with json/yaml parse,
  high blast radius.
- `wf3e-remote-inframe-snapshot-persist` — persistable `snapshot()` INSIDE a
  remote-transferred frame (see combined row above).
- `wf3e-extension-version-hash` (D8) — `ForeignFunctionEntry::compute_content_hash`
  omits the extension version; the version lives only in the loaded runtime's
  descriptor (link/exec time), not at the compile-time hash. Needs the
  `ExtensionReq` wire-field mechanism (design A2).
