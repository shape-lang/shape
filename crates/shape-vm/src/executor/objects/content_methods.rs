//! Content method dispatch for ContentNode values.
//!
//! SC2 (R8 — supervisor): the style chain (`bold` / `italic` / `underline`
//! / `dim` / `fg` / `bg`), `toString`, and the Table `max_rows` builder are
//! implemented via the kinded MethodHandler ABI — receiver is
//! `NativeKind::Ptr(HeapKind::Content)`, each method clones the underlying
//! `ContentNode` and applies one of the `content.rs` `with_*` mutation
//! helpers, returning a fresh Content slot (or a String slot for
//! `toString`, which renders via the same `TerminalRenderer` the `print()`
//! Content arm uses). The chart-builder methods (`series` / `title` /
//! `x_label` / `y_label`) remain `NotImplemented(SURFACE)` because
//! `Content.chart` itself is deferred to v0.4 (supervisor D4).
//!
//! `Content` is a surviving `HeapKind` variant
//! (`Content(Arc<ContentNode>)` per ADR-006 §2.3 +
//! `crates/shape-value/src/heap_variants.rs`): the receiver is a
//! `NativeKind::Ptr(HeapKind::Content)` slot whose bits are
//! `Arc::into_raw(Arc<ContentNode>)` (set by `KindedSlot::from_content`).
//! Each method borrows the receiver via `recv_content`, clones the inner
//! node, mutates one field, and rewraps a fresh Content slot; `toString`
//! returns a `NativeKind::String` slot. No Bool-default, no dynamic
//! fallback, no `ValueWord`-era crate-boundary detour — the dead
//! `content_builders.rs` (deferred()-only) and the runtime
//! `call_content_method` (always-None) were deleted, superseded by this
//! opcode/MethodHandler path.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{HeapKind, KindedSlot, NativeKind, VMError};

/// Chart-builder methods (`series` / `title` / `x_label` / `y_label`)
/// stay surfaced because `Content.chart` itself is deferred to v0.4
/// (supervisor D4). They are wired here so the method names resolve, but
/// building a `ChartSpec` is out of scope until chart rendering lands.
#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "Content.{}(): chart builder methods are deferred to v0.4 \
         (supervisor D4) — Content.chart rendering is not implemented this \
         round.",
        method
    ))
}

// ─────────────────────────────────────────────────────────────────────────
// W18.5 Content builder method helpers (R8 W4, 2026-05-24 — supervisor D4)
//
// Receivers are `Ptr(HeapKind::Content)` slots whose bits are
// `Arc::into_raw(Arc<ContentNode>)` (set by `KindedSlot::from_content`
// at `Content.text` / `Content.code` / W18.5 `*BuilderNew` / etc.). Each
// builder method (a) clones the underlying ContentNode, (b) mutates a
// single field on the matching variant, (c) wraps a fresh Arc and
// returns a new `Ptr(HeapKind::Content)` slot via
// `KindedSlot::from_content`. The receiver's share retires when the
// dispatcher drops its `args[0]`.
//
// `.build()` is identity — returns a fresh KindedSlot pointing at the
// same ContentNode (via a clone of the existing Arc). The supervisor D4
// "shortest path builder → content → renderer" disposition means
// `.build()` exists for ergonomics; it is NOT a typed Table → content
// conversion (no parallel typed Table value, no separate builder
// HeapKind, no shape-runtime crate detour).
//
// String-typed MVP: methods like `.border("rounded")` take a string
// (not a `BorderStyleSpec` enum). Per task spec coordination with
// `r8w4-w18-4-fstring-styling` (which revives the shared spec types
// module): after W18.4 lands, a follow-up refactor swaps the string-
// typed params for spec-type params. This avoids the parallel-
// implementation defection (CLAUDE.md §Parallel-implementation across
// producer/consumer carrier-shape boundaries).
// ─────────────────────────────────────────────────────────────────────────

