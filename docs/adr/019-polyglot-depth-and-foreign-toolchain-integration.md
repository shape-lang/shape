# ADR-019: Polyglot Depth and Foreign Toolchain Integration

## Status

Proposed 2026-07-27 (pending ratification).

Composes with ADR-011 through ADR-016. ADR-012 §5 owns the target-adapter
matrix and its v1 acceptances/rejections; ADR-013 owns the tracked-provider
mechanism this ADR builds on; ADR-014 owns the `Ffi` effect. This ADR deepens
the foreign story from a correct boundary into a checked, shareable,
reproducible one. It changes exactly one adapter-matrix cell, by paired
amendment in ADR-012 §5: Python/TypeScript async/suspend becomes
`reject in v1` (§5 below), because the shipped blocking implementation
misrepresents the declared contract; the cell's target state is unchanged.
Every other cell is untouched, and the matrix remains the single authority
for target support.

## Context

Shape's inline polyglot surface (`fn python name(...)`, `fn typescript
name(...)`, `extern "C" fn`) has no real peer among typed languages. The
current implementation, however, treats the boundary as declared-and-trusted
rather than checked, copied rather than shared, and ambient rather than
reproducible:

- the foreign body is an opaque string end to end; the compiler never
  inspects it, and the vtable's `register_types` channel — designed for stub
  exchange — is a stub on both sides with no host caller;
- every value crosses as MessagePack by deep copy; `TypedArray<f64>` is
  walked element-by-element, and the Arrow bridge is a two-function stub;
- no foreign-ref carrier exists: Shape cannot hold a Python or TypeScript
  object at all;
- Python package dependencies have no declaration surface; the interpreter is
  whatever pyo3 `auto-initialize` found at build time; package resolution is
  runtime virtualenv sniffing with a silent no-op fallback; TypeScript has no
  module loader, so `import` in a body cannot resolve;
- `ForeignFunctionEntry.content_hash` covers (language, body, param types,
  return type) and therefore claims a determinism the environment does not
  provide — directly undercutting the content-addressed distributed story;
- `async fn python`/`typescript` compiles to a blocking synchronous call
  (fresh event loop or `block_on` per invocation) while presenting itself as
  async — an untruthful contract under ADR-011 §7.

Meanwhile the language server already proves the hardest sub-problem: it
generates virtual foreign documents with bidirectional position mapping,
spawns pyright/typescript-language-server children, and relays their
diagnostics — but LSP-only, invisible to compilation, which is exactly the
dual-authority split ADR-011 §6 forbids.

## Decision

### 1. Foreign contracts are checked at compile time through tracked toolchain providers

Foreign body checking becomes a compile-stage fact produced through one
`ForeignToolchainProvider` per language, registered with `ComptimeHost`
(ADR-013 §4):

- the host calls the extension's stub channel (`register_types`, made
  load-bearing) to obtain real interface stubs (`.pyi`, `.d.ts`) generated
  from the declared Shape contract and exposed schemas;
- the provider runs the foreign checker (pyright, tsc) over the inline body
  plus stubs and returns structured diagnostics;
- diagnostics map into the fence through the same bidirectional position
  mapping the LSP pipeline already implements; compiler and LSP consume one
  semantic fact per ADR-011 §6 — the current LSP-only diagnostic path is
  migration debt, promoted, not duplicated;
- provider inputs are tracked: toolchain identity and version, body digest,
  stub digest, and environment digest (§4). Results are memoizable and
  reproducible under ADR-013's rules; a changed toolchain or body reruns
  exactly the dependent checks.

A declared-contract mismatch reported by the foreign checker is a compile
error by default. A host without the foreign toolchain gets a structured
degradation diagnostic in local development, but release artifacts and remote
placement require the check evidence, mirroring ADR-013's
reproducible-snapshot rule for external inputs. The foreign checker never
becomes semantic authority over Shape types: it checks the foreign side of a
contract whose Shape side ADR-011/012 already own.

The checker itself is pinned, not merely tracked: the per-language lockfile
(§4) records the checker's identity, version, and settings digest (strict
level, language-version target), and those enter `ForeignEnvironmentDigest`
alongside the interpreter and package facts. Tracking alone would make
drift detectable but not reproducible — identical source and lockfile must
produce the same verdict on every host, and a checker upgrade is a lockfile
change reviewed like any dependency bump, never an ambient source-free
build break.

### 2. Buffer sharing is a declared, versioned vtable capability

Zero-copy interop is added as a new versioned capability in the extension
vtable (the reserved ABI tail is the designated slot), negotiated at load
time, never assumed:

- a `TypedArray<T>` parameter may be declared shared: exported to Python as a
  buffer-protocol view and to TypeScript as an `ArrayBuffer` view over the
  same memory;
- ownership follows ADR-006: a shared export is an immutable shared-borrow
  view; a mutable export is an exclusive-borrow view; both are call-scoped.
  The buffer is pinned for the invocation and released on return;
- sharing moves the memory-safety boundary from extension authors (a
  handful of trusted native crates) to **anyone who writes an inline
  fence**: foreign code can stash a view — `numpy.asarray(buf)` appended to
  a module global needs no vtable re-entry — and a later buffer
  reallocation then corrupts memory from an ordinary-looking Shape source
  file. This ADR names that class rather than hiding it in "trusted
  extension" framing. Accordingly: sharing requires an explicit per-
  parameter `shared` / `shared mut` spelling in the Shape declaration — the
  visible acknowledgement of the widened boundary — and retention checking
  is as strong as each runtime allows, not "where feasible": for Python the
  host verifies via the buffer protocol's export count that every view was
  released before the call returns, and an unreleased export is a
  structured runtime failure at the boundary, not undefined behavior later.
  Where a runtime offers no release accounting, the mode is refused for
  that language rather than silently weakened;
- deep copy remains the default; sharing is opt-in per parameter in the
  declared adapter contract, and the element-wise MessagePack walk is
  replaced for bulk native arrays in both modes.

#### 2.1 Implementation amendment — the capability block (#199, 2026-07-28)

The §2 design says "the reserved ABI tail is the designated slot". At
implementation time exactly ONE reserved fn-pointer slot remained in
`LanguageRuntimeVTable` (#196, #202 and #200 having taken the other three),
and buffer sharing needs two entry points by itself: the invoke that carries
views, and the release-accounting query §2 requires. Spending the last slot
on either would have left the other needing an ABI version bump, and every
capability after it one apiece.

**Decision:** the slot becomes `capabilities` — an accessor returning a
versioned, size-guarded `ExtensionCapabilities` block. Each block's first
field is its own `struct_size`, and a reader touches a field only when that
size covers it, so later capabilities are appended to the block rather than
to the vtable. `#198`'s environment facts and anything after them grow the
same way.

