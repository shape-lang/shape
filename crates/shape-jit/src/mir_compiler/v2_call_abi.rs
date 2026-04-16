//! Typed function call ABI for the Shape v2 runtime.
//!
//! The v1 ABI passes all arguments as NaN-boxed `I64` values and returns
//! results through `ctx.stack[0]`.  This module defines a v2 ABI where
//! compile-time-proven types are passed in their native Cranelift
//! representation:
//!
//! | SlotKind      | Cranelift type | Register class | Notes                       |
//! |---------------|---------------|----------------|-----------------------------|
//! | Float64       | F64           | XMM (x86)      | Plain double                |
//! | NullableFloat64 | F64         | XMM            | NaN sentinel = null         |
//! | Int64/IntSize | I64           | GPR            | Native int, NOT NaN-boxed   |
//! | Int32/UInt32  | I32           | GPR            | 32-bit integer              |
//! | Int16/UInt16  | I16           | GPR            | 16-bit integer              |
//! | Int8/UInt8    | I8            | GPR            | 8-bit integer               |
//! | Bool          | I8            | GPR            | 0 = false, 1 = true         |
//! | Unknown/other | I64           | GPR            | NaN-boxed fallback          |
//!
//! The v2 signature still starts with `ctx_ptr: I64` as the first parameter
//! (same as v1) and returns `I32` as a signal code (same as v1 direct calls).
//! The difference is that *user arguments* following `ctx_ptr` use native
//! types instead of uniformly boxing to `I64`.
//!
//! # Integration plan
//!
//! This module provides signature-building primitives.  The call site in
//! `translator/opcodes/functions.rs` (`compile_direct_call`) and the
//! function declaration in `compiler/program.rs` (`compile_program`,
//! `compile_function_with_user_funcs`) will be updated in a follow-up
//! change to use `build_cranelift_signature` instead of the current
//! uniform `AbiParam::new(types::I64)` loop.

use cranelift::prelude::*;
use shape_vm::bytecode::Function;
use shape_vm::type_tracking::SlotKind;

// ---------------------------------------------------------------------------
// TypedFunctionSignature
// ---------------------------------------------------------------------------

/// A typed function signature that describes the native representation of
/// each parameter and the return value.
///
/// Unlike the v1 ABI where every slot is `I64` (NaN-boxed), this signature
/// carries per-slot type information extracted from the compiler's
/// `FrameDescriptor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedFunctionSignature {
    /// Native type per parameter (positional, in declaration order).
    pub param_types: Vec<SlotKind>,
    /// Native type of the return value.
    pub return_type: SlotKind,
}

impl TypedFunctionSignature {
    /// Returns true when every parameter *and* the return type are
    /// `SlotKind::Unknown` (or Dynamic), meaning this signature carries no
    /// more information than the v1 ABI and can be compiled with the legacy
    /// uniform-I64 path.
    pub fn is_fully_untyped(&self) -> bool {
        self.param_types.iter().all(|k| is_untyped_slot(*k))
            && is_untyped_slot(self.return_type)
    }
}

/// True when a slot kind would produce the same representation as the v1
/// NaN-boxed ABI (I64 GPR).
fn is_untyped_slot(kind: SlotKind) -> bool {
    matches!(kind, SlotKind::Unknown | SlotKind::Dynamic)
}

// ---------------------------------------------------------------------------
// SlotKind -> Cranelift type mapping
// ---------------------------------------------------------------------------

/// Map a single `SlotKind` to the Cranelift IR type that should be used in
/// the function signature.
///
/// The mapping follows the native-width convention:
/// - Floating-point kinds -> `F64` (passed in XMM registers on x86-64)
/// - Integer kinds -> `I64`, `I32`, `I16`, or `I8` depending on width
/// - Bool -> `I8`
/// - String / Unknown / Dynamic -> `I64` (dynamic fallback)
pub fn slot_kind_to_clif_type(kind: SlotKind) -> types::Type {
    match kind {
        // --- floating point ---
        SlotKind::Float64 | SlotKind::NullableFloat64 => types::F64,

        // --- 64-bit integers ---
        SlotKind::Int64
        | SlotKind::NullableInt64
        | SlotKind::UInt64
        | SlotKind::NullableUInt64
        | SlotKind::IntSize
        | SlotKind::NullableIntSize
        | SlotKind::UIntSize
        | SlotKind::NullableUIntSize => types::I64,

        // --- 32-bit integers ---
        SlotKind::Int32 | SlotKind::NullableInt32 | SlotKind::UInt32 | SlotKind::NullableUInt32 => {
            types::I32
        }

        // --- 16-bit integers ---
        SlotKind::Int16 | SlotKind::NullableInt16 | SlotKind::UInt16 | SlotKind::NullableUInt16 => {
            types::I16
        }

        // --- 8-bit integers ---
        SlotKind::Int8 | SlotKind::NullableInt8 | SlotKind::UInt8 | SlotKind::NullableUInt8 => {
            types::I8
        }

        // --- boolean ---
        SlotKind::Bool => types::I8,

        // --- fallback (dynamic) ---
        SlotKind::String | SlotKind::Dynamic | SlotKind::Unknown => types::I64,
    }
}