/// Borrow the receiver `KindedSlot` as `&ContentNode`. Errors when the
/// kind is wrong. The returned reference borrows from `args[0]` — the
/// caller-owned share keeps the inner ContentNode alive.
#[inline]
fn recv_content<'a>(
    args: &'a [KindedSlot],
    method: &str,
) -> Result<&'a shape_value::content::ContentNode, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "Content.{}(): no receiver",
            method
        )));
    }
    if args[0].kind != NativeKind::Ptr(HeapKind::Content) {
        return Err(VMError::RuntimeError(format!(
            "Content.{}(): expected Content receiver, got kind {:?}",
            method, args[0].kind
        )));
    }
    let bits = args[0].slot.raw();
    if bits == 0 {
        return Err(VMError::RuntimeError(format!(
            "Content.{}(): null Content receiver",
            method
        )));
    }
    // SAFETY: Content-kind slot bits are `Arc::into_raw(Arc<ContentNode>)`
    // per `KindedSlot::from_content` (kinded_slot.rs:376). The dispatcher
    // owns one strong-count share for the call duration; the returned
    // `&ContentNode` borrows for the lifetime of `args[0]`.
    let node: &shape_value::content::ContentNode =
        unsafe { &*(bits as *const shape_value::content::ContentNode) };
    Ok(node)
}

/// Wrap a freshly-built `ContentNode` into a `KindedSlot` of kind
/// `Ptr(HeapKind::Content)`. Builder methods return the result of this
/// helper so chained calls flow through the same carrier shape.
#[inline]
fn content_slot(node: shape_value::content::ContentNode) -> KindedSlot {
    KindedSlot::from_content(std::sync::Arc::new(node))
}

/// Parse a border-style string (case-insensitive) into the matching
/// `BorderStyle` enum variant. Unknown strings produce an error.
#[inline]
fn parse_border_style(s: &str) -> Result<shape_value::content::BorderStyle, VMError> {
    use shape_value::content::BorderStyle;
    match s.to_ascii_lowercase().as_str() {
        "rounded" => Ok(BorderStyle::Rounded),
        "sharp" => Ok(BorderStyle::Sharp),
        "heavy" => Ok(BorderStyle::Heavy),
        "double" => Ok(BorderStyle::Double),
        "minimal" => Ok(BorderStyle::Minimal),
        "none" => Ok(BorderStyle::None),
        other => Err(VMError::RuntimeError(format!(
            "Content.border(): unknown border style '{}' — expected one of \
             rounded / sharp / heavy / double / minimal / none",
            other
        ))),
    }
}

/// Assert a style method received no extra arguments (receiver only).
#[inline]
fn expect_no_args(args: &[KindedSlot], method: &str) -> Result<(), VMError> {
    if args.len() != 1 {
        return Err(VMError::RuntimeError(format!(
            "Content.{}() takes no arguments, got {}",
            method,
            args.len().saturating_sub(1)
        )));
    }
    Ok(())
}

/// Parse the SC1 string Color carrier into a `content::Color`. SC1
/// `Color.red` lowers to the canonical snake_case string `"red"`;
/// `Color.rgb(r,g,b)` lowers to `"rgb(r,g,b)"` (no spaces). Unknown
/// strings produce an error.
#[inline]
fn parse_color(s: &str) -> Result<shape_value::content::Color, VMError> {
    use shape_value::content::{Color, NamedColor};
    let trimmed = s.trim();
    if let Some(inner) = trimmed
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() != 3 {
            return Err(VMError::RuntimeError(format!(
                "Content color: rgb() expects 3 channels, got {}",
                parts.len()
            )));
        }
        let mut chan = [0u8; 3];
        for (i, p) in parts.iter().enumerate() {
            chan[i] = p.trim().parse::<u8>().map_err(|_| {
                VMError::RuntimeError(format!(
                    "Content color: rgb() channel '{}' out of range (0-255)",
                    p.trim()
                ))
            })?;
        }
        return Ok(Color::Rgb(chan[0], chan[1], chan[2]));
    }
    let named = match trimmed.to_ascii_lowercase().as_str() {
        "red" => NamedColor::Red,
        "green" => NamedColor::Green,
        "blue" => NamedColor::Blue,
        "yellow" => NamedColor::Yellow,
        "magenta" => NamedColor::Magenta,
        "cyan" => NamedColor::Cyan,
        "white" => NamedColor::White,
        "default" => NamedColor::Default,
        other => {
            return Err(VMError::RuntimeError(format!(
                "Content color: unknown color '{}' — expected red / green / \
                 blue / yellow / magenta / cyan / white / default or rgb(r,g,b)",
                other
            )))
        }
    };
    Ok(Color::Named(named))
}

