//! Content trait dispatch — render any ValueWord value as a ContentNode.
//!
//! The `render_as_content` function implements the Content trait dispatch logic:
//! 1. If the value IS already a ContentNode → return as-is
//! 2. Match by NanTag/HeapValue type → built-in Content impls for primitives
//! 3. Fallback → Display as ContentNode::plain(display_string)
//!
//! Built-in Content implementations:
//! - string → ContentNode::plain(self)
//! - number/int/decimal → ContentNode::plain(formatted)
//! - bool → ContentNode::plain("true"/"false")
//! - Vec<TypedObject> → ContentNode::Table with columns from schema fields
//! - Vec<scalar> → ContentNode::plain("[1, 2, 3]")
//! - HashMap<K,V> → ContentNode::KeyValue
//! - TypedObject → ContentNode::KeyValue with field names from schema
//!
//! ## ContentFor<Adapter>
//!
//! The `render_as_content_for` function adds adapter-aware dispatch:
//! 1. ContentFor<CurrentAdapter> → adapter-specific rendering
//! 2. Content → generic content rendering
//! 3. Display fallback → plain text
//!
//! Adapter types: Terminal, Html, Markdown, Json, Plain

use crate::content_renderer::RendererCapabilities;
use crate::type_schema::{SchemaId, lookup_schema_by_id_public};
use shape_value::content::{BorderStyle, ContentNode, ContentTable};
use shape_value::heap_value::HeapValue;
use shape_value::value_word::NanTag;
use shape_value::{DataTable, ValueWord};

/// Well-known adapter names for ContentFor<Adapter> dispatch.
pub mod adapters {
    pub const TERMINAL: &str = "Terminal";
    pub const HTML: &str = "Html";
    pub const MARKDOWN: &str = "Markdown";
    pub const JSON: &str = "Json";
    pub const PLAIN: &str = "Plain";
}

/// Optional user-defined Content impl resolver.
///
/// When set, `render_as_content` calls this function before falling through to
/// built-in dispatch. If the resolver returns `Some(node)`, that node is used.
/// The resolver is typically set by the VM executor to check for user-defined
/// `impl Content for MyType { fn render(self) -> ContentNode }` blocks.
pub type UserContentResolver = dyn Fn(&ValueWord) -> Option<ContentNode> + Send + Sync;

static USER_CONTENT_RESOLVER: std::sync::OnceLock<Box<UserContentResolver>> =
    std::sync::OnceLock::new();

/// Register a user-defined Content trait resolver.
///
/// Called by the VM during initialization to enable user-implementable Content trait.
pub fn set_user_content_resolver(resolver: Box<UserContentResolver>) {
    let _ = USER_CONTENT_RESOLVER.set(resolver);
}

/// Render a ValueWord value as a ContentNode using Content dispatch.
///
/// Dispatch order:
/// 1. If value IS a ContentNode → return as-is
/// 2. If a user-defined Content impl exists → call user's render()
/// 3. If value type has a built-in Content impl → produce structured output
/// 4. Else → Display fallback → ContentNode::plain(display_string)
pub fn render_as_content(value: &ValueWord) -> ContentNode {
    // Fast path: already a content node
    if let Some(node) = value.as_content() {
        return node.clone();
    }

    // Check for user-defined Content impl
    if let Some(resolver) = USER_CONTENT_RESOLVER.get() {
        if let Some(node) = resolver(value) {
            return node;
        }
    }

    match value.tag() {
        NanTag::I48 => ContentNode::plain(format!("{}", value)),
        NanTag::F64 => ContentNode::plain(format!("{}", value)),
        NanTag::Bool => ContentNode::plain(format!("{}", value)),
        NanTag::None => ContentNode::plain("none".to_string()),
        NanTag::Unit => ContentNode::plain("()".to_string()),
        NanTag::Heap => render_heap_as_content(value),
        _ => ContentNode::plain(format!("{}", value)),
    }
}

/// Render a ValueWord value as a ContentNode with adapter-specific dispatch.
///
/// Dispatch order:
/// 1. ContentFor<adapter> → adapter-specific rendering (future: user-defined impls)
/// 2. Content → generic content rendering via `render_as_content`
/// 3. Display fallback → plain text
///
/// The `caps` parameter provides renderer capabilities so the Content impl
/// can adapt output (e.g., use ANSI codes only when `caps.ansi` is true).
/// Optional adapter-specific Content resolver.
pub type UserContentForResolver =
    dyn Fn(&ValueWord, &str, &RendererCapabilities) -> Option<ContentNode> + Send + Sync;

