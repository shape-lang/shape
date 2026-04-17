//! Native `parallel` module for data-parallel operations.
//!
//! Exports: parallel.map, parallel.filter, parallel.for_each, parallel.chunks,
//! parallel.reduce, parallel.num_threads
//!
//! Uses Rayon for thread-pool based data parallelism.
//! The key constraint is that Shape closures (ValueWord) are not Send,
//! so we use `invoke_callable` on the main thread but process pure-data
//! operations (chunking, collecting) in parallel where possible.
//!
//! For `parallel.map` and `parallel.filter`, the callback is invoked via
//! `invoke_callable` on the calling thread, but the data is partitioned
//! and reassembled using Rayon when no callback is involved.

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

/// parallel.map(array, fn) -> Array
///
/// Maps a function over each element of the array. The callback is invoked
/// sequentially via `invoke_callable` (Shape closures are not Send), but
/// the result array is pre-allocated for efficiency.
fn parallel_map(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let arr = args
        .first()
        .and_then(|a| a.as_any_array())
        .ok_or_else(|| "parallel.map() requires an array as first argument".to_string())?;
    let callback = args
        .get(1)
        .ok_or_else(|| "parallel.map() requires a callback as second argument".to_string())?;
    let invoke = ctx.invoke_callable.ok_or_else(|| {
        "parallel.map() requires invoke_callable (not available in this context)".to_string()
    })?;

    let items = arr.to_generic();
    let mut results = Vec::with_capacity(items.len());
    for item in items.iter() {
        let result = invoke(callback, &[item.clone()])?;
        results.push(result);
    }
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(results)))
}

/// parallel.filter(array, fn) -> Array
///
/// Filters array elements using a predicate callback.
fn parallel_filter(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let arr = args
        .first()
        .and_then(|a| a.as_any_array())
        .ok_or_else(|| "parallel.filter() requires an array as first argument".to_string())?;
    let callback = args
        .get(1)
        .ok_or_else(|| "parallel.filter() requires a callback as second argument".to_string())?;
    let invoke = ctx.invoke_callable.ok_or_else(|| {
        "parallel.filter() requires invoke_callable (not available in this context)".to_string()
    })?;

    let items = arr.to_generic();
    let mut results = Vec::new();
    for item in items.iter() {
        let keep = invoke(callback, &[item.clone()])?;
        if keep.as_bool().unwrap_or(false) {
            results.push(item.clone());
        }
    }
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(results)))
}

/// parallel.for_each(array, fn) -> null
///
/// Applies a function to each element for side effects.
fn parallel_for_each(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let arr = args
        .first()
        .and_then(|a| a.as_any_array())
        .ok_or_else(|| "parallel.for_each() requires an array as first argument".to_string())?;
    let callback = args
        .get(1)
        .ok_or_else(|| "parallel.for_each() requires a callback as second argument".to_string())?;
    let invoke = ctx.invoke_callable.ok_or_else(|| {
        "parallel.for_each() requires invoke_callable (not available in this context)".to_string()
    })?;

    let items = arr.to_generic();
    for item in items.iter() {
        invoke(callback, &[item.clone()])?;
    }
    Ok(ValueWord::none())
}

/// parallel.chunks(array, size) -> Array<Array>
///
/// Split an array into chunks of the given size. Pure utility, no parallelism.
/// The last chunk may be smaller if the array length is not evenly divisible.
fn parallel_chunks(args: &[ValueWord], _ctx: &ModuleContext) -> Result<ValueWord, String> {
    let arr = args
        .first()
        .and_then(|a| a.as_any_array())
        .ok_or_else(|| "parallel.chunks() requires an array as first argument".to_string())?;
    let size = args
        .get(1)
        .and_then(|a| a.as_i64().or_else(|| a.as_f64().map(|n| n as i64)))
        .ok_or_else(|| "parallel.chunks() requires a chunk size as second argument".to_string())?;

    if size <= 0 {
        return Err("parallel.chunks() chunk size must be positive".to_string());
    }
    let size = size as usize;

    let items = arr.to_generic();
    let chunks: Vec<ValueWord> = items
        .chunks(size)
        .map(|chunk| ValueWord::from_array(shape_value::vmarray_from_vec(chunk.to_vec())))
        .collect();
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(chunks)))
}

