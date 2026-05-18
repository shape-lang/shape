# RFC-006: Reason-String Indexing for Escape Directives

- **Status:** Draft
- **Date:** 2026-05-18
- **Depends on:** RFC-001 (`attribute_directive` grammar, escape directives), RFC-004 (`#[replaces]`)
- **Relates to:** RFC-008 (channel signal — future)
- **Forbidden-rationalization anchors:** `CLAUDE.md` §"Forbidden rationalizations" (9 phrases) + §"Renames to refuse on sight" (broader-family regex)

## Summary

Every CARS-shaped mechanism (Capability / Audit / Refusal / Sigil) has a load-bearing failure mode: the **escape hatch rots**. The mechanical gate stays sharp; the free-text reason attached to each escape decays toward boilerplate, copy-paste, and cargo-cult. Within a few quarters the escape population is dominated by reasons that *sound* engineering-shaped but encode no actual judgement — "temporary workaround", "edge case", "FFI-boundary bridge".

Shape's `CLAUDE.md` already enumerates this failure mode by hand: nine literal "Forbidden rationalizations" and a `(decode|tag|kind|dispatch|...) (bridge|probe|helper|hop|translator|adapter|shim)` regex covering ~25 renames. That hand-curated list is the proof of concept: humans on a real codebase under real defection pressure *have already extracted* the rotted-reason patterns. It is the only thing that stopped the W-series rename from repeating during Phase 2d work.

This RFC mechanizes that list. It (1) captures every escape directive (RFC-001 `#[distinct_from]` / `#[fork_from]` / `#[orthogonal]` / `#[exhume]` + RFC-004 `#[replaces]`) into an append-only `.shape/reason-corpus.jsonl`; (2) embeds each reason string with a **deterministic static embedding** (Model2Vec / potion-retrieval-32M) so reasons cluster by *meaning*, not by surface syntax; (3) surfaces near-neighbor matches as **soft warnings**, escalates to **hard error** on cluster-recurrence, rejects banlist-matching reasons (the literal lift of `CLAUDE.md` §Forbidden-rationalizations) outright; (4) provides one override directive `#[reason_cluster_acknowledged(cluster_id, signature)]` — Ed25519-signed sigil that records *who* overrode *which* cluster, for forensic audit.

Phase 1 (banlist alone, 1 week) is strictly better than today's documentation-only enforcement. Phase 2 (embedding + advisory, 2 weeks) and Phase 3 (clustering + sign-off, 1 week) build on it. Total: 3–4 weeks against the empirically-measured 4–6 week cleanup of the single W-series rename this RFC would have caught at first occurrence.

## Motivation

### The W-series, in one paragraph

`docs/v2-nanbox-removal-plan.md` Step 6 originally read "delete `ValueWord`". Mid-execution it was rewritten to "ValueBits shim retained as documented FFI-boundary bridge". That rename converted a one-time deletion into permanent maintenance debt: the 2,650-line `ValueWord` module preserved, nine W-series commits of "decode bridges", four deferred v2-raw-heap aliasing tests `#[ignore]`'d in `bin/shape-cli/tests/stdlib/simulation.rs`, ~23 ignored `shape-jit` tests, ~48 `shape-test` failures (`shape-test-residuals-audit`). The empirical cost in `CLAUDE.md` §"Why this matters" is 4–6 weeks of cumulative cleanup; `docs/defections.md` (7,201 lines, 28+ dated entries) is the receipt. The *rationalization that made the rename feel reasonable in the moment* was three words: "FFI-boundary bridge".

The mechanical gate (`scripts/check-no-dynamic.sh`) was not yet in place at the time — but even if it had been, a rename to a name not on the banlist would have passed. The rename made it past human review *because the reason sounded reasonable*. The mechanism that should have caught it is recognising that "FFI-boundary bridge" is the same shape as "tag-decode bridge" is the same shape as "ValueBits shim", and that this shape has appeared 30+ times in the project's defection log. An embedding-space clustering would have placed all three within ε = 0.1 of each other, and "FFI-boundary bridge" would have crossed the cluster-recurrence threshold before it landed.

### Confirmation from prior art (the failure mode is universal)

Every system that pairs a mechanical gate with a free-text escape suffers the same rot:

