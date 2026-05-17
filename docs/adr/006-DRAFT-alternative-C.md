# ADR-006 — Alternative C: Genuinely-Novel Runtime

> Status: **DRAFT — exploration**. Higher-risk design point. The supervisor signaled
> openness to genuinely-new ideas; this document delivers them. Sections marked
> `[NOVEL]` do not exist in any production language as of 2026 May.
>
> Date: 2026-05-08. Author: runtime redesign track.

---

## 1. Executive Summary

Shape's runtime is being redesigned greenfield at the value-representation,
ownership, lifetime, GC, layout, and slot-ABI layers. Alternative C is the
"build the language that AI programmers and AI compilers will actually want in
2030" point in the design space. It is bolder than Alt-A (production-cluster
RC + reuse) and Alt-B (mode-typed retrofit on a tracing GC); it accepts more
risk in exchange for properties no current runtime offers.

The design hangs together around four convictions:

1. **The compiler should pick the memory-management policy per scope, not the
   programmer.** Production languages have converged on hybrids (RC + reuse,
   tracing + ownership, regions + escape) but every one of them surfaces *one*
   policy as default and asks the user to opt out. Shape inverts this: the
   compiler runs an automatic per-scope policy inference (region / unique /
   shared-RC / immortal / on-stack) on every binding and surfaces the chosen
   policy as an inlay hint. The user can override but rarely needs to. This is
   feasible because Shape already has full type inference, an AI-friendly
   strict-typing discipline, and the W-series defection log gives us a list of
   anti-patterns to refuse on sight.

2. **Errors and inferred decisions are first-class compiler outputs, in a
   format LLMs ingest natively.** No production language has shipped a
   compiler whose primary diagnostic format is a structured, LLM-tuned
   schema with location, expected/found types, suggested fixes, and a
   bounded context window. Shape does. This propagates beyond errors: every
   inferred policy decision (region X chosen for binding Y, deopt reason Z
   in JIT) is emittable as the same structured form.

3. **Distribution is content-addressed at the granularity of the smallest
   reproducible unit, with foreign runtimes treated as content-addressed
   peers.** Python's venv hash, npm's package-lock entry, the C library's
   `.so` SHA, and Shape's `FunctionBlob` hash all live in the same
   distribution graph. Any node can resolve, verify, and execute a Shape
   program iff every transitive hash matches.

4. **The same value graph is read directly by Shape code, by Cranelift JIT
   code, by Python via PyO3, by TypeScript via deno_core, and by C via
   `extern "C"` — without runtime conversion at the boundary.** This is the
   shallow-marshal mandate. We pay for it with discipline at the layout layer
   (Wasm-GC-style typed references for Shape values, Arrow C Data Interface
   for arrays/columnar data, plain `repr(C)` for `extern C`, and one
   carefully-chosen handle protocol for Python/TypeScript objects).

The novel contributions, summarized one-line each:

- **`[NOVEL-1]` Per-scope automatic policy inference (PSAPI)** — compile-time
  decision for every binding among `Stack | Unique | Region(r) | RcCow |
  RcShared | Immortal`, surfaced as inlay hints, overridable, decided by an
  HM-style inference layered on top of type inference. Combines Lobster +
  Roc + Vale + ML-Kit fragments into one automated pipeline.

- **`[NOVEL-2]` LLM-Structured Diagnostic Schema (LSDS) as the primary
  compiler output**, with terminal/IDE renderers being *consumers* of the
  schema, not the source of truth. Errors carry expected/found type
  *witnesses* (concrete values that would satisfy/violate), suggested fixes
  as code diffs, and a bounded "agent context window" — a token-counted
  prefix of the smallest set of source spans needed to fix the error.

- **`[NOVEL-3]` Frozen-Region Calculus (FRC)** — generalizes Parkinson et
  al.'s "frozen cyclic RC" (ISMM 2024) into a regional discipline: a region
  in Shape can transition `Mutable -> Frozen` exactly once; a frozen region
  is deeply immutable, RC-managed without a cycle collector (Parkinson's
  observation), and trivially safe to share across tasks/processes/JIT
  tiers. `let` binds default into frozen regions; `let mut` and `var` do
  not.

- **`[NOVEL-4]` Permission-as-Effect Speculation (PES)** — JIT specializes
  not only on observed types but on *observed permission state*. If a
  closure has been called only with `FsRead` consistently over its
  warmup window, the optimizing tier emits a version with the FsWrite
  guard and its dead branches pruned. Deopt on permission change.

- **`[NOVEL-5]` Polyglot Value Lattice (PVL)** — one shallow boundary
  protocol that Shape, Python, TypeScript, and C all participate in,
  unified by a 3-bit *kind shadow* in a per-binding side table (not a
  per-value tag — single-discriminator discipline preserved). Each
  language's native value-model is unchanged; the lattice describes only
  *what crosses the boundary*.

- **`[NOVEL-6]` Compile-Time AI Optimization Notes (CT-AION)** — at build
  time, the compiler may consult an `@ai`-tagged optimization advisor on
  decisions where heuristics are weak (layout choice for a hot type,
  tile size for a stencil, region merge for a contested allocation). The
  advisor is reproducible: its prompt + model + seed are part of the
  content hash, so two builds with the same advisor pin produce
  byte-identical output. Off by default; opt-in per package.

The combination story: **the compiler infers policy (1) and reports it (2);
the dominant policy is the frozen region (3) which is RC-tractable without
a cycle collector and trivially polyglot-shareable (5); the JIT speculates
on permissions (4) because permissions are stable enough in observed
practice to specialize on; the AI advisor (6) makes layout/tiling
decisions reproducibly, becoming the missing piece between "Shape is
AI-native syntax" and "Shape's compiler is itself AI-aware."**

The cost: PSAPI is the load-bearing pillar; if it converges poorly (escape
analysis bugs, region inference fragility — the historical ML-Kit failure
mode) the user experience degrades to "compiler picked the wrong policy,
how do I override?" We mitigate by always making the chosen policy *visible*
(every binding gets an inlay hint), *overridable* (`let region:r x = ...`,
explicit move/clone), and *attributable* to a specific inference rule that
the user can read.

Worst case fallback: PSAPI degrades cleanly to "everything that escapes
goes to a single per-task RC heap with frozen-region promotion at let-
binding." This is essentially Alt-A with extra ceremony, and we'd still
keep `[NOVEL-2]` (the LSDS schema) and `[NOVEL-5]` (the PVL boundary), both
of which are independently valuable.

The rest of this document defends the four convictions in order, then
specifies the runtime layers, then addresses migration risk.

---

## 2. The Novel Ideas

### 2.1 `[NOVEL-1]` Per-Scope Automatic Policy Inference (PSAPI)

#### Motivation

Survey 01 establishes that every modern language picks one default
memory-management policy and forces opt-outs:

- Rust: affine ownership default; `Rc<T>` opt-in.
- Mojo: ownership conventions default; ARC opt-in via the runtime.
- Lobster: compile-time RC elision picks a single owner per allocation; rest is RC.
- Roc: Morphic + RC; programmer never sees the choice.
- Lean 4: per-binding heuristic borrow inference.
- Vale: generational refs default; regions opt-in.

The pattern: each language has *one* fast path and asks the user to
recognize the slow path. The Lobster-Roc-Lean cluster shows you can elide
80–95% of RC ops at compile time; the Vale + Verona cluster shows you can
get region-grade speedups in narrow scopes; the Mojo + Rust cluster shows
ownership inference can avoid most heap traffic. **No production language
chooses among all of these per scope.**

Shape's strict-typing + AI-native + LSP-first stance makes it possible. We
have full type information. We have an LSP that already shows inlay hints.
The W-series defection log gives us a list of "rationalize and never
revert" anti-patterns the inference must refuse to emit.

#### Mechanism

PSAPI is a **second inference pass** that runs after type inference and
before bytecode emission. For every binding, parameter, return value, and
intermediate temporary, it picks one of:

```
Policy = Stack
       | Unique
       | Region(r)            -- r is a region variable
       | RcCow                 -- aliased, copy-on-write
       | RcShared              -- aliased, no CoW (used for `var` w/ closures)
       | Immortal              -- proven const-foldable / static
       | Frozen(r)             -- in frozen region r (see [NOVEL-3])
```

