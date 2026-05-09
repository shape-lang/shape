# Phase 1.B-vm Wave 6.5 substep-2 — Playbook

**Branch (parent):** `bulldozer-strictly-typed-phase-1b-vm` (HEAD `11efd9c`)
**Substep-1 closed:** `11efd9c` — 30 transitional shims deleted from `vm_impl/stack.rs`
**Substep-2 scope:** migrate every caller of every deleted shim across all 5 clusters
**Binding:** ADR-006 §2.7, §2.7.5, §2.7.5.1, §2.7.6, §2.7.7 (Q1–Q9 ruling list); CLAUDE.md "Forbidden Patterns"

This playbook is **binding for all 5 clusters**. Helper signatures, kind-sourcing rules, and the
canonical rewrite pattern are locked here so the parallel agents converge without coordinating.

---

## 1. Locked shared-helper signatures

These helpers live at the call site (§2.7.6 heterogeneous-kind body pattern), **not** as methods on
`KindedSlot`. Bundling kinds on the carrier surface is forbidden by Q8.

**File:** `crates/shape-vm/src/executor/builtins/kind_coerce.rs`

```rust
// EXISTING (Wave 5a) — DO NOT modify body without ADR sign-off.
#[inline]
pub(crate) fn coerce_to_f64(slot: &KindedSlot) -> Option<f64>;

// NEW — locked signatures. Bodies match the shape below verbatim.
#[inline]
pub(crate) fn number_operand(slot: &KindedSlot) -> Result<f64, VMError> {
    coerce_to_f64(slot).ok_or_else(|| VMError::TypeError(
        format!("expected int or float, got {:?}", slot.kind)
    ))
}

#[inline]
pub(crate) fn int_operand(slot: &KindedSlot) -> Result<i64, VMError> {
    match slot.kind {
        NativeKind::Int8 | NativeKind::Int16 | NativeKind::Int32 | NativeKind::Int64
        | NativeKind::IntSize | NativeKind::UInt8 | NativeKind::UInt16
        | NativeKind::UInt32 | NativeKind::UInt64 | NativeKind::UIntSize =>
            slot.as_i64().ok_or_else(|| VMError::TypeError(
                format!("expected integer, got {:?}", slot.kind)
            )),
        _ => Err(VMError::TypeError(
            format!("expected integer, got {:?}", slot.kind))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumericDomain {
    Int,      // Int8..Int64, IntSize, UInt8..UInt64, UIntSize (and nullable variants)
    Float,    // Float64, NullableFloat64
    Decimal,  // Ptr(HeapKind::Decimal)
    BigInt,   // Ptr(HeapKind::BigInt)
}

#[inline]
pub(crate) fn numeric_domain(slot: &KindedSlot) -> Result<NumericDomain, VMError> {
    match slot.kind {
        k if k.is_integer_family() => Ok(NumericDomain::Int),
        NativeKind::Float64 | NativeKind::NullableFloat64 => Ok(NumericDomain::Float),
        NativeKind::Ptr(HeapKind::Decimal) => Ok(NumericDomain::Decimal),
        NativeKind::Ptr(HeapKind::BigInt)  => Ok(NumericDomain::BigInt),
        _ => Err(VMError::TypeError(
            format!("expected numeric, got {:?}", slot.kind))),
    }
}
```

**Hard rules:**

- **No `KindedSlot::as_X()` additions outside Q8 cardinality bound.** Every helper above lives in
  `kind_coerce.rs`, not on the carrier.
- **Cluster A defines `NumericDomain`** as a new public-in-crate enum in `kind_coerce.rs` if not
  already present at dispatch time. Variants are exactly `{Int, Float, Decimal, BigInt}`. Adding
  variants is an architectural decision — surface to supervisor.
- **No `coerce_to_*_or_default`, no `as_any_numeric`, no `as_number_coerce`.** Body sites that need
  cross-domain dispatch use `match numeric_domain(slot)?` explicitly.

---

## 2. Kind-sourcing rules per opcode category

The rule below tells you *where* the kind for each pushed value comes from. Kind is **never**
defaulted to `Bool` ("leak-free because Drop is no-op" is the W-series rationalization explicitly
forbidden by §2.7.7). If the kind isn't sourcable locally, the opcode itself is wrong-shape — surface.

