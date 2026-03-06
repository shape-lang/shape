# P8 Streaming Capability Plan

Status: planned  
Last updated: 2026-02-17  
Primary roadmap: `shape/docs/vision/p0-p11-module-capability-roadmap.md` (P8)

## Objective

Introduce a plugin-only streaming capability (`shape.stream`) that is:

- ABI-stable (`shape_abi_v1`)
- runtime-dispatch only (no built-in host connector logic)
- backpressure-aware
- replay/checkpoint capable
- consistent with existing `shape.module` and `shape.datasource` capability loading

This is the pre-tier foundation for event/stream-driven data execution before Tier 1-5 runtime work.

## Invariants

1. No hardcoded source logic in host runtime.
2. Every stream provider is a loadable extension (`.so`) declared in `[[extensions]]`.
3. Capability wiring is single-source-of-truth through manifest + vtable dispatch.
4. Cross-boundary payloads use `shape-wire`; no new VMValue boundary APIs.
5. Failures must preserve spans/diagnostics in host-facing errors.

## Capability Model

Required capability for providers: `shape.module`  
Optional streaming capability: `shape.stream`

`shape.stream` covers:

- open/subscribe
- pull/poll batch retrieval
- credit-based backpressure
- checkpoint/read checkpoint token
- ack/nack
- close

The runtime owns session lifecycle; plugins own source-specific offsets/tokens.

## Runtime Semantics

### Session-oriented flow

1. Resolve provider via existing plugin registry.
2. Open stream session with config + optional resume token.
3. Poll batches under explicit credits/window.
4. Emit rows/events into runtime queue or direct consumer.
5. Commit checkpoints/acks.
6. Close session deterministically.

### Backpressure contract

- Host grants credits (`N` records or bytes budget).
- Plugin must not exceed granted credits.
- Credits replenished only after host consumption/ack.

### Replay/Resume contract

- Plugin returns opaque checkpoint token.
- Runtime persists token in unified lock/artifact flow.
- Reopen with prior token resumes from provider-defined boundary.

## Implementation Plan (ordered patch sets)

### P8.1 Contract scaffolding

- Add `shape.stream` contract constant and capability kind.
- Add stream capability schema types to extension ABI crate.
- Add manifest parsing + validation tests.

Acceptance:
- Loader recognizes `shape.stream` in manifest.
- Invalid/duplicate contracts are rejected with actionable diagnostics.

### P8.2 Loader and registry wiring

- Extend plugin loader to retrieve stream capability vtable.
- Register stream-capable providers in runtime registry.
- Ensure failure behavior mirrors existing data source/output sink paths.

Acceptance:
- Plugin with `shape.stream` is discoverable via runtime registry.
- Plugin without vtable but declared capability fails load deterministically.

### P8.3 Runtime capability wrapper

- Add `plugins/stream_capability.rs` wrapper (typed FFI boundary).
- Centralize input/output wire conversions.
- Remove/avoid duplicated conversion code in call sites.

Acceptance:
- Wrapper unit tests cover open/poll/ack/checkpoint/close call contracts.

### P8.4 Stream session manager

- Add runtime session manager (`stream/session_manager.rs`).
- Track session ids, provider refs, checkpoint state, and credit windows.
- Ensure close on drop and runtime shutdown.

Acceptance:
- Session lifecycle is deterministic and leak-free in tests.

### P8.5 Backpressure mechanics

- Implement host-issued credits and consumption accounting.
- Enforce plugin batch limit against current credits.
- Add bounded buffering policy (configurable).

Acceptance:
- Over-credit delivery is rejected.
- Slow consumer scenarios remain bounded and stable.

### P8.6 Checkpoint persistence integration

- Persist stream checkpoint artifacts via unified lock helpers.
- Reuse shared lock path resolution (project `shape.lock` / standalone `<script>.lock`).
- Add stale/invalid checkpoint diagnostics.

Acceptance:
- Resume after restart uses stored checkpoint token.
- Corrupt checkpoint produces clear diagnostics without hidden fallback.

### P8.7 Snapshot + stream coherence

- Define ordering between runtime snapshots and stream checkpoints.
- Ensure function/global snapshot restore can rehydrate stream sessions safely.
- Document non-goals where provider cannot guarantee exact-once.

Acceptance:
- Snapshot/resume integration test verifies no duplicated or skipped confirmed events.

### P8.8 Event queue integration

- Route stream batches through unified runtime event queue path.
- Normalize event envelope metadata (source, sequence, timestamp, checkpoint tag).
- Keep compatibility with existing output sink routing surface.

Acceptance:
- Stream events and normal events coexist without ordering regressions in queue tests.

### P8.9 Conformance test harness

- Add capability conformance tests for:
  - subscribe/open failure modes
  - poll batching
  - credit enforcement
  - ack/nack behavior
  - checkpoint roundtrip
  - close idempotency

Acceptance:
- Shared conformance suite can validate any stream plugin crate.

### P8.10 Reference plugin

- Provide one reference stream extension (minimal synthetic source).
- Use it for runtime and CLI integration tests.
- Ensure all paths are plugin-only; no host fallback.

Acceptance:
- End-to-end stream script runs with extension declared in frontmatter/`shape.toml`.

### P8.11 Docs + operator UX

- Document stream capability contract in book + vision docs.
- Add operator diagnostics section (common stream errors and fixes).
- Document lock/checkpoint behavior for projects and standalone scripts.

Acceptance:
- Docs match runtime behavior and tests.

### P8.12 Hardening + cleanup gate

- Remove any temporary split paths introduced during migration.
- Enforce no-duplication on stream contract parsing/invocation.
- Add guard checks preventing reintroduction of host-special stream logic.

Acceptance:
- Clean architecture: one loader path, one registry path, one session manager path.

## Conformance Matrix

Minimum regression matrix:

1. Open + poll + close happy path.
2. Backpressure overflow blocked.
3. Checkpoint persists and resumes.
4. Ack retry/idempotency behavior.
5. Plugin load failure with malformed manifest.
6. Mixed runtime (stream + datasource) capability coexistence.

## Out of Scope for P8

- Full relational IR/MLIR lowering (Tier 1+).
- GPU execution planning.
- New query syntax changes.
- Cross-node distributed streaming runtime (documented separately).

## Exit Criteria

P8 is complete when:

1. Stream capability is fully plugin-dispatched.
2. Backpressure + checkpoint semantics are implemented and tested.
3. No host hardcoded stream/source logic remains.
4. Docs and tests are aligned and reproducible.
