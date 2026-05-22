//! Special-op builtin bodies (ADR-006 §2.7.6 / Q8).
//!
//! Wave 5e: the table-from-rows constructor body is re-introduced on the
//! kinded carrier ABI. `MakeTableFromRows` builds an Arrow `RecordBatch`
//! from a row-major flat value buffer and wraps it in a
//! `TableViewData::TypedTable` heap value (`Ptr(HeapKind::TableView)`
//! carrier). It is a method on `VirtualMachine` because it consults
//! `self.program.type_schema_registry` to resolve field names and types
//! from the compiled schema ID.
//!
//! Migration source: the pre-strict-typing body lived as
//! `builtin_make_table_from_rows(Vec<ValueWord>) -> Result<ValueWord, …>`
//! and read field values through the deleted `ValueWord::as_*` accessors.
//! Wave 5e re-introduces it on `&[KindedSlot]`; per-field value extraction
//! dispatches on each slot's `NativeKind` at the body site (the §2.7.6
//! heterogeneous-kind body pattern) — no `ValueWord`, no coercion opcode,
//! no new dispatch path.

use crate::executor::VirtualMachine;
use arrow_array::{BooleanArray, Float64Array, Int64Array, StringArray};
use arrow_schema::{DataType, Field, Schema};
use shape_value::datatable::DataTable;
use shape_value::heap_value::{HeapKind, HeapValue, TableViewData};
use shape_value::{KindedSlot, NativeKind, ValueSlot, VMError};
use std::sync::Arc;

/// Read a `KindedSlot` as `i64` for an integer-typed table column.
/// Integer-family and float kinds coerce; everything else is 0.
#[inline]
fn cell_as_i64(slot: &KindedSlot) -> i64 {
    match slot.kind {
        k if k.is_integer_family() => slot.as_i64().unwrap_or(0),
        NativeKind::Float64 => slot.as_f64().unwrap_or(0.0) as i64,
        _ => 0,
    }
}

/// Read a `KindedSlot` as `f64` for a float/decimal-typed table column.
#[inline]
fn cell_as_f64(slot: &KindedSlot) -> f64 {
    match slot.kind {
        NativeKind::Float64 => slot.as_f64().unwrap_or(0.0),
        k if k.is_integer_family() => slot.as_i64().unwrap_or(0) as f64,
        _ => 0.0,
    }
}

/// Read a `KindedSlot` as `bool` for a boolean-typed table column.
#[inline]
fn cell_as_bool(slot: &KindedSlot) -> bool {
    match slot.kind {
        NativeKind::Bool => slot.as_bool().unwrap_or(false),
        _ => false,
    }
}

/// Read a `KindedSlot` as an owned `String` for a string-typed table
/// column. String kinds yield their text; numeric kinds stringify.
fn cell_as_string(slot: &KindedSlot) -> String {
    match slot.kind {
        NativeKind::String => slot.as_str().map(str::to_string).unwrap_or_default(),
        NativeKind::Ptr(HeapKind::String) => match slot.slot.as_heap_value() {
            HeapValue::String(s) => s.as_ref().clone(),
            _ => String::new(),
        },
        NativeKind::StringV2 => {
            let bits = slot.slot.raw();
            if bits == 0 {
                String::new()
            } else {
                // SAFETY: kind=StringV2 means bits is a live `*const
                // StringObj` whose refcount the carrier holds a share of.
                let ptr = bits as *const shape_value::v2::string_obj::StringObj;
                unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr).to_string() }
            }
        }
        NativeKind::Int64 => slot.as_i64().map(|i| i.to_string()).unwrap_or_default(),
        NativeKind::Float64 => slot.as_f64().map(|f| f.to_string()).unwrap_or_default(),
        NativeKind::Bool => slot.as_bool().map(|b| b.to_string()).unwrap_or_default(),
        _ => String::new(),
    }
}

