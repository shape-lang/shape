//! Standard library modules for the Shape runtime.
//!
//! Each submodule implements a `std::*` namespace accessible from Shape code.
//! Modules follow the [`ModuleExports`](crate::module_exports::ModuleExports)
//! pattern established by `stdlib_time.rs`.
//!
//! All I/O-capable modules are tagged with required capabilities in
//! [`capability_tags`] and enforced at compile time via the permission system.

pub mod archive;
pub mod arrow_module;
pub mod byte_utils;
pub mod capability_tags;
pub mod compress;
pub mod crypto;
pub mod csv_module;
pub mod deterministic;
pub mod env;
pub mod file;
pub mod http;
pub mod json;
pub mod msgpack_module;
pub mod regex;
pub mod runtime_policy;
pub mod toml_module;
pub mod unicode;
pub mod virtual_fs;
pub mod xml;
pub mod yaml;

/// Return all shipped native stdlib modules defined in `shape-runtime`.
///
/// This is the canonical registry — every `create_*_module()` in the stdlib,
/// `stdlib_time`, and `stdlib_io` trees is called exactly once. VM-side
/// modules (state, transport, remote) live in `shape-vm` and must be added
/// separately by the VM.
pub fn all_stdlib_modules() -> Vec<crate::module_exports::ModuleExports> {
    vec![
        regex::create_regex_module(),
        http::create_http_module(),
        crypto::create_crypto_module(),
        env::create_env_module(),
        json::create_json_module(),
        toml_module::create_toml_module(),
        yaml::create_yaml_module(),
        xml::create_xml_module(),
        compress::create_compress_module(),
        archive::create_archive_module(),
        unicode::create_unicode_module(),
        csv_module::create_csv_module(),
        msgpack_module::create_msgpack_module(),
        file::create_file_module(),
        arrow_module::create_arrow_module(),
        crate::stdlib_time::create_time_module(),
        crate::stdlib_io::create_io_module(),
        crate::intrinsics::vector::create_vector_intrinsics_module(),
        crate::intrinsics::math::create_math_intrinsics_module(),
        crate::intrinsics::array_transforms::create_array_transforms_module(),
        crate::intrinsics::rolling::create_rolling_intrinsics_module(),
        crate::intrinsics::statistical::create_statistical_intrinsics_module(),
        crate::intrinsics::random::create_random_intrinsics_module(),
        crate::intrinsics::distributions::create_distributions_intrinsics_module(),
        crate::intrinsics::convolution::create_convolution_intrinsics_module(),
        crate::intrinsics::stochastic::create_stochastic_intrinsics_module(),
        crate::intrinsics::matrix::create_matrix_intrinsics_module(),
        crate::intrinsics::fft::create_fft_intrinsics_module(),
        crate::intrinsics::recurrence::create_recurrence_intrinsics_module(),
    ]
}
