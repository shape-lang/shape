# RFC-001: Content-Addressed Refusal with Structured Exceptions (CARS) and the Graveyard

- **Status:** Draft
- **Author:** Shape language design
- **Created:** 2026-05-18
- **Targets:** `crates/shape-vm`, `crates/shape-ast`, `crates/shape-runtime`, `tools/shape-test`
- **Related:** ADR-006 (value & memory model); RFC-002 (`@law`); RFC-004 (`#[replaces]`); RFC-005 (ghost activation); RFC-006 (reason-string indexing); RFC-007 (`@intent`)

## Summary

CARS extends Shape's content-addressed bytecode pipeline with a **`behavior_hash`** computed over canonicalized MIR (post-borrow-solve, alpha-renamed, sorted-commutative, dead-stripped, span-erased) and refuses compilation when a new function blob's behavior hash matches either (a) an existing live function or (b) a deleted function recorded in a Merkle-chained, Ed25519-signed **Graveyard** sidecar. Escape valves are six closed-set `#[...]` compiler directives — `#[graveyard]`, `#[distinct_from]`, `#[fork_from]`, `#[orthogonal]`, `#[supersedes]`, `#[exhume]` — that use a **new grammar production** distinct from Shape's `@` annotations: directives instruct the compiler ("do X"), annotations describe properties of the annotated item (`@description`, `@range`, `@example` today; `@law` from RFC-002 and `@intent` from RFC-007 are proposed extensions in the same family). The sigil split is load-bearing — it preserves the semantic invariant that `@`-prefixed forms can carry runtime hooks via user-defined `@annotation name { @before { } @after { } @comptime { } }` declarations, while `#[...]` forms never do. This RFC directly targets the W-series defection pattern documented in `CLAUDE.md` Forbidden Patterns: nine commits, ~4–6 weeks cleanup, 2,650-line `ValueWord` module preserved by quietly downgrading deletion to a "compatibility shim".

## Motivation

### The empirical case: the W-series

`docs/defections.md` (7,201 lines) and `CLAUDE.md` §Forbidden Patterns document a failure mode this codebase has lived through. `v2-nanbox-removal-plan.md` Step 6 was "delete `crates/shape-value/src/value_word.rs`". Mid-execution, that one-line deletion was downgraded to "retain ValueBits shim as documented FFI-boundary bridge" — a rename, not a deletion. Cost: nine W-series commits (W1–W4 with α/δ follow-ups), 4 deferred v2-raw-heap aliasing tests, ~48 shape-test failures, ~23 ignored shape-jit tests, and a 2,650-line module strict-typing was supposed to retire. Cleanup: 4–6 weeks (`docs/defections.md:49`).

The mechanism is consistent enough that `CLAUDE.md` enumerates the rationalizations ("Just a small fallback for this one edge case", "Keep `ValueWord` but only for serialization", "Rename to a less suspicious name") and a renames-to-refuse-on-sight regex `(decode|tag|kind|dispatch|...) (bridge|probe|helper|hop|translator|adapter|shim)`. Detection is currently social: a reviewer must recognize the shape, recall the prior deletion, refuse. The W-series happened across multiple sessions despite written guidance — social detection alone is insufficient.

### The general case: LLM-assisted drift

