use super::super::*;

impl VirtualMachine {
    pub fn create_typed_enum(
        &self,
        enum_name: &str,
        variant_name: &str,
        payload: Vec<ValueWord>,
    ) -> Option<ValueWord> {
        let nb_payload: Vec<ValueWord> = payload.into_iter().map(|v| v).collect();
        self.create_typed_enum_nb(enum_name, variant_name, nb_payload)
            .map(|nb| nb.clone())
    }

    /// Create a TypedObject enum value using ValueWord payload directly.
    pub fn create_typed_enum_nb(
        &self,
        enum_name: &str,
        variant_name: &str,
        payload: Vec<ValueWord>,
    ) -> Option<ValueWord> {
        let schema = self.program.type_schema_registry.get(enum_name)?;
        let enum_info = schema.get_enum_info()?;
        let variant_id = enum_info.variant_id(variant_name)?;

        // Build slots: slot 0 = variant_id, slot 1+ = payload
        let slot_count = 1 + enum_info.max_payload_fields() as usize;
        let mut slots = Vec::with_capacity(slot_count);
        let mut heap_mask: u64 = 0;

        // Slot 0: variant discriminator is an i64 field (`__variant`).
        slots.push(ValueSlot::from_int(variant_id as i64));

        // Payload slots
        for (i, nb) in payload.into_iter().enumerate() {
            let slot_idx = 1 + i;
            match nb.tag() {
                shape_value::NanTag::F64 => {
                    slots.push(ValueSlot::from_number(nb.as_f64().unwrap_or(0.0)))
                }
                shape_value::NanTag::I48 => {
                    slots.push(ValueSlot::from_number(nb.as_i64().unwrap_or(0) as f64))
                }
                shape_value::NanTag::Bool => {
                    slots.push(ValueSlot::from_bool(nb.as_bool().unwrap_or(false)))
                }
                shape_value::NanTag::None => slots.push(ValueSlot::none()),
                _ => {
                    if let Some(hv) = nb.as_heap_ref() {
                        slots.push(ValueSlot::from_heap(hv.clone()));
                        heap_mask |= 1u64 << slot_idx;
                    } else {
                        // Function/ModuleFunction/Unit/other inline types: store as int slot
                        let id = nb
                            .as_function()
                            .or_else(|| nb.as_module_function().map(|u| u as u16))
                            .unwrap_or(0);
                        slots.push(ValueSlot::from_int(id as i64));
                    }
                }
            }
        }

        // Fill remaining payload slots with None
        while slots.len() < slot_count {
            slots.push(ValueSlot::none());
        }

        Some(ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema.id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        }))
    }

    // === Raw u64 stack constants ===

    /// The raw u64 bit pattern for `ValueWord::none()`.
    /// This is `TAG_BASE | (TAG_NONE << 48) = 0xFFFB_0000_0000_0000`.
    pub(crate) const NONE_BITS: u64 = 0xFFFB_0000_0000_0000u64;

    // --- ValueWord-direct stack ops (compatibility shims over Vec<u64>) ---

    /// Push a ValueWord onto the `Vec<u64>` stack.
    ///
    /// Transfers ownership of any embedded Arc refcount into the stack slot
    /// by using `into_raw_bits()` (which `mem::forget`s the ValueWord).
    ///
    /// Hot path: single bounds check + write.  The stack growth and overflow
    /// checks are split into a cold `push_vw_slow` to keep the hot path tight.
    #[inline(always)]
    pub fn push_vw(&mut self, value: ValueWord) -> Result<(), VMError> {
        let bits = value.into_raw_bits();
        if self.sp >= self.stack.len() {
            return self.push_vw_slow(bits);
        }
        // The old slot is a u64; overwriting it does NOT run any Drop.
        // We must manually drop the old ValueWord if it was heap-tagged.
        // However, stack slots above `sp` are always dead (NONE_BITS sentinel)
        // so there is nothing to drop.
        self.stack[self.sp] = bits;
        self.sp += 1;
        Ok(())
    }

    /// Cold path for push_vw: grow the stack or return StackOverflow.
    #[cold]
    #[inline(never)]
    pub fn push_vw_slow(&mut self, bits: u64) -> Result<(), VMError> {
        if self.sp >= self.config.max_stack_size {
            // Reconstruct ValueWord so its Drop runs (releases any heap ref).
            drop(ValueWord::from_raw_bits(bits));
            return Err(VMError::StackOverflow);
        }
        let new_len = self.sp * 2 + 1;
        self.stack.reserve(new_len - self.stack.len());
        while self.stack.len() < new_len {
            self.stack.push(Self::NONE_BITS);
        }
        self.stack[self.sp] = bits;
        self.sp += 1;
        Ok(())
    }

    /// Pop a ValueWord from the `Vec<u64>` stack.
    ///
    /// Reads the raw u64 bits and reconstructs a `ValueWord` that owns the
    /// embedded refcount.  The slot is overwritten with `NONE_BITS` so a
    /// subsequent `drop_stack_range` / VM teardown doesn't double-free.
    #[inline(always)]
    pub fn pop_vw(&mut self) -> Result<ValueWord, VMError> {
        if self.sp == 0 {
            return Self::pop_vw_underflow();
        }
        self.sp -= 1;
        let bits = self.stack[self.sp];
        self.stack[self.sp] = Self::NONE_BITS;
        // SAFETY: The bits were produced by `into_raw_bits()` in `push_vw`
        // (or by one of the `stack_write_vw` helpers) and represent a valid
        // ValueWord with exactly one outstanding refcount.
        Ok(ValueWord::from_raw_bits(bits))
    }

    #[cold]
    #[inline(never)]
    pub fn pop_vw_underflow() -> Result<ValueWord, VMError> {
        Err(VMError::StackUnderflow)
    }

    /// Pop and materialize a ValueWord from the stack (convenience for tests and legacy callers).
    pub fn pop(&mut self) -> Result<ValueWord, VMError> {
        self.pop_vw()
    }

    // === Indexed stack access helpers (ValueWord ↔ u64) ===

    /// Read a clone of the `ValueWord` at `stack[idx]` **without** removing it.
    ///
    /// This bumps the Arc refcount for heap-tagged values (clone semantics).
    #[inline(always)]
    pub(crate) fn stack_read_vw(&self, idx: usize) -> ValueWord {
        let bits = self.stack[idx];
        // Construct a temporary to call `.clone()`, which bumps the refcount,
        // then forget the temporary so its Drop doesn't decrement.
        let tmp = ValueWord::from_raw_bits(bits);
        let c = tmp.clone();
        std::mem::forget(tmp);
        c
    }

    /// Write a `ValueWord` into `stack[idx]`, dropping the previous occupant
    /// and transferring ownership of `value` into the slot.
    #[inline(always)]
    pub(crate) fn stack_write_vw(&mut self, idx: usize, value: ValueWord) {
        // Drop the old occupant (may decrement Arc refcount).
        let old_bits = self.stack[idx];
        drop(ValueWord::from_raw_bits(old_bits));
        self.stack[idx] = value.into_raw_bits();
    }

    /// Take ownership of the `ValueWord` at `stack[idx]`, replacing the slot
    /// with `NONE_BITS`.  Does NOT drop the old value — the caller owns it.
    #[inline(always)]
    pub(crate) fn stack_take_vw(&mut self, idx: usize) -> ValueWord {
        let bits = self.stack[idx];
        self.stack[idx] = Self::NONE_BITS;
        ValueWord::from_raw_bits(bits)
    }

    /// Peek at the raw u64 bits in `stack[idx]` and call a method on a
    /// *temporary* `ValueWord` reference.  The temporary is forgotten
    /// afterwards so no refcount change occurs.
    ///
    /// This is useful for read-only inspection (e.g. `.tag()`, `.is_i64()`,
    /// `.as_heap_ref()`) without paying a clone cost.
    ///
    /// # Safety
    /// The closure must NOT store the `&ValueWord` reference beyond the call.
    #[inline(always)]
    pub(crate) fn stack_peek_vw<F, R>(&self, idx: usize, f: F) -> R
    where
        F: FnOnce(&ValueWord) -> R,
    {
        let bits = self.stack[idx];
        // SAFETY: from_raw_bits just wraps the u64; the temporary lives for
        // the duration of the closure and is then forgotten.
        let tmp = ValueWord::from_raw_bits(bits);
        let result = f(&tmp);
        std::mem::forget(tmp);
        result
    }

    /// Get a read-only `&[ValueWord]` view of a stack range.
    ///
    /// # Safety
    /// ValueWord is `#[repr(transparent)]` over `u64`, so this transmute is safe.
    /// The returned slice must NOT be used to take ownership or drop ValueWords —
    /// it is a borrow-only view.
    #[inline(always)]
    pub(crate) fn stack_slice_vw(&self, range: std::ops::Range<usize>) -> &[ValueWord] {
        let slice = &self.stack[range];
        // SAFETY: ValueWord is #[repr(transparent)] over u64.
        unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const ValueWord, slice.len()) }
    }

    /// Peek at the top N raw u64 values on the stack without popping.
    ///
    /// Returns a slice `&[u64]` of the topmost `count` stack slots, with
    /// `slice[0]` being the deepest (pushed first) and `slice[count-1]`
    /// being the top of stack. This is the natural order for method args
    /// where the receiver was pushed first.
    ///
    /// Used by `MethodFnV2` native handlers to read args directly from
    /// the stack without allocating a `Vec<ValueWord>`.
    #[inline(always)]
    pub(crate) fn peek_args_slice(&self, count: usize) -> Result<&[u64], VMError> {
        if count > self.sp {
            return Err(VMError::StackUnderflow);
        }
        let start = self.sp - count;
        Ok(&self.stack[start..self.sp])
    }

    /// Get a read-only `&[ValueWord]` view of the module_bindings.
    #[inline(always)]
    pub(crate) fn bindings_slice_vw(&self) -> &[ValueWord] {
        let slice = &self.module_bindings;
        unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const ValueWord, slice.len()) }
    }

    // === Module binding helpers (same pattern as stack) ===

    /// Read a clone of the `ValueWord` at `module_bindings[idx]`.
    #[inline(always)]
    pub(crate) fn binding_read_vw(&self, idx: usize) -> ValueWord {
        let bits = self.module_bindings[idx];
        let tmp = ValueWord::from_raw_bits(bits);
        let c = tmp.clone();
        std::mem::forget(tmp);
        c
    }

    /// Write a `ValueWord` into `module_bindings[idx]`, dropping the old value.
    #[inline(always)]
    pub(crate) fn binding_write_vw(&mut self, idx: usize, value: ValueWord) {
        let old_bits = self.module_bindings[idx];
        drop(ValueWord::from_raw_bits(old_bits));
        self.module_bindings[idx] = value.into_raw_bits();
    }

    /// Take ownership of the `ValueWord` at `module_bindings[idx]`, replacing
    /// the slot with `NONE_BITS`.
    #[inline(always)]
    pub(crate) fn binding_take_vw(&mut self, idx: usize) -> ValueWord {
        let bits = self.module_bindings[idx];
        self.module_bindings[idx] = Self::NONE_BITS;
        ValueWord::from_raw_bits(bits)
    }

    // --- Raw typed stack tag checks (peek without popping) ---

    /// Check whether the top two stack values are both inline i48 integers.
    /// Used by typed int opcodes to select the raw fast path vs NaN-boxed fallback.
    #[inline(always)]
    pub(crate) fn stack_top_both_i48(&self) -> bool {
        if self.sp < 2 {
            return false;
        }
        // Peek at raw bits without touching ValueWord Clone/Drop.
        unsafe {
            let ptr = self.stack.as_ptr();
            let b_bits = std::ptr::read(ptr.add(self.sp - 1) as *const u64);
            let a_bits = std::ptr::read(ptr.add(self.sp - 2) as *const u64);
            shape_value::tags::is_tagged(a_bits)
                && shape_value::tags::get_tag(a_bits) == shape_value::tags::TAG_INT
                && shape_value::tags::is_tagged(b_bits)
                && shape_value::tags::get_tag(b_bits) == shape_value::tags::TAG_INT
        }
    }

    /// Check whether the top stack value is an inline i48 integer.
    #[inline(always)]
    pub(crate) fn stack_top_is_i48(&self) -> bool {
        if self.sp == 0 {
            return false;
        }
        unsafe {
            let bits = std::ptr::read(self.stack.as_ptr().add(self.sp - 1) as *const u64);
            shape_value::tags::is_tagged(bits)
                && shape_value::tags::get_tag(bits) == shape_value::tags::TAG_INT
        }
    }

    /// Check whether the top two stack values are both plain f64 (not tagged).
    /// Used by typed number opcodes to select the raw fast path vs NaN-boxed fallback.
    #[inline(always)]
    pub(crate) fn stack_top_both_f64(&self) -> bool {
        if self.sp < 2 {
            return false;
        }
        unsafe {
            let ptr = self.stack.as_ptr();
            let b_bits = std::ptr::read(ptr.add(self.sp - 1) as *const u64);
            let a_bits = std::ptr::read(ptr.add(self.sp - 2) as *const u64);
            !shape_value::tags::is_tagged(a_bits) && !shape_value::tags::is_tagged(b_bits)
        }
    }

    /// Check whether the top stack value is a plain f64 (not tagged).
    #[inline(always)]
    pub(crate) fn stack_top_is_f64(&self) -> bool {
        if self.sp == 0 {
            return false;
        }
        unsafe {
            let bits = std::ptr::read(self.stack.as_ptr().add(self.sp - 1) as *const u64);
            !shape_value::tags::is_tagged(bits)
        }
    }

    /// Check whether the top stack value is a NaN-boxed bool.
    #[inline(always)]
    pub(crate) fn stack_top_is_bool(&self) -> bool {
        if self.sp == 0 {
            return false;
        }
        unsafe {
            let bits = std::ptr::read(self.stack.as_ptr().add(self.sp - 1) as *const u64);
            shape_value::tags::is_tagged(bits)
                && shape_value::tags::get_tag(bits) == shape_value::tags::TAG_BOOL
        }
    }

    // --- Raw typed stack ops (zero NaN-box overhead) ---
    //
    // These bypass ValueWord construction/destruction entirely.  The caller
    // (typed opcodes) guarantees the value on the stack has the declared type,
    // so we can reinterpret the raw u64 bits directly.

    /// Pop an i64 from the stack, assuming the top-of-stack is a NaN-boxed i48.
    ///
    /// Reads the raw u64 bits, sign-extends the 48-bit payload to i64, and
    /// writes a None sentinel over the slot.  No ValueWord Clone/Drop overhead.
    ///
    /// # Safety contract
    /// The compiler must have proven the value is `int` (i48 inline).
    /// If the value is not i48-tagged, the result is garbage (but memory-safe).
    #[inline(always)]
    pub(crate) fn pop_raw_i64(&mut self) -> Result<i64, VMError> {
        if self.sp == 0 {
            return Err(VMError::StackUnderflow);
        }
        self.sp -= 1;
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp);
            let bits = std::ptr::read(ptr as *const u64);
            // Write None sentinel (non-heap, so no Drop needed on previous value
            // since we know it was an inline i48).
            std::ptr::write(ptr as *mut u64, 0xFFFB_0000_0000_0000u64);
            Ok(shape_value::tags::sign_extend_i48(
                shape_value::tags::get_payload(bits),
            ))
        }
    }

    /// Push an i64 as a NaN-boxed i48 value.
    ///
    /// Values must be in the i48 range [-2^47, 2^47-1].
    /// Writes the raw tagged u64 directly into the stack slot.
    #[inline(always)]
    pub(crate) fn push_raw_i64(&mut self, value: i64) -> Result<(), VMError> {
        if self.sp >= self.stack.len() {
            // Cold path: grow via push_vw_slow
            return self.push_vw(ValueWord::from_i64(value));
        }
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp) as *mut u64;
            // Construct tagged i48 inline: TAG_BASE | (TAG_INT << 48) | (payload & PAYLOAD_MASK)
            let payload = (value as u64) & shape_value::tags::PAYLOAD_MASK;
            let bits = shape_value::tags::make_tagged(shape_value::tags::TAG_INT, payload);
            std::ptr::write(ptr, bits);
        }
        self.sp += 1;
        Ok(())
    }

    /// Pop an f64 from the stack, assuming the top-of-stack is a plain f64.
    ///
    /// Reads the raw u64 bits and reinterprets as f64.  No ValueWord overhead.
    ///
    /// # Safety contract
    /// The compiler must have proven the value is `number` (plain f64).
    /// If the value is tagged (not a plain f64), the result is garbage (but memory-safe).
    #[inline(always)]
    pub(crate) fn pop_raw_f64(&mut self) -> Result<f64, VMError> {
        if self.sp == 0 {
            return Err(VMError::StackUnderflow);
        }
        self.sp -= 1;
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp);
            let bits = std::ptr::read(ptr as *const u64);
            // Write None sentinel (the previous value was a plain f64, no heap Drop).
            std::ptr::write(ptr as *mut u64, 0xFFFB_0000_0000_0000u64);
            Ok(f64::from_bits(bits))
        }
    }

    /// Pop a raw u64 from the stack with no interpretation.
    ///
    /// This is used by v2 typed handlers that store raw native pointers / values
    /// directly in stack slots, bypassing ValueWord semantics entirely.  No
    /// Drop is run on the popped slot — the caller owns the bits.
    ///
    /// # Safety contract
    /// The slot must contain a value placed by a v2 raw push (not a heap-tagged
    /// ValueWord), or the caller must be otherwise aware that no Arc refcount
    /// is being released.
    #[inline(always)]
    pub(crate) fn pop_raw_u64(&mut self) -> Result<u64, VMError> {
        if self.sp == 0 {
            return Err(VMError::StackUnderflow);
        }
        self.sp -= 1;
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp);
            let bits = std::ptr::read(ptr as *const u64);
            std::ptr::write(ptr as *mut u64, 0xFFFB_0000_0000_0000u64);
            Ok(bits)
        }
    }

    /// Push a raw u64 onto the stack with no NaN-box tagging.
    ///
    /// Companion to `pop_raw_u64` — used by v2 typed handlers that store
    /// raw native pointers / values in stack slots.
    #[inline(always)]
    pub(crate) fn push_raw_u64(&mut self, bits: u64) -> Result<(), VMError> {
        if self.sp >= self.stack.len() {
            return self.push_vw_slow(bits);
        }
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp) as *mut u64;
            std::ptr::write(ptr, bits);
        }
        self.sp += 1;
        Ok(())
    }

    /// Pop a bool from the stack, assuming the top-of-stack is a NaN-boxed bool.
    ///
    /// Reads the raw u64 bits and decodes the bool payload.  No ValueWord overhead.
    ///
    /// # Safety contract
    /// The compiler must have proven the value is `bool`.
    /// If the value is not bool-tagged, the result is garbage (but memory-safe).
    #[inline(always)]
    pub(crate) fn pop_raw_bool(&mut self) -> Result<bool, VMError> {
        if self.sp == 0 {
            return Err(VMError::StackUnderflow);
        }
        self.sp -= 1;
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp);
            let bits = std::ptr::read(ptr as *const u64);
            // Write None sentinel — bool is inline, no heap Drop needed.
            std::ptr::write(ptr as *mut u64, 0xFFFB_0000_0000_0000u64);
            // Bool payload is bit 0; nonzero = true.
            Ok(shape_value::tags::get_payload(bits) != 0)
        }
    }

    /// Push a bool directly onto the stack as a NaN-boxed bool tag.
    ///
    /// Writes the raw tagged u64 directly into the stack slot — no ValueWord
    /// construction/Drop overhead.
    #[inline(always)]
    pub(crate) fn push_raw_bool(&mut self, value: bool) -> Result<(), VMError> {
        if self.sp >= self.stack.len() {
            // Cold path: grow via push_vw_slow
            return self.push_vw(ValueWord::from_bool(value));
        }
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp) as *mut u64;
            let bits = shape_value::tags::make_tagged(shape_value::tags::TAG_BOOL, value as u64);
            std::ptr::write(ptr, bits);
        }
        self.sp += 1;
        Ok(())
    }

    pub(crate) fn push_raw_f64(&mut self, value: f64) -> Result<(), VMError> {
        if self.sp >= self.stack.len() {
            // Cold path: grow via push_vw_slow
            return self.push_vw(ValueWord::from_f64(value));
        }
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp) as *mut u64;
            let bits = if value.is_nan() {
                shape_value::tags::CANONICAL_NAN
            } else {
                value.to_bits()
            };
            std::ptr::write(ptr, bits);
        }
        self.sp += 1;
        Ok(())
    }

    // ===== Hash and frame helpers =====

    pub(crate) fn blob_hash_for_function(&self, func_id: u16) -> Option<FunctionHash> {
        self.function_hashes
            .get(func_id as usize)
            .copied()
            .flatten()
    }

    pub(crate) fn current_locals_base(&self) -> usize {
        self.call_stack
            .last()
            .map(|frame| frame.base_pointer)
            .unwrap_or(0)
    }

    /// Wave C Phase C1: Look up the FrameDescriptor for the currently executing
    /// function. Returns None if no call frame is active or the active function
    /// has no FrameDescriptor (legacy bytecode).
    ///
    /// Used by typed handlers (`op_load_local_trusted`, etc.) to skip ValueWord
    /// wrapping when the slot kind is a known scalar.
    #[inline]
    pub(crate) fn current_frame_descriptor(&self) -> Option<&crate::type_tracking::FrameDescriptor> {
        let func_id = self.call_stack.last()?.function_id?;
        let func = self.program.functions.get(func_id as usize)?;
        func.frame_descriptor.as_ref()
    }
}

