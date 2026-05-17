//! Process operation implementations for the io module.
//!
//! Phase 2d migration: ported to the typed marshal layer.
//! - Cluster #2 option γ for IoHandle-touching functions
//!   (`spawn` / `exec` / `process_*` / `read_line`).
//! - Phase 2d Array cluster (this commit, 2026-05-07): the optional
//!   `args: Array<string>` parameter on `io.spawn` / `io.exec` flows
//!   through `Vec<Arc<String>>` via the new `FromSlot` impl, which is
//!   the leaf decision that unblocked this migration.
//!
//! Exports: spawn, exec, process_wait, process_kill, process_write,
//!          process_read, process_read_err, process_read_line,
//!          stdin, stdout, stderr, read_line.
//!
//! Tests deferred — ValueWord-based test fixtures can't compile and
//! aren't reconstructed until the shape-vm cascade provides a typed
//! test harness, mirroring the file_ops migration in commit d716482.

use crate::marshal::{
    register_typed_fn_0, register_typed_fn_1, register_typed_fn_2_full,
};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{HeapValue, IoHandleData, IoResource};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;

/// Helper: build a `Command` with the given executable and optional
/// `Array<string>` argument list. The args parameter comes through the
/// marshal layer as `Vec<Arc<HeapValue>>` because the body declared its
/// signature as `args?: Array<string>` — but we want to read each element
/// as a string. We pattern-match the inner `HeapValue::TypedArray
/// (TypedArrayData::String(...))` here since that's the runtime shape
/// produced by Shape array literals at the call site. If a caller passed
/// a different array shape (e.g. an array of typed objects) the body
/// returns an error rather than panicking — this is a user-facing
/// type-mismatch surface, not the marshal-kind contract violation that
/// the FromSlot impl panics for.
fn build_command_with_args(
    cmd: &str,
    args: Option<Vec<Arc<HeapValue>>>,
    fn_name: &str,
) -> Result<Command, String> {
    let mut command = Command::new(cmd);
    if let Some(arg_list) = args {
        for elem in arg_list {
            match &*elem {
                HeapValue::String(s) => {
                    command.arg(&**s);
                }
                other => {
                    return Err(format!(
                        "{}: each element of args must be a string, got {}",
                        fn_name,
                        other.type_name()
                    ));
                }
            }
        }
    }
    Ok(command)
}

