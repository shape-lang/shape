//! Builtin dispatch slice (ADR-006 §2.7.6 / Q8).
//!
//! Wave 5a (phase-1b-vm) flipped the dispatch SHAPE here: every arm now
//! produces / consumes `Vec<KindedSlot>` (and `&[KindedSlot]`), aligned
//! with the carrier-API bound spec'd at §2.7.6. The body interiors
//! (math kernels, array kernels, content builders, type-introspection,
//! stats, intrinsics, JSON helpers, table builders, content / DateTime /
//! concurrency constructors) are deferred to Waves 5b-5e.
//!
//! - **Wave 5b (LANDED)**: math + array + utility bodies (`builtin_abs`,
//!   `builtin_push`, `builtin_object_rest`, `builtin_snapshot`,
//!   `builtin_exit`, etc.) are now `Fn(&[KindedSlot], ...) -> Result<KindedSlot, VMError>`
//!   and the dispatch arms call them directly.
//! - **Wave 5c**: type-introspection + conversion + native-interop bodies
//!   (`builtin_is_*`, `builtin_to_*`, `dispatch_native_interop_builtin`).
//! - **Wave 5d**: closure-driven array builtins (`map`, `filter`, `reduce`,
//!   etc.) + intrinsic dispatch (`handle_intrinsic_builtin`,
//!   `handle_vector_intrinsic`, `handle_matrix_intrinsic`).
//! - **Wave 5e**: content + DateTime + concurrency constructors + window /
//!   join / reflect / state-builtin bodies + `executor/printing.rs` formatter.
//!
//! The companion §2.7.6 / Q8 carrier-API bound: NO per-heap-variant
//! accessors on `KindedSlot`; bodies that inspect heap payloads use
//! `slot.as_heap_value()` + `HeapValue` match. NO cross-kind accessors
//! (`as_number_coerce`, etc.) on the carrier; coercion lives at
//! `executor/builtins/kind_coerce.rs` (free helper at the body site).
//!
//! # `pop_builtin_args` runtime semantics (Wave 6: kinded stack ABI)
//!
//! Wave 6 (ADR-006 §2.7.7 / Q9) added a parallel `Vec<NativeKind>` track
//! to the VM stack. `pop_builtin_args` now reads the per-arg `NativeKind`
//! directly from the parallel track via `pop_kinded()`. Wave 5b's
//! transitional `NativeKind::Bool` sentinel is removed — every arg's kind
//! is the kind that the producing opcode emitted into the parallel track
//! at push time.
//!
//! **Ownership transfer**: `pop_kinded()` moves one strong-count share
//! (for heap-bearing kinds) out of the stack slot into the returned
//! tuple. Wrapping it in a `KindedSlot` transfers that share to the
//! carrier; `KindedSlot::Drop` retires the share when the args `Vec` is
//! dropped at the end of the builtin call. **No `clone_with_kind`
//! needed** here — that's only for `read_owned_kinded` (which keeps the
//! slot live on the stack while handing a share out).

use super::super::*;
use crate::executor::result_option_carrier;
use shape_value::{HeapKind, KindedSlot, NativeKind, VMError, ValueSlot};

impl VirtualMachine {
    /// Pop the builtin call's args off the typed VM stack into a
    /// `Vec<KindedSlot>` (ADR-006 §2.7.7 / Q9).
    ///
    /// The topmost stack slot is the arg count (pushed as a numeric
    /// constant by the compiler). Each subsequent pop hands back the raw
    /// u64 bits **plus** the `NativeKind` recorded by the producing opcode
    /// in the parallel kinds track.
    ///
    /// **Ownership**: `pop_kinded()` transfers the slot's strong-count
    /// share into the returned tuple; wrapping it in a `KindedSlot`
    /// transfers ownership to the carrier. `KindedSlot::Drop` retires the
    /// share when the returned `Vec` goes out of scope.
    pub(crate) fn pop_builtin_args(&mut self) -> Result<Vec<KindedSlot>, VMError> {
        // Top of stack: the arg count, pushed as a typed integer constant
        // by the compiler (`PushConst(Int(arg_count as i64))`). The count
        // slot is an integer-family inline scalar; `int_operand` dispatches
        // per the §2.7.6 heterogeneous-kind body pattern (same shape as
        // `op_call` / `op_call_value` use).
        //
        // Historical note (W17-make-closure): prior to the arg-count emit
        // migration the compiler emitted `Number(arg_count as f64)` and
        // this body decoded `f64::from_bits(count_bits) as usize`. That
        // shape made `op_call` (which uses `int_operand`) reject the same
        // arg-count slot, surfacing as the smoke-2 "Expected integer for
        // arg count" failure. The fix landed here together with the
        // call-site emit changes in `compiler/expressions/function_calls.rs`.
        let (count_bits, count_kind) = self.pop_kinded()?;
        let count_slot = KindedSlot::new(ValueSlot::from_raw(count_bits), count_kind);
        let count =
            crate::executor::builtins::kind_coerce::int_operand(&count_slot).map_err(|_| {
                VMError::RuntimeError(format!(
                    "pop_builtin_args: arg-count slot must be integer-family, got kind {:?}",
                    count_kind
                ))
            })? as usize;
        // Drop the arg-count's share (inline scalar — no-op for integer
        // kinds, but the discipline lives at the §2.7.7 parallel-kind
        // boundary).
        crate::executor::vm_impl::stack::drop_with_kind(count_bits, count_kind);

        let mut args: Vec<KindedSlot> = Vec::with_capacity(count);
        for _ in 0..count {
            let (bits, kind) = self.pop_kinded()?;
            // The pop transferred the slot's share to us; wrap it in a
            // KindedSlot which will Drop-retire the share when the
            // builtin call's arg vec is dropped.
            args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
        }
        args.reverse();
        Ok(args)
    }

    /// Push a `KindedSlot` result back onto the stack. The carrier's
    /// share transfers into the slot; we `mem::forget` the carrier so its
    /// `Drop` does not retire the share that the slot now owns.
    #[inline]
    pub(crate) fn push_kinded_slot(&mut self, slot: KindedSlot) -> Result<(), VMError> {
        let bits = slot.slot().raw();
        let kind = slot.kind();
        std::mem::forget(slot);
        self.push_kinded(bits, kind)
    }

    /// Native-interop out-param cell builtins (WF-2A stage 2 — backing the
    /// `extern C` `out`-param stub). The compiler-generated stub
    /// (`compiler/functions_foreign.rs::emit_out_param_stub`) allocates one
    /// cell per `out` param via `NativePtrNewCell`, initializes / reads it via
    /// `NativePtrWritePtr` / `NativePtrReadPtr`, passes its ADDRESS to the C
    /// function as a `ptr` arg, and frees it via `NativePtrFreeCell`.
    ///
    /// Anti-UB invariant: a foreign / cell address is never
    /// `NativeKind::Ptr(HeapKind::…)`; it is a raw `UIntSize` scalar so
    /// `KindedSlot::Drop` never touches it (no `Arc::decrement_strong_count`
    /// on a raw heap address).
    pub(crate) fn dispatch_native_interop_builtin(
        &mut self,
        builtin: crate::bytecode::BuiltinFunction,
        args: Vec<KindedSlot>,
    ) -> Result<KindedSlot, VMError> {
        use crate::bytecode::BuiltinFunction;
        use crate::executor::control_flow::native_abi;

        // Read a UIntSize/integer-kinded cell address argument as `usize`.
        fn cell_addr(slot: &KindedSlot, ctx: &str) -> Result<usize, VMError> {
            match slot.kind() {
                NativeKind::UIntSize
                | NativeKind::IntSize
                | NativeKind::UInt64
                | NativeKind::Int64 => Ok(slot.raw() as usize),
                other => Err(VMError::RuntimeError(format!(
                    "{ctx}: expected a pointer-sized (UIntSize) cell address, got kind {other:?}"
                ))),
            }
        }

        match builtin {
            BuiltinFunction::NativePtrSize => Ok(KindedSlot::new(
                ValueSlot::from_raw(8),
                NativeKind::UIntSize,
            )),
            BuiltinFunction::NativePtrNewCell => {
                let addr = native_abi::native_cell_new().ok_or_else(|| {
                    VMError::RuntimeError(
                        "NativePtrNewCell: out of memory allocating out-param cell".to_string(),
                    )
                })?;
                Ok(KindedSlot::new(
                    ValueSlot::from_raw(addr as u64),
                    NativeKind::UIntSize,
                ))
            }
            BuiltinFunction::NativePtrWritePtr => {
                let cell = args.first().ok_or_else(|| {
                    VMError::RuntimeError("NativePtrWritePtr: missing cell".into())
                })?;
                let value = args.get(1).ok_or_else(|| {
                    VMError::RuntimeError("NativePtrWritePtr: missing value".into())
                })?;
                let addr = cell_addr(cell, "NativePtrWritePtr")?;
                // SAFETY: `addr` is a live cell from NativePtrNewCell.
                unsafe { native_abi::native_cell_write(addr, value.raw()) };
                Ok(KindedSlot::none())
            }
            BuiltinFunction::NativePtrReadPtr => {
                let cell = args.first().ok_or_else(|| {
                    VMError::RuntimeError("NativePtrReadPtr: missing cell".into())
                })?;
                let addr = cell_addr(cell, "NativePtrReadPtr")?;
                // SAFETY: `addr` is a live cell from NativePtrNewCell.
                let bits = unsafe { native_abi::native_cell_read(addr) };
                Ok(KindedSlot::new(
                    ValueSlot::from_raw(bits),
                    NativeKind::UIntSize,
                ))
            }
            BuiltinFunction::NativePtrFreeCell => {
                let cell = args.first().ok_or_else(|| {
                    VMError::RuntimeError("NativePtrFreeCell: missing cell".into())
                })?;
                let addr = cell_addr(cell, "NativePtrFreeCell")?;
                // SAFETY: `addr` was returned by NativePtrNewCell and is freed
                // exactly once (the stub frees each cell once).
                unsafe { native_abi::native_cell_free(addr) };
                Ok(KindedSlot::none())
            }
            other => Err(VMError::RuntimeError(format!(
                "dispatch_native_interop_builtin: unexpected builtin {other:?}"
            ))),
        }
    }

    // ========================================================================
    // Builtin Dispatch
    //
    // Wave 5a flipped the dispatch SHAPE: every arm produces /
    // consumes `Vec<KindedSlot>`. Wave 5b lands the math/array/utility
    // body migrations and wires the dispatch arms.