| Opcode category | Result kind sourced from | Worked example |
|---|---|---|
| Typed-arith (`AddInt`, `SubFloat`, `MulInt`, `DivDecimal`, …) | Opcode-name suffix → `NativeKind` | `AddInt` ⇒ `NativeKind::Int64`; `MulFloat` ⇒ `NativeKind::Float64`; `AddDecimal` ⇒ `NativeKind::Ptr(HeapKind::Decimal)` |
| Comparison (`EqInt`, `LtFloat`, `Le…`, `Ne…`) | Always `NativeKind::Bool` | `EqInt` returns 0/1 raw bits with `NativeKind::Bool` |
| Logical (`And`, `Or`, `Not`, `NullCoalesce`) | Always `NativeKind::Bool` (filter-expr branch: `NativeKind::Ptr(HeapKind::NativeView)`) | mirrors `executor/logical/mod.rs` (template — see §6) |
| Local load (`LoadLocal`, `LoadLocalTyped`) | `FrameDescriptor.slots[slot_idx]` | `let kind = self.current_frame_descriptor()?.slots[idx];` |
| Local store (`StoreLocal`, `StoreLocalTyped`) | Top-of-stack kind via `pop_kinded()` | `let (bits, kind) = self.pop_kinded()?;` then `stack_write_kinded(local_base + idx, bits, kind)` |
| Loop iteration value (`IterNext`, `ForEach…`) | Iterator's element FieldType (in scope at loop-header opcode) | `NativeKind` from `iter.element_kind` — capture once when the loop is entered, reuse per element |
| Function call return (`CallFunc`, `CallBuiltin`, `CallTyped`, `Invoke…`) | `FrameDescriptor.return_kind` of the called function (via the `program` registry) | `let return_kind = self.program.frames[func_id].return_kind;` |
| Stack manipulations (`Dup`, `Swap`, `Rot`) | Preserve via `stack_read_kinded_raw` + `clone_with_kind` | mirrors `executor/stack_ops/mod.rs` (template — see §6) |
| Heap-construction (`MakeArray`, `MakeMap`, `MakeObject`, `Box…`) | `NativeKind::Ptr(HeapKind::*)` per construction target | `MakeTypedArray` ⇒ `NativeKind::Ptr(HeapKind::TypedArray)` |
| Constants (`PushConst`) | Per `Constant::*` arm — already migrated in `stack_ops/op_push_const` | mirrors `stack_ops/mod.rs:69-149` |
| Null / Unit | `0u64` bits + `NativeKind::Bool` (the §2.7 sentinel — Drop no-op by construction) | `self.push_kinded(0u64, NativeKind::Bool)` |

**Forbidden source-of-kind shapes:**

- Decoding kind from the high bits of `bits` (`(bits >> 48) & 0xFF` etc.) — this is the deleted
  ValueWord tag-decode pattern.
- Probing the heap object via `bits as *const HeapValue` and reading its discriminant — that's a
  parallel discriminator violation of ADR-005 §1.
- "Use `NativeKind::Bool` as a default and trust the Drop is a no-op." This is the W-series
  rationalization §2.7.7 forbids verbatim.
- `NativeKind::Unknown` / `NativeKind::Pending` — both deleted by the bulldozer; do not
  re-introduce under any name.

---

## 3. Canonical `ValueWord`-construction rewrite

Every deleted shim caller threads a `ValueWord::from_*` construction expression into a `push_raw_u64`
or `push_native_*`. Substep-2 rewrites both halves together. The pattern below is mechanical per
`HeapKind` variant.

```rust
// BEFORE (substep-1 deletion target — does not compile)
self.push_raw_u64(ValueWord::from_decimal(d).into_raw_bits())?;
self.push_raw_u64(ValueWord::from_int(i).into_raw_bits())?;
self.push_raw_u64(ValueWord::from_string_arc(arc.clone()).into_raw_bits())?;
self.push_native_i64(i)?;
self.push_native_bool(b)?;
self.push_raw_f64(n)?;

// AFTER — kinded API. Construction uses Arc::into_raw for heap arms,
// raw bits + matching NativeKind for inline scalars.
let bits = Arc::into_raw(Arc::new(d)) as u64;
self.push_kinded(bits, NativeKind::Ptr(HeapKind::Decimal))?;

self.push_kinded(i as u64, NativeKind::Int64)?;

let bits = Arc::into_raw(arc) as u64;          // takes ownership — caller should NOT clone first
self.push_kinded(bits, NativeKind::String)?;

self.push_kinded(i as u64, NativeKind::Int64)?;
self.push_kinded(b as u64, NativeKind::Bool)?;
self.push_kinded(n.to_bits(), NativeKind::Float64)?;
```

**Per-`HeapKind` push pattern:**

```rust
// String      — Arc<String>
let bits = Arc::into_raw(arc) as u64;
self.push_kinded(bits, NativeKind::String)?;

// TypedArray  — Arc<TypedArrayData>
let bits = Arc::into_raw(arc) as u64;
self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))?;

// TypedObject — Arc<TypedObjectStorage>
let bits = Arc::into_raw(arc) as u64;
self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))?;

// HashMap     — Arc<HashMapData>
// Decimal     — Arc<rust_decimal::Decimal>
// BigInt      — Arc<i64>
// DataTable   — Arc<DataTable>
// IoHandle    — Arc<IoHandleData>
// NativeView  — Arc<NativeViewData>
// Content     — Arc<ContentNode>
// Instant     — Arc<std::time::Instant>
// Temporal    — Arc<TemporalData>
// TableView   — Arc<TableViewData>
// TaskGroup   — Arc<TaskGroupData>
// Char        — codepoint as u64 (NO Arc), kind = Ptr(HeapKind::Char) for dispatch uniformity
```

**Pop pattern:**

