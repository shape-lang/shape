# Snapshot / Resume — W17 Completion Design

**Status:** RATIFIED 2026-07-05 (user) — all recommended defaults adopted (overview §3 Q16–Q25 + Q3); no override touches this doc. See `00-priority-spine-overview.md` §Ratification record and the §8 record below. (rev 2 — revised same day against the three-lens adversarial review — resolutions inline, rebuttals in §5)
**Implements against:** audit `docs/cluster-audits/audit-2026-07-04-claimed-vs-real.md` §4.3; fix-plan `docs/cluster-audits/fix-plan-2026-07-05-workflows.md` WF-2B (+ interlocks with WF-2C, WF-2F, WF-1B)
**Binding constraints:** `CLAUDE.md` §Forbidden Patterns (full), ADR-006 (`docs/adr/006-value-and-memory-model.md`) §2.7.4/§2.7.5 (typed marshal), §2.7.7/Q9 (parallel kind track), §2.7.8/Q10 (cell kinds, no Bool-default), §2.7.10/Q11 + §2.7.11/Q12 (ABIs), §2.7.30 (reference serialization, RATIFIED 2026-05-29), `docs/runtime-v2-spec.md`
**Sibling designs:** `docs/design/ffi-rebuild.md` (foreign-frame opacity), `docs/design/distributed-function-transfer.md` (blob transport), `docs/design/polyglot-distributed-integration.md`

---

## 1. Goals & non-goals

### Goals

1. **`snapshot()` works end-to-end**: a Shape program calls `std::core::snapshot::snapshot()`, the VM state is persisted content-addressed, the *same process continues* with `Snapshot::Hash(id)`, and a later `shape --resume <hash>` restores the VM and continues from the suspension point with `Snapshot::Resumed` — evolving the stdlib contract shipped in `crates/shape-runtime/stdlib-src/core/snapshot.shape:13-19` to a `Result` return (§4.1.4, Open Question 8) so refusals are expressible in Shape's Result-based error model.
2. **Ctrl+C interrupt-save works**: first Ctrl+C during `shape run` persists a snapshot atomically and prints the resume command; second Ctrl+C force-exits (handler already does this, `script_cmd.rs:158-166`).
3. **Both resume entry points real**: `resume_snapshot` (same bytecode) and `recompile_and_resume` (edited source, hash-first frame relocation), replacing the two stubs at `crates/shape-vm/src/execution.rs:190-223`.
4. **Content-addressed persistence** that references per-function `FunctionBlob` hashes (a *CodeManifest*), so a resume can verify and fetch exactly the code it needs — composing with the distributed vertical (WF-2C/WF-2F) instead of pinning a monolithic program blob.
5. **`std::core::state` primitive set functional**: `hash`/`serialize`/`deserialize` correct (unblocked by the WF-1B marshal fix), `capture`/`capture_all`/`capture_module` returning real typed objects, `resume`/`resume_frame` restoring real state, `fn_hash`/`schema_hash`/`caller` complete.
6. **Explicit suspension barriers**: every VM state that cannot be soundly captured refuses with a *clean, structured, user-facing* error — never a garbage snapshot, never a Bool-default fabrication, never a leaked internal sentinel string.
7. **Determinism & limits across resume defined**: resource-limit counters, RNG state under `Deterministic`, permission re-verification at resume — all specified, not improvised.
8. **Cross-node resumability specified**: exactly what must match (blob hashes, schema registry, extension set, permission grant) for a snapshot taken on node A to resume on node B.

### Non-goals (explicitly OUT)

- **Live-loan continuation on a resumed VM** — restored references are replay-only per ADR-006 §2.7.30.5; runtime-loan re-establishment is v0.3.4+ (§2.7.30.6). This design does not widen it.
- **Cross-node `&mut` coherence** — v0.3.4+ per §2.7.30.6. Whole-VM restore preserves exclusivity by construction (one VM instance); nothing here introduces multi-instance loans.
- **Broad container/closure-env reference-escape flips** — the flipped sink set stays exactly `ReturnSlot` + `ModuleBinding` (§2.7.30.3/.6).
- **Serializing foreign (Python/TS/C) runtime state** — foreign frames are opaque suspension barriers (coordinated with the FFI design), never pickled.
- **Time-travel debugger UX** — the recorder (`dispatch.rs:208-221`) keeps working; this design only retires its `*const SnapshotStore` aliasing wart as a side effect of the new persistence seam.
- **A GC or new heap model** — capture/restore uses the existing Arc/refcount model and the existing SerializableVMValue wire tier.

---

## 2. Current state (recon 2026-07-05, file:line verified)

### 2.1 What is real (more than the stubs claim)

The stub rationale — "depends on the deleted ValueWord carrier" (`execution.rs:197-199`, `:217-219`) — is **stale/false at HEAD**. The §2.7.7 kinded round-trip machinery exists and passes unit tests:

- **Capture**: `VirtualMachine::snapshot()` (`crates/shape-vm/src/executor/snapshot.rs:146-242`) serializes the full operand stack + module bindings via `slot_to_serializable_ctx` with per-slot `NativeKind`, persisting parallel `stack_kinds` / `module_binding_kinds` tracks (`VmSnapshot` fields, `crates/shape-runtime/src/snapshot.rs:239-277`). This is exactly the ADR-006 §2.7.7/Q9 shape.
- **Restore**: `from_snapshot()` (`executor/snapshot.rs:262-388`) does the STAGE-R5 two-pass identity restore (`SerializeIdentityCtx` `shape-runtime/src/snapshot.rs:917`, `RestoreLinkCtx` `:954`, `materialize_cell_bodies` `:1747`) and `restore_call_stack` rebuilds frames with `OwnedClosureBlock` (`:409-511`). Round-trip unit tests pass (`executor/snapshot.rs:981-1370`).
- **References**: ADR-006 §2.7.30 is implemented — `RefTarget::PromotedCell` owning-Arc carrier (`shape-value/src/reference.rs:173-236`), B0003 suppression for exactly ReturnSlot+ModuleBinding (`mir/solver.rs:1177-1300`, trigger `:1451`), `SV::Reference{handle,is_mut}` + `SharedCell{handle,inner}` + `SharedCellRef{handle}` arms with the shared identity-map two-pass restore (`shape-runtime/src/snapshot.rs:~520-585`).
- **CLI plumbing complete**: three-way resume branch, snapshot-store loads, `ShapeError::Interrupted` consumer that prints the resume command (`bin/shape-cli/src/commands/script_cmd.rs:169-329`). It calls the stubs.
- **Store**: content-addressed `SnapshotStore` with `SNAPSHOT_VERSION`-stamped `ExecutionSnapshot{version, semantic, context, vm_hash, bytecode_hash}` (`shape-runtime/src/snapshot.rs:37,102-133,174-186`), engine-side `snapshot_with_hashes` (`engine/mod.rs:281`), chunked BlobRef sidecars for large payloads (`snapshot.rs:800-830`).
- **Identity resolution for recompile**: `resolve_function_identity` (`executor/snapshot.rs:38-97`) implements hash-first function resolution — consumer-complete, producer-dead (see 2.2.v).
- **In-loop snapshot precedent**: the time-travel recorder calls `self.snapshot(store)` live inside the dispatch loop (`dispatch.rs:208-221`) — proof the capture works mid-execution.

### 2.2 What is dead or broken

i. **Sentinel with zero consumers.** `SNAPSHOT_FUTURE_ID = u64::MAX` (`executor/mod.rs:62`), raised only by `BuiltinFunction::Snapshot` (`vm_impl/builtins.rs:304-310`) as `VMError::Suspended{future_id, resume_ip}`. The dispatch loop converts it to `ExecutionResult::Suspended` (`dispatch.rs:228-238`), but the host boundary `BytecodeExecutor::execute_program` collapses **every** `VMError` into `ShapeError::RuntimeError{e.to_string()}` (`execution.rs:565-581`) → user sees `Suspended on future 18446744073709551615`, exit 1, nothing persisted.
ii. **Both resume entry points are unconditional `Err` stubs** (`execution.rs:190-223`) citing deleted-ValueWord dependencies that no longer hold.
iii. **Ctrl+C saves nothing.** Interrupt flag wiring is complete (`script_cmd.rs:155-165`; VM checks every 1024 instructions, `dispatch.rs:156-158/321-323/462-464`) but `VMError::Interrupted` is collapsed by the same `execution.rs:567-581` sink; `ShapeError::Interrupted{snapshot_hash}` has **zero producers**.
iv. **Live Bool-default forbidden-pattern violation** at `executor/snapshot.rs:483`: the no-layout legacy-upvalue restore path passes `expected = NativeKind::Bool` while its own comment claims no Bool-default.
v. **Capture side never populates relocation fields**: `ip_blob_hash`/`ip_local_offset` never written (`executor/snapshot.rs:236-238`), per-frame `local_ip: None` (`:597`) — so recompile-and-resume relocation is producer-dead.
vi. **Restore gaps**: loop/timeframe/exception-handler stacks are exported but not restored (`executor/snapshot.rs:371-383`); `VmSnapshot.locals` always empty (`:206` — locals live in stack register windows, which *are* serialized; the field is redundant, see §4.2.3).
vii. **`std::core::state`**: `hash`/`serialize` bodies are real (`state_builtins/core.rs:440-455`) but poisoned upstream — `register_typed_function`'s variadic shim stamps `arg_kinds` all-Bool (`shape-runtime/src/marshal.rs:2284-2300`) → near-constant digests. `capture*` reads real live state via `VmStateSnapshot` (`vm_state_snapshot.rs:64-247`, kind-threaded) but the return arm surfaces (`introspection.rs:77-130`). `diff`/`patch` surface-and-stop (deleted 1486-LoC `state_diff` module, `core.rs:368-385`). Whole-VM `state.resume` decodes to an **empty** `VmSnapshot` (ip=0, empty call_stack/module_bindings — `resume.rs:365-390`); `apply_pending_frame_resume` (`resume.rs:233-343`) is fully implemented. `fn_hash` mostly wired, FunctionRef/TraitObject decode missing (`core.rs:457-520`).
viii. **Opaque restore arms** surface-and-stop: Iterator/Deque/Channel/FilterExpr reject (`shape-runtime/src/snapshot.rs:2332-2340`), Mutex/Lazy reject (`:2356-2360`). User rulings 2026-05-29 dictate their fates (§4.6.2).
ix. **Snapshots pin a monolithic program**: the whole serialized `BytecodeProgram` is stored under one `bytecode_hash`; `VmSnapshot`/`SerializableCallFrame` already carry per-frame `blob_hash: [u8;32]` fields, and `build_minimal_blobs_by_hash` (`remote.rs:454-489`) computes transitive blob closures — but nothing connects them. `shape-wire` has no snapshot/blob transport arms.
x. **NOT verified at HEAD**: whether a `from_snapshot`-restored VM actually resumes mid-function end-to-end. No integration test exercises resume-at-nonzero-ip with live frames (the deleted integration tests noted at `execution.rs:771-778` were never rebuilt). Stage 2's first task is building that proof (§6).

---

## 3. Constraints (binding, quoted)

The following rules bind every line of this design. Where the design touches their territory, §4 cites the rule.

