# Phase 3 cluster-0 — Team-lead handover (Round 14 → Round 15 seam)

**Generated:** 2026-05-13.
**Successor handoff point:** after Round 14 audit-only merges, before Round 15 dispatch.
**Predecessor team-lead session:** rolled at context-fill (~81%); this is the continuation.
**Supervisor session:** continuous (same Claude instance has been making cluster-0 calls Round 5 onward).

## Your role this session

You are the **team lead** for Phase 3 cluster-0 of the Shape language refactor. Job:

1. Operate the `Agent` tool to dispatch sub-cluster agents per supervisor relays.
2. Verify close gates (`cargo check --workspace --lib --tests`, `bash scripts/verify-merge.sh`, `bash scripts/check-no-dynamic.sh`, AGENTS.md row).
3. Merge sub-cluster branches into `bulldozer-strictly-typed`. Take-both merge resolution for AGENTS.md row collisions + dispatch-table arm-line conflicts is the established pattern.
4. Run smoke-matrix verification under both `--mode vm` and `--mode jit` after every round closes.
5. Update `docs/cluster-audits/phase-3-cluster-0-status.md` after each round + supplementary -ext smoke disposition tracking.
6. Surface architectural questions to the supervisor via the user (strategic-owner relays).

The supervisor is a separate Claude instance — the user copies my relays into your session, you copy your responses back to the user, the user pastes them to me. Do **not** make architectural calls yourself (ADR amendments, cluster scoping, defection-pattern refusals, partial-close authorization, cluster-tag authorization); surface them to the supervisor with structured context.

## First action — read these in order

1. **`docs/cluster-audits/phase-3-cluster-0-status.md`** — canonical state. Read first.
2. **`docs/cluster-audits/phase-3-kickoff-prompt.md`** — original supervisor contract; cluster-0 close criterion lives at lines 96-100 (the 4 kickoff smokes).
3. **`docs/cluster-audits/phase-2d-handover.md` §0** — discipline rules (forbidden patterns, 4-table lockstep, 5-arm receiver-recovery, surface-and-stop discipline).
4. **`CLAUDE.md`** — Forbidden Patterns + Renames to refuse on sight + Single-discriminator (ADR-005) + Value & memory model (ADR-006) + Mechanical enforcement.
5. **`AGENTS.md`** — live roster (very long file; the relevant rows are the Round 5-14 entries at the bottom).
6. **`docs/adr/006-value-and-memory-model.md`** §2.7.5 / §2.7.14 / §2.7.24 / §2.7.27 — the cluster-0-load-bearing sections.
7. **This file's "Pre-handover update" section at bottom** — last-minute state from the rolling team-lead session, including the supervisor's Round 15 dispatch authorization.

Post a 1-line confirmation: *"Read 7 mandatory docs; team-lead role ready. Current state: <one sentence>."*

## Current state at handover

**Branch HEAD:** `bulldozer-strictly-typed` at the post-Round-14-merge commit (see Pre-handover update for actual hash).

**Smoke matrix (canonical kickoff, post-Round-14):**

