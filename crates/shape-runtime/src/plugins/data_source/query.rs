//! Query handling, validation, and schema parsing for plugin data sources

use std::ffi::{CStr, c_void};
use std::ptr;

use serde_json::Value;
use shape_abi_v1::{DataSourceVTable, OutputField, QueryParam, QuerySchema};
use shape_ast::error::{Result, ShapeError};

use super::schema::{ParsedOutputField, ParsedOutputSchema, ParsedQueryParam, ParsedQuerySchema};

/// Validate a query before execution
pub(super) fn validate_query(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
    query: &Value,
) -> Result<()> {
    let validate_fn = match vtable.validate_query {
        Some(f) => f,
        None => return Ok(()), // No validation function, assume valid
    };

    let query_bytes = rmp_serde::to_vec(query).map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to serialize query: {}", e),
        location: None,
    })?;

    let mut error_ptr: *mut std::ffi::c_char = ptr::null_mut();

    let result = unsafe {
        validate_fn(
            instance,
            query_bytes.as_ptr(),
            query_bytes.len(),
            &mut error_ptr,
        )
    };

    if result != 0 {
        let error_msg = if error_ptr.is_null() {
            "Query validation failed".to_string()
        } else {
            let msg = unsafe { CStr::from_ptr(error_ptr).to_string_lossy().to_string() };
            // Free the error string if we have a free function
            if let Some(free_fn) = vtable.free_string {
                unsafe { free_fn(error_ptr) };
            }
            msg
        };
        return Err(ShapeError::SemanticError {
            message: error_msg,
            location: None,
        });
    }

    Ok(())
}

/// Parse query schema from vtable
pub(super) fn parse_query_schema_from_vtable(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
) -> Result<ParsedQuerySchema> {
    let get_schema_fn = match vtable.get_query_schema {
        Some(f) => f,
        None => {
            return Ok(ParsedQuerySchema {
                params: Vec::new(),
                example_query: None,
            });
        }
    };

    let schema_ptr = unsafe { get_schema_fn(instance) };
    if schema_ptr.is_null() {
        return Ok(ParsedQuerySchema {
            params: Vec::new(),
            example_query: None,
        });
    }

    parse_query_schema(unsafe { &*schema_ptr })
}

/// Parse query schema from C ABI schema
fn parse_query_schema(schema: &QuerySchema) -> Result<ParsedQuerySchema> {
    let mut params = Vec::new();

    if !schema.params.is_null() && schema.params_len > 0 {
        let params_slice = unsafe { std::slice::from_raw_parts(schema.params, schema.params_len) };

        for param in params_slice {
            params.push(parse_query_param(param)?);
        }
    }

    let example_query = if !schema.example_query.is_null() && schema.example_query_len > 0 {
        let bytes =
            unsafe { std::slice::from_raw_parts(schema.example_query, schema.example_query_len) };
        rmp_serde::from_slice(bytes).ok()
    } else {
        None
    };

    Ok(ParsedQuerySchema {
        params,
        example_query,
    })
}

/// Parse a single query parameter from C ABI parameter
fn parse_query_param(param: &QueryParam) -> Result<ParsedQueryParam> {
    let name = unsafe { CStr::from_ptr(param.name) }
        .to_string_lossy()
        .to_string();
    let description = unsafe { CStr::from_ptr(param.description) }
        .to_string_lossy()
        .to_string();

    let default_value = if !param.default_value.is_null() && param.default_value_len > 0 {
        let bytes =
            unsafe { std::slice::from_raw_parts(param.default_value, param.default_value_len) };
        rmp_serde::from_slice(bytes).ok()
    } else {
        None
    };

    let allowed_values = if !param.allowed_values.is_null() && param.allowed_values_len > 0 {
        let bytes =
            unsafe { std::slice::from_raw_parts(param.allowed_values, param.allowed_values_len) };
        rmp_serde::from_slice::<Vec<Value>>(bytes).ok()
    } else {
        None
    };

    let nested_schema = if !param.nested_schema.is_null() {
        Some(Box::new(parse_query_schema(unsafe {
            &*param.nested_schema
        })?))
    } else {
        None
    };

    Ok(ParsedQueryParam {
        name,
        description,
        param_type: param.param_type,
        required: param.required,
        default_value,
        allowed_values,
        nested_schema,
    })
}

/// Parse output schema from vtable
pub(super) fn parse_output_schema_from_vtable(
    vtable: &DataSourceVTable,
    instance: *mut c_void,
) -> Result<ParsedOutputSchema> {
    let get_schema_fn = match vtable.get_output_schema {
        Some(f) => f,
        None => return Ok(ParsedOutputSchema { fields: Vec::new() }),
    };

    let schema_ptr = unsafe { get_schema_fn(instance) };
    if schema_ptr.is_null() {
        return Ok(ParsedOutputSchema { fields: Vec::new() });
    }

    let schema = unsafe { &*schema_ptr };
    let mut fields = Vec::new();

    if !schema.fields.is_null() && schema.fields_len > 0 {
        let fields_slice = unsafe { std::slice::from_raw_parts(schema.fields, schema.fields_len) };

        for field in fields_slice {
            fields.push(parse_output_field(field));
        }
    }

    Ok(ParsedOutputSchema { fields })
}

/// Parse a single output field from C ABI field
fn parse_output_field(field: &OutputField) -> ParsedOutputField {
    ParsedOutputField {
        name: unsafe { CStr::from_ptr(field.name) }
            .to_string_lossy()
            .to_string(),
        field_type: field.field_type,
        description: unsafe { CStr::from_ptr(field.description) }
            .to_string_lossy()
            .to_string(),
    }
}
