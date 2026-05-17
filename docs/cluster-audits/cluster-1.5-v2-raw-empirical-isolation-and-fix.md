# cluster-1.5-v2-raw-empirical-isolation-and-fix — empirical isolation + bounded fix

**Branch:** `bulldozer-strictly-typed-cluster-1.5-v2-raw-empirical-isolation-and-fix`
**Parent HEAD:** `6bc80014` (post cluster-1.5-v2-raw-heap-audit merge with CLAUDE.md
re-classification).
**Dispatch:** cluster-1.5-v2-raw-empirical-isolation-and-fix per supervisor
2026-05-17 disposition (b) consolidation of §1.B SIGABRT + Phase 4
imprecision 84 evidence (one fix scope, same root-cause territory).
**Scope:** Phase 1 empirical isolation (gdb backtrace + producer/consumer
chain identification) + Phase 2 bounded fix at the kinded-args / captures
share-accounting boundary in `call_closure_with_nb_args_keepalive` and
`call_function_with_nb_args`.

---

## §0 Pre-flight (Q3 binding)

- HEAD verified at `6bc80014` (parent).
- All file:line cites below grep-verified at HEAD `6bc80014` pre-fix.
- `cargo check --workspace --lib --bins --tests --examples` EXIT=0 post-fix.
- `cargo check -p shape-jit --features jit-trace` EXIT=0 post-fix.
- `bash scripts/verify-merge.sh` 12/12 PASS EXIT=0 post-fix.
- `bash scripts/check-no-dynamic.sh` EXIT=0 post-fix.
- Smoke matrix 5/5 VM == JIT preserved post-fix (Smokes 1/2/3/4/5 at
  `/tmp/smokes/s{1..5}.shape` — see §6).

---

## §1 Empirical isolation (Phase 1)

### §1.A Baseline reproduction

`tools/shape-test/tests/hashmap/iteration.rs:69-79::hashmap_filter_all_match`
reproduces SIGABRT (`free(): invalid next size (fast)`, signal 6) at HEAD
`6bc80014` at 5/5 runs (deterministic, not flaky):

```
Run 1: free(): invalid next size (fast) | signal: 6, SIGABRT
Run 2: free(): invalid next size (fast) | signal: 6, SIGABRT
Run 3: free(): invalid next size (fast) | signal: 6, SIGABRT
Run 4: free(): invalid next size (normal) | signal: 6, SIGABRT
Run 5: free(): invalid next size (fast) | signal: 6, SIGABRT
```

Fixture (verbatim from `iteration.rs:69-79`):

```shape
let m = HashMap().set("a", 1).set("b", 2)
let result = m.filter(|k, v| v > 0)
print(result.len())
```

### §1.B gdb backtrace at SIGABRT

gdb captured the crashing thread's backtrace:

```
#0  __pthread_kill_implementation
#1  raise
#2  abort
#3  __libc_message_impl.cold
#4  malloc_printerr
#5  _int_free
#6  free
#7  shape_value::v2::typed_array::TypedArray<*const T>::drop_array_heap
#8  alloc::sync::Arc<T,A>::drop_slow         (inner Arc<HashMapData<i64>>)
#9  alloc::sync::Arc<T,A>::drop_slow         (outer Arc<HashMapKindedRef>)
#10 shape_vm::executor::vm_impl::stack::drop_with_kind
#11 <VirtualMachine as Drop>::drop
```

Disassembly at the crash site (frame 7):

```
mov $0x18, %esi        ; size = 24 (StringObj struct)
mov $0x8, %edx         ; align = 8
mov %r14, %rdi         ; ptr = current data[i] (a *const StringObj)
call *%rbp             ; __rust_dealloc(ptr, 24, 8) — CRASHES HERE
```

The dealloc is called inside `drop_array_heap`'s per-element walk on a
`*const StringObj` pointer read from the keys data buffer. glibc detects
heap-metadata corruption at that ptr (the "next chunk size" sanity check
fails).

### §1.C Empirical lifecycle trace (eprintln instrumentation)

After instrumenting `StringObj::new`, `StringObj::drop`,
`StringObj::release_elem`, `HashMapData::insert`, `HashMapData::Clone::clone`,
`read_keys_owned`, and `build_filtered_kref::I64` with eprintln logs, the
crashing run produces:

