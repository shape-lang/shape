# RFC-005: Ghost Activation Table

- **Feature name:** ghost-activation-table
- **Status:** Draft
- **Authors:** Shape core team
- **Date:** 2026-05-18
- **Companion RFCs:** RFC-001 (`#[graveyard("reason")]` directive — provides the deletion event RFC-005 consumes), RFC-008 (real-time LLM channel — possible downstream consumer of soft-hint signals; see §Future possibilities)

## Summary

The Ghost Activation Table (GAT) is a runtime-fingerprint persistence mechanism that detects **semantic re-introduction** of previously deleted code, even when the new code is structurally different from the old. It complements RFC-001's `#[graveyard("reason")]` directive — a *static* defection guard preventing textually or structurally similar code from re-landing — by catching what RFC-001 cannot: an LLM (or human) rewrites the deleted behavior with different identifiers, different control flow, or a different decomposition, while preserving observable I/O.

Mechanism: during `just test` runs, every pure function at tier `≥ Tier::BaselineJit` (≥100 calls) with call depth `≤ 8` is sampled at rate 1-in-N (default 64). The sample computes SimHash-64 over the MessagePack-canonicalized `(args, return_value)` tuple using Shape's existing `shape-wire` codec. Sketches are stored as a ring buffer (last 32) per `FunctionBlob` content hash. When a function is deleted with `#[graveyard("reason")]`, its last-known sketches are persisted into `.shape/ghost-activations.cas` alongside RFC-001's graveyard entry. The CI gate `just check-ghost-activations` compares every live function's recent sketches against the ghost table; a Jaccard match at `τ = 0.85` over `≥ 10` samples drawn from `≥ 3` distinct input shapes is a hard CI failure that cites the *original* deletion reason.

The mechanism is uniquely cheap for Shape because of one design lever no other language has cleanly: Shape's compiler already bakes `required_permissions` into every `FunctionBlob.content_hash` (`crates/shape-vm/src/bytecode/content_addressed.rs:64`). GAT sketches **only pure functions** — `required_permissions.is_empty()` — which means non-determinism is excluded at the type level, for free. Sketch reproducibility comes from the type system, not from runtime taint tracking, sandboxing, or noise scrubbing.

This RFC is the **novelty leader** of the current RFC slate: we are unaware of any production system that persists runtime behavioral fingerprints of deleted code to catch later semantic reintroduction. The closest extant analogue is Hypothesis's failing-example database, which preserves *triggering inputs* but not *behavioral signatures*, and fires on a property test rather than on an absence-of-symbol contract.

## Motivation

`CLAUDE.md` documents a recurring failure mode the project calls a "W-series defection": a plan to delete a runtime mechanism (`ValueWord`, the dynamic-dispatch path, `synthesize_value_word_from_raw`, the kind-blind `MethodFnV2` ABI) is enacted, then within weeks a future session reintroduces an isomorphic mechanism under a different name — "ValueBits shim", "FFI-boundary bridge", "tag-decode helper". The W-series alone consumed nine commits and an estimated 4–6 engineer-weeks of cleanup. The §"Renames to refuse on sight" list, with its broader-family regex

```
(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)
```

is the closest extant prior art: a *static, manually curated* defection guard maintained as a CI grep. It works for the names already on the list. It does not work for names the next agent invents tomorrow.

RFC-001 adds a structured deletion directive. A `#[graveyard("reason")]` annotation marks code as permanently deleted; the static analyzer refuses any commit that reintroduces a function with the same name, signature, or hash. This catches **structural reintroduction**: copy-paste, near-rename, hash-equal restore. It does **not** catch the W-series' most common failure mode: an LLM agent, given a problem the deleted code happened to solve, reimplements a *behaviorally equivalent* solution with different identifiers, decomposition, and syntactic shape. The author has no memory the code existed; the AST does not match; RFC-001 sees nothing.

GAT is the runtime layer of a two-tier defense:

1. **Static (RFC-001):** prevents textually or structurally similar code from re-landing.
2. **Dynamic (RFC-005):** prevents *behaviorally* similar code from re-landing — even when the implementation is unrecognizable.