static USER_CONTENT_FOR_RESOLVER: std::sync::OnceLock<Box<UserContentForResolver>> =
    std::sync::OnceLock::new();

/// Register a user-defined ContentFor<Adapter> resolver.
pub fn set_user_content_for_resolver(resolver: Box<UserContentForResolver>) {
    let _ = USER_CONTENT_FOR_RESOLVER.set(resolver);
}

pub fn render_as_content_for(
    value: &ValueWord,
    adapter: &str,
    caps: &RendererCapabilities,
) -> ContentNode {
    // Check for user-defined ContentFor<Adapter> impl
    if let Some(resolver) = USER_CONTENT_FOR_RESOLVER.get() {
        if let Some(node) = resolver(value, adapter, caps) {
            return node;
        }
    }
    // Fall through to generic Content dispatch
    render_as_content(value)
}

/// Create a RendererCapabilities descriptor for a given adapter name.
pub fn capabilities_for_adapter(adapter: &str) -> RendererCapabilities {
    match adapter {
        adapters::TERMINAL => RendererCapabilities::terminal(),
        adapters::HTML => RendererCapabilities::html(),
        adapters::MARKDOWN => RendererCapabilities::markdown(),
        adapters::PLAIN => RendererCapabilities::plain(),
        adapters::JSON => RendererCapabilities {
            ansi: false,
            unicode: true,
            color: false,
            interactive: false,
        },
        _ => RendererCapabilities::plain(),
    }
}

/// Dispatch Content rendering for heap-allocated values.
fn render_heap_as_content(value: &ValueWord) -> ContentNode {
    // Handle unified arrays (bit-47 tagged).
    if shape_value::tags::is_unified_heap(value.raw_bits()) {
        let kind = unsafe { shape_value::tags::unified_heap_kind(value.raw_bits()) };
        if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
            if let Some(view) = value.as_any_array() {
                let elems = view.to_generic();
                return render_array_as_content(&elems);
            }
        }
    }
    match value.as_heap_ref() {
        Some(HeapValue::String(s)) => ContentNode::plain(s.as_ref().clone()),
        Some(HeapValue::Decimal(d)) => ContentNode::plain(d.to_string()),
        Some(HeapValue::BigInt(i)) => ContentNode::plain(i.to_string()),
        Some(HeapValue::Array(arr)) => render_array_as_content(arr),
        Some(HeapValue::HashMap(d)) => render_hashmap_as_content(&d.keys, &d.values),
        Some(HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        }) => render_typed_object_as_content(*schema_id, slots, *heap_mask),
        Some(HeapValue::DataTable(dt)) => datatable_to_content_node(dt, None),
        Some(HeapValue::TypedTable { table, .. }) => datatable_to_content_node(table, None),
        Some(HeapValue::IndexedTable { table, .. }) => datatable_to_content_node(table, None),
        // Typed arrays: render as plain text with bracket notation
        Some(HeapValue::IntArray(a)) => {
            let elems: Vec<String> = a.iter().map(|v| v.to_string()).collect();
            ContentNode::plain(format!("[{}]", elems.join(", ")))
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
            ContentNode::plain(format!("[{}]", elems.join(", ")))
        }
        Some(HeapValue::FloatArraySlice {
            parent,
            offset,
            len,
        }) => {
            let start = *offset as usize;
            let end = start + *len as usize;
            let elems: Vec<String> = parent.data[start..end]
                .iter()
                .map(|v| {
                    if *v == v.trunc() && v.abs() < 1e15 {
                        format!("{}", *v as i64)
                    } else {
                        format!("{}", v)
                    }
                })
                .collect();
            ContentNode::plain(format!("[{}]", elems.join(", ")))
        }
        Some(HeapValue::BoolArray(a)) => {
            let elems: Vec<String> = a
                .iter()
                .map(|v| if *v != 0 { "true" } else { "false" }.to_string())
                .collect();
            ContentNode::plain(format!("[{}]", elems.join(", ")))
        }
        _ => ContentNode::plain(format!("{}", value)),
    }
}

