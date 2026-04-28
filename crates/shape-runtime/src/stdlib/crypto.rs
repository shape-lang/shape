//! Native `crypto` module for hashing, encoding, and signing utilities.
//!
//! Exports: crypto.sha256, crypto.sha512, crypto.sha1, crypto.md5,
//!          crypto.hmac_sha256, crypto.base64_encode, crypto.base64_decode,
//!          crypto.hex_encode, crypto.hex_decode, crypto.random_bytes,
//!          crypto.ed25519_generate_keypair, crypto.ed25519_sign, crypto.ed25519_verify

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

/// Create the `crypto` module with hashing and encoding functions.
pub fn create_crypto_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::crypto");
    module.description = "Cryptographic hashing and encoding utilities".to_string();

    // crypto.sha256(data: string) -> string
    register_typed_function(
        &mut module,
        "sha256",
        "Compute the SHA-256 hash of a string, returning a hex-encoded digest",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Data to hash".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, _ctx| {
            use sha2::{Digest, Sha256};

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.sha256() requires a string argument".to_string())?;

            let mut hasher = Sha256::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::String(hex::encode(result)))
        },
    );

    // crypto.hmac_sha256(data: string, key: string) -> string
    register_typed_function(
        &mut module,
        "hmac_sha256",
        "Compute HMAC-SHA256 of data with the given key, returning hex digest",
        vec![
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to authenticate".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "key".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "HMAC key".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |args, _ctx| {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;

            let data = args.first().and_then(|a| a.as_str()).ok_or_else(|| {
                "crypto.hmac_sha256() requires a data string argument".to_string()
            })?;

            let key = args
                .get(1)
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.hmac_sha256() requires a key string argument".to_string())?;

            type HmacSha256 = Hmac<Sha256>;
            let mut mac = HmacSha256::new_from_slice(key.as_bytes())
                .map_err(|e| format!("crypto.hmac_sha256() key error: {}", e))?;
            mac.update(data.as_bytes());
            let result = mac.finalize();
            Ok(TypedReturn::String(hex::encode(result.into_bytes())))
        },
    );

    // crypto.base64_encode(data: string) -> string
    register_typed_function(
        &mut module,
        "base64_encode",
        "Encode a string to Base64",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Data to encode".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, _ctx| {
            use base64::Engine;

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.base64_encode() requires a string argument".to_string())?;

            let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
            Ok(TypedReturn::String(encoded))
        },
    );

    // crypto.base64_decode(encoded: string) -> Result<string>
    register_typed_function(
        &mut module,
        "base64_decode",
        "Decode a Base64 string",
        vec![ModuleParam {
            name: "encoded".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Base64-encoded string to decode".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |args, _ctx| {
            use base64::Engine;

            let encoded = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.base64_decode() requires a string argument".to_string())?;

            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| format!("crypto.base64_decode() failed: {}", e))?;

            let decoded = String::from_utf8(bytes)
                .map_err(|e| format!("crypto.base64_decode() invalid UTF-8: {}", e))?;

            Ok(TypedReturn::Ok(Box::new(TypedReturn::String(decoded))))
        },
    );

    // crypto.hex_encode(data: string) -> string
    register_typed_function(
        &mut module,
        "hex_encode",
        "Encode a string as hexadecimal",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Data to hex-encode".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, _ctx| {
            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.hex_encode() requires a string argument".to_string())?;
            Ok(TypedReturn::String(hex::encode(data.as_bytes())))
        },
    );

    // crypto.hex_decode(hex: string) -> Result<string>
    register_typed_function(
        &mut module,
        "hex_decode",
        "Decode a hexadecimal string",
        vec![ModuleParam {
            name: "hex".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Hex-encoded string to decode".to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |args, _ctx| {
            let hex_str = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.hex_decode() requires a string argument".to_string())?;

            let bytes =
                hex::decode(hex_str).map_err(|e| format!("crypto.hex_decode() failed: {}", e))?;

            let decoded = String::from_utf8(bytes)
                .map_err(|e| format!("crypto.hex_decode() invalid UTF-8: {}", e))?;

            Ok(TypedReturn::Ok(Box::new(TypedReturn::String(decoded))))
        },
    );

    // crypto.sha512(data: string) -> string
    register_typed_function(
        &mut module,
        "sha512",
        "Compute the SHA-512 hash of a string, returning a hex-encoded digest",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Data to hash".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, _ctx| {
            use sha2::{Digest, Sha512};

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.sha512() requires a string argument".to_string())?;

            let mut hasher = Sha512::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::String(hex::encode(result)))
        },
    );

    // crypto.sha1(data: string) -> string
    register_typed_function(
        &mut module,
        "sha1",
        "Compute the SHA-1 hash of a string, returning a hex-encoded digest (legacy)",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Data to hash".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, _ctx| {
            use sha1::Digest;

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.sha1() requires a string argument".to_string())?;

            let mut hasher = sha1::Sha1::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::String(hex::encode(result)))
        },
    );

    // crypto.md5(data: string) -> string
    register_typed_function(
        &mut module,
        "md5",
        "Compute the MD5 hash of a string, returning a hex-encoded digest (legacy)",
        vec![ModuleParam {
            name: "data".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Data to hash".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, _ctx| {
            use md5::Digest;

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.md5() requires a string argument".to_string())?;

            let mut hasher = md5::Md5::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(TypedReturn::String(hex::encode(result)))
        },
    );

    // crypto.random_bytes(n: int) -> string
    register_typed_function(
        &mut module,
        "random_bytes",
        "Generate n random bytes, returned as a hex-encoded string",
        vec![ModuleParam {
            name: "n".to_string(),
            type_name: "int".to_string(),
            required: true,
            description: "Number of random bytes to generate (0..65536)".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Random)?;
            use rand::RngCore;

            let n = args
                .first()
                .and_then(|a| a.as_i64())
                .ok_or_else(|| "crypto.random_bytes() requires an int argument".to_string())?;

            if n < 0 || n > 65536 {
                return Err("crypto.random_bytes() n must be between 0 and 65536".to_string());
            }

            let mut buf = vec![0u8; n as usize];
            rand::thread_rng().fill_bytes(&mut buf);
            Ok(TypedReturn::String(hex::encode(buf)))
        },
    );

    // crypto.ed25519_generate_keypair() -> object
    register_typed_function(
        &mut module,
        "ed25519_generate_keypair",
        "Generate an Ed25519 keypair, returning an object with hex-encoded public_key and secret_key",
        vec![],
        ConcreteType::Object,
        |_args, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::Random)?;
            use rand::RngCore;

            let mut secret = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut secret);
            let signing_key = ed25519_dalek::SigningKey::from_bytes(&secret);
            let verifying_key = signing_key.verifying_key();

            Ok(TypedReturn::HashMapStringString(vec![
                (
                    "public_key".to_string(),
                    hex::encode(verifying_key.to_bytes()),
                ),
                (
                    "secret_key".to_string(),
                    hex::encode(signing_key.to_bytes()),
                ),
            ]))
        },
    );

    // crypto.ed25519_sign(message: string, secret_key: string) -> string
    register_typed_function(
        &mut module,
        "ed25519_sign",
        "Sign a message with an Ed25519 secret key, returning a hex-encoded signature",
        vec![
            ModuleParam {
                name: "message".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Message to sign".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "secret_key".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Hex-encoded 32-byte Ed25519 secret key".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |args, _ctx| {
            use ed25519_dalek::Signer;

            let message = args.first().and_then(|a| a.as_str()).ok_or_else(|| {
                "crypto.ed25519_sign() requires a message string argument".to_string()
            })?;

            let secret_hex = args.get(1).and_then(|a| a.as_str()).ok_or_else(|| {
                "crypto.ed25519_sign() requires a secret_key hex string argument".to_string()
            })?;

            let secret_bytes = hex::decode(secret_hex)
                .map_err(|e| format!("crypto.ed25519_sign() invalid secret_key hex: {}", e))?;

            let secret_arr: [u8; 32] = secret_bytes.as_slice().try_into().map_err(|_| {
                format!(
                    "crypto.ed25519_sign() secret_key must be 32 bytes (got {})",
                    secret_bytes.len()
                )
            })?;

            let signing_key = ed25519_dalek::SigningKey::from_bytes(&secret_arr);
            let signature = signing_key.sign(message.as_bytes());
            Ok(TypedReturn::String(hex::encode(signature.to_bytes())))
        },
    );

    // crypto.ed25519_verify(message: string, signature: string, public_key: string) -> bool
    register_typed_function(
        &mut module,
        "ed25519_verify",
        "Verify an Ed25519 signature against a message and public key",
        vec![
            ModuleParam {
                name: "message".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Message that was signed".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "signature".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Hex-encoded 64-byte Ed25519 signature".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "public_key".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Hex-encoded 32-byte Ed25519 public key".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::Bool,
        |args, _ctx| {
            use ed25519_dalek::Verifier;

            let message = args.first().and_then(|a| a.as_str()).ok_or_else(|| {
                "crypto.ed25519_verify() requires a message string argument".to_string()
            })?;

            let sig_hex = args.get(1).and_then(|a| a.as_str()).ok_or_else(|| {
                "crypto.ed25519_verify() requires a signature hex string argument".to_string()
            })?;

            let pub_hex = args.get(2).and_then(|a| a.as_str()).ok_or_else(|| {
                "crypto.ed25519_verify() requires a public_key hex string argument".to_string()
            })?;

            let sig_bytes = hex::decode(sig_hex)
                .map_err(|e| format!("crypto.ed25519_verify() invalid signature hex: {}", e))?;

            let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
                format!(
                    "crypto.ed25519_verify() signature must be 64 bytes (got {})",
                    sig_bytes.len()
                )
            })?;

            let pub_bytes = hex::decode(pub_hex)
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
            Ok(TypedReturn::Bool(valid))
        },
    );

    module
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
    fn test_crypto_module_creation() {
        let module = create_crypto_module();
        assert_eq!(module.name, "std::core::crypto");
        assert!(module.has_export("sha256"));
        assert!(module.has_export("hmac_sha256"));
        assert!(module.has_export("base64_encode"));
        assert!(module.has_export("base64_decode"));
        assert!(module.has_export("hex_encode"));
        assert!(module.has_export("hex_decode"));
    }

    #[test]
    fn test_sha256_known_digest() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("sha256", 
            &[ValueWord::from_string(Arc::new("hello".to_string()))],
            &ctx,
        ).unwrap()
        .unwrap();
        // Known SHA-256 digest for "hello"
        assert_eq!(
            result.as_str(),
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
        );
    }

    #[test]
    fn test_sha256_empty_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("sha256", &[ValueWord::from_string(Arc::new(String::new()))], &ctx).unwrap().unwrap();
        assert_eq!(
            result.as_str(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[test]
    fn test_sha256_requires_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(module.invoke_export("sha256", &[ValueWord::from_f64(42.0)], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_hmac_sha256() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("hmac_sha256", 
            &[
                ValueWord::from_string(Arc::new("hello".to_string())),
                ValueWord::from_string(Arc::new("secret".to_string())),
            ],
            &ctx,
        ).unwrap()
        .unwrap();
        // HMAC-SHA256("hello", "secret") is a known value
        let digest = result.as_str().unwrap();
        assert_eq!(digest.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_hmac_sha256_requires_both_args() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(
            module.invoke_export("hmac_sha256", 
                &[ValueWord::from_string(Arc::new("data".to_string()))],
                &ctx
            ).unwrap()
            .is_err()
        );
        assert!(module.invoke_export("hmac_sha256", &[], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_base64_roundtrip() {
        let module = create_crypto_module();
        let ctx = test_ctx();

        let original = "Hello, World!";
        let encoded = module.invoke_export("base64_encode", 
            &[ValueWord::from_string(Arc::new(original.to_string()))],
            &ctx,
        ).unwrap()
        .unwrap();
        assert_eq!(encoded.as_str(), Some("SGVsbG8sIFdvcmxkIQ=="));

        let decoded = module.invoke_export("base64_decode", &[encoded], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some(original));
    }

    #[test]
    fn test_base64_decode_invalid() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("base64_decode", &[ValueWord::from_string(Arc::new("!!!".to_string()))], &ctx).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_roundtrip() {
        let module = create_crypto_module();
        let ctx = test_ctx();

        let original = "hello";
        let encoded = module.invoke_export("hex_encode", 
            &[ValueWord::from_string(Arc::new(original.to_string()))],
            &ctx,
        ).unwrap()
        .unwrap();
        assert_eq!(encoded.as_str(), Some("68656c6c6f"));

        let decoded = module.invoke_export("hex_decode", &[encoded], &ctx).unwrap().unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some(original));
    }

    #[test]
    fn test_hex_decode_invalid() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("hex_decode", 
            &[ValueWord::from_string(Arc::new("zzzz".to_string()))],
            &ctx,
        ).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_crypto_schemas() {
        let module = create_crypto_module();

        let sha_schema = module.get_schema("sha256").unwrap();
        assert_eq!(sha_schema.params.len(), 1);
        assert_eq!(sha_schema.return_type.as_deref(), Some("string"));

        let hmac_schema = module.get_schema("hmac_sha256").unwrap();
        assert_eq!(hmac_schema.params.len(), 2);
        assert!(hmac_schema.params[0].required);
        assert!(hmac_schema.params[1].required);

        let b64d_schema = module.get_schema("base64_decode").unwrap();
        assert_eq!(b64d_schema.return_type.as_deref(), Some("Result<string>"));
    }

    #[test]
    fn test_crypto_module_has_new_exports() {
        let module = create_crypto_module();
        assert!(module.has_export("sha512"));
        assert!(module.has_export("sha1"));
        assert!(module.has_export("md5"));
        assert!(module.has_export("random_bytes"));
        assert!(module.has_export("ed25519_generate_keypair"));
        assert!(module.has_export("ed25519_sign"));
        assert!(module.has_export("ed25519_verify"));
    }

    #[test]
    fn test_sha512_known_digest() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("sha512", 
            &[ValueWord::from_string(Arc::new("hello".to_string()))],
            &ctx,
        ).unwrap()
        .unwrap();
        // Known SHA-512 digest for "hello"
        assert_eq!(
            result.as_str(),
            Some(
                "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043"
            )
        );
    }

    #[test]
    fn test_sha512_empty_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("sha512", &[ValueWord::from_string(Arc::new(String::new()))], &ctx).unwrap().unwrap();
        // SHA-512 of empty string
        assert_eq!(
            result.as_str(),
            Some(
                "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
            )
        );
    }

    #[test]
    fn test_sha512_requires_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(module.invoke_export("sha512", &[ValueWord::from_f64(42.0)], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_sha1_known_digest() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("sha1", 
            &[ValueWord::from_string(Arc::new("hello".to_string()))],
            &ctx,
        ).unwrap()
        .unwrap();
        // Known SHA-1 digest for "hello"
        assert_eq!(
            result.as_str(),
            Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")
        );
    }

    #[test]
    fn test_sha1_empty_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("sha1", &[ValueWord::from_string(Arc::new(String::new()))], &ctx).unwrap().unwrap();
        assert_eq!(
            result.as_str(),
            Some("da39a3ee5e6b4b0d3255bfef95601890afd80709")
        );
    }

    #[test]
    fn test_sha1_requires_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(module.invoke_export("sha1", &[ValueWord::from_f64(42.0)], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_md5_known_digest() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("md5", 
            &[ValueWord::from_string(Arc::new("hello".to_string()))],
            &ctx,
        ).unwrap()
        .unwrap();
        // Known MD5 digest for "hello"
        assert_eq!(result.as_str(), Some("5d41402abc4b2a76b9719d911017c592"));
    }

    #[test]
    fn test_md5_empty_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("md5", &[ValueWord::from_string(Arc::new(String::new()))], &ctx).unwrap().unwrap();
        assert_eq!(result.as_str(), Some("d41d8cd98f00b204e9800998ecf8427e"));
    }

    #[test]
    fn test_md5_requires_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(module.invoke_export("md5", &[ValueWord::from_f64(42.0)], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_random_bytes_length() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("random_bytes", &[ValueWord::from_i64(16)], &ctx).unwrap().unwrap();
        let hex_str = result.as_str().unwrap();
        // 16 bytes = 32 hex chars
        assert_eq!(hex_str.len(), 32);
    }

    #[test]
    fn test_random_bytes_zero() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("random_bytes", &[ValueWord::from_i64(0)], &ctx).unwrap().unwrap();
        assert_eq!(result.as_str(), Some(""));
    }

    #[test]
    fn test_random_bytes_negative_rejected() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(module.invoke_export("random_bytes", &[ValueWord::from_i64(-1)], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_random_bytes_too_large_rejected() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(module.invoke_export("random_bytes", &[ValueWord::from_i64(65537)], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_random_bytes_requires_int() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        assert!(module.invoke_export("random_bytes", &[ValueWord::from_string(Arc::new("10".to_string()))], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_ed25519_generate_keypair() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let result = module.invoke_export("ed25519_generate_keypair", &[], &ctx).unwrap().unwrap();

        // Result should be a HashMap with public_key and secret_key
        let hm = result.as_hashmap_data().expect("should be a HashMap");
        let pub_key = hm.shape_get("public_key").expect("should have public_key");
        let sec_key = hm.shape_get("secret_key").expect("should have secret_key");

        // 32 bytes = 64 hex chars
        assert_eq!(pub_key.as_str().unwrap().len(), 64);
        assert_eq!(sec_key.as_str().unwrap().len(), 64);
    }

    #[test]
    fn test_ed25519_sign_and_verify_roundtrip() {
        let module = create_crypto_module();
        let ctx = test_ctx();

        // Generate a keypair
        let keypair = module.invoke_export("ed25519_generate_keypair", &[], &ctx).unwrap().unwrap();
        let hm = keypair.as_hashmap_data().unwrap();

        let pub_key = hm.shape_get("public_key").unwrap().clone();
        let sec_key = hm.shape_get("secret_key").unwrap().clone();

        let message = ValueWord::from_string(Arc::new("test message".to_string()));

        // Sign
        let signature = module.invoke_export("ed25519_sign", &[message.clone(), sec_key], &ctx).unwrap().unwrap();
        // 64 bytes = 128 hex chars
        assert_eq!(signature.as_str().unwrap().len(), 128);

        // Verify — should succeed
        let valid = module.invoke_export("ed25519_verify", &[message, signature, pub_key], &ctx).unwrap().unwrap();
        assert_eq!(valid.as_bool(), Some(true));
    }

    #[test]
    fn test_ed25519_verify_wrong_message() {
        let module = create_crypto_module();
        let ctx = test_ctx();

        let keypair = module.invoke_export("ed25519_generate_keypair", &[], &ctx).unwrap().unwrap();
        let hm = keypair.as_hashmap_data().unwrap();

        let pub_key = hm.shape_get("public_key").unwrap().clone();
        let sec_key = hm.shape_get("secret_key").unwrap().clone();

        let message = ValueWord::from_string(Arc::new("correct message".to_string()));
        let wrong_message = ValueWord::from_string(Arc::new("wrong message".to_string()));

        let signature = module.invoke_export("ed25519_sign", &[message, sec_key], &ctx).unwrap().unwrap();

        let valid = module.invoke_export("ed25519_verify", &[wrong_message, signature, pub_key], &ctx).unwrap().unwrap();
        assert_eq!(valid.as_bool(), Some(false));
    }

    #[test]
    fn test_ed25519_sign_invalid_secret_key() {
        let module = create_crypto_module();
        let ctx = test_ctx();

        // Too short
        let result = module.invoke_export("ed25519_sign", 
            &[
                ValueWord::from_string(Arc::new("msg".to_string())),
                ValueWord::from_string(Arc::new("abcd".to_string())),
            ],
            &ctx,
        ).unwrap();
        assert!(result.is_err());

        // Invalid hex
        let result = module.invoke_export("ed25519_sign", 
            &[
                ValueWord::from_string(Arc::new("msg".to_string())),
                ValueWord::from_string(Arc::new("zzzz".to_string())),
            ],
            &ctx,
        ).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_ed25519_verify_invalid_inputs() {
        let module = create_crypto_module();
        let ctx = test_ctx();

        // Missing arguments
        assert!(module.invoke_export("ed25519_verify", &[ValueWord::from_string(Arc::new("msg".to_string()))], &ctx).unwrap().is_err());

        // Invalid hex in signature
        assert!(
            module.invoke_export("ed25519_verify", 
                &[
                    ValueWord::from_string(Arc::new("msg".to_string())),
                    ValueWord::from_string(Arc::new("not_hex".to_string())),
                    ValueWord::from_string(Arc::new("ab".repeat(32))),
                ],
                &ctx
            ).unwrap()
            .is_err()
        );
    }

    #[test]
    fn test_new_function_schemas() {
        let module = create_crypto_module();

        let sha512_schema = module.get_schema("sha512").unwrap();
        assert_eq!(sha512_schema.params.len(), 1);
        assert_eq!(sha512_schema.return_type.as_deref(), Some("string"));

        let sha1_schema = module.get_schema("sha1").unwrap();
        assert_eq!(sha1_schema.params.len(), 1);
        assert_eq!(sha1_schema.return_type.as_deref(), Some("string"));

        let md5_schema = module.get_schema("md5").unwrap();
        assert_eq!(md5_schema.params.len(), 1);
        assert_eq!(md5_schema.return_type.as_deref(), Some("string"));

        let rb_schema = module.get_schema("random_bytes").unwrap();
        assert_eq!(rb_schema.params.len(), 1);
        assert_eq!(rb_schema.params[0].type_name, "int");
        assert_eq!(rb_schema.return_type.as_deref(), Some("string"));

        let gen_schema = module.get_schema("ed25519_generate_keypair").unwrap();
        assert_eq!(gen_schema.params.len(), 0);
        assert_eq!(gen_schema.return_type.as_deref(), Some("object"));

        let sign_schema = module.get_schema("ed25519_sign").unwrap();
        assert_eq!(sign_schema.params.len(), 2);
        assert_eq!(sign_schema.return_type.as_deref(), Some("string"));

        let verify_schema = module.get_schema("ed25519_verify").unwrap();
        assert_eq!(verify_schema.params.len(), 3);
        assert_eq!(verify_schema.return_type.as_deref(), Some("bool"));
    }
}
