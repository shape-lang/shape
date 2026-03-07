//! Special builtin operations
//!
//! Handles: Print, ControlFold, and other special-case builtins

use crate::executor::SNAPSHOT_FUTURE_ID;
use crate::executor::VirtualMachine;
use crate::executor::printing::ValueFormatter;
use arrow_array::{
    Array, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array, LargeStringArray,
    StringArray, UInt32Array, UInt64Array,
};
use arrow_schema::DataType;
use shape_ast::interpolation::{FormatAlignment, FormatColor, TableFormatSpec};
use shape_runtime::context::ExecutionContext;
use shape_value::{DataTable, PrintResult, PrintSpan, VMError, ValueWord, heap_value::HeapValue};

const FORMAT_SPEC_FIXED: i64 = 1;
const FORMAT_SPEC_TABLE: i64 = 2;

impl VirtualMachine {
    /// Snapshot: Suspend execution and allow the host to snapshot state.
    ///
    /// This is an explicit suspension point for resumability/time-travel.
    pub(in crate::executor) fn builtin_snapshot(
        &mut self,
        _args: Vec<ValueWord>,
        _ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        Err(VMError::Suspended {
            future_id: SNAPSHOT_FUTURE_ID,
            resume_ip: self.ip,
        })
    }

    /// Print: Print values to output using VM-native formatting
    ///
    /// Output handling:
    /// - If ExecutionContext is provided, uses its OutputAdapter (proper transport via wire)
    /// - Falls back to VM's output buffer (for tests) or stdout
    ///
    /// For type-annotated values, calls the compiled meta format function if available.
    pub(in crate::executor) fn builtin_print(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        let mut rendered = String::new();

        // Render Content HTML for Content values (for host adapters like the playground)
        if ctx.is_some() {
            for nb in &args {
                let (_, html, _) = shape_runtime::wire_conversion::nb_extract_content(nb);
                if let Some(html) = html {
                    if let Some(ref mut c) = ctx {
                        c.output_adapter_mut().print_content_html(html);
                    }
                }
            }
        }

        for (arg_idx, nb) in args.iter().enumerate() {
            if arg_idx > 0 {
                rendered.push(' ');
            }
            let formatted = self.format_nb_with_meta(nb, ctx.as_deref_mut())?;
            rendered.push_str(&formatted);
        }

        if let Some(ctx) = ctx {
            let span = PrintSpan::Literal {
                text: rendered.clone(),
                start: 0,
                end: rendered.len(),
                span_id: "span_0".to_string(),
            };
            let result = PrintResult {
                rendered,
                spans: vec![span],
            };
            let v = ctx.output_adapter_mut().print(result);
            Ok(v)
        } else {
            // Output to VM buffer or stdout
            self.write_output(&rendered);
            Ok(ValueWord::unit())
        }
    }

    /// Format a ValueWord value, unwrapping TypeAnnotatedValue if present
    fn format_nb_with_meta(
        &mut self,
        value: &ValueWord,
        _ctx: Option<&mut ExecutionContext>,
    ) -> Result<String, VMError> {
        use shape_value::heap_value::HeapValue;

        // Content values render via the TerminalRenderer for full ANSI support
        if let Some(node) = value.as_content() {
            use shape_runtime::content_renderer::ContentRenderer;
            use shape_runtime::renderers::terminal::TerminalRenderer;
            return Ok(TerminalRenderer::new().render(node));
        }

        // Non-content values: check for Content trait dispatch
        // If the value type has a Content impl, render it as content first
        if !matches!(
            value.tag(),
            shape_value::value_word::NanTag::None | shape_value::value_word::NanTag::Unit
        ) {
            let content_node = shape_runtime::content_dispatch::render_as_content(value);
            // Only use content dispatch if it produced structured content (Table, KeyValue, Fragment with >1 part)
            // For plain text, fall through to the normal formatting path to preserve Display behavior
            match &content_node {
                shape_value::content::ContentNode::Table(_)
                | shape_value::content::ContentNode::KeyValue(_)
                | shape_value::content::ContentNode::Chart(_)
                | shape_value::content::ContentNode::Code { .. } => {
                    use shape_runtime::content_renderer::ContentRenderer;
                    use shape_runtime::renderers::terminal::TerminalRenderer;
                    return Ok(TerminalRenderer::new().render(&content_node));
                }
                _ => {} // Fall through to normal formatting
            }
        }

        // Unwrap type-annotated wrappers and capture explicit impl selectors.
        let mut preferred_type_name: Option<String> = None;
        let mut selected_impl_name: Option<String> = None;
        let unwrapped = if let Some(HeapValue::TypeAnnotatedValue {
            type_name,
            value: inner,
        }) = value.as_heap_ref()
        {
            if let Some(impl_name) = type_name.strip_prefix("__impl__:") {
                selected_impl_name = Some(impl_name.to_string());
            } else {
                preferred_type_name = Some(type_name.clone());
            }
            inner.as_ref().clone()
        } else {
            value.clone()
        };

        if let Some(rendered) = self.try_format_with_display_impl(
            &unwrapped,
            preferred_type_name.as_deref(),
            selected_impl_name.as_deref(),
        )? {
            return Ok(rendered);
        }

        Ok(self.format_value_default_nb(&unwrapped))
    }

