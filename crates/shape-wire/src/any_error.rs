//! Structured AnyError parsing and rendering.
//!
//! This module decodes runtime `AnyError` payloads from `WireValue` and renders
//! them for different targets (plain text, ANSI terminal, HTML).

use crate::WireValue;

/// Parsed stack frame in an AnyError trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyErrorFrame {
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub ip: Option<usize>,
}

/// Parsed AnyError chain node.
#[derive(Debug, Clone, PartialEq)]
pub struct AnyError {
    pub category: String,
    pub message: String,
    pub code: Option<String>,
    pub payload: WireValue,
    pub frames: Vec<AnyErrorFrame>,
    pub cause: Option<Box<AnyError>>,
}

impl AnyError {
    /// Decode an `AnyError` from a `WireValue`.
    pub fn from_wire(value: &WireValue) -> Option<Self> {
        let obj = value.as_object()?;
        let category = obj.get("category").and_then(WireValue::as_str)?.to_string();
        if category != "AnyError" {
            return None;
        }

        let payload = obj.get("payload").cloned().unwrap_or(WireValue::Null);
        let message = obj
            .get("message")
            .and_then(WireValue::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| brief_value(&payload));
        let code = obj
            .get("code")
            .and_then(WireValue::as_str)
            .map(str::to_string);

        let frames = obj
            .get("trace_info")
            .map(parse_trace_info)
            .unwrap_or_default();

        let cause = obj
            .get("cause")
            .filter(|v| !v.is_null())
            .and_then(Self::from_wire)
            .map(Box::new);

        Some(Self {
            category,
            message,
            code,
            payload,
            frames,
            cause,
        })
    }

    /// Best-effort primary source location from the top frame.
    pub fn primary_location(&self) -> Option<AnyErrorFrame> {
        self.frames.first().cloned().or_else(|| {
            self.cause
                .as_deref()
                .and_then(|cause| cause.primary_location())
        })
    }
}

/// Renderer contract for AnyError output targets.
pub trait AnyErrorRenderer {
    fn render(&self, error: &AnyError) -> String;
}

/// Plain-text AnyError renderer.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlainAnyErrorRenderer;

impl AnyErrorRenderer for PlainAnyErrorRenderer {
    fn render(&self, error: &AnyError) -> String {
        let mut out = String::from("Uncaught exception:");
        render_plain_node(error, true, &mut out);
        out
    }
}

/// ANSI terminal AnyError renderer.
#[derive(Debug, Default, Clone, Copy)]
pub struct AnsiAnyErrorRenderer;

impl AnyErrorRenderer for AnsiAnyErrorRenderer {
    fn render(&self, error: &AnyError) -> String {
        let mut out = String::new();
        out.push_str("\x1b[1;31mUncaught exception:\x1b[0m");
        render_ansi_node(error, true, &mut out);
        out
    }
}

/// HTML AnyError renderer for server/UI targets.
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlAnyErrorRenderer;

impl AnyErrorRenderer for HtmlAnyErrorRenderer {
    fn render(&self, error: &AnyError) -> String {
        let mut out = String::new();
        out.push_str("<div class=\"shape-error\">");
        out.push_str("<div class=\"shape-error-header\">Uncaught exception:</div>");
        render_html_node(error, true, &mut out);
        out.push_str("</div>");
        out
    }
}

/// Render an AnyError from wire value using a renderer.
pub fn render_any_error_with<R: AnyErrorRenderer>(
    value: &WireValue,
    renderer: &R,
) -> Option<String> {
    AnyError::from_wire(value).map(|err| renderer.render(&err))
}

/// Plain rendering helper.
pub fn render_any_error_plain(value: &WireValue) -> Option<String> {
    render_any_error_with(value, &PlainAnyErrorRenderer)
}

/// ANSI rendering helper.
pub fn render_any_error_ansi(value: &WireValue) -> Option<String> {
    render_any_error_with(value, &AnsiAnyErrorRenderer)
}

/// HTML rendering helper.
pub fn render_any_error_html(value: &WireValue) -> Option<String> {
    render_any_error_with(value, &HtmlAnyErrorRenderer)
}

/// Render AnyError for terminal output using environment-aware capabilities.
///
/// Prefers ANSI output when color/ANSI appears supported. Falls back to plain
/// text for `NO_COLOR`, non-interactive/dumb terminals, or explicitly disabled
/// color settings.
pub fn render_any_error_terminal(value: &WireValue) -> Option<String> {
    if terminal_supports_ansi() {
        render_any_error_ansi(value)
    } else {
        render_any_error_plain(value)
    }
}

