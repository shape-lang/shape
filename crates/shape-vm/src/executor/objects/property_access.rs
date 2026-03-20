//! Property access operations (GetProp, SetProp, Length)
//!
//! Handles property access for all VM types including optimized O(1) PHF lookup
//! for series properties and direct byte offset access for typed objects.

use crate::bytecode::{Instruction, Operand};
use crate::executor::VirtualMachine;
use crate::memory::{record_heap_write, write_barrier_vw};
use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use shape_value::NanTag;
use shape_value::{HeapValue, VMError, ValueWord, heap_value::NativeScalar};
use std::sync::Arc;

/// PHF map for Time property access — replaces sequential string comparisons.
static TIME_PROPERTIES: phf::Map<&'static str, fn(&DateTime<FixedOffset>) -> ValueWord> = phf::phf_map! {
    "year" => |dt| ValueWord::from_f64(dt.year() as f64),
    "month" => |dt| ValueWord::from_f64(dt.month() as f64),
    "day" => |dt| ValueWord::from_f64(dt.day() as f64),
    "hour" => |dt| ValueWord::from_f64(dt.hour() as f64),
    "minute" => |dt| ValueWord::from_f64(dt.minute() as f64),
    "second" => |dt| ValueWord::from_f64(dt.second() as f64),
    "weekday" => |dt| ValueWord::from_f64(dt.weekday().num_days_from_monday() as f64),
    "timestamp" => |dt| ValueWord::from_f64(dt.timestamp() as f64),
    "timestamp_millis" => |dt| ValueWord::from_f64(dt.timestamp_millis() as f64),
};

fn read_native_u64(value: &ValueWord) -> Option<u64> {
    if let Some(scalar) = value.as_native_scalar() {
        return match scalar {
            NativeScalar::U8(v) => Some(v as u64),
            NativeScalar::U16(v) => Some(v as u64),
            NativeScalar::U32(v) => Some(v as u64),
            NativeScalar::U64(v) => Some(v),
            NativeScalar::Usize(v) => Some(v as u64),
            NativeScalar::Ptr(v) => Some(v as u64),
            NativeScalar::I8(v) if v >= 0 => Some(v as u64),
            NativeScalar::I16(v) if v >= 0 => Some(v as u64),
            NativeScalar::I32(v) if v >= 0 => Some(v as u64),
            NativeScalar::I64(v) if v >= 0 => Some(v as u64),
            NativeScalar::Isize(v) if v >= 0 => Some(v as u64),
            _ => None,
        };
    }

    if let Some(i) = value.as_i64()
        && i >= 0
    {
        return Some(i as u64);
    }
    if let Some(b) = value.as_bool() {
        return Some(if b { 1 } else { 0 });
    }
    None
}

fn read_native_f64(value: &ValueWord) -> Option<f64> {
    if let Some(n) = value.as_number_strict() {
        return Some(n);
    }
    if matches!(value.tag(), NanTag::I48) {
        return value.as_i64().map(|v| v as f64);
    }
    None
}

fn read_native_view_field(
    view: &shape_value::heap_value::NativeViewData,
    field: &shape_value::heap_value::NativeLayoutField,
) -> Result<ValueWord, VMError> {
    if view.ptr == 0 {
        return Err(VMError::RuntimeError(format!(
            "Cannot read field '{}' on null native view",
            field.name
        )));
    }

    let addr = unsafe { (view.ptr as *const u8).add(field.offset as usize) };
    match field.c_type.as_str() {
        "i8" => Ok(ValueWord::from_native_i8(unsafe { *(addr as *const i8) })),
        "u8" => Ok(ValueWord::from_native_u8(unsafe { *(addr as *const u8) })),
        "i16" => Ok(ValueWord::from_native_i16(unsafe { *(addr as *const i16) })),
        "u16" => Ok(ValueWord::from_native_u16(unsafe { *(addr as *const u16) })),
        "i32" => Ok(ValueWord::from_native_i32(unsafe { *(addr as *const i32) })),
        "u32" => Ok(ValueWord::from_native_u32(unsafe { *(addr as *const u32) })),
        "i64" => Ok(ValueWord::from_native_scalar(NativeScalar::I64(unsafe {
            *(addr as *const i64)
        }))),
        "u64" => Ok(ValueWord::from_native_u64(unsafe { *(addr as *const u64) })),
        "isize" => Ok(ValueWord::from_native_isize(unsafe {
            *(addr as *const isize)
        })),
        "usize" => Ok(ValueWord::from_native_usize(unsafe {
            *(addr as *const usize)
        })),
        "ptr" => Ok(ValueWord::from_native_ptr(unsafe {
            *(addr as *const usize)
        })),
        "f32" => Ok(ValueWord::from_native_f32(unsafe { *(addr as *const f32) })),
        "f64" => Ok(ValueWord::from_f64(unsafe { *(addr as *const f64) })),
        "bool" => Ok(ValueWord::from_bool(unsafe { *(addr as *const u8) } != 0)),
        "cstring" => {
            let ptr = unsafe { *(addr as *const *const std::ffi::c_char) };
            if ptr.is_null() {
                return Err(VMError::RuntimeError(format!(
                    "Field '{}.{}' is null but declared as non-null cstring",
                    view.layout.name, field.name
                )));
            }
            let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
                .to_string_lossy()
                .to_string();
            Ok(ValueWord::from_string(Arc::new(s)))
        }
        "cstring?" => {
            let ptr = unsafe { *(addr as *const *const std::ffi::c_char) };
            if ptr.is_null() {
                Ok(ValueWord::none())
            } else {
                let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .to_string();
                Ok(ValueWord::from_some(ValueWord::from_string(Arc::new(s))))
            }
        }
        _ => Ok(ValueWord::from_native_ptr(addr as usize)),
    }
}