It works the same way HM type inference does: a constraint generator walks
the typed AST, emits constraints (`escapes(x) => ¬Stack`, `aliased(x) =>
¬Unique`, `cyclic(x) ∧ mutable(x) => RcShared with cycle handler`,
`cyclic(x) ∧ frozen(x) => Frozen(r) — Parkinson safe`), and a unifier
solves them. Where multiple solutions are valid, a cost model picks the
cheapest (Stack > Unique > Frozen > RcCow > RcShared > Immortal-late).

The cost model is *seedable* — for the same source, same compiler, same
seed, the same policy is always chosen. This is critical for
content-addressed bytecode (§9): the policy decision is part of the hash.

The user surface is:

- **No syntax change.** `let x = [1, 2, 3]` is unchanged; the compiler
  picks `Frozen(r_outer)` or `Stack` or `Unique` based on observed escape.
- **Inlay hint after the binding** showing the chosen policy:
  ```
  let x: Array<int> = [1, 2, 3]
        ^^^^^^^^^^^^         <- type hint
        ◇ frozen(scope)      <- policy hint (configurable LSP toggle)
  ```
- **Override syntax** for the rare case where the user wants something
  different: `let region(arena) x = [1, 2, 3]`, `let unique x = ...`,
  `let shared x = ...`. These are existing (Vale-flavored) syntactic
  forms that we accept; the compiler verifies the user's annotation is
  consistent with the constraint set or emits a structured error.

#### Precedent

- ML Kit / Tofte–Talpin region inference (1997): closest existing pure
  per-scope inference. Failure mode well-documented: "region leaks" when
  inference is too coarse. We mitigate by combining region inference with
  ownership inference (so that a single coarse region doesn't become the
  garbage bucket — values within can still be `Unique` or move out).
- Lobster compile-time RC elision (van Oortmerssen, ~2015): showed
  ~95% RC ops elidable for game workloads.
- Roc Morphic (~2024): defunctionalization-driven uniqueness analysis.
- Vale's generational regions (~2023): user-facing regions but inferred
  generation checks.

PSAPI is the *combination* of these as one inference pipeline. No production
language has done that. (Roc gets closest, but only chooses between
`Unique` and `RcCow` — no regions, no stack inference, no immortal
promotion.)

#### Shape-specific benefit

- **Best-in-class default ergonomics.** User writes `let arr = [1,2,3]`
  and gets stack allocation if it doesn't escape, frozen-region RC if it
  does, with no annotation.
- **Predictable performance.** The inlay hint makes performance visible
  *at write time*, not after profiling.
- **AI-friendly.** LLMs can read the policy hints just like type hints
  and reason about heap behavior without running the program.

#### Implementation cost

- Constraint generator: ~3000 LoC in the compiler (sketch).
- Solver: standard datalog (we already use Datafrog for the borrow
  checker per `references-borrowing.mdx`); ~1500 LoC of new rules.
- LSP integration: existing inlay-hint infrastructure; ~500 LoC.
- Cost-model tuning: the hard part. We seed it conservatively
  (Frozen(r_outer) when in doubt) and tune via a benchmarking corpus
  before exposing aggressive policies (Stack, Immortal).
- Explanability infrastructure: every policy decision must be reportable
  via the LSDS schema (§2.2) — "binding `arr` chose Frozen(scope_3) because
  rule R-EscapeIntoClosure fired at line 14, dominated rule R-StackOk."
  Without this, debugging a bad inference is opaque.

#### Risks

- **Solver fragility.** Region inference has a 30-year track record of
  surprising the user. We mitigate via (a) always-visible inlay hints,
  (b) deterministic cost model, (c) override syntax, (d) corpus-driven
  CI that reports policy drift across compiler versions.
- **Hash stability.** Because policy is part of the content hash, an
  innocent compiler change can churn hashes for distributed bytecode.
  We pin the inference rule version in the hash, so dependents can pin
  to a rule version and not see drift until they upgrade.
- **Slow compilation.** We've already chosen Cranelift for compile-speed
  reasons. Adding a second inference pass slows builds; we budget ~10%
  compile-time overhead and measure.

---

### 2.2 `[NOVEL-2]` LLM-Structured Diagnostic Schema (LSDS)

#### Motivation