1. **No ValueWord under any rename** — CLAUDE.md §Forbidden Patterns: "*`ValueWord` at runtime. Deleted. Do not reintroduce as a 'shim', 'bridge', 'compatibility layer', or 'serialization helper'. Snapshot/wire uses per-slot kind metadata.*" The stub comments' framing ("depends on the deleted ValueWord carrier") is a trap: the rebuild is orchestration over the existing §2.7.7 machinery, not carrier work.
2. **Parallel kind track is the wire shape** — ADR-006 §2.7.7/Q9: snapshot/wire serialization uses "*parallel `Vec<u64>` data + `Vec<NativeKind>` kinds*". Already the implemented shape (`VmSnapshot.stack_kinds`/`module_binding_kinds`). Forbidden: `Vec<KindedSlot>` for the stack, 16-byte slots, packed tag bits, `Option<NativeKind>`/`Unknown` placeholders.
3. **No Bool-default kind fabrication** — ADR-006 §2.7.8/Q10: no Bool-default for cell/capture kinds; "*surface-and-stop with `NotImplemented(SURFACE)` instead*". The live violation at `executor/snapshot.rs:483` is retired by this design (§4.2.4).
4. **Typed marshal only** — ADR-006 §2.7.4/§2.7.5: all host↔VM crossings via `KindedSlot`/`NativeKind` carriers per FieldType. Method ABI §2.7.10/Q11; value-call ABI §2.7.11/Q12. No raw-u64 kind-blind slices, no kind synthesis from raw bits.
5. **SerializableVMValue lockstep** — ADR-006 §2.7.5.1: the SV arm set is in 4-table lockstep with `HeapKind`; new HeapKinds require lockstep wire arms (enforced by `scripts/verify-merge.sh`).
6. **`SerializableVMValue` is WIRE-tier only** — ADR-005 §1: no new parallel *runtime* discriminator. SV never flows on hot paths; it exists only inside serialize/deserialize.
7. **No `from_heap_arc` catch-all** — ADR-006 Q6: per-FieldType constructors only.
8. **`KindedSlot` must not leak into the typed VM↔JIT slot ABI** — ADR-006 §2.7/`docs/runtime-v2-spec.md`. Snapshot capture/restore lives on the runtime tier (GENERIC_CARRIER sites), not in JIT-compiled code.
9. **Reference serialization is §2.7.30, verbatim** — §2.7.30.5: identity-handle + shared `restore_identity_map`, replay-only, `is_mut` carried-not-read; §2.7.30.7: "*NO non-owning reference carrier … NO new ValueWord-shape carrier / 'serialization helper' / 'reference marshal bridge' … NO Bool-default; NO raw-pointer-token … NO silent broad-scope creep.*" This design **reuses** that identity-map machinery for all aliased-identity restore; it does not invent parallel identity.
10. **Share-accounting discipline** — explicit `clone_with_kind` retain **before** `KindedSlot::new` claim (the W5 UAF fix pattern, `vm_state_snapshot.rs:297-326`; same shape as `executor/mod.rs:792`).
11. **No NEW ConcreteReturn-projection surface** — *design decision of this doc, not a quoted binding ruling* (earlier drafts wrongly cited "ConcreteReturn is bulldozed"; `ConcreteReturn`/`TypedReturn` are alive at HEAD in 49 files and are the current state-builtin return ABI — `typed_module_exports.rs:55/:196`, `state_builtins/core.rs:444-452`). Rule as adopted here: all NEW return-arm marshal (state.capture* projections, the `Snapshot` enum markers) is KindedSlot-direct; the existing `Result<TypedReturn, String>` state-builtin seam is migrated in Stage 5 per §4.6.3.1 (Open Question 9), not extended with new `ConcreteReturn` arms.
12. **User-ruled opaque-arm dispositions (2026-05-29, binding)**: Iterator / Deque / FilterExpr = clean-refuse; Channel = clean-refuse; Mutex / Lazy = defined-reset; PriorityQueue = serialize; Reference / SharedCell = identity-handle replay-only. §4.6.2 encodes this table; it is not re-litigated here.
13. **Forbidden-rename regex** — any proposal matching `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` is refused on sight. Named components in this design are: *SuspensionOutcome* (enum), *CodeManifest* (persistence struct), *persist_execution_state* (free function), *SnapshotError* (structured refusal enum, §4.1.4; barrier refusals are its `Barrier{reason}` variant). None is a bridge/shim/adapter by name or by function.

---

## 4. Design

### 4.0 Overview — one suspension spine, two consumers

All pause/persist behavior hangs off **one spine**: the dispatch loop already distinguishes suspension-class `VMError`s from real errors (`dispatch.rs:228-244` handles `Suspended` and `ResumeRequested`). The design adds two consumers at the two tiers where each belongs:

- **In-loop consumer (dispatch shell)** for `Suspended{SNAPSHOT_FUTURE_ID}`: capture → persist → push `Snapshot::Hash(id)` → **continue executing**. Required because the stdlib contract (`snapshot.shape:16-17`) says the snapshotting run *returns* `Snapshot::Hash(id)` and keeps going. A host-boundary consumer cannot continue (the VM has already unwound out of `execute`).
- **Host-boundary consumer (`execution.rs` Err-branch)** for `Interrupted`: capture → persist → return `ShapeError::Interrupted{snapshot_hash: Some(hash)}`. Ctrl+C *terminates* the run, so persist-and-exit at the boundary is correct, and the CLI consumer already exists (`script_cmd.rs:307-329`).

Resume is the mirror image: rebuild the VM from `VmSnapshot` via the existing `from_snapshot`, push `Snapshot::Resumed`, run the normal dispatch loop from `resume_ip`, project the completion through the existing kinded `wire_conversion::slot_*` helpers (`execution.rs:595-608`).

```
  run:      dispatch loop ──snapshot()──▶ VMError::Suspended{SNAPSHOT_FUTURE_ID, resume_ip}
                 │                              │ (in-loop consumer, §4.1)
                 │                     vm.snapshot(store) → VmSnapshot
                 │                     persist_execution_state(...) → hash
                 │                     push Snapshot::Hash(hash)   [kinded enum, §4.1.3]
                 ◀──── ip = resume_ip, continue ┘

  Ctrl+C:   dispatch loop ──flag──▶ VMError::Interrupted
                                        │ (host-boundary consumer, §4.4)
                              vm.snapshot(store) → persist → ShapeError::Interrupted{hash}

  resume:   store.load(hash) → verify CodeManifest (§4.3) → link blobs → from_snapshot
            → push Snapshot::Resumed → dispatch loop from resume_ip → kinded projection
```

### 4.1 The in-loop suspension consumer

**Site:** a new arm in the dispatch shell beside the existing `ResumeRequested` branch (`dispatch.rs:241-244`), replacing the blind `return Ok(ExecutionResult::Suspended{..})` for the `SNAPSHOT_FUTURE_ID` case. Real async future IDs (≠ `SNAPSHOT_FUTURE_ID`) keep the existing return path untouched.

**Flow (all inside the loop, VM intact):**

1. Match `VMError::Suspended{future_id: SNAPSHOT_FUTURE_ID, resume_ip}`.
2. **Barrier check** (§4.6): if any suspension barrier is active, construct `Err(SnapshotError::Barrier{reason})` as the intrinsic's return value (§4.1.4) — a kinded Result enum value pushed onto the stack — set `self.ip = resume_ip`, and `continue`. The program observes the refusal through the ordinary Result channel, handles it, and keeps running (e.g. skips this checkpoint). No partial persist, nothing written.
3. Set `self.ip = resume_ip` **before** capture, so the persisted `VmSnapshot.ip` is the post-`snapshot()`-call instruction — the resumed VM continues *after* the call site with the marker as the call's result.
4. `let snap = self.snapshot(store)?` — the existing §2.7.7 capture (`executor/snapshot.rs:146-242`), extended per §4.2.
5. `let hash = persist_execution_state(store, &envelope_seed, ctx, snap, &self.code_manifest)?` — new free function in shape-runtime (§4.3.4). Takes the `SnapshotStore`, the host-installed `SnapshotEnvelopeSeed` (§4.3.4 — the engine-owned envelope halves the loop cannot reach), the live `Option<&mut ExecutionContext>` (already threaded through the dispatch loop as `ctx.as_deref_mut()`), the `VmSnapshot`, and the VM-cached `CodeManifest`. Returns the envelope `HashDigest`.
6. Build the `Ok(Snapshot::Hash(hash_hex))` marker (§4.1.3) and push it kinded: `push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))` — the value the bytecode after `resume_ip` expects as `__intrinsic_snapshot()`'s return.
7. `continue` the dispatch loop.

**Store access.** The VM gets a `snapshot_store: Option<Arc<SnapshotStore>>` handle installed by the host before `execute` (mirroring how the time-travel recorder obtains its store, but owned via `Arc` instead of the current `*const SnapshotStore` reinterpretation at `dispatch.rs:210` — that unsafe aliasing wart is retired by this change; the recorder switches to the same `Arc` handle). **Who installs a store:** `shape run` AND `shape repl` install the default user snapshot store (the same store the `--resume` path reads) — the first place a user tries the book example must work. The opt-out set is exactly: embedded hosts via the library API that do not call `set_snapshot_store`, and `Deterministic`-sandboxed evaluation contexts that deliberately forbid persistence. In those contexts `snapshot()` returns `Err(SnapshotError::Barrier{NoStore})`, whose rendered message (§4.11) tells the host developer which API call enables it. To be explicit (clarifying touch per the 2026-07-05 ratification, overview §4.7): the qualifier is load-bearing — only `Deterministic` contexts that *forbid persistence* opt out via `NoStore`; a deterministic sandbox with a store configured snapshots fine, and §4.7.2/T11's deterministic capture semantics (RNG stream + virtual clock persisted) are authoritative. `Deterministic` does NOT imply "no snapshots".

**Failure atomicity.** Steps 4–5 are all-or-nothing: any capture or persist failure is converted to `Err(SnapshotError::...)` and pushed as the intrinsic's return value (exactly one Result value is pushed on every path — success, barrier, or persist failure), and no envelope is written (§4.3.5 write ordering guarantees no dangling references even if chunks landed). Internal invariant violations (kind-track corruption, share-accounting failure) remain hard `VMError`s — they indicate VM bugs, not user-handleable conditions.

### 4.1.4 The `snapshot()` contract becomes `Result` (stdlib contract change — Open Question 8)

Shape has no `catch` (CLAUDE.md §Error Handling: Result types, not exceptions), and the shipped signature `pub fn snapshot() -> Snapshot` (`crates/shape-runtime/stdlib-src/core/snapshot.shape:18`) has **no error channel** — under it, every barrier refusal would have to be an uncatchable runtime abort, killing an hour-3 computation because a Channel happened to be reachable. That is unacceptable for the primary checkpointing use case. This design therefore changes the stdlib contract:

```
pub enum SnapshotError {
    Barrier(reason: string),        // structured reason name + rendered detail, §4.11 catalog
    PersistFailed(detail: string),  // store I/O — nothing written (atomicity, §4.3.5)
}
builtin fn __intrinsic_snapshot() -> Result<Snapshot, SnapshotError>;
pub fn snapshot() -> Result<Snapshot, SnapshotError> { __intrinsic_snapshot() }
```

`Snapshot` itself stays `Hash(string) | Resumed`. Callers write `match snapshot()? { ... }` or handle the `Err` and continue without a checkpoint. **This is a user-visible contract change** — flagged for ratification as Open Question 8 rather than silently applied. Justification: the shipped signature has never worked (the feature is a dead stub per the 2026-07-04 audit), so no working program breaks; and the alternative — barrier = program termination — contradicts Goal 6's "clean, structured, user-facing" refusals. The Rust-side `SnapshotError` enum is the single refusal namespace for the whole design (resume-time refusals are variants of the same enum, §4.11), giving consistent naming by construction.

### 4.1.2 Why in-loop and not host-boundary for `snapshot()`

The host boundary (`execution.rs:565`) sees the error only after `vm.execute` returns — VM stack/frames are still alive in `vm`, so a boundary consumer *could* capture. But it cannot **continue**: re-entering `vm.execute` mid-frame is exactly the "run the suspend/resume loop" machinery the legacy stub describes, and doing it at the boundary duplicates the dispatch loop's re-entry logic. The dispatch shell already owns re-entry (`ResumeRequested` → `apply_pending_resume` → `continue`, `dispatch.rs:241-244`). One spine, two consumers — not two spines.

### 4.1.3 The `Snapshot` marker values (kinded, no ConcreteReturn)

`Snapshot` is a Shape enum (`crates/shape-runtime/stdlib-src/core/snapshot.shape:6-11`): `Hash(string)` | `Resumed`; the intrinsic's return is `Result<Snapshot, SnapshotError>` (§4.1.4). Markers are constructed exactly as compiled Shape code constructs enum values at HEAD: look up the enum schemas in `program.type_schema_registry` (schema ids resolved once at program load and cached beside `SNAPSHOT_FUTURE_ID` handling), allocate the variant payloads via the production typed-object allocation path (the `_new` v2-raw carrier — the same allocator-pairing discipline the W5 fixture fix enforced), and push the outer `Result` value with `NativeKind::Ptr(HeapKind::TypedObject)`. `Ok(Hash(string))` embeds the hex digest as the payload string.

- No `create_typed_enum_nb`, no `nb_to_wire`, no ValueWord — the construction is the ordinary kinded enum-value path (Constraint 1, 11).
- Share accounting per Constraint 10: retain before claim; the pushed slot owns exactly one strong share.
- The resumed side (§4.5) pushes `Ok(Snapshot::Resumed)` the same way before entering the loop.

### 4.2 Capture set — what a snapshot contains

