# Priority-Spine Design Overview — Consolidated Ratification Sheet (WF-D)

**Status:** RATIFIED 2026-07-05 (user)

## Ratification record (2026-07-05, user)

- **All 54 consolidated defaults in §3 are ADOPTED**, with exactly one override (Q13, below). The recommended defaults are the ruling for the other 53 questions; no per-question inline annotation is needed beyond Q13's.
- **Q13 — OVERRIDDEN to (b):** nonconforming foreign returns (e.g. Python returns a string for declared `Result<int>`) are **class-1 `Err` on the user's declared `Result`** — the same trust class as a foreign exception, catchable via `match`/`?` — NOT `VMError::RuntimeError`. **Bound consequence (answered in text, not dropped):** marshal-arm gaps and genuine contract violations must remain distinguishable from ordinary foreign failures by another means — a structured discriminator is threaded through the Err payload (stable `TypeConformanceError:` message prefix inside the Q5-ratified string payload; upgraded to a typed `kind` field when the Q5 structured-`FfiError` follow-up lands), and true marshal-table gaps (missing arm) remain the compile-time marshalability error / runtime surface-and-stop backstop. Applied to `ffi-rebuild.md` §4.5 / R3 / R7 / probe 6 / OQ10 and the integration doc's receiver error-mapping step.
- **Q15/Q28/Q52 — the recommended asymmetry is CONFIRMED as the ruling:** local `shape run` grants `Ffi` unscoped; `shape serve` defaults `ffi_languages` strict-empty; loopback binds get `sandboxed()` limits + the moderate set, non-loopback binds are Pure-only until `[permissions]` is configured.
- **Q53(b) — GREEN-LIT as WF-2F-adjacent work:** the foreign-function-ref closure-capture typed carrier follow-up proceeds (the v1 refusal stays in force until the typed carrier lands; the carrier = a dedicated serialized arm carrying the entry `content_hash`, rebound receiver-side via the §4.2.0 ordinal↔hash correspondence).
- The textual touches of §4.1–§4.4/§4.6 and the §4.7 clarification were applied to the affected docs in this same ratification pass.

**Covers:** the five WF-D design docs, all drafted, adversarially reviewed (three lenses each), and revised:

| # | Doc | Vertical | Workflow |
|---|-----|----------|----------|
| 1 | `ffi-rebuild.md` | extern C + fn python + fn typescript runtime | WF-2A (+ WF-1D stage 0) |
| 2 | `snapshot-resume.md` | W17 completion: snapshot()/Ctrl+C/resume/state.* | WF-2B |
| 3 | `distributed-function-transfer.md` | @remote / remote::call / blob transfer | WF-2C |
| 4 | `comptime-excellence.md` | introspection contract, marshal root-fix, codegen, showcases | WF-1B |
| 5 | `polyglot-distributed-integration.md` | polyglot × distributed × snapshot composition | WF-2F |

All five bind to CLAUDE.md §Forbidden Patterns, ADR-006 (§2.7.4/§2.7.5/§2.7.7/§2.7.8/§2.7.10/§2.7.11/§2.7.29/§2.7.30), and `docs/runtime-v2-spec.md`. No doc introduces tag-decode machinery, ValueWord-shaped carriers, kind-blind ABIs, or dynamic fallback; every marshal crossing is `KindedSlot`/`NativeKind` per FieldType on the runtime side, with the §2.7.5-sanctioned raw ABI only at the stable extension/wire boundary. **This sheet is the single list the user ratifies against: 54 consolidated open questions (§3), after deduplication of 56 raw questions across the five docs.**

---

## 1. Per-doc digests (ratification-critical decisions in bold)

### 1.1 `ffi-rebuild.md` — foreign-call runtime

Rebuilds the dead foreign-call path (op_call_foreign stub, zeroed native_abi, fatal eager linking, JIT todo!()) on the **modular** extension system: **lazy linking** (declaring a foreign fn is never fatal; link/compile errors surface structured at first call, with opt-in `--eager-link`/`check --link`), **one shared implementation `invoke_foreign_kinded`** called from both interpreter and JIT (J1 refuse-to-JIT, then J2 out-of-line call — divergence impossible by construction, enforced by differential CI), a **per-FieldType typed marshal table** (existing `foreign_marshal.rs` arms kept; scalar Array/HashMap arms pulled into stage 3 so the book gate honestly covers list/dict passing; everything uncovered is a **compile-time marshalability error**, with runtime surface-and-stop only as backstop), a **three-class error channel** (foreign exception → `Err` on the declared `Result<T,string>`; marshal/contract violations → RuntimeError — *per the Q13 ratification override, the nonconforming-foreign-return sub-case moved to class-1 `Err` with the `TypeConformanceError:` discriminator; the remaining class-2 population stays RuntimeError*; link/permission → structured first-call error; panic containment moves **extension-side** into the `language_runtime_plugin!` macro with the ABI 3→4 bump), and a **new 17th `Permission::Ffi`** with `ffi_languages`/`ffi_libraries`/`ffi_symbols` scope constraints checked **before any dlopen** (ELF constructors never run pre-refusal). Vtable gains additive `runtime_descriptor` + `state_model` (STATEFUL_OPAQUE ⇒ foreign frames are snapshot barriers; foreign state never serialized) + reserved tail padding; entry content hash gains `is_async` + `param_names`. **Deterministic mode refuses foreign-bearing programs at LOAD time.** Ratification-critical: the Ffi default-grant posture (OQ13), the error-class split for nonconforming foreign returns (OQ10), the `Result<T,string>` payload (OQ1), the version-out-of-hash split (OQ2), and the v1 decline of cooperative cancellation (OQ11).

### 1.2 `snapshot-resume.md` — checkpoint/resume completion