/// Register the 12 process IO functions on the io module.
/// Cluster #2 option γ + Phase 2d Array cluster per
/// `docs/defections.md` 2026-05-07.
pub fn register_process_io(module: &mut ModuleExports) {
    // io.spawn(cmd: string, args?: Array<string>) -> IoHandle
    register_typed_fn_2_full::<_, Arc<String>, Vec<Arc<HeapValue>>>(
        module,
        "spawn",
        "Spawn a subprocess with piped stdin/stdout/stderr",
        [
            ModuleParam {
                name: "cmd".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Command to run".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "args".to_string(),
                type_name: "Array<string>".to_string(),
                required: false,
                description: "Optional command arguments".to_string(),
                default_snippet: Some("[]".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::IoHandle,
        |cmd, args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let cmd_s = cmd.as_str();
            let mut command = build_command_with_args(cmd_s, Some(args), "io.spawn()")?;
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let child = command
                .spawn()
                .map_err(|e| format!("io.spawn(\"{}\"): {}", cmd_s, e))?;
            let handle = IoHandleData::new_child_process(child, cmd_s.to_string());
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                handle,
            ))))
        },
    );

    // io.exec(cmd: string, args?: Array<string>) -> object { status: int, stdout: string, stderr: string }
    register_typed_fn_2_full::<_, Arc<String>, Vec<Arc<HeapValue>>>(
        module,
        "exec",
        "Run a command to completion and capture its output",
        [
            ModuleParam {
                name: "cmd".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Command to run".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "args".to_string(),
                type_name: "Array<string>".to_string(),
                required: false,
                description: "Optional command arguments".to_string(),
                default_snippet: Some("[]".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::TypedObject,
        |cmd, args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let cmd_s = cmd.as_str();
            let mut command = build_command_with_args(cmd_s, Some(args), "io.exec()")?;
            let output = command
                .output()
                .map_err(|e| format!("io.exec(\"{}\"): {}", cmd_s, e))?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let status = output.status.code().unwrap_or(-1) as i64;
            Ok(TypedReturn::TypedObject(vec![
                ("status".to_string(), ConcreteReturn::I64(status)),
                ("stdout".to_string(), ConcreteReturn::String(stdout)),
                ("stderr".to_string(), ConcreteReturn::String(stderr)),
            ]))
        },
    );

    // io.process_wait(handle: IoHandle) -> int (exit code)
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "process_wait",
        "Wait for a child process to exit and return its exit code",
        "handle",
        "IoHandle",
        ConcreteType::Int,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.process_wait(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.process_wait(): handle is closed".to_string())?;
            match resource {
                IoResource::ChildProcess(child) => {
                    let status = child
                        .wait()
                        .map_err(|e| format!("io.process_wait(): {}", e))?;
                    Ok(TypedReturn::Concrete(ConcreteReturn::I64(
                        status.code().unwrap_or(-1) as i64,
                    )))
                }
                _ => Err("io.process_wait(): handle is not a ChildProcess".to_string()),
            }
        },
    );

    // io.process_kill(handle: IoHandle) -> unit
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "process_kill",
        "Kill a running child process",
        "handle",
        "IoHandle",
        ConcreteType::Unit,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.process_kill(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.process_kill(): handle is closed".to_string())?;
            match resource {
                IoResource::ChildProcess(child) => {
                    child
                        .kill()
                        .map_err(|e| format!("io.process_kill(): {}", e))?;
                    Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
                }
                _ => Err("io.process_kill(): handle is not a ChildProcess".to_string()),
            }
        },
    );

    // io.process_write(handle: IoHandle, data: string) -> int
    register_typed_fn_2_full::<_, Arc<IoHandleData>, Arc<String>>(
        module,
        "process_write",
        "Write to a child process's stdin, returning bytes written",
        [
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "ChildProcess or PipeWriter handle".to_string(),
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
        |handle, data, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.process_write(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.process_write(): handle is closed".to_string())?;
            match resource {
                IoResource::ChildProcess(child) => {
                    let stdin = child
                        .stdin
                        .as_mut()
                        .ok_or_else(|| "io.process_write(): stdin pipe not available".to_string())?;
                    let written = stdin
                        .write(data.as_bytes())
                        .map_err(|e| format!("io.process_write(): {}", e))?;
                    Ok(TypedReturn::Concrete(ConcreteReturn::I64(written as i64)))
                }
                IoResource::PipeWriter(stdin) => {
                    let written = stdin
                        .write(data.as_bytes())
                        .map_err(|e| format!("io.process_write(): {}", e))?;
                    Ok(TypedReturn::Concrete(ConcreteReturn::I64(written as i64)))
                }
                _ => Err(
                    "io.process_write(): handle is not a ChildProcess or PipeWriter".to_string(),
                ),
            }
        },
    );

    // io.process_read(handle: IoHandle, n?: int) -> string
    register_typed_fn_2_full::<_, Arc<IoHandleData>, i64>(
        module,
        "process_read",
        "Read up to n bytes from a child process's stdout",
        [
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "ChildProcess or PipeReader handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Max bytes to read (default: 65536)".to_string(),
                default_snippet: Some("65536".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |handle, n, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let buf_size = if n > 0 { n as usize } else { 65536 };
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.process_read(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.process_read(): handle is closed".to_string())?;
            let s = match resource {
                IoResource::ChildProcess(child) => {
                    let stdout = child
                        .stdout
                        .as_mut()
                        .ok_or_else(|| "io.process_read(): stdout pipe not available".to_string())?;
                    let mut buf = vec![0u8; buf_size];
                    let bytes_read = stdout
                        .read(&mut buf)
                        .map_err(|e| format!("io.process_read(): {}", e))?;
                    buf.truncate(bytes_read);
                    String::from_utf8(buf)
                        .map_err(|e| format!("io.process_read(): invalid UTF-8: {}", e))?
                }
                IoResource::PipeReader(stdout) => {
                    let mut buf = vec![0u8; buf_size];
                    let bytes_read = stdout
                        .read(&mut buf)
                        .map_err(|e| format!("io.process_read(): {}", e))?;
                    buf.truncate(bytes_read);
                    String::from_utf8(buf)
                        .map_err(|e| format!("io.process_read(): invalid UTF-8: {}", e))?
                }
                _ => {
                    return Err(
                        "io.process_read(): handle is not a ChildProcess or PipeReader"
                            .to_string(),
                    );
                }
            };
            Ok(TypedReturn::Concrete(ConcreteReturn::String(s)))
        },
    );

    // io.process_read_err(handle: IoHandle, n?: int) -> string
    register_typed_fn_2_full::<_, Arc<IoHandleData>, i64>(
        module,
        "process_read_err",
        "Read up to n bytes from a child process's stderr",
        [
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "ChildProcess or PipeReaderErr handle".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Max bytes to read (default: 65536)".to_string(),
                default_snippet: Some("65536".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |handle, n, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let buf_size = if n > 0 { n as usize } else { 65536 };
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.process_read_err(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.process_read_err(): handle is closed".to_string())?;
            let s = match resource {
                IoResource::ChildProcess(child) => {
                    let stderr = child.stderr.as_mut().ok_or_else(|| {
                        "io.process_read_err(): stderr pipe not available".to_string()
                    })?;
                    let mut buf = vec![0u8; buf_size];
                    let bytes_read = stderr
                        .read(&mut buf)
                        .map_err(|e| format!("io.process_read_err(): {}", e))?;
                    buf.truncate(bytes_read);
                    String::from_utf8(buf)
                        .map_err(|e| format!("io.process_read_err(): invalid UTF-8: {}", e))?
                }
                IoResource::PipeReaderErr(stderr) => {
                    let mut buf = vec![0u8; buf_size];
                    let bytes_read = stderr
                        .read(&mut buf)
                        .map_err(|e| format!("io.process_read_err(): {}", e))?;
                    buf.truncate(bytes_read);
                    String::from_utf8(buf)
                        .map_err(|e| format!("io.process_read_err(): invalid UTF-8: {}", e))?
                }
                _ => {
                    return Err(
                        "io.process_read_err(): handle is not a ChildProcess or PipeReaderErr"
                            .to_string(),
                    );
                }
            };
            Ok(TypedReturn::Concrete(ConcreteReturn::String(s)))
        },
    );

    // io.process_read_line(handle: IoHandle) -> string
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "process_read_line",
        "Read a single line from a child process's stdout (including newline)",
        "handle",
        "IoHandle",
        ConcreteType::String,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.process_read_line(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.process_read_line(): handle is closed".to_string())?;
            let line = match resource {
                IoResource::ChildProcess(child) => {
                    let stdout = child.stdout.as_mut().ok_or_else(|| {
                        "io.process_read_line(): stdout pipe not available".to_string()
                    })?;
                    let mut line = String::new();
                    BufReader::new(stdout)
                        .read_line(&mut line)
                        .map_err(|e| format!("io.process_read_line(): {}", e))?;
                    line
                }
                IoResource::PipeReader(stdout) => {
                    let mut line = String::new();
                    BufReader::new(stdout)
                        .read_line(&mut line)
                        .map_err(|e| format!("io.process_read_line(): {}", e))?;
                    line
                }
                _ => {
                    return Err(
                        "io.process_read_line(): handle is not a ChildProcess or PipeReader"
                            .to_string(),
                    );
                }
            };
            Ok(TypedReturn::Concrete(ConcreteReturn::String(line)))
        },
    );

    // io.stdin() -> IoHandle
    register_typed_fn_0(
        module,
        "stdin",
        "Return an IoHandle for the current process's standard input",
        ConcreteType::IoHandle,
        |ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .open("/dev/stdin")
                .map_err(|e| format!("io.stdin(): {}", e))?;
            let handle =
                IoHandleData::new_file(file, "/dev/stdin".to_string(), "r".to_string());
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                handle,
            ))))
        },
    );

    // io.stdout() -> IoHandle
    register_typed_fn_0(
        module,
        "stdout",
        "Return an IoHandle for the current process's standard output",
        ConcreteType::IoHandle,
        |ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/stdout")
                .map_err(|e| format!("io.stdout(): {}", e))?;
            let handle =
                IoHandleData::new_file(file, "/dev/stdout".to_string(), "w".to_string());
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                handle,
            ))))
        },
    );

    // io.stderr() -> IoHandle
    register_typed_fn_0(
        module,
        "stderr",
        "Return an IoHandle for the current process's standard error",
        ConcreteType::IoHandle,
        |ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/stderr")
                .map_err(|e| format!("io.stderr(): {}", e))?;
            let handle =
                IoHandleData::new_file(file, "/dev/stderr".to_string(), "w".to_string());
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                handle,
            ))))
        },
    );

    // io.read_line(handle: IoHandle) -> string
    //
    // Reads a line from a file/pipe handle. The original optional-handle
    // form (read from process stdin when no handle is provided) is left
    // for a follow-up — varargs/no-arg fallback semantics interlock with
    // the marshal-optional-args sub-cluster's "first-position optional"
    // shape, which the Phase 2c entry surfaced as deferred. Callers who
    // want process-stdin reading should call `io.stdin()` first and pass
    // the result.
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "read_line",
        "Read a single line from an IoHandle (file or pipe)",
        "handle",
        "IoHandle",
        ConcreteType::String,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.read_line(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.read_line(): handle is closed".to_string())?;
            let line = match resource {
                IoResource::File(file) => {
                    let mut line = String::new();
                    BufReader::new(file)
                        .read_line(&mut line)
                        .map_err(|e| format!("io.read_line(): {}", e))?;
                    line
                }
                IoResource::ChildProcess(child) => {
                    let stdout = child.stdout.as_mut().ok_or_else(|| {
                        "io.read_line(): stdout pipe not available".to_string()
                    })?;
                    let mut line = String::new();
                    BufReader::new(stdout)
                        .read_line(&mut line)
                        .map_err(|e| format!("io.read_line(): {}", e))?;
                    line
                }
                IoResource::PipeReader(stdout) => {
                    let mut line = String::new();
                    BufReader::new(stdout)
                        .read_line(&mut line)
                        .map_err(|e| format!("io.read_line(): {}", e))?;
                    line
                }
                _ => {
                    return Err("io.read_line(): handle does not support reading".to_string());
                }
            };
            Ok(TypedReturn::Concrete(ConcreteReturn::String(line)))
        },
    );
}
