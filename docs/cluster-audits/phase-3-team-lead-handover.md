# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-24 at main HEAD `8570e228` (**R8 W6 CLOSED** — G.2
bulk panic→SURFACE + G.1 JIT FrameDescriptor + G.1 W17 IoHandle/DataTable
+ G.3 http options-arg + 5 audit docs + ADR §2.7.4 drafts file). v0.3
code criteria substantially complete (A 5.5/6, B/C/D/E/F COMPLETE, J 3/4);
G.1 Step 1 disposition fully ground-truthed + 4 of 5 supervisor groups
substantially closed.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized.
Supervisor handles architectural calls; user authorizes tags + language
semantics. User relays between team-lead and supervisor.

## Current state

| | |
|---|---|
| Main HEAD | `8570e228` (R8 W6 close: 4 merges + 1 audit-batch commit) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness; re-verified at HEAD `8570e228` post-final-merge) |
| verify-merge / check-no-dynamic / check-clean | 13/13 / exit 0 / exit 0 |
| git-stash pre-commit hook | DEPLOYED (R8 W5+W6 ZERO violations cumulative) |
| Co-Authored-By trailers (cumulative) | 0 |
| Bad-code merges (cumulative) | 0 |
| v0.3.0 tag | NOT landed — gated on remaining R8 W7+ dispositions + user authorization |

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

## R8 W6 outcome (CLOSED 2026-05-24)

4 merges + 5 audit docs + ADR drafts file landed; all close-gates green
at every checkpoint; smoke 5/5 VM == JIT preserved throughout.

| Merge | Commit | Substance |
|---|---|---|
| G.2 bulk panic→SURFACE | (in d8d79daf history) `d61171f4` → merge | 13 dispatch arms in vm_impl/builtins.rs converted Wave-5c/5d/5e |
| G.1 W17 IoHandle/DataTable | `5b134204` → `6bdf09fc` | 2 KindedSlot ctors + 2 project_typed_return arms; eliminates transport::tcp VM/JIT divergence |
| G.1 JIT FrameDescriptor | `1ef4ca9d` → `9639d8c4` | annotation-handler local-storage-hint capture; verifier complaints eliminated |
| G.3 http options-arg | `94dc8fa9` → `d8d79daf` | doc-fix path: 8 fns in http.shape + http.mdx (shape-web) |
| R8 W6 audit-round close | `8570e228` | 5 audit docs + ADR drafts + handover refresh |

## Supervisor surfaces (for relay)

1. **ADR §2.7.4 addendum drafts file** at `docs/cluster-audits/v0.3-adr-2-7-4-addendum-drafts.md` — supervisor reads from disk per the new mechanism (bypasses relay-chain text-loss). 4 `[SUPERVISOR-DECISION: ...]` markers: (a) NEW top-level §2.7.28/.29 vs amendment-subsection under §2.7.4; (b) insertion position (numeric vs file-position dominant); (c) retro-add `// §2.7.28` / `// §2.7.29` marker comments; (d) Q-number allocation (next available Q26/Q27).
2. **Match enum-payload inference (G5)** — recommended v0.4 in audit; surface if supervisor reads doc-contract leg of no-known-incorrectness binding stronger than clean-compile-error leg. See `docs/cluster-audits/v0.3-r8w6-match-enum-payload-inference-audit.md` §2.

## Remaining v0.3 scope (post-R8-W6)

- **G5 pure-Shape stdlib cluster fixes** (~8-10h across 3 clusters per audit
  `v0.3-r8w6-pure-shape-stdlib-inference-audit.md`): module-level-binding
  rejection (2 files), generic-arithmetic inference loss (5 files),
  empty-array element-type loss (2 files); 1 file already passes (remote.shape).
- **G5 HashMap key-kind divergence-elimination** (~1.5 sessions per audit
  `v0.3-r8w6-hashmap-key-kind-audit.md` §5): JIT V2-verifier-failure path
  fatal-or-interpreter-fallback. Full int-key HashMap support → v0.4 epic.
- **G3 a-class follow-ups** surfaced during R8 W6 (none net-new this round
  beyond the http options-arg fix).
- **J.4-rest residuals** (joinStr + small sort-family items).
- **G.2 Step 2** doc-truth round (USER-followable validation; gated on
  Step 1 finalized + main stable; nominally ready now that R8 W6 closes).
- **ADR §2.7.4 text-ratify** (gated on supervisor reading the drafts file).
- **v0.3.0 tag** (user-authorization-gated; gated on all above).

## ADR §2.7.4 addendum text-ratify — STRUCTURAL FIX

Relay-chain forwarding failed THREE times. Substance + direction ratified
2026-05-24 (commits `33f165cd` typed-module-exports + `e9f73b57` foreign-ffi);
verbatim ADR-doc-insertion text owed. NEW MECHANISM: agent drafts verbatim
text into `docs/cluster-audits/v0.3-adr-2-7-4-addendum-drafts.md`. Supervisor
reads file directly from disk, bypasses relay. Same Q3 pre-flight +
post-apply-grep discipline as §2.7.13 ratify.

## Trajectory

~**3-5 sessions** to v0.3 tag (re-projected post-R8-W6 close). Sized:
G5 pure-Shape stdlib (1-2) + G5 HashMap divergence (0.5-1) + J.4-rest (0.5)
+ G.2 Step 2 (1) + ADR text-ratify (0.5) + close + tag (0.5). R8 W6
substantially compressed the trajectory by closing 4 dispositions in one
round.

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

*R8 W6 CLOSED 2026-05-24: 4 merges landed (G.2 bulk surface, G.1 JIT
framedesc, G.1 W17 IoHandle, G.3 http options-arg); 5 audit docs + ADR
§2.7.4 drafts file committed. Smoke 5/5 + 13/13 + git-stash hook ZERO
violations preserved throughout. Next round: G5 pure-Shape stdlib + G5
HashMap divergence-elimination.*
