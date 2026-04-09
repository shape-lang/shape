//! Accessor methods and utility functions
//!
//! This module contains simple accessor methods and helper functions
//! for querying JIT compiler state.

use super::setup::JITCompiler;
use crate::context::JittedStrategyFn;
use shape_vm::bytecode::{BuiltinFunction, BytecodeProgram, Instruction, OpCode, Operand};

impl JITCompiler {
    /// Get the function table for setting up JITContext
    #[inline(always)]
    pub fn get_function_table(&self) -> &[*const u8] {
        &self.function_table
    }

    /// Get a compiled function pointer by function index
    #[inline(always)]
    pub fn get_function_by_index(&self, idx: usize) -> Option<JittedStrategyFn> {
        self.function_table.get(idx).and_then(|&ptr| {
            if ptr.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute(ptr) })
            }
        })
    }
}

/// Program-level preflight report for JIT capability checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JitPreflightReport {
    /// Opcodes that require VM execution for correct behavior.
    pub vm_only_opcodes: Vec<OpCode>,
    /// Builtins that are not lowered by the JIT translator.
    pub unsupported_builtins: Vec<BuiltinFunction>,
}

impl JitPreflightReport {
    /// True when the program can run safely in JIT without semantic downgrades.
    pub fn can_jit(&self) -> bool {
        self.vm_only_opcodes.is_empty() && self.unsupported_builtins.is_empty()
    }

