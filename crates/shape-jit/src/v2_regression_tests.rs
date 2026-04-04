//! v2 Runtime Regression Tests
//!
//! End-to-end tests that exercise v2 runtime components together:
//! TypedArray, StructLayout, HeapHeader, v2 Math FFI, v2 String FFI, v2 Struct FFI.
//!
//! These tests verify the correctness of the v2 native-typed runtime before
//! it is wired into the JIT compilation pipeline.

#[cfg(test)]
mod tests {
    use shape_value::heap_header::{HeapHeader, FLAG_MARKED, FLAG_PINNED};
    use shape_value::heap_value::HeapKind;
    use shape_value::v2_struct_layout::{
        V2FieldType, V2_STRUCT_HEADER_SIZE, compute_struct_layout, v2_struct_alloc,
        v2_struct_free, v2_struct_get_bool, v2_struct_get_f64, v2_struct_get_i32,
        v2_struct_get_ptr, v2_struct_refcount, v2_struct_release, v2_struct_retain,
        v2_struct_set_bool, v2_struct_set_f64, v2_struct_set_i32, v2_struct_set_ptr,
    };
    use shape_value::v2_typed_array::{
        TypedArrayHeader, V2ElemType, v2_typed_array_get_bool, v2_typed_array_get_f64,
        v2_typed_array_get_i32, v2_typed_array_get_i64, v2_typed_array_push_bool,
        v2_typed_array_push_f64, v2_typed_array_push_i32, v2_typed_array_push_i64,
        v2_typed_array_set_f64, v2_typed_array_set_i32,
    };

    use crate::ffi::v2_math;
    use crate::ffi::v2_string_ffi::{
        V2StringHeader, jit_v2_string_concat, jit_v2_string_eq, jit_v2_string_release,
    };

    // ========================================================================
    // 1. TypedArray lifecycle test: f64
    // ========================================================================

    #[test]
    fn test_v2_typed_array_f64_lifecycle() {
        // Alloc f64 array with initial capacity 16
        let mut ptr = TypedArrayHeader::alloc(V2ElemType::F64, 16);
        assert!(!ptr.is_null());

        // Push 100 elements
        for i in 0..100u32 {
            ptr = unsafe { v2_typed_array_push_f64(ptr, i as f64 * 1.5) };
            assert!(!ptr.is_null(), "push_f64 failed at index {}", i);
        }

        // Verify length
        assert_eq!(unsafe { (*ptr).len }, 100);

        // Read all 100 and verify
        for i in 0..100u32 {
            let val = unsafe { v2_typed_array_get_f64(ptr, i) };
            assert_eq!(val, i as f64 * 1.5, "mismatch at index {}", i);
        }

        // Out-of-bounds get returns NaN
        let oob = unsafe { v2_typed_array_get_f64(ptr, 100) };
        assert!(oob.is_nan());

        // Release
        assert!(unsafe { (*ptr).release() });
        unsafe { TypedArrayHeader::free(ptr) };
    }

    // ========================================================================
    // 2. TypedArray i32 test
    // ========================================================================

    #[test]
    fn test_v2_typed_array_i32_push_get_set() {
        let mut ptr = TypedArrayHeader::alloc(V2ElemType::I32, 4);
        assert!(!ptr.is_null());

        // Push 5 elements
        for i in 0..5i32 {
            ptr = unsafe { v2_typed_array_push_i32(ptr, i * 10) };
            assert!(!ptr.is_null());
        }

        assert_eq!(unsafe { (*ptr).len }, 5);

        // Verify element-level correctness
        for i in 0..5u32 {
            let val = unsafe { v2_typed_array_get_i32(ptr, i) };
            assert_eq!(val, i as i32 * 10);
        }

        // Set element at index 2 to -999
        unsafe { v2_typed_array_set_i32(ptr, 2, -999) };
        assert_eq!(unsafe { v2_typed_array_get_i32(ptr, 2) }, -999);

        // Surrounding elements are unchanged
        assert_eq!(unsafe { v2_typed_array_get_i32(ptr, 1) }, 10);
        assert_eq!(unsafe { v2_typed_array_get_i32(ptr, 3) }, 30);

        // Out-of-bounds get returns 0
        assert_eq!(unsafe { v2_typed_array_get_i32(ptr, 100) }, 0);

        // Cleanup
        assert!(unsafe { (*ptr).release() });
        unsafe { TypedArrayHeader::free(ptr) };
    }

