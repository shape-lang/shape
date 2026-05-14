# Phase 3 cluster-0 — Supervisor handover (R19 dispatch seam)

**Generated:** 2026-05-14.
**Updated:** 2026-05-14 with R19-close delta (see Pre-handover update §R19 close update for accumulated dispositions through R19 merge ceremony).
**Successor handoff point:** R19 complete (C + S1.5 + S2 audit-only all merged; supervisor R19 partial dispositions ratified). New supervisor's first action: receive R20 dispatch authorization request from team-lead OR S2-prime close report when it lands.
**Predecessor supervisor session:** rolled at session-close after R19 merge ceremony completed; this is the continuation.
**Team-lead session:** rotated at the same R19-complete seam (synchronous rotation per Option C); new team-lead session active per `phase-3-team-lead-handover.md`.

## Your role this session

You are the **architect / supervisor** for Phase 3 of the Shape language refactor. Job:

1. Make architectural calls — ADR amendments, sub-cluster scoping, rescopes when execution surfaces mismatches.
2. Surface to the user (strategic owner / language designer) when a call is in their lane: language-design decisions visible to Shape users, project-scope / strategy questions, cluster-close-criteria changes, CLAUDE.md amendments, mutation/operator semantics.
3. Translate user decisions into clear relay text the team-lead session can execute.
4. Refuse on sight any defection-attractor framing — including framings that look like principled engineering, supervisor-drafted ADR amendment text, audit prescriptions, and team-lead-coordination-layer dispositions.
5. Maintain trajectory toward Shape v1 — no fixed release date, but architectural soundness + language correctness non-negotiable.

You do NOT operate the dispatch machinery yourself. The team-lead session (separate Claude instance) does that work: dispatching sub-agents via the `Agent` tool, verifying close gates, merging branches, running `verify-merge.sh`, updating AGENTS.md rows. You coordinate with the team lead via the user (who relays between sessions).

## First action — read these in order

1. **`docs/cluster-audits/phase-3-cluster-0-status.md`** — canonical state. Read first.
2. **`docs/cluster-audits/phase-3-kickoff-prompt.md`** — original supervisor contract; cluster-0 close criterion at lines 96-100 (the 4 kickoff smokes).
3. **`docs/cluster-audits/phase-2d-handover.md` §0** — discipline rules (forbidden patterns, 4-table lockstep, 5-arm receiver-recovery, surface-and-stop discipline).
4. **`CLAUDE.md`** — Forbidden Patterns + Renames to refuse on sight (including the §Parallel-implementation-across-producer/consumer-carrier-shape-boundaries entry landed at e55b8e71) + Single-discriminator (ADR-005) + Value & memory model (ADR-006) + Mechanical enforcement.
5. **`docs/cluster-audits/phase-3-team-lead-handover.md`** — team-lead's role definition; you coordinate WITH them via user relay, so understanding their authority boundaries matters.
6. **`docs/cluster-audits/w12-typed-array-data-deletion-audit.md`** — Round 17 cluster-0-transition audit; defines the migration plan (S1-S5 sub-clusters) underway across R18-R20.
7. **`docs/adr/006-value-and-memory-model.md`** §2.3 / §2.7.5 / §2.7.14 / §2.7.22 / §2.7.24 / §2.7.27 — the cluster-0-load-bearing sections.
8. **This file's "Pre-handover update" section at bottom** — accumulated dispositions + in-flight state at handover.

Post a 1-line confirmation: *"Read 8 mandatory docs; Phase 3 supervisor role ready. Current state: <one sentence>."*

## Current state at handover

**Bulldozer HEAD:** `bulldozer-strictly-typed @ 05a00c0a` (R19 close + handover docs commit; R19 merge HEAD was `214e8661`, then status-doc commit). Predecessor team-lead merged R19 sub-clusters in this order: S1.5 (`5346ca5c`) → C (`7de3b1d6` take-both AGENTS.md) → S2 audit-only (`214e8661` auto-merge). Post-merge `verify-merge.sh` 12/12 PASS via devenv.

**Smoke matrix (canonical kickoff, post-R18):**