The pre-condition that makes layer 2 viable is the pre-condition motivating the strict-typing program: Shape *already* has compile-time-proven purity (the `required_permissions` set, baked into `content_hash`) and *already* has a deterministic canonical serialization (`shape_wire::codec::encode`). The infrastructure is in place; GAT is the additional wiring.

A worked example. In session N, `fn normalize_tag_bits(raw: u64) -> u64` is deleted with `#[graveyard("reintroduces the deleted ValueWord tag path; see CLAUDE.md §Forbidden Patterns")]`. Before deletion it accumulated 28 sketches across 8 distinct input shapes. In session N+5, a new agent — given a bug report mentioning "stack slots show odd low-bit patterns" — writes `fn canonicalize_slot_word(slot: u64) -> u64` with different control flow. RFC-001 sees a new function with a new name and new hash — passes. But `just test` now runs the new function with similar inputs, producing similar outputs. After 12 sample-gated executions, GAT computes Jaccard(new, ghost) = 0.91. CI fails:

```
error[GAT-001]: live function `canonicalize_slot_word`
  (crates/shape-vm/src/executor/slots.rs:142)
  matches ghost-activation sketch of deleted `normalize_tag_bits`
  (Jaccard 0.91, 12 samples, 5 distinct input shapes)

  original deletion reason (from graveyard entry, 2026-04-22):
    "reintroduces the deleted ValueWord tag path; see CLAUDE.md
     §Forbidden Patterns"

  ghost hashes: sha256(7c9a..) vs live: sha256(2b1d..)
  threshold τ = 0.85; matched bits 58/64 (avg)

  next step: confirm intent. If the new function genuinely needs to
  exist (i.e. the graveyard reason no longer applies), update the
  graveyard entry with a counter-rationale. Otherwise revert.
```

The diagnostic carries the same authority the §"Renames to refuse on sight" list carries today, but it is *automatic and behavior-derived* rather than name-pattern-matched.

## Guide-level explanation

GAT is invisible during normal development. There is no annotation to write, no opt-in flag to remember. The user-visible touchpoints are:

1. The `#[graveyard("reason")]` directive (defined by RFC-001, *consumed* by RFC-005).
2. The CI recipe `just check-ghost-activations` and its diagnostic.
3. The on-disk file `.shape/ghost-activations.cas`, checked in alongside graveyard entries.

When you delete a function with `#[graveyard("reason")]`, the next `just test` run promotes any existing sketches for that function's `content_hash` into the ghost table. The promotion is atomic with the graveyard commit.

When a later commit introduces a new function that accumulates enough sketches to constitute a behavioral signature, the CI gate compares each live function's ring against every ghost. The two configurable knobs are:

- **τ (similarity threshold):** default `0.85`. Match means Jaccard(live_set, ghost_set) ≥ τ.
- **Witness budget:** default `≥ 10 samples` drawn from `≥ 3` distinct input shapes. Below this, the match is held *advisory* — surfaced in `shape lsp` hover and `cargo test` summary but not a CI failure. This prevents false positives on tiny helpers where unrelated functions coincidentally agree on `1 + 1 == 2`.

RFC-001 owns the deletion event; RFC-005 reads it. The `.shape/` directory holds both files:

```
.shape/
  graveyard.cas             # RFC-001: deletion entries, content-addressed
  ghost-activations.cas     # RFC-005: last-known sketches per ghosted hash
```

Both are append-only and content-addressed (the `.cas` suffix mirrors `crates/shape-vm/src/bytecode/content_addressed.rs`). Both ride the same Ed25519 signing infrastructure (`crates/shape-runtime/src/crypto/signing.rs`) — a ghost entry signed by the deleter has the same provenance guarantee as a package manifest.

## Reference-level explanation

### 1. Sample-collection hooks

Two executor hook points capture `(args, return_value)` at function boundaries.

**Entry hook.** `call_function_with_nb_args` at `crates/shape-vm/src/executor/call_convention.rs:557` is the non-closure frame-setup site. The arg slice has been validated and the frame pushed, but execution has not started. The slice is `&[KindedSlot]` per ADR-006 §2.7.10 / Q11 — each slot carries a `(bits, NativeKind)` typed pair. The sketch kernel reads bits + kind for each arg. No new value-shape contract; the carrier is the §2.7.7 parallel-kind ABI.

