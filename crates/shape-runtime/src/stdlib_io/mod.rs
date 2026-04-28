//! Native `io` module for file system and network operations.
//!
//! Exports: io.open(), io.read(), io.write(), io.close(), io.exists(), io.stat(),
//! io.mkdir(), io.remove(), io.rename(), io.read_dir(), io.join(), io.dirname(),
//! io.basename(), io.extension(), io.resolve(),
//! io.tcp_connect(), io.tcp_listen(), io.tcp_accept(), io.tcp_read(),
//! io.tcp_write(), io.tcp_close(), io.udp_bind(), io.udp_send(), io.udp_recv()

pub mod async_file_ops;
pub mod file_ops;
pub mod network_ops;
pub mod path_ops;
pub mod process_ops;

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{
    ConcreteType, TypedReturn, register_typed_async_function, register_typed_function,
};

/// Wrap a legacy-shaped `fn(&[ValueWord], &ModuleContext) -> Result<ValueWord, String>`
/// into a typed-body closure that round-trips the produced `ValueWord` through
/// `TypedReturn::ValueWord`. The actual `ValueWord` construction still lives
/// in the original op module — this Phase 4c.2 migration changes only the
/// registration surface so the legacy `add_function_with_schema` callers go
/// to zero, unblocking the typed-dispatch refactor in Phase 4d.
fn wrap_legacy<F>(
    f: F,
) -> impl for<'ctx> Fn(
    &[shape_value::ValueWord],
    &crate::module_exports::ModuleContext<'ctx>,
) -> Result<TypedReturn, String>
       + Send
       + Sync
       + 'static
where
    F: for<'ctx> Fn(
            &[shape_value::ValueWord],
            &crate::module_exports::ModuleContext<'ctx>,
        ) -> Result<shape_value::ValueWord, String>
        + Send
        + Sync
        + 'static,
{
    move |args, ctx| f(args, ctx).map(TypedReturn::ValueWord)
}

/// Same as [`wrap_legacy`] but for async-shaped function pointers.
fn wrap_legacy_async<F, Fut>(
    f: F,
) -> impl Fn(Vec<shape_value::ValueWord>) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<TypedReturn, String>> + Send>,
> + Send
       + Sync
       + Clone
       + 'static
where
    F: Fn(Vec<shape_value::ValueWord>) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<shape_value::ValueWord, String>> + Send + 'static,
{
    move |args| {
        let fut = f(args);
        Box::pin(async move { fut.await.map(TypedReturn::ValueWord) })
    }
}

