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

use crate::module_exports::{ModuleExports, ModuleFunction, ModuleParam};

/// Create the `io` module with file system operations.
pub fn create_io_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::io");
    module.description = "File system and path operations".to_string();

    // === File handle operations ===

    module.add_function_with_schema(
        "open",
        file_ops::io_open,
        ModuleFunction {
            description: "Open a file and return a handle".to_string(),
            params: vec![
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
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "read",
        file_ops::io_read,
        ModuleFunction {
            description: "Read from a file handle (n bytes or all)".to_string(),
            params: vec![
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
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "read_to_string",
        file_ops::io_read_to_string,
        ModuleFunction {
            description: "Read entire file as a string".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle from io.open()".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "read_bytes",
        file_ops::io_read_bytes,
        ModuleFunction {
            description: "Read bytes from a file as array of ints".to_string(),
            params: vec![
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
            return_type: Some("Vec<int>".to_string()),
        },
    );

    module.add_function_with_schema(
        "write",
        file_ops::io_write,
        ModuleFunction {
            description: "Write string or bytes to a file".to_string(),
            params: vec![
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
            return_type: Some("int".to_string()),
        },
    );

    module.add_function_with_schema(
        "close",
        file_ops::io_close,
        ModuleFunction {
            description: "Close a file handle".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle to close".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    module.add_function_with_schema(
        "flush",
        file_ops::io_flush,
        ModuleFunction {
            description: "Flush buffered writes to disk".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle to flush".to_string(),
                ..Default::default()
            }],
            return_type: Some("unit".to_string()),
        },
    );

    // === Stat operations (no handle needed) ===

    module.add_function_with_schema(
        "exists",
        file_ops::io_exists,
        ModuleFunction {
            description: "Check if a path exists".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to check".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    module.add_function_with_schema(
        "stat",
        file_ops::io_stat,
        ModuleFunction {
            description: "Get file/directory metadata".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to stat".to_string(),
                ..Default::default()
            }],
            return_type: Some("object".to_string()),
        },
    );

    module.add_function_with_schema(
        "is_file",
        file_ops::io_is_file,
        ModuleFunction {
            description: "Check if path is a file".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to check".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    module.add_function_with_schema(
        "is_dir",
        file_ops::io_is_dir,
        ModuleFunction {
            description: "Check if path is a directory".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to check".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    // === Directory operations ===

    module.add_function_with_schema(
        "mkdir",
        file_ops::io_mkdir,
        ModuleFunction {
            description: "Create a directory".to_string(),
            params: vec![
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
            return_type: Some("unit".to_string()),
        },
    );

    module.add_function_with_schema(
        "remove",
        file_ops::io_remove,
        ModuleFunction {
            description: "Remove a file or directory".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to remove".to_string(),
                ..Default::default()
            }],
            return_type: Some("unit".to_string()),
        },
    );

    module.add_function_with_schema(
        "rename",
        file_ops::io_rename,
        ModuleFunction {
            description: "Rename/move a file or directory".to_string(),
            params: vec![
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
            return_type: Some("unit".to_string()),
        },
    );

    module.add_function_with_schema(
        "read_dir",
        file_ops::io_read_dir,
        ModuleFunction {
            description: "List directory contents".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Directory path to list".to_string(),
                ..Default::default()
            }],
            return_type: Some("Vec<string>".to_string()),
        },
    );

    // === Path operations (sync, pure string manipulation) ===

    module.add_function_with_schema(
        "join",
        path_ops::io_join,
        ModuleFunction {
            description: "Join path components".to_string(),
            params: vec![ModuleParam {
                name: "parts".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path components to join".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "dirname",
        path_ops::io_dirname,
        ModuleFunction {
            description: "Get parent directory of a path".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "basename",
        path_ops::io_basename,
        ModuleFunction {
            description: "Get filename component of a path".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "extension",
        path_ops::io_extension,
        ModuleFunction {
            description: "Get file extension".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "resolve",
        path_ops::io_resolve,
        ModuleFunction {
            description: "Resolve/canonicalize a path".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to resolve".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // === TCP operations ===

    module.add_function_with_schema(
        "tcp_connect",
        network_ops::io_tcp_connect,
        ModuleFunction {
            description: "Connect to a TCP server".to_string(),
            params: vec![ModuleParam {
                name: "addr".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Address to connect to (e.g. \"127.0.0.1:8080\")".to_string(),
                ..Default::default()
            }],
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "tcp_listen",
        network_ops::io_tcp_listen,
        ModuleFunction {
            description: "Bind a TCP listener".to_string(),
            params: vec![ModuleParam {
                name: "addr".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Address to bind (e.g. \"0.0.0.0:8080\")".to_string(),
                ..Default::default()
            }],
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "tcp_accept",
        network_ops::io_tcp_accept,
        ModuleFunction {
            description: "Accept an incoming TCP connection".to_string(),
            params: vec![ModuleParam {
                name: "listener".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "TcpListener handle from io.tcp_listen()".to_string(),
                ..Default::default()
            }],
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "tcp_read",
        network_ops::io_tcp_read,
        ModuleFunction {
            description: "Read from a TCP stream".to_string(),
            params: vec![
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
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "tcp_write",
        network_ops::io_tcp_write,
        ModuleFunction {
            description: "Write to a TCP stream".to_string(),
            params: vec![
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
            return_type: Some("int".to_string()),
        },
    );

    module.add_function_with_schema(
        "tcp_close",
        network_ops::io_tcp_close,
        ModuleFunction {
            description: "Close a TCP handle".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "TCP handle to close".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    // === UDP operations ===

    module.add_function_with_schema(
        "udp_bind",
        network_ops::io_udp_bind,
        ModuleFunction {
            description: "Bind a UDP socket".to_string(),
            params: vec![ModuleParam {
                name: "addr".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Address to bind (e.g. \"0.0.0.0:0\" for ephemeral)".to_string(),
                ..Default::default()
            }],
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "udp_send",
        network_ops::io_udp_send,
        ModuleFunction {
            description: "Send a UDP datagram".to_string(),
            params: vec![
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
            return_type: Some("int".to_string()),
        },
    );

    module.add_function_with_schema(
        "udp_recv",
        network_ops::io_udp_recv,
        ModuleFunction {
            description: "Receive a UDP datagram".to_string(),
            params: vec![
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
            return_type: Some("object".to_string()),
        },
    );

    // === Process operations ===

    module.add_function_with_schema(
        "spawn",
        process_ops::io_spawn,
        ModuleFunction {
            description: "Spawn a subprocess with piped I/O".to_string(),
            params: vec![
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
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "exec",
        process_ops::io_exec,
        ModuleFunction {
            description: "Run a command and capture output".to_string(),
            params: vec![
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
            return_type: Some("object".to_string()),
        },
    );

    module.add_function_with_schema(
        "process_wait",
        process_ops::io_process_wait,
        ModuleFunction {
            description: "Wait for a process to exit".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "Process handle from io.spawn()".to_string(),
                ..Default::default()
            }],
            return_type: Some("int".to_string()),
        },
    );

    module.add_function_with_schema(
        "process_kill",
        process_ops::io_process_kill,
        ModuleFunction {
            description: "Kill a running process".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "Process handle from io.spawn()".to_string(),
                ..Default::default()
            }],
            return_type: Some("unit".to_string()),
        },
    );

    module.add_function_with_schema(
        "process_write",
        process_ops::io_process_write,
        ModuleFunction {
            description: "Write to a process stdin".to_string(),
            params: vec![
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
            return_type: Some("int".to_string()),
        },
    );

    module.add_function_with_schema(
        "process_read",
        process_ops::io_process_read,
        ModuleFunction {
            description: "Read from a process stdout".to_string(),
            params: vec![
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
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "process_read_err",
        process_ops::io_process_read_err,
        ModuleFunction {
            description: "Read from a process stderr".to_string(),
            params: vec![
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
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "process_read_line",
        process_ops::io_process_read_line,
        ModuleFunction {
            description: "Read one line from process stdout".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "Process handle".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "stdin",
        process_ops::io_stdin,
        ModuleFunction {
            description: "Get handle for current process stdin".to_string(),
            params: vec![],
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "stdout",
        process_ops::io_stdout,
        ModuleFunction {
            description: "Get handle for current process stdout".to_string(),
            params: vec![],
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "stderr",
        process_ops::io_stderr,
        ModuleFunction {
            description: "Get handle for current process stderr".to_string(),
            params: vec![],
            return_type: Some("IoHandle".to_string()),
        },
    );

    module.add_function_with_schema(
        "read_line",
        process_ops::io_read_line,
        ModuleFunction {
            description: "Read a line from a handle or stdin".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: false,
                description: "Handle to read from (default: stdin)".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // === Async file I/O operations ===

    module.add_async_function_with_schema(
        "read_file_async",
        async_file_ops::io_read_file_async,
        ModuleFunction {
            description: "Asynchronously read entire file as a string".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path to read".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_async_function_with_schema(
        "write_file_async",
        async_file_ops::io_write_file_async,
        ModuleFunction {
            description: "Asynchronously write a string to a file".to_string(),
            params: vec![
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
            return_type: Some("int".to_string()),
        },
    );

    module.add_async_function_with_schema(
        "append_file_async",
        async_file_ops::io_append_file_async,
        ModuleFunction {
            description: "Asynchronously append a string to a file".to_string(),
            params: vec![
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
            return_type: Some("int".to_string()),
        },
    );

    module.add_async_function_with_schema(
        "read_bytes_async",
        async_file_ops::io_read_bytes_async,
        ModuleFunction {
            description: "Asynchronously read file as raw bytes".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path to read".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<int>".to_string()),
        },
    );

    module.add_async_function_with_schema(
        "exists_async",
        async_file_ops::io_exists_async,
        ModuleFunction {
            description: "Asynchronously check if a path exists".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to check".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    // === Gzip file I/O ===

    module.add_function_with_schema(
        "read_gzip",
        file_ops::io_read_gzip,
        ModuleFunction {
            description: "Read a gzip-compressed file and return decompressed string".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to gzip file".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    module.add_function_with_schema(
        "write_gzip",
        file_ops::io_write_gzip,
        ModuleFunction {
            description: "Compress a string with gzip and write to a file".to_string(),
            params: vec![
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
            return_type: Some("null".to_string()),
        },
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