`VirtualMachine::snapshot()` today captures most of this; the deltas are marked **[NEW]**.

| # | State | Carrier | Notes |
|---|-------|---------|-------|
| 1 | Operand stack (incl. register windows) | parallel `Vec<u64>` + `Vec<NativeKind>` → `Vec<SerializableVMValue>` + `stack_kinds` | §2.7.7/Q9 shape, already implemented (`executor/snapshot.rs:146-242`) |
| 2 | Call frames | `SerializableCallFrame{return_ip, locals_base, locals_count, function_id, upvalues, blob_hash, local_ip}` per frame; top-level ip relocation via `VmSnapshot.{ip_blob_hash, ip_local_offset, ip_function_id}` (`shape-runtime/src/snapshot.rs:250-260, 279-294`) | **[NEW]** producer populates the producer-dead fields at capture (§4.2.2): per-frame `local_ip` (today `None`, `executor/snapshot.rs:597`) and the top-level `ip_*` trio (today never written, `:236-238`). Per-frame `blob_hash` is **already written** today (`:591`). All fields exist; nothing new added for relocation |
| 3 | Module bindings | names + `Vec<u64>` + `module_binding_kinds` | implemented |
| 4 | Closure cells / captures | `OwnedClosureBlock` layout-driven per-capture SV + `layout.capture_native_kind(i)` | implemented for layout-carrying closures; no-layout path fixed per §4.2.4 |
| 5 | References & SharedCells | `SV::Reference{handle,is_mut}` / `SharedCell{handle,inner}` / `SharedCellRef{handle}` + shared `restore_identity_map` | implemented (§2.7.30.5); this design adds round-trip *integration* tests only |
| 6 | Loop / timeframe / exception-handler stacks | already exported | **[NEW]** restore side (`executor/snapshot.rs:371-383` documented follow-up) — required for correctness of resume inside `for`/`loop`/`try`-guarded regions |
| 7 | Instruction pointer | `VmSnapshot.ip` = `resume_ip` (§4.1 step 3) | implemented |
| 8 | Resource-limit counters | **[NEW]** `instruction_count`, output-bytes-used, memory high-water, and cumulative wall-clock `elapsed_at_capture` (a duration) are all persisted; absolute wall-clock *timestamps* are not — the timer restarts at resume against the remaining budget (§4.7.1) |
| 9 | Deterministic-sandbox state | **[NEW]** RNG stream state + virtual-clock offset, captured **iff** `Deterministic` permission active (§4.7.2) |
| 10 | Required permissions + scope constraints | **[NEW]** union from the `CodeManifest` (§4.3.2); re-verified at resume |
| 11 | Pending async tasks | **NOT captured** — non-quiescent async state is a suspension barrier in v1 (§4.6.1); recorded here so the omission is explicit, not accidental |
| 12 | Large payload sidecars (TypedArray / DataTable) | chunked BlobRefs via existing SNAPSHOT chunk machinery (`snapshot.rs:800-830`) | implemented |

#### 4.2.2 Frame relocation fields (producer side)

The relocation fields live in two places (both exist at HEAD, `#[serde(default)]`): per-frame `SerializableCallFrame.{blob_hash, local_ip}`, and top-level `VmSnapshot.{ip_blob_hash, ip_local_offset, ip_function_id}` for the suspension ip itself. At capture: for every live frame, resolve `function_id → FunctionHash` via the VM's `function_hashes` map (rebuilt at load, `vm_impl/program.rs:8-52,126-220`) — `blob_hash` is already written today (`executor/snapshot.rs:591`) — and compute `local_ip = frame_ip − blob_entry_offset` (today hardwired `None`, `:597`); for the top level, write `ip_blob_hash` = the hash of the function owning `VmSnapshot.ip` and `ip_local_offset` = its blob-relative offset (today never written, `:236-238`). No new wire fields are needed for relocation. This turns `resolve_function_identity` (`executor/snapshot.rs:38-97`) from consumer-complete/producer-dead into a working pair, and is the entire enabler for recompile-and-resume (§4.5.2) and cross-node resume (§4.8).

#### 4.2.3 `VmSnapshot.locals` — removed as a carrier, kept as a view

Locals live in stack register windows (`base_pointer..base_pointer+locals_count`), which row 1 already serializes with kinds. The always-empty `VmSnapshot.locals` field (`executor/snapshot.rs:206`) is **redundant as a restore carrier** and stays empty-and-deprecated at the wire tier (it is removed at the Stage-2 `SNAPSHOT_VERSION` bump, §4.3.3). `std::core::state`'s `Frame.locals` view (§4.6.3) is *derived at capture time* by slicing the stack via `base_pointer`/`locals_count` — a projection, not a second source of truth. **This resolves recon gap 7 by construction, not by replay**: no replay-correctness proof is needed because locals are bit-identically restored as part of the stack.

#### 4.2.4 Retiring the Bool-default violation (`executor/snapshot.rs:483`)

The no-layout legacy-upvalue restore path fabricates `expected = NativeKind::Bool`. Per Constraint 3 this becomes surface-and-stop:

- Restore of a no-layout upvalue whose SV arm does not self-describe its kind → internal sentinel `VMError::NotImplemented("SURFACE: from_snapshot frame[i] legacy-upvalue[j] has no closure layout; kind cannot be derived — ADR-006 §2.7.8")`. That string is the **internal** sentinel only; the user-rendered message follows the §4.11 rendering rule (plain language + remediation, no `SURFACE:`/ADR jargon).
- Mitigation that makes the surface nearly unreachable: SV arms are self-describing for every heap variant and every scalar except raw bit-patterns; the capture side (which *has* the kind from the closure block) additionally records the per-upvalue `NativeKind` in the serialized frame for no-layout closures — a **new** `SerializableCallFrame.upvalue_kinds: Option<Vec<NativeKind>>` wire field, so restore never needs to guess. Because the snapshot wire encoding is bincode (non-self-describing — `#[serde(default)]` does not make a missing trailing field decodable) and snapshot producers are live at HEAD (time-travel recorder `dispatch.rs:208-221`), **this field lands together with a `SNAPSHOT_VERSION` bump in Stage 0** (§4.3.3). A snapshot from an older version → clean version refusal (`SnapshotError::VersionMismatch`), never Bool.
- `just check-no-dynamic` grows a grep for `NativeKind::Bool` used as a `serializable_to_slot` `expected` argument outside test code (exact pattern set in Stage 0, §6).

### 4.3 Persistence format — content-addressed CodeManifest

#### 4.3.1 Today's envelope, kept

`ExecutionSnapshot{version: SNAPSHOT_VERSION, created_at_ms, semantic_hash, context_hash, vm_hash, bytecode_hash, script_path}` (`shape-runtime/src/snapshot.rs:169-186`; the version constant is `SNAPSHOT_VERSION`, `:37` — earlier drafts miscalled it FORMAT_VERSION) stays the envelope. Additions and one replacement:

```
ExecutionSnapshot (Stage-2 shape) {
    version:        SNAPSHOT_VERSION,         // bump schedule in §4.3.3
    created_at_ms:  i64,                      // <unchanged, already present> — feeds `shape snapshot list` (§4.12.2)
    script_path:    Option<String>,           // <unchanged, already present> — feeds `shape snapshot list`
    semantic_hash:  HashDigest,               // <unchanged>
    context_hash:   HashDigest,               // <unchanged>
    vm_hash:        Option<HashDigest>,        // → VmSnapshot (as today)
    code_manifest:  Option<HashDigest>,        // → CodeManifest  [REPLACES monolithic bytecode_hash as the authoritative code reference]
    bytecode_hash:  Option<HashDigest>,        // transitional twin: whole-program blob, written Stages 1–5, dropped at Stage 6 close (§6)
    label:          Option<String>,            // reserved now for snapshot-management tooling (§4.12.2 / Open Question 10) — avoids a later bump
}
```

#### 4.3.2 CodeManifest

A new content-addressed struct (shape-runtime, beside `ExecutionSnapshot`):

```
CodeManifest {
    program_root_hash:   [u8;32],             // hash of the manifest's own blob list (defined below)
    blobs:               Vec<[u8;32]>,        // sorted FunctionBlob content hashes — the transitive
                                              // closure via build_minimal_blobs_by_hash (remote.rs:454-489)
    entry:               FunctionHash,        // blob containing VmSnapshot.ip's function
    schema_registry_hash: [u8;32],            // content hash of the serialized type-schema registry
                                              //   (TypedObject schema_ids are program-relative → must pin)
    closure_layouts_hash: [u8;32],            // content hash of closure_function_layouts
                                              //   (required by restore_call_stack)
    foreign_entries:     Vec<[u8;32]>,        // content hashes of ForeignEntryObjects (polyglot programs) —
                                              //   adopted from polyglot-distributed-integration.md amendment A2 (its §4.10);
                                              //   each stored as its own content-addressed object (§4.3.5 step 2)
    extensions:          Vec<ExtensionReq>,   // ExtensionReq{language_id, extension_name, semver_req} — the A2
                                              //   shape, owned by polyglot-distributed-integration.md §4.3.1;
                                              //   observed_version recorded informationally BESIDE each req
                                              //   (diagnostics only, not a match input). Deliberately NO
                                              //   abi_version (ABI is a node property enforced by each host's
                                              //   own loader gate, not a traveling program fact) and NOT a .so
                                              //   content hash: an artifact hash is platform/build-specific and
                                              //   would make cross-node resume impossible by construction
                                              //   (defeats Goal 8)
    dormant_callback_registrations: u32,      // A2(iii): count of live C-callback registrations at snapshot
                                              //   time — drives the one-time re-registration notice at resume
                                              //   (semantics owned by the integration design §4.5)
    required_permissions: PermissionSet,      // linker union (total_required_permissions,
                                              //   content_addressed.rs:540) + ScopeConstraints
}
```

> **Cross-doc note:** an earlier sketch here (`ExtensionReq{name, abi_version, content_hash}`) was superseded by the integration design's amendment A2 (`polyglot-distributed-integration.md` §4.3 + §4.10) before ratification; this doc adopts A2 **verbatim and in full** — including the deliberate *absence* of `abi_version`, the `foreign_entries` + `dormant_callback_registrations` fields, and the generalization of the sketched `SnapshotExtensionMissing` refusal to `SnapshotError::CapabilityMissing{sub_kind, language_or_library, needed, found}` (A2(iv); sub-kinds `MissingRuntime | VersionSkew | AbiSkew | MissingNativeLibrary`) — so the user ratifies exactly one manifest shape. Extension matching semantics (semver constraint satisfaction, host-local ABI gate, default policy) live in that doc, not here. The full cross-design ledger is §4.10.

Each `FunctionBlob` is stored as its **own** content-addressed store object (key = its `content_hash`, which already covers permissions, `content_addressed.rs:122-142`). The schema registry and closure-layout table are stored as their own objects too.

**What this buys:** dedup across snapshots of the same program (N snapshots share blobs); per-function fetch on another node (WF-2C transport carries blobs, not programs); permission re-verification at blob granularity (hash mismatch = tamper or version-drift, caught before execution); and the polyglot composition surface WF-2F needs (`extensions` + `foreign_entries` travel with the manifest).

**What it costs:** a `SNAPSHOT_VERSION` bump (Stage 2, §4.3.3) and a staged migration (monolithic hash written as a transitional twin through Stage 5, dropped at Stage 6 close, so same-node resume lands before the manifest work completes). Recommendation over the recon's open choice (monolithic vs blob-graph): **blob-graph**, staged — this is the priority-spine ruling's "polyglot must compose with distributed computing" made concrete. (Open Question 1 asks for ratification.)

#### 4.3.3 Versioning

The snapshot wire encoding is bincode (`snapshot.rs:102-123`) — non-self-describing, so **every wire-shape change requires a `SNAPSHOT_VERSION` bump** (`#[serde(default)]` cannot paper over missing fields at decode). An earlier draft promised "exactly one bump"; that was inconsistent with the staged plan and is withdrawn. The bump schedule is:

- **Stage 0**: per-frame `upvalue_kinds` field (§4.2.4) — producers are live at HEAD, so the shape change cannot ride under the current version.
- **Stage 2**: new envelope (`code_manifest`, `label`, `locals` removal) + resource counters + deterministic state.
- **Stage 5**: new SV arms — Mutex/Lazy inner-payload (today `MutexOpaque`/`LazyOpaque` discard the payload, `snapshot.rs:2356-2360`) + PriorityQueue serialize arm (§4.6.2).

