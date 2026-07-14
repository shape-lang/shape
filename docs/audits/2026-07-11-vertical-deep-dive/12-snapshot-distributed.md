# Vertical Deep-Dive Audit 12: Snapshots, Wire Protocol & Distributed Execution

**Auditor**: 12 of 19 (ultra-deep-dive, 2026-07-11)
**Territory**: snapshot machinery (`crates/shape-runtime/src/snapshot.rs`, `crates/shape-vm/src/executor/snapshot.rs`, `crates/shape-vm/src/executor/vm_state_snapshot.rs`, snapshot v7 identity maps), `crates/shape-wire/` (MessagePack + QUIC/TCP transports, wire protocol), the runtime-side marshal/blob layers (`crates/shape-runtime/src/{wire_conversion,blob_wire_format,blob_prefetch}.rs`), `crates/shape-vm/src/bytecode/content_addressed.rs`, `crates/shape-vm/src/linker.rs`, `crates/shape-vm/src/remote.rs`, `bin/shape-cli/src/commands/{snapshot_cmd,serve_cmd,wire_serve_cmd}.rs`, and the CLI distributed E2E suite `bin/shape-cli/tests/distributed_*_e2e.rs`
**Tree state**: DIRTY working tree audited as-is (branch `main`, HEAD `ce332ca2`).

All runtime claims below were verified empirically against the prebuilt working-tree binary
`/home/dev/dev/shape-lang/shape/target/debug/shape`; transcripts are pasted inline (extension-load
warnings and engine-init banner lines are filtered out). Scratch programs live under the session
scratchpad `verticals/snapshot-distributed/`.

---

## 0. Executive summary

**Overall health verdict: STRONG — this user-priority vertical is genuinely working end-to-end,
with one fresh P1 restore bug, one designed-refusal feature the book oversells as working, and a
legacy server that should be deleted.**

This is the most-improved vertical relative to its own audit history. The 2026-07-04 audit
(memory: `project_audit_2026_07_04_claimed_vs_real`) called snapshot/resume "DEAD stubs"; that is
now **refuted with discriminating empirical evidence** on this working tree:

- Mid-loop SIGINT interrupt produces a snapshot that resumes at the exact loop position: first run
  prints `i=1000000, i=2000000` then saves on Ctrl+C; resume prints `i=3000000, i=4000000, done`
  — no recomputation (§2.1.3).
- `snapshot()` checkpoints resume with values AND control flow intact, including inside a function
  frame carrying an `int`, an `Array<int>`, and a user `type Point` — resume prints
  `resumed x=42 arr0=100 arr1=200 px=1.5 py=2.5` (§2.1.2).
- The full distributed matrix reproduces against loopback servers started in this session:
  `@remote` transfer of pure-Shape, `extern C`, and `fn python` functions to a live `shape serve`
  node; receiver-side mid-run `snapshot()`; local resume of the receiver's checkpoint
  (`RESUMED:43`) (§2.2). Server logs prove genuine transfer (`blobs=2 foreign_entries=1`), and a
  dead-port control fails with `Connection refused` — no local fallback.
- Receiver-side zero-trust enforcement is real: a `--sandbox strict` node refuses the transferred
  `extern C` call with `the server does not grant [ffi.call]`; blob content hashes are recomputed
  from received bytes before linking (`remote.rs:1064-1079`).

The main deductions:

1. A **new P1 restore bug**: any top-level slot holding a `StringV2`-kinded value (e.g.
   `let first = items[0]` from an `Array<string>`, or a for-over-array loop variable) snapshots
   successfully but **always fails to restore** with
   `SerializableVMValue arm String cannot satisfy expected kind StringV2` — a capture/restore
   asymmetry that hands the user a dead hash (§9.1).
2. **Recompile-and-resume is a designed clean refusal** in this build — even for byte-identical
   source (`execution.rs:215-239`) — while the book documents it as a working ordinal-remap
   feature with safe-edit guidance (§8.2).
3. **`wire-serve` is a security-and-correctness liability**: it shells out to `shape` on `$PATH`
   (breaks when absent — reproduced over a raw socket), executes with full user permissions, no
   auth, no sandbox, and its `validate` is parse-only (§9.4, §9.5).
4. **QUIC ships disabled**: client-only code behind a non-default cargo feature, no QUIC server
   anywhere in the repo, so the "QUIC transport" headline in CLAUDE.md describes latent code, not
   the shipped binary (§5.4, §8.4).

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P1 | `StringV2`-kinded top-level slot (string read out of an `Array<string>`) captures fine but **never restores** — structured kind-mismatch on every resume; for-over-array with a checkpoint inside is the natural repro | §9.1 transcript; serialize arm `snapshot.rs:1568` vs restore arms `snapshot.rs:3242,3289` (no `(SV::String, StringV2)` arm) |
| 2 | P1 | Recompile-and-resume (`shape --resume <hash> file.shape`) is a **designed clean refusal** even for byte-identical source; book documents ordinal-remap as working | §9.2 transcript; `crates/shape-vm/src/execution.rs:215-239`; book `resumability.mdx:56-92` |
| 3 | P1 | `wire-serve` Execute breaks when `shape` is not on `$PATH` (`No such file or directory (os error 2)`), runs code with **full user permissions, no auth, no sandbox**; superseded by `serve` but still shipped | §9.4 raw-socket transcript; `wire_serve_cmd.rs:159` `Command::new("shape")` |
| 4 | P1 | Both servers' `validate` is **parse-only** — a type-broken program (`let x: int = "not an int"`) returns `success: true`; wire-serve's own doc comment claims "parse + type-check" | §9.5 transcript; `wire_serve_cmd.rs:17,171-181`; `serve_cmd.rs:927-946` |
| 5 | P2 | Verbatim duplicate `expected_kind_from_serializable` in shape-runtime (`snapshot.rs:3167`) and shape-vm (`executor/snapshot.rs:1086`) **has already diverged**: runtime copy maps `SV::ModuleFunction → Ptr(ModuleFn)`, VM copy falls through to `Bool` | §4.1 side-by-side |
| 6 | P2 | Closures and live references across `snapshot()` are **barrier-refused** (good messages), but the v0.3.3 user ruling pulled reference serialization INTO scope and the ratified DESIGN.md is still pre-implementation for the module-binding/return-ref flip | §2.1.5 transcripts; `docs/design/v0.3.3-reference-serialization/DESIGN.md` verdict block |
| 7 | P2 | Decompression-bomb guard checks size **after** `zstd::decode_all` fully materializes the payload — the 256 MB cap does not bound allocation; book claims it prevents bombs | `framing.rs:62-72`; book `wire-protocol.mdx:69-72` |
| 8 | P2 | QUIC: client-only (`quinn::Endpoint::client`; zero `Endpoint::server` hits repo-wide), feature-gated off in the shipped CLI (`default = ["jit","gc"]`); `transport::quic()` = "module has no export" in the shipped binary | §5.4 transcript; `bin/shape-cli/Cargo.toml:13` |
| 9 | P2 | `@remote` refuses functions whose parameters have heterogeneous element types ("Runtime annotation args require a single statically proven element type") — e.g. `(Array<string>, string)` params can't be shipped | §2.2.3 transcript |
| 10 | P2 | Resume results and `remote::execute` values print internal WireValue representation (`{"String": "RESUMED:43"}`, `value=Int(6)`) instead of rendered values | §2.2 transcripts; `script_cmd.rs:435-439` |

**Feature-completeness score: 78/100** — full resume, interrupt-resume, function-level
checkpoints, content-addressed transfer with receiver-side permission enforcement, and
polyglot×distributed all work end-to-end; deductions for the StringV2 restore bug, the
recompile-resume refusal, closure/reference capture barriers, and the QUIC/wire-serve gaps.

**Code-quality score: 82/100** — exemplary zero-trust receiver path, structured error taxonomy
with a load-bearing pre-send/post-send retry split, version-guarded formats, honest
surface-and-stop discipline throughout, and a 46-test CLI distributed E2E suite
(`bin/shape-cli/tests/distributed_*_e2e.rs`) whose receiver-log-assertion pattern is
distributed-testing done right (§7.1, §10.11); deductions for the 6,133-line `snapshot.rs`
monolith, duplicated kind-derivation tables, the legacy `wire-serve` server, 1,318 lines of
orphaned blob modules (§3.4), and high (mostly justified) unsafe density.

**Biggest risk:** the serialization arm-matrix (58 `SerializableVMValue` variants × `NativeKind`
expectations × two hand-maintained kind-derivation tables in two crates) has no mechanical
closure pressure. The StringV2 bug (finding 1) is exactly the class of asymmetry this breeds: a
new kind was added to the serialize side (W12 StringV2/DecimalV2 amendment, `snapshot.rs:1558`)
without a matching restore arm, and nothing — no exhaustive-match forcing, no round-trip property
test over the kind space — forced the pair to close. A capture that cannot restore is worse than
a barrier: users discover the loss only when they need the checkpoint. The same drift force
already shows in the ModuleFunction divergence between the duplicated tables (finding 5). The
proof the fix pattern works sits one module over: the sibling wire-marshal layer
(`wire_conversion.rs`) applied the same W12 amendment completely because its wildcard-free
match over `NativeKind` refuses to compile without every kind's arm (§1.5, §5.7).

---

## 1. Architecture & code structure map

### 1.1 Module inventory (LOC via `wc -l`, working tree)

| Module | LOC | Responsibility |
|---|---|---|
| `crates/shape-runtime/src/snapshot.rs` | 6,133 | Snapshot store (content-addressed zstd blobs), `ExecutionSnapshot` envelope, `VmSnapshot`, `SerializableVMValue` (58 variants), the kind-threaded slot↔serializable converters, v7 identity-map two-pass (serialize + restore link ctx), 5 embedded test modules |
| `crates/shape-vm/src/remote.rs` | 4,255 | Wire message enum (`WireMessage`), remote call request/response, `RemoteBlobCache` + negotiation, zero-trust `execute_remote_call*` receiver path, sidecar extraction/reassembly, call-request builders (by name / id / closure), ~43 tests |
| `bin/shape-cli/src/commands/serve_cmd.rs` | 2,970 | `shape serve`: TCP(+TLS) accept loop, sandbox-posture derivation, auth, Execute/Validate/Call/CancelCall/Negotiation dispatch, in-process execution, 7 tokio integration tests incl. a polyglot serve-node fixture |
| `crates/shape-vm/src/executor/snapshot.rs` | 2,428 | VM-side capture (`perform_snapshot_capture`, barriers), whole-VM restore (`from_snapshot` two-pass driver, `restore_call_stack`), `snapshot()`-marker construction, ip-relocation metadata, 33 tests |
| `crates/shape-wire/src/` (16 files) | 4,061 | `WireValue`/`ValueEnvelope`/codec (MessagePack + JSON), `AnyError` rendering, transport abstraction: TCP (324), QUIC (301, feature-gated), memoized (346), framing/zstd (140), factory (99) |
| `crates/shape-vm/src/linker.rs` | 807 | Content-addressed `Program` → `LinkedProgram`: topo-sort, pool merge + operand remap, foreign-ordinal inversion, transitive permission union |
| `crates/shape-vm/src/bytecode/content_addressed.rs` | 653 | `FunctionBlob` + SHA-256 identity (`FunctionBlobHashInput`), `Program`, `LinkedProgram`, ADR-006 §2.7.5 conduit side-tables |
| `crates/shape-vm/src/executor/vm_state_snapshot.rs` | 372 | Read-only `VmStateSnapshot` accessor for `std::state` introspection (distinct from resumable snapshots) |
| `crates/shape-vm/src/execution.rs` (resume portion) | ~250 of 1,000+ | `resume_snapshot` / `recompile_and_resume` orchestration, snapshot-store wiring |
| `bin/shape-cli/src/commands/snapshot_cmd.rs` | 99 | `shape snapshot list/info/rm`, store-root resolution (`SHAPE_SNAPSHOT_STORE` env / `--snapshot-store` / platform data dir) |
| `bin/shape-cli/src/commands/wire_serve_cmd.rs` | 182 | Legacy `wire-serve` TCP server (subprocess-per-request) |
| `crates/shape-runtime/src/wire_conversion.rs` | 1,674 | Kind-threaded slot↔WireValue marshal layer: `slot_to_wire(bits, kind, ctx)` / `wire_to_slot(wire, expected_kind)` / `slot_to_envelope` / `slot_extract_content`, per-HeapKind heap projection, TypedObject Result/Option ↔ `WireValue::Result`/Null-coding, DataTable ↔ Arrow IPC bytes; 20 tests in 5 modules |
| `crates/shape-runtime/src/blob_wire_format.rs` | 890 | Versioned cross-language FunctionBlob binary format ("SHBL" magic, 50-byte header, 8 msgpack section types, `validate_blob` SHA-256 check, `TypeMappingRegistry`); **orphaned** — no production caller (§3.4) |
| `crates/shape-runtime/src/blob_prefetch.rs` | 428 | Speculative blob prefetcher (call-probability graph from blob deps, top-N warming, stats); **orphaned** — no production caller (§3.4) |
| `crates/shape-runtime/stdlib-src/core/{snapshot,remote,transport,state}.shape` | 40+191+41+135 | Shape-side API: `snapshot()`, `Snapshot` enum, `@remote` annotation, `remote::{call,call_async,execute,ping}`, `RemoteError` enum, transport handles, `state::{capture_call,serialize,deserialize}` |
| `bin/shape-cli/tests/distributed_*_e2e.rs` (9 files) + `tests/support/distributed_snapshot_polyglot.rs` | 2,212 + 491 | CLI-level distributed E2E suite: 46 `#[test]`s driving the real `shape` binary + real `shape serve` child processes over real sockets (TCP and TLS) — matrix, proof-matrix, composition, content-addressed resupply/negotiation, async, async-cancellation, extern-C/dynamic/polyglot snapshot-resume; shared harness with `ServeNode` (stderr-log capture), `WireClient` (raw framed-MessagePack client), isolated snapshot stores (§7.1) |