    /// Human-readable blocker summary for diagnostics/logging.
    pub fn blockers_summary(&self) -> String {
        let mut parts = Vec::new();

        if !self.vm_only_opcodes.is_empty() {
            let opcodes = self
                .vm_only_opcodes
                .iter()
                .map(|op| format!("{op:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("opcodes=[{opcodes}]"));
        }

        if !self.unsupported_builtins.is_empty() {
            let builtins = self
                .unsupported_builtins
                .iter()
                .map(|builtin| format!("{builtin:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("builtins=[{builtins}]"));
        }

        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join("; ")
        }
    }
}

/// Program-level JIT parity entry (opcode/builtin support row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JitParityEntry {
    pub target: JitParityTarget,
    pub jit_supported: bool,
    pub reason: &'static str,
}

/// What the parity row describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JitParityTarget {
    Opcode(OpCode),
    Builtin(BuiltinFunction),
}

fn push_unique_opcode(out: &mut Vec<OpCode>, opcode: OpCode) {
    if !out.contains(&opcode) {
        out.push(opcode);
    }
}

fn push_unique_builtin(out: &mut Vec<BuiltinFunction>, builtin: BuiltinFunction) {
    if !out.contains(&builtin) {
        out.push(builtin);
    }
}

fn sort_opcodes(opcodes: &mut [OpCode]) {
    opcodes.sort_by_key(|op| format!("{op:?}"));
}

fn sort_builtins(builtins: &mut [BuiltinFunction]) {
    builtins.sort_by_key(|builtin| format!("{builtin:?}"));
}

const ALL_OPCODES: &[OpCode] = &[
    OpCode::PushConst,
    OpCode::PushNull,
    OpCode::Pop,
    OpCode::Dup,
    OpCode::Swap,
    OpCode::Add,
    OpCode::Sub,
    OpCode::Mul,
    OpCode::Div,
    OpCode::Mod,
    OpCode::Pow,
    OpCode::BitAnd,
    OpCode::BitOr,
    OpCode::BitShl,
    OpCode::BitShr,
    OpCode::BitNot,
    OpCode::BitXor,
    OpCode::Gt,
    OpCode::Lt,
    OpCode::Gte,
    OpCode::Lte,
    OpCode::EqDynamic,
    OpCode::NeqDynamic,
    OpCode::GtInt,
    OpCode::GtNumber,
    OpCode::GtDecimal,
    OpCode::LtInt,
    OpCode::LtNumber,
    OpCode::LtDecimal,
    OpCode::GteInt,
    OpCode::GteNumber,
    OpCode::GteDecimal,
    OpCode::LteInt,
    OpCode::EqInt,
    OpCode::EqNumber,
    OpCode::NeqInt,
    OpCode::NeqNumber,
    OpCode::EqString,
    OpCode::GtString,
    OpCode::LtString,
    OpCode::GteString,
    OpCode::LteString,
    OpCode::EqDecimal,
    OpCode::IsNull,
    OpCode::And,
    OpCode::Or,
    OpCode::Not,
    OpCode::AddInt,
    OpCode::AddNumber,
    OpCode::AddDecimal,
    OpCode::SubInt,
    OpCode::SubNumber,
    OpCode::SubDecimal,
    OpCode::MulInt,
    OpCode::MulNumber,
    OpCode::MulDecimal,
    OpCode::DivInt,
    OpCode::DivNumber,
    OpCode::DivDecimal,
    OpCode::ModInt,
    OpCode::NegInt,
    OpCode::NegNumber,
    OpCode::Jump,
    OpCode::JumpIfFalse,
    OpCode::JumpIfFalseTrusted,
    OpCode::JumpIfTrue,
    OpCode::Call,
    OpCode::Return,
    OpCode::ReturnValue,
    OpCode::CallValue,
    OpCode::LoadLocal,
    OpCode::LoadLocalTrusted,
    OpCode::StoreLocal,
    OpCode::LoadModuleBinding,
    OpCode::StoreModuleBinding,
    OpCode::LoadClosure,
    OpCode::StoreClosure,
    OpCode::MakeClosure,
    OpCode::CloseUpvalue,
    OpCode::MakeRef,
    OpCode::DerefLoad,
    OpCode::DerefStore,
    OpCode::SetIndexRef,
    OpCode::NewArray,
    OpCode::NewTypedArray,
    OpCode::NewObject,
    OpCode::GetProp,
    OpCode::SetProp,
    OpCode::Length,
    OpCode::ArrayPush,
    OpCode::ArrayPop,
    OpCode::MergeObject,
    OpCode::SetLocalIndex,
    OpCode::SetModuleBindingIndex,
    OpCode::ArrayPushLocal,
    OpCode::LoopStart,
    OpCode::LoopEnd,
    OpCode::Break,
    OpCode::Continue,
    OpCode::IterNext,
    OpCode::IterDone,
    OpCode::CallMethod,
    OpCode::PushTimeframe,
    OpCode::PopTimeframe,
    OpCode::BuiltinCall,
    OpCode::TypeCheck,
    OpCode::Convert,
    OpCode::ModNumber,
    OpCode::ModDecimal,
    OpCode::PowInt,
    OpCode::PowNumber,
    OpCode::PowDecimal,
    OpCode::LteNumber,
    OpCode::LteDecimal,
    OpCode::SetupTry,
    OpCode::PopHandler,
    OpCode::Throw,
    OpCode::TryUnwrap,
    OpCode::UnwrapOption,
    OpCode::ErrorContext,
    OpCode::IsOk,
    OpCode::IsErr,
    OpCode::UnwrapOk,
    OpCode::UnwrapErr,
    OpCode::SliceAccess,
    OpCode::NullCoalesce,
    OpCode::MakeRange,
    OpCode::GetDataField,
    OpCode::GetDataRow,
    OpCode::GetFieldTyped,
    OpCode::SetFieldTyped,
    OpCode::NewTypedObject,
    OpCode::TypedMergeObject,
    OpCode::WrapTypeAnnotation,
    OpCode::Yield,
    OpCode::Suspend,
    OpCode::Resume,
    OpCode::Poll,
    OpCode::AwaitBar,
    OpCode::AwaitTick,
    OpCode::Await,
    OpCode::SpawnTask,
    OpCode::EmitAlert,
    OpCode::EmitEvent,
    OpCode::JoinInit,
    OpCode::JoinAwait,
    OpCode::CancelTask,
    OpCode::AsyncScopeEnter,
    OpCode::AsyncScopeExit,
    OpCode::LoadColF64,
    OpCode::LoadColI64,
    OpCode::LoadColBool,
    OpCode::LoadColStr,
    OpCode::BindSchema,
    OpCode::BoxTraitObject,
    OpCode::DynMethodCall,
    OpCode::Nop,
    OpCode::Halt,
    OpCode::IntToNumber,
    OpCode::NumberToInt,
    OpCode::CallForeign,
    OpCode::AddTyped,
    OpCode::SubTyped,
    OpCode::MulTyped,
    OpCode::DivTyped,
    OpCode::ModTyped,
    OpCode::CmpTyped,
    OpCode::StoreLocalTyped,
    OpCode::StoreModuleBindingTyped,
    OpCode::CastWidth,
    // v2 typed array opcodes
    OpCode::NewTypedArrayF64,
    OpCode::NewTypedArrayI64,
    OpCode::NewTypedArrayI32,
    OpCode::NewTypedArrayBool,
    OpCode::TypedArrayGetF64,
    OpCode::TypedArrayGetI64,
    OpCode::TypedArrayGetI32,
    OpCode::TypedArrayGetBool,
    OpCode::TypedArraySetF64,
    OpCode::TypedArraySetI64,
    OpCode::TypedArraySetI32,
    OpCode::TypedArraySetBool,
    OpCode::TypedArrayPushF64,
    OpCode::TypedArrayPushI64,
    OpCode::TypedArrayPushI32,
    OpCode::TypedArrayPushBool,
    OpCode::TypedArrayLen,
];

const ALL_BUILTINS: &[BuiltinFunction] = &[
    // Math (18)
    BuiltinFunction::Abs,
    BuiltinFunction::Sqrt,
    BuiltinFunction::Ln,
    BuiltinFunction::Pow,
    BuiltinFunction::Exp,
    BuiltinFunction::Log,
    BuiltinFunction::Min,
    BuiltinFunction::Max,
    BuiltinFunction::Floor,
    BuiltinFunction::Ceil,
    BuiltinFunction::Round,
    BuiltinFunction::Sin,
    BuiltinFunction::Cos,
    BuiltinFunction::Tan,
    BuiltinFunction::Asin,
    BuiltinFunction::Acos,
    BuiltinFunction::Atan,
    BuiltinFunction::StdDev,
    // Array (8)
    BuiltinFunction::Range,
    BuiltinFunction::Slice,
    BuiltinFunction::Push,
    BuiltinFunction::Pop,
    BuiltinFunction::First,
    BuiltinFunction::Last,
    BuiltinFunction::Zip,
    BuiltinFunction::Filled,
    // HOF (8)
    BuiltinFunction::Map,
    BuiltinFunction::Filter,
    BuiltinFunction::Reduce,
    BuiltinFunction::ForEach,
    BuiltinFunction::Find,
    BuiltinFunction::FindIndex,
    BuiltinFunction::Some,
    BuiltinFunction::Every,
    // Utility (5)
    BuiltinFunction::Print,
    BuiltinFunction::Format,
    BuiltinFunction::Len,
    BuiltinFunction::Snapshot,
    BuiltinFunction::Exit,
    // Object (1)
    BuiltinFunction::ObjectRest,
    // Control (1)
    BuiltinFunction::ControlFold,
    // Type (7)
    BuiltinFunction::TypeOf,
    BuiltinFunction::IsNumber,
    BuiltinFunction::IsString,
    BuiltinFunction::IsBool,
    BuiltinFunction::IsArray,
    BuiltinFunction::IsObject,
    BuiltinFunction::IsDataRow,
    // Conversion (3)
    BuiltinFunction::ToString,
    BuiltinFunction::ToNumber,
    BuiltinFunction::ToBool,
    // Native ptr (8)
    BuiltinFunction::NativePtrSize,
    BuiltinFunction::NativePtrNewCell,
    BuiltinFunction::NativePtrFreeCell,
    BuiltinFunction::NativePtrReadPtr,
    BuiltinFunction::NativePtrWritePtr,
    BuiltinFunction::NativeTableFromArrowC,
    BuiltinFunction::NativeTableFromArrowCTyped,
    BuiltinFunction::NativeTableBindType,
    // Format (2)
    BuiltinFunction::FormatValueWithMeta,
    BuiltinFunction::FormatValueWithSpec,
    // Math intrinsics (6)
    BuiltinFunction::IntrinsicSum,
    BuiltinFunction::IntrinsicMean,
    BuiltinFunction::IntrinsicMin,
    BuiltinFunction::IntrinsicMax,
    BuiltinFunction::IntrinsicStd,
    BuiltinFunction::IntrinsicVariance,
    // Random (5)
    BuiltinFunction::IntrinsicRandom,
    BuiltinFunction::IntrinsicRandomInt,
    BuiltinFunction::IntrinsicRandomSeed,
    BuiltinFunction::IntrinsicRandomNormal,
    BuiltinFunction::IntrinsicRandomArray,
    // Distribution (5)
    BuiltinFunction::IntrinsicDistUniform,
    BuiltinFunction::IntrinsicDistLognormal,
    BuiltinFunction::IntrinsicDistExponential,
    BuiltinFunction::IntrinsicDistPoisson,
    BuiltinFunction::IntrinsicDistSampleN,
    // Stochastic (4)
    BuiltinFunction::IntrinsicBrownianMotion,
    BuiltinFunction::IntrinsicGbm,
    BuiltinFunction::IntrinsicOuProcess,
    BuiltinFunction::IntrinsicRandomWalk,
    // Rolling window (7)
    BuiltinFunction::IntrinsicRollingSum,
    BuiltinFunction::IntrinsicRollingMean,
    BuiltinFunction::IntrinsicRollingStd,
    BuiltinFunction::IntrinsicRollingMin,
    BuiltinFunction::IntrinsicRollingMax,
    BuiltinFunction::IntrinsicEma,
    BuiltinFunction::IntrinsicLinearRecurrence,
    // Series transform (7)
    BuiltinFunction::IntrinsicShift,
    BuiltinFunction::IntrinsicDiff,
    BuiltinFunction::IntrinsicPctChange,
    BuiltinFunction::IntrinsicFillna,
    BuiltinFunction::IntrinsicCumsum,
    BuiltinFunction::IntrinsicCumprod,
    BuiltinFunction::IntrinsicClip,
    // Statistics (4)
    BuiltinFunction::IntrinsicCorrelation,
    BuiltinFunction::IntrinsicCovariance,
    BuiltinFunction::IntrinsicPercentile,
    BuiltinFunction::IntrinsicMedian,
    // Trigonometric (4)
    BuiltinFunction::IntrinsicAtan2,
    BuiltinFunction::IntrinsicSinh,
    BuiltinFunction::IntrinsicCosh,
    BuiltinFunction::IntrinsicTanh,
    // Char codes (2)
    BuiltinFunction::IntrinsicCharCode,
    BuiltinFunction::IntrinsicFromCharCode,
    // Series (1)
    BuiltinFunction::IntrinsicSeries,
    // Vector intrinsics (11)
    BuiltinFunction::IntrinsicVecAbs,
    BuiltinFunction::IntrinsicVecSqrt,
    BuiltinFunction::IntrinsicVecLn,
    BuiltinFunction::IntrinsicVecExp,
    BuiltinFunction::IntrinsicVecAdd,
    BuiltinFunction::IntrinsicVecSub,
    BuiltinFunction::IntrinsicVecMul,
    BuiltinFunction::IntrinsicVecDiv,
    BuiltinFunction::IntrinsicVecMax,
    BuiltinFunction::IntrinsicVecMin,
    BuiltinFunction::IntrinsicVecSelect,
    // Matrix (2)
    BuiltinFunction::IntrinsicMatMulVec,
    BuiltinFunction::IntrinsicMatMulMat,
    // Eval helpers (6)
    BuiltinFunction::EvalTimeRef,
    BuiltinFunction::EvalDateTimeExpr,
    BuiltinFunction::EvalDataDateTimeRef,
    BuiltinFunction::EvalDataSet,
    BuiltinFunction::EvalDataRelative,
    BuiltinFunction::EvalDataRelativeRange,
    // Option/Result ctors (3)
    BuiltinFunction::SomeCtor,
    BuiltinFunction::OkCtor,
    BuiltinFunction::ErrCtor,
    // Collection ctors (4)
    BuiltinFunction::HashMapCtor,
    BuiltinFunction::SetCtor,
    BuiltinFunction::DequeCtor,
    BuiltinFunction::PriorityQueueCtor,
    // JSON (5)
    BuiltinFunction::JsonObjectGet,
    BuiltinFunction::JsonArrayAt,
    BuiltinFunction::JsonObjectKeys,
    BuiltinFunction::JsonArrayLen,
    BuiltinFunction::JsonObjectLen,
    // Window functions (14)
    BuiltinFunction::WindowRowNumber,
    BuiltinFunction::WindowRank,
    BuiltinFunction::WindowDenseRank,
    BuiltinFunction::WindowNtile,
    BuiltinFunction::WindowLag,
    BuiltinFunction::WindowLead,
    BuiltinFunction::WindowFirstValue,
    BuiltinFunction::WindowLastValue,
    BuiltinFunction::WindowNthValue,
    BuiltinFunction::WindowSum,
    BuiltinFunction::WindowAvg,
    BuiltinFunction::WindowMin,
    BuiltinFunction::WindowMax,
    BuiltinFunction::WindowCount,
    // Join (1)
    BuiltinFunction::JoinExecute,
    // Reflection (1)
    BuiltinFunction::Reflect,
    // Content (3 + 6 constructors)
    BuiltinFunction::MakeContentText,
    BuiltinFunction::MakeContentFragment,
    BuiltinFunction::ApplyContentStyle,
    BuiltinFunction::MakeContentChartFromValue,
    BuiltinFunction::ContentChart,
    BuiltinFunction::ContentTextCtor,
    BuiltinFunction::ContentTableCtor,
    BuiltinFunction::ContentCodeCtor,
    BuiltinFunction::ContentKvCtor,
    BuiltinFunction::ContentFragmentCtor,
    // DateTime (6)
    BuiltinFunction::DateTimeNow,
    BuiltinFunction::DateTimeUtc,
    BuiltinFunction::DateTimeParse,
    BuiltinFunction::DateTimeFromEpoch,
    BuiltinFunction::DateTimeFromParts,
    BuiltinFunction::DateTimeFromUnixSecs,
    // Concurrency (4)
    BuiltinFunction::MutexCtor,
    BuiltinFunction::AtomicCtor,
    BuiltinFunction::LazyCtor,
    BuiltinFunction::ChannelCtor,
    // Math extras (7)
    BuiltinFunction::Sign,
    BuiltinFunction::Gcd,
    BuiltinFunction::Lcm,
    BuiltinFunction::Hypot,
    BuiltinFunction::Clamp,
    BuiltinFunction::IsNaN,
    BuiltinFunction::IsFinite,
    // Table construction
    BuiltinFunction::MakeTableFromRows,
];

fn vm_only_opcode_reason(_opcode: OpCode) -> Option<&'static str> {
    // All opcodes are now compiled by the JIT translator — either natively
    // or via FFI trampoline calls to the VM runtime.  No interpreter
    // fallback is required.
    None
}

