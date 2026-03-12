use std::path::PathBuf;

pub mod add_cmd;
pub mod build_cmd;
pub mod check_cmd;
pub mod doctest_cmd;
pub mod expand_comptime_cmd;
pub mod ext_cmd;
pub mod info_cmd;
pub mod jit_cmd;
pub mod keys_cmd;
pub mod login_cmd;
pub mod publish_cmd;
pub mod register_cmd;
pub mod remove_cmd;
pub mod repl_cmd;
pub mod schema_cmd;
pub mod script_cmd;
pub mod search_cmd;
pub mod serve_cmd;
pub mod snapshot_cmd;
pub mod tree_cmd;
pub mod tui_cmd;
pub mod wire_serve_cmd;

// Re-export command entry points
pub use add_cmd::run_add;
pub use check_cmd::run_check;
pub use build_cmd::run_build;
pub use doctest_cmd::run_doctest;
pub use expand_comptime_cmd::run_expand_comptime;
pub use ext_cmd::{run_ext_install, run_ext_list, run_ext_remove};
pub use info_cmd::run_info;
pub use jit_cmd::run_jit_parity;
pub use keys_cmd::{run_keys_generate, run_keys_list, run_keys_trust, run_sign, run_verify};
pub use login_cmd::run_login;
pub use publish_cmd::run_publish;
pub use register_cmd::run_register;
pub use remove_cmd::run_remove;
pub use repl_cmd::run_repl;
pub use schema_cmd::{run_schema_fetch, run_schema_status};
pub use script_cmd::run_script;
pub use search_cmd::run_search;
pub use serve_cmd::run_serve;
pub use snapshot_cmd::{run_snapshot_delete, run_snapshot_info, run_snapshot_list};
pub use tree_cmd::run_tree;
pub use tui_cmd::run_tui;
pub use wire_serve_cmd::run_wire_serve;

// Re-export ExecutionModeArg from cli_args
pub use crate::cli_args::ExecutionModeArg;

/// Execution mode for running Shape code
#[derive(Debug, Clone, Copy)]
pub enum ExecutionMode {
    BytecodeVM,
    #[cfg(feature = "jit")]
    JIT,
}

/// Options for loading data providers
#[derive(Debug, Clone, Default)]
pub struct ProviderOptions {
    /// Optional path to an extension module config file.
    pub config_path: Option<PathBuf>,
    /// Directory to scan for extension module shared libraries
    pub extension_dir: Option<PathBuf>,
}
