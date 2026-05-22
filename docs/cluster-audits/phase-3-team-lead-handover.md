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
| Main HEAD | `43d3f86c` (post 4-audit-day landing + same-day dialogue refresh) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness; re-verified 2026-05-22 post-refresh) |
| verify-merge / check-no-dynamic | 13/13 / exit 0 |
| Round-6+ no-known-incorrectness set | **EMPTY** per user 2026-05-20 binding |
| 2026-05-22 expanded-scope gating set | **OPEN** — audits CLOSED for A/B/C/D; first-wave fix dispatch in flight (W16.2-J.1 + W17.3-4.1 + D-δ); E (W18) audit-first PENDING next round; F (Len trait + membership) PENDING dispatch alongside W18; doc-truth round G.1/G.2 gated on A–F |
| v0.3.0 tag | NOT landed — gated on expanded-scope work + user authorization |

## 2026-05-22 expanded scope (binding — user, dialogue-supplemented same day)

Six v0.3-gating criteria (A–F) + the 2-step doc-truth round (G.1/G.2)
land pre-tag. **Sequencing:** code work first (parallel where territories
don't overlap) → doc-truth Step 1 → Step 2 → tag.

Full criteria + dispositions: `docs/v0.3-close-summary.md` §0.A.

| Crit | Workstream | Audit shape | Status at HEAD `43d3f86c` |
|---|---|---|---|
| A | **W16.2-J PHF-retirement** (architectural) | AUDIT-CLOSED | 5 sub-clusters (J.1–J.5); 24–37h; ADR-006 §2.7.24 amendment likely needed. **J.1 dispatched first-wave 2026-05-22.** |
| B | **W17.3-4 per-container `FieldType`** | AUDIT-CLOSED | 3 sub-clusters (.1–.3); 18–24h; no ADR amendment. Supervisor SURFACE: HeapKind::Set ordinal. **W17.3-4.1 dispatched first-wave 2026-05-22.** |
| C | **Phase-2c host-tier marshal/snapshot rebuild** (ADR-006 §2.7.4) | AUDIT-CLOSED | 8 sub-clusters; 70–90h serial / 25–30h parallel. **4 supervisor architectural under-specs surfaced.** Fix-dispatch pending supervisor disposition. |
| D | **6 Known Constraints** | AUDIT-CLOSED | 4 v0.3-gating (D-α/β/γ/δ); 8–17h. **D-δ dispatched first-wave 2026-05-22.** KC #5 → DROP test (user decided). KC #2 PENDING user. |
| E | **W18 content-rendering kind-threaded rebuild** (NEW 2026-05-22 — v0.3-gating regression) | AUDIT-FIRST mandatory; NEXT audit round | Phase 2b deleted ~600 LoC dispatch chain; user-value → ContentNode rebuild missing. Symptoms: enum Display leaks `{__variant:N,__payload_0:...}`. ~2–4 sessions. |
| F | **Len trait + membership-naming standardization** (NEW 2026-05-22) | Direct dispatch (close-coupled w/ W18) | New `trait Len { method len() -> int; method isEmpty() -> bool }`; rename `Set.size() → Set.len()`; standardize `.includes(x)` / `.some(\|x\| pred)`; reject `.contains()`. ~1 session. |
| G.1 | **Doc-truth Step 1** (DOC↔CODE reconciliation) | Parallel slice agents per book chapter | GATED on A–F close + main stable |
| G.2 | **Doc-truth Step 2** (USER-followable validation) | Same partition, on updated docs | GATED on Step 1 fully merging |

**Classification rule for doc-truth (G.1 + G.2):**
- (a) doc aspirational/wrong, code right → fix doc in place
- (b) doc was a contract, code wrong → NEW known-incorrect → v0.3-gating
- (c) doc describes v0.4 feature → annotate doc + add to §5 inventory

**Eq/Display struct-semantic clarifications** fold into G.1 (a)/(b)/(c)
classification per the standard rule.

## Pending user-decision items

- **KC #2 `format()` shadowing** — supervisor rec: rename globals to
  `format_pct/format_num/format_string`. User not yet ruled; carry to
  next relay.
- **Comptime trait primitive (yes/no)** — surfaced 2026-05-22; if yes,
  ~1–2 sessions for parser + type-checker + comptime-evaluator dispatch;
  scope-discipline = JUST the primitive, no const-fn-markers /
  compile-time-generics sprawl.

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

*Round-6+ CLOSED; audit-day landing for A/B/C/D landed 2026-05-22 at
commit `43d3f86c`. First-wave fix dispatch (W16.2-J.1 + W17.3-4.1 + D-δ)
in flight. E (W18 content-rendering rebuild) + F (Len trait + membership-
naming) added same day per user dialogue. Next: settle the supervisor
architectural surfaces (4 phase-2c under-specs; W17.3-4 HeapKind::Set
ordinal; W16.2-J ADR amendment text) + the 2 user-decision items (KC #2;
comptime trait primitive); land second-wave fix dispatch.*
