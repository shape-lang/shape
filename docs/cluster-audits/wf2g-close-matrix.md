# WF-2G close record — snapshot projection completeness (2026-07-06)

Goal (user TOP priority): *"resumability and distributed execution works …
polyglot works with distributed computing together."* Close the two remaining
snapshot-serialization **projection** gaps so the resumability × distributed
composition is fully persistable.

Design (binding, ratified 2026-07-05): `docs/design/snapshot-resume.md`,
`docs/design/distributed-function-transfer.md`,
`docs/design/polyglot-distributed-integration.md`.
Branch: `wave2/snapshot-completeness`. Base: WF-2F merge (`403fe04c`).
Both gaps are in the **projection** (runtime value → `SerializableVMValue`) +
its resume rebind, NOT the `SerializableVMValue` enum.

## Gaps closed

- **GAP A — ModuleFn projection** (`8fbe82f4`). A `Ptr(HeapKind::ModuleFn)`
  slot's bits are an inline-scalar module-fn id (a process-local index into
  `VirtualMachine::module_fn_table`; native Rust `Arc<dyn Fn>` bodies, NO
  content hash, re-registered deterministically on every node). The sound
  cross-process identity is the **qualified export name** `module::export`,
  carried by `SerializableVMValue::ModuleFunction(String)` — NOT a content hash
  (the content-hash carrier belongs to the transferred Shape-function
  Function/Closure path; foreign-function-ref *values* stay refuse-by-design per
  polyglot-distributed §4.10 A3(iii)/OQ7). Projection resolves id → name via an
  install-once thread-local table (`populate_module_objects`, ambient-resolver
  shape like `current_registry()`); restore resolves name → id on the resuming
  host; both **surface-and-stop** (never fabricate) when unresolvable. The id is
  an inline scalar ⇒ no share minted ⇒ balanced refcounts. Null-pointer guard
  exempts ModuleFn (id 0 is the first-registered fn, a valid value). This is the
  arm that previously made a `snapshot()` taken with receiver-populated ModuleFn
  bindings live return a clean **barrier `Err`** (the WF-2F combined-cell
  yellow, `snapstate=0`).

- **GAP B — heap-element arrays** (`b4bfad1c`). The projection of a runtime
  `TypedArray` whose elements are heap pointers (`Array<string>` /
  `Array<Decimal>` / `Array<TypedObject>`) previously hit an opaque/refuse arm;
  only scalar arrays round-tripped. Projection now walks the v2-raw element
  buffers (`*const StringObj` / `*const DecimalObj` / `*const TypedObjectStorage`)
  via typed `TypedArray::<Ptr>::as_slice` carriers into
  `Array(Vec<SerializableVMValue>)`; restore rebuilds the monomorphized
  `ELEM_TYPE_*` carrier with balanced share-accounting (fresh refcount=1 per
  element, transferred to the array). No new SV arm, no `SNAPSHOT_VERSION` bump.
  ADR-006 §2.3/§2.5/§2.7.5.1.

Both fixes read typed slots via typed `Arc` carriers + the parallel `NativeKind`
track. **No** ValueWord/ValueBits, tag-decode, Bool-default-for-`Load*Ptr`,
`is_heap()` probe, or raw-u64 slot reinterpretation. `just check-no-dynamic`
EXIT 0; `scripts/verify-merge.sh` 15/15.

## Finisher gates (this session, release build, worktree `shape-wf2g-snapshot-completeness`)

- `just check-clean` — EXIT 0 (only the pre-existing `test_array_indexOf`
  snake_case warn).
- `just check-no-dynamic` — EXIT 0 (no forbidden dispatch symbols).
- `scripts/verify-merge.sh` — **15/15 PASSED**.
- `just test` (Tier 2, unit+deep) — the only failures workspace-wide are the 4
  pinned pre-existing `shape-jit … jit_closure_capture_*`. Verified with an
  explicit `--no-fail-fast` re-run (cargo's default fail-fast otherwise stops at
  shape-jit, which sorts before the snapshot-bearing crates): **shape-runtime
  1492 / 0 failed** (incl. the new `wf2g_module_fn_projection_and_restore_round_trip`
  unit test), **shape-vm 2887 / 0 failed**, shape-ast 601, shape-abi-v1 41,
  shape-wire 60, all others 0 new failures. (The jit deep-tests SIGABRT under
  the `--no-fail-fast` concurrent run is the documented default-parallelism race,
  CLAUDE.md Known Constraints — not a regression; the isolated `just test` run
  reports the jit binary as 772 passed / 4 pinned.)
