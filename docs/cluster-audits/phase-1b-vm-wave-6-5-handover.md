# Phase 1.B-vm Wave 6.5 substep-2 — Handover

**Date:** 2026-05-09
**Reason for handover:** prior session at 76% context after dispatching/closing Waves 1-6 + Wave 6.5 substep-1. Wave 6.5 substep-2 (cascade migration) is the next unit of work and needs a fresh context window.

## TL;DR — what to do immediately

1. Write `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` (~1 page) locking the shared-helper signatures, kind-sourcing rules per opcode category, and canonical `ValueWord`-construction rewrite pattern. Spec given in §6 below.
2. Create 5 worktrees on branches `bulldozer-strictly-typed-phase-1b-vm-cluster-{A,B,C,D,E}`, each branched from `bulldozer-strictly-typed-phase-1b-vm`'s HEAD.
3. Dispatch **3 parallel agents** (clusters A, B, C) first per user's preference for incremental fan-out.
4. As each cluster closes, merge back to `phase-1b-vm` and dispatch the next.
5. Final gate: `grep -rn 'push_raw_u64\|pop_raw_u64\|push_native_i64\|stack_read_owned\|stack_peek_raw' crates/shape-vm/src/` returns zero hits, AND `cargo check -p shape-vm --lib` errors strictly decreased toward 0.

## 1. Project context

Shape is an AI-native statically-typed language implemented in Rust. The "strict-typing bulldozer" is a multi-month migration deleting `ValueWord` (the historical 8-byte tag-decoding word) in favor of typed slots + per-FieldType constructors. Plan source: `~/.claude/plans/stop-native-vs-tagged-tax.md`. Defection-attractor history: CLAUDE.md "Forbidden Patterns" section names the W-series (9 commits of shim regressions). The discipline is hard — every "transitional shim" past sessions introduced became permanent debt. **No shims, ever.**

## 2. Phases closed (durable)

| Phase | Branch | Close commit | Merge | What landed |
|---|---|---|---|---|
| **1.A** | `bulldozer-strictly-typed-phase-1a` | `8567f81` | `94eb34d` | ADR-006 foundation: typed-Arc HeapValue payloads; per-FieldType `ValueSlot::from_*`; `TypedObjectStorage` struct |
| **1.B** | `bulldozer-strictly-typed-phase-1b` | `f218a5d` | `c5f6672` | shape-runtime --lib 62→0; `KindedSlot` carrier in shape-value; ADR §2.7+§2.7.4+§2.7.5+Q7; 16 `pending Phase 2c` stubs left |
| **2** (LSDS-1) | `bulldozer-strictly-typed-phase-2` | `7b89739` | (in main) | LSDS schema crate + B-series vertical slice + migration plan |

## 3. Phase 1.B-vm — current state

**Branch**: `bulldozer-strictly-typed-phase-1b-vm`
**Worktree**: `/home/dev/dev/shape-lang/shape-phase-1b-vm`

**Wave commits** (sequential on the branch):

| Wave | Commit | Δ shape-vm | What landed |
|---|---|---|---|
| 1 | `95d52bc` | -22 | `FrameDescriptor` §2.7.5.1 cleanup; `FrameAnalysisState` analysis-tier struct (`Vec<Option<NativeKind>>`); `NativeKind::Unknown` removed from wire format |
| 2 | `9d01005` | 0 (jit-side, vm cascade-blocked) | JIT `mir_compiler/*` analysis tier; `Vec<Option<NativeKind>>` per Wave 1 pattern; `cranelift_type_for_slot_opt` family |
| 3 | `3827bd1` | -31 | `KindedSlot::as_str()` accessor; variadic `register_typed_function` per-test-file thin wrappers; `TypedReturn::{Bool,Unit}` → `Concrete(ConcreteReturn::*)` |
| 4 | `4483ba1` | -16 | `printing.rs` `PrintResult`/`PrintSpan` import switch to `shape_runtime::output_adapter::*`; `RareHeapData` arm purge (7 variants + 4 `ConcurrencyData::*`) |
| 5a | `66e7b90` | -57 | KindedSlot bounded foundation (5 scalar accessors + 2 ctors + `coerce_to_f64` free helper); `vm_impl/builtins.rs` dispatch slice flip; `build_config` → `OpaqueTypedObject`; 29 `todo!()` stubs deferred to 5b-5e |
| 5b | `fa2bafc` | -156 | 33 BuiltinFunction body migrations (24 math + 8 array + 1 object_rest); `pop_builtin_args` runtime body w/ `NativeKind::Bool` transitional sentinel (Wave 6 surface) |
| 6.0 | `d782401` | -34 | Parallel `Vec<NativeKind>` stack track + `clone_with_kind`/`drop_with_kind` helpers; `pop_builtin_args` reads real kind; `stack_ops/mod.rs` and `logical/mod.rs` migrated. **Also introduced shim layer (REJECTED by supervisor; §2.7.7 explicit-forbid landed).** |
| 6.5 substep-1 | `11efd9c` | +907 (cascade exposed) | All 30 transitional shims deleted from `vm_impl/stack.rs`. Substep-2 (cascade migration) is the pending work. |

