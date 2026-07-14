# Wave 40H: Remoting Provider Interface

Date: 2026-07-10

Scope: read-only interface design over accepted Wave-40 semantics, current remote stdlib/runtime/wire code,
extension/provider patterns, and distributed design docs. This report is the only file changed. No cargo,
just, build, test, extraction, or book command was run.

## Decision

Replace every public remote address string with an opaque destination made by a named provider:

```text
opaque RemoteDestination<P: RemotingProvider>
opaque Placement<P: RemotingProvider>
```

Use one deep asynchronous provider/session boundary. A provider owns destination decoding, discovery,
routing, physical address encoding, transport, authentication, codec selection, protocol negotiation,
connection reuse, and provider-specific telemetry. The runtime owns the semantic invocation, typed
signature and `ArgumentPack<Sig>`, function/schema identity, content hashes, permissions, deadline ceiling,
cancellation ownership, response validation, failure cause plus execution certainty, and final
`RemoteOutcome<R>`.

A provider does not return `R`, `RemoteError`, `RuntimeFailure`, or `ExecutionCertainty`. It returns a
terminal protocol envelope or structured transport observation. The host validates and projects that result.
This keeps `@remote`, `remote::call`, async calls, and recovery policy transport-neutral.

This is a clean break. Do not retain `addr: string` overloads, `shape+tls://...` parsing, a default host/port
interpretation, or a global mutable transport singleton.

## Current Coupling

One source string currently carries endpoint syntax, transport choice, TLS roots, SNI, and deployment
identity (`crates/shape-runtime/stdlib-src/core/remote.shape:7-11,31-32,87-95`). All three call builtins take
that string, and `@remote("worker:9527")` captures it directly (`remote.shape:95-115,165-190`).

The sender parses it into a private TCP/TLS enum, including a filesystem CA path in query parameters
(`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1061-1131`). The same sender function selects the
transport, MessagePack-encodes `WireMessage`, performs I/O, and decodes the reply (`remote_builtins.rs:1157-1180`).
Sync and async paths duplicate mechanics and already disagree on deadlines.

The lower seams remain opinionated:

- `Transport::send/connect` accept `&str` (`crates/shape-wire/src/transport/mod.rs:29-52`).
- `WireTransportProvider` maps a closed `TransportKind` to a transport
  (`crates/shape-wire/src/transport/factory.rs:11-17,83-99`).
- The VM stores one global provider plus global QUIC config
  (`crates/shape-vm/src/executor/builtins/transport_provider.rs:11-58`).
- `RemoteDispatcher` exposes three nearly identical `&str` methods
  (`crates/shape-runtime/src/module_exports.rs:100-162`).
- The peer advertises a numeric version and string capabilities, not an authenticated codec/delivery contract
  (`crates/shape-vm/src/remote.rs:325-369,505-514`).

The extension loader supplies the better precedent: named/versioned capability manifests
(`crates/shape-abi-v1/src/lib.rs:76-129`) and mandatory ABI plus structural-fingerprint refusal before a vtable
call (`crates/shape-runtime/src/plugins/loader.rs:112-218`). Remoting should be a first-class
`shape.remoting` capability, not a global trait selected after string parsing.

## Design It Three Ways

### 1. Public Component Stack

```text
RemoteStack<Discovery, Router, AddressCodec, Transport, Auth, Codec, Protocol>
remote::call(stack, Discovery::Destination, Callable<Sig>, ArgumentPack<Sig>)
```

Users could mix Kubernetes discovery, least-loaded routing, QUIC, workload identity, and a custom codec.
This maximizes substitution and unit-testability. It also exposes a shallow graph with invalid combinations:
an address codec can disagree with its transport, auth can run after data leaks, and components can disagree
about deadline and submission boundaries. Generic types infect function/LSP metadata, and security review is
combinatorial. This is useful inside a provider, not as the public remoting API.

### 2. Declarative Remote Plan

```text
provider.plan(placement, requirements) -> RemotePlan
RemotePlan = Resolve | Select | Connect | Authenticate | Negotiate | Encode |
             Send | Receive | Decode | Emit | Branch | Finish
```

The host could validate, serialize, inspect, and deterministically test plans before executing them. This is
attractive for ordinary sockets. It becomes a second programming language for queues, leases, stream
multiplexing, peer challenges, connection migration, or cloud SDK callbacks. Every new mechanism adds plan
opcodes and requires a runtime release. A plan is useful optional diagnostics, but too closed as the provider
execution boundary.

