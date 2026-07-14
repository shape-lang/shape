# Wave 40R: Execution ABI Binding For Transferred Artifacts

Date: 2026-07-10

Scope: read-only audit of current function/program artifacts, canonical hashes,
remote negotiation and caches, linker reconstruction, nested closures,
snapshots, package formats, and extension ABI gates. This is a clean-break
artifact design, not an implementation claim.

## Recommendation

Do not put only a hand-maintained Shape version integer on `FunctionBlob`.
Carry one hash-covered execution binding on every function artifact:

```text
ExecutionBinding {
    artifact_format: u32,
    abi_epoch: u32,
    execution_abi_id: [u8; 32],
    required_capabilities: sorted Vec<CapabilityId>,
}
```

`artifact_format` selects the canonical decoder. `abi_epoch` is concise
diagnostic and migration routing metadata. `execution_abi_id` is the exact
compatibility authority. The capability set permits optional, additive engine
features without pretending that a receiver supports them merely because its
base ABI matches.

Derive `execution_abi_id` from a canonical semantic descriptor containing both
generated structural facts and manually reviewed semantic revisions. Do not
carry a separate Rust-layout fingerprint: `size_of`/`offset_of` is right for an
in-process C vtable but neither portable nor sufficient for bytecode semantics.

All four fields are inside `FunctionHash`. A receiver accepts an artifact only
when format is supported, ABI ID exactly matches the selected session ABI, and
the receiver satisfies every required capability. There is no best-effort
execution, semver inference, opcode translation, or silent downgrade.

## Which Signal Does What

| Candidate | Decision | Reason |
|---|---|---|
| Shape package semver / `shape_version` | diagnostic only | Too coarse; development builds with different semantics can share it. |
| Integer ABI version | retain as `abi_epoch`, never authoritative | Useful in errors and migration dispatch, but a forgotten bump silently aliases. |
| Exact semantic ABI ID | required and authoritative | Binds decoded bytes to one execution meaning. |
| Structural fingerprint | fold into ABI-ID derivation | Standalone layout facts miss behavioral changes; Rust memory layout is irrelevant to portable bytecode. |
| Required capability set | required per artifact | Expresses optional opcodes/protocol features without changing unchanged base semantics. |
| Compiler git/dirty fingerprint | local freshness only | Correct for local compile-cache invalidation, too build-specific for portable artifacts. |
| Extension `.so` ABI/version/hash | node-local gate, not function ABI | The executing host must load an extension compatible with itself; logical extension requirements travel separately. |

## Current Artifact Identity

### FunctionBlob

`FunctionBlob` is a directly serde-encoded struct
(`crates/shape-vm/src/bytecode/content_addressed.rs:33-119`). Its canonical
hash projection (`:122-152`) covers name, arity, parameter names, locals,
closure/async/ref metadata, mutable captures, frame descriptor, capture kinds,
instructions, constants, strings, dependency hashes, type-schema names,
sorted permissions, and foreign dependency hashes. `compute_hash` uses
struct-as-array MessagePack then SHA-256 (`:157-192`).

This is strong call-ABI coverage, but it has no domain tag, format revision,
execution ABI, or engine capability requirement. Serde field/enum ordering and
the meanings of opcodes, builtins, `NativeKind`, `HeapKind`, constants, frame
slots, and dependency ordinals are therefore ambient facts supplied by the
receiving binary.

`content_hash` is also duplicated inside the payload but excluded from its own
hash. Receipt checks recomputed bytes against the map key only
(`remote.rs:1063-1078`), while the linker later indexes and records
`blob.content_hash` (`linker.rs:368,514,642`). A packet can therefore carry
`map_key == recomputed_hash != blob.content_hash`. The clean format removes the
embedded duplicate; the envelope key is the sole hash identity.

