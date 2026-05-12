# Phase 2d — Hardening follow-up stack

**Started:** 2026-05-11 (during Wave 2.5 close).
**Purpose:** Track non-blocking follow-up items surfaced by Wave 1 / Wave 2 / Wave 2.5 agents. None of these block Phase 2d close on their own; they get resolved opportunistically or as their own sub-clusters when the cost/benefit aligns.

Format: `(<letter>) <one-line title>` — body explains finding + recommended resolution.

---

## (a) `verify-merge.sh` CHECK 8 — `HeapKind::X => {}` shorthand style rule

**Status (post-Wave 2.5):** RESOLVED at the call-site level; reframed.

**The finding (reframed 2026-05-11):** CHECK 8 is NOT a flake or false positive. It correctly catches the `HeapKind::X => {}` inline-empty-arm shorthand inside dispatch tables — that shorthand makes the awk state machine's `}`-closing detector unable to pair arms correctly when an exhaustive-match comment-and-arm-on-same-line pattern shows up later. The original framing as a "false positive" was a misread by early Wave 1 / Wave 2 agents (and reinforced by a supervisor pipe-tail measurement bug; see CHECK-COMMS-1).

**Resolution that landed:** W17-array-typed-receiver (Wave 2 close commit `6eee767`) refactored 11 inline-empty arms across `stack.rs` / `kinded_slot.rs` / `closure_layout.rs` / `heap_value.rs` to multi-line form. CHECK 8 now reaches `=== SUMMARY ===` cleanly at HEAD.

**Going-forward style rule:** Never write `HeapKind::X => {}` inline. If an arm has an empty body, write it as

```rust
HeapKind::X => {
    // intentionally empty: <reason>
}
```

Future agents that add new HeapKind variants (e.g. W17-trait-object-storage's `TraitObject=29`, W17-concurrency's `Mutex/Atomic/Lazy` already in) MUST keep their new arms multi-line. CHECK 8 will catch backsliders.

---

## (b) `bump_closure_share` retrofit + principled relocation

**Status:** retrofit needed; principled fix queued.

**The finding:** W17-array-closure-callback (Wave 2 close `06d0d66`) added a caller-side compensation helper `bump_closure_share` at `crates/shape-vm/src/executor/objects/array_transform.rs:104`, called from per-iteration closure invocation sites in `array_sort.rs:283`, `array_query.rs:237,333,599`, `array_sets.rs:618`, `array_joins.rs` and others. Each call pays one `Arc::increment_strong_count<HeapValue>` to compensate for `op_return`'s `drop_with_kind(closure_heap_bits, kind)` consuming the dispatch-shell-owned closure share on the FIRST iteration (subsequent iterations would be dangling without compensation).

The Wave 2.5 dispatch instructions established the breadcrumb format: `// phase-2d-hardening:(f) — <description>` at any `bump_closure_share`-style closure-share workaround site. Since W17-array-closure-callback landed before this format existed, the existing call sites have no breadcrumbs yet.

**Resolutions (pick one when time allows):**

1. **Retrofit breadcrumbs.** Add `// phase-2d-hardening:(b) — bump_closure_share compensates frame-teardown decrement; principled fix is to move share-bump into call_value_immediate_nb itself.` at every call site + the function definition. Mechanical; ~7 sites.
2. **Principled fix.** Move the share-bump INTO `call_value_immediate_nb` (the value-call entry point in `executor/call_convention.rs`). All callers stop needing the compensation. Touches a frame-teardown invariant in §2.7.11/Q12 — needs ADR amendment proposal.

Either way, the (f) breadcrumb naming convention stays: `phase-2d-hardening:(b)` was renumbered to `:(f)` in the Wave 2.5 dispatch text (no usages exist yet — the convention is forward-looking for any future closure-share workaround).

---

## (c) Rust 2024 edition `unsafe_op_in_unsafe_fn` warnings

**Status:** warnings only (cargo check clean); not blocking.

**The finding:** W17-references-mutation (Wave 2 close `30b9ebf`) added two `unsafe fn` helpers in `crates/shape-value/src/heap_value.rs`:
- `TypedObjectStorage::write_slot_in_place` (~line 1681+)
- `TypedArrayData::write_index_in_place` (~line 2180+)

The function bodies use raw-pointer arithmetic + dereferences. In Rust 2024 edition, unsafe ops inside an `unsafe fn` body still require an explicit `unsafe { ... }` block. The current bodies don't wrap them, producing forward-compat warnings.

**Resolution:** wrap the raw-pointer ops in `unsafe { ... }` blocks inside the function bodies. Mechanical; ~16 sites between the two functions. Doesn't change behavior, doesn't change the function signature.

---

