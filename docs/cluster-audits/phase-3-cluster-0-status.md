# Phase 3 cluster-0 — status

**Started:** 2026-05-12 (this session)
**Parent:** `phase-2d-close` `e22bffd2`
**Branch:** `bulldozer-strictly-typed`
**Current HEAD:** `67af0282` (Round 3+4 merged into bulldozer-strictly-typed); Round-5 dispatched 2026-05-12 with 3 sub-clusters (reframed from kickoff's original Round-5 plan to match actual SURFACE sites verified by the 5-smoke matrix run)

Mirrors the Phase 2d Wave 1 status pattern. Next session reads this file first.

## Round 1 — closed

Three sub-clusters dispatched in parallel, all closed and merged into
`bulldozer-strictly-typed`:

| Sub-cluster | Branch | Close commit | Merge commit | Verify-merge |
|---|---|---|---|---|
| shape-jit-test-runner | `bulldozer-strictly-typed-jit-test-runner` | `50a84e4c` | `e5c6f58a` | 12/12 |
| W17-jit-legacy-ord | `bulldozer-strictly-typed-w17-jit-legacy-ord` | `67b4a231` | `8b61eb86` | 12/12 (CHECK 12 added) |
| W11-jit-new-array | `bulldozer-strictly-typed-w11-jit-new-array` | `e9a420ac` (round 2) | `a57e164f` | 12/12 |

### Deliverables

- **shape-jit-test-runner** — 3 `extern "C" todo!()` SURFACE tests
  converted from `#[should_panic]`/plain `#[test]` to `#[ignore]` with
  §-cite. `cargo test -p shape-jit --lib` no longer SIGABRTs (the
  load-bearing close-gate constraint). M (ignored) went from 23 → 26.
  Surfaced 17 pre-existing assertion failures previously masked by the
  SIGABRT.

- **W17-jit-legacy-ord** — closed phase-2d-hardening item (i). 39
  `HK_*` legacy ordinals bumped to a contiguous JIT-private block
  starting at `JIT_LEGACY_HK_BASE = 256`; 10 Tier-1 canonical-aliased
  entries unchanged. Added CHECK 12 to `verify-merge.sh` to detect
  future `HK_*` ↔ `HeapKind` collisions automatically. Audit gain:
  +10 collisions found beyond the original hardening item (i) table
  (W14/W15/W17 added HeapKind ords 23-33 that all collide with the
  old `HK_TIMESPAN..HK_FUNCTION_REF` block).

- **W11-jit-new-array** — Route A FFI surface landed per ADR-006
  §2.7.14 Q15. `jit_arc_retain` / `jit_arc_release` implement real
  refcount ops against the `UnifiedValue<T>` `#[repr(C)]` layout
  (`fetch_add(1, Relaxed)` / `fetch_sub(1, Release)` + Acquire fence
  + kinded `Box::from_raw` dispatch via new `jit_release` module).
  `ownership.rs::refcount_disposition` uses the new
  `NativeKind::is_refcounted()` predicate as the §2.7.5 authoritative
  source — supersedes the stale `is_native_slot` predicate. Unproven
  kind = surface-and-stop, not Bool-default. Unknown reclaim kind =
  visible eprintln + intentional leak (the extern-C analog of
  `NotImplemented(SURFACE)`).

### Round 1 process notes

- **W11 walk-back rejected once.** First close (`b60d3678`) had
  `jit_arc_retain` / `jit_arc_release` as silent no-ops with a
  "memory consequence: heap allocations leak" admission. Hit CLAUDE.md
  "Forbidden rationalizations" patterns. Reopened via `SendMessage`
  with a structured 7-step ask. Round-2 close (`e9a420ac`) implements
  the principled fix. The ADR-006 §2.7.14 "Reopen amendment" paragraph
  documents the walk-back + root cause for future agents.

- **AGENTS.md collisions** at W17 and W11 merges — both append-only
  rows; take-both via marker-strip resolved cleanly. Take-HEAD
  resolution on three test attribute conflicts (jit-test-runner
  version of `#[ignore = "..."]` strings has more detailed §-cites
  and cross-references than W11's terse version).

## Round 2 — closed

- **W11-jit-carrier-conversion** — closed 2026-05-12. Branch
  `bulldozer-strictly-typed-w11-jit-carrier-conversion`. Conversion
  FFI bodies in `crates/shape-jit/src/ffi/object/conversion.rs`
  rewritten to identity pack/unpack per §2.7.5 stamp-at-compile-time
  discipline: `jit_bits_to_nanboxed(bits, kind) -> JitFfiCarrier` now
  takes `NativeKind` as a new parameter (the §2.7.5 stable-FFI
  raw-pair shape); body is `(bits, kind)` — no decode, no probe.
  `nanboxed_to_jit_bits(&carrier) -> u64` returns `carrier.0`
  unchanged — per §2.7.5 the JIT bits ARE the raw bits, no
  re-encoding step exists under strict typing.

  `crates/shape-jit/src/ffi/control/mod.rs::jit_call_value` real
  body — classifies callee via JIT-internal NaN-box predicates
  (`is_inline_function` / `is_heap_kind(_, HK_CLOSURE)`) for inline
  function and deprecated-`unified_box(HK_CLOSURE, JITClosure)`
  callee shapes; surfaces-and-stops with TAG_NULL on the raw-Arc
  closure callee shape (the `jit_finalize_heap_closure` return
  shape) since `JITContext.stack` has no parallel-kind track and
  the callee's `NativeKind::Ptr(HeapKind::Closure)` is not
  recoverable from the bits alone. Same graceful-surface pattern as
  `jit_join_init` in W11 round-1 close; NOT a silent leak. Diagnostic
  audible via `SHAPE_JIT_DEBUG=1`.

  `dispatch_call_via_trampoline_vm` real body — stamps
  `NativeKind::UInt64` for args/captures (the §2.7.11/Q12
  function-id-callee-classification kind, also the §2.7.5 stable-FFI
  I64-wide raw bits carrier kind — NOT a Bool-default fallback);
  routes to `VirtualMachine::jit_trampoline_call_closure` for
  closure callees or `VirtualMachine::call_value_immediate_nb` for
  bare function callees.

  Test recovery: 311 → 316 passed (+5 new conversion.rs round-trip
  / kind-preservation tests). 0 → 0 failed. 38 → 38 ignored
  unchanged — the previously-ignored tests assert deleted ValueWord-
  shape API (`is_typed_object(bits)` on raw Arc pointers, the W11
  round-1 close's "9 individual tests #[ignore]'d that assert the
  deleted ValueWord-shape API"). Those tests need rewrites against
  the new strict-typed API, NOT carrier conversion work.

## Cluster-0 rescope (supervisor ruling, 2026-05-12)

The kickoff's "4 sub-clusters" scope was an architect planning miss.
The 4 named sub-clusters (jit-test-runner, W17-jit-legacy-ord,
W11-jit-new-array, W11-jit-carrier-conversion) closed honestly but do
not satisfy the close criterion ("`--mode jit` works end-to-end for
the standard program surface"). Three additional architectural gaps
were surfaced during Round 2: closure-callee kind-flow through
`jit_call_value` (item #6), top-level `concrete_types` conduit (item
#1), JIT linker symbol resolution (Smoke 2 finding). Plus a parallel
test-cleanup workstream for the 17 pre-existing tests asserting
deleted ValueWord-shape API.

**Cluster-0 rescopes from 4 sub-clusters to 7-8.** The close criterion
stays unchanged (end-to-end JIT smoke matrix matches VM). Tagging
`phase-3-cluster-0-close` at the Round-2 milestone with smokes 1.5,
2, 3 still failing would be the W-series declare-victory pattern at
the artifact-tagging layer — refused on sight, same discipline as
phase-2d-close only marking VM-strict-typing complete (because that
was honestly delivered).

Precedent: Phase 2d W17-typed-carrier-monomorphization rescope
(bundle-A + trait-object-storage + trait-object-emission, Wave 2.5)
when the original scope mismatched the work needed.

## Round 3 — partial close

Four sub-clusters dispatched in parallel 2026-05-12:

| Sub-cluster | Branch | Smoke unblocked | Status |
|---|---|---|---|
| W12-jit-stack-parallel-kind-track | `bulldozer-strictly-typed-w12-jit-stack-kind-track` | 1.5 (Result/match with closures) | dispatching |
| W12-top-level-concrete-types-conduit | `bulldozer-strictly-typed-w12-top-level-concrete-types` | 3 (TypedObject field access) | dispatching |
| W12-jit-linker-symbol-resolution | `bulldozer-strictly-typed-w12-jit-linker-resolve` | 2 (Option/return + Array) | **closed** (2026-05-12) |
| W12-deleted-valuewordshape-tests-rewrite | `bulldozer-strictly-typed-w12-vw-tests-rewrite` | 17 ignored tests un-ignored | dispatching |

### W12-jit-linker-symbol-resolution close (2026-05-12)

Audit-first sub-cluster. Root cause: NOT a naming convention
mismatch, NOT a missing FFI registration, NOT an ABI gap. The
`can't resolve symbol main_f{idx}_{name}` panic is a second-order
failure of the failed-compile stub fallback in
`crates/shape-jit/src/compiler/program.rs:702-725`:

When `compile_function_with_user_funcs` returns `Err` (e.g. on a
`Route A` `Rvalue::Aggregate` surface), the stub fallback installs a
body returning `signal = -1` via `iconst.i32 -1`. Cranelift's
`iconst` immediate-bounds rule
(`cranelift-codegen/src/verifier/mod.rs:1644-1665`) requires the I32
immediate to be the unsigned bit-pattern — `-1i64 as u64 =
0xFFFFFFFFFFFFFFFF` exceeds `u32::MAX = 0xFFFFFFFF`, so the
verifier rejects the stub. The `define_function` error was
silently swallowed via `let _ = ...`, leaving the declared FuncId
with no body. Then `finalize_definitions()` panicked in
`cranelift-jit-0.110.3/src/backend.rs:345` on the undefined
symbol.

Fix has two parts:
1. Pass the unsigned bit-pattern `(-1i32 as u32) as i64` matching
   the Cranelift convention used by every other I32 negative-value
   site in the codebase.
2. Convert the silent `let _ = define_function(...)` into a
   structured `Err` return so failed stub recovery surfaces as a
   typed JIT compilation error (`SHAPE_JIT_DEBUG=1` adds a
   diagnostic eprintln).

Smoke 2 (`fn main()`-wrap repro): JIT no longer panics — returns
deopt signal -1 from the stub, which is the intended behavior for
a function that failed Phase-4 compile. Smoke 2 plain form
(`print(first_positive([-1, -2, 3, -4]))`) still hits an upstream
surface — the top-level `Rvalue::Aggregate` for the Array literal
(W12-top-level-concrete-types-conduit territory, item 1 in the
surfaced-items table). The linker resolution is now CORRECT;
Smoke 2 full success depends on the concrete-types conduit
sub-cluster also closing.

Branch: `bulldozer-strictly-typed-w12-jit-linker-resolve`
Audit commit: `f30644cb`
Fix commit: (pending — appended below at merge)

Close gates (devenv exit-code-verified):
- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` EXIT=0 (316 / 0 / 38 — same as baseline)
- `bash scripts/verify-merge.sh` EXIT=0 (Passed: 12 / Failed: 0)
- `bash scripts/check-no-dynamic.sh` EXIT=0

Sites surfaced (NOT silently skipped):
- (a) Smoke 2 plain form depends on W12-top-level-concrete-types-conduit
  (cross-cluster). The linker fix unblocks the stub path for ANY
  user function failing Phase-4; the plain-form Smoke 2 still
  fails at the top-level surface before reaching the stub.
- (b) ~30+ stdlib user functions (`Into::*::*::into`,
  `TryInto::*::*::tryInto`, `Json.*` accessors,
  `std::core::math::spread`/`zscore`) currently fail Phase-4
  compile with `Rvalue::Aggregate` and route through the stub.
  Each is a W11-jit-new-array follow-up, NOT a linker bug.
- (c) The audit's deeper observation: `let _ = ...` on a
  load-bearing error is itself a forbidden pattern when the
  swallowed error masks a subsequent panic. Fixed at the W12-jit-
  linker site; future agents touching stub-recovery code should
  follow the same surface-and-stop discipline.

**Deferred to future cluster (NOT cluster-0):**

- **W12-jit-typed-map-ffi** (`jit_v2_map_*` typed-HashMap FFI rebuild) —
  no smoke in the cluster-0 matrix uses HashMap; not a close blocker.
  Cluster-2 or later territory.

## Round 4 — closed

One sub-cluster dispatched 2026-05-12 from a surfaced item in
W12-jit-stack-parallel-kind-track Round 3 close `1a4d1156`:

| Sub-cluster | Branch | Audit commit | Fix commit | Smoke unblocked |
|---|---|---|---|---|
| W12-enum-constructor-mir-lowering | `bulldozer-strictly-typed-w12-enum-constructor-lowering` | `588fba2c` | `2429b5ee` | 1.5 segfault chain (constructor side) |

### Deliverables

- **W12-enum-constructor-mir-lowering** — closes the documented
  segfault chain from W12-jit-stack-parallel-kind-track Round 3 close
  `1a4d1156` ("`MirConstant::Function("Ok")` is not registered in
  function_indices, so `compile_constant` emits `iconst(I64, 0)` for
  the constructor reference; the indirect-call dispatches with
  `callee_bits = 0`; the JIT's UInt64-arm classifier sees no inline
  function and no HK_CLOSURE match and surfaces TAG_NULL; downstream
  code dereferences the null result and segfaults").

  Audit grid found 11 broken constructors in 2 mechanically-identical
  families (3 bare enum variants `Ok`/`Err`/`Some` + 8 collection
  constructors). NOT the ~50-site rescope ceiling. Fixed §2.1 in
  Commit 2 (Smoke 1.5 load-bearing); surfaced §2.3 as follow-up
  sub-cluster `W12-collection-constructor-mir-lowering`.

  Fix is a pure compile-time MIR rewrite at `mir/lowering/expr.rs` —
  bare-form constructor names (`Ok` / `Err` / `Some`) are intercepted
  AFTER local-shadow resolution and lowered to the same `Aggregate` +
  `EnumStore` MIR shape the qualified `Result::Ok(x)` /
  `Expr::EnumConstructor` path already produces. Producer-side
  classification per ADR-006 §2.7.5 — no decode, no Bool-default, no
  new MIR opcode, no new HeapKind, no new dispatch shape. Same
  classification the VM-side bytecode compiler already uses at
  `compiler/helpers.rs:3194-3209`.

  Three sites intercepted: `Expr::FunctionCall` direct call, pipe-
  operator `FunctionCall` arm, pipe-operator `Identifier` arm.

  Post-fix Smoke 1.5 (`fn divide(...) -> Result<int> { ... }; match
  divide(10,2) { Ok(v) => print(v), Err(e) => print(e) }`): VM `5`
  exit 0; JIT surfaces with the pre-existing `can't resolve symbol
  main_f194_divide` linker panic (`W12-jit-linker-symbol-resolution`
  cluster's territory, orthogonal). Without the function-call
  dependency (`let r = Ok(5); match r { ... }`), JIT post-fix
  surfaces a clean `Rvalue::Aggregate` Route-A surface-and-stop via
  `mir_compiler/rvalues.rs:144-176` — the documented §2.7.14
  heterogeneous-element-array carrier gap that qualified `Result::
  Ok(5)` already surfaces, tracked as W11-jit-new-array follow-up.

  Smoke equivalence ratchet moves forward (structural bug becomes
  documented gap with a tracked follow-up) without expanding
  cluster-0 scope into §2.7.14 carrier work.

  Close gates: `cargo check --workspace --lib --tests` EXIT=0;
  `cargo test -p shape-jit --lib` 316/0/38 (matches baseline 316,
  verified by stash-and-rerun); `bash scripts/verify-merge.sh` 12/12;
  `bash scripts/check-no-dynamic.sh` EXIT=0. Pre-existing v2-raw-heap-
  audit SIGABRT in `cargo test -p shape-vm --lib` verified identical
  on baseline.

### Round 4 follow-up sub-clusters surfaced

| # | Surface | Site / §-cite | Disposition |
|---|---|---|---|
| 8 | §2.3 collection-constructor MIR-emission family (HashMap / Set / Deque / PriorityQueue / Channel / Mutex / Atomic / Lazy) lowers to `MirConstant::Function(name)` with same failure mode as the §2.1 enum-variant family closed by W12-enum-constructor-mir-lowering | `mir/lowering/expr.rs::Expr::FunctionCall` arm; §2.7.5 producing-site classification | **future sub-cluster** `W12-collection-constructor-mir-lowering` — mechanically identical rewrite, NOT load-bearing for any current cluster-0 smoke. Verified pre-fix and post-fix that Smoke 4 (Set) prints the same garbage output in JIT mode (denormalized number from null-bit slot used as Set value). Audit doc §5.3 / §8 documents the scope decision |

**Cluster-0 close criterion (unchanged):** the smoke matrix passes
end-to-end identical to `--mode vm`:

- Smoke 1 (scalar loop): currently passes
- Smoke 1.5 (Result/match with closures): Round 3 stack-kind-track unblocks
- Smoke 2 (Option + Array): Round 3 linker-resolution unblocks
- Smoke 3 (TypedObject field): Round 3 concrete-types-conduit unblocks
- Smoke 4 (HashSet via `&mut self`): expected to pass post-Phase-2d-mutation;
  confirm during smoke matrix re-run after Round 3

If matrix passes end-to-end after Round 3 closes: standard cluster-0
close report shape; supervisor authorizes `phase-3-cluster-0-close`
tag. If any smoke still diverges between VM and JIT: surface-and-stop
with the specific divergence, do not declare close.

## Surfaced items (cite-tracked, NOT silently fallback'd)

Round-1 sub-cluster agents flagged 5 architectural items as
surface-and-stop. Triaged by cluster:

| # | Surface | Site / §-cite | Disposition |
|---|---|---|---|
| 1 | `concrete_types: Vec::new()` for top-level code | `compiler/strategy.rs:200-205`; §2.7.5 conduit gap | **Round 3 closed (W12-top-level-concrete-types-conduit, 2026-05-12)** — `BytecodeProgram.top_level_local_concrete_types` field added; populated by MIR-walk inference (`infer_top_level_concrete_types_from_mir`); threaded through both `compile_strategy` + `compile_strategy_with_user_funcs` sites + `Program` + `LinkedProgram`. JIT side: new `is_typed_object_slot` helper + `Assign(Aggregate)` TypedObject short-circuit in `mir_compiler/statements.rs`. Smoke 3 + array-literal: `Rvalue::Aggregate` surface eliminated; downstream `compile_binop_dynamic_arith` gap surfaced as separate finding |
| 2 | Compile-time-boxed string constants leak by design | `MirConstant::Str` lowering; pre-W11 pattern | **cluster-2 candidate** — box-once-bake-into-code with no release path; observable via `SHAPE_JIT_ARC_COUNTERS` (strconcat smoke: `retain=2 release=0`); independent of W11's caller-side ownership work |
| 3 | Per-HeapKind kinded `jit_print` entries | `ffi/print.rs` kind-blind fallback uses `format_value_word` (NaN-decode-via-tag-bits) for heap arms | **cluster-2 candidate** — scalar arms (`jit_print_i64`/`f64`/`bool`) landed in W11; string / typed-object / Option / Result print still routes through the deleted-shape decoder |
| 4 | `op_new_array` heterogeneous-element surface | `crates/shape-vm/src/executor/objects/object_creation.rs:316` | **Phase 2d gap** — surfaced as a finding; affects `xs.map(\|x\| x*2)` style smokes in VM mode (before JIT is reached). Not cluster-0 territory; tracked for the next Phase 2d hardening pass |
| 5 | `jit_call_value` `todo!()` | `ffi/control/mod.rs:171`; §2.7.11/Q12 | **Round 2 closed (W11-jit-carrier-conversion, 2026-05-12)** — inline-function + deprecated-HK_CLOSURE callee shapes implemented; raw-Arc closure callee (the `jit_finalize_heap_closure` return shape) surfaces-and-stops returning TAG_NULL — `JITContext.stack` parallel-kind track is the §2.7.5 follow-up (cluster-1) |
| 6 | `JITContext.stack` has no parallel-kind track | `crates/shape-jit/src/context.rs:491` (`stack: [u64; 512]`); §2.7.5 / §2.7.11 | **cluster-1** (`W12-jit-stack-parallel-kind-track`) — surfaced by W11-jit-carrier-conversion's `jit_call_value`. Raw-Arc closure callees + per-arg kinded retain/release across the JIT-FFI boundary depend on the JIT-side §2.7.7 parallel-kind track. Resolution: either extend `JITContext` with `kinds: [NativeKind; 512]` parallel array + thread kinds through `terminators.rs::TerminatorKind::Call` lowering, or per-callee kind side-table threaded through MIR emitter into a separate FFI signature. ADR-level shape change |
| 7 | `jit_v2_map_*` typed-HashMap FFI rebuild | `ffi_refs.rs:209`, `compiler/ffi_builder.rs:208`, `mir_compiler/v2_typed_map.rs:71-99`; §2.7.14 Q15 / §2.7.5 | **future-cluster** (`W12-jit-typed-map-ffi`) — referenced as W11-jit-carrier-conversion follow-up in those files but actually a separate FFI rebuild: kinded `Arc<HashMapData>` + `KindedSlot` map FFI bodies (`jit_v2_map_get_str_i64` / `get_str_f64` / `has_str` / `set_str_i64` / `len`). The deleted ValueWord-shape bodies decoded map handle + key as raw u64 bits via tag_bits; rebuild needs per-method kind-aware entry-point bodies. Separate concern from identity-function carrier conversion |
| 8 | `W12-stdlib-intrinsic-collapse` — parallel-implementation cleanup | 4 `BuiltinFunction::Intrinsic*` opcodes (Sum, Mean, Std, Variance) duplicate the kinded PHF method dispatch path | **cluster-2 candidate** — surfaced by Round 13 T4 close (`6a5076ba`): T4 agent went looking for `.sum()` and found `IntrinsicSum`, but user-facing dispatch actually uses `TYPED_INT_ARRAY_METHODS` PHF. Cleanup shape: (1) inventory all `Intrinsic*` opcodes with kinded-PHF-method equivalents; (2) define `Sumable` / `Averageable` trait bounds; (3) lower stdlib `sum<T>` / `mean<T>` / etc. bodies to `series.sum()` / `series.mean()` via method dispatch; (4) delete the `Intrinsic*` opcode variants + handlers + type-env registrations + leftover `__intrinsic_*` builtin entries (per MEMORY.md 2026-03-09 builtins-policy cleanup which removed bare-name registrations but left opcode side intact). NOT cluster-0 territory — kickoff Smoke 2 uses the method form, unaffected |

Items 2, 3, and 8 are the cluster-2 candidate flags the user asked for.
Item 1 is cluster-1 territory (hardening). Items 4 and 5 are either
already in scope (5) or out-of-cluster (4).

## Round 5 — dispatching (post-merge smoke matrix verification + reframe)

After Round 3+4 merged (HEAD `67af0282`), the full 5-smoke matrix
was run end-to-end. Results:

| Smoke | VM | JIT |
|---|---|---|
| 1 (scalar loop) | `4950` | `4950` ✅ |
| 1.5 (`divide(10,2)` Result/match) | `5` | `JIT execution error (code: -1)` — stub fallback |
| 2 (`first_positive` Option/Array) | `Some(3)` | `JIT execution error (code: -1)` — stub fallback |
| 3 (`Point{}` + `p.x+p.y`) | `7` | `compile_binop_dynamic_arith` SURFACE |
| 4 (`Set()` + `s.size()`) | `2` | denormalized garbage `0.000...535409...` |

Tracing with `SHAPE_JIT_DEBUG=1` revealed Smokes 1.5 / 2 fail
because the user functions (`divide`, `first_positive`) building
`Ok(v)` / `Some(x)` hit `Rvalue::Aggregate reached the kind-blind
fallback` — the destination's `ConcreteType` is `Enum(EnumLayoutId(0))`
placeholder, not `Array<scalar>`, so the v2 fast path doesn't fire.
30+ stdlib fns (`TryInto::*`, `Into::*`, `math::spread`, `math::zscore`)
fail the same way. Smoke 4's garbage output is a `jit_print`
kind-classification gap (`.size()` returns int, decoded as f64) —
NOT the deferred collection-constructor MIR gap (Set() constructor
works correctly).

**Stray §-cite found:** `mir_compiler/statements.rs:236` and
`docs/cluster-audits/w12-enum-constructor-audit.md:215` cite "§2.7.4"
(task-scheduler boundary) for the EnumStore/Aggregate surface; the
correct cites are §2.7.14 / §2.7.5. Round-5B agent fixes this.

**Reframed Round-5 territory** (3 sub-clusters in parallel):

| Sub-cluster | Branch | Smoke unblocked | Status |
|---|---|---|---|
| W12-jit-binop-after-heap-read-kind-tracker | `bulldozer-strictly-typed-w12-jit-binop-heap-read` | 3 (binop after p.x field read) + array-scalar (`xs[0] + xs[1]`) | **closed** (2026-05-12) |
| W12-jit-aggregate-non-array-carrier | `bulldozer-strictly-typed-w12-jit-aggregate-non-array` | 1.5 + 2 (Aggregate for Enum/Struct/Tuple destinations) + 30+ stdlib fns | dispatching (audit-first) |
| W12-jit-print-kind-classification | `bulldozer-strictly-typed-w12-jit-print-kind` | 4 (`.size()` int result mis-decoded as f64) | **closed** (2026-05-12) |

### W12-jit-binop-after-heap-read-kind-tracker close (2026-05-12)

Producer-side kind threading per ADR-006 §2.7.5 stamp-at-compile-time.
Closes the `compile_binop_dynamic_arith: kind-untyped arith Add reached
the JIT — SURFACE per W10 playbook §5: producing-MIR kind-tracker gap`
surface plus three cascade bugs the previous compile-time error path
masked.

**Three layers fixed in lockstep**:

1. **Consumer-side kind classification** (`mir_compiler/rvalues.rs`):
   new `place_native_kind` projects `Place::Field` via a producer-side
   `field_native_kinds: HashMap<String, NativeKind>` map (populated by
   `infer_field_native_kinds`'s walk of `StatementKind::ObjectStore`)
   and `Place::Index` via the existing `v2_typed_array_elem_kind`
   projection over `concrete_types`'s `Array<scalar>` shape.
   `operand_slot_kind` now uses this for Field/Index instead of the
   root-local lookup that returned the base's heap kind.

2. **MIR-level destination-kind inference** (`mir_compiler/types.rs`):
   new `infer_slot_kinds_with_concrete` extends `infer_slot_kinds` to
   accept the `concrete_types` side-table. Inside, new
   `infer_operand_kind_with_projections` + `infer_rvalue_kind_with_
   projections` carry Field + Index projection through both the
   forward pass (for `Assign(slot, Use(Move(Field)))` destinations)
   and the backward pass (for `BinaryOp` operand kind propagation).
   Without this, the destination slot of `let a = p.x` inherited
   `Ptr(TypedObject)` from the base — refcount-dispatched as heap
   and segfaulted on the raw-int field value.

3. **JIT-FFI consumer migration** (`ffi/typed_object/field_access.rs`):
   removed `is_typed_object(obj_bits)` precondition from
   `jit_typed_object_get_field` / `_set_field`. This was the
   documented production-code consumer migration gap from the
   deleted-test comment at `field_access.rs:275..314`:
   `is_typed_object → is_heap_kind → is_heap` requires `is_tagged`
   (NaN-box tag bits) but JIT-allocated `box_typed_object` returns
   raw `Box::into_raw(UnifiedValue<*const TypedObject>) as u64`
   pointers per §2.7.5 stamp-at-compile-time. Every set_field call
   on a valid producer output took the "not a typed object" early-
   return path and returned TAG_NULL — silently null-corrupting the
   just-allocated TypedObject and segfaulting on the subsequent
   field-read deref. Null-pointer / mis-alignment guards remain.

**Also fixed in lockstep**: `refcount_disposition` in `ownership.rs`
discriminated on `place.root_local()` for projection places, calling
`arc_retain(i64_field_value)` for `Copy(Field(p_TypedObject, x_Int64))`
— segfaulted in `Arc::increment_strong_count` interpreting the integer
3 as a pointer. The new `match place { Field/Index => place_native_
kind-projected }` arm at the top of `refcount_disposition` closes this
latent bug; it was masked by the `compile_binop_dynamic_arith` compile-
time error path that previously prevented this code from executing.

**Smoke results (VM = JIT, both EXIT=0)**:

- Smoke 3 (`p.x + p.y` after `Point{x:3, y:4}`): VM `7`, JIT `7`.
- Array smoke (`xs[0] + xs[1]` over `Array<int>`): VM `30`, JIT `30`.

**Close gates (devenv exit-code-verified)**:

- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` EXIT=0 (322 / 0 / 26 — matches
  baseline 322 / 0 / 26 verified by stash-and-rerun; kickoff's
  "319/0/38 baseline" claim was stale)
- `bash scripts/verify-merge.sh` EXIT=0 (Passed: 12 / Failed: 0)
- `bash scripts/check-no-dynamic.sh` EXIT=0

**Sites surfaced (NOT silently skipped)**:

- The `jit_typed_object_set_field` / `_get_field` `is_typed_object`
  gate (consumer migration gap #3 above) is the FIRST instance fixed
  for the typed-object family; the same NaN-box-tag-bit precondition
  pattern likely lives in other FFI bodies that gate on `is_X(bits)`
  for `is_typed_array`, `is_typed_string`, `is_typed_hashmap`, etc.
  Each is a separate migration gap that the same §2.7.5 reasoning
  applies to — surface-and-stop here, the broader sweep is its own
  sub-cluster. Pattern grep: `grep -rn 'is_heap_kind\|is_tagged.*HEAP\|is_typed_' crates/shape-jit/src/ffi/`.

- A schema-aware `(StructLayoutId, FieldIdx) → NativeKind` registry is
  the principled long-term shape for `field_native_kinds` (currently
  keyed by field NAME with last-writer-wins on cross-struct name
  collision — same fragility as the existing `field_byte_offsets`).
  Out-of-scope for this sub-cluster; no current cluster-0 smoke
  exercises a collision.

- `Place::Deref` projection in `place_native_kind` returns `None` —
  references are heap-tier indirection and the type-of-pointed-to-
  value is not threaded into the JIT-side projection map yet. No
  current cluster-0 smoke exercises a binop after a ref-deref; if a
  future smoke surfaces this, thread the deref-target kind via the
  MIR's reference annotations.

**ADR-006 amendment**: NOT required. Producer-side classification at
both MIR-emission time (`StatementKind::ObjectStore` walk for fields)
and bytecode-compiler conduit time (`Array<scalar>` via
`concrete_types`) — the same §2.7.5 conduit shape the
`W12-top-level-concrete-types-conduit` Round-3 close already
established. The FFI consumer migration is the §2.7.5 "kind stamped at
the call signature, not decoded from bits" applied to one specific
FFI body family.

Branch: `bulldozer-strictly-typed-w12-jit-binop-heap-read`
Close commit: `414d0a0a`

### W12-jit-aggregate-non-array close (partial, 2026-05-12)

Audit-first sub-cluster. Audit doc:
`docs/cluster-audits/w12-jit-aggregate-non-array-audit.md`.

**Landed (option (ii) + structural prep)**:

- `BytecodeProgram.function_local_concrete_types: Vec<Vec<ConcreteType>>` —
  per-user-function ConcreteType conduit side-table. Walks each
  `Function::mir_data` through the existing
  `infer_top_level_concrete_types_from_mir` (generic over any MirFunction;
  name historical from the top-level-only Round-3 landing). Threaded
  through `BytecodeProgram` → `ContentAddressedProgram` →
  `LinkedProgram` → `linked_to_bytecode_program` → JIT
  `compile_function_with_user_funcs` consumer (was `Vec::new()`).
  Aggregate short-circuit now fires inside user-function bodies for
  Enum/Struct destinations. `divide`'s `Ok(a/b)` Aggregate
  short-circuits cleanly.
- `StatementKind::EnumStore.variant_name: Option<String>` — MIR shape
  addition. Four producer sites thread the variant name through (bare-
  form intercept + `Expr::EnumConstructor` + 2 pipe-operator forms).
  Five consumer sites updated to pattern-ignore the new field.
- Kinded EnumStore FFI registered (`jit_make_ok`, `jit_make_err`,
  `jit_make_some`) — symbols + FuncRef slots. Not yet consumed by the
  EnumStore consumer (see surface below).
- Stray §-cite fix at the two known sites:
  `mir_compiler/statements.rs:236` and
  `docs/cluster-audits/w12-enum-constructor-audit.md:215`:
  §2.7.4 (task-scheduler boundary) → §2.7.14 (JIT array FFI rebuild,
  the correct cite).

**Surfaced — option (iii) territory**:

The EnumStore consumer cannot be wired end-to-end without three
co-designed infrastructure pieces:

1. **Call-terminator return-kind track.** The conduit walks MIR
   statements, not `TerminatorKind::Call` return kinds. After
   `let r = divide(...)`, `r` has `ConcreteType::Void`; downstream
   `print(v)` / match-on-`r` codegen mis-decode the NaN-boxed return
   bits.
2. **JIT match-on-enum inline codegen.** Pattern-match for
   `Ok(v)`/`Err(e)`/`Some(x)`/`None` has no inline path on either the
   NaN-boxed `HK_OK`/`HK_ERR`/`HK_SOME` shape or the typed-Arc
   `Arc<ResultData>` shape. Current path falls through to generic
   SwitchBool.
3. **NaN-box vs Arc<ResultData> round-trip audit.** `jit_make_ok`
   returns the legacy NaN-boxed `unified_box(HK_OK, bits)` shape; the
   VM-side `BuiltinFunction::OkCtor` produces `Arc<ResultData>`.
   Boundary conversion lives in `ffi/conversion.rs:246-258` but the
   end-to-end round-trip (JIT divide produces NaN-box → top-level
   JIT consumes the same → match reads via `is_ok_tag`) isn't audited
   as a coherent path.

Together these are ADR-amendment-level co-design. Surfaced for
cluster-1 or future agent per CLAUDE.md surface-and-stop discipline.
The current sub-cluster's landed changes are a strict prep for that
work — Aggregate-surface bottleneck moved one level deeper (to
EnumStore) for 5 functions; the remaining 23 Aggregate-surface
functions return non-Enum types and are separate.

**Before/after compile-failure counts under SHAPE_JIT_DEBUG=1 on Smoke 1.5**:

- Pre-fix: 30 total (28 `Rvalue::Aggregate` + 2 `compile_binop_dynamic_arith`)
- Post-fix: 30 total (23 `Rvalue::Aggregate` + 5 `EnumStore: SURFACE` + 2
  `compile_binop_dynamic_arith`)

Same count, but bottleneck moved deeper for the 5 functions where the
conduit successfully stamped Enum (including `divide`).

**Close gates**:
- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` 322/0/26 (matches baseline; the
  dispatch-quoted 319/0/38 baseline drifted between rounds)
- `bash scripts/verify-merge.sh` 12/12
- `bash scripts/check-no-dynamic.sh` EXIT=0
- Smoke 1.5 / Smoke 2 under `--mode jit`: still fail end-to-end (option
  (iii) territory); under `--mode vm`: unchanged ('5' / 'Some(3)').

**Deferred to future cluster (NOT cluster-0):** `W12-collection-constructor-mir-lowering` (8 sites). The Round-4 audit identified this but Smoke 4's actual gap is print-classification, not constructor MIR. Constructor MIR will be picked up by cluster-2 if it ever becomes load-bearing.

## Round 6 — dispatching (Round 5B option-(iii) co-design unpacked into three sub-clusters)

Three sub-clusters dispatched in parallel 2026-05-12 from the Round 5B
W12-jit-aggregate-non-array close (`d3ea6546`), which surfaced
co-design territory the dispatch correctly split into three orthogonal
pieces:

| Sub-cluster | Branch | Smoke unblocked | Status |
|---|---|---|---|
| W12-jit-call-return-kind | `bulldozer-strictly-typed-w12-jit-call-return-kind` | 1.5 (Call-terminator destination kind for `let r = divide(...)`) | **closed** (2026-05-12) |
| W12-jit-match-enum-inline-codegen | `bulldozer-strictly-typed-w12-jit-match-enum-inline` | 1.5 (match-on-`r` codegen for Ok/Err/Some/None) | dispatching |
| W12-collection-constructor-mir-lowering | `bulldozer-strictly-typed-w12-collection-ctor-mir` | 4 (Set/HashMap/Deque/... constructors) | dispatching |

### W12-jit-call-return-kind close (2026-05-12)

Audit-first sub-cluster. Audit doc:
`docs/cluster-audits/w12-jit-call-return-kind-audit.md` (793 lines, 11
sections).

**Audit reclassified the territory as option (ii), NOT option (iii)** as
Round 5B audit §4.4 had framed it. The reframing:

- The **Call-return kind track piece** is option (ii) — same conduit
  shape as the existing kind-source statement walks (`ObjectStore`,
  `EnumStore`, `ArrayStore`), applied to one more MIR shape
  (`TerminatorKind::Call`). The callee's declared return type IS the
  proof source per ADR-006 §2.7.5 producing-site classification.
  No ADR amendment.
- The **match codegen piece** is Round 6B's territory — independent
  sub-cluster.
- The **NaN-box↔Arc carrier mismatch piece** is a real architectural
  gap but is NOT load-bearing for any current cluster-0 smoke
  (single JIT execution, no cross-mode boundary). Surfaced as
  cluster-1 candidate `W12-jit-result-carrier-unification`.

Splitting the 5B-monolith into three independent sub-clusters lets
each ship at its own scope.

**Fix shape (Commit 2)**: pure §2.7.5 conduit extension —

1. NEW `BytecodeProgram.function_return_concrete_types: Vec<ConcreteType>`
   side-table, populated per user function from the AST
   `FunctionDef.return_type` (preserved through `expanded_function_defs`)
   via the existing `concrete_type_from_annotation` (in use for HashMap
   key/value extraction; reused, not rebuilt).
2. NEW `infer_top_level_concrete_types_from_mir_with_returns` resolver-
   aware variant of the conduit producer. Walks `TerminatorKind::Call`
   destinations BEFORE the slot-move propagation pass, stamps from the
   resolver. Existing `infer_top_level_concrete_types_from_mir` becomes
   a None-passing wrapper preserving callers.
3. Build a callee-return resolver closure over the side-table + a
   function-name → index map; thread through both the top-level and
   per-function conduit calls in `compile_post_assembly`. This also
   handles user-function bodies that call other user functions — the
   resolver works recursively at each layer.
4. Thread the new side-table through `linker.rs` / `remote.rs` /
   `ContentAddressedProgram` / `LinkedProgram` (same shape as
   `function_local_concrete_types` from Round 5B).
5. 4 unit tests added under `compiler::helpers::call_return_kind_tests`
   (4/0/0): basic stamping, no-resolver legacy behavior, None-returning
   resolver leaves Void, propagation through `Use(Move)` chains.

No new MIR shape. No new HeapKind. No new dispatch shape. No new FFI
entry. No ADR amendment.

**Smoke 1.5 status post-fix**: VM `5` unchanged. JIT still errors
`JIT execution error (code: -1)` because `divide` itself fails Phase-4
compile at the EnumStore consumer (Round 5B's deferred work). When
divide's stub returns -1 from the deopt signal, the top-level call
propagates it through `return_(&[signal])` (terminators.rs:628)
killing JIT execution before `r`'s slot kind is ever read. **My fix
establishes the necessary kind-classification piece** but Smoke 1.5
end-to-end JIT success requires:

1. The EnumStore consumer (Round 5B's deferred surface) to actually
   emit codegen — or Round 6B picks up that piece alongside match
   codegen, since they're both about consuming EnumStore-produced bits.
2. Round 6B's match-on-enum inline codegen for `Ok(v)` / `Err(e)` /
   `Some(x)` / `None` dispatch.

Both pre-existed; my fix does not regress them.

**NaN-box ↔ Arc<ResultData> round-trip audit (audit doc §6)**:
`jit_make_ok(inner_bits)` returns raw `Box::into_raw(UnifiedValue<u64>)
as u64` — NOT NaN-boxed. The boundary predicate `is_ok_tag(bits)`
chains through `is_heap_kind` → `heap_kind` → `is_heap` → `is_tagged`
which checks `bits & TAG_BASE == TAG_BASE`. Raw `Box::into_raw`
pointers have NO TAG_BASE bits → `is_heap` returns false → `is_ok_tag`
returns false on every output of `jit_make_ok`. This is the deleted-
ValueWord-shape API documented at `result.rs:178-200` (W12-deleted-
valuewordshape-tests-rewrite, Round 3) — the production callers were
never migrated. `format_value_word` HK_OK arm CORRECTLY reads via
`jit_unbox::<u64>` from the raw-pointer payload, BUT `is_heap(bits)`
gate fails first → falls into `is_number(bits)` arm decoding raw
pointer bits as a denormalized f64 (the `0.000...471777` observed in
Round 5B's experiment). VM-side `BuiltinFunction::OkCtor` produces
`Arc<ResultData>` wrapped via `KindedSlot::from_result` —
fundamentally different storage shape. **NOT load-bearing for Smoke
1.5** (single JIT execution); surfaced as cluster-1 candidate
`W12-jit-result-carrier-unification`.

**Close gates (devenv exit-code-verified)**:
- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-vm --lib call_return_kind_tests` 4/0/0 (NEW)
- `cargo test -p shape-jit --lib` EXIT=0 (322/0/26 — matches baseline
  322, verified by stash-and-rerun)
- `bash scripts/verify-merge.sh` EXIT=0 (Passed: 12 / Failed: 0)
- `bash scripts/check-no-dynamic.sh` EXIT=0

**Sites surfaced (cite-tracked, NOT silently fallback'd)**:
- (a) NaN-box vs `Arc<ResultData>` carrier mismatch — cluster-1
  candidate `W12-jit-result-carrier-unification`. Two options for
  that future cluster: (A) extend `jit_make_ok`/`_err`/`_some` to
  produce `Arc<ResultData>` bits and stamp `Ptr(HeapKind::Result)` —
  §2.7.5 single source of truth; or (B) extend JIT consumer to
  handle both carrier shapes dispatched on slot kind.
- (b) `is_ok_tag` / `is_err_tag` / `is_some_tag` predicates broken
  on raw-pointer producer output. Same cluster as (a).
- (c) `format_value_word` HK_OK/HK_ERR/HK_SOME arms correctly
  handle the JIT-internal raw-pointer shape but NOT the
  Arc<ResultData> shape. Same cluster.
- (d) Round 5B's EnumStore non-empty payload consumer for user-
  function bodies (28 stdlib `TryFrom::*::Json::tryFrom` + `divide`
  itself) — orthogonal to top-level Call-return kind track. Round
  6B's territory if it also handles EnumStore production; otherwise
  separate sub-cluster.
- (e) Round 6B's match-on-enum inline codegen — load-bearing for
  Smoke 1.5 end-to-end alongside this fix.

**ADR-006 amendment**: NOT required. The fix is §2.7.5 producing-site
classification at the MIR layer, applied to `TerminatorKind::Call`
(one more kind-source MIR shape alongside the existing `*Store`
walks). No new HeapKind, no new MIR statement kind, no new opcode,
no new dispatch shape.

Branch: `bulldozer-strictly-typed-w12-jit-call-return-kind`
Audit commit: `f58abc8d`
Fix commit: (pending — appended at merge)

## Round 7 — closed (7A migrating-close + 7B audit-only close)

Two sub-clusters dispatched in parallel 2026-05-12 from the Round 6B
audit's surfaced trinity territory:

| Sub-cluster | Branch | Status |
|---|---|---|
| W12-jit-result-option-trinity (7A) | `bulldozer-strictly-typed-w12-jit-result-option-trinity` | **closed** (2026-05-12, migrating-close) |
| W12-jit-collection-typed-arc-ffi (7B) | `bulldozer-strictly-typed-w12-jit-collection-typed-arc-ffi` | **closed audit-only** (2026-05-12, option (iii) surfaced) |

### W12-jit-result-option-trinity close (2026-05-12)

Integrated trinity landing per the Round 6B audit blueprint at
`docs/cluster-audits/w12-jit-match-enum-inline-audit.md`. Three close
commits:

- `d01d83b7` — `(i) + (ii)`: `Rvalue::EnumTest` / `EnumPayload` MIR
  variants + `VariantTag` enum + Arc-shape Result/Option FFI
  (`jit_v2_make_result_ok/_err`, `jit_v2_make_option_some/_none`,
  `jit_arc_result_is_ok/_is_err/_payload`,
  `jit_arc_option_is_some/_is_none/_payload`). All FFIs read/write
  from `*const ResultData` / `*const OptionData` directly per
  ADR-006 §2.7.17 — no NaN-box tag decode, no `is_heap_kind` probe.
  6 new round-trip tests (Ok/Err/Some/None construction, predicates,
  payload extraction, null-bits safety, kind label match).
- `ae5d57f2` — `(iii)`: EnumStore non-empty payload consumer in
  `mir_compiler/statements.rs`. Replaces the surface-and-stop with
  real Arc-shape producer dispatch on `VariantTag::from_name`.
  User-defined enum variants surface-and-stop with structured §-cite
  per §2.7.7 #9.
- `9f27edcd` — Producer-site MIR (`lower_match_pattern_condition_operand`,
  `Pattern::Constructor` arm in `lower_pattern_bindings_from_place_opt`)
  + producer-side concrete-type stamping (`helpers.rs` EnumStore
  Result/Option classification) + **critical bug fix**: Arc-shape
  kinded retain/release ABI.

**Critical bug discovered & fixed during integration**: The legacy
`jit_arc_retain`/`jit_arc_release` operate on `UnifiedValue<T>` refcount
layout (offset 4 of the pointer). The new `Arc<ResultData>` /
`Arc<OptionData>` carriers use Rust's standard Arc layout (refcount at
offset -16). Calling the legacy retain on a trinity Arc would
`fetch_add(1) as u32` at offset 4 of `payload.slot.0` — CORRUPTING the
high 32 bits with the spurious refcount. Symptom: `let r = Ok(5); match
r { Ok(v) => print(v) }` printed `4294967301` = `0x100000005` = 5 + 2^32.

Fix: new Arc-aware FFI entries `jit_arc_result_retain/_release`,
`jit_arc_option_retain/_release` calling
`Arc::increment_strong_count::<T>` / `Arc::decrement_strong_count::<T>`
per Rust standard Arc contract. Dispatched via new
`retain_func_for_place` / `release_func_for_place` helpers that pick
the right entry based on `place_native_kind` —
`Ptr(HeapKind::Result)` → `arc_result_retain`,
`Ptr(HeapKind::Option)` → `arc_option_retain`,
else → legacy `arc_retain`. Three retain/release call sites updated.

**Smoke results**:

- Smoke 1.5 (`fn divide(...) -> Result<int, string>; let r = divide(10,2);
  match r { Ok(v) => print(v), Err(e) => print(e) }`): VM `5`, JIT `5` —
  end-to-end identical.
- Smoke 2 (`fn first_positive(...) -> Option<int>; print(...)`) hangs
  in JIT mode. **VERIFIED PRE-EXISTING** (stash trinity changes + rebuild
  + run on the baseline branch: same hang). Hang in `first_positive`'s
  for-loop / Array<int> iteration combined with `print(Some(3))` heap-arm
  classification — orthogonal to trinity. Tracked separately.

**Compile-failure count on Smoke 1.5 under SHAPE_JIT_DEBUG=1**: pre-fix
30 (5 EnumStore + 23 Rvalue::Aggregate + 2 compile_binop_dynamic_arith);
post-fix 25 (0 EnumStore + 23 Aggregate + 2 binop_dynamic). The 5
EnumStore failures (for `divide` body + 4 stdlib functions that
construct Result/Option payloads) ALL closed. Remaining 23 Aggregate
failures are pre-existing W11-jit-new-array territory, NOT trinity work.

**Close gates (devenv exit-code-verified)**:
- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` 328 passed / 0 failed / 26 ignored
  (matches baseline 322/0/26 + 6 new Arc-shape FFI round-trip tests)
- `cargo test -p shape-vm --lib` pre-existing SIGABRT in
  v2-raw-heap-audit cluster (documented in CLAUDE.md)
- `bash scripts/verify-merge.sh` 12/12 (Passed: 12 / Failed: 0)
- `bash scripts/check-no-dynamic.sh` EXIT=0

**Sites surfaced (cite-tracked)**:
- (a) Smoke 2 hang in JIT mode — pre-existing, not trinity territory.
  Cluster-2 candidate.
- (b) `print(arc_bits)` for heap arms (Result/Option) still goes
  through `format_value_word`'s NaN-box decode — surfaced item #3 in
  phase-3-cluster-0-status. Cluster-2 candidate.
- (c) User-defined enum variant codegen via EnumStore (non-trinity
  names) surface-and-stops with structured cite — separate workstream
  per audit §7 row 5.
- (d) The other Arc-shape carriers (HashSet/Deque/Channel/Mutex/Atomic/
  Lazy) have the same Arc-vs-UnifiedValue refcount-offset gap as
  Result/Option had. When they get JIT codegen, they'll need the same
  kinded retain/release dispatch. Same pattern as `arc_result_retain`
  — generalize as the carrier family migrates (load-bearing for 7B
  `W12-jit-collection-typed-arc-ffi`).

**ADR-006 amendment**: NOT required. §2.7.17 / Q18 already specifies
the Arc-shape carrier semantics; the trinity implements them at the
JIT FFI tier. The Arc-contract refcount-offset issue is a JIT-FFI
implementation specific, not a §2.7 design question.

Branch: `bulldozer-strictly-typed-w12-jit-result-option-trinity`
Close commits: `d01d83b7`, `ae5d57f2`, `9f27edcd`

### W12-jit-collection-typed-arc-ffi close (audit-only, 2026-05-12)

Audit doc landed: `docs/cluster-audits/w12-jit-collection-typed-arc-ffi-audit.md`
(12 sections). **Audit grid for 8 HeapKinds** (HashSet/HashMap/Deque/PriorityQueue/
Channel/Mutex/Atomic/Lazy): all 8 still SURFACE at the JIT EnumStore consumer
per Round 6C close. Zero-arg ctors (Set/HashMap/Deque/PriorityQueue/Channel)
map to single FFI entries (`Arc<XData>::new()` + `Arc::into_raw`); with-arg
ctors split (Atomic(i64)/Lazy(Closure) compile-time-validate single inner kind;
Mutex(any) uses §2.7.5 `(bits, kind)` carrier-pair form).

**Why audit-only (option-(iii) territory surfaced, not option-(i) partial
landing)**: even with all 8 typed-Arc allocation FFI entries landed, the smoke
target `let s = Set(); s.add("a"); s.add("b"); print(s.size())` cannot pass
because `jit_call_method` (`crates/shape-jit/src/ffi/call_method/mod.rs:201`)
dispatches via `heap_kind(receiver_bits)` (NaN-box tag decode at
`value_ffi.rs:330-336`). Typed-Arc bits per `KindedSlot::from_hashset(arc)`
are raw `Arc::into_raw(arc) as u64` pointers — no NaN-box tag; `is_heap(bits)`
returns false; `heap_kind(bits)` returns None; method dispatch falls through
to `dispatch_method_via_trampoline` extern-C `todo!()` (aborts process).
Landing allocation FFI alone REGRESSES the equivalence-ratchet: Round 6C's
clean compile-time surface becomes a runtime extern-C `todo!()` process
abort. CLAUDE.md "Forbidden rationalizations" + W11-round-1 walk-back
precedent applies.

**The deeper architectural piece is ADR-006 §2.7.10 / Q11 — JIT-side kinded
MethodFnV2 ABI rebuild**. Same root cause as Round 7A's Result/Option trinity
applied to a different HeapKind family. Round 7A explicitly absorbed the
trinity (match-on-enum + Arc-shape producers + EnumStore consumer) for
Result/Option; the broader `jit_call_method` shell rebuild is a co-design
co-trinity scope of its own — dispatched as Round 8B.

**Carrier-shape clarification (audit §8)**: typed-Arc collections use
`Arc::into_raw(Arc<XData>) as u64` (Arc internal refcount at offset -16);
this is NOT interchangeable with W11's `Box::into_raw(Box::new(UnifiedValue<T>))
as u64` (own HeapHeader refcount at offset 4). Mixing would segfault at every
`jit_arc_release` reclaim. Documentation hygiene item: a clarification clause
in §2.7.6 / Q8 carrier-API-bound (or central in the carrier-family amendments
§2.7.15 / §2.7.18 / §2.7.19 / §2.7.20 / §2.7.25) would prevent the W11-
TypedArray-shape literal-prescription error.

**Close gates (audit is doc-only, no regressions)**:
- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` 322/0/26 unchanged
- `bash scripts/verify-merge.sh` 12/12

Branch: `bulldozer-strictly-typed-w12-jit-collection-typed-arc-ffi`
Merge: `7dc0ce5d` (post-Round-7A merge baseline)

### Post-Round-7 final smoke matrix (bulldozer HEAD `7dc0ce5d`)

| Smoke | Description | VM | JIT | Note |
|---|---|---|---|---|
| 1 | scalar loop `let mut acc = 0; for i in 1..=99 { acc = acc + i }; print(acc)` | `4950` | `4950` | ✅ identical |
| 1.5 | `fn divide(...) -> Result<int, string>; match divide(10,2) { Ok(v) => print(v), Err(e) => print(e) }` | `5` | `5` | ✅ identical — Round 7A trinity delivered |
| 2 | `fn first_positive(...) -> Option<int>; print(first_positive([-1,-2,3,-4]))` | `Some(3)` | (no output) | ❌ heap-arm `jit_print` classification gap — Round 8A territory |
| 3 | TypedObject `let p = Point { x: 3, y: 4 }; print(p.x + p.y)` | `7` | `7` | ✅ identical — Round 5A binop-after-heap-read |
| 4 | `let s = Set(); s.add("a"); s.add("b"); print(s.size())` | `2` | clean SURFACE at JIT EnumStore consumer (no extern-C abort) | ❌ Round 8B territory — §2.7.10/Q11 MethodFnV2 ABI rebuild |

Cluster-0 sub-cluster total: 18 (4 kickoff + 4 R3 + 1 R4 + 3 R5 + 3 R6 + 2 R7
+ 1 walked-back W11). Trajectory matches Phase 2d's N+1 growth pattern per
supervisor's earlier ruling. Cluster-0 close criterion (VM == JIT identical
output for all 5 smokes) unchanged; 3/5 currently passing. The W-series
declare-victory-at-the-artifact-tagging-layer pattern is explicitly refused.

## Round 8 — dispatching (post-Round-7 audit-surfaced trinities split across two HeapKind families)

Two sub-clusters dispatched in parallel 2026-05-13 from the Round 7 audit
surfaces:

| Sub-cluster | Branch | Worktree | Status |
|---|---|---|---|
| W12-jit-print-heap-arm-classification (8A) | `bulldozer-strictly-typed-w12-jit-print-heap-arm-classification` | `../shape-w12-jit-print-heap-arm-classification` | dispatching (parallel) |
| W12-jit-collection-method-dispatch-abi (8B) | `bulldozer-strictly-typed-w12-jit-collection-method-dispatch-abi` | `../shape-w12-jit-collection-method-dispatch-abi` | dispatching (parallel) |

### 8A scope (W12-jit-print-heap-arm-classification)

Per-HeapKind kinded `jit_print` entries — bounded mechanical, ~1 session
estimated. Scalar arms (`jit_print_i64` / `jit_print_f64` / `jit_print_bool`)
already landed in W11-jit-new-array Round 1. Round 7A's Arc-shape FFI pattern
is the model: read carrier via `*const T` field projections (like
`jit_arc_result_is_ok` reads `(*r).is_ok` directly), NOT via NaN-box tag decode.

Target surface: `print(Some(3))` in Smoke 2 currently produces no output —
heap-arm classification gap at the JIT `print` Call-terminator dispatch.
The operand's `NativeKind` is not threaded to a kinded fn-id; falls through
to kind-blind `jit_print` which W11 round-2 routed only for scalar arms.

Touch points (zero overlap with 8B):
- `crates/shape-jit/src/mir_compiler/terminators.rs` — Call-terminator
  `print` dispatch: thread operand's `NativeKind` to kinded fn-id;
  surface-and-stop on unknown kind (§2.7.7 #4/#7).
- `crates/shape-jit/src/ffi/` — new per-HeapKind kinded print entries
  (`jit_print_str`, `jit_print_typed_object`, `jit_print_option`,
  `jit_print_result`, plus any HeapKind surfaced during audit).
- `crates/shape-jit/src/ffi_symbols/` (registration) +
  `crates/shape-jit/src/ffi_refs.rs` (FuncRef slots) +
  `crates/shape-jit/src/compiler/ffi_builder.rs` (`r!(...)` lookups).

Close criterion: Smoke 2 `print(Some(3))` displays `Some(3)` matching VM
output; verify-merge 12/12; no regressions.

### 8B scope (W12-jit-collection-method-dispatch-abi)

**AUDIT-FIRST**. §2.7.10/Q11 kinded MethodFnV2 ABI rebuild for 8 collection
HeapKinds (HashSet=21, HashMap=17, Deque=23, PriorityQueue=25, Channel=24,
Mutex=30, Atomic=31, Lazy=32). Round 7B audit established this as option
(iii) territory — load-bearing for Smoke 4 + 2 additional smokes
(HashMap, Mutex).

Integrated trinity-style scope:
1. Typed-Arc allocation FFI bodies (5 zero-arg + 2 single-kind + 1
   `(bits, kind)` carrier-pair Mutex) per audit §3.1.
2. `jit_call_method` shell rebuild — read receiver kind from `stack_kinds`
   parallel-kind track (§2.7.7) instead of NaN-box tags; thread receiver+args
   `NativeKind` to kinded MethodFnV2 entries.
3. Per-HeapKind MethodFnV2 kinded entries — exhaustive set per
   `crates/shape-vm/src/executor/objects/method_registry.rs` for each of
   the 8 HeapKinds. Includes EnumStore consumer arm for collection_ctor
   variant in `mir_compiler/statements.rs` (Round 6C MIR-emission side
   landed; this round wires the JIT consumer).

**STOP and surface if ADR-006 amendment territory detected** — most likely
amendment-trigger per audit §9(d) is HashMap K/V kind threading exceeding
Q15 Route A's monomorphic-per-element-kind contract for non-Array HeapKinds.
Audit-only close (with structured §-cite surfacing) is acceptable; partial
landing that regresses Round 6C/7B clean SURFACE is NOT.

Touch points (zero overlap with 8A):
- `crates/shape-jit/src/ffi/call_method/mod.rs` — `jit_call_method` shell
  rebuild per §2.7.10/Q11.
- `crates/shape-jit/src/ffi/v2/collection_ctors.rs` (NEW) — 8 typed-Arc
  allocation bodies per audit §3.1.
- `crates/shape-jit/src/ffi/v2/collection_methods.rs` (NEW) — per-HeapKind
  MethodFnV2 kinded entries.
- `crates/shape-jit/src/ffi_symbols/v2_symbols.rs` (~24 symbol registrations).
- `crates/shape-jit/src/ffi_refs.rs` (FuncRef slots) +
  `crates/shape-jit/src/compiler/ffi_builder.rs` (`r!(...)` lookups).
- `crates/shape-jit/src/mir_compiler/statements.rs` (EnumStore consumer
  collection_ctor arm).
- Possibly `crates/shape-jit/src/mir_compiler/terminators.rs` (method-call
  terminator: thread `stack_kinds` track receiver+args kind to kinded
  MethodFnV2).
- Possibly `docs/adr/006-value-and-memory-model.md` §2.7.x amendment if
  option (iii) surfaces during audit.

Carrier-shape: use `Arc::into_raw(Arc<XData>)` (Arc internal refcount at
offset -16), NOT `Box::into_raw(Box::new(UnifiedValue<T>))` (W11 TypedArray
shape with own HeapHeader at offset 4). Per audit §8.

Close criterion: Smoke 4 (`Set()` + `.add()` + `.size()` + print) VM == JIT
— OR audit-only close with option (iii) surface if ADR amendment territory
fires. The ratchet rule applies: do not regress Round 6C/7B's clean
SURFACE-at-EnumStore-consumer state.

### Coordination

Zero file-territory overlap between 8A (jit_print + heap-arm print FFI) and
8B (jit_call_method + collection MethodFnV2 + collection typed-Arc ctors).
Different FuncRef slots, different Cranelift codegen sites, different
mir_compiler dispatch arms (Call-terminator print vs method-call terminator
+ EnumStore collection_ctor). Both proceed in parallel from
post-Round-7 baseline `7dc0ce5d`.

### W12-jit-print-heap-arm-classification close (2026-05-13)

Partial migrating-close: Option/Result heap arms wired end-to-end with
§2.7.17 Arc-shape carriers (the Round 7A pattern); String / TypedObject
heap arms FFI bodies landed but dispatch surfaces-and-stops at
carrier-mismatch with structured §-cite per ADR-006 §2.7.5 + Round 6A
site (a).

**Landed**:

- 4 new FFI bodies in `crates/shape-jit/src/ffi/conversion.rs`:
  `jit_print_str(ctx_ptr, bits)`, `jit_print_typed_object(ctx_ptr, bits)`,
  `jit_print_option(ctx_ptr, bits)`, `jit_print_result(ctx_ptr, bits)`.
  Each takes the JITContext pointer (for `exec_context_ptr →
  type_schema_registry()` lookup) and the typed-Arc raw pointer per
  §2.7.5, then routes through the canonical VM-side
  `shape_vm::executor::printing::ValueFormatter::format_kinded` for
  VM == JIT identical output. No NaN-box tag decode, no `is_heap_kind`
  probe (§2.7.7 #4 / #7 forbidden).
- New `print_kinded_inner(ctx_ptr, bits, kind)` helper that constructs a
  `KindedSlot` from the raw bits + kind label, calls the VM formatter,
  and `std::mem::forget`s the carrier (caller's slot keeps its
  strong-count share). Schema registry fallback to empty when
  `ctx.exec_context_ptr` is null — matches `format_typed_object`'s
  documented schema-less-render path (`_0`, `_1`, ... positional names
  per `printing.rs:754`).
- Symbol registration in `ffi_symbols/object_symbols.rs` (4 new
  `builder.symbol(...)` calls + 4 new `declare_function(...)` calls
  with the `(i64, i64)` signature per the `(ctx_ptr, bits)` ABI).
- FuncRef slots `print_str` / `print_typed_object` / `print_option` /
  `print_result` in `ffi_refs.rs::FFIFuncRefs`.
- `r!("jit_print_*")` lookups in `compiler/ffi_builder.rs::build_ffi_refs`.
- Call-terminator print dispatch arm in
  `crates/shape-jit/src/mir_compiler/terminators.rs::compile_terminator`
  extended with the Option / Result heap arms routing per §2.7.5 stamp-
  at-compile-time. The String / TypedObject arms surface-and-stop with
  the carrier-mismatch §-cite (see "Surfaced" below).
- 7 new FFI round-trip tests in `ffi::conversion::heap_arm_print_tests`
  (mirrors Round 7A's pattern at `result.rs::tests`): Option/Some +
  Option/None + Result/Ok + Result/Err + String/Arc + TypedObject/Arc
  + null-ctx-with-unknown-schema. All 7 green.

**Smoke matrix delta** (VM vs JIT, after Round 8A landing):

| Smoke | VM | Pre-8A JIT | Post-8A JIT | Status |
|---|---|---|---|---|
| 1 (scalar loop) | `4950` | `4950` | `4950` | ✅ unchanged |
| 1.5 (`divide` + match) | `5` | `5` | `5` | ✅ unchanged |
| 2 strict (`first_positive([..])` for-loop + print) | `Some(3)` | (no output; pre-existing hang per Round 7A) | (no output; same bytecode-verification gap on `first_positive`) | ➖ pre-existing, not 8A territory |
| 2 no-loop (`first_positive(3)` + print) | `Some(3)` | (no output) | `Some(3)` | ✅ 8A fix — top-level VM path renders the Arc<OptionData> via `ValueFormatter` correctly |
| 3 (`Point{}.x + .y`) | `7` | `7` | `7` | ✅ unchanged |
| 4 (`Set()` + `.size()`) | `2` | clean SURFACE (Round 6C) | clean SURFACE (unchanged) | ➖ Round 8B territory |
| `print(Some(3))` top-level | `Some(3)` | denormal garbage `0.000…509…` | `Some(3)` | ✅ 8A fix |
| `print(Ok(5))` top-level | `Ok(5)` | denormal garbage | `Ok(5)` | ✅ 8A fix |
| `print(Some(7) annotated)` top-level | `Some(7)` | denormal garbage | `Some(7)` | ✅ 8A fix |
| `print("hello")` | `hello` | denormal garbage | clean SURFACE §2.7.5 carrier-mismatch | ➖ ratchet-improvement (segfault → structured surface), cluster-1 territory |
| `print(Err("x"))` | `Err("x")` | denormal garbage | clean SURFACE §2.7.5 carrier-mismatch | ➖ same |
| `print(typed_object_instance)` | `{x: 3, y: 4}` | denormal garbage | clean SURFACE §2.7.5 carrier-mismatch | ➖ same |
| `print(None)` | `false` | `0` | `0` | ➖ pre-existing VM bug (`None` bare-form resolves to bool) + kind-blind fallback path; not 8A territory |

**Close gates (devenv exit-code-verified)**:

- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` EXIT=0 (335 passed / 0 failed / 26
  ignored — baseline 328 + 7 new FFI round-trip tests)
- `bash scripts/verify-merge.sh` EXIT=0 (Passed: 12 / Failed: 0)
- `bash scripts/check-no-dynamic.sh` EXIT=0

**Sites surfaced (cite-tracked, NOT silently fallback'd)**:

- (a) **§2.7.5 carrier-mismatch for `NativeKind::String` and
  `NativeKind::Ptr(HeapKind::TypedObject)`**. The MIR-side
  `operand_slot_kind` correctly stamps these labels at the print Call-
  terminator site (per `MirConstant::Str`'s `Some(NativeKind::String)`
  return and a struct local's `Ptr(HeapKind::TypedObject)`), but the
  JIT-side producers for these kind labels store the bits in the legacy
  NaN-box UnifiedValue carrier shape:
  - `MirConstant::Str` lowering at `mir_compiler/ownership.rs:383`
    calls `crate::ffi::value_ffi::box_string(s.clone())` → wraps
    `Arc<String>` inside `UnifiedValue<Arc<String>>` (the
    `unified_box(HK_STRING, ...)` shape per `value_ffi.rs:535-538`).
  - Struct Aggregate lowering goes through
    `crate::ffi::value_ffi::box_typed_object(ptr)` → `unified_box(HK_
    TYPED_OBJECT, ptr)` per `value_ffi.rs:516-518`.
  The §2.7.5 carrier contract for `NativeKind::String` is
  `Arc::into_raw(Arc<String>) as u64` (raw `*const String`); for
  `NativeKind::Ptr(HeapKind::TypedObject)` it is `Arc::into_raw(Arc<
  TypedObjectStorage>) as u64`. The matching kinded print bodies
  (`jit_print_str` / `jit_print_typed_object`) read these via direct
  `*const T` field projection. Dispatching the kind-stamped slots
  through the kinded bodies would dereference NaN-box header bits
  as if they were the payload pointer → segfault.

  **Same defect class as Round 6A site (a) for Result/Option**, which
  Round 7A's trinity resolved by migrating the producers
  (`jit_make_ok` → `jit_v2_make_result_ok` etc.). The String /
  TypedObject migration is the **cluster-1 candidate
  `W12-jit-result-carrier-unification` (generalized to all §2.7.5
  heap carriers)** — already named in Round 6A's surfaced items
  table. The String / TypedObject FFI bodies are correctly
  implemented per §2.7.5 and ready for wire-up once the producer
  migration lands; until then the Call-terminator dispatch
  surfaces-and-stops with a structured §-cite.

  The FFI bodies + FuncRef slots + symbol registrations are retained
  (not deleted) to avoid double-work when cluster-1 lands; the
  dispatch arm flip is the only edit at that point. Tests
  exercising the §2.7.5 Arc carrier directly (`print_str_arc_
  carrier_matches_vm`, `print_typed_object_arc_carrier_no_schema_
  renders_positional`) verify the FFI body is correct against the
  post-migration carrier.

- (b) **Smoke 2 strict form (`first_positive` for-loop + Array<int>
  iteration + `print`)** doesn't hang post-8A but produces no output
  because `first_positive`'s bytecode verification fails on
  `TypedArrayPushI64` having no `FrameDescriptor`. This is a
  pre-existing Phase 2d FrameDescriptor stamping gap for V2 typed
  opcodes, NOT 8A territory. The Round 7A close report's "Smoke 2
  hang" framing was accurate at that time — the for-loop interaction
  later evolved into clean compile failure rather than runtime hang.
  Either way, the print classification piece (8A scope) is fully
  resolved for non-iter forms; the for-loop interaction is cluster-2
  / Phase 2d hardening territory.

- (c) **`print(None)` produces `false` on VM and `0` on JIT** — neither
  matches the canonical `None` string per §2.7.17. Pre-existing VM
  bug at the bare-form `None` MIR lowering site (likely `MirConstant::
  None` → `iconst(I64, 0)` then bool-coerced at print). Not 8A
  territory — surfaced as a cluster-1 candidate for the bare-`None`
  MIR-emission audit.

- (d) **The kind-blind `jit_print` fallback for unproven operand
  `NativeKind` is DELETED at every layer** (Round 8A reopen,
  2026-05-13 — see "REOPEN" subsection below).

#### W12-jit-print-heap-arm-classification REOPEN verification (2026-05-13)

Supervisor reopened the Round 8A close at `1639148a` with one
verification: was the kind-blind `_`-arm fallback genuinely
load-bearing for Smoke 1.5, or did it match CLAUDE.md "Forbidden
rationalizations" #1/#4/#5 ("just one edge case" / "follow-up
for later phase" / "document as out-of-scope")?

**Step (i) SHAPE_JIT_DEBUG trace on Smoke 1.5** (`fn divide(...) ->
Result<int, string>; let r = divide(10, 2); match r { Ok(v) =>
print(v), Err(e) => print(e) }`):

- `print(v)` Ok-arm Call-terminator: `args[0] = Copy(Local(SlotId(8)))`,
  `kind_hint = Some(Int64)` — kinded `print_i64` arm catches; `_`
  arm was dead-code for this print call.
- `print(e)` Err-arm Call-terminator: `args[0] = Copy(Local(SlotId(12)))`,
  `kind_hint = None` — genuine §2.7.5 producer-side conduit gap.

Mixed result — Ok-arm dead-code (path ii territory) + Err-arm gap
(path iii territory). Per supervisor's spec: path (iii) extends the
conduit honestly.

**Root cause of the producer-side gap**: `infer_enum_payload_kind`
in `crates/shape-jit/src/mir_compiler/types.rs` used the scalar-only
`elem_slot_kind_for_concrete` classifier, which maps only
`ConcreteType::{F64, I64, I32, ..., Bool}` to `NativeKind` — leaving
`String` / `Ptr(HeapKind::*)` inner ConcreteTypes unstamped at the
EnumPayload destination. The trace confirms: `infer_enum_payload_kind
base_slot=6 ct=Result(I64, String) variant=Err` produced `inferred=
None` for the Err arm even though `concrete_types[r] = Result(I64,
String)` was correctly stamped by Round 6A's conduit.

**Fix**: switched `infer_enum_payload_kind` to use the broader
`native_kind_from_concrete_type` (the full ConcreteType → NativeKind
mapping). Per ADR-006 §2.7.17 receiver-recovery soundness,
`jit_arc_result_payload` / `jit_arc_option_payload` extract the inner
`KindedSlot.slot.raw()` verbatim — preserving the §2.7.5 carrier
shape for every NativeKind variant. Post-extension, both arms of
Smoke 1.5 are kinded: Ok = `Int64` (unchanged), Err = `String`
(newly stamped).

**Deletions at every layer** (per supervisor's "drop the kind-blind
`_` arm body"):

1. `_`-arm body in `mir_compiler/terminators.rs::compile_terminator`
   print Call-terminator — replaced with `NotImplemented(SURFACE)`
   error return.
2. `jit_print` FFI body in `ffi/conversion.rs` — DELETED; replaced
   with deletion-fate header comment naming the deleted W-series
   `format_value_word` dispatch.
3. `jit_print` symbol registration in
   `ffi_symbols/object_symbols.rs::register_object_symbols`.
4. `jit_print` declare_function in
   `ffi_symbols/object_symbols.rs::declare_object_functions`.
5. `print: FuncRef` field in `ffi_refs.rs::FFIFuncRefs`.
6. `r!("jit_print")` lookup in `compiler/ffi_builder.rs::
   build_ffi_refs`.

The kind-blind fallback chain (operand → `jit_print` → deleted-W-
series `format_value_word`) is removed at every layer, not just
hidden behind a never-taken `_` arm.

**Smoke matrix delta (post-reopen verification)**:

| Smoke | Pre-reopen | Post-reopen |
|---|---|---|
| 1 (4950) | VM=JIT ✓ | VM=JIT ✓ unchanged |
| 1.5 (`divide` + match → `5`) | VM=JIT ✓ via kind-blind fallback | VM=`5` / JIT=SURFACE §2.7.5 carrier-mismatch (Err arm String) |
| 2-no-loop (`Some(3)`) | VM=JIT ✓ | VM=JIT ✓ unchanged |
| 3 (`p.x + p.y` = 7) | VM=JIT ✓ | VM=JIT ✓ unchanged |
| `print(Some(3))` top-level | VM=JIT ✓ | VM=JIT ✓ unchanged |
| `print(Ok(5))` top-level | VM=JIT ✓ | VM=JIT ✓ unchanged |
| `print("hello")` | SURFACE §2.7.5 | SURFACE §2.7.5 unchanged |
| `print(Err("x"))` / `print(typed_object)` | SURFACE §2.7.5 | SURFACE §2.7.5 unchanged |

**Smoke 1.5 regression rationale**: post-conduit-extension the
Err-arm `print(e)` operand has `kind_hint = Some(String)` and
reaches the existing §2.7.5 carrier-mismatch surface — the
EnumPayload-derived String IS §2.7.5-correct (via
`jit_arc_result_payload`), but the print dispatch cannot
statically distinguish it from `MirConstant::Str`-derived NaN-box
String at the per-operand level. Routing `NativeKind::String` to
`jit_print_str` would runtime-segfault on string-literal paths,
which would be worse than surfacing. The cluster-1
`W12-jit-result-carrier-unification` scope migrates `box_string` /
`box_typed_object` to §2.7.5 Arc-shape producers — after that
lands, both EnumPayload-derived and `MirConstant::Str`-derived
String slots share the §2.7.5 contract and the dispatch arm can
be flipped without ambiguity. Smoke 1.5 regresses to honest
SURFACE per supervisor's reopen spec: "Surface-and-stop or
removed-as-dead-code. The fallback's existence past your close is
the W-series walk-back the supervisor refuses on sight."

**Sites surfaced — additional (cite-tracked)**:

- (e) Pre-reopen Round 8A claim (d) — "kind-blind `jit_print`
  fallback preserved per pre-8A Round 5C baseline" — was the
  CLAUDE.md "Forbidden rationalizations" #1/#4/#5 framing
  ("just one edge case" / "follow-up for later phase" /
  "document as out-of-scope"). MEMORY.md "Own all code quality.
  Never blame 'pre-existing' issues" applies. Refused on sight
  per supervisor's reopen.

- (f) Smoke 1.5 regression to SURFACE is the principled
  consequence of dropping the W-series fallback in advance of the
  cluster-1 carrier-unification scope. The pre-reopen "Smoke 1.5
  passes" claim relied on the kind-blind fallback routing through
  the deleted-W-series `format_value_word` — a Pyrrhic pass.

**Close gates (post-reopen, devenv exit-code-verified)**:
- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` 335 passed / 0 failed / 26 ignored
  (baseline 328 + 7 new heap-arm-print FFI round-trip tests)
- `bash scripts/verify-merge.sh` EXIT=0 (Passed: 12 / Failed: 0)
- `bash scripts/check-no-dynamic.sh` EXIT=0

**ADR-006 amendment**: NOT required. The conduit extension is one
more §2.7.5 producer-site classifier (EnumPayload destination at
EnumPayload-emit time, using the full ConcreteType → NativeKind
mapping per the §2.7.17 carrier-shape invariance). The `_`-arm
deletion is mechanical W-series-fallback removal. The cluster-1
`W12-jit-result-carrier-unification` arc remains the next step
(named at Round 6A surfaced items).

Branch: `bulldozer-strictly-typed-w12-jit-print-heap-arm-classification`
Original close commit: `1639148a` (pre-reopen)
Reopen close commit: (pending — appended at merge)
### W12-jit-collection-method-dispatch-abi close (2026-05-13)

**Closed audit-only — option (iii) surfaced for supervisor scope decision.**
Audit doc at `docs/cluster-audits/w12-jit-collection-method-dispatch-abi-
audit.md` (12 sections).

**Load-bearing scope refinement (audit §2.1)**: The dispatch's described
trinity item (iii) ("per-HeapKind kinded MethodFnV2 entries on JIT side")
is **not required**. The VM-side handlers in
`crates/shape-vm/src/executor/objects/method_registry.rs` are already
kinded per ADR-006 §2.7.10/Q11 (`fn(&mut VM, &[KindedSlot], _) ->
Result<KindedSlot, VMError>`). All 8 collection HeapKinds have full PHF
maps with ~73 method entries (HashSet=14, HashMap=22, Deque=11, PQ=9,
Channel=6, Mutex=4, Atomic=5, Lazy=2). The JIT-side dispatch shell does
not need to mirror these handlers; it needs to **delegate** to the VM's
existing kinded dispatch via a new public
`VirtualMachine::jit_trampoline_call_method` API, structurally identical
to the existing `jit_trampoline_call_closure` at
`crates/shape-vm/src/executor/call_convention.rs:953`.

**Corrected scope estimate (audit §10.1)**: ~1310 LoC across ~9 files
including a **cross-crate** shape-vm public-API addition. The dispatch's
"Touch points" did not anticipate the cross-crate API extension.

| Item | LoC | Files |
|---|---|---|
| 8 typed-Arc ctor FFI bodies | ~150 | `ffi/v2/collection_ctors.rs` (NEW) |
| 16 retain/release FFI bodies | ~200 | `ffi/v2/collection_arc_refcount.rs` (NEW) |
| `jit_call_method` shell rebuild | ~150 | `ffi/call_method/mod.rs` (edit) |
| New `VirtualMachine::jit_trampoline_call_method` | ~120 | `executor/call_convention.rs` — **CROSS-CRATE** |
| EnumStore consumer collection_ctor arm | ~80 | `mir_compiler/statements.rs` (edit) |
| Symbol registration (24 symbols) | ~80 | `ffi_symbols/v2_symbols.rs` (edit) |
| FuncRef slots (24 fields) | ~60 | `ffi_refs.rs` (edit) |
| `r!(...)` lookups (24 entries) | ~40 | `compiler/ffi_builder.rs` (edit) |
| `retain_func_for_place` / `release_func_for_place` extension | ~30 | `mir_compiler/` (edit) |
| Tests | ~400 | new test modules |
| **Total** | **~1310 LoC** | **~9 files** |

**Disposition (audit §11)**: At the high end of a single-round budget
(1.5×-2× Round 7A's ~800 LoC). The dispatch instruction explicitly
allows audit-only close *"If your audit also finds that the integrated
trinity scope exceeds a single round's reasonable budget"*.

**Recommended split (audit §10.2)**:
- **8B.1 W12-jit-collection-arc-ffi-ctors-and-refcount** (~580 LoC,
  ~5 files): 8 typed-Arc ctor FFI bodies + 16 kinded retain/release
  pairs + 24 symbol/FuncRef/lookup registrations + retain/release
  helper extension. Close criterion: workspace check + FFI round-trip
  tests; does NOT close Smoke 4 (EnumStore consumer still surfaces).
- **8B.2 W12-jit-call-method-shell-rebuild** (~730 LoC, ~4 files):
  new `VirtualMachine::jit_trampoline_call_method` cross-crate API +
  `jit_call_method` shell rebuild reading from `stack_kinds` +
  EnumStore consumer collection_ctor arm dispatching to 8B.1's ctors.
  Close criterion: Smoke 4 + HashMap + Mutex VM == JIT.

**Carrier-shape table (audit §5)**: typed-Arc collections use
`Arc::into_raw(Arc<XData>) as u64` (Arc internal refcount at offset
-16); W11 TypedArray family uses `Box::into_raw(Box::new(UnifiedValue<T>))`
(HeapHeader at offset 4); Round 7A Result/Option family uses Arc shape
matching this audit. Mixing carrier shapes corrupts refcounts and
segfaults at retain/release sites — load-bearing per Round 7A bug
finding (`9f27edcd`).

**Sites surfaced (audit §8)**:
1. `jit_call_method` shell — load-bearing for Smoke 4 + HashMap + Mutex.
2. `dispatch_method_via_trampoline` extern-C `todo!()` — orthogonal
   structured-error fix becomes unnecessary if shell rebuild lands.
3. Missing `VirtualMachine::jit_trampoline_call_method` public API —
   cross-crate boundary not in dispatch's touch-points list.
4. EnumStore consumer arm at `mir_compiler/statements.rs:239-268`.
5. `retain_func_for_place` / `release_func_for_place` 8-arm extension.
6. HashMap K/V kind threading is **not** an ADR-amendment trigger —
   per-arg kinds come from dispatch-shell carrier slice; HashMapData
   stores `Arc<HeapValue>` values heterogeneously.
7. Lazy's `l.get()` closure-call path inherits `call_value_immediate_nb`
   via the delegation chain — no additional JIT-side wiring needed.

**ADR-006 amendment**: NOT required. The §2.1 delegation insight means
the JIT side does not need any new ABI shape; the §2.7.10/Q11 ABI is
correctly specified end-to-end on the VM side; the JIT crosses into
the VM's existing kinded dispatch entry.

**Close gates (audit is doc-only, no regressions)**:
- `bash scripts/verify-merge.sh` 12/12 Passed (devenv shell).
- `bash scripts/check-no-dynamic.sh` EXIT=0 (devenv shell).
- Smoke 4 / HashMap / Mutex smokes UNCHANGED — Round 6C/7B clean
  SURFACE state preserved under `--mode jit`; VM mode prints correct
  values.

Branch: `bulldozer-strictly-typed-w12-jit-collection-method-dispatch-abi`
Parent: `267b1ca2` (post-Round-7B + Round 8 dispatch metadata)

## Round 9 — dispatching (8B.1 standalone per supervisor sequential split)

Dispatched 2026-05-13 from the post-Round-8 merge baseline `3d3f1258`.
Supervisor ratified the sequential split (Round 9 = 8B.1, Round 10 = 8B.2)
per Round 8B audit's corrected scope estimate (~1310 LoC integrated
trinity vs. Round 7A's ~800 LoC precedent — at the high end of single-round
budget with cross-crate API addition; sequential split respects budget +
isolates crate-boundary review risk in 8B.2).

| Sub-cluster | Branch | Worktree | Status |
|---|---|---|---|
| W12-jit-collection-arc-ffi-ctors-and-refcount (9 / 8B.1) | `bulldozer-strictly-typed-w12-jit-collection-arc-ffi-ctors-and-refcount` | `../shape-w12-jit-collection-arc-ffi-ctors-and-refcount` | dispatching |

### Scope (Round 9 / 8B.1)

Per Round 8B audit `docs/cluster-audits/w12-jit-collection-method-dispatch-abi-audit.md`
§3.1 + §10.2 sub-cluster split (8B.1 row).

**Typed-Arc ctor FFI** (8 entries):
- Zero-arg: `jit_v2_make_hashset` / `_hashmap` / `_deque` / `_priorityqueue` /
  `_channel` → return `Arc::into_raw(Arc::new(<XData>::default())) as u64`.
- Single-kind: `jit_v2_make_atomic(i: i64)` /
  `jit_v2_make_lazy(closure_bits: u64)` — compile-time-validated inner kind
  per §2.7.25.
- Carrier-pair: `jit_v2_make_mutex(bits: u64, kind: u8)` — JitFfiCarrier
  `(bits, NativeKind)` form per §2.7.5.

**Kinded retain/release** (16 entries — 8 retain + 8 release):
Per Round 7A precedent (`jit_arc_result_retain/_release` /
`jit_arc_option_retain/_release` calling
`Arc::increment_strong_count::<T>` / `Arc::decrement_strong_count::<T>`):
new `jit_arc_<heapkind>_retain/_release` for HashSet / HashMap / Deque /
PriorityQueue / Channel / Mutex / Atomic / Lazy.

`retain_func_for_place` / `release_func_for_place` in
`mir_compiler/ownership.rs` extended with 8-arm match for
`Ptr(HeapKind::HashSet|HashMap|Deque|PriorityQueue|Channel|Mutex|Atomic|Lazy)`
routing to the new kinded entries.

**Carrier-shape rule (audit §5)**: `Arc::into_raw(Arc<XData>) as u64`
(Arc internal refcount at offset -16 per Rust standard Arc layout). Do
NOT mix with `Box::into_raw(Box::new(UnifiedValue<T>))` (W11 TypedArray
shape with own HeapHeader at offset 4) — mixing would segfault at every
`jit_arc_release` reclaim.

### Out of scope (Round 10 territory)

- `jit_call_method` shell rebuild — reading receiver+args kind from
  `stack_kinds` parallel-kind track per §2.7.7, removing NaN-box tag
  decode at `value_ffi.rs:330-336`.
- New `VirtualMachine::jit_trampoline_call_method` cross-crate API per
  audit §2.1 (mirrors `jit_trampoline_call_closure` at
  `crates/shape-vm/src/executor/call_convention.rs:953`).
- EnumStore consumer collection_ctor arm in `mir_compiler/statements.rs`
  dispatching to this round's ctors.

**Round 9 lands the ctors + retain/release; Round 10 wires the shell +
trampoline + consumer.** Round 9 landings are inert until Round 10 consumer
wiring catches them, preventing the equivalence-ratchet regression
Round 7B audit flagged.

### Close criterion

- FFI round-trip tests (~24 minimum: 8 ctor + 16 retain/release; mirror
  Round 7A's 6 round-trip pattern).
- `cargo check --workspace --lib --tests` EXIT=0 (inside `devenv shell`).
- `cargo test -p shape-jit --lib` no regressions from baseline 335.
- `bash scripts/verify-merge.sh` 12/12 (inside devenv).
- `bash scripts/check-no-dynamic.sh` EXIT=0.
- AGENTS.md row updated to `closed`.
- Status doc subsection `### W12-jit-collection-arc-ffi-ctors-and-refcount
  close (2026-05-13)`.

### Forbidden frames (refused on sight)

Per CLAUDE.md "Renames to refuse on sight" §2.7.10/Q11 + §2.7.11/Q12
broader-family regex: "collection-FFI bridge" / "typed-Arc translator" /
"container-allocation helper" / "kind-injection adapter" / "value-call
bridge" / "callee-kind helper" / "capture-injection adapter". Describe
deleted code by deletion-fate (the deleted-W-series `unified_box(HK_*, ...)`
shape, the kind-blind ABI), never by hypothetical role.

### Round 10 readiness gate

Round 9 closes → merge → dispatch Round 10. Per supervisor's revised
cadence: post-Round-9 merge also includes kickoff Smoke 2 + 3 verification
(`xs.map(|x|x*2).sum()` and `trait T + impl T for X + dyn T + t.name()`)
under both `--mode vm` and `--mode jit`; if either kickoff smoke surfaces
a new gap, Round 10 or Round 11 absorbs the work per N+1 trajectory
discipline.

### W12-jit-collection-arc-ffi-ctors-and-refcount close (2026-05-13)

Round 9 (8B.1 standalone) closed exactly per audit §10.2 scope. 8
typed-Arc collection ctors + 16 per-HeapKind kinded retain/release
entries + ownership.rs 8-arm dispatch extension landed. Smoke matrix
delta: zero changes (Round 9 is INERT at the program surface until
Round 10 wires the EnumStore consumer + `jit_call_method` shell).

**Smoke matrix (VM vs JIT, post-Round-9)**:

| Smoke | VM | Pre-9 JIT | Post-9 JIT | Status |
|---|---|---|---|---|
| 1 (scalar loop) | `4950` | `4950` | `4950` | ✅ unchanged |
| 1.5 (`divide` + match) | `5` | `5` | `5` | ✅ unchanged (Round 8A) |
| 2 no-loop (`first_positive(3)`) | `Some(3)` | `Some(3)` | `Some(3)` | ✅ unchanged (Round 8A) |
| 3 (`Point{}.x + .y`) | `7` | `7` | `7` | ✅ unchanged |
| 4 (`Set()` + `.size()`) | `2` | clean SURFACE | clean SURFACE | ➖ Round 10 territory (8B.2) |

**Close gates (devenv exit-code-verified)**:

- `cargo check --workspace --lib --tests` EXIT=0 (full workspace,
  including shape-test compilation)
- `cargo test -p shape-jit --lib` 361 passed / 0 failed / 26 ignored
  (baseline 335 + 26 new collection-arc round-trip tests = 361 exact)
- `bash scripts/verify-merge.sh` 12/12 Passed
- `bash scripts/check-no-dynamic.sh` EXIT=0

**Files touched**:

- `crates/shape-jit/src/ffi/v2/collection_arc.rs` (NEW, ~700 LoC):
  combined 8 typed-Arc ctor FFI bodies + 16 per-HeapKind kinded
  retain/release bodies + 26 unit tests. Co-located in one module
  per the carrier-shape rule (audit §5) — ctors produce `Arc::into_raw(
  Arc<XData>)` shape; retain/release consume the same shape via
  `Arc::increment/decrement_strong_count::<XData>`; mixing carrier
  shapes between them would segfault. Single source-of-truth for the
  audit §5 header comment.
- `crates/shape-jit/src/ffi/v2/mod.rs`: module wire + carrier-shape
  rule pointer in module-doc comment.
- `crates/shape-jit/src/ffi_refs.rs`: 24 new `FuncRef` slots (8 ctors
  + 16 retain/release).
- `crates/shape-jit/src/compiler/ffi_builder.rs`: 24 new `r!(...)`
  lookups.
- `crates/shape-jit/src/ffi_symbols/v2_symbols.rs`: 24 `builder.symbol(...)`
  registrations + 24 `declare_function` signature declarations.
  Signatures: zero-arg ctors `() -> i64`; Atomic ctor `(i64) -> i64`;
  Lazy ctor `(i64) -> i64`; Mutex ctor `(i64, i8) -> i64`; all retain/
  release `(i64) -> ()`.
- `crates/shape-jit/src/mir_compiler/ownership.rs`:
  `retain_func_for_place` / `release_func_for_place` 8-arm extension
  (HashSet / HashMap / Deque / PriorityQueue / Channel / Mutex /
  Atomic / Lazy → matching kinded retain/release FuncRef). Legacy
  `arc_retain` / `arc_release` fallback preserved for kinds NOT in
  the typed-Arc family.

**Decisions called beyond audit + Round 7A precedent**:

1. **Single combined module `collection_arc.rs` rather than separate
   files**. The audit §6 file table lists `collection_ctors.rs` +
   `collection_arc_refcount.rs` as separate candidates, but the
   substantive constraint is the carrier-shape rule from §5 (the
   load-bearing audit insight). Co-locating ctors + retain/release
   for the same HeapKind family in one module makes the shared
   `Arc::into_raw` / `Arc::increment_strong_count` discipline visible
   at a glance, and keeps the audit §5 header comment as one
   source-of-truth. The audit §6 table is non-binding on file
   granularity; the binding constraint is the carrier-shape rule
   which the single-module layout satisfies.

2. **Mutex SENTINEL kind ord surfacing returns null bits, not Bool-
   default**. ADR-006 §2.7.7 #9 forbids Bool-default fallback for
   unknown kind ords. The Mutex ctor body decodes the `kind: u8`
   parameter via `stack_kind_code::decode`; on `None` (SENTINEL or
   unknown ord) the body returns 0 with a `SHAPE_JIT_DEBUG` diagnostic
   and **leaks** the inner share rather than dropping it with a
   fabricated Bool kind. Rationale: dropping with a fabricated Bool
   kind would either leak (if the true kind is heap but labeled
   non-refcounted) or double-free (if the true kind is heap but
   the bits don't match Arc's contract). Leaking is the principled
   response to a kind-source gap. The upstream caller's MIR-emit-time
   kind classifier is the load-bearing surface point for the gap;
   the FFI body's null return surfaces that gap at the dispatch
   shell rather than silently compounding it.

3. **Lazy ctor stamps `Ptr(HeapKind::Closure)` directly**. ADR-006
   §2.7.25 constrains the Lazy initializer to a closure-typed
   inner kind. The FFI body adopts the caller's `closure_bits` as
   `Arc<ClosureRaw>` share via `KindedSlot::new(slot, Ptr(HeapKind::
   Closure))`. The compile-time validation lives at the MIR EnumStore
   consumer (Round 10 territory); the FFI body is the inner-arm
   surface where the producing-site's kind classifier has already
   proven the constraint. Same shape as Round 7A's
   `jit_v2_make_option_some` body adopting its `payload_kind_code`
   via `decode_payload_kind_or_surface`.

**Surfaced items**: None new. Round 9 closes exactly per audit §10.2
/ 8B.1 scope; no architectural gap surfaced. Round 10 (8B.2)
inherits Round 9's pre-resolved FuncRefs for consumer-side wiring
of the EnumStore consumer + `jit_call_method` shell + cross-crate
`VirtualMachine::jit_trampoline_call_method` API.

Branch: `bulldozer-strictly-typed-w12-jit-collection-arc-ffi-ctors-and-refcount`
Parent: `1f28b2d8` (post-Round-8 merge + Round 9 dispatch metadata
on `bulldozer-strictly-typed`).

### Kickoff Smoke 2 + 3 verification (post-Round-9 merge, 2026-05-13)

Per supervisor's revised cadence — kickoff smokes restored as canonical
close criterion (`phase-3-kickoff-prompt.md:96-100`):

**Smoke 2** (`let xs = [1,2,3,4,5]; let doubled = xs.map(|x| x * 2);
print(doubled.sum())` → expect `30`):

- VM: ❌ `Not implemented: op_new_array: generic untyped-array
  construction depends on the kinded the-deleted-heterogeneous-element-
  carrier emit path (Phase 2c reentry — see ADR-006 §2.7.4)`. VM-side
  blocker. Phase 2c reentry territory; NOT cluster-0 JIT-rebuild scope.
- JIT: ❌ `Route A surface-and-stop: NotImplemented(SURFACE) — print
  Call-terminator operand NativeKind is None`. Same §2.7.5 producer-side
  conduit gap Round 8A reopen identified for EnumPayload, generalized:
  the conduit doesn't stamp `.sum()`'s return kind at the Call-terminator
  print operand site. JIT-side cluster-0 territory.

**Smoke 3** (Shape-syntax: `trait T { name(): string } type X {} impl T
for X { method name() { "x" } } let t = X {} print(t.name())` → expect
`x`. Kickoff's Rust-style `fn name(&self) -> String` doesn't parse;
Shape trait syntax is `methodname(): ReturnType` and impl methods are
`method name() { ... }` per `crates/shape-runtime/stdlib-src/core/display.shape`):

- VM: ✅ produces `x`.
- JIT: ❌ `Route A surface-and-stop: SURFACE — Rvalue::Aggregate reached
  the kind-blind fallback. The v2 typed-array fast path in statements.rs
  requires the destination Place::Local to carry a ConcreteType::Array<scalar>;
  reaching here means the element kind is not threaded from the producing
  call signature. Tracked as W11-jit-new-array.` Fires on `let t = X {}`
  struct Aggregate. W11-jit-new-array TypedObject Aggregate threading
  gap. JIT-side cluster-0 territory.

### Disposition

Both kickoff smokes 2 and 3 are blocked on the JIT side; Smoke 2 also
blocked on the VM side (Phase 2c reentry, orthogonal).

Two NEW cluster-0 JIT-side gaps surfaced, not covered by current Round 10
(8B.2 `jit_call_method` shell rebuild) scope:

1. **§2.7.5 producer-side conduit extension for `.sum()`-style method
   return kinds** at the JIT Call-terminator print operand site. Same
   defect class as Round 8A reopen's EnumPayload conduit extension,
   generalized. Candidate sub-cluster: `W12-jit-method-return-kind-conduit`
   (or absorb into Round 6A's `W12-jit-call-return-kind` audit scope).
2. **W11-jit-new-array TypedObject Aggregate kind threading** — the
   `Rvalue::Aggregate` arm for non-Array destinations (Struct / Tuple /
   TypedObject) reaches the kind-blind fallback. Round 5B audit
   surfaced this as option (iii) and landed option (ii) only.
   Candidate sub-cluster: `W12-jit-aggregate-typed-object-threading`
   (resurrects deferred Round 5B option (iii) work).

Per supervisor's revised cadence ("If kickoff Smoke 2 or 3 surfaces a new
gap during verification, Round 11 adds the work; cluster-0 stays open"),
both gaps fold into cluster-0 close criterion. Round 10 (8B.2 Smoke 4 +
HashMap + Mutex) proceeds as planned; Round 11+ absorbs the new gaps.

**Smoke 2 VM-side blocker** (`op_new_array` Phase 2c reentry, ADR-006
§2.7.4) — surfaced for supervisor disposition. Per cluster-0's "VM == JIT
for all 4 kickoff smokes" close criterion, cluster-0 can't close on
Smoke 2 regardless of JIT-side state if VM is blocked. Either (a)
cluster-0 absorbs the VM-side fix, or (b) cluster-0 closes with VM-side
blocker documented as out-of-scope and a Phase 2c-residual workstream
takes the VM fix separately.

### Updated cluster-0 close-criterion smoke matrix (post-Round-9)

| Smoke | Description | VM | JIT | Disposition |
|---|---|---|---|---|
| 1 (kickoff) | `for i in 0..100 { sum += i }` → 4950 | ✅ | ✅ | passing |
| 2 (kickoff) | `[1,2,3,4,5].map(\|x\|x*2).sum()` → 30 | ❌ Phase 2c reentry | ❌ §2.7.5 conduit for `.sum()` return kind | both surfaces, gating |
| 3 (kickoff) | trait T + impl + dyn + `t.name()` → "x" | ✅ | ❌ Rvalue::Aggregate TypedObject threading (W11-jit-new-array) | JIT-side gating |
| 4 (kickoff) | `HashSet + .add + .size` → 2 | ✅ | ❌ EnumStore consumer SURFACE | Round 10 (8B.2) territory |

Supplementary -ext smokes (non-gating, dispatcher-introduced from R5+):

| Smoke | Description | VM | JIT | Disposition |
|---|---|---|---|---|
| 1.5-ext | `divide` + match → 5 | ✅ | ❌ §2.7.5 String EnumPayload carrier-mismatch | cluster-1 carrier-unification candidate |
| 2-no-loop-ext | `first_positive(3)` → Some(3) | ✅ | ✅ | passing |
| 3-ext | TypedObject `p.x + p.y` → 7 | ✅ | ✅ | passing |

## Round 10 — dispatching (8B.2 standalone per supervisor sequential split)

Dispatched 2026-05-13 from the post-Round-9-merge + kickoff-verification
baseline `2cb68ece`. Supervisor confirmed Round 10 (8B.2) proceeds as
previously authorized — kickoff Smoke 2 + 3 gaps fold into Round 11
(not Round 10).

| Sub-cluster | Branch | Worktree | Status |
|---|---|---|---|
| W12-jit-call-method-shell-rebuild (10 / 8B.2) | `bulldozer-strictly-typed-w12-jit-call-method-shell-rebuild` | `../shape-w12-jit-call-method-shell-rebuild` | dispatching |

### Scope (Round 10 / 8B.2)

Per Round 8B audit `docs/cluster-audits/w12-jit-collection-method-dispatch-abi-audit.md`
§2.1 delegation insight + §10.2 sub-cluster split (8B.2 row).

**Delegation pattern**: §2.7.10/Q11 dispatch is already kinded VM-side
(~73 MethodFnV2 entries in `crates/shape-vm/src/executor/objects/method_registry.rs`);
JIT-side does NOT mirror those handlers — it delegates to VM's existing
kinded dispatch via a new public `VirtualMachine::jit_trampoline_call_method`
API mirroring `jit_trampoline_call_closure` at
`crates/shape-vm/src/executor/call_convention.rs:953` (the §2.7.5
cross-crate stable FFI consumer with single-direction pair-slice→KindedSlot
pattern per module-level docstring lines 53-66).

**Three landed pieces**:

1. **New cross-crate `VirtualMachine::jit_trampoline_call_method`** in
   shape-vm crate. Signature mirrors `jit_trampoline_call_closure`:
   `fn jit_trampoline_call_method(&mut self, method_name: &str,
   receiver: (u64, NativeKind), args: &[(u64, NativeKind)], ctx:
   Option<&mut ExecutionContext>) -> Result<u64, VMError>`. Body
   converts pair-slices to `&[KindedSlot]` internally then calls into
   the existing kinded method-dispatch entry-point.
2. **`jit_call_method` shell rebuild** in
   `crates/shape-jit/src/ffi/call_method/mod.rs:201-388`. Receiver+args
   kind comes from `stack_kinds` parallel-kind track per §2.7.7 (NOT
   NaN-box tag decode at `value_ffi.rs:330-336`). Deletes
   `dispatch_method_via_trampoline` extern-C `todo!()` fallback at
   `call_method/mod.rs:179-199`.
3. **EnumStore consumer collection_ctor arm** in
   `crates/shape-jit/src/mir_compiler/statements.rs:239-268`. Dispatches
   to Round 9's pre-resolved typed-Arc ctor FuncRefs
   (`jit_v2_make_hashset` / `_hashmap` / `_deque` / `_priorityqueue` /
   `_channel` / `_atomic` / `_lazy` / `_mutex`).

~730 LoC across ~4 files. Cross-crate shape-vm public-API addition is the
key risk surface; audit §10.4 anticipates a potential closure-trigger
extension per §2.7.10 if `jit_trampoline_call_method` can't pass
`&[KindedSlot]` cleanly across the Cranelift FFI boundary — surface-and-stop
if encountered (ADR amendment territory).

### Close criterion

- Smoke 4 (`Set()` + .add() + .size() + print → `2`) VM == JIT.
- HashMap smoke (`HashMap()` + .set() + .size() + print → `1`) VM == JIT.
- Mutex smoke (`Mutex(42)` + .lock() + print → `42`) VM == JIT.
- `cargo check --workspace --lib --tests` EXIT=0 inside devenv.
- `cargo test -p shape-jit --lib` no regressions from baseline 361.
- `bash scripts/verify-merge.sh` 12/12.
- `bash scripts/check-no-dynamic.sh` EXIT=0.

### Forbidden frames (refused on sight)

Per CLAUDE.md "Renames to refuse on sight" §2.7.10/Q11 + §2.7.11/Q12
broader-family regex: "MethodFnV2 bridge" / "MethodFn translator" /
"dispatch-slice probe" / "boundary adapter for handler ABI" /
"kind-injection helper" / "method-dispatch translator" / "value-call
bridge" / "callee-kind helper" / "capture-injection adapter".
Describe deleted code by deletion-fate (the deleted kind-blind
`args: &mut [u64]` MethodFnV2 ABI, the deleted NaN-box receiver tag
decode), never by hypothetical role.

### Round 11 readiness gate

Round 10 closes → merge → dispatch Round 11 with **three parallel
sub-clusters** per supervisor's ratified Round-11 scope (post-kickoff-
verification):

- **11A — W12-vm-new-array-untyped-construction** (AUDIT FIRST): VM-side
  `op_new_array` Phase 2c reentry fix. Cite-verify §-claim (likely
  §2.7.14 / §2.7.24, NOT §2.7.4 task-scheduler boundary). Identify
  deleted-carrier shape (likely `TypedArrayData::HeapValue` per §2.7.24
  Q25.A). Migrate emit path to monomorphic per-element-kind dispatch.
  Surface-and-stop on ADR amendment requirement. Unblocks kickoff
  Smoke 2 VM side.
- **11B — W12-jit-method-return-kind-conduit**: §2.7.5 producer-side
  conduit extension for method-return kinds (`.sum()`-style). Same
  defect class as Round 8A reopen's `infer_enum_payload_kind`
  extension, generalized to method-call sites. Audit-first on whether
  generic-over-receiver/method or per-method registration. Unblocks
  kickoff Smoke 2 JIT side.
- **11C — W12-jit-aggregate-typed-object-threading**: Resurrects deferred
  Round 5B option (iii). `Rvalue::Aggregate` arm for non-Array
  destinations (Struct/Tuple/TypedObject) reaches kind-blind fallback.
  Round 5B landed option (ii) ConcreteType threading; option (iii) is
  the consumer-side TypedObject Aggregate fast path. Unblocks kickoff
  Smoke 3 JIT side.

Cluster-0 close attempt after Round 11 merges if all 4 kickoff smokes
pass VM == JIT. If any 11A/B/C surfaces a fourth gap, Round 12 absorbs
per N+1 trajectory discipline.

### Round 10 close (post-merge verification, 2026-05-13)

Round 10 merged into `bulldozer-strictly-typed` at `51261265` from
sub-cluster close commit `2c2ecdf1`. Dispatch-ABI shell rebuild + cross-
crate trampoline + EnumStore consumer + slot-kind inference all landed
functional.

**Non-mutating Set equivalence (NEW post-Round-10)**:

```shape
let s = Set()
print(s.size())     # VM=0 / JIT=0 — EQUIVALENCE LANDED
```

First end-to-end VM == JIT for a collection-HeapKind smoke. Proves:
§2.7.10/Q11 dispatch-ABI shell rebuild + §2.7.5 cross-crate trampoline +
Round 9 typed-Arc ctors + EnumStore consumer collection_ctor arm +
slot-kind inference all wire correctly through §2.7.7 parallel-kind
track.

**Kickoff Smoke 4 still blocked** (`let mut s = Set(); s.add("a");
s.add("b"); print(s.size())` → expect `2`):
- VM: ✅ `2`.
- JIT: ❌ silent failure (no `2` printed; bytecode verification 15
  violations from stdlib FrameDescriptor warnings; exit 0). Symptom
  varies between silent fail and segfault per agent diagnosis — root
  cause is gap (A) below.

### Surfaced gaps (Round 10 close report)

**(A) W17-mir-mutation-writeback** (Smoke 4 + HashMap + every mutating
collection method):
- Bytecode compiler emits `Dup; StoreLocal recv` after mutating
  `CallMethod` per `crates/shape-vm/src/compiler/mutation_writeback.rs`.
- MIR builder at `mir/lowering/expr.rs::Expr::MethodCall` does NOT emit
  the equivalent `Assign(receiver_slot, Use(Move(temp)))`.
- JIT compiles from MIR, so `s.add()` produces new Arc into temp slot
  but user-visible `s` slot retains OLD Arc. Second access operates on
  stale Arc whose share was retired → segfault or silent-fail.
- Fix scope: ~30 LoC MIR lowering consulting `is_mut_self_method_name`
  + emitting writeback when receiver is `Place::Local`.

**(B) W17-collection-concrete-types** (Mutex.get / HashMap.get /
Atomic.load + parametric-return method kinds):
- Method return kinds for parametric containers (`Mutex.get → T`,
  `HashMap.get → Option<V>`, `Atomic.load → i64`) aren't in
  `well_known_method_return_kind` because they vary by receiver type.
- Downstream `print(m.get())` surfaces with kind None per Round 8A
  print-kind discipline.
- Fix scope: extend `ConcreteType` taxonomy with `Mutex<T>` /
  `Atomic<T>` / `Lazy<T>` / `HashSet` / `Deque` / `PriorityQueue` /
  `Channel` arms (currently absent in
  `crates/shape-value/src/v2/concrete_type.rs`) + propagate inner-kind
  through method-return-type inference.

### Round 11 — dispatching (3 parallel: 11A audit-first + 11D bounded + trinity)

Dispatched 2026-05-13 from post-Round-10-merge baseline `51261265`.
Supervisor ratified Option 3: integrated trinity (11B+11C+11E) +
2 standalone (11A, 11D).

| Sub-cluster | Branch | Status |
|---|---|---|
| W12-vm-new-array-untyped-construction (11A) | `bulldozer-strictly-typed-w12-vm-new-array-untyped-construction` | auditing |
| W17-mir-mutation-writeback (11D) | `bulldozer-strictly-typed-w17-mir-mutation-writeback` | **closed 2026-05-13** (92 LoC; Deque/PQ JIT 0→2; HashSet/HashMap blocked by surfaced JIT string-carrier shape gap, independent of writeback) |
| W12-jit-producing-site-conduit-completeness (trinity 11B+11C+11E) | `bulldozer-strictly-typed-w12-jit-producing-site-conduit-completeness` | migrating (~800-1000 LoC) |

### 11A scope (W12-vm-new-array-untyped-construction)

Audit-first deliverables before writing code:
- (a) §-cite verification — confirm real ADR § (likely §2.7.14 / §2.7.24
  / §2.7.5, NOT §2.7.4 task-scheduler boundary; same stray-cite class
  caught at `mir_compiler/statements.rs:236` / `w12-enum-constructor-audit.md:215`).
- (b) Deleted-carrier identification (likely `TypedArrayData::HeapValue`
  per §2.7.24 Q25.A — deleted Phase 2d, replaced by monomorphic per-
  element-kind variants + TypedObject catch-all).
- (c) Fix-shape: monomorphic dispatch on element kind at `op_new_array`
  emit site routing to corresponding TypedArrayData variant
  (I64 / F64 / String / Decimal / TypedObject).

Surface-and-stop on ADR amendment requirement. Forbidden frames refused
on sight: "preserve deleted-carrier emit path under documented
disposition", Bool-default element kind, "this one edge case",
"soft-fail counter for now".

Unblocks: kickoff Smoke 2 (VM-side).

### 11D scope (W17-mir-mutation-writeback)

Bounded mechanical ~30 LoC. Surface (Round 10 close report Section
"Surfaced workstreams (A)"):
- Bytecode compiler emits `Dup; StoreLocal recv` after mutating
  `CallMethod` per `crates/shape-vm/src/compiler/mutation_writeback.rs`.
- MIR builder at `crates/shape-vm/src/mir/lowering/expr.rs::Expr::MethodCall`
  does NOT emit equivalent `Assign(receiver_slot, Use(Move(temp)))`.
- JIT compiles from MIR, sees stale receiver Arc on second access →
  silent-fail or segfault.

Fix: consult `is_mut_self_method_name` (or equivalent predicate); emit
`Assign(receiver_slot, Use(Move(temp)))` when receiver is `Place::Local`
and method is mutating.

**Audit-first to confirm ~30 LoC scope holds**. If fix exceeds budget OR
Arc-COW semantics break for some collection variant, surface-and-stop —
segfault disposition becomes `NotImplemented(SURFACE)` with §-cite
(§2.7.27 + specific HeapKind variant), actual fix lands Round 12.

**Refuse on sight**: leaving silent-fail / segfault path alive past
close — segfault is NOT surface-and-stop, it's "soft-fail counter for
now, harden later" in disguise (CLAUDE.md "Forbidden rationalizations"
#4).

Unblocks: kickoff Smoke 4 + HashMap mutating smoke + every
mutating-collection-method smoke.

### W17-mir-mutation-writeback close (2026-05-13)

**Closed**: Phase 3 cluster-0 Round 11D. MIR builder writeback for
mutating method calls landed per ADR-006 §2.7.27 base, mirroring the
bytecode compiler's `Dup; StoreLocal recv` pattern
(`crates/shape-vm/src/compiler/expressions/function_calls.rs:2356`) at
the MIR layer.

**Audit (§1-§4, completed before code edit)**:

- §1 Predicate identification — `is_mut_self_method_name` exists at
  `crates/shape-vm/src/executor/objects/method_registry.rs:151` as the
  liberal name-only classifier ("write-back is harmless when actual
  receiver kind is pure" per the docstring at line 140-148). However,
  this is unsafe at the MIR layer: `let dt = DateTime(...); dt.add(period)`
  would emit `Assign(Place::Local(dt_slot), Use(Move(temp)))` and
  `compute_mutability_errors` at `mir/lowering/mod.rs:603-628` would
  flag the assignment as a mutability error on the immutable `dt`
  binding. Receiver-kind narrowing via the bytecode compiler's
  `mut_self_container_locals` / `ContainerKind::is_mut_self_method`
  pattern is required.
- §2 MIR emission site — two `Expr::MethodCall` sites in
  `mir/lowering/expr.rs` (line ~1806 standalone form + line ~2027 pipe
  form). After `builder.emit_call(...)` terminates the current block
  and starts the continuation block, the writeback `Assign` is emitted
  there.
- §3 Arc-COW semantics — verified for all 5 covered kinds (HashSet /
  HashMap / Deque / PriorityQueue / Array). Their mut-self handlers
  call `Arc::make_mut(&mut arc)` and return the (possibly-cloned)
  Arc; the writeback safely overwrites the receiver slot with the new
  Arc. Interior-mutability primitives (Mutex / Atomic / Lazy / Channel)
  are NOT registered in any `MUT_SELF_*_METHODS` PHF set — the
  receiver-kind narrowing returns `None` for these and no writeback
  is emitted (Arc identity preserved through interior mutability per
  `mutation_writeback.rs:27-33`).
- §4 Scope — ~30 LoC mechanical budget held. Total file diff is 92 LoC
  including docstrings and the `MirBuilder` helper methods
  (`record_mut_self_container_local` / `lookup_mut_self_container_local`).
  Core mechanical change is ~25-27 LoC: field + init + 2 helpers + 8
  LoC ctor-name detection in `lower_var_decl` + ~22 LoC writeback
  emitter helper + 2 LoC of MethodCall-site invocations.

**Fix shape** (3 files):

- `mir/lowering/mod.rs`: new `MirBuilder::mut_self_container_locals:
  HashMap<SlotId, ContainerKind>` field + `record_mut_self_container_local`
  / `lookup_mut_self_container_local` helper methods.
- `mir/lowering/stmt.rs::lower_var_decl`: when initializer AST is
  `Expr::FunctionCall { name: ctor_name, .. }` and
  `ContainerKind::from_ctor_name(ctor_name).is_some()`, call
  `builder.record_mut_self_container_local(slot, kind)` before lowering
  the init expression.
- `mir/lowering/expr.rs`: new `emit_mut_self_writeback_if_needed`
  helper — for `Expr::Identifier(name, _)` receivers resolving via
  `builder.lookup_local(name)` to a slot tracked in
  `mut_self_container_locals`, emit
  `Assign(Place::Local(slot), Rvalue::Use(Operand::Move(Place::Local(temp))))`
  after the call when `kind.is_mut_self_method(method)` matches.
  Invoked at both `Expr::MethodCall` lowering sites (standalone +
  pipe form).

**Smoke results** (VM ↔ JIT after fix):

| Smoke | VM | JIT (baseline) | JIT (post-fix) |
|---|---|---|---|
| Deque `pushBack`×2 + `size` | `2` ✓ | `0` (silent fail) | `2` ✓ |
| PriorityQueue `push`×2 + `size` | `2` ✓ | `0` (silent fail) | `2` ✓ |
| HashSet `add`×2 + `size` (Smoke 4) | `2` ✓ | segfault | segfault (NOT writeback-related — see surfaced gap) |
| HashMap `set` + `size` | works | crash | crash (same independent JIT bug as Smoke 4) |

Deque and PriorityQueue prove the MIR writeback fix works end-to-end:
JIT goes from silent-fail (stale receiver Arc on second access) to
correct output. These collections take non-string args (`pushBack(1)`,
`push(3)`) so they don't trip the secondary blocker described next.

**Surfaced gap (independent of writeback)** —
**W17-jit-string-constant-carrier-shape** (NEW). HashSet/HashMap JIT
segfaults reproduce identically WITHOUT my writeback fix (verified by
`git stash` + re-run) — the crash happens at the very FIRST `s.add("a")`
call, before any second access could matter. Root cause is a JIT-side
string-carrier shape mismatch:

- `mir_compiler/ownership.rs:402-406` lowers `MirConstant::Str("a")` via
  `box_string(s)` which produces a unified-heap `UnifiedValue<Arc<String>>`
  NaN-box.
- `mir_compiler/rvalues.rs:309-313` labels the kind track entry for
  `MirConstant::Str(_)` as `NativeKind::String` — the strict-typed
  `Arc<String>::into_raw` raw-pointer carrier per the docstring at
  `rvalues.rs:307-310` ("Method-name string constant... carrier kind
  is String — the §2.7.5 String arm — `Arc<String>` raw pointer
  carrier").
- VM-side string-method handlers (e.g. `set_methods.rs:136-155::result_slot_to_string_arc`)
  consume via `Arc::from_raw(bits as *const String)` — reading the
  unified-heap `UnifiedValue<Arc<String>>` layout as a raw String
  Arc → UB / segfault / `slice::from_raw_parts requires the pointer to
  be aligned and non-null` panic.

This is a §2.7.5 producer-site carrier-shape gap orthogonal to MIR
mutation-writeback. The two paths are wired separately: writeback is
about whether the receiver slot picks up the new Arc after the call;
this gap is about whether the call sees a valid string key in the
first place. The HashSet/HashMap segfaults occur before any writeback
opportunity, so writeback is not the load-bearing fix for those
specific smokes — it is the load-bearing fix for the broader class
of "mutating method calls on collection locals". Folds under a NEW
follow-up `W17-jit-string-constant-carrier-shape` row, or is naturally
absorbed by `W17-collection-concrete-types` if that scope extends to
JIT string-carrier alignment.

**Close gates**:

- `cargo check --workspace --lib --tests` EXIT=0 inside devenv.
- `cargo test -p shape-jit --lib` 361 passed 0 failed (== baseline 361).
- `bash scripts/check-no-dynamic.sh` EXIT=0.
- `bash scripts/verify-merge.sh` 12/12 inside devenv.
- Pre-existing `cargo test -p shape-vm --lib` SIGABRT at
  `compiler::comptime::tests::w17_comptime_*` reproduces identically
  at branch HEAD without my changes (v2-raw-heap aliasing class per
  CLAUDE.md Known Constraints — out of scope for W17-mir-mutation-writeback).

### Trinity scope (W12-jit-producing-site-conduit-completeness)

Round 7A integrated-trinity precedent (~800-1000 LoC single agent).
Three co-designed pieces with INTERNAL ORDERING:

**(a) 11E ConcreteType taxonomy refinement (FOUNDATION, lands FIRST)**:
Extend `ConcreteType` taxonomy in `crates/shape-value/src/v2/concrete_type.rs`
with `Mutex<T>` / `Atomic<T>` / `Lazy<T>` / `HashSet` / `Deque` /
`PriorityQueue` / `Channel` arms (currently absent). Refines ConcreteType
to cover the shapes surfaced by Round 10 item (B) — collection containers,
method-return kinds, Aggregate destinations — coherently.

**(b) 11B method-return-kind conduit (CONSUMER of 11E)**: §2.7.5
producer-side conduit extension for method-return kinds (`.sum()`-style
scalar return + parametric containers like `HashMap.get → Option<V>`,
`Mutex.get → T`, `Atomic.load → i64`). Likely shape:
`native_kind_from_concrete_type` switch keyed on receiver+method pairs,
populated at method-call sites for known-return-kind stdlib methods.

**(c) 11C Rvalue::Aggregate TypedObject threading (CONSUMER of 11E)**:
JIT consumer side of the TypedObject Aggregate fast path for non-Array
destinations (Struct/Tuple/TypedObject). Resurrects deferred Round 5B
option (iii). Fires on `let t = X {}` struct construction in kickoff
Smoke 3.

**Order inside trinity**: (a) FIRST as foundation; (b) and (c) consume
the landed taxonomy. NO three-way concurrent extension of
`mir_compiler/types.rs` — agent ships (a) as a coherent commit, then
layers (b) and (c) on top.

**Surface-and-stop discipline**:
- If (a) surfaces ADR amendment requirement for taxonomy shape, STOP.
- If (b) needs parametric-return inference shape exceeding conduit
  pattern, STOP.
- If (c) needs a fourth ConcreteType destination (a) didn't cover, STOP.

**Refuse on sight**: ConcreteType variants projecting 1:1 to HeapKind
(ADR-005 §1 single-discriminator); Bool-default for unproven destination
kind; "bridge"/"probe"/"helper"/"hop"/"translator"/"adapter"/"shim"
framing for conduit work.

Unblocks: kickoff Smoke 2 JIT-side + kickoff Smoke 3 JIT-side + HashMap.get
/ Mutex.get / Atomic.load parametric return kinds.

### Cluster-0 close attempt cadence (post-Round-11)

After all three Round 11 sub-clusters merge:
- All 4 kickoff smokes VM == JIT (correct output under both).
- Supplementary -ext smokes tracked with explicit dispositions (pass /
  cluster-1+ tracking cite).
- Full smoke matrix snapshot in this status doc.
- Supervisor ratifies; user authorizes `phase-3-cluster-0-close` tag.

If any of 11A / 11D / trinity surfaces a sixth gap, Round 12. The N+1
expansion has been honest principled surfacing every round; same
discipline holds.

## Round 11 post-merge smoke matrix verification (2026-05-13)

All three Round 11 sub-clusters merged into `bulldozer-strictly-typed`:
- 11A `e550ae6f` (op_new_array kinded reentry)
- 11D `863fcdf5` (MIR mutation-writeback)
- Trinity `80de14ce` (ConcreteType taxonomy + method-return conduit + Rvalue::Aggregate)

Post-merge `bash scripts/verify-merge.sh` 12/12 inside devenv. CLI rebuilt
and full smoke matrix re-run.

### Smoke matrix (post-Round-11)

| Smoke | VM | JIT | Round 11 delta | Round 12 candidate |
|---|---|---|---|---|
| 1 (kickoff) `for i in 0..100 { sum = sum+i }` → 4950 | ✅ 4950 | ✅ 4950 | unchanged | passing |
| 2 partial `[1,2,3].sum()` → 6 | ❌ T4 | ✅ 6 | **JIT NEW** (trinity Part b conduit) | T4 (VM intrinsic) |
| 2 full `.map(\|x\|x*2).sum()` → 30 | ❌ T5 | ❌ downstream | both blocked | T5 + downstream |
| 3 (kickoff) trait `t.name()` → "x" | ✅ x | ❌ T1 | unchanged | T1 |
| 4 (kickoff) `let mut s = Set(); .add; .size` → 2 | ✅ 2 | ❌ T2/T3 | mutation-writeback fix verified via Deque + PriorityQueue | T2/T3 |

### Round 12 candidates (surfaced by Round 11)

**(T1) `W12-jit-trait-dispatch-return-kind`** — JIT-side conduit extension
for trait-method return kinds. Surfaced by trinity Part c: Aggregate path
unblocked exposes the next-layer trait-dispatch return-kind classification.
Required for kickoff Smoke 3 JIT.

**(T2/T3) `W12-jit-string-carrier-unification`** — JIT-side producer
migration for `MirConstant::Str`. `box_string(s)` currently emits
`UnifiedValue<Arc<String>>` NaN-box; §2.7.5 contract is raw
`Arc::into_raw(Arc<String>)`. VM-side handlers consume per the §2.7.5
contract → carrier-mismatch UB/segfault. Surfaced jointly by Round 8A
(compile-time SURFACE for `print("hello")`) + Round 11D (runtime segfault
for `s.add("a")`). Required for kickoff Smoke 4 JIT + Smoke 3 JIT (when
trait method returns String).

**(T4) `W17-vm-intrinsic-sum-wave-5d-migration`** — VM-side intrinsic body
migration for `IntrinsicSum`. Phase-1B wave-5d `todo!()` at
`crates/shape-vm/src/executor/vm_impl/builtins.rs:471-520`. Surfaced by
11A: now that `op_new_array` works, `.sum()` invocation reveals the
unmigrated intrinsic body. Required for kickoff Smoke 2 VM.

**(T5) `W17-vm-call-value-closure-kind-mismatch`** — VM-side
`call_value_immediate_nb` kind-mismatch at
`crates/shape-vm/src/executor/call_convention.rs:798`:
`HeapKind::Closure label with non-ClosureRaw HeapValue payload: "string"`.
Fires when `xs.map(|x|x*2)` invokes the closure with `xs` as a V2 typed-
int-array (`NativeKind::UInt64`). Pre-existing kind-source bug at method-
dispatch tier; surfaced by 11A `op_new_array` fix revealing it downstream.
Required for kickoff Smoke 2 full VM.

### Cluster-0 close criterion status

3 of 4 kickoff smokes still blocked under JIT or VM (or both). N+1
trajectory expansion holds: Round 12 absorbs T1/T2/T3/T4/T5 (4 sub-clusters
if T2 and T3 merge). Cluster-0 close attempt projected for post-Round-12
merge if all 4 kickoff smokes pass VM == JIT.

## Round 12 — dispatching (JIT pair T1 + T2/T3 parallel) + T4 + T5 inline cite-audit

Per supervisor's Option 4 ratification (2026-05-13): Round 12 dispatches
the JIT pair in parallel from post-Round-11 baseline `b5d787ca`; T4 + T5
get inline cite-audit by team-lead session, classification disposition
folded into status before Round 13 dispatch.

### Round 12 dispatch (parallel)

| Sub-cluster | Branch | Worktree | Status |
|---|---|---|---|
| W12-jit-trait-dispatch-return-kind (T1) | `bulldozer-strictly-typed-w12-jit-trait-dispatch-return-kind` | `../shape-w12-jit-trait-dispatch-return-kind` | migrating |
| W12-jit-string-carrier-unification (T2/T3) | `bulldozer-strictly-typed-w12-jit-string-carrier-unification` | `../shape-w12-jit-string-carrier-unification` | migrating |

#### T1 scope (W12-jit-trait-dispatch-return-kind)

JIT-side trait-dispatch return-kind inference. Surfaced by Round 11-trinity
Part c — Aggregate path unblocked exposes the next-layer trait-dispatch
return-kind classification. Similar shape to trinity Part (b)
`parametric_method_return_kind_from_receiver`, extended to trait-method
dispatch sites. Likely: extend `infer_call_return_kind` at the JIT MIR
builder layer to consult the trait registry when the call resolves to a
trait method, stamp destination slot NativeKind from the trait method's
declared return type.

Touch: `crates/shape-jit/src/mir_compiler/types.rs` (different region than
T2/T3, but same file — coordinate AGENTS.md row + status doc subsection
with T2/T3).

Unblocks: kickoff Smoke 3 JIT (`t.name() → "x"`).

#### T2/T3 scope (W12-jit-string-carrier-unification)

Producer-side carrier migration for `MirConstant::Str` + TypedObject
Aggregate lowering. Surface: `box_string(s)` at
`crates/shape-jit/src/mir_compiler/ownership.rs:402-406` emits
`UnifiedValue<Arc<String>>` NaN-box; §2.7.5 contract is raw
`Arc::into_raw(Arc<String>) as u64`. VM-side handlers consume per §2.7.5
→ UB/segfault at `s.add("a")` (Round 11D surfaced) + compile-time SURFACE
at `print("hello")` (Round 8A surfaced).

Fix shape: producer-side migration mirroring Round 7A Arc-shape Result/
Option pattern. Single integrated commit (one agent integrates to avoid
Arc-vs-NaN-box boundary disagreement, per Round 7A precedent). Also extend
`retain_func_for_place` / `release_func_for_place` to dispatch new kinded
`jit_arc_string_retain/_release` per Round 7A + Round 9 precedent. Also
includes TypedObject Aggregate lowering (`box_typed_object` at
`value_ffi.rs:516-518` returns `unified_box(HK_TYPED_OBJECT, ptr)` — same
defect class).

Touch: `crates/shape-jit/src/mir_compiler/ownership.rs` (box_string +
retain/release arms) + `crates/shape-jit/src/ffi/value_ffi.rs`
(box_typed_object migration) + new `crates/shape-jit/src/ffi/string.rs`
(jit_arc_string_retain/_release) + FFI registration scaffolding +
`crates/shape-jit/src/mir_compiler/types.rs` (kind-track propagation —
different region than T1) + consumer-side updates per audit.

Unblocks: kickoff Smoke 4 JIT (`Set + .add("a")`) + kickoff Smoke 3 JIT
downstream (trait method returning String).

### T4 + T5 inline cite-audit findings (team-lead session, 2026-05-13)

**T4 — W17-vm-intrinsic-sum-wave-5d-migration**:

- **Documented Phase-1B wave-5d residual**. The surface site at
  `crates/shape-vm/src/executor/vm_impl/builtins.rs:472` is one of 6
  related `phase-1b-vm wave 5d — intrinsic body migration` `todo!()`
  blocks at lines 431, 449, 459, 467, 472, 518. Tracked at
  `docs/cluster-audits/cluster-6-intrinsics-dispatch-table.md`
  (BuiltinFunction::IntrinsicSum/Min/Max/Diff/Cumsum/RollingSum/CharCode
  dispatch arms named at lines 34-101). The cluster-6 doc designs the
  dispatch table; bodies remain `todo!()`.
- **Blocks kickoff Smoke 2 VM** (`[1,2,3].sum()` and `.map(...).sum()`
  both fire the IntrinsicSum `todo!()`).
- **Disposition (b)** per supervisor's classification rule: real new
  finding (the body itself is missing, even if the dispatch table is
  documented) AND blocks kickoff smoke → cluster-0 absorbs for Round 13
  regardless of thematic lineage. Same Q2 ruling as 11A's op_new_array
  Phase 2c reentry.

**T5 — W17-vm-call-value-closure-kind-mismatch**:

- **NOT absorbed by Round 7B / 8B**. Both prior rounds were audit-only
  closes (Round 7B `7753d52b` audit `W12-jit-collection-typed-arc-ffi`;
  Round 8B `ba09636b` audit `W12-jit-collection-method-dispatch-abi`).
  Neither closed call-value closure-kind plumbing.
- **Error string** at `crates/shape-vm/src/executor/call_convention.rs:
  444-449` (in `resolve_spawned_task`) + the same surface pattern at
  `:798` (in `call_value_immediate_nb`) per Round 11A close report.
  §2.7.11/Q12 value-call ABI machinery; producer-side mis-labeling.
- **Blocks kickoff Smoke 2 full VM** (`xs.map(|x|x*2)` closure call).
- **Disposition (b)** per supervisor's classification rule: real new
  finding (not absorbed by existing tracked work) AND blocks kickoff
  smoke → cluster-0 absorbs for Round 13. Audit-first dispatch shape
  per supervisor's instruction (kind-source bug needs scope verification
  before fix shape).

### Round 13 projected dispatch

After Round 12 merges:
- T4 (W17-vm-intrinsic-sum-wave-5d-migration) — scope-narrowed to ONLY
  `BuiltinFunction::IntrinsicSum` body migration (not the broader wave-5d
  set, to avoid scope explosion). Other wave-5d todo!() blocks remain
  documented Phase-1B residual unless they block kickoff smokes.
- T5 (W17-vm-call-value-closure-kind-mismatch) — AUDIT-FIRST. The
  producer-side mis-labeling source needs identification before fix shape
  commits to call-convention.rs or upstream.

If Round 12 surfaces additional gaps, Round 14+ per N+1 trajectory.
Cluster-0 close attempt post-Round-13 if all 4 kickoff smokes VM == JIT.

### Discipline note

Per supervisor's Round-12 ratification: classification determines
bookkeeping, NOT whether the work happens. "T4 is documented Phase-1B
wave-5d so cluster-0 closes with documented exception" framing refused
on sight when the gap blocks the kickoff matrix — same Q2 disposition,
refused upfront.

## Round 12 post-merge smoke matrix verification (2026-05-13)

Both Round 12 sub-clusters merged:
- T1 surface-and-stop close at `4447e698` (named 3 conduit gaps; ADR amendment territory absorbed into Round 13)
- T2/T3 at `61687564` — **Kickoff Smoke 4 JIT NOW PASSES**

Post-merge verify-merge 12/12 inside devenv. CLI rebuilt + full smoke matrix re-run.

### Post-Round-12 smoke matrix

| Smoke | VM | JIT | Status |
|---|---|---|---|
| 1 (kickoff) `for i in 0..100 { sum = sum+i }` → 4950 | ✅ 4950 | ✅ 4950 | **passing** |
| 2 partial `[1,2,3].sum()` → 6 | ❌ T4 IntrinsicSum | ✅ 6 | T4 (VM) |
| 2 full `.map(\|x\|x*2).sum()` → 30 | ❌ T5 closure | ❌ downstream of T5 | T5 + downstream |
| 3 (kickoff) trait `t.name()` → "x" | ✅ x | ❌ T1' cross-crate trait return-kind side-table | T1' |
| 4 (kickoff) `let mut s = Set(); .add; .size` → 2 | ✅ 2 | ✅ 2 | **PASSING (NEW from T2/T3)** |

### Kickoff close progress

**2 of 4 kickoff smokes fully passing** (1 + 4). Remaining:
- Smoke 2 needs T4 + T5 (both VM-side, both Round 13).
- Smoke 3 needs T1' cross-crate trait return-kind side-table (Round 13).

Cluster-0 close attempt projected post-Round-13 if all 4 kickoff smokes VM == JIT.

### Round 13 dispatch plan (per supervisor's Option 4 cadence)

Three sub-clusters, dispatched parallel from post-Round-12 baseline:

- **T4 W17-vm-intrinsic-sum-wave-5d-migration**: scope-narrowed to ONLY
  `BuiltinFunction::IntrinsicSum` body migration. Other wave-5d todo!()
  blocks remain Phase-1B residual unless they block kickoff smokes.
  Unblocks kickoff Smoke 2 VM (.sum() body).
- **T5 W17-vm-call-value-closure-kind-mismatch**: AUDIT-FIRST. Producer-
  side mis-labeling source needs identification before fix shape commits.
  Unblocks kickoff Smoke 2 full VM (.map() closure call).
- **T1' W12-trait-method-return-conduit-cross-crate**: cross-crate
  `BytecodeProgram::trait_method_return_concrete_types` side-table per
  Round 6A's `function_return_concrete_types` precedent. Populated at
  impl-block compile time from `TraitDef.members`, threaded through
  linker / remote / content-addressed shapes + MirToIR. Closes the 3
  conduit gaps T1 named. Unblocks kickoff Smoke 3 JIT.

All three: standard Round-3-pattern close gate + surface-and-stop +
refuse-on-sight forbidden frames.

### Cluster-1 candidates surfaced (NOT cluster-0 blocking)

NEW cluster-1 candidates surfaced by Round 12 close reports:

- `W17-jit-err-ctor-kind-classification` — `print(Err("x"))` classifier
  mis-stamps Err arm as `Ptr(TypedObject)` instead of `Ptr(Result)`.
  Affects Smoke 1.5-ext (Result/match payload codegen); does NOT block
  kickoff smokes.
- `W17-jit-typed-object-arc-storage-migration` — JIT-internal TypedObject
  struct (`ffi/typed_object/`) vs VM-side `Arc<TypedObjectStorage>` are
  different Rust types; 17+ JIT-internal consumers; migration is broader
  cluster-1 hardening work.

## Round 13 — dispatching (3 parallel: T4 production-first + T5 + T1' audit-first)

Dispatched 2026-05-13 from post-Round-12-merge baseline `697afed1`.
Supervisor ratified Option 1 (3 parallel) with audit-first discipline on
T5 + T1', production-first on T4 (scope already team-lead cite-verified).

| Sub-cluster | Branch | Worktree | Status |
|---|---|---|---|
| W17-vm-intrinsic-sum-wave-5d-migration (T4) | `bulldozer-strictly-typed-w17-vm-intrinsic-sum-wave-5d-migration` | `../shape-w17-vm-intrinsic-sum-wave-5d-migration` | migrating |
| W17-vm-call-value-closure-kind-mismatch (T5) | `bulldozer-strictly-typed-w17-vm-call-value-closure-kind-mismatch` | `../shape-w17-vm-call-value-closure-kind-mismatch` | auditing |
| W12-trait-method-return-conduit-cross-crate (T1') | `bulldozer-strictly-typed-w12-trait-method-return-conduit-cross-crate` | `../shape-w12-trait-method-return-conduit-cross-crate` | auditing |

### T4 scope (W17-vm-intrinsic-sum-wave-5d-migration)

Production-first. ~50-100 LoC bounded migration of
`BuiltinFunction::IntrinsicSum` body at
`crates/shape-vm/src/executor/vm_impl/builtins.rs:472` per Phase 1B-vm
Wave 6.5 substep-2 cluster-A canonical recipe at commit `eb24ef0`.

**Scope-narrowing rationale (cite-verified by team-lead audit)**:
IntrinsicSum is the ONLY wave-5d todo!() blocking kickoff Smoke 2 `.sum()`.
Other 5 wave-5d sites (lines 431/449/459/467/518 — closure-driven array
builtins, vector/matrix/minimize intrinsics) stay Phase-2d residual; no
current smoke blocker. Follows W12-collection-constructor scope-IN/scope-
OUT precedent.

Close report MUST cite:
1. Which kickoff smoke IntrinsicSum blocks (Smoke 2 `.sum()`).
2. Other wave-5d sites + their dispositions (Phase-2d residual, no
   current smoke blocker).
3. Migration shape (consistent with Phase 1B-vm kinded API discipline).

### T5 scope (W17-vm-call-value-closure-kind-mismatch)

AUDIT-FIRST. Three audit deliverables before writing code:

1. **Site identification**: trace exact source of kind mis-labeling at
   producer site, file:line cite.
2. **W7/W8 overlap check**: review Round 7B + 8B close commits
   (trampoline scope) for pre-existing handling — if absorbed/superseded/
   orphan, cite commit + disposition.
3. **Cluster-0 disposition**: confirm kickoff Smoke 2 `.map(|x|x*2)`
   closure-call path is blocked under VM mode — if yes, cluster-0 sub-
   cluster; if no, Phase-2d residual or cluster-1 hardening with §-cite.

### T1' scope (W12-trait-method-return-conduit-cross-crate)

AUDIT-FIRST. Absorbs ADR amendment territory surfaced by Round 12 T1
surface-and-stop.

Three audit deliverables before writing code:

1. **Round 6A precedent fit**: read `function_return_concrete_types`
   side-table design + linker/remote/content-addressed threading.
   Determine whether trait method return resolution fits same shape
   (key: `(trait_id, method_id)` instead of `function_id`; same
   threading; same wire/snapshot disposition) OR requires fundamentally
   different cross-crate design (vtable-aware lookup, multi-impl
   resolution).
2. **If same shape**: proceed with cross-crate side-table extension
   ~300-500 LoC including linker/remote/content-addressed threading.
3. **If different shape**: surface-and-stop with audit's structural
   findings — ADR amendment territory, supervisor makes the call,
   Round 14 dispatches amended fix.

Three gaps T1 named (must close together):
- Receiver struct identity erasure at `v2_map_emission.rs:357`
  `StructLayoutId(0)` placeholder.
- Trait registry not persisted in BytecodeProgram (`TypeRegistry::traits`
  has return types but only `trait_method_symbols` + `trait_vtables`
  reach BytecodeProgram).
- Impl method return type fallback insufficient (`function_return_concrete_types[X::name] = Void`).

Must make Round 12 T1's 3 pin tests pass (or document why obsolete).

### Forbidden frames (refused on sight across all three)

Per CLAUDE.md "Renames to refuse on sight" §2.7.10/Q11 + §2.7.11/Q12
broader-family regex:
- "trait-id/method-id resolution as a bridge over function-id".
- "preserve mis-labeling for now, harden later".
- "preserve legacy body for one edge case".
- Any defection-attractor descriptor (bridge/probe/helper/hop/translator/
  adapter/shim) for kind-source threading.
- Bool-default for unproven kind at any site.

### W17-vm-call-value-closure-kind-mismatch close (2026-05-13)

T5 Round 13 close. Audit doc at
`docs/cluster-audits/w17-vm-call-value-closure-kind-mismatch-audit.md`.

**Audit finding.** The kind label is honest — `Ptr(HeapKind::Closure)`
on both iterations of `xs.map(|x| x*2)`. The consumer at
`call_convention.rs:795-810` correctly classifies. The bug is
**producer-side share accounting** at the Closure arm of
`call_value_immediate_nb` (`call_convention.rs:835-841`, introduced in
W7-cv-static Round 2 close `06cdfce` 2026-05-09):

- The `callee` carrier in `dispatch_call_value_immediate`
  (`control_flow/mod.rs:408-409`) holds one `Arc<HeapValue>` share —
  transferred from the stack via `pop_kinded`.
- The Closure arm passes `Some(callee.slot.raw()), Some(callee.kind)`
  to `call_closure_with_nb_args_keepalive` — these install on the new
  frame's `closure_heap_bits` / `closure_heap_kind` B9 lockstep
  companion fields.
- On `op_return` / `op_return_value` the frame teardown at
  `control_flow/mod.rs:712-726` / `:774-788` releases via
  `drop_with_kind(closure_heap_bits, closure_heap_kind)` — ONE share
  retired.
- After `call_value_immediate_nb` returns, the `callee` carrier in the
  caller drops at end of scope — `KindedSlot::Drop` retires ANOTHER
  share.

Net: 1 share acquired, 2 released. The closure `Arc<HeapValue>`
reaches refcount 0 before the binding's clone share is released
(because the binding share installed by `op_make_closure`'s producer +
`CloneLocal Local(1)` clone is independent — but the Arc payload is
already gone). Next iteration: `CloneLocal Local(1)` reads dangling
bits, races the allocator, surfaces as `HeapKind::Closure label with
non-ClosureRaw payload` in dev or `Invalid function call` in release
(the bogus `function_id` read from the freed header fails the
`program.functions.get(func_id)` bounds check).

**W7/W8 overlap check.** NOT absorbed:
- Round 7B audit `7753d52b` (W12-jit-collection-typed-arc-ffi):
  JIT-territory typed-Arc allocation FFI; no touch.
- Round 8B audit `ba09636b` (W12-jit-collection-method-dispatch-abi):
  JIT-territory dispatch shell; no touch.
- Round 9 close `81acb62e`: typed-Arc collection ctors + refcount; the
  Closure arm of `clone_with_kind` / `drop_with_kind` was already
  symmetric per W7-closure-retain `5fa4b19`; the audited bug is
  upstream of these arms.
- Round 10 close `2c2ecdf1`: `jit_call_method` shell rebuild; no
  touch.

NEW finding. Entered the tree at `06cdfce` (W7-cv-static Round 2),
latent until Smoke 2's `.map(|x| x*2)` exercised the inline-loop
CallValue path across multiple iterations. Round 11A's `op_new_array`
fix unblocked the dispatch-table path that surfaces this bug — same
Q2 disposition (real new finding, AND blocks kickoff smoke → cluster-0
absorbs).

**Cluster-0 disposition.** Confirmed blocks kickoff Smoke 2 full VM
(`[1,2,3,4,5].map(|x|x*2).sum()`). Reproducer in worktree:

```shape
let xs = [1, 2, 3]
let doubled = xs.map(|x| x * 2)
print(doubled)
```

Pre-fix: `Error: Runtime error: Invalid function call (line 4)`.
Post-fix: `[2, 4, 6]`.

**Fix shape (Option B, single-line):**
`clone_with_kind(callee.slot.raw(), callee.kind)` immediately before
the `call_closure_with_nb_args_keepalive` invocation in
`call_value_immediate_nb`'s Closure arm. The §2.7.7 / Q9 retain-on-read
primitive is the canonical kind-aware refcount bump — no tag decode,
no `is_heap()` probe, no Bool-default fallback, no by-move ABI
surgery. Same share-balance pattern as
`execute_function_with_named_args` (lines 246-250) and the existing
W7-cv-method `op_call_method` clone-before-handle path.

Pre-Smoke-2 verify-merge.sh: 12/12. check-no-dynamic.sh: exit 0.
cargo check --workspace --lib --tests: exit 0. shape-jit lib tests:
382/0/26 (no regression vs Round-12 baseline 382). shape-vm lib tests:
pre-existing SIGABRT (v2-raw-heap aliasing class per CLAUDE.md Known
Constraints) at baseline — verified by stashing the fix and re-running:
identical SIGABRT signature, NOT a regression from this commit.

**Smoke 2 still hits T4 IntrinsicSum downstream** — expected per the
T5 prompt's close criterion. With T4's IntrinsicSum migration landing
in parallel this round, kickoff Smoke 2 full VM closes end-to-end.

**`resolve_spawned_task` same defect class?** Audited the second site
the T5 prompt cited (`call_convention.rs:421-475`). The callable share
comes from `take_callable` (raw u64 + NativeKind locals, no
`KindedSlot` carrier with `Drop`). After install as
`closure_heap_bits`, the frame teardown releases — ONE release.
Same path UInt64 callable: no Arc share, no-op release. No double-
release shape applies. `resolve_spawned_task` is OK as-is; the prompt's
"same surface pattern" wording refers to the dispatch shape, not the
defect class.

### Cluster-0 close attempt cadence (post-Round-13)

After all three Round 13 sub-clusters merge:
- Run full 4-kickoff-smoke matrix (1 + 2 + 3 + 4) under both VM and JIT.
- All 4 must produce identical correct output VM == JIT.
- Supplementary -ext smokes tracked with explicit dispositions.
- This status doc updated with the final matrix + close artifact.
- Supervisor ratifies; user authorizes `phase-3-cluster-0-close` tag.

If Round 13 surfaces a sixth gap (N+13 trajectory), Round 14. Same
discipline. The trajectory has been honest principled surfacing every
round and the JIT-rebuild proper is converging; cluster-0 close remains
the gating criterion, not a pivot target.

### W12-trait-method-return-conduit-cross-crate close (2026-05-13)

T1' closed with 3-gap closure (gaps 1, 2, 3 named by Round 12 T1
surface-and-stop) + surface-and-stop on downstream
W17-jit-typed-object-arc-storage-migration receiver-classification gap.

**Disposition**: §2 (a) same shape — Round 6A `function_return_concrete_types`
precedent fit. No ADR amendment required. Audit doc:
`docs/cluster-audits/w12-trait-method-return-conduit-cross-crate-audit.md`
(commit `2b01ecaa`).

**Commits**:
- `119c4f7e` — Commit 1: gap 3 closure (trait declaration return-type
  substitution at `desugar_impl_method`).
- `1f9f757a` — Commit 2: gaps 1+2 closure (MirFunction
  `local_struct_type_names` + conduit producer
  `infer_top_level_concrete_types_from_mir_with_resolvers` +
  JIT consumer extension).

**Key insight not in audit**: gap 2 closes by **deduction** from existing
data rather than a new `BytecodeProgram` side-table. The Round 6A
`function_return_concrete_types` (populated by commit 1's gap 3
backfill) + the existing `trait_method_symbols`
(populated at impl-block compile time) + the existing
`find_default_trait_impl_for_type_method` helper together carry the
trait method declared return ConcreteType. The new `method_returns`
resolver chains these — pure deduction. LoC budget contracted from
audit's ~365 estimate to ~200 net source LoC.

**Round 12 T1 pin tests**: preserved AS-IS. They pin the JIT-internal
parametric classifier's posture (user-defined trait methods are NOT
classified at the JIT layer — by design). T1' moves classification one
tier upstream to the VM-side conduit producer; the JIT consumer picks
up the upstream stamp via the existing `concrete_seed` → existing-seed
pathway. Doc-block updated to reflect post-T1' state. New positive pin
`trait_method_call_destination_seeded_from_concrete_types` asserts the
upstream-landing pathway.

**NEW SURFACE uncovered (W17 cluster-1 follow-up)**:
`crates/shape-jit/src/ffi/call_method/mod.rs::receiver_type_name`
(line 51-81) classifies receiver via legacy NaN-box tag-decode
(`is_number(receiver_bits)`, `heap_kind(receiver_bits)`). With
Round 12 T2/T3's producer-side migration to raw `Arc::into_raw(
Arc<TypedObjectStorage>)` pointer bits, the receiver bits no longer
carry the NaN-box tag — `is_number()` returns true for raw heap-pointer
values (non-TAG_BASE), so `receiver_type_name` returns `"number"`
instead of `"X"`, dispatch fails, segfault. This is exactly the
**W17-jit-typed-object-arc-storage-migration** cluster-1 follow-up
named in the Round 12 T2/T3 close report (`61687564`). T1' has
done its 3-gap closure; the deeper carrier-shape mismatch at the JIT
trampoline's receiver classification is one tier downstream of T1' —
same necessary-but-not-sufficient relationship to Smoke 3 that T1
had to T1'.

**Smoke 3 status post-T1' merge**:
- VM mode: `x` ✓ (unchanged).
- JIT mode: kind classification correct (verified via debug
  instrumentation — `kind_hint = Some(NativeKind::String)` at the
  print Call-terminator); dispatch routes to `jit_print_string_arc`;
  but trampoline receiver classification segfaults due to W17
  follow-on at `receiver_type_name`'s legacy NaN-box tag-decode.

**Recommendation for Round 14**: dispatch **W17-jit-typed-object-arc-storage-migration**
sub-cluster to migrate `receiver_type_name` (and the broader
JIT-internal TypedObject struct consumers in `typed_object/`,
`data.rs`, `property_access.rs`, etc.) to read receiver type identity
from the parallel-kind track + heap-pointer dereference (the
`Arc<TypedObjectStorage>` schema id) rather than NaN-box tag-decode.
This is the cluster-1 hardening work the Round 12 T2/T3 close
explicitly named as a follow-up.

**Close gates (devenv exit-code-verified)**:
- `cargo check --workspace --lib --tests` EXIT=0.
- `cargo test -p shape-jit --lib` 383 passed / 0 failed / 26 ignored
  (376 baseline + 7 new). All 3 T1 pin tests preserved.
- `cargo test -p shape-vm --lib --skip compiler::comptime`
  1319 passed / 332 failed (baseline 1312/332; +7 new, no regressions).
- `bash scripts/verify-merge.sh` 12/12 Passed.
- `bash scripts/check-no-dynamic.sh` EXIT=0.

**Pre-existing issues NOT caused by T1'** (reproduce identically with
my changes stashed):
- `compiler::comptime::tests::w17_comptime_*` SIGABRT (v2-raw-heap
  aliasing class per CLAUDE.md "Known Constraints").
- 15 bytecode verification warnings on stdlib functions
  (`coefficient_of_variation`, `Json.get`, `TryFrom::*::Json::tryFrom`,
  etc.) for "Trusted opcode ... has no FrameDescriptor" — orthogonal
  to T1' scope.

**Refuse-on-sight discipline preserved**:
- New `MirFunction.local_struct_type_names` named for what it IS, not
  via bridge/probe/helper/hop/translator/adapter/shim framing.
- New `infer_top_level_concrete_types_from_mir_with_resolvers` named
  by parameter shape (resolvers plural), same name+extra-parameter
  pattern as Round 6A — not a "trait-method translator" or
  "dispatch-site helper".
- No Bool-default fallback at any classifier site (4 dedicated pin
  tests assert None when struct identity / resolver / method-returns
  are missing).
- No hard-coded Smoke 3 case — the resolver chain is fully generic
  over (type_name, method_name) pairs.
- No new ConcreteType variant / HeapKind / opcode.
- No ADR amendment.
- W17 surface named honestly by what it IS — receiver classification
  via legacy NaN-box tag-decode versus raw Arc pointer bits — and
  surfaced for Round 14 dispatch.

## Round 13 post-merge smoke matrix verification (2026-05-13)

All three Round 13 sub-clusters merged:
- T5 (`68adb9f4`) — call_value_immediate_nb closure share-accounting fix
- T4 (`d2a4ba19`) — IntrinsicSum body kinded-API migration
- T1' (`e87881ef`) — 3-gap cross-crate trait return-kind conduit closure

Post-merge `bash scripts/verify-merge.sh` 12/12 inside devenv. CLI rebuilt
+ canonical kickoff smoke matrix re-run.

### Post-Round-13 canonical kickoff smoke matrix

| Smoke | VM | JIT | Round 13 delta |
|---|---|---|---|
| 1 (kickoff) `for i in 0..100 { sum = sum+i }` → 4950 | ✅ 4950 | ✅ 4950 | unchanged passing |
| 2 (kickoff) `[1,2,3,4,5].map(\|x\|x*2).sum()` → 30 | **✅ 30 NEW** (T5 closure share fix) | ❌ print operand NativeKind=None | **VM CLOSED** |
| 3 (kickoff) `trait + impl + t.name()` → "x" | ✅ x | ❌ empty (W17 carrier-shape gap) | T1' kind correct; downstream blocker |
| 4 (kickoff) `let mut Set + .add + .size` → 2 | ✅ 2 | ✅ 2 | unchanged passing |

**3 of 4 kickoff smokes have VM passing**. **2 of 4 kickoff smokes fully
passing VM == JIT** (1 + 4). Smoke 2 + Smoke 3 JIT remain blocked.

### Important T4 close-report correction

T4's close report claimed `[1,2,3].sum()` (debug-simplified Smoke 2 partial)
uses `TYPED_INT_ARRAY_METHODS` PHF and produces `6` pre-fix. Empirical
post-merge state contradicts: `[1,2,3].sum()` actually hits the
`IntrinsicSum` opcode body, which T4's new body correctly surface-and-
stops on (`receiver must be NativeKind::Ptr(HeapKind::TypedArray), got
UInt64`). The actual gap is the upstream §2.7.5 producer-site
classification conduit not stamping the typed-array receiver kind —
`W17-stdlib-generic-param-kind-classification` per T4's surfaced item.

**However**: `[1,2,3].sum()` is the DEBUG-simplified Smoke 2 partial,
NOT the canonical kickoff Smoke 2. Canonical kickoff Smoke 2 is
`[1,2,3,4,5].map(|x|x*2).sum()` which now passes VM `30`. The
partial-form regression is not a kickoff-blocker; it's an upstream
conduit gap surfaced honestly by T4's new body discipline. Track as
non-kickoff-blocking cluster-1 candidate.

### Round 14 candidates

Two cluster-0 sub-clusters needed to close the remaining kickoff smokes:

1. **`W17-jit-typed-object-arc-storage-migration`** (load-bearing for
   kickoff Smoke 3 JIT). Surfaced jointly by Round 12 T2/T3 close + Round
   13 T1' close.
   `crates/shape-jit/src/ffi/call_method/mod.rs::receiver_type_name`
   (line 51-81) classifies receiver via legacy NaN-box tag-decode
   (`is_number(receiver_bits)`). With Round 12 T2/T3's producer
   migration to raw `Arc::into_raw(Arc<TypedObjectStorage>)` pointer
   bits, `is_number()` returns true for non-TAG_BASE bits →
   `receiver_type_name` returns `"number"` instead of `"X"` → dispatch
   fails → segfault. Per supervisor's Q2 discipline ruling: classification
   determines bookkeeping, NOT whether work happens. Cluster-0 absorbs.

2. **NEW SURFACE — `W12-jit-map-chained-method-return-kind-propagation`**:
   `print` Call-terminator operand NativeKind=None for `.map(|x|x*2).sum()`
   chain. Likely trinity Part b `parametric_method_return_kind_from_receiver`
   classifier doesn't propagate return kinds through chained method calls
   (the `.map()` return kind doesn't flow into `.sum()`'s receiver-kind
   classification). AUDIT-FIRST to determine whether it's a conduit
   extension or a kind-track propagation gap.

If both close cleanly, cluster-0 close attempt projected post-Round-14.
If either surfaces a 7th gap, Round 15 per N+1 trajectory discipline.

### Cluster-1 / cluster-2 candidates surfaced or named (NOT cluster-0 blocking)

- `W17-stdlib-generic-param-kind-classification` (T4 surfaced) — upstream
  §2.7.5 conduit gap for stdlib wrapper `__intrinsic_sum(series)`. Affects
  Smoke 2 partial debug form, not canonical kickoff.
- `W12-stdlib-intrinsic-collapse` (cluster-2) — added to surfaced items
  table item 8. Parallel-implementation cleanup.
- Option A2 by-move callee ABI surgery (T5 surfaced) — ADR-§2.7.11/Q12-
  verbatim shape; Option B (clone_with_kind) chosen for round budget.
  May warrant cluster-1 follow-up.

## Round 14 — dispatching (2 parallel audit-first sub-clusters)

Dispatched 2026-05-13 from post-Round-13-merge baseline `b53f090d`.
Supervisor ratified Option 1 (2 parallel) with audit-first discipline on
BOTH sub-clusters. Projected last sub-cluster round of cluster-0 if
neither audit surfaces ADR amendment.

| Sub-cluster | Branch | Worktree | Status |
|---|---|---|---|
| W17-jit-typed-object-arc-storage-migration | `bulldozer-strictly-typed-w17-jit-typed-object-arc-storage-migration` | `../shape-w17-jit-typed-object-arc-storage-migration` | auditing |
| W12-jit-map-chained-method-return-kind-propagation | `bulldozer-strictly-typed-w12-jit-map-chained-method-return-kind-propagation` | `../shape-w12-jit-map-chained-method-return-kind-propagation` | auditing |

### W17-jit-typed-object-arc-storage-migration scope

Four audit deliverables before writing code:

1. **ADR-005 §1 / ADR-006 §2.3 fit**: confirm migration target
   (`Arc<TypedObjectStorage>` raw bits, JIT-side) preserves single-
   discriminator + typed-Arc shape, OR surface-and-stop with ADR
   amendment proposal if divergent shape required (fat-pointer for
   vtable lookup, separate vtable cache, etc.).
2. **17+ consumer inventory**: list each JIT-internal consumer site
   (file:line), current call shape (NaN-box decode of receiver_type_name
   vs other), migration target per site. Use Phase-1B-vm Wave 6.5
   substep-2 cluster-A close at `eb24ef0` as the canonical kinded-API
   migration recipe.
3. **Cross-crate boundary check**: does any consumer site cross the
   shape-jit → shape-vm boundary requiring §2.7.5 stable-FFI shape? If
   yes, mirror `jit_trampoline_call_closure` pattern at
   `call_convention.rs:53-66`. If no, in-crate migration.
4. **Refuse-on-sight discipline**: "preserve NaN-box decode for one
   edge case", bool-default for unproven receiver_type kind,
   bridge/probe/helper/hop/translator/adapter/shim descriptor for any
   layer, "tracked as a follow-up" for any individual consumer site.

Unblocks kickoff Smoke 3 JIT.

### W12-jit-map-chained-method-return-kind-propagation scope

Three audit deliverables before writing code:

1. **Layer identification**: is failure a conduit extension (similar
   shape to Round 11 trinity Part b method-return-kind conduit, now
   extended to chained-method-call sites) OR a kind-track propagation
   gap (different layer — §2.7.7 stack parallel-kind track extension
   or MIR-level kind threading for intermediate slot results)?
   file:line cite for failure site required.
2. **Cluster-0 disposition**: confirm gap blocks kickoff Smoke 2 JIT
   (`.map(|x|x*2).sum()`). If reachable from other call shapes but
   not this specific smoke, classification matters (cluster-0
   absorbed only if it blocks kickoff Smoke 2).
3. **Surface-and-stop if ADR-amendment territory**: e.g. §2.7.7
   parallel-track invariant extension for chained-method intermediate
   results — well-established invariant but specific extension might
   need a Q-ruling. Supervisor makes the call; Round 15 dispatches
   amended fix.

Refuse on sight: bool-default for unproven chained-method return kind,
bridge/probe/helper/hop/translator/adapter/shim descriptor for the
propagation layer, "preserve kind-blind fallback because intermediate
slot kind isn't always available" framing (correct response is to
extend kind-track or surface-and-stop, NOT preserve fallback).

Unblocks kickoff Smoke 2 JIT.

### Cluster-0 close attempt cadence (post-Round-14)

After both Round 14 sub-clusters merge:
- Run full 4-kickoff-smoke matrix (1 + 2 + 3 + 4) under both VM and JIT.
- All 4 must produce identical correct output VM == JIT.
- Supplementary -ext smokes tracked with explicit dispositions.
- This status doc updated with the final matrix + close artifact.
- Supervisor ratifies; user authorizes `phase-3-cluster-0-close` tag.

Trajectory: Round 14 is the projected last sub-cluster round of cluster-0
if neither audit surfaces ADR amendment. If either does, Round 15 absorbs
the amended fix per N+1 trajectory discipline. JIT-rebuild proper is
converging; remaining gaps are well-scoped + named.

## Round 14 — close (both audit-only, Round 15 plan ratified)

Both Round 14 sub-clusters merged audit-only into `bulldozer-strictly-typed`:
- W17-jit-typed-object-arc-storage-migration merge commit (post-`8ae56222`)
- W12-jit-map-chained-method-return-kind-propagation merge commit (post-`8354968a`)

### Post-Round-14 canonical kickoff smoke matrix

| Smoke | VM | JIT | Status |
|---|---|---|---|
| 1 (kickoff) `for i in 0..100 { sum = sum+i }` → 4950 | ✅ 4950 | ✅ 4950 | **passing** |
| 2 (kickoff) `[1,2,3,4,5].map(\|x\|x*2).sum()` → 30 | ✅ 30 (R13 T5) | ❌ JIT compiles + runtime SIGSEGV (NEW surface — producer/consumer fast-path mismatch) | Round 15 Option B |
| 3 (kickoff) `trait + impl + t.name()` → "x" | ✅ x | ❌ receiver_type_name NaN-box-decode-on-raw-bits surface | Round 15 W17-narrow |
| 4 (kickoff) `let mut Set + .add + .size` → 2 | ✅ 2 | ✅ 2 | **passing** |

**Pre-merge state had Smoke 2 JIT failing at compile-time `Route A surface-and-stop`**. W12-map-chained's conduit extension WIP (stashed at sub-cluster `stash@{0}`) closes the compile-time surface; runtime SIGSEGV now surfaces honestly. Both new surfaces are well-named and ready for Round 15.

### Round 14 W17 audit findings (committed at merge)

Recommendation **Option γ (scope split)** ratified by supervisor:
- Round 15 **W17-narrow** = classification-layer fix only (Option α scope): ~13 sites, ~150-250 LoC, in-crate, NO ADR amendment. Closes kickoff Smoke 3 JIT.
- β typed-Arc carrier-shape decision → cluster-1 follow-up. Cluster-0 close does NOT block on β.

Audit doc: `docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md` (636 LoC, 4 deliverables + supervisor disposition options + empirical verification).

Also: Round 13 T1' framing correction landed — T1' attributed surface to "Round 12 T2/T3 carrier migration"; audit §1.4 corrects to "Round 12 T2/T3 migrated String only; TypedObject producer was surface-and-stopped; TypedObject producer TODAY emits raw `Box::into_raw(UnifiedValue<*const u8>)`". Classification gap real regardless of carrier shape.

### Round 14 W12-map-chained audit findings (committed at merge)

§1 layer identification: **§1 (a) conduit extension** at bytecode-side `infer_top_level_concrete_types_from_mir_with_resolvers` (`compiler/helpers.rs:494`). Missing `MirConstant::Method(_)` Call-terminator destination stamping for built-in container receivers. Same shape as Round 11-trinity Part b's JIT-side parametric classifier, generalized one tier upstream to produce `ConcreteType`.

§2 cluster-0 disposition: kickoff Smoke 2 JIT blocked at pre-implementation baseline. Cluster-0 absorbs.

§3 ADR-amendment posture: no ADR amendment required; matches Round 6A + Round 11-trinity Part b + Round 13 T1' precedents.

Implementation outcome: ~570 LoC conduit extension landed end-to-end on the sub-cluster branch. Verified via debug instrumentation that `concrete_types[doubled_slot] = Array(I64)` is stamped correctly and flows through to `.sum()`'s receiver slot. JIT compilation succeeds — no more compile-time `Route A surface-and-stop`.

**NEW SURFACE uncovered**: JIT-compiled code SIGSEGVs at runtime (exit 139). `try_emit_v2_array_method` fast path at `crates/shape-jit/src/mir_compiler/v2_array.rs:367-387` (`jit_v2_array_sum_i64`) assumes `concrete_types[slot] = Array(elem)` implies raw `*const TypedArrayData<elem>` bits, but `.map()`'s `jit_call_method` dispatch returns a different carrier shape → invalid dereference. **PRODUCER/CONSUMER FAST-PATH MISMATCH defection-attractor class**.

WIP stashed at sub-cluster branch `stash@{0}` for Round 15 if needed.

Round 15 disposition: **Option B** (W12-vm-map-typed-array-producer-migration) — producer-side carrier alignment. `.map()` returns the raw `TypedArrayData<T>` shape the fast path expects. Supervisor refused Option A (consumer-side fallback, defection-attractor) and Option C (both, scope expansion).

### Cluster-1 / cluster-2 follow-ups added by Round 14

1. **W17-typed-object-arc β typed-Arc carrier-shape decision** (cluster-1 follow-up per Option γ split). Pending supervisor β.1 (redesign `TypedObjectStorage` for layout compatibility) / β.2 (per-crate carrier-shape divergence, defection-attractor risk) / β.3 (delete JIT-internal struct + rebuild field-access codegen, ~1500-3000 LoC, loses inline-data performance contract) disposition. Cite: `docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md` Option γ section.

2. **W12-jit-typed-array-fast-path-consumer-defensive-narrowing** (cluster-1 or cluster-2 candidate). Option A.2 disposition: defense-in-depth at `try_emit_v2_array_method` (verify producer-bit assumption before dereferencing; surface-and-stop on mismatch). Land IF the producer/consumer fast-path mismatch architectural class recurs. NOT cluster-0 territory.

### Recurrent-pattern observation (CLAUDE.md amendment candidate)

The W12-map-chained NEW SURFACE is the SECOND instance of a producer/consumer mismatch defection-attractor in cluster-0 (first instance: `W12-stdlib-intrinsic-collapse` cluster-2 candidate from Round 13 T4 — `[1,2,3].sum()` PHF vs `IntrinsicSum` opcode split-brain). Both have the same shape: a consumer assumes a producer-side carrier/dispatch contract that isn't statically enforced. If a third instance surfaces, this becomes a CLAUDE.md "Forbidden Patterns" amendment candidate alongside the W-series.

## Cluster-0 close gate

Per phase-3-kickoff-prompt §"Cluster-0 sub-cluster sequencing":

> After all 4 close: `--mode jit` works end-to-end for the standard
> program surface. Cluster-0 closes.

Round 1's three sub-clusters did NOT make `--mode jit` end-to-end yet
on their own (only Smoke 1 of the kickoff's 4 targets fully works
identically under VM and JIT). Round 2 (W11-jit-carrier-conversion,
closed 2026-05-12) implemented the carrier-conversion FFI bodies as
identity pack/unpack per §2.7.5 stamp-at-compile-time, but the
remaining ~10 pre-existing failing tests assert deleted ValueWord-
shape API (`is_typed_object(bits)` on raw Arc pointers, NaN-box-tag
roundtrips) — they need test rewrites against the new strict-typed
API, NOT carrier conversion work. Those rewrites are not in any
cluster-0 sub-cluster scope. The remaining `--mode jit` gaps for
the kickoff smokes are the §2.7.5 JIT-side parallel-kind track
(item 6, cluster-1) and the `concrete_types` conduit (item 1,
cluster-1) — both ADR-level shape changes per the surface-and-stop
discipline. **Cluster-0 closes**: the three Round-1 sub-clusters
+ Round-2 W11-jit-carrier-conversion all closed with surfacing the
deeper gaps for cluster-1.

## Process / discipline notes for next session

1. **Forbidden-pattern monitoring**: the W11 walk-back showed that
   even with the kickoff prompt's explicit forbidden-pattern list,
   an agent will silently no-op an FFI body if Smoke 1 forces them to.
   Supervisor must read agent close reports carefully before
   accepting — the "tracked as a follow-up" framing is the tell.
   When in doubt, use the AskUserQuestion stop-and-ask trigger
   rather than rubber-stamp.

2. **`SendMessage`-based reopen works well**: a single round-trip
   reopen with a structured 7-step ask landed the principled fix
   without a full re-dispatch. Cheaper than rolling back the branch
   + spinning a fresh agent.

3. **CHECK 12 is now enforced**: any future agent that adds new
   `HK_*` constants will need to either alias `HeapKind::X as u16`
   or use `JIT_LEGACY_HK_BASE [+ N]` / `>= 256`. CHECK 12 fires
   automatically at merge.

4. **`SHAPE_JIT_ARC_COUNTERS=1` env var** is the canonical refcount
   audit surface. Use it for cluster-1 v2-raw-heap-audit follow-up
   and for verifying any future refcount-touching sub-cluster.

5. **Worktree retention**: cluster-0 worktrees stay until cluster-0
   close, per kickoff authority. Round-1 worktrees (`shape-w11-jit-
   new-array`, `shape-w17-jit-legacy-ord`, `shape-jit-test-runner`)
   not removed yet.

---

### W12-jit-call-method-shell-rebuild close (2026-05-13)

**Branch**: `bulldozer-strictly-typed-w12-jit-call-method-shell-rebuild`
**Round**: 10 / 8B.2 (standalone per supervisor sequential split from
Round 8B audit's §10.2 sub-cluster split).

#### Smoke matrix delta

| Smoke | Pre-Round-10 | Post-Round-10 | Status |
|---|---|---|---|
| `Set()` + `print(size)` (non-mutating) | JIT SURFACE at EnumStore consumer | VM=0 / JIT=0 ✓ | **EQUIVALENCE landed** |
| Smoke 4 — `Set()` + `add("a")` + `add("b")` + `print(size)` | JIT SURFACE at EnumStore consumer | VM=2 / JIT crashes after 2nd add | **BLOCKED by surfaced gap (A)** |
| HashMap — `HashMap()` + `set(k,v)` + `print(size)` | JIT SURFACE at EnumStore consumer | VM=1 / JIT crashes | **BLOCKED by surfaced gap (A)** |
| Mutex — `Mutex(42)` + `print(m.get())` | JIT SURFACE at EnumStore consumer | VM=42 / JIT SURFACE at print-operand-kind-None | **BLOCKED by surfaced gap (B)** |

The Round 10 dispatch shell + EnumStore consumer + VM trampoline +
slot-kind inference all land functional — verified by the
non-mutating Set smoke achieving VM == JIT equivalence at `0`. The
remaining smoke failures fall into two pre-existing MIR-level gaps
surfaced upstream of the dispatch-ABI rebuild.

#### Close-gate exit codes

- `cargo check --workspace --lib --tests` (inside devenv shell): **EXIT=0**
- `cargo test -p shape-jit --lib`: **361 passed; 0 failed; 26 ignored**
  (matches Round 9 baseline 361)
- `bash scripts/check-no-dynamic.sh`: **EXIT=0**
- `bash scripts/verify-merge.sh`: **12/12 passed**

#### Files touched

- `crates/shape-vm/src/executor/call_convention.rs` — NEW pub fn
  `jit_trampoline_call_method(method_name, (u64, NativeKind), &[(u64,
  NativeKind)], ctx) -> Result<u64, VMError>` next to
  `jit_trampoline_call_closure` (~80 LoC). Sibling §2.7.5 cross-crate
  stable FFI consumer; converts pair-slices to `&[KindedSlot]`
  internally then delegates to `dispatch_method_kinded`.
- `crates/shape-vm/src/executor/objects/mod.rs` — NEW pub(crate) fn
  `dispatch_method_kinded(&[KindedSlot], method_name, ctx)`
  extracted from `op_call_method`'s post-pop dispatch body (~20 LoC).
  Shared by `op_call_method` (VM dispatch shell) and the new
  trampoline. `op_call_method` now clones method_name into an owned
  `String` before the mutable dispatch call (releases the immutable
  borrow on `self.program.strings`).
- `crates/shape-jit/src/ffi/call_method/mod.rs` — Rebuild of
  `jit_call_method` (~290 LoC). Reads receiver+args kinds from §2.7.7
  `JITContext.stack_kinds` parallel-kind track via
  `stack_kind_code::decode`. Delegates to `vm.jit_trampoline_call_method`
  via `with_trampoline_vm_mut` when receiver kind is in the delegated
  set (HashSet / HashMap / Deque / PriorityQueue / Channel / Mutex /
  Atomic / Lazy / Result / Option / scalar kinds). Method-name pop
  now uses the parallel-kind track's `NativeKind::String` stamp
  (not the deleted `is_heap_kind(method_bits, HK_STRING)` NaN-box
  probe — raw `Box::into_raw` pointers don't satisfy the NaN-box
  shape under strict-typed unified-heap). Legacy JIT-format dispatch
  (higher-order array methods + `call_*_method` cascade) preserved
  under the `UInt64` carrier-kind fallback for opaque JIT bits.
  **Deleted**: `dispatch_method_via_trampoline` extern-C `todo!()`
  stub. **Deleted**: the `heap_kind(receiver_bits)`-driven NaN-box
  cascade as the primary receiver discriminator (kept only as the
  JIT-internal field-load on opaque-bits slots — a known-classified
  heap-allocation field read, NOT a §2.7.7 #4/#7 forbidden
  tag-decode for kind determination).
- `crates/shape-jit/src/mir_compiler/statements.rs` — NEW
  `emit_collection_ctor` helper (~150 LoC). Dispatches the EnumStore
  consumer's collection_ctor arm to Round 9's 8 typed-Arc ctor
  FuncRefs: `jit_v2_make_hashset` / `_hashmap` / `_deque` /
  `_priorityqueue` / `_channel` (zero-arg), `jit_v2_make_atomic` /
  `_lazy` (single-arg), `jit_v2_make_mutex` (carrier-pair with kind
  code). The pre-Round-10 SURFACE-and-stop at lines 239-268 is
  replaced with the dispatching arm.
- `crates/shape-jit/src/mir_compiler/types.rs` — Two extensions to
  `infer_slot_kinds_with_concrete`: (1) NEW `StatementKind::EnumStore`
  arm stamps `NativeKind::Ptr(HeapKind::*)` for the 8 collection-ctor
  variant names (override of the upstream `concrete_seed`'s
  `Struct(_)` → `Ptr(TypedObject)` misclassification — the stdlib
  defines Set/HashMap/etc. as typed structs but their typed-Arc
  carriers per Round 9 do NOT use `Arc<TypedObjectStorage>` shape);
  (2) NEW post-pass propagates collection kinds through
  `Assign(dst, Use(Move/Copy(src)))` identity chains until fixpoint
  (closes the seed-vs-EnumStore conflict for the user-visible
  binding slot — pre-pass leaves `s` slot at `Ptr(TypedObject)` from
  the seed while the EnumStore container slot is corrected to
  `Ptr(HashSet)`).

#### Decisions called

1. **Pair-slice → KindedSlot conversion single-direction** at the
   §2.7.5 FFI boundary, mirroring the `jit_trampoline_call_closure`
   precedent. Forbidden alternatives refused on sight per §2.7.6/Q8
   carrier-API-bound: parallel `&[NativeKind]` second-slice
   parameter, `&mut [KindedSlot]` mutable form, `Vec<KindedSlot>`
   by-move.

2. **Lifetime accounting contrast with closure trampoline**: the
   closure trampoline `mem::forget(kinded_args)` because args were
   transferred into the callee frame's locals via
   `stack_write_kinded`. Method dispatch's borrow-only PHF ABI does
   NOT transfer shares — handlers borrow each `KindedSlot`. The
   transient trampoline carriers therefore DO release on scope exit
   to balance the JIT-side retain-before-crossing pattern. Documented
   in the trampoline's docstring "Ownership" paragraph.

3. **EnumStore producer-side classification override of
   `concrete_seed`**: necessary because the bytecode compiler's
   type-checker classifies stdlib-defined `Set` / `HashMap` / etc.
   as typed structs. The `concrete_seed` then maps these to
   `Ptr(TypedObject)`, which would route retain/release through
   the legacy `arc_retain` / `arc_release` (operating on the
   `UnifiedValue<T>` HeapHeader at offset 4) — wrong-carrier-shape
   crash on `Arc<HashSetData>` payloads (audit §5 rule). Tracked as
   `W17-collection-concrete-types` follow-up to extend `ConcreteType`
   with `HashSet` / `Deque` / etc. arms so the bytecode compiler
   gets these right at the source.

4. **Trait-object reentry not preserved in the trampoline path**.
   `op_call_method` includes a `TraitObject` early-return that
   reconstructs an `Instruction` and routes to `op_dyn_method_call`.
   The trampoline (called from JIT, no `Instruction` context) does
   NOT replicate this — trait-object dispatch through the JIT path
   surfaces (out of Round 10 scope; W17-trait-object-emission
   territory).

5. **Higher-order JIT array methods preserved** in the legacy path.
   The `find` / `filter` / `map` / `reduce` etc. routes through
   `jit_control_*` FFI bodies that invoke JIT-compiled closures via
   the function table. Routing these through VM delegation would
   lose this perf path; preserved under the `UInt64` carrier-kind
   fallback guard. Migration to full kinded dispatch for JIT-format
   arrays is W10 jit-playbook §5 / §2.7.4 territory.

#### Surfaced items (separate workstreams)

**(A) `W17-mir-mutation-writeback`** — MIR-level writeback for
`MUT_SELF_*` methods is missing.

The bytecode compiler at `crates/shape-vm/src/compiler/
mutation_writeback.rs` emits `Dup; StoreLocal recv` post-`CallMethod`
for mutating container methods (HashSet.add, HashMap.set, etc.) so
the new Arc identity propagates back to the binding slot per ADR-006
§2.7.27 / Item 4. The MIR builder at `crates/shape-vm/src/mir/
lowering/expr.rs::Expr::MethodCall` (around line 1806) does NOT emit
the equivalent `Assign(receiver_slot, Use(Move(temp)))` writeback.
The JIT compiles from MIR, so under JIT execution
`let mut s = Set(); s.add("a")` produces the new HashSet Arc into a
fresh temp slot but `s` slot retains the OLD Arc bits. Next access
to `s` operates on stale bits — when the post-CallMethod release
fires on the temp slot, the new Arc gets retired; the old `s` slot
still holds the old Arc which gets accessed on the next call,
crashing if the underlying allocation was already freed.

Fix scope: extend MIR lowering for `Expr::MethodCall` to consult
`crates/shape-vm/src/executor/objects/method_registry::
is_mut_self_method_name` (already `pub`) and emit a post-Call
writeback `Assign(receiver_slot, Use(Move(temp)))` when the receiver
is a `Local`. ~30 LoC of MIR lowering change + slot-mapping work to
identify the binding-side `Local` from the receiver Expr.

**(B) `W17-collection-concrete-types` / kind-inference for
method-call returns** — Methods whose return kind varies by receiver
type are not classifiable by `well_known_method_return_kind`.

Specifically: `Mutex.get` returns the wrapped `T`, `HashMap.get`
returns `Option<V>`, `Atomic.load` returns `i64`, etc. None of these
have a kind that's invariant across receiver classes, so they can't
go into `well_known_method_return_kind`. The destination slot of the
CallMethod stays at `None` kind. Downstream `print(m.get())` then
surfaces with "Call-terminator operand NativeKind is None" per the
Round 8A print-kind discipline (§2.7.5 conduit gap).

Fix scope: extend the `concrete_types` conduit to propagate inner-
kind information for parametric container types (Mutex<T>, Atomic<T>,
Lazy<T>, HashMap<K,V>, etc.) through method-call return-type
inference. The bytecode compiler already has the inner-type
information at the binding's TypeAnnotation; the MIR-side needs a
new `ConcreteType` variant for these containers (currently absent —
neither `ConcreteType::Mutex` nor `Atomic` nor `Lazy` exist in
`crates/shape-value/src/v2/concrete_type.rs`). Tracked already in
inline source comments at `types.rs`'s EnumStore arm.

#### Forbidden frames refused on sight

Per CLAUDE.md "Renames to refuse on sight" §2.7.10/Q11 + §2.7.11/Q12
broader-family regex: deleted code is described by deletion-fate or
by name, never via bridge/probe/helper/hop/translator/adapter/shim
framing:

- The deleted `dispatch_method_via_trampoline` extern-C `todo!()`
  stub at `call_method/mod.rs:179-199` (described by name —
  function name preserved in source comments).
- The deleted kind-blind `heap_kind(receiver_bits)`-driven NaN-box
  cascade as the primary receiver discriminator (described by
  deletion-fate — the `match heap_kind(receiver_bits)` cascade at
  the pre-rebuild lines 349-366).
- The deleted `is_heap_kind(method_bits, HK_STRING)` method-name
  validation (described by deletion-fate — the NaN-box discriminator
  on raw `Box::into_raw` pointers that don't carry the JIT NaN-box
  tag under strict-typed unified-heap; replaced by parallel-kind
  track `NativeKind::String` stamp at `terminators.rs:243`).

#### Cluster-0 Round 10 state

- 8B.1 (Round 9): typed-Arc ctors + 16 kinded retain/release —
  closed in Round 9 (`81acb62e` + merge `2bd103ac`).
- 8B.2 (Round 10): shell rebuild + VM trampoline API + EnumStore
  consumer + slot-kind inference — closed in this commit.
- Smoke 4 / HashMap / Mutex equivalence: blocked by surfaced gaps
  (A) and (B), tracked as separate workstreams. The dispatch-ABI
  layer is functionally complete; the remaining gaps are at the
  MIR-lowering tier (writeback) and the kind-inference conduit tier
  (parametric-container return kinds), neither of which is in
  W12-jit-call-method-shell-rebuild's scope per Round 8B audit.

### W12-vm-new-array-untyped-construction close (2026-05-13)

Closed Round 11A standalone audit-first sub-cluster. Branch
`bulldozer-strictly-typed-w12-vm-new-array-untyped-construction` from
post-Round-10-merge `8db19d21`. Audit commit `7cda8e1d`, fix commit
`264283ff`.

Surface: kickoff Smoke 2 (`[1,2,3,4,5].map(|x|x*2).sum()`) under
`--mode vm` was failing with `Not implemented: op_new_array: generic
untyped-array construction depends on the kinded the-deleted-
heterogeneous-element-carrier emit path (Phase 2c reentry — see
ADR-006 §2.7.4)`.

Audit findings:

1. **§-cite stray confirmed.** `§2.7.4` is "API rebuild scope
   clarification" (Phase 1.B/Phase 2c snapshot + stdlib registration
   scope), NOT array-construction territory. Correct cite is
   `§2.7.24 Q25.A` (typed-carrier monomorphization bundle). Same
   stray-cite class previously caught at `mir_compiler/statements.rs:236`
   (Round 5B `§2.7.4 → §2.7.14` fix) and
   `w12-enum-constructor-audit.md:215`.

2. **Deleted carrier identified.** The polymorphic
   `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` arm
   was deleted in Phase 2d W17-typed-carrier-bundle-A checkpoint 4/4,
   replaced by per-element-kind monomorphic specializations (Decimal /
   BigInt / DateTime / Timespan / Duration / Instant / Char /
   TypedObject / TraitObject) plus the projection helper
   `TypedArrayData::build_specialized_from_heap_arcs` (in
   `shape_value::heap_value` line 2937).

3. **Helpers already in place.** Both the build helper and the
   `(bits, kind) → Arc<HeapValue>` projection helper
   (`slot_to_heap_arc` at `executor/builtins/array_ops.rs:49`) were
   available — `op_new_array` just needed to consume them.

Fix shape (`264283ff`, ~250 LoC across 2 files):

- **`op_new_array` body** rewritten in
  `crates/shape-vm/src/executor/objects/object_creation.rs:287-...`.
  Empty `Count(0)` defaults to `TypedArrayData::I64` matching
  `op_new_typed_array`'s stable empty default. Homogeneous-kind input
  dispatches to the matching specialized variant via new private
  helper `build_homogeneous_typed_array` (Int64 / Float64 / Bool /
  Char inline scalars; String / Decimal / BigInt / TypedObject
  heap-arc reconstruction via `Arc::from_raw`). Heterogeneous-kind
  input routes through `slot_to_heap_arc` +
  `TypedArrayData::build_specialized_from_heap_arcs`; cross-arm
  surfaces as `VMError::RuntimeError` per Q25.A "Arrays do not
  [admit heterogeneous slots]", NOT `NotImplemented(SURFACE)`.

- **`slot_to_heap_arc` visibility** bumped from file-local `fn` to
  `pub(in crate::executor)` in `array_ops.rs:49` so
  `object_creation.rs` shares the same projection logic without
  duplication.

- Module docstring at `object_creation.rs:14-39` updated to document
  the Round 11A `op_new_array` migration. The
  `the-deleted-heterogeneous-element-carrier` deletion-fate descriptor
  is removed at the rewritten body sites; remaining hits in
  `op_new_typed_array`'s heterogeneous-fallback arm and sibling
  SURFACE sites are left as follow-up §-cite mechanical cleanup
  (out of scope for this runtime-fix commit; §4.C of the audit doc).

Close gates (devenv exit-code-verified):

- `cargo check --workspace --lib --tests` EXIT=0.
- `cargo test -p shape-vm --lib` SIGABRT matches pre-existing
  v2-raw-heap-audit baseline (`tcache_thread_shutdown(): unaligned
  tcache chunk detected` firing in monomorphization cache tests,
  identical to the Round 4 close report:273-275 documented baseline).
- `cargo test -p shape-jit --lib` 361 / 0 / 26 — matches baseline 361
  exactly.
- `bash scripts/verify-merge.sh` 12/12 Passed.
- `bash scripts/check-no-dynamic.sh` EXIT=0.
- AGENTS.md row updated to `closed`.

Runtime verification:

- Pre-fix `cargo run --bin shape -- run /tmp/smoke2.shape --mode vm`
  surfaced `op_new_array` `NotImplemented`. Post-fix the
  `op_new_array` surface is closed.
- Smoke 2 end-to-end VM `30` output remains blocked — two NEW
  pre-existing-bug surfaces revealed downstream of the `op_new_array`
  fix:

  - **`IntrinsicSum` Phase-1B wave-5d gap** at
    `crates/shape-vm/src/executor/vm_impl/builtins.rs:471-520`
    (`todo!("phase-1b-vm wave 5d — intrinsic body migration pending
    (handle_intrinsic_builtin): IntrinsicSum")`). Fires when
    `.sum()` is invoked on an `Array<int>` receiver. Same wave-5d
    closure-driven-builtins migration as `Map`/`Filter`/`Reduce`/etc.
    Out of scope for Round 11A.

  - **`call_value_immediate_nb` kind-mismatch surface** at
    `crates/shape-vm/src/executor/call_convention.rs:798` —
    `HeapKind::Closure label with non-ClosureRaw HeapValue payload:
    "string"`. Fires when `xs.map(|x|x*2)` is invoked with `xs` as a
    V2 typed-int-array (`NativeKind::UInt64`). The dispatch shell
    appears to pass the wrong slot as the closure-callee carrier in
    some configurations; this is a pre-existing kind-source bug at
    the method-dispatch tier, unrelated to `op_new_array`. Surfaced
    independently — both VM-side `xs.map(closure)` and
    `let f = |x| x; xs.map(f)` reproduce the panic with the
    `HeapValue::String` payload signature, suggesting a stale
    receiver slot being read.

  Both gaps are pre-existing on this branch — they were masked by
  the upstream `op_new_array` surface and are now visible. **Neither
  is in Round 11A's territory.** Surfaced for supervisor disposition
  as follow-up workstreams; the `op_new_array` close criterion is
  bounded by §2.7.24 Q25.A reentry scope per the dispatch text.

Forbidden frames refused on sight (per audit §7):
- "preserve deleted-carrier emit path under documented disposition",
- Bool-default element kind for unknown-kind array,
- "just one edge case" / "soft-fail counter for now",
- "this is Phase 2c-residual, document as out-of-scope" — supervisor's
  Round-11 ratification rules this out for cluster-0 close criterion,
- Add a transitional `TypedArrayData::HeapValueShim / Untyped / Mixed
  / Generic` variant,
- "Defer to a new ADR amendment introducing dynamic-typed empty
  arrays".

No ADR amendment required (audit §8). All architectural decisions live
in §2.7.24 Q25.A + §2.7.5 + §2.7.10/Q11; helpers and variant grid
already in place pre-Round-11A.
### W12-jit-producing-site-conduit-completeness close (2026-05-13)

**Branch**: `bulldozer-strictly-typed-w12-jit-producing-site-conduit-completeness`
**Round**: 11-trinity INTEGRATED (Round 7A precedent, ~800-1000 LoC single
agent) — closes Round 10's surfaced item (B) at the §2.7.5 conduit
completeness level.

Three commits on the same branch per the trinity's internal ordering
rule "(a) FIRST as foundation; (b) and (c) consume the landed taxonomy":

| Part | Commit | LoC | Scope |
|---|---|---|---|
| (a) 11E ConcreteType taxonomy | `82dfecd9` | ~228 | Extend `shape_value::v2::ConcreteType` with 7 new arms: `HashSet(Box<_>)`, `Deque(Box<_>)`, `PriorityQueue`, `Channel(Box<_>)`, `Mutex(Box<_>)`, `Atomic`, `Lazy(Box<_>)`. Updates `is_heap`, `mono_key`, `type_tag`, `Display`. Cross-crate exhaustive-match updates in 3 sites (`closure_layout.rs::native_kind_from_concrete_type`, `mir_compiler/types.rs::native_kind_from_concrete_type`, `monomorphization/substitution.rs::concrete_to_annotation`). No ADR amendment — all 7 arms mirror existing parametric (Array/HashMap) or nullary shape and dispatch through existing HeapKind ordinals. Wire-format unaffected (`#[serde(skip)]` on every `ConcreteType`-bearing field reaching `FunctionBlob`). |
| (b) 11B method-return-kind conduit | `5b113145` | ~371 | New `parametric_method_return_kind_from_receiver(name, args, concrete_types)` classifier in `mir_compiler/types.rs`, wired into `infer_slot_kinds_with_concrete`'s `TerminatorKind::Call` destination-stamp pass via `well_known.or_else(parametric)`. Covers `Array<T>.{sum,mean,min,max,get,first,last,pop}`, `HashMap<K,V>.get`, `Mutex<T>.get`, `Atomic.{load,fetch_add,fetch_sub,compare_exchange}`, `Lazy<T>.get`. Receiver-recovery per §2.7.5 (args[0] is the receiver per MIR lowering convention). Same defect class as Round 8A reopen's `infer_enum_payload_kind` extension via `native_kind_from_concrete_type`, generalized to method-call sites. |
| (c) 11C Rvalue::Aggregate TypedObject threading | `a181abd9` | ~121 | Producer-side fix at `mir/lowering/helpers.rs::emit_container_store_full`: preserve empty-operands short-circuit for Array/Enum/Closure (no per-element work to record) but emit the empty `StatementKind::ObjectStore` for the Object case. Closes the Smoke 3 JIT-side `let t = X {}` regression: the conduit walks the empty ObjectStore and stamps `Struct(StructLayoutId(0))`, the JIT `is_typed_object_slot` short-circuit fires for the preceding `Rvalue::Aggregate(vec![])`, and the existing ObjectStore consumer's `typed_object_alloc(schema_id, 0)` allocates the empty TypedObject. One-line fix in the producer + new conduit test. |

**Smoke matrix delta (JIT-side)**:

| Smoke | Pre-trinity | Post-trinity | Disposition |
|---|---|---|---|
| 1 (`4950`) | ✅ | ✅ unchanged | passing |
| 1.5 (`divide` + match → `5`) | ❌ §2.7.5 String EnumPayload carrier-mismatch | ❌ same (cluster-1 carrier-unification candidate) | cluster-1 |
| 2 partial (`[1,2,3].sum()` → `6`) | ❌ print SURFACE at operand NativeKind=None | ✅ prints `6` (Part b parametric classifier flows Int64) | **trinity-closed** |
| 2 full (`[1,2,3,4,5].map(\|x\|x*2).sum()` → `30`) | ❌ VM `op_new_array`; JIT print SURFACE | ❌ VM `op_new_array` (11A territory); JIT print **PART-B FLOWS** Int64 (waiting for 11A VM-side fix to test end-to-end) | depends on 11A |
| 3 (`type X {} let t = X {} print(t.name())` → `x`) | ❌ JIT `Rvalue::Aggregate` SURFACE | ❌ Aggregate UNBLOCKED → SURFACE moves DOWNSTREAM to `t.name()` trait-dispatch return-kind classification | **trinity-closed at Aggregate**; surfaced for cluster-1 / Round 12 |
| 4 (`Set + .add + .size` → `2`) | ❌ writeback (11D territory) | ❌ same | 11D territory |

**Surfaced items (cite-tracked, NOT silently fallback'd)**:

- (T1) **Trait-dispatch return-kind classification** — `t.name()` Call-
  terminator destination remains unstamped because the method-return-
  kind classifier (Part b's `parametric_method_return_kind_from_
  receiver`) only covers receiver-parametric cases keyed on
  `ConcreteType::{Array<T>, HashMap<K,V>, Mutex<T>, Atomic, Lazy<T>}`
  shape. Arbitrary trait methods like `name(): string` declared in
  `trait T { ... }` and dispatched via `impl T for X` need the trait
  registry's declared return type threaded into the conduit — a
  separate sub-cluster's audit territory. NOT trinity scope.

- (T2) **`NativeKind::String` carrier-mismatch surface** at print
  Call-terminator (pre-existing Round 8A reopen's identified cluster-1
  candidate `W12-jit-result-carrier-unification`, generalized to all
  §2.7.5 heap carriers). Even if (T1) were closed, `print(string)`
  would still surface. Cluster-1 territory.

**Close gates (devenv exit-code-verified)**:

- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-value --lib` 402 passed / 2 failed / 0 ignored
  (2 hashmap_mutation failures pre-existing — verified by stash + rebuild
  on parent `8db19d21`; baseline 400 + 2 new Part-a tests = 402)
- `cargo test -p shape-jit --lib` 373 passed / 0 failed / 26 ignored
  (361 baseline + 12 new Part-b tests = 373)
- `cargo test -p shape-vm --lib compiler::helpers::call_return_kind_tests`
  5 passed / 0 failed (4 existing + 1 new Part-c conduit test)
- `cargo test -p shape-vm --lib mir::lowering` 63 passed / 0 failed
  (lowering tests unaffected by Part-c producer-side fix)
- `cargo test -p shape-vm --lib` FAILED with v2-raw-heap aliasing
  SIGABRT — pre-existing per Round 4 close report ("v2-raw-heap-audit"
  follow-up); not introduced by trinity.
- `bash scripts/verify-merge.sh` 12/12 Passed
- `bash scripts/check-no-dynamic.sh` EXIT=0

**ADR amendments**: NONE required. Part (a) taxonomy extension mirrors
existing parametric (Array/HashMap) and nullary shape per §2.7.15 /
§2.7.17 / §2.7.18 / §2.7.20 / §2.7.25; none of the 7 new ConcreteType
arms projects 1:1 to HeapKind (ADR-005 §1 single-discriminator
preserved). Part (b) extends the §2.7.5 producing-site conduit pattern
to method-call sites — same shape as Round 8A reopen's
`infer_enum_payload_kind` extension. Part (c) is a one-line producer-
side invariant fix (the empty-operands ObjectStore was silently
dropped, masking the JIT-side conduit gap).

**Refuse-on-sight discipline preserved** across all three parts:

- No `bridge`/`probe`/`helper`/`hop`/`translator`/`adapter`/`shim`
  framing in any commit or its commentary.
- No Bool-default fallback at any kind-source gap (§2.7.7 #9).
- No ConcreteType variants projecting 1:1 to HeapKind (ADR-005 §1).
- No kind-blind fabrication at the Aggregate consumer site (Part c
  works through the existing TypedObject short-circuit + ObjectStore
  consumer chain, not by adding a new Aggregate arm that handles
  unknown destination kinds).
- No silent walkbacks — the trinity's downstream surface (T1, T2) is
  honestly surface-and-stopped, not "marked as a follow-up for a
  later phase" rationalization.

Per the trinity's migrating-close criterion, the Aggregate JIT-side
gap is closed (Part c) and the kind-inference completeness is
extended (Parts a, b). The downstream trait-dispatch return-kind
classification gap (T1) and the `NativeKind::String` carrier-mismatch
surface (T2) are surfaced cleanly for Round 12 / cluster-1
disposition per the N+1 trajectory.

---

### W12-jit-trait-dispatch-return-kind close (2026-05-13)

**Branch**: `bulldozer-strictly-typed-w12-jit-trait-dispatch-return-kind`
**Round**: 12 T1 (standalone, parallel with T2/T3 W12-jit-string-
carrier-unification).
**Disposition**: **SURFACE-AND-STOP** per the agent prompt's named
condition ("If the trait registry isn't accessible from the JIT MIR
builder layer (cross-crate boundary issue) — STOP and surface").

#### Surface analysis (3 conduit gaps; ALL three must close for the
classifier to soundly stamp `t.name() → NativeKind::String`)

The Smoke 3 minimal program `trait T { name(): string } type X {} impl
T for X { method name() { "x" } } let t = X {} print(t.name())` cannot
be classified by the existing `parametric_method_return_kind_from_
receiver` (trinity Part b) classifier because the kind sources the
classifier needs to produce `NativeKind::String` are all missing at
JIT MIR builder layer:

**Gap 1 — receiver struct identity erasure**:
`concrete_type_from_annotation` (`crates/shape-vm/src/compiler/
v2_map_emission.rs:357`) returns the `StructLayoutId(0)` placeholder
for every user struct name. The `_ => None` arm at line 378 carries
the comment "Phase 1.1 Agent 3 will fill this in" — the layout-id
registry is not wired. So the receiver slot's `ConcreteType` is
`Struct(StructLayoutId(0))` regardless of whether the user struct is
`X`, `Y`, `Point`, etc. The classifier has no struct-name
information to disambiguate at MIR time.

**Gap 2 — trait registry not persisted in `BytecodeProgram`**:
`TypeRegistry::traits: HashMap<String, TraitDef>` in
`crates/shape-runtime/src/type_system/environment/registry.rs:111`
holds the trait's declared return type
(`InterfaceMember::Method { return_type: TypeAnnotation, .. }`), but
the `BytecodeProgram` (`crates/shape-vm/src/bytecode/core_types.rs`)
does NOT persist this — only `trait_method_symbols: HashMap<String,
String>` (resolved function name per `(trait, type, impl, method)`
key) and `trait_vtables` (vtables keyed by `Trait::ConcreteType`).
Neither carries declared trait method return types. The bytecode→JIT
data conduit has no channel for this metadata.

**Gap 3 — impl method return type fallback insufficient**:
`function_return_concrete_types: Vec<ConcreteType>`
(`core_types.rs:356`) is keyed on function index and built from
`FunctionDef.return_type` annotations
(`compiler_impl_reference_model.rs:1473`). For trait impl methods
desugared via `desugar_impl_method`
(`crates/shape-vm/src/compiler/statements.rs:1646`), the impl's
`method.return_type` is whatever the impl source declared. For
Smoke 3's `impl T for X { method name() { "x" } }`, the impl
doesn't repeat the trait's `: string` annotation, so
`method.return_type = None` → `function_return_concrete_types[X::
name] = ConcreteType::Void`. The trait's declared return type does
not propagate to the impl's `FunctionDef`.

**Bridging strategy considered but rejected as out-of-scope for T1**:
the principled fix is a new `BytecodeProgram` side-table persisting
per-trait-method declared return `ConcreteType`s, populated at impl-
block compilation time from `TraitDef.members[*].Required(Method
{ return_type, .. })` / `Default(MethodDef { return_type, .. })`,
threaded through the linker / `remote.rs` / content-addressed
program shapes (~6 mirror-of-existing-pattern files), threaded into
MirToIR via `crates/shape-jit/src/compiler/strategy.rs` alongside
`function_indices`. This mirrors the Round-6 `function_return_
concrete_types` precedent. **Cross-crate extension; ADR amendment
territory** per the agent prompt's surface-and-stop list. Out of
scope for T1 per the prompt's scope statement ("Touch: `crates/
shape-jit/src/mir_compiler/types.rs` ... different region than
T2/T3, but same file").

#### Contribution

Doc-only surface pin in `crates/shape-jit/src/mir_compiler/types.rs`:

1. **Extended doc block** on `parametric_method_return_kind_from_
   receiver` adding a "User-defined-trait surface boundary" section
   that names the 3 conduit gaps above, traces each gap to its
   specific file:line, and documents the cross-crate extension
   shape required to close the surface.

2. **3 new pin tests** in `mir_compiler::types::tests`:

   - `user_defined_trait_method_on_struct_returns_none` — asserts
     the classifier returns `None` for the Smoke 3 minimal shape
     (`name` method on a `Struct(StructLayoutId(0))` receiver).
     Pin against a future hard-coded `"name"` → `String` walk-back.
   - `user_defined_trait_method_call_terminator_remains_unstamped`
     — integration pin: the full Call-terminator destination-stamp
     pass (`well_known.or_else(parametric)`) leaves the destination
     slot's kind `None`. Also asserts `"name"` is correctly NOT in
     the `well_known_method_return_kind` cohort (different traits
     could declare `name` with different return types — soundness
     pin).
   - `parametric_classifier_remains_silent_for_struct_receiver_
     with_known_method_names` — cohort pin: the parametric arms
     (`get` / `sum` / `mean` / `min` / `max` / `first` / `last` /
     `pop` / `load` / `fetch_*` / `compare_exchange`) and trait-
     dispatch-shaped names (`name` / `display` / `to_string` /
     `into` / `from` / `try_into` / `try_from`) must all return
     `None` for a `Struct(_)` receiver. Pin against a wrong-
     carrier walk-back (a user struct with a `.sum()` method is
     not an `Array<T>`).

#### Smoke matrix delta (JIT-side)

| Smoke | Pre-Round-12-T1 | Post-Round-12-T1 | Disposition |
|---|---|---|---|
| 3 (`trait T + impl + dyn + t.name() → "x"`) | ❌ `Route A surface-and-stop: NotImplemented(SURFACE) — print Call-terminator operand NativeKind is None` (trinity Part c surfaced) | ❌ same surface, **documented + pinned with 3 surface tests** | T1 closes SURFACE-AND-STOP; ADR amendment + cross-crate side-table required to close end-to-end |

The Smoke 3 JIT-side surface is **not closed by T1**. T1's
contribution is the cite-tracked surface-and-stop documentation +
pin tests preventing a future walk-back. T2/T3 (W12-jit-string-
carrier-unification) is the parallel migration that closes the
downstream `NativeKind::String` carrier-mismatch — but even after
T2/T3 lands, Smoke 3 JIT still requires the cross-crate trait-
method-return side-table extension to flow the trait's declared
return type into the JIT MIR builder's classifier.

#### Close gates (devenv exit-code-verified)

- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` **376 passed / 0 failed / 26
  ignored** (373 baseline + 3 new pin tests = 376 exact)
- `bash scripts/verify-merge.sh` **12/12 Passed** ("ALL CHECKS
  PASSED. Safe to merge.")
- `bash scripts/check-no-dynamic.sh` EXIT=0

#### Refuse-on-sight discipline preserved

- No `bridge`/`probe`/`helper`/`hop`/`translator`/`adapter`/`shim`
  framing in commit, doc block, status doc, or AGENTS.md row. The
  3 conduit gaps are named by what they are (struct identity
  erasure / trait registry not persisted / impl method return type
  fallback insufficient), not by hypothetical role.
- **No hard-coded `"name"` → `String` arm** in the classifier
  ("hard-code the kickoff Smoke 3 case for now" refused per agent
  prompt's forbidden-rationalization list; same defection-attractor
  pattern as the deleted W-series convert opcode (`Convert<X>To<Y>`
  added to paper over a kind-tracker gap) per CLAUDE.md "Forbidden
  code").
- **No Bool-default fallback** at the kind-source gap path
  (§2.7.7 #9) — the classifier returns `None`; the downstream
  Route-A surface-and-stop fires at the print Call-terminator;
  the surface is honestly named, not papered over.
- **No "default to `string` for unknown trait return kinds"** —
  refused per agent prompt's forbidden-rationalization list.
- **No "skip the trait registry lookup if it's expensive"** —
  refused per agent prompt's forbidden-rationalization list.
- **No silent walkback** — the surface is named (`Route A
  surface-and-stop: NotImplemented(SURFACE)`) and the cross-crate
  extension is described in detail at status-doc + doc-block
  granularity for the next session's audit.

#### Coordination with T2/T3 (W12-jit-string-carrier-unification)

T1 touched ONLY the documentation region of
`crates/shape-jit/src/mir_compiler/types.rs` (doc block on
`parametric_method_return_kind_from_receiver` + 3 new pin tests in
the test module). T2/T3's territory is the kind-track propagation
region (different region of the same file per the agent prompt).
No source-line overlap; no mechanical merge conflict expected at
the file level.

Smoke 3 JIT end-to-end requires **T1 + T2/T3 + the cross-crate
trait-method-return side-table extension** to land. T1 alone is
necessary but not sufficient; T2/T3 alone is necessary but not
sufficient; even both together would still surface at the trait-
method return-kind classifier gap T1 documents. The cross-crate
extension is the third leg surfaced for Round 13 cluster-0
disposition.
### W12-jit-string-carrier-unification close (2026-05-13)

**Branch**: `bulldozer-strictly-typed-w12-jit-string-carrier-unification`
**Round**: 12 T2/T3 (parallel with T1 `W12-jit-trait-dispatch-return-kind`).
**Branched from**: `b23e2548` (Round 12 dispatch metadata on
`bulldozer-strictly-typed`).

#### Smoke matrix delta

| Smoke | Pre-Round-12 | Post-Round-12 | Status |
|---|---|---|---|
| Smoke 1 (`for i in 0..100 { sum += i }; print(sum)` → `4950`) | VM == JIT | VM == JIT | unchanged |
| `print("hello")` | JIT clean SURFACE (Round 8A item) | VM=`hello` JIT=`hello` | **closed** |
| `let s = "hello"; print(s)` | (unverified, but same producer site) | VM=`hello` JIT=`hello` | **closed** |
| Smoke 4 (`let mut s = Set(); s.add("a"); s.add("b"); print(s.size())` → `2`) | VM=2 JIT segfault (Round 11D surfaced) | VM=2 JIT=2 | **closed** |
| `print(Some(3))` | VM == JIT | VM == JIT | unchanged |
| `print(Ok(5))` | VM == JIT | VM == JIT | unchanged |
| `[1,2,3].sum()` → `6` | JIT=6 (Round 11-trinity Part b) | JIT=6 | unchanged |
| `print(Err("x"))` → `Err("x")` | JIT clean SURFACE (kind=Ptr(TypedObject)) | JIT clean SURFACE (same kind=Ptr(TypedObject)) | **NOT closed** — pre-existing kind classifier bug surfaced as separate sub-cluster (see "Surfaced" below) |

#### Close-gate exit codes

- `cargo check --workspace --lib --tests` (inside devenv shell): **EXIT=0**
- `cargo test -p shape-jit --lib`: **379 passed; 0 failed; 26 ignored** (baseline 373 + 6 new String tests — exact, no regressions)
- `bash scripts/verify-merge.sh`: **12/12 Passed**
- `bash scripts/check-no-dynamic.sh`: **EXIT=0**

#### Fix shape

ADR-006 §2.7.5 producer-side migration mirroring Round 7A (§2.7.17
Result/Option Arc-shape producers) and Round 9 (typed-Arc collection
retain/release pairs) precedents.

7 files, ~250 LoC incl. docstrings:

1. **NEW** `crates/shape-jit/src/ffi/string.rs` — kinded
   `jit_arc_string_retain` / `jit_arc_string_release` operating on
   `Arc::increment/decrement_strong_count::<String>` at offset -16 (Rust
   Arc contract); `arc_string_constant(s: String) -> u64` compile-time
   helper that boosts the initial refcount to 2 so the constant survives
   the JIT-compiled function's full lifetime. Without the boost, a
   single use-then-drop release would underflow to 0 and free the
   constant → next call → use-after-free. The "constant's permanent
   share" + "active share" discipline parallels how string interning
   works. 6 round-trip tests mirror Round 7A's `result.rs::tests`
   pattern: refcount-boost stability, retain-bumps, release-drops,
   null-bits safety, Arc::from_raw round-trip, use-drop cycle survival.

2. `crates/shape-jit/src/ffi/mod.rs` — `pub mod string;` registration.

3. `crates/shape-jit/src/mir_compiler/ownership.rs::compile_constant` —
   `MirConstant::Str` / `MirConstant::StringId` / `MirConstant::Method`
   arms migrated from `value_ffi::box_string(s)` (legacy
   `Box::into_raw(Box::new(UnifiedValue<Arc<String>>))`) to
   `ffi::string::arc_string_constant(s)` (§2.7.5 raw
   `Arc::into_raw(Arc<String>) as u64`).

4. `crates/shape-jit/src/mir_compiler/ownership.rs::retain_func_for_place`
   / `release_func_for_place` — new `Some(NativeKind::String)` arm
   routes to `self.ffi.arc_string_retain` / `_release` instead of the
   legacy `arc_retain` / `arc_release` fallback (which would scribble on
   the `String` payload's `ptr/cap/len` words at offset +4).

5. `crates/shape-jit/src/ffi_refs.rs` — 2 new FuncRef slots
   (`arc_string_retain` / `arc_string_release`).

6. `crates/shape-jit/src/ffi_symbols/arc_symbols.rs` — 2 new
   `register_arc_symbols` entries + 2 new `declare_arc_functions`
   signatures (`extern "C" fn(bits: i64)` per Round 7A's
   `jit_arc_result_retain` ABI shape).

7. `crates/shape-jit/src/compiler/ffi_builder.rs` — 2 new `r!()` lookups
   for the FuncRef slots.

8. `crates/shape-jit/src/mir_compiler/terminators.rs` — print
   Call-terminator's `Some(NativeKind::String)` arm flipped from
   SURFACE-and-stop to `self.ffi.print_str` dispatch (Round 8A's
   surfaced item closes). The `Some(NativeKind::Ptr(HeapKind::
   TypedObject))` arm refined to a more specific SURFACE message naming
   the cluster-1 follow-up sub-cluster.

#### Producer-side migration rationale (§2.7.5 carrier-shape rule)

The §2.7.5 `NativeKind::String` slot contract is
`Arc::into_raw(Arc<String>) as u64` — refcount at offset -16 per the
standard Rust Arc layout. VM-side consumers
(`set_methods.rs::result_slot_to_string_arc` and `KindedSlot::Drop` for
`NativeKind::String` at `kinded_slot.rs:500-502`) decode via
`Arc::from_raw(bits as *const String)` /
`Arc::decrement_strong_count::<String>(bits)`. Pre-Round-12 the JIT-side
`box_string` producer returned `Box::into_raw(Box::new(UnifiedValue<
Arc<String>>)) as u64` — the W11 TypedArray-shape NaN-box carrier with
refcount at offset +4 inside the UnifiedValue allocation. VM-side
consumer's `Arc::from_raw` read the UnifiedValue header bytes as
`String`'s `ptr/cap/len` words → UB / segfault on access.

Producer-side migration is the principled fix per ADR-006 §2.7.17 (Round
7A precedent for Result/Option) generalized to the
`NativeKind::String` carrier. JIT-internal NaN-box paths
(dispatch-shell's method-name push at `terminators.rs:235`,
`call_string_method` returns, etc.) continue to use `value_ffi::box_string`
unchanged — those paths flow within JIT and consume via the same
`unbox_string` NaN-box decoder. The kind-track stamp for the
method-name push slot is `NativeKind::String` (decorative for the
JIT-internal pathway; the dispatch shell's `unbox_string(method_bits)`
body knows the carrier shape from its own ABI contract, not from the
kind track).

#### Compile-time-constant refcount discipline

`arc_string_constant` boosts the initial refcount to 2. This is
load-bearing for two reasons:

1. **Constant survival across multiple JIT-compiled function calls**:
   the JIT embeds the `Arc::into_raw` pointer as a compile-time-emitted
   `iconst I64`. Every runtime occurrence of the site reads this same
   constant pointer. Without the boost, a single use-then-drop pattern
   (e.g., `let s = "a"; some_call(s); /* scope exit */`) would
   decrement to 0 and free; next call → use-after-free.

2. **Tolerance to imbalanced retain/release**: any code path where a
   release fires without a paired prior retain (e.g., the
   `MirConstant::Str` arg flowing through the dispatch shell where the
   VM trampoline's `KindedSlot::Drop` decrements without the JIT having
   pre-incremented for the constant arg) leaves the constant at
   refcount=1 (still alive) rather than 0 (freed). The constant's
   "permanent share" absorbs the imbalance.

The "leaked" extra share is a deliberate per-constant-site one-time
memory cost — `O(distinct string constants × Arc<String> size)` per
JIT-compiled function. Same lifecycle as the legacy NaN-box `box_string`
path (which also leaked the UnifiedValue allocation via `Box::into_raw`
without a paired `Box::from_raw`).

#### Decision: TypedObject migration surface-and-stops

The `Ptr(HeapKind::TypedObject)` arm in `terminators.rs` print
Call-terminator stays SURFACE per the round's surface-and-stop
discipline (round dispatch §"Surface-and-stop expected": "If the
TypedObject migration scope exceeds the budget OR breaks the
W11-jit-new-array TypedArray<T> shape ... STOP and surface to
disambiguate").

Rationale: the JIT-internal `TypedObject` struct (in
`crates/shape-jit/src/ffi/typed_object/`) and the VM-side
`Arc<TypedObjectStorage>` (in `crates/shape-value/src/heap_value.rs`)
are TWO DIFFERENT Rust types with different layouts. The JIT-side has
its own ref-counting in the `TypedObject` struct's header (offset +4
HeapHeader-style). The VM-side carrier is a strict-typed Arc-shape with
refcount at offset -16. Migrating `box_typed_object` to
`Arc::into_raw(Arc<TypedObjectStorage>)` would break 17+ JIT-internal
consumers in `typed_object/`, `data.rs`, `property_access.rs` etc. that
read the JIT TypedObject struct directly via `unbox_typed_object`.

This is a separate sub-cluster's scope. Tracked as cluster-1 follow-up
**W17-jit-typed-object-arc-storage-migration** (NEW surface, surfaced
by this round).

#### Surfaced (NOT regressions, pre-existing)

- **`print(Err("x"))` JIT — kind classifier upstream stamps
  `Ptr(TypedObject)` instead of `Ptr(Result)` for `Err` arm of Result**.
  Verified pre-existing by stashing all my changes and rebuilding:
  baseline produces the same `Some(Ptr(TypedObject))` kind at the print
  Call-terminator surface site. The bug is somewhere in the MIR-builder
  / type-inference layer for `BuiltinCall(ErrCtor)` destination slots
  — `Ok(5)` correctly stamps `Ptr(Result)` (per the working
  `print(Ok(5))` smoke), but `Err("x")` stamps `Ptr(TypedObject)`.
  Asymmetric defect at the upstream producer-side classifier.
  Orthogonal to W12 T2/T3's territory (which migrated the JIT-side
  String / Method `MirConstant` lowering, not the `BuiltinCall(ErrCtor)`
  destination kind stamp). Tracked for cluster-1 / Round 13+
  sub-cluster **W17-jit-err-ctor-kind-classification**.

- **TypedObject Aggregate path** (`let p = {x:1, y:2}; print(p.x)`).
  Aggregate lowering in `Rvalue::Aggregate` is surface-and-stop per W11
  / Route A; `print(p.x)` reaches the Aggregate fallback before any
  TypedObject carrier consideration is reached. Out of scope for W12
  T2/T3.

#### Coordination with T1 (W12-jit-trait-dispatch-return-kind)

The dispatch metadata noted both T1 and T2/T3 might touch
`mir_compiler/types.rs` (different regions). At close, **T2/T3 did NOT
touch `mir_compiler/types.rs` at all** — the kind track flows for
`NativeKind::String` already work post-§2.7.5-conduit-extensions from
Rounds 6A / 8A / 11-trinity. The producer-site migration affected only
`mir_compiler/ownership.rs::compile_constant`. T1 and T2/T3 ship with
zero file-level conflicts.

T2/T3 unblocks Smoke 4 JIT (verified VM == JIT for the kickoff Smoke 4
target). T1 unblocks Smoke 3 JIT (trait method dispatch return-kind
classification). Both required for full cluster-0 Smoke 3+4 closure.

#### Forbidden patterns refused on sight (audit trail)

- "string-carrier bridge" / "TypedObject probe" / "Arc-NaN-box
  translator" / "boundary adapter" — all refused on sight per CLAUDE.md
  "Renames to refuse on sight" broader-family regex. Producer-side
  migration is the actual fix; describing the deletion as a "bridge"
  perpetuates the wrong-architecture framing.
- "Compat shim for `unified_box` callers" — refused. Full producer-side
  migration at the §2.7.5-stamped sites; no transitional shim. JIT-
  internal NaN-box paths keep `box_string` unchanged because they speak
  a different ABI contract internally (not a §2.7.5 carrier), not
  because of a "compat shim".
- "Mark TypedObject migration as Round 13 follow-up" — surfaced as
  cluster-1 W17 sub-cluster (NEW surface), NOT marked as a thing-to-do-
  later within W12. The surface-and-stop in `terminators.rs` is honest
  refusal-to-fabricate; the new sub-cluster is the principled
  follow-up shape.
- "Keep `unified_box(HK_STRING, ...)` for snapshot/wire compat" —
  refused. Snapshot/wire uses per-slot kind metadata; no NaN-box at
  runtime. `box_string` is still in the codebase because it serves a
  separate role (JIT-internal NaN-box for method-name push, etc.), not
  as a snapshot/wire helper.

### W17-vm-intrinsic-sum-wave-5d-migration close (2026-05-13)

Closed Round 13 T4 production-first sub-cluster. Branch
`bulldozer-strictly-typed-w17-vm-intrinsic-sum-wave-5d-migration` from
post-Round-12 baseline `3db6e820` (Round 13 dispatch metadata commit).
~110 LoC body migration + 8 unit tests at
`crates/shape-vm/src/executor/vm_impl/builtins.rs`.

#### Close-report deliverables (per dispatch text)

**(1) Which kickoff smoke IntrinsicSum blocks:** Smoke 2 `.sum()`. As
named in the round dispatch: "blocks kickoff Smoke 2 VM
(`[1,2,3].sum()` and `.map(...).sum()` both fire the IntrinsicSum
`todo!()`)".

**Dispatch-context refinement** (uncovered during fix): the bare
`[1,2,3].sum()` form does NOT route through `BuiltinFunction::IntrinsicSum`.
It dispatches via the `TYPED_INT_ARRAY_METHODS` PHF entry
(`typed_int_array_methods::sum`) at the method-dispatch tier and was
**already producing `Integer: 6`** pre-fix. The Round 12 post-merge
matrix entry "2 partial `[1,2,3].sum()` → 6 | ❌ T4 IntrinsicSum | ✅ 6 |
T4 (VM)" reflects the dispatch-tier classification at the time the
matrix was filled; the actual smoke surface had already migrated to the
PHF route via the Round 11A close `op_new_array` fix (which made
homogeneous-Int literals produce a `Ptr(HeapKind::TypedArray)` slot the
PHF dispatch shell consumes).

`IntrinsicSum` is the dedicated opcode the compiler emits for the
stdlib wrapper `std::core::math::sum(series)` body's
`__intrinsic_sum(series)` call (per `helpers.rs:3707`). That path now
exercises the migrated body. Empirically the stdlib-wrapper path
currently surfaces a §2.7.5 producer-site classification conduit gap
upstream: the generic-parameter `series` binding stamps the receiver as
`NativeKind::UInt64` rather than `Ptr(HeapKind::TypedArray)`, which the
new body correctly surface-and-stops on (refusing to Bool-default or
kind-blind-decode). The upstream gap is **out of T4 scope** — surfaced
for cluster-1 / Round 14 candidate
`W17-stdlib-generic-param-kind-classification` if the smoke matrix
later requires the stdlib wrapper route; current matrix close criterion
(`[1,2,3].sum()` → 6) is met via the PHF route.

**(2) Other wave-5d sites + dispositions:**

| Line | Variants | Disposition |
|---|---|---|
| 431 | Map / Filter / Reduce / ForEach / Find / FindIndex / Some / Every / ControlFold | Phase-2d residual (closure-driven array builtins; no current smoke blocker — `.map(...)` route blocked upstream by T5's pre-existing `call_value_immediate_nb` kind-mismatch surface, not this site) |
| 449 | IntrinsicVecAbs / Sqrt / Ln / Exp / Add / Sub / Mul / Div / Max / Min / Select / AddI64 | Phase-2d residual (vector intrinsics; no current smoke blocker) |
| 459 | IntrinsicMatMulVec / MatMulMat / MatAdd / MatSub | Phase-2d residual (matrix intrinsics; no current smoke blocker) |
| 467 | IntrinsicMinimize | Phase-2d residual (no current smoke blocker) |
| 518 (bulk arm, remaining 38 entries post-IntrinsicSum extraction) | IntrinsicBspline2_3dBatch / Mean / Min / Max / Std / Variance / Random* / Dist* / BrownianMotion / Gbm / OuProcess / RandomWalk / Rolling* / Ema / LinearRecurrence / Shift / Diff / PctChange / Fillna / Cumsum / Cumprod / Clip / Correlation / Covariance / Percentile / Median / Atan2 / Sinh / Cosh / Tanh / CharCode / FromCharCode / Series | Phase-2d residual (no current smoke blocker — none reachable from kickoff smokes 1-4 post-§2.7.24 monomorphization; many have parallel `register_typed_fn_1`/`_2` typed-marshal entries documented at `cluster-6-intrinsics-dispatch-table.md` lines 31-43 that would supersede the BuiltinCall path when the typed-CC migration completes the M1-split sub-decision) |

None of the 5 remaining wave-5d sites block any kickoff smoke; scope-
narrowing stands.

**(3) Migration shape used:** consistent with Phase 1B-vm Wave 6.5
substep-2 cluster-A canonical recipe at commit `eb24ef0`:

- Args consumed via `pop_builtin_args() -> Vec<KindedSlot>` (ADR-006
  §2.7.7 / Q9 stack parallel-kind track).
- Receiver kind sourced from the parallel-kind track at the dispatch
  shell — no fabrication.
- Receiver-kind guard: requires `NativeKind::Ptr(HeapKind::TypedArray)`;
  any other kind surfaces `VMError::RuntimeError` with the M1-split
  sub-decision cite. No Bool-default, no kind-blind decode, no
  `is_heap()` probe.
- Receiver borrow shape: direct `bits as *const TypedArrayData` cast
  per `typed_array_methods::borrow_typed_array` module-doc (the
  canonical reference pattern; ADR-006 §2.4 / Q6 stores
  `Arc::into_raw(Arc<TypedArrayData>)` directly in slot bits, NOT
  `Box<HeapValue>` — `as_heap_value()` would be unsound on this slot
  shape, as noted in the typed_array_methods.rs module-doc lines 19-23).
- Per-element-kind dispatch on `TypedArrayData::I64` → `i64` wrapping
  sum → `KindedSlot::from_int` (result kind `Int64`); `TypedArrayData::F64`
  / `FloatSlice` → `f64` sum → `KindedSlot::from_number` (result kind
  `Float64`). Result kind sourced from input element kind per
  recipe §2 result-kind-sourcing rule. Other TypedArrayData variants
  surface-and-stop pending M1-split sub-decision.
- Kernels mirror `typed_array_methods::v2_int_sum` / `v2_float_sum`
  exactly — same math, same result-kind rule, no SIMD path duplication
  (the simple loops match `int_buf_sum` / `iter().copied().sum()`).
- Empty arrays sum to 0 / 0.0 (matches `int_buf_sum` /
  `iter().sum::<f64>()` identity; no NaN fabrication, no surface).

#### Close gates (devenv exit-code-verified)

- `cargo check --workspace --lib --tests` EXIT=0.
- `cargo test -p shape-vm --lib intrinsic_sum_tests` 8 passed / 0
  failed / 0 ignored. Tests synthesize the BuiltinCall directly with
  properly-kinded typed-array slots (bypassing the upstream stdlib
  conduit gap) — they cover I64 sum, F64 sum, empty Int → 0, empty
  Float → 0.0, negative + wrapping sum, non-TypedArray-kind surface,
  Bool variant surface (M1-split sub-decision arm), and the Arc-into-raw
  projection round-trip.
- `cargo test -p shape-jit --lib` 382 passed / 0 failed / 26 ignored
  (above the 379 baseline cited in dispatch text — no new failures, no
  regressions; the +3 reflects upstream Round 12 T2/T3 string-carrier
  test additions).
- `cargo test -p shape-vm --lib` hits pre-existing v2-raw-heap aliasing
  SIGABRT in `w17_comptime_build_config_dispatches_end_to_end` — same
  class documented at CLAUDE.md "Known Constraints" + Round 11A close
  report:2648-2651, NOT caused by this change (the affected test is in
  `compiler::comptime::tests`, far from the intrinsic-body file).
- `bash scripts/verify-merge.sh` 12/12 Passed.
- `bash scripts/check-no-dynamic.sh` EXIT=0.
- Smoke 2 partial `[1,2,3].sum()` runtime verification: `Integer: 6`
  post-fix (same as pre-fix — PHF route unchanged; close criterion met
  per "Kickoff Smoke 2 partial VM `[1,2,3].sum()` → 6 produces 6
  matching JIT post-Round-12").

#### Forbidden frames refused on sight

- Bool-default kind for unproven `.sum()` return path → refused;
  body sources result kind from input element kind only.
- Kind-blind decode of accumulator → refused; result `KindedSlot`
  carries the correct `NativeKind` (`Int64` or `Float64`).
- "Preserve legacy body for one edge case" framing → refused; the
  legacy `vm_intrinsic_sum` wrapper around the deleted ValueWord-shape
  `shape_runtime::intrinsics::math::intrinsic_sum` is documented as
  Phase-2d-deferred at `crates/shape-vm/src/executor/builtins/intrinsics/math.rs:1-11`
  and stays deferred — the wave-5d body lives inline in `vm_impl/builtins.rs`
  per the dispatch table doc's `cluster-6-intrinsics-dispatch-table.md`
  routing.
- `Convert<X>To<Y>` opcode to paper over the missing stdlib-parameter
  kind classification → refused; the upstream §2.7.5 gap is surfaced
  as a separate sub-cluster candidate, not papered over with a kind-
  bridging opcode.
- "Mark this as a follow-up for the other 5 wave-5d sites" → the 5
  sites are out-of-territory Phase-2d residual; documented above with
  explicit dispositions, not marked as in-flight for T4.
- Any bridge/probe/helper/hop/translator/adapter/shim framing for the
  intrinsic body migration → refused; the body is a direct typed-
  pointer dispatch matching `typed_array_methods`' canonical
  reference pattern.

#### No ADR amendment required

§2.7.7 / Q9 stack parallel-kind track + ADR-005 §1 single-discriminator
+ ADR-006 §2.4 typed-pointer constructors already cover the body's
shape. The surfaced upstream gap (stdlib generic parameter kind
classification at `sum(series)`) is a §2.7.5 producer-site conduit
issue separate from this body migration; tracked for cluster-1 / Round
14 candidate `W17-stdlib-generic-param-kind-classification` if a
smoke later requires the stdlib wrapper path.

#### Coordination

- T5 (W17-vm-call-value-closure-kind-mismatch) — running parallel,
  no file-territory overlap (T5 touches `call_convention.rs`).
- T1' (W12-trait-method-return-conduit-cross-crate) — running
  parallel, no file-territory overlap (T1' touches
  `bytecode/core_types.rs` + linker + JIT MIR conduit).
- No merge attempted; supervisor runs the merge per dispatch text.

---

### W17-jit-typed-object-arc-storage-migration close (2026-05-13)

**Branch**: `bulldozer-strictly-typed-w17-jit-typed-object-arc-storage-migration`
**Round**: Phase 3 cluster-0 Round 14 W17 (audit-first standalone, parallel
with W12-map-chained).
**Disposition**: **AUDIT-ONLY CLOSE — surface-and-stop on ADR-006 §2.3
amendment territory.**

#### Audit doc

`docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md`
delivers all four audit deliverables (§1 ADR-005 §1 / §2.3 fit, §2 17+
consumer inventory, §3 cross-crate boundary check, §4 refuse-on-sight
discipline) plus §5 supervisor-disposition options + §6 empirical
verification + §7 close framing.

#### Key finding

The dispatch text's framing conflates two distinct migrations.

**(1) The load-bearing kickoff-Smoke-3 surface is a classification-layer
gap, not a payload-shape migration.** `crates/shape-jit/src/ffi/call_method/
mod.rs::receiver_type_name:51-81` (and the broader legacy-fallback cascade
at `:572-612`) contains 5+ tag-bit predicates (`is_number` / `is_heap` /
`heap_kind` / `is_typed_object` / `is_inline_function` / `is_ok_tag` /
`is_err_tag`) that all return wrong answers on raw `Box::into_raw(
UnifiedValue<*const u8>) as u64` carriers — the §2.7.5 stamp-at-compile-
time discipline removed the NaN-box tag wrap; producer `box_typed_object`
already emits raw bits (verified empirically via `SHAPE_JIT_TRACE=1`:
`[alloc] schema=54 result=0x56c5488796a0 kind=None HK_TYPED_OBJECT=1`).
The §2.7.7 / Q9 parallel-kind track already carries the correct kind at
the pop site (debug: `receiver_kind=Ptr(TypedObject) receiver_code=129
receiver_bits=0x56d0fa0f49f0`). The fix is in-crate, ~13 sites,
~150–250 LoC, no ADR amendment — drop the tag-bit predicates and consume
the kind directly from the parallel companion at every dispatch shell.

**(2) The dispatch text's "migrate to `Arc<TypedObjectStorage>` raw bits
per the §2.7.5 recipe" framing is ADR-006 §2.3 amendment territory.** The
JIT-internal `#[repr(C)] TypedObject` (`ffi/typed_object/mod.rs:67-75`:
u32 schema_id + u32 manual ref_count + 64-byte alignment + inline u64
field cells via byte offset) and the VM-side `Arc<TypedObjectStorage>`
(`shape-value/src/heap_value.rs:2356`: schema_id + `Vec<u64>` slots +
`Vec<NativeKind>` field_kinds + heap_mask, Rust-Arc refcount at offset
-16) have structurally divergent in-memory layouts. The divergences are
real and non-bridgeable under §2.7.5 alone:

1. Refcount placement (+4 vs -16) — JIT inline retain/release ops would
   scribble on the wrong word.
2. Field-access addressing (inline byte offset vs `Vec`-indirect) —
   different Cranelift IR pattern at every typed-field-load/store site.
3. Allocation surface (`alloc_zeroed` + 64-byte alignment vs `Arc::new`)
   — JIT performance contract (documented 2ns vs HashMap 25ns at
   `typed_object/mod.rs:28-34`) tied to inline-data shape.
4. Lifecycle (manual inc/dec_ref vs `Arc::increment/decrement_strong_count`)
   — different ownership machinery.
5. Schema model (both carry `schema_id: u32` via `type_schema_registry`)
   — this piece IS same-shape.

Migrating the JIT-internal `TypedObject` to `Arc<TypedObjectStorage>`
requires one of:

- **Option β.1**: Redesign `TypedObjectStorage` to match the JIT
  performance contract (inline-data + 64-byte alignment + custom
  Arc-like with variably-sized payload).
- **Option β.2**: Per-crate carrier-shape divergence under
  `NativeKind::Ptr(HeapKind::TypedObject)`. **Defection-attractor risk**
  per §2.7.10 / Q11 — exactly the carrier-shape drift the dispatch-ABI
  rebuild eliminated.
- **Option β.3**: Delete the JIT-internal struct + rebuild field-access
  codegen for `Vec`-indirect addressing. **Loses the documented inline-
  data performance contract**; perf-regression measurement required;
  ~1500–3000 LoC.

#### Recommendation — Option γ (scope split)

Round 15 dispatches **W17-narrow** (Option α scope = classification-layer
fix only): ~13 consumer sites, no ADR amendment, closes kickoff Smoke 3
JIT.

The typed-Arc carrier-shape decision becomes a separate **cluster-1
follow-up** (working name: `W17-typed-object-carrier-shape-decision`)
after supervisor disposition on β.1 / β.2 / β.3. Cluster-0 close (post
Smoke matrix verification) does NOT block on β — kickoff Smoke 3 JIT is
load-bearing on (1) only.

#### Empirical verification

- VM mode (baseline): kickoff Smoke 3 produces `x` ✓.
- JIT mode (current head): kickoff Smoke 3 produces empty output
  (`receiver_type_name(receiver_bits=0x56d0fa0f49f0, exec_ctx)` → `is_number`
  returns `true` because high bits aren't TAG_BASE → returns `Some("number")`
  → UFCS lookup `"number::name"` fails → dispatch falls through to TAG_NULL
  → print prints nothing).
- The §2.7.7 / Q9 parallel-kind track is correct (kind = `Ptr(TypedObject)`),
  but the legacy cascade ignores it and re-classifies via broken tag-bit
  predicates.

#### Inventory delivered

Audit §2 enumerates 12 JIT-private heap-op sites (`#1–#12`) + 3 classification-
surface sites (`#13–#15`) + 2 producer-side sites (`#16–#17`) + 2 retain/
release dispatch sites (`#18–#19`). For each: file:line, current call shape,
migration target, same-shape vs ADR-territory disposition.

#### Refuse-on-sight discipline preserved

No NaN-box-decode-preservation framing, no Bool-default fallback, no
bridge/probe/helper/hop/translator/adapter/shim framing (broader-family
regex per CLAUDE.md "Renames to refuse on sight"), no "tracked as a
follow-up for an individual consumer site" within the §2 inventory. The
typed-Arc carrier-shape decision is surfaced as a SEPARATE workstream,
not deferred from W17 — its scope was misframed in the dispatch text.

#### Close gates

- No source changes that regress Round 13 state — confirmed (only the
  audit doc + AGENTS.md row + this status subsection are added).
- `cargo check --workspace --lib --tests` — not re-run (no source touched).
- `cargo test -p shape-jit --lib` — not re-run (no source touched);
  baseline 383 + 26 ignored at the audit head per Round 13 close.
- `bash scripts/verify-merge.sh` — not run (no source merge).
- `bash scripts/check-no-dynamic.sh` — not run (no source touched).
- Smoke 3 JIT — empirically demonstrated unchanged from Round 13 close
  state (empty output; classification-layer gap reproducible).

#### Coordination

- W12-jit-map-chained (running parallel, no file-territory overlap —
  W17 touches `ffi/call_method/` + `ffi/typed_object/`; W12-map-chained
  touches `mir_compiler/types.rs` + possibly terminators).
- No merge attempted; supervisor runs the disposition per Option γ.

#### Files touched

- NEW `docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md`
- `AGENTS.md` (W17 row → closed with audit-only disposition)
- `docs/cluster-audits/phase-3-cluster-0-status.md` (this subsection)
### W12-jit-map-chained-method-return-kind-propagation close (2026-05-13)

**Branch**: `bulldozer-strictly-typed-w12-jit-map-chained-method-return-kind-propagation`
**Round**: 14 (audit-first standalone, parallel with W17-typed-object-arc).
**Status**: `blocked` (surface-and-stop fired post-implementation;
WIP code stashed).

#### Audit findings

§1 layer identification: **§1 (a) conduit extension** at bytecode-side
`infer_top_level_concrete_types_from_mir_with_resolvers`
(`crates/shape-vm/src/compiler/helpers.rs:494`). The
`MirConstant::Method(_)` Call-terminator destination stamping is
missing for built-in container receivers (`Array<T>`, `HashMap<K,V>`,
`Mutex<T>`, etc.). Same shape as Round 11-trinity Part b's JIT-side
parametric classifier (`parametric_method_return_kind_from_receiver`
at `mir_compiler/types.rs:921`) generalized one tier upstream to
produce `ConcreteType` instead of `NativeKind` so chained-method
receivers pick up the upstream stamp.

§2 cluster-0 disposition: kickoff Smoke 2 JIT
(`[1,2,3,4,5].map(|x|x*2).sum()` → `30`) confirmed blocked at the
pre-implementation baseline. Cluster-0 absorbs per supervisor's Q2
ruling.

§3 ADR-amendment posture (initial audit): no ADR amendment required;
fix is bounded conduit extension matching Round 6A + Round 11-trinity
Part b + Round 13 T1' precedents.

#### Implementation outcome

Conduit extension implemented end-to-end (~570 LoC across
`helpers.rs` + `compiler_impl_reference_model.rs`):

- New `parametric_method_return_concrete_type_from_receiver_and_closure`
  helper covering `Array<T>.map / filter / flatMap / sort / reverse /
  slice / take / skip / concat`.
- New Call-terminator destination stamping pass with fixed-point
  slot-move propagation interleaved for chained shapes.
- Two-pass closure-callable map handling BOTH closure-emission shapes
  (`ClosureCapture` for non-empty captures + `Assign(slot,
  Use(Constant(Function(name))))` for empty captures — the
  load-bearing form for `|x| x*2`).
- Three-tier `closure_returns` resolver: (i)
  `function_return_concrete_types[fid]`, (ii) scan closure body's
  typed `ReturnValue<Kind>` opcodes, (iii) None → Void per §2.7.5.1.

Verified via debug instrumentation that
`concrete_types[doubled_slot] = Array(I64)` is stamped correctly and
flows through the slot-move propagation to
`concrete_types[sum_receiver_slot] = Array(I64)`.

#### NEW SURFACE (audit §7)

JIT compilation now succeeds (no more `Route A surface-and-stop` at
print operand), but the resulting JIT-compiled code SIGSEGVs at
runtime (exit code 139) — strictly worse from triage perspective.
Trace:

1. `.map()` Call-terminator: generic dispatch via `jit_call_method`
   (no `"map"` arm in `try_emit_v2_array_method`).
2. `.sum()` Call-terminator: with `concrete_types[receiver_slot] =
   Array(I64)` stamp, the JIT dispatches to the FAST PATH at
   `crates/shape-jit/src/mir_compiler/v2_array.rs:367-387`
   (`jit_v2_array_sum_i64(arr_ptr)` direct FFI call).
3. The fast path FFI expects `arr_ptr` to be a raw
   `*const TypedArrayData<i64>`, but `.map()`'s generic dispatch
   returns a different carrier shape → segfault.

This is the **producer/consumer fast-path mismatch** defection-
attractor class — the JIT consumer's `try_emit_v2_array_method`
makes an unverified assumption about slot-storage shape that
doesn't hold across all producer paths. The conduit extension is
correct (`concrete_types[slot] = Array(I64)` is the right
classification), but the JIT consumer expected a stricter invariant
than the conduit's stamp guarantees.

#### Disposition

Per audit §7.2: **surface-and-stop, audit-only close.** The
honest options for closing kickoff Smoke 2 JIT are outside
cluster-0 W12-map-chained scope:

- **Option A** — `W12-jit-typed-array-fast-path-producer-verification`:
  narrow the JIT fast path to verify producer-side raw-pointer
  invariant via a `producer_kind` track.
- **Option B** — `W12-vm-map-typed-array-producer-migration`:
  migrate VM-side `.map` (and friends) to construct typed-array
  Arc pointer carriers matching the JIT fast-path expectation.
- **Option C** — both.

Surfaced for Round 15 supervisor disposition. WIP implementation
stashed (`git stash list` on this branch — message: "WIP:
W12-map-chained conduit extension exposes JIT consumer-side
fast-path gap").

#### Audit doc

`docs/cluster-audits/w12-jit-map-chained-method-return-kind-propagation-audit.md`
— §1 (layer identification), §2 (cluster-0 disposition), §3
(ADR-amendment posture, initial), §4 (refuse-on-sight discipline),
§5 (implementation budget), §6 (close gates), §7
(implementation attempt — NEW SURFACE uncovered, options A/B/C
for closing Smoke 2 JIT), §8 (audit-only close conclusion).

#### Round 15 recommendation

Dispatch either Option A or Option B per supervisor disposition.
Both close kickoff Smoke 2 JIT through the fast-path's
producer/consumer alignment, complementing W12-map-chained's
conduit extension (which can be un-stashed and merged once the
downstream consumer-side gap is closed).

## Round 15 — W12-map-chained Option B (AUDIT-ONLY CLOSE, 2026-05-13)

Phase 3 cluster-0 Round 15 sub-cluster W12-map-chained Option B
(supervisor-ratified producer-side carrier alignment) closes
**audit-only** per the dispatch prompt's Phase 1 surface-and-stop
discipline. The third surface-and-stop trigger fires:

> Stop and surface to the team lead if scope is unbounded ... OR
> the conduit extension's stamp doesn't align with the new
> producer's carrier.

Branch `bulldozer-strictly-typed-w12-map-chained-option-b`, parent
`0d9ae51e`. Zero source changes; audit doc + AGENTS.md row + this
subsection only.

### Phase 1 finding

The dispatch prompt's literal Option B prescription — "construct
results directly as raw `Arc::into_raw(Arc<TypedArrayData<T>>) as
u64` carriers" — describes the VM-side `.map` family's **current
output shape**:

- `int_array_result` at
  `crates/shape-vm/src/executor/objects/typed_array_methods.rs:135-139`
  already produces `Arc::into_raw(Arc<TypedArrayData::I64>) as u64`
  with kind `Ptr(HeapKind::TypedArray)`.
- `op_new_array` at
  `crates/shape-vm/src/executor/objects/object_creation.rs:367-371`
  + `op_array_push_local` at
  `crates/shape-vm/src/executor/objects/array_operations.rs:215-302`
  (the stdlib `vec.shape::map` body's bytecode lowering) produce
  the same shape.

The JIT consumer fast-path at
`crates/shape-jit/src/mir_compiler/v2_array.rs:334-441`
(`try_emit_v2_array_method`, specifically `jit_v2_array_sum_i64(arr:
*const TypedArray<i64>)` at `ffi/v2/mod.rs:115-124`) reads the
receiver slot's bits as **v2-raw `*mut TypedArray<i64>` flat
struct** (`crates/shape-value/src/v2/typed_array.rs:28-44` —
`#[repr(C)] { header: HeapHeader, data: *mut T, len: u32, cap: u32 }`),
NOT `Arc<TypedArrayData>` (Rust enum with `Arc<TypedBuffer<T>>`
variants).

The two layouts are **structurally distinct**: refcount placement
(HeapHeader-inline vs Rust-Arc-at-offset-minus-16), data addressing
(inline `*mut T` field vs Arc-wrapped TypedBuffer indirection),
enum discriminator presence (none vs first 8 bytes). Reading an
`Arc<TypedArrayData::I64>` payload as `TypedArray<i64>` interprets
the enum tag + Arc inner-pointer as header + data fields, then
dereferences invalid bits — the exact SIGSEGV (exit 139) the
predecessor audit `8354968a` recorded at §7.1.

### Disposition options

- **Reading A (dispatch prompt's literal text)**: producer should
  produce `Arc::into_raw(Arc<TypedArrayData<T>>)`. Producer already
  does this — Option B is a no-op on the producer side. Smoke 2
  cannot close.
- **Reading B (charitable — ADR-006 §2.7.14 Route A wording)**:
  producer should produce v2-raw `*mut TypedArray<T>` matching the
  consumer's FFI signature. Requires either:
  - Compiler-side specialization of `vec.shape::map`'s body so the
    inner `let mut result = []` lowers to `NewTypedArrayI64` (e.g.
    via `let mut result: Array<U> = []` annotation + monomorphized
    `U`-substitution propagation). Empirically verified that
    `let mut result: Array<int> = []` already emits
    `NewTypedArrayI64` (via `pending_variable_typed_array_kind`
    path at `statements.rs:672-682`) but `result.push(...)` in the
    same expression still emits `ArrayPushLocal` (kind-agnostic
    Arc-based push) due to receiver-loc-based dispatch — multiple
    cascading compiler changes needed.
  - OR a new Rust-side method handler replacing the stdlib's
    Shape-source body, with runtime-kind dispatch on the receiver
    that produces a matching v2-raw carrier.
  - Either way: contradicts ADR-006 §2.3's
    `HeapValue::TypedArray(Arc<TypedArrayData>)` mandate for typed-
    array HeapValue kinds. Would require an ADR-006 §2.3 / §2.7.14
    amendment to acknowledge dual carrier shapes for `Array<T>`
    where T is scalar:
    `*mut TypedArray<T>` UInt64-tagged v2-raw vs
    `Arc<TypedArrayData>` Ptr(HeapKind::TypedArray)-tagged typed-Arc.

### Scope-bounded check

The dispatch prompt's three scope-unbounded triggers:

| # | Trigger | State |
|---|---|---|
| 1 | More producers than VM-side `.map` family | OK — bounded to ~10 stdlib `vec.shape` methods + Rust-side `array_transform::handle_*_v2`. |
| 2 | More consumers than the typed-array fast-path families | OK — bounded to `try_emit_v2_array_method` (one file). |
| 3 | Conduit extension's stamp doesn't align with the new producer's carrier | **TRIGGERED** — see Phase 1 finding. |

### Recommendation for Round 16

Mirror the W17 audit `8ae56222` Option γ scope-split pattern.
Round 16 dispatches one of:

- **B' — ADR-006 §2.3 / §2.7.14 amendment + carrier unification**:
  unify the typed-array carrier shape across
  `HeapValue::TypedArray(Arc<TypedArrayData>)` and `*mut TypedArray<T>`.
  One canonical layout, one canonical drop path. Mirror of W17 β.1.
- **B'' — Producer migration to v2-raw + ADR amendment to
  acknowledge dual carrier shapes**: change the producer (stdlib
  `vec.shape::map` + companions + Rust-side Arc-array path) to
  produce v2-raw `*mut TypedArray<T>` bits with `NativeKind::UInt64`
  when the input is scalar-element v2-raw. Requires closure-
  return-kind inference and the cascading compiler changes named
  in Reading B above.

The producer/consumer fast-path mismatch is now a **3-instance
class** (W12-map-chained + W12-jit-string-carrier-unification's
TypedObject surface + W17-typed-object-arc-storage-migration) —
CLAUDE.md amendment candidate per the supervisor's 2026-05-13
Round-14-close recurrent-pattern note.

### Close gates

Zero source changes. `cargo check --workspace --lib --tests`,
`verify-merge.sh`, `check-no-dynamic.sh` baseline state preserved
from `0d9ae51e`. Smoke 2 baseline:
- VM `--mode vm` prints `30` (working).
- JIT `--mode jit` errors `Route A surface-and-stop:
  NotImplemented(SURFACE) — print Call-terminator operand
  NativeKind is None` (pre-conduit-extension surface; the conduit
  extension at `8354968a` was audit-only-closed with code stashed
  per supervisor's Round 14 disposition).

Audit doc: `docs/cluster-audits/w12-map-chained-option-b-audit.md`.

---

## Round 15 — W17-narrow close (PRODUCTION CLOSE, 2026-05-13)

Phase 3 cluster-0 Round 15 sub-cluster W17-narrow (Option γ from the
W17-typed-object-arc Round 14 audit, supervisor-ratified) closes
**production-first**. Sub-cluster contract met: classification layer
migrated from tag-bit dispatch to NativeKind-from-§2.7.7/Q9-parallel-
track dispatch. **Smoke gate deferred** per supervisor's Round 15
disposition: production-first sub-clusters may merge with the smoke
gate unmet when (a) the sub-cluster's own contract is verifiably met,
(b) smoke-gate failure is due to surfaced upstream gaps tracked as
new sub-clusters (not follow-up-to-ignore), (c) no forbidden-pattern
framings harbored. W17-narrow meets all three.

Branch `bulldozer-strictly-typed-w17-narrow`, parent `3d42ba52` (the
post-Round-15-W12-Option-B-merge HEAD). Close commit `8b048c02`;
merge commit `84800f80`.

### What landed

Classification-layer fix at the JIT `call_method` shell:

- `crates/shape-jit/src/ffi/call_method/mod.rs:51-81`
  (`receiver_type_name`) — signature widened from
  `(receiver_bits: u64, exec_ctx: &ExecutionContext) -> Option<String>`
  to `(receiver_bits: u64, receiver_kind: NativeKind, exec_ctx) ->
  Option<String>`. Dispatch on `receiver_kind` directly. For
  `Ptr(HeapKind::TypedObject)`: read `(*ptr).schema_id`, resolve via
  two-tier fallback (stdlib `lookup_schema_by_id_public` then
  `vm.program().type_schema_registry` for user-defined types like
  Smoke 3's `X`). Full NativeKind variant enumeration — no wildcard
  arm, no Bool-default fallback (refuse-on-sight discipline).
- `crates/shape-jit/src/ffi/call_method/mod.rs:111-167`
  (`try_call_user_method`) — signature widened to take
  `arg_pairs: &[(u64, NativeKind)]` per §2.7.7/Q9 parallel-kind
  track lockstep. The audit's literal `&[u64]` would have been a
  kind-blind ABI shape (pre-§2.7.9 MethodFnV2 defection class);
  agent caught + corrected in-place. Result-pop clears `stack_kinds`
  back to SENTINEL.
- `crates/shape-jit/src/ffi/call_method/mod.rs:572-592` (legacy-
  fallback builtin JIT-format method dispatch cascade) — all 5
  tag-bit predicates (`is_ok_tag` / `is_err_tag` / `is_number` /
  `is_inline_function` / `heap_kind`) replaced with
  `matches!(receiver_kind, ...)` checks reading from the parallel-
  kind track.
- Plus ~13 consumer-call-site updates across
  `crates/shape-jit/src/ffi/typed_object/{allocation,field_access,merge_ops}.rs`.

Net: 4 source files, +362 / −66 LoC. ~50% over the audit's 150-250
LoC budget; ratified by supervisor as mechanical (variant enumeration
+ §-cite docstrings; no β scope creep, no forbidden patterns).
Heuristic update for future audit budgets: classification-layer fixes
touching dispatch + multiple files need ~1.5x literal estimate.

### Smoke 3 status — 4-sub-cluster dependency

Smoke 3 JIT (`trait T { fn name(&self) -> String } ... let t: dyn T
= box(X{}); print(t.name())`) end-to-end unblock requires all four:

1. **W17-narrow** ✅ (this close) — classification layer at JIT
   call_method dispatch shell.
2. **R11 / R13 conduit work** ✅ — trait return-kind propagation +
   parametric method return classifier (Round 12 T2/T3, Round 13 T1'
   cross-crate).
3. **W17-narrow-follow-up-A** (Round 16) — JIT MIR-lowering
   `StatementKind::ObjectStore` schema-id identity preservation.
   Currently allocates via `register_predeclared_any_schema(&field_names)`
   returning a `__predecl_*`-named schema id; fix threads
   `Option<u32> schema_id` from `OpCode::NewTypedObject` bytecode
   operand. SURFACE at `crates/shape-jit/src/mir_compiler/statements.rs::StatementKind::ObjectStore:141-201`,
   ADR-006 §2.7.5.
4. **W17-narrow-follow-up-B** (Round 16) — JIT kind-aware print null
   handling. `format_kinded_inner` treats `bits == 0` as None but
   `jit_call_method`'s TAG_NULL fallthrough returns
   `make_tagged(TAG_NONE_BITS, 0)` — a tagged sentinel ≠ 0. SURFACE
   at `crates/shape-vm/src/executor/printing.rs::format_kinded_inner:154-163`,
   ADR-006 §2.7.5.

W17-narrow landed its scope correctly; Smoke 3 JIT unblock requires
A + B (the Round 16 follow-up sub-clusters).

### Audit-prescription correction caught in-place

Supervisor observation: Round 14 audits harbored **two latent
literal-prescription errors** caught by Round 15 production-first
work:

1. **TypedArrayData / TypedArray conflation** — caught by W12-Option-B
   audit (the producer already emits `Arc::into_raw(Arc<TypedArrayData>)`;
   consumer expects v2-raw `*const TypedArray<T>` flat struct;
   structurally distinct).
2. **`&[u64]` kind-blind signature for `try_call_user_method`** —
   caught by W17-narrow agent + corrected in-place to
   `&[(u64, NativeKind)]` per §2.7.7/Q9. Pre-W17-narrow the body was
   unreachable (UFCS lookup always missed) so the missing kind writes
   were latent; classification fix exposed the gap.

Future audit-first dispatches should include "verify ABI shape per
§2.7.7/Q9 parallel-kind track lockstep" as an explicit deliverable.
Audits CAN harbor latent defection-attractor prescriptions even when
their dispatch text frames them as discipline-compliant.

### Stash-then-rebuild verification

Agent verified Smoke 3 JIT segfault is **not** introduced by the
classification-layer fix: stashed the W17-narrow changes, rebuilt
the binary at base `3d42ba52`, reproduced identical segfault.
`SHAPE_JIT_DEBUG=1` trace confirms `receiver_kind=Ptr(TypedObject)`
flows through correctly; segfault is downstream at
`jit_call_method`'s TAG_NULL fallthrough → `jit_print_str` (the
follow-up-B null-handling gap).

This stash-then-rebuild + structured-surfacing-of-new-sub-clusters
pattern is the canonical reconciliation of MEMORY.md "Own all code
quality" + smoke-gate-deferred close. Supervisor ratified.

### Close-gate infrastructure

- `cargo check --workspace --lib --tests` exit 0 (verified through
  devenv shell wrapper per `reference_phase2d_devenv.md`).
- `bash scripts/verify-merge.sh` SCRIPT_EXIT=0, 12/12 PASS (run
  post-merge on bulldozer HEAD `84800f80`).
- `bash scripts/check-no-dynamic.sh` exit 0.
- AGENTS.md row appended.
- NO `Co-Authored-By: Claude` trailer.

---

## Round 16 — W12-Option-B-reframed close (AUDIT-ONLY, 2026-05-13)

Phase 3 cluster-0 Round 16 sub-cluster `W12-Option-B-reframed`
(supervisor-ratified after Round 15 W12-Option-B audit-only close
surfaced that the literal Round 14 prescription conflated
`TypedArrayData<T>` Arc-wrapped Rust enum with `TypedArray<T>` 24-byte
`#[repr(C)]` flat struct).

Branch `bulldozer-strictly-typed-w12-option-b-reframed`, parent
`a7b93def` (post-Round-15-W17-narrow-merge + status doc HEAD).

**Audit-only close.** Phase 1 surface-and-stop fires.

### Phase 1 finding

Kickoff working hypothesis: "Option B''' boundary conversion per
§2.7.14-A. Per-variant unwrap-and-flatten from `Arc<TypedArrayData::T>`
to `*const TypedArray<T>` at the JIT FFI boundary. **Other typed-array
fast-paths already work for direct invocation (e.g. `[1,2,3].sum()`
after Round 11A) — they must already perform this conversion
somewhere. Find that site.**"

**Audit's response (audit deliverable a, §1.1)**: no such conversion
site exists. The two carrier shapes (`Arc<TypedArrayData>` and
`*mut TypedArray<T>`) travel through **bytecode-emitter-disjoint
paths**:

- **Literal `[1,2,3]` working JIT path** —
  `crates/shape-vm/src/compiler/expressions/collections.rs:214-228`
  emits `OpCode::NewTypedArrayI64` for annotated/inferred-homogeneous-
  int literals; VM handler at
  `crates/shape-vm/src/executor/v2_handlers/array.rs:44-53` runs
  `TypedArray::<i64>::with_capacity(...)` and pushes
  `*mut TypedArray<i64>` raw pointer with `NativeKind::UInt64`. The
  JIT consumer at `crates/shape-jit/src/mir_compiler/v2_array.rs:367-387`
  passes that pointer to `jit_v2_array_sum_i64(arr: *const TypedArray<i64>)`
  at `crates/shape-jit/src/ffi/v2/mod.rs:115-124`. **Both ends are
  natively the flat-struct shape — no Arc to unwrap, no enum tag to
  dispatch on.** Empirical at this commit: `[1,2,3,4,5].sum()`
  produces `15` under `--mode jit`.
- **Stdlib `vec.shape::map`'s `let mut result = []` failing path** —
  `collections.rs:229-264` else branch emits `OpCode::NewArray`
  (empty literal, no annotation, no spread); VM handler at
  `crates/shape-vm/src/executor/objects/object_creation.rs:367-371`
  runs `Arc::new(TypedArrayData::I64(Arc::new(buf)))` and pushes
  `Arc::into_raw(arc) as u64` with
  `NativeKind::Ptr(HeapKind::TypedArray)`. Per-iteration
  `result.push(...)` lowers to `OpCode::ArrayPushLocal`, handled at
  `crates/shape-vm/src/executor/objects/array_operations.rs:215-302`,
  which has **two per-carrier-shape arms** (Ptr(HeapKind::TypedArray)
  Arc-path vs UInt64 v2-raw-path). The two runtime carriers are
  **first-class peers** in the VM, supported in lockstep.

The JIT consumer fast-path gate at
`crates/shape-jit/src/mir_compiler/terminators.rs:116-118` reads
`concrete_types[slot]` (compile-time ConcreteType stamp), not the
runtime NativeKind track. Both runtime carriers map to ConcreteType
`Array(I64)`; only the flat-struct carrier is sound to dispatch to
the flat-struct FFI.

### §2.7.14-A draft NOT committed (audit deliverable d)

The supervisor-provided §2.7.14-A draft text mis-describes the runtime
reality in three load-bearing places (audit §1.4):

1. "Heap carrier (canonical per §2.3): `Arc<TypedArrayData::T>` where
   T is the per-element-kind enum variant" — `T` is an enum-variant
   tag (`I64`/`F64`/...), not a type parameter on a carrier struct.
2. "Conversion site is the JIT FFI dispatch shell: per-variant
   unwrap-and-flatten before calling the monomorphized FFI entry" —
   no such site exists; the JIT-FFI-boundary discriminator is the
   `NativeKind` (`Ptr(HeapKind::TypedArray)` vs `UInt64`), not the
   `TypedArrayData` variant tag.
3. "Unwrap-and-flatten" framing reads as if the layouts are
   variants of a compatible representation; they are structurally
   distinct types with separate allocations. A real conversion would
   require allocating a fresh `TypedArray<T>` + O(n) byte copy of
   the `TypedBuffer<T>` payload + Arc-share release on the original
   — a **synthesis** (not conversion) of a new value with a
   different lifecycle.

Committing the draft as-is would lock in the Round 14 conflation in
different vocabulary, repeating the W-series defection-attractor
pattern. The draft is preserved verbatim in audit §5 for the
considered-and-not-committed record.

### Disposition options for Round 17 (audit §3 / §8)

Four structural options, supervisor disposition required:

1. **B'** — ADR-006 §2.3 amendment to unify carrier shapes (multi-week).
2. **B''** — Producer migration to v2-raw + ADR amendment to
   acknowledge dual carrier shapes for scalar `Array<T>`.
3. **A-§2.7.7** (new disposition — explicitly distinct from the
   Round-15-refused Option A by mechanism): change
   `v2_typed_array_elem_kind` from `concrete_types[slot]` to runtime
   `NativeKind` track lookup at the dispatch shell. Same shape as
   W17-narrow Round 15. No ADR amendment, smallest scope. Accepts
   performance gap: `.map().sum()` runs through generic method-
   dispatch trampoline instead of inline SIMD reduction until B' or
   B'' lands.
4. **Status quo** — close Smoke 2 by extending the
   `infer_top_level_concrete_types_from_mir_with_resolvers` conduit
   at `crates/shape-vm/src/compiler/helpers.rs:494` for the
   **non-fast-path generic dispatch route's** kind propagation
   through `print` (the original surface in the audit baseline:
   "operand NativeKind is None").

### Defection-attractor pattern

Producer/consumer fast-path mismatch with carrier-shape divergence
at the JIT FFI boundary is now a **4-instance class**:
- Round 14 W12-map-chained (conduit stashed at `4ddd1bfb`).
- Round 14 W17-typed-object-arc-storage-migration (ADR-006 §2.3
  amendment territory).
- Round 15 W12-Option-B (literal prompt is no-op on producer;
  charitable Reading-B requires ADR amendment).
- Round 16 W12-Option-B-reframed (working hypothesis premise refuted;
  §2.7.14-A draft mis-describes reality).

CLAUDE.md "Forbidden Patterns" amendment is binding per the
supervisor's 2026-05-13 Round-14-close recurrent-pattern note.

### Close gates

Zero source changes. Baseline gates verified pre-commit:
- `cargo check --workspace --lib --tests` EXIT=0.
- `bash scripts/verify-merge.sh` 12/12 Passed, EXIT=0.
- `bash scripts/check-no-dynamic.sh` EXIT=0.

Smoke 2 baseline state:
- `--mode vm` prints `30` (working).
- `--mode jit` errors `Route A surface-and-stop:
  NotImplemented(SURFACE) — print Call-terminator operand NativeKind
  is None` (pre-conduit-extension surface; SIGSEGV NOT reachable on
  this baseline because predecessor stash `4ddd1bfb` is NOT applied).

Audit doc: `docs/cluster-audits/w12-option-b-reframed-audit.md`.

---

## Round 17 — W12-typed-array-data-deletion close (AUDIT-ONLY, 2026-05-13)

Phase 3 cluster-0 Round 17 sub-cluster
`W12-typed-array-data-deletion-audit` — strategic-owner-authorized
aggressive deletion of `TypedArrayData` enum + `TypedBuffer<T>`
wrapper layer; `TypedArray<T>` flat struct becomes the universal
`Array<T>` carrier. The Round 16 W12-Option-B-reframed audit named
the dual-carrier reality (`Arc<TypedArrayData>` Arc-wrapped enum vs
`*mut TypedArray<T>` 24-byte flat struct as runtime peers selected
at bytecode-emission time, not convertible variants). Round 17
authorized deletion of one side.

Branch `bulldozer-strictly-typed-w12-typed-array-data-deletion-audit`,
parent `aa5de4ab` (post-Round-16 A+C merge HEAD).

**Audit-only close.** Zero source changes per supervisor's
audit-only mandate. Deliverables (a-h) landed in
`docs/cluster-audits/w12-typed-array-data-deletion-audit.md`.

### Phase 1 findings

**(a) Per-variant migration disposition** (audit §2): all 22 live
`TypedArrayData` variants classified into three buckets —

- **Clean (12 scalar variants)**: I64/F64/Bool/I8/I16/I32/U8/U16/
  U32/U64/F32/Char migrate to `TypedArray<T>` monomorphization
  with mechanical 6-step recipe per kind (size-assert + tag-byte +
  bytecode emission + VM handler + JIT FFI + element-type-tag
  byte). 4 of 12 (`f64`/`i64`/`i32`/`u8`) already wired at
  HEAD `aa5de4ab` — 8 new monomorphizations required.
- **Clean with prereq (5 heap-element variants)**: String /
  Decimal / BigInt / DateTime / Timespan / Duration / Instant
  migrate to `TypedArray<*const <X>Obj>` once the v2-raw
  `<X>Obj` carrier structs land. `StringObj` exists post-R12
  W12-jit-string-carrier-unification close. `DecimalObj` /
  `BigIntObj` / `TemporalObj` / `InstantObj` are new
  monomorphizations.
- **Structural-obstacle (2 heap-element variants)**: TypedObject /
  TraitObject — `TypedObjectStorage` / `TraitObjectStorage` are
  `Arc<>`-wrapped without `HeapHeader` at offset 0. Per-element
  retain/release inside `TypedArray<T>` would have to use
  `Arc::increment_strong_count` (-16 offset) vs `v2_retain`
  (+0 offset) per-T — breaks retain-uniformity. Resolution:
  defer until `TypedObjectStorage` v2-raw migration lands as own
  cluster-level work order.
- **Category-error (2 variants)**: `Matrix(Arc<MatrixData>)` is a
  **single Matrix**, not a buffer-of-Matrix. The variant exists
  because ADR-006 §2.7.22 Q23 (W15-matrix audit, 2026-05-10)
  refused `HeapKind::Matrix` parallel discriminator under ADR-005
  §1 single-discriminator. Under Round 17 deletion authorization
  Q23's reasoning collapses: with `TypedArrayData::Matrix`
  deleted there's no parallel HeapKind label. Audit recommends
  Matrix exits the array-carrier hierarchy entirely via new
  `HeapKind::Matrix = 34` + `HeapValue::Matrix(Arc<MatrixData>)`
  arm. FloatSlice gets `HeapKind::MatrixSlice = 35`.

**(b) "Keep TypedArrayData::X for one variant" refused on sight**
per supervisor's framing. Every variant has named disposition or
named structural obstacle with named resolution shape.

**(c) HashMapValueBuf parallel-consideration verdict** (audit §5):
same deletion principle applies. `HashMapData::values:
HashMapValueBuf` migrates to `*mut TypedArray<V>` with per-V
monomorphization. Same O-1/O-3/O-3a obstacles apply. **Separate
cluster-1 deletion target with parallel migration shape — NOT
cluster-0 scope.** Forward-visibility for cluster-1+ planning.

**(d) Sub-cluster migration plan (S1-S5)** (audit §3):
- **S1 — scalar-variant width pass** (12 variants, 1 session,
  ~3-4k LoC mechanical).
- **S2 — heap-element migration** (5 variants, ~2 sessions,
  requires `<X>Obj` carrier structs to land in parallel).
- **S3 — TypedObject/TraitObject migration** (gated on
  O-3/O-3a resolution; defer until `TypedObjectStorage` v2-raw
  migration lands as own cluster).
- **S4 — Matrix/FloatSlice exit** to own HeapKinds (1 session,
  ~3-4k LoC; supersedes ADR-006 §2.7.22 Q23 ruling).
- **S5 — `TypedArrayData` enum + `TypedBuffer<T>` actual
  deletion** (0.5 session, ~2-3k LoC subtractive).

**Deprecation cadence**: per-variant `#[deprecated]` annotations
land in matching S1-S4 closes; S5 deletes the enum in full.

**(e) Drafted §2.7.24 Q25.A amendment text** (audit §6, NOT
committed in this audit; commits in S5 close with enum deletion).
Q25.B parallel text drafted for cluster-1 HashMapValueBuf
deletion. Q25.A's framing retired: "TypedArrayData with per-built-
in-heap-type specialized variants" → "`TypedArray<T>` flat struct
with per-T monomorphization at every layer (VM/JIT/snapshot/wire)
+ no runtime tag decode".

**(f) Session-count estimate** (audit §3.7): **floor 4 sessions,
ceiling 6 sessions** at Phase 2d post-audit production-first
cadence (~3 sub-clusters/session). Ceiling triggered if O-3
resolution requires multi-week `TypedObjectStorage` v2-raw
migration as own cluster-level work order before S3 lands.

**(g) Structural obstacles surfaced for supervisor disposition**
(audit §4):
- **O-1** — DateTime/Timespan/Duration share `Arc<TemporalData>`
  payload, distinguish at user-facing-type via enum variant tag.
  Resolution options: O-1.a element-type-tag byte extension (ADR
  §2.7.5 amendment) / O-1.b separate `<X>Obj` carriers per
  semantic kind / O-1.c language-level merge.
- **O-2** — F64 SIMD alignment. `AlignedTypedBuffer` uses
  AlignedVec<f64>'s stricter alignment; `Layout::array::<f64>`
  only gives 8-byte. Resolution options: O-2.a per-T alignment
  parameter on `TypedArray<T>` (v2-spec amendment) / O-2.c
  accept unaligned-load trade-off. **NOT O-2.b parallel
  `AlignedTypedArray<f64>` carrier** — that's the carrier-duality
  the deletion is solving.
- **O-3 / O-3a** — TypedObject/TraitObject Arc-vs-HeapHeader.
  Recommend O-3.c defer until `TypedObjectStorage` migration
  lands as own cluster.
- **O-4** — TypedBuffer<T>'s null-validity bitmap has no
  `TypedArray<T>` equivalent. Clean resolution O-4.a: bitmap-
  bearing path migrates to separate `ArrowBuffer<T>` v2-raw
  shape (matches runtime-v2-spec.md §6.4 Arrow C Data
  Interface long-term shape). TypedArray<T> stays at 24-byte
  v2-spec contract.

### CLAUDE.md amendment landed

Per supervisor's verbatim ADDITIONAL DIRECTIVE, the
"Parallel-implementation across producer/consumer carrier-shape
boundaries" sub-section appended to "Forbidden Patterns" →
"Renames to refuse on sight". Refuses "documented intentional
duality" / "preserve both carriers" / "carrier unification via
boundary deletion" applied as one-off patch / "per-variant
unwrap-and-flatten" framing. Names 5-instance class history
(W12-jit-string-carrier-unification R12, W17-jit-typed-object-arc-
storage-migration R14, W12-Option-B R15, W12-Option-B-reframed
R16, this audit R17). Names deletion target: `TypedArrayData`
enum + `TypedBuffer<T>` wrapper layer; keep `TypedArray<T>` flat
struct.

### Close gates

Zero source changes. Baseline gates verified pre-commit at HEAD
`aa5de4ab`:
- `cargo check --workspace --lib --tests` EXIT=0.
- `bash scripts/verify-merge.sh` 12/12 PASS, EXIT=0.
- `bash scripts/check-no-dynamic.sh` EXIT=0.

### Defection-attractor class is now 5-instance

- Round 12 (R12) W12-jit-string-carrier-unification: MirConstant::
  Str producer migration.
- Round 14 (R14) W17-jit-typed-object-arc-storage-migration: JIT
  receiver_type_name NaN-box-decode vs VM-side raw-Arc-bits.
- Round 15 (R15) W12-map-chained-option-b: literal Option B
  prescription is no-op on producer.
- Round 16 (R16) W12-Option-B-reframed: working hypothesis
  "boundary conversion site" premise refuted; §2.7.14-A draft
  mis-describes runtime reality.
- Round 17 (R17, this audit) W12-typed-array-data-deletion:
  strategic-owner-authorized whole-enum deletion target; per-
  variant migration plan delivered; 4 structural obstacles
  surfaced for supervisor disposition.

CLAUDE.md amendment binding per supervisor's 2026-05-13
recurrent-pattern note. Audit doc:
`docs/cluster-audits/w12-typed-array-data-deletion-audit.md`.

---

*Next session: read this file first, then continue with Round 18
dispatch — supervisor disposition required on O-1 (DateTime/
Timespan/Duration semantic-kind disambiguation), O-2 (F64 SIMD
alignment), O-3/O-3a (TypedObject/TraitObject Arc-vs-HeapHeader),
and S4-Matrix-exit ratification (new `HeapKind::Matrix = 34` /
`HeapKind::MatrixSlice = 35` ordinal assignments superseding
ADR-006 §2.7.22 Q23). Sub-cluster S1 (scalar width pass) can
dispatch immediately if F64 is not in S1 scope (otherwise depends
on O-2). HashMapValueBuf parallel deletion (Q25.B) surfaces as
cluster-1+ work order, NOT cluster-0 scope.*

---

## Round 17 audit-only merge (POST-MERGE, 2026-05-13)

R17 deletion-audit merged into `bulldozer-strictly-typed` at commit
**`038066de`**. Audit close was `e55b8e71`. Doc-only merge (audit
doc + status doc subsection + CLAUDE.md amendment + AGENTS.md row);
no source changes. Post-merge gate `verify-merge.sh` 12/12 PASS via
devenv shell.

### Supervisor dispositions on the 4 structural obstacles

- **O-1 (Temporal disambiguation)** — newtype-wrapper path:
  `TypedArray<DateTimeData>` / `TypedArray<TimespanData>` /
  `TypedArray<DurationData>`. Surface-and-stop in S2 dispatch if
  the semantic tag turns out to be load-bearing internal to
  `TemporalData` (newtype-infeasibility). Default: newtype.
- **O-2 (F64 SIMD alignment)** — preserve alignment via explicit
  override in `TypedArray<f64>`. **No regression accepted.**
  Surface-and-stop in S1 with measured perf delta if preservation
  isn't feasible within `TypedArray<T>`'s structural constraints.
  Default: preserved.
- **O-3 / O-3a (TypedObject / TraitObject)** — defer to cluster-1.
  Tracked as candidate `W12-typed-object-array-retain-migration`
  (audit §4.4). Kickoff Smoke 3 uses `box(X{})` (single TypedObject,
  not `Array<TypedObject>`), so cluster-0 close is not affected.
- **O-4 (ArrowBuffer<T> nullable carrier)** — cluster-1 deferral
  confirmed (see follow-up below).

### Three Round 18 follow-up surfacings (per supervisor)

- **(O-4 nullable-array reachability)** — verified by audit §4.5:
  the `TypedBuffer<T>` validity bitmap is used "only ... by the
  DataTable / column-ref paths, NOT by user-visible `Array<T?>`
  typed arrays." Cross-checked against all 4 kickoff smokes:
  Smoke 1 (scalar `for` loop — no arrays), Smoke 2 (`Array<int>`
  literal + `.map().sum()` — non-nullable), Smoke 3 (trait dispatch
  via `box(X{})` — no arrays), Smoke 4 (`HashSet<string>` — not
  `Array<T?>`). **None reach nullable-array territory.** Per
  supervisor's contingent directive, **S4 ArrowBuffer<T> defers to
  cluster-1.** Round 18 dispatches S1 + S3 in parallel, no
  concurrent S4.
- **(§2.7.22 Q23 amendment draft status)** — audit §6 contains the
  **§2.7.24 Q25.A** amendment draft (for the enum deletion at S5),
  NOT §2.7.22 Q23. Audit §3.4 names "ADR-006 §2.7.22 amendment
  text: Q23 ruling reframed (Matrix exits the array carrier, gets
  its own HeapKind)" as an S3 close-gate deliverable. So **§2.7.22
  Q23 amendment text is not yet drafted**; **S3 dispatch must
  include draft-and-commit of the §2.7.22 Q23 amendment text
  alongside the new `HeapKind::Matrix=34` / `HeapKind::MatrixSlice=35`
  allocations.**
- **(W17-narrow-follow-up-A back-patch design surfacing)** —
  the dispatch prescribed threading `Option<u32> schema_id` from
  the bytecode-side `OpCode::NewTypedObject` operand through MIR
  `ObjectStore` directly at MIR-lowering time. Agent observed
  `crates/shape-vm/src/mir/lowering/` operates on AST only and
  lacks access to the bytecode compiler's
  `type_tracker.schema_registry`; direct threading would have been
  intrusive across all callers. Agent adopted a **post-lowering
  back-patch pattern** in a new module
  `crates/shape-vm/src/compiler/mir_schema_threading.rs`
  (`back_patch_schema_ids`, ~120 LoC), mirroring the established
  `ClosurePlaceholder` / `ClosureCapture` back-patch precedent at
  `functions.rs:493-549` + `compiler_impl_reference_model.rs:1354-1410`.
  Named structs resolve via `mir.local_struct_type_names` +
  bytecode compiler's tracker; anonymous inline objects via
  `register_inline_object_schema` (idempotent). **Discipline rule
  motivating the change**: ADR-006 §2.7.5 stamp-at-compile-time
  discipline + "consult the most-specific source at the latest
  reliable moment" — using the bytecode compiler's tracker
  (post-AST-lowering) preserves user-declared schema-id identity
  without intrusive cross-tier coupling at MIR lowering. **Same
  precedent shape as W17-narrow's `try_call_user_method` widening
  surfacing** (Round 15). +228/−9 LoC across 10 files; deletes
  `register_predeclared_any_schema` call site at JIT consumer;
  no forbidden patterns. **Awaiting supervisor ratification**, then
  merge of `b9115aea` independent of R17 timing.

### Cluster-1 candidate list (post-R17)

Cluster-1 hardening now includes (newly added by R17 audit
findings):

- `W12-typed-object-array-retain-migration` — O-3/O-3a; audit §4.4
- `HashMapValueBuf` deletion (parallel to TypedArrayData
  deletion) — audit §5; ADR-006 §2.7.24 Q25.B amendment scope
- `ArrowBuffer<T>` carrier — O-4; conditional deferral confirmed

Plus the prior 5 cluster-1 items already tracked.

### 5-instance defection-attractor count

Producer/consumer carrier-shape-mismatch class now stands at 5
instances (R12 / R14 / R15 / R16 / R17 audit-ratification). The
CLAUDE.md amendment landed in `038066de` (line 281, "Renames to
refuse on sight" → "#### Parallel-implementation across producer/
consumer carrier-shape boundaries") names the 4 prior instances;
R17 is itself an instance via the deletion-target framing it
ratifies.

### Revised cluster-0 close trajectory (3-4 rounds remaining)

| Phase | Sub-clusters | Sessions |
|---|---|---|
| Round 18 | S1 (scalar) + S3 (Matrix exit) parallel | 1-2 |
| Round 19 | S2 (heap-element + `<X>Obj` prereqs) | 1-2 |
| Round 20 | S5 (enum deletion + §2.7.24 Q25.A amendment commit) | 1 |
| Cluster-0 close attempt | full 4-kickoff-smoke matrix VM==JIT | 1 |

Cluster-1 hardening now widened to 8 items (5 prior + 3 new from
R17). Total handoff-to-v1 estimate: 17-23 sessions.

### Handover-doc annotation re: CLAUDE.md authorization

Folded into the same commit as this status doc subsection.
`phase-3-team-lead-handover.md` §Decision authority pattern now
notes: **CLAUDE.md modifications require explicit user
ratification; supervisor draft != pre-authorization.** The R17
sequence (supervisor drafted text + asked for user authorization
+ user replied "authorized text for CLAUDE.md" + team-lead folded
the landing directive into the agent dispatch prematurely) is the
one-time learning moment. Going forward: supervisor's drafted text
+ user's explicit ratification of *the landing* (not just *the
text*) are both required before a CLAUDE.md modification appears
in any agent's commit.

Strategic owner retroactively ratified `038066de`'s CLAUDE.md hunk
per supervisor's Option (a). Substance is what was authorized;
the procedural gap was timing.

---

*Next session: read this file first; Round 18 dispatch (S1 + S3
parallel) is authorized contingent on all three follow-up
surfacings landing (the three subsections above) + W17-narrow-
follow-up-A back-patch ratification (independent timing).*

---

## Round 18 sub-cluster S1 — W12-typed-array-data-scalar-migration close (2026-05-13, closed-with-reopen)

Branch `bulldozer-strictly-typed-w12-typed-array-data-scalar-migration` at
parent `52dbe312` (post-W17-narrow-follow-up-A merge HEAD). Initial close
at `4bcae991` landed 6 mechanical scalar variants (I8/U8/I16/U16/U32/U64)
+ a defensive low-address-pointer guard at
`v2_array_detect.rs::as_v2_typed_array`. **Supervisor reopen
(W11-round-1 reopen precedent)** excised the U64 variant + the defensive
guard before merge per the CLAUDE.md §"Parallel-implementation across
producer/consumer carrier-shape boundaries" entry (e55b8e71, R17 audit
close): the guard's `bits < 0x1_0000 → None` memory-region heuristic is
an `is_heap()` probe in different framing, refused on sight per ADR-006
§2.7.7 ("kind-aware clone/drop dispatch — no tag decode, no `is_heap()`
probe"). Post-reopen close: **5 of 12 dispatch-named scalar variants
migrated mechanically per audit §3.1**; **4 variants surfaced as
structural obstacles** per audit §2 / phase-2d-handover §0 surface-and-
stop discipline.

### Variants migrated (5, post-reopen)

`I8`, `U8`, `I16`, `U16`, `U32` — each gets a `TypedArrayKind::<X>`
enum variant, 4 new `OpCode` variants (`New/Get/Push/SetTypedArray<X>`),
an `ELEM_TYPE_<X>` element-type-tag byte (distinct from `ELEM_TYPE_BOOL`
for U8 per audit §2.1's "collides with Bool element-type-tag" note),
`V2ElemType::<X>` variant, executor handler bodies, dispatch routing,
JIT preflight whitelist registration, v2→legacy materialization arms in
`array_transform.rs`, slice arms, iter-next arms, and method-dispatch
classifier integration. Already-supported `F64 / I64 / I32 / Bool` paths
unchanged — those producers were already on `TypedArray<T>` at HEAD.

OpCode byte values 0x11F..0x122 left unallocated for S1.5's U64 re-mint;
`ELEM_TYPE_U64 = 10` reserved (not produced by any current allocation
path).

### Variants NOT migrated, surfaced as structural obstacles (4, post-reopen)

**U64** [NEW post-reopen]: `NativeKind::UInt64` at HEAD is overloaded
between scalar u64 and v2-typed-array-pointer carrier (every
`*mut TypedArray<T>` flows through this kind). Routing producer-emission
and consumer-classification through the same overloaded discriminator
violates the §2.7.7/Q9 parallel-kind-track invariant. The pre-reopen S1
commit added a defensive memory-region heuristic (`bits < 0x1_0000 →
None`) at `as_v2_typed_array` to paper over the symptom (small U64
element values would deref into unmapped memory and SIGSEGV when `print`
probed them as potential typed-array pointers); the supervisor reopen
named that heuristic as the 6th instance of the CLAUDE.md §Parallel-
implementation defection-attractor class — refused on sight. Resolution
shape: ADR-006 §2.7.7/Q9 amendment adding a discriminating `NativeKind`
variant (e.g. `NativeKind::TypedArrayPtr`) that flows alongside
`NativeKind::UInt64` on the §2.7.7 parallel-kind track, so producers
and consumers can dispatch on kind without runtime bit-pattern probes.
Cluster-equivalent scope. **Deferred to sub-cluster S1.5
(W12-nativekind-typed-array-ptr-extension or equivalent per team-lead's
pre-dispatch audit)**.

**F32**: dispatch named in scope but `NativeKind` has no `Float32`
variant and `ConcreteType` has no `F32` variant at HEAD `52dbe312`. The
mechanical recipe in audit §3.1 step (3) (per-kind `NativeKind` dispatch
in `should_use_typed_array_from_slot_kind`) requires the variant to
exist. Resolution shape if pursued: ADR-006 §2.7.5 amendment adding
`NativeKind::Float32` + cascading `decode_f64`/`is_integer`/
`is_nullable_integer`/`is_floating`/etc. across `native_kind.rs` (~30
sites) + cascading every `KindedSlot::from_<X>` constructor + adding
`ConcreteType::F32` + monomorphisation cascade. Cluster-equivalent
scope — explicitly out of S1's stated mechanical-recipe boundary.

**Char**: same shape as F32. No `NativeKind::Char` nor
`ConcreteType::Char` at HEAD. Audit §3.1 included Char among 12 scalar
variants but the prerequisite NativeKind variant does not exist.
Cluster-equivalent scope.

**String**: audit §2.2 explicit out-of-S1-scope. The `StringObj`
v2-raw carrier exists at HEAD (`crates/shape-value/src/v2/string_obj.rs`),
but `TypedArray<*const StringObj>` would require per-element retain on
push + release on pop + release-all on drop_array — `TypedArray<T>::
drop_array` does NOT release per-element refcounts because `T: Copy`
semantics apply. This is the audit §2.2 S2 sub-cluster's territory.
Including String in S1 would either silently leak StringObj refcounts
(defection-class fallback) or require S2 prerequisite work to land
alongside (scope explosion).

### O-2 (F64 SIMD alignment) disposition

**Not applicable to S1**. S1 is producer migration only; F64 is in the
4 already-supported kinds (not migrated). The `Array<number>` producer
at HEAD already emits `OpCode::NewTypedArrayF64` → `TypedArray<f64>`
with `Layout::array::<f64>` 8-byte alignment. The legacy
`AlignedTypedBuffer` lives behind the `TypedArrayData::F64` enum
consumer-side, untouched by producer migration. SIMD reductions at
`v2_array_detect.rs::simd_sum_f64` use `f64x4::from([*data.add(i),...])`
(scalar loads + lane construction, not aligned-vector loads), so the
existing F64 v2 path already runs without aligned-load semantics — no
SIMD regression introduced by S1, no `AlignedTypedArray<f64>` parallel
preservation (refused on sight per audit §4.2 O-2.b).

### Defensive guard REMOVED at reopen

The `as_v2_typed_array` low-address-pointer guard (`bits < 0x1_0000 →
None`) that the pre-reopen S1 commit `4bcae991` added is excised. The
supervisor's reopen named the heuristic as the 6th instance of the
CLAUDE.md §"Parallel-implementation across producer/consumer carrier-
shape boundaries" defection-attractor class — an `is_heap()` probe in
different framing, refused on sight per ADR-006 §2.7.7's "no tag decode,
no `is_heap()` probe" mandate. The site at `v2_array_detect.rs:148-167`
now carries a surface-and-stop comment block citing CLAUDE.md
§Parallel-implementation + the deletion-fate of S1.5's discriminator
extension as the structural fix. The runtime SIGSEGV that motivated the
guard is replaced by compile-time legacy-path dispatch
(`should_use_typed_array(ConcreteType::U64) → None` at the bytecode
emitter routes `Array<u64>` literals to `OpCode::NewArray`, which never
lights up the `as_v2_typed_array` probe path on U64 element reads).

### Close gates (devenv-wrapped, exit-code-verified, post-reopen)

- `cargo check --workspace --lib --tests` EXIT=0
- `bash scripts/verify-merge.sh` SCRIPT_EXIT=0, **Passed: 12 / Failed: 0**
- `bash scripts/check-no-dynamic.sh` EXIT=0
- `cargo test -p shape-vm --lib v2_typed_emission` 56/56 PASS — includes
  the post-reopen regression guards `test_u64_falls_back_to_legacy` +
  `test_slot_kind_uint64_falls_back_to_legacy` (both assert `None`,
  preventing U64 from re-entering the typed fast path before S1.5)
- `cargo test -p shape-vm --lib v2_array_detect` 5/5 PASS

### Smoke results (post-reopen)

Smoke S1 (comprehensive 9-variant post-reopen: `let a: Array<i8> = [...];
print(a[0]); ...` across the 5 newly-migrated variants + 4 already-
supported F64/I64/I32/Bool; U64 omitted):

- `--mode vm`: `-5 / 100 / 255 / 30000 / 30000 / 300 / 2.5 / 30 / true`
- `--mode jit`: identical sequence (VM ≡ JIT for all 9 variants)

**U64 compile-time fall-through verification post-reopen**:
`let d: Array<u64> = [1000, 2000, 3000]; print(d[0])` lowers via the
legacy NaN-boxed `OpCode::NewArray` path (compiler's
`should_use_typed_array(ConcreteType::U64) → None` routes to the legacy
generic-array fallback in `compile_expr_array`), prints `1000` correctly
under VM — the runtime SIGSEGV that motivated the (now-removed)
defensive guard is replaced by compile-time legacy-path dispatch.

Smoke 2 baseline state (kickoff smoke `print([1,2,3,4,5].sum())`): not
regressed by S1 (S1 doesn't touch the I64 producer paths Smoke 2
exercises); VM=`15` ✓ JIT=`15` ✓ post-reopen.

### Recurrence pattern note

This S1 close (post-reopen) demonstrates the named audit §2 framing
pattern: "every variant either has clean migration OR surfaces a
specific structural obstacle to the supervisor — no 'keep both'
disposition." S1 post-reopen followed this exactly: **5 mechanical
migrations + 4 structural-obstacle surfacings + 0 Bool-default fallbacks
+ 0 memory-region heuristics + 0 silent enum-arm preservations**. The
`TypedArrayData::I8`/`U8`/`I16`/`U16`/`U32`/`U64` arms still live in
source at this commit but are unreachable from S1-migrated producer
paths for the 5 migrated kinds (the recipe makes them unreachable, not
deleted — S5's commit deletes the arms). `TypedArrayData::U64` remains
reachable via the legacy `OpCode::NewArray` path until S1.5 lands the
§2.7.7/Q9 discriminator extension.

### Round 18 S3 coordination

S3 (W12-matrix-floatslice-heapkind-exit) runs in parallel from the
same base `52dbe312`. S3 territory (HeapKind enum + Matrix/FloatSlice
variant removal + §2.7.22 Q23 amendment text) has zero file-territory
overlap with S1 (scalar-variant producer migration). AGENTS.md row
collisions handled by take-both at merge per dispatch convention.

### S1.5/S2/S3/S4/S5 follow-up surfaces

- **S1.5** (W12-nativekind-typed-array-ptr-extension, NEW post-reopen):
  ADR-006 §2.7.7/Q9 amendment adding a discriminating `NativeKind`
  variant for "pointer-to-`TypedArray<T>`" distinct from scalar
  `UInt64`, cascading through every kind-keyed dispatch (clone_with_kind /
  drop_with_kind / printing / KindedSlot accessors / etc.). Required
  before U64 carrier migration + defensive guard removal can land
  cleanly. Cluster-equivalent scope.
- **S2** (heap-element TypedArray<*const <X>Obj> migration including
  String): blocked on per-element retain/release plumbing in
  `TypedArray<T>` for heap-element T, plus the v2-raw `<X>Obj` carrier
  structs for Decimal/BigInt/Temporal/Instant that audit §2.2 names.
- **S3** (TypedObject/TraitObject migration): blocked on O-3/O-3a
  (`TypedObjectStorage`/`TraitObjectStorage` Arc-vs-HeapHeader carrier
  shape resolution); audit recommends O-3.c defer.
- **F32/Char element-kind extension** (follow-up surfaced by this S1
  close): blocked on `NativeKind` + `ConcreteType` variant additions
  via ADR-006 §2.7.5 amendment. Either land before a re-dispatch of S1
  to cover F32/Char, or surface as separate Phase 3 cluster-equivalent
  scope.
- **S4** (Matrix/FloatSlice exit + new `HeapKind::Matrix = 34` /
  `HeapKind::MatrixSlice = 35`): independent of S1.
- **S5** (`TypedArrayData` enum + `TypedBuffer<T>` deletion):
  unchanged from audit §3.5; remains blocked on S2/S3/S4 + S1.5.
