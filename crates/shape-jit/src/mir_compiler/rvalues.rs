//! Rvalue compilation: MIR Rvalue → Cranelift IR.
//!
//! Maps each Rvalue variant to Cranelift instructions:
//! - Use(operand): ownership-aware value load
//! - BinaryOp: arithmetic, comparison, logical operators
//! - UnaryOp: negation, logical not
//! - Clone: explicit clone (arc_retain)
//! - Borrow: reference creation (deferred)
//! - Aggregate: array/object construction

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile an Rvalue to a Cranelift value.
    pub(crate) fn compile_rvalue(&mut self, rvalue: &Rvalue) -> Result<Value, String> {
        match rvalue {
            Rvalue::Use(operand) => self.compile_operand(operand),

            Rvalue::BinaryOp(op, lhs, rhs) => {
                // Check source operand kinds BEFORE compiling (needed for I64 disambiguation).
                let lhs_kind = self.operand_slot_kind(lhs);
                let rhs_kind = self.operand_slot_kind(rhs);

                let l = self.compile_operand(lhs)?;
                let r = self.compile_operand(rhs)?;

                // Check operand types for native inline paths.
                let l_type = self.builder.func.dfg.value_type(l);
                let r_type = self.builder.func.dfg.value_type(r);

                // F5.a/F5.b: string `+` — concat via FFI. Either operand being a
                // `NativeKind::String` is enough; the FFI handles `str + <any>` by
                // falling back to `format_value_word` on non-string operands,
                // which matches the lowering emitted by f-string interpolation.
                if matches!(op, BinOp::Add) && self.either_string(lhs_kind, rhs_kind) {
                    return self.compile_string_concat(l, r);
                }

                if l_type == types::F64 && r_type == types::F64 {
                    // Both operands are native F64 — inline float ops.
                    self.compile_binop_f64(op, l, r)
                } else if l_type == types::I32 && r_type == types::I32 {
                    // Both operands are native I32 — inline i32 ops.
                    match op {
                        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                            self.compile_binop_i32_native(op, l, r)
                        }
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le
                        | BinOp::Gt | BinOp::Ge => {
                            self.compile_cmp_i32_native(op, l, r)
                        }
                        _ => self.compile_binop(op, l, r),
                    }
                } else if l_type == types::I8 && r_type == types::I8 {
                    // Both operands are native I8 (Bool) — inline bool ops.
                    self.compile_binop_bool(op, l, r)
                } else if self.both_int64(lhs_kind, rhs_kind) {
                    // Both operands are Int64 slots (NaN-boxed ints) — inline i64 arithmetic.
                    // Extract 48-bit payload, operate natively, re-box.
                    self.compile_binop_int64(op, l, r)
                } else {
                    // Mixed or unknown types — use FFI generic path.
                    self.compile_binop(op, l, r)
                }
            }

            Rvalue::UnaryOp(op, operand) => {
                let val = self.compile_operand(operand)?;
                self.compile_unop(op, val)
            }

            Rvalue::Clone(operand) => {
                // Explicit clone: get the value and (if heap-kinded) retain.
                //
                // W11-jit-new-array (ADR-006 §2.7.5 / §2.7.6 / Q8): the
                // pre-W11 unconditional retain here was the symmetric
                // version of the `compile_operand` Copy bug — fired on
                // every Clone regardless of kind, which segfaulted on
                // `NativeKind::Int64` slots whose bits are a raw int
                // (the `MIR-emits-Clone-on-non-heap` case the W-series
                // ABI tolerated via tag-bit decode). The principled
                // response is to use the same kind-aware disposition
                // path as Copy. When the operand has no `Place::Local`
                // (e.g. `Operand::Constant`), there's no slot to
                // discriminate by — and the bytecode compiler does not
                // emit `Rvalue::Clone(Constant(...))` (Clone is by
                // construction a place-rooted operation), so the
                // fallback arm surface-and-stops with a clear marker.
                let val = self.compile_operand_raw(operand)?;
                let place = match operand {
                    shape_vm::mir::types::Operand::Copy(p)
                    | shape_vm::mir::types::Operand::Move(p)
                    | shape_vm::mir::types::Operand::MoveExplicit(p) => p,
                    shape_vm::mir::types::Operand::Constant(_) => {
                        return Err(
                            "MirToIR: Rvalue::Clone(Constant) — Clone is \
                             defined on place-rooted operands per ADR-006 \
                             §2.7.5; emitter contract violated. SURFACE."
                                .to_string(),
                        );
                    }
                };
                if self.refcount_disposition_for_place(place)? {
                    self.builder.ins().call(self.ffi.arc_retain, &[val]);
                }
                Ok(val)
            }

            Rvalue::Borrow(_kind, place) => {
                // R4.2F: allocate a native-sized/aligned stack cell that
                // matches the root local's Cranelift type. References are
                // strictly per-function — they never cross Cranelift call
                // boundaries — so picking a native width here is safe and
                // removes the width-extension wrap/unwrap pair.
                //
                // For non-native slot kinds (heap / string / unknown),
                // `cranelift_type_for_slot` returns I64, collapsing to the
                // legacy 8-byte cell with no behavioural change.
                let raw_val = self.read_place(place)?;
                let root = place.root_local();
                let kind = super::types::slot_kind_for_local(&self.slot_kinds, root.0)
                    .unwrap_or(shape_vm::type_tracking::NativeKind::Int64);
                let cl_ty = super::types::cranelift_type_for_slot(kind);
                let size = cl_ty.bytes();
                // `create_sized_stack_slot` takes the log2 of the alignment;
                // `trailing_zeros` of a power-of-two size is exactly that.
                let align_shift = size.trailing_zeros() as u8;
                let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size,
                    align_shift,
                ));
                // Store the value at its native width — no NaN-box wrap.
                self.builder.ins().stack_store(raw_val, slot, 0);
                // Track root local + native type for reload-after-call.
                self.ref_stack_slots.insert(root, (slot, cl_ty));
                // Return the stack slot address as the reference value.
                Ok(self.builder.ins().stack_addr(types::I64, slot, 0))
            }

            Rvalue::Aggregate(_operands) => {
                // Route A (ADR-006 §2.7.14 / W11-jit-new-array close):
                // typed-array allocation is kind-monomorphized on the
                // destination slot's element kind. The kind-blind
                // `jit_new_array` + `jit_array_push_elem` ABI was the
                // deleted ValueWord-shape path.
                //
                // statements.rs's `StatementKind::Assign` handler short-
                // circuits to `emit_v2_array_aggregate` (which calls
                // `jit_v2_array_new_<kind>` directly) when the destination
                // place has a proven scalar element kind via
                // `v2_typed_array_elem_kind`. Reaching this fallback means
                // the destination place could not be resolved to a typed-
                // array slot — either the destination isn't a local, or
                // the local's `ConcreteType` lacks the `Array<T>` shape
                // the v2 fast path requires.
                //
                // Per §2.7.14 forbidden list ("Bool-default fallback for
                // unknown element kinds") the correct response is
                // surface-and-stop; falling back to a kind-blind allocator
                // would resurrect the deleted UnifiedArray heap layout.
                Err(
                    "Route A surface-and-stop: SURFACE — Rvalue::Aggregate \
                     reached the kind-blind fallback. The v2 typed-array \
                     fast path in statements.rs requires the destination \
                     `Place::Local` to carry a `ConcreteType::Array<scalar>`; \
                     reaching here means the element kind is not threaded \
                     from the producing call signature. Tracked as \
                     W11-jit-new-array per phase-3-kickoff-prompt.md. \
                     ADR-006 §2.7.14 / §2.7.5."
                        .to_string(),
                )
            }

            Rvalue::EnumTest { operand, variant } => {
                // ADR-006 §2.7.17 / Q18 (W12-jit-result-option-trinity,
                // Phase 3 cluster-0 Round 7A, 2026-05-12). The operand is
                // an `Arc::into_raw(Arc<ResultData>) as u64` or
                // `Arc::into_raw(Arc<OptionData>) as u64` slot per the
                // §2.7.7 stack-tier kind label; the FFI accessor reads
                // `is_ok` / `is_some` from the `*const T` directly. NOT a
                // NaN-box tag decode (§2.7.7 #4 / #7 forbidden), NOT a
                // generic SwitchBool fallthrough — kind-aware codegen per
                // the audit blueprint at
                // `docs/cluster-audits/w12-jit-match-enum-inline-audit.md`
                // §6.1.
                let bits = self.compile_operand_raw(operand)?;
                let bits_i64 = self.to_i64_bits(bits);
                let func_ref = match variant {
                    VariantTag::Ok => self.ffi.arc_result_is_ok,
                    VariantTag::Err => self.ffi.arc_result_is_err,
                    VariantTag::Some_ => self.ffi.arc_option_is_some,
                    VariantTag::None_ => self.ffi.arc_option_is_none,
                };
                let inst = self.builder.ins().call(func_ref, &[bits_i64]);
                // FFI returns I8 (native bool). Caller's destination slot
                // kind is `Bool` per `infer_rvalue_kind`'s EnumTest arm.
                Ok(self.builder.inst_results(inst)[0])
            }

            Rvalue::EnumPayload { operand, variant } => {
                // ADR-006 §2.7.17 / Q18 (W12-jit-result-option-trinity).
                // Caller has proven the variant matches via `EnumTest` and
                // control-flow only enters this arm in the matching branch.
                // The FFI clones the inner KindedSlot's share (per §2.7.17
                // receiver-recovery soundness) and returns the raw bits as
                // an owned slot at the caller's destination. Payload kind
                // flows via the EnumStore producer's compile-time stamp +
                // 6A's call-return-kind track (the destination slot's kind
                // is set at MIR-inference time, not at this codegen site).
                //
                // `VariantTag::None_` here is a producer-side bug — the
                // None arm has no payload to extract. Surface-and-stop.
                let func_ref = match variant {
                    VariantTag::Ok | VariantTag::Err => self.ffi.arc_result_payload,
                    VariantTag::Some_ => self.ffi.arc_option_payload,
                    VariantTag::None_ => {
                        return Err(
                            "EnumPayload: SURFACE — VariantTag::None_ has no \
                             payload to extract per ADR-006 §2.7.17 \
                             `OptionData::none()` (placeholder Bool slot). \
                             The MIR producer in \
                             `lower_constructor_bindings_from_place_opt` \
                             must not emit `EnumPayload { variant: None_ }`. \
                             Producer-site contract violated."
                                .to_string(),
                        );
                    }
                };
                let bits = self.compile_operand_raw(operand)?;
                let bits_i64 = self.to_i64_bits(bits);
                let inst = self.builder.ins().call(func_ref, &[bits_i64]);
                Ok(self.builder.inst_results(inst)[0])
            }
        }
    }

    // ── Operand kind helpers ───────────────────────────────────────

    /// Get the NativeKind of an operand's source, falling back to the
    /// documented §2.7.5 stable-FFI carrier kind `NativeKind::UInt64` when
    /// the producing-site inference left the slot kind undetermined.
    ///
    /// ADR-006 §2.7.5 designates `UInt64` as the "I64-wide raw bits without
    /// further classification" carrier kind — the same kind
    /// `dispatch_call_via_trampoline_vm` stamps for function-id-class
    /// callees and for I64-widened args at the JIT-FFI boundary. It is
    /// NOT a Bool-default rationalization (§2.7.7 #9 / CLAUDE.md
    /// "Forbidden rationalizations"); `UInt64` is the documented carrier
    /// kind for the bit-pattern the JIT actually pushes onto the stack
    /// (every operand widens to I64 before the push per terminators.rs
    /// R4.2E inline-widening discipline).
    ///
    /// Precise kinds — `Ptr(HeapKind::Closure)` for closure slots seeded by
    /// `infer_slot_kinds::ClosureCapture`, `Float64` / `Bool` / etc. for
    /// inferred scalar slots — flow through unchanged. The fallback only
    /// applies to slots whose producing-site is opaque to MIR inference
    /// (field reads through heap projections, opaque-source calls, etc.)
    /// — in those cases the value IS I64-wide raw bits by construction,
    /// and `UInt64` is the structurally-correct §2.7.5 carrier kind.
    ///
    /// For the load-bearing closure-callee classification at
    /// `jit_call_value`'s indirect-call entry, the §2.7.11/Q12 dispatch
    /// requires precise `Ptr(HeapKind::Closure)` kinds — seeded via
    /// `infer_slot_kinds`'s ClosureCapture arm. The `UInt64` fallback at
    /// other push sites preserves the existing JIT-internal NaN-box
    /// bit-shape dispatch path inside `jit_call_value` (cases 1 / 2 —
    /// inline `TAG_FUNCTION` function refs and legacy `HK_CLOSURE`
    /// unified-heap callees).
    #[allow(dead_code)]
    pub(crate) fn operand_slot_kind_or_carrier(
        &self,
        operand: &Operand,
    ) -> shape_value::NativeKind {
        self.operand_slot_kind(operand)
            .unwrap_or(shape_value::NativeKind::UInt64)
    }

    /// Get the NativeKind of an operand's source (before compilation).
    ///
    /// ADR-006 §2.7.5 / §2.7.11: the producing site classifies the operand
    /// kind at JIT-compile time. Function refs widen to the documented
    /// `NativeKind::UInt64` carrier kind (the §2.7.11/Q12 function-id-class
    /// callee-classification kind, also used as the "I64-wide raw bits
    /// carrier" sentinel at the §2.7.5 stable-FFI boundary). Method-name
    /// constants are heap String pointers (kind = `NativeKind::String`).
    /// String and StringId constants are likewise heap String pointers.
    pub(crate) fn operand_slot_kind(&self, operand: &Operand) -> Option<shape_vm::type_tracking::NativeKind> {
        use shape_vm::type_tracking::NativeKind;
        match operand {
            Operand::Constant(MirConstant::Int(_)) => Some(NativeKind::Int64),
            Operand::Constant(MirConstant::Float(_)) => Some(NativeKind::Float64),
            Operand::Constant(MirConstant::Bool(_)) => Some(NativeKind::Bool),
            // ADR-006 §2.7.11/Q12 function-id-class callee-classification
            // kind: a `MirConstant::Function(name)` lowers to the JIT-
            // internal `box_function(fn_id)` shape (TAG_FUNCTION NaN-box),
            // whose carrier kind across the §2.7.5 stable-FFI boundary is
            // `NativeKind::UInt64`. The trampoline VM consumer
            // (`dispatch_call_via_trampoline_vm`) classifies this same
            // kind as the function-id callee per `call_convention.rs`
            // UInt64 arm.
            Operand::Constant(MirConstant::Function(_)) => Some(NativeKind::UInt64),
            // Method-name string constant. The JIT emits a heap String
            // pointer via `box_string`; carrier kind is `String` (the
            // §2.7.5 String arm — `Arc<String>` raw pointer carrier).
            Operand::Constant(MirConstant::Method(_)) => Some(NativeKind::String),
            // String constants and string-id constants both materialize
            // as heap `Arc<String>` raw pointers; carrier kind is String.
            Operand::Constant(MirConstant::Str(_)) => Some(NativeKind::String),
            Operand::Constant(MirConstant::StringId(_)) => Some(NativeKind::String),
            // ClosurePlaceholder is the producing-site forward-reference
            // for closures whose function_id is patched later. The slot
            // it lowers to carries `Arc<HeapValue::ClosureRaw>` bits per
            // §2.7.11/Q12.
            Operand::Constant(MirConstant::ClosurePlaceholder) => Some(NativeKind::Ptr(
                shape_value::heap_value::HeapKind::Closure,
            )),
            Operand::Constant(MirConstant::None) => None,
            Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => {
                // Centralized projection: `place_native_kind` handles
                // both Round 5A's Field projection (via
                // `field_native_kinds`) AND Round 5C's Index projection
                // (via `v2_typed_array_elem_kind` → `concrete_types`'s
                // `Array<scalar>` shape) in a single helper that
                // `ownership::refcount_disposition` also shares.
                self.place_native_kind(p)
            }
        }
    }

    /// Project a `Place` to the `NativeKind` of the value it produces at
    /// the consumer site, per ADR-006 §2.7.5 stamp-at-compile-time
    /// discipline (W12-jit-binop-after-heap-read-kind-tracker close).
    ///
    /// - `Place::Local(slot)`: read the slot's MIR-inferred kind from
    ///   `slot_kinds`.
    /// - `Place::Field(base, field_idx)`: look up the field name via
    ///   `field_name_table`, then the per-field kind in
    ///   `field_native_kinds` — populated by the producer-side
    ///   `StatementKind::ObjectStore` walk at MirToIR construction time.
    ///   This threads the producer's kind classification across the
    ///   TypedObject field-read projection without runtime tag-bit
    ///   decode (§2.7.7 #4 / #7 forbidden).
    /// - `Place::Index(base, _)`: when the base local's `ConcreteType`
    ///   is `Array<scalar>` (per the W12-top-level-concrete-types-
    ///   conduit close), project to the element's `NativeKind` via
    ///   `v2_typed_array_elem_kind`. This is the same kind the v2
    ///   `read_place` fast path uses to load the element at its native
    ///   width. Same projection the W12-jit-print-kind (Round 5C) sub-
    ///   cluster needs at the `print(xs[0])` dispatch site.
    /// - `Place::Deref(_)`: not stamped — references are heap-tier
    ///   indirection and the type-of-pointed-to-value is not threaded
    ///   into the JIT-side projection map yet. Returns `None` so the
    ///   BinaryOp lowering surfaces honestly rather than papering.
    ///
    /// Returns `None` when no proof exists at this consumer site;
    /// callers in `compile_rvalue` then choose between surface-and-stop
    /// (the dynamic-arith / dynamic-cmp arms) and continuing through the
    /// `UInt64` carrier fallback in `operand_slot_kind_or_carrier`.
    ///
    /// `pub(crate)` so `ownership::refcount_disposition` can project
    /// through `Field` / `Index` to decide retain/release on the value
    /// being copied — the value's kind is the field's / element's kind,
    /// not the base struct/array's heap kind. This closes the segfault
    /// where `Copy(Field(p_TypedObject, x_Int64))` previously routed
    /// through the base's heap retain and called `arc_retain(i64_3)`.
    pub(crate) fn place_native_kind(&self, place: &Place) -> Option<shape_vm::type_tracking::NativeKind> {
        match place {
            Place::Local(slot) => {
                super::types::slot_kind_for_local(&self.slot_kinds, slot.0)
            }
            Place::Field(_, field_idx) => {
                let name = self.mir.field_name_table.get(field_idx)?;
                self.field_native_kinds.get(name).copied()
            }
            Place::Index(base, _) => {
                // The v2 typed-array element-kind helper takes a Place
                // and reads `concrete_types[base.root_local()]`. It is
                // the same source the `read_place` fast path uses to
                // pick the native-width load width for the element —
                // pairing the producer-side kind classification with the
                // consumer-side BinaryOp picker.
                self.v2_typed_array_elem_kind(base)
            }
            Place::Deref(_) => None,
        }
    }

    /// Check if both operand kinds are Int64 (NaN-boxed integers suitable for inline i64 ops).
    fn both_int64(
        &self,
        lhs: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Option<shape_vm::type_tracking::NativeKind>,
    ) -> bool {
        matches!(
            (lhs, rhs),
            (
                Some(shape_vm::type_tracking::NativeKind::Int64),
                Some(shape_vm::type_tracking::NativeKind::Int64)
            )
        )
    }

    /// F5.a/F5.b: true if either operand kind is `NativeKind::String`. The MIR
    /// emits `BinOp::Add` on heterogeneous operand types for f-string
    /// interpolation (e.g. `str + number + str`) — the FFI's non-string
    /// fallback (`format_value_word`) does the rest.
    fn either_string(
        &self,
        lhs: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Option<shape_vm::type_tracking::NativeKind>,
    ) -> bool {
        matches!(lhs, Some(shape_vm::type_tracking::NativeKind::String))
            || matches!(rhs, Some(shape_vm::type_tracking::NativeKind::String))
    }

    /// F5.a/F5.b: emit a call to `jit_string_concat(a_bits, b_bits) -> bits`.
    ///
    /// Both operand `Value`s must be widened to I64 bit-patterns (the FFI
    /// signature expects two `i64` params). This handles the cases where the
    /// MIR lowering produced a native-typed constant for one side — e.g.
    /// `f"x={n}"` where `n: int` is `NativeKind::Int64` (I64 bits already) or
    /// a plain number constant (F64, must bitcast to I64).
    fn compile_string_concat(
        &mut self,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        let a = self.to_i64_bits(lhs);
        let b = self.to_i64_bits(rhs);
        let inst = self.builder.ins().call(self.ffi.string_concat, &[a, b]);
        Ok(self.builder.inst_results(inst)[0])
    }

    // ── Inline Float64 arithmetic and comparisons ──────────────────

    /// Compile a binary op on native F64 operands — direct Cranelift float instructions.
    /// ~100x faster per operation vs FFI generic_add/etc.
    fn compile_binop_f64(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Add => Ok(self.builder.ins().fadd(lhs, rhs)),
            BinOp::Sub => Ok(self.builder.ins().fsub(lhs, rhs)),
            BinOp::Mul => Ok(self.builder.ins().fmul(lhs, rhs)),
            BinOp::Div => Ok(self.builder.ins().fdiv(lhs, rhs)),
            BinOp::Mod => {
                // f64 mod: a % b = a - trunc(a/b) * b (pure Cranelift, no FFI)
                let div = self.builder.ins().fdiv(lhs, rhs);
                let truncated = self.builder.ins().trunc(div);
                let product = self.builder.ins().fmul(truncated, rhs);
                Ok(self.builder.ins().fsub(lhs, product))
            }
            BinOp::Eq => {
                let cmp = self.builder.ins().fcmp(FloatCC::Equal, lhs, rhs);
                // fcmp returns I8 (native bool) — this is fine for Bool slots
                Ok(cmp)
            }
            BinOp::Ne => {
                let cmp = self.builder.ins().fcmp(FloatCC::NotEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Lt => {
                let cmp = self.builder.ins().fcmp(FloatCC::LessThan, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Le => {
                let cmp = self.builder.ins().fcmp(FloatCC::LessThanOrEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Gt => {
                let cmp = self.builder.ins().fcmp(FloatCC::GreaterThan, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Ge => {
                let cmp = self
                    .builder
                    .ins()
                    .fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::And | BinOp::Or => {
                // Logical ops on floats — box and use generic path
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    // ── Native I32 arithmetic (no ireduce/sextend needed) ───────────

    /// Compile i32 binary arithmetic on native I32 values (no boxing overhead).
    fn compile_binop_i32_native(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Add => Ok(self.builder.ins().iadd(lhs, rhs)),
            BinOp::Sub => Ok(self.builder.ins().isub(lhs, rhs)),
            BinOp::Mul => Ok(self.builder.ins().imul(lhs, rhs)),
            BinOp::Div => {
                let zero = self.builder.ins().iconst(types::I32, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, rhs, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                Ok(self.builder.ins().sdiv(lhs, rhs))
            }
            BinOp::Mod => {
                let zero = self.builder.ins().iconst(types::I32, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, rhs, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                Ok(self.builder.ins().srem(lhs, rhs))
            }
            _ => Err(format!("unsupported native i32 binop: {:?}", op)),
        }
    }

    /// Compile i32 comparison on native I32 values — returns I8 (native bool).
    fn compile_cmp_i32_native(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        let cc = match op {
            BinOp::Eq => IntCC::Equal,
            BinOp::Ne => IntCC::NotEqual,
            BinOp::Lt => IntCC::SignedLessThan,
            BinOp::Le => IntCC::SignedLessThanOrEqual,
            BinOp::Gt => IntCC::SignedGreaterThan,
            BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
            _ => return Err(format!("unsupported native i32 cmp: {:?}", op)),
        };
        // icmp returns I8 (native bool)
        Ok(self.builder.ins().icmp(cc, lhs, rhs))
    }

    // ── Inline Int64 arithmetic (raw native i64) ──────────────────

    /// Compile a binary op on proven `NativeKind::Int64` operands.
    ///
    /// Per ADR-006 §2.7.5 the JIT slots are raw native bits with the kind
    /// stamped on the parallel JitFfiCarrier companion — Int64 slots hold
    /// raw i64 values, not `tag_bits` payloads. Inputs and the output flow
    /// through unchanged: no payload extraction, no re-box.
    fn compile_binop_int64(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let result = match op {
                    BinOp::Add => self.builder.ins().iadd(lhs, rhs),
                    BinOp::Sub => self.builder.ins().isub(lhs, rhs),
                    BinOp::Mul => self.builder.ins().imul(lhs, rhs),
                    BinOp::Div => {
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let is_zero = self.builder.ins().icmp(IntCC::Equal, rhs, zero);
                        self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                        self.builder.ins().sdiv(lhs, rhs)
                    }
                    BinOp::Mod => {
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let is_zero = self.builder.ins().icmp(IntCC::Equal, rhs, zero);
                        self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                        self.builder.ins().srem(lhs, rhs)
                    }
                    _ => unreachable!(),
                };
                Ok(result)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let cc = match op {
                    BinOp::Eq => IntCC::Equal,
                    BinOp::Ne => IntCC::NotEqual,
                    BinOp::Lt => IntCC::SignedLessThan,
                    BinOp::Le => IntCC::SignedLessThanOrEqual,
                    BinOp::Gt => IntCC::SignedGreaterThan,
                    BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
                    _ => unreachable!(),
                };
                let cmp = self.builder.ins().icmp(cc, lhs, rhs);
                // icmp returns I8 (native bool)
                Ok(cmp)
            }
            _ => {
                // Logical ops — use FFI path
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    // ── Native Bool operations ──────────────────────────────────────

    /// Compile a binary op on native I8 (Bool) operands.
    fn compile_binop_bool(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Eq => Ok(self.builder.ins().icmp(IntCC::Equal, lhs, rhs)),
            BinOp::Ne => Ok(self.builder.ins().icmp(IntCC::NotEqual, lhs, rhs)),
            BinOp::And => Ok(self.builder.ins().band(lhs, rhs)),
            BinOp::Or => Ok(self.builder.ins().bor(lhs, rhs)),
            _ => {
                // Other ops on bools — box and use generic path
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    /// Compile a binary operation on a dynamic (NaN-boxed) slot.
    ///
    /// R7.1: After R5.1–R5.6 retargeted all dynamic arithmetic /
    /// comparison fallbacks (typed bitwise, user operator traits,
    /// DateTime, Matrix/Vec, string+scalar) to typed opcodes or
    /// `CallMethod`, the JIT no longer receives fully dynamic
    /// arithmetic / comparison binops from MIR. The `generic_*`
    /// FFI trampolines (`generic_add`/`sub`/`mul`/`div`/`mod`,
    /// `generic_eq`/`neq`, `generic_lt`/`le`/`gt`/`ge`) were the
    /// last things pinning those FuncRefs alive and have been
    /// removed in this commit.
    ///
    /// This helper remains for the `BinOp::And` / `BinOp::Or`
    /// fallthroughs from `compile_binop_f64`, `compile_binop_int64`,
    /// and `compile_binop_bool` where the logical op mixes with a
    /// NaN-boxed bool encoding (TAG_BOOL_TRUE / TAG_BOOL_FALSE).
    ///
    /// Session 2: Dynamic arithmetic binops from CallValue-returned
    /// slots (closure calls whose return type isn't provable at MIR
    /// level) are lowered via an inline NaN-box dispatch — `Both-Number`
    /// (hot path: `!is_tagged(l) && !is_tagged(r)` → native fadd/etc.) or
    /// `Both-Int` (`is_tagged_int(l) && is_tagged_int(r)` → i48 math).
    /// Mixed or heap operands trap the JIT function, triggering an
    /// error-signal return that the caller observes via the deopt
    /// pathway. This preserves `no generic_* FFI` while keeping
    /// closure-return-arith JIT-compilable.
    fn compile_binop(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // Widen native-typed operands into their NaN-boxed I64 bit-pattern so
        // the dynamic dispatch helpers can treat both uniformly. This handles
        // the mixed cases (e.g. F64 literal vs I64 NaN-boxed heap handle)
        // that `compile_rvalue` routes here after the typed fast paths.
        let l = self.to_i64_bits(lhs);
        let r = self.to_i64_bits(rhs);
        match op {
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Mod => self.compile_binop_dynamic_arith(op, l, r),

            BinOp::Eq
            | BinOp::Ne
            | BinOp::Lt
            | BinOp::Le
            | BinOp::Gt
            | BinOp::Ge => self.compile_binop_dynamic_cmp(op, l, r),

            // v2-boundary: logical ops on NaN-boxed values use TAG_BOOL_TRUE/FALSE
            BinOp::And => {
                let tag_true = self.builder.ins().iconst(
                    types::I64,
                    1i64,
                );
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, l, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, r, tag_true);
                let both = self.builder.ins().band(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    0i64,
                );
                Ok(self.builder.ins().select(both, tag_true, false_val))
            }
            BinOp::Or => {
                let tag_true = self.builder.ins().iconst(
                    types::I64,
                    1i64,
                );
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, l, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, r, tag_true);
                let either = self.builder.ins().bor(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(
                    types::I64,
                    0i64,
                );
                Ok(self.builder.ins().select(either, tag_true, false_val))
            }
        }
    }

    // ── Session 2: Dynamic arith / cmp inline NaN-box dispatch ────────

    /// Widen an operand Value to its NaN-boxed I64 bit-pattern.
    ///
    /// - `F64` → bitcast to `I64` (the f64 bit-pattern *is* the NaN-box payload
    ///   because plain numbers have sign=0).
    /// - `I32` / `I16` → sign-extend to `I64`. NaN-boxed int slots use
    ///   `TAG_INT | (i48_payload_mask & value)` upstream; narrow-int slots
    ///   reaching `compile_binop` are rare (the native-I32 fast path catches
    ///   both-I32 already), so this conservative sign-extend keeps the raw
    ///   integer value visible to the dynamic dispatch's `int` branch.
    /// - `I8` (native bool) → zero-extend to `I64`. The logical-op branches of
    ///   `compile_binop` compare against the literal `1i64` ⇔ `TAG_BOOL_TRUE`
    ///   encoding, so widening to I64 preserves truth semantics.
    /// - `I64` → passed through unchanged.
    fn to_i64_bits(&mut self, v: Value) -> Value {
        let ty = self.builder.func.dfg.value_type(v);
        if ty == types::I64 {
            v
        } else if ty == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), v)
        } else if ty == types::I32 || ty == types::I16 {
            self.builder.ins().sextend(types::I64, v)
        } else if ty == types::I8 {
            self.builder.ins().uextend(types::I64, v)
        } else {
            v
        }
    }


    /// Compile a dynamic-operand arithmetic binop (Add/Sub/Mul/Div/Mod).
    ///
    /// Per ADR-006 §2.7.5 + CLAUDE.md "Forbidden code" (`tag_bits` runtime
    /// dispatch deleted): every operand has a proven `NativeKind` at MIR
    /// compile time. The pre-strict-typing W-series IC body branched on
    /// `tag_bits` to discriminate `Number` vs `TAG_INT` operand bits at
    /// runtime — that path no longer exists. Reaching this site indicates
    /// a producing-MIR kind-tracker gap; surface-and-stop per W10
    /// playbook §5 so the gap is fixed at the producing opcode rather
    /// than papered over with the deleted W-series tag-bit IC.
    fn compile_binop_dynamic_arith(
        &mut self,
        op: &BinOp,
        _lhs: Value,
        _rhs: Value,
    ) -> Result<Value, String> {
        Err(format!(
            "compile_binop_dynamic_arith: kind-untyped arith {:?} reached the JIT — \
             SURFACE per W10 playbook §5: producing-MIR kind-tracker gap; \
             every JIT operand must have a proven NativeKind at compile time \
             (ADR-006 §2.7.5 / CLAUDE.md \"Forbidden code\" — runtime tag_bits \
             dispatch deleted with the W-series IC).",
            op
        ))
    }

    /// Compile a dynamic-operand comparison binop (Eq/Ne/Lt/Le/Gt/Ge).
    ///
    /// Per ADR-006 §2.7.5 + CLAUDE.md "Forbidden code" (`tag_bits` runtime
    /// dispatch deleted): every operand has a proven `NativeKind` at MIR
    /// compile time. The pre-strict-typing W-series body branched on
    /// `tag_bits` to discriminate `Number` / `TAG_INT` / mixed operand
    /// bits at runtime — that path no longer exists. Eq/Ne is preserved
    /// as raw bitwise compare (kind-mismatched bits are unequal by
    /// construction); Lt/Le/Gt/Ge surface-and-stop per W10 playbook §5
    /// because they require a kind-direction the producing-MIR
    /// kind-tracker must supply.
    fn compile_binop_dynamic_cmp(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // Bitwise Eq/Ne: any mismatched kind also means values are not equal.
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let cc = if matches!(op, BinOp::Eq) {
                IntCC::Equal
            } else {
                IntCC::NotEqual
            };
            return Ok(self.builder.ins().icmp(cc, lhs, rhs));
        }

        Err(format!(
            "compile_binop_dynamic_cmp: kind-untyped ordered cmp {:?} reached the JIT — \
             SURFACE per W10 playbook §5: producing-MIR kind-tracker gap; \
             every JIT operand must have a proven NativeKind at compile time \
             (ADR-006 §2.7.5 / CLAUDE.md \"Forbidden code\" — runtime tag_bits \
             dispatch deleted with the W-series IC).",
            op
        ))
    }

    /// Compile a unary operation.
    fn compile_unop(&mut self, op: &UnOp, val: Value) -> Result<Value, String> {
        let val_type = self.builder.func.dfg.value_type(val);
        match op {
            UnOp::Neg => {
                if val_type == types::F64 {
                    // Native F64: direct fneg
                    Ok(self.builder.ins().fneg(val))
                } else {
                    // NaN-boxed: bitcast to F64, negate, bitcast back
                    let f64_val = self.builder.ins().bitcast(types::F64, MemFlags::new(), val);
                    let neg = self.builder.ins().fneg(f64_val);
                    Ok(self.builder.ins().bitcast(types::I64, MemFlags::new(), neg))
                }
            }
            UnOp::Not => {
                if val_type == types::I8 {
                    // Native I8 bool: XOR with 1 to flip
                    let one = self.builder.ins().iconst(types::I8, 1);
                    Ok(self.builder.ins().bxor(val, one))
                } else {
                    // v2-boundary: NaN-boxed bool uses TAG_BOOL_TRUE/FALSE tags
                    let tag_true = self.builder.ins().iconst(
                        types::I64,
                        1i64,
                    );
                    let false_val = self.builder.ins().iconst(
                        types::I64,
                        0i64,
                    );
                    let is_true = self.builder.ins().icmp(IntCC::Equal, val, tag_true);
                    Ok(self.builder.ins().select(is_true, false_val, tag_true))
                }
            }
        }
    }
}