/// Render a single TypedObject as ContentNode::KeyValue using schema field names.
fn render_typed_object_as_content(
    schema_id: u64,
    slots: &[shape_value::slot::ValueSlot],
    heap_mask: u64,
) -> ContentNode {
    let sid = schema_id as SchemaId;
    if let Some(schema) = lookup_schema_by_id_public(sid) {
        let mut pairs = Vec::with_capacity(schema.fields.len());
        for (i, field_def) in schema.fields.iter().enumerate() {
            if i < slots.len() {
                let val = extract_slot_value(&slots[i], heap_mask, i, &field_def.field_type);
                let value_node = render_as_content(&val);
                pairs.push((field_def.name.clone(), value_node));
            }
        }
        ContentNode::KeyValue(pairs)
    } else {
        // Schema not found — fall back to Display
        ContentNode::plain(format!("TypedObject(schema={})", schema_id))
    }
}

/// Extract a ValueWord value from a ValueSlot using the schema field type.
fn extract_slot_value(
    slot: &shape_value::slot::ValueSlot,
    heap_mask: u64,
    index: usize,
    field_type: &crate::type_schema::FieldType,
) -> ValueWord {
    use crate::type_schema::FieldType;
    if heap_mask & (1u64 << index) != 0 {
        slot.as_heap_nb()
    } else {
        match field_type {
            FieldType::I64 => ValueWord::from_i64(slot.as_f64() as i64),
            FieldType::Bool => ValueWord::from_bool(slot.as_bool()),
            FieldType::Decimal => ValueWord::from_decimal(
                rust_decimal::Decimal::from_f64_retain(slot.as_f64()).unwrap_or_default(),
            ),
            _ => ValueWord::from_f64(slot.as_f64()),
        }
    }
}

/// Render an array as a ContentNode.
///
/// For arrays of typed objects, renders as a table with columns from the schema.
/// For scalar arrays, renders as "[1, 2, 3]".
fn render_array_as_content(arr: &[ValueWord]) -> ContentNode {
    if arr.is_empty() {
        return ContentNode::plain("[]".to_string());
    }

    // Check first element to determine rendering strategy
    if let Some(HeapValue::TypedObject { .. }) = arr.first().and_then(|v| v.as_heap_ref()) {
        return render_typed_array_as_table(arr);
    }

    // Scalar array → "[1, 2, 3]"
    let items: Vec<String> = arr.iter().map(|v| format!("{}", v)).collect();
    ContentNode::plain(format!("[{}]", items.join(", ")))
}

/// Render an array of typed objects as a ContentNode::Table.
///
/// Extracts headers from the first element's schema and renders each row's
/// field values as Content-dispatched cells. Falls back to single-column
/// display if the schema cannot be resolved.
fn render_typed_array_as_table(arr: &[ValueWord]) -> ContentNode {
    // Get schema from first element
    if let Some((schema_id, _, _)) = arr.first().and_then(|v| v.as_typed_object()) {
        let sid = schema_id as SchemaId;
        if let Some(schema) = lookup_schema_by_id_public(sid) {
            let headers: Vec<String> = schema.fields.iter().map(|f| f.name.clone()).collect();

            let mut rows: Vec<Vec<ContentNode>> = Vec::with_capacity(arr.len());
            for elem in arr {
                if let Some((_eid, slots, heap_mask)) = elem.as_typed_object() {
                    let mut row_cells: Vec<ContentNode> = Vec::with_capacity(schema.fields.len());
                    for (i, field_def) in schema.fields.iter().enumerate() {
                        if i < slots.len() {
                            let val =
                                extract_slot_value(&slots[i], heap_mask, i, &field_def.field_type);
                            row_cells.push(render_as_content(&val));
                        } else {
                            row_cells.push(ContentNode::plain("".to_string()));
                        }
                    }
                    rows.push(row_cells);
                } else {
                    // Non-TypedObject element in the array — single cell fallback
                    let mut cells = vec![ContentNode::plain(format!("{}", elem))];
                    cells.resize(headers.len(), ContentNode::plain("".to_string()));
                    rows.push(cells);
                }
            }

            return ContentNode::Table(ContentTable {
                headers,
                rows,
                border: BorderStyle::default(),
                max_rows: None,
                column_types: None,
                total_rows: None,
                sortable: false,
            });
        }
    }

    // Fallback: single-column display
    let mut rows: Vec<Vec<ContentNode>> = Vec::with_capacity(arr.len());
    for elem in arr {
        rows.push(vec![ContentNode::plain(format!("{}", elem))]);
    }

    ContentNode::Table(ContentTable {
        headers: vec!["value".to_string()],
        rows,
        border: BorderStyle::default(),
        max_rows: None,
        column_types: None,
        total_rows: None,
        sortable: false,
    })
}

