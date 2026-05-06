//! Native `compress` module for data compression and decompression.
//!
//! Exports: compress.gzip, compress.gunzip, compress.zstd, compress.unzstd,
//!          compress.deflate, compress.inflate
//!
//! Phase 4b: all 6 exports migrated to `TypedModuleExports`.
//! Phase 2c: ported to the typed marshal layer (option β: owned `Vec<i64>`
//! / `Vec<u8>` / `Arc<String>` via FromSlot, `TypedReturn::Concrete(
//! ConcreteReturn::Bytes | ConcreteReturn::String)` outputs). The 3 compress
//! functions return `ConcreteReturn::Bytes` (semantically `Array<int>` of
//! u8 widened to i64); the 3 decompress functions return
//! `ConcreteReturn::String`.

use super::byte_utils::bytes_from_i64_slice;
use crate::marshal::{register_typed_fn_1, register_typed_fn_2};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Create the `compress` module with compression/decompression functions.
pub fn create_compress_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::compress");
    module.description = "Data compression and decompression (gzip, zstd, deflate)".to_string();

    // compress.gzip(data: string) -> Array<int>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "gzip",
        "Compress a string using gzip, returning a byte array",
        "data",
        "string",
        ConcreteType::Bytes,
        |data, _ctx| {
            use flate2::Compression;
            use flate2::write::GzEncoder;
            use std::io::Write;

            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(data.as_bytes())
                .map_err(|e| format!("compress.gzip() failed: {}", e))?;
            let compressed = encoder
                .finish()
                .map_err(|e| format!("compress.gzip() failed: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::Bytes(compressed)))
        },
    );

    // compress.gunzip(data: Array<int>) -> string
    register_typed_fn_1::<_, Vec<i64>>(
        &mut module,
        "gunzip",
        "Decompress a gzip byte array back to a string",
        "data",
        "Array<int>",
        ConcreteType::String,
        |data, _ctx| {
            use flate2::read::GzDecoder;
            use std::io::Read;

            let bytes = bytes_from_i64_slice(&data)
                .map_err(|e| format!("compress.gunzip(): {}", e))?;

            let mut decoder = GzDecoder::new(&bytes[..]);
            let mut output = String::new();
            decoder
                .read_to_string(&mut output)
                .map_err(|e| format!("compress.gunzip() failed: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::String(output)))
        },
    );

    // compress.zstd(data: string, level?: int) -> Array<int>
    register_typed_fn_2::<_, Arc<String>, i64>(
        &mut module,
        "zstd",
        "Compress a string using Zstandard, returning a byte array",
        [("data", "string"), ("level", "int")],
        ConcreteType::Bytes,
        |data, level, _ctx| {
            let compressed = zstd::encode_all(data.as_bytes(), level as i32)
                .map_err(|e| format!("compress.zstd() failed: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::Bytes(compressed)))
        },
    );

    // compress.unzstd(data: Array<int>) -> string
    register_typed_fn_1::<_, Vec<i64>>(
        &mut module,
        "unzstd",
        "Decompress a Zstandard byte array back to a string",
        "data",
        "Array<int>",
        ConcreteType::String,
        |data, _ctx| {
            let bytes = bytes_from_i64_slice(&data)
                .map_err(|e| format!("compress.unzstd(): {}", e))?;

            let decompressed = zstd::decode_all(&bytes[..])
                .map_err(|e| format!("compress.unzstd() failed: {}", e))?;

            let output = String::from_utf8(decompressed)
                .map_err(|e| format!("compress.unzstd() invalid UTF-8: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::String(output)))
        },
    );

    // compress.deflate(data: string) -> Array<int>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "deflate",
        "Compress a string using raw deflate, returning a byte array",
        "data",
        "string",
        ConcreteType::Bytes,
        |data, _ctx| {
            use flate2::Compression;
            use flate2::write::DeflateEncoder;
            use std::io::Write;

            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(data.as_bytes())
                .map_err(|e| format!("compress.deflate() failed: {}", e))?;
            let compressed = encoder
                .finish()
                .map_err(|e| format!("compress.deflate() failed: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::Bytes(compressed)))
        },
    );

    // compress.inflate(data: Array<int>) -> string
    register_typed_fn_1::<_, Vec<i64>>(
        &mut module,
        "inflate",
        "Decompress a raw deflate byte array back to a string",
        "data",
        "Array<int>",
        ConcreteType::String,
        |data, _ctx| {
            use flate2::read::DeflateDecoder;
            use std::io::Read;

            let bytes = bytes_from_i64_slice(&data)
                .map_err(|e| format!("compress.inflate(): {}", e))?;

            let mut decoder = DeflateDecoder::new(&bytes[..]);
            let mut output = String::new();
            decoder
                .read_to_string(&mut output)
                .map_err(|e| format!("compress.inflate() failed: {}", e))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::String(output)))
        },
    );

    module
}