- `just diff-vmjit --fresh` — **MATCH=466, unexpected=0** (1 known-red
  pre-existing `ACC__…unknown_hof_return_kind_confusion`). VM == JIT preserved.

## Authoritative persist + resume evidence

| gap | scenario | evidence | result |
|-----|----------|----------|--------|
| **B** | `Array<string>` + `Array<Decimal>` + `Array<TypedObject>` built pre-checkpoint, read back post-checkpoint | **LIVE this session** — `shape run` → `Ok(Snapshot::Hash)`; fresh-process `shape --resume <hash>` | ORIGINAL `ALL_CHECKS_PASSED`; RESUMED `ALL_CHECKS_PASSED` (all 3 element types byte-identical, no SIGABRT at drop) |
| **A** | `Ptr(HeapKind::ModuleFn)` slot (id 0 + id 1) → `SV::ModuleFunction(name)` → id parity on restore; unresolvable name clean-refuses | **LIVE this session** — unit test `wf2g_module_fn_projection_and_restore_round_trip` (shape-runtime, PASS) | projected qualified name + restored id parity; unresolvable name refuses (never fabricates) |
| **A** | native/foreign ModuleFn binding in scope across `snapshot()` inside a remote-transferred fn (cell_c/py/ts) | committed release-binary fresh-process resume proof (`8fbe82f4`) | cell_c 42→99, cell_py 105→120, cell_ts 105→120 — `ALL_CHECKS_PASSED` |

### WF-2F combined cells (the yellow this closes)

The WF-2F yellow was exactly: `snapshot()` taken **inside a remote-transferred
function** returned a clean **barrier `Err`** (`snapstate=0`) because the
receiver-populated `HeapKind::ModuleFn` module bindings had **no**
`SerializableVMValue` arm. GAP A adds that arm (`SV::ModuleFunction`), so the
projection barrier root-cause is removed and such a snapshot now yields a
persistable `Ok(Snapshot::Hash)`.

| foreign | combined (`@remote` + `snapshot()` mid-exec) — pre-WF-2G | post-WF-2G projection status |
|---------|-----------------------------------------------------------|------------------------------|
| **C**          | executes correctly; `snapshot()` = barrier `Err` (`snapstate=0`) | ModuleFn projection arm present → persistable `Hash` (projection root-cause closed) |
| **python**     | executes correctly; `snapshot()` = barrier `Err` (`snapstate=0`) | ModuleFn projection arm present → persistable `Hash` |
| **typescript** | executes correctly; `snapshot()` = barrier `Err` (`snapstate=0`) | ModuleFn projection arm present → persistable `Hash` |

## Residual (routed to lane `wf2g-combined-live-reverify`)

The **live 3-node combined harness re-run** (`@remote` transfer → foreign call →
`snapshot()` mid-exec on the receiver → resume on a second node) was **not
independently reproduced to green this session**. The original WF-2F combined
programs were run ad-hoc and never committed; a from-scratch reconstruction
(extern-C `labs` combined cell against a live `shape serve --sandbox none`)
reached the receiver — the serve node accepted the foreign-bearing transferred
blob (`[serve] inbound Call fn="work___impl" blobs=2 foreign_entries=1`) — but
tripped an **orthogonal** `frame_descriptor has 0 slots but arity is 1` error in
the remote frame-setup path, which fires *before* `snapshot()` and is untouched
by the snapshot-serialization work in gaps A/B (a minimal single-arg `@remote`
fn transfers and returns correctly: `MIN_REMOTE_RESULT=42`). This is a
remote-frame-descriptor reconstruction issue for a transferred function carrying
locals, tracked separately.

Because of this, the combined cells' end-to-end persistability is substantiated
here at the **unit + committed-integration** level (GAP A unit test PASS + the
`8fbe82f4` committed release-binary resume proof) and by the projection
root-cause removal, but the **full live 3-node combined re-run remains a named
open verification lane** — this is why WF-2G closes at **yellow**, not green.
Gap B is fully live-proven green.
