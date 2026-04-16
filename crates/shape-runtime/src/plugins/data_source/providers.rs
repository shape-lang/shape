//! Data loading implementations for plugin data sources
//!
//! This module contains the data loading methods including historical data,
//! streaming subscriptions, and binary columnar format loading.

use std::ffi::c_void;
use std::ptr;

use serde_json::Value;
use shape_abi_v1::DataSourceVTable;
use shape_value::ValueWordExt;
use shape_ast::error::{Result, ShapeError};

use super::query;

/// Load historical data
pub(super) fn load(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
    name: &str,
    query: &Value,
) -> Result<Value> {
    // Validate first
    query::validate_query(vtable, instance, query)?;

    let load_fn = vtable.load.ok_or_else(|| ShapeError::RuntimeError {
        message: format!("Plugin '{}' has no load function", name),
        location: None,
    })?;

    let query_bytes = rmp_serde::to_vec(query).map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to serialize query: {}", e),
        location: None,
    })?;

    let mut out_ptr: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;

    let result = unsafe {
        load_fn(
            instance,
            query_bytes.as_ptr(),
            query_bytes.len(),
            &mut out_ptr,
            &mut out_len,
        )
    };

    if result != 0 {
        return Err(ShapeError::RuntimeError {
            message: format!("Plugin '{}' load failed with code {}", name, result),
            location: None,
        });
    }

    if out_ptr.is_null() || out_len == 0 {
        return Err(ShapeError::RuntimeError {
            message: format!("Plugin '{}' returned empty data", name),
            location: None,
        });
    }

    // Deserialize the result
    let data_slice = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let value: Value = rmp_serde::from_slice(data_slice).map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to deserialize plugin response: {}", e),
        location: None,
    })?;

    // Free the buffer
    if let Some(free_fn) = vtable.free_buffer {
        unsafe { free_fn(out_ptr, out_len) };
    }

    Ok(value)
}

/// Subscribe to streaming data
///
/// # Arguments
/// * `vtable` - Data source vtable
/// * `instance` - Plugin instance pointer
/// * `name` - Plugin name (for error messages)
/// * `query` - Query parameters
/// * `callback` - Called for each data point
///
/// # Returns
/// Subscription ID (use to unsubscribe)
pub(super) fn subscribe<F>(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
    name: &str,
    query: &Value,
    callback: F,
) -> Result<u64>
where
    F: Fn(Value) + Send + Sync + 'static,
{
    let subscribe_fn = vtable.subscribe.ok_or_else(|| ShapeError::RuntimeError {
        message: format!("Plugin '{}' does not support streaming", name),
        location: None,
    })?;

    let query_bytes = rmp_serde::to_vec(query).map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to serialize query: {}", e),
        location: None,
    })?;

    // Box the callback so we can pass it through FFI
    let callback_box = Box::new(callback);
    let callback_ptr = Box::into_raw(callback_box) as *mut c_void;

    // FFI callback that calls our Rust closure
    extern "C" fn ffi_callback<F: Fn(Value)>(
        data_ptr: *const u8,
        data_len: usize,
        user_data: *mut c_void,
    ) {
        if data_ptr.is_null() || data_len == 0 {
            return;
        }
        let callback = unsafe { &*(user_data as *const F) };
        let data_slice = unsafe { std::slice::from_raw_parts(data_ptr, data_len) };
        if let Ok(value) = rmp_serde::from_slice::<Value>(data_slice) {
            callback(value);
        }
    }

    let subscription_id = unsafe {
        subscribe_fn(
            instance,
            query_bytes.as_ptr(),
            query_bytes.len(),
            ffi_callback::<F>,
            callback_ptr,
        )
    };

    if subscription_id == 0 {
        // Clean up the callback box since subscription failed
        let _ = unsafe { Box::from_raw(callback_ptr as *mut F) };
        return Err(ShapeError::RuntimeError {
            message: format!("Plugin '{}' subscribe failed", name),
            location: None,
        });
    }

    Ok(subscription_id)
}

