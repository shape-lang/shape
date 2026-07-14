//! Typed, compilation-order-independent closure specialization identity.

use shape_ast::ast::CaptureMode;
use shape_value::v2::closure_layout::CaptureKind;
use shape_value::v2::concrete_type::ConcreteType;
use shape_value::v2::function_type_registry::FunctionSignature;

use crate::compiler::BytecodeCompiler;

use super::super::CapturePack;

/// Structural specialization identity for one compiled closure instance.
///
/// Opaque registry IDs and `func_idx` are deliberately absent. The identity
/// is the typed capture layout plus the typed callable signature that drove
/// compilation, so query results are independent of compilation order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedCaptureSpecializationIdentity {
    capture_types: Vec<ConcreteType>,
    capture_modes: Vec<Option<CaptureMode>>,
    capture_kinds: Vec<CaptureKind>,
    callable_signature: FunctionSignature,
}

impl GeneratedCaptureSpecializationIdentity {
    pub fn capture_types(&self) -> &[ConcreteType] {
        &self.capture_types
    }

    pub fn capture_modes(&self) -> &[Option<CaptureMode>] {
        &self.capture_modes
    }

    pub fn capture_kinds(&self) -> &[CaptureKind] {
        &self.capture_kinds
    }

    pub fn callable_signature(&self) -> &FunctionSignature {
        &self.callable_signature
    }

    /// Stable diagnostic/debug rendering. It is never parsed as identity.
    pub fn canonical_descriptor(&self) -> String {
        let captures = self
            .capture_types
            .iter()
            .zip(&self.capture_modes)
            .zip(&self.capture_kinds)
            .map(|((ty, mode), kind)| {
                let mode = mode.map_or("inferred", CaptureMode::variant_name);
                let kind = super::super::capture_kind_spelling(*kind);
                format!("{}:{mode}:{kind}", ty.mono_key())
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "captures:[{captures}]:{}",
            self.callable_signature.mono_key()
        )
    }
}

/// One exact descriptor type within one structural specialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCaptureSpecialization {
    identity: GeneratedCaptureSpecializationIdentity,
    capture_type: ConcreteType,
}

impl GeneratedCaptureSpecialization {
    pub fn identity(&self) -> &GeneratedCaptureSpecializationIdentity {
        &self.identity
    }

    pub fn capture_type(&self) -> &ConcreteType {
        &self.capture_type
    }
}

pub(super) fn specialization_for(
    compiler: &BytecodeCompiler,
    pack: &CapturePack,
    descriptor_ordinal: usize,
) -> Result<GeneratedCaptureSpecialization, &'static str> {
    let function_type_id = compiler
        .function_type_ids
        .iter()
        .find_map(|(closure, type_id)| (*closure == pack.closure).then_some(*type_id))
        .ok_or("closure pack has no callable-signature identity")?;
    let callable_signature = compiler
        .function_type_registry
        .get(function_type_id)
        .cloned()
        .ok_or("closure pack has an unknown callable-signature identity")?;
    let descriptor = pack
        .descriptors
        .get(descriptor_ordinal)
        .ok_or("capture descriptor ordinal is outside its closure pack")?;
    let identity = GeneratedCaptureSpecializationIdentity {
        capture_types: pack
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capture_type.clone())
            .collect(),
        capture_modes: pack
            .descriptors
            .iter()
            .map(|descriptor| descriptor.declared)
            .collect(),
        capture_kinds: pack
            .descriptors
            .iter()
            .map(|descriptor| descriptor.lowered)
            .collect(),
        callable_signature,
    };
    Ok(GeneratedCaptureSpecialization {
        identity,
        capture_type: descriptor.capture_type.clone(),
    })
}
