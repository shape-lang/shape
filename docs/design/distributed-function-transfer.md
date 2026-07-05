# Design: Per-Function Distributed Transfer Done Right

**Status:** RATIFIED 2026-07-05 (user) — all recommended defaults adopted (overview §3 Q26–Q35 + Q1/Q4); no override touches this doc; OQ-11/OQ-12 rulings recorded inline in §8. See `00-priority-spine-overview.md` §Ratification record.
**Implements against:** WF-2C `remote-per-function-transfer` (+ WF-1D permission plumbing, WF-2F polyglot×distributed follow-on)
**Audit basis:** `docs/cluster-audits/audit-2026-07-04-claimed-vs-real.md` §Q3, §4.6, rec 8
**Binding constraints:** `CLAUDE.md` §Forbidden Patterns, `docs/adr/006-value-and-memory-model.md` §2.7.4/§2.7.5/§2.7.7/§2.7.8, `docs/runtime-v2-spec.md`

---

## 1. Goals & non-goals

### Goals

1. **A working user path.** `@remote("host:port")` on a function makes calls to it execute on a remote `shape serve` node, end-to-end, with typed arguments and a typed return value. `remote::call` (public name; the HEAD stdlib's `__call` dunder is retired from the public surface — §4.1.1, OQ-11) is the low-level primitive underneath it, with a real (non-stub) implementation and a signature that matches between stdlib and native registration.
2. **Content-addressed transfer, not source shipping.** The unit of transfer is the minimal transitive closure of `FunctionBlob`s (already computed by `build_minimal_blobs_by_hash`, `remote.rs:454-489`), cached receiver-side by content hash, negotiated instead of blindly re-sent.
3. **Missing blobs are a protocol event, not a panic.** `RemoteErrorKind::MissingModuleFunction` (declared at `remote.rs:125`, never constructed) becomes the structured signal; the sender reacts by supplying the missing blobs and retrying.
4. **The three-tier security model survives the wire.** Permissions baked into the content hash → linker union → **receiver-side** load-time check via `load_linked_program_with_permissions` against a **receiver-configured** granted set → **fail-closed runtime gating** (`check_permission` with `granted_permissions: Some(server_granted)` mandatory on the remote path — `None` is forbidden there because `check_permission` is fail-open when `None`, `module_exports.rs:150-163`). The load-time check is fast-fail UX for honest senders; the runtime gate is the security boundary against dishonest ones (§4.6). Remote execution runs under `ResourceLimits`.
5. **Closures cross the wire with their kind track, or are refused with a precise error.** Per-capture `NativeKind` metadata is serialized alongside capture values (ADR-006 §2.7.8/Q10 shape); nothing is ever re-derived from raw bits.
6. **Hostnames work.** `localhost:9527` and `worker.internal:9527` resolve; the book's examples run as written.
7. **The missing tests exist**: permission-in-hash, permission-union, receiver-enforcement, hash-tamper.

### Non-goals

- **Cross-node reference coherence.** The send-copy model stands (module doc, `remote.rs:21-26`): captures and arguments are value copies; remote mutation is not reflected back. (Consistent with the 2026-05-29 reference-serialization ruling: whole-VM snapshot preserves `&mut` exclusivity, so cross-node coherence is not needed.)
- **Foreign-function (polyglot) blobs over the wire.** That is WF-2F territory, designed in `docs/design/polyglot-distributed-integration.md`. This doc reserves the seams (§4.9) but does not design extension resolution.
- **A scheduler / placement layer.** `@remote(addr)` takes an explicit address. Load balancing, retry policies, and service discovery are stdlib/userland (annotations + comptime make this user-buildable later — that is the point of the annotation design).
- **Streaming / long-lived remote sessions.** One request → one response. `for await` over remote streams is future work.
- **Replacing `remote::execute`.** Source-shipping execution stays as an explicitly separate, documented tool (§4.8).

---

## 2. Current state (file:line grounded)

### What works (library level)

- **`FunctionBlob`** (`crates/shape-vm/src/bytecode/content_addressed.rs:33-92`): self-contained unit — metadata (name/arity/param_names/locals/is_closure/captures_count/is_async/ref flags/mutable_captures/`frame_descriptor`), code (instructions/constants/strings), `required_permissions`, `dependencies: Vec<FunctionHash>`, `type_schemas`, `foreign_dependencies`, `source_map`.
- **Permissions are in the hash**: `FunctionBlobHashInput` (`content_addressed.rs:96-117`) includes sorted `required_permission_names` (line 114); `compute_hash` (122-153) is SHA-256 over deterministic rmp-serde bytes. NOT hashed today: `frame_descriptor`, `source_map`, `callee_names`.
- **Minimal closure**: `build_minimal_blobs_by_hash` (`remote.rs:454-489`) BFS over `blob.dependencies`. Name-based wrapper returns `None` on ambiguity (`remote.rs:493-516`), which `build_call_request` silently converts into full-program shipping (`remote.rs:1065-1073`).
- **Linker permission union**: `linker.rs:327-329` folds all blobs' `required_permissions` into `LinkedProgram.total_required_permissions`. Compiler derives per-function permissions from capability tags with transitive caller-inherits-callee propagation (`compiler_impl_initialization.rs:276, 475-501`).
- **Receiver marshal is the correct ADR-006 template**: `run_remote_call` (`remote.rs:694-897`) reads per-arg `NativeKind` from the callee's `frame_descriptor.slots` (structured error if absent and arity > 0, per §2.7.5.1 / §0.A.iv supervisor ruling, `remote.rs:652-655`), materializes args via `serializable_to_slot` → `KindedSlot`, dispatches `execute_function_by_id`, cross-checks the returned kind against `abi_return_kind()`, projects out via `slot_to_serializable`. `test_remote_function_call_over_tcp` (`serve_cmd.rs:1110-1145`) passes.
- **Permission-checked loaders exist and are tested**: `load_program_with_permissions` / `load_linked_program_with_permissions` (`executor/vm_impl/program.rs:357-400`) — **zero call sites** on the remote path.

### What is broken

| # | Break | Evidence |
|---|-------|----------|
| B1 | Stdlib/native arity split-brain: stdlib declares `__call(addr, fn_ref, args)` (3 params, `remote.shape:81`); native registers `(addr, fn_name)` (2 params, `remote_builtins.rs:318-341`); body is unconditional surface-and-stop `Err`. `@remote` (remote.shape:98-105) is dead end-to-end. | audit §4.6 CONFIRMED |
| B2 | Receiver rejects all closure calls: `request.upvalues.is_some()` → structured error (`remote.rs:731-739`); wire format has `upvalues: Option<Vec<SerializableVMValue>>` but **no per-capture kind track**. | recon §4a |
| B3 | Receiver never enforces permissions: `remote.rs:743` uses plain `vm.load_program(program)` on `VMConfig::default()` (`remote.rs:742`) — no granted set, no `ResourceLimits`, no sandbox. `serve_cmd.rs:430` discards `config.sandbox`. | audit rows "Load-time permission check dead code" |
| B4 | Missing dependency blob **panics** the handler: `load_program` panics on link failure (`program.rs:9-15`); `LinkError::MissingBlob` (`linker.rs:24-25`) is never mapped; `RemoteErrorKind::MissingModuleFunction` (`remote.rs:125`) never constructed. | recon §4c/§4d |
| B5 | Negotiation/sidecar dead on receive: `handle_call(req, _state, ...)` ignores `ConnectionState` (`serve_cmd.rs:735`); `blob_cache` + `pending_sidecars` (`serve_cmd.rs:63-64, 353-357`) populated but never consulted; a negotiated (stripped) request panics at link. `program_hash` field shipped, receiver has no cache keyed by it. | recon §4e |
| B6 | TLS accepted-then-discarded: non-loopback bind requires `--tls-cert/--tls-key` (`serve_cmd.rs:91-99`) but then `let _ = (tls_cert, tls_key); // TLS support is a future enhancement` (`serve_cmd.rs:112`) — plaintext TCP despite certs. QUIC exists but is unreachable (factory uses `TransportKind::Tcp` only, `remote_builtins.rs:104-106`) and `new_self_signed` trusts a throwaway localhost cert (`quic.rs:112-117`). | recon §5 |
| B7 | Hostnames rejected: TCP transport does `destination.parse::<SocketAddr>()` (`tcp.rs:41-43, 57-59`) — `localhost:9527` fails with "invalid socket address syntax"; every book example uses `localhost`. | audit §72 |
| B8 | `frame_descriptor` is load-bearing for marshal but excluded from the content hash — a tampered/divergent descriptor is undetectable by hash. | recon constraint (7) |
| B9 | Polymorphic returns degrade: `wire_to_json_value` (`remote_builtins.rs:131-184`) maps Table/Range/FunctionRef/Content/PrintResult to placeholder strings. | recon §6 |
| B10 | `CURRENT_PROGRAM` thread-local plumbing (`remote_builtins.rs:69-89`) is `#[allow(dead_code)]` — and clones the entire `BytecodeProgram` per set. | recon §3 |

---

## 3. Constraints (binding, quoted)

1. **CLAUDE.md §Forbidden Patterns.** No `ValueWord` at runtime ("Do not reintroduce as a 'shim', 'bridge', 'compatibility layer', or 'serialization helper'. Snapshot/wire uses per-slot kind metadata."). No generic opcodes, no `Convert<X>To<Y>` opcodes to paper over kind gaps, no `SlotKind::Dynamic`, no dynamic-fallback handlers, no feature flags around dynamic dispatch. The broader-family regex `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` is refused on sight — this design names machinery by what it does (serializer, cache, negotiation) and never introduces a boundary translation layer.
2. **ADR-006 §2.7.7 / Q9:** "Snapshot/wire serialization uses parallel `Vec<u64>` data + `Vec<NativeKind>` kinds." Forbidden: `Option<NativeKind>` / `Unknown` placeholders; Bool-default kinds.
3. **ADR-006 §2.7.5.1** (via `FrameDescriptor`, `type_tracking.rs:168-187`): "Every slot's kind is proven at `FunctionBlob` construction time … `NativeKind::Unknown` was deleted from the enum and is never re-introduced under any name." Frame-descriptor absence on a remote call = structured error, never silent degrade (supervisor ruling §0.A.iv, quoted at `remote.rs:652-655`).
4. **ADR-006 §2.7.8 / Q10:** the parallel-kind-track invariant "extends to every cell-storage struct that holds raw heap-pointer bits" — closure cells carry `Vec<NativeKind>` lockstep with `Vec<u64>`. A closure that crosses the wire must carry that track explicitly; "no Bool-default for `Load*Ptr` (surface-and-stop with `NotImplemented(SURFACE)` instead)".
5. **ADR-006 §2.7.5 cross-crate ABI policy:** "stable ABI surfaces (extension contracts, **persisted formats**, FFI handoffs to non-Rust callers) stay on raw bits + parallel `NativeKind`. Internal Rust dispatch … uses `KindedSlot`." The wire format is a persisted format: it carries self-describing `SerializableVMValue` payloads plus explicit `NativeKind` metadata where cell-shaped (captures); `KindedSlot` itself never serializes.
6. **runtime-v2-spec:** `KindedSlot` must not leak into the typed VM↔JIT slot ABI. The remote boundary is a host-tier boundary (GENERIC_CARRIER site) — `KindedSlot` use in `run_remote_call` is correct and stays.
7. **Permission-in-hash identity is preserved.** `required_permission_names` stays in `FunctionBlobHashInput` (`content_addressed.rs:114`); nothing in this design moves permissions out of the hash.
8. **No panics on the receiver path.** `load_program`'s panic (`program.rs:9-15`) must not be reachable from network input.
9. **Read-only recon; this doc lives in `docs/design/` and changes no code.**

---

## 4. Design

### 4.0 Overview — one diagram

```
 SENDER (client VM)                                RECEIVER (shape serve)
 ──────────────────                                ─────────────────────
 @remote("worker:9527")                            ConnectionState
 fn compute(xs: Array<int>) -> int                   ├─ authenticated: bool
        │  call site: compute([1,2,3])               ├─ blob_cache: RemoteBlobCache   (content-hash keyed)
        ▼                                            └─ program_cache: LinkedProgramCache (NEW, §4.5)
 annotation before-hook (specialized per site)
        │  remote::call(addr, compute, args…)
        ▼
 call builtin (VM-aware, &[KindedSlot] ABI)        WireMessage::Call(RemoteCallRequest)
   1. resolve fn_ref → function_id → content hash          │
   2. blobs = minimal transitive closure                   ▼
   3. args: slot_to_serializable(bits, kind, store)  handle_call(req, state, runtimes, limits)
      (kind from the KindedSlot the dispatch shell     1. merge sidecars + cached blobs
       built from the §2.7.7 stack kind track)         2. verify each blob: recompute content hash
   4. captures (if closure): values + kinds from       3. link → LinkError::MissingBlob ⇒
      the §2.7.8 closure-cell kind track                    CallResponse(Err{kind: MissingModuleFunction,
   5. negotiate (BlobNegotiation) → strip known             missing_blobs: [hashes]})   ── no panic
   6. send Call                                        4. load_linked_program_with_permissions(
        │                                                   linked, &server_granted)  ⇒ deny = structured Err
        │   ◄── CallResponse(Err MissingModuleFunction)5. VM with ResourceLimits from serve config
        │   resend with missing blobs, retry once      6. run_remote_call marshal (UNCHANGED template):
        ▼                                                 frame_descriptor kinds → serializable_to_slot
 result: SerializableVMValue → KindedSlot                 → KindedSlot args → execute_function_by_id
   materialized at the callee's declared                  → return-kind cross-check → slot_to_serializable
   return kind (frame_descriptor cross-check)          7. CallResponse(Ok value)
```

Design rule of thumb: **the receiver marshal (`run_remote_call`) is already right — this design changes what surrounds it** (user surface, blob logistics, permissions, transport), not the marshal protocol itself.

### 4.1 User-facing surface

#### 4.1.1 `remote::call` — the primitive, fixed

**Naming.** The public primitive is `remote::call`. The HEAD stdlib's `__call` (`remote.shape:81`, B1) is retired from the public surface: the `__` prefix is this codebase's convention for internal/compiler-generated machinery (`__intrinsic_*`, `__into_*`), and this API is explicitly *documented user surface* — users call it directly for recoverable transport errors (§4.1.2) and T7 tests it as such. A discoverable public API must not read as private. The compiler-elaborated sibling `__call_raising` (§4.1.2) keeps the dunder because it is internal-only (compiler-generated from annotation expansion, callable by generated code like `__into_*`, never documented as user API). Ratified 2026-07-05: OQ-11 — the rename's recorded cross-doc blast radius (`polyglot-distributed-integration.md`'s residual `remote::__call`/`__call` sender-flow references) was applied in the same ratification pass.