// ---------------------------------------------------------------------------
// Signature builders
// ---------------------------------------------------------------------------

/// Build a Cranelift `Signature` from a `TypedFunctionSignature`.
///
/// The emitted signature has the form:
///
/// ```text
/// fn(ctx_ptr: I64, arg0: <native>, arg1: <native>, ...) -> I32
/// ```
///
/// - The first parameter is always `ctx_ptr` (pointer-width integer)
///   regardless of the typed signature.  This matches the v1 ABI contract
///   that every JIT-compiled function receives a `*mut JITContext`.
/// - The return type is `I32` (signal code: >= 0 means success, < 0 means
///   deopt).  The *semantic* return value is written to `ctx.stack[0]` by
///   the callee, matching the existing convention.  A future v2 extension
///   may promote the return value into a native register as well.
pub fn build_cranelift_signature(sig: &TypedFunctionSignature) -> Signature {
    let mut clif_sig = Signature::new(cranelift::prelude::isa::CallConv::SystemV);

    // ctx_ptr is always the first parameter.
    clif_sig.params.push(AbiParam::new(types::I64));

    // User arguments in their native representation.
    for kind in &sig.param_types {
        clif_sig.params.push(AbiParam::new(slot_kind_to_clif_type(*kind)));
    }

    // Signal return (same as v1 direct-call convention).
    clif_sig.returns.push(AbiParam::new(types::I32));

    clif_sig
}

// ---------------------------------------------------------------------------
// Resolving a TypedFunctionSignature from bytecode metadata
// ---------------------------------------------------------------------------

/// Extract a `TypedFunctionSignature` from a bytecode `Function`.
///
/// When the function carries a `FrameDescriptor`, the first `arity` slots
/// are the parameter types and `return_kind` is the return type.  When no
/// descriptor is present (legacy code), all slots default to `Unknown`
/// which produces a v1-compatible all-I64 signature.
///
/// `slot_kinds_override` is an optional caller-provided slice that takes
/// precedence over the frame descriptor.  This is useful when the JIT's
/// own type inference (e.g., feedback-guided profiling) has refined the
/// types beyond what the bytecode compiler emitted.
pub fn resolve_function_signature(
    func: &Function,
    slot_kinds_override: &[SlotKind],
) -> TypedFunctionSignature {
    let arity = func.arity as usize;

    // If the caller provided overrides, use them directly.
    if !slot_kinds_override.is_empty() {
        let param_types: Vec<SlotKind> = if slot_kinds_override.len() >= arity {
            slot_kinds_override[..arity].to_vec()
        } else {
            // Pad with Unknown for any missing slots.
            let mut kinds = slot_kinds_override.to_vec();
            kinds.resize(arity, SlotKind::Unknown);
            kinds
        };

        // The override slice doesn't carry a return type; fall back to
        // the frame descriptor if available, otherwise Unknown.
        let return_type = func
            .frame_descriptor
            .as_ref()
            .map(|fd| fd.return_kind)
            .unwrap_or(SlotKind::Unknown);

        return TypedFunctionSignature {
            param_types,
            return_type,
        };
    }

    // Use the frame descriptor when available.
    if let Some(fd) = &func.frame_descriptor {
        let param_types: Vec<SlotKind> = if fd.slots.len() >= arity {
            fd.slots[..arity].to_vec()
        } else {
            let mut kinds = fd.slots.clone();
            kinds.resize(arity, SlotKind::Unknown);
            kinds
        };

        TypedFunctionSignature {
            param_types,
            return_type: fd.return_kind,
        }
    } else {
        // No type information at all -- fully untyped (v1 compatible).
        TypedFunctionSignature {
            param_types: vec![SlotKind::Unknown; arity],
            return_type: SlotKind::Unknown,
        }
    }
}