The stubs' "deleted ValueWord dependency" rationale is **false at HEAD** — the §2.7.7 kinded capture/restore machinery exists; the rebuild is orchestration: **one suspension spine, two consumers** (in-loop consumer for `snapshot()` so the run *continues* with `Snapshot::Hash(id)`; host-boundary consumer for Ctrl+C persist-and-exit with exit 130), both resume entry points real (`resume_snapshot` same-code; `recompile_and_resume` hash-first frame relocation with a **defined mismatch-refusal table** — changed live function = clean refuse). Persistence becomes a **content-addressed CodeManifest** (per-FunctionBlob objects + schema registry + closure layouts + foreign entries + ExtensionReqs + permission union; monolithic hash only as transitional twin through Stage 5) — the concrete form of "polyglot composes with distributed". **Explicit suspension barriers** (foreign frame, non-quiescent async, live Channel/Iterator/Deque/FilterExpr, held Mutex, mid-drop, residual JIT frame) each yield a structured `SnapshotError` with a normative rendered-message catalog; the 2026-05-29 opaque-arm rulings are encoded, not re-litigated. Resource counters/deterministic RNG/permissions all persist and re-verify at resume (limits cumulative — no budget laundering; zero trust in the snapshot's self-declared permissions). JIT: **transitive suspension-point pinning** (statically-reaching chains never tier up) + residual `Barrier{JitFrame}` for indirect calls. Ratification-critical: **`snapshot()` becomes `Result<Snapshot, SnapshotError>`** (OQ8 — the shipped no-error-channel signature would make every barrier an uncatchable abort; nothing working breaks), the blob-graph end state (OQ1), the JIT pinning cost + residual barrier (OQ2), the state-builtin return-ABI migration to `Result<KindedSlot, VMError>` scoped to `state.*` (OQ9), three `SNAPSHOT_VERSION` bumps (Stage 0/2/5), and live Bool-default retirement at `executor/snapshot.rs:483`.

### 1.3 `distributed-function-transfer.md` — per-function transfer

