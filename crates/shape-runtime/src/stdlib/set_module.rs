//! Native `set` module for unordered collections of unique elements.
//!
//! Backed by HashMap for O(1) lookup.
//! Exports: set.new(), set.from_array(arr), set.add(s, item), set.remove(s, item),
//!          set.contains(s, item), set.union(a, b), set.intersection(a, b),
//!          set.difference(a, b), set.to_array(s), set.size(s)

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
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

/// Helper: build the standard "set as ModuleParam" descriptor for `s`/`a`/`b`.
fn set_param(name: &str, description: &str) -> ModuleParam {
    ModuleParam {
        name: name.to_string(),
        type_name: "HashMap".to_string(),
        required: true,
        description: description.to_string(),
        ..Default::default()
    }
}

/// Helper: build a `ModuleParam` for `item: any`.
fn item_param(description: &str) -> ModuleParam {
    ModuleParam {
        name: "item".to_string(),
        type_name: "any".to_string(),
        required: true,
        description: description.to_string(),
        ..Default::default()
    }
}

/// Create the `set` module with set operations.
///
/// Phase 4b: all 10 exports migrated to `TypedModuleExports`. Sets are
/// HashMap-shaped; element types pass through user-provided values, so
/// the return uses `TypedReturn::ValueWord` with `ConcreteType::HashMap`
/// (8 of 10 exports). `contains` returns Bool, `size` returns Int,
/// `to_array` returns ArrayValueWord (set elements as-is).
pub fn create_set_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::set");
    module.description = "Unordered collection of unique elements".to_string();

    // set.new() -> set
    register_typed_function(
        &mut module,
        "new",
        "Create a new empty set",
        vec![],
        ConcreteType::HashMap,
        |_args, _ctx| Ok(TypedReturn::ValueWord(empty_set())),
    );

    // set.from_array(arr) -> set
    register_typed_function(
        &mut module,
        "from_array",
        "Create a set from an array (deduplicates)",
        vec![ModuleParam {
            name: "arr".to_string(),
            type_name: "Array".to_string(),
            required: true,
            description: "Array of items to add to the set".to_string(),
            ..Default::default()
        }],
        ConcreteType::HashMap,
        |args, _ctx| {
            let items = args
                .first()
                .and_then(materialize_array_items)
                .ok_or_else(|| "set.from_array() requires an array argument".to_string())?;

            let mut result = empty_set();
            for item in items.iter() {
                result = set_insert(&result, item)?;
            }
            Ok(TypedReturn::ValueWord(result))
        },
    );

    // set.add(s, item) -> set
    register_typed_function(
        &mut module,
        "add",
        "Add an item to the set, returns new set",
        vec![set_param("s", "The set"), item_param("Item to add")],
        ConcreteType::HashMap,
        |args, _ctx| {
            let s = args
                .first()
                .ok_or_else(|| "set.add() requires a set argument".to_string())?;
            let item = args
                .get(1)
                .ok_or_else(|| "set.add() requires an item argument".to_string())?;
            Ok(TypedReturn::ValueWord(set_insert(s, item)?))
        },
    );

    // set.remove(s, item) -> set
    register_typed_function(
        &mut module,
        "remove",
        "Remove an item from the set, returns new set",
        vec![set_param("s", "The set"), item_param("Item to remove")],
        ConcreteType::HashMap,
        |args, _ctx| {
            let s = args
                .first()
                .ok_or_else(|| "set.remove() requires a set argument".to_string())?;
            let item = args
                .get(1)
                .ok_or_else(|| "set.remove() requires an item argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.remove(): expected a set (HashMap)".to_string())?;

            let result = if let Some(idx) = data.find_key(item) {
                let mut keys = data.keys.clone();
                let mut values = data.values.clone();
                keys.remove(idx);
                values.remove(idx);
                ValueWord::from_hashmap_pairs(keys, values)
            } else {
                s.clone()
            };
            Ok(TypedReturn::ValueWord(result))
        },
    );

    // set.contains(s, item) -> bool
    register_typed_function(
        &mut module,
        "contains",
        "Check if set contains an item",
        vec![set_param("s", "The set"), item_param("Item to check")],
        ConcreteType::Bool,
        |args, _ctx| {
            let s = args
                .first()
                .ok_or_else(|| "set.contains() requires a set argument".to_string())?;
            let item = args
                .get(1)
                .ok_or_else(|| "set.contains() requires an item argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.contains(): expected a set (HashMap)".to_string())?;

            Ok(TypedReturn::Bool(data.find_key(item).is_some()))
        },
    );

    // set.union(a, b) -> set
    register_typed_function(
        &mut module,
        "union",
        "Union of two sets",
        vec![set_param("a", "First set"), set_param("b", "Second set")],
        ConcreteType::HashMap,
        |args, _ctx| {
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
            Ok(TypedReturn::ValueWord(result))
        },
    );

    // set.intersection(a, b) -> set
    register_typed_function(
        &mut module,
        "intersection",
        "Intersection of two sets",
        vec![set_param("a", "First set"), set_param("b", "Second set")],
        ConcreteType::HashMap,
        |args, _ctx| {
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
            Ok(TypedReturn::ValueWord(result))
        },
    );

    // set.difference(a, b) -> set
    register_typed_function(
        &mut module,
        "difference",
        "Difference (a - b)",
        vec![set_param("a", "First set"), set_param("b", "Second set")],
        ConcreteType::HashMap,
        |args, _ctx| {
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
            Ok(TypedReturn::ValueWord(result))
        },
    );

    // set.to_array(s) -> Array
    register_typed_function(
        &mut module,
        "to_array",
        "Convert set to array",
        vec![set_param("s", "The set")],
        ConcreteType::Array,
        |args, _ctx| {
            let s = args
                .first()
                .ok_or_else(|| "set.to_array() requires a set argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.to_array(): expected a set (HashMap)".to_string())?;

            Ok(TypedReturn::ArrayValueWord(data.keys.clone()))
        },
    );

    // set.size(s) -> int
    register_typed_function(
        &mut module,
        "size",
        "Get the number of elements",
        vec![set_param("s", "The set")],
        ConcreteType::Int,
        |args, _ctx| {
            let s = args
                .first()
                .ok_or_else(|| "set.size() requires a set argument".to_string())?;

            let data = s
                .as_hashmap_data()
                .ok_or_else(|| "set.size(): expected a set (HashMap)".to_string())?;

            Ok(TypedReturn::I64(data.keys.len() as i64))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
    fn test_set_module_creation() {
        let module = create_set_module();
        assert_eq!(module.name, "std::core::set");
        for export in [
            "new",
            "from_array",
            "add",
            "remove",
            "contains",
            "union",
            "intersection",
            "difference",
            "to_array",
            "size",
        ] {
            assert!(module.has_export(export), "missing {}", export);
            assert!(
                module.typed_exports().get(export).is_some(),
                "{} should be in typed registry",
                export
            );
        }
        assert_eq!(module.typed_exports().functions.len(), 10);
    }

    #[test]
    fn test_set_add_contains_size() {
        let module = create_set_module();
        let ctx = test_ctx();

        let s = module.invoke_export("new", &[], &ctx).unwrap().unwrap();
        let a = ValueWord::from_string(Arc::new("a".to_string()));
        let s = module.invoke_export("add", &[s, a.clone()], &ctx).unwrap().unwrap();
        let s = module.invoke_export("add", &[s.clone(), a.clone()], &ctx).unwrap().unwrap(); // dup
        assert_eq!(
            module.invoke_export("contains", &[s.clone(), a.clone()], &ctx).unwrap()
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(module.invoke_export("size", &[s], &ctx).unwrap().unwrap().as_i64(), Some(1));
    }

    #[test]
    fn test_set_to_array_and_remove() {
        let module = create_set_module();
        let ctx = test_ctx();

        let s = module.invoke_export("new", &[], &ctx).unwrap().unwrap();
        let s = module.invoke_export("add", &[s, ValueWord::from_i64(1)], &ctx).unwrap().unwrap();
        let s = module.invoke_export("add", &[s, ValueWord::from_i64(2)], &ctx).unwrap().unwrap();
        let arr = module.invoke_export("to_array", &[s.clone()], &ctx).unwrap().unwrap();
        assert_eq!(arr.as_any_array().unwrap().to_generic().len(), 2);

        let s = module.invoke_export("remove", &[s, ValueWord::from_i64(1)], &ctx).unwrap().unwrap();
        assert_eq!(module.invoke_export("size", &[s], &ctx).unwrap().unwrap().as_i64(), Some(1));
    }

    #[test]
    fn test_set_union_intersection_difference() {
        let module = create_set_module();
        let ctx = test_ctx();

        let mut a = module.invoke_export("new", &[], &ctx).unwrap().unwrap();
        a = module.invoke_export("add", &[a, ValueWord::from_i64(1)], &ctx).unwrap().unwrap();
        a = module.invoke_export("add", &[a, ValueWord::from_i64(2)], &ctx).unwrap().unwrap();
        let mut b = module.invoke_export("new", &[], &ctx).unwrap().unwrap();
        b = module.invoke_export("add", &[b, ValueWord::from_i64(2)], &ctx).unwrap().unwrap();
        b = module.invoke_export("add", &[b, ValueWord::from_i64(3)], &ctx).unwrap().unwrap();

        let u = module.invoke_export("union", &[a.clone(), b.clone()], &ctx).unwrap().unwrap();
        assert_eq!(module.invoke_export("size", &[u], &ctx).unwrap().unwrap().as_i64(), Some(3));

        let i = module.invoke_export("intersection", &[a.clone(), b.clone()], &ctx).unwrap().unwrap();
        assert_eq!(module.invoke_export("size", &[i], &ctx).unwrap().unwrap().as_i64(), Some(1));

        let d = module.invoke_export("difference", &[a, b], &ctx).unwrap().unwrap();
        assert_eq!(module.invoke_export("size", &[d], &ctx).unwrap().unwrap().as_i64(), Some(1));
    }
}
