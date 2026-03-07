//! Python traceback -> Shape source location mapping.

use crate::runtime::CompiledFunction;

/// Parsed representation of a Python traceback frame.
#[derive(Debug, Clone)]
pub struct PythonFrame {
    pub filename: String,
    pub line: u32,
    pub function: String,
    pub text: Option<String>,
}

/// Parse a Python traceback string into structured frames.
pub fn parse_traceback(_traceback: &str) -> Vec<PythonFrame> {
    Vec::new()
}

/// Map a Python line number inside `__shape_fn__` back to the Shape
/// source line number.
pub fn map_python_line_to_shape(python_line: u32, shape_body_start_line: u32) -> u32 {
    if python_line < 2 {
        shape_body_start_line
    } else {
        shape_body_start_line + (python_line - 1)
    }
}

/// Format a Python error with context from the compiled function.
#[cfg(feature = "pyo3")]
pub fn format_python_error(
    py: pyo3::Python<'_>,
    err: &pyo3::PyErr,
    func: &CompiledFunction,
) -> String {
    use pyo3::types::PyTracebackMethods;
    let traceback_str = err
        .traceback(py)
        .and_then(|tb| tb.format().ok())
        .unwrap_or_default();

    // Try to extract the relevant line number from the traceback
    let mut shape_line = None;
    for line in traceback_str.lines() {
        if line.contains("<shape>") || line.contains("__shape__") {
            // Parse "line N" from the traceback
            if let Some(pos) = line.find("line ") {
                let after = &line[pos + 5..];
                if let Some(end) = after.find(|c: char| !c.is_ascii_digit()) {
                    if let Ok(py_line) = after[..end].parse::<u32>() {
                        shape_line = Some(map_python_line_to_shape(
                            py_line,
                            func.shape_body_start_line,
                        ));
                    }
                } else if let Ok(py_line) = after.trim().parse::<u32>() {
                    shape_line = Some(map_python_line_to_shape(
                        py_line,
                        func.shape_body_start_line,
                    ));
                }
            }
        }
    }

    if let Some(line) = shape_line {
        format!("Python error in '{}' at line {}: {}", func.name, line, err)
    } else {
        format!("Python error in '{}': {}", func.name, err)
    }
}

/// Fallback when pyo3 is not enabled.
#[cfg(not(feature = "pyo3"))]
pub fn format_python_error(_err: &str, func: &CompiledFunction) -> String {
    format!("Python error in '{}': pyo3 not enabled", func.name)
}
