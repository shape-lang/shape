//! VM-native value formatting
//!
//! This module provides native formatting for ValueWord values without converting to
//! runtime values. It respects TypeSchema for TypedObjects with their field names.
//!
//! The primary entry point is `format_nb()` which formats ValueWord values directly
//! using NanTag/HeapValue dispatch.

use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_runtime::type_schema::field_types::FieldType;
use shape_runtime::type_system::annotation_to_string;
use shape_value::heap_value::HeapValue;
use shape_value::{NanTag, ValueWord};

/// Formatter for ValueWord values
///
/// Uses TypeSchemaRegistry to format TypedObjects with their field names.
pub struct ValueFormatter<'a> {
    /// Type schema registry for TypedObject field resolution
    schema_registry: &'a TypeSchemaRegistry,
}

/// Backward-compat alias used by test code.
#[cfg(test)]
pub type VMValueFormatter<'a> = ValueFormatter<'a>;

impl<'a> ValueFormatter<'a> {
    /// Create a new formatter
    pub fn new(schema_registry: &'a TypeSchemaRegistry) -> Self {
        Self { schema_registry }
    }

    /// Format a ValueWord to string (test-only, delegates to ValueWord path)
    #[cfg(test)]
    pub fn format(&self, value: &ValueWord) -> String {
        let nb = value.clone();
        self.format_nb_with_depth(&nb, 0)
    }

    /// Format a ValueWord value to string — primary entry point.
    ///
    /// Uses NanTag/HeapValue dispatch for inline types (f64, i48, bool, None,
    /// Unit) and heap types (String, Array, TypedObject, Decimal, etc.).
    pub fn format_nb(&self, value: &ValueWord) -> String {
        self.format_nb_with_depth(value, 0)
    }

