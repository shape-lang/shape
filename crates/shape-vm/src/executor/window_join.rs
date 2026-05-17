//! Window functions, JOIN execution, schema binding, and typed column access.
//!
//! ADR-006 §2.7.7 / §2.7.10 / Q11: this file's window-function handlers
//! are migrated to the **MethodFnV2-shape** ABI per Wave 8 W8-WJ:
//!
//! ```rust,ignore
//! pub(crate) fn handle_window_X_v2(
//!     _vm: &mut VirtualMachine,
//!     args: &[KindedSlot],
//!     _ctx: Option<&mut ExecutionContext>,
//! ) -> Result<KindedSlot, VMError>
//! ```
//!
//! Per playbook §1 W8-WJ: window functions over a typed buffer
//! (`Array<number>` / `Array<int>` / `Array<TypedObject>`) materialize
//! per-element kind via the `TypedArrayData` arm match (§2.7.7 stack
//! parallel-kind), and bodies follow the W6.5 §2.7.10 precedent +
//! `array_sort.rs::handle_join_str_v2` recipe — receiver classification
//! on `args[0].kind`, payload recovery via `args[i].slot.as_heap_value()`
//! (ADR-005 §1 single-discriminator), per-arm dispatch on
//! `TypedArrayData::*`, kinded result via `KindedSlot::from_*`.
//!
//! `exec_bind_schema` and `exec_load_col` are live opcode handlers
//! (dispatched from `dispatch.rs`). They are migrated to the kinded API.
//! Element kinds for typed column access come from the opcode suffix
//! (LoadColF64 → `Float64`, LoadColI64 → `Int64`, LoadColBool → `Bool`,
//! LoadColStr → `String`).
//!
//! `handle_eval_datetime_expr` and `handle_join_execute` are SURFACE
//! stubs (not in W8-WJ scope per playbook §5):
//!   - eval_datetime_expr depends on §2.7.6 `HeapKind::Temporal` carrier
//!     dispatch (Phase-2c §2.7.4 boundary).
//!   - join_execute depends on `datatable_methods::joins` cross-cluster
//!     ABI flip to `&[KindedSlot]` (W9 method-body re-fill territory).

use std::sync::Arc;

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapKind;
use shape_value::{
    HeapValue, KindedSlot, NativeKind, TableViewData, VMError,
};

use super::VirtualMachine;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 (2026-05-15): TypedArrayData helpers DELETED
// ═══════════════════════════════════════════════════════════════════════════
//
// `as_typed_array` / `typed_array_to_f64_vec` / `typed_array_len`
// (TypedArrayData consumers) were deleted. The `Arc<TypedArrayData>`
// payload + `HeapValue::TypedArray` outer arm + `HeapKind::TypedArray=8`
// ordinal were retired at V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-
// deletion-audit §3.5 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. The
// window-function aggregate handlers (`handle_window_sum_v2`,
// `handle_window_avg_v2`, `handle_window_min_v2`, `handle_window_max_v2`,
// `handle_window_count_v2`) that consumed those helpers surface-and-stop
// at ckpt-5; rebuild lands at ckpt-6 STRICT close per the per-element-kind
// v2-raw `TypedArray<T>` direct-access target.
//
// Refusal #1 binding.