#[cfg(test)]
mod raw_stack_tests {
    use super::*;
    use crate::executor::VMConfig;

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    // ----- Bool round-trip -----

    #[test]
    fn raw_bool_round_trip_true() {
        let mut vm = make_vm();
        vm.push_raw_bool(true).unwrap();
        assert!(vm.stack_top_is_bool());
        let v = vm.pop_raw_bool().unwrap();
        assert_eq!(v, true);
    }

    #[test]
    fn raw_bool_round_trip_false() {
        let mut vm = make_vm();
        vm.push_raw_bool(false).unwrap();
        assert!(vm.stack_top_is_bool());
        let v = vm.pop_raw_bool().unwrap();
        assert_eq!(v, false);
    }

    #[test]
    fn raw_bool_compatible_with_vw_pop() {
        // pop_vw on a raw_bool slot must materialize a bool ValueWord
        let mut vm = make_vm();
        vm.push_raw_bool(true).unwrap();
        let vw = vm.pop_vw().unwrap();
        assert!(vw.is_bool());
        assert_eq!(unsafe { vw.as_bool_unchecked() }, true);
    }

    #[test]
    fn vw_bool_compatible_with_raw_pop() {
        // push_vw of a bool followed by pop_raw_bool must yield same value
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_bool(true)).unwrap();
        assert!(vm.stack_top_is_bool());
        assert_eq!(vm.pop_raw_bool().unwrap(), true);
    }

    // ----- f64 round-trip including NaN -----

    #[test]
    fn raw_f64_round_trip_nan() {
        let mut vm = make_vm();
        vm.push_raw_f64(f64::NAN).unwrap();
        // After push, the bits are CANONICAL_NAN (canonicalized to prevent
        // collisions with the tagged range). pop_raw_f64 reinterprets as f64.
        let v = vm.pop_raw_f64().unwrap();
        assert!(v.is_nan(), "expected NaN, got {}", v);
    }

    #[test]
    fn raw_f64_round_trip_neg_zero() {
        let mut vm = make_vm();
        vm.push_raw_f64(-0.0).unwrap();
        let v = vm.pop_raw_f64().unwrap();
        // -0.0.to_bits() != 0.0.to_bits() but -0.0 == 0.0 numerically
        assert_eq!(v, -0.0);
        assert_eq!(v.to_bits(), (-0.0_f64).to_bits());
    }

    #[test]
    fn raw_f64_round_trip_infinity() {
        let mut vm = make_vm();
        vm.push_raw_f64(f64::INFINITY).unwrap();
        let v = vm.pop_raw_f64().unwrap();
        assert_eq!(v, f64::INFINITY);
    }

    // ----- i64 round-trip including i48 boundary -----

    #[test]
    fn raw_i64_round_trip_max_i48() {
        const I48_MAX: i64 = (1i64 << 47) - 1;
        let mut vm = make_vm();
        vm.push_raw_i64(I48_MAX).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), I48_MAX);
    }

    #[test]
    fn raw_i64_round_trip_min_i48() {
        const I48_MIN: i64 = -(1i64 << 47);
        let mut vm = make_vm();
        vm.push_raw_i64(I48_MIN).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), I48_MIN);
    }

    #[test]
    fn raw_i64_round_trip_negative() {
        let mut vm = make_vm();
        vm.push_raw_i64(-12345).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), -12345);
    }

    // ----- Underflow -----

    #[test]
    fn raw_pop_underflows() {
        let mut vm = make_vm();
        assert!(vm.pop_raw_bool().is_err());
        assert!(vm.pop_raw_i64().is_err());
        assert!(vm.pop_raw_f64().is_err());
        assert!(vm.pop_raw_u64().is_err());
    }

    // ----- u64 round-trip (raw, no NaN-box semantics) -----

    #[test]
    fn raw_u64_round_trip_arbitrary_bits() {
        let mut vm = make_vm();
        let bits: u64 = 0x12345678_9abcdef0;
        vm.push_raw_u64(bits).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap(), bits);
    }
}
