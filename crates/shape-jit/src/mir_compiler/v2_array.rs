//! Inline typed array codegen for the v2 runtime.
//!
//! Emits Cranelift IR for direct-memory-access typed array operations
//! with zero FFI overhead and zero NaN-boxing.
//!
//! ## TypedArrayHeader layout (at the array pointer)
//!
//! ```text
//! offset  0: refcount  (u32)
//! offset  4: kind      (u16)
//! offset  6: elem_type (u8)
//! offset  7: _pad      (u8)
//! offset  8: data      (*mut T)  — pointer to contiguous element buffer
//! offset 16: len       (u32)
//! offset 20: cap       (u32)
//! ```
//!
//! ## Element sizes
//!
//! | NativeKind  | Cranelift type | Size (bytes) |
//! |-----------|---------------|--------------|
//! | Float64   | F64           | 8            |
//! | Int64     | I64           | 8            |
//! | Int32     | I32           | 4            |
//! | Int16     | I16           | 2            |
//! | Int8/Bool | I8            | 1            |

use cranelift::prelude::*;
use shape_value::v2::ConcreteType;
use shape_vm::mir::types::{Operand, Place, SlotId};
use shape_vm::type_tracking::NativeKind;

use super::MirToIR;
use super::types::is_v2_typed_array_slot;

// ── TypedArrayHeader field offsets ───────────────────────────────────────────

/// Offset of the `data` pointer field (`*mut T`) inside `TypedArrayHeader`.
const DATA_PTR_OFFSET: i32 = 8;

/// Offset of the `len` field (`u32`) inside `TypedArrayHeader`.
const LEN_OFFSET: i32 = 16;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Return the (Cranelift IR type, element byte size) for a given `NativeKind`.
///
/// Panics on slot kinds that do not map to a scalar element type (e.g.
/// `String`, `Dynamic`, `Unknown`).
fn elem_type_info(kind: NativeKind) -> (types::Type, i64) {
    match kind {
        NativeKind::Float64 | NativeKind::NullableFloat64 => (types::F64, 8),
        NativeKind::Int64 | NativeKind::NullableInt64 | NativeKind::UInt64 | NativeKind::NullableUInt64 => {
            (types::I64, 8)
        }
        NativeKind::IntSize | NativeKind::NullableIntSize | NativeKind::UIntSize | NativeKind::NullableUIntSize => {
            // Pointer-sized — 8 bytes on 64-bit targets.
            (types::I64, 8)
        }
        NativeKind::Int32 | NativeKind::NullableInt32 | NativeKind::UInt32 | NativeKind::NullableUInt32 => {
            (types::I32, 4)
        }
        NativeKind::Int16 | NativeKind::NullableInt16 | NativeKind::UInt16 | NativeKind::NullableUInt16 => {
            (types::I16, 2)
        }
        NativeKind::Int8 | NativeKind::NullableInt8 | NativeKind::UInt8 | NativeKind::NullableUInt8 => {
            (types::I8, 1)
        }
        NativeKind::Bool => (types::I8, 1),
        other => panic!("v2_array: unsupported element NativeKind: {:?}", other),
    }
}

/// Return the zero/default Cranelift constant for a given `NativeKind`.
///
/// Used as the out-of-bounds fallback value in `v2_array_get`.
fn emit_default(builder: &mut FunctionBuilder, kind: NativeKind) -> Value {
    let (ty, _) = elem_type_info(kind);
    match ty {
        types::F64 => builder.ins().f64const(0.0),
        types::I64 => builder.ins().iconst(types::I64, 0),
        types::I32 => builder.ins().iconst(types::I32, 0),
        types::I16 => builder.ins().iconst(types::I16, 0),
        types::I8 => builder.ins().iconst(types::I8, 0),
        _ => unreachable!(),
    }
}

