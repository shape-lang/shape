//! Shape Python language runtime extension.
//!
//! Provides a `shape.language_runtime` capability that embeds CPython
//! for executing `foreign "python" { ... }` blocks in Shape programs.
//!
//! # ABI Exports
//!
//! - `shape_plugin_info()` -- plugin metadata
//! - `shape_abi_version()` -- ABI version tag
//! - `shape_capability_manifest()` -- declares LanguageRuntime capability
//! - `shape_capability_vtable(contract, len)` -- generic vtable dispatch
//! - `shape_language_runtime_vtable()` -- direct vtable accessor

pub mod arrow_bridge;
pub mod error_mapping;
pub mod marshaling;
pub mod runtime;

use shape_abi_v1::{
    ABI_VERSION, CAPABILITY_LANGUAGE_RUNTIME, CapabilityDescriptor, CapabilityKind,
    CapabilityManifest, ErrorModel, LanguageRuntimeVTable, PluginInfo, PluginType,
};
use std::ffi::c_void;

// ============================================================================
// Plugin Metadata
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn shape_plugin_info() -> *const PluginInfo {
    static INFO: PluginInfo = PluginInfo {
        name: c"python".as_ptr(),
        version: c"0.1.0".as_ptr(),
        plugin_type: PluginType::DataSource, // closest existing variant
        description: c"Python language runtime for foreign function blocks".as_ptr(),
    };
    &INFO
}

#[unsafe(no_mangle)]
pub extern "C" fn shape_abi_version() -> u32 {
    ABI_VERSION
}

// ============================================================================
// Capability Manifest
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn shape_capability_manifest() -> *const CapabilityManifest {
    static CAPABILITIES: [CapabilityDescriptor; 1] = [CapabilityDescriptor {
        kind: CapabilityKind::LanguageRuntime,
        contract: c"shape.language_runtime".as_ptr(),
        version: c"1".as_ptr(),
        flags: 0,
    }];
    static MANIFEST: CapabilityManifest = CapabilityManifest {
        capabilities: CAPABILITIES.as_ptr(),
        capabilities_len: CAPABILITIES.len(),
    };
    &MANIFEST
}

// ============================================================================
// VTable
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn shape_language_runtime_vtable() -> *const LanguageRuntimeVTable {
    static VTABLE: LanguageRuntimeVTable = LanguageRuntimeVTable {
        init: Some(runtime::python_init),
        register_types: Some(runtime::python_register_types),
        compile: Some(runtime::python_compile),
        invoke: Some(runtime::python_invoke),
        dispose_function: Some(runtime::python_dispose_function),
        language_id: Some(runtime::python_language_id),
        get_lsp_config: Some(runtime::python_get_lsp_config),
        free_buffer: Some(runtime::python_free_buffer),
        drop: Some(runtime::python_drop),
        error_model: ErrorModel::Dynamic,
    };
    &VTABLE
}

#[unsafe(no_mangle)]
pub extern "C" fn shape_capability_vtable(
    contract: *const u8,
    contract_len: usize,
) -> *const c_void {
    if contract.is_null() {
        return std::ptr::null();
    }
    let contract = unsafe { std::slice::from_raw_parts(contract, contract_len) };
    if contract == CAPABILITY_LANGUAGE_RUNTIME.as_bytes() {
        shape_language_runtime_vtable() as *const c_void
    } else {
        std::ptr::null()
    }
}