```rust
// BEFORE
let bits = self.pop_raw_u64()?;
let vw = ValueWord::from_raw_bits(bits);
let s = vw.as_string_arc().ok_or(VMError::TypeError("...".into()))?;

// AFTER
let (bits, kind) = self.pop_kinded()?;
match kind {
    NativeKind::String => {
        // SAFETY: pop_kinded transfers ownership; reconstruct the Arc share.
        let arc: Arc<String> = unsafe { Arc::from_raw(bits as *const String) };
        // ... use arc ...
    }
    _ => {
        drop_with_kind(bits, kind);  // release the popped share — DO NOT skip
        return Err(VMError::TypeError(format!("expected string, got {:?}", kind)));
    }
}
```

**Borrow pattern (peek-without-consume):**

```rust
// BEFORE
let bits = self.stack_peek_raw(idx);
let vw = ValueWord::from_raw_bits(bits);

// AFTER
let (bits, kind) = self.stack_read_kinded_raw(idx);   // returns (u64, NativeKind), no refcount change
// Use bits + kind directly. If you need a KindedSlot for §2.7.6 dispatch:
let slot = self.read_owned_kinded(idx);                // BUMPS refcount on heap arms; pair with explicit drop_with_kind on the carrier
```

**WB2.4 retain-on-read invariant** — every site that hands an *owning* share to a runtime-tier
consumer (e.g. into a `KindedSlot`, a `Vec<KindedSlot>` arg slice, a frame-snapshot `KindedSlot`
field) must bump the heap refcount. Use `read_owned_kinded` (which bumps) when the consumer takes
ownership; use `stack_read_kinded_raw` (no bump) when the consumer borrows. Mismatched discipline
is the bug class WB2.4 was designed to prevent.

**Drop discipline:** every `pop_kinded`'d slot **must** either be re-pushed (its share lives on the
stack) or have `drop_with_kind(bits, kind)` called on it explicitly. Missing the drop leaks an
`Arc<T>`; double-dropping double-frees. The compiler does not enforce this — the discipline lives at
each call site, the same as the deleted `vw_drop(bits)` it replaces.

---

## 4. §2.7.7 forbidden shapes — restated

Refuse on sight, all 5 clusters:

| # | Forbidden | Why |
|---|---|---|
| 1 | Re-introducing any deleted shim under any name (`push_typed_u64`, `pop_value`, `stack_peek`, `push_kinded_bool` as a Bool-default wrapper, …) | §2.7.7 explicit-forbid; W-series defection-attractor |
| 2 | `Vec<KindedSlot>` for the VM stack | §2.7.5 forbids KindedSlot in VM↔JIT slot ABI |
| 3 | 16-byte stack slots (`{ bits: u64, kind: NativeKind }` packed) | §2.1 fixes slot at 8 bytes; §2.7.7 alternative explicitly rejected |
| 4 | Tag bits packed in the `u64` | Deleted ValueWord pattern |
| 5 | `Vec<Option<NativeKind>>` for the kind track | §2.7.7 / §2.7.5.1 — stack contents are post-proof |
| 6 | `NativeKind::Unknown`, `NativeKind::Pending`, `NativeKind::Dynamic` | All deleted; CLAUDE.md "Renames to refuse on sight" |
| 7 | `is_heap()`, `as_heap_ref()`, `tag_bits::*`, `synthesize_value_word_from_raw` | Forbidden tag-decode hops |
| 8 | `vw_clone(bits)`, `vw_drop(bits)` | Replaced by `clone_with_kind(bits, kind)` / `drop_with_kind(bits, kind)` |
| 9 | "Default to Bool kind because Drop is a no-op" | The W-series rationalization §2.7.7 names verbatim |
| 10 | New per-heap-variant accessors on `KindedSlot` (`as_typed_array`, `as_decimal`, …) | Q8 — heap dispatch goes through `slot.as_heap_value()` + `HeapValue` match |
| 11 | "Mark this as a follow-up for a later phase" / "feature-flag the migration" / "just one decode at the boundary" | §2.7.7 forbidden rationalizations |

**On encountering a forbidden shape during migration:** stop, surface to supervisor with the
shape and the call-site. Do not paper over.

---

## 5. Cluster file lists (refined from handover §6.2)

The handover §6.2 split was undercounted: it omitted `variables/mod.rs` (267 sites — the largest
single file), `debugger_integration.rs`, `osr.rs`, `dispatch.rs`, `resume.rs`. It also estimated
Cluster C at ~400 sites; the actual count for `typed_handlers/* + v2_handlers/* + objects/*` is
~970, which doesn't fit one agent. This refined split keeps the 5-cluster shape but rebalances.

Counts shown as **mandatory + sibling = total** per file, where mandatory = the 5 grep-gated shims
(`push_raw_u64`, `pop_raw_u64`, `push_native_i64`, `stack_read_owned`, `stack_peek_raw`) and
sibling = the other ~25 shims deleted in substep-1.

### Cluster A — Numeric / logical / comparison opcodes (~370 sites, 3 files)

