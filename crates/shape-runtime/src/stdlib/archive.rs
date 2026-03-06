//! Native `archive` module for creating and extracting zip/tar archives.
//!
//! Exports: archive.zip_create, archive.zip_extract, archive.tar_create, archive.tar_extract

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use shape_value::heap_value::HeapValue;
use std::sync::Arc;

/// Extract a byte array (Array<int>) from a ValueWord into a Vec<u8>.
fn bytes_from_array(val: &ValueWord) -> Result<Vec<u8>, String> {
    let arr = val
        .as_any_array()
        .ok_or_else(|| "expected an Array<int> of bytes".to_string())?
        .to_generic();
    let mut bytes = Vec::with_capacity(arr.len());
    for item in arr.iter() {
        let byte_val = item
            .as_i64()
            .or_else(|| item.as_f64().map(|n| n as i64))
            .ok_or_else(|| "array elements must be integers (0-255)".to_string())?;
        if !(0..=255).contains(&byte_val) {
            return Err(format!("byte value out of range: {}", byte_val));
        }
        bytes.push(byte_val as u8);
    }
    Ok(bytes)
}

/// Convert a Vec<u8> into a ValueWord Array<int>.
fn bytes_to_array(bytes: &[u8]) -> ValueWord {
    let items: Vec<ValueWord> = bytes
        .iter()
        .map(|&b| ValueWord::from_i64(b as i64))
        .collect();
    ValueWord::from_array(Arc::new(items))
}

/// Extract entries from an Array of {name: string, data: string} objects.
/// Supports both TypedObject and HashMap representations.
fn extract_entries(val: &ValueWord) -> Result<Vec<(String, String)>, String> {
    let arr = val
        .as_any_array()
        .ok_or_else(|| "expected an Array of entry objects".to_string())?
        .to_generic();

    let mut entries = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let (name, data) =
            extract_entry_fields(item).map_err(|e| format!("entry [{}]: {}", i, e))?;
        entries.push((name, data));
    }
    Ok(entries)
}

/// Extract `name` and `data` fields from a single entry (TypedObject or HashMap).
fn extract_entry_fields(val: &ValueWord) -> Result<(String, String), String> {
    // Try TypedObject first
    if let Some(HeapValue::TypedObject {
        slots, heap_mask, ..
    }) = val.as_heap_ref()
    {
        // Convention: slot 0 = name, slot 1 = data (both heap/string)
        if slots.len() >= 2 {
            let name_nb = if heap_mask & 1 != 0 {
                slots[0].as_heap_nb()
            } else {
                unsafe { ValueWord::clone_from_bits(slots[0].raw()) }
            };
            let data_nb = if heap_mask & 2 != 0 {
                slots[1].as_heap_nb()
            } else {
                unsafe { ValueWord::clone_from_bits(slots[1].raw()) }
            };
            if let (Some(name), Some(data)) = (name_nb.as_str(), data_nb.as_str()) {
                return Ok((name.to_string(), data.to_string()));
            }
        }
    }

    // Try HashMap
    if let Some((keys, values, _)) = val.as_hashmap() {
        let mut name = None;
        let mut data = None;
        for (k, v) in keys.iter().zip(values.iter()) {
            if let Some(key_str) = k.as_str() {
                match key_str {
                    "name" => name = v.as_str().map(|s| s.to_string()),
                    "data" => data = v.as_str().map(|s| s.to_string()),
                    _ => {}
                }
            }
        }
        if let (Some(n), Some(d)) = (name, data) {
            return Ok((n, d));
        }
    }

    Err("entry must have 'name' (string) and 'data' (string) fields".to_string())
}

/// Build an entry object as a HashMap with `name` and `data` keys.
fn make_entry(name: &str, data: &str) -> ValueWord {
    let keys = vec![
        ValueWord::from_string(Arc::new("name".to_string())),
        ValueWord::from_string(Arc::new("data".to_string())),
    ];
    let values = vec![
        ValueWord::from_string(Arc::new(name.to_string())),
        ValueWord::from_string(Arc::new(data.to_string())),
    ];
    ValueWord::from_hashmap_pairs(keys, values)
}