    pub fn op_builtin_call(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        if let Some(Operand::Builtin(builtin)) = instruction.operand {
            let _ctx = ctx;
            match builtin {
                // ── Wave 5b: math builtins ────────────────────────────────
                BuiltinFunction::Abs => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_abs(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Sqrt => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_sqrt(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Ln => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_ln(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Pow => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_pow(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Exp => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_exp(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Log => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_log(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Floor => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_floor(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Ceil => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_ceil(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Round => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_round(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Sin => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_sin(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Cos => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_cos(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Tan => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_tan(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Asin => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_asin(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Acos => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_acos(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Atan => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_atan(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Min => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_min(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Max => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_max(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::StdDev => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_stddev(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Sign => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_sign(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Gcd => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_gcd(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Lcm => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_lcm(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Hypot => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_hypot(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Clamp => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_clamp(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IsNaN => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_is_nan(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IsFinite => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_is_finite(&args)?;
                    self.push_kinded_slot(r)?;
                }

                // ── Wave 5b: array builtins ───────────────────────────────
                BuiltinFunction::Push => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_push(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Pop => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_pop(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::First => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_first(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Last => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_last(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Zip => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_zip(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Filled => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_filled(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Range => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_range(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Slice => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_slice(&args)?;
                    self.push_kinded_slot(r)?;
                }

                // ── Wave 5b: utility builtins ─────────────────────────────
                BuiltinFunction::ObjectRest => {
                    let args = self.pop_builtin_args()?;
                    let r = self.builtin_object_rest(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::Snapshot => {
                    // Snapshot suspends execution; never returns a value.
                    let _args = self.pop_builtin_args()?;
                    return Err(VMError::Suspended {
                        future_id: SNAPSHOT_FUTURE_ID,
                        resume_ip: self.ip,
                    });
                }
                BuiltinFunction::Exit => {
                    let args = self.pop_builtin_args()?;
                    let code = if args.is_empty() {
                        0
                    } else {
                        // Best-effort code extraction. The arg comes in as
                        // Bool-kinded (Wave 6 stack-ABI gap); reinterpret the
                        // raw bits as i64 since `exit(code)` is documented to
                        // take an int.
                        args[0].slot.raw() as i64 as i32
                    };
                    std::process::exit(code);
                }
                BuiltinFunction::Print => {
                    // ADR-006 §2.7.4 — pop the kinded args, format each
                    // through `ValueFormatter::format_kinded` (top-level
                    // unquoted-string rendering, nested quotes inside
                    // containers), join with spaces, surface to the
                    // `OutputAdapter::print` of the active
                    // `ExecutionContext`. Returns the unit/null sentinel
                    // per the §2.7.4 GENERIC_CARRIER ABI.
                    //
                    // The pushed result is a `Ptr(HeapKind::String)`-kind
                    // null slot rather than `KindedSlot::none()`'s
                    // `Bool=0` shape: `wire_conversion::slot_to_wire`
                    // projects `Ptr(_)` with bits=0 to `WireValue::Null`,
                    // which the script runner suppresses when printing
                    // the program's final value (`script_cmd.rs:1353`).
                    // The `Bool=0` sentinel would otherwise surface as a
                    // spurious `false` line after every `print()`.
                    let args = self.pop_builtin_args()?;
                    self.builtin_print(&args, _ctx)?;
                    let null_slot = KindedSlot::new(
                        ValueSlot::from_raw(0),
                        shape_value::NativeKind::Ptr(shape_value::HeapKind::String),
                    );
                    self.push_kinded_slot(null_slot)?;
                }
                BuiltinFunction::Format | BuiltinFunction::FormatValueWithMeta => {
                    // Universal value-to-string. `Format` joins multiple
                    // args without separator (Shape's `format("a", "b")`
                    // → `"ab"` legacy semantics); `FormatValueWithMeta`
                    // is the single-arg `expr.to_string()` /
                    // `f"{expr}"` interpolation path.
                    let args = self.pop_builtin_args()?;
                    let r = self.builtin_format(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::FormatValueWithSpec => {
                    // Args: [value, spec_tag, …spec-payload]. Currently
                    // routes the basic FORMAT_SPEC_FIXED path; the Table
                    // arm surfaces as `NotImplemented` per W13 playbook
                    // §7.4 surface-and-stop.
                    let args = self.pop_builtin_args()?;
                    let r = self.builtin_format_with_spec(&args)?;
                    self.push_kinded_slot(r)?;
                }

                // ── Wave 5c: type-introspection + conversion + native-interop ──
                BuiltinFunction::IsArray | BuiltinFunction::IsObject => {
                    let args = self.pop_builtin_args()?;
                    let value = args.first().ok_or_else(|| {
                        VMError::RuntimeError(format!("{:?} expects 1 argument", builtin))
                    })?;
                    let result = match builtin {
                        BuiltinFunction::IsArray => {
                            matches!(value.kind, NativeKind::Ptr(HeapKind::TypedArray))
                        }
                        BuiltinFunction::IsObject => {
                            matches!(value.kind, NativeKind::Ptr(HeapKind::TypedObject))
                        }
                        _ => unreachable!("outer match restricts builtin"),
                    };
                    self.push_kinded_slot(KindedSlot::from_bool(result))?;
                }
                BuiltinFunction::IsNumber
                | BuiltinFunction::IsString
                | BuiltinFunction::IsBool
                | BuiltinFunction::IsDataRow => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5c —
                    // is_* type-check body migration deferred. Drain args
                    // to keep the §2.7.7 parallel-kind track balanced,
                    // then return a structured `NotImplemented` so the
                    // dispatch arm fails loudly without panicking. The
                    // re-fill rebuilds bodies against the `KindedSlot`
                    // carrier (Q8 carrier-API-bound).
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "phase-1b-vm-wave-5c-is-type-check: {:?} body \
                         migration to kinded carrier (ADR-006 §2.7.6) \
                         pending (v0.4 / planned)",
                        builtin
                    )));
                }
                BuiltinFunction::ToString | BuiltinFunction::ToNumber | BuiltinFunction::ToBool => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5c —
                    // conversion body migration (`dispatch_conversion_builtin`)
                    // deferred. Drain args to balance the §2.7.7 parallel-
                    // kind track before surfacing.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "phase-1b-vm-wave-5c-conversion: {:?} body \
                         migration to kinded carrier (dispatch_conversion_builtin) \
                         pending (v0.4 / planned)",
                        builtin
                    )));
                }
                BuiltinFunction::NativePtrSize
                | BuiltinFunction::NativePtrNewCell
                | BuiltinFunction::NativePtrFreeCell
                | BuiltinFunction::NativePtrReadPtr
                | BuiltinFunction::NativePtrWritePtr => {
                    // WF-2A stage 2: out-param cell primitives backing the
                    // `extern C` `out`-param stub. Kinded end to end.
                    let args = self.pop_builtin_args()?;
                    let r = self.dispatch_native_interop_builtin(builtin, args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::NativeTableFromArrowC
                | BuiltinFunction::NativeTableFromArrowCTyped
                | BuiltinFunction::NativeTableBindType => {
                    // SURFACE per ADR-006 §2.7.14: the Arrow-C table bridge
                    // (`arrow_bridge.rs`) is out of the FFI-rebuild gate (§2
                    // non-goals). Drain args to balance the §2.7.7 parallel-
                    // kind track.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "native Arrow-C table bridge ({:?}) is out of the WF-2A FFI-rebuild \
                         scope (arrow_bridge.rs; ffi-rebuild §2 non-goals)",
                        builtin
                    )));
                }
                BuiltinFunction::TypeOf => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5c —
                    // `TypeOf` body migration pending. The legacy body
                    // popped via the deleted raw-bits stack shim; needs a
                    // kinded-carrier rebuild per ADR-006 §2.7.6. No args
                    // are popped because the legacy emit shape did not
                    // emit an arity prefix here.
                    return Err(VMError::NotImplemented(
                        "phase-1b-vm-wave-5c-typeof: TypeOf body migration \
                         to kinded carrier (ADR-006 §2.7.6) pending \
                         (v0.4 / planned)"
                            .to_string(),
                    ));
                }

                // ── Wave 5d: closure-driven array builtins + intrinsics ──────
                BuiltinFunction::Map
                | BuiltinFunction::Filter
                | BuiltinFunction::Reduce
                | BuiltinFunction::ForEach
                | BuiltinFunction::Find
                | BuiltinFunction::FindIndex
                | BuiltinFunction::Some
                | BuiltinFunction::Every
                | BuiltinFunction::ControlFold => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5d —
                    // closure-driven array builtin body migration deferred.
                    // The rebuild routes through the §2.7.11 / Q12 kinded
                    // value-call ABI for the closure invocation. Drain
                    // args to balance the §2.7.7 parallel-kind track.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "phase-1b-vm-wave-5d-closure-array: {:?} body \
                         migration to kinded carrier + value-call ABI \
                         (ADR-006 §2.7.11/Q12) pending (v0.4 / planned)",
                        builtin
                    )));
                }
                BuiltinFunction::IntrinsicVecAbs
                | BuiltinFunction::IntrinsicVecSqrt
                | BuiltinFunction::IntrinsicVecLn
                | BuiltinFunction::IntrinsicVecExp
                | BuiltinFunction::IntrinsicVecAdd
                | BuiltinFunction::IntrinsicVecSub
                | BuiltinFunction::IntrinsicVecMul
                | BuiltinFunction::IntrinsicVecDiv
                | BuiltinFunction::IntrinsicVecMax
                | BuiltinFunction::IntrinsicVecMin
                | BuiltinFunction::IntrinsicVecSelect
                | BuiltinFunction::IntrinsicVecAddI64 => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5d —
                    // SIMD vector intrinsic body migration
                    // (`handle_vector_intrinsic`) deferred. No arg pop
                    // because the legacy dispatcher handled it internally;
                    // surface immediately so the operator sees a clean
                    // error before any state mutation. Stack rebalancing
                    // is the rebuild's responsibility per
                    // `executor/builtins/vector_intrinsics.rs`.
                    return Err(VMError::NotImplemented(format!(
                        "phase-1b-vm-wave-5d-vec-intrinsic: {:?} body \
                         migration to kinded carrier (handle_vector_intrinsic) \
                         pending (v0.4 / planned)",
                        builtin
                    )));
                }
                BuiltinFunction::IntrinsicMatAdd | BuiltinFunction::IntrinsicMatSub => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::matrix_intrinsics::builtin_matrix_arithmetic(
                        builtin, &args,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicMatMulVec | BuiltinFunction::IntrinsicMatMulMat => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5d —
                    // matrix intrinsic body migration
                    // (`handle_matrix_intrinsic`) deferred. See sibling
                    // vector arm above for arg-pop rationale.
                    return Err(VMError::NotImplemented(format!(
                        "phase-1b-vm-wave-5d-mat-intrinsic: {:?} body \
                         migration to kinded carrier (handle_matrix_intrinsic) \
                         pending (v0.4 / planned)",
                        builtin
                    )));
                }
                BuiltinFunction::IntrinsicMinimize => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5d —
                    // `minimize` intrinsic body deferred. Drain args to
                    // balance the §2.7.7 parallel-kind track.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(
                        "phase-1b-vm-wave-5d-minimize: minimize() intrinsic \
                         body migration to kinded carrier + value-call ABI \
                         (ADR-006 §2.7.11/Q12) pending (v0.4 / planned)"
                            .to_string(),
                    ));
                }
                // W12-stdlib-intrinsic-collapse (Wave-2-Agent-G, 2026-05-14):
                // `BuiltinFunction::IntrinsicSum` deleted as the canonical
                // 7th defection-attractor instance (parallel-implementation
                // across producer/consumer carrier-shape boundaries — the
                // old handler body's own comment said "mirror of
                // v2_int_sum/v2_float_sum exactly"). Stdlib
                // `pub fn sum(series) { series.sum() }` now routes through
                // the PHF `.sum()` method dispatch — single discriminator
                // per ADR-005 §1, `MethodFnV2` ABI per ADR-006 §2.7.10/Q11.
                // ── MA1: aggregate statistics + scalar hyperbolic/atan2.
                // Documented at stdlib/core/math (sum/mean/std/variance) and
                // stdlib/native/math (atan2/sinh/cosh/tanh). Bodies read the
                // numeric typed-array argument through the kind-generic v2
                // view (ADR-005 §1 single discriminator; ADR-006 §2.7.6). ──
                BuiltinFunction::IntrinsicMean => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_mean(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicStd => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_std(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicVariance => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_variance(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicAtan2 => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_atan2(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicSinh => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_sinh(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicCosh => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_cosh(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicTanh => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_tanh(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicRandom
                | BuiltinFunction::IntrinsicRandomInt
                | BuiltinFunction::IntrinsicRandomSeed
                | BuiltinFunction::IntrinsicRandomNormal
                | BuiltinFunction::IntrinsicRandomArray => {
                    let args = self.pop_builtin_args()?;
                    let r = vm_random_intrinsic_slot(builtin, &args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::IntrinsicBspline2_3dBatch
                | BuiltinFunction::IntrinsicMin
                | BuiltinFunction::IntrinsicMax
                | BuiltinFunction::IntrinsicDistUniform
                | BuiltinFunction::IntrinsicDistLognormal
                | BuiltinFunction::IntrinsicDistExponential
                | BuiltinFunction::IntrinsicDistPoisson
                | BuiltinFunction::IntrinsicDistSampleN
                | BuiltinFunction::IntrinsicBrownianMotion
                | BuiltinFunction::IntrinsicGbm
                | BuiltinFunction::IntrinsicOuProcess
                | BuiltinFunction::IntrinsicRandomWalk
                | BuiltinFunction::IntrinsicRollingSum
                | BuiltinFunction::IntrinsicRollingMean
                | BuiltinFunction::IntrinsicRollingStd
                | BuiltinFunction::IntrinsicRollingMin
                | BuiltinFunction::IntrinsicRollingMax
                | BuiltinFunction::IntrinsicEma
                | BuiltinFunction::IntrinsicLinearRecurrence
                | BuiltinFunction::IntrinsicShift
                | BuiltinFunction::IntrinsicDiff
                | BuiltinFunction::IntrinsicPctChange
                | BuiltinFunction::IntrinsicFillna
                | BuiltinFunction::IntrinsicCumsum
                | BuiltinFunction::IntrinsicCumprod
                | BuiltinFunction::IntrinsicClip
                | BuiltinFunction::IntrinsicCorrelation
                | BuiltinFunction::IntrinsicCovariance
                | BuiltinFunction::IntrinsicPercentile
                | BuiltinFunction::IntrinsicMedian
                | BuiltinFunction::IntrinsicCharCode
                | BuiltinFunction::IntrinsicFromCharCode
                | BuiltinFunction::IntrinsicSeries => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5d —
                    // stats / distribution / stochastic / rolling / scalar-math /
                    // series intrinsic body migration
                    // (`handle_intrinsic_builtin`) deferred. Covers the
                    // ~37-variant cluster (Bspline2_3dBatch, Mean, Min,
                    // Max, Std, Variance, Dist*, BrownianMotion,
                    // Gbm, OuProcess, RandomWalk, Rolling*, Ema,
                    // LinearRecurrence, Shift, Diff, PctChange, Fillna,
                    // Cumsum, Cumprod, Clip, Correlation, Covariance,
                    // Percentile, Median, Atan2, Sinh, Cosh, Tanh,
                    // CharCode, FromCharCode, Series). No arg pop because
                    // the legacy dispatcher handled per-variant arity; the
                    // rebuild lives at `executor/builtins/intrinsics/`
                    // (math.rs / statistical.rs / signal.rs).
                    return Err(VMError::NotImplemented(format!(
                        "phase-1b-vm-wave-5d-intrinsic: {:?} body migration \
                         to kinded carrier (handle_intrinsic_builtin) \
                         pending (v0.4 / planned)",
                        builtin
                    )));
                }

                // ── Wave 5e: constructors (Result/Option, Set, Deque,
                // PriorityQueue, HashMap, Mutex/Atomic/Lazy/Channel),
                // Content builders, DateTime constructors, Table from
                // rows, JSON navigation helpers, Window functions, Join,
                // Reflect, MatFromFlat, MakeContent*. ─────────────────────
                BuiltinFunction::SomeCtor => {
                    // L5 canonical carrier: `Some(x)` is a `__Option`
                    // TypedObject. The payload carrier's share moves into
                    // field 1, and TypedObjectStorage releases it via the
                    // field_kinds track.
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Some() expects 1 argument, got {}",
                            args.len()
                        )));
                    }
                    let payload = args.remove(0);
                    self.push_kinded_slot(result_option_carrier::build_some(
                        &self.builtin_schemas,
                        payload,
                    ))?;
                }
                BuiltinFunction::OkCtor => {
                    // L5 canonical carrier: `Ok(x)` is a `__Result`
                    // TypedObject with the public enum tag for Ok.
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Ok() expects 1 argument, got {}",
                            args.len()
                        )));
                    }
                    let payload = args.remove(0);
                    self.push_kinded_slot(result_option_carrier::build_ok(
                        &self.builtin_schemas,
                        payload,
                    ))?;
                }
                BuiltinFunction::ErrCtor => {
                    // L5 canonical carrier: `Err(e)` is a `__Result`
                    // TypedObject with the public enum tag for Err.
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Err() expects 1 argument, got {}",
                            args.len()
                        )));
                    }
                    let payload = args.remove(0);
                    self.push_kinded_slot(result_option_carrier::build_err(
                        &self.builtin_schemas,
                        payload,
                    ))?;
                }
                BuiltinFunction::HashMapCtor => {
                    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per
                    // ADR-006 §2.7.24 Q25.B SUPERSEDED, `let m = HashMap()`
                    // produces a fresh empty `Arc<HashMapKindedRef>` slot.
                    // Default empty variant chosen is `String` (the typical
                    // initial element type in user code; the variant tag
                    // gets specialized on first insert via clone-on-write
                    // — ckpt-3 mutation-API rebuild). Reader contract: kind
                    // == Ptr(HeapKind::HashMap), bits =
                    // Arc::into_raw::<HashMapKindedRef>.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let empty_kref = shape_value::heap_value::HashMapKindedRef::String(
                        std::sync::Arc::new(shape_value::heap_value::HashMapData::<
                            *const shape_value::v2::string_obj::StringObj,
                        >::new()),
                    );
                    let hm = std::sync::Arc::new(empty_kref);
                    self.push_kinded_slot(KindedSlot::from_hashmap(hm))?;
                }
                BuiltinFunction::SetCtor => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::RuntimeError(
                        "Set(): missing static element kind; typed Set<T> construction must be stamped by the compiler".to_string(),
                    ));
                }
                BuiltinFunction::SetCtorString | BuiltinFunction::SetCtorI64 => {
                    // W74B redrive: empty Set constructors are statically
                    // stamped by the compiler from `Set<T>` proof. The runtime
                    // never chooses an arm from the first inserted element.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let empty = match builtin {
                        BuiltinFunction::SetCtorString => {
                            std::sync::Arc::new(shape_value::heap_value::HashSetData::new_string())
                        }
                        BuiltinFunction::SetCtorI64 => {
                            std::sync::Arc::new(shape_value::heap_value::HashSetData::new_i64())
                        }
                        _ => unreachable!(),
                    };
                    let result = KindedSlot::from_hashset(empty);
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::DequeCtor => {
                    // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20,
                    // 2026-05-10): empty Deque ctor — `Deque()` takes
                    // no args at landing; `Deque([elements])`
                    // initialization is a follow-up. Build empty
                    // Arc<DequeData> and push via KindedSlot::from_deque.
                    // Reader contract: kind == Ptr(HeapKind::Deque),
                    // bits = Arc::into_raw::<DequeData>.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let empty = std::sync::Arc::new(shape_value::heap_value::DequeData::new());
                    let result = KindedSlot::from_deque(empty);
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::PriorityQueueCtor => {
                    // Wave 15 W15-priority-queue (ADR-006 §2.7.18 /
                    // Q19, 2026-05-10): empty PriorityQueue ctor —
                    // discard any args (the surface form
                    // `PriorityQueue()` takes no args at landing;
                    // `PriorityQueue([elements])` initialization is a
                    // follow-up). Build an empty
                    // `Arc::new(PriorityQueueData::new())` and push as
                    // a `KindedSlot::from_priority_queue(...)`.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let empty =
                        std::sync::Arc::new(shape_value::heap_value::PriorityQueueData::new());
                    let result = KindedSlot::from_priority_queue(empty);
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ChannelCtor => {
                    // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
                    // 2026-05-10): empty Channel ctor — `Channel()`
                    // takes no args at landing; bounded-capacity
                    // initialization is a follow-up. Build empty
                    // `Arc<ChannelData>` (interior `Mutex<ChannelInner>`)
                    // and push via `KindedSlot::from_channel`. Reader
                    // contract: kind == Ptr(HeapKind::Channel),
                    // bits = Arc::into_raw::<ChannelData>.
                    //
                    // Cross-task blocking `recv()` requires the §2.7.4
                    // task-scheduler boundary and is SURFACE'd at the
                    // method body — see
                    // `executor/objects/channel_methods.rs`.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let empty = std::sync::Arc::new(shape_value::heap_value::ChannelData::new());
                    let result = KindedSlot::from_channel(empty);
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::MutexCtor => {
                    // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
                    // `Mutex(initial_value)` builds an `Arc<MutexData>`
                    // wrapping the initial value `KindedSlot` (any
                    // kind). The initial-value share moves into the
                    // MutexInner cell — `pop_builtin_args` already
                    // consumed the arg's stack share, and
                    // `MutexData::new` takes ownership of the slot.
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Mutex() requires exactly 1 argument \
                             (initial value), got {}",
                            args.len()
                        )));
                    }
                    let initial = args.remove(0);
                    let m = std::sync::Arc::new(shape_value::heap_value::MutexData::new(initial));
                    let result = KindedSlot::from_mutex(m);
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::AtomicCtor => {
                    // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
                    // `Atomic(initial)` builds an `Arc<AtomicData>`
                    // wrapping a `std::sync::atomic::AtomicI64`.
                    // i64-only at landing — non-int args error.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Atomic() requires exactly 1 argument \
                             (initial int value), got {}",
                            args.len()
                        )));
                    }
                    let initial = args[0].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "Atomic() argument must be an int (got \
                             kind {:?}); typed-payload Atomic<T> is a \
                             future amendment per ADR-006 §2.7.25",
                            args[0].kind
                        ))
                    })?;
                    let a = std::sync::Arc::new(shape_value::heap_value::AtomicData::new(initial));
                    let result = KindedSlot::from_atomic(a);
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::LazyCtor => {
                    // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
                    // `Lazy(|| ...)` builds an `Arc<LazyData>` wrapping
                    // the initializer closure. The closure share moves
                    // into the LazyInner cell — the handler tier's
                    // `lazy.get()` takes the initializer back out via
                    // `take_initializer()` for the
                    // `vm.call_value_immediate_nb` invocation.
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Lazy() requires exactly 1 argument \
                             (initializer closure), got {}",
                            args.len()
                        )));
                    }
                    let initializer = args.remove(0);
                    // Kind-validate: must be a Closure (closure-call
                    // path goes through `call_value_immediate_nb` which
                    // requires Ptr(HeapKind::Closure) callee kind).
                    if !matches!(
                        initializer.kind,
                        shape_value::NativeKind::Ptr(shape_value::heap_value::HeapKind::Closure)
                    ) {
                        return Err(VMError::RuntimeError(format!(
                            "Lazy() argument must be a closure (got \
                             kind {:?})",
                            initializer.kind
                        )));
                    }
                    let l =
                        std::sync::Arc::new(shape_value::heap_value::LazyData::new(initializer));
                    let result = KindedSlot::from_lazy(l);
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ContentTextCtor => {
                    // W18.6 (R8 W3 2026-05-24 — supervisor D3+D4):
                    // `Content.text(s: string) -> content` user-facing
                    // constructor. Pops one string arg, wraps as
                    // `ContentNode::plain(s)`, pushes as a
                    // `Ptr(HeapKind::Content)` kinded slot via the new
                    // `KindedSlot::from_content` constructor. The slot
                    // owns one `Arc<ContentNode>` strong-count share;
                    // Drop / Clone arms for `HeapKind::Content` already
                    // dispatch the matching retain/release.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Content.text() requires exactly 1 argument \
                             (string), got {}",
                            args.len()
                        )));
                    }
                    let s = args[0].as_str().ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "Content.text() argument must be a string (got \
                             kind {:?})",
                            args[0].kind
                        ))
                    })?;
                    let node = shape_value::content::ContentNode::plain(s);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ContentCodeCtor => {
                    // W18.6 (R8 W3 2026-05-24): `Content.code(source: string)
                    // -> content` minimum-viable constructor (single-arg
                    // form; no language label). Mirror of ContentTextCtor
                    // for the `ContentNode::Code` variant. Per-renderer
                    // dispatch handles the styling.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Content.code() requires exactly 1 argument \
                             (source string), got {}",
                            args.len()
                        )));
                    }
                    let s = args[0].as_str().ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "Content.code() argument must be a string (got \
                             kind {:?})",
                            args[0].kind
                        ))
                    })?;
                    let node = shape_value::content::ContentNode::Code {
                        language: None,
                        source: s.to_string(),
                    };
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ContentChart => {
                    // ChartBuilder (strict-flip SC, 2026-06): `Content.chart(t)
                    // -> content` constructor. `t` is the string carrier of a
                    // `ChartType` namespace member (`ChartType.line` lowers to
                    // the canonical `"line"` string per SC1 property-access
                    // path) or a plain string literal (`"line"`). Wraps an
                    // empty-channel `ChartSpec` of the given type into a
                    // `Ptr(HeapKind::Content)` slot — sibling to
                    // `ContentTableCtor`. Channels / title / axis labels are
                    // filled by the chart builder-method chain
                    // (`content_methods.rs` `.add` / `.title` / `.x_label` /
                    // `.y_label` / `.width` / `.height`). No Bool-default, no
                    // dynamic fallback: the chart-type string is the only arg
                    // and validates against the 9-variant `ChartType` enum.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Content.chart() requires exactly 1 argument \
                             (chart type), got {}",
                            args.len()
                        )));
                    }
                    let type_str = args[0].as_str().ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "Content.chart() argument must be a chart-type \
                             string (e.g. ChartType.line), got kind {:?}",
                            args[0].kind
                        ))
                    })?;
                    let chart_type = parse_chart_type(type_str)?;
                    let spec = shape_value::content::ChartSpec {
                        chart_type,
                        channels: Vec::new(),
                        x_categories: None,
                        title: None,
                        x_label: None,
                        y_label: None,
                        width: None,
                        height: None,
                        echarts_options: None,
                        interactive: true,
                    };
                    let node = shape_value::content::ContentNode::Chart(spec);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ContentTableCtor => {
                    // W18.5 (R8 W4, 2026-05-24 — supervisor D4):
                    // `Content.table(headers: Array<string>, rows: Array<Array<string>>)`
                    // direct ctor. Builds a `ContentNode::Table` with the
                    // provided headers + rows + default border. Sibling to
                    // the per-type `Table::new().headers(...).row(...).build()`
                    // builder (TableBuilderNew + Content method chain). Per
                    // supervisor D4 "shortest path builder → content →
                    // renderer", returns a `Ptr(HeapKind::Content)` slot
                    // directly — no intermediate typed Table value.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let table = build_table_from_headers_and_rows(&args)?;
                    let node = shape_value::content::ContentNode::Table(table);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ContentKvCtor => {
                    // W18.5 (R8 W4, 2026-05-24 — supervisor D4):
                    // `Content.kv(keys: Array<string>, values: Array<*>)`
                    // direct ctor. Pairs each key with its corresponding
                    // value formatted as a `ContentNode::plain`. Mirrors
                    // the per-type `KeyValue::new().pair("k", v).build()`
                    // builder. ContentNode::KeyValue stores `Vec<(String,
                    // ContentNode)>` so heterogeneous value types coerce
                    // through `format_kinded` (numeric / bool / string /
                    // nested content all render).
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let pairs = build_kv_pairs_from_keys_values(self, &args)?;
                    let node = shape_value::content::ContentNode::KeyValue(pairs);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ContentFragmentCtor => {
                    // W18.5 (R8 W4, 2026-05-24 — supervisor D4):
                    // `Content.fragment(parts: Array<content>)` direct
                    // ctor. Wraps a sequence of Content nodes into a
                    // single `ContentNode::Fragment` for composition.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let parts = collect_content_nodes_from_array_arg(&args)?;
                    let node = shape_value::content::ContentNode::Fragment(parts);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::ColorRgbCtor => {
                    // SC1 (R8 — supervisor): `Color.rgb(r, g, b) -> string`.
                    // The runtime carrier for every style spec (Color /
                    // Border / ChartType) is a `NativeKind::String` holding
                    // the canonical spec text; this arm builds the explicit
                    // `rgb(r,g,b)` form from three proven-int channel values.
                    // The named members (`Color.red`, etc.) are emitted as
                    // compile-time `Constant::String` by the property-access
                    // path, so this is the only style-spec arm needing a
                    // runtime builtin. Channels validate to 0–255; the
                    // string is consumed by the existing string-typed
                    // `.border(style)` method and the future `.fg`/`.bg`
                    // parsers (no new HeapKind, no parallel discriminator).
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 3 {
                        return Err(VMError::RuntimeError(format!(
                            "Color.rgb() requires exactly 3 arguments (r, g, b), got {}",
                            args.len()
                        )));
                    }
                    let channel = |idx: usize, name: &str| -> Result<u8, VMError> {
                        let v = args[idx].as_i64().ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Color.rgb(): {} channel must be an int (got kind {:?})",
                                name, args[idx].kind
                            ))
                        })?;
                        if !(0..=255).contains(&v) {
                            return Err(VMError::RuntimeError(format!(
                                "Color.rgb(): {} channel {} out of range 0–255",
                                name, v
                            )));
                        }
                        Ok(v as u8)
                    };
                    let r = channel(0, "red")?;
                    let g = channel(1, "green")?;
                    let b = channel(2, "blue")?;
                    let spec = format!("rgb({},{},{})", r, g, b);
                    let result = KindedSlot::from_string_arc(std::sync::Arc::new(spec));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::TableBuilderNew => {
                    // W18.5 (R8 W4, 2026-05-24 — supervisor D4):
                    // `Table::new()` returns an empty `ContentNode::Table`
                    // seed. Chainable methods (`headers`, `row`, `border`,
                    // `build`) are registered in `CONTENT_METHODS` PHF and
                    // dispatched on the Content receiver. `.build()` is
                    // identity — returns the receiver. Each chained method
                    // immutably clones + mutates the underlying ContentNode
                    // and pushes a fresh `Ptr(HeapKind::Content)` slot.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if !args.is_empty() {
                        return Err(VMError::RuntimeError(format!(
                            "Table::new() takes no arguments, got {}",
                            args.len()
                        )));
                    }
                    let empty = shape_value::content::ContentTable {
                        headers: Vec::new(),
                        rows: Vec::new(),
                        border: shape_value::content::BorderStyle::default(),
                        max_rows: None,
                        column_types: None,
                        total_rows: None,
                        sortable: false,
                    };
                    let node = shape_value::content::ContentNode::Table(empty);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::CodeBuilderNew => {
                    // W18.5 (R8 W4, 2026-05-24 — supervisor D4):
                    // `Code::new()` returns an empty `ContentNode::Code`
                    // seed with no language and empty source. Chainable
                    // methods (`language`, `source`, `build`) live in
                    // `CONTENT_METHODS`. The W18.6 `Content.code(s)`
                    // one-liner ctor coexists — keep both per task spec:
                    // Content.code(s) is single-arg, Code::new() builder
                    // is the multi-property form.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if !args.is_empty() {
                        return Err(VMError::RuntimeError(format!(
                            "Code::new() takes no arguments, got {}",
                            args.len()
                        )));
                    }
                    let node = shape_value::content::ContentNode::Code {
                        language: None,
                        source: String::new(),
                    };
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::KeyValueBuilderNew => {
                    // W18.5 (R8 W4, 2026-05-24 — supervisor D4):
                    // `KeyValue::new()` returns an empty
                    // `ContentNode::KeyValue` seed with no pairs. Chainable
                    // `.pair(key, value)` accumulates the pair; `.build()`
                    // is identity.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if !args.is_empty() {
                        return Err(VMError::RuntimeError(format!(
                            "KeyValue::new() takes no arguments, got {}",
                            args.len()
                        )));
                    }
                    let node = shape_value::content::ContentNode::KeyValue(Vec::new());
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::FStringContentText => {
                    // R8 W4 W18.4: wrap a string as `ContentNode::plain` for
                    // literal segments of a styled f-string.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "FStringContentText requires exactly 1 argument \
                             (string), got {}",
                            args.len()
                        )));
                    }
                    let s = args[0].as_str().ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "FStringContentText argument must be a string \
                             (got kind {:?})",
                            args[0].kind
                        ))
                    })?;
                    let node = shape_value::content::ContentNode::plain(s);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::FStringContentStyledText => {
                    // R8 W4 W18.4: wrap a string as a styled
                    // `ContentNode::Text` (single span). Args:
                    // `[value_str, fg_kind, fg_payload, bg_kind, bg_payload,
                    //   flags]` (see opcode_defs comment for encoding).
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 6 {
                        return Err(VMError::RuntimeError(format!(
                            "FStringContentStyledText requires 6 arguments \
                             (value, fg_kind, fg_payload, bg_kind, \
                             bg_payload, flags), got {}",
                            args.len()
                        )));
                    }
                    let value = args[0].as_str().ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "FStringContentStyledText value must be a string \
                             (got kind {:?})",
                            args[0].kind
                        ))
                    })?;
                    let fg_kind = args[1].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText fg_kind must be int".to_string(),
                        )
                    })?;
                    let fg_payload = args[2].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText fg_payload must be int".to_string(),
                        )
                    })?;
                    let bg_kind = args[3].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText bg_kind must be int".to_string(),
                        )
                    })?;
                    let bg_payload = args[4].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText bg_payload must be int".to_string(),
                        )
                    })?;
                    let flags = args[5].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText flags must be int".to_string(),
                        )
                    })?;

                    let style =
                        decode_fstring_style(fg_kind, fg_payload, bg_kind, bg_payload, flags)?;
                    let node = shape_value::content::ContentNode::styled(value, style);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::FStringContentChart => {
                    // R8 W6 host residuals: `{value: chart(...), x(...),
                    // y(...)}` lowers here with the original typed value
                    // still intact. The helper validates the carrier as a
                    // schema-backed table or typed-object array, then builds
                    // chart channels from numeric typed fields.
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let result = build_fstring_content_chart(self, &args)?;
                    self.push_kinded_slot(result)?;
                }
                BuiltinFunction::FStringContentFragment => {
                    // R8 W4 W18.4: combine N content nodes into a Fragment.
                    // Per ADR-006 §2.3 v2-raw-heap: `Ptr(HeapKind::Content)`
                    // slots store `Arc::into_raw(Arc<ContentNode>)` bits
                    // directly (NOT a `Box<HeapValue>` wrapper). Classify
                    // on `args[i].kind`, then deref the raw `*const
                    // ContentNode` (mirror of `printing.rs::format_heap_
                    // kind`'s Content arm and the `Arc::decrement_strong_
                    // count::<ContentNode>` in `heap_value.rs::drop_with_
                    // kind` HeapKind::Content arm).
                    let args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let mut nodes: Vec<shape_value::content::ContentNode> =
                        Vec::with_capacity(args.len());
                    for (i, arg) in args.iter().enumerate() {
                        if arg.kind != shape_value::NativeKind::Ptr(shape_value::HeapKind::Content)
                        {
                            return Err(VMError::RuntimeError(format!(
                                "FStringContentFragment arg #{} must be a \
                                 content value (got kind {:?})",
                                i, arg.kind
                            )));
                        }
                        let bits = arg.slot.raw();
                        if bits == 0 {
                            return Err(VMError::RuntimeError(format!(
                                "FStringContentFragment arg #{} is a null \
                                 content pointer",
                                i
                            )));
                        }
                        // SAFETY: per the `KindedSlot::from_content`
                        // construction contract (and §heap_value.rs::
                        // drop_with_kind HeapKind::Content arm), a
                        // `Ptr(HeapKind::Content)` slot's bits are
                        // `Arc::into_raw(Arc<ContentNode>)`. The borrow is
                        // bounded by `args`'s lifetime; the underlying
                        // share survives because `args[i]` still owns it.
                        // We deep-clone the `ContentNode` (cheap — the
                        // node enum carries owned strings + Vecs) into the
                        // Fragment because the receiving Vec owns its
                        // elements.
                        let node: &shape_value::content::ContentNode =
                            unsafe { &*(bits as *const shape_value::content::ContentNode) };
                        nodes.push(node.clone());
                    }
                    let node = shape_value::content::ContentNode::Fragment(nodes);
                    let result = KindedSlot::from_content(std::sync::Arc::new(node));
                    self.push_kinded_slot(result)?;
                }
                // ── Wave 5e: DateTime constructor builtins ────────────────
                //
                // DateTime values are `HeapValue::Temporal` carrying
                // `TemporalData::DateTime` (ADR-006 §2.3 typed-Arc payload);
                // the constructor bodies live in
                // `executor/builtins/datetime_builtins.rs` on the
                // `&[KindedSlot] -> Result<KindedSlot, VMError>` carrier ABI.
                BuiltinFunction::DateTimeNow => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins::builtin_datetime_now(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeUtc => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins::builtin_datetime_utc(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeParse => {
                    let args = self.pop_builtin_args()?;
                    let r =
                        super::super::builtins::datetime_builtins::builtin_datetime_parse(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeFromEpoch => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins::builtin_datetime_from_epoch(
                        &args,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeFromParts => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins::builtin_datetime_from_parts(
                        &args,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeFromUnixSecs => {
                    let args = self.pop_builtin_args()?;
                    let r =
                        super::super::builtins::datetime_builtins::builtin_datetime_from_unix_secs(
                            &args,
                        )?;
                    self.push_kinded_slot(r)?;
                }
                // ── Wave 5e: mat() row-major matrix constructor ───────────
                BuiltinFunction::MatFromFlat => {
                    let args = self.pop_builtin_args()?;
                    let r =
                        super::super::builtins::datetime_builtins::builtin_mat_from_flat(&args)?;
                    self.push_kinded_slot(r)?;
                }
                // ── Wave 5e: Table<T> from-rows constructor ───────────────
                BuiltinFunction::MakeTableFromRows => {
                    let args = self.pop_builtin_args()?;
                    let r = self.builtin_make_table_from_rows(&args)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::JsonObjectGet
                | BuiltinFunction::JsonArrayAt
                | BuiltinFunction::JsonObjectKeys
                | BuiltinFunction::JsonArrayLen
                | BuiltinFunction::JsonObjectLen => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5e —
                    // JSON navigation helper body migration deferred.
                    // Rebuild target lives at
                    // `executor/builtins/json_helpers.rs`. Drain args to
                    // balance the §2.7.7 parallel-kind track.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "phase-1b-vm-wave-5e-json-nav: {:?} body migration \
                         to kinded carrier (executor/builtins/json_helpers.rs) \
                         pending (v0.4 / planned)",
                        builtin
                    )));
                }
                // ── W8-WJ: window function dispatch (ADR-006 §2.7.10/Q11) ──
                //
                // Each handler is a free fn matching the MethodFnV2 body
                // shape: `fn(&mut VM, &[KindedSlot], Option<&mut Ctx>) ->
                // Result<KindedSlot, VMError>`. The dispatch shell pops
                // builtin args via `pop_builtin_args` (which constructs
                // `Vec<KindedSlot>` from the §2.7.7 stack parallel-kind
                // track), borrows it as `&[KindedSlot]` to the handler,
                // then re-pushes the kinded result via `push_kinded_slot`.
                BuiltinFunction::WindowRowNumber
                | BuiltinFunction::WindowRank
                | BuiltinFunction::WindowDenseRank
                | BuiltinFunction::WindowNtile => {
                    let args = self.pop_builtin_args()?;
                    let r =
                        super::super::window_join::handle_window_row_number_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowLag | BuiltinFunction::WindowLead => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_lag_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowFirstValue
                | BuiltinFunction::WindowLastValue
                | BuiltinFunction::WindowNthValue => {
                    let args = self.pop_builtin_args()?;
                    let r =
                        super::super::window_join::handle_window_first_value_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowSum => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_sum_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowAvg => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_avg_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowMin => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_min_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowMax => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_max_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowCount => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_count_v2(self, &args, _ctx)?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::JoinExecute => {
                    // SURFACE — cross-cluster cascade with
                    // `datatable_methods::joins` ABI flip (W9 method-body
                    // re-fill). Drains stack args to keep the parallel-
                    // kind track balanced, then surfaces.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return self.handle_join_execute();
                }
                BuiltinFunction::Reflect => {
                    // SURFACE per ADR-006 §2.7.14: phase-1b-vm wave 5e —
                    // `reflect()` builtin body migration deferred. No arg
                    // pop because the legacy emit shape did not emit an
                    // arity prefix at this dispatch arm; the rebuild
                    // wires arg-popping along with the body re-fill.
                    return Err(VMError::NotImplemented(
                        "phase-1b-vm-wave-5e-reflect: reflect() body \
                         migration to kinded carrier pending \
                         (v0.4 / planned)"
                            .to_string(),
                    ));
                }

                // ── Eval-* removed-feature stubs (preserved as runtime
                // errors per pre-Wave 5a behaviour). These do not need
                // body migration; their semantics is already terminal. ──
                BuiltinFunction::EvalTimeRef => {
                    return Err(VMError::NotImplemented(
                        "eval_time_ref() (VM-only mode)".to_string(),
                    ));
                }
                BuiltinFunction::EvalDateTimeExpr => {
                    // C1-temporal-lowering (Phase 2d Wave 2): the
                    // `compiler/expressions/temporal.rs::compile_expr_datetime`
                    // emit sequence is PushConst(DateTimeExpr) +
                    // BuiltinCall(EvalDateTimeExpr). The
                    // `Constant::DateTimeExpr` arm in `op_push_const`
                    // (`stack_ops/mod.rs`) now evaluates the AST via
                    // `eval_datetime_expr_recursive` and pushes a
                    // `NativeKind::Ptr(HeapKind::Temporal)` Temporal::DateTime
                    // slot directly. There is therefore no work for this
                    // builtin to do — the value the legacy semantics
                    // produced ("pop DateTimeExpr Temporal, evaluate, push
                    // DateTime Temporal") is already on the stack. Skip
                    // arg-count pop: the compiler does not emit one, and
                    // re-adding it would require changing the legacy emit
                    // shape compiler-side without an upstream benefit.
                    // ADR-006 §2.7.4.
                }
                BuiltinFunction::EvalDataDateTimeRef
                | BuiltinFunction::EvalDataSet
                | BuiltinFunction::EvalDataRelative
                | BuiltinFunction::EvalDataRelativeRange => {
                    return Err(VMError::RuntimeError(
                        "DataReference / DataRow type has been removed".to_string(),
                    ));
                }
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    // ===== Print / Format helpers (ADR-006 §2.7.4) =====

    /// Format every arg via `ValueFormatter::format_kinded`, join the
    /// rendered fragments with a space, then route through the active
    /// `ExecutionContext`'s [`OutputAdapter::print`] (or fall back to
    /// stdout when no context is plumbed — e.g. the bytecode-level
    /// `eval_*` helpers used by tests).
    ///
    /// W18.6 (R8 W3 2026-05-24 — supervisor D3+D4): TypedObject args
    /// whose schema's source-level type has a user-defined `Display` impl
    /// dispatch through `<TypeName>::display() -> content`. The returned
    /// `Content` KindedSlot is then routed through the formatter's
    /// `HeapKind::Content` arm (W18.2-wired TerminalRenderer path), so
    /// `print(Point { x: 1, y: 2 })` produces the user's
    /// `Content.text("(1, 2)")` projection rather than the schema-walk
    /// `{x: 1, y: 2}` fallback.
    pub(crate) fn builtin_print(
        &mut self,
        args: &[KindedSlot],
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        // The TypedObject schema names live on `self.program.type_schema_registry`
        // (the BytecodeProgram-bound registry that `lookup_schema` reads).
        // The ExecutionContext's registry is the runtime-tier copy populated
        // via stdlib loading; both are searched so user-defined types and
        // stdlib types both resolve.
        //
        // W18.6: walk args; for each TypedObject arg whose source-level
        // schema name has a registered `Display::display` impl, invoke
        // it and substitute the returned `content` carrier before formatting.
        // We need to materialize owned KindedSlots either way (the
        // formatter borrows; Display dispatch produces fresh ones), so
        // the loop builds a `Vec<KindedSlot>` of values-to-format.
        let mut to_format: Vec<KindedSlot> = Vec::with_capacity(args.len());
        for a in args {
            if let Some(replaced) = self.try_dispatch_display(a)? {
                to_format.push(replaced);
            } else {
                // Borrow-share: Clone bumps refcount so the owned slot
                // can drop without disturbing the caller's share.
                to_format.push(a.clone());
            }
        }

        let rendered = {
            let formatter =
                super::super::printing::ValueFormatter::new(&self.program.type_schema_registry);
            to_format
                .iter()
                .map(|a| formatter.format_kinded(a))
                .collect::<Vec<_>>()
                .join(" ")
        };

        let result = shape_runtime::print_result::PrintResult {
            rendered,
            spans: Vec::new(),
        };
        // Route to the active `OutputAdapter` when an `ExecutionContext`
        // is plumbed (script runner, REPL, shape-server playground /
        // notebook — all of which install a capture/REPL/stdout adapter).
        // Fall back to stdout only when no context was supplied, e.g.
        // the bytecode-level `eval_*` helpers in tests.
        //
        // W18.6 (R8 W3 2026-05-24) originally dropped `ctx` here because
        // the Display dispatch loop took `&mut self` for an opaque
        // duration — but `try_dispatch_display` does not need `ctx` (the
        // Display body is expected to be pure per the inline doc on
        // `try_dispatch_display`), so the `&mut` borrow on `ctx` is
        // free at this point. Routing the rendered line to the adapter
        // restores hosted-embedder capture (`SharedCaptureAdapter` for
        // shape-server, `ReplAdapter` for REPL spans) without touching
        // the W18.6 Display-trait dispatch above.
        if let Some(ctx) = ctx {
            ctx.output_adapter_mut().print(result);
        } else {
            println!("{}", result.rendered);
        }
        Ok(())
    }

    /// W18.6 (R8 W3 2026-05-24 — supervisor D3+D4): if `arg` is a
    /// TypedObject whose source-level schema name has a registered
    /// `Display::display` trait impl, invoke `<TypeName>::display()` and
    /// return the produced `content` KindedSlot. Returns `Ok(None)` when
    /// the arg is not a TypedObject, the schema has no name, or no
    /// Display impl is registered.
    ///
    /// `_ctx` is currently dropped — `execute_function_by_name` accepts
    /// an `Option<&mut ExecutionContext>`, but the borrow lifetime in
    /// the caller's loop is incompatible with re-acquiring `ctx` per
    /// iteration. Pre-W18.6 the print path also took `ctx` by &mut and
    /// did not pass it to nested calls; the Display body is expected to
    /// be pure / cheap (a single `Content.text(...)` wrap is the
    /// canonical pattern).
    fn try_dispatch_display(&mut self, arg: &KindedSlot) -> Result<Option<KindedSlot>, VMError> {
        use shape_value::heap_value::HeapKind;
        // Only TypedObject receivers can have user-defined Display impls.
        let shape_value::NativeKind::Ptr(HeapKind::TypedObject) = arg.kind else {
            return Ok(None);
        };
        let bits = arg.slot.raw();
        if bits == 0 {
            return Ok(None);
        }
        // SAFETY: per the `KindedSlot::from_typed_object` construction-
        // side contract, `Ptr(TypedObject)` bits are
        // `Arc::into_raw(Arc<TypedObjectStorage>)`. The borrow is
        // bounded by the caller's `args` lifetime.
        let storage: &shape_value::heap_value::TypedObjectStorage =
            unsafe { &*(bits as *const shape_value::heap_value::TypedObjectStorage) };
        let schema = self
            .program
            .type_schema_registry
            .get_by_id(storage.schema_id as u32);
        let Some(schema) = schema else {
            return Ok(None);
        };
        let type_name = schema.name.clone();
        if type_name.is_empty() {
            return Ok(None);
        }
        // Skip enum types — their `format_enum_typed_object` path
        // already produces the right `Variant(payload)` shape and
        // W18.0 enum-variant-display did not introduce a Display impl
        // for enums.
        if schema.is_enum() {
            return Ok(None);
        }
        let Some(func_name) = self
            .program
            .find_default_trait_impl_for_type_method(&type_name, "display")
            .map(|s| s.to_string())
        else {
            return Ok(None);
        };
        // Build the args vector: receiver (self) is the TypedObject
        // arg. `execute_function_by_id` takes `Vec<KindedSlot>` by value
        // and drops each slot at scope exit, retiring one share per
        // arg. `.clone()` bumps the receiver's refcount so the caller's
        // share is preserved.
        let receiver = arg.clone();
        let result = self.execute_function_by_name(&func_name, vec![receiver], None)?;
        Ok(Some(result))
    }

    /// Format every arg via `ValueFormatter::format_kinded` and
    /// concatenate (no separator). Returns the rendered text wrapped in
    /// a `String`-kinded `KindedSlot`. Used by `format(…)` (multi-arg
    /// concat) and by `FormatValueWithMeta` (single-arg
    /// `expr.to_string()` / interpolation).
    pub(crate) fn builtin_format(&mut self, args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
        let mut to_format: Vec<KindedSlot> = Vec::with_capacity(args.len());
        for a in args {
            if let Some(replaced) = self.try_dispatch_display(a)? {
                to_format.push(replaced);
            } else {
                to_format.push(a.clone());
            }
        }

        let formatter =
            super::super::printing::ValueFormatter::new(&self.program.type_schema_registry);
        let mut out = String::new();
        for a in &to_format {
            out.push_str(&formatter.format_kinded(a));
        }
        Ok(KindedSlot::from_string_arc(std::sync::Arc::new(out)))
    }

    /// `FormatValueWithSpec`: `[value, spec_tag, …spec-payload]`. Routes
    /// the FORMAT_SPEC_FIXED arm (precision-controlled f64 rendering);
    /// the Table arm surfaces per W13 playbook §7.4 surface-and-stop.
    pub(crate) fn builtin_format_with_spec(
        &mut self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        const FORMAT_SPEC_FIXED: i64 = 1;
        const FORMAT_SPEC_TABLE: i64 = 2;

        if args.is_empty() {
            return Err(VMError::RuntimeError(
                "FormatValueWithSpec requires at least 1 argument".to_string(),
            ));
        }

        // The spec_tag arrives as an `int` constant (`PushConst(Constant::Int(_))`)
        // — kind `Int64` in the post-§2.7.7 stack ABI. Read defensively:
        // kind-mismatch falls through to the meta path so a malformed
        // dispatch still produces a string rather than crashing.
        let spec_tag = args.get(1).and_then(|s| match s.kind {
            shape_value::NativeKind::Int64
            | shape_value::NativeKind::Int32
            | shape_value::NativeKind::Int16
            | shape_value::NativeKind::Int8
            | shape_value::NativeKind::IntSize => Some(s.slot.as_i64()),
            _ => None,
        });

        match spec_tag {
            Some(tag) if tag == FORMAT_SPEC_FIXED => {
                let precision = args.get(2).and_then(|s| match s.kind {
                    shape_value::NativeKind::Int64
                    | shape_value::NativeKind::Int32
                    | shape_value::NativeKind::Int16
                    | shape_value::NativeKind::Int8
                    | shape_value::NativeKind::IntSize => Some(s.slot.as_i64()),
                    _ => None,
                });
                let v = &args[0];
                // Coerce numeric kinds; non-numeric fall back to default
                // formatting so the spec is a no-op rather than an error.
                let f = match v.kind {
                    shape_value::NativeKind::Float64 | shape_value::NativeKind::NullableFloat64 => {
                        Some(v.slot.as_f64())
                    }
                    shape_value::NativeKind::Int64
                    | shape_value::NativeKind::Int32
                    | shape_value::NativeKind::Int16
                    | shape_value::NativeKind::Int8
                    | shape_value::NativeKind::IntSize => Some(v.slot.as_i64() as f64),
                    shape_value::NativeKind::UInt64
                    | shape_value::NativeKind::UInt32
                    | shape_value::NativeKind::UInt16
                    | shape_value::NativeKind::UInt8
                    | shape_value::NativeKind::UIntSize => Some(v.slot.as_u64() as f64),
                    _ => None,
                };
                let rendered = match (f, precision) {
                    (Some(f), Some(p)) if p >= 0 => {
                        format!("{:.*}", p as usize, f)
                    }
                    _ => self
                        .builtin_format(&args[..1])?
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                };
                Ok(KindedSlot::from_string_arc(std::sync::Arc::new(rendered)))
            }
            Some(tag) if tag == FORMAT_SPEC_TABLE => Err(VMError::NotImplemented(
                "FormatValueWithSpec: FORMAT_SPEC_TABLE rendering deferred — \
                     W13-print-formatter scope is the FORMAT_SPEC_FIXED + \
                     no-spec path. Table rendering reuses the DataTable / \
                     TableView Display impls; surface-and-stop pending the \
                     next pass per W13 playbook §7.4."
                    .to_string(),
            )),
            _ => self.builtin_format(&args[..1]),
        }
    }

    // Runtime bridge functions (pop_builtin_args impl, eval_runtime_*)
    // moved to builtins/runtime_bridge.rs.
    // map_runtime_error and type_of_name moved to module_registry module.

    // ===== Helper Methods =====
    // binary_arithmetic, eval_runtime_binary_op_value, binary_comparison
    // moved to arithmetic/mod.rs
}

// ─────────────────────────────────────────────────────────────────────────
// Wave 8 random intrinsic carriers.
//
// The RNG state itself stays in shape-runtime's thread-local `with_rng`
// hook so VM intrinsics and typed runtime modules observe one deterministic
// sequence. Carrier construction stays here and is statically known:
// Float64, Null/void, or v2 `TypedArray<f64>`.
// ─────────────────────────────────────────────────────────────────────────

fn random_intrinsic_arity(
    builtin: crate::bytecode::BuiltinFunction,
    args: &[KindedSlot],
    expected: usize,
) -> Result<(), VMError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(VMError::RuntimeError(format!(
            "{:?} expects {} argument(s), got {}",
            builtin,
            expected,
            args.len()
        )))
    }
}

fn random_number_arg(
    builtin: crate::bytecode::BuiltinFunction,
    args: &[KindedSlot],
    idx: usize,
    name: &str,
) -> Result<f64, VMError> {
    crate::executor::builtins::kind_coerce::number_operand(&args[idx]).map_err(|_| {
        VMError::RuntimeError(format!(
            "{:?}: argument '{}' must be number (got kind {:?})",
            builtin, name, args[idx].kind
        ))
    })
}

fn random_int_arg(
    builtin: crate::bytecode::BuiltinFunction,
    args: &[KindedSlot],
    idx: usize,
    name: &str,
) -> Result<i64, VMError> {
    crate::executor::builtins::kind_coerce::int_operand(&args[idx]).map_err(|_| {
        VMError::RuntimeError(format!(
            "{:?}: argument '{}' must be int (got kind {:?})",
            builtin, name, args[idx].kind
        ))
    })
}

fn random_array_number_slot(values: Vec<f64>) -> KindedSlot {
    use shape_value::v2::typed_array::{ELEM_TYPE_F64, TypedArray, stamp_elem_type};

    let ptr = TypedArray::<f64>::from_slice(&values);
    unsafe {
        stamp_elem_type(ptr as *mut u8, ELEM_TYPE_F64);
    }
    KindedSlot::new(
        ValueSlot::from_u64(ptr as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

fn vm_random_intrinsic_slot(
    builtin: crate::bytecode::BuiltinFunction,
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    use rand::{Rng as _, SeedableRng as _};

    match builtin {
        crate::bytecode::BuiltinFunction::IntrinsicRandom => {
            random_intrinsic_arity(builtin, args, 0)?;
            let value = shape_runtime::intrinsics::random::with_rng(|rng| rng.r#gen::<f64>());
            Ok(KindedSlot::from_number(value))
        }
        crate::bytecode::BuiltinFunction::IntrinsicRandomInt => {
            random_intrinsic_arity(builtin, args, 2)?;
            let lo = random_number_arg(builtin, args, 0, "lo")? as i64;
            let hi = random_number_arg(builtin, args, 1, "hi")? as i64;
            if lo > hi {
                return Err(VMError::RuntimeError(format!(
                    "__intrinsic_random_int: lo ({}) must be <= hi ({})",
                    lo, hi
                )));
            }
            let value = shape_runtime::intrinsics::random::with_rng(|rng| rng.gen_range(lo..=hi));
            Ok(KindedSlot::from_number(value as f64))
        }
        crate::bytecode::BuiltinFunction::IntrinsicRandomSeed => {
            random_intrinsic_arity(builtin, args, 1)?;
            let seed = random_number_arg(builtin, args, 0, "seed")? as u64;
            shape_runtime::intrinsics::random::with_rng(|rng| {
                *rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            });
            Ok(KindedSlot::none())
        }
        crate::bytecode::BuiltinFunction::IntrinsicRandomNormal => {
            random_intrinsic_arity(builtin, args, 2)?;
            let mean = random_number_arg(builtin, args, 0, "mean")?;
            let std = random_number_arg(builtin, args, 1, "std")?;
            if std < 0.0 {
                return Err(VMError::RuntimeError(
                    "__intrinsic_random_normal: std must be non-negative".to_string(),
                ));
            }
            let value = shape_runtime::intrinsics::random::with_rng(|rng| {
                let u1: f64 = rng.r#gen();
                let u2: f64 = rng.r#gen();
                let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                mean + std * z
            });
            Ok(KindedSlot::from_number(value))
        }
        crate::bytecode::BuiltinFunction::IntrinsicRandomArray => {
            random_intrinsic_arity(builtin, args, 1)?;
            let n = random_int_arg(builtin, args, 0, "n")?;
            if n < 0 {
                return Err(VMError::RuntimeError(
                    "__intrinsic_random_array: n must be non-negative".to_string(),
                ));
            }
            let values = shape_runtime::intrinsics::random::with_rng(|rng| {
                (0..n as usize)
                    .map(|_| rng.r#gen::<f64>())
                    .collect::<Vec<f64>>()
            });
            Ok(random_array_number_slot(values))
        }
        other => Err(VMError::RuntimeError(format!(
            "vm_random_intrinsic_slot called with non-random builtin {:?}",
            other
        ))),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// W18.5 content builder helpers (R8 W4, 2026-05-24 — supervisor D4).
//
// Free-function helpers (not VirtualMachine methods) used by the
// `Content.table` / `Content.kv` / `Content.fragment` constructor arms and
// the Content method handlers in `objects/content_methods.rs`. The
// builder pattern relies on these helpers to:
//   - read a `Ptr(HeapKind::Content)` slot back into a `ContentNode`
//   - read string elements from a v2 `TypedArray<*const StringObj>`
//   - format a heterogeneous `KindedSlot` value into a `ContentNode::plain`
//     cell (for `Table.row(...)` / `KeyValue.pair("k", v)` value coercion)
//
// Per supervisor D4 "shortest path builder → content → renderer", these
// helpers stay at the dispatch shell — no cross-crate detour into
// shape-runtime, no parallel-implementation of styling spec types (the
// W18.4 shared spec module is a follow-up — see commit message).
// ─────────────────────────────────────────────────────────────────────────

/// Read string elements from a v2 `TypedArray<*const StringObj>` slot.
///
/// Returns `None` if the slot is not a v2-raw typed array of strings; the
/// caller surfaces a typed error. The returned `Vec<String>` owns its
/// contents — each element is copied out of the array's interned UTF-8.
pub(in crate::executor) fn read_string_array(slot: &KindedSlot) -> Option<Vec<String>> {
    use crate::executor::v2_handlers::v2_array_detect::{
        V2ElemType, as_v2_typed_array, read_element,
    };
    use shape_value::{HeapKind, NativeKind};
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return None;
    }
    let view = as_v2_typed_array(slot.slot.raw(), slot.kind)?;
    if view.elem_type != V2ElemType::String {
        return None;
    }
    let mut out = Vec::with_capacity(view.len as usize);
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i)?;
        // `read_element` retains a fresh share on the element header per
        // its Wave-2-Agent-A2 contract (see
        // `executor/v2_handlers/v2_array_detect.rs:378-388`); wrap into
        // a `KindedSlot` so the share retires on drop after we've copied
        // the UTF-8 out via `as_str`.
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let s = elem_slot.as_str()?.to_string();
        // `elem_slot` drops at end of iteration — releases the share.
        let _ = elem_slot;
        out.push(s);
    }
    Some(out)
}

/// Read a `Ptr(HeapKind::Content)` slot as an `Arc<ContentNode>`.
///
/// Returns `None` if the kind doesn't match. The returned Arc is a fresh
/// strong-count share (incremented from the slot's bits); the caller is
/// responsible for the share-accounting of the returned Arc.
pub(in crate::executor) fn read_content_arc(
    slot: &KindedSlot,
) -> Option<std::sync::Arc<shape_value::content::ContentNode>> {
    use shape_value::{HeapKind, NativeKind};
    if slot.kind != NativeKind::Ptr(HeapKind::Content) {
        return None;
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return None;
    }
    // SAFETY: by construction `Ptr(HeapKind::Content)` slot bits are
    // `Arc::into_raw(Arc<ContentNode>) as u64` (set by
    // `KindedSlot::from_content` and its producers — `ContentTextCtor`,
    // `ContentCodeCtor`, this module's W18.5 ctors, Display.display()
    // returns, etc.). The slot owns one strong-count share for the
    // dispatch duration. We borrow the inner Arc by reconstituting it,
    // cloning to get a fresh share, then `mem::forget`-ing the original
    // reconstitution so the slot's share remains intact.
    unsafe {
        let raw = bits as *const shape_value::content::ContentNode;
        let arc = std::sync::Arc::from_raw(raw);
        let cloned = arc.clone();
        std::mem::forget(arc);
        Some(cloned)
    }
}

/// Parse a chart-type string (the carrier of a `ChartType` namespace member,
/// or a plain string literal) into the matching `ChartType` enum variant.
///
/// The user-facing member names mirror `is_style_spec_member`
/// (`property_access.rs`): the book writes `boxplot` one word while the Rust
/// variant is `BoxPlot` (serde `box_plot`), so both spellings are accepted.
/// Unknown strings reject cleanly — no Bool-default, no dynamic fallback.
pub fn parse_chart_type(s: &str) -> Result<shape_value::content::ChartType, VMError> {
    use shape_value::content::ChartType;
    match s.to_ascii_lowercase().as_str() {
        "line" => Ok(ChartType::Line),
        "bar" => Ok(ChartType::Bar),
        "scatter" => Ok(ChartType::Scatter),
        "area" => Ok(ChartType::Area),
        "candlestick" => Ok(ChartType::Candlestick),
        "histogram" => Ok(ChartType::Histogram),
        "boxplot" | "box_plot" => Ok(ChartType::BoxPlot),
        "heatmap" => Ok(ChartType::Heatmap),
        "bubble" => Ok(ChartType::Bubble),
        other => Err(VMError::RuntimeError(format!(
            "Content.chart(): unknown chart type '{}' — expected one of \
             line, bar, scatter, area, candlestick, histogram, boxplot, \
             heatmap, bubble (e.g. ChartType.line)",
            other
        ))),
    }
}

struct ChartProjection {
    x: Option<(String, Vec<f64>)>,
    y: Vec<(String, Vec<f64>)>,
}

fn build_fstring_content_chart(
    vm: &VirtualMachine,
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if args.len() < 4 {
        return Err(VMError::RuntimeError(format!(
            "FStringContentChart requires value, chart type, x column, and \
             at least one y column; got {} arguments",
            args.len()
        )));
    }
    let chart_type_str = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FStringContentChart chart type must be a string, got kind {:?}",
            args[1].kind
        ))
    })?;
    let chart_type = parse_chart_type(chart_type_str)?;
    let x_column_raw = args[2].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FStringContentChart x column must be a string, got kind {:?}",
            args[2].kind
        ))
    })?;
    let x_column = if x_column_raw.is_empty() {
        None
    } else {
        Some(x_column_raw)
    };
    let mut y_columns = Vec::with_capacity(args.len().saturating_sub(3));
    for arg in &args[3..] {
        let name = arg.as_str().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "FStringContentChart y column must be a string, got kind {:?}",
                arg.kind
            ))
        })?;
        if name.is_empty() {
            return Err(VMError::RuntimeError(
                "FStringContentChart y column cannot be empty".to_string(),
            ));
        }
        y_columns.push(name.to_string());
    }

    let projection = project_chart_columns(vm, &args[0], x_column, &y_columns)?;
    let mut channels = Vec::new();
    if let Some((label, values)) = projection.x {
        channels.push(shape_value::content::ChartChannel {
            name: "x".to_string(),
            label,
            values,
            color: None,
        });
    }
    for (label, values) in projection.y {
        channels.push(shape_value::content::ChartChannel {
            name: "y".to_string(),
            label,
            values,
            color: None,
        });
    }

    let spec = shape_value::content::ChartSpec {
        chart_type,
        channels,
        x_categories: None,
        title: None,
        x_label: x_column.map(str::to_string),
        y_label: if y_columns.len() == 1 {
            Some(y_columns[0].clone())
        } else {
            None
        },
        width: None,
        height: None,
        echarts_options: None,
        interactive: true,
    };
    Ok(KindedSlot::from_content(std::sync::Arc::new(
        shape_value::content::ContentNode::Chart(spec),
    )))
}

