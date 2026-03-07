//! Shape <-> V8 type conversion via MessagePack.
//!
//! Converts between msgpack-encoded Shape wire values and V8 JavaScript values.

use deno_core::v8;

// ============================================================================
// Shape type -> TypeScript type hint (for generated wrappers)
// ============================================================================

/// Convert a Shape type name to a TypeScript type annotation string.
pub fn shape_type_to_ts_hint(shape_type: &str) -> String {
    match shape_type {
        "number" => "number".to_string(),
        "int" => "number".to_string(),
        "bool" => "boolean".to_string(),
        "string" => "string".to_string(),
        "none" => "void".to_string(),
        s if s.starts_with("Array<") => {
            let inner = &s[6..s.len() - 1];
            format!("Array<{}>", shape_type_to_ts_hint(inner))
        }
        _ => "any".to_string(),
    }
}

// ============================================================================
// msgpack -> V8
// ============================================================================

/// Deserialize a msgpack byte buffer into a vector of V8 values.
///
/// The buffer is expected to contain a msgpack array whose elements
/// correspond 1:1 with the function's positional parameters.
pub fn msgpack_to_v8<'s>(
    scope: &mut v8::HandleScope<'s>,
    bytes: &[u8],
) -> Result<Vec<v8::Local<'s, v8::Value>>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }

    let values: Vec<rmpv::Value> =
        rmp_serde::from_slice(bytes).map_err(|e| format!("Failed to deserialize args: {}", e))?;

    values
        .iter()
        .map(|v| rmpv_to_v8(scope, v))
        .collect::<Result<Vec<_>, _>>()
}

/// Convert a single rmpv::Value to a V8 value.
fn rmpv_to_v8<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: &rmpv::Value,
) -> Result<v8::Local<'s, v8::Value>, String> {
    match value {
        rmpv::Value::Nil => Ok(v8::null(scope).into()),

        rmpv::Value::Boolean(b) => Ok(v8::Boolean::new(scope, *b).into()),

        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                // Use Integer if it fits in i32, otherwise Number (f64)
                if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
                    Ok(v8::Integer::new(scope, n as i32).into())
                } else {
                    Ok(v8::Number::new(scope, n as f64).into())
                }
            } else if let Some(n) = i.as_u64() {
                if n <= i32::MAX as u64 {
                    Ok(v8::Integer::new_from_unsigned(scope, n as u32).into())
                } else {
                    Ok(v8::Number::new(scope, n as f64).into())
                }
            } else {
                Ok(v8::null(scope).into())
            }
        }

        rmpv::Value::F32(f) => Ok(v8::Number::new(scope, *f as f64).into()),

        rmpv::Value::F64(f) => Ok(v8::Number::new(scope, *f).into()),

        rmpv::Value::String(s) => {
            if let Some(s) = s.as_str() {
                let v8_str = v8::String::new(scope, s)
                    .ok_or_else(|| "Failed to create V8 string".to_string())?;
                Ok(v8_str.into())
            } else {
                Ok(v8::null(scope).into())
            }
        }

        rmpv::Value::Array(arr) => {
            let v8_arr = v8::Array::new(scope, arr.len() as i32);
            for (i, elem) in arr.iter().enumerate() {
                let v8_val = rmpv_to_v8(scope, elem)?;
                v8_arr.set_index(scope, i as u32, v8_val);
            }
            Ok(v8_arr.into())
        }

        rmpv::Value::Map(entries) => {
            let v8_obj = v8::Object::new(scope);
            for (k, v) in entries {
                let v8_key = rmpv_to_v8(scope, k)?;
                let v8_val = rmpv_to_v8(scope, v)?;
                v8_obj.set(scope, v8_key, v8_val);
            }
            Ok(v8_obj.into())
        }

        rmpv::Value::Binary(_) | rmpv::Value::Ext(_, _) => Ok(v8::null(scope).into()),
    }
}

// ============================================================================
// V8 -> msgpack
// ============================================================================

/// Convert a V8 value to a msgpack byte buffer.
pub fn v8_to_msgpack(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<u8>, String> {
    let rmpv_val = v8_to_rmpv(scope, value)?;
    rmp_serde::to_vec(&rmpv_val).map_err(|e| format!("Failed to serialize result: {}", e))
}

/// Convert a V8 value to an rmpv::Value.
fn v8_to_rmpv(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<rmpv::Value, String> {
    if value.is_null_or_undefined() {
        return Ok(rmpv::Value::Nil);
    }

    if value.is_boolean() {
        let b = value.boolean_value(scope);
        return Ok(rmpv::Value::Boolean(b));
    }

    // Check integer before general number (V8 integers are also numbers)
    if value.is_int32() {
        let i = value.int32_value(scope).unwrap_or(0);
        return Ok(rmpv::Value::Integer(rmpv::Integer::from(i as i64)));
    }

    if value.is_uint32() {
        let u = value.uint32_value(scope).unwrap_or(0);
        return Ok(rmpv::Value::Integer(rmpv::Integer::from(u as i64)));
    }

    if value.is_number() {
        let f = value.number_value(scope).unwrap_or(0.0);
        // If the float is an exact integer that fits in i64, encode as integer
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            return Ok(rmpv::Value::Integer(rmpv::Integer::from(f as i64)));
        }
        return Ok(rmpv::Value::F64(f));
    }

    if value.is_string() {
        let s = value.to_rust_string_lossy(scope);
        return Ok(rmpv::Value::String(rmpv::Utf8String::from(s)));
    }

    if value.is_array() {
        let arr = v8::Local::<v8::Array>::try_from(value)
            .map_err(|_| "Failed to cast to Array".to_string())?;
        let len = arr.length();
        let mut items = Vec::with_capacity(len as usize);
        for i in 0..len {
            if let Some(elem) = arr.get_index(scope, i) {
                items.push(v8_to_rmpv(scope, elem)?);
            } else {
                items.push(rmpv::Value::Nil);
            }
        }
        return Ok(rmpv::Value::Array(items));
    }

    if value.is_object() {
        let obj = value.to_object(scope).unwrap();
        let property_names = obj
            .get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
            .ok_or_else(|| "Failed to get property names".to_string())?;
        let len = property_names.length();
        let mut entries = Vec::with_capacity(len as usize);
        for i in 0..len {
            let key = property_names.get_index(scope, i).unwrap();
            let val = obj
                .get(scope, key)
                .unwrap_or_else(|| v8::null(scope).into());
            let rmpv_key = v8_to_rmpv(scope, key)?;
            let rmpv_val = v8_to_rmpv(scope, val)?;
            entries.push((rmpv_key, rmpv_val));
        }
        return Ok(rmpv::Value::Map(entries));
    }

    // Fallback for symbols, bigints, functions, etc.
    Ok(rmpv::Value::Nil)
}