### 1.2 Data flow

**Checkpoint path** (`snapshot()` builtin → resumable artifact):

```
Shape: snapshot()                      stdlib core/snapshot.shape
  → VM opcode dispatch                 executor/snapshot.rs:167 (snapshot())
  → perform_snapshot_capture           executor/snapshot.rs:835
      barriers: foreign frame live / no store / pending Future /
                live reference / live closure (each = SnapshotOutcome::Barrier)
  → VmSnapshot { ip, stack + stack_kinds (§2.7.7 parallel track),
                 module_bindings + kinds, call_stack (SerializableCallFrame),
                 blob-hash ip relocation fields }
  → slot_to_serializable_ctx           shape-runtime snapshot.rs:1435
      (per-slot NativeKind threaded; heap graphs → SerializableVMValue tree;
       cycle-capable kinds → HeapNode{handle,body}/HeapRef via SerializeIdentityCtx)
  → SnapshotStore.put_struct (bincode + zstd, SHA-256 addressed)
  → ExecutionSnapshot envelope { version=7, semantic_hash, context_hash,
                                 vm_hash, bytecode_hash, script_path, created_at_ms }
```

**Resume path** (`shape --resume <hash>`):

```
script_cmd.rs:309-444
  → SnapshotStore.resolve_hash (full hash or unique prefix, git-style)
  → engine.load_snapshot / apply_snapshot   (semantic + context restore)
  → get_struct::<VmSnapshot> + get_struct::<BytecodeProgram>
  → BytecodeExecutor::resume_snapshot       execution.rs:191
  → VirtualMachine::from_snapshot           executor/snapshot.rs:289
      Pass 1: materialize_cell_bodies (identity map, abort ledger)
      Pass 2: serializable_to_slot_ctx per slot with persisted kind track
      ip restore + restore_call_stack (OwnedClosureBlock rebuild when layout known)
  → push Ok(Snapshot::Resumed) marker       build_snapshot_resumed_marker
  → normal dispatch loop continues at saved ip
```

**Remote call path** (`@remote` / `remote::call`):

```
@remote annotation before-hook (stdlib remote.shape:180)
  → __call_raising builtin (remote_builtins.rs)
  → build_call_request* (remote.rs:2008+): entry blob + minimal transitive
    blob closure (incl. foreign entries post-WF-3E) + serialized args
  → WireMessage::Call over TcpTransport (length prefix + flags byte + zstd)
  → serve_cmd handle_connection → handle_call → spawn_blocking
  → execute_remote_call_with_runtimes (remote.rs:886):
      recompute every blob hash → reject mismatch (HashMismatch)
      accumulate missing deps (MissingModuleFunction, single round-trip resupply)
      linker::link → recomputed permission union
      load_linked_program_with_permissions(granted) → refuse or run
  → CallResponse (result or structured RemoteCallError)
```

### 1.3 Key types

- `ExecutionSnapshot` (`snapshot.rs:309`): version-stamped envelope; `SNAPSHOT_VERSION = 7`
  (`snapshot.rs:116`) with an explicit refuse-on-mismatch guard at `get_snapshot`
  (`snapshot.rs:209-217`) because bincode is non-self-describing.
- `VmSnapshot` (`snapshot.rs:513`): ip, stack image + **parallel kind track**
  (`stack_kinds: Vec<NativeKind>`), module bindings + kinds, serializable call frames, loop/
  timeframe/exception-handler stacks (persisted but not yet restored — §2.1.7).
- `SerializableVMValue` (`snapshot.rs:603-964`): 58 variants covering scalars, containers,
  closures (function_id + type_id + upvalues), TypedObject (schema_id + slot_data + heap_mask),
  DataTable/TypedTable via `BlobRef` chunked blobs, opaque dispositions (Iterator/Deque/Channel/
  FilterExpr = clean-refuse; Mutex/Lazy = defined-reset), and v7 `HeapNode`/`HeapRef` identity arms.
- `FunctionBlob` (`content_addressed.rs:33`): self-contained code unit; hash input includes
  permissions (sorted names), `frame_descriptor`, and `capture_kinds` — call-ABI and capability
  identity are hash-covered (`content_addressed.rs:119-152`).
- `WireMessage` (`remote.rs:330`): Auth/Ping/Execute/Validate/Call/CancelCall/BlobNegotiation/
  Sidecar + responses; `RemoteError` (stdlib `remote.shape:129`) with the pre-send/post-send
  retry-safety split documented per-variant.
