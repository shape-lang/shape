//! Runtime detection and uniform access for typed arrays — Phase 2c stubs.
//!
//! ## Status
//!
//! This module previously wrapped a `_pad`-byte element-type discriminant
//! plus a `ValueWord`-keyed dispatch surface (`as_native_typed_array`,
//! `read_element`, `write_element`, `push_element`, `pop_element`,
//! `sum_elements`, `avg_elements`, `min_elements`, `max_elements`,
//! `variance_elements`, `std_elements`, `dot_elements`, `norm_elements`,
//! `count_true_elements`, `any_elements`, `all_elements`, `clone_array`)
//! over the legacy `shape_value::native::typed_array::TypedArray<T>` /
//! `shape_value::native::heap_header::HeapHeader` primitive layer.
//!
//! Both the `ValueWord` carrier and the `shape_value::native::*` typed-array
//! primitive layer were deleted during the strict-typing bulldozer cycles.
//! Every public symbol re-exported through the orphan
//! `executor/objects/typed_array_handlers.rs` (itself not declared as a
//! module anywhere — never compiled) had zero live consumers in the
//! current VM dispatch.
//!
//! ## Disposition (Wave 6.5 substep-2 / Wave-α `D-array-detect`)
//!
//! The file is a **forbidden-helper carrier** with no opcode handlers and
//! no live consumers. Cluster C took the same shape on `typed_handlers/
//! typed_array.rs` (orphan dispatch surface) and replaced every entry with
//! a kinded-API `NotImplemented` stub. Here there are no opcode handlers
//! and no `&mut VirtualMachine` entries — only pure helper functions whose
//! signatures presupposed `ValueWord` / `NativeScalar` / `TypedArray<T>`,
//! every one of which is forbidden under `CLAUDE.md` "Forbidden Patterns".
//!
//! Therefore the helpers are **deleted**, not stubbed: re-introducing them
//! in any form (renamed, gated, "for one edge case") would be a W-series
//! defection of the shape "Renames to refuse on sight" lists in CLAUDE.md.
//! Phase 2c is the natural reentry point if and when a live caller needs
//! a typed-array detection surface — at that point it must be re-emitted
//! on top of the kinded `Arc<TypedArrayData>` model + `HeapValue::TypedArray`
//! dispatch (Q8), never on a parallel discriminator (ADR-005 §1).
//!
//! See `docs/adr/006-value-and-memory-model.md` §2.7 + Q7-Q10, and
//! `CLAUDE.md` "Forbidden Patterns" + "Renames to refuse on sight".