/// Unsubscribe from streaming data
pub(super) fn unsubscribe(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
    name: &str,
    subscription_id: u64,
) -> Result<()> {
    let unsubscribe_fn = vtable.unsubscribe.ok_or_else(|| ShapeError::RuntimeError {
        message: format!("Plugin '{}' does not support unsubscribe", name),
        location: None,
    })?;

    let result = unsafe { unsubscribe_fn(instance, subscription_id) };

    if result != 0 {
        return Err(ShapeError::RuntimeError {
            message: format!("Plugin '{}' unsubscribe failed with code {}", name, result),
            location: None,
        });
    }

    Ok(())
}

/// Query the schema for a specific data source.
///
/// This enables runtime schema discovery - the plugin returns what columns
/// are available for a given source ID.
///
/// # Arguments
/// * `vtable` - Data source vtable
/// * `instance` - Plugin instance pointer
/// * `name` - Plugin name (for error messages)
/// * `source_id` - The source identifier (e.g., symbol, table name, device ID)
///
/// # Returns
/// The plugin schema with column information
pub(super) fn get_source_schema(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
    name: &str,
    source_id: &str,
) -> Result<shape_abi_v1::PluginSchema> {
    let get_schema_fn = vtable
        .get_source_schema
        .ok_or_else(|| ShapeError::RuntimeError {
            message: format!("Plugin '{}' does not support schema discovery", name),
            location: None,
        })?;

    let source_bytes = source_id.as_bytes();
    let mut out_ptr: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;

    let result = unsafe {
        get_schema_fn(
            instance,
            source_bytes.as_ptr(),
            source_bytes.len(),
            &mut out_ptr,
            &mut out_len,
        )
    };

    if result != 0 {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' get_source_schema failed with code {}",
                name, result
            ),
            location: None,
        });
    }

    if out_ptr.is_null() || out_len == 0 {
        return Err(ShapeError::RuntimeError {
            message: format!("Plugin '{}' returned empty schema", name),
            location: None,
        });
    }

    // Deserialize the result
    let data_slice = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let schema: shape_abi_v1::PluginSchema =
        rmp_serde::from_slice(data_slice).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to deserialize plugin schema: {}", e),
            location: None,
        })?;

    // Free the buffer
    if let Some(free_fn) = vtable.free_buffer {
        unsafe { free_fn(out_ptr, out_len) };
    }

    Ok(schema)
}

/// Load historical data in binary columnar format (ABI v2)
pub(super) fn load_binary(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
    name: &str,
    query: &Value,
    _granularity: crate::progress::ProgressGranularity,
    _progress_handle: Option<&crate::progress::ProgressHandle>,
) -> Result<shape_value::ValueWord> {
    let load_fn = vtable.load_binary.ok_or_else(|| ShapeError::RuntimeError {
        message: format!("Plugin '{}' has no load_binary function", name),
        location: None,
    })?;

    let query_bytes = rmp_serde::to_vec(query).map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to serialize query: {}", e),
        location: None,
    })?;

    let mut out_ptr: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;

    let result = unsafe {
        load_fn(
            instance,
            query_bytes.as_ptr(),
            query_bytes.len(),
            0,               // granularity
            None,            // progress_callback
            ptr::null_mut(), // progress_user_data
            &mut out_ptr,
            &mut out_len,
        )
    };

    if result != 0 {
        return Err(ShapeError::RuntimeError {
            message: format!("Plugin '{}' load_binary failed with code {}", name, result),
            location: None,
        });
    }

    if out_ptr.is_null() || out_len == 0 {
        return Err(ShapeError::RuntimeError {
            message: format!("Plugin '{}' returned empty binary data", name),
            location: None,
        });
    }

    let data_slice = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let dt = crate::binary_reader::read_binary_to_datatable(data_slice)?;

    // Free the buffer
    if let Some(free_fn) = vtable.free_buffer {
        unsafe { free_fn(out_ptr, out_len) };
    }

    Ok(shape_value::ValueWord::from_datatable(std::sync::Arc::new(
        dt,
    )))
}