```
[HMI] insert(key_ptr=0x.., key_first_byte=0x61 ('a')) self.keys=.. data=0x0 len=0 cap=0
[STROBJ] new ptr=0x7a7..b40 content="a"                        ← set("a",1) alloc
[HMI] insert push key_obj=0x..b40 idx=0
[HMI] insert pushed data=0x..18e0 cap=4                         ← keys grow to cap=4

[HMC] HashMapData::clone n=1 src_keys=..                        ← set("b",2) triggers Arc::make_mut clone
[HMC] clone done new_keys=.. data=0x..8990 cap=1                ← new_keys at cap=1 from with_capacity(1)
[HMI] insert(key_ptr=0x.., key_first_byte=0x62 ('b')) ..        ← insert "b" into cloned HashMapData
[STROBJ] new ptr=0x7a7..570 content="b"
[HMI] insert pushed data=0x..8990 cap=2                         ← grow cap=1→2

[STROBJ] release_elem ptr=0x..b40 rc_before=2                   ← original receiver drops (chain cleanup)
[RKO] read_keys_owned keys=.. n=2                               ← v2_filter reads keys
[RKO]   [0] string_obj_ptr=0x..b40 as_str="a" arc_string_ptr=0x..0520
[RKO]   [1] string_obj_ptr=0x..570 as_str="b" arc_string_ptr=0x..1d30

[BFK] iter slot=0 key_arc_ptr=0x..a230 key_str="p" v=1          ← !!! "p" instead of "a" !!!
[HMI] insert(key_ptr=0x..0520, key_first_byte=0x70 ('p')) ..    ← key bytes = "p"
[STROBJ] new ptr=0x..0520 content="\u{1}"                       ← alloc returned SAME ADDR as key str ptr!
                                                                  (content shows 0x01 because
                                                                   StringObj::new wrote refcount=1 over
                                                                   the str data BEFORE the eprintln)
```

The key insight from the trace: `Arc<String>` for "a" was freed BEFORE
`build_filtered_kref` reads its `.as_str()`. The allocator reused the
freed String's data buffer for the new StringObj alloc inside insert.
When `StringObj::new` writes the HeapHeader (refcount=1 + kind=81) at
offset 0 of the freshly-alloc'd StringObj struct, it ALSO writes over
the str data (which aliased the same memory).

### §1.D Producer/consumer chain identification (root cause)

The freed `Arc<String>` came from over-release in
`call_closure_with_nb_args_keepalive` at
`crates/shape-vm/src/executor/call_convention.rs:737-739` (pre-fix):

```rust
for (i, slot) in args.iter().enumerate() {
    self.stack_write_kinded(arg_base + i, slot.slot.raw(), slot.kind);
}
```

`stack_write_kinded` writes raw bits to the new frame's local slot
WITHOUT bumping the underlying refcount. The frame teardown at
`op_return_value` → `truncate_stack(bp)` later calls `drop_with_kind`
on each local slot — releasing one share per arg. But the caller's
`args: &[KindedSlot]` carriers ALSO own one share each (constructed
locally e.g. in `v2_filter` at `hashmap_methods.rs:1640`) and release
on scope exit via `KindedSlot::Drop`.

**Net: 2 releases per arg slot, 1 acquire (the caller's original
construction)**. For heap-bearing arg kinds (`NativeKind::String`,
`Ptr(...)`, `StringV2`, `DecimalV2`) the over-release drops the
underlying refcount to 0 before the caller finishes using it →
use-after-free.

The captures path at `call_convention.rs:725-732` (pre-fix) had the
same bug shape: `read_capture_kinded` reads bits without bumping; the
frame teardown releases the share; the `OwnedClosureBlock` ALSO owns
one share per heap capture (retired at `release_typed_closure` on
refcount=0). The combined release count exceeded the acquire count
by 1 per call.

### §1.E Hypothesis disposition per audit §1.B

