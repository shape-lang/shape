# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-24 at main HEAD `a635ae36` (post Round-8 W4 close).
R7 + R8 W1 + R8 W2 + R8 W3 + **R8 W4 (7 merges: 6 fix + 1 audit)**.
v0.3 criterion progress: **A 5.5/6** (J.0/.1/.2/.3/J.5a/.5b/.5c/.5d/.5f
merged; J.4-rest partial; J.5e iterator-protocol → v0.4 per audit
recommendation); B 3/3 COMPLETE; **C 8/8 + 2 RESIDUALS**; D COMPLETE;
**E COMPLETE** (W18.0+.2+.3+.4+.5+.6 all merged; W18.1 v0.4-deferred);
F COMPLETE; J 3/4.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized. A
supervisor handles architectural calls; the user (strategic owner) authorizes
tags and language semantics. The user relays between team-lead and supervisor.

## Current state — Round-6+ CLOSED, expanded-scope work in flight

| | |
|---|---|
| Main HEAD | `a635ae36` (R8 W4 7-merge batch incl. 2 conflict-resolutions on v2_array_detect.rs take-both) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness; re-verified at HEAD `a635ae36` post-merge) |
| verify-merge / check-no-dynamic / check-clean | 13/13 / exit 0 / exit 0 |
| Round-6+ no-known-incorrectness set | **EMPTY** per user 2026-05-20 binding |
| 2026-05-22+05-23 expanded-scope gating set | **OPEN.** Audits CLOSED A/B/C/D/E/J + 2 R8-round-audit-2 docs (W18 deep-research + J.4-rest re-audit). **Round-8 W1 (9 merges) + W2 (8 merges incl. 2 audits) at main HEAD `ae34b01f`.** R8 W2 fix merges: V3-S5 `b4250000` + W17-snapshot-resume `014cdf60` + W17-typed-module-exports `a4fa323d` + W17-foreign-ffi `aadfdc2d` + J-CT.1 `eaea9c87` + J-CT.2 `ae34b01f`. **Criteria status at HEAD ae34b01f:** **A 3.1/6** (J.5 60-90h scope mapped; 4 architectural decisions queued for user/supervisor: tuple-carrier / deep-equality / closure-return-kind / v0.4-deferral-line); **B 3/3 COMPLETE**; **C 7.5/8** (residuals: W17-marshal-return-arms + W17-typed-module-exports-followup-constant-pool); **D COMPLETE**; **E 2/4 + deep-research-audit RESHAPED** (W18.1 HOLDS indefinitely; 4 user-decisions queued: styled f-string return type / c-string retirement timing / builder API shape / v0.3 scope line; 4-cluster partition W18.3-.6 28-38h total); **F COMPLETE**; **J 3/4** (J-CT.3 already met cumulatively). 2 ADR §2.7.4 amendment texts DRAFTED for supervisor text-ratify (items 2+6 direction-ruled territory). G.1/G.2 doc-truth gated on remaining A/E + 2 phase-2c residuals. |
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

**`git stash` ABSOLUTE BINDING (R7 W3 hardening — supervisor 2026-05-23;
R8 W4 pre-commit-hook authorization — supervisor 2026-05-24):**

> Parallel-dispatch agents are FORBIDDEN from `git stash` in any form.
> State-recovery uses targeted `git checkout -- <file>`, `git reset
> HEAD <file>`, or explicit commits in the agent's own pinned worktree.
> The shared `.git/refs/stash` stack is off-limits. Every dispatch
> prompt for a parallel-worktree agent MUST include this verbatim.

**Mechanical enforcement landed R8 W4 close 2026-05-24:** pre-commit
hook at `.git/hooks/pre-commit` rejects any commit while `git stash
list` is non-empty (shared across all worktrees via shared `.git/`).
Recovery instructions baked into the hook's stderr message: `git
stash pop` (or `git stash drop` if recovered) + `git stash clear` +
retry commit. **Tested empirically:** stash present → hook fires +
exit 1; stash empty → commit succeeds.

R7 W3 + R8 W2 + R8 W3 + R8 W4 cumulative: 12 violations (5 + 3 + 1 +
3 self-reported) under baseline-verification pressure despite the
verbatim binding + worked-example. Pre-commit hook is the mechanical
fallback per supervisor 2026-05-24 disposition.

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

*Round-6+ CLOSED. R7 W1-W3 + audit-round-2 closed. **Round-8 W1
(9 parallel dispatches MERGED 2026-05-23):** J-CT.0 + W17.3-4.3 +
C1 + C2 + C3 + J.4-rest (partial) + T1 + W18.0 + W18.2. v0.3 criterion
B (per-container FieldType) **COMPLETE**; C 4/8; E 2/4; J 1/4; A 3.1/6.
**Cumulative audit-layer imprecisions: 8 logged** (R7 #1-6 + R8 #7
truncation-mid-build pattern + R8 #8 J.4-rest scope mis-classification
— audit framed as "lowest-frequency residuals" but ~90% needs
primitive-layer build, distinct territory). **Cumulative git-stash
violations: 6** (5 in R7 W3 + 1 in R8 C3 self-reported despite hardened
binding). **R8 commit-first procedure VALIDATED** for build-wait
truncation recovery (C1 + W18.2 finalized via that pattern; bake into
R8 W2 dispatch template). **Next: R8 W2** — primary territory: J-CT.1
type-checker + J-CT.2 evaluator + J-CT.3 integration; J.5
primitive-layer scope re-audit before dispatch; W17-snapshot-resume +
W17-typed-module-exports + W17-foreign-ffi + V3-S5 host-tier-eval (now
T1-unblocked); W18.1 (HOLDS on user Item 2 ruling). v0.3-gating
remaining: A 2.9/6 + B 0/3 + C 4/8 + E 2/4 + J 3/4 + G.1/G.2 doc-truth.*