/// Convert a DataTable (Arrow RecordBatch wrapper) to a ContentNode::Table.
///
/// Extracts column names as headers, determines column types from the Arrow schema,
/// and converts each cell to a plain text ContentNode. Optionally limits the number
/// of rows displayed.
pub fn datatable_to_content_node(dt: &DataTable, max_rows: Option<usize>) -> ContentNode {
    use arrow_array::Array;

    let headers = dt.column_names();
    let total = dt.row_count();
    let limit = max_rows.unwrap_or(total).min(total);

    // Determine column types from Arrow schema
    let schema = dt.inner().schema();
    let column_types: Vec<String> = schema
        .fields()
        .iter()
        .map(|f| arrow_type_label(f.data_type()))
        .collect();

    // Build rows
    let batch = dt.inner();
    let mut rows = Vec::with_capacity(limit);
    for row_idx in 0..limit {
        let mut cells = Vec::with_capacity(headers.len());
        for col_idx in 0..headers.len() {
            let col = batch.column(col_idx);
            let text = if col.is_null(row_idx) {
                "null".to_string()
            } else {
                arrow_cell_display(col.as_ref(), row_idx)
            };
            cells.push(ContentNode::plain(text));
        }
        rows.push(cells);
    }

    ContentNode::Table(ContentTable {
        headers,
        rows,
        border: BorderStyle::default(),
        max_rows: None, // already truncated above
        column_types: Some(column_types),
        total_rows: if total > limit { Some(total) } else { None },
        sortable: true,
    })
}

/// Map Arrow DataType to a human-readable type label.
fn arrow_type_label(dt: &arrow_schema::DataType) -> String {
    use arrow_schema::DataType;
    match dt {
        DataType::Float16 | DataType::Float32 | DataType::Float64 => "number".to_string(),
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
            "number".to_string()
        }
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            "number".to_string()
        }
        DataType::Boolean => "boolean".to_string(),
        DataType::Utf8 | DataType::LargeUtf8 => "string".to_string(),
        DataType::Date32 | DataType::Date64 => "date".to_string(),
        DataType::Timestamp(_, _) => "date".to_string(),
        DataType::Duration(_) => "duration".to_string(),
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => "number".to_string(),
        _ => "string".to_string(),
    }
}

/// Display a single Arrow cell value as a string.
fn arrow_cell_display(array: &dyn arrow_array::Array, index: usize) -> String {
    use arrow_array::cast::AsArray;
    use arrow_array::types::*;
    use arrow_schema::DataType;

    match array.data_type() {
        DataType::Float64 => format!("{}", array.as_primitive::<Float64Type>().value(index)),
        DataType::Float32 => format!("{}", array.as_primitive::<Float32Type>().value(index)),
        DataType::Int64 => format!("{}", array.as_primitive::<Int64Type>().value(index)),
        DataType::Int32 => format!("{}", array.as_primitive::<Int32Type>().value(index)),
        DataType::Int16 => format!("{}", array.as_primitive::<Int16Type>().value(index)),
        DataType::Int8 => format!("{}", array.as_primitive::<Int8Type>().value(index)),
        DataType::UInt64 => format!("{}", array.as_primitive::<UInt64Type>().value(index)),
        DataType::UInt32 => format!("{}", array.as_primitive::<UInt32Type>().value(index)),
        DataType::UInt16 => format!("{}", array.as_primitive::<UInt16Type>().value(index)),
        DataType::UInt8 => format!("{}", array.as_primitive::<UInt8Type>().value(index)),
        DataType::Boolean => format!("{}", array.as_boolean().value(index)),
        DataType::Utf8 => array.as_string::<i32>().value(index).to_string(),
        DataType::LargeUtf8 => array.as_string::<i64>().value(index).to_string(),
        DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, _) => {
            let ts = array
                .as_primitive::<TimestampMicrosecondType>()
                .value(index);
            match chrono::DateTime::from_timestamp_micros(ts) {
                Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
                None => ts.to_string(),
            }
        }
        DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, _) => {
            let ts = array
                .as_primitive::<TimestampMillisecondType>()
                .value(index);
            match chrono::DateTime::from_timestamp_millis(ts) {
                Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
                None => ts.to_string(),
            }
        }
        _ => format!("{}", index),
    }
}