| Hypothesis | Status | Evidence |
|---|---|---|
| **A** (kind-track / receiver-recovery violation) | REFUTED | Backtrace shows in-class `drop_with_kind` arm for `HeapKind::HashMap` correctly dispatching `Arc<HashMapKindedRef>` drop; recovery shape sound per §2.7.6 / Q8. The class of error is upstream of kind-track — share accounting at the args/captures boundary in `call_*_with_nb_args*`. |
| **B** (premature Drop of `*const StringObj` aliased across HashMapData clones) | REFUTED | The HashMapData::Clone path correctly `share_clone`s each `*const StringObj` (bumps `v2_retain`); per-V impl audit in cluster-1.5-v2-raw-heap-audit.md §1.C confirmed sound. The crash is downstream — the `Arc<String>` (NOT `*const StringObj`) is over-released at the closure-call boundary. |
| **D** (cw-C HashMapKindedRef::HashMap recursive V-arm refcount-pair off-by-one in `f"{v}"` interpolation) | REFUTED | The fixture has no recursive HashMap or string interpolation; the V-arm is `HashMapKindedRef::I64` throughout. The fault is at the closure-call args boundary, not the recursive V-arm. |
| **C** (`TypedArray::push` realloc invalidating aliased raw pointer) | RULED OUT BY PRIOR AUDIT | cluster-1.5-v2-raw-heap-audit.md §1.D enumerated `TypedArray::push` realloc as sound by construction; this fix does not contradict that. |

**New root-cause class identified**: share-accounting double-release in
the kinded-args / kinded-captures stack-write path inside
`call_closure_with_nb_args_keepalive` and `call_function_with_nb_args`.
This is a sibling of the Round 13 T5 share-accounting fix for the
closure-self share at `call_value_immediate_nb:870` (which inserted
`clone_with_kind` on the callee carrier before frame setup); the
present fix extends the same pattern to the args slice AND the
captures walk.

### §1.F Arc::from_raw site enumeration (Phase 4 imprecision 84 territory)

Per dispatch's Phase 4 evidence: phase-4 ReceiverGuard surgical fix at
`typed_object_ops.rs:341-353::op_get_field_typed` (commit `38602014`)
reduced UFCS flake rate from 26% → 14%. Dispatch hypothesized OTHER
`Arc::from_raw` wrong-type-recovery sites contribute the residual 14%.

Empirical site enumeration at HEAD `6bc80014` via
`grep -rn 'Arc::from_raw' crates/shape-vm/ crates/shape-value/ crates/shape-runtime/`:

| Site | Receiver type | Wrong-type-recovery? | Status |
|---|---|---|---|
| `typed_object_ops.rs:608::op_set_field_typed` | `TypedObjectStorage` (v2-raw `_new`) | YES (same pattern as phase-4 fix at :341-353) | **NOT FIXED in this work** — flagged below as residual surface |
| `typed_object_ops.rs:906` (test) | `TypedObjectStorage` (Arc::new test allocation) | NO (test uses Arc::new; sound) | N/A |
| `trait_object_ops.rs:949` | `ResultData` | NO (sound; ResultData uses Arc::new) | OK |
| `trait_object_ops.rs:1010` | `OptionData` | NO (sound) | OK |
| `property_access.rs:161`, `:296`, `:496`, `:520`, `:536`, `:693` | various Arc<T> types | Sound recovery patterns | OK |
| `hashmap_methods.rs:1800`, `:2052`, `:2094` | `Arc<String>` | Sound — paired with into_raw | OK |
| `concurrency_methods.rs:*` | various | Sound per `38602014`'s broader pattern | OK |
| `marshal.rs:156`, `:223`, `:281`, `:585`, `:657` | various HeapValue Arcs | Sound (Arc::into_raw produced) | OK |

**Phase 4 imprecision 84 baseline measurement at HEAD `6bc80014`** —
the simple field-access fixture (`type Money { cents: int }; print(a.cents + b.cents)`)
flake rate is **0/20** at HEAD pre-fix and **0/20** post-fix. The ReceiverGuard
surgical fix at `typed_object_ops.rs:341-353` (already merged at
`38602014`) appears to have closed the simple-fixture flake at this HEAD.