| File | Sites | Pattern |
|---|---|---|
| `crates/shape-vm/src/executor/arithmetic/mod.rs` | 93 + 115 = 208 | Typed-arith opcode-suffix → result kind; `numeric_domain(slot)?` for cross-domain bodies |
| `crates/shape-vm/src/executor/comparison/mod.rs` | 74 + 82 = 156 | Result kind always `NativeKind::Bool`; per-domain comparison via `numeric_domain` |
| `crates/shape-vm/src/executor/logical/mod.rs` | 0 + 6 = 6 | **Wave 6.0 finishing only.** The 6 leftover `push_native_bool(r)?` calls become `push_kinded(r as u64, NativeKind::Bool)?` |

**Cluster A reference template:** `executor/stack_ops/mod.rs` (Wave 6.0 close, fully migrated).

### Cluster B — Control path & locals (~380 sites, 8 files)

| File | Sites | Pattern |
|---|---|---|
| `crates/shape-vm/src/executor/variables/mod.rs` | 221 + 46 = 267 | Local load/store; kind from `FrameDescriptor.slots[idx]` |
| `crates/shape-vm/src/executor/control_flow/mod.rs` | 34 + 12 = 46 | Branch / jump opcodes; condition pop reads `NativeKind::Bool` from kind track |
| `crates/shape-vm/src/executor/loops/mod.rs` | 16 + 10 = 26 | Loop iteration value kind from iterator's element FieldType |
| `crates/shape-vm/src/executor/call_convention.rs` | 7 + 11 = 18 | Call return kind from `FrameDescriptor.return_kind` of called function |
| `crates/shape-vm/src/executor/debugger_integration.rs` | 0 + 10 = 10 | Read-only stack inspection — use `stack_read_kinded_raw` (no refcount churn) |
| `crates/shape-vm/src/executor/osr.rs` | 0 + 7 = 7 | OSR materializes locals; kind from `FrameDescriptor.slots[idx]` (same as variables/mod.rs) |
| `crates/shape-vm/src/executor/dispatch.rs` | 0 + 3 = 3 | Opcode dispatch shell — likely just shim-name renames to `pop_kinded` |
| `crates/shape-vm/src/executor/resume.rs` | 0 + 3 = 3 | Frame restore on suspension — kind from saved `FrameDescriptor` |

**Cluster B reference template:** `executor/stack_ops/mod.rs` (constant-push + Dup pattern), plus
the `current_frame_descriptor()` helper at `vm_impl/stack.rs:430` for kind-from-FrameDescriptor.

### Cluster C — Typed method dispatch (~660 sites, 14 files)

| File | Sites | Pattern |
|---|---|---|
| `executor/typed_handlers/typed_array.rs` | 69 + 92 = 161 | Method dispatch on `TypedArrayData`; reads `NativeKind::Ptr(HeapKind::TypedArray)` from kind track |
| `executor/typed_handlers/typed_map.rs` | 52 + 26 = 78 | `NativeKind::Ptr(HeapKind::HashMap)` |
| `executor/typed_handlers/int.rs` | 0 + 33 = 33 | `NativeKind::Int64` (and family) |
| `executor/typed_handlers/field.rs` | 10 + 7 = 17 | Field load/store on TypedObject |
| `executor/typed_handlers/string.rs` | 7 + 2 = 9 | `NativeKind::String` |
| `executor/typed_handlers/typed_enum.rs` | 3 + 1 = 4 | `NativeKind::Ptr(HeapKind::TypedObject)` (enum payload) |
| `executor/v2_handlers/typed_array.rs` | 133 + 16 = 149 | v2 dispatch — same kind sources as typed_handlers/typed_array |
| `executor/v2_handlers/typed_map.rs` | 53 + 26 = 79 | |
| `executor/v2_handlers/typed_array_elem.rs` | 42 + 14 = 56 | Element access — element kind from `TypedArrayData::element_kind()` |
| `executor/v2_handlers/int.rs` | 6 + 30 = 36 | |
| `executor/v2_handlers/array.rs` | 17 + 21 = 38 | |
| `executor/v2_handlers/field.rs` | 10 + 7 = 17 | |
| `executor/v2_handlers/string.rs` | 7 + 2 = 9 | |
| `executor/v2_handlers/enum_v2.rs` | 3 + 1 = 4 | |

**Cluster C reference template:** the typed-handler dispatch already uses `pop_kinded` at the
entry boundary (Wave 5b body migrations). Pattern: pop, match on `kind`, dispatch to the matching
`HeapValue::*` arm via `as_heap_value()`. **No per-heap-variant accessors** (Q8).

### Cluster D — Heap-side objects & misc (~310 sites, 13 files)

