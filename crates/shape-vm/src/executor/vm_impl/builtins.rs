use super::super::*;
use shape_value::ValueWordExt;

impl VirtualMachine {
    pub(crate) fn pop_builtin_args(&mut self) -> Result<Vec<ValueWord>, VMError> {
        // Pop arg count (top of stack)
        let count_nb = self.pop_raw_u64()?;
        let count = count_nb.as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Expected numeric arg count, got {:?}",
                count_nb.type_name()
            ))
        })? as usize;

        // Pop args in reverse order (stack is LIFO) then reverse to get correct order
        let mut args = Vec::with_capacity(count);
        for _ in 0..count {
            args.push(self.pop_raw_u64()?);
        }
        args.reverse();
        Ok(args)
    }

    // ========================================================================
    // Builtin Dispatch

    pub fn op_builtin_call(
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
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Sqrt => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_sqrt(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Ln => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_ln(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Pow => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_pow(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Exp => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_exp(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Log => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_log(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Floor => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_floor(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Ceil => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_ceil(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Round => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_round(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Sin => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_sin(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Cos => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_cos(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Tan => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_tan(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Asin => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_asin(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Acos => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_acos(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Atan => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_atan(args)?;
                    self.push_raw_u64(result)?;
                }
                // Stats builtins (3)
                BuiltinFunction::Min => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_min(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Max => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_max(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::StdDev => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_stddev(args)?;
                    self.push_raw_u64(result)?;
                }
                // Array builtins (6)
                BuiltinFunction::Push => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_push(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Pop => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_pop(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::First => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_first(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Last => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_last(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Zip => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_zip(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Len => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_len(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Filled => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_filled(args)?;
                    self.push_raw_u64(result)?;
                }
                // Utility builtins (2)
                BuiltinFunction::Format => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_format(args)?;
                    self.push_raw_u64(result)?;
                }
                // BuiltinFunction::Throw removed: Shape uses Result types
                BuiltinFunction::Range => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_range(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Slice => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_slice(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Map => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_map(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Filter => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_filter(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Reduce => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_reduce(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ForEach => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_for_each(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Find => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_find(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::FindIndex => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_find_index(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Some => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_some(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Every => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_every(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Print => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_print(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Snapshot => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_snapshot(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Exit => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_exit(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ObjectRest => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_object_rest(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsNumber => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_number(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsString => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_string(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsBool => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_bool(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsArray => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_array(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsObject => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_object(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsDataRow => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_data_row(args)?;
                    self.push_raw_u64(result)?;
                }
                b @ (BuiltinFunction::ToString
                | BuiltinFunction::ToNumber
                | BuiltinFunction::ToBool) => {
                    let args = self.pop_builtin_args()?;
                    let result = self.dispatch_conversion_builtin(b, args)?;
                    self.push_raw_u64(result)?;
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
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::FormatValueWithMeta => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_format_with_meta(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::FormatValueWithSpec => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_format_with_spec(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::TypeOf => {
                    let args: Vec<ValueWord> = vec![]; // TypeOf uses self.pop_raw_u64() internally
                    let result = self.builtin_type_of(args)?;
                    self.push_raw_u64(result)?;
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
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::OkCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_ok_ctor(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ErrCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_err_ctor(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::HashMapCtor => {
                    let _args = self.pop_builtin_args()?;
                    self.push_raw_u64(ValueWord::empty_hashmap())?;
                }
                BuiltinFunction::SetCtor => {
                    let args = self.pop_builtin_args()?;
                    if args.is_empty() {
                        self.push_raw_u64(ValueWord::empty_set())?;
                    } else if args.len() == 1 {
                        // Set(array) — initialize from array
                        if let Some(arr) = args[0].as_any_array() {
                            let items = std::sync::Arc::try_unwrap(arr.to_generic()).unwrap_or_else(|a| (*a).clone());
                            self.push_raw_u64(ValueWord::from_set(items))?;
                        } else {
                            // Single non-array item — wrap in set
                            self.push_raw_u64(ValueWord::from_set(vec![args[0].clone()]))?;
                        }
                    } else {
                        // Set(a, b, c) — multiple args become set items
                        self.push_raw_u64(ValueWord::from_set(args))?;
                    }
                }
                BuiltinFunction::DequeCtor => {
                    let args = self.pop_builtin_args()?;
                    if args.is_empty() {
                        self.push_raw_u64(ValueWord::empty_deque())?;
                    } else if args.len() == 1 {
                        // Deque(array) — initialize from array
                        if let Some(arr) = args[0].as_any_array() {
                            let items = std::sync::Arc::try_unwrap(arr.to_generic()).unwrap_or_else(|a| (*a).clone());
                            self.push_raw_u64(ValueWord::from_deque(items))?;
                        } else {
                            // Single non-array item
                            self.push_raw_u64(ValueWord::from_deque(vec![args[0].clone()]))?;
                        }
                    } else {
                        // Deque(a, b, c)
                        self.push_raw_u64(ValueWord::from_deque(args))?;
                    }
                }
                BuiltinFunction::PriorityQueueCtor => {
                    let args = self.pop_builtin_args()?;
                    if args.is_empty() {
                        self.push_raw_u64(ValueWord::empty_priority_queue())?;
                    } else if args.len() == 1 {
                        if let Some(arr) = args[0].as_any_array() {
                            let items = std::sync::Arc::try_unwrap(arr.to_generic()).unwrap_or_else(|a| (*a).clone());
                            self.push_raw_u64(ValueWord::from_priority_queue(items))?;
                        } else {
                            self.push_raw_u64(ValueWord::from_priority_queue(vec![args[0].clone()]))?;
                        }
                    } else {
                        self.push_raw_u64(ValueWord::from_priority_queue(args))?;
                    }
                }
                BuiltinFunction::ControlFold => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_control_fold(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IntrinsicMinimize => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_minimize(args, ctx)?;
                    self.push_raw_u64(result)?;
                }
                // Delegate ALL intrinsics to helper method
                b @ (BuiltinFunction::IntrinsicBspline2_3dBatch
                | BuiltinFunction::IntrinsicSum
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
                | BuiltinFunction::IntrinsicAtan2
                | BuiltinFunction::IntrinsicSinh
                | BuiltinFunction::IntrinsicCosh
                | BuiltinFunction::IntrinsicTanh
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
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::MakeContentFragment => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_make_content_fragment(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ApplyContentStyle => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_apply_content_style(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::MakeContentChartFromValue => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_make_content_chart_from_value(args)?;
                    self.push_raw_u64(result)?;
                }

                // Content namespace constructors
                BuiltinFunction::ContentChart => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_chart(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ContentTextCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_text(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ContentTableCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_table(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ContentCodeCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_code(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ContentKvCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_kv(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::ContentFragmentCtor => {
                    let args = self.pop_builtin_args()?;
                    let result = shape_runtime::content_builders::content_fragment(&args)
                        .map_err(|e| VMError::RuntimeError(format!("{}", e)))?;
                    self.push_raw_u64(result)?;
                }

                // DateTime constructor builtins
                BuiltinFunction::DateTimeNow => {
                    let result = ValueWord::from_time(chrono::Local::now().fixed_offset());
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::DateTimeUtc => {
                    let result = ValueWord::from_time_utc(chrono::Utc::now());
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::DateTimeParse => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_datetime_parse(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::DateTimeFromEpoch => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_datetime_from_epoch(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::DateTimeFromParts => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_datetime_from_parts(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::DateTimeFromUnixSecs => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_datetime_from_unix_secs(args)?;
                    self.push_raw_u64(result)?;
                }

                // Concurrency primitive constructors
                BuiltinFunction::MutexCtor => {
                    let args = self.pop_builtin_args()?;
                    let inner_value = args.into_iter().next().unwrap_or_else(ValueWord::none);
                    self.push_raw_u64(ValueWord::from_mutex(inner_value))?;
                }
                BuiltinFunction::AtomicCtor => {
                    let args = self.pop_builtin_args()?;
                    let init_val = args.first().and_then(|nb| nb.as_i64()).unwrap_or(0);
                    self.push_raw_u64(ValueWord::from_atomic(init_val))?;
                }
                BuiltinFunction::LazyCtor => {
                    let args = self.pop_builtin_args()?;
                    let initializer = args.into_iter().next().unwrap_or_else(ValueWord::none);
                    self.push_raw_u64(ValueWord::from_lazy(initializer))?;
                }
                BuiltinFunction::ChannelCtor => {
                    let _args = self.pop_builtin_args()?;
                    let (sender, receiver) = shape_value::heap_value::ChannelData::new_pair();
                    let arr = vec![
                        ValueWord::from_channel(sender),
                        ValueWord::from_channel(receiver),
                    ];
                    self.push_raw_u64(ValueWord::from_array(std::sync::Arc::new(arr)))?;
                }

                // Additional math builtins
                BuiltinFunction::Sign => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_sign(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Gcd => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_gcd(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Lcm => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_lcm(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Hypot => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_hypot(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::Clamp => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_clamp(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsNaN => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_nan(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::IsFinite => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_is_finite(args)?;
                    self.push_raw_u64(result)?;
                }

                // Matrix construction (normally compiled to NewMatrix opcode)
                BuiltinFunction::MatFromFlat => {
                    let args = self.pop_builtin_args()?;
                    if args.len() < 2 {
                        return Err(VMError::RuntimeError(
                            "mat() requires at least rows and cols arguments".to_string(),
                        ));
                    }
                    let rows = args[0].as_i64().unwrap_or(0) as u32;
                    let cols = args[1].as_i64().unwrap_or(0) as u32;
                    let expected = (rows as usize) * (cols as usize);
                    let mut data = shape_value::aligned_vec::AlignedVec::with_capacity(expected);
                    // If the third argument is an array, extract its elements
                    if args.len() == 3 {
                        if let Some(arr) = args[2].as_any_array() {
                            for i in 0..arr.len() {
                                if let Some(v) = arr.get_nb(i) {
                                    data.push(v.as_number_coerce().unwrap_or(0.0));
                                }
                            }
                        } else {
                            data.push(args[2].as_number_coerce().unwrap_or(0.0));
                        }
                    } else {
                        for v in &args[2..] {
                            data.push(v.as_number_coerce().unwrap_or(0.0));
                        }
                    }
                    let mat = shape_value::heap_value::MatrixData::from_flat(data, rows, cols);
                    self.push_raw_u64(ValueWord::from_matrix(std::sync::Arc::new(mat)))?;
                }

                // Table construction
                BuiltinFunction::MakeTableFromRows => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_make_table_from_rows(args)?;
                    self.push_raw_u64(result)?;
                }

                // Json navigation helpers
                BuiltinFunction::JsonObjectGet => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_object_get(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::JsonArrayAt => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_array_at(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::JsonObjectKeys => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_object_keys(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::JsonArrayLen => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_array_len(args)?;
                    self.push_raw_u64(result)?;
                }
                BuiltinFunction::JsonObjectLen => {
                    let args = self.pop_builtin_args()?;
                    let result = self.builtin_json_object_len(args)?;
                    self.push_raw_u64(result)?;
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
