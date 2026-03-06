//! Shape guard emission for HashMap property access.
//!
//! When the JIT compiler encounters a `GetProp` on a HashMap with a known
//! shape (from profiling feedback), it emits a shape guard: a cheap u32
//! comparison of the HashMap's `shape_id` against the expected value.
//!
//! On guard success: O(1) indexed load (`values[slot_index]` direct access).
//! On guard failure: deopt (fall back to interpreter).
//!
//! Shape guards are the JIT equivalent of V8's "hidden class" guards. They
//! enable monomorphic property access to be compiled to a single array index
//! instead of a hash table lookup.

use cranelift::prelude::*;

use crate::nan_boxing::*;
use crate::translator::types::BytecodeToIR;

use shape_value::shape_graph::ShapeId;

/// Shape guard metadata recorded during compilation.
///
/// Used to register shape dependencies with the `DeoptTracker` so that
/// shape transitions can invalidate stale JIT code.
#[derive(Debug, Clone)]
pub struct ShapeGuardInfo {
    /// The shape ID that was guarded.
    pub shape_id: ShapeId,
    /// The property name hash used for the indexed load.
    pub property_hash: u32,
    /// The slot index within the shape's property layout.
    pub slot_index: usize,
    /// Bytecode IP where the guard was emitted.
    pub bytecode_ip: usize,
}

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Emit a shape-guarded HashMap property access.
    ///
    /// This generates:
    /// 1. Verify the value is a HashMap (heap kind check)
    /// 2. Call FFI to extract shape_id (u32)
    /// 3. Compare against expected shape (single u32 icmp)
    /// 4. On match: call FFI for O(1) indexed value access
    /// 5. On mismatch: branch to deopt block
    ///
    /// The caller must have already determined (from profiling feedback or
    /// static analysis) that this property access site is monomorphic with
    /// the given shape.
    ///
    /// # Arguments
    /// * `obj` - The NaN-boxed HashMap value (Cranelift i64)
    /// * `expected_shape` - The shape ID to guard against
    /// * `slot_index` - Pre-computed property slot within the shape
    ///
    /// # Returns
    /// The loaded property value (Cranelift i64, NaN-boxed)
    pub(crate) fn emit_shape_guarded_get(
        &mut self,
        obj: Value,
        expected_shape: ShapeId,
        slot_index: usize,
    ) -> Value {
        // Step 1: Verify the object is a HashMap
        let is_hashmap = self.emit_is_heap_kind(obj, HK_HASHMAP);
        self.deopt_if_false(is_hashmap);

        // Step 2: Extract shape_id via FFI
        // jit_hashmap_shape_id(obj_bits: u64) -> u32
        let inst = self.builder.ins().call(self.ffi.hashmap_shape_id, &[obj]);
        let actual_shape_id = self.builder.inst_results(inst)[0]; // i32

        // Step 3: Compare against expected shape
        let expected = self
            .builder
            .ins()
            .iconst(types::I32, expected_shape.0 as i64);
        let shape_matches = self
            .builder
            .ins()
            .icmp(IntCC::Equal, actual_shape_id, expected);
        self.deopt_if_false(shape_matches);

        // Step 4: Shape guard passed — load value at known slot index (O(1))
        // jit_hashmap_value_at(obj_bits: u64, slot_index: u64) -> u64
        let slot_val = self.builder.ins().iconst(types::I64, slot_index as i64);
        let inst = self
            .builder
            .ins()
            .call(self.ffi.hashmap_value_at, &[obj, slot_val]);
        let result = self.builder.inst_results(inst)[0]; // i64 (NaN-boxed)

        // Record this shape guard for dependency tracking
        self.shape_guards_emitted.push(expected_shape);

        result
    }

    /// Emit a shape-guarded HashMap property access with FFI fallback.
    ///
    /// Like `emit_shape_guarded_get`, but instead of deopt-ing on shape
    /// mismatch, falls back to the generic `get_prop` FFI call. This is
    /// useful for polymorphic sites where deopt would be too aggressive.
    ///
    /// # Arguments
    /// * `obj` - The NaN-boxed HashMap value (Cranelift i64)
    /// * `expected_shape` - The shape ID to guard against
    /// * `slot_index` - Pre-computed property slot within the shape
    /// * `key` - The property key (NaN-boxed string) for the fallback path
    ///
    /// # Returns
    /// The loaded property value (Cranelift i64, NaN-boxed)
    pub(crate) fn emit_shape_guarded_get_with_fallback(
        &mut self,
        obj: Value,
        expected_shape: ShapeId,
        slot_index: usize,
        key: Value,
    ) -> Value {
        // Check if HashMap
        let is_hashmap = self.emit_is_heap_kind(obj, HK_HASHMAP);

        let hashmap_block = self.builder.create_block();
        let fallback_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);

        self.builder
            .ins()
            .brif(is_hashmap, hashmap_block, &[], fallback_block, &[]);

        // HashMap path: check shape
        self.builder.switch_to_block(hashmap_block);
        self.builder.seal_block(hashmap_block);

        let inst = self.builder.ins().call(self.ffi.hashmap_shape_id, &[obj]);
        let actual_shape_id = self.builder.inst_results(inst)[0];

        let expected = self
            .builder
            .ins()
            .iconst(types::I32, expected_shape.0 as i64);
        let shape_matches = self
            .builder
            .ins()
            .icmp(IntCC::Equal, actual_shape_id, expected);

        let fast_block = self.builder.create_block();

        self.builder
            .ins()
            .brif(shape_matches, fast_block, &[], fallback_block, &[]);

        // Fast path: shape matches, do indexed access
        self.builder.switch_to_block(fast_block);
        self.builder.seal_block(fast_block);

        let slot_val = self.builder.ins().iconst(types::I64, slot_index as i64);
        let inst = self
            .builder
            .ins()
            .call(self.ffi.hashmap_value_at, &[obj, slot_val]);
        let fast_result = self.builder.inst_results(inst)[0];
        self.builder.ins().jump(merge_block, &[fast_result]);

        // Fallback: generic get_prop
        self.builder.switch_to_block(fallback_block);
        self.builder.seal_block(fallback_block);

        let inst = self.builder.ins().call(self.ffi.get_prop, &[obj, key]);
        let slow_result = self.builder.inst_results(inst)[0];
        self.builder.ins().jump(merge_block, &[slow_result]);

        // Merge
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);

        // Record shape dependency
        self.shape_guards_emitted.push(expected_shape);

        self.builder.block_params(merge_block)[0]
    }
}