fn is_supported_builtin(_builtin: BuiltinFunction) -> bool {
    // All builtins are now supported — either via dedicated JIT lowering
    // or via the generic builtin FFI trampoline.
    true
}

/// Run JIT compatibility preflight on a raw instruction slice.
///
/// This is the shared core used by both `preflight_blob_jit_compatibility`
/// and `preflight_jit_compatibility`. It enables per-function JIT/interpreter
/// decisions in the mixed function table.
pub fn preflight_instructions(instructions: &[Instruction]) -> JitPreflightReport {
    let mut report = JitPreflightReport::default();

    for instr in instructions {
        if vm_only_opcode_reason(instr.opcode).is_some() {
            push_unique_opcode(&mut report.vm_only_opcodes, instr.opcode);
        }

        if instr.opcode == OpCode::BuiltinCall {
            if let Some(Operand::Builtin(builtin)) = instr.operand {
                if !is_supported_builtin(builtin) {
                    push_unique_builtin(&mut report.unsupported_builtins, builtin);
                }
            }
        }
    }

    sort_opcodes(&mut report.vm_only_opcodes);
    sort_builtins(&mut report.unsupported_builtins);
    report
}

/// Run JIT compatibility preflight on a single function blob.
///
/// Same logic as the whole-program preflight but operating on a single
/// `FunctionBlob`'s instruction stream. This enables per-function
/// JIT/interpreter decisions in the mixed function table.
pub fn preflight_blob_jit_compatibility(
    blob: &shape_vm::bytecode::FunctionBlob,
) -> JitPreflightReport {
    preflight_instructions(&blob.instructions)
}

