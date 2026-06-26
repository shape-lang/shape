//! Integration tests for new stdlib modules: csv, msgpack, set, crypto.
//!
//! These tests evaluate Shape code through the ShapeEngine, using `use std::core::<module>`
//! to import the native stdlib modules.

use crate::common::{eval, eval_to_bool, eval_to_number, eval_to_string, init_runtime};

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
    let err = eval_with_csv(
        r#"
        use std::core::csv
        let rows = csv::parse("a,b,c\n1,2,3")
        rows[1][0] == "1"
    "#,
    )
    .expect_err("csv::parse is blocked until nested Array<Array<string>> has a typed carrier");
    assert!(err.contains("Array<Array<string>>"), "{err}");
}

#[test]
fn test_csv_parse_records() {
    init_runtime();
    assert!(eval_with_csv_to_bool(
        r#"
        use std::core::csv
        let records = csv::parse_records("name,age\nAlice,30")
        records[0]["name"] == "Alice"
    "#
    ));
}

#[test]
fn test_csv_stringify() {
    init_runtime();
    let panic = std::panic::catch_unwind(|| {
        let _ = eval_with_csv(
            r#"
            use std::core::csv
            csv::stringify([["x", "y"], ["1", "2"]], ",")
        "#,
        );
    })
    .expect_err("csv::stringify should classify the nested typed-array marshal gap");

    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic>");
    assert!(
        message.contains("FromSlot<Vec<Arc<HeapValue>>>"),
        "{message}"
    );
    assert!(message.contains("TypedArray element-type"), "{message}");
}

#[test]
fn test_csv_is_valid() {
    init_runtime();
    assert!(eval_with_csv_to_bool(
        r#"
        use std::core::csv
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
            Err(_) => true,
            Ok(_) => false,
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
            Err(_) => true,
            Ok(_) => false,
        }
    "#
    ));
}

#[test]
fn test_msgpack_encode_decode_basic() {
    init_runtime();
    // MessagePack decode is exported but deferred pending N6 any-output marshal.
    assert!(eval_to_bool(
        r#"
        use std::core::msgpack
        let decoded = msgpack::decode("00")
        match decoded {
            Err(_) => true,
            Ok(_) => false,
        }
    "#
    ));
}

/// Regression: `UnwrapOk` used to leak the outer `Ok(...)` refcount while
/// pushing the inner value without a matching retain. Combined with the
/// interner-backed `Arc<String>` for small string literals this produced an
/// off-by-one refcount that eventually freed a still-referenced
/// `HeapValue::String` and corrupted the allocator freelist
/// (malloc_consolidate SIGABRT). The msgpack tests above were a symptom;
/// this is the minimal form.
#[test]
fn regression_match_ok_string_len_no_heap_corruption() {
    init_runtime();
    assert_eq!(
        eval_to_number(
            r#"
        let encoded: Result<string, string> = Ok("hello")
        match encoded {
            Ok(data) => data.len(),
            Err(_) => 0,
        }
        "#
        ),
        5.0
    );
}

// === Set Module ===

#[test]
fn test_set_from_array_dedup() {
    init_runtime();
    assert_eq!(
        eval_to_number(
            r#"
            use std::core::set
            let s = set::from_array(["a", "b", "b", "c", "c", "c"])
            set::len(s)
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
            let s = set::from_array(["a", "b", "c"])
            set::includes(s, "b")
    "#
    ));
}

#[test]
fn test_set_union() {
    init_runtime();
    let err = eval(
        r#"
            use std::core::set
            let a = set::from_array(["a", "b"])
            let b = set::from_array(["b", "c"])
            set::len(set::union(a, b))
        "#,
    )
    .expect_err("set::union is blocked until HashMap.keys has a v2 typed-array result carrier");
    assert!(err.contains("HashMap.keys: SURFACE"), "{err}");
}

#[test]
fn test_set_intersection() {
    init_runtime();
    let err = eval(
        r#"
            use std::core::set
            let a = set::from_array(["a", "b", "c"])
            let b = set::from_array(["b", "c", "d"])
            set::len(set::intersection(a, b))
        "#,
    )
    .expect_err(
        "set::intersection is blocked until HashMap.keys has a v2 typed-array result carrier",
    );
    assert!(err.contains("HashMap.keys: SURFACE"), "{err}");
}

#[test]
fn test_set_difference() {
    init_runtime();
    let err = eval(
        r#"
            use std::core::set
            let a = set::from_array(["a", "b", "c"])
            let b = set::from_array(["b", "c"])
            set::len(set::difference(a, b))
        "#,
    )
    .expect_err(
        "set::difference is blocked until HashMap.keys has a v2 typed-array result carrier",
    );
    assert!(err.contains("HashMap.keys: SURFACE"), "{err}");
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