impl VirtualMachine {
    /// `MakeTableFromRows` — build a typed `Table<T>` from a row-major flat
    /// value buffer.
    ///
    /// Argument layout (emitted by `compiler::expressions::collections::
    /// compile_table_rows`): `[schema_id, row_count, field_count,
    /// v[0][0], v[0][1], …, v[r-1][f-1]]`. The result is a
    /// `TableViewData::TypedTable` heap value carried as
    /// `Ptr(HeapKind::TableView)`.
    pub(in crate::executor) fn builtin_make_table_from_rows(
        &self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        if args.len() < 3 {
            return Err(VMError::RuntimeError(
                "MakeTableFromRows requires at least 3 args \
                 (schema_id, row_count, field_count)"
                    .to_string(),
            ));
        }

        let schema_id = cell_as_i64(&args[0]) as u32;
        let row_count = cell_as_i64(&args[1]) as usize;
        let field_count = cell_as_i64(&args[2]) as usize;

        let expected_vals = row_count * field_count;
        if args.len() != 3 + expected_vals {
            return Err(VMError::RuntimeError(format!(
                "MakeTableFromRows: expected {} values ({} rows × {} fields), got {}",
                expected_vals,
                row_count,
                field_count,
                args.len() - 3
            )));
        }

        // Resolve field names and types from the compiled schema.
        let schema = self
            .program
            .type_schema_registry
            .get_by_id(schema_id)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "MakeTableFromRows: unknown schema ID {}",
                    schema_id
                ))
            })?;

        if schema.fields.len() != field_count {
            return Err(VMError::RuntimeError(format!(
                "MakeTableFromRows: schema has {} fields but field_count is {}",
                schema.fields.len(),
                field_count
            )));
        }

        let type_name = schema.name.clone();
        let values = &args[3..];

        // Build Arrow columns from the row-major values.
        let mut arrow_fields: Vec<Field> = Vec::with_capacity(field_count);
        let mut columns: Vec<arrow_array::ArrayRef> = Vec::with_capacity(field_count);

        for col_idx in 0..field_count {
            let field_def = &schema.fields[col_idx];
            let field_name = field_def.name.clone();
            let col_values: Vec<&KindedSlot> = (0..row_count)
                .map(|row_idx| &values[row_idx * field_count + col_idx])
                .collect();

            use shape_runtime::type_schema::FieldType;
            match &field_def.field_type {
                FieldType::I64
                | FieldType::Timestamp
                | FieldType::I8
                | FieldType::U8
                | FieldType::I16
                | FieldType::U16
                | FieldType::I32
                | FieldType::U32
                | FieldType::U64 => {
                    let arr: Vec<i64> =
                        col_values.iter().map(|v| cell_as_i64(v)).collect();
                    arrow_fields.push(Field::new(field_name, DataType::Int64, false));
                    columns.push(Arc::new(Int64Array::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::F64 | FieldType::Decimal => {
                    let arr: Vec<f64> =
                        col_values.iter().map(|v| cell_as_f64(v)).collect();
                    arrow_fields.push(Field::new(field_name, DataType::Float64, false));
                    columns
                        .push(Arc::new(Float64Array::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::Bool => {
                    let arr: Vec<bool> =
                        col_values.iter().map(|v| cell_as_bool(v)).collect();
                    arrow_fields.push(Field::new(field_name, DataType::Boolean, false));
                    columns
                        .push(Arc::new(BooleanArray::from(arr)) as arrow_array::ArrayRef);
                }
                FieldType::String
                | FieldType::Object(_)
                | FieldType::Any
                | FieldType::Array(_)
                | FieldType::Option(_)
                // W17.3-4.1 — HashMap<K, V> / Set<T> heap-resident
                // containers stringify into Utf8 columns at the
                // DataTable-construction boundary (same shape as
                // Array/Object/Option pre-W17.3-4). Container-aware
                // Arrow column projection is W17.3-4.3 territory
                // (runtime dispatch + snapshot/wire integration).
                | FieldType::HashMap { .. }
                | FieldType::Set(_) => {
                    let arr: Vec<String> =
                        col_values.iter().map(|v| cell_as_string(v)).collect();
                    arrow_fields.push(Field::new(field_name, DataType::Utf8, false));
                    columns
                        .push(Arc::new(StringArray::from(arr)) as arrow_array::ArrayRef);
                }
            }
        }

        let arrow_schema = Arc::new(Schema::new(arrow_fields));
        let batch =
            arrow_array::RecordBatch::try_new(arrow_schema, columns).map_err(|e| {
                VMError::RuntimeError(format!(
                    "MakeTableFromRows: failed to create RecordBatch: {}",
                    e
                ))
            })?;

        let dt = DataTable::with_type_name(batch, type_name).with_schema_id(schema_id);
        let table = Arc::new(dt);

        // Wrap in a TypedTable TableView — carrier
        // `Ptr(HeapKind::TableView)`, bits = `Arc::into_raw::<TableViewData>`.
        let tv = Arc::new(TableViewData::TypedTable {
            schema_id: schema_id as u64,
            table,
        });
        let bits = Arc::into_raw(tv) as u64;
        Ok(KindedSlot::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::TableView),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::{FieldType, TypeSchema};

    /// Build a minimal `TypeSchema` for testing without going through the
    /// full compiler — id, name, fields are all that the table builder
    /// reads.
    fn make_schema(id: u32, name: &str, fields: &[(&str, FieldType)]) -> TypeSchema {
        let field_defs: Vec<(String, FieldType)> = fields
            .iter()
            .map(|(n, t)| (n.to_string(), t.clone()))
            .collect();
        TypeSchema::with_id(id, name, field_defs)
    }

    /// Drive `builtin_make_table_from_rows` against a VM whose program
    /// registry has been seeded with `schema`, returning `(rows, cols)` of
    /// the produced table.
    fn run_table_build(
        schema: TypeSchema,
        args: Vec<KindedSlot>,
    ) -> Result<(usize, usize), VMError> {
        let mut vm = VirtualMachine::new(crate::VMConfig::default());
        vm.program.type_schema_registry.register(schema);
        let result = vm.builtin_make_table_from_rows(&args)?;
        assert_eq!(
            result.kind,
            NativeKind::Ptr(HeapKind::TableView),
            "MakeTableFromRows must return a TableView-kind slot"
        );
        let bits = result.slot.raw();
        assert_ne!(bits, 0, "TableView slot bits must be non-null");
        // SAFETY: kind == Ptr(TableView) ⇒ bits = Arc::into_raw::<TableViewData>;
        // the result `KindedSlot` owns one strong-count share for the borrow.
        // `slot.as_heap_value()` would be unsound here — the bits are an
        // `Arc<TableViewData>` pointer, not a `*const HeapValue`.
        let tv: &TableViewData = unsafe { &*(bits as *const TableViewData) };
        match tv {
            TableViewData::TypedTable { table, .. } => {
                Ok((table.row_count(), table.column_count()))
            }
            other => panic!("expected TypedTable, got {:?}", other),
        }
    }

    #[test]
    fn make_table_from_rows_basic() {
        // type T { id: int, score: number, ok: bool }, 2 rows.
        let schema = make_schema(
            7000,
            "T",
            &[
                ("id", FieldType::I64),
                ("score", FieldType::F64),
                ("ok", FieldType::Bool),
            ],
        );
        let args = vec![
            KindedSlot::from_int(7000), // schema_id
            KindedSlot::from_int(2),    // row_count
            KindedSlot::from_int(3),    // field_count
            // row 0
            KindedSlot::from_int(1),
            KindedSlot::from_number(9.5),
            KindedSlot::from_bool(true),
            // row 1
            KindedSlot::from_int(2),
            KindedSlot::from_number(8.0),
            KindedSlot::from_bool(false),
        ];
        let (rows, cols) = run_table_build(schema, args).expect("build must not panic");
        assert_eq!((rows, cols), (2, 3));
    }

    #[test]
    fn make_table_from_rows_string_column() {
        let schema = make_schema(
            7001,
            "Person",
            &[("name", FieldType::String), ("age", FieldType::I64)],
        );
        let args = vec![
            KindedSlot::from_int(7001),
            KindedSlot::from_int(2),
            KindedSlot::from_int(2),
            KindedSlot::from_string("Ada"),
            KindedSlot::from_int(36),
            KindedSlot::from_string("Grace"),
            KindedSlot::from_int(85),
        ];
        let (rows, cols) = run_table_build(schema, args).expect("build must not panic");
        assert_eq!((rows, cols), (2, 2));
    }

    #[test]
    fn make_table_from_rows_empty_rows() {
        let schema = make_schema(7002, "Empty", &[("v", FieldType::I64)]);
        let args = vec![
            KindedSlot::from_int(7002),
            KindedSlot::from_int(0),
            KindedSlot::from_int(1),
        ];
        let (rows, cols) = run_table_build(schema, args).expect("build must not panic");
        assert_eq!((rows, cols), (0, 1));
    }

    #[test]
    fn make_table_from_rows_rejects_value_count_mismatch() {
        let schema = make_schema(7003, "T", &[("a", FieldType::I64)]);
        // Claims 2 rows × 1 field = 2 values, but only 1 supplied.
        let args = vec![
            KindedSlot::from_int(7003),
            KindedSlot::from_int(2),
            KindedSlot::from_int(1),
            KindedSlot::from_int(42),
        ];
        let mut vm = VirtualMachine::new(crate::VMConfig::default());
        vm.program.type_schema_registry.register(schema);
        assert!(vm.builtin_make_table_from_rows(&args).is_err());
    }

    #[test]
    fn make_table_from_rows_rejects_unknown_schema() {
        let args = vec![
            KindedSlot::from_int(999_999),
            KindedSlot::from_int(0),
            KindedSlot::from_int(1),
        ];
        let vm = VirtualMachine::new(crate::VMConfig::default());
        assert!(vm.builtin_make_table_from_rows(&args).is_err());
    }

    #[test]
    fn make_table_from_rows_rejects_too_few_args() {
        let vm = VirtualMachine::new(crate::VMConfig::default());
        assert!(
            vm.builtin_make_table_from_rows(&[KindedSlot::from_int(1)])
                .is_err()
        );
    }
}