/// Run JIT preflight and collect all constructs that require VM fallback.
pub fn preflight_jit_compatibility(program: &BytecodeProgram) -> JitPreflightReport {
    let mut report = JitPreflightReport::default();

    for instr in &program.instructions {
        if vm_only_opcode_reason(instr.opcode).is_some() {
            push_unique_opcode(&mut report.vm_only_opcodes, instr.opcode);
        }

        if instr.opcode == OpCode::BuiltinCall {
            if let Some(Operand::Builtin(builtin)) = instr.operand {
                if !is_supported_builtin(builtin) {
                    push_unique_builtin(&mut report.unsupported_builtins, builtin);
                }
            }
        }
    }

    sort_opcodes(&mut report.vm_only_opcodes);
    sort_builtins(&mut report.unsupported_builtins);
    report
}

/// Build a program-specific JIT parity matrix.
///
/// This is intended for diagnostics/CI tooling that wants an automatic
/// per-program support report.
pub fn build_program_parity_matrix(program: &BytecodeProgram) -> Vec<JitParityEntry> {
    let mut opcodes = Vec::new();
    let mut builtins = Vec::new();

    for instr in &program.instructions {
        push_unique_opcode(&mut opcodes, instr.opcode);
        if instr.opcode == OpCode::BuiltinCall {
            if let Some(Operand::Builtin(builtin)) = instr.operand {
                push_unique_builtin(&mut builtins, builtin);
            }
        }
    }

    sort_opcodes(&mut opcodes);
    sort_builtins(&mut builtins);

    let mut matrix = Vec::with_capacity(opcodes.len() + builtins.len());

    for opcode in opcodes {
        if let Some(reason) = vm_only_opcode_reason(opcode) {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Opcode(opcode),
                jit_supported: false,
                reason,
            });
        } else {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Opcode(opcode),
                jit_supported: true,
                reason: "Opcode is lowered by the JIT translator.",
            });
        }
    }

    for builtin in builtins {
        if is_supported_builtin(builtin) {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Builtin(builtin),
                jit_supported: true,
                reason: "Builtin is lowered by JIT builtin handlers.",
            });
        } else {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Builtin(builtin),
                jit_supported: false,
                reason: "Builtin is not lowered by JIT and must run on VM.",
            });
        }
    }

    matrix.sort_by_key(|entry| format!("{:?}", entry.target));
    matrix
}

