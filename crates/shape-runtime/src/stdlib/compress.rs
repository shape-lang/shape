//! Native `compress` module for data compression and decompression.
//!
//! Exports: compress.gzip, compress.gunzip, compress.zstd, compress.unzstd,
//!          compress.deflate, compress.inflate

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use std::sync::Arc;
use super::byte_utils::{bytes_from_array, bytes_to_array};

/// Create the `compress` module with compression/decompression functions.
pub fn create_compress_module() -> ModuleExports {
    let mut module = ModuleExports::new("compress");
    module.description = "Data compression and decompression (gzip, zstd, deflate)".to_string();

    // compress.gzip(data: string) -> Array<int>
    module.add_function_with_schema(
        "gzip",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use flate2::Compression;
            use flate2::write::GzEncoder;
            use std::io::Write;

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "compress.gzip() requires a string argument".to_string())?;

            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(data.as_bytes())
                .map_err(|e| format!("compress.gzip() failed: {}", e))?;
            let compressed = encoder
                .finish()
                .map_err(|e| format!("compress.gzip() failed: {}", e))?;

            Ok(bytes_to_array(&compressed))
        },
        ModuleFunction {
            description: "Compress a string using gzip, returning a byte array".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String data to compress".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<int>".to_string()),
        },
    );

    // compress.gunzip(data: Array<int>) -> string
    module.add_function_with_schema(
        "gunzip",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use flate2::read::GzDecoder;
            use std::io::Read;

            let input = args
                .first()
                .ok_or_else(|| "compress.gunzip() requires an Array<int> argument".to_string())?;
            let bytes = bytes_from_array(input).map_err(|e| format!("compress.gunzip(): {}", e))?;

            let mut decoder = GzDecoder::new(&bytes[..]);
            let mut output = String::new();
            decoder
                .read_to_string(&mut output)
                .map_err(|e| format!("compress.gunzip() failed: {}", e))?;

            Ok(ValueWord::from_string(Arc::new(output)))
        },
        ModuleFunction {
            description: "Decompress a gzip byte array back to a string".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "Array<int>".to_string(),
                required: true,
                description: "Gzip-compressed byte array".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // compress.zstd(data: string, level?: int) -> Array<int>
    module.add_function_with_schema(
        "zstd",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "compress.zstd() requires a string argument".to_string())?;

            let level = args
                .get(1)
                .and_then(|a| a.as_i64().or_else(|| a.as_f64().map(|n| n as i64)))
                .unwrap_or(3) as i32;

            let compressed = zstd::encode_all(data.as_bytes(), level)
                .map_err(|e| format!("compress.zstd() failed: {}", e))?;

            Ok(bytes_to_array(&compressed))
        },
        ModuleFunction {
            description: "Compress a string using Zstandard, returning a byte array".to_string(),
            params: vec![
                ModuleParam {
                    name: "data".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "String data to compress".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "level".to_string(),
                    type_name: "int".to_string(),
                    required: false,
                    description: "Compression level (default: 3)".to_string(),
                    default_snippet: Some("3".to_string()),
                    ..Default::default()
                },
            ],
            return_type: Some("Array<int>".to_string()),
        },
    );

    // compress.unzstd(data: Array<int>) -> string
    module.add_function_with_schema(
        "unzstd",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let input = args
                .first()
                .ok_or_else(|| "compress.unzstd() requires an Array<int> argument".to_string())?;
            let bytes = bytes_from_array(input).map_err(|e| format!("compress.unzstd(): {}", e))?;

            let decompressed = zstd::decode_all(&bytes[..])
                .map_err(|e| format!("compress.unzstd() failed: {}", e))?;

            let output = String::from_utf8(decompressed)
                .map_err(|e| format!("compress.unzstd() invalid UTF-8: {}", e))?;

            Ok(ValueWord::from_string(Arc::new(output)))
        },
        ModuleFunction {
            description: "Decompress a Zstandard byte array back to a string".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "Array<int>".to_string(),
                required: true,
                description: "Zstd-compressed byte array".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // compress.deflate(data: string) -> Array<int>
    module.add_function_with_schema(
        "deflate",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use flate2::Compression;
            use flate2::write::DeflateEncoder;
            use std::io::Write;

            let data = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "compress.deflate() requires a string argument".to_string())?;

            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(data.as_bytes())
                .map_err(|e| format!("compress.deflate() failed: {}", e))?;
            let compressed = encoder
                .finish()
                .map_err(|e| format!("compress.deflate() failed: {}", e))?;

            Ok(bytes_to_array(&compressed))
        },
        ModuleFunction {
            description: "Compress a string using raw deflate, returning a byte array".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String data to compress".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<int>".to_string()),
        },
    );

    // compress.inflate(data: Array<int>) -> string
    module.add_function_with_schema(
        "inflate",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use flate2::read::DeflateDecoder;
            use std::io::Read;

            let input = args
                .first()
                .ok_or_else(|| "compress.inflate() requires an Array<int> argument".to_string())?;
            let bytes =
                bytes_from_array(input).map_err(|e| format!("compress.inflate(): {}", e))?;

            let mut decoder = DeflateDecoder::new(&bytes[..]);
            let mut output = String::new();
            decoder
                .read_to_string(&mut output)
                .map_err(|e| format!("compress.inflate() failed: {}", e))?;

            Ok(ValueWord::from_string(Arc::new(output)))
        },
        ModuleFunction {
            description: "Decompress a raw deflate byte array back to a string".to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "Array<int>".to_string(),
                required: true,
                description: "Deflate-compressed byte array".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
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
    fn test_compress_module_creation() {
        let module = create_compress_module();
        assert_eq!(module.name, "compress");
        assert!(module.has_export("gzip"));
        assert!(module.has_export("gunzip"));
        assert!(module.has_export("zstd"));
        assert!(module.has_export("unzstd"));
        assert!(module.has_export("deflate"));
        assert!(module.has_export("inflate"));
    }

    #[test]
    fn test_gzip_roundtrip() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let gzip_fn = module.get_export("gzip").unwrap();
        let gunzip_fn = module.get_export("gunzip").unwrap();

        let input = ValueWord::from_string(Arc::new("hello world".to_string()));
        let compressed = gzip_fn(&[input], &ctx).unwrap();

        // Compressed should be an array
        assert!(compressed.as_any_array().is_some());

        let decompressed = gunzip_fn(&[compressed], &ctx).unwrap();
        assert_eq!(decompressed.as_str(), Some("hello world"));
    }

    #[test]
    fn test_zstd_roundtrip() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let zstd_fn = module.get_export("zstd").unwrap();
        let unzstd_fn = module.get_export("unzstd").unwrap();

        let input = ValueWord::from_string(Arc::new("hello zstd compression".to_string()));
        let compressed = zstd_fn(&[input], &ctx).unwrap();

        assert!(compressed.as_any_array().is_some());

        let decompressed = unzstd_fn(&[compressed], &ctx).unwrap();
        assert_eq!(decompressed.as_str(), Some("hello zstd compression"));
    }

    #[test]
    fn test_zstd_with_level() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let zstd_fn = module.get_export("zstd").unwrap();
        let unzstd_fn = module.get_export("unzstd").unwrap();

        let input = ValueWord::from_string(Arc::new("level test".to_string()));
        let level = ValueWord::from_i64(1);
        let compressed = zstd_fn(&[input, level], &ctx).unwrap();

        let decompressed = unzstd_fn(&[compressed], &ctx).unwrap();
        assert_eq!(decompressed.as_str(), Some("level test"));
    }

    #[test]
    fn test_deflate_roundtrip() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let deflate_fn = module.get_export("deflate").unwrap();
        let inflate_fn = module.get_export("inflate").unwrap();

        let input = ValueWord::from_string(Arc::new("deflate test data".to_string()));
        let compressed = deflate_fn(&[input], &ctx).unwrap();

        assert!(compressed.as_any_array().is_some());

        let decompressed = inflate_fn(&[compressed], &ctx).unwrap();
        assert_eq!(decompressed.as_str(), Some("deflate test data"));
    }

    #[test]
    fn test_gzip_requires_string() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let gzip_fn = module.get_export("gzip").unwrap();

        let result = gzip_fn(&[ValueWord::from_i64(42)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_gunzip_invalid_data() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let gunzip_fn = module.get_export("gunzip").unwrap();

        let bad_data = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ]));
        let result = gunzip_fn(&[bad_data], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_string_roundtrip() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let gzip_fn = module.get_export("gzip").unwrap();
        let gunzip_fn = module.get_export("gunzip").unwrap();

        let input = ValueWord::from_string(Arc::new(String::new()));
        let compressed = gzip_fn(&[input], &ctx).unwrap();
        let decompressed = gunzip_fn(&[compressed], &ctx).unwrap();
        assert_eq!(decompressed.as_str(), Some(""));
    }

    #[test]
    fn test_large_data_roundtrip() {
        let module = create_compress_module();
        let ctx = test_ctx();
        let gzip_fn = module.get_export("gzip").unwrap();
        let gunzip_fn = module.get_export("gunzip").unwrap();

        let large = "a".repeat(10_000);
        let input = ValueWord::from_string(Arc::new(large.clone()));
        let compressed = gzip_fn(&[input], &ctx).unwrap();

        // Compressed should be smaller than original
        let arr = compressed.as_any_array().unwrap().to_generic();
        assert!(arr.len() < 10_000);

        let decompressed = gunzip_fn(&[compressed], &ctx).unwrap();
        assert_eq!(decompressed.as_str(), Some(large.as_str()));
    }

    #[test]
    fn test_schemas() {
        let module = create_compress_module();

        let gzip_schema = module.get_schema("gzip").unwrap();
        assert_eq!(gzip_schema.params.len(), 1);
        assert!(gzip_schema.params[0].required);
        assert_eq!(gzip_schema.return_type.as_deref(), Some("Array<int>"));

        let zstd_schema = module.get_schema("zstd").unwrap();
        assert_eq!(zstd_schema.params.len(), 2);
        assert!(zstd_schema.params[0].required);
        assert!(!zstd_schema.params[1].required);

        let gunzip_schema = module.get_schema("gunzip").unwrap();
        assert_eq!(gunzip_schema.return_type.as_deref(), Some("string"));
    }
}