fn project_chart_columns(
    vm: &VirtualMachine,
    value: &KindedSlot,
    x_column: Option<&str>,
    y_columns: &[String],
) -> Result<ChartProjection, VMError> {
    use shape_value::{HeapKind, NativeKind};
    match value.kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            project_typed_object_array_chart_columns(vm, value, x_column, y_columns)
        }
        NativeKind::Ptr(HeapKind::TableView) => {
            project_table_view_chart_columns(vm, value, x_column, y_columns)
        }
        other => Err(VMError::RuntimeError(format!(
            "FStringContentChart: expected typed object array or Table<T> \
             value, got kind {:?}",
            other
        ))),
    }
}

fn project_typed_object_array_chart_columns(
    vm: &VirtualMachine,
    value: &KindedSlot,
    x_column: Option<&str>,
    y_columns: &[String],
) -> Result<ChartProjection, VMError> {
    use crate::executor::v2_handlers::v2_array_detect::{
        V2ElemType, as_v2_typed_array, read_element,
    };
    let view = as_v2_typed_array(value.raw(), value.kind).ok_or_else(|| {
        VMError::RuntimeError(
            "FStringContentChart: array value has invalid typed-array header".to_string(),
        )
    })?;
    if view.elem_type != V2ElemType::TypedObject {
        return Err(VMError::RuntimeError(format!(
            "FStringContentChart: chart array elements must be typed objects, \
             got {:?}",
            view.elem_type
        )));
    }
    if view.len == 0 {
        return Err(VMError::RuntimeError(
            "FStringContentChart: empty typed-object array has no schema-bearing \
             row to project"
                .to_string(),
        ));
    }

    let mut x_values = x_column.map(|_| Vec::with_capacity(view.len as usize));
    let mut y_values: Vec<Vec<f64>> = y_columns
        .iter()
        .map(|_| Vec::with_capacity(view.len as usize))
        .collect();
    for row_idx in 0..view.len {
        let (bits, kind) = read_element(&view, row_idx).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "FStringContentChart: failed to read typed-object row {}",
                row_idx
            ))
        })?;
        // Ownership: the elem_type guard above guarantees `read_element`
        // uses its `V2ElemType::TypedObject` arm, which calls
        // `copy_typed_object_for_bind` and returns a fresh `_new`-allocated
        // TypedObjectStorage share. Wrapping that pair in KindedSlot is
        // therefore ownership-correct; `drop(row_slot)` retires the copied
        // row, not a borrowed array element.
        let row_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let row = row_slot.as_typed_object_storage().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "FStringContentChart: row {} is not a typed object (kind {:?})",
                row_idx, kind
            ))
        })?;
        if let (Some(name), Some(values)) = (x_column, x_values.as_mut()) {
            values.push(typed_object_numeric_field(vm, row, name)?);
        }
        for (col_idx, name) in y_columns.iter().enumerate() {
            y_values[col_idx].push(typed_object_numeric_field(vm, row, name)?);
        }
        drop(row_slot);
    }

    Ok(ChartProjection {
        x: x_column.map(|name| (name.to_string(), x_values.unwrap_or_default())),
        y: y_columns.iter().cloned().zip(y_values).collect::<Vec<_>>(),
    })
}