`capture_names` and `source_map` are non-hash diagnostics, which is valid.
`callee_names` is `serde(skip)` yet helps the linker distinguish mutual
recursion from self recursion. Combined with `FunctionHash::ZERO` cycle
sentinels, it is semantic metadata outside the transferred identity. A clean
format needs explicit, hash-covered `Self`/SCC-member dependency references,
not an absent name fallback.

### Program and linked metadata

There is no `ProgramBlob` or canonical program-root artifact. `Program` is a
serde container around `function_store` plus program metadata
(`content_addressed.rs:198-413`); `LinkedProgram` is an execution assembly
(`:437-660`). Both contain `serde(skip)` side tables. Some are optimization
hints, but closure layouts and several JIT surface-and-stop/dispatch facts can
affect correct execution. `program_from_blobs_by_hash` copies what survived in
the stub source program and otherwise defaults it (`remote.rs:752-805`).

The current `program_hash` is SHA-256 of a serde-encoded `BytecodeProgram`
(`remote.rs:1882-1891`). It includes serialization incidentals, excludes
`serde(skip)` facts, has no verified claim check, and is not used by the
receiver. It is not a program artifact identity.

Type-schema registry contents, trait dispatch data, foreign entries, and native
layout tables travel beside function blobs in the stub/full program. A
function hash names schema strings and foreign entry hashes, but the current
root does not hash-bind all companion object bytes to the entry closure. ABI
binding alone cannot repair that provenance gap.

## Current Transfer And Validation Path

`RemoteCallRequest` directly embeds a `BytecodeProgram`, optional
`Vec<(FunctionHash, FunctionBlob)>`, entry hash, arguments, and capture values
(`crates/shape-vm/src/remote.rs:56-116`). The sender walks static dependencies,
`ClosureAlloc` targets, and module-binding function targets to form a minimal
closure (`remote.rs:638-722`). Nested closure blobs therefore transfer
transitively today.

Negotiation offers only bare `FunctionHash` values (`remote.rs:377-387`).
`RemoteBlobCache` is keyed only by that hash and exposes unchecked insertion
(`:548-635`). The production serve path does recompute every supplied blob
before cache insertion and then hydrates dependencies from its connection cache
(`bin/shape-cli/src/commands/serve_cmd.rs:1186-1226`). The reusable library
`handle_wire_message` inserts before verification (`remote.rs:2346-2355`).

The disk `BlobCache` deserializes a file at a bare-hash path without recomputing
the hash, and `JitCodeCache` is also keyed only by `FunctionHash`
(`blob_cache_v2.rs:10-137`). None is namespaced by artifact format, execution
ABI, target architecture, or JIT backend as appropriate.

`run_remote_call` reconstructs `Program` before integrity checking and validates
direct closure captures before the blob hash loop (`remote.rs:997-1022,
1063-1078`). It then checks missing dependencies, links, enforces permissions,
and validates call-frame kinds. The linker itself accepts an ordinary
`Program`; it has no verified-artifact token or ABI check
(`linker.rs:337-381`).

If content-addressed metadata is absent, the full `BytecodeProgram` fallback is
loaded without an artifact hash or ABI gate (`remote.rs:1139-1144`).

### Protocol fields

`shape-wire` declares `WIRE_PROTOCOL_V1/V2`; V2 means additional message types,
not bytecode semantics (`crates/shape-wire/src/lib.rs:51-60`). `ServerInfo`
reports `shape_version`, one `wire_protocol` integer, and string capabilities
only after an optional Ping (`remote.rs:507-514`). Calls do not require Ping or
perform a handshake.

There is no `call_format` field in `RemoteCallRequest`; the only code references
are stale comments around `RemoteErrorKind`. `VersionSkew` exists but has no
producer. A wire-enum decode mismatch occurs before the server can return that
kind. Rustls `with_safe_default_protocol_versions()` selects TLS versions and
is unrelated to Shape artifact compatibility.

## Nested Closure Status

