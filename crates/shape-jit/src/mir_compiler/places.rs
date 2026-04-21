//! Place resolution: MIR Place → Cranelift Value.
//!
//! A Place represents something that can be read from or written to:
//! - `Place::Local(slot)` → Cranelift variable
//! - `Place::Field(base, idx)` → **inline** typed struct access when byte offset is known, FFI fallback otherwise
//! - `Place::Index(base, operand)` → **inline** array access (no FFI call)

use cranelift::prelude::*;

use super::MirToIR;
// v2-boundary: inline array access still uses NaN-boxed heap pointer layout
use crate::ffi::jit_kinds::JIT_ALLOC_DATA_OFFSET;
use shape_value::tag_bits::UNIFIED_PTR_MASK;
use shape_vm::mir::types::*;

/// Byte offset of the `data` field within `UnifiedValue<T>` (kind u16 + flags u8 + _reserved u8 + refcount u32 = 8).
const UNIFIED_VALUE_DATA_OFFSET: i32 = 8;

/// Header size of a TypedObject in bytes (schema_id u32 + ref_count u32 = 8).
const TYPED_OBJ_HEADER: i32 = 8;

impl<'a, 'b> MirToIR<'a, 'b> {
    // ── Track A.1E: Shared capture lock fast path ─────────────────
    //
    // A `Shared` capture slot holds a raw `*const SharedCell`. The
    // SharedCell layout (pinned in `shape_value::v2::closure_layout`):
    //
    //   #[repr(C)]
    //   pub struct SharedCell {
    //       state: AtomicU8,          // offset 0 — 0=unlocked, 1=locked
    //       _pad:  [u8; 7],           // offsets 1..=7
    //       value: UnsafeCell<u64>,   // offset 8 — ValueWord payload
    //   }
    //
    // Size 16 bytes, align 8. The JIT's inline lock/unlock emits a
    // compare-exchange on the state byte at offset 0; on failure it
    // calls the slow-path FFI helpers (`jit_shared_lock_contended` /
    // `jit_shared_unlock_contended`). The value is read/written as
    // `i64` at offset 8.
    //
    // IR sketch for lock acquire:
    //
    //   old = atomic_cas.i8 [ptr + 0], 0, 1  (Acquire on success)
    //   brif (old != 0) → contended_block else → locked_block
    //   contended_block:
    //     call jit_shared_lock_contended(ptr); jump locked_block
    //   locked_block: ...
    //
    // Cranelift's `atomic_cas` returns the *previous* value. On success
    // the old byte was 0 (unlocked); on failure the old byte was 1 (or
    // whatever the contended holder left).

    /// Emit the inline lock fast path on a SharedCell pointer. On
    /// success, the state byte transitions from 0→1 with `Acquire`
    /// ordering; on failure, calls `jit_shared_lock_contended` which
    /// spin-waits. After this emission the current block is a
    /// freshly-switched block in which the lock is held.
    ///
    /// # SAFETY
    ///
    /// The caller must have validated that `cell_ptr` is a non-null
    /// `*const SharedCell` (equivalently, the closure's Ptr slot still
    /// holds the raw pointer bits installed by `emit_heap_closure` /
    /// `jit_arc_shared_retain`). The Arc strong share owned by the
    /// closure keeps the allocation alive for the duration of any
    /// JIT'd call that uses this slot, so the state byte read/write is
    /// always in-bounds.
    fn emit_shared_lock(&mut self, cell_ptr: Value) {
        use shape_value::v2::closure_layout::{
            SHARED_CELL_LOCKED, SHARED_CELL_STATE_OFFSET, SHARED_CELL_UNLOCKED,
        };
        // Inline CAS from 0→1 with Acquire ordering on success.
        // `atomic_cas` in Cranelift returns the previous value, so
        // success = (prev == 0).
        let state_addr = if SHARED_CELL_STATE_OFFSET == 0 {
            cell_ptr
        } else {
            // Compile-time constant — this branch is currently dead
            // because SHARED_CELL_STATE_OFFSET == 0, but kept for
            // forward-compatibility if the layout ever shifts.
            self.builder
                .ins()
                .iadd_imm(cell_ptr, SHARED_CELL_STATE_OFFSET as i64)
        };
        let unlocked = self
            .builder
            .ins()
            .iconst(types::I8, SHARED_CELL_UNLOCKED as i64);
        let locked = self
            .builder
            .ins()
            .iconst(types::I8, SHARED_CELL_LOCKED as i64);
        let prev = self
            .builder
            .ins()
            .atomic_cas(MemFlags::trusted(), state_addr, unlocked, locked);
        // If prev != 0, the CAS failed (state was already locked). Call
        // the contended helper which spin-waits until it can flip to 1.
        let ok = self.builder.ins().icmp_imm(
            IntCC::Equal,
            prev,
            SHARED_CELL_UNLOCKED as i64,
        );
        let contended_block = self.builder.create_block();
        let after_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(ok, after_block, &[], contended_block, &[]);
        self.builder.switch_to_block(contended_block);
        self.builder.seal_block(contended_block);
        self.builder
            .ins()
            .call(self.ffi.shared_lock_contended, &[cell_ptr]);
        self.builder.ins().jump(after_block, &[]);
        self.builder.switch_to_block(after_block);
        self.builder.seal_block(after_block);
    }