    // ========================================================================
    // 3. Struct alloc + field access (f64 at offset 8)
    // ========================================================================

    #[test]
    fn test_v2_struct_alloc_and_f64_field() {
        // Layout: header(8) + f64(8) + f64(8) = 24 bytes
        let layout = compute_struct_layout(&[V2FieldType::F64, V2FieldType::F64]);
        assert_eq!(layout.offsets, vec![8, 16]);
        assert_eq!(layout.total_size, 24);

        let ptr = v2_struct_alloc(&layout);
        assert!(!ptr.is_null());

        // Write f64 at offset 8, read it back
        unsafe { v2_struct_set_f64(ptr, 8, 3.14159) };
        let val = unsafe { v2_struct_get_f64(ptr, 8) };
        assert_eq!(val, 3.14159);

        // Write f64 at offset 16
        unsafe { v2_struct_set_f64(ptr, 16, 2.71828) };
        assert_eq!(unsafe { v2_struct_get_f64(ptr, 16) }, 2.71828);

        // First field unchanged
        assert_eq!(unsafe { v2_struct_get_f64(ptr, 8) }, 3.14159);

        unsafe { v2_struct_free(ptr, &layout) };
    }

    // ========================================================================
    // 4. Struct mixed fields: f64 + i32 + bool
    // ========================================================================

    #[test]
    fn test_v2_struct_mixed_fields() {
        let layout = compute_struct_layout(&[V2FieldType::F64, V2FieldType::I32, V2FieldType::Bool]);
        let ptr = v2_struct_alloc(&layout);
        assert!(!ptr.is_null());

        let f64_offset = layout.offsets[0];
        let i32_offset = layout.offsets[1];
        let bool_offset = layout.offsets[2];

        // Set all fields
        unsafe {
            v2_struct_set_f64(ptr, f64_offset, 42.5);
            v2_struct_set_i32(ptr, i32_offset, -7);
            v2_struct_set_bool(ptr, bool_offset, true);
        }

        // Read all fields back
        assert_eq!(unsafe { v2_struct_get_f64(ptr, f64_offset) }, 42.5);
        assert_eq!(unsafe { v2_struct_get_i32(ptr, i32_offset) }, -7);
        assert!(unsafe { v2_struct_get_bool(ptr, bool_offset) });

        // Toggle bool
        unsafe { v2_struct_set_bool(ptr, bool_offset, false) };
        assert!(!unsafe { v2_struct_get_bool(ptr, bool_offset) });

        // Other fields unchanged
        assert_eq!(unsafe { v2_struct_get_f64(ptr, f64_offset) }, 42.5);
        assert_eq!(unsafe { v2_struct_get_i32(ptr, i32_offset) }, -7);

        unsafe { v2_struct_free(ptr, &layout) };
    }

    // ========================================================================
    // 5. HeapHeader refcount test
    // ========================================================================