## (d) Pre-existing `test_object_operations` SIGSEGV

**Status:** pre-existing baseline issue; out of all Wave territories.

**The finding:** `cargo test -p shape-vm --lib` SIGSEGVs in `test_object_operations`. Confirmed reproducible at base `e7b3eea` and earlier by multiple Wave 2 / Wave 2.5 agents (independent corroboration: W17-array-typed-receiver, C3-expr-lowering-misc). The territory in-scope tests pass cleanly when run in isolation; the test runner crash blocks the full-suite verification path.

**Resolution:** dedicated bisect + fix sub-cluster. Likely connected to the broader v2-raw-heap-audit follow-up (CLAUDE.md). Not bundled with any current Phase 2d sub-cluster.

---

## (e) `SerializableVMValue` wire-format gap

**Status:** owned by W17-snapshot-roundtrip (Wave 2.6+).

**The finding:** W17-snapshot-surfaces (Wave 2.5 close `0db5920`) identified that `SerializableVMValue` in `shape-runtime/src/snapshot.rs` has wire-format arms for pre-bulldozer carriers but no arms for the post-W14/W15/W16 HeapKinds: `HashSet`, `Iterator`, `Result`, `Option`, `Deque`, `Channel`, `PriorityQueue`, `Reference`, `FilterExpr`, `SharedCell`. The W17-concurrency close (Wave 2.5 `6ba7e59`) adds another 3 needing arms: `Mutex`, `Atomic`, `Lazy`.

**Resolution:** W17-snapshot-roundtrip will land its own ADR-006 §2.7.5.1 amendment that extends `SerializableVMValue` for all currently-defined HeapKinds, alongside the kind-threaded `slot_to_serializable` / `serializable_to_slot` API. This is one of three blockers for W17-snapshot-roundtrip (the others being `invoke_module_fn_id_stub` and API alignment).

---

## (f) `FieldType::Any` rejection in comptime path

**Status:** queued; Phase-2c follow-up.

**The finding:** C2-comptime-rebuild (Wave 2.5 close `a5df165`) explicitly rejects `FieldType::Any` in `comptime.rs::field_kind_for_readback` rather than guessing slot kinds. This means TypedObject readback of comptime-produced objects through `nb_to_expr` surfaces a structured error when a predeclared schema uses Any. The comptime predeclared schemas use Any.

**Resolution:** narrow the predeclared schemas to specific kinds (so comptime-produced TypedObjects round-trip cleanly) OR add a `Any → Dynamic` readback path that's bounded by `KindedSlot.kind` rather than the schema. Either is its own sub-cluster.

---

## (g) Pre-existing test failures: `channel_ops` × 14

**Status:** pre-existing baseline issue.

**The finding:** W17-concurrency (Wave 2.5 close `6ba7e59`) noted 14 pre-existing `channel_ops` test failures at baseline `f36bf1e`, unrelated to its concurrency territory. The HeapKind::Channel arm work landed in Wave 15 (`c9c8807`).

**Resolution:** dedicated channel_ops test audit. Out of all Wave 2.5 territories.

---

## (h) `let-in-fn` triggers `DropCall` SURFACE — D-trait-obj territory

**Status:** owned by future D-trait-obj sub-cluster.

**The finding:** C1-temporal-lowering (Wave 2 close `1d751e6`) discovered that any `let x = expr` inside a function body hits `DropCall` dispatch at function scope exit, which surfaces a `NotImplemented` at `trait_object_ops.rs:123`. The `Drop` trait dispatch for locals going out of scope is owned by D-trait-obj (a future sub-cluster aliased to W17-trait-object-emission per the Wave 2.5 rescope). Multiple Wave 2 / Wave 2.5 agents (C1, C2, others) re-shaped their smokes from `let x = …` in fn-body to `return …` to avoid this.

**Resolution:** W17-trait-object-emission's Erase_T + DynMethodCall lowering work will satisfy this dispatch path. Until then, agents continue using the workaround.

---

## Observation: pipe-tail vs file-redirect for verify-merge.sh exit capture (CHECK-COMMS-1)

**Status:** documented; process change applied to Wave 2.5+ dispatch prompts.

**The finding:** Several Wave 1 / Wave 2 close-reports in `AGENTS.md` describe `verify-merge.sh` as exiting 0 when the script actually exited 1. Root cause was a measurement bug in supervisor verification commands:

```bash
bash scripts/verify-merge.sh 2>&1 | tail -10; echo EXIT=$?
```

Without `set -o pipefail`, `$?` after the pipeline reflects the EXIT of `tail`, NOT the script. `tail` always exits 0. So `EXIT=0` was reported even when CHECK 8 fired and the script's actual exit was 1. The hits printed by CHECK 8 were truncated by `tail -10` before reaching the SUMMARY line, hiding the fail.