    /// Emit the inline unlock fast path. On success, state byte
    /// transitions from 1→0 with `Release` ordering. The current
    /// implementation always uses a release store (no PARKED_BIT to
    /// check for); the "contended" branch is vestigial but kept for
    /// ABI parity with the lock side.
    fn emit_shared_unlock(&mut self, cell_ptr: Value) {
        use shape_value::v2::closure_layout::{
            SHARED_CELL_LOCKED, SHARED_CELL_STATE_OFFSET, SHARED_CELL_UNLOCKED,
        };
        let state_addr = if SHARED_CELL_STATE_OFFSET == 0 {
            cell_ptr
        } else {
            self.builder
                .ins()
                .iadd_imm(cell_ptr, SHARED_CELL_STATE_OFFSET as i64)
        };
        // Inline CAS from 1→0 with Release ordering on success. If the
        // CAS fails (someone smashed our state byte — a bug, but we
        // defend against it), call the contended unlocker which writes
        // state=0 with release ordering unconditionally.
        let locked = self
            .builder
            .ins()
            .iconst(types::I8, SHARED_CELL_LOCKED as i64);
        let unlocked = self
            .builder
            .ins()
            .iconst(types::I8, SHARED_CELL_UNLOCKED as i64);
        let prev = self
            .builder
            .ins()
            .atomic_cas(MemFlags::trusted(), state_addr, locked, unlocked);
        let ok = self.builder.ins().icmp_imm(
            IntCC::Equal,
            prev,
            SHARED_CELL_LOCKED as i64,
        );
        let contended_block = self.builder.create_block();
        let after_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(ok, after_block, &[], contended_block, &[]);
        self.builder.switch_to_block(contended_block);
        self.builder.seal_block(contended_block);
        self.builder
            .ins()
            .call(self.ffi.shared_unlock_contended, &[cell_ptr]);
        self.builder.ins().jump(after_block, &[]);
        self.builder.switch_to_block(after_block);
        self.builder.seal_block(after_block);
    }