    /// Format a ValueWord value with depth tracking.
    fn format_nb_with_depth(&self, value: &ValueWord, depth: usize) -> String {
        if depth > 50 {
            return "[max depth reached]".to_string();
        }

        // Fast path: inline types (no heap access needed)
        match value.tag() {
            NanTag::F64 => {
                if let Some(n) = value.as_f64() {
                    return format_number(n);
                }
                // Shouldn't happen, but fallback
                return "NaN".to_string();
            }
            NanTag::I48 => {
                if let Some(i) = value.as_i64() {
                    return i.to_string();
                }
                return "0".to_string();
            }
            NanTag::Bool => {
                if let Some(b) = value.as_bool() {
                    return b.to_string();
                }
                return "false".to_string();
            }
            NanTag::None => return "None".to_string(),
            NanTag::Unit => return "()".to_string(),
            NanTag::Function => {
                if let Some(id) = value.as_function() {
                    return format!("[Function:{}]", id);
                }
                return "[Function]".to_string();
            }
            NanTag::ModuleFunction => return "[ModuleFunction]".to_string(),
            NanTag::Ref => return "&ref".to_string(),
            NanTag::Heap => {}
        }

        // Heap path: dispatch on HeapValue variant
        match value.as_heap_ref() {
            Some(HeapValue::String(s)) => s.as_ref().clone(),
            Some(HeapValue::Array(arr)) => self.format_nanboxed_array(arr.as_ref(), depth),
            Some(HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            }) => self.format_typed_object(*schema_id as u32, slots, *heap_mask, depth),
            Some(HeapValue::Decimal(d)) => format!("{}D", d),
            Some(HeapValue::BigInt(i)) => i.to_string(),
            Some(HeapValue::Closure { function_id, .. }) => format!("[Closure:{}]", function_id),
            Some(HeapValue::HostClosure(_)) => "<HostClosure>".to_string(),
            Some(HeapValue::DataTable(dt)) => format!("{}", dt),
            Some(HeapValue::TypedTable { table, .. }) => format!("{}", table),
            Some(HeapValue::RowView { table, row_idx, .. }) => {
                format!("[Row {} of {} rows]", row_idx, table.row_count())
            }
            Some(HeapValue::ColumnRef { table, col_id, .. }) => {
                let col = table.inner().column(*col_id as usize);
                let dtype = col.data_type();
                let type_str = match dtype {
                    arrow_schema::DataType::Float64 => "f64",
                    arrow_schema::DataType::Int64 => "i64",
                    arrow_schema::DataType::Boolean => "bool",
                    arrow_schema::DataType::Utf8 | arrow_schema::DataType::LargeUtf8 => "string",
                    _ => "unknown",
                };
                let name = table
                    .column_names()
                    .get(*col_id as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("col_{}", col_id));
                format!("Column<{}>({}, {} rows)", type_str, name, col.len())
            }
            Some(HeapValue::IndexedTable {
                table, index_col, ..
            }) => {
                let col_name = table
                    .column_names()
                    .get(*index_col as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("col_{}", index_col));
                format!(
                    "IndexedTable({} rows, index: {})",
                    table.row_count(),
                    col_name
                )
            }
            Some(HeapValue::Range {
                start,
                end,
                inclusive,
            }) => {
                let start_str = start
                    .as_ref()
                    .map(|s| self.format_nb_with_depth(s, depth + 1))
                    .unwrap_or_default();
                let end_str = end
                    .as_ref()
                    .map(|e| self.format_nb_with_depth(e, depth + 1))
                    .unwrap_or_default();
                let op = if *inclusive { "..=" } else { ".." };
                format!("{}{}{}", start_str, op, end_str)
            }
            Some(HeapValue::Enum(e)) => format!("{:?}", e),
            Some(HeapValue::Some(inner)) => {
                format!("Some({})", self.format_nb_with_depth(inner, depth + 1))
            }
            Some(HeapValue::Ok(inner)) => {
                format!("Ok({})", self.format_nb_with_depth(inner, depth + 1))
            }
            Some(HeapValue::Err(inner)) => {
                format!("Err({})", self.format_nb_with_depth(inner, depth + 1))
            }
            Some(HeapValue::Future(id)) => format!("[Future:{}]", id),
            Some(HeapValue::TaskGroup { kind, task_ids }) => {
                let kind_str = match kind {
                    0 => "All",
                    1 => "Race",
                    2 => "Any",
                    3 => "Settle",
                    _ => "Unknown",
                };
                format!("[TaskGroup:{}({})]", kind_str, task_ids.len())
            }
            Some(HeapValue::TraitObject { value, .. }) => {
                self.format_nb_with_depth(value, depth + 1)
            }
            Some(HeapValue::ExprProxy(col)) => format!("<ExprProxy:{}>", col),
            Some(HeapValue::FilterExpr(node)) => format!("<FilterExpr:{:?}>", node),
            Some(HeapValue::Time(t)) => t.to_rfc3339(),
            Some(HeapValue::Duration(duration)) => format!("{}{:?}", duration.value, duration.unit),
            Some(HeapValue::TimeSpan(ts)) => format!("{:?}", ts),
            Some(HeapValue::Timeframe(tf)) => format!("{:?}", tf),
            Some(HeapValue::TimeReference(value)) => format!("{:?}", value),
            Some(HeapValue::DateTimeExpr(value)) => format!("{:?}", value),
            Some(HeapValue::DataDateTimeRef(value)) => format!("{:?}", value),
            Some(HeapValue::TypeAnnotation(value)) => annotation_to_string(value),
            Some(HeapValue::TypeAnnotatedValue { value, .. }) => {
                self.format_nb_with_depth(value, depth + 1)
            }
            Some(HeapValue::PrintResult(p)) => p.rendered.clone(),
            Some(HeapValue::SimulationCall(_)) => "[SimulationCall]".to_string(),
            Some(HeapValue::FunctionRef { .. }) => "[FunctionRef]".to_string(),
            Some(HeapValue::DataReference(_)) => "[DataReference]".to_string(),
            Some(HeapValue::NativeScalar(v)) => v.to_string(),
            Some(HeapValue::NativeView(v)) => format!(
                "<{}:{}@0x{:x}>",
                if v.mutable { "cmut" } else { "cview" },
                v.layout.name,
                v.ptr
            ),
            Some(HeapValue::HashMap(d)) => {
                let mut parts = Vec::new();
                for (k, v) in d.keys.iter().zip(d.values.iter()) {
                    parts.push(format!(
                        "{}: {}",
                        self.format_nb_with_depth(k, depth + 1),
                        self.format_nb_with_depth(v, depth + 1)
                    ));
                }
                format!("HashMap{{{}}}", parts.join(", "))
            }
            Some(HeapValue::Set(d)) => {
                let parts: Vec<String> = d
                    .items
                    .iter()
                    .map(|v| self.format_nb_with_depth(v, depth + 1))
                    .collect();
                format!("Set{{{}}}", parts.join(", "))
            }
            Some(HeapValue::Deque(d)) => {
                let parts: Vec<String> = d
                    .items
                    .iter()
                    .map(|v| self.format_nb_with_depth(v, depth + 1))
                    .collect();
                format!("Deque[{}]", parts.join(", "))
            }
            Some(HeapValue::PriorityQueue(d)) => {
                let parts: Vec<String> = d
                    .items
                    .iter()
                    .map(|v| self.format_nb_with_depth(v, depth + 1))
                    .collect();
                format!("PriorityQueue[{}]", parts.join(", "))
            }
            Some(HeapValue::Content(node)) => format!("{}", node),
            Some(HeapValue::Instant(t)) => format!("<instant:{:?}>", t.elapsed()),
            Some(HeapValue::IoHandle(data)) => {
                let status = if data.is_open() { "open" } else { "closed" };
                format!("<io_handle:{}:{}>", data.path, status)
            }
            Some(HeapValue::SharedCell(arc)) => {
                self.format_nb_with_depth(&arc.read().unwrap(), depth)
            }
            Some(HeapValue::IntArray(a)) => {
                let elems: Vec<String> = a.iter().map(|v| v.to_string()).collect();
                format!("Vec<int>[{}]", elems.join(", "))
            }
            Some(HeapValue::FloatArray(a)) => {
                let elems: Vec<String> = a
                    .iter()
                    .map(|v| {
                        if *v == v.trunc() && v.abs() < 1e15 {
                            format!("{}", *v as i64)
                        } else {
                            format!("{}", v)
                        }
                    })
                    .collect();
                format!("Vec<number>[{}]", elems.join(", "))
            }
            Some(HeapValue::BoolArray(a)) => {
                let elems: Vec<String> = a
                    .iter()
                    .map(|v| if *v != 0 { "true" } else { "false" }.to_string())
                    .collect();
                format!("Vec<bool>[{}]", elems.join(", "))
            }
            Some(HeapValue::I8Array(a)) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("Vec<i8>[{}]", elems.join(", "))
            }
            Some(HeapValue::I16Array(a)) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("Vec<i16>[{}]", elems.join(", "))
            }
            Some(HeapValue::I32Array(a)) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("Vec<i32>[{}]", elems.join(", "))
            }
            Some(HeapValue::U8Array(a)) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("Vec<u8>[{}]", elems.join(", "))
            }
            Some(HeapValue::U16Array(a)) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("Vec<u16>[{}]", elems.join(", "))
            }
            Some(HeapValue::U32Array(a)) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("Vec<u32>[{}]", elems.join(", "))
            }
            Some(HeapValue::U64Array(a)) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("Vec<u64>[{}]", elems.join(", "))
            }
            Some(HeapValue::F32Array(a)) => {
                let elems: Vec<String> = a
                    .data
                    .iter()
                    .map(|v| {
                        if *v == v.trunc() && v.abs() < 1e15 {
                            format!("{}", *v as i64)
                        } else {
                            format!("{}", v)
                        }
                    })
                    .collect();
                format!("Vec<f32>[{}]", elems.join(", "))
            }
            Some(HeapValue::Matrix(m)) => {
                format!("<Mat<number>:{}x{}>", m.rows, m.cols)
            }
            Some(HeapValue::Iterator(it)) => {
                format!("<iterator:pos={}>", it.position)
            }
            Some(HeapValue::Generator(g)) => {
                format!("<generator:state={}>", g.state)
            }
            Some(HeapValue::Mutex(_)) => "<mutex>".to_string(),
            Some(HeapValue::Atomic(a)) => {
                format!(
                    "<atomic:{}>",
                    a.inner.load(std::sync::atomic::Ordering::Relaxed)
                )
            }
            Some(HeapValue::Lazy(l)) => {
                let initialized = l.value.lock().map(|g| g.is_some()).unwrap_or(false);
                if initialized {
                    "<lazy:initialized>".to_string()
                } else {
                    "<lazy:pending>".to_string()
                }
            }
            Some(HeapValue::Channel(c)) => {
                if c.is_sender() {
                    "<channel:sender>".to_string()
                } else {
                    "<channel:receiver>".to_string()
                }
            }
            None => format!("<unknown:{}>", value.type_name()),
        }
    }

    /// Format an array of ValueWord values
    fn format_nanboxed_array(&self, arr: &[ValueWord], depth: usize) -> String {
        let elements: Vec<String> = arr
            .iter()
            .map(|nb| self.format_nb_with_depth(nb, depth + 1))
            .collect();
        format!("[{}]", elements.join(", "))
    }

    /// Format a TypedObject using its schema
    fn format_typed_object(
        &self,
        schema_id: u32,
        slots: &[shape_value::ValueSlot],
        heap_mask: u64,
        depth: usize,
    ) -> String {
        let schema_ref = self.schema_registry.get_by_id(schema_id);
        let schema = if let Some(s) = schema_ref {
            s
        } else {
            let nb = ValueWord::from_heap_value(HeapValue::TypedObject {
                schema_id: schema_id as u64,
                slots: slots.to_vec().into_boxed_slice(),
                heap_mask,
            });
            if let Some(map) = shape_runtime::type_schema::typed_object_to_hashmap_nb(&nb) {
                let mut fields: Vec<(String, String)> = map
                    .into_iter()
                    .map(|(k, v)| (k, self.format_nb_with_depth(&v, depth + 1)))
                    .collect();
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                return format!(
                    "{{ {} }}",
                    fields
                        .into_iter()
                        .map(|(k, v)| format!("{}: {}", k, v))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            return format!("[TypedObject:{}]", schema_id);
        };

        // Check if this is an enum type - format specially
        if let Some(enum_info) = schema.get_enum_info() {
            return self.format_enum(&schema.name, enum_info, slots, heap_mask, depth);
        }

        // Default format: { field1: val1, field2: val2, ... }
        let mut fields = Vec::new();

        for field in &schema.fields {
            let field_index = field.index as usize;
            if field_index < slots.len() {
                let formatted = self.format_slot_value(
                    slots,
                    heap_mask,
                    field_index,
                    &field.field_type,
                    depth + 1,
                );
                fields.push(format!("{}: {}", field.name, formatted));
            }
        }

        if fields.is_empty() {
            "{}".to_string()
        } else {
            format!("{{ {} }}", fields.join(", "))
        }
    }

    /// Format an enum value using its variant info
    fn format_enum(
        &self,
        enum_name: &str,
        enum_info: &shape_runtime::type_schema::EnumInfo,
        slots: &[shape_value::ValueSlot],
        heap_mask: u64,
        depth: usize,
    ) -> String {
        // Read variant ID from slot 0
        if slots.is_empty() {
            return format!("{}::?", enum_name);
        }

        let variant_id = slots[0].as_i64() as u16;

        // Look up variant by ID
        let variant = match enum_info.variant_by_id(variant_id) {
            Some(v) => v,
            None => return format!("{}::?[{}]", enum_name, variant_id),
        };

        // Unit variant (no payload)
        if variant.payload_fields == 0 {
            return format!("{}::{}", enum_name, variant.name);
        }

        // Variant with payload - read payload values from slots 1+
        let mut payload_values = Vec::new();
        for i in 0..variant.payload_fields {
            let slot_idx = 1 + i as usize;
            if slot_idx < slots.len() {
                payload_values.push(self.format_slot_value(
                    slots,
                    heap_mask,
                    slot_idx,
                    &FieldType::Any,
                    depth + 1,
                ));
            }
        }

        if payload_values.is_empty() {
            format!("{}::{}", enum_name, variant.name)
        } else if payload_values.len() == 1 {
            // Single payload - use parentheses style
            format!("{}::{}({})", enum_name, variant.name, payload_values[0])
        } else {
            // Multiple payloads - use tuple style with variant name
            format!(
                "{}::{}({})",
                enum_name,
                variant.name,
                payload_values.join(", ")
            )
        }
    }

    /// Format a TypedObject slot value directly from its ValueSlot.
    /// Heap slots are converted via `as_heap_nb()` and formatted with `format_nb_with_depth`.
    /// Non-heap slots are dispatched by field type to read the correct representation.
    fn format_slot_value(
        &self,
        slots: &[shape_value::ValueSlot],
        heap_mask: u64,
        index: usize,
        field_type: &FieldType,
        depth: usize,
    ) -> String {
        if index >= slots.len() {
            return "None".to_string();
        }
        let slot = &slots[index];
        if heap_mask & (1u64 << index) != 0 {
            // Heap slot: read as HeapValue, wrap in ValueWord, format directly
            let nb = slot.as_heap_nb();
            self.format_nb_with_depth(&nb, depth)
        } else {
            // Non-heap: dispatch on field type to read raw bits correctly
            match field_type {
                FieldType::I64 | FieldType::Timestamp => slot.as_i64().to_string(),
                FieldType::Bool => slot.as_bool().to_string(),
                FieldType::F64 | FieldType::Decimal => format_number(slot.as_f64()),
                // Any other non-heap type: reconstruct via as_value_word to
                // preserve inline NanTag variants for correct Display formatting
                _ => {
                    let vw = slot.as_value_word(false);
                    self.format_nb_with_depth(&vw, depth)
                }
            }
        }
    }
}

/// Format a number, removing unnecessary decimal places
fn format_number(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_string()
    } else if n.is_infinite() {
        if n.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        // Integer-like numbers: show without decimal
        format!("{}", n as i64)
    } else {
        // Use default formatting
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_registry() -> TypeSchemaRegistry {
        TypeSchemaRegistry::new()
    }

    fn predeclared_object(fields: &[(&str, ValueWord)]) -> ValueWord {
        let field_names: Vec<String> = fields.iter().map(|(name, _)| (*name).to_string()).collect();
        let _ = shape_runtime::type_schema::register_predeclared_any_schema(&field_names);
        shape_runtime::type_schema::typed_object_from_pairs(fields)
    }

    #[test]
    fn test_format_primitives() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        assert_eq!(formatter.format(&ValueWord::from_f64(42.0)), "42");
        assert_eq!(formatter.format(&ValueWord::from_f64(3.14)), "3.14");
        assert_eq!(
            formatter.format(&ValueWord::from_string(Arc::new("hello".to_string()))),
            "hello"
        );
        assert_eq!(formatter.format(&ValueWord::from_bool(true)), "true");
        assert_eq!(formatter.format(&ValueWord::none()), "None");
        assert_eq!(formatter.format(&ValueWord::unit()), "()");
    }

    #[test]
    fn test_format_array() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        let arr = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
        ]));
        assert_eq!(formatter.format(&arr), "[1, 2, 3]");
    }

    #[test]
    fn test_format_object() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        let value = predeclared_object(&[
            ("x", ValueWord::from_f64(1.0)),
            ("y", ValueWord::from_f64(2.0)),
        ]);

        let formatted = formatter.format(&value);
        // TypedObject fields come from schema order
        assert!(formatted.contains("x: 1"));
        assert!(formatted.contains("y: 2"));
    }

    #[test]
    fn test_format_number_integers() {
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(-100.0), "-100");
        assert_eq!(format_number(0.0), "0");
    }

    #[test]
    fn test_format_number_decimals() {
        assert_eq!(format_number(3.14), "3.14");
        assert_eq!(format_number(-2.5), "-2.5");
    }

    #[test]
    fn test_format_number_special() {
        assert_eq!(format_number(f64::NAN), "NaN");
        assert_eq!(format_number(f64::INFINITY), "Infinity");
        assert_eq!(format_number(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn test_format_decimal() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        let d = ValueWord::from_decimal(rust_decimal::Decimal::from(42));
        assert_eq!(formatter.format(&d), "42D");

        let d2 = ValueWord::from_decimal(rust_decimal::Decimal::new(314, 2)); // 3.14 exactly
        assert_eq!(formatter.format(&d2), "3.14D");
    }

    #[test]
    fn test_format_int() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        assert_eq!(formatter.format(&ValueWord::from_i64(42)), "42");
        assert_eq!(formatter.format(&ValueWord::from_i64(-100)), "-100");
        assert_eq!(formatter.format(&ValueWord::from_i64(0)), "0");
    }

    // ===== ValueWord format_nb tests =====

    #[test]
    fn test_format_nb_primitives() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        assert_eq!(formatter.format_nb(&ValueWord::from_f64(42.0)), "42");
        assert_eq!(formatter.format_nb(&ValueWord::from_f64(3.14)), "3.14");
        assert_eq!(
            formatter.format_nb(&ValueWord::from_string(Arc::new("hello".to_string()))),
            "hello"
        );
        assert_eq!(formatter.format_nb(&ValueWord::from_bool(true)), "true");
        assert_eq!(formatter.format_nb(&ValueWord::from_bool(false)), "false");
        assert_eq!(formatter.format_nb(&ValueWord::none()), "None");
        assert_eq!(formatter.format_nb(&ValueWord::unit()), "()");
    }

    #[test]
    fn test_format_nb_integers() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        assert_eq!(formatter.format_nb(&ValueWord::from_i64(42)), "42");
        assert_eq!(formatter.format_nb(&ValueWord::from_i64(-100)), "-100");
        assert_eq!(formatter.format_nb(&ValueWord::from_i64(0)), "0");
    }

    #[test]
    fn test_format_nb_decimal() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        assert_eq!(
            formatter.format_nb(&ValueWord::from_decimal(rust_decimal::Decimal::from(42))),
            "42D"
        );
        assert_eq!(
            formatter.format_nb(&ValueWord::from_decimal(rust_decimal::Decimal::new(314, 2))),
            "3.14D"
        );
    }

    #[test]
    fn test_format_nb_array() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        let arr = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
        ]));
        assert_eq!(formatter.format_nb(&arr), "[1, 2, 3]");
    }

    #[test]
    fn test_format_nb_mixed_array() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        let arr = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_string(Arc::new("two".to_string())),
            ValueWord::from_bool(true),
        ]));
        assert_eq!(formatter.format_nb(&arr), "[1, two, true]");
    }

    #[test]
    fn test_format_nb_object() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        let value = predeclared_object(&[
            ("x", ValueWord::from_f64(1.0)),
            ("y", ValueWord::from_f64(2.0)),
        ]);
        let nb = value;

        let formatted = formatter.format_nb(&nb);
        assert!(formatted.contains("x: 1"));
        assert!(formatted.contains("y: 2"));
    }

    #[test]
    fn test_format_nb_special_numbers() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        assert_eq!(formatter.format_nb(&ValueWord::from_f64(f64::NAN)), "NaN");
        assert_eq!(
            formatter.format_nb(&ValueWord::from_f64(f64::INFINITY)),
            "Infinity"
        );
        assert_eq!(
            formatter.format_nb(&ValueWord::from_f64(f64::NEG_INFINITY)),
            "-Infinity"
        );
    }

    #[test]
    fn test_format_nb_result_types() {
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        assert_eq!(
            formatter.format_nb(&ValueWord::from_ok(ValueWord::from_i64(42))),
            "Ok(42)"
        );
        assert_eq!(
            formatter.format_nb(&ValueWord::from_err(ValueWord::from_string(Arc::new(
                "fail".to_string()
            )))),
            "Err(fail)"
        );
        assert_eq!(
            formatter.format_nb(&ValueWord::from_some(ValueWord::from_f64(3.14))),
            "Some(3.14)"
        );
    }

    #[test]
    fn test_format_nb_consistency_with_vmvalue() {
        // Verify that format_nb produces the same output as format for common types
        let schema_reg = create_test_registry();
        let formatter = VMValueFormatter::new(&schema_reg);

        let test_cases: Vec<(ValueWord, ValueWord)> = vec![
            (ValueWord::from_f64(42.0), ValueWord::from_f64(42.0)),
            (ValueWord::from_f64(3.14), ValueWord::from_f64(3.14)),
            (ValueWord::from_i64(99), ValueWord::from_i64(99)),
            (ValueWord::from_bool(true), ValueWord::from_bool(true)),
            (ValueWord::none(), ValueWord::none()),
            (ValueWord::unit(), ValueWord::unit()),
            (
                ValueWord::from_string(Arc::new("test".to_string())),
                ValueWord::from_string(Arc::new("test".to_string())),
            ),
        ];

        for (vmval, nb) in &test_cases {
            assert_eq!(
                formatter.format(vmval),
                formatter.format_nb(nb),
                "Mismatch for ValueWord: {:?}",
                vmval
            );
        }
    }
}
