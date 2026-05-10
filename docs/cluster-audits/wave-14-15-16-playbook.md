# Wave 14 + 15 + 16 — Large Team Phase-2c Closure

**Branch parent:** `bulldozer-strictly-typed` HEAD `2b74389` (W13 Round 2 close).
**Goal:** close W14 (variant codegen), W15 (Stage C HeapKind family), W16 (op_call_method dispatch shell) in one large parallel push.

This playbook is the **shared ruleset** for all 8 agents. Read §0 first.

---

## §0 — Shared ruleset (every agent reads this)

### Forbidden patterns (refuse on sight)

1. **`ValueWord` / `ValueWordExt` / `ValueBits` / `tag_bits::*` resurrection** — these are deleted; do not import under any name.
2. **`Bool-default fallback`** for unknown kind. Surface-and-stop instead.
3. **Defection-attractor framing:** any of these naming patterns is refused on sight:
   - `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` per CLAUDE.md broader-family rule.
4. **`unimplemented!()` without explicit Phase-2c surface comment.** Either fill, or surface with `NotImplemented(SURFACE)` + ADR-006 §2.7.4 cite + named blocker.
5. **Resurrecting deleted shape under a renamed alias** (e.g., `LegacyResultData`, `OldRangeShape`).

### Required reading per agent

1. This playbook §0 + your sub-cluster section.
2. ADR-006 `docs/adr/006-value-and-memory-model.md` §2.7.6/Q8, §2.7.9 (FilterExpr precedent), §2.7.10/Q11, §2.7.11/Q12, §2.7.15/Q16 (HashSet — your template), §2.7.16/Q17 (Iterator — your template).
3. CLAUDE.md "Forbidden Patterns" + "Renames to refuse on sight".
4. **W13-hashset-rebuild close commit `0da1477`** — your canonical recipe. `git show 0da1477` for the diff.
5. **W13-iterator-state close commit `52c8ef5`** — second canonical recipe.

### Pre-assigned HeapKind ordinals

To avoid merge collisions, each agent uses the pre-assigned ordinal for any new variant:

| Wave | Agent | Variant | Ordinal |
|---|---|---|---|
| W14 | variant-codegen | `HeapKind::Result` | 23 |
| W14 | variant-codegen | `HeapKind::Option` | 24 |
| W15 | priority-queue | `HeapKind::PriorityQueue` | 25 |
| W15 | deque | `HeapKind::Deque` | 26 |
| W15 | channel | `HeapKind::Channel` | 27 |
| W15 | column | `HeapKind::Column` | 28 |
| W15 | matrix | `HeapKind::Matrix` | 29 |
| W15 | range | `HeapKind::Range` | 30 |
| W16 | op-call-method | (no new variant) | — |

**Rule:** if your ordinal is already taken at edit time, **bump to the next free** and add a comment "ordinal X (not the originally drafted Y) — agent <Z> took Y first at merge time" — same precedent as W8-T25/T26 (19↔20) and W13-hashset/iterator (21↔22).

### Required dispatch tables for new HeapKind variants

Per the §2.7.9 FilterExpr / §2.7.13 Reference / §2.7.15 HashSet precedents, every new `HeapKind` variant requires arms added to:

1. `crates/shape-vm/src/executor/vm_impl/stack.rs::clone_with_kind` (retain)
2. `crates/shape-vm/src/executor/vm_impl/stack.rs::drop_with_kind` (release)
3. `crates/shape-value/src/kinded_slot.rs::KindedSlot::Drop`
4. `crates/shape-value/src/kinded_slot.rs::KindedSlot::Clone`
5. `crates/shape-value/src/v2/closure_layout.rs::SharedCell::drop`
6. `crates/shape-value/src/heap_value.rs::TypedObjectStorage::drop`

Plus knock-on `kind_type_name` maps in:
- `crates/shape-vm/src/executor/printing.rs`
- `crates/shape-vm/src/executor/arithmetic/mod.rs`
- `crates/shape-vm/src/executor/comparison/mod.rs`
- `crates/shape-vm/src/executor/objects/typed_access.rs`

Plus wire/JSON conversion arms (rejection or proper) in:
- `crates/shape-runtime/src/wire_conversion.rs`
- `crates/shape-runtime/src/json_value.rs` (if needed)

### Required ADR amendment text per W15 sub-cluster

