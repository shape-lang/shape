//! Builtin dispatch slice (ADR-006 §2.7.6 / Q8).
//!
//! Wave 5a (phase-1b-vm) flipped the dispatch SHAPE here: every arm now
//! produces / consumes `Vec<KindedSlot>` (and `&[KindedSlot]`), aligned
//! with the carrier-API bound spec'd at §2.7.6. The body interiors
//! (math kernels, array kernels, content builders, type-introspection,
//! stats, intrinsics, JSON helpers, table builders, content / DateTime /
//! concurrency constructors) are deferred to Waves 5b-5e:
//!
//! - **Wave 5b**: math + array + utility bodies (`builtin_abs`, `builtin_push`,
//!   `builtin_format`, etc.) + `pop_builtin_args` runtime implementation.
//! - **Wave 5c**: type-introspection + conversion + native-interop bodies
//!   (`builtin_is_*`, `builtin_to_*`, `dispatch_native_interop_builtin`).
//! - **Wave 5d**: closure-driven array builtins (`map`, `filter`, `reduce`,
//!   etc.) + intrinsic dispatch (`handle_intrinsic_builtin`,
//!   `handle_vector_intrinsic`, `handle_matrix_intrinsic`).
//! - **Wave 5e**: content + DateTime + concurrency constructors + window /
//!   join / reflect / state-builtin bodies + `executor/printing.rs` formatter.
//!
//! Until those bodies land, every arm here is `todo!(...)` with a sub-cluster
//! tag. The dispatch SHAPE compiles in isolation; runtime invocation panics
//! cleanly with the sub-cluster name.
//!
//! The companion §2.7.6 / Q8 carrier-API bound: NO per-heap-variant
//! accessors on `KindedSlot`; bodies that inspect heap payloads use
//! `slot.as_heap_value()` + `HeapValue` match. NO cross-kind accessors
//! (`as_number_coerce`, etc.) on the carrier; coercion lives at
//! `executor/builtins/kind_coerce.rs` (free helper at the body site).

use super::super::*;
use shape_value::KindedSlot;

impl VirtualMachine {
    /// Pop the builtin call's args off the typed VM stack into a
    /// `Vec<KindedSlot>`. The arg count is the topmost stack slot;
    /// per-arg kinds are projected from the frame descriptor / ARG
    /// opcode stream in the Wave 5b body migration.
    ///
    /// Wave 5a foundation: signature flipped from
    /// `Result<Vec<ValueWord>, VMError>` to
    /// `Result<Vec<KindedSlot>, VMError>` (per §2.7.6, deletes the
    /// last `ValueWord`-shaped collection in the dispatch slice). The
    /// runtime implementation is stubbed for Wave 5b; the kind-threaded
    /// pop-and-pair logic lives there.
    pub(crate) fn pop_builtin_args(&mut self) -> Result<Vec<KindedSlot>, VMError> {
        todo!(
            "phase-1b-vm wave 5b — pop_builtin_args runtime: pop count from \
             stack, project each arg through (ValueSlot::from_raw(bits), \
             NativeKind from FrameDescriptor / ARG opcode kind)"
        )
    }

    // ========================================================================
    // Builtin Dispatch
    //
    // Wave 5a flipped the dispatch SHAPE: every arm produces /
    // consumes `Vec<KindedSlot>`. Body interiors stubbed with
    // `todo!("phase-1b-vm wave 5{b,c,d,e} — body migration pending")`.

    pub fn op_builtin_call(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        if let Some(Operand::Builtin(builtin)) = instruction.operand {
            let _ctx = ctx;
            match builtin {
                // ── Wave 5b: math + array + utility builtins ──────────────
                BuiltinFunction::Abs
                | BuiltinFunction::Sqrt
                | BuiltinFunction::Ln
                | BuiltinFunction::Pow
                | BuiltinFunction::Exp
                | BuiltinFunction::Log
                | BuiltinFunction::Floor
                | BuiltinFunction::Ceil
                | BuiltinFunction::Round
                | BuiltinFunction::Sin
                | BuiltinFunction::Cos
                | BuiltinFunction::Tan
                | BuiltinFunction::Asin
                | BuiltinFunction::Acos
                | BuiltinFunction::Atan
                | BuiltinFunction::Min
                | BuiltinFunction::Max
                | BuiltinFunction::StdDev
                | BuiltinFunction::Sign
                | BuiltinFunction::Gcd
                | BuiltinFunction::Lcm
                | BuiltinFunction::Hypot
                | BuiltinFunction::Clamp
                | BuiltinFunction::IsNaN
                | BuiltinFunction::IsFinite => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5b — math builtin body migration \
                         pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::Push
                | BuiltinFunction::Pop
                | BuiltinFunction::First
                | BuiltinFunction::Last
                | BuiltinFunction::Zip
                | BuiltinFunction::Filled
                | BuiltinFunction::Range
                | BuiltinFunction::Slice => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5b — array builtin body migration \
                         pending: {:?}",
                        builtin
                    );
                }
                BuiltinFunction::Format
                | BuiltinFunction::FormatValueWithMeta
                | BuiltinFunction::FormatValueWithSpec
                | BuiltinFunction::ObjectRest
                | BuiltinFunction::Print
                | BuiltinFunction::Snapshot
                | BuiltinFunction::Exit => {
                    let _args: Vec<KindedSlot> = self.pop_builtin_args()?;
                    todo!(
                        "phase-1b-vm wave 5b — utility builtin body migration \
                         pending: {:?}",
                        builtin
                    );
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
                    // DateTimeNow/Utc don't consume args, but pop is a
                    // no-op when count is zero — the runtime impl in 5b
                    // handles both shapes uniformly.
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
        // unreachable today: every reachable arm above either returns or
        // panics with `todo!`. Once Wave 5b lands `pop_builtin_args` and
        // the math/array/utility bodies, the structural-fallthrough arms
        // will produce a result and reach this line.
        #[allow(unreachable_code)]
        Ok(())
    }

    // Runtime bridge functions (pop_builtin_args impl, eval_runtime_*)
    // moved to builtins/runtime_bridge.rs.
    // map_runtime_error and type_of_name moved to module_registry module.

    // ===== Helper Methods =====
    // binary_arithmetic, eval_runtime_binary_op_value, binary_comparison
    // moved to arithmetic/mod.rs
}
