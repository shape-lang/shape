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
//! # `pop_builtin_args` runtime semantics (Wave 5b → architectural surface)
//!
//! Wave 5b implements `pop_builtin_args` by popping the call's arg-count
//! slot (always Int64-kinded, pushed as `PushConst(Number(n))` by the
//! compiler), then popping each arg's raw u64 bits off the stack. The
//! per-arg `NativeKind` is **not yet recoverable from the stack ABI** —
//! the typed VM stack stores raw u64 with kinds threaded only through the
//! producing opcode operand and (for locals) the `FrameDescriptor`. There
//! is no per-stack-slot kind shadow. Wave 6 owns that work.
//!
//! As a transitional shim, every popped arg is currently tagged with
//! `NativeKind::Bool` (the §2.7 default sentinel — Drop is a no-op for
//! Bool-kinded slots regardless of the underlying bits, so this is leak-
//! free at the cost of being kind-blind). Builtin bodies will fail their
//! kind discriminator at runtime when the bits are not actually Bool —
//! making the architectural gap immediately visible. No silent corruption.
//!
//! When Wave 6 lands a kind-bearing stack ABI, this is the call site that
//! gets the real `NativeKind` per arg from the stack metadata. See the
//! audit's §C1/§D entries (`type_tracking.rs:286`, `vm_impl/stack.rs`).

use super::super::*;
use shape_value::{KindedSlot, NativeKind};