    #[test]
    fn test_heap_header_refcount_transitions() {
        // HeapHeader uses a non-atomic refcount model (it's a plain struct).
        // For the v2 struct system we test the struct-level refcount.
        let layout = compute_struct_layout(&[V2FieldType::F64]);
        let ptr = v2_struct_alloc(&layout);
        assert!(!ptr.is_null());

        // Initial refcount is 1
        assert_eq!(unsafe { v2_struct_refcount(ptr) }, 1);

        // Retain 5 times
        for i in 0..5 {
            unsafe { v2_struct_retain(ptr) };
            assert_eq!(unsafe { v2_struct_refcount(ptr) }, 2 + i as u32);
        }
        assert_eq!(unsafe { v2_struct_refcount(ptr) }, 6);

        // Release 5 times -- should not free (refcount 6 -> 1)
        for _ in 0..5 {
            let freed = unsafe { v2_struct_release(ptr) };
            assert!(!freed, "should not be freed yet");
        }
        assert_eq!(unsafe { v2_struct_refcount(ptr) }, 1);

        // Final release frees
        let freed = unsafe { v2_struct_release(ptr) };
        assert!(freed, "should be freed on final release");

        // Do not access ptr after free -- it is dangling.
        // Instead just verify the freed flag was correct above.
        // We must still actually deallocate since v2_struct_release only
        // reports that it should be freed; it doesn't free itself.
        // (In production, the caller frees. Here we already got freed=true.)
        // Actually free the memory:
        unsafe { v2_struct_free(ptr, &layout) };
    }

    // ========================================================================
    // 6. String alloc + concat
    // ========================================================================

    #[test]
    fn test_v2_string_alloc_and_concat() {
        let hello = V2StringHeader::alloc("hello");
        assert!(!hello.is_null());
        assert_eq!(unsafe { (*hello).len }, 5);
        assert_eq!(unsafe { (*hello).as_str() }, "hello");

        let world = V2StringHeader::alloc(" world");
        assert!(!world.is_null());
        assert_eq!(unsafe { (*world).as_str() }, " world");

        // Concat
        let result = jit_v2_string_concat(hello, world);
        assert!(!result.is_null());
        assert_eq!(unsafe { (*result).as_str() }, "hello world");
        assert_eq!(unsafe { (*result).len }, 11);

        // Cleanup
        jit_v2_string_release(hello);
        jit_v2_string_release(world);
        jit_v2_string_release(result);
    }

    // ========================================================================
    // 7. String equality
    // ========================================================================

    #[test]
    fn test_v2_string_equality() {
        let a = V2StringHeader::alloc("shape");
        let b = V2StringHeader::alloc("shape");
        let c = V2StringHeader::alloc("other");

        // Same content => equal
        assert!(jit_v2_string_eq(a, b));

        // Different content => not equal
        assert!(!jit_v2_string_eq(a, c));

        // Self-equality
        assert!(jit_v2_string_eq(a, a));

        // Null cases
        assert!(jit_v2_string_eq(std::ptr::null(), std::ptr::null()));
        assert!(!jit_v2_string_eq(a, std::ptr::null()));
        assert!(!jit_v2_string_eq(std::ptr::null(), a));

        jit_v2_string_release(a);
        jit_v2_string_release(b);
        jit_v2_string_release(c);
    }

    // ========================================================================
    // 8. Math FFI: add_f64, mul_i64, div_f64
    // ========================================================================

    #[test]
    fn test_v2_math_ffi_basic() {
        // f64 addition
        assert_eq!(v2_math::jit_add_f64(1.5, 2.5), 4.0);
        assert_eq!(v2_math::jit_add_f64(-1.0, 1.0), 0.0);

        // i64 multiplication
        assert_eq!(v2_math::jit_mul_i64(6, 7), 42);
        assert_eq!(v2_math::jit_mul_i64(-3, 4), -12);

        // f64 division
        assert_eq!(v2_math::jit_div_f64(10.0, 2.0), 5.0);
        assert_eq!(v2_math::jit_div_f64(7.0, 2.0), 3.5);

        // f64 subtraction
        assert_eq!(v2_math::jit_sub_f64(10.0, 3.0), 7.0);

        // i64 addition
        assert_eq!(v2_math::jit_add_i64(100, 200), 300);

        // f64 multiplication
        assert_eq!(v2_math::jit_mul_f64(3.0, 4.0), 12.0);

        // Comparison
        assert!(v2_math::jit_lt_f64(1.0, 2.0));
        assert!(!v2_math::jit_lt_f64(2.0, 1.0));
        assert!(v2_math::jit_eq_i64(42, 42));
        assert!(!v2_math::jit_eq_i64(42, 43));
    }

    // ========================================================================
    // 9. Math edge cases: NaN, overflow, div-by-zero
    // ========================================================================