- **TypeScript `@ts-expect-error` / `@ts-ignore`.** `@ts-expect-error` improves on `@ts-ignore` precisely because it *fails when the suppression becomes useless* — a feedback loop. Neither directive constrains the *reason* string; both rot into "TODO" / "fix later" / "TS bug". Best-practice posts (Hahn, Osbourne, Goldberg) all converge on "include a reason and review periodically" — manual discipline that does not scale.
- **TikTok ESLint bulk suppressions.** The TikTok frontend monorepo accumulated **70,000+ ESLint suppressions** before building `eslint-bulk-suppressions` (upstreamed April 2025) to ship them out-of-band in a JSON sidecar. The inline-rationalization channel had collapsed under volume long before the tool was built.
- **Rust `#[allow]` → `#[expect]`.** Rust 1.81 (2024) added `#[expect]` because `#[allow]` accumulates silently. `#[expect]` fires a warning when the underlying lint *stops* triggering. Same lesson as `@ts-expect-error`: the mechanical channel has to push back.
- **Karampatsis & Pradel, FSE 2025 — "An Empirical Study of Suppressed Static Analysis Warnings".** Across Pylint / Checkstyle / PMD / ESLint over 6.69M LoC: **7,357 suppressions, monotonically increasing over project lifetime, 50.8% useless** (no longer suppress anything). The empirical population statistic for the failure mode.
- **`cargo audit` `audit.toml` ignore-with-reasons.** Closest to what this RFC proposes — `ignore = [{ id = "RUSTSEC-…", reason = "…" }]`. Reason is free-text, ungrep'd, unclustered: a documentation slot, not a signal.

Across all five: the mechanical gate is the easy part; the reason channel is where the rot lives; every existing system treats reason strings as documentation, not signal.

### The asymmetry this RFC closes

In Shape today, the mechanical gate is sharp: `scripts/check-no-dynamic.sh` runs against a per-symbol monotonic baseline, `scripts/verify-merge.sh` runs 12 exit-code-based checks, `prove_native_kind()` returns a `ProofGap` whose constructor is private. The escape channel is dull: free-text English in `docs/defections.md`, periodically grep'd by a human, periodically distilled into `CLAUDE.md` §Forbidden-rationalizations by hand. This RFC closes the asymmetry.

## Guide-level explanation

### Author workflow — escape directive flagged as cluster-near-neighbor

```shape
#[distinct_from(crates::shape_value::value_word, reason = "needed as an FFI-boundary translation layer for snapshot serialization")]
fn snapshot_value_to_bytes(slot: KindedSlot) -> Vec<u8> { ... }
```

Compile output:

```
warning: reason text similar (cosine = 0.91) to 11 prior escape reasons across 7 commits:
  - 2026-04-18: "ValueBits shim retained as documented FFI-boundary bridge"   (commit 8a3f2c1)
  - 2026-05-02: "tag-decode bridge for the cross-VM/JIT boundary"             (commit 1c8e7f3)
  - 2026-05-04: "boundary translation helper for capture-decode hop"          (commit 92ab44e)
  ... 8 more
  cluster_id: c-bridge-002    size: 11    centroid: "FFI/boundary/bridge/translation"
  status: ADVISORY (compiles)
  override: add #[reason_cluster_acknowledged(c-bridge-002, signature = "...")]  after sign-off
  see: docs/defections.md#2026-04-18, CLAUDE.md §Renames-to-refuse-on-sight
```

Build proceeds. The author now has information *that otherwise lived only in tribal memory*: this rationalization shape has been used eleven times before. The decision to use it again is on the record, not a fresh mistake.

### Author workflow — banlist hit (hard error)

```shape
#[exhume(crates::shape_value::value_word, reason = "needed for this one edge case, small fallback at the FFI boundary")]
fn migrate_legacy_snapshot(bytes: &[u8]) -> Result<KindedSlot, Error> { ... }
```

Compile output:

```
error[E-RFC006-banlist]: reason text matches forbidden-rationalization regex
  --> snapshot_compat.shape:14
   |
14 | #[exhume(..., reason = "needed for this one edge case, small fallback at the FFI boundary")]
   |                                       ^^^^^^^^^^                    ^^^^^^^^^^^^^^^^^^^^^^
   |
   = matched: /\bedge case\b/i  ← CLAUDE.md §Forbidden-rationalizations #5: "Document it as out-of-scope"
   = matched: /\bsmall fallback\b/i ← CLAUDE.md §Forbidden-rationalizations #1: "Just a small fallback"
   = these phrases have been used as cover for the deleted ValueWord dispatch path
   = log to docs/defections.md if you have a non-rotted reason; surface to the maintainer otherwise
```

