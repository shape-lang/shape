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

Items 2 and 3 are the cluster-2 candidate flags the user asked for.
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
| W12-jit-binop-after-heap-read-kind-tracker | `bulldozer-strictly-typed-w12-jit-binop-heap-read` | 3 (binop after p.x field read) | dispatching |
| W12-jit-aggregate-non-array-carrier | `bulldozer-strictly-typed-w12-jit-aggregate-non-array` | 1.5 + 2 (Aggregate for Enum/Struct/Tuple destinations) + 30+ stdlib fns | dispatching (audit-first) |
| W12-jit-print-kind-classification | `bulldozer-strictly-typed-w12-jit-print-kind` | 4 (`.size()` int result mis-decoded as f64) | **closed** (2026-05-12) |

**Deferred to future cluster (NOT cluster-0):** `W12-collection-constructor-mir-lowering` (8 sites). The Round-4 audit identified this but Smoke 4's actual gap was framed as print-classification, not constructor MIR. Round-5C's audit-and-fix shows the constructor MIR gap IS load-bearing for Smoke 4's value-correctness (see W12-jit-print-kind-classification close below) — but the print-classification fix is independently correct and is the prerequisite for any future Smoke-4 close.

### W12-jit-print-kind-classification close (2026-05-12)

Audit-first sub-cluster (Round 5C). Root cause: the print
dispatch in `crates/shape-jit/src/mir_compiler/terminators.rs:298-396`
already routes correctly per `operand_slot_kind(&args[0])` — the
kinded entries (`jit_print_i64` / `jit_print_f64` / `jit_print_bool`)
landed in W11-jit-new-array Round 1 fire correctly when the kind is
known. The gap is upstream: `kinds[slot]` returns `None` for two
load-bearing patterns:

1. **`TerminatorKind::Call` destinations.** `infer_slot_kinds`
   only walks `StatementKind::Assign` writes. The destination of a
   Call terminator is a separate kind-source the statement-walk
   misses. `let n = s.size()` writes the method-call result into a
   temp slot whose kind stays `None`; the downstream `Assign(n_slot,
   Use(Move(temp)))` propagates `None` into the user-visible binding.
2. **`Place::Index` element-kind projection.** `operand_slot_kind`
   collapses `Place::Index(arr, _)` to `root_local()` and returns
   the array's kind, not the element kind. `print(xs[0])` flows to
   the print path with kind `Ptr(HeapKind::TypedArray)` and falls
   through to the kind-blind fallback (`format_value_word`, a deleted-
   W-series tag-decode pattern per CLAUDE.md "Forbidden code").

**Fix shape (ADR-006 §2.7.5 producing-site classification)**:

- New `infer_slot_kinds_with_concrete(mir, existing, concrete_types)`
  entry point that adds two passes to the legacy `infer_slot_kinds`:
  (a) a Call-terminator pre-pass BEFORE the forward statement pass —
  destination slots of `TerminatorKind::Call` with
  `MirConstant::Method(name)` / `MirConstant::Function(name)` are
  stamped from a bounded well-known return-kind registry (`size` /
  `len` / `length` / `count` → `Int64`; `isEmpty` / `is_empty` / `has`
  / `contains` → `Bool`; global `len` builtin → `Int64`). Pre-forward
  placement is load-bearing: it makes the call temp's kind visible
  to the forward pass's propagation through `Assign(n_slot, Use(Move(
  temp)))`. (b) An `infer_index_element_kind` helper consulted at the
  forward-pass entry — when the Rvalue is `Use(Copy/Move/MoveExplicit(
  Place::Index(Local(arr), _)))` and the receiver slot's
  `ConcreteType` is `Array(elem)` with a scalar element kind, the
  destination's kind is the element kind.
- `operand_slot_kind` in `rvalues.rs` extended to recognize
  `Place::Index(Local(arr), _)` and project the array's
  `ConcreteType::Array(elem)` to the element `NativeKind` via the
  existing `elem_slot_kind_for_concrete` helper — closes the
  kind-source gap at the FFI dispatch site without touching the
  slot-write path.

**Forbidden frames refused on sight**: (i) NO Bool-default fallback
for unknown kind — the print dispatch's kind-blind fallback path is
preserved as-is for genuinely-unproven operand kinds (the §2.7.5
surface-and-stop prescription); the fix removes the conditions under
which the fallback fires for the named smokes. (ii) NO "print-
classification bridge" / "kind-routing helper" / "print-decode probe"
/ similar bridge-probe-helper-hop-translator-adapter-shim framing —
the producing-site stamp is at the MIR-compile layer, not as a
runtime probe in the print FFI body. (iii) NO decoding kind from
raw bits via `format_value_word` NaN-decode — kind comes from the
§2.7.5 stamp at MIR-compile time. (iv) NO heap-arm kinded print
entries (`jit_print_str` / `jit_print_typed_object` / `jit_print_
result` / `jit_print_option`) — these are the cluster-2 candidate
item #3 surfaced in the table above, and not exercised by the
cluster-0 smoke matrix (all current smokes have scalar results that
route through the existing `print_i64` / `print_f64` / `print_bool`
entries correctly).

**Smoke results — under `--mode vm` and `--mode jit` separately**:

| Smoke | VM | JIT |
|---|---|---|
| `print(42)` | `42` | `42` ✅ |
| `print(3.14)` | `3.14` | `3.14` ✅ |
| `print(true)` | `true` | `true` ✅ |
| `let xs: Array<int> = [10, 20]; print(xs[0])` | `10` | `10` ✅ (element-kind projection) |
| `let xs: Array<int> = [10, 20, 30]; print(xs.length())` | `3` | `3` ✅ (well-known return-kind) |
| `let xs: Array<int> = [10, 20, 30]; let n = xs.length(); print(n)` | `3` | `3` ✅ (terminator pass before forward pass) |
| Smoke 4 (`Set()` + `s.add()` + `print(s.size())`) | `2` | integer value (kind classification correct; **value wrong because `Set()` constructor doesn't construct**) |

**Sites surfaced (NOT silently fallback'd)**:

- (a) **Smoke 4's value-correctness depends on the deferred
  `W12-collection-constructor-mir-lowering` sub-cluster.** The
  pre-dispatch audit in this Round-5 status section claimed "Set()
  constructor works correctly and program reaches `print(s.size())`
  cleanly" — `SHAPE_JIT_DEBUG=1` reveals a `[jit-call-value] SURFACE
  §2.7.5: callee_bits=0x0` at the `Set()` call site, confirming
  `MirConstant::Function("Set")` is unresolved in `function_indices`
  exactly like the §2.3 collection-ctor family the Round-4 audit grid
  identified. The print classification fix is complete and correct
  (visible by the f64-denormalized `0.000...535409` output flipping
  to a clean integer output) regardless of the upstream `Set()`
  constructor gap — but the VM=JIT smoke-4 equivalence requires the
  collection-constructor MIR lowering too. Surfaced as a finding
  that contradicts the playbook's pre-dispatch audit; supervisor
  decides whether to retag `W12-collection-constructor-mir-lowering`
  as load-bearing for cluster-0 close or accept the divergence
  with a tracked follow-up.
- (b) `well_known_method_return_kind` is a bounded registry by
  design — names outside the set return `None` and the slot's kind
  genuinely isn't statically classifiable at the producing-MIR
  layer alone. Adding `toArray` / `toString` / etc. would require
  verifying every receiver-side dispatch-table entry returns the
  declared kind across every receiver type the dispatch reaches.
  Potential follow-up if the smoke matrix ever exercises one of
  those names.
- (c) Heap-arm print entries (`jit_print_str` / `jit_print_typed_
  object` / `jit_print_result` / `jit_print_option`) per Round-1
  surfaced item #3 still stand as a cluster-2 candidate — not
  exercised by the cluster-0 smoke matrix.

**Branch:** `bulldozer-strictly-typed-w12-jit-print-kind`
**Close commit:** (pending — appended below at merge)

**Close gates (devenv exit-code-verified)**:
- `cargo check --workspace --lib --tests` EXIT=0
- `cargo test -p shape-jit --lib` EXIT=0 (322 passed, 0 failed, 26
  ignored — matches baseline 322/0/26 on `7a78799b`)
- `bash scripts/verify-merge.sh` EXIT=0 (Passed: 12 / Failed: 0)
- `bash scripts/check-no-dynamic.sh` EXIT=0

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

*Next session: read this file first, then continue with Round-2
close-out (or pivot per supervisor's call between cluster-1 hardening
and cluster-2 Wave-3 surfaces).*
