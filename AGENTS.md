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
| `bulldozer-strictly-typed-phase-1b` | `../shape-phase-1b` | Phase 1.B migrator | — | territory: `crates/shape-runtime/src/` ValueWord callers — see `docs/cluster-audits/phase-1b-valueword-callers.md` for recipe | idle (last: phase-1b cluster-close `f218a5d` — `shape-runtime --lib` 57→0 errors; 1011 lib tests pass; cross-crate residuals shape-vm=2001 / shape-jit=2001 expected per §2.7.5) | 2026-05-08 |
| `bulldozer-strictly-typed-phase-1b-vm` | `../shape-phase-1b-vm` | Phase 1.B-vm consumer migration | Wave 6.5 substep-1 done; substep-2 surfaced as ARCHITECTURAL (cascade-balloon + scope-vs-gate conflict) | territory: `crates/shape-vm/src/executor/{vm_impl/stack,objects/raw_helpers,arithmetic/mod,comparison/mod,loops/mod,call_convention,control_flow/mod}.rs` + every caller of the deleted shims | **blocked** (2026-05-09): substep-1 (delete shims + opcode_defs comment cleanup) committed at WIP `<see report>`; substep-2 surface — actual cascade is 1294 caller sites for the 5 mandatory-gate shims across 43 files (audit estimate +700 across 7 files; +84% off and 6× wider file footprint). Wave 6.5 territory (7 files) holds only 232 of those 1294 hits; the remaining 1062 hits live in 36 OOR files explicitly listed as Wave 7/8/9 territory ("surface if Wave 6.5 absolutely needs to touch them"). Substep-4 grep-fail gate ("zero hits anywhere in source") cannot be satisfied without violating the no-touch boundary on Wave 7/8/9 files. Additionally, every caller migration in arithmetic/comparison/call_convention pulls in parallel `ValueWord`-construction rewrites (~160 deleted-`ValueWord` references in territory files: e.g. `ValueWord::from_decimal(d).into_raw_bits()` → `Arc::into_raw(Arc::new(d)) as u64` plus kind sourcing) — push-site kind IS sourcable locally, but the construction expressions are not mechanical-substitution shapes. shape-vm error count post-shim-deletion: 1684 → 2591 (+907 cascade as expected). Per-territory file: arithmetic 34→147, comparison 47→101, loops 43→58, control_flow 29→57, call_convention ~7→30, raw_helpers 57→57 (already broken pre-Wave-6.5), opcode_defs 0→0 (comments-only, fully migrated). | 2026-05-09 |
| `bulldozer-strictly-typed-stage-c-dev2` | `../shape-stage-c-dev2` | Cluster #5/#7 migrator (idle) | — | territory: `crates/shape-runtime/src/{json_value.rs,stdlib/{io,http,toml,yaml,msgpack,csv}.rs,stdlib_io/network_ops.rs}` | idle (last: sub-cluster 1 (network_ops) verified-already-closed at `820980d`; N7 close at `7bab206`) | 2026-05-07 |
| `bulldozer-strictly-typed-phase-2` | `../shape-phase-2` | Phase 2 LSDS migrator | — | `crates/shape-diagnostics/, docs/lsds-migration-plan.md, crates/shape-vm/src/compiler/functions.rs (borrow_error_to_lsds + diagnostic_to_shape_error bridge)` | idle (last: phase-2 first session close, schema + B-series vertical slice + migration plan landed) | 2026-05-08 |
| `bulldozer-strictly-typed-phase-1b-vm-cluster-B` | `../shape-phase-1b-vm-cluster-B` | Phase 1.B-vm Wave 6.5 substep-2 cluster B migrator | Wave 6.5 substep-2 cluster B — control path & locals | `crates/shape-vm/src/executor/{variables,control_flow,loops}/mod.rs + executor/{call_convention,debugger_integration,osr,dispatch,resume}.rs` | **blocked** (2026-05-09): cluster B territory is architecturally pre-broken on FORBIDDEN patterns predating substep-1 (tag_bits::*, is_tagged(), as_heap_ref(), NativeKind::Unknown, NativeKind::Dynamic, ValueWord, ValueWordExt, vw_clone, vw_drop, synthesize_value_word_from_raw, nb_to_external, marshal_arg_to_jit(&ValueWord, _) ABI). Mandatory shim grep yields 278 hits and sibling shim grep 102 hits across the 8 files, but nearly every call site sits inside an expression whose surrounding code uses one of the listed forbidden shapes. Mechanical shim-name migration alone produces zero compilable lines: e.g. `pop_raw_u64()? -> u64` callers chain `.is_truthy()` / `.as_heap_ref()` / `.raw_bits()` (`ValueWordExt` methods on u64 — trait deleted), `stack_take_raw(self.sp)` is wrapped to return `ValueWord` (deleted), `stack_slice_raw(...)` feeds `nb_to_external` (deleted), `marshal_arg_to_jit(&vw_slice[0], kind)` takes `&ValueWord` (deleted) and returns `ValueWord` (deleted). The genuine migration is a deep architectural rewrite, not a shim-name rename — and it cascades into out-of-territory files (cluster D's `raw_helpers.rs` for the `as_heap_ref` replacement, control_flow/jit_abi.rs for the marshal_arg_to_jit signature, shape-value for nb_to_external/ValueWord/ValueWordExt). Per playbook §8 surface-and-stop triggers: (1) "Cross-cluster migration cascade", (2) "shape-jit FFI consumer trips" (jit_abi.rs is technically `control_flow/jit_abi.rs` — out of cluster B's listed file scope but cluster B's mod.rs depends on it), (3) the opcode itself (IterNext, OSR materialize) is wrong-shape: kind sourcing per playbook §2 requires replacing the as_heap_ref-style dispatch with HeapValue:: match, but doing so deletes ~60% of loops/mod.rs and call_convention.rs body code. AGENTS.md row 23's blocked note from supervisor confirms this was an anticipated cascade. shape-vm error count: parent baseline 2591; no edits applied. WIP: clean (no stash). | 2026-05-09 |

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
| 2 | `just verify-phase-2` | shape-runtime --lib at 0 errors (2026-05-08, post phase-1b merge `c5f6672`); shape-vm/jit at ~2000 each — next session's scope per §2.7.5 |
| 5 | `just verify-phase-5` (calls `check-no-dynamic`) | passes at frozen baseline; 0/0 forbidden phrases |

Phases 3 and 4 do not yet have automated gates — see `docs/strictly-typed-baseline.md`.
