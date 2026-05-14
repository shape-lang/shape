# Bulldozer Multi-Session Chain Pattern

## Authority

- Strategic-owner authorization 2026-05-14 (D3 disposition D-α — Round 3a close)
- Supervisor disposition Round 3a' close 2026-05-14 (this doc landing pre-D4-dispatch)
- Discipline pattern documentation for future wave dispatch reuse

## When to use

Reserved for atomic-lockstep cascades that exceed single-LLM-session execution
capacity (ceiling-c per Round 3a D3 finding: ~50-100 non-mechanical edits per
session at discipline-coherent quality bar). Indicated when:

- The cascade cannot be split atomically due to type-system propagation
  (e.g. variant signature changes propagate to all consumer destructure patterns —
  partial-flip leaves either producer or consumer in incompatible state)
- Cannot accept type-confusion-window between sub-commits (mid-flip state =
  heap corruption / SIGSEGV / UB)
- The ~100-site cascade ceiling waiver alone is insufficient because the bound
  is execution capacity, not site count

Specifically NOT for:

- Non-atomic cascades that could be split into independent atomic sub-commits
  (the per-handler-family split pattern A2-followup used is the right shape
  for non-atomic cascades; multi-session chain is for genuinely atomic-lockstep)
- Bypassing the ~100-site cascade ceiling for non-atomic cascades (waiver
  authorization applies separately for those)
- Sub-cluster work that fits within single-session execution capacity even
  with cascade-ceiling waiver

## Operational shape

1. Team-lead creates feature branch:
   `bulldozer-strictly-typed-<wave>-<sub-cluster>-checkpoint-<N>`

2. Dispatch sub-agent 1: starts from canonical HEAD; lands ~50-100 edits;
   commits to feature branch at `checkpoint-1`; cargo check **MAY BE BROKEN**
   on feature branch; surface-and-stops with structured state pointer.

3. Dispatch sub-agent 2: starts from `checkpoint-1` (NOT canonical HEAD); lands
   next ~50-100 edits; commits at `checkpoint-2`; cargo check MAY BE BROKEN;
   surface-and-stops.

4. ... N sub-agents in sequence (dynamic chain length per actual cascade shape
   vs ceiling-c bound — agents surface-and-stop when local scope exceeds
   capacity; team-lead dispatches next agent continuing from current state).

5. Final sub-agent: starts from `checkpoint-(N-1)`; lands the FINAL atomic
   integration commit; **STRICT close gate**:
   - `cargo check --workspace --lib --tests` EXIT=0
   - `bash scripts/verify-merge.sh` EXIT=0; 12/12 PASS
   - `bash scripts/check-no-dynamic.sh` EXIT=0
   - Smoke matrix preserved (or improved per the migration's intent)
   - AGENTS.md row state-flipped active → closed
   - NO `Co-Authored-By: Claude` trailer

6. Team-lead merges feature branch `checkpoint-final` into
   `bulldozer-strictly-typed` canonical with standard take-both ceremony.

**Structured state pointer (for intermediate sub-agent surface-and-stop, step 2/3/4):**

Each intermediate sub-agent's close report MUST include:

- **What's been done**: enumerated by file:line + edit category (variant signature
  change / destructure adaptation / dispatch arm flip / etc.)
- **What remains**: enumerated grep-verified list with file:line cites at current
  feature-branch HEAD