/// Create the `archive` module with zip/tar creation and extraction functions.
pub fn create_archive_module() -> ModuleExports {
    let mut module = ModuleExports::new("archive");
    module.description = "Archive creation and extraction (zip, tar)".to_string();

    // archive.zip_create(entries: Array<{name: string, data: string}>) -> Array<int>
    module.add_function_with_schema(
        "zip_create",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use std::io::{Cursor, Write};

            let entries_val = args
                .first()
                .ok_or_else(|| "archive.zip_create() requires an entries array".to_string())?;
            let entries =
                extract_entries(entries_val).map_err(|e| format!("archive.zip_create(): {}", e))?;

            let buf = Cursor::new(Vec::new());
            let mut zip_writer = zip::ZipWriter::new(buf);

            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            for (name, data) in &entries {
                zip_writer.start_file(name.as_str(), options).map_err(|e| {
                    format!(
                        "archive.zip_create() failed to start file '{}': {}",
                        name, e
                    )
                })?;
                zip_writer.write_all(data.as_bytes()).map_err(|e| {
                    format!("archive.zip_create() failed to write '{}': {}", name, e)
                })?;
            }

            let cursor = zip_writer
                .finish()
                .map_err(|e| format!("archive.zip_create() failed to finish: {}", e))?;

            Ok(bytes_to_array(&cursor.into_inner()))
        },
        ModuleFunction {
            description: "Create a zip archive in memory from an array of entries".to_string(),
            params: vec![ModuleParam {
                name: "entries".to_string(),
                type_name: "Array<{name: string, data: string}>".to_string(),
                required: true,
                description: "Array of objects with 'name' and 'data' fields".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<int>".to_string()),
        },
    );

    // archive.zip_extract(data: Array<int>) -> Array<{name: string, data: string}>
    module.add_function_with_schema(
        "zip_extract",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use std::io::{Cursor, Read};

            let input = args.first().ok_or_else(|| {
                "archive.zip_extract() requires an Array<int> argument".to_string()
            })?;
            let bytes =
                bytes_from_array(input).map_err(|e| format!("archive.zip_extract(): {}", e))?;

            let cursor = Cursor::new(bytes);
            let mut archive = zip::ZipArchive::new(cursor)
                .map_err(|e| format!("archive.zip_extract() invalid zip: {}", e))?;

            let mut entries = Vec::new();
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

                entries.push(make_entry(&name, &contents));
            }

            Ok(ValueWord::from_array(Arc::new(entries)))
        },
        ModuleFunction {
            description: "Extract a zip archive from a byte array into an array of entries"
                .to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "Array<int>".to_string(),
                required: true,
                description: "Zip archive as byte array".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<{name: string, data: string}>".to_string()),
        },
    );

    // archive.tar_create(entries: Array<{name: string, data: string}>) -> Array<int>
    module.add_function_with_schema(
        "tar_create",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let entries_val = args
                .first()
                .ok_or_else(|| "archive.tar_create() requires an entries array".to_string())?;
            let entries =
                extract_entries(entries_val).map_err(|e| format!("archive.tar_create(): {}", e))?;

            let mut builder = tar::Builder::new(Vec::new());

            for (name, data) in &entries {
                let data_bytes = data.as_bytes();
                let mut header = tar::Header::new_gnu();
                header.set_size(data_bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();

                builder
                    .append_data(&mut header, name.as_str(), data_bytes)
                    .map_err(|e| format!("archive.tar_create() failed for '{}': {}", name, e))?;
            }

            let tar_bytes = builder
                .into_inner()
                .map_err(|e| format!("archive.tar_create() failed to finish: {}", e))?;

            Ok(bytes_to_array(&tar_bytes))
        },
        ModuleFunction {
            description: "Create a tar archive in memory from an array of entries".to_string(),
            params: vec![ModuleParam {
                name: "entries".to_string(),
                type_name: "Array<{name: string, data: string}>".to_string(),
                required: true,
                description: "Array of objects with 'name' and 'data' fields".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<int>".to_string()),
        },
    );

    // archive.tar_extract(data: Array<int>) -> Array<{name: string, data: string}>
    module.add_function_with_schema(
        "tar_extract",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use std::io::{Cursor, Read};

            let input = args.first().ok_or_else(|| {
                "archive.tar_extract() requires an Array<int> argument".to_string()
            })?;
            let bytes =
                bytes_from_array(input).map_err(|e| format!("archive.tar_extract(): {}", e))?;

            let cursor = Cursor::new(bytes);
            let mut archive = tar::Archive::new(cursor);

            let mut entries = Vec::new();
            for entry_result in archive
                .entries()
                .map_err(|e| format!("archive.tar_extract() invalid tar: {}", e))?
            {
                let mut entry = entry_result
                    .map_err(|e| format!("archive.tar_extract() failed to read entry: {}", e))?;

                // Skip directories
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

                entries.push(make_entry(&name, &contents));
            }

            Ok(ValueWord::from_array(Arc::new(entries)))
        },
        ModuleFunction {
            description: "Extract a tar archive from a byte array into an array of entries"
                .to_string(),
            params: vec![ModuleParam {
                name: "data".to_string(),
                type_name: "Array<int>".to_string(),
                required: true,
                description: "Tar archive as byte array".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<{name: string, data: string}>".to_string()),
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

    fn make_test_entries() -> ValueWord {
        let entries = vec![
            make_entry("hello.txt", "Hello, World!"),
            make_entry("data/numbers.txt", "1 2 3 4 5"),
        ];
        ValueWord::from_array(Arc::new(entries))
    }

    #[test]
    fn test_archive_module_creation() {
        let module = create_archive_module();
        assert_eq!(module.name, "archive");
        assert!(module.has_export("zip_create"));
        assert!(module.has_export("zip_extract"));
        assert!(module.has_export("tar_create"));
        assert!(module.has_export("tar_extract"));
    }

    #[test]
    fn test_zip_roundtrip() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let zip_create_fn = module.get_export("zip_create").unwrap();
        let zip_extract_fn = module.get_export("zip_extract").unwrap();

        let entries = make_test_entries();
        let zip_bytes = zip_create_fn(&[entries], &ctx).unwrap();

        // Should be a byte array
        assert!(zip_bytes.as_any_array().is_some());

        let extracted = zip_extract_fn(&[zip_bytes], &ctx).unwrap();
        let arr = extracted.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 2);

        // Check first entry
        let (name0, data0) = extract_entry_fields(&arr[0]).unwrap();
        assert_eq!(name0, "hello.txt");
        assert_eq!(data0, "Hello, World!");

        // Check second entry
        let (name1, data1) = extract_entry_fields(&arr[1]).unwrap();
        assert_eq!(name1, "data/numbers.txt");
        assert_eq!(data1, "1 2 3 4 5");
    }

    #[test]
    fn test_tar_roundtrip() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let tar_create_fn = module.get_export("tar_create").unwrap();
        let tar_extract_fn = module.get_export("tar_extract").unwrap();

        let entries = make_test_entries();
        let tar_bytes = tar_create_fn(&[entries], &ctx).unwrap();

        assert!(tar_bytes.as_any_array().is_some());

        let extracted = tar_extract_fn(&[tar_bytes], &ctx).unwrap();
        let arr = extracted.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 2);

        let (name0, data0) = extract_entry_fields(&arr[0]).unwrap();
        assert_eq!(name0, "hello.txt");
        assert_eq!(data0, "Hello, World!");

        let (name1, data1) = extract_entry_fields(&arr[1]).unwrap();
        assert_eq!(name1, "data/numbers.txt");
        assert_eq!(data1, "1 2 3 4 5");
    }

    #[test]
    fn test_zip_create_empty() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let zip_create_fn = module.get_export("zip_create").unwrap();
        let zip_extract_fn = module.get_export("zip_extract").unwrap();

        let empty = ValueWord::from_array(Arc::new(Vec::new()));
        let zip_bytes = zip_create_fn(&[empty], &ctx).unwrap();

        let extracted = zip_extract_fn(&[zip_bytes], &ctx).unwrap();
        let arr = extracted.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 0);
    }

    #[test]
    fn test_tar_create_empty() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let tar_create_fn = module.get_export("tar_create").unwrap();
        let tar_extract_fn = module.get_export("tar_extract").unwrap();

        let empty = ValueWord::from_array(Arc::new(Vec::new()));
        let tar_bytes = tar_create_fn(&[empty], &ctx).unwrap();

        let extracted = tar_extract_fn(&[tar_bytes], &ctx).unwrap();
        let arr = extracted.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 0);
    }

    #[test]
    fn test_zip_extract_invalid_data() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let zip_extract_fn = module.get_export("zip_extract").unwrap();

        let bad_data = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
        ]));
        let result = zip_extract_fn(&[bad_data], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_tar_extract_invalid_data() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let tar_extract_fn = module.get_export("tar_extract").unwrap();

        let bad_data = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
        ]));
        let result = tar_extract_fn(&[bad_data], &ctx);
        // tar with just 2 bytes will likely result in empty entries (not enough for header)
        // or an error — either is acceptable
        if let Ok(val) = result {
            let arr = val.as_any_array().unwrap().to_generic();
            assert_eq!(arr.len(), 0);
        }
    }

    #[test]
    fn test_zip_create_requires_array() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let zip_create_fn = module.get_export("zip_create").unwrap();

        let result = zip_create_fn(&[ValueWord::from_i64(42)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_schemas() {
        let module = create_archive_module();

        let zip_create_schema = module.get_schema("zip_create").unwrap();
        assert_eq!(zip_create_schema.params.len(), 1);
        assert!(zip_create_schema.params[0].required);
        assert_eq!(zip_create_schema.return_type.as_deref(), Some("Array<int>"));

        let zip_extract_schema = module.get_schema("zip_extract").unwrap();
        assert_eq!(
            zip_extract_schema.return_type.as_deref(),
            Some("Array<{name: string, data: string}>")
        );

        let tar_create_schema = module.get_schema("tar_create").unwrap();
        assert_eq!(tar_create_schema.params.len(), 1);

        let tar_extract_schema = module.get_schema("tar_extract").unwrap();
        assert_eq!(
            tar_extract_schema.return_type.as_deref(),
            Some("Array<{name: string, data: string}>")
        );
    }

    #[test]
    fn test_zip_unicode_content() {
        let module = create_archive_module();
        let ctx = test_ctx();
        let zip_create_fn = module.get_export("zip_create").unwrap();
        let zip_extract_fn = module.get_export("zip_extract").unwrap();

        let entries = vec![make_entry("unicode.txt", "Hello \u{1F600} World \u{00E9}")];
        let input = ValueWord::from_array(Arc::new(entries));
        let zip_bytes = zip_create_fn(&[input], &ctx).unwrap();

        let extracted = zip_extract_fn(&[zip_bytes], &ctx).unwrap();
        let arr = extracted.as_any_array().unwrap().to_generic();
        let (_, data) = extract_entry_fields(&arr[0]).unwrap();
        assert_eq!(data, "Hello \u{1F600} World \u{00E9}");
    }
}
