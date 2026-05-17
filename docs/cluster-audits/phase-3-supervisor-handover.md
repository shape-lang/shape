# Phase 3 cluster-0+1 — Supervisor handover (R20-complete → bulldozer-cadence seam)

**Generated:** 2026-05-14.
**Updated:** 2026-05-14 with R20-close delta + **bulldozer-cadence authorization** (strategic-owner 2026-05-14). Audit-first sub-cluster cadence (R7-R20) replaced by bulldozer waves for remaining cluster-0 + cluster-1 deletion targets. See §Cadence shift below + §Pre-handover update.
**Successor handoff point:** R20 complete (γ merged at `14494605` + S2-prime-production audit-only-close RATIFIED for merge + cadence shift authorized). New supervisor's first action: receive Wave 1 close report when it lands (single audit-day; comprehensive deletion inventory).
**Predecessor supervisor session:** rolled at session-close after R20 disposition + cadence-shift authorization; this is the continuation.
**Team-lead session:** rotated at the same R20-complete + cadence-shift seam (synchronous rotation per strategic-owner authorization 2026-05-14 — fresh context infuses bulldozer cadence without audit-first attractors); new team-lead session active per `phase-3-team-lead-handover.md`.

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

## Cadence shift — bulldozer waves (load-bearing, 2026-05-14)

**Strategic-owner authorization 2026-05-14:** audit-first sub-cluster cadence (R7-R20) replaced by **bulldozer waves** for remaining cluster-0 + cluster-1 deletion targets. User's framing:

> "what happened to the bulldozer approach? why are we not deleting first, then finding
> out what that means? i think with our current plan, we leave attractors too long in
> the code. why are we deferring the removal, instead of agressively remove, then look
> at the blast radius and how to migrate? code removal is the best way to easier
> readability. the surface currently is huge, and i think at least 50% is outdated/
> wrong architecture/dead or only used by also outdated paths"

Supervisor honest assessment confirming user instinct: audit-first cadence preserved attractors in source for months (W17-typed-carrier-bundle-A Q25.A dead arms ~2 months; TypedArrayData 2-arm-shadow under O-3.c deferral; HashMapValueBuf parallel deletion target cluster-1+ deferred); 5-instance supervisor-/audit-layer imprecision pattern is the signal that audits encode beliefs not measured reality until grep verifies; session expansion 10-15 → 17-23 sessions is not justified by bug-prevention math.

**Bulldozer cadence shape:**

```
Wave 1 — Single audit-day (1 session, 1 agent)
Wave 2 — Parallel bulldoze (1-2 sessions, 6-8 agents in parallel)
Wave 3 — Stabilize + cluster-0+1 close (1 session)
```

Total: 3-4 sessions to cluster-0 + cluster-1 close. v1 trajectory becomes ~6-9 sessions remaining (Wave 1+2+3 + cluster-2 cleanup + Phase 4), not 11-16.

**Wave 2 in-scope deletion targets** (cluster-0 + cluster-1 folded together):

- TypedArrayData enum (all 22 arms; ~25 producer sites; ~120 consumer match arms)
- TypedBuffer<T> + AlignedTypedBuffer wrapper layer (485 LoC)
- HashMapValueBuf enum (Q25.B parallel deletion target)
- O-3.a TypedObjectStorage Arc → HeapHeader migration (audit §4.3 multi-week estimate restated as 1 parallel agent)
- O-3a TraitObjectStorage HeapHeader migration
- Q25.A specialization dead arms (Q25.A SUPERSEDED already landed at R20 (c))
- W12-stdlib-intrinsic-collapse (IntrinsicSum / .sum() PHF split-brain — 7th defection-attractor instance evidence)
- String + Decimal producer migration to v2-raw TypedArray<*const StringObj/DecimalObj>
- Q25.C TraitObject rebuild IF Surface A user-decision is (b)

**Discipline preserved verbatim:** all CLAUDE.md Forbidden Patterns + Renames to refuse on sight + Parallel-implementation entry; all ADR-006 §2.7.x rulings; 4-table HeapKind lockstep; §2.7.5 stamp-at-compile-time; 5-arm receiver-recovery soundness; verify-merge.sh 12/12 gate; check-no-dynamic.sh exit 0 gate; surface-and-stop discipline; no Co-Authored-By trailers; own all code quality.