pub fn v2_content_bold(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "bold")?;
    expect_no_args(args, "bold")?;
    Ok(content_slot(node.clone().with_bold()))
}

pub fn v2_content_italic(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "italic")?;
    expect_no_args(args, "italic")?;
    Ok(content_slot(node.clone().with_italic()))
}

pub fn v2_content_underline(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "underline")?;
    expect_no_args(args, "underline")?;
    Ok(content_slot(node.clone().with_underline()))
}

pub fn v2_content_dim(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "dim")?;
    expect_no_args(args, "dim")?;
    Ok(content_slot(node.clone().with_dim()))
}

pub fn v2_content_fg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "fg")?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.fg(color) requires exactly 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let color_str = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.fg(): color argument must be a string, got kind {:?}",
            args[1].kind
        ))
    })?;
    let color = parse_color(color_str)?;
    Ok(content_slot(node.clone().with_fg(color)))
}

pub fn v2_content_bg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "bg")?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.bg(color) requires exactly 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let color_str = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.bg(): color argument must be a string, got kind {:?}",
            args[1].kind
        ))
    })?;
    let color = parse_color(color_str)?;
    Ok(content_slot(node.clone().with_bg(color)))
}

pub fn v2_content_to_string(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "toString")?;
    expect_no_args(args, "toString")?;
    // Render through the same TerminalRenderer the `print()` Content arm
    // uses (printing.rs HeapKind::Content). The rendered string is the
    // user-visible representation; return it as a String-kind slot.
    use shape_runtime::content_renderer::ContentRenderer;
    let renderer = shape_runtime::renderers::terminal::TerminalRenderer::new();
    let rendered = renderer.render(node);
    Ok(KindedSlot::from_string(&rendered))
}

/// `table.border(style: string) -> content` — W18.5 builder method
/// (supervisor D4, R8 W4 2026-05-24). Sets the border style on a
/// `ContentNode::Table` receiver. Style strings: `"rounded"` (default),
/// `"sharp"`, `"heavy"`, `"double"`, `"minimal"`, `"none"`. Returns a
/// fresh Content slot.
///
/// String-typed MVP: post-W18.4 shared-styling-spec module merge, swap
/// this for `BorderStyleSpec` to retire the string-parse roundtrip.
pub fn v2_content_border(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "border")?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.border(style) requires exactly 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let style_str = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.border(): style argument must be a string, got kind {:?}",
            args[1].kind
        ))
    })?;
    let style = parse_border_style(style_str)?;
    match node {
        shape_value::content::ContentNode::Table(t) => {
            let mut new_table = t.clone();
            new_table.border = style;
            Ok(content_slot(shape_value::content::ContentNode::Table(
                new_table,
            )))
        }
        other => Err(VMError::RuntimeError(format!(
            "Content.border() is only valid on Table receivers (got {})",
            describe_content_variant(other)
        ))),
    }
}

#[inline]
fn describe_content_variant(node: &shape_value::content::ContentNode) -> &'static str {
    match node {
        shape_value::content::ContentNode::Text(_) => "Text",
        shape_value::content::ContentNode::Table(_) => "Table",
        shape_value::content::ContentNode::Code { .. } => "Code",
        shape_value::content::ContentNode::Chart(_) => "Chart",
        shape_value::content::ContentNode::KeyValue(_) => "KeyValue",
        shape_value::content::ContentNode::Fragment(_) => "Fragment",
    }
}

// ─────────────────────────────────────────────────────────────────────────
// W18.5 builder methods (R8 W4, 2026-05-24 — supervisor D4)
// ─────────────────────────────────────────────────────────────────────────

/// `table.headers(headers: Array<string>) -> content` — set the column
/// header strings on a Table receiver. Replaces any prior headers.
pub fn v2_content_headers(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "headers")?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.headers(arr) requires exactly 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let headers =
        crate::executor::vm_impl::builtins::read_string_array(&args[1]).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Content.headers(): argument must be Array<string>, got kind \
                 {:?}",
                args[1].kind
            ))
        })?;
    match node {
        shape_value::content::ContentNode::Table(t) => {
            let mut new_table = t.clone();
            new_table.headers = headers;
            Ok(content_slot(shape_value::content::ContentNode::Table(
                new_table,
            )))
        }
        other => Err(VMError::RuntimeError(format!(
            "Content.headers() is only valid on Table receivers (got {})",
            describe_content_variant(other)
        ))),
    }
}

