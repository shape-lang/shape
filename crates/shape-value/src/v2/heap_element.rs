//! v2-raw HeapHeader-equipped element carrier trait.
//!
//! ## Purpose
//!
//! `HeapElement` is the compile-time trait that constrains the element type `T`
//! of a `TypedArray<*const T>` instantiation to a HeapHeader-equipped v2-raw
//! carrier. Implementors (`StringObj`, `DecimalObj`, ...) own the per-T drop
//! semantics: `release_elem` decrements the refcount and fully deallocates
//! when the count reaches zero.
//!
//! ## Authority
//!
//! Per ADR-006 §2.7.24 Q25.A SUPERSEDED + R20 S2-prime audit deliverable (b)
//! §4.1.B decision: option (a) — `HeapElement` trait dispatch.
//!
//! The trait is the compile-time-monomorphized per-T release dispatcher for
//! `TypedArray<*const T>::drop_array_heap`. Bodies dispatch via the Rust type
//! system; no runtime `NativeKind` parameter, no `is_heap()` probe, no
//! Bool-default fallback.
//!
//! ## Discipline
//!
//! - One method (`release_elem`), one `*const Self` parameter.
//! - Implementor types must have `HeapHeader` at offset 0 (so
//!   `v2_release(&(*ptr).header)` is sound). The `#[repr(C)]` + first-field-
//!   header invariant per `StringObj` precedent enforces this structurally.
//! - The trait is `unsafe` because callers must guarantee `ptr` validity
//!   (live allocation, no concurrent free).
//!
//! ## Forbidden
//!
//! Per ADR-006 §2.7.24 Q25.A SUPERSEDED + audit §4.1.B.3:
//!
//! - `HeapElement::release_elem` taking a `NativeKind` parameter — refused;
//!   the trait dispatches via the Rust type system, not via a runtime kind
//!   probe. This is the §2.7.7 #4/#7 forbidden pattern (`tag_bits`-style
//!   runtime dispatch on the kind discriminator) at the per-element layer.
//! - Bool-default fallback in `release_elem` body — surface-and-stop with
//!   `NotImplemented(SURFACE: ...)` at the construction-site when an inner
//!   payload's drop semantics are unproven. Per §2.7.7 #9.
//! - Implementing `HeapElement` for non-HeapHeader-equipped types
//!   (e.g. `Arc<>`-wrapped storage). The trait is structurally constrained
//!   to types with `HeapHeader` at offset 0; implementing it for an
//!   `Arc<>`-wrapped struct would fail the `(*ptr).header` field access
//!   at compile time.
//! - Renaming `HeapElement` to defection-attractor framing (heap-bridge,
//!   elem-helper, release-translator, etc.) — per CLAUDE.md broader-family
//!   regex. The trait describes a structural property (this T lives on
//!   the v2-raw heap), not a dispatch role.

/// v2-raw HeapHeader-equipped element carrier trait.
///
/// Implementors are `#[repr(C)]` structs with `HeapHeader` at offset 0. The
/// trait dispatches per-T release at compile time via the Rust type system;
/// `TypedArray<*const T: HeapElement>::drop_array_heap` calls
/// `T::release_elem(elem_ptr)` for each stored element.
///
/// # Safety
///
/// Implementors must guarantee:
/// 1. `Self` is `#[repr(C)]` with a `HeapHeader` field at offset 0.
/// 2. `release_elem(ptr)` is sound when `ptr` points to a live `Self`
///    allocation (one that the implementor's allocator produced and that
///    has not yet been freed).
/// 3. `release_elem(ptr)` decrements the refcount via `v2_release` and, if
///    `v2_release` returns true, fully deallocates the allocation
///    (including any nested payload buffers per the implementor's drop
///    semantics).
pub unsafe trait HeapElement {
    /// Decrement the reference count of `*ptr`. If the refcount reaches
    /// zero, fully deallocate the object (including any nested payload
    /// buffers per the implementor's drop semantics).
    ///
    /// # Safety
    /// `ptr` must point to a valid, live `Self` allocated via the
    /// implementor's canonical v2-raw allocator. After this call returns,
    /// `ptr` must not be dereferenced (the allocation may have been freed).
    unsafe fn release_elem(ptr: *const Self);
}
