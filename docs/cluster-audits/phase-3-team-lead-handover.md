# Phase 3 cluster-0 — Team-lead handover (R19-complete → R20 seam)

**Generated:** 2026-05-14.
**Successor handoff point:** Round 19 complete (C + S1.5 + S2 audit-only all merged into bulldozer; supervisor's R19 partial dispositions ratified; S2-prime dispatch is the new team-lead's first action). The predecessor team-lead session completed the R19 merge ceremony before rotation (didn't rotate mid-flight to avoid killing in-flight S1.5 subagent).
**Predecessor team-lead session:** rolled at session-close after R19 merge ceremony completed; this is the continuation.
**Supervisor session:** see `docs/cluster-audits/phase-3-supervisor-handover.md` for supervisor-side state.

## Your role this session

You are the **team lead** for Phase 3 cluster-0 of the Shape language refactor. Job:

1. Operate the `Agent` tool to dispatch sub-cluster agents per supervisor relays.
2. Verify close gates (`cargo check --workspace --lib --tests`, `bash scripts/verify-merge.sh`, `bash scripts/check-no-dynamic.sh`, AGENTS.md row).
3. Merge sub-cluster branches into `bulldozer-strictly-typed`. Take-both merge resolution for AGENTS.md row collisions + dispatch-table arm-line conflicts is the established pattern.
4. Run smoke-matrix verification under both `--mode vm` and `--mode jit` after every round closes.
5. Update `docs/cluster-audits/phase-3-cluster-0-status.md` after each round + supplementary -ext smoke disposition tracking.
6. Surface architectural questions to the supervisor via the user (strategic-owner relays).

The supervisor is a separate Claude instance — the user copies their relays into your session, you copy your responses back to the user, the user pastes them to the supervisor. Do **not** make architectural calls yourself (ADR amendments, cluster scoping, defection-pattern refusals, partial-close authorization, cluster-tag authorization); surface them to the supervisor with structured context.

## First action — read these in order

1. **`docs/cluster-audits/phase-3-cluster-0-status.md`** — canonical state. Read first.
2. **`docs/cluster-audits/phase-3-kickoff-prompt.md`** — original supervisor contract; cluster-0 close criterion at lines 96-100 (the 4 kickoff smokes).
3. **`docs/cluster-audits/phase-2d-handover.md` §0** — discipline rules (forbidden patterns, 4-table lockstep, 5-arm receiver-recovery, surface-and-stop discipline).
4. **`CLAUDE.md`** — Forbidden Patterns + Renames to refuse on sight (including the §Parallel-implementation-across-producer/consumer-carrier-shape-boundaries entry landed at e55b8e71) + Single-discriminator (ADR-005) + Value & memory model (ADR-006) + Mechanical enforcement.
5. **`AGENTS.md`** — live roster (long file; relevant rows are R5-R19 entries at the bottom).
6. **`docs/adr/006-value-and-memory-model.md`** §2.3 / §2.7.5 / §2.7.14 / §2.7.22 / §2.7.24 / §2.7.27 — the cluster-0-load-bearing sections.
7. **`docs/cluster-audits/w12-typed-array-data-deletion-audit.md`** — Round 17 cluster-0-transition audit; defines the migration plan (S1-S5 sub-clusters).
8. **This file's "Pre-handover update" section at bottom** — last-minute state from the rolling team-lead session, including the supervisor's R19-partial dispositions + S2-prime dispatch shape.

Post a 1-line confirmation: *"Read 8 mandatory docs; team-lead role ready. Current state: <one sentence>."*

## Current state at handover

**Branch HEAD:** `bulldozer-strictly-typed @ 214e8661` — reflects S1.5 + C + S2 audit-only all merged. R19 merge ceremony completed by predecessor before session rotation. (Predecessor merge order: S1.5 first `5346ca5c`, then C with take-both AGENTS.md `7de3b1d6`, then S2 audit-only auto-merge `214e8661`. Post-R19-merge `verify-merge.sh` 12/12 PASS via devenv.)

**Smoke matrix (canonical kickoff, post-R19-complete):**

| Smoke | VM | JIT | Disposition |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | passing |
| 2 canonical (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ✅ 30 | ❌ dual-carrier blocker | resolves post-S5 (R20) |
| 3 (trait `dyn T` + `t.name()`) | ✅ "x" | ⚠ None (was SIGSEGV pre-C; β filter intercepts; γ upstream gap remains) | γ R20 sub-cluster unblocks |
| 4 (Set + .add + .size) | ✅ 2 | ✅ 2 | passing |

**Smoke matrix verified post-R19-merge** (direct run via `cargo run --bin shape -- run /tmp/smokes/sN.shape --mode {vm,jit}` through devenv): Smoke 1 VM=4950 / JIT=4950 ✓; Smoke 2 canonical (`[1,2,3,4,5].map(\|x\|x*2).sum()`) VM=30 ✓ / JIT pre-existing R14 conduit blocker (unchanged from baseline; resolves post-S5); Smoke 3 VM=`x` ✓ / JIT=`None` (β filter intercepts SIGSEGV per C close report; γ workstream unblocks `x` matching VM); Smoke 4 (`Set()`-syntax) VM=2 / JIT=2 ✓ (per W17-narrow R15 + S1 reopen prior verification). S1.5's F32 + Char additions introduced no smoke-matrix delta.

**Cumulative through R19 complete:** ~33+ sub-clusters across 19 rounds, ~3 / session steady cadence.

**R19 merged sub-clusters (all in predecessor's session before rotation):**
- **C** (W17-narrow-follow-up-B-β) at 9bf2cf35 — RATIFIED + merged. β filter intercepts TAG_NULL at print_kinded_inner; SIGSEGV → None for Smoke 3 JIT. γ upstream gap named as R20 sub-cluster.
- **S1.5** (W12-nativekind-scalar-additions) at close commit `80d8c485`, merge commit `5346ca5c` — RATIFIED + merged. F32 + Char NativeKind/ConcreteType scalar additions per Shape D ratification (U64 deferred post-S5); ADR-006 §2.7.5 amendment text inline; audit doc Char-bucket clarification inline (§2.1/§2.2/§3.1/§3.2 — Char removed from heap-element bucket per supervisor R19 (3) ratification). 26 files / +522/-27 net; cascade landed at ~22 sites (well under ~100-site surface-and-stop ceiling); cascade-surface-and-stop NOT triggered. Cross-tier compat: `KindedSlot::as_char` dual-label match recognizes both `NativeKind::Char` (new) AND `NativeKind::Ptr(HeapKind::Char)` (pre-amendment) while 32 unmigrated `Ptr(HeapKind::Char)` consumer sites remain (cluster-1 hardening territory).
- **S2** (W12-typed-array-data-heap-element-migration) at 1bf8dbd — audit-only merged for doc-record. Surface-and-stop with 3 structural obstacles dispositioned by supervisor: Q25.A SUPERSEDED amendment (1b); TemporalData 7-variant split audit-first in S2-prime; BigInt defer.

**R20 dispatch (your first action):**
- **S2-prime** — W12-typed-array-data-heap-element-migration REOPEN with Q25.A SUPERSEDED amendment scope. See §In-flight state + §Pre-handover update for the dispatch shape.
- **S5** — TypedArrayData enum deletion + Q25.A SUPERSEDED amendment commit. Dispatches after S2-prime completes.
- **γ** — W12-jit-trait-impl-method-registry (UFCS lookup gap when receiver is `dyn T`). Dispatches in R20 alongside S5 or in parallel with S2-prime per supervisor authorization.
- **U64 relabel-step** (Shape D) — fold into S5 OR separate S6 sub-cluster per supervisor's R20 dispatch authorization.

**Velocity:** total handoff-to-v1 ~17-23 sessions. Cluster-0 close projected at R20+1.

## Discipline rules (load-bearing — refuse on sight)

Read CLAUDE.md for full lists. The supervisor refuses these framings at the relay layer; you should refuse them at the dispatch + close-report review layer:

1. **Partial-close / declare-victory at any artifact-tagging layer.** Refused at Rounds 5/6/7/8 ("phase-3-cluster-0-partial-close"). Same shape, same refusal.
2. **"Pre-existing" as a disposition.** MEMORY.md: "Own all code quality. Never blame 'pre-existing' issues; all code is the agent's responsibility once they touch the area." If an agent's close report uses "pre-existing" to justify leaving a forbidden pattern alive, reopen via SendMessage (W11-round-1 precedent).
3. **Bool-default for unknown kind.** §2.7.7 / §2.7.8 #4 — `NotImplemented(SURFACE: …)` with §-cite, not `kind: NativeKind::Bool`.
4. **"Bridge/probe/helper/hop/translator/adapter/shim" framings** — CLAUDE.md broader-family rule.
5. **Kind-blind dispatch / NaN-box decode at FFI boundaries** — §2.7.5 stamp-at-compile-time discipline.
6. **Silent fallback / no-op with "tracked as follow-up" framing** — W11-round-1 walk-back precedent.
7. **All "Forbidden rationalizations"** in CLAUDE.md (just one edge case / mark as follow-up / soft-fail counter / document as out-of-scope / rename to less suspicious / add a feature flag / new opcode for this specific conversion / just one decode at the boundary).
8. **Resurrecting deleted shape under renamed alias** — CLAUDE.md lists.
9. **Parallel-implementation across producer/consumer carrier-shape boundaries** (landed at e55b8e71 R17 close as CLAUDE.md entry). 8+ instances observed cluster-0; pattern is real. Refuse "documented intentional duality" / "preserve both carriers" / "two solutions for two different problems" framings without explicit ADR amendment naming the duality + compile-time classification rule.

When you spot one in an agent's close report: **do not merge.** Surface to supervisor with structured shape (file:line, framing quote, applicable CLAUDE.md rule cite). Supervisor returns SendMessage-reopen relay or re-dispatch decision.

## Dispatch cadence + close-gate shape

**Sub-cluster dispatch prompt template** (mirrors `phase-3-kickoff-prompt.md`):

1. **5 mandatory docs first** (phase-2d-handover.md §0, phase-2d-close-summary.md, CLAUDE.md Forbidden Patterns + Renames to refuse on sight, ADR-006 §2.7.5 / §2.7.14 / §2.7.27 as relevant, phase-3-cluster-0-status.md).
2. **Territory** — explicit file paths, audit-first if scope uncertain.
3. **Smoke target** (kickoff smoke being unblocked) — or close-gate proof if non-smoke audit/migration.
4. **Audit deliverables** when audit-first (file:line cites required, §-cite verification, ADR-fit confirmation, scope inventory).
5. **Close gate:** `cargo check --workspace --lib --tests` exit 0 (verified via EXIT CODE, not grep) + `bash scripts/verify-merge.sh > /tmp/vm.out 2>&1; echo SCRIPT_EXIT=$?` 12/12 PASS (CHECK-COMMS-1 file-redirect pattern; pipe-tail measurement bug) + `bash scripts/check-no-dynamic.sh` exit 0 + AGENTS.md row appended.
6. **Refuse-on-sight discipline** (the 9 items above, named explicitly in the dispatch prompt).
7. **No `Co-Authored-By: Claude` trailer** (MEMORY.md user rule).

**Merge resolution:** take-both on AGENTS.md row collisions + dispatch-table arm-line conflicts. Take-HEAD on test-attribute conflicts where one branch has more detailed §-cites. After any take-both pass: `cargo check --workspace --lib` MUST pass before commit.

**Verify-merge.sh measurement:** always file-redirect for exit capture per CHECK-COMMS-1.

**Smoke matrix re-verification:** after every round merges, run all 4 kickoff smokes under both modes. Update `phase-3-cluster-0-status.md`.

**Manual worktree creation** (avoid Agent `isolation:` parameter — known defect surfaced at R15 W17):

```
git -C /home/dev/dev/shape-lang/shape worktree add \
  /home/dev/dev/shape-lang/shape-<slug> \
  -b bulldozer-strictly-typed-<slug> <base-commit>
```

Run cargo / verify-merge.sh via `devenv shell --quiet -- bash -c "cd <worktree-path> && <command>"` from `/home/dev/dev/shape-lang/`. See `reference_phase2d_devenv.md` in supervisor's auto-loaded memory for the canonical invocation pattern (devenv is the dev-environment provider; the project does NOT use a direct `flake.nix`).

## In-flight state at handover

R19 merge ceremony was completed by predecessor team-lead before session rotation (deliberate — couldn't cancel in-flight S1.5 subagent without killing it; waited for S1.5 close + merges + status doc update + then rotated). You inherit a post-R19-complete bulldozer state.

**Immediate next actions (in order):**

1. **Dispatch S2-prime** (W12-typed-array-data-heap-element-migration REOPEN) per supervisor's R19 partial disposition. See §Pre-handover update for the dispatch shape, the 3 obstacles' resolutions (Q25.A SUPERSEDED amendment scope, TemporalData split audit-first, BigInt defer), and the Q25.A SUPERSEDED amendment text shape that S2-prime's agent verifies-and-refines.

2. **After S2-prime closes:** verify migration shape clean (no defection-attractor framings; no remaining Q25.A specialized-variants-inside-enum producers); merge + status doc update; surface to supervisor for R20 authorization.

3. **R20 dispatch** (supervisor authorizes after S2-prime closes):
   - **S5** — W12-typed-array-data-enum-deletion + ADR-006 §2.7.24 Q25.A SUPERSEDED amendment commit.
   - **γ** — W12-jit-trait-impl-method-registry. UFCS lookup gap when receiver is dyn T.
   - **U64 relabel-step (Shape D)** — fold or S6 per supervisor's R20 disposition.

4. **Cluster-0 close attempt** after R20 merges + full kickoff smoke matrix passes VM == JIT. Cluster-0 close report → supervisor ratifies → user authorizes `phase-3-cluster-0-close` tag.

**Pending items from predecessor handover (verify status):**
- Char audit-doc bucket clarification (move from §2.2/§3.2 heap-element to §2.1/§3.1 scalar): should have landed alongside S1.5 close commit per supervisor disposition. Verify in audit doc + status doc; flag if not landed.
- Smoke 4 kickoff-prompt typo (`HashSet()` → `Set()` in `phase-3-kickoff-prompt.md`): should have landed alongside R19 status doc update. Verify; flag if not.
- Team-lead handover-doc annotation re: supervisor-layer imprecision pattern (3 documented instances): should have landed in this handover doc's §Discipline-pattern observations. Verify (already present in this version).

## Decision authority pattern

You ARE authorized to:
- Run inline cite-audit before dispatch (Q1/Q2 precedent — kickoff smoke text verification, §-cite verification against ADR-006 / phase-2d-hardening.md).
- Propose round structure (parallel vs sequential, integrated trinity vs split, audit-first vs production-first) — supervisor ratifies.
- Coordinate AGENTS.md row updates + merge order + take-both resolution.
- Run reopen via SendMessage on a closed-but-not-merged sub-cluster when an audit gap is small + recoverable (W11-round-1 precedent).
- Refuse a sub-cluster's close report at merge time if it harbors a forbidden pattern (then surface to supervisor for reopen vs re-dispatch decision).
- Complete ceremony for agent-API-error WIP that's verifiably correct (S1 reopen R18 precedent: gates + AGENTS.md + commit + smoke matrix, with explicit commit-message attribution to the agent for substantive work). Each instance requires supervisor authorization until a durable pattern is established.
- Update `phase-3-cluster-0-status.md` + `AGENTS.md`.

You are NOT authorized without explicit supervisor approval:
- Dispatch new sub-cluster agents (supervisor ratifies dispatch scope + audit-first vs production cadence).
- Refuse defection-pattern framings on the agent's behalf at the meta-architectural level — refusing is the supervisor's call; you flag + surface.
- Authorize ADR amendments.
- Re-scope cluster boundaries (cluster-0 → cluster-1 reclassification, kickoff matrix changes, close criterion modifications).
- Tag `phase-3-cluster-0-close` (user authorizes after supervisor ratifies).
- **Land `CLAUDE.md` modifications in any commit (agent dispatch directive OR team-lead direct edit) without explicit user ratification of *the landing*.** Supervisor drafting text != pre-authorization. The chain is: supervisor drafts → user ratifies the text AND the landing → team lead folds the landing directive into the next dispatch / commits the change. R17 (2026-05-13) was the one-time learning instance — supervisor drafted text, user said "authorized text for CLAUDE.md", team lead folded landing into the deletion-audit agent's dispatch without waiting for supervisor's planned next-relay formalization. Strategic owner retroactively ratified per Option (a). Going forward: text + landing both require explicit user authorization.

## Discipline-pattern observations (carry forward)

**Supervisor-layer + audit-layer imprecision pattern** (3 instances cluster-0):
1. R16 §2.7.14-A draft — supervisor's "unwrap-and-flatten" framing repeated Round 14 conflation. Caught by W12-Option-B-reframed agent.
2. R18 S1 reopen SendMessage — supervisor's "Array<u64> fails at compile-time" was imprecise (legacy fallback ≠ compile-time rejection). Caught by S1 reopen agent.
3. R19 S1.5 audit Shape B framing — "runtime element-kind from HeapHeader byte at FFI dispatch shell" had §2.7.5 boundary risk. Caught by supervisor; resolved via Shape D deferral.

Pattern: when audits or supervisor relays propose "minimal extension via runtime tag-byte read at FFI dispatch shell" or equivalent framings, that's the §2.7.5 stamp-at-compile-time boundary; require explicit clarification of which layer reads the byte (compile-time call signature vs runtime FFI dispatch) before ratifying. The discipline check is robust enough to catch these at multiple layers.

**Stash-then-rebuild + structured-surfacing pattern** (W17-narrow R15 precedent; reused R18 S1 reopen + R19 C): when a sub-cluster's own contract is verifiably met but the smoke target fails due to a surfaced upstream gap, the discipline-compliant disposition is (a) verify own contract clean, (b) verify upstream gap is pre-existing (via stash-then-rebuild or detached-HEAD check), (c) structured-surface the upstream gap as a new sub-cluster (not "follow-up to ignore"). Sub-cluster merges with smoke gate unmet; new sub-cluster dispatched separately.

**Agent API-error recovery pattern** (S1 reopen R18 precedent): when sub-agent API-errors mid-work with WIP uncommitted, three recovery options:
1. SendMessage retry — first-line response; transient API errors often recover.
2. Team-lead completes ceremony for verified-correct agent WIP — requires supervisor authorization per instance; guardrails per the §Decision authority pattern; commit message attributes substantive work to agent + ceremony to team-lead.
3. Re-dispatch fresh agent — conservative-wasteful; only when (1)/(2) aren't viable.

**Cross-tier compat pattern during NativeKind variant additions** (S1.5 R19 precedent): when adding a new NativeKind variant that replaces a pre-amendment label (e.g. `NativeKind::Char` replacing `NativeKind::Ptr(HeapKind::Char)`), the accessor/dispatch sites need a dual-label match during the migration window — both old and new labels recognized at the consumer layer while producer sites migrate incrementally. After consumer-site migration completes (typically as a follow-up cluster-1 hardening item), the dual-label match collapses to single-label. S1.5's `KindedSlot::as_char` (recognizing both `NativeKind::Char` new variant AND `NativeKind::Ptr(HeapKind::Char)` pre-amendment) is the model. 32 unmigrated `Ptr(HeapKind::Char)` consumer sites tracked as cluster-1 territory.

## User preferences + working style

- **No `Co-Authored-By: Claude` trailer in commits.** MEMORY.md rule.
- **Own all code quality.** Never frame as "pre-existing" — all code is the agent's responsibility once touched.
- **Plain code fences for relay text**, not blockquotes. The user copies relay blocks verbatim; blockquote `>` prefixes break paste.
- **Direct, concise communication.** Tight responses; substantive when needed; no padding.
- **Strategic owner / language designer.** Delegates architectural calls to the supervisor. Will surface explicitly on language-design or project-scope decisions.
- **Working in agent velocity.** Multi-sub-cluster-per-session cadence is expected.

## Operational continuity

1. **First action** after reading the 8 docs: post the 1-line confirmation + execute the immediate-next-actions queue from §In-flight state.
2. **Standard interaction pattern**: agent closes → you verify gate + read close report → you draft consolidated status → user relays to supervisor → supervisor responds → user pastes back → you execute.
3. **Don't re-derive context** that's already in `phase-3-cluster-0-status.md`, `CLAUDE.md`, ADR-006, the audit docs, or this handover.
4. **The supervisor session rotated independently** at the R19 dispatch seam; new supervisor session active per `docs/cluster-audits/phase-3-supervisor-handover.md`. You communicate via the user as always; the supervisor rotation is transparent at your layer.

**Most-likely-next-action:** dispatch S2-prime per supervisor's R19 partial disposition (in §Pre-handover update). After S2-prime closes + merges + smoke matrix re-verification + status doc update, surface to supervisor for R20 dispatch authorization (S5 + γ + U64 relabel).

---

## Pre-handover update (filled in 2026-05-14)

**R19 partial state at handover:**

- **C — W17-narrow-follow-up-B-β** at `9bf2cf35`: RATIFIED for merge by supervisor. β filter contract met (TAG_NULL filter at print_kinded_inner; DRY; +37 LoC). Smoke 3 JIT improved SIGSEGV → None (real architectural improvement). γ upstream gap named as R20 sub-cluster W12-jit-trait-impl-method-registry (UFCS lookup gap when receiver is dyn T = X{}). Merge per standard ceremony.

- **S2 — W12-typed-array-data-heap-element-migration** at `1bf8dbd`: surface-and-stop with 3 structural obstacles. Supervisor dispositioned REOPEN as S2-prime. See dispatch shape below.

- **S1.5 — W12-nativekind-scalar-additions**: still running at handover. Close report pending. Cascade-surface-and-stop fallback at ~100 sites is the supervisor-ratified contingency (F32 spills to S1.6 if cascade exceeds bound).

**Supervisor R19 partial dispositions:**

**Obstacle 1 (§2.2 vs §2.7.24 Q25.A architectural conflict):** Resolution (1b) — stand by R17 cluster-0-transition deletion authorization; Q25.A SUPERSEDED. Refused (1a) as walk-back of strategic-owner authorization; refused (1c) as "preserve fallback for one period" defection (Forbidden rationalization #4 + §Parallel-implementation entry). ADR amendment shape drafted by supervisor (below); S2-prime agent verifies-and-refines against actual ADR + code.

ADR-006 §2.7.24 Q25.A SUPERSEDED amendment text (draft shape; agent refines):

```
§2.7.24 Q25.A SUPERSEDED by Round 17 cluster-0-transition deletion target
(strategic-owner authorization 2026-05-13).

Phase 2d Q25.A monomorphized the polymorphic catch-all (TypedArrayData::HeapValue)
into per-built-in-heap-type specialized variants kept inside the TypedArrayData
enum. This was a transitional architectural state.

Cluster-0 S5 deletes the TypedArrayData enum entirely. Heap-element variants
migrate to TypedArray<*const <X>Obj> per the R17 deletion audit §2.2 carrier
pattern (carrier OUTSIDE the enum; per-T monomorphization via <X>Obj newtypes).
The specialized variants Q25.A introduced are part of the deletion target.

Forbidden post-supersession: "documented intentional duality" framings
preserving specialized-variants-inside-enum carriers alongside v2-raw
TypedArray<*const <X>Obj> carriers; "preserve specialized variants for cluster-0
close, migrate post" framings (the deletion authorization stands for cluster-0
itself).
```

**Obstacle 2 (TemporalData 7-variant split):** audit-first inside S2-prime. Verify which variants are user-facing (Array<X> reachable in user code → need migration to TypedArray<*const <X>Obj>) vs AST-internal (comptime/query-DSL/etc. → stay legacy Arc<TemporalData>). Likely outcome: DateTime/Duration/TimeSpan are user-facing (3 newtype <X>Obj carriers); Timeframe/TimeReference/DateTimeExpr/DataDateTimeRef are AST-internal (stay legacy).

**Obstacle 3 (BigInt doesn't exist at HEAD):** defer. No kickoff smoke depends on BigInt; no producer / no consumer / no migration needed. Q25.A specialized BigInt variant gets deleted in S5 alongside the rest of the enum. Cluster-1+ candidate: W12-bigint-typedef-and-v2-raw-migration (when BigInt Rust struct design lands).

**S2-prime dispatch shape (after C merges + S1.5 closes):**

Territory: heap-element variant migration per deletion audit §2.2 with Q25.A supersession.
- Migrate user-facing TemporalData variants (DateTime/Duration/TimeSpan likely; audit verifies) to TypedArray<*const <X>Obj> via newtype carriers (mirror StringObj precedent at `crates/shape-value/src/v2/string_obj.rs`)
- Migrate Decimal to TypedArray<*const DecimalObj>
- Migrate Instant to TypedArray<*const InstantObj>
- Skip BigInt (deferred); skip Char (now scalar bucket, S1.5 territory); String already exists post-R12
- Draft ADR-006 §2.7.24 Q25.A SUPERSEDED amendment text (verify shape against actual ADR-006 + Q25.A text; refine if needed) — lands as part of close commit
- Per-element retain/release ABI: HeapElement trait OR per-T drop_array specialization (agent decides via audit deliverable)

Audit-first deliverables before writing migration code:
- (a) TemporalData variant classification (user-facing vs AST-internal) with file:line cites
- (b) Per-element retain/release ABI shape (HeapElement trait vs per-T drop_array specialization)
- (c) Q25.A SUPERSEDED amendment text refined against actual ADR-006 + Q25.A text
- (d) Per-variant <X>Obj carrier shape (mirror StringObj precedent + element-type requirements)

Surface-and-stop dispositions (refuse on sight):
- "documented intentional duality" / preserve specialized-variants-inside-enum alongside v2-raw
- "preserve fallback for one period" / cluster-0 keeps Q25.A specialized, cluster-1 deletes
- Bool-default for unproven element-kind at retain/release boundary
- bridge/probe/helper/hop framing for the migration boundary

Standard close gate.

**R20 dispatch preview (supervisor authorizes after R19 fully closes):**

- **S5** — W12-typed-array-data-enum-deletion + ADR-006 §2.7.24 Q25.A SUPERSEDED amendment commit. Deletes TypedArrayData enum + TypedBuffer<T> wrapper layer entirely.
- **γ** — W12-jit-trait-impl-method-registry. UFCS lookup gap when receiver is dyn T (Smoke 3 JIT TAG_NULL fallthrough upstream of C's β filter).
- **U64 relabel-step (Shape D):** fold into S5 OR dispatch as separate S6 sub-cluster per supervisor's R20 disposition.

**Cluster-0 close attempt** after R20 merges + full kickoff smoke matrix passes VM == JIT.

**Open cluster-1+ candidates (tracking, not cluster-0 blockers):**

- W12-jit-trait-impl-method-registry (γ — moved to R20 as cluster-0; named here only for visibility)
- O-3/O-3a TypedObject/TraitObject Arc-vs-HeapHeader retain mismatch (W12-typed-object-array-retain-migration)
- HashMapValueBuf parallel deletion target (W12-hashmapvaluebuf-deletion)
- W17-narrow β typed-Arc carrier-shape decision
- ArrowBuffer<T> nullable carrier (O-4 deferred per kickoff smoke non-reachability)
- W12-bigint-typedef-and-v2-raw-migration (when BigInt Rust struct design lands)
- v2-raw-heap audit (phase-2d-hardening item d)
- Other phase-2d-hardening.md items (b/c/f/g)

**Open cluster-2 candidates:**

- W12-stdlib-intrinsic-collapse (IntrinsicSum / `.sum()` PHF split-brain — 7th-instance evidence at R18 Smoke 2 simplified)
- Per-HeapKind kinded jit_print
- Compile-time-boxed string-constant leak
- W12-collection-constructor-mir-lowering

**Velocity calibration at handover:**

| Phase | Sessions remaining |
|---|---|
| S2-prime dispatch + close + merge | 1 |
| R20 (S5 + γ + U64 relabel-step) | 1-2 |
| Cluster-0 close attempt | 1 |
| Cluster-1 hardening | 4-5 |
| Cluster-2 cleanup | 1-2 |
| Phase 4 | 1-2 |
| **Remaining to v1** | **9-13** |

Total handoff-to-v1 trajectory: ~**17-23 sessions** (vs prior supervisor's original 10-15 estimate; expansion concentrated in cluster-0 from honest N+1 surfacing now structurally resolved via cluster-0-transition audit).

**Last supervisor relay summary — R19 partial dispositions + S2-prime dispatch:**

(See supervisor's relay text in user's clipboard from the R19 partial close turn; the dispositions + dispatch shape are summarized above and the literal relay text is the canonical source.)

---

*End of handover. Read §First action before any dispatch.*
