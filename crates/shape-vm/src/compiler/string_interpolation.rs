//! Compile-time string interpolation compilation.
//!
//! Interpolation syntax parsing itself lives in `shape-ast` so compiler,
//! type inference, and LSP all use the same parser.

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::InterpolationMode;
use shape_ast::error::{Result, ShapeError};
use shape_ast::interpolation::{
    FormatAlignment, FormatColor, InterpolationFormatSpec, InterpolationPart,
    parse_interpolation_with_mode,
};
pub use shape_ast::interpolation::{has_interpolation, has_interpolation_with_mode};

const FORMAT_SPEC_FIXED: i64 = 1;
const FORMAT_SPEC_TABLE: i64 = 2;

impl BytecodeCompiler {
    fn emit_interpolation_format_call(
        &mut self,
        format_spec: Option<&InterpolationFormatSpec>,
    ) -> Result<()> {
        match format_spec {
            None => {
                // Args: [value]
                let count = self.program.add_constant(Constant::Int(1));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(BuiltinFunction::FormatValueWithMeta)),
                ));
            }
            Some(InterpolationFormatSpec::Fixed { precision }) => {
                // Args: [value, spec_tag, precision]
                let tag = self.program.add_constant(Constant::Int(FORMAT_SPEC_FIXED));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tag)),
                ));
                let precision = self.program.add_constant(Constant::Int(*precision as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(precision)),
                ));
                let count = self.program.add_constant(Constant::Int(3));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(BuiltinFunction::FormatValueWithSpec)),
                ));
            }
            Some(InterpolationFormatSpec::Table(spec)) => {
                // Args: [value, spec_tag, max_rows, align, precision, color, border]
                let tag = self.program.add_constant(Constant::Int(FORMAT_SPEC_TABLE));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tag)),
                ));

                let max_rows = self
                    .program
                    .add_constant(Constant::Int(spec.max_rows.map(|v| v as i64).unwrap_or(-1)));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(max_rows)),
                ));

                let align = self.program.add_constant(Constant::Int(
                    spec.align
                        .map(|v| match v {
                            FormatAlignment::Left => 0,
                            FormatAlignment::Center => 1,
                            FormatAlignment::Right => 2,
                        })
                        .unwrap_or(-1),
                ));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(align)),
                ));

                let precision = self.program.add_constant(Constant::Int(
                    spec.precision.map(|v| v as i64).unwrap_or(-1),
                ));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(precision)),
                ));

                let color = self.program.add_constant(Constant::Int(
                    spec.color
                        .map(|v| match v {
                            FormatColor::Default => 0,
                            FormatColor::Red => 1,
                            FormatColor::Green => 2,
                            FormatColor::Yellow => 3,
                            FormatColor::Blue => 4,
                            FormatColor::Magenta => 5,
                            FormatColor::Cyan => 6,
                            FormatColor::White => 7,
                        })
                        .unwrap_or(-1),
                ));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(color)),
                ));

                let border = self.program.add_constant(Constant::Bool(spec.border));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(border)),
                ));

                let count = self.program.add_constant(Constant::Int(7));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(BuiltinFunction::FormatValueWithSpec)),
                ));
            }
        }

        Ok(())
    }

    /// Compile an interpolated string, producing a single string value on the stack.
    ///
    /// For `text {expr} more`:
    /// 1. Push literal `text `
    /// 2. Compile expression, call `FormatValueWithMeta`
    /// 3. Concatenate with `Add`
    /// 4. Continue for remaining parts
    pub(in crate::compiler) fn compile_interpolated_string_expression(
        &mut self,
        s: &str,
        mode: InterpolationMode,
    ) -> Result<()> {
        let parts = parse_interpolation_with_mode(s, mode)?;

        if parts.is_empty() {
            // Empty string
            let const_idx = self.program.add_constant(Constant::String(String::new()));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(const_idx)),
            ));
            return Ok(());
        }

        let mut first = true;

        for part in parts {
            match part {
                InterpolationPart::Literal(text) => {
                    let const_idx = self.program.add_constant(Constant::String(text));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(const_idx)),
                    ));
                }
                InterpolationPart::Expression { expr, format_spec } => {
                    // Parse the expression string
                    let expr = shape_ast::parser::parse_expression_str(&expr).map_err(|e| {
                        ShapeError::RuntimeError {
                            message: format!(
                                "Failed to parse expression '{}' in interpolation: {}",
                                expr, e
                            ),
                            location: None,
                        }
                    })?;

                    // Compile the expression
                    self.compile_expr(&expr)?;

                    // Format value using typed interpolation spec.
                    self.emit_interpolation_format_call(format_spec.as_ref())?;
                }
            }

            // Concatenate with previous result (except for first part).
            // Every interpolation part is statically a String — literal parts
            // come from `Constant::String` and expression parts go through
            // `emit_interpolation_format_call` which always produces a string.
            if !first {
                self.emit(Instruction::simple(OpCode::StringConcat));
            }
            first = false;
        }

        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::interpolation::parse_interpolation_with_mode;

    fn parse_braces(s: &str) -> shape_ast::error::Result<Vec<InterpolationPart>> {
        parse_interpolation_with_mode(s, InterpolationMode::Braces)
    }

    #[test]
    fn test_no_interpolation() {
        let parts = parse_braces("Hello World").unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "Hello World"));
    }

    #[test]
    fn test_simple_interpolation() {
        let parts = parse_braces("value: {x}").unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "value: "));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "x"
        ));
    }

    #[test]
    fn test_expression_interpolation() {
        let parts = parse_braces("sum: {x + y}").unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "sum: "));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "x + y"
        ));
    }

    #[test]
    fn test_multiple_interpolations() {
        let parts = parse_braces("a={a}, b={b}").unwrap();
        assert_eq!(parts.len(), 4);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "a="));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "a"
        ));
        assert!(matches!(&parts[2], InterpolationPart::Literal(s) if s == ", b="));
        assert!(matches!(
            &parts[3],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "b"
        ));
    }

    #[test]
    fn test_escaped_braces() {
        let parts = parse_braces("Use {{x}} for literal").unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "Use {x} for literal"));
    }

    #[test]
    fn test_as_type_in_interpolation() {
        let parts = parse_braces("{x as Percent}").unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "x as Percent"
        ));
    }

    #[test]
    fn test_nested_braces_in_object() {
        let parts = parse_braces("obj: {x.method({a: 1})}").unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "x.method({a: 1})"
        ));
    }

    #[test]
    fn test_interpolation_with_format_spec() {
        let parts = parse_braces("px={price:fixed(2)}").unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "px="));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: Some(spec)
            } if expr == "price"
                && *spec == InterpolationFormatSpec::Fixed { precision: 2 }
        ));
    }

    #[test]
    fn test_interpolation_does_not_split_double_colon() {
        let parts = parse_braces("{Type::Variant}").unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "Type::Variant"
        ));
    }

    #[test]
    fn test_missing_format_spec_error() {
        let result = parse_braces("value: {x:}");
        assert!(result.is_err());
    }

    #[test]
    fn test_unmatched_close_brace_error() {
        let result = parse_braces("value: }");
        assert!(result.is_err());
    }

    #[test]
    fn test_has_interpolation() {
        assert!(has_interpolation_with_mode(
            "value: {x}",
            InterpolationMode::Braces
        ));
        assert!(has_interpolation_with_mode(
            "{x + y}",
            InterpolationMode::Braces
        ));
        assert!(!has_interpolation_with_mode(
            "Hello World",
            InterpolationMode::Braces
        ));
        assert!(!has_interpolation_with_mode(
            "Use {{x}} for literal",
            InterpolationMode::Braces
        )); // Escaped, no real interpolation
    }

    #[test]
    fn test_empty_interpolation_error() {
        let result = parse_braces("value: {}");
        assert!(result.is_err());
    }

    #[test]
    fn test_dollar_mode_interpolation() {
        let parts =
            parse_interpolation_with_mode("{\"name\": ${user.name}}", InterpolationMode::Dollar)
                .unwrap();
        assert_eq!(parts.len(), 3);
        assert!(matches!(
            &parts[0],
            InterpolationPart::Literal(s) if s == "{\"name\": "
        ));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "user.name"
        ));
        assert!(matches!(&parts[2], InterpolationPart::Literal(s) if s == "}"));
    }
}
