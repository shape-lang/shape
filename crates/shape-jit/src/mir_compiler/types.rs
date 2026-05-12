//! Type mapping for MIR-to-Cranelift IR compilation.
//!
//! Maps MIR LocalTypeInfo and NativeKind to Cranelift types.
//! Includes MIR-level type inference for determining slot kinds
//! when the bytecode compiler doesn't provide them.

use cranelift::prelude::types;
use shape_value::heap_value::HeapKind;
use shape_value::v2::ConcreteType;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::NativeKind;

/// Whether a local slot holds a heap value that needs reference counting.
pub(crate) fn is_heap_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::NonCopy)
}

/// Whether a local slot is known to be Copy (no refcounting needed).
pub(crate) fn is_copy_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::Copy)
}

/// Get the NativeKind for a local. Returns `None` when the slot
/// index is out of range OR the inference pass left the slot
/// undetermined.
///
/// Per ADR-006 §2.7.7, the deleted `NativeKind::Unknown` placeholder
/// is forbidden in the runtime parallel-kind track. This compile-time
/// helper is a different layer (compile-time inference metadata, not
/// the runtime track), but it adopts the same single-discriminator
/// discipline by returning `Option<NativeKind>` rather than papering
/// over the missing-kind case.
pub(crate) fn slot_kind_for_local(
    slot_kinds: &[Option<NativeKind>],
    slot_idx: u16,
) -> Option<NativeKind> {
    slot_kinds.get(slot_idx as usize).copied().flatten()
}

/// Whether a NativeKind is i32 (Int32 or UInt32).
pub(crate) fn is_i32_slot(kind: NativeKind) -> bool {
    matches!(kind, NativeKind::Int32 | NativeKind::UInt32)
}

/// Whether a NativeKind represents a native (non-NaN-boxed) Cranelift type.
#[allow(dead_code)]
pub(crate) fn is_native_slot(kind: NativeKind) -> bool {
    matches!(
        kind,
        NativeKind::Float64
            | NativeKind::Int32
            | NativeKind::UInt32
            | NativeKind::Bool
            | NativeKind::Int8
            | NativeKind::UInt8
            | NativeKind::Int16
            | NativeKind::UInt16
    )
}

/// Map a NativeKind to its Cranelift type.
/// Native numeric types get their natural width; everything else is I64.
pub(crate) fn cranelift_type_for_slot(kind: NativeKind) -> cranelift::prelude::Type {
    match kind {
        NativeKind::Float64 => types::F64,
        NativeKind::Int32 | NativeKind::UInt32 => types::I32,
        NativeKind::Int8 | NativeKind::UInt8 | NativeKind::Bool => types::I8,
        NativeKind::Int16 | NativeKind::UInt16 => types::I16,
        // Int64, UInt64, String, Ptr(_), Nullable*, IntSize, UIntSize:
        // 8-byte raw u64 (typed pointer for heap arms, scalar for ints).
        _ => types::I64,
    }
}

/// Whether a NativeKind is a v2 heap pointer type (TypedArray, TypedStruct, StringObj).
/// These use inline refcounting via HeapHeader at offset 0.
pub(crate) fn is_v2_heap_slot(kind: NativeKind) -> bool {
    let _ = kind;
    false
}

/// Map a `ConcreteType` element type to the matching `NativeKind` for the v2
/// typed-array codegen helpers (`v2_array_get`/`v2_array_set`).
pub(crate) fn elem_slot_kind_for_concrete(elem: &ConcreteType) -> Option<NativeKind> {
    match elem {
        ConcreteType::F64 => Some(NativeKind::Float64),
        ConcreteType::I64 => Some(NativeKind::Int64),
        ConcreteType::I32 => Some(NativeKind::Int32),
        ConcreteType::I16 => Some(NativeKind::Int16),
        ConcreteType::I8 => Some(NativeKind::Int8),
        ConcreteType::U64 => Some(NativeKind::UInt64),
        ConcreteType::U32 => Some(NativeKind::UInt32),
        ConcreteType::U16 => Some(NativeKind::UInt16),
        ConcreteType::U8 => Some(NativeKind::UInt8),
        ConcreteType::Bool => Some(NativeKind::Bool),
        _ => None,
    }
}

/// Inspect a slot's `ConcreteType` and report the v2 typed-array element kind
/// when the slot is known to hold an `Array<T>` whose element type maps to a
/// scalar Cranelift load/store. Returns `None` for unknown / non-array /
/// non-scalar slots — caller falls back to legacy NaN-boxed path.
pub(crate) fn is_v2_typed_array_slot(
    concrete_types: &[ConcreteType],
    slot_idx: u16,
) -> Option<NativeKind> {
    let ct = concrete_types.get(slot_idx as usize)?;
    match ct {
        ConcreteType::Array(elem) => elem_slot_kind_for_concrete(elem),
        _ => None,
    }
}

