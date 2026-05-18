# Phase 3 team-lead handover — v0.3 Phase 4 (test hardening + book truth)

**Generated:** 2026-05-18 (Phase 4 reframe per user 2026-05-18 acceptance criterion revision).
**Authority for Phase 4 reframe:** user dispatch 2026-05-18 expanded v0.3 acceptance criteria to require test coverage ≥99% per-feature (W14) + book 100% correct vs HEAD (W15) + W13 differential fuzz, with W11-fup-C jit_print_typed_array as the closing Phase 3d sub-cluster (merged at `672bda24`). v0.3 tag NOT yet annotated; close-summary §6 paste-ready text was STRIPPED at the same atomic commit to prevent premature tag landing.

## Current state at Phase 4 entry

| | |
|---|---|
| Main HEAD at handover bundle | (set by atomic bundled commit; see git log first-parent) |
| Smoke matrix s1–s5 | 5/5 VM == JIT preserved per release-binary corrected harness |
| v0.3 trajectory phase | Phase 4 (test hardening + W13 differential fuzz + W14 test coverage + W15 book truth) |
| v0.3 tag | NOT yet annotated; gated on Phase 4 close |
| Cumulative bad-code merges into main | 0 per release-binary corrected harness |

### Phase 4 acceptance criteria (binding; supersedes pre-2026-05-18 v0.3 acceptance)

- Operator coverage full set (substantially MET at HEAD per W1.1–W1.11 sub-clusters)
- LSP en-par with rust-analyzer (substantially MET at HEAD per W2.1 audit + W2.2/W2.3/W2.4/W2.5/W2.6/W2.7/W2.8/W2.9 closure-waves)
- Book 100% correct vs HEAD (W15 workstream; zero made-up content; every labeled-runnable snippet runs cleanly VM == JIT)
- Test coverage ≥99% per-feature with DOCUMENTED EXCEPTIONS ONLY WHEN VERY HARD TO TEST (W14 workstream; cargo-tarpaulin measurement)
- Smoke 5/5 VM == JIT preserved (corrected release-binary harness)
- W12 fall-through + diagnostic operational
- v2-raw residuals RESOLVED (cluster-1.5)
- 0 bad-code merges into main preserved per corrected harness

### Phase 4 sequencing

- **Phase 4a** — parallel sub-agent audit-day round (4 agents in parallel):
  - W11-fup-C status check (residual surfaces post-merge audit)
  - W13 differential fuzz audit-day (scope + cargo-fuzz vs custom; per-domain inventory)
  - W14.1 test coverage audit-day (cargo-tarpaulin install + invocation pattern + per-crate baseline measurement + per-feature coverage classification)
  - W15.1 book re-audit-day (every labeled-runnable snippet at HEAD; W3.2-A/B/C/D/E divergence residuals)
- **Phase 4b** — parallel sub-agent fix waves (batched 5-10 per ceremony round; strict folder/file structure rules per audit):
  - W14.2 parallel test-coverage fix waves (per W14.1 audit per-crate gaps)
  - W15.2 parallel book fix waves (per W15.1 residuals; smaller batches)
- **Phase 4c** — after batch merges:
  - W14.3 coverage gate (cargo-tarpaulin Phase-4b-batch-merge gate + nightly post-v0.3)
  - W15.3 book-truth-gate (CI executes every labeled-runnable snippet under VM AND JIT; fails on output divergence OR runtime error)
- **v0.3 close attempt + tag** (post Phase 4c)

### Test execution policy (binding)

- Per-commit gate = FOCUSED tests relevant to the specific change (sub-agent determines scope per change; team-lead enforces at close-gate review)
- Full suite + fuzz = NIGHTLY ONLY (not per-commit; not per-merge-ceremony unless audit-day batch merge)
- Coverage tool runs at Phase-4b-batch-merge + nightly only
- Coverage tooling: cargo-tarpaulin (line + branch + dead-code via `--ignore-tests` + `--skip-clean` variants); install + invocation pattern in W14.1 audit deliverable

### Trajectory estimate

8–10 sessions to v0.3 close (Phase 4a 1 + Phase 4b 3–4 + Phase 4c 1 + close 0.5 + buffer).

## First action — fresh team-lead

1. Read this doc + `docs/v0.3-close-summary.md` (current state at Phase 4 entry) + `docs/v0.3-roadmap.md` (§Phase-4 expansion) + `CLAUDE.md` (Forbidden Patterns + Renames to refuse on sight + ADR-005 / ADR-006 key rules).
2. Post the 1-line confirmation: *"Read 4 mandatory docs; team-lead role ready under Phase 4 (test hardening + book truth). Current state: <one sentence>."*
3. Verify main HEAD + smoke matrix 5/5 VM == JIT via release-binary corrected harness before any sub-agent dispatch.
4. Surface Phase 4a parallel partition shape to supervisor (4 agents: W11-fup-C status check + W13 audit-day + W14.1 audit-day + W15.1 audit-day). Wait for ratification.
5. Dispatch sub-agents in a single message with parallel Agent calls (no sequencing — territory non-overlap per audit-only deliverables).

## Your role this session

Team lead for Phase 4. Job:

1. Dispatch sub-agents via `Agent` tool per supervisor relays.
2. Verify close gates: `just check-clean` exit 0 + `bash scripts/verify-merge.sh` 12/12 exit 0 + `bash scripts/check-no-dynamic.sh` exit 0 + AGENTS.md row appended + smoke matrix 5/5 VM == JIT preserved via release-binary corrected harness.
3. Merge sub-agent branches into main (take-both for AGENTS.md row + ADR-006 amendment text + dispatch-table arm collisions).
4. Update `docs/v0.3-close-summary.md` + `docs/v0.3-roadmap.md` + this handover doc as Phase 4 progresses.
5. Surface architectural questions to the supervisor via the user (strategic-owner relays).

Architectural calls (ADR amendments, scope changes, defection-pattern refusals at the meta layer, tag authorization) are NOT in your lane — surface them.

## Discipline bindings (load-bearing; refuse on sight)

All standard Phase 3 / cluster-1.5 / v0.2.0 / v0.3 disciplines apply unchanged:

- **NO ARCHAEOLOGY in living docs** (strip wholesale; git holds history)
- **Smoke-gate harness MUST use release binary + verified exit code**
- **All CLAUDE.md Forbidden Patterns + Renames to refuse on sight** (broader-family regex; bridge/probe/helper/hop/translator/adapter/shim; W-series ValueWord renames; "compatibility layer" rationalization)
- **All ADR-006 §2.7.x rulings** (4-table HeapKind lockstep, §2.7.5 stamp-at-compile-time, §2.7.6/Q8 carrier-API-bound, §2.7.7/Q9 stack parallel-kind, §2.7.8/Q10 cell-storage parallel-kind, §2.7.10/Q11 method-dispatch ABI, §2.7.11/Q12 value-call ABI)
- **ADR-005 §1 single-discriminator + §2 String exception**
- **5-arm receiver-recovery soundness rule** (W13→W16 lesson)
- **Refuse #10 anti-deferral within wave scope** ("preserve for v0.4", "needs its own audit sub-cluster", "multi-week scope" — all refused)
- **Refuse #11 Ptr-newtype-shim defection** (TypedObjectPtr / TraitObjectPtr canonical per D4; not transitional shims)
- **"Pre-existing" framing FORBIDDEN if sub-agent introduces a regression** (per CLAUDE.md "Own all code quality"; instance 82 CRITICAL recovery precedent)
- **CLAUDE.md modifications require explicit user authorization**
- **Tag landings require explicit user authorization**
- **No Co-Authored-By: Claude trailer; own all code quality**

### Cadence binding (Reading 3 carry-forward, 2026-05-16 + 2026-05-17 + 2026-05-18)

- Max ~100 lines per supervisor relay
- Surfacings = NEW facts + ONE specific ask (NO re-citation of cumulative state, NO multi-paragraph rationale, NO pathway taxonomies when direct recommendation suffices)
- Pre-flight ground-truth binding extends to dispatch prompts you draft: every file:line / commit hash / symbol presence MUST grep-verify at HEAD before sub-agent commits time on broken-path enumeration
- One refinement pass per dispatch prompt
- 30–50 parallel sub-agent dispatch precedent established 2026-05-18

## Decision authority pattern

- **Supervisor authorizes:** cluster/wave scope, ADR amendments, sub-cluster dispatch shape, defection-pattern refusals at meta-architectural layer
- **User authorizes:** CLAUDE.md modifications, cluster-close + release tags (incl. `v0.3.0` tag), language-design semantics, project-scope decisions, Phase 4 acceptance criterion revisions
- **Team-lead authorizes:** wave-agent dispatch within supervisor-ratified scope, close-gate verification, merge ceremony, take-both resolution, status / handover / roadmap doc updates, AGENTS.md updates, reopen via SendMessage for small recoverable fixes (per S1-R18 precedent)

## Smoke matrix shape (canonical fixtures)

```
s1 scalar-loop          let mut sum = 0; for i in 0..100 { sum += i }; print(sum) → 4950
s2 typed-array map+sum  [1,2,3,4,5].map(|x|x*2).sum() → 30
s3 UFCS dispatch        canonical fixture → x
s4 Set basics           Set + .add + .size → 2
s5 dyn T trait-object   canonical fixture → x
```

Corrected release-binary harness shape:

```bash
out=$(timeout 30 ./target/release/shape run --mode $mode $file 2>/dev/null | tail -1)
ec=$?
```

`ec` must be 0 + `out` must match expected value for VM == JIT to hold.

## User preferences + working style

- **No `Co-Authored-By: Claude` trailer in commits.** MEMORY.md rule.
- **Own all code quality.** Never frame as "pre-existing" — all code is the agent's responsibility once touched.
- **Plain code fences for relay text**, not blockquotes. The user copies relay blocks verbatim; blockquote `>` prefixes break paste.
- **Direct, concise communication.** Tight responses; substantive when needed; no padding.
- **Strategic owner / language designer.** Delegates architectural calls to the supervisor. Surfaces explicitly on language-design / project-scope / cadence questions.
- **Working in agent velocity.** 30–50 parallel sub-agent dispatch precedent (2026-05-18).

## Operational continuity

1. Standard interaction pattern: sub-agent closes → team-lead verifies gate + reads close report → drafts consolidated status → user relays to supervisor → supervisor responds → user pastes back → team-lead executes.
2. Don't re-derive context that's already in `v0.3-close-summary.md`, `v0.3-roadmap.md`, `CLAUDE.md`, ADR-006, or this handover.
3. After Phase 4 closes + smoke matrix 5/5 VM == JIT empirically re-verified + supervisor ratifies + user authorizes `v0.3.0` tag → team-lead lands tag on main at user-authorized commit. v0.3 trajectory complete.

---

*End of handover. Read §First action before any dispatch.*