No build. Not blockable by a `#[reason_cluster_acknowledged]` override — banlist hits are unconditional. Authors who genuinely need an escape and find their phrasing matches the banlist are required to *rewrite* the reason in a way that *does not* match. This is the load-bearing constraint: a reason that sounds like a rationalization gets refused, exactly as CLAUDE.md §"Renames to refuse on sight" already requires by hand.

### `shape reasons cluster` REPL session

```
$ shape reasons cluster
Reading .shape/reason-corpus.jsonl … 1,847 entries (47 days)
Embedding model: potion-retrieval-32M (sha256:…, frozen 2026-04-30)
DBSCAN (ε=0.15, minPts=3) … 12 clusters, 1,124 cluster-assigned, 723 noise

  c-bridge-002       size=87   centroid: "FFI / boundary / bridge / translation / decode"
  c-temp-fix-001     size=42   centroid: "temporary / fix later / TODO / works around"
  c-perf-only-003    size=31   centroid: "perf-critical / hot path / measured slowdown"
  c-experimental-004 size=18   centroid: "experimental / behind a flag / spike"
  ... 8 more

$ shape reasons audit
Quarterly reason audit — 2026-Q2
  Total escapes: 1,847 (Q1: 1,203, +53.5%)
  Cluster c-bridge-002 (size 87): RECOMMEND PROMOTE TO BANLIST
  Cluster c-temp-fix-001 (size 42): RECOMMEND PROMOTE TO BANLIST
  Banlist hits this quarter: 134 (rejected at compile time)
  Cluster-acknowledged overrides: 6 (Ed25519-signed)
  Useless suppressions detected: 38
```

The promote-to-banlist recommendation is the *positive feedback loop*: clusters that grow past threshold are reviewed by the maintainer; rationalizations that survive review get added to the banlist by name. The banlist co-evolves with the codebase instead of decaying behind it.

## Reference-level explanation

### Corpus storage

`.shape/reason-corpus.jsonl` — one JSON object per line, append-only, committed. Fields: `ts` (ISO-8601 UTC), `directive` (`distinct_from`|`fork_from`|`orthogonal`|`exhume`|`replaces`), `target_hash` (RFC-001 content hash), `site` (file:line), `reason_text`, `author`, `commit`, `embedding` (256-dim f32), `embedding_model` (model name + SHA-256).

Append-only because (a) the corpus is a *log*, not a database — rewriting loses the thing being measured; (b) merge conflicts vanish, per the empirical N7+N9 concurrent cluster-close precedent (2026-05-07) recorded in `docs/defections.md`; (c) verbatim history is the entire point.

### Embedding model: `potion-retrieval-32M`

- **Architecture.** Tokenize → look up token embeddings from a 32K-token static table → mean-pool. No transformer forward pass at inference time. (Model2Vec distillation: sentence-transformer → PCA + Zipf weighting; potion-retrieval-32M is additionally fine-tuned via Tokenlearn.)
- **Size / perf.** ~32 MB table (32K × 256 × f32). MinishLab MTEB (Jan 2025): 49.76 avg, retrieval 36.35 (86.65% of `all-MiniLM-L6-v2`), STS 73.22 — STS is exactly our task family. Sub-millisecond per embedding, single core, no GPU.
- **Determinism.** Tokenize + table-lookup + mean-pool; no FP non-determinism beyond IEEE-754 sum-order (canonicalised by sorting tokens before pooling); no hardware drift x86/ARM/RISC-V. The lookup table ships as a fixed binary blob with `shape-cli`; its SHA-256 is recorded in every corpus row's `embedding_model` field, so cross-machine comparison is sound by construction.

Model bumps are Shape-toolchain version bumps (see §Unresolved). Existing rows keep their original embedding + tag; new rows use the new model; cross-version queries are rejected (surface-and-stop). `shape reasons reindex --to potion-retrieval-32M@<new-sha>` is the explicit migration.

### Why static, not transformer

Load-bearing design choice. A transformer-based encoder (`all-MiniLM-L6-v2`, StarEncoder) would give ~13% more retrieval accuracy. Rejected for four reasons:

1. **`#[reason_cluster_acknowledged]` is Ed25519-signed.** The signature commits to the cluster id, which is a function of (corpus, embedding model, DBSCAN parameters). FP non-determinism between machines — GPU operator-fusion order, MKL vs OpenBLAS sum-order, ARM vs x86 SIMD — would silently drift cluster ids and invalidate signatures across the team. Static lookup + integer-deterministic pooling sidesteps the class entirely.
2. **Reproducibility of the audit trail.** When auditing in 2029 why `c-bridge-002` was acknowledged, the audit must be re-runnable. A transformer published in 2026 may not be loadable in 2029. A 32 MB lookup table with recorded SHA-256 is.
3. **CI cost.** The check runs on every PR. A transformer forward pass per escape across a high-PR repo is non-trivial GPU spend; static lookup is ~free.
4. **Adequacy.** The signal needed is "near a known rotted-reason cluster?", not "what is the precise semantics?". The `CLAUDE.md` regex `(decode|tag|kind|...) (bridge|probe|helper|hop|...)` proves *surface-token co-occurrence* suffices. Static embeddings give the same signal with smoother edges ("translation layer" ≈ "bridge", "edge case" ≈ "one-off") without paying for unused contextual modeling.

### Compiler hook

`crates/shape-runtime/src/annotation_context.rs` already exposes `on_define(fn, ctx)`. Escape directives use the existing RFC-001 `attribute_directive` grammar (no parser changes). At `on_define`: (1) read `reason`; (2) run banlist regex — hit = compile error; (3) embed via Model2Vec, query corpus for neighbours above τ_reason = 0.88 — match = warning with cluster summary; (4) DBSCAN incremental update — if the point joins a cluster of size ≥ 5 unacknowledged, compile error; (5) append the row to `.shape/reason-corpus.jsonl`.

Append happens *after* gating — banlist or cluster-recurrence rejections do not pollute the corpus.

### Clustering: DBSCAN(ε = 0.15, minPts = 3)

Cosine distance on L2-normalised embeddings. ε = 0.15 calibrated on `docs/defections.md`: groups W-series rationalizations into one cluster, separates them from "perf-critical" and "experimental flag" clusters; tunable per-repo via `.shape/reasons.toml`. minPts = 3 forms a cluster at three co-occurrences; below this, points are noise (advisory only). At ≥ 5 cluster members, escalates to error. Incremental updates per-PR via HNSW (`vicinity` / MinishLab `semhash` stack); nightly re-cluster from scratch. **τ_reason = 0.88** (cosine, = 1 − distance 0.12) is the advisory threshold — slightly tighter than ε so the warning fires *before* a point joins the cluster, giving the author a chance to rewrite before contributing to recurrence.

### Banlist (Phase 1 — the literal lift)

`docs/check-reasons-banlist.txt` is modelled byte-for-byte on `docs/check-no-dynamic-baseline.txt`: per-pattern monotonic-non-increasing count, scanned by `scripts/check-reasons.sh`. Initial seed is the literal text of `CLAUDE.md` §"Forbidden rationalizations":

```
# Phase 1 banlist — literal lift. Case-insensitive regex; hit = hard error.

\bsmall fallback\b
\bonly for serialization\b
\bfollow.?up for (a later|the next) phase\b
\bsoft.?fail counter for now\b
\bout.of.scope\b
\bfeature flag (we can|we will|to) toggle\b
\brename to a less suspicious\b
\bopcode for this (specific|one) conversion\b
\bone decode at the boundary\b

# Broader-family (§Renames-to-refuse-on-sight, 2026-05-09 ruling):
\b(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture)\s+(bridge|probe|helper|hop|translator|adapter|shim)\b

# Catch-all rot terms (lower precision; refined by Phase 2 cluster signal):
\bedge case\b|\btypo\b|\bworks around\b|\bfor now\b|\btemp\b|\bTODO\b|\bFIXME\b
```

Phase 1 alone is strictly better than today: the list currently lives in `CLAUDE.md` as prose humans must remember. `scripts/check-reasons.sh` is the byte-for-byte twin of `scripts/check-no-dynamic.sh`; `just check-reasons` parallels `just check-no-dynamic`; both wire into `scripts/verify-merge.sh` as CHECK 13.

### `#[reason_cluster_acknowledged]` sigil — RFC-001 grammar

