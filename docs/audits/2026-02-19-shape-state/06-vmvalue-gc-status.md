# VMValue and GC Status

## VMValue Retirement Status

## Verdict
`VMValue` is **not** fully removed.

### Inventory (re-run)
Command: `cargo xtask vmvalue inventory`

- references: `1345`
- files: `69`
- per-crate references:
  - `shape-vm`: `647`
  - `shape-value`: `376`
  - `shape-runtime`: `320`
  - `shape-jit`: `2`
- location categories (file count):
  - `hot_path`: `36`
  - `support_or_legacy`: `22`
  - `test_or_bench`: `9`
  - `boundary`: `2`

### Guard behavior
Command: `cargo xtask vmvalue check`
- Guard passes for “no new non-allowlisted VMValue usage”.
- This is a spread-control guard, not a retirement-complete signal.

### Structural evidence
- `shape-value` crate still defines VMValue as foundational type (`shape/shape-value/src/lib.rs:3`, `shape/shape-value/src/value.rs:198`).
- VM/JIT/runtime conversion paths remain active (`shape/shape-vm/src/executor/call_convention.rs:24`, `shape/shape-runtime/src/snapshot.rs:721`, `shape/shape-jit/src/ffi/object/conversion.rs:44`).

## Practical Interpretation
There are two coherent paths:

1. **Boundary-only VMValue strategy**
   - Keep VMValue for interop/debugger/snapshot/wire compatibility.
   - Keep execution core NanBoxed-first.

2. **Full retirement strategy**
   - Remove VMValue from value core and formalize only one runtime representation.
   - This is higher-risk and requires deep serializer/interop redesign.

Current codebase is in-between: migration started, but runtime remains dual-representation.

## GC Status (with GC now implemented)

## What exists
- Dedicated `shape-gc` crate is present.
- VM has feature-gated GC heap field and safepoint polling in dispatch loops.
- Evidence: `shape/shape-vm/src/executor/mod.rs:220`, `shape/shape-vm/src/executor/dispatch.rs:103`.

## Gaps and caveats
1. `init_gc_heap()` exists but has no discovered call sites.
   - Definition: `shape/shape-vm/src/executor/mod.rs:314`
   - Search result: no external invocations.

2. VM memory wrapper still documents and exposes stub/no-op behavior for non-gc paths.
   - Evidence: `shape/shape-vm/src/memory.rs:3`, `shape/shape-vm/src/memory.rs:53`.

3. Pointer fixup logic in `shape-gc` still contains placeholder notes for type-specific object pointer tracing.
   - Evidence: `shape/shape-gc/src/fixup.rs:37`.

## Net assessment
GC is integrated enough to be considered “present,” but full production confidence requires:
- explicit VM boot path initialization for GC heap,
- completed fixup/tracing guarantees,
- clear invariants for NanBoxed heap pointer lifecycle under relocation.

## Recommended next actions
1. Decide and document VMValue end-state (boundary-only vs full retirement).
2. Add CI check that `init_gc_heap()` is invoked in GC-enabled runtime boot paths.
3. Add focused GC correctness tests for relocation/fixup over realistic NanBoxed object graphs.
4. Track and reduce VMValue conversions in hot path categories first.