    #[test]
    fn test_v2_math_edge_cases() {
        // NaN propagation
        let nan_result = v2_math::jit_add_f64(f64::NAN, 1.0);
        assert!(nan_result.is_nan());

        let nan_mul = v2_math::jit_mul_f64(f64::NAN, 2.0);
        assert!(nan_mul.is_nan());

        // NaN != NaN
        assert!(!v2_math::jit_eq_f64(f64::NAN, f64::NAN));

        // NaN comparisons always false
        assert!(!v2_math::jit_lt_f64(f64::NAN, 1.0));
        assert!(!v2_math::jit_lt_f64(1.0, f64::NAN));
        assert!(!v2_math::jit_le_f64(f64::NAN, f64::NAN));

        // i64 overflow wrapping
        let overflow = v2_math::jit_add_i64(i64::MAX, 1);
        assert_eq!(overflow, i64::MIN); // wraps around

        let underflow = v2_math::jit_sub_i64(i64::MIN, 1);
        assert_eq!(underflow, i64::MAX); // wraps around

        let mul_overflow = v2_math::jit_mul_i64(i64::MAX, 2);
        assert_eq!(mul_overflow, -2); // wrapping_mul

        // f64 division by zero => infinity (IEEE 754)
        let div_zero = v2_math::jit_div_f64(1.0, 0.0);
        assert!(div_zero.is_infinite());
        assert!(div_zero.is_sign_positive());

        let div_neg_zero = v2_math::jit_div_f64(-1.0, 0.0);
        assert!(div_neg_zero.is_infinite());
        assert!(div_neg_zero.is_sign_negative());

        // 0.0 / 0.0 => NaN
        let zero_div_zero = v2_math::jit_div_f64(0.0, 0.0);
        assert!(zero_div_zero.is_nan());

        // i64 division by zero => 0 (safe)
        assert_eq!(v2_math::jit_div_i64(42, 0), 0);

        // i64 MIN / -1 overflow => 0 (safe)
        assert_eq!(v2_math::jit_div_i64(i64::MIN, -1), 0);

        // i64 remainder by zero => 0 (safe)
        assert_eq!(v2_math::jit_rem_i64(42, 0), 0);
    }

    // ========================================================================
    // 10. Struct layout: Point{x:f64, y:f64}
    // ========================================================================

    #[test]
    fn test_v2_struct_layout_point() {
        let layout = compute_struct_layout(&[V2FieldType::F64, V2FieldType::F64]);

        // Header is 8 bytes, first f64 at offset 8, second at offset 16
        assert_eq!(layout.offsets, vec![8, 16]);
        // Total = 24 (header 8 + 2*f64 = 8+16 = 24)
        assert_eq!(layout.total_size, 24);
        assert_eq!(layout.alignment, 8);
    }

    // ========================================================================
    // 11. Mixed-type struct layout: bool + f64 + i32
    // ========================================================================

    #[test]
    fn test_v2_struct_layout_mixed_alignment() {
        // Fields: bool (1), f64 (8), i32 (4)
        // Layout after 8-byte header:
        //   offset 8: bool (1 byte)
        //   offset 9-15: padding to align f64
        //   offset 16: f64 (8 bytes)
        //   offset 24: i32 (4 bytes)
        //   total = 28, padded to 32 (multiple of 8)
        let layout = compute_struct_layout(&[V2FieldType::Bool, V2FieldType::F64, V2FieldType::I32]);

        assert_eq!(layout.offsets[0], 8);  // bool at 8
        assert_eq!(layout.offsets[1], 16); // f64 at 16 (aligned to 8)
        assert_eq!(layout.offsets[2], 24); // i32 at 24 (aligned to 4)
        assert_eq!(layout.total_size, 32); // 28 padded to 32
        assert_eq!(layout.alignment, 8);

        // Verify the layout is valid by allocating and using it
        let ptr = v2_struct_alloc(&layout);
        assert!(!ptr.is_null());
        unsafe {
            v2_struct_set_bool(ptr, layout.offsets[0], true);
            v2_struct_set_f64(ptr, layout.offsets[1], 99.9);
            v2_struct_set_i32(ptr, layout.offsets[2], -42);

            assert!(v2_struct_get_bool(ptr, layout.offsets[0]));
            assert_eq!(v2_struct_get_f64(ptr, layout.offsets[1]), 99.9);
            assert_eq!(v2_struct_get_i32(ptr, layout.offsets[2]), -42);

            v2_struct_free(ptr, &layout);
        }
    }

