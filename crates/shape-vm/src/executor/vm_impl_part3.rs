use super::*;

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
    pub(super) fn push_vw_slow(&mut self, value: ValueWord) -> Result<(), VMError> {
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
    pub(super) fn pop_vw(&mut self) -> Result<ValueWord, VMError> {
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
    pub(super) fn pop_vw_underflow() -> Result<ValueWord, VMError> {
        Err(VMError::StackUnderflow)
    }

    /// Pop and materialize a ValueWord from the stack (convenience for tests and legacy callers).
    pub fn pop(&mut self) -> Result<ValueWord, VMError> {
        Ok(self.pop_vw()?.clone())
    }

    // ===== Builtin Dispatch =====

    pub(super) fn op_builtin_call(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        if let Some(Operand::Builtin(builtin)) = instruction.operand {
            let mut ctx = ctx;
            match builtin {
                // Math builtins (15)
                BuiltinFunction::Abs => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_abs(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Sqrt => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_sqrt(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Ln => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_ln(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Pow => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_pow(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Exp => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_exp(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Log => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_log(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Floor => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_floor(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Ceil => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_ceil(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Round => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_round(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Sin => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_sin(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Cos => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_cos(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Tan => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_tan(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Asin => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_asin(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Acos => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_acos(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Atan => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_atan(args)?;
                    self.push_vw(result)?;
                }
                // Stats builtins (3)
                BuiltinFunction::Min => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_min(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Max => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_max(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::StdDev => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_stddev(args)?;
                    self.push_vw(result)?;
                }
                // Array builtins (6)
                BuiltinFunction::Push => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_push(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Pop => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_pop(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::First => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_first(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Last => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_last(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Zip => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_zip(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Len => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_len(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Filled => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_filled(args)?;
                    self.push_vw(result)?;
                }
                // Utility builtins (2)
                BuiltinFunction::Format => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_format(args)?;
                    self.push_vw(result)?;
                }
                // BuiltinFunction::Throw removed: Shape uses Result types
                BuiltinFunction::Range => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_range(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Slice => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_slice(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Map => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_map(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Filter => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_filter(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Reduce => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_reduce(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ForEach => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_for_each(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Find => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_find(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::FindIndex => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_find_index(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Some => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_some(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Every => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_every(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Print => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_print(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Snapshot => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_snapshot(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Exit => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_exit(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ObjectRest => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_object_rest(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsNumber => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_number(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsString => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_string(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsBool => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_bool(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsArray => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_array(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsObject => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_object(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsDataRow => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_data_row(args)?;
                    self.push_vw(result)?;
                }
                b @ (BuiltinFunction::ToString
                | BuiltinFunction::ToNumber
                | BuiltinFunction::ToBool
                | BuiltinFunction::IntoInt
                | BuiltinFunction::IntoNumber
                | BuiltinFunction::IntoDecimal
                | BuiltinFunction::IntoBool
                | BuiltinFunction::IntoString
                | BuiltinFunction::TryIntoInt
                | BuiltinFunction::TryIntoNumber
                | BuiltinFunction::TryIntoDecimal
                | BuiltinFunction::TryIntoBool
                | BuiltinFunction::TryIntoString) => {
                    let args = self.pop_builtin_args()?;
                    let result = self.dispatch_conversion_builtin(b, args)?;
                    self.push_vw(result)?;
                }
                b @ (BuiltinFunction::NativePtrSize
                | BuiltinFunction::NativePtrNewCell
                | BuiltinFunction::NativePtrFreeCell
                | BuiltinFunction::NativePtrReadPtr
                | BuiltinFunction::NativePtrWritePtr
                | BuiltinFunction::NativeTableFromArrowC
                | BuiltinFunction::NativeTableFromArrowCTyped
                | BuiltinFunction::NativeTableBindType) => {
                    let args = self.pop_builtin_args()?;
                    let result = self.dispatch_native_interop_builtin(b, args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::FormatValueWithMeta => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_format_with_meta(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::FormatValueWithSpec => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_format_with_spec(args, ctx)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::TypeOf => {
                    let args: Vec<ValueWord> = vec![]; // TypeOf uses self.pop_vw() internally
                    let result = self.builtin_type_of(args)?;
                    self.push_vw(result)?;
                }
                b @ (BuiltinFunction::IntrinsicVecAbs
                | BuiltinFunction::IntrinsicVecSqrt
                | BuiltinFunction::IntrinsicVecLn
                | BuiltinFunction::IntrinsicVecExp
                | BuiltinFunction::IntrinsicVecAdd
                | BuiltinFunction::IntrinsicVecSub
                | BuiltinFunction::IntrinsicVecMul
                | BuiltinFunction::IntrinsicVecDiv
                | BuiltinFunction::IntrinsicVecMax
                | BuiltinFunction::IntrinsicVecMin
                | BuiltinFunction::IntrinsicVecSelect) => {
                    return self.handle_vector_intrinsic(b, ctx.as_deref_mut());
                }
                b @ (BuiltinFunction::IntrinsicMatMulVec | BuiltinFunction::IntrinsicMatMulMat) => {
                    return self.handle_matrix_intrinsic(b, ctx.as_deref_mut());
                }
                BuiltinFunction::SomeCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_some_ctor(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::OkCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_ok_ctor(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ErrCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_err_ctor(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::HashMapCtor => {
                    let _args = self.pop_builtin_args()?;
                    self.push_vw(ValueWord::empty_hashmap())?;
                }
                BuiltinFunction::SetCtor => {
                    let args = self.pop_builtin_args()?;
                    if args.is_empty() {
                        self.push_vw(ValueWord::empty_set())?;
                    } else if args.len() == 1 {
                        // Set(array) — initialize from array
                        if let Some(arr) = args[0].as_array() {
                            self.push_vw(ValueWord::from_set(arr.to_vec()))?;
                        } else {
                            // Single non-array item — wrap in set
                            self.push_vw(ValueWord::from_set(vec![args[0].clone()]))?;
                        }
                    } else {
                        // Set(a, b, c) — multiple args become set items
                        self.push_vw(ValueWord::from_set(args))?;
                    }
                }
                BuiltinFunction::DequeCtor => {
                    let args = self.pop_builtin_args()?;
                    if args.is_empty() {
                        self.push_vw(ValueWord::empty_deque())?;
                    } else if args.len() == 1 {
                        // Deque(array) — initialize from array
                        if let Some(arr) = args[0].as_array() {
                            self.push_vw(ValueWord::from_deque(arr.to_vec()))?;
                        } else {
                            // Single non-array item
                            self.push_vw(ValueWord::from_deque(vec![args[0].clone()]))?;
                        }
                    } else {
                        // Deque(a, b, c)
                        self.push_vw(ValueWord::from_deque(args))?;
                    }
                }
                BuiltinFunction::PriorityQueueCtor => {
                    let args = self.pop_builtin_args()?;
                    if args.is_empty() {
                        self.push_vw(ValueWord::empty_priority_queue())?;
                    } else if args.len() == 1 {
                        if let Some(arr) = args[0].as_array() {
                            self.push_vw(ValueWord::from_priority_queue(arr.to_vec()))?;
                        } else {
                            self.push_vw(ValueWord::from_priority_queue(vec![args[0].clone()]))?;
                        }
                    } else {
                        self.push_vw(ValueWord::from_priority_queue(args))?;
                    }
                }
                BuiltinFunction::ControlFold => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_control_fold(args, ctx)?;
                    self.push_vw(result)?;
                }
                // Delegate ALL intrinsics to helper method
                b @ (BuiltinFunction::IntrinsicSum
                | BuiltinFunction::IntrinsicMean
                | BuiltinFunction::IntrinsicMin
                | BuiltinFunction::IntrinsicMax
                | BuiltinFunction::IntrinsicStd
                | BuiltinFunction::IntrinsicVariance
                | BuiltinFunction::IntrinsicRandom
                | BuiltinFunction::IntrinsicRandomInt
                | BuiltinFunction::IntrinsicRandomSeed
                | BuiltinFunction::IntrinsicRandomNormal
                | BuiltinFunction::IntrinsicRandomArray
                | BuiltinFunction::IntrinsicDistUniform
                | BuiltinFunction::IntrinsicDistLognormal
                | BuiltinFunction::IntrinsicDistExponential
                | BuiltinFunction::IntrinsicDistPoisson
                | BuiltinFunction::IntrinsicDistSampleN
                | BuiltinFunction::IntrinsicBrownianMotion
                | BuiltinFunction::IntrinsicGbm
                | BuiltinFunction::IntrinsicOuProcess
                | BuiltinFunction::IntrinsicRandomWalk
                | BuiltinFunction::IntrinsicRollingSum
                | BuiltinFunction::IntrinsicRollingMean
                | BuiltinFunction::IntrinsicRollingStd
                | BuiltinFunction::IntrinsicRollingMin
                | BuiltinFunction::IntrinsicRollingMax
                | BuiltinFunction::IntrinsicEma
                | BuiltinFunction::IntrinsicLinearRecurrence
                | BuiltinFunction::IntrinsicShift
                | BuiltinFunction::IntrinsicDiff
                | BuiltinFunction::IntrinsicPctChange
                | BuiltinFunction::IntrinsicFillna
                | BuiltinFunction::IntrinsicCumsum
                | BuiltinFunction::IntrinsicCumprod
                | BuiltinFunction::IntrinsicClip
                | BuiltinFunction::IntrinsicCorrelation
                | BuiltinFunction::IntrinsicCovariance
                | BuiltinFunction::IntrinsicPercentile
                | BuiltinFunction::IntrinsicMedian
                | BuiltinFunction::IntrinsicCharCode
                | BuiltinFunction::IntrinsicFromCharCode
                | BuiltinFunction::IntrinsicSeries) => {
                    return self.handle_intrinsic_builtin(b, ctx.as_deref_mut());
                }
                BuiltinFunction::EvalTimeRef => {
                    return Err(VMError::NotImplemented(
                        "eval_time_ref() (VM-only mode)".to_string(),
                    ));
                }
                BuiltinFunction::EvalDateTimeExpr => {
                    return self.handle_eval_datetime_expr(ctx);
                }
                BuiltinFunction::EvalDataDateTimeRef => {
                    // DataReference type removed - this operation is no longer supported
                    return Err(VMError::RuntimeError(
                        "DataReference type has been removed".to_string(),
                    ));
                }
                BuiltinFunction::EvalDataSet => {
                    // DataRow type removed - this operation is no longer supported
                    return Err(VMError::RuntimeError(
                        "DataRow type has been removed".to_string(),
                    ));
                }
                BuiltinFunction::EvalDataRelative => {
                    // DataReference type removed - this operation is no longer supported
                    return Err(VMError::RuntimeError(
                        "DataReference type has been removed".to_string(),
                    ));
                }
                BuiltinFunction::EvalDataRelativeRange => {
                    // DataReference type removed - this operation is no longer supported
                    return Err(VMError::RuntimeError(
                        "DataReference type has been removed".to_string(),
                    ));
                }

                // Window functions - these delegate to the runtime WindowExecutor
                // In VM mode, window functions are evaluated differently than in JIT
                b @ (BuiltinFunction::WindowRowNumber
                | BuiltinFunction::WindowRank
                | BuiltinFunction::WindowDenseRank
                | BuiltinFunction::WindowNtile
                | BuiltinFunction::WindowLag
                | BuiltinFunction::WindowLead
                | BuiltinFunction::WindowFirstValue
                | BuiltinFunction::WindowLastValue
                | BuiltinFunction::WindowNthValue
                | BuiltinFunction::WindowSum
                | BuiltinFunction::WindowAvg
                | BuiltinFunction::WindowMin
                | BuiltinFunction::WindowMax
                | BuiltinFunction::WindowCount) => {
                    return self.handle_window_functions(b);
                }

                // JOIN operation
                BuiltinFunction::JoinExecute => {
                    return self.handle_join_execute();
                }

                // Reflection
                BuiltinFunction::Reflect => {
                    return self.builtin_reflect();
                }

                // Content string builtins
                BuiltinFunction::MakeContentText => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_make_content_text(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::MakeContentFragment => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_make_content_fragment(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ApplyContentStyle => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_apply_content_style(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::MakeContentChartFromValue => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_make_content_chart_from_value(args)?;
                    self.push_vw(result)?;
                }

                // Content namespace constructors
                BuiltinFunction::ContentChart => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_chart(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ContentTextCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_text(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ContentTableCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_table(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ContentCodeCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_code(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ContentKvCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_kv(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::ContentFragmentCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_fragment(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_vw(result)?;
                }

                // DateTime constructor builtins
                BuiltinFunction::DateTimeNow => {
                    let result = ValueWord::from_time(chrono::Local::now().fixed_offset());
                    self.push_vw(result)?;
                }
                BuiltinFunction::DateTimeUtc => {
                    let result = ValueWord::from_time_utc(chrono::Utc::now());
                    self.push_vw(result)?;
                }
                BuiltinFunction::DateTimeParse => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_datetime_parse(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::DateTimeFromEpoch => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_datetime_from_epoch(args)?;
                    self.push_vw(result)?;
                }

                // Concurrency primitive constructors
                BuiltinFunction::MutexCtor => {
                    let args = self.pop_builtin_args()?;
                    let inner_value = args.into_iter().next().unwrap_or_else(ValueWord::none);
                    self.push_vw(ValueWord::from_mutex(inner_value))?;
                }
                BuiltinFunction::AtomicCtor => {
                    let args = self.pop_builtin_args()?;
                    let init_val = args.first().and_then(|nb| nb.as_i64()).unwrap_or(0);
                    self.push_vw(ValueWord::from_atomic(init_val))?;
                }
                BuiltinFunction::LazyCtor => {
                    let args = self.pop_builtin_args()?;
                    let initializer = args.into_iter().next().unwrap_or_else(ValueWord::none);
                    self.push_vw(ValueWord::from_lazy(initializer))?;
                }
                BuiltinFunction::ChannelCtor => {
                    let _args = self.pop_builtin_args()?;
                    let (sender, receiver) = shape_value::heap_value::ChannelData::new_pair();
                    let arr = vec![
                        ValueWord::from_channel(sender),
                        ValueWord::from_channel(receiver),
                    ];
                    self.push_vw(ValueWord::from_array(std::sync::Arc::new(arr)))?;
                }

                // Additional math builtins
                BuiltinFunction::Sign => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_sign(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Gcd => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_gcd(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Lcm => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_lcm(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Hypot => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_hypot(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::Clamp => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_clamp(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsNaN => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_nan(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::IsFinite => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_finite(args)?;
                    self.push_vw(result)?;
                }

                // Table construction
                BuiltinFunction::MakeTableFromRows => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_make_table_from_rows(args)?;
                    self.push_vw(result)?;
                }

                // Json navigation helpers
                BuiltinFunction::JsonObjectGet => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_object_get(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::JsonArrayAt => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_array_at(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::JsonObjectKeys => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_object_keys(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::JsonArrayLen => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_array_len(args)?;
                    self.push_vw(result)?;
                }
                BuiltinFunction::JsonObjectLen => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_object_len(args)?;
                    self.push_vw(result)?;
                }
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    // Runtime bridge functions (pop_builtin_args, eval_runtime_*) moved to builtins/runtime_bridge.rs
    // map_runtime_error and type_of_name moved to module_registry module

    // ===== Exception Handling Operations =====
    // handle_exception moved to exceptions/mod.rs

    // ===== Slice and Null Coalescing Operations =====

    // ===== Loop Control Operations =====

    // ===== Helper Methods =====
    // binary_arithmetic, eval_runtime_binary_op_value, binary_comparison moved to arithmetic/mod.rs
}