The Stage-5 version is the one v0.3.3 ships and stabilizes on. Loading any older-version snapshot: clean refuse with both versions named (`SnapshotError::VersionMismatch{found, supported}`). No silent migration in v1; pre-stabilization snapshots are development artifacts (Open Question 5 covers post-stabilization migration policy).

#### 4.3.4 `persist_execution_state`

Free function in shape-runtime (callable from the dispatch shell with what the loop already has — Constraint 8 keeps this off the JIT ABI). `engine.snapshot_with_hashes` (`engine/mod.rs:281-322`) consumes **engine-owned** state the loop cannot reach through `ExecutionContext`: `self.exported_symbols` (→ `SemanticSnapshot`), `self.script_path`, `self.runtime.persistent_context()`, and it writes back `self.last_snapshot`. The design closes that gap with a host-installed seed:

```
SnapshotEnvelopeSeed {                       // installed on the VM by the host beside the Arc<SnapshotStore>,
    semantic_hash: HashDigest,               //   at program load (exported_symbols are fixed after load; the
    script_path:   Option<String>,           //   host writes the SemanticSnapshot object once, idempotently)
}

persist_execution_state(
    store: &SnapshotStore,
    seed: &SnapshotEnvelopeSeed,
    ctx: Option<&mut ExecutionContext>,      // context envelope half; None (embedded paths without a
                                             //   persistent context) → Err(SnapshotError::Barrier{NoPersistentContext})
    vm_snapshot: VmSnapshot,
    manifest: &CodeManifest,                 // cached on the VM at program load
) -> Result<HashDigest, SnapshotError>
```

It absorbs the store-writing body of `snapshot_with_hashes`; the engine method becomes a thin delegator (builds the seed from its own fields, forwards, then updates `self.last_snapshot` — the in-loop path does not touch engine state; the CLI learns the hash from the pushed `Snapshot::Hash` / the `ShapeError::Interrupted` payload, not from `last_snapshot`). Blobs/schema-registry/layout objects are written idempotently (content-addressed put is a no-op if present).

#### 4.3.5 Write ordering & atomicity

Store writes are staged so a crash at any point never yields a referenced-but-missing object:

1. chunk sidecars → 2. FunctionBlobs + schema registry + layouts → 3. `CodeManifest` → 4. `VmSnapshot` → 5. `ExecutionSnapshot` envelope (temp-file + atomic rename — the envelope is the *only* entry point, so a missing envelope = no snapshot, and everything below it is already durable).

Garbage from aborted persists is unreferenced content-addressed data — safely collectible by an offline `shape snapshot gc` (out of scope, noted for tooling).

### 4.4 Ctrl+C interrupt-save

**Producer site:** the `execution.rs:565-581` Err-branch stops collapsing suspension-class errors. New match order:

```
Err(VMError::Interrupted) => {
    match capture_and_persist(&mut vm, ...) {           // same §4.2 capture + §4.3.4 persist
        Ok(hash) => Err(ShapeError::Interrupted { snapshot_hash: Some(hash.to_hex()) }),
        Err(e)   => Err(ShapeError::Interrupted { snapshot_hash: None }),   // no-save message names the barrier (§4.11); CLI consumer at script_cmd.rs:320-322
    }
}
Err(VMError::Suspended { future_id: SNAPSHOT_FUTURE_ID, .. }) =>
    unreachable-by-construction after §4.1 (in-loop consumer); if reached, map to a clean
    ShapeError naming snapshot(), never the raw "Suspended on future 18446744073709551615" string.
Err(e) => <existing collapse>
```

- **Interrupt ip must point at the un-executed instruction — NOT valid by construction today.** At HEAD the dispatch loop *fetches* the instruction at `self.ip` (`dispatch.rs:130`), increments `self.ip` (`:153`), and only then observes the interrupt flag and returns `Err(VMError::Interrupted)` (`:157-158`) — **before the fetched instruction executes**. `vm.snapshot()` records `snapshot_ip() = self.ip` (`executor/snapshot.rs:520`), so a naive boundary capture would persist `ip = (unexecuted instruction)+1` and resume would silently **skip one instruction** — silent state corruption. The design rule: **the interrupt-flag check moves to the top of the loop iteration, before the fetch** (and identically at the other two check sites, `dispatch.rs:321-323`/`:462-464`), so at observation time `self.ip` is exactly the next-unexecuted instruction and `VmSnapshot.ip` is a valid resume point. (Equivalent fallback if the check cannot move: the Interrupted path captures at `self.ip − 1`.) T4 gains an explicit no-skip/no-duplicate probe (§7).
- **Interrupt observation is gated on `foreign_reentry_depth == 0`.** A Shape callback invoked *from* foreign code runs a nested dispatch loop that would otherwise observe the flag while a foreign frame is live (the re-entry path `polyglot-distributed-integration.md` §4.5 instruments with the `foreign_reentry_depth` counter). If the nested loop raised `Interrupted`, it would (a) unwind through the foreign runtime and risk resurfacing as a *catchable foreign-call error* under the FFI error-class mapping — Ctrl+C swallowed by user code — and (b) by the time the boundary consumer captured, the foreign frame would be unwound (`foreign_reentry_depth` back to 0) so the §4.6.1 foreign-frame barrier could not fire, yet the outer frame's ip would sit past a partially-executed foreign call whose result was never pushed — a "clean" snapshot of torn state. Rule: while `foreign_reentry_depth > 0`, the flag stays set but is **not observed**; the outer loop's first check after the foreign call returns performs the capture. Consequence: `VMError::Interrupted` can never unwind through a foreign runtime, so no unwind-opacity machinery is needed.
- **Termination is immediate once the flag is observed** — there is no "wait for barrier-free" loop. If capture then hits a persistent barrier (e.g. a module-scope Channel alive for the whole run), the run terminates *now* with `ShapeError::Interrupted{snapshot_hash: None}` and the no-save message names the offending barrier (§4.11) so the user learns why nothing was saved. The only deferral is the foreign-call gate above (flag simply not yet observed); a never-returning foreign call means the second Ctrl+C force-exit (already implemented) is the escape hatch and nothing is saved — documented behavior, matching `ffi-rebuild.md` §4.12's designed v1 decline of cooperative cancellation (its Open Question 11 puts that decline to the user; Open Question 7 here defers to it).
- **Exit code**: an interrupted run exits **130** (128+SIGINT convention) whether or not a snapshot was saved — scripts can distinguish "completed" (0) from "interrupted-with-save" (130 + hash printed) from "interrupted-no-save" (130 + barrier-named message). T4 asserts 130, not 0.
- **Atomicity**: §4.3.5 ordering; first Ctrl+C never leaves a corrupt store.
- The CLI consumer (`script_cmd.rs:307-329`) changes only its exit-code path (130) — message plumbing already exists.

### 4.5 Resume — both entry points

#### 4.5.1 `resume_snapshot` (same code)

Replaces the stub at `execution.rs:190-203`. Orchestration only — every step exists:

1. **Verify**: envelope `SNAPSHOT_VERSION`; `CodeManifest` present; every `blobs[]` / `foreign_entries[]` hash resolvable in the store (or fetchable, §4.8); recompute each object's content hash on load (tamper check — blob hashes cover permissions); `required_permissions ⊆ granted permissions` else `SnapshotError::PermissionRefused{missing}` (§4.7.3); extension/native-library requirements satisfied (else `SnapshotError::CapabilityMissing{sub_kind, language_or_library, needed, found}` — A2(iv), §4.3.2 cross-doc note).
2. **Link**: assemble a `BytecodeProgram` from the manifest's blobs via the existing linker path (permission re-union cross-checked against the manifest's recorded union — mismatch = refuse). Transitional: while `bytecode_hash` twin exists (Stages 1–5), load the monolithic program instead.
3. **Restore**: `VirtualMachine::from_snapshot` (STAGE-R5 two-pass identity restore + `restore_call_stack`) — extended by Stage 0 to restore loop/timeframe/exception stacks and per-upvalue kinds (§4.2.4), and by Stage 2 to restore resource counters and deterministic state (§4.7).
4. **Marker**: push `Ok(Snapshot::Resumed)` (kinded, §4.1.3).
5. **Run**: normal dispatch loop from `VmSnapshot.ip`. A resumed VM is indistinguishable from a running one — it can snapshot again (chained snapshots), be interrupted again, etc.
6. **Project**: completion flows through the existing kinded `wire_conversion::slot_*` projection (`execution.rs:595-608`) into `ProgramExecutorResult` — the CLI already prints it (`script_cmd.rs:286-295`).

**Replay-only reference discipline (Constraint 9):** restored `PromotedCell` references re-link through the shared `restore_identity_map`; the resumed VM replays the same MIR, so no live-loan re-establishment is needed; `is_mut` is carried, never read. Any SV arm the restore cannot link → surface-and-stop, never a fabricated slot.

#### 4.5.2 `recompile_and_resume` (edited source)

Replaces the stub at `execution.rs:209-223`. Same as §4.5.1 with step 2 replaced:

2′. **Recompile & relocate**: compile the new source (CLI already parses/analyzes it, `script_cmd.rs:238`); build the new program's `function_hashes`; run `resolve_function_identity` (`executor/snapshot.rs:38-97`) per frame — **hash-first**: a frame whose `blob_hash` exists verbatim in the new program relocates its ip via `ip_blob_hash + ip_local_offset` (content-identical function ⇒ identical bytecode ⇒ offset valid).

**Mismatch semantics (defined, not improvised):**

| Situation | Behavior |
|---|---|
| Frame's function content-identical in new program | relocate, resume |
| Frame's function **changed** (hash absent) | **clean refuse** naming the function: `SnapshotError::ResumeFunctionChanged{name, old_hash}` — an ip cannot be soundly mapped into different bytecode. No heuristic line-mapping in v1 (Open Question 3). |
| Function changed but **not live on the call stack** | fine — module-level code and future calls use the new version. This is the useful edit-and-resume case: fix a not-yet-executing function, resume. |
| Module binding's *type* changed (schema mismatch vs new schema registry) | clean refuse `SnapshotError::ResumeSchemaMismatch{binding, old, new}` |
| Entry/top frame changed | refuse (special case of row 2) |
| Snapshot's `required_permissions` ⊄ new program's union | refuse (permission narrowing across an edit must be deliberate: re-grant explicitly) |