### 3. Provider-Owned Session, Host-Owned Semantics

```text
provider.resolve(placement, requirements, context) -> RouteSet
provider.open(route, attempt_context) -> RemoteSession
session.exchange(canonical_invocation, host_services) -> ProviderAttemptReport
```

This is the recommendation. It hides arbitrary mechanics while host services retain authority over semantic
encoding, credentials, time, cancellation, telemetry, and validation. Providers may internally compose
components or generate plans. Compared with Design 1 it sacrifices public mix-and-match for coherent,
valid-by-construction bundles. Compared with Design 2 it trusts a loaded capability with mechanics but avoids
an ever-growing host instruction language.

## Fixed Versus Pluggable

| Fixed above providers | Pluggable inside providers |
|---|---|
| Frozen signature and `ArgumentPack<Sig>` | Destination schema and typed constructors |
| Original `R` for transparent `@remote` | Discovery sources and cache policy |
| Canonical `RemoteDispatch` / `RemoteOutcome<R>` | Candidate routing and pre-submit balancing |
| `RemoteErrorCause` plus `ExecutionCertainty` | Physical address representation/encoding |
| Logical-call/attempt identity and single submission | Socket, QUIC, mesh, queue, actor, cloud transport |
| Target, schema, argument, capture, permission facts | Peer auth and credential challenge flow |
| Function/blob hashes and verification | Codec, compression, framing, sidecars |
| Sender and receiver permission enforcement | Protocol handshake and feature negotiation |
| Absolute deadline ceiling and cancellation owner | Pooling, multiplexing, keepalive |
| Return ABI/kind validation | Namespaced attributes and transport metrics |
| No local fallback or unknown-outcome auto-retry | Safe pre-submit candidate fallback |

Providers may change representation, not meaning. A custom codec must carry the canonical target hash,
signature witness, args, captures, schemas, permissions, identities, and options. The host validates the
decoded response against the same facts before constructing `R`.

## Shape Source Interface

The notation is semantic. `P` is a nominal provider type exported by a loaded capability, not a string tag.

```text
trait RemotingProvider { type DestinationSpec }

opaque RemoteDestination<P: RemotingProvider>
opaque Placement<P: RemotingProvider>

struct PlacementPreferences {
    region: Preference<string>?
    zone: Preference<string>?
    labels: HashMap<string, string>
    affinity_key: string?
}

struct RemoteCallOptions { deadline: Deadline? }

remote::place<P>(RemoteDestination<P>, PlacementPreferences = {}) -> Placement<P>

remote::call<P, Sig>(
    placement: Placement<P>,
    target: Callable<Sig>,
    args: ArgumentPack<Sig>,
    options: RemoteCallOptions = {},
) -> Result<Sig::Return, RemoteError>

remote::call_async<P, Sig>(...) -> Future<Result<Sig::Return, RemoteError>>
remote::ping<P>(RemoteDestination<P>) -> Result<RemotePeerInfo, RemoteError>
remote::execute<P>(RemoteDestination<P>, code: string, RemoteCallOptions = {})
    -> Result<RemoteExecution, RemoteError>
annotation remote<P>(Placement<P>, RemoteCallOptions = {})
```

User syntax remains variadic; the compiler lowers it to the exact signature-indexed pack. The pack is not an
`Array<_>`, and a nested array remains one parameter. This follows `CONTEXT.md:15-34` and
`docs/cluster-audits/wave40-annotation-hook-type-model.md:75-145`.

There is no `RemoteDestination::from_string`, provider cast, or generic object constructor. Provider modules
export typed constructors:

```shape
let gpu_service: RemoteDestination<Kubernetes> = kube::service(
    cluster: "research-prod",
    namespace: "compute",
    service: "shape-worker",
    labels: { accelerator: "a100" },
)

let gpu = remote::place(
    gpu_service,
    preferences: { region: prefer("eu-west"), labels: { tier: "batch" } },
)

@remote(gpu, options: { deadline: 30.seconds.from_now() })
fn price_paths(paths: int, seed: int) -> Array<number> {
    monte_carlo(paths, seed)
}
```

A direct provider remains possible without giving `remote` an address grammar:

