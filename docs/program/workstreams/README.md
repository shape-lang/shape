# Proposed workstreams — ERGO / PERF / POLY (2026-07-27, PENDING RATIFICATION)

Status: drafted at user direction 2026-07-27, grounded in a six-lane code
scout at main `7e343c20`; **open rulings answered by the user 2026-07-27**
(see [ratification-grill.md](./ratification-grill.md): Q1 B, Q2 A, Q3 C,
Q4 as recommended, Q5 B). **Not yet published**; enactment awaits explicit
go-ahead. The ADR-011–016 tracker publication (89 entries, 285 edges,
frontier #111) is frozen; the only approved changes to it are the Q2 delta
below.

First-wave lane assignment (Q1 ruling): the spine claims #111 → #91;
lane A: PERF-SUITE; lane B: ERGO-FIX-CHANNEL + ERGO-VAR-TRUTH; lane C:
POLY-STUB-CHANNEL + POLY-ASYNC-TRUTH then POLY-ASYNC-OFFLOAD (Q3 = C;
scoped days-scale by the 2026-07-27 feasibility scout). Lanes may
start in pinned worktrees immediately; no lane merges to main before
#111's authority commit lands.

Authority chain:

- Semantics: ADR-014 §8, ADR-015 §10, ADR-016 §10, ADR-017, ADR-018, ADR-019
  (all marked proposed/pending ratification).
- Execution rulings: R21–R25 addendum in
  `docs/design/typed-comptime/adr011-012-execution-rulings.md`.
- This directory: ticket decomposition, blockers, and acceptance tripwires.

Publication path after user ratification: these tickets enter GitHub through
the same atomic expansion protocol the program manifest requires for
inventory waves — exact rows committed first, then published, then audited by
re-fetch. Blocker references to published issues use their real numbers;
references to tickets inside this proposal use symbolic IDs.

**Ratification-time delta to published state (disclosed, not "purely
additive"):** ratifying this bundle approves (a) two new native blocker
edges into published issues — ERGO-QUASIQUOTE → #110 (the string-arm
deletion path) and the R21 row-in-type slice → EFFECT-CONTRACT #143
(precedence) — applied at publication through the issue-amendment process
with re-fetch audit; and (b) four scope-by-reference expansions: #143,
#112, #113, and #163 carry acceptance criteria of the form "conforms to
ADR-014 / ADR-016", and those ADRs gain §8 / §10 in this bundle. No other
published body or edge changes; where work lands inside an existing
ticket's scope (e.g. POLY-ASYNC-TRUTH within #163/#164), that is stated
explicitly instead of editing the ticket.

Per-slice evidence: every ticket follows the program-wide slice rule
(semantic fact + compiler/LSP projection + VM/JIT behavior + negative
diagnostics + docs/manifest updates + shrink-only deletion evidence), plus
the workstream-specific gates named in each file.

| Workstream | File | Tickets | Theme |
|---|---|---|---|
| ERGO | [ergo.md](./ergo.md) | 7 | sugar parity, script tier, fix channel, quasiquote |
| PERF | [perf.md](./perf.md) | 10 | charter, deopt granularity, RC elision, BCE, arenas |
| POLY | [poly-depth.md](./poly-depth.md) | 8 | foreign checking, env pinning, zero-copy, foreign refs, real foreign async |
