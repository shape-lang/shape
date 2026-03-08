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

    // --- ValueWord-direct stack ops for hot paths ---

    /// Push a ValueWord value directly (no ValueWord conversion).
    ///
    /// Hot path: single bounds check + write.  The stack growth and overflow
    /// checks are split into a cold `push_vw_slow` to keep the hot path tight.
    #[inline(always)]
    pub(crate) fn push_vw(&mut self, value: ValueWord) -> Result<(), VMError> {
        if self.sp >= self.stack.len() {
            return self.push_vw_slow(value);
        }
        self.stack[self.sp] = value;
        self.sp += 1;
        Ok(())
    }

    /// Cold path for push_vw: grow the stack or return StackOverflow.
    #[cold]
    #[inline(never)]
    pub(crate) fn push_vw_slow(&mut self, value: ValueWord) -> Result<(), VMError> {
        if self.sp >= self.config.max_stack_size {
            return Err(VMError::StackOverflow);
        }
        let new_len = self.sp * 2 + 1;
        self.stack.reserve(new_len - self.stack.len());
        while self.stack.len() < new_len {
            self.stack.push(ValueWord::none());
        }
        self.stack[self.sp] = value;
        self.sp += 1;
        Ok(())
    }

    /// Pop a ValueWord value directly (no ValueWord conversion).
    ///
    /// Uses `ptr::read` to take ownership of the value, then writes a
    /// ValueWord::none() sentinel via raw pointer to prevent double-free on
    /// Vec drop — avoiding bounds checks and the full `mem::replace` protocol.
    ///
    /// The underflow check is retained for safety but marked cold so the
    /// branch predictor always predicts the fast path (sp > 0).
    #[inline(always)]
    pub(crate) fn pop_vw(&mut self) -> Result<ValueWord, VMError> {
        if self.sp == 0 {
            return Self::pop_vw_underflow();
        }
        self.sp -= 1;
        // SAFETY: sp was > 0 before decrement, so self.sp is a valid index
        // into self.stack (which is pre-allocated to at least DEFAULT_STACK_CAPACITY).
        // We take ownership via ptr::read and immediately overwrite the slot with
        // a None sentinel so the Vec destructor won't double-free any heap ValueWord.
        unsafe {
            let ptr = self.stack.as_mut_ptr().add(self.sp);
            let val = std::ptr::read(ptr);
            // Write ValueWord::none() bit pattern directly. This is TAG_BASE | (TAG_NONE << 48)
            // = 0xFFFB_0000_0000_0000. It's a non-heap tagged value so Drop is a no-op.
            std::ptr::write(ptr as *mut u64, 0xFFFB_0000_0000_0000u64);
            Ok(val)
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_vw_underflow() -> Result<ValueWord, VMError> {
        Err(VMError::StackUnderflow)
    }

    /// Pop and materialize a ValueWord from the stack (convenience for tests and legacy callers).
    pub fn pop(&mut self) -> Result<ValueWord, VMError> {
        Ok(self.pop_vw()?.clone())
    }

    // ===== Hash and frame helpers =====

    pub(crate) fn blob_hash_for_function(&self, func_id: u16) -> Option<FunctionHash> {
        self.function_hashes
            .get(func_id as usize)
            .copied()
            .flatten()
    }

    #[inline]
    pub(crate) fn function_id_for_blob_hash(&self, hash: FunctionHash) -> Option<u16> {
        self.function_id_by_hash.get(&hash).copied()
    }

    pub(crate) fn current_locals_base(&self) -> usize {
        self.call_stack
            .last()
            .map(|frame| frame.base_pointer)
            .unwrap_or(0)
    }
}