The immutable transferred-closure lane is a useful model. Hash-covered
`is_closure`, capture count, mutable flags, frame descriptor, and capture kinds
are enough to reconstruct zero-capture and immutable by-value layouts. The
fallible reconstruction validates cross-field agreement and refuses mutable,
reference, shared-cell, nested-closure, and resource captures
(`bytecode/closure_layout_fallback.rs:48-180`).

The new binding applies independently to every nested closure blob. A named
parent and all `ClosureAlloc` descendants must have the same base ABI. Layout
reconstruction occurs only after artifact verification. Future mutable/shared
closure formats require a new hash-covered descriptor and capability or a new
ABI ID; they must not weaken current refusal by inferring storage from bits.

## Existing Versioning Precedents

The extension loader checks required `ABI_VERSION == 4`, then a structural
`abi_build_fingerprint()` before dereferencing plugin data or vtables
(`shape-abi-v1/src/lib.rs:1570-1669`; `shape-runtime/src/plugins/loader.rs:104-222`).
That two-part fail-closed order is correct for a same-process `repr(C)` seam.
The artifact analogue keeps the integer but replaces memory-layout identity
with a semantic descriptor hash.

`SHAPEPKG` places magic plus `FORMAT_VERSION` before its MessagePack payload and
rejects unsupported versions before decode
(`shape-runtime/src/package_bundle.rs:11-25,235-274`). This is the right outer
envelope shape. Its `compiler_version` and the VM's git/dirty compiler
fingerprint remain freshness/diagnostic facts, not execution compatibility.

## Clean Artifact Contract

Use a stable outer envelope whose header can be read before decoding the
versioned payload:

```text
FunctionArtifactEnvelope {
    object_class: Function,
    artifact_format: u32,
    abi_epoch: u32,
    execution_abi_id: [u8; 32],
    required_capabilities: sorted Vec<CapabilityId>,
    payload: canonical bytes for FunctionPayload(format),
}

FunctionHash = SHA256(
    "shape.function-artifact\0" || canonical(header_without_hash, payload)
)
```

The wire/store key carries `(artifact_format, execution_abi_id, FunctionHash)`
so a receiver selects a decoder and cache namespace before touching payload
bytes. The first two are cryptographically redundant after verification but
operationally necessary before verification. `abi_epoch` is also hash-covered.

`CapabilityId` is a canonical semantic feature ID, not a free-form advertised
string or permission. IDs are exact and revisioned by identity, for example
`exec.closure.immutable-layout.v1`; a host may advertise multiple revisions.
Authority such as `FsWrite` remains the independent hash-covered permission and
receiver-policy gate.

### Execution ABI ID derivation

Generate the ID from one reviewed `ExecutionAbiDescriptor` containing:

1. ABI epoch and descriptor schema revision.
2. Opcode numeric values, operand forms, stack/call semantics, and builtin IDs.
3. `Constant`, `NativeKind`, and `HeapKind` serialized tags plus carrier,
   ownership, clone/drop, and null semantics.
4. Frame, parameter, ref, return, closure capture, and suspension conventions.
5. Dependency/recursion and foreign-ordinal linker rules.
6. VM/JIT call, deopt, host-boundary marshal, and failure-channel revisions.
7. Generated decoded payload-field/tag inventory wherever it affects execution.

Generated tables catch structural drift; explicit semantic revision constants
cover behavior that Rust layout cannot reveal. A golden descriptor test fails
when either changes without intentional ABI review. `CARGO_PKG_VERSION`, git
SHA, build timestamp, target triple, and optimization profile are excluded.

Portable bytecode semantics must be target-independent. Any target-dependent
fact still exposed by `IntSize`/`UIntSize`, pointer width, native layout, or AOT
code is an explicit target capability or a separate native artifact class,
never an ambient assumption hidden behind the same ABI ID.