| Smoke | VM | JIT | Disposition |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | passing |
| 2 (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ✅ 30 | ❌ runtime SIGSEGV (producer/consumer carrier mismatch) | Round 15 W12-map-chained Option B |
| 3 (trait `dyn T` + `t.name()`) | ✅ "x" | ❌ TypedObject Arc storage classification | Round 15 W17-narrow (Option γ) |
| 4 (`let mut Set; .add; .size`) | ✅ 2 | ✅ 2 | passing |

**Cumulative through Round 14:** 28 sub-clusters across 14 rounds, ~3 / session steady cadence.

**Round 15 dispatch authorized** (per supervisor relay in Pre-handover update): W17-narrow + W12-map-chained Option B in parallel from post-Round-14 baseline. Both production-first (audit work already landed in Round 14).

**Velocity (supervisor's calibration):** Round 15 is the projected last sub-cluster round of cluster-0 if no N+1 surfaces. Cluster-0 close attempt likely in 1-2 more team-lead-session work units. Total v1 estimate: 14-19 sessions handoff-to-v1 (vs original 10-15), expansion concentrated in cluster-0.

## Discipline rules (load-bearing — refuse on sight)

Read CLAUDE.md for the full lists. The supervisor specifically refuses these framings on sight at the relay layer; you should refuse them at the dispatch layer (in agent prompts) AND at the close-report review layer:

1. **Partial-close / declare-victory at any artifact-tagging layer.** "phase-3-cluster-0-partial-close" was refused at Rounds 5/6/7/8. Same shape, same refusal.
2. **"Pre-existing" as a disposition.** MEMORY.md rule: "Own all code quality. Never blame 'pre-existing' issues; all code is the agent's responsibility once they touch the area." If an agent's close report uses "pre-existing" to justify leaving a forbidden pattern alive, reopen via SendMessage (W11-round-1 precedent at ADR-006 §2.7.14).
3. **Bool-default for unknown kind.** The correct response to "I don't have a kind for this slot" is `NotImplemented(SURFACE: ...)` with §-cite, not `kind: NativeKind::Bool`.
4. **"Bridge/probe/helper/hop/translator/adapter/shim" framings** for any of: tag-decode dispatch, kind-blind ABI, value-call ABI, dispatch-shell ABI, frame-setup carriers, capture-injection. CLAUDE.md "Renames to refuse on sight" broader-family rule.
5. **Kind-blind dispatch / NaN-box decode at FFI boundaries.** §2.7.5 stamp-at-compile-time discipline.
6. **Silent fallback / no-op with "tracked as follow-up" framing.** W11-round-1 walk-back precedent — the agent closed with `jit_arc_retain` as silent no-op; supervisor reopened; agent re-closed with real `Arc::increment/decrement_strong_count`.
7. **"Just one edge case" / "Mark as follow-up for later phase" / "Soft-fail counter for now, harden later" / "Document as out-of-scope" / "Rename to less suspicious name"** — CLAUDE.md "Forbidden rationalizations" full list.
8. **Resurrecting deleted shape under renamed alias.** `LegacyResultData`, `OldRangeShape`, `MethodFnLegacy`, etc. — listed in CLAUDE.md.
9. **Parallel implementations of the same operation across producer-consumer carrier-shape boundaries.** Surfaced as a recurrent class during the rolling team-lead session: first instance was `BuiltinFunction::IntrinsicSum` vs `.sum()` PHF method dispatch (cluster-2 candidate W12-stdlib-intrinsic-collapse); second instance is the `.map()` return-shape vs `try_emit_v2_array_method` fast-path expectation that surfaced Smoke 2 JIT SIGSEGV. Two instances so far; CLAUDE.md amendment trigger at 3.

When you spot one in an agent's close report: **do not merge.** Surface to supervisor with the structured shape (file:line, framing quote, applicable CLAUDE.md rule cite). Supervisor will return either a SendMessage-reopen relay or a re-dispatch decision.

## Dispatch cadence + close-gate shape

**Sub-cluster dispatch prompt template** (mirrors `phase-3-kickoff-prompt.md` / `phase-2d-wave-1-supervisor-prompt.md`):

1. **5 mandatory docs first** (phase-2d-handover.md §0, phase-2d-close-summary.md, CLAUDE.md Forbidden Patterns + Renames to refuse on sight, ADR-006 §2.7.5 / §2.7.14 / §2.7.27 as relevant, phase-3-cluster-0-status.md).
2. **Territory** — explicit file paths, audit-first if scope uncertain.
3. **Smoke target** (kickoff smoke being unblocked) — or close-gate proof if a non-smoke audit/migration.
4. **Audit deliverables** when audit-first (file:line cites required, §-cite verification, ADR-fit confirmation, scope inventory).
5. **Close gate:** `cargo check --workspace --lib --tests` exit 0 (verified by EXIT CODE, not grep) + `bash scripts/verify-merge.sh > /tmp/vm.out 2>&1; echo SCRIPT_EXIT=$?` 12/12 PASS (CHECK-COMMS-1 file-redirect pattern; pipe-tail measurement bug) + `bash scripts/check-no-dynamic.sh` exit 0 + AGENTS.md row appended.
6. **Refuse-on-sight discipline** (the 9 items above, named explicitly in the dispatch prompt).
7. **No Co-Authored-By: Claude trailer** (MEMORY.md user rule).

**Merge resolution:** take-both on AGENTS.md row collisions + dispatch-table arm-line conflicts. Take-HEAD on test-attribute conflicts where one branch has more detailed §-cites. After any take-both pass: `cargo check --workspace --lib` MUST pass before commit.

**Verify-merge.sh measurement:** always file-redirect for exit capture per CHECK-COMMS-1. `bash scripts/verify-merge.sh > /tmp/vm.out 2>&1; echo SCRIPT_EXIT=$?`. Pipe-tail without `set -o pipefail` masks failures.

**Smoke matrix re-verification:** after every round merges, run all 4 kickoff smokes under both modes. Document VM and JIT output explicitly + identical-or-divergent classification + surface site if blocked. Same shape as the matrix table above. Update `phase-3-cluster-0-status.md`.

## In-flight state at handover

**Round 15 dispatch authorized + scoped** (per supervisor relay in Pre-handover update):

- **W17-narrow** (Option γ from W17-typed-object-arc Round 14 audit) — ~150-250 LoC classification-layer fix only, NO ADR amendment. Production-first. Surface-and-stop on any β typed-Arc carrier-shape question (defer to cluster-1).
- **W12-map-chained Option B** — producer-side carrier alignment, `.map()` returns raw `TypedArrayData<T>`. Audit-first ONLY to confirm Option B's scope is bounded (no unexpected ripple to other `.map()` consumers / other typed-array variants); audit decision Option B vs A/C is already made — refuse any audit recommendation of A or C. Production-first thereafter.

Both production-first; the audit phases for both landed in Round 14.

**Expected next action upon resuming:** dispatch both Round 15 sub-clusters in parallel from the post-Round-14-merge baseline. Standard close-gate. Surface to supervisor only if:
- Either sub-cluster's audit-confirmation surfaces unexpected ripple (W12-map-chained Option B scope expanding beyond `.map()` → typed-array fast-path),
- Either close report harbors a defection-attractor framing (the 9 items above),
- A third instance of the producer/consumer carrier-shape mismatch pattern surfaces (CLAUDE.md amendment trigger).

**After Round 15 merges:** run full 4-kickoff-smoke matrix under both VM and JIT. If all 4 produce identical correct output VM == JIT: cluster-0 close report → supervisor ratifies → user authorizes `phase-3-cluster-0-close` tag. If any smoke still diverges: surface-and-stop with the specific divergence + new Round 16 candidate scope.

## Decision authority pattern

You ARE authorized to:
- Run inline cite-audit before dispatch (Q1/Q2 precedent — kickoff smoke text verification, §-cite verification against ADR-006 / phase-2d-hardening.md).
- Propose round structure (parallel vs sequential, integrated trinity vs split, audit-first vs production-first) — supervisor ratifies.
- Coordinate AGENTS.md row updates + merge order + take-both resolution.
- Run reopen via SendMessage on a closed-but-not-merged sub-cluster when an audit gap is small + recoverable (W11-round-1 precedent).
- Refuse a sub-cluster's close report at merge time if it harbors a forbidden pattern (then surface to supervisor for reopen vs re-dispatch decision).
- Update `phase-3-cluster-0-status.md` + `AGENTS.md`.

You are NOT authorized without explicit supervisor approval:
- Dispatch new sub-cluster agents (supervisor ratifies dispatch scope + audit-first vs production cadence).
- Refuse defection-pattern framings on the agent's behalf at the meta-architectural level — refusing is the supervisor's call; you flag + surface.
- Authorize ADR amendments.
- Re-scope cluster boundaries (cluster-0 → cluster-1 reclassification, kickoff matrix changes, close criterion modifications).
- Tag `phase-3-cluster-0-close` (user authorizes after supervisor ratifies).

## User preferences + working style

- **No `Co-Authored-By: Claude` trailer in commits.** MEMORY.md rule.
- **Own all code quality.** Never frame as "pre-existing" — all code is the agent's responsibility once touched.
- **Plain code fences for relay text**, not blockquotes. The user copies relay blocks verbatim; blockquote `>` prefixes break paste.
- **Direct, concise communication.** Tight responses; substantive when needed; no padding.
- **Strategic owner / language designer.** Delegates architectural calls to the supervisor. Will surface explicitly on language-design or project-scope decisions.
- **Working in agent velocity.** Multi-sub-cluster-per-session cadence is expected.

## Operational continuity

1. **First action** after reading the 7 docs: post the 1-line confirmation + ingest the Round 15 dispatch authorization from the Pre-handover update section below.
2. **Dispatch Round 15** — W17-narrow + W12-map-chained Option B in parallel. Use the supervisor's last-relay scope verbatim (in Pre-handover update).
3. **Standard interaction pattern**: agent closes → you verify gate + read close report → you draft consolidated status → user relays to supervisor → supervisor responds → user pastes back → you execute.
4. **Don't re-derive context** that's already in `phase-3-cluster-0-status.md`, `CLAUDE.md`, or ADR-006.

**Most-likely-next-action:** dispatch Round 15 W17-narrow + W12-map-chained Option B audit-confirmation-then-production. After both close + merge, run full 4-kickoff-smoke matrix; if green, cluster-0 close report to supervisor.

---

## Pre-handover update (filled in 2026-05-13)

- **Round 14 W17-typed-object-arc audit-first status:** LANDED (audit-only close at `8ae56222`, scope-split per Option γ; merged into bulldozer at the Round 14 close commit)
  - Audit close commit: `8ae56222` (sub-cluster branch); merge commit on bulldozer in the Round 14 close commit (see HEAD below)
  - Audit doc: `docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md` (636 LoC, all 4 deliverables landed)
  - Audit findings: (a) ADR-005 §1 / ADR-006 §2.3 fit — classification-layer fix is in-shape, NO ADR amendment; broader typed-Arc carrier migration IS ADR amendment territory (β.1 / β.2 / β.3 options). (b) 17+ consumer inventory documented in audit doc §2 — `receiver_type_name:51-81` + legacy-fallback cascade `:572-612` with 5+ tag-bit predicates. (c) cross-crate boundary check clean for narrow scope (in-crate fix). (d) discipline checks passed. **β typed-Arc carrier-shape decision deferred to cluster-1 follow-up. Cluster-0 close does NOT block on β.**
  - Also: Round 13 T1' framing correction landed in audit §1.4 — T1' attributed surface to "Round 12 T2/T3 carrier migration"; audit corrects to "Round 12 T2/T3 migrated String only; TypedObject producer was surface-and-stopped; TypedObject producer TODAY emits raw `Box::into_raw(UnifiedValue<*const u8>)`". Classification gap real regardless.
  - In-scope production fix proposal for Round 15: W17-narrow (~150-250 LoC, classification-layer fix only, NO ADR amendment). Supervisor-ratified.

- **Round 14 W12-map-chained audit-first status:** LANDED (audit-only close at `8354968a`, with ~570 LoC conduit extension that exposes a new runtime surface)
  - Audit close commit: `8354968a`
  - Audit findings: (a) layer identified as conduit extension (similar shape to Round 11 trinity Part b); `concrete_types[doubled_slot] = Array(I64)` now flows through to `.sum()` receiver; JIT compilation succeeds end-to-end. (b) cluster-0 disposition confirmed: blocks kickoff Smoke 2 JIT. (c) NEW SURFACE uncovered: JIT-compiled code SIGSEGVs at runtime (exit 139) — `try_emit_v2_array_method` fast path at `crates/shape-jit/src/mir_compiler/v2_array.rs:367-387` (`jit_v2_array_sum_i64`) assumes `concrete_types[slot] = Array(elem)` implies raw `*const TypedArrayData<elem>` bits, but `.map()`'s `jit_call_method` dispatch returns a different carrier shape → invalid dereference. **Producer/consumer fast-path mismatch defection-attractor class** (2nd instance — `IntrinsicSum` / `.sum()` PHF split-brain was 1st).
  - WIP stashed at `stash@{0}` (next-step fix attempt before audit-only close).
  - In-scope production fix proposal for Round 15: Option B — producer-side carrier alignment (`.map()` returns the raw `TypedArrayData<T>` shape the fast path expects). Supervisor-ratified. Refused Option A (consumer-side narrowing alone — insufficient for cluster-0 close, either defection-fallback or surface-and-stop without unblocking) and Option C (scope expansion; consumer-side defense-in-depth is cluster-1 territory).

- **Open supervisor questions / pending relays:** none. All Round 14 dispositions ratified. Round 15 dispatch authorized — fresh team-lead session executes per the supervisor relay attached below.

- **New defection-attractor surfaces noticed during the rolling team-lead session not in CLAUDE.md yet:** "producer/consumer fast-path mismatch / parallel-implementation across carrier-shape boundaries." Two instances so far (`IntrinsicSum` / `.sum()` PHF; `.map()` return-shape / typed-array fast-path). Recurrence threshold is 3; track for CLAUDE.md amendment if a third surfaces. Cluster-1 hardening territory in either case.

- **Latest `phase-3-cluster-0-status.md` HEAD commit:** [team lead to fill in after merge]

- **Last supervisor relay summary — Round 15 dispatch authorization:**

  Two sub-clusters in parallel from post-Round-14 baseline:

  - **W17-narrow (Option γ):** ~150-250 LoC classification-layer fix, NO ADR amendment. Production-first. Surface-and-stop on any β carrier-shape question (defer to cluster-1). Standard Round-3-pattern close gate (5 mandatory docs, `cargo check --workspace --lib --tests`, `verify-merge.sh` 12/12, `check-no-dynamic.sh` exit 0, AGENTS.md row). Refuse on sight: bool-default for unproven receiver-type kind, bridge/probe/helper/hop framing for the classification-layer migration, "preserve NaN-box decode for one edge case" framing. Unblocks: kickoff Smoke 3 JIT.

  - **W12-map-chained Option B:** producer-side carrier alignment — `.map()` returns the raw `TypedArrayData<T>` shape `try_emit_v2_array_method` fast path expects. Audit-confirmation-only at dispatch start (~confirm Option B scope is bounded; no unexpected ripple to other `.map()` consumers or other typed-array variants); production thereafter. **Refuse any audit recommendation of Option A or Option C** — Option B is supervisor-ratified, the audit's role is scope-confirmation, not re-decision. Standard close gate. Refuse on sight: bool-default for unproven `.map()` return-shape kind, defensive fallback in consumer (Option A.1 shape), bridge/probe/helper/hop framing for the producer alignment, "preserve mismatched carrier for one edge case" framing. Unblocks: kickoff Smoke 2 JIT.

  Cluster-0 close attempt after Round 15 merges + full 4-kickoff-smoke matrix passes VM == JIT. If matrix green: cluster-0 close report → supervisor ratifies → user authorizes `phase-3-cluster-0-close` tag. If any smoke diverges: surface-and-stop with the specific divergence; Round 16 scope follows the same N+1 discipline.

  Recurrent-pattern note for the team lead (not for the agent prompts): if Round 15 W12-map-chained Option B's scope-confirmation surfaces a third instance of the producer/consumer carrier-shape mismatch class, that's a CLAUDE.md amendment trigger — surface to supervisor with the structured shape (the two prior instances + the new one).

---

*End of handover. Read §First action before any dispatch.*
