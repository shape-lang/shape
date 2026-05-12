# Phase 2d — Close Summary

**Close commit:** `e22bffd2`
**Close date:** 2026-05-12
**Tag:** `phase-2d-close`
**Predecessor:** `phase-2c` (pre-existed bulldozer-strictly-typed)
**Successor:** Phase 3 cluster-0 (JIT rebuild — see `phase-3-kickoff-prompt.md`)

## Scope artifact

Phase 2d delivered the **VM-path strict-typing migration** in full:

- `ValueWord` deleted. The 2,650-line module and its 9 W-series follow-ups
  are gone. No "shim", no "FFI-boundary bridge", no rename-resurrection.
- All `HeapKind` ordinals 0..33 use kinded typed-`Arc` carriers per
  ADR-005 §1 single-discriminator + ADR-006 §2.3 typed-Arc shape.
- Method dispatch is kind-aware via `&[KindedSlot]` carrier slices per
  ADR-006 §2.7.10. No tag decode, no `is_heap()` probe, no kind-blind
  dispatch ABI.
- Mutation semantics: `&mut self` opt-in, compile-time-codegen
  writeback at the call site (no runtime dispatch shell extension).
  Self-returning + tuple-return variants both ship per ADR-006 §2.7.27.
- Universal `dyn Trait` with `Erase_T` auto-boxing, 6-variant
  `VTableEntry`, per-(impl,method) thunk generation per ADR-006 §2.7.24
  Q25.C. Direct + simple BoxedReturn + SelfArg + Generic + Compound +
  nested-BoxedReturn + Closure variants wired.
- Snapshot/restore at VM-direct level + state.* tier + 4 of 5 comptime
  introspection forms end-to-end via ADR-006 §2.7.5.1.A wire format.
- Concurrency primitives (Mutex=30, Atomic=31, Lazy=32) +
  Channel (24) + Result (27) + Option (28) + Range (26) +
  PriorityQueue (25) + Deque (23) + Iterator (22) + HashSet (21) +
  SharedCell (20) + Reference (19) + FilterExpr (18) + HashMap (17) +
  ModuleFn (33) + TraitObject (29) — all kinded.

## Out of Phase 2d scope

**JIT path is not functional at close.** This is not a Phase 2d
regression.

`jit_new_array` FFI stub at
`crates/shape-jit/src/ffi_symbols/array_symbols.rs:30` is referenced
unconditionally at `crates/shape-jit/src/compiler/ffi_builder.rs:73`.
Every JIT compilation aborts at compile time with:

    JIT compilation failed: phase-2c §2.7.14 / W10 jit-playbook §5:
    JitArray rebuild required for JIT execution path — FFI symbol
    jit_new_array is not registered.

This blocker pre-existed Phase 2d by ~3 weeks (phase-2c W10
§2.7.14 SURFACE). Item 3 verification confirmed structural,
end-to-end, and pre-existing via independent reproduction on
programs that don't touch arrays at all. Phase 2d's work
neither caused nor exacerbated it.

The JIT rebuild is **Phase 3 cluster-0** — four sub-clusters
catalogued and ready to dispatch:

1. `W11-jit-new-array` (~1-3 sessions) — kinded `Arc<TypedArrayData>`
   FFI implementations + JIT-side `TypedArray<T>` reintroduction per
   ADR-006 §2.7.14 Q15. Unblocks all `--mode jit`.
2. `W11-jit-carrier-conversion` (~1-2 sessions) —
   `jit_bits_to_nanboxed` / `nanboxed_to_jit_bits` per-arm re-encoding
   for the §2.7.5 JitFfiCarrier.
3. `W17-jit-legacy-ordinal-disambiguation` (~1 session) — 6 HK_*
   u16-prefix collisions catalogued in Item 3 (Range=12 vs Instant=12,
   Result=27 vs NativeView=15, etc.).
4. shape-jit lib test runner (~1 session) — convert
   `#[should_panic(expected = "phase-2c")]` on `extern "C" todo!()` to
   `#[ignore]` with §-cite. `extern C` can't unwind; SIGABRT aborts
   the test process.

## ADR-006 amendments landed in Phase 2d

| § | Phase | Subject |
|---|---|---|
| §2.7.6.A | Wave 3 | KindedSlot::from_temporal / from_instant constructors |
| §2.7.24 | Wave 2.5 | Typed-carrier monomorphization (TypedArrayData::HeapValue deletion + HashMapValueBuf + HeapKind::TraitObject + universal dyn Trait) |
| §2.7.25 | Wave 2.5 | Mutex / Atomic / Lazy HeapKinds (ords 30/31/32) |
| §2.7.5.1.A | Wave 2.6 | SerializableVMValue wire-format extension (15 arms) |
| §2.7.26 | Wave 3 | HeapKind::ModuleFn=33 pure-discriminator inline-scalar |
| §2.7.27 | Item 4 + W17-pop-mutation | Mutation semantics (self-returning + tuple-return) + operator sugar + widening lattice |

## HeapKind ordinal table at close

Contiguous 0..33. Free 34+.

 0  String          17  HashMap         24  Channel
 1  TypedObject     18  FilterExpr      25  PriorityQueue
 2  Closure         19  Reference       26  Range
 3  Decimal         20  SharedCell      27  Result
 4  BigInt          21  HashSet         28  Option
 5  DataTable       22  Iterator        29  TraitObject
 6  Future          23  Deque           30  Mutex
 7  TaskGroup                           31  Atomic
 8  TypedArray                          32  Lazy
 9  Temporal                            33  ModuleFn
