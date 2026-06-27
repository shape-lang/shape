//! Compile-time string interpolation compilation.
//!
//! Interpolation syntax parsing itself lives in `shape-ast` so compiler,
//! type inference, and LSP all use the same parser.

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::InterpolationMode;
use shape_ast::content_style::{ChartTypeSpec, ColorSpec, ContentFormatSpec, NamedContentColor};
use shape_ast::error::{Result, ShapeError};
use shape_ast::interpolation::{
    FormatAlignment, FormatColor, InterpolationFormatSpec, InterpolationPart,
    parse_interpolation_with_mode,
};
pub use shape_ast::interpolation::{has_interpolation, has_interpolation_with_mode};

const FORMAT_SPEC_FIXED: i64 = 1;
const FORMAT_SPEC_TABLE: i64 = 2;

// R8 W4 W18.4: encoding constants mirroring `executor/vm_impl/builtins.rs`
// `decode_fstring_*` helpers. Keep these in lockstep.
const FSTRING_COLOR_NONE: i64 = -1;
const FSTRING_COLOR_NAMED: i64 = 0;
const FSTRING_COLOR_RGB: i64 = 1;

const FSTRING_FLAG_BOLD: i64 = 1;
const FSTRING_FLAG_ITALIC: i64 = 2;
const FSTRING_FLAG_UNDERLINE: i64 = 4;
const FSTRING_FLAG_DIM: i64 = 8;

fn encode_color_args(color: Option<&ColorSpec>) -> (i64, i64) {
    match color {
        None => (FSTRING_COLOR_NONE, 0),
        Some(ColorSpec::Named(named)) => {
            let id: i64 = match named {
                NamedContentColor::Red => 0,
                NamedContentColor::Green => 1,
                NamedContentColor::Blue => 2,
                NamedContentColor::Yellow => 3,
                NamedContentColor::Magenta => 4,
                NamedContentColor::Cyan => 5,
                NamedContentColor::White => 6,
                NamedContentColor::Default => 7,
            };
            (FSTRING_COLOR_NAMED, id)
        }
        Some(ColorSpec::Rgb(r, g, b)) => {
            let payload = ((*r as i64) << 16) | ((*g as i64) << 8) | (*b as i64);
            (FSTRING_COLOR_RGB, payload)
        }
    }
}

fn encode_flag_bits(spec: &ContentFormatSpec) -> i64 {
    let mut bits: i64 = 0;
    if spec.bold {
        bits |= FSTRING_FLAG_BOLD;
    }
    if spec.italic {
        bits |= FSTRING_FLAG_ITALIC;
    }
    if spec.underline {
        bits |= FSTRING_FLAG_UNDERLINE;
    }
    if spec.dim {
        bits |= FSTRING_FLAG_DIM;
    }
    bits
}

