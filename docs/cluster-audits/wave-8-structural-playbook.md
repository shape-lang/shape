# Phase 1.B-vm Wave 8 — Structural Amendments + Small Surfaces Playbook

**Branch (parent):** `bulldozer-strictly-typed` HEAD `bbf622e` (W7 close).
**Predecessor:** Wave 7 close — kinded value-call dispatch ABI live.
**ADR binding:** ADR-006 §2.7.4 (Phase-2c discipline), §2.7.6/Q8 (carrier-API-bound),
§2.7.7 (stack parallel-kind), §2.7.8/Q10 (cell-storage parallel-kind),
§2.7.9 (FilterExpr precedent for HeapKind variant amendments),
§2.7.11/Q12 (just landed — value-call ABI).
**Anticipated new ADR amendments:** §2.7.12 (T25 SharedCell), §2.7.13 (T26 RefTarget).

This playbook covers 5 parallel sub-clusters — all dependency-orthogonal to each
other and to W7. Each fan-out target has a single agent.

---

## 1. Sub-clusters

### W8-T25 — `HeapKind::SharedCell` variant amendment

**Territory:**
- `crates/shape-value/src/heap_variants.rs` — add `SharedCell` ordinal to
  `HeapKind` enum (mirror of FilterExpr §2.7.9 amendment).
- `crates/shape-vm/src/executor/variables/mod.rs` — `op_alloc_shared_local`
  + `op_alloc_shared_module_binding` (currently NotImplemented; need T25 to
  gate the new variant).
- All 4 §2.7.9 dispatch tables: `clone_with_kind`/`drop_with_kind` in
  `vm_impl/stack.rs`, `KindedSlot` Drop/Clone in `kinded_slot.rs`,
  `SharedCell::drop` in `v2/closure_layout.rs` (yes — SharedCell drops itself
  via the new variant), `TypedObjectStorage::drop` in `heap_value.rs`.
- ADR-006 §2.7.12/Q13 amendment.

**Pattern:** mirror of W7 closure-retain. `Arc<SharedCell>` is the share carrier
(SharedCell is already an `Arc`-wrapped struct per §2.7.8 Wave-α B8 commit). Slot
bits = `Arc::into_raw(Arc<SharedCell>) as u64`. Each dispatch table arm:
`Arc::increment/decrement_strong_count(bits as *const SharedCell)`.

**Note on naming:** SharedCell-the-variant labels `Arc<SharedCell>`-shaped
payloads, distinct from existing kinds. There's no HeapValue::SharedCell arm —
this is a pure-discriminator HeapKind variant per §2.7.9 FilterExpr precedent
(`as_heap_value()` is unsound on SharedCell-labeled bits; the `&Arc<SharedCell>`
is recovered directly via `bits as *const SharedCell`).

**Agent budget:** ~2 hours.

### W8-T26 — `RefTarget`/`RefProjection` kinded redesign

**Territory:**
- `crates/shape-vm/src/executor/variables/mod.rs` — `op_make_ref`,
  `op_make_field_ref`, `op_make_index_ref`, `op_load_ref`, `op_store_ref`
  family (~10 NotImplemented sites; all currently surface "Phase-2c").
- ADR-006 §2.7.13/Q14 amendment specifying the kinded RefTarget shape.

**Pattern:** the deleted `nanboxed::RefTarget` was a ValueWord-shaped enum
projecting through chained type tags. Post-§2.7.11, the kinded RefTarget is a
`HeapValue::Reference(RefTarget)` arm carrying:
- `RefTarget::Local { frame_index: u32, slot_index: u32, kind: NativeKind }`
- `RefTarget::TypedField { receiver: Arc<HeapValue>, field_offset: u32, kind: NativeKind }`
- `RefTarget::TypedIndex { receiver: Arc<HeapValue>, index: u64, elem_kind: NativeKind }`

The `kind` field on each variant is the `NativeKind` of the *projected* slot —
threaded from the producing-opcode emit via `prove_native_kind()`. Loading a
ref reads the kinded slot via the same parallel-track pattern as
§2.7.7/§2.7.8. Storing through a ref writes via `stack_write_kinded`.

**This sub-cluster needs the ADR amendment FIRST.** The agent writes
§2.7.13/Q14 + drafts the body shape, surfaces back. Supervisor reviews. Then
agent migrates. **Pre-flight audit + ADR draft = surface to supervisor.**

**Agent budget:** ~3 hours including ADR draft.

### W8-EX — Exception handler rebuild

**Territory:**
- `crates/shape-vm/src/executor/exceptions/mod.rs` (~11 NotImplemented sites).
- `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs:49,66` (2 sites).
- `crates/shape-vm/src/executor/vm_impl/builtins.rs:315,613` (2 sites).

**Pattern:** Exception payloads are `KindedSlot` carriers per §2.7.6/Q8.
Throw/catch wires `KindedSlot` through the unwind path; no fabricated kind.
The `foreign_marshal` translates between Shape exceptions and host-side
errors (Result<_, VMError> at FFI boundaries) per §2.7.5 stable-FFI rules.

