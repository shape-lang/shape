use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "shape")]
#[command(about = "Shape - A programming language for data analysis and simulation", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Shape script file to execute (e.g. `shape foo.shape`)
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,

    /// Expand compile-time generated code instead of executing the script
    /// (shorthand for `shape expand-comptime <FILE>`)
    #[arg(long)]
    pub expand: bool,

    /// Filter expanded output to functions belonging to a module namespace
    /// (best-effort name-based filter, e.g. `duckdb`)
    #[arg(long)]
    pub module: Option<String>,

    /// Filter expanded output to a specific function name
    #[arg(long)]
    pub function: Option<String>,

    /// Execution mode: vm or jit
    #[arg(short, long, default_value = "vm")]
    pub mode: ExecutionModeArg,

    /// Extension module shared libraries to load at startup
    /// Can be specified multiple times: --extension ./csv.so --extension ./api.so
    #[arg(long = "extension", value_name = "PATH")]
    pub extensions: Vec<PathBuf>,

    /// Resume execution from a snapshot hash
    #[arg(long, value_name = "HASH")]
    pub resume: Option<String>,

    /// Path to data providers configuration file
    #[arg(long, value_name = "PATH")]
    pub providers_config: Option<PathBuf>,

    /// Load extension modules from this directory at startup
    #[arg(long, value_name = "DIR")]
    pub extension_dir: Option<PathBuf>,
}

/// Execution mode for running Shape code
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum ExecutionModeArg {
    /// Use bytecode VM (default execution mode)
    #[default]
    Vm,
    /// Use JIT compilation (~0.1-1µs/row, 100x+ faster)
    Jit,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Execute a Shape script (preferred explicit form of `shape <file>`)
    Run {
        /// Path to Shape script file
        script: Option<PathBuf>,
        #[command(flatten)]
        opts: RunCommandOptions,
    },

    /// Start interactive REPL
    Repl {
        #[command(flatten)]
        opts: RuntimeCommandOptions,
    },

    /// Start TUI notebook interface
    Tui {
        #[command(flatten)]
        opts: RuntimeCommandOptions,
    },

    /// Test code examples in markdown documentation files
    Doctest {
        /// Path to markdown file or directory containing markdown files
        path: PathBuf,
        /// Show verbose output including each test
        #[arg(short, long)]
        verbose: bool,
    },

    /// Manage data-source schema cache for compile-time validation
    Schema {
        #[command(subcommand)]
        action: SchemaAction,
        #[command(flatten)]
        opts: ProviderCommandOptions,
    },

    /// Manage execution snapshots
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },

    /// Print dependency tree for the current project
    Tree {
        /// Include bundled/native dependency scopes from .shapec metadata
        #[arg(long)]
        native: bool,
    },

    /// Manage Shape extensions (install, list, remove)
    Ext {
        #[command(subcommand)]
        action: ExtAction,
    },

    /// JIT diagnostics and parity reports
    Jit {
        #[command(subcommand)]
        action: JitAction,
    },

    /// Expand compile-time generated code (annotations/comptime directives)
    ExpandComptime {
        /// Path to Shape script file
        script: PathBuf,
        #[command(flatten)]
        opts: ExpandFilterOptions,
    },

    /// Compile a Shape package into a distributable .shapec bundle
    Build {
        /// Output path for the bundle (defaults to <name>-<version>.shapec)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optimization level (0-3)
        #[arg(long, default_value = "0")]
        opt_level: u8,
    },

    /// Sign a .shapec bundle with an Ed25519 key
    Sign {
        /// Path to the .shapec bundle file
        bundle: PathBuf,
        /// Path to the Ed25519 signing key file
        #[arg(short, long)]
        key: PathBuf,
    },

    /// Verify the signature on a .shapec bundle
    Verify {
        /// Path to the .shapec bundle file
        bundle: PathBuf,
    },

    /// Manage Ed25519 signing keys and trust
    Keys {
        #[command(subcommand)]
        action: KeysAction,
    },
}

