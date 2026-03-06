//! Process operation implementations for the io module.
//!
//! Exports: spawn, exec, process_wait, process_kill, process_write,
//!          process_read, process_read_err, process_read_line

use shape_value::ValueWord;
use shape_value::heap_value::{IoHandleData, IoResource};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;

/// io.spawn(cmd, args?) -> IoHandle (ChildProcess)
///
/// Spawn a subprocess with piped stdin/stdout/stderr.
/// Returns a process handle. Use process_read/process_write/process_wait on it.
pub fn io_spawn(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
    let cmd = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.spawn() requires a command string".to_string())?;

    let mut command = Command::new(cmd);

    // Optional args array
    if let Some(view) = args.get(1).and_then(|a| a.as_any_array()) {
        let arr = view.to_generic();
        for arg in arr.iter() {
            if let Some(s) = arg.as_str() {
                command.arg(s);
            }
        }
    }

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = command
        .spawn()
        .map_err(|e| format!("io.spawn(\"{}\"): {}", cmd, e))?;

    let handle = IoHandleData::new_child_process(child, cmd.to_string());
    Ok(ValueWord::from_io_handle(handle))
}

/// io.exec(cmd, args?) -> object { status: int, stdout: string, stderr: string }
///
/// Run a command to completion and capture its output.
pub fn io_exec(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Process)?;
    let cmd = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.exec() requires a command string".to_string())?;

    let mut command = Command::new(cmd);

    if let Some(view) = args.get(1).and_then(|a| a.as_any_array()) {
        let arr = view.to_generic();
        for arg in arr.iter() {
            if let Some(s) = arg.as_str() {
                command.arg(s);
            }
        }
    }

    let output = command
        .output()
        .map_err(|e| format!("io.exec(\"{}\"): {}", cmd, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let status = output.status.code().unwrap_or(-1) as i64;

    let pairs: Vec<(&str, ValueWord)> = vec![
        ("status", ValueWord::from_i64(status)),
        ("stdout", ValueWord::from_string(Arc::new(stdout))),
        ("stderr", ValueWord::from_string(Arc::new(stderr))),
    ];
    Ok(crate::type_schema::typed_object_from_pairs(&pairs))
}

/// io.process_wait(handle) -> int (exit code)
///
/// Wait for a child process to exit and return its exit code.
pub fn io_process_wait(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.process_wait() requires a ChildProcess IoHandle".to_string())?;

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
            Ok(ValueWord::from_i64(status.code().unwrap_or(-1) as i64))
        }
        _ => Err("io.process_wait(): handle is not a ChildProcess".to_string()),
    }
}

/// io.process_kill(handle) -> unit
///
/// Kill a running child process.
pub fn io_process_kill(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.process_kill() requires a ChildProcess IoHandle".to_string())?;

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
            Ok(ValueWord::unit())
        }
        _ => Err("io.process_kill(): handle is not a ChildProcess".to_string()),
    }
}

/// io.process_write(handle, data) -> int (bytes written)
///
/// Write to a child process's stdin. Takes the process handle directly;
/// extracts the stdin pipe internally.
pub fn io_process_write(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.process_write() requires a ChildProcess IoHandle".to_string())?;

    let data = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.process_write() requires a string as second argument".to_string())?;

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
            Ok(ValueWord::from_i64(written as i64))
        }
        IoResource::PipeWriter(stdin) => {
            let written = stdin
                .write(data.as_bytes())
                .map_err(|e| format!("io.process_write(): {}", e))?;
            Ok(ValueWord::from_i64(written as i64))
        }
        _ => Err("io.process_write(): handle is not a ChildProcess or PipeWriter".to_string()),
    }
}