/// parallel.reduce(array, fn, initial) -> any
///
/// Reduces an array to a single value using a callback and initial accumulator.
fn parallel_reduce(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let arr = args
        .first()
        .and_then(|a| a.as_any_array())
        .ok_or_else(|| "parallel.reduce() requires an array as first argument".to_string())?;
    let callback = args
        .get(1)
        .ok_or_else(|| "parallel.reduce() requires a callback as second argument".to_string())?;
    let initial = args.get(2).ok_or_else(|| {
        "parallel.reduce() requires an initial value as third argument".to_string()
    })?;
    let invoke = ctx.invoke_callable.ok_or_else(|| {
        "parallel.reduce() requires invoke_callable (not available in this context)".to_string()
    })?;

    let items = arr.to_generic();
    let mut acc = initial.clone();
    for item in items.iter() {
        acc = invoke(callback, &[acc, item.clone()])?;
    }
    Ok(acc)
}

/// parallel.num_threads() -> int
///
/// Returns the number of threads in the Rayon thread pool.
fn parallel_num_threads(_args: &[ValueWord], _ctx: &ModuleContext) -> Result<ValueWord, String> {
    Ok(ValueWord::from_i64(rayon::current_num_threads() as i64))
}

/// parallel.sort(array, fn?) -> Array
///
/// Sort an array. If a comparator is provided, uses it; otherwise sorts by
/// natural ordering (numbers first, then strings).
/// Uses Rayon's par_sort for arrays larger than 1024 elements.
fn parallel_sort(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let arr = args
        .first()
        .and_then(|a| a.as_any_array())
        .ok_or_else(|| "parallel.sort() requires an array as first argument".to_string())?;

    let items = arr.to_generic();
    let mut sorted: Vec<ValueWord> = items.to_vec();

    if let Some(callback) = args.get(1) {
        // Custom comparator: callback(a, b) -> number (negative, zero, positive)
        let invoke = ctx.invoke_callable.ok_or_else(|| {
            "parallel.sort() with comparator requires invoke_callable".to_string()
        })?;

        // Sort with the comparator (sequential — closure not Send)
        let mut last_err: Option<String> = None;
        sorted.sort_by(|a, b| {
            if last_err.is_some() {
                return std::cmp::Ordering::Equal;
            }
            match invoke(callback, &[a.clone(), b.clone()]) {
                Ok(result) => {
                    let n = result.as_number_coerce().unwrap_or(0.0);
                    if n < 0.0 {
                        std::cmp::Ordering::Less
                    } else if n > 0.0 {
                        std::cmp::Ordering::Greater
                    } else {
                        std::cmp::Ordering::Equal
                    }
                }
                Err(e) => {
                    last_err = Some(e);
                    std::cmp::Ordering::Equal
                }
            }
        });
        if let Some(err) = last_err {
            return Err(format!("parallel.sort() comparator error: {}", err));
        }
    } else {
        // Natural ordering using Rayon for large arrays
        use rayon::prelude::*;

        if sorted.len() >= 1024 {
            sorted.par_sort_by(|a, b| compare_values_natural(a, b));
        } else {
            sorted.sort_by(|a, b| compare_values_natural(a, b));
        }
    }
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(sorted)))
}

