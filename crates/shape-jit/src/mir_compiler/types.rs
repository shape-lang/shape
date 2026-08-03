//! Type mapping for MIR-to-Cranelift IR compilation.
//!
//! Maps MIR LocalTypeInfo and NativeKind to Cranelift types.
//! Includes MIR-level type inference for determining slot kinds
//! when the bytecode compiler doesn't provide them.

use cranelift::prelude::types;
use shape_value::heap_value::HeapKind;
use shape_value::v2::{ConcreteType, closure_layout};
use shape_vm::mir::types::*;
use shape_vm::type_tracking::NativeKind;
use std::collections::HashMap;

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
/// U4-7: the JIT does not own a second `ConcreteType -> NativeKind` map.
/// It calls the VM/value-layer projection and only wraps the no-slot
/// `ConcreteType::Void` case as `None` for MIR metadata.
pub(crate) fn native_kind_from_concrete_type(ct: &ConcreteType) -> Option<NativeKind> {
    if matches!(ct, ConcreteType::Void) {
        return None;
    }
    Some(closure_layout::native_kind_from_concrete_type(ct))
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
/// ADR-020 §3.3 — the slots that carry UNIT, i.e. no value at all.
///
/// "Unit — no value. Unit calls are void (zero-return signatures). No
/// `TAG_UNIT`." MIR still names a destination slot for a void call, but
/// nothing is produced for it: such a slot gets no Cranelift variable, no
/// storage kind and no write, and a MIR `Assign` that moves it into another
/// slot propagates unit-ness rather than a value.
///
/// This is the kind-source that `infer_slot_kinds` structurally cannot
/// supply, because there is no kind to supply — asking it for one is the
/// question ADR-020 §6 forbids answering with a sentinel word. Before this
/// existed, `terminators.rs`'s `print` lowering stored an `iconst 0` into
/// the destination and `write_place` refused it (#257), which whole-program
/// bailed every top-level program containing a `print` — the dominant term
/// in the 11/488 native-execution rate measured on 2026-08-03.
///
/// Two static proof sources, no name allowlist beyond the one the emit site
/// already tests:
///  - a `Call` whose callee resolves to a function index in
///    `unit_returning_funcs`, which `harvest_return_abi` fills from
///    `FrameDescriptor::returns_no_value()`;
///  - the `print` builtin — the same `name == "print"` with no user
///    function of that name that its own lowering branches on. Mirroring
///    that one site is not a growing allowlist; if `print` ever stops
///    being special-cased in codegen this predicate goes with it.
///
/// A callee whose return kind is merely *unproven* is NOT unit. That case
/// still surfaces at `write_place`, which is the whole point of #257.
pub(crate) fn infer_unit_slots(
    mir: &MirFunction,
    function_indices: &HashMap<String, u16>,
    unit_returning_funcs: &std::collections::HashSet<u16>,
) -> std::collections::HashSet<SlotId> {
    let mut unit: std::collections::HashSet<SlotId> = std::collections::HashSet::new();

    for block in &mir.blocks {
        if let TerminatorKind::Call {
            func, destination, ..
        } = &block.terminator.kind
        {
            let Place::Local(slot) = destination else {
                continue;
            };
            let produces_no_value = match func {
                Operand::Constant(MirConstant::Function(name)) => {
                    match resolve_named_function_index(name, function_indices) {
                        Some(fid) => unit_returning_funcs.contains(&fid),
                        None => name == "print",
                    }
                }
                _ => false,
            };
            if produces_no_value {
                unit.insert(*slot);
            }
        }
    }

    // `Assign(dst, Use(Move|Copy(Local(src))))` where `src` is unit makes
    // `dst` unit too — top-level MIR ends with exactly this shape, moving
    // the trailing statement's result into the return slot. Iterate to a
    // fixpoint so a chain of such moves propagates.
    loop {
        let mut changed = false;
        for block in &mir.blocks {
            for stmt in &block.statements {
                let StatementKind::Assign(Place::Local(dst), Rvalue::Use(operand)) = &stmt.kind
                else {
                    continue;
                };
                let src = match operand {
                    Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => p,
                    Operand::Constant(_) => continue,
                };
                if let Place::Local(src_slot) = src {
                    if unit.contains(src_slot) && unit.insert(*dst) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    unit
}

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
    infer_slot_kinds_with_concrete_and_function_returns(mir, existing, concrete_types, None, None)
}

pub(crate) fn infer_slot_kinds_with_concrete_and_function_returns(
    mir: &MirFunction,
    existing: &[Option<NativeKind>],
    concrete_types: &[ConcreteType],
    function_indices: Option<&HashMap<String, u16>>,
    function_return_kinds: Option<&HashMap<u16, NativeKind>>,
) -> Vec<Option<NativeKind>> {
    infer_slot_kinds_with_concrete_function_returns_and_field_kinds(
        mir,
        existing,
        concrete_types,
        function_indices,
        function_return_kinds,
        None,
    )
}

pub(crate) fn infer_slot_kinds_with_concrete_function_returns_and_field_kinds(
    mir: &MirFunction,
    existing: &[Option<NativeKind>],
    concrete_types: &[ConcreteType],
    function_indices: Option<&HashMap<String, u16>>,
    function_return_kinds: Option<&HashMap<u16, NativeKind>>,
    schema_field_kinds: Option<&HashMap<String, NativeKind>>,
) -> Vec<Option<NativeKind>> {
    let n = mir.num_locals as usize;
    let mut kinds: Vec<Option<NativeKind>> = vec![None; n];
    let local_struct_type_names = propagated_local_struct_type_names(mir);

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
                            //
                            // The receiver-in-pass-kinds fallback
                            // (`method_return_kind_from_in_pass_kinds`) is
                            // NOT consulted here because at this point in
                            // the pass the EnumStore-driven kind seeds (line
                            // ~582) have not been applied — bare-form
                            // collection ctors like `HashMap()` are still
                            // unclassified. The chain-temp fallback runs in
                            // a SECOND fixpoint-iterated call-stamp pass
                            // BELOW (line ~692, after the forward pass) per
                            // Phase 4b Round 5c-2-α HashMap-has-2-chain
                            // ratify 2026-05-19.
                            well_known_method_return_kind(name)
                                .or_else(|| {
                                    parametric_method_return_kind_from_receiver(
                                        name,
                                        args,
                                        concrete_types,
                                    )
                                })
                                .or_else(|| {
                                    user_method_return_kind_from_receiver(
                                        name,
                                        args,
                                        concrete_types,
                                        &local_struct_type_names,
                                        function_indices,
                                        function_return_kinds,
                                    )
                                })
                                // Wave 1b SEAM B (2026-06-15): iterator lazy
                                // adapters (`map`/`filter`/`take`/...) return a
                                // new `Ptr(HeapKind::Iterator)` when applied to
                                // an Iterator receiver. The receiver's kind is
                                // read from the in-progress `kinds` track (the
                                // `iter()` result was stamped Iterator just
                                // above; the forward pass + the fixpoint loop
                                // below propagate chained adapters). Keeps a
                                // chained terminal's receiver classified as
                                // Iterator → VM-trampoline delegation, never
                                // the legacy `UInt64` garbage path.
                                .or_else(|| iterator_adapter_return_kind(name, args, &kinds))
                        }
                        Operand::Constant(MirConstant::Function(name)) => {
                            named_function_return_kind(
                                name,
                                function_indices,
                                function_return_kinds,
                            )
                            .or_else(|| well_known_function_return_kind(name))
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
    let field_kinds_for_projection: std::collections::HashMap<String, NativeKind> = {
        let mut merged = schema_field_kinds.cloned().unwrap_or_default();
        // Local ObjectStore facts are more specific than the registry-wide
        // schema pre-population and preserve the existing name-collision
        // precedence.
        merged.extend(
            field_kinds_pre
                .iter()
                .map(|(name, kind)| (name.clone(), *kind)),
        );
        merged
    };
    let field_kinds_for_projection = if field_kinds_for_projection.is_empty() {
        None
    } else {
        Some(&field_kinds_for_projection)
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
                                field_kinds_for_projection,
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
                                field_kinds_for_projection,
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
                        "PriorityQueue" => Some(NativeKind::Ptr(HeapKind::PriorityQueue)),
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
    // Phase 4b Round 5c-2-α HashMap-has-2-chain VM/JIT divergence fix
    // (v0.3-gating SOUNDNESS BUG ratified 2026-05-19). Unified fixpoint
    // pass covering both (a) the existing collection-alias propagation
    // (`let t = s` re-bindings through `Assign(_, Use(Copy/Move))`) and
    // (b) the NEW chain-temp call-terminator stamping for HashMap
    // mutators (`set` / `delete` / `merge`) returning self.
    //
    // Both stamping rules need to be at the same fixpoint level because
    // chain-temp patterns interleave: `HashMap().set(...).set(...)` has
    // each `.set` call's destination temp aliased into a downstream
    // `let m = ...; m.set(...)` rebind which depends on the alias
    // propagation, AND each `.set` call's destination kind depends on
    // its receiver's in-pass `kinds[]` which is the alias-propagated
    // value from the previous chain link.
    //
    // Empirical pre-fix at HEAD 7eb82205: `HashMap().set("a",1).has("a")`
    // already diverged (VM=true / JIT=false) at the 1-chain length
    // because:
    //   1. The EnumStore arm stamps temp_HashMap_ctor's slot as
    //      `Ptr(HeapKind::HashMap)`.
    //   2. The call `temp_set = temp_HashMap_ctor.set("a", 1)` has no
    //      kind classifier entry for `set`, so temp_set stayed None.
    //   3. The MIR copy `m = temp_set` then needs collection-alias-
    //      propagation to inherit temp_set's kind — but temp_set has
    //      no kind, so `m` also stays None.
    //   4. The next call `temp_has = m.has("a")` pushes `m` as receiver
    //      with kind None → fallback to `UInt64` carrier at the
    //      `operand_slot_kind_or_carrier` site →
    //      `jit_call_method` shell's delegation predicate routes to
    //      legacy JIT-format dispatch which doesn't recognize
    //      `Arc::into_raw(Arc<HashMapKindedRef>)` bits → TAG_NULL.
    //   5. The destination kind for `temp_has` is correctly Bool (from
    //      `well_known_method_return_kind("has")`), so TAG_NULL bits
    //      under a Bool consumer renders `false`.
    //
    // Post-fix: the unified fixpoint propagates `Ptr(HeapKind::HashMap)`
    // through each chain link, routing the final `.has()` call's
    // receiver through `jit_trampoline_call_method` per §2.7.10 / Q11
    // dispatch to the VM's `HASHMAP_METHODS.get("has") => v2_has`
    // handler.
    //
    // Per ADR-006 §2.7.5 producer-side stamp. The HashMap-mutator
    // classifier reads the VM-side handler signatures:
    // `hashmap_methods::v2_set` at line 789 returns
    // `KindedSlot::from_hashmap(...)`; `v2_delete` at 1235 and
    // `v2_merge` at 1469 do likewise. VM producer-stamp and JIT
    // consumer-classify converge on the same `Ptr(HeapKind::HashMap)`
    // kind without any cross-layer translation step.
    let mut changed = true;
    let mut iterations = 0;
    let max_iterations = n + 4; // safety bound
    while changed && iterations < max_iterations {
        changed = false;
        iterations += 1;
        for block in &mir.blocks {
            // (a) Collection-alias propagation through `let t = s` /
            // `let u = t` re-bindings — preserves the existing
            // behavior (was a standalone loop pre-Round-5c-2-α).
            for stmt in &block.statements {
                if let StatementKind::Assign(Place::Local(dst), Rvalue::Use(operand)) = &stmt.kind {
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
                                if is_collection_kind(src_kind) && kinds[dst_idx] != Some(src_kind)
                                {
                                    kinds[dst_idx] = Some(src_kind);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
            // (b) Chain-temp call-terminator stamping for HashMap
            // mutators — NEW in Round 5c-2-α. Consults the in-pass
            // `kinds[]` for the receiver's NativeKind via
            // `method_return_kind_from_in_pass_kinds`, which the first
            // call-stamp pass at line ~331 doesn't have access to
            // (`concrete_types` doesn't carry the chain-temp shape).
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
                                method_return_kind_from_in_pass_kinds(name, args, &kinds).or_else(
                                    || {
                                        user_method_return_kind_from_receiver(
                                            name,
                                            args,
                                            concrete_types,
                                            &local_struct_type_names,
                                            function_indices,
                                            function_return_kinds,
                                        )
                                    },
                                )
                            }
                            Operand::Constant(MirConstant::Function(name)) => {
                                named_function_return_kind(
                                    name,
                                    function_indices,
                                    function_return_kinds,
                                )
                            }
                            _ => None,
                        };
                        if let Some(k) = ret_kind {
                            kinds[idx] = Some(k);
                            changed = true;
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

fn propagated_local_struct_type_names(mir: &MirFunction) -> HashMap<SlotId, String> {
    let mut names = mir.local_struct_type_names.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for block in &mir.blocks {
            for stmt in &block.statements {
                let StatementKind::Assign(
                    Place::Local(dst),
                    Rvalue::Use(
                        Operand::Copy(Place::Local(src))
                        | Operand::Move(Place::Local(src))
                        | Operand::MoveExplicit(Place::Local(src)),
                    ),
                ) = &stmt.kind
                else {
                    continue;
                };
                match (names.get(dst).cloned(), names.get(src).cloned()) {
                    (Some(name), None) => {
                        names.insert(*src, name);
                        changed = true;
                    }
                    (None, Some(name)) => {
                        names.insert(*dst, name);
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
    }
    names
}

fn named_function_return_kind(
    name: &str,
    function_indices: Option<&HashMap<String, u16>>,
    function_return_kinds: Option<&HashMap<u16, NativeKind>>,
) -> Option<NativeKind> {
    let function_indices = function_indices?;
    let function_return_kinds = function_return_kinds?;
    let idx = resolve_named_function_index(name, function_indices)?;
    function_return_kinds.get(&idx).copied()
}

/// Resolve a user-defined method call's return kind from the receiver's
/// concrete struct name and the JIT-visible function return table.
///
/// Extend-generated methods use `Type.method`; impl methods use
/// `Type::method`. The runtime `jit_call_method` user-method fallback checks
/// both forms before invoking a JIT-compiled callee; this compile-time helper
/// mirrors that lookup only to stamp the call destination kind. The return
/// kind itself is sourced from the existing function-return conduit.
fn user_method_return_kind_from_receiver(
    method_name: &str,
    args: &[Operand],
    concrete_types: &[ConcreteType],
    local_struct_type_names: &HashMap<SlotId, String>,
    function_indices: Option<&HashMap<String, u16>>,
    function_return_kinds: Option<&HashMap<u16, NativeKind>>,
) -> Option<NativeKind> {
    let function_indices = function_indices?;
    let function_return_kinds = function_return_kinds?;
    let receiver = args.first()?;
    let receiver_slot = match receiver {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            place.root_local()
        }
        Operand::Constant(_) => return None,
    };
    let type_name = match concrete_types.get(receiver_slot.0 as usize) {
        Some(ConcreteType::Struct(named)) => named.name_str().or_else(|| {
            local_struct_type_names
                .get(&receiver_slot)
                .map(String::as_str)
        })?,
        _ => local_struct_type_names
            .get(&receiver_slot)
            .map(String::as_str)?,
    };
    let candidates = [
        format!("{}::{}", type_name, method_name),
        format!("{}.{}", type_name, method_name),
    ];
    candidates.iter().find_map(|candidate| {
        function_indices
            .get(candidate)
            .and_then(|idx| function_return_kinds.get(idx).copied())
    })
}

fn resolve_named_function_index(
    name: &str,
    function_indices: &HashMap<String, u16>,
) -> Option<u16> {
    if let Some(idx) = function_indices.get(name).copied() {
        return Some(idx);
    }
    if name.contains("::") {
        return None;
    }
    let suffix = format!("::{}", name);
    let mut found = None;
    for (full_name, idx) in function_indices {
        if full_name.ends_with(&suffix) {
            if found.is_some() {
                return None;
            }
            found = Some(*idx);
        }
    }
    found
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
/// Wave 1b SEAM B (2026-06-15): classify the return kind of an iterator
/// LAZY ADAPTER (`map` / `filter` / `take` / `skip` / `flatMap` /
/// `enumerate` / `chain`) — each returns a new `Ptr(HeapKind::Iterator)`,
/// but ONLY when the receiver is itself an iterator (the same names are
/// Array methods with non-Iterator returns). The receiver's kind is read
/// from the in-progress `kinds` slot-kind track (the `iter()` factory is
/// stamped `Ptr(HeapKind::Iterator)` by `well_known_method_return_kind`,
/// and chained adapters propagate forward). Returns `None` for non-adapter
/// names or non-Iterator receivers, so the caller falls through unchanged.
///
/// This is what keeps a chained `iter().filter(..).count()` sound: without
/// it, the `filter` result slot stays `None` → defaults to the `UInt64`
/// opaque-JIT carrier → the `.count()` receiver is then mis-classified
/// `UInt64` and routed into the legacy JIT-format dispatch (no Iterator
/// registry) which reads a garbage placeholder. Stamping the adapter
/// result `Ptr(HeapKind::Iterator)` makes the terminal receiver delegate
/// to the VM trampoline's authoritative iterator handlers (VM == JIT).
fn iterator_adapter_return_kind(
    name: &str,
    args: &[Operand],
    kinds: &[Option<NativeKind>],
) -> Option<NativeKind> {
    use shape_value::heap_value::HeapKind;
    match name {
        "map" | "filter" | "take" | "skip" | "flatMap" | "enumerate" | "chain" => {
            let receiver = args.first()?;
            let receiver_slot = match receiver {
                Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => p.root_local(),
                Operand::Constant(_) => return None,
            };
            match kinds.get(receiver_slot.0 as usize).copied().flatten() {
                Some(NativeKind::Ptr(HeapKind::Iterator)) => {
                    Some(NativeKind::Ptr(HeapKind::Iterator))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

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
        // Wave 1b SEAM B (2026-06-15): `iter()` produces a lazy iterator on
        // EVERY receiver (Array / String / Range / HashMap) — the runtime
        // handlers (`handle_array_iter` / `handle_string_iter` /
        // `range_iter` / `handle_hashmap_iter`) all return
        // `KindedSlot::from_iterator(Arc<IteratorState>)`, i.e.
        // `Ptr(HeapKind::Iterator)`. This is receiver-INVARIANT, so it
        // belongs here rather than in the parametric cohort. Stamping the
        // kind is load-bearing: WITHOUT it the `iter()` result slot stays
        // `None` → defaults to the `UInt64` opaque-JIT carrier, and the
        // downstream `.count()` / `.collect()` / ... receiver is then
        // classified `UInt64` in `jit_call_method`, falling into the
        // legacy JIT-format dispatch (no Iterator registry) which read a
        // garbage placeholder (`-1407374883553280`) where the bytecode VM
        // returns the correct value. With the kind stamped as
        // `Ptr(HeapKind::Iterator)`, the terminal-method receiver is
        // classified correctly and delegates to the VM trampoline's
        // authoritative iterator handlers (VM == JIT).
        "iter" => Some(NativeKind::Ptr(shape_value::heap_value::HeapKind::Iterator)),
        // Iterator lazy adapters (`map` / `filter` / `take` / `skip` /
        // `flatMap` / `enumerate` / `chain`) ALSO return a new
        // `Ptr(HeapKind::Iterator)` — but these names are NOT receiver-
        // invariant (`Array.map` returns an Array, etc.), so they cannot
        // be stamped here. They are classified receiver-parametrically in
        // `iterator_adapter_return_kind` (called from the call-terminator
        // stamping loop) when the receiver slot's already-inferred kind is
        // `Ptr(HeapKind::Iterator)` — this keeps a chained
        // `iter().filter(..).count()` from leaving the `filter` result
        // slot `UInt64` (which would route the `.count()` receiver into
        // the legacy garbage path).
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
/// (`trait T { method name() -> string } type X {} impl T for X { method name()
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
///    (`TraitMemberSignature::Method { return_type: TypeAnnotation, .. }`),
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
        // WF-1A Item 5b (Array<int>.mean drift): `mean()` is an arithmetic
        // (fractional) average — the VM-side `avg_elements` returns Float64 for
        // EVERY element kind, including an `Array<int>` receiver (the average of
        // ints is fractional). Only sum/min/max inherit the element kind. The
        // pre-fix shared arm claimed the element kind (Int64) for
        // `Array<int>.mean()` while `v2_int_avg` returns Float64 → the JIT read
        // the f64 mean bits as an i64. Special-case `mean` to Float64; the VM
        // handler is the source of truth.
        ("mean", ConcreteType::Array(_)) => Some(NativeKind::Float64),
        ("sum" | "min" | "max", ConcreteType::Array(elem)) => native_kind_from_concrete_type(elem),
        // `Array.get(i) -> Option<T>` — bounds-safe accessor (book C4).
        // The VM-side `array_basic::handle_get_v2` returns the canonical
        // fixed-layout `Option<T>` carrier built by
        // `result_option_carrier::build_some` / `build_none` — a TypedObject
        // (schema id `builtin_schemas.option`, `__variant` + payload),
        // Some(elem) in-range / None OOB. The producer-stamped carrier kind
        // is therefore `Ptr(HeapKind::TypedObject)` (L5 Option carrier shape),
        // NOT `Ptr(HeapKind::Option)`. Match/`==` deopt to the interpreter
        // (EnumPayload §2.7.17 soundness gap), which reads this TypedObject
        // via `read_option`.
        ("get", ConcreteType::Array(_)) => Some(NativeKind::Ptr(HeapKind::TypedObject)),
        // `Array.first() / .last() / .pop()` — return the bare element
        // kind directly. Phase 4b Round 4 W15 LANG-9-spin-3-first JIT
        // fix (ADR-006 §2.7.5 producer-side stamp).
        //
        // The VM-side PHF handlers (`executor/objects/typed_int_array_
        // methods::first / last / pop`, `typed_number_array_methods::
        // first / last / pop`) return a `KindedSlot` whose `kind` is the
        // ELEMENT kind for non-empty arrays (`Int64` / `Float64` / etc.)
        // and the `KindedSlot::none()` Bool/0 sentinel for empty arrays.
        // The previous mapping to `Ptr(HeapKind::Option)` here mismatched
        // the producer's stamp — JIT downstream consumers treated the
        // bare element bits (e.g. `Int64=2`) as an `Arc<OptionData>`
        // pointer and rendered "None" on print (pre-fix F3b reproducer
        // `[1,2,3,4,5].map(|x|x*2).first()` JIT=None vs VM=2).
        //
        // Per ADR-006 §2.7.5: the receiver's element type IS the proof
        // of the result kind. The PHF handler (producer) stamps the
        // element kind into its returned `KindedSlot`; the JIT codegen
        // (consumer) must use the same kind expectation. No tag-decode
        // bridge: VM PHF and JIT codegen share the producer-stamped
        // element kind from `ConcreteType::Array(elem)`.
        //
        // The empty-array Bool(0) sentinel case yields slot bits=0 with
        // kind=element; downstream `print` of an Int64=0 renders "0"
        // (NOT "None"). That is the existing PHF contract — the audit's
        // F3 reproducer `[1,2,3,4,5].first()` is non-empty, expected
        // result 1 (Int64); the empty-array Option<T> wrapping is a
        // separate semantic question tracked under a §2.7.17
        // Option-return-shape amendment, NOT the LANG-9-spin-3-first
        // territory.
        ("first" | "last" | "pop", ConcreteType::Array(elem)) => {
            native_kind_from_concrete_type(elem)
        }
        // ── HashMap.get ────────────────────────────────────────────
        // `HashMap<K, V>.get(k) → Option<V>` — the VM-side
        // `hashmap_methods::v2_get` returns
        // `KindedSlot::from_option(Arc<OptionData::some/none>(v))`.
        // Carrier kind is `Ptr(HeapKind::Option)`; the inner V flows
        // through EnumPayload at the destructure site.
        ("get", ConcreteType::HashMap(_, _)) => Some(NativeKind::Ptr(HeapKind::Option)),
        // ── HashMap.set / .delete / .merge ─────────────────────────
        // Phase 4b Round 5c-2-α HashMap-has-2-chain VM/JIT divergence
        // fix (v0.3-gating SOUNDNESS BUG ratified 2026-05-19). Per
        // ADR-006 §2.7.5 producer-side stamp, the VM-side handlers
        // (`hashmap_methods::v2_set` at line 789, `v2_delete` at 1235,
        // `v2_merge` at 1469) all return
        // `KindedSlot::from_hashmap(...)` — carrier kind
        // `Ptr(HeapKind::HashMap)`. Pre-fix the JIT call-terminator
        // stamping loop (`infer_slot_kinds_with_concrete` ~331) had no
        // entry for these mutators, so the chained-set destination
        // slot's kind stayed `None` → fell back to `UInt64` carrier in
        // `operand_slot_kind_or_carrier` at the next call's receiver
        // push → `jit_call_method` shell's delegation predicate (line
        // 665 `NativeKind::UInt64 => false`) routed to legacy JIT-
        // format dispatch → `read_heap_kind` on `Arc::into_raw(Arc<
        // HashMapKindedRef>)` bits returned garbage → dispatch fell
        // into `_ => TAG_NULL` (line 878) → `try_call_user_method`
        // declined → final TAG_NULL bits interpreted as Bool=false by
        // the destination `kinds[idx]=Some(Bool)` (from
        // `well_known_method_return_kind("has")`). Empirical
        // reproducer at HEAD 7eb82205: `HashMap().set("a",1)
        // .set("b",2).has("a")` was VM=true / JIT=false. Post-fix:
        // each mutator stamps the destination slot
        // `Ptr(HeapKind::HashMap)`, so the next `.set` or `.has`
        // receiver kind is the right Ptr arm and `jit_call_method`
        // delegates to VM through the §2.7.10 / Q11 path.
        ("set" | "delete" | "merge", ConcreteType::HashMap(_, _)) => {
            Some(NativeKind::Ptr(HeapKind::HashMap))
        }
        // WF-1A Item 4 (hashmap-filter): `HashMap.filter` / `HashMap.map` /
        // `HashMap.groupBy` all return a FRESH HashMap — the VM handlers
        // `v2_filter` / `v2_map` return `KindedSlot::from_hashmap(...)`, and
        // `v2_group_by` returns `KindedSlot::from_hashmap(HashMapKindedRef::
        // HashMap(...))` (`HashMap<group_key, HashMap>`); carrier
        // `Ptr(HeapKind::HashMap)` in every case. After the item-4
        // `receiver_is_array` gate both modes emit the plain
        // `filter`/`map`/`groupBy` name on a HashMap receiver (instead of the
        // Array-only `filterIndexed`/`mapIndexed`/`groupByIndexed`); without
        // this arm the JIT left the result slot unstamped and a following
        // `.len()` read the raw HashMap pointer bits as an int (audit §4(d)
        // tail — `groupBy` is the same class, exposed by the same gate). The
        // registry is the source of truth.
        ("filter" | "map" | "groupBy", ConcreteType::HashMap(_, _)) => {
            Some(NativeKind::Ptr(HeapKind::HashMap))
        }
        // ── HashSet.add / .delete / .union / .intersection / ───────
        //    .difference ──────────────────────────────────────────
        // Phase 4b Round 5c-2-β-α collection-mutator-chain VM/JIT
        // divergence fix (v0.3-gating SOUNDNESS BUG ratified
        // 2026-05-20; sister-class of the HashMap-has-2-chain fix
        // immediately above). Per ADR-006 §2.7.5 producer-side stamp,
        // the VM-side handlers in `set_methods.rs` —
        // `v2_add` (line 259, `MUT_SELF_HASHSET_METHODS`),
        // `v2_delete`, `v2_union` (312), `v2_intersection` (341),
        // `v2_difference` (374) — all return
        // `KindedSlot::from_hashset(...)`, carrier kind
        // `Ptr(HeapKind::HashSet)`. (`add` / `delete` return the
        // mutated receiver Arc; `union` / `intersection` /
        // `difference` return a fresh result set — both produce the
        // same `Ptr(HeapKind::HashSet)` carrier, mirror of how the
        // HashMap arm covers self-returning `set` / `delete`
        // alongside set-valued `merge`.) Pre-fix the chain-temp
        // slot's kind stayed `None` → `UInt64` carrier fallback →
        // `jit_call_method` legacy-format dispatch → `read_heap_kind`
        // garbage → `_ => TAG_NULL` → the final `.has()` Bool
        // consumer rendered `false`. Empirical reproducer at HEAD
        // db3668c5: `Set().add("a").add("b").has("a")` was VM=true /
        // JIT=false (the 1-chain `Set().add("a").has("a")` also
        // diverged — same root cause as the HashMap 1-chain).
        ("add" | "delete" | "union" | "intersection" | "difference", ConcreteType::HashSet(_)) => {
            Some(NativeKind::Ptr(HeapKind::HashSet))
        }
        // ── Deque.pushBack / .pushFront ────────────────────────────
        // Phase 4b Round 5c-2-β-α collection-mutator-chain fix. The
        // VM-side handlers `deque_methods::v2_push_back` (line 308)
        // and `v2_push_front` (327) — both `MUT_SELF_DEQUE_METHODS`
        // members — return `KindedSlot::from_deque(...)`, carrier
        // kind `Ptr(HeapKind::Deque)`. (`popBack` / `popFront` are
        // tuple-return — they return the popped element, not the
        // deque — so they stay off this arm, mirror of the
        // `MUT_SELF_TUPLE_RETURN_DEQUE_METHODS` exclusion.) Empirical
        // reproducer at HEAD db3668c5:
        // `Deque().pushBack(1).pushBack(2).size()` was VM=2 /
        // JIT=garbage `-1407374883553280`.
        ("pushBack" | "pushFront", ConcreteType::Deque(_)) => {
            Some(NativeKind::Ptr(HeapKind::Deque))
        }
        // ── PriorityQueue.push ─────────────────────────────────────
        // Phase 4b Round 5c-2-β-α collection-mutator-chain fix. The
        // VM-side handler `priority_queue_methods::v2_push` (line
        // 235) — the sole `MUT_SELF_PRIORITY_QUEUE_METHODS` member —
        // returns `KindedSlot::from_priority_queue(...)`, carrier
        // kind `Ptr(HeapKind::PriorityQueue)`. (`pop` is tuple-return
        // per `MUT_SELF_TUPLE_RETURN_PRIORITY_QUEUE_METHODS` — stays
        // off this arm.) Empirical reproducer at HEAD db3668c5:
        // `PriorityQueue().push(5).push(3).size()` was VM=2 /
        // JIT=garbage `-1407374883553280`.
        ("push", ConcreteType::PriorityQueue) => Some(NativeKind::Ptr(HeapKind::PriorityQueue)),
        // ── Channel.send / .close ──────────────────────────────────
        // Phase 4b Round 5c-2-β-α collection-mutator-chain fix. The
        // VM-side handlers `channel_methods::v2_channel_send` (line
        // 98) and `v2_channel_close` (193) both return
        // `KindedSlot::from_channel(...)`, carrier kind
        // `Ptr(HeapKind::Channel)` (the receiver share). `send` /
        // `close` aren't in a `MUT_SELF_*` writeback set —
        // `ChannelData` carries interior mutability via
        // `Mutex<ChannelInner>` so mutations are observed through any
        // Arc share with no slot writeback needed — but they ARE
        // chainable self-returns. Pre-fix the chain-temp kind stayed
        // `None` → `UInt64` carrier → legacy dispatch →
        // wrong-type `Arc<ChannelData>` retain/release. Empirical
        // reproducer at HEAD db3668c5 (hot-loop JIT-compiled):
        // `Channel().send(7).send(9).try_recv()` SIGSEGV'd under JIT
        // (use-after-free on the mis-dispatched `Arc<ChannelData>`)
        // vs VM correct.
        ("send" | "close", ConcreteType::Channel(_)) => Some(NativeKind::Ptr(HeapKind::Channel)),
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
        ("load" | "fetch_add" | "fetch_sub" | "compare_exchange", ConcreteType::Atomic) => {
            Some(NativeKind::Int64)
        }
        // ── Lazy.get ───────────────────────────────────────────────
        // `Lazy<T>.get() → T` per §2.7.25. The cached value's
        // `KindedSlot::value` payload is cloned from `LazyInner.value`
        // after first-init; same receiver-recovery shape as Mutex.
        ("get", ConcreteType::Lazy(inner)) => native_kind_from_concrete_type(inner),
        _ => None,
    }
}

/// Phase 4b Round 5c-2-α HashMap-has-2-chain VM/JIT divergence fix
/// (v0.3-gating SOUNDNESS BUG ratified 2026-05-19).
///
/// ADR-006 §2.7.5 producer-side stamp companion of
/// `parametric_method_return_kind_from_receiver` for the case where the
/// receiver slot's `ConcreteType` is `Void` (no bytecode-compiler
/// concrete-type seed) but the in-pass `kinds[]` track has already
/// classified the receiver's `NativeKind` from an upstream source.
///
/// Load-bearing for chained collection-mutator patterns like
/// `HashMap().set("a",1).set("b",2).has("a")`: the bare-form `HashMap()`
/// ctor receiver gets `Ptr(HeapKind::HashMap)` from the `EnumStore` arm
/// (`types.rs` ~559), but the next `.set(...)` call's destination slot
/// has no ConcreteType entry (the bytecode compiler's concrete-type
/// inference doesn't propagate through method-call return types). With
/// only the ConcreteType-keyed classifier, the chain temp's kind stays
/// `None` → next `.set` / `.has` receiver kind falls back to `UInt64`
/// carrier → `jit_call_method` shell's delegation predicate routes to
/// legacy JIT-format dispatch which doesn't recognize the
/// `Arc::into_raw(Arc<HashMapKindedRef>)` bits → silent TAG_NULL →
/// downstream Bool-kind consumer renders `false`.
///
/// This helper reads the receiver's NativeKind directly from `kinds[]`
/// and classifies HashMap mutator returns (`set` / `delete` / `merge`)
/// — each VM-side handler returns `KindedSlot::from_hashmap(...)` per
/// `hashmap_methods.rs` (v2_set at 789, v2_delete at 1235, v2_merge at
/// 1469). Per §2.7.5: the receiver's `Ptr(HeapKind::HashMap)` kind IS
/// the proof the result is also `Ptr(HeapKind::HashMap)`.
///
/// The companion call-stamp loop in `infer_slot_kinds_with_concrete`
/// runs iteratively until fixpoint (mirrors the existing collection-
/// alias propagation pattern at line 651-687) so chain temps propagate
/// from the EnumStore-stamped ctor receiver through arbitrary chain
/// depth.
fn method_return_kind_from_in_pass_kinds(
    name: &str,
    args: &[Operand],
    kinds: &[Option<NativeKind>],
) -> Option<NativeKind> {
    use shape_value::heap_value::HeapKind;
    let receiver = args.first()?;
    let receiver_slot = match receiver {
        Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => p.root_local(),
        Operand::Constant(_) => return None,
    };
    let receiver_kind = kinds.get(receiver_slot.0 as usize).and_then(|k| *k)?;
    match (name, receiver_kind) {
        // HashMap mutators returning self for chaining. See
        // `parametric_method_return_kind_from_receiver` "HashMap.set /
        // .delete / .merge" arm for the VM-side handler citations.
        ("set" | "delete" | "merge", NativeKind::Ptr(HeapKind::HashMap)) => {
            Some(NativeKind::Ptr(HeapKind::HashMap))
        }
        // WF-1A Item 4 (hashmap-filter): `HashMap.filter` / `HashMap.map` /
        // `HashMap.groupBy` return a fresh HashMap (VM `v2_filter` / `v2_map` /
        // `v2_group_by` -> `from_hashmap`). Load-bearing for the bare
        // `HashMap()` ctor-chain receiver (its `ConcreteType` is Void/Struct,
        // so the parametric classifier can't key it) — the EnumStore arm seeds
        // the ctor temp's `kinds[]` slot and this fixpoint pass propagates
        // `Ptr(HashMap)` through the `.filter`/`.map`/`.groupBy` link so the
        // terminal `.len()` dispatches on the right receiver kind instead of
        // reading garbage.
        ("filter" | "map" | "groupBy", NativeKind::Ptr(HeapKind::HashMap)) => {
            Some(NativeKind::Ptr(HeapKind::HashMap))
        }
        // HashSet / Deque / PriorityQueue / Channel mutator-chain
        // links — Phase 4b Round 5c-2-β-α collection-mutator-chain
        // VM/JIT divergence fix (v0.3-gating SOUNDNESS BUG ratified
        // 2026-05-20; sister-class of the HashMap-has-2-chain arm
        // above). Each receiver-kind / method-name pair mirrors the
        // corresponding `parametric_method_return_kind_from_receiver`
        // arm — VM-side handler citations are in that function's
        // comments. `add` / `delete` (HashSet), `pushBack` /
        // `pushFront` (Deque), `push` (PriorityQueue), `send` /
        // `close` (Channel) return the (mutated) receiver Arc;
        // `union` / `intersection` / `difference` (HashSet) return a
        // fresh result set — both shapes carry
        // `Ptr(HeapKind::<CollectionType>)`. This in-pass-kinds
        // classifier is the load-bearing one for bare-form ctors
        // (`Set()` / `Deque()` / `PriorityQueue()` / `Channel()`)
        // whose receiver `ConcreteType` is `Struct(_)`/`Void` (not
        // the typed-Arc collection type) — the EnumStore arm seeds
        // the ctor temp's `kinds[]` slot, and this fixpoint pass
        // propagates `Ptr(HeapKind::*)` through each chain link.
        (
            "add" | "delete" | "union" | "intersection" | "difference",
            NativeKind::Ptr(HeapKind::HashSet),
        ) => Some(NativeKind::Ptr(HeapKind::HashSet)),
        ("pushBack" | "pushFront", NativeKind::Ptr(HeapKind::Deque)) => {
            Some(NativeKind::Ptr(HeapKind::Deque))
        }
        ("push", NativeKind::Ptr(HeapKind::PriorityQueue)) => {
            Some(NativeKind::Ptr(HeapKind::PriorityQueue))
        }
        ("send" | "close", NativeKind::Ptr(HeapKind::Channel)) => {
            Some(NativeKind::Ptr(HeapKind::Channel))
        }
        // Wave 1b SEAM B (2026-06-15): iterator lazy adapters return a new
        // `Ptr(HeapKind::Iterator)` when applied to an Iterator receiver.
        // This fixpoint-iterated in-pass classifier propagates the
        // Iterator kind through a chained adapter pipeline
        // (`iter().map(..).filter(..).count()`) regardless of block order,
        // so the terminal's receiver is classified Iterator and delegates
        // to the VM trampoline (never the legacy `UInt64` garbage path).
        // The runtime adapter bodies (`handle_map` / `handle_filter` /
        // `handle_take` / `handle_skip` / `handle_flat_map` /
        // `handle_enumerate` / `handle_chain` in `iterator_methods.rs`)
        // each return `wrap_iterator(...)` = `Ptr(HeapKind::Iterator)`.
        (
            "map" | "filter" | "take" | "skip" | "flatMap" | "enumerate" | "chain",
            NativeKind::Ptr(HeapKind::Iterator),
        ) => Some(NativeKind::Ptr(HeapKind::Iterator)),
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
            let either_string =
                matches!(lk, Some(NativeKind::String)) || matches!(rk, Some(NativeKind::String));
            match (lk, rk) {
                _ if matches!(op, BinOp::Add) && either_string => Some(NativeKind::String),
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
        Rvalue::FuzzyComparison { .. } => Some(NativeKind::Bool),
        Rvalue::UnaryOp(UnOp::Neg, operand) => infer_operand_kind_with_projections(
            operand,
            kinds,
            field_kinds,
            field_name_table,
            concrete_types,
        ),
        // W14.2-A1 (Phase 4b, 2026-05-18): `BitNot` propagates the operand
        // kind — Int64 in / Int64 out per the VM's `BitNotInt` typed opcode
        // at `arithmetic/mod.rs:229`. Same `infer_operand_kind_with_projections`
        // shape as `UnOp::Neg` above. `None` operand kind surfaces unstamped
        // per §2.7.7 #9 / forbidden #9 (no fabricated default).
        Rvalue::UnaryOp(UnOp::BitNot, operand) => infer_operand_kind_with_projections(
            operand,
            kinds,
            field_kinds,
            field_name_table,
            concrete_types,
        ),
        // W10 jit-call-method-user-trait-fix (2026-05-17): when the
        // operand is a user-struct receiver (`Ptr(HeapKind::TypedObject)`),
        // the `!x` lowering routes through the `Not::not(self) -> Self`
        // trait method, returning the same user-struct kind. The legacy
        // `Bool` arm only fires for the built-in `!bool` form (where
        // the operand's kind is `Bool` already).
        Rvalue::UnaryOp(UnOp::Not, operand) => {
            let op_kind = infer_operand_kind_with_projections(
                operand,
                kinds,
                field_kinds,
                field_name_table,
                concrete_types,
            );
            // User-struct receiver → trait dispatch returns Self (same
            // pointer kind). Any other operand kind → built-in `!bool`
            // (Bool result by construction). `None` operand kind →
            // surface unstamped per §2.7.7 #9 / forbidden #9 (no
            // fabricated default).
            match op_kind {
                Some(shape_value::NativeKind::Ptr(_)) => op_kind,
                Some(_) => Some(NativeKind::Bool),
                None => None,
            }
        }
        Rvalue::Clone(operand) => infer_operand_kind_with_projections(
            operand,
            kinds,
            field_kinds,
            field_name_table,
            concrete_types,
        ),
        Rvalue::Borrow(_, _) => None, // References are heap pointers
        Rvalue::Aggregate(_) => None, // Arrays are heap objects
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
        // TypePatternTest emits a native Bool — kind is Bool by
        // construction. The JIT consumer surfaces-and-stops on this
        // Rvalue today (preflight rejects it), but the destination slot's
        // kind is still well-defined per ADR-006 §2.7.5 producer-side
        // classification; downstream MIR passes (kind-flow inference,
        // SwitchBool operand classification) see the same Bool stamp
        // whether or not the JIT reaches codegen for the body. W15.2-LANG-5.
        Rvalue::TypePatternTest { .. } => Some(NativeKind::Bool),
        // EnumDiscriminantTest emits a native Bool — kind is Bool by
        // construction (mirror of the TypePatternTest arm above). The JIT
        // consumer surfaces-and-stops on this Rvalue today (preflight
        // rejects it); the destination slot's kind is still well-defined
        // per ADR-006 §2.7.5 producer-side classification. W15.2-LANG-1.
        Rvalue::EnumDiscriminantTest { .. } => Some(NativeKind::Bool),
        // Every formatted-string expression part is materialized before
        // accumulation. The producer is therefore a canonical String even
        // when its input is an inline scalar.
        Rvalue::FormatValue { .. } => Some(NativeKind::String),
        // PrimitiveCast (`expr as int/number/string/bool/decimal/char`) —
        // the destination slot kind is the cast TARGET kind per ADR-006
        // §2.7.5 producer-side classification (the target type name is
        // carried verbatim from `ast::TypeAnnotation`). The JIT consumer
        // surfaces-and-stops on this Rvalue (preflight rejects it →
        // whole-program deopt to the bytecode interpreter), but the
        // destination kind is still well-defined for the kind-flow passes
        // that run before the preflight decision. f-string bool-as-int
        // VM!=JIT divergence fix (2026-06).
        Rvalue::PrimitiveCast { target, .. } => Some(match target.as_str() {
            "int" => NativeKind::Int64,
            "number" => NativeKind::Float64,
            "bool" => NativeKind::Bool,
            "char" => NativeKind::Char,
            "string" => NativeKind::String,
            "decimal" => NativeKind::DecimalV2,
            _ => return None,
        }),
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
///   `RefcountDisposition::SkipTypedCellCarrier` arm (per
///   `ownership.rs:99`) before reaching the `slot_kind` discriminator.
///   If a future smoke surfaces a similar refcount-on-element-read bug,
///   thread `concrete_types` here.
fn infer_operand_kind_with_fields(
    operand: &Operand,
    kinds: &[Option<NativeKind>],
    field_kinds: Option<&std::collections::HashMap<String, NativeKind>>,
    field_name_table: Option<&std::collections::HashMap<FieldIdx, String>>,
) -> Option<NativeKind> {
    infer_operand_kind_with_projections(operand, kinds, field_kinds, field_name_table, None)
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
            // `let n = arr.length` on a PROVEN typed-array receiver. The JIT
            // already produces this shape (`read_place`'s `v2_array_len` +
            // `sextend` to i64) and `place_native_kind` projects it at
            // direct-use sites; without the same projection here the
            // *hoisted* bound local (`let n = arr.length - k; while i < n`)
            // keeps an unproven kind and deopts the whole function. Same
            // receiver proof, same width. Checked before the name-keyed
            // `field_kinds` table, and gated on the receiver being a typed
            // array, so a user struct field spelled `length` cannot reach it.
            if let (Place::Field(base, field_idx), Some(fnt), Some(cts)) =
                (place, field_name_table, concrete_types)
            {
                if fnt.get(field_idx).is_some_and(|n| n == "length")
                    && is_v2_typed_array_slot(cts, base.root_local().0).is_some()
                {
                    return Some(NativeKind::Int64);
                }
            }
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
        // Phase 3 cluster-2 Round 4 cw-D-fam12 follow-up (instance 57,
        // 2026-05-16). ADR-006 §2.7.5 amendment Round 19 S1.5: Char is a
        // 4-byte scalar `NativeKind` variant (codepoint inline in low 32
        // bits, no Arc wrapping). Stamp at the producing site so the `print`
        // dispatch in `terminators.rs` matches the `NativeKind::Char` arm at
        // line ~679 (post-amendment scalar label) and routes to
        // `jit_print_char(u32)`.
        MirConstant::Char(_) => Some(NativeKind::Char),
        // WS-8 (2026-05-22): the canonical post-amendment scalar label for
        // a decimal carrier is `NativeKind::DecimalV2` per ADR-006 §2.7.5
        // amendment Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-
        // additions. Stamping it here surfaces correctly downstream when
        // `compile_constant` returns Err (the `print` dispatch's DecimalV2
        // arm is not wired today, so the W12 fall-through routes to the VM
        // interpreter — VM == JIT, both print correctly).
        MirConstant::Decimal(_) => Some(NativeKind::DecimalV2),
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

    #[test]
    fn u47_jit_projection_delegates_to_shared_value_map() {
        use shape_value::heap_value::HeapKind;
        use shape_value::v2::concrete_type::{
            ClosureTypeId, EnumLayoutId, FunctionTypeId, StructLayoutId,
        };

        let cases = [
            ConcreteType::I64,
            ConcreteType::I32,
            ConcreteType::I16,
            ConcreteType::I8,
            ConcreteType::U64,
            ConcreteType::U32,
            ConcreteType::U16,
            ConcreteType::U8,
            ConcreteType::F64,
            ConcreteType::F32,
            ConcreteType::Char,
            ConcreteType::Bool,
            ConcreteType::String,
            ConcreteType::Array(Box::new(ConcreteType::I64)),
            ConcreteType::HashMap(Box::new(ConcreteType::String), Box::new(ConcreteType::I64)),
            ConcreteType::Option(Box::new(ConcreteType::I64)),
            ConcreteType::Result(Box::new(ConcreteType::I64), Box::new(ConcreteType::String)),
            ConcreteType::placeholder_struct(StructLayoutId(0)),
            ConcreteType::placeholder_enum(EnumLayoutId(0)),
            ConcreteType::Closure(ClosureTypeId(0)),
            ConcreteType::Function(FunctionTypeId(0)),
            ConcreteType::Pointer(Box::new(ConcreteType::U8)),
            ConcreteType::Tuple(vec![ConcreteType::I64, ConcreteType::String]),
            ConcreteType::Decimal,
            ConcreteType::BigInt,
            ConcreteType::DateTime,
            ConcreteType::HashSet(Box::new(ConcreteType::String)),
            ConcreteType::Deque(Box::new(ConcreteType::I64)),
            ConcreteType::PriorityQueue,
            ConcreteType::Channel(Box::new(ConcreteType::I64)),
            ConcreteType::Mutex(Box::new(ConcreteType::I64)),
            ConcreteType::Atomic,
            ConcreteType::Lazy(Box::new(ConcreteType::I64)),
        ];

        for ct in cases {
            assert_eq!(
                native_kind_from_concrete_type(&ct),
                Some(shape_value::v2::closure_layout::native_kind_from_concrete_type(&ct)),
                "JIT must not own a second ConcreteType -> NativeKind map for {ct:?}"
            );
        }

        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::Option(Box::new(ConcreteType::I64))),
            Some(NativeKind::Ptr(HeapKind::TypedObject))
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::Result(
                Box::new(ConcreteType::I64),
                Box::new(ConcreteType::String)
            )),
            Some(NativeKind::Ptr(HeapKind::TypedObject))
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::Pointer(Box::new(ConcreteType::U8))),
            Some(NativeKind::Ptr(HeapKind::NativeView))
        );
        assert_eq!(native_kind_from_concrete_type(&ConcreteType::Void), None);
    }

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
            local_typed_array_element_types: Default::default(),
            local_declared_scalar_types: Default::default(),
            binding_slots: Default::default(),
            local_names: Vec::new(),
            var_binding_slots: Default::default(),
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
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::I64))];
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
    fn parametric_array_first_last_pop_return_element_kind() {
        // Phase 4b Round 4 W15 LANG-9-spin-3-first JIT fix (2026-05-18):
        // `Array<T>.first/last/pop` now return the bare element kind
        // matching the VM-side PHF handler (`typed_int_array_methods::
        // first/last/pop` returns `KindedSlot::from_int(...)` for
        // non-empty arrays). The previous mapping to `Ptr(HeapKind::
        // Option)` mismatched the producer's stamp — JIT downstream
        // consumers treated `Int64=2` bits as an `Arc<OptionData>`
        // pointer and rendered "None" on print. Per ADR-006 §2.7.5: the
        // receiver's element type IS the proof of the result kind.
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::I64))];
        assert_eq!(
            parametric_method_return_kind_from_receiver("first", &[copy_local(0)], &cts),
            Some(NativeKind::Int64)
        );
        assert_eq!(
            parametric_method_return_kind_from_receiver("last", &[copy_local(0)], &cts),
            Some(NativeKind::Int64)
        );
        assert_eq!(
            parametric_method_return_kind_from_receiver("pop", &[copy_local(0)], &cts),
            Some(NativeKind::Int64)
        );

        // `Array<number>.first/last/pop → Float64`
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::F64))];
        assert_eq!(
            parametric_method_return_kind_from_receiver("first", &[copy_local(0)], &cts),
            Some(NativeKind::Float64)
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
        let kind = parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::Option)));
    }

    #[test]
    fn parametric_mutex_get_returns_inner_kind() {
        // Mutex<int>.get() → Int64 per §2.7.25 receiver-recovery.
        let cts = vec![ConcreteType::Mutex(Box::new(ConcreteType::I64))];
        let kind = parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Int64));

        // Mutex<bool>.get() → Bool.
        let cts = vec![ConcreteType::Mutex(Box::new(ConcreteType::Bool))];
        let kind = parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Bool));
    }

    #[test]
    fn parametric_atomic_load_fetch_returns_int64() {
        // Atomic is i64-only at landing per §2.7.25.
        let cts = vec![ConcreteType::Atomic];
        for name in &["load", "fetch_add", "fetch_sub", "compare_exchange"] {
            let kind = parametric_method_return_kind_from_receiver(name, &[copy_local(0)], &cts);
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
        let kind = parametric_method_return_kind_from_receiver("get", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Int64));
    }

    #[test]
    fn parametric_unknown_method_returns_none() {
        // Unknown method names produce None — no Bool-default fallback
        // per §2.7.7 #9.
        let cts = vec![ConcreteType::Array(Box::new(ConcreteType::I64))];
        let kind =
            parametric_method_return_kind_from_receiver("unknown_method", &[copy_local(0)], &cts);
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
        let kind = parametric_method_return_kind_from_receiver("sum", &[copy_local(0)], &cts);
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
        let kind = parametric_method_return_kind_from_receiver("size", &[copy_local(0)], &cts);
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
            local_typed_array_element_types: Default::default(),
            local_declared_scalar_types: Default::default(),
            binding_slots: Default::default(),
            local_names: Vec::new(),
            var_binding_slots: Default::default(),
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

    // ── Phase 4b Round 5c-2-α HashMap-has-2-chain regression tests ────
    //
    // v0.3-gating SOUNDNESS BUG ratified 2026-05-19. Empirical pre-fix
    // at HEAD 7eb82205: `HashMap().set("a",1).set("b",2).has("a")`
    // returned VM=true / JIT=false. Root cause:
    // `parametric_method_return_kind_from_receiver` had no entry for
    // HashMap mutators (`set` / `delete` / `merge`) returning self, so
    // chain temps' kinds stayed None → fell back to `UInt64` carrier
    // → `jit_call_method` shell routed to legacy JIT-format dispatch
    // → `read_heap_kind` on `Arc::into_raw(Arc<HashMapKindedRef>)`
    // bits returned garbage → TAG_NULL → Bool-kind consumer rendered
    // `false`.
    //
    // Post-fix: two stamp sites — (a) the existing
    // `parametric_method_return_kind_from_receiver` classifier gains
    // HashMap mutator arms keyed on `ConcreteType::HashMap(_, _)`;
    // (b) the new `method_return_kind_from_in_pass_kinds` classifier
    // reads the receiver's NativeKind from the in-pass `kinds[]` track
    // (load-bearing for the bare-`HashMap()` ctor case where
    // `concrete_types` is Void but the EnumStore arm has stamped
    // `Ptr(HeapKind::HashMap)` in `kinds[]`); the call-stamp loop is
    // unified with the collection-alias-propagation fixpoint so
    // chain-temps converge through arbitrary depth.

    #[test]
    fn parametric_hashmap_set_returns_hashmap_carrier() {
        // HashMap.set(k, v) → HashMap (chainable). The VM-side
        // `hashmap_methods::v2_set` at line 789 returns
        // `KindedSlot::from_hashmap(...)`. Per ADR-006 §2.7.5
        // producer-side stamp: the destination slot's kind is
        // `Ptr(HeapKind::HashMap)`.
        let cts = vec![ConcreteType::HashMap(
            Box::new(ConcreteType::String),
            Box::new(ConcreteType::I64),
        )];
        let kind = parametric_method_return_kind_from_receiver("set", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::HashMap)));
    }

    #[test]
    fn parametric_hashmap_delete_returns_hashmap_carrier() {
        // HashMap.delete(k) → HashMap (chainable). The VM-side
        // `hashmap_methods::v2_delete` at line 1235 returns
        // `KindedSlot::from_hashmap(...)`.
        let cts = vec![ConcreteType::HashMap(
            Box::new(ConcreteType::String),
            Box::new(ConcreteType::I64),
        )];
        let kind = parametric_method_return_kind_from_receiver("delete", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::HashMap)));
    }

    #[test]
    fn parametric_hashmap_merge_returns_hashmap_carrier() {
        // HashMap.merge(other) → HashMap. The VM-side
        // `hashmap_methods::v2_merge` at line 1469 returns
        // `KindedSlot::from_hashmap(...)`.
        let cts = vec![ConcreteType::HashMap(
            Box::new(ConcreteType::String),
            Box::new(ConcreteType::I64),
        )];
        let kind = parametric_method_return_kind_from_receiver("merge", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::HashMap)));
    }

    #[test]
    fn in_pass_kinds_classifier_stamps_hashmap_mutators_from_kinds_track() {
        // The bare-form `HashMap()` ctor case: `concrete_types` carries
        // Void (the bytecode compiler's concrete-type inference doesn't
        // synthesize `ConcreteType::HashMap(_, _)` for the bare ctor —
        // the EnumStore arm in `infer_slot_kinds_with_concrete` stamps
        // `Ptr(HeapKind::HashMap)` into `kinds[]` instead). The new
        // `method_return_kind_from_in_pass_kinds` classifier reads the
        // receiver kind directly from `kinds[]` so chain temps following
        // a bare ctor still classify correctly.
        let kinds = vec![Some(NativeKind::Ptr(HeapKind::HashMap)), None, None];
        let kind = method_return_kind_from_in_pass_kinds("set", &[copy_local(0)], &kinds);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::HashMap)));

        let kind = method_return_kind_from_in_pass_kinds("delete", &[copy_local(0)], &kinds);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::HashMap)));

        let kind = method_return_kind_from_in_pass_kinds("merge", &[copy_local(0)], &kinds);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::HashMap)));
    }

    #[test]
    fn in_pass_kinds_classifier_returns_none_for_non_hashmap_receivers() {
        // Non-HashMap receivers must return None — no fabricated default
        // per §2.7.7 #9. Pins the cohort: only HashMap mutators returning
        // self are classified by this helper. Array.push, Vec.push, etc.
        // are NOT classified here (their carrier shape differs; broader
        // scope per dispatch supervisor disposition).
        let kinds = vec![Some(NativeKind::Ptr(HeapKind::TypedArray)), None];
        let kind = method_return_kind_from_in_pass_kinds("set", &[copy_local(0)], &kinds);
        assert_eq!(kind, None, "Non-HashMap receiver must not be classified");

        let kinds = vec![Some(NativeKind::Int64), None];
        let kind = method_return_kind_from_in_pass_kinds("set", &[copy_local(0)], &kinds);
        assert_eq!(kind, None, "Scalar receiver must not be classified");

        // Unknown method on HashMap receiver — return None.
        let kinds = vec![Some(NativeKind::Ptr(HeapKind::HashMap)), None];
        let kind =
            method_return_kind_from_in_pass_kinds("unknown_method", &[copy_local(0)], &kinds);
        assert_eq!(kind, None, "Unknown method must not be classified");
    }

    #[test]
    fn hashmap_chain_propagates_kind_through_call_stamp_fixpoint() {
        // Integration test for the load-bearing chain pattern:
        //   temp0 = HashMap()       (EnumStore → Ptr(HashMap) in kinds[])
        //   temp1 = temp0.set(...)  (call-stamp fixpoint → Ptr(HashMap))
        //   temp2 = temp1.set(...)  (call-stamp fixpoint → Ptr(HashMap))
        //   temp3 = temp2.has(...)  (well_known → Bool)
        //
        // Pre-fix: temp1/temp2 stayed None (no classifier entry for
        // `set`), causing the `.has` receiver-kind dispatch to surface
        // the divergence. Post-fix: each chain link's destination slot
        // is stamped `Ptr(HeapKind::HashMap)` and the .has dispatch
        // routes through `jit_trampoline_call_method` correctly.
        let mir = MirFunction {
            name: "hashmap_chain".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![
                        // temp0 = HashMap() — EnumStore arm stamps
                        // Ptr(HashMap) in the forward pass.
                        MirStatement {
                            kind: StatementKind::Assign(
                                Place::Local(SlotId(0)),
                                Rvalue::Aggregate(vec![]),
                            ),
                            span: shape_ast::Span::default(),
                            point: Point(0),
                        },
                        MirStatement {
                            kind: StatementKind::EnumStore {
                                container_slot: SlotId(0),
                                operands: vec![],
                                variant_name: Some("HashMap".to_string()),
                            },
                            span: shape_ast::Span::default(),
                            point: Point(1),
                        },
                    ],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Method("set".to_string())),
                            args: vec![
                                copy_local(0),
                                Operand::Constant(MirConstant::Str("a".to_string())),
                                Operand::Constant(MirConstant::Int(1)),
                            ],
                            destination: Place::Local(SlotId(1)),
                            next: BasicBlockId(1),
                        },
                        span: shape_ast::Span::default(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Method("set".to_string())),
                            args: vec![
                                copy_local(1),
                                Operand::Constant(MirConstant::Str("b".to_string())),
                                Operand::Constant(MirConstant::Int(2)),
                            ],
                            destination: Place::Local(SlotId(2)),
                            next: BasicBlockId(2),
                        },
                        span: shape_ast::Span::default(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(2),
                    statements: vec![],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Method("has".to_string())),
                            args: vec![
                                copy_local(2),
                                Operand::Constant(MirConstant::Str("a".to_string())),
                            ],
                            destination: Place::Local(SlotId(3)),
                            next: BasicBlockId(2),
                        },
                        span: shape_ast::Span::default(),
                    },
                },
            ],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: shape_ast::Span::default(),
            field_name_table: Default::default(),
            local_struct_type_names: Default::default(),
            local_typed_array_element_types: Default::default(),
            local_declared_scalar_types: Default::default(),
            binding_slots: Default::default(),
            local_names: Vec::new(),
            var_binding_slots: Default::default(),
        };
        // No ConcreteType seeds — every slot is Void, mirroring the
        // bare-`HashMap()` ctor pattern.
        let concrete_types = vec![
            ConcreteType::Void,
            ConcreteType::Void,
            ConcreteType::Void,
            ConcreteType::Void,
        ];
        let kinds = infer_slot_kinds_with_concrete(&mir, &[], &concrete_types);
        assert_eq!(
            kinds[0],
            Some(NativeKind::Ptr(HeapKind::HashMap)),
            "temp0 (HashMap() ctor) must be classified via EnumStore arm"
        );
        assert_eq!(
            kinds[1],
            Some(NativeKind::Ptr(HeapKind::HashMap)),
            "temp1 (1st .set) must inherit Ptr(HashMap) via in-pass-kinds classifier"
        );
        assert_eq!(
            kinds[2],
            Some(NativeKind::Ptr(HeapKind::HashMap)),
            "temp2 (2nd .set) must inherit Ptr(HashMap) — fixpoint propagates the chain"
        );
        assert_eq!(
            kinds[3],
            Some(NativeKind::Bool),
            "temp3 (.has) must be Bool from well_known_method_return_kind"
        );
    }

    // ── Phase 4b Round 5c-2-β-α collection-mutator-chain regression ───
    //
    // Sister-class of the HashMap-has-2-chain tests above. Each
    // collection type (HashSet / Deque / PriorityQueue / Channel) has a
    // chainable mutator whose VM-side handler returns
    // `KindedSlot::from_<collection>(...)`; pre-fix the JIT classifiers
    // had no entry for these names so the chain-temp slot's kind stayed
    // `None` → `UInt64` carrier fallback → legacy JIT-format dispatch →
    // wrong consumer rendering (HashSet `false`, Deque/PQ garbage int,
    // Channel SIGSEGV). These pin the producer-side stamp arms in both
    // `parametric_method_return_kind_from_receiver` (ConcreteType-keyed)
    // and `method_return_kind_from_in_pass_kinds` (in-pass-kinds-keyed,
    // load-bearing for bare-form ctors).

    #[test]
    fn parametric_hashset_mutators_return_hashset_carrier() {
        // HashSet.add / .delete return the mutated receiver Arc;
        // .union / .intersection / .difference return a fresh result
        // set — all five VM-side handlers in `set_methods.rs` (v2_add
        // line 259, v2_delete, v2_union 312, v2_intersection 341,
        // v2_difference 374) return `KindedSlot::from_hashset(...)`.
        let cts = vec![ConcreteType::HashSet(Box::new(ConcreteType::String))];
        for name in ["add", "delete", "union", "intersection", "difference"] {
            let kind = parametric_method_return_kind_from_receiver(name, &[copy_local(0)], &cts);
            assert_eq!(
                kind,
                Some(NativeKind::Ptr(HeapKind::HashSet)),
                "HashSet.{name} must classify to Ptr(HeapKind::HashSet)"
            );
        }
    }

    #[test]
    fn parametric_deque_mutators_return_deque_carrier() {
        // Deque.pushBack / .pushFront return the mutated receiver Arc.
        // VM-side `deque_methods::v2_push_back` (308) / `v2_push_front`
        // (327) return `KindedSlot::from_deque(...)`. `popBack` /
        // `popFront` are tuple-return (pop the element) — NOT on the arm.
        let cts = vec![ConcreteType::Deque(Box::new(ConcreteType::I64))];
        for name in ["pushBack", "pushFront"] {
            let kind = parametric_method_return_kind_from_receiver(name, &[copy_local(0)], &cts);
            assert_eq!(
                kind,
                Some(NativeKind::Ptr(HeapKind::Deque)),
                "Deque.{name} must classify to Ptr(HeapKind::Deque)"
            );
        }
        // popBack / popFront stay unclassified by this arm.
        for name in ["popBack", "popFront"] {
            let kind = parametric_method_return_kind_from_receiver(name, &[copy_local(0)], &cts);
            assert_eq!(
                kind, None,
                "Deque.{name} (tuple-return) must not classify here"
            );
        }
    }

    #[test]
    fn parametric_priority_queue_push_returns_priority_queue_carrier() {
        // PriorityQueue.push returns the mutated receiver Arc. VM-side
        // `priority_queue_methods::v2_push` (235) returns
        // `KindedSlot::from_priority_queue(...)`. `ConcreteType::
        // PriorityQueue` is nullary (i64-only at landing per §2.7.18).
        let cts = vec![ConcreteType::PriorityQueue];
        let kind = parametric_method_return_kind_from_receiver("push", &[copy_local(0)], &cts);
        assert_eq!(kind, Some(NativeKind::Ptr(HeapKind::PriorityQueue)));
        // pop is tuple-return — not classified here.
        let kind = parametric_method_return_kind_from_receiver("pop", &[copy_local(0)], &cts);
        assert_eq!(
            kind, None,
            "PriorityQueue.pop (tuple-return) must not classify here"
        );
    }

    #[test]
    fn parametric_channel_mutators_return_channel_carrier() {
        // Channel.send / .close return the receiver share. VM-side
        // `channel_methods::v2_channel_send` (98) / `v2_channel_close`
        // (193) return `KindedSlot::from_channel(...)`.
        let cts = vec![ConcreteType::Channel(Box::new(ConcreteType::I64))];
        for name in ["send", "close"] {
            let kind = parametric_method_return_kind_from_receiver(name, &[copy_local(0)], &cts);
            assert_eq!(
                kind,
                Some(NativeKind::Ptr(HeapKind::Channel)),
                "Channel.{name} must classify to Ptr(HeapKind::Channel)"
            );
        }
    }

    #[test]
    fn in_pass_kinds_classifier_stamps_collection_mutators_from_kinds_track() {
        // The bare-form ctor case (`Set()` / `Deque()` / etc.): the
        // receiver `ConcreteType` is Void, but the EnumStore arm has
        // stamped the collection's `Ptr(HeapKind::*)` into `kinds[]`.
        // `method_return_kind_from_in_pass_kinds` reads that and
        // classifies the chain-temp.
        let hs_kinds = vec![Some(NativeKind::Ptr(HeapKind::HashSet)), None];
        for name in ["add", "delete", "union", "intersection", "difference"] {
            assert_eq!(
                method_return_kind_from_in_pass_kinds(name, &[copy_local(0)], &hs_kinds),
                Some(NativeKind::Ptr(HeapKind::HashSet)),
                "HashSet.{name} in-pass-kinds classification"
            );
        }
        let dq_kinds = vec![Some(NativeKind::Ptr(HeapKind::Deque)), None];
        for name in ["pushBack", "pushFront"] {
            assert_eq!(
                method_return_kind_from_in_pass_kinds(name, &[copy_local(0)], &dq_kinds),
                Some(NativeKind::Ptr(HeapKind::Deque)),
                "Deque.{name} in-pass-kinds classification"
            );
        }
        let pq_kinds = vec![Some(NativeKind::Ptr(HeapKind::PriorityQueue)), None];
        assert_eq!(
            method_return_kind_from_in_pass_kinds("push", &[copy_local(0)], &pq_kinds),
            Some(NativeKind::Ptr(HeapKind::PriorityQueue)),
        );
        let ch_kinds = vec![Some(NativeKind::Ptr(HeapKind::Channel)), None];
        for name in ["send", "close"] {
            assert_eq!(
                method_return_kind_from_in_pass_kinds(name, &[copy_local(0)], &ch_kinds),
                Some(NativeKind::Ptr(HeapKind::Channel)),
                "Channel.{name} in-pass-kinds classification"
            );
        }
    }

    #[test]
    fn in_pass_kinds_classifier_rejects_cross_collection_method_names() {
        // Cohort discipline: a method name is classified ONLY when the
        // receiver kind is the matching collection. `add` on a Deque
        // receiver, `pushBack` on a HashSet receiver, etc. must return
        // None — no fabricated default per §2.7.7 #9.
        let dq_kinds = vec![Some(NativeKind::Ptr(HeapKind::Deque)), None];
        assert_eq!(
            method_return_kind_from_in_pass_kinds("add", &[copy_local(0)], &dq_kinds),
            None,
            "HashSet method `add` on a Deque receiver must not classify"
        );
        let hs_kinds = vec![Some(NativeKind::Ptr(HeapKind::HashSet)), None];
        assert_eq!(
            method_return_kind_from_in_pass_kinds("pushBack", &[copy_local(0)], &hs_kinds),
            None,
            "Deque method `pushBack` on a HashSet receiver must not classify"
        );
        assert_eq!(
            method_return_kind_from_in_pass_kinds("send", &[copy_local(0)], &hs_kinds),
            None,
            "Channel method `send` on a HashSet receiver must not classify"
        );
        // Scalar receiver — never classified.
        let scalar_kinds = vec![Some(NativeKind::Int64), None];
        assert_eq!(
            method_return_kind_from_in_pass_kinds("add", &[copy_local(0)], &scalar_kinds),
            None,
        );
    }

    #[test]
    fn hashset_chain_propagates_kind_through_call_stamp_fixpoint() {
        // End-to-end: `Set().add("a").add("b").has("a")`.
        //   temp0 = Set()          (EnumStore → Ptr(HashSet) in kinds[])
        //   temp1 = temp0.add(...) (in-pass-kinds fixpoint → Ptr(HashSet))
        //   temp2 = temp1.add(...) (in-pass-kinds fixpoint → Ptr(HashSet))
        //   temp3 = temp2.has(...) (well_known → Bool)
        let mir = collection_chain_mir(
            "Set",
            "add",
            &[Operand::Constant(MirConstant::Str("a".to_string()))],
            "has",
            &[Operand::Constant(MirConstant::Str("a".to_string()))],
        );
        let concrete_types = vec![ConcreteType::Void; 4];
        let kinds = infer_slot_kinds_with_concrete(&mir, &[], &concrete_types);
        assert_eq!(kinds[0], Some(NativeKind::Ptr(HeapKind::HashSet)));
        assert_eq!(
            kinds[1],
            Some(NativeKind::Ptr(HeapKind::HashSet)),
            "1st .add chain temp must inherit Ptr(HashSet)"
        );
        assert_eq!(
            kinds[2],
            Some(NativeKind::Ptr(HeapKind::HashSet)),
            "2nd .add chain temp must inherit Ptr(HashSet) — fixpoint propagates"
        );
        assert_eq!(
            kinds[3],
            Some(NativeKind::Bool),
            ".has destination must be Bool from well_known_method_return_kind"
        );
    }

    #[test]
    fn deque_chain_propagates_kind_through_call_stamp_fixpoint() {
        // End-to-end: `Deque().pushBack(1).pushBack(2).size()`.
        let mir = collection_chain_mir(
            "Deque",
            "pushBack",
            &[Operand::Constant(MirConstant::Int(1))],
            "size",
            &[],
        );
        let concrete_types = vec![ConcreteType::Void; 4];
        let kinds = infer_slot_kinds_with_concrete(&mir, &[], &concrete_types);
        assert_eq!(kinds[0], Some(NativeKind::Ptr(HeapKind::Deque)));
        assert_eq!(kinds[1], Some(NativeKind::Ptr(HeapKind::Deque)));
        assert_eq!(kinds[2], Some(NativeKind::Ptr(HeapKind::Deque)));
        assert_eq!(
            kinds[3],
            Some(NativeKind::Int64),
            ".size destination must be Int64 from well_known_method_return_kind"
        );
    }

    #[test]
    fn priority_queue_and_channel_chains_propagate_kind_through_fixpoint() {
        // PriorityQueue: `PriorityQueue().push(5).push(3).size()`.
        let pq_mir = collection_chain_mir(
            "PriorityQueue",
            "push",
            &[Operand::Constant(MirConstant::Int(5))],
            "size",
            &[],
        );
        let pq_kinds = infer_slot_kinds_with_concrete(&pq_mir, &[], &vec![ConcreteType::Void; 4]);
        assert_eq!(pq_kinds[0], Some(NativeKind::Ptr(HeapKind::PriorityQueue)));
        assert_eq!(pq_kinds[1], Some(NativeKind::Ptr(HeapKind::PriorityQueue)));
        assert_eq!(pq_kinds[2], Some(NativeKind::Ptr(HeapKind::PriorityQueue)));
        assert_eq!(pq_kinds[3], Some(NativeKind::Int64));

        // Channel: `Channel().send(7).send(9).is_closed()`.
        let ch_mir = collection_chain_mir(
            "Channel",
            "send",
            &[Operand::Constant(MirConstant::Int(7))],
            "is_closed",
            &[],
        );
        let ch_kinds = infer_slot_kinds_with_concrete(&ch_mir, &[], &vec![ConcreteType::Void; 4]);
        assert_eq!(ch_kinds[0], Some(NativeKind::Ptr(HeapKind::Channel)));
        assert_eq!(ch_kinds[1], Some(NativeKind::Ptr(HeapKind::Channel)));
        assert_eq!(ch_kinds[2], Some(NativeKind::Ptr(HeapKind::Channel)));
    }

    /// Build a 4-slot collection-mutator-chain MIR mirroring the shape of
    /// `hashmap_chain_propagates_kind_through_call_stamp_fixpoint`:
    ///   temp0 = <ctor>()       (Aggregate + EnumStore)
    ///   temp1 = temp0.<mut>(.) (Call terminator)
    ///   temp2 = temp1.<mut>(.) (Call terminator)
    ///   temp3 = temp2.<query>(.) (Call terminator)
    fn collection_chain_mir(
        ctor: &str,
        mutator: &str,
        mut_arg: &[Operand],
        query: &str,
        query_args: &[Operand],
    ) -> MirFunction {
        let mut mut_args_1 = vec![copy_local(0)];
        mut_args_1.extend_from_slice(mut_arg);
        let mut mut_args_2 = vec![copy_local(1)];
        mut_args_2.extend_from_slice(mut_arg);
        let mut query_full = vec![copy_local(2)];
        query_full.extend_from_slice(query_args);
        MirFunction {
            name: "collection_chain".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![
                        MirStatement {
                            kind: StatementKind::Assign(
                                Place::Local(SlotId(0)),
                                Rvalue::Aggregate(vec![]),
                            ),
                            span: shape_ast::Span::default(),
                            point: Point(0),
                        },
                        MirStatement {
                            kind: StatementKind::EnumStore {
                                container_slot: SlotId(0),
                                operands: vec![],
                                variant_name: Some(ctor.to_string()),
                            },
                            span: shape_ast::Span::default(),
                            point: Point(1),
                        },
                    ],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Method(mutator.to_string())),
                            args: mut_args_1,
                            destination: Place::Local(SlotId(1)),
                            next: BasicBlockId(1),
                        },
                        span: shape_ast::Span::default(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Method(mutator.to_string())),
                            args: mut_args_2,
                            destination: Place::Local(SlotId(2)),
                            next: BasicBlockId(2),
                        },
                        span: shape_ast::Span::default(),
                    },
                },
                BasicBlock {
                    id: BasicBlockId(2),
                    statements: vec![],
                    terminator: Terminator {
                        kind: TerminatorKind::Call {
                            func: Operand::Constant(MirConstant::Method(query.to_string())),
                            args: query_full,
                            destination: Place::Local(SlotId(3)),
                            next: BasicBlockId(2),
                        },
                        span: shape_ast::Span::default(),
                    },
                },
            ],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: shape_ast::Span::default(),
            field_name_table: Default::default(),
            local_struct_type_names: Default::default(),
            local_typed_array_element_types: Default::default(),
            local_declared_scalar_types: Default::default(),
            binding_slots: Default::default(),
            local_names: Vec::new(),
            var_binding_slots: Default::default(),
        }
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
    // So Smoke 3 (`trait T { method name() -> string } type X {} impl T for X {
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
        // return type (`string` from `trait T { method name() -> string }`) is
        // unreachable from this classifier.
        //
        // The classifier must return `None` (surface-and-stop posture),
        // NOT a fabricated `NativeKind::String` from hard-coding `"name"`
        // — that would be a CLAUDE.md "Forbidden rationalizations"
        // walk-back ("hard-code the kickoff Smoke 3 case for now").
        let cts = vec![ConcreteType::placeholder_struct(
            shape_value::v2::concrete_type::StructLayoutId(0),
        )];
        let kind = parametric_method_return_kind_from_receiver("name", &[copy_local(0)], &cts);
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
                        func: Operand::Constant(MirConstant::Method("name".to_string())),
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
            local_typed_array_element_types: Default::default(),
            local_declared_scalar_types: Default::default(),
            binding_slots: Default::default(),
            local_names: Vec::new(),
            var_binding_slots: Default::default(),
        };
        let concrete_types = vec![
            ConcreteType::placeholder_struct(shape_value::v2::concrete_type::StructLayoutId(0)),
            ConcreteType::Void,
            ConcreteType::Void,
            ConcreteType::Void,
        ];
        let kinds = infer_slot_kinds_with_concrete(&mir, &[], &concrete_types);
        assert_eq!(
            kinds[1], None,
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
             {{ method name() -> string }}` vs `trait U {{ method name() -> int }}`)."
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
        let cts = vec![ConcreteType::placeholder_struct(
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
            let kind =
                parametric_method_return_kind_from_receiver(method_name, &[copy_local(0)], &cts);
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
        // `trait T { method name() -> string }`). The caller threads this
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
                        func: Operand::Constant(MirConstant::Method("name".to_string())),
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
            local_typed_array_element_types: Default::default(),
            local_declared_scalar_types: Default::default(),
            binding_slots: Default::default(),
            local_names: Vec::new(),
            var_binding_slots: Default::default(),
        };
        // Simulate post-T1' upstream state: `concrete_types[1]` is
        // stamped String by the VM-side conduit; the caller has
        // projected it through `native_kind_from_concrete_type` to
        // form `existing[1] = Some(NativeKind::String)`.
        let concrete_types = vec![
            ConcreteType::placeholder_struct(shape_value::v2::concrete_type::StructLayoutId(0)),
            ConcreteType::String,
            ConcreteType::Void,
            ConcreteType::Void,
        ];
        let existing = vec![None, Some(NativeKind::String), None, None];
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

/// WF-0A gate hardening (2026-07-05): automated cross-check between the
/// JIT return-kind classifier tables in this module —
/// `well_known_method_return_kind` (receiver-invariant),
/// `parametric_method_return_kind_from_receiver` (ConcreteType-keyed),
/// `method_return_kind_from_in_pass_kinds` (NativeKind-keyed) and
/// `iterator_adapter_return_kind` — and the VM method registry
/// (`crates/shape-vm/src/executor/objects/method_registry.rs`).
///
/// The previous sync mechanism was a hand-maintained comment ("Verified
/// against every dispatch table in `method_registry.rs`"). This module
/// replaces trust-the-comment with execute-the-handler: for every method a
/// JIT table claims a return kind for, the REAL registry handler is invoked
/// with a representative receiver, and the returned `KindedSlot.kind` must
/// equal the JIT claim. Claims are read from the actual classifier
/// functions (never re-transcribed), so a change on either side that the
/// other doesn't follow fails these tests with the drifted entries listed.
///
/// Pre-existing drift found when this check first ran is PINNED via
/// `known_drift` (soundness bugs to be fixed in compiler/JIT territory, not
/// silently rebaselined here). A pin that stops reproducing also fails —
/// the pin list must only shrink.
#[cfg(test)]
mod registry_cross_check {
    use super::*;
    use shape_value::heap_value::{
        AtomicData, ChannelData, DequeData, HashMapData, HashMapKindedRef, HashSetData, MutexData,
        PriorityQueueData, RangeData,
    };
    use shape_value::v2::typed_array::{
        ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I64, TypedArray, stamp_elem_type,
    };
    use shape_value::{IteratorSource, IteratorState, KindedSlot, VMError, ValueSlot};
    use shape_vm::executor::method_registry as reg;
    use shape_vm::{VMConfig, VirtualMachine};
    use std::sync::Arc;

    // ── Registry map roster ─────────────────────────────────────────────
    //
    // Every PHF dispatch table in method_registry.rs. The completeness
    // sweep iterates this roster, so adding a map to the registry without
    // adding it here is caught by `sweep_covers_every_registry_map` below
    // (count assertion), and adding an entry whose NAME collides with a
    // JIT invariant-table name is caught by
    // `every_invariant_name_registry_entry_is_cross_checked`.
    fn all_maps() -> Vec<(
        &'static str,
        &'static phf::Map<&'static str, reg::MethodHandler>,
    )> {
        vec![
            ("ARRAY_METHODS", &reg::ARRAY_METHODS),
            ("DATATABLE_METHODS", &reg::DATATABLE_METHODS),
            ("HASHMAP_METHODS", &reg::HASHMAP_METHODS),
            ("SET_METHODS", &reg::SET_METHODS),
            ("DEQUE_METHODS", &reg::DEQUE_METHODS),
            ("PRIORITY_QUEUE_METHODS", &reg::PRIORITY_QUEUE_METHODS),
            ("DATETIME_METHODS", &reg::DATETIME_METHODS),
            ("TIMESPAN_METHODS", &reg::TIMESPAN_METHODS),
            ("INSTANT_METHODS", &reg::INSTANT_METHODS),
            ("ITERATOR_METHODS", &reg::ITERATOR_METHODS),
            ("MATRIX_METHODS", &reg::MATRIX_METHODS),
            ("INDEXED_TABLE_METHODS", &reg::INDEXED_TABLE_METHODS),
            ("FLOAT_ARRAY_METHODS", &reg::FLOAT_ARRAY_METHODS),
            ("INT_ARRAY_METHODS", &reg::INT_ARRAY_METHODS),
            ("BOOL_ARRAY_METHODS", &reg::BOOL_ARRAY_METHODS),
            ("MUTEX_METHODS", &reg::MUTEX_METHODS),
            ("ATOMIC_METHODS", &reg::ATOMIC_METHODS),
            ("LAZY_METHODS", &reg::LAZY_METHODS),
            ("CHANNEL_METHODS", &reg::CHANNEL_METHODS),
            ("NUMBER_METHODS", &reg::NUMBER_METHODS),
            ("STRING_METHODS", &reg::STRING_METHODS),
            ("BOOL_METHODS", &reg::BOOL_METHODS),
            ("CHAR_METHODS", &reg::CHAR_METHODS),
            ("CONTENT_METHODS", &reg::CONTENT_METHODS),
            ("RANGE_METHODS", &reg::RANGE_METHODS),
        ]
    }

    fn map_by_name(name: &str) -> &'static phf::Map<&'static str, reg::MethodHandler> {
        all_maps()
            .into_iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("unknown registry map `{name}` in cross-check case"))
            .1
    }

    /// Invoke a registry handler exactly the way the dispatch shell does:
    /// `args[0]` = receiver, `args[1..]` = call arguments, `ctx = None`.
    fn call(h: reg::MethodHandler, args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
        let mut vm = VirtualMachine::new(VMConfig::default());
        h(&mut vm, args, None)
    }

    // ── Representative receivers ────────────────────────────────────────
    //
    // Built through the same production constructors the VM uses
    // (`TypedArray::with_capacity` + `stamp_elem_type` v2-raw allocator
    // pair; `KindedSlot::from_*` typed-Arc constructors). No kind
    // fabrication: every slot's kind matches its real payload.

    /// Non-empty `TypedArray<i64>` receiver `[10, 20, 30]`.
    fn int_array() -> KindedSlot {
        unsafe {
            let p = TypedArray::<i64>::with_capacity(3) as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I64);
            let arr = p as *mut TypedArray<i64>;
            TypedArray::<i64>::push(arr, 10);
            TypedArray::<i64>::push(arr, 20);
            TypedArray::<i64>::push(arr, 30);
            KindedSlot::new(
                ValueSlot::from_raw(p as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            )
        }
    }

    /// Non-empty `TypedArray<f64>` receiver `[1.5, 2.5]`.
    fn float_array() -> KindedSlot {
        unsafe {
            let p = TypedArray::<f64>::with_capacity(2) as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F64);
            let arr = p as *mut TypedArray<f64>;
            TypedArray::<f64>::push(arr, 1.5);
            TypedArray::<f64>::push(arr, 2.5);
            KindedSlot::new(
                ValueSlot::from_raw(p as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            )
        }
    }

    /// Non-empty `TypedArray<u8>` bool receiver `[true, false]`.
    fn bool_array() -> KindedSlot {
        unsafe {
            let p = TypedArray::<u8>::with_capacity(2) as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_BOOL);
            let arr = p as *mut TypedArray<u8>;
            TypedArray::<u8>::push(arr, 1);
            TypedArray::<u8>::push(arr, 0);
            KindedSlot::new(
                ValueSlot::from_raw(p as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            )
        }
    }

    fn string_recv() -> KindedSlot {
        KindedSlot::from_string("hello world")
    }

    fn range_recv() -> KindedSlot {
        KindedSlot::from_range(Arc::new(RangeData::exclusive(0, 5)))
    }

    /// Empty `HashMap<string, int>` (I64-value carrier).
    fn hashmap_empty() -> KindedSlot {
        KindedSlot::from_hashmap(Arc::new(HashMapKindedRef::I64(
            Arc::new(HashMapData::new()),
        )))
    }

    /// `HashMap<string, int>` with `{"k": 1}` — built through the real
    /// `v2_set` handler so the carrier shape matches production.
    fn hashmap_k1() -> KindedSlot {
        let set = *reg::HASHMAP_METHODS
            .get("set")
            .expect("HASHMAP_METHODS must register `set`");
        call(
            set,
            &[
                hashmap_empty(),
                KindedSlot::from_string("k"),
                KindedSlot::from_int(1),
            ],
        )
        .expect("building the {\"k\": 1} fixture via v2_set must succeed")
    }

    /// String set `{"a"}`.
    fn hashset_a() -> KindedSlot {
        KindedSlot::from_hashset(Arc::new(HashSetData::from_keys(vec![Arc::new(
            "a".to_string(),
        )])))
    }

    fn deque_empty() -> KindedSlot {
        KindedSlot::from_deque(Arc::new(DequeData::new()))
    }

    fn pq_empty() -> KindedSlot {
        KindedSlot::from_priority_queue(Arc::new(PriorityQueueData::new()))
    }

    fn channel_open() -> KindedSlot {
        KindedSlot::from_channel(Arc::new(ChannelData::new()))
    }

    /// Lazy range iterator over `0..5`.
    fn iter_range() -> KindedSlot {
        KindedSlot::from_iterator(Arc::new(IteratorState::new(IteratorSource::Range {
            start: 0,
            end: 5,
            step: 1,
        })))
    }

    fn mutex_int() -> KindedSlot {
        KindedSlot::from_mutex(Arc::new(MutexData::new(KindedSlot::from_int(42))))
    }

    fn atomic_one() -> KindedSlot {
        KindedSlot::from_atomic(Arc::new(AtomicData::new(1)))
    }

    // ── Case runner ─────────────────────────────────────────────────────

    struct Case {
        /// Human label, e.g. `Array<int>.mean()`.
        label: &'static str,
        /// Registry map the dispatch shell would consult for this
        /// receiver (mirrors `objects/mod.rs` receiver-kind routing,
        /// incl. `typed_array_method_registry`'s per-elem-kind map with
        /// ARRAY_METHODS fallback).
        map_name: &'static str,
        method: &'static str,
        /// `args[0]` = receiver, `args[1..]` = call arguments.
        args: Vec<KindedSlot>,
        /// JIT-side claimed return kind — ALWAYS produced by calling the
        /// actual classifier fn, never transcribed by hand.
        claim: Option<NativeKind>,
        /// `Some(reason)` pins pre-existing drift found when this check
        /// first ran (2026-07-05). Pinned entries must keep drifting —
        /// a pin that starts agreeing fails as stale, so fixes must
        /// remove the pin in the same change.
        known_drift: Option<&'static str>,
    }

    fn run_cases(table_name: &str, cases: Vec<Case>) {
        let mut failures: Vec<String> = Vec::new();
        let mut pinned: Vec<String> = Vec::new();
        for case in cases {
            let Some(claim) = case.claim else {
                failures.push(format!(
                    "{}: JIT table `{table_name}` no longer classifies this entry \
                     (arm deleted or receiver-shape changed) — update the cross-check",
                    case.label
                ));
                continue;
            };
            let Some(handler) = map_by_name(case.map_name).get(case.method) else {
                failures.push(format!(
                    "{}: `{}` has no `{}` entry — JIT claims a return kind for a \
                     method the registry does not register",
                    case.label, case.map_name, case.method
                ));
                continue;
            };
            match call(*handler, &case.args) {
                Ok(result) => {
                    let agree = result.kind == claim;
                    match (agree, case.known_drift) {
                        (true, None) => {}
                        (true, Some(reason)) => failures.push(format!(
                            "{}: pinned drift no longer reproduces — JIT and VM now both \
                             return {:?}; remove the stale pin ({reason})",
                            case.label, claim
                        )),
                        (false, Some(reason)) => pinned.push(format!(
                            "{}: JIT claims {:?}, VM `{}[\"{}\"]` returned {:?} — {reason}",
                            case.label, claim, case.map_name, case.method, result.kind
                        )),
                        (false, None) => failures.push(format!(
                            "{}: JIT `{table_name}` claims {:?}, VM `{}[\"{}\"]` returned {:?}",
                            case.label, claim, case.map_name, case.method, result.kind
                        )),
                    }
                }
                // A pinned entry may also reproduce as an invocation
                // failure (e.g. the HashMap.iter carrier mismatch, where
                // the handler cannot decode the receiver at all).
                Err(e) => match case.known_drift {
                    Some(reason) => pinned.push(format!(
                        "{}: VM `{}[\"{}\"]` failed instead of returning {:?}: {e:?} — {reason}",
                        case.label, case.map_name, case.method, claim
                    )),
                    None => failures.push(format!(
                        "{}: harness invocation of `{}[\"{}\"]` failed: {e:?}",
                        case.label, case.map_name, case.method
                    )),
                },
            }
        }
        if !pinned.is_empty() {
            eprintln!(
                "KNOWN JIT<->method_registry return-kind drift in `{table_name}` \
                 (pinned soundness bugs — list must only shrink):\n{}",
                pinned.join("\n")
            );
        }
        assert!(
            failures.is_empty(),
            "JIT return-kind table `{table_name}` drifted from method_registry.rs:\n{}",
            failures.join("\n")
        );
    }

    /// `args` operand vec putting the receiver in MIR slot 0 — the shape
    /// both parametric classifiers key their receiver lookup on.
    fn recv_operands() -> Vec<Operand> {
        vec![Operand::Copy(Place::Local(SlotId(0)))]
    }

    // ── Test 1: receiver-invariant table ────────────────────────────────

    /// `(map, method, args)` for every registry entry whose name appears
    /// in `well_known_method_return_kind` and whose receiver is
    /// constructible in a unit test.
    fn invariant_cases() -> Vec<Case> {
        let inv = |label, map_name, method: &'static str, args| Case {
            label,
            map_name,
            method,
            args,
            claim: well_known_method_return_kind(method),
            known_drift: None,
        };
        vec![
            // Array (TypedArray<i64> receiver; ARRAY_METHODS is the
            // fallback map for every element kind).
            inv("Array.len()", "ARRAY_METHODS", "len", vec![int_array()]),
            inv(
                "Array.length()",
                "ARRAY_METHODS",
                "length",
                vec![int_array()],
            ),
            inv(
                "Array.isEmpty()",
                "ARRAY_METHODS",
                "isEmpty",
                vec![int_array()],
            ),
            inv("Array.count()", "ARRAY_METHODS", "count", vec![int_array()]),
            inv("Array.iter()", "ARRAY_METHODS", "iter", vec![int_array()]),
            // HashMap
            inv(
                "HashMap.has(k)",
                "HASHMAP_METHODS",
                "has",
                vec![hashmap_k1(), KindedSlot::from_string("k")],
            ),
            inv(
                "HashMap.len()",
                "HASHMAP_METHODS",
                "len",
                vec![hashmap_k1()],
            ),
            inv(
                "HashMap.length()",
                "HASHMAP_METHODS",
                "length",
                vec![hashmap_k1()],
            ),
            inv(
                "HashMap.isEmpty()",
                "HASHMAP_METHODS",
                "isEmpty",
                vec![hashmap_k1()],
            ),
            // WF-1A Item 5b (2026-07-05): RETIRED pin. `handle_hashmap_iter`
            // now recovers the receiver via `hashmap_methods::as_hashmap` (the
            // kinded `Arc<HashMapKindedRef>` path the producer wrote), so it
            // returns `Ptr(Iterator)` cleanly on a `v2_set`-produced receiver —
            // JIT claim and VM handler now agree. Demoted to an invariant case.
            inv(
                "HashMap.iter()",
                "HASHMAP_METHODS",
                "iter",
                vec![hashmap_k1()],
            ),
            // Set
            inv(
                "Set.has(k)",
                "SET_METHODS",
                "has",
                vec![hashset_a(), KindedSlot::from_string("a")],
            ),
            inv("Set.len()", "SET_METHODS", "len", vec![hashset_a()]),
            inv("Set.length()", "SET_METHODS", "length", vec![hashset_a()]),
            inv("Set.isEmpty()", "SET_METHODS", "isEmpty", vec![hashset_a()]),
            // Deque
            inv("Deque.size()", "DEQUE_METHODS", "size", vec![deque_empty()]),
            inv("Deque.len()", "DEQUE_METHODS", "len", vec![deque_empty()]),
            inv(
                "Deque.length()",
                "DEQUE_METHODS",
                "length",
                vec![deque_empty()],
            ),
            inv(
                "Deque.isEmpty()",
                "DEQUE_METHODS",
                "isEmpty",
                vec![deque_empty()],
            ),
            // PriorityQueue
            inv(
                "PriorityQueue.size()",
                "PRIORITY_QUEUE_METHODS",
                "size",
                vec![pq_empty()],
            ),
            inv(
                "PriorityQueue.len()",
                "PRIORITY_QUEUE_METHODS",
                "len",
                vec![pq_empty()],
            ),
            inv(
                "PriorityQueue.length()",
                "PRIORITY_QUEUE_METHODS",
                "length",
                vec![pq_empty()],
            ),
            inv(
                "PriorityQueue.isEmpty()",
                "PRIORITY_QUEUE_METHODS",
                "isEmpty",
                vec![pq_empty()],
            ),
            // Iterator
            inv(
                "Iterator.count()",
                "ITERATOR_METHODS",
                "count",
                vec![iter_range()],
            ),
            // Typed-array per-elem-kind maps
            inv(
                "Vec<number>.len()",
                "FLOAT_ARRAY_METHODS",
                "len",
                vec![float_array()],
            ),
            inv(
                "Vec<number>.length()",
                "FLOAT_ARRAY_METHODS",
                "length",
                vec![float_array()],
            ),
            inv(
                "Vec<int>.len()",
                "INT_ARRAY_METHODS",
                "len",
                vec![int_array()],
            ),
            inv(
                "Vec<int>.length()",
                "INT_ARRAY_METHODS",
                "length",
                vec![int_array()],
            ),
            inv(
                "Vec<bool>.len()",
                "BOOL_ARRAY_METHODS",
                "len",
                vec![bool_array()],
            ),
            inv(
                "Vec<bool>.length()",
                "BOOL_ARRAY_METHODS",
                "length",
                vec![bool_array()],
            ),
            inv(
                "Vec<bool>.isEmpty()",
                "BOOL_ARRAY_METHODS",
                "isEmpty",
                vec![bool_array()],
            ),
            inv(
                "Vec<bool>.count()",
                "BOOL_ARRAY_METHODS",
                "count",
                vec![bool_array()],
            ),
            // String
            inv("String.len()", "STRING_METHODS", "len", vec![string_recv()]),
            inv(
                "String.length()",
                "STRING_METHODS",
                "length",
                vec![string_recv()],
            ),
            inv(
                "String.contains(s)",
                "STRING_METHODS",
                "contains",
                vec![string_recv(), KindedSlot::from_string("lo")],
            ),
            inv(
                "String.iter()",
                "STRING_METHODS",
                "iter",
                vec![string_recv()],
            ),
            // Range
            inv(
                "Range.contains(i)",
                "RANGE_METHODS",
                "contains",
                vec![range_recv(), KindedSlot::from_int(3)],
            ),
            inv(
                "Range.length()",
                "RANGE_METHODS",
                "length",
                vec![range_recv()],
            ),
            inv("Range.size()", "RANGE_METHODS", "size", vec![range_recv()]),
            inv("Range.len()", "RANGE_METHODS", "len", vec![range_recv()]),
            inv(
                "Range.isEmpty()",
                "RANGE_METHODS",
                "isEmpty",
                vec![range_recv()],
            ),
            inv("Range.iter()", "RANGE_METHODS", "iter", vec![range_recv()]),
        ]
    }

    /// Registry entries whose name IS in the invariant table but whose
    /// receiver cannot be constructed in this unit-test harness. Each
    /// needs a reason; the completeness sweep fails on any entry that is
    /// neither verified nor listed here.
    fn invariant_unverified() -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            (
                "DATATABLE_METHODS",
                "len",
                "DataTable receiver requires an arrow RecordBatch fixture",
            ),
            (
                "DATATABLE_METHODS",
                "length",
                "DataTable receiver requires an arrow RecordBatch fixture",
            ),
            (
                "DATATABLE_METHODS",
                "count",
                "DataTable receiver requires an arrow RecordBatch fixture",
            ),
        ]
    }

    #[test]
    fn invariant_table_matches_method_registry() {
        run_cases("well_known_method_return_kind", invariant_cases());
    }

    /// Completeness sweep: every registry entry (all maps × all names)
    /// whose name the JIT invariant table classifies must be either
    /// invoked by `invariant_cases()` or explicitly allow-listed with a
    /// reason. Guards against silent drift when a NEW registry entry
    /// reuses an invariant-classified name (`len`, `count`, `has`, ...)
    /// with a different return shape.
    #[test]
    fn every_invariant_name_registry_entry_is_cross_checked() {
        use std::collections::HashSet;
        let verified: HashSet<(&str, &str)> = invariant_cases()
            .iter()
            .map(|c| (c.map_name, c.method))
            .collect();
        let unverified: HashSet<(&str, &str)> = invariant_unverified()
            .iter()
            .map(|(m, n, _)| (*m, *n))
            .collect();
        let mut missing = Vec::new();
        for (map_name, map) in all_maps() {
            for name in map.keys() {
                if well_known_method_return_kind(name).is_none() {
                    continue;
                }
                let key = (map_name, *name);
                if !verified.contains(&key) && !unverified.contains(&key) {
                    missing.push(format!(
                        "{map_name}[\"{name}\"] matches a JIT invariant-table name but has \
                         no cross-check case — add it to invariant_cases() (or, with a \
                         reason, to invariant_unverified())"
                    ));
                }
            }
        }
        assert!(missing.is_empty(), "{}", missing.join("\n"));
    }

    /// The roster above must track method_registry.rs. If a map is added
    /// or removed there, update `all_maps()` (the sweep is only as
    /// complete as the roster).
    #[test]
    fn sweep_covers_every_registry_map() {
        assert_eq!(
            all_maps().len(),
            25,
            "registry map roster out of date — sync all_maps() with the \
             `pub static *_METHODS` set in method_registry.rs"
        );
    }

    // ── Test 2: ConcreteType-parametric table ───────────────────────────

    #[test]
    fn parametric_receiver_table_matches_method_registry() {
        let claim = |method: &'static str, ct: ConcreteType| {
            parametric_method_return_kind_from_receiver(method, &recv_operands(), &[ct])
        };
        let int_arr_ct = || ConcreteType::Array(Box::new(ConcreteType::I64));
        let float_arr_ct = || ConcreteType::Array(Box::new(ConcreteType::F64));
        let hashmap_ct =
            || ConcreteType::HashMap(Box::new(ConcreteType::String), Box::new(ConcreteType::I64));
        let hashset_ct = || ConcreteType::HashSet(Box::new(ConcreteType::String));
        let deque_ct = || ConcreteType::Deque(Box::new(ConcreteType::I64));
        let channel_ct = || ConcreteType::Channel(Box::new(ConcreteType::I64));
        let mutex_ct = || ConcreteType::Mutex(Box::new(ConcreteType::I64));

        // Map choice per case mirrors the dispatch shell: TypedArray
        // receivers consult `typed_array_method_registry` (INT_ARRAY /
        // FLOAT_ARRAY per elem kind) first, then fall back to
        // ARRAY_METHODS (`objects/mod.rs::typed_array_method_registry`).
        let pc =
            |label, map_name, method: &'static str, ct: ConcreteType, args, known_drift| Case {
                label,
                map_name,
                method,
                args,
                claim: claim(method, ct),
                known_drift,
            };
        let cases = vec![
            // Array<int> element-parametric accessors/aggregations
            pc(
                "Array<int>.sum()",
                "INT_ARRAY_METHODS",
                "sum",
                int_arr_ct(),
                vec![int_array()],
                None,
            ),
            pc(
                "Array<int>.mean()",
                "INT_ARRAY_METHODS",
                "mean",
                int_arr_ct(),
                vec![int_array()],
                // WF-1A Item 5b (2026-07-05): RETIRED. The
                // `parametric_method_return_kind_from_receiver` sum/mean/min/max
                // arm now special-cases `mean` -> Float64, matching the VM's
                // `v2_int_avg` -> `avg_elements` fractional-average return. JIT
                // and VM now agree (both Float64) for `Array<int>.mean()`.
                None,
            ),
            pc(
                "Array<int>.min()",
                "INT_ARRAY_METHODS",
                "min",
                int_arr_ct(),
                vec![int_array()],
                None,
            ),
            pc(
                "Array<int>.max()",
                "INT_ARRAY_METHODS",
                "max",
                int_arr_ct(),
                vec![int_array()],
                None,
            ),
            pc(
                "Array<int>.get(i)",
                "ARRAY_METHODS",
                "get",
                int_arr_ct(),
                vec![int_array(), KindedSlot::from_int(1)],
                None,
            ),
            pc(
                "Array<int>.first()",
                "ARRAY_METHODS",
                "first",
                int_arr_ct(),
                vec![int_array()],
                None,
            ),
            pc(
                "Array<int>.last()",
                "ARRAY_METHODS",
                "last",
                int_arr_ct(),
                vec![int_array()],
                None,
            ),
            pc(
                "Array<int>.pop()",
                "ARRAY_METHODS",
                "pop",
                int_arr_ct(),
                vec![int_array()],
                None,
            ),
            // Array<number>
            pc(
                "Array<number>.sum()",
                "FLOAT_ARRAY_METHODS",
                "sum",
                float_arr_ct(),
                vec![float_array()],
                None,
            ),
            pc(
                "Array<number>.mean()",
                "FLOAT_ARRAY_METHODS",
                "mean",
                float_arr_ct(),
                vec![float_array()],
                None,
            ),
            pc(
                "Array<number>.min()",
                "FLOAT_ARRAY_METHODS",
                "min",
                float_arr_ct(),
                vec![float_array()],
                None,
            ),
            pc(
                "Array<number>.max()",
                "FLOAT_ARRAY_METHODS",
                "max",
                float_arr_ct(),
                vec![float_array()],
                None,
            ),
            pc(
                "Array<number>.get(i)",
                "ARRAY_METHODS",
                "get",
                float_arr_ct(),
                vec![float_array(), KindedSlot::from_int(0)],
                None,
            ),
            pc(
                "Array<number>.first()",
                "ARRAY_METHODS",
                "first",
                float_arr_ct(),
                vec![float_array()],
                None,
            ),
            pc(
                "Array<number>.last()",
                "ARRAY_METHODS",
                "last",
                float_arr_ct(),
                vec![float_array()],
                None,
            ),
            pc(
                "Array<number>.pop()",
                "ARRAY_METHODS",
                "pop",
                float_arr_ct(),
                vec![float_array()],
                None,
            ),
            // HashMap<string, int>
            pc(
                "HashMap<string,int>.get(k) [hit]",
                "HASHMAP_METHODS",
                "get",
                hashmap_ct(),
                vec![hashmap_k1(), KindedSlot::from_string("k")],
                Some(
                    "PIN(2026-07-05, RECLASSIFIED WF-1A -> WF-3A type-system-edges \
                     / ADR-006 §2.7.17 Option-return-shape amendment): JIT claims \
                     Ptr(Option) (the table comment says `v2_get` returns \
                     `KindedSlot::from_option`), but the actual handler returns a \
                     NON-UNIFORM kind — the BARE value kind on hit (`get_kinded` \
                     -> `from_int` for I64 maps) and Null on miss. No single \
                     static return-kind can align the JIT table to a non-uniform \
                     handler; the sound fix is handler-side (make `HashMap.get` \
                     return a real uniform `Option<V>`), a language-semantics \
                     change beyond WF-1A JIT-table alignment (touches \
                     destructuring / `match` / `?`). Kept pinned as a soundness \
                     bug; NOT a JIT return-kind-table drift the harness can fix.",
                ),
            ),
            pc(
                "HashMap<string,int>.get(k) [miss]",
                "HASHMAP_METHODS",
                "get",
                hashmap_ct(),
                vec![hashmap_k1(), KindedSlot::from_string("absent")],
                Some(
                    "PIN(2026-07-05, RECLASSIFIED WF-1A -> WF-3A type-system-edges \
                     / ADR-006 §2.7.17 Option-return-shape amendment): JIT claims \
                     Ptr(Option), but the actual handler returns \
                     `KindedSlot::none()` (kind Null) on miss — the miss half of \
                     the non-uniform `HashMap.get` return above. Sound fix is a \
                     uniform `Option<V>` handler return (handler-side, \
                     language-semantics); kept pinned, not a harness-fixable \
                     JIT-table drift.",
                ),
            ),
            pc(
                "HashMap<string,int>.set(k, v)",
                "HASHMAP_METHODS",
                "set",
                hashmap_ct(),
                vec![
                    hashmap_k1(),
                    KindedSlot::from_string("k2"),
                    KindedSlot::from_int(2),
                ],
                None,
            ),
            pc(
                "HashMap<string,int>.delete(k)",
                "HASHMAP_METHODS",
                "delete",
                hashmap_ct(),
                vec![hashmap_k1(), KindedSlot::from_string("k")],
                None,
            ),
            pc(
                "HashMap<string,int>.merge(other)",
                "HASHMAP_METHODS",
                "merge",
                hashmap_ct(),
                vec![hashmap_k1(), hashmap_k1()],
                None,
            ),
            // HashSet<string>
            pc(
                "Set<string>.add(k)",
                "SET_METHODS",
                "add",
                hashset_ct(),
                vec![hashset_a(), KindedSlot::from_string("b")],
                None,
            ),
            pc(
                "Set<string>.delete(k)",
                "SET_METHODS",
                "delete",
                hashset_ct(),
                vec![hashset_a(), KindedSlot::from_string("a")],
                None,
            ),
            pc(
                "Set<string>.union(other)",
                "SET_METHODS",
                "union",
                hashset_ct(),
                vec![hashset_a(), hashset_a()],
                None,
            ),
            pc(
                "Set<string>.intersection(other)",
                "SET_METHODS",
                "intersection",
                hashset_ct(),
                vec![hashset_a(), hashset_a()],
                None,
            ),
            pc(
                "Set<string>.difference(other)",
                "SET_METHODS",
                "difference",
                hashset_ct(),
                vec![hashset_a(), hashset_a()],
                None,
            ),
            // Deque<int>
            pc(
                "Deque<int>.pushBack(v)",
                "DEQUE_METHODS",
                "pushBack",
                deque_ct(),
                vec![deque_empty(), KindedSlot::from_int(1)],
                None,
            ),
            pc(
                "Deque<int>.pushFront(v)",
                "DEQUE_METHODS",
                "pushFront",
                deque_ct(),
                vec![deque_empty(), KindedSlot::from_int(1)],
                None,
            ),
            // PriorityQueue
            pc(
                "PriorityQueue.push(p)",
                "PRIORITY_QUEUE_METHODS",
                "push",
                ConcreteType::PriorityQueue,
                vec![pq_empty(), KindedSlot::from_int(5)],
                None,
            ),
            // Channel<int>
            pc(
                "Channel<int>.send(v)",
                "CHANNEL_METHODS",
                "send",
                channel_ct(),
                vec![channel_open(), KindedSlot::from_int(7)],
                None,
            ),
            pc(
                "Channel<int>.close()",
                "CHANNEL_METHODS",
                "close",
                channel_ct(),
                vec![channel_open()],
                None,
            ),
            // Mutex<int>
            pc(
                "Mutex<int>.get()",
                "MUTEX_METHODS",
                "get",
                mutex_ct(),
                vec![mutex_int()],
                None,
            ),
            // Atomic (i64-only per §2.7.25)
            pc(
                "Atomic.load()",
                "ATOMIC_METHODS",
                "load",
                ConcreteType::Atomic,
                vec![atomic_one()],
                None,
            ),
            pc(
                "Atomic.fetch_add(d)",
                "ATOMIC_METHODS",
                "fetch_add",
                ConcreteType::Atomic,
                vec![atomic_one(), KindedSlot::from_int(1)],
                None,
            ),
            pc(
                "Atomic.fetch_sub(d)",
                "ATOMIC_METHODS",
                "fetch_sub",
                ConcreteType::Atomic,
                vec![atomic_one(), KindedSlot::from_int(1)],
                None,
            ),
            pc(
                "Atomic.compare_exchange(e, n)",
                "ATOMIC_METHODS",
                "compare_exchange",
                ConcreteType::Atomic,
                vec![
                    atomic_one(),
                    KindedSlot::from_int(1),
                    KindedSlot::from_int(2),
                ],
                None,
            ),
            // NOT covered: `Lazy<T>.get()` — the JIT arm exists (asserted
            // below) but `v2_lazy_get` on an uninitialized Lazy must run a
            // real closure initializer, which this harness cannot
            // construct. Tracked as unverified.
        ];
        run_cases("parametric_method_return_kind_from_receiver", cases);

        // Lazy<T>.get(): assert the JIT arm still classifies (Int64 for
        // Lazy<int>) and the registry still registers the handler, so a
        // deletion on either side surfaces here even though the handler
        // can't be invoked without a closure.
        assert_eq!(
            claim("get", ConcreteType::Lazy(Box::new(ConcreteType::I64))),
            Some(NativeKind::Int64),
            "JIT parametric table lost the Lazy<T>.get() arm"
        );
        assert!(
            reg::LAZY_METHODS.get("get").is_some(),
            "LAZY_METHODS no longer registers `get`"
        );
    }

    // ── Test 3: in-pass-kinds parametric table ──────────────────────────

    #[test]
    fn in_pass_kinds_table_matches_method_registry() {
        let claim = |method: &'static str, kind: NativeKind| {
            method_return_kind_from_in_pass_kinds(method, &recv_operands(), &[Some(kind)])
        };
        let kc = |label, map_name, method: &'static str, kind: NativeKind, args| Case {
            label,
            map_name,
            method,
            args,
            claim: claim(method, kind),
            known_drift: None,
        };
        let hm = NativeKind::Ptr(HeapKind::HashMap);
        let hs = NativeKind::Ptr(HeapKind::HashSet);
        let dq = NativeKind::Ptr(HeapKind::Deque);
        let pq = NativeKind::Ptr(HeapKind::PriorityQueue);
        let ch = NativeKind::Ptr(HeapKind::Channel);
        let it = NativeKind::Ptr(HeapKind::Iterator);
        let cases = vec![
            kc(
                "HashMap.set(k, v) [in-pass]",
                "HASHMAP_METHODS",
                "set",
                hm,
                vec![
                    hashmap_k1(),
                    KindedSlot::from_string("k2"),
                    KindedSlot::from_int(2),
                ],
            ),
            kc(
                "HashMap.delete(k) [in-pass]",
                "HASHMAP_METHODS",
                "delete",
                hm,
                vec![hashmap_k1(), KindedSlot::from_string("k")],
            ),
            kc(
                "HashMap.merge(other) [in-pass]",
                "HASHMAP_METHODS",
                "merge",
                hm,
                vec![hashmap_k1(), hashmap_k1()],
            ),
            kc(
                "Set.add(k) [in-pass]",
                "SET_METHODS",
                "add",
                hs,
                vec![hashset_a(), KindedSlot::from_string("b")],
            ),
            kc(
                "Set.delete(k) [in-pass]",
                "SET_METHODS",
                "delete",
                hs,
                vec![hashset_a(), KindedSlot::from_string("a")],
            ),
            kc(
                "Set.union(o) [in-pass]",
                "SET_METHODS",
                "union",
                hs,
                vec![hashset_a(), hashset_a()],
            ),
            kc(
                "Set.intersection(o) [in-pass]",
                "SET_METHODS",
                "intersection",
                hs,
                vec![hashset_a(), hashset_a()],
            ),
            kc(
                "Set.difference(o) [in-pass]",
                "SET_METHODS",
                "difference",
                hs,
                vec![hashset_a(), hashset_a()],
            ),
            kc(
                "Deque.pushBack(v) [in-pass]",
                "DEQUE_METHODS",
                "pushBack",
                dq,
                vec![deque_empty(), KindedSlot::from_int(1)],
            ),
            kc(
                "Deque.pushFront(v) [in-pass]",
                "DEQUE_METHODS",
                "pushFront",
                dq,
                vec![deque_empty(), KindedSlot::from_int(1)],
            ),
            kc(
                "PriorityQueue.push(p) [in-pass]",
                "PRIORITY_QUEUE_METHODS",
                "push",
                pq,
                vec![pq_empty(), KindedSlot::from_int(5)],
            ),
            kc(
                "Channel.send(v) [in-pass]",
                "CHANNEL_METHODS",
                "send",
                ch,
                vec![channel_open(), KindedSlot::from_int(7)],
            ),
            kc(
                "Channel.close() [in-pass]",
                "CHANNEL_METHODS",
                "close",
                ch,
                vec![channel_open()],
            ),
            // Iterator lazy adapters with non-closure arguments — these
            // empirically pin the shared `append_transform` ->
            // `wrap_iterator` return path that `map`/`filter`/`flatMap`
            // also use (those three need a real closure argument and are
            // covered structurally below).
            kc(
                "Iterator.take(n) [in-pass]",
                "ITERATOR_METHODS",
                "take",
                it,
                vec![iter_range(), KindedSlot::from_int(2)],
            ),
            kc(
                "Iterator.skip(n) [in-pass]",
                "ITERATOR_METHODS",
                "skip",
                it,
                vec![iter_range(), KindedSlot::from_int(1)],
            ),
            kc(
                "Iterator.enumerate() [in-pass]",
                "ITERATOR_METHODS",
                "enumerate",
                it,
                vec![iter_range()],
            ),
            kc(
                "Iterator.chain(other) [in-pass]",
                "ITERATOR_METHODS",
                "chain",
                it,
                vec![iter_range(), iter_range()],
            ),
        ];
        run_cases("method_return_kind_from_in_pass_kinds", cases);

        // Closure-arg adapters: cannot be invoked without a real closure
        // block, but the claim + registration must both still exist, and
        // their return statement is the same `append_transform` ->
        // `wrap_iterator` path pinned by take/skip/enumerate/chain above.
        for name in ["map", "filter", "flatMap"] {
            assert_eq!(
                claim(name, it),
                Some(NativeKind::Ptr(HeapKind::Iterator)),
                "JIT in-pass table lost the Iterator.{name} adapter arm"
            );
            assert!(
                reg::ITERATOR_METHODS.get(name).is_some(),
                "ITERATOR_METHODS no longer registers `{name}`"
            );
        }
    }

    // ── Test 4: the two JIT-side iterator-adapter classifiers agree ─────

    #[test]
    fn iterator_adapter_classifiers_agree() {
        let args = recv_operands();
        let kinds = vec![Some(NativeKind::Ptr(HeapKind::Iterator))];
        for name in [
            "map",
            "filter",
            "take",
            "skip",
            "flatMap",
            "enumerate",
            "chain",
        ] {
            assert_eq!(
                iterator_adapter_return_kind(name, &args, &kinds),
                method_return_kind_from_in_pass_kinds(name, &args, &kinds),
                "`iterator_adapter_return_kind` and \
                 `method_return_kind_from_in_pass_kinds` disagree on `{name}`"
            );
        }
        // Non-Iterator receivers must classify to None in both.
        let non_iter = vec![Some(NativeKind::Int64)];
        for name in [
            "map",
            "filter",
            "take",
            "skip",
            "flatMap",
            "enumerate",
            "chain",
        ] {
            assert_eq!(iterator_adapter_return_kind(name, &args, &non_iter), None);
            assert_eq!(
                method_return_kind_from_in_pass_kinds(name, &args, &non_iter),
                None
            );
        }
    }
}
