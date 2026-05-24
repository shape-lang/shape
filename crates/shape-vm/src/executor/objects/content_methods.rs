//! Content method dispatch for ContentNode values.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `Content` *is* a surviving `HeapKind` variant
//! (`Content(Arc<ContentNode>)` per ADR-006 §2.3 +
//! `crates/shape-value/src/heap_variants.rs`), so a kind-correct rewrite
//! of these handlers is mechanical: receiver is
//! `NativeKind::Ptr(HeapKind::Content)`, dispatch via
//! `slot.as_heap_value()` + `HeapValue::Content(arc)` match per Q8, push
//! the result as `Arc::into_raw(Arc<ContentNode>) as u64` with kind
//! `NativeKind::Ptr(HeapKind::Content)` (string return arms push
//! `NativeKind::String`).
//!
//! Migration is blocked on the MethodHandler ABI rewrite to
//! `&mut [KindedSlot] -> Result<KindedSlot>` (cluster
//! E-builtins-backlog, Wave 5b template, commit `fa2bafc`). The
//! pre-Wave-6 implementation imported the deleted
//! `shape_value::{ValueWord, ValueWordExt, ValueWordDisplay}` surface,
//! the deleted `ValueWord::from_content` / `from_string` /
//! `from_raw_bits` / `clone_from_bits` constructors, and the
//! `objects::raw_helpers::{extract_content, extract_number_coerce,
//! extract_str}` helpers (deleted in cluster D-raw-helpers — only the
//! FilterExpr extractor remains). The macro-generated runtime delegators
//! (`v2_content_border`, `v2_content_series`, etc.) call into
//! `shape_runtime::content_methods::call_content_method` which itself
//! takes `ValueWord` arguments — that crate-boundary signature is also
//! awaiting the kinded redesign per playbook §8 cross-cluster cascade.
//! Per playbook §4 #1 / #9 a Bool-default kinded shim is forbidden; per
//! §7.4 the correct response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{HeapKind, KindedSlot, NativeKind, VMError};

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Content.{}(): MethodHandler ABI needs kinded migration \
         (cluster E-builtins-backlog, Wave 5b template); receiver kind \
         NativeKind::Ptr(HeapKind::Content), dispatch via \
         slot.as_heap_value() + HeapValue::Content match per ADR-006 \
         §2.7.6 / Q8. Runtime delegators (border/series/title/etc.) also \
         depend on the shape-runtime crate-boundary kinded redesign per \
         playbook §8 cross-cluster cascade.",
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
fn recv_content<'a>(args: &'a [KindedSlot], method: &str) -> Result<&'a shape_value::content::ContentNode, VMError> {
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

pub fn v2_content_bold(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("bold"))
}

pub fn v2_content_italic(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("italic"))
}

pub fn v2_content_underline(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("underline"))
}

pub fn v2_content_dim(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("dim"))
}

pub fn v2_content_fg(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("fg"))
}

pub fn v2_content_bg(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("bg"))
}

pub fn v2_content_to_string(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toString"))
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
    let headers = crate::executor::vm_impl::builtins::read_string_array(&args[1])
        .ok_or_else(|| {
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
    use crate::executor::v2_handlers::v2_array_detect::{
        as_v2_typed_array, read_element,
    };
    if args[1].kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Content.row(): cells argument must be an Array, got kind {:?}",
            args[1].kind
        )));
    }
    let view = as_v2_typed_array(args[1].slot.raw(), args[1].kind)
        .ok_or_else(|| {
            VMError::RuntimeError(
                "Content.row(): cells array has invalid v2 header".to_string(),
            )
        })?;
    let formatter = crate::executor::printing::ValueFormatter::new(
        &vm.program.type_schema_registry,
    );
    let mut cells: Vec<shape_value::content::ContentNode> =
        Vec::with_capacity(view.len as usize);
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Content.row(): failed to read cell at index {}",
                i
            ))
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
    let formatter = crate::executor::printing::ValueFormatter::new(
        &vm.program.type_schema_registry,
    );
    let rendered = formatter.format_kinded(&args[2]);
    let value_node = shape_value::content::ContentNode::plain(rendered);
    match node {
        shape_value::content::ContentNode::KeyValue(pairs) => {
            let mut new_pairs = pairs.clone();
            new_pairs.push((key.to_string(), value_node));
            Ok(content_slot(
                shape_value::content::ContentNode::KeyValue(new_pairs),
            ))
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

pub fn v2_content_max_rows(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("max_rows"))
}

pub fn v2_content_max_rows_camel(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("maxRows"))
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