Makes `@remote`/`remote::call` real: **content-addressed minimal blob-closure transfer** (never source shipping), missing blobs a structured protocol event with a bounded retry-once resupply loop, receiver caching by verified hash. **`remote::call` is compiler-elaborated** (positional type-check against the callee's declared params, TypedObject `_0.._n` pack carrier, `R` from the declared return; a plain stdlib signature is not typeable in strict Shape) and registered **per-arity typed** — the Bool-default variadic path is refused, making Stage 1.3 **hard-dependent on WF-1B's marshal fix**. Closures cross with an explicit **`upvalue_kinds` parallel track** (§2.7.8 shape); `capture_kinds` + `frame_descriptor` enter `FunctionBlobHashInput` (call-ABI is identity); a defined refusal matrix (mutable/ref/resource/foreign/nested-closure captures) with variable-name, Shape-surface-syntax messages, and **comptime pre-flight** on `@remote` so statically-knowable refusals fire at compile time. The security model survives the wire: receiver recomputes hashes and the linker union from verified bytes, `load_linked_program_with_permissions` + mandatory fail-closed runtime gating (`granted_permissions: Some(...)` — the runtime gate, not the load gate, is the boundary against dishonest senders). Structured `RemoteError` Shape enum with a **normative pre-send/post-send split** (`Transport` = did-not-execute; `Timeout`/`ConnectionLost` = may-have-executed — what retry annotations branch on). Transport: hostname resolution fixed; **TLS-on-TCP v1**, non-loopback refuses without TLS + auth token. Ratification-critical: the elaboration approach (OQ-9), the one-time hash break (OQ-6, batched doc-set-wide), `__call` → `remote::call` rename (OQ-11), `@remote` failure surface (OQ-1), and the runtime-hook `ctx.target` dependency (OQ-12, delivered by comptime §4.1.5).

### 1.4 `comptime-excellence.md` — introspection contract + root fixes

Fixes the two root causes by **deletion, not machinery**: (A) the Bool-collapse variadic marshal — `TypedInvoke` becomes a `&[KindedSlot]` trait object, the raw-bits flatten at `modules.rs:716` and the Bool stamp at `marshal.rs:2284/2295` are deleted, kinds flow from the §2.7.7 stack track end to end (class-aware fixed-arity checks + kind-directed reads handle the String/StringV2 carrier split); (B) descriptor schema-identity collision — **named, concrete, reserved, deterministically-registered schemas** resolved by name in the consumer's registry (ids never cross registries; the rejected alternative was a translation table, refused on sight). Ships a **frozen v1 introspection contract** (`target` descriptor with `fields`/`params`/`annotations`/`captures`/`doc`, canonical type-rendering table, additive-only evolution, `comptime_api: 1`), **directive type-safety** via a whole-program pass-1.5 (the `set return` SIGSEGV becomes an ordinary compile error; `__original__` becomes a direct typed call), a **normative runtime-hook contract §4.1.5** (`ctx.target` + specialization-time kinded args pack — the deliverable distributed OQ-12 demands), **diagnostics at the Zig/Rust bar** (LSDS routing, spans, comptime traces, jargon firewall), a **v1 generation surface** (source-string emission through the normal parser; new computed `extend (expr)`; no hygiene system, honest rules; expansion-anchored error spans), **comptime purity** (empty permission set; `build_config()` the only environment window; NEW hash-tracked `[build.config]` table + keyed `build_config(key)` via compile-time arity rewrite), and two stdlib showcases (`std::serde` derive, `std::llm` tools/prompts) making the CLAUDE.md LLM-patterns claim true. Ratification-critical: type-target annotation application pulled into WF-1B (OQ1), the `__original__` contract break (OQ6), comptime purity as a language guarantee (OQ8), `[build.config]` (OQ11), duplicate-annotation compile error (OQ12).

### 1.5 `polyglot-distributed-integration.md` — the composition

Zero new value carriers — composition happens at the **metadata layer**. **One identity story**: `ForeignFunctionEntry.content_hash` coverage ratified field by field (IN: language, body, types, param_names, is_async, declared native alias; OUT with named recovery: name (advisory-only), arg_count, dynamic_errors, schema ids, extension version); entries are **hoisted to content-addressed store/wire objects** (§4.2), and `CallForeign` operands are canonicalized to **blob-local ordinals** with a linker remap (fixes the program-level-index soundness hole C10 and makes cross-program blob dedup real). **Extensions are declared node capabilities** (`ExtensionReq{language_id, extension_name, semver_req}` — deliberately no abi_version, no .so hash; matched against `NodeCapabilitySet` from `runtime_descriptor`; refusals are structured `CapabilityUnavailable`/`SnapshotError::CapabilityMissing` with `CapabilitySubKind` and remediation verbs; **no auto-install on network input, ever**). The **foreign-frame snapshot barrier is ratified as a single rule** consumed identically by WF-2A/WF-2B (refuse, never run-to-completion; Ctrl+C composes via the deferred-observation gate, quoted verbatim from snapshot-resume §4.4). `Ffi` is enforced **four times** (compile-hash / link-union / load-gate / per-call), with permission-before-dlopen ordering on the receiver. Every {language × movement} failure cell is a defined matrix row with message templates; foreign-*environment* failures (missing numpy etc.) are the honest bounded residual — surfaced at first call with traceback, diagnosable via ping `backend`, not load-verifiable. Seven **named amendments A1–A7** complete sibling seams (all adopted or bound into WF-2A stage 0). Ratification-critical: the entry hoist + amendments (OQ-2), version matching policy w/ cargo-caret 0.x semantics (OQ-1, paired with ffi OQ2), the no-auto-install guarantee (OQ-4), strict-empty `ffi_languages` serve default (OQ-6), and keeping the foreign-function-ref capture refusal post-WF-2F (OQ-7).

---

## 2. Cross-doc dependency & sequencing picture

### 2.1 The spine

```
WAVE 1
  WF-1D security wiring ──── Permission::Ffi variant, granted-set/ScopeConstraints threading,
        │                    ResourceLimits on serve, check_permission semantics
        ▼
  WF-2A STAGE 0  ═══ THE ONE HASH-STABILIZATION COMMIT (Q1) ═══
        ABI 3→4 bump · vtable tails (runtime_descriptor/state_model) · extension-side
        panic containment · entry-hash += is_async/param_names (ffi §4.7) ·
        A6 foreign_dependencies ordering + CallForeign ordinal rewrite + linker remap ·
        A7 declared-alias storage · A5(ii) __ffi_h{hex16}_return ·
        frame_descriptor + capture_kinds into FunctionBlobHashInput (distributed §4.8)
        → all content hashes final BEFORE any SnapshotStore / RemoteBlobCache populates
        │
  WF-1B comptime (S1 marshal fix ▸ S2 schema identity ▸ S3 directives+codegen ▸ §4.1.5 hook contract)
        │            └── S1 unblocks: WF-2E (json/msgpack/toml/yaml), state::hash (WF-2B),
        │                remote::call registration (WF-2C 1.3), state builtins (WF-2B stage 5)
  WF-1A(c) annotation-hook JIT parity ── gates jit legs of @remote probes + comptime P9
  WF-0B differential harness + book truth-gate recipes ── hard gate infra for ALL stage-2+ merges

WAVE 2 (parallel lanes, ordered interlocks)
  WF-2A ffi stages 1–7          WF-2B snapshot stages 0–6       WF-2C distributed stages 1–4
        │  (stage 3 needs WF-1B?no —   │  (stage 5 needs WF-1B S1;      │  (1.3 needs WF-1B S1;
        │   stage 1 registry, stage 3   │   stage 6 T9(a) needs WF-2A    │   1.4 needs WF-1B §4.1.5
        │   scalar container arms)      │   python fixture)              │   + WF-1A(c) for jit legs)
        └──────────────┬────────────────┴──────────────┬────────────────┘
                       ▼                                ▼
  WF-2F integration F0–F5 (after WF-2A 0–3, WF-2B 0–2, WF-2C 1) — F0 *verifies* the stage-0
        format; F5 = {py, ts, C} × {transfer, snapshot→resume, remote+resume} + book

External: V3-S5 carriers gate ffi stage 7 (non-scalar Array arms) and comptime type_info(T).fields.
```

### 2.2 Which decisions lock which

- **Q1 (hash break)** locks everything: no snapshot store, remote cache, or book example involving content hashes should exist before it lands. Answering "no batch" forces ≥2 invalidation events with populated stores in between — the docs' shared rationale rejects that.
- **Q7/Q22 (snapshot() → Result)** locks the stdlib surface, the entire §4.11 error catalog, the book "Checkpoint & Resume" chapter, and integration's barrier cells (all assume a catchable `Err`).
- **Q16 (CodeManifest blob-graph)** locks recompile-and-resume (Q18 is only possible under it), cross-node resume, and the WF-2C/WF-2F fetch model. Q48 (entry hoist) rides the same manifest and WF-2B's Stage-2 version bump.
- **Q2 (version-out-of-hash + matching policy)** locks `ExtensionReq` semantics in both the manifest (resume) and `RemoteCallRequest` (transfer); flipping to version-in-hash changes cache-churn economics cluster-wide.
- **Q33 (remote::call elaboration)** is existential for the typed primitive — rejecting elaboration rejects typed `remote::call` (not typeable in strict Shape otherwise), which strands `@remote` and the WF-2C book chapter.
- **Q4 (ctx.target)** gates distributed Stage 1.4 (`@remote` end-to-end); comptime §4.1.5 is the deliverable, WF-1B owns it.
- **Q9/Q46 ([build.config])** gates the blessed `@remote(build_config("WORKER_ADDR"))` deployment form (Q34) and the T16 book chapter.
- **Q10 (async foreign)** gates integration's async matrix row (vacuously covered if the answer is compile-error-for-now).
- **Q3 (cancellation decline)** governs three docs at once: ffi §4.12, snapshot §4.4 interrupt no-save behavior, integration's never-returning-call matrix cell.
- **Q15 + Q28 + Q52 (permission posture trio)** must be answered coherently: local-run default (ffi), serve granted set (distributed), serve `ffi_languages` (integration — deliberately stricter than ffi's local semantics). The book's FFI and distributed chapters render differently under each combination.
- **Q45 (required_permissions on function targets)** is the data source for Q28's folded comptime permission-disclosure warning on `@remote` targets.
- **Q53 (foreign-ref capture refusal)** keeps distributed §4.4's refusal row in force after WF-2F — answering "ship it" without the typed-carrier design is the named defection-attractor both docs refuse.

### 2.3 Verified-resolved seams (checked during this consolidation; no action needed)

- `ExtensionReq` shape: snapshot-resume **adopted amendment A2 in full** (no abi_version, no .so hash) — the pre-adoption drift two review lenses flagged is gone; exactly one manifest shape is presented.
- Ctrl+C-during-foreign-call: both snapshot-resume §4.4 and integration §4.5 state **one rule** (deferred flag *observation* while `foreign_reentry_depth > 0`; terminate-now for every other barrier) — the earlier "wait for barrier-free" phrasing is retired everywhere.
- Deterministic × Ffi timing: ffi §4.8.3 (load-time refusal) is consistently consumed by integration (M7, A4(iii)) and ffi's own probe 9(d).
- ffi §4.11.1 foreign-identity wording already carries integration A1's hoist language.

---

## 3. CONSOLIDATED OPEN QUESTIONS FOR THE USER

56 raw questions across the five docs; two true merges (ffi OQ11 ≡ snapshot OQ7; distributed OQ-12 resolved jointly with comptime §4.1.5) and one doc-mandated pairing (ffi OQ2 + integration OQ-1) yield **54 consolidated questions**. Recommended defaults are the docs' own recommendations — ratifying all defaults is a coherent, mutually-consistent set.

### Cluster A — Cross-cutting (answer first; they lock formats and sequencing)

**Q1 — The one-time content-hash / blob-format break.** *(distributed OQ-6; binds integration A5(ii)/A6/A7 and ffi §4.7's entry-hash fix)*
Batch every hash-affecting format change into WF-2A stage 0's Wave-1 commit: `frame_descriptor` + `capture_kinds` into `FunctionBlobHashInput`; `foreign_dependencies` ordered first-use-deduped + `CallForeign` blob-local ordinal rewrite + linker remap (A6); `NativeAbiSpec.library` stores the declared alias (A7); hash-derived `__ffi_h{hex16}_return` schema name (A5(ii)); foreign-entry hash gains `is_async` + `param_names`. Pre-1.0, no persisted stores in the wild. **Options:** one batched invalidation event (rec) vs per-doc events (≥2 breaks with populated stores in between). **Default: approve the batch.**

**Q2 — Extension version: identity vs capability (ratify the pair together).** *(ffi OQ2 + integration OQ-1)*
(a) Content hash covers (language, source, signature, is_async, param_names); extension version stays OUT — a node-capability constraint matched via `runtime_descriptor`. Alternative: fold the extension **major** version into the hash (stronger reproducibility; invalidates all blobs on major upgrades). (b) Matching policy: explicit `semver_req` honored; absent one, same-major as `observed_version` with **cargo-caret semantics for 0.x** (same-minor window — what actually fires today since both extensions are 0.x); else any provider. Alternative: pin-to-observed-exact (reproducible, brittle clusters). **Default: (a) version out of hash, (b) constraint-satisfaction.**

**Q3 — Cooperative cancellation of long-running foreign calls.** *(ffi OQ11 ≡ snapshot OQ7 — one answer governs both docs + integration's hang cell)*
v1 declines the hook: foreign calls are atomic; interrupt/snapshot requests defer to call return; never-returning call ⇒ second Ctrl+C force-exits with nothing saved (documented). Additive `request_cancel` vtable tail fn (best-effort `PyErr_SetInterrupt` / v8 `TerminateExecution`) is the designed follow-up at zero ABI cost (reserved tail padding). **Options:** v1 decline + follow-up (rec) vs require the hook in v1 (extension-side work in ffi stage 3). **Default: decline for v1.**

**Q4 — Runtime-hook typed target accessor `ctx.target`.** *(distributed OQ-12, delivered by comptime §4.1.5 — resolve jointly)*
Distributed asks: (a) WF-1B owns the runtime-hook accessor spec as a named deliverable before its Stage 1.4, (b) `ctx.target` as the surface name. Comptime §4.1.5 now **delivers exactly that spec under that name** (typed function value statically bound in the specialized handler; specialization-time kinded args pack; no `ctx["__impl"] ?? args[0]` fallback). **Options:** ratify §4.1.5 as the deliverable + the name (rec) vs rename/redesign (blocks `@remote`). **Default: ratify both.** (Note: distributed's parenthetical describing comptime's runtime ctx as `{module_path, file, build}` is stale — see conflict §4.1.)

### Cluster B — FFI rebuild (`ffi-rebuild.md`)

**Q5** *(ffi OQ1)* — **`Result<T>` error payload.** `Result<T, string>` for v1 (foreign exception rendered as string), structured `FfiError{kind, message, traceback}` as later additive change — or require the structured object now (costs a builtin schema + extension error_mapping contract in stage 3). **Default: string for v1.**

**Q6** *(ffi OQ3)* — **Deterministic × Ffi.** Hard refusal of foreign-bearing programs under `Deterministic`, at LOAD time; allow-with-attestation only as a possible later vtable capability. **Default: hard refusal.**

**Q7** *(ffi OQ4)* — **C-view/slice annotation spellings.** Lowercase `cview<T>`/`cmut<T>`/`cslice<T>`/`cmut_slice<T>` canonical (book wins); CamelCase forms one-release deprecated aliases with compile warning — or flip the book to CamelCase. **Default: lowercase canonical.**

**Q8** *(ffi OQ5)* — **C-returned `cstring` ownership.** v1 borrowed-copy-never-free (leaks if the C API expected caller-free) — or add an `owned cstring` return annotation (host frees) in stage 2. **Default: borrowed-copy v1.**

**Q9** *(ffi OQ6)* — **Foreign-code containment limits.** "Documented limitation" for extern-C process-fatal crashes and foreign-runtime memory outside MemLimited — or commission process-isolated FFI (subprocess/wasm sandbox) as a designed follow-up. **Default: documented limitation.**

**Q10** *(ffi OQ7)* — **Async foreign functions.** v1: `async fn python/ts` executes on the scheduler's blocking lane completing a Shape future (no vtable change) — or defer entirely with a compile error until a streaming/async vtable revision. Gates integration's async matrix row (vacuous under the compile-error branch). **Default: blocking-lane v1.**

**Q11** *(ffi OQ8)* — **Arrow IPC scope.** Keep `arrow_bridge.rs` out of this rebuild's acceptance gate — or pull a minimal Table-arg path into stage 7. **Default: out.**

**Q12** *(ffi OQ9)* — **JIT stage J2 timing.** Land J1+J2 inside WF-2A as planned — or ship J1 only and move J2 to the post-program JIT-coverage lane (v0.4) if the schedule slips. No correctness difference. **Default: J1+J2 in WF-2A.**

**Q13** *(ffi OQ10)* — **Error class of nonconforming foreign returns.** Python returns `"str"` for declared `Result<int>`: (a) class-2 `VMError::RuntimeError` (rec — declared-type conformance is the boundary contract; keeps marshal-arm gaps distinguishable from foreign failures) or (b) class-1 `Err` on the user's Result (foreign misbehavior in the exception trust class; under (a), a flaky third-party fn kills the program with no `match`/`?` recourse). Bigger user-facing decision than the syntax questions. **Default: (a), escalated rather than decided silently.**
**→ RATIFIED 2026-07-05 WITH OVERRIDE: (b).** Nonconforming foreign returns are class-1 `Err` on the user's declared `Result` (foreign-exception trust class; catchable via `match`/`?`). The distinguishability concern option (a) defended is answered inside the payload instead: the Err string carries the stable `TypeConformanceError:` discriminator prefix (see ffi-rebuild §4.5 as rewritten), and marshal-arm gaps (missing arm) stay the compile-time marshalability error / runtime surface-and-stop backstop — never `Err`.

**Q14** *(ffi OQ12)* — **Stage-3 marshal scope vs the book gate.** Pull minimal scalar-element `Array<T>` + scalar-V `HashMap` arms into stage 3 so the "python/TS book examples green" gate honestly covers list/dict passing — or let the stage-3 gate exclude Array/HashMap examples until stage 7. **Default: pull into stage 3.**

**Q15** *(ffi OQ13; answer with Q28 + Q52)* — **Default local Ffi grant posture.** Plain `shape run` (trusted-local default grants) includes `Ffi` unscoped — FsRead/NetConnect parity; FFI hello-world works out of the box — while sandboxed contexts exclude it unless granted. Or `Ffi` opt-in even locally (every book FFI example needs a `[permissions]` preamble). **Default: include locally, exclude in sandboxes.**

### Cluster C — Snapshot/Resume (`snapshot-resume.md`)

**Q16** *(snapshot OQ1)* — **CodeManifest end state.** Blob-graph persistence (per-FunctionBlob content-addressed objects + manifest); monolithic hash only a transitional twin through Stage 5, dropped Stage 6. Load-bearing for recompile-and-resume, cross-node resume, WF-2C/WF-2F. **Default: blob-graph.**

**Q17** *(snapshot OQ2)* — **JIT frames at snapshot time.** (a) transitive `contains_suspension_point` pinning — statically-reaching call chains never tier up (the never-tier cost, owned explicitly); (b) residual `Barrier{JitFrame}` for `snapshot()` reached only via indirect calls under a hot caller; full-stack deopt-then-capture as follow-up investigation — or make deopt-capture blocking for the vertical. **Default: pinning + residual barrier.**

**Q18** *(snapshot OQ3)* — **Recompile-and-resume for changed LIVE functions.** v1 refuses (hash-identical relocation only). Ship refusal as the durable semantic — or is editing the currently-executing function a required capability (v2 debug-info ip-mapping)? **Default: refusal is durable.**

**Q19** *(snapshot OQ4)* — **Async quiescence.** v1: any non-quiescent async state is a barrier. Follow-up capturing not-yet-started queued tasks (pure-VM thunks as CallPayloads) while refusing in-flight host futures — or snapshot-at-quiescence-only long-term? (Interacts with WF-2D async decision D1.) **Default: quiescence-only v1; follow-up open.**

**Q20** *(snapshot OQ5)* — **Format migration.** v1 refuses older `SNAPSHOT_VERSION` snapshots cleanly. Commit to N-1 read support now — or refuse-and-recompute until the format stabilizes post-v0.4? **Default: refuse-and-recompute.**

**Q21** *(snapshot OQ6)* — **`state::diff`/`state::patch`.** Confirm deferral out of WF-2B (clean-unavailable message; rebuild as its own lane, possibly v0.4 delta-sync) — or pull into v0.3.3. **Default: defer.**

**Q22** *(snapshot OQ8)* — **`snapshot()` contract change.** `pub fn snapshot() -> Snapshot` becomes `Result<Snapshot, SnapshotError>` so barrier refusals are handleable in Shape's Result model instead of uncatchable aborts. No working program breaks (the feature was a dead stub). **Default: ratify the change.**

**Q23** *(snapshot OQ9)* — **State-builtin return-ABI migration.** Stage 5 migrates the `state.*` builtin family's returns from `Result<TypedReturn, String>` to `Result<KindedSlot, VMError>` (§2.7.10 shape), scoped to that module only; a `TypedReturn::Kinded` arm was rejected as parallel-discriminator drift. Ratify the scoped migration and its boundary (raised because the earlier "ConcreteReturn is bulldozed" constraint was ungrounded — this needs user ownership). **Default: scoped migration.**

**Q24** *(snapshot OQ10)* — **Snapshot tooling scope.** `shape snapshot list` + `inspect` land with Stage 3 (the Ctrl+C hash-recovery path); `rm` + `gc` at Stage 6; `label` envelope field reserved at the Stage-2 bump either way. v0.3.3 or fast-follow? **Default: list/inspect Stage 3, rm/gc Stage 6, all v0.3.3.**

**Q25** *(snapshot OQ11)* — **Byte-payload surface for `state::serialize`.** v1 ships `Array<int>` (byte values 0–255), replacing the off-language `Vec<int>` declaration; dedicated `bytes` type as named follow-up — or block `state::serialize` on the `bytes` type. **Default: `Array<int>` v1.**

### Cluster D — Distributed transfer (`distributed-function-transfer.md`)

**Q26** *(distributed OQ-1)* — **`@remote` failure surface.** Transport/remote failures raise as ordinary runtime errors (annotated fn's own `Result<T,E>` passes through); recoverable handling via `remote::call` returning `Result<R, RemoteError>`. Alternative: force `Result<T, RemoteError>` returns on annotated fns. **Default: raise; `remote::call` for recoverable.**

**Q27** *(distributed OQ-2)* — **Mutable captures.** Refuse in v1 with remediation ("pass the value as an argument and return the new value") — or allow with documented send-copy divergence (remote writes lost). **Default: refuse.**

**Q28** *(distributed OQ-3; answer with Q15 + Q52)* — **`shape serve` default granted set + permission-surface UX.** Recommended: loopback bind ⇒ `sandboxed()` limits + moderate set (`Time`, `Random`, `NetConnect`?); non-loopback ⇒ Pure-only until `[permissions]` configured. Decide the exact per-bind-class allowlist and whether a `--grant` CLI flag mirrors shape.toml. Folded in: comptime permission-surface disclosure on `@remote` targets as a `comptime warning` listing transitive `required_permissions` (rec) or silent. **Default: recommended splits + warning; exact allowlist is the open item.**

**Q29** *(distributed OQ-4)* — **Blessed cross-host transport.** TLS-on-TCP (tokio-rustls) v1; QUIC feature-gated; non-loopback refuses without TLS AND requires `--auth-token` (warning → hard refusal). Or QUIC-first / auth-token optional. **Default: TLS-on-TCP + mandatory token.**

**Q30** *(distributed OQ-5)* — **Cache sharing scope.** Process-wide verified-blob + `LinkedProgram` LRU (bounded, permission-epoch keyed) vs per-connection only; any disk persistence (`~/.shape/blob-cache`) now or later? **Default: process-wide, no disk persistence yet.**

**Q31** *(distributed OQ-7)* — **`remote::execute` book positioning.** Retain as a separate source-shipping tool with a "not the distributed-computing path" callout — or deprecate toward `@remote`-only. **Default: retain with callout.**

**Q32** *(distributed OQ-8)* — **Non-idempotent retry policy.** Retries only on `MissingModuleFunction` (provably pre-execution); `Timeout`/`ConnectionLost` (may-have-executed) never auto-retry. Future `@remote(idempotent: true)` opt-in — or userland-annotation territory? **Default: design-side retry only on MissingModuleFunction; idempotent opt-in left open.**

**Q33** *(distributed OQ-9)* — **`remote::call` compiler elaboration.** Ratify the compiler-recognized elaboration (positional type-check, TypedObject `_0.._n` pack carrier, `R` from declared return, specialized-handler path for `__call_raising`); includes verifying (and landing if absent) comptime evaluation in annotation-argument position for `@remote(build_config(...))`. The alternative — a plain stdlib signature — is not typeable in strict Shape, so rejecting elaboration rejects the typed primitive. **Default: ratify.**

**Q34** *(distributed OQ-10)* — **Runtime-expression addresses for `@remote`.** v1: comptime-evaluable only (`build_config` form blessed; runtime addressing via `remote::call`). Should `@remote` ever accept runtime expressions, or is comptime-only permanent? **Default: comptime-only v1; permanence open.**

**Q35** *(distributed OQ-11)* — **Public naming.** Retire `__call` from the public surface; public primitive is `remote::call`; internal sibling keeps the dunder (`__call_raising`). Blast radius: integration doc lines 227/359 take a one-line rename touch on ratification (§4.2 conflict below). **Default: ratify the rename.**

### Cluster E — Comptime (`comptime-excellence.md`)

**Q36** *(comptime OQ1)* — **Type-target annotation application.** Pull `@ann`-on-`type` into WF-1B now (machinery exists; every derive story needs it) — or ship the derive showcase in its clumsy comptime-block fallback form. **Default: pull in.**

**Q37** *(comptime OQ2)* — **`optional` in the v1 contract.** Code builds it, book omits it. Contract includes `optional`; book corrected. **Default: confirm.**

**Q38** *(comptime OQ3)* — **`type_info(T).fields` sequencing.** Contract now + clean SURFACE error until V3-S5 `Array<TypedObject>` carriers land — or hold the `fields` key out of contract v1. **Default: contract now + SURFACE.**

**Q39** *(comptime OQ4)* — **Contract v2 scope.** Roadmap three named v2 items — (i) first-class type values, (ii) typed annotation args (fixes `@default(3)` vs `@default("3")`), (iii) method/impl/trait-conformance introspection — as comptime contract v2, or is the v1 string-composition surface the intended long-term shape? **Default: roadmap as v2.**

**Q40** *(comptime OQ5)* — **`ctx` v1 contract.** `{ module_path, file }` read-only; `build` dropped because `build_config()` is the single build-info surface. Ratify, extend, or keep reserved. **Default: ratify as revised.**

**Q41** *(comptime OQ6)* — **`__original__` convention break.** Replace the never-worked `__original__(args)` array convention with direct typed forwarding across the four book files, no deprecation shim (nothing working to deprecate). **Default: confirm.**

**Q42** *(comptime OQ7)* — **LSP expansion lens placement.** "Show comptime expansion" code lens + annotation hover: bundle into WF-3B ux-polish — or defer post-v0.3.3. **Default: WF-3B.**

**Q43** *(comptime OQ8)* — **Comptime purity as policy.** "Empty permission set; `build_config()` is the only environment window" as a book-documented language guarantee — forecloses future comptime I/O unless explicitly revisited (a later embed-file story would be a dedicated hash-tracked builtin, not general FsRead). **Default: ratify strict stance.**

**Q44** *(comptime OQ9)* — **`std::llm` namespace.** Ratify the namespace and the "no model calls at compile time" positioning. **Default: ratify.**

**Q45** *(comptime OQ10)* — **`required_permissions` contract extension.** Committed-additive `required_permissions: Array<string>` on function targets (delivered with WF-2C, sourced from the linker's transitive union); pre-landing access is an ordinary no-such-field compile error (no sentinel, no empty-array lie). Data source for Q28's disclosure warning. **Default: ratify.**

**Q46** *(comptime OQ11)* — **`[build.config]` + keyed `build_config(key)`.** New hash-tracked shape.toml table (project-file surface, participates in content addressing, NOT env vars); compile-time arity-rewrite resolution to internal `__build_config_key` (Shape has no overloading). Sibling dependency: Q34's blessed `@remote(build_config("WORKER_ADDR"))` form. **Default: ratify.**

**Q47** *(comptime OQ12)* — **Duplicate annotation application = compile error in v1.** Makes `own_args` unambiguous by construction; invocation-index handler context is the v2 alternative if repeated annotations are ever wanted. **Default: ratify.**

### Cluster F — Polyglot × distributed integration (`polyglot-distributed-integration.md`)

**Q48** *(integration OQ-2)* — **`ForeignEntryObject` hoist + one-time format touch (A1–A3).** Entries become content-addressed store/wire objects (canonical narrowed serialization via `foreign_entry_canonical`); `CodeManifest` and `RemoteCallRequest` gain fields (manifest fields ride WF-2B's Stage-2 `SNAPSHOT_VERSION` bump). Ratify the hoist and the three amendments. **Default: ratify.**

**Q49** *(integration OQ-3)* — **`STATEFUL_OPAQUE` resume caveat surface + notice knob.** Book doc + one-time suppressible stderr notice (same channel covers the dormant-C-callback notice, probe M10); knob spelling `--quiet-resume-notices` / serve `resume_notices = "quiet"`. Alternatives: docs-only, or hard opt-in ack flag. Which surface, and ratify the spelling? **Default: notice + knob as spelled.**

**Q50** *(integration OQ-4)* — **No-auto-install stance.** "A receiver never installs/fetches/builds extensions in response to network input; refusal + operator remediation is the protocol" as a hard, book-documented security guarantee (signed-registry-mediated opt-in designable later as its own vertical). **Default: ratify.**

**Q51** *(integration OQ-5)* — **extern C cross-node strictness.** v1 = alias resolvability + dlopen/dlsym symbol verification at load-verify, no platform-triple gate, no library shipping. Is a stricter declared-platform constraint wanted, or is resolution-failure-is-the-signal sufficient? **Default: v1 as designed, no platform gate.**

**Q52** *(integration OQ-6; answer with Q15 + Q28)* — **Default `ffi_languages` for `shape serve`.** Strict empty-by-default (remote foreign execution always a deliberate operator opt-in — deliberately stricter than ffi's local semantics because the caller is the network) vs parity (`Ffi` grant implies all languages unless scoped). **Default: strict empty.**

**Q53** *(integration OQ-7)* — **Foreign-function-ref closure captures.** v1 keeps the refusal row: transferring a closure that captures a foreign function as a VALUE is refused (no SerializableVMValue arm, no §2.7.8 kind-track story, no receiver rebinding rule — shipping undesigned invites an ad-hoc kind-blind encoding). Ratify (a) the v1 refusal and (b) whether the follow-up typed carrier (serialized arm carrying the entry `content_hash`, rebound via the §4.2.0 ordinal↔hash correspondence) is green-lit as WF-2F-adjacent work or deferred indefinitely. **Default: (a) refuse; (b) user's call on the follow-up.**

**Q54** *(integration OQ-8)* — **Declared foreign-environment constraints.** v1 surfaces foreign-body environment failures (missing python packages, deno cache misses, interpreter skew) at first call with full traceback, plus `backend` diagnostics in ping + manifest — but programs cannot declare "needs CPython≥3.12" / "needs numpy≥2" for load-verify matching. Grow `ExtensionReq.backend_req` and/or a per-program foreign-package table (verification mechanism unclear) — or defer with diagnostics + book caveat, revisiting on usage evidence rather than absorbing a package-manager-shaped surface? **Default: defer.**

---

## 4. Conflicts & residual cross-doc inconsistencies

Read against each other at current text, the five docs do **not** semantically contradict one another on any ratified rule — the integration doc's amendment discipline (A1–A7, all adopted or bound) closed the substantive collisions found in review. What remains is one deliberate posture divergence that needs a coherent ruling, four bookkeeping/naming inconsistencies that need textual touches on ratification, and one intra-doc ambiguity. Stated plainly:

### 4.1 Stale premise in distributed OQ-12 / §4.1.2 (superseded by comptime §4.1.5)

`distributed-function-transfer.md` states (§4.1.2, OQ-12) that the runtime-hook `ctx.target` accessor is "specified nowhere today" and that comptime's runtime-hook `ctx` v1 is `{module_path, file, build}`. Both claims are stale against the current `comptime-excellence.md`: its §4.1.5 **now specifies the accessor under exactly the name `ctx.target`** with the specialization-time kinded args pack, and its `ctx` v1 dropped `build` (now `{module_path, file}`, its OQ5). This is convergence, not disagreement — but the distributed doc's text misdescribes its sibling and needs a revision touch when Q4/Q40 are ratified. (Comptime's own cross-doc note flags this.)

### 4.2 `remote::__call` naming residue in the integration doc

`polyglot-distributed-integration.md` §4.4 still titles the sender flow "`remote::__call`" (lines ~227/359), while distributed OQ-11 (Q35) retires `__call` from the public surface in favor of `remote::call`. The distributed doc itself records this blast radius as a one-line rename touch on ratification of Q35. Pending, known, not semantic.

### 4.3 ffi-rebuild §6 stage-0 row understates the ratified stage-0 scope

`ffi-rebuild.md`'s own stage-0 table row lists the ABI bump, vtable tails, panic containment, and its entry-hash fix — but **not** the items its siblings bind into that same commit: A6 (`foreign_dependencies` ordering + ordinal rewrite + linker remap), A7 (declared-alias storage), A5(ii) (schema-name change), and distributed's `frame_descriptor`/`capture_kinds` blob-hash additions (Q1). The sequencing intent is identical across all three docs (one invalidation event, WF-2A stage 0); only the ffi doc's stage-0 enumeration lags. Textual fold-in required on Q1 ratification so the implementing agent of stage 0 has one complete list.

### 4.4 `Timeout` enum-tier misnaming in the integration doc

For the never-returning-foreign-call cell, `polyglot-distributed-integration.md` (§4.6 sandbox paragraph and the §4.9 hang row) says the sender gets **`RemoteErrorKind::Timeout`** — the *wire* enum kind. `distributed-function-transfer.md` §4.2 explicitly **reserves** wire `RemoteErrorKind::Timeout` as *not produced in v1* (the sender-side read timeout is sender-local and never crosses the wire); the correct surface per its normative mapping is the Shape-level **`RemoteError::Timeout { message }`**. The intended behavior is not in dispute (sender-local timeout at the request deadline); the integration doc names the wrong tier's enum. One-line correction on ratification; no design change.

### 4.5 Deliberate posture divergence: local Ffi vs serve `ffi_languages` (Q15 vs Q52)

`ffi-rebuild.md` recommends `Ffi` **unscoped in the trusted-local default grant set** (FsRead parity); `polyglot-distributed-integration.md` recommends `shape serve` hosts **no foreign languages by default** even when `Ffi` is granted — explicitly "stricter than the FFI doc's local semantics because the caller is the network." The integration doc flags this itself; it is a deliberate asymmetry, not an oversight — but Q15/Q28/Q52 must be answered as one coherent posture or the book will teach two contradictory security stories.

### 4.6 `SNAPSHOT_VERSION` bump-count phrasing drift (integration A2/OQ-2 vs snapshot §4.3.3)

Integration A2 and its OQ-2 (Q48) say the manifest fields "ride WF-2B's single planned FORMAT_VERSION bump." `snapshot-resume.md` §4.3.3 has since **withdrawn** the one-bump promise (bincode is non-self-describing) and schedules **three** bumps (Stage 0 upvalue_kinds; Stage 2 envelope/manifest; Stage 5 SV arms) — the manifest fields ride the *Stage-2* bump specifically — and corrected the constant's name to `SNAPSHOT_VERSION` (not FORMAT_VERSION). No semantic conflict (the manifest fields still land in exactly one bump), but the integration doc's phrasing and constant name lag the sibling. Textual touch on Q48 ratification.

### 4.7 Intra-doc ambiguity noted for a clarifying touch (snapshot-resume)

`snapshot-resume.md` §4.1 lists "`Deterministic`-sandboxed evaluation contexts that deliberately forbid persistence" in the `NoStore` opt-out set, while §4.7.2/T11 design and test **deterministic-mode snapshot capture as supported** (RNG stream + virtual clock persisted). The resolving reading is the qualifier — only deterministic contexts *that forbid persistence* opt out; a deterministic sandbox with a store snapshots fine — but the sentence is easy to misread as "Deterministic ⇒ no snapshots," which T11 contradicts. One clarifying sentence recommended; flagged here rather than smoothed over.

---

*End of consolidated ratification sheet. Ratified 2026-07-05 (see the ratification record at the top: 54/54 adopted, Q13 overridden to (b)); the four textual touches in §4.1–§4.4/§4.6 and the clarification in §4.7 were applied to the affected docs in the same pass.*