/// `table.row(cells: Array<*>) -> content` — append one row to a Table
/// receiver. Cell values render through `format_kinded` so heterogeneous
/// types (strings, numbers, bools) all coerce to plain-text cells.
pub fn v2_content_row(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "row")?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.row(arr) requires exactly 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, read_element};
    if args[1].kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Content.row(): cells argument must be an Array, got kind {:?}",
            args[1].kind
        )));
    }
    let view = as_v2_typed_array(args[1].slot.raw(), args[1].kind).ok_or_else(|| {
        VMError::RuntimeError("Content.row(): cells array has invalid v2 header".to_string())
    })?;
    let formatter =
        crate::executor::printing::ValueFormatter::new(&vm.program.type_schema_registry);
    let mut cells: Vec<shape_value::content::ContentNode> = Vec::with_capacity(view.len as usize);
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!("Content.row(): failed to read cell at index {}", i))
        })?;
        let cell_slot = KindedSlot::new(shape_value::ValueSlot::from_raw(bits), kind);
        let rendered = formatter.format_kinded(&cell_slot);
        drop(cell_slot);
        cells.push(shape_value::content::ContentNode::plain(rendered));
    }
    match node {
        shape_value::content::ContentNode::Table(t) => {
            let mut new_table = t.clone();
            new_table.rows.push(cells);
            Ok(content_slot(shape_value::content::ContentNode::Table(
                new_table,
            )))
        }
        other => Err(VMError::RuntimeError(format!(
            "Content.row() is only valid on Table receivers (got {})",
            describe_content_variant(other)
        ))),
    }
}

/// `code.language(lang: string) -> content` — set the language tag on a
/// Code receiver. Renderers use the tag for syntax-highlight hints.
pub fn v2_content_language(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "language")?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.language(s) requires exactly 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let lang = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.language(): argument must be a string, got kind {:?}",
            args[1].kind
        ))
    })?;
    match node {
        shape_value::content::ContentNode::Code { source, .. } => {
            Ok(content_slot(shape_value::content::ContentNode::Code {
                language: Some(lang.to_string()),
                source: source.clone(),
            }))
        }
        other => Err(VMError::RuntimeError(format!(
            "Content.language() is only valid on Code receivers (got {})",
            describe_content_variant(other)
        ))),
    }
}

/// `code.source(src: string) -> content` — set the source body on a Code
/// receiver.
pub fn v2_content_source(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "source")?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.source(s) requires exactly 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let src = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.source(): argument must be a string, got kind {:?}",
            args[1].kind
        ))
    })?;
    match node {
        shape_value::content::ContentNode::Code { language, .. } => {
            Ok(content_slot(shape_value::content::ContentNode::Code {
                language: language.clone(),
                source: src.to_string(),
            }))
        }
        other => Err(VMError::RuntimeError(format!(
            "Content.source() is only valid on Code receivers (got {})",
            describe_content_variant(other)
        ))),
    }
}

/// `kv.pair(key: string, value: *) -> content` — append one
/// key/value pair to a KeyValue receiver. The value coerces through
/// `format_kinded` to a plain-text node (heterogeneous-value MVP);
/// post-styling-spec, value will accept a `content` arg directly so
/// nested content can be embedded.
pub fn v2_content_pair(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "pair")?;
    if args.len() != 3 {
        return Err(VMError::RuntimeError(format!(
            "Content.pair(key, value) requires exactly 2 arguments, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let key = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.pair(): key argument must be a string, got kind {:?}",
            args[1].kind
        ))
    })?;
    // Value coerces via `format_kinded` for the MVP. If the value slot is
    // itself a Content node, render its display form so nested content is
    // collapsed into a string cell (post-W18.4 swap can preserve nested
    // content; for v0.3 the KeyValue display path projects everything to
    // ContentNode::plain anyway).
    let formatter =
        crate::executor::printing::ValueFormatter::new(&vm.program.type_schema_registry);
    let rendered = formatter.format_kinded(&args[2]);
    let value_node = shape_value::content::ContentNode::plain(rendered);
    match node {
        shape_value::content::ContentNode::KeyValue(pairs) => {
            let mut new_pairs = pairs.clone();
            new_pairs.push((key.to_string(), value_node));
            Ok(content_slot(shape_value::content::ContentNode::KeyValue(
                new_pairs,
            )))
        }
        other => Err(VMError::RuntimeError(format!(
            "Content.pair() is only valid on KeyValue receivers (got {})",
            describe_content_variant(other)
        ))),
    }
}