fn typed_object_numeric_field(
    vm: &VirtualMachine,
    row: &shape_value::TypedObjectStorage,
    field_name: &str,
) -> Result<f64, VMError> {
    let schema = vm
        .program
        .type_schema_registry
        .get_by_id(row.schema_id as u32)
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "FStringContentChart: unknown typed-object schema ID {}",
                row.schema_id
            ))
        })?;
    let field = schema.get_field(field_name).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FStringContentChart: schema '{}' has no field '{}'",
            schema.name, field_name
        ))
    })?;
    ensure_chart_numeric_field_type(&schema.name, field_name, &field.field_type)?;
    let slot = row
        .clone_field_kinded(field.index as usize)
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "FStringContentChart: failed to read field '{}' from schema '{}'",
                field_name, schema.name
            ))
        })?;
    let value = chart_slot_to_f64(&slot).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FStringContentChart: field '{}.{}' must be numeric, got kind {:?}",
            schema.name, field_name, slot.kind
        ))
    })?;
    drop(slot);
    Ok(value)
}

fn project_table_view_chart_columns(
    vm: &VirtualMachine,
    value: &KindedSlot,
    x_column: Option<&str>,
    y_columns: &[String],
) -> Result<ChartProjection, VMError> {
    let (schema_id, table) = table_view_data_table(value)?;
    if table.schema_id() != Some(schema_id as u32) {
        return Err(VMError::RuntimeError(format!(
            "FStringContentChart: TableView schema ID {} does not match \
             DataTable schema ID {:?}",
            schema_id,
            table.schema_id()
        )));
    }
    let schema = vm
        .program
        .type_schema_registry
        .get_by_id(schema_id as u32)
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "FStringContentChart: unknown table schema ID {}",
                schema_id
            ))
        })?;
    let x = x_column
        .map(|name| {
            table_numeric_column(schema, table, name).map(|values| (name.to_string(), values))
        })
        .transpose()?;
    let mut y = Vec::with_capacity(y_columns.len());
    for name in y_columns {
        y.push((name.clone(), table_numeric_column(schema, table, name)?));
    }
    Ok(ChartProjection { x, y })
}