    // ========================================================================
    // 12. TypedArray growth: push beyond initial capacity
    // ========================================================================

    #[test]
    fn test_v2_typed_array_growth() {
        // Start with capacity 2
        let mut ptr = TypedArrayHeader::alloc(V2ElemType::F64, 2);
        assert!(!ptr.is_null());
        assert_eq!(unsafe { (*ptr).cap }, 2);

        // Push 50 elements (forces multiple reallocations)
        for i in 0..50u32 {
            ptr = unsafe { v2_typed_array_push_f64(ptr, (i as f64) + 0.1) };
            assert!(!ptr.is_null(), "realloc failed at index {}", i);
        }

        assert_eq!(unsafe { (*ptr).len }, 50);
        // Capacity should have grown beyond 2
        assert!(unsafe { (*ptr).cap } >= 50);

        // All elements preserved after growth
        for i in 0..50u32 {
            let val = unsafe { v2_typed_array_get_f64(ptr, i) };
            let expected = (i as f64) + 0.1;
            assert!(
                (val - expected).abs() < 1e-10,
                "element {} mismatch: got {} expected {}",
                i,
                val,
                expected
            );
        }

        // i64 array growth
        let mut i64_ptr = TypedArrayHeader::alloc(V2ElemType::I64, 1);
        for i in 0..30i64 {
            i64_ptr = unsafe { v2_typed_array_push_i64(i64_ptr, i * 100) };
            assert!(!i64_ptr.is_null());
        }
        assert_eq!(unsafe { (*i64_ptr).len }, 30);
        for i in 0..30u32 {
            assert_eq!(unsafe { v2_typed_array_get_i64(i64_ptr, i) }, i as i64 * 100);
        }

        // Cleanup
        assert!(unsafe { (*ptr).release() });
        unsafe { TypedArrayHeader::free(ptr) };
        assert!(unsafe { (*i64_ptr).release() });
        unsafe { TypedArrayHeader::free(i64_ptr) };
    }

    // ========================================================================
    // 13. Cross-component: struct with TypedArray field pointer
    // ========================================================================

    #[test]
    fn test_v2_cross_component_struct_with_array() {
        // Create a struct with a pointer field (to hold a TypedArray)
        // Layout: header(8) + f64(8) + ptr(8) = 24
        let layout = compute_struct_layout(&[V2FieldType::F64, V2FieldType::Ptr]);
        assert_eq!(layout.offsets, vec![8, 16]);

        let struct_ptr = v2_struct_alloc(&layout);
        assert!(!struct_ptr.is_null());

        // Set the f64 field (a "length" or "id" field)
        unsafe { v2_struct_set_f64(struct_ptr, layout.offsets[0], 42.0) };

        // Create a TypedArray and store its pointer in the struct
        let mut arr_ptr = TypedArrayHeader::alloc(V2ElemType::F64, 4);
        assert!(!arr_ptr.is_null());

        // Push some elements
        arr_ptr = unsafe { v2_typed_array_push_f64(arr_ptr, 10.0) };
        arr_ptr = unsafe { v2_typed_array_push_f64(arr_ptr, 20.0) };
        arr_ptr = unsafe { v2_typed_array_push_f64(arr_ptr, 30.0) };

        // Store array pointer in struct's pointer field
        unsafe { v2_struct_set_ptr(struct_ptr, layout.offsets[1], arr_ptr as *const u8) };

        // Read the array pointer back from the struct
        let recovered_ptr =
            unsafe { v2_struct_get_ptr(struct_ptr, layout.offsets[1]) } as *mut TypedArrayHeader;
        assert_eq!(recovered_ptr, arr_ptr);

        // Access array elements through the recovered pointer
        assert_eq!(unsafe { (*recovered_ptr).len }, 3);
        assert_eq!(unsafe { v2_typed_array_get_f64(recovered_ptr, 0) }, 10.0);
        assert_eq!(unsafe { v2_typed_array_get_f64(recovered_ptr, 1) }, 20.0);
        assert_eq!(unsafe { v2_typed_array_get_f64(recovered_ptr, 2) }, 30.0);

        // Verify the f64 field is still correct
        assert_eq!(unsafe { v2_struct_get_f64(struct_ptr, layout.offsets[0]) }, 42.0);

        // Cleanup: free the array, then the struct
        assert!(unsafe { (*arr_ptr).release() });
        unsafe { TypedArrayHeader::free(arr_ptr) };
        unsafe { v2_struct_free(struct_ptr, &layout) };
    }