**Surface form** (what users write; see "typing mechanism" below for why this is compiler-elaborated, not a plain stdlib signature):

```shape
// stdlib-src/core/remote.shape — declaration is compiler-known (like `x as int` → __into_*)
pub builtin fn call<R>(addr: string, fn_ref, args) -> Result<R, RemoteError>;
```

- **Typing mechanism (compiler-elaborated arg pack).** The previous draft's `fn(..) -> R` / `Array<_>` signature was not typeable in strict Shape: there is no wildcard function-type in the grammar (`shape.pest:897-899` — `function_type` requires a concrete param list), bare unparameterized generic names are invalid everywhere (2026-05-31 ruling), and a heterogeneous pack `{int, number, string, bool, Array<int>}` (exactly T6) inhabits no `Array<T>` since `int`/`number` don't unify. Therefore `remote::call` is **compiler-recognized** (the same special-casing class as `as`-casts → `__into_*`): at each call site the compiler (a) resolves `fn_ref` to a concrete function/closure **type**, (b) type-checks `args` **positionally against `fn_ref`'s declared param types** — a compile error on arity or type mismatch, before any network I/O, (c) lowers the pack to a **TypedObject pack carrier with positional fields `_0.._n`** whose per-field types are the declared param types (per-element kinds come from the pack's registered schema — compiler-proven, never re-derived), and (d) instantiates `R` from `fn_ref`'s declared return type. `args` may also be written as an existing tuple value of matching type — tuples at HEAD already materialize as `_0`/`_n` TypedObjects, so that is the *same* carrier. Grammar/elaboration details (including the specialized-handler path, §4.1.2): OQ-9.
  - **Why TypedObject, not a "tuple heap value":** tuples are **not** a first-class heap value at HEAD — there is no `HeapKind::Tuple` and no `Expr::Tuple` literal; the supervisor D1 binding (`crates/shape-vm/src/executor/objects/array_basic.rs:283-298`, 2026-05-24) fixes TypedObject `_0`/`_1` as the tuple carrier (the zip/`entries()` precedent) and names general `TypedArray<TupleN>` as the v0.4 ADR-006 §2.7.24 typed-carrier-monomorphization workstream. This design does **not** depend on that promotion; when it lands, the lowering migrates mechanically (same positional schema). Minting a new `HeapKind::Tuple` for this design alone is refused — it implicates the 4-table HeapKind lockstep rule and ADR-005 §1 single-discriminator discipline for zero marginal benefit over the existing carrier.