The alternative considered was taking the ABI version bump, which the
no-users ruling makes cheap. It was rejected because it only postpones the
same question one slot-set later, and because `abi_build_fingerprint` — which
folds this struct's layout to catch silent skew — keeps its meaning only
while the vtable stops changing. The block's offset and the fingerprint are
unchanged by this amendment, so an extension built before #199 reads as
"offers no optional capabilities" rather than being mis-dispatched.

Four scope decisions taken with it, each narrower than §2's text and each
for a reason §2 already implies:

1. **The shareable element set is `int` and `number`.** `Array<bool>` is one
   byte per element with only 0 and 1 valid, so a writable view could store a
   value that is neither `true` nor `false` — a soundness hole, not a missing
   projection. `Array<string>` holds host pointers with host refcounts, so
   sharing the buffer would export the heap. Both are refused at the
   declaration with those reasons, not deferred.
2. **`shared` and `async` do not compose.** A view is valid for the duration
   of the call; an offloaded call (§5) returns to Shape immediately and runs
   the body on a worker, so the caller's last share of the array can be
   dropped while foreign code still holds the pointer. Composition needs a pin
   that outlives the interpreter frame, which is a separate design. Until
   then the combination is refused at the call, naming both ways out.
3. **Aliasing is checked by address at the call.** ADR-006 makes a mutable
   view an exclusive borrow, and Shape's borrow solver does not see through a
   foreign call's argument list, so passing one array to two shared positions
   where either is mutable is refused before any pointer is exported.
4. **TypeScript declares no buffer capability.** Its ArrayBuffer path has no
   release accounting wired, and §2 fixes that case as refusal rather than a
   weaker guarantee. The `.d.ts` renderer types a shared parameter `never`,
   so a TypeScript body that names one fails tsc — the editor-time echo of the
   runtime refusal. Wiring deno's external backing store with accounting is a
   later slice; the refusal is the truthful state until it lands.

### 3. Foreign references are a first-class opaque carrier

Shape gains a foreign-ref carrier: an opaque, refcounted handle to a foreign
object, held as a typed heap value under the ADR-005/ADR-006 single-
discriminator discipline (a dedicated `HeapKind` with a typed `Arc` payload
binding instance, handle, and disposer — the pure-discriminator pattern the
`FilterExpr` precedent already establishes; the 2026-07-05 design
ratification green-lit this carrier). Rules:

- drop routes through the owning extension instance's dispose entry, under
  ordinary ADR-010 teardown authority. Dispose is synchronous and
  infallible in v1 (a foreign disposer that can fail or suspend is a later
  design under ADR-010 §6's finalization contract, not a v1 surface);
- every operation through a foreign ref contributes `Ffi` (ADR-014 §1) —
  including the compiler-inserted drop, so a callable that merely holds a
  foreign ref to end of scope carries `Ffi` in its row. That is stated
  here because it will surprise users: holding is calling, at drop time;
- foreign state is `STATE_MODEL_STATEFUL_OPAQUE`: a snapshot containing a
  live foreign ref refuses with a structured diagnostic naming the value and
  its origin — never a silent skip and never a fabricated restoration
  (ADR-015 §8's refusal discipline applies);
- a foreign ref crossing a remote boundary is rejected at artifact
  construction unless the receiving placement admits the same extension,
  version, and environment digest, and even then only by explicit
  re-establishment protocol, never by serializing the handle.

### 4. Foreign environments are declared, locked, and content-addressed

`shape.toml` gains per-language dependency tables (Python packages,
TypeScript modules) with lockfiles, joining the existing
`[native-dependencies]` surface for C libraries. From these the toolchain
produces one `ForeignEnvironmentDigest` per language covering interpreter or
runtime identity and version, resolved package set, and lockfile hash.

- The digest is a `TrackedBuildInput` (ADR-013 §4) for every compile-stage
  foreign check, and joins `ForeignFunctionEntry`'s content hash: the
  current hash claims determinism the environment does not provide, and
  that defect closes here. Content addressing survives because the digest
  is derived from **declared, locked inputs** — the lockfiles and pinned
  identities — never from ambient host inspection: same source plus same
  lockfiles yields the same digest and the same hash on every host, and a
  host that cannot provide the locked environment fails pre-entry rather
  than producing a different hash. Joining the digest is a deliberate
  break of every existing foreign function's hash; it is versioned as part
  of the content-addressing compatibility domain (bytecode format bump,
  sequenced with the artifact-persistence lane), and a loading host whose
  provided environment does not match a blob's recorded digest refuses the
  blob with a version-refusal-class diagnostic — never a silent run under
  a different environment;
- the digest enters the portable artifact's exact foreign-dependency manifest
  (ADR-012 §7) and is validated by receiver admission for remote placement of
  foreign-calling continuations;
- environment construction is toolchain-owned and reproducible; the runtime
  virtualenv-sniffing fallback is deleted. A declared environment that cannot
  be provided is a structured pre-entry failure (ADR-012 §5's
  extension-absence class), never a silent fallback to ambient site-packages;
- TypeScript gains a lockfile-backed module loader; an unresolvable import is
  a compile-stage diagnostic through §1, not a runtime V8 error.

#### 4.1 Implementation amendment — declaration, refusal, and the staged join (#198, 2026-07-28)

§4's design is implemented as written, with five decisions it leaves open and
two places where this slice delivers less than its text. Each is recorded here
rather than deferred, because each is a point where a plausible alternative
would quietly reintroduce what §4 deletes.

**The surface is `[foreign.<language>]`, a typed table.** Runtime identity,
version, environment root, lockfile path, and the §1 checker pin. A named
manifest field rather than an entry in the extension-section catch-all, so the
toolchain can read a project's declared environment whether or not the
extension that will run it is loaded — a compile-stage fact must not depend on
a runtime's presence. Lockfiles are per-language TOML, `BTreeMap`-keyed
throughout, and serialize canonically.

**The lock hash is over the parsed document, not the file bytes.** A lockfile
that was reformatted or had its tables reordered resolves to the same
environment, and a digest that moved for it would put every content-addressed
artifact at a formatter's mercy — the opposite of §4's "same source plus same
lockfiles yields the same digest on every host".

**The resolved set enters the digest only through the lock hash.** §4 lists
"resolved package set, and lockfile hash" as separate contributions. Writing
both into the pre-image would leave "which one decides?" open the first time
they could disagree, so the pre-image carries the lock hash and the value
carries the resolved set for diagnostics. One authority per fact.

**The declared root does NOT enter the digest.** Where an environment sits on
this host is not what the environment IS: two checkouts at different paths must
produce the same artifact. The root is a provisioning input, consumed by
`check_provided` — the single place host inspection is legal, and one that can
only fail, never substitute.

**Search paths are constructed from the declared version, not discovered.**
`lib/python3.11/site-packages` follows from `runtime = "cpython"` and
`version = "3.11.7"`. This is why deleting the sniffer costs no capability:
the layout was always derivable from facts the manifest now states, and
guessing them was the sniffer's entire job. A root whose layout disagrees with
the declared version is a refusal, not a search.

**Refusal is `[C0936]`, and its scope is narrower than §4's text.** §4 reads
as though an undeclared environment refuses outright. This slice refuses a
DECLARED environment that cannot be provided, at link-now, before the foreign
body is compiled — and leaves an undeclared language running against the base
interpreter with no search path added, which is exactly what deleting the
sniffer leaves and is truthful. Requiring a declaration before any `fn python`
may run is a release/artifact rule and belongs to #160 and #167 with the rest
of admission; the `Undeclared` refusal variant exists for them.

**The content-hash join is staged, not landed.** §4 is explicit that joining
the digest breaks every existing foreign function's hash and that the break is
versioned with the artifact-persistence lane. So the `\0env\0` run exists in
`compute_content_hash`, gated on an `env_digest` field that no compiler path
sets; #160 wires it in one line. Two obligations transfer with it: the
bytecode-format version bump, and enriching `LinkError::MissingForeignEntry`
to name the two environment digests, so a receiver's refusal reads as an
environment disagreement rather than a bare missing hash.

**The TypeScript loader is narrower than "a module loader".** It resolves from
the declared lockfile's module table and nothing else — no `node_modules`
walk, no registry, no network — and verifies `integrity` when the lock declares
it. Three exclusions are deliberate: turning a `package.json` into a lockfile
is a resolver and a separate program; vendored sources load as JavaScript
because this runtime embeds V8 and not tsc, with a `.ts` path refused at
resolve time naming pre-transpilation as the remedy; and a top-level `import`
statement in a fence remains a syntax error because a foreign body is compiled
as a function body, so the spelling that reaches the loader is dynamic
`import()`. §4's "an unresolvable import is a compile-stage diagnostic through
§1" is therefore not yet true — the refusal is at the import — and becomes
true when POLY-FOREIGN-CHECK runs tsc over the fence with the same lock in
hand.

### 5. Foreign async is truthful: transitional rejection, then offload parity

Compiling an async declaration to a VM-thread-blocking call misrepresents
both the `Suspend` effect and the caller's concurrency model, and is
forbidden as of this ADR's ratification. The remedy is two ordered steps
(user-ratified 2026-07-27, grill Q3 = C):

1. **Transitional rejection** (days): an `async` foreign declaration
   rejects with a structured diagnostic naming the owning ticket. This is
   a flip-to-green control for step 2, not an indefinite state.
2. **Offload parity** (fast-tracked): the async foreign stub returns a
   future; the invoke runs off-thread (Python via blocking-pool GIL
   attach; TypeScript via a dedicated worker thread owning the V8 isolate,
   the documented thread-affine-instance pattern); `await` resolves it
   through the existing pending-task completion channel. Two async foreign
   calls overlap instead of serializing. This satisfies the adapter-matrix
   cell ("declared contract plus `Suspend`") at exactly the fidelity of
   Shape's own shipped async — whose `await` on in-flight work is an
   eager-offload-plus-blocking-receive, not a true interpreter suspension.

True interpreter suspension (a host scheduler resuming a suspended frame)
is a runtime-wide later item — the resume path is unimplemented and gated
on the snapshot-tier rebuild — and upgrading to it changes no declared
contract. Foreign async does not wait for it, and no foreign-specific
suspension machinery may be built ahead of the general one.

## Grounding (2026-07-27)

- Vtable and trust model: `crates/shape-abi-v1/src/lib.rs:742` (vtable,
  `reserved0..3` tail at :884), in-process dlopen loader with ABI/fingerprint
  gates at `crates/shape-runtime/src/plugins/loader.rs:12,126,179`.
- Opaque body pipeline: grammar `crates/shape-ast/src/shape.pest:301,332`;
  `ForeignFunctionDef.body_text` at `crates/shape-ast/src/ast/functions.rs:54`;
  types resolved without body inspection at
  `crates/shape-runtime/src/type_system/inference/items.rs:117`; first
  extension contact at first call,
  `crates/shape-vm/src/executor/control_flow/mod.rs:833,982`.
- Dead stub channel: `register_types` has no host caller
  (`crates/shape-runtime/src/plugins/language_runtime.rs:188`); both
  extensions stub it (`extensions/python/src/runtime.rs:145`,
  `extensions/typescript/src/runtime.rs:70`).
- Existing LSP pipeline to promote: `tools/shape-lsp/src/foreign_lsp.rs`
  (virtual docs :71–:152, child servers :502, diagnostics relay :323).
- Copy-only marshaling: element-wise `TypedArray` walk at
  `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs:145`; Arrow
  bridge stub `extensions/python/src/arrow_bridge.rs`; no foreign-ref
  carrier anywhere in `crates/` or `extensions/`.
- Environment gap: pyo3 `auto-initialize` (unpinned interpreter); venv
  sniffing with silent fallback at `extensions/python/src/runtime.rs:73`; no
  TS module loader; content hash without environment at
  `crates/shape-vm/src/bytecode/core_types.rs:64`.
- Untruthful async: `asyncio.run` per call at
  `extensions/python/src/runtime.rs:203`, `block_on` at
  `extensions/typescript/src/runtime.rs:178`; no `Future` in the declared
  Shape type.
- Per-call module re-execution: Python `invoke` re-executes the whole
  generated module on every call instead of caching the compiled module
  object (`extensions/python/src/runtime.rs:252`) — an implementation
  defect fixed alongside the stub-channel work.
- Async-offload feasibility (2026-07-27 scout): Shape's own `await` on
  in-flight work is eager offload + blocking receive
  (`vm_impl/modules.rs:611-654`, `:665-682`), not true suspension — the
  external-completion bridge (`register_external`/`external_receivers`)
  has zero production callers and `VirtualMachine::resume()` is `todo!()`
  (`call_convention.rs:369`). §5's offload design copies the shipped
  pattern; it also fixes two latent defects: foreign calls inside spawned
  user async fns fail today (`run_isolated_async_fn` never receives
  `language_runtimes`, `async_runtime.rs:102-126`), and concurrent invoke
  on one extension instance is UB-shaped (`&mut` through a `Send + Sync`
  `*mut c_void`, `extensions/typescript/src/runtime.rs:357`).

## Consequences

- A type error inside inline Python or TypeScript becomes a Shape compile
  error with an exact fence location — capability no other typed language
  offers — and it is reproducible because the toolchain and environment are
  tracked inputs.
- The polyglot and distributed stories compose truthfully: what a portable
  artifact claims about its foreign dependencies is what receiver admission
  checks.
- Bulk numeric interop stops paying a per-element serialization tax, using
  the ownership model rather than bypassing it.
- Shape programs can hold and manage foreign objects with ordinary drop
  discipline, at the cost of a new carrier under the single-discriminator
  rules and explicit snapshot refusal semantics.
- Compilation of foreign-calling code gains an optional dependency on
  foreign toolchains, made explicit and fail-closed rather than ambient.
- Existing `async` foreign declarations reject until suspension integration
  lands; that is a deliberate truthfulness regression-fix, not a feature
  removal.

## Rejected alternatives

- **Trust declared boundary types (status quo).** An unchecked declaration is
  documentation, not a contract; runtime structural checks catch only what
  execution happens to reach.
- **Embed foreign interpreters in the compiler for checking.** Providers keep
  the checkers external, versioned, tracked, and absent-tolerant; embedding
  would freeze one toolchain into the compiler and violate the ComptimeHost
  boundary.
- **Let the LSP keep its own foreign diagnostics.** Dual authority is the
  exact ADR-011 §6 defect class; the pipeline is promoted, not forked.
- **Zero-copy by default.** Sharing changes aliasing and lifetime semantics;
  it must be visible in the declared contract.
- **Serialize foreign handles in snapshots or artifacts.** Opaque foreign
  state cannot be truthfully restored; refusal with provenance is the only
  honest behavior.
- **Keep runtime venv sniffing as a fallback.** A silent environment fallback
  makes the same source mean different programs on different hosts — the
  ambient-input defect ADR-013 exists to eliminate.
- **Keep blocking async as a bridge.** A declared `async` that blocks the VM
  thread is a false contract; ADR-011 §7 does not permit it as an interim.

## Related decisions

- ADR-005 / ADR-006: single-discriminator and value/memory model
- ADR-010: Verified Region Teardown and Callable Lifecycle
- ADR-011: Resolved Semantic Identity and Typed Elaboration
- ADR-012: Verified Annotation Elaboration and Callable Transforms (§5, §7)
- ADR-013: Incremental Semantic Queries and Tracked Comptime
- ADR-014: Closed Effects and Static Capability Ownership
- ADR-016: Executable Public Feature Documentation