/// Create the `io` module with file system operations.
pub fn create_io_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::io");
    module.description = "File system and path operations".to_string();

    // === File handle operations ===

    register_typed_function(
        &mut module,
        "open",
        "Open a file and return a handle",
        vec![
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path to open".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "mode".to_string(),
                type_name: "string".to_string(),
                required: false,
                description: "Open mode: \"r\" (default), \"w\", \"a\", \"rw\"".to_string(),
                default_snippet: Some("\"r\"".to_string()),
                allowed_values: Some(vec![
                    "r".to_string(),
                    "w".to_string(),
                    "a".to_string(),
                    "rw".to_string(),
                ]),
                ..Default::default()
            },
        ],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(file_ops::io_open),
    );

    register_typed_function(
        &mut module,
        "read",
        "Read from a file handle (n bytes or all)",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle from io.open()".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Number of bytes to read (omit for all)".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        wrap_legacy(file_ops::io_read),
    );

    register_typed_function(
        &mut module,
        "read_to_string",
        "Read entire file as a string",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "File handle from io.open()".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(file_ops::io_read_to_string),
    );

    register_typed_function(
        &mut module,
        "read_bytes",
        "Read bytes from a file as array of ints",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle from io.open()".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Number of bytes to read (omit for all)".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Named("Vec<int>".to_string()),
        wrap_legacy(file_ops::io_read_bytes),
    );

    register_typed_function(
        &mut module,
        "write",
        "Write string or bytes to a file",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle from io.open()".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to write".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Int,
        wrap_legacy(file_ops::io_write),
    );

    register_typed_function(
        &mut module,
        "close",
        "Close a file handle",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "File handle to close".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        wrap_legacy(file_ops::io_close),
    );

    register_typed_function(
        &mut module,
        "flush",
        "Flush buffered writes to disk",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "File handle to flush".to_string(),
            ..Default::default()
        }],
        ConcreteType::Unit,
        wrap_legacy(file_ops::io_flush),
    );

    // === Stat operations (no handle needed) ===

    register_typed_function(
        &mut module,
        "exists",
        "Check if a path exists",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to check".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        wrap_legacy(file_ops::io_exists),
    );

    register_typed_function(
        &mut module,
        "stat",
        "Get file/directory metadata",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to stat".to_string(),
            ..Default::default()
        }],
        ConcreteType::Object,
        wrap_legacy(file_ops::io_stat),
    );

    register_typed_function(
        &mut module,
        "is_file",
        "Check if path is a file",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to check".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        wrap_legacy(file_ops::io_is_file),
    );

    register_typed_function(
        &mut module,
        "is_dir",
        "Check if path is a directory",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to check".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        wrap_legacy(file_ops::io_is_dir),
    );

    // === Directory operations ===

    register_typed_function(
        &mut module,
        "mkdir",
        "Create a directory",
        vec![
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Directory path to create".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "recursive".to_string(),
                type_name: "bool".to_string(),
                required: false,
                description: "Create parent directories if needed".to_string(),
                default_snippet: Some("false".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::Unit,
        wrap_legacy(file_ops::io_mkdir),
    );

    register_typed_function(
        &mut module,
        "remove",
        "Remove a file or directory",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to remove".to_string(),
            ..Default::default()
        }],
        ConcreteType::Unit,
        wrap_legacy(file_ops::io_remove),
    );

    register_typed_function(
        &mut module,
        "rename",
        "Rename/move a file or directory",
        vec![
            ModuleParam {
                name: "old".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Current path".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "new".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "New path".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Unit,
        wrap_legacy(file_ops::io_rename),
    );

    register_typed_function(
        &mut module,
        "read_dir",
        "List directory contents",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Directory path to list".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named("Vec<string>".to_string()),
        wrap_legacy(file_ops::io_read_dir),
    );

    // === Path operations (sync, pure string manipulation) ===

    register_typed_function(
        &mut module,
        "join",
        "Join path components",
        vec![ModuleParam {
            name: "parts".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path components to join".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(path_ops::io_join),
    );

    register_typed_function(
        &mut module,
        "dirname",
        "Get parent directory of a path",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "File path".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(path_ops::io_dirname),
    );

    register_typed_function(
        &mut module,
        "basename",
        "Get filename component of a path",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "File path".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(path_ops::io_basename),
    );

    register_typed_function(
        &mut module,
        "extension",
        "Get file extension",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "File path".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(path_ops::io_extension),
    );

    register_typed_function(
        &mut module,
        "resolve",
        "Resolve/canonicalize a path",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to resolve".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(path_ops::io_resolve),
    );

    // === TCP operations ===

    register_typed_function(
        &mut module,
        "tcp_connect",
        "Connect to a TCP server",
        vec![ModuleParam {
            name: "addr".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Address to connect to (e.g. \"127.0.0.1:8080\")".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(network_ops::io_tcp_connect),
    );

    register_typed_function(
        &mut module,
        "tcp_listen",
        "Bind a TCP listener",
        vec![ModuleParam {
            name: "addr".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Address to bind (e.g. \"0.0.0.0:8080\")".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(network_ops::io_tcp_listen),
    );

    register_typed_function(
        &mut module,
        "tcp_accept",
        "Accept an incoming TCP connection",
        vec![ModuleParam {
            name: "listener".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "TcpListener handle from io.tcp_listen()".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(network_ops::io_tcp_accept),
    );

    register_typed_function(
        &mut module,
        "tcp_read",
        "Read from a TCP stream",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "TcpStream handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Max bytes to read (default 65536)".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        wrap_legacy(network_ops::io_tcp_read),
    );

    register_typed_function(
        &mut module,
        "tcp_write",
        "Write to a TCP stream",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "TcpStream handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to send".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Int,
        wrap_legacy(network_ops::io_tcp_write),
    );

    register_typed_function(
        &mut module,
        "tcp_close",
        "Close a TCP handle",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "TCP handle to close".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        wrap_legacy(network_ops::io_tcp_close),
    );

    // === UDP operations ===

    register_typed_function(
        &mut module,
        "udp_bind",
        "Bind a UDP socket",
        vec![ModuleParam {
            name: "addr".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Address to bind (e.g. \"0.0.0.0:0\" for ephemeral)".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(network_ops::io_udp_bind),
    );

    register_typed_function(
        &mut module,
        "udp_send",
        "Send a UDP datagram",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "UdpSocket handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to send".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "target".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Target address (e.g. \"127.0.0.1:9000\")".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Int,
        wrap_legacy(network_ops::io_udp_send),
    );

    register_typed_function(
        &mut module,
        "udp_recv",
        "Receive a UDP datagram",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "UdpSocket handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Max receive buffer size (default 65536)".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Object,
        wrap_legacy(network_ops::io_udp_recv),
    );

    // === Process operations ===

    register_typed_function(
        &mut module,
        "spawn",
        "Spawn a subprocess with piped I/O",
        vec![
            ModuleParam {
                name: "cmd".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Command to execute".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "args".to_string(),
                type_name: "Vec<string>".to_string(),
                required: false,
                description: "Command arguments".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(process_ops::io_spawn),
    );

    register_typed_function(
        &mut module,
        "exec",
        "Run a command and capture output",
        vec![
            ModuleParam {
                name: "cmd".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Command to execute".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "args".to_string(),
                type_name: "Vec<string>".to_string(),
                required: false,
                description: "Command arguments".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Object,
        wrap_legacy(process_ops::io_exec),
    );

    register_typed_function(
        &mut module,
        "process_wait",
        "Wait for a process to exit",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "Process handle from io.spawn()".to_string(),
            ..Default::default()
        }],
        ConcreteType::Int,
        wrap_legacy(process_ops::io_process_wait),
    );

    register_typed_function(
        &mut module,
        "process_kill",
        "Kill a running process",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "Process handle from io.spawn()".to_string(),
            ..Default::default()
        }],
        ConcreteType::Unit,
        wrap_legacy(process_ops::io_process_kill),
    );

    register_typed_function(
        &mut module,
        "process_write",
        "Write to a process stdin",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "Process handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to write to stdin".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Int,
        wrap_legacy(process_ops::io_process_write),
    );

    register_typed_function(
        &mut module,
        "process_read",
        "Read from a process stdout",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "Process handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Max bytes to read (default 65536)".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        wrap_legacy(process_ops::io_process_read),
    );

    register_typed_function(
        &mut module,
        "process_read_err",
        "Read from a process stderr",
        vec![
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "Process handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Max bytes to read (default 65536)".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        wrap_legacy(process_ops::io_process_read_err),
    );

    register_typed_function(
        &mut module,
        "process_read_line",
        "Read one line from process stdout",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: true,
            description: "Process handle".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(process_ops::io_process_read_line),
    );

    register_typed_function(
        &mut module,
        "stdin",
        "Get handle for current process stdin",
        vec![],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(process_ops::io_stdin),
    );

    register_typed_function(
        &mut module,
        "stdout",
        "Get handle for current process stdout",
        vec![],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(process_ops::io_stdout),
    );

    register_typed_function(
        &mut module,
        "stderr",
        "Get handle for current process stderr",
        vec![],
        ConcreteType::Named("IoHandle".to_string()),
        wrap_legacy(process_ops::io_stderr),
    );

    register_typed_function(
        &mut module,
        "read_line",
        "Read a line from a handle or stdin",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "IoHandle".to_string(),
            required: false,
            description: "Handle to read from (default: stdin)".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(process_ops::io_read_line),
    );

    // === Async file I/O operations ===

    register_typed_async_function(
        &mut module,
        "read_file_async",
        "Asynchronously read entire file as a string",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "File path to read".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy_async(async_file_ops::io_read_file_async),
    );

    register_typed_async_function(
        &mut module,
        "write_file_async",
        "Asynchronously write a string to a file",
        vec![
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path to write".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to write".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Int,
        wrap_legacy_async(async_file_ops::io_write_file_async),
    );

    register_typed_async_function(
        &mut module,
        "append_file_async",
        "Asynchronously append a string to a file",
        vec![
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path to append to".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to append".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Int,
        wrap_legacy_async(async_file_ops::io_append_file_async),
    );

    register_typed_async_function(
        &mut module,
        "read_bytes_async",
        "Asynchronously read file as raw bytes",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "File path to read".to_string(),
            ..Default::default()
        }],
        ConcreteType::ArrayInt,
        wrap_legacy_async(async_file_ops::io_read_bytes_async),
    );

    register_typed_async_function(
        &mut module,
        "exists_async",
        "Asynchronously check if a path exists",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to check".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        wrap_legacy_async(async_file_ops::io_exists_async),
    );

    // === Gzip file I/O ===

    register_typed_function(
        &mut module,
        "read_gzip",
        "Read a gzip-compressed file and return decompressed string",
        vec![ModuleParam {
            name: "path".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Path to gzip file".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        wrap_legacy(file_ops::io_read_gzip),
    );

    register_typed_function(
        &mut module,
        "write_gzip",
        "Compress a string with gzip and write to a file",
        vec![
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Output file path".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String data to compress and write".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "level".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Compression level 0-9 (default: 6)".to_string(),
                default_snippet: Some("6".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::Named("null".to_string()),
        wrap_legacy(file_ops::io_write_gzip),
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_module_creation() {
        let module = create_io_module();
        assert_eq!(module.name, "std::core::io");

        // File operations
        assert!(module.has_export("open"));
        assert!(module.has_export("read"));
        assert!(module.has_export("read_to_string"));
        assert!(module.has_export("read_bytes"));
        assert!(module.has_export("write"));
        assert!(module.has_export("close"));
        assert!(module.has_export("flush"));

        // Stat operations
        assert!(module.has_export("exists"));
        assert!(module.has_export("stat"));
        assert!(module.has_export("is_file"));
        assert!(module.has_export("is_dir"));

        // Directory operations
        assert!(module.has_export("mkdir"));
        assert!(module.has_export("remove"));
        assert!(module.has_export("rename"));
        assert!(module.has_export("read_dir"));

        // Path operations
        assert!(module.has_export("join"));
        assert!(module.has_export("dirname"));
        assert!(module.has_export("basename"));
        assert!(module.has_export("extension"));
        assert!(module.has_export("resolve"));

        // TCP operations
        assert!(module.has_export("tcp_connect"));
        assert!(module.has_export("tcp_listen"));
        assert!(module.has_export("tcp_accept"));
        assert!(module.has_export("tcp_read"));
        assert!(module.has_export("tcp_write"));
        assert!(module.has_export("tcp_close"));

        // UDP operations
        assert!(module.has_export("udp_bind"));
        assert!(module.has_export("udp_send"));
        assert!(module.has_export("udp_recv"));

        // Process operations
        assert!(module.has_export("spawn"));
        assert!(module.has_export("exec"));
        assert!(module.has_export("process_wait"));
        assert!(module.has_export("process_kill"));
        assert!(module.has_export("process_write"));
        assert!(module.has_export("process_read"));
        assert!(module.has_export("process_read_err"));
        assert!(module.has_export("process_read_line"));

        // Std stream operations
        assert!(module.has_export("stdin"));
        assert!(module.has_export("stdout"));
        assert!(module.has_export("stderr"));
        assert!(module.has_export("read_line"));

        // Async file I/O operations
        assert!(module.has_export("read_file_async"));
        assert!(module.has_export("write_file_async"));
        assert!(module.has_export("append_file_async"));
        assert!(module.has_export("read_bytes_async"));
        assert!(module.has_export("exists_async"));
    }

    #[test]
    fn test_io_module_schemas() {
        let module = create_io_module();

        let open_schema = module.get_schema("open").unwrap();
        assert_eq!(open_schema.params.len(), 2);
        assert_eq!(open_schema.return_type.as_deref(), Some("IoHandle"));

        let write_schema = module.get_schema("write").unwrap();
        assert_eq!(write_schema.params.len(), 2);

        let exists_schema = module.get_schema("exists").unwrap();
        assert_eq!(exists_schema.return_type.as_deref(), Some("bool"));

        // Network schemas
        let tcp_connect = module.get_schema("tcp_connect").unwrap();
        assert_eq!(tcp_connect.params.len(), 1);
        assert_eq!(tcp_connect.return_type.as_deref(), Some("IoHandle"));

        let tcp_read = module.get_schema("tcp_read").unwrap();
        assert_eq!(tcp_read.params.len(), 2);
        assert_eq!(tcp_read.return_type.as_deref(), Some("string"));

        let udp_send = module.get_schema("udp_send").unwrap();
        assert_eq!(udp_send.params.len(), 3);
        assert_eq!(udp_send.return_type.as_deref(), Some("int"));

        let udp_recv = module.get_schema("udp_recv").unwrap();
        assert_eq!(udp_recv.params.len(), 2);
        assert_eq!(udp_recv.return_type.as_deref(), Some("object"));
    }

    #[test]
    fn test_io_module_export_count() {
        let module = create_io_module();
        let names = module.export_names();
        assert_eq!(names.len(), 48); // 20 file/path + 9 network + 8 process + 4 std streams + 5 async + 2 gzip
    }
}
