//! Standard library modules for the Shape runtime.
//!
//! Each submodule implements a `std::*` namespace accessible from Shape code.
//! Modules follow the [`ModuleExports`](crate::module_exports::ModuleExports)
//! pattern established by `stdlib_time.rs`.
//!
//! All I/O-capable modules are tagged with required capabilities in
//! [`capability_tags`] and enforced at compile time via the permission system.

pub mod archive;
pub mod byte_utils;
pub mod capability_tags;
pub mod compress;
pub mod crypto;
pub mod csv_module;
pub mod deterministic;
pub mod env;
pub mod file;
pub mod helpers;
pub mod http;
pub mod json;
pub mod msgpack_module;
pub mod parallel;
pub mod regex;
pub mod runtime_policy;
pub mod set_module;
pub mod toml_module;
pub mod unicode;
pub mod virtual_fs;
pub mod xml;
pub mod yaml;