**Exit hook.** `return_value_inner` at `crates/shape-vm/src/executor/control_flow/mod.rs:759` is the shared inner body that both `op_return_value` and the typed `op_return_value_<Kind>` family (Wave-E+3, opcodes 0x198..=0x1A2) funnel through. Its arguments are `(return_bits: u64, return_kind: NativeKind)` — the same §2.7.7 carrier.

Both hooks are `#[cold]`-marked branches behind a single predictable AND, borrowing the GC safepoint pattern at `crates/shape-vm/src/executor/dispatch.rs:446`:

```rust
if self.instruction_count & 0x3FF == 0 && self.interrupt.load(Ordering::Relaxed) > 0 {
    return Err(VMError::Interrupted);
}
```

`& 0x3FF == 0` is exactly the shape GAT's sample gate emits — one bit-AND and one branch, predicted not-taken on the common path. Hot-path cost when *not* sketching is one ALU op per call boundary.

### 2. Sample-rate gating

```rust
const SAMPLE_RATE: u64 = 64;  // power of two; tunable via env var

#[inline(always)]
fn should_sketch(&mut self, function_id: u16) -> bool {
    // gate-1: cheap counter mask (single AND, predictable branch)
    let counter = self.sketch_counter.wrapping_add(1);
    self.sketch_counter = counter;
    if counter & (SAMPLE_RATE - 1) != 0 { return false; }

    // gate-2: tier check (already cached on FunctionTierState)
    let tier = self.tier_manager.tier(function_id);
    if tier < Tier::BaselineJit { return false; }

    // gate-3: call-depth cap (cheap field read)
    if self.call_stack.len() > 8 { return false; }

    // gate-4: purity filter (single PermissionSet::is_empty)
    let blob = &self.program.function_blobs[function_id as usize];
    if !blob.required_permissions.is_empty() { return false; }

    true
}
```

The four gates compose into a cheap funnel: most calls fail gate-1; the few that pass reject on tier, then depth (excludes deep recursion that would non-linearly amplify sample volume), then purity. The cost of a passed gate is dominated by the SimHash kernel below, amortized over 64 calls.

### 3. SimHash-64 kernel

For each sketched call, the kernel computes one 64-bit signature over `(args, return_value)`:

```rust
fn sketch(args: &[KindedSlot], ret: KindedSlot, kind_ret: NativeKind) -> u64 {
    // Step 1: canonical serialization via shape-wire MessagePack codec
    //   (crates/shape-wire/src/codec.rs:27 — already used by REPL + fchart)
    let envelope = ValueEnvelope::tuple(args, ret);
    let bytes = shape_wire::codec::encode(&envelope);

    // Step 2: feature extraction — 4-byte shingles over the MessagePack bytes
    //   (4 chosen empirically: small enough to be granular over short values,
    //   large enough to keep the feature set bounded for long ones)
    let mut acc = [0i32; 64];
    for window in bytes.windows(4) {
        let h = xxhash64(window);
        for bit in 0..64 {
            acc[bit] += if (h >> bit) & 1 == 1 { 1 } else { -1 };
        }
    }

    // Step 3: sign collapse — each accumulator becomes one bit of the sketch
    let mut sketch = 0u64;
    for bit in 0..64 {
        if acc[bit] > 0 { sketch |= 1 << bit; }
    }
    sketch
}
```

This is canonical SimHash-64 (Charikar 2002) over a 4-byte shingle alphabet, with MessagePack as the canonicalization layer. Two functions producing the same `(args, return_value)` produce the same sketch; two functions producing *similar* tuples produce sketches with low Hamming distance (which we later convert to Jaccard — see §Rationale).

### 4. Ring-buffer storage

Per `FunctionBlob`, a `SketchRing` of capacity 32 holds the most recent sketches. 32 × 8 bytes ≈ 280 bytes per blob with overhead. For a 2,000-blob program, worst-case incremental memory is ~560 KB — negligible.

`SketchRing` lives as a sidecar on `FeedbackVector` (`crates/shape-vm/src/feedback.rs:101`):

```rust
pub struct FeedbackVector {
    pub function_id: u16,
    pub slots: HashMap<usize, FeedbackSlot>,
    pub generation: u32,
    pub sketch_ring: Option<Box<SketchRing>>,    // RFC-005 — new field
}
```

