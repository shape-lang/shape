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
use shape_value::{KindedSlot, VMError, ValueSlot};

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
        let count = crate::executor::builtins::kind_coerce::int_operand(&count_slot)
            .map_err(|_| {
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
                        shape_value::NativeKind::Ptr(
                            shape_value::HeapKind::String,
                        ),
                    );
                    self.push_kinded_slot(null_slot)?;
                }
                BuiltinFunction::Format
                | BuiltinFunction::FormatValueWithMeta => {
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
                BuiltinFunction::IsNumber
                | BuiltinFunction::IsString
                | BuiltinFunction::IsBool
                | BuiltinFunction::IsArray
                | BuiltinFunction::IsObject
                | BuiltinFunction::IsDataRow => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5c — is_* type-check body migration \
                         pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::ToString
                | BuiltinFunction::ToNumber
                | BuiltinFunction::ToBool => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5c — conversion body migration \
                         pending (dispatch_conversion_builtin): {:?}",
                        builtin
                    );
                }
                BuiltinFunction::NativePtrSize
                | BuiltinFunction::NativePtrNewCell
                | BuiltinFunction::NativePtrFreeCell
                | BuiltinFunction::NativePtrReadPtr
                | BuiltinFunction::NativePtrWritePtr
                | BuiltinFunction::NativeTableFromArrowC
                | BuiltinFunction::NativeTableFromArrowCTyped
                | BuiltinFunction::NativeTableBindType => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5c — native-interop body migration \
                         pending (dispatch_native_interop_builtin): {:?}",
                        builtin
                    );
                }
                BuiltinFunction::TypeOf => {
                    todo!(
                        "phase-1b-vm wave 5c — TypeOf body migration pending \
                         (legacy body popped via the deleted raw-bits stack \
                         shim; needs kinded-carrier rebuild — see ADR-006 \
                         §2.7.6)"
                    );
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
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5d — closure-driven array builtin \
                         body migration pending: {:?}",
                        builtin
                    );
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
                    todo!(
                        "phase-1b-vm wave 5d — vector intrinsic body \
                         migration pending (handle_vector_intrinsic): {:?}",
                        builtin
                    );
                }
                BuiltinFunction::IntrinsicMatMulVec
                | BuiltinFunction::IntrinsicMatMulMat
                | BuiltinFunction::IntrinsicMatAdd
                | BuiltinFunction::IntrinsicMatSub => {
                    todo!(
                        "phase-1b-vm wave 5d — matrix intrinsic body \
                         migration pending (handle_matrix_intrinsic): {:?}",
                        builtin
                    );
                }
                BuiltinFunction::IntrinsicMinimize => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5d — minimize intrinsic body \
                         migration pending"
                    );
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
                BuiltinFunction::IntrinsicBspline2_3dBatch
                | BuiltinFunction::IntrinsicMean
                | BuiltinFunction::IntrinsicMin
                | BuiltinFunction::IntrinsicMax
                | BuiltinFunction::IntrinsicStd
                | BuiltinFunction::IntrinsicVariance
                | BuiltinFunction::IntrinsicRandom
                | BuiltinFunction::IntrinsicRandomInt
                | BuiltinFunction::IntrinsicRandomSeed
                | BuiltinFunction::IntrinsicRandomNormal
                | BuiltinFunction::IntrinsicRandomArray
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
                | BuiltinFunction::IntrinsicAtan2
                | BuiltinFunction::IntrinsicSinh
                | BuiltinFunction::IntrinsicCosh
                | BuiltinFunction::IntrinsicTanh
                | BuiltinFunction::IntrinsicCharCode
                | BuiltinFunction::IntrinsicFromCharCode
                | BuiltinFunction::IntrinsicSeries => {
                    todo!(
                        "phase-1b-vm wave 5d — intrinsic body migration \
                         pending (handle_intrinsic_builtin): {:?}",
                        builtin
                    );
                }

                // ── Wave 5e: constructors (Result/Option, Set, Deque,
                // PriorityQueue, HashMap, Mutex/Atomic/Lazy/Channel),
                // Content builders, DateTime constructors, Table from
                // rows, JSON navigation helpers, Window functions, Join,
                // Reflect, MatFromFlat, MakeContent*. ─────────────────────
                BuiltinFunction::SomeCtor => {
                    // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
                    // 2026-05-10): `Some(x)` builds a fresh
                    // `Arc<OptionData>` carrier with `is_some=true` and
                    // the popped argument as the typed payload share.
                    // The `pop_builtin_args` carrier owns one strong-
                    // count share per heap-bearing kind; ownership
                    // transfers into the `OptionData::payload` slot
                    // verbatim (the carrier is moved, not cloned).
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Some() expects 1 argument, got {}",
                            args.len()
                        )));
                    }
                    let payload = args.remove(0);
                    let opt = std::sync::Arc::new(
                        shape_value::heap_value::OptionData::some(payload),
                    );
                    self.push_kinded_slot(KindedSlot::from_option(opt))?;
                }
                BuiltinFunction::OkCtor => {
                    // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
                    // 2026-05-10): `Ok(x)` builds a fresh
                    // `Arc<ResultData>` carrier with `is_ok=true` and
                    // the popped argument as the typed payload share.
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Ok() expects 1 argument, got {}",
                            args.len()
                        )));
                    }
                    let payload = args.remove(0);
                    let res = std::sync::Arc::new(
                        shape_value::heap_value::ResultData::ok(payload),
                    );
                    self.push_kinded_slot(KindedSlot::from_result(res))?;
                }
                BuiltinFunction::ErrCtor => {
                    // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
                    // 2026-05-10): `Err(e)` builds a fresh
                    // `Arc<ResultData>` carrier with `is_ok=false` and
                    // the popped argument as the typed payload share.
                    let mut args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    if args.len() != 1 {
                        return Err(VMError::RuntimeError(format!(
                            "Err() expects 1 argument, got {}",
                            args.len()
                        )));
                    }
                    let payload = args.remove(0);
                    let res = std::sync::Arc::new(
                        shape_value::heap_value::ResultData::err(payload),
                    );
                    self.push_kinded_slot(KindedSlot::from_result(res))?;
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
                        std::sync::Arc::new(
                            shape_value::heap_value::HashMapData::<
                                *const shape_value::v2::string_obj::StringObj,
                            >::new(),
                        ),
                    );
                    let hm = std::sync::Arc::new(empty_kref);
                    self.push_kinded_slot(KindedSlot::from_hashmap(hm))?;
                }
                BuiltinFunction::SetCtor => {
                    // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
                    // 2026-05-10): empty Set ctor — `Set()` takes no
                    // args at landing; `Set([elements])` initialization
                    // is a follow-up. Build empty Arc<HashSetData> and
                    // push via KindedSlot::from_hashset.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let empty = std::sync::Arc::new(
                        shape_value::heap_value::HashSetData::new(),
                    );
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
                    let empty = std::sync::Arc::new(
                        shape_value::heap_value::DequeData::new(),
                    );
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
                    let empty = std::sync::Arc::new(
                        shape_value::heap_value::PriorityQueueData::new(),
                    );
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
                    let empty = std::sync::Arc::new(
                        shape_value::heap_value::ChannelData::new(),
                    );
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
                    let m = std::sync::Arc::new(
                        shape_value::heap_value::MutexData::new(initial),
                    );
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
                    let a = std::sync::Arc::new(
                        shape_value::heap_value::AtomicData::new(initial),
                    );
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
                        shape_value::NativeKind::Ptr(
                            shape_value::heap_value::HeapKind::Closure
                        )
                    ) {
                        return Err(VMError::RuntimeError(format!(
                            "Lazy() argument must be a closure (got \
                             kind {:?})",
                            initializer.kind
                        )));
                    }
                    let l = std::sync::Arc::new(
                        shape_value::heap_value::LazyData::new(initializer),
                    );
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
                    // Chart constructor is v0.4 scope per supervisor D4
                    // (R8 W3, 2026-05-24) — ECharts integration is its own
                    // workstream. W18.5 ships Table / Code / KeyValue only;
                    // surfacing the Chart MVP keeps the dispatch arm honest
                    // (no Bool-default kinded shim per playbook §4 #9).
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(
                        "Content.chart(...) is v0.4 scope per supervisor D4 \
                         (R8 W3, 2026-05-24) — ECharts integration is its \
                         own workstream. W18.5 ships Table / Code / KeyValue \
                         builders only."
                            .to_string(),
                    ));
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
                    let result =
                        KindedSlot::from_content(std::sync::Arc::new(node));
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
                            "FStringContentStyledText fg_kind must be int"
                                .to_string(),
                        )
                    })?;
                    let fg_payload = args[2].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText fg_payload must be int"
                                .to_string(),
                        )
                    })?;
                    let bg_kind = args[3].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText bg_kind must be int"
                                .to_string(),
                        )
                    })?;
                    let bg_payload = args[4].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText bg_payload must be int"
                                .to_string(),
                        )
                    })?;
                    let flags = args[5].as_i64().ok_or_else(|| {
                        VMError::RuntimeError(
                            "FStringContentStyledText flags must be int"
                                .to_string(),
                        )
                    })?;

                    let style = decode_fstring_style(
                        fg_kind, fg_payload, bg_kind, bg_payload, flags,
                    )?;
                    let node = shape_value::content::ContentNode::styled(
                        value, style,
                    );
                    let result =
                        KindedSlot::from_content(std::sync::Arc::new(node));
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
                        if arg.kind
                            != shape_value::NativeKind::Ptr(
                                shape_value::HeapKind::Content,
                            )
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
                        let node: &shape_value::content::ContentNode = unsafe {
                            &*(bits as *const shape_value::content::ContentNode)
                        };
                        nodes.push(node.clone());
                    }
                    let node = shape_value::content::ContentNode::Fragment(
                        nodes,
                    );
                    let result =
                        KindedSlot::from_content(std::sync::Arc::new(node));
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
                    let r = super::super::builtins::datetime_builtins::builtin_datetime_now(
                        &args,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeUtc => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins::builtin_datetime_utc(
                        &args,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeParse => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins::builtin_datetime_parse(
                        &args,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeFromEpoch => {
                    let args = self.pop_builtin_args()?;
                    let r =
                        super::super::builtins::datetime_builtins::builtin_datetime_from_epoch(
                            &args,
                        )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeFromParts => {
                    let args = self.pop_builtin_args()?;
                    let r =
                        super::super::builtins::datetime_builtins::builtin_datetime_from_parts(
                            &args,
                        )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::DateTimeFromUnixSecs => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins
                        ::builtin_datetime_from_unix_secs(&args)?;
                    self.push_kinded_slot(r)?;
                }
                // ── Wave 5e: mat() row-major matrix constructor ───────────
                BuiltinFunction::MatFromFlat => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::datetime_builtins::builtin_mat_from_flat(
                        &args,
                    )?;
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
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — JSON navigation helper body \
                         migration pending: {:?}",
                        builtin
                    );
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
                    let r = super::super::window_join::handle_window_row_number_v2(
                        self, &args, _ctx,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowLag | BuiltinFunction::WindowLead => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_lag_v2(
                        self, &args, _ctx,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowFirstValue
                | BuiltinFunction::WindowLastValue
                | BuiltinFunction::WindowNthValue => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_first_value_v2(
                        self, &args, _ctx,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowSum => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_sum_v2(
                        self, &args, _ctx,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowAvg => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_avg_v2(
                        self, &args, _ctx,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowMin => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_min_v2(
                        self, &args, _ctx,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowMax => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_max_v2(
                        self, &args, _ctx,
                    )?;
                    self.push_kinded_slot(r)?;
                }
                BuiltinFunction::WindowCount => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::window_join::handle_window_count_v2(
                        self, &args, _ctx,
                    )?;
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
                    todo!(
                        "phase-1b-vm wave 5e — reflect builtin body \
                         migration pending"
                    );
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
                        "DataReference / DataRow type has been removed"
                            .to_string(),
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
        let _ = ctx;

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
        // `ctx` was consumed by the dispatch loop above; re-acquire via
        // the shared output adapter on the VM-level executor context if
        // present. For W18.6 we route to stdout unconditionally when no
        // adapter was supplied — same fallback as the pre-W18.6 path.
        println!("{}", result.rendered);
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
    fn try_dispatch_display(
        &mut self,
        arg: &KindedSlot,
    ) -> Result<Option<KindedSlot>, VMError> {
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
    pub(crate) fn builtin_format(
        &mut self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        let formatter =
            super::super::printing::ValueFormatter::new(&self.program.type_schema_registry);
        let mut out = String::new();
        for a in args {
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
                    shape_value::NativeKind::Float64
                    | shape_value::NativeKind::NullableFloat64 => Some(v.slot.as_f64()),
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
                    _ => self.builtin_format(&args[..1])?.as_str().unwrap_or("").to_string(),
                };
                Ok(KindedSlot::from_string_arc(std::sync::Arc::new(rendered)))
            }
            Some(tag) if tag == FORMAT_SPEC_TABLE => {
                Err(VMError::NotImplemented(
                    "FormatValueWithSpec: FORMAT_SPEC_TABLE rendering deferred — \
                     W13-print-formatter scope is the FORMAT_SPEC_FIXED + \
                     no-spec path. Table rendering reuses the DataTable / \
                     TableView Display impls; surface-and-stop pending the \
                     next pass per W13 playbook §7.4."
                        .to_string(),
                ))
            }
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
        as_v2_typed_array, read_element, V2ElemType,
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
    use crate::executor::v2_handlers::v2_array_detect::{
        as_v2_typed_array, read_element,
    };
    use shape_value::{HeapKind, NativeKind};
    if args[1].kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Content.table: rows argument must be Array<Array<string>>, \
             got kind {:?}",
            args[1].kind
        )));
    }
    let outer_view = as_v2_typed_array(args[1].slot.raw(), args[1].kind)
        .ok_or_else(|| {
            VMError::RuntimeError(
                "Content.table: rows array has invalid v2 header".to_string(),
            )
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
    use crate::executor::v2_handlers::v2_array_detect::{
        as_v2_typed_array, read_element,
    };
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
    use crate::executor::v2_handlers::v2_array_detect::{
        as_v2_typed_array, read_element,
    };
    use shape_value::{HeapKind, NativeKind};
    if args[0].kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Content.fragment: parts argument must be Array<content>, got \
             kind {:?}",
            args[0].kind
        )));
    }
    let view = as_v2_typed_array(args[0].slot.raw(), args[0].kind).ok_or_else(|| {
        VMError::RuntimeError(
            "Content.fragment: parts array has invalid v2 header".to_string(),
        )
    })?;
    let mut parts = Vec::with_capacity(view.len as usize);
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Content.fragment: failed to read element {}",
                i
            ))
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

fn decode_fstring_color(kind: i64, payload: i64) -> Result<Option<shape_value::content::Color>, VMError> {
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
            FSTRING_FLAG_BOLD
                | FSTRING_FLAG_ITALIC
                | FSTRING_FLAG_UNDERLINE
                | FSTRING_FLAG_DIM,
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