**New refusal #10 (added 2026-05-14):** "Preserve X for cluster-1+" / "needs its own audit sub-cluster" / "multi-week scope is too risky" / "defer to cluster-1.5 post-close" / any framing reverting bulldozer cadence to audit-first. Cadence shift is explicit refusal of deferral framings within wave scope. Genuinely intractable in-wave gaps surface-and-stop with structured shape; supervisor disposes whether to extend wave or genuinely defer.

**What you (supervisor) authorize under bulldozer cadence:**

- Wave 1 single-audit-day dispatch (after team-lead readiness + R20 status-doc close commit lands)
- Wave 2 parallel-bulldoze dispatch (after Wave 1 ratifies; per Wave 1 (L) agent partition recommendation)
- Wave 3 stabilize + close dispatch (after Wave 2 ratifies; smoke matrix VM == JIT + cluster-0+1 close report)
- ADR amendments fold into wave merge commits (Q25.B HashMapValueBuf deletion + §4.3 O-3.a/O-3a + §2.7.5 / §2.7.6 / Q8 amendments per Wave 1 (H) cross-tier shape-conversion design)
- Cluster-1 deletion targets dispatched IN cluster-0 wave (HashMapValueBuf, O-3/O-3a, IntrinsicSum) — strategic-owner authorized as part of cadence shift

**What you do NOT do under bulldozer cadence:**

- Authorize per-target audit sub-clusters (the audit-first cadence) — refuse #10 applies at supervisor layer too
- Defer deletion targets to cluster-N+1 without surface-and-stop justification
- Re-introduce sequencing where Wave 1 (L) recommends parallel territory non-overlap
- Land CLAUDE.md modifications without explicit user ratification (R17 + 2026-05-14 compaction precedents)

## Current state at handover

**Bulldozer HEAD:** `bulldozer-strictly-typed @ 8a87ddd7` — D4 multi-session chain complete; PATH B atomic landing merged; Audit §4.3 Obstacles O-3.a + O-3a RESOLVED. Wave 2 cumulative through Round 3a (D4): Round 1 (6 agents) + Round 2 (4 agents) + Round 3a' (8 agents per-handler-family + gate-flip) + Round 3a (D4 6-sub-agent multi-session chain) all merged. Smoke matrix 3/4 VM == JIT at canonical fixture; Smoke 2 gated on Wave 3 stabilize (S5 + A2-followup-producer-cascade).

