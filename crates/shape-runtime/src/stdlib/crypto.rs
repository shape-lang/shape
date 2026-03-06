//! Native `crypto` module for hashing and encoding utilities.
//!
//! Exports: crypto.sha256, crypto.hmac_sha256, crypto.base64_encode,
//!          crypto.base64_decode, crypto.hex_encode, crypto.hex_decode

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use std::sync::Arc;

/// Create the `crypto` module with hashing and encoding functions.
pub fn create_crypto_module() -> ModuleExports {
    let mut module = ModuleExports::new("crypto");
    module.description = "Cryptographic hashing and encoding utilities".to_string();

    // crypto.sha256(data: string) -> string
    module.add_function_with_schema(
        "sha256",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use sha2::{Digest, Sha256};

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.sha256() requires a string argument".to_string())?;

            let mut hasher = Sha256::new();
            hasher.update(data.as_bytes());
            let result = hasher.finalize();
            Ok(ValueWord::from_string(Arc::new(hex::encode(result))))
        },
        ModuleFunction {
            description: "Compute the SHA-256 hash of a string, returning a hex-encoded digest"
                .to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to hash".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // crypto.hmac_sha256(data: string, key: string) -> string
    module.add_function_with_schema(
        "hmac_sha256",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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
            Ok(ValueWord::from_string(Arc::new(hex::encode(
                result.into_bytes(),
            ))))
        },
        ModuleFunction {
            description: "Compute HMAC-SHA256 of data with the given key, returning hex digest"
                .to_string(),
            params: vec![
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
            return_type: Some("string".to_string()),
        },
    );

    // crypto.base64_encode(data: string) -> string
    module.add_function_with_schema(
        "base64_encode",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use base64::Engine;

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.base64_encode() requires a string argument".to_string())?;

            let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
            Ok(ValueWord::from_string(Arc::new(encoded)))
        },
        ModuleFunction {
            description: "Encode a string to Base64".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to encode".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // crypto.base64_decode(encoded: string) -> Result<string>
    module.add_function_with_schema(
        "base64_decode",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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

            Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(
                decoded,
            ))))
        },
        ModuleFunction {
            description: "Decode a Base64 string".to_string(),
            params: vec![ModuleParam {
                name: "encoded".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Base64-encoded string to decode".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<string>".to_string()),
        },
    );

    // crypto.hex_encode(data: string) -> string
    module.add_function_with_schema(
        "hex_encode",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.hex_encode() requires a string argument".to_string())?;

            Ok(ValueWord::from_string(Arc::new(hex::encode(
                data.as_bytes(),
            ))))
        },
        ModuleFunction {
            description: "Encode a string as hexadecimal".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Data to hex-encode".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // crypto.hex_decode(hex: string) -> Result<string>
    module.add_function_with_schema(
        "hex_decode",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let hex_str = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "crypto.hex_decode() requires a string argument".to_string())?;

            let bytes =
                hex::decode(hex_str).map_err(|e| format!("crypto.hex_decode() failed: {}", e))?;

            let decoded = String::from_utf8(bytes)
                .map_err(|e| format!("crypto.hex_decode() invalid UTF-8: {}", e))?;

            Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(
                decoded,
            ))))
        },
        ModuleFunction {
            description: "Decode a hexadecimal string".to_string(),
            params: vec![ModuleParam {
                name: "hex".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Hex-encoded string to decode".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<string>".to_string()),
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
        assert_eq!(module.name, "crypto");
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
        let sha_fn = module.get_export("sha256").unwrap();
        let result = sha_fn(
            &[ValueWord::from_string(Arc::new("hello".to_string()))],
            &ctx,
        )
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
        let sha_fn = module.get_export("sha256").unwrap();
        let result = sha_fn(&[ValueWord::from_string(Arc::new(String::new()))], &ctx).unwrap();
        assert_eq!(
            result.as_str(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[test]
    fn test_sha256_requires_string() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let sha_fn = module.get_export("sha256").unwrap();
        assert!(sha_fn(&[ValueWord::from_f64(42.0)], &ctx).is_err());
    }

    #[test]
    fn test_hmac_sha256() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let hmac_fn = module.get_export("hmac_sha256").unwrap();
        let result = hmac_fn(
            &[
                ValueWord::from_string(Arc::new("hello".to_string())),
                ValueWord::from_string(Arc::new("secret".to_string())),
            ],
            &ctx,
        )
        .unwrap();
        // HMAC-SHA256("hello", "secret") is a known value
        let digest = result.as_str().unwrap();
        assert_eq!(digest.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_hmac_sha256_requires_both_args() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let hmac_fn = module.get_export("hmac_sha256").unwrap();
        assert!(
            hmac_fn(
                &[ValueWord::from_string(Arc::new("data".to_string()))],
                &ctx
            )
            .is_err()
        );
        assert!(hmac_fn(&[], &ctx).is_err());
    }

    #[test]
    fn test_base64_roundtrip() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let encode_fn = module.get_export("base64_encode").unwrap();
        let decode_fn = module.get_export("base64_decode").unwrap();

        let original = "Hello, World!";
        let encoded = encode_fn(
            &[ValueWord::from_string(Arc::new(original.to_string()))],
            &ctx,
        )
        .unwrap();
        assert_eq!(encoded.as_str(), Some("SGVsbG8sIFdvcmxkIQ=="));

        let decoded = decode_fn(&[encoded], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some(original));
    }

    #[test]
    fn test_base64_decode_invalid() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let decode_fn = module.get_export("base64_decode").unwrap();
        let result = decode_fn(&[ValueWord::from_string(Arc::new("!!!".to_string()))], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_roundtrip() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let encode_fn = module.get_export("hex_encode").unwrap();
        let decode_fn = module.get_export("hex_decode").unwrap();

        let original = "hello";
        let encoded = encode_fn(
            &[ValueWord::from_string(Arc::new(original.to_string()))],
            &ctx,
        )
        .unwrap();
        assert_eq!(encoded.as_str(), Some("68656c6c6f"));

        let decoded = decode_fn(&[encoded], &ctx).unwrap();
        let inner = decoded.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some(original));
    }

    #[test]
    fn test_hex_decode_invalid() {
        let module = create_crypto_module();
        let ctx = test_ctx();
        let decode_fn = module.get_export("hex_decode").unwrap();
        let result = decode_fn(
            &[ValueWord::from_string(Arc::new("zzzz".to_string()))],
            &ctx,
        );
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
}
