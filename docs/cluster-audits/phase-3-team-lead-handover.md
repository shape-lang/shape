# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-21 at main HEAD `33ddace6`. Round 6+. Git holds prior
content; no archaeology here.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized. A
supervisor handles architectural calls; the user (strategic owner) authorizes
tags and language semantics. The user relays between team-lead and supervisor.

## Current state

| | |
|---|---|
| Main HEAD | `33ddace6` |
| Smoke matrix s1–s5 | 5/5 VM == JIT (canonical (ii) F' release-binary harness) |
| verify-merge / check-no-dynamic | 13/13 / exit 0 |
| v0.3 tag | NOT landed; decision-gated (see below) |

## Canonical (ii) F' smoke harness

Release binary; NOT pipe-to-tail.
```bash
out=$(timeout 30 ./target/release/shape run --mode $m $f 2>&1)
ec=$?; last=$(echo "$out" | tail -1)
```
Fixtures `tests/smokes/s{1..5}.shape`; expected s1 `4950` / s2 `30` / s3 `x`
/ s4 `2` / s5 `x`, all ec=0, VM == JIT.

## Round-6+ — merged + verified (10 workstreams)

WS-1 (W16.2-C op_new_array spread/comprehension), WS-2 (ζ-round loop inference
+ `id(None)`), WS-3 (Result/AnyError F1-F4 + array-rest), WS-4 (object
destructuring 4a/4b/4c), WS-6 + WS-6b (generic-arg monomorphization), WS-7
(JIT array-param SIGSEGV + legacy array-OOB), WS-9 (index-access-into-
unannotated-param inference + silent-wrong-result), WS-9b (property-access-
into-unannotated-param inference), WS-1b (bare-accumulator op_new_array).
Audit docs: `v0.3-w16-2-c-empty-literal-audit.md`, `v0.3-ws3-result-anyerror-
audit.md`, `v0.3-ws4-object-destructuring-audit.md`, `v0.3-ws8-consumer-
cascade-audit.md`, `v0.3-ws9-generic-fn-inference-audit.md`,
`v0.3-ws10-preexisting-items-audit.md`.

## Decision-gated — remaining v0.3 work (nothing autonomously dispatchable)

1. **WS-8 consumer-cascade fix-wave** — gated on the user's string-array
   implement-vs-clean-error ruling (WS-8 map relayed). Settled/unconditional:
   6 bool + 2 decimal VM/JIT divergences + 2 string bugs (bundle on dispatch).
   Open: implement string-array basic methods in v0.3 vs de-jargon-to-v0.4;
   `.contains()` disposition (alias `.includes` vs absent-by-design).
2. **Anonymous-object-factory inference loss** — `fn aabb(lo,hi){{min:lo,
   max:hi}}` spurious-rejects ("cannot infer ... unknown"); the true root of
   the ~14 `simulation.rs` Cluster-A tests. `apply_callsite_unions` widens a
   `types` map without binding the param variable in the unifier → object-
   literal fields from unannotated params freeze to `unknown`. Decision: (A)
   `apply_callsite_unions` restructure in v0.3 (architectural), or (B) rule it
   a v0.3 language limitation (annotate such params) + clean-error + v0.4
   restructure.
3. **WS-5** ADR-006 §2.7.13 amendment ratify (β1 `a287c795` / `278aa214`).
   **W16.2-B** scalar-element `dyn Trait` scope (`Array<dyn Speak>=[1,2]`
   stores raw Int64; `arr[0].s()` fails).

Pre-existing, WS-10-classified v0.4 (no action): bytecode-verifier "16
violations" stderr noise (stale rule, not a soundness gap); 314 shape-vm test
failures (285 phase-2c host-tier stubs + 29 other; 0 round-6 regressions).

## Dispatch hygiene (binding — round-6 lesson)

Agent `isolation: "worktree"` is unreliable here. Pre-create a sibling
worktree (`git worktree add /home/dev/dev/shape-lang/shape-r6-<ws> -b <branch>
<main-HEAD>`), dispatch WITHOUT `isolation`, pin the agent to that absolute
path as its first `cd`, forbid `cd`-ing to `/home/dev/dev/shape-lang/shape`.
Avoid `git stash` (shared `.git` stash-stack race). Each fix branch is built +
its reproducers re-verified on main post-merge before the next dispatch.

## Bindings — refuse on sight

Full detail: CLAUDE.md + ADR-006 §2.7.x. Operative: No-known-incorrectness-
ships-in-v0.3 (user 2026-05-20) — known-incorrect (crash / VM-JIT divergence /
wrong result / memory-unsafety / silent-wrong-output / spurious-reject of
valid code) → v0.3-gating; incomplete-but-CLEAN → v0.4-OK; jargon-dump is NOT
clean. Q3 ground-truth before disposition (and verify FULL breadth, not an
agent's chosen reproducers). Hypotheses INVESTIGATE not FIX. HEAD-commit-cite.
All CLAUDE.md Forbidden Patterns + Renames + ADR-006 §2.7.x. NO ARCHAEOLOGY;
no Co-Authored-By trailer; own all code quality.

## Cadence

Autonomous. Surface only on: defection-attractor framing / ADR amendment /
novel architectural gap needing a scope decision / user-decision item.
Relays ≤ ~80 lines; plain code fences.

## Close gates (every checkpoint)

`just check-clean` exit 0 · `verify-merge.sh` all pass · `check-no-dynamic.sh`
exit 0 · smoke s1–s5 5/5 VM == JIT · AGENTS.md row; no Co-Authored-By.

## v0.3 tag

After the gating set is empty + smoke 5/5 → relay close evidence to supervisor
→ supervisor ratifies → USER authorizes the `v0.3.0` tag. Land only on
explicit user authorization.

---

*Live operational state. Next rotation: verify HEAD + smoke 5/5 + verify-merge,
read the round-6 `cluster-audits/` docs, resolve the decision-gated items.*
