//! Window functions, JOIN execution, schema binding, and typed column access.
//!
//! ADR-006 §2.7.7 / Wave 6.5 sub-cluster D-window-join: this file's
//! handlers are migrated to the kinded VM stack ABI. The legacy
//! ValueWord-shape paths (`pop_raw_u64` + `as_heap_ref` + `ValueWord::from_*`)
//! are forbidden post-bulldozer (CLAUDE.md "Forbidden Patterns").
//!
//! The window / join / eval-datetime entrypoints (`handle_window_functions`,
//! `handle_join_execute`, `handle_eval_datetime_expr`) are surfaced as
//! `NotImplemented(SURFACE)` placeholders per playbook §7 REVISED. Their
//! callers in `vm_impl/builtins.rs` already `todo!()` on Wave 5e body
//! migration; these stubs migrate the file off forbidden patterns without
//! attempting body re-implementation in scope of this sub-cluster.
//!
//! `exec_bind_schema` and `exec_load_col` are live opcode handlers
//! (dispatched from `dispatch.rs`). They are migrated to the kinded API.
//! Element kinds for typed column access come from the opcode suffix
//! (LoadColF64 → `Float64`, LoadColI64 → `Int64`, LoadColBool → `Bool`,
//! LoadColStr → `String`).

use std::sync::Arc;

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::heap_value::HeapKind;
use shape_value::{NativeKind, TableViewData, VMError};

use super::VirtualMachine;

