# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-22 at main HEAD `45dedd02`. Round-6+ CLOSED. Git holds
prior content; no archaeology here.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized. A
supervisor handles architectural calls; the user (strategic owner) authorizes
tags and language semantics. The user relays between team-lead and supervisor.

## Current state — Round-6+ CLOSED, awaiting tag authorization

| | |
|---|---|
| Main HEAD | `45dedd02` (post §2.7.13 amendment) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness) |
| verify-merge / check-no-dynamic | 13/13 / exit 0 |
| Round-6+ no-known-incorrectness set | **EMPTY** per user 2026-05-20 binding |
| v0.3.0 tag | NOT landed — awaiting user authorization |

## Round-6+ workstreams (15 merged + verified)

WS-1 (W16.2-C empty-literal/spread/comprehension), WS-1b (bare-accumulator),
WS-2 (ζ-round), WS-3 (Result/AnyError F1–F4 + array-rest), WS-4 (object
destructuring 4a/4b/4c), WS-6 + WS-6b (generic-arg monomorphization), WS-7
(JIT array-param SIGSEGV + legacy OOB), WS-8 (consumer-cascade kind-generic
bundle), WS-9 (index-access inference + silent-wrong-result), WS-9b
(property-access inference), WS-9c (factory inference / `apply_callsite_unions`),
WS-10b (verifier-noise fix + 29-classification), WS-11 (REPL cross-cell
persistence), WS-12 (Option as-cast).

ADR-006 §2.7.13 amendment landed (commit `45dedd02`): `TypedField.receiver
→ TypedObjectPtr` per β1 (`a287c795` / `278aa214`); `TypedIndex` deletion
recorded; full strike set applied (post-apply check = 1 line, the single
canonical historical deletion-note).

## Canonical (ii) F' smoke harness

Release binary; NOT pipe-to-tail.
```bash
out=$(timeout 30 ./target/release/shape run --mode $m $f 2>&1)
ec=$?; last=$(echo "$out" | tail -1)
```
Fixtures `tests/smokes/s{1..5}.shape`; expected s1 `4950` / s2 `30` / s3 `x`
/ s4 `2` / s5 `x`, all ec=0, VM == JIT.

## Pending the v0.3 tag

1. Relay close-evidence to supervisor (this turn).
2. Supervisor ratifies close.
3. **User** authorizes the `v0.3.0` tag (CLAUDE.md / supervisor handover
   binding — release-tag authority is the user's lane).
4. Team-lead lands the tag at the authorized commit. Do NOT annotate the
   tag pre-authorization.

## v0.4 inventory (close-summary §5)

In `docs/v0.3-close-summary.md` §5.9–§5.12 (new in round-6+): W16.2-J
PHF-retirement workstream; scalar-element `dyn Trait` arrays (W16.2-B);
native JIT decimal-scalar codegen; REPL declaration-statement `false`
print; WS-9c `tyvar` marker → `TypeAnnotation::Variable` variant; plus
the prior set (W15.2-K, §4.D.10, W17.3-4, shape-web-worktree-infra,
annotated-closure-param parse + `sortBy` defect, phase-2c host-tier
marshal/snapshot rebuild).

## Bindings — refuse on sight

Full detail: CLAUDE.md + ADR-006 §2.7.x. Operative:
- **No-known-incorrectness-ships-in-v0.3** (user 2026-05-20): known-incorrect
  (crash / VM-JIT divergence / wrong result / memory-unsafety / silent-wrong-
  output / spurious-reject of valid code) → v0.3-gating; incomplete-but-CLEAN
  → v0.4-OK; jargon-dump NOT clean.
- **Q3 ground-truth before disposition + full-breadth verification.**
- **Run-verify binding extension (supervisor 2026-05-22):** every repro in
  a surfacing MUST run-verify at HEAD before relay — same standing as the
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

## Cadence

Autonomous. Surface only on: defection-attractor framing / ADR amendment /
novel architectural gap needing a scope decision / user-decision item.
Relays ≤ ~80 lines; plain code fences (user pastes verbatim).

## Close gates (every checkpoint)

`just check-clean` exit 0 · `verify-merge.sh` all pass · `check-no-dynamic.sh`
exit 0 · smoke s1–s5 5/5 VM == JIT · AGENTS.md row; no Co-Authored-By trailer.

---

*Round-6+ CLOSED. The v0.3-close-summary.md §0/§4/§5 are refreshed; the
close-relay carries the final close evidence. Next rotation: read this doc +
the close-summary, verify HEAD + smoke 5/5, and proceed per the user's tag
authorization.*