    // ── Track A.1D.2: OwnedMutable capture cell write widening ──────
    //
    // A cell stores a raw `ValueWord` u64 bit-pattern regardless of the
    // narrow Cranelift type the compiler proved for the captured
    // variable (int as native I32, bool as I8, etc.). This helper
    // widens / bitcasts to I64 before `store.i64` through the cell
    // pointer, matching the interpreter's `op_store_owned_mutable_capture`
    // which writes a raw u64 via `std::ptr::write`. Uses `bitcast` for
    // F64 (preserve bit pattern) and zero-extend for narrow ints (the
    // cell only reads back the low `width` bytes via matching narrow
    // loads — A.1D.2 always reads back with `load.i64` in `read_place`,
    // so zero-extension is safe since the high bits get re-examined
    // only by the MIR level's subsequent `ireduce` via `ensure_kind`).
    fn coerce_value_to_i64_bits(&mut self, val: Value) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        if val_type == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
        } else if val_type == types::I64 {
            val
        } else if val_type == types::I32
            || val_type == types::I16
            || val_type == types::I8
        {
            self.builder.ins().uextend(types::I64, val)
        } else {
            val
        }
    }

    // ── Inline array access helpers ──────────────────────────────────────
    // Ported from BytecodeToIR::inline_ops.rs for the MirToIR path.
    // These bypass FFI calls and emit direct Cranelift memory loads,
    // eliminating ~50-100ns per array access in hot loops.

    /// Extract the raw heap pointer from a NaN-boxed heap value.
    /// Masks off tag bits and the unified heap flag (bit 47).
    #[inline]
    fn emit_payload_ptr(&mut self, boxed: Value) -> Value {
        let ptr_mask = self.builder.ins().iconst(types::I64, UNIFIED_PTR_MASK as i64);
        self.builder.ins().band(boxed, ptr_mask)
    }

    /// Get pointer to the JitArray/UnifiedArray data fields (past 8-byte header).
    #[inline]
    fn emit_array_ptr(&mut self, arr_boxed: Value) -> Value {
        let alloc_ptr = self.emit_payload_ptr(arr_boxed);
        self.builder.ins().iadd_imm(alloc_ptr, JIT_ALLOC_DATA_OFFSET as i64)
    }

    /// Load (data_ptr, length) from a JitArray/UnifiedArray.
    /// JitArray layout after header: data_ptr at +0, len at +8.
    #[inline]
    fn emit_array_data_and_len(&mut self, arr_boxed: Value) -> (Value, Value) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self.builder.ins().load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let length = self.builder.ins().load(types::I64, MemFlags::trusted(), arr_ptr, 8);
        (data_ptr, length)
    }

    /// Convert a NaN-boxed index to a raw i64.
    /// Handles both NaN-boxed f64 (number) and NaN-boxed i48 (int).
    fn emit_index_to_i64(&mut self, index_bits: Value) -> Value {
        // If bits < TAG_BASE (0xFFF8...), it's a raw f64 — bitcast and convert.
        // If bits >= TAG_BASE, it's a tagged value (int) — extract i48 payload.
        // For performance, we use bitcast → fcvt which handles the common f64 case.
        // For NaN-boxed ints, fcvt_to_sint_sat on NaN gives 0, so we also extract
        // the int payload and select based on a check.
        let tag_base = self.builder.ins().iconst(types::I64, 0xFFF8_0000_0000_0000u64 as i64);
        let is_tagged = self.builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index_bits, tag_base);

        // Float path: bitcast to f64, convert to i64
        let as_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), index_bits);
        let from_float = self.builder.ins().fcvt_to_sint_sat(types::I64, as_f64);

        // Int path: sign-extend lower 48 bits
        let shifted_left = self.builder.ins().ishl_imm(index_bits, 16);
        let from_int = self.builder.ins().sshr_imm(shifted_left, 16);

        // Select: if tagged (int), use int extraction; else use float conversion
        self.builder.ins().select(is_tagged, from_int, from_float)
    }

    /// Normalize negative array index: if idx < 0, idx = length + idx.
    #[inline]
    fn normalize_index(&mut self, idx: Value, length: Value) -> Value {
        let zero = self.builder.ins().iconst(types::I64, 0);
        let is_negative = self.builder.ins().icmp(IntCC::SignedLessThan, idx, zero);
        let adjusted = self.builder.ins().iadd(length, idx);
        self.builder.ins().select(is_negative, adjusted, idx)
    }

    /// Bounds check: if index >= length (unsigned), return 0 (safe default).
    /// Using unsigned comparison catches both negative (wrapped) and too-large indices.
    #[inline]
    fn bounds_check(&mut self, index: Value, length: Value) -> Value {
        let in_bounds = self.builder.ins().icmp(IntCC::UnsignedLessThan, index, length);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().select(in_bounds, index, zero)
    }

    /// Convert an index value to i64, specializing for native types.
    /// For native I32: sextend (1 instruction).
    /// For NaN-boxed I64: extract payload (7 instructions via emit_index_to_i64).
    fn index_to_i64(&mut self, index_val: Value) -> Value {
        let idx_type = self.builder.func.dfg.value_type(index_val);
        if idx_type == types::F64 {
            // Native F64 index — convert to I64 via fcvt_to_sint_sat
            self.builder.ins().fcvt_to_sint_sat(types::I64, index_val)
        } else if idx_type == types::I32 {
            // Native I32 index — sign-extend to I64
            self.builder.ins().sextend(types::I64, index_val)
        } else if idx_type == types::I8 {
            // Native I8 — zero-extend
            self.builder.ins().uextend(types::I64, index_val)
        } else {
            // I64: NaN-boxed int or NaN-boxed float
            self.emit_index_to_i64(index_val)
        }
    }

    /// Inline array element read: arr[index] → direct memory load.
    /// ~8 Cranelift instructions instead of an FFI call.
    fn inline_array_get(&mut self, arr_boxed: Value, index_val: Value) -> Value {
        let (data_ptr, length) = self.emit_array_data_and_len(arr_boxed);
        let idx_i64 = self.index_to_i64(index_val);
        let final_idx = self.normalize_index(idx_i64, length);
        let safe_idx = self.bounds_check(final_idx, length);

        // Element address: data_ptr + safe_idx * 8 (u64 slots)
        let byte_offset = self.builder.ins().ishl_imm(safe_idx, 3); // * 8
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder.ins().load(types::I64, MemFlags::trusted(), elem_addr, 0)
    }

    /// Inline array element write: arr[index] = value → direct memory store.
    fn inline_array_set(&mut self, arr_boxed: Value, index_val: Value, val: Value) {
        let (data_ptr, length) = self.emit_array_data_and_len(arr_boxed);
        let idx_i64 = self.index_to_i64(index_val);
        let final_idx = self.normalize_index(idx_i64, length);
        let safe_idx = self.bounds_check(final_idx, length);

        let byte_offset = self.builder.ins().ishl_imm(safe_idx, 3);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder.ins().store(MemFlags::trusted(), val, elem_addr, 0);
    }

    // ── Inline typed-struct field access ──────────────────────────────
    //
    // When the compiler knows the field byte offset at compile time, we
    // emit 2 Cranelift loads (pointer chase through the UnifiedValue
    // wrapper) instead of an FFI call to jit_typed_object_get_field.
    //
    // Memory layout:
    //   NaN-boxed bits  --&(UNIFIED_PTR_MASK)-->  UnifiedValue<*const u8>
    //     +8 (data field) -->  raw TypedObject*
    //       +8 (TYPED_OBJ_HEADER) + field_byte_offset --> field u64 slot

    /// Extract the raw `TypedObject*` from a NaN-boxed typed-object value.
    ///
    /// Two-step pointer chase:
    /// 1. `uv_ptr = bits & UNIFIED_PTR_MASK` → `UnifiedValue<*const u8>*`
    /// 2. `to_ptr = load i64 [uv_ptr + 8]`   → `TypedObject*`
    fn emit_typed_object_ptr(&mut self, nanboxed_bits: Value) -> Value {
        let ptr_mask = self.builder.ins().iconst(types::I64, UNIFIED_PTR_MASK as i64);
        let uv_ptr = self.builder.ins().band(nanboxed_bits, ptr_mask);
        // Load the `data` field from the UnifiedValue wrapper
        self.builder.ins().load(types::I64, MemFlags::trusted(), uv_ptr, UNIFIED_VALUE_DATA_OFFSET)
    }

    /// Inline typed field read: load u64 from `[typed_obj_ptr + HEADER + byte_off]`.
    fn inline_typed_field_get(&mut self, nanboxed_bits: Value, byte_off: u16) -> Value {
        let to_ptr = self.emit_typed_object_ptr(nanboxed_bits);
        let offset = TYPED_OBJ_HEADER + byte_off as i32;
        self.builder.ins().load(types::I64, MemFlags::trusted(), to_ptr, offset)
    }

    /// Inline typed field write: store u64 to `[typed_obj_ptr + HEADER + byte_off]`.
    fn inline_typed_field_set(&mut self, nanboxed_bits: Value, byte_off: u16, val: Value) {
        let to_ptr = self.emit_typed_object_ptr(nanboxed_bits);
        let offset = TYPED_OBJ_HEADER + byte_off as i32;
        self.builder.ins().store(MemFlags::trusted(), val, to_ptr, offset);
    }

    // ── Field offset resolution ────────────────────────────────────────

    fn try_resolve_field_byte_offset(&self, field_idx: &FieldIdx) -> Option<u16> {
        let name = self.mir.field_name_table.get(field_idx)?;
        self.field_byte_offsets.get(name).copied()
    }

    // v2-boundary: get_prop/set_prop FFI uses NaN-boxed string keys
    fn field_idx_to_boxed_key(&self, field_idx: &FieldIdx) -> Option<u64> {
        let name = self.mir.field_name_table.get(field_idx)?;
        Some(crate::ffi::value_ffi::box_string(name.clone()))
    }

    // ── Place resolution ─────────────────────────────────────────────────

    /// Read a value from a Place.
    pub(crate) fn read_place(&mut self, place: &Place) -> Result<Value, String> {
        match place {
            Place::Local(slot) => {
                let var = self.locals.get(slot).ok_or_else(|| {
                    format!("MirToIR: unknown local slot {}", slot)
                })?;
                // Track A.1D.2: OwnedMutable capture slots hold the raw
                // `*mut ValueWord` bits of a `Box::into_raw`'d cell
                // (allocated by `jit_alloc_owned_mut_cell` in
                // `emit_heap_closure`). The captured variable's current
                // value lives inside the cell; reading the slot directly
                // would yield pointer bits, not the value. Emit a
                // pointer-deref load — observationally equivalent to the
                // interpreter's `op_load_owned_mutable_capture` handler
                // (`std::ptr::read(cell_ptr)`).
                //
                // SAFETY: the pointer is non-null and 8-aligned (Box
                // alignment for u64), valid for the closure's refcounted
                // lifetime — `jit_alloc_owned_mut_cell` created it via
                // `Box::into_raw(Box::new(initial))`, and
                // `release_typed_closure`'s `Box::from_raw` reclaim only
                // runs after the closure's refcount hits zero (which
                // happens strictly after every call using this cell
                // returns).
                if self.owned_mutable_capture_slots.contains(slot) {
                    let cell_ptr = self.builder.use_var(*var);
                    return Ok(self.builder.ins().load(
                        types::I64,
                        MemFlags::trusted(),
                        cell_ptr,
                        0,
                    ));
                }
                // Track A.1E: Shared capture slots hold the raw
                // `*const SharedCell` bits of an Arc-shared cell
                // (retained via `jit_arc_shared_retain` in
                // `emit_heap_closure`). The captured variable's current
                // value lives inside the cell's `value` field at offset
                // `SHARED_CELL_VALUE_OFFSET` (8); reads/writes must
                // lock-gate through the state byte at offset 0.
                //
                // Matches the interpreter's `op_load_shared_capture`
                // handler semantics exactly: acquire the mutex, copy
                // the inner ValueWord bits, release the mutex. No
                // retain/release on the Arc strong share — the closure
                // owns one share for the lifetime of the frame.
                //
                // SAFETY: the pointer is non-null and 8-aligned (Rust
                // `Arc::<SharedCell>` allocator + `SharedCell`'s
                // align=8). The Arc share owned by the closure keeps
                // the allocation alive for the duration of any JIT'd
                // call that reads this slot; the reclaim (via
                // `Arc::from_raw` in `release_typed_closure`) only
                // runs after the closure's refcount hits zero, which
                // is strictly after the JIT body returns.
                if self.shared_capture_slots.contains(slot) {
                    use shape_value::v2::closure_layout::SHARED_CELL_VALUE_OFFSET;
                    let cell_ptr = self.builder.use_var(*var);
                    self.emit_shared_lock(cell_ptr);
                    let value = self.builder.ins().load(
                        types::I64,
                        MemFlags::trusted(),
                        cell_ptr,
                        SHARED_CELL_VALUE_OFFSET,
                    );
                    self.emit_shared_unlock(cell_ptr);
                    return Ok(value);
                }
                Ok(self.builder.use_var(*var))
            }
            Place::Field(base, field_idx) => {
                // v2 fast path: `arr.length` on a typed-array slot — emit a
                // single inline `v2_array_len` load and sign-extend to i64.
                if self.v2_typed_array_elem_kind(base).is_some() {
                    if let Some(name) = self.mir.field_name_table.get(field_idx) {
                        if name == "length" {
                            let arr_ptr = self.read_place(base)?;
                            let len_i32 = self.v2_array_len(arr_ptr);
                            let len_i64 = self.builder.ins().sextend(types::I64, len_i32);
                            return Ok(len_i64);
                        }
                    }
                }

                // R4.2C: FFI signatures accept plain u64 bit-patterns — no
                // box wrap needed at call site. `get_prop` / `inline_typed_field_get`
                // take the heap pointer as an already-ValueWord-encoded I64.
                let base_val = self.read_place(base)?;
                if let Some(byte_off) = self.try_resolve_field_byte_offset(field_idx) {
                    // Inline typed field read — 2 loads, no FFI call.
                    Ok(self.inline_typed_field_get(base_val, byte_off))
                } else if let Some(boxed_key) = self.field_idx_to_boxed_key(field_idx) {
                    let key = self.builder.ins().iconst(types::I64, boxed_key as i64);
                    let inst = self.builder.ins().call(self.ffi.get_prop, &[base_val, key]);
                    Ok(self.builder.inst_results(inst)[0])
                } else {
                    let field = self.builder.ins().iconst(types::I64, field_idx.0 as i64);
                    let inst = self.builder.ins().call(self.ffi.get_prop, &[base_val, field]);
                    Ok(self.builder.inst_results(inst)[0])
                }
            }
            Place::Index(base, operand) => {
                // v2 fast path: when the base local holds a v2 `Array<scalar>`
                // pointer, use the inline `v2_array_get` helper.
                if let Some(elem_kind) = self.v2_typed_array_elem_kind(base) {
                    let arr_ptr = self.read_place(base)?;
                    let raw_idx = self.compile_operand_raw(operand)?;
                    let idx_i32 = self.coerce_index_to_i32(raw_idx);
                    let elem_val = self.v2_array_get(arr_ptr, idx_i32, elem_kind);
                    return Ok(elem_val);
                }

                // R4.2B: FFI signatures accept plain u64 bit-patterns — no
                // box wrap needed at call site. `inline_array_get` takes the
                // heap pointer as an already-ValueWord-encoded I64.
                let base_val = self.read_place(base)?;
                // Index can stay native — index_to_i64 handles all types
                let index_val = self.compile_operand_raw(operand)?;
                Ok(self.inline_array_get(base_val, index_val))
            }
            Place::Deref(inner) => {
                let ref_addr = self.read_place(inner)?;
                Ok(self.builder.ins().load(types::I64, MemFlags::new(), ref_addr, 0))
            }
        }
    }

    /// Write a value to a Place, converting to the slot's native type if needed.
    pub(crate) fn write_place(
        &mut self,
        place: &Place,
        val: Value,
    ) -> Result<(), String> {
        match place {
            Place::Local(slot) => {
                // Track A.1D.2: OwnedMutable capture slots redirect the
                // write through the `*mut ValueWord` cell pointer held
                // in the slot. `var` itself must keep the pointer bits
                // for the whole frame — we never `def_var` over it.
                //
                // The value is widened to I64 (the cell's slot type) via
                // `widen_to_i64`, because the cell stores a
                // `ValueWord`-encoded bit pattern regardless of the
                // compile-time narrow type of the captured variable.
                // Matches the interpreter's `op_store_owned_mutable_capture`
                // which writes a raw `u64` via `std::ptr::write`.
                //
                // SAFETY: see `read_place`. Writing 8 bytes in-place
                // does not release the previous cell contents — mirrors
                // the interpreter's comment: "The old cell contents are
                // overwritten in place — if the old contents were a
                // heap-tagged ValueWord share, the caller is responsible
                // for ensuring the write does not leak". Typical `let
                // mut` captures hold primitive (int/float/bool) values,
                // which have no heap refcount; heap-typed captures
                // would need MIR-level Drop insertion to balance retain
                // counts (deferred — matches interpreter parity).
                if self.owned_mutable_capture_slots.contains(slot) {
                    let var = *self.locals.get(slot).ok_or_else(|| {
                        format!("MirToIR: unknown local slot {}", slot)
                    })?;
                    let cell_ptr = self.builder.use_var(var);
                    let bits = self.coerce_value_to_i64_bits(val);
                    self.builder
                        .ins()
                        .store(MemFlags::trusted(), bits, cell_ptr, 0);
                    return Ok(());
                }
                // Track A.1E: Shared capture slot write — lock-gated
                // store through the SharedCell. Mirrors the interpreter's
                // `op_store_shared_capture`: take the mutex, overwrite
                // the inner ValueWord payload, drop the guard. The Arc
                // pointer bits in the slot are NOT modified (the closure
                // still owns its one strong share for frame lifetime).
                //
                // SAFETY: see `read_place` Shared branch.
                if self.shared_capture_slots.contains(slot) {
                    use shape_value::v2::closure_layout::SHARED_CELL_VALUE_OFFSET;
                    let var = *self.locals.get(slot).ok_or_else(|| {
                        format!("MirToIR: unknown local slot {}", slot)
                    })?;
                    let cell_ptr = self.builder.use_var(var);
                    let bits = self.coerce_value_to_i64_bits(val);
                    self.emit_shared_lock(cell_ptr);
                    self.builder.ins().store(
                        MemFlags::trusted(),
                        bits,
                        cell_ptr,
                        SHARED_CELL_VALUE_OFFSET,
                    );
                    self.emit_shared_unlock(cell_ptr);
                    return Ok(());
                }

                let target_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
                let var = *self.locals.get(slot).ok_or_else(|| {
                    format!("MirToIR: unknown local slot {}", slot)
                })?;
                // Convert value to match the slot's declared Cranelift type.
                let converted = self.ensure_kind(val, target_kind);
                self.builder.def_var(var, converted);
                Ok(())
            }
            Place::Field(base, field_idx) => {
                // R4.2C: FFI signatures accept plain u64 bit-patterns — no
                // box wrap needed at call site. Both the heap pointer base
                // and the field value reach `set_prop` / `typed_object_set_field`
                // as already-ValueWord-encoded I64 slots.
                let base_val = self.read_place(base)?;
                if let Some(byte_off) = self.try_resolve_field_byte_offset(field_idx) {
                    // Inline typed field write — 2 loads + 1 store, no FFI call.
                    // Write barrier is a no-op without the `gc` feature, so we skip it.
                    self.inline_typed_field_set(base_val, byte_off, val);
                } else if let Some(boxed_key) = self.field_idx_to_boxed_key(field_idx) {
                    let key = self.builder.ins().iconst(types::I64, boxed_key as i64);
                    self.builder.ins().call(self.ffi.set_prop, &[base_val, key, val]);
                } else {
                    let field = self.builder.ins().iconst(types::I64, field_idx.0 as i64);
                    self.builder.ins().call(self.ffi.set_prop, &[base_val, field, val]);
                }
                Ok(())
            }
            Place::Index(base, operand) => {
                // v2 fast path: same logic as `read_place`. The slot is a raw
                // `*mut TypedArray<T>`, the index becomes an i32, and the
                // value is coerced to the element's native type.
                if let Some(elem_kind) = self.v2_typed_array_elem_kind(base) {
                    let arr_ptr = self.read_place(base)?;
                    let raw_idx = self.compile_operand_raw(operand)?;
                    let idx_i32 = self.coerce_index_to_i32(raw_idx);
                    let elem_val = self.coerce_to_v2_elem(val, elem_kind);
                    self.v2_array_set(arr_ptr, idx_i32, elem_val, elem_kind);
                    return Ok(());
                }

                // R4.2B: FFI signatures accept plain u64 bit-patterns — no
                // box wrap needed at call site. Both the heap pointer base
                // and the element value reach `inline_array_set` as
                // already-ValueWord-encoded I64 slots.
                let base_val = self.read_place(base)?;
                let index_val = self.compile_operand_raw(operand)?;
                self.inline_array_set(base_val, index_val, val);
                Ok(())
            }
            Place::Deref(inner) => {
                // R4.2F: ref cells are native-width — store at the value's
                // natural Cranelift type. Width is inferred from `val`'s
                // type, which matches the cell shape created in
                // `Rvalue::Borrow`. No NaN-box wrap needed.
                let ref_addr = self.read_place(inner)?;
                self.builder.ins().store(MemFlags::new(), val, ref_addr, 0);
                Ok(())
            }
        }
    }

    /// Write zero/null to a Place's root local.
    /// Used after Move to prevent double-drop.
    /// Uses type-appropriate zero for native slots (0.0 for F64, 0 for I32, etc.)
    pub(crate) fn null_place(&mut self, place: &Place) -> Result<(), String> {
        let slot = place.root_local();
        // Track A.1D.2: OwnedMutable capture slots permanently hold the
        // raw `*mut ValueWord` bits of the cell for the duration of the
        // frame. Zeroing the slot would zero the cell pointer —
        // subsequent reads/writes would deref null, and
        // `release_typed_closure`'s `Box::from_raw` (gated on the
        // `owned_mutable_capture_mask`, A.1A) would see a null pointer.
        // The interpreter's handlers never touch the upvalue slot on
        // Move semantics either; this preserves parity.
        //
        // Track A.1E: same story for Shared capture slots — they
        // permanently hold the `*const SharedCell` bits of the Arc-
        // shared cell, and `release_typed_closure`'s `Arc::from_raw`
        // (gated on `shared_capture_mask`, A.1A) reclaims them exactly
        // once when the closure's refcount hits zero.
        if matches!(place, Place::Local(_))
            && (self.owned_mutable_capture_slots.contains(&slot)
                || self.shared_capture_slots.contains(&slot))
        {
            return Ok(());
        }
        // Only null the root local for simple locals.
        // Field/Index moves don't null the entire container.
        if matches!(place, Place::Local(_)) {
            let var = self.locals.get(&slot).ok_or_else(|| {
                format!("MirToIR: unknown local slot {}", slot)
            })?;
            let kind = self.slot_kind_of(slot);
            let null = match kind {
                shape_vm::type_tracking::SlotKind::Float64 => {
                    self.builder.ins().f64const(0.0)
                }
                shape_vm::type_tracking::SlotKind::Int32
                | shape_vm::type_tracking::SlotKind::UInt32 => {
                    self.builder.ins().iconst(types::I32, 0)
                }
                shape_vm::type_tracking::SlotKind::Bool
                | shape_vm::type_tracking::SlotKind::Int8
                | shape_vm::type_tracking::SlotKind::UInt8 => {
                    self.builder.ins().iconst(types::I8, 0)
                }
                shape_vm::type_tracking::SlotKind::Int16
                | shape_vm::type_tracking::SlotKind::UInt16 => {
                    self.builder.ins().iconst(types::I16, 0)
                }
                // v2-boundary: I64 (NaN-boxed) slots use TAG_NULL as zero value
                _ => self
                    .builder
                    .ins()
                    .iconst(types::I64, 0i64),
            };
            self.builder.def_var(*var, null);
        }
        Ok(())
    }
}