`Option<Box<_>>` keeps `FeedbackVector`'s common-path layout untouched: on non-pure functions or before the first sketch, the field is a single null-checked pointer. Same layout-preserving discipline as `CallFrame.closure_heap_bits` (ADR-006 §2.7.8 / Q10).

`SketchRing` also tracks the **input-shape set** the witness budget needs. "Shape" means the canonical MessagePack type-tag tuple of `args` — e.g. `(int, int)`, `(Array<number>, decimal)`. A `HashSet<u32>` of FNV-hashed shape tags counts distinct shapes; only the hashes are persisted.

### 5. Ghost-activation persistence

When the compiler processes a `#[graveyard("reason")]` directive (RFC-001), it emits a build-time hook that, at the *next* successful `just test` run, drains the deleted function's `SketchRing` into `.shape/ghost-activations.cas`. Format:

```rust
#[derive(Serialize, Deserialize)]
pub struct GhostActivation {
    /// FunctionBlob content_hash at time of deletion (32 bytes)
    pub deleted_hash: [u8; 32],
    /// Original function name (for diagnostic only — not used in matching)
    pub name: String,
    /// Graveyard reason verbatim from #[graveyard("...")] — diagnostic anchor
    pub reason: String,
    /// Sketches accumulated up to deletion (max 32, may be fewer)
    pub sketches: Vec<u64>,
    /// Distinct input-shape hashes observed
    pub input_shapes: Vec<u32>,
    /// Per-arg NativeKind tuple — used to skip type-incompatible live candidates
    pub arg_kinds: Vec<NativeKind>,
    /// Return NativeKind
    pub return_kind: NativeKind,
    /// Optional Ed25519 signature (signs sha256 of all above)
    pub signature: Option<ModuleSignatureData>,
}
```

Signature reuses `ModuleSignatureData::sign` at `crates/shape-runtime/src/crypto/signing.rs:14-56`. The signed payload is the SHA-256 of the rmp-serde encoding of every field above `signature` — the same structural pattern Shape uses for module manifests. A signed ghost entry cannot be silently dropped or modified without invalidating the signature, and the CI gate refuses such a commit.

`.cas` files are append-only. Removing a ghost requires an explicit "exhumation" commit (a new RFC-001 directive variant, out of scope here) that updates the record with a counter-rationale — analogous to `docs/defections.md`.

### 6. CI-gate algorithm

`just check-ghost-activations` runs after `just test` completes (or, in iterative use, against the most recent `target/sketch-ring.bin` snapshot the executor writes on test-runner shutdown). The algorithm:

```
for live_fn in current_program.functions:
    if live_fn.required_permissions is not empty: continue
    live_ring = sketch_rings[live_fn.id]
    if live_ring.distinct_shapes < 3 or live_ring.len < 10: continue

    for ghost in ghost_table:
        if ghost.arg_kinds != live_fn.arg_kinds: continue
        if ghost.return_kind != live_fn.return_kind: continue
        jaccard = jaccard_estimate(live_ring.sketches, ghost.sketches)
        if jaccard >= 0.85:
            FAIL with diagnostic citing ghost.reason
```

`jaccard_estimate` over SimHash signatures uses `J ≈ 1 − H/64` where `H` is the average pairwise Hamming distance between sketches in the two sets. For 32-vs-32 rings, 1024 XOR+popcnt ops per candidate pair (~2 µs); ~2 ms per live function for a 1000-ghost table; ~2 s for a 1000-function program. The kind-tuple pre-filter cuts the candidate set sharply in practice; expected total gate runtime is under 1 s for plausible program sizes.

The gate is exit-code-based, following `scripts/check-no-dynamic.sh` and `scripts/verify-merge.sh`. Recipe:

```just
# .shape/ghost-activations.cas regression guard.
# See docs/rfcs/005-ghost-activation-table.md.
check-ghost-activations:
    bash scripts/check-ghost-activations.sh
```

### 7. Purity filter — the Shape leverage

