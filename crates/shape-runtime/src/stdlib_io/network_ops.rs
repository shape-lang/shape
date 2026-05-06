//! Network operation implementations for the io module.
//!
//! Phase 2d migration: ported to the typed marshal layer (cluster #2
//! option γ for IoHandle-touching functions). Mirrors the shape used by
//! `file_ops.rs::register_file_io_handle_ops` — `Arc<IoHandleData>`
//! parameters via `FromSlot`, `register_typed_fn_N` / `_N_full` for
//! optional `n`, returns built from `ConcreteReturn::IoHandle` /
//! `String` / `I64` / `Bool` and `TypedReturn::TypedObject` for
//! `udp_recv`'s `{data, addr}` shape.
//!
//! TCP: tcp_connect, tcp_listen, tcp_accept, tcp_read, tcp_write, tcp_close
//! UDP: udp_bind, udp_send, udp_recv
//!
//! All operations use blocking std::net (not tokio).

use crate::marshal::{
    register_typed_fn_1, register_typed_fn_2, register_typed_fn_2_full, register_typed_fn_3,
};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{IoHandleData, IoResource};
use std::io::{Read, Write};
use std::sync::Arc;

/// Register the 9 network IO functions on the io module.
/// Cluster #2 (option γ) per docs/defections.md 2026-05-06.
pub fn register_network_io(module: &mut ModuleExports) {
    // ── TCP ────────────────────────────────────────────────────────────────

    // io.tcp_connect(addr: string) -> IoHandle
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "tcp_connect",
        "Connect to a TCP server",
        "addr",
        "string",
        ConcreteType::IoHandle,
        |addr, ctx| {
            let addr = addr.as_str();
            crate::module_exports::check_net_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
                addr,
            )?;
            let stream = std::net::TcpStream::connect(addr)
                .map_err(|e| format!("io.tcp_connect(\"{}\"): {}", addr, e))?;
            let handle = IoHandleData::new_tcp_stream(stream, addr.to_string());
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                handle,
            ))))
        },
    );

    // io.tcp_listen(addr: string) -> IoHandle
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "tcp_listen",
        "Bind a TCP listener",
        "addr",
        "string",
        ConcreteType::IoHandle,
        |addr, ctx| {
            let addr = addr.as_str();
            crate::module_exports::check_net_permission(
                ctx,
                shape_abi_v1::Permission::NetListen,
                addr,
            )?;
            let listener = std::net::TcpListener::bind(addr)
                .map_err(|e| format!("io.tcp_listen(\"{}\"): {}", addr, e))?;
            let handle = IoHandleData::new_tcp_listener(listener, addr.to_string());
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                handle,
            ))))
        },
    );

    // io.tcp_accept(listener: IoHandle) -> IoHandle
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "tcp_accept",
        "Accept the next incoming TCP connection",
        "listener",
        "IoHandle",
        ConcreteType::IoHandle,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetListen)?;
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
                    Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                        client,
                    ))))
                }
                _ => Err("io.tcp_accept(): handle is not a TcpListener".to_string()),
            }
        },
    );

    // io.tcp_read(handle: IoHandle, n?: int) -> string
    register_typed_fn_2_full::<_, Arc<IoHandleData>, i64>(
        module,
        "tcp_read",
        "Read up to n bytes from a TCP stream",
        [
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
                description: "Max bytes to read (default: 65536)".to_string(),
                default_snippet: Some("65536".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |handle, n, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
            let mut guard = handle
                .resource
                .lock()
                .map_err(|_| "io.tcp_read(): lock poisoned".to_string())?;
            let resource = guard
                .as_mut()
                .ok_or_else(|| "io.tcp_read(): handle is closed".to_string())?;
            match resource {
                IoResource::TcpStream(stream) => {
                    let buf_size = if n > 0 { n as usize } else { 65536 };
                    let mut buf = vec![0u8; buf_size];
                    let bytes_read = stream
                        .read(&mut buf)
                        .map_err(|e| format!("io.tcp_read(): {}", e))?;
                    buf.truncate(bytes_read);
                    let s = String::from_utf8(buf)
                        .map_err(|e| format!("io.tcp_read(): invalid UTF-8: {}", e))?;
                    Ok(TypedReturn::Concrete(ConcreteReturn::String(s)))
                }
                _ => Err("io.tcp_read(): handle is not a TcpStream".to_string()),
            }
        },
    );

    // io.tcp_write(handle: IoHandle, data: string) -> int
    register_typed_fn_2::<_, Arc<IoHandleData>, Arc<String>>(
        module,
        "tcp_write",
        "Write a string to a TCP stream, returning bytes written",
        [("handle", "IoHandle"), ("data", "string")],
        ConcreteType::Int,
        |handle, data, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
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
                    Ok(TypedReturn::Concrete(ConcreteReturn::I64(written as i64)))
                }
                _ => Err("io.tcp_write(): handle is not a TcpStream".to_string()),
            }
        },
    );

    // io.tcp_close(handle: IoHandle) -> bool
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "tcp_close",
        "Close a TCP stream or listener, returning whether it was open",
        "handle",
        "IoHandle",
        ConcreteType::Bool,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(handle.close())))
        },
    );

    // ── UDP ────────────────────────────────────────────────────────────────

    // io.udp_bind(addr: string) -> IoHandle
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "udp_bind",
        "Bind a UDP socket to addr",
        "addr",
        "string",
        ConcreteType::IoHandle,
        |addr, ctx| {
            let addr = addr.as_str();
            crate::module_exports::check_net_permission(
                ctx,
                shape_abi_v1::Permission::NetListen,
                addr,
            )?;
            let socket = std::net::UdpSocket::bind(addr)
                .map_err(|e| format!("io.udp_bind(\"{}\"): {}", addr, e))?;
            let local = socket
                .local_addr()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| addr.to_string());
            let handle = IoHandleData::new_udp_socket(socket, local);
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(
                handle,
            ))))
        },
    );

    // io.udp_send(handle: IoHandle, data: string, target: string) -> int
    register_typed_fn_3::<_, Arc<IoHandleData>, Arc<String>, Arc<String>>(
        module,
        "udp_send",
        "Send a UDP datagram to target, returning bytes sent",
        [
            ("handle", "IoHandle"),
            ("data", "string"),
            ("target", "string"),
        ],
        ConcreteType::Int,
        |handle, data, target, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
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
                        .send_to(data.as_bytes(), target.as_str())
                        .map_err(|e| format!("io.udp_send(): {}", e))?;
                    Ok(TypedReturn::Concrete(ConcreteReturn::I64(sent as i64)))
                }
                _ => Err("io.udp_send(): handle is not a UdpSocket".to_string()),
            }
        },
    );

    // io.udp_recv(handle: IoHandle, n?: int) -> object { data: string, addr: string }
    register_typed_fn_2_full::<_, Arc<IoHandleData>, i64>(
        module,
        "udp_recv",
        "Receive a UDP datagram, returning {data, addr}",
        [
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
                description: "Max receive buffer size (default: 65536)".to_string(),
                default_snippet: Some("65536".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::TypedObject,
        |handle, n, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
            let guard = handle
                .resource
                .lock()
                .map_err(|_| "io.udp_recv(): lock poisoned".to_string())?;
            let resource = guard
                .as_ref()
                .ok_or_else(|| "io.udp_recv(): handle is closed".to_string())?;
            match resource {
                IoResource::UdpSocket(socket) => {
                    let buf_size = if n > 0 { n as usize } else { 65536 };
                    let mut buf = vec![0u8; buf_size];
                    let (bytes_read, src_addr) = socket
                        .recv_from(&mut buf)
                        .map_err(|e| format!("io.udp_recv(): {}", e))?;
                    buf.truncate(bytes_read);
                    let data = String::from_utf8(buf)
                        .map_err(|e| format!("io.udp_recv(): invalid UTF-8: {}", e))?;
                    Ok(TypedReturn::TypedObject(vec![
                        ("data".to_string(), ConcreteReturn::String(data)),
                        (
                            "addr".to_string(),
                            ConcreteReturn::String(src_addr.to_string()),
                        ),
                    ]))
                }
                _ => Err("io.udp_recv(): handle is not a UdpSocket".to_string()),
            }
        },
    );
}