/// `builder.build() -> content` — identity / finalize. Per supervisor D4
/// "shortest path builder → content → renderer", `.build()` returns the
/// receiver Content as a fresh KindedSlot — no typed Table → content
/// projection, no shape-runtime detour, no parallel typed builder value.
/// Exists for ergonomics so users can write `Table::new()...build()`.
pub fn v2_content_build(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, "build")?;
    Ok(content_slot(node.clone()))
}

/// `table.max_rows(n: int) -> content` — cap the number of rows the
/// renderer displays on a Table receiver. `n <= 0` clears the cap
/// (renders all rows). Returns a fresh Content slot.
fn content_max_rows_impl(args: &[KindedSlot], method: &str) -> Result<KindedSlot, VMError> {
    let node = recv_content(args, method)?;
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "Content.{}(n) requires exactly 1 argument, got {}",
            method,
            args.len().saturating_sub(1)
        )));
    }
    let n = args[1].as_i64().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Content.{}(): argument must be an int, got kind {:?}",
            method, args[1].kind
        ))
    })?;
    let cap = if n <= 0 { None } else { Some(n as usize) };
    match node {
        shape_value::content::ContentNode::Table(t) => {
            let mut new_table = t.clone();
            new_table.max_rows = cap;
            Ok(content_slot(shape_value::content::ContentNode::Table(
                new_table,
            )))
        }
        other => Err(VMError::RuntimeError(format!(
            "Content.{}() is only valid on Table receivers (got {})",
            method,
            describe_content_variant(other)
        ))),
    }
}

pub fn v2_content_max_rows(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    content_max_rows_impl(args, "max_rows")
}

pub fn v2_content_max_rows_camel(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    content_max_rows_impl(args, "maxRows")
}

pub fn v2_content_series(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("series"))
}

pub fn v2_content_title(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("title"))
}

pub fn v2_content_x_label(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("x_label"))
}

pub fn v2_content_x_label_camel(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("xLabel"))
}

pub fn v2_content_y_label(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("y_label"))
}

pub fn v2_content_y_label_camel(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("yLabel"))
}

#[cfg(test)]
mod sc1_style_spec_tests {
    //! SC1 (R8 — supervisor): Color / Border / ChartType namespace
    //! constructors. The runtime carrier is a `string` (canonical serde
    //! snake_case name; `rgb(r,g,b)` for the explicit RGB form), so the
    //! existing string-typed `.border(style)` method consumes them with no
    //! new HeapKind. Named members lower to a `Constant::String` at the
    //! property-access compile path; `Color.rgb(...)` is the `ColorRgbCtor`
    //! builtin.
    use crate::executor::tests::test_utils::{eval, eval_result};