This is the design decision that distinguishes GAT from a generic system. §1 hooks, §3 kernel, §5 persistence are all language-agnostic; the purity filter (gate-4) makes the apparatus *cheap and correct* in a way that, to our knowledge, no other language can match without significant additional engineering.

In Python, Ruby, Java, or JavaScript, "pure function" is something the runtime must *infer* — via taint analysis, side-effect-tracking instrumentation, or coarse "did this call any I/O syscall" wrappers. Each option adds ~30–50% overhead and is approximate. In Shape, purity is `PermissionSet::is_empty()` on a field that is already populated at compile time, baked into the function's `content_hash`, and validated by the type system. Gate-4 is one cache-resident pointer chase plus one bitmask.

The sharper consequence is correctness, not performance. Because purity is compile-time-proven, every sketch we ever store is over a deterministic computation. No clock-read, random source, FS access, or network read can perturb the sketch between runs. Reproducibility on a fixed input is a property of the source code, not of the sampling apparatus. This is what removes the class of false positives that Daikon-style invariant inferencers must defend against in industrial deployment, and what makes GAT viable as a *hard* CI gate rather than an advisory signal.

## Drawbacks

**Hot-path cost.** Every call boundary pays one ALU op + one predictable branch. Modeled against the existing GC-safepoint pattern at `dispatch.rs:446`, this is the same cost class: under 1% on JIT-translated code (the JIT inlines the gate and DCE eliminates it when disabled at the call site) and under 3% on the interpreter at default sample rate. We will publish before/after numbers from `crates/shape-vm/benches/vm_benchmarks.rs` at Phase 1 close.

**Non-determinism breaks sketches.** Without the purity filter, two `time_millis()` calls produce different return values, different sketches, useless comparison. The purity filter eliminates this class. Functions that *appear* pure but transitively call impure stdlib fail the compile-time `required_permissions` check and are excluded automatically — no escape hatch.

**Sketch storage growth.** Worst-case 2,000 pure functions × 256 B per ring × N workers ≈ 512 KB × N. For `just test-all` at 8 workers, ~4 MB resident — below `cargo test`'s own noise floor.

**Staleness.** A ghost signed at deletion time may later collide with a live function that uses a different shape (field reorder, swapped argument order) for the same computation. The arg-kind / return-kind pre-filter in §6 restricts to true behavioral isomorphism — but a *genuinely* different-typed rewrite is exactly what we want *not* to match. The witness budget (≥3 distinct shapes) mitigates single-overlap collisions.

**Privacy implications under production sampling.** This is the binding constraint. Sketches carry MessagePack-canonicalized argument bytes; an adversary with read access to `.shape/ghost-activations.cas` could in principle recover input distributions for the deleted function. Test-corpus-only sampling sidesteps this entirely. RFC-005 **explicitly defers** production sampling to a future opt-in mechanism (see §Future possibilities).

**False negatives on rare-input regressions.** A reintroduced function not exercised ≥10 times across ≥3 shapes will not fire. This is intentional: the witness budget keeps the false-positive rate below the level that would erode trust in the gate. The corresponding false-negative rate is the tradeoff.

**The defection itself.** The central risk of GAT is that a future agent rationalizes disabling it. The §Forbidden Patterns rules in `CLAUDE.md` apply transitively — "soft-fail counter for now, harden later" is an explicitly refused rationalization. The CI gate must be exit-code, not advisory.

## Rationale and alternatives

**Why test-corpus-only, not production?** Hypothesis's precedent: the failing-example database is local to the developer's machine, not shipped from production. Same reason here — production I/O is the user's data, and arbitrary persisted sketches cross privacy and security boundaries. The differential-privacy literature shows that even small leaks of input distributions support membership inference. We do not opt into that liability without an explicit, signed, configurable consent mechanism — RFC-005-future, not RFC-005-initial.

**Why SimHash, not MinHash?** Both estimate Jaccard at fixed memory cost. MinHash sketches a set as k minimum hash values (typically 128–256 hashes for usable accuracy). SimHash sketches a bag as one fixed-width signature. For fingerprinting an (args, return_value) tuple, SimHash's 64-bit signature is enough at τ=0.85, and its XOR+popcnt similarity test outperforms MinHash's k-hash comparison by ~10× on the hot path. Google's 2007 web-crawl-dedup study landed on SimHash for the same reason.