- **Cargo check status**: errors enumerated (file:line + error[E####] message)
  if broken
- **verify-merge.sh status**: which checks pass, which fail (12 total)
- **Cascade-ceiling exceeded reason**: ceiling-a (~100 sites), ceiling-b
  (~48 method-handler entry points), or ceiling-c (~50-100 non-mechanical
  edits per session)

## Discipline preserved

- Each sub-agent has bounded scope (single-session capacity per agent's own
  capability estimate)
- Final sub-agent has STRICT close gate (no relaxation)
- `bulldozer-strictly-typed` canonical never receives broken intermediate state
- Feature branch is throwaway; canonical preserves invariants
- Refusal #10 ("defer to cluster-1+") DOES NOT apply — work stays in current wave
- All Forbidden Patterns + Renames to refuse on sight + Parallel-implementation
  framings still apply to every sub-agent
- ADR-005 single-discriminator + ADR-006 §2.7.x rulings preserved
- Pre-flight ground-truth check binding (Round 1 disposition 6 + Round 2
  disposition 3) extends to multi-session chain: each sub-agent grep-verifies
  its scope at feature-branch HEAD before edits

## Discipline relaxed (bounded)

- **Intermediate sub-agent close gate**: cargo check broken state OK on feature
  branch (NOT on canonical)
- Sub-agents may commit incomplete refactors to feature branch as long as
  structured state pointer surfaces what's been done + what remains
- Multi-session execution authorized only for genuinely atomic-lockstep
  cascades that exceed ceiling-c (NOT a default cadence)

## Forbidden under this pattern

- Merging feature branch with broken cargo check into canonical (must reach
  STRICT close gate on final sub-agent first)
- Skipping the final atomic flip (the whole point is one atomic transition;
  partial-flip-permanent forbidden)
- Using this pattern for non-atomic cascades that could be split into
  independent atomic sub-commits (use per-sub-family atomic sub-commit split
  instead — A2-followup-mechanical Round 3a' precedent)
- Using this pattern to bypass the ~100-site cascade ceiling for non-atomic
  cascades (cascade-ceiling waiver applies separately)
- bridge/probe/helper/hop/translator/adapter/shim framings for the intermediate
  state (per CLAUDE.md broader-family regex — describe intermediate state by
  what's been done + what remains, not by hypothetical role)
- "Preserve broken cargo check across wave boundary" / "extend feature branch
  beyond current wave" / any deferral framing within or past wave (refusal #10)

## Recovery from sub-agent failure

- **S1-R18 DURABLE PATTERN applies**: team-lead-completes-ceremony for
  verified-correct WIP at sub-agent layer per user 2026-05-14 ratification
  (4-criterion test: cargo check workspace EXIT=0 in worktree pre-commit OR
  documented broken-state-OK-on-feature-branch + verify-merge.sh 12/12 OR
  documented broken-state + diff-scoped-to-territory + commit attribution
  to sub-agent + ceremony to team-lead)
- **Sub-agent surface-and-stop in chain**: team-lead dispatches next sub-agent
  with current state pointer (no relay round needed for routine chain
  progression)
- **Sub-agent produces wrong intermediate state**: team-lead can rewind
  feature branch to last good checkpoint (git reset --hard <ckpt-K>) and
  re-dispatch the failed sub-agent with refined scope or extended dispatch
  prompt; team-lead surface to supervisor only if rewind is non-mechanical

## Cumulative imprecision-pattern instances tracking

Through Round 3a' close: 18 instances cumulative (see
`docs/cluster-audits/phase-3-cluster-0-status.md` for canonical list). The
pre-flight ground-truth check binding catches these BEFORE bad code lands;
the trend through Round 3a' is decreasing severity (most instances now caught
at first sub-agent's pre-flight check).

**Velocity expectation (informational):**

- Single-session execution capacity per sub-agent: ~50-100 non-mechanical edits
- N sub-agents = (total cascade size) / (per-session capacity)
- Plus final atomic-integration commit (separate session for STRICT close gate)
- Plus team-lead merge ceremony (small, ~0.25 session)

For D3's ~270-320 site cascade: N=4-5 sub-agents expected per team-lead initial
planning; dynamic chain authorization (supervisor 2026-05-14 disposition)
allows team-lead to extend N if a sub-agent surface-and-stops mid-scope.

---

*This pattern is a discipline extension for genuinely atomic-lockstep cascades
that exceed single-LLM-session execution capacity. It is NOT a default cadence.
The bulldozer cadence prefers single-session atomic landings where structurally
possible; this pattern unblocks the rare cases where the atomic unit exceeds
agent capacity.*
