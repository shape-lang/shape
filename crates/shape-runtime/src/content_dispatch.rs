//! Content trait dispatch — typed-handle subset.
//!
//! Phase 2b minimization: the broad ValueWord-typed `render_as_content` /
//! `render_as_content_for` dispatch chain (~600 LoC) was deleted along
//! with the polymorphic-tag-decode helpers it depended on (tag_bits,
//! ValueBits, ValueWordDisplay). The kind-threaded replacement lands
//! when shape-vm's content rendering is rebuilt on top of `slot_to_wire`
//! / `slot_extract_content` (the wire_conversion entry points).
//!
//! What remains: typed-handle helpers used by `wire_conversion` and the
//! REPL output path — `datatable_to_content_node`, `arrow_type_label`,
//! `arrow_cell_display`, plus the user-resolver hooks (kept as
//! placeholders since they're write-only at the moment — the readers
//! were in the deleted dispatch chain).

use crate::content_renderer::RendererCapabilities;
use shape_value::DataTable;
use shape_value::content::{BorderStyle, ContentNode, ContentTable};

/// Well-known adapter names for ContentFor<Adapter> dispatch.
pub mod adapters {
    pub const TERMINAL: &str = "Terminal";
    pub const HTML: &str = "Html";
    pub const MARKDOWN: &str = "Markdown";
    pub const JSON: &str = "Json";
    pub const PLAIN: &str = "Plain";
}

pub fn capabilities_for_adapter(adapter: &str) -> RendererCapabilities {
    match adapter {
        adapters::TERMINAL => RendererCapabilities::terminal(),
        adapters::HTML => RendererCapabilities::html(),
        adapters::MARKDOWN => RendererCapabilities::markdown(),
        adapters::JSON => RendererCapabilities::json(),
        _ => RendererCapabilities::plain(),
    }
}

/// Render a `DataTable` as a structured `ContentNode::Table`. Used by
/// `wire_conversion::slot_extract_content` and the REPL output path.
pub fn datatable_to_content_node(dt: &DataTable, max_rows: Option<usize>) -> ContentNode {
    use arrow_array::Array;

    let headers = dt.column_names();
    let total = dt.row_count();
    let limit = max_rows.unwrap_or(total).min(total);

    let schema = dt.inner().schema();
    let column_types: Vec<String> = schema
        .fields()
        .iter()
        .map(|f| arrow_type_label(f.data_type()))
        .collect();

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
        max_rows: None,
        column_types: Some(column_types),
        total_rows: if total > limit { Some(total) } else { None },
        sortable: true,
    })
}

fn arrow_type_label(dt: &arrow_schema::DataType) -> String {
    use arrow_schema::DataType;
    match dt {
        DataType::Int8 => "i8".to_string(),
        DataType::Int16 => "i16".to_string(),
        DataType::Int32 => "i32".to_string(),
        DataType::Int64 => "int".to_string(),
        DataType::UInt8 => "u8".to_string(),
        DataType::UInt16 => "u16".to_string(),
        DataType::UInt32 => "u32".to_string(),
        DataType::UInt64 => "u64".to_string(),
        DataType::Float32 => "f32".to_string(),
        DataType::Float64 => "number".to_string(),
        DataType::Boolean => "bool".to_string(),
        DataType::Utf8 | DataType::LargeUtf8 => "string".to_string(),
        DataType::Date32 | DataType::Date64 => "date".to_string(),
        DataType::Timestamp(_, _) => "timestamp".to_string(),
        other => format!("{:?}", other),
    }
}

fn arrow_cell_display(array: &dyn arrow_array::Array, index: usize) -> String {
    use arrow_array::cast::AsArray;
    use arrow_schema::DataType;

    match array.data_type() {
        DataType::Int8 => array.as_primitive::<arrow_array::types::Int8Type>().value(index).to_string(),
        DataType::Int16 => array.as_primitive::<arrow_array::types::Int16Type>().value(index).to_string(),
        DataType::Int32 => array.as_primitive::<arrow_array::types::Int32Type>().value(index).to_string(),
        DataType::Int64 => array.as_primitive::<arrow_array::types::Int64Type>().value(index).to_string(),
        DataType::UInt8 => array.as_primitive::<arrow_array::types::UInt8Type>().value(index).to_string(),
        DataType::UInt16 => array.as_primitive::<arrow_array::types::UInt16Type>().value(index).to_string(),
        DataType::UInt32 => array.as_primitive::<arrow_array::types::UInt32Type>().value(index).to_string(),
        DataType::UInt64 => array.as_primitive::<arrow_array::types::UInt64Type>().value(index).to_string(),
        DataType::Float32 => array.as_primitive::<arrow_array::types::Float32Type>().value(index).to_string(),
        DataType::Float64 => array.as_primitive::<arrow_array::types::Float64Type>().value(index).to_string(),
        DataType::Boolean => array.as_boolean().value(index).to_string(),
        DataType::Utf8 => array.as_string::<i32>().value(index).to_string(),
        DataType::LargeUtf8 => array.as_string::<i64>().value(index).to_string(),
        _ => format!("{:?}", array.slice(index, 1)),
    }
}