fn terminal_supports_ansi() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }

    if std::env::var("CLICOLOR").ok().as_deref() == Some("0") {
        return false;
    }

    if std::env::var_os("FORCE_COLOR").is_some()
        || std::env::var("CLICOLOR_FORCE")
            .map(|v| v != "0")
            .unwrap_or(false)
    {
        return true;
    }

    match std::env::var("TERM") {
        Ok(term) if !term.is_empty() && term != "dumb" => true,
        _ => false,
    }
}

fn render_plain_node(error: &AnyError, root: bool, out: &mut String) {
    if root {
        if let Some(code) = &error.code {
            out.push_str(&format!("\nError [{}]: {}", code, error.message));
        } else {
            out.push_str(&format!("\nError: {}", error.message));
        }
    } else if let Some(code) = &error.code {
        out.push_str(&format!("\nCaused by [{}]: {}", code, error.message));
    } else {
        out.push_str(&format!("\nCaused by: {}", error.message));
    }

    for frame in &error.frames {
        out.push_str("\n  at ");
        out.push_str(frame.function.as_deref().unwrap_or("<anonymous>"));
        if frame.file.is_some() || frame.line.is_some() || frame.column.is_some() {
            out.push_str(" (");
            match (&frame.file, frame.line, frame.column) {
                (Some(file), Some(line), Some(column)) => {
                    out.push_str(&format!("{file}:{line}:{column}"))
                }
                (Some(file), Some(line), None) => out.push_str(&format!("{file}:{line}")),
                (Some(file), None, _) => out.push_str(file),
                (None, Some(line), Some(column)) => out.push_str(&format!("line {line}:{column}")),
                (None, Some(line), None) => out.push_str(&format!("line {line}")),
                (None, None, Some(column)) => out.push_str(&format!("column {column}")),
                (None, None, None) => {}
            }
            out.push(')');
        }
        if let Some(ip) = frame.ip {
            out.push_str(&format!(" [ip {}]", ip));
        }
    }

    if let Some(cause) = &error.cause {
        render_plain_node(cause, false, out);
    }
}

fn render_ansi_node(error: &AnyError, root: bool, out: &mut String) {
    if root {
        if let Some(code) = &error.code {
            out.push_str(&format!(
                "\n\x1b[1;31mError [{}]\x1b[0m: {}",
                code, error.message
            ));
        } else {
            out.push_str(&format!("\n\x1b[1;31mError\x1b[0m: {}", error.message));
        }
    } else if let Some(code) = &error.code {
        out.push_str(&format!(
            "\n\x1b[33mCaused by [{}]\x1b[0m: {}",
            code, error.message
        ));
    } else {
        out.push_str(&format!("\n\x1b[33mCaused by\x1b[0m: {}", error.message));
    }

    for frame in &error.frames {
        out.push_str("\n  \x1b[36mat\x1b[0m ");
        out.push_str(frame.function.as_deref().unwrap_or("<anonymous>"));
        if frame.file.is_some() || frame.line.is_some() || frame.column.is_some() {
            out.push_str(" (\x1b[2m");
            match (&frame.file, frame.line, frame.column) {
                (Some(file), Some(line), Some(column)) => {
                    out.push_str(&format!("{file}:{line}:{column}"))
                }
                (Some(file), Some(line), None) => out.push_str(&format!("{file}:{line}")),
                (Some(file), None, _) => out.push_str(file),
                (None, Some(line), Some(column)) => out.push_str(&format!("line {line}:{column}")),
                (None, Some(line), None) => out.push_str(&format!("line {line}")),
                (None, None, Some(column)) => out.push_str(&format!("column {column}")),
                (None, None, None) => {}
            }
            out.push_str("\x1b[0m)");
        }
        if let Some(ip) = frame.ip {
            out.push_str(&format!(" [ip {}]", ip));
        }
    }

    if let Some(cause) = &error.cause {
        render_ansi_node(cause, false, out);
    }
}

