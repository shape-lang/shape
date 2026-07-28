#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use shape_runtime::snapshot::SerializableVMValue;
use shape_vm::bytecode::{BytecodeProgram, FunctionBlob, FunctionHash};
use shape_vm::remote::{
    BlobNegotiationRequest, RemoteCallError, RemoteCallRequest, RemoteErrorKind, WireMessage,
    build_call_request,
};
use support::*;

#[test]
fn content_addressed_missing_dependency_resupplies_over_real_socket() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let program = compile_shape(
        r#"
fn helper(x: int) -> int { x + 1 }

fn main_fn(x: int) -> int {
    helper(x) + helper(x)
}
"#,
    );
    let full_request = build_call_request(&program, "main_fn", vec![SerializableVMValue::Int(10)]);
    let entry_hash = full_request
        .function_hash
        .expect("main_fn should have a content-addressed hash");
    let full_blobs = full_request
        .function_blobs
        .as_ref()
        .expect("main_fn request should carry content-addressed blobs")
        .clone();
    assert!(
        full_blobs.len() >= 2,
        "test setup should include entry plus helper blobs, got {}",
        full_blobs.len()
    );

    let mut stripped = full_request.clone();
    stripped
        .function_blobs
        .as_mut()
        .expect("stripped request still has a blob vector")
        .retain(|(hash, _)| *hash == entry_hash);
    assert_eq!(
        stripped.function_blobs.as_ref().unwrap().len(),
        1,
        "stripped first request should carry only the entry blob"
    );

    let mut client = WireClient::connect(&server.addr);
    let first = client.roundtrip(&WireMessage::Call(stripped.clone()));
    let missing = missing_blob_hashes(first);
    assert!(
        missing.iter().any(|hash| *hash != entry_hash),
        "receiver should report at least one missing helper blob; missing={missing:?}"
    );

    let resupply_blobs = blobs_for_hashes(&full_blobs, entry_hash, &missing);
    assert!(
        resupply_blobs.len() > 1,
        "resupply request should include the entry blob plus reported missing blobs"
    );
    let mut resupplied: RemoteCallRequest = stripped;
    resupplied.function_blobs = Some(resupply_blobs.clone());

    let second = client.roundtrip(&WireMessage::Call(resupplied));
    assert_int_response(second, 22);

    let offered_hashes = full_blobs.iter().map(|(hash, _)| *hash).collect::<Vec<_>>();
    let negotiation = client.roundtrip(&WireMessage::BlobNegotiation(BlobNegotiationRequest {
        offered_hashes: offered_hashes.clone(),
    }));
    let WireMessage::BlobNegotiationReply(negotiation) = negotiation else {
        panic!("expected BlobNegotiationReply after resupply, got {negotiation:?}");
    };
    assert_eq!(
        negotiation.known_hashes, offered_hashes,
        "verified resupply blobs should be reusable on this connection"
    );

    let mut reused = full_request;
    reused.function_blobs = Some(vec![]);
    assert_int_response(client.roundtrip(&WireMessage::Call(reused)), 22);

    let serve_log = serve_stderr(&server);
    assert!(
        serve_log.contains("blobs=1")
            && serve_log.contains(&format!("blobs={}", resupply_blobs.len())),
        "serve log should prove both real inbound content-addressed calls landed; stderr:\n{serve_log}"
    );
}