```shape
let dev = direct::tcp(
    host: dns_name("worker.internal"),
    port: port(9527),
    security: mtls_profile("shape-dev-workers"),
)

let result = remote::call(
    remote::place(dev), compute, input,
    options: { deadline: 5.seconds.from_now() },
)
```

The destination stays provider-typed even when its constructor accepts typed host/port fields. TLS roots,
tokens, private keys, and filesystem paths are operator config or credential references, never destination
data.

`@remote` remains an ordinary transparent before hook:

```text
HookDecision::Return {
    args,
    result: remote::__dispatch_raising(placement, ctx.target, args, options),
    state: Unit,
}
```

Success is exactly `R`; provider failure becomes `Failed(RuntimeFailure::Remote)`. `remote::call` projects
the same internal outcome to `Completed(Result<R, RemoteError>)`. Providers never select that projection
(`CONTEXT.md:154-166`).

## Failover Is Policy

Provider routing may choose equivalent candidates before submission and may try another candidate after a
host-recorded pre-submission failure. Once submission begins, it cannot silently switch routes.

```shape
fn call_with_failover(input: Job) -> Result<Output, RemoteError> {
    match remote::call(primary, run_job, input) {
        Ok(value) => Ok(value)
        Err(e) if e.certainty == DefinitelyNotExecuted =>
            remote::call(secondary, run_job, input)
        Err(e) => Err(e)
    }
}
```

`OutcomeUnknown` and `ExecutionStarted` require a separate idempotency or negotiated dedup/result-replay
policy. A provider cannot infer idempotency from a target, call ID, transport, or cause. This preserves the
accepted single-submission model
(`docs/cluster-audits/wave40-remote-delivery-semantics.md:7-31,207-236`).

## Provider Identity And Configuration

```text
struct ProviderIdentity {
    contract: "shape.remoting"
    contract_version: SemVer
    name: Symbol
    implementation_version: SemVer
    build_fingerprint: Hash256
}

struct ProviderInstanceIdentity {
    provider: ProviderIdentity
    configuration_digest: Hash256
    generation: u64
    runtime_epoch: OpaqueEpoch
}
```

The first identifies code/ABI; the second identifies an immutable validated config generation. The digest
covers normalized non-secret settings and secret reference identities, never secret values. Reload creates a
new generation rather than changing in-flight behavior.

Add dedicated `CapabilityKind::Remoting` and `shape.remoting`, not security-sensitive behavior hidden under
`Custom`. Its descriptor uses typed capabilities:

```text
struct RemotingProviderDescriptor {
    identity: ProviderIdentity
    destination_schema_hash: Hash256
    protocols: Set<ProtocolIdentity>
    codecs: Set<CodecIdentity>
    authentication: Set<AuthMechanism>
    cancellation: CancellationCapability
    delivery_evidence: DeliveryEvidenceCapability
    sidecars: SidecarCapability
    persistent_sessions: bool
    snapshot_destination: SnapshotDestinationCapability
    required_host_permissions: PermissionSet
}
```

The operator/embedding host creates instances. Shape code may select configured instances and construct typed
destinations; it cannot load libraries, inject raw provider config, or obtain secrets as values.

## Host/Provider SPI

```text
trait RemotingProviderFactory: Send + Sync {
    fn descriptor(&self) -> &RemotingProviderDescriptor
    fn start(config: SealedProviderConfig, host: ProviderHost)
        -> Result<Arc<dyn RemotingProvider>, ProviderStartFailure>
}

trait RemotingProvider: Send + Sync {
    fn identity(&self) -> ProviderInstanceIdentity
    fn validate_destination(&self, destination: &SealedDestination)
        -> Result<DestinationFingerprint, ProviderFailure>
    async fn resolve(&self, placement: &SealedPlacement,
                     requirements: &DispatchRequirements,
                     context: &ResolveContext)
        -> Result<RouteSet, ProviderFailure>
    async fn open(&self, route: &OpaqueRoute, context: &AttemptContext)
        -> Result<Box<dyn RemoteSession>, ProviderFailure>
    async fn shutdown(&self, deadline: Deadline)
}

trait RemoteSession: Send {
    async fn establish(&mut self, offer: ProtocolOffer,
                       services: &SessionHostServices)
        -> Result<SessionAgreement, ProviderFailure>
    async fn exchange(&mut self, request: &CanonicalRemoteInvocation,
                      services: &AttemptHostServices)
        -> ProviderAttemptReport
    async fn cancel(&mut self, cancellation: &RemoteCancellation,
                    services: &AttemptHostServices)
        -> ProviderCancellationReport
    async fn close(self: Box<Self>)
}
```