/// io.process_read(handle, n?) -> string
///
/// Read from a child process's stdout. If n is given, read up to n bytes.
pub fn io_process_read(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.process_read() requires a ChildProcess IoHandle".to_string())?;

    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .unwrap_or(65536.0) as usize;

    let mut guard = handle
        .resource
        .lock()
        .map_err(|_| "io.process_read(): lock poisoned".to_string())?;
    let resource = guard
        .as_mut()
        .ok_or_else(|| "io.process_read(): handle is closed".to_string())?;

    match resource {
        IoResource::ChildProcess(child) => {
            let stdout = child
                .stdout
                .as_mut()
                .ok_or_else(|| "io.process_read(): stdout pipe not available".to_string())?;
            let mut buf = vec![0u8; n];
            let bytes_read = stdout
                .read(&mut buf)
                .map_err(|e| format!("io.process_read(): {}", e))?;
            buf.truncate(bytes_read);
            let s = String::from_utf8(buf)
                .map_err(|e| format!("io.process_read(): invalid UTF-8: {}", e))?;
            Ok(ValueWord::from_string(Arc::new(s)))
        }
        IoResource::PipeReader(stdout) => {
            let mut buf = vec![0u8; n];
            let bytes_read = stdout
                .read(&mut buf)
                .map_err(|e| format!("io.process_read(): {}", e))?;
            buf.truncate(bytes_read);
            let s = String::from_utf8(buf)
                .map_err(|e| format!("io.process_read(): invalid UTF-8: {}", e))?;
            Ok(ValueWord::from_string(Arc::new(s)))
        }
        _ => Err("io.process_read(): handle is not a ChildProcess or PipeReader".to_string()),
    }
}

/// io.process_read_err(handle, n?) -> string
///
/// Read from a child process's stderr.
pub fn io_process_read_err(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.process_read_err() requires a ChildProcess IoHandle".to_string())?;

    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .unwrap_or(65536.0) as usize;

    let mut guard = handle
        .resource
        .lock()
        .map_err(|_| "io.process_read_err(): lock poisoned".to_string())?;
    let resource = guard
        .as_mut()
        .ok_or_else(|| "io.process_read_err(): handle is closed".to_string())?;

    match resource {
        IoResource::ChildProcess(child) => {
            let stderr = child
                .stderr
                .as_mut()
                .ok_or_else(|| "io.process_read_err(): stderr pipe not available".to_string())?;
            let mut buf = vec![0u8; n];
            let bytes_read = stderr
                .read(&mut buf)
                .map_err(|e| format!("io.process_read_err(): {}", e))?;
            buf.truncate(bytes_read);
            let s = String::from_utf8(buf)
                .map_err(|e| format!("io.process_read_err(): invalid UTF-8: {}", e))?;
            Ok(ValueWord::from_string(Arc::new(s)))
        }
        IoResource::PipeReaderErr(stderr) => {
            let mut buf = vec![0u8; n];
            let bytes_read = stderr
                .read(&mut buf)
                .map_err(|e| format!("io.process_read_err(): {}", e))?;
            buf.truncate(bytes_read);
            let s = String::from_utf8(buf)
                .map_err(|e| format!("io.process_read_err(): invalid UTF-8: {}", e))?;
            Ok(ValueWord::from_string(Arc::new(s)))
        }
        _ => {
            Err("io.process_read_err(): handle is not a ChildProcess or PipeReaderErr".to_string())
        }
    }
}

/// io.process_read_line(handle) -> string
///
/// Read a single line from a child process's stdout (including newline).
pub fn io_process_read_line(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.process_read_line() requires a ChildProcess IoHandle".to_string())?;

    let mut guard = handle
        .resource
        .lock()
        .map_err(|_| "io.process_read_line(): lock poisoned".to_string())?;
    let resource = guard
        .as_mut()
        .ok_or_else(|| "io.process_read_line(): handle is closed".to_string())?;

    match resource {
        IoResource::ChildProcess(child) => {
            let stdout = child
                .stdout
                .as_mut()
                .ok_or_else(|| "io.process_read_line(): stdout pipe not available".to_string())?;
            let mut line = String::new();
            BufReader::new(stdout)
                .read_line(&mut line)
                .map_err(|e| format!("io.process_read_line(): {}", e))?;
            Ok(ValueWord::from_string(Arc::new(line)))
        }
        IoResource::PipeReader(stdout) => {
            let mut line = String::new();
            BufReader::new(stdout)
                .read_line(&mut line)
                .map_err(|e| format!("io.process_read_line(): {}", e))?;
            Ok(ValueWord::from_string(Arc::new(line)))
        }
        _ => Err("io.process_read_line(): handle is not a ChildProcess or PipeReader".to_string()),
    }
}

/// io.stdin() -> IoHandle
///
/// Return an IoHandle for the current process's standard input.
pub fn io_stdin(
    _args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .open("/dev/stdin")
        .map_err(|e| format!("io.stdin(): {}", e))?;
    let handle = IoHandleData::new_file(file, "/dev/stdin".to_string(), "r".to_string());
    Ok(ValueWord::from_io_handle(handle))
}