    // ========================================================================
    // Additional: TypedArray bool lifecycle
    // ========================================================================

    #[test]
    fn test_v2_typed_array_bool() {
        let mut ptr = TypedArrayHeader::alloc(V2ElemType::Bool, 4);
        assert!(!ptr.is_null());

        // Push alternating bools
        for i in 0..10u32 {
            ptr = unsafe { v2_typed_array_push_bool(ptr, i % 2 == 0) };
            assert!(!ptr.is_null());
        }

        assert_eq!(unsafe { (*ptr).len }, 10);

        for i in 0..10u32 {
            let val = unsafe { v2_typed_array_get_bool(ptr, i) };
            assert_eq!(val, i % 2 == 0, "bool mismatch at index {}", i);
        }

        // Out of bounds returns false
        assert!(!unsafe { v2_typed_array_get_bool(ptr, 100) });

        assert!(unsafe { (*ptr).release() });
        unsafe { TypedArrayHeader::free(ptr) };
    }

    // ========================================================================
    // Additional: TypedArray refcount with retain/release
    // ========================================================================

    #[test]
    fn test_v2_typed_array_refcount() {
        let ptr = TypedArrayHeader::alloc(V2ElemType::F64, 4);
        assert!(!ptr.is_null());
        assert_eq!(unsafe { (*ptr).ref_count() }, 1);

        // Retain
        unsafe { (*ptr).retain() };
        assert_eq!(unsafe { (*ptr).ref_count() }, 2);

        unsafe { (*ptr).retain() };
        assert_eq!(unsafe { (*ptr).ref_count() }, 3);

        // Release (3 -> 2): should not free
        assert!(!unsafe { (*ptr).release() });
        assert_eq!(unsafe { (*ptr).ref_count() }, 2);

        // Release (2 -> 1): should not free
        assert!(!unsafe { (*ptr).release() });
        assert_eq!(unsafe { (*ptr).ref_count() }, 1);

        // Release (1 -> 0): should free
        assert!(unsafe { (*ptr).release() });
        unsafe { TypedArrayHeader::free(ptr) };
    }

    // ========================================================================
    // Additional: HeapHeader flag operations and kind dispatch
    // ========================================================================

    #[test]
    fn test_heap_header_flags_and_kind() {
        let mut h = HeapHeader::new(HeapKind::Array);
        assert_eq!(h.heap_kind(), Some(HeapKind::Array));
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(!h.has_flag(FLAG_PINNED));

        h.set_flag(FLAG_MARKED);
        assert!(h.has_flag(FLAG_MARKED));

        h.set_flag(FLAG_PINNED);
        assert!(h.has_flag(FLAG_PINNED));
        assert!(h.has_flag(FLAG_MARKED));

        h.clear_flag(FLAG_MARKED);
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(h.has_flag(FLAG_PINNED));

        // HeapHeader with len and aux
        let h2 = HeapHeader::with_len_aux(HeapKind::TypedObject, 10, 0xCAFE);
        assert_eq!(h2.len, 10);
        assert_eq!(h2.aux, 0xCAFE);
        assert_eq!(h2.heap_kind(), Some(HeapKind::TypedObject));
    }

