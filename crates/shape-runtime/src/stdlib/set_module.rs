//! Native `set` module for unordered collections of unique elements.
//!
//! Backed by HashMap for O(1) lookup.
//! Exports: set.new(), set.from_array(arr), set.add(s, item), set.remove(s, item),
//!          set.contains(s, item), set.union(a, b), set.intersection(a, b),
//!          set.difference(a, b), set.to_array(s), set.size(s)

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::{ValueWord, ValueWordExt};

/// Create an empty set (HashMap with no entries).
fn empty_set() -> ValueWord {
    ValueWord::from_hashmap_pairs(Vec::new(), Vec::new())
}

/// Materialize an array argument as `Vec<ValueWord>`, handling both the legacy
/// `HeapValue::Array` / `HeapValue::TypedArray` representations reachable via
/// `as_any_array()` and the v2 raw-ptr `TypedArray<T>` representation held as
/// `NativeScalar::Ptr` (produced by the `NewTypedArrayF64/I64/I32/Bool`
/// opcodes the compiler emits for typed array literals like `[1, 2, 3]`).
///
/// Returns `None` if `arg` is not any recognized array representation.
fn materialize_array_items(arg: &ValueWord) -> Option<Vec<ValueWord>> {
    if let Some(view) = arg.as_any_array() {
        return Some(view.to_generic().iter().copied().collect());
    }
    // v2 raw-pointer fallback: typed array literals compile to a
    // `TypedArray<T>` pointer held as `NativeScalar::Ptr`. Decode the element
    // type from the stamped byte at `HeapHeader` offset 7 and materialize its
    // contents as `ValueWord`s.
    if let Some(shape_value::heap_value::NativeScalar::Ptr(p)) = arg.as_native_scalar() {
        return v2_typed_array_ptr_to_items(p);
    }
    None
}

/// Read a v2 `TypedArray<T>` via its raw pointer and materialize its contents
/// as `Vec<ValueWord>`. Returns `None` for any other heap kind.
///
/// Element-type discriminants mirror
/// `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs`.
fn v2_typed_array_ptr_to_items(p: usize) -> Option<Vec<ValueWord>> {
    use shape_value::v2::heap_header::{HEAP_KIND_V2_TYPED_ARRAY, HeapHeader};
    use shape_value::v2::typed_array::TypedArray;

    const ELEM_TYPE_F64: u8 = 1;
    const ELEM_TYPE_I64: u8 = 2;
    const ELEM_TYPE_I32: u8 = 3;
    const ELEM_TYPE_BOOL: u8 = 4;

    if p == 0 {
        return None;
    }
    // Verify the object kind via the HeapHeader at offset 0.
    let header = unsafe { &*(p as *const HeapHeader) };
    if header.kind != HEAP_KIND_V2_TYPED_ARRAY {
        return None;
    }
    let elem_byte = unsafe { *(p as *const u8).add(7) };
    let items: Vec<ValueWord> = match elem_byte {
        ELEM_TYPE_F64 => {
            let arr = p as *const TypedArray<f64>;
            let slice = unsafe { TypedArray::as_slice(arr) };
            slice.iter().map(|&v| ValueWord::from_f64(v)).collect()
        }
        ELEM_TYPE_I64 => {
            let arr = p as *const TypedArray<i64>;
            let slice = unsafe { TypedArray::as_slice(arr) };
            slice.iter().map(|&v| ValueWord::from_i64(v)).collect()
        }
        ELEM_TYPE_I32 => {
            let arr = p as *const TypedArray<i32>;
            let slice = unsafe { TypedArray::as_slice(arr) };
            slice.iter().map(|&v| ValueWord::from_i64(v as i64)).collect()
        }
        ELEM_TYPE_BOOL => {
            let arr = p as *const TypedArray<u8>;
            let slice = unsafe { TypedArray::as_slice(arr) };
            slice.iter().map(|&v| ValueWord::from_bool(v != 0)).collect()
        }
        _ => return None,
    };
    Some(items)
}