    #[test]
    fn color_red_member_is_string() {
        let v = eval(r#"Color.red"#);
        assert_eq!(v.as_str(), Some("red"));
    }

    #[test]
    fn border_rounded_member_is_string() {
        let v = eval(r#"Border.rounded"#);
        assert_eq!(v.as_str(), Some("rounded"));
    }

    #[test]
    fn chart_type_member_is_string() {
        let v = eval(r#"ChartType.candlestick"#);
        assert_eq!(v.as_str(), Some("candlestick"));
    }

    #[test]
    fn color_rgb_call_builds_spec_string() {
        let v = eval(r#"Color.rgb(255, 0, 0)"#);
        assert_eq!(v.as_str(), Some("rgb(255,0,0)"));
    }

    #[test]
    fn color_rgb_out_of_range_rejects() {
        let err = eval_result(r#"Color.rgb(300, 0, 0)"#);
        assert!(err.is_err(), "out-of-range channel must reject");
    }

    #[test]
    fn border_member_consumable_by_border_method() {
        // The string carrier flows into the existing string-typed
        // `.border(style)` method and produces a Content (Table) slot —
        // proving Border.rounded is consumable with no new carrier.
        use shape_value::{HeapKind, NativeKind};
        let v = eval_result(r#"Content.table(["A"], [["1"]]).border(Border.rounded)"#)
            .expect(".border(Border.rounded) must succeed");
        assert_eq!(v.kind, NativeKind::Ptr(HeapKind::Content));
    }

    #[test]
    fn unknown_style_spec_member_rejects() {
        let err = eval_result(r#"Color.bogus"#);
        assert!(err.is_err(), "Color.bogus must reject cleanly");
    }
}

#[cfg(test)]
mod sc2_style_chain_tests {
    //! SC2 (R8 — supervisor): style chain + table/chart builder methods.
    //! The style methods clone-mutate-rewrap the underlying `ContentNode`
    //! via the `content.rs` `with_*` helpers; `toString` renders through
    //! the `TerminalRenderer`. Chart-builder methods stay surfaced (D4
    //! deferred to v0.4).
    use crate::executor::tests::test_utils::{eval, eval_result};
    use shape_value::{HeapKind, NativeKind};

    #[test]
    fn bold_returns_content_slot() {
        let v = eval_result(r#"Content.text("hi").bold()"#)
            .expect(".bold() must succeed on a Text receiver");
        assert_eq!(v.kind, NativeKind::Ptr(HeapKind::Content));
    }

    #[test]
    fn style_chain_composes() {
        let v = eval_result(r#"Content.text("hi").bold().italic().underline().dim()"#)
            .expect("style chain must compose");
        assert_eq!(v.kind, NativeKind::Ptr(HeapKind::Content));
    }

    #[test]
    fn fg_consumes_sc1_color_named() {
        let v = eval_result(r#"Content.text("hi").fg(Color.red)"#)
            .expect(".fg(Color.red) must succeed");
        assert_eq!(v.kind, NativeKind::Ptr(HeapKind::Content));
    }

    #[test]
    fn bg_consumes_sc1_color_rgb() {
        let v = eval_result(r#"Content.text("hi").bg(Color.rgb(255, 0, 0))"#)
            .expect(".bg(Color.rgb(...)) must succeed");
        assert_eq!(v.kind, NativeKind::Ptr(HeapKind::Content));
    }

    #[test]
    fn fg_rejects_unknown_color() {
        let err = eval_result(r#"Content.text("hi").fg("octarine")"#);
        assert!(err.is_err(), "unknown fg color must reject");
    }

    #[test]
    fn to_string_renders_to_string_kind() {
        let v = eval(r#"Content.text("plain").toString()"#);
        assert_eq!(v.as_str(), Some("plain"));
    }

    #[test]
    fn to_string_renders_styled_text() {
        // Bold renders ANSI escape codes around the text via the
        // TerminalRenderer; assert the body text survives.
        let v = eval(r#"Content.text("boldtext").bold().toString()"#);
        let s = v.as_str().expect("toString returns a string");
        assert!(s.contains("boldtext"), "rendered string must contain the body, got {:?}", s);
    }

    #[test]
    fn table_border_then_max_rows_chains() {
        let v = eval_result(
            r#"Content.table(["A"], [["1"], ["2"], ["3"]]).border(Border.rounded).max_rows(2)"#,
        )
        .expect(".border().max_rows() must chain on a Table receiver");
        assert_eq!(v.kind, NativeKind::Ptr(HeapKind::Content));
    }

    #[test]
    fn max_rows_rejects_on_text_receiver() {
        let err = eval_result(r#"Content.text("hi").max_rows(2)"#);
        assert!(err.is_err(), "max_rows on Text receiver must reject");
    }

    #[test]
    fn chart_builder_methods_remain_surfaced() {
        // Content.chart is deferred (D4 v0.4); the chart-builder methods
        // stay NotImplemented(SURFACE). Calling .title on a non-chart
        // Content surfaces rather than mutating.
        let err = eval_result(r#"Content.text("hi").title("x")"#);
        assert!(err.is_err(), "title() must remain surfaced (chart deferred)");
    }
}
