//! Native `crypto` module for hashing, encoding, and signing utilities.
//!
//! Exports: crypto.sha256, crypto.sha512, crypto.sha1, crypto.md5,
//!          crypto.hmac_sha256, crypto.base64_encode, crypto.base64_decode,
//!          crypto.hex_encode, crypto.hex_decode, crypto.random_bytes,
//!          crypto.ed25519_generate_keypair, crypto.ed25519_sign, crypto.ed25519_verify
//!
//! Phase 2c: migrated to the typed marshal layer
//! (`crate::marshal::register_typed_fn_N`).

use crate::marshal::{register_typed_fn_0, register_typed_fn_1, register_typed_fn_2, register_typed_fn_3};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Create the `crypto` module with hashing and encoding functions.
pub fn create_crypto_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::crypto");
    module.description = "Cryptographic hashing and encoding utilities".to_string();

    // crypto.sha256(data: string) -> string
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "sha256",
        "Compute the SHA-256 hash of a string, returning a hex-encoded digest",
        "data",
        "string",
        ConcreteType::String,
        |data, _ctx| {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(result))))
        },
    );

    // crypto.hmac_sha256(data: string, key: string) -> string
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "hmac_sha256",
        "Compute HMAC-SHA256 of data with the given key, returning hex digest",
        [("data", "string"), ("key", "string")],
        ConcreteType::String,
        |data, key, _ctx| {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;
            let mut mac = HmacSha256::new_from_slice(key.as_bytes())
                .map_err(|e| format!("crypto.hmac_sha256() key error: {}", e))?;
            mac.update(data.as_bytes());
            let result = mac.finalize();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(result.into_bytes()))))
        },
    );

    // crypto.base64_encode(data: string) -> string
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "base64_encode",
        "Encode a string to Base64",
        "data",
        "string",
        ConcreteType::String,
        |data, _ctx| {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
            Ok(TypedReturn::Concrete(ConcreteReturn::String(encoded)))
        },
    );

    // crypto.base64_decode(encoded: string) -> Result<string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "base64_decode",
        "Decode a Base64 string",
        "encoded",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |encoded, _ctx| {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded.as_str())
                .map_err(|e| format!("crypto.base64_decode() failed: {}", e))?;
            let decoded = String::from_utf8(bytes)
                .map_err(|e| format!("crypto.base64_decode() invalid UTF-8: {}", e))?;
            Ok(TypedReturn::Ok(ConcreteReturn::String(decoded)))
        },
    );

    // crypto.hex_encode(data: string) -> string
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "hex_encode",
        "Encode a string as hexadecimal",
        "data",
        "string",
        ConcreteType::String,
        |data, _ctx| {
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(data.as_bytes()))))
        },
    );

    // crypto.hex_decode(hex: string) -> Result<string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "hex_decode",
        "Decode a hexadecimal string",
        "hex",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |hex_str, _ctx| {
            let bytes = hex::decode(hex_str.as_str())
                .map_err(|e| format!("crypto.hex_decode() failed: {}", e))?;
            let decoded = String::from_utf8(bytes)
                .map_err(|e| format!("crypto.hex_decode() invalid UTF-8: {}", e))?;
            Ok(TypedReturn::Ok(ConcreteReturn::String(decoded)))
        },
    );

    // crypto.sha512(data: string) -> string
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "sha512",
        "Compute the SHA-512 hash of a string, returning a hex-encoded digest",
        "data",
        "string",
        ConcreteType::String,
        |data, _ctx| {
            use sha2::{Digest, Sha512};
            let mut hasher = Sha512::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(result))))
        },
    );

    // crypto.sha1(data: string) -> string
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "sha1",
        "Compute the SHA-1 hash of a string, returning a hex-encoded digest (legacy)",
        "data",
        "string",
        ConcreteType::String,
        |data, _ctx| {
            use sha1::Digest;
            let mut hasher = sha1::Sha1::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(result))))
        },
    );

    // crypto.md5(data: string) -> string
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "md5",
        "Compute the MD5 hash of a string, returning a hex-encoded digest (legacy)",
        "data",
        "string",
        ConcreteType::String,
        |data, _ctx| {
            use md5::Digest;
            let mut hasher = md5::Md5::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(result))))
        },
    );

    // crypto.random_bytes(n: int) -> string
    register_typed_fn_1::<_, i64>(
        &mut module,
        "random_bytes",
        "Generate n random bytes, returned as a hex-encoded string",
        "n",
        "int",
        ConcreteType::String,
        |n, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Random)?;
            use rand::RngCore;
            if n < 0 || n > 65536 {
                return Err("crypto.random_bytes() n must be between 0 and 65536".to_string());
            }
            let mut buf = vec![0u8; n as usize];
            rand::thread_rng().fill_bytes(&mut buf);
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(buf))))
        },
    );

    // crypto.ed25519_generate_keypair() -> object
    register_typed_fn_0(
        &mut module,
        "ed25519_generate_keypair",
        "Generate an Ed25519 keypair, returning an object with hex-encoded public_key and secret_key",
        ConcreteType::Object,
        |ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Random)?;
            use rand::RngCore;
            let mut secret = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut secret);
            let signing_key = ed25519_dalek::SigningKey::from_bytes(&secret);
            let verifying_key = signing_key.verifying_key();
            Ok(TypedReturn::ObjectPairs(vec![
                (
                    "public_key".to_string(),
                    ConcreteReturn::String(hex::encode(verifying_key.to_bytes())),
                ),
                (
                    "secret_key".to_string(),
                    ConcreteReturn::String(hex::encode(signing_key.to_bytes())),
                ),
            ]))
        },
    );

    // crypto.ed25519_sign(message: string, secret_key: string) -> string
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "ed25519_sign",
        "Sign a message with an Ed25519 secret key, returning a hex-encoded signature",
        [("message", "string"), ("secret_key", "string")],
        ConcreteType::String,
        |message, secret_hex, _ctx| {
            use ed25519_dalek::Signer;
            let secret_bytes = hex::decode(secret_hex.as_str())
                .map_err(|e| format!("crypto.ed25519_sign() invalid secret_key hex: {}", e))?;
            let secret_arr: [u8; 32] = secret_bytes.as_slice().try_into().map_err(|_| {
                format!(
                    "crypto.ed25519_sign() secret_key must be 32 bytes (got {})",
                    secret_bytes.len()
                )
            })?;
            let signing_key = ed25519_dalek::SigningKey::from_bytes(&secret_arr);
            let signature = signing_key.sign(message.as_bytes());
            Ok(TypedReturn::Concrete(ConcreteReturn::String(hex::encode(signature.to_bytes()))))
        },
    );

    // crypto.ed25519_verify(message: string, signature: string, public_key: string) -> bool
    register_typed_fn_3::<_, Arc<String>, Arc<String>, Arc<String>>(
        &mut module,
        "ed25519_verify",
        "Verify an Ed25519 signature against a message and public key",
        [("message", "string"), ("signature", "string"), ("public_key", "string")],
        ConcreteType::Bool,
        |message, sig_hex, pub_hex, _ctx| {
            use ed25519_dalek::Verifier;
            let sig_bytes = hex::decode(sig_hex.as_str())
                .map_err(|e| format!("crypto.ed25519_verify() invalid signature hex: {}", e))?;
            let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
                format!(
                    "crypto.ed25519_verify() signature must be 64 bytes (got {})",
                    sig_bytes.len()
                )
            })?;
            let pub_bytes = hex::decode(pub_hex.as_str())
                .map_err(|e| format!("crypto.ed25519_verify() invalid public_key hex: {}", e))?;
            let pub_arr: [u8; 32] = pub_bytes.as_slice().try_into().map_err(|_| {
                format!(
                    "crypto.ed25519_verify() public_key must be 32 bytes (got {})",
                    pub_bytes.len()
                )
            })?;
            let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pub_arr)
                .map_err(|e| format!("crypto.ed25519_verify() invalid public key: {}", e))?;
            let signature = ed25519_dalek::Signature::from_bytes(&sig_arr);
            let valid = verifying_key.verify(message.as_bytes(), &signature).is_ok();
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(valid)))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crypto_module_creation() {
        let module = create_crypto_module();
        assert_eq!(module.name, "std::core::crypto");
        assert!(module.has_export("sha256"));
        assert!(module.has_export("hmac_sha256"));
        assert!(module.has_export("base64_encode"));
        assert!(module.has_export("base64_decode"));
        assert!(module.has_export("hex_encode"));
        assert!(module.has_export("hex_decode"));
        assert!(module.has_export("sha512"));
        assert!(module.has_export("sha1"));
        assert!(module.has_export("md5"));
        assert!(module.has_export("random_bytes"));
        assert!(module.has_export("ed25519_generate_keypair"));
        assert!(module.has_export("ed25519_sign"));
        assert!(module.has_export("ed25519_verify"));
    }

    #[test]
    fn test_crypto_schemas() {
        let module = create_crypto_module();

        let sha_schema = module.get_schema("sha256").unwrap();
        assert_eq!(sha_schema.params.len(), 1);
        assert_eq!(sha_schema.return_type.as_deref(), Some("string"));

        let hmac_schema = module.get_schema("hmac_sha256").unwrap();
        assert_eq!(hmac_schema.params.len(), 2);

        let b64d_schema = module.get_schema("base64_decode").unwrap();
        assert_eq!(b64d_schema.return_type.as_deref(), Some("Result<string>"));

        let rb_schema = module.get_schema("random_bytes").unwrap();
        assert_eq!(rb_schema.params[0].type_name, "int");

        let gen_schema = module.get_schema("ed25519_generate_keypair").unwrap();
        assert_eq!(gen_schema.params.len(), 0);

        let verify_schema = module.get_schema("ed25519_verify").unwrap();
        assert_eq!(verify_schema.params.len(), 3);
    }

    #[test]
    fn test_crypto_typed_registry() {
        let module = create_crypto_module();
        let typed = module.typed_exports();
        assert_eq!(typed.functions.len(), 13);

        let sha = typed.get("sha256").unwrap();
        assert_eq!(sha.arg_kinds.len(), 1);
        assert_eq!(sha.arg_kinds[0], shape_value::NativeKind::String);

        let rb = typed.get("random_bytes").unwrap();
        assert_eq!(rb.arg_kinds[0], shape_value::NativeKind::Int64);

        let keygen = typed.get("ed25519_generate_keypair").unwrap();
        assert!(keygen.arg_kinds.is_empty());
    }

    // Behavioural invocation tests removed — they used `module.invoke_export`
    // with `ValueWord` arrays, which is the deleted dynamic-dispatch entry
    // point. End-to-end behaviour is now covered through typed-slot dispatch
    // via the marshal layer; integration tests in `shape-test` will exercise
    // the full path once the strict-typed cascade reaches shape-vm.
}
