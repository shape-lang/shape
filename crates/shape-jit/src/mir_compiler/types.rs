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
///
/// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
/// extended with `String → StringV2` / `Decimal → DecimalV2` per ADR-006
/// §2.7.5 + §2.7.24 Q25.A SUPERSEDED + audit deliverable (b) §4.1.B. The
/// `StringV2` / `DecimalV2` element kinds route through the v2-raw
/// `TypedArray<*const StringObj>` / `TypedArray<*const DecimalObj>`
/// allocators added in `v2_array_new_func`; per-element literal-upgrade
/// is handled in `emit_v2_array_aggregate`'s StringV2/DecimalV2 arms
/// mirroring the VM-side `NewStringV2` / `NewDecimalV2` opcodes at
/// `crates/shape-vm/src/executor/v2_handlers/array.rs:803-858`.
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
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment.
        ConcreteType::F32 => Some(NativeKind::Float32),
        ConcreteType::Char => Some(NativeKind::Char),
        // ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
        // Array<string> / Array<decimal> route through v2-raw
        // `TypedArray<*const StringObj>` / `TypedArray<*const DecimalObj>`
        // carriers per ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED + audit
        // deliverable (b) §4.1.B.
        ConcreteType::String => Some(NativeKind::StringV2),
        ConcreteType::Decimal => Some(NativeKind::DecimalV2),
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
        // ── Phase 3 cluster-0 Round 11-trinity 11E (2026-05-13) ─────────
        // Collection / concurrency carriers — taxonomy extended in
        // `shape-value/src/v2/concrete_type.rs` per the Round 10 surfaced
        // item (B). Each ConcreteType arm maps to its dedicated
        // `HeapKind` ordinal (§2.7.15 / §2.7.17 / §2.7.18 / §2.7.20 /
        // §2.7.25) and dispatches through Round 9's `retain_func_for_place`
        // / `release_func_for_place` 8-arm extension. Pre-11E the JIT
        // EnumStore consumer carried out-of-band kind seeding at the
        // `mir_compiler/types.rs` EnumStore arm because ConcreteType
        // didn't have these variants; with 11E landed the in-band
        // `concrete_seed` path is authoritative.
        ConcreteType::HashSet(_) => NativeKind::Ptr(HeapKind::HashSet),
        ConcreteType::Deque(_) => NativeKind::Ptr(HeapKind::Deque),
        ConcreteType::PriorityQueue => NativeKind::Ptr(HeapKind::PriorityQueue),
        ConcreteType::Channel(_) => NativeKind::Ptr(HeapKind::Channel),
        ConcreteType::Mutex(_) => NativeKind::Ptr(HeapKind::Mutex),
        ConcreteType::Atomic => NativeKind::Ptr(HeapKind::Atomic),
        ConcreteType::Lazy(_) => NativeKind::Ptr(HeapKind::Lazy),
        // ── Round 19 S1.5 W12-nativekind-scalar-additions ──────────
        // (2026-05-14) — ADR-006 §2.7.5 amendment.
        ConcreteType::F32 => NativeKind::Float32,
        ConcreteType::Char => NativeKind::Char,
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
            func,
            args,
            destination,
            ..
        } = &block.terminator.kind
        {
            if let Place::Local(slot) = destination {
                let idx = slot.0 as usize;
                if idx < n && kinds[idx].is_none() {
                    let ret_kind = match func {
                        Operand::Constant(MirConstant::Method(name)) => {
                            // ADR-006 §2.7.5 producing-site conduit
                            // extension for parametric-return methods
                            // (Phase 3 cluster-0 Round 11-trinity Part b,
                            // 2026-05-13). Method-return kinds split into
                            // two cohorts: invariant-across-receivers
                            // (`size`/`isEmpty`/...) classified via
                            // `well_known_method_return_kind(name)`, and
                            // receiver-parametric (`HashMap.get →
                            // Option<V>`, `Mutex.get → T`, `Atomic.load →
                            // i64`, `Array.sum/mean/min/max → element`)
                            // classified via
                            // `parametric_method_return_kind(name,
                            // receiver_ct)` where `receiver_ct` is
                            // `concrete_types[args[0].root_local()]`.
                            //
                            // Invariant-name classification runs first
                            // (current behavior); when it returns None,
                            // fall through to the receiver-parametric
                            // classifier. This preserves the existing
                            // Round 5C semantics for size/len/etc.
                            // exactly, and extends classification for
                            // methods whose return kind genuinely
                            // depends on the receiver shape.
                            well_known_method_return_kind(name).or_else(|| {
                                parametric_method_return_kind_from_receiver(
                                    name,
                                    args,
                                    concrete_types,
                                )
                            })
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
                // W12-jit-call-method-shell-rebuild (Phase 3 cluster-0
                // Round 10 / 8B.2, 2026-05-13): ADR-006 §2.7.5 producer-
                // side classification for primitive-collection ctors.
                //
                // The bytecode compiler doesn't synthesize a
                // `ConcreteType::HashSet` / `Deque` / `PriorityQueue` /
                // `Channel` / `Mutex` / `Atomic` / `Lazy` variant — those
                // types aren't modeled in the §2.7.6 concrete-types
                // taxonomy yet (W17-collection-concrete-types is the
                // tracked follow-up). The MIR-emit-side EnumStore is the
                // load-bearing kind source: when `variant_name` is one of
                // the 8 collection names, the container slot bits are
                // exactly `Arc::into_raw(Arc<XData>) as u64` per Round 9's
                // typed-Arc ctor FFI bodies, and the slot's `NativeKind`
                // is the matching `Ptr(HeapKind::*)` arm.
                //
                // Without this seed the slot kind on the §2.7.7 / Q9
                // parallel-kind track stays `None` → falls back to the
                // §2.7.5 carrier kind `UInt64` at the receiver push site
                // → the `jit_call_method` shell's delegation predicate
                // routes to the legacy JIT-format dispatch path (which
                // doesn't know how to read `Arc<HashSetData>` raw
                // pointers as JIT NaN-box bits) → method dispatch
                // surfaces silently as TAG_NULL. The kind seed here
                // closes that gap.
                //
                // The `HashMap` collection ctor maps to
                // `Ptr(HeapKind::HashMap)`. Note that this overlaps with
                // the `ConcreteType::HashMap(K, V)` →
                // `Ptr(HeapKind::HashMap)` seed for v2 typed HashMaps;
                // both paths converge on the same carrier kind. The MIR
                // EnumStore for `HashMap()` runs only for the bare-form
                // ctor (`is_bare_collection_ctor` accepts it); typed
                // HashMaps from `HashMap<string, int>()` go through the
                // bytecode compiler's typed-HashMap fast path, which
                // populates `concrete_types[slot] = HashMap(_, _)`
                // directly and the `concrete_seed` upstream of this pass
                // already handled it.
                StatementKind::EnumStore {
                    container_slot,
                    variant_name: Some(name),
                    ..
                } => {
                    let collection_kind = match name.as_str() {
                        "Set" | "HashSet" => Some(NativeKind::Ptr(HeapKind::HashSet)),
                        "HashMap" => Some(NativeKind::Ptr(HeapKind::HashMap)),
                        "Deque" => Some(NativeKind::Ptr(HeapKind::Deque)),
                        "PriorityQueue" => {
                            Some(NativeKind::Ptr(HeapKind::PriorityQueue))
                        }
                        "Channel" => Some(NativeKind::Ptr(HeapKind::Channel)),
                        "Mutex" => Some(NativeKind::Ptr(HeapKind::Mutex)),
                        "Atomic" => Some(NativeKind::Ptr(HeapKind::Atomic)),
                        "Lazy" => Some(NativeKind::Ptr(HeapKind::Lazy)),
                        _ => None,
                    };
                    if let Some(k) = collection_kind {
                        let idx = container_slot.0 as usize;
                        if idx < n {
                            // Override the upstream concrete_seed: the
                            // bytecode compiler's type-checker classifies
                            // `Set` / `HashMap` / etc. as `ConcreteType::
                            // Struct(_)` (since the stdlib defines them as
                            // typed structs), which `concrete_seed` maps
                            // to `Ptr(HeapKind::TypedObject)`. That's a
                            // wrong-carrier classification for the typed-
                            // Arc ctors landed in Round 9 — the slot bits
                            // are `Arc::into_raw(Arc<HashSetData>)`, NOT
                            // `Arc::into_raw(Arc<TypedObjectStorage>)`,
                            // and the kind drives downstream
                            // retain/release dispatch through Round 9's
                            // `retain_func_for_place` /
                            // `release_func_for_place` 8-arm extension.
                            // A `TypedObject`-labeled slot would dispatch
                            // through `arc_retain` / `arc_release` on the
                            // legacy `UnifiedValue<T>` HeapHeader at
                            // offset 4, which would scribble on the
                            // `HashSetData` payload (audit §5 carrier-
                            // shape rule). The EnumStore producer-site
                            // classification IS authoritative for these
                            // slots per the §2.7.5 stamp-at-MIR-emit
                            // discipline.
                            //
                            // ADR-006 W17-collection-concrete-types is the
                            // tracked follow-up to extend `ConcreteType`
                            // with `HashSet` / `Deque` / `PriorityQueue`
                            // / `Channel` / `Mutex` / `Atomic` / `Lazy`
                            // arms so the bytecode compiler's seed gets
                            // these right at the source.
                            kinds[idx] = Some(k);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // W12-jit-call-method-shell-rebuild post-pass (Phase 3 cluster-0 Round
    // 10 / 8B.2, 2026-05-13): propagate collection-ctor kinds through
    // identity-Use chains. The bytecode compiler's `concrete_seed` maps
    // `let s = Set()`'s user-visible slot to `Ptr(HeapKind::TypedObject)`
    // (since the stdlib defines `Set` as a typed struct). The forward
    // pass's "first write wins / no overwrite on conflict" rule preserves
    // that wrong-carrier classification: the EnumStore arm above
    // overrides the EnumStore container slot to `Ptr(HeapKind::HashSet)`,
    // but a downstream `Assign(s_slot, Use(Move(tmp_slot)))` leaves
    // `s_slot` at its pre-seeded `Ptr(TypedObject)` instead of inheriting
    // the corrected `Ptr(HashSet)` from `tmp_slot`.
    //
    // This post-pass walks Assign-Use chains and propagates any of the 8
    // typed-Arc collection kinds from source to destination, overriding
    // the pre-seeded `Ptr(TypedObject)` (or any other carrier kind) —
    // because the typed-Arc carrier-shape rule (audit §5) requires the
    // slot kind to drive retain/release dispatch correctly. A
    // `TypedObject`-labeled slot would route through `arc_retain` /
    // `arc_release` on the `UnifiedValue<T>` HeapHeader at offset 4,
    // scribbling on the `Arc<HashSetData>` payload. Override is correct
    // because the EnumStore producer is authoritative.
    //
    // The pass iterates until fixpoint (bounded: each iteration converts
    // at most one slot, so it terminates in O(num_locals) iterations).
    // For `let s = Set(); let t = s; let u = t; ...` chains this
    // propagates through every binding to the deepest use.
    fn is_collection_kind(k: NativeKind) -> bool {
        matches!(
            k,
            NativeKind::Ptr(HeapKind::HashSet)
                | NativeKind::Ptr(HeapKind::HashMap)
                | NativeKind::Ptr(HeapKind::Deque)
                | NativeKind::Ptr(HeapKind::PriorityQueue)
                | NativeKind::Ptr(HeapKind::Channel)
                | NativeKind::Ptr(HeapKind::Mutex)
                | NativeKind::Ptr(HeapKind::Atomic)
                | NativeKind::Ptr(HeapKind::Lazy)
        )
    }
    let mut changed = true;
    let mut iterations = 0;
    let max_iterations = n + 4; // safety bound
    while changed && iterations < max_iterations {
        changed = false;
        iterations += 1;
        for block in &mir.blocks {
            for stmt in &block.statements {
                if let StatementKind::Assign(
                    Place::Local(dst),
                    Rvalue::Use(operand),
                ) = &stmt.kind
                {
                    let src_slot = match operand {
                        Operand::Copy(Place::Local(s))
                        | Operand::Move(Place::Local(s))
                        | Operand::MoveExplicit(Place::Local(s)) => Some(*s),
                        _ => None,
                    };
                    if let Some(src) = src_slot {
                        let dst_idx = dst.0 as usize;
                        let src_idx = src.0 as usize;
                        if dst_idx < n && src_idx < n {
                            if let Some(src_kind) = kinds[src_idx] {
                                if is_collection_kind(src_kind)
                                    && kinds[dst_idx] != Some(src_kind)
                                {
                                    kinds[dst_idx] = Some(src_kind);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
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

/// ADR-006 §2.7.5 producing-site classification for parametric-return
/// method calls (Phase 3 cluster-0 Round 11-trinity Part b, 2026-05-13).
///
/// Companion of `well_known_method_return_kind`: that classifier covers
/// methods whose return type is INVARIANT across receiver types
/// (`size`/`len`/`length`/`count` → Int64; `isEmpty`/`contains`/`has` →
/// Bool — verified against every dispatch table in
/// `crates/shape-vm/src/executor/objects/method_registry.rs`). This
/// classifier covers methods whose return type DEPENDS on the receiver's
/// `ConcreteType` parametric form:
///
/// - `Array<T>.sum() / .mean() / .min() / .max() / .first() / .last() /
///   .pop() / .get(i)` — return kind flows from `ConcreteType::Array(T)`
///   element type to a scalar `NativeKind` (Int64 for `Array<int>`,
///   Float64 for `Array<number>`, etc.). `.first()/.last()/.pop()`
///   wrap in `Option<T>`, classified as `Ptr(HeapKind::Option)` carrier
///   bits per §2.7.17.
/// - `HashMap<K, V>.get(K) → Option<V>` — receiver
///   `ConcreteType::HashMap(_, V)` returns `Ptr(HeapKind::Option)`
///   (the wrapped V is on the Option's inner kind track, picked up by
///   downstream EnumPayload via `infer_enum_payload_kind`).
/// - `Mutex<T>.get() → T` — receiver `ConcreteType::Mutex(T)` returns
///   `native_kind_from_concrete_type(T)`.
/// - `Atomic.load() / .fetch_add(d) / .fetch_sub(d) /
///   .compare_exchange(...)` — i64-only at landing per §2.7.25; return
///   Int64 unconditionally.
/// - `Lazy<T>.get() → T` — receiver `ConcreteType::Lazy(T)` returns
///   `native_kind_from_concrete_type(T)`.
///
/// Names outside this set return `None` — the slot's kind genuinely
/// isn't statically classifiable from the receiver+method pair alone,
/// per §2.7.7 (no Bool-default fallback).
///
/// The receiver's `ConcreteType` is sourced from `concrete_types[args[0]
/// .root_local()]` per §2.7.5 producing-site discipline. When the
/// receiver isn't a `Place::Local` projection (e.g. constant receiver,
/// no concrete_types entry), the classifier returns `None` — the
/// classifier is one of multiple kind sources at this point in the
/// inference pass, and surfacing-and-stopping isn't appropriate here
/// (other downstream passes still get a chance to stamp the slot).
///
/// # User-defined-trait surface boundary (Phase 3 cluster-0 Round 12 T1)
///
/// `ConcreteType::Struct(_)` receivers (user-defined `type X {}` values)
/// fall into the `_ => None` arm by design. Smoke 3
/// (`trait T { name(): string } type X {} impl T for X { method name()
/// { "x" } } let t = X {} print(t.name())`) requires `t.name()` to be
/// classified as `NativeKind::String` from the trait's declared return
/// type. The classifier cannot do this because the receiver kind-source
/// is structurally insufficient:
///
/// 1. The receiver slot's `ConcreteType` is `Struct(StructLayoutId(0))`
///    — the bytecode compiler's `concrete_type_from_annotation`
///    (`crates/shape-vm/src/compiler/v2_map_emission.rs:357`) returns
///    the `StructLayoutId(0)` placeholder for every user struct name
///    because the layout-id registry is not wired (the function's
///    `_ => None` arm at line 378 carries the comment "Phase 1.1 Agent 3
///    will fill this in"). So `concrete_types[receiver_slot]` does NOT
///    distinguish `X` from `Y` from `Point` from any other user struct.
/// 2. The trait registry (`TypeRegistry::traits: HashMap<String,
///    TraitDef>` in `crates/shape-runtime/src/type_system/environment/
///    registry.rs:111`) holds the trait's declared return type
///    (`InterfaceMember::Method { return_type: TypeAnnotation, .. }`),
///    but the `BytecodeProgram` (`crates/shape-vm/src/bytecode/
///    core_types.rs`) does NOT persist this — it only carries
///    `trait_method_symbols: HashMap<String, String>` (the resolved
///    function name per `(trait, type, impl, method)` key) and
///    `trait_vtables` (vtables keyed by `Trait::ConcreteType`). Neither
///    carries the declared trait method return type.
/// 3. The `function_return_concrete_types: Vec<ConcreteType>` side-table
///    (the parallel pattern §2.7.5 the JIT consumes for direct calls,
///    `core_types.rs:356`) is keyed on function index and built from
///    `FunctionDef.return_type` annotations
///    (`compiler_impl_reference_model.rs:1473`). For trait impl methods
///    desugared via `desugar_impl_method`
///    (`crates/shape-vm/src/compiler/statements.rs:1646`), the impl's
///    `method.return_type` is whatever the impl source declared — for
///    Smoke 3's `impl T for X { method name() { "x" } }` it is `None`
///    (the impl doesn't repeat the trait's `: string` annotation), so
///    `function_return_concrete_types[X::name] = ConcreteType::Void`.
///    The trait's declared return type does not propagate to the impl's
///    function definition.
///
/// Closing this surface requires extending the bytecode→JIT data
/// conduit — adding a new side-table on `BytecodeProgram` that
/// persists per-trait-method declared return `ConcreteType`s,
/// populated at impl-block compilation time from the type registry's
/// `TraitDef.members[*].Required(Method { return_type, .. })` and
/// `TraitDef.members[*].Default(MethodDef { return_type, .. })`
/// entries. This is a cross-crate extension (mirrors the existing
/// `function_return_concrete_types` pattern from Round-6 W12-jit-call-
/// return-kind close 2026-05-12) and is ADR amendment territory per
/// the agent prompt's surface-and-stop list ("If the trait registry
/// isn't accessible from the JIT MIR builder layer (cross-crate
/// boundary issue) — STOP and surface").
///
/// The pin tests `user_defined_trait_method_on_struct_returns_none`
/// and `user_defined_trait_method_call_terminator_remains_unstamped`
/// assert the surface — they are intentional surface pins, not
/// regressions to be papered over by a Bool-default fallback or a
/// hard-coded `"name"` → `String` arm.
fn parametric_method_return_kind_from_receiver(
    name: &str,
    args: &[Operand],
    concrete_types: &[ConcreteType],
) -> Option<NativeKind> {
    use shape_value::heap_value::HeapKind;
    // args[0] is the receiver per the MIR lowering convention
    // (`mir/lowering/expr.rs::Expr::MethodCall` pushes the receiver as
    // arg index 0). Constant receivers can't carry a ConcreteType slot
    // — no classification possible.
    let receiver = args.first()?;
    let receiver_slot = match receiver {
        Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => p.root_local(),
        Operand::Constant(_) => return None,
    };
    let receiver_ct = concrete_types.get(receiver_slot.0 as usize)?;
    // Skip when the receiver slot's ConcreteType wasn't proven by the
    // upstream concrete-types conduit (the Void placeholder).
    if matches!(receiver_ct, ConcreteType::Void) {
        return None;
    }
    match (name, receiver_ct) {
        // ── Array element-typed accessors ──────────────────────────
        // `Array<T>.sum() / .mean() / .min() / .max()` return the
        // element type's scalar kind. The VM-side `array_basic.rs` /
        // `typed_array_methods.rs` PHF entries return
        // `KindedSlot::from_<elem>(...)` per receiver-element kind.
        // ADR-006 §2.7.5 / Round 8A receiver-recovery soundness:
        // the §2.7.5 carrier shape for the element is preserved
        // verbatim in the return value.
        ("sum" | "mean" | "min" | "max", ConcreteType::Array(elem)) => {
            native_kind_from_concrete_type(elem)
        }
        // `Array.get(i)` — returns element T directly (the VM-side
        // bounds-checked accessor; non-Option return).
        ("get", ConcreteType::Array(elem)) => native_kind_from_concrete_type(elem),
        // `Array.first() / .last() / .pop()` — wrap in Option<T>.
        // The destination slot's bits are an `Arc::into_raw(Arc<
        // OptionData>) as u64` carrier per §2.7.17; the EnumPayload
        // path picks up the inner V from the surrounding
        // `concrete_types[r]` Option arm.
        ("first" | "last" | "pop", ConcreteType::Array(_)) => {
            Some(NativeKind::Ptr(HeapKind::Option))
        }
        // ── HashMap.get ────────────────────────────────────────────
        // `HashMap<K, V>.get(k) → Option<V>` — the VM-side
        // `hashmap_methods::v2_get` returns
        // `KindedSlot::from_option(Arc<OptionData::some/none>(v))`.
        // Carrier kind is `Ptr(HeapKind::Option)`; the inner V flows
        // through EnumPayload at the destructure site.
        ("get", ConcreteType::HashMap(_, _)) => Some(NativeKind::Ptr(HeapKind::Option)),
        // ── Mutex.get ──────────────────────────────────────────────
        // `Mutex<T>.get() → T` per §2.7.25. The VM-side
        // `executor/objects/mutex_methods::v2_get` clones the inner
        // `KindedSlot::value` payload — the §2.7.5 carrier shape for
        // the inner T is preserved verbatim.
        ("get", ConcreteType::Mutex(inner)) => native_kind_from_concrete_type(inner),
        // ── Atomic.load / fetch_add / fetch_sub / compare_exchange ─
        // `Atomic` is i64-only at landing per §2.7.25; every return
        // path produces a raw i64 (the `AtomicI64::load` / `fetch_*`
        // result). Pre-typed-payload-amendment all four method names
        // surface Int64.
        (
            "load" | "fetch_add" | "fetch_sub" | "compare_exchange",
            ConcreteType::Atomic,
        ) => Some(NativeKind::Int64),
        // ── Lazy.get ───────────────────────────────────────────────
        // `Lazy<T>.get() → T` per §2.7.25. The cached value's
        // `KindedSlot::value` payload is cloned from `LazyInner.value`
        // after first-init; same receiver-recovery shape as Mutex.
        ("get", ConcreteType::Lazy(inner)) => native_kind_from_concrete_type(inner),
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
        // EnumTest emits a native Bool — kind is Bool by construction
        // per the JIT consumer's `jit_arc_result_is_ok` / `_is_some`
        // signature (returns I8 / `NativeKind::Bool`).
        Rvalue::EnumTest { .. } => Some(NativeKind::Bool),
        // EnumPayload extracts the inner payload bits from
        // `Arc<ResultData>` / `Arc<OptionData>`. The payload's kind is
        // classified at the OPERAND's source via 6A's call-return-kind
        // conduit — `concrete_types[base_slot]` holds the
        // `ConcreteType::Result(Ok_inner, Err_inner)` /
        // `ConcreteType::Option(Some_inner)` for a slot bound to a
        // function-call result. The variant tag selects which arm's
        // inner type to project.
        //
        // When the projection succeeds, the inner type maps to a
        // `NativeKind` via `concrete_to_native_kind` (existing helper).
        // When the operand's `concrete_types` entry isn't `Result(_,_)`
        // / `Option(_)` (e.g. opaque source), returning `None` lets
        // bidirectional inference pick up the kind from downstream uses
        // — not a Bool-default fallback per §2.7.7 #9.
        //
        // Producer-site classification chains via:
        //   `Ok(a/b)` emit → EnumStore[r, var:Ok, op:a/b] → r is
        //   Arc<ResultData> → caller's `let r = divide(...)` slot has
        //   `concrete_types[r] = Result(I64, String)` via 6A → in
        //   downstream `match r { Ok(v) => ... }`, the binding's
        //   `EnumPayload { operand: Copy(r), variant: Ok }` reads
        //   `concrete_types[r].ok_arm` = I64 → v's slot kind = Int64.
        Rvalue::EnumPayload { operand, variant } => {
            infer_enum_payload_kind(operand, *variant, concrete_types)
        }
    }
}

/// Project an EnumPayload Rvalue to the destination slot's kind.
/// Reads `concrete_types[operand.root_local()]` and dispatches on the
/// `VariantTag` to select the arm's inner `ConcreteType`, then maps to
/// `NativeKind` via the scalar-kind helper.
///
/// Returns `None` when:
/// - The operand isn't a `Place::Local` projection (e.g. constant or
///   complex projection) — no concrete_types entry exists.
/// - The operand slot's `ConcreteType` isn't `Result(_,_)` / `Option(_)`
///   (e.g. opaque receiver, intermediate temp before the 6A conduit's
///   propagation pass).
/// - The arm's inner `ConcreteType` doesn't map to a scalar
///   `NativeKind` (e.g. nested heap container — the EnumPayload returns
///   the raw inner-Arc bits and the destination slot kind would be a
///   Ptr; the §2.7.5 conduit hasn't yet stamped Ptr arms for inner
///   types but the upcoming 6A propagation does).
/// Project an `EnumPayload` Rvalue to the destination slot's `NativeKind`
/// per ADR-006 §2.7.5 producing-site classification + §2.7.17 receiver-
/// recovery soundness.
///
/// `jit_arc_result_payload` / `jit_arc_option_payload` extract the inner
/// `KindedSlot.slot.raw()` from the typed-Arc carrier. The returned bits
/// preserve the inner's §2.7.5 carrier shape verbatim — for an `Int64`
/// inner the bits are raw native i64; for a `String` inner the bits are
/// `Arc::into_raw(Arc<String>) as u64`; for a `Ptr(HeapKind::*)` inner
/// the bits are the corresponding typed-Arc raw pointer.
///
/// This classifier uses `native_kind_from_concrete_type` (the full
/// ConcreteType → NativeKind mapping) rather than the more restrictive
/// `elem_slot_kind_for_concrete` (which only handles scalar arms for the
/// v2 typed-array fast path) because the inner carrier coming out of
/// `jit_arc_*_payload` IS the §2.7.5-shaped raw bits + kind label for
/// every NativeKind variant. Pre-Round-8A this used the scalar-only
/// classifier, which left `Err(String)` / `Some(typed_object)` payload
/// slots without a kind stamp; the consumer-side print dispatch then
/// surfaced as `kind_hint = None` and routed through the kind-blind
/// `jit_print` fallback — the W-series defection pattern this round
/// closes.
///
/// Returns `None` only when:
/// - operand isn't a `Place::Local` projection — no `concrete_types[idx]`
///   to read (e.g. `Operand::Constant(_)`),
/// - the operand slot's ConcreteType isn't `Result(_,_)` / `Option(_)` —
///   producer-side gap upstream of EnumPayload,
/// - the arm's inner ConcreteType is `Void` (None variant of an Option,
///   or unmatched Err arm of an Ok-only Result) — no payload exists.
fn infer_enum_payload_kind(
    operand: &Operand,
    variant: VariantTag,
    concrete_types: Option<&[ConcreteType]>,
) -> Option<NativeKind> {
    let concrete_types = concrete_types?;
    let place = match operand {
        Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => p,
        Operand::Constant(_) => return None,
    };
    let base_slot = place.root_local();
    let ct = concrete_types.get(base_slot.0 as usize)?;
    let inner: &ConcreteType = match (variant, ct) {
        (VariantTag::Ok, ConcreteType::Result(ok, _)) => ok.as_ref(),
        (VariantTag::Err, ConcreteType::Result(_, err)) => err.as_ref(),
        (VariantTag::Some_, ConcreteType::Option(inner)) => inner.as_ref(),
        // None has no payload — kind isn't meaningful.
        _ => return None,
    };
    native_kind_from_concrete_type(inner)
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
            local_struct_type_names: Default::default(),
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

    // ── Phase 3 cluster-0 Round 11-trinity Part b (2026-05-13) ──────────
    // Tests for `parametric_method_return_kind_from_receiver`. Verifies
    // the receiver+method-name pair classification against
    // ConcreteType-bearing receivers.

    use shape_value::heap_value::HeapKind;
    use shape_value::v2::ConcreteType;

    fn copy_local(slot: u16) -> Operand {
        Operand::Copy(Place::Local(SlotId(slot)))
    }

    #[test]
    fn parametric_array_sum_returns_element_kind() {
        // `Array<int>.sum() → Int64`
        let cts = vec![
            ConcreteType::Array(Box::new(ConcreteType::I64)),
        ];
        let kind = parametric_method_return_kind_from_receiver("sum", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Int64));

        // `Array<number>.sum() → Float64`
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::F64))];
        let kind = parametric_method_return_kind_from_receiver("sum", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Float64));
    }

    #[test]
    fn parametric_array_mean_and_min_max_inherit_element() {
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::F64))];
        assert_eq!(
            parametric_method_return_kind_from_receiver("mean", &[copy_local(0)], &cts),
            Some(NativeKind::Float64)
        );
        assert_eq!(
            parametric_method_return_kind_from_receiver("min", &[copy_local(0)], &cts),
            Some(NativeKind::Float64)
        );
        assert_eq!(
            parametric_method_return_kind_from_receiver("max", &[copy_local(0)], &cts),
            Some(NativeKind::Float64)
        );
    }

    #[test]
    fn parametric_array_first_last_pop_return_option_carrier() {
        // Array.first/last/pop wrap in Option<T> — destination slot
        // carries Ptr(HeapKind::Option) per §2.7.17.
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::I64))];
        assert_eq!(
            parametric_method_return_kind_from_receiver("first", &[copy_local(0)], &cts),
            Some(NativeKind::Ptr(HeapKind::Option))
        );
        assert_eq!(
            parametric_method_return_kind_from_receiver("last", &[copy_local(0)], &cts),
            Some(NativeKind::Ptr(HeapKind::Option))
        );
        assert_eq!(
            parametric_method_return_kind_from_receiver("pop", &[copy_local(0)], &cts),
            Some(NativeKind::Ptr(HeapKind::Option))
        );
    }

    #[test]
    fn parametric_hashmap_get_returns_option_carrier() {
        // HashMap.get(k) → Option<V>; destination slot carries
        // Ptr(HeapKind::Option) per §2.7.17. The wrapped V flows
        // through EnumPayload at the destructure site.
        let cts = vec![ConcreteType::HashMap(
            Box::new(ConcreteType::String),
            Box::new(ConcreteType::I64),
        )];
        let kind =
            parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::Option)));
    }

    #[test]
    fn parametric_mutex_get_returns_inner_kind() {
        // Mutex<int>.get() → Int64 per §2.7.25 receiver-recovery.
        let cts = vec![ConcreteType::Mutex(Box::new(ConcreteType::I64))];
        let kind =
            parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Int64));

        // Mutex<bool>.get() → Bool.
        let cts = vec![ConcreteType::Mutex(Box::new(ConcreteType::Bool))];
        let kind =
            parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Bool));
    }

    #[test]
    fn parametric_atomic_load_fetch_returns_int64() {
        // Atomic is i64-only at landing per §2.7.25.
        let cts = vec![ConcreteType::Atomic];
        for name in &["load", "fetch_add", "fetch_sub", "compare_exchange"] {
            let kind =
                parametric_method_return_kind_from_receiver(name, &[copy_local(0)], &cts);
            assert_eq!(
                kind,
                Some(NativeKind::Int64),
                "Atomic.{name} should return Int64"
            );
        }
    }

    #[test]
    fn parametric_lazy_get_returns_inner_kind() {
        // Lazy<int>.get() → Int64 per §2.7.25 receiver-recovery.
        let cts = vec![ConcreteType::Lazy(Box::new(ConcreteType::I64))];
        let kind =
            parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Int64));
    }

    #[test]
    fn parametric_unknown_method_returns_none() {
        // Unknown method names produce None — no Bool-default fallback
        // per §2.7.7 #9.
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::I64))];
        let kind = parametric_method_return_kind_from_receiver(
            "unknown_method",
            &[copy_local(0)],
            &cts,
        );
        assert_eq!(kind, None);
    }

    #[test]
    fn parametric_constant_receiver_returns_none() {
        // A constant-operand receiver has no slot to source ConcreteType
        // from — classification is impossible, return None.
        let kind = parametric_method_return_kind_from_receiver(
            "sum",
            &[Operand::Constant(MirConstant::Int(42))],
            &[],
        );
        assert_eq!(kind, None);
    }

    #[test]
    fn parametric_void_receiver_returns_none() {
        // When the receiver slot's ConcreteType is Void (the upstream
        // conduit couldn't prove a kind), classification falls through
        // to None — no fabricated default.
        let cts = vec![ConcreteType::Void];
        let kind =
            parametric_method_return_kind_from_receiver("sum", &[copy_local(0)], &cts);
        assert_eq!(kind, None);
    }

    #[test]
    fn parametric_size_is_invariant_not_parametric() {
        // `size` is in `well_known_method_return_kind` (invariant
        // across receivers); the parametric classifier should NOT
        // catch it. This pins the cohort split — invariant names land
        // in the well_known path, parametric names in the parametric
        // path. No overlap.
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::I64))];
        let kind =
            parametric_method_return_kind_from_receiver("size", &[copy_local(0)], &cts);
        assert_eq!(
            kind, None,
            "size belongs to well_known_method_return_kind, not the parametric cohort"
        );
        // But well_known catches it.
        assert_eq!(
            well_known_method_return_kind("size"),
            Some(NativeKind::Int64)
        );
    }

    #[test]
    fn parametric_method_return_kind_integrates_in_call_terminator_seed() {
        // Integration test: a Call terminator for `arr.sum()` on an
        // Array<int> receiver seeds the destination slot's kind to
        // Int64 via the parametric classifier. Mirrors the
        // Round 5C TerminatorKind::Call destination-stamp path; the
        // parametric extension reaches it via the
        // `well_known.or_else(parametric)` chain at the Call-terminator
        // pass.
        //
        // MIR shape:
        //   local 0 = Array<int> receiver (concrete_types seeded)
        //   call .sum(local 0) → local 1
        let mir = MirFunction {
            name: "test_sum".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![],
                terminator: Terminator {
                    kind: TerminatorKind::Call {
                        func: Operand::Constant(MirConstant::Method("sum".to_string())),
                        args: vec![copy_local(0)],
                        destination: Place::Local(SlotId(1)),
                        next: BasicBlockId(0),
                    },
                    span: shape_ast::Span::default(),
                },
            }],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: shape_ast::Span::default(),
            field_name_table: Default::default(),
            local_struct_type_names: Default::default(),
        };
        let concrete_types = vec![
            ConcreteType::Array(Box::new(ConcreteType::I64)),
            ConcreteType::Void,
            ConcreteType::Void,
            ConcreteType::Void,
        ];
        let kinds = infer_slot_kinds_with_concrete(&mir, &[], &concrete_types);
        assert_eq!(
            kinds[1],
            Some(NativeKind::Int64),
            ".sum() on Array<int> should stamp Int64 on the destination slot"
        );
    }

    // ── Phase 3 cluster-0 Round 12 T1 surface pin tests ────────────────
    //
    // Surface pins for the user-defined-trait method dispatch boundary
    // documented at `parametric_method_return_kind_from_receiver`'s
    // "User-defined-trait surface boundary" doc block. These tests
    // assert the JIT-internal classifier's posture — they are
    // intentional pins, not regressions to be papered over by a
    // Bool-default fallback or a hard-coded method-name arm.
    //
    // ── Round 13 T1' status (2026-05-13) ────────────────────────────
    //
    // The user-defined-trait method dispatch boundary closes at the
    // **VM-side conduit producer**, not at the JIT-internal
    // parametric classifier. The producer
    // (`crates/shape-vm/src/compiler/helpers.rs::infer_top_level_concrete_types_from_mir_with_resolvers`)
    // stamps the Call-terminator destination slot's ConcreteType from
    // the trait's declared return type via the new method-returns
    // resolver chain:
    //
    //   `mir.local_struct_type_names[receiver_slot]` (gap 1 closure)
    //   → `find_default_trait_impl_for_type_method(type_name, method)`
    //   → `function_return_concrete_types[fn_idx]` (post gap 3 backfill)
    //
    // So Smoke 3 (`trait T { name(): string } type X {} impl T for X {
    // method name() { "x" } } let t = X {} print(t.name())` → `x`)
    // closes via the upstream `concrete_types[t_name_slot]
    // = ConcreteType::String` stamp; the JIT consumer at
    // `infer_slot_kinds_with_concrete` projects this through
    // `concrete_seed` (`crates/shape-jit/src/mir_compiler/mod.rs:564`)
    // to `NativeKind::String` automatically — no change to the
    // JIT-internal `parametric_method_return_kind_from_receiver`
    // classifier needed.
    //
    // The 3 pin tests below remain valid post-T1': they assert that the
    // JIT-internal classifier is NOT the place where user-defined trait
    // method classification happens (it would be a wrong-layer
    // classification per CLAUDE.md "Renames to refuse on sight" / Round
    // 6A precedent). The classification correctly lives at the VM-side
    // conduit producer one tier upstream.
    //
    // The new positive pin
    // (`trait_method_call_destination_seeded_from_concrete_types`)
    // asserts the upstream-landing pathway: when the VM-side conduit
    // has stamped `concrete_types[result_slot] = ConcreteType::String`,
    // the JIT consumer's `concrete_seed` projection picks it up to
    // `NativeKind::String`.

    #[test]
    fn user_defined_trait_method_on_struct_returns_none() {
        // Smoke 3 minimal case at the classifier level: receiver
        // `t: X` carries `ConcreteType::Struct(StructLayoutId(0))`
        // because `concrete_type_from_annotation` returns the
        // `StructLayoutId(0)` placeholder for every user struct name
        // (the layout-id registry is not wired — see the function's
        // `_ => None` arm at `v2_map_emission.rs:378` "Phase 1.1
        // Agent 3 will fill this in"). The classifier has no
        // struct-name information to disambiguate `X` from any other
        // user struct, and the trait registry is not threaded into
        // the JIT MIR builder layer — so the trait method's declared
        // return type (`string` from `trait T { name(): string }`) is
        // unreachable from this classifier.
        //
        // The classifier must return `None` (surface-and-stop posture),
        // NOT a fabricated `NativeKind::String` from hard-coding `"name"`
        // — that would be a CLAUDE.md "Forbidden rationalizations"
        // walk-back ("hard-code the kickoff Smoke 3 case for now").
        let cts = vec![ConcreteType::Struct(
            shape_value::v2::concrete_type::StructLayoutId(0),
        )];
        let kind =
            parametric_method_return_kind_from_receiver("name", &[copy_local(0)], &cts);
        assert_eq!(
            kind, None,
            "User-defined trait method on Struct receiver must surface \
             (return None); the trait registry's declared return type is \
             not threaded into the JIT MIR builder. See classifier doc \
             block 'User-defined-trait surface boundary'."
        );
    }

    #[test]
    fn user_defined_trait_method_call_terminator_remains_unstamped() {
        // Integration pin: the Call-terminator destination-stamp pass
        // at `infer_slot_kinds_with_concrete` chains
        // `well_known.or_else(parametric)`. Neither classifier catches
        // `name` on a `Struct(_)` receiver:
        //
        // - `well_known_method_return_kind("name")` returns `None` —
        //   `"name"` is not a collection-size / emptiness invariant.
        // - `parametric_method_return_kind_from_receiver("name",
        //   args, [Struct(0)])` returns `None` per the pin above.
        //
        // Result: the destination slot's kind remains `None` at JIT
        // MIR time, the downstream `print(t.name())` Call-terminator
        // surfaces at the print-operand-kind-None Route A
        // surface-and-stop. This is the load-bearing Smoke 3 surface
        // shape Round 12 T1 surfaces for cross-crate conduit
        // extension.
        let mir = MirFunction {
            name: "test_trait_dispatch".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![],
                terminator: Terminator {
                    kind: TerminatorKind::Call {
                        func: Operand::Constant(MirConstant::Method(
                            "name".to_string(),
                        )),
                        args: vec![copy_local(0)],
                        destination: Place::Local(SlotId(1)),
                        next: BasicBlockId(0),
                    },
                    span: shape_ast::Span::default(),
                },
            }],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: shape_ast::Span::default(),
            field_name_table: Default::default(),
            local_struct_type_names: Default::default(),
        };
        let concrete_types = vec![
            ConcreteType::Struct(shape_value::v2::concrete_type::StructLayoutId(0)),
            ConcreteType::Void,
            ConcreteType::Void,
            ConcreteType::Void,
        ];
        let kinds = infer_slot_kinds_with_concrete(&mir, &[], &concrete_types);
        assert_eq!(
            kinds[1],
            None,
            "Call-terminator destination for `t.name()` on a Struct(_) \
             receiver must remain unstamped — the trait-dispatch return \
             kind cannot be classified without a cross-crate conduit \
             extension. See classifier doc block 'User-defined-trait \
             surface boundary'."
        );
        // Pin the well_known cohort: `"name"` is NOT a well-known
        // invariant method name; without the parametric arm catching
        // it (which it cannot, per the pin above), there is no
        // classification path.
        assert_eq!(
            well_known_method_return_kind("name"),
            None,
            "`name` must not be a well-known method name — that would \
             be a soundness violation (different traits could declare \
             `name` with different return types, e.g. `trait T \
             {{ name(): string }}` vs `trait U {{ name(): int }}`)."
        );
    }

    #[test]
    fn parametric_classifier_remains_silent_for_struct_receiver_with_known_method_names() {
        // Cohort pin: the parametric arms for `get` / `sum` / `mean` /
        // `min` / `max` / `first` / `last` / `pop` / `load` / `fetch_*`
        // / `compare_exchange` are all keyed on receiver `ConcreteType`
        // matching `Array(_)` / `HashMap(_,_)` / `Mutex(_)` / `Atomic`
        // / `Lazy(_)`. A `Struct(_)` receiver must NOT accidentally
        // fall through to any of these arms — that would be a wrong-
        // carrier classification (a user struct with a `.sum()` method
        // is not an `Array<T>`).
        let cts = vec![ConcreteType::Struct(
            shape_value::v2::concrete_type::StructLayoutId(0),
        )];
        for method_name in [
            "get",
            "sum",
            "mean",
            "min",
            "max",
            "first",
            "last",
            "pop",
            "load",
            "fetch_add",
            "fetch_sub",
            "compare_exchange",
            // Trait-dispatch-shaped names that could exist on user
            // structs but are NOT well-known or parametric arms:
            "name",
            "display",
            "to_string",
            "into",
            "from",
            "try_into",
            "try_from",
        ] {
            let kind = parametric_method_return_kind_from_receiver(
                method_name,
                &[copy_local(0)],
                &cts,
            );
            assert_eq!(
                kind, None,
                "method `{method_name}` on Struct(_) receiver must \
                 not be classified by the parametric cohort"
            );
        }
    }

    // ── Phase 3 cluster-0 Round 13 T1' positive pin (2026-05-13) ────────
    //
    // The companion of the 3 surface pins above. Asserts the
    // upstream-landing pathway works: when the VM-side conduit
    // producer
    // (`crates/shape-vm/src/compiler/helpers.rs::infer_top_level_concrete_types_from_mir_with_resolvers`)
    // has stamped `concrete_types[result_slot] = ConcreteType::String`
    // via the method-returns resolver chain (`mir.local_struct_type_names`
    // → `find_default_trait_impl_for_type_method` →
    // `function_return_concrete_types`), the JIT consumer's
    // `concrete_seed` projection
    // (`crates/shape-jit/src/mir_compiler/mod.rs:564`) picks it up to
    // `NativeKind::String` and `infer_slot_kinds_with_concrete`
    // preserves that kind through its existing-seed pass.

    #[test]
    fn trait_method_call_destination_seeded_from_concrete_types() {
        // Simulates the post-T1' compilation state: the VM-side
        // conduit producer has stamped the Call destination slot's
        // ConcreteType to the trait's declared return type
        // (`ConcreteType::String` for Smoke 3's `t.name()` where
        // `trait T { name(): string }`). The caller threads this
        // through `concrete_seed` so `existing[result_slot] =
        // Some(NativeKind::String)` when `infer_slot_kinds_with_concrete`
        // is invoked.
        //
        // Verifies: the existing-seed pass preserves the upstream
        // stamp — the Call-terminator pass at lines ~306-359 only
        // sets `kinds[idx]` if `kinds[idx].is_none()` (the
        // `idx < n && kinds[idx].is_none()` guard at line 316), so
        // the upstream `Some(NativeKind::String)` flows through
        // untouched.
        let mir = MirFunction {
            name: "test_trait_dispatch_post_t1prime".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![],
                terminator: Terminator {
                    kind: TerminatorKind::Call {
                        func: Operand::Constant(MirConstant::Method(
                            "name".to_string(),
                        )),
                        args: vec![copy_local(0)],
                        destination: Place::Local(SlotId(1)),
                        next: BasicBlockId(0),
                    },
                    span: shape_ast::Span::default(),
                },
            }],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: shape_ast::Span::default(),
            field_name_table: Default::default(),
            local_struct_type_names: Default::default(),
        };
        // Simulate post-T1' upstream state: `concrete_types[1]` is
        // stamped String by the VM-side conduit; the caller has
        // projected it through `native_kind_from_concrete_type` to
        // form `existing[1] = Some(NativeKind::String)`.
        let concrete_types = vec![
            ConcreteType::Struct(shape_value::v2::concrete_type::StructLayoutId(0)),
            ConcreteType::String,
            ConcreteType::Void,
            ConcreteType::Void,
        ];
        let existing = vec![
            None,
            Some(NativeKind::String),
            None,
            None,
        ];
        let kinds = infer_slot_kinds_with_concrete(&mir, &existing, &concrete_types);
        assert_eq!(
            kinds[1],
            Some(NativeKind::String),
            "Post-T1' upstream-seeded Call-terminator destination slot \
             must preserve the trait-method declared return kind through \
             the JIT consumer's existing-seed pass — no clobber by the \
             classifier fallthrough"
        );
    }
}
