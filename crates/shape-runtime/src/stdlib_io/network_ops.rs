//! Network operation implementations for the io module.
//!
//! TCP: tcp_connect, tcp_listen, tcp_accept, tcp_read, tcp_write, tcp_close
//! UDP: udp_bind, udp_send, udp_recv
//!
//! All operations use blocking std::net (not tokio).

use shape_value::ValueWord;
use shape_value::heap_value::{IoHandleData, IoResource};
use std::io::{Read, Write};
use std::sync::Arc;

// ── TCP ─────────────────────────────────────────────────────────────────────

/// io.tcp_connect(addr) -> IoHandle
///
/// Connect to a TCP server at `addr` (e.g. "127.0.0.1:8080").
pub fn io_tcp_connect(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let addr = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.tcp_connect() requires a string address".to_string())?;
    crate::module_exports::check_net_permission(ctx, shape_abi_v1::Permission::NetConnect, addr)?;

    let stream = std::net::TcpStream::connect(addr)
        .map_err(|e| format!("io.tcp_connect(\"{}\"): {}", addr, e))?;

    let handle = IoHandleData::new_tcp_stream(stream, addr.to_string());
    Ok(ValueWord::from_io_handle(handle))
}

/// io.tcp_listen(addr) -> IoHandle
///
/// Bind a TCP listener to `addr` (e.g. "0.0.0.0:8080").
pub fn io_tcp_listen(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let addr = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.tcp_listen() requires a string address".to_string())?;
    crate::module_exports::check_net_permission(ctx, shape_abi_v1::Permission::NetListen, addr)?;

    let listener = std::net::TcpListener::bind(addr)
        .map_err(|e| format!("io.tcp_listen(\"{}\"): {}", addr, e))?;

    let handle = IoHandleData::new_tcp_listener(listener, addr.to_string());
    Ok(ValueWord::from_io_handle(handle))
}

/// io.tcp_accept(listener) -> IoHandle
///
/// Accept the next incoming connection on a TcpListener.
/// Returns a new IoHandle (TcpStream) for the accepted connection.
pub fn io_tcp_accept(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetListen)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.tcp_accept() requires a TcpListener IoHandle".to_string())?;

    let guard = handle
        .resource
        .lock()
        .map_err(|_| "io.tcp_accept(): lock poisoned".to_string())?;
    let resource = guard
        .as_ref()
        .ok_or_else(|| "io.tcp_accept(): handle is closed".to_string())?;

    match resource {
        IoResource::TcpListener(listener) => {
            let (stream, peer) = listener
                .accept()
                .map_err(|e| format!("io.tcp_accept(): {}", e))?;
            let peer_str = peer.to_string();
            let client = IoHandleData::new_tcp_stream(stream, peer_str);
            Ok(ValueWord::from_io_handle(client))
        }
        _ => Err("io.tcp_accept(): handle is not a TcpListener".to_string()),
    }
}

/// io.tcp_read(handle, n?) -> string
///
/// Read from a TCP stream. If `n` is given, read up to `n` bytes;
/// otherwise read whatever is available in a single recv (up to 64KB).
pub fn io_tcp_read(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.tcp_read() requires a TcpStream IoHandle".to_string())?;

    let n = args.get(1).and_then(|a| a.as_number_coerce());

    let mut guard = handle
        .resource
        .lock()
        .map_err(|_| "io.tcp_read(): lock poisoned".to_string())?;
    let resource = guard
        .as_mut()
        .ok_or_else(|| "io.tcp_read(): handle is closed".to_string())?;

    match resource {
        IoResource::TcpStream(stream) => {
            let buf_size = n.map(|v| v as usize).unwrap_or(65536);
            let mut buf = vec![0u8; buf_size];
            let bytes_read = stream
                .read(&mut buf)
                .map_err(|e| format!("io.tcp_read(): {}", e))?;
            buf.truncate(bytes_read);
            let s = String::from_utf8(buf)
                .map_err(|e| format!("io.tcp_read(): invalid UTF-8: {}", e))?;
            Ok(ValueWord::from_string(Arc::new(s)))
        }
        _ => Err("io.tcp_read(): handle is not a TcpStream".to_string()),
    }
}

