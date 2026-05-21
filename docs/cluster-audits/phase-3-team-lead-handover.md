# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-21 at main HEAD `3f227c63`. Round 6 (post the 5-family
no-known-incorrectness program). Git holds prior content; no archaeology here.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized. A
supervisor handles architectural calls; the user (strategic owner) authorizes
tags and language semantics. The user relays between team-lead and supervisor.

## Current state

| | |
|---|---|
| Main HEAD | `3f227c63` |
| Smoke matrix s1–s5 | 5/5 VM == JIT (canonical (ii) F' release-binary harness) |
| verify-merge / check-no-dynamic | 13/13 / exit 0 |
| v0.3 tag | NOT landed; gated on WS-1..4 fix close |

## Canonical (ii) F' smoke harness

Release binary; NOT pipe-to-tail.
```bash
out=$(timeout 30 ./target/release/shape run --mode $m $f 2>&1)
ec=$?; last=$(echo "$out" | tail -1)
```
Fixtures `tests/smokes/s{1..5}.shape`; expected s1 `4950` / s2 `30` / s3 `x`
/ s4 `2` / s5 `x`, all ec=0, VM == JIT.

## Round-6 workstreams

The four audits are CLOSED (docs in `docs/cluster-audits/`). Fix wave follows.

**WS-1 — V3-S5 ckpt-5/6 `op_new_array` construction.** Audit:
`v0.3-w16-2-c-empty-literal-audit.md`. W16.2-A is MERGED (`2924b685`); smoke
s5 passes. Remaining = **W16.2-C** (empty-literal / spread / list-comprehension
— 10 `Count(0)` `NewArray` sites SURFACE with a jargon dump; comprehension
JIT path diverges from VM → v0.3-gating). W16.2-B is NOT redundant (scalar-
element `dyn Trait` arrays still SURFACE) but its round-4 branch is not
mergeable — re-implement fresh. 6.B.1 codegen choice is supervisor-ratify;
WS-1 fix is HELD pending that ratify.

**WS-2 — ζ-round.** (a) unannotated nested fn calls in a loop → VM silent-
wrong `1.0`, JIT correct (loop-triggered, unannotated-only). (b) `id(None)`
hangs (ec=124). Investigate-first. Fix agent re-dispatched.

**WS-3 — Result/AnyError machinery.** Audit:
`v0.3-ws3-result-anyerror-audit.md` — 7 v0.3-gating sub-items, all localized,
no opcode/ABI/schema change: F1 (JIT typed-array OOB → 0; `v2_array.rs:838`),
F2a (`?` type-loss; `advanced.rs:32`), F2b (`?` runtime SIGSEGV;
`exceptions/mod.rs:662`), F3 (`!!` no dispatch arm; `binary_ops.rs:853`),
F4 (jargon-dump is `handle_exception` no-handler branch `exceptions/mod.rs:243`
— `build_any_error` is NOT stubbed), array-rest clean-error, `type_check_kinded`
`"array"` arm.

**WS-4 — Object destructuring.** Audit:
`v0.3-ws4-object-destructuring-audit.md` — 4a (`type_check_kinded` missing
`"object"` arm; fixing it resolves 4a without an F4 rebuild), 4b (destructure
binding-type loss; `destructure.rs` Object arms + `inference/items.rs:1176`),
4c (`match` struct-pattern misclassified — parse ambiguity `shape.pest` +
classification `checking.rs`/`binding.rs`). One moderate sub-cluster, ~5 files.

**WS-5 — ADR-006 §2.7.13 amendment ratify.** Surfacing. β1 carrier migration
merged (`a287c795`); §2.7.13 text stale (`TypedField.receiver` → `TypedObjectPtr`;
`TypedIndex` variant deleted).

WS-3 + WS-4 share `exceptions/mod.rs` (`type_check_kinded` array/object arms)
+ `destructure.rs` — resolve the 2-file overlap at integration (take-both,
distinct arms/functions).

## Dispatch hygiene (Round-6 lesson — binding)

Agent `isolation: "worktree"` is unreliable here: some worktrees were created
on a stale base (`ec27b1ef`), and agents `cd`'d into the main repo because the
prompt named its path. For every dispatch: **pre-create a sibling worktree**
(`git worktree add ../shape-r6-<ws> -b <branch> 3f227c63`), pin the agent to
that absolute path, and explicitly forbid `cd`-ing to `/home/dev/dev/shape-lang/shape`
(the main repo). Serialize any `git stash` step across parallel worktrees.

## Bindings — refuse on sight

Full detail: CLAUDE.md + ADR-006 §2.7.x. Operative:
- **No-known-incorrectness-ships-in-v0.3** (user 2026-05-20): known-incorrect
  (crash / VM-JIT divergence / wrong result / memory-unsafety / silent-wrong-
  output) → v0.3-gating. Incomplete-but-CLEAN → v0.4-OK. A jargon-dump stub is
  NOT "clean".
- **Q3 recursive pre-flight:** ground-truth before disposition.
- Hypotheses are INVESTIGATE not FIX until empirical bisect.
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
AGENTS.md row appended; no Co-Authored-By trailer.

## v0.3 tag

After WS-1..4 fix close + no-known-incorrectness set empty + smoke 5/5 → relay
close evidence to supervisor → supervisor ratifies → USER authorizes the
`v0.3.0` tag. Land the tag only on explicit user authorization.

---

*Live operational state. Next rotation: verify HEAD + smoke 5/5 + verify-merge,
read the four `cluster-audits/` round-6 docs, continue the fix wave.*
