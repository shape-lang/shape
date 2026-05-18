# RFC-008: Real-Time Compiler↔LLM Channel

- **Status:** Draft
- **Author:** Shape language team
- **Date:** 2026-05-18
- **Cross-references:** RFC-001 (`#[replaces]` directive, `behavior_hash`), RFC-002 (`@law` property), RFC-004 (`#[replaces]` directive surface), RFC-007 (`@intent` property)
- **Sigils introduced:** none. This RFC is purely a transport-and-protocol layer that consumes existing sigils as event payload sources.

## Summary

Shape's compiler already knows an enormous amount about the program a user (or an LLM) is in the middle of writing — which bindings constrain which other bindings, where an `Option<T>` is provably non-null, which code shape matches a known anti-pattern, which permissions a function just acquired and why. Today, almost none of that knowledge reaches an LLM during token generation; the LLM sees a buffer of source text and, after the user saves, a list of LSP diagnostics.

This RFC proposes a **real-time compiler↔LLM channel**: a new LSP notification family `shape/inference`, layered on top of `tower-lsp-server`'s existing custom-notification mechanism, that streams structured *partial-program facts* — coactivation, narrowing, attractor matches, duplicate candidates, permission inference, law-witness counterexamples, and intent collisions — out of the same `analyze_document` pass that already produces `publishDiagnostics`. The LLM connects through `shape-mcp` as a first-class subscriber, queries the latest snapshot between tokens, and feeds those facts back into its prompt.

We ship **v0.1: enriched LSP**. Shape's existing LSP infrastructure (`tools/shape-lsp/`) and the inference engine (`crates/shape-runtime/src/type_system/inference/`) already record the structured data; we surface it. **v1: full Hazelnut-Live semantics** — every partial AST is well-typed, every typed hole carries an evaluation closure — is explicitly deferred to a multi-year research track behind a hard empirical gate (§Phasing).

The phasing is the most important decision in this RFC, and §Rationale defends it at length against a documented codebase failure pattern.

## Motivation

The LLM authoring loop in Shape today looks like this:

1. LLM emits code based on prompt + recent context.
2. User (or wrapper tool) saves the buffer.
3. `shape-lsp` runs `analyze_document`, returns diagnostics.
4. LLM (in a subsequent prompt) sees the diagnostics, reasons, edits.
5. GOTO 1.

This loop has three failures:

- **Latency.** Saving and re-prompting happens at human-perception scale (≥hundreds of ms). The LLM produces tokens in tens of ms. Diagnostics arrive long after the relevant token has been committed.
- **Lossiness.** The compiler has already proven, e.g., that the `x` in `arr.map(|x| ...)` has type `Item` (from the receiver's `Array<Item>` element type via bidirectional inference at `crates/shape-runtime/src/type_system/inference/mod.rs:121` `callsite_type_args`). That fact is consumed for type checking and discarded. The LLM never sees it. It then writes `x.foo` against an `Item` that has no `foo`, and the loop restarts.
- **Re-derivation cost.** The LLM, lacking the proof, infers it badly from text. Cursor and Copilot in 2025-2026 spend most of their context budget on retrieved files trying to *re-derive* facts the compiler already has [(TensorZero reverse-engineering writeup)](https://www.tensorzero.com/blog/reverse-engineering-cursors-llm-client/).

The Omar group's "Statically Contextualizing Large Language Models with Typed Holes" (OOPSLA 2024, [arXiv 2409.00921](https://arxiv.org/abs/2409.00921)) measures this directly: type-context contextualization shifts LLM completion quality more than any other intervention they tried, including their own bespoke retrieval pipeline. The MVUBench results validate the *direction* of this RFC — the question is only how aggressively we generalize beyond type context.

The Shape compiler already records seven distinct families of facts during a single `analyze_program_semantics` pass (`tools/shape-lsp/src/analysis.rs:25`). We can surface them, rank them, and stream them. That is the entire content of v0.1.

## Guide-level explanation

### The channel from the LLM's perspective

The LLM connects to `shape-mcp` and calls a new tool:

```json
{
  "method": "tools/call",
  "params": {
    "name": "subscribe_inference",
    "arguments": {
      "uri": "file:///home/me/project/src/main.shape",
      "cursor": { "line": 42, "column": 18 },
      "event_families": ["coactivation", "narrowing", "permission_inference"],
      "snapshot_token": null
    }
  }
}
```

`shape-mcp` returns a snapshot of currently-relevant inference events plus an opaque `snapshot_token`. On the next token boundary, the LLM calls `get_inference_snapshot` with the token to receive a *delta* — only events whose relevance to the cursor has changed since.

### The seven event families

Each event is a structured JSON payload conforming to a schema in `crates/shape-diagnostics/` (§Reference-level). Concrete examples:

**1. `coactivation` — "your type was constrained by these other bindings"**

```json
{
  "family": "coactivation",
  "subject": { "kind": "binding", "name": "filter_fn", "loc": {"file": "main.shape", "line": 12, "col": 5, "span": [180, 189]} },
  "constraints": [
    { "constrained_by": "items", "loc": {"line": 9, "col": 9, "span": [120, 125]}, "via": "callsite_param_types" },
    { "constrained_by": "Item.score", "loc": {"line": 4, "col": 11, "span": [55, 60]}, "via": "field_access_on_param" }
  ],
  "inferred_type": "fn(Item) -> bool",
  "snapshot_token": "s7392"
}
```

Source data: `TypeInferenceEngine.constraint_origins` (`mod.rs:71`), `callable_origins_by_name` (`mod.rs:74`), `callsite_param_types` (`mod.rs:89`).

**2. `narrowing` — "this `Option<T>` is provably non-null here"**

```json
{
  "family": "narrowing",
  "subject": { "kind": "binding", "name": "user", "loc": {"line": 23, "col": 9, "span": [310, 314]} },
  "from_type": "User?",
  "to_type": "User",
  "narrowed_at": {"line": 22, "col": 8, "span": [295, 304]},
  "narrowing_condition": "user != null",
  "valid_in_range": [310, 380],
  "snapshot_token": "s7393"
}
```

Source data: `extract_narrowings` (`crates/shape-runtime/src/type_system/inference/statements.rs:220`).

**3. `attractor_match` — "this matches a known anti-pattern"**

```json
{
  "family": "attractor_match",
  "subject": { "kind": "expr", "loc": {"line": 50, "col": 12, "span": [720, 760]} },
  "attractor_name": "manual_option_unwrap_with_error",
  "matched_shape": "if x == null { return Err(...) } else { x.foo }",
  "canonical_form": "let v = x ?? return Err(\"…\"); v.foo",
  "severity": "hint",
  "snapshot_token": "s7394"
}
```

Source data: registry of attractor patterns to be added in `crates/shape-diagnostics/`; initially seeded from the CLAUDE.md `§Forbidden rationalizations` list applied to user code patterns, not just compiler-internal code.

**4. `duplicate_candidate` — "this is structurally isomorphic to an existing function"** *(gated by RFC-001 `behavior_hash`)*

```json
{
  "family": "duplicate_candidate",
  "subject": { "kind": "function_under_cursor", "name": "scoreUser", "loc": {"line": 60, "col": 1, "span": [800, 950]} },
  "candidate": {
    "name": "rate_user",
    "loc": {"file": "ranking.shape", "line": 12, "col": 1, "span": [180, 320]},
    "behavior_hash": "b3:7e8f...c2a1",
    "similarity": 0.94
  },
  "rfc": "RFC-001",
  "suggested_directive": "#[replaces(rate_user)]",
  "snapshot_token": "s7395"
}
```

Source data: function-body `behavior_hash` registry maintained per RFC-001 / RFC-004.

**5. `permission_inference` — "this function just gained `FsWrite` because…"**

```json
{
  "family": "permission_inference",
  "subject": { "kind": "function", "name": "save_report", "loc": {"line": 30, "col": 1, "span": [400, 600]} },
  "permissions_now": ["FsWrite", "Time"],
  "permissions_before": ["Time"],
  "added": [
    { "perm": "FsWrite", "cause": "call to std::core::file::write_file at line 35" }
  ],
  "scope_constraints_suggested": [{"path_glob": "./reports/*.json"}],
  "snapshot_token": "s7396"
}
```

Source data: `required_permissions` (`crates/shape-runtime/src/stdlib/capability_tags.rs:14`), called incrementally on every stdlib call resolution.

**6. `law_witness` — "the property test for `@law(commutative)` is failing on input X"** *(gated by RFC-002)*

```json
{
  "family": "law_witness",
  "subject": { "kind": "function", "name": "merge", "loc": {"line": 80, "col": 1, "span": [1100, 1300]} },
  "law": "commutative",
  "rfc": "RFC-002",
  "status": "counterexample_found",
  "counterexample": { "lhs": "merge(a, b)", "rhs": "merge(b, a)", "a": "[1, 2]", "b": "[3, 4]", "lhs_result": "[1, 2, 3, 4]", "rhs_result": "[3, 4, 1, 2]" },
  "snapshot_token": "s7397"
}
```

Source data: RFC-002 property-test runner output, exposed live as it produces witnesses.

**7. `intent_collision` — "your `@intent` is 0.92 similar to existing function Y"** *(gated by RFC-007, advisory only)*

```json
{
  "family": "intent_collision",
  "subject": { "kind": "function_under_cursor", "name": "process_payment", "intent": "Charge the user's card for the order total." },
  "candidate": {
    "name": "charge_order",
    "loc": {"file": "billing.shape", "line": 45, "col": 1, "span": [600, 800]},
    "intent": "Charges the buyer's payment method for the cart subtotal.",
    "similarity": 0.92
  },
  "rfc": "RFC-007",
  "advisory": true,
  "snapshot_token": "s7398"
}
```

Source data: embedding-based similarity over the `@intent` corpus per RFC-007.

### How digesting works

A naive channel emits hundreds of events per keystroke (every coactivation in the whole program changes when a single binding is renamed). The MCP server applies a two-stage filter before returning a snapshot:

1. **Relevance**: events ranked by edit-distance from cursor in AST-span terms, multiplied by event-family weight (errors > narrowings > coactivation > intent collisions).
2. **Novelty**: events whose payload has not changed since the last `snapshot_token` the LLM holds are suppressed.

The result is a digest — typically 1–8 events — sized to fit in <500 cl100k tokens (the LSDS payload budget at `crates/shape-diagnostics/src/lib.rs` SCHEMA_VERSION 1 comment).

### Client-side rendering

The same payloads, when delivered to a *human* editor, render as inline hints (narrowing arrows in the gutter), code lenses ("3 coactivations"), or hover tooltips. This is intentional: the LLM and the human consume the same channel, framed differently. There is no separate "LLM mode" to maintain.

## Reference-level explanation

### LSP protocol extension

A single new server→client notification:

```
method:  shape/inference
params:  ShapeInferenceParams (see below)
```

`tower-lsp-server`'s `Client::send_notification::<N>` ([upstream example](https://github.com/ebkalderon/tower-lsp/blob/master/examples/custom_notification.rs)) supports custom server→client notifications without protocol fork or version bump. Existing LSP clients that don't subscribe simply ignore the notification — full backward compatibility.

```rust
// crates/shape-diagnostics/src/inference_channel.rs (new)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapeInferenceParams {
    pub schema_version: u32,        // mirror SCHEMA_VERSION
    pub uri: String,
    pub snapshot_token: String,     // opaque, monotonic per document
    pub events: Vec<InferenceEvent>,
    pub digest_truncated: Option<usize>, // count suppressed by digest
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum InferenceEvent {
    Coactivation(CoactivationEvent),
    Narrowing(NarrowingEvent),
    AttractorMatch(AttractorEvent),
    DuplicateCandidate(DuplicateEvent),       // RFC-001 / RFC-004
    PermissionInference(PermissionEvent),
    LawWitness(LawWitnessEvent),              // RFC-002
    IntentCollision(IntentCollisionEvent),    // RFC-007
}
```

Each event type reuses `shape_diagnostics::Location` (the LSDS `Location` struct at `crates/shape-diagnostics/src/lib.rs:70`) and `Severity`. **No new location or severity enum.** Reusing LSDS keeps every renderer (terminal, LSP, MCP) single-pathed.

### Where the data comes from

Every event family already has source data inside the existing `analyze_program_semantics` pass:

| Event family | Source data | Validated anchor |
|---|---|---|
| `coactivation` | `TypeInferenceEngine.constraint_origins`, `callable_origins_by_name`, `callsite_param_types` | `crates/shape-runtime/src/type_system/inference/mod.rs:71, :74, :89` |
| `narrowing` | `extract_narrowings()` | `crates/shape-runtime/src/type_system/inference/statements.rs:220` |
| `attractor_match` | new attractor registry in `shape-diagnostics`, walked over AST | n/a (new) |
| `duplicate_candidate` | RFC-001 `behavior_hash` registry | RFC-001 |
| `permission_inference` | `required_permissions()` per stdlib call resolution | `crates/shape-runtime/src/stdlib/capability_tags.rs:14` |
| `law_witness` | RFC-002 property runner | RFC-002 |
| `intent_collision` | RFC-007 `@intent` corpus + embeddings | RFC-007 |

For v0.1 we add a single emit step at the end of `analyze_document` in `tools/shape-lsp/src/server.rs:266` (after `analyze_program_semantics` returns at `analysis.rs:25`). Engine fields that today are `pub(crate)` (per `inference/mod.rs:71`–`:89`) gain narrow public accessors used only by the event-emitter.

### Ranking and digest algorithm

```text
score(event) = w_family[event.family]
             * decay(byte_distance(event.span, cursor.byte))
             * novelty(event, last_snapshot)

w_family:  error > narrowing > permission_inference > attractor_match
         > duplicate_candidate > coactivation > intent_collision

decay(d):  1.0 if d < 200
           0.5 if d < 2000
           0.1 otherwise

novelty:   1.0 if event not present in last snapshot or payload changed
           0.3 if present and unchanged but cursor moved within range
           0.0 if identical and cursor unchanged
```

Top-K (K=8 by default) events are returned. `digest_truncated` reports the count of suppressed events so the LLM can decide whether to call `get_inference_snapshot` with `event_families=["…"]` to inspect a specific dropped family.

This is the same shape as Hazel's [ChatLSP](https://hazel.org/papers/chatlsp-oopsla2024.pdf) prompt budget, generalized to a streaming/delta protocol rather than per-completion prompt assembly.

### Integration with shape-mcp

`shape-mcp/src/tools.rs` adds two tools alongside the existing nine (`search_shape_docs`, `get_shape_syntax`, `get_shape_examples`, `run_shape_code`, `get_shape_api`, `search_shape_packages`, plus three from the recent expansion):

- `subscribe_inference(uri, cursor, event_families?, snapshot_token?) → InferenceSnapshot`
- `get_inference_snapshot(uri, cursor, snapshot_token, event_families?) → InferenceDelta`

The MCP server holds an in-process subscription to the LSP server (same binary in `shape-cli`'s `wire-serve` mode; cross-process via JSON-RPC otherwise). Subscriptions are cheap — they're keyed by `(uri, snapshot_token)` and reuse the LSP's existing per-document analysis cache.

### Loop-stability: `Origin::User` / `Origin::Llm` / `Origin::Tooling`

Naive feedback loops oscillate: LLM writes code → channel fires → LLM consumes → LLM writes more code → channel fires on its own output → loop accelerates.

We tag every document edit with one of three origins:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// Human keystrokes via the editor.
    User,
    /// LLM-generated edits via MCP or an agent client.
    Llm,
    /// Refactor, format-on-save, code action, code lens.
    Tooling,
}
```

The MCP `applyEdit` tool stamps `Origin::Llm` automatically. Editor keystrokes stamp `Origin::User`. The LSP `textDocument/formatting` and `workspace/applyEdit` for code actions stamp `Origin::Tooling`.

**Channel emission rule:** events from `Origin::Llm` edits go to a separate `shape/inferenceProposed` channel that **the LLM is not subscribed to by default**. A wrapper agent (Cursor, Claude Code, the eventual Shape playground) can opt into reading `shape/inferenceProposed` for its own validation but must not pipe it into the same model's next-token context.

This is the same principle as rust-analyzer's [salsa cancellation discipline](https://rust-analyzer.github.io/blog/2023/07/24/durable-incrementality.html): the structural invariant ("typing inside a function's body never invalidates global derived data") is what makes incremental work tractable. We extend it to the LLM loop: "LLM-driven edits never feed the LLM's own input channel" is what makes the loop converge.

### Persistence for cold-start LLMs

The MCP server retains the last N=64 snapshots per document, keyed by `snapshot_token`. A fresh LLM session re-attaching to an open document calls `get_inference_snapshot(uri, cursor, snapshot_token=null)` and receives the most recent full snapshot, not just a delta. This lets handoffs across LLM sessions (e.g., subagent boundaries in Claude Code) resume without losing channel state.

### Relationship to other RFCs

This RFC is the *transport substrate* that several other RFCs rely on to deliver their feedback:

- **RFC-001 (`#[replaces]` directive, `behavior_hash`)** — `duplicate_candidate` events surface candidate `#[replaces]` targets in real time. Without RFC-008, the directive's value is post-hoc; with it, the LLM can emit `#[replaces(...)]` correctly on first write.
- **RFC-002 (`@law` property)** — `law_witness` events surface property-test counterexamples as the LLM is still writing the function body, not after CI fails.
- **RFC-004 (`#[replaces]` directive surface)** — same channel as RFC-001 for the directive-side semantics.
- **RFC-007 (`@intent` property)** — `intent_collision` events flag a new function whose intent text matches an existing function.

None of those RFCs requires RFC-008 to *function*; RFC-008 makes them *fast*.

## Drawbacks

- **Compiler retrofit cost (full version).** Doing this *properly* — every partial AST well-typed, every hole carrying an evaluation closure — is what Hazel does, and it took the Omar group from 2017 (POPL Hazelnut) through 2024 (OOPSLA ChatLSP) on top of an Agda-mechanized metatheory. Shape's type system is ~10k lines of Rust with no mechanized metatheory. A full retrofit is 3–5 engineer-years. v0.1 deliberately doesn't attempt it. v1 might never happen. §Phasing explains why that's acceptable.
- **Channel bandwidth tuning.** The digest algorithm has weights we're guessing at. Wrong weights either drown the LLM or starve it. Mitigation: weights live in `crates/shape-diagnostics/src/inference_channel.rs` as a single constant table, tunable without API change; we'll instrument hit-rate per family.
- **Feedback-loop instability risk.** The `Origin` segregation is necessary but not sufficient. If an agent wrapper merges `Origin::User` and `Origin::Llm` channels (e.g., by treating LLM edits as user edits after acceptance), oscillation is possible. The MCP server can detect this — bouncing same-event-family snapshots at >5 Hz — and emit a `loop_warning` event, but cannot prevent it.
- **LSP protocol bloat.** Adding a custom notification family is cheap (clients ignore unknown methods), but adding *seven* event types behind it is more surface than any single LSP feature today carries. Clients that want to render the channel natively (the eventual Shape playground; possibly a custom Zed extension) inherit non-trivial implementation work.
- **Perf cost of computing events incrementally.** Today, `analyze_program_semantics` runs the inference engine in `RecoverAll` mode on every keystroke (`tools/shape-lsp/src/analysis.rs:57`). Adding event emission adds a small constant per inference fact. Real cost is in the *delta* computation against the last snapshot; we mitigate via salsa-style query keying on `(span, family)`.
- **Event-payload schema drift.** SCHEMA_VERSION discipline (already in LSDS) carries over, but seven event families means more rename risk. We commit to the stability contract documented in `crates/shape-diagnostics/src/lib.rs:18` (only-additive field changes; version bump for breaking).

## Rationale and alternatives

### Why LSP, not a separate daemon

A separate "Shape inference daemon" was considered and rejected. Reasons:

1. LSP already runs (every editor we care about — Zed, VS Code, Helix, Neovim — speaks it). A daemon is a second always-on process with its own lifecycle bugs.
2. The data we want to emit is computed inside the LSP's existing analysis pass. Splitting it out means duplicating ~600 lines of pipeline state, or routing every keystroke through two processes.
3. `tower-lsp-server` already supports custom server→client notifications. Zero protocol-fork cost.

The cost of using LSP is that we inherit LSP's per-document model. For cross-file events (RFC-001 `duplicate_candidate` against another file's `behavior_hash`), we maintain a workspace-level event index that the per-document emission consults.

### Why enriched-LSP for v0.1, not Hazelnut-Live for v1

The CLAUDE.md §"Why this matters" describes the precise failure mode this phasing is designed to prevent. To restate it for record: the v2-nanbox-removal-plan Step 6 ("delete `ValueWord`") was quietly downgraded mid-execution to "ValueBits shim retained as documented FFI-boundary bridge." That single rationalization converted a one-time deletion into permanent maintenance debt: 2,650-line preserved module, 9 follow-up commits of decode bridges, 4 deferred aliasing tests, 23 ignored JIT tests, ~48 shape-test failures in the same bug class. Estimated cost: 4–6 weeks of cumulative cleanup.

A "Hazelnut Live for Shape" v1 is exactly the shape of project that produces that failure mode at scale. Concretely:

- **Mechanization gap.** Hazel's Hazelnut Live calculus is Agda-mechanized; Shape's type system is unmechanized Rust. A faithful port requires either (a) reimplementing the type system on top of a calculus we haven't proven sound, or (b) mechanizing Shape's type system first. Both are multi-year.
- **Partial-program well-typedness.** Hazelnut Live's invariant — every intermediate edit state is a well-typed term with holes — requires every editor edit to route through a structure editor or through bidirectional cast insertion. Shape today uses Pest text parsing with `RecoverAll` diagnostic mode (`analysis.rs:57`). Switching paradigms touches everything from `shape-ast` to the LSP to `shape-test`.
- **Evaluation around holes.** Hazel runs the *unfilled* program: each hole's closure is captured and propagated. Shape's VM (`crates/shape-vm/`) has no notion of a hole — adding it interacts with every typed-opcode, every JIT lowering, every snapshot.
- **Surface-area expansion.** §Forbidden Patterns explicitly refuses the standard escape hatches (renames, "small fallbacks", feature flags). A Hazelnut Live port that runs into a corner cannot quietly add `ValueWord`-shaped band-aids. It must either succeed structurally or be abandoned cleanly.

The phasing is therefore: **ship v0.1, measure, then *decide*.** Specifically:

> v1 (full Hazelnut-Live semantics) is gated on v0.1 producing ≥15% pass@1 improvement on a pre-registered Shape-LLM-coding benchmark (parallel to MVUBench), measured against a v0.0 control (LSP diagnostics only). The benchmark, gate threshold, and measurement methodology are committed before v0.1 ships.

If v0.1 doesn't move the needle, v1 doesn't get a vote. If v0.1 moves the needle a lot, v1's design space contracts to "what additional channel data justifies the rewrite", which is a much more answerable question.

This is the same discipline the Wave 3 stabilize round in the codebase enforces today: classify before deepening, measure before extending. The strict-typing plan's `prove_native_kind() -> Result<NativeKind, ProofGap>` (CLAUDE.md §Mechanical enforcement) makes it impossible for emit code to fabricate "I proved it"; the v0.1 benchmark gate makes it impossible for v1 to fabricate "I'm worth doing."

### Why structured events, not text

A plausible alternative is "emit a text comment block before every LLM turn that describes the current inference state." This is what some Continue.dev-style extensions do. We reject it because:

1. Text events can't be deduplicated against the last snapshot — every comment is novel from the LSP's perspective.
2. Text events can't be ranked — the LLM sees the firehose.
3. Text events can't be re-rendered for a human editor — there's no way to derive an inline hint from a comment string.

Structured events let the LLM, the human editor, and any third tool all consume the same source of truth.

### Why MCP subscription, not polling

Polling at LLM-token cadence (~10 ms) overwhelms the LSP. Subscription-with-delta (the model here) is the same architecture as rust-analyzer's salsa query graph: compute once on change, broadcast deltas to subscribers. The `snapshot_token` is the salsa version number, repurposed across a process boundary.

## Prior art

- **Hazel + Hazelnut Live** ([Omar et al., POPL 2019](https://dl.acm.org/doi/10.1145/3009837.3009900), [arXiv 1805.00155](https://arxiv.org/abs/1805.00155); [hazel.org](https://hazel.org/)). The original "every partial program is well-typed" position. RFC-008 v1 is "Hazelnut Live for Shape." RFC-008 v0.1 is "the Hazelnut Live ergonomic story without the metatheory rewrite."
- **ChatLSP / Statically Contextualizing LLMs with Typed Holes** (Blinn, Li, Kim, Omar, OOPSLA 2024, [arXiv 2409.00921](https://arxiv.org/abs/2409.00921), [PDF](https://hazel.org/papers/chatlsp-oopsla2024.pdf)). The direct prior art for the RFC. MVUBench shows type-context contextualization dominates other interventions. ChatLSP is a *pull* protocol; RFC-008 is *push*-with-delta. Both produce the same payloads in steady state.
- **Hazel Totally Live Programming (HATRA 2023 progress report)** ([PDF](https://hazel.org/papers/hazel-hatra23.pdf)). The "live evaluation around holes" milestone — a v1 reference design but not a v0.1 dependency.
- **rust-analyzer + salsa** ([architecture](https://rust-analyzer.github.io/book/contributing/architecture.html), [Durable Incrementality 2023](https://rust-analyzer.github.io/blog/2023/07/24/durable-incrementality.html), [recent salsa improvements June 2025](https://rust-analyzer.github.io/thisweek/2025/06/16/changelog-290.html)). The incremental-query architecture we mirror for event delta computation. The 2023 "durable incrementality" piece is the canonical statement of the invariant we extend to LLM-edit segregation.
- **LSP 3.17** ([spec](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/)) + LSP issue #737 (custom notification ergonomics). LSP 3.18 added `textDocument/inlineCompletion`; RFC-008's `shape/inference` is a server-driven analogue.
- **Cursor / Continue / Codeium architectures** ([Cursor reverse-engineering, TensorZero](https://www.tensorzero.com/blog/reverse-engineering-cursors-llm-client/), [Cursor 2.0 / Composer](https://www.artezio.com/pressroom/blog/revolutionizes-architecture-proprietary/)). These spend most of their context budget on retrieval to *re-derive* facts a language server already has. RFC-008 is the converse bet: surface the facts directly, let the LLM spend its budget on reasoning.
- **MerlinJS / merlin for OCaml.** The original "LSP for AI doesn't need to be invented — make the existing LSP smarter" position.
- **Smalltalk image-based development / Pharo / Glamorous Toolkit.** The lineage of "the running system knows things; surface them" that Hazel and this RFC both inherit from.
- **Bret Victor's "Inventing on Principle" / "Learnable Programming."** The principle: tools should make the invisible visible. RFC-008 makes the invisible (compiler-internal facts) visible to a non-human consumer.

## Unresolved questions

- **Digest algorithm weights.** The initial constants in §Ranking are guesses. We need an offline benchmark (the Shape-LLM-coding benchmark gating v1) to tune them. Open question: should weights be per-LLM-model? Probably no for v0.1, possibly yes later.
- **Cross-LLM-session persistence.** The N=64 snapshot retention is arbitrary. For a long-running agentic session (Claude Code with subagents), 64 may be too few. For a one-shot completion, it's wasteful. Open: ring-buffer with size hint from the MCP client's `Implementation.info`?
- **Should `Origin::Llm` edits *ever* feed back into the user-edit channel?** A counterargument: when the user accepts a proposed LLM edit, it morally becomes a user edit. Rule: acceptance is a `Tooling`-stamped re-emission of the LLM's diff, not a relabeling. This may be wrong for some workflows; deferred.
- **Per-family perf cost.** We have no measurement of incremental computation cost for `attractor_match` (regex/AST walk over the whole document) or `duplicate_candidate` (workspace-wide `behavior_hash` lookup). These may dominate. Open: per-family opt-out at subscription time.
- **Coactivation graph cycles.** When `x` constrains `y` and `y` constrains `x`, we emit two events. The LLM might benefit from seeing the cycle explicitly. Open: should `coactivation` events carry a graph-edge ID for cycle detection?
- **Token-stream-position events.** Could we tag each emitted event with the LLM's *token offset* in its current generation, so the LLM correlates "I emitted token T₅₃, the channel told me X" with later regret/replan? Speculative; out of scope for v0.1.
- **Wire-protocol multiplexing.** RFC-008 events go over LSP today. Should they also flow over `shape-wire` ([crates/shape-wire/src/lib.rs:51](crates/shape-wire/src/lib.rs)) for distributed execution scenarios where the LLM is on a different host than the LSP? Deferred to v0.2.

## Future possibilities

- **Full Hazelnut-Live semantics (v1).** Every partial AST well-typed, every hole carrying an evaluation closure. Gated on v0.1 benchmark results per §Phasing. 3–5 engineer-years if pursued; possibly never pursued.
- **Cross-LLM-session channel persistence.** Beyond per-document retention, persist channel history at the workspace level so a new model attaching mid-task gets a richer warm-start.
- **Channel as training signal for LLM fine-tuning.** The (cursor-position, channel-snapshot, next-token-accepted) triple is a rich training signal. With user consent and on-device aggregation, fine-tuning a small model specifically on this signal is plausible. Pure speculation for the RFC; called out so we don't accidentally architect it away.
- **Runtime-driven event sources.** `crates/shape-vm/src/feedback.rs` IC state machine (Uninitialized → Monomorphic → Polymorphic → Megamorphic) is a natural source of `feedback_hint` events: "this call site has gone polymorphic; consider a typed wrapper." Out of scope for v0.1 (it's runtime, not compile-time), but a clean future extension.
- **Channel snapshots as Shape values.** Per CLAUDE.md `snapshot()`, the VM can capture full state. Lifting an inference channel snapshot into a first-class `InferenceSnapshot` Shape value would let user code introspect "what does the compiler think about me right now" — useful for `@law`-style meta-programming.
- **Event-family plugins.** A trait `InferenceEventProducer` would let third-party Shape extensions register new event families (e.g., `domain_model_warning` for a healthcare DSL). Out of scope; called out so the v0.1 schema reserves an `Extension(name, payload)` variant.

## Phasing and cost

### v0.1 — Enriched LSP + MCP subscription. **Budget: 12–18 engineer-months.**

| Workstream | Cost |
|---|---|
| LSP custom notification + schema in `shape-diagnostics` | 1 EM |
| Event emitters for the 4 families with existing source data (`coactivation`, `narrowing`, `permission_inference`, `duplicate_candidate`) | 3 EM |
| Attractor-pattern registry + walker (new event family) | 2 EM |
| `law_witness` integration (depends on RFC-002 runner being live) | 2 EM |
| `intent_collision` integration (depends on RFC-007 corpus being live) | 1 EM |
| `shape-mcp` `subscribe_inference` / `get_inference_snapshot` tools | 1.5 EM |
| `Origin` tagging through editor / MCP / refactor paths | 1.5 EM |
| Ranking + digest + persistence | 1 EM |
| Pre-registered Shape-LLM-coding benchmark + gate measurement | 2 EM |
| Docs, error stories, examples | 1 EM |

Total: ~16 EM. Two engineers, ~8 months. One engineer, ~16 months. The first 8 EM are *additive* on top of existing infrastructure — no rewrite, no protocol fork, no compiler invariant change.

### v1 — Full Hazelnut-Live semantics. **Hard-gated.**

> v1 is not scheduled. v1 is *eligible* once v0.1 ships and the benchmark gate fires positive.

If eligible, expected cost is 3–5 engineer-years. Approach to be re-RFC'd at that time, informed by:

- What v0.1 measurement revealed about which event families matter most.
- Whether mechanizing Shape's type system has happened in the interim (independent track).
- Whether the Hazel group's Grove calculus (POPL 2025) provides a more incrementally portable foundation than the original Hazelnut Live.

### Acceptance criteria for v0.1

1. `shape/inference` notification ships behind a client capability flag (`shape.experimental.inferenceChannel`).
2. All seven event-family schemas are versioned per LSDS SCHEMA_VERSION discipline.
3. `Origin::Llm`-stamped edits do not produce events on the default subscription channel. CI test enforces.
4. `subscribe_inference` returns within 50 ms p95 on a 5k-LOC document.
5. The Shape-LLM-coding benchmark exists, is pre-registered, and produces a measured baseline number for v0.0 (LSP diagnostics only) before v0.1 ships.
6. The CLAUDE.md §Forbidden Patterns regex passes against the new code paths: no "bridge", "shim", "adapter", "fallback", "compatibility layer" rationalizations in the inference-channel implementation, and no parallel-implementation framings that would reconstruct deleted dispatch shapes.

### Why this cost is justified

The shape of the LLM authoring loop is the single biggest lever on Shape's adoption story. We have a working LSP, a working inference engine, and a working MCP server. The marginal cost of *connecting them properly* is small. The marginal value — if the benchmark shows it — is the difference between "Shape is a nice language with an LSP" and "Shape is the first language whose compiler talks to the LLM at token-generation cadence."

The phasing makes the bet legible. v0.1 is cheap and reversible. v1 is expensive and gated. Neither commits us to the kind of mid-execution downgrade the codebase has had to clean up before.