**Cumulative**: shape-vm `--lib` 2000 → **2591 errors** (after shim deletion exposed the cascade). shape-jit `--lib` ~2000 (Wave 10 territory, untouched). shape-value clean.

**Cluster-audit doc (binding for territory)**: `docs/cluster-audits/phase-1b-vm-valueword-callers.md` — has Wave 2 follow-up correction (Dynamic deleted) at the end.

## 4. ADR-006 sections that matter (binding)

Read each before any new code:

| Section | Q ruling | What it pins |
|---|---|---|
| §2.7 | Q7 | `KindedSlot { slot, kind }` carrier struct (not discriminator); shape-runtime tier only; NOT in VM↔JIT slot ABI |
| §2.7.1 | — | Per-site usage policy (STATIC_KIND vs GENERIC_CARRIER vs dispatch slice) |
| §2.7.2 | — | KindedSlot forbidden uses (no leakage into typed slot ABI, no variants) |
| §2.7.3 | — | Migration roadmap (Phase 1.B scope) |
| §2.7.4 | — | API rebuild scope (snapshot defer to Phase 2c, variadic `register_typed_function`, PrintResult moved, display/utility helper rulings, audit accuracy) |
| §2.7.5 | — | Cross-crate ABI: extension contracts stay raw u64, internal Rust uses KindedSlot, shape-jit/jit FFI raw |
| §2.7.5.1 | (part of Q1) | Wire-format structs are post-proof; `FrameDescriptor.slots: Vec<NativeKind>` no Option/Unknown; intermediate state in separate analysis struct |
| §2.7.6 | Q8 | KindedSlot API bounded by NativeKind cardinality; **no per-heap-variant accessors**; heap dispatch via `slot.as_heap_value()` + `HeapValue` match |
| §2.7.7 | Q9 | VM stack carries parallel `Vec<NativeKind>` track; `clone_with_kind`/`drop_with_kind` for WB2.4; **transitional shims explicitly forbidden** (the W-series-bug-class enumeration is in this section's "Forbidden shapes") |

## 5. Worktree map

```
/home/dev/dev/shape-lang/shape                  bulldozer-strictly-typed (supervisor)
/home/dev/dev/shape-lang/shape-phase-1a         bulldozer-strictly-typed-phase-1a (idle)
/home/dev/dev/shape-lang/shape-phase-1b         bulldozer-strictly-typed-phase-1b (idle)
/home/dev/dev/shape-lang/shape-phase-1b-vm      bulldozer-strictly-typed-phase-1b-vm (Wave 6.5 in progress)
/home/dev/dev/shape-lang/shape-phase-2          bulldozer-strictly-typed-phase-2 (idle)
/home/dev/dev/shape-lang/shape-stage-c-dev2     bulldozer-strictly-typed-stage-c-dev2 (idle)
```

For Wave 6.5 substep-2 fan-out, **create 5 new worktrees**:
```
/home/dev/dev/shape-lang/shape-phase-1b-vm-cluster-A
/home/dev/dev/shape-lang/shape-phase-1b-vm-cluster-B
/home/dev/dev/shape-lang/shape-phase-1b-vm-cluster-C
/home/dev/dev/shape-lang/shape-phase-1b-vm-cluster-D
/home/dev/dev/shape-lang/shape-phase-1b-vm-cluster-E
```

Each branched from `bulldozer-strictly-typed-phase-1b-vm`'s HEAD (`11efd9c` or whatever's there when fresh session starts):
```bash
cd /home/dev/dev/shape-lang/shape
git worktree add ../shape-phase-1b-vm-cluster-A -b bulldozer-strictly-typed-phase-1b-vm-cluster-A bulldozer-strictly-typed-phase-1b-vm
# repeat for B, C, D, E
```

## 6. The Wave 6.5 substep-2 cascade

**Scope**: ~1290 caller sites across 43 files for 5 mandatory shims (`push_raw_u64`, `pop_raw_u64`, `push_native_i64`, `stack_read_owned`, `stack_peek_raw`) + ~700 sibling-shim callers across the same files.