fn table_view_data_table<'a>(
    value: &'a KindedSlot,
) -> Result<(u64, &'a shape_value::DataTable), VMError> {
    use shape_value::{HeapKind, NativeKind, TableViewData};
    if value.kind != NativeKind::Ptr(HeapKind::TableView) {
        return Err(VMError::RuntimeError(format!(
            "FStringContentChart: expected TableView carrier, got {:?}",
            value.kind
        )));
    }
    let bits = value.raw();
    if bits == 0 {
        return Err(VMError::RuntimeError(
            "FStringContentChart: null TableView carrier".to_string(),
        ));
    }
    // SAFETY: Ptr(TableView) slots carry Arc::into_raw(Arc<TableViewData>).
    let table_view: &TableViewData = unsafe { &*(bits as *const TableViewData) };
    match table_view {
        TableViewData::TypedTable { schema_id, table } => Ok((*schema_id, table.as_ref())),
        other => Err(VMError::RuntimeError(format!(
            "FStringContentChart: chart projection expects a TypedTable \
             carrier, got {}",
            other.type_name()
        ))),
    }
}

fn table_numeric_column(
    schema: &shape_runtime::type_schema::TypeSchema,
    table: &shape_value::DataTable,
    name: &str,
) -> Result<Vec<f64>, VMError> {
    let field = schema.get_field(name).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FStringContentChart: schema '{}' has no field '{}'",
            schema.name, name
        ))
    })?;
    ensure_chart_numeric_field_type(&schema.name, name, &field.field_type)?;
    let column = table.column_by_name(name).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FStringContentChart: table has no column '{}'",
            name
        ))
    })?;
    if let Some(values) = column.as_any().downcast_ref::<arrow_array::Float64Array>() {
        return Ok((0..values.len()).map(|idx| values.value(idx)).collect());
    }
    if let Some(values) = column.as_any().downcast_ref::<arrow_array::Float32Array>() {
        return Ok((0..values.len())
            .map(|idx| values.value(idx) as f64)
            .collect());
    }
    if let Some(values) = column.as_any().downcast_ref::<arrow_array::Int64Array>() {
        return Ok((0..values.len())
            .map(|idx| values.value(idx) as f64)
            .collect());
    }
    if let Some(values) = column.as_any().downcast_ref::<arrow_array::Int32Array>() {
        return Ok((0..values.len())
            .map(|idx| values.value(idx) as f64)
            .collect());
    }
    Err(VMError::RuntimeError(format!(
        "FStringContentChart: column '{}' must be numeric, got Arrow type {:?}",
        name,
        column.data_type()
    )))
}