Changing compression or an outer wire codec changes wire/artifact format, not
execution semantics. Changing an existing opcode, kind, frame convention, or
link rule changes `execution_abi_id`. A genuinely additive optional feature
uses a new capability ID only when all pre-existing semantics remain intact.

### Program root and companions

Do not serialize in-memory `Program` as the canonical artifact. Introduce a
hash-covered root manifest:

```text
ProgramArtifactManifest {
    base_binding: { artifact_format, abi_epoch, execution_abi_id },
    entry: FunctionHash,
    functions: sorted Vec<FunctionHash>,
    companions: sorted Vec<{ object_class, content_hash }>,
    required_capabilities: recomputed union,
    required_permissions: recomputed union,
}
```

Companions include canonical schema registry, trait-dispatch recipes, foreign
entry objects, native layouts, and any future cleanup/effect/layout metadata
that cannot be reconstructed from function payloads. Every execution-relevant
fact must be either hash-covered in a function/companion or deterministically
reconstructed from verified facts. `serde(skip)` may remain only for proven
optimization caches or diagnostics whose absence cannot change results.

The manifest content hash, domain-separated as `shape.program-artifact`, is the
program root. It covers entry and companions, unlike the current
`CodeManifest.program_root_hash`, which hashes only the sorted blob list
(`shape-runtime/src/snapshot.rs:349-399`). Debug/source maps can be separate
attachments keyed to a function hash and may vary without changing execution.

## Single Verification Seam

Create one deep verifier module used by remote calls, disk caches, snapshots,
bundles, hot reload, and the linker. Its external interface is conceptually:

```text
verify_program(root, supplied_objects, verified_cache, host_contract)
    -> VerifiedProgramArtifacts | ArtifactError
```

Only `VerifiedFunctionArtifact` enters a cache, and only
`VerifiedProgramArtifacts` enters the linker. This removes the current public
unchecked cache insertion and ordinary-`Program` network path.

Validation order is normative:

1. Enforce transport framing/size limits and authentication.
2. Decode the stable Call/artifact headers; require the session wire/call
   schema and supported `artifact_format`.
3. Require exact session `abi_epoch`/`execution_abi_id` before selecting the
   payload decoder. Do not attempt translation.
4. Resolve exact-key cache hits. For misses, decode canonically, reject unknown
   fields/tags for that format, and require canonical re-encoding.
5. Recompute each domain-separated hash and compare its claimed key. Never
   cache a mismatch or trust a duplicate in-payload hash.
6. Validate local structure: bounds, arity/count lockstep, frame/capture facts,
   opcode operands, dependency references, foreign ordinals, and object class.
7. Walk from the entry; require complete companions/dependencies, one base ABI
   across the closure, explicit valid recursion references, and no conflicting
   duplicate key. Recompute program root, capability union, and permissions.
8. Check receiver execution capabilities, logical extension/provider
   requirements, then permissions/scopes before dlopen or execution.
9. Link verified artifacts, validate call argument/return kinds, and execute.

Individual artifacts may enter the verified byte cache after step 6 even when
the closure is incomplete or this node lacks a capability. Linked-program cache
insertion waits until step 9. Error classes stay distinct:
`UnsupportedArtifactFormat`, `ExecutionAbiMismatch`, `HashMismatch`,
`InvalidArtifact`, `MixedExecutionAbi`, `MissingArtifact`,
`CapabilityUnavailable`, and `PermissionDenied`.

## Transitive Rules

1. Entry, every reachable function including nested closures, and every
   execution companion use one exact base binding. Mixed-ABI linking is
   forbidden even if the host supports both ABIs independently.
2. Per-function capabilities are direct requirements. The verifier recomputes
   their transitive union; callers need not duplicate callee requirements.
3. Dependency hashes bind dependency bytes, which bind their ABI and
   capabilities. Missing/resupply operates on full `ArtifactKey`, never an
   unqualified 32-byte hash.
4. Recursive edges use explicit hash-covered self/SCC member references. ZERO
   plus skipped callee names is not valid canonical artifact state.
