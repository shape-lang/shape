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
        // Top of stack: the arg count, pushed as a numeric constant by the
        // compiler (`PushConst(Number(arg_count as f64))`). The count slot
        // is inline-scalar (Float64-kinded), so dropping its share is a
        // no-op — but we still go through `pop_kinded` for invariant
        // discipline.
        let (count_bits, _count_kind) = self.pop_kinded()?;
        let count = f64::from_bits(count_bits) as usize;

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
                    // routes the basic FORMAT_SPEC_FIXED path; richer
                    // spec arms (Table, ContentStyle) surface as
                    // `NotImplemented` per W13 playbook §7.4 surface-and-stop.
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
                    // Phase-2c §2.7.4 SURFACE: Option/Result ctors produce
                    // values whose deconstructors (`op_try_unwrap`,
                    // `op_unwrap_option`, `op_is_ok`, `op_is_err`,
                    // `op_unwrap_ok`, `op_unwrap_err` in
                    // `executor/exceptions/mod.rs`) all return
                    // `NotImplemented(PHASE_2C_EXCEPTION_OBJECT_SURFACE)`
                    // pending the variant-codegen rebuild on the kinded
                    // `Arc<TypedObjectStorage>` model. Landing the
                    // constructor side without the inspection side would
                    // produce values nothing can read — the surface stays
                    // closed at both ends until the variant-codegen
                    // cluster lands. Pop args (their `KindedSlot::Drop`
                    // retires per-arg shares per §2.7.7) and surface.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "{:?} — SURFACE: Option/Result variant ctor depends \
                         on Phase-2c variant-codegen rebuild (kinded \
                         Arc<TypedObjectStorage> model). Paired with the \
                         deconstructor SURFACE in \
                         executor/exceptions/mod.rs (PHASE_2C_\
                         EXCEPTION_OBJECT_SURFACE: op_try_unwrap, \
                         op_unwrap_option, op_is_ok, op_is_err, \
                         op_unwrap_ok, op_unwrap_err). ADR-006 §2.7.4 / \
                         playbook §10 E-exceptions row.",
                        builtin
                    )));
                }
                BuiltinFunction::HashMapCtor => {
                    // Wave 5e W13-hashmap-ctor (2026-05-10): `let m = HashMap()`
                    // produces a fresh empty `Arc<HashMapData>` slot.
                    // Reader contract: kind == Ptr(HeapKind::HashMap),
                    // bits = Arc::into_raw::<HashMapData>.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    let hm = std::sync::Arc::new(
                        shape_value::heap_value::HashMapData::new(),
                    );
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
                BuiltinFunction::DequeCtor
                | BuiltinFunction::PriorityQueueCtor => {
                    // Phase-2c §2.7.4 SURFACE: Stage C HeapKind family
                    // rebuild required. The pre-bulldozer HeapKind
                    // ordinals for `Set`, `Deque`, `PriorityQueue`,
                    // `Channel`, `Column`, `Matrix`, `Range` were deleted
                    // when their `HeapValue` arms were trimmed in the
                    // strict-typing Phase-2 pass; only `HashMap` was
                    // rebuilt (Stage C P1(b), 2026-05-07). Each of these
                    // ctors is its own Stage C cluster (per playbook
                    // "Out of scope" list) — surface here, do not
                    // fabricate a HeapKind.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "{:?} — SURFACE: Stage C HeapKind family rebuild \
                         required. Set/Deque/PriorityQueue have no \
                         HeapKind variant after the strict-typing Phase-2 \
                         deletion (only HashMap was rebuilt, Stage C \
                         P1(b)). Each is its own Stage C cluster per \
                         the W13 playbook 'Out of scope' list. ADR-006 \
                         §2.7.4 / §2.7.15.",
                        builtin
                    )));
                }
                BuiltinFunction::MutexCtor
                | BuiltinFunction::AtomicCtor
                | BuiltinFunction::LazyCtor
                | BuiltinFunction::ChannelCtor => {
                    // Phase-2c §2.7.4 SURFACE: Stage C HeapKind family
                    // rebuild required. Concurrency primitives (Mutex,
                    // Atomic, Lazy, Channel) lost their HeapKind /
                    // HeapValue arms in the strict-typing Phase-2 pass;
                    // no kinded carrier exists to represent them. Each
                    // is a separate Stage C cluster.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return Err(VMError::NotImplemented(format!(
                        "{:?} — SURFACE: Stage C HeapKind family rebuild \
                         required. Mutex/Atomic/Lazy/Channel have no \
                         HeapKind variant after the strict-typing Phase-2 \
                         deletion. Each is its own Stage C cluster per \
                         the W13 playbook 'Out of scope' list. ADR-006 \
                         §2.7.4.",
                        builtin
                    )));
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
                    // SURFACE — Phase-2c §2.7.4 boundary (HeapKind::Temporal
                    // carrier dispatch). Drains stack args first to keep
                    // the parallel-kind track balanced.
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    return self.handle_eval_datetime_expr(_ctx);
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
        let rendered = {
            let formatter =
                super::super::printing::ValueFormatter::new(&self.program.type_schema_registry);
            args.iter()
                .map(|a| formatter.format_kinded(a))
                .collect::<Vec<_>>()
                .join(" ")
        };

        let result = shape_runtime::print_result::PrintResult {
            rendered,
            spans: Vec::new(),
        };
        if let Some(ctx_mut) = ctx {
            // Drop the returned KindedSlot — `print()` always yields the
            // GENERIC_CARRIER none-slot per §2.7.4. The dispatch shell
            // re-pushes the null sentinel itself.
            let _ = ctx_mut.output_adapter_mut().print(result);
        } else {
            // No execution context — write directly to stdout. Mirrors
            // the `StdoutAdapter` default so REPL/test harnesses without
            // explicit ctx setup still see output.
            println!("{}", result.rendered);
        }
        Ok(())
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
    /// the Table and ContentStyle arms surface per W13 playbook §7.4
    /// surface-and-stop.
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