impl VirtualMachine {
    /// Pop the builtin call's args off the typed VM stack into a
    /// `Vec<KindedSlot>`. The topmost stack slot is the arg count
    /// (Int64-kinded, pushed by the compiler as a numeric `PushConst`).
    ///
    /// **Wave 5b semantic gap (surfaced as architectural concern)**: the
    /// VM stack does not currently carry per-slot `NativeKind`. The
    /// compiler emits typed pushes (e.g. `PushNativeInt64`,
    /// `PushNativeF64`) but the kind is consumed by the producing
    /// opcode and not retained on the stack itself. Until Wave 6 lands a
    /// kind-bearing stack ABI, this method tags every popped arg with
    /// `NativeKind::Bool` (§2.7 default sentinel — Drop / Clone is a
    /// no-op for the Bool arm regardless of underlying bits).
    ///
    /// Builtin bodies will surface the gap by failing their kind
    /// discriminator at runtime; the failure mode is a clean
    /// `VMError::RuntimeError` (`"<name>() argument must be a number"`,
    /// etc.) — never silent heap corruption.
    pub(crate) fn pop_builtin_args(&mut self) -> Result<Vec<KindedSlot>, VMError> {
        // Top of stack: the arg count, pushed as a numeric constant by the
        // compiler (`PushConst(Number(arg_count as f64))`). Some sites omit
        // the count (zero-arg builtins like `Snapshot`) — we cannot
        // distinguish here, so we pop the count when the topmost slot
        // looks numeric; otherwise treat as zero. Wave 6 will bake the
        // count into the BuiltinCall operand.
        //
        // Today the count is always present: the compiler in
        // `compiler/expressions/function_calls.rs:1151-1158` only emits the
        // PushConst(arg_count) when `builtin_requires_arg_count(builtin)`
        // returns true, but for the math/array/utility scope of Wave 5b it
        // does. We rely on that contract.
        let count_bits = self.pop_raw_u64()?;
        // The count was pushed via `PushConst(Number(n as f64))` —
        // reinterpret the raw bits as f64 and cast.
        let count = f64::from_bits(count_bits) as usize;

        let mut args: Vec<KindedSlot> = Vec::with_capacity(count);
        for _ in 0..count {
            let bits = self.pop_raw_u64()?;
            // Wave 6 architectural surface: stack slots do not carry
            // NativeKind metadata. Tag with the §2.7 default (Bool, no
            // Drop side-effects) and let the body surface the kind
            // mismatch via its own type-error path.
            args.push(KindedSlot::new(
                shape_value::ValueSlot::from_raw(bits),
                NativeKind::Bool,
            ));
        }
        args.reverse();
        Ok(args)
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
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Sqrt => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_sqrt(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Ln => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_ln(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Pow => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_pow(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Exp => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_exp(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Log => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_log(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Floor => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_floor(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Ceil => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_ceil(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Round => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_round(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Sin => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_sin(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Cos => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_cos(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Tan => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_tan(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Asin => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_asin(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Acos => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_acos(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Atan => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_atan(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Min => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_min(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Max => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_max(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::StdDev => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_stddev(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Sign => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_sign(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Gcd => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_gcd(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Lcm => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_lcm(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Hypot => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_hypot(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Clamp => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_clamp(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::IsNaN => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_is_nan(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::IsFinite => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::math::builtin_is_finite(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }

                // ── Wave 5b: array builtins ───────────────────────────────
                BuiltinFunction::Push => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_push(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Pop => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_pop(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::First => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_first(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Last => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_last(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Zip => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_zip(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Filled => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_filled(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Range => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_range(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }
                BuiltinFunction::Slice => {
                    let args = self.pop_builtin_args()?;
                    let r = super::super::builtins::array_ops::builtin_slice(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
                }

                // ── Wave 5b: utility builtins ─────────────────────────────
                BuiltinFunction::ObjectRest => {
                    let args = self.pop_builtin_args()?;
                    let r = self.builtin_object_rest(&args)?;
                    self.push_raw_u64(r.slot().raw())?;
                    std::mem::forget(r);
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
                BuiltinFunction::Print
                | BuiltinFunction::Format
                | BuiltinFunction::FormatValueWithMeta
                | BuiltinFunction::FormatValueWithSpec => {
                    // Print/Format machinery touches the formatter
                    // (`executor/printing.rs`), Content rendering, and the
                    // OutputAdapter — all explicitly Wave 5e scope per the
                    // dispatch comment. Wave 5b deliberately leaves these
                    // bodies unmigrated and surfaces a clean runtime error
                    // when invoked, rather than panicking.
                    let _args = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "{:?} body migration deferred to Wave 5e (formatter \
                         lives in executor/printing.rs)",
                        builtin
                    )));
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
                         (uses self.pop_raw_u64 internally; needs kind \
                         carrier rebuild)"
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
                BuiltinFunction::IntrinsicBspline2_3dBatch
                | BuiltinFunction::IntrinsicSum
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
                BuiltinFunction::SomeCtor
                | BuiltinFunction::OkCtor
                | BuiltinFunction::ErrCtor => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — Option/Result ctor body \
                         migration pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::HashMapCtor
                | BuiltinFunction::SetCtor
                | BuiltinFunction::DequeCtor
                | BuiltinFunction::PriorityQueueCtor => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — collection ctor body \
                         migration pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::MutexCtor
                | BuiltinFunction::AtomicCtor
                | BuiltinFunction::LazyCtor
                | BuiltinFunction::ChannelCtor => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — concurrency ctor body \
                         migration pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::MakeContentText
                | BuiltinFunction::MakeContentFragment
                | BuiltinFunction::ApplyContentStyle
                | BuiltinFunction::MakeContentChartFromValue => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — content builder body \
                         migration pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::ContentChart
                | BuiltinFunction::ContentTextCtor
                | BuiltinFunction::ContentTableCtor
                | BuiltinFunction::ContentCodeCtor
                | BuiltinFunction::ContentKvCtor
                | BuiltinFunction::ContentFragmentCtor => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — content namespace ctor body \
                         migration pending (shape_runtime::content_builders): \
                         {:?}",
                        builtin
                    );
                }
                BuiltinFunction::DateTimeNow
                | BuiltinFunction::DateTimeUtc
                | BuiltinFunction::DateTimeParse
                | BuiltinFunction::DateTimeFromEpoch
                | BuiltinFunction::DateTimeFromParts
                | BuiltinFunction::DateTimeFromUnixSecs => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — DateTime ctor body migration \
                         pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::MatFromFlat => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — mat() ctor body migration \
                         pending"
                    );
                }
                BuiltinFunction::MakeTableFromRows => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5e — make_table_from_rows body \
                         migration pending"
                    );
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
                BuiltinFunction::WindowRowNumber
                | BuiltinFunction::WindowRank
                | BuiltinFunction::WindowDenseRank
                | BuiltinFunction::WindowNtile
                | BuiltinFunction::WindowLag
                | BuiltinFunction::WindowLead
                | BuiltinFunction::WindowFirstValue
                | BuiltinFunction::WindowLastValue
                | BuiltinFunction::WindowNthValue
                | BuiltinFunction::WindowSum
                | BuiltinFunction::WindowAvg
                | BuiltinFunction::WindowMin
                | BuiltinFunction::WindowMax
                | BuiltinFunction::WindowCount => {
                    todo!(
                        "phase-1b-vm wave 5e — window function body \
                         migration pending (handle_window_functions): {:?}",
                        builtin
                    );
                }
                BuiltinFunction::JoinExecute => {
                    todo!(
                        "phase-1b-vm wave 5e — JOIN body migration pending \
                         (handle_join_execute)"
                    );
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
                    todo!(
                        "phase-1b-vm wave 5e — handle_eval_datetime_expr \
                         body migration pending"
                    );
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

    // Runtime bridge functions (pop_builtin_args impl, eval_runtime_*)
    // moved to builtins/runtime_bridge.rs.
    // map_runtime_error and type_of_name moved to module_registry module.

    // ===== Helper Methods =====
    // binary_arithmetic, eval_runtime_binary_op_value, binary_comparison
    // moved to arithmetic/mod.rs
}
