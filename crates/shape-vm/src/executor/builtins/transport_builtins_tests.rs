use super::*;
use shape_value::ValueWordExt;
use std::net::TcpListener;

fn test_ctx() -> ModuleContext<'static> {
    let registry = Box::leak(Box::new(
        shape_runtime::type_schema::TypeSchemaRegistry::new(),
    ));
    ModuleContext {
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
fn test_create_transport_module() {
    let module = create_transport_module();
    assert_eq!(module.name, "std::core::transport");
    assert!(module.has_export("tcp"));
    assert!(module.has_export("send"));
    assert!(module.has_export("connect"));
    assert!(module.has_export("connection_send"));
    assert!(module.has_export("connection_recv"));
    assert!(module.has_export("connection_close"));
    assert!(module.has_export("memoized"));
    assert!(module.has_export("memo_stats"));
    assert!(module.has_export("memo_invalidate"));
}

#[test]
fn test_transport_tcp_creates_handle() {
    let ctx = test_ctx();
    let result = transport_tcp(&[], &ctx);
    assert!(result.is_ok());
    let handle = result.unwrap();
    assert!(handle.as_io_handle().is_some());
}

#[test]
fn test_nanboxed_array_to_bytes_and_back() {
    let original = vec![0u8, 1, 127, 255, 42];
    let nb = bytes_to_nanboxed_array(&original);
    let roundtrip = nanboxed_array_to_bytes(&nb).unwrap();
    assert_eq!(original, roundtrip);
}

#[test]
fn test_nanboxed_array_to_bytes_out_of_range() {
    let arr = ValueWord::from_array(Arc::new(vec![ValueWord::from_i64(256)]));
    let result = nanboxed_array_to_bytes(&arr);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("out of range"));
}

#[test]
fn test_length_prefixed_roundtrip() {
    use shape_wire::transport::tcp::{read_length_prefixed, write_length_prefixed};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let payload = b"hello transport";

    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        let data = read_length_prefixed(&mut conn).unwrap();
        write_length_prefixed(&mut conn, &data).unwrap();
    });

    let mut stream = std::net::TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    write_length_prefixed(&mut stream, payload).unwrap();
    let response = read_length_prefixed(&mut stream).unwrap();

    assert_eq!(&response, payload);
    server.join().unwrap();
}

#[test]
fn test_transport_connect_and_send_recv() {
    use shape_wire::transport::tcp::{read_length_prefixed, write_length_prefixed};

    let ctx = test_ctx();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let data = read_length_prefixed(&mut conn).unwrap();
        write_length_prefixed(&mut conn, &data).unwrap();
    });

    let transport = transport_tcp(&[], &ctx).unwrap();

    let conn_result = transport_connect(
        &[transport.clone(), ValueWord::from_string(Arc::new(addr))],
        &ctx,
    )
    .unwrap();

    let conn = match conn_result.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Ok(inner)) => (**inner).clone(),
        _ => panic!("expected Ok result from transport.connect()"),
    };

    let payload = bytes_to_nanboxed_array(b"test data");
    let send_result = connection_send_fn(&[conn.clone(), payload], &ctx);
    assert!(send_result.is_ok());

    let recv_result = connection_recv_fn(&[conn.clone(), ValueWord::from_i64(5000)], &ctx).unwrap();

    let data_nb = match recv_result.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Ok(inner)) => (**inner).clone(),
        _ => panic!("expected Ok result from transport.connection_recv()"),
    };
    let data_bytes = nanboxed_array_to_bytes(&data_nb).unwrap();
    assert_eq!(&data_bytes, b"test data");

    let close_result = connection_close_fn(&[conn], &ctx);
    assert!(close_result.is_ok());

    server.join().unwrap();
}

#[test]
fn test_transport_send_one_shot() {
    use shape_wire::transport::tcp::{read_length_prefixed, write_length_prefixed};

    let ctx = test_ctx();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let data = read_length_prefixed(&mut conn).unwrap();
        let mut response = b"reply:".to_vec();
        response.extend_from_slice(&data);
        write_length_prefixed(&mut conn, &response).unwrap();
    });

    let transport = transport_tcp(&[], &ctx).unwrap();
    let payload = bytes_to_nanboxed_array(b"ping");

    let result = transport_send(
        &[transport, ValueWord::from_string(Arc::new(addr)), payload],
        &ctx,
    )
    .unwrap();

    let data_nb = match result.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Ok(inner)) => (**inner).clone(),
        _ => panic!("expected Ok result from transport.send()"),
    };
    let response_bytes = nanboxed_array_to_bytes(&data_nb).unwrap();
    assert_eq!(&response_bytes, b"reply:ping");

    server.join().unwrap();
}

#[test]
fn test_transport_connect_refused() {
    let ctx = test_ctx();
    let transport = transport_tcp(&[], &ctx).unwrap();

    let result = transport_connect(
        &[
            transport,
            ValueWord::from_string(Arc::new("127.0.0.1:1".to_string())),
        ],
        &ctx,
    );
    // Connection refused now returns Ok(Err(...)) instead of Err(...)
    // so users can handle it with ? or pattern matching
    assert!(
        result.is_ok(),
        "transport_connect should return Ok even on connection failure"
    );
    let val = result.unwrap();
    match val.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Err(_)) => {
            // Expected: Result::Err with error message
        }
        other => panic!("expected Result::Err, got: {:?}", other),
    }
}