fn ensure_chart_numeric_field_type(
    type_name: &str,
    field_name: &str,
    field_type: &shape_runtime::type_schema::FieldType,
) -> Result<(), VMError> {
    use shape_runtime::type_schema::FieldType;
    if matches!(
        field_type,
        FieldType::F64
            | FieldType::I64
            | FieldType::I8
            | FieldType::U8
            | FieldType::I16
            | FieldType::U16
            | FieldType::I32
            | FieldType::U32
            | FieldType::U64
    ) {
        return Ok(());
    }
    Err(VMError::RuntimeError(format!(
        "FStringContentChart: field '{}.{}' must be numeric, got schema \
         field type {}",
        type_name, field_name, field_type
    )))
}

fn chart_slot_to_f64(slot: &KindedSlot) -> Option<f64> {
    use shape_value::NativeKind;
    match slot.kind {
        NativeKind::Float64 => Some(f64::from_bits(slot.raw())),
        NativeKind::Float32 => Some(f32::from_bits(slot.raw() as u32) as f64),
        NativeKind::Int64 => Some(slot.raw() as i64 as f64),
        NativeKind::Int32 => Some(slot.raw() as u32 as i32 as f64),
        NativeKind::Int16 => Some(slot.raw() as u16 as i16 as f64),
        NativeKind::Int8 => Some(slot.raw() as u8 as i8 as f64),
        NativeKind::UInt64 => Some(slot.raw() as f64),
        NativeKind::UInt32 => Some(slot.raw() as u32 as f64),
        NativeKind::UInt16 => Some(slot.raw() as u16 as f64),
        NativeKind::UInt8 => Some(slot.raw() as u8 as f64),
        NativeKind::IntSize => Some(slot.raw() as isize as f64),
        NativeKind::UIntSize => Some(slot.raw() as usize as f64),
        _ => None,
    }
}