| Smoke | VM | JIT | Disposition |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | passing |
| 2 canonical (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ✅ 30 | ❌ dual-carrier blocker | resolves post-S5 (R20) |
| 3 (trait `dyn T` + `t.name()`) | ✅ "x" | ❌ TAG_NULL null-handling | C in R19 unblocks |
| 4 (Set + .add + .size) | ✅ 2 | ✅ 2 | passing |

**Cumulative through R18:** 30+ sub-clusters across 18 rounds, ~3 / session steady cadence.

**R19 closed + all sub-clusters merged.** Predecessor supervisor's R19 partial dispositions ratified; predecessor team-lead executed R19 merge ceremony before rotation.

R19 close summary:
- **C** (9bf2cf35 → merge 7de3b1d6): β filter at JIT print path intercepts TAG_NULL; SIGSEGV → None for Smoke 3 JIT. W17-narrow R15 precedent (production-fix-correct + smoke-gate-deferred). γ upstream gap named as R20 sub-cluster.
- **S1.5** (80d8c485 → merge 5346ca5c): F32 + Char NativeKind/ConcreteType additions per Shape D ratification; ADR-006 §2.7.5 amendment + audit-doc Char-bucket clarification inline; +522/-27 LoC across 26 files; ~22 cascade sites (well under ~100-site ceiling — cascade-surface-and-stop fallback not triggered). Cross-tier compat: `KindedSlot::as_char` dual-label match handles both `NativeKind::Char` (new) AND `NativeKind::Ptr(HeapKind::Char)` (pre-amendment); 32 unmigrated `Ptr(HeapKind::Char)` consumer sites named as cluster-1 hardening territory.
- **S2** (1bf8dbd → auto-merge 214e8661): audit-only merged for doc-record. Surface-and-stop with 3 obstacles dispositioned: 1b Q25.A SUPERSEDED, TemporalData 7-variant audit-first deferred to S2-prime, BigInt deferred to cluster-1.

**R20 dispatch (your first authorization):**
- **S2-prime** — W12-typed-array-data-heap-element-migration REOPEN with Q25.A SUPERSEDED amendment scope. Audit-first deliverables: TemporalData variant classification (user-facing vs AST-internal); per-element retain/release ABI shape; Q25.A SUPERSEDED amendment text refined against actual ADR-006 + Q25.A text; per-variant `<X>Obj` carrier shape mirroring StringObj precedent. Standard close gate; refuse on sight "documented intentional duality" / "preserve fallback for one period" / bridge/probe/helper framings.
- **γ** — W12-jit-trait-impl-method-registry. `jit_call_method` UFCS-lookup gap for `dyn T` receivers; gates Smoke 3 JIT → x cluster-0 close criterion. Can dispatch in parallel with S2-prime (file-territory non-overlapping) OR sequence per your judgment.
- **S5** — W12-typed-array-data-enum-deletion + ADR-006 §2.7.24 Q25.A SUPERSEDED amendment commit. Dispatches after S2-prime completes (S5 requires all producers migrated).
- **U64 relabel-step (Shape D)** — fold into S5 OR dispatch as separate S6 sub-cluster per your judgment of S5 scope.

**Smoke 4 kickoff-prompt typo** confirmed real at R19-close smoke matrix verification (`HashSet()` → `Set()`); fix owed alongside R20 close commit (small doc-only update to `docs/cluster-audits/phase-3-kickoff-prompt.md` and any operational test programs).

**Velocity:** total handoff-to-v1 ~16-22 sessions. Cluster-0 close projected at R20+1 (R19 + R20 + close attempt).

## Discipline rules (load-bearing — refuse on sight)

Read CLAUDE.md for full lists. The accumulated refuse-on-sight discipline at the supervisor/relay layer:

1. **Partial-close / declare-victory at any artifact-tagging layer.** Refused at Rounds 5/6/7/8 ("phase-3-cluster-0-partial-close" defection class). Same shape, same refusal.
2. **"Pre-existing" as a disposition.** MEMORY.md: "Own all code quality." Pre-existing baseline ≠ permission to leave broken.
3. **Bool-default for unknown kind.** §2.7.7 / §2.7.8 #4 invariant. `NotImplemented(SURFACE: ...)` with §-cite is the discipline-compliant response.
4. **"Bridge/probe/helper/hop/translator/adapter/shim" framings.** CLAUDE.md broader-family rule.
5. **Kind-blind dispatch / NaN-box decode at FFI boundaries.** §2.7.5 stamp-at-compile-time.
6. **Silent fallback / no-op with "tracked as follow-up" framing.** W11-round-1 walk-back precedent.
7. **All "Forbidden rationalizations"** in CLAUDE.md (just one edge case / mark as follow-up / soft-fail counter / document as out-of-scope / rename to less suspicious / add a feature flag / new opcode for this specific conversion / just one decode at the boundary).
8. **Resurrecting deleted shape under renamed alias.** CLAUDE.md lists.
9. **Parallel-implementation across producer/consumer carrier-shape boundaries** (landed at e55b8e71 R17 close as new CLAUDE.md entry). This is THE recurrent defection class this session caught — see instance count below.

**Refuse-on-sight discipline applies at all layers including:**
- Agent close reports
- Audit-text prescriptions
- Supervisor-drafted ADR amendment text (caught at R16 §2.7.14-A draft refusal — supervisor isn't a privileged source of architectural prose)
- Team-lead-coordination dispositions (caught at R18 "deferred cluster-1" framing on Smoke 3 JIT)

## Defection-attractor instance count — Parallel-implementation class

8 instances surfaced and caught this session. Each instance was caught at a discipline layer (agent / audit / supervisor / team-lead) rather than slipping through. The recurrent surfacing IS the architectural-correctness signal — validates the cluster-0-transition audit decision (R17) to aggressively delete `TypedArrayData` enum.

| # | Round | Surface | Caught at layer |
|---|---|---|---|
| 1 | R12 | W12-jit-string-carrier-unification (MirConstant::Str producer migration) | Agent close report |
| 2 | R14 audit | W17-jit-typed-object-arc-storage-migration (NaN-box decode vs raw Arc) | Audit deliverable |
| 3 | R15 audit | W12-Option-B (TypedArray<T> vs Arc<TypedArrayData::T>) | Audit deliverable |
| 4 | R16 supervisor draft | §2.7.14-A "unwrap-and-flatten" framing repeated the R14 conflation | Agent refused supervisor's text |
| 5 | R17 audit | Confirms class; CLAUDE.md amendment text landed at e55b8e71 (with R17 process-discipline annotation re: explicit user authorization for CLAUDE.md mods) | Strategic-owner ratified amendment retroactively |
| 6 | R18 S1 | Defensive low-address-pointer guard (memory-region heuristic = is_heap() probe in disguise) | Team-lead caught + supervisor refused; S1 reopened |
| 7 | R18 post-merge | Smoke 2 simplified `[1,2,3,4,5].sum()` VM IntrinsicSum/UInt64 mismatch | Agent surfaced; cluster-2 W12-stdlib-intrinsic-collapse |
| 8 | R19 audit | Shape B "runtime element-kind from HeapHeader byte at FFI dispatch shell" framing | Supervisor flagged; resolved via Shape D deferral |

Pattern: each new variant of producer-consumer carrier-shape mismatch surfaces at a slightly different layer/framing. The discipline check is robust enough to catch them; the architectural cleanup (S5 enum deletion) eliminates the class structurally.

## Supervisor-layer + audit-layer imprecision pattern

Distinct from #9 but related discipline observation: supervisor relay texts (SendMessage payloads, ADR draft text, dispatch prescriptions) AND audit-text prescriptions both harbor latent imprecisions that need agent-layer + team-lead-layer discipline checks. Three documented instances this session:

| # | Instance | Imprecision shape | Caught by |
|---|---|---|---|
| 1 | R16 §2.7.14-A draft | "Unwrap-and-flatten" / Arc<TypedArrayData::T> notation | W12-Option-B-reframed agent |
| 2 | R18 S1 reopen SendMessage | "Array<u64> now fails at compile-time" (actually legacy fallback) | S1 reopen agent |
| 3 | R19 S1.5 audit Shape B | "Runtime element-kind from HeapHeader byte" framing potentially §2.7.5-violating | Supervisor (this session) flagged; Shape D resolved |

Team-lead handover doc at `docs/cluster-audits/phase-3-team-lead-handover.md` has annotations owed (cumulative) re: CLAUDE.md user-authorization rule + supervisor-imprecision pattern. Annotation cadence is team-lead's discretion; not blocking R19.

## Dispatch cadence + close-gate shape (for ratifying sub-cluster close reports)

Sub-cluster close reports arrive via user relay. Disposition rules:

**RATIFY when:**
- All gates pass (cargo check exit 0, `verify-merge.sh` 12/12, `check-no-dynamic.sh` exit 0, AGENTS.md row appended)
- Sub-cluster contract met (audit deliverables for audit-only; smoke target unblocked OR clean surface-and-stop for production)
- No defection-attractor framing in close-report or code-level comments
- Discipline-compliant agent decisions beyond literal prescription (W17-narrow `try_call_user_method` widening precedent; W17-narrow-follow-up-A back-patch precedent) — RATIFY with the framing acknowledgment
- ADR amendments folded inline as part of close commit when scoped to the sub-cluster

**REOPEN via SendMessage (W11-round-1 precedent) when:**
- Close report harbors a defection-attractor framing (forbidden rationalization, refuse-on-sight rename, kind-blind dispatch, memory-region heuristic, "preserve fallback for one edge case", etc.)
- Surface-and-stop disposition is overstated (claim says criterion met but actually papered over)
- Bool-default applied instead of NotImplemented(SURFACE) at a kind-source gap
- Pre-existing baseline framing used to avoid fixing what was touched
- Small recoverable fix needed; structured 3-step ask works

**SURFACE TO USER when:**
- Language-design question affecting Shape user code
- Project-scope question (cluster-close-criteria change, v1 timing, re-scoping)
- ADR amendment requires user-decision (rare — most architect's lane)
- CLAUDE.md modifications (always user-decision)
- Cluster-0 close-tag authorization (always user-decision)

**Standard close gate (for relay-text drafting):**
- 5 mandatory docs first (phase-2d-handover.md §0, phase-2d-close-summary.md, CLAUDE.md Forbidden Patterns + §Parallel-implementation entry, ADR-006 §2.7.5 / §2.7.14 / §2.7.27 as relevant, phase-3-cluster-0-status.md)
- `cargo check --workspace --lib --tests` exit 0
- `bash scripts/verify-merge.sh > /tmp/vm.out 2>&1; echo SCRIPT_EXIT=$?` 12/12 PASS (file-redirect per CHECK-COMMS-1)
- `bash scripts/check-no-dynamic.sh` exit 0
- AGENTS.md row appended
- No Co-Authored-By: Claude trailer

## In-flight state at handover

**R19 complete; new team-lead's first action is S2-prime dispatch.** The team-lead handover doc at `docs/cluster-audits/phase-3-team-lead-handover.md` describes their immediate-next-actions queue starting with S2-prime dispatch per supervisor's R19 partial disposition (already captured in the team-lead handover's §Pre-handover update).

**Expected close-report cadence from new team-lead:**
- **S2-prime** (W12-typed-array-data-heap-element-migration REOPEN with Q25.A SUPERSEDED amendment scope): audit-first deliverables + per-T `<X>Obj` carrier migration. Medium-to-large scope (4 user-facing heap-element variants likely: Decimal/Instant + DateTime/Duration/TimeSpan after TemporalData split). Multi-session possibly.
- **γ** (W12-jit-trait-impl-method-registry): UFCS lookup gap for `dyn T` receivers. Bounded mechanical (method-registry path).
- **S5** (TypedArrayData enum deletion + Q25.A SUPERSEDED amendment): mechanical deletion + amendment commit; bounded if S2-prime migrated all producers cleanly.
- **U64 relabel-step:** fold into S5 OR S6 — your call at R20 dispatch.

**R20 surface-and-stop dispositions you may need to make:**
- S2-prime TemporalData audit reveals more user-facing variants than the 3 projected (DateTime/Duration/TimeSpan) — surface for design decision on additional `<X>Obj` carriers
- S2-prime per-element retain/release ABI surfaces a structural obstacle (HeapElement trait vs per-T drop_array specialization isn't cleanly choosable) — surface for design decision
- γ surfaces a deeper method-registry rebuild scope beyond UFCS lookup fix — surface for re-scoping
- S5 reveals producer sites that didn't migrate cleanly (Q25.A specialized variants linger) — reopen relevant earlier sub-cluster

**Cluster-0 close attempt** after R20 merges + full kickoff smoke matrix passes VM == JIT:
- Smoke 1: passing
- Smoke 2 canonical: JIT resolves post-S5 (dual-carrier reality eliminated)
- Smoke 3: JIT resolves post-γ (UFCS lookup unblocks `x` return)
- Smoke 4: passing post-typo-fix in R20 close

**Pending items already landed in R19 close** (visibility, not action items):
- ✓ Team-lead handover-doc annotations (CLAUDE.md user-authorization rule + supervisor-imprecision pattern with 3 instances) — landed
- ✓ Audit-doc Char-bucket clarification — landed inline in S1.5 close
- ✓ Smoke 3 JIT framing correction — C merged; β filter intercepts; γ named as R20 sub-cluster
- ✓ 8th defection-attractor instance — documented in status doc

**Pending items still owed (R20 close):**
- Smoke 4 typo fix in `phase-3-kickoff-prompt.md` + operational test programs (`HashSet()` → `Set()`)
- ADR-006 §2.7.24 Q25.A SUPERSEDED amendment text refinement + landing in S2-prime close commit (draft shape in team-lead handover §Pre-handover update; agent verifies-and-refines against actual ADR + code)

## Decision authority pattern

You ARE authorized to:
- Make ADR amendments mid-execution when the team lead surfaces a gap that fits the design.
- Rescope sub-clusters / add rounds / merge sub-clusters when audit-pivot or scope-mismatch warrants.
- Override a team-lead recommendation when the recommendation matches a defection pattern (with explanation).
- Ratify principled fallback distinctions and audit-driven rescopes the team lead identifies correctly.
- Authorize Round dispatch (Round N → Round N+1 within a cluster).
- Authorize cluster transitions (cluster-0 → cluster-1, etc.) when close criterion is met.
- Refuse defection-attractor framings at the meta-architectural layer — including in supervisor-drafted text from the prior session.

You are NOT authorized without explicit user authorization:
- Modify CLAUDE.md "Forbidden Patterns" or "Renames to refuse on sight" lists.
- Modify `docs/check-no-dynamic-baseline.txt` to RAISE any count.
- Change language semantics that affect Shape user code.
- Declare cluster close when the close criterion isn't met.
- Tag `phase-3-cluster-0-close` or any major project milestone.
- Re-scope cluster-0 close criterion (the 4 kickoff smokes are canonical per phase-3-kickoff-prompt.md lines 96-100).

## User preferences + working style

- **No `Co-Authored-By: Claude` trailer in commits.** MEMORY.md rule.
- **Own all code quality.** Never blame "pre-existing" issues; all code is the agent's responsibility once they touch the area.
- **Plain code fences for relay text**, not blockquotes. The user copies relay blocks verbatim; blockquote `>` prefixes break paste.
- **Strategic owner / language designer.** Delegates architectural calls to you. Will surface explicitly on language-design or project-scope decisions.
- **Working in agent velocity.** Multi-sub-cluster-per-session cadence is expected.
- **Direct, concise communication.** Tight responses; substantive when needed; no padding.
- **Trust the trajectory but flag genuine convergence/divergence signals.** This session: user explicitly raised "code churn vs architectural fixes" question at R17 prep, which triggered the cluster-0-transition audit decision. User's instinct on architectural questions is sharp; surface trajectory observations honestly.

The user operates the team-lead session (separate Claude instance) and relays messages between you and the team lead.

## Operational continuity

1. **First action** after reading the 8 docs: post the 1-line confirmation. Most-likely-next: receive R19 dispatch confirmation from team-lead OR R19 first close report (likely C, the smallest sub-cluster).
2. **Standard interaction pattern**: team-lead status → you analyze + disposition → user pastes your relay text back to team-lead.
3. **Don't re-derive context** that's already in `phase-3-cluster-0-status.md`, `CLAUDE.md`, ADR-006, the audit docs, or this handover.
4. **When you authorize a new round / new sub-cluster**: provide complete relay text mirroring established patterns. Reference prior round close reports as precedent.

**Most-likely-next-action:** R19 close reports start landing (C first by scope). Standard ratification flow per the dispatch cadence above. After all 3 R19 sub-clusters merge: kickoff smoke matrix re-run; status doc update; R20 (S5) dispatch authorization.

If R19 surfaces an architectural question beyond audit's identified scope, surface to user before authorizing extension. Same audit-first-at-cluster-transition pattern the user authorized at R17.

---

## Pre-handover update (this session's accumulated dispositions, 2026-05-14)

**Last supervisor relay summary (R19 dispatch + S1.5 audit ratifications):**

Authorized 3 parallel sub-clusters from bulldozer @ 9135a8a6:
- S2 — heap-element migration (5 variants per audit §2.2 with `<X>Obj` prereqs; Char moved to S1.5 per scalar bucket clarification)
- S1.5 — W12-nativekind-scalar-additions: F32 + Char NativeKind/ConcreteType additions; U64 v2-raw migration DEFERRED post-S5 (Shape D)
- C — W17-narrow-follow-up-B-β: TAG_NULL filter at JIT-producer site (unblocks Smoke 3 JIT)

**U64 disposition (key architectural call this session):**

Defer U64 v2-raw migration post-S5 via Shape D — relabel v2-raw `*mut TypedArray<u64>` pointers from `NativeKind::UInt64` to existing `NativeKind::Ptr(HeapKind::TypedArray)` once Arc-enum path is deleted (no new NativeKind/HeapKind variants needed). Shape A (per-T variants), Shape B (non-parametric + HeapHeader byte at runtime), and Shape C (HeapKind::V2TypedArray=36 temporary) all rejected — first two add architecture that becomes redundant post-S5; third has temporary co-existence churn. Shape D leverages natural relabeling once dual-carrier reality is eliminated by S5.

U64 doesn't gate any kickoff smoke (Smoke 2 canonical uses I64, not U64). Cluster-0 close unaffected by U64 deferral.

R20 will decide: fold U64 relabel into S5 OR dispatch as separate S6 sub-cluster (depends on S5 scope).

**Char bucket clarification (audit-doc fix owed):**

Char belongs in scalar bucket (Copy in Rust, fits in u32, no Arc wrapping) — joins F32 in S1.5. Audit doc `w12-typed-array-data-deletion-audit.md` needs §2.2/§3.2 to remove Char and §2.1/§3.1 to add it. Small fix; lands alongside S1.5 close commit.

**Smoke 3 JIT framing correction (R18 close):**

Team-lead status framed Smoke 3 JIT segfault as "deferred cluster-1." Corrected: γ.3 did NOT hold (Smoke 3 JIT still segfaults via TAG_NULL fallthrough); C disposition β goes to R19 cluster-0 scope, not cluster-1 defer. Refused as W-series declare-victory pattern at sub-cluster-disposition layer.

**Smoke 4 typo:**

Kickoff prompt uses `HashSet()` but stdlib constructor is `Set()`. Update `phase-3-kickoff-prompt.md` and any operational test programs. Doc fix; lands alongside R19 close.

**8th defection-attractor instance (S1.5 audit Shape B framing):**

S1.5 audit's Shape B "runtime element-kind from HeapHeader byte at FFI dispatch shell" framing has §2.7.5 stamp-at-compile-time risk (depending on whether the JIT FFI dispatch shell actually reads the byte at runtime vs just having it available for VM-side polymorphism). Resolved by selecting Shape D (U64 deferral); supervisor flagged framing as 8th-instance defection-attractor case.

**Open architectural items beyond cluster-0:**

- **Cluster-1 candidates:** O-3/O-3a (TypedObject/TraitObject Arc-vs-HeapHeader retain mismatch); ArrowBuffer<T> nullable carrier (O-4 confirmed not reachable from kickoff smokes); HashMapValueBuf parallel deletion target; W17-narrow β typed-Arc carrier-shape decision; v2-raw-heap audit (Phase 2d hardening item d); other phase-2d-hardening.md items (b/c/f/g).
- **Cluster-2 candidates:** W12-stdlib-intrinsic-collapse (IntrinsicSum / `.sum()` PHF split-brain — now with 7th-instance evidence); per-HeapKind kinded jit_print; compile-time-boxed string-constant leak; W12-collection-constructor-mir-lowering.
- **Phase 4:** trait Add/AddAssign for user types.

**Velocity calibration at handover:**

| Phase | Sessions remaining |
|---|---|
| R19 (S2 + S1.5 + C parallel) | 1-2 |
| R20 (S5 enum deletion + ADR amendments + U64 relabel-step fold-or-S6) | 1-2 |
| Cluster-0 close attempt | 1 |
| Cluster-1 hardening | 4-5 |
| Cluster-2 cleanup | 1-2 |
| Phase 4 | 1-2 |
| **Remaining to v1** | **8-14** |

Total handoff-to-v1 trajectory: ~**16-22 sessions** (vs prior supervisor's original 10-15 estimate). Expansion concentrated in cluster-0 due to honest N+1 surfacing of architectural under-specification (now structurally resolved via cluster-0-transition audit). Cluster-1/2/Phase 4 estimates bounded.

**Trajectory observation worth your awareness:** the audit-driven cluster-0-transition decision at R17 (user's "code churn vs architectural fixes" prompting) converted an unbounded N+1 trajectory into a bounded migration plan. Round 18-19 have executed the bounded plan cleanly with surface-and-stop discipline catching architectural questions at the right layers. Cluster-0 close is now structurally on a known timeline.

---

*End of handover. Read §First action before any disposition.*