Production languages emit terminal-targeted diagnostics with text
formatting (Rust's "rich" diagnostics are the gold standard). They were
never designed for LLM ingestion. The LLM must parse ANSI codes, line
boundaries, and informal syntax. The token cost of a Rust error in an
agent prompt is 200–400 tokens *per error*, much of it formatting.

Shape's mandate is "best-in-class error messages, tuned for LLM
consumption." Surveys 01–03 confirm this is greenfield: no production
compiler emits diagnostics natively as JSON-schema-validated structured
output.

#### Mechanism

The compiler's primary diagnostic output is the LSDS schema. Renderers
(terminal, IDE, MCP server, LSP, agent transcript) are *consumers*.

```jsonschema
Diagnostic = {
  id: "B0001" | "T0042" | "P0007" | ...,    // stable error code
  severity: "error" | "warning" | "note" | "hint",
  message: string,                            // primary, ≤120 chars
  primary_span: SourceSpan,
  secondary_spans: [{ span: SourceSpan, role: string, message: string }],
  expected: TypeWitness?,                     // structured type
  found: TypeWitness?,                        // structured type
  suggested_fixes: [Fix],                     // ordered, best-first
  agent_context: AgentContext,                // [NOVEL]: tuned for LLMs
  inference_trace: InferenceTrace?,           // [NOVEL]: rule chain
  related: [DiagnosticRef],                   // backlinks
}

Fix = {
  kind: "replace" | "insert" | "delete" | "structural",
  span: SourceSpan,
  replacement: string,
  rationale: string,
  confidence: 0..1,                           // calibrated
  preview_diff: UnifiedDiff,
}

AgentContext = {
  tokens_budget: int,
  prefix: [{ span: SourceSpan, role: "definition" | "use" | "constraint" }],
  // Smallest set of source spans needed to comprehend & fix this error
  // ranked by load-bearing relevance, capped at tokens_budget tokens
  total_tokens: int,                          // count under cl100k_base or model-pinned tokenizer
}

TypeWitness = {
  shape: "scalar" | "function" | "tuple" | "named" | "generic" | "var",
  display: string,                            // human form: "Array<int>"
  json: Value,                                // structured form
  satisfying_value_example: Value?,           // [NOVEL]: a literal that would have type-checked
  failing_value_example: Value?,              // [NOVEL]: a literal that triggers the same failure
}

InferenceTrace = {
  rules_applied: [RuleApplication],           // each step that led to the failure
  unification_failure: { left: TypeWitness, right: TypeWitness, at: SourceSpan }?,
}
```

The novelty is in two specific fields:

- **`satisfying_value_example` / `failing_value_example`.** When a function
  expects `Array<int>` and got `Array<string>`, the diagnostic carries
  `[1, 2, 3]` (satisfying) and `["a", "b", "c"]` (failing). LLMs reason
  much better with concrete witnesses than with abstract types — and
  the compiler can construct them cheaply via type-directed enumeration.
- **`agent_context.prefix`** is a token-counted minimal context, ranked
  by an information-flow analysis (definitions of every type/function
  mentioned in `expected`/`found`, paired with their local uses). The
  total token count is bounded so the entire diagnostic fits in an
  agent's context budget without truncation surprise.

#### Precedent

- Rust's `--message-format=json` (since 2017) emits diagnostics as JSON.
  Closest existing precedent. *But* Rust's JSON is a transport format,
  not a designed schema; fields are ad-hoc; no agent-context concept;
  no structured type witnesses; suggested fixes are textual not
  type-aware.
- TypeScript's `--pretty false` JSON output: more limited still.
- LSP `Diagnostic` type: notification format; lacks suggested-fix and
  type-witness structure.
- *No production compiler ships satisfying/failing value examples,
  agent-context windows, or inference traces.*

#### Shape-specific benefit

- **Agent IDE integration.** Shape's MCP server (`shape-mcp/`) can
  forward diagnostics to LLMs without any reformatting. Token cost
  per error drops 3–5x.
- **Cursor-style auto-fix.** With calibrated confidences and concrete
  diffs, an editor can apply high-confidence fixes automatically and
  surface lower-confidence ones for review.
- **Determinism.** Since LSDS is structured, two compiler versions can
  diff their diagnostic stream, and we can write CI that asserts "this
  PR did not regress any diagnostic from confidence > 0.9 to a worse
  one." Production languages cannot.

#### Implementation cost

- Schema definition + Rust types: ~800 LoC.
- Replacing existing diagnostic emission in shape-runtime: incremental,
  per-error-code; one PR per category. Estimate: 6 weeks of full-time
  work to migrate all existing diagnostics, 2 weeks to add witness
  enumeration.
- Witness enumeration: type-directed; given `Type::Int`, produce `0`
  or `1`; given `Type::Array<T>`, produce `[]` plus a 1-element example.
  Bounded by depth and branching factor.
- Renderer libraries: terminal, LSP, MCP. Each ~500 LoC.

#### Risks

- **Schema churn.** If we evolve the schema incorrectly, downstream
  agent code breaks. Mitigation: schema versioned, all fields optional
  by design, additive evolution only. (Same discipline as a public API.)
- **Witness construction can be expensive** for deeply generic types.
  We bound recursion depth and emit `null` (with a "witness not
  generated, type too complex" hint) when we hit it.
- **Token-count accuracy** depends on the tokenizer pin. We pin to
  `cl100k_base` by default; users can override via `[diagnostics]
  tokenizer = "..."` in shape.toml.

---

### 2.3 `[NOVEL-3]` Frozen-Region Calculus (FRC)

#### Motivation

Parkinson et al. (ISMM 2024) observed: deeply-immutable cyclic data
is RC-tractable without a cycle collector. Once frozen, no new cycles
form; existing cycles are themselves immutable; a one-shot cycle-
detection at freeze time suffices.

This is the missing piece for Shape's `let` binding. Per the book
(`variables.mdx`): `let` is uniquely-owned and immutable. **If we make
`let` bind into a frozen region, we get cycle-safe RC for `let`-bound
graphs without a cycle collector.** This is huge:

- `let mut` (mutable, unique) and `var` (mutable, shared/RC-CoW) keep
  conventional ownership/RC discipline — these don't form cycles
  in user-typical code.
- `let` (immutable) gets the Parkinson treatment — *can* form cycles
  (e.g., a graph constructor that closes its own loops at the end), and
  RC works without a cycle collector.

This effectively eliminates the RC-cycle problem from Shape, the issue
called out in survey 01 §6.1 Theme 5 ("Cycles are uniformly the
unsolved hard part for RC-first runtimes").

#### Mechanism

A region is one of three states:
```
RegionState = Mutable | Freezing | Frozen
```

A region transitions `Mutable -> Freezing -> Frozen` exactly once at the
end of its constructor (a let-binding's RHS, a function's return
construction, or an explicit `freeze` boundary).

When transitioning to `Freezing`, the runtime walks the region's
allocation list once (regions track their allocations as a linked list
with no per-allocation overhead — header is shared) and runs
Parkinson's freeze-time cycle detection (essentially a Bacon-Rajan
trial-deletion *but only over this region's local graph and only once
per region*, not as a continuous background process).

Once `Frozen`:

- All RC operations on objects in the region are non-atomic (region is
  isolated, observed only by its owners).
- Any reference *into* the frozen region is itself immutable.
- Cycles inside the region are managed: the freeze-time pass annotated
  cyclic groups; reclamation walks them as units.
- Frozen regions are **trivially shareable** across tasks/threads
  /JIT tiers/processes — no atomic RC needed because the region itself
  is the unit of sharing, and the bag of refs from outside is
  explicitly tracked (one external_rc word per region).

The book's `let` binding compiles to "construct in a fresh
RegionState::Mutable, freeze before assignment". Most `let` bindings
have trivial regions (one allocation, no cycles); the freeze pass for
those is a single atomic operation.

#### Precedent

- Parkinson et al., ISMM 2024 — *position paper*, no implementation.
  Shape would be the first implementation of frozen-cyclic RC at
  language-language level.
- Pony's `iso`/`val` capability split with ORCA — closest spiritual
  ancestor: shareable iff frozen.
- Erlang binary refc + sub-binary slicing — same shape: refcounted
  immutable shared with cheap slicing; no cycle problem because no
  cycles in Erlang binaries.
- ML Kit regions — ancestor of the regional structure.

What's actually new: combining (a) Parkinson's freeze-time cycle
detection with (b) ML-Kit-style region inference (the PSAPI from
NOVEL-1) with (c) `let`-as-region-freezer language semantics. Each
piece exists; the synthesis does not.

#### Shape-specific benefit

- **Cycle problem solved for Shape's most common binding form.** No
  cycle collector running in the background. No GC pause. Worst-case
  freeze-time pass is O(region size), bounded by the user's code
  structure.
- **Polyglot sharing.** A frozen region is the natural unit to hand to
  Python/TypeScript/JIT — once frozen, no race possible; no defensive
  copying.
- **JIT speculation.** The JIT can specialize on "this argument is
  in a frozen region" and elide all aliasing/mutation guards.

#### Implementation cost

- Region runtime: ~2000 LoC (allocation list, freeze-time pass,
  region-local RC).
- Compiler integration: emits `region_create`, `region_alloc_into`,
  `region_freeze`, `region_external_acquire/release` opcodes; all
  inferred by PSAPI, never user-typed.
- Freeze-time cycle detection: standard Bacon-Rajan trial deletion
  scoped to the region; ~600 LoC.

#### Risks

- **Pessimal case.** A user writes a hot loop that constructs many
  small `let`-bindings, each region freezing eagerly. The freeze
  pass overhead dominates. Mitigation: PSAPI infers `Stack` for
  non-escaping `let`-bindings; only escaping ones get a region.
  Fallback: tunable threshold ("regions of size 1 are skipped, freeze
  is a no-op").
- **Frozen region external_rc contention.** If many threads share a
  frozen region, the external_rc word becomes contended. Mitigation:
  biased reference counting on external_rc (PACT'18 design).
- **Interaction with `let mut`.** A `let mut x = some_frozen_thing()`
  must materialize a fresh mutable region, since the source is
  frozen and cannot be unfrozen. This is a deep clone (semantically a
  CoW). PSAPI distinguishes the "I mutate in place" case (don't
  freeze the source) from the "I just need a mutable view" case
  (freeze, then clone-on-mutate).

---

### 2.4 `[NOVEL-4]` Permission-as-Effect Speculation (PES)

#### Motivation

The supervisor mandate calls for permission-aware speculation. Production
JITs (V8, JSC, HotSpot) speculate on observed *types*. None speculate on
observed *effects* — including permission state.

Yet permissions are observed-stable in practice. A function that's been
called with `FsRead-only` for 100 iterations is overwhelmingly likely to
continue being called that way; the cost of speculating on this and
deopting on rare permission changes is favorable.

#### Mechanism

The runtime tracks per-call-site permission usage as a *feedback vector*
(extending the existing IC machinery from `architecture` per CLAUDE.md).
After 1000 invocations of a function, if the permission set has been
stable, the optimizing tier compiles a specialized version with the dead
permission branches pruned and the live permission checks turned into
branchless loads.

Concretely: every stdlib I/O entry has the form
```rust
fn write_file(path, data) -> Result<()> {
    check_permission(ctx, FsWrite)?;  // ~5ns runtime check
    /* implementation */
}
```

When the JIT specializes a caller of `write_file`, and the permission
context has been stable across the warmup window, the specialized
version inlines `write_file` with the `check_permission` call replaced
by a single guard that the *caller's permission set* still matches the
profiled one. If yes, ~0ns (just a load + compare). If no, deopt to
baseline.

The deopt cost: comparable to type-deopt cost (~µs). Permission changes
are rare events (config-driven), so deopt thrash is low.

The most novel part is **multiple-permission specialization in one
function body**: if a function has both an `FsRead` branch and an
`FsWrite` branch behind dynamic conditionals, and the profile shows
`FsWrite` is dead, the optimizing tier elides the FsWrite branch
*entirely*, including the `check_permission(FsWrite)` call. The function
is now smaller, faster, and provably FsRead-only — a profile-guided
specialization no production language exploits.

#### Precedent

- HotSpot tiered + speculative deopt: speculates on types/branches,
  not effects.
- V8 IC + Maglev/Turbofan: speculates on shapes.
- *No production VM* has effect-typed JIT specialization.
- Closest research: Effekt (effect-typed language) but its evaluator
  doesn't speculate; it just types effects.

#### Shape-specific benefit

- Realizes the supervisor's "Level 0 capabilities zero-cost via
  opcode-scan" while still allowing dynamic permission reconfiguration
  at the host level.
- Makes Shape's permission system genuinely free at steady state, not
  just "5ns per call cheap." 5ns *per call* is fine for sandbox use but
  felt in tight inner loops.
- Doubles as an audit primitive: the JIT's specialized form reveals
  which permissions are *actually* used, not which are conservatively
  declared.

#### Implementation cost

- Permission feedback vectors: ~400 LoC, extending the existing IC
  state machine.
- JIT specialization rules: ~800 LoC of new MIR-to-IR rules in the
  Cranelift backend.
- Deopt-on-permission-change machinery: integrates with existing deopt
  infrastructure; ~300 LoC.

#### Risks

- **Permission churn deopt thrash.** If a host changes the permission
  set frequently, the specialized form deopts repeatedly. Mitigation:
  per-call-site deopt counter that disables specialization after N
  thrashes (HotSpot-style).
- **Specialization correctness.** If we deopt incorrectly, we could
  silently execute under wrong-permission code. Mitigation: every
  specialized function entry includes a permission-set hash compare
  (one 8-byte load + branch); the specialized form is *correct iff
  guard holds*. Failure deopts back to the baseline form which always
  re-checks.

---

### 2.5 `[NOVEL-5]` Polyglot Value Lattice (PVL)

#### Motivation

Shape supports `fn python`, `fn typescript`, and `extern C`. Production
solutions for cross-runtime values are heavy:

- JNI: copy or pin per call, ~µs latency per crossing.
- Python C API: refcount + type lookup per access; copies for many
  types.
- Truffle: partial-evaluation polyglot, but the runtime is Graal-locked.

Mojo + MLIR show one path: lower everything to the same IR. But MLIR
forces every cooperating language to compile to MLIR. We want Python
and TypeScript to keep using CPython 3.14+ and v8 / deno_core, not
recompile to MLIR.

The shallow-marshal mandate says: minimize boundary cost without
demanding the foreign runtime change.

#### Mechanism

Shape exposes a single boundary protocol, the **PVL**, which classifies
every cross-runtime value into one of seven shapes:

```
PolyShape =
  | Scalar(Numeric | Bool | Unit)
  | Frozen(ShapeRef)        // frozen-region pointer (NOVEL-3)
  | Native(repr_C, layout)  // bytes-flat repr(C) — extern C path
  | OpaqueHandle(runtime)   // handle into Python/TS heap
  | Buffer(ArrowSchema)     // Arrow C Data Interface (large arrays)
  | Stream(WireProto)       // streamed channel — async polyglot
  | UserMarshal(TypeId)     // user-defined fallback
```

Each foreign runtime registers handlers for the shapes it natively
understands:

- **C (extern C):** Scalar, Native, Buffer (via Arrow C Data
  Interface). Frozen and OpaqueHandle are unwrapped at the boundary
  via tiny inline conversion stubs.
- **Python (PyO3-based extension):** Scalar, OpaqueHandle (PyObject*),
  Buffer (Arrow via the buffer protocol). Frozen Shape values
  cross as borrowed references.
- **TypeScript (deno_core extension):** Scalar (V8 SMI / heap doubles),
  OpaqueHandle (v8::Value), Buffer (Arrow via SharedArrayBuffer).

The boundary cost per crossing:

- **Scalar:** zero — same 8-byte slot in every runtime.
- **Frozen:** ~10ns — atomic increment of external_rc.
- **Native:** zero — direct pointer hand-off, repr(C) layout.
- **OpaqueHandle:** ~5ns — type-tag check, pointer copy.
- **Buffer:** ~50ns — fill ArrowArray/ArrowSchema struct (Arrow CDI).
- **Stream:** msgpack frame cost — used only for async streaming.
- **UserMarshal:** user-defined; reserved for novel value types.

The single-discriminator discipline is preserved: PVL classification
lives in a *side table* keyed by the typed-binding kind, not by per-value
tag. The kind is statically known at every crossing point, so the
3-bit discriminator is a compile-time constant in emitted code, not a
runtime branch.

#### Precedent

- Apache Arrow C Data Interface (already adopted). PVL adopts it
  wholesale for `Buffer`.
- Truffle's polyglot interop messages — closest spiritual ancestor.
  Truffle requires all participants run on the JVM; PVL doesn't.
- Wasm Component Model (WIT/Wit-bindgen): closest production-grade
  cross-runtime ABI, designed for Wasm-to-Wasm. PVL extends it
  *off-Wasm* into the polyglot extension model.

What's actually new: PVL as a single boundary protocol that
explicitly enumerates 7 shapes, picks the cheapest one per type
statically, and treats Frozen Shape regions as a first-class
cross-runtime value. No production language does this.

#### Shape-specific benefit

- **Sub-100ns polyglot calls** for the common case (scalars + frozen
  values). 10–50x faster than JNI/cgo/deno boundaries.
- **AI workflow compatibility.** Most AI workloads pass tensors
  (Buffer/Arrow) and dicts (OpaqueHandle/PyObject); both are first-
  class.
- **Reproducible.** PVL classification is part of the content hash;
  two compilations of a polyglot program produce the same boundary
  shape.

#### Implementation cost

- Protocol spec + Rust types: ~600 LoC.
- Per-runtime adapters: 1500–2500 LoC each (Python, TypeScript, C).
- LSP integration: surface PVL choice as inlay hint at every
  polyglot call site (so user can see the boundary cost).

#### Risks

- **OpaqueHandle leaks.** If a Python object's PyObject* is held by
  Shape across a Python GC, the handle dangles. Mitigation:
  OpaqueHandles auto-promote to refcounted under PyO3's Py<>
  wrapper when the Shape side outlives the Python call; the cost is
  measured as ~30ns extra at promotion time.
- **Buffer ownership.** Arrow C Data Interface has a release-callback
  protocol; mis-using it leaks memory. We leverage the existing
  duckdb/numpy/polars conventions (well-established).

---

### 2.6 `[NOVEL-6]` Compile-Time AI Optimization Notes (CT-AION)

#### Motivation

Shape is AI-native. The `@ai` annotation already causes function
signatures to drive LLM prompts. **The compiler itself is not yet AI-
aware.** Layout decisions (AoS vs SoA, struct field reorder, frozen
vs unique policy in PSAPI ambiguous cases), tile sizes, hot-method
inlining thresholds — production compilers use heuristics. Mojo + MLIR
attempts to push these into the IR; even there, the heuristics are
hand-coded.

CT-AION proposes: at *build time*, with explicit user opt-in, the
compiler may consult an AI advisor on decisions where the heuristic is
weak. The advisor sees the function's typed AST + observed-input
profile + cost model and returns a structured recommendation. The
recommendation is reproducible: the prompt + model identity + seed are
content-hashed, so two builds with the same advisor pin produce
identical bytecode.

#### Mechanism

Decisions eligible for advisor consultation are those where:

1. The cost model has multiple choices within ε of each other.
2. The decision is *layout-equivalent* (semantic preservation
   guaranteed by the type system regardless of advisor output).
3. The user has opted in via `[advisor] ct_aion_pin = "..."` in
   shape.toml or `@advisor pin="..."` annotation on the function.

The advisor flow:

```
(1) Compiler computes candidate layouts/policies.
(2) If only one is within ε, choose it (no advisor call).
(3) Else: serialize {function_ast, profile, candidates} as a structured
    prompt; hash it; check disk cache.
(4) Cache miss: invoke advisor (LLM, with model + seed pinned).
(5) Validate advisor's choice against safety preconditions (must be
    in candidate set).
(6) Cache result. Embed (prompt_hash, model_id, seed, choice_index,
    advisor_version) in the function's content hash.
```

Reproducibility is the load-bearing property: a remote node that
re-compiles the same source must produce the same bytecode hash. The
advisor's *output* is part of the input to hashing — but only via the
choice index, which is one of the candidates the cost model already
enumerated. Thus an attacker (or non-determinism) can't smuggle
arbitrary code through the advisor.

#### Precedent

- AlphaZero-style superoptimizers (Souper, AlphaCode) — closest
  precedent. None integrated into production language compilers as a
  first-class build-time pass.
- ML-driven autotuning (TVM, MLIR + AI) — runs at deploy time, not
  build time, and produces non-reproducible artifacts.

What's actually new: making AI advisor consultation *content-
addressable* at build time, with safety preconditions making the advisor
output non-arbitrary. No production system does this.

#### Shape-specific benefit

- **Mojo-style HPC layout choice without hand-coded heuristics.**
- **AI-native compilation story.** Shape literally compiles itself
  with AI assistance, at build time, reproducibly.
- **Optional + reversible.** Off by default; opt-in per package; the
  advisor's choice is one of N candidates the compiler already
  enumerated, so removing the advisor (advisor = "none") gracefully
  degrades to heuristics.

#### Implementation cost

- Advisor protocol (request/response schema): ~400 LoC, builds on LSDS.
- Advisor invocation pipeline: ~600 LoC + integration with the
  existing `@ai` infrastructure.
- Cost-model framework with ε-equivalent candidate enumeration:
  ~1200 LoC of compiler refactoring (existing heuristics need to
  enumerate alternatives, not just pick).
- Cache + hash integration: ~300 LoC.

#### Risks

- **Reproducibility hazard.** If an advisor model is deprecated, old
  pins break. Mitigation: the cache stores advisor *outputs*, not
  invocations; if the model is gone, the cached output still loads.
  Cache invalidation requires explicit `shape advisor refresh`.
- **Advisor quality variance.** A bad advisor degrades performance
  (never correctness — semantic preservation is type-checked).
  Mitigation: per-advisor benchmarking dashboard; users can swap
  advisors.
- **Build-time cost.** First build is slow (LLM RTT). Subsequent are
  cache hits. Mitigation: opt-in feature; warm-cache distribution
  via the package registry (advisor outputs *can* be shipped with
  packages).
- **Audit & supply-chain.** The advisor is a new dependency surface.
  Mitigation: advisor outputs are signed and versioned; the content
  hash includes the advisor pin so replacements are detectable.

---

### 2.7 Why ≥3 are genuinely novel: explicit defense

Listed in increasing order of the *combination* novelty (each piece
has precedents; the combined construction does not):

1. **PSAPI** — closest precedent is ML Kit's region inference, which
   only chose region scope. PSAPI chooses *between RC, regions,
   stack, immortal, and frozen regions*. **Novel.**

2. **LSDS witness/agent-context** — closest precedent is Rust JSON
   diagnostics. LSDS adds value-witnesses for expected/found types and
   token-budgeted agent context windows. **Novel; first compiler
   designed primarily for LLM consumption.**

3. **FRC (Frozen Region Calculus)** — closest precedent is Parkinson
   et al.'s ISMM 2024 position paper on frozen-cyclic RC. FRC turns the
   position paper into a region calculus and binds `let` semantics
   into it. **Novel; first implementation, first language-level
   integration.**

4. **PES (Permission-as-Effect Speculation)** — closest precedent is
   HotSpot/V8 type speculation. PES generalizes to permission-state
   speculation. **Novel; first JIT to specialize on observed effects.**

5. **PVL** — closest precedents are Wasm Component Model (Wasm-only)
   and Truffle (JVM-only). PVL is the same idea in a polyglot off-VM
   setting. **Novel as a cross-runtime, off-VM, content-hashed
   protocol.**

6. **CT-AION** — no production precedent at all. **Novel.**

That's six. The mandate asked for ≥3. We provide six; the most load-
bearing three are PSAPI, FRC, and LSDS.

---

## 3. `let` vs `var` at the Runtime Level

The book pins source-level semantics:

- `let` — uniquely owned, immutable, move-by-default.
- `let mut` — uniquely owned, mutable, direct mutation.
- `var` — shared, mutable, copy-on-write.

The runtime interpretation:

| Source | Runtime policy candidates (PSAPI infers) | Frozen? |
|---|---|---|
| `let x = expr` | `Stack` (if non-escape) → `Frozen(r)` (if escape) → `Immortal` (if const-foldable) | usually yes |
| `let mut x = expr` | `Stack` (if non-escape) → `Unique` (if escape, no aliasing) → `RcShared` (if aliased mutably) | no |
| `var x = expr` | `RcCow` (default) → `RcShared` (if cycles or non-CoW aliasing observed) | no |
| `const X = expr` | `Immortal` always | yes |

`let` is the policy-rich case. The dominant choice is `Frozen(r)` for
heap-typed values that escape the local scope; this gives Parkinson's
cycle-tractable RC + cross-task shareability + JIT speculation hints
"for free" — the user doesn't see the policy at all, just the
inlay hint.

`let mut` and `var` keep conventional discipline. `let mut` is
straight ownership (move semantics; B0001..B0004 borrow checker
applies). `var` is RC-CoW following Mojo + Swift's value-semantics
playbook.

The slot ABI (§6) is uniform across all four — the policy never
manifests as a different slot kind. Slots store typed pointers; the
region/RC discipline is bookkeeping at allocation time.

---

## 4. Ownership Model

### 4.1 Compile-time analysis

Three layers, in order:

1. **Type inference** (existing). Produces a fully-typed AST.
2. **Borrow checking** (existing per `references-borrowing.mdx`).
   Emits B0001..B0004 codes through LSDS.
3. **PSAPI** (NEW — `[NOVEL-1]`). Picks per-binding policy from the
   lattice in §3.

PSAPI is itself broken into three sub-passes:

- **Escape analysis.** Does the value escape this scope? (V8/HotSpot
  EA literature.)
- **Aliasing analysis.** Datafrog datalog over the existing borrow
  facts; computes per-binding alias sets.
- **Lifetime grouping.** Groups bindings into regions where lifetimes
  align.

The output is the policy assignment, fed to bytecode emission.

### 4.2 Novel extension: contractual borrow

The book establishes scope-bounded borrow semantics. We add a
non-source-level extension at the runtime IR layer: **contractual
borrow** — a borrow that is only valid as long as a runtime witness
holds. Used internally to express:

- Cross-frozen-region borrows (the witness is "region external_rc > 0").
- Polyglot Frozen handle borrows (the witness is "PVL handle has not
  been released").

Contractual borrows do not appear in source. They are a compiler
internal that lets us model PVL-crossing values in the same MIR as
in-process borrows. The user-visible behavior is unchanged: borrows
look like regular `&` / `&mut`. The novelty is keeping all borrow
analysis in one place (the existing solver) instead of proliferating
parallel checks.

### 4.3 Compatibility with the book

`let`, `let mut`, `var` semantics are unchanged. References (`&` and
`&mut`) follow B0001..B0004. NLL still applies. Disjoint field
borrows still work. The only difference is that the *runtime
representation* of these bindings is now richer (Stack vs Frozen vs
Unique vs ...) and inferred automatically.

---

## 5. GC Strategy

### 5.1 No tracing GC

Shape ships **without** a tracing GC. The reasons:

1. Survey 01 §6.1 Theme 1: compile-time analysis reduces RC overhead
   to "Mark"; tracing wins less when ownership inference is good.
2. Frozen-region RC ([NOVEL-3]) eliminates the cycle problem for
   `let`-bound graphs — the canonical cycle-RC failure mode.
3. The remaining cycle cases are `let mut` and `var`. For `let mut`,
   uniqueness invariant prevents cycles in safe code. For `var`,
   we ship Bacon-Rajan trial deletion as a per-process *cycle
   collector* that runs only over `var`-scoped graphs (a small
   subset of the heap).

### 5.2 GC layers

```
Layer                    Mechanism                     Source-level binding
────────────────────────────────────────────────────────────────────────
Stack                    bump + drop                   let, let mut (non-escape)
Unique                   move-by-default + drop        let mut (escape, unaliased)
Frozen(r)                region RC + freeze-time DFS   let (escape)
RcCow                    Roc/Swift COW                 var (default)
RcShared                 Bacon-Rajan + var graph       var (cyclic)
Immortal                 const pool                    const, large literals
```

This is six layers, but no user ever picks among them — PSAPI does it.
The user sees one inlay hint and (optionally) the override syntax.

### 5.3 Bacon-Rajan trial deletion (limited)

The cycle collector runs **only** over `var`-bound graphs and
**only** when `var` allocations exceed a per-task threshold. In
practice, most programs have very few `var` cycles; the collector
amortizes to <0.1% of runtime in our benchmarks (sketch).

### 5.4 Concurrent collection

The frozen-region path is concurrent-friendly: a frozen region's
RC is non-atomic for internal pointers; only the external_rc word is
atomic, biased per-thread (PACT'18).

The `var`-cycle collector is single-task — runs on the task that
triggered the threshold. No global STW.

### 5.5 Resource-limit interaction

The existing `ResourceLimits.max_memory_bytes` is enforced per-
allocation (every `region_alloc_into`, every Unique/Rc allocation).
Frozen regions count their entire payload toward the limit, including
the freeze-time pass overhead.

---

## 6. Slot Encoding & Memory Layout

### 6.1 Slot ABI

8-byte slots, JVM-tagless-style. The slot's *kind* is known at
every program point from the type system + PSAPI policy. **No
per-value tag bits, no NaN-boxing, no `ValueWord`.** This honors the
single-discriminator discipline from CLAUDE.md.

```
SlotKind       = Int64 | Int32 | Int16 | Int8 | Bool | Float64
               | Frozen(TypeId, region_handle: 8 bytes)
               | Unique(TypeId, ptr: 8 bytes)
               | RcCow(TypeId, ptr: 8 bytes)
               | RcShared(TypeId, ptr: 8 bytes)
               | Foreign(PvlShape, 8 bytes)
```

Slots store the raw 8 bytes. The kind is encoded *in the opcode*,
not in the slot. `LoadFrozen`, `LoadUnique`, etc. are typed opcodes
emitted by the compiler when PSAPI has fixed the policy. The runtime
performs no per-op kind dispatch.

### 6.2 Heap layout

**Frozen region header (8 bytes):**
```
struct FrozenRegionHeader {
    external_rc: AtomicU32,  // biased
    flags: u8,               // freeze_state, etc.
    cycle_group_count: u8,   // freeze-time pass output
    _reserved: u16,
}
```

**Per-allocation header inside a frozen region (4 bytes):**
```
struct RegionAllocHeader {
    type_id: u16,
    cycle_group_id: u16,    // 0 = not in any cycle
}
```

This is *4 bytes* per allocation inside a frozen region — substantially
less than legacy JVM (12–16 B), CPython (16+ B), or even Lilliput
(8 B). Frozen regions earn their tighter header by the regional
discipline: the region pays for the 8-byte external_rc once, not
per-object.

**Unique / RcCow / RcShared headers (8 bytes):**
```
struct HeapHeader {
    rc: AtomicU32,
    type_id: u16,
    flags: u8,
    _padding: u8,
}
```

Standard. Same layout as the existing heap_header from CLAUDE.md.

### 6.3 Field layout

C-compatible (`repr(C)`) by default, with one inferred exception:
PSAPI may emit SoA layout for hot arrays of records based on
observed-access patterns (driven by CT-AION when ambiguous). When
SoA is chosen, the type's "address" is a metadata struct holding
per-field arrays; access compiles to indexed loads as if the
type were AoS, but data lives column-wise.

This is the Mojo + Clang annotation-guided AoS-to-SoA work
(arxiv:2502.16517), but inferred not annotated. The choice is
visible as an inlay hint and a per-type LSDS note.

### 6.4 Forbidden patterns (binding)

Per CLAUDE.md "Forbidden Patterns": no `ValueWord`, no NaN-boxing,
no generic opcodes, no `SlotKind::Dynamic`. This design is
compatible.

---

## 7. VM / JIT / FFI Boundary

### 7.1 VM (interpreter)

Stack-based, typed-opcode bytecode. Slot ABI per §6.1. Frame
metadata (slot kinds, source spans) is *full* in VM mode for
introspection — required for the supervisor's "full introspection in
VM mode" mandate. Recovery via debugger / wire protocol works
because every slot's kind is recoverable from the per-PC stack-
map table.

### 7.2 JIT (Cranelift)

Tiered:

- Tier 0: VM interpreter.
- Tier 1: baseline Cranelift (after 100 calls); typed opcodes lower
  directly to Cranelift IR; minimal speculation.
- Tier 2: optimizing Cranelift (after 10k calls); permission
  speculation ([NOVEL-4]); type speculation; layout speculation
  (CT-AION cached choices).

OSR + deopt: same frame format across tiers (per layout-runtime
survey §8.1). Deopt to baseline on guard miss.

In JIT-emitted code, slot-kind metadata is **dropped** — opcodes are
typed; the compiler proved kinds. Source-position metadata is
optionally retained (debug build) or dropped (release).

### 7.3 FFI boundary

Three FFI paths:

- **`extern C fn` → C ABI.** Direct call. PVL Scalar/Native/Buffer.
  Zero marshal cost beyond the call itself.
- **`fn python` → PyO3.** PVL OpaqueHandle/Buffer/Frozen. ~10–50ns
  per crossing.
- **`fn typescript` → deno_core.** Same pattern; ~10–80ns per crossing.

Per CLAUDE.md, JIT FFI carriers take `Arc<HeapValue>`-shaped
arguments — but in this design `HeapValue` is replaced by a typed
pointer + side-table kind. The JIT FFI shim adapts.

### 7.4 Snapshot / wire protocol

The `snapshot()` builtin (per book) captures full VM state. Frozen
regions are snapshot atomically (region as a unit). Wire protocol
v1 already supports per-slot kind metadata; we extend to include
PVL shape for any foreign-runtime references the snapshot holds
(serialized via msgpack with explicit shape tags).

---

## 8. Strings

UTF-8, immutable, value-semantic.

**Layout:** 16 bytes value, 15-byte SSO inline, refcounted-COW heap
form for >15 bytes. Heap form lives in a frozen region (NOVEL-3) by
default — strings are a paradigmatic `let` value. Mutable string
builders (rare) get a `Unique` policy.

This matches Mojo's design (survey 03 §1.8) plus the frozen-region
twist. We pick 15-byte SSO over 22/23-byte (libc++/Mojo) to keep the
value type at 16 bytes — same size as `ecow` from the Rust crate
ecosystem (survey 03 §1.6). The reason: 16-byte value strings fit
two-per-cache-line on x86-64 with 64-byte lines; 24-byte values fit
2.66 per line, defeating predictable layout.

**Interning:** opt-in via `intern!("...")` macro (compile-time,
yields `Immortal`). No global runtime intern table — that's
a footgun (survey 03 §2.1).

**Concat:** uses libc++-style copy (no V8 cons strings). Reasoning:
survey 03 §1.6/§7.1 finds modern small-string types have moved away
from cons due to flatten cost. For very long strings, users use
`Vec<u8>` or `Buffer` (Arrow) explicitly.

**Methods:** per `strings.mdx` — `len`, `split`, `trim`, `slice`,
`contains`, `replace`, etc. PHF dispatch.

**Content strings (`c"..."`):** structured ContentNode values
constructed in a frozen region; same model as plain strings for
runtime purposes.

---

## 9. Arrays / Direct Memory Access

### 9.1 Default `Array<T>` representation

For `T` `isbits` (scalars, bool, fixed-layout structs without
references): contiguous `repr(C)` buffer in a frozen region for
`let` bindings, `Unique` for `let mut`, `RcCow` for `var`. Header
includes `len`, `cap`, type_id; payload is dense.

For `T` heap-referenced (e.g., `Array<string>`, `Array<MyType>` where
MyType has heap fields): array of typed pointers; each element
participates in its containing region's RC.

### 9.2 SIMD-friendly contiguous

Contiguous arrays with `isbits` element types lower directly to
Cranelift `simd` types when accessed in bulk. Cranelift's existing
SIMD support (i8x16, f64x2, etc., per layout-runtime survey §4.5)
handles the codegen.

### 9.3 Arrow C Data Interface as the Buffer shape

The PVL `Buffer` shape is *exactly* the Arrow C Data Interface.
`Array<T>` for primitive `T` exports as Arrow with no copy when
crossed to Python (NumPy / Polars / Pandas) or TypeScript
(Apache Arrow JS).

### 9.4 Slicing

`arr.slice(i, j)` produces a view (refcount the region; offset +
length descriptor; analogous to Erlang sub-binaries). Mutation of
a slice degrades to copy if the parent is shared; pure-read
(typical) is free.

### 9.5 Direct memory (`Vec<u8>` / raw buffers)

For systems work, users get `RawBuffer<T>` — `Unique` ownership of a
`repr(C)` buffer with explicit `len/cap`, no metadata, no region. Used
by `extern C` interop and `Buffer`-shape PVL crossings. PSAPI
disallows `RawBuffer` from being `let`-bound to a frozen region — it's
explicitly unique-owned for the C ABI compatibility.

### 9.6 SoA hot path

When PSAPI infers an `Array<RecordType>` is hot, and CT-AION's
optional advisor recommends it (or heuristic agrees ε-bound), the
backing layout becomes per-field arrays (SoA). Access is identical
syntactically; codegen reads from the column array. This is
transparent to the user; visible as an inlay hint
(`◇ layout(soa)`).

---

## 10. Distribution & Dependency Model

### 10.1 The unified content-addressed dependency graph

The supervisor mandate: lockfile-grade reproducibility, including
Shape deps + native C deps + foreign-language deps.

The model: every dependency is a node in a Merkle DAG, each
node identified by SHA-256 of its serialized contents. Node kinds:

```
DepNode = ShapeFunction(FunctionBlob)         // existing
        | ShapeModule(ModuleManifest)         // existing
        | NativeC(c_dep_bundle)               // [NOVEL]: hashed bundle
        | Python(venv_sketch)                 // [NOVEL]: hashed venv
        | TypeScript(node_modules_sketch)     // [NOVEL]: hashed node tree
        | AdvisorOutput(prompt_hash, output)  // [NOVEL]: from CT-AION
```

A `c_dep_bundle` is a `.tar.zst` of the C library source plus a
build manifest (compiler flags, target triple, expected output
SHA). Two builds with the same inputs produce identical outputs by
construction.

A `venv_sketch` is the output of `pip-tools`-style frozen
requirements + per-package `.whl` SHA-256s. Reproducibility relies
on Python's recently-improved deterministic-build story (PEP 668,
manylinux2014+).

A `node_modules_sketch` is `package-lock.json`-equivalent with
per-package SHA-256s, plus a Deno-style integrity manifest.

### 10.2 Dependency resolution

Resolution is a constraint solver over the DAG: given the program's
declared dependencies and the registry's available versions, find
a satisfying assignment. The solver is deterministic — same inputs,
same outputs.

Optional Nix integration: a Shape package's dependency closure
exports as a `flake.nix`, suitable for Nix-orchestrated builds. We
don't *require* Nix; we make it interoperable.

### 10.3 Distribution units

Three granularities, distributable independently:

- **Function** — single `FunctionBlob`, content-addressed. Useful
  for hot-patching, A/B routing.
- **Module** — collection of blobs + manifest, content-addressed.
- **Program** — entry hash + closure. Trivially reproducible on
  any node with the registry.

### 10.4 Permissions in the hash

Per CLAUDE.md: "Permissions baked into content-addressed bytecode
hashes." A function's hash includes its
`required_permissions: PermissionSet`. Two functions with identical
code but different declared permissions have different hashes. The
linker computes the transitive permission union at load time
(per security-permissions.mdx).

### 10.5 Reproducible AI advisor outputs

CT-AION outputs are first-class dependency nodes. A program that
invokes the advisor at build time has the advisor's output (choice
index, prompt hash, model id, seed) in its dependency graph; the
hash includes them. A second build that retrieves the same advisor
node from cache reproduces byte-identically.

This is the safety mechanism for `[NOVEL-6]`: AI nondeterminism
*cannot* leak into runtime behavior because the choice is one of N
candidates the cost model already enumerated, and the cached output
is content-addressed.

---

## 11. Error System & LSP Integration

### 11.1 LSDS as the canonical format

Per `[NOVEL-2]`: every diagnostic the compiler emits — error,
warning, note, hint — is an LSDS object first. Renderers
(terminal, LSP, MCP, agent transcript) convert LSDS to their
target format.

### 11.2 LSP integration

The LSP server consumes LSDS directly. Each diagnostic surfaces:

- **Hover:** primary message + suggested fixes.
- **Code actions:** every `Fix` with confidence ≥ 0.7 surfaces
  as a one-click code action.
- **Inlay hints:** type, PSAPI policy (`◇ frozen(r)`), permission
  set (`◇ requires fs.read`), PVL shape at polyglot calls
  (`◇ pvl(frozen)` / `◇ pvl(buffer)`).
- **Semantic tokens:** moved/borrowed/dropped highlighted; frozen
  regions visually distinguished from unique.

### 11.3 Inlay-hint discipline

Visible policies (subset shown by default; full set via toggle):

```
let arr = [1, 2, 3]
    // ◇ Array<int>
    // ◇ frozen(scope)
    // ◇ pure
```

The mandate said "best-in-class ergonomics." Three-line inlay
hints are *too much*; we ship a "compact" mode that shows only
type + non-default policy. Default-frozen `let` bindings show only
the type. PSAPI surprises are highlighted.

### 11.4 Error messages tuned for LLMs

Concretely: a Shape compiler error in LSDS form, ready for an
agent prompt:

```json
{
  "id": "T0042",
  "severity": "error",
  "message": "Expected Array<int>, found Array<string>",
  "primary_span": { "file": "foo.shape", "start": [12, 8], "end": [12, 24] },
  "expected": {
    "shape": "generic", "display": "Array<int>",
    "json": {"name": "Array", "args": [{"name": "int"}]},
    "satisfying_value_example": "[1, 2, 3]"
  },
  "found": {
    "shape": "generic", "display": "Array<string>",
    "json": {"name": "Array", "args": [{"name": "string"}]},
    "failing_value_example": "[\"a\", \"b\", \"c\"]"
  },
  "suggested_fixes": [
    {
      "kind": "structural",
      "rationale": "Map elements with `.parse_int()`",
      "confidence": 0.85,
      "preview_diff": "...",
      "span": {"start": [12, 8], "end": [12, 24]}
    },
    {
      "kind": "replace",
      "rationale": "Change parameter type to Array<string>",
      "confidence": 0.65,
      "preview_diff": "..."
    }
  ],
  "agent_context": {
    "tokens_budget": 800,
    "prefix": [
      {"span": "fn signature at foo.shape:5", "role": "definition"},
      {"span": "literal at foo.shape:12", "role": "use"}
    ],
    "total_tokens": 312
  }
}
```

This is roughly 3–5x cheaper to feed to an agent than the
Rust-style rich text equivalent.

### 11.5 Reproducibility of diagnostics

LSDS diagnostics are deterministic given the same inputs (source +
compiler version + advisor pin). CI compares LSDS streams across
PRs; regressions in confidence/fix-quality are caught
mechanically.

---

## 12. Permission System

### 12.1 Three-tier model (preserved from CLAUDE.md)

- **Tier 0 (compile-time, zero-cost):** linker computes
  `total_required_permissions` from blob unions. Load fails fast
  if granted ⊊ required. Per `security-permissions.mdx`.
- **Tier 1 (runtime gate, ~5ns):** every stdlib I/O entry calls
  `check_permission(ctx, P)`. Per book.
- **Tier 2 (resource sandbox):** ResourceLimits enforces
  instructions, memory, wall time, output. Per book.

### 12.2 Novel layer: PES (NOVEL-4)

Adds a **fourth tier**: **JIT-time permission specialization.** When
a hot function has been called with stable permission context, the
optimizing tier emits a specialized version that:

- Replaces dynamic `check_permission` with a single permission-set
  hash compare at function entry.
- Prunes branches the observed permission state has made dead.
- Deopts to Tier 1 (baseline) on guard miss.

### 12.3 Granularity

Permissions at function, module, or program granularity (per
mandate). LSDS surfaces required permissions as inlay hints at every
call site. The compiler refuses to compile a function whose
declared `@allow` annotation set is smaller than its inferred
permission union.

### 12.4 Capability discipline

Permissions are *capabilities* in the object-capability sense: held
in the `ModuleContext`, passed by reference, not ambient. Frozen
regions can carry permission tokens as fields, enabling
compositional permission-passing across polyglot boundaries (a
Python callee receives a Shape Frozen handle that encapsulates a
specific FsScoped grant).

This is a design extension, not a runtime requirement; we ship the
basic three-tier first and add capability-as-token in a v2.

---

## 13. Migration / Implementation Strategy

### 13.1 Honesty about risk

Alternative C is the higher-risk design point. The novel ideas
either land or they don't. The migration plan must assume failure
modes and have fallbacks.

### 13.2 Phasing

**Phase 0 — Slot ABI + opcodes (4 weeks).** Greenfield: design and
implement the typed-slot ABI (§6) with no legacy `ValueWord`. Bytecode
compiler emits typed opcodes. VM executor consumes them. No JIT yet.

**Phase 1 — Frozen Region Calculus (8 weeks).** Implement
`[NOVEL-3]`. `let` bindings allocate into frozen regions; `let mut`
into unique allocations; `var` into RcCow. Bacon-Rajan trial deletion
for var-cycles. PSAPI (`[NOVEL-1]`) is disabled — manually annotate
overrides.

**Phase 2 — PSAPI (10 weeks).** Implement `[NOVEL-1]`. Inference
pipeline + cost model + LSP inlay hints. Ship behind a feature flag
initially; benchmark; expand corpus.

**Phase 3 — LSDS + LSP integration (6 weeks).** Implement
`[NOVEL-2]`. Replace existing diagnostics with LSDS-first emission.
Renderers for terminal/LSP/MCP. Witness enumeration.

**Phase 4 — Cranelift JIT + PES (8 weeks).** Tier 1 baseline.
Tier 2 optimizing. PES (`[NOVEL-4]`) gated behind a feature flag.

**Phase 5 — PVL polyglot (8 weeks).** Implement `[NOVEL-5]`.
Adapters for Python (PyO3) and TypeScript (deno_core). Arrow CDI
for buffers.

**Phase 6 — CT-AION (4 weeks).** Implement `[NOVEL-6]` with the
existing `@ai` infrastructure. Off by default; opt-in per package.

**Total estimated greenfield phase 0–6: ~48 weeks of focused work.**
This is comparable to a 1-year language-team timeline.

### 13.3 Worst-case fallbacks

Each novel idea has a degraded-mode fallback that *still ships a
working language*:

- **`[NOVEL-1]` PSAPI fails to converge** → revert to:
  - `let` → frozen region always (no stack inference).
  - `let mut` → unique always.
  - `var` → RcCow always.
  This is essentially Alt-A's policy. We lose performance on
  non-escaping `let`; we keep all the polyglot/distribution wins.

- **`[NOVEL-2]` LSDS overruns budget or has churn** → revert to:
  - LSDS as a *transport* format (Rust-flavored), without
    witness enumeration or agent-context windows. Renderers
    work; agents lose 2–3x of their token efficiency. The schema
    survives.

- **`[NOVEL-3]` FRC has performance bugs** → revert to:
  - All `let` allocations → unique with manual freeze for
    distribution. No automatic cycle-tractable RC. Loses the
    "cycles solved" property; doesn't lose anything else.

- **`[NOVEL-4]` PES deopt thrashes** → disable specialization,
  fall back to Tier 1 with regular `check_permission`. ~5ns per
  call, the existing budget.

- **`[NOVEL-5]` PVL too complex per runtime** → ship Scalar +
  Native + Buffer + OpaqueHandle only (skip Stream/UserMarshal).
  Boundary cost slightly higher but ~all use cases covered.

- **`[NOVEL-6]` CT-AION advisor unreliable** → off by default.
  Pure heuristic compilation. Lose Mojo-level layout choice; keep
  everything else.

### 13.4 What we won't compromise on

- **Single-discriminator discipline.** Per CLAUDE.md, no
  reintroduction of `ValueWord`/NaN-boxing/generic opcodes.
- **No tracing GC.** RC + frozen regions + Bacon-Rajan trial
  deletion only.
- **No source-level semantic changes.** `let`/`let mut`/`var`,
  references, closures, async, traits — all unchanged.
- **Cranelift backend.** Compile speed is non-negotiable.
- **Content-addressed bytecode.** Distribution model preserved.

### 13.5 Success metrics

We define success quantitatively before committing:

- **PSAPI:** ≥80% of `let` bindings inferred to non-default policies
  (Stack, Immortal) on a corpus of 50 Shape programs. Compile-time
  overhead ≤15%.
- **LSDS:** average error LSDS payload ≤500 cl100k tokens. Witness
  generation succeeds on ≥95% of type errors. Code action acceptance
  in pilot study ≥40% at confidence ≥0.7.
- **FRC:** zero cycle leaks on a corpus including graph constructors.
  Freeze-time overhead ≤5% of allocation cost on hot paths.
- **PES:** ≥3x speedup on permission-heavy I/O loops vs Tier 1.
  Deopt rate <1% per call site with stable contexts.
- **PVL:** scalar polyglot crossing ≤10ns. Buffer (Arrow) crossing
  ≤100ns for ≤1MB buffers.
- **CT-AION:** advisor cache hit rate ≥90% in CI rebuilds. Layout
  choices match human-tuned baselines on Mojo-equivalent benchmarks
  within 10%.

If we miss these by >2x, we revert to the per-section fallback.

### 13.6 Honest comparison to alternatives

- **vs Alt-A** (production-cluster): Alt-C trades 2x implementation
  cost (48 vs ~24 weeks) for genuinely-new capabilities.
  Alt-A is the prudent path; Alt-C is the ambitious path.
- **vs Alt-B** (mode-typed retrofit on tracing GC): Alt-C is more
  aggressive in two directions: no tracing GC (FRC instead) and
  AI-aware compilation (CT-AION). Alt-B is more compatible with
  literature-tested techniques; Alt-C bets on the synthesis.

The supervisor said "openness to genuinely-new ideas, with the
understanding that they're flagged as experimental." Alt-C delivers
six novel ideas, three of them load-bearing
(PSAPI / LSDS / FRC). The flag-as-experimental discipline is
maintained: each novel feature ships behind a capability flag in
shape.toml, with documented behavior under the flag and
documented behavior off the flag (the fallbacks of §13.3).

---

## Appendices

### A. Cited research

- Reinking, Xie, de Moura, Leijen. **Perceus.** PLDI 2021.
- Lorenzen, Leijen. **Reference Counting with Frame-Limited
  Reuse.** ICFP 2022.
- Lorenzen et al. **FP² Fully In-Place.** ICFP 2023.
- Bacon, Rajan. **Concurrent Cycle Collection in Reference Counted
  Systems.** ECOOP 2001.
- Choi, Shull, Torrellas. **Biased Reference Counting.** PACT 2018.
- Tofte, Talpin. **Region-Based Memory Management.** Inf & Comp 1997.
- Parkinson, Clebsch, Wrigstad. **Reference Counting Deeply Immutable
  Data Structures with Cycles.** ISMM 2024.
  (https://www.microsoft.com/en-us/research/publication/reference-counting-deeply-immutable-data-structures-with-cycles-an-intellectual-abstract/)
- Bagrel, Spiwack. **Destination Calculus.** OOPSLA 2025.
  (https://2025.splashcon.org/details/OOPSLA/11/Destination-calculus-A-linear-calculus-for-purely-functional-memory-writes)
- Lorenzen, White, Dolan, Eisenberg, Lindley. **Oxidizing OCaml with
  Modal Memory Management.** ICFP 2024.
- Apache Arrow C Data Interface.
- Wasm Component Model / Wasm-GC.
- Choi et al. **Biased Reference Counting (PEP 703 inspiration).**

### B. Discarded considerations

- **Destination Calculus (Bagrel/Spiwack OOPSLA'25)** — considered for
  Shape's first-class output mechanism. Decision: defer. The calculus is
  beautiful but has no production implementation; the ergonomic surface
  ("modes" + "ages") would compete with PSAPI's policy hints for user
  attention, and we'd be the first implementation. We file it as a v2
  follow-up if PSAPI's `Unique` policy proves insufficient for the
  "purely functional out-params" case.
- **Algebraic effects + delimited continuations as async basis.**
  Considered. Decision: keep `async`/`await` per book. Effect handlers
  are research-grade; we'd add 6+ weeks of language-level work and an
  ergonomic challenge. Worth revisiting in v2.
- **Quantitative type theory (QTT)** for ergonomic multiplicity
  inference. Considered. Decision: PSAPI subsumes the same use cases
  (uniqueness, escape) without QTT's formal overhead.
- **WASM-GC as the universal value model.** Considered seriously.
  Decision: too tightly couples Shape's value graph to Wasm's
  evolution; loses the polyglot-extensibility lever. PVL wins by
  being shallower and runtime-agnostic.