fn write_native_view_field(
    view: &mut shape_value::heap_value::NativeViewData,
    field: &shape_value::heap_value::NativeLayoutField,
    value: &ValueWord,
) -> Result<(), VMError> {
    if !view.mutable {
        return Err(VMError::RuntimeError(format!(
            "Cannot assign to read-only cview<{}> field '{}'",
            view.layout.name, field.name
        )));
    }
    if view.ptr == 0 {
        return Err(VMError::RuntimeError(format!(
            "Cannot assign field '{}' on null native view",
            field.name
        )));
    }

    let addr = unsafe { (view.ptr as *mut u8).add(field.offset as usize) };
    match field.c_type.as_str() {
        "i8" => {
            let v = value.as_i64().ok_or_else(|| VMError::TypeError {
                expected: "i8",
                got: value.type_name(),
            })?;
            if !(i8::MIN as i64..=i8::MAX as i64).contains(&v) {
                return Err(VMError::RuntimeError(format!(
                    "Value {} out of range for i8 field '{}'",
                    v, field.name
                )));
            }
            unsafe { *(addr as *mut i8) = v as i8 };
        }
        "u8" => {
            let v = read_native_u64(value).ok_or_else(|| VMError::TypeError {
                expected: "u8",
                got: value.type_name(),
            })?;
            if v > u8::MAX as u64 {
                return Err(VMError::RuntimeError(format!(
                    "Value {} out of range for u8 field '{}'",
                    v, field.name
                )));
            }
            unsafe { *(addr as *mut u8) = v as u8 };
        }
        "i16" => {
            let v = value.as_i64().ok_or_else(|| VMError::TypeError {
                expected: "i16",
                got: value.type_name(),
            })?;
            if !(i16::MIN as i64..=i16::MAX as i64).contains(&v) {
                return Err(VMError::RuntimeError(format!(
                    "Value {} out of range for i16 field '{}'",
                    v, field.name
                )));
            }
            unsafe { *(addr as *mut i16) = v as i16 };
        }
        "u16" => {
            let v = read_native_u64(value).ok_or_else(|| VMError::TypeError {
                expected: "u16",
                got: value.type_name(),
            })?;
            if v > u16::MAX as u64 {
                return Err(VMError::RuntimeError(format!(
                    "Value {} out of range for u16 field '{}'",
                    v, field.name
                )));
            }
            unsafe { *(addr as *mut u16) = v as u16 };
        }
        "i32" => {
            let v = value.as_i64().ok_or_else(|| VMError::TypeError {
                expected: "i32",
                got: value.type_name(),
            })?;
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                return Err(VMError::RuntimeError(format!(
                    "Value {} out of range for i32 field '{}'",
                    v, field.name
                )));
            }
            unsafe { *(addr as *mut i32) = v as i32 };
        }
        "u32" => {
            let v = read_native_u64(value).ok_or_else(|| VMError::TypeError {
                expected: "u32",
                got: value.type_name(),
            })?;
            if v > u32::MAX as u64 {
                return Err(VMError::RuntimeError(format!(
                    "Value {} out of range for u32 field '{}'",
                    v, field.name
                )));
            }
            unsafe { *(addr as *mut u32) = v as u32 };
        }
        "i64" => {
            let v = value.as_i64().ok_or_else(|| VMError::TypeError {
                expected: "i64",
                got: value.type_name(),
            })?;
            unsafe { *(addr as *mut i64) = v };
        }
        "u64" => {
            let v = read_native_u64(value).ok_or_else(|| VMError::TypeError {
                expected: "u64",
                got: value.type_name(),
            })?;
            unsafe { *(addr as *mut u64) = v };
        }
        "isize" => {
            let v = value.as_i64().ok_or_else(|| VMError::TypeError {
                expected: "isize",
                got: value.type_name(),
            })?;
            if v < isize::MIN as i64 || v > isize::MAX as i64 {
                return Err(VMError::RuntimeError(format!(
                    "Value {} out of range for isize field '{}'",
                    v, field.name
                )));
            }
            unsafe { *(addr as *mut isize) = v as isize };
        }
        "usize" | "ptr" => {
            let v = read_native_u64(value).ok_or_else(|| VMError::TypeError {
                expected: "usize",
                got: value.type_name(),
            })?;
            let ptr_val = usize::try_from(v).map_err(|_| {
                VMError::RuntimeError(format!(
                    "Value {} does not fit in usize for field '{}'",
                    v, field.name
                ))
            })?;
            unsafe { *(addr as *mut usize) = ptr_val };
        }
        "f32" => {
            let v = read_native_f64(value).ok_or_else(|| VMError::TypeError {
                expected: "f32",
                got: value.type_name(),
            })?;
            unsafe { *(addr as *mut f32) = v as f32 };
        }
        "f64" => {
            let v = read_native_f64(value).ok_or_else(|| VMError::TypeError {
                expected: "f64",
                got: value.type_name(),
            })?;
            unsafe { *(addr as *mut f64) = v };
        }
        "bool" => {
            let b = value
                .as_bool()
                .unwrap_or_else(|| value.as_i64().is_some_and(|i| i != 0));
            unsafe { *(addr as *mut u8) = if b { 1 } else { 0 } };
        }
        "cstring" => {
            let ptr_val = read_native_u64(value).ok_or_else(|| VMError::TypeError {
                expected: "pointer",
                got: value.type_name(),
            })?;
            if ptr_val == 0 {
                return Err(VMError::RuntimeError(format!(
                    "Field '{}.{}' is non-null cstring and cannot be set to null",
                    view.layout.name, field.name
                )));
            }
            unsafe { *(addr as *mut *const std::ffi::c_char) = ptr_val as *const std::ffi::c_char };
        }
        "cstring?" => {
            if value.is_none() {
                unsafe { *(addr as *mut *const std::ffi::c_char) = std::ptr::null() };
            } else {
                let ptr_val = read_native_u64(value).ok_or_else(|| VMError::TypeError {
                    expected: "pointer or none",
                    got: value.type_name(),
                })?;
                unsafe {
                    *(addr as *mut *const std::ffi::c_char) = ptr_val as *const std::ffi::c_char
                };
            }
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "Unsupported writable native field type '{}' on '{}.{}'",
                other, view.layout.name, field.name
            )));
        }
    }
    Ok(())
}