**Why filter to pure functions?** Three reasons. (1) Correctness: impure functions emit different sketches on identical inputs, breaking Jaccard semantics. (2) Cost: the check is one `PermissionSet::is_empty()` on a field Shape already maintains (§7). (3) Defection-attractor avoidance: a "we'll sketch impure functions too, with a clock-fuzz scrubber" rationalization is exactly the shape `CLAUDE.md` §Forbidden refuses. Pure-only is a bright line that does not invite re-litigation.

**Why JIT-tier gating?** Cold functions are not the W-series risk profile. The W-series regressions live in load-bearing runtime code that test suites hit tens of thousands of times. Tier ≥ BaselineJit (≥100 calls) is the existing heuristic for "this function matters"; reusing it costs zero new infrastructure and excludes the long tail of one-shot helpers whose sketches would dilute the ghost table.

**Why Jaccard, not raw Hamming?** Convertible (`J ≈ 1 − H/64`), but Jaccard expresses the threshold in set-similarity units that compose with the witness budget. Hamming over single signatures answers "are these two calls similar"; Jaccard over sketch *sets* answers "do these functions exhibit similar behavior across a witness population". The gate fires on the latter.

**Why not Daikon-style invariant inference?** Daikon (Ernst et al., MIT CSAIL, 2001) infers likely invariants from observed I/O — the same data GAT collects. The two systems differ in target: Daikon publishes invariants for documentation; GAT compares fingerprints to ghosts as a regression gate. Daikon's commercial trajectory (Agitator at Agitar, ISSTA 2006) is a cautionary tale — invariant inference is expensive and false-positive-prone at industrial scale. GAT's narrower charter — match-or-not against a small ghost table with a hard witness budget — is what keeps it tractable.

**Why not eBPF-style uprobes?** eBPF function-argument tracing in production is mature (Pixie, Datadog, Keploy at 1–5% overhead). For Shape it would require kernel infrastructure that doesn't compose with the VM's content-addressed model. Our hook points are inside the executor — no syscalls, no kernel, no portability hit.

**Why not record-and-replay (rr / Pernosco)?** rr captures *full* deterministic execution traces for reverse debugging, ~1.2× overhead on Firefox suites. GAT is two orders of magnitude cheaper because it samples one 64-bit signature per N calls, not every memory access. The two are complementary: rr answers "what happened in this run"; GAT answers "does the program now exhibit a behavior we previously deleted".

**Store full input-output examples instead of sketches?** Rejected. (1) Storage: real test workloads run 10^6+ calls per pure function. (2) Diff semantics: full-tuple equality is too brittle, full-tuple inequality too loose. SimHash interpolates at fixed cost.

**AST-similarity matching at compile time?** That is RFC-001's territory. The entire motivation here is to catch reintroduction that is *not* structurally similar.

## Prior art

**Hypothesis (Python, MacIver et al., 2013–).** `.hypothesis/examples/` persists failing examples across runs — the closest extant analogue to GAT's ghost table. Differences: Hypothesis persists *triggering inputs* for property-test replay, not *behavioral signatures*; it fires on property failure, GAT on behavior match regardless of any property; its database is per-developer-machine and explicitly fragile across version upgrades, while GAT's `.cas` files are checked-in repo artifacts with Ed25519 signatures.

**Daikon (MIT CSAIL, Ernst et al., 1999–).** Dynamic invariant detection: observes I/O at function boundaries, infers likely invariants (`result > arg1`, `arg1.length == result.length`). Published as runtime documentation. Commercial trajectory through Agitator (Agitar, ISSTA 2006) was marginal — false-positive rate at industrial scale required substantial human curation. GAT borrows the I/O-at-boundaries observation pattern but is narrower in scope (fingerprint match, not invariant inference) and stricter in admission (pure functions only via the compile-time permission check).

**V8 / HotSpot / LuaJIT profile recording.** Tiered VMs record per-call-site type observations into IC feedback vectors; HotSpot's profile-caching work (Pelizzari et al., 2017) persists profiles across runs to amortize warm-up. Shape's `FeedbackVector` (`crates/shape-vm/src/feedback.rs:101`) follows the same lineage; GAT extends it with a `SketchRing` sidecar. IC profiles capture *type* observations (which schema_id at this property access); sketches capture *value* observations (SimHash over the canonical encoding of args+return).