// ── Implementation ──────────────────────────────────────────────────────────

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Look up the `ConcreteType` (if any) the bytecode compiler recorded for
    /// a local slot.
    #[allow(dead_code)]
    pub(crate) fn concrete_type_for_slot(&self, slot: SlotId) -> Option<&ConcreteType> {
        let ct = self.concrete_types.get(slot.0 as usize)?;
        if matches!(ct, ConcreteType::Void) {
            None
        } else {
            Some(ct)
        }
    }

    /// If the place's root local is known to hold a v2 `Array<T>` whose
    /// element type is a scalar primitive, return the matching element
    /// `NativeKind`. Returns `None` for non-array slots, arrays of non-scalar
    /// elements, or unresolved types — caller falls back to legacy path.
    ///
    /// Source: the per-MirToIR `concrete_types` vector, threaded from
    /// `BytecodeProgram.top_level_local_concrete_types` per ADR-006
    /// §2.7.5 (W12-top-level-concrete-types-conduit close, 2026-05-12).
    pub(crate) fn v2_typed_array_elem_kind(&self, place: &Place) -> Option<NativeKind> {
        let slot = match place {
            Place::Local(s) => *s,
            _ => return None,
        };
        is_v2_typed_array_slot(&self.concrete_types, slot.0)
    }

    /// True when the place's root local is known to hold a TypedObject
    /// (`ConcreteType::Struct(_)` / `ConcreteType::Enum(_)` /
    /// `ConcreteType::Option(_)` / `ConcreteType::Result(_, _)` /
    /// `ConcreteType::Tuple(_)`). These all share the `HeapKind::TypedObject`
    /// carrier and are materialised by the subsequent
    /// `StatementKind::ObjectStore` / `EnumStore`.
    ///
    /// Used by the `Assign(Aggregate)` short-circuit in `statements.rs`:
    /// when the bytecode compiler proved the destination slot is a
    /// TypedObject, the preceding `Rvalue::Aggregate` is a MIR scratch step
    /// — the real allocation happens in the following `ObjectStore`.
    /// Skipping the Aggregate avoids the `Route A surface-and-stop`
    /// previously hit at compile time for `Point { x, y }`-style literals.
    ///
    /// Source: the per-MirToIR `concrete_types` vector, threaded from
    /// `BytecodeProgram.top_level_local_concrete_types` per ADR-006
    /// §2.7.5 (W12-top-level-concrete-types-conduit close, 2026-05-12).
    pub(crate) fn is_typed_object_slot(&self, place: &Place) -> bool {
        let slot = match place {
            Place::Local(s) => *s,
            _ => return false,
        };
        let Some(ct) = self.concrete_types.get(slot.0 as usize) else {
            return false;
        };
        matches!(
            ct,
            ConcreteType::Struct(_)
                | ConcreteType::Enum(_)
                | ConcreteType::Option(_)
                | ConcreteType::Result(_, _)
                | ConcreteType::Tuple(_)
        )
    }

    /// Return the FFI `FuncRef` for `jit_v2_array_new_<elem>`.
    ///
    /// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
    /// extended with `StringV2` / `DecimalV2` arms routing to
    /// `jit_new_typed_array_string` / `jit_new_typed_array_decimal`. These
    /// allocate `TypedArray<*const StringObj>` / `TypedArray<*const
    /// DecimalObj>` carriers per ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED +
    /// audit deliverable (b) §4.1.B. Per-element pointer payload is the
    /// v2-raw heap-element shape produced by VM-side `NewStringV2` /
    /// `NewDecimalV2` opcodes at
    /// `crates/shape-vm/src/executor/v2_handlers/array.rs:803-858`.
    pub(crate) fn v2_array_new_func(&self, elem: NativeKind) -> Option<cranelift::codegen::ir::FuncRef> {
        match elem {
            NativeKind::Float64 => Some(self.ffi.v2_array_new_f64),
            NativeKind::Int64 | NativeKind::UInt64 => Some(self.ffi.v2_array_new_i64),
            NativeKind::Int32 | NativeKind::UInt32 => Some(self.ffi.v2_array_new_i32),
            NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => Some(self.ffi.v2_array_new_bool),
            NativeKind::StringV2 => Some(self.ffi.v2_array_new_string),
            NativeKind::DecimalV2 => Some(self.ffi.v2_array_new_decimal),
            _ => None,
        }
    }

    /// Return the element byte size for `NativeKind`s backed by the generic
    /// `jit_v2_array_push` dispatcher, or `None` for unsupported kinds. The
    /// caller uses the returned size as the `elem_size` I8 immediate passed
    /// to the dispatcher.
    ///
    /// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
    /// `StringV2` / `DecimalV2` are 8-byte pointer carriers — the element
    /// payload is a `*const StringObj` / `*const DecimalObj` raw pointer,
    /// pushed via the generic `jit_v2_array_push` I64-shaped dispatcher.
    pub(crate) fn v2_array_push_elem_size(&self, elem: NativeKind) -> Option<i64> {
        match elem {
            NativeKind::Float64 => Some(8),
            NativeKind::Int64 | NativeKind::UInt64 => Some(8),
            NativeKind::Int32 | NativeKind::UInt32 => Some(4),
            NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => Some(1),
            NativeKind::StringV2 | NativeKind::DecimalV2 => Some(8),
            _ => None,
        }
    }

    /// Emit a call to the generic `jit_v2_array_push` FFI dispatcher. `val`
    /// is the element value Cranelift SSA value already coerced to the
    /// native Cranelift type for `elem` (via `coerce_to_v2_elem`). This
    /// helper zero/sign-extends or bitcasts the value to I64 and passes
    /// `elem_size` as an I8 immediate.
    pub(crate) fn emit_v2_array_push_call(
        &mut self,
        arr_ptr: Value,
        val: Value,
        elem: NativeKind,
    ) -> Result<(), String> {
        let elem_size = match self.v2_array_push_elem_size(elem) {
            Some(s) => s,
            None => return Err(format!("v2_array_push: unsupported elem kind {:?}", elem)),
        };
        let bits = self.widen_to_i64_bits(val);
        let size_val = self.builder.ins().iconst(types::I8, elem_size);
        self.builder
            .ins()
            .call(self.ffi.v2_array_push, &[arr_ptr, bits, size_val]);
        Ok(())
    }

    /// Widen/bitcast an arbitrary Cranelift element value into an I64 bit
    /// pattern suitable for the generic `jit_v2_array_push` dispatcher.
    fn widen_to_i64_bits(&mut self, val: Value) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        if val_type == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
        } else if val_type == types::I64 {
            val
        } else if val_type == types::I32
            || val_type == types::I16
            || val_type == types::I8
        {
            // Zero-extend: the dispatcher uses only the low `elem_size` bytes,
            // so sign bits above that are ignored.
            self.builder.ins().uextend(types::I64, val)
        } else {
            val
        }
    }

    /// Convert a Cranelift value into the native type expected by the v2
    /// element store/push helpers for `elem`.
    pub(crate) fn coerce_to_v2_elem(&mut self, val: Value, elem: NativeKind) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        match elem {
            NativeKind::Float64 => {
                if val_type == types::F64 {
                    val
                } else if val_type == types::I64 {
                    self.builder.ins().bitcast(types::F64, MemFlags::new(), val)
                } else {
                    let i64_val = if val_type == types::I32 {
                        self.builder.ins().sextend(types::I64, val)
                    } else if val_type == types::I8 {
                        self.builder.ins().uextend(types::I64, val)
                    } else {
                        val
                    };
                    self.builder.ins().fcvt_from_sint(types::F64, i64_val)
                }
            }
            NativeKind::Int64 | NativeKind::UInt64 => {
                if val_type == types::I64 {
                    let shifted = self.builder.ins().ishl_imm(val, 16);
                    self.builder.ins().sshr_imm(shifted, 16)
                } else if val_type == types::I32 {
                    self.builder.ins().sextend(types::I64, val)
                } else if val_type == types::I8 {
                    self.builder.ins().uextend(types::I64, val)
                } else {
                    val
                }
            }
            NativeKind::Int32 | NativeKind::UInt32 => {
                if val_type == types::I32 {
                    val
                } else if val_type == types::I64 {
                    let shifted = self.builder.ins().ishl_imm(val, 16);
                    let i64_val = self.builder.ins().sshr_imm(shifted, 16);
                    self.builder.ins().ireduce(types::I32, i64_val)
                } else if val_type == types::I8 {
                    self.builder.ins().uextend(types::I32, val)
                } else {
                    val
                }
            }
            NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => {
                if val_type == types::I8 {
                    val
                } else if val_type == types::I64 {
                    self.builder.ins().ireduce(types::I8, val)
                } else if val_type == types::I32 {
                    self.builder.ins().ireduce(types::I8, val)
                } else {
                    val
                }
            }
            // ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
            // StringV2 / DecimalV2 elements are 8-byte raw pointers — the
            // operand value is already an I64-shaped `*const StringObj` /
            // `*const DecimalObj` produced by the per-element constant
            // materializer in `emit_v2_array_aggregate`'s StringV2/DecimalV2
            // arm. No coercion needed.
            NativeKind::StringV2 | NativeKind::DecimalV2 => val,
            _ => val,
        }
    }

    /// Coerce an arbitrary index Cranelift value into an `i32`.
    pub(crate) fn coerce_index_to_i32(&mut self, index_val: Value) -> Value {
        let idx_type = self.builder.func.dfg.value_type(index_val);
        if idx_type == types::I32 {
            index_val
        } else if idx_type == types::F64 {
            let i64_val = self
                .builder
                .ins()
                .fcvt_to_sint_sat(types::I64, index_val);
            self.builder.ins().ireduce(types::I32, i64_val)
        } else if idx_type == types::I8 {
            self.builder.ins().uextend(types::I32, index_val)
        } else {
            let shifted = self.builder.ins().ishl_imm(index_val, 16);
            let payload = self.builder.ins().sshr_imm(shifted, 16);
            self.builder.ins().ireduce(types::I32, payload)
        }
    }

    /// Allocate a v2 typed array of the given element kind via FFI, then push
    /// each operand value into it. Returns the raw `*mut TypedArray<T>` as an
    /// `i64` Cranelift value, or `None` when no v2 helper exists.
    ///
    /// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
    /// `StringV2` element kind takes a kind-specific per-element path —
    /// each `MirConstant::Str` / `MirConstant::StringId` operand is
    /// materialized at JIT-compile time as a `*const StringObj` constant
    /// via `crate::ffi::v2::string_obj_constant` (refcount-boosted permanent
    /// share, mirroring `crate::ffi::string::arc_string_constant` for the
    /// legacy `Arc<String>` carrier). The constant pointer is embedded as
    /// an `iconst I64` and pushed via the generic `jit_v2_array_push`
    /// dispatcher with elem_size=8. This is the JIT-side equivalent of the
    /// VM's `NewStringV2` opcode + `TypedArrayPushString` per-element
    /// transfer at `crates/shape-vm/src/executor/v2_handlers/array.rs:803`.
    ///
    /// `DecimalV2` element kind currently surfaces-and-stops at the MIR
    /// producer site — `MirConstant` has no `Decimal` variant, so Array
    /// <decimal> literals can't currently flow through MIR. Wiring the
    /// per-element NewDecimalV2 equivalent requires MIR-side producer
    /// support (`MirConstant::Decimal` variant or equivalent constant-pool
    /// reference), which is downstream territory beyond Group X's JIT FFI
    /// build scope.
    pub(crate) fn emit_v2_array_aggregate(
        &mut self,
        operands: &[Operand],
        elem: NativeKind,
    ) -> Result<Option<Value>, String> {
        let alloc_func = match self.v2_array_new_func(elem) {
            Some(f) => f,
            None => return Ok(None),
        };
        if self.v2_array_push_elem_size(elem).is_none() {
            return Ok(None);
        }

        let cap = self.builder.ins().iconst(types::I32, operands.len() as i64);
        let inst = self.builder.ins().call(alloc_func, &[cap]);
        let arr_ptr = self.builder.inst_results(inst)[0];

        match elem {
            // ckpt-6-prime Group X JIT FFI String/Decimal BUILD: per-element
            // NewStringV2 equivalent at the JIT mir_compiler dispatch site.
            // Each operand must be a `MirConstant::Str` / `MirConstant::
            // StringId` — the only producer sites for `NativeKind::StringV2`
            // Array<string> literals per ADR-006 §2.7.5 + audit deliverable
            // (b) §4.1.B. Other operand shapes structurally cannot produce
            // a StringV2-kind value and surface-and-stop here (no Bool-
            // default per §2.7.7 #9 / CLAUDE.md "Forbidden rationalizations").
            NativeKind::StringV2 => {
                use shape_vm::mir::types::MirConstant;
                for op in operands {
                    let s: String = match op {
                        Operand::Constant(MirConstant::Str(s)) => s.clone(),
                        Operand::Constant(MirConstant::StringId(id)) => {
                            let idx = *id as usize;
                            if idx >= self.strings.len() {
                                return Err(format!(
                                    "emit_v2_array_aggregate: StringV2 elem StringId({}) \
                                     out of bounds (pool len = {}) — string-pool conduit \
                                     mismatch at JIT compile time. ADR-006 §2.7.5 / Group X \
                                     JIT FFI String/Decimal BUILD.",
                                    id, self.strings.len()
                                ));
                            }
                            self.strings[idx].clone()
                        }
                        other => {
                            return Err(format!(
                                "emit_v2_array_aggregate: SURFACE — StringV2 elem kind \
                                 requires `MirConstant::Str` / `MirConstant::StringId` \
                                 operand per Group X NewStringV2-equivalent dispatch \
                                 (ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED + audit \
                                 deliverable (b) §4.1.B). Got: {:?}. No Bool-default \
                                 fallback per §2.7.7 #9 / CLAUDE.md Forbidden \
                                 rationalizations.",
                                other
                            ));
                        }
                    };
                    // Compile-time materialize a `*const StringObj` permanent-
                    // share constant (refcount=2; one share is the active
                    // share transferred to the array, the other is the
                    // constant's permanent share that survives JIT-function
                    // Drop chains).
                    let string_obj_ptr = crate::ffi::v2::string_obj_constant(&s);
                    let val = self
                        .builder
                        .ins()
                        .iconst(types::I64, string_obj_ptr as usize as i64);
                    self.emit_v2_array_push_call(arr_ptr, val, elem)?;
                }
            }
            // ckpt-6-prime Group X JIT FFI String/Decimal BUILD: per-element
            // NewDecimalV2 equivalent surface-and-stop. `MirConstant` has no
            // `Decimal` variant so Array<decimal> literals can't currently
            // flow through MIR — the FFI allocator + carrier-routing is
            // wired (above) but the per-element producer requires MIR-side
            // support that's beyond Group X's JIT FFI build scope.
            NativeKind::DecimalV2 => {
                return Err(format!(
                    "emit_v2_array_aggregate: SURFACE — DecimalV2 elem-kind \
                     per-element materialization requires MIR-side producer \
                     support (`MirConstant::Decimal` variant or equivalent \
                     constant-pool reference) which is not yet wired. Group X \
                     scope covers the JIT FFI allocator + carrier-routing \
                     (jit_new_typed_array_decimal + v2_array_new_func \
                     DecimalV2 arm); per-element materializer awaits the MIR \
                     producer's wiring. ADR-006 §2.7.5 + §2.7.24 Q25.A \
                     SUPERSEDED + audit deliverable (b) §4.1.B. {} operands \
                     received; no Bool-default per §2.7.7 #9.",
                    operands.len()
                ));
            }
            _ => {
                // Scalar element kinds (Float64/Int64/Int32/Bool/etc.) —
                // existing inline path. compile_operand_raw produces a
                // Cranelift SSA value already in the native element type;
                // coerce_to_v2_elem normalizes and emit_v2_array_push_call
                // routes through the generic dispatcher.
                for op in operands {
                    let raw = self.compile_operand_raw(op)?;
                    let val = self.coerce_to_v2_elem(raw, elem);
                    self.emit_v2_array_push_call(arr_ptr, val, elem)?;
                }
            }
        }

        Ok(Some(arr_ptr))
    }

    /// Try to emit an inline v2 typed-array method call.
    pub(crate) fn try_emit_v2_array_method(
        &mut self,
        method_name: &str,
        receiver: &Place,
        rest_args: &[Operand],
        destination: &Place,
        elem: NativeKind,
    ) -> Result<Option<()>, String> {
        match method_name {
            "length" | "len" => {
                let arr_ptr = self.read_place(receiver)?;
                let len_i32 = self.v2_array_len(arr_ptr);
                let len_i64 = self.builder.ins().sextend(types::I64, len_i32);
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, len_i64)?;
                Ok(Some(()))
            }
            "push" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                if self.v2_array_push_elem_size(elem).is_none() {
                    return Ok(None);
                }
                let arr_ptr = self.read_place(receiver)?;
                let raw_arg = self.compile_operand_raw(&rest_args[0])?;
                let val = self.coerce_to_v2_elem(raw_arg, elem);
                self.emit_v2_array_push_call(arr_ptr, val, elem)?;
                let none_val = self.builder.ins().iconst(types::I64, 0i64);
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, none_val)?;
                Ok(Some(()))
            }
            "sum" => {
                // Phase C.3: Bypass method dispatch entirely — call the SIMD
                // reduction FFI (`jit_v2_array_sum_f64` / `jit_v2_array_sum_i64`)
                // in one shot. The FFI uses `wide::f64x4`/`wide::i64x4` lanes
                // so AVX2/NEON-capable CPUs get a ~4x throughput over the
                // scalar loop.
                if !rest_args.is_empty() {
                    return Ok(None);
                }
                let sum_func = match elem {
                    NativeKind::Float64 => self.ffi.v2_array_sum_f64,
                    NativeKind::Int64 | NativeKind::UInt64 => self.ffi.v2_array_sum_i64,
                    _ => return Ok(None),
                };
                let arr_ptr = self.read_place(receiver)?;
                let inst = self.builder.ins().call(sum_func, &[arr_ptr]);
                let result = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, result)?;
                Ok(Some(()))
            }
            // f64-only SIMD reductions. Dispatched only for Array<number>.
            "min" | "max" | "mean" | "avg" | "sumSquares" | "sum_squares" => {
                if !rest_args.is_empty() {
                    return Ok(None);
                }
                if !matches!(elem, NativeKind::Float64) {
                    return Ok(None);
                }
                let func = match method_name {
                    "min" => self.ffi.v2_array_min_f64,
                    "max" => self.ffi.v2_array_max_f64,
                    "mean" | "avg" => self.ffi.v2_array_mean_f64,
                    "sumSquares" | "sum_squares" => self.ffi.v2_array_sum_squares_f64,
                    _ => unreachable!(),
                };
                let arr_ptr = self.read_place(receiver)?;
                let inst = self.builder.ins().call(func, &[arr_ptr]);
                let result = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, result)?;
                Ok(Some(()))
            }
            // f64 scalar broadcast — returns a new Array<number>.
            "scale" | "addScalar" | "add_scalar" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                if !matches!(elem, NativeKind::Float64) {
                    return Ok(None);
                }
                let func = match method_name {
                    "scale" => self.ffi.v2_array_scale_f64,
                    "addScalar" | "add_scalar" => self.ffi.v2_array_add_scalar_f64,
                    _ => unreachable!(),
                };
                let arr_ptr = self.read_place(receiver)?;
                let raw = self.compile_operand_raw(&rest_args[0])?;
                let scalar = self.coerce_to_v2_elem(raw, NativeKind::Float64);
                let inst = self.builder.ins().call(func, &[arr_ptr, scalar]);
                let new_arr = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, new_arr)?;
                Ok(Some(()))
            }
            // f64 element-wise binary ops — both operands are Array<number>,
            // returns a new Array<number>.
            "addArray" | "add_array" | "mulArray" | "mul_array" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                if !matches!(elem, NativeKind::Float64) {
                    return Ok(None);
                }
                let func = match method_name {
                    "addArray" | "add_array" => self.ffi.v2_array_add_f64,
                    "mulArray" | "mul_array" => self.ffi.v2_array_mul_f64,
                    _ => unreachable!(),
                };
                let arr_ptr = self.read_place(receiver)?;
                let other = self.compile_operand_raw(&rest_args[0])?;
                // The other argument is an Array<number> (pointer); no coercion
                // needed, but make sure the value type is i64 before handoff.
                let other_i64 = {
                    let ty = self.builder.func.dfg.value_type(other);
                    if ty == types::I64 {
                        other
                    } else {
                        // Fall back to generic dispatch if we couldn't resolve
                        // the other operand to a plain pointer-sized value.
                        return Ok(None);
                    }
                };
                let inst = self.builder.ins().call(func, &[arr_ptr, other_i64]);
                let new_arr = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, new_arr)?;
                Ok(Some(()))
            }
            _ => Ok(None),
        }
    }

    /// Inline typed array element read.
    ///
    /// Emits:
    /// 1. Load `data` pointer from `[arr_ptr + 8]`
    /// 2. Load `len` (u32) from `[arr_ptr + 16]`
    /// 3. Bounds check: `if index >= len` return zero-default
    /// 4. Compute element address: `data + index * elem_size`
    /// 5. Load element with the correct Cranelift type
    ///
    /// `arr_ptr` is a Cranelift `i64` value pointing to a `TypedArrayHeader`.
    /// `index` is a Cranelift `i32` value (unsigned index).
    /// Returns the loaded element value (type depends on `elem_type`).
    pub fn v2_array_get(
        &mut self,
        arr_ptr: Value,
        index: Value,
        elem_type: NativeKind,
    ) -> Value {
        let (cl_type, elem_size) = elem_type_info(elem_type);

        // 1. Load data pointer (i64) from arr_ptr + DATA_PTR_OFFSET
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, DATA_PTR_OFFSET);

        // 2. Load length (u32) from arr_ptr + LEN_OFFSET
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET);

        // 3. Bounds check: if index >= len, branch to out-of-bounds block
        let in_bounds_block = self.builder.create_block();
        let oob_block = self.builder.create_block();
        let merge_block = self.builder.create_block();

        // The merge block receives the result as a block parameter.
        self.builder.append_block_param(merge_block, cl_type);

        let cmp = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, index, len);
        self.builder
            .ins()
            .brif(cmp, in_bounds_block, &[], oob_block, &[]);

        // ── Out-of-bounds path: return default ──────────────────────────
        self.builder.switch_to_block(oob_block);
        self.builder.seal_block(oob_block);

        let default_val = emit_default(self.builder, elem_type);
        self.builder.ins().jump(merge_block, &[default_val]);

        // ── In-bounds path: compute address and load element ────────────
        self.builder.switch_to_block(in_bounds_block);
        self.builder.seal_block(in_bounds_block);

        // 4. Compute byte offset: index (u32) -> i64, then * elem_size
        let index_i64 = self.builder.ins().uextend(types::I64, index);
        let byte_offset = if (elem_size as u64).is_power_of_two() {
            let shift = (elem_size as u64).trailing_zeros() as i64;
            self.builder.ins().ishl_imm(index_i64, shift)
        } else {
            let size_val = self.builder.ins().iconst(types::I64, elem_size);
            self.builder.ins().imul(index_i64, size_val)
        };
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);

        // 5. Load element with trusted flags (bounds already checked)
        let loaded = self
            .builder
            .ins()
            .load(cl_type, MemFlags::trusted(), elem_addr, 0);

        self.builder.ins().jump(merge_block, &[loaded]);

        // ── Merge ───────────────────────────────────────────────────────
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);

        self.builder.block_params(merge_block)[0]
    }

    /// Inline typed array length.
    ///
    /// Emits a single `load i32 [arr_ptr + 16]`.
    pub fn v2_array_len(&mut self, arr_ptr: Value) -> Value {
        self.builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET)
    }

    /// Inline typed array element write.
    ///
    /// Emits:
    /// 1. Load `data` pointer from `[arr_ptr + 8]`
    /// 2. Load `len` (u32) from `[arr_ptr + 16]`
    /// 3. Bounds check: `if index >= len` skip (silent no-op for OOB)
    /// 4. Compute element address: `data + index * elem_size`
    /// 5. Store element with the correct Cranelift type
    ///
    /// `val` must be a Cranelift value whose type matches `elem_type`.
    pub fn v2_array_set(
        &mut self,
        arr_ptr: Value,
        index: Value,
        val: Value,
        elem_type: NativeKind,
    ) {
        let (_cl_type, elem_size) = elem_type_info(elem_type);

        // 1. Load data pointer
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, DATA_PTR_OFFSET);

        // 2. Load length
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET);

        // 3. Bounds check
        let in_bounds_block = self.builder.create_block();
        let continue_block = self.builder.create_block();

        let cmp = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, index, len);
        self.builder
            .ins()
            .brif(cmp, in_bounds_block, &[], continue_block, &[]);

        // ── In-bounds path: store element ───────────────────────────────
        self.builder.switch_to_block(in_bounds_block);
        self.builder.seal_block(in_bounds_block);

        let index_i64 = self.builder.ins().uextend(types::I64, index);
        let byte_offset = if (elem_size as u64).is_power_of_two() {
            let shift = (elem_size as u64).trailing_zeros() as i64;
            self.builder.ins().ishl_imm(index_i64, shift)
        } else {
            let size_val = self.builder.ins().iconst(types::I64, elem_size);
            self.builder.ins().imul(index_i64, size_val)
        };
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);

        self.builder
            .ins()
            .store(MemFlags::trusted(), val, elem_addr, 0);

        self.builder.ins().jump(continue_block, &[]);

        // ── Continue ────────────────────────────────────────────────────
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