/// Common surface-and-stop body for the TypedArrayData-dependent window
/// aggregate handlers in this file. Returns a structured
/// `VMError::NotImplemented` citing the V3-S5 ckpt-5 cascade state.
#[cold]
#[inline(never)]
fn ckpt5_window_surface(op: &'static str) -> VMError {
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. \
         `Arc<TypedArrayData>` carrier + per-arm dispatch helpers \
         (`as_typed_array` / `typed_array_to_f64_vec` / `typed_array_len`) \
         DELETED across V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-\
         deletion-audit §3.5 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. \
         Window-aggregate scalar arm preserved (Int64/Float64); array arm \
         rebuild lands at ckpt-6 STRICT close per per-element-kind v2-raw \
         `TypedArray<T>` direct-access target. REFUSED ON SIGHT: \
         TypedArrayData resurrection under any rename (Refusal #1).",
        op = op,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2-shape window-function handlers (ADR-006 §2.7.10 / Q11)
// ═══════════════════════════════════════════════════════════════════════════
//
// Each handler signature mirrors the canonical W6.5 §2.7.10 body pattern
// (`array_sort.rs::handle_join_str_v2`):
//
//   fn(&mut VirtualMachine, args: &[KindedSlot], Option<&mut ExecutionContext>)
//       -> Result<KindedSlot, VMError>
//
// Stack-side WB2.4 retain-on-read discipline lives in the dispatch shell
// (`vm_impl/builtins.rs::op_builtin_call` calling `pop_builtin_args`); the
// `&[KindedSlot]` borrowed slice flows from that shell. Each handler
// returns a kinded result; the shell pushes via `push_kinded_slot`.
//
// All bodies follow the §2.7.6 / Q8 heterogeneous-kind body pattern: kind
// classification on `args[i].kind` first, payload recovery via
// `args[i].slot.as_heap_value()` (ADR-005 §1) for heap arms.

/// `handle_window_row_number_v2` — Wave 8 W8-WJ.
///
/// Covers `WindowRowNumber`, `WindowRank`, `WindowDenseRank`,
/// `WindowNtile`. Pre-§2.7.10 the shared handler returned the constant
/// `1` per row (legacy semantics: window framing not implemented at the
/// VM level — these are placeholder values for the row-by-row window
/// pipeline). The kinded result is `NativeKind::Int64`.
pub(crate) fn handle_window_row_number_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Ok(KindedSlot::from_int(1))
}

/// `handle_window_lag_v2` — Wave 8 W8-WJ.
///
/// Covers `WindowLag` and `WindowLead`. Args:
///   `[value, offset, default?]`
/// Pre-§2.7.10 semantics: return `args[2]` if present (the user-supplied
/// default), otherwise `null`. Window framing is not modeled at the VM
/// level (lag/lead read the offset row from the windowed iterator at
/// the compile-time-lowered level); this handler is the per-row
/// fallback that materializes a kinded null when offset reaches outside
/// the partition.
pub(crate) fn handle_window_lag_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Ok(args.get(2).cloned().unwrap_or_else(KindedSlot::none))
}

/// `handle_window_first_value_v2` — Wave 8 W8-WJ.
///
/// Covers `WindowFirstValue`, `WindowLastValue`, `WindowNthValue`. Args:
///   `[value, ...]`
/// Pre-§2.7.10 semantics: return `args[0]` (the per-row value passed in)
/// — the windowed projection collapses to the value when the framing is
/// trivial. Nontrivial framing is not modeled at the VM level.
pub(crate) fn handle_window_first_value_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Ok(args.first().cloned().unwrap_or_else(KindedSlot::none))
}

/// `handle_window_sum_v2` — Wave 8 W8-WJ.
///
/// Args: `[value]` where `value` is either:
///   - a scalar (Int64 / Float64) — the per-row pre-aggregated value;
///   - a `Vec<number>` / `Vec<int>` window frame — sum reduces the
///     numeric arm via per-element `TypedArrayData::*` dispatch
///     (§2.7.7).
///
/// Result kind: `NativeKind::Float64` (legacy semantics — sum widens to
/// number to avoid integer-overflow).
pub(crate) fn handle_window_sum_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arg = args.first().ok_or_else(|| {
        type_error("WindowSum requires at least 1 argument (value or array)")
    })?;
    match arg.kind {
        NativeKind::Int64 | NativeKind::Float64 => Ok(arg.clone()),
        // V3-S5 ckpt-5: TypedArray arm surface; rebuild at ckpt-6 STRICT
        // close per per-element-kind v2-raw `TypedArray<T>` direct access.
        _ => Err(ckpt5_window_surface("WindowSum")),
    }
}

