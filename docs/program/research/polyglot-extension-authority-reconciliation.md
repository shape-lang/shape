# Polyglot implementation and extension-authority reconciliation

**Question:** Against the product constraints ratified by
[Map: Production data and polyglot execution excellence](https://github.com/shape-lang/shape/issues/40),
what does the current system actually provide, and which seams should be
retained, deepened, amended, deleted, or superseded?

**Scope:** Planning evidence only. This report describes current production
code, authoritative decisions, open work, and published-documentation evidence.
It does not propose an implementation.

## Resolution

The current polyglot stack is not empty or disposable. It already has a useful
extension ABI, declaration-level typed contracts, real Python and TypeScript
runtime adapters, structured dynamic-call outcomes, negotiated Python
zero-copy buffers, asynchronous offload, content-addressed function blobs, and
receiver-owned permission checks. It additionally carries a complete but
**unreachable** foreign-reference lifetime seam, which §4 classifies as staged
rather than live.

Its central defect is **authority placement**. ADR-019 and the implementation
built from it make Shape core/tooling the authority for:

- the `[foreign.<language>]` environment schema;
- checker identity and settings;
- Python environment layout;
- the canonical cross-language marshalling table;
- which TypeScript module forms are supported; and
- when and how the foreign checker participates.

The map instead fixes a language-runtime extension as the exclusive authority
for its body checking, native package ecosystem, environment construction,
language-specific marshalling, and execution. Shape owns the typed seam,
permissions, portable identity, admission, and user-visible outcome. These are
incompatible allocations of responsibility. They must not coexist as two
layers.

The route is therefore:

1. retain the valuable Shape-owned typed, permission, lifecycle, and identity
   seams;
2. deepen the extension interface where it already carries real variability;
3. amend ADR-019 where its desired guarantees remain right but its authority is
   wrong; and
4. supersede ambient or core-owned environment, packaging, marshalling, and
   remote-admission shapes with the contracts being decided by the map's
   children.

## Evidence discipline

- **Live production code** means a non-test path reachable from the CLI,
  compiler, VM, extension loader, or extension implementation.
- **Live but conditional** means production code requiring an installed shared
  library or an explicit capability.
- **Staged** means code or fields deliberately present but not wired.
- **Design-only** means an ADR, issue, patch, test-only construct, or ignored /
  self-skipping evidence. It is not reported as shipped behavior.
- ADR-019 is formally **Proposed**, not accepted
  ([ADR-019 lines 3-16](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L3)),
  even though its implementation amendments for stubs, buffers, environments,
  foreign references, and async are in production code
  ([ADR-019 lines 120-169](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L120),
  [lines 230-300](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L230)).
  The implementation is evidence; “Proposed” is not permission to treat its
  authority split as ratified.

## Classification summary

| Surface | Current seam | Evidence state | Classification |
|---|---|---|---|
| Dynamic foreign call | `fn <language>` declaration → bytecode entry → language-runtime vtable | Live | **Retain and deepen** the typed call seam; **amend** core-owned foreign representation decisions |
| Extension loading | Path/global scan → `dlopen` → ABI/fingerprint/capability gates | Live | **Retain** ABI negotiation; **supersede** ambient discovery/install/trust |
| Package environment | Core `[foreign.<language>]` + core lock schema/digest + host root check | Live, hash join staged | **Supersede** authority; retain content identity and fail-closed outcomes as requirements |
| Marshalling | Core `ForeignType` table + MessagePack + extension conversions | Live | **Amend** the Shape typed interface; **supersede** core as language-specific marshalling authority |
| Zero-copy buffer | ABI capability block + `shared` numeric arrays + Python buffer adapter | Live for Python; refused for TypeScript | **Retain and deepen** capability/borrow/refusal seam; **amend** the narrow language-specific surface |
| Foreign references | Opaque host carrier + extension disposer + snapshot/remote refusal | Staged: machinery complete and unit-tested, but no production path mints one; remote re-establishment absent | **Retain and deepen** |
| Native compute kernels | Generic `CapabilityKind::Compute`; unrelated internal kernel modules | Label live, dedicated interface absent | **Delete as a claimed contract** and **supersede** with the dedicated kernel capability |
| Portable artifact | Full `BytecodeProgram` plus optional content-addressed function blobs | Live; verified persistence and environment join incomplete | **Retain** verified blob/identity work; **supersede** the aggregate with Remote-Ready Artifact |
| Loopback/remote admission | Receiver permissions + preinstalled runtimes + program/blob verification | Live but incomplete; polyglot evidence conditionally self-skips | **Retain and deepen** receiver authority; **supersede** language-name/preinstallation admission |

## 1. Dynamic foreign calls

### What is live

The extension ABI has a real `LanguageRuntimeVTable` whose interface includes
initialization, contract delivery, compile, MessagePack invoke, handle disposal,
language identity, LSP configuration, error model, runtime descriptor, state
model, generated stubs, concurrency, foreign-reference disposal, and optional
capabilities
([shape-abi-v1 lines 742-839](../../../crates/shape-abi-v1/src/lib.rs#L742),
[line 872](../../../crates/shape-abi-v1/src/lib.rs#L872),
[lines 880-987](../../../crates/shape-abi-v1/src/lib.rs#L880)).

For the dynamic forms, the compiler requires explicit types, requires
dynamic-language returns to be `Result<T>`, and rejects types outside the
current foreign table at declaration time
([functions.rs lines 341-380](../../../crates/shape-ast/src/ast/functions.rs#L341),
[lines 438-492](../../../crates/shape-ast/src/ast/functions.rs#L438)).
`extern C fn` is exempt from that last check — the unmapped-type scan
early-returns for the native ABI
([functions.rs lines 385-387](../../../crates/shape-ast/src/ast/functions.rs#L385))
— and is scoped by `ffi_libraries` / `ffi_symbols` rather than by the
`ForeignType` table.
At first link, the VM checks a declared-environment refusal, looks up the
receiver's runtime, delivers the whole declared contract once, asks the
extension to compile the body, and caches the handle
([control_flow lines 972-1055](../../../crates/shape-vm/src/executor/control_flow/mod.rs#L972)).
The same contract builder is shared by VM and JIT
([foreign_contract lines 1-20](../../../crates/shape-vm/src/executor/control_flow/foreign_contract.rs#L1)).

Foreign exceptions become the user's `Err`; nonconforming returns become
`Err("TypeConformanceError: ...")`; host marshal gaps remain VM errors
([foreign_marshal lines 1019-1063](../../../crates/shape-vm/src/executor/control_flow/foreign_marshal.rs#L1019)).
Async foreign calls use extension-declared concurrency and either shared
offload or extension-instance-affine worker pools
([foreign_async lines 1-32](../../../crates/shape-vm/src/executor/foreign_async.rs#L1),
[lines 314-338](../../../crates/shape-vm/src/executor/foreign_async.rs#L314)).
TypeScript's offload is structurally live but cannot perform IO: bare
`deno_core` ships no timers and no `fetch`
([#214](https://github.com/shape-lang/shape/issues/214)).

### Classification

**Retain** `fn <language>` as the domain-neutral declaration, Shape-declared
types, `Result<T>` as the visible dynamic outcome, receiver permissions,
extension-declared concurrency, and VM/JIT contract parity.

**Deepen** the language-runtime extension seam: body checking, compilation,
execution, and language-specific conversion are real variability already
represented by two adapters. The interface should hide more of that variability
from core, not expose it through more language switches.

**Amend** the current typed contract so Shape remains authority for the Shape
type and ownership facts without remaining authority for Python/TypeScript
representations. The exact contract belongs to
[Define the dynamic foreign-call and marshalling contract](https://github.com/shape-lang/shape/issues/242).

## 2. Extension loading, packaging, trust, and lifecycle

### What is live

`PluginLoader::load` opens a native shared library and explicitly warns that
loading executes arbitrary code. Before dispatch it requires an ABI version and
a structural layout fingerprint
([loader lines 94-218](../../../crates/shape-runtime/src/plugins/loader.rs#L94)).
The provider registry then instantiates capabilities and indexes language
runtimes by the extension-declared language id
([provider_registry lines 217-275](../../../crates/shape-runtime/src/provider_registry.rs#L217)).

Discovery is not project-pinned or content-addressed. The CLI merges paths from
project TOML, frontmatter, a config file, an extension directory, CLI flags,
and an automatic scan of `~/.shape/extensions`
([extension_loading lines 67-176](../../../bin/shape-cli/src/extension_loading.rs#L67)).
`shape ext install` accepts a wildcard version by default, generates a temporary
Cargo project, runs `cargo build --release`, and copies the resulting shared
library into that global directory
([ext_cmd lines 17-98](../../../bin/shape-cli/src/commands/ext_cmd.rs#L17)).
There is no signature, trust-root, isolation-class, target-artifact, or
revocation admission in that path.

### Classification

**Retain** the small ABI-version/fingerprint/capability interface and the
language-id lookup. These are useful fail-closed load gates and a real seam with
multiple adapters.

**Supersede** global scanning, wildcard source builds, path precedence, and
“loaded means trusted in-process” as the production packaging/trust model.
ABI compatibility is not artifact identity or trust.

The complete replacement authority is being decided by
[Define pinned extension packaging, trust, isolation, and lifecycle](https://github.com/shape-lang/shape/issues/244).
No compatibility shim for ambient pre-production discovery is required by the
map.

## 3. Foreign package environments

### What is live and staged

Core parses a fixed `[foreign.<language>]` schema containing runtime, version,
lockfile, root, and checker pin
([foreign_env lines 48-118](../../../crates/shape-runtime/src/project/foreign_env.rs#L48),
[project_config lines 54-84](../../../crates/shape-runtime/src/project/project_config.rs#L54)).
Core owns the lockfile model and derives a domain-separated digest from runtime,
version, lock hash, and checker settings
([foreign_env lines 219-307](../../../crates/shape-runtime/src/project/foreign_env.rs#L219)).
It also hardcodes CPython's `lib/pythonX.Y/site-packages` layout
([foreign_env lines 348-403](../../../crates/shape-runtime/src/project/foreign_env.rs#L348)).

The CLI binds every declared environment before extension initialization, sends
the whole provided-environment map to every extension, and installs core-owned
refusals for unavailable roots
([extension_loading lines 179-218](../../../bin/shape-cli/src/extension_loading.rs#L179),
[module_loading lines 28-65](../../../bin/shape-cli/src/module_loading.rs#L28)).
An undeclared language is currently allowed to run against the base interpreter;
ADR-019 records this narrower behavior
([ADR-019 lines 270-277](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L270)).

The environment digest does **not** currently identify a compiled foreign
function. `ForeignFunctionEntry.env_digest` is explicitly staged and unset
([core_types lines 93-118](../../../crates/shape-vm/src/bytecode/core_types.rs#L93),
[functions_foreign lines 281-295](../../../crates/shape-vm/src/compiler/functions_foreign.rs#L281)).
The committed Book patch states this limitation honestly
([declared-environments patch lines 150-156](../workstreams/book-patches/198-declared-environments.patch#L150)).

### ADR-019 authority conflict

ADR-019 says a host `ForeignToolchainProvider` runs pyright/tsc and that a
core-defined per-language lockfile pins the checker
([ADR-019 lines 52-87](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L52)).
It says environment construction is “toolchain-owned” and derives the
environment from `shape.toml` language tables
([ADR-019 lines 197-228](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L197)).
Its implementation amendment deliberately keeps this readable independently
of the loaded runtime extension
([ADR-019 lines 237-243](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L237)).

The map's authority is different: the extension consumes its native ecosystem
inputs, owns environment construction and body checking, and produces a
content-addressed Foreign Environment Artifact. Shape tracks and admits that
artifact without defining Python, TypeScript, `uv`, pyright, tsc, virtualenv, or
module-loader semantics.

### Classification

**Supersede**, do not layer beside, the core-owned `[foreign.<language>]`
schema, checker pin, lockfile schema, CPython path construction, and
TypeScript-specific module rules.

**Retain as requirements**, not necessarily as the current module interface:
content-derived identity, canonical reproducibility, no ambient fallback,
fail-closed provisioning, actionable mismatch diagnostics, and compiler/LSP
agreement.

The deciding ticket is
[Define extension-owned foreign environments and package artifacts](https://github.com/shape-lang/shape/issues/243).

## 4. Marshalling and foreign references

### What is live

Core owns `ForeignType`, a closed table of scalars, scalar arrays, scalar maps,
optionals, and objects. It classifies declarations, drives VM MessagePack
projection, and is handed to extensions for `.pyi` / `.d.ts` rendering
([foreign_types lines 1-35](../../../crates/shape-abi-v1/src/foreign_types.rs#L1),
[lines 198-259](../../../crates/shape-abi-v1/src/foreign_types.rs#L198)).
Outbound scalar arrays are walked element-by-element under the copied path
([foreign_marshal lines 127-195](../../../crates/shape-vm/src/executor/control_flow/foreign_marshal.rs#L127)).
Inbound values are checked against the core-classified declared type
([foreign_marshal lines 530-580](../../../crates/shape-vm/src/executor/control_flow/foreign_marshal.rs#L530))
— except a named-type object return, which passes the declaration gate and
then dies at first call for want of a registered `schema_id`
([foreign_marshal lines 700-707](../../../crates/shape-vm/src/executor/control_flow/foreign_marshal.rs#L700),
[#207](https://github.com/shape-lang/shape/issues/207)).
Each extension then performs another MessagePack-to-language conversion.

The opaque foreign-reference carrier, its disposer contract, the
mint-without-disposer refusal, and worker-addressed disposal of thread-affine
references are all built and unit-tested
([language_runtime lines 443-468](../../../crates/shape-runtime/src/plugins/language_runtime.rs#L443),
[foreign_async lines 88-107](../../../crates/shape-vm/src/executor/foreign_async.rs#L88)).

They are also **unreachable**, on three independent legs. `mint_foreign_ref`
has no non-test caller — all eight call sites sit inside the `#[cfg(test)]`
module that opens at
[foreign_ref line 200](../../../crates/shape-vm/src/executor/foreign_ref.rs#L200).
The `language_runtime_plugin!` macro hardcodes `dispose_ref: None`
([shape-abi-v1 line 2379](../../../crates/shape-abi-v1/src/lib.rs#L2379)), so
every extension built with it — including both shipped adapters, neither of
which mentions the symbol — reports `can_dispose_refs() == false` and would be
refused a handle anyway. And `ForeignType` carries no reference variant
([foreign_types lines 198-230](../../../crates/shape-abi-v1/src/foreign_types.rs#L198)),
so no declaration can name one in the first place. Under this report's own
evidence discipline that is **staged**, not live. They are not serializable
handles and no remote re-establishment protocol is live.

### Classification

**Retain** Shape's authority over declared Shape types, ownership/borrows,
permission effects, portable admissibility, and the final visible
`Ok`/foreign-error/type-conformance/host-failure distinction.

**Supersede** the assumption that one core `ForeignType`/MessagePack table is
the language-specific marshalling authority. It is a shallow seam: core knows
representation details while extensions repeat the conversion. The map assigns
that conversion to the extension.

**Retain and deepen** the opaque foreign-reference lifecycle seam — as a
design, since nothing in production exercises it today. Local identity,
disposer ownership, snapshot refusal, and remote non-serialization are sound
constraints, so the first task under the owning contract is reachability (a
`ForeignType` reference variant and a disposer an extension can actually
declare), not more machinery. Remote re-establishment remains an explicit
future contract, not an implicit handle copy.

## 5. Zero-copy buffers

### What is live

The ABI has a versioned, size-guarded capability block. The host refuses unknown,
truncated, missing-invoke, or missing-release-accounting capabilities
([language_runtime lines 196-259](../../../crates/shape-runtime/src/plugins/language_runtime.rs#L196)).
`shared` and `shared mut` are explicit source modes; only `Array<int>` and
`Array<number>` qualify
([foreign_types lines 88-107](../../../crates/shape-abi-v1/src/foreign_types.rs#L88),
[lines 112-181](../../../crates/shape-abi-v1/src/foreign_types.rs#L112)).
The host checks capabilities and aliasing before exposing pointers, invokes,
then gives retained-view accounting priority over the foreign result
([control_flow lines 1194-1249](../../../crates/shape-vm/src/executor/control_flow/mod.rs#L1194)).

Python supplies the live buffer adapter. TypeScript explicitly offers no buffer
capability because release accounting is absent
([ADR-019 lines 145-169](../../adr/019-polyglot-depth-and-foreign-toolchain-integration.md#L145)).

### Classification

**Retain and deepen** the negotiated capability seam, explicit borrow intent,
aliasing/lifetime rules, release accounting, and refusal rather than silent
copy. This is a deep module candidate: callers should learn one typed borrowed
buffer interface while adapters hide runtime-specific views.

**Amend** the current Python-shaped, array-only interface under the dynamic
foreign-call contract. Do not conflate it with the separate Shape-owned
columnar/batch data plane required for native compute kernels.

## 6. Native compute kernels

### What is live

The public ABI contains only a generic `CapabilityKind::Compute` enum value
([shape-abi-v1 line 94](../../../crates/shape-abi-v1/src/lib.rs#L94)).
No dedicated compute-kernel vtable, typed discovery record, borrowed
columnar/batch descriptor, output builder, concurrency contract, cancellation
contract, or target-specialization interface is attached to it.

`shape-jit`'s `kernel_ir.rs` and runtime matrix intrinsics are internal
implementations, not extension adapters satisfying a common kernel interface.
They do not establish the map's kernel seam.

### Classification

**Delete as a claimed product contract** the inference that the generic enum
label constitutes native-kernel support.

**Supersede** it with the dedicated interface decided by
[Define the native compute-kernel extension ABI](https://github.com/shape-lang/shape/issues/245).
The common extension discovery/lifecycle envelope is retained; the kernel data
plane is a distinct capability, not dynamic-language MessagePack marshalling.

## 7. Artifacts and loopback/remote admission

### What is live

`RemoteCallRequest` carries a full `BytecodeProgram`, function identity,
arguments, schemas, program hash, and optional transitive function blobs. It
does not carry exact extension artifact, extension ABI contract, execution
class, target, Foreign Environment Artifact, or kernel requirements
([remote lines 50-116](../../../crates/shape-vm/src/remote.rs#L50)).

The receiver correctly owns permissions, validates content-addressed blobs,
enforces a strict dynamic-language allow-list, and injects its own preinstalled
language runtimes
([remote lines 1023-1057](../../../crates/shape-vm/src/remote.rs#L1023)).
This is real remote execution, but admission is “receiver has some runtime for
this language and allows it,” not exact artifact materialization.

The Shape-owned half of that admission is stronger than the aggregate suggests.
`Permission::Ffi` is derived at compile time and stamped into the extern-stub
blob's `required_permissions`
([functions_foreign lines 465-478](../../../crates/shape-vm/src/compiler/functions_foreign.rs#L465)),
so it rides the linker's transitive union into the content hash: two otherwise
identical programs, one calling foreign code, hash differently. It then gates
at load — the Deterministic-mode refusal fires before a single instruction runs,
and fires even when `Ffi` itself is granted
([program lines 5-20](../../../crates/shape-vm/src/executor/vm_impl/program.rs#L5)) —
and at call, where presence plus `ffi_languages` scope are checked on every
call before link-now
([control_flow lines 1533-1545](../../../crates/shape-vm/src/executor/control_flow/mod.rs#L1533)).
The call-time gate is envelope-conditional by ratified posture: trusted-local
`shape run` installs no envelope and grants `Ffi` unscoped. This is the concrete
basis for the retention below.

The polyglot-over-TCP tests prove remote execution when extension shared
libraries are provisioned, but they return early when the `.so` is absent.
The default test gate can therefore pass without exercising polyglot transport
([serve_cmd lines 2568-2585](../../../bin/shape-cli/src/commands/serve_cmd.rs#L2568)).
That is **conditional evidence**, not clean-receiver admission evidence.

Verified artifact persistence remains open in
[Persist one verified lowered artifact](https://github.com/shape-lang/shape/issues/160),
and exact dynamic-provider admission remains open in
[Admit exact dynamic providers remotely](https://github.com/shape-lang/shape/issues/167).
Those issues predate the map and do not by themselves decide the new
extension-owned artifact authority.

### Classification

**Retain** content hashes, transitive blob verification, receiver-owned
permissions, strict language opt-in, and fail-closed refusal.

**Deepen** receiver admission so the interface consumes exact portable
requirements rather than inspecting whatever runtimes happen to be installed.

**Supersede** the full-program request as the map's final artifact and
language-name/preinstallation as sufficient foreign admission. The deciding
contract is
[Define Remote-Ready Artifact and clean-receiver admission](https://github.com/shape-lang/shape/issues/246).
Real transport, placement, scheduling, exchange, and recovery remain outside
this map.

## 8. Published documentation and evidence truth

The published Book is unusually explicit about two current limits:
compile-time foreign-body checking is not wired, and the environment digest
does not yet enter foreign-function identity
([Polyglot Functions lines 521-537](https://github.com/shape-lang/shape-web/blob/3016b627f3b683c3dee05780ed001d8a52be2e02/book/book-site/src/content/docs/tooling/polyglot.mdx#L521-L537),
[lines 670-701](https://github.com/shape-lang/shape-web/blob/3016b627f3b683c3dee05780ed001d8a52be2e02/book/book-site/src/content/docs/tooling/polyglot.mdx#L670-L701)).
Those caveats agree with production code.

Two broader public claims do not survive this reconciliation:

- “Nothing about Python (or any language) is hardcoded in Shape's core”
  ([Polyglot Functions lines 629-638](https://github.com/shape-lang/shape-web/blob/3016b627f3b683c3dee05780ed001d8a52be2e02/book/book-site/src/content/docs/tooling/polyglot.mdx#L629-L638))
  conflicts with the core-owned environment schema, checker pin, CPython path
  construction, and TypeScript module policy above.
- The extension system is documented as exclusively for language runtimes,
  globally installed and auto-detected, with `shape.language_runtime` as its
  only supported contract
  ([Extensions lines 8-11](https://github.com/shape-lang/shape-web/blob/3016b627f3b683c3dee05780ed001d8a52be2e02/book/book-site/src/content/docs/tooling/extensions.mdx#L8-L11),
  [lines 26-81](https://github.com/shape-lang/shape-web/blob/3016b627f3b683c3dee05780ed001d8a52be2e02/book/book-site/src/content/docs/tooling/extensions.mdx#L26-L81)).
  That is current CLI behavior, but it conflicts with the map's common pinned
  extension envelope and dedicated compute-kernel capability.

The distributed Book's “genuine” Python/TypeScript matrix requires a built
extension already present on the receiver
([Polyglot + Distributed lines 35-77](https://github.com/shape-lang/shape-web/blob/3016b627f3b683c3dee05780ed001d8a52be2e02/book/book-site/src/content/docs/advanced/polyglot-distributed.mdx#L35-L77)).
That is a truthful transport claim under its stated prerequisite, not evidence
of clean-receiver artifact materialization. The release-evidence contract must
keep those claims distinct.

### Classification

**Amend** the two over-broad architecture/install claims when the owning
contracts resolve. **Retain** the existing explicit caveats and the
preprovisioned-runtime transport examples, but do not promote them into
extension-authority or clean-admission evidence.

## Decision implications for the remaining map

The research makes the already-open children precise; it does not require a new
implementation ticket:

- [Define the dynamic foreign-call and marshalling contract](https://github.com/shape-lang/shape/issues/242)
  must distinguish Shape-owned typed conformance/outcomes from
  extension-owned language representations.
- [Define extension-owned foreign environments and package artifacts](https://github.com/shape-lang/shape/issues/243)
  must explicitly supersede ADR-019 §1/§4's core/toolchain authority, while
  preserving reproducibility and fail-closed evidence requirements.
- [Define pinned extension packaging, trust, isolation, and lifecycle](https://github.com/shape-lang/shape/issues/244)
  must replace ambient global scanning and wildcard source builds.
- [Define the native compute-kernel extension ABI](https://github.com/shape-lang/shape/issues/245)
  starts from “no dedicated current interface,” not from the generic Compute
  label.
- [Define Remote-Ready Artifact and clean-receiver admission](https://github.com/shape-lang/shape/issues/246)
  must bind exact extension, environment, kernel, target, and blob identities;
  current loopback proves execution only after out-of-band provisioning.

One follow-on decision is now sharp enough to record in the map's fog or as a
future ticket: **which exact ADR-019 clauses are retained, amended, or
superseded after the four owning contracts close, and what single authoritative
document replaces its conflicting authority statements?** That editorial
reconciliation should follow those decisions; doing it first would guess their
answers.
