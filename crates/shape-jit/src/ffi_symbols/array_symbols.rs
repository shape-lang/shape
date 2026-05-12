//! Array FFI Symbol Registration.
//!
//! ## Route A close (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! Q15 resolved Route A: kinded per-element-kind `Arc<TypedArrayData>`
//! monomorphization. The deleted kind-blind `jit_new_array` /
//! `jit_array_push_elem` symbols are not re-registered here — the
//! kinded surface is the existing `v2_array_new_<f64,i64,i32,bool>` +
//! `v2_array_push` family (registered in
//! `ffi_symbols/v2_symbols.rs::register_v2_symbols`). MIR call sites
//! that lack a proven element kind surface-and-stop at JIT compile
//! time with the `Route A surface-and-stop` marker (see
//! `mir_compiler/statements.rs::ArrayStore` / `EnumStore` and
//! `mir_compiler/rvalues.rs::Aggregate`).
//!
//! The full §2.7.14 cascade list (`jit_array_get/push/pop`,
//! `jit_array_first/last/min/max`, `jit_slice`, `jit_range`,
//! `jit_make_range`, `jit_array_filled`, `jit_array_reverse`,
//! `jit_array_zip`, `jit_hof_array_alloc/push`, `jit_array_info`)
//! routes through the v2 typed-array primitives at the MIR codegen
//! layer: `try_emit_v2_array_method` in `mir_compiler/v2_array.rs`
//! handles `len` / `push` / `sum` / `min` / `max` / `mean` /
//! `scale` / `addScalar` / `addArray` / `mulArray` directly against
//! the `TypedArray<T>` layout. Cascade entries beyond that set
//! (slice, reverse, zip, ...) remain a §2.7.14 follow-up — they're
//! reachable via the generic `jit_call_method` trampoline path until
//! per-method Cranelift codegen lands, which is W11-jit-new-array's
//! follow-up scope.
//!
//! ## Forbidden under any future expansion
//!
//! - `JitArray` revival under any renamed shape (CLAUDE.md "Renames
//!   to refuse on sight" — broader-family regex).
//! - Bool-default fallback for unknown element kind (CLAUDE.md
//!   "Forbidden rationalizations").
//! - `tag_bits`-based element decoder (CLAUDE.md "Forbidden Patterns" #4).
//! - Mixed-route incrementalism across the §2.7.14 cascade list.

use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::FuncId;
use std::collections::HashMap;

/// Route A close (W11-jit-new-array): the kind-blind array-FFI surface
/// is replaced by the kinded `v2_array_new_<kind>` allocators and the
/// `v2_array_push` size-dispatched push helper, both registered by
/// `register_v2_symbols`. No symbols to register at this layer.
pub fn register_array_symbols(_builder: &mut JITBuilder) {
    // No-op (see module docs).
}

/// Route A close (W11-jit-new-array): the kind-blind array-FFI surface
/// is replaced by the kinded `v2_array_new_<kind>` declarations in
/// `declare_v2_functions`. No declarations at this layer.
pub fn declare_array_functions(_module: &mut JITModule, _ffi_funcs: &mut HashMap<String, FuncId>) {
    // No-op (see module docs).
}