/// Build a `ContentTable` from a `Content.table(headers, rows)` arg list.
///
/// `args[0]` is the headers array (`Array<string>`), `args[1]` is the
/// rows array (`Array<Array<string>>`). Cell values render as
/// `ContentNode::plain` strings — string-typed MVP per supervisor D4
/// "string-typed-MVP follow-up surfaced for W18.4 spec-types swap".
fn build_table_from_headers_and_rows(
    args: &[KindedSlot],
) -> Result<shape_value::content::ContentTable, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.table(headers, rows) requires exactly 2 arguments, \
             got {}",
            args.len()
        )));
    }
    let headers = read_string_array(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.table: headers argument must be Array<string>, got \
             kind {:?}",
            args[0].kind
        ))
    })?;
    // Rows is an Array<Array<string>>. The outer array carries
    // TypedArray-of-TypedArray pointers. Read each inner row via
    // `read_string_array`.
    use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, read_element};
    use shape_value::{HeapKind, NativeKind};
    if args[1].kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Content.table: rows argument must be Array<Array<string>>, \
             got kind {:?}",
            args[1].kind
        )));
    }
    let outer_view = as_v2_typed_array(args[1].slot.raw(), args[1].kind).ok_or_else(|| {
        VMError::RuntimeError("Content.table: rows array has invalid v2 header".to_string())
    })?;
    let mut rows: Vec<Vec<shape_value::content::ContentNode>> =
        Vec::with_capacity(outer_view.len as usize);
    for i in 0..outer_view.len {
        let (bits, kind) = read_element(&outer_view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Content.table: failed to read row {} from rows array",
                i
            ))
        })?;
        let row_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let cells = read_string_array(&row_slot).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Content.table: row {} must be Array<string>, got kind {:?}",
                i, row_slot.kind
            ))
        })?;
        // Drop row_slot to release the share held by `read_element`.
        drop(row_slot);
        let row_nodes: Vec<shape_value::content::ContentNode> = cells
            .into_iter()
            .map(shape_value::content::ContentNode::plain)
            .collect();
        rows.push(row_nodes);
    }
    Ok(shape_value::content::ContentTable {
        headers,
        rows,
        border: shape_value::content::BorderStyle::default(),
        max_rows: None,
        column_types: None,
        total_rows: None,
        sortable: false,
    })
}