**Forbidden:** Bool-default for exception-payload kind; tag_bits decode at
catch-site; transitional shim names ("exception bridge", "throw probe", etc).

**Agent budget:** ~2 hours.

### W8-WJ — Window join rebuild

**Territory:**
- `crates/shape-vm/src/executor/window_join.rs` (~7 NotImplemented sites).

**Pattern:** window functions over a typed buffer (Array<number> or
Array<TypedObject>) materialize per-element kind via the `TypedArrayData`
arm match (already kinded per §2.7.7). The handler ABI mirrors §2.7.10/Q11
MethodFnV2 (these are method handlers attached via PHF). Each handler
signature: `fn(&mut VirtualMachine, args: &[KindedSlot], _ctx) -> Result<KindedSlot, VMError>`.

**This is mostly mechanical** — bodies follow the W6.5 §2.7.10 precedent
+ `array_sort.rs::handle_join_str_v2` recipe.

**Agent budget:** ~1.5 hours.

### W8-AS — Async/transport/remote preparatory ABI

**Territory:**
- `crates/shape-vm/src/executor/async_ops/mod.rs` (5 NotImplemented sites,
  including `op_await` from W7-cv-async surfaced).
- `crates/shape-vm/src/remote.rs` (2 sites).
- `crates/shape-vm/src/executor/builtins/{transport_builtins,remote_builtins}.rs`
  (3 sites each).
- `crates/shape-vm/src/executor/vm_impl/modules.rs` (1 site —
  `invoke_module_fn_id_stub`).

**Pattern:** Module-export typing rebuild. Each typed-module-export carries a
`KindedSlot` payload through transport (msgpack encode/decode preserves the
NativeKind discriminant). `op_await` integrates with W7-cv-async via the
`resolve_spawned_task` path that's now live.

**§2.7.4 Phase-2c boundary:** snapshot/restore of in-flight async tasks
stays `todo!()`. Sync resolution + module-export typing land here.

**Agent budget:** ~2-3 hours.

---

## 2. Dispatch order

All 5 sub-clusters fan out in parallel. **Independent territories**; merges in
any order. Single round.

| Sub-cluster | Files owned | Conflict zone |
|---|---|---|
| W8-T25 | shape-value/heap_variants.rs + 4 dispatch tables + variables/mod.rs alloc_shared opcodes | overlaps with W8-T26 in variables/mod.rs (different opcode regions) |
| W8-T26 | variables/mod.rs op_make_ref family | overlaps with W8-T25 (different opcode regions) |
| W8-EX | exceptions/mod.rs + foreign_marshal + 2 vm_impl/builtins.rs sites | none |
| W8-WJ | window_join.rs only | none |
| W8-AS | async_ops/, remote.rs, transport/remote builtins, vm_impl/modules.rs | none |

W8-T25 and W8-T26 both touch `variables/mod.rs` but at different line ranges
(T25 at lines 1455+, T26 at lines 167 + 2315+). 3-way merge should resolve
cleanly.

---

## 3. Forbidden patterns

Wave 7 §6 list applies verbatim. Add:

| # | Forbidden | Why |
|---|---|---|
| 19 | "shared-cell bridge / probe / helper / hop / translator / adapter / shim" | T25 defection-attractor family |
| 20 | "ref-target bridge / probe / ..." | T26 defection-attractor family |
| 21 | "exception bridge / throw probe / catch helper / unwind translator / boundary adapter" | EX defection-attractor family |
| 22 | Any non-kinded RefTarget shape (`u64` projection bits, tag-bit chains, ValueWord-shape revival) | T26 #1 — that's the deleted form |
| 23 | `HeapKind::SharedCell` variant added without ADR §2.7.12 amendment | T25 #1 — gated by §2.7.6/Q8 carrier-API-bound |

---

## 4. Wave-level gates

Mirror W7 §7 REVISED:

- **Gate 1**: `grep -c 'todo!('` in target files = 0 (or only Phase-2c
  out-of-scope per §2.7.4).
- **Gate 2**: zero W8-specific SURFACE messages in target files.
- **Gate 3**: `bash scripts/check-no-dynamic.sh` exit 0.
- **Gate 4**: `cargo build -p shape-vm --lib` succeeds.

---

## 5. Surface-and-stop triggers

- T26 ADR amendment unclear → surface to supervisor before migrating.
- T25 SharedCell payload is not `Arc<SharedCell>` (some other share shape) → surface.
- EX exception payloads cross a §2.7.4 Phase-2c snapshot boundary → stay `todo!()`.
- WJ depends on a W9 method body that's still SURFACE → leave that arm SURFACE.
- AS task-scheduler integration requires changes to `task_scheduler.rs` shape →
  surface; that's a §2.7.8 cell-extension.

---

## 6. What's NOT in W8

- ~150 method-body re-fill (W9).
- shape-jit consumer migration (W10).
- Test re-enable (W11).
- shape-cli polish (W11).
- Snapshot/restore (Phase-2c §2.7.4).

---

*Playbook closed for edits during fan-out. Amendments require supervisor
sign-off.*