#[test]
fn test_connection_send_on_closed() {
    let ctx = test_ctx();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server = std::thread::spawn(move || {
        let (_conn, _) = listener.accept().unwrap();
    });

    let transport = transport_tcp(&[], &ctx).unwrap();
    let conn_result =
        transport_connect(&[transport, ValueWord::from_string(Arc::new(addr))], &ctx).unwrap();

    let conn = match conn_result.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Ok(inner)) => (**inner).clone(),
        _ => panic!("expected Ok"),
    };

    connection_close_fn(&[conn.clone()], &ctx).unwrap();

    let payload = bytes_to_nanboxed_array(b"data");
    let result = connection_send_fn(&[conn, payload], &ctx);
    assert!(result.is_err());

    server.join().unwrap();
}

#[test]
fn test_memoized_creates_handle() {
    let ctx = test_ctx();
    let result = transport_memoized(&[], &ctx);
    assert!(result.is_ok());
    let handle_nb = result.unwrap();
    let handle = handle_nb.as_io_handle().unwrap();
    assert_eq!(handle.kind, IoHandleKind::Custom);
    assert_eq!(handle.path, "transport:memoized_tcp");
}

#[test]
fn test_memoized_with_max_entries() {
    let ctx = test_ctx();
    let result = transport_memoized(&[ValueWord::from_i64(512)], &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_memoized_invalid_max_entries() {
    let ctx = test_ctx();
    let result = transport_memoized(&[ValueWord::from_i64(0)], &ctx);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("max_entries must be >= 1"));
}

#[test]
fn test_memo_stats_initial() {
    let ctx = test_ctx();
    let handle_nb = transport_memoized(&[], &ctx).unwrap();
    let stats = transport_memo_stats(&[handle_nb], &ctx).unwrap();
    let arr = stats.to_array_arc().unwrap();
    assert_eq!(arr.len(), 4);
    // All stats should be 0 initially
    for i in 0..4 {
        assert_eq!(arr[i].as_i64().unwrap(), 0);
    }
}

#[test]
fn test_memo_invalidate() {
    let ctx = test_ctx();
    let handle_nb = transport_memoized(&[], &ctx).unwrap();
    let result = transport_memo_invalidate(&[handle_nb], &ctx);
    assert!(result.is_ok());
}

#[test]
fn test_memoized_send_caches_results() {
    use shape_wire::transport::tcp::{read_length_prefixed, write_length_prefixed};

    let ctx = test_ctx();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    // Server echoes back with "reply:" prefix. Only accept one connection.
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let data = read_length_prefixed(&mut conn).unwrap();
        let mut response = b"reply:".to_vec();
        response.extend_from_slice(&data);
        write_length_prefixed(&mut conn, &response).unwrap();
    });

    let handle_nb = transport_memoized(&[], &ctx).unwrap();
    let payload = bytes_to_nanboxed_array(b"ping");

    // First send -- goes to network.
    let r1 = transport_send(
        &[
            handle_nb.clone(),
            ValueWord::from_string(Arc::new(addr.clone())),
            payload.clone(),
        ],
        &ctx,
    )
    .unwrap();

    let data1 = match r1.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Ok(inner)) => (**inner).clone(),
        _ => panic!("expected Ok result"),
    };
    let bytes1 = nanboxed_array_to_bytes(&data1).unwrap();
    assert_eq!(&bytes1, b"reply:ping");

    // Second send -- should come from cache (server already closed).
    server.join().unwrap();

    let r2 = transport_send(
        &[
            handle_nb.clone(),
            ValueWord::from_string(Arc::new(addr)),
            payload,
        ],
        &ctx,
    )
    .unwrap();

    let data2 = match r2.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Ok(inner)) => (**inner).clone(),
        _ => panic!("expected Ok result"),
    };
    let bytes2 = nanboxed_array_to_bytes(&data2).unwrap();
    assert_eq!(&bytes2, b"reply:ping");

    // Check stats: 1 hit, 1 miss, 2 total
    let stats = transport_memo_stats(&[handle_nb], &ctx).unwrap();
    let arr = stats.to_array_arc().unwrap();
    assert_eq!(arr[0].as_i64().unwrap(), 1); // cache_hits
    assert_eq!(arr[1].as_i64().unwrap(), 1); // cache_misses
    assert_eq!(arr[3].as_i64().unwrap(), 2); // total_requests
}

#[test]
fn test_transport_send_error_returns_result_err() {
    let ctx = test_ctx();
    let transport = transport_tcp(&[], &ctx).unwrap();
    let payload = bytes_to_nanboxed_array(b"data");

    // Send to a port that won't be listening
    let result = transport_send(
        &[
            transport,
            ValueWord::from_string(Arc::new("127.0.0.1:1".to_string())),
            payload,
        ],
        &ctx,
    );
    // Should return Ok(Err(...)) not a runtime error
    assert!(
        result.is_ok(),
        "transport_send should return Ok even on network failure"
    );
    let val = result.unwrap();
    match val.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Err(_)) => {
            // Expected: transport error wrapped in Result::Err
        }
        other => panic!("expected Result::Err, got: {:?}", other),
    }
}

#[test]
fn test_transport_builtins_has_no_tcpstream_fallback() {
    let src = include_str!("transport_builtins.rs");
    assert!(
        !src.contains("IoResource::TcpStream"),
        "transport_builtins must not bypass shape-wire Connection via IoResource::TcpStream"
    );
}

#[cfg(feature = "quic")]
#[test]
fn test_transport_quic_requires_config() {
    let ctx = test_ctx();
    let err = transport_quic(&[], &ctx).unwrap_err();
    assert!(err.contains("not configured"));
}
