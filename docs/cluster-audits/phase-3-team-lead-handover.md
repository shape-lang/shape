# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-21 at main HEAD `b61f8bf2`. Round 6. Git holds prior
content; no archaeology here.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized. A
supervisor handles architectural calls; the user (strategic owner) authorizes
tags and language semantics. The user relays between team-lead and supervisor.

## Current state

| | |
|---|---|
| Main HEAD | `b61f8bf2` |
| Smoke matrix s1–s5 | 5/5 VM == JIT (canonical (ii) F' release-binary harness) |
| verify-merge / check-no-dynamic | 13/13 / exit 0 |
| v0.3 tag | NOT landed; gated on the round-6 fix set below |

## Canonical (ii) F' smoke harness

Release binary; NOT pipe-to-tail.
```bash
out=$(timeout 30 ./target/release/shape run --mode $m $f 2>&1)
ec=$?; last=$(echo "$out" | tail -1)
```
Fixtures `tests/smokes/s{1..5}.shape`; expected s1 `4950` / s2 `30` / s3 `x`
/ s4 `2` / s5 `x`, all ec=0, VM == JIT.

## Round-6 workstreams

Four audits closed (`docs/cluster-audits/v0.3-w16-2-c-empty-literal-audit.md`,
`v0.3-ws3-result-anyerror-audit.md`, `v0.3-ws4-object-destructuring-audit.md`).

| WS | Scope | State |
|----|-------|-------|
| WS-2 | ζ-round: loop-body inference + `id(None)` hang | ✅ merged, verified |
| WS-3 | Result/AnyError: F1 JIT OOB / F2 `?` / F3 `!!` / F4 jargon / array-rest | ✅ merged, verified |
| WS-4 | Object destructuring 4a/4b/4c | ✅ merged, verified |
| WS-6 | Generic free-fn calls accept non-scalar args | ✅ merged, verified (partial — see WS-6b) |
| WS-6b | Generic args: inferred-type variables + HashMap-result type loss | 🔄 fix agent running |
| WS-7 | JIT SIGSEGV on unannotated array param `xs[i]` + legacy array-OOB | 🔄 fix agent running |
| WS-1 | V3-S5 ckpt-5/6 W16.2-C empty-literal / spread / comprehension | ⏸ held — pending 6.B.1 codegen ratify |
| WS-5 | ADR-006 §2.7.13 amendment ratify | ⏸ surfaced — pending ratify |

**Ground-truthed findings (new this round, beyond the 5-family set):**
- WS-6 fixed generic args for direct constructors + annotated bindings but
  NOT inferred-type variables (`let p = P{a:7}; id(p)` still fails) nor the
  HashMap generic-call result type → WS-6b.
- The "F1 second defect" is worse than the audit's silent-OOB framing: an
  unannotated array param `fn get(xs,i){xs[i]}` JIT-tier-compiled SIGSEGVs
  even on a valid in-bounds index → WS-7.

WS-6 carries a tracked v0.4 follow-up: JIT struct-value codegen out of a
struct-monomorphized specialization is unsound; `compile_program_selective`
honestly refuses it and `--mode jit` falls back to the interpreter (correct
output, no crash, no divergence).

## Dispatch hygiene (binding — round-6 lesson)

Agent `isolation: "worktree"` is unreliable here (stale base + agents `cd` into
the main repo). For every dispatch: **pre-create a sibling worktree**
(`git worktree add /home/dev/dev/shape-lang/shape-r6-<ws> -b <branch> <main-HEAD>`),
dispatch WITHOUT `isolation`, pin the agent to that absolute path as its first
`cd`, and explicitly forbid `cd`-ing to `/home/dev/dev/shape-lang/shape`.

## Bindings — refuse on sight

Full detail: CLAUDE.md + ADR-006 §2.7.x. Operative:
- **No-known-incorrectness-ships-in-v0.3** (user 2026-05-20): known-incorrect
  (crash / VM-JIT divergence / wrong result / memory-unsafety / silent-wrong-
  output / spurious-reject of valid code) → v0.3-gating. Incomplete-but-CLEAN
  → v0.4-OK. A jargon-dump stub is NOT "clean".
- **Q3 recursive pre-flight:** ground-truth before disposition — and verify
  the FULL breadth, not just an agent's chosen reproducers (WS-6 "enum works"
  was true only for direct/annotated forms).
- Hypotheses INVESTIGATE not FIX until empirical bisect.
- HEAD-commit-cite every surfacing; workspace cargo-check at API-change closes.
- All CLAUDE.md Forbidden Patterns + Renames + ADR-006 §2.7.x + 4-table
  HeapKind lockstep + 5-arm receiver-recovery.
- NO ARCHAEOLOGY in living docs; no Co-Authored-By trailer; own all code
  quality (never "pre-existing").

## Cadence

Autonomous. Surface to supervisor (via user) only on: (1) defection-attractor
framing; (2) ADR amendment needed; (3) novel architectural gap needing a scope
decision; (4) user-decision item. Relays ≤ ~80 lines; plain code fences.

## Close gates (every checkpoint)

`just check-clean` exit 0 · `bash scripts/verify-merge.sh` all pass ·
`bash scripts/check-no-dynamic.sh` exit 0 · smoke s1–s5 5/5 VM == JIT ·
AGENTS.md row appended; no Co-Authored-By trailer. Each fix branch is built
and its reproducers re-verified on main post-merge.

## Pre-existing observation (not round-6 scope)

The bytecode verifier prints `Bytecode verification failed: 16 violation(s)`
to stderr on **every** program run (stdlib-prelude `Trusted` opcodes lacking
FrameDescriptors + `__main__ slot 0 Unknown kind`); execution proceeds
correctly. A v0.3-close polish/decision item — flagged, not actioned.

## v0.3 tag

After the round-6 fix set closes + no-known-incorrectness set empty + smoke
5/5 → relay close evidence to supervisor → supervisor ratifies → USER
authorizes the `v0.3.0` tag. Land the tag only on explicit user authorization.

---

*Live operational state. Next rotation: verify HEAD + smoke 5/5 + verify-merge,
read the round-6 `cluster-audits/` docs, continue the fix wave.*