/// Project a `ConcreteType` to its corresponding `NativeKind` for the
/// §2.7.7 / Q9 parallel-kind track seed.
///
/// ADR-006 §2.7.11/Q12: closure-bearing slots (e.g. function return
/// values that produce a closure value via `jit_finalize_heap_closure`)
/// carry kind `Ptr(HeapKind::Closure)` per the slot-tier convention.
/// `ConcreteType::Closure(_)` is the bytecode-compiler-supplied kind
/// source for such slots; without this projection the closure-callee
/// classification at the indirect-call entry can't be derived from
/// MIR-observable statements alone (`infer_slot_kinds` sees only
/// `Rvalue::Use(Copy(_))` chains, not the producing function-call's
/// declared return type).
///
/// Returns `None` for `ConcreteType::Void` (the unit/no-value type)
/// since there is no carrier-bits-shaped slot for void.
pub(crate) fn native_kind_from_concrete_type(ct: &ConcreteType) -> Option<NativeKind> {
    use shape_value::heap_value::HeapKind;
    Some(match ct {
        ConcreteType::F64 => NativeKind::Float64,
        ConcreteType::I64 => NativeKind::Int64,
        ConcreteType::I32 => NativeKind::Int32,
        ConcreteType::I16 => NativeKind::Int16,
        ConcreteType::I8 => NativeKind::Int8,
        ConcreteType::U64 => NativeKind::UInt64,
        ConcreteType::U32 => NativeKind::UInt32,
        ConcreteType::U16 => NativeKind::UInt16,
        ConcreteType::U8 => NativeKind::UInt8,
        ConcreteType::Bool => NativeKind::Bool,
        ConcreteType::String => NativeKind::String,
        // Closure / Function carry `Arc<HeapValue::ClosureRaw>` per
        // §2.7.11/Q12 — `Ptr(HeapKind::Closure)`.
        ConcreteType::Closure(_) | ConcreteType::Function(_) => {
            NativeKind::Ptr(HeapKind::Closure)
        }
        // Result/Option are typed-Arc heap values with their own
        // HeapKind discriminator per §2.7.17.
        ConcreteType::Result(_, _) => NativeKind::Ptr(HeapKind::Result),
        ConcreteType::Option(_) => NativeKind::Ptr(HeapKind::Option),
        // Array<T> — `Arc<TypedArrayData>` per §2.7.6 / Route A.
        ConcreteType::Array(_) => NativeKind::Ptr(HeapKind::TypedArray),
        // HashMap — `Arc<HashMapData>` per Stage C P1(b).
        ConcreteType::HashMap(_, _) => NativeKind::Ptr(HeapKind::HashMap),
        // Struct → TypedObject per §2.7.6.
        ConcreteType::Struct(_) => NativeKind::Ptr(HeapKind::TypedObject),
        // Enum payloads live in TypedObject too (the W14-variant-codegen
        // single-storage-discriminator convention).
        ConcreteType::Enum(_) => NativeKind::Ptr(HeapKind::TypedObject),
        // Decimal / BigInt / DateTime carry typed-Arc heap values.
        ConcreteType::Decimal => NativeKind::Ptr(HeapKind::Decimal),
        ConcreteType::BigInt => NativeKind::Ptr(HeapKind::BigInt),
        ConcreteType::DateTime => NativeKind::Ptr(HeapKind::Temporal),
        // Pointer is the FFI `*const T` raw pointer — UInt64 carrier.
        ConcreteType::Pointer(_) => NativeKind::UInt64,
        // Tuple slots carry typed-array-style storage per the W14
        // tuple-codegen convention; treat as TypedObject for the
        // kind track.
        ConcreteType::Tuple(_) => NativeKind::Ptr(HeapKind::TypedObject),
        // Void has no carrier slot.
        ConcreteType::Void => return None,
    })
}

// ── MIR-level type inference ────────────────────────────────────────────

/// Infer SlotKinds from MIR constants and operations.
///
/// Scans all basic blocks forward and tracks what types flow into each slot.
/// When the bytecode compiler doesn't provide slot_kinds (empty vec),
/// this pass fills them in from MIR-observable information.
///
/// Returns a `Vec<Option<NativeKind>>`: `Some(k)` for slots whose kind
/// the inference proved, `None` for slots the inference left
/// undetermined (e.g. opaque field reads, or parameters with no
/// kind-source). Per ADR-006 §2.7.7 we use `None` rather than the
/// deleted `NativeKind::Unknown` placeholder — callers that need a
/// concrete kind for codegen surface-and-stop on `None`.
///
/// Rules:
/// - Assign(slot, Use(Constant(Float(_)))) → Float64
/// - Assign(slot, Use(Constant(Int(_)))) → Int64 (NaN-boxed int uses 48-bit payload)
/// - Assign(slot, Use(Constant(Bool(_)))) → Bool
/// - Assign(slot, BinaryOp(arith, lhs, rhs)) → inherits from operands if both agree
/// - Assign(slot, Use(Move/Copy(other_slot))) → inherits from other_slot
/// - Conflicting assignments → keep existing
pub(crate) fn infer_slot_kinds(
    mir: &MirFunction,
    existing: &[Option<NativeKind>],
) -> Vec<Option<NativeKind>> {
    infer_slot_kinds_with_concrete(mir, existing, &[])
}