Reserved in `DirectiveKind`. Form:

```shape
#[reason_cluster_acknowledged(cluster_id = "c-bridge-002", signature = "0x4a8…")]
#[exhume(crates::shape_value::value_word, reason = "snapshot v0 backwards-compat per ADR-009 §4")]
fn migrate_v0_snapshot(bytes: &[u8]) -> Result<KindedSlot, Error> { ... }
```

- `signature` is Ed25519 over `(cluster_id, target_hash, author, ts)` by a key in `.shape/reason-sign-off-keys.toml`. Reuses `crates/shape-runtime/src/crypto/signing.rs` (`ModuleSignatureData`) — no new crypto path.
- Signing keys (typically 2–3) are themselves signed by the project root key.
- The signature commits to cluster id + target hash, so a later cluster renumbering or target-hash change invalidates the sigil and the author must re-acknowledge. This is the "useless suppression detected" feedback loop (cf. Rust `#[expect]`).
- Banlist hits are **not** overridable. The override is for the cluster gate only.

### CLI surface

`shape reasons cluster` (list clusters), `shape reasons history <id>` (escapes in cluster), `shape reasons nearest <text>` (pre-commit check), `shape reasons audit` (quarterly report), `shape reasons reindex --to <model@sha>` (explicit model-bump migration).

### Quantitative mapping: CLAUDE.md → corpus

| `CLAUDE.md` §Forbidden-rationalizations / §Renames-to-refuse-on-sight | Banlist regex | Estimated Phase-2 corpus seeding (from `docs/defections.md`) |
|---|---|---|
| "small fallback for this one edge case" | `\bsmall fallback\b`, `\bedge case\b` | 4 W-series + 6 cross-cluster |
| "Keep ValueWord but only for serialization" | `\bonly for serialization\b` | 2 N7-era + 1 N9 |
| "Mark this as a follow-up for a later phase" | `\bfollow.?up for…\b` | 7 across W/N/Phase-2d |
| "ValueBits shim / FFI-boundary bridge" cluster | broader-family regex | 11+ W-series instances → seeds `c-bridge-002` directly |
| "tag-decode bridge" sub-family | broader-family regex | 9 W-series instances |
| "MethodFnV2 bridge / dispatch-slice probe" | broader-family regex | 4 ADR-006 §2.7.10 instances |
| "value-call bridge / capture-injection adapter" | broader-family regex | 5 ADR-006 §2.7.11 instances |

The right-hand column is the *initial corpus*: ~15 high-signal rows from W-series alone, plus another ~15 from N-series and Phase-2d defections, plus the 9 literal phrases serving as their own pseudo-rows for cluster-centroid stability. Total seed corpus: ~40 rows, sufficient for DBSCAN to immediately form the `c-bridge-002`, `c-temp-fix-001`, and `c-experimental-004` clusters with cluster-size escalation thresholds met for at least `c-bridge-002` at day zero.

## Drawbacks

- **False-positive collisions.** A legitimate `#[orthogonal]` with the word "boundary" may cosine-match the bridge cluster. Mitigation: `τ_reason = 0.88` fires only on real similarity; cluster-recurrence escalation requires *five* matches before becoming a hard error; the override sigil is one line.
- **Authors learn to write evasive reasons.** "FFI-boundary bridge" → "interface-edge translator" (the `CLAUDE.md` regex was widened on 2026-05-09 precisely because of this). Mitigation: clusters are computed in embedding space, not regex space, so semantic neighbours are still caught; the quarterly audit surfaces cluster *growth* so a new euphemism colonising the same neighbourhood is visible to the human-in-the-loop; the banlist gains entries as the audit recommends. Co-evolution, not stalemate.
- **Model bump invalidates clusters and signatures.** Mitigation: `shape reasons reindex` is an explicit, audit-trailed migration; cross-version queries are rejected; model version is pinned in toolchain release notes.
- **Banlist over-blocking.** A "this *is* genuinely one structurally exceptional case where we need a small fallback" reason gets rejected with no override path. By design: a reason in those words is bad evidence regardless of underlying judgement. Authors are required to *describe the structurally exceptional thing* rather than reach for the rationalization word; if the description still clusters with rotted reasons, the override sigil applies.
- **Long-running migrations can trip cluster escalation.** Mitigation: a migration declares `#[reason_cluster_acknowledged]` once with the strategic-owner key, reusing the cluster id across all of its `#[replaces]` directives.
- **JSONL files grow.** ~250 bytes × 1,847 projected rows = ~460 KB at end of year 1. Tractable; periodic compaction feasible but unnecessary.