/// Build a full opcode parity matrix across the entire VM opcode surface.
pub fn build_full_opcode_parity_matrix() -> Vec<JitParityEntry> {
    let mut matrix = Vec::with_capacity(ALL_OPCODES.len());
    for &opcode in ALL_OPCODES {
        if let Some(reason) = vm_only_opcode_reason(opcode) {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Opcode(opcode),
                jit_supported: false,
                reason,
            });
        } else {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Opcode(opcode),
                jit_supported: true,
                reason: "Opcode is lowered by the JIT translator.",
            });
        }
    }
    matrix.sort_by_key(|entry| format!("{:?}", entry.target));
    matrix
}

/// Build a full builtin parity matrix across the entire BuiltinFunction surface.
pub fn build_full_builtin_parity_matrix() -> Vec<JitParityEntry> {
    let mut matrix = Vec::with_capacity(ALL_BUILTINS.len());
    for &builtin in ALL_BUILTINS {
        if is_supported_builtin(builtin) {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Builtin(builtin),
                jit_supported: true,
                reason: "Builtin is lowered by JIT builtin handlers.",
            });
        } else {
            matrix.push(JitParityEntry {
                target: JitParityTarget::Builtin(builtin),
                jit_supported: false,
                reason: "Builtin is not lowered by JIT and must run on VM.",
            });
        }
    }
    matrix.sort_by_key(|entry| format!("{:?}", entry.target));
    matrix
}

