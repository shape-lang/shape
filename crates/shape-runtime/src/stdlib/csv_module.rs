//! Native `csv` module for CSV parsing and serialization.
//!
//! Exports: csv.parse, csv.parse_records, csv.stringify, csv.stringify_records,
//!          csv.read_file, csv.is_valid

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use std::sync::Arc;

/// Create the `csv` module with CSV parsing and serialization functions.
pub fn create_csv_module() -> ModuleExports {
    let mut module = ModuleExports::new("csv");
    module.description = "CSV parsing and serialization".to_string();

    // csv.parse(text: string) -> Array<Array<string>>
    module.add_function_with_schema(
        "parse",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "csv.parse() requires a string argument".to_string())?;

            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(text.as_bytes());

            let mut rows: Vec<ValueWord> = Vec::new();
            for result in reader.records() {
                let record = result.map_err(|e| format!("csv.parse() failed: {}", e))?;
                let row: Vec<ValueWord> = record
                    .iter()
                    .map(|field| ValueWord::from_string(Arc::new(field.to_string())))
                    .collect();
                rows.push(ValueWord::from_array(Arc::new(row)));
            }

            Ok(ValueWord::from_array(Arc::new(rows)))
        },
        ModuleFunction {
            description: "Parse CSV text into an array of rows (each row is an array of strings)"
                .to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "CSV text to parse".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<Array<string>>".to_string()),
        },
    );

    // csv.parse_records(text: string) -> Array<HashMap<string, string>>
    module.add_function_with_schema(
        "parse_records",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "csv.parse_records() requires a string argument".to_string())?;

            let mut reader = csv::ReaderBuilder::new()
                .has_headers(true)
                .from_reader(text.as_bytes());

            let headers: Vec<String> = reader
                .headers()
                .map_err(|e| format!("csv.parse_records() failed to read headers: {}", e))?
                .iter()
                .map(|h| h.to_string())
                .collect();

            let mut records: Vec<ValueWord> = Vec::new();
            for result in reader.records() {
                let record = result.map_err(|e| format!("csv.parse_records() failed: {}", e))?;
                let mut keys = Vec::with_capacity(headers.len());
                let mut values = Vec::with_capacity(headers.len());
                for (i, field) in record.iter().enumerate() {
                    if i < headers.len() {
                        keys.push(ValueWord::from_string(Arc::new(headers[i].clone())));
                        values.push(ValueWord::from_string(Arc::new(field.to_string())));
                    }
                }
                records.push(ValueWord::from_hashmap_pairs(keys, values));
            }

            Ok(ValueWord::from_array(Arc::new(records)))
        },
        ModuleFunction {
            description:
                "Parse CSV text using the header row as keys, returning an array of hashmaps"
                    .to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "CSV text to parse (first row is headers)".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<HashMap<string, string>>".to_string()),
        },
    );

    // csv.stringify(data: Array<Array<string>>, delimiter?: string) -> string
    module.add_function_with_schema(
        "stringify",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let data = args
                .first()
                .and_then(|a| a.as_any_array())
                .ok_or_else(|| {
                    "csv.stringify() requires an Array<Array<string>> argument".to_string()
                })?
                .to_generic();

            let delimiter = args
                .get(1)
                .and_then(|a| a.as_str())
                .and_then(|s| s.as_bytes().first().copied())
                .unwrap_or(b',');

            let mut writer = csv::WriterBuilder::new()
                .delimiter(delimiter)
                .from_writer(Vec::new());

            for row_val in data.iter() {
                let row_arr = row_val.as_any_array().ok_or_else(|| {
                    "csv.stringify() each row must be an Array<string>".to_string()
                })?;
                let row = row_arr.to_generic();
                let fields: Vec<String> = row
                    .iter()
                    .map(|f| {
                        f.as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| f.to_string())
                    })
                    .collect();
                writer
                    .write_record(&fields)
                    .map_err(|e| format!("csv.stringify() failed: {}", e))?;
            }

            let bytes = writer
                .into_inner()
                .map_err(|e| format!("csv.stringify() failed to flush: {}", e))?;
            let output = String::from_utf8(bytes)
                .map_err(|e| format!("csv.stringify() UTF-8 error: {}", e))?;

            Ok(ValueWord::from_string(Arc::new(output)))
        },
        ModuleFunction {
            description: "Convert an array of rows to a CSV string".to_string(),
            params: vec![
                ModuleParam {
                    name: "data".to_string(),
                    type_name: "Array<Array<string>>".to_string(),
                    required: true,
                    description: "Array of rows, each row is an array of field strings".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "delimiter".to_string(),
                    type_name: "string".to_string(),
                    required: false,
                    description: "Field delimiter character (default: comma)".to_string(),
                    default_snippet: Some("\",\"".to_string()),
                    ..Default::default()
                },
            ],
            return_type: Some("string".to_string()),
        },
    );

    // csv.stringify_records(data: Array<HashMap<string, string>>, headers?: Array<string>) -> string
    module.add_function_with_schema(
        "stringify_records",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let data = args
                .first()
                .and_then(|a| a.as_any_array())
                .ok_or_else(|| {
                    "csv.stringify_records() requires an Array<HashMap<string, string>> argument"
                        .to_string()
                })?
                .to_generic();

            // Determine headers: explicit argument or from first record's keys.
            let explicit_headers: Option<Vec<String>> = args.get(1).and_then(|a| {
                let arr = a.as_any_array()?.to_generic();
                let headers: Vec<String> = arr
                    .iter()
                    .filter_map(|h| h.as_str().map(|s| s.to_string()))
                    .collect();
                if headers.is_empty() {
                    None
                } else {
                    Some(headers)
                }
            });

            let headers = if let Some(h) = explicit_headers {
                h
            } else if let Some(first) = data.first() {
                let (keys, _, _) = first.as_hashmap().ok_or_else(|| {
                    "csv.stringify_records() each element must be a HashMap".to_string()
                })?;
                keys.iter()
                    .filter_map(|k| k.as_str().map(|s| s.to_string()))
                    .collect()
            } else {
                return Ok(ValueWord::from_string(Arc::new(String::new())));
            };

            let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());

            // Write header row
            writer
                .write_record(&headers)
                .map_err(|e| format!("csv.stringify_records() failed: {}", e))?;

            // Write data rows
            for record_val in data.iter() {
                let (keys, values, _) = record_val.as_hashmap().ok_or_else(|| {
                    "csv.stringify_records() each element must be a HashMap".to_string()
                })?;

                let mut row = Vec::with_capacity(headers.len());
                for header in &headers {
                    // Find the value for this header key
                    let mut found = String::new();
                    for (i, k) in keys.iter().enumerate() {
                        if let Some(key_str) = k.as_str() {
                            if key_str == header {
                                found = values[i]
                                    .as_str()
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| values[i].to_string());
                                break;
                            }
                        }
                    }
                    row.push(found);
                }

                writer
                    .write_record(&row)
                    .map_err(|e| format!("csv.stringify_records() failed: {}", e))?;
            }

            let bytes = writer
                .into_inner()
                .map_err(|e| format!("csv.stringify_records() failed to flush: {}", e))?;
            let output = String::from_utf8(bytes)
                .map_err(|e| format!("csv.stringify_records() UTF-8 error: {}", e))?;

            Ok(ValueWord::from_string(Arc::new(output)))
        },
        ModuleFunction {
            description: "Convert an array of hashmaps to a CSV string with headers".to_string(),
            params: vec![
                ModuleParam {
                    name: "data".to_string(),
                    type_name: "Array<HashMap<string, string>>".to_string(),
                    required: true,
                    description: "Array of records (hashmaps with string keys and values)"
                        .to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "headers".to_string(),
                    type_name: "Array<string>".to_string(),
                    required: false,
                    description: "Explicit header order (default: keys from first record)"
                        .to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("string".to_string()),
        },
    );

    // csv.read_file(path: string) -> Result<Array<Array<string>>>
    module.add_function_with_schema(
        "read_file",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let path = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "csv.read_file() requires a path string".to_string())?;

            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("csv.read_file() failed to read '{}': {}", path, e))?;

            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(text.as_bytes());

            let mut rows: Vec<ValueWord> = Vec::new();
            for result in reader.records() {
                let record = result.map_err(|e| format!("csv.read_file() parse error: {}", e))?;
                let row: Vec<ValueWord> = record
                    .iter()
                    .map(|field| ValueWord::from_string(Arc::new(field.to_string())))
                    .collect();
                rows.push(ValueWord::from_array(Arc::new(row)));
            }

            Ok(ValueWord::from_ok(ValueWord::from_array(Arc::new(rows))))
        },
        ModuleFunction {
            description: "Read and parse a CSV file into an array of rows".to_string(),
            params: vec![ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Path to the CSV file".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<Array<Array<string>>>".to_string()),
        },
    );

    // csv.is_valid(text: string) -> bool
    module.add_function_with_schema(
        "is_valid",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "csv.is_valid() requires a string argument".to_string())?;

            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(text.as_bytes());

            let valid = reader.records().all(|r| r.is_ok());
            Ok(ValueWord::from_bool(valid))
        },
        ModuleFunction {
            description: "Check if a string is valid CSV".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String to validate as CSV".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
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
    fn test_csv_module_creation() {
        let module = create_csv_module();
        assert_eq!(module.name, "csv");
        assert!(module.has_export("parse"));
        assert!(module.has_export("parse_records"));
        assert!(module.has_export("stringify"));
        assert!(module.has_export("stringify_records"));
        assert!(module.has_export("read_file"));
        assert!(module.has_export("is_valid"));
    }

    #[test]
    fn test_csv_parse_simple() {
        let module = create_csv_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("a,b,c\n1,2,3\n4,5,6".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let rows = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(rows.len(), 3);
        // First row
        let row0 = rows[0]
            .as_any_array()
            .expect("row should be array")
            .to_generic();
        assert_eq!(row0.len(), 3);
        assert_eq!(row0[0].as_str(), Some("a"));
        assert_eq!(row0[1].as_str(), Some("b"));
        assert_eq!(row0[2].as_str(), Some("c"));
        // Second row
        let row1 = rows[1]
            .as_any_array()
            .expect("row should be array")
            .to_generic();
        assert_eq!(row1[0].as_str(), Some("1"));
        assert_eq!(row1[1].as_str(), Some("2"));
        assert_eq!(row1[2].as_str(), Some("3"));
    }

    #[test]
    fn test_csv_parse_quoted_fields() {
        let module = create_csv_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input =
            ValueWord::from_string(Arc::new("\"hello, world\",\"foo\"\"bar\"\n".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let rows = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(rows.len(), 1);
        let row0 = rows[0]
            .as_any_array()
            .expect("row should be array")
            .to_generic();
        assert_eq!(row0[0].as_str(), Some("hello, world"));
        assert_eq!(row0[1].as_str(), Some("foo\"bar"));
    }

    #[test]
    fn test_csv_parse_empty() {
        let module = create_csv_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let rows = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(rows.len(), 0);
    }

    #[test]
    fn test_csv_parse_requires_string() {
        let module = create_csv_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let result = parse_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_parse_records() {
        let module = create_csv_module();
        let parse_fn = module.get_export("parse_records").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("name,age\nAlice,30\nBob,25".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let records = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(records.len(), 2);

        // First record: {name: "Alice", age: "30"}
        let (keys0, values0, _) = records[0].as_hashmap().expect("should be hashmap");
        assert_eq!(keys0.len(), 2);
        // Find "name" key
        let mut found_name = false;
        let mut found_age = false;
        for (i, k) in keys0.iter().enumerate() {
            if k.as_str() == Some("name") {
                assert_eq!(values0[i].as_str(), Some("Alice"));
                found_name = true;
            }
            if k.as_str() == Some("age") {
                assert_eq!(values0[i].as_str(), Some("30"));
                found_age = true;
            }
        }
        assert!(found_name, "should have 'name' key");
        assert!(found_age, "should have 'age' key");
    }

    #[test]
    fn test_csv_parse_records_requires_string() {
        let module = create_csv_module();
        let parse_fn = module.get_export("parse_records").unwrap();
        let ctx = test_ctx();
        let result = parse_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_stringify_simple() {
        let module = create_csv_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let data = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new("a".to_string())),
                ValueWord::from_string(Arc::new("b".to_string())),
            ])),
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new("1".to_string())),
                ValueWord::from_string(Arc::new("2".to_string())),
            ])),
        ]));
        let result = stringify_fn(&[data], &ctx).unwrap();
        let output = result.as_str().expect("should be string");
        assert_eq!(output, "a,b\n1,2\n");
    }

    #[test]
    fn test_csv_stringify_with_delimiter() {
        let module = create_csv_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let data = ValueWord::from_array(Arc::new(vec![ValueWord::from_array(Arc::new(vec![
            ValueWord::from_string(Arc::new("a".to_string())),
            ValueWord::from_string(Arc::new("b".to_string())),
        ]))]));
        let delimiter = ValueWord::from_string(Arc::new("\t".to_string()));
        let result = stringify_fn(&[data, delimiter], &ctx).unwrap();
        let output = result.as_str().expect("should be string");
        assert_eq!(output, "a\tb\n");
    }

    #[test]
    fn test_csv_stringify_requires_array() {
        let module = create_csv_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();
        let result = stringify_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_stringify_records() {
        let module = create_csv_module();
        let stringify_fn = module.get_export("stringify_records").unwrap();
        let ctx = test_ctx();
        let record1 = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("name".to_string())),
                ValueWord::from_string(Arc::new("age".to_string())),
            ],
            vec![
                ValueWord::from_string(Arc::new("Alice".to_string())),
                ValueWord::from_string(Arc::new("30".to_string())),
            ],
        );
        let record2 = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("name".to_string())),
                ValueWord::from_string(Arc::new("age".to_string())),
            ],
            vec![
                ValueWord::from_string(Arc::new("Bob".to_string())),
                ValueWord::from_string(Arc::new("25".to_string())),
            ],
        );
        let data = ValueWord::from_array(Arc::new(vec![record1, record2]));
        let result = stringify_fn(&[data], &ctx).unwrap();
        let output = result.as_str().expect("should be string");
        // Should contain header row and two data rows
        let lines: Vec<&str> = output.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        // Header should be name,age (order from first record's keys)
        assert!(lines[0].contains("name"));
        assert!(lines[0].contains("age"));
    }

    #[test]
    fn test_csv_stringify_records_with_explicit_headers() {
        let module = create_csv_module();
        let stringify_fn = module.get_export("stringify_records").unwrap();
        let ctx = test_ctx();
        let record = ValueWord::from_hashmap_pairs(
            vec![
                ValueWord::from_string(Arc::new("name".to_string())),
                ValueWord::from_string(Arc::new("age".to_string())),
            ],
            vec![
                ValueWord::from_string(Arc::new("Alice".to_string())),
                ValueWord::from_string(Arc::new("30".to_string())),
            ],
        );
        let data = ValueWord::from_array(Arc::new(vec![record]));
        let headers = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_string(Arc::new("age".to_string())),
            ValueWord::from_string(Arc::new("name".to_string())),
        ]));
        let result = stringify_fn(&[data, headers], &ctx).unwrap();
        let output = result.as_str().expect("should be string");
        let lines: Vec<&str> = output.trim().lines().collect();
        assert_eq!(lines[0], "age,name");
        assert_eq!(lines[1], "30,Alice");
    }

    #[test]
    fn test_csv_stringify_records_empty() {
        let module = create_csv_module();
        let stringify_fn = module.get_export("stringify_records").unwrap();
        let ctx = test_ctx();
        let data = ValueWord::from_array(Arc::new(vec![]));
        let result = stringify_fn(&[data], &ctx).unwrap();
        let output = result.as_str().expect("should be string");
        assert_eq!(output, "");
    }

    #[test]
    fn test_csv_is_valid_true() {
        let module = create_csv_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("a,b,c\n1,2,3".to_string()));
        let result = is_valid_fn(&[input], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_csv_is_valid_empty() {
        let module = create_csv_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("".to_string()));
        let result = is_valid_fn(&[input], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_csv_is_valid_requires_string() {
        let module = create_csv_module();
        let is_valid_fn = module.get_export("is_valid").unwrap();
        let ctx = test_ctx();
        let result = is_valid_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_read_file() {
        let module = create_csv_module();
        let read_fn = module.get_export("read_file").unwrap();
        let ctx = test_ctx();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.csv");
        std::fs::write(&path, "a,b\n1,2\n3,4").unwrap();

        let result = read_fn(
            &[ValueWord::from_string(Arc::new(
                path.to_str().unwrap().to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let rows = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(rows.len(), 3);
        let row0 = rows[0]
            .as_any_array()
            .expect("row should be array")
            .to_generic();
        assert_eq!(row0[0].as_str(), Some("a"));
        assert_eq!(row0[1].as_str(), Some("b"));
    }

    #[test]
    fn test_csv_read_file_nonexistent() {
        let module = create_csv_module();
        let read_fn = module.get_export("read_file").unwrap();
        let ctx = test_ctx();
        let result = read_fn(
            &[ValueWord::from_string(Arc::new(
                "/nonexistent/path/file.csv".to_string(),
            ))],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_read_file_requires_string() {
        let module = create_csv_module();
        let read_fn = module.get_export("read_file").unwrap();
        let ctx = test_ctx();
        let result = read_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_schemas() {
        let module = create_csv_module();

        let parse_schema = module.get_schema("parse").unwrap();
        assert_eq!(parse_schema.params.len(), 1);
        assert_eq!(parse_schema.params[0].name, "text");
        assert!(parse_schema.params[0].required);
        assert_eq!(
            parse_schema.return_type.as_deref(),
            Some("Array<Array<string>>")
        );

        let parse_records_schema = module.get_schema("parse_records").unwrap();
        assert_eq!(parse_records_schema.params.len(), 1);
        assert_eq!(
            parse_records_schema.return_type.as_deref(),
            Some("Array<HashMap<string, string>>")
        );

        let stringify_schema = module.get_schema("stringify").unwrap();
        assert_eq!(stringify_schema.params.len(), 2);
        assert!(stringify_schema.params[0].required);
        assert!(!stringify_schema.params[1].required);
        assert_eq!(stringify_schema.return_type.as_deref(), Some("string"));

        let stringify_records_schema = module.get_schema("stringify_records").unwrap();
        assert_eq!(stringify_records_schema.params.len(), 2);
        assert!(stringify_records_schema.params[0].required);
        assert!(!stringify_records_schema.params[1].required);

        let read_file_schema = module.get_schema("read_file").unwrap();
        assert_eq!(read_file_schema.params.len(), 1);
        assert_eq!(
            read_file_schema.return_type.as_deref(),
            Some("Result<Array<Array<string>>>")
        );

        let is_valid_schema = module.get_schema("is_valid").unwrap();
        assert_eq!(is_valid_schema.params.len(), 1);
        assert_eq!(is_valid_schema.return_type.as_deref(), Some("bool"));
    }

    #[test]
    fn test_csv_roundtrip() {
        let module = create_csv_module();
        let parse_fn = module.get_export("parse").unwrap();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        let csv_text = "name,age,city\nAlice,30,NYC\nBob,25,LA\n";
        let parsed = parse_fn(
            &[ValueWord::from_string(Arc::new(csv_text.to_string()))],
            &ctx,
        )
        .unwrap();

        let re_stringified = stringify_fn(&[parsed], &ctx).unwrap();
        let output = re_stringified.as_str().expect("should be string");
        assert_eq!(output, csv_text);
    }

    #[test]
    fn test_csv_records_roundtrip() {
        let module = create_csv_module();
        let parse_records_fn = module.get_export("parse_records").unwrap();
        let stringify_records_fn = module.get_export("stringify_records").unwrap();
        let ctx = test_ctx();

        let csv_text = "name,age\nAlice,30\nBob,25\n";
        let parsed = parse_records_fn(
            &[ValueWord::from_string(Arc::new(csv_text.to_string()))],
            &ctx,
        )
        .unwrap();

        let headers = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_string(Arc::new("name".to_string())),
            ValueWord::from_string(Arc::new("age".to_string())),
        ]));
        let re_stringified = stringify_records_fn(&[parsed, headers], &ctx).unwrap();
        let output = re_stringified.as_str().expect("should be string");
        assert_eq!(output, csv_text);
    }
}
