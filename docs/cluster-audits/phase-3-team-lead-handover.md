# Phase 3 cluster-0+1 — Team-lead handover (cluster-0+1-close seam; tag authorization pending)

**Generated:** 2026-05-14 (cadence-shift rotation); updated 2026-05-15 (V3-S5-complete + V3-S6 surface rotation); updated 2026-05-16 (cluster-0+1-close seam — V3-S6 5-checkpoint chain MERGED at `50e5c024`; cluster-0+1-close-criterion gates ALL MET under β2 disposition; tag authorization pending supervisor ratification + user authorization).
**Successor handoff point:** V3-S6 retroactive 5-checkpoint multi-session chain (V3-S6a 43ac9b7a + V3-S6b 48e05f3f + V3-S6c 2544f89f + V3-S6d 2f011e93 + V3-S6e d4d5454c) MERGED into `bulldozer-strictly-typed` canonical at `50e5c024`. Smoke matrix 3/4 VM == JIT at canonical fixture (Smoke 2 JIT TIMEOUT rc=124 in V3-S6f territory folds to cluster-2 per β2 supervisor disposition 2026-05-16). All cluster-0+1-close-criterion gates MET. Reading 2 empirically confirmed at V3-S6e: V3-S6 chain produced real architectural work (V3-S5 conduit completion at compile-time layer) + V3-S6f is the next architectural layer (runtime-execution within specialized body) belonging to cluster-2's broader §2.7.5 JIT-side conduit completion. New team-lead's first execute action: receive supervisor's tag authorization relay + apply `phase-3-cluster-0-close` + `phase-3-cluster-1-close` tags on canonical at `50e5c024` + dispatch cluster-2 audit-day per supervisor's next-phase disposition.
**Predecessor team-lead session:** rotated at the cluster-0+1-close-seam after V3-S6 chain merge + status doc cluster-0+1 close subsection landing.
**Supervisor session:** see `docs/cluster-audits/phase-3-supervisor-handover.md` for supervisor-side state.

## Your role this session

You are the **team lead** for Phase 3 cluster-0+1 of the Shape language refactor under the **bulldozer cadence**. Job:

1. Operate the `Agent` tool to dispatch wave agents per supervisor relays.
2. Verify close gates (`cargo check --workspace --lib --tests`, `bash scripts/verify-merge.sh`, `bash scripts/check-no-dynamic.sh`, AGENTS.md row).
3. Merge wave-agent branches into `bulldozer-strictly-typed`. Take-both for AGENTS.md row + dispatch-table arm collisions is established; the bulldozer cadence will produce MORE collisions per wave (parallel deletion across multiple agents), so take-both attention is heightened.
4. Run smoke-matrix verification under both `--mode vm` and `--mode jit` after every wave merges.
5. Update `docs/cluster-audits/phase-3-cluster-0-status.md` after each wave.
6. Surface architectural questions to the supervisor via the user (strategic-owner relays).

The supervisor is a separate Claude instance — the user copies their relays into your session, you copy your responses back to the user, the user pastes them to the supervisor. Do **not** make architectural calls yourself (ADR amendments, wave-scope changes, defection-pattern refusals at the meta layer, cluster-tag authorization); surface them to the supervisor.

## Cadence shift — bulldozer waves (load-bearing)

**Strategic-owner authorization 2026-05-14:** the audit-first sub-cluster cadence (R7-R20) is replaced by the bulldozer-wave cadence for the remaining cluster-0 + cluster-1 deletion targets. Reasoning (per user prompt 2026-05-14): "code removal is the best way to easier readability... at least 50% [of the surface] is outdated/wrong architecture/dead or only used by also outdated paths." The audit-first cadence preserved attractors in source for months (W17-typed-carrier-bundle-A dead arms ~2 months); session count expansion (R7-R20 added 6+ sessions vs original 10-15 estimate) is not justified by bug-prevention math.

**Bulldozer cadence shape:**

```
Wave 1 — Single audit-day (1 session, 1 agent)
Wave 2 — Parallel bulldoze (1-2 sessions, 6-8 agents in parallel)
Wave 3 — Stabilize + cluster-0+1 close (1 session)
```

Total: 3-4 sessions to cluster-0 + cluster-1 close. v1 trajectory becomes ~6-9 sessions remaining (Wave 1+2+3 + cluster-2 cleanup + Phase 4), not 11-16.

**Discipline preserved verbatim:**

- All CLAUDE.md Forbidden Patterns + Renames to refuse on sight + Parallel-implementation entry
- All ADR-006 §2.7.x rulings (4-table HeapKind lockstep, §2.7.5 stamp-at-compile-time, §2.7.6/Q8 carrier-API-bound, §2.7.7/Q9 stack parallel-kind, §2.7.8/Q10 cell-storage parallel-kind, §2.7.10/Q11 method-dispatch ABI, §2.7.11/Q12 value-call ABI)
- ADR-005 §1 single-discriminator + §2 String exception
- 5-arm receiver-recovery soundness rule (W13→W16 lesson)
- `verify-merge.sh` 12/12 gate
- `check-no-dynamic.sh` exit 0 gate
- No Co-Authored-By trailers (MEMORY.md rule)
- Own all code quality (MEMORY.md rule)
- surface-and-stop discipline for genuine architectural gaps

**What changes (cadence, not discipline):**

- Parallel deletion across multiple wave-2 agents; per-agent territory bounded; merge ceremony loud.
- Cluster-1 deletion targets dispatched IN cluster-0 wave (HashMapValueBuf, O-3/O-3a TypedObjectStorage/TraitObjectStorage HeapHeader migration, IntrinsicSum split-brain). Strategic-owner authorized; not "scope creep."
- O-3.a (TypedObjectStorage HeapHeader migration) lands in Wave 2 as one parallel agent (audit §4.3 estimated "multi-week" assumed sequential careful work; parallel + verify-merge.sh gate is a different cost calculation; surface-and-stop if it surfaces something genuinely novel).
- ADR amendments fold into Wave 2 merge commits as needed (Q25.A SUPERSEDED already landed at R20 (c); Q25.B HashMapValueBuf deletion + ADR-006 §4.3 O-3 amendments land in Wave 2).
- **Refuse on sight in wave reports**: "preserve carrier X for cluster-1+" / "this deletion target needs its own audit sub-cluster" / "multi-week scope is too risky for one wave" / "defer this to cluster-1.5 after cluster-0 close" — all become the audit-first cadence we deleted. The cadence shift is the explicit refusal of that framing.

## First action — read these in order

1. **`docs/cluster-audits/phase-3-supervisor-handover.md`** — supervisor-side state + cadence-shift context. Read first.
2. **`docs/cluster-audits/phase-3-cluster-0-status.md`** — canonical state (post-R20 close update).
3. **`docs/cluster-audits/phase-3-kickoff-prompt.md`** — original supervisor contract; the canonical kickoff smokes at lines 89-110. Surface A (fixture-vs-prose drift) is the user-pending disposition.
4. **`docs/cluster-audits/phase-2d-handover.md` §0** — discipline rules (forbidden patterns, 4-table lockstep, 5-arm receiver-recovery, surface-and-stop discipline). Carry forward unchanged.
5. **`CLAUDE.md`** — Forbidden Patterns + Renames to refuse on sight + Parallel-implementation entry + Single-discriminator (ADR-005) + Value & memory model (ADR-006) + Mechanical enforcement. Compacted at 2026-05-14 user authorization (35.6k chars, was 44.9k); rules preserved verbatim.
6. **`AGENTS.md`** — live roster (R5-R20 rows at bottom).
7. **`docs/adr/006-value-and-memory-model.md`** §2.3 / §2.7.5 / §2.7.14 / §2.7.22 / §2.7.24 (Q25.A SUPERSEDED text from R20 (c)) / §2.7.27 / §4.3 (O-3 / O-3a obstacles).
8. **`docs/cluster-audits/w12-typed-array-data-deletion-audit.md`** — R17 cluster-0-transition audit + R19 / R20 amendments. The deletion target list (§2) IS the Wave 2 scope.
9. **This file's "In-flight state" + "Wave 1 dispatch shape" sections at bottom.**

Post a 1-line confirmation: *"Read 9 mandatory docs; team-lead role ready under bulldozer cadence. Current state: <one sentence>."*