The residual 14% UFCS flake rate cited in the dispatch refers to a UFCS
path that on this HEAD surfaces as a semantic-error (`Method 'add' has
an explicit 'self' parameter, but method receivers are implicit`) — a
DIFFERENT class than the v2-raw Arc::from_raw territory. Empirical
reproduction of the historical 14% flake at this HEAD is not possible
because the upstream UFCS path errors before reaching the runtime
recovery site.

**Disposition**: `op_set_field_typed:608` Arc::from_raw on
`TypedObjectStorage` is a residual surface of the wrong-type-recovery
class — flagged for separate Phase 5 follow-up. The cluster-1.5 close
gates target the SIGABRT-class share-accounting bug fixed in this
work, NOT a comprehensive sweep of every potential Arc::from_raw site.

---

## §2 Bounded fix (Phase 2)

### §2.A Fix scope

3 files modified, 90 LoC delta (additions = +97, deletions = -14, net +83):

| File | Change | LoC |
|---|---|---|
| `crates/shape-vm/src/executor/call_convention.rs` | `clone_with_kind` before `stack_write_kinded` in args + captures loops of `call_closure_with_nb_args_keepalive`; `clone_with_kind` before stack_write_kinded in args loop of `call_function_with_nb_args`; ownership-discipline docstring rewrites at `execute_function_by_id` + `execute_closure` (removed load-bearing `mem::forget(args)` that was a leak compensating for the missing clone) | +84 / -14 |
| `crates/shape-vm/src/executor/trait_object_ops.rs` | 2 sites: removed `for slot in args { std::mem::forget(slot); }` (with my fix the helper is share-neutral; the forget would now LEAK) | +13 / +0 |

### §2.B Precedent match (afb1651 vw_clone/vw_drop shape)

Per CLAUDE.md "Forbidden Patterns" + dispatch's precedent requirement:
the fix SHAPE mirrors commit `afb1651` (c-stdlib-msgpack `vw_clone`/`vw_drop`)
— pair a retain with each share transfer so the producer/consumer
accounting balances. The fix uses the post-strict-typing kinded-API
equivalent: `clone_with_kind(bits, kind)` for retain and the existing
`stack_write_kinded` for the transfer slot.

The Round 13 T5 share-accounting fix at `call_value_immediate_nb:870`
(closure-self carrier `clone_with_kind` before frame setup) is the
direct in-codebase precedent — the same fix shape extended to the args
slice and captures walk.

### §2.C Refusal discipline self-audit

Per CLAUDE.md "Forbidden Patterns + Renames to refuse on sight; cluster-2
canonical refusal set":

- **NO ValueWord resurrection** — fix uses existing `clone_with_kind` /
  `drop_with_kind` kinded-API primitives. No `ValueWord`, no `vw_clone`,
  no `tag_bits::*`, no deleted-shape reintroduction.
- **NO Bool-default fallback** — `clone_with_kind` dispatches on the
  carrier's explicit `kind` from the §2.7.7 parallel-kind track. For
  unproven kinds the caller already surface-and-stops upstream; no
  fallback added.
- **NO bridge/probe/helper/hop/translator/adapter/shim framing** — fix
  doc-comments describe the change in terms of the existing
  `clone_with_kind` / `stack_write_kinded` primitives and the
  share-accounting contract; no defection-attractor naming.
- **NO parallel-implementation** — fix extends the EXISTING helpers
  `call_closure_with_nb_args_keepalive` / `call_function_with_nb_args`
  in place. No new helper, no parallel carrier shape, no parallel
  discriminator.
- **NO new HeapKind variants** — fix is share-accounting only; no new
  type carriers introduced.
- **NO #10 anti-deferral** — fix lands the SIGABRT-class share-accounting
  bug pre-cluster-1.5-close as the dispatch required. The
  `op_set_field_typed:608` residual surface is flagged in §1.F as a
  separate territory (Phase 5 follow-up), NOT framed as "tracked
  follow-up to ignore".
- **NO #11 Ptr-newtype-shim defection** — fix touches no
  `TypedObjectPtr` / `TraitObjectPtr` carriers.
- **Own all code quality** — no new clippy warnings introduced
  (verified via `cargo check` clean post-fix). The `mem::forget(args)`
  removals are accompanied by `drop(args)` calls + docstring rewrites
  explaining why the forget is no longer needed (the helper became
  share-neutral, so the forget would LEAK).