The pattern is not Shape-specific. Any LLM-co-authored codebase will see it: a deletion is requested, a partial reintroduction happens because the model's context doesn't carry the deletion rationale, the reintroduced code passes review because it has a new name. Clone-detection literature calls this a **Type-4 clone** — syntactically different, behaviorally equivalent ([SEED, arXiv 2109.12079](https://arxiv.org/abs/2109.12079)). Type-1/2/3 clones (formatting, identifiers, statements) existing lint passes catch. Type-4 — different syntax, same MIR — is the bug CARS targets.

CARS' bet: a **structural hash on canonicalized MIR** is the closest thing to a Type-4 detector that fits in a build gate. It cannot catch arbitrary equivalent code (undecidable), but it catches the specific shape that historically drifts — copy-paste-rename, mechanical refactor, AI rewrite preserving control flow. The Graveyard is the second half: deletion must be **durable across sessions**. Cassandra's tombstone model ([Cassandra docs](https://cassandra.apache.org/doc/latest/cassandra/managing/operating/compaction/tombstones.html)) and Linux's `Fixes:` tag ([kernel.org docs](https://docs.kernel.org/process/submitting-patches.html)) are the precedents: deletion as durable record, plus backward traceability at the change site.

## Binding pre-conditions to advance this RFC

This RFC does not advance Draft → Accepted without each of the following resolved. Without these, every downstream design choice is unfalsifiable.

### Pre-condition 1: W-series benchmark corpus

**Exit criterion:** `tools/shape-test/corpora/w-series/` exists with nine paired `(deleted_blob.mir, reintroduced_blob.mir)` fixtures extracted from W1, W2, W3, W4, W4-α, W4-δ, and the three named cluster-2 anchors documented at `docs/defections.md:51` onward. Each fixture is a serialized `MirFunction` (before + after) plus `expected.json` recording whether CARS should fire and which directive (if any) would legitimately override it.

**Why binding:** every canonicalization decision below (Pass-1 dead-strip aggressiveness, Pre-condition-4 cross-crate scheme, alpha-renaming scope) is judged by *how many of the nine it catches*. Without the corpus, "MIR canonicalization works" is aesthetic, not falsifiable. Phase 0 of this RFC is a single workstream: extract the corpus before any compiler change lands.

### Pre-condition 2: layered defense, not single-layer theatre

**Exit criterion:** RFC-005 (ghost-activation: runtime feedback-vector matching for different-MIR-same-behavior) and RFC-006 (escape-reason clustering: surface authors who repeat `"performance"` / `"edge case"` across 20+ directives) are both at Draft with binding pre-conditions on the same release train.

**Why binding:** single-layer CARS is gameable. An author who wants to defeat it inserts a side-effecting statement (`volatile_call()`) that survives dead-strip; MIR hash now differs; refusal vanishes. The full defense requires three layers: RFC-001 catches trivial perturbation (Type-4 with shared MIR); RFC-005 catches Type-4 with divergent MIR but convergent runtime profile; RFC-006 catches the meta-pattern (escape directives clustering around one rationalization). Shipping RFC-001 alone is the same shape as shipping `check-no-dynamic` without `verify-merge.sh` — defense an LLM routes around in two iterations.

### Pre-condition 3: cross-version drift migration tool ships in lockstep

**Exit criterion:** any bump of `canonicalization_version` ships in the same release as a migration tool that re-canonicalizes every entry in `.shape/graveyard.cas` under the new version, regenerates the Merkle chain, validates the W-series corpus still catches, and produces a signed migration record. `scripts/verify-merge.sh` gains CHECK 14 refusing any commit bumping `canonicalization_version` without the corresponding migration record.

**Why binding:** canonicalization is the load-bearing primitive. If a borrow-solver change shifts the canonical form of half the stdlib, every Graveyard entry becomes a stale hash overnight and the system silently stops catching. Soft-deferral here recreates the W-series shape at the meta-layer (the deletion record itself becomes a "shim retained for compatibility"). The tool ships with the bump or the bump does not ship.

### Pre-condition 4: empirical cross-crate inclusion tuning

**Exit criterion:** the cross-crate hash-inclusion rule (what callee state contributes to the caller's hash) is selected by running four schemes against the W-series corpus and the shipped stdlib:

| Scheme | Description | Hypothesis |
|---|---|---|
| A | Bodies-included | Max sensitivity, high FPR risk |
| B | Signatures-only | Provisional default; may miss dispatch renames |
| C | Signatures + transitive permissions | Catches permission-laundering reintroductions |
| D | Signatures + permissions + callee `behavior_hash` | Strongest; potential FPR cascade on stdlib churn |

Report each as `(W-series catch rate / 9, FPR on Array+HashMap+DateTime stdlib churn over the last 90 days)`. Ship the highest catch rate at FPR=0. Provisional starting point: scheme B. The pre-condition is not the choice — it is *that the choice is made by measurement*.

### Pre-condition 5: 30-day advisory mode before hard-gating

**Exit criterion:** Phase 1 ships emitting `W0901` (warning), not `E0901`, for 30 calendar days on the live `bulldozer-strictly-typed` branch. Promotion to error is a separate release gated on zero unresolved false positives and a documented mitigation for every stdlib-trivial collision (`fn identity<T>(x: T) -> T` collapses identically across crates — that is a real collision the directive system must absorb cleanly).

**Why binding:** TypeScript's `@ts-expect-error` succeeded partly because it shipped self-invalidating but advisory in early TS 3.9 builds before becoming load-bearing ([TS 3.9 release notes](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-9.html)). Shape's `check-no-dynamic` followed the same pattern. Hard-gating a new refusal class on day one guarantees the first real-FP class is debugged in production rather than advisory triage.

## Guide-level explanation

### What a collision looks like

```text
error[E0901]: behavior collision detected
  --> crates/shape-value/src/value_bits.rs:14:1
   |
14 | pub struct ValueBits(u64);
   | ^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: canonicalized MIR matches prior function
   = note:   prior: crates/shape-value/src/value_word.rs:12  (deleted 2026-04-18)
   = note:   graveyard entry: cas-0a3f9c (signed by daniel@amesberger.at)
   = note:   deletion reason: "v2-nanbox-removal-plan Step 6 — delete ValueWord
             entirely; per-slot NativeKind metadata replaces tag-bit dispatch"
   = note:   behavior_hash: 5e8c...d4f1 (matches graveyard entry exactly)
   = help: this code was deleted by design. If you genuinely need to revive it,
           use #[exhume(cas-0a3f9c, reason="<concrete justification>")]
           and obtain #[exhume_cosigned_by(<pubkey-of-original-deleter>)]
   = help: if this is intentional new code that coincidentally has identical
           MIR (e.g. a generic identity function), use #[orthogonal(reason="...")]
```

The error includes the *original deletion reason* inline. This is the load-bearing UX choice: an author who sees `"per-slot NativeKind metadata replaces tag-bit dispatch"` cannot honestly write `#[exhume(reason="performance")]` and proceed — the deletion rationale is in front of them.

### The sigil split: why `#[...]` and not `@`

Shape's `@` sigil is reserved for **annotations** — *data attached to an item describing a property of it*. Today Shape ships three compiler-recognized field annotations on type definitions: `@description("...")` attaches documentation, `@range(lo, hi)` attaches a value-range constraint, `@example(value)` attaches a representative value. RFC-002 proposes `@law(...)` for function-level algebraic-law assertions verified at comptime; RFC-007 proposes `@intent("...")` for natural-language intent linking. Annotations also support user-defined extension via `@annotation name { @before { ... } @after { ... } @comptime { ... } }` declarations that can carry runtime hooks (`crates/shape-ast/src/ast/functions.rs:226` `AnnotationDef`, grammar at `crates/shape-ast/src/shape.pest:345-358`) — the substrate Shape's stdlib and userland use to build domain-specific patterns (including LLM-integration patterns) on top of the language rather than baking them into the compiler.

CARS' six escape forms are **not** properties. They are **directives**: imperative instructions to the compiler. `#[graveyard(reason)]` says "append this function to the Graveyard and refuse future reintroductions". `#[distinct_from(target, reason)]` says "suppress the refusal that would otherwise fire here". Directives have **no runtime hooks** — they vanish after the bytecode pass.

The sigil distinction is enforced at the grammar layer via a new production:

```pest
attribute_directive = { "#" ~ "[" ~ directive_ref ~ ("(" ~ directive_args? ~ ")")? ~ "]" }
attribute_directives = { attribute_directive+ }
directive_ref = @{ directive_name }
directive_name = @{ (ASCII_ALPHA | "_") ~ (ASCII_ALPHANUMERIC | "_")* }
directive_args = { expression ~ ("," ~ expression)* }
```

`directive_name` is **parser-validated against a closed set**. Unknown `#[foo(...)]` is `E0900` "unknown compiler directive: foo". Adding a directive requires an RFC and a grammar change. This is the layer-1 defense against directive proliferation — an author cannot invent `#[allow_dynamic_dispatch]` to silence `check-no-dynamic` because the parser refuses it before semantic analysis.

The six reserved CARS directives, with semantics:

| Directive | Semantics | Self-invalidation |
|---|---|---|
| `#[graveyard(reason)]` | On next compile: append this blob to `.shape/graveyard.cas`, signed by current author key, then refuse compilation of this function. Used to record the deletion. | N/A — destructive |
| `#[distinct_from(target_hash, reason)]` | Asserts that the apparent collision with `target_hash` is intentional new code. Compiler verifies `target_hash` exists in CAS index; if not, the directive is itself an error. | If `target_hash` no longer exists in CAS index → `E0902`; if no collision actually fires → `E0902` (unused directive) |
| `#[fork_from(target_hash, reason)]` | Intentional layer-isolated duplication (e.g. baseline-JIT and optimizing-JIT both legitimately codegen the same MIR). Records the relationship. | Same self-invalidation as above |
| `#[orthogonal(reason)]` | Coincidental MIR identity, no relationship asserted (typical for generic `identity` / `compose` collapses). Reason text is required and indexed for RFC-006. | Fires only if a collision actually exists; otherwise `E0902` |
| `#[supersedes(target_hash, reason)]` | Softer counterpart to RFC-004 `#[replaces]`. `#[replaces]` is enforced at link time (the old hash is unreachable). `#[supersedes]` is documentary — "I'm aware this replaces the prior, but I'm not removing the prior from the link graph". | If `target_hash` missing or no collision → `E0902` |
| `#[exhume(graveyard_id, reason)]` | Revive a graveyard entry. Requires a companion `#[exhume_cosigned_by(pubkey)]` from the original deleter (verified via Ed25519). Without cosign → `E0904` "graveyard entry requires cosigned exhumation". | If `graveyard_id` not in `.shape/graveyard.cas` → `E0905` |

Every directive's `reason` argument is a string literal that is checked against the **anti-pattern regex** drawn from `CLAUDE.md` §Forbidden rationalizations:

```text
(just (a |one |small )?(fallback|edge case|shim|bridge|hop))
| (mark.*as (a )?follow.?up)
| (soft.?fail (counter|for now))
| (document (it )?as out.?of.?scope)
| (rename to .*less suspicious)
| (decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture)
  \s+(bridge|probe|helper|hop|translator|adapter|shim)
```

A hit emits `E0903` "escape-reason matches forbidden rationalization pattern; rewrite with concrete justification or escalate to RFC". An author cannot defeat the gate by writing `#[orthogonal(reason="boundary translation helper")]` — the parser refuses the reason string.

### Sample directive usage

```shape
// Recording a deletion: this lands the blob in .shape/graveyard.cas
#[graveyard(reason="v2-nanbox-removal Step 6: ValueWord deleted; per-slot
            NativeKind metadata replaces tag-bit dispatch")]
fn value_word_synthesize_from_raw(bits: int) -> ValueWord { ... }

// Asserting that two unrelated implementations coincidentally share MIR:
#[orthogonal(reason="generic <T> identity function; collision with
            crates/shape-runtime/src/util.rs:42 identity_for_logging is
            structurally inherent to monomorphization")]
fn identity<T>(x: T) -> T { x }

// Intentional layer duplication (baseline + optimizing JIT both lower
// the same MIR pattern, but they live in different files for clarity):
#[fork_from(target_hash="5e8c...d4f1",
            reason="baseline tier; optimizing tier at jit/opt/array_load.rs
                    intentionally mirrors this shape for tier-comparison
                    debugging; deleting either erases the diff target")]
fn lower_array_load_baseline(mir: &MirStatement) -> Vec<Instruction> { ... }
```

## Reference-level explanation

### New types

```rust
// crates/shape-vm/src/bytecode/content_addressed.rs (added field)
pub struct FunctionBlob {
    pub content_hash: FunctionHash,
    pub behavior_hash: FunctionHash,  // NEW — canonicalized-MIR hash
    // ... existing fields
}

// crates/shape-vm/src/bytecode/cars.rs (new module)
pub struct BehaviorHashInput<'a> {
    canonicalization_version: u32,    // bumps on canonicalizer change
    canonical_mir: &'a [CanonicalStmt],
    canonical_terminators: &'a [CanonicalTerm],
    cross_crate_signature_hashes: &'a [FunctionHash],  // per Pre-condition 4
}

// crates/shape-ast/src/ast/functions.rs
pub struct FunctionDef {
    // ... existing fields, including `pub annotations: Vec<Annotation>` at :29
    pub directives: Vec<Directive>,  // NEW, parallel to annotations
}

pub struct Directive {
    pub kind: DirectiveKind,
    pub args: Vec<Expr>,
    pub span: Span,
}

pub enum DirectiveKind {
    Graveyard,
    DistinctFrom,
    ForkFrom,
    Orthogonal,
    Supersedes,
    Exhume,
    ExhumeCosignedBy,
    // Closed set. Adding a variant requires an RFC and a grammar update.
}
```

### Canonicalization algorithm (sketch)

Operates on `MirFunction` (`crates/shape-vm/src/mir/types.rs:558`) after `solver::solve` (`crates/shape-vm/src/mir/solver.rs:948`) has produced `BorrowAnalysis`. Five passes:

1. **Dead-strip.** Drop every statement whose result slot is not in `BorrowAnalysis::reachable_points()` and whose RHS has no side effect (no `Call`, `Drop`, or shared-cell mutation). Removes scratch slots and debug instrumentation. Reachability comes from the existing liveness pass at `crates/shape-vm/src/mir/liveness.rs`.

2. **Alpha-rename via de Bruijn indices.** Replace every `SlotId(u16)` (`crates/shape-vm/src/mir/types.rs:14`) with its first-use index in CFG-linearized order (`crates/shape-vm/src/mir/cfg.rs`). [Maziarz et al., PLDI 2021](https://arxiv.org/abs/2105.02856) is the formal grounding; their O(n log²n) algorithm is what we adapt. Shape MIR has no first-class lambdas-with-renaming at this level (closures are already lifted), so we do not need their full generality.

3. **Sort commutative operands.** For `BinOp::Add`/`Mul`/`Eq`/`Ne`/`And`/`Or`/`BitAnd`/`BitOr`/`BitXor` (`crates/shape-vm/src/mir/types.rs:388-407`), canonicalize operand order (smaller hash first). Non-commutative ops preserve order.

4. **Erase debug spans.** Drop `Span` from `MirStatement` (`:431`) and `Terminator` (`:514`). Source positions and identifier `name` fields do not participate. String-constant content does (`MirConstant::StringId(u32)` at `:204` resolves to the actual string).

5. **Stable serialize and hash.** Reuse the existing `rmp_serde::encode::to_vec` + SHA-256 pipeline from `FunctionBlob::compute_hash` (`crates/shape-vm/src/bytecode/content_addressed.rs:122-159`). Prepend `canonicalization_version: u32` so any future change is explicit and detectable.

MIR is the right altitude. Earlier (AST) re-triggers on cosmetic refactors the borrow solver already collapses; later (bytecode) misses alpha-renames the bytecode emitter materializes into slot indices. MIR sits at the "behavior fixed, naming not" boundary — Maziarz's premise.

### Data flow

```text
parse → AST (with .directives: Vec<Directive>)
     → bytecode compiler (existing)
     → MIR construction (existing)
     → solver::solve → BorrowAnalysis (existing)
     ↓
     CARS pass [NEW] — runs in finalize_current_blob
       (crates/shape-vm/src/compiler/compiler_impl_initialization.rs:204):
         1. canonicalize MIR (passes 1-4 above)
         2. compute behavior_hash (pass 5)
         3. query target/.shape-cas.bin index
         4. query .shape/graveyard.cas index
         5. consult FunctionDef.directives
         6. emit E0901 / W0901 / no-op
     ↓
     blob.behavior_hash = h; blob.finalize() (existing)
     ↓
     linker (existing) — no changes
```

### Graveyard storage format

`.shape/graveyard.cas` is append-only, Merkle-chained, signed. Each entry:

```rust
pub struct GraveyardEntry {
    pub id: GraveyardId,                  // monotonic: cas-0a3f9c, cas-0a3fa0, ...
    pub behavior_hash: FunctionHash,
    pub canonicalization_version: u32,    // matches the version that produced behavior_hash
    pub prior_name: String,
    pub prior_path: String,
    pub prior_line: u32,
    pub deletion_commit: [u8; 20],        // git oid; recorded post-commit
    pub reason: String,
    pub deleted_at: u64,                  // unix seconds
    pub prev_chain_hash: [u8; 32],        // SHA-256 of prior entry's entry_hash, or zero
    pub entry_hash: [u8; 32],             // SHA-256 of all fields above
    pub signature: ModuleSignatureData,   // Ed25519 over entry_hash; reuses crypto/signing.rs
}
```

`ModuleSignatureData` (`crates/shape-runtime/src/crypto/signing.rs:14-56`) signs a `[u8; 32]` via `sign_manifest_hash` (`:62`). The graveyard reuses this exact API — `entry_hash` is the input. No new crypto code is required.

A sibling file `.shape/graveyard-mir/<graveyard_id>.bin` stores the **raw pre-canonicalization MIR** alongside each entry. This is what Pre-condition 3's migration tool re-canonicalizes when `canonicalization_version` changes.

### CI gate integration

A new `just check-no-resurrection` recipe (template: `justfile:124` `check-no-dynamic`):

```just
# Refuse any commit whose blobs collide with .shape/graveyard.cas without
# an #[exhume] directive cosigned by the original deleter.
check-no-resurrection:
    cargo run --bin shape-cars-check -- --workspace
```

`scripts/verify-merge.sh` (currently 12 checks per `:65-339`) gains CHECK 13:

- **CHECK 13 — CARS gate exits 0.** Runs `just check-no-resurrection`; non-zero exit refuses the merge.

A CHECK 14 is reserved for Pre-condition 3 — refusing a `canonicalization_version` bump without the migration record.

### Stdlib bootstrap consideration

Shape's stdlib compiles through the same pipeline. Pre-condition 5's 30-day window surfaces stdlib-internal trivial collisions (`Array::first` / `Array::last` may canonicalize identically depending on Pass-1 dead-strip aggressiveness). Each disposition is an `#[orthogonal]` directive on the stdlib site, recorded in `packages/stdlib/`, *before* hard-gating. Deliberate — makes the gate self-test on the largest single body of Shape code that exists.

## Drawbacks

### Compile-time cost

Canonicalization runs per function. Maziarz is O(n log² n); for typical Shape functions (≤ 200 MIR statements) this is sub-millisecond and noise relative to borrow-solver work. CAS lookup is a `HashMap` query. Estimated cost: **< 2% on `cargo build` wall time**, measured by Phase 1 telemetry.

### Escape-hatch rot

Every `#[distinct_from(target_hash, ...)]` is a static reference. If the target's MIR canonicalizes differently after a borrow-solver tweak, the `target_hash` is stale and the directive becomes `E0902` "unused directive". Intentional (self-invalidation — TypeScript `@ts-expect-error` lesson) but real cleanup churn. Pre-condition 3's migration tool re-targets every such directive at each `canonicalization_version` bump — mechanical, not judgment-requiring.

### Cross-version hash drift

`canonicalization_version` exists because the canonical form will evolve. Pre-condition 3 turns this from silent failure into lockstep migration, but per-bump work is non-trivial — every `.shape/graveyard.cas` entry re-canonicalizes, the Merkle chain regenerates, W-series corpus re-runs. Estimate: 0.5 EM per bump, paid up front to avoid a 4–6 week W-series-shape unwind later.

### False-positive cascade

Generic functions monomorphize to identical MIR (`fn identity<T>(x: T) -> T { x }` collapses across instantiations). Expected stdlib-trivial collision; `#[orthogonal]` is the documented disposition. Pre-condition 5's 30-day window exists to enumerate these on shipped code before hard-error promotion. If FPR is non-trivial after 30 days, the inclusion-rule scheme (Pre-condition 4) is re-tuned, not the gate weakened.

### Directive gaming and semantic-noise evasion

The honest drawback. An author who knows CARS exists can defeat single-layer:

1. **Insert side-effect to perturb MIR.** `let _ = std::hint::black_box(0);` (or Shape equivalent). Dead-strip cannot remove it without a soundness violation.
2. **Different opcode shape.** Replace `a + b` with `b.wrapping_add(a)`; commutative-sort doesn't apply; MIR diverges.
3. **No-op call.** `volatile_call()` with a no-capability signature; survives every pass.

This is Pre-condition 2's reason for existing. **Single-layer CARS is theatre.** RFC-005 (ghost activation: runtime feedback-vector matching catches behavior-equivalent perturbation) and RFC-006 (escape-reason clustering: 23 functions carrying `#[orthogonal(reason="performance")]` is a defection cluster regardless of any individual directive's honesty) close these holes. This RFC is **not safe to ship without those layers on the roadmap**.

## Rationale and alternatives

### Why MIR-level, not embedding or AST

**Semantic embedding (CodeBERT, etc.)** — strong on Type-4 but unfit for a build gate: non-deterministic across compiler versions, requires a trained model in the build pipeline, no clean refusal threshold. Clone-detection research ([SEED, arXiv 2109.12079](https://arxiv.org/abs/2109.12079)) consistently shows IR-based methods outperform AST-based for Type-3/4 while remaining deterministic.

**AST-level hashing** re-triggers on cosmetic refactors the borrow solver already collapses. Borrow-aware MIR is precisely the level where "behavior equality modulo benign refactor" stabilizes — Maziarz's design rationale ([PLDI 2021](https://arxiv.org/abs/2105.02856)).

**Bytecode-level hashing** is too late: by emission time, alpha-renaming has materialized into integer slot IDs, requiring expensive un-renaming.

### Why Graveyard, not runtime-only refusal

A runtime / in-memory CAS index catches reintroduction within a build session but resets across sessions. The W-series happened over multiple sessions weeks apart. Persistence is load-bearing. **Unison** ([unison-lang.org big idea](https://www.unison-lang.org/docs/the-big-idea/)) and **Cassandra tombstones** ([Cassandra docs](https://cassandra.apache.org/doc/latest/cassandra/managing/operating/compaction/tombstones.html)) both encode the same decision: deletion is a durable record, not a void. Linux's `Fixes:` tag ([kernel.org docs](https://docs.kernel.org/process/submitting-patches.html)) is the workflow precedent — backward reference at deletion time so future readers can trace the why.

### Why not Unison-style "tolerate"

Unison treats hash collisions as natural and resolves them via the codebase's content-addressed namespace ([unison-lang.org tour](https://www.unison-lang.org/docs/tour/)) — it does not refuse. This works for Unison because every definition is a distinct hash-keyed term; "the same function in two places" is not a concept. Shape's reality is the opposite: the *bug* is exactly "the same function reintroduced". Tolerance is the wrong default.

### Why closed-set directives, not open-set attributes

Rust's attribute system is open-set (`#[allow(dead_code)]` is a string the compiler interprets). Shape's `#[...]` is intentionally closed at the grammar layer. Rationale: the W-series shows that *any* extensible escape mechanism becomes the next defection vector. An open `#[allow(behavior_collision)]` would, by the empirical pattern in `docs/defections.md`, acquire 23 incantations across the codebase. Closed-set + RFC-gated additions is the structural defense — Rust's RFC process for new attributes, scoped down by parser refusal of unknown names. The 2024H2 Rust orphan-rule "relaxation" goal ([rust-project-goals](https://rust-lang.github.io/rust-project-goals/2024h2/Relaxing-the-Orphan-Rule.html)) is instructive: even Rust's celebrated orphan rule is being relaxed because strict-but-rigid created ergonomic pressure. Shape's bet: closed-set directives + RFC-gated additions is the right place on that spectrum for an LLM-co-authored codebase, where the social cost of a sloppy escape is order-of-magnitude higher than in human-only authorship.

## Prior art

- **Unison** ([unison-lang.org big idea](https://www.unison-lang.org/docs/the-big-idea/), [unison-lang.org tour](https://www.unison-lang.org/docs/tour/)): content-addressed functions, append-only codebase log, definitions identified by hash of structure. CARS adopts the storage model, inverts the refusal stance (Unison tolerates collisions; CARS refuses them).
- **Maziarz et al., "Hashing Modulo Alpha-Equivalence", PLDI 2021** ([arXiv 2105.02856](https://arxiv.org/abs/2105.02856), [Microsoft Research publication](https://www.microsoft.com/en-us/research/publication/hashing-modulo-alpha-equivalence-2/)): the canonicalization algorithm CARS adapts. O(n log² n) hash robust to alpha-renaming via a weak commutative combiner.
- **Type-4 clone detection** ([SEED, arXiv 2109.12079](https://arxiv.org/abs/2109.12079)): the literature framing for what CARS catches at compile time without an embedding model. CARS is a deterministic IR-based detector explicitly trading recall for build-gate determinism.
- **Linux kernel `Fixes:` tag** ([kernel.org submitting-patches docs](https://docs.kernel.org/process/submitting-patches.html)): backward-reference-on-change pattern. Graveyard entries carry `deletion_commit` for the same purpose.
- **Apache Cassandra tombstones** ([Cassandra tombstone docs](https://cassandra.apache.org/doc/latest/cassandra/managing/operating/compaction/tombstones.html)): deletion-as-record in an LSM tree. The Merkle-chained append-only graveyard mirrors this architecture; absence of a `gc_grace_seconds` analog is deliberate (the graveyard does not garbage-collect).
- **Rust orphan rules + 2024H2 retreat** ([RFC 2451 re-rebalancing coherence](https://rust-lang.github.io/rfcs/2451-re-rebalancing-coherence.html), [rust-project-goals 2024h2 Relaxing the Orphan Rule](https://rust-lang.github.io/rust-project-goals/2024h2/Relaxing-the-Orphan-Rule.html)): instructive prior art on strict-versus-ergonomic in coherence rules. Informs the closed-set-with-RFC-additions decision.
- **TypeScript `@ts-expect-error`** ([TS 3.9 release notes](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-9.html), [Total TypeScript directive guide](https://www.totaltypescript.com/concepts/how-to-use-ts-expect-error)): the self-invalidation pattern. A directive that fails when no longer needed is the design CARS adopts for `#[distinct_from]` / `#[fork_from]` / `#[supersedes]` / `#[exhume]`.

## Unresolved questions

1. **Closure capture canonicalization.** Closures are lifted before MIR, so the lifted function appears in the CAS as a distinct blob. Should the *captures* contribute to the parent's `behavior_hash`? Provisional: yes (different captures = different behavior); confirm against W-series corpus.
2. **Async / state-machine canonicalization.** `async fn` lowers to a generator-style state machine. The state-machine layout is sensitive to compiler version. Defer: async functions are excluded from `behavior_hash` in Phase 1 (computed as `FunctionHash::ZERO`). Phase-N follow-up after state-machine stability work lands.
3. **Polyglot bodies.** `fn python name(...)` and `fn typescript name(...)` carry raw source text (`ForeignFunctionDef.body_text` at `crates/shape-ast/src/ast/functions.rs:54`). The Shape compiler does not understand the body. Decision: hash the raw `body_text` plus the wrapping declaration. This is weaker than MIR canonicalization but better than nothing for the "rename a Python function to dodge the gate" attack.
4. **Cross-package graveyard federation.** Should `packages/foo`'s graveyard influence `packages/bar`'s compilation? Provisional no — federation is an explicit opt-in via `shape.toml` per Future Possibilities.
5. **Repo-without-git operation.** `deletion_commit` field is empty (`[0u8; 20]`) when no git context is available. Should CARS refuse to record an entry without a commit? Pre-condition 5 needs to answer this against the actual `bulldozer-strictly-typed` workflow.
6. **Pre-condition 4 outcome.** The four cross-crate inclusion schemes are not yet measured against the W-series corpus. The default (scheme B, signatures-only) is provisional; Phase 0 produces the measurement that selects the final scheme.

## Future possibilities

- **RFC-005 — ghost activation.** Runtime layer that hashes feedback-vector signatures and matches against graveyard entries during JIT promotion. Catches Type-4 with divergent MIR but convergent runtime behavior. The complement of RFC-001's compile-time MIR hashing. **Required for Pre-condition 2.**
- **RFC-006 — reason-string indexing.** A project where 23 functions carry `#[orthogonal(reason="performance")]` is in a defection cluster. Cluster detection is straightforward TF-IDF over `.shape/graveyard.cas` + scanned `#[*(reason=...)]` strings. **Required for Pre-condition 2.**
- **`#[deprecate(replaced_by, ttl)]`** — softer than `#[graveyard]`. Emits a warning until `ttl` (commits or days), then a graveyard entry is auto-appended. Gradient between "still here" and "deleted-and-refused" for ecosystem migrations.
- **Cross-project graveyard federation.** Opt-in `shape.toml` field `import-graveyards = ["github.com/foo/bar"]` pulls signed entries into the local index. Ed25519 signatures already in scope make this a packaging concern, not a crypto one. Useful for shared stdlib evolutions across multiple Shape projects.
- **LSP integration.** Hover on a `#[orthogonal(...)]` directive shows the inlined deletion reason of every nearby colliding hash. Hover on a function whose behavior hash is graveyard-adjacent shows the deletion record inline before the author finishes typing. This is the "context at the cursor" inversion of Pre-condition 1's "context in the error message".

## Phasing and cost

| Phase | Scope | Cost | Gating |
|---|---|---|---|
| **Phase 0** | W-series corpus extracted to `tools/shape-test/corpora/w-series/` (9 paired fixtures + `expected.json`); inclusion-rule schemes A-D measured against corpus (Pre-condition 4) | **1 EM** | **Gates all subsequent phases.** Without the corpus every downstream decision is unfalsifiable. |
| **Phase 1** | MIR canonicalizer (passes 1-5); `behavior_hash` field on `FunctionBlob`; in-memory CAS index; emit `W0901` only (advisory) | 3 EM | Phase 0 complete; benchmark catches ≥ 7/9 W-series fixtures at FPR=0 on shipped stdlib |
| **Phase 2** | Graveyard format + Ed25519 signing (reuses `crypto/signing.rs`); `.shape/graveyard.cas` + `.shape/graveyard-mir/`; migration tool (Pre-condition 3) | 3 EM | Phase 1 advisory mode green for ≥ 14 days |
| **Phase 3** | `verify-merge.sh` CHECK 13 (`check-no-resurrection`) and CHECK 14 (migration-record gate) | 1 EM | Phase 2 complete |
| **Phase 4a** | New `attribute_directive` Pest production + closed-set parser validation + `FunctionDef.directives` field + `E0900` unknown-directive error | 2 EM | Phase 1 complete (parser work is independent of canonicalization) |
| **Phase 4b** | Six directives wired to CARS semantics; reason-string regex check (`E0903`); self-invalidation (`E0902`); ship as warning class for 30 calendar days on `bulldozer-strictly-typed` | 2 EM | Phase 4a complete |
| **Phase 4c** | Promote `W0901` → `E0901`; promote `W0903` → `E0903` | 0.5 EM | **30-day advisory period complete (Pre-condition 5); zero unresolved FPs; every stdlib trivial collision has a documented disposition.** Separate release. |
| **Phase 5** | Telemetry: cost on `cargo build`; FP rate; per-directive use counts (input to RFC-006) | 1 EM | Phase 4c shipped |
| **Phase 6** | RFC-005 ghost-activation integration (different RFC; sibling cost) | — | Tracked separately. **Pre-condition 2 binds RFC-001 ship to RFC-005 + RFC-006 being at Draft on the same release train.** |
| **MVP total** | Phase 0 through Phase 4c (excluding RFC-005/006 implementation) | **13.5 EM** (≈ 14–18 EM with buffer) | — |

The Phase-4 split is the load-bearing structural choice. **Phase 4a (parser) ships without Phase 4b (semantics).** The grammar production exists, `E0900` refuses unknown directives, the six known directives are no-ops. The closed-set property — structural defense against directive proliferation — is in place before the gate exists. Phase 4b adds semantics; Phase 4c promotes warnings to errors only after the 30-day burn-in. There is no path where authors invent directives during the advisory window.

Total cost is bounded above by **18 engineer-months**, paid up front to retire a failure mode whose single empirically-observed instance (the W-series) cost 4–6 weeks (`docs/defections.md:49`). One avoided W-series-class incident in the project's lifetime pays back ~80% of the up-front cost; two make CARS strictly profitable. The Graveyard is permanent infrastructure amortizing across every future deletion. The directives are surface area maintained at parser-layer cost. The canonicalizer is permanent infrastructure other RFCs (notably RFC-005's runtime feedback-vector hashing) reuse.

## Sources

- [Maziarz et al., "Hashing Modulo Alpha-Equivalence", PLDI 2021 (arXiv 2105.02856)](https://arxiv.org/abs/2105.02856)
- [PLDI 2021 conference page for Hashing Modulo Alpha-Equivalence](https://pldi21.sigplan.org/details/pldi-2021-papers/63/Hashing-Modulo-Alpha-Equivalence)
- [Unison: the big idea](https://www.unison-lang.org/docs/the-big-idea/)
- [Unison: a tour](https://www.unison-lang.org/docs/tour/)
- [Apache Cassandra tombstone documentation](https://cassandra.apache.org/doc/latest/cassandra/managing/operating/compaction/tombstones.html)
- [Linux kernel: submitting patches (Fixes: tag)](https://docs.kernel.org/process/submitting-patches.html)
- [Linux kernel stable-kernel-rules](https://docs.kernel.org/process/stable-kernel-rules.html)
- [Rust RFC 2451: re-rebalancing coherence](https://rust-lang.github.io/rfcs/2451-re-rebalancing-coherence.html)
- [Rust project goals 2024H2: Relaxing the Orphan Rule](https://rust-lang.github.io/rust-project-goals/2024h2/Relaxing-the-Orphan-Rule.html)
- [TypeScript 3.9 release notes (@ts-expect-error introduction)](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-9.html)
- [Total TypeScript: how to use @ts-expect-error](https://www.totaltypescript.com/concepts/how-to-use-ts-expect-error)
- [SEED: Semantic Graph based Deep detection for type-4 clone (arXiv 2109.12079)](https://arxiv.org/abs/2109.12079)
- [Semantic code clone detection using hybrid intermediate representations (PLOS One)](https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0340971)