fn render_html_node(error: &AnyError, root: bool, out: &mut String) {
    if root {
        out.push_str("<div class=\"shape-error-main\">");
        if let Some(code) = &error.code {
            out.push_str(&format!(
                "<span class=\"shape-error-label\">Error [{}]</span>: <span class=\"shape-error-message\">{}</span>",
                escape_html(code),
                escape_html(&error.message),
            ));
        } else {
            out.push_str(&format!(
                "<span class=\"shape-error-label\">Error</span>: <span class=\"shape-error-message\">{}</span>",
                escape_html(&error.message),
            ));
        }
        out.push_str("</div>");
    } else {
        out.push_str("<div class=\"shape-error-cause\">");
        if let Some(code) = &error.code {
            out.push_str(&format!(
                "<span class=\"shape-error-cause-label\">Caused by [{}]</span>: <span class=\"shape-error-message\">{}</span>",
                escape_html(code),
                escape_html(&error.message),
            ));
        } else {
            out.push_str(&format!(
                "<span class=\"shape-error-cause-label\">Caused by</span>: <span class=\"shape-error-message\">{}</span>",
                escape_html(&error.message),
            ));
        }
        out.push_str("</div>");
    }

    for frame in &error.frames {
        out.push_str("<div class=\"shape-error-frame\">");
        out.push_str("<span class=\"shape-error-at\">at</span> ");
        out.push_str(&escape_html(
            frame.function.as_deref().unwrap_or("<anonymous>"),
        ));
        if frame.file.is_some() || frame.line.is_some() || frame.column.is_some() {
            out.push_str(" <span class=\"shape-error-loc\">(");
            match (&frame.file, frame.line, frame.column) {
                (Some(file), Some(line), Some(column)) => {
                    out.push_str(&escape_html(&format!("{file}:{line}:{column}")))
                }
                (Some(file), Some(line), None) => {
                    out.push_str(&escape_html(&format!("{file}:{line}")))
                }
                (Some(file), None, _) => out.push_str(&escape_html(file)),
                (None, Some(line), Some(column)) => {
                    out.push_str(&escape_html(&format!("line {line}:{column}")))
                }
                (None, Some(line), None) => out.push_str(&escape_html(&format!("line {line}"))),
                (None, None, Some(column)) => {
                    out.push_str(&escape_html(&format!("column {column}")))
                }
                (None, None, None) => {}
            }
            out.push_str(")</span>");
        }
        if let Some(ip) = frame.ip {
            out.push_str(&format!(
                " <span class=\"shape-error-ip\">[ip {}]</span>",
                ip
            ));
        }
        out.push_str("</div>");
    }

    if let Some(cause) = &error.cause {
        render_html_node(cause, false, out);
    }
}

fn parse_trace_info(value: &WireValue) -> Vec<AnyErrorFrame> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let kind = obj
        .get("kind")
        .and_then(WireValue::as_str)
        .unwrap_or("full");
    if kind == "single" {
        obj.get("frame")
            .and_then(parse_trace_frame)
            .into_iter()
            .collect()
    } else {
        match obj.get("frames") {
            Some(WireValue::Array(frames)) => frames.iter().filter_map(parse_trace_frame).collect(),
            _ => Vec::new(),
        }
    }
}

fn parse_trace_frame(value: &WireValue) -> Option<AnyErrorFrame> {
    let obj = value.as_object()?;
    Some(AnyErrorFrame {
        function: obj
            .get("function")
            .and_then(WireValue::as_str)
            .map(str::to_string),
        file: obj
            .get("file")
            .and_then(WireValue::as_str)
            .map(str::to_string),
        line: obj.get("line").and_then(as_usize),
        column: obj.get("column").and_then(as_usize),
        ip: obj.get("ip").and_then(as_usize),
    })
}

fn as_usize(value: &WireValue) -> Option<usize> {
    match value {
        WireValue::Integer(i) if *i >= 0 => Some(*i as usize),
        WireValue::Number(n) if *n >= 0.0 => Some(*n as usize),
        WireValue::I8(v) if *v >= 0 => Some(*v as usize),
        WireValue::U8(v) => Some(*v as usize),
        WireValue::I16(v) if *v >= 0 => Some(*v as usize),
        WireValue::U16(v) => Some(*v as usize),
        WireValue::I32(v) if *v >= 0 => Some(*v as usize),
        WireValue::U32(v) => Some(*v as usize),
        WireValue::I64(v) if *v >= 0 => Some(*v as usize),
        WireValue::U64(v) => usize::try_from(*v).ok(),
        WireValue::Isize(v) if *v >= 0 => Some(*v as usize),
        WireValue::Usize(v) => usize::try_from(*v).ok(),
        WireValue::Ptr(v) => usize::try_from(*v).ok(),
        WireValue::F32(v) if *v >= 0.0 => Some(*v as usize),
        _ => None,
    }
}

fn brief_value(value: &WireValue) -> String {
    match value {
        WireValue::Null => "null".to_string(),
        WireValue::Bool(v) => v.to_string(),
        WireValue::Integer(v) => v.to_string(),
        WireValue::Number(v) => v.to_string(),
        WireValue::I8(v) => v.to_string(),
        WireValue::U8(v) => v.to_string(),
        WireValue::I16(v) => v.to_string(),
        WireValue::U16(v) => v.to_string(),
        WireValue::I32(v) => v.to_string(),
        WireValue::U32(v) => v.to_string(),
        WireValue::I64(v) => v.to_string(),
        WireValue::U64(v) => v.to_string(),
        WireValue::Isize(v) => v.to_string(),
        WireValue::Usize(v) => v.to_string(),
        WireValue::Ptr(v) => format!("0x{v:x}"),
        WireValue::F32(v) => v.to_string(),
        WireValue::String(v) => v.clone(),
        WireValue::Result { ok, value } => {
            if *ok {
                format!("Ok({})", brief_value(value))
            } else {
                format!("Err({})", brief_value(value))
            }
        }
        WireValue::Object(_) => "{object}".to_string(),
        WireValue::Array(v) => format!("[array:{}]", v.len()),
        WireValue::FunctionRef { name } => format!("fn {}", name),
        WireValue::Timestamp(ts) => format!("ts({})", ts),
        WireValue::Duration { value, unit } => format!("{value:?}{unit:?}"),
        WireValue::Range { .. } => "<range>".to_string(),
        WireValue::Table(t) => format!("<table {}x{}>", t.row_count, t.column_count),
        WireValue::PrintResult(pr) => pr.rendered.clone(),
    }
}