    // ========================================================================
    // Additional: Empty struct layout
    // ========================================================================

    #[test]
    fn test_v2_struct_layout_empty() {
        let layout = compute_struct_layout(&[]);
        assert_eq!(layout.offsets.len(), 0);
        assert_eq!(layout.total_size, V2_STRUCT_HEADER_SIZE);
        assert_eq!(layout.alignment, 8);
    }

    // ========================================================================
    // Additional: String with empty and unicode content
    // ========================================================================

    #[test]
    fn test_v2_string_empty_and_unicode() {
        // Empty string
        let empty = V2StringHeader::alloc("");
        assert!(!empty.is_null());
        assert_eq!(unsafe { (*empty).len }, 0);
        assert_eq!(unsafe { (*empty).as_str() }, "");

        // Unicode string
        let uni = V2StringHeader::alloc("hello \u{1F600} world");
        assert!(!uni.is_null());
        assert_eq!(unsafe { (*uni).as_str() }, "hello \u{1F600} world");

        // Concat empty + non-empty
        let result = jit_v2_string_concat(empty, uni);
        assert_eq!(unsafe { (*result).as_str() }, "hello \u{1F600} world");

        jit_v2_string_release(empty);
        jit_v2_string_release(uni);
        jit_v2_string_release(result);
    }

    // ========================================================================
    // Additional: f64 array set_f64 in-bounds mutation
    // ========================================================================

    #[test]
    fn test_v2_typed_array_f64_set() {
        let mut ptr = TypedArrayHeader::alloc(V2ElemType::F64, 8);
        for i in 0..5 {
            ptr = unsafe { v2_typed_array_push_f64(ptr, i as f64) };
        }

        // Overwrite element 2
        unsafe { v2_typed_array_set_f64(ptr, 2, 999.0) };
        assert_eq!(unsafe { v2_typed_array_get_f64(ptr, 2) }, 999.0);
        // Others unchanged
        assert_eq!(unsafe { v2_typed_array_get_f64(ptr, 0) }, 0.0);
        assert_eq!(unsafe { v2_typed_array_get_f64(ptr, 4) }, 4.0);

        // Out-of-bounds set is a no-op
        unsafe { v2_typed_array_set_f64(ptr, 100, 1.0) };

        assert!(unsafe { (*ptr).release() });
        unsafe { TypedArrayHeader::free(ptr) };
    }

    // ========================================================================
    // Additional: Struct layout with all pointer fields
    // ========================================================================

    #[test]
    fn test_v2_struct_layout_all_pointers() {
        let layout = compute_struct_layout(&[V2FieldType::Ptr, V2FieldType::Ptr, V2FieldType::Ptr]);
        assert_eq!(layout.offsets, vec![8, 16, 24]);
        assert_eq!(layout.total_size, 32);
        assert_eq!(layout.alignment, 8);
    }

    // ========================================================================
    // Additional: Math FFI remainder and negation
    // ========================================================================

    #[test]
    fn test_v2_math_rem_and_neg() {
        // f64 remainder
        assert_eq!(v2_math::jit_rem_f64(10.0, 3.0), 1.0);
        assert_eq!(v2_math::jit_rem_f64(7.5, 2.5), 0.0);

        // f64 negation
        assert_eq!(v2_math::jit_neg_f64(5.0), -5.0);
        assert_eq!(v2_math::jit_neg_f64(-3.0), 3.0);
        assert_eq!(v2_math::jit_neg_f64(0.0), -0.0);

        // i64 remainder
        assert_eq!(v2_math::jit_rem_i64(10, 3), 1);
        assert_eq!(v2_math::jit_rem_i64(-10, 3), -1);

        // i64 negation
        assert_eq!(v2_math::jit_neg_i64(5), -5);
        assert_eq!(v2_math::jit_neg_i64(i64::MIN), i64::MIN); // wrapping
    }
}
