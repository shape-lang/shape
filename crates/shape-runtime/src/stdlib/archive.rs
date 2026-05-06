//! Native `archive` module for creating and extracting zip/tar archives.
//!
//! Exports (post-cluster-#3, partial): archive.zip_extract, archive.tar_extract.
//!
//! Phase 2c partial migration: the two extract functions take `Array<int>`
//! input (cluster #3 Array<T> marshal scope) and are wired here on the
//! typed marshal layer. The two create functions (`archive.zip_create` /
//! `archive.tar_create`) take `Array<{name: string, data: string}>` input
//! which requires the typed-object marshal extension (cluster #4 in
//! `docs/defections.md` 2026-05-06). They are NOT registered until that
//! cluster lands; users calling them today get a "no such method" error
//! rather than a half-typed shell. The create logic itself is uncomplicated
//! (zip/tar writer over a Vec<u8> sink) and rebuilds in a few lines once
//! the typed-object FromSlot impl exists.

use super::byte_utils::bytes_from_i64_slice;
use crate::marshal::register_typed_fn_1;
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};

/// Create the `archive` module with extraction functions registered.
/// Create functions deferred to cluster #4 (see module doc comment).
pub fn create_archive_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::archive");
    module.description = "Archive extraction (zip, tar)".to_string();

    // archive.zip_extract(data: Array<int>) -> Array<{name: string, data: string}>
    register_typed_fn_1::<_, Vec<i64>>(
        &mut module,
        "zip_extract",
        "Extract a zip archive from a byte array into an array of entries",
        "data",
        "Array<int>",
        ConcreteType::ArrayObject("Array<{name: string, data: string}>".to_string()),
        |data, _ctx| {
            use std::io::{Cursor, Read};

            let bytes = bytes_from_i64_slice(&data)
                .map_err(|e| format!("archive.zip_extract(): {}", e))?;

            let cursor = Cursor::new(bytes);
            let mut archive = zip::ZipArchive::new(cursor)
                .map_err(|e| format!("archive.zip_extract() invalid zip: {}", e))?;

            let mut entries: Vec<Vec<(String, ConcreteReturn)>> = Vec::new();
            for i in 0..archive.len() {
                let mut file = archive.by_index(i).map_err(|e| {
                    format!("archive.zip_extract() failed to read entry {}: {}", i, e)
                })?;

                if file.is_dir() {
                    continue;
                }

                let name = file.name().to_string();
                let mut contents = String::new();
                file.read_to_string(&mut contents).map_err(|e| {
                    format!("archive.zip_extract() failed to read '{}': {}", name, e)
                })?;

                entries.push(vec![
                    ("name".to_string(), ConcreteReturn::String(name)),
                    ("data".to_string(), ConcreteReturn::String(contents)),
                ]);
            }

            Ok(TypedReturn::ArrayObjectPairs(entries))
        },
    );

    // archive.tar_extract(data: Array<int>) -> Array<{name: string, data: string}>
    register_typed_fn_1::<_, Vec<i64>>(
        &mut module,
        "tar_extract",
        "Extract a tar archive from a byte array into an array of entries",
        "data",
        "Array<int>",
        ConcreteType::ArrayObject("Array<{name: string, data: string}>".to_string()),
        |data, _ctx| {
            use std::io::{Cursor, Read};

            let bytes = bytes_from_i64_slice(&data)
                .map_err(|e| format!("archive.tar_extract(): {}", e))?;

            let cursor = Cursor::new(bytes);
            let mut archive = tar::Archive::new(cursor);

            let mut entries: Vec<Vec<(String, ConcreteReturn)>> = Vec::new();
            for entry_result in archive
                .entries()
                .map_err(|e| format!("archive.tar_extract() invalid tar: {}", e))?
            {
                let mut entry = entry_result
                    .map_err(|e| format!("archive.tar_extract() failed to read entry: {}", e))?;

                if entry.header().entry_type().is_dir() {
                    continue;
                }

                let name = entry
                    .path()
                    .map_err(|e| format!("archive.tar_extract() invalid path: {}", e))?
                    .to_string_lossy()
                    .to_string();

                let mut contents = String::new();
                entry.read_to_string(&mut contents).map_err(|e| {
                    format!("archive.tar_extract() failed to read '{}': {}", name, e)
                })?;

                entries.push(vec![
                    ("name".to_string(), ConcreteReturn::String(name)),
                    ("data".to_string(), ConcreteReturn::String(contents)),
                ]);
            }

            Ok(TypedReturn::ArrayObjectPairs(entries))
        },
    );

    module
}
