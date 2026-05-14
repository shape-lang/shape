//! Typed function call ABI for the Shape v2 runtime.
//!
//! The v1 ABI passes all arguments as NaN-boxed `I64` values and returns
//! results through `ctx.stack[0]`.  This module defines a v2 ABI where
//! compile-time-proven types are passed in their native Cranelift
//! representation:
//!
//! | NativeKind      | Cranelift type | Register class | Notes                       |
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
use shape_vm::type_tracking::NativeKind;

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
    pub param_types: Vec<NativeKind>,
    /// Native type of the return value.
    pub return_type: NativeKind,
}

impl TypedFunctionSignature {
    /// Returns true when every parameter *and* the return type are
    /// `NativeKind::Unknown` (or Dynamic), meaning this signature carries no
    /// more information than the v1 ABI and can be compiled with the legacy
    /// uniform-I64 path.
    pub fn is_fully_untyped(&self) -> bool {
        self.param_types.iter().all(|k| is_untyped_slot(*k))
            && is_untyped_slot(self.return_type)
    }
}

/// True when a slot kind would produce the same representation as the v1
/// NaN-boxed ABI (I64 GPR). Per ADR-006 the deleted
/// `NativeKind::Unknown`/`Dynamic` placeholders are gone — kind-tracked
/// signatures are exhaustive at call signature build time, so
/// `is_fully_untyped` only fires when *every* signature slot already has
/// a typed kind that happens to map to I64 (Int64/UInt64/IntSize/etc.).
fn is_untyped_slot(kind: NativeKind) -> bool {
    // The deleted Unknown / Dynamic arms are removed — every kind is
    // typed. The legacy "uniform-I64 v1 path" gate now never fires.
    let _ = kind;
    false
}

// ---------------------------------------------------------------------------
// NativeKind -> Cranelift type mapping
// ---------------------------------------------------------------------------

/// Map a single `NativeKind` to the Cranelift IR type that should be used in
/// the function signature.
///
/// The mapping follows the native-width convention:
/// - Floating-point kinds -> `F64` (passed in XMM registers on x86-64)
/// - Integer kinds -> `I64`, `I32`, `I16`, or `I8` depending on width
/// - Bool -> `I8`
/// - String / Unknown / Dynamic -> `I64` (dynamic fallback)
pub fn slot_kind_to_clif_type(kind: NativeKind) -> types::Type {
    match kind {
        // --- floating point ---
        NativeKind::Float64 | NativeKind::NullableFloat64 => types::F64,

        // --- 64-bit integers ---
        NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize => types::I64,

        // --- 32-bit integers ---
        NativeKind::Int32 | NativeKind::NullableInt32 | NativeKind::UInt32 | NativeKind::NullableUInt32 => {
            types::I32
        }

        // --- 16-bit integers ---
        NativeKind::Int16 | NativeKind::NullableInt16 | NativeKind::UInt16 | NativeKind::NullableUInt16 => {
            types::I16
        }

        // --- 8-bit integers ---
        NativeKind::Int8 | NativeKind::NullableInt8 | NativeKind::UInt8 | NativeKind::NullableUInt8 => {
            types::I8
        }

        // --- boolean ---
        NativeKind::Bool => types::I8,

        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment — F32 ABI passes in XMM (F32 native
        // type); Char ABI passes as I32 (codepoint bits).
        NativeKind::Float32 => types::F32,
        NativeKind::Char => types::I32,

        // --- pointer-sized typed slots (heap arms + String) ---
        NativeKind::String | NativeKind::Ptr(_) => types::I64,
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
    slot_kinds_override: &[NativeKind],
) -> TypedFunctionSignature {
    // Per ADR-006 §2.7.7 the deleted `NativeKind::Unknown`/`Dynamic`
    // placeholders are gone. When inference produced no kind for a
    // missing slot or absent FrameDescriptor, fall back to `Int64`
    // (matches the legacy I64-NaN-box ABI width via
    // `slot_kind_to_clif_type`'s catch-all). Per-call-signature
    // strict typing requires every slot to have a real kind; this
    // path only fires for functions with no FrameDescriptor at all
    // (heavily-legacy code paths).
    let legacy_default = NativeKind::Int64;
    let arity = func.arity as usize;

    // If the caller provided overrides, use them directly.
    if !slot_kinds_override.is_empty() {
        let param_types: Vec<NativeKind> = if slot_kinds_override.len() >= arity {
            slot_kinds_override[..arity].to_vec()
        } else {
            // Pad with the legacy I64-ABI default for missing slots.
            let mut kinds = slot_kinds_override.to_vec();
            kinds.resize(arity, legacy_default);
            kinds
        };

        // The override slice doesn't carry a return type; fall back to
        // the frame descriptor if available, otherwise the legacy default.
        let return_type = func
            .frame_descriptor
            .as_ref()
            .and_then(|fd| fd.return_kind)
            .unwrap_or(legacy_default);

        return TypedFunctionSignature {
            param_types,
            return_type,
        };
    }

    // Use the frame descriptor when available.
    if let Some(fd) = &func.frame_descriptor {
        let param_types: Vec<NativeKind> = if fd.slots.len() >= arity {
            fd.slots[..arity].to_vec()
        } else {
            let mut kinds = fd.slots.clone();
            kinds.resize(arity, legacy_default);
            kinds
        };

        TypedFunctionSignature {
            param_types,
            return_type: fd.return_kind.unwrap_or(legacy_default),
        }
    } else {
        // No type information at all — fully I64-ABI legacy.
        TypedFunctionSignature {
            param_types: vec![legacy_default; arity],
            return_type: legacy_default,
        }
    }
}