10  TableView
11  Content
12  Instant
13  IoHandle
14  NativeScalar
15  NativeView
16  Char

## Sub-cluster delivery summary

~25+ sub-clusters dispatched across 9 sessions:

- **Wave 1**: T1-host-tier-marshal-rebuild, W17-make-closure
- **Wave 2**: W17-array-typed-receiver, W17-iterator-tableview,
  W17-references-mutation, W17-builtin-coercions, W17-foreign-ffi,
  W17-typed-module-exports, W17-array-closure-callback,
  W17-native-scalar-carrier, C1-temporal-lowering,
  W17-method-bodies-misc
- **Wave 2.5**: C3-expr-lowering-misc, W17-typed-object-mutation,
  W17-concurrency, C2-comptime-rebuild, W17-snapshot-surfaces
  (rescoped from W17-snapshot-resume)
- **Wave 2.5 surface-and-stop + rescope**: W17-typed-carrier-
  monomorphization → split into W17-typed-carrier-bundle-A +
  W17-trait-object-storage + W17-trait-object-emission +
  W17-snapshot-roundtrip
- **Wave 2.6**: W17-typed-carrier-bundle-A (with C+ pair-resolution),
  W17-trait-object-storage, W17-snapshot-roundtrip,
  W17-trait-object-emission
- **Wave 3**: W17-from-temporal-instant-constructors,
  W17-trait-object-thunks, W17-state-tier-roundtrip,
  W17-iterator-reference-rebuild, W17-out-of-bundle-A-followups,
  W17-comptime-vm-dispatch
- **Wave 4**: Item 5 (bench compile), Item 6 (--all-targets gate)
- **Final**: W17-mutation-writeback, Item 3 (JIT verify),
  W17-pop-mutation

## Process discipline that survived

- `scripts/verify-merge.sh` — 11 checks, exit-code-based merge gate.
  Runs `just verify-merge` / `just verify-merge-fast`. Caught at least
  4 take-both regex misses across the migration.
- `scripts/check-no-dynamic.sh` — frozen forbidden-symbol baseline.
  Zero regressions across all 25+ sub-clusters.
- Audit-first posture per playbook §0 — multiple sub-clusters audit-
  pivoted (C1-temporal, W17-method-bodies-misc, W17-from-temporal-
  instant-constructors, W17-typed-carrier rescope) rather than
  mechanically following the playbook framing. This caught at least
  3 receiver-recovery soundness violations (Temporal in objects/mod.rs,
  TypedObject in iterator op_iter_*, comptime build_config schema_id
  aliasing).
- Take-both merge resolution + manual conflict handling for AGENTS.md
  row overlaps + dispatch-table arm-line conflicts — supervisor
  pattern that scaled to ~5 concurrent agents per wave.

## Hardening backlog at close

Open items (deferred to Phase 3 cluster-1 or later):

| Item | Subject | Phase 3 cluster |
|------|---------|----|
| (b) | bump_closure_share retrofit (principled fix in `call_value_immediate_nb`) | 1 |
| (c) | Rust 2024 `unsafe_op_in_unsafe_fn` warnings sweep | 1 |
| (d) | `test_object_operations` SIGSEGV + 4 deferred simulation tests (v2-raw-heap audit) | 1 |
| (f) | `FieldType::Any` in comptime (predeclared-schema kind narrowing) | 1 |
| (g) | channel_ops pre-existing failures audit | 1 |
| (i) | JIT structural blocker | 0 (this is cluster-0) |

Resolved items: (a) CHECK 8 style rule, (e) wire-format §2.7.5.1.A,
(h) DropCall (partial — types with Drop impls work post-trait-object-
emission).

## Open surfaces flagged from Phase 2d (cite-tracked, not blockers)

Caught during Wave 3 dispatches; surface-and-stop with §-cites for
natural follow-up Phase 3 cluster-2 work:

- Nested-TypedArray Q25.A follow-ups for xml.parse children,
  csv.parse_records, arrow_module, datatable_methods rows /
  columnsRef / toMat.
- Marshal-return extensions for state.capture* / state.caller /
  state.args / state.locals at the marshal boundary
  (project_typed_return).
- VTableEntry::Closure dispatch (gated on W7 closure-trait-impl
  emission landing).
- W17-variadic-arg-kinds (register_typed_function arg_kinds-as-Bool
  placeholder; body-level only, dispatch path intact).
- Deep payload serialization workstreams: DataFrame in data/cache.rs,
  ExecutionContext in context/mod.rs, Deque/Channel/Iterator/Reference/
  FilterExpr/SharedCell/Mutex/Lazy per-kind opaque-stubs.

## Worktree cleanup

~17 Phase 2d worktrees retained. After this tag lands, batch-remove
authorized per handover §4:

  git worktree remove ../shape-w17-* ../shape-t1-* \
                      ../shape-c1-* ../shape-c2-* ../shape-c3-* \
                      ../shape-item-*

Close commits are in tagged history; worktrees no longer load-bearing.

## What `phase-2d-close` is and isn't

This tag marks **the strict-typing migration on the VM path, complete**.
It does NOT mark a ship-ready Shape v1 — JIT remains non-functional
(separately, as documented), the hardening backlog has 6 open items,
and the Wave-3 surfaces above are unfilled.

Shape v1 ships after Phase 3 clusters 0-2 + Phase 4 (trait Add/AddAssign
for user types) land cleanly. Estimated ~10-15 sessions from this tag
at observed Phase 2d velocity.

---

*See `phase-3-kickoff-prompt.md` for the cluster-0 dispatch contract.*