/// io.stdout() -> IoHandle
///
/// Return an IoHandle for the current process's standard output.
pub fn io_stdout(
    _args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/stdout")
        .map_err(|e| format!("io.stdout(): {}", e))?;
    let handle = IoHandleData::new_file(file, "/dev/stdout".to_string(), "w".to_string());
    Ok(ValueWord::from_io_handle(handle))
}

/// io.stderr() -> IoHandle
///
/// Return an IoHandle for the current process's standard error.
pub fn io_stderr(
    _args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/stderr")
        .map_err(|e| format!("io.stderr(): {}", e))?;
    let handle = IoHandleData::new_file(file, "/dev/stderr".to_string(), "w".to_string());
    Ok(ValueWord::from_io_handle(handle))
}

/// io.read_line(handle?) -> string
///
/// Read a line from an IoHandle (file or pipe). If no handle is given,
/// reads from the current process's stdin.
pub fn io_read_line(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    // If a handle argument is provided, read from it
    if let Some(handle) = args.first().and_then(|a| a.as_io_handle()) {
        let mut guard = handle
            .resource
            .lock()
            .map_err(|_| "io.read_line(): lock poisoned".to_string())?;
        let resource = guard
            .as_mut()
            .ok_or_else(|| "io.read_line(): handle is closed".to_string())?;

        match resource {
            IoResource::File(file) => {
                let mut line = String::new();
                BufReader::new(file)
                    .read_line(&mut line)
                    .map_err(|e| format!("io.read_line(): {}", e))?;
                Ok(ValueWord::from_string(Arc::new(line)))
            }
            IoResource::ChildProcess(child) => {
                let stdout = child
                    .stdout
                    .as_mut()
                    .ok_or_else(|| "io.read_line(): stdout pipe not available".to_string())?;
                let mut line = String::new();
                BufReader::new(stdout)
                    .read_line(&mut line)
                    .map_err(|e| format!("io.read_line(): {}", e))?;
                Ok(ValueWord::from_string(Arc::new(line)))
            }
            IoResource::PipeReader(stdout) => {
                let mut line = String::new();
                BufReader::new(stdout)
                    .read_line(&mut line)
                    .map_err(|e| format!("io.read_line(): {}", e))?;
                Ok(ValueWord::from_string(Arc::new(line)))
            }
            _ => Err("io.read_line(): handle does not support reading".to_string()),
        }
    } else {
        // No handle: read from current process stdin
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("io.read_line(): {}", e))?;
        Ok(ValueWord::from_string(Arc::new(line)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

    #[test]
    fn test_exec_echo() {
        let ctx = test_ctx();
        let result = io_exec(
            &[
                ValueWord::from_string(Arc::new("echo".to_string())),
                ValueWord::from_array(Arc::new(vec![ValueWord::from_string(Arc::new(
                    "hello world".to_string(),
                ))])),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.type_name(), "object");
    }

    #[test]
    fn test_exec_false() {
        let ctx = test_ctx();
        let result = io_exec(
            &[ValueWord::from_string(Arc::new("false".to_string()))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.type_name(), "object");
    }

    #[test]
    fn test_exec_nonexistent() {
        let ctx = test_ctx();
        let result = io_exec(
            &[ValueWord::from_string(Arc::new(
                "__nonexistent_command_xyz__".to_string(),
            ))],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_spawn_and_wait() {
        let ctx = test_ctx();
        let handle = io_spawn(
            &[
                ValueWord::from_string(Arc::new("echo".to_string())),
                ValueWord::from_array(Arc::new(vec![ValueWord::from_string(Arc::new(
                    "test".to_string(),
                ))])),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(handle.type_name(), "io_handle");

        let code = io_process_wait(&[handle.clone()], &ctx).unwrap();
        assert_eq!(code.as_number_coerce(), Some(0.0));
    }

    #[test]
    fn test_spawn_read_stdout() {
        let ctx = test_ctx();
        let handle = io_spawn(
            &[
                ValueWord::from_string(Arc::new("echo".to_string())),
                ValueWord::from_array(Arc::new(vec![ValueWord::from_string(Arc::new(
                    "hello from process".to_string(),
                ))])),
            ],
            &ctx,
        )
        .unwrap();

        // Wait for process to finish first
        io_process_wait(&[handle.clone()], &ctx).unwrap();

        let output = io_process_read(&[handle.clone()], &ctx).unwrap();
        let text = output.as_str().unwrap();
        assert!(text.contains("hello from process"));

        handle.as_io_handle().unwrap().close();
    }

    #[test]
    fn test_spawn_write_stdin() {
        let ctx = test_ctx();
        // Use `cat` which echoes stdin to stdout
        let handle =
            io_spawn(&[ValueWord::from_string(Arc::new("cat".to_string()))], &ctx).unwrap();

        // Write to stdin
        let written = io_process_write(
            &[
                handle.clone(),
                ValueWord::from_string(Arc::new("input data".to_string())),
            ],
            &ctx,
        )
        .unwrap();
        assert!(written.as_number_coerce().unwrap() > 0.0);

        // Close stdin to signal EOF to cat (drop the stdin pipe)
        {
            let h = handle.as_io_handle().unwrap();
            let mut guard = h.resource.lock().unwrap();
            if let Some(IoResource::ChildProcess(child)) = guard.as_mut() {
                drop(child.stdin.take());
            }
        }

        // Wait for process to finish
        io_process_wait(&[handle.clone()], &ctx).unwrap();

        // Read stdout
        let output = io_process_read(&[handle.clone()], &ctx).unwrap();
        assert_eq!(output.as_str().unwrap(), "input data");

        handle.as_io_handle().unwrap().close();
    }

    #[test]
    fn test_spawn_kill() {
        let ctx = test_ctx();
        // Start a long-running process
        let handle = io_spawn(
            &[
                ValueWord::from_string(Arc::new("sleep".to_string())),
                ValueWord::from_array(Arc::new(vec![ValueWord::from_string(Arc::new(
                    "60".to_string(),
                ))])),
            ],
            &ctx,
        )
        .unwrap();

        // Kill it
        let result = io_process_kill(&[handle.clone()], &ctx);
        assert!(result.is_ok());

        // Wait should return non-zero (signal)
        let code = io_process_wait(&[handle.clone()], &ctx).unwrap();
        // Killed processes typically have non-zero exit
        let exit_code = code.as_number_coerce().unwrap() as i64;
        assert!(exit_code != 0 || exit_code == -1);

        handle.as_io_handle().unwrap().close();
    }

    #[test]
    fn test_spawn_read_stderr() {
        let ctx = test_ctx();
        // Use a command that writes to stderr
        let handle = io_spawn(
            &[
                ValueWord::from_string(Arc::new("sh".to_string())),
                ValueWord::from_array(Arc::new(vec![
                    ValueWord::from_string(Arc::new("-c".to_string())),
                    ValueWord::from_string(Arc::new("echo error_msg >&2".to_string())),
                ])),
            ],
            &ctx,
        )
        .unwrap();

        io_process_wait(&[handle.clone()], &ctx).unwrap();

        let err_output = io_process_read_err(&[handle.clone()], &ctx).unwrap();
        let text = err_output.as_str().unwrap();
        assert!(text.contains("error_msg"));

        handle.as_io_handle().unwrap().close();
    }

    #[test]
    fn test_stdout_handle() {
        let ctx = test_ctx();
        let handle = io_stdout(&[], &ctx).unwrap();
        assert_eq!(handle.type_name(), "io_handle");
        let data = handle.as_io_handle().unwrap();
        assert_eq!(data.path, "/dev/stdout");
        assert_eq!(data.mode, "w");
        data.close();
    }

    #[test]
    fn test_stderr_handle() {
        let ctx = test_ctx();
        let handle = io_stderr(&[], &ctx).unwrap();
        assert_eq!(handle.type_name(), "io_handle");
        let data = handle.as_io_handle().unwrap();
        assert_eq!(data.path, "/dev/stderr");
        assert_eq!(data.mode, "w");
        data.close();
    }

    #[test]
    fn test_read_line_from_pipe() {
        let ctx = test_ctx();
        // Spawn echo to produce a line, then read_line from the process handle
        let handle = io_spawn(
            &[
                ValueWord::from_string(Arc::new("echo".to_string())),
                ValueWord::from_array(Arc::new(vec![ValueWord::from_string(Arc::new(
                    "line output".to_string(),
                ))])),
            ],
            &ctx,
        )
        .unwrap();

        io_process_wait(&[handle.clone()], &ctx).unwrap();

        let line = io_process_read_line(&[handle.clone()], &ctx).unwrap();
        assert!(line.as_str().unwrap().contains("line output"));

        handle.as_io_handle().unwrap().close();
    }
}