fn escape_html(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect::<Vec<_>>(),
            '>' => "&gt;".chars().collect::<Vec<_>>(),
            '"' => "&quot;".chars().collect::<Vec<_>>(),
            '\'' => "&#39;".chars().collect::<Vec<_>>(),
            _ => vec![c],
        })
        .collect()
}

trait WireValueObjectExt {
    fn as_object(&self) -> Option<&std::collections::BTreeMap<String, WireValue>>;
}

impl WireValueObjectExt for WireValue {
    fn as_object(&self) -> Option<&std::collections::BTreeMap<String, WireValue>> {
        if let WireValue::Object(obj) = self {
            Some(obj)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn trace_frame(function: &str, file: &str, line: i64, column: i64, ip: i64) -> WireValue {
        let mut obj = BTreeMap::new();
        obj.insert(
            "function".to_string(),
            WireValue::String(function.to_string()),
        );
        obj.insert("file".to_string(), WireValue::String(file.to_string()));
        obj.insert("line".to_string(), WireValue::Integer(line));
        obj.insert("column".to_string(), WireValue::Integer(column));
        obj.insert("ip".to_string(), WireValue::Integer(ip));
        WireValue::Object(obj)
    }

    fn trace_info_single(frame: WireValue) -> WireValue {
        let mut obj = BTreeMap::new();
        obj.insert("kind".to_string(), WireValue::String("single".to_string()));
        obj.insert("frame".to_string(), frame);
        WireValue::Object(obj)
    }

    fn any_error(
        message: &str,
        code: Option<&str>,
        cause: Option<WireValue>,
        trace_info: WireValue,
    ) -> WireValue {
        let mut obj = BTreeMap::new();
        obj.insert(
            "category".to_string(),
            WireValue::String("AnyError".to_string()),
        );
        obj.insert(
            "payload".to_string(),
            WireValue::String(message.to_string()),
        );
        obj.insert("cause".to_string(), cause.unwrap_or(WireValue::Null));
        obj.insert("trace_info".to_string(), trace_info);
        obj.insert(
            "message".to_string(),
            WireValue::String(message.to_string()),
        );
        obj.insert(
            "code".to_string(),
            code.map(|c| WireValue::String(c.to_string()))
                .unwrap_or(WireValue::Null),
        );
        WireValue::Object(obj)
    }

    #[test]
    fn parse_and_render_plain() {
        let root = any_error(
            "low level",
            None,
            None,
            trace_info_single(trace_frame("read_file", "cfg.shape", 3, 10, 11)),
        );
        let outer = any_error(
            "high level",
            Some("OPTION_NONE"),
            Some(root),
            trace_info_single(trace_frame("load_config", "cfg.shape", 7, 12, 29)),
        );

        let parsed = AnyError::from_wire(&outer).expect("should parse anyerror");
        assert_eq!(parsed.code.as_deref(), Some("OPTION_NONE"));
        assert_eq!(parsed.frames[0].line, Some(7));
        assert_eq!(parsed.frames[0].column, Some(12));

        let rendered = PlainAnyErrorRenderer.render(&parsed);
        assert!(rendered.contains("Uncaught exception:"));
        assert!(rendered.contains("Error [OPTION_NONE]: high level"));
        assert!(rendered.contains("cfg.shape:7:12"));
        assert!(rendered.contains("Caused by: low level"));
    }

    #[test]
    fn render_ansi_and_html() {
        let err = any_error(
            "boom <bad>",
            Some("E_TEST"),
            None,
            trace_info_single(trace_frame("run", "main.shape", 1, 2, 3)),
        );
        let ansi = render_any_error_ansi(&err).expect("ansi render");
        assert!(ansi.contains("\x1b[1;31m"));
        assert!(ansi.contains("E_TEST"));

        let html = render_any_error_html(&err).expect("html render");
        assert!(html.contains("shape-error"));
        assert!(html.contains("E_TEST"));
        assert!(html.contains("&lt;bad&gt;"));
    }
}
