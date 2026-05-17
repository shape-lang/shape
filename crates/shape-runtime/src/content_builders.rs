//! Content namespace builder functions.
//!
//! Phase 1.B (ADR-006 §2.7.4 audit-accuracy ruling): the pre-bulldozer
//! builders decoded `&[ValueWord]` arguments via tag-bit dispatch
//! (`as_str()`, `as_any_array()`, `to_generic()`) and constructed
//! results via `ValueWord::from_content` / `from_string`. The kind-
//! threaded rebuild lands in Phase 2c alongside the broader content-
//! tree marshalling migration; until then, every builder returns a
//! deferred error rather than emit a partial / wrong-typed
//! `ContentNode` payload.
//!
//! shape-vm consumers (`vm_impl/builtins.rs:556` etc.) call these
//! handlers directly and break in the next-session shape-vm cleanup
//! workstream per ADR-006 §2.7.5.

use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;

fn deferred(name: &str) -> ShapeError {
    ShapeError::RuntimeError {
        message: format!(
            "{}: pending Phase 2c content-tree kind threading — see ADR-006 §2.7.4",
            name
        ),
        location: None,
    }
}

pub fn content_text(_args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(deferred("Content.text"))
}

pub fn content_table(_args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(deferred("Content.table"))
}

pub fn content_chart(_args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(deferred("Content.chart"))
}

pub fn content_code(_args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(deferred("Content.code"))
}

pub fn content_kv(_args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(deferred("Content.kv"))
}

pub fn content_fragment(_args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(deferred("Content.fragment"))
}

pub fn color_named(_name: &str) -> Result<KindedSlot> {
    Err(deferred("Color.named"))
}

pub fn color_rgb(_args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(deferred("Color.rgb"))
}

pub fn border_named(_name: &str) -> Result<KindedSlot> {
    Err(deferred("Border.named"))
}

pub fn chart_type_named(_name: &str) -> Result<KindedSlot> {
    Err(deferred("ChartType.named"))
}

pub fn align_named(_name: &str) -> Result<KindedSlot> {
    Err(deferred("Align.named"))
}