`ProviderHost` supplies scoped capabilities: monotonic clock, task spawning, allowed network/process access,
credential broker, codec registry, structured event sink, and provider-owned storage namespace. It exposes no
VM stack, raw `KindedSlot`, arbitrary filesystem, or global secret map.

`DispatchRequirements` is immutable and host-authored: target/signature hashes, required protocol features,
permissions, foreign/runtime needs, payload estimate, deadline, and security minimum. Providers may add but
not remove constraints. They do not inspect argument values for routing; callers put legitimate routing keys
in the typed destination or preferences.

`RouteSet` contains ranked opaque routes and non-secret labels. Routes are ephemeral and instance-bound, not
Shape values or snapshot data. The runtime never parses one as host/port.

## Codec And Negotiation

`CanonicalRemoteInvocation` is a semantic host object containing logical call/attempt IDs, function hash and
minimal content closure, signature/frame witness, typed args/captures, schemas/foreign requirements, required
permissions, deadline/cancellation correlation, and policy-approved trace context.

```text
trait RemoteCodec {
    fn identity(&self) -> CodecIdentity
    fn encode(&self, invocation: &CanonicalRemoteInvocation)
        -> Result<EncodedRequest, CodecFailure>
    fn decode(&self, bytes: &[u8], expected: &ExpectedRemoteResponse)
        -> Result<RemoteTerminalEnvelope, CodecFailure>
}
```

The provider selects only from the host `ProtocolOffer`. The peer-authenticated `SessionAgreement` binds
provider instance, peer, protocol, codec, hash algorithm, features, size limits, cancellation, and delivery
evidence. The runtime rejects any agreement below its security minimum or unable to carry the canonical call.

Codecs control bytes, compression, framing, and sidecars, not identity. Function hashes are over canonical
`FunctionBlob` content before encoding; route, transport, auth, codec, and compression cannot change them.
The receiver recomputes hashes and applies its permission gate before VM entry. Existing MessagePack
`WireMessage` can be the first adapter, not the universal remoting ABI.

## Certainty, Deadlines, And Cancellation

Providers report mechanics; the host derives certainty through a monotonic attempt recorder:

```text
Prepared -> Routed -> Connected -> SubmissionBegan
         -> ReceiverAdmitted -> ExecutionStarted -> TerminalObserved
```

Before `SubmissionBegan`, failure is `DefinitelyNotExecuted`. Entering it makes the conservative state
`OutcomeUnknown`. Only an authenticated protocol-valid pre-execution rejection may prove
`DefinitelyNotExecuted`; authenticated started/terminal failure gives `ExecutionStarted`. Decoded validated
success completes. Providers cannot set certainty directly or call a generic write failure retry-safe.

Missing-blob resupply is a fixed continuation only after an authenticated pre-execution rejection naming the
hashes. Its resupply is a separately observed attempt, not general retry.

The host converts a relative timeout to one absolute deadline before discovery. Every phase receives that
deadline and a child cancellation token. Providers may budget remaining time but cannot extend it. Sync calls
are projections over this async core, preventing sync/async timeout drift.

Cancellation preserves two facts: local wait cancelled and remote cancel requested/acknowledged. Only an
authenticated queued/pre-execution acknowledgement proves non-execution; best-effort send, unknown call, or
already-running does not.

## Observability

The host emits canonical events keyed by logical call, attempt, provider instance, destination fingerprint,
phase, elapsed time, negotiated protocol/codec, byte counts, cause, and certainty. Providers may attach
descriptor-declared namespaced typed attributes with redaction classes. Raw args, captures, credentials,
physical addresses, destination payloads, and response bytes are excluded by default. Provider log strings
are presentation, never authoritative semantics. Trace propagation is host policy; call/trace IDs are
correlation tokens, not idempotency keys.

## Security And Misuse Prevention