impl VirtualMachine {
    /// Handle eval datetime expression.
    ///
    /// Wave 5e backlog: the body of this handler historically popped a
    /// `HeapValue::Temporal(TemporalData::DateTimeExpr(...))` via
    /// `pop_raw_u64` + `as_heap_ref` (forbidden the deleted tag_bits dispatch) and pushed
    /// a `HeapValue::Temporal(TemporalData::DateTime(...))` via
    /// `ValueWord::from_time` (deleted). Migration to the kinded API on
    /// `NativeKind::Ptr(HeapKind::Temporal)` requires the §2.7.6 carrier
    /// dispatch shape that Wave 5e is responsible for; the call site in
    /// `vm_impl/builtins.rs` already `todo!()`s on this body, so the
    /// surface is observable and tracked.
    pub(crate) fn handle_eval_datetime_expr(
        &mut self,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "phase-1b-vm wave 5e — handle_eval_datetime_expr body migration \
             pending (D-window-join surfaced; kind-source: \
             NativeKind::Ptr(HeapKind::Temporal))"
                .to_string(),
        ))
    }

    /// Recursively evaluate a DateTimeExpr into a chrono DateTime.
    ///
    /// Pure-AST helper retained for the Wave 5e body re-implementation of
    /// `handle_eval_datetime_expr`; consumes no VM stack state and uses no
    /// forbidden patterns.
    #[allow(dead_code)]
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
                        VMError::RuntimeError("DateTime overflow in addition".to_string())
                    }),
                    "-" => base_dt.checked_sub_signed(chrono_dur).ok_or_else(|| {
                        VMError::RuntimeError("DateTime overflow in subtraction".to_string())
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
    /// Wave 5e backlog: the body historically dispatched on a
    /// `Vec<ValueWord>` arg slice produced by `pop_builtin_args` (legacy
    /// ABI), inspected each arg via `as_any_array`/`as_number_coerce` /
    /// `as_f64`/`as_i64` (deleted ValueWord helpers), and pushed
    /// `ValueWord::from_f64` / `ValueWord::from_i64` / `ValueWord::none`
    /// results via `push_raw_u64` (deleted shim). Migration to the kinded
    /// `pop_builtin_args -> Vec<KindedSlot>` ABI plus per-arg
    /// `numeric_domain` dispatch on the `NativeKind` track is tracked under
    /// Wave 5e; the call site in `vm_impl/builtins.rs` already `todo!()`s.
    pub(crate) fn handle_window_functions(
        &mut self,
        builtin: crate::bytecode::BuiltinFunction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(format!(
            "phase-1b-vm wave 5e — window function body migration pending \
             (D-window-join surfaced; kind-source: per-arg NativeKind via \
             numeric_domain dispatch on KindedSlot inputs): {:?}",
            builtin
        )))
    }

    /// Handle JOIN execution.
    ///
    /// Wave 5e backlog: the body historically built a `Vec<u64>` raw-bits
    /// arg slice via `into_raw_bits` (deleted ValueWord op) and dispatched
    /// to `datatable_methods::handle_*_join` whose own ABI takes
    /// `&mut [u64]` of legacy ValueWord bits (also pre-migration). Both
    /// halves of that pipeline must move to the kinded ABI together;
    /// migrating only the dispatch shell here would leak forbidden patterns
    /// into the join handler call boundary. Surface and stop per playbook
    /// §8 (cross-cluster cascade — datatable_methods/joins.rs is its own
    /// territory). The call site in `vm_impl/builtins.rs` already
    /// `todo!()`s.
    pub(crate) fn handle_join_execute(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "phase-1b-vm wave 5e — JOIN body migration pending \
             (D-window-join surfaced; depends on datatable_methods::joins \
             ABI flip to &[KindedSlot])"
                .to_string(),
        ))
    }

    /// Execute BindSchema: validate a DataTable against a TypeSchema at runtime.
    ///
    /// Stack: [datatable] -> [typed_table]
    /// Operand: Count(schema_id)
    ///
    /// **Kinded migration (D-window-join, ADR-006 §2.7.7):**
    /// - Pop expects `NativeKind::Ptr(HeapKind::TableView)` (TypedTable /
    ///   IndexedTable variants both carry the underlying DataTable Arc) or
    ///   `NativeKind::Ptr(HeapKind::DataTable)` for the bare DataTable
    ///   case. Inputs flow as `Arc::into_raw::<TableViewData>` /
    ///   `Arc::into_raw::<DataTable>` per playbook §3 per-HeapKind table.
    /// - Push: `NativeKind::Ptr(HeapKind::TableView)`, bits =
    ///   `Arc::into_raw::<TableViewData>` of a fresh `TypedTable` payload.
    pub(crate) fn exec_bind_schema(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let schema_id = match &instruction.operand {
            Some(Operand::Count(id)) => *id as u64,
            _ => {
                return Err(VMError::RuntimeError(
                    "BindSchema requires Count operand (schema_id)".to_string(),
                ));
            }
        };

        let (bits, kind) = self.pop_kinded()?;

        // Borrow / reconstitute the underlying DataTable Arc by kind-
        // dispatch on the popped slot. Mirrors the `slot.as_heap_value()`
        // + `HeapValue` match discipline (§2.7.6 / Q8) without per-heap-
        // variant accessors on the carrier.
        //
        // SAFETY: when `kind` selects the `TableView` / `DataTable` heap
        // arms, `bits` are the result of `Arc::into_raw::<T>` for the
        // matching `T` (playbook §3).
        // - TableView arm: borrow `&Arc<DataTable>` out of the inner
        //   `TableViewData`, `Arc::clone` to get an independent share, then
        //   `drop_with_kind` retires the popped TableView share.
        // - DataTable arm: the popped slot already owns one
        //   `Arc<DataTable>` share — `Arc::from_raw` reconstitutes it
        //   directly, transferring the share into the local `Arc`. NO
        //   `drop_with_kind` call here (the share is consumed).
        let table = match kind {
            NativeKind::Ptr(HeapKind::TableView) => {
                let tv = unsafe { &*(bits as *const TableViewData) };
                let cloned = match tv {
                    TableViewData::TypedTable { table, .. }
                    | TableViewData::IndexedTable { table, .. }
                    | TableViewData::RowView { table, .. }
                    | TableViewData::ColumnRef { table, .. } => Arc::clone(table),
                };
                drop_with_kind(bits, kind);
                cloned
            }
            NativeKind::Ptr(HeapKind::DataTable) => {
                // SAFETY: pop_kinded transferred the strong-count share to
                // us; Arc::from_raw reconstitutes the owning Arc.
                unsafe { Arc::from_raw(bits as *const shape_value::DataTable) }
            }
            _ => {
                drop_with_kind(bits, kind);
                return Err(VMError::RuntimeError(format!(
                    "BindSchema expected DataTable/TableView, got {:?}",
                    kind
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
                // Push a TypedTable TableView — playbook §3 per-HeapKind table:
                // `Arc::into_raw::<TableViewData>` + `NativeKind::Ptr(HeapKind::TableView)`.
                let tv = Arc::new(TableViewData::TypedTable {
                    schema_id,
                    table,
                });
                let out_bits = Arc::into_raw(tv) as u64;
                self.push_kinded(out_bits, NativeKind::Ptr(HeapKind::TableView))?;
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
    ///
    /// **Kinded migration (D-window-join, ADR-006 §2.7.7):**
    /// - Pop expects `NativeKind::Ptr(HeapKind::TableView)` containing a
    ///   `RowView` payload (bits = `Arc::into_raw::<TableViewData>`).
    /// - Push kind sourced from the opcode suffix (playbook §2 typed-arith
    ///   suffix rule applied to typed-column-access):
    ///     - `LoadColF64`  → `NativeKind::Float64`,  bits = `f64::to_bits`
    ///     - `LoadColI64`  → `NativeKind::Int64`,    bits = `i64 as u64`
    ///     - `LoadColBool` → `NativeKind::Bool`,     bits = `b as u64`
    ///     - `LoadColStr`  → `NativeKind::String`,   bits = `Arc::into_raw::<String>`
    pub(crate) fn exec_load_col(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let col_id = match &instruction.operand {
            Some(Operand::ColumnAccess { col_id }) => *col_id,
            _ => return Err(VMError::InvalidOperand),
        };

        let (bits, kind) = self.pop_kinded()?;

        // Borrow the RowView payload by kind-dispatch (Q8: no per-heap-
        // variant accessor on the carrier; this is the body-side
        // dispatch on the popped (bits, kind) tuple).
        //
        // SAFETY: when `kind == NativeKind::Ptr(HeapKind::TableView)`,
        // `bits` is `Arc::into_raw::<TableViewData>` per playbook §3.
        // We borrow without consuming the share; `drop_with_kind` retires
        // it on every exit path below.
        let result = match kind {
            NativeKind::Ptr(HeapKind::TableView) => unsafe {
                let tv = &*(bits as *const TableViewData);
                match tv {
                    TableViewData::RowView { table, row_idx, .. } => {
                        let row_idx = *row_idx;
                        let ptrs = match table.column_ptr(col_id as usize) {
                            Some(p) => p,
                            None => {
                                let cc = table.column_count();
                                drop_with_kind(bits, kind);
                                return Err(VMError::RuntimeError(format!(
                                    "Column index {} out of bounds (table has {} columns)",
                                    col_id, cc
                                )));
                            }
                        };

                        if row_idx >= table.row_count() {
                            let rc = table.row_count();
                            drop_with_kind(bits, kind);
                            return Err(VMError::RuntimeError(format!(
                                "Row index {} out of bounds (table has {} rows)",
                                row_idx, rc
                            )));
                        }

                        // Compute the typed result and its kind from the
                        // opcode suffix. No coercion — every arm sources
                        // its own kind locally per playbook §2.
                        match instruction.opcode {
                            OpCode::LoadColF64 => {
                                let v = match &ptrs.data_type {
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
                                };
                                Ok((v.to_bits(), NativeKind::Float64))
                            }
                            OpCode::LoadColI64 => {
                                let v = match &ptrs.data_type {
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
                                };
                                Ok((v as u64, NativeKind::Int64))
                            }
                            OpCode::LoadColBool => {
                                let byte_idx = row_idx / 8;
                                let bit_idx = row_idx % 8;
                                let byte = *ptrs.values_ptr.add(byte_idx);
                                let v = (byte >> bit_idx) & 1 == 1;
                                Ok((v as u64, NativeKind::Bool))
                            }
                            OpCode::LoadColStr => {
                                let offsets = ptrs.offsets_ptr as *const i32;
                                let start = *offsets.add(row_idx) as usize;
                                let end = *offsets.add(row_idx + 1) as usize;
                                let bytes = std::slice::from_raw_parts(
                                    ptrs.values_ptr.add(start),
                                    end - start,
                                );
                                let s = std::str::from_utf8_unchecked(bytes);
                                let arc = Arc::new(s.to_string());
                                let str_bits = Arc::into_raw(arc) as u64;
                                Ok((str_bits, NativeKind::String))
                            }
                            _ => Err(VMError::RuntimeError(format!(
                                "exec_load_col called with non-LoadCol opcode: {:?}",
                                instruction.opcode
                            ))),
                        }
                    }
                    _ => Err(VMError::RuntimeError(format!(
                        "LoadCol* expected RowView TableView, got {:?}",
                        kind
                    ))),
                }
            },
            _ => Err(VMError::RuntimeError(format!(
                "LoadCol* expected RowView (TableView heap kind), got {:?}",
                kind
            ))),
        };

        // Retire the popped TableView share regardless of success/failure.
        drop_with_kind(bits, kind);

        let (out_bits, out_kind) = result?;
        self.push_kinded(out_bits, out_kind)?;
        Ok(())
    }
}
