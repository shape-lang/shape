//! Inline typed struct field access for the v2 runtime.
//!
//! Generates Cranelift IR for O(1) typed struct field reads and writes using
//! compile-time constant offsets.  No schema lookup, no HashMap, no FFI call.
//!
//! # v2 Struct Memory Layout
//!
//! ```text
//! offset 0..8   HeapHeader  (refcount u32 @ 0, kind u16 @ 4, flags u8 @ 6, _pad u8 @ 7)
//! offset 8..    fields in declaration order, naturally aligned
//! ```
//!
//! For `type Point { x: number, y: number }`:
//!
//! ```text
//! [0..8)   HeapHeader
//! [8..16)  x: f64
//! [16..24) y: f64
//! ```
//!
//! `point.x` compiles to a single `load f64 [ptr + 8]`.

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use shape_vm::type_tracking::SlotKind;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Size of the v2 HeapHeader in bytes.
///
/// Layout: refcount(u32) + kind(u16) + flags(u8) + _pad(u8) = 8 bytes.
pub const V2_HEAP_HEADER_SIZE: u32 = 8;

/// Byte offset of the refcount field (u32) within the HeapHeader.
pub const V2_HEADER_REFCOUNT_OFFSET: u32 = 0;

/// Byte offset of the kind field (u16) within the HeapHeader.
pub const V2_HEADER_KIND_OFFSET: u32 = 4;

/// Byte offset of the flags field (u8) within the HeapHeader.
pub const V2_HEADER_FLAGS_OFFSET: u32 = 6;

// ---------------------------------------------------------------------------
// SlotKind -> Cranelift type mapping
// ---------------------------------------------------------------------------

/// Map a `SlotKind` to the corresponding Cranelift IR type.
///
/// This determines the load/store width for a given field type.
pub fn cranelift_type_for_slot(kind: SlotKind) -> types::Type {
    match kind {
        SlotKind::Float64 | SlotKind::NullableFloat64 => types::F64,

        SlotKind::Int64 | SlotKind::NullableInt64 | SlotKind::UInt64 | SlotKind::NullableUInt64 => {
            types::I64
        }

        SlotKind::Int32 | SlotKind::NullableInt32 | SlotKind::UInt32 | SlotKind::NullableUInt32 => {
            types::I32
        }

        SlotKind::Int16 | SlotKind::NullableInt16 | SlotKind::UInt16 | SlotKind::NullableUInt16 => {
            types::I16
        }

        SlotKind::Int8
        | SlotKind::NullableInt8
        | SlotKind::UInt8
        | SlotKind::NullableUInt8
        | SlotKind::Bool => types::I8,

        SlotKind::IntSize
        | SlotKind::NullableIntSize
        | SlotKind::UIntSize
        | SlotKind::NullableUIntSize => types::I64, // pointer-width

        // Boxed/pointer-sized values
        SlotKind::String | SlotKind::Dynamic | SlotKind::Unknown => types::I64,
    }
}

/// Return the byte width of a `SlotKind` for layout computation.
pub fn slot_byte_width(kind: SlotKind) -> u32 {
    match kind {
        SlotKind::Float64 | SlotKind::NullableFloat64 => 8,
        SlotKind::Int64 | SlotKind::NullableInt64 | SlotKind::UInt64 | SlotKind::NullableUInt64 => {
            8
        }
        SlotKind::Int32 | SlotKind::NullableInt32 | SlotKind::UInt32 | SlotKind::NullableUInt32 => {
            4
        }
        SlotKind::Int16 | SlotKind::NullableInt16 | SlotKind::UInt16 | SlotKind::NullableUInt16 => {
            2
        }
        SlotKind::Int8
        | SlotKind::NullableInt8
        | SlotKind::UInt8
        | SlotKind::NullableUInt8
        | SlotKind::Bool => 1,
        SlotKind::IntSize
        | SlotKind::NullableIntSize
        | SlotKind::UIntSize
        | SlotKind::NullableUIntSize => 8,
        SlotKind::String | SlotKind::Dynamic | SlotKind::Unknown => 8,
    }
}