1. Refuse provider load on capability version, ABI/fingerprint, destination schema, or host-permission mismatch.
2. Destinations are immutable, provider-typed, instance-bound, and made/reopened only by provider code.
3. Source cannot embed secrets, trust-root paths, or raw provider config; the broker issues scoped opaque leases.
4. Resolution cannot widen allowed provider/network/destination scope; host policy checks before I/O.
5. Providers cannot remove `required_permissions`, alter hashes, fabricate frame kinds, or bypass receiver gates.
6. Custom codecs must pass host arity, kind, schema, hash, correlation, and return validation.
7. Negotiation cannot downgrade peer auth, integrity, deadline, cancellation, or delivery evidence.
8. Route switching stops at submission unless explicit higher-level retry authorizes a new attempt.
9. No provider may execute locally as fallback or turn failure into completion.
10. Causes/phases remain structured; adapters never recover semantics by parsing messages.

There is also no string constructor, prefix-selected default provider, global active provider, untyped source
config, provider-specific `RemoteError` variant, provider-authored certainty, automatic unknown-outcome retry,
response-bits acceptance before validation, cancellation-as-rollback claim, call-ID-as-dedup claim, or live
provider pointer in snapshots.

Receiver permissions/resource limits remain authoritative. Sender checks are early diagnostics, not the
security boundary. Required permissions remain in content identity
(`docs/design/distributed-function-transfer.md:12-20,60-68`).

## Lifecycle

1. **Load:** verify `shape.remoting`, ABI/fingerprint, descriptor, and nominal destination types; pin code while used.
2. **Configure:** validate operator config/secret references, compute digest, grant scoped capabilities, create generation.
3. **Construct:** provider functions make sealed destinations; `remote::place` adds host-neutral preferences.
4. **Dispatch:** freeze generation, validate destination/scope, derive requirements, resolve/select before attempt.
5. **Establish:** open transport, authenticate peer, negotiate codec/protocol, validate agreement before call bytes.
6. **Exchange:** encode, record submission, decode terminal envelope, host-validate and project outcome.
7. **Pool:** key by generation, destination, route security identity, principal, protocol, and codec.
8. **Reload:** publish a new generation; new calls use it while old sessions drain unchanged.
9. **Shutdown:** stop resolution, cancel pre-submit work, request in-flight cancellation, drain, close, unload.

## Snapshot And Wire

Destinations/placements are snapshot-safe only if the provider declares a stable destination codec. Persist
provider contract/version range, provider name/config reference and digest, destination schema hash plus sealed
canonical payload, and host-neutral preferences. Never persist resolved routes, sockets, sessions, credentials,
secrets, runtime epochs, attempt recorders, or cancellation handles.

Restore requires a compatible configured provider, revalidates payload and permission scope, then performs
fresh discovery. Missing provider/config fails explicitly; another provider never reinterprets the payload.
In-flight calls remain non-resumable. Durable remote futures, dedup, and replay require a separate protocol
with retention/crash semantics; serializing a destination does not supply them.

Wire envelopes may differ completely but must retain canonical semantic facts and authenticated evidence for
host validation/classification. Codec bytes are not snapshot identity. Hashes remain codec-independent, and
caches cannot cross peer, principal, protocol, or security contexts. The runtime snapshot provenance proof
does not cover provider handles; destination payloads use typed serialization, never opaque live pointers.

## Tradeoffs

The selected boundary is less publicly composable than separate resolver/transport/auth/codec objects. That
is deliberate: a provider ships one coherent stack and the host verifies one deep contract, while providers
may compose internal components freely.

The extension ABI is larger and async. It needs owned-buffer rules, cancellation-safe callbacks, panic
containment, lifecycle pinning, versioned vtables, and semantic conformance tests beyond ABI fingerprints.
Custom codecs enlarge the trusted base, but restricting providers to MessagePack would make codec customization
fictional; canonical host objects plus post-decode validation are the narrower trust boundary.

Opaque destinations require provider type/schema metadata in compiler and LSP. A missing provider becomes an
early compile/config error, and snapshot portability depends on provider support/config. Async-first dispatch
adds small scheduling cost to sync calls but removes the more dangerous sync/async semantic divergence.

## Bottom Line

Shape should model remoting as a pluggable execution-provider capability, not TCP with a clever string.
`RemoteDestination<P>` supplies a typed logical target, `Placement<P>` adds neutral constraints, a deep
provider/session hides discovery through response bytes, and the runtime keeps authority over types, outcomes,
certainty, hashes, permissions, deadlines, cancellation, and validation. This supports sockets, service
discovery, meshes, queues, actors, and cloud fabrics without making one of them the language's remote-host model.
