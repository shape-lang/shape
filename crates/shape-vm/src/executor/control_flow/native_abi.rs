//! Native C ABI linking and invocation for `extern C` foreign functions.
//!
//! ADR-006 §2.7.4 / §2.7.5 SURFACE: this module is a forbidden-pattern
//! carrier — every input / output path consumed the deleted `ValueWord`
//! / `tag_bits` / `as_heap_ref` / `vmarray_from_vec` / `as_native_*` /
//! `value_to_*` surfaces. The C ABI extension contract (libffi-driven
//! call) stays raw bits per §2.7.5 cross-crate ABI policy, but the
//! marshalling between the VM stack (kinded) and the FFI argument
//! buffer (raw) is the consumer-side migration target. Per the
//! B11-control-flow-heap dispatch the body is stubbed pending phase-2c
//! rebuild.
//!
//! The pre-rebuild surface (now deleted):
//!
//! - `link_native_function(spec, layouts, cache)` — parsed the textual
//!   C signature, validated layout references, opened the dynamic
//!   library, resolved the symbol, built a libffi `Cif`. The signature
//!   parsing + layout resolution path is independent of `ValueWord`
//!   and could be preserved verbatim — but `NativeLinkedFunction`
//!   itself is the public type used by `executor/mod.rs:222` for the
//!   `ForeignFunctionHandle::Native(Arc<NativeLinkedFunction>)`
//!   variant; its fields stay unchanged.
//!
//! - `invoke_linked_function(linked, args: &[ValueWord], raw_invoker,
//!   vm_stack: &mut [ValueWord])` — encoded VM args into FFI buffers
//!   per `CType` arm, called libffi, then decoded the return value
//!   back into `ValueWord` via `from_native_*` / `from_string` /
//!   `from_array(vmarray_from_vec(...))`. The mutable-slice writeback
//!   path also wrote back via `vmarray_from_vec`. All of the above
//!   are deleted ValueWord-shape constructors; rebuild needs `&[KindedSlot]`
//!   in / `KindedSlot` out, with `&mut [u64]` + parallel `&mut
//!   [NativeKind]` for the writeback path on the §2.7.7 stack ABI.
//!
//! Rebuild plan: keep the `CType` parser, `CSignature`,
//! `NativeTypeLayout` resolution, libffi `Cif` construction, and all
//! the type-classification helpers (none of those touch ValueWord).
//! Migrate `invoke_linked_function` to the kinded API; the
//! `RawCallableInvoker` extension contract on the §2.7.5 FFI side
//! stays `(*mut c_void, &u64, &[u64]) -> Result<u64, String>` per
//! `module_exports.rs:21`.

use crate::bytecode::{NativeAbiSpec, NativeStructLayoutEntry};
use shape_runtime::module_exports::RawCallableInvoker;
use shape_value::KindedSlot;
use std::collections::HashMap;
use std::sync::Arc;

/// Phase-2c rebuild placeholder. The pre-rebuild type carried libffi
/// `Cif` + `CodePtr` + parsed `CSignature` + a layout map + an
/// `Arc<Library>` keep-alive — that internal layout is reconstructed
/// in phase-2c on top of the kinded marshalling path. The placeholder
/// keeps the public type name + `Arc`-wrap visible to
/// `executor/mod.rs:222`'s `ForeignFunctionHandle::Native(Arc<...>)`
/// variant.
pub struct NativeLinkedFunction {
    /// SURFACE: the field bag is kept zero-sized so attempted
    /// instantiation surfaces a NotImplemented at the link site rather
    /// than papering over with a dead body that compiles into wrong
    /// runtime behavior.
    _phase_2c_placeholder: (),
}

/// Phase-2c surface stub. The pre-rebuild body parsed the textual C
/// signature (`fn(<params>) -> <ret>`), validated `cview<...>` /
/// `cmut<...>` / `cslice<...>` layout references against the
/// program-supplied `NativeStructLayoutEntry` table, opened the dynamic
/// library via `libloading`, resolved the symbol, and built a libffi
/// `Cif`. The signature parser and layout resolver themselves do not
/// touch `ValueWord` — they're preserved as a starting point for the
/// phase-2c rebuild. The library opening / symbol resolution path is
/// also kind-independent.
pub fn link_native_function(
    _spec: &NativeAbiSpec,
    _native_layouts: &[NativeStructLayoutEntry],
    _library_cache: &mut HashMap<String, Arc<libloading::Library>>,
) -> Result<NativeLinkedFunction, String> {
    Err(format!(
        "native_abi::link_native_function: phase-2c — extern C FFI rebuild (ADR-006 §2.7.4 / §2.7.5)"
    ))
}

/// Phase-2c surface stub. Pre-rebuild body marshalled `&[ValueWord]`
/// args into libffi argument buffers per `CType` arm, called libffi
/// via `Cif::call::<T>`, and decoded the return value into a
/// `ValueWord` via the deleted `from_native_*` / `from_string` /
/// `from_array(vmarray_from_vec(...))` constructors. Rebuild target
/// signature: `&[KindedSlot] -> Result<KindedSlot, String>` with
/// per-`NativeKind` arg dispatch at the body site (§2.7.6) and a
/// kinded result whose `NativeKind` is sourced from the FFI return
/// `CType` (post-proof per §2.7.5.1). The `vm_stack` writeback path
/// for `cmut_slice<T>` argument writebacks rebuilds against the
/// §2.7.7 parallel `(stack: &mut [u64], kinds: &mut [NativeKind])`
/// pair.
pub fn invoke_linked_function(
    _linked: &NativeLinkedFunction,
    _args: &[KindedSlot],
    _raw_invoker: Option<RawCallableInvoker>,
    _vm_stack_data: Option<&mut [u64]>,
    _vm_stack_kinds: Option<&mut [shape_value::NativeKind]>,
) -> Result<KindedSlot, String> {
    Err(format!(
        "native_abi::invoke_linked_function: phase-2c — extern C FFI rebuild (ADR-006 §2.7.4 / §2.7.5)"
    ))
}
