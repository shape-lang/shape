# Strict-Typing Agent Team Registry

Live state of parallel agents working on the strict-typing plan
(`~/.claude/plans/stop-native-vs-tagged-tax.md`). One row per worker. Update
this file at every state transition (start of cluster, close of cluster,
blocked). The supervisor reviews this before dispatching new work.

## Why this exists

Two-or-more agents working in parallel on `bulldozer-strictly-typed` can
collide on shared territory (`type_schema/mod.rs`, `json_value.rs`,
marshal-layer files). Cluster-close merges already collided once on
`docs/defections.md`. This registry is the cheapest possible coordination
surface: agents declare intent up front; the supervisor catches overlap
before it's coded.

## Roster

| Branch | Worktree | Role | Active cluster | Files owned (rg pattern) | Status | Last update |
|---|---|---|---|---|---|---|
| `bulldozer-strictly-typed-phase-1a` | `../shape-phase-1a` | Phase 1.A migrator | — | `crates/shape-value/src/{slot,heap_value,heap_variants,heap_header}.rs` | idle (last: phase-1a close `8567f81` merged into main at `94eb34d` — ADR-006 foundation steps 1-6 + 9 landed) | 2026-05-08 |
| `bulldozer-strictly-typed-stage-c-dev2` | `../shape-stage-c-dev2` | Cluster #5/#7 migrator (idle) | — | territory: `crates/shape-runtime/src/{json_value.rs,stdlib/{io,http,toml,yaml,msgpack,csv}.rs,stdlib_io/network_ops.rs}` | idle (last: sub-cluster 1 (network_ops) verified-already-closed at `820980d`; N7 close at `7bab206`) | 2026-05-07 |
| `bulldozer-strictly-typed-phase-2` | `../shape-phase-2` | Phase 2 LSDS migrator | — | `crates/shape-diagnostics/, docs/lsds-migration-plan.md, crates/shape-vm/src/compiler/functions.rs (borrow_error_to_lsds + diagnostic_to_shape_error bridge)` | idle (last: phase-2 first session close, schema + B-series vertical slice + migration plan landed) | 2026-05-08 |

## Status values

| Status | Meaning |
|---|---|
| `idle` | No active cluster; safe to dispatch new work. |
| `auditing` | Read-only Audit 1+2 in progress. No code edits. |
| `awaiting-decision` | Audit complete; supervisor must rule on architectural shape before migration starts. |
| `migrating` | Decision made; mechanical migration in progress. |
| `blocked` | Surface-and-stop fired (see `git stash list` on the worker branch). Needs supervisor ruling. |

## Update protocol

- **At cluster start:** the dispatching prompt (or the agent on first action) appends/edits its row with `active cluster`, `files owned`, status `auditing`. Supervisor confirms no overlap with other rows before greenlighting.
- **At decision-gate:** agent flips status to `awaiting-decision` after writing audit doc; surfaces to supervisor.
- **At cluster close:** agent flips status to `idle`, clears `active cluster`, updates "last update" date, and notes the close commit hash in the same row.
- **On stop-and-surface:** agent flips status to `blocked` and stashes the WIP. Supervisor triages stashes as part of session-start review.

## Forbidden zones (no edits without supervisor sign-off)

- `docs/defections.md` — append-only; new entries go at the end of file (see header comment).
- `docs/check-no-dynamic-baseline.txt` — only edit to **lower** a count after deletion progress. Never raise.
- `CLAUDE.md` "Forbidden Patterns" / "Renames to refuse on sight" — these lists are immutable to agents; only the supervisor adds entries (after a successful defection-attempt review).
- This file — agent rows are agent-editable; the schema/headers are not.

## Phase-gate references

| Phase | Gate command | Current |
|---|---|---|
| 2 | `just verify-phase-2` | shape-runtime --lib at 62 errors (2026-05-08, post phase-1a merge) |
| 5 | `just verify-phase-5` (calls `check-no-dynamic`) | passes at frozen baseline; 0/0 forbidden phrases |

Phases 3 and 4 do not yet have automated gates — see `docs/strictly-typed-baseline.md`.