/// io.tcp_write(handle, data) -> int
///
/// Write a string to a TCP stream. Returns bytes written.
pub fn io_tcp_write(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.tcp_write() requires a TcpStream IoHandle".to_string())?;

    let data = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.tcp_write() requires a string as second argument".to_string())?;

    let mut guard = handle
        .resource
        .lock()
        .map_err(|_| "io.tcp_write(): lock poisoned".to_string())?;
    let resource = guard
        .as_mut()
        .ok_or_else(|| "io.tcp_write(): handle is closed".to_string())?;

    match resource {
        IoResource::TcpStream(stream) => {
            let written = stream
                .write(data.as_bytes())
                .map_err(|e| format!("io.tcp_write(): {}", e))?;
            Ok(ValueWord::from_i64(written as i64))
        }
        _ => Err("io.tcp_write(): handle is not a TcpStream".to_string()),
    }
}

/// io.tcp_close(handle) -> bool
///
/// Shut down and close a TCP stream or listener. Returns true if it was open.
pub fn io_tcp_close(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.tcp_close() requires an IoHandle".to_string())?;

    Ok(ValueWord::from_bool(handle.close()))
}

// ── UDP ─────────────────────────────────────────────────────────────────────

/// io.udp_bind(addr) -> IoHandle
///
/// Bind a UDP socket to `addr` (e.g. "0.0.0.0:0" for ephemeral port).
pub fn io_udp_bind(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let addr = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.udp_bind() requires a string address".to_string())?;
    crate::module_exports::check_net_permission(ctx, shape_abi_v1::Permission::NetListen, addr)?;

    let socket =
        std::net::UdpSocket::bind(addr).map_err(|e| format!("io.udp_bind(\"{}\"): {}", addr, e))?;

    // Get the actual bound address (useful when binding to port 0)
    let local = socket
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| addr.to_string());

    let handle = IoHandleData::new_udp_socket(socket, local);
    Ok(ValueWord::from_io_handle(handle))
}

/// io.udp_send(handle, data, target_addr) -> int
///
/// Send a datagram to `target_addr`. Returns bytes sent.
pub fn io_udp_send(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.udp_send() requires a UdpSocket IoHandle".to_string())?;

    let data = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.udp_send() requires a string as second argument".to_string())?;

    let target = args
        .get(2)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.udp_send() requires a target address as third argument".to_string())?;

    let guard = handle
        .resource
        .lock()
        .map_err(|_| "io.udp_send(): lock poisoned".to_string())?;
    let resource = guard
        .as_ref()
        .ok_or_else(|| "io.udp_send(): handle is closed".to_string())?;

    match resource {
        IoResource::UdpSocket(socket) => {
            let sent = socket
                .send_to(data.as_bytes(), target)
                .map_err(|e| format!("io.udp_send(): {}", e))?;
            Ok(ValueWord::from_i64(sent as i64))
        }
        _ => Err("io.udp_send(): handle is not a UdpSocket".to_string()),
    }
}