## Current state at handover

**Branch HEAD at rotation:** `bulldozer-strictly-typed @ 14494605` — reflects γ merge only. Predecessor ceremony OWED: S2-prime-production audit-only merge (RATIFIED by supervisor R20; branch at `10cd1a56`) + R20 status-doc close commit + 5 annotations. **Predecessor team-lead held on S2-prime-production merge pending supervisor disposition on Surfaces A/B/C/D; that hold was over-cautious — Surface B (merge) was RATIFIED standalone in the R20 relay, not conditional on Surfaces A/C/D.** Under bulldozer cadence (refuse #10) the "wait for every disposition before any action" pattern IS the audit-first attractor we're refusing. Your first execute action after reading docs + posting confirmation is the predecessor ceremony.

Smoke matrix at HEAD `14494605`:

| Smoke | VM | JIT | Cluster-0 criterion |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | ✓ |
| 2 (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ✅ 30 | ❌ rc=1 | gated on S5 (Wave 2 unblocks) |
| 3 (canonical fixture `let t = X{}`) | ✅ x | ✅ x | ✓ post-γ |
| 4 (Set + .add + .size) | ✅ 2 | ✅ 2 | ✓ |

**3 of 4 kickoff smokes pass VM == JIT at canonical fixture.** Smoke 2 unblocks post-Wave-2 (TypedArrayData enum + TypedBuffer<T> wholesale deletion eliminates dual-carrier reality, the R14 conduit blocker).

**Smoke 3 fixture-vs-prose drift (Surface A):** fixture passes; kickoff prompt prose at lines 102-105 (`let t: dyn T = box(X{})`) requires Q25.C TraitObject rebuild. **User-pending disposition** — see §In-flight state for the three options. Wave 1 audit-day maps all three; Wave 2 scope conditionals on the user's answer.

**Cumulative through R20:** ~33+ sub-clusters across 20 rounds, ~3 / session steady cadence. **Bulldozer-cadence target:** Wave 1 + Wave 2 + Wave 3 close cluster-0 + cluster-1 in 3-4 sessions.

## Discipline rules (load-bearing — refuse on sight)

Read CLAUDE.md for full lists. The supervisor refuses these framings at the relay layer; you refuse them at the wave-agent dispatch + close-report review layer:

1. **Partial-close / declare-victory at any artifact-tagging layer.**
2. **"Pre-existing" as a disposition.** Own all code quality.
3. **Bool-default for unknown kind.** §2.7.7 / §2.7.8 #4 — `NotImplemented(SURFACE)` with §-cite.
4. **"Bridge/probe/helper/hop/translator/adapter/shim" framings.** CLAUDE.md broader-family regex.
5. **Kind-blind dispatch / NaN-box decode at FFI boundaries.** §2.7.5 stamp-at-compile-time.
6. **Silent fallback / no-op with "tracked as follow-up" framing.** W11-round-1 walk-back precedent.
7. **All "Forbidden rationalizations"** in CLAUDE.md.
8. **Resurrecting deleted shape under renamed alias.**
9. **Parallel-implementation across producer/consumer carrier-shape boundaries.** 8 instances cluster-0 logged; pattern is real.

**New refusal under bulldozer cadence (added 2026-05-14):**

10. **"Preserve X for cluster-1+" / "needs its own audit sub-cluster" / "multi-week scope is too risky" / "defer to cluster-1.5" / any framing reverting bulldozer cadence to audit-first.** The cadence shift is the explicit refusal of deferral framings within the bulldozer wave scope. If a deletion target surfaces a genuine architectural gap not addressable in-wave, surface-and-stop with structured shape (§-cite, file:line, why in-wave dispatch isn't viable); supervisor disposes whether to extend the wave or genuinely defer.

When you spot one in an agent's close report: **do not merge.** Surface to supervisor with structured shape.

## Wave 1 dispatch shape — Single audit-day

**Dispatch prompt template** (paste verbatim into Agent tool with `subagent_type="general-purpose"` after supervisor ratifies):

```
You are a Phase 3 cluster-0+1 sub-agent under the BULLDOZER CADENCE. Your sub-cluster
is Wave-1-single-audit-day.

Your job is to produce ONE comprehensive deletion-inventory audit document mapping all
remaining cluster-0 + cluster-1 deletion targets in a single dispatch. NO per-target
audit sub-clusters; NO speculative "needs another audit" disposition. Single audit doc;
single agent; 1-2 days; comprehensive ground-truth coverage.

YOU MUST read these docs in order before touching the codebase:

1. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-2d-handover.md §0
2. /home/dev/dev/shape-lang/shape/CLAUDE.md (Forbidden Patterns + Renames to refuse on
   sight + Parallel-implementation entry + ADR-006 key rules)
3. /home/dev/dev/shape-lang/shape/docs/adr/006-value-and-memory-model.md §2.3 / §2.7.5 /
   §2.7.14 / §2.7.22 / §2.7.24 (Q25.A SUPERSEDED) / §4.3 (O-3 / O-3a)
4. /home/dev/dev/shape-lang/shape/docs/cluster-audits/w12-typed-array-data-deletion-audit.md
   (this is the R17 audit; your job is the next-level inventory consuming it)
5. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-3-cluster-0-status.md (R20
   close state)
6. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-3-team-lead-handover.md
   §In-flight state (the deletion-target list below is your starting scope)

BINDING SUPERVISOR DISPOSITIONS (carry into your design analysis):

- R20 prereq 1 disposition: +8 typed opcodes per kind for String + Decimal producer
  migration (mirror S1 scalar recipe per audit §3.1). Re-shape op_new_array path is
  REFUSED as too-easily-defection-attractor-adjacent (any single change that makes
  the operand element-kind reader read from heap header instead of operand silently
  violates §2.7.5 stamp-at-compile-time).
- R20 prereq 3 disposition: VM Decimal SIGSEGV at baseline is pre-existing v2-raw-
  heap aliasing class (status doc known constraint). NOT a Wave 2 blocker; document
  that the migration moves Array<decimal> from legacy carrier (broken) to v2-raw
  carrier (may surface same bug class); v2-raw-heap-audit is the cross-cutting fix
  folded into Wave 2 if territory permits OR Wave 3 stabilize-fix.
- Surface A all three options (a)/(b)/(c): map without blocking. Supervisor
  recommendation (c) split — but user-pending. Your audit deliverable (I) covers
  all three scope estimates.

YOUR TERRITORY: produce
docs/cluster-audits/bulldozer-wave-1-inventory.md
with sections per deletion target. NO source changes. NO partial migration. NO new
infrastructure landed. Audit-only deliverable.

DELETION TARGETS (your scope — every one of these must have file:line cite coverage):

A. TypedArrayData enum (crates/shape-value/src/heap_value.rs:~2877-3052)
   - Enumerate every producer construction site by file:line (current grep ~25 sites)
   - Enumerate every consumer match arm by file:line (current grep ~120 sites)
   - Per-arm migration target per audit §2 (TypedArray<*const X>Obj for heap-element;
     existing TypedArray<T> for scalars; HeapKind::Matrix=34 / MatrixSlice=35 for the
     category-error arms)
   - The 4 dead arms (DateTime/Timespan/Duration/Instant) verify zero producers + zero
     reachable consumers by ground-truth grep

B. TypedBuffer<T> + AlignedTypedBuffer wrapper layer
   (crates/shape-value/src/typed_buffer.rs entire file ~485 LoC)
   - Enumerate every reference by file:line
   - Confirm zero deletion blockers post-TypedArrayData migration

C. HashMapValueBuf enum (audit §5 — Q25.B parallel deletion target)
   - Enumerate every producer + consumer site by file:line
   - Per-V monomorphization migration design (HashMapData<V> generic vs inline element-
     kind tag — pick one; cite ADR-005 §1 if pertinent)
   - DateTime/Timespan/Duration/Instant dead arms verify zero producers

D. TypedObjectStorage Arc → HeapHeader migration (audit §4.3 O-3)
   - Every TypedObject construction site by file:line
   - Field-access fast-path locations in JIT (crates/shape-jit/src/)
   - Drop dispatch sites
   - Refcount semantics audit (Arc<TypedObjectStorage> → HeapHeader+0 offset migration)
   - "Multi-week scope" framing in audit §4.3 is sequential-careful-work cost; with
     one parallel agent + verify-merge.sh + 4-table lockstep gate, restate the cost as
     "1 parallel agent wave-2 territory" (or surface-and-stop if genuinely intractable
     in-wave)

E. TraitObjectStorage HeapHeader migration (audit §4.3 O-3a)
   - Same shape as D, fat-pointer variant
   - Vtable refcount-share analysis

F. W17-typed-carrier-bundle-A Q25.A specialization dead arms
   - DateTime/Timespan/Duration/Instant TypedArrayData arms (zero producers per R20
     S2-prime ground truth)
   - HashMapValueBuf temporal/instant arms (same shape per audit §4.1.A.3)
   - Wholesale deletion targets; no producer migration needed

G. W12-stdlib-intrinsic-collapse (IntrinsicSum / .sum() PHF split-brain)
   - Map every IntrinsicSum opcode/handler reference
   - Map every .sum() PHF method handler reference
   - Migration design: collapse to one path (cite ADR-005 §1 if pertinent)
   - This is the 7th defection-attractor instance from R18 close; cluster-2 candidate
     in original plan, folded into bulldozer wave per cadence shift

H. Cross-tier shape-conversion design for Array<string> / Array<decimal> v2-raw
   read path (the R20 S2-prime prereq 2)
   - Materialize-on-read (clone inner Arc<String> back into NativeKind::String at
     read) vs push-pointer-shape (new bits + kind tuple) vs new NativeKind variant
     (StringV2 / DecimalV2 — Q8 cardinality amendment)
   - Per-shape cascade count + ADR-006 amendment shape if needed
   - Read-cost estimate per shape
   - Recommended design + ratification gate (does it need supervisor ADR amendment,
     or is it covered by existing §2.7.5 / §2.7.6 / Q8?)

I. Surface A — kickoff-prompt-vs-fixture mismatch
   - Map current /tmp/smokes/s3.shape (fixture: `let t = X{}` UFCS dispatch)
   - Map kickoff prompt prose at phase-3-kickoff-prompt.md:102-105 (`let t: dyn T =
     box(X{})` trait-object dispatch through Q25.C HeapKind::TraitObject)
   - Audit Q25.C TraitObject rebuild scope IF Surface A user-decision is (b)
     (rebuild pre-cluster-0-close): VTable thunk additions per Q25.C.5, Self-arg
     runtime check per Q25.C.2, generic-method type-info per Q25.C.3, ETO-001/ETO-002
     error generation, etc. Estimate agent-territory scope.
   - Document the cluster-1.5 split shape IF Surface A user-decision is (c)
     ("Smoke 3-trait-object" added as cluster-1.5 close-criterion item)
   - Document the silent-rescope shape IF Surface A user-decision is (a)
     (fixture replaces prose; status doc + kickoff prompt + AGENTS.md updates)

J. The 23+ shape-jit #[ignore]'d tests (status doc Known Constraints)
   - Map each test file:line
   - Per-test disposition: rewire (test is salvageable) vs delete (test asserted
     deleted shape) — fold-into-wave-2 disposition
   - The 5 modules behind `deep-tests` feature gate stay gated (root-cause perf
     work tracked separately); this scope is the per-test #[ignore]'s only

K. The 48 shape-test pre-existing failures (status doc Known Constraints)
   - Map by category (generic-fn instantiation Null, typed-closure inference,
     array transformation, string .join, window functions, array slice/sort/some,
     destructuring rest)
   - Per-category disposition: cluster-2 audit triage (most likely) vs in-wave-2
     fixable
   - This may surface as too-large-for-Wave-2 — surface-and-stop is fine; supervisor
     disposes whether the 48 fold into Wave 2 or become cluster-2 audit territory

L. Wave 2 agent partition recommendation
   - Given the A-K inventory, propose 6-8 agent territories with file-set non-
     overlap so parallel dispatch is viable
   - Per-agent: territory file glob + responsibility + close gate + AGENTS.md row
     shape + ADR amendments owed in close commit
   - Per-pair territory intersection check (if two agents touch the same file, name
     it explicitly and propose merge-ceremony shape)

AUDIT-FIRST DELIVERABLES (per CLAUDE.md surface-and-stop discipline applied
recursively):
- File:line cites required for every claim ("zero producers for X" means actual
  grep output, not predicted via reasoning)
- ADR-fit confirmation per migration design (cite the § paragraph)
- Cascade-site count per change shape (the R19 S1.5 ~100-site ceiling shape is the
  upper bound; surface-and-stop if any single migration exceeds)
- Pre-flight ground-truth check: every audit claim grep-verified against actual
  source at HEAD (the 5-instance supervisor-/audit-layer imprecision pattern is the
  signal to verify EVERY ground-truth claim before landing)

FORBIDDEN IN THIS DISPATCH (refuse on sight):
- "Preserve X for cluster-1+" / "needs its own audit sub-cluster" / "multi-week
  scope" / "defer to cluster-1.5 post-close" — bulldozer cadence refuses these
- Resurrecting ValueWord / tag_bits / W-series dispatch shapes under any rename
- Bool-default fallback for unknown kind
- bridge/probe/helper/hop/translator/adapter/shim descriptors per CLAUDE.md
  broader-family regex
- Parallel-implementation framings ("documented intentional duality" /
  "preserve both carriers" / "carrier unification via boundary deletion as
  one-off patch")
- Re-introducing JitArray as parallel discriminator to TypedArrayData
- Audit-text imprecisions that lack file:line ground truth (R20 S2-prime caught
  the rust_decimal align-of=8 vs measured align-of=4 imprecision via ground-
  truthing; same discipline applies here)
- "Single deletion target requires CLAUDE.md modification" without flagging it
  explicitly — if any deletion target's design surfaces a NEW forbidden pattern or
  refuse-on-sight phrase that should land in CLAUDE.md, FLAG IT EXPLICITLY in your
  close report. CLAUDE.md modifications require explicit user authorization (R17
  precedent + 2026-05-14 compaction precedent); team-lead does not land them
  without user ratification.

CLOSE GATE:
- bulldozer-wave-1-inventory.md exists with A-L sections; every section has
  file:line cites + per-deletion-target migration design + cascade-site count
  estimate
- AGENTS.md row appended (audit-only ceremony)
- No source changes (audit-only; no .rs / .toml / .lock modifications)
- bash scripts/verify-merge.sh exit 0 (audit-only doc-record close per W17-narrow
  + S2 + R20 S2-prime precedent)
- Status doc R20-close subsection updated with Wave 1 dispatch + close summary
- NO Co-Authored-By: Claude trailer

When you finish, commit on bulldozer-strictly-typed-wave-1-audit branch with a clear
message. Then report back with:
1. Branch name + close commit hash
2. Output of bash scripts/verify-merge.sh (last 20 lines)
3. Each deletion target A-L: status (mapped + designed / surface-and-stopped /
   intractable in-wave)
4. Wave 2 agent partition recommendation (concrete agent count + territories)
5. ADR amendments owed in Wave 2 close commits (per-amendment file:line + draft
   text shape if substantive)
6. Surface A coverage: all three (a)/(b)/(c) options mapped per the audit
7. Any genuinely intractable deletion-target that requires supervisor ADR-level
   decision before Wave 2 dispatches (surface-and-stop with structured shape)
8. CLAUDE.md modifications surfaced (if any) — flag the proposed change + draft
   text + which deletion target requires it. Team-lead surfaces to supervisor for
   supervisor-to-user-relay; user ratifies landing.
```

After Wave 1 closes: team-lead reads audit doc + verify-merge.sh; status-doc R20+1 update; surface to supervisor with the Wave 2 dispatch shape for ratification.

## Wave 2 dispatch shape — Parallel bulldoze (after Wave 1 ratifies)

Per Wave 1's audit recommendation (L), 6-8 parallel agents land deletion + migration in coordinated waves. Each agent is dispatched with:

- Territory file-set (Wave 1 (L) names exclusive file glob per agent)
- Close gate (`cargo check --workspace --lib --tests` exit 0 + `verify-merge.sh` 12/12 + `check-no-dynamic.sh` exit 0 + AGENTS.md row + ADR amendments in close commit)
- Refuse-on-sight discipline (the 10 items above, all named in the dispatch prompt)
- Parallel-coordination shape: at merge time, take-both for AGENTS.md row + dispatch-table arms + ADR-006 amendment text; verify-merge.sh 12/12 after every merge

Provisional agent partition (Wave 1 (L) refines):

- **Agent A — TypedArrayData enum + TypedBuffer<T> + AlignedTypedBuffer wholesale deletion.** Territory: `crates/shape-value/src/heap_value.rs:2877-3052` + `typed_buffer.rs` + consumer cascade in `crates/shape-vm/src/executor/objects/`. Close gate includes all consumer match-arms cascaded.
- **Agent B — String + Decimal producer migration to v2-raw TypedArray<*const StringObj/DecimalObj>.** Territory: `crates/shape-vm/src/compiler/expressions/collections.rs` + `compiler/typed_emission.rs` + `executor/v2_handlers/array.rs` + JIT FFI in `crates/shape-jit/src/ffi/v2/mod.rs` + opcode definitions in `shape-vm/src/opcodes/`. Coordinates with Agent A on `op_new_array` shape change (per Wave 1 (H) read-side design).
- **Agent C — HashMapValueBuf wholesale deletion + per-V monomorphization migration.** Territory: `crates/shape-value/src/heap_value.rs` HashMapValueBuf definition + every HashMap producer/consumer site. Per Wave 1 (C) design.
- **Agent D — TypedObjectStorage Arc → HeapHeader migration (O-3.a).** Territory: `crates/shape-value/src/heap_value.rs::TypedObjectStorage` + every TypedObject construction site + JIT field-access fast path in `crates/shape-jit/src/mir_compiler/`. Per Wave 1 (D) design.
- **Agent E — TraitObjectStorage HeapHeader migration (O-3a).** Territory: `crates/shape-value/src/heap_value.rs::TraitObjectStorage` + vtable refcount paths. Per Wave 1 (E) design.
- **Agent F — Q25.A/Q25.B specialization dead arms wholesale deletion.** Territory: the 4 dead TemporalData/Instant arms in TypedArrayData + the parallel HashMapValueBuf arms. Audit-confirmed zero producers, zero reachable consumers. Should be the smallest agent territory.
- **Agent G — W12-stdlib-intrinsic-collapse.** Territory: `crates/shape-runtime/src/stdlib/intrinsics/` IntrinsicSum + `crates/shape-vm/src/executor/objects/` `.sum()` PHF handlers. Per Wave 1 (G) design.
- **Agent H (conditional on Surface A (b)) — Q25.C TraitObject rebuild.** Territory: VTable thunk additions + Self-arg runtime check + generic-method type-info + ETO-001/ETO-002 errors. Per Wave 1 (I) Q25.C scope estimate.

Wave 2 dispatches in 2 rounds if Wave 1 (L) suggests inter-agent file overlap that requires staging (e.g. Agent A + Agent B both touch consumer cascade in executor/objects/ — stage A first, then B), OR all 7-8 agents in parallel if file territories are clean.

## Multi-session chain pattern (D-α; user-ratified 2026-05-14)

For atomic-lockstep cascades that exceed single-LLM-session execution capacity
(ceiling-c per Round 3a D3 finding: ~50-100 non-mechanical edits per session
at discipline-coherent quality bar), dispatch uses the multi-session sub-agent
chain pattern. See `docs/cluster-audits/bulldozer-multi-session-chain-pattern.md`
for the full discipline doc (authority, operational shape, structured state
pointer, discipline preserved + relaxed bounds, forbidden under pattern,
recovery from sub-agent failure, velocity expectation).

Pattern instantiated for D4 (TypedObjectStorage Arc→raw cascade ~270-320 sites)
per supervisor Round 3a' close 2026-05-14 + user 2026-05-14 D-α ratification.
Dynamic chain authorization: team-lead may extend chain length if a sub-agent
surface-and-stops mid-scope (no per-instance supervisor authorization needed
for chain progression).

## Wave 3 dispatch shape — Stabilize + close (after Wave 2 ratifies)

After Wave 2 merges:

1. Kickoff smoke matrix re-verification VM == JIT (all 4 or 5 smokes per Surface A disposition)
2. Status doc cluster-0 + cluster-1 close summary
3. ADR-006 master amendment commit consolidating wave-merge amendment scattering (if Wave 2 produced enough amendment text that consolidation is warranted; otherwise skip)
4. Cluster-0+1 close report → supervisor ratifies → user authorizes `phase-3-cluster-0-close` + `phase-3-cluster-1-close` tags

After cluster-0+1 close:
- Cluster-2 cleanup (per-HeapKind kinded jit_print, compile-time-boxed string-constant leak, W12-collection-constructor-mir-lowering, the 48 shape-test pre-existing failures triage if not folded into Wave 2)
- Phase 4 (trait Add/AddAssign for user types per existing scope)
- v1 close attempt

## Dispatch cadence + close-gate shape

**Wave dispatch prompt template** (mirrors `phase-3-kickoff-prompt.md` + bulldozer cadence additions):

1. **6 mandatory docs first** (phase-2d-handover.md §0, CLAUDE.md, ADR-006 sections, status doc, kickoff prompt, audit doc).
2. **Territory** — explicit file paths from Wave 1 (L) partition.
3. **Close gate** — `cargo check --workspace --lib --tests` exit 0 (EXIT CODE, not grep) + `bash scripts/verify-merge.sh > /tmp/vm.out 2>&1; echo SCRIPT_EXIT=$?` 12/12 PASS (CHECK-COMMS-1 file-redirect) + `bash scripts/check-no-dynamic.sh` exit 0 + AGENTS.md row appended + ADR amendments in close commit.
4. **Refuse-on-sight discipline** (the 10 items above, named explicitly in the dispatch prompt — especially the new #10 anti-deferral rule).
5. **No Co-Authored-By: Claude trailer** (MEMORY.md user rule).
6. **Cascade-surface-and-stop ceiling** at ~100 sites per single migration (R19 S1.5 precedent — bulldozer cadence preserves this fallback).

**Merge resolution:** take-both for AGENTS.md rows + dispatch-table arms + ADR-006 amendment text. Take-HEAD for test attributes where one branch has detailed §-cites. After any take-both pass: `cargo check --workspace --lib` MUST pass before commit.

**Verify-merge.sh measurement:** always file-redirect for exit capture per CHECK-COMMS-1.

**Smoke matrix re-verification:** after every wave merges, run all 4 (or 5) kickoff smokes under both modes. Update `phase-3-cluster-0-status.md`.

**Manual worktree creation** (avoid Agent `isolation:` parameter — known defect at R15 W17):

```
git -C /home/dev/dev/shape-lang/shape worktree add \
  /home/dev/dev/shape-lang/shape-wave-<N>-<slug> \
  -b bulldozer-strictly-typed-wave-<N>-<slug> <base-commit>
```

Run cargo / verify-merge.sh via `devenv shell --quiet -- bash -c "cd <worktree-path> && <command>"` from `/home/dev/dev/shape-lang/`.

## In-flight state at handover — cluster-0+1-close seam (2026-05-16)

**Canonical HEAD:** `bulldozer-strictly-typed @ 50e5c024` — V3-S5 architectural sunset COMPLETE at 9523d57a + V3-S6 retroactive 5-checkpoint multi-session chain MERGED via take-both ceremony. V3-S6 chain ancestors integrated: V3-S6a 43ac9b7a (VM-side resolver Phase B + closure-return-typed generic monomorphization + Vec<> conduit + parametric Array method classifier ~435 LoC) + V3-S6b 48e05f3f (PATH α side-table infrastructure on BytecodeProgram/Program/LinkedProgram + linker threading + population at try_monomorphize_method_call ~370 LoC) + V3-S6c 2544f89f (PATH α-prime JIT routing block at terminators.rs:176 bypassing jit_call_method trampoline + handle_int_map ckpt3_surface SIGSEGV class +256/-88 LoC) + V3-S6d 2f011e93 (complementary stamping restoration at helpers.rs +58/-31 LoC) + V3-S6e d4d5454c (Gap B+C combined fix: local_typed_array_element_types on MirFunction + lowering producer + helpers.rs consumer ~244 LoC). Total V3-S6 chain ~1500 LoC integrated.

**Cluster-0+1 close criterion (β2 supervisor disposition 2026-05-16):** MET

**Smoke matrix at canonical 50e5c024:**

| Smoke | VM | JIT | Cluster-0+1 close criterion (β2) |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | ✓ |
| 2 (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ✅ 30 | ❌ TIMEOUT rc=124 (V3-S6f runtime infinite-loop in specialized Vec.map body) | cluster-2 fold per β2 |
| 3 (canonical fixture) | ✅ x | ✅ x | ✓ |
| 4 (Set + .add + .size) | ✅ 2 | ✅ 2 | ✓ |

**3/4 VM == JIT at canonical fixture.** Smoke 2 JIT runtime-execution gap folds to cluster-2 as canonical fixture for broader §2.7.5 JIT-side conduit completion work.

**All cluster-0+1-close-criterion gates met under β2:**

- ✓ Q25.A SUPERSEDED (R20 + V3-S5) + Q25.B SUPERSEDED (Round 3b)
- ✓ Path B canonical Ptr-newtype (D4); Audit §4.3 O-3.a + O-3a RESOLVED (D4)
- ✓ Producer-side foundation (Wave 3 R1 V3-A2-followup); hashmap-value-v-arm → cluster-2 fold
- ✓ V3-S5 wholesale enum + buffer deletion (Framing (ii) live-grep 0/0/0 LIVE .rs preserved; 1 pre-existing markdown audit doc hit at V2_METHOD_DISPATCH_AUDIT.md:252 from phase-1b-vm commit 26f30d98)
- ✓ JIT FFI String/Decimal build (Group X option (a))
- ✓ V3-S6 retroactive 5-checkpoint chain MERGED (V3-S6a/b/c/d/e folded into canonical at 50e5c024)
- ✓ `cargo check --workspace --lib --tests` EXIT=0
- ✓ `just check-clean` EXIT=0
- ✓ `bash scripts/verify-merge.sh` 12/12 PASS EXIT=0
- ✓ `bash scripts/check-no-dynamic.sh` EXIT=0
- ✓ 4-table HeapKind::TypedArray lockstep 0/4 LIVE arms (variant identifier preserved per V3-S5 audit §3.6 unreachable!() arm bodies)
- ✓ Smoke matrix 3/4 VM == JIT at canonical fixture (β2 disposition; Smoke 2 JIT cluster-2 fold)

**Tag authorization request:** pending supervisor cluster-0+1-close-criterion ratification + user tag authorization. Tags: `phase-3-cluster-0-close` + `phase-3-cluster-1-close` on canonical at `50e5c024`.

**V3-S6 chain meta-pattern: architectural-prediction subclass — 5 instances (37/38/39/40/41) all caught pre-merge:**

- 37th (V3-S6 dispatch): "same gap at two layers" → wrong (two distinct gaps)
- 38th (V3-S6b PATH α): "single intended consumer" → wrong (implicit-consumer-activation; v2-fast-path dual-consumer SIGSEGV)
- 39th (V3-S6c Step 2): "alternatives" → wrong (complementary-not-alternatives; V3-S6c + V3-S6d both needed)
- 40th (V3-S6d): "callee FuncRef exists" → wrong (callee FuncRef points to -1 stub because callee body fails JIT-compile)
- 41st (V3-S6e): "compile-time fix sufficient" → wrong (runtime execution layer revealed V3-S6f inner gap; Reading 2 empirically confirmed)

**Reading 2 empirically confirmed:** chain pattern is incomplete-conduit-completion-at-multiple-layers; cluster-2 absorbs broader §2.7.5 JIT-side conduit completion for cleaner architectural conversation. The V3-S6 chain produced real architectural work (V3-S5 conduit completion at compile-time layer); V3-S6f + broader work is cluster-2 territory.

**Cumulative discipline-pattern instances through cluster-0+1 close:**

- 41 imprecision-pattern instances (10 supervisor / 14 audit / 10 team-lead-prompt / 5 agent-execution-report / 2 candidate); all caught pre-merge
- 9 self-detected defection-attractor refusals (unchanged from V3-S5; V3-S6 chain preserved discipline at every checkpoint — PATH γ refused; PATH β fallback-only; no bridge/probe/helper/hop framings)
- 5 S1-R18 DURABLE PATTERN instances (unchanged; pattern operational)
- 4 successful multi-session chains (D4 6-sub-agent + Round 3b 4-sub-agent + V3-S5 10-checkpoint + V3-S6 5-checkpoint)
- 8 parallel-implementation defection-attractor instances all structurally resolved
- 0 bad-code merges into canonical across entire cluster-0+1 trajectory

**Next workstream: cluster-2** (§2.7.5 JIT-side conduit completion):

- **V3-S6f-jit-specialized-vec-map-runtime-execution** — Smoke 2 JIT cluster-2 canonical fixture; hypothesis space: (a) for-loop iterator state-machine for v2 typed-array source; (b) closure-call indirect-call shape inside inlined specialization; (c) receiver-self slot kind threading from V3-S6c routing
- **Broader W11-jit-new-array general fix** — extend function_local_concrete_types population to ALL user functions (not just monomorphized specializations); address runtime-execution-path classification across all parametric body slots
- **hashmap-value-v-arm follow-up** (per Wave 3 R1 close subsection)
- **shape-test-residuals-audit** (10 failure classes per Wave 3 R1 baseline-classification)
- **per-HeapKind kinded jit_print** + **compile-time-boxed string-constant leak** + **W12-collection-constructor-mir-lowering**

Cluster-2 dispatch shape: audit-day first (single sub-agent comprehensive inventory) → closure waves after (parallel sub-agents per Wave 1 partition recommendation). Trajectory remains 3-5 sessions to v1 (cluster-2 cleanup 1-2 + Phase 4 trait Add/AddAssign 1-2 + v1 close attempt 0.5).

**Immediate next actions (in order):**

1. **Read 9 mandatory docs** + post 1-line confirmation (per §First action).
2. **Receive supervisor cluster-0+1-close-criterion ratification relay from user** + apply `phase-3-cluster-0-close` + `phase-3-cluster-1-close` tags on canonical at `50e5c024` once user authorizes.
3. **After tag authorization lands:** dispatch cluster-2 audit-day per supervisor's next-phase disposition. Territory: V3-S6f hypothesis space + broader W11-jit-new-array general fix + shape-test-residuals-audit + hashmap-value-v-arm + per-HeapKind kinded jit_print + compile-time-boxed string-constant leak + W12-collection-constructor-mir-lowering. Single sub-agent at ceiling-c expected for audit-only deliverable.
4. **After cluster-2 audit-day closes + ratifies:** dispatch parallel cluster-2 sub-cluster waves per audit recommendation.
5. **After cluster-2 closes:** Phase 4 (trait Add/AddAssign for user types).
6. **Then v1 close attempt.**

**Pending items archived at handover:**

- ✓ Wave 1 audit-day + Wave 2 Round 1/2/3a'/3a + Round 3b + Wave 3 R1 V3-A2-followup + Wave 3 R2 V3-S5 + Wave 3 R3 V3-S6 chain all merged
- ✓ ADR-006 amendments landed (Q25.A SUPERSEDED + Q25.B SUPERSEDED + Path B §2.3 + §Q25.C.5 from V3-S5 + Round 3b + D4)
- ✓ Status doc cluster-0+1 close subsection landed (this session)
- ✓ S1-R18 DURABLE PATTERN ratified by user (4-criterion test; 5 instances)
- ✓ Surface A user disposition (c) split — Q25.C TraitObject rebuild = cluster-1.5 follow-up
- ✓ CLAUDE.md compacted (44.9k → 35.6k chars; 2026-05-14)
- ✓ All forbidden-pattern + refuse-on-sight discipline preserved (10 items + #10 anti-deferral + #11 Ptr-newtype-shim refusal)

**Pending items beyond cluster-0+1 (cluster-2 + Phase 4):**

- V3-S6f-jit-specialized-vec-map-runtime-execution (Smoke 2 JIT canonical fixture)
- Broader W11-jit-new-array general fix (§2.7.5 JIT-side conduit completion across all user functions)
- Q25.C TraitObject rebuild (`dyn T = box(X{})` per kickoff prompt prose; cluster-1.5)
- Cluster-2 audit subjects: per-HeapKind kinded jit_print + compile-time-boxed string-constant leak + W12-collection-constructor-mir-lowering + shape-test-residuals-audit (10 failure classes)
- Phase 4: trait Add/AddAssign for user types
- v1 close attempt

## In-flight state (archived) — V3-S5-complete + V3-S6 surface-and-stop + user (B') seam (2026-05-15)

**Archived pre-V3-S6 in-flight state preserved below for historical reference.**

**Pre-cluster-0+1-close-seam HEAD:** `bulldozer-strictly-typed @ 9523d57a` — V3-S5 10-ckpt multi-session chain COMPLETE + MERGED + VERIFIED. TypedArrayData enum + TypedBuffer<T> + AlignedTypedBuffer wholesale deletion done. 1223 LIVE refs → 0 LIVE refs (Framing (ii) live-grep zero across all 3 patterns). ADR-006 §2.7.24 Q25.A SUPERSEDED fully fulfilled.

**Wave 2 + Wave 3 cumulative through V3-S5:**

- Wave 1 audit-day (`bulldozer-wave-1-inventory.md`): A-R sections; cluster-0+1 deletion targets mapped.
- Wave 2 Round 1 (6 agents A1/B/C/D1/F/G): ~6.8k LoC. RATIFIED + merged.
- Wave 2 Round 2 (4 agents A2/D2/E/C2a): E substantive + 3 surface-and-stops. RATIFIED + merged.
- Wave 2 Round 3a' (8 agents handler-family α-η + gate-flip): ~951 LoC migration foundation. RATIFIED + merged.
- Wave 2 Round 3a (D4 6-sub-agent multi-session chain): PATH B atomic Ptr-newtype canonical landed; Audit §4.3 O-3.a/O-3a RESOLVED. RATIFIED + merged.
- Wave 2 Round 3b (C2-joint 4-sub-agent multi-session chain): HashMapData<V> per-V monomorphization; Q25.B SUPERSEDED. RATIFIED + merged.
- Wave 3 R1 (V3-A2-followup-producer-cascade + V3-baseline-classification): Array<string>/<decimal> literal LoadConst-to-NewStringV2/DecimalV2 + baseline classification (10 failure classes cluster-2 territory). RATIFIED + merged.
- Wave 3 R2 V3-S5 (10-ckpt multi-session chain ckpt-1 through ckpt-6-prime): TypedArrayData wholesale architectural sunset + JIT FFI String/Decimal build. RATIFIED + merged at 9523d57a.

**Smoke matrix at HEAD `9523d57a`:**

| Smoke | VM | JIT | Status |
|---|---|---|---|
| 1 (scalar loop) | ✅ 4950 | ✅ 4950 | ✓ |
| 2 (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | ❌ SURFACE | ❌ SURFACE | V3-S5 regression; V3-S6a-resolver-extension dispatch authorized (user choice (B') 2026-05-15) |
| 3 (canonical fixture `let t = X{}`) | ✅ x | ✅ x | ✓ |
| 4 (Set + .add + .size) | ✅ 2 | ✅ 2 | ✓ |

**3 of 4 VM == JIT at canonical fixture.** Smoke 2 regressed from VM=30 → SURFACE during V3-S5 ckpt-5 consumer-cascade tier 3 (locus: Vec.map<U> body `let mut result = []` empty literal where closure-return-typed generic U doesn't bind through resolver chain). V3-S6a-resolver-extension is the cluster-0+1-close-gating fix.

**V3-S6 SURFACE-AND-STOP (2026-05-15, clean working tree, no commits):**

Initial V3-S6 dispatch was based on supervisor 36th-imprecision (my Part 1 framing claimed "[1,2,3,4,5] literal emission falls through to legacy op_new_array path"). Sub-agent verified via existing passing tests (test_int_array_emits_typed_opcodes + test_int_array_executes_correctly at typed_emission.rs:468/543) that `[1,2,3,4,5]` ALREADY emits NewTypedArrayI64. Producer-side for Array<int> literals is sound. The regression locus is in `crates/shape-runtime/stdlib-src/core/vec.shape:51-57` Vec.map<U> body's empty `[]` literal — closure-return-typed generic U doesn't bind through `resolve_call_site_type_args_with_closures` resolver chain.

**User choice (B') RATIFIED 2026-05-15:** dispatch V3-S6a-resolver-extension to extend `resolve_call_site_type_args_with_closures` at `crates/shape-runtime/src/type_system/inference/type_resolution.rs:760-850+` to bind generics from closure return types when the type-only resolver leaves them unbound. ~100-200 LoC; single sub-agent at ceiling-c expected. Fix unblocks both Smoke 2 VM AND JIT simultaneously (sub-agent finding: same gap surfaces at both layers).

**Cumulative discipline-pattern instances through V3-S5 + V3-S6 surface:**

- 36 imprecision instances cumulative (11 supervisor-layer / 14 audit-layer / 5 team-lead-prompt-layer / 5 agent-execution-report-layer; +1 36th from V3-S6 framing); all caught pre-merge
- 9 self-detected defection-attractor refusals (unchanged from V3-S5; clean execution)
- 5 S1-R18 DURABLE PATTERN instances (unchanged; pattern operational)
- 8 parallel-implementation defection-attractor instances all structurally resolved

**Cluster-0+1-close-criterion gates at HEAD 9523d57a:**

Resolved (RATIFIED by supervisor):
- ✓ Q25.A SUPERSEDED (R20 + V3-S5) + Q25.B SUPERSEDED (Round 3b)
- ✓ Path B canonical Ptr-newtype pattern (D4)
- ✓ Audit §4.3 O-3.a + O-3a (D4)
- ✓ Producer-side foundation (Wave 3 R1)
- ✓ hashmap-value-v-arm → cluster-2 fold
- ✓ V3-S5 wholesale enum + buffer deletion (live-grep 0/0/0)
- ✓ JIT FFI String/Decimal build (Group X, ckpt-6-prime)
- ✓ All gate checks (cargo check / just check-clean / verify-merge.sh 12/12 / check-no-dynamic / 4-table lockstep 0/4 LIVE)
- ✓ Framing (ii) live-grep zero across all 3 patterns

Pending V3-S6a:
- ⏳ Smoke matrix 4/4 VM == JIT at canonical fixture producing expected values (Smoke 2 30/30; restored by V3-S6a-resolver-extension)

**Wave 2 closed cumulative (HEAD 8a87ddd7):**

- **Round 1** (6 agents): A1 + B + C + D1 + F + G. ~6.8k LoC. Round 1 surfaced 7 imprecision instances + 8 defection-attractor instances logged. RATIFIED + merged.
- **Round 2** (4 agents): A2 + D2 + E + C2a. 1 substantive (E) + 3 surface-and-stops (A2/D2 spilled to Round 3a; C2a structurally refused runtime/FFI split → C2-joint Round 3b). 4 imprecision instances. RATIFIED + merged.
- **Round 3a'** (8 agents: 7 per-handler-family α-η + 1 sequential gate-flip): ~951 LoC migration foundation. S1-R18 DURABLE PATTERN ratified by user (4-criterion test). 4 imprecision instances. RATIFIED + merged.
- **Round 3a (D4)** (6 sub-agents multi-session chain per bulldozer-multi-session-chain-pattern.md): ckpt-1 + ckpt-2 + ckpt-3 + ckpt-final + ckpt-final-prime + ckpt-final-prime² with PATH B atomic single-commit. ~617 LoC across ~30 source files. ADR-006 §2.3 amendment landed (Path B TypedObjectPtr/TraitObjectPtr canonical pattern + bounded forbidden framings). 5-arm receiver-recovery violation at `executor/builtins/object_ops.rs:59` FIXED inline. 4 imprecision instances (19-22 cumulative through D4). 0 S1-R18 instances during D4 chain. RATIFIED + merged at `8a87ddd7`.

**D4 PATH B canonical pattern (ADR-006 §2.3 amendment landed in `47b55a63` atomic commit):**

- HeapValue::TypedObject(TypedObjectPtr) — `#[repr(transparent)]` newtype around `*const TypedObjectStorage`; manual Drop calling `release_elem → v2_release + Self::_drop`; manual Clone calling `v2_retain`; manual `unsafe impl Send + Sync` (orphan-rule workaround); HeapValue auto-derives Drop/Clone/Send/Sync chain through newtype discipline.
- HeapValue::TraitObject(TraitObjectPtr) — mirror newtype at v2/trait_object_ptr.rs.
- TypedArrayData::TypedObject + HashMapValueBuf::TypedObject use the same Ptr-newtype payload.
- Pattern is CANONICAL for v2-raw HeapHeader-equipped storage types only. Arc<String> (ADR-005 §2 exception) remains canonical for String payload; no "StringPtr" sibling.

**Forbidden (extending CLAUDE.md §Renames to refuse on sight, per supervisor 2026-05-14 ratification):**

- "TypedObjectPtr shim" / "TraitObjectPtr bridge" / "Ptr-newtype helper" / any bridge/probe/helper/hop framing for these newtypes
- Parallel `Arc<TypedObjectStorage>` / `Arc<TraitObjectStorage>` payloads alongside Ptr-newtype shapes (single canonical payload; refuse parallel-implementation framings)
- Ptr-newtype siblings for non-HeapHeader-equipped storage types (no "StringPtr"; no "BigIntPtr" until BigInt itself becomes HeapHeader-equipped)

**Cumulative discipline-pattern instances (22 imprecision; 5 S1-R18 durable; 8 parallel-implementation defection):**

- 22 imprecision-pattern instances cumulative (8 supervisor-layer; 14 audit-layer; all caught at agent layer pre-source-change)
- 5 S1-R18 DURABLE PATTERN instances (Wave 1 audit + D1 drive-by + Round 3a' δ/ε/ζ)
- 8 parallel-implementation defection-attractor instances (all surfaced + caught + structurally resolved)

**Immediate next actions (in order):**

1. **Read 9 mandatory docs** + post 1-line confirmation (per §First action).
2. **Dispatch Round 3b C2-joint** — single atomic ~5k LoC / 40 files commit per C2a structural finding (HashMapData<V> per-V monomorphization runtime + JIT FFI cannot split per type-confusion-window invariant). Likely needs bulldozer-multi-session-chain-pattern.md again per ceiling-c. Territory below.
3. **After Round 3b closes + merges:** dispatch Wave 3 stabilize. Scope: S5 wholesale TypedArrayData enum deletion + A2-followup-producer-cascade (Array<string> literal upgrade post-gate-flip) + shape-test classification per baseline check (verify pre-Round-3a' baseline async failures pre-existing vs new-regression).
4. **After Wave 3 stabilize closes:** cluster-0+1 close attempt. Smoke matrix VM == JIT across all 4 canonical fixtures. Surface to supervisor → user authorizes `phase-3-cluster-0-close` + `phase-3-cluster-1-close` tags.
5. **Then cluster-2 + Phase 4** to v1.

**Round 3b C2-joint dispatch shape (supervisor pre-ratification; team-lead refines for multi-session chain):**

Territory (single atomic ~5k LoC / 40 files):
- `crates/shape-value/src/heap_value.rs` — HashMapData<V> generic + impl + Drop; HeapValue::HashMap arm signature change Arc<HashMapData> → HashMapKindedRef
- `crates/shape-value/src/kinded_slot.rs` — HashMapKindedRef carrier construction + Drop + Clone
- `crates/shape-vm/src/executor/objects/hashmap_methods.rs` — per-V dispatch
- `crates/shape-vm/src/executor/objects/array_*.rs` — HashMap-aware paths
- `crates/shape-vm/src/compiler/type_tracking.rs` — HashMap kind tracking
- `crates/shape-jit/src/ffi/v2/typed_map.rs` — per-V dispatch (maintains raw-bits ABI per §2.7.5)
- `crates/shape-jit/src/ffi/v2/collection_arc.rs:74-263` — retain/release flip
- `crates/shape-jit/src/mir_compiler/v2_typed_map.rs` — per-V codegen
- `crates/shape-jit/src/ffi_refs.rs` — per-V FFI refs
- All 33 HeapValue::HashMap match sites adapt to per-V monomorphization

Atomic-lockstep invariant: HeapValue::HashMap signature change cascades to JIT FFI's `Arc::decrement_strong_count` at `collection_arc.rs:74-263`; runtime-tier-only flip would leave JIT FFI operating on bits whose layout changed = heap corruption window. Single commit lands ALL of: runtime + JIT FFI + producer sites.

HashMapKindedRef stays within shape-value / shape-runtime / shape-vm internal Rust boundaries per §2.7.5; does NOT leak into extension contract raw-bits ABI.

ADR-006 §Q25.B SUPERSEDED amendment text already landed at C1 close (Round 1 merge). C2-joint commit reaffirms + may refine if implementation detail shifted from C1 audit prediction.

Multi-session chain shape (per bulldozer-multi-session-chain-pattern.md):
- Feature branch: `bulldozer-strictly-typed-round-3b-c2-joint-checkpoint-N`
- ckpt-1 sub-agent: HashMapData<V> struct + impl + HashMapKindedRef carrier (foundation; ~50-100 edits)
- ckpt-2 sub-agent: HeapValue::HashMap arm signature + 33 match sites adaptation (~50-100 edits)
- ckpt-3 sub-agent: hashmap_methods.rs per-V dispatch + array_*.rs HashMap-aware paths (~50-100 edits)
- ckpt-4 sub-agent (or ckpt-final): JIT FFI flip (typed_map.rs + collection_arc.rs + mir_compiler + ffi_refs) + STRICT close gate (cargo check PASSES + verify-merge.sh 12/12 + check-no-dynamic + smoke matrix preserved)
- Team-lead merges feature branch into canonical post-final-sub-agent

Pre-flight territory-enumeration check binding (extended per Round 3a' close): grep cascade at feature branch HEAD; surface site-count ratio in close report; surface-and-stop pre-edit if cascade exceeds enumeration by >2× (21st imprecision precedent).

Dynamic chain authorization per D-α discipline: if any sub-agent surfaces ceiling-c capacity exceeded, dispatch sub-agent-prime continuing from state pointer.

**Wave 3 stabilize scope (after Round 3b closes):**

- **S5 wholesale TypedArrayData enum deletion** — single agent; ~400-site cleanup; close-gate is `grep -rn 'TypedArrayData::' crates/` returning zero hits (TypedObject + TraitObject arms migrated to Ptr-newtype payloads in D4; scalar arms deleted; dead Temporal/Instant arms deleted; U64 relabel-step per Shape D folds in)
- **A2-followup-producer-cascade** — Array<string> literal upgrade post-gate-flip; LoadConst-to-NewStringV2 transition + bytecode-emission layer adjustments at string-literal handling sites. Unblocks Smoke 2 JIT alongside S5 dual-carrier elimination.
- **Shape-test baseline classification** — verify pre-Round-3a' baseline async failures (9 in async_concurrency suite + 1 book_doctests) pre-existing vs new-regression via worktree at HEAD `4c7b1d9d`. Pre-existing → fold into cluster-2 `shape-test-residuals-audit`. New-regression → fix in Wave 3 stabilize.

**Honest velocity: 4-7 sessions remaining to v1.**

| Stage | Sessions |
|---|---|
| Round 3b C2-joint (multi-session chain; ~5k LoC atomic) | 1-2 |
| Wave 3 stabilize (S5 + A2-followup-producer-cascade + shape-test classification) | 1-2 |
| Cluster-0+1 close attempt | 0.5 |
| Cluster-2 cleanup (per-HeapKind kinded jit_print + compile-time-boxed string-constant leak + W12-collection-constructor-mir-lowering + shape-test-residuals-audit) | 1-2 |
| Phase 4 (trait Add/AddAssign for user types) | 1-2 |
| **Total remaining** | **4-7** |

Bulldozer cadence delivering. D4 chain validated multi-session pattern operationally; same shape applies to Round 3b. 22 imprecision instances cumulative; all caught at agent layer pre-source-change. Trust the cadence.

**Pending items already at handover:**
- ✓ Wave 1 audit-day landed (`bulldozer-wave-1-inventory.md`)
- ✓ Wave 2 Round 1 + Round 2 + Round 3a' (gate-flip) + Round 3a (D4) all merged
- ✓ ADR-006 §2.3 amendment landed (D4 Path B Ptr-newtype canonical pattern at lines 301-388)
- ✓ ADR-006 §Q25.A SUPERSEDED landed at R20 S2-prime (c); §Q25.B SUPERSEDED landed at C1 close; §Q25.C.5 consistency note at D4 close
- ✓ `bulldozer-multi-session-chain-pattern.md` operational
- ✓ S1-R18 DURABLE PATTERN ratified by user (4-criterion test; 5 cumulative instances)
- ✓ Surface A user disposition (c) split — Q25.C TraitObject rebuild = cluster-1.5 follow-up; not blocking cluster-0+1 close
- ✓ CLAUDE.md compacted (44.9k → 35.6k chars) per user authorization 2026-05-14
- ✓ All forbidden-pattern + refuse-on-sight discipline preserved (10 items + #10 anti-deferral + #11 Ptr-newtype-shim refusal extended per D4)

**Pending items beyond Wave 3 (cluster-2 + Phase 4):**
- Q25.C TraitObject rebuild (`dyn T = box(X{})` per kickoff prompt prose; cluster-1.5 per Surface A (c))
- Cluster-2: per-HeapKind kinded jit_print + compile-time-boxed string-constant leak + W12-collection-constructor-mir-lowering + shape-test-residuals-audit (48 failures triage)
- Phase 4: trait Add/AddAssign for user types
- v1 close attempt

## Decision authority pattern

You ARE authorized to:
- Run inline cite-audit before dispatch (per R19 S1.5 + R20 precedent).
- Verify wave-agent close reports against the dispatch contract.
- Refuse a wave-agent's close report at merge time if it harbors a forbidden pattern (then surface to supervisor for reopen vs re-dispatch decision).
- Coordinate AGENTS.md row updates + merge order + take-both resolution across parallel wave-2 agents.
- Run reopen via SendMessage on a closed-but-not-merged wave-agent when an audit gap is small + recoverable (W11-round-1 precedent).
- Complete ceremony for agent-API-error WIP that's verifiably correct (S1 reopen R18 precedent). Each instance requires supervisor authorization until durable pattern established.
- Update `phase-3-cluster-0-status.md` + `AGENTS.md`.

You are NOT authorized without explicit supervisor approval:
- Dispatch new waves (supervisor ratifies wave scope + agent partition + cadence per cluster).
- Refuse defection-pattern framings on the agent's behalf at the meta-architectural level — refusing is the supervisor's call; you flag + surface.
- Authorize ADR amendments.
- Re-scope cluster boundaries (cluster-0 → cluster-1 reclassification, kickoff matrix changes, close criterion modifications).
- Tag `phase-3-cluster-0-close` / `phase-3-cluster-1-close` (user authorizes after supervisor ratifies).
- **Land `CLAUDE.md` modifications in any commit** without explicit user ratification of the landing (R17 precedent + 2026-05-14 compaction precedent — both required explicit user authorization).
- Revert the bulldozer cadence to audit-first without explicit user authorization (cadence shift was strategic-owner 2026-05-14; refuse #10 applies at the meta layer).

## Discipline-pattern observations (carry forward)

**Supervisor-/audit-layer imprecision pattern** (5 instances cluster-0, possibly continuing under bulldozer cadence):
1. R16 §2.7.14-A draft — supervisor's "unwrap-and-flatten" framing. Caught by W12-Option-B-reframed agent.
2. R18 S1 reopen SendMessage — "Array<u64> fails at compile-time" imprecise (legacy fallback ≠ compile-time rejection). Caught by S1 reopen agent.
3. R19 S1.5 audit Shape B framing — "runtime element-kind from HeapHeader byte" §2.7.5 risk. Caught by supervisor pre-dispatch.
4. R19 S2 dispatch double-bind (refuse-on-sight vs ADR-conflict). Caught + corrected via R19 partial disposition + ratified at R20 S2-prime (c).
5. R20 γ kickoff-prompt-vs-fixture mismatch (canonical artifact ≠ canonical prose; 9 rounds against fixture). Caught at γ agent-execution layer via SHAPE_JIT_DEBUG trace ground-truthing.

**Pattern shape:** audits + supervisor relays harbor latent imprecisions; the discipline check is multi-layer (agent + team-lead + supervisor). Each instance is caught BEFORE bad code merges; the trend is decreasing severity. The bulldozer cadence adds **pre-flight ground-truth verification** at the wave-agent dispatch level (every audit claim grep-verified against source at HEAD before agent commits) — see Wave 1 dispatch prompt audit-first deliverables.

**Stash-then-rebuild + structured-surfacing pattern** (W17-narrow R15 + R18 S1 reopen + R19 C precedent): when a wave-agent's own contract is verifiably met but the smoke target fails due to a surfaced upstream gap, (a) verify own contract clean, (b) verify upstream gap is pre-existing (stash-then-rebuild or detached-HEAD check), (c) structured-surface upstream gap as new sub-target (not "follow-up to ignore"). Under bulldozer cadence: the new sub-target either folds into the same wave OR gets explicit supervisor disposition (NOT cluster-N+1 defer).

**Agent API-error recovery pattern** (S1 reopen R18 precedent): three recovery options when sub-agent API-errors with WIP uncommitted. (1) SendMessage retry first. (2) Team-lead completes ceremony for verified-correct WIP (requires supervisor authorization per instance). (3) Re-dispatch fresh agent (conservative-wasteful).

**Cross-tier compat pattern during NativeKind variant additions** (S1.5 R19 precedent): dual-label match at consumer layer during migration window; collapses to single-label after consumer-site migration completes (cluster-1 hardening territory pre-bulldozer; folded into Wave 2 cascade under bulldozer cadence).

**Bulldozer cadence pattern observation (new, 2026-05-14):** the audit-first cadence preserved attractors in source (W17-typed-carrier-bundle-A dead arms ~2 months; TypedArrayData enum 2-arm-shadow under O-3.c deferral; HashMapValueBuf parallel deletion target cluster-1+ deferred). The cadence shift refuses the deferral framing within wave scope. Surface-and-stop is still allowed for GENUINELY intractable in-wave gaps — supervisor disposes whether to extend the wave or genuinely defer.

## User preferences + working style

- **No `Co-Authored-By: Claude` trailer in commits.** MEMORY.md rule.
- **Own all code quality.** Never frame as "pre-existing" — all code is the agent's responsibility once touched.
- **Plain code fences for relay text**, not blockquotes. The user copies relay blocks verbatim; blockquote `>` prefixes break paste.
- **Direct, concise communication.** Tight responses; substantive when needed; no padding.
- **Strategic owner / language designer.** Delegates architectural calls to the supervisor. Will surface explicitly on language-design / project-scope / cadence questions.
- **Working in agent velocity.** Multi-wave-agent-per-session cadence is expected under bulldozer cadence.

## Operational continuity

1. **First action** after reading the 9 docs: post the 1-line confirmation + verify predecessor ceremony complete + dispatch Wave 1 audit-day per supervisor's authorization (once R20 status-doc close commit lands).
2. **Standard interaction pattern**: wave-agent closes → you verify gate + read close report → you draft consolidated status → user relays to supervisor → supervisor responds → user pastes back → you execute.
3. **Don't re-derive context** that's already in `phase-3-cluster-0-status.md`, `CLAUDE.md` (compacted), ADR-006, the audit docs, or this handover.
4. **The supervisor session is current** at the R20 close + cadence-shift authorization seam; supervisor handover at `docs/cluster-audits/phase-3-supervisor-handover.md` may need an update reflecting the cadence shift if supervisor rotates during Wave 1 (predecessor supervisor's continuity into Wave 2 dispatch is expected).

**Most-likely-next-action:** dispatch Wave 1 single-audit-day per the §Wave 1 dispatch shape above. After Wave 1 closes + merges + status-doc update, surface to supervisor for Wave 2 partition ratification + ADR amendment text shapes. Then Wave 2 + Wave 3 + cluster-0+1 close.

---

*End of handover. Read §First action before any dispatch.*