**Team-lead session rotated at D4-complete seam** (context exhaustion at 97% post-D4-merge; supervisor absorbed handover doc updates; successor team-lead's first action is Round 3b C2-joint dispatch).

**Smoke matrix (canonical kickoff, post-R20):**

| Smoke | VM | JIT | Cluster-0 criterion |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | ✓ |
| 2 (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ✅ 30 | ❌ rc=1 | gated on Wave 2 (TypedArrayData deletion eliminates dual-carrier reality) |
| 3 (canonical fixture `let t = X{}`) | ✅ x | ✅ x | ✓ post-γ |
| 4 (Set + .add + .size) | ✅ 2 | ✅ 2 | ✓ |

**3 of 4 kickoff smokes pass VM == JIT at canonical fixture.** Smoke 2 unblocks post-Wave-2. Smoke 3 fixture-vs-prose drift (Surface A) awaits user disposition.

**Surface A — user-pending disposition (cluster-0 close criterion shape):**

- (a) "Smoke fixtures pass VM == JIT" — what's been operationally tested R10-R20; silent re-scope
- (b) "Kickoff prompt prose intent matches" — requires Q25.C TraitObject rebuild pre-close; +3-5 sessions
- (c) Split — current canonical-smokes-as-fixtures = cluster-0 close; Q25.C rebuild + `dyn T = box(X{})` fixture = explicit cluster-1.5 close-criterion item

Supervisor recommendation: (c) split. User decides; Wave 1 audit-day maps all three options without blocking.

**Cumulative through R20:** ~33+ sub-clusters across 20 rounds; cadence shift converts remainder from N+ audit-first sub-clusters to 3-4 bulldozer waves.

**Wave 2 close summary (R20 + Wave 1 + Wave 2 Round 1/2/3a'/3a all merged):**

- **R20 γ** (`28bd0a7f` → merge `14494605`): JITContext.function_names_ptr/_len linking. Smoke 3 canonical fixture VM == JIT.
- **R20 S2-prime / S2-prime-production**: HeapElement trait + DecimalObj + StringObj impl + drop_array_heap; merged at predecessor ceremony.
- **Wave 1 audit-day** (`bulldozer-wave-1-inventory.md`): A-R sections covering all cluster-0+1 deletion targets; agent partition recommendation.
- **Wave 2 Round 1** (6 agents A1/B/C/D1/F/G): ~6.8k LoC; 7 imprecision instances; merged.
- **Wave 2 Round 2** (4 agents A2/D2/E/C2a): E substantive land + 3 surface-and-stops; merged.
- **Wave 2 Round 3a'** (8 agents per-handler-family α-η + gate-flip): ~951 LoC migration foundation; S1-R18 DURABLE PATTERN ratified by user; merged.
- **Wave 2 Round 3a (D4)** (6-sub-agent multi-session chain): PATH B atomic landing at `47b55a63`; ADR-006 §2.3 amendment landed (Path B TypedObjectPtr/TraitObjectPtr canonical pattern); 5-arm receiver-recovery violation FIXED inline at `object_ops.rs:59`; merged at `8a87ddd7`. Audit §4.3 Obstacles O-3.a + O-3a RESOLVED.

**D4 PATH B canonical Ptr-newtype pattern** (ratified 2026-05-14; ADR-006 §2.3 lines 301-388):

- `HeapValue::TypedObject(TypedObjectPtr)` — `#[repr(transparent)]` newtype around `*const TypedObjectStorage`; manual Drop/Clone calling `release_elem`/`v2_retain`; manual `unsafe impl Send + Sync`; HeapValue auto-derives chain through newtype.
- `HeapValue::TraitObject(TraitObjectPtr)` — mirror newtype.
- CANONICAL for v2-raw HeapHeader-equipped storage types only. Arc<String> remains canonical for String payload (ADR-005 §2 exception); no "StringPtr" sibling.

**Bounded forbidden under D4 (extends §Renames to refuse on sight):**

- "TypedObjectPtr shim" / "TraitObjectPtr bridge" / "Ptr-newtype helper" / any bridge/probe/helper/hop framing for these newtypes
- Parallel `Arc<TypedObjectStorage>` / `Arc<TraitObjectStorage>` payloads alongside Ptr-newtype shapes
- Ptr-newtype siblings for non-HeapHeader-equipped storage types

**Cumulative discipline-pattern instances through D4:**

- 22 imprecision-pattern instances cumulative (8 supervisor-layer / 14 audit-layer; all caught at agent layer pre-source-change)
- 5 S1-R18 DURABLE PATTERN instances (Wave 1 audit + D1 drive-by + Round 3a' δ/ε/ζ); pattern operational per user ratification
- 8 parallel-implementation defection-attractor instances (all surfaced + structurally resolved)

**Round 3b C2-joint dispatch (your first authorization):**

HashMapData<V> per-V monomorphization atomic single-commit (~5k LoC / 40 files) per C2a structural finding. Cannot split runtime/FFI tiers per type-confusion-window invariant. Likely needs multi-session chain per ceiling-c (mirror D4 pattern). Territory + dispatch shape detailed in `phase-3-team-lead-handover.md §In-flight state`. Team-lead dispatches after successor reads 9 mandatory docs + posts confirmation.

After Round 3b closes: Wave 3 stabilize (S5 wholesale TypedArrayData enum deletion + A2-followup-producer-cascade for Array<string> literal upgrade + shape-test baseline classification) → cluster-0+1 close attempt.

**Velocity:** total handoff-to-v1 4-7 sessions remaining (Round 3b 1-2 + Wave 3 stabilize 1-2 + cluster-0+1 close 0.5 + cluster-2 1-2 + Phase 4 1-2). Was 17-23 sessions at session-start.

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
