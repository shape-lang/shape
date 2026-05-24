# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-24 at main HEAD `79783499` (R8 W5 close — G.1 Step 1
doc-truth audits + shape-web (a)-class batch merged; **R8 W6 dispatch in
flight**). v0.3 code criteria substantially complete (A 5.5/6, B/C/D/E/F
COMPLETE, J 3/4); G.1 Step 1 surfaced 16 doc/code-drift items dispositioned
into 5 groups by supervisor 2026-05-24.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized.
Supervisor handles architectural calls; user authorizes tags + language
semantics. User relays between team-lead and supervisor.

## Current state

| | |
|---|---|
| Main HEAD | `79783499` (R8 W5 doc-only close) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness; re-verified at HEAD `79783499` start of R8 W6) |
| verify-merge / check-no-dynamic / check-clean | 13/13 / exit 0 / exit 0 |
| git-stash pre-commit hook | DEPLOYED (R8 W5 ZERO violations) |
| Co-Authored-By trailers (cumulative) | 0 |
| Bad-code merges (cumulative) | 0 |
| v0.3.0 tag | NOT landed — gated on R8 W6 dispositions + user authorization |

## R8 W6 dispatch — Supervisor 16-item disposition (binding 2026-05-24)

Apply user 2026-05-20 no-known-incorrectness binding: memory-unsafety /
VM-JIT divergence / silent-wrong-output / spurious-reject of valid code
ship as v0.3-gating. Incomplete-but-CLEAN ships as v0.4-OK with surface-
and-stop messaging.

| Group | Disposition | Items |
|---|---|---|
| **1** | v0.3-gating MUST FIX (memory-unsafety / divergence; unconditional) | B JIT TypedArrayPushI64 FrameDescriptor SEGFAULT (audit-first); E b-4 transport::tcp() VM/JIT divergence (audit-first) |
| **2** | v0.3-gating panic→structured-error conversion per ADR-006 §2.7.14 SURFACE (feature impl → v0.4) | E b-1 Wave 5d intrinsic ~40+ todo!()'s; io path utilities (if panic); as-? operator (if panic); string-keyed object literals (if panic); transport::quic NYI (if panic) |
| **3** | v0.3 a-class doc fix | http options-arg required (8 fns); other a-class surfaced during G.2 |
| **4** | v0.4 deferral (already errors cleanly OR pure feature-add) | clone in call-arg (verify clean), cview/cmut C-ABI, E b-5 module schema gap, E b-6 transport::quic NYI, F domain legacy type syntax, A G1-B-FQ7-V2VERIFIER stderr noise, J.5e iterator-protocol |
| **5** | Investigate-then-classify (per-binding routing) | C Match enum-payload inference; E b-2 pure-Shape stdlib inference family (~9-10 files); E b-3 HashMap key-kind discriminator gap + W17.3-4.3 alignment |

## R8 W6 active dispatches (this session)

7 parallel agents:
1. **G1 audit** — JIT TypedArrayPushI64 FrameDescriptor SEGFAULT (read-only, main repo)
2. **G1 audit** — W17 factory-return arms (ConcreteReturn::Discriminant(16)) + transport::tcp divergence (read-only, main repo)
3. **G2 conversion** — Wave 5d ~40+ todo!() panics → structured errors (worktree: `../shape-r8w6-g2-bulk-surface`, branch `r8w6-g2-bulk-surface`)
4. **G5 investigate** — match enum-payload inference loss (read-only, main repo)
5. **G5 investigate** — pure-Shape stdlib inference family ~9-10 files (read-only, main repo)
6. **G5 investigate** — HashMap key-kind discriminator gap (read-only, main repo)
7. **ADR drafts** — 2 §2.7.4 addendum verbatim-text drafts to `docs/cluster-audits/v0.3-adr-2-7-4-addendum-drafts.md` (read-only, main repo)

## Remaining v0.3 scope (post-G.1-Step-1)

- Group 1 fixes (after audits land + dispatch as separate worktree branches)
- Group 2 bulk conversion (in flight)
- Group 3 a-class doc fixes (queued)
- Group 5 routing per investigation outputs (queued)
- J.4-rest residuals (joinStr + small sort-family items)
- G.2 Step 2 (doc-based example writing per slice; gated on Step 1 fully closed)
- 2 ADR §2.7.4 addendum texts (in flight via drafts file mechanism)
- v0.3 close + tag (user-authorization-gated)

## ADR §2.7.4 addendum text-ratify — STRUCTURAL FIX

Relay-chain forwarding failed THREE times. Substance + direction ratified
2026-05-24 (commits `33f165cd` typed-module-exports + `e9f73b57` foreign-ffi);
verbatim ADR-doc-insertion text owed. NEW MECHANISM: agent drafts verbatim
text into `docs/cluster-audits/v0.3-adr-2-7-4-addendum-drafts.md`. Supervisor
reads file directly from disk, bypasses relay. Same Q3 pre-flight +
post-apply-grep discipline as §2.7.13 ratify.

## Trajectory