- `SnapshotStore` (`snapshot.rs:123`): two flat dirs (`blobs/`, `snapshots/`), files named
  `<sha256>.bin.zst`; `put_blob` dedupes on existence (§2.1.8 shows 20 blobs / 240 KB after this
  session's runs).

### 1.4 Entry points

- CLI: `shape run file.shape` (auto-enables snapshot store, `script_cmd.rs:242-245`); `shape
  --resume <hash>` (full resume); `shape --resume <hash> file.shape` (recompile mode — currently
  refuses, §9.2); `shape snapshot list|info|rm`; `shape serve` (v2 protocol server); `shape
  wire-serve` (legacy v1).
- Shape code: `snapshot()`, `@remote(addr)`, `remote::{call,call_async,execute,ping}`,
  `transport::{tcp,memoized,send,connect,...}`, `state::{capture_call,serialize,deserialize}`.
- Rust embedder: `configure_quic_transport` (`executor/mod.rs:1142`, feature-gated),
  `set_transport_provider` (`executor/mod.rs:1128`).

### 1.5 What actually crosses the wire — two value vocabularies, one marshal layer

Two distinct serialized value shapes leave the process, and knowing which is which matters for
several findings:

- **`SerializableVMValue`** (snapshot wire shape, `snapshot.rs:603`) is the carrier for remote
  *call* arguments and results (`RemoteCallRequest.args` / `CallResponse.result`,
  `remote.rs:330+`) and for all snapshot state. It travels embedded in `WireMessage` via
  rmp-serde over the framed transport.
- **`WireValue`/`ValueEnvelope`** (`shape-wire`) is the host-boundary result shape for
  Execute responses and program/resume completion values.

The bridge from typed slots to the second vocabulary is
`crates/shape-runtime/src/wire_conversion.rs` (1,674 lines) — the Phase 2b kind-threaded
marshal layer. `slot_to_wire(bits, kind, ctx)` dispatches purely on the threaded `NativeKind`
(no tag-bit probing; heap slots dispatch per `HeapKind` with a debug-only consistency assert,
`wire_conversion.rs:42-146`). Notably, it *does* carry the W12 v2-carrier arms the snapshot
restorer lacks: `NativeKind::StringV2` projects via `StringObj::as_str` and `DecimalV2` via
`DecimalObj::value` (`wire_conversion.rs:111-138`) — the same ADR-006 §2.7.5 amendment that
§9.1 shows was left half-applied on the snapshot-restore side. It also carries the R5b-2
Null-kind fix (the `None`-renders-as-`{"Bool": false}` bug class, documented at
`wire_conversion.rs:74-84`), TypedObject-backed Result/Option projection to
`WireValue::Result`/Null-coding (schema-id driven, `wire_conversion.rs:551+`), and
DataTable ↔ Arrow IPC bytes (`datatable_to_ipc_bytes`/`from_ipc_bytes`,
`wire_conversion.rs:1176+`).

The inverse, `wire_to_slot(wire, expected_kind)` (`wire_conversion.rs:866-914`), is
deliberately narrow — unhandled `(wire variant, kind)` pairs return a structured
`MarshalError`, with an in-code note that each new arm is added as concrete consumers appear.
Today it has **zero production callers** in the workspace (only the `lib.rs:140` re-export;
its consumers are its own tests) — the inbound direction of this layer is staged, not live.
`slot_to_envelope` (`wire_conversion.rs:1021-1037`) currently **ignores its `type_name`
argument** (Phase 1.B placeholder — envelope `type_info` falls back to wire-side inference),
which is honest in-code but means envelope type metadata is inferred, not compiler-derived.

Production callers: `execution.rs:408,965` (host-boundary projection of fresh-run and resume
completion values — the direct upstream of finding 10's output), `execution.rs:1119,1198`.
The marshal itself is correct at these sites; finding 10's `{"String": "RESUMED:43"}` leak is
the CLI serializing the resulting `WireValue` enum with raw serde JSON (`script_cmd.rs:435-439`),
not a marshal bug.

---

## 2. Feature completeness (empirical)

Legend: **WORKS-E2E** (demonstrated in this session), **CODE-EXISTS** (read but not driven),
**DESIGNED-REFUSAL** (deliberate barrier with a good message), **BROKEN**, **MISSING**.

### 2.1 Snapshot / resume

#### 2.1.1 Top-level checkpoint + full resume — WORKS-E2E

```
$ SHAPE_SNAPSHOT_STORE=... shape run snap1.shape        # let mut acc = sum(0..5); snapshot(); print(acc*2)
pre-snapshot acc=10
LOCAL_SNAPSHOT=HASH:0f3f934e9a433d14a1249dd89af769583f56da1c910491376f8e5761c8f18541
post=20

$ shape --resume 0f3f934e9a433d14
Resuming from snapshot: 0f3f934e9a433d14...
resumed with acc=10          ← same snapshot() site returns Snapshot::Resumed
post=20                      ← post-checkpoint code re-ran with restored acc
```

Values survive (`acc=10`), control flow resumes at the checkpoint (the `pre-snapshot` line does
NOT reprint), and truncated-hash resolution works (`resolve_hash`, `snapshot.rs:265-289`).

#### 2.1.2 Function-level checkpoint with typed heap state — WORKS-E2E

`fn checkpointed(x: int, arr: Array<int>, p: Point) -> string` with `snapshot()` inside the frame:

```
$ shape run snap3.shape
LOCAL_SNAPSHOT=HASH:12ac32be...
$ shape --resume 12ac32be
resumed x=42 arr0=100 arr1=200 px=1.5 py=2.5
```

A deep call frame, its `int` arg, a heap `Array<int>`, and a user `type Point { x, y: number }`
all round-trip; the frame's continuation (`x + 1`) executes on the restored arg. Call-stack
restore is `restore_call_stack` (`executor/snapshot.rs:465`) rebuilding `CallFrame`s and
`OwnedClosureBlock`-backed upvalues when a `ClosureLayout` is registered.

#### 2.1.3 Ctrl+C interrupt snapshot + mid-loop resume — WORKS-E2E (discriminating test)

```
$ shape run loop2.shape &      # while i < 4000000, print every 1M
$ kill -INT $pid
i=1000000
i=2000000
Interrupting — saving snapshot...
Snapshot saved: 3eb98c77e66e379c87760a587de052d9904872319c955f96bc21f223f603925e
Resume with: shape --resume 3eb98c77...          (exit code 130 per design §4.4)

$ shape --resume 3eb98c77
i=3000000
i=4000000
done i=4000000
```

The resume prints **only** the remaining milestones — genuine mid-loop continuation of an
interrupted run, not a re-run. This is the exact scenario the WF-3F fix note in
`executor/snapshot.rs:339-357` addresses (the pre-fix `reset()` stack-image shift read loop
counters as 0). Exit-code contract (130 with or without a saved snapshot) matches
`script_cmd.rs:466-481`.

#### 2.1.4 Checkpoint inside `for` loops — range WORKS-E2E; array-iteration RESUME-BROKEN

Range loop (`for i in 0..10` with `snapshot()` at i==5): capture and resume both work; resume
prints `resumed at i=5 total=15` then `final total=45` (correct — iterations 6..9 ran post-resume).

Array loop (`for s in ["a","b","c","d"]`, checkpoint at the 2nd element): **capture succeeds,
resume always fails** —

```
$ shape --resume aa1b4fa2
Error: Runtime error: resume: failed to restore VM state: Not implemented:
VirtualMachine::from_snapshot stack[2]: serializable_to_slot: W17-snapshot-roundtrip
surface — SerializableVMValue arm String cannot satisfy expected kind StringV2. ...
```

Root cause and minimal repro in §9.1 (it is not the iterator — it is any `StringV2`-kinded slot).

#### 2.1.5 Closures and references across a checkpoint — DESIGNED-REFUSAL (catchable barrier)

```
# closure value live across snapshot():
no checkpoint: Barrier("cannot checkpoint here: a live closure value is reachable and
this build cannot yet save closures. Avoid holding a closure across the checkpoint, ...")

# shared reference live across snapshot():
err: Barrier("cannot checkpoint here: a live reference into your data is still active
at this point and cannot be saved. Finish using the reference, or move the snapshot()
call to a point where no borrow is held, then try again.")
```

Both surface as `Err(...)` on the `snapshot()` Result — the program continues (`closure after: 42`
and `after r.v=100` printed post-barrier). The messages are user-legible and actionable. However:
per memory note `project_reference_serialization_v033`, the user pulled reference serialization
(escape-RC-promote + serialize-with-identity) **into** v0.3.3 scope on 2026-05-29, and the ratified
design (`docs/design/v0.3.3-reference-serialization/DESIGN.md`) is explicit that the flip needs an
O1 carrier ruling and is **not yet implemented** for Local/ModuleBinding-rooted references. The
restore machinery for `SV::Reference`/`SV::SharedCell` (STAGE-R5 arms, `snapshot.rs:2376-2568`,
`link_promoted_reference` `snapshot.rs:3115`) exists and is tested at unit level, but the
capture-side barrier still fires for a plain `let r = &boxed` held across a checkpoint. Closure
capture is likewise CODE-EXISTS on the restore side (`SV::Closure` with layout rebuild,
`executor/snapshot.rs:465+`) while top-level closure *values* barrier at capture.

#### 2.1.6 Cyclic / self-referential structures — WORKS-E2E (language level) + unit-tested (v7)

`var a = Node { name: "a", next: None }; a.next = Some(a)` then checkpoint: serializer terminates,
capture and resume both succeed (`resumed, a.next.name=a`). The mutual-reference variant
(`a.next = Some(b); b.next = Some(a)` shape) also round-trips. At unit level,
`gc_phase5_identity_tests` (`snapshot.rs:5320+`) builds genuine raw self-cyclic
`TypedObjectStorage` graphs (refcount 2 = holder + self-edge) and asserts the restored node's
`next` field **aliases the restored node itself** — with the comment "Pre-v7 this
INFINITE-RECURSED the serializer". The v6→v7 version guard refuses old snapshots cleanly
(`snapshot.rs:209-217`); I could not fabricate a v6 blob to drive the guard empirically, but the
guard is value-based on the stable envelope field, which is the right mechanism for a
non-self-describing format.

#### 2.1.7 What is captured vs. restored — one honest gap

`VmSnapshot` persists `loop_stack`, `timeframe_stack`, and `exception_handlers`, but
`from_snapshot` deliberately does not restore them (`executor/snapshot.rs:434-446`: "empty
loop/timeframe state on resume is the documented contract... reserved for the
W17-snapshot-control-flow follow-up"). Empirically this did not bite my loop tests (while/range
`for` resumed correctly — those compile to jump-based loops), but a checkpoint inside a construct
that needs a live handler stack (e.g. resumed code that later hits `break` handled via the loop
stack, or an in-flight catch frame) would silently lose that state. Rated P2 risk in §9.6.

Frame-level introspection metadata is also partially stubbed in the *accessor* snapshot:
`FrameInfo.local_ip = 0` and `args = []` with named follow-ups
(`vm_state_snapshot.rs:122-130`) — this affects `std::state` introspection, not resume.

#### 2.1.8 Snapshot store + CLI — WORKS-E2E

```
$ shape snapshot list
HASH              CREATED                   SCRIPT
12ac32be8c1e0b6c  2026-07-11 09:54:17       snap3.shape
0f3f934e9a433d14  2026-07-11 09:53:17       snap1.shape

$ shape snapshot info 12ac32be
Hash: 12ac32be... / Version: 7 / VM state: yes / Bytecode: yes
```

Store layout after this session: `blobs/` = 20 zstd files, `snapshots/` = envelopes, 240 KB total
— chunked-blob dedup working. Gaps: `delete_snapshot` removes only the envelope, never the
referenced VM/bytecode blobs (`snapshot.rs:294-301`) — orphaned blobs accumulate forever (no
store GC); `get_blob` never re-verifies the SHA-256 of decompressed bytes against the filename,
so a corrupted/tampered store file is trusted silently (§11.4).

### 2.2 Distributed execution (`shape serve` + `@remote`)

#### 2.2.1 Pure-Shape remote transfer — WORKS-E2E

```
$ shape serve --address 127.0.0.1:9702 --sandbox none      # session-started node
$ shape run remote1.shape --mode vm                        # @remote extern-C labs()
REMOTE_C_ABS=42
server log: [serve] inbound Call id=None fn="remote_abs___impl" blobs=2 foreign_entries=1

# heap args round-trip:
GREET=hello-world        # (a: string, b: string) -> string
SUM=10                   # (xs: Array<int>) -> int
```

Controls proving genuineness: dead port → `remote call to 127.0.0.1:9799 failed: remote:
transport error: connection failed: ... Connection refused` (pre-send `Transport` class);
`--sandbox strict` node → `remote call 'remote_abs___impl' refused — the server does not grant
[ffi.call]` (receiver-side permission union enforcement).

#### 2.2.2 Polyglot × distributed — WORKS-E2E (re-verified, per tasking)

```
$ shape serve --address 127.0.0.1:9704 --sandbox none --ffi-languages python,typescript
$ shape run remote_py.shape --mode vm      # @remote wrapper calling fn python padd(x)
REMOTE_PY=105
server log: inbound Call fn="remote_py___impl" blobs=2 foreign_entries=1
```

The client never loads the Python extension; the opted-in receiver executes the foreign body.
This re-confirms the memory note `project_polyglot_distributed_composition` (COMPOSES,
post-WF-3E) on the current working tree, including the `blobs=2` signal that the foreign stub
blob travelled.

**Combined (transfer + receiver-side mid-run snapshot + resume) — WORKS-E2E:**

```
$ shape serve --address 127.0.0.1:9705 --sandbox none --snapshot-store rcvstore
$ shape run remote_snap.shape --mode vm
REMOTE_C_SNAPSHOT=HASH:ea954d5947c18c6b...
$ SHAPE_SNAPSHOT_STORE=rcvstore shape --mode vm --resume ea954d59
{ "String": "RESUMED:43" }     ← before=42 survived; post-resume labs(-1) re-linked and ran
```

(The JSON-wrapped output is finding 10 — the book transcript shows a bare `RESUMED:43`.)

#### 2.2.3 `@remote` parameter-shape limitation — P2

```
fn remote_join(names: Array<string>, sep: string) -> string  +  @remote(...)
error[SEMANTIC]: cannot build annotation args for function 'remote_join': parameters have
heterogeneous element types. Runtime annotation args require a single statically proven
element type.
```

Homogeneous params (two strings; one `Array<int>`) work. This is an annotation-machinery limit
(the `before(args, ctx)` hook needs a typed `args` array), and it silently constrains which
functions are remotable — undocumented in the book's `@remote` sections.

#### 2.2.4 Receiver zero-trust pipeline — CODE-EXISTS, partially driven

Read end-to-end at `remote.rs:1020-1160`: (a) every received blob's hash recomputed and
mismatches rejected (`HashMismatch`); (b) missing deps accumulated into a structured
`MissingModuleFunction` for single-round-trip resupply — this leg is also **client-driven E2E
tested and green this session** (`distributed_content_addressed_e2e.rs` hand-strips the blob
set over a raw socket and completes the resupply round-trip, §7.1); (c) permission union
recomputed by
`linker::link` from the *verified* blobs (`linker.rs:407`) and gated against the receiver grant
via `load_linked_program_with_permissions` (`program.rs:415`), with the `PermissionDenied` path
empirically driven in §2.2.1. Note the **full-payload fallback** (`remote.rs:1138-1143`): a
request without content-addressed metadata skips hash verification and the load-time permission
union check; runtime `check_permission` gating (fail-closed since `granted` is `Some`) remains
the boundary there. Defensible, but it is a second, weaker trust path (§11.6).

#### 2.2.5 `remote::{ping,execute,call}` primitives — WORKS-E2E

```
ping ok v0.3.2 proto=2
execute value=Int(6)          ← WireValue leak again
call ok: 42                   ← remote::call("127.0.0.1:9702", triple, 14), typed Result<R, RemoteError>
```

Arg-order note: `remote::call(addr, fn, args...)`; passing the fn first yields a good semantic
error ("must name a statically-known function or closure binding"). `RemoteError`'s
pre-send/post-send variant split (stdlib `remote.shape:129-163`) is a genuinely good API design
(§10.3). Cancellation (`CancelCall` + `RemoteCallRegistry` queued/running/finished states,
`serve_cmd.rs:220-291,1293-1321`) and blob negotiation (`negotiate_blobs`, `remote.rs:1967`)
are **WORKS-E2E via the CLI test suite** (§7.1): `distributed_async_cancellation_e2e.rs` drives
real client programs whose `async scope` exit / `join race` loser must produce a `CancelCall`
in the serve node's log (asserted at `:168,:229`), and
`distributed_content_addressed_e2e.rs:72,173` round-trips `WireMessage::BlobNegotiation` over a
live connection. I ran both content-addressed tests and the scope-cancel test in this session —
all green (transcript in §7.1).

### 2.3 Wire protocol & transports

- **TCP transport** — WORKS-E2E (it carried every remote call above). Length-prefix + flags byte
  + zstd ≥256 B when smaller (`framing.rs:25-48`); 64 MB payload cap (`tcp.rs:12`).
- **`serve` protocol v2** (Auth/Ping/Execute/Validate/Call/Cancel/Negotiation, JSON framing for
  light clients per `lib.rs:58-60`) — Ping/Execute/Call driven E2E; Auth enforced per-message
  when a token is configured (`serve_cmd.rs:603-660`); non-loopback bind refuses without TLS
  *and* token (`serve_cmd.rs:308-332`), TLS termination active when configured
  (`build_tls_acceptor`, `serve_cmd.rs:522`).
- **`wire-serve` protocol v1** — WORKS only if `shape` is on `$PATH`; see §9.4. Driven over a raw
  socket with hand-built MessagePack: `version` → `{shape_version: "0.3.2", wire_protocol: 1}`;
  `execute print(6*7)` → `output "42\n"` (PATH-fixed run).
- **QUIC** — CODE-EXISTS, NOT SHIPPED: `transport::quic()` in the shipped binary →
  `error[SEMANTIC]: module 'transport' has no export 'quic'` (feature `quic` absent from
  `shape-cli` default features, `Cargo.toml:13`); client-only even when compiled in.
- **Memoized transport** — WORKS (constructed): `transport::memoized(16)` returns a handle;
  cache keyed on SHA-256(destination+payload), LRU + stats (`memoized.rs`, 9 unit tests).
- **Module import quirk**: the book's `from std::core::transport use { tcp, ... }` form fails
  (`Undefined function: 'tcp'`) while `use std::core::transport` + `transport::tcp()` works —
  native modules and .shape stdlib modules have different import surfaces (§8.5).

### 2.4 Content-addressed bytecode & linker — WORKS-E2E (driven directly and indirectly)

Every remote call above exercised blob build → wire → verify → link → execute, and the
missing-dep/negotiation legs are directly driven by `distributed_content_addressed_e2e.rs`
(ran green this session, §7.1). Hash identity
covers instructions, constants, strings, deps, type schemas, sorted permission names,
`frame_descriptor`, and `capture_kinds` (`content_addressed.rs:119-152`) — permissions and call
ABI cannot be tampered without changing the hash. Linker: topo-sort with cycle detection, pool
overflow checks, structured errors for malicious blobs (`MissingForeignEntry`,
`ForeignOrdinalOutOfRange` — "never an out-of-bounds index panic", `linker.rs:23-62`), transitive
permission union (`linker.rs:407-409`). 10 tests in `linker_tests.rs`.

---

## 3. Code quality

### 3.1 Idiom & naming — good, with heavy narrative commenting

The territory is consistently typed-carrier idiomatic: `KindedSlot` construction always pairs an
explicit retain (`clone_with_kind`) with the claim (`vm_state_snapshot.rs:306-335` documents the
double-release bug class the pattern prevents, with the W5 root-cause history inline). Error
handling is `Result` + `thiserror` structured enums everywhere network input can flow
(`LinkError`, `TransportError`, `RemoteCallError`/`RemoteErrorKind`), and the WF-2C "de-panic"
discipline is visible: linker errors explicitly promise "never an out-of-bounds index panic" for
malformed received blobs (`linker.rs:34-52`).

Comment density is extreme — much of `snapshot.rs` and `content_addressed.rs` reads as an ADR
change-log (wave IDs, ratification dates, §-references on nearly every field). This is
deliberately load-bearing for the project's agent workflow, but it also means genuinely dead
narration survives: `content_addressed.rs:241` still says "cached-program loads fall through to
the **legacy NaN-boxed path**" — a path CLAUDE.md declares deleted; the comment describes a
fallback that can no longer exist as named.

### 3.2 Unsafe usage — high density, mixed justification coverage

Counts (`grep -c 'unsafe '`): `shape-runtime/snapshot.rs` **105**, `executor/snapshot.rs` 13,
`vm_state_snapshot.rs` 4, `remote.rs` 4, linker/content_addressed/serve_cmd **0**, shape-wire
**0**. The snapshot serializer necessarily walks raw v2 heap carriers (`*mut TypedObjectStorage`,
`StringObj`, closure blocks), so density is expected. However, a mechanical scan finds only
**18 of 104** unsafe sites in `snapshot.rs` have a `SAFETY:` comment within the preceding 6
lines (21 `SAFETY` markers total in the file). The ones I read closely (`StringV2` arm
`snapshot.rs:1568-1580`, cyclic-object test harness `snapshot.rs:5349+`, closure-block borrow
`vm_state_snapshot.rs:269-300`) carry precise construction-contract citations; the long
`slot_heap_to_serializable` / `serializable_to_heap_slot` bodies have many bare `unsafe` blocks
relying on a contract stated once far above. Not a soundness finding per se, but at 105 sites in
one 6,133-line file, per-site SAFETY discipline is the cheap insurance this file doesn't have.

### 3.3 Complexity hotspots

| Function | Span | Size |
|---|---|---|
| `slot_heap_to_serializable` | `snapshot.rs:1744-2376` | ~632 lines, one match over HeapKind with nested per-variant graph walks |
| `serializable_to_heap_slot` | `snapshot.rs:3544-4017` | ~473 lines, inverse match |
| `from_snapshot` + `restore_call_stack` | `executor/snapshot.rs:289-592` | ~300 lines, two-pass + ledger |
| `run_serve` + `handle_connection` | `serve_cmd.rs:292-823` | ~530 lines combined |
| `execute_remote_call_with_runtimes` | `remote.rs:886-1500+` | ~600 lines: verify → link → gate → marshal → run → project |

The two giant serializer matches are the maintenance core of the vertical: every new HeapKind or
NativeKind must be added to both, plus the two `expected_kind_from_serializable` tables, plus
`serializable_arm_name` (`snapshot.rs:4205`). §9.1 shows what happens when one side is missed.

### 3.4 Dead code in-territory

- `serialize_datatable` / `deserialize_datatable` (`snapshot.rs:4271-4290+`) are
  `#[allow(dead_code)]` — "staged ahead of the snapshot wire path that drives it". DataTable
  snapshot support is therefore CODE-EXISTS-UNWIRED despite the `SerializableDataTable` /
  `BlobRef` wire shapes existing.
- `bytes_as_slice` / `slice_as_bytes` helpers (`snapshot.rs:1172-1190`) are `#[allow(dead_code)]`.
- `wire_serve_cmd.rs` ignores its own `--mode` and `--extension` CLI args (`_mode`, `_extensions`,
  `wire_serve_cmd.rs:53-55`) — flags parse and silently do nothing.
- `serve_cmd.rs` `ServeConfig._mode` (`serve_cmd.rs:411`) — the serve node likewise never uses
  the vm/jit mode flag it accepts.
- Legacy-named but live: `serializable_to_slot_ctx_legacy` / `into_legacy_snapshot_pair`
  (`snapshot.rs:3024,3354`) are the raw-pair compatibility boundary, still called from the
  public `serializable_to_slot_ctx` — "legacy" here is API-shape, not deadness; a rename or a
  doc comment distinguishing them from deletion-fate code would prevent confusion with the
  Forbidden-Patterns vocabulary.
- **`blob_wire_format.rs` (890 lines) is a whole orphaned module**: a versioned cross-language
  FunctionBlob binary format ("SHBL" magic + 50-byte header + 18-byte section-table entries +
  8 msgpack sections + `validate_blob` SHA-256 verification + a `TypeMappingRegistry`) whose
  only in-workspace reference is the `pub mod` in `lib.rs:23`. Its module doc promises a
  `From<&FunctionBlob>` conversion "in shape-vm"; no such impl exists — `EncodableBlob` is
  never constructed outside the module's own 12 tests. The *live* wire path serializes
  `FunctionBlob` structs directly inside `WireMessage` via rmp-serde (§5.7). Two latent flaws
  worth noting if it is ever wired: `decode_from_bytes` (`blob_wire_format.rs:365-495`) never
  verifies the header's content hash (verification is the separate, optional `validate_blob`
  call), and its section-bounds check compares against the attacker-controlled header
  `total_size` rather than `data.len()`, so a truncated buffer with an inflated section table
  panics on slice indexing — the exact malformed-input-panic class the linker's de-panic
  discipline (`linker.rs:34-52`) eliminates.
- **`blob_prefetch.rs` (428 lines) is likewise orphaned**: a speculative prefetcher
  (call-probability graph from blob dependencies, top-N callee warming, hit/waste stats) with
  10 unit tests and zero production callers (`pub mod` at `lib.rs:21` is the only reference).
  Nothing in `remote.rs`, the blob cache, or the JIT consults it.
- `wire_conversion::wire_to_slot` — live tests, `pub` re-export, **no production caller**
  (§1.5). Staged inbound-marshal API; not dead narration, but a one-way layer today.

### 3.5 Error-message quality — a genuine strength

Barrier messages name the problem and the remediation in user vocabulary ("Finish using the
reference, or move the snapshot() call..."). `render_capture_barrier` has a dedicated test that
internal jargon never leaks (`executor/snapshot.rs:1181-1205`,
`render_capture_barrier_never_leaks_internal_jargon`). The v6→v7 refusal explains *why* and
*what to do* ("Re-capture the snapshot with a matching build"). `resolve_hash` turns the
truncated-hash footgun into git-style prefix resolution with clean ambiguity errors
(`snapshot.rs:265-289`). This is well above the codebase median.

---

## 4. Duplication & DRY violations

### 4.1 `expected_kind_from_serializable` — duplicated AND diverged (dangerous)

- Copy A (pub): `crates/shape-runtime/src/snapshot.rs:3167-3198`.
- Copy B (private): `crates/shape-vm/src/executor/snapshot.rs:1086-1135`.

Both map `SerializableVMValue` discriminators to `NativeKind` with a `_ => NativeKind::Bool`
surface-clean fallback. They are line-for-line identical **except**: Copy A maps
`SV::ModuleFunction(_) => NativeKind::Ptr(HeapKind::ModuleFn)` (`snapshot.rs:3196` region);
Copy B has **no ModuleFunction arm** — it falls to `Bool`. Copy B is the one used by the
whole-VM restore driver for pre-R5 snapshots lacking a persisted kind track
(`executor/snapshot.rs:366,384`). Consequence: a legacy-track snapshot whose stack holds a
module-function value would restore through the `Bool` mismatch path in the VM driver while the
runtime-side API would accept it — same input, two behaviors, purely from copy drift. Since
Copy A is `pub`, Copy B could be deleted outright.

### 4.2 `remap_operand` — two parallel switch tables over `Operand`

- Linker version: `linker.rs:177-263` (full remap incl. function/foreign resolution).
- Hot-patch version: `executor/mod.rs:1173-1197` (const/string pools only, for blob splices).

Different jobs, but both must enumerate which operands reference pools. The executor version's
exemption list is a comment (`executor/mod.rs:1192-1196`); a new pool-referencing operand added
to the linker's match would compile clean while the hot-patch path silently fails to remap it —
a wrong-constants bug at hot-reload time. A shared `fn operand_pool_refs(&Operand)` classifier
would collapse the drift surface.

### 4.3 Frame/upvalue recovery — same 30-line ritual three times

`try_borrow_closure_block` + retain + `read_capture_kinded` loop appears in
`vm_state_snapshot.rs:143-174` (accessor), `executor/snapshot.rs:1003-1080` (serialize), and the
restore-side rebuild in `restore_call_stack`. Each carries its own SAFETY narration for the same
Q10 contract. One `for_each_capture_kinded(frame, |bits, kind|)` helper would centralize the
unsafe surface.

### 4.4 Two independent wire servers (see also §5.1)

`wire-serve` (v1, subprocess, 182 lines) and `serve` (v2, in-process, 2,970 lines) both implement
length-prefix + framing + MessagePack dispatch loops independently (`wire_serve_cmd.rs:73-112`
vs `serve_cmd.rs:576-601`). The v1 loop lacks the 256 MB message cap the v2 loop has
(`serve_cmd.rs:586-588` vs none in `wire_serve_cmd.rs:82`) — an unbounded `vec![0u8; msg_len]`
allocation from a 4-byte header on the legacy server.

### 4.5 Store-root resolution duplicated CLI-side — resolved correctly

`snapshot_store_root` (`snapshot_cmd.rs:9-17`) is the single resolution rule and `run`/`serve`/
`snapshot` subcommands all delegate to it — this one is done right and worth naming as the
pattern the others should follow.

---

## 5. Split-brain analysis

### 5.1 `wire-serve` vs `serve` — same concept, two protocols, one obsolete

The CLI ships two servers answering the same question ("run Shape code for a remote client").
`serve`'s own clap doc says it "replaces wire-serve" (`cli_args.rs:326`), yet v1 remains: no
auth, no sandbox derivation, no TLS path, subprocess execution resolving `shape` from `$PATH`,
protocol reports `WIRE_PROTOCOL_V1` while `serve` reports V2. Every security property the team
built into `serve` (§2.2, §11) silently does not exist on the other listener. Drift is not
hypothetical — it is the current state. Deletion (or reduction to an alias) is the fix.

### 5.2 Serialize-side vs restore-side arm matrices — the structural split-brain

The serializer (`slot_to_serializable*`, keyed on `NativeKind`) and restorer
(`serializable_to_*slot*`, keyed on `(SV variant, expected NativeKind)` pairs) are two
hand-maintained projections of one bijection. Nothing forces closure: the W12 amendment added
`NativeKind::StringV2/DecimalV2 → SV::String/SV::Decimal` on the serialize side
(`snapshot.rs:1558-1596`) and no restore arm accepts those pairs (§9.1). The persisted kind
track (a correctness feature) is what *exposes* the gap — restore knows the slot was StringV2
and correctly refuses to fabricate. The missing piece is a round-trip property test iterating
`NativeKind` × representative values, which would have caught this the day it was introduced.

### 5.3 Doc-vs-code splits

- Book `resumability.mdx:56-92` documents recompile-and-resume with ordinal remap and safe-edit
  rules; code refuses unconditionally (`execution.rs:215-239`). The in-code design note is
  honest about why (no frame-relocation producer yet); the book is not (§8.2).
- Book `wire-protocol.mdx:69-72` claims the 256 MB decompression cap "prevents decompression
  bombs"; the check runs after full materialization (`framing.rs:62-72`).
- `docs/codebase-index.md:94` pins "Snapshot capture" at `executor/snapshot.rs:80`; it now
  starts at `:167` (index staleness, minor).
- `wire_serve_cmd.rs:17` doc comment: "Validate Shape code (parse + type-check)" — implementation
  is parse-only (`wire_serve_cmd.rs:171-181`).

### 5.4 QUIC: three availability stories

(1) CLAUDE.md repo table: "MessagePack serialization and QUIC transport, wire protocol v1" —
reads as shipped. (2) Book `transport-layer.mdx:265-320`: honest that `quic` is a cargo feature
requiring host-side Rust configuration. (3) Shipped binary: feature absent
(`shape-cli/Cargo.toml:13` `default = ["jit", "gc"]`), so `transport::quic()` is
"module has no export"; and repo-wide there is **no QUIC server** (`Endpoint::client` only,
`quic.rs:52`; zero `Endpoint::server` hits), so even a feature-enabled build has no Shape peer
to call. Three surfaces, three different answers; only the book's is close to true.

### 5.5 VM-vs-JIT for this vertical — cleanly resolved by design

Snapshot/resume is VM-only by explicit contract (book `resumability.mdx:127-140`;
snapshot-bearing functions never tier up). The `[jit-fallback]` whole-program deopt observed on
every `snapshot()` test program (EnumPayload pattern lowering, §2.7.17 surface-and-stop) means
in practice *any* program matching on `snapshot()`'s Result runs interpreted end-to-end. Honest,
loud, and consistent — but it also means this flagship vertical currently forfeits the JIT tier
entirely, which is worth stating in the book's performance sections.

### 5.6 Two message-size ceilings

Client transport caps payloads at 64 MB (`tcp.rs:12`); the serve accept loop tolerates 256 MB
inbound (`serve_cmd.rs:586`). A third cap (256 MB decompressed, `framing.rs:14`) sits between
them. Not a bug — a Shape client can never hit the server ceiling — but three unaligned
constants in one pipeline is drift surface for any non-Shape client implementation.

### 5.7 Two serialized-value vocabularies + a second, unwired blob encoding

Related but distinct from §5.2 (which is serialize-vs-restore *within* the snapshot shape):

- **`SerializableVMValue` vs `WireValue`** (§1.5). Remote-call args/results and snapshots speak
  the first; Execute responses and completion envelopes speak the second, produced by
  `wire_conversion.rs`. Both are legitimate (they answer different questions — full-fidelity VM
  state vs host-consumable value), but they are two independent projections of the same
  `NativeKind`/`HeapKind` space, and the W12 StringV2/DecimalV2 amendment landed **completely in
  one** (`wire_conversion.rs:111-138` handles both carriers — forced to, because `slot_to_wire`
  is a wildcard-free compiler-exhaustive match over `NativeKind`) **and half in the other**
  (snapshot serialize yes, snapshot restore no — §9.1, where the restore key is a
  non-exhaustible `(SV, kind)` pair). Same amendment, three tables, one gap: exactly the drift
  shape §5.2 predicts, now visible *across* vocabularies, not just within one.
- **`blob_wire_format.rs` vs the live `WireMessage` embedding.** The repo contains two encodings
  for shipping a `FunctionBlob`: the live one (blob structs rmp-serde-embedded in
  `WireMessage::Call`, hash-verified at `remote.rs:1064-1079`) and the orphaned "SHBL"
  sectioned format (§3.4) built for cross-language implementations, which nothing produces or
  consumes. Until a non-Rust consumer exists, the second format is a parallel implementation
  with its own version constant (`WIRE_FORMAT_VERSION = 1`), its own hash-verification story
  (opt-in, unlike the live path's mandatory recompute), and its own drift clock. Wire it or
  delete it (§12).

---

## 6. ADR & spec conformance

Marker density: 77 `ADR-005`/`ADR-006` citations in `shape-runtime/snapshot.rs`, 38 across
`executor/snapshot.rs` + `remote.rs` + shape-wire. Rule-by-rule for the rules that bind this
territory:

### 6.1 Forbidden Patterns (CLAUDE.md) — CONFORMS

Grep for every forbidden symbol family (`synthesize_value_word_from_raw`, `is_tagged`,
`SlotKind::Dynamic|Unknown`, `exec_*_dynamic_fallback`, `push_raw_u64`/`pop_raw_u64`,
`normalize_persisted_for_slot`) across the territory: **zero live hits**. All `ValueWord`
mentions are deletion-fate documentation (e.g. `snapshot.rs:12-13,89-91` describing the deleted
v5 format; `remote.rs:3463,3754` naming deleted constructors in test comments). The snapshot
wire shape uses parallel `Vec<u64>`-equivalent data + `Vec<NativeKind>` kinds per §2.7.7/Q9 —
`VmSnapshot.stack_kinds` / `module_binding_kinds` (`snapshot.rs:513+`), exactly the mandated
shape (no `Vec<KindedSlot>`, no packed tags, no `Option<NativeKind>` placeholders).

### 6.2 ADR-006 §2.7.5.1 (SV discriminator authoritative; no Bool-default fabrication) — CONFORMS, with a sharp edge

Restore refuses discriminator-vs-kind mismatches with structured errors naming the arm and the
§ (the exact error text in §9.1 demonstrates the rule firing). The two
`expected_kind_from_serializable` tables document their `_ => NativeKind::Bool` fallthrough as
a deliberate surface-trigger, *not* a value fabrication (`snapshot.rs:3164-3166`) — conformant,
but §9.1 shows the same refusal discipline converts a serializer-side gap into a permanently
dead snapshot. Conformance is not the issue; arm-matrix closure is.

### 6.3 ADR-006 §2.7.7 / §2.7.8 (parallel kind tracks; Q10 cell storage) — CONFORMS

Capture reads kinds from the live parallel track with direct indexing that "surfaces any
invariant violation as a loud OOB panic instead of fabricating a Bool no-op sentinel"
(`vm_state_snapshot.rs:100-107,196-199,216-222`). Closure upvalues are read via
`OwnedClosureBlock::read_capture_kinded` (the Q10 layout side-table) at all three sites
(accessor `vm_state_snapshot.rs:169`, serialize `executor/snapshot.rs:1052`, restore rebuild).
The `clone_slot_kinded` retain-before-claim pattern (`vm_state_snapshot.rs:306-335`) mirrors the
canonical `module_binding_read_owned_kinded` shape and documents the W5 double-release class it
fixes.

### 6.4 ADR-006 §2.7.9 (FilterExpr pure-discriminator label) — CONFORMS

`SV::FilterExprOpaque` exists as a wire arm; capture refuses live FilterExpr as clean-refuse
by design, and restore's arm is terminal defense-in-depth
(`opaque_disposition_tests`, `snapshot.rs:4860-4888` asserting the wording and that
`as_heap_value()` is never invoked on FilterExpr-labeled bits).

### 6.5 ADR-006 §2.7.30.5 (STAGE-R5 two-pass identity restore) — CONFORMS

`SerializeIdentityCtx` (reserve-before-recurse cycle guard, provenance-pointer interning,
`snapshot.rs:1250-1283`) and `RestoreLinkCtx` (identity map + LIFO abort ledger,
`snapshot.rs:1290-1360`) implement the two-pass with leak/double-free protection on abort
(`from_snapshot` releases base shares on both success and failure,
`executor/snapshot.rs:398-401`). The v7 identity generalization to all cycle-capable HeapKinds
is directly tested (`gc_phase5_identity_tests`: object cycles, heap-element arrays,
TypedObject-valued maps; identity-aliasing assertions).

### 6.6 ADR-005 §1 (single discriminator / no parallel HeapKind-projecting sum types) — CONFORMS WITH A NAMED DEBT

`SerializableVMValue` is, structurally, a 58-variant sum type many of whose arms project 1:1 to
HeapKinds — the shape ADR-005 warns about. The file itself flags this: the `TypedObject` arm
carries an ADR-005 forward-pointer noting "Audit of this path for full ADR-005 conformance is
queued for a future cluster" (`snapshot.rs:643-650`). A wire schema legitimately needs its own
enum (serde requires a concrete data shape; you cannot serialize a raw pointer graph), so I
assess this as an *allowed serialization-boundary projection* rather than a violation — but the
drift the ADR predicts is exactly what §9.1 found, so the queued audit should be treated as due.

### 6.7 Distributed design contract (§4.x, ratified 2026-07-05) — CONFORMS on every point checked

- §4.3-2 hash re-verification on receipt: `remote.rs:1064-1079` (empirically exercised via
  every successful call; the negative path is unit-tested).
- §4.6 receiver-owned permissions, zero sender trust: `remote.rs:1037-1047` + strict-node
  refusal transcript (§2.2.1); `ffi_languages` strict-empty opt-in (`serve_cmd.rs:160-190`).
- §4.7 non-loopback posture: refuse without TLS+token (`serve_cmd.rs:308-332`); non-loopback
  grant clamped to pure (`serve_cmd.rs:147-152`).
- §4.8 call-ABI identity hash-covered: `frame_descriptor` + `capture_kinds` in
  `FunctionBlobHashInput` (`content_addressed.rs:133-142`).
- §4.9 RemoteError normative mapping incl. pre/post-send split: stdlib `remote.shape:129-163`.
- §4.5.2 recompile-refusal: implemented as specified (`execution.rs:200-239`) — the *book* is
  what diverges, not the code-vs-design pair.

### 6.8 Runtime-v2 spec (typed slots at the VM↔JIT ABI) — NOT VIOLATED BY THIS TERRITORY

`KindedSlot` appears only at GENERIC_CARRIER sites (snapshot capture/restore, module bindings,
suspension) per §2.7 — it does not leak into the typed slot ABI from anything in this vertical.
Snapshot/resume deliberately bypasses the JIT tier (§5.5), sidestepping the relocation problem
the spec would otherwise impose.

---

## 7. Test coverage in-territory

### 7.1 Counts

| Location | Tests | Character |
|---|---|---|
| `shape-runtime/snapshot.rs` | 30 (`#[test]`) in 5 modules | `l5_typed_object_result_option`, `wf2g_gap_b_heap_element_array`, `opaque_disposition`, `gc_phase5_identity`, `snapshot_wire_restore_miri_provenance` |
| `shape-vm/executor/snapshot.rs` | 33 | function-identity resolution (hash/id/name/ambiguity), ip-relocation field presence + legacy-absence, future-on-stack refusal ×4, foreign-frame refusal, marker construction |
| `shape-vm/remote.rs` | 43 | request builders, blob cache/negotiation, minimal-closure construction, hash-mismatch, sidecar extraction/reassembly |
| `shape-vm/linker_tests.rs` | 10 | link/remap/permission-union |
| `shape-vm/lib_tests_parts/interrupt_resume_tests.rs` | dedicated WF-3F regression | full engine-level SIGINT→snapshot→resume mirror of `shape run` |
| `serve_cmd.rs` | 7 (`#[tokio::test]`) | real-socket integration: auth, execute, remote_abs transfer, permission escape attempt (`file::write_text` over wire), TLS session, polyglot serve node (python + typescript transfer fixtures) |
| shape-wire (all files) | 60 | codec/envelope/metadata/formatter round-trips; tcp framing incl. oversize rejection; memoized cache incl. eviction/stats; framing threshold/boundary cases |
| `shape-runtime/wire_conversion.rs` | 20 in 5 modules | targeted regression tests: u64 full-range round-trip (i64::MAX+1 must not sign-corrupt), `NativeKind::Char` vs pre-amendment `Ptr(HeapKind::Char)` label (misaligned-deref guard), v2 typed-array projection, TypedObject Result/Option wire coding both directions; serialize-side closure comes from `slot_to_wire`'s wildcard-free match over `NativeKind` (compiler-enforced) rather than a test |
| `shape-runtime/blob_wire_format.rs` / `blob_prefetch.rs` | 12 / 10 | encode/decode/validate round-trips, tamper detection, section handling / graph construction, top-N selection, stats — all self-contained (modules are orphaned, §3.4) |
| **`bin/shape-cli/tests/distributed_*_e2e.rs`** (9 files, 2,212 lines + 491-line shared harness) | **46** | **CLI-level distributed E2E suite** — drives the real `shape` binary and real `shape serve` child processes over real TCP/TLS sockets; detailed below |

The CLI E2E suite deserves its own breakdown — it is the layer that turns the §2 flows into
regression tests:

| File | Tests | What it proves |
|---|---|---|
| `distributed_snapshot_polyglot_e2e.rs` (488) | 14 (1 ignored) | `snapshot()`→`snapshot info`→`--resume` CLI round-trip; `remote::execute`/`remote::call` user surfaces (Ok + transport-Err); receiver-side `snapshot()` over remote call, hash landing in the **selected receiver store** and resumable from it; extern-C transfer + strict-node ffi refusal; python/typescript transfer with self-skip when the extension `.so` is absent and opt-in refusal; TLS user surface (TLS ok + plaintext-against-TLS refused); SIGINT save+resume (ignored: timing-sensitive) |
| `distributed_async_cancellation_e2e.rs` (491) | 9 (all ignored: "timing-sensitive... run serialized under the supervisor cgroup lane") | `remote::call_async` cancellation: `async scope` exit and `join race` loser must complete substantially faster than a 1500 ms awaited control **and** leave `CancelCall` in the serve log (`:168,:229`); queued-call cancel must log `outcome=AcceptedQueued`/"before receiver execution"; running-call cancel must log honest `outcome=AlreadyRunning`/"not preemptible"; TLS variants of each; TLS-blackhole handshake cancellation |
| `distributed_async_e2e.rs` (327) | 9 | `remote::call_async` await/compose/`join all` ordering; callee-returned futures materialize payloads (sync + async + join-all); live-future checkpoint barrier then await; transport error as inner Result |
| `distributed_content_addressed_e2e.rs` (264) | 2 | hand-stripped blob sets over a raw `WireClient`: `MissingModuleFunction` resupply round-trip (entry-only → missing-helper report → resupply → `Int(22)`), nested `map`-closure blob discovery, then `WireMessage::BlobNegotiation` (`:72,:173`) proving verified blobs are reusable (`known_hashes` echo + empty-blob-vector re-call succeeds), with serve-log `blobs=N` assertions |
| `distributed_matrix_e2e.rs` (227) | 6 | TLS trust matrix (missing CA root refused, mismatched `server_name` refused); snapshot store isolation (receiver store, **not** caller store, gets the hash — plaintext + TLS); python/typescript refusal on a receiver without `--ffi-languages` opt-in |
| `distributed_proof_matrix_e2e.rs` (116) | 1 | TLS + `join all` of two remote snapshotting calls: both tagged hashes land in the receiver store only, verified via `snapshot info` against both stores |
| `distributed_composition_e2e.rs` (128) | 2 | TLS × python/typescript × receiver-side snapshot × resume-from-selected-receiver-store (the full four-way composition) |
| `distributed_dynamic_snapshot_e2e.rs` (113) | 2 | plaintext python/typescript remote snapshot-resume |
| `distributed_extern_c_snapshot_e2e.rs` (58) | 1 | extern-C remote snapshot hash resumable from receiver store |

Session verification: I ran both `distributed_content_addressed_e2e` tests and one ignored
cancellation test on the working tree —

```
test content_addressed_transfers_nested_map_closure_over_real_socket ... ok
test content_addressed_missing_dependency_resupplies_over_real_socket ... ok
test result: ok. 2 passed ... finished in 0.88s

test remote_call_async_scope_cancel_returns_promptly ... ok   (-- --ignored --exact, 7.66s)
```

The harness (`tests/support/distributed_snapshot_polyglot.rs`, 491 lines) is itself notable:
`ServeNode` captures the child server's stderr to a file so tests can assert *receiver-side*
facts (`serve_stderr`, `assert_serve_logged_foreign_stub` parsing `blobs=N foreign_entries=1`);
`WireClient` (`:404-452`) is a minimal raw-`TcpStream` client — it reuses the shape-wire codec
(`encode_message` + `encode_framed`) but hand-writes the length-prefixed loop instead of using
the production `TcpTransport`, so the content-addressed tests drive the server below the
client-side transport abstraction; `IsolatedEnv` gives every test its own snapshot store;
a process-wide mutex + OS-assigned per-node ports (bind-to-`:0` probe,
`start_serve_inner:244-247`) plus per-node self-signed rcgen TLS certs keep the socket tests
hermetic.

`#[ignore]` inventory: 10 of the suite's 46 tests are ignored — all with explicit reason
strings (9 timing-sensitive cancellation proofs + 1 SIGINT flow), i.e. gated for scheduling
reasons, not brokenness (the one I ran passes). Elsewhere in the territory there are zero
`#[ignore]` attributes (one serve test documents its gating as a comment instead of an ignore,
`serve_cmd.rs:2579` — the stated reason, avoiding a silently-rotting ignore, holds).

### 7.2 Assertion quality — high where it matters

The tests assert *mechanism*, not just outcomes: refcount balances before/after serialize
(`gc_phase5_identity_tests` reads `v2_get_refcount` around round-trips), identity aliasing of
restored cycle edges, barrier message wording (both that design vocabulary IS present and that
internal jargon is NOT — `render_capture_barrier_never_leaks_internal_jargon`), and a dedicated
Miri-provenance module for the raw-pointer restore path (`snapshot_wire_restore_miri_provenance`,
`snapshot.rs:5841+`). The serve tests drive real sockets including a **permission-escape
attempt** (a transferred function calling `file::write_text` must be refused —
`serve_cmd.rs:2527`).

### 7.3 Gaps

1. **No kind-space round-trip property test on the snapshot path** — the gap that let §9.1
   ship. A test iterating every `NativeKind` (and every `SV` arm) through
   `slot_to_serializable` → `serializable_to_kinded_slot` with the *persisted* kind as
   `expected_kind` would fail today on StringV2/DecimalV2. The sibling marshal layer shows the
   structural alternative: `wire_conversion::slot_to_wire` is a wildcard-free match over
   `NativeKind` (`wire_conversion.rs:42-140`), so a new kind cannot compile without a wire arm
   — which is exactly why the W12 kinds got arms there. The snapshot restore path is keyed on
   `(SV variant, expected kind)` *pairs*, where no exhaustiveness pressure exists; a property
   test is the only closure mechanism available to it short of restructuring.
2. **Every CLI-level `--resume` E2E carries only scalar or no top-level state.** The suite's
   resume tests (`snapshot_hash_resume_cli_roundtrip` asserts only the `SNAP_RESUMED` marker;
   the receiver-store resume tests carry an `int`; the WF-3F interrupt test uses scalar loop
   state) — a variant with a string-out-of-array binding would have caught §9.1 at the
   user-visible layer. This is the precise hole in an otherwise thorough E2E net: 46 tests
   cover transfer/security/store-isolation/cancellation combinatorics, and none holds a
   `StringV2`-kinded slot across a checkpoint.
3. **wire-serve has zero tests** (`wire_serve_cmd.rs` — none). Consistent with its
   deletion-candidate status, but it ships.
4. **The 9 cancellation E2E tests are all `#[ignore]`d** (timing-sensitive, reserved for a
   serialized supervisor lane), so the CancelCall wire round-trip — though genuinely
   client-driven-tested and green when run (§7.1) — is not exercised by any default-tier
   `just test*` run; a timing regression would surface only when the lane fires. (An earlier
   draft of this report claimed blob negotiation and CancelCall "lack client-driven E2E
   coverage" — that was wrong, an artifact of not surveying `bin/shape-cli/tests/`;
   `distributed_async_cancellation_e2e.rs:168,229` and
   `distributed_content_addressed_e2e.rs:72,173` are exactly that coverage.)
5. **No QUIC integration test** can exist (no server); the QUIC module's own tests are
   construction-only.
6. **`blob_wire_format.rs` / `blob_prefetch.rs` tests only prove the orphan modules against
   themselves** (§3.4) — 22 tests with no production integration to protect.

---

## 8. Book/docs vs reality

The book (`/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/`) devotes six
advanced chapters to this vertical: `resumability`, `polyglot-distributed`, `transport-layer`,
`wire-protocol`, `content-addressed-bytecode`, `module-distribution`, plus
`stdlib/core/snapshot.mdx`. Verdicts:

### 8.1 `polyglot-distributed.mdx` — ACCURATE (re-verified empirically)

The 3×3 composition matrix (`{extern C, python, typescript} × {remote, snapshot, combined}`)
claims all-green with "genuine" values. I reproduced the extern-C remote cell (`REMOTE_C_ABS=42`
with the documented `blobs=2 foreign_entries=1` server log), the python remote cell
(`REMOTE_PY=105` against an opted-in node), both genuineness controls (dead port → connection
refused; strict node → `does not grant [ffi.call]`, matching the book's predicted error), and
the combined extern-C cell (`HASH:` on transfer, `RESUMED:43` on resume). The page's honest-
limitation block (live-foreign-frame snapshot barrier) matches
`executor/snapshot.rs:840-856`. The only discrepancy: the book's resume transcript shows bare
`RESUMED:43`; the binary prints `{ "String": "RESUMED:43" }` (finding 10).

### 8.2 `resumability.mdx` — HALF-ACCURATE; the recompile section is FALSE for this build

True and verified: the two-pass `snapshot()` semantics (Hash → Resumed), full resume, function-
level checkpoints, the Ctrl+C flow with printed resume command, "code after snapshot() runs
again on resume", "resume flows are VM-based". **False:** "### 2) Recompile-and-Resume ...
restores runtime context, recompiles current source, remaps the saved snapshot() position by
ordinal, then resumes on the new bytecode" plus the entire "Safe Edit Guidance" and "Current
Recompile Boundary Rule" sections (`resumability.mdx:56-92`). Reality: **any** source argument —
including byte-identical (§9.2) — refuses with the frame-relocation-metadata message. The
in-code design note (`execution.rs:200-214`) says this is the v1 posture until the relocation
producer lands; the book describes the post-Stage-4 behavior as current. The book's "What Is
Captured" list also omits that loop/timeframe/exception-handler stacks are captured but not
restored (§2.1.7), and does not mention the closure/reference barriers anywhere on this page
(they surface only via the `Err` arm in examples).

### 8.3 `wire-protocol.mdx` — MOSTLY ACCURATE, one security overclaim

Frame format, compression policy/thresholds, and blob negotiation match `framing.rs`/`remote.rs`
exactly (constants agree: 256 B threshold, level 3, flags bit 0x01). Overclaim: "A
MAX_DECOMPRESSED_SIZE limit of 256 MB **prevents decompression bombs**" — the guard runs after
`decode_all` materializes the payload (`framing.rs:62-72`), so it bounds acceptance, not
allocation (§9.7). The `state::capture_call`/`serialize`/`deserialize` APIs referenced exist
(`stdlib-src/core/state.shape:78-112`); the pipeline snippets are marked `runnable=false`
(honest).

### 8.4 `transport-layer.mdx` — HONEST about QUIC, but the Shape import syntax shown is wrong

The page correctly labels QUIC as feature-gated with required host-side Rust configuration and
marks the Shape snippets as pseudocode. Two issues: (a) the import form it shows —
`from std::core::transport use { tcp, send, ... }` — fails on the shipped binary
(`Undefined function: 'tcp'`); the working form is `use std::core::transport` +
`transport::tcp()` (verified: "got tcp transport"). (b) Nothing on the page (or anywhere) says
there is **no QUIC server implementation**, so "QUIC peer" in the examples has no possible
Shape-side counterpart today.

### 8.5 `content-addressed-bytecode.mdx` / `module-distribution.mdx` — STRUCTURALLY ACCURATE

All snippets are `runnable=false`. Spot-checks hold: `FunctionBlob` field inventory matches
`content_addressed.rs:33-117`; hashing covers permissions (verified in `FunctionBlobHashInput`);
`MemoryBlobStore`/`FsBlobStore` exist (`crates/shape-runtime/src/blob_store.rs`); linker
description matches `linker.rs`. Not driven end-to-end in this session (module distribution is
primarily vertical 16's territory).

### 8.6 CLAUDE.md / codebase-index — two stale claims

- Crate table: shape-wire = "serialization (MessagePack) and QUIC transport, wire protocol v1
  at lib.rs:51". Reality: QUIC not in the shipped binary and client-only (§5.4); the crate now
  defines V1 **and** V2 (`lib.rs:56,60`), and `serve` speaks V2 — the index undersells the
  actual protocol.
- `codebase-index.md:94` "Snapshot capture → executor/snapshot.rs:80" — now `:167`.

### 8.7 The stale-audit correction that matters

`docs/audits`-adjacent memory (2026-07-04) recorded "snapshot/resume ... DEAD stubs despite
book". On this working tree that is **no longer true in any respect I could find**: capture,
store, list/info/rm, full resume, interrupt resume, function-frame resume, remote transfer, and
receiver-side snapshot+resume all work (transcripts in §2). Anyone triaging from that audit
should re-baseline on this report.

---

## 9. Bugs & correctness risks found

### 9.1 P1 — StringV2/DecimalV2 top-level slots: capture succeeds, restore always fails

**Repro (minimal):**

```shape
from std::core::snapshot use { Snapshot, snapshot }
let items = ["a", "b"]
let first = items[0]          // ← StringV2-kinded binding
match snapshot() { Ok(Snapshot::Hash(id)) => print(f"LOCAL_SNAPSHOT=HASH:{id}"), ... }
```

```
$ shape run strv2.shape            → LOCAL_SNAPSHOT=HASH:fafc3f15...   (capture OK)
$ shape --resume fafc3f15
Error: Runtime error: resume: failed to restore VM state: Not implemented:
VirtualMachine::from_snapshot module_binding[63]: serializable_to_slot:
W17-snapshot-roundtrip surface — SerializableVMValue arm String cannot satisfy
expected kind StringV2. ... ADR-006 §2.7.5.1.
```

Also reproduces via `for s in ["a","b","c","d"]` with a checkpoint inside the loop
(`stack[2]` variant), i.e. the most natural "checkpoint inside data processing" shape.

**Root cause:** serialize maps the v2 string carrier to the shared wire arm —
`NativeKind::StringV2 → SV::String` (`snapshot.rs:1568-1580`, W12 amendment) — and persists
`StringV2` in the kind track; restore has arms only for `(SV::String, NativeKind::String)`
(`snapshot.rs:3242`, `:3289`) and correctly refuses the mismatch. Same asymmetry exists for
`NativeKind::DecimalV2 → SV::Decimal` (`snapshot.rs:1586-1596`) vs restore expecting
`Ptr(HeapKind::Decimal)` — untested empirically but structurally identical. The sharpest
version of the diagnosis: the same W12 amendment was applied **completely** to the sibling
wire-marshal layer — `wire_conversion.rs:111-138` projects both v2 carriers, because its
wildcard-free match over `NativeKind` would not compile without them (§1.5) — and only
half-applied to the snapshot pair, whose restore is keyed on `(SV variant, kind)` pairs that
no compiler check can close. The knowledge existed in-repo; only the exhaustiveness-forced
projection got it.

**Impact:** any program whose top-level state (or frame locals) contains a string obtained from
an array — extremely common — produces checkpoints that can never be resumed. The failure is
deferred to resume time, so the user has already lost the work. **Fix is small:** accept
`(SV::String, StringV2)` / `(SV::Decimal, DecimalV2)` in the restore arms by constructing the
v2 carriers (or normalize the persisted kind at capture); add the kind-space round-trip test
(§7.3.1).

### 9.2 P1 — Recompile-and-resume refuses unconditionally; book says it works

```
$ shape --resume 0f3f934e snap1.shape        # byte-identical source
Recompiling with updated source: snap1.shape
Error: Runtime error: cannot resume with an edited source file in this build: sound ip
relocation into recompiled code requires the frame-relocation metadata that this snapshot
does not yet carry. Resume the original code with `shape --resume <hash>` (no source file).
```

`recompile_and_resume` (`execution.rs:215-239`) compiles the new source for validation and then
returns this error on every path. The design note is a reasoned soundness argument
(recompilation is not byte-stable; heuristic remap into changed bytecode is rejected per §5.11),
and plain resume covers the "same code" case. The bug is the *book* (§8.2) and the mode's
existence in the CLI surface: a documented flagship flow (`shape --resume <hash> script.shape`)
is a guaranteed error. Either land the relocation producer or re-document the mode as
validate-only.

### 9.3 P1 — Snapshot capture can silently substitute `IteratorOpaque` for unsupported closure captures

`snapshot_frame_upvalues_serializable` (`executor/snapshot.rs:1067-1073`): when a closure
capture fails serialization for any reason other than a pending Future, the code writes
`SerializableVMValue::IteratorOpaque` as a sentinel — "Restore will reject this via the
OpaqueOnRestore contract". Consequences: (a) capture reports success and hands out a hash whose
restore will fail — same deferred-loss shape as §9.1; (b) the eventual restore error claims the
user had a live *Iterator*, which is false and unactionable. The barrier philosophy used
everywhere else (refuse at capture with the real reason) should apply here. (Not empirically
driven — top-level closures barrier first; this path needs a closure *frame* on the stack at
capture, i.e. `snapshot()` inside a closure body with an exotic capture.)

### 9.4 P1 — `wire-serve`: PATH-dependent subprocess execution, no auth, no sandbox

Raw-socket transcript (hand-built MessagePack over the framing protocol):

```
→ {"type":"version"}   ← {type:"version", shape_version:"0.3.2", wire_protocol:1}
→ {"type":"execute","code":"print(1 + 2)"}
← {type:"result", success:false, error:"No such file or directory (os error 2)"}   # no `shape` on PATH
# with target/debug prepended to PATH:
← {type:"result", success:true, output:"42\n"}
```

`execute_shape_code` spawns `Command::new("shape")` (`wire_serve_cmd.rs:159`) — whichever
`shape` binary is first on the *server's* PATH, with the server-user's full permissions, no
resource limits, no auth gate, no TLS, and unbounded `msg_len` allocation (§4.4). Every one of
these is solved in `serve`. Recommend deletion or demotion to a hidden dev alias.

### 9.5 P1 — `validate` is parse-only on both servers (false green for a strict-typed language)

```
→ {"type":"validate","code":"let x: int = \"not an int\""}
← {type:"result", success:true, diagnostics:[]}          # wire-serve, reproduced
```

`wire_serve_cmd.rs:171-181` and `serve_cmd.rs:927-946` both call only `shape_ast::parse_program`.
Shape's whole pitch is that this program must not compile; external tools using Validate (the
documented purpose: "external tool integration") get a green light on type-broken code. The
engine's `parse_and_analyze` path used by Execute is right there to reuse.

### 9.6 P2 — Captured-but-not-restored control-flow stacks

`loop_stack`, `timeframe_stack`, `exception_handlers` are serialized into `VmSnapshot` but
dropped at restore (`executor/snapshot.rs:434-446`, documented follow-up). While/range-for
resumes verified working (§2.1.3/4); the risk window is checkpoints taken with a live handler
or loop-stack-dependent construct. Persisting data that restore ignores also inflates snapshots
and creates a false impression of coverage in the wire format.

### 9.7 P2 — Decompression-bomb guard runs post-materialization

`framing.rs:62-72`: `zstd::stream::decode_all(body)` completes (allocating the full decompressed
size) before the `MAX_DECOMPRESSED_SIZE` comparison. A hostile peer can craft a small frame
that decompresses toward memory exhaustion; the 256 MB cap never prevents the allocation.
Mitigations in practice: serve's 256 MB *compressed* inbound cap, loopback-default binds, and
auth/TLS for non-loopback — but the book sells this constant as bomb protection (§8.3). Fix:
`zstd` streaming decode with an output cap.

### 9.8 P2 — `@remote` heterogeneous-parameter refusal (undocumented capability cliff)

Transcript in §2.2.3. Any function mixing element types in its parameter list cannot be
`@remote`-annotated; the error is a compile-time semantic error naming annotation machinery,
not the feature limitation. Book pages show only single-`int` wrappers, so users discover the
cliff by hitting it. (The `remote::call` primitive with explicit args may not share this limit —
`call ok: 42` used a single arg; untested for mixed-arg calls.)

### 9.9 P2 — Store hygiene: blob leak on delete, no read-back integrity check

`delete_snapshot` (`snapshot.rs:294-301`) removes the envelope only; `vm_hash`/`bytecode_hash`
blobs (the heavy artifacts) are orphaned forever — no refcount, no store GC. `get_blob`
(`snapshot.rs:161-169`) trusts filename-addressed content without re-hashing; a bit-flipped or
maliciously swapped blob under `~/.local/share/shape/snapshots/blobs/` deserializes as VM state
via bincode with no integrity failure. For a store whose identity story is "content-addressed",
read-side verification is one `Sha256::digest` away.

### 9.10 P2 — Resume/`remote::execute` output leaks internal WireValue encoding

`{ "String": "RESUMED:43" }` (resume, `script_cmd.rs:435-439` serializes `WireValue` with serde
JSON, exposing the enum tag) and `execute value=Int(6)` (Shape-side HashMap projection of
`ExecuteResponse.value`). The marshal layer upstream is *not* at fault: `wire_conversion::
slot_to_envelope` (via `execution.rs:965`) correctly produces `WireValue::String("RESUMED:43")`;
the CLI then prints the enum's serde encoding instead of rendering the value. Cosmetic, but it
is the first thing every distributed-flow user sees, and the book transcripts show the clean
form.

### 9.11 P2 — Non-loopback serve nodes are pure-only with no configuration escape

`derive_serve_security` clamps any non-loopback bind to `PermissionSet::pure()`
(`serve_cmd.rs:147-152`, "Pure-only until configured") — but no flag exists to configure it.
A production (non-loopback) node therefore cannot run any code needing fs/net/time/random/ffi,
making the documented remote-worker story loopback-only in practice. Fail-closed is right; the
missing grant knob is the gap.

---

## 10. What is done well

1. **Zero-trust receiver pipeline as a single readable sequence** (`remote.rs:1020-1160`): hash
   re-verification → structured missing-dep accumulation (single-round-trip resupply) →
   permission union recomputed from *verified* blobs → receiver-grant gate → runtime fail-closed
   check. Each step names its design clause. The strict-node refusal transcript (§2.2.1) shows
   it working, and the `serve_cmd.rs:2527` test attacks it with an actual escape attempt.

2. **Permissions and call-ABI inside the content hash** (`content_addressed.rs:119-152`).
   Making `required_permissions`, `frame_descriptor`, and `capture_kinds` hash-identity means a
   sender *cannot* claim Pure for FsWrite-demanding bytes and still verify — the security
   property is structural, not procedural. The deliberate exclusions (source_map, capture
   *names*) are documented with rationale (rename ≠ new function).

3. **Barrier-first honesty in capture** (`executor/snapshot.rs:835-890`): five distinct refusal
   classes (foreign frame, no store, pending future, live reference, live closure), each a
   catchable `Err` on the user's `Result`, each with remediation wording, and a test asserting
   jargon never leaks. Programs continue after a refused checkpoint — verified in §2.1.5.

4. **The abort-safe two-pass identity restore** (`RestoreLinkCtx` + LIFO base-share ledger,
   `snapshot.rs:1290-1360`, driven from `executor/snapshot.rs:322-401`): restore failure paths
   release exactly the scaffolding shares — no leak, no double-free — and the
   `gc_phase5_identity_tests` assert refcounts numerically around the round-trip.

5. **`RemoteError`'s pre-send/post-send split** (stdlib `remote.shape:129-163`): `Transport` =
   provably-not-executed (retry-safe) vs `ConnectionLost`/`Timeout` = may-have-executed (never
   auto-retry). Encoding idempotency-safety into the error taxonomy — with per-variant doc
   comments explaining the boundary — is distributed-systems design done right, and
   `transport_send_phase` (`remote.rs:283`) implements the classification at the transport seam.

6. **Version discipline on a non-self-describing format**: `SNAPSHOT_VERSION = 7` with a
   value-equality refuse *before* trusting sub-objects (`snapshot.rs:201-217`), and a doc
   changelog of every version's breaking change (`snapshot.rs:82-115`). The same discipline
   appears in the wire constants (V1 preserved for external tools, V2 additive).

7. **The WF-3F fix note pattern** (`executor/snapshot.rs:339-357`): the sp-shift bug that once
   made interrupt-resume read locals as zero is explained at the fix site with the exact
   mechanism, and a dedicated engine-level regression test exists
   (`lib_tests_parts/interrupt_resume_tests.rs`). My discriminating mid-loop test (§2.1.3) is
   green because of precisely this.

8. **Operational ergonomics**: git-style hash-prefix resolution with clean ambiguity errors;
   exit code 130 contract on interrupt; the printed `Resume with:` hint deliberately using the
   plain form (with an in-code comment explaining why appending the source file would select the
   refusing mode — `script_cmd.rs:471-477`); `SHAPE_SNAPSHOT_STORE` env override shared by all
   subcommands through one resolver.

9. **serve's fail-closed posture derivation** (`serve_cmd.rs:105-190`): sandbox level × bind
   class × ffi opt-in collapsed into one function returning the full envelope
   (grants/scope/limits), with the startup banner printing the effective posture — the operator
   can *see* the security state. Non-loopback refuses to even start without TLS+token.

10. **Structured de-panic of network-reachable code paths**: the linker's malicious-blob error
    variants (`linker.rs:34-62`) and `cache_and_hydrate_call_blobs`'s never-cache-unverified
    rule (`serve_cmd.rs:1194-1201`) show consistent "hostile bytes must reach a structured
    error, never a panic/index-OOB" thinking.

11. **The CLI distributed E2E suite's receiver-side proof pattern** (§7.1): tests don't just
    assert client-visible output — they capture the serve child's stderr and assert what the
    *receiver* did (`blobs=2 foreign_entries=1` for genuine transfer, `CancelCall` +
    `outcome=AcceptedQueued` vs the honest `outcome=AlreadyRunning`/"not preemptible" for
    cancellation semantics), use timing differentials against an awaited control rather than
    bare sleeps, verify snapshot hashes land in the receiver's store and NOT the caller's, and
    drive the server from an independent from-scratch `WireClient` rather than the production
    transport. This is distributed-systems test design of unusually high quality, and it
    directly encodes the §4.x design contract's observable guarantees.

---

## 11. What is done poorly / tech debt

1. **`snapshot.rs` is a 6,133-line single file** containing store, wire schema, two giant
   serializer matches, identity machinery, and five test modules. The two ~500-600-line match
   functions (§3.3) are the change-amplifiers: every new kind touches both plus two duplicate
   kind tables in another crate. This is the file where §9.1 was born.

2. **Duplicated-and-diverged kind table** (§4.1) — the VM copy should be deleted today; the
   divergence (ModuleFunction) is live drift, not risk.

3. **`wire-serve` in its entirety** (§9.4): a second server with none of the first server's
   properties, zero tests, dead CLI flags, PATH-dependent subprocess execution. Its continued
   existence is the single cheapest deletion in the vertical.

4. **Store lifecycle**: no blob GC on snapshot delete, no read-side hash verification, eager
   full deserialization in `list_snapshots` (self-documented as a future bottleneck,
   `snapshot.rs:224-231`). None hard; all unowned.

5. **Deferred-failure captures**: two shapes (StringV2 §9.1, IteratorOpaque sentinel §9.3) where
   capture hands out a hash that restore will reject. The vertical's own barrier philosophy —
   refuse at the earliest honest point — is violated by its serializer edges. A capture-time
   "can this restore?" self-check (or at minimum the round-trip property test) is the debt.

6. **The full-payload fallback trust path** (`remote.rs:1138-1143`): requests without
   content-addressed metadata skip hash verification and load-time permission-union gating.
   Runtime gates still hold, but two trust paths of different strength through one endpoint is
   the kind of asymmetry attackers enumerate. Worth either hard-deprecating full-payload or
   documenting it as the lower-assurance mode in the security chapter.

7. **Unsafe-comment coverage** in the serializer (§3.2): 18/104 sites with local SAFETY
   narration. The contracts exist (stated at module/arm level); per-site breadcrumbs are absent
   exactly where future editors will be moving arms around.

8. **Placeholder metrics on the wire**: `ExecuteResponse.metrics` hardcodes
   `instructions_executed: 0, memory_bytes_peak: 0` (`serve_cmd.rs:905-909`) — shipping a
   metrics struct that lies invites downstream dashboards built on zeros.

9. **Stale narration**: the "legacy NaN-boxed path" comment (`content_addressed.rs:241`) and
   "legacy"-named live functions (§3.4) blur the line the Forbidden-Patterns policy draws
   between deletion-fate vocabulary and living code.

10. **Book drift concentrated on the two flagship promises** (§8.2, §8.4): recompile-and-resume
    and QUIC — precisely the claims an evaluator would test first.

11. **1,318 lines of orphaned modules in shape-runtime** (§3.4): `blob_wire_format.rs` (890) and
    `blob_prefetch.rs` (428) are fully-tested, publicly-exported, and consumed by nothing. The
    former is a second FunctionBlob encoding with weaker default integrity (opt-in hash check)
    and a malformed-input panic path; carrying it unwired means any future consumer inherits
    those flaws silently. Wire them to real consumers or delete them.

---

## 12. Prioritized recommendations

### P0 (correctness of the flagship path; small, high-leverage)

1. **Close the StringV2/DecimalV2 restore gap** (§9.1). Add `(SV::String, NativeKind::StringV2)`
   and `(SV::Decimal, NativeKind::DecimalV2)` arms constructing the v2 carriers in
   `serializable_to_kinded_slot{,_ctx}` (+ the heap-field path if reachable). Effort: hours.
2. **Add the kind-space round-trip property test** (§7.3.1): for every `NativeKind` with a
   representative value, assert `serializable_to_kinded_slot(slot_to_serializable(v), persisted
   kind)` succeeds and round-trips. This converts the whole §5.2 split-brain from
   discipline-dependent to mechanically-enforced. Effort: half a day.
3. **Delete the VM-side `expected_kind_from_serializable` copy**, import the pub runtime one
   (§4.1) — removes live drift including the ModuleFunction divergence. Effort: minutes.

### P1 (feature honesty & attack surface)

4. **Recompile-and-resume**: either land the frame-relocation producer (design §6 Stage 4 —
   weeks) or, immediately, fix `resumability.mdx` to describe the refusal and reframe the CLI
   mode as "validate edit against checkpoint" (hours). The current book/CLI pair guarantees a
   bad first impression.
5. **Retire `wire-serve`** (§9.4): delete or alias to `serve`; if kept for one release, at
   minimum use `std::env::current_exe()`, add the message-size cap, and mark deprecated in
   `--help`. Effort: deletion is hours; hardening is a day.
6. **Make both Validate paths type-check** (§9.5) via `parse_and_analyze`. Effort: a day
   including diagnostics mapping (line/column are already in the wire shape, currently `None`).
7. **Refuse-at-capture for unsupported closure captures** instead of the IteratorOpaque
   sentinel (§9.3). Effort: hours.
8. **Reference serialization scope call**: the v0.3.3 ruling pulled it in; the ratified design
   needs its O1 carrier decision executed or the scope formally re-dispositioned. Surface to
   the strategic owner rather than letting the barrier quietly become permanent. Effort:
   decision + the design's own estimate.

### P2 (hygiene & operations)

9. Streaming zstd decode with an output cap (§9.7); align the three size ceilings (§5.6).
10. Store hygiene: blob refcount/GC on delete + read-side hash verification (§9.9). Effort: a day.
11. Render resume/`remote::execute` values through the existing WireRenderer instead of raw
    serde JSON (§9.10). Effort: hours.
12. Add a `--grant`/config mechanism for non-loopback serve nodes (§9.11) — the documented
    remote-worker story currently only works on loopback. Effort: 1-2 days incl. tests.
13. Restore (or stop persisting) loop/timeframe/exception stacks (§9.6); document the boundary
    in `resumability.mdx` either way. Effort: audit half a day; restore work per follow-up plan.
14. Document the `@remote` homogeneous-args limitation (§9.8) and the native-module import form
    (§8.4); fix the codebase-index line numbers and the CLAUDE.md shape-wire row (§8.6).
15. Fold `serve` real metrics or drop the fields (§11.8). Effort: hours either way.
16. Disposition the orphaned `blob_wire_format.rs` / `blob_prefetch.rs` modules (§3.4, §11.11):
    delete, or wire to a named consumer — and if `blob_wire_format` is kept, make
    `decode_from_bytes` verify the content hash and bound sections by `data.len()`. Effort:
    deletion is minutes; hardening is hours.
17. Add a string-out-of-array (StringV2) binding to one CLI `--resume` E2E (§7.3.2) — the
    cheapest permanent guard for the §9.1 bug class at the user-visible layer, and a natural
    companion to the §12.1/§12.2 fixes. Effort: minutes once §12.1 lands.

---

*End of report. All transcripts reproducible from the scratchpad programs under
`verticals/snapshot-distributed/`; background servers used ports 9702-9707 on loopback and were
terminated at session end.*