/// Insert a key into a set (HashMap where all values are `true`).
/// Returns a new set with the key added.
fn set_insert(set: &ValueWord, item: &ValueWord) -> Result<ValueWord, String> {
    let data = set
        .as_hashmap_data()
        .ok_or_else(|| "set: expected a set (HashMap)".to_string())?;

    // Check if item already exists
    if data.find_key(item).is_some() {
        return Ok(set.clone());
    }

    let mut keys = data.keys.clone();
    let mut values = data.values.clone();
    keys.push(item.clone());
    values.push(ValueWord::from_bool(true));
    Ok(ValueWord::from_hashmap_pairs(keys, values))
}

/// Create the `set` module with set operations.
pub fn create_set_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::set");
    module.description = "Unordered collection of unique elements".to_string();

    // set.new() -> set
    module.add_function_with_schema(
        "new",
        |_args: &[ValueWord], _ctx: &ModuleContext| Ok(empty_set()),
        ModuleFunction {
            description: "Create a new empty set".to_string(),
            params: vec![],
            return_type: Some("HashMap".to_string()),
        },
    );

    // set.from_array(arr) -> set
    module.add_function_with_schema(
        "from_array",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let items = args
                .first()
                .and_then(materialize_array_items)
                .ok_or_else(|| "set.from_array() requires an array argument".to_string())?;

            let mut result = empty_set();
            for item in items.iter() {
                result = set_insert(&result, item)?;
            }
            Ok(result)
        },
        ModuleFunction {
            description: "Create a set from an array (deduplicates)".to_string(),
            params: vec![ModuleParam {
                name: "arr".to_string(),
                type_name: "Array".to_string(),
                required: true,
                description: "Array of items to add to the set".to_string(),
                ..Default::default()
            }],
            return_type: Some("HashMap".to_string()),
        },
    );

    // set.add(s, item) -> set
    module.add_function_with_schema(
        "add",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let s = args
                .first()
                .ok_or_else(|| "set.add() requires a set argument".to_string())?;
            let item = args
                .get(1)
                .ok_or_else(|| "set.add() requires an item argument".to_string())?;

            set_insert(s, item)
        },
        ModuleFunction {
            description: "Add an item to the set, returns new set".to_string(),
            params: vec![
                ModuleParam {
                    name: "s".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "The set".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "item".to_string(),
                    type_name: "any".to_string(),
                    required: true,
                    description: "Item to add".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("HashMap".to_string()),
        },
    );

    // set.remove(s, item) -> set
    module.add_function_with_schema(
        "remove",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let s = args
                .first()
                .ok_or_else(|| "set.remove() requires a set argument".to_string())?;
            let item = args
                .get(1)
                .ok_or_else(|| "set.remove() requires an item argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.remove(): expected a set (HashMap)".to_string())?;

            if let Some(idx) = data.find_key(item) {
                let mut keys = data.keys.clone();
                let mut values = data.values.clone();
                keys.remove(idx);
                values.remove(idx);
                Ok(ValueWord::from_hashmap_pairs(keys, values))
            } else {
                Ok(s.clone())
            }
        },
        ModuleFunction {
            description: "Remove an item from the set, returns new set".to_string(),
            params: vec![
                ModuleParam {
                    name: "s".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "The set".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "item".to_string(),
                    type_name: "any".to_string(),
                    required: true,
                    description: "Item to remove".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("HashMap".to_string()),
        },
    );

    // set.contains(s, item) -> bool
    module.add_function_with_schema(
        "contains",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let s = args
                .first()
                .ok_or_else(|| "set.contains() requires a set argument".to_string())?;
            let item = args
                .get(1)
                .ok_or_else(|| "set.contains() requires an item argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.contains(): expected a set (HashMap)".to_string())?;

            Ok(ValueWord::from_bool(data.find_key(item).is_some()))
        },
        ModuleFunction {
            description: "Check if set contains an item".to_string(),
            params: vec![
                ModuleParam {
                    name: "s".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "The set".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "item".to_string(),
                    type_name: "any".to_string(),
                    required: true,
                    description: "Item to check".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("bool".to_string()),
        },
    );

    // set.union(a, b) -> set
    module.add_function_with_schema(
        "union",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let a = args
                .first()
                .ok_or_else(|| "set.union() requires two set arguments".to_string())?;
            let b = args
                .get(1)
                .ok_or_else(|| "set.union() requires two set arguments".to_string())?;

            let a_data = a
                .as_hashmap_data()
                .ok_or_else(|| "set.union(): first argument must be a set".to_string())?;
            let b_data = b
                .as_hashmap_data()
                .ok_or_else(|| "set.union(): second argument must be a set".to_string())?;

            let mut result = a.clone();
            for key in &b_data.keys {
                if a_data.find_key(key).is_none() {
                    result = set_insert(&result, key)?;
                }
            }
            Ok(result)
        },
        ModuleFunction {
            description: "Union of two sets".to_string(),
            params: vec![
                ModuleParam {
                    name: "a".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "First set".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "b".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "Second set".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("HashMap".to_string()),
        },
    );

    // set.intersection(a, b) -> set
    module.add_function_with_schema(
        "intersection",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let a = args
                .first()
                .ok_or_else(|| "set.intersection() requires two set arguments".to_string())?;
            let b = args
                .get(1)
                .ok_or_else(|| "set.intersection() requires two set arguments".to_string())?;

            let a_data = a
                .as_hashmap_data()
                .ok_or_else(|| "set.intersection(): first argument must be a set".to_string())?;
            let b_data = b
                .as_hashmap_data()
                .ok_or_else(|| "set.intersection(): second argument must be a set".to_string())?;

            let mut result = empty_set();
            for key in &a_data.keys {
                if b_data.find_key(key).is_some() {
                    result = set_insert(&result, key)?;
                }
            }
            Ok(result)
        },
        ModuleFunction {
            description: "Intersection of two sets".to_string(),
            params: vec![
                ModuleParam {
                    name: "a".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "First set".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "b".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "Second set".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("HashMap".to_string()),
        },
    );

    // set.difference(a, b) -> set
    module.add_function_with_schema(
        "difference",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let a = args
                .first()
                .ok_or_else(|| "set.difference() requires two set arguments".to_string())?;
            let b = args
                .get(1)
                .ok_or_else(|| "set.difference() requires two set arguments".to_string())?;

            let a_data = a
                .as_hashmap_data()
                .ok_or_else(|| "set.difference(): first argument must be a set".to_string())?;
            let b_data = b
                .as_hashmap_data()
                .ok_or_else(|| "set.difference(): second argument must be a set".to_string())?;

            let mut result = empty_set();
            for key in &a_data.keys {
                if b_data.find_key(key).is_none() {
                    result = set_insert(&result, key)?;
                }
            }
            Ok(result)
        },
        ModuleFunction {
            description: "Difference (a - b)".to_string(),
            params: vec![
                ModuleParam {
                    name: "a".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "First set".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "b".to_string(),
                    type_name: "HashMap".to_string(),
                    required: true,
                    description: "Second set".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("HashMap".to_string()),
        },
    );

    // set.to_array(s) -> Array
    module.add_function_with_schema(
        "to_array",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let s = args
                .first()
                .ok_or_else(|| "set.to_array() requires a set argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.to_array(): expected a set (HashMap)".to_string())?;

            Ok(ValueWord::from_array(shape_value::vmarray_from_vec(
                data.keys.clone(),
            )))
        },
        ModuleFunction {
            description: "Convert set to array".to_string(),
            params: vec![ModuleParam {
                name: "s".to_string(),
                type_name: "HashMap".to_string(),
                required: true,
                description: "The set".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array".to_string()),
        },
    );

    // set.size(s) -> int
    module.add_function_with_schema(
        "size",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let s = args
                .first()
                .ok_or_else(|| "set.size() requires a set argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.size(): expected a set (HashMap)".to_string())?;

            Ok(ValueWord::from_i64(data.keys.len() as i64))
        },
        ModuleFunction {
            description: "Get the number of elements".to_string(),
            params: vec![ModuleParam {
                name: "s".to_string(),
                type_name: "HashMap".to_string(),
                required: true,
                description: "The set".to_string(),
                ..Default::default()
            }],
            return_type: Some("int".to_string()),
        },
    );

    module
}
