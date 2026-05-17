//! MIR `StatementKind::ObjectStore` schema-id threading.
//!
//! Phase 3 cluster-0 Round 16 W17-narrow-follow-up-A
//! (ADR-006 §2.7.5 stamp-at-compile-time).
//!
//! The MIR lowering pass at `crate::mir::lowering::*` produces
//! `StatementKind::ObjectStore { schema_id: None, .. }` because it
//! does not have direct access to the bytecode compiler's
//! `type_tracker.schema_registry`. This module supplies the post-MIR-
//! lowering back-patch that aligns the MIR-side `ObjectStore` carrier
//! with the parallel bytecode-side `OpCode::NewTypedObject` operand
//! (`Operand::TypedObjectAlloc { schema_id, field_count }`), so the
//! JIT MIR consumer at
//! `crates/shape-jit/src/mir_compiler/statements.rs::
//! StatementKind::ObjectStore` writes the user-declared schema id into
//! `(*ptr).schema_id` rather than the prior
//! `register_predeclared_any_schema` `__predecl_*`-named id.
//!
//! Resolution order at each `ObjectStore` site:
//!
//! 1. Named-struct path — if `mir.local_struct_type_names[container_slot]`
//!    has an entry, look it up in `type_tracker.schema_registry()`.
//!    Matches `Expr::StructLiteral { type_name, .. }` lowering — e.g.
//!    `let t = X {}` in Smoke 3.
//!
//! 2. Anonymous-inline path — for `Expr::Object(...)` with no
//!    spreads, use `register_inline_object_schema(&field_refs)` (the
//!    same helper the bytecode-side compiler calls at
//!    `crates/shape-vm/src/compiler/expressions/collections.rs:396`).
//!    Empty-field-name entries (spread placeholders) are skipped.
//!
//! 3. Otherwise — leave `schema_id: None`. The downstream JIT
//!    consumer surfaces-and-stops on `None` per ADR-006 §2.7.5
//!    forbidden list (no `register_predeclared_any_schema` fallback,
//!    no Bool-default).
//!
//! ADR-006 §2.7.5 forbidden list (refused on sight): no
//! `register_predeclared_any_schema` fallback, no
//! `register_predeclared_any_schema` resurrection under a rename.
//! Once the threading is wired, the JIT-side
//! `register_predeclared_any_schema` call site at
//! `crates/shape-jit/src/mir_compiler/statements.rs:152` is deleted in
//! the same commit.

use crate::mir::types::{MirFunction, StatementKind};
use crate::type_tracking::TypeTracker;

/// Back-patch `StatementKind::ObjectStore { schema_id }` entries in
/// `mir` using `type_tracker.schema_registry()` for named structs and
/// `type_tracker.register_inline_object_schema()` for anonymous
/// inline objects.
///
/// Mutates `mir` in place. Statements that cannot be resolved
/// (unknown struct name, spread-bearing inline object) keep
/// `schema_id: None`; the downstream JIT consumer surfaces-and-stops
/// per the §2.7.5 discipline.
pub(crate) fn back_patch_schema_ids(mir: &mut MirFunction, type_tracker: &mut TypeTracker) {
    for block in &mut mir.blocks {
        for stmt in &mut block.statements {
            if let StatementKind::ObjectStore {
                container_slot,
                field_names,
                schema_id,
                ..
            } = &mut stmt.kind
            {
                if schema_id.is_some() {
                    // Already patched (idempotent — defensive against
                    // re-entry via the multi-pass closure back-patch
                    // loop at `compile_function_body`).
                    continue;
                }

                // Named-struct path: `Expr::StructLiteral` recorded
                // the user-struct type name on the destination slot
                // at MIR lowering time
                // (`record_local_struct_type_name`). Resolve via the
                // bytecode compiler's schema registry — the same
                // registry the bytecode-side
                // `compile_struct_literal` calls
                // (`expressions/collections.rs:800-834`).
                if let Some(type_name) = mir.local_struct_type_names.get(container_slot) {
                    if let Some(schema) = type_tracker.schema_registry().get(type_name) {
                        *schema_id = Some(schema.id);
                        continue;
                    }
                    // Named struct on the slot but no registered
                    // schema — leave `None` and let the JIT consumer
                    // surface-and-stop with the producer-side
                    // diagnostic.
                    continue;
                }

                // Anonymous-inline path: `Expr::Object` (no struct
                // type name) lowers to `ObjectStore { field_names, .. }`
                // where every entry has a non-empty `name`. Mirrors
                // the bytecode-side
                // `register_inline_object_schema` call at
                // `expressions/collections.rs:396` (typed variant
                // unavailable here — typed fields require expression
                // inference state the MIR back-patch does not carry).
                //
                // Skip entries with empty names (spread placeholders);
                // an `ObjectStore` mixing spreads with named fields
                // cannot be resolved with this simple shape and is
                // left `None` for surface-and-stop.
                let has_spread = field_names.iter().any(|n| n.is_empty());
                if has_spread {
                    continue;
                }
                let field_refs: Vec<&str> = field_names.iter().map(|s| s.as_str()).collect();
                // `register_inline_object_schema` is idempotent: a
                // second call with the same field ordering returns
                // the existing schema id (the registry's
                // `lookup_predeclared_id_by_field_order` fast path).
                let sid = type_tracker.register_inline_object_schema(&field_refs);
                *schema_id = Some(sid);
            }
        }
    }
}