- **Compile-time pre-flight rides on elaboration.** Because elaboration always resolves `fn_ref`'s type statically, the **type-level** rows of the §4.4 refusal matrix (unserializable param/return types; foreign-dependency refusal until WF-2F) run at **compile time for every `remote::call` site**, not just for `@remote` (§4.1.2b). **Capture-level** rows (mutable captures, `&`/`&mut` captures, resource captures) are also compile-time when `fn_ref` names a declaration; only when `fn_ref` is a runtime closure *value* (a variable of function type — the type is static, the callee identity is not) do capture checks fall to call time (§4.4). This closes the asymmetry where the annotation path was legible at compile time and the primitive was not.
- **Native registration is per-arity, not variadic.** After elaboration the native arity is fixed at 3 (`addr`, `fn_ref`, arg-pack), so per ADR-006 §2.7.4's own preference ("per-arity is preferred when the function arity is fixed") registration uses the per-arity typed path (`register_typed_fn_3`-class) with declared `NativeKind`s — **not** the variadic registration path. Post-elaboration param kinds: `addr` is `String`-class and the arg pack is an ordinary `Ptr(HeapKind::TypedObject)` whose schema the compiler registered at elaboration — both already declarable in the marshal param universe. `fn_ref` is **not**: marshal's `FromSlot` universe today (18 impls, `crates/shape-runtime/src/marshal.rs` — i64/f64/bool/`Arc<String>`/TypedObjectPtr/DataTable/IoHandleData/Vec forms) has no receiver for a `Ptr(HeapKind::Closure)`/function-ref slot, so the native-implementation stage adds **one new typed `FromSlot` carrier** for function-ref params — a declared-kind carrier per ADR-006 §2.7.4 (kind declared at registration, never probed from bits; not a forbidden shape), whose impl resolves the heap pointer to `function_id` → content hash. This is new marshal surface, sized into §6 Stage 1.3 explicitly (the previous draft's "no new marshal carrier type is required" was wrong for `fn_ref`). The per-arity choice is load-bearing: at HEAD the variadic path is a Bool-default forbidden shape (`register_typed_function` stamps `arg_kinds` all-`NativeKind::Bool` and wraps raw u64 bits as `KindedSlot::new(ValueSlot::from_raw(bits), NativeKind::Bool)`, `crates/shape-runtime/src/marshal.rs:2284-2300`), and module-fn dispatch flattens the caller's true `KindedSlot`s to raw u64 one call earlier (`crates/shape-vm/src/executor/vm_impl/modules.rs:715-716`, per `comptime-excellence.md` §2.2 root cause A). The previous draft's citation of "§2.7.10 dispatch shell" was wrong — §2.7.10 governs **method** dispatch (`MethodFnV2`), not module-builtin dispatch. The actual seam is the module-builtin typed-dispatch path, and **the native-implementation stage (§6 Stage 1.3) hard-depends on WF-1B's variadic/module-marshal kind-threading fix** (`comptime-excellence.md` §2.2-A) so that per-arity dispatch receives true kinds end-to-end; landing the builtin before that fix would route the new primitive through an existing Bool-collapse path, which is refused.
- Slot kinds at dispatch: `args[0]` = addr, `NativeKind::String` (the dedicated `Arc<String>`-pointer variant; `StringV2` accepted where the v2-raw producer migration has landed — the dispatch shell passes whichever kind the §2.7.7 stack kind track holds, never a fabricated one); `args[1]` = fn_ref (`Ptr(HeapKind::Closure)` / function-ref carrier); `args[2]` = arg pack (`Ptr(HeapKind::TypedObject)`, per-field kinds from its schema).
- **Program/blob access — the `ModuleContext` seam, named.** Builtins receive `ModuleContext` (`crates/shape-runtime/src/module_exports.rs:104`), **not** `ExecutionContext` (the previous draft misnamed this; `context/mod.rs:43` is the legacy tree-walk context with no program access), and `ModuleContext` today exposes only `function_hashes: Option<&[Option<[u8;32]>]>` — deliberately raw-bytes-shaped to avoid a shape-runtime → shape-vm dependency. `remote::call` needs the content store (`build_minimal_blobs_by_hash` + §4.3-5 retry resupply), so `ModuleContext` gains a VM-provided callback field that respects the same crate firewall:
  ```rust
  /// Serialized minimal transitive blob closure for an entry hash, plus
  /// direct lookup of individual blobs (retry-once resupply, §4.3-5).
  /// Bytes are rmp-serialized FunctionBlobs; shape-runtime never sees the type.
  pub content_blobs: Option<&'a dyn ContentBlobSupplier>,
  // trait ContentBlobSupplier {
  //   fn minimal_closure(&self, entry: &[u8;32]) -> Result<Vec<([u8;32], Vec<u8>)>, String>;
  //   fn blob_bytes(&self, hash: &[u8;32]) -> Option<Vec<u8>>;
  // }
  ```
  The VM implements the trait over `ContentAddressedProgram.function_store` (where `build_minimal_blobs_by_hash` already lives, `remote.rs:454`); serialization happens VM-side. **Not** via the `CURRENT_PROGRAM` thread-local (deleted; it clones the whole `BytecodeProgram` per dispatch, B10).
- **Return typing:** `R` is instantiated from `fn_ref`'s declared return type at elaboration time (see above). The receiver's return-kind cross-check (`remote.rs:877-888`) plus sender-side materialization at the expected kind make this sound: if the remote returns a kind that disagrees with `fn_ref`'s declared return, `remote::call` returns `Err`, it does not reinterpret.
- **Structured errors, not strings.** `remote::call` returns `Result<R, RemoteError>` where `RemoteError` is a Shape stdlib enum:

  ```shape
  enum RemoteError {
      Transport { message: string },              // PRE-SEND failure: connect refused / DNS / send
                                                  //   failure / payload > cap — the request was never
                                                  //   fully sent; the call did NOT execute
      ConnectionLost { message: string },         // connection closed/reset AFTER the request was fully
                                                  //   sent, before a reply arrived — the call MAY
                                                  //   have executed
      Timeout { message: string },                // sender-side read timeout — the call MAY have executed
      PermissionDenied { missing: Array<string> },
      AuthRequired { message: string },           // token missing/rejected — a distinct, user-actionable
                                                  //   class (pass/refresh a token), never folded into
                                                  //   Protocol
      MissingFunction { name: string },
      ResourceLimitExceeded { limit: string },    // incl. receiver wall-time overrun (limit: "wall_time")
      VersionSkew { server: int, client: int },
      UnsupportedCapture { name: string, reason: string },   // capture VARIABLE name, not slot index
      Protocol { message: string },               // transfer-integrity class: hash mismatch, malformed
                                                  //   request, retry-exhausted missing blobs,
                                                  //   call-ABI (arity / arg-kind / return-kind) mismatch
      Remote { message: string },                 // the callee's own runtime error
  }
  ```

  The **complete wire-kind → `RemoteError` mapping** is normative in §4.9 — every wire `RemoteErrorKind` and every sender-local failure has exactly one Shape variant. The may-have-executed / did-not-execute distinction is encoded **per variant** — `Transport` = did NOT execute (pre-send failures only); `Timeout` and `ConnectionLost` = MAY have executed — because that is precisely what retry annotations branch on. The pre-send/post-send split is load-bearing: a connection that dies *after* the request went out is not retry-safe, and a single "connection closed ⇒ Transport ⇒ did not execute" bucket (the previous draft's shape) would make a variant-trusting retry annotation unsafely auto-retry non-idempotent calls. Framing makes the boundary crisp: requests are length-prefixed frames, so a write failure mid-send leaves the receiver with an undecodable partial frame (not executed); once the frame is fully written, any subsequent loss is `ConnectionLost`. This is what makes §1's non-goal delegation real: userland retry/load-balancing/idempotency annotations `match` on variants — they never parse error-message text. (The previous draft's `Result<R, string>` broke its own userland story.)
- Sender-side argument serialization: each pack field is serialized via `slot_to_serializable(bits, kind, store)` with the compiler-proven field kind — no container-schema guessing, no heterogeneity problem.
- Permission: `remote::call` checks `NetConnect` sender-side (as today, `remote_builtins.rs:329-332`); `NetScoped` constraints are checked against the **hostname string** before resolution (§4.6).

#### 4.1.2 `@remote` — the annotation, working

```shape
@remote("worker:9527")
fn compute(data: Array<int>) -> int { ... }

let r = compute([1, 2, 3])   // executes on worker:9527
```

The annotation stays a thin stdlib policy layer (Layer 4 in the module's own architecture diagram, `remote.rs:9-16`) over the `remote::call` family:

```shape
pub annotation remote(addr: string) {
    targets: [function]
    before(args, ctx) {
        // ctx.target: typed annotation-context accessor for the annotated
        // function (specified by comptime-excellence.md §4.1.5 under this
        // exact name; ratified 2026-07-05 — OQ-12) — no stringly
        // ctx["__impl"] magic, and NO fallback to args[0]: if the target
        // binding is unavailable that is an annotation-machinery bug and
        // compiles/fails loudly, it never misdispatches a call argument
        // as the callee (the remote.shape:100 `?? args[0]` latent bug is
        // not carried into this design).
        { result: __call_raising(addr, ctx.target, args) }
    }
}
```

- **Hook contract — `{ result: v }` short-circuits.** A before-hook that returns an object with a `result` key substitutes `v` as the call's result and the original body does **not** run; a before-hook without it falls through to the original body. This is the existing annotation-hook convention (the audit's before-hook short-circuit probe; JIT parity gated by `comptime-excellence.md` §7 P9 / WF-1A(c)), restated here so this doc is self-contained: `@remote` uses the short-circuit to substitute remote execution for local execution.
- **Where elaboration happens — per-application-site handler specialization.** `__call_raising(addr, ctx.target, args)` sits inside a *generic* annotation body, where neither the callee nor the arg pack is concrete — §4.1.1's elaboration preconditions do not hold at the annotation-definition site. They hold because before/after handlers are **compiled per application site**: the compiler specializes the handler template for each annotated function (`compile_specialized_annotation_handler`, `functions_annotations.rs:2004-2010`), and `@remote`'s elaboration is defined to run **inside the specialized handler**, where `ctx.target` is statically the concrete annotated function and `args` is that site's kinded carrier mapped 1:1 onto the target's declared params (arity and types known). This is a stated **hard requirement on WF-1B's typed-ctx / kinded-annotation-carrier design** — now a citation into an existing, ratified specification: `comptime-excellence.md` §4.1.5 specifies the runtime-hook contract under exactly this name (`ctx.target` as a typed function value statically bound in the specialized handler, plus the specialization-time kinded args pack), and its runtime-hook `ctx` v1 is `{module_path, file}` (`build` dropped — `build_config()` is the single build-info surface, its OQ5). §4.1.5 is the deliverable OQ-12 demanded of WF-1B; ratified 2026-07-05. The carrier must support positional elaboration in specialized handlers. If it cannot, that is a design conflict to surface to the user — not something to bridge with a runtime-dispatched variadic `__call_raising`, which would require exactly the kind-blind path this design refuses. `__call_raising` has **no** non-elaborated contract.
- **Error semantics — precise, no exceptions (recommended, see OQ-1).** Shape has Result types, not exceptions (CLAUDE.md), so "raise as ordinary runtime error" means exactly one thing: the native body returns `Err(String)` at the builtin layer, which the VM surfaces as a runtime error (`VMError`-class) that halts execution with a legible diagnostic — the same propagation as any failed builtin (e.g. integer-overflow per the numeric ruling D3). Concretely: `__call_raising` is a compiler-elaborated internal sibling of `remote::call` whose native body maps the `RemoteError` case to a native `Err` carrying the §4.6/§4.9 user-legible message. It is not user-callable API: like `__into_*` it remains reachable by compiler-generated code but is undocumented and not in the book. So under the recommended OQ-1 branch, `let r = compute([1,2,3])` keeps type `int`; a dropped network halts the program with `remote call 'compute' to worker:9527 failed: connection refused` — it is **not** catchable, by design. If the annotated function's own return type is `Result<T, E>`, the remote callee's `Ok/Err` passes through unchanged; only transport/protocol failures raise. Users who want recoverable transport failures call `remote::call` directly (it returns `Result<R, RemoteError>`, §4.1.1) or write their own annotation over it — that composability is the design goal, not a gap. The snippet above implements the recommended branch; if OQ-1 resolves to the alternative (force `Result` returns), the hook body switches to folding `RemoteError` into the function's `Err` channel.
- The annotation args carrier must preserve per-arg kinds through the before-hook. This rides on WF-1B's comptime/annotation marshal fixes (the Bool-collapse root cause, fix-plan §WF-1B); this design **depends on** WF-1B's carrier being kinded and adds an acceptance probe for mixed-kind args through `@remote` (§7).
- **Config-driven addressing.** Annotation arguments accept any **comptime-evaluable expression**, not just string literals: `@remote(build_config("WORKER_ADDR"))` is the blessed non-toy deployment form (comptime `build_config` exists today). **Verification note (pre-book):** the annotation-args grammar accepts full expressions (`shape.pest:360-362`), so this form *parses* — but whether comptime *evaluation* runs in annotation-argument position must be verified (and landed if absent) before T16's book chapter teaches it; folded into OQ-9's elaboration scope. Runtime-expression addresses are out of scope for the annotation (use `remote::call` directly, whose `addr` is an ordinary runtime `string`); whether `@remote` should ever accept runtime expressions is OQ-10. The book chapter (T16) teaches the `build_config` form, not hardcoded hosts.

#### 4.1.2b Comptime pre-flight — refusals at compile time, not after a round trip

The 2026-07-05 ruling requires comptime to be excellent and ergonomic; a distributed-call design that discovers statically-knowable refusals at call time is repaired, not excellent. Everything in the §4.4 refusal matrix that is a property of the **annotated function itself** is compiler-known at annotation-expansion time (capture metadata — `mutable_captures`, ref flags — is already stamped into blob metadata, §2; param/return types and `foreign_dependencies` likewise). Therefore `@remote` carries a `@comptime` block that validates at compile time and `comptime error`s with the same remediation text the runtime path would give:

- **Refused captures** (mutable, `&`/`&mut`, FFI-handle/resource, foreign refs until the Q53-green-lit typed carrier lands (integration A3(iii) — the refusal outlives WF-2F itself), nested closures in v1 — the full §4.4 matrix) ⇒ `comptime error` naming the capture and the fix ("pass the value as an argument and return the new value").
- **Unserializable param/return types** ⇒ `comptime error` naming the offending type in Shape surface syntax.
- **Permission surface disclosure (recommended):** `comptime warning` listing the target's transitive `required_permissions` ("remote target 'compute' requires [fs.write] — the server must grant it"), so deployment surprises move to build time. Ratify as warning vs. silent: folded into OQ-3's UX scope.

The comptime hook reads target metadata through the bare `target` descriptor that `comptime-excellence.md` §4.1.1 **already specifies** for `@comptime` annotation handlers — `target.name` / `target.params` / `target.return_type` / `target.captures`, plus the committed-additive `target.required_permissions` extension that doc delivers with WF-2C — so the **comptime** pre-flight stands on a specified contract. The **runtime** before-hook's `ctx.target` accessor is specified by `comptime-excellence.md` §4.1.5 (the OQ-12 deliverable, ratified 2026-07-05); the two contexts must still not be conflated. The direct `remote::call` path gets the **same compile-time coverage via elaboration** (§4.1.1): type-level checks always run at compile time; capture-level checks run at compile time whenever the callee is a named declaration. Sender-side **call-time** pre-flight (§4.4, §4.9) remains only for capture-level checks when `fn_ref` is a runtime closure value.

#### 4.1.3 `remote::execute` and `remote::ping` — retained, rescoped

- `remote::ping` unchanged (works today).
- `remote::execute(addr, code)` is **retained** as the explicitly-documented "ship source, run it there" tool (REPL/notebook/admin use). The book gets a callout: *execute ships source and recompiles remotely; `@remote` ships content-addressed compiled functions with permission identity — use `@remote` for distributed computing.* Its placeholder-string degradation (B9) is fixed by WF-2E/W17-followup typed projection, not here.
- The two stacks stop being "disjoint" (audit gap 10) because `@remote` now works; they remain *distinct on purpose*.

### 4.2 Wire protocol changes (v1-compatible, additive)

All changes are `#[serde(default)]`-additive to existing structs. The wire codec is `rmp_serde::to_vec_named` — **map-named encoding** (`crates/shape-wire/src/codec.rs:108-112`) — which fixes the actual compatibility semantics; the previous draft stated them wrong, so they are spelled out precisely:

- **New sender → old receiver: the request decodes.** Unknown map keys (`call_format`, `upvalue_kinds`) are silently *ignored* by serde defaults — old receivers do not reject them, structurally or otherwise. `call_format` is therefore **unenforceable by any pre-upgrade server**: it never sees the field and can never reply `VersionSkew`. This is benign in v1 for exactly one reason: the only new-format behavior an old server would misinterpret is a closure call, and old servers blanket-reject *all* closure calls with a structured error (B2, `remote.rs:731-739`). New-format **non-closure** calls are processed by old servers under old semantics (including B3's missing permission gate) — this design accepts that explicitly: version *enforcement* begins at the first upgraded server; version *diagnosis* rides on `remote::ping`'s `wire_protocol` field, which old servers do answer.
- **Old sender → new receiver: decodes, `call_format` defaults to 0.** **Kind-downgrade rule:** replies to `call_format == 0` requests use only the four pre-existing `RemoteErrorKind`s (`FunctionNotFound` / `ArgumentError` / `RuntimeError` / `MissingModuleFunction`), carrying the specific detail in `message` — because a reply bearing a new variant name fails decode on the old sender and degrades a structured error into a connection-level codec error.
- **Undecodable envelope:** decode failure on the receiver propagates via `?` and closes the connection (`serve_cmd.rs:224-225`); no structured reply is possible for bytes that don't parse. The §4.9 version-skew row states this honestly ("if the envelope parses … else closes").

```rust
// remote.rs — RemoteCallRequest gains:
pub struct RemoteCallRequest {
    ...existing fields...,
    /// Per-capture NativeKind track, lockstep with `upvalues`.
    /// REQUIRED when `upvalues` is Some — parallel-vec shape per ADR-006 §2.7.7/§2.7.8.
    #[serde(default)]
    pub upvalue_kinds: Option<Vec<shape_value::NativeKind>>,
    /// Sender's wire feature revision (monotone int). Receivers reply
    /// with a structured VersionSkew error if they cannot serve it.
    #[serde(default)]
    pub call_format: u32,   // = 1 for this design
}

// RemoteCallError gains a structured payload channel:
pub struct RemoteCallError {
    pub message: String,
    pub kind: RemoteErrorKind,
    /// Content hashes the receiver needs to link the entry function.
    /// Populated when kind == MissingModuleFunction.
    #[serde(default)]
    pub missing_blobs: Option<Vec<FunctionHash>>,
}

// RemoteErrorKind gains new kinds (additive; under named encoding variant order is
// non-semantic — placement is readability only. Per the kind-downgrade rule above,
// new kinds are only ever sent in replies to call_format >= 1 requests):
pub enum RemoteErrorKind {
    FunctionNotFound, ArgumentError, RuntimeError,
                                    // ArgumentError = the call-ABI-mismatch class: arity,
                                    //   argument-kind, AND return-kind cross-check failures
                                    //   (§4.9) — not only "bad argument"
    MissingModuleFunction,          // NOW CONSTRUCTED (B4)
    PermissionDenied,               // NEW — receiver refused required_permissions
    HashMismatch,                   // NEW — blob content failed hash verification
    VersionSkew,                    // NEW — wire/call_format/protocol mismatch
    UnsupportedCapture,             // NEW — closure capture refused (§4.4)
    AuthRequired,                   // NEW — token missing/rejected. Sent only in replies to
                                    //   call_format >= 1 requests (kind-downgrade rule);
                                    //   the pre-upgrade token-gate behavior is unchanged
                                    //   for call_format == 0 senders
    ResourceLimitExceeded,          // NEW — remote execution hit ResourceLimits
                                    //       (incl. wall-time overrun: limit "wall_time")
    Timeout,                        // RESERVED — not produced in v1. Receiver execution-
                                    //   deadline overrun is ResourceLimitExceeded; the
                                    //   sender-side read timeout is sender-LOCAL and never
                                    //   crosses the wire (§4.9). Reserved for a future
                                    //   per-request deadline distinct from ResourceLimits.
}
```

No `WireMessage` variants are removed; `BlobNegotiation`/`Sidecar` (already in the enum, `remote.rs:147-155`) go from dead to used (§4.5).

### 4.3 Blob transfer protocol

**Sender (inside `remote::call`):**

1. Resolve `fn_ref` → `function_id` → `function_hash` via `program.function_blob_hashes` (canonical identity; the name-based ambiguity path is only a fallback and now **errors** instead of silently shipping the full program — see §5-R7).
2. `build_minimal_blobs_by_hash` computes the transitive closure (existing, `remote.rs:454-489`).
3. On a pooled connection (below), send `BlobNegotiation{offered_hashes}` first; strip blobs the receiver already has (`build_call_request_with_negotiation`, exists at `remote.rs:1006`). On ephemeral connections, skip negotiation and send everything (correctness first, bandwidth second).
4. Send `Call`.

**Sender connection lifecycle (new — negotiation, §4.5, and T10/T11 depend on it).** Today the builtin path is one-shot: `wire_roundtrip` builds a fresh `TcpTransport` per call (`remote_builtins.rs:95-110`), which makes per-connection negotiation a fiction on the sender side. `remote::call` gains a process-wide **connection pool** owned by the remote builtins module: `Mutex<HashMap<addr: String, PooledConnection>>`, where `PooledConnection` holds the transport plus the **sender-side negotiated-hash set** (which hashes this connection's receiver has acknowledged holding). Rules:

- **Negotiation state lives and dies with its connection.** Any transport error or reconnect discards the pooled entry *and* its negotiated set together — a stripped request can never be sent on a fresh connection whose receiver state is unknown.
- **One in-flight call per pooled connection** (checkout/return under the mutex). A concurrent call to the same addr opens an ephemeral, non-pooled, non-negotiating connection — correctness first; pooling a second connection is a later optimization.
- **Idle connections** are dropped after a keep-alive window (bounded by the receiver's existing 30 s read timeout, `tcp.rs:26`).

This is deliberately minimal — no multiplexing, no health checks, no load balancing (§1 non-goal). It exists to make negotiation and the per-connection receiver cache real, not to be a connection manager.

**Receiver (`handle_call`, now taking `&mut ConnectionState` for real):**

1. **Merge**: `request.function_blobs ∪ state.blob_cache` — the cache supplies blobs stripped by negotiation. Pending sidecars referenced by the request are folded in (sidecar payloads carry serialized blobs for >threshold sizes; threshold and framing detail deferred to implementation, protocol slot already exists).
2. **Verify**: for every blob received over the wire, recompute `blob.compute_hash()` and compare to the claimed key. Mismatch ⇒ `CallResponse(Err{kind: HashMismatch})`, blob NOT cached. This is what makes "never trust the sender" real: permissions are inside the hash (`content_addressed.rs:114`), so a sender cannot claim `Pure` for a blob whose bytes demand `FsWrite` — the hash won't verify against a store keyed by verified hashes, and the linker union below is computed from verified blob contents only.
3. **Cache**: verified blobs enter `state.blob_cache` (and a process-wide shared cache — see OQ-5) keyed by content hash.
4. **Link, fallibly**: build the content-addressed `Program`, call `crate::linker::link` directly. `Err(LinkError::MissingBlob(hash))` ⇒ collect **all** missing hashes (linker returns the first today; extend to accumulate) and reply:
   `CallResponse(Err{kind: MissingModuleFunction, missing_blobs: Some(hashes), message: "cannot link '<fn>': missing N dependency blob(s)"})`.
   `vm.load_program`'s panic path (`program.rs:9-15`) becomes unreachable from network input because the remote path never calls it — it calls the fallible linker + `load_linked_program_with_permissions` (§4.6).
5. **Sender retry**: `remote::call` receives `MissingModuleFunction`, looks up the named hashes in its own content store (`ContentBlobSupplier::blob_bytes`, §4.1.1), sends them (as blobs or sidecars), retries **once**. Second failure surfaces to the user as `RemoteError::Protocol` ("receiver still missing N blob(s) after resupply"). If the sender's own store lacks a requested hash (should be impossible for a closure the sender itself computed; reachable in principle via store eviction), the retry aborts with `RemoteError::Protocol { message: "receiver is missing blob <hash> and the sender cannot resupply it" }` — a defined terminal error, not a hang or a panic (§4.9 row). This bounded loop is the whole negotiation story — no unbounded chatter.
6. **Program cache**: `ConnectionState` (and the shared layer) gains `program_cache: HashMap<CacheKey, Arc<LinkedProgram>>` where `CacheKey = (entry FunctionHash, granted-permission epoch)`. Repeat calls to the same entry hash skip relink entirely. The shipped `program_hash: [u8;32]` field is *retired from semantic use* in favor of entry-hash keying (full-program `rmp` hash is not content-addressed identity; see §5-R8). **API note (cost stated honestly — the previous draft's "shared loading without cloning" was unsubstantiated):** `load_linked_program_with_permissions` takes `LinkedProgram` **by value** (`program.rs:381-387`), and loading *moves* the program body into the VM-owned `BytecodeProgram` (`program.rs:128-175`) — you cannot move out of an `Arc`. A zero-clone shared-execution API is therefore not a small sibling function: it means either Arc-backed body fields inside `BytecodeProgram` or the executor running directly off `Arc<LinkedProgram>` — a wide executor refactor whose blast radius includes serde, since `BytecodeProgram` is itself a serialized wire field (`RemoteCallRequest.program`, `remote.rs:56-59`). **Not claimed here.** The v1 contract: the cache stores `Arc<LinkedProgram>`; a cache hit skips **re-verify + re-link** (the expensive steps the cache exists for) and pays **one program-body clone** into the by-value permission-checked loader per execution. The cache ships with Stage 1.5. If the per-hit clone measures hot, the zero-clone executor refactor is a separately-ratified follow-up — not smuggled in as a cache detail.

### 4.4 Closures / upvalues across the wire

**Wire shape:** parallel vectors, exactly the §2.7.7/§2.7.8 shape — `upvalues: Vec<SerializableVMValue>` + `upvalue_kinds: Vec<NativeKind>`, equal length, index-aligned with the closure's capture slots. `upvalues.is_some() && upvalue_kinds.is_none()` ⇒ structured `ArgumentError` ("closure request missing capture kind track") — never a Bool-default, never kind-from-bits.

**Blob-side identity:** `FunctionBlob` gains `capture_kinds: Vec<NativeKind>` (`#[serde(default)]`), stamped at blob construction from the compiler's proven capture kinds (same proof source as `frame_descriptor.slots`), and **included in `FunctionBlobHashInput`** — a closure body that reads capture 0 as `Float64` is a different function from one that reads it as `Ptr(TypedObject)`; capture layout is call-ABI identity exactly like param kinds. Metadata also gains `capture_names: Vec<String>` (the captured variables' source names, sibling of the existing `param_names`) — **not** hash-identity, but load-bearing for legible errors below.

**Receiver flow** (replacing the blanket rejection at `remote.rs:731-739`):

1. Cross-check `upvalue_kinds` against the callee blob's `capture_kinds` (length and per-index equality). Mismatch ⇒ `ArgumentError` naming the index and both kinds.
2. Materialize each capture via `serializable_to_slot(value, kind, store)` into the closure cell store, writing the parallel `ClosureCell.kinds` track (ADR-006 §2.7.8 struct shape) — the receiving VM's `Load*Ptr` handlers then work unmodified.
3. Dispatch through the §2.7.11 value-call ABI with `CallFrame.closure_heap_kind` set from the callee blob (existing field, per ADR-006 §2.7.11).

**What is refused, and with what error.** All refusals are `UnsupportedCapture { name, reason }`. Messages are **user-legible by rule, not by exception**: they name the captured *variable* (from `capture_names`), render its type in **Shape surface syntax** (via the blob's `type_schemas` — `Array<int>`, not `Ptr(HeapKind::TypedArray)`), and state the reason plus remediation. `NativeKind` variant names and slot indices never appear in user-facing messages — the §4.6 no-internal-jargon rule (WF-3B) applies to **every** error row in this design, runtime and comptime alike, so the §4.1.2b compile-time path and the call-time path speak one vocabulary:

| Capture | Verdict | Rationale |
|---|---|---|
| Scalars (`Float64/Int64/Int32/Int8/Bool/Unit/Null`), strings, decimals | shipped | value copy, trivially serializable |
| TypedArray / TypedObject / HashMap / enum payloads | shipped | `SerializableVMValue` + snapshot store already carry these |
| **Nested closure captures** (a capture that is itself a closure) | **refused in v1** | the transfer unit is the *static* blob closure (BFS over `blob.dependencies`, `remote.rs:454-489`); a runtime closure value's function blob is not statically discoverable from the entry blob, so shipping it needs runtime-value-driven blob discovery plus recursive capture kind-tracks — rejected for v1 as R13. Message suggests hoisting the inner closure to a named top-level function (a static dependency) or passing its result as a value. (Resolves the previous draft's §4.4-vs-§4.1.2b contradiction: refusal is normative in both.) |
| **Mutable captures** (`mutable_captures[i] == true`) | **refused in v1** | send-copy would silently drop write-back; refusing is honest. Message suggests: pass the value as an argument and return the new value. (OQ-2) |
| `&`/`&mut` reference captures | refused | references don't cross nodes (non-goal; 2026-05-29 ruling keeps coherence single-node) |
| FFI handles, open resources (files, sockets), `Drop`-bearing resource objects | refused | no meaningful remote identity |
| Foreign-function refs (python/ts/C) | refused in v1 — and the refusal **remains in force after WF-2F** until the ratified typed carrier lands (integration A3(iii)/OQ-7, overview Q53(b) green-lit 2026-07-05) | extension resolution undefined here; no `SerializableVMValue` arm / §2.7.8 kind-track story / receiver rebinding rule exists for the captured *value* |

### 4.5 Receiver caching

Two layers, both hash-keyed:

- **Per-connection** `ConnectionState.blob_cache` (exists, `serve_cmd.rs:62-63`) — now actually consulted by `handle_call` (fixes B5); serves negotiation replies (already wired at `serve_cmd.rs:353`).
- **Process-wide** `Arc<RwLock<RemoteBlobCache>>` + `LinkedProgram` cache shared across connections, bounded LRU (size from serve config). Verified-hash-only admission (§4.3-2) means the cache can never be poisoned by an unverified blob.

Cache invalidation: content-addressed data never invalidates (hash = identity). The `LinkedProgram` cache key includes a granted-permission **epoch** (bumped when server permission config reloads) so a permission tightening cannot be bypassed by a warm cache.

### 4.6 Permission story end-to-end (never trust the sender)

```
compile time   capability_tags → per-blob required_permissions → INSIDE content_hash
                (compiler_impl_initialization.rs:276, content_addressed.rs:114)
link time      linker union → LinkedProgram.total_required_permissions
                (linker.rs:327-329)  — recomputed ON THE RECEIVER from verified blobs
load time      load_linked_program_with_permissions(linked, &server_granted)
                (program.rs:381-400)  — deny ⇒ CallResponse(Err PermissionDenied)
run time       check_permission() gating per stdlib call, as everywhere else
resource tier  VM built with ResourceLimits from serve config — never VMConfig::default()
```

- **Granted set is receiver-owned.** `shape serve` derives `granted: PermissionSet` + `ScopeConstraints` from `--sandbox <level>` / shape.toml `[permissions]`/`[sandbox]` (plumbing is WF-1D's `Thread` phase; this design consumes it). Nothing in the request influences the granted set. Recommended defaults: loopback bind ⇒ `sandboxed()` limits + a moderate permission set; non-loopback ⇒ deny-all-but-Pure until explicitly configured (OQ-3).
- **Deny UX:** `PermissionDenied` error carries the missing permission names (already computed by `PermissionError::InsufficientPermissions`, `program.rs:365-371`) and the entry function name. Sender-side, `remote::call` surfaces: `remote call 'compute' refused by worker:9527 — the server does not grant [fs.write]`. The message is renderable **from the payload alone** (`PermissionDenied { missing }` — the wire carries only the missing set): disclosing the server's *full granted set* in the refusal was considered and **rejected** (R14) — it leaks server security configuration to possibly-unauthenticated callers, and the missing set is sufficient remediation. The operator sees the granted set server-side (logging contract below). No internal jargon (WF-3B rule — applied to every §4.9 row, see §4.4).
- **Receiver-side operator surface (logging contract).** Every structured refusal logs one warn-level line on `shape serve`: peer address, entry function name + content hash, refusal class, and class-specific detail — `PermissionDenied` logs the missing set **and** the active granted set with its config source (flag / shape.toml / default), i.e. exactly what the client-facing message deliberately omits; `HashMismatch` logs the claimed hash; `AuthRequired` logs the peer and nothing secret; `UnsupportedCapture` logs the capture variable and reason. Argument/capture *values* are never logged. This is the operator's answer to "why do clients get PermissionDenied"; T9 asserts the server-side line alongside the sender-side error.
- **Why the sender can't cheat — stated precisely (the previous draft overstated the hash's power).** Recompute-verify (§4.3-2) proves **integrity only**: the received bytes are exactly the blob the hash names. That kills in-flight tampering and makes permission edits change identity (T1) — for *compiler-produced* blobs. It does **not** prove the permission claim is honest: a hand-crafted blob whose instructions call `file::write_text` but whose `required_permissions` is `{}` hashes perfectly validly, and the linker union over such blobs is a union of sender-authored claims. Therefore, per Goal 4: the **load-time gate is fast-fail UX for honest senders; the runtime gate is the security boundary against dishonest ones.** On the remote path every stdlib I/O call hits `check_permission` with `granted_permissions: Some(server_granted)` — mandatory, because `None` is fail-open (`module_exports.rs:150-163`).
- **How the granted set reaches the runtime gate (mechanism, named):** `ModuleContext` construction at builtin dispatch currently hardcodes `granted_permissions: None` (`crates/shape-vm/src/executor/vm_impl/modules.rs:705`). The remote-execution VM is constructed with the serve-config granted set + `ScopeConstraints` in its VM configuration, and every `ModuleContext` built during that VM's dispatch populates `granted_permissions: Some(server_granted)` / `scope_constraints` from it. This is WF-1D `Thread`-phase plumbing (Stage 0) that this design consumes; the remote path **refuses to execute** if the plumbing is absent (surface-and-stop — network-supplied code never runs against a fail-open `None`).
- **Optional hardening (recorded, not v1-required):** the receiver *can* additionally re-derive `required_permissions` from the verified instructions (the same `capability_tags` table the compiler uses, unioned over the already-transferred dependency closure) and take `max(claimed, derived)` at load time — making the load gate meaningful against dishonest senders too. Deferred: the runtime gate already holds the boundary, and unlike R6 this is a table lookup over builtin calls, not a second kind-inference implementation; noted here so the option isn't lost.
- **ResourceLimits:** `handle_call` builds the execution VM with limits from serve config (default `ResourceLimits::sandboxed()`: 256 MB / 30 s / 1 MB output). Exceeding ⇒ `ResourceLimitExceeded` structured error. `let _sandbox = config.sandbox` (`serve_cmd.rs:430`) dies in WF-1D.
- **Ffi permission (reserved seam):** WF-1D lands the enum variant; blobs with non-empty `foreign_dependencies` will union `Ffi` and be refused by receivers not granting it — enforcement semantics designed in WF-2F.

### 4.7 Addressing & transport

- **Hostname resolution (B7):** `tcp.rs` `send`/`connect` replace `destination.parse::<SocketAddr>()` with `std::net::ToSocketAddrs` resolution (first-address semantics, connect-timeout preserved). `localhost:9527` and DNS names work; book examples run as written. `NetScoped` host constraints are matched against the **pre-resolution hostname string** (so scopes read naturally: `"*.internal:9527"`); DNS-rebinding hardening (pin the resolved IP for the connection lifetime) noted as an implementation requirement.
- **Transport security (B6):** the `--tls-cert/--tls-key` gate becomes honest — recommended v1: **TLS-on-TCP via tokio-rustls** on the serve side and a rustls client config in `TcpTransport` (address scheme `tls://host:port` or a transport-kind flag), because the entire existing framing/serve stack is TCP and QUIC's tokio runtime nesting inside builtins is riskier. QUIC (`quic.rs`) remains feature-gated for later; `new_self_signed` (`quic.rs:112-117`) gets a `#[doc = "dev-only"]` fence and is never reachable from a default build path. Until TLS lands, the non-loopback gate **refuses** rather than pretending (`serve_cmd.rs:112` comment lie removed). Ruling requested: OQ-4.
- **Auth:** token auth (existing, `serve_cmd.rs:271-287`) stays; recommendation: non-loopback binds **require** `--auth-token` (upgrade the current warning at `serve_cmd.rs:102-106` to a refusal), loopback keeps it optional. Folded into OQ-4.

### 4.8 `frame_descriptor` integrity (B8)

`frame_descriptor` defines the call ABI (per-slot `NativeKind`, `abi_return_kind`) and drives the remote marshal, but is excluded from `FunctionBlobHashInput` — so hash-identical blobs can disagree on the one thing the receiver trusts for marshal.

**Decision (recommended): hash it.** Add `frame_descriptor` (and the new `capture_kinds`, §4.4) to `FunctionBlobHashInput`. Consequences: every existing content hash changes once (pre-1.0, no persisted blob stores in the wild per the audit — cheap now, expensive later); `source_map` and `callee_names` remain non-identity (debug/ephemeral, correct to exclude).

**Scheduling (one doc-set-wide break, not one per doc):** `polyglot-distributed-integration.md` §4.2.0-4 schedules its own hash-affecting blob-format change (A6, `foreign_dependencies` canonicalization) inside the **WF-2A stage-0 hash-stabilization window**, explicitly because "two separate invalidation events would be worse" — that rationale binds here identically. The `FunctionBlobHashInput` additions are therefore **pulled forward out of Stage 2 into that same stage-0 window** and batched with A6: the ratified doc set contains exactly **one** blob-hash invalidation event. This is feasible early because the compiler already holds proven capture kinds (the same proof source as `frame_descriptor.slots`) — stamping and hashing the fields does not wait for the Stage 2 closure runtime path, which then lands against already-final hashes. Alternative (re-derive descriptors receiver-side from instructions) rejected in §5-R6. User sign-off: OQ-6.

### 4.9 Failure modes table

Every row names the **Shape-level `RemoteError` variant** the user matches on plus a sample message (the §4.4 legibility rule applies to all rows: variable/parameter names and Shape surface types, never `NativeKind` names or slot indices).

| Failure | Detected by | `RemoteError` variant surfaced by `remote::call` | Sample message | Sender behavior |
|---|---|---|---|---|
| Connect refused / DNS failure / send failure | transport (sender-local, pre-send) | `Transport { message }` | `cannot reach worker:9527: connection refused` | immediate, no retry (call never started) |
| Connection lost awaiting reply (closed/reset after full send) | transport (sender-local, post-send) | `ConnectionLost { message }` | `connection to worker:9527 lost after the request was sent — the call may have executed` | no auto-retry (not idempotent-safe) |
| Timeout (transport read) | `TcpTransport.read_timeout` (30 s default, `tcp.rs:26`), sender-local | `Timeout { message }` | `no reply from worker:9527 within 30s — the call may have executed` | no auto-retry (not idempotent-safe) |
| Timeout (execution) | receiver `ResourceLimits` wall-time | `ResourceLimitExceeded { limit: "wall_time" }` | `remote 'compute' exceeded the server's 30s execution limit` | surfaced |
| Version skew (wire enum) | msgpack decode failure | `Transport { message }` (sender-side decode) / receiver replies `VersionSkew` if envelope parses, else closes | sender-local: `reply from worker:9527 could not be decoded — possible wire-protocol mismatch; run remote::ping to compare wire_protocol` (a local decode failure cannot know the server's version — "N vs M" wording is only producible from a parsed `VersionSkew` reply or the ping diagnostic) | surfaced; `ping.wire_protocol` is the diagnostic tool |
| Version skew (call_format) | **upgraded** receiver's `call_format > supported` check (pre-upgrade servers never see the field, §4.2) | `VersionSkew { server, client }` | `worker:9527 supports call format 1, client sent 2` | surfaced |
| Missing dependency blobs | linker `MissingBlob` (accumulated) | *(handled)* → `Protocol { message }` only after retry exhausted | `receiver still missing 2 blob(s) after resupply` | resend missing blobs, retry once, then surface |
| Retry resupply impossible | sender's own content store lacks a requested hash (§4.3-5) | `Protocol { message }` | `receiver is missing blob 3fa2… and the sender cannot resupply it` | surfaced, no retry |
| Hash mismatch (tampered/corrupt blob) | receiver recompute-verify | `Protocol { message }` | `blob 3fa2… failed content verification — rejected` | surfaced; no retry (data is wrong, not missing) |
| Permission refusal | `load_linked_program_with_permissions` | `PermissionDenied { missing }` | deny UX, §4.6 | surfaced |
| Function not found | callee resolution (`remote.rs:747-782`) | `MissingFunction { name }` | `worker:9527 has no function 'compute'` | surfaced |
| Arity mismatch | `remote.rs:797-807` | `Protocol { message }` | `'compute' takes 2 arguments, request carried 3` | surfaced (should be unreachable: elaboration checks arity at compile time) |
| Kind mismatch (arg) | `serializable_to_slot` against expected kind | `Protocol { message }` | `argument 'data' of 'compute': expected Array<int>, received string` | surfaced (should be unreachable post-elaboration; reachable from hand-built requests) |
| Kind mismatch (return) | return cross-check (`remote.rs:877-888`); receiver replies wire `ArgumentError` (call-ABI-mismatch class, §4.2) | `Protocol { message }` | `'compute' declared return int but produced number` | surfaced, never reinterpreted. Mapped to `Protocol`, not `Remote`: a declared-vs-produced return-kind disagreement is transfer/identity integrity (the `Protocol` class definition), not the callee's own runtime error — users read `Remote` as "my code failed remotely" |
| Frame descriptor absent | `remote.rs:824-833` | `Protocol { message }` (§2.7.5.1 structured, exists) | `blob for 'compute' carries no frame descriptor — re-export it with a current compiler` | surfaced |
| Unsupported capture | compile-time pre-flight (§4.1.1/§4.1.2b) or call-time/receiver cross-check | `UnsupportedCapture { name, reason }` | `closure captures 'counter' mutably — pass it as an argument and return the new value` | surfaced with remediation hint |
| Auth token missing/wrong | token gate (`serve_cmd.rs:271-287`); non-loopback servers **refuse** unauthenticated connections (§4.7) | `AuthRequired { message }` | `worker:9527 requires authentication — pass an auth token (see shape serve --auth-token)` | surfaced; distinct variant so credential-handling annotations (refresh token, prompt operator) match on it without parsing text |
| Payload > 64 MB | transport cap (`tcp.rs:13-14`), sender-local pre-send | `Transport { message }` | `request payload is 71 MB, the transport cap is 64 MB — reduce argument sizes (pass a reference such as a path/URL, or split the data across calls)` | surfaced. Code blobs above the sidecar threshold are split **automatically** (§4.3-1) — users have no sidecar knob, so "use sidecars" is not a remediation; a cap hit whose bulk is blobs means the automatic split failed (an implementation bug, not a user action). The message targets argument data, the only part the user controls |
| Remote runtime error | callee execution | `Remote { message }` | `remote 'compute' failed: index 12 out of bounds` | surfaced |

**Normative wire-kind → `RemoteError` mapping** (total — every wire kind has exactly one Shape variant; sender-local failures never cross the wire):

| Wire `RemoteErrorKind` | Shape `RemoteError` |
|---|---|
| `FunctionNotFound` | `MissingFunction { name }` |
| `ArgumentError` | `Protocol { message }` |
| `RuntimeError` | `Remote { message }` |
| `MissingModuleFunction` | absorbed by the retry-once loop; `Protocol { message }` when retry is exhausted or resupply impossible |
| `PermissionDenied` | `PermissionDenied { missing }` |
| `HashMismatch` | `Protocol { message }` |
| `VersionSkew` | `VersionSkew { server, client }` |
| `UnsupportedCapture` | `UnsupportedCapture { name, reason }` |
| `AuthRequired` | `AuthRequired { message }` |
| `ResourceLimitExceeded` | `ResourceLimitExceeded { limit }` |
| `Timeout` | reserved, not produced in v1 (§4.2); if it ever arrives: `Timeout { message }` |
| *(sender-local, pre-send: connect/DNS/send failure/payload-cap)* | `Transport { message }` |
| *(sender-local, post-send: connection closed/reset awaiting reply)* | `ConnectionLost { message }` |
| *(sender-local: read timeout)* | `Timeout { message }` |

---

## 5. Alternatives considered & rejected

- **R1 — "Just send raw u64 slots; the receiver knows the frame descriptor."** A kind-blind wire carrier is exactly the deleted ValueWord shape (a `u64` whose meaning is discovered later). Rejected on sight per CLAUDE.md §Forbidden. The wire carries self-describing `SerializableVMValue` payloads *checked against* descriptor kinds — the descriptor selects the expected kind, it never reinterprets bits.
- **R2 — A tagged-word wire value ("compact NaN-boxed wire encoding").** Same family, renamed. Rejected. §2.7.7's parallel data+kinds shape is the only serialized-slot format.
- **R3 — Bool-default (or `Unknown`) kinds for captures whose kind isn't tracked yet.** The canonical W-series rationalization, explicitly forbidden by §2.7.7 #9 and §2.7.8. Rejected: captures without a proven kind are *refused* (`UnsupportedCapture`), surface-and-stop.
- **R4 — A `ConvertRemoteReturn` opcode / receiver-side coercion when return kind mismatches.** The W4-δ `ConvertBoolToString` pattern. Rejected: kind mismatch is an error (§4.9); if it recurs systematically the fix is the compiler's kind stamping, not a conversion.
- **R5 — Ship source text for `@remote` (extend `remote::execute` instead of blobs).** Rejected: destroys permission-in-hash identity (source has no permission-bearing hash), forces receiver recompilation (version-skew surface), defeats caching and minimal transfer. `remote::execute` survives only as the explicitly separate admin/REPL tool (§4.1.3).
- **R6 — Don't hash `frame_descriptor`; re-derive it receiver-side from instructions.** Rejected: re-derivation means the receiver runs kind inference over untrusted bytecode — a second implementation of the compiler's proof that will drift (parallel-implementation defection attractor). Identity data belongs in the hash (§4.8).
- **R7 — Keep the silent full-program fallback when blob lookup is ambiguous (`build_call_request`, `remote.rs:1065-1073`).** Rejected: silent fallback is how minimal transfer quietly dies ("works, just 40 MB per call"). Ambiguity becomes an error naming the colliding functions; hash-based resolution is canonical.
- **R8 — Receiver program cache keyed by the shipped `program_hash` (rmp hash of the full `BytecodeProgram`).** Rejected: that hash isn't content-addressed identity (it covers serialization incidentals, not the linked closure) and trusting it means trusting the sender's claim of "same program". Cache keys on the verified entry `FunctionHash` (§4.3-6).
- **R9 — Trust the sender's `required_permissions` / skip receiver-side relink+union.** Rejected outright: the whole point of permission-in-hash is that the receiver verifies. Trusting sender claims reduces the three-tier model to a comment.
- **R10 — `CURRENT_PROGRAM` thread-local (finish wiring the existing dead plumbing).** Rejected: full `BytecodeProgram` clone per dispatch, invisible data flow, and a footgun in async contexts. The builtin gets program access through the `ModuleContext` `ContentBlobSupplier` callback (§4.1.1 — an earlier draft of this bullet said "execution context", which is a different, legacy type; `ModuleContext` is the actual builtin-dispatch seam, `module_exports.rs:104`).
- **R11 — Mutable captures ship with write-back (a distributed cell protocol).** Rejected for v1: it is cross-node reference coherence, a non-goal per the 2026-05-29 ruling scope. Refusal with a clear message beats silent copy-divergence.
- **R12 — QUIC as the v1 blessed transport.** Deferred, not rejected (§4.7, OQ-4): existing serve stack, framing, and tests are TCP; QUIC's embedded tokio runtime inside synchronous builtins (`quic.rs` `runtime.block_on`) is an extra risk axis. TLS-on-TCP delivers the security property with less motion.
- **R13 — Ship nested-closure captures in v1 (the previous draft's §4.4 row).** Rejected for v1: the minimal-transfer unit is the *static* blob closure (`build_minimal_blobs_by_hash` BFS over `blob.dependencies`), and a closure captured as a runtime *value* references a function whose blob is not statically reachable from the entry blob — shipping it correctly requires runtime-value-driven blob discovery plus recursive per-closure capture kind-tracks. Refused with a remediation (hoist to a named top-level function, which *is* a static dependency) rather than shipped half-right. Revisit when runtime-value-driven discovery is designed; the previous draft shipped this row while §4.1.2b refused it — the contradiction resolves to refusal (T8d probes it).
- **R14 — Disclose the server's granted permission set in the `PermissionDenied` reply.** Rejected: it would let any (possibly unauthenticated) caller enumerate the server's security configuration by sending one refused call. The wire payload carries only `missing`; the sender message renders from that alone; the operator sees the granted set in the server-side log (§4.6 logging contract). An earlier UX sample rendered "server grants […]" — that sample was not producible from the payload and is corrected.

No alternative on this list was "kept as a small fallback". Each is either fully rejected or explicitly deferred with an owner (OQ / WF reference).

---

## 6. Implementation plan sketch (mergeable stages)

Mapped to `docs/cluster-audits/fix-plan-2026-07-05-workflows.md`. Each stage lands green through `verify-merge` + `just check-clean` + blast-radius diff independently.

**Stage 0 — WF-1D `security-wiring` (prerequisite, already planned):** granted-set + ScopeConstraints derivation from serve config / shape.toml; `ResourceLimits` on serve paths; kill `let _sandbox` (`serve_cmd.rs:430`); reserve `Ffi` permission variant. *This design consumes Stage 0; it does not duplicate it.*

**Stage 1 — WF-2C `Fix` pipeline (5 sites, ordered):**
1. **Transport addressing:** `ToSocketAddrs` in `tcp.rs` send/connect; `NetScoped` hostname matching; book examples un-broken. (Smallest, fully independent.)
2. **Receiver de-panic + permission enforcement:** remote path stops calling `load_program`; fallible link with accumulated `MissingBlob` → `MissingModuleFunction{missing_blobs}`; `load_linked_program_with_permissions(linked, &granted)` with `PermissionDenied` mapping; `ResourceLimits` VM construction. New `RemoteErrorKind` variants land here (additive enum tail).
3. **`remote::call` real implementation (named, non-closure subset) — compiler-feature scale, not stub-fix scale:** compiler-recognized elaboration per §4.1.1/OQ-9 (call-site positional type-check against `fn_ref`'s declared params, TypedObject `_0.._n` pack lowering, `R` instantiation from the declared return type); **per-arity** typed native registration (`register_typed_fn_3`-class with declared `NativeKind`s — **not** the variadic registration path, which at HEAD is the Bool-default forbidden shape, `marshal.rs:2284-2300`); the new typed `FromSlot` function-ref carrier (§4.1.1 — resolves `Ptr(HeapKind::Closure)` → `function_id` → content hash); program/blob access via the `ModuleContext` `ContentBlobSupplier` callback (§4.1.1 — **not** "execution context", **not** `CURRENT_PROGRAM`, which is deleted); hash-canonical callee resolution; per-field `slot_to_serializable`; stdlib signature reconciled; ambiguity-fallback → error (R7). **Hard prerequisite: WF-1B's variadic/module-marshal kind-threading fix** (`comptime-excellence.md` §2.2-A) — landing this builtin before that fix routes dispatch through the existing kind-flatten at `modules.rs:715-716` and is refused (§4.1.1).
4. **`@remote` end-to-end** over 3 (depends on WF-1B's kinded annotation carrier **and** the runtime-hook `ctx.target` accessor — specified by comptime-excellence.md §4.1.5, ratified OQ-12; sequence after WF-1B merge). The `--mode jit` legs of the `@remote` acceptance probes additionally gate on **WF-1A(c)** before/after-hook JIT parity (§7 carve-out; Stage 3 edge below).
5. **Negotiation + caches:** `handle_call` takes `ConnectionState` for real; blob verify-on-receive (`HashMismatch`); per-connection + process-wide caches; sender retry-once loop; `LinkedProgram` cache with permission epoch.

**Stage 2 — closures/upvalues (WF-2C, after Stage 1.3):** the hash-input change does **not** land here — `capture_kinds` + `frame_descriptor` enter `FunctionBlobHashInput` in the **WF-2A stage-0 hash-stabilization window**, batched with the polyglot A6 blob-format change (§4.8: one doc-set-wide hash-breaking event, one commit). Stage 2 proper: `upvalue_kinds` wire field; receiver capture materialization into the §2.7.8 cell kind track; refusal matrix (§4.4, incl. the nested-closure refusal, R13). Blocked on OQ-2/OQ-6 rulings.

**Stage 3 — WF-2C `E2E` + missing tests:** the audit-flagged permission-in-hash and permission-union unit tests; enforcement/tamper/negotiation e2e probes (§7); failure-modes matrix tests; book chapter `adv-distributed` refresh. **Prerequisite edge for the jit legs:** WF-1A(c) (annotation-hook JIT parity) gates the `--mode jit` legs of T5/T6/T12/T16; until it merges, the close gate runs those probes vm-only and records their jit legs as *blocked-on-WF-1A(c)* — never as passed (§7 carve-out).

**Stage 4 — transport security:** TLS-on-TCP per OQ-4 ruling; non-loopback auth-token refusal. Can proceed parallel to Stage 2/3.

**Stage 5 — WF-2F seam:** foreign-dependency blobs, extension resolution, Ffi union/enforcement — per `polyglot-distributed-integration.md`, out of scope here.

---

## 7. Acceptance tests

All e2e probes run the sender under **both** `--mode vm` and `--mode jit` (fix-plan rule 8); receiver runs its default tiering. "e2e" = real `shape serve` on 127.0.0.1 + a real client process, not library-level calls.

**JIT carve-out (declared here, not discovered mid-implementation):** `@remote` rides the annotation before-hook short-circuit, whose JIT parity is WF-1A(c) territory (`comptime-excellence.md` §7 P9) — at HEAD, before/after hooks are silently dropped whenever calling code JIT-compiles (audit §1 Q1(b)). Until WF-1A(c) merges, the `--mode jit` legs of the `@remote`-path probes (T5, T6, T12, T16) are **blocked-on-WF-1A(c)**: the Stage 3 close gate runs them vm-only and records the jit legs as blocked, and they become required the moment WF-1A(c) lands (§6 Stage 3 prerequisite edge). T7 (elaborated `remote::call` — no annotation hooks involved) runs vm+jit from the start.

**Unit (the audit-flagged missing tests):**
- **T1 permission-in-hash:** two `FunctionBlob`s, byte-identical except `required_permissions` (`{}` vs `{FsWrite}`) ⇒ `compute_hash()` differs. (Directly tests `content_addressed.rs:114` claim.)
- **T2 permission-union:** program A→B→C where only C needs `NetConnect` and only B needs `FsRead` ⇒ `link(...)` yields `total_required_permissions == {NetConnect, FsRead}`; entry blob's own set stays minimal. Transitivity asserted through two hops.
- **T3 hash-tamper:** mutate one instruction in a received blob, keep the claimed hash ⇒ receiver verify rejects with `HashMismatch`; blob absent from cache afterward.
- **T4 missing-blob structured:** strip one dependency blob from a request ⇒ `CallResponse(Err{kind: MissingModuleFunction, missing_blobs: [that hash]})`; server process still alive (no panic).

**E2E (vm + jit):**
- **T5 `@remote` happy path:** `@remote fn add(a: int, b: int) -> int`; assert result AND (via server-side log/counter) that execution happened remotely, not locally.
- **T6 mixed-kind args:** `fn f(a: int, b: number, c: string, d: bool, e: Array<int>) -> string` through `@remote` — exercises per-arg kind marshal for every scalar family + a heap kind.
- **T7 `remote::call` direct** (the public name, §4.1.1/OQ-11) with an explicit fn ref; return type inference asserted at compile time (assigning to a wrongly-typed binding is a compile error).
- **T8 closure transfer:** immutable captures `(int, string, TypedObject)` execute remotely with correct values; **T8b refusal:** `let mut` capture ⇒ `UnsupportedCapture` naming the capture **variable** + remediation (never a slot index — §4.4 legibility rule); **T8c:** kind-track absent (hand-built request with `upvalues` but no `upvalue_kinds`) ⇒ structured `ArgumentError`, never Bool-default execution; **T8d nested-closure refusal:** a capture that is itself a closure ⇒ comptime error at annotation/elaboration time when the callee is a named declaration, `UnsupportedCapture` naming the capture variable at call time for a runtime closure value (§4.4, R13).
- **T9 permission refusal e2e:** blob requiring `FsWrite` sent to a server granting `{NetConnect, Time}` ⇒ `PermissionDenied` listing `fs.write`; server-side file untouched (negative assertion). Inverse: server granting `FsWrite` executes it.
- **T10 negotiation/cache:** two calls on one connection ⇒ second `Call` carries zero blobs (wire capture asserts payload size drop); result identical.
- **T11 retry-once:** receiver with cold cache + stripped request ⇒ sender transparently resupplies and succeeds; exactly 2 Call messages on the wire.
- **T12 hostname:** `@remote("localhost:PORT")` works (the audit's `invalid socket address syntax` probe flipped).
- **T13 ResourceLimits:** infinite-loop function remotely ⇒ `ResourceLimitExceeded` within configured wall time; server alive and serving next request.
- **T14 return-kind cross-check:** hand-built request whose blob descriptor declares `Int64` return against a callee returning `Float64` ⇒ structured wire `ArgumentError` (call-ABI-mismatch class, §4.2) surfacing sender-side as `Protocol`, no reinterpretation (asserts the `remote.rs:877-888` guard end-to-end through the §4.9 mapping).
- **T15 version skew:** client sends `call_format: 999` ⇒ `VersionSkew`; `remote::ping` reports `wire_protocol` usable for diagnosis.
- **T16 book gate:** `adv-distributed` chunk 8/15 → 15/15 (fix-plan WF-2C close gate). **Scope note:** the failing examples count against *this* gate only insofar as they exercise `@remote`/`remote::call`/`shape serve`; any that are whole-VM snapshot/state examples belong to `snapshot-resume.md`'s gate. The binding close condition here is "every distributed-transfer example in the chunk green", with chunk-level 15/15 as the joint WF-2C target across both designs.

**Regression guards:** `just check-no-dynamic` + sentinel `no_dynamic.rs` stay green (no forbidden symbols introduced); `test_remote_function_call_over_tcp` (`serve_cmd.rs:1110`) keeps passing unchanged (the marshal template is not touched).

---

## 8. Open questions for the user

**Ratification record (2026-07-05):** all recommended defaults below were ratified by the user (consolidated as `00-priority-spine-overview.md` §3, Q26–Q35 + Q1/Q4). OQ-3's bind-class split was confirmed as part of the Q15/Q28/Q52 permission trio (loopback ⇒ `sandboxed()` + moderate set; non-loopback ⇒ Pure-only until configured; serve `ffi_languages` strict-empty per the integration doc). No override touches this doc; OQ-11 and OQ-12 carry their rulings inline below.

1. **OQ-1 — `@remote` failure surface.** Recommended: transport/remote failures raise as ordinary runtime errors, and the annotated function's own `Result<T,E>` (if any) passes through untouched; users wanting transport-error handling call `remote::call` directly — it returns `Result<R, RemoteError>` with the structured variant set of §4.1.1 (match on variants, never parse messages) — or build their own annotation over it. Alternative: `@remote` *requires* the annotated function to return `Result<T, RemoteError>` and folds transport errors into it (more explicit, more intrusive). Which? *(Restated against the current contract: an earlier draft of this question referenced `__call` returning `Result<R, string>` — both superseded by §4.1.1.)*
2. **OQ-2 — mutable captures.** Recommended v1: refuse (`UnsupportedCapture`) with a remediation hint. Alternative: allow with documented send-copy divergence (remote writes lost). Refuse or copy?
3. **OQ-3 — default granted set for `shape serve` + permission-surface UX.** Recommended: loopback ⇒ `sandboxed()` resource limits + a moderate permission set (`Time`, `Random`, `NetConnect`?); non-loopback ⇒ Pure-only until `[permissions]` explicitly configured. What is the exact default allowlist per bind class, and should there be a `--grant` CLI flag mirroring shape.toml? **Also folded into this question's UX scope (§4.1.2b):** should the compile-time permission-surface disclosure on `@remote` targets be a `comptime warning` listing the target's transitive `required_permissions` (recommended — deployment surprises move to build time), or silent?
4. **OQ-4 — blessed cross-host transport.** Recommended: TLS-on-TCP (tokio-rustls) as v1; QUIC stays feature-gated; non-loopback binds refuse without TLS *and* require `--auth-token` (warning → hard refusal). Ratify, or prefer QUIC-first / keep auth-token optional?
5. **OQ-5 — cache sharing scope.** Recommended: process-wide verified-blob + LinkedProgram LRU shared across connections (bounded, permission-epoch keyed). Alternative: per-connection only (simpler, re-transfers per connection). Any persistence-to-disk ambition (a `~/.shape/blob-cache`) now, or later?
6. **OQ-6 — the doc-set's single hash break.** Including `frame_descriptor` + new `capture_kinds` in `FunctionBlobHashInput` changes every content hash (pre-1.0; no known persisted blob stores). Scheduling per §4.8: batched into the **WF-2A stage-0 hash-stabilization window** together with `polyglot-distributed-integration.md`'s A6 blob-format change — **one** invalidation event and one commit for the entire ratified doc set, not one per doc (an earlier draft scheduled this doc's break separately in Stage 2, which would have made two events; corrected). Approve the batched one-time break?
7. **OQ-7 — `remote::execute` book positioning.** Retained as a separate source-shipping tool with an explicit "not the distributed-computing path" callout (§4.1.3). Ratify retention, or deprecate toward `@remote`-only?
8. **OQ-8 — non-idempotent retry policy.** Design retries **only** on `MissingModuleFunction` (provably pre-execution). Transport timeouts and post-send connection loss (`Timeout` / `ConnectionLost` — may-have-executed, §4.1.1) never auto-retry. Should a future `@remote(idempotent: true)` opt-in enable such retries, or is that userland-annotation territory?
9. **OQ-9 — `remote::call` compiler elaboration.** The primitive is compiler-recognized (§4.1.1 — the same special-casing class as `as`-casts → `__into_*`): call-site positional type-check against `fn_ref`'s declared params, lowering of `args` to the TypedObject `_0.._n` pack carrier (the supervisor-D1 tuple carrier), `R` instantiated from the declared return type, and the same elaboration running *inside per-site specialized annotation handlers* for `__call_raising` (§4.1.2). Also folded in: verifying (and landing, if absent) comptime *evaluation* in annotation-argument position, so `@remote(build_config(...))` works before the book teaches it (§4.1.2). Ratify the elaboration approach? (The alternative — a plain non-elaborated stdlib signature — is not typeable in strict Shape per §4.1.1's grammar analysis, so rejecting elaboration means rejecting the typed primitive.)
10. **OQ-10 — runtime-expression addresses for `@remote`.** v1: annotation arguments accept comptime-evaluable expressions only; `@remote(build_config("WORKER_ADDR"))` is the blessed deployment form, and runtime addressing goes through `remote::call` (its `addr` is an ordinary runtime `string`). Should `@remote` ever accept runtime-expression addresses, or is comptime-only the permanent contract?
11. **OQ-11 — public naming: `__call` → `remote::call`.** Retire `__call` from the public surface (the `__` prefix is this codebase's internal-machinery convention, and this is documented user API); the compiler-elaborated internal sibling keeps its dunder (`__call_raising`, never documented). Ratify the rename?
    **→ RATIFIED 2026-07-05.** The public primitive is `remote::call`; `__call` is retired from the public surface; the internal compiler-elaborated sibling remains `__call_raising`. The recorded blast radius — the integration doc's residual `remote::__call` sender-flow references — was renamed in the same ratification pass.
12. **OQ-12 — runtime-hook typed target accessor (`ctx.target`).** §4.1.2's before-hook needs a typed accessor for the annotated function inside RUNTIME before/after hooks — replacing the `ctx["__impl"] ?? args[0]` latent bug (`remote.shape:99-100`) with **no fallback**. This design requires WF-1B to specify the runtime-hook accessor (surface name, descriptor type, and its compile-time resolution inside specialized handlers) before §6 Stage 1.4 can start. Ratify: (a) WF-1B owns this specification as a named deliverable, and (b) `ctx.target` as the proposed surface name?
    **→ RESOLVED 2026-07-05, jointly with comptime §4.1.5 — (a) and (b) both ratified.** `comptime-excellence.md` §4.1.5 IS the named WF-1B deliverable: it specifies the runtime-hook contract under exactly the surface name `ctx.target` (typed function value statically bound in the specialized handler) with the specialization-time kinded args pack; its runtime-hook `ctx` v1 is `{module_path, file}` (`build` dropped, its OQ5 — an earlier draft of this entry cited a stale `{module_path, file, build}` shape, corrected per overview §4.1).
