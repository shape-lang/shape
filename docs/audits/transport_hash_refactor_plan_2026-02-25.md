# Transport Boundary + Hash-First Identity Refactor Plan

Date: 2026-02-25
Owner: VM/runtime/compiler team
Status: Proposed

## Why this refactor

Two correctness/design risks are active:

1. Function identity is name-based in critical paths (resume + remote minimal blobs), but function names are only module-scope unique.
2. QUIC protocol selection and instantiation are coupled to VM builtin code (stringly-typed IoHandle paths + direct `shape_wire::transport::quic::QuicTransport` construction).

This plan makes function hash the execution identity and moves protocol specifics out of VM dispatch builtins.

---

## Scope

In scope:
- Hash-first execution identity for `state.capture*`, `state.resume*`, and remote call blob resolution.
- Transport boundary cleanup so VM/builtins use protocol-agnostic handles.
- Backward-compatible migration path.
- Test coverage and file-structure cleanup steps.

Out of scope (separate epics):
- Cluster have-set handshake/bloom filter protocol.
- Cross-function JIT ABI specialization.
- Manifest trust policy UX and key distribution.

---

## Track A: Hash-First Execution Identity

### A1. Introduce canonical hash->function lookup in VM

Files:
- `shape/shape-vm/src/executor/mod.rs`

Changes:
- Add VM field:
  - `function_id_by_hash: HashMap<FunctionHash, u16>`
- Populate it in:
  - `load_linked_program(...)` from `LinkedProgram.hash_to_id`
  - `populate_content_addressed_metadata()` for non-linked load path
- Keep `function_name_index` for user ergonomics only, not state restore identity.

Acceptance:
- VM can resolve function id from hash in O(1), independent of function name collisions.

### A2. Make frame identity hash-first in state builtins

Files:
- `shape/shape-vm/src/executor/state_builtins.rs`
- `shape/shape-runtime/src/module_exports.rs`
- `shape/shape-vm/src/executor/vm_state_snapshot.rs`

Changes:
- Frame schema revision (v2) for capture/resume:
  - Required: `blob_hash`
  - Optional/debug: `function_name`, `function_id`
  - Include `upvalues` explicitly (currently omitted)
- `state.capture()` / `state.capture_all()` must emit upvalues + hash identity.
- Keep reading legacy v1 frame shape for compatibility.

Acceptance:
- Closure frame snapshots preserve upvalues.
- Captured state from same hash resumes even when names collide.

### A3. Replace ad-hoc resume reconstruction with shared validated path

Files:
- `shape/shape-vm/src/executor/dispatch.rs`
- `shape/shape-vm/src/executor/snapshot.rs`

Changes:
- Extract shared helper (e.g., `restore_frames_from_state(...)`) used by both:
  - `from_snapshot(...)`
  - `apply_pending_resume(...)`
- Validation rules:
  - If `blob_hash` exists, resolve by hash only.
  - If hash is missing (legacy), allow name-based fallback only when unique; otherwise hard error.
  - Validate hash mismatch as explicit runtime error.
- Resume-frame timing behavior remains current fixed order (set pending before invoke).

Acceptance:
- `state.resume(...)` and snapshot restore have equivalent validation semantics.
- No silent fallback to entry 0 on unresolved function.

### A4. Remote minimal blob lookup by hash (not function name)

Files:
- `shape/shape-vm/src/remote.rs`

Changes:
- Add hash-based APIs:
  - `build_minimal_blobs_by_hash(program, hash)`
  - `program_from_blobs_by_hash(blobs, entry_hash, source)`
- Keep old name-based helpers as wrappers for compatibility, but route through hash when possible.
- Closure remote requests use function_id -> hash vector -> blob closure by hash.
- Request payload migration:
  - Add `function_hash: Option<FunctionHash>` now.
  - In next major, make `function_hash` required, keep `function_name` debug-only.

Acceptance:
- Duplicate names across modules no longer mis-select dependency closure.

---

## Track B: Transport Boundary Cleanup (De-leak QUIC from VM)

### B1. Introduce protocol-agnostic transport handle wrapper

Files:
- `shape/shape-vm/src/executor/builtins/transport_builtins.rs`
- `shape/shape-value/src/heap_value.rs` (if needed for typed custom markers)

Changes:
- Replace string path routing (`transport:quic`, `transport:tcp`) with typed wrappers:
  - `TransportHandle { inner: Arc<dyn shape_wire::transport::Transport> }`
  - `ConnectionHandle { inner: Mutex<Box<dyn shape_wire::transport::Connection>> }`
- Store wrappers in `IoResource::Custom`, dispatch by downcast type only.

Acceptance:
- No protocol decision based on `IoHandle.path` string prefix.

### B2. Move transport construction policy behind a registry/factory

Files (new):
- `shape/shape-vm/src/executor/builtins/transport_factory.rs`

Files (modified):
- `shape/shape-vm/src/executor/builtins/transport_builtins.rs`