**eBPF / bpftrace / uprobes (Linux kernel, 2014–).** Kernel-side sampled function tracing with argument capture, 1–5% overhead in production deployments (Pixie, Datadog). Establishes the engineering viability of low-overhead sampled I/O capture at boundaries. GAT lives in the VM, not the kernel, which keeps it portable across hosts.

**rr / Pernosco (Mozilla, 2014–).** Deterministic full-trace record-and-replay with reverse debugging, ~1.2× overhead on Firefox suites. GAT is two orders of magnitude cheaper because it samples and fingerprints rather than retaining the full trace.

**MinHash (Broder, 1997) / SimHash (Charikar, 2002) / LSH literature.** Sketch-based set similarity at constant memory; standard tooling at web scale (Google's 2007 SimHash crawl-dedup deployment). GAT's kernel is canonical SimHash-64 over MessagePack shingles — no new theory, novel application.

**`CLAUDE.md` §"Renames to refuse on sight" regex.** The closest *internal* prior art: a curated CI grep against `(decode|tag|kind|dispatch|value.call|...) (bridge|probe|helper|...)`. Effective for known names, powerless against the next name the agent invents. GAT is the behavioral-fingerprint complement: the rename regex catches new names humans write; the sketch ring catches new behaviors agents emit.

**Negative result on prior art.** We searched explicitly for "deletion fingerprint persistence at runtime", "anti-pattern reintroduction detection at CI", and "regression-policy database" and found no production system. The closest hits are clone-detection tools (CCFinder, NiCad, Deckard) that find textual or AST near-duplicates in the *current* codebase, and regression test selection (Celik et al., ICSE 2018+) using dynamic dependency fingerprints to skip tests — neither fires policy gates against *removed* code. Persisting runtime behavioral fingerprints of deleted code as a contract that live runtime behavior must not match appears unenumerated in the literature we surveyed.

## Unresolved questions

**Sketch versioning across MessagePack format changes.** rmp-serde is stable within a major version, but a struct-field reorder in a stdlib type invalidates every prior sketch. Proposal: ghost entries record a `sketch_format_version: u16` field; a version mismatch demotes the ghost to advisory and emits a diagnostic suggesting re-sketching against the deleted hash's last-known commit. Open: whether to attempt automatic re-sketching by checking out the prior commit in CI, or treat format bumps as a human-curated cutover.

**Recursive-call sketch policy.** A call at depth 9 fails the call-depth gate but contributes nothing to its own outer-frame sketch. Should outer-frame samples count toward the witness budget? Current proposal: yes (the outer args/return are what we care about). Open: whether to also sample the deepest non-capped frame as a "tail sketch" to catch recursion-shape changes.

**How would production opt-in be designed?** Sketch payloads in raw form leak input distributions. A production-safe design would (a) apply differential-privacy noise (Gaussian on per-bit accumulators before §3's sign-collapse), (b) require per-deployment Ed25519 attestation, (c) ship sketches to a separate `.shape/ghost-activations-prod.cas` not version-controlled by default, (d) raise τ to 0.95 and the witness budget to ≥100 samples / ≥10 shapes to compensate for noise. RFC-005-future, not RFC-005-initial.

**Cross-platform sketch reproducibility under FP non-determinism.** `Float64` slot values can diverge across x86 and aarch64 for transcendental ops. Options: (a) exclude `Float64` returns from sketching; (b) round to 53-bit precision before encoding; (c) per-architecture ghost tables. Lean (b) for Phase 1.

**Interaction with `comptime` evaluation.** Comptime calls execute the same bytecode. Sketch them? Pro: most stable input distribution (all literals). Con: a comptime-only function accumulates samples that may not reflect runtime behavior. Lean: skip comptime samples in Phase 1 via a `vm.is_comptime` check.

## Future possibilities

**Production sampling with differential-privacy salt + opt-in attestation.** Per the unresolved-questions sketch: a deployer signs a per-deployment Ed25519 attestation, production sketches are noised at capture time, signed with the attestation key, and shipped to a separate ghost file. The CI gate cross-references both. Noise scaling follows the standard `(ε, δ)`-DP framework; ε ≤ 1 is plausible given the existing sample-rate gating.

**Federation across projects.** A package author publishing to `shape-registry` can opt into uploading `.shape/ghost-activations.cas` as part of the manifest. Downstream consumers' CI then checks live functions against the *transitive* ghost set across dependencies. Per-entry Ed25519 signatures preserve authorship and prevent one package from poisoning another's gate. Natural extension of RFC-001's graveyard model from per-project to per-ecosystem.

**Ghost activations as soft-hint to RFC-008's real-time LLM channel.** When the LLM is about to land code GAT would flag, RFC-008's channel surfaces the ghost reason and offers two paths: revert, or draft a counter-rationale to be added to the ghost entry. Path (b) requires explicit human signoff and a `docs/defections.md` entry. Converts GAT from a one-way refuse-on-CI gate into a structured negotiation surface — the agent's mistake becomes a self-documenting moment in institutional memory.

**LSP integration.** Surface an inlay hint on any live function whose ring is approaching a ghost match; hover shows the ghost's name, reason, and Jaccard percentage; the hint becomes a CI-failing squiggle once the witness budget is met. Shifts feedback from CI-time to edit-time.

**Cross-function sketch comparison.** Sketch at call-site granularity rather than function granularity, catching behavioral reintroduction split across multiple new functions. Cost: O(call_sites × ghosts) instead of O(functions × ghosts). Defer until the simpler form proves out.

## Phasing and cost

Total: **5–6 engineer-months** across three phases. Assumes the entry/exit hook sites and `FeedbackVector` sidecar pattern survive review unchanged.

**Phase 1 — executor hook + ring buffer + ghost-activation storage. ~2 EM.** Insert the sample gate at `call_function_with_nb_args:557` and `return_value_inner:759`. Implement `SketchRing` and the SimHash kernel. Add the `Option<Box<SketchRing>>` field to `FeedbackVector`. Define `GhostActivation` and the `.shape/ghost-activations.cas` write path. Integrate with RFC-001's `#[graveyard("reason")]` (mock the directive until RFC-001 lands). Bench against `crates/shape-vm/benches/vm_benchmarks.rs` to validate the <1% / <3% hot-path cost targets. **Exit:** `just test` runs at default sample rate without measurable regression on the bench suite; `.shape/ghost-activations.cas` correctly populated on a synthetic deletion test.

**Phase 2 — CI gate diagnostic. ~1.5 EM.** Write `scripts/check-ghost-activations.sh` following the exit-code template at `scripts/check-no-dynamic.sh` and `scripts/verify-merge.sh`. Wire `just check-ghost-activations`. Build the diagnostic formatter (Jaccard %, matched bits, ghost reason citation, graveyard path). Integration tests that synthesize a deleted-then-reintroduced function pair and verify the gate fires correctly. **Exit:** synthetic regression test fires the gate; an engineered false-positive corpus of 10 small pure-helper pairs that coincidentally agree does *not* fire it.

**Phase 3 — LSP surfacing. ~1.5 EM.** Extend `tools/shape-lsp/` with inlay-hint and hover protocol; adapt semantic-tokens to surface ghost-match warnings as squiggles; integrate with diagnostic streaming so warnings update as the developer edits. **Exit:** in a VS Code session against a synthetic project with one ghosted function, typing a reintroduction surfaces a squiggle within the typical LSP refresh cycle.

The estimate assumes RFC-001 lands first. If RFC-001 ships extensions GAT can reuse — `.shape/` directory layout, Ed25519 workflow — Phase 1 drops by ~0.5 EM.

The largest *risk* is not engineering — it is institutional. The §Drawbacks "defection itself" point is binding: every behavioral-fingerprint system this RFC's authors have studied (Daikon's commercial fate; the long tail of half-deployed observability tools) failed not because the technology was wrong, but because operators rationalized disabling it after the first false positive. The mitigations baked into RFC-005 — hard exit code (no advisory mode), Ed25519-signed ghost entries (no silent deletion), the `CLAUDE.md` §Forbidden Patterns regex that explicitly refuses "soft-fail counter for now, harden later" — are the architectural answer. They are not a substitute for ongoing discipline.