/// Build a `Vec<(String, ContentNode)>` from a `Content.kv(keys, values)`
/// arg list — keys array is `Array<string>`, values array is the parallel
/// `Array<*>` whose elements coerce through `format_kinded`.
fn build_kv_pairs_from_keys_values(
    vm: &VirtualMachine,
    args: &[KindedSlot],
) -> Result<Vec<(String, shape_value::content::ContentNode)>, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.kv(keys, values) requires exactly 2 arguments, got {}",
            args.len()
        )));
    }
    let keys = read_string_array(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.kv: keys argument must be Array<string>, got kind {:?}",
            args[0].kind
        ))
    })?;
    use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, read_element};
    use shape_value::{HeapKind, NativeKind};
    if args[1].kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Content.kv: values argument must be an Array, got kind {:?}",
            args[1].kind
        )));
    }
    let view = as_v2_typed_array(args[1].slot.raw(), args[1].kind).ok_or_else(|| {
        VMError::RuntimeError("Content.kv: values array has invalid v2 header".to_string())
    })?;
    if (view.len as usize) != keys.len() {
        return Err(VMError::RuntimeError(format!(
            "Content.kv: keys.len() ({}) != values.len() ({})",
            keys.len(),
            view.len
        )));
    }
    let formatter = super::super::printing::ValueFormatter::new(&vm.program.type_schema_registry);
    let mut pairs = Vec::with_capacity(keys.len());
    for (i, key) in keys.into_iter().enumerate() {
        let (bits, kind) = read_element(&view, i as u32).ok_or_else(|| {
            VMError::RuntimeError(format!("Content.kv: failed to read value at index {}", i))
        })?;
        let val_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let rendered = formatter.format_kinded(&val_slot);
        // val_slot drops here — releases the share.
        drop(val_slot);
        pairs.push((key, shape_value::content::ContentNode::plain(rendered)));
    }
    Ok(pairs)
}

/// Read an `Array<content>` argument and collect each element as an
/// owned `ContentNode`. Each element of the array must have kind
/// `Ptr(HeapKind::Content)`; the function clones the inner ContentNode
/// out of the read-share Arc returned by `read_content_arc`.
fn collect_content_nodes_from_array_arg(
    args: &[KindedSlot],
) -> Result<Vec<shape_value::content::ContentNode>, VMError> {
    if args.len() != 1 {
        return Err(VMError::RuntimeError(format!(
            "Content.fragment(parts) requires exactly 1 argument \
             (Array<content>), got {}",
            args.len()
        )));
    }
    use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, read_element};
    use shape_value::{HeapKind, NativeKind};
    if args[0].kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Content.fragment: parts argument must be Array<content>, got \
             kind {:?}",
            args[0].kind
        )));
    }
    let view = as_v2_typed_array(args[0].slot.raw(), args[0].kind).ok_or_else(|| {
        VMError::RuntimeError("Content.fragment: parts array has invalid v2 header".to_string())
    })?;
    let mut parts = Vec::with_capacity(view.len as usize);
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!("Content.fragment: failed to read element {}", i))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let arc = read_content_arc(&elem_slot).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Content.fragment: element {} must be a content value, got \
                 kind {:?}",
                i, elem_slot.kind
            ))
        })?;
        // Clone the underlying ContentNode out of the Arc (cheap — most
        // variants are themselves Arc-shaped vectors / structs that share
        // their interior storage on clone).
        parts.push((*arc).clone());
        drop(elem_slot);
    }
    Ok(parts)
}

// W12-stdlib-intrinsic-collapse (Wave-2-Agent-G, 2026-05-14): the
// `intrinsic_sum_tests` module previously here exercised the deleted
// `BuiltinFunction::IntrinsicSum` opcode body. Equivalent coverage lives
// in the PHF method-dispatch handlers' own test surface
// (`typed_array_methods` / `array_aggregation` / `typed_int_array_methods`
// / `typed_number_array_methods`) — single discriminator per ADR-005 §1.

// ===== R8 W4 W18.4: f-string styled-content lowering helpers =====
//
// (supervisor 2026-05-24 D1 + (a-modified) REVIVE-WITH-SHARED-MODULE)
//
// The compiler at `compiler/string_interpolation.rs` lowers each styled
// interpolation `{x:bold,red}` to a `FStringContentStyledText` builtin
// call whose i64-encoded payload is decoded here back into a
// `shape_value::content::Style`.

const FSTRING_COLOR_NONE: i64 = -1;
const FSTRING_COLOR_NAMED: i64 = 0;
const FSTRING_COLOR_RGB: i64 = 1;

const FSTRING_FLAG_BOLD: i64 = 1;
const FSTRING_FLAG_ITALIC: i64 = 2;
const FSTRING_FLAG_UNDERLINE: i64 = 4;
const FSTRING_FLAG_DIM: i64 = 8;

fn decode_fstring_color(
    kind: i64,
    payload: i64,
) -> Result<Option<shape_value::content::Color>, VMError> {
    use shape_value::content::{Color, NamedColor};
    match kind {
        FSTRING_COLOR_NONE => Ok(None),
        FSTRING_COLOR_NAMED => {
            let named = match payload {
                0 => NamedColor::Red,
                1 => NamedColor::Green,
                2 => NamedColor::Blue,
                3 => NamedColor::Yellow,
                4 => NamedColor::Magenta,
                5 => NamedColor::Cyan,
                6 => NamedColor::White,
                7 => NamedColor::Default,
                other => {
                    return Err(VMError::RuntimeError(format!(
                        "decode_fstring_color: invalid named-color id {}",
                        other
                    )));
                }
            };
            Ok(Some(Color::Named(named)))
        }
        FSTRING_COLOR_RGB => {
            let r = ((payload >> 16) & 0xFF) as u8;
            let g = ((payload >> 8) & 0xFF) as u8;
            let b = (payload & 0xFF) as u8;
            Ok(Some(Color::Rgb(r, g, b)))
        }
        other => Err(VMError::RuntimeError(format!(
            "decode_fstring_color: invalid color kind {}",
            other
        ))),
    }
}

fn decode_fstring_style(
    fg_kind: i64,
    fg_payload: i64,
    bg_kind: i64,
    bg_payload: i64,
    flags: i64,
) -> Result<shape_value::content::Style, VMError> {
    Ok(shape_value::content::Style {
        fg: decode_fstring_color(fg_kind, fg_payload)?,
        bg: decode_fstring_color(bg_kind, bg_payload)?,
        bold: (flags & FSTRING_FLAG_BOLD) != 0,
        italic: (flags & FSTRING_FLAG_ITALIC) != 0,
        underline: (flags & FSTRING_FLAG_UNDERLINE) != 0,
        dim: (flags & FSTRING_FLAG_DIM) != 0,
    })
}

#[cfg(test)]
mod random_intrinsic_tests {
    use super::*;
    use crate::bytecode::BuiltinFunction;

    #[test]
    fn random_seed_restarts_shared_runtime_rng_sequence() {
        let seed_args = [KindedSlot::from_number(42.0)];
        let seed_slot =
            vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandomSeed, &seed_args).unwrap();
        assert_eq!(seed_slot.kind, NativeKind::Null);

        let first = vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandom, &[]).unwrap();
        let first_value = first.as_f64().unwrap();

        let seed_args = [KindedSlot::from_number(42.0)];
        let _ = vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandomSeed, &seed_args).unwrap();
        let second = vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandom, &[]).unwrap();
        assert_eq!(second.as_f64().unwrap(), first_value);
    }

    #[test]
    fn random_normal_returns_float64_slot() {
        let seed_args = [KindedSlot::from_number(7.0)];
        let _ = vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandomSeed, &seed_args).unwrap();
        let args = [KindedSlot::from_number(10.0), KindedSlot::from_number(2.0)];
        let slot = vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandomNormal, &args).unwrap();
        assert_eq!(slot.kind, NativeKind::Float64);
        assert!(slot.as_f64().unwrap().is_finite());
    }

    #[test]
    fn random_array_returns_v2_f64_typed_array_carrier() {
        let seed_args = [KindedSlot::from_number(11.0)];
        let _ = vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandomSeed, &seed_args).unwrap();
        let args = [KindedSlot::from_int(3)];
        let slot = vm_random_intrinsic_slot(BuiltinFunction::IntrinsicRandomArray, &args).unwrap();
        assert_eq!(slot.kind, NativeKind::Ptr(HeapKind::TypedArray));

        let ptr = slot.slot().raw() as *const shape_value::v2::typed_array::TypedArray<f64>;
        let values = unsafe { shape_value::v2::typed_array::TypedArray::<f64>::as_slice(ptr) };
        assert_eq!(values.len(), 3);
        assert!(values.iter().all(|v| *v >= 0.0 && *v < 1.0));
    }
}

#[cfg(test)]
mod fstring_decode_tests {
    use super::*;

    #[test]
    fn decode_color_none() {
        assert_eq!(decode_fstring_color(FSTRING_COLOR_NONE, 0).unwrap(), None);
    }

    #[test]
    fn decode_color_named_red() {
        use shape_value::content::{Color, NamedColor};
        assert_eq!(
            decode_fstring_color(FSTRING_COLOR_NAMED, 0).unwrap(),
            Some(Color::Named(NamedColor::Red))
        );
    }

    #[test]
    fn decode_color_rgb_roundtrip() {
        use shape_value::content::Color;
        // (10 << 16) | (20 << 8) | 30
        let payload = (10 << 16) | (20 << 8) | 30;
        assert_eq!(
            decode_fstring_color(FSTRING_COLOR_RGB, payload).unwrap(),
            Some(Color::Rgb(10, 20, 30))
        );
    }

    #[test]
    fn decode_style_bold_red() {
        use shape_value::content::{Color, NamedColor};
        let style = decode_fstring_style(
            FSTRING_COLOR_NAMED,
            0, // Red
            FSTRING_COLOR_NONE,
            0,
            FSTRING_FLAG_BOLD,
        )
        .unwrap();
        assert!(style.bold);
        assert!(!style.italic);
        assert_eq!(style.fg, Some(Color::Named(NamedColor::Red)));
        assert_eq!(style.bg, None);
    }

    #[test]
    fn decode_style_all_flags() {
        let style = decode_fstring_style(
            FSTRING_COLOR_NONE,
            0,
            FSTRING_COLOR_NONE,
            0,
            FSTRING_FLAG_BOLD | FSTRING_FLAG_ITALIC | FSTRING_FLAG_UNDERLINE | FSTRING_FLAG_DIM,
        )
        .unwrap();
        assert!(style.bold);
        assert!(style.italic);
        assert!(style.underline);
        assert!(style.dim);
    }

    #[test]
    fn decode_color_invalid_kind() {
        assert!(decode_fstring_color(99, 0).is_err());
    }

    #[test]
    fn decode_color_invalid_named_id() {
        assert!(decode_fstring_color(FSTRING_COLOR_NAMED, 99).is_err());
    }
}