**Process fix (applied Wave 2.5+):** every dispatch prompt now instructs agents to capture verify-merge.sh exit via file redirect:

```bash
bash scripts/verify-merge.sh > /tmp/vm.out 2>&1; echo SCRIPT_EXIT=$?
```

This captures the script's actual exit. The tail output for inspection is read from `/tmp/vm.out` afterward.

This isn't a code follow-up — it's a process / supervisor-discipline note. Recording here so future supervisors don't repeat the misread.

---

---

## (i) JIT FFI `HK_*` legacy-ordinal collisions with current HeapKind

**Status:** latent; not currently reachable due to upstream JIT-execute SURFACE.

**The finding:** Phase 2d Item 3 JIT verification (Wave-4, 2026-05-12) audited
the `HK_*` u16-prefix table in `crates/shape-jit/src/ffi/value_ffi.rs:153-201`
against the current `HeapKind` enum at `crates/shape-value/src/heap_variants.rs`.
The table uses "legacy ordinals" for kinds that were renumbered between
pre-strict-typing and current HEAD; per the file's own doc-comment those
ordinals are deliberately frozen for "ABI stability of the JIT-emitted
constants" because they tag `JitAlloc<T>` / `UnifiedValue<T>` prefixes, not
runtime `HeapKind` discriminator labels. But several legacy ordinals now
collide with newly-assigned `HeapKind` ordinals:

| Legacy `HK_*` | Value | Current HeapKind at same ord | Notes |
|---|---|---|---|
| `HK_ARRAY` | 1 | TypedObject | JIT-internal "Array" allocations |
| `HK_RANGE` | 12 | Instant | post-W15-range produces this via `jit_box(HK_RANGE, *range)` in `context.rs:427` |
| `HK_SOME` | 14 | NativeScalar | post-W14 produces this via `box_some(inner_bits)` |
| `HK_OK` | 15 | NativeView | post-W14 produces this via `box_ok(inner_bits)` |
| `HK_ERR` | 16 | Char | post-W14 produces this via `box_err(inner_bits)` |
| `HK_TRAIT_OBJECT` | 19 | Reference | reserved; no current JIT producer |
| `HK_HOST_CLOSURE` | 6 | DataTable | legacy, no current producer |
| `HK_TYPED_TABLE` | 8 | TypedArray | legacy |
| `HK_ROW_VIEW` | 9 | Temporal | legacy |
| `HK_COLUMN_REF` | 10 | TableView | post-strict-typing `box_column_ref` uses this |
| `HK_INDEXED_TABLE` | 11 | Content | legacy |
| `HK_ENUM` | 13 | IoHandle | legacy |
| `HK_FUTURE_..` | 17,18 | HashMap,FilterExpr | legacy fanout |
| `HK_EXPR_PROXY` | 20 | SharedCell | legacy |
| `HK_TIME` | 22 | Iterator | post-strict-typing `unified_box(HK_TIME, ...)` |
| `HK_DURATION` | 23 | Deque | post-strict-typing `jit_box(HK_DURATION, ...)` |

The collisions don't currently cause bugs at HEAD because the runtime-tier
`HeapKind` discriminator never crosses the JIT FFI boundary: the
`jit_bits_to_nanboxed` / `nanboxed_to_jit_bits` carrier-conversion functions
in `crates/shape-jit/src/ffi/object/conversion.rs:70,217` are themselves
SURFACE (`todo!("phase-2c §2.7.5: ...")`). When the W10 jit-playbook §5
carrier-conversion rebuild lands, any cross-boundary heap value carrying
the runtime `HeapKind` label as a u16 prefix will collide with the legacy
ordinal on the JIT side and dispatch to the wrong arm.

**Resolution:** dedicated sub-cluster (suggested name `W11-jit-legacy-
ordinal-disambiguation`). Either:

1. Switch the JIT-side `HK_*` constants to use distinct values starting
   at e.g. 256 (above the `HeapKind as u16` range) so they never collide
   regardless of how `HeapKind` grows.
2. Migrate JIT-internal allocations to use `HeapKind`-aligned ordinals
   directly (drop the "legacy" label), and add the missing kinds
   (`HK_RESULT = HeapKind::Result as u16` etc.) so producer + consumer
   arms reference the canonical discriminator.

Either is preferable to the current "the constants are legacy but the
values look like HeapKind ordinals" arrangement. Tracked here because
the JIT execute path itself is upstream-SURFACE; no fix lands until the
carrier-conversion rebuild creates a callsite that would hit the
collision.

---

*End of stack.* New items append at the bottom with the next available letter.
