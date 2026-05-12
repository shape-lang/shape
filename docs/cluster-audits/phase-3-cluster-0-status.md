# Phase 3 cluster-0 — status

**Started:** 2026-05-12 (this session)
**Parent:** `phase-2d-close` `e22bffd2`
**Branch:** `bulldozer-strictly-typed`
**Current HEAD:** `ff1ad3e6` (Round-2 W11-jit-carrier-conversion merged); Round-3 dispatched 2026-05-12

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

## Round 3 — dispatching

Four sub-clusters dispatched in parallel 2026-05-12:

| Sub-cluster | Branch | Smoke unblocked | Est | Status |
|---|---|---|---|---|
| W12-jit-stack-parallel-kind-track | `bulldozer-strictly-typed-w12-jit-stack-kind-track` | 1.5 (Result/match with closures) | ~1 session | migrating |
| W12-top-level-concrete-types-conduit | `bulldozer-strictly-typed-w12-top-level-concrete-types` | 3 (TypedObject field access) — `Rvalue::Aggregate` surface closed | ~1 session | **closed** 2026-05-12 |
| W12-jit-linker-symbol-resolution | `bulldozer-strictly-typed-w12-jit-linker-resolve` | 2 (Option/return + Array) | ~1 session (audit-first) | auditing |
| W12-deleted-valuewordshape-tests-rewrite | `bulldozer-strictly-typed-w12-vw-tests-rewrite` | 17 ignored tests un-ignored | ~1 session (parallel test-infra) | migrating |

### W12-top-level-concrete-types-conduit close (2026-05-12)

Branch: `bulldozer-strictly-typed-w12-top-level-concrete-types` (see commit hash in close commit).

**Conduit shape:** new `#[serde(skip)]` field
`top_level_local_concrete_types: Vec<ConcreteType>` on
`BytecodeProgram` + `Program` + `LinkedProgram`, populated by a
MIR-walk inference pass (`infer_top_level_concrete_types_from_mir` in
`crates/shape-vm/src/compiler/helpers.rs`). The walk stamps slots from
MIR-level kind-source statements:

- `StatementKind::ObjectStore { container_slot, .. }` →
  `ConcreteType::Struct(StructLayoutId(0))` (the schema-id placeholder
  is irrelevant for the JIT short-circuit which only checks the
  variant tag)
- `StatementKind::EnumStore { container_slot, .. }` →
  `ConcreteType::Enum(EnumLayoutId(0))` (same)
- `StatementKind::ArrayStore { container_slot, operands }` →
  `ConcreteType::Array(scalar)` when all operands resolve to a
  homogeneous scalar kind via a per-slot scalar pre-pass walking
  `Assign(slot, Use(Constant))`; heterogeneous / non-literal arrays
  leave the slot at `ConcreteType::Void` (the explicit "no information"
  sentinel per §2.7.5.1, NOT a Bool-default fallback per forbidden #9)
- Use-of-local fixed-point propagation: `Assign(dst, Use(Move|Copy
  local))` propagates the source slot's stamped ConcreteType to the
  destination slot, handling the `let p = temp` pattern emitted by
  the MIR lowering after `Aggregate` + `ObjectStore`

**Why MIR-walk and not bytecode-compiler slot mapping:** top-level
code allocates the user's bindings as module_bindings (NOT bytecode
locals — `self.next_local` is 0 at top level), so the bytecode-
compiler's per-local side-tables (`local_array_element_types`,
`current_function_local_concrete_types`) do not carry top-level
`let p = Point{...}` slots. The cached top-level MIR already encodes
the structural type information through the `ObjectStore` / `ArrayStore`
/ `EnumStore` statements. The walk is purely from the proven MIR
shape; no runtime decode, no Bool-default fallback.

**JIT consumer side:** new helper `is_typed_object_slot` in
`crates/shape-jit/src/mir_compiler/v2_array.rs` (matches `Struct(_)`/
`Enum(_)`/`Option(_)`/`Result(_, _)`/`Tuple(_)`). The
`Assign(Aggregate)` handler in `crates/shape-jit/src/mir_compiler/
statements.rs` adds a TypedObject short-circuit: when the destination
slot is a TypedObject, skip the Aggregate (the redundant MIR scratch
step). The subsequent `ObjectStore` does the real `typed_object_alloc`
+ per-field-set work. This mirrors the existing typed-array
short-circuit (`v2_typed_array_elem_kind` + `emit_v2_array_aggregate`).

**Smoke 3 (`type Point { x, y } let p = Point{x:3, y:4}; print(p.x +
p.y)`):**
- VM mode: prints `7` ✓
- JIT mode: `Rvalue::Aggregate` surface eliminated; now hits
  `compile_binop_dynamic_arith: kind-untyped arith Add reached the
  JIT — SURFACE per W10 playbook §5: producing-MIR kind-tracker gap`
  on the `p.x + p.y` field-access addition. **This is a separate
  downstream gap** (the JIT's TypedObject field-read codegen doesn't
  thread the field kind through to the BinaryOp emission). Belongs to
  a follow-up sub-cluster — out of scope for the conduit.

**Array literal (`let xs: Array<int> = [1, 2, 3, 4, 5]; print(xs[0] +
xs[1] + xs[2])`):**
- VM mode: prints `6` ✓
- JIT mode: `Rvalue::Aggregate` surface eliminated; same downstream
  `compile_binop_dynamic_arith` surface on the array-element-read
  addition.

Both Smoke 3 and the array smoke now compile past the original
`Rvalue::Aggregate` surface. The remaining surface is in different
JIT territory (kind tracking through TypedObject field access /
TypedArray scalar read → BinaryOp), tracked as a separate
sub-cluster.

**Forbidden patterns observed:** none reintroduced. Specifically: no
Bool-default fallback, no runtime tag_bits decode, no
"bridge"/"probe"/"helper"/"hop"/"translator"/"adapter"/"shim" framing.
The `ConcreteType::Void` sentinel is a real `ConcreteType` enum
variant per §2.7.5.1, not a placeholder kind — downstream consumers
fall through to legacy path on `Void`.

**ADR-006 amendment needed?** No. The conduit landed cleanly in the
existing §2.7.5 stamp-at-compile-time framework. The added field
`top_level_local_concrete_types` is publicly observable but
`#[serde(skip)]` (not on the wire); the existing `top_level_frame`
field has the same shape for `NativeKind` per-slot data. No ABI
change to the JIT FFI boundary; no new HeapKind variants.

**Findings surfaced for downstream sub-clusters:**

- **`compile_binop_dynamic_arith` after TypedObject field read** —
  Smoke 3's `p.x + p.y` hits this. The JIT TypedObject field access
  codegen returns the field bits but doesn't stamp the result's
  `NativeKind` for the subsequent `BinaryOp`. ADR-006 §2.7.5 / W10
  playbook §5. Likely territory of `W12-jit-stack-parallel-kind-track`
  or a separate JIT kind-tracker follow-up.
- **`compile_binop_dynamic_arith` after TypedArray scalar read** —
  same surface for `xs[0] + xs[1]`. Same fix-class as above.

**Deferred to future cluster (NOT cluster-0):**

- **W12-jit-typed-map-ffi** (`jit_v2_map_*` typed-HashMap FFI rebuild) —
  no smoke in the cluster-0 matrix uses HashMap; not a close blocker.
  Cluster-2 or later territory.

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