// ---------------------------------------------------------------------------
// Struct layout computation
// ---------------------------------------------------------------------------

/// A pre-computed field descriptor: name, byte offset from struct base, and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLayout {
    /// Field name (for diagnostics / debugging).
    pub name: String,
    /// Byte offset from the struct base pointer (includes HeapHeader).
    pub offset: u32,
    /// The slot kind that determines Cranelift load/store type.
    pub kind: SlotKind,
}

/// Compute the layout of a v2 typed struct given its fields in declaration order.
///
/// Fields are placed after the 8-byte HeapHeader with natural alignment:
/// each field is aligned to its own byte width (1, 2, 4, or 8).
///
/// Returns a vector of `FieldLayout` descriptors and the total struct size
/// (including any trailing padding to align the whole struct to 8 bytes).
pub fn compute_struct_layout(fields: &[(String, SlotKind)]) -> (Vec<FieldLayout>, u32) {
    let mut layouts = Vec::with_capacity(fields.len());
    let mut cursor = V2_HEAP_HEADER_SIZE; // start after header

    for (name, kind) in fields {
        let width = slot_byte_width(*kind);
        let align = width; // natural alignment = size for primitive types

        // Align cursor up to the field's natural alignment
        let misalign = cursor % align;
        if misalign != 0 {
            cursor += align - misalign;
        }

        layouts.push(FieldLayout {
            name: name.clone(),
            offset: cursor,
            kind: *kind,
        });

        cursor += width;
    }

    // Pad total size to 8-byte boundary for struct-of-structs arrays
    let misalign = cursor % 8;
    if misalign != 0 {
        cursor += 8 - misalign;
    }

    (layouts, cursor)
}

// ---------------------------------------------------------------------------
// MirToIR - v2 MIR-level Cranelift code generator
// ---------------------------------------------------------------------------

/// MIR-to-Cranelift-IR compiler for the v2 runtime.
///
/// Wraps a Cranelift `FunctionBuilder` and provides methods to emit inline
/// typed struct operations (field get/set, allocation) without any FFI overhead.
pub struct MirToIR<'a, 'b: 'a> {
    /// The Cranelift function builder used to emit instructions.
    pub builder: &'a mut FunctionBuilder<'b>,
}