**Per-file error counts** (from agent's pre-substep-1 measurement):
- arithmetic 34→147 (+113 cascade)
- comparison 47→101 (+54)
- loops 43→58 (+15)
- control_flow 29→57 (+28)
- call_convention ~7→30 (+23)
- raw_helpers 57→57 (already broken; uses forbidden `tag_bits` — likely deletable)

### 6.1 Playbook to write (do this FIRST, before fan-out)

Write `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` containing:

**(A) Locked shared-helper signatures** — every cluster uses these exact signatures:
```rust
// crates/shape-vm/src/executor/builtins/kind_coerce.rs (Wave 5a free helper, extend)
pub(crate) fn coerce_to_f64(slot: &KindedSlot) -> Option<f64> { ... }  // EXISTING

// NEW helpers to add (in same file or sibling):
pub(crate) fn number_operand(slot: &KindedSlot) -> Result<f64, VMError> { ... }
pub(crate) fn int_operand(slot: &KindedSlot) -> Result<i64, VMError> { ... }
pub(crate) fn numeric_domain(slot: &KindedSlot) -> NumericDomain { ... }  // (define NumericDomain enum if not present)
```
Body shapes: `match slot.kind { ... } -> Result<T, VMError>` per the §2.7.6 heterogeneous-kind body pattern. **No KindedSlot accessor additions** — these helpers live at the call site.

**(B) Kind-sourcing rules per opcode category:**
- Typed-arith opcodes (`AddInt`, `SubFloat`, `MulInt`): result kind from opcode-name suffix (Int → `NativeKind::Int64`, Float → `NativeKind::Float64`, Decimal → `NativeKind::Ptr(HeapKind::Decimal)`)
- Comparison/logical (`EqInt`, `LtFloat`, `And`, `Or`, `Not`): always `NativeKind::Bool`
- Loop iteration values: from iterator's element FieldType (in scope at loop opcode)
- Function call returns: from `FrameDescriptor.return_kind` of the called function
- Stack manipulations (Dup/Swap/Rot): preserve via `read_kinded_raw` + `clone_with_kind`

**(C) Canonical `ValueWord`-construction rewrite pattern:**
```rust
// BEFORE
self.push_raw_u64(ValueWord::from_decimal(d).into_raw_bits())?;

// AFTER
let bits = Arc::into_raw(Arc::new(d)) as u64;
self.push_kinded(bits, NativeKind::Ptr(HeapKind::Decimal))?;
```
Generalize per HeapKind variant.

**(D) Re-state §2.7.7 forbidden shapes** with one-line summary each (ensures all clusters internalize):
- No shim re-introduction under any name
- No `Vec<KindedSlot>` for stack
- No 16-byte slots
- No tag bits packed in u64
- No `Option<NativeKind>` in kind track
- No `NativeKind::Unknown` / `Dynamic`
- No `is_heap()` probe / `as_heap_ref()` / `tag_bits` / `synthesize_value_word_from_raw`
- No `vw_clone` / `vw_drop`
- No "fall back to Bool kind if we don't know"

**(E) Per-cluster file lists** (cut from §6.2 below).

### 6.2 Cluster split

| Cluster | Files | ~Sites | Pattern |
|---|---|---|---|
| **A — Opcodes (numeric/logical)** | `executor/arithmetic/mod.rs`, `executor/comparison/mod.rs` | ~215 | Typed-arith opcodes; helpers per playbook (A) |
| **B — Control path** | `executor/control_flow/mod.rs`, `executor/loops/mod.rs`, `executor/call_convention.rs` | ~80 | Iterator element kind from FieldType; call return kind from FrameDescriptor |
| **C — Heap-side & VM_RAW** | `executor/objects/*` (selectively — those touching the deleted shims), `executor/objects/raw_helpers.rs`, `executor/typed_handlers/*`, `executor/v2_handlers/*` | ~400 | `slot.as_heap_value()` + HeapValue match per §2.7.6; typed-handler opcodes already raw, just shim-name swap |
| **D — Builtins backlog (5c+5d+5e)** | `executor/builtins/*` (the `todo!()` stubs Wave 5b deferred + the shim callers) + `executor/printing.rs` formatter | ~400 | Body migrations to `&[KindedSlot]` per Wave 5b pattern + printing.rs formatter rewrite |
| **E — Everything else** | `executor/exceptions/`, `executor/state_builtins/`, `executor/async_ops/`, `executor/vm_state_snapshot.rs`, `compiler/expressions/`, `compiler/comptime_target.rs`, `bytecode/{verifier,content_addressed}.rs`, `mir/*` (whatever has shim refs) | ~250 | Mostly mechanical caller migration |

### 6.3 Per-cluster agent prompt template

For each cluster A-E, the agent prompt should include:

1. Worktree path + branch name
2. Required reading (in order):
   - `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` (the playbook you wrote)
   - ADR-006 §2.7, §2.7.6, §2.7.7 (with the explicit-forbid clause)
   - `docs/cluster-audits/phase-1b-vm-valueword-callers.md` (for the cluster's audit recipe)
   - CLAUDE.md "Forbidden Patterns"
   - This handover doc for context
3. Cluster's specific file list + ~caller-site count
4. Reference: Wave 6.0's `executor/{stack_ops,logical}/mod.rs` shows the kinded migration pattern in action (already done; use as template)
5. Definition of done: every shim caller in cluster's files migrated, `cargo check -p shape-vm --lib` errors decreased, AGENTS.md row to `idle` with cluster-close commit hash
6. Forbidden: any shim re-introduction (under any name); KindedSlot leakage into VM stack ABI; per-§2.7.7 list
7. Surface-and-stop protocol: stop on cross-cluster needs, missing helpers, kind-sourcing gaps

### 6.4 Merge process (supervisor — me/the fresh session)

For each cluster as it closes:
1. Verify cluster's gate (cargo check decreased, no shim references in cluster's files)
2. From `/home/dev/dev/shape-lang/shape`: `git merge --no-ff bulldozer-strictly-typed-phase-1b-vm-cluster-X -m "Merge cluster X — Wave 6.5 substep-2 partial"` (NOT into supervisor's branch directly; merge cluster INTO `bulldozer-strictly-typed-phase-1b-vm`)
3. Fix AGENTS.md conflicts at merge (always conflicts; pattern matches prior wave merges)
4. Sync the cluster worktree: `cd ../shape-phase-1b-vm && git merge bulldozer-strictly-typed --no-ff -m "merge: pull cluster-X close"` (so phase-1b-vm worktree stays current)

Then dispatch the next cluster (D, E) once 3 of A/B/C have merged cleanly.

### 6.5 Final gate (when all 5 clusters merged)

```bash
cd /home/dev/dev/shape-lang/shape-phase-1b-vm
# Gate 1: shim function definitions deleted (already passes after substep-1)
grep -rn 'fn push_raw_u64\|fn pop_raw_u64\|fn push_native_i64\|fn stack_read_owned\|fn stack_peek_raw' crates/shape-vm/src/
# Expected: 0 hits

# Gate 2: shim function callers migrated
grep -rn 'push_raw_u64\|pop_raw_u64\|push_native_i64\|stack_read_owned\|stack_peek_raw' crates/shape-vm/src/
# Expected: 0 hits

# Gate 3: shape-vm compiles (or significantly closer to 0)
cargo check -p shape-vm --lib 2>&1 | grep -c "^error\["
# Target: significantly < 2591; ideal 0

# Gate 4: defection guard
bash scripts/check-no-dynamic.sh
# Expected: exit 0

# Gate 5: functional check — heap-bearing builtin runs end-to-end
# Add an integration test calling len(array) or similar; should not type-error at body entry
```

After gates pass, close Wave 6.5, flip AGENTS.md row to `idle` with the close commit, and merge `bulldozer-strictly-typed-phase-1b-vm` back into `bulldozer-strictly-typed` via `--no-ff` per phase-close convention.

## 7. Forbidden patterns (binding for all clusters)

From CLAUDE.md "Forbidden Patterns" + ADR-006 §2.7.x family:

**Forbidden code:**
- `ValueWord` at runtime (deleted)
- Generic opcodes (deleted)
- Runtime tag-decode hops (`is_tagged()`, `as_heap_ref()`, `tag_bits::*`, `synthesize_value_word_from_raw`)
- `Convert<X>To<Y>` opcodes
- `SlotKind::Dynamic` / `SlotKind::Unknown`
- `vw_clone` / `vw_drop` (replaced by `clone_with_kind` / `drop_with_kind`)
- Per-heap-variant `KindedSlot` accessors (Q8 / §2.7.6)
- `Vec<KindedSlot>` for the stack (§2.7.5 / §2.7.7)
- 16-byte stack slots
- `Option<NativeKind>` in stack kind track (post-proof per §2.7.5.1)
- `NativeKind::Unknown` / `NativeKind::Dynamic` (deleted)
- **Transitional shims preserving deleted ValueWord-shape names** (the §2.7.7 explicit-forbid landed in commit `efeb432`)

**Forbidden rationalizations** (refuse on sight):
- "Just a small fallback for this one edge case"
- "Mark this as a follow-up for a later phase"
- "Keep the shim until Wave N"
- "Document it as out-of-scope"
- "Add a feature flag"
- "Just one decode at the boundary"
- "Borrowed slot semantics — leak-free because Bool"

## 8. Reference commit hashes

```
ADR-006 anchor commits:
- 1230a7c  ADR-006 §2.7 + Q7: KindedSlot carrier ruling
- 58596f6  ADR-006 §2.7.4 + §2.7.5: phase-1b working-session ruling
- 4412cbd  ADR-006 §2.7.5.1: FrameDescriptor stays Vec<NativeKind>
- e3fa1ad  ADR-006 §2.7.6 + Q8: KindedSlot carrier API bound
- 31a98bb  ADR-006 §2.7.7 + Q9: VM stack ABI parallel Vec<NativeKind>
- efeb432  ADR-006 §2.7.7: explicit-forbid transitional shim layer (Wave 6.0 push-back)

Phase 1.B-vm wave commits:
- 95d52bc  Wave 1
- 9d01005  Wave 2
- 3827bd1  Wave 3
- 4483ba1  Wave 4
- 66e7b90  Wave 5a
- fa2bafc  Wave 5b
- d782401  Wave 6.0 (kept; load-bearing parts)
- 11efd9c  Wave 6.5 substep-1 (shim deletion)
- a3fbe7f  Merge: §2.7.7 explicit-forbid + Wave 6.5 dispatch row (on phase-1b-vm worktree)
```

## 9. Tool usage notes

- **Worktree per agent**: each cluster gets its own `git worktree add` so concurrent agents don't conflict on the file system. Sequential merges resolve conflicts at merge time.
- **AGENTS.md**: every roster row must be agent-editable; agents update their own row only. Schema/headers are immutable. Merge conflicts on AGENTS.md are routine — supervisor resolves at merge time by combining rows.
- **Background agents**: use `run_in_background: true` for the 3-cluster fan-out. Do NOT poll the output file (the runtime warned against it). Wait for completion notifications.
- **Surface-and-stop protocol**: agents stash WIP + flip AGENTS.md row to `blocked` + explain the architectural question. Supervisor rules, then re-dispatches with the ruling in the prompt.
- **Defection log**: `docs/defections.md` is append-only; new entries at end of file. Agents log "considered-but-rejected" decisions only.

## 10. Open architectural questions for clusters to potentially surface

These are NOT resolved; if a cluster trips over them, surface to supervisor for ruling:

1. **`raw_helpers.rs` deletion vs migration** — the file uses `tag_bits::*` / `is_tagged()` / `synthesize_value_word_from_raw` (all CLAUDE.md forbidden). Cluster C decides per-file whether to delete or migrate consumers off the tag-decode helpers.
2. **`numeric_domain` enum shape** — if the playbook's `NumericDomain` enum doesn't already exist, Cluster A pins its variants. Likely `Int(i64) | Float(f64) | Decimal(Arc<Decimal>) | BigInt(Arc<i64>)`.
3. **`pop_builtin_args` debug cross-check** — Wave 6.0 deferred per-position FrameDescriptor cross-check. Cluster B may run into it if function-call args span a kind-mismatch.
4. **shape-jit FFI cascade** — Wave 10 territory; if Cluster C's `typed_handlers/v2_handlers` migration cascades into shape-jit FFI, surface as Wave 10 territory creep.
5. **Build_config ConcreteReturn::OpaqueTypedObject pattern** — Wave 5a's existing pattern; clusters returning typed objects mirror it.

## 11. What you should NOT do

- Do not re-introduce shims (§2.7.7 explicit-forbid).
- Do not extend KindedSlot beyond Q8/§2.7.6 cardinality bound.
- Do not modify `clone_with_kind` / `drop_with_kind` dispatch tables (they mirror `KindedSlot::Drop`/`Clone`; divergence = refcount bug).
- Do not modify ADR-006 sections without explicit user sign-off (the user has approved Q1-Q9 through this session; new Q rulings need their own confirmation).
- Do not race to a "0 errors" close at the cost of correctness. Partial close at sub-cluster boundaries is acceptable.
- Do not bypass the supervisor-relay protocol. ARCHITECTURAL questions get surfaced; DETAIL decisions get made.
- Do not poll background agent output files — wait for notifications.