/// Check if a bytecode program can be fully JIT-compiled
#[inline(always)]
pub fn can_jit_compile(program: &BytecodeProgram) -> bool {
    preflight_jit_compatibility(program).can_jit()
}

/// Get a list of unsupported opcodes in a program (for debugging)
#[inline(always)]
pub fn get_unsupported_opcodes(program: &BytecodeProgram) -> Vec<OpCode> {
    let report = preflight_jit_compatibility(program);
    let mut unsupported = report.vm_only_opcodes;

    if !report.unsupported_builtins.is_empty() && !unsupported.contains(&OpCode::BuiltinCall) {
        unsupported.push(OpCode::BuiltinCall);
    }

    sort_opcodes(&mut unsupported);
    unsupported
}

/// Get a list of opcodes that have placeholder (incomplete) implementations
pub fn get_incomplete_opcodes(_program: &BytecodeProgram) -> Vec<OpCode> {
    // All opcodes now have full implementations (native or FFI).
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_vm::bytecode::{Instruction, Operand};

    #[test]
    fn preflight_accepts_all_opcodes() {
        // All opcodes are now supported — no VM-only gates remain.
        let program = BytecodeProgram {
            instructions: vec![Instruction::simple(OpCode::Await)],
            ..Default::default()
        };
        let report = preflight_jit_compatibility(&program);
        assert!(report.can_jit());
    }

    #[test]
    fn preflight_accepts_all_builtins() {
        // All builtins are now supported — dedicated or generic trampoline.
        let program = BytecodeProgram {
            instructions: vec![Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(BuiltinFunction::Snapshot)),
            )],
            ..Default::default()
        };
        let report = preflight_jit_compatibility(&program);
        assert!(report.can_jit());
    }

    #[test]
    fn parity_matrix_marks_all_builtins_supported() {
        let program = BytecodeProgram {
            instructions: vec![Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(BuiltinFunction::Snapshot)),
            )],
            ..Default::default()
        };
        let matrix = build_program_parity_matrix(&program);
        assert!(matrix.iter().all(|row| row.jit_supported));
    }

    #[test]
    fn preflight_instructions_compatible_slice() {
        let instructions = vec![
            Instruction::simple(OpCode::PushConst),
            Instruction::simple(OpCode::Add),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let report = preflight_instructions(&instructions);
        assert!(report.can_jit());
    }

    #[test]
    fn preflight_instructions_all_opcodes_pass() {
        // Even async opcodes now pass preflight.
        let instructions = vec![
            Instruction::simple(OpCode::PushConst),
            Instruction::simple(OpCode::Await),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let report = preflight_instructions(&instructions);
        assert!(report.can_jit());
    }

    #[test]
    fn preflight_blob_passes_with_spawn_task() {
        use shape_vm::bytecode::FunctionBlob;

        let blob = FunctionBlob {
            content_hash: shape_vm::bytecode::FunctionHash::ZERO,
            name: "test_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            instructions: vec![
                Instruction::simple(OpCode::PushConst),
                Instruction::simple(OpCode::SpawnTask),
                Instruction::simple(OpCode::ReturnValue),
            ],
            constants: vec![],
            strings: vec![],
            required_permissions: Default::default(),
            dependencies: vec![],
            callee_names: vec![],
            type_schemas: vec![],
            source_map: vec![],
            foreign_dependencies: vec![],
            frame_descriptor: None,
        };

        let report = preflight_blob_jit_compatibility(&blob);
        assert!(report.can_jit());
    }

    #[test]
    fn all_opcodes_pass_preflight() {
        // Exhaustive check: every opcode in ALL_OPCODES must pass preflight.
        for &opcode in ALL_OPCODES {
            assert!(
                vm_only_opcode_reason(opcode).is_none(),
                "Opcode {:?} should pass preflight",
                opcode
            );
        }
    }

    #[test]
    fn all_builtins_pass_preflight() {
        // Exhaustive check: every builtin in ALL_BUILTINS must be supported.
        for &builtin in ALL_BUILTINS {
            assert!(
                is_supported_builtin(builtin),
                "Builtin {:?} should be supported",
                builtin
            );
        }
    }
}