The transitional monolithic path cannot support any of this (whole-program hash changes on any edit) — recompile-and-resume is therefore gated on the CodeManifest landing (Stage 4 after Stage 2's manifest producer, §6).

#### 4.5.3 CLI surface (explicit grammar)

Two resume forms, distinguished by the presence of a source-file argument (matching the existing CLI plumbing at `script_cmd.rs:238`, which already parses the positional file):

- `shape --resume <hash>` — **plain resume** (§4.5.1): same code, loaded from the store; a source file on disk is ignored (the snapshot is the code authority).
- `shape --resume <hash> <file.shape>` — **recompile-and-resume** (§4.5.2): the file is compiled and the stricter mismatch-semantics table applies. Passing the file is the explicit opt-in to relocation semantics; there is no implicit "pick up the edited file automatically" mode. The `--resume` help text and the book chapter (§4.12) document both forms and the stricter refusal behavior of the second.

An explicit `--with-source` flag was considered and dropped: the positional form is already shipped plumbing, and the file argument is itself the explicit signal (no ambiguity — plain resume takes no file).

### 4.6 Suspension barriers & the opaque-arm ruling table

#### 4.6.1 Barriers — states that refuse `snapshot()` / interrupt-save cleanly

A barrier check runs before capture (§4.1 step 2). Each barrier yields `SnapshotError::Barrier{reason}`, surfaced to the program through `snapshot()`'s Result channel (§4.1.4) with a user-facing message naming the offending state (§4.11); the program handles the `Err` and continues; nothing is persisted. On the interrupt-save path the same barrier produces the no-save outcome (§4.4).

| Barrier | Detection | Rationale |
|---|---|---|
| **Foreign frame on the call stack** (Python/TS/C in-flight; incl. `snapshot()` reached via a callback from foreign code) | frame's function is foreign, or VM entered via `LanguageRuntimeVTable` re-entry | Foreign runtime state is opaque — **coordinated contract with the FFI design**: `docs/design/ffi-rebuild.md` must declare foreign frames *opaque-to-snapshot*; the barrier consumes that declaration. Never pickle extension state. |
| **Non-quiescent async** (spawned task not yet joined; pending host future; inside `async scope` with live children) | task registry non-empty / awaiting host I/O | Host futures (tokio/io) are not serializable; partial task capture = torn program state. v1 refuses; quiescent-queue capture is future work (Open Question 4). |
| **Live Channel** anywhere reachable | SV serialize hits Channel arm | user-ruled clean-refuse (Constraint 12) |
| **Live Iterator / Deque / FilterExpr** reachable | SV arms `snapshot.rs:2332-2340` | user-ruled clean-refuse |
| **No snapshot store installed** | `snapshot_store.is_none()` | embedded hosts opt out (§4.1 enumerates the opt-out set); `snapshot()` must not trap |
| **Held Mutex** (a lock guard is live anywhere) | Mutex heap object carries a `locked` flag set/cleared by the guard's acquire/drop; the SV Mutex arm refuses when `locked` is set at serialize time | snapshotting a held lock = torn critical section on resume; §4.6.2's defined-reset row only ever restores an *unheld* mutex because of this barrier |
| **Mid-drop** (`Drop` glue frames on stack) | drop-call frame marker | drop re-execution on resume would double-drop; refuse rather than special-case in v1 |
| **Residual JIT frame on the stack** (§4.9 rule 2) | tier map / activation records: any live activation not interpreter-materialized | JIT-resident frames have no serializable ip/register mapping; refuse (`Barrier{JitFrame{function}}`) until deopt-then-capture is verified available (§4.9, Open Question 2) |

Serialize-time refusal (SV arm errors) vs pre-capture barrier check: the pre-check covers structural states (frames, tasks, store); reachability-dependent states (Channel-in-a-binding, held Mutex) are caught by the SV arms during capture — both routes produce the same `SnapshotError::Barrier{reason}` shape, and capture failure is atomic (§4.1).

#### 4.6.2 Opaque-arm disposition table (user rulings 2026-05-29 — encoded, not re-litigated)

| HeapKind | Disposition | Restore behavior |
|---|---|---|
| Iterator | **clean-refuse** | capture fails with barrier error naming the value & binding |
| Deque | **clean-refuse** | same |
| FilterExpr | **clean-refuse** | same (pure-discriminator kind, §2.7.9 — no HeapValue arm to serialize) |
| Channel | **clean-refuse** | same |
| Mutex | **defined-reset** | serialized as its inner value + `poisoned=false`; restored **unlocked**. Snapshot while *held* is a torn-state hazard → holding a Mutex guard is a barrier (refuse), so defined-reset only ever restores an unheld mutex. |
| Lazy | **defined-reset** | forced value serialized if already forced; unforced Lazy restored unforced (thunk re-links by function hash via the manifest). Closure-in-Lazy whose captures hit a refuse-arm → refuse propagates. |
| PriorityQueue | **serialize** | full element round-trip via existing SV element arms |
| Reference / SharedCell / SharedCellRef | **identity-handle replay-only** | implemented (STAGE-R5, §2.7.30.5) — shared `restore_identity_map`, aliases dedupe to one restored referent |

New HeapKinds added later must extend this table in 4-table lockstep (Constraint 5) — `verify-merge.sh` extends to check the barrier/SV tables against the HeapKind roster.

#### 4.6.3 `std::core::state` primitive set

Declared surface: `crates/shape-runtime/stdlib-src/core/state.shape`. Per-primitive plan:

| Primitive | State at HEAD | Design |
|---|---|---|
| `hash`, `serialize`, `deserialize` | bodies real (`core.rs:440-455`), inputs poisoned by the variadic all-Bool shim (`marshal.rs:2284-2300`) | **Fixed by WF-1B** (`comptime-excellence.md` §4.2: `&[KindedSlot]` carriers whose kinds come from the caller's §2.7.7 stack kind track — the sole runtime kind source; the registered signature is only a stamped-vs-stamped cross-check, never a kind source — the "derive kinds from the registered signature" mechanism is the alternative the owning doc rejects in its §5.2). Acceptance: distinct inputs ⇒ distinct digests; round-trip identity. |
| `capture` → `FrameState` | reads real state (`vm_state_snapshot.rs:64-247`), return arm surfaces (`introspection.rs:77-130`) | Build the `FrameState` TypedObject KindedSlot-direct (Constraint 11): schema lookup, per-field kinded writes; `locals`/`args`/`upvalues` projected from the live stack slice (§4.2.3 view). No ConcreteReturn. |
| `capture_all` → `VmState` | same | same, over all frames + module bindings; `Frame.function` = `FunctionRef` built from `ctx.function_hashes` |
| `capture_module` → `ModuleState` | same | bindings + schema-name→schema-hash map (hashes from the CodeManifest's schema registry) |
| `capture_call` | unbuilt | `CallPayload` from a function value + kinded args; function identity via `fn_hash` |
| `resume(vm)` | decodes to **empty** VmSnapshot (`resume.rs:365-390`) | real decode: `VmState` TypedObject → frames (`Array<FrameState>` deep round-trip) + module-binding map → `VmSnapshot` → the existing `ResumeRequested` → `apply_pending_resume` path (`dispatch.rs:241-244`). Verification identical to §4.5.1 step 1 (a `VmState` from another node carries function hashes — they must resolve). |
| `resume_frame(f)` | **implemented** (`apply_pending_frame_resume`, `resume.rs:233-343`) | keep; add round-trip tests |
| `fn_hash` | mostly wired (`core.rs:457-520`) | complete FunctionRef/TraitObject decode arms |
| `schema_hash` | partial | serve from CodeManifest schema-registry hashes |
| `caller`, `args`, `locals` | partial | projections over the same `VmStateSnapshot` reader — kinded, per §4.2.3 |
| `diff`, `patch` | deleted module (`core.rs:368-385`) | **Deferred out of WF-2B** — rebuild of the 1486-LoC content-hash-tree differ is its own workstream and blocks nothing above (Open Question 6). Surface stays clean-refuse with an accurate unavailability message (WF-3B tone rules); the `state.shape` doc-comments for `diff`/`patch` are rewritten in the same stage to say "not available in this release" instead of promising the feature (§4.12.3). |

##### 4.6.3.1 The state-builtin return-ABI seam (Stage 5, Open Question 9)

At HEAD every state builtin returns `Result<TypedReturn, String>` with `TypedReturn::Concrete(ConcreteReturn::...)` payloads (`typed_module_exports.rs:196`, e.g. `state_hash` at `state_builtins/core.rs:444-452`) — while the *input* side is already §2.7.10-shaped (`args: &[KindedSlot]`). A KindedSlot-built `FrameState`/`VmState` TypedObject cannot cross that return signature without either a new `TypedReturn` arm or an ABI change. The design names its choice: **Stage 5 migrates the `state_builtins` module's return signature to `Result<KindedSlot, VMError>`** (the §2.7.10 result shape), scoped to the `state.*` builtin family; builtins outside `state.*` keep `TypedReturn` untouched (their migration is WF-1B/follow-up territory, not this design's). Adding a new `TypedReturn::Kinded(KindedSlot)` arm was rejected: it grows a parallel return discriminator that every consumer must then match — the ADR-005 §1 drift shape. This is an ABI change inside one module with a compile-time-enforced call shape; Open Question 9 puts it to the user because Constraint 11's earlier "bulldozed" citation was ungrounded (see §3).

##### 4.6.3.2 Corrected `state.shape` declared signatures (Stage 5 ships these, not the HEAD text)

Two declarations in `crates/shape-runtime/stdlib-src/core/state.shape` are broken as written and must not be baked in by implementing against them:

- `capture_call<F>(f: F, args: Vec<F>) -> CallPayload` (`state.shape:80`) types the args by the *function's* type parameter — heterogeneous args are inexpressible. Ships as `capture_call<F, Args>(f: F, args: Args) -> CallPayload` where `Args` is a tuple type (Shape has tuples; a 2-arg call passes `(a, b)`).
- `serialize<T>(value: T) -> Vec<int>` / `deserialize<T>(bytes: Vec<int>) -> T` (`state.shape:111-114`) use `Vec<int>`, which is not the language's collection surface. Ships as `Array<int>` (byte values 0–255) for v1; a dedicated `bytes` type is Open Question 11.

### 4.7 Determinism, limits, and permissions across resume

#### 4.7.1 Resource limits (`resource_limits.rs`)

- **Persisted counters** (snapshot row 8): `instruction_count`, output-bytes-emitted, memory high-water. On resume, counters restore ⇒ **limits are cumulative across the logical program execution**, not per-process — a sandboxed program cannot launder its instruction budget through snapshot/resume cycles.
- **Wall-clock (`TimeLimited`)**: elapsed wall time is *not* meaningfully resumable (the wall moved). The timer restarts at resume with the *remaining* budget = `limit − elapsed_at_capture` (elapsed persisted as a duration). **`elapsed_at_capture` is cumulative across all prior run segments**: a resumed VM initializes its elapsed accumulator from the restored duration, and a re-snapshot (chained snapshot/resume, §4.5.1 step 5) persists `restored_elapsed + this_segment_elapsed` — a chain of snapshots cannot launder the wall budget any more than the instruction counter can. If remaining ≤ 0, resume refuses with the limit error immediately — no free reset.
- Resume-side limits may be *narrowed* by the resuming host (its own `ResourceLimits` apply as a floor/ceiling min-merge); they may not be silently widened above what the snapshot's context recorded for sandboxed runs.

#### 4.7.2 `Time` / `Random` / `Deterministic`

- **Non-deterministic runs** (`Time`/`Random` granted, `Deterministic` off): `time::now()` returns real time after resume (it moved — that is the semantics of real time), RNG re-seeds from the OS. Nothing captured. Documented.
- **`Deterministic` sandbox**: the whole point is replay-stability, so snapshot row 9 captures the seeded RNG stream state and the virtual-clock offset; resume continues the *same* stream/clock. A deterministic program produces the identical output whether or not it was snapshot/resumed in the middle — this becomes an acceptance test (§7, T11).
- **`Capture` permission**: captured-output buffers persist with the output-bytes counter (they are part of `context`).

#### 4.7.3 Permission story

1. Snapshot records `required_permissions` (+ ScopeConstraints) in the `CodeManifest` — derived by the linker union (`content_addressed.rs:540`), and *independently verifiable* because each blob's content hash covers its permission names (`content_addressed.rs:122-142`).
2. Resume-time check (§4.5.1 step 1): recompute each blob's hash → recompute the union → compare to manifest → then `union ⊆ granted` on the resuming host. Any failure = `SnapshotError::PermissionRefused{missing}` / `SnapshotError::ManifestVerifyFailed{blob}` (tamper) **before** any bytecode executes. Zero trust in the snapshot's self-declaration.
3. A resuming host may grant a superset (fine) or a subset (refuse at load — never lazily at first violation, because the program may have already re-executed effects by then).
4. Runtime gating (`check_permission`, ~5ns) continues unchanged on the resumed VM — tier-2 defense stays.

### 4.8 Cross-node resume

Resume on a different node is §4.5.1 with fetch added at step 1. **Must-match set** (each mismatch has a named refusal):

| Requirement | Verified how | On mismatch |
|---|---|---|
| `SNAPSHOT_VERSION` | envelope | `SnapshotError::VersionMismatch{found, supported}` |
| All `CodeManifest.blobs[]` + `foreign_entries[]` present | local store, else fetch via the WF-2C blob transport (`build_minimal_blobs_by_hash` closure is exactly the blob fetch set; foreign entries fetched in their own object class per integration A3); recompute content hash on receipt | `SnapshotError::MissingBlob{hash}` / `MissingForeignEntry{hash}` after fetch failure |
| Schema registry + closure layouts | fetch by hash, recompute | refuse (`ManifestVerifyFailed`) |
| Extension set (polyglot programs) | `CodeManifest.extensions[]` — `ExtensionReq` semver/capability matching against the node's `NodeCapabilitySet` (contract owned by `polyglot-distributed-integration.md` §4.3; **no** `.so` hash and **no** cross-node ABI equality check — ABI is enforced by each node's own loader gate) | `SnapshotError::CapabilityMissing{sub_kind, language_or_library, needed, found}` |
| Permission grant ⊇ recomputed union | §4.7.3 | `SnapshotError::PermissionRefused{missing}` |
| Word size / endianness | SV/bincode wire encoding is explicit little-endian fixed-width (already true of wire protocol v1); no raw host pointers exist in SV by construction — all identity is handle-based (§2.7.30.5) | n/a — portable by format |
| Filesystem/host state referenced by ScopeConstraints | NOT verified — scoped paths may not exist on node B; first I/O fails with the normal runtime error | documented, not a load-time check |

Out of scope here (WF-2C/WF-2F territory): the transport itself, node discovery, and foreign-frame *transfer* (barriers already guarantee no snapshot contains a foreign frame, so cross-node resume never has to move extension state — only extension *availability* matters).

### 4.9 JIT interplay

Snapshot capture serializes interpreter frames. JIT-resident frames (Cranelift-compiled activations) have no serializable ip/register mapping. Rules:

1. **Suspension-point pinning is transitive over the static call graph**: at blob load, `contains_suspension_point` is stamped by the compiler for every function whose body contains a suspension-point call site (`snapshot()` / `state.resume` / `state.capture_all`), then **propagated to every function that statically reaches a stamped one** — the blob dependency closure (`build_minimal_blobs_by_hash`'s caller→callee edge walk, `remote.rs:454-489`) already computes exactly this reachability; the flag is its reverse closure. Pinned functions are excluded from T1/T2 promotion. This makes the direct- and static-transitive-call cases **warmth-independent**: `snapshot()` at the bottom of a statically visible call chain behaves identically at call 1 and call 10⁶. Direct-only pinning was rejected: every transitive caller would still tier up at 100 calls and flip the chain into rule 2's barrier — a nondeterministic, warmth-dependent failure cliff for the vertical's core primitive (flagged by adversarial review). Cost owned explicitly: a checkpointing program's statically-reachable-from-`main`-to-checkpoint spine never tiers up; leaf compute functions that don't reach a suspension point still JIT normally. Open Question 2 ratifies this trade.
2. **Residual JIT frames** — activations that transitive pinning cannot prevent, i.e. `snapshot()` reached only through an *indirect* call (function value, closure, trait object) that static analysis cannot attribute to the caller, or any other JIT-compiled activation live on the stack: v1 **refuses** — `SnapshotError::Barrier{JitFrame{function}}` (§4.6.1 row) — unless the existing deoptimization machinery can already materialize interpreter frames for all JIT activations on the stack at the suspension point. Whether full-stack deopt-on-demand is currently reachable is **not verified by recon** (Open Question 2); the design does not assume it. If/when it is available, the barrier is replaced by deopt-then-capture with zero format change.
3. Resume always starts in the interpreter; normal tiering re-warms afterward (feedback vectors are not persisted — cold-start after resume is accepted; persisting feedback is a non-goal).
4. `KindedSlot`/capture machinery stays off the VM↔JIT slot ABI per Constraint 8 — the barrier/flag approach requires no JIT ABI change at all.

Acceptance tests run the full matrix in both `--mode vm` and `--mode jit` (§7). "Either identical behavior or the clean barrier" is **not** an acceptable blanket test predicate — it would let a jit-mode suite pass without ever executing a resume (adversarial-review finding). §7 therefore requires: (a) at least one resume-exercising probe whose entire call chain is structurally rule-1 (statically reaches `snapshot()` ⇒ pinned ⇒ resume really runs under `--mode jit`), and (b) a dedicated probe (T15) that deterministically produces the rule-2 barrier via function-value indirection under a forced-hot caller.

### 4.10 Cross-design consistency ledger

All four WF-D designs go to ratification together; this ledger pins every seam this doc shares with a sibling, so exactly one owner exists per decision and the user never ratifies two conflicting shapes.

| Seam | Owner | This doc consumes / provides |
|---|---|---|
| `ExtensionReq` shape + matching semantics (semver, capability set, default policy) | `polyglot-distributed-integration.md` §4.3 (amendment A2, its §4.10) | consumed verbatim in §4.3.2 — including **no** `abi_version` (ABI enforced host-locally by the loader gate, never matched cross-node) and no `.so` content hash |
| `CodeManifest.foreign_entries` + `dormant_callback_registrations` fields; `MissingForeignEntry{hash}` sibling refusal | integration A2/A3 | consumed (§4.3.2, §4.8); entry objects written at §4.3.5 step 2 |
| `SnapshotExtensionMissing` → `SnapshotError::CapabilityMissing{sub_kind, …}` generalization | integration A2(iv) (`CapabilitySubKind` + message templates, its §4.3.2) | consumed (§4.5.1, §4.8, §4.11 catalog) |
| `foreign_reentry_depth` counter (incremented around vtable/libffi calls + callback re-entry) | `ffi-rebuild.md` §4.9 shared core; instrumented per integration §4.5 | consumed by §4.4's interrupt-observation gate. Note: the integration doc (its 2026-07-05 second revision, §4.5 + §4.9 Ctrl+C row) now quotes §4.4's rule verbatim — foreign re-entry *defers observation*, every other barrier is *terminate-now with no-save*; its earlier "capture happens at the first barrier-free check" phrasing is retired. The two docs state one rule. |
| Foreign frames opaque-to-snapshot declaration | `ffi-rebuild.md` (contract named in §4.6.1 row 1) | consumed — the barrier fires on that declaration, never on pickling |
| Cooperative cancellation of long foreign calls | `ffi-rebuild.md` §4.12 + its Open Question 11 (v1 decline, `request_cancel` vtable tail as designed follow-up) | consumed by §4.4 / Open Question 7 (kept there only for consolidated ratification — the sibling's answer governs both) |
| Blob transport, `MissingModuleFunction` retry classification | `distributed-function-transfer.md` (WF-2C) + integration A3 | consumed by §4.8's fetch column; this doc owns format + verification, never transport |

### 4.11 Refusal namespace & rendered-message catalog

**One enum.** Every refusal this design produces — snapshot-time, interrupt-time, resume-time — is a variant of the single Rust-side `SnapshotError` (Constraint 13; introduced §4.1.4):

```
SnapshotError {
    Barrier(BarrierReason),                    // snapshot-time refusals, §4.6.1
    PersistFailed{detail},                     // store I/O; atomic — nothing written (§4.3.5)
    VersionMismatch{found, supported},
    MissingBlob{hash} | MissingForeignEntry{hash},
    ManifestVerifyFailed{object},              // recomputed content hash ≠ recorded (tamper / corruption)
    ResumeFunctionChanged{name, old_hash},     // recompile-and-resume, §4.5.2
    ResumeSchemaMismatch{binding, old, new},
    PermissionRefused{missing},
    CapabilityMissing{sub_kind, language_or_library, needed, found},   // A2(iv)
}
BarrierReason { NoStore | NoPersistentContext | ForeignFrame{language, function}
              | AsyncPending{live_tasks} | LiveChannel{binding} | LiveIterator{binding}
              | LiveDeque{binding} | LiveFilterExpr{binding} | HeldMutex{binding}
              | MidDrop{type_name} | JitFrame{function} }
```

**Rendering rule (Goal 6 made mechanical).** Two layers, never mixed:

1. **Internal sentinels stay internal.** `NotImplemented("SURFACE: …")` strings, ADR citations, `NativeKind` names, raw slot bits, full 32-byte hashes — these live in `VMError`/logs/test assertions only. No internal sentinel string is ever the rendered message (the `Suspended on future 18446…` leak is the class this rule kills).
2. **Every rendered message = plain-language what-happened + the user-visible name of the offending thing (binding/function/value name, never an internal id) + one remediation sentence.** Hashes render truncated (first 12 hex) and only where the user can act on them (a resume command, a store operation).

The renderer is one function beside the enum; CLI and REPL consume it; acceptance probes (§7) assert against **catalog text**, so message drift fails CI. The same catalog table is the source for the book's "what cannot be checkpointed" page (§4.12.1) — one table, two consumers, no drift.

**Catalog (exact rendered text; `{x}` = interpolation).** Remediation is the second sentence of each.

| Variant | Rendered message |
|---|---|
| `Barrier{NoStore}` | `snapshot() is not available here: no snapshot store is configured. 'shape run' and 'shape repl' configure one automatically; embedded hosts call set_snapshot_store(...) before executing.` |
| `Barrier{NoPersistentContext}` | `snapshot() is not available here: this host runs without a persistent execution context. Use the engine API with a persistent context to enable checkpoints.` |
| `Barrier{ForeignFrame}` | `cannot checkpoint while a {language} call is in progress (inside '{function}'). Let the call return, or move the snapshot() call outside it.` |
| `Barrier{AsyncPending}` | `cannot checkpoint while {live_tasks} async task(s) are still running. Join or cancel them (e.g. close the async scope) before checkpointing.` |
| `Barrier{LiveChannel}` | `cannot checkpoint while channel '{binding}' is alive: channels cannot be saved. Drop or close it before checkpointing.` |
| `Barrier{LiveIterator}` / `{LiveDeque}` / `{LiveFilterExpr}` | `cannot checkpoint while {an iterator / a deque / a query filter} ('{binding}') is alive: this value cannot be saved. Drop it or finish using it before checkpointing.` |
| `Barrier{HeldMutex}` | `cannot checkpoint while mutex '{binding}' is locked. Release the lock before checkpointing.` |
| `Barrier{MidDrop}` | `cannot checkpoint while a value of type '{type_name}' is being dropped. This is a narrow timing window - retry the checkpoint.` |
| `Barrier{JitFrame}` | `cannot checkpoint here: '{function}' is running as optimized native code. Call snapshot() directly from your code rather than through a stored function value, or run with --mode vm.` |
| `PersistFailed` | `checkpoint could not be written: {detail}. Nothing was saved; the program continues. Check the snapshot store location and free space.` |
| `VersionMismatch` | `this snapshot uses format {found}; this build of shape reads format {supported}. It cannot be resumed - re-run the program to create a new snapshot.` |
| `MissingBlob` / `MissingForeignEntry` | `cannot resume: the snapshot references code ({short_hash}...) that is not in the store and could not be fetched. Copy the complete snapshot store, or resume on the node that created it.` |
| `ManifestVerifyFailed` | `cannot resume: stored code ({short_hash}...) does not match its recorded hash - the store is corrupt or was tampered with. Do not trust this snapshot; recreate it from source.` |
| `ResumeFunctionChanged` | `cannot resume with this source: function '{name}' changed since the snapshot and is currently executing. Revert '{name}', or resume without a source file to run the original code.` |
| `ResumeSchemaMismatch` | `cannot resume with this source: the type of '{binding}' changed since the snapshot. Revert the type change, or resume without a source file.` |
| `PermissionRefused` | `cannot resume: this snapshot needs permission(s) not granted here: {missing}. Re-run with those permissions granted.` |
| `CapabilityMissing` | message templates owned by the integration design (§4.3.2), rendered verbatim — e.g. `MissingRuntime`: `this snapshot needs the {language} extension; none is installed. Install it with: shape ext install {language}.` |

**Composites.** Interrupt-with-save prints `Snapshot saved: {short_hash}` + `Resume with: shape --resume {short_hash}` (exit 130, §4.4). Interrupt-no-save prints `Interrupted - no snapshot saved: {rendered barrier message}` (exit 130). The unreachable-by-construction host-boundary `Suspended{SNAPSHOT_FUTURE_ID}` arm (§4.4) renders `internal error: snapshot() reached the host boundary - this is a shape bug, please report it` — a named bug message, still no sentinel leak.

Each variant's catalog row lands in the same stage as its producer (§6); the catalog table above is normative — implementation copies it, not the reverse.

### 4.12 User surface & documentation (book-gate plan)

The standing hard gate applies: every feature this design ships must be in the book with a gate-runnable example that executes green under vm+jit, and the full book truth-gate re-runs at each stage close (§6 gates reference this section).

#### 4.12.1 Book chapters + gate-runnable examples

| Ships in | Book surface | Gate-runnable example |
|---|---|---|
| Stage 1–2 | **New chapter "Checkpoint & Resume"**: the `Result<Snapshot, SnapshotError>` contract, `match` on `Hash`/`Resumed`/`Err`, both `--resume` CLI forms (§4.5.3), Ctrl+C save + exit 130 | T1-shaped program (compute → checkpoint → continue) asserting the `Hash` arm; resume flow shown in the chapter, executed e2e by T2's CLI probe (two-process flows run in the acceptance tier; the in-book example itself stays single-process gate-runnable) |
| Stage 1 | **"What cannot be checkpointed"** section: the §4.6.1 barrier table rendered in user language via the §4.11 catalog — the book is the *only* discoverability surface for barriers (a runtime `Err` is discovery-after-the-fact); each barrier: what, why, what to do | example matching on `Err(SnapshotError::Barrier(...))` and continuing without a checkpoint (the recommended long-computation pattern) |
| Stage 3 | Ctrl+C interrupt-save subsection + exit-code contract (0 / 130) | scripted-SIGINT harness probe (T4) — acceptance tier; chapter documents, gate example asserts the flag-check builtin behavior single-process |
| Stage 4 | Recompile-and-resume subsection: the mismatch-semantics table (§4.5.2) in user terms | T7-shaped fixture pair |
| Stage 5 | **`std::core::state` reference chapter**: corrected signatures (§4.6.3.2), capture/capture_all/resume_frame examples; `diff`/`patch` explicitly marked "not available in this release" | capture → inspect FrameState example; serialize/deserialize round-trip example |
| Stage 6 | Distributed chapter cross-link: cross-node must-match set (§4.8) in user terms | owned jointly with WF-2C/WF-2F book work |

#### 4.12.2 Snapshot discovery & management tooling

The only handle `snapshot()`/Ctrl+C ever prints is a hash; if the terminal scrolls away there must be a recovery path (adversarial-review gap). Surface:

- `shape snapshot list` — short-hash, `created_at_ms`, `script_path`, `label` (all already/now in the envelope: §4.3.1 reserves `label` at the Stage-2 bump precisely so list/inspect need no later format change).
- `shape snapshot inspect <hash>` — envelope + manifest summary: entry function, blob count, permissions union, extension reqs, creation time, source path.
- `shape snapshot rm <hash>` / `shape snapshot gc` — delete an envelope / sweep unreferenced content-addressed objects (§4.3.5's aborted-persist garbage).

All four are thin store readers with zero VM surface. Recommended scheduling: `list`/`inspect` land with Stage 3 (they make T4's flows debuggable and are the Ctrl+C recovery path); `rm`/`gc` at Stage 6. Open Question 10 ratifies scope (v0.3.3 vs fast-follow).

#### 4.12.3 Doc-comment reconciliation (merges with the code it describes)

- `crates/shape-runtime/stdlib-src/core/snapshot.shape` doc-comment → the `Result` contract + one-paragraph barrier summary linking the book chapter (Stage 1, same merge as the intrinsic change).
- `crates/shape-runtime/stdlib-src/core/state.shape` — `diff`/`patch` doc-comments rewritten to "not available in this release" (Stage 5, per §4.6.3); the two broken declarations replaced per §4.6.3.2.
- `--resume` CLI help text → both forms + stricter recompile refusal semantics (Stage 4, per §4.5.3).

---

## 5. Alternatives considered & rejected

1. **"ValueWord serialization helper" for the marker/stack round-trip** (the shape the stub comments imply). REJECTED — forbidden by name (CLAUDE.md: "*Do not reintroduce as a … 'serialization helper'*"). Moot anyway: §2.7.7 parallel tracks are implemented and tested at HEAD.
2. **Bool-default kinds for no-layout upvalues** (status quo at `executor/snapshot.rs:483`). REJECTED — ADR-006 §2.7.8/Q10 violation, live today, retired by Stage 0. Replacement is capture-side kind recording + surface-and-stop.
3. **Host-boundary-only consumer for `snapshot()`** (persist-and-exit; resume later gets `Resumed`). REJECTED — silently changes the shipped stdlib contract (`snapshot.shape:16-17`: the snapshotting run continues with `Hash(id)`). Exit-on-snapshot also makes `snapshot()` useless for checkpointing inside long computations, the primary use case.
4. **Re-entering `vm.execute` from `execution.rs` to continue after a boundary-consumed snapshot.** REJECTED — duplicates the dispatch loop's re-entry logic (a second spine); the dispatch shell already owns continuation (`ResumeRequested` precedent).
5. **A "resume shim" that replays from ip=0** instead of restoring frames (re-execute to the snapshot point). REJECTED — re-runs side effects; also the exact "replay recovers locals" hand-wave the recon flags as unverified. Locals are restored bit-identically via the stack instead (§4.2.3).
6. **A second identity map for references, separate from the SharedCell map.** REJECTED — §2.7.30.5 mandates the *shared* `restore_identity_map`; parallel identity machinery is the parallel-discriminator defection-attractor (ADR-005 §1).
7. **Serializing raw `Vec<u64>` stack without the kind track, kind-annotating lazily at restore.** REJECTED — that *is* runtime tag synthesis (`synthesize_value_word_from_raw` by deletion-fate); §2.7.7 requires the parallel kind track at capture time, which is also what exists.
8. **Pickling foreign runtime state** (Python `__reduce__`, V8 heap snapshot) to allow mid-foreign-call capture. REJECTED — unbounded opacity, extension-version-fragile, security hole (deserializing foreign heap = code execution). Foreign frames are barriers; the FFI design declares them opaque.
9. **Monolithic program hash as the permanent format** (status quo shape, just wired up). REJECTED as the end state (kept as a transitional twin) — no dedup, no per-function fetch, no permission re-check granularity, and recompile-and-resume is impossible under it (any edit changes the one hash). It would strand WF-2C/WF-2F.
10. **`Vec<KindedSlot>` as the VmSnapshot stack carrier.** REJECTED — explicitly forbidden shape (§2.7.7: no 16-byte slots, no `Vec<KindedSlot>` for the stack). Parallel vecs stay.
11. **Heuristic ip re-mapping into edited functions** (line-table-based relocation for changed functions in recompile-and-resume). REJECTED for v1 — unsound in general (changed bytecode ⇒ changed stack discipline at the target ip); hash-identical relocation only, changed-live-frame = clean refuse (Open Question 3 offers the v2 direction).
12. **Persisting JIT state / feedback vectors** so a resumed VM is instantly hot. REJECTED — format coupling to Cranelift internals for a warm-up win; resume starts interpreted and re-tiers.
13. **Snapshot-store writes via the existing `*const SnapshotStore` aliasing pattern** (`dispatch.rs:210`). REJECTED — pre-existing unsafety wart; the `Arc<SnapshotStore>` handle (§4.1) retires it for both the new consumer and the time-travel recorder.

Considered-but-rejected compromises that brushed forbidden territory (items 1, 2, 7, 10) are additionally logged in `docs/defections.md` per CLAUDE.md when implementation starts.

### 5.R Review-finding rebuttals (2026-07-05 adversarial pass)

Findings from the three-lens review are resolved inline throughout §4/§6/§7/§8. Two points where a finding was (partially) wrong are recorded here instead of changing the design:

- **"The manifest extension row must adopt semver/ABI matching."** Half right, and the half matters: amendment A2 does semver/**capability** matching but deliberately has **no** ABI matching — `ExtensionReq` carries no `abi_version` (integration doc §4.3.1: ABI is a node property enforced fail-safe by each host's own loader gate; a cross-node ABI equality check would forbid valid resumes — e.g. onto a node with a newer Shape ABI and matching extensions — for zero safety gain). This doc adopts A2 including that omission (§4.3.2, §4.8); the extension row names capability matching, not ABI matching. Restoring `abi_version` to the manifest would re-open the cross-platform-resume defect A2 exists to close.
- **"The envelope constant is at `snapshot.rs:25`."** It is `SNAPSHOT_VERSION` at `crates/shape-runtime/src/snapshot.rs:37` (verified at HEAD). The finding's substantive point — the constant is named `SNAPSHOT_VERSION`, not `FORMAT_VERSION` — was correct and is applied everywhere (§4.3.1/§4.3.3/§4.8/§6).

---

## 6. Implementation plan sketch (ordered, mergeable stages → WF-2B phases)

Each stage merges independently through `verify-merge.sh` + `just check-clean` + `just test`; blast-radius diff per the regression-scope rule. Branch: `rebuild/snapshot-resume`.

| Stage | WF-2B phase | Content | Close gate |
|---|---|---|---|
| **0. Capture-completeness** | (pre-`Catch` hardening) | Fix Bool-default at `executor/snapshot.rs:483` → per-upvalue kind recording (`upvalue_kinds` wire field + **`SNAPSHOT_VERSION` bump #1**, §4.3.3) + surface-and-stop; populate `local_ip`/`ip_blob_hash`/`ip_local_offset` at capture (§4.2.2); restore loop/timeframe/exception stacks (§4.2 row 6); extend `check-no-dynamic` grep (§4.2.4). **Plus the unverified-at-HEAD proof**: a unit/integration test that `from_snapshot` → run resumes mid-function at nonzero ip with live frames (recon gap x). | new no_dynamic sentinel green; mid-function round-trip test green |
| **1. Catch** | `Catch` | In-loop sentinel consumer (§4.1): `Arc<SnapshotStore>` handle (retire `*const` wart), barrier pre-check skeleton (store-missing + foreign-frame + async + mid-drop barriers), `persist_execution_state` (monolithic-twin envelope), kinded `Snapshot::Hash` marker, continue-after-snapshot. | `snapshot()` returns `Hash(id)` and the program continues; leaked `Suspended on future 18446…` string unreachable (grep + e2e) |
| **2. Persist/Resume** | `Persist/Resume` | `resume_snapshot` orchestration (§4.5.1) over the monolithic twin; `Snapshot::Resumed` marker; version checks; **CodeManifest producer** (§4.3.2) written alongside (dual-format) + new envelope fields incl. `label` (**`SNAPSHOT_VERSION` bump #2**, §4.3.3); counters + deterministic state persisted (§4.7.1/.2). | T1–T3 (§7) green same-node; manifest written & verified on load |
| **3. Interrupt** | `Interrupt` | `ShapeError::Interrupted` producer at `execution.rs` (§4.4); atomic write ordering (§4.3.5). | T4 green; kill-during-persist leaves loadable-or-absent store, never corrupt |
| **4. Recompile** | `Persist/Resume` (tail) | `recompile_and_resume` via `resolve_function_identity` over the CodeManifest; full mismatch-semantics table (§4.5.2). | T7–T8 green |
| **5. StatePrims** | `StatePrims` | *(after WF-1B merges — marshal per-position kinds)* state-builtin return-ABI migration (§4.6.3.1); `capture`/`capture_all`/`capture_module` return-arm projection KindedSlot-direct; whole-VM `state.resume` real decode; `fn_hash`/`schema_hash`/`caller` completion; opaque-arm ruling table enforced + Mutex/Lazy inner-payload arms + PriorityQueue serialize arm (**`SNAPSHOT_VERSION` bump #3**, §4.3.3); `diff`/`patch` clean-unavailable message + doc-comment rewrite (§4.12.3); corrected `state.shape` signatures (§4.6.3.2). | T9(b–e)–T10 green; `state::hash` distinct-digest probe green |
| **6. Cross-node manifest close** | `RoundTrip` (+ WF-2C + FFI interlock) | Stop writing the monolithic twin (the `Option` field stays at the wire tier; no version bump — the Stage-5 version is the one that stabilizes, §4.3.3); blob-granular load path; cross-node verification set (§4.8) minus transport (transport = WF-2C; combined e2e = WF-2F). JIT transitive `contains_suspension_point` pinning + JIT-frame barrier (§4.9). T9(a) foreign-frame e2e probe runs here (needs the FFI rebuild's Python fixture — see dependency notes). | T5–T6, T9(a), T11–T15 green vm+jit; WF-2F unblocked |

Dependency notes: Stage 5 depends on WF-1B (fix-plan wave graph already encodes "state::hash correctness arrives from WF-1B"). **T9(a) (snapshot from a Python-called Shape callback) additionally depends on the FFI rebuild's runnable Python fixture — polyglot/FFI is a dead stub at HEAD per the 2026-07-04 audit, so gating Stage 5's close on it would deadlock; the probe is therefore gated at Stage 6 (WF-2F interlock), while the barrier logic itself lands in Stage 1 and is unit-tested there against a synthetic foreign-frame marker.** Stage 6's transport half belongs to WF-2C — this design owns the format and verification, WF-2C owns moving bytes, WF-2F owns the polyglot × distributed combined matrix. The `RoundTrip` fan-out phase of WF-2B (N programs × snapshot→kill→resume→assert) runs after Stage 3 and grows with each later stage. **Book gate:** every stage's close additionally includes the §4.12.1 book additions for what that stage ships, verified by the full book truth-gate re-run (standing hard gate).

---

## 7. Acceptance tests (e2e probes, each run under `--mode vm` AND `--mode jit`)

JIT-mode expectation per §4.9: identical behavior when the whole chain is transitively pinned (rule 1), or the documented clean `SnapshotError::Barrier{JitFrame}` (rule 2) — asserted explicitly per probe, never as an either/or blanket predicate (a suite must not be able to pass `--mode jit` without executing a resume). T1–T3, T5–T8, T11–T14 are structurally rule-1 (their programs reach `snapshot()` only through static calls, so transitive pinning applies and resume really executes under `--mode jit`); T15 is the dedicated rule-2 probe. All probes go through the real CLI (`shape run` / `shape --resume`), not test-harness shortcuts, and assert user-facing text against the §4.11 catalog, never internal sentinel strings.

- **T1 — basic contract**: program computes, calls `snapshot()`, matches `Hash(id)` (prints id, keeps computing to completion). Assert: exit 0; the old leaked sentinel string absent; store contains a loadable envelope.
- **T2 — resume continuation**: run T1's program, capture the printed hash, `shape --resume <hash>`. Assert: resumed run takes the `Resumed` match arm, produces exactly the remaining output (byte-diff against the tail of an uninterrupted run), exit 0.
- **T3 — mid-loop identity**: loop 1..1000 printing i, snapshot at i==500, kill after snapshot, resume. Assert resumed output is exactly 501..1000 — proves frames, loop stack (§4.2 row 6), and locals-via-stack (§4.2.3) restore.
- **T4 — Ctrl+C save**: long-running loop; send SIGINT once; assert "Snapshot saved: <hash>" + resume command printed, **exit 130** (§4.4 — 0 is reserved for completed runs); `--resume` completes the computation. **No-skip/no-duplicate probe**: the loop body appends to a file with a strictly-increasing counter; the resumed output concatenated with the pre-interrupt output must be gap-free and duplicate-free (catches any off-by-one at the interrupt ip, §4.4). Barrier variant: run with a module-scope Channel alive → SIGINT terminates immediately with the no-save message naming the channel, exit 130. Second-SIGINT force-exit still works. Kill -9 *during* persist in a separate probe: store is loadable-or-absent, never corrupt (§4.3.5).
- **T5 — reference identity (§2.7.30)**: module-scope `let r = &x` (promoted cell) + a SharedCell alias of the same referent; snapshot; resume; mutate through one route, read through the other. Assert aliases dedup'd to ONE restored referent (identity-map), values coherent.
- **T6 — carrier matrix round-trip**: one program per carrier — closure with captures (layout path), TypedObject graph, `Array<number>` TypedArray big enough to force chunked BlobRef sidecars, HashMap, enum values, string/decimal — snapshot mid-use, resume, assert deep equality of subsequent output.
- **T7 — recompile-and-resume, compatible edit**: snapshot inside `f`; edit a *different, not-live* function `g`; `shape --resume <hash> file.shape`. Assert: resumes, and post-resume calls to `g` observe the NEW behavior.
- **T8 — recompile-and-resume, refused edit**: same but edit `f` (live on the stack). Assert clean `ResumeFunctionChanged` naming `f` + short old hash (§4.11 text); exit nonzero; no partial execution. Also: version-mismatch envelope (hand-bumped) → `VersionMismatch{found, supported}`; missing blob → `MissingBlob{hash}`.
- **T9 — barrier matrix**: (a) `snapshot()` invoked from a Python-called Shape callback → clean foreign-frame barrier (coordinated FFI fixture — **runs at Stage 6**, see §6 dependency notes; the barrier's unit test against a synthetic foreign frame runs from Stage 1); (b) snapshot with a spawned un-joined async task → async barrier; (c) live Channel / Iterator / Deque reachable → clean-refuse naming the value; (d) Mutex + Lazy round-trip per defined-reset (Mutex restored unlocked; unforced Lazy still lazy, forced Lazy keeps value) + held-Mutex barrier probe (guard live at `snapshot()` → `Barrier{HeldMutex}` naming the binding); (e) PriorityQueue full round-trip. All refusal texts asserted against the §4.11 catalog.
- **T10 — `std::core::state`**: `state::hash` distinct digests for distinct inputs + stable digest across runs (WF-1B probe re-run here); `capture()` returns a real `FrameState` whose `locals`/`args` match known values; `capture_all()` → `serialize` → `deserialize` → `resume` on a fresh process reproduces the remaining output; `resume_frame` re-enters and returns the frame's result.
- **T11 — determinism**: `Deterministic` sandbox program using `random()` + virtual clock, output A = uninterrupted run, output B = snapshot-mid-way + resume. Assert A == B byte-identical (§4.7.2).
- **T12 — limits carry-over**: instruction-limited sandbox that snapshots at ~60% budget; resumed run must hit the limit at the same *cumulative* count (no budget laundering); wall-time probe: resume with expired remaining budget refuses immediately.
- **T13 — permissions**: snapshot a program requiring `FsRead`; resume under a grant *without* `FsRead` → `PermissionRefused{missing: FsRead}` before any execution (assert no output side effect); resume with the grant → completes. Tampered-manifest probe: flip one blob byte in the store → `ManifestVerifyFailed` hash-recompute refusal.
- **T14 — cross-node simulation**: snapshot in store A; copy *only* the envelope + manifest-referenced objects (enumerated via the manifest — proves the closure is complete) to fresh store B in a clean workdir; resume from B. Assert identical completion. (Real transport = WF-2C's gate; polyglot-bearing variant = WF-2F's matrix.)
- **T15 — JIT rule-2 barrier determinism (`--mode jit` only)**: `snapshot()` reachable *only* through a function-value indirection (`let f = checkpoint_fn; f()`) under a caller forced hot (>100 calls before the checkpoint call) → deterministic clean `Barrier{JitFrame}` naming the JIT-compiled function (§4.11 text), program continues via the `Err` arm to completion, exit 0. Same program under `--mode vm` checkpoints successfully — the divergence is exactly the documented barrier, nothing else.
- **Regression floor**: existing snapshot unit suite (`executor/snapshot.rs:981-1370`), STAGE-R5 identity tests, and `apply_pending_frame_resume` tests stay green at every stage; `just check-no-dynamic` + sentinel `no_dynamic.rs` green including the new Stage-0 pattern.

---

## 8. Open questions for the user

**Ratification record (2026-07-05):** all recommended defaults below were ratified by the user (consolidated as `00-priority-spine-overview.md` §3, Q16–Q25 + Q3). No override touches this doc. In particular: OQ8's `snapshot() -> Result<Snapshot, SnapshotError>` contract change (Q22) and OQ9's scoped state-builtin return-ABI migration (Q23) are adopted. OQ7 was answered at its consolidation home — ffi-rebuild OQ11 ≡ overview Q3: the v1 cooperative-cancellation decline is ratified, and that one answer governs both docs (§4.10 ledger).

1. **CodeManifest ratification (§4.3).** Recommendation: blob-graph persistence (per-FunctionBlob content-addressed objects + manifest), with the monolithic program hash kept only as a transitional twin through Stage 5 and dropped in Stage 6. This is load-bearing for recompile-and-resume, cross-node resume, and WF-2C/WF-2F composition. Ratify blob-graph as the end state? (The alternative — monolithic forever — forecloses §4.5.2 and cheap cross-node fetch; rejected in §5.9 but it is a format commitment you should own.)
2. **JIT frames at snapshot time (§4.9).** v1 rule: `contains_suspension_point` is pinned **transitively** over the static call graph (every function that statically reaches `snapshot()`/`state.resume`/`state.capture_all` never tiers up) — direct-only pinning was rejected because it made `snapshot()` a warmth-dependent cliff (works cold, refuses once any transitive caller crosses the 100-call T1 threshold). Two things to ratify: (a) **the never-tier cost** — a checkpointing program's statically-reachable spine from `main` to the checkpoint stays interpreted forever (leaf compute functions that don't reach a suspension point still JIT normally); (b) **the residual barrier** — `snapshot()` reached only through an *indirect* call (function value / closure / trait object), which static pinning cannot attribute, refuses with `Barrier{JitFrame}` under a hot caller (T15 pins this behavior). Full-stack deopt-then-capture would remove both; recon did not verify the deopt machinery can materialize interpreter frames on demand at arbitrary safepoints. Accept transitive-pinning + residual barrier for v1 with deopt-capture as a follow-up investigation, or make deopt-capture a blocking requirement of this vertical?
3. **Recompile-and-resume for *changed live* functions (§4.5.2).** v1 refuses (hash-identical relocation only). A v2 could attempt debug-info-based ip mapping for "safe" edits. OK to ship refusal as the durable semantic (my recommendation — it is honest and predictable), or is edit-the-currently-executing-function a required capability?
4. **Async quiescence (§4.6.1).** v1: any non-quiescent async state is a barrier. Should a follow-up capture *not-yet-started* queued tasks (pure-VM thunks, expressible as CallPayloads) while still refusing in-flight host futures — or is snapshot-at-quiescence-only acceptable long-term? (Interacts with WF-2D's async semantics decision D1.)
5. **Snapshot format migration (§4.3.3).** v1 refuses older `SNAPSHOT_VERSION` snapshots with a clean error. Given snapshots may become long-lived artifacts (distributed checkpoints), do you want a migration commitment (N-1 version read support) starting now, or is refuse-and-recompute acceptable until the format stabilizes post-v0.4?
6. **`state::diff`/`state::patch` (§4.6.3).** The 1486-LoC content-hash-tree differ was deleted; nothing in pause/resume/distributed-transfer depends on it. Recommendation: keep clean-unavailable in WF-2B and schedule the rebuild as its own lane (possibly v0.4 with the delta-sync story). Confirm deferral, or pull into v0.3.3 scope?
7. **Interrupt-save inside foreign calls (§4.4).** First Ctrl+C during a long foreign (Python/TS/C) call cannot capture until control returns to the VM; if it never returns, second Ctrl+C force-exits with nothing saved. Acceptable documented behavior, or should the FFI design be asked for a cooperative-cancellation hook so foreign calls can be unwound to a barrier-free point first? *Sibling's answer already on record:* `ffi-rebuild.md` §4.12 declines the hook for v1 (documented behavior; second-Ctrl+C escape hatch) and designs an additive `request_cancel` vtable tail fn as the follow-up at zero ABI cost; its Open Question 11 puts that decline to you. This entry is retained only so ratification sees one consolidated decision — **answer it there; that answer governs both docs** (§4.10 ledger).
8. **`snapshot()` stdlib contract change to `Result` (§4.1.4).** The shipped signature `pub fn snapshot() -> Snapshot` has no error channel, and Shape has no catch — under it every barrier refusal would be an uncatchable runtime abort (an hour-3 computation dies because a Channel was reachable at the checkpoint). The design changes the contract to `Result<Snapshot, SnapshotError>` (`Snapshot` itself unchanged: `Hash(string) | Resumed`). No working program breaks — the feature has never worked (2026-07-04 audit). Ratify the contract change? (The alternative — keep the signature, barrier = program termination — is rejected in the design as contradicting Goal 6, but it is your call to make.)
9. **State-builtin return-ABI migration (§4.6.3.1).** The state builtins return `Result<TypedReturn, String>` with `ConcreteReturn` payloads at HEAD; a KindedSlot-built `FrameState`/`VmState` cannot cross that seam. Stage 5 migrates the `state.*` builtin family's return signature to `Result<KindedSlot, VMError>` (the §2.7.10 result shape), scoped to that one module — builtins outside `state.*` keep `TypedReturn` (their migration is WF-1B/follow-up territory). A new `TypedReturn::Kinded(KindedSlot)` arm was rejected as parallel-return-discriminator drift (ADR-005 §1 shape). Ratify the scoped migration and its scope boundary? (Raised because an earlier draft's "ConcreteReturn is bulldozed" constraint was ungrounded — see §3 item 11; this is a design decision needing your ownership, not a quoted ruling.)
10. **Snapshot discovery/management tooling scope (§4.12.2).** The only handle a user gets today is a hash printed once; if it scrolls away there is no recovery path. Recommendation: `shape snapshot list` + `inspect` land with Stage 3 (they are the Ctrl+C recovery path), `rm` + `gc` at Stage 6; the `label` envelope field is reserved at the Stage-2 bump either way so nothing needs a later format change. Ratify this scope for v0.3.3, or move some/all of the subcommands to a post-v0.3.3 fast-follow?
11. **Byte-payload surface for `state::serialize` (§4.6.3.2).** The declared `Vec<int>` is off-language; v1 ships `Array<int>` (byte values 0–255) to match the shipped collection surface. A dedicated `bytes` (or `Array<int8>`) type would be the better long-term surface — 8x denser and honest about range — but is a type-system addition beyond this vertical's scope. Accept `Array<int>` for v1 with a `bytes` type as a named follow-up, or block `state::serialize` on the `bytes` type?
