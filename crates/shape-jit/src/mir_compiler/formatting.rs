//! Typed MIR formatted-string lowering.

use cranelift::prelude::*;
use shape_ast::ast::Span;
use shape_vm::mir::types::{MirFormatSpec, Operand};
use shape_vm::type_tracking::NativeKind;

use super::MirToIR;
use crate::ffi::{formatting, stack_kind_code};

/// Return the whole-program JIT preflight blocker for a VM-owned format class.
pub(super) fn preflight_blocker(spec: MirFormatSpec, span: Span) -> Option<String> {
    match spec {
        MirFormatSpec::Default | MirFormatSpec::Fixed { .. } => None,
        MirFormatSpec::Table => Some(format!(
            "FormatValue Table spec has no native renderer; whole-program \
             deopt via `[jit-fallback]` preserves the VM's explicit \
             FORMAT_SPEC_TABLE rejection at {span:?}"
        )),
        MirFormatSpec::ContentStyle => Some(format!(
            "FormatValue ContentStyle produces typed content, not a native \
             String; whole-program deopt via `[jit-fallback]` preserves the \
             VM content lowering at {span:?}"
        )),
    }
}

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Materialize one f-string expression part as a canonical String.
    ///
    /// Both the source carrier and formatting class are proven before any
    /// native call is emitted. The FFI consumes the moved operand share and
    /// returns a fresh `Arc<String>` raw carrier.
    pub(super) fn compile_format_value(
        &mut self,
        operand: &Operand,
        spec: MirFormatSpec,
    ) -> Result<Value, String> {
        let (spec_code, precision) = match spec {
            MirFormatSpec::Default => (formatting::FORMAT_DEFAULT, 0),
            MirFormatSpec::Fixed { precision } => (formatting::FORMAT_FIXED, precision),
            MirFormatSpec::Table => {
                return Err(
                    "FormatValue: Table formatting is not implemented by the JIT; \
                     preflight must route the complete program through the VM"
                        .to_string(),
                );
            }
            MirFormatSpec::ContentStyle => {
                return Err(
                    "FormatValue: ContentStyle produces content, not a native String; \
                     preflight must route the complete program through the VM"
                        .to_string(),
                );
            }
        };

        let source_kind = self.operand_slot_kind(operand).ok_or_else(|| {
            "FormatValue: source NativeKind is not proven at JIT compile time; \
             refusing before native execution"
                .to_string()
        })?;
        if !matches!(
            source_kind,
            NativeKind::Int64 | NativeKind::Bool | NativeKind::Float64 | NativeKind::String
        ) {
            return Err(format!(
                "FormatValue: source kind {source_kind:?} has no typed JIT formatter; \
                 refusing before native execution"
            ));
        }

        let value = self.compile_operand(operand)?;
        let bits = self.to_i64_bits(value);
        let kind = self
            .builder
            .ins()
            .iconst(types::I8, stack_kind_code::encode(source_kind) as i64);
        let spec = self.builder.ins().iconst(types::I8, spec_code as i64);
        let precision = self.builder.ins().iconst(types::I8, precision as i64);
        let call = self
            .builder
            .ins()
            .call(self.ffi.format_value, &[bits, kind, spec, precision]);
        Ok(self.builder.inst_results(call)[0])
    }
}