## Rationale and alternatives

**Why static embeddings.** The audit-trail use of `#[reason_cluster_acknowledged]` is the deciding factor: the sigil signs a tuple including the cluster id; the cluster id is downstream of the embedding model; a non-deterministic embedding makes the signature non-portable across machines. Static lookup tables remove the variable entirely. (`docs/adr/006-value-and-memory-model.md` §2.7.7 is the in-tree precedent: parallel `Vec<u64>` data + `Vec<NativeKind>` kinds was chosen over packed tag bits *for the same reason* — determinism over compactness.)

**Why advisory before blocking.** Phase 2 advisory runs for one quarter before Phase 3 turns on escalation. This lets the corpus populate, lets the maintainer calibrate `ε` and `τ_reason` against actual repo content, and avoids the failure mode where a tight threshold on day one rejects half of legitimate escapes and the team disables the feature. The Phase 1 banlist *is* blocking from day one — those patterns we already know are bad.

**Why not just a stricter regex.** The `CLAUDE.md` regex has been widened twice (2026-05-09 broader-family, 2026-05-13 cluster-0 sub-family). Every widening was reactive to a defection the previous regex missed. Regex-only is structurally one step behind the rationalization frontier. Embedding-space clustering is one step ahead: a new rationalization that does not match any existing regex but *clusters* with prior rotted reasons is caught at first occurrence.

**Why not human-LLM-as-reviewer.** (a) Cost and latency at compile time; (b) non-determinism — same prompt, different verdict across model versions — collides with the audit-trail concern; (c) prompt-injection via reason text. LLM-driven reason-quality assessment is plausible as a separate async off-critical-path review (see §Future possibilities), not the compile-time gate.

## Prior art

- **TypeScript `@ts-expect-error`** (TS 3.9, 2020). Closest to the *useless-suppression-detection* mechanism: directive fails when the error it suppresses no longer fires. Same idea adopted via the `#[reason_cluster_acknowledged]` signature committing to the cluster id — when the cluster changes, the signature invalidates. Not a precedent for clustering, but for "the suppression mechanism must push back".
- **ESLint bulk suppressions** (TikTok 2024, upstreamed April 2025). Solves the *scale* problem — 70K suppressions moved out-of-line into a JSON sidecar. Adjacent to our corpus design (out-of-line, JSON, gitable). Not a precedent for clustering or reason-quality checking — pure volume management.
- **Rust `#[allow]` → `#[expect]`** (Rust 1.81, 2024). Same lesson as `@ts-expect-error`. Adopted; extended with reason-similarity clustering.
- **Conventional Commits enforcement** (`commitlint`, ~2017–). Mechanical gate on commit-message *shape*, not content. Demonstrates that lint-on-natural-language is feasible at scale; does not attempt similarity / clustering.
- **Sourcegraph Cody embedding-migration debt** (mid-2024 retro). Customers' embedding indexes went stale on model upgrade; migration cost non-trivial. Direct cost evidence for the model-bump concern; mitigated by per-row model SHA + explicit `shape reasons reindex`.
- **Model2Vec / potion** (MinishLab, late 2024–Jan 2025). The static-embedding approach used here. MTEB: 49.76 avg, 36.35 retrieval, 73.22 STS at 32 MB / sub-ms per embedding. `potion-retrieval-32M` is the strongest static retrieval model published as of Jan 2025.
- **MinishLab SemHash** (Jan 2025). Identical embedding stack, applied to *dataset deduplication* via Vicinity nearest-neighbour search. Our use is the dual: not "find near-duplicates and drop them", but "find near-duplicates and warn the author". Library is reusable.
- **`cargo audit` `audit.toml` `ignore = [{ id, reason }]`.** Closest existing system. Reason is free-text, ungrep'd, unclustered — pure documentation. Evidence that the *shape* (each escape carries a reason) is industry practice; not a precedent for treating reason as signal.
- **Karampatsis & Pradel, "An Empirical Study of Suppressed Static Analysis Warnings" (FSE 2025).** Empirical base rate: 50.8% of suppressions in long-lived codebases are *useless*. Direct evidence for the failure mode.