/// R8 W4 W18.4: presence-test for any `ContentStyle` arm in a parsed
/// f-string. Drives the D1 syntax-determined `string` → `content` flip:
/// if any interpolation part carries a content-styling spec, the whole
/// f-string lowers through the `ContentNode::Fragment` path and the
/// inference engine reports `content` as the return type.
pub fn has_content_style_spec(parts: &[InterpolationPart]) -> bool {
    parts.iter().any(|p| {
        matches!(
            p,
            InterpolationPart::Expression {
                format_spec: Some(InterpolationFormatSpec::ContentStyle(_)),
                ..
            }
        )
    })
}

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
            Some(InterpolationFormatSpec::ContentStyle(_)) => {
                // Unreachable on the string-concat lowering path:
                // `compile_interpolated_string_expression` dispatches to
                // the content-fragment path BEFORE this is called when any
                // part has a `ContentStyle` spec. Reaching here means the
                // dispatch decision is out of sync with this match — a
                // compiler bug, not user error.
                return Err(ShapeError::RuntimeError {
                    message: "internal: ContentStyle reached \
                              string-concat emitter — \
                              compile_interpolated_string_expression \
                              dispatch is out of sync"
                        .to_string(),
                    location: None,
                });
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

    /// Compile an interpolated string. Per R8 W4 W18.4 (supervisor
    /// 2026-05-24 D1 + (a-modified) REVIVE-WITH-SHARED-MODULE), the
    /// lowering shape is syntax-determined:
    ///
    /// - If NO part carries a `ContentStyle` spec: traditional
    ///   string-concat path (preserves the `string` return type used by
    ///   500+ existing call sites).
    /// - If ANY part carries a `ContentStyle` spec: lower to a
    ///   `ContentNode::Fragment` of styled/plain text nodes; return type
    ///   is `content` (`Ptr(HeapKind::Content)`).
    pub(in crate::compiler) fn compile_interpolated_string_expression(
        &mut self,
        s: &str,
        mode: InterpolationMode,
    ) -> Result<()> {
        let parts = parse_interpolation_with_mode(s, mode)?;

        if has_content_style_spec(&parts) {
            self.compile_interpolated_string_as_content(&parts)
        } else {
            self.compile_interpolated_string_as_string(parts)
        }
    }

    /// Original f-string lowering path: every part lowers to a string,
    /// concatenated via `StringConcat`. Preserved verbatim from pre-W18.4
    /// behaviour — every existing call site without styling syntax stays
    /// on this path.
    fn compile_interpolated_string_as_string(
        &mut self,
        parts: Vec<InterpolationPart>,
    ) -> Result<()> {
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

                    // Compile the expression. The re-parsed `expr` carries
                    // parser-local spans; guard the ownership-move query so a
                    // fragment-local span collision cannot emit a spurious
                    // `LoadLocalMove` on a live binding (e.g. a loop counter).
                    // See `in_interpolation_expr_depth`.
                    self.in_interpolation_expr_depth += 1;
                    let r = self.compile_expr(&expr);
                    self.in_interpolation_expr_depth -= 1;
                    r?;

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

    /// R8 W4 W18.4 content-fragment lowering:
    /// 1. For each plain literal part: push the string, call
    ///    `FStringContentText` → `ContentNode::Text` plain span on stack.
    /// 2. For each expression with NO styling: compile expr →
    ///    `FormatValueWithMeta` to string → `FStringContentText`.
    /// 3. For each expression WITH `ContentStyle` spec: compile expr →
    ///    `FormatValueWithMeta` to string → push encoded style args →
    ///    `FStringContentStyledText` → styled `ContentNode::Text` on stack.
    /// 4. After all parts emitted: push count → `FStringContentFragment`
    ///    → `ContentNode::Fragment` on stack.
    fn compile_interpolated_string_as_content(
        &mut self,
        parts: &[InterpolationPart],
    ) -> Result<()> {
        if parts.is_empty() {
            // Empty f-string with no parts shouldn't reach here (we only
            // dispatch when content-style is present), but be defensive:
            // emit a plain empty content text.
            self.emit_empty_content_text()?;
            return Ok(());
        }

        let part_count = parts.len();
        let single_part_is_chart = matches!(
            parts,
            [InterpolationPart::Expression {
                format_spec: Some(InterpolationFormatSpec::ContentStyle(spec)),
                ..
            }] if spec.chart_type.is_some()
        );

        for part in parts {
            match part {
                InterpolationPart::Literal(text) => {
                    // Push literal string, then FStringContentText.
                    let const_idx = self.program.add_constant(Constant::String(text.clone()));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(const_idx)),
                    ));
                    self.emit_fstring_content_text_call()?;
                }
                InterpolationPart::Expression { expr, format_spec } => {
                    let parsed_expr =
                        shape_ast::parser::parse_expression_str(expr).map_err(|e| {
                            ShapeError::RuntimeError {
                                message: format!(
                                    "Failed to parse expression '{}' in \
                                     interpolation: {}",
                                    expr, e
                                ),
                                location: None,
                            }
                        })?;
                    // Parser-local spans on the re-parsed inner expression —
                    // guard the ownership-move query (see the string-concat
                    // path above and `in_interpolation_expr_depth`).
                    self.in_interpolation_expr_depth += 1;
                    let r = self.compile_expr(&parsed_expr);
                    self.in_interpolation_expr_depth -= 1;
                    r?;

                    match format_spec {
                        Some(InterpolationFormatSpec::ContentStyle(spec)) => {
                            if spec.chart_type.is_some() {
                                self.emit_fstring_content_chart(spec)?;
                            } else {
                                // Convert expression value to string first.
                                self.emit_format_value_with_meta()?;
                                // Then emit FStringContentStyledText with
                                // encoded style payload.
                                self.emit_fstring_content_styled_text(spec)?;
                            }
                        }
                        _ => {
                            // Plain or fixed/table-spec'd expression: route
                            // through the existing format path to a string,
                            // then wrap as plain content.
                            self.emit_interpolation_format_call(format_spec.as_ref())?;
                            self.emit_fstring_content_text_call()?;
                        }
                    }
                }
            }
        }

        if !single_part_is_chart {
            // Combine all part-results into a Fragment. A single chart-styled
            // expression remains a Chart node so table-to-chart tests can
            // assert the structural carrier directly.
            let count_idx = self.program.add_constant(Constant::Int(part_count as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(count_idx)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(BuiltinFunction::FStringContentFragment)),
            ));
        }

        Ok(())
    }

    fn emit_format_value_with_meta(&mut self) -> Result<()> {
        let count = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::FormatValueWithMeta)),
        ));
        Ok(())
    }

    fn emit_fstring_content_text_call(&mut self) -> Result<()> {
        let count = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::FStringContentText)),
        ));
        Ok(())
    }

    fn emit_fstring_content_styled_text(&mut self, spec: &ContentFormatSpec) -> Result<()> {
        // Stack on entry: [value_str]. Push 5 i64 style args, then 6 as
        // arg-count, then BuiltinCall.
        let (fg_kind, fg_payload) = encode_color_args(spec.fg.as_ref());
        let (bg_kind, bg_payload) = encode_color_args(spec.bg.as_ref());
        let flags = encode_flag_bits(spec);

        for v in [fg_kind, fg_payload, bg_kind, bg_payload, flags] {
            let idx = self.program.add_constant(Constant::Int(v));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(idx)),
            ));
        }
        let count_idx = self.program.add_constant(Constant::Int(6));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count_idx)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::FStringContentStyledText)),
        ));
        Ok(())
    }

    fn emit_fstring_content_chart(&mut self, spec: &ContentFormatSpec) -> Result<()> {
        let chart_type = spec
            .chart_type
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "internal: chart content emitter called without chart type".to_string(),
                location: None,
            })?;
        let chart_type_idx = self
            .program
            .add_constant(Constant::String(chart_type_name(chart_type).to_string()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(chart_type_idx)),
        ));

        let x_column_idx = self
            .program
            .add_constant(Constant::String(spec.x_column.clone().unwrap_or_default()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(x_column_idx)),
        ));

        for y_column in &spec.y_columns {
            let idx = self
                .program
                .add_constant(Constant::String(y_column.clone()));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(idx)),
            ));
        }

        let count_idx = self
            .program
            .add_constant(Constant::Int(3 + spec.y_columns.len() as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count_idx)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::FStringContentChart)),
        ));
        Ok(())
    }

    fn emit_empty_content_text(&mut self) -> Result<()> {
        let const_idx = self.program.add_constant(Constant::String(String::new()));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
        self.emit_fstring_content_text_call()
    }
}

fn chart_type_name(chart_type: &ChartTypeSpec) -> &'static str {
    match chart_type {
        ChartTypeSpec::Line => "line",
        ChartTypeSpec::Bar => "bar",
        ChartTypeSpec::Scatter => "scatter",
        ChartTypeSpec::Area => "area",
        ChartTypeSpec::Histogram => "histogram",
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
