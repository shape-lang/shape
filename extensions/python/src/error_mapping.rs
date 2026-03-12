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
///
/// Recognises the standard CPython traceback format:
///
/// ```text
/// Traceback (most recent call last):
///   File "script.py", line 10, in <module>
///     some_code()
///   File "other.py", line 5, in func
///     do_thing()
/// ErrorType: message
/// ```
///
/// Each `File "...", line N, in <name>` line becomes a [`PythonFrame`].
/// The optional indented source-text line that follows is captured in
/// [`PythonFrame::text`].
pub fn parse_traceback(traceback: &str) -> Vec<PythonFrame> {
    let lines: Vec<&str> = traceback.lines().collect();
    let mut frames = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("File \"") {
            if let Some(frame) = parse_file_line(trimmed) {
                // Check if the next line is indented source text (not another
                // File line or the error summary).
                let text = if i + 1 < lines.len() {
                    let next = lines[i + 1];
                    let next_trimmed = next.trim();
                    // Source text lines are indented and do NOT start with "File "
                    if !next_trimmed.is_empty()
                        && !next_trimmed.starts_with("File \"")
                        && !next_trimmed.starts_with("Traceback")
                        && next.starts_with(' ')
                    {
                        i += 1; // consume the source text line
                        Some(next_trimmed.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                };

                frames.push(PythonFrame {
                    filename: frame.0,
                    line: frame.1,
                    function: frame.2,
                    text,
                });
            }
        }
        i += 1;
    }

    frames
}

/// Parse a single `File "filename", line N, in funcname` line.
/// Returns `(filename, line_number, function_name)` on success.
fn parse_file_line(trimmed: &str) -> Option<(String, u32, String)> {
    // Strip the leading `File "` prefix
    let rest = trimmed.strip_prefix("File \"")?;
    let quote_end = rest.find('"')?;
    let filename = &rest[..quote_end];
    let after_quote = &rest[quote_end + 1..];

    // Extract line number from ", line N" portion
    let line_start = after_quote.find("line ")?;
    let num_str = &after_quote[line_start + 5..];

    // The line number ends at the next comma (or end-of-string)
    let line_no = if let Some(comma) = num_str.find(',') {
        num_str[..comma].trim().parse::<u32>().ok()?
    } else {
        num_str.trim().parse::<u32>().ok()?
    };

    // Extract function name from ", in <name>" portion (if present)
    let function = after_quote
        .rfind("in ")
        .map(|i| after_quote[i + 3..].trim().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());

    Some((filename.to_string(), line_no, function))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_traceback_full_example() {
        let tb = "\
Traceback (most recent call last):
  File \"script.py\", line 10, in <module>
    some_code()
  File \"other.py\", line 5, in func
    do_thing()
ValueError: bad value";
        let frames = parse_traceback(tb);
        assert_eq!(frames.len(), 2);

        assert_eq!(frames[0].filename, "script.py");
        assert_eq!(frames[0].line, 10);
        assert_eq!(frames[0].function, "<module>");
        assert_eq!(frames[0].text.as_deref(), Some("some_code()"));

        assert_eq!(frames[1].filename, "other.py");
        assert_eq!(frames[1].line, 5);
        assert_eq!(frames[1].function, "func");
        assert_eq!(frames[1].text.as_deref(), Some("do_thing()"));
    }

    #[test]
    fn parse_traceback_no_source_text() {
        let tb = "\
Traceback (most recent call last):
  File \"a.py\", line 1, in main
TypeError: oops";
        let frames = parse_traceback(tb);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].filename, "a.py");
        assert_eq!(frames[0].line, 1);
        assert_eq!(frames[0].function, "main");
        assert!(frames[0].text.is_none());
    }

    #[test]
    fn parse_traceback_empty_input() {
        assert!(parse_traceback("").is_empty());
    }

    #[test]
    fn parse_traceback_no_traceback_lines() {
        let tb = "RuntimeError: something went wrong";
        assert!(parse_traceback(tb).is_empty());
    }

    #[test]
    fn parse_traceback_shape_internal_frame() {
        let tb = "  File \"<shape>\", line 3, in __shape_fn__\n    return x + 1";
        let frames = parse_traceback(tb);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].filename, "<shape>");
        assert_eq!(frames[0].line, 3);
        assert_eq!(frames[0].function, "__shape_fn__");
        assert_eq!(frames[0].text.as_deref(), Some("return x + 1"));
    }

    #[test]
    fn map_python_line_to_shape_basics() {
        // line < 2 maps to start
        assert_eq!(map_python_line_to_shape(1, 10), 10);
        assert_eq!(map_python_line_to_shape(0, 10), 10);
        // line >= 2 maps to start + (line - 1)
        assert_eq!(map_python_line_to_shape(2, 10), 11);
        assert_eq!(map_python_line_to_shape(5, 10), 14);
    }
}