#[derive(Args)]
pub struct ProviderCommandOptions {
    /// Extension module shared libraries to load at startup
    /// Can be specified multiple times: --extension ./csv.so --extension ./api.so
    #[arg(long = "extension", value_name = "PATH")]
    pub extensions: Vec<PathBuf>,

    /// Path to data providers configuration file
    #[arg(long, value_name = "PATH")]
    pub providers_config: Option<PathBuf>,

    /// Load extension modules from this directory at startup
    #[arg(long, value_name = "DIR")]
    pub extension_dir: Option<PathBuf>,
}

#[derive(Args)]
pub struct RuntimeCommandOptions {
    /// Execution mode: vm or jit
    #[arg(short, long, default_value = "vm")]
    pub mode: ExecutionModeArg,

    #[command(flatten)]
    pub provider: ProviderCommandOptions,
}

#[derive(Args)]
pub struct ExpandFilterOptions {
    /// Filter expanded output to functions belonging to a module namespace
    /// (best-effort name-based filter, e.g. `duckdb`)
    #[arg(long)]
    pub module: Option<String>,

    /// Filter expanded output to a specific function name
    #[arg(long)]
    pub function: Option<String>,
}

#[derive(Args)]
pub struct RunCommandOptions {
    /// Expand compile-time generated code instead of executing the script
    /// (shorthand for `shape expand-comptime <FILE>`)
    #[arg(long)]
    pub expand: bool,

    /// Resume execution from a snapshot hash
    #[arg(long, value_name = "HASH")]
    pub resume: Option<String>,

    #[command(flatten)]
    pub runtime: RuntimeCommandOptions,

    #[command(flatten)]
    pub expand_filter: ExpandFilterOptions,
}

#[derive(Subcommand)]
pub enum SnapshotAction {
    /// List all saved snapshots
    List,
    /// Show detailed info about a snapshot
    Info {
        /// Snapshot hash (full or prefix)
        hash: String,
    },
    /// Delete a snapshot
    Delete {
        /// Snapshot hash (full or prefix)
        hash: String,
    },
}

#[derive(Subcommand)]
pub enum SchemaAction {
    /// Fetch data-source schemas and cache in shape.lock artifacts
    Fetch {
        /// Specific source URI to fetch (e.g., "duckdb://analytics.db").
        /// If omitted, scans source files for connect() calls.
        uri: Option<String>,
    },
    /// Show cached data-source schemas and their staleness
    Status,
}

#[derive(Subcommand)]
pub enum KeysAction {
    /// Generate a new Ed25519 signing key pair
    Generate {
        /// Output path for the key file (defaults to ~/.shape/keys/<name>.key)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Human-readable name for the key
        #[arg(short, long, default_value = "default")]
        name: String,
    },
    /// Trust an author's public key for module verification
    Trust {
        /// Hex-encoded Ed25519 public key (64 hex chars)
        public_key: String,
        /// Human-readable name for the author
        #[arg(short, long)]
        name: String,
        /// Trust scope: "full", or a comma-separated list of module prefixes
        #[arg(short, long, default_value = "full")]
        scope: String,
    },
    /// List trusted keys
    List,
}

#[derive(Subcommand)]
pub enum ExtAction {
    /// Install an extension from crates.io (builds from source)
    Install {
        /// Extension name (e.g. 'python', 'typescript')
        name: String,
        /// Version requirement (e.g. '0.1.0', '>=0.2'). Defaults to latest.
        #[arg(long, default_value = None)]
        version: Option<String>,
    },
    /// List installed and available extensions
    List,
    /// Remove an installed extension
    Remove {
        /// Extension name (e.g. 'python', 'typescript')
        name: String,
    },
}

#[derive(Subcommand)]
pub enum JitAction {
    /// Print JIT parity matrix for all opcodes
    Parity {
        /// Include builtins in the matrix output
        #[arg(long)]
        builtins: bool,
        /// Show only VM-only rows (unsupported in JIT)
        #[arg(long)]
        unsupported_only: bool,
    },
}