5. Companion references are typed by object class, so a foreign-entry hash
   cannot satisfy a function or schema request.
6. A receiver never merges side tables from a local program with a remote
   closure unless their program root and ABI binding are identical.

## Handshake Relationship

With no compatibility constraint, make `Hello`/`Accept` mandatory before
BlobNegotiation or Call. Negotiate wire protocol, call schema, artifact format,
one exact execution ABI ID, codec, and limits; advertise the receiver's
execution capability set after the appropriate authentication policy.

The handshake is an optimization and routing gate, not artifact attestation.
Every call still carries or references its self-binding artifact root and runs
the verifier. A compromised peer cannot turn Hello claims into cache entries.
Connections select one execution ABI; pools route artifacts to a matching
connection rather than downgrade them.

Wire protocol describes framing/messages. Call schema describes Call fields.
Execution ABI describes bytecode meaning. Keep all three distinct. Retire
`program_hash`; do not introduce `call_format` as a substitute for execution
ABI. `Ping` remains diagnostics and can report supported ABI IDs/capabilities,
but is not the mandatory gate.

## Cache Contract

Use `ArtifactKey { object_class, artifact_format, execution_abi_id,
content_hash }`. Disk layout is namespaced accordingly, for example
`objects/function/f1/<abi-id>/<hash>.blob`. Reads reverify before promotion to
memory; writes are atomic and occur only for verified canonical bytes.

Blob negotiation offers full keys and reports only exact verified hits. A bare
hash from another ABI is never `known`. A one-time resupply may hydrate only
objects requested by typed key.

Linked cache keys add the verified program root, link-environment fingerprint
(schema/extension/provider generation), and permission-policy epoch. JIT cache
keys additionally include target triple, CPU feature set, JIT backend ABI, and
optimization tier. Portable bytecode may cross machines; native code may not.

Because ABI and format are hash-covered, upgrades naturally mint new hashes.
The explicit namespace still prevents pre-verification decode under the wrong
schema and permits deterministic eviction of obsolete generations.

## Snapshot Contract

`SNAPSHOT_VERSION == 7` currently gates state serialization only
(`shape-runtime/src/snapshot.rs:86-116,209-218`). `CodeManifest` carries bare
blob hashes and permissions but no execution binding or capability set; VM
construction currently supplies `entry: None` and derives the permission list
from grants (`executor/vm_impl/init.rs:242-260`). No current reader reconstructs
code from the manifest.

The snapshot manifest must carry `ProgramArtifactManifest` by hash plus the
exact execution binding. Capture persists verified canonical function and
companion objects before the snapshot envelope. Resume order is:

1. Validate snapshot container version and envelope hash.
2. Load and hash-verify the program manifest.
3. Require exact execution ABI and capabilities before decoding VM/frame state.
4. Fetch and verify the complete artifact graph and receiver policy/provider
   requirements.
5. Relocate frames by verified function hash, then restore values/tasks.

Snapshot version and execution ABI are independent gates. A state-format match
does not authorize code from another ABI. Legacy snapshots without a binding
are refused under this clean break. `bytecode_hash` cannot be a fallback
authority. Snapshot restore never recompiles source and assumes equivalence.

## Extension ABI Relationship

The local extension loader continues to require its integer ABI and
`repr(C)` fingerprint before advertising an extension. Function artifacts do
not carry `shape_abi_v1::ABI_VERSION` or a `.so` hash. They carry the core
execution ABI plus logical, hash-covered foreign-entry and extension/provider
requirements. The receiving node satisfies those only with locally loaded
extensions that passed its own ABI gate.

Extension name/version constraints belong in typed manifest capability records,
not in `FunctionHash` merely because one node happens to have that patch
installed. The core ABI descriptor does include the host-to-extension marshal
contract revision. This preserves cross-platform artifacts without weakening
the node-local unsafe ABI check.