#[test]
fn content_addressed_transfers_nested_map_closure_over_real_socket() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let program = compile_shape(
        r#"
fn double_all(data: Array<int>) -> Array<int> {
    data.map(|x| x * 2)
}
"#,
    );
    let full_request = build_call_request(
        &program,
        "double_all",
        vec![SerializableVMValue::Array(vec![
            SerializableVMValue::Int(1),
            SerializableVMValue::Int(2),
            SerializableVMValue::Int(3),
        ])],
    );
    let entry_hash = full_request
        .function_hash
        .expect("double_all should have a content-addressed hash");
    let full_blobs = full_request
        .function_blobs
        .as_ref()
        .expect("double_all request should carry content-addressed blobs")
        .clone();
    let entry_blob = full_blobs
        .iter()
        .find(|(hash, _)| *hash == entry_hash)
        .map(|(_, blob)| blob)
        .expect("minimal blob set should contain double_all");
    let closure_hashes = entry_blob
        .dependencies
        .iter()
        .copied()
        .filter(|hash| {
            full_blobs.iter().any(|(available, blob)| {
                available == hash && blob.is_closure && blob.captures_count == 0
            })
        })
        .collect::<Vec<_>>();
    assert!(
        !closure_hashes.is_empty(),
        "minimal blob set should include the zero-capture map closure; blobs={:?}",
        full_blobs
            .iter()
            .map(|(hash, blob)| (hash, &blob.name, blob.is_closure))
            .collect::<Vec<_>>()
    );

    let mut stripped = full_request.clone();
    stripped
        .function_blobs
        .as_mut()
        .expect("stripped request still has a blob vector")
        .retain(|(hash, _)| *hash == entry_hash);
    assert_eq!(
        stripped.function_blobs.as_ref().unwrap().len(),
        1,
        "stripped first request should carry only the entry blob"
    );

    let mut client = WireClient::connect(&server.addr);
    let first = client.roundtrip(&WireMessage::Call(stripped.clone()));
    let missing = missing_blob_hashes(first);
    assert!(
        missing.iter().any(|hash| closure_hashes.contains(hash)),
        "receiver should report the missing nested closure blob; missing={missing:?}"
    );

    let resupply_blobs = blobs_for_hashes(&full_blobs, entry_hash, &missing);
    let mut resupplied: RemoteCallRequest = stripped;
    resupplied.function_blobs = Some(resupply_blobs);
    assert_array_response(client.roundtrip(&WireMessage::Call(resupplied)), &[2, 4, 6]);

    let offered_hashes = full_blobs.iter().map(|(hash, _)| *hash).collect::<Vec<_>>();
    let negotiation = client.roundtrip(&WireMessage::BlobNegotiation(BlobNegotiationRequest {
        offered_hashes: offered_hashes.clone(),
    }));
    let WireMessage::BlobNegotiationReply(negotiation) = negotiation else {
        panic!("expected BlobNegotiationReply after resupply, got {negotiation:?}");
    };
    assert_eq!(
        negotiation.known_hashes, offered_hashes,
        "verified nested-closure blobs should be reusable on this connection"
    );

    let mut reused = full_request;
    reused.function_blobs = Some(vec![]);
    assert_array_response(client.roundtrip(&WireMessage::Call(reused)), &[2, 4, 6]);
}

fn compile_shape(source: &str) -> BytecodeProgram {
    let program = shape_ast::parser::parse_program(source).expect("parse Shape source");
    shape_vm::BytecodeCompiler::new()
        .compile(&program)
        .expect("compile Shape source")
}

fn missing_blob_hashes(response: WireMessage) -> Vec<FunctionHash> {
    let WireMessage::CallResponse(response) = response else {
        panic!("expected CallResponse for stripped call, got {response:?}");
    };
    match response.result {
        Err(RemoteCallError {
            kind: RemoteErrorKind::MissingModuleFunction,
            missing_blobs: Some(missing),
            message,
        }) => {
            assert!(
                !missing.is_empty(),
                "MissingModuleFunction should name missing blobs; message={message}"
            );
            missing
        }
        other => panic!(
            "stripped call should return MissingModuleFunction with missing blobs, got {other:?}"
        ),
    }
}

fn blobs_for_hashes(
    full_blobs: &[(FunctionHash, FunctionBlob)],
    entry_hash: FunctionHash,
    missing: &[FunctionHash],
) -> Vec<(FunctionHash, FunctionBlob)> {
    let resupply = full_blobs
        .iter()
        .filter(|(hash, _)| *hash == entry_hash || missing.contains(hash))
        .cloned()
        .collect::<Vec<_>>();
    for hash in missing {
        assert!(
            resupply.iter().any(|(available, _)| available == hash),
            "sender should be able to resupply receiver-reported blob {hash}"
        );
    }
    resupply
}

fn assert_int_response(response: WireMessage, expected: i64) {
    let WireMessage::CallResponse(response) = response else {
        panic!("expected CallResponse for resupplied call, got {response:?}");
    };
    match response.result {
        Ok(SerializableVMValue::Int(value)) => assert_eq!(value, expected),
        other => panic!("resupplied call should return Int({expected}), got {other:?}"),
    }
}

fn assert_array_response(response: WireMessage, expected: &[i64]) {
    let WireMessage::CallResponse(response) = response else {
        panic!("expected CallResponse for mapped call, got {response:?}");
    };
    match response.result {
        Ok(SerializableVMValue::Array(values)) => {
            let actual = values
                .into_iter()
                .map(|value| match value {
                    SerializableVMValue::Int(value) => value,
                    other => panic!("mapped array should contain only Int values, got {other:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "mapped array result should be exact");
        }
        other => panic!("mapped call should return an integer array, got {other:?}"),
    }
}
