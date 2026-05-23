# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-22 at main HEAD `3f364aea` (post round-7 wave-3
+ audit-round-2 close). Round-6+ CLOSED; expanded-scope audits LANDED;
round-7 W1 + W2 + W3 MERGED (8 fix sub-clusters + 4 audit-round dispositions
+ 2 round-2 audits). v0.3 criterion D (Known Constraints) + criterion F
(Len + membership + KC #2 deletion) CLOSED; criterion B 2/3 + criterion A
3/6 advanced; E + J audits closed. Git holds prior content; no archaeology
here.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized. A
supervisor handles architectural calls; the user (strategic owner) authorizes
tags and language semantics. The user relays between team-lead and supervisor.

## Current state — Round-6+ CLOSED, expanded-scope work in flight

| | |
|---|---|
| Main HEAD | `3f364aea` (R7 W3 8-merge batch + R2-audit 2-doc batch + close-doc record) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness; re-verified at HEAD `3f364aea` post 8-branch merge) |
| verify-merge / check-no-dynamic / check-clean | 13/13 / exit 0 / exit 0 |
| Round-6+ no-known-incorrectness set | **EMPTY** per user 2026-05-20 binding |
| 2026-05-22 expanded-scope gating set | **OPEN.** Audits CLOSED A/B/C/D/E/J. **Round-7 W3 + audit-round-2 (10 parallel dispatches):** J.1 PHF-deletion MERGED `99d53183`; J.2 VM-opcode macro MERGED `13868c8c`; J.3 JIT-FFI macro MERGED `fd7f25e7`; W17.3-4.2 compiler-integration MERGED `baefe876`; F bundle MERGED `fe197f70` (KC #2 deletion folded); D-α.1 closure-param MERGED `572791a3`; D-β string-join MERGED `44780043`; D-α.2 `.length` opcode-stamp MERGED `3f364aea`. E W18 + J comptime-trait audits CLOSED + landed at `1a94c9ee`. **Criteria status:** A 3/6 (J.4-rest + J.5 queued); B 2/3 (.3 runtime + snapshot/wire queued); C 0/8 (4 supervisor rulings landed; 8 sub-clusters queued); **D COMPLETE** (all 5 v0.3-gating sub-clusters merged; 6(b)/6(c) class-shifted to V3-S5/criterion-C territory); E AUDIT-CLOSED (4-cluster partition; 2 user decisions pending); **F COMPLETE**; J AUDIT-CLOSED (4-cluster partition; dispatch-ready). G.1/G.2 doc-truth gated on remaining A/B/C/E/J fix close. |
| v0.3.0 tag | NOT landed — gated on expanded-scope work + user authorization |

## 2026-05-22 expanded scope (binding — user, dialogue-supplemented same day)

Six v0.3-gating criteria (A–F) + the 2-step doc-truth round (G.1/G.2)
land pre-tag. **Sequencing:** code work first (parallel where territories
don't overlap) → doc-truth Step 1 → Step 2 → tag.

Full criteria + dispositions: `docs/v0.3-close-summary.md` §0.A.

| Crit | Workstream | Audit shape | Status at HEAD `43d3f86c` |
|---|---|---|---|
| A | **W16.2-J PHF-retirement** (architectural) | AUDIT-CLOSED (REVISED) | 6 sub-clusters (J.0–J.5); 30–47h; ADR-006 §2.7.24 amendment likely needed. **W16.2-J.0 MERGED `fbe86020` 2026-05-22**; J.1 hypothesis VALIDATED via probe; J.1 + J.2 + J.3 queued next wave. |
| B | **W17.3-4 per-container `FieldType`** | AUDIT-CLOSED | 3 sub-clusters (.1–.3); 18–24h; no ADR amendment. **W17.3-4.1 MERGED `4b6d6833` 2026-05-22.** HeapKind::Set ordinal SURFACE retired empirically (HeapKind::HashMap ord 17 + HeapKind::HashSet ord 21 already exist; 4-table lockstep intact). .2 + .3 queued. |
| C | **Phase-2c host-tier marshal/snapshot rebuild** (ADR-006 §2.7.4) | AUDIT-CLOSED | 8 sub-clusters; 70–90h serial / 25–30h parallel. **4 supervisor architectural rulings:** 4 RULED, 2 direction-ruled-with-text-pending (per user 2026-05-22). Fix-dispatch unblocked. |
| D | **6 Known Constraints** | AUDIT-CLOSED + D-α audit-first CLOSED | **5 v0.3-gating sub-clusters** (D-α split into D-α.1 + D-α.2 per D-α audit): D-α.1 closure-param inference loss (4–8h); D-α.2 flow-sensitive type loss through reassignment chain (1–3h, orthogonal); D-β string-join Bool-receiver dispatch (1–3h); **D-γ MERGED `bf864620` 2026-05-22** (UFCS-generic-monomorphization → linker-ZERO-sentinel → __main__-self-recursion → infinite loop; fixed via probe-then-filter guard); **D-δ MERGED `975536d3` 2026-05-22**. Remaining 6–14h. D-α.1 + D-α.2 + D-β parallel-dispatchable. KC #5 → DROP test (user); KC #2 → RESOLVE BY DELETION (user; folded into F). |
| E | **W18 content-rendering kind-threaded rebuild** (NEW 2026-05-22 — v0.3-gating regression) | AUDIT-FIRST mandatory; NEXT audit round | Phase 2b deleted ~600 LoC dispatch chain; user-value → ContentNode rebuild missing. Symptoms: enum Display leaks `{__variant:N,__payload_0:...}`. ~2–4 sessions. |
| F | **Len trait + membership-naming standardization + KC #2 format-deletion** (NEW 2026-05-22; KC #2 folded) | Direct dispatch (close-coupled w/ W18) | New `trait Len { method len() -> int; method isEmpty() -> bool }`; rename `Set.size() → Set.len()`; standardize `.includes(x)` / `.some(\|x\| pred)`; reject `.contains()`; delete `format_*` prefix globals (KC #2). ~1 session. |
| G.1 | **Doc-truth Step 1** (DOC↔CODE reconciliation) | Parallel slice agents per book chapter | GATED on A–F + J close + main stable |
| G.2 | **Doc-truth Step 2** (USER-followable validation) | Same partition, on updated docs | GATED on Step 1 fully merging |
| J | **Comptime trait primitive** (NEW 2026-05-22 — user YES) | Audit-first short pass; direct dispatch | `comptime trait` + `comptime impl Trait for Type` parser + type-checker + comptime-evaluator dispatch. **Scope DISCIPLINE = JUST the primitive + dispatch + error.** NO const-fn markers / compile-time-generics / other comptime sprawl (v0.4). Refusal #10 family applies if scope creeps. ~1–2 sessions. |

**Classification rule for doc-truth (G.1 + G.2):**
- (a) doc aspirational/wrong, code right → fix doc in place
- (b) doc was a contract, code wrong → NEW known-incorrect → v0.3-gating
- (c) doc describes v0.4 feature → annotate doc + add to §5 inventory

**Eq/Display struct-semantic clarifications** fold into G.1 (a)/(b)/(c)
classification per the standard rule.

## User decisions — all landed (2026-05-22)

- KC #5 `object_len_function` → **DROP test.**
- KC #2 `format()` shadowing → **RESOLVE BY DELETION** (delete `format_*`
  prefix globals; keep bare `format()` + `DateTime.format()`; no
  backwards-compat shim). Folded into criterion F.
- Comptime trait primitive → **YES** (criterion J added). Scope discipline:
  JUST primitive + dispatch + error. Refusal #10 if scope creeps.
- Send/Sync → **v0.4** (design-question, not feature-gap).

**No user decisions outstanding from the 2026-05-22 dialogue.**

## Send/Sync — CONFIRMED v0.4 (2026-05-22)

User annotation: "Rust way too complicated; needs deep design thought."
§5.13 of `v0.3-close-summary.md` records this as a v0.4 design-question
workstream, not a feature-gap.

## Trajectory

~**13 ± 4 sessions** to v0.3 tag (was ~10 ± 3 pre-2026-05-22 dialogue
additions). Each addition bounded; they compound.

## Round-6+ workstreams (15 merged + verified) — preserved

WS-1 (W16.2-C empty-literal/spread/comprehension), WS-1b (bare-accumulator),
WS-2 (ζ-round), WS-3 (Result/AnyError F1–F4 + array-rest), WS-4 (object
destructuring 4a/4b/4c), WS-6 + WS-6b (generic-arg monomorphization), WS-7
(JIT array-param SIGSEGV + legacy OOB), WS-8 (consumer-cascade kind-generic
bundle), WS-9 (index-access inference + silent-wrong-result), WS-9b
(property-access inference), WS-9c (factory inference / `apply_callsite_unions`),
WS-10b (verifier-noise fix + 29-classification), WS-11 (REPL cross-cell
persistence), WS-12 (Option as-cast).

ADR-006 §2.7.13 amendment landed (commit `45dedd02`): `TypedField.receiver
→ TypedObjectPtr` per β1; `TypedIndex` deletion recorded.

## Canonical (ii) F' smoke harness

Release binary; NOT pipe-to-tail.
```bash
out=$(timeout 30 ./target/release/shape run --mode $m $f 2>&1)
ec=$?; last=$(echo "$out" | tail -1)
```
Fixtures `tests/smokes/s{1..5}.shape`; expected s1 `4950` / s2 `30` / s3 `x`
/ s4 `2` / s5 `x`, all ec=0, VM == JIT.

## Pending the v0.3 tag

1. A–D land + smoke 5/5 preserved at every checkpoint.
2. Doc-truth Step 1 fully merges before Step 2 starts (no stale-doc race).
3. Doc-truth Step 2 closes; any (b)-class findings folded into the gating
   set + fixed.
4. Relay close-evidence to supervisor; supervisor ratifies.
5. **User** authorizes the `v0.3.0` tag (release-tag authority is the
   user's lane).
6. Team-lead lands the tag at the authorized commit. Do NOT annotate the
   tag pre-authorization.

## v0.4 inventory (close-summary §5 — POST-expansion pruning)

W15.2-K stdlib doc pages; §4.D.10 emission-rename; shape-web-per-agent-
worktree-infra; annotated-closure-param parse + `sortBy` closure defect.
**MOVED to v0.3 §0.A:** W16.2-J PHF-retirement (criterion A); W17.3-4
per-container FieldType (B); phase-2c host-tier marshal/snapshot rebuild
(C). v0.4 still holds: W16.2-B scalar-element `dyn Trait` arrays, native
JIT decimal-scalar codegen, REPL declaration-statement `false` print,
WS-9c `tyvar` marker → `TypeAnnotation::Variable` variant.

## Bindings — refuse on sight

Full detail: CLAUDE.md + ADR-006 §2.7.x. Operative under expanded scope:

- **No-known-incorrectness-ships-in-v0.3** (user 2026-05-20): known-incorrect
  (crash / VM-JIT divergence / wrong result / memory-unsafety / silent-wrong-
  output / spurious-reject of valid code) → v0.3-gating; incomplete-but-CLEAN
  → v0.4-OK; jargon-dump NOT clean.
- **Class-shift verify-at-close (supervisor 2026-05-23):** KC #6(b)
  typed_closure_in_array_map + KC #6(c) test_complex_bubble_sort
  class-shifted to V3-S5 / criterion-C territory at R7 W3 close. **Standing
  rule:** if criterion-C close doesn't sweep these EMPIRICALLY (re-run
  both tests at criterion-C close), they come back as un-resolved
  v0.3-gating items. No deferral-by-renaming.
- **W16.2-J must be REAL retirement** — no half-retire ("preserve PHF for
  one edge case" / "rename to a less suspicious name"). Refusal #10 family.
  Monomorphization-to-concrete only — NEVER reintroduce a tagged/dynamic
  path.
- **Doc-truth round: RECONCILE-doc-to-shipped, NOT aspirational rewrite.**
  If a slice wants to expand language scope to match an aspirational doc,
  surface it.
- **Q3 ground-truth before disposition + full-breadth verification.**
- **Run-verify binding (supervisor 2026-05-22):** every repro in a
  surfacing MUST run-verify at HEAD before relay — same standing as the
  file:line / commit-hash grep-verify rule.
- All CLAUDE.md Forbidden Patterns + Renames + ADR-006 §2.7.x. NO ARCHAEOLOGY.
  No Co-Authored-By trailer; own all code quality.

## Dispatch hygiene (binding — round-6 lesson + R7 W3 git-stash hardening)

Agent `isolation: "worktree"` is unreliable here. Pre-create a sibling
worktree (`git worktree add /home/dev/dev/shape-lang/shape-<branch> -b
<branch> <main-HEAD>`), dispatch WITHOUT `isolation`, pin the agent to
that absolute path as its first `cd`, forbid `cd`-ing to
`/home/dev/dev/shape-lang/shape`. Each fix branch is built + its
reproducers re-verified on main post-merge before the next dispatch.

**`git stash` ABSOLUTE BINDING (R7 W3 hardening — supervisor 2026-05-23):**

> Parallel-dispatch agents are FORBIDDEN from `git stash` in any form.
> State-recovery uses targeted `git checkout -- <file>`, `git reset
> HEAD <file>`, or explicit commits in the agent's own pinned worktree.
> The shared `.git/refs/stash` stack is off-limits. Every dispatch
> prompt for a parallel-worktree agent MUST include this verbatim.

R7 W3 hit 5 violations (J.1, F, W17.3-4.2, J.2, D-α.2 × 2) under
baseline-verification pressure — prior "serialize git-stash ops" form
was too soft. Hard form is enforceable at dispatch-prompt-review.

**Audit-day exception:** read-only audits (no source changes) can run in
the main repo without worktrees provided they each write only their own
audit doc and stop without committing. Team-lead commits the audit docs
together at audit-round close.

## Cadence

Autonomous. Surface only on: (1) defection-attractor framing; (2) ADR
amendment genuinely needed (W16.2-J likely qualifies; surface drafted
text); (3) novel architectural gap needing a scope decision; (4) user-
decision item. Do NOT recreate a per-round ratification gate.
Relays ≤ ~80 lines; plain code fences (user pastes verbatim).

Multi-session rotation expected. Refresh this doc at every rotation
(don't let it drift round-stale like the prior cycle).

## Close gates (every checkpoint)

`just check-clean` exit 0 · `verify-merge.sh` all 13 pass ·
`check-no-dynamic.sh` exit 0 · smoke s1–s5 5/5 VM == JIT (canonical (ii)
F' release-binary harness) · AGENTS.md row; no Co-Authored-By trailer.

---

*Round-6+ CLOSED. Round-7 W1 (3 dispatches): W17.3-4.1 + D-δ MERGED;
J.1 surface-and-stop + audit §3 corrected. Round-7 W2 (3 dispatches):
D-α audit (2-family split); J.0 MERGED; D-γ MERGED. **Round-7 W3 +
audit-round-2 (10 parallel dispatches) MERGED 2026-05-22**: J.1/J.2/J.3
(W16.2-J 3/6 merged) + W17.3-4.2 + F bundle (KC #2 folded) + D-α.1 +
D-α.2 + D-β; 2 audit-round-2 audits closed (E W18 + J comptime trait).
**6 audit-layer imprecisions logged for cumulative tally:** (a) D-α
1-family hypothesis falsified at audit (2-family); (b) D-α.2
reassignment-chain hypothesis falsified at fix (single .length stamp);
(c) D-β kind-tracker mis-stamp hypothesis falsified at fix (4-stage
compile-time cascade through builder-leak); (d) W17.3-4 audit §5.B
Set-literal + HashMap-literal syntax claims falsified (neither in Pest
grammar); (e) KC #6(b) typed_closure_in_array_map stack-overflow anchor
incidentally retired between audit + fix dispatch (class-shift to
V3-S5); (f) KC #6(c) bubble_sort PASS-gate inherited by V3-S5 (criterion
C territory). **5 git-stash binding violations** (J.1, F, W17.3-4.2,
J.2, D-α.2 × 2) — under parallel-worktree pressure when verifying
baselines. Future-dispatch addendum mandatory: instruct agents to use
`git show HEAD:path` + saved-patch backups + dedicated baseline
worktree, never `git stash`. **Next**: criterion C phase-2c fix-dispatch
when items 2 + 6 text-ratify lands; J fix-dispatch (4 sub-clusters);
A J.4-rest + J.5 dispatch; B W17.3-4.3 dispatch; E W18.0/.1/.2/.3
dispatch (gated on 2 user decisions). v0.3-gating gating-set remaining:
A 3/6, B 1/3, C 8/8, E 4/4 (+2 user decisions), J 4/4.*
