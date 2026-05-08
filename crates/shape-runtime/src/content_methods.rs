//! Content method dispatch for ContentNode instance methods.
//!
//! Phase 1.B (ADR-006 §2.7.4 audit-accuracy ruling): the pre-bulldozer
//! method handlers decoded `&ValueWord` arguments via tag-bit dispatch
//! (`as_str()`, `as_f64()`, `as_content_ref()`, etc.) and constructed
//! results via `ValueWord::from_content`. The kind-threaded rebuild
//! lands in Phase 2c alongside the broader content-tree marshalling
//! migration. Until then, the dispatcher returns `None` for every
//! method name (so callers fall through to the generic
//! "method not found" error path) and the helper signatures are
//! retained at the [`KindedSlot`] shape per ADR-006 §2.7.5.
//!
//! shape-vm consumers (`vm_impl/builtins.rs`) call these handlers
//! directly and break in the next-session shape-vm cleanup workstream
//! per ADR-006 §2.7.5. Phase 1.B does not preserve the legacy
//! `ValueWord` signature on the runtime side just to keep shape-vm
//! compiling through this session.

use shape_ast::error::Result;
use shape_value::KindedSlot;

/// Look up and call a content method by name.
///
/// Phase 1.B: returns `None` for every method name; the kind-threaded
/// dispatcher lands in Phase 2c. shape-vm consumers will see the
/// "method not found" path until the rebuild.
pub fn call_content_method(
    _method_name: &str,
    _receiver: KindedSlot,
    _args: Vec<KindedSlot>,
) -> Option<Result<KindedSlot>> {
    None
}
