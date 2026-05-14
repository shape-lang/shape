# Phase 3 cluster-0 — Supervisor handover (R19 dispatch seam)

**Generated:** 2026-05-14.
**Successor handoff point:** R19 (S2 + S1.5 + C parallel) authorized but execution pending; team-lead session rotates independently at their natural seam.
**Predecessor supervisor session:** rolled at context-fill (~end of R18 close-and-R19-dispatch-authorization cycle); this is the continuation.
**Team-lead session:** in-flight (separate Claude instance); continuous through their rotation.

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

**Bulldozer HEAD:** `bulldozer-strictly-typed @ 9135a8a6` (post-Smoke-3-framing-correction, R19 dispatch base).

**Smoke matrix (canonical kickoff, post-R18):**

| Smoke | VM | JIT | Disposition |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | passing |
| 2 canonical (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ✅ 30 | ❌ dual-carrier blocker | resolves post-S5 (R20) |
| 3 (trait `dyn T` + `t.name()`) | ✅ "x" | ❌ TAG_NULL null-handling | C in R19 unblocks |
| 4 (Set + .add + .size) | ✅ 2 | ✅ 2 | passing |

**Cumulative through R18:** 30+ sub-clusters across 18 rounds, ~3 / session steady cadence.

**R19 authorized + dispatch text drafted** (in prior supervisor session's last relay):
- **S2** — W12-typed-array-data-heap-element-migration. Heap-element variants per audit §2.2 with `<X>Obj` carrier prereqs (Decimal/BigInt/Temporal-via-newtypes/Instant/String; Char NOW in scalar bucket per audit-doc clarification, joins S1.5 instead).
- **S1.5** — W12-nativekind-scalar-additions. Revised scope: F32 + Char NativeKind/ConcreteType additions ONLY. U64 v2-raw migration DEFERRED post-S5 (Shape D: relabel to existing `Ptr(HeapKind::TypedArray)` once Arc-enum path is deleted). ADR-006 §2.7.5 amendment for F32/Char.
- **C** — W17-narrow-follow-up-B-β. TAG_NULL filter at JIT-producer site. Unblocks kickoff Smoke 3 JIT.

R19 expects 3 parallel sub-cluster close reports; file-territory non-overlapping; standard close gate.

**R20 projected:** S5 enum deletion + ADR-006 §2.7.24 Q25.A amendment + U64 relabel-step (folded into S5 OR as separate S6 sub-cluster; supervisor's call at R20 dispatch authorization based on S5 scope).

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

**R19 dispatch authorized but execution pending.** Team-lead's last status (S1.5 pre-dispatch audit ratification) received my disposition. Their next action: execute R19 dispatch with manual worktree creation per the relay text in the prior supervisor session's last response (S2 + S1.5 + C parallel from bulldozer @ 9135a8a6).

**Expected close-report cadence:**
- C (W17-narrow-follow-up-B-β): smallest scope (~50 LoC TAG_NULL filter in shape-jit/src/ffi/conversion.rs or per jit_print_* body); likely first to land
- S1.5 (W12-nativekind-scalar-additions, F32 + Char only): ~100 site cascade; medium scope
- S2 (W12-typed-array-data-heap-element-migration): heap-element variants + `<X>Obj` prereqs; largest scope

**R19 surface-and-stop dispositions you may need to make:**
- If C surfaces TAG_NULL predicates not cleanly available in shape-jit: revisit α (move predicates to shape-value + ADR amendment) vs β-with-local-synthesis (β was selected; α is fallback)
- If S1.5 F32 cascade exceeds ~100 sites materially: split F32 into S1.6 follow-up sub-cluster; S1.5 ships Char only
- If S2 surfaces heap-element variants needing different `<X>Obj` carrier shape than audit projected: surface for design decision

**R20 dispatch authority preview** (when R19 closes):
- **S5** — W12-typed-array-data-enum-deletion + ADR-006 §2.7.24 Q25.A amendment. Deletes `TypedArrayData` enum + `TypedBuffer<T>` wrapper layer. Critical close criterion: all producers migrate to `TypedArray<T>` flat-struct.
- **U64 relabel-step:** decide at R20 dispatch whether to fold into S5 OR dispatch as separate S6 sub-cluster. Folding is cleaner if S5 is well-scoped; separating is safer if S5 is large.
- Cluster-0 close attempt after R20 merges + full kickoff smoke matrix passes VM == JIT.

**Pending items owed at next status-doc-update:**
- Team-lead handover-doc annotations (CLAUDE.md user-authorization rule + supervisor-imprecision pattern with 3 instances)
- Audit-doc clarification: Char in §2.1/§3.1 scalar bucket (remove from §2.2/§3.2 heap-element)
- 8th defection-attractor instance documented in status doc

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