// ===========================================================================
// Unit tests for inline typed-struct field access
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::Module;

    /// Build a minimal Cranelift JIT environment for testing.
    fn make_jit_env() -> (JITModule, cranelift::codegen::Context, FunctionBuilderContext) {
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

    /// Simulate the NaN-boxed UnifiedValue<*const u8> pointer chase that
    /// `inline_typed_field_get` / `inline_typed_field_set` perform.
    ///
    /// Allocates:
    /// - A TypedObject (header 8 bytes + N field slots of 8 bytes each)
    /// - A UnifiedValue wrapper: [kind:u16, flags:u8, _reserved:u8, refcount:u32, data:*const u8]
    ///
    /// Returns `(nanboxed_bits, typed_obj_ptr, uv_ptr)` — caller must free both allocations.
    unsafe fn make_test_typed_object(field_count: usize) -> (u64, *mut u8, *mut u8) {
        use crate::ffi::typed_object::TYPED_OBJECT_HEADER_SIZE;

        // 1. Allocate the TypedObject itself (8-byte header + fields)
        let to_size = TYPED_OBJECT_HEADER_SIZE + field_count * 8;
        let to_layout = std::alloc::Layout::from_size_align(to_size, 8).unwrap();
        let to_ptr = unsafe { std::alloc::alloc_zeroed(to_layout) };
        assert!(!to_ptr.is_null());

        // 2. Allocate the UnifiedValue<*const u8> wrapper (16 bytes: 8 header + 8 data)
        let uv_layout = std::alloc::Layout::from_size_align(16, 8).unwrap();
        let uv_ptr = unsafe { std::alloc::alloc_zeroed(uv_layout) };
        assert!(!uv_ptr.is_null());

        // Fill the UnifiedValue fields:
        //   kind (u16) at offset 0
        unsafe { *(uv_ptr as *mut u16) = crate::ffi::value_ffi::HK_TYPED_OBJECT };
        //   refcount (u32) at offset 4
        unsafe { *(uv_ptr.add(4) as *mut u32) = 1 };
        //   data (*const u8) at offset 8
        unsafe { *(uv_ptr.add(8) as *mut *const u8) = to_ptr as *const u8 };

        // 3. Build NaN-boxed bits: TAG_HEAP with UNIFIED_HEAP_FLAG + pointer
        let bits = shape_value::ValueBits::make_unified_heap(uv_ptr as *const u8).raw();

        (bits, to_ptr, uv_ptr)
    }

    unsafe fn free_test_typed_object(to_ptr: *mut u8, uv_ptr: *mut u8, field_count: usize) {
        use crate::ffi::typed_object::TYPED_OBJECT_HEADER_SIZE;
        let to_size = TYPED_OBJECT_HEADER_SIZE + field_count * 8;
        let to_layout = std::alloc::Layout::from_size_align(to_size, 8).unwrap();
        unsafe { std::alloc::dealloc(to_ptr, to_layout) };
        let uv_layout = std::alloc::Layout::from_size_align(16, 8).unwrap();
        unsafe { std::alloc::dealloc(uv_ptr, uv_layout) };
    }

    /// Test that the inline typed field read produces the correct result
    /// by compiling a Cranelift function that performs the UNIFIED_PTR_MASK +
    /// double-load pattern used by `inline_typed_field_get`.
    #[test]
    fn inline_typed_field_get_through_unified_value() {
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        // fn(nanboxed_bits: i64) -> i64
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function(
                "test_inline_field_get",
                cranelift_module::Linkage::Local,
                &sig,
            )
            .unwrap();

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let bits = builder.block_params(entry)[0];

            // Manually emit the inline_typed_field_get pattern for field at byte_off=8
            // (second field, first field is at byte_off=0).
            let ptr_mask = builder.ins().iconst(types::I64, UNIFIED_PTR_MASK as i64);
            let uv_ptr = builder.ins().band(bits, ptr_mask);
            let to_ptr = builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                uv_ptr,
                UNIFIED_VALUE_DATA_OFFSET,
            );
            // field at byte_off=8 -> total offset = TYPED_OBJ_HEADER(8) + 8 = 16
            let result = builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                to_ptr,
                TYPED_OBJ_HEADER + 8,
            );

            builder.ins().return_(&[result]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let (bits, to_ptr, uv_ptr) = make_test_typed_object(3);

            // Write test values to TypedObject fields (NaN-boxed numbers)
            let field_base = to_ptr.add(8) as *mut u64; // past 8-byte header
            *field_base = crate::ffi::value_ffi::box_number(100.0); // field[0] at offset 0
            *field_base.add(1) = crate::ffi::value_ffi::box_number(200.0); // field[1] at offset 8
            *field_base.add(2) = crate::ffi::value_ffi::box_number(300.0); // field[2] at offset 16

            let func: unsafe fn(u64) -> u64 = std::mem::transmute(code_ptr);
            let result = func(bits);
            assert_eq!(
                crate::ffi::value_ffi::unbox_number(result),
                200.0,
                "inline_typed_field_get should load the second field (byte_off=8)"
            );

            free_test_typed_object(to_ptr, uv_ptr, 3);
        }
    }

    /// Test that the inline typed field write correctly stores a value
    /// by compiling a Cranelift function that performs the UNIFIED_PTR_MASK +
    /// load + store pattern used by `inline_typed_field_set`.
    #[test]
    fn inline_typed_field_set_through_unified_value() {
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();

        // fn(nanboxed_bits: i64, value: i64)
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function(
                "test_inline_field_set",
                cranelift_module::Linkage::Local,
                &sig,
            )
            .unwrap();

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let bits = builder.block_params(entry)[0];
            let val = builder.block_params(entry)[1];

            // Manually emit the inline_typed_field_set pattern for field at byte_off=0
            // (first field).
            let ptr_mask = builder.ins().iconst(types::I64, UNIFIED_PTR_MASK as i64);
            let uv_ptr = builder.ins().band(bits, ptr_mask);
            let to_ptr = builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                uv_ptr,
                UNIFIED_VALUE_DATA_OFFSET,
            );
            // field at byte_off=0 -> total offset = TYPED_OBJ_HEADER(8) + 0 = 8
            builder.ins().store(
                MemFlags::trusted(),
                val,
                to_ptr,
                TYPED_OBJ_HEADER + 0,
            );

            builder.ins().return_(&[]);
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);

        unsafe {
            let (bits, to_ptr, uv_ptr) = make_test_typed_object(2);

            let func: unsafe fn(u64, u64) = std::mem::transmute(code_ptr);
            func(bits, crate::ffi::value_ffi::box_number(999.0));

            // Verify the value was written to the correct location
            let field_base = to_ptr.add(8) as *const u64;
            let stored = *field_base;
            assert_eq!(
                crate::ffi::value_ffi::unbox_number(stored),
                999.0,
                "inline_typed_field_set should store to the first field (byte_off=0)"
            );

            free_test_typed_object(to_ptr, uv_ptr, 2);
        }
    }

    /// Verify that the constants match the actual Rust struct layouts.
    #[test]
    fn constants_match_struct_layouts() {
        assert_eq!(
            UNIFIED_VALUE_DATA_OFFSET as usize,
            std::mem::offset_of!(crate::ffi::jit_kinds::UnifiedValue::<*const u8>, data),
            "UNIFIED_VALUE_DATA_OFFSET must match UnifiedValue<*const u8>::data offset"
        );
        assert_eq!(
            TYPED_OBJ_HEADER as usize,
            crate::ffi::typed_object::TYPED_OBJECT_HEADER_SIZE,
            "TYPED_OBJ_HEADER must match TYPED_OBJECT_HEADER_SIZE"
        );
    }

    // -----------------------------------------------------------------------
    // R4.2F — borrow stack cell round-trip tests
    //
    // Mirrors the `Rvalue::Borrow` + `reload_referenced_locals` pattern at
    // the Cranelift level: a native-sized StackSlot is allocated, the input
    // value is `stack_store`d at offset 0, and the same type is read back
    // via `stack_load`. The round-trip must preserve the exact value for
    // every native slot width touched by the borrow path (I8 bool, I32 int,
    // F64 number, I64 fallback). Regressions here would indicate the
    // size/alignment computation in `Rvalue::Borrow` drifted.
    // -----------------------------------------------------------------------

    /// Compile `fn(v: T) -> T { let slot: T; stack_store v, slot; stack_load T, slot }`
    /// and return the resulting native function pointer wrapped in a u64-carrier
    /// closure. Size/align_shift are derived from `cl_ty` exactly as
    /// `Rvalue::Borrow` does.
    fn build_roundtrip_fn(
        module: &mut JITModule,
        ctx: &mut cranelift::codegen::Context,
        fb_ctx: &mut FunctionBuilderContext,
        name: &str,
        cl_ty: Type,
    ) -> *const u8 {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(cl_ty));
        sig.returns.push(AbiParam::new(cl_ty));

        let func_id = module
            .declare_function(name, cranelift_module::Linkage::Local, &sig)
            .unwrap();

        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, fb_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let v = builder.block_params(entry)[0];

            // Exact mirror of the R4.2F Rvalue::Borrow sizing logic.
            let size = cl_ty.bytes();
            let align_shift = size.trailing_zeros() as u8;
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                size,
                align_shift,
            ));

            builder.ins().stack_store(v, slot, 0);
            let reloaded = builder.ins().stack_load(cl_ty, slot, 0);
            builder.ins().return_(&[reloaded]);
            builder.finalize();
        }

        module.define_function(func_id, ctx).unwrap();
        module.clear_context(ctx);
        module.finalize_definitions().unwrap();
        module.get_finalized_function(func_id)
    }

    #[test]
    fn r4_2f_borrow_cell_roundtrip_bool_i8() {
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();
        let code = build_roundtrip_fn(
            &mut module,
            &mut ctx,
            &mut fb_ctx,
            "rt_bool",
            types::I8,
        );
        unsafe {
            let f: unsafe fn(u8) -> u8 = std::mem::transmute(code);
            assert_eq!(f(1), 1, "I8 borrow cell must preserve `true`");
            assert_eq!(f(0), 0, "I8 borrow cell must preserve `false`");
            assert_eq!(f(0xAA), 0xAA, "I8 borrow cell must preserve all 8 bits");
        }
    }

    #[test]
    fn r4_2f_borrow_cell_roundtrip_int_i32() {
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();
        let code = build_roundtrip_fn(
            &mut module,
            &mut ctx,
            &mut fb_ctx,
            "rt_i32",
            types::I32,
        );
        unsafe {
            let f: unsafe fn(i32) -> i32 = std::mem::transmute(code);
            assert_eq!(f(0), 0);
            assert_eq!(f(42), 42);
            assert_eq!(f(-1), -1, "I32 borrow cell must preserve sign bit");
            assert_eq!(f(i32::MIN), i32::MIN);
            assert_eq!(f(i32::MAX), i32::MAX);
        }
    }

    #[test]
    fn r4_2f_borrow_cell_roundtrip_number_f64() {
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();
        let code = build_roundtrip_fn(
            &mut module,
            &mut ctx,
            &mut fb_ctx,
            "rt_f64",
            types::F64,
        );
        unsafe {
            let f: unsafe fn(f64) -> f64 = std::mem::transmute(code);
            assert_eq!(f(1.5), 1.5, "F64 borrow cell must preserve fraction bits");
            assert_eq!(f(-0.0).to_bits(), (-0.0f64).to_bits(),
                "F64 borrow cell must preserve sign of zero");
            let nan = f64::from_bits(0x7FF8_0000_0000_0001);
            assert_eq!(f(nan).to_bits(), nan.to_bits(),
                "F64 borrow cell must preserve NaN payload");
        }
    }

    #[test]
    fn r4_2f_borrow_cell_roundtrip_fallback_i64() {
        // Non-native slot kinds (heap / string / unknown) → I64 cell.
        // This is the legacy 8-byte path and must remain byte-accurate.
        let (mut module, mut ctx, mut fb_ctx) = make_jit_env();
        let code = build_roundtrip_fn(
            &mut module,
            &mut ctx,
            &mut fb_ctx,
            "rt_i64",
            types::I64,
        );
        unsafe {
            let f: unsafe fn(u64) -> u64 = std::mem::transmute(code);
            assert_eq!(f(0), 0);
            assert_eq!(f(u64::MAX), u64::MAX);
            assert_eq!(f(0xFFF8_0000_0000_0001), 0xFFF8_0000_0000_0001,
                "I64 borrow cell must preserve NaN-boxed tag patterns");
        }
    }
}