Changes:
- Add small factory API:
  - `create_transport(kind: TransportKind, cfg: TransportConfig) -> Arc<dyn Transport>`
- `transport.tcp()` and `transport.quic()` call factory; builtins no longer instantiate `TcpTransport`/`QuicTransport` directly.
- Keep feature-gated QUIC in factory only.

Acceptance:
- VM builtin layer is protocol-agnostic and testable with mock transport.

### B3. Production QUIC configuration path

Files:
- `shape/shape-wire/src/transport/quic.rs`
- `shape/shape-vm/src/executor/builtins/transport_factory.rs`

Changes:
- Keep `new_self_signed()` for local/dev/testing, but add explicit production constructors:
  - `with_client_config(...)`
  - `with_trust_anchors(...)`
  - optional `server_name` input (not fixed `"localhost"`)
- Factory chooses constructor via explicit config, not implicit defaults.

Acceptance:
- QUIC can be configured for real trust roots and stable peer identity.
- Dev-mode self-signed path is explicit and isolated.

### B4. Runtime boundary statement

Boundary target:
- `shape-wire`: owns transport traits + protocol implementations.
- `shape-vm`: owns Shape-facing builtin adaptation only.
- `shape-runtime`: no transport orchestration coupling (wire types/codec usage is fine).

Acceptance:
- VM builtins contain no direct QUIC protocol setup logic.

---

## Backward Compatibility Plan

Phase 1 (non-breaking):
- Accept both legacy and hash-first frame schemas.
- Keep name-based remote APIs but internally prefer hash when available.
- Keep existing transport function names/signatures.

Phase 2 (deprecation warnings):
- Warn when resume payload lacks `blob_hash`.
- Warn when remote call chooses function by name without hash.

Phase 3 (breaking window):
- Require hash identity in resume and remote execution core paths.

---

## Test Plan (must-add)

### Identity / Resume
- Duplicate function names in different modules:
  - capture frame in module A fn `foo`
  - ensure resume resolves A hash, never B name-match.
- Closure with mutable upvalue:
  - capture -> resume and verify upvalue value preserved.
- Legacy snapshot payload without hash:
  - unique name path succeeds
  - ambiguous name path fails with explicit error.

### Remote
- Closure minimal blobs by hash with same-name sibling function present.
- `program_from_blobs_by_hash` fails fast when entry hash missing.

### Transport
- Builtin `transport.connect/send/recv/close` works for both TCP and QUIC through typed wrappers.
- No logic branch depends on `IoHandle.path` prefix.
- QUIC production config constructor unit tests (trust roots + server name).

### Regression
- Existing passing suites:
  - `cargo test -p shape-wire --features quic`
  - `cargo test -p shape-vm transport_builtins --features quic`
  - `cargo test -p shape-vm state_builtins`
  - `cargo test -p shape-vm remote`

---

## DRY + File Structure Refactor (<=800-line target)

Current hotspots:
- `transport_builtins.rs` (1085)
- `state_builtins.rs` (1450)
- `dispatch.rs` (827)
- `module_loader/mod.rs` (1319)

Refactor slices:

1. `transport_builtins.rs`
- Split into:
  - `transport/api.rs` (module exports + schemas)
  - `transport/handle.rs` (typed wrappers/downcasts)
  - `transport/send_connect.rs`
  - `transport/connection_ops.rs`
  - `transport/memoized.rs`
  - `transport/tests.rs`

2. `state_builtins.rs`
- Split into:
  - `state/api.rs` (exports + type schemas)
  - `state/hash_serialize.rs`
  - `state/diff_patch.rs`
  - `state/capture_resume.rs`
  - `state/introspection.rs`
  - `state/tests.rs`

3. `dispatch.rs`
- Split resume logic into `dispatch/resume.rs`
- Keep dispatch loop only in `dispatch/loop.rs`

4. `module_loader/mod.rs`
- Move content-addressed loader path to `module_loader/content_addressed.rs`
- Keep resolver/path logic in `module_loader/resolution.rs`

Target: no single file > 800 lines in modified areas.

---

## Implementation Order (recommended)

1. A1 + A4 (hash maps + remote hash API)
2. A2 + A3 (state schema + shared restore)
3. B1 + B2 (typed transport wrappers + factory)
4. B3 (production QUIC config)
5. Structure split for large files
6. Compatibility warnings + docs

---

## Rollback Strategy

- Keep compatibility parser for legacy state payloads.
- Keep name-based remote entry as fallback during transition.
- Feature-gate new transport factory path behind `transport_v2` until stable, then flip default.

---

## Done Criteria

- Resume/remote identity never depends on non-unique function names.
- QUIC/TCP selection is typed/factory-driven; no path-prefix dispatch.
- Closure upvalues preserved through capture/resume.
- New tests cover collisions + upvalues + QUIC wrappers.
- Touched files in these areas are <= 800 lines.
