//! Built-in functions for multi-table analysis.
//!
//! Per ADR-006 §2.7.5 (cross-crate ABI policy), `align_tables` migrates
//! to the `&[KindedSlot]` signature. The shape-jit consumer at
//! `crates/shape-jit/src/ffi_symbols/data_access/mod.rs:95` was passing
//! the legacy `(ctx, &[ValueWord])` shape and breaks in the next-session
//! shape-jit cleanup workstream — Phase 1.B does not preserve the legacy
//! signature on the runtime side just to keep shape-jit compiling
//! through this session. The shape-vm cluster picks up the consumer
//! migration alongside its own KindedSlot-threading work.
//!
//! Phase 1.B body shim: per ADR-006 §2.7.4 audit-accuracy ruling, the
//! pre-bulldozer body decoded `&ValueWord` arrays via tag-bit dispatch
//! (`as_any_array()`, `as_str()`, `as_f64()`, etc.) that no longer
//! exist. The kind-threaded rebuild (per-position `NativeKind` from
//! the registered schema) lands in Phase 2c. Until then the body
//! returns a deferred error — same shape as the intrinsic stubs.

use crate::context::ExecutionContext;
use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;

/// Align multiple datasets.
///
/// **Migration deferred** pending ModuleContext-access architectural
/// decision (`ExecutionContext::get_current_timeframe()` not exposed via
/// `&ModuleContext`) AND shape-jit cleanup workstream (cross-crate
/// consumer migration). Phase 1.B body returns a deferred error; the
/// signature already takes `&[KindedSlot]` per ADR-006 §2.7.5.
pub fn align_tables(_ctx: &mut ExecutionContext, _args: &[KindedSlot]) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "align_tables: pending Phase 2c kind threading + ModuleContext-access decision — see ADR-006 §2.7.4 / §2.7.5".to_string(),
        location: None,
    })
}

/// Correlation between two series (placeholder; returns 0.0).
///
/// **Migration deferred** alongside `align_tables` to keep the
/// `multi_table` file in a coherent partial-deferred state.
pub fn correlation(_ctx: &mut ExecutionContext, _args: &[KindedSlot]) -> Result<KindedSlot> {
    Ok(KindedSlot::from_number(0.0))
}
