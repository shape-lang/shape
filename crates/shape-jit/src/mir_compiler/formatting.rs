//! Typed MIR formatted-string lowering.

use cranelift::prelude::*;
use shape_ast::ast::Span;
use shape_vm::mir::types::{MirFormatSpec, Operand};
use shape_vm::type_tracking::NativeKind;

use super::MirToIR;

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
        let precision = match spec {
            MirFormatSpec::Default => None,
            MirFormatSpec::Fixed { precision } => Some(precision),
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

        let (formatter, expected_type) = match (precision, source_kind) {
            (None, NativeKind::Int64) => (self.ffi.format_default_i64, types::I64),
            (None, NativeKind::Bool) => (self.ffi.format_default_bool, types::I8),
            (None, NativeKind::Float64) => (self.ffi.format_default_f64, types::F64),
            (None, NativeKind::String) => (self.ffi.format_default_string, types::I64),
            (Some(_), NativeKind::Int64) => (self.ffi.format_fixed_i64, types::I64),
            (Some(_), NativeKind::Bool) => (self.ffi.format_fixed_bool, types::I8),
            (Some(_), NativeKind::Float64) => (self.ffi.format_fixed_f64, types::F64),
            (Some(_), NativeKind::String) => (self.ffi.format_fixed_string, types::I64),
            (_, unsupported) => {
                return Err(format!(
                    "FormatValue: source kind {unsupported:?} has no typed JIT formatter; \
                     refusing before native execution"
                ));
            }
        };

        let value = self.compile_operand(operand)?;
        let actual_type = self.builder.func.dfg.value_type(value);
        if actual_type != expected_type {
            return Err(format!(
                "FormatValue: proven source kind {source_kind:?} requires native \
                 {expected_type:?}, but operand compiled as {actual_type:?}; refusing \
                 before emitting a formatting call"
            ));
        }

        let call = match precision {
            None => self.builder.ins().call(formatter, &[value]),
            Some(precision) => {
                let precision = self.builder.ins().iconst(types::I8, precision as i64);
                self.builder.ins().call(formatter, &[value, precision])
            }
        };
        Ok(self.builder.inst_results(call)[0])
    }
}