/// Render a HashMap as ContentNode::KeyValue pairs.
fn render_hashmap_as_content(keys: &[ValueWord], values: &[ValueWord]) -> ContentNode {
    let mut pairs = Vec::with_capacity(keys.len());
    for (k, v) in keys.iter().zip(values.iter()) {
        let key_str = if let Some(s) = k.as_str() {
            s.to_string()
        } else {
            format!("{}", k)
        };
        let value_node = render_as_content(v);
        pairs.push((key_str, value_node));
    }
    ContentNode::KeyValue(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::ContentNode;
    use std::sync::Arc;

    #[test]
    fn test_render_string_as_plain_text() {
        let val = ValueWord::from_string(Arc::new("hello".to_string()));
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("hello"));
    }

    #[test]
    fn test_render_integer_as_plain_text() {
        let val = ValueWord::from_i64(42);
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("42"));
    }

    #[test]
    fn test_render_float_as_plain_text() {
        let val = ValueWord::from_f64(3.14);
        let node = render_as_content(&val);
        let text = node.to_string();
        assert!(text.contains("3.14"), "expected 3.14, got: {}", text);
    }

    #[test]
    fn test_render_bool_true() {
        let val = ValueWord::from_bool(true);
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("true"));
    }

    #[test]
    fn test_render_bool_false() {
        let val = ValueWord::from_bool(false);
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("false"));
    }

    #[test]
    fn test_render_none() {
        let val = ValueWord::none();
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("none"));
    }

    #[test]
    fn test_render_content_node_passthrough() {
        let original = ContentNode::plain("already content");
        let val = ValueWord::from_content(original.clone());
        let node = render_as_content(&val);
        assert_eq!(node, original);
    }

    #[test]
    fn test_render_scalar_array() {
        let arr = Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ]);
        let val = ValueWord::from_array(arr);
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("[1, 2, 3]"));
    }

    #[test]
    fn test_render_empty_array() {
        let arr = Arc::new(vec![]);
        let val = ValueWord::from_array(arr);
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("[]"));
    }

    #[test]
    fn test_render_hashmap_as_key_value() {
        let keys = vec![ValueWord::from_string(Arc::new("name".to_string()))];
        let values = vec![ValueWord::from_string(Arc::new("Alice".to_string()))];
        let val = ValueWord::from_hashmap_pairs(keys, values);
        let node = render_as_content(&val);
        match &node {
            ContentNode::KeyValue(pairs) => {
                assert_eq!(pairs.len(), 1);
                assert_eq!(pairs[0].0, "name");
                assert_eq!(pairs[0].1, ContentNode::plain("Alice"));
            }
            _ => panic!("expected KeyValue, got: {:?}", node),
        }
    }

    #[test]
    fn test_render_decimal_as_plain_text() {
        use rust_decimal::Decimal;
        let val = ValueWord::from_decimal(Decimal::new(1234, 2)); // 12.34
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("12.34"));
    }

    #[test]
    fn test_render_unit() {
        let val = ValueWord::unit();
        let node = render_as_content(&val);
        assert_eq!(node, ContentNode::plain("()"));
    }

    #[test]
    fn test_typed_object_renders_as_key_value() {
        use crate::type_schema::typed_object_from_pairs;

        let obj = typed_object_from_pairs(&[
            (
                "name",
                ValueWord::from_string(Arc::new("Alice".to_string())),
            ),
            ("age", ValueWord::from_i64(30)),
        ]);
        let node = render_as_content(&obj);
        match &node {
            ContentNode::KeyValue(pairs) => {
                assert_eq!(pairs.len(), 2);
                // Field names come from schema
                let names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
                assert!(
                    names.contains(&"name"),
                    "expected 'name' field, got: {:?}",
                    names
                );
                assert!(
                    names.contains(&"age"),
                    "expected 'age' field, got: {:?}",
                    names
                );
            }
            _ => panic!("expected KeyValue for TypedObject, got: {:?}", node),
        }
    }

    #[test]
    fn test_typed_array_renders_as_table_with_headers() {
        use crate::type_schema::typed_object_from_pairs;

        let row1 = typed_object_from_pairs(&[
            ("x", ValueWord::from_i64(1)),
            ("y", ValueWord::from_i64(2)),
        ]);
        let row2 = typed_object_from_pairs(&[
            ("x", ValueWord::from_i64(3)),
            ("y", ValueWord::from_i64(4)),
        ]);
        let arr = Arc::new(vec![row1, row2]);
        let val = ValueWord::from_array(arr);
        let node = render_as_content(&val);
        match &node {
            ContentNode::Table(table) => {
                assert_eq!(table.headers.len(), 2);
                assert!(
                    table.headers.contains(&"x".to_string()),
                    "expected 'x' header"
                );
                assert!(
                    table.headers.contains(&"y".to_string()),
                    "expected 'y' header"
                );
                assert_eq!(table.rows.len(), 2);
                // Each row should have 2 cells
                assert_eq!(table.rows[0].len(), 2);
                assert_eq!(table.rows[1].len(), 2);
            }
            _ => panic!("expected Table for Vec<TypedObject>, got: {:?}", node),
        }
    }

    #[test]
    fn test_adapter_capabilities() {
        let terminal = capabilities_for_adapter(adapters::TERMINAL);
        assert!(terminal.ansi);
        assert!(terminal.color);
        assert!(terminal.unicode);

        let plain = capabilities_for_adapter(adapters::PLAIN);
        assert!(!plain.ansi);
        assert!(!plain.color);

        let html = capabilities_for_adapter(adapters::HTML);
        assert!(!html.ansi);
        assert!(html.color);
        assert!(html.interactive);

        let json = capabilities_for_adapter(adapters::JSON);
        assert!(!json.ansi);
        assert!(!json.color);
        assert!(json.unicode);
    }

    #[test]
    fn test_render_as_content_for_falls_through() {
        let val = ValueWord::from_i64(42);
        let caps = capabilities_for_adapter(adapters::TERMINAL);
        let node = render_as_content_for(&val, adapters::TERMINAL, &caps);
        assert_eq!(node, ContentNode::plain("42"));
    }

    #[test]
    fn test_datatable_to_content_node() {
        use arrow_schema::{DataType, Field};
        use shape_value::DataTableBuilder;

        let mut builder = DataTableBuilder::with_fields(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]);
        builder.add_string_column(vec!["alpha", "beta", "gamma"]);
        builder.add_f64_column(vec![1.0, 2.0, 3.0]);
        let dt = builder.finish().expect("should build DataTable");

        let node = datatable_to_content_node(&dt, None);
        match &node {
            ContentNode::Table(table) => {
                assert_eq!(table.headers, vec!["name", "value"]);
                assert_eq!(table.rows.len(), 3);
                assert_eq!(table.rows[0][0], ContentNode::plain("alpha"));
                assert_eq!(table.rows[0][1], ContentNode::plain("1"));
                assert!(table.column_types.is_some());
                let types = table.column_types.as_ref().unwrap();
                assert_eq!(types[0], "string");
                assert_eq!(types[1], "number");
                assert!(table.sortable);
            }
            _ => panic!("expected Table, got: {:?}", node),
        }
    }

    #[test]
    fn test_datatable_to_content_node_with_max_rows() {
        use arrow_schema::{DataType, Field};
        use shape_value::DataTableBuilder;

        let mut builder =
            DataTableBuilder::with_fields(vec![Field::new("x", DataType::Int64, false)]);
        builder.add_i64_column(vec![10, 20, 30, 40, 50]);
        let dt = builder.finish().expect("should build DataTable");

        let node = datatable_to_content_node(&dt, Some(2));
        match &node {
            ContentNode::Table(table) => {
                assert_eq!(table.rows.len(), 2);
                assert_eq!(table.total_rows, Some(5));
            }
            _ => panic!("expected Table, got: {:?}", node),
        }
    }

    #[test]
    fn test_datatable_renders_via_content_dispatch() {
        use arrow_schema::{DataType, Field};
        use shape_value::DataTableBuilder;

        let mut builder =
            DataTableBuilder::with_fields(vec![Field::new("col", DataType::Utf8, false)]);
        builder.add_string_column(vec!["hello"]);
        let dt = builder.finish().expect("should build DataTable");

        let val = ValueWord::from_datatable(Arc::new(dt));
        let node = render_as_content(&val);
        match &node {
            ContentNode::Table(table) => {
                assert_eq!(table.headers, vec!["col"]);
                assert_eq!(table.rows.len(), 1);
            }
            _ => panic!("expected Table for DataTable, got: {:?}", node),
        }
    }
}