## Unresolved questions

- **Embedding model versioning policy.** Does a bump require a Shape minor or major bump? Default: minor (migration is mechanical, audit-trailed); revisit after first bump.
- **Per-repo clustering parameters.** ε and minPts are repo-dependent (10-person startup ≠ 200-person platform). Defaults in `.shape/reasons.toml`; tunable; calibrate after six months across multiple Shape-using repos.
- **Signature expiration.** Should `#[reason_cluster_acknowledged]` signatures expire (e.g. 18 months) and force re-ack? Expiration forces review (good) but churns stable legacy (bad). Default: no expiration; rely on cluster-id invalidation as natural trigger.
- **Cross-project reason federation.** Registry-hosted, opt-in via `shape.toml`. High value, real privacy concern (reason text may leak intent). Deferred to RFC-007 if demand.
- **Interaction with RFC-008.** Reason-similarity probably belongs as one input to RFC-008's channel calculus; integration boundary not yet specified.

## Future possibilities

- **LLM-driven reason quality assessment** as a separate async off-critical-path review (not the compile gate; see §Rationale). Nightly job flagging high-recurrence + low-LLM-quality reasons.
- **Federated reason corpora** via the Shape registry (see §Unresolved).
- **Reason similarity as RFC-008 channel input** — expose per-escape cluster id / similarity score as a channel signal.
- **Embedding-space visualizations** in the quarterly audit (UMAP / t-SNE) surfacing cluster topology.
- **shape-lsp inlay hints** showing cosine similarity to nearest cluster centroid as the author writes the reason.

## Phasing and cost

- **Phase 1 — Banlist alone (1 week).** `scripts/check-reasons.sh` (twin of `scripts/check-no-dynamic.sh`) + `docs/check-reasons-banlist.txt` seeded from `CLAUDE.md` §Forbidden-rationalizations. Wired into `just check-reasons` and `scripts/verify-merge.sh`. Strictly better than today: the banlist becomes mechanical instead of documentation.
- **Phase 2 — Corpus + embedding + advisory warnings (2 weeks).** Bundle `potion-retrieval-32M` (~32 MB) into `shape-cli`. Wire embedding into the existing `on_define(fn, ctx)` hook in `crates/shape-runtime/src/annotation_context.rs`. Implement `.shape/reason-corpus.jsonl` append + Vicinity / HNSW nearest-neighbour. Seed corpus from `docs/defections.md` W-series + N-series (~40 rows). Ship `shape reasons cluster|history|nearest`.
- **Phase 3 — Clustering escalation + Ed25519 sign-off + audit CLI (1 week).** DBSCAN incremental clustering. `#[reason_cluster_acknowledged]` grammar + Ed25519 verification via `crates/shape-runtime/src/crypto/signing.rs`. `shape reasons audit` quarterly report. Escalation threshold turned on after one quarter of advisory-mode calibration.

**Total: 3–4 weeks** against the 4–6 week W-series cleanup. Phase 1 alone — a single week — would have caught the original "FFI-boundary bridge" rename if any of the four canonical phrases ("small fallback", "edge case", "compatibility layer", "documented FFI-boundary helper") had been a regex at the time. They were all in `CLAUDE.md` as prose by the time the cleanup finished. This RFC closes the loop.

---

### Cross-references

- `CLAUDE.md` §"Forbidden Patterns" / §"Forbidden rationalizations" / §"Renames to refuse on sight" — proof-of-concept; Phase 1 is the literal lift.
- `docs/defections.md` — Phase 2 corpus seed (28+ dated instances).
- `scripts/check-no-dynamic.sh` + `docs/check-no-dynamic-baseline.txt` — template for `scripts/check-reasons.sh` + `docs/check-reasons-banlist.txt`.
- `scripts/verify-merge.sh` — adds CHECK 13.
- `crates/shape-runtime/src/annotation_context.rs` — `on_define(fn, ctx)` compiler hook.
- `crates/shape-runtime/src/crypto/signing.rs` — `ModuleSignatureData` reuse for the sigil.
- `docs/adr/006-value-and-memory-model.md` §2.7.7 — determinism-over-compactness precedent.
- RFC-001 (`attribute_directive` + escape directives), RFC-004 (`#[replaces]`), RFC-008 (channel calculus, future).
