# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-22 at main HEAD `43d3f86c` (post audit-day landing
+ same-day dialogue additions). Round-6+ CLOSED; expanded-scope audits
LANDED; first-wave fix dispatch in flight. Git holds prior content; no
archaeology here.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized. A
supervisor handles architectural calls; the user (strategic owner) authorizes
tags and language semantics. The user relays between team-lead and supervisor.

## Current state — Round-6+ CLOSED, expanded-scope work in flight

| | |
|---|---|
| Main HEAD | `975536d3` (W17.3-4.1 + D-δ merged; audit corrections landed) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness; re-verified post-D-δ-merge 2026-05-22) |
| verify-merge / check-no-dynamic | 13/13 / exit 0 |
| Round-6+ no-known-incorrectness set | **EMPTY** per user 2026-05-20 binding |
| 2026-05-22 expanded-scope gating set | **OPEN.** Audits CLOSED A/B/C/D. **First wave: 2/3 MERGED** — W17.3-4.1 (`4b6d6833`); D-δ (`975536d3`); W16.2-J.1 surface-and-stopped + audit §3 corrected (NEW J.0 prereq). Slate 5→6 sub-clusters; effort 24–37h→30–47h. E (W18) + J (comptime trait) audit-first next round. F (Len + membership + KC #2 deletion) PENDING dispatch alongside W18. G.1/G.2 doc-truth gated on A–F+J. |
| v0.3.0 tag | NOT landed — gated on expanded-scope work + user authorization |

## 2026-05-22 expanded scope (binding — user, dialogue-supplemented same day)

Six v0.3-gating criteria (A–F) + the 2-step doc-truth round (G.1/G.2)
land pre-tag. **Sequencing:** code work first (parallel where territories
don't overlap) → doc-truth Step 1 → Step 2 → tag.

Full criteria + dispositions: `docs/v0.3-close-summary.md` §0.A.

| Crit | Workstream | Audit shape | Status at HEAD `43d3f86c` |
|---|---|---|---|
| A | **W16.2-J PHF-retirement** (architectural) | AUDIT-CLOSED | 5 sub-clusters (J.1–J.5); 24–37h; ADR-006 §2.7.24 amendment likely needed. **J.1 dispatched first-wave 2026-05-22.** |
| B | **W17.3-4 per-container `FieldType`** | AUDIT-CLOSED | 3 sub-clusters (.1–.3); 18–24h; no ADR amendment. **W17.3-4.1 MERGED `4b6d6833` 2026-05-22.** HeapKind::Set ordinal SURFACE retired empirically (HeapKind::HashMap ord 17 + HeapKind::HashSet ord 21 already exist; 4-table lockstep intact). .2 + .3 queued. |
| C | **Phase-2c host-tier marshal/snapshot rebuild** (ADR-006 §2.7.4) | AUDIT-CLOSED | 8 sub-clusters; 70–90h serial / 25–30h parallel. **4 supervisor architectural rulings:** 4 RULED, 2 direction-ruled-with-text-pending (per user 2026-05-22). Fix-dispatch unblocked. |
| D | **6 Known Constraints** | AUDIT-CLOSED | 4 v0.3-gating (D-α/β/γ/δ); 8–17h. **D-δ dispatched first-wave 2026-05-22.** KC #5 → DROP test (user); KC #2 → RESOLVE BY DELETION (user: delete `format_*` prefix globals; keep bare `format()` + `DateTime.format()`; no backwards-compat shim). KC #2 absorbed into F. |
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

## Dispatch hygiene (binding — round-6 lesson)

Agent `isolation: "worktree"` is unreliable here. Pre-create a sibling
worktree (`git worktree add /home/dev/dev/shape-lang/shape-r6-<ws> -b <branch>
<main-HEAD>`), dispatch WITHOUT `isolation`, pin the agent to that absolute
path as its first `cd`, forbid `cd`-ing to `/home/dev/dev/shape-lang/shape`.
Avoid `git stash` (shared `.git` stash-stack race). Each fix branch is built +
its reproducers re-verified on main post-merge before the next dispatch.

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

*Round-6+ CLOSED. 4-audit-day landing 2026-05-22; first-wave fix dispatch
2026-05-22 (W17.3-4.1 MERGED `4b6d6833`; W16.2-J.1 surface-and-stopped +
audit §3 corrected; D-δ in flight). All 2026-05-22 user-dialogue
decisions LANDED (KC #5 drop; KC #2 deletion; comptime trait YES; Send/Sync
v0.4). 4 supervisor architectural rulings ruled. Next: D-δ close +
second-wave fix dispatch (W16.2-J.0 prereq; W17.3-4.2; D-α audit-first;
D-β; D-γ); E (W18) + J (comptime trait) audit-first; F (Len + membership
+ KC #2 deletion) dispatch alongside W18.*