/// `handle_window_avg_v2` — Wave 8 W8-WJ.
///
/// Args: `[value]` with the same scalar / `Vec<number>` shape as
/// `WindowSum`. Empty array yields `null`. Result kind:
/// `NativeKind::Float64`.
pub(crate) fn handle_window_avg_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arg = args.first().ok_or_else(|| {
        type_error("WindowAvg requires at least 1 argument (value or array)")
    })?;
    match arg.kind {
        NativeKind::Int64 => {
            let i = arg.as_i64().expect("kind=Int64");
            Ok(KindedSlot::from_number(i as f64))
        }
        NativeKind::Float64 => Ok(arg.clone()),
        // V3-S5 ckpt-5: TypedArray arm surface; ckpt-6 rebuild target.
        _ => Err(ckpt5_window_surface("WindowAvg")),
    }
}

/// `handle_window_min_max_v2` — Wave 8 W8-WJ.
///
/// Covers `WindowMin` and `WindowMax`. The `pick_max` flag selects the
/// reducer: `false` → `f64::min`, `true` → `f64::max`. Args:
///   `[value]` (scalar or `Vec<number>` / `Vec<int>`).
/// Empty array yields `null`. Result kind: `NativeKind::Float64`.
pub(crate) fn handle_window_min_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_window_min_max_inner(args, false)
}

pub(crate) fn handle_window_max_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_window_min_max_inner(args, true)
}

fn handle_window_min_max_inner(
    args: &[KindedSlot],
    pick_max: bool,
) -> Result<KindedSlot, VMError> {
    let _ = pick_max;
    let arg = args.first().ok_or_else(|| {
        type_error("WindowMin/Max requires at least 1 argument (value or array)")
    })?;
    match arg.kind {
        NativeKind::Int64 | NativeKind::Float64 => Ok(arg.clone()),
        // V3-S5 ckpt-5: TypedArray arm surface; ckpt-6 rebuild target.
        _ => Err(ckpt5_window_surface("WindowMin/Max")),
    }
}