impl<'a, 'b: 'a> MirToIR<'a, 'b> {
    /// Create a new MirToIR code generator wrapping the given function builder.
    pub fn new(builder: &'a mut FunctionBuilder<'b>) -> Self {
        Self { builder }
    }

    // -----------------------------------------------------------------------
    // Typed field access
    // -----------------------------------------------------------------------

    /// Emit an inline typed field read: `load T [struct_ptr + field_offset]`.
    ///
    /// `field_offset` is the absolute byte offset from the struct base pointer
    /// (including the HeapHeader).  `field_type` determines the Cranelift load
    /// width (F64, I64, I32, I16, I8).
    ///
    /// This compiles to a single Cranelift `load` instruction with a constant
    /// offset -- no schema lookup, no FFI call.
    pub fn v2_field_get(
        &mut self,
        struct_ptr: Value,
        field_offset: u32,
        field_type: SlotKind,
    ) -> Value {
        let cl_type = cranelift_type_for_slot(field_type);
        self.builder
            .ins()
            .load(cl_type, MemFlags::trusted(), struct_ptr, field_offset as i32)
    }

    /// Emit an inline typed field write: `store T val -> [struct_ptr + field_offset]`.
    ///
    /// `field_offset` is the absolute byte offset from the struct base pointer
    /// (including the HeapHeader).  `field_type` is used for documentation /
    /// assertions; the actual store width comes from the Cranelift value type.
    ///
    /// This compiles to a single Cranelift `store` instruction.
    pub fn v2_field_set(
        &mut self,
        struct_ptr: Value,
        field_offset: u32,
        val: Value,
        _field_type: SlotKind,
    ) {
        self.builder
            .ins()
            .store(MemFlags::trusted(), val, struct_ptr, field_offset as i32);
    }

    // -----------------------------------------------------------------------
    // Struct allocation
    // -----------------------------------------------------------------------

    /// Emit a call to allocate a v2 typed struct with the given total byte size.
    ///
    /// The `alloc_fn` parameter is a Cranelift `FuncRef` for the runtime's
    /// allocation function with signature `(i64) -> i64` (byte size in, pointer
    /// out).  The allocator is expected to zero-initialize the memory.
    ///
    /// After allocation the caller should write the HeapHeader fields (kind,
    /// refcount) and each struct field using `v2_field_set`.
    pub fn v2_struct_alloc(&mut self, total_size: u32, alloc_fn: FuncRef) -> Value {
        let size_val = self.builder.ins().iconst(types::I64, total_size as i64);
        let inst = self.builder.ins().call(alloc_fn, &[size_val]);
        self.builder.inst_results(inst)[0]
    }

    /// Write the refcount field of a freshly-allocated v2 struct.
    ///
    /// Sets the `u32` at `V2_HEADER_REFCOUNT_OFFSET` to the given initial value
    /// (typically 1).
    pub fn v2_write_refcount(&mut self, struct_ptr: Value, initial: u32) {
        let rc = self.builder.ins().iconst(types::I32, initial as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            rc,
            struct_ptr,
            V2_HEADER_REFCOUNT_OFFSET as i32,
        );
    }

    /// Write the kind field of a freshly-allocated v2 struct.
    ///
    /// Sets the `u16` at `V2_HEADER_KIND_OFFSET`.
    pub fn v2_write_kind(&mut self, struct_ptr: Value, kind: u16) {
        let k = self.builder.ins().iconst(types::I16, kind as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            k,
            struct_ptr,
            V2_HEADER_KIND_OFFSET as i32,
        );
    }

    /// Write the flags field of a freshly-allocated v2 struct.
    ///
    /// Sets the `u8` at `V2_HEADER_FLAGS_OFFSET`.
    pub fn v2_write_flags(&mut self, struct_ptr: Value, flags: u8) {
        let f = self.builder.ins().iconst(types::I8, flags as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            f,
            struct_ptr,
            V2_HEADER_FLAGS_OFFSET as i32,
        );
    }
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::Module;

    // -----------------------------------------------------------------------
    // Layout computation tests (pure, no Cranelift needed)
    // -----------------------------------------------------------------------

    #[test]
    fn v2_field_layout_point_all_f64() {
        // type Point { x: number, y: number }
        let fields = vec![
            ("x".to_string(), SlotKind::Float64),
            ("y".to_string(), SlotKind::Float64),
        ];
        let (layout, total) = compute_struct_layout(&fields);

        assert_eq!(layout.len(), 2);
        assert_eq!(layout[0].name, "x");
        assert_eq!(layout[0].offset, 8); // right after 8-byte header
        assert_eq!(layout[0].kind, SlotKind::Float64);

        assert_eq!(layout[1].name, "y");
        assert_eq!(layout[1].offset, 16); // 8 (header) + 8 (x)
        assert_eq!(layout[1].kind, SlotKind::Float64);

        assert_eq!(total, 24); // 8 header + 8 x + 8 y
    }

    #[test]
    fn v2_field_layout_mixed_types() {
        // type Mixed { flag: bool, count: i32, value: number }
        let fields = vec![
            ("flag".to_string(), SlotKind::Bool),
            ("count".to_string(), SlotKind::Int32),
            ("value".to_string(), SlotKind::Float64),
        ];
        let (layout, total) = compute_struct_layout(&fields);

        assert_eq!(layout.len(), 3);

        // flag: i8, at offset 8 (right after header, 1-byte aligned)
        assert_eq!(layout[0].name, "flag");
        assert_eq!(layout[0].offset, 8);
        assert_eq!(layout[0].kind, SlotKind::Bool);

        // count: i32, needs 4-byte alignment. cursor was at 9, aligns to 12.
        assert_eq!(layout[1].name, "count");
        assert_eq!(layout[1].offset, 12);
        assert_eq!(layout[1].kind, SlotKind::Int32);

        // value: f64, needs 8-byte alignment. cursor was at 16, already aligned.
        assert_eq!(layout[2].name, "value");
        assert_eq!(layout[2].offset, 16);
        assert_eq!(layout[2].kind, SlotKind::Float64);

        // total = 16 + 8 = 24, already 8-byte aligned
        assert_eq!(total, 24);
    }

    #[test]
    fn v2_field_layout_i16_alignment() {
        // type Shorts { a: i16, b: i16, c: i64 }
        let fields = vec![
            ("a".to_string(), SlotKind::Int16),
            ("b".to_string(), SlotKind::Int16),
            ("c".to_string(), SlotKind::Int64),
        ];
        let (layout, total) = compute_struct_layout(&fields);

        // a: i16, at offset 8
        assert_eq!(layout[0].offset, 8);
        // b: i16, at offset 10 (2-byte aligned, cursor was 10)
        assert_eq!(layout[1].offset, 10);
        // c: i64, needs 8-byte alignment. cursor was 12, aligns to 16.
        assert_eq!(layout[2].offset, 16);

        // total = 16 + 8 = 24
        assert_eq!(total, 24);
    }

    #[test]
    fn v2_field_layout_single_bool() {
        // type Flag { active: bool }
        let fields = vec![("active".to_string(), SlotKind::Bool)];
        let (layout, total) = compute_struct_layout(&fields);

        assert_eq!(layout[0].offset, 8);
        // total: 8 (header) + 1 (bool) = 9, padded to 16? No: padded to 8-byte boundary = 16
        assert_eq!(total, 16);
    }

    #[test]
    fn v2_field_layout_empty_struct() {
        let fields: Vec<(String, SlotKind)> = vec![];
        let (layout, total) = compute_struct_layout(&fields);

        assert_eq!(layout.len(), 0);
        assert_eq!(total, 8); // just the header, already 8-byte aligned
    }

    #[test]
    fn v2_field_layout_padding_between_fields() {
        // type Padded { a: bool, b: i64 }
        // a is 1 byte at offset 8, then padding to align b at offset 16.
        let fields = vec![
            ("a".to_string(), SlotKind::Bool),
            ("b".to_string(), SlotKind::Int64),
        ];
        let (layout, total) = compute_struct_layout(&fields);

        assert_eq!(layout[0].offset, 8);
        assert_eq!(layout[1].offset, 16); // 8 + 1 -> pad to 16
        assert_eq!(total, 24); // 16 + 8
    }

    #[test]
    fn v2_field_layout_all_i32() {
        // type Vec3i { x: i32, y: i32, z: i32 }
        let fields = vec![
            ("x".to_string(), SlotKind::Int32),
            ("y".to_string(), SlotKind::Int32),
            ("z".to_string(), SlotKind::Int32),
        ];
        let (layout, total) = compute_struct_layout(&fields);

        assert_eq!(layout[0].offset, 8);
        assert_eq!(layout[1].offset, 12);
        assert_eq!(layout[2].offset, 16);
        // total = 16 + 4 = 20, padded to 24
        assert_eq!(total, 24);
    }

    // -----------------------------------------------------------------------
    // cranelift_type_for_slot mapping tests
    // -----------------------------------------------------------------------

    #[test]
    fn v2_field_slot_to_cranelift_type() {
        assert_eq!(cranelift_type_for_slot(SlotKind::Float64), types::F64);
        assert_eq!(cranelift_type_for_slot(SlotKind::NullableFloat64), types::F64);
        assert_eq!(cranelift_type_for_slot(SlotKind::Int64), types::I64);
        assert_eq!(cranelift_type_for_slot(SlotKind::Int32), types::I32);
        assert_eq!(cranelift_type_for_slot(SlotKind::Int16), types::I16);
        assert_eq!(cranelift_type_for_slot(SlotKind::Bool), types::I8);
        assert_eq!(cranelift_type_for_slot(SlotKind::Int8), types::I8);
        assert_eq!(cranelift_type_for_slot(SlotKind::Dynamic), types::I64);
        assert_eq!(cranelift_type_for_slot(SlotKind::String), types::I64);
    }

    // -----------------------------------------------------------------------
    // slot_byte_width tests
    // -----------------------------------------------------------------------

    #[test]
    fn v2_field_slot_byte_widths() {
        assert_eq!(slot_byte_width(SlotKind::Float64), 8);
        assert_eq!(slot_byte_width(SlotKind::Int64), 8);
        assert_eq!(slot_byte_width(SlotKind::Int32), 4);
        assert_eq!(slot_byte_width(SlotKind::Int16), 2);
        assert_eq!(slot_byte_width(SlotKind::Bool), 1);
        assert_eq!(slot_byte_width(SlotKind::Int8), 1);
        assert_eq!(slot_byte_width(SlotKind::String), 8);
    }

    // -----------------------------------------------------------------------
    // Cranelift codegen integration tests
    // -----------------------------------------------------------------------

    /// Build a minimal Cranelift JIT environment for testing MirToIR code gen.
    fn make_jit_env() -> (
        JITModule,
        cranelift::codegen::Context,
        FunctionBuilderContext,
    ) {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let module = JITModule::new(builder);
        let ctx = cranelift::codegen::Context::new();
        let fb_ctx = FunctionBuilderContext::new();
        (module, ctx, fb_ctx)
    }

    /// Helper: allocate a fake struct with HeapHeader + field data, return pointer.
    unsafe fn alloc_test_struct(total_bytes: usize) -> *mut u8 {
        let layout = std::alloc::Layout::from_size_align(total_bytes, 8).unwrap();
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        ptr
    }

    /// Helper: free a test struct allocated by `alloc_test_struct`.
    unsafe fn free_test_struct(ptr: *mut u8, total_bytes: usize) {
        let layout = std::alloc::Layout::from_size_align(total_bytes, 8).unwrap();
        unsafe { std::alloc::dealloc(ptr, layout) };
    }

    #[test]
    fn v2_field_get_f64_codegen_and_execute() {
        // Compile a function: fn(ptr: i64) -> f64 { load f64 [ptr + 8] }
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        let ptr_type = module.target_config().pointer_type();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.returns.push(AbiParam::new(types::F64));

        let func_id = module
            .declare_function("test_get_f64", cranelift_module::Linkage::Local, &sig)
            .unwrap();

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let struct_ptr = builder.block_params(entry)[0];

            let result = {
                let mut mir = MirToIR::new(&mut builder);
                mir.v2_field_get(struct_ptr, 8, SlotKind::Float64)
            };
            builder.ins().return_(&[result]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let struct_mem = alloc_test_struct(24);
            let field_ptr = struct_mem.add(8) as *mut f64;
            *field_ptr = 42.5;

            let func: unsafe fn(u64) -> f64 = std::mem::transmute(code_ptr);
            let result = func(struct_mem as u64);
            assert_eq!(result, 42.5);

            free_test_struct(struct_mem, 24);
        }
    }

    #[test]
    fn v2_field_set_f64_codegen_and_execute() {
        // Compile a function: fn(ptr: i64, val: f64) { store f64 val -> [ptr + 16] }
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        let ptr_type = module.target_config().pointer_type();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.params.push(AbiParam::new(types::F64));

        let func_id = module
            .declare_function("test_set_f64", cranelift_module::Linkage::Local, &sig)
            .unwrap();

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let struct_ptr = builder.block_params(entry)[0];
            let val = builder.block_params(entry)[1];

            {
                let mut mir = MirToIR::new(&mut builder);
                mir.v2_field_set(struct_ptr, 16, val, SlotKind::Float64);
            }
            builder.ins().return_(&[]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let struct_mem = alloc_test_struct(24);

            let func: unsafe fn(u64, f64) = std::mem::transmute(code_ptr);
            func(struct_mem as u64, 99.75);

            let stored = *(struct_mem.add(16) as *const f64);
            assert_eq!(stored, 99.75);

            free_test_struct(struct_mem, 24);
        }
    }

    #[test]
    fn v2_field_get_i32_codegen_and_execute() {
        // Compile a function: fn(ptr: i64) -> i32 { load i32 [ptr + 12] }
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        let ptr_type = module.target_config().pointer_type();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.returns.push(AbiParam::new(types::I32));

        let func_id = module
            .declare_function("test_get_i32", cranelift_module::Linkage::Local, &sig)
            .unwrap();

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let struct_ptr = builder.block_params(entry)[0];

            let result = {
                let mut mir = MirToIR::new(&mut builder);
                mir.v2_field_get(struct_ptr, 12, SlotKind::Int32)
            };
            builder.ins().return_(&[result]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let struct_mem = alloc_test_struct(16);
            let field_ptr = struct_mem.add(12) as *mut i32;
            *field_ptr = 0x7FFF_FFFE;

            let func: unsafe fn(u64) -> i32 = std::mem::transmute(code_ptr);
            let result = func(struct_mem as u64);
            assert_eq!(result, 0x7FFF_FFFE);

            free_test_struct(struct_mem, 16);
        }
    }

    #[test]
    fn v2_field_get_bool_codegen_and_execute() {
        // Compile a function: fn(ptr: i64) -> i8 { load i8 [ptr + 8] }
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        let ptr_type = module.target_config().pointer_type();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.returns.push(AbiParam::new(types::I8));

        let func_id = module
            .declare_function("test_get_bool", cranelift_module::Linkage::Local, &sig)
            .unwrap();

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let struct_ptr = builder.block_params(entry)[0];

            let result = {
                let mut mir = MirToIR::new(&mut builder);
                mir.v2_field_get(struct_ptr, 8, SlotKind::Bool)
            };
            builder.ins().return_(&[result]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let struct_mem = alloc_test_struct(16);
            *struct_mem.add(8) = 1u8;

            let func: unsafe fn(u64) -> i8 = std::mem::transmute(code_ptr);
            assert_eq!(func(struct_mem as u64), 1);

            *struct_mem.add(8) = 0u8;
            assert_eq!(func(struct_mem as u64), 0);

            free_test_struct(struct_mem, 16);
        }
    }

    #[test]
    fn v2_field_point_two_fields_codegen_and_execute() {
        // End-to-end: type Point { x: number, y: number }
        // Compile fn(ptr) -> f64 that returns x + y.
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        let ptr_type = module.target_config().pointer_type();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.returns.push(AbiParam::new(types::F64));

        let func_id = module
            .declare_function("test_point_sum", cranelift_module::Linkage::Local, &sig)
            .unwrap();

        let fields = vec![
            ("x".to_string(), SlotKind::Float64),
            ("y".to_string(), SlotKind::Float64),
        ];
        let (layout, total) = compute_struct_layout(&fields);
        assert_eq!(layout[0].offset, 8);
        assert_eq!(layout[1].offset, 16);

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let struct_ptr = builder.block_params(entry)[0];

            let (x, y) = {
                let mut mir = MirToIR::new(&mut builder);
                let x = mir.v2_field_get(struct_ptr, layout[0].offset, layout[0].kind);
                let y = mir.v2_field_get(struct_ptr, layout[1].offset, layout[1].kind);
                (x, y)
            };
            let sum = builder.ins().fadd(x, y);
            builder.ins().return_(&[sum]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let struct_mem = alloc_test_struct(total as usize);
            *(struct_mem.add(8) as *mut f64) = 3.0;
            *(struct_mem.add(16) as *mut f64) = 4.0;

            let func: unsafe fn(u64) -> f64 = std::mem::transmute(code_ptr);
            assert_eq!(func(struct_mem as u64), 7.0);

            free_test_struct(struct_mem, total as usize);
        }
    }

    #[test]
    fn v2_field_mixed_struct_codegen_and_execute() {
        // type Mixed { flag: bool, count: i32, value: number }
        // Compile fn(ptr) -> f64 -- reads the `value` field at its naturally
        // aligned offset to verify mixed-type layout correctness.
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        let ptr_type = module.target_config().pointer_type();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.returns.push(AbiParam::new(types::F64));

        let func_id = module
            .declare_function(
                "test_mixed_value",
                cranelift_module::Linkage::Local,
                &sig,
            )
            .unwrap();

        let fields = vec![
            ("flag".to_string(), SlotKind::Bool),
            ("count".to_string(), SlotKind::Int32),
            ("value".to_string(), SlotKind::Float64),
        ];
        let (layout, total) = compute_struct_layout(&fields);

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let struct_ptr = builder.block_params(entry)[0];

            let value = {
                let mut mir = MirToIR::new(&mut builder);
                mir.v2_field_get(struct_ptr, layout[2].offset, layout[2].kind)
            };
            builder.ins().return_(&[value]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let struct_mem = alloc_test_struct(total as usize);
            *struct_mem.add(layout[0].offset as usize) = 1u8;
            *(struct_mem.add(layout[1].offset as usize) as *mut i32) = 42;
            *(struct_mem.add(layout[2].offset as usize) as *mut f64) = 3.14;

            let func: unsafe fn(u64) -> f64 = std::mem::transmute(code_ptr);
            assert_eq!(func(struct_mem as u64), 3.14);

            free_test_struct(struct_mem, total as usize);
        }
    }

    #[test]
    fn v2_field_write_then_read_roundtrip() {
        // Compile two functions:
        //   writer(ptr, f64): stores val at offset 8
        //   reader(ptr) -> f64: loads from offset 8
        // Verify roundtrip.
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        let ptr_type = module.target_config().pointer_type();

        // Writer: (i64, f64) -> void
        let mut writer_sig = module.make_signature();
        writer_sig.params.push(AbiParam::new(ptr_type));
        writer_sig.params.push(AbiParam::new(types::F64));

        let writer_id = module
            .declare_function(
                "test_roundtrip_w",
                cranelift_module::Linkage::Local,
                &writer_sig,
            )
            .unwrap();

        ctx.func.signature = writer_sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let sp = builder.block_params(entry)[0];
            let val = builder.block_params(entry)[1];

            {
                let mut mir = MirToIR::new(&mut builder);
                mir.v2_field_set(sp, 8, val, SlotKind::Float64);
            }
            builder.ins().return_(&[]);
            builder.finalize();
        }
        module.define_function(writer_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);

        // Reader: (i64) -> f64
        let mut reader_sig = module.make_signature();
        reader_sig.params.push(AbiParam::new(ptr_type));
        reader_sig.returns.push(AbiParam::new(types::F64));

        let reader_id = module
            .declare_function(
                "test_roundtrip_r",
                cranelift_module::Linkage::Local,
                &reader_sig,
            )
            .unwrap();

        ctx.func.signature = reader_sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let sp = builder.block_params(entry)[0];

            let result = {
                let mut mir = MirToIR::new(&mut builder);
                mir.v2_field_get(sp, 8, SlotKind::Float64)
            };
            builder.ins().return_(&[result]);
            builder.finalize();
        }
        module.define_function(reader_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);

        module.finalize_definitions().unwrap();

        let writer_ptr = module.get_finalized_function(writer_id);
        let reader_ptr = module.get_finalized_function(reader_id);

        unsafe {
            let struct_mem = alloc_test_struct(16);

            let writer: unsafe fn(u64, f64) = std::mem::transmute(writer_ptr);
            let reader: unsafe fn(u64) -> f64 = std::mem::transmute(reader_ptr);

            writer(struct_mem as u64, -123.456);
            let got = reader(struct_mem as u64);
            assert_eq!(got, -123.456);

            writer(struct_mem as u64, f64::INFINITY);
            assert_eq!(reader(struct_mem as u64), f64::INFINITY);

            free_test_struct(struct_mem, 16);
        }
    }
}