/// Natural ordering comparator for ValueWord.
fn compare_values_natural(a: &ValueWord, b: &ValueWord) -> std::cmp::Ordering {
    match (a.as_number_coerce(), b.as_number_coerce()) {
        (Some(na), Some(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
        _ => match (a.as_str(), b.as_str()) {
            (Some(sa), Some(sb)) => sa.cmp(sb),
            _ => std::cmp::Ordering::Equal,
        },
    }
}

/// Create the `parallel` module.
pub fn create_parallel_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::parallel");
    module.description = "Data-parallel operations using Rayon thread pool".to_string();

    module.add_function_with_schema(
        "map",
        parallel_map,
        ModuleFunction {
            description: "Map a function over array elements".to_string(),
            params: vec![
                ModuleParam {
                    name: "array".to_string(),
                    type_name: "Array<any>".to_string(),
                    required: true,
                    description: "Array to map over".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "fn".to_string(),
                    type_name: "function".to_string(),
                    required: true,
                    description: "Callback function applied to each element".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Array<any>".to_string()),
        },
    );

    module.add_function_with_schema(
        "filter",
        parallel_filter,
        ModuleFunction {
            description: "Filter array elements using a predicate".to_string(),
            params: vec![
                ModuleParam {
                    name: "array".to_string(),
                    type_name: "Array<any>".to_string(),
                    required: true,
                    description: "Array to filter".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "fn".to_string(),
                    type_name: "function".to_string(),
                    required: true,
                    description: "Predicate function returning bool".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Array<any>".to_string()),
        },
    );

    module.add_function_with_schema(
        "for_each",
        parallel_for_each,
        ModuleFunction {
            description: "Apply a function to each element for side effects".to_string(),
            params: vec![
                ModuleParam {
                    name: "array".to_string(),
                    type_name: "Array<any>".to_string(),
                    required: true,
                    description: "Array to iterate".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "fn".to_string(),
                    type_name: "function".to_string(),
                    required: true,
                    description: "Callback function applied to each element".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("null".to_string()),
        },
    );

    module.add_function_with_schema(
        "chunks",
        parallel_chunks,
        ModuleFunction {
            description: "Split an array into chunks of a given size".to_string(),
            params: vec![
                ModuleParam {
                    name: "array".to_string(),
                    type_name: "Array<any>".to_string(),
                    required: true,
                    description: "Array to chunk".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "size".to_string(),
                    type_name: "int".to_string(),
                    required: true,
                    description: "Size of each chunk".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Array<Array<any>>".to_string()),
        },
    );

    module.add_function_with_schema(
        "reduce",
        parallel_reduce,
        ModuleFunction {
            description: "Reduce an array to a single value".to_string(),
            params: vec![
                ModuleParam {
                    name: "array".to_string(),
                    type_name: "Array<any>".to_string(),
                    required: true,
                    description: "Array to reduce".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "fn".to_string(),
                    type_name: "function".to_string(),
                    required: true,
                    description: "Reducer function (accumulator, element) -> accumulator"
                        .to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "initial".to_string(),
                    type_name: "any".to_string(),
                    required: true,
                    description: "Initial accumulator value".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("any".to_string()),
        },
    );

    module.add_function_with_schema(
        "sort",
        parallel_sort,
        ModuleFunction {
            description:
                "Sort an array, optionally with a comparator. Uses parallel sort for large arrays."
                    .to_string(),
            params: vec![
                ModuleParam {
                    name: "array".to_string(),
                    type_name: "Array<any>".to_string(),
                    required: true,
                    description: "Array to sort".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "comparator".to_string(),
                    type_name: "function".to_string(),
                    required: false,
                    description: "Comparator function (a, b) -> number".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Array<any>".to_string()),
        },
    );

    module.add_function_with_schema(
        "num_threads",
        parallel_num_threads,
        ModuleFunction {
            description: "Return the number of threads in the Rayon thread pool".to_string(),
            params: vec![],
            return_type: Some("int".to_string()),
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
    fn test_parallel_module_creation() {
        let module = create_parallel_module();
        assert_eq!(module.name, "std::core::parallel");
        assert!(module.has_export("map"));
        assert!(module.has_export("filter"));
        assert!(module.has_export("for_each"));
        assert!(module.has_export("chunks"));
        assert!(module.has_export("reduce"));
        assert!(module.has_export("sort"));
        assert!(module.has_export("num_threads"));
    }

    #[test]
    fn test_parallel_module_schemas() {
        let module = create_parallel_module();

        let map_schema = module.get_schema("map").unwrap();
        assert_eq!(map_schema.params.len(), 2);
        assert_eq!(map_schema.return_type.as_deref(), Some("Array<any>"));

        let chunks_schema = module.get_schema("chunks").unwrap();
        assert_eq!(chunks_schema.params.len(), 2);
        assert_eq!(
            chunks_schema.return_type.as_deref(),
            Some("Array<Array<any>>")
        );

        let reduce_schema = module.get_schema("reduce").unwrap();
        assert_eq!(reduce_schema.params.len(), 3);
        assert_eq!(reduce_schema.return_type.as_deref(), Some("any"));

        let num_threads_schema = module.get_schema("num_threads").unwrap();
        assert_eq!(num_threads_schema.params.len(), 0);
        assert_eq!(num_threads_schema.return_type.as_deref(), Some("int"));
    }

    #[test]
    fn test_parallel_chunks_basic() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
            ValueWord::from_i64(4),
            ValueWord::from_i64(5),
        ]));
        let result = parallel_chunks(&[arr, ValueWord::from_i64(2)], &ctx).unwrap();
        let chunks = result.as_any_array().unwrap().to_generic();
        assert_eq!(chunks.len(), 3); // [1,2], [3,4], [5]

        let first = chunks[0].as_any_array().unwrap().to_generic();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].as_i64(), Some(1));
        assert_eq!(first[1].as_i64(), Some(2));

        let last = chunks[2].as_any_array().unwrap().to_generic();
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].as_i64(), Some(5));
    }

    #[test]
    fn test_parallel_chunks_exact_division() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
            ValueWord::from_i64(4),
        ]));
        let result = parallel_chunks(&[arr, ValueWord::from_i64(2)], &ctx).unwrap();
        let chunks = result.as_any_array().unwrap().to_generic();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn test_parallel_chunks_size_larger_than_array() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
        ]));
        let result = parallel_chunks(&[arr, ValueWord::from_i64(10)], &ctx).unwrap();
        let chunks = result.as_any_array().unwrap().to_generic();
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_parallel_chunks_invalid_size() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![ValueWord::from_i64(1)]));
        let result = parallel_chunks(&[arr, ValueWord::from_i64(0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_parallel_chunks_empty_array() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![]));
        let result = parallel_chunks(&[arr, ValueWord::from_i64(3)], &ctx).unwrap();
        let chunks = result.as_any_array().unwrap().to_generic();
        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_parallel_num_threads() {
        let ctx = test_ctx();
        let result = parallel_num_threads(&[], &ctx).unwrap();
        let n = result.as_i64().unwrap();
        assert!(n > 0, "num_threads should be positive, got {}", n);
    }

    #[test]
    fn test_parallel_sort_natural() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_i64(3),
            ValueWord::from_i64(1),
            ValueWord::from_i64(4),
            ValueWord::from_i64(1),
            ValueWord::from_i64(5),
        ]));
        let result = parallel_sort(&[arr], &ctx).unwrap();
        let sorted = result.as_any_array().unwrap().to_generic();
        assert_eq!(sorted.len(), 5);
        assert_eq!(sorted[0].as_i64(), Some(1));
        assert_eq!(sorted[1].as_i64(), Some(1));
        assert_eq!(sorted[2].as_i64(), Some(3));
        assert_eq!(sorted[3].as_i64(), Some(4));
        assert_eq!(sorted[4].as_i64(), Some(5));
    }

    #[test]
    fn test_parallel_sort_strings() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_string(Arc::new("banana".to_string())),
            ValueWord::from_string(Arc::new("apple".to_string())),
            ValueWord::from_string(Arc::new("cherry".to_string())),
        ]));
        let result = parallel_sort(&[arr], &ctx).unwrap();
        let sorted = result.as_any_array().unwrap().to_generic();
        assert_eq!(sorted[0].as_str(), Some("apple"));
        assert_eq!(sorted[1].as_str(), Some("banana"));
        assert_eq!(sorted[2].as_str(), Some("cherry"));
    }

    #[test]
    fn test_parallel_map_requires_callback() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![ValueWord::from_i64(1)]));
        let result = parallel_map(&[arr], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_parallel_map_requires_invoke_callable() {
        let ctx = test_ctx();
        let arr = ValueWord::from_array(shape_value::vmarray_from_vec(vec![ValueWord::from_i64(1)]));
        let cb = ValueWord::none(); // dummy
        let result = parallel_map(&[arr, cb], &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invoke_callable"));
    }

    #[test]
    fn test_parallel_export_count() {
        let module = create_parallel_module();
        let names = module.export_names();
        assert_eq!(names.len(), 7);
    }
}