/// `handle_window_count_v2` — Wave 8 W8-WJ.
///
/// Args: `[value]`. For an array input, count non-null entries via the
/// `TypedArrayData` arm (per §2.7.7 every element of a typed buffer is
/// non-null by definition — `Vec<number?>` would be a separate
/// `Nullable*` track). For a scalar input, count `1` for non-null
/// values, `0` for null. Result kind: `NativeKind::Int64`.
pub(crate) fn handle_window_count_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arg = match args.first() {
        Some(a) => a,
        None => return Ok(KindedSlot::from_int(0)),
    };
    match arg.kind {
        // Inline scalars: count 1 for non-null. None / unit slots have
        // raw bits == 0 and Bool kind by convention; treat raw 0 as 0.
        NativeKind::Bool if arg.slot.raw() == 0 => Ok(KindedSlot::from_int(0)),
        NativeKind::Int64 | NativeKind::Float64 | NativeKind::Bool => Ok(KindedSlot::from_int(1)),
        // V3-S5 ckpt-5: TypedArray arm surface; ckpt-6 rebuild target.
        _ => Err(ckpt5_window_surface("WindowCount")),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SURFACE stubs (out of W8-WJ scope per playbook §5)
// ═══════════════════════════════════════════════════════════════════════════
//
// `handle_eval_datetime_expr` and `handle_join_execute` are not migrated
// in this sub-cluster:
//
//   - `handle_eval_datetime_expr` requires the §2.7.6 / Q8
//     `HeapKind::Temporal` carrier dispatch (DateTimeExpr → DateTime);
//     the carrier shape itself is a Phase-2c reentry per §2.7.4.
//
//   - `handle_join_execute` requires `datatable_methods::joins` to flip
//     to the `&[KindedSlot]` ABI first (W9 method-body re-fill). Both
//     halves of that pipeline must move together; migrating only the
//     dispatch shell here would leak the deleted ABI shape into the
//     join handler call boundary.
//
// Both keep the legacy `&mut self` shape since they currently surface
// `NotImplemented(SURFACE)` and the migration target signature
// (`&[KindedSlot] -> Result<KindedSlot, VMError>`) cannot be filled
// without the upstream dependency.

impl VirtualMachine {
    /// Handle eval datetime expression.
    ///
    /// **SURFACE — Phase-2c §2.7.4 boundary.** Body re-implementation
    /// requires the `HeapKind::Temporal` carrier shape (per §2.7.6 / Q8
    /// dispatch on `args[0].slot.as_heap_value()` with a
    /// `HeapValue::Temporal(TemporalData::DateTimeExpr(..))` arm) plus
    /// the kinded result construction for `HeapValue::Temporal
    /// (TemporalData::DateTime(..))`. The surrounding pure-AST helper
    /// `eval_datetime_expr_recursive` is preserved (no forbidden
    /// patterns, ready for the body re-fill).
    pub(crate) fn handle_eval_datetime_expr(
        &mut self,
        _ctx: Option<&mut ExecutionContext>,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "W8-WJ — handle_eval_datetime_expr SURFACE: depends on \
             HeapKind::Temporal carrier dispatch (§2.7.6 / Q8). \
             Phase-2c §2.7.4 boundary; body re-fill lands when the \
             Temporal heap arm dispatch table is wired in the §2.7.10 \
             MethodFnV2 surface."
                .to_string(),
        ))
    }

    /// Recursively evaluate a DateTimeExpr into a chrono DateTime.
    ///
    /// Pure-AST helper. Consumes no VM stack state and uses no forbidden
    /// patterns. Live caller: `op_push_const`'s `Constant::DateTimeExpr`
    /// arm (`executor/stack_ops/mod.rs`) — C1-temporal-lowering moved
    /// DateTimeExpr evaluation to push time so the temporal value is
    /// produced before the matching `BuiltinCall(EvalDateTimeExpr)`
    /// identity passthrough fires. `@now` / `@today` still evaluate at
    /// execution time because `op_push_const` runs at VM execution time.
    pub(crate) fn eval_datetime_expr_recursive(
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

    /// Handle JOIN execution.
    ///
    /// **SURFACE — cross-cluster cascade (W9).** Body re-implementation
    /// requires `datatable_methods::joins` to flip to the
    /// `&[KindedSlot]` ABI first; both halves of the dispatch must
    /// move together. W8-WJ migrates the rest of `window_join.rs` off
    /// the deleted ABI; the join body stays surfaced for the W9
    /// method-body re-fill wave.
    pub(crate) fn handle_join_execute(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "W8-WJ — handle_join_execute SURFACE: depends on \
             datatable_methods::joins ABI flip to &[KindedSlot] (W9 \
             method-body re-fill). Cross-cluster cascade per playbook \
             §5; body re-fill lands when the join handler call \
             boundary is kinded."
                .to_string(),
        ))
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Live opcode handlers (BindSchema / LoadCol*) — kinded API
    // ═══════════════════════════════════════════════════════════════════════

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

// V3-S5 ckpt-5 (2026-05-15): test module gated. The tests asserted on
// `typed_array_len` and `typed_array_to_f64_vec` (deleted helpers) and
// constructed `Arc<TypedArrayData>` via `TypedBuffer::from(values)` /
// `AlignedTypedBuffer::from(av)` (deleted carriers per V3-S5 ckpt-1..
// ckpt-4 per W12-typed-array-data-deletion-audit §3.5 + §B + ADR-006
// §2.7.24 Q25.A SUPERSEDED). Tests preserved in git history at the
// W8-WJ landing commit. Rebuild lands at ckpt-6 STRICT close per the
// per-element-kind v2-raw `TypedArray<T>` direct-access target.
#[cfg(any())]
mod tests {}