## Upgrade And Migration Policy

No compatibility means one coordinated break:

1. Introduce artifact format 1, ABI epoch 1, and the first semantic ID.
2. Add the binding and domain separator to canonical hashing; every function,
   dependency, program root, cache key, and JIT key changes once.
3. Replace direct blob serde on Call with the stable envelope and mandatory
   handshake; remove the full unversioned `BytecodeProgram` Call fallback.
4. Bump `SNAPSHOT_VERSION`, `SHAPEPKG` format, and wire/call schema where they
   embed or reference executable artifacts.
5. Flush legacy caches, recompile bundles/source, and recapture snapshots.

Mixed clusters run separate old/new listener pools and cache namespaces while
draining; they never cross-execute. Source recompilation is the canonical
migration. A future artifact migrator must decode with an explicit old decoder,
emit and verify new canonical artifacts with new hashes, and cannot preserve
old identity. Suspended snapshots require a separately proved frame/state
migration; otherwise they refuse cleanly.

Format-only changes may retain the semantic ABI ID if decoded meaning is exact.
Changes to established execution meaning mint a new ID. Additive isolated
features mint capability IDs only. There is no range-based ABI acceptance in
the first contract.

## Bounded Proof Matrix

Focused hash/descriptor proofs:

1. A golden execution descriptor is deterministic across debug/release and
   target hosts; changing an opcode, kind tag, frame rule, or semantic revision
   changes the ID.
2. Mutating format, epoch, ABI ID, capability set, permission, or semantic
   payload changes `FunctionHash`; debug attachment changes do not.
3. Canonically equivalent builds produce byte-identical artifacts and roots;
   map insertion order cannot affect them.

Verifier/cache proofs:

1. Unknown format and ABI mismatch fail before payload/link/closure handling.
2. Tampered payload, wrong object class, malformed counts/ordinals, companion
   substitution, and mixed-ABI dependency closure fail without cache insertion.
3. Same payload/hash text under another ABI namespace is not a negotiation hit;
   tampered disk entries are evicted after re-verification.
4. Self and mutual recursion resolve through explicit canonical references;
   missing typed objects are reported in one bounded resupply response.

Execution composition proofs:

1. Existing real-socket nested zero-capture closure transfer, stripped
   resupply, negotiation, and zero-blob reuse succeeds under one ABI; changing
   only the nested closure ABI fails before link. Mutable/reference/resource
   refusals remain unchanged.
2. Same wire protocol plus different execution ABI fails at Hello; matching
   Hello followed by a hash mismatch still fails integrity verification.
3. Missing capability and permission produce distinct pre-execution errors.
4. VM/JIT run the same verified root; JIT cache misses across target/ABI/backend.
5. Snapshot same-ABI resume succeeds; ABI mismatch, missing capability, legacy
   unbound manifest, and companion tamper fail before VM state restoration.
6. A locally ABI-stale extension is refused by the loader even when the core
   artifact matches; a locally compatible provider satisfying the same logical
   requirement can execute the portable artifact.

## Bounded Landing Order

1. Add portable binding/key/capability types and the generated semantic
   descriptor; freeze golden ID tests.
2. Introduce canonical function envelopes, remove the embedded hash duplicate,
   and perform the coordinated rehash.
3. Add the single verifier and verified-only cache/linker interfaces.
4. Add canonical program/companion manifests and explicit recursion refs.
5. Replace Call/negotiation with mandatory Hello and full artifact keys.
6. Namespace disk/JIT/linked caches and migrate snapshot/package envelopes.
7. Add focused verifier tests, then extend the existing real-socket and
   snapshot matrices.

No production, test, book-site, script, `CONTEXT.md`, or `AGENTS.md` file was
edited, and no cargo, just, test, build, extraction, or book-truth command ran.

## Changed File

`docs/cluster-audits/wave40-execution-abi-binding.md`