/// io.udp_recv(handle, n?) -> object { data: string, addr: string }
///
/// Receive a datagram. Returns an object with the data and sender address.
/// `n` is the max receive buffer (default 65536).
pub fn io_udp_recv(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.udp_recv() requires a UdpSocket IoHandle".to_string())?;

    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .unwrap_or(65536.0) as usize;

    let guard = handle
        .resource
        .lock()
        .map_err(|_| "io.udp_recv(): lock poisoned".to_string())?;
    let resource = guard
        .as_ref()
        .ok_or_else(|| "io.udp_recv(): handle is closed".to_string())?;

    match resource {
        IoResource::UdpSocket(socket) => {
            let mut buf = vec![0u8; n];
            let (bytes_read, src_addr) = socket
                .recv_from(&mut buf)
                .map_err(|e| format!("io.udp_recv(): {}", e))?;
            buf.truncate(bytes_read);
            let data = String::from_utf8(buf)
                .map_err(|e| format!("io.udp_recv(): invalid UTF-8: {}", e))?;

            let pairs: Vec<(&str, ValueWord)> = vec![
                ("data", ValueWord::from_string(Arc::new(data))),
                (
                    "addr",
                    ValueWord::from_string(Arc::new(src_addr.to_string())),
                ),
            ];
            Ok(crate::type_schema::typed_object_from_pairs(&pairs))
        }
        _ => Err("io.udp_recv(): handle is not a UdpSocket".to_string()),
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
    fn test_tcp_connect_refused() {
        let ctx = test_ctx();
        // Connecting to a port that nothing listens on should fail
        let result = io_tcp_connect(
            &[ValueWord::from_string(Arc::new("127.0.0.1:1".to_string()))],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_tcp_listen_and_accept_echo() {
        let ctx = test_ctx();
        // Bind listener to ephemeral port
        let listener_handle = io_tcp_listen(
            &[ValueWord::from_string(Arc::new("127.0.0.1:0".to_string()))],
            &ctx,
        )
        .unwrap();

        // Get the actual bound address from the listener
        let bound_addr = {
            let data = listener_handle.as_io_handle().unwrap();
            let guard = data.resource.lock().unwrap();
            match guard.as_ref().unwrap() {
                IoResource::TcpListener(l) => l.local_addr().unwrap().to_string(),
                _ => panic!("expected TcpListener"),
            }
        };

        // Set listener to non-blocking so accept won't hang if test breaks
        {
            let data = listener_handle.as_io_handle().unwrap();
            let guard = data.resource.lock().unwrap();
            if let Some(IoResource::TcpListener(l)) = guard.as_ref() {
                l.set_nonblocking(false).unwrap();
            }
        }

        // Connect from a client in a background thread
        let addr_clone = bound_addr.clone();
        let client_thread = std::thread::spawn(move || {
            let ctx = test_ctx();
            let stream = std::net::TcpStream::connect(&addr_clone).unwrap();
            let handle = IoHandleData::new_tcp_stream(stream, addr_clone);
            let nb = ValueWord::from_io_handle(handle);

            // Write
            io_tcp_write(
                &[
                    nb.clone(),
                    ValueWord::from_string(Arc::new("ping".to_string())),
                ],
                &ctx,
            )
            .unwrap();

            // Read echo back
            // Set a read timeout so we don't hang forever
            {
                let h = nb.as_io_handle().unwrap();
                let g = h.resource.lock().unwrap();
                if let Some(IoResource::TcpStream(s)) = g.as_ref() {
                    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                }
            }
            let response = io_tcp_read(&[nb.clone()], &ctx).unwrap();
            assert_eq!(response.as_str().unwrap(), "ping");

            io_tcp_close(&[nb], &ctx).unwrap();
        });

        // Accept the incoming connection
        let server_conn = io_tcp_accept(&[listener_handle.clone()], &ctx).unwrap();

        // Set read timeout on server connection
        {
            let h = server_conn.as_io_handle().unwrap();
            let g = h.resource.lock().unwrap();
            if let Some(IoResource::TcpStream(s)) = g.as_ref() {
                s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
            }
        }

        // Read from client
        let data = io_tcp_read(&[server_conn.clone()], &ctx).unwrap();
        assert_eq!(data.as_str().unwrap(), "ping");

        // Echo back
        io_tcp_write(&[server_conn.clone(), data], &ctx).unwrap();

        // Wait for client thread
        client_thread.join().unwrap();

        // Close everything
        io_tcp_close(&[server_conn], &ctx).unwrap();
        io_tcp_close(&[listener_handle], &ctx).unwrap();
    }

    #[test]
    fn test_tcp_close_returns_false_on_double_close() {
        let ctx = test_ctx();
        let listener = io_tcp_listen(
            &[ValueWord::from_string(Arc::new("127.0.0.1:0".to_string()))],
            &ctx,
        )
        .unwrap();

        let first = io_tcp_close(&[listener.clone()], &ctx).unwrap();
        assert_eq!(first.as_bool(), Some(true));

        let second = io_tcp_close(&[listener], &ctx).unwrap();
        assert_eq!(second.as_bool(), Some(false));
    }

    #[test]
    fn test_tcp_read_on_closed_handle() {
        let ctx = test_ctx();
        let listener = io_tcp_listen(
            &[ValueWord::from_string(Arc::new("127.0.0.1:0".to_string()))],
            &ctx,
        )
        .unwrap();

        // Get the addr for a client connect
        let addr = {
            let h = listener.as_io_handle().unwrap();
            let g = h.resource.lock().unwrap();
            match g.as_ref().unwrap() {
                IoResource::TcpListener(l) => l.local_addr().unwrap().to_string(),
                _ => panic!(),
            }
        };

        let conn = io_tcp_connect(&[ValueWord::from_string(Arc::new(addr))], &ctx).unwrap();
        io_tcp_close(&[conn.clone()], &ctx).unwrap();

        let result = io_tcp_read(&[conn], &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("closed"));
    }

    #[test]
    fn test_udp_send_recv() {
        let ctx = test_ctx();
        // Bind two sockets
        let sock_a = io_udp_bind(
            &[ValueWord::from_string(Arc::new("127.0.0.1:0".to_string()))],
            &ctx,
        )
        .unwrap();
        let sock_b = io_udp_bind(
            &[ValueWord::from_string(Arc::new("127.0.0.1:0".to_string()))],
            &ctx,
        )
        .unwrap();

        // Get sock_b's address
        let addr_b = {
            let h = sock_b.as_io_handle().unwrap();
            let g = h.resource.lock().unwrap();
            match g.as_ref().unwrap() {
                IoResource::UdpSocket(s) => s.local_addr().unwrap().to_string(),
                _ => panic!(),
            }
        };

        // Set recv timeout on sock_b
        {
            let h = sock_b.as_io_handle().unwrap();
            let g = h.resource.lock().unwrap();
            if let Some(IoResource::UdpSocket(s)) = g.as_ref() {
                s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
            }
        }

        // Send from A to B
        let sent = io_udp_send(
            &[
                sock_a.clone(),
                ValueWord::from_string(Arc::new("hello udp".to_string())),
                ValueWord::from_string(Arc::new(addr_b)),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(sent.as_number_coerce(), Some(9.0)); // "hello udp" = 9 bytes

        // Receive on B
        let recv_result = io_udp_recv(&[sock_b.clone()], &ctx).unwrap();
        // recv_result is a TypedObject { data, addr }
        // We verify it's an object type
        assert_eq!(recv_result.type_name(), "object");

        // Close both
        io_tcp_close(&[sock_a], &ctx).unwrap();
        io_tcp_close(&[sock_b], &ctx).unwrap();
    }

    #[test]
    fn test_udp_bind_ephemeral() {
        let ctx = test_ctx();
        let handle = io_udp_bind(
            &[ValueWord::from_string(Arc::new("127.0.0.1:0".to_string()))],
            &ctx,
        )
        .unwrap();
        assert_eq!(handle.type_name(), "io_handle");

        // The path should show the actual bound address, not "127.0.0.1:0"
        let h = handle.as_io_handle().unwrap();
        assert!(h.path.starts_with("127.0.0.1:"));
        // Port should be non-zero since OS picks one
        let port: u16 = h.path.split(':').last().unwrap().parse().unwrap();
        assert!(port > 0);

        io_tcp_close(&[handle], &ctx).unwrap();
    }

    #[test]
    fn test_tcp_accept_on_non_listener() {
        let ctx = test_ctx();
        // Create a TCP stream handle and try accept -- should fail
        let listener = io_tcp_listen(
            &[ValueWord::from_string(Arc::new("127.0.0.1:0".to_string()))],
            &ctx,
        )
        .unwrap();
        let addr = {
            let h = listener.as_io_handle().unwrap();
            let g = h.resource.lock().unwrap();
            match g.as_ref().unwrap() {
                IoResource::TcpListener(l) => l.local_addr().unwrap().to_string(),
                _ => panic!(),
            }
        };

        let stream = io_tcp_connect(&[ValueWord::from_string(Arc::new(addr))], &ctx).unwrap();

        // Accept on the *stream* handle should fail
        let result = io_tcp_accept(&[stream.clone()], &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a TcpListener"));

        io_tcp_close(&[stream], &ctx).unwrap();
        io_tcp_close(&[listener], &ctx).unwrap();
    }
}
