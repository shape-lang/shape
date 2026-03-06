//! Compile-time string interpolation compilation.
//!
//! Interpolation syntax parsing itself lives in `shape-ast` so compiler,
//! type inference, and LSP all use the same parser.

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::InterpolationMode;
use shape_ast::error::{Result, ShapeError};
use shape_ast::interpolation::{
    ColorSpec, ContentFormatSpec, FormatAlignment, FormatColor, InterpolationFormatSpec,
    InterpolationPart, NamedContentColor, parse_content_interpolation_with_mode,
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
                let count = self.program.add_constant(Constant::Number(1.0));
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
                let count = self.program.add_constant(Constant::Number(3.0));
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

                let count = self.program.add_constant(Constant::Number(7.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(BuiltinFunction::FormatValueWithSpec)),
                ));
            }
            Some(InterpolationFormatSpec::ContentStyle(_)) => {
                // Content style specs are handled at the content rendering level,
                // not during string interpolation compilation.
                // For now, treat as plain format (no spec).
                let count = self.program.add_constant(Constant::Number(1.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(BuiltinFunction::FormatValueWithMeta)),
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

            // Concatenate with previous result (except for first part)
            if !first {
                self.emit(Instruction::simple(OpCode::Add));
            }
            first = false;
        }

        Ok(())
    }

    /// Compile a content string expression, producing a ContentNode on the stack.
    ///
    /// For `c"text {expr:fg(red), bold} more"`:
    /// 1. Push literal `text ` → MakeContentText → ContentNode::plain("text ")
    /// 2. Compile expression, convert to string, MakeContentText,
    ///    then ApplyContentStyle with the format spec
    /// 3. Push literal `more` → MakeContentText → ContentNode::plain(" more")
    /// 4. MakeContentFragment to join all parts
    pub(in crate::compiler) fn compile_content_string_expression(
        &mut self,
        s: &str,
        mode: InterpolationMode,
    ) -> Result<()> {
        let parts = parse_content_interpolation_with_mode(s, mode)?;

        if parts.is_empty() {
            // Empty content string → ContentNode::plain("")
            let const_idx = self.program.add_constant(Constant::String(String::new()));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(const_idx)),
            ));
            self.emit_content_builtin(BuiltinFunction::MakeContentText, 1)?;
            return Ok(());
        }

        let part_count = parts.len();

        for part in parts {
            match part {
                InterpolationPart::Literal(text) => {
                    let const_idx = self.program.add_constant(Constant::String(text));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(const_idx)),
                    ));
                    // Convert string to ContentNode::plain(text)
                    self.emit_content_builtin(BuiltinFunction::MakeContentText, 1)?;
                }
                InterpolationPart::Expression { expr, format_spec } => {
                    // Parse and compile the expression
                    let expr = shape_ast::parser::parse_expression_str(&expr).map_err(|e| {
                        ShapeError::RuntimeError {
                            message: format!(
                                "Failed to parse expression '{}' in content string: {}",
                                expr, e
                            ),
                            location: None,
                        }
                    })?;
                    self.compile_expr(&expr)?;

                    // Convert value to string first (ToString builtin)
                    let count = self.program.add_constant(Constant::Number(1.0));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(count)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(BuiltinFunction::FormatValueWithMeta)),
                    ));

                    // Wrap string as ContentNode::plain(text)
                    self.emit_content_builtin(BuiltinFunction::MakeContentText, 1)?;

                    // Apply content style if present
                    if let Some(InterpolationFormatSpec::ContentStyle(ref spec)) = format_spec {
                        self.emit_content_style_args(spec)?;
                        self.emit_content_builtin(BuiltinFunction::ApplyContentStyle, 7)?;
                    }
                }
            }
        }

        // If there are multiple parts, combine them into a Fragment
        if part_count > 1 {
            let count_const = self.program.add_constant(Constant::Int(part_count as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(count_const)),
            ));
            self.emit_content_builtin(BuiltinFunction::MakeContentFragment, part_count + 1)?;
        }

        Ok(())
    }

    /// Emit a content builtin call with the given arg count pushed on stack.
    fn emit_content_builtin(&mut self, builtin: BuiltinFunction, arg_count: usize) -> Result<()> {
        let count = self
            .program
            .add_constant(Constant::Number(arg_count as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));
        Ok(())
    }

    /// Push ContentFormatSpec fields onto the stack as constants.
    ///
    /// Stack layout (7 values): [content_node, fg_color, bg_color, bold, italic, underline, dim]
    /// Color encoding: -1 = none, 0-7 = named colors, 256+ = RGB(r*65536 + g*256 + b)
    fn emit_content_style_args(&mut self, spec: &ContentFormatSpec) -> Result<()> {
        // fg color
        let fg_val = Self::encode_color_spec(&spec.fg);
        let fg = self.program.add_constant(Constant::Int(fg_val));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(fg)),
        ));

        // bg color
        let bg_val = Self::encode_color_spec(&spec.bg);
        let bg = self.program.add_constant(Constant::Int(bg_val));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(bg)),
        ));

        // bold
        let bold = self.program.add_constant(Constant::Bool(spec.bold));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(bold)),
        ));

        // italic
        let italic = self.program.add_constant(Constant::Bool(spec.italic));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(italic)),
        ));

        // underline
        let underline = self.program.add_constant(Constant::Bool(spec.underline));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(underline)),
        ));

        // dim
        let dim = self.program.add_constant(Constant::Bool(spec.dim));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(dim)),
        ));

        Ok(())
    }

    /// Encode a ColorSpec as an i64 constant.
    /// -1 = none, 0-7 = named colors, 256+ = RGB
    fn encode_color_spec(color: &Option<ColorSpec>) -> i64 {
        match color {
            None => -1,
            Some(ColorSpec::Named(named)) => match named {
                NamedContentColor::Red => 0,
                NamedContentColor::Green => 1,
                NamedContentColor::Blue => 2,
                NamedContentColor::Yellow => 3,
                NamedContentColor::Magenta => 4,
                NamedContentColor::Cyan => 5,
                NamedContentColor::White => 6,
                NamedContentColor::Default => 7,
            },
            Some(ColorSpec::Rgb(r, g, b)) => {
                256 + (*r as i64) * 65536 + (*g as i64) * 256 + (*b as i64)
            }
        }
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