    /// Try trait-based Display formatting for a value.
    ///
    /// Returns:
    /// - `Ok(Some(rendered))` when a display impl is found and returns a string.
    /// - `Ok(None)` when no applicable display impl exists.
    /// - `Err(_)` for invalid named impl selection or non-string display return values.
    fn try_format_with_display_impl(
        &mut self,
        value: &ValueWord,
        preferred_type_name: Option<&str>,
        selected_impl_name: Option<&str>,
    ) -> Result<Option<String>, VMError> {
        use shape_value::heap_value::HeapValue;

        let type_name = preferred_type_name
            .map(|s| s.to_string())
            .or_else(|| match value.as_heap_ref() {
                Some(HeapValue::TypedObject { schema_id, .. }) => self
                    .lookup_schema(*schema_id as u32)
                    .map(|schema| schema.name.clone()),
                _ => None,
            })
            .or_else(|| {
                let tn = value.type_name();
                if tn == "object" || tn == "unknown" {
                    None
                } else {
                    Some(tn.to_string())
                }
            });

        let Some(type_name) = type_name else {
            return Ok(None);
        };

        let resolved_symbol = if let Some(impl_name) = selected_impl_name {
            self.program
                .lookup_trait_method_symbol("Display", &type_name, Some(impl_name), "display")
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "No named Display impl '{}' for type '{}'",
                        impl_name, type_name
                    ))
                })?
        } else if let Some(symbol) = self
            .program
            .lookup_trait_method_symbol("Display", &type_name, None, "display")
        {
            symbol.to_string()
        } else {
            let named_impls = self
                .program
                .named_trait_impls_for_method("Display", &type_name, "display");
            return match named_impls.len() {
                0 => Ok(None),
                1 => Err(VMError::RuntimeError(format!(
                    "No default Display impl for type '{}'. Use `value using {}`.",
                    type_name, named_impls[0]
                ))),
                _ => Err(VMError::RuntimeError(format!(
                    "Ambiguous Display impl for type '{}': {}. Use `value using <ImplName>`.",
                    type_name,
                    named_impls.join(", ")
                ))),
            };
        };

        let Some(&func_id) = self.function_name_index.get(&resolved_symbol) else {
            return Err(VMError::RuntimeError(format!(
                "Display dispatch target '{}' is not a compiled function",
                resolved_symbol
            )));
        };

        let func_nb = ValueWord::from_function(func_id);
        let rendered_nb = self.call_value_immediate_nb(&func_nb, &[value.clone()], None)?;
        let rendered = rendered_nb.as_str().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Display impl '{}' returned non-string value '{}'",
                resolved_symbol,
                rendered_nb.type_name()
            ))
        })?;
        Ok(Some(rendered.to_string()))
    }

    /// Format a ValueWord value using default formatting (ValueWord-native path)
    pub(in crate::executor) fn format_value_default_nb(&self, value: &ValueWord) -> String {
        let formatter = ValueFormatter::new(&self.program.type_schema_registry);
        formatter.format_nb(value)
    }

    /// FormatValueWithMeta builtin: Format a value respecting meta formatting
    ///
    /// This is used by string interpolation to apply custom formatters for
    /// TypeAnnotatedValues while preserving the simple toString behavior for
    /// regular values.
    pub(in crate::executor) fn builtin_format_with_meta(
        &mut self,
        args: Vec<ValueWord>,
        ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        use std::sync::Arc;

        if args.len() != 1 {
            return Err(VMError::RuntimeError(
                "formatValueWithMeta() requires exactly 1 argument".to_string(),
            ));
        }

        let formatted = self.format_nb_with_meta(&args[0], ctx)?;
        Ok(ValueWord::from_string(Arc::new(formatted)))
    }

    /// FormatValueWithSpec builtin: format a value with a typed interpolation spec.
    ///
    /// Encoded arg shapes:
    /// - Fixed: [value, 1, precision]
    /// - Table: [value, 2, max_rows, align, precision, color, border]
    pub(in crate::executor) fn builtin_format_with_spec(
        &mut self,
        args: Vec<ValueWord>,
        ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        use std::sync::Arc;

        if args.len() < 2 {
            return Err(VMError::RuntimeError(
                "formatValueWithSpec() requires at least 2 arguments".to_string(),
            ));
        }

        let spec_tag = as_i64_arg(&args[1], "format spec tag")?;
        let value = &args[0];

        let rendered = match spec_tag {
            FORMAT_SPEC_FIXED => {
                if args.len() != 3 {
                    return Err(VMError::RuntimeError(
                        "fixed format requires exactly 3 args: [value, tag, precision]".to_string(),
                    ));
                }
                let precision = as_i64_arg(&args[2], "fixed precision")?;
                if !(0..=255).contains(&precision) {
                    return Err(VMError::RuntimeError(format!(
                        "fixed precision must be in range 0..=255, got {}",
                        precision
                    )));
                }
                format_fixed_value(value, precision as usize)?
            }
            FORMAT_SPEC_TABLE => {
                if args.len() != 7 {
                    return Err(VMError::RuntimeError(
                        "table format requires exactly 7 args: [value, tag, max_rows, align, precision, color, border]".to_string(),
                    ));
                }
                let spec = decode_table_format_spec(&args[2..])?;
                let table = extract_table_ref(value).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "table format requires a table value, got '{}'",
                        value.type_name()
                    ))
                })?;
                render_table_with_spec(table, &spec)
            }
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Unknown interpolation format spec tag '{}'",
                    spec_tag
                )));
            }
        };

        // Spec formatting operates on already-compiled interpolation expressions,
        // so no additional Display dispatch is needed here.
        let _ = ctx;
        Ok(ValueWord::from_string(Arc::new(rendered)))
    }

    /// Reflect: Return type schema info as a TypedObject
    ///
    /// `reflect("TypeName")` → { name, fields: [{ name, type, annotations: [{ name, args }] }] }
    pub(in crate::executor) fn builtin_reflect(&mut self) -> Result<(), VMError> {
        use std::sync::Arc;

        let nb_args = self.pop_builtin_args()?;
        if nb_args.len() != 1 {
            return Err(VMError::RuntimeError(
                "reflect() requires exactly 1 argument (type name)".to_string(),
            ));
        }
        let type_name = nb_args[0]
            .as_str()
            .ok_or_else(|| {
                VMError::RuntimeError("reflect() argument must be a string".to_string())
            })?
            .to_string();

        let schema = self.program.type_schema_registry.get(&type_name);

        let schema = match schema {
            Some(s) => s,
            None => {
                return Err(VMError::RuntimeError(format!(
                    "reflect(): unknown type '{}'",
                    type_name
                )));
            }
        };

        // Build fields array — collect data first to avoid borrow issues
        let field_data: Vec<_> = schema
            .fields
            .iter()
            .map(|field| {
                let ann_data: Vec<_> = field
                    .annotations
                    .iter()
                    .map(|ann| {
                        let args_arr: Vec<ValueWord> = ann
                            .args
                            .iter()
                            .map(|a| ValueWord::from_string(Arc::new(a.clone())))
                            .collect();
                        (ann.name.clone(), args_arr)
                    })
                    .collect();
                (
                    field.name.clone(),
                    format!("{:?}", field.field_type),
                    ann_data,
                )
            })
            .collect();

        let ann_schema = self.builtin_schemas.reflect_annotation;
        let field_schema = self.builtin_schemas.reflect_field;
        let result_schema = self.builtin_schemas.reflect_result;

        let mut fields: Vec<ValueWord> = Vec::with_capacity(field_data.len());
        for (name, field_type, ann_data) in field_data {
            let mut ann_values: Vec<ValueWord> = Vec::with_capacity(ann_data.len());
            for (ann_name, args_arr) in ann_data {
                ann_values.push(ValueWord::from_heap_value(HeapValue::TypedObject {
                    schema_id: ann_schema as u64,
                    slots: vec![
                        shape_value::ValueSlot::from_heap(HeapValue::String(Arc::new(ann_name))),
                        shape_value::ValueSlot::from_heap(HeapValue::Array(Arc::new(args_arr))),
                    ]
                    .into_boxed_slice(),
                    heap_mask: 0b11,
                }));
            }
            fields.push(ValueWord::from_heap_value(HeapValue::TypedObject {
                schema_id: field_schema as u64,
                slots: vec![
                    shape_value::ValueSlot::from_heap(HeapValue::String(Arc::new(name))),
                    shape_value::ValueSlot::from_heap(HeapValue::String(Arc::new(field_type))),
                    shape_value::ValueSlot::from_heap(HeapValue::Array(Arc::new(ann_values))),
                ]
                .into_boxed_slice(),
                heap_mask: 0b111,
            }));
        }

        let result = HeapValue::TypedObject {
            schema_id: result_schema as u64,
            slots: vec![
                shape_value::ValueSlot::from_heap(HeapValue::String(Arc::new(type_name))),
                shape_value::ValueSlot::from_heap(HeapValue::Array(Arc::new(fields))),
            ]
            .into_boxed_slice(),
            heap_mask: 0b11,
        };
        self.push_vw(ValueWord::from_heap_value(result))?;
        Ok(())
    }

    /// Format: concatenate string representations of all arguments
    pub(in crate::executor) fn builtin_format(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use std::sync::Arc;
        let result: Vec<String> = args
            .iter()
            .map(|nb| self.format_value_default_nb(nb))
            .collect();
        Ok(ValueWord::from_string(Arc::new(result.join(""))))
    }

    // builtin_throw removed: Shape uses Result types, not exceptions

    /// Exit: Terminate the process with an optional exit code
    pub(in crate::executor) fn builtin_exit(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        let code = if !args.is_empty() {
            args[0]
                .as_i64()
                .map(|i| i as i32)
                .or_else(|| args[0].as_number_coerce().map(|n| n as i32))
                .unwrap_or(0)
        } else {
            0
        };
        std::process::exit(code);
    }

    /// MakeContentText: wrap a string value as ContentNode::plain(text)
    ///
    /// Args: [string_value]
    pub(in crate::executor) fn builtin_make_content_text(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use shape_value::content::ContentNode;

        if args.len() != 1 {
            return Err(VMError::RuntimeError(
                "MakeContentText requires exactly 1 argument".to_string(),
            ));
        }

        let text = if let Some(s) = args[0].as_str() {
            s.to_string()
        } else {
            format!("{}", args[0])
        };

        Ok(ValueWord::from_content(ContentNode::plain(text)))
    }

    /// MakeContentFragment: collect N ContentNodes into a Fragment
    ///
    /// Args: [node_1, node_2, ..., node_N, count]
    /// The count is the last arg, nodes precede it.
    pub(in crate::executor) fn builtin_make_content_fragment(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use shape_value::content::ContentNode;

        if args.is_empty() {
            return Ok(ValueWord::from_content(ContentNode::Fragment(vec![])));
        }

        // The last arg is the count (pushed by the compiler), the preceding args are the nodes
        let count = args
            .last()
            .and_then(|nb| {
                nb.as_i64()
                    .or_else(|| nb.as_number_coerce().map(|n| n as i64))
            })
            .unwrap_or(args.len() as i64) as usize;

        let mut parts = Vec::with_capacity(count);
        for arg in args.iter().take(count) {
            if let Some(node) = arg.as_content() {
                parts.push(node.clone());
            } else {
                // Fallback: convert to string and wrap as plain text
                parts.push(ContentNode::plain(format!("{}", arg)));
            }
        }

        Ok(ValueWord::from_content(ContentNode::Fragment(parts)))
    }

    /// ApplyContentStyle: apply style parameters to a ContentNode
    ///
    /// Args: [content_node, fg_color, bg_color, bold, italic, underline, dim]
    /// Color encoding: -1 = none, 0-7 = named colors, 256+ = RGB
    pub(in crate::executor) fn builtin_apply_content_style(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use shape_value::content::ContentNode;

        if args.len() != 7 {
            return Err(VMError::RuntimeError(format!(
                "ApplyContentStyle requires 7 args [node, fg, bg, bold, italic, underline, dim], got {}",
                args.len()
            )));
        }

        let node = if let Some(n) = args[0].as_content() {
            n.clone()
        } else {
            ContentNode::plain(format!("{}", args[0]))
        };

        let fg = decode_content_color(&args[1])?;
        let bg = decode_content_color(&args[2])?;
        let bold = args[3].as_bool().unwrap_or(false);
        let italic = args[4].as_bool().unwrap_or(false);
        let underline = args[5].as_bool().unwrap_or(false);
        let dim = args[6].as_bool().unwrap_or(false);

        let mut styled = node;
        if let Some(color) = fg {
            styled = styled.with_fg(color);
        }
        if let Some(color) = bg {
            styled = styled.with_bg(color);
        }
        if bold {
            styled = styled.with_bold();
        }
        if italic {
            styled = styled.with_italic();
        }
        if underline {
            styled = styled.with_underline();
        }
        if dim {
            styled = styled.with_dim();
        }

        Ok(ValueWord::from_content(styled))
    }

    /// MakeContentChartFromValue: create a chart ContentNode from a table/array value
    ///
    /// Args: [value, chart_type_str, x_column_str, y_count, y_col1, y_col2, ...]
    pub(in crate::executor) fn builtin_make_content_chart_from_value(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use shape_value::content::{ChartChannel, ChartSpec, ChartType, ContentNode};

        if args.len() < 4 {
            return Err(VMError::RuntimeError(
                "MakeContentChartFromValue requires at least 4 args".to_string(),
            ));
        }

        let value = &args[0];
        let chart_type_str = args[1]
            .as_str()
            .ok_or_else(|| VMError::RuntimeError("chart type must be a string".to_string()))?;
        let x_column = args[2].as_str().unwrap_or("").to_string();
        let y_count = args[3]
            .as_i64()
            .or_else(|| args[3].as_number_coerce().map(|n| n as i64))
            .unwrap_or(0) as usize;

        let mut y_columns: Vec<String> = Vec::with_capacity(y_count);
        for i in 0..y_count {
            if let Some(s) = args.get(4 + i).and_then(|a| a.as_str()) {
                y_columns.push(s.to_string());
            }
        }

        let chart_type = match chart_type_str.to_lowercase().as_str() {
            "line" => ChartType::Line,
            "bar" => ChartType::Bar,
            "scatter" => ChartType::Scatter,
            "area" => ChartType::Area,
            "histogram" => ChartType::Histogram,
            _ => ChartType::Line,
        };

        // Handle DataTable / TypedTable (Table<T>) directly via columnar access
        let dt_ref = value.as_datatable().or_else(|| {
            value.as_typed_table().map(|(_, t)| t)
        });
        if let Some(dt) = dt_ref {
            return self.chart_from_datatable(dt, chart_type, x_column, y_columns);
        }

        // Extract rows from the value (should be an array of typed objects)
        let rows = value
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError(
                    "chart format spec requires an array value (e.g., Table<T>)".to_string(),
                )
            })?
            .to_generic();

        if rows.is_empty() {
            return Ok(ValueWord::from_content(ContentNode::Chart(ChartSpec {
                chart_type,
                channels: vec![],
                x_categories: None,
                title: None,
                x_label: if x_column.is_empty() {
                    None
                } else {
                    Some(x_column)
                },
                y_label: None,
                width: None,
                height: None,
                echarts_options: None,
                interactive: true,
            })));
        }

        // Get field names from the first row's schema, or fall back to hashmap keys
        let field_map = self.extract_row_field_names(&rows[0])?;

        // If no x/y columns specified, try to auto-detect
        let x_col = if x_column.is_empty() {
            // Use first string/date column as x, or first column
            field_map
                .iter()
                .find(|(_, v)| v.as_str().is_some())
                .or_else(|| field_map.iter().next())
                .map(|(k, _)| k.clone())
                .unwrap_or_default()
        } else {
            x_column
        };

        let y_cols: Vec<String> = if y_columns.is_empty() {
            // Use all numeric columns except x as y series
            field_map
                .iter()
                .filter(|(k, v)| *k != &x_col && v.as_number_coerce().is_some())
                .map(|(k, _)| k.clone())
                .collect()
        } else {
            y_columns
        };

        // Build channels: x channel + one y channel per y column
        let mut channels: Vec<ChartChannel> = Vec::new();

        // Extract x values from rows
        let mut x_values: Vec<f64> = Vec::with_capacity(rows.len());
        for (row_idx, row) in rows.iter().enumerate() {
            let row_map = self.extract_row_fields(row);
            let x_val = row_map
                .as_ref()
                .and_then(|m| m.get(&x_col))
                .and_then(|v| v.as_number_coerce())
                .unwrap_or(row_idx as f64);
            x_values.push(x_val);
        }
        channels.push(ChartChannel {
            name: "x".to_string(),
            label: x_col.clone(),
            values: x_values,
            color: None,
        });

        for y_col in &y_cols {
            let mut y_values: Vec<f64> = Vec::with_capacity(rows.len());
            for row in rows.iter() {
                let row_map = self.extract_row_fields(row);
                let y_val = row_map
                    .as_ref()
                    .and_then(|m| m.get(y_col.as_str()))
                    .and_then(|v| v.as_number_coerce())
                    .unwrap_or(0.0);
                y_values.push(y_val);
            }
            channels.push(ChartChannel {
                name: "y".to_string(),
                label: y_col.clone(),
                values: y_values,
                color: None,
            });
        }

        Ok(ValueWord::from_content(ContentNode::Chart(ChartSpec {
            chart_type,
            channels,
            x_categories: None,
            title: None,
            x_label: Some(x_col),
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        })))
    }

    /// Build a chart directly from a DataTable's columnar data.
    fn chart_from_datatable(
        &self,
        dt: &std::sync::Arc<DataTable>,
        chart_type: shape_value::content::ChartType,
        x_column: String,
        y_columns: Vec<String>,
    ) -> Result<ValueWord, VMError> {
        use shape_value::content::{ChartChannel, ChartSpec, ContentNode};

        let col_names = dt.column_names();
        let num_rows = dt.row_count();

        let x_col = if x_column.is_empty() {
            col_names.first().cloned().unwrap_or_default()
        } else {
            x_column
        };

        let y_cols: Vec<String> = if y_columns.is_empty() {
            col_names
                .iter()
                .filter(|n| *n != &x_col)
                .cloned()
                .collect()
        } else {
            y_columns
        };

        // Build channels
        let mut channels: Vec<ChartChannel> = Vec::new();

        // x channel
        let x_vals: Vec<f64> = Self::read_column_as_f64(dt, &x_col, num_rows);
        channels.push(ChartChannel {
            name: "x".to_string(),
            label: x_col.clone(),
            values: x_vals,
            color: None,
        });

        // y channels
        for y_col in &y_cols {
            let y_vals = Self::read_column_as_f64(dt, y_col, num_rows);
            channels.push(ChartChannel {
                name: "y".to_string(),
                label: y_col.clone(),
                values: y_vals,
                color: None,
            });
        }

        Ok(ValueWord::from_content(ContentNode::Chart(ChartSpec {
            chart_type,
            channels,
            x_categories: None,
            title: None,
            x_label: Some(x_col),
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        })))
    }

    /// Read a DataTable column as f64 values, coercing int columns.
    fn read_column_as_f64(dt: &DataTable, col: &str, num_rows: usize) -> Vec<f64> {
        if let Some(arr) = dt.get_f64_column(col) {
            (0..num_rows).map(|i| arr.value(i)).collect()
        } else if let Some(arr) = dt.get_i64_column(col) {
            (0..num_rows).map(|i| arr.value(i) as f64).collect()
        } else {
            (0..num_rows).map(|i| i as f64).collect()
        }
    }

    /// Extract field name→value map from a typed object row.
    fn extract_row_field_names(
        &self,
        row: &ValueWord,
    ) -> Result<std::collections::HashMap<String, ValueWord>, VMError> {
        use crate::executor::objects::object_creation::read_slot_nb;

        if let Some((schema_id, slots, heap_mask)) = row.as_typed_object() {
            let sid = schema_id as u32;
            if let Some(schema) = self.lookup_schema(sid) {
                let mut map =
                    std::collections::HashMap::with_capacity(schema.fields.len());
                for field_def in &schema.fields {
                    let val = read_slot_nb(
                        slots,
                        field_def.index as usize,
                        heap_mask,
                        Some(&field_def.field_type),
                    );
                    map.insert(field_def.name.clone(), val);
                }
                return Ok(map);
            }
        }
        // Fall back to runtime schema
        shape_runtime::type_schema::typed_object_to_hashmap_nb(row)
            .ok_or_else(|| VMError::RuntimeError("Cannot extract fields from row".to_string()))
    }

    /// Extract fields from a row (uses VM schema or runtime fallback).
    fn extract_row_fields(
        &self,
        row: &ValueWord,
    ) -> Option<std::collections::HashMap<String, ValueWord>> {
        use crate::executor::objects::object_creation::read_slot_nb;

        if let Some((schema_id, slots, heap_mask)) = row.as_typed_object() {
            let sid = schema_id as u32;
            if let Some(schema) = self.lookup_schema(sid) {
                let mut map =
                    std::collections::HashMap::with_capacity(schema.fields.len());
                for field_def in &schema.fields {
                    let val = read_slot_nb(
                        slots,
                        field_def.index as usize,
                        heap_mask,
                        Some(&field_def.field_type),
                    );
                    map.insert(field_def.name.clone(), val);
                }
                return Some(map);
            }
        }
        shape_runtime::type_schema::typed_object_to_hashmap_nb(row)
    }

    /// ControlFold: Fold operation with accumulator
    pub(in crate::executor) fn builtin_control_fold(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 3 {
            return Err(VMError::RuntimeError(
                "fold() requires exactly 3 arguments (array, initial, reducer)".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("fold() first argument must be an array".to_string())
            })?
            .to_generic();

        let mut accumulator = args[1].clone();

        for nb in array.iter() {
            accumulator = self.call_value_immediate_nb(
                &args[2],
                &[accumulator, nb.clone()],
                ctx.as_deref_mut(),
            )?;
        }

        Ok(accumulator)
    }

    /// MakeTableFromRows: build a TypedTable from inline row values.
    ///
    /// Args: [schema_id, row_count, field_count, val1, val2, ..., valN]
    /// where N = row_count * field_count.
    /// Values are in row-major order (all fields of row 0, then row 1, etc.).
    pub(in crate::executor) fn builtin_make_table_from_rows(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        use arrow_array::RecordBatch;
        use arrow_schema::{Field, Schema};
        use shape_value::datatable::DataTableBuilder;
        use std::sync::Arc;

        if args.len() < 3 {
            return Err(VMError::RuntimeError(
                "MakeTableFromRows requires at least 3 args (schema_id, row_count, field_count)"
                    .to_string(),
            ));
        }

        let schema_id = args[0]
            .as_i64()
            .ok_or_else(|| VMError::RuntimeError("schema_id must be int".to_string()))?
            as u32;
        let row_count = args[1]
            .as_i64()
            .ok_or_else(|| VMError::RuntimeError("row_count must be int".to_string()))?
            as usize;
        let field_count = args[2]
            .as_i64()
            .ok_or_else(|| VMError::RuntimeError("field_count must be int".to_string()))?
            as usize;

        let expected_vals = row_count * field_count;
        if args.len() != 3 + expected_vals {
            return Err(VMError::RuntimeError(format!(
                "MakeTableFromRows: expected {} values ({} rows × {} fields), got {}",
                expected_vals,
                row_count,
                field_count,
                args.len() - 3
            )));
        }

        // Look up the schema to get field names and types
        let schema = self
            .program
            .type_schema_registry
            .get_by_id(schema_id)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "MakeTableFromRows: unknown schema ID {}",
                    schema_id
                ))
            })?;

        if schema.fields.len() != field_count {
            return Err(VMError::RuntimeError(format!(
                "MakeTableFromRows: schema has {} fields but field_count is {}",
                schema.fields.len(),
                field_count
            )));
        }

        let type_name = schema.name.clone();
        let values = &args[3..];

        // Build Arrow columns from the row-major values
        let mut arrow_fields = Vec::with_capacity(field_count);
        let mut columns: Vec<arrow_array::ArrayRef> = Vec::with_capacity(field_count);

        for col_idx in 0..field_count {
            let field_def = &schema.fields[col_idx];
            let field_name = &field_def.name;

            // Collect all values for this column across rows
            let col_values: Vec<&ValueWord> = (0..row_count)
                .map(|row_idx| &values[row_idx * field_count + col_idx])
                .collect();

            // Determine Arrow type from schema field type
            use shape_runtime::type_schema::FieldType;
            match &field_def.field_type {
                FieldType::I64 => {
                    let arr: Vec<i64> = col_values
                        .iter()
                        .map(|v| v.as_i64().unwrap_or(0))
                        .collect();
                    arrow_fields
                        .push(Field::new(field_name.clone(), DataType::Int64, false));
                    columns.push(Arc::new(Int64Array::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::F64 => {
                    let arr: Vec<f64> = col_values
                        .iter()
                        .map(|v| {
                            v.as_f64()
                                .or_else(|| v.as_i64().map(|i| i as f64))
                                .unwrap_or(0.0)
                        })
                        .collect();
                    arrow_fields
                        .push(Field::new(field_name.clone(), DataType::Float64, false));
                    columns.push(Arc::new(Float64Array::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::Bool => {
                    let arr: Vec<bool> = col_values
                        .iter()
                        .map(|v| v.as_bool().unwrap_or(false))
                        .collect();
                    arrow_fields
                        .push(Field::new(field_name.clone(), DataType::Boolean, false));
                    columns.push(Arc::new(BooleanArray::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::Decimal => {
                    // Decimal stored as f64 in table columns
                    let arr: Vec<f64> = col_values
                        .iter()
                        .map(|v| {
                            v.as_f64()
                                .or_else(|| v.as_i64().map(|i| i as f64))
                                .unwrap_or(0.0)
                        })
                        .collect();
                    arrow_fields
                        .push(Field::new(field_name.clone(), DataType::Float64, false));
                    columns.push(Arc::new(Float64Array::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::Timestamp => {
                    let arr: Vec<i64> = col_values
                        .iter()
                        .map(|v| v.as_i64().unwrap_or(0))
                        .collect();
                    arrow_fields
                        .push(Field::new(field_name.clone(), DataType::Int64, false));
                    columns.push(Arc::new(Int64Array::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::I8 | FieldType::U8 | FieldType::I16 | FieldType::U16
                | FieldType::I32 | FieldType::U32 | FieldType::U64 => {
                    // Width-typed integers stored as i64
                    let arr: Vec<i64> = col_values
                        .iter()
                        .map(|v| v.as_i64().unwrap_or(0))
                        .collect();
                    arrow_fields
                        .push(Field::new(field_name.clone(), DataType::Int64, false));
                    columns.push(Arc::new(Int64Array::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::String | FieldType::Object(_) | FieldType::Any | FieldType::Array(_) => {
                    let arr: Vec<String> = col_values
                        .iter()
                        .map(|v| {
                            if let Some(s) = v.as_str() {
                                s.to_string()
                            } else if let Some(i) = v.as_i64() {
                                i.to_string()
                            } else if let Some(f) = v.as_f64() {
                                f.to_string()
                            } else {
                                String::new()
                            }
                        })
                        .collect();
                    arrow_fields
                        .push(Field::new(field_name.clone(), DataType::Utf8, false));
                    columns.push(Arc::new(StringArray::from(arr)) as arrow_array::ArrayRef);
                }
            }
        }

        let arrow_schema = Arc::new(Schema::new(arrow_fields));
        let batch = RecordBatch::try_new(arrow_schema, columns).map_err(|e| {
            VMError::RuntimeError(format!("MakeTableFromRows: failed to create RecordBatch: {}", e))
        })?;

        let dt = DataTable::with_type_name(batch, type_name)
            .with_schema_id(schema_id);
        let table = Arc::new(dt);

        Ok(ValueWord::from_heap_value(HeapValue::TypedTable {
            schema_id: schema_id as u64,
            table,
        }))
    }
}

// render_content_ansi removed — ContentNode rendering now delegated to
// shape_runtime::renderers::terminal::TerminalRenderer via the ContentRenderer trait.

/// Decode a color spec from the integer encoding used by the compiler.
/// -1 = none, 0-7 = named colors, 256+ = RGB(r*65536 + g*256 + b)
fn decode_content_color(arg: &ValueWord) -> Result<Option<shape_value::content::Color>, VMError> {
    use shape_value::content::{Color, NamedColor};

    let raw = arg
        .as_i64()
        .or_else(|| arg.as_number_coerce().map(|n| n as i64))
        .unwrap_or(-1);

    match raw {
        -1 => Ok(None),
        0 => Ok(Some(Color::Named(NamedColor::Red))),
        1 => Ok(Some(Color::Named(NamedColor::Green))),
        2 => Ok(Some(Color::Named(NamedColor::Blue))),
        3 => Ok(Some(Color::Named(NamedColor::Yellow))),
        4 => Ok(Some(Color::Named(NamedColor::Magenta))),
        5 => Ok(Some(Color::Named(NamedColor::Cyan))),
        6 => Ok(Some(Color::Named(NamedColor::White))),
        7 => Ok(Some(Color::Named(NamedColor::Default))),
        n if n >= 256 => {
            let val = n - 256;
            let r = ((val >> 16) & 0xFF) as u8;
            let g = ((val >> 8) & 0xFF) as u8;
            let b = (val & 0xFF) as u8;
            Ok(Some(Color::Rgb(r, g, b)))
        }
        _ => Err(VMError::RuntimeError(format!(
            "Invalid content color encoding: {}",
            raw
        ))),
    }
}

fn as_i64_arg(arg: &ValueWord, label: &str) -> Result<i64, VMError> {
    arg.as_i64()
        .or_else(|| arg.as_number_coerce().map(|n| n as i64))
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "{} must be numeric, got '{}'",
                label,
                arg.type_name()
            ))
        })
}

fn decode_table_format_spec(args: &[ValueWord]) -> Result<TableFormatSpec, VMError> {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "internal table format decode requires 5 args".to_string(),
        ));
    }

    let max_rows_raw = as_i64_arg(&args[0], "table max_rows")?;
    let align_raw = as_i64_arg(&args[1], "table align")?;
    let precision_raw = as_i64_arg(&args[2], "table precision")?;
    let color_raw = as_i64_arg(&args[3], "table color")?;
    let border = args[4].as_bool().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "table border must be bool, got '{}'",
            args[4].type_name()
        ))
    })?;

    let max_rows = if max_rows_raw < 0 {
        None
    } else {
        Some(max_rows_raw as usize)
    };
    let align = match align_raw {
        -1 => None,
        0 => Some(FormatAlignment::Left),
        1 => Some(FormatAlignment::Center),
        2 => Some(FormatAlignment::Right),
        _ => {
            return Err(VMError::RuntimeError(format!(
                "invalid table align enum '{}'",
                align_raw
            )));
        }
    };
    let precision = if precision_raw < 0 {
        None
    } else if precision_raw <= 255 {
        Some(precision_raw as u8)
    } else {
        return Err(VMError::RuntimeError(format!(
            "table precision must be in range 0..=255, got {}",
            precision_raw
        )));
    };
    let color = match color_raw {
        -1 => None,
        0 => Some(FormatColor::Default),
        1 => Some(FormatColor::Red),
        2 => Some(FormatColor::Green),
        3 => Some(FormatColor::Yellow),
        4 => Some(FormatColor::Blue),
        5 => Some(FormatColor::Magenta),
        6 => Some(FormatColor::Cyan),
        7 => Some(FormatColor::White),
        _ => {
            return Err(VMError::RuntimeError(format!(
                "invalid table color enum '{}'",
                color_raw
            )));
        }
    };

    Ok(TableFormatSpec {
        max_rows,
        align,
        precision,
        color,
        border,
    })
}

fn extract_table_ref(value: &ValueWord) -> Option<&DataTable> {
    if let Some(dt) = value.as_datatable() {
        return Some(dt.as_ref());
    }
    if let Some((_schema_id, table)) = value.as_typed_table() {
        return Some(table.as_ref());
    }
    if let Some((_schema_id, table, _index_col)) = value.as_indexed_table() {
        return Some(table.as_ref());
    }
    None
}

fn format_fixed_value(value: &ValueWord, precision: usize) -> Result<String, VMError> {
    if let Some(decimal) = value.as_decimal() {
        let number = decimal.to_string().parse::<f64>().map_err(|_| {
            VMError::RuntimeError("failed to convert decimal to float for fixed format".to_string())
        })?;
        return Ok(format!("{:.*}", precision, number));
    }
    if let Some(number) = value.as_number_coerce() {
        return Ok(format!("{:.*}", precision, number));
    }
    Err(VMError::RuntimeError(format!(
        "fixed format requires numeric value, got '{}'",
        value.type_name()
    )))
}

fn render_table_with_spec(table: &DataTable, spec: &TableFormatSpec) -> String {
    let col_count = table.column_count();
    let headers = table.column_names();
    let row_count = table.row_count();
    let shown_rows = spec.max_rows.unwrap_or(row_count).min(row_count);

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(shown_rows);
    for row_idx in 0..shown_rows {
        let mut row_cells = Vec::with_capacity(col_count);
        for col_idx in 0..col_count {
            let col = table.inner().column(col_idx);
            row_cells.push(format_array_cell(col.as_ref(), row_idx, spec.precision));
        }
        rows.push(row_cells);
    }

    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.chars().count());
        }
    }

    let overflow = row_count.saturating_sub(shown_rows);
    if overflow > 0 && !widths.is_empty() {
        widths[0] = widths[0].max(format!("... {} more rows", overflow).chars().count());
    }

    let aligns: Vec<FormatAlignment> = (0..col_count)
        .map(|col_idx| {
            spec.align.unwrap_or_else(|| {
                default_alignment_for_datatype(table.inner().column(col_idx).data_type())
            })
        })
        .collect();

    let mut out = String::new();
    if spec.border {
        append_border_line(&mut out, &widths);
    }
    append_row_line(
        &mut out,
        &headers,
        &widths,
        &aligns,
        spec.border,
        spec.color,
    );
    if spec.border {
        append_border_line(&mut out, &widths);
    }
    for row in &rows {
        append_row_line(&mut out, row, &widths, &aligns, spec.border, spec.color);
    }
    if overflow > 0 {
        let mut extra = vec![String::new(); col_count];
        if let Some(first) = extra.first_mut() {
            *first = format!("... {} more rows", overflow);
        }
        append_row_line(&mut out, &extra, &widths, &aligns, spec.border, spec.color);
    }
    if spec.border {
        append_border_line(&mut out, &widths);
    }

    out.trim_end_matches('\n').to_string()
}

fn format_array_cell(array: &dyn Array, row_idx: usize, precision: Option<u8>) -> String {
    if array.is_null(row_idx) {
        return "None".to_string();
    }

    if let Some(col) = array.as_any().downcast_ref::<Float64Array>() {
        return format_number(col.value(row_idx), precision);
    }
    if let Some(col) = array.as_any().downcast_ref::<Float32Array>() {
        return format_number(col.value(row_idx) as f64, precision);
    }
    if let Some(col) = array.as_any().downcast_ref::<Int64Array>() {
        return col.value(row_idx).to_string();
    }
    if let Some(col) = array.as_any().downcast_ref::<Int32Array>() {
        return col.value(row_idx).to_string();
    }
    if let Some(col) = array.as_any().downcast_ref::<UInt64Array>() {
        return col.value(row_idx).to_string();
    }
    if let Some(col) = array.as_any().downcast_ref::<UInt32Array>() {
        return col.value(row_idx).to_string();
    }
    if let Some(col) = array.as_any().downcast_ref::<BooleanArray>() {
        return col.value(row_idx).to_string();
    }
    if let Some(col) = array.as_any().downcast_ref::<StringArray>() {
        return col.value(row_idx).to_string();
    }
    if let Some(col) = array.as_any().downcast_ref::<LargeStringArray>() {
        return col.value(row_idx).to_string();
    }

    "<unsupported>".to_string()
}

fn format_number(value: f64, precision: Option<u8>) -> String {
    if let Some(p) = precision {
        return format!("{:.*}", p as usize, value);
    }
    if value.fract() == 0.0 {
        return format!("{}", value as i64);
    }
    format!("{}", value)
}

fn default_alignment_for_datatype(dt: &DataType) -> FormatAlignment {
    match dt {
        DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Decimal128(_, _)
        | DataType::Decimal256(_, _) => FormatAlignment::Right,
        DataType::Boolean => FormatAlignment::Center,
        _ => FormatAlignment::Left,
    }
}

fn append_border_line(out: &mut String, widths: &[usize]) {
    out.push('+');
    for width in widths {
        out.push_str(&"-".repeat(*width + 2));
        out.push('+');
    }
    out.push('\n');
}

fn append_row_line<T: AsRef<str>>(
    out: &mut String,
    cells: &[T],
    widths: &[usize],
    aligns: &[FormatAlignment],
    border: bool,
    color: Option<FormatColor>,
) {
    if border {
        out.push('|');
    }

    for col_idx in 0..widths.len() {
        let raw = cells.get(col_idx).map_or("", |v| v.as_ref());
        let align = aligns
            .get(col_idx)
            .copied()
            .unwrap_or(FormatAlignment::Left);
        let padded = pad_cell(raw, widths[col_idx], align);
        let colored = apply_color(&padded, color);

        if border {
            out.push(' ');
            out.push_str(&colored);
            out.push(' ');
            out.push('|');
        } else {
            if col_idx > 0 {
                out.push_str("  ");
            }
            out.push_str(&colored);
        }
    }

    out.push('\n');
}

fn pad_cell(s: &str, width: usize, align: FormatAlignment) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }
    let pad = width - len;
    match align {
        FormatAlignment::Left => format!("{s}{}", " ".repeat(pad)),
        FormatAlignment::Right => format!("{}{s}", " ".repeat(pad)),
        FormatAlignment::Center => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
        }
    }
}