| File | Sites | Pattern |
|---|---|---|
| `executor/objects/property_access.rs` | 57 + 4 = 61 | Property read/write on TypedObject; `slot.as_heap_value()` + `HeapValue::TypedObject` match |
| `executor/objects/array_joins.rs` | 40 + 0 = 40 | Multi-array join; element kinds from `TypedArrayData` |
| `executor/objects/mod.rs` | 39 + 0 = 39 | Generic object dispatch shell |
| `executor/objects/array_operations.rs` | 26 + 12 = 38 | Array push/pop/concat |
| `executor/builtins/type_ops.rs` | 33 + 3 = 36 | Type predicates (`is_int`, `typeof`, `instanceof`) — dispatch on `kind` directly |
| `executor/objects/typed_access.rs` | 26 + 8 = 34 | Typed field access fast path |
| `executor/window_join.rs` | 17 + 4 = 21 | Window/join — typed array element kinds |
| `executor/objects/object_creation.rs` | 19 + 0 = 19 | Object factory; pushes `NativeKind::Ptr(HeapKind::TypedObject)` |
| `executor/typed_object_ops.rs` | 11 + 3 = 14 | `NativeKind::Ptr(HeapKind::TypedObject)` |
| `executor/trait_object_ops.rs` | 10 + 0 = 10 | Trait dispatch — vtable + Arc shape |
| `executor/objects/concat.rs` | 7 + 0 = 7 | String concat |
| `executor/objects/object_operations.rs` | 6 + 0 = 6 | |
| `executor/objects/concurrency_methods.rs` | 3 + 0 = 3 | |