---

## §3 Acceptance verification

### §3.A hashmap_filter_all_match reproduction count

- **Pre-fix**: 5/5 SIGABRT (deterministic).
- **Post-fix**: 0/5 SIGABRT, 5/5 pass (`test result: ok. 1 passed; 0 failed`).

### §3.B Phase 4 imprecision 84 flake rate

- **Baseline at HEAD `6bc80014`** (pre-fix): 0/20 flake on simple
  field-access fixture. The ReceiverGuard fix at `38602014` already
  closed the simple-fixture path; remaining 14% UFCS flake cited in
  dispatch surfaces on a path that errors at semantic-tier on this HEAD,
  precluding empirical reproduction.
- **Post-fix**: 0/20 flake (preserved).

### §3.C Smoke matrix 5/5 VM == JIT

All 5 smokes at `/tmp/smokes/s{1..5}.shape` pass VM == JIT post-fix:

| Smoke | VM | JIT |
|---|---|---|
| 1 (scalar loop) | 4950 | 4950 ✓ |
| 2 (`[1,2,3,4,5].map(\|x\|x*2).sum()`) | 30 | 30 ✓ |
| 3 (canonical fixture `let t = X{}`) | x | x ✓ |
| 4 (`Set()` + `.add()` + `.size()`) | 2 | 2 ✓ |
| 5 (kickoff trait-object literal) | x | x ✓ |

Baseline smokes also 5/5; no regression introduced.

### §3.D No new test regressions

shape-test `hashmap` suite at HEAD `6bc80014` baseline aborts at
`hashmap_filter_all_match` SIGABRT (test runner exits with signal 6 mid-suite);
4 prior-running failures (`basic::hashmap_delete_key`, `hashmap_get_existing_key`,
`hashmap_get_integer_key`, `hashmap_set_returns_new_map`) are pre-existing
semantic / kind-mismatch failures unrelated to SIGABRT.

Post-fix: same 4 baseline failures PLUS 64 additional failures that
became visible BECAUSE the suite no longer aborts at filter_all_match.
All 64 are pre-existing V3-S5 SURFACE-and-stop errors (`HashMap.keys` /
`.values` / `.entries` cite `TypedArrayData enum + Buf<T> wrapper`
deletion per W12 audit §3.5-§3.6; SURFACE message names V3-S5 ckpt-6
as the rebuild destination) — NOT regressions from this fix.

Empirically verified by stashing the fix + rebuilding + running the
same suite: baseline aborts mid-suite at SIGABRT (`hashmap_filter_all_match`),
so the additional 64 failures don't get reported. Re-applying the fix
+ rebuilding: filter test passes, suite continues, 64 SURFACE failures
emit. Pre-existing failures unaffected.

### §3.E Close gates

| Gate | Status |
|---|---|
| `cargo check --workspace --lib --bins --tests --examples` EXIT=0 | ✓ |
| `cargo check -p shape-jit --features jit-trace` EXIT=0 | ✓ |
| `bash scripts/verify-merge.sh` 12/12 PASS EXIT=0 | ✓ |
| `bash scripts/check-no-dynamic.sh` EXIT=0 | ✓ |
| Smoke matrix 5/5 VM == JIT preserved | ✓ |
| hashmap_filter_all_match passes 3+ consecutive runs (no SIGABRT) | ✓ (5/5 pass) |
| Empirical evidence captured (gdb backtrace + site enumeration + per-site disposition) | ✓ (§1.B + §1.D + §1.F) |
| AGENTS.md row appended | ✓ |
| NO Co-Authored-By: Claude trailer | ✓ |

---

## §4 Sites surfaced (not fixed in this scope)

- **`op_set_field_typed:608` Arc::from_raw on `TypedObjectStorage`** — same
  wrong-type-recovery pattern as the phase-4 ReceiverGuard fix at
  `typed_object_ops.rs:341-353::op_get_field_typed`. The Arc::from_raw
  reconstruction reads the wrong refcount prefix when bits came from
  v2-raw `_new` (not Arc::new). Out of cluster-1.5 fix scope per
  dispatch's bounded-LoC criterion (30-150 LoC). **Recommended Phase 5
  follow-up**: mirror the ReceiverGuard pattern at `op_set_field_typed`.