fn apply_color(s: &str, color: Option<FormatColor>) -> String {
    let code = match color {
        None | Some(FormatColor::Default) => return s.to_string(),
        Some(FormatColor::Red) => "31",
        Some(FormatColor::Green) => "32",
        Some(FormatColor::Yellow) => "33",
        Some(FormatColor::Blue) => "34",
        Some(FormatColor::Magenta) => "35",
        Some(FormatColor::Cyan) => "36",
        Some(FormatColor::White) => "37",
    };
    format!("\x1b[{code}m{s}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::RecordBatch;
    use arrow_schema::{Field, Schema};
    use std::sync::Arc;

    fn make_table() -> DataTable {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("price", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["A", "B"])) as arrow_array::ArrayRef,
                Arc::new(Float64Array::from(vec![1.234, 5.0])) as arrow_array::ArrayRef,
            ],
        )
        .expect("record batch");
        DataTable::new(batch)
    }

    #[test]
    fn test_render_table_with_spec_precision_and_overflow() {
        let table = make_table();
        let spec = TableFormatSpec {
            max_rows: Some(1),
            align: Some(FormatAlignment::Right),
            precision: Some(2),
            color: None,
            border: false,
        };
        let rendered = render_table_with_spec(&table, &spec);
        assert!(
            rendered.contains("1.23"),
            "expected fixed precision cell, got: {}",
            rendered
        );
        assert!(
            rendered.contains("... 1 more rows"),
            "expected overflow marker, got: {}",
            rendered
        );
    }

    #[test]
    fn test_render_table_with_spec_border_and_color() {
        let table = make_table();
        let spec = TableFormatSpec {
            max_rows: Some(1),
            align: None,
            precision: None,
            color: Some(FormatColor::Green),
            border: true,
        };
        let rendered = render_table_with_spec(&table, &spec);
        assert!(
            rendered.contains('+'),
            "expected bordered rendering, got: {}",
            rendered
        );
        assert!(
            rendered.contains("\x1b[32m"),
            "expected ANSI green color, got: {}",
            rendered
        );
    }

    #[test]
    fn test_format_fixed_value_non_numeric_errors() {
        let err = format_fixed_value(&ValueWord::from_string(Arc::new("x".to_string())), 2)
            .expect_err("expected type error");
        assert!(
            err.to_string()
                .contains("fixed format requires numeric value"),
            "unexpected error: {}",
            err
        );
    }
}