~**5-8 sessions** to v0.3 tag (re-projected post-R8 W5 supervisor disposition).
Sized: G1 (1-2) + G2 (1) + G3 (0.5) + G5 routing (0.5) + J.4-rest (0.5) +
G.2 Step 2 (1-2) + ADR ratify (0.5) + close + tag (0.5). Re-project after
G1 audits + G2 conversion close.

## Dispatch hygiene (binding — R7 W3 + R8 W4 hardening)

Agent `isolation: "worktree"` is unreliable. Pre-create a sibling worktree
(`git worktree add /home/dev/dev/shape-lang/shape-<branch> -b <branch>
<main-HEAD>`), dispatch WITHOUT `isolation`, pin the agent to that
absolute path as its first `cd`, forbid `cd`-ing to
`/home/dev/dev/shape-lang/shape`. Each fix branch is built + reproducers
re-verified on main post-merge before the next dispatch.

**`git stash` ABSOLUTE BINDING (supervisor 2026-05-23 + 2026-05-24
pre-commit-hook enforcement):**

> Parallel-dispatch agents are FORBIDDEN from `git stash` in any form.
> State-recovery uses targeted `git checkout -- <file>`, `git reset
> HEAD <file>`, or explicit commits in the agent's own pinned worktree.
> The shared `.git/refs/stash` stack is off-limits. Every dispatch
> prompt for a parallel-worktree agent MUST include this verbatim.

Mechanical enforcement: pre-commit hook at `.git/hooks/pre-commit` rejects
commits while `git stash list` is non-empty (shared across all worktrees).
**R8 W5 ZERO violations** — hook is working.

**Audit-day exception:** read-only audits (no source changes) run in the
main repo without worktrees provided each writes only its own audit doc and
stops without committing. Team-lead commits the audit docs together at
audit-round close. R8 W6 has 5 read-only audits + investigations active.

## Canonical (ii) F' smoke harness

Release binary; NOT pipe-to-tail.
```bash
out=$(timeout 30 ./target/release/shape run --mode $m $f 2>&1)
ec=$?; last=$(echo "$out" | tail -1)
```
Fixtures `tests/smokes/s{1..5}.shape`; expected s1 `4950` / s2 `30` / s3 `x`
/ s4 `2` / s5 `x`, all ec=0, VM == JIT.

## Pending the v0.3 tag

1. A–F land + smoke 5/5 preserved at every checkpoint.
2. G.1 Step 1 (a)/(b)/(c) groups close (R8 W6 in flight).
3. G.2 Step 2 closes; any new (b)-class findings folded.
4. ADR §2.7.4 addendum text ratifies.
5. Relay close-evidence to supervisor; supervisor ratifies.
6. **User** authorizes the `v0.3.0` tag.
7. Team-lead lands the tag at the authorized commit.

## Bindings — refuse on sight

Full detail: CLAUDE.md + ADR-006 §2.7.x. Operative:

- **No-known-incorrectness-ships-in-v0.3** (user 2026-05-20): known-incorrect
  (crash / VM-JIT divergence / wrong result / memory-unsafety / silent-wrong-
  output / spurious-reject of valid code) → v0.3-gating; incomplete-but-CLEAN
  → v0.4-OK with surface-and-stop messaging.
- **Regressions are not an option** (user 2026-05-22).
- All CLAUDE.md Forbidden Patterns + Renames-to-refuse-on-sight + ADR-005 §1
  + ADR-006 §2.7.x + 4-table HeapKind lockstep + 5-arm receiver-recovery.
- **Run-verify binding** (supervisor 2026-05-22): every repro in a surfacing
  MUST run-verify at HEAD before relay.
- **Q3 ground-truth before disposition.**
- No Co-Authored-By trailer; own all code quality.
- **Group 2 conversion discipline:** structured errors per ADR-006 §2.7.14
  SURFACE pattern (feature-name + clear "v0.4 / planned" annotation). NOT
  silent no-ops; NOT Bool-default; NOT panic.

## Cadence

Autonomous. Surface only on: (1) defection-attractor framing; (2) ADR
amendment text drafted (use the designated drafts file); (3) novel
architectural gap needing scope decision; (4) user-decision item. Relays
≤ ~80 lines; plain code fences; HEAD-cite + facts + one specific ask.

Multi-session rotation expected. Refresh this doc at every rotation.

## Close gates (every checkpoint + tag commit)

`just check-clean` exit 0 · `verify-merge.sh` 13/13 · `check-no-dynamic.sh`
exit 0 · smoke s1–s5 5/5 VM == JIT (canonical (ii) F' release-binary
harness) · git-stash pre-commit hook ZERO violations preserved · AGENTS.md
row appended; no Co-Authored-By trailer.

---

*R8 W5 CLOSED (G.1 Step 1 6-slice audits + shape-web (a)-class batch).
R8 W6 dispatched 2026-05-24: 2 G1 audits + 1 G2 bulk-conversion +
3 G5 investigations + 1 ADR-drafts agent in parallel.*