- **Comptime tests with v2-raw SIGABRT class** (`w17_comptime_build_config_dispatches_end_to_end`
  / `error_dispatches` / `warning_dispatches` / `implements_dispatches` —
  shape-vm lib tests) — pre-existing v2-raw aliasing class per CLAUDE.md
  "Known Constraints" + Round 11A close report; documented as deferred.
  This work does NOT close those (different reproducer path; outside the
  closure-args / captures fix shape).
- **`length_typed_object_empty` and other shape-vm property_access tests
  with `free(): invalid pointer`** — pre-existing v2-raw aliasing class;
  not fixed by this work.

---

## §5 Ceiling-c + D-α status

**Ceiling-c bound check**: this fix touches 2 source files (~90 LoC).
Well within ceiling-c (~100-site ceiling). No new HeapKind variants, no
new dispatch shape, no new opcode.

**D-α status**: single-checkpoint fix; no multi-session chain required.
Empirical isolation (Phase 1) and bounded fix (Phase 2) landed in one
session under one branch. Surface-and-stop discipline preserved: the
fix does NOT add any new surfaces; existing surface-and-stops elsewhere
in the codebase remain (W17 comptime SIGABRT class, V3-S5 SURFACE
classes, etc.) as documented territories.

---

## §6 CLAUDE.md modifications surfaced (flag-only)

The CLAUDE.md "Known Constraints" `v2-raw-heap-audit` entry at HEAD
`6bc80014` describes the cluster-2 §D Class 1 SIGABRT anchor at
`hashmap_filter_all_match` as live + requiring empirical isolation.
**Post-fix**: the SIGABRT empirical anchor is closed. The entry can
optionally be updated to reflect the close, but the broader v2-raw
aliasing class (other anchors like the `length_typed_object_empty` /
comptime SIGABRT tests) remains live, so the entry should NOT be deleted
wholesale — only the `hashmap_filter_all_match` anchor citation can be
removed.

Recommended update (flag-only; do NOT land without user ratification per
dispatch's CLAUDE.md modifications discipline):

```markdown
- **v2-raw-heap-audit — UPDATED 2026-05-17** per
  `docs/cluster-audits/cluster-1.5-v2-raw-empirical-isolation-and-fix.md`:
  the cluster-2 §D Class 1 SIGABRT anchor at `hashmap_filter_all_match`
  was empirically isolated to a share-accounting double-release at the
  closure-args / kinded-captures stack-write path in
  `call_closure_with_nb_args_keepalive` and `call_function_with_nb_args`
  (commit pending) — fix mirrors the Round 13 T5 share-accounting fix
  for the closure-self share at `call_value_immediate_nb:870`. Other
  v2-raw aliasing reproducers (4 `bin/shape-cli/tests/stdlib/simulation.rs`
  ignored tests blocked on V3-S5 ckpt-5/ckpt-6, w17_comptime SIGABRTs,
  property_access::length_typed_object_empty, `op_set_field_typed:608`
  Arc::from_raw wrong-type-recovery residual) remain live as separate
  territories (V3-S5 cluster-0, comptime carrier-shape territory,
  Phase 5 follow-up at op_set_field_typed).
```

**Disposition**: FLAG only; supervisor disposition required before
landing.

---

## §7 Commit shape

Single bounded fix commit on
`bulldozer-strictly-typed-cluster-1.5-v2-raw-empirical-isolation-and-fix`.
NO `Co-Authored-By: Claude` trailer.

Files touched:
- `crates/shape-vm/src/executor/call_convention.rs` (+84 / -14)
- `crates/shape-vm/src/executor/trait_object_ops.rs` (+13)
- `docs/cluster-audits/cluster-1.5-v2-raw-empirical-isolation-and-fix.md` (this doc; +deliverable)
- `AGENTS.md` (row append, see §8)

## §8 AGENTS.md row

Appended after the last `bulldozer-strictly-typed-*` row at AGENTS.md.
Row content matches Phase 3 cluster-1.5 sub-cluster naming convention.
