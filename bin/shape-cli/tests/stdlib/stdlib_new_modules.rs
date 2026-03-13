//! Integration tests for new stdlib modules: csv, msgpack, set, crypto.
//!
//! These tests evaluate Shape code through the ShapeEngine, using `use std::core::<module>`
//! to import the native stdlib modules.

use crate::common::{eval_to_bool, eval_to_number, eval_to_string, init_runtime};

/// Eval helper that registers the csv extension module (not included by default).
fn eval_with_csv(code: &str) -> Result<serde_json::Value, String> {
    use shape_runtime::engine::ShapeEngine;
    use shape_vm::BytecodeExecutor;

    let mut engine = ShapeEngine::new().map_err(|e| e.to_string())?;
    engine.load_stdlib().map_err(|e| e.to_string())?;
    let mut executor = BytecodeExecutor::new();
    executor.register_extension(shape_runtime::stdlib::csv_module::create_csv_module());
    let result = engine
        .execute(&mut executor, code)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&result.value).map_err(|e| e.to_string())
}

fn eval_with_csv_to_string(code: &str) -> String {
    let val = eval_with_csv(code).unwrap_or_else(|e| panic!("Expected string, got error: {}", e));
    match val {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) if map.contains_key("String") => match &map["String"] {
            serde_json::Value::String(s) => s.clone(),
            other => panic!("Expected string in Object, got: {:?}", other),
        },
        other => panic!("Expected string, got: {:?}", other),
    }
}

fn eval_with_csv_to_bool(code: &str) -> bool {
    let val = eval_with_csv(code).unwrap_or_else(|e| panic!("Expected bool, got error: {}", e));
    match val {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Object(map) if map.contains_key("Bool") => match &map["Bool"] {
            serde_json::Value::Bool(b) => *b,
            other => panic!("Expected bool in Object, got: {:?}", other),
        },
        other => panic!("Expected bool, got: {:?}", other),
    }
}

// === CSV Module ===

#[test]
fn test_csv_parse() {
    init_runtime();
    assert!(eval_with_csv_to_bool(
        r#"
        use csv
        let rows = csv::parse("a,b,c\n1,2,3")
        rows[1][0] == "1"
    "#
    ));
}

#[test]
fn test_csv_parse_records() {
    init_runtime();
    assert!(eval_with_csv_to_bool(
        r#"
        use csv
        let records = csv::parse_records("name,age\nAlice,30")
        records[0]["name"] == "Alice"
    "#
    ));
}

#[test]
fn test_csv_stringify() {
    init_runtime();
    let result = eval_with_csv_to_string(
        r#"
        use csv
        csv::stringify([["x", "y"], ["1", "2"]])
    "#,
    );
    assert!(!result.is_empty());
}

#[test]
fn test_csv_is_valid() {
    init_runtime();
    assert!(eval_with_csv_to_bool(
        r#"
        use csv
        csv::is_valid("a,b\n1,2")
    "#
    ));
}

// === MessagePack Module ===

#[test]
fn test_msgpack_roundtrip_number() {
    init_runtime();
    assert!(eval_to_bool(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode(42)
        match encoded {
            Ok(data) => {
                let decoded = msgpack::decode(data)
                match decoded {
                    Ok(val) => val == 42,
                    Err(_) => false,
                }
            },
            Err(_) => false,
        }
    "#
    ));
}

#[test]
fn test_msgpack_roundtrip_string() {
    init_runtime();
    assert!(eval_to_bool(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode("hello")
        match encoded {
            Ok(data) => {
                let decoded = msgpack::decode(data)
                match decoded {
                    Ok(val) => val == "hello",
                    Err(_) => false,
                }
            },
            Err(_) => false,
        }
    "#
    ));
}

#[test]
fn test_msgpack_encode_decode_basic() {
    init_runtime();
    // Verify encode produces a non-empty hex string (Ok result)
    assert!(eval_to_bool(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode("test")
        match encoded {
            Ok(data) => len(data) > 0,
            Err(_) => false,
        }
    "#
    ));
}

// === Set Module ===

#[test]
fn test_set_from_array_dedup() {
    init_runtime();
    assert_eq!(
        eval_to_number(
            r#"
            use std::core::set
            let s = set::from_array([1, 2, 2, 3, 3, 3])
            set::size(s)
        "#
        ),
        3.0
    );
}

#[test]
fn test_set_contains() {
    init_runtime();
    assert!(eval_to_bool(
        r#"
        use std::core::set
        let s = set::from_array([1, 2, 3])
        set::contains(s, 2)
    "#
    ));
}

#[test]
fn test_set_union() {
    init_runtime();
    assert_eq!(
        eval_to_number(
            r#"
            use std::core::set
            let a = set::from_array([1, 2])
            let b = set::from_array([2, 3])
            set::size(set::union(a, b))
        "#
        ),
        3.0
    );
}

#[test]
fn test_set_intersection() {
    init_runtime();
    assert_eq!(
        eval_to_number(
            r#"
            use std::core::set
            let a = set::from_array([1, 2, 3])
            let b = set::from_array([2, 3, 4])
            set::size(set::intersection(a, b))
        "#
        ),
        2.0
    );
}

#[test]
fn test_set_difference() {
    init_runtime();
    assert_eq!(
        eval_to_number(
            r#"
            use std::core::set
            let a = set::from_array([1, 2, 3])
            let b = set::from_array([2, 3])
            set::size(set::difference(a, b))
        "#
        ),
        1.0
    );
}

// === Crypto Module (new functions) ===

#[test]
fn test_crypto_sha512() {
    init_runtime();
    let hash = eval_to_string(
        r#"
        use std::core::crypto
        crypto::sha512("hello")
    "#,
    );
    assert_eq!(hash.len(), 128); // 64 bytes hex-encoded
}

#[test]
fn test_crypto_sha1() {
    init_runtime();
    let hash = eval_to_string(
        r#"
        use std::core::crypto
        crypto::sha1("hello")
    "#,
    );
    assert_eq!(hash, "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
}

#[test]
fn test_crypto_md5() {
    init_runtime();
    let hash = eval_to_string(
        r#"
        use std::core::crypto
        crypto::md5("hello")
    "#,
    );
    assert_eq!(hash, "5d41402abc4b2a76b9719d911017c592");
}

#[test]
fn test_crypto_random_bytes() {
    init_runtime();
    let hex = eval_to_string(
        r#"
        use std::core::crypto
        crypto::random_bytes(16)
    "#,
    );
    assert_eq!(hex.len(), 32); // 16 bytes = 32 hex chars
}

#[test]
fn test_crypto_ed25519_roundtrip() {
    init_runtime();
    assert!(eval_to_bool(
        r#"
        use std::core::crypto
        let kp = crypto::ed25519_generate_keypair()
        let sig = crypto::ed25519_sign("test message", kp["secret_key"])
        crypto::ed25519_verify("test message", sig, kp["public_key"])
    "#
    ));
}
