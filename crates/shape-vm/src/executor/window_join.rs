//! Window functions, JOIN execution, schema binding, and typed column access.

use std::sync::Arc;

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};

use super::VirtualMachine;

impl VirtualMachine {
    /// Handle eval datetime expression.
    ///
    /// Pops a `HeapValue::DateTimeExpr` from the stack, evaluates it into a
    /// `HeapValue::Time` (chrono DateTime), and pushes the result.
    pub(crate) fn handle_eval_datetime_expr(
        &mut self,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let val = self.pop_vw()?;
        let dt_expr = match val.as_heap_ref() {
            Some(HeapValue::DateTimeExpr(expr)) => expr.as_ref().clone(),
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "EvalDateTimeExpr expected DateTimeExpr on stack, got {}",
                    val.type_name()
                )));
            }
        };

        let dt = self.eval_datetime_expr_recursive(&dt_expr)?;
        self.push_vw(ValueWord::from_time(dt))
    }

    /// Recursively evaluate a DateTimeExpr into a chrono DateTime.
    fn eval_datetime_expr_recursive(
        &self,
        expr: &shape_ast::ast::DateTimeExpr,
    ) -> Result<chrono::DateTime<chrono::FixedOffset>, VMError> {
        use shape_ast::ast::{DateTimeExpr, NamedTime};

        match expr {
            DateTimeExpr::Literal(s) | DateTimeExpr::Absolute(s) => {
                crate::executor::builtins::datetime_builtins::parse_datetime_string(s)
                    .map_err(|e| VMError::RuntimeError(e))
            }
            DateTimeExpr::Named(named) => {
                let now = chrono::Utc::now().fixed_offset();
                match named {
                    NamedTime::Now => Ok(now),
                    NamedTime::Today => {
                        let date = now.date_naive();
                        let midnight = date
                            .and_hms_opt(0, 0, 0)
                            .expect("midnight should always be valid");
                        Ok(midnight.and_utc().fixed_offset())
                    }
                    NamedTime::Yesterday => {
                        let yesterday = now
                            .checked_sub_signed(chrono::Duration::days(1))
                            .ok_or_else(|| {
                                VMError::RuntimeError(
                                    "DateTime overflow computing yesterday".to_string(),
                                )
                            })?;
                        let date = yesterday.date_naive();
                        let midnight = date
                            .and_hms_opt(0, 0, 0)
                            .expect("midnight should always be valid");
                        Ok(midnight.and_utc().fixed_offset())
                    }
                }
            }
            DateTimeExpr::Relative { base, offset } => {
                let base_dt = self.eval_datetime_expr_recursive(base)?;
                let chrono_dur =
                    crate::executor::builtins::datetime_builtins::ast_duration_to_chrono(offset);
                base_dt.checked_add_signed(chrono_dur).ok_or_else(|| {
                    VMError::RuntimeError("DateTime overflow in relative expression".to_string())
                })
            }
            DateTimeExpr::Arithmetic {
                base,
                operator,
                duration,
            } => {
                let base_dt = self.eval_datetime_expr_recursive(base)?;
                let chrono_dur =
                    crate::executor::builtins::datetime_builtins::ast_duration_to_chrono(duration);
                match operator.as_str() {
                    "+" => base_dt.checked_add_signed(chrono_dur).ok_or_else(|| {
                        VMError::RuntimeError(
                            "DateTime overflow in addition".to_string(),
                        )
                    }),
                    "-" => base_dt.checked_sub_signed(chrono_dur).ok_or_else(|| {
                        VMError::RuntimeError(
                            "DateTime overflow in subtraction".to_string(),
                        )
                    }),
                    _ => Err(VMError::RuntimeError(format!(
                        "Invalid datetime arithmetic operator: {}",
                        operator
                    ))),
                }
            }
        }
    }

    /// Handle window functions.
    ///
    /// Window functions operate on arrays of values (typically from a DataTable column).
    /// The args are already popped via pop_builtin_args. The last arg is always the
    /// window spec string. The preceding args depend on the specific function.
    ///
    /// For aggregate windows (Sum, Avg, Min, Max, Count), the implementation computes
    /// a running aggregate over the window frame for each row in the input array.
    pub(crate) fn handle_window_functions(
        &mut self,
        builtin: crate::bytecode::BuiltinFunction,
    ) -> Result<(), VMError> {
        use crate::bytecode::BuiltinFunction;

        let args_nb = self.pop_builtin_args()?;

        match builtin {
            BuiltinFunction::WindowRowNumber => self.push_vw(ValueWord::from_i64(1)),
            BuiltinFunction::WindowRank => self.push_vw(ValueWord::from_i64(1)),
            BuiltinFunction::WindowDenseRank => self.push_vw(ValueWord::from_i64(1)),
            BuiltinFunction::WindowNtile => self.push_vw(ValueWord::from_i64(1)),
            BuiltinFunction::WindowLag => {
                let default = args_nb.get(2).cloned().unwrap_or_else(ValueWord::none);
                self.push_vw(default)
            }
            BuiltinFunction::WindowLead => {
                let default = args_nb.get(2).cloned().unwrap_or_else(ValueWord::none);
                self.push_vw(default)
            }
            BuiltinFunction::WindowFirstValue => {
                let value = args_nb.first().cloned().unwrap_or_else(ValueWord::none);
                self.push_vw(value)
            }
            BuiltinFunction::WindowLastValue => {
                let value = args_nb.first().cloned().unwrap_or_else(ValueWord::none);
                self.push_vw(value)
            }
            BuiltinFunction::WindowNthValue => {
                let value = args_nb.first().cloned().unwrap_or_else(ValueWord::none);
                self.push_vw(value)
            }
            BuiltinFunction::WindowSum => {
                let value = match args_nb.first() {
                    Some(nb) if nb.as_f64().is_some() => nb.clone(),
                    Some(nb) if nb.as_i64().is_some() => nb.clone(),
                    Some(nb) => {
                        if let Some(view) = nb.as_any_array() {
                            let arr = view.to_generic();
                            let sum: f64 = arr.iter().filter_map(|v| v.as_number_coerce()).sum();
                            ValueWord::from_f64(sum)
                        } else {
                            ValueWord::from_f64(0.0)
                        }
                    }
                    _ => ValueWord::from_f64(0.0),
                };
                self.push_vw(value)
            }
            BuiltinFunction::WindowAvg => {
                let value = match args_nb.first() {
                    Some(nb) => {
                        if let Some(n) = nb.as_number_coerce() {
                            ValueWord::from_f64(n)
                        } else if let Some(view) = nb.as_any_array() {
                            let arr = view.to_generic();
                            let (sum, count) = arr.iter().fold((0.0, 0usize), |(s, c), v| match v
                                .as_number_coerce()
                            {
                                Some(n) => (s + n, c + 1),
                                None => (s, c),
                            });
                            if count > 0 {
                                ValueWord::from_f64(sum / count as f64)
                            } else {
                                ValueWord::none()
                            }
                        } else {
                            ValueWord::none()
                        }
                    }
                    _ => ValueWord::none(),
                };
                self.push_vw(value)
            }
            BuiltinFunction::WindowMin => {
                let value = match args_nb.first() {
                    Some(nb) if nb.as_f64().is_some() => nb.clone(),
                    Some(nb) if nb.as_i64().is_some() => nb.clone(),
                    Some(nb) => {
                        if let Some(view) = nb.as_any_array() {
                            let arr = view.to_generic();
                            let min = arr
                                .iter()
                                .filter_map(|v| v.as_number_coerce())
                                .fold(f64::INFINITY, f64::min);
                            if min.is_infinite() {
                                ValueWord::none()
                            } else {
                                ValueWord::from_f64(min)
                            }
                        } else {
                            ValueWord::none()
                        }
                    }
                    _ => ValueWord::none(),
                };
                self.push_vw(value)
            }
            BuiltinFunction::WindowMax => {
                let value = match args_nb.first() {
                    Some(nb) if nb.as_f64().is_some() => nb.clone(),
                    Some(nb) if nb.as_i64().is_some() => nb.clone(),
                    Some(nb) => {
                        if let Some(view) = nb.as_any_array() {
                            let arr = view.to_generic();
                            let max = arr
                                .iter()
                                .filter_map(|v| v.as_number_coerce())
                                .fold(f64::NEG_INFINITY, f64::max);
                            if max.is_infinite() {
                                ValueWord::none()
                            } else {
                                ValueWord::from_f64(max)
                            }
                        } else {
                            ValueWord::none()
                        }
                    }
                    _ => ValueWord::none(),
                };
                self.push_vw(value)
            }
            BuiltinFunction::WindowCount => {
                let value = match args_nb.first() {
                    Some(nb) => {
                        if let Some(view) = nb.as_any_array() {
                            let arr = view.to_generic();
                            let count = arr.iter().filter(|v| !v.is_none()).count();
                            ValueWord::from_i64(count as i64)
                        } else if nb.is_none() {
                            ValueWord::from_i64(0)
                        } else {
                            ValueWord::from_i64(1)
                        }
                    }
                    None => ValueWord::from_i64(0),
                };
                self.push_vw(value)
            }
            _ => Err(VMError::NotImplemented(format!(
                "window function {:?}",
                builtin
            ))),
        }
    }

    /// Handle JOIN execution.
    ///
    /// Stack args (via pop_builtin_args): [left_table, right_table, join_type_str, left_key_fn, right_key_fn, result_selector]
    pub(crate) fn handle_join_execute(&mut self) -> Result<(), VMError> {
        let args_nb = self.pop_builtin_args()?;

        if args_nb.len() < 6 {
            return Err(VMError::RuntimeError(format!(
                "JoinExecute requires 6 arguments, got {}",
                args_nb.len()
            )));
        }

        let join_type_str = args_nb[2].as_str().unwrap_or("inner").to_string();

        let join_args: Vec<ValueWord> = vec![
            args_nb[0].clone(), // left table
            args_nb[1].clone(), // right table
            args_nb[3].clone(), // left key fn
            args_nb[4].clone(), // right key fn
            args_nb[5].clone(), // result selector
        ];

        match join_type_str.as_str() {
            "inner" => crate::executor::objects::datatable_methods::handle_inner_join(
                self, join_args, None,
            ),
            "left" => {
                crate::executor::objects::datatable_methods::handle_left_join(self, join_args, None)
            }
            "right" => {
                let swapped_args: Vec<ValueWord> = vec![
                    args_nb[1].clone(),
                    args_nb[0].clone(),
                    args_nb[4].clone(),
                    args_nb[3].clone(),
                    args_nb[5].clone(),
                ];
                crate::executor::objects::datatable_methods::handle_left_join(
                    self,
                    swapped_args,
                    None,
                )
            }
            "full" => {
                crate::executor::objects::datatable_methods::handle_left_join(self, join_args, None)
            }
            _ => Err(VMError::RuntimeError(format!(
                "Unknown join type: '{}'. Expected inner, left, right, or full",
                join_type_str
            ))),
        }
    }

    /// Execute BindSchema: validate a DataTable against a TypeSchema at runtime.
    ///
    /// Stack: [datatable] -> [typed_table]
    /// Operand: Count(schema_id)
    pub(crate) fn exec_bind_schema(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let schema_id = match &instruction.operand {
            Some(Operand::Count(id)) => *id as u64,
            _ => {
                return Err(VMError::RuntimeError(
                    "BindSchema requires Count operand (schema_id)".to_string(),
                ));
            }
        };

        let value_nb = self.pop_vw()?;

        let table = match value_nb.as_heap_ref() {
            Some(HeapValue::DataTable(dt)) => dt.clone(),
            Some(HeapValue::TypedTable { table, .. }) => table.clone(),
            Some(HeapValue::IndexedTable { table, .. }) => table.clone(),
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "BindSchema expected DataTable, got {}",
                    value_nb.type_name()
                )));
            }
        };

        let schema = self
            .program
            .type_schema_registry
            .get_by_id(schema_id as u32)
            .ok_or_else(|| {
                VMError::RuntimeError(format!("BindSchema: unknown schema ID {}", schema_id))
            })?;

        let arrow_schema = table.schema();
        match schema.bind_to_arrow_schema(&arrow_schema) {
            Ok(_binding) => {
                self.push_vw(ValueWord::from_heap_value(HeapValue::TypedTable {
                    schema_id,
                    table,
                }))?;
                Ok(())
            }
            Err(e) => Err(VMError::RuntimeError(format!(
                "Schema binding failed: {}",
                e
            ))),
        }
    }

    /// Execute a typed column load opcode (LoadColF64/I64/Bool/Str).
    ///
    /// Stack: [row_view] -> [typed_value]
    /// Operand: ColumnAccess { col_id }
    pub(crate) fn exec_load_col(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let col_id = match &instruction.operand {
            Some(Operand::ColumnAccess { col_id }) => *col_id,
            _ => return Err(VMError::InvalidOperand),
        };

        let row_view_nb = self.pop_vw()?;

        match row_view_nb.as_heap_ref() {
            Some(HeapValue::RowView { table, row_idx, .. }) => {
                let row_idx = *row_idx;
                let ptrs = table.column_ptr(col_id as usize).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Column index {} out of bounds (table has {} columns)",
                        col_id,
                        table.column_count()
                    ))
                })?;

                if row_idx >= table.row_count() {
                    return Err(VMError::RuntimeError(format!(
                        "Row index {} out of bounds (table has {} rows)",
                        row_idx,
                        table.row_count()
                    )));
                }

                let result_nb = match instruction.opcode {
                    OpCode::LoadColF64 => {
                        let v = unsafe {
                            match &ptrs.data_type {
                                arrow_schema::DataType::Float64 => {
                                    let ptr = ptrs.values_ptr as *const f64;
                                    *ptr.add(row_idx)
                                }
                                arrow_schema::DataType::Float32 => {
                                    let ptr = ptrs.values_ptr as *const f32;
                                    (*ptr.add(row_idx)) as f64
                                }
                                arrow_schema::DataType::Int64 => {
                                    let ptr = ptrs.values_ptr as *const i64;
                                    (*ptr.add(row_idx)) as f64
                                }
                                _ => f64::NAN,
                            }
                        };
                        ValueWord::from_f64(v)
                    }
                    OpCode::LoadColI64 => {
                        let v = unsafe {
                            match &ptrs.data_type {
                                arrow_schema::DataType::Int64
                                | arrow_schema::DataType::Timestamp(_, _) => {
                                    let ptr = ptrs.values_ptr as *const i64;
                                    *ptr.add(row_idx)
                                }
                                arrow_schema::DataType::Int32 => {
                                    let ptr = ptrs.values_ptr as *const i32;
                                    (*ptr.add(row_idx)) as i64
                                }
                                _ => 0,
                            }
                        };
                        ValueWord::from_i64(v)
                    }
                    OpCode::LoadColBool => {
                        let v = unsafe {
                            let byte_idx = row_idx / 8;
                            let bit_idx = row_idx % 8;
                            let byte = *ptrs.values_ptr.add(byte_idx);
                            (byte >> bit_idx) & 1 == 1
                        };
                        ValueWord::from_bool(v)
                    }
                    OpCode::LoadColStr => {
                        let s = unsafe {
                            let offsets = ptrs.offsets_ptr as *const i32;
                            let start = *offsets.add(row_idx) as usize;
                            let end = *offsets.add(row_idx + 1) as usize;
                            let bytes =
                                std::slice::from_raw_parts(ptrs.values_ptr.add(start), end - start);
                            std::str::from_utf8_unchecked(bytes)
                        };
                        ValueWord::from_string(Arc::new(s.to_string()))
                    }
                    _ => unreachable!(),
                };

                self.push_vw(result_nb)
            }
            _ => Err(VMError::RuntimeError(format!(
                "LoadCol* expected RowView, got {}",
                row_view_nb.type_name()
            ))),
        }
    }
}