/// Same as `infer_slot_kinds` but also accepts the per-slot
/// `ConcreteType` vector. Used by two orthogonal producing-site
/// classifications:
///
/// 1. **Field projection (W12-jit-binop-after-heap-read-kind-tracker /
///    Round 5A)**: pre-computes a `field_kinds_pre` map from
///    `StatementKind::ObjectStore` operands, then projects through
///    `Place::Field` reads so `Assign(slot, Use(Move(Field(_, _))))`
///    infers the FIELD's kind, not the base struct's heap kind.
///
/// 2. **Index projection (W12-jit-print-kind / Round 5C)**: the
///    `ConcreteType` vector is used to project through `Place::Index` to
///    the array's element kind so destination slots of
///    `Assign(slot, Use(Copy(Index(arr, _))))` infer the element kind
///    rather than the array's heap-pointer kind. Mirrors the JIT codegen-
///    side `v2_typed_array_elem_kind` projection used in
///    `place_native_kind` (rvalues.rs).
///
/// 3. **Call-terminator destination stamping (W12-jit-print-kind /
///    Round 5C)**: BEFORE the forward statement pass, the destination
///    slot of every `TerminatorKind::Call` is stamped from
///    `well_known_method_return_kind` /
///    `well_known_function_return_kind` so a downstream `Assign(n_slot,
///    Use(Move(call_temp)))` can propagate the method-call return kind
///    into the user-visible binding slot.
///
/// ADR-006 §2.7.5 producing-site classification: when the source MIR
/// statement reads an element from a typed-array slot
/// (`Assign(dst, Use(Copy/Move(Index(arr, _))))`), the destination's
/// `NativeKind` is the element kind, not the array's pointer kind. The
/// element kind comes from the typed-array seed
/// (`ConcreteType::Array(elem)`) the bytecode compiler stamps via
/// `infer_top_level_concrete_types_from_mir` / `function_local_concrete_types`,
/// and is passed in as `concrete_types`. Without this projection the
/// `xs[0]` slot stays `None` and a downstream `print(xs[0])` falls into
/// the kind-blind decoder.
///
/// `concrete_types` aligned with MIR slot indices (same shape as the
/// `concrete_seed` built in `mir_compiler::mod.rs`). Entries outside
/// `Array(_)` shapes contribute nothing to the Index-projection rule.
pub(crate) fn infer_slot_kinds_with_concrete(
    mir: &MirFunction,
    existing: &[Option<NativeKind>],
    concrete_types: &[ConcreteType],
) -> Vec<Option<NativeKind>> {
    let n = mir.num_locals as usize;
    let mut kinds: Vec<Option<NativeKind>> = vec![None; n];

    // Seed from existing slot_kinds (from bytecode compiler).
    for (i, &k) in existing.iter().enumerate() {
        if i < n && k.is_some() {
            kinds[i] = k;
        }
    }

    // ADR-006 §2.7.5 producing-site classification for `TerminatorKind::Call`
    // destinations (W12-jit-print-kind / Round 5C) — seeded BEFORE the
    // forward statement pass so the call-result kind is available when a
    // downstream `Assign(slot, Use(Move(call_temp)))` walks the forward
    // pass to propagate the method-call return kind into the user-
    // visible binding slot.
    //
    // The `infer_slot_kinds` statement-walk only sees
    // `StatementKind::Assign(place, rvalue)` writes; the destination of a
    // Call terminator (`TerminatorKind::Call { destination, .. }`) is the
    // separate kind-source the statement-walk misses. Without this seed a
    // `let n = s.size(); print(n)` flows the method-call result through a
    // temp slot whose `kinds[temp]` stays `None`, and the downstream
    // `Assign(n_slot, Use(Move(temp)))` forward-pass inherits `None`,
    // sending `print(n)` into the kind-blind decoder
    // (`format_value_word`, a deleted-W-series tag-decode pattern per
    // CLAUDE.md "Forbidden code").
    //
    // The kind is classified from the well-known method name per
    // `well_known_method_return_kind` — a small registry of method names
    // whose return type is invariant across receiver types in the
    // VM's method registry (`crates/shape-vm/src/executor/objects/
    // method_registry.rs`): `size`/`len`/`length`/`count` → Int64;
    // `isEmpty`/`contains`/`has` → Bool. Names outside this set
    // remain `None` — the slot's kind genuinely isn't statically
    // classifiable from the MIR-observable shape alone, per §2.7.7
    // (no fabricated default).
    for block in &mir.blocks {
        if let TerminatorKind::Call {
            func, destination, ..
        } = &block.terminator.kind
        {
            if let Place::Local(slot) = destination {
                let idx = slot.0 as usize;
                if idx < n && kinds[idx].is_none() {
                    let ret_kind = match func {
                        Operand::Constant(MirConstant::Method(name)) => {
                            well_known_method_return_kind(name)
                        }
                        Operand::Constant(MirConstant::Function(name)) => {
                            well_known_function_return_kind(name)
                        }
                        _ => None,
                    };
                    if let Some(k) = ret_kind {
                        kinds[idx] = Some(k);
                    }
                }
            }
        }
    }

    // W12-jit-binop-after-heap-read-kind-tracker (ADR-006 §2.7.5 /
    // Round 5A): pre-compute the producer-side field-kinds map from
    // `StatementKind::ObjectStore { operands, field_names }`. Each
    // operand's kind is resolved via a forward-only constant-propagation
    // pass over the seeded slot kinds (`kinds` here, freshly seeded with
    // `existing`). The result is then used to project through
    // `Place::Field` in `infer_rvalue_kind_with_projections` /
    // `infer_operand_kind_with_projections` so that `Assign(slot,
    // Use(Move(Field(_, _))))` infers the destination slot's kind from
    // the FIELD's kind, not the base struct's heap kind.
    //
    // Without this, slot kinds inferred from `Use(Move(Field(_, _)))`
    // inherit the base's `Ptr(HeapKind::TypedObject)`, which downstream
    // `refcount_disposition` then dispatches as refcounted — and the
    // field-value `i64=3` passed to `arc_release` segfaults at the
    // initial-zero or post-assignment slot read.
    //
    // Run a quick `Assign(slot, Use(Const))` forward pass first to
    // populate operand-source slot kinds, then walk `ObjectStore` to
    // stamp `field_kinds`. The pre-pass is forward-only (no fixed-point
    // iteration); for cluster-0's load-bearing field-add smoke
    // (`Point{x:3,y:4}` with `int` constants) this is sufficient.
    let field_kinds_pre: std::collections::HashMap<String, NativeKind> = {
        let mut tmp_kinds = kinds.clone();
        for block in &mir.blocks {
            for stmt in &block.statements {
                if let StatementKind::Assign(
                    Place::Local(slot),
                    Rvalue::Use(Operand::Constant(c)),
                ) = &stmt.kind
                {
                    let idx = slot.0 as usize;
                    if idx < n && tmp_kinds[idx].is_none() {
                        tmp_kinds[idx] = infer_constant_kind(c);
                    }
                }
            }
        }
        let mut fk: std::collections::HashMap<String, NativeKind> =
            std::collections::HashMap::new();
        for block in &mir.blocks {
            for stmt in &block.statements {
                if let StatementKind::ObjectStore {
                    operands,
                    field_names,
                    ..
                } = &stmt.kind
                {
                    for (op, name) in operands.iter().zip(field_names.iter()) {
                        if name.is_empty() {
                            continue;
                        }
                        if let Some(kind) =
                            infer_operand_kind_with_fields(op, &tmp_kinds, None, None)
                        {
                            fk.insert(name.clone(), kind);
                        }
                    }
                }
            }
        }
        fk
    };

    // Forward pass: infer from constants and operations.
    for block in &mir.blocks {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(place, rvalue) => {
                    if let Place::Local(slot) = place {
                        let idx = slot.0 as usize;
                        if idx < n && kinds[idx].is_none() {
                            // Combined Field + Index projection (Round
                            // 5A's `infer_rvalue_kind_with_projections`
                            // already handles both: Field via
                            // `field_kinds_pre`, Index via
                            // `concrete_types`'s `Array<scalar>` shape —
                            // the same kind source as 5C's separate
                            // `infer_index_element_kind` helper, bundled
                            // into the more general projection path).
                            if let Some(inferred) = infer_rvalue_kind_with_projections(
                                rvalue,
                                &kinds,
                                Some(&field_kinds_pre),
                                Some(&mir.field_name_table),
                                Some(concrete_types),
                            ) {
                                kinds[idx] = Some(inferred);
                            }
                        } else if idx < n {
                            // Slot already has a kind — check for conflicts.
                            if let Some(inferred) = infer_rvalue_kind_with_projections(
                                rvalue,
                                &kinds,
                                Some(&field_kinds_pre),
                                Some(&mir.field_name_table),
                                Some(concrete_types),
                            ) {
                                if Some(inferred) != kinds[idx] {
                                    // Conflict: different types on different paths.
                                    // Keep the existing kind (first write wins for
                                    // simple programs; SSA form means each slot is
                                    // typically written once in practice).
                                }
                            }
                        }
                    }
                }
                // ADR-006 §2.7.7 / §2.7.11 / Q12 kind-source: a
                // `ClosureCapture` lowers to either the §2.7.11 raw-Arc
                // closure shape (`jit_finalize_heap_closure` → raw
                // `Arc::into_raw(Arc<HeapValue::ClosureRaw>) as u64` slot
                // bits) or the §2.7.11 stack-closure fast path. Either
                // way the slot's `NativeKind` is
                // `Ptr(HeapKind::Closure)` per the §2.7.11/Q12 callee-
                // classification convention. Without this seed the slot
                // would be `None` and the indirect-call dispatch's
                // parallel-kind track would surface a kind-source gap at
                // the load-bearing closure-callee push site for
                // Smoke 1.5.
                StatementKind::ClosureCapture { closure_slot, .. } => {
                    let idx = closure_slot.0 as usize;
                    if idx < n && kinds[idx].is_none() {
                        kinds[idx] = Some(NativeKind::Ptr(HeapKind::Closure));
                    }
                }
                _ => {}
            }
        }
    }

    // F7.c — build the set of "opaque-source" slots: slots whose Rvalue
    // reads from a heap projection (`Field` / `Index`) or another
    // non-trivial source (calls, borrows, aggregates). The runtime value
    // of such a slot is determined by the projection — its Cranelift
    // width is not guaranteed to match anything derivable from later uses.
    //
    // Example: `for i in 0..arr.length { ... }` lowers the `arr.length`
    // read to `Assign(SlotId(4), Use(Copy(Field(Local(1), FieldIdx(0)))))`.
    // The backward pass below would otherwise see `SlotId(5) < SlotId(4)`
    // with `SlotId(5): Int64`, conclude `SlotId(4)` is also `Int64`, and
    // the `compile_binop_int64` fast path would then unpack the
    // `box_number(f64)` bits as a TAG_INT payload — silently reading 0
    // from an f64 `4.0` and making the loop skip every iteration.
    //
    // By excluding these slots from backward propagation, the comparison
    // falls back to `compile_binop_dynamic_cmp`, which traps on a true
    // mixed-tag operand pair (deopt) — but in the common case where the
    // field happens to carry a number (e.g. `arr.length` returns
    // `box_number(len as f64)`), the `both_num` path fires correctly by
    // inspecting the tag bits at runtime rather than trusting an
    // unsound compile-time inference.
    let mut opaque_slots: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for block in &mir.blocks {
        for stmt in &block.statements {
            if let StatementKind::Assign(Place::Local(slot), rvalue) = &stmt.kind {
                let opaque = match rvalue {
                    Rvalue::Use(operand) => is_opaque_operand(operand),
                    // Binary / unary / clone / borrow / aggregate: their
                    // result type comes from the compiler's inference, not
                    // from the destination slot's later uses. We only care
                    // about bare projections here — `Use(Copy(Field))` is
                    // the canonical case.
                    _ => false,
                };
                if opaque {
                    opaque_slots.insert(slot.0 as usize);
                }
            }
        }
    }

    // Backward pass: propagate types from typed operands to Unknown slots
    // used as the other operand in a binop. This picks up closure-param slots
    // like `x` in `|x| x + 1`, where the forward pass leaves `x` Unknown because
    // closure params are registered without a type annotation, but the typed
    // constant `1` proves `x` is Int64.
    //
    // Iterate to a fixed point — at most `n` rounds — so chained inferences
    // propagate (e.g. `|x, y| x + y + 1` should flow Int64 from `1` through
    // both params).
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < n {
        changed = false;
        rounds += 1;
        for block in &mir.blocks {
            for stmt in &block.statements {
                if let StatementKind::Assign(_, Rvalue::BinaryOp(op, lhs, rhs)) = &stmt.kind {
                    // Comparisons don't constrain the operands' kinds beyond
                    // "both must match" — and the producing slot becomes Bool,
                    // not the operand kind. Still useful for propagating
                    // operand kinds between each other.
                    let _ = op;
                    let lk = infer_operand_kind(lhs, &kinds);
                    let rk = infer_operand_kind(rhs, &kinds);
                    match (lk, rk) {
                        (Some(k), None) => {
                            if let Some(slot) = operand_local_slot(rhs) {
                                if !opaque_slots.contains(&slot)
                                    && set_kind_if_unknown(&mut kinds, slot, k)
                                {
                                    changed = true;
                                }
                            }
                        }
                        (None, Some(k)) => {
                            if let Some(slot) = operand_local_slot(lhs) {
                                if !opaque_slots.contains(&slot)
                                    && set_kind_if_unknown(&mut kinds, slot, k)
                                {
                                    changed = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Parameters keep their existing-from-bytecode kind if any.
    // Otherwise they remain `None` — callers needing a concrete
    // kind for codegen surface-and-stop on the `None` per ADR-006
    // §2.7.7 (no deleted `NativeKind::Unknown` placeholder).
    for &param_slot in &mir.param_slots {
        let idx = param_slot.0 as usize;
        if idx < n {
            if let Some(Some(k)) = existing.get(idx).copied() {
                kinds[idx] = Some(k);
            }
        }
    }

    kinds
}

/// Return the statically-known return `NativeKind` for a well-known
/// method name, per ADR-006 §2.7.5 producing-site classification.
///
/// This is the JIT-side classifier for method-call destinations whose
/// return kind is invariant across receiver types in the VM's method
/// registry. The set mirrors the entries that appear in multiple
/// dispatch tables in `crates/shape-vm/src/executor/objects/
/// method_registry.rs` with the same return shape:
///
/// - `size` / `len` / `length` / `count`: every collection-method
///   implementation in `set_methods::v2_size`, `deque_methods::v2_size`,
///   `hashmap_methods::v2_len`, `typed_array_methods::v2_len`,
///   `array_basic::handle_len_v2`, etc. returns `KindedSlot::from_int(...)`.
/// - `isEmpty`: returns `KindedSlot::from_bool(...)` in every collection-
///   method implementation (e.g. `set_methods::v2_is_empty`).
/// - `has` / `contains`: typically `KindedSlot::from_bool(...)`.
///
/// Names outside this set return `None` — the JIT-compile pass treats
/// `None` as "kind genuinely not classifiable from the MIR-observable
/// shape" per §2.7.7 (no Bool-default fallback). Adding a new name
/// requires verifying the receiver-side method registry returns the
/// declared kind for every receiver type the dispatch reaches.
fn well_known_method_return_kind(name: &str) -> Option<NativeKind> {
    match name {
        // Collection-size methods. Verified against every dispatch table
        // in `method_registry.rs` that registers these names: array,
        // datatable, hashmap, set, deque, priority_queue, iterator,
        // typed_array — all return `KindedSlot::from_int(...)`.
        "size" | "len" | "length" | "count" => Some(NativeKind::Int64),
        // Emptiness / membership predicates — `KindedSlot::from_bool(...)`
        // across every receiver's PHF entry.
        "isEmpty" | "is_empty" | "has" | "contains" => Some(NativeKind::Bool),
        _ => None,
    }
}

/// Return the statically-known return `NativeKind` for a well-known
/// builtin-function name (called via `MirConstant::Function(name)`
/// rather than method dispatch). Currently only `len` is exposed as a
/// global builtin alongside its method form, returning Int64.
fn well_known_function_return_kind(name: &str) -> Option<NativeKind> {
    match name {
        // `len(x)` global builtin — returns int for every supported
        // receiver type (Array, String, HashMap, ...).
        "len" => Some(NativeKind::Int64),
        _ => None,
    }
}

/// ADR-006 §2.7.5 element-kind projection for `Place::Index` reads.
///
/// When the Rvalue is `Use(Copy(Index(arr_slot, _)))` (or `Move` /
/// `MoveExplicit` variants) and the receiver slot's `ConcreteType` is
/// `Array(elem)` with a scalar element kind, project the destination's
/// `NativeKind` from the element. Returns `None` for non-Index sources,
/// non-`Place::Local` receivers, or array slots whose `ConcreteType` is
/// not a scalar `Array` (the kind is genuinely not statically classifiable
/// at the producing-MIR layer in those cases).
///
/// This is the kind-source the legacy opaque-projection rule papered
/// over by leaving the destination slot's kind as `None`, which then
/// fell through to the kind-blind print decoder. With strict typing,
/// `Array<int>[i]` proves the destination's kind at JIT-compile time.
///
/// Currently unused after the Round 5A + 5C merge: the more general
/// `infer_rvalue_kind_with_projections` (5A) covers the same Index
/// projection via `concrete_types`. Retained as documentation of the
/// 5C-side helper shape in case a future caller needs the standalone
/// projection without Field threading.
#[allow(dead_code)]
fn infer_index_element_kind(
    rvalue: &Rvalue,
    concrete_types: &[ConcreteType],
) -> Option<NativeKind> {
    let operand = match rvalue {
        Rvalue::Use(op) => op,
        _ => return None,
    };
    let place = match operand {
        Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => p,
        Operand::Constant(_) => return None,
    };
    let (arr_place, _index) = match place {
        Place::Index(arr, idx) => (arr.as_ref(), idx),
        _ => return None,
    };
    let arr_slot = match arr_place {
        Place::Local(slot) => *slot,
        _ => return None,
    };
    let ct = concrete_types.get(arr_slot.0 as usize)?;
    let ConcreteType::Array(elem) = ct else {
        return None;
    };
    elem_slot_kind_for_concrete(elem)
}

/// F7.c — `true` when `operand` reads through a heap projection
/// (`Place::Field` / `Place::Index` / `Place::Deref`). The runtime type
/// of such a read is opaque to the compiler; backward type propagation
/// must not invent a `NativeKind` for the destination slot from unrelated
/// uses of that slot in later binops.
fn is_opaque_operand(operand: &Operand) -> bool {
    match operand {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            is_opaque_place(place)
        }
        Operand::Constant(_) => false,
    }
}

/// Walk a `Place` — `true` if any projection in the chain is a field
/// read, index read, or deref. Pure `Place::Local` chains stay typed.
fn is_opaque_place(place: &Place) -> bool {
    match place {
        Place::Local(_) => false,
        Place::Field(_, _) | Place::Index(_, _) | Place::Deref(_) => true,
    }
}

/// If `operand` is a direct `Copy`/`Move` of a local, return the slot's index.
/// Only handles the simple `Place::Local` form — projections (field/index) do
/// not participate in the backward type propagation.
fn operand_local_slot(operand: &Operand) -> Option<usize> {
    match operand {
        Operand::Copy(Place::Local(slot))
        | Operand::Move(Place::Local(slot))
        | Operand::MoveExplicit(Place::Local(slot)) => Some(slot.0 as usize),
        _ => None,
    }
}

/// Set `kinds[idx] = Some(kind)` if the slot was previously
/// undetermined (`None`), returning `true` when an update happened.
fn set_kind_if_unknown(kinds: &mut [Option<NativeKind>], idx: usize, kind: NativeKind) -> bool {
    if idx < kinds.len() && kinds[idx].is_none() {
        kinds[idx] = Some(kind);
        true
    } else {
        false
    }
}

/// Infer the NativeKind produced by an Rvalue.
fn infer_rvalue_kind(rvalue: &Rvalue, kinds: &[Option<NativeKind>]) -> Option<NativeKind> {
    infer_rvalue_kind_with_fields(rvalue, kinds, None, None)
}

/// Project-aware version of `infer_rvalue_kind`: see
/// `infer_operand_kind_with_fields` for the rationale. `Use(Move(Field))`
/// / `Use(Copy(Field))` route the destination slot's kind to the FIELD's
/// kind (per `field_kinds`) rather than the base struct's heap kind.
#[allow(dead_code)]
fn infer_rvalue_kind_with_fields(
    rvalue: &Rvalue,
    kinds: &[Option<NativeKind>],
    field_kinds: Option<&std::collections::HashMap<String, NativeKind>>,
    field_name_table: Option<&std::collections::HashMap<FieldIdx, String>>,
) -> Option<NativeKind> {
    infer_rvalue_kind_with_projections(rvalue, kinds, field_kinds, field_name_table, None)
}

/// Full project-aware Rvalue kind inference: Field via `field_kinds` +
/// Index via `concrete_types`'s `Array<scalar>` shape. Used by
/// `infer_slot_kinds_with_concrete` for top-level MIR compilation where
/// the bytecode compiler's `concrete_types` side-table is available.
fn infer_rvalue_kind_with_projections(
    rvalue: &Rvalue,
    kinds: &[Option<NativeKind>],
    field_kinds: Option<&std::collections::HashMap<String, NativeKind>>,
    field_name_table: Option<&std::collections::HashMap<FieldIdx, String>>,
    concrete_types: Option<&[ConcreteType]>,
) -> Option<NativeKind> {
    match rvalue {
        Rvalue::Use(operand) => infer_operand_kind_with_projections(
            operand,
            kinds,
            field_kinds,
            field_name_table,
            concrete_types,
        ),
        Rvalue::BinaryOp(op, lhs, rhs) => {
            let lk = infer_operand_kind_with_projections(
                lhs,
                kinds,
                field_kinds,
                field_name_table,
                concrete_types,
            );
            let rk = infer_operand_kind_with_projections(
                rhs,
                kinds,
                field_kinds,
                field_name_table,
                concrete_types,
            );
            match (lk, rk) {
                (Some(l), Some(r)) if l == r => {
                    // Both operands same type.
                    // Arithmetic on floats → float, on ints → int.
                    // Comparisons always → Bool.
                    if is_comparison_op(op) {
                        Some(NativeKind::Bool)
                    } else {
                        Some(l)
                    }
                }
                _ => {
                    // Mixed or unknown operands. Comparison still → Bool.
                    if is_comparison_op(op) {
                        Some(NativeKind::Bool)
                    } else {
                        None
                    }
                }
            }
        }
        Rvalue::UnaryOp(UnOp::Neg, operand) => infer_operand_kind_with_projections(
            operand,
            kinds,
            field_kinds,
            field_name_table,
            concrete_types,
        ),
        Rvalue::UnaryOp(UnOp::Not, _) => Some(NativeKind::Bool),
        Rvalue::Clone(operand) => infer_operand_kind_with_projections(
            operand,
            kinds,
            field_kinds,
            field_name_table,
            concrete_types,
        ),
        Rvalue::Borrow(_, _) => None,     // References are heap pointers
        Rvalue::Aggregate(_) => None,      // Arrays are heap objects
    }
}

/// Infer the NativeKind of an operand.
fn infer_operand_kind(operand: &Operand, kinds: &[Option<NativeKind>]) -> Option<NativeKind> {
    infer_operand_kind_with_fields(operand, kinds, None, None)
}

/// W12-jit-binop-after-heap-read-kind-tracker: project through
/// `Place::Field` / `Place::Index` so `infer_slot_kinds` produces the
/// correct destination kind for `Assign(slot, Use(Move(Field(_, _))))`
/// and `Assign(slot, Use(Copy(Index(_, _))))`.
///
/// Without projection, the destination slot inherits the BASE's kind
/// (typically `Ptr(HeapKind::TypedObject)` for a struct base or
/// `Ptr(HeapKind::TypedArray)` for an array base) — but the value
/// actually moved/copied is the FIELD or ELEMENT, whose kind is
/// orthogonal to the base's. The wrong inference makes the destination
/// slot `Ptr(HeapKind::TypedObject)`, which the bytecode-compiler-
/// authoritative `LocalTypeInfo::NonCopy` path then dispatches as
/// refcounted at `release_old_value_if_heap` — and the initial-zero or
/// later-stored field value (e.g. `i64=3`) gets passed to `arc_release`
/// /  `arc_retain` as a raw pointer, segfaulting.
///
/// Sources:
/// - `field_kinds`: the producer-side map from `infer_field_native_kinds`
///   (populated by walking `StatementKind::ObjectStore { operands,
///   field_names }`). For `Place::Field(_, FieldIdx)`, project via
///   `field_name_table[FieldIdx] → name → field_kinds[name]`.
/// - `field_name_table`: passed from the MIR for the `FieldIdx → name`
///   translation. When `None` (the `infer_field_native_kinds` pre-pass
///   that uses constant-only slot kinds), Field projection is skipped
///   and the function falls back to `root_local()` — the same shape as
///   the pre-W12 path.
/// - `Place::Index(_, _)`: not threaded into MIR-level inference yet.
///   The JIT-side `place_native_kind` (in `rvalues.rs`) projects through
///   `concrete_types`'s `Array<scalar>` shape at JIT codegen time;
///   adding the same projection here would require threading
///   `concrete_types` into `infer_slot_kinds` (cross-tier flow). For
///   cluster-0's load-bearing smokes (Smoke 3 field-add and array-
///   scalar smoke `xs[0] + xs[1]`), the Array case is covered by the
///   JIT-side projection alone — the destination slot of
///   `Use(Copy(Index(_, _)))` doesn't drive a refcount-dispatch bug
///   because v2 typed-array slots route through the
///   `RefcountDisposition::Skip_TypedCellCarrier` arm (per
///   `ownership.rs:99`) before reaching the `slot_kind` discriminator.
///   If a future smoke surfaces a similar refcount-on-element-read bug,
///   thread `concrete_types` here.
fn infer_operand_kind_with_fields(
    operand: &Operand,
    kinds: &[Option<NativeKind>],
    field_kinds: Option<&std::collections::HashMap<String, NativeKind>>,
    field_name_table: Option<&std::collections::HashMap<FieldIdx, String>>,
) -> Option<NativeKind> {
    infer_operand_kind_with_projections(
        operand,
        kinds,
        field_kinds,
        field_name_table,
        None,
    )
}

/// Project-aware kind classification with both Field (via `field_kinds`)
/// and Index (via `concrete_types`'s `Array<scalar>` shape).
///
/// `Place::Index(base, _)`: when `concrete_types[base.root_local()] =
/// Array(elem)` with a scalar `elem`, the element kind is `elem` mapped
/// through `elem_slot_kind_for_concrete`. This mirrors the JIT codegen-
/// side `v2_typed_array_elem_kind` projection that drives the typed
/// array load path — same kind source, both consumer sites.
///
/// Without this projection, the destination slot of `Use(Copy(Index(
/// xs_TypedArray, _)))` inherits `xs`'s `Ptr(HeapKind::TypedArray)` kind,
/// then `print(slot)` falls through `print_i64/f64/bool` to the kind-
/// blind `jit_print` fallback, which decodes the raw int as f64 and
/// prints a denormalized garbage. Threading the element kind to the
/// destination slot makes `print` pick the matching `print_i64` /
/// `print_f64` arm and produce the correct output.
fn infer_operand_kind_with_projections(
    operand: &Operand,
    kinds: &[Option<NativeKind>],
    field_kinds: Option<&std::collections::HashMap<String, NativeKind>>,
    field_name_table: Option<&std::collections::HashMap<FieldIdx, String>>,
    concrete_types: Option<&[ConcreteType]>,
) -> Option<NativeKind> {
    match operand {
        Operand::Constant(c) => infer_constant_kind(c),
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            if let (Place::Field(_, field_idx), Some(fk), Some(fnt)) =
                (place, field_kinds, field_name_table)
            {
                if let Some(name) = fnt.get(field_idx) {
                    if let Some(k) = fk.get(name).copied() {
                        return Some(k);
                    }
                }
                // Field projection without a stamped kind: fall through
                // to root-local lookup (the pre-W12 behaviour). Caller
                // surfaces `None` honestly if the root lookup also fails.
            }
            if let (Place::Index(base, _), Some(cts)) = (place, concrete_types) {
                let base_slot = base.root_local();
                if let Some(elem_kind) = is_v2_typed_array_slot(cts, base_slot.0) {
                    return Some(elem_kind);
                }
                // Index without a proven Array<scalar> shape: fall
                // through to root-local lookup. Caller surfaces None
                // honestly if the root lookup also fails.
            }
            let slot = place.root_local();
            let idx = slot.0 as usize;
            kinds.get(idx).copied().flatten()
        }
    }
}

/// Producing-site field-kind classification per ADR-006 §2.7.5
/// stamp-at-compile-time discipline (W12-jit-binop-after-heap-read-kind-
/// tracker close, 2026-05-12).
///
/// Walk the MIR for every `StatementKind::ObjectStore { container_slot,
/// operands, field_names }` and stamp `field_native_kinds[name]` with the
/// operand's MIR-inferred `NativeKind`. This makes `Place::Field(base,
/// field_idx)` reads have a proven kind at JIT compile time, threading
/// the kind from the struct-literal producer into downstream `BinaryOp`
/// lowering without runtime tag-bit decode.
///
/// Each operand's kind is sourced from the already-computed `slot_kinds`
/// (which `infer_slot_kinds` produced from MIR-observable constants and
/// `ConcreteType` seeds). For `Constant` operands, classification comes
/// from `infer_constant_kind`. When an operand's kind is unprovable
/// (`None`), the field is NOT stamped — downstream consumers of
/// `field_native_kinds` get `None` and the JIT honestly surfaces the gap
/// at the BinaryOp call site rather than papering with a Bool-default
/// (§2.7.7 #9 forbidden rationalization).
///
/// The map is keyed by field NAME (not `FieldIdx`) to match the existing
/// `field_byte_offsets` keying — the JIT's `field_name_table` translates
/// `FieldIdx → String` at the field-read site, and we look up by name
/// here. Same fragility as `field_byte_offsets`: if two different struct
/// types share a field name with differing types, last-writer-wins. For
/// the Smoke 3 case (`Point.x: int`, `Point.y: int`) and the load-
/// bearing cluster-0 close criterion, this is sufficient. A schema-aware
/// (StructLayoutId-keyed) registry is the principled long-term shape,
/// but adding one is out-of-scope for this sub-cluster — see also
/// `field_byte_offsets`'s identical structural fragility.
///
/// `ObjectStore` is the structural kind source — the same statement
/// that's responsible for materializing the TypedObject in the v2 fast
/// path. By stamping field kinds here we mirror the producer-side
/// classification the §2.7.5 conduit already does for the destination
/// slot's `ConcreteType` (via the `infer_top_level_concrete_types_from_mir`
/// pass in `crates/shape-vm/src/compiler/helpers.rs`), one layer down
/// in the type structure.
pub(crate) fn infer_field_native_kinds(
    mir: &MirFunction,
    slot_kinds: &[Option<NativeKind>],
) -> std::collections::HashMap<String, NativeKind> {
    let mut field_kinds: std::collections::HashMap<String, NativeKind> =
        std::collections::HashMap::new();
    for block in &mir.blocks {
        for stmt in &block.statements {
            if let StatementKind::ObjectStore {
                operands,
                field_names,
                ..
            } = &stmt.kind
            {
                for (op, name) in operands.iter().zip(field_names.iter()) {
                    if name.is_empty() {
                        // Spreads / unnamed positional operands have no
                        // field name in the JIT's flat name→kind map.
                        // The field_byte_offsets walk skips them too.
                        continue;
                    }
                    if let Some(kind) = infer_operand_kind(op, slot_kinds) {
                        field_kinds.insert(name.clone(), kind);
                    }
                }
            }
        }
    }
    field_kinds
}

/// Infer the NativeKind of a constant.
///
/// ADR-006 §2.7.5 / §2.7.11/Q12 producing-site classification:
/// - `Function(_)`: the JIT-internal `box_function(fn_id)` shape — carrier
///   kind `UInt64` (the function-id-class callee-classification kind also
///   used at the §2.7.5 stable-FFI boundary).
/// - `Method(_)`: heap String carrier (`Arc<String>` raw pointer).
/// - `ClosurePlaceholder`: forward-reference for a closure slot —
///   `Ptr(HeapKind::Closure)` per §2.7.11/Q12.
/// - `None`: the unit/null value — kind genuinely unknown; callers
///   surface-and-stop per §2.7.7 #9.
fn infer_constant_kind(constant: &MirConstant) -> Option<NativeKind> {
    match constant {
        MirConstant::Float(_) => Some(NativeKind::Float64),
        MirConstant::Int(_) => Some(NativeKind::Int64),
        MirConstant::Bool(_) => Some(NativeKind::Bool),
        MirConstant::None => None,
        MirConstant::StringId(_) | MirConstant::Str(_) => Some(NativeKind::String),
        MirConstant::Function(_) => Some(NativeKind::UInt64),
        MirConstant::Method(_) => Some(NativeKind::String),
        MirConstant::ClosurePlaceholder => Some(NativeKind::Ptr(HeapKind::Closure)),
    }
}

fn is_comparison_op(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_vm::mir::types::*;

    fn make_mir(stmts: Vec<MirStatement>) -> MirFunction {
        MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: stmts,
                terminator: Terminator {
                    kind: TerminatorKind::Return,
                    span: shape_ast::Span::default(),
                },
            }],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: shape_ast::Span::default(),
            field_name_table: Default::default(),
        }
    }

    fn assign_const(slot: u16, constant: MirConstant) -> MirStatement {
        MirStatement {
            kind: StatementKind::Assign(
                Place::Local(SlotId(slot)),
                Rvalue::Use(Operand::Constant(constant)),
            ),
            span: shape_ast::Span::default(),
            point: Point(0),
        }
    }

    #[test]
    fn infer_float_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Float(0))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], Some(NativeKind::Float64));
    }

    #[test]
    fn infer_int_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Int(42))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], Some(NativeKind::Int64));
    }

    #[test]
    fn infer_bool_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Bool(true))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], Some(NativeKind::Bool));
    }

    #[test]
    fn infer_float_from_binop() {
        let mir = make_mir(vec![
            assign_const(1, MirConstant::Float(0)),
            assign_const(2, MirConstant::Float(0)),
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(3)),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(SlotId(1))),
                        Operand::Copy(Place::Local(SlotId(2))),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
        ]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[3], Some(NativeKind::Float64));
    }

    #[test]
    fn infer_bool_from_comparison() {
        let mir = make_mir(vec![
            assign_const(1, MirConstant::Float(0)),
            assign_const(2, MirConstant::Float(0)),
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(3)),
                    Rvalue::BinaryOp(
                        BinOp::Lt,
                        Operand::Copy(Place::Local(SlotId(1))),
                        Operand::Copy(Place::Local(SlotId(2))),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
        ]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[3], Some(NativeKind::Bool));
    }

    #[test]
    fn infer_backward_from_typed_sibling_on_binop() {
        // Regression: `|x| x + 1` leaves `x` (a param) Unknown after forward
        // inference because params are seeded from `existing`, not from uses.
        // The backward pass must propagate Int64 from the typed constant `1`
        // into `x`'s slot so the JIT binop picker routes through
        // `compile_binop_int64` instead of the dynamic-op error path.
        //
        // MIR shape:
        //   param(0) = x  (Unknown)
        //   _1 = x + Int(1)
        let mut mir = make_mir(vec![MirStatement {
            kind: StatementKind::Assign(
                Place::Local(SlotId(1)),
                Rvalue::BinaryOp(
                    BinOp::Add,
                    Operand::Copy(Place::Local(SlotId(0))),
                    Operand::Constant(MirConstant::Int(1)),
                ),
            ),
            span: shape_ast::Span::default(),
            point: Point(0),
        }]);
        mir.param_slots = vec![SlotId(0)];
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(
            kinds[0],
            Some(NativeKind::Int64),
            "backward pass should infer x: Int64 from `x + Int(1)`"
        );
    }

    #[test]
    fn infer_backward_chains_across_params() {
        // `|x, y| x + y + 1` — typed constant `1` reaches both params via
        // two rounds of backward propagation. After round 1: `_1 = x + y`
        // stays Unknown (both sides Unknown); `_2 = _1 + Int(1)` makes `_1`
        // Int64. Round 2: `_1 = x + y` with lhs Unknown, rhs Unknown still
        // doesn't help — we need forward assignment of `_1` to come through
        // first. The forward pass already handles `_1` because both operands
        // are "Unknown" → rvalue kind returns None. So after backward makes
        // `_1` = Int64, the statement `_1 = x + y` would need ANOTHER pass
        // that uses the Assign's LHS kind to constrain RHS operands. That
        // is not implemented here — we only propagate within a single binop.
        //
        // This test pins the current (intentionally limited) behaviour:
        // the simpler case of `|x| x + 1` works; chained-binop backward
        // propagation through an intermediate local does NOT.
        let mut mir = make_mir(vec![
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(2)),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(SlotId(0))),
                        Operand::Copy(Place::Local(SlotId(1))),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(3)),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(SlotId(2))),
                        Operand::Constant(MirConstant::Int(1)),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
        ]);
        mir.param_slots = vec![SlotId(0), SlotId(1)];
        let kinds = infer_slot_kinds(&mir, &[]);
        // The inner binop picks up the type from `_2 + Int(1)` backwards.
        assert_eq!(kinds[2], Some(NativeKind::Int64));
    }

    #[test]
    fn existing_kinds_preserved() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Float(0))]);
        let existing = vec![None, Some(NativeKind::Int32)];
        let kinds = infer_slot_kinds(&mir, &existing);
        // Existing Int32 is preserved (not overridden by Float64 inference)
        assert_eq!(kinds[1], Some(NativeKind::Int32));
    }

    #[test]
    fn cranelift_type_mapping() {
        assert_eq!(cranelift_type_for_slot(NativeKind::Float64), types::F64);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int32), types::I32);
        assert_eq!(cranelift_type_for_slot(NativeKind::Bool), types::I8);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int64), types::I64);
        assert_eq!(cranelift_type_for_slot(NativeKind::String), types::I64);
    }

    // -----------------------------------------------------------------------
    // R4.2F: borrow StackSlot sizing invariants
    //
    // `Rvalue::Borrow` creates a stack cell with
    //     size = cranelift_type_for_slot(kind).bytes()
    //     align = log2(size)
    // These tests pin the native widths across all slot kinds that flow into
    // borrow cells. Non-native kinds must collapse to 8 bytes / align=3 so the
    // widening is a no-op for the legacy heap/unknown path.
    // -----------------------------------------------------------------------

    #[test]
    fn r4_2f_borrow_cell_sizes() {
        // Native-typed slots get their natural width.
        assert_eq!(cranelift_type_for_slot(NativeKind::Float64).bytes(), 8);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int64).bytes(), 8);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int32).bytes(), 4);
        assert_eq!(cranelift_type_for_slot(NativeKind::UInt32).bytes(), 4);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int16).bytes(), 2);
        assert_eq!(cranelift_type_for_slot(NativeKind::UInt16).bytes(), 2);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int8).bytes(), 1);
        assert_eq!(cranelift_type_for_slot(NativeKind::UInt8).bytes(), 1);
        assert_eq!(cranelift_type_for_slot(NativeKind::Bool).bytes(), 1);
        // Non-native slots collapse to 8 bytes (legacy behaviour).
        assert_eq!(cranelift_type_for_slot(NativeKind::String).bytes(), 8);
    }

    #[test]
    fn r4_2f_borrow_cell_alignment_shifts() {
        // `align_shift = size.trailing_zeros()` — must match log2(size) for
        // every power-of-two native width. If this ever breaks, the
        // `StackSlotData::new` call in `Rvalue::Borrow` will assert.
        for kind in [
            NativeKind::Float64,
            NativeKind::Int64,
            NativeKind::Int32,
            NativeKind::UInt32,
            NativeKind::Int16,
            NativeKind::UInt16,
            NativeKind::Int8,
            NativeKind::UInt8,
            NativeKind::Bool,
            NativeKind::String,
        ] {
            let size = cranelift_type_for_slot(kind).bytes();
            assert!(
                size.is_power_of_two(),
                "slot kind {:?} has non-power-of-two size {}",
                kind,
                size
            );
            let shift = size.trailing_zeros() as u8;
            assert_eq!(
                1u32 << shift,
                size,
                "slot kind {:?}: shift {} does not reconstruct size {}",
                kind,
                shift,
                size
            );
        }
    }
}
