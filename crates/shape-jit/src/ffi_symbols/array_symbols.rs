//! Array FFI Symbol Registration — surface-and-stop.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! The full array FFI surface (`jit_new_array`, `jit_array_get`,
//! `jit_array_push`, `jit_array_pop`, `jit_array_zip`,
//! `jit_array_first`/`last`/`min`/`max`, `jit_slice`, `jit_range`,
//! `jit_make_range`, `jit_array_filled`, `jit_array_reverse`,
//! `jit_array_push_*`, `jit_hof_array_alloc`/`push`, `jit_array_info`)
//! all wrapped the deleted `JitArray` heap layout. `crate::ffi::array`
//! no longer exports the implementations, so the import + symbol
//! registration here would never resolve. The kinded rebuild
//! re-introduces these as `TypedArray<T>`-aware entries per ADR-006
//! §2.7.6/Q8 with element kinds threaded from the JIT call signature
//! per §2.7.5.
//!
//! Until the rebuild lands (W11 / deeper Phase-2c), both
//! `register_array_symbols` and `declare_array_functions` are no-ops:
//! Cranelift `call` sites that emit array-FFI references will fail
//! the link step with an unresolved-symbol error pointing back to
//! this module. That is the deletion-fate signal §5 calls for.

use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::FuncId;
use std::collections::HashMap;

/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): array FFI
/// registrations are gated on the kinded `TypedArray<T>` rebuild.
/// This is intentionally a no-op — see module docs.
pub fn register_array_symbols(_builder: &mut JITBuilder) {
    // No-op until the kinded TypedArray<T> array-FFI rebuild lands.
    // Callers that emit Cranelift `call jit_array_*` will surface at
    // link time per W10 jit-playbook §5.
}

/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): array FFI
/// declarations are gated on the kinded `TypedArray<T>` rebuild.
/// This is intentionally a no-op — see module docs.
pub fn declare_array_functions(_module: &mut JITModule, _ffi_funcs: &mut HashMap<String, FuncId>) {
    // No-op until the kinded TypedArray<T> array-FFI rebuild lands.
}