impl VirtualMachine {
    pub(in crate::executor) fn op_get_prop(
        &mut self,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let key_nb = self.pop_vw()?;
        let obj_nb = self.pop_vw()?;

        // Unwrap TypeAnnotatedValue once at the top so all dispatch sees the inner value.
        let obj_ref = match obj_nb.as_heap_ref() {
            Some(HeapValue::TypeAnnotatedValue { value, .. }) => value.as_ref(),
            _ => &obj_nb,
        };

        // Extract key string once (used by most paths).
        let key_str = key_nb.as_str();

        // Handle unified arrays (bit-47 tagged) before HeapValue dispatch.
        if shape_value::tags::is_unified_heap(obj_ref.raw_bits()) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(obj_ref.raw_bits()) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits(obj_ref.raw_bits())
                };
                if let Some(ks) = key_str {
                    if ks == "length" {
                        return self.push_vw(ValueWord::from_i64(arr.len() as i64));
                    }
                    return Err(VMError::UndefinedProperty(ks.to_string()));
                }
                let idx_opt = key_nb
                    .as_i64()
                    .or_else(|| key_nb.as_f64().map(|f| f as i64));
                if let Some(idx) = idx_opt {
                    let len = arr.len() as i64;
                    let actual = if idx < 0 { len + idx } else { idx };
                    if actual >= 0 && (actual as usize) < arr.len() {
                        let elem_bits = *arr.get(actual as usize).unwrap();
                        let elem = unsafe { ValueWord::clone_from_bits(elem_bits) };
                        return self.push_vw(elem);
                    } else {
                        return self.push_vw(ValueWord::none());
                    }
                }
            }
        }

        // Primary dispatch: check if obj is a heap value.
        if let Some(hv) = obj_ref.as_heap_ref() {
            match hv {
                // TypedObject: perform runtime schema lookup for field access
                HeapValue::TypedObject {
                    schema_id,
                    slots,
                    heap_mask,
                } => {
                    if let Some(ks) = key_str {
                        if let Some(schema) = self
                            .program
                            .type_schema_registry
                            .get_by_id(*schema_id as u32)
                        {
                            if let Some(field) = schema.get_field(ks) {
                                let field_index = field.index as usize;
                                if field_index < slots.len() {
                                    let is_heap = (*heap_mask & (1u64 << field_index)) != 0;
                                    let result = match &field.field_type {
                                        shape_runtime::type_schema::FieldType::I64
                                        | shape_runtime::type_schema::FieldType::Timestamp => {
                                            if is_heap {
                                                slots[field_index].as_heap_nb()
                                            } else {
                                                ValueWord::from_i64(slots[field_index].as_i64())
                                            }
                                        }
                                        shape_runtime::type_schema::FieldType::Bool => {
                                            if is_heap {
                                                slots[field_index].as_heap_nb()
                                            } else {
                                                ValueWord::from_bool(slots[field_index].as_bool())
                                            }
                                        }
                                        shape_runtime::type_schema::FieldType::F64
                                        | shape_runtime::type_schema::FieldType::Decimal => {
                                            if is_heap {
                                                slots[field_index].as_heap_nb()
                                            } else {
                                                ValueWord::from_f64(slots[field_index].as_f64())
                                            }
                                        }
                                        // Width integer types: stored via from_int(), read via as_i64()
                                        ft if ft.is_width_integer() => {
                                            if is_heap {
                                                slots[field_index].as_heap_nb()
                                            } else {
                                                let raw_bits = slots[field_index].as_i64() as u64;
                                                if matches!(
                                                    ft,
                                                    shape_runtime::type_schema::FieldType::U64
                                                ) && raw_bits > i64::MAX as u64
                                                {
                                                    ValueWord::from_native_u64(raw_bits)
                                                } else {
                                                    ValueWord::from_i64(slots[field_index].as_i64())
                                                }
                                            }
                                        }
                                        // Any and non-primitive types: use as_value_word to
                                        // preserve all inline NanTag variants (Function, etc.)
                                        _ => slots[field_index].as_value_word(is_heap),
                                    };
                                    return self.push_vw(result);
                                }
                            }
                        }
                        return self.push_vw(ValueWord::none());
                    }
                }

                // NativeView: pointer-backed zero-copy field access.
                HeapValue::NativeView(view) => {
                    if let Some(ks) = key_str {
                        let field = view
                            .layout
                            .field(ks)
                            .ok_or_else(|| VMError::UndefinedProperty(ks.to_string()))?;
                        let result = read_native_view_field(view, field)?;
                        return self.push_vw(result);
                    }
                }

                // Array: handle both string key (.length) and numeric index
                HeapValue::Array(arr) => {
                    if let Some(ks) = key_str {
                        if ks == "length" {
                            return self.push_vw(ValueWord::from_i64(arr.len() as i64));
                        }
                        return Err(VMError::UndefinedProperty(ks.to_string()));
                    }
                    // Numeric index
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let len = arr.len() as i64;
                        let actual = if idx < 0 { len + idx } else { idx };
                        if actual >= 0 && (actual as usize) < arr.len() {
                            return self.push_vw(arr[actual as usize].clone());
                        } else {
                            return self.push_vw(ValueWord::none());
                        }
                    }
                }

                // IntArray: typed array indexing
                HeapValue::IntArray(arr) => {
                    if let Some(ks) = key_str {
                        if ks == "length" {
                            return self.push_vw(ValueWord::from_i64(arr.len() as i64));
                        }
                        return Err(VMError::UndefinedProperty(ks.to_string()));
                    }
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let len = arr.len() as i64;
                        let actual = if idx < 0 { len + idx } else { idx };
                        if actual >= 0 && (actual as usize) < arr.len() {
                            return self.push_vw(ValueWord::from_i64(arr[actual as usize]));
                        } else {
                            return self.push_vw(ValueWord::none());
                        }
                    }
                }

                // FloatArray: typed array indexing
                HeapValue::FloatArray(arr) => {
                    if let Some(ks) = key_str {
                        if ks == "length" {
                            return self.push_vw(ValueWord::from_i64(arr.len() as i64));
                        }
                        return Err(VMError::UndefinedProperty(ks.to_string()));
                    }
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let len = arr.len() as i64;
                        let actual = if idx < 0 { len + idx } else { idx };
                        if actual >= 0 && (actual as usize) < arr.len() {
                            return self.push_vw(ValueWord::from_f64(arr[actual as usize]));
                        } else {
                            return self.push_vw(ValueWord::none());
                        }
                    }
                }

                // FloatArraySlice: zero-copy read-only view into matrix row
                HeapValue::FloatArraySlice { parent, offset, len } => {
                    let slice_len = *len as usize;
                    let off = *offset as usize;
                    if let Some(ks) = key_str {
                        if ks == "length" {
                            return self.push_vw(ValueWord::from_i64(slice_len as i64));
                        }
                        return Err(VMError::UndefinedProperty(ks.to_string()));
                    }
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let actual = if idx < 0 { slice_len as i64 + idx } else { idx };
                        if actual >= 0 && (actual as usize) < slice_len {
                            return self.push_vw(ValueWord::from_f64(parent.data[off + actual as usize]));
                        } else {
                            return self.push_vw(ValueWord::none());
                        }
                    }
                }

                // BoolArray: typed array indexing
                HeapValue::BoolArray(arr) => {
                    if let Some(ks) = key_str {
                        if ks == "length" {
                            return self.push_vw(ValueWord::from_i64(arr.len() as i64));
                        }
                        return Err(VMError::UndefinedProperty(ks.to_string()));
                    }
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let len = arr.len() as i64;
                        let actual = if idx < 0 { len + idx } else { idx };
                        if actual >= 0 && (actual as usize) < arr.len() {
                            return self.push_vw(ValueWord::from_bool(arr[actual as usize] != 0));
                        } else {
                            return self.push_vw(ValueWord::none());
                        }
                    }
                }

                // String: .length or char indexing
                HeapValue::String(s) => {
                    if let Some(ks) = key_str {
                        if ks == "length" {
                            return self.push_vw(ValueWord::from_i64(s.chars().count() as i64));
                        }
                        return Err(VMError::UndefinedProperty(ks.to_string()));
                    }
                    // Numeric char index
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let char_count = s.chars().count() as i64;
                        let actual = if idx < 0 { char_count + idx } else { idx };
                        if actual >= 0 && actual < char_count {
                            if let Some(c) = s.chars().nth(actual as usize) {
                                return self.push_vw(ValueWord::from_char(c));
                            }
                        }
                        return self.push_vw(ValueWord::none());
                    }
                }

                // Matrix: .rows, .cols, .length, or numeric index
                HeapValue::Matrix(mat) => {
                    if let Some(ks) = key_str {
                        match ks {
                            "rows" => return self.push_vw(ValueWord::from_i64(mat.rows as i64)),
                            "cols" => return self.push_vw(ValueWord::from_i64(mat.cols as i64)),
                            "length" => {
                                return self.push_vw(ValueWord::from_i64(mat.data.len() as i64));
                            }
                            _ => return Err(VMError::UndefinedProperty(ks.to_string())),
                        }
                    }
                    // Numeric index => extract row as zero-copy FloatArraySlice
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let rows = mat.rows as i64;
                        let cols = mat.cols;
                        let actual = if idx < 0 { rows + idx } else { idx };
                        if actual >= 0 && (actual as u32) < mat.rows {
                            let parent_arc = mat.clone();
                            let offset = actual as u32 * cols;
                            let len = cols;
                            return self.push_vw(ValueWord::from_heap_value(
                                HeapValue::FloatArraySlice { parent: parent_arc, offset, len },
                            ));
                        } else {
                            return self.push_vw(ValueWord::none());
                        }
                    }
                }

                // Time: PHF map dispatch
                HeapValue::Time(dt) => {
                    if let Some(ks) = key_str {
                        if let Some(accessor) = TIME_PROPERTIES.get(ks) {
                            return self.push_vw(accessor(dt));
                        }
                        return Err(VMError::UndefinedProperty(ks.to_string()));
                    }
                }

                // TypedTable → ColumnRef
                HeapValue::TypedTable { schema_id, table } => {
                    if let Some(ks) = key_str {
                        let col_id = table
                            .inner()
                            .schema()
                            .index_of(ks)
                            .map_err(|_| VMError::UndefinedProperty(ks.to_string()))?
                            as u32;
                        return self.push_vw(ValueWord::from_column_ref(
                            *schema_id,
                            table.clone(),
                            col_id,
                        ));
                    }
                }

                // DataTable → ColumnRef
                HeapValue::DataTable(table) => {
                    if let Some(ks) = key_str {
                        let col_id = table
                            .inner()
                            .schema()
                            .index_of(ks)
                            .map_err(|_| VMError::UndefinedProperty(ks.to_string()))?
                            as u32;
                        return self.push_vw(ValueWord::from_column_ref(0, table.clone(), col_id));
                    }
                }

                // IndexedTable → ColumnRef
                HeapValue::IndexedTable {
                    schema_id, table, ..
                } => {
                    if let Some(ks) = key_str {
                        let col_id = table
                            .inner()
                            .schema()
                            .index_of(ks)
                            .map_err(|_| VMError::UndefinedProperty(ks.to_string()))?
                            as u32;
                        return self.push_vw(ValueWord::from_column_ref(
                            *schema_id,
                            table.clone(),
                            col_id,
                        ));
                    }
                }

                // RowView → column value
                HeapValue::RowView { table, row_idx, .. } => {
                    if let Some(ks) = key_str {
                        let col_idx = table
                            .inner()
                            .schema()
                            .index_of(ks)
                            .map_err(|_| VMError::UndefinedProperty(ks.to_string()))?;
                        let column = table.inner().column(col_idx);
                        use arrow_array::Array;
                        let result = if column.is_null(*row_idx) {
                            ValueWord::none()
                        } else if let Some(arr) =
                            column.as_any().downcast_ref::<arrow_array::Float64Array>()
                        {
                            ValueWord::from_f64(arr.value(*row_idx))
                        } else if let Some(arr) =
                            column.as_any().downcast_ref::<arrow_array::Float32Array>()
                        {
                            ValueWord::from_f64(arr.value(*row_idx) as f64)
                        } else if let Some(arr) =
                            column.as_any().downcast_ref::<arrow_array::Int64Array>()
                        {
                            ValueWord::from_f64(arr.value(*row_idx) as f64)
                        } else if let Some(arr) =
                            column.as_any().downcast_ref::<arrow_array::Int32Array>()
                        {
                            ValueWord::from_f64(arr.value(*row_idx) as f64)
                        } else if let Some(arr) =
                            column.as_any().downcast_ref::<arrow_array::StringArray>()
                        {
                            ValueWord::from_string(Arc::new(arr.value(*row_idx).to_string()))
                        } else if let Some(arr) = column
                            .as_any()
                            .downcast_ref::<arrow_array::LargeStringArray>()
                        {
                            ValueWord::from_string(Arc::new(arr.value(*row_idx).to_string()))
                        } else if let Some(arr) =
                            column.as_any().downcast_ref::<arrow_array::BooleanArray>()
                        {
                            ValueWord::from_bool(arr.value(*row_idx))
                        } else {
                            ValueWord::none()
                        };
                        return self.push_vw(result);
                    }
                }

                // ExprProxy → nested ExprProxy
                HeapValue::ExprProxy(col) => {
                    if let Some(ks) = key_str {
                        return self
                            .push_vw(ValueWord::from_string(Arc::new(format!("{}.{}", col, ks))));
                    }
                }

                // HashMap: shape-guarded O(1) property access, with hash fallback
                HeapValue::HashMap(data) => {
                    if let Some(ks) = key_str {
                        // Fast path: shape-guarded O(1) lookup
                        if let Some(val) = data.shape_get(ks) {
                            // Record shape-based property access for JIT feedback.
                            // shape_id → schema_id, slot index → field_idx.
                            if let Some(sid) = data.shape_id {
                                let ic_ip = self.ip;
                                let prop_hash = shape_value::shape_graph::hash_property_name(ks);
                                if let Some(slot_idx) =
                                    shape_value::shape_graph::shape_property_index(sid, prop_hash)
                                {
                                    if let Some(fv) = self.current_feedback_vector() {
                                        fv.record_property(
                                            ic_ip,
                                            sid.0 as u64,
                                            slot_idx as u16,
                                            0,
                                            crate::feedback::RECEIVER_HASHMAP,
                                        );
                                    }
                                }
                            }
                            return self.push_vw(val.clone());
                        }
                        // Slow path: hash-based lookup
                        let key_vw = ValueWord::from_string(Arc::new(ks.to_string()));
                        let hash = key_vw.vw_hash();
                        if let Some(bucket) = data.index.get(&hash) {
                            if let Some(&idx) =
                                bucket.iter().find(|&&i| data.keys[i].vw_equals(&key_vw))
                            {
                                return self.push_vw(data.values[idx].clone());
                            }
                        }
                        return self.push_vw(ValueWord::none());
                    }
                    // Numeric key access
                    let idx_opt = key_nb
                        .as_i64()
                        .or_else(|| key_nb.as_f64().map(|f| f as i64));
                    if let Some(idx) = idx_opt {
                        let key_vw = ValueWord::from_i64(idx);
                        if let Some(found_idx) = data.find_key(&key_vw) {
                            return self.push_vw(data.values[found_idx].clone());
                        }
                        return self.push_vw(ValueWord::none());
                    }
                }

                _ => {} // fall through to error
            }
        }

        Err(VMError::RuntimeError(format!(
            "Cannot get property {} on {}",
            key_nb.type_name(),
            obj_ref.type_name()
        )))
    }

    fn parse_array_index(key_nb: &ValueWord) -> Result<i64, VMError> {
        key_nb
            .as_i64()
            .or_else(|| key_nb.as_f64().map(|f| f as i64))
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Cannot set property '{}' on array",
                    key_nb.type_name()
                ))
            })
    }

    pub(in crate::executor) fn set_array_index_on_object(
        object_nb: &mut ValueWord,
        key_nb: &ValueWord,
        value_nb: ValueWord,
    ) -> Result<(), VMError> {
        if let Some(key_str) = key_nb.as_str() {
            if let Some(HeapValue::NativeView(view)) = object_nb.as_heap_mut() {
                let field = view
                    .layout
                    .field(key_str)
                    .cloned()
                    .ok_or_else(|| VMError::UndefinedProperty(key_str.to_string()))?;
                // NativeView writes to raw C memory, not GC-tracked heap — no barrier needed.
                return write_native_view_field(view, &field, &value_nb);
            }
            if matches!(object_nb.as_heap_ref(), Some(HeapValue::TypedObject { .. })) {
                return Err(VMError::RuntimeError(format!(
                    "Compiler bug: generic SetProp used for typed object field '{}'. Expected SetFieldTyped.",
                    key_str
                )));
            }
        }

        // Handle unified arrays (bit-47 tagged) for index assignment.
        if shape_value::tags::is_unified_heap(object_nb.raw_bits()) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(object_nb.raw_bits()) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let idx = Self::parse_array_index(key_nb)?;
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits_mut(object_nb.raw_bits())
                };
                let len = arr.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 {
                    return Err(VMError::RuntimeError(format!(
                        "Array index {} is out of bounds",
                        idx
                    )));
                }
                let index = actual as usize;
                if index < arr.len() {
                    record_heap_write();
                    let old_bits = *arr.get(index).unwrap();
                    let old = unsafe { ValueWord::clone_from_bits(old_bits) };
                    write_barrier_vw(&old, &value_nb);
                    drop(old);
                    // Decrement old element refcount
                    if shape_value::tags::is_tagged(old_bits)
                        && shape_value::tags::get_tag(old_bits) == shape_value::tags::TAG_HEAP
                    {
                        let old_vw = unsafe { ValueWord::clone_from_bits(old_bits) };
                        drop(old_vw); // extra decrement
                    }
                    let new_bits = value_nb.raw_bits();
                    std::mem::forget(value_nb);
                    arr.set_boxed(index, new_bits);
                } else {
                    // Extend with none values
                    while arr.len() < index {
                        arr.push(ValueWord::none().raw_bits());
                    }
                    record_heap_write();
                    let new_bits = value_nb.raw_bits();
                    std::mem::forget(value_nb);
                    arr.push(new_bits);
                }
                return Ok(());
            }
        }

        if let Some(HeapValue::Array(arr)) = object_nb.as_heap_mut() {
            let idx = Self::parse_array_index(key_nb)?;
            let len = arr.len() as i64;
            let actual = if idx < 0 { len + idx } else { idx };
            if actual < 0 {
                return Err(VMError::RuntimeError(format!(
                    "Array index {} is out of bounds",
                    idx
                )));
            }
            let index = actual as usize;

            // Mutate in-place when unique, otherwise clone-on-write.
            let arr_mut = Arc::make_mut(arr);
            if index < arr_mut.len() {
                record_heap_write();
                write_barrier_vw(&arr_mut[index], &value_nb);
                arr_mut[index] = value_nb;
            } else {
                arr_mut.resize_with(index + 1, ValueWord::none);
                record_heap_write();
                write_barrier_vw(&arr_mut[index], &value_nb);
                arr_mut[index] = value_nb;
            }
            return Ok(());
        }

        // Typed array index assignment (IntArray, FloatArray, BoolArray)
        if let Some(heap) = object_nb.as_heap_mut() {
            match heap {
                HeapValue::IntArray(arr) => {
                    let idx = Self::parse_array_index(key_nb)?;
                    let len = arr.len() as i64;
                    let actual = if idx < 0 { len + idx } else { idx };
                    if actual < 0 || actual as usize >= arr.len() {
                        return Err(VMError::RuntimeError(format!(
                            "Array index {} is out of bounds for Vec<int> of length {}",
                            idx,
                            arr.len()
                        )));
                    }
                    let val = value_nb
                        .as_i64()
                        .or_else(|| value_nb.as_f64().map(|f| f as i64))
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Cannot assign {} to Vec<int>",
                                value_nb.type_name()
                            ))
                        })?;
                    let arr_mut = Arc::make_mut(arr);
                    arr_mut.data[actual as usize] = val;
                    return Ok(());
                }
                HeapValue::FloatArray(arr) => {
                    let idx = Self::parse_array_index(key_nb)?;
                    let len = arr.len() as i64;
                    let actual = if idx < 0 { len + idx } else { idx };
                    if actual < 0 || actual as usize >= arr.len() {
                        return Err(VMError::RuntimeError(format!(
                            "Array index {} is out of bounds for Vec<number> of length {}",
                            idx,
                            arr.len()
                        )));
                    }
                    let val = value_nb
                        .as_f64()
                        .or_else(|| value_nb.as_i64().map(|i| i as f64))
                        .ok_or_else(|| {
                            VMError::RuntimeError(format!(
                                "Cannot assign {} to Vec<number>",
                                value_nb.type_name()
                            ))
                        })?;
                    let arr_mut = Arc::make_mut(arr);
                    arr_mut.data.as_mut_slice()[actual as usize] = val;
                    return Ok(());
                }
                HeapValue::FloatArraySlice { .. } => {
                    return Err(VMError::RuntimeError(
                        "cannot mutate read-only row view".to_string(),
                    ));
                }
                HeapValue::BoolArray(arr) => {
                    let idx = Self::parse_array_index(key_nb)?;
                    let len = arr.len() as i64;
                    let actual = if idx < 0 { len + idx } else { idx };
                    if actual < 0 || actual as usize >= arr.len() {
                        return Err(VMError::RuntimeError(format!(
                            "Array index {} is out of bounds for Vec<bool> of length {}",
                            idx,
                            arr.len()
                        )));
                    }
                    let val = value_nb.as_bool().ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "Cannot assign {} to Vec<bool>",
                            value_nb.type_name()
                        ))
                    })?;
                    let arr_mut = Arc::make_mut(arr);
                    arr_mut.data[actual as usize] = if val { 1 } else { 0 };
                    return Ok(());
                }
                _ => {}
            }
        }

        Err(VMError::RuntimeError(format!(
            "Cannot set property '{}' on '{}'",
            key_nb.type_name(),
            object_nb.type_name()
        )))
    }

    pub(in crate::executor) fn op_set_prop(&mut self) -> Result<(), VMError> {
        let value_nb = self.pop_vw()?;
        let key_nb = self.pop_vw()?;
        let mut object_nb = self.pop_vw()?;

        Self::set_array_index_on_object(&mut object_nb, &key_nb, value_nb)?;
        self.push_vw(object_nb)
    }

    pub(in crate::executor) fn op_set_local_index(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let value_nb = self.pop_vw()?;
        let key_nb = self.pop_vw()?;
        let local_idx = match instruction.operand {
            Some(Operand::Local(idx)) => idx as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        let slot = self.current_locals_base() + local_idx;
        if slot >= self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "Local slot {} is out of bounds",
                local_idx
            )));
        }

        let mut object_nb = std::mem::replace(&mut self.stack[slot], ValueWord::none());
        let result = Self::set_array_index_on_object(&mut object_nb, &key_nb, value_nb);
        record_heap_write();
        write_barrier_vw(&ValueWord::none(), &object_nb);
        self.stack[slot] = object_nb;
        result
    }

    pub(in crate::executor) fn op_set_module_binding_index(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let value_nb = self.pop_vw()?;
        let key_nb = self.pop_vw()?;
        let binding_idx = match instruction.operand {
            Some(Operand::ModuleBinding(idx)) => idx as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        if binding_idx >= self.module_bindings.len() {
            self.module_bindings
                .resize_with(binding_idx + 1, ValueWord::none);
        }

        let mut object_nb =
            std::mem::replace(&mut self.module_bindings[binding_idx], ValueWord::none());
        let result = Self::set_array_index_on_object(&mut object_nb, &key_nb, value_nb);
        record_heap_write();
        write_barrier_vw(&ValueWord::none(), &object_nb);
        self.module_bindings[binding_idx] = object_nb;
        result
    }

    pub(in crate::executor) fn op_length(&mut self) -> Result<(), VMError> {
        let nb = self.pop_vw()?;
        // Handle unified arrays (bit-47 tagged).
        if shape_value::tags::is_unified_heap(nb.raw_bits()) {
            let kind = unsafe { shape_value::tags::unified_heap_kind(nb.raw_bits()) };
            if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                let arr = unsafe {
                    shape_value::unified_array::UnifiedArray::from_heap_bits(nb.raw_bits())
                };
                return self.push_vw(ValueWord::from_i64(arr.len() as i64));
            }
        }
        // Fast path: inspect HeapValue directly without materializing ValueWord
        if let Some(hv) = nb.as_heap_ref() {
            let length = match hv {
                HeapValue::Array(arr) => arr.len(),
                HeapValue::IntArray(arr) => arr.len(),
                HeapValue::FloatArray(arr) => arr.len(),
                HeapValue::FloatArraySlice { len, .. } => *len as usize,
                HeapValue::BoolArray(arr) => arr.len(),
                HeapValue::TypedObject { slots, .. } => slots.len(),
                HeapValue::NativeView(view) => view.layout.fields.len(),
                HeapValue::String(s) => s.chars().count(),
                HeapValue::HashMap(d) => d.keys.len(),
                HeapValue::Set(d) => d.items.len(),
                HeapValue::Deque(d) => d.items.len(),
                HeapValue::PriorityQueue(d) => d.items.len(),
                HeapValue::Matrix(m) => m.data.len(),
                _ => {
                    return Err(VMError::TypeError {
                        expected: "array, object, string, or matrix",
                        got: hv.type_name(),
                    });
                }
            };
            return self.push_vw(ValueWord::from_i64(length as i64));
        }
        // Non-heap types don't have length
        Err(VMError::TypeError {
            expected: "array, object, or string",
            got: nb.type_name(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_native_u64_rejects_float_values() {
        let float_nb = ValueWord::from_f64(5.0);
        assert_eq!(read_native_u64(&float_nb), None);
    }

    #[test]
    fn read_native_u64_accepts_exact_unsigned_scalars() {
        let u64_nb = ValueWord::from_native_u64(u64::MAX);
        assert_eq!(read_native_u64(&u64_nb), Some(u64::MAX));
    }

    #[test]
    fn read_native_f64_rejects_native_i64_without_cast() {
        let i64_nb = ValueWord::from_native_scalar(NativeScalar::I64(42));
        assert_eq!(read_native_f64(&i64_nb), None);
    }

    #[test]
    fn read_native_f64_accepts_i48_and_f32() {
        let int_nb = ValueWord::from_i64(7);
        let f32_nb = ValueWord::from_native_f32(3.5);
        assert_eq!(read_native_f64(&int_nb), Some(7.0));
        assert_eq!(read_native_f64(&f32_nb), Some(3.5));
    }
}