Each W15 agent appends a §2.7.{17-22} / Q{18-23} amendment to ADR-006 (mirror of §2.7.15 / Q16 HashSet precedent). Agents pick their §-numbers in the same lockstep as ordinals:

| Agent | ADR § | Q |
|---|---|---|
| W14-variant-codegen | §2.7.17 | Q18 |
| W15-priority-queue | §2.7.18 | Q19 |
| W15-deque | §2.7.19 | Q20 |
| W15-channel | §2.7.20 | Q21 |
| W15-column | §2.7.21 | Q22 |
| W15-matrix | §2.7.22 | Q23 |
| W15-range | §2.7.23 | Q24 |

If your §-number is taken at edit time, bump to next free + same provenance comment as ordinal-bump rule.

### Gates per sub-cluster

- `cargo check --workspace --all-targets` exit 0 (preserve from W13 close)
- `bash scripts/check-no-dynamic.sh` exit 0
- AGENTS.md row added (idle) with close commit hash
- Per sub-cluster: at least 4 unit tests in shape-value (storage layer) for the new data type (insert/remove/iterate/empty pattern, mirror of HashSetData tests)

### Surface-and-stop is acceptable

If your sub-cluster's bodies depend on:
- `op_call_method` dispatch shell (W16 — chained method calls) → leave that handler SURFACE, cite W16
- `op_make_closure` bodies that aren't filled (none should be at this point — verify) → cite §2.7.4
- A schema gap (e.g., your data type needs a built-in schema that doesn't exist) → leave SURFACE + cite shape-runtime/src/type_schema/builtin_schemas.rs

Surface-and-stop is **not** a free pass — surface only on genuinely external dependencies, not on self-imposed scope cuts.

---

## §1 — W14: variant codegen (Result + Option)

**Single agent: W14-variant-codegen.**

**Territory:**
- New `HeapKind::Result = 23`, `HeapKind::Option = 24` variants
- New `HeapValue::Result(Arc<ResultData>)` + `HeapValue::Option(Arc<OptionData>)` arms
- New schemas in `crates/shape-runtime/src/type_schema/builtin_schemas.rs::register_builtin_schemas` — `__Result` (variants: Ok, Err) + `__Option` (variants: Some, None)
- Fill 3 ctor bodies: `BuiltinFunction::SomeCtor` / `OkCtor` / `ErrCtor` in `vm_impl/builtins.rs`
- Fill 8 op_* bodies in `crates/shape-vm/src/executor/exceptions/mod.rs`: `op_type_check`, `op_error_context`, `op_try_unwrap`, `op_unwrap_option`, `op_is_ok`, `op_is_err`, `op_unwrap_ok`, `op_unwrap_err`

**Architectural shape:**
```rust
struct ResultData { is_ok: bool, payload: KindedSlot, /* unused half None */ }
struct OptionData { is_some: bool, payload: KindedSlot }
```
or alternately as `enum`s with discriminator-tagged variants. Audit the pattern against existing TypedObject precedent in `objects/object_creation.rs::op_new_typed_object` and choose what's cleanest.

**Smoke target:** `let r = Ok(42); if r.is_ok() { print(r.unwrap()) }` outputs `42`.

**Time:** 4-6h. The largest single agent.

---

## §2 — W15: Stage C HeapKind family (6 mirror-of-HashSet rebuilds)

Each is a separate agent following the W13-hashset recipe verbatim.

### W15-priority-queue

**Territory:** new `HeapKind::PriorityQueue = 25`, `HeapValue::PriorityQueue(Arc<PriorityQueueData>)`. PriorityQueueData is a min-heap or max-heap over `KindedSlot` payloads (audit pre-bulldozer shape — `git log --all --oneline | grep -i priority` to find the reference). Methods at `crates/shape-vm/src/executor/objects/priority_queue_methods.rs` (audit for SURFACE list).

**Smoke:** `let pq = PriorityQueue(); pq.push(3); pq.push(1); pq.push(2); pq.pop()` returns `1`.

**Time:** 3-4h.

### W15-deque

**Territory:** `HeapKind::Deque = 26`, `HeapValue::Deque(Arc<DequeData>)`. Double-ended queue over `KindedSlot` payloads. Methods at `objects/deque_methods.rs`.

**Smoke:** `let d = Deque(); d.push_back(1); d.push_front(0); d.pop_back()` returns `1`.

**Time:** 3-4h.

### W15-channel

**Territory:** `HeapKind::Channel = 27`, `HeapValue::Channel(Arc<ChannelData>)`. Concurrency primitive — mpsc-style. Methods at `objects/channel_methods.rs`. **NOTE:** Channel is async-context-aware — verify your impl integrates with the §2.7.4 task-scheduler boundary cleanly. If async integration requires Phase-2c work, leave async paths SURFACE'd.

**Smoke:** `let c = Channel(); c.send(1); c.recv()` returns `1` (sync send/recv in same thread).

**Time:** 4-5h.

### W15-column

**Territory:** `HeapKind::Column = 28`, `HeapValue::Column(Arc<ColumnData>)`. Single typed-buffer column (audit: was this redundant with `TypedArrayData`? if so, surface the redundancy and propose either drop or keep with rationale). Methods at `objects/column_methods.rs`.

**Smoke:** depends on audit.

**Time:** 3-4h. **Audit may reveal this is a deletion candidate, not a rebuild.**

### W15-matrix

**Territory:** `HeapKind::Matrix = 29`, `HeapValue::Matrix(Arc<MatrixData>)`. 2D typed buffer (rows × cols, single element kind). Methods at `objects/matrix_methods.rs`. The largest data-shape design.

**Smoke:** `let m = matrix([[1,2],[3,4]]); m.transpose()[0][0]` returns `1`.

**Time:** 4-6h.

### W15-range

**Territory:** `HeapKind::Range = 30`, `HeapValue::Range(Arc<RangeData>)` with `{ start, end, step, inclusive }`. Methods at `objects/range_methods.rs`. **Possible overlap with `IteratorState`** (W13-iterator-state); audit and document the boundary — Range may be just an iterator factory.

**Smoke:** `(0..5).iter().collect()` returns `[0, 1, 2, 3, 4]`.

**Time:** 3-4h.

---

## §3 — W16: op_call_method dispatch shell

**Single agent: W16-op-call-method.**

**Territory:** the §2.7.10/Q11 dispatch shell at `crates/shape-vm/src/executor/objects/mod.rs:303` (or wherever it lives — audit). The MethodFnV2 ABI has been live since W6.5 close (`5ac1e89`); this fills the shell that pops args, classifies receiver, dispatches via PHF.

**Smoke target:** `let s = Set(); s.add("a"); print(s.size())` outputs `1` (chained method calls work).

**Body shape (per W7-op-call-value precedent):**

```rust
fn op_call_method(&mut self, instruction: &Instruction) -> Result<(), VMError> {
    let arg_count = /* from operand */;
    let method_name_id = /* from operand */;
    let mut args: Vec<KindedSlot> = Vec::with_capacity(arg_count + 1);
    for _ in 0..(arg_count + 1) {  // +1 for receiver
        let (bits, kind) = self.pop_kinded()?;
        args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
    }
    args.reverse();
    // Classify receiver kind, look up method handler in PHF (method_registry.rs)
    let handler = lookup_method_for_receiver(&args[0].kind, method_name_id)?;
    let result = handler(self, &args, ctx)?;
    self.push_kinded(result.slot.into_raw(), result.kind)?;
    std::mem::forget(result);  // share transferred to stack
    Ok(())
}
```

**Time:** 3-4h.

---

## §4 — Dispatch protocol

1. Supervisor pre-creates worktrees + branches per the team list.
2. Agents work in parallel. Each owns its territory; no overlap on data structs / methods. Conflicts at merge time on:
   - HeapKind enum (per-line ordinal addition — auto-merges if ordinals are pre-assigned)
   - 4 dispatch tables (per-line arm addition)
   - vm_impl/builtins.rs ctor block (per-line arm)
3. Merge order (deterministic): W14 first, then W15 by ordinal (PriorityQueue → Deque → Channel → Column → Matrix → Range), then W16.
4. Each merge resolves trivially: take both sides, ordinal already pre-assigned.
5. Workspace check between merges to catch any cross-cluster surface.

---

## §5 — Out of scope (Wave 17+)

- §2.7.14 JitArray rebuild (Q15, multi-day, formal deferral)
- Snapshot/restore in-flight async (multi-week)
- Foreign-fn kind-blind retain/release ABI

---

*Playbook closed for edits during fan-out.*
