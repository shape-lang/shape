//! Content method dispatch for ContentNode values.
//!
//! Supports style methods: `.fg()`, `.bg()`, `.bold()`, `.italic()`, `.underline()`, `.dim()`
//! Table methods: `.border()`, `.max_rows()`
//! Chart methods: `.series()`, `.title()`, `.x_label()`, `.y_label()`

use crate::executor::VirtualMachine;
use shape_value::content::{Color, ContentNode, NamedColor};
use shape_value::{VMError, ValueWord};

impl VirtualMachine {
    /// Dispatch a method call on a Content value.
    pub(in crate::executor) fn handle_content_method(
        &mut self,
        method: &str,
        args: Vec<ValueWord>,
    ) -> Result<(), VMError> {
        // args[0] is the receiver (content node)
        if args.is_empty() {
            return Err(VMError::RuntimeError(
                "Content method called with no receiver".to_string(),
            ));
        }

        let node = args[0]
            .as_content()
            .cloned()
            .unwrap_or_else(|| ContentNode::plain(format!("{}", args[0])));

        let result = match method {
            "bold" => node.with_bold(),
            "italic" => node.with_italic(),
            "underline" => node.with_underline(),
            "dim" => node.with_dim(),
            "fg" => {
                let color = parse_color_arg(&args, 1, "fg")?;
                node.with_fg(color)
            }
            "bg" => {
                let color = parse_color_arg(&args, 1, "bg")?;
                node.with_bg(color)
            }
            "toString" => {
                let text = format!("{}", node);
                self.push_vw(ValueWord::from_string(std::sync::Arc::new(text)))?;
                return Ok(());
            }
            // Delegate to runtime content_methods for table/chart methods
            "border" | "max_rows" | "maxRows" | "series" | "title" | "x_label" | "xLabel"
            | "y_label" | "yLabel" => {
                let receiver = args[0].clone();
                let method_args = args[1..].to_vec();
                match shape_runtime::content_methods::call_content_method(
                    method,
                    receiver,
                    method_args,
                ) {
                    Some(Ok(result_nb)) => {
                        self.push_vw(result_nb)?;
                        return Ok(());
                    }
                    Some(Err(e)) => {
                        return Err(VMError::RuntimeError(format!("{}", e)));
                    }
                    None => {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown method '{}' on Content type",
                            method
                        )));
                    }
                }
            }
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Unknown method '{}' on Content type. Available: bold, italic, underline, dim, fg, bg, border, max_rows, series, title, x_label, y_label, toString",
                    method
                )));
            }
        };

        self.push_vw(ValueWord::from_content(result))?;
        Ok(())
    }
}

/// Parse a color argument from a method call (e.g., `.fg("red")` or `.fg(255, 0, 0)`).
fn parse_color_arg(
    args: &[ValueWord],
    start_idx: usize,
    method_name: &str,
) -> Result<Color, VMError> {
    if args.len() <= start_idx {
        return Err(VMError::RuntimeError(format!(
            "Content.{}() requires a color argument",
            method_name
        )));
    }

    // String color name
    if let Some(name) = args[start_idx].as_str() {
        return match name.to_lowercase().as_str() {
            "red" => Ok(Color::Named(NamedColor::Red)),
            "green" => Ok(Color::Named(NamedColor::Green)),
            "blue" => Ok(Color::Named(NamedColor::Blue)),
            "yellow" => Ok(Color::Named(NamedColor::Yellow)),
            "magenta" => Ok(Color::Named(NamedColor::Magenta)),
            "cyan" => Ok(Color::Named(NamedColor::Cyan)),
            "white" => Ok(Color::Named(NamedColor::White)),
            "default" => Ok(Color::Named(NamedColor::Default)),
            _ => Err(VMError::RuntimeError(format!(
                "Unknown color name '{}'. Available: red, green, blue, yellow, magenta, cyan, white, default",
                name
            ))),
        };
    }

    // RGB as three numeric args
    if args.len() >= start_idx + 3 {
        let r = args[start_idx]
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("RGB red component must be numeric".to_string()))?
            as u8;
        let g = args[start_idx + 1].as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError("RGB green component must be numeric".to_string())
        })? as u8;
        let b = args[start_idx + 2].as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError("RGB blue component must be numeric".to_string())
        })? as u8;
        return Ok(Color::Rgb(r, g, b));
    }

    Err(VMError::RuntimeError(format!(
        "Content.{}() requires a color name string or three RGB components",
        method_name
    )))
}