**Cluster D ruling on `executor/objects/raw_helpers.rs`:** the file uses forbidden `tag_bits::*` /
`is_tagged()` / `synthesize_value_word_from_raw`. It has zero callers of the 5 mandatory shims.
Cluster D **must** decide per consumer: either delete the function (and rewrite the caller to a
post-§2.7.7 shape) or migrate the body off the forbidden helpers. The file currently has at least
one live consumer: `logical/mod.rs` calls `raw_helpers::extract_filter_expr` for the And/Or/Not
filter-expr branch (cluster A's territory but a downstream dependency). Coordinate with cluster A
via supervisor if rewriting `extract_filter_expr` requires logical/mod.rs changes outside cluster
A's scope. Default disposition: rewrite `extract_filter_expr` to take `(bits, kind)` directly and
dispatch on `kind == NativeKind::Ptr(HeapKind::NativeView)`; delete the rest.

### Cluster E — Tail (system, snapshot, exceptions, builtins backlog, tests) (~150 sites + Wave 5b backlog)

| File | Sites | Pattern |
|---|---|---|
| `executor/v2_stack_tests.rs` | 38 + 0 = 38 | Test harness — migrate test setups to `push_kinded` directly |
| `executor/tests/table_iteration.rs` | 35 + 0 = 35 | Test harness |
| `executor/exceptions/mod.rs` | 26 + 0 = 26 | Exception payload kind = `NativeKind::Ptr(HeapKind::TypedObject)` (per ADR-006 exception-as-typed-object) |
| `executor/async_ops/mod.rs` | 13 + 0 = 13 | Future/TaskGroup payload kinds |
| `crates/shape-vm/src/compiler/helpers.rs` | 8 + 2 = 10 | Compile-time helpers — likely emit `push_kinded` opcode operands directly |
| `executor/vm_impl/{program,output,builtins,modules,stack}.rs` | (3+1+1+1+3) + (2+2+0+1+1) = 14 | Internal — most likely already-migrated incidental refs in stack.rs; verify and delete remainder |
| `executor/execution.rs` | 3 + 0 = 3 | Top-level execute loop |
| `executor/vm_state_snapshot.rs` | 2 + 1 = 3 | **Defer to Phase 2c per §2.7.4.** Replace shim callers with `todo!("phase-2c snapshot rebuild — see §2.7.4")` if the call site doesn't have a kinded equivalent yet |
| `executor/snapshot.rs` | 0 + 2 = 2 | Same Phase 2c deferral |
| `executor/tests/mod.rs` | 0 + 2 = 2 | |
| `compiler/expressions/identifiers.rs` | 2 + 0 = 2 | |
| `compiler/loops.rs` | 0 + 1 = 1 | |
| `executor/builtins/{math,array_ops,object_ops,intrinsics/*,…}.rs` | Wave 5b deferred body migrations | Per Wave 5b template (`fa2bafc`) — body sig `Fn(&[KindedSlot], &mut ExecutionContext) -> Result<KindedSlot>` |
| `executor/printing.rs` | (formatter rewrite) | `PrintResult` / `PrintSpan` post-§2.7.4 — already imported from `shape_runtime::output_adapter`; finish the formatter call sites |

**Cluster E reference template:** Wave 5b body migrations in `executor/builtins/math.rs` (already
landed). For snapshot/restore, the §2.7.4 Phase-2c deferral pattern: replace deleted-API call sites
with `todo!("phase-2c snapshot rebuild")` rather than papering over with placeholder serializers.

---

## 6. Reference templates (already-migrated; use as cheat-sheet)

These two files were fully migrated in Wave 6.0 and are stable templates. Read them before starting
migration in any cluster — the kind-sourcing patterns + push/pop discipline are demonstrated end-to-end.

| Template | Demonstrates |
|---|---|
| `crates/shape-vm/src/executor/stack_ops/mod.rs` (153 lines) | `op_push_const` per-Constant kind sourcing; Dup retain-on-read with `clone_with_kind`; Pop drop discipline with `drop_with_kind` |
| `crates/shape-vm/src/executor/logical/mod.rs` (192 lines) | Filter-expr branch with `NativeKind::Ptr(HeapKind::NativeView)`; `kinded_truthy(bits, kind)` heterogeneous-kind body; **note: 6 leftover `push_native_bool` calls — cluster A finishes** |

`vm_impl/stack.rs` (the post-substep-1 kept primitives) is the canonical kinded API surface:
`push_kinded`, `pop_kinded`, `read_owned_kinded`, `stack_read_kinded_raw`, `stack_write_kinded`,
`stack_take_kinded`, `truncate_stack`, `stack_peek_kinded`. Plus the `NONE_BITS` sentinel and the
`clone_with_kind` / `drop_with_kind` helpers (lines 51 and 157).

---

## 7. Per-cluster definition of done (REVISED 2026-05-09)

A cluster closes when **all** of the following hold inside its file list:

1. **No mandatory-shim references remain in cluster files**:
   ```
   grep -n 'push_raw_u64\|pop_raw_u64\|push_native_i64\|stack_read_owned\|stack_peek_raw' <cluster files>
   ```
2. **No sibling-shim references remain in cluster files** (full sibling list — `pop_native_i64`,
   `push_native_bool`, `pop_native_bool`, `push_raw_u64_slow`, `push_raw_f64`, `pop_raw_f64`,
   `push_tagged_*`, `pop_tagged_*`, `stack_top_both_*`, `stack_top_is_*`, `stack_read_raw`,
   `stack_write_raw`, `stack_take_raw`, `stack_slice_raw`, `peek_args_slice`,
   `bindings_slice_raw`, `binding_read_*`, `binding_write_raw`, `binding_take_raw`).
3. **No new forbidden-pattern introductions** (the §2.7.7 / §2.7.8 §4 list of this playbook).
   Forbidden patterns the cluster file already had (pre-existing `ValueWord`, `tag_bits::*`,
   `as_heap_ref`, `synthesize_value_word_from_raw`, `vw_clone`, `vw_drop`, `NativeKind::Unknown`,
   `vmarray_from_vec`, `value_word_drop`, etc.) are **migrated off** during the cluster work —
   never preserved, never copied to a new file, never reintroduced in a different shape. If a
   call site cannot be migrated cleanly, the correct shape is `NotImplemented(SURFACE)` /
   `todo!("phase-2c — see ADR-006 §2.7.4")` / surface-and-stop, never a forbidden-pattern
   workaround.
4. **Cluster files compile cleanly OR the un-compiling sites have a documented surface**: if a
   call site cannot be migrated this round, leave a `NotImplemented(SURFACE: <why>)` /
   `todo!("phase-2c <X>")` placeholder per §2.7.4. The placeholder is tracked, not silently
   left as a compile error.
5. **`bash scripts/check-no-dynamic.sh` exits 0** (defection guard — frozen baseline).
6. **AGENTS.md updated**: cluster's row flipped to `idle` with close commit hash.
7. **Cluster commit message** cites this playbook + the relevant ADR-006 §2.7.x section + the
   per-cluster pattern.

**`cargo check -p shape-vm --lib` error count is INFORMATIONAL, NOT GATING.** Capture before/after
counts in the close report so the supervisor sees the cumulative arc, but the count CAN go up when
a cluster correctly migrates off forbidden patterns and exposes downstream cascade in non-territory
files. **Do not introduce forbidden patterns to keep the count down.** That is the W-series
defection-attractor verbatim.

Examples of legitimate count rises:
- Migrating `as_heap_ref` consumers to `as_heap_value` exposes pre-existing `as_heap_ref` callers
  in OOR files that were previously hidden by upstream errors.
- Replacing a `ValueWord::from_*` constructor with `Arc::into_raw + push_kinded` exposes
  type-annotation gaps in callers that the old shape papered over.
- Removing forbidden imports from a header makes the rest of the file's body errors visible.

In all of these cases, the cluster did the right thing. The count delta is a measurement of the
cleanup surface, not a quality signal.

**Do NOT run `cargo check` yourself unless investigating a specific compile error.** With many
parallel clusters, running `cargo check -p shape-vm --lib` 15× in parallel thrashes the build
cache. The supervisor runs check at merge time. Per-cluster verification: grep gates (above) +
spot-read your modified files for forbidden-pattern reintroduction.

---

## 8. Surface-and-stop triggers

Stop, stash WIP, flip AGENTS.md row to `blocked`, and surface to supervisor on:

- A push site where the kind cannot be sourced from the rules in §2 (typed-arith suffix,
  FrameDescriptor, opcode operand). The opcode itself may be wrong-shape.
- A `Constant::*` arm not handled by `op_push_const` in `stack_ops/mod.rs:69-149` — the constant
  table needs alignment with the kinded heap encoding.
- Cross-cluster migration cascade: a fix in your cluster requires changing a file owned by another
  cluster. Surface so supervisor coordinates — do **not** edit out of territory.
- shape-jit FFI consumer trips: a typed_handler/v2_handler change in cluster C cascades into
  `shape-jit` FFI symbols. That's Wave 10 territory creep — surface, don't follow.
- `NumericDomain` enum needs a 5th variant beyond `{Int, Float, Decimal, BigInt}` — surface for
  ADR-006 amendment.
- `raw_helpers.rs` cannot be cleanly deleted/migrated without a logical/mod.rs rewrite — surface;
  cluster D + cluster A coordinate via supervisor.
- `vm_state_snapshot.rs` / `snapshot.rs` call site has a kinded equivalent that doesn't fit the
  Phase-2c deferral pattern — surface (likely §2.7.4 amendment).

**Do not** surface for: drop-discipline ambiguity at a single call site (read §3, the rule is
"either re-push or `drop_with_kind`"); helper signature questions answered in §1; pattern-recognition
at heap-kind dispatch (use `slot.as_heap_value()` + `HeapValue::*` match per Q8).

---

## 9. Pointers

- ADR-006: `docs/adr/006-value-and-memory-model.md` (§2.7 - §2.7.7 + Q1-Q9)
- Forbidden patterns: `CLAUDE.md` "Forbidden Patterns" + "Renames to refuse on sight"
- Wave 6.5 handover: `docs/cluster-audits/phase-1b-vm-wave-6-5-handover.md`
- Cluster audit (territory binding): `docs/cluster-audits/phase-1b-vm-valueword-callers.md`
- Defection log (append-only): `docs/defections.md`
- Substep-1 close commit: `11efd9c` (deleted shim list in commit body)
- Wave 6.0 anchor commit: `d782401` (kept primitives + clone/drop helpers)

---

*Playbook closed for edits during cluster fan-out. Amendments require supervisor sign-off and a
fresh dispatch round.*

---

## 10. Wave-α fine-grained sub-cluster split (2026-05-09 — for massive parallelism)

**Why:** clusters A/B/C ran one agent each over 167-660 sites. Cluster B partial-closed correctly
on architectural surfaces; clusters D/E first-round were aborted after the strictly-decreased gate
(removed in §7 above) pushed an agent toward forbidden-pattern workarounds. Wave-α splits the
remaining D/E territory + cluster B-round-2 into per-file or per-handler-class units so 15-20
agents can run in parallel and the surface-and-stop discipline scales — each agent has small
enough scope to fit in one context window and one focused topic.

**Branch convention:** `bulldozer-strictly-typed-phase-1b-vm-<name>` per sub-cluster, all branched
from phase-1b-vm HEAD `ad0a954` (post-§2.7.8 amendment). Worktree at
`/home/dev/dev/shape-lang/shape-phase-1b-vm-<name>`.

### Wave-α — independent sub-clusters (dispatch all in parallel)

Each sub-cluster owns the listed file(s); no overlap between sub-clusters; ordering doesn't matter.

| Sub-cluster | Files | Sites | Pattern |
|---|---|---|---|
| `D-prop-access` | `executor/objects/property_access.rs` | 61 | TypedObject property dispatch via `slot.as_heap_value()` + `HeapValue::TypedObject` match |
| `D-array-joins` | `executor/objects/array_joins.rs` | 40 | Multi-array element-kind dispatch from `TypedArrayData` |
| `D-objects-mod` | `executor/objects/mod.rs` | 39 | Generic object dispatch shell |
| `D-array-ops` | `executor/objects/array_operations.rs` | 38 | Array push/pop/concat |
| `D-type-ops` | `executor/builtins/type_ops.rs` | 36 | Type predicates (`is_int`, `typeof`, `instanceof`) — dispatch on `kind` directly |
| `D-typed-access` | `executor/objects/typed_access.rs` | 34 | Typed field access fast path |
| `D-window-join` | `executor/window_join.rs` | 21 | Window/join element kinds |
| `D-obj-create` | `executor/objects/object_creation.rs` | 19 | TypedObject factory |
| `D-typed-obj-ops` | `executor/typed_object_ops.rs` | 14 | TypedObject ops |
| `D-trait-obj` | `executor/trait_object_ops.rs` | 10 | Trait dispatch — vtable + Arc shape |
| `D-obj-tail` | `executor/objects/{concat,object_operations,concurrency_methods}.rs` | 16 combined | Tail object files; concat/object_ops/concurrency |
| `D-raw-helpers` | `executor/objects/raw_helpers.rs` | 0 mandatory + uses forbidden tag_bits | **P1 — unblocks B-round-2.** Default disposition: rewrite `extract_filter_expr` to take `(bits, kind)` directly + delete forbidden helpers. Verify `logical/mod.rs` filter-expr branch still compiles after. |
| `D-array-detect` | `executor/typed_handlers/array_detect.rs` (~829 lines) | forbidden-helper carrier (no mandatory shim) | Audit liveness per opcode; migrate live ones to kinded; stub dead ones to `NotImplemented(SURFACE: phase-2c reentry)` |
| `D-v2-array-detect` | `executor/v2_handlers/v2_array_detect.rs` (~1118 lines) | forbidden-helper carrier | Same approach as `D-array-detect` |
| `E-snapshot` | `executor/snapshot.rs` + `executor/vm_state_snapshot.rs` | 5 | **§2.7.4 Phase-2c deferral pattern** — `todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")` |
| `E-exceptions` | `executor/exceptions/mod.rs` | 26 | Exception payload kind = `NativeKind::Ptr(HeapKind::TypedObject)` |
| `E-async` | `executor/async_ops/mod.rs` | 13 | Future/TaskGroup payload kinds |
| `E-tests` | `executor/v2_stack_tests.rs` + `executor/tests/{table_iteration,mod}.rs` | 75 | Test harness — migrate `push_kinded`/`pop_kinded` directly |
| `E-vm-impl-tail` | `executor/vm_impl/{program,output,builtins,modules,stack}.rs` | 14 | Internal — verify and delete remainder |
| `E-execution` | `executor/execution.rs` | 3 | Top-level execute loop |
| `E-compiler` | `crates/shape-vm/src/compiler/{helpers.rs, expressions/identifiers.rs, loops.rs}` | 13 | Compile-time helpers |
| `E-builtins-backlog` | `executor/builtins/{array_ops,object_ops,intrinsics/*,…}.rs` (Wave 5b deferred) | TBD | Body migration to `&[KindedSlot]` per Wave 5b template (`fa2bafc`) |
| `E-printing` | `executor/printing.rs` | (formatter rewrite) | `PrintResult` / `PrintSpan` post-§2.7.4 — finish formatter call sites |
| `B7-closure-cells` | `executor/closure_raw.rs` (or current closure-layout struct) — STRUCTURAL | §2.7.8 / Q10 | Extend `ClosureCell` with `kinds: Vec<NativeKind>`; constructors + push/pop signatures accept/return `(bits, kind)` |
| `B8-shared-cell` | shared-cell payload struct (locate via grep) — STRUCTURAL | §2.7.8 / Q10 | Extend `SharedCell` with `kind: NativeKind` (single-slot, set at construction) |
| `B9-callframe-kind` | `executor/mod.rs:188` `CallFrame.closure_heap_bits` — STRUCTURAL | §2.7.8 / Q10 | Add companion `closure_heap_kind: Option<NativeKind>`; teardown calls `drop_with_kind` |

### Wave-β — depends on Wave-α (dispatch after merges)

| Sub-cluster | Files | Depends on | Pattern |
|---|---|---|---|
| `B6-variables-loadptr` | `executor/variables/mod.rs` (Load*Ptr/Store*Ptr handlers — the 130 mandatory + 33 sibling sites cluster B partial-closed leaving as `NotImplemented(SURFACE)`) | B7 + B8 + B9 (cell extension landed) | Replace `NotImplemented(SURFACE)` returns with kind-threaded reads via the §2.7.8 cell-storage parallel-kind tracks |
| `B10-loops-heap` | `executor/loops/mod.rs` (`as_heap_ref` heap-side dispatch in `op_iter_next`/`op_iter_done` — the 16 mandatory + 10 sibling sites cluster B couldn't migrate) | D-raw-helpers (raw_helpers rewritten/deleted) | Replace `as_heap_ref` with `slot.as_heap_value()` + `HeapValue::*` match; element kind from iterator FieldType |
| `B11-control-flow-heap` | `executor/control_flow/mod.rs` (the 18 remaining mandatory sites in arg-slicing / op_call_value / op_call_closure paths) | D-raw-helpers | Same replacement; return-kind from `FrameDescriptor.return_kind` per playbook §2 |
| `B12-polymorphic-defer` | the polymorphic / legacy paths cluster B surface-3 listed (op_load_local polymorphic, op_make_ref, etc.) | none structural — pure §2.7.4 deferral | Phase-2c todo!() pattern at every cited call site |

### Dispatch protocol for Wave-α

Each agent prompt must:
- Cite this playbook §10 + the agent's specific row
- Cite ADR-006 §2.7.6, §2.7.7, **§2.7.8 (binding for B7-B9 + cell-bound consumers)**, Q7-Q10
- **Forbid running `cargo check`** except for spot-checking a specific compile error (per §7
  REVISED) — the supervisor runs check at merge time. Saves the build cache from 15× thrash.
- Forbid editing files outside the sub-cluster's listed territory (cross-cluster cascade →
  surface-and-stop).
- Mandate `NotImplemented(SURFACE: <reason>)` or `todo!("phase-2c — <reason>")` over forbidden-
  pattern workarounds. The agent's job is to migrate or surface, never to keep error count down
  by reintroducing forbidden patterns.

**AGENTS.md** is hand-written by supervisor for Wave-α — agents do NOT update AGENTS.md
themselves to avoid 15-way merge conflict. Each agent's commit message documents its territory,
sites migrated, surfaces, and close hash; supervisor consolidates into AGENTS.md at merge time.
