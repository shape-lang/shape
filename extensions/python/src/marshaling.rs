//! WireValue <-> Python object conversion.

/// Shape type name -> Python type hint string.
pub fn shape_type_to_python_hint(shape_type: &str) -> String {
    match shape_type {
        "number" => "float".to_string(),
        "int" => "int".to_string(),
        "bool" => "bool".to_string(),
        "string" => "str".to_string(),
        "none" => "None".to_string(),
        s if s.starts_with("Array<") => {
            let inner = &s[6..s.len() - 1];
            format!("list[{}]", shape_type_to_python_hint(inner))
        }
        _ => "object".to_string(),
    }
}

/// Convert an rmpv::Value to a Python object.
#[cfg(feature = "pyo3")]
pub fn msgpack_to_pyobject(
    py: pyo3::Python<'_>,
    value: &rmpv::Value,
) -> Result<pyo3::Py<pyo3::PyAny>, String> {
    use pyo3::IntoPyObject;

    match value {
        rmpv::Value::Nil => Ok(py.None().into_pyobject(py).unwrap().unbind().into()),
        rmpv::Value::Boolean(b) => Ok(b.into_pyobject(py).unwrap().to_owned().unbind().into()),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Ok(n.into_pyobject(py).unwrap().unbind().into())
            } else if let Some(n) = i.as_u64() {
                Ok(n.into_pyobject(py).unwrap().unbind().into())
            } else {
                Ok(py.None().into_pyobject(py).unwrap().unbind().into())
            }
        }
        rmpv::Value::F32(f) => Ok((*f as f64).into_pyobject(py).unwrap().unbind().into()),
        rmpv::Value::F64(f) => Ok(f.into_pyobject(py).unwrap().unbind().into()),
        rmpv::Value::String(s) => {
            if let Some(s) = s.as_str() {
                Ok(s.into_pyobject(py).unwrap().unbind().into())
            } else {
                Ok(py.None().into_pyobject(py).unwrap().unbind().into())
            }
        }
        rmpv::Value::Array(arr) => {
            let items: Vec<pyo3::Py<pyo3::PyAny>> = arr
                .iter()
                .map(|v| msgpack_to_pyobject(py, v))
                .collect::<Result<_, _>>()?;
            let list = pyo3::types::PyList::new(py, &items)
                .map_err(|e| format!("Failed to create Python list: {}", e))?;
            Ok(list.unbind().into())
        }
        rmpv::Value::Map(entries) => {
            use pyo3::types::PyDictMethods;
            let dict = pyo3::types::PyDict::new(py);
            for (k, v) in entries {
                let py_key = msgpack_to_pyobject(py, k)?;
                let py_val = msgpack_to_pyobject(py, v)?;
                dict.set_item(py_key, py_val)
                    .map_err(|e| format!("Failed to set dict item: {}", e))?;
            }
            Ok(dict.unbind().into())
        }
        rmpv::Value::Binary(_) | rmpv::Value::Ext(_, _) => {
            Ok(py.None().into_pyobject(py).unwrap().unbind().into())
        }
    }
}

/// Convert a Python object to an rmpv::Value (untyped path).
#[cfg(feature = "pyo3")]
pub fn pyobject_to_msgpack(
    py: pyo3::Python<'_>,
    obj: &pyo3::Bound<'_, pyo3::PyAny>,
) -> Result<rmpv::Value, String> {
    use pyo3::types::*;

    // Check bool BEFORE int (bool is subclass of int in Python)
    if obj.is_instance_of::<PyBool>() {
        let b: bool = obj
            .extract()
            .map_err(|e| format!("Failed to extract bool: {}", e))?;
        return Ok(rmpv::Value::Boolean(b));
    }

    if obj.is_instance_of::<PyInt>() {
        let i: i64 = obj
            .extract()
            .map_err(|e| format!("Failed to extract int: {}", e))?;
        return Ok(rmpv::Value::Integer(rmpv::Integer::from(i)));
    }

    if obj.is_instance_of::<PyFloat>() {
        let f: f64 = obj
            .extract()
            .map_err(|e| format!("Failed to extract float: {}", e))?;
        return Ok(rmpv::Value::F64(f));
    }

    if obj.is_instance_of::<PyString>() {
        let s: String = obj
            .extract()
            .map_err(|e| format!("Failed to extract string: {}", e))?;
        return Ok(rmpv::Value::String(rmpv::Utf8String::from(s)));
    }

    if obj.is_none() {
        return Ok(rmpv::Value::Nil);
    }

    if obj.is_instance_of::<PyList>() {
        let list = obj
            .cast::<PyList>()
            .map_err(|e| format!("Failed to downcast list: {}", e))?;
        let items: Vec<rmpv::Value> = list
            .iter()
            .map(|item| pyobject_to_msgpack(py, &item))
            .collect::<Result<_, _>>()?;
        return Ok(rmpv::Value::Array(items));
    }

    if obj.is_instance_of::<PyDict>() {
        let dict = obj
            .cast::<PyDict>()
            .map_err(|e| format!("Failed to downcast dict: {}", e))?;
        let entries: Vec<(rmpv::Value, rmpv::Value)> = dict
            .iter()
            .map(|(k, v)| {
                let mk = pyobject_to_msgpack(py, &k)?;
                let mv = pyobject_to_msgpack(py, &v)?;
                Ok((mk, mv))
            })
            .collect::<Result<_, String>>()?;
        return Ok(rmpv::Value::Map(entries));
    }

    // Fallback: try to convert to string representation
    Ok(rmpv::Value::Nil)
}

// ============================================================================
// Type-aware marshalling
// ============================================================================

/// Strip `Result<...>` wrapper from a type string, returning the inner type.
pub fn strip_result_wrapper(s: &str) -> &str {
    if s.starts_with("Result<") && s.ends_with('>') {
        &s[7..s.len() - 1]
    } else {
        s
    }
}

/// Extract `T` from `Array<T>`, returning `None` if `s` is not an Array type.
fn strip_array_wrapper(s: &str) -> Option<&str> {
    if s.starts_with("Array<") && s.ends_with('>') {
        Some(&s[6..s.len() - 1])
    } else {
        None
    }
}

/// Parse `{f1: T1, f2: T2}` to a Vec of (field_name, field_type) pairs.
fn parse_object_fields(s: &str) -> Vec<(&str, &str)> {
    let s = s.trim();
    if !s.starts_with('{') || !s.ends_with('}') {
        return Vec::new();
    }
    let inner = s[1..s.len() - 1].trim();
    if inner.is_empty() {
        return Vec::new();
    }

    let mut fields = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;

    // Split on commas, respecting nested angle brackets and braces
    for (i, ch) in inner.char_indices() {
        match ch {
            '<' | '{' => depth += 1,
            '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                if let Some(pair) = parse_single_field(inner[start..i].trim()) {
                    fields.push(pair);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    // Last field
    if let Some(pair) = parse_single_field(inner[start..].trim()) {
        fields.push(pair);
    }

    fields
}

/// Parse a single `name: type` or `name?: type` field spec.
fn parse_single_field(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    let colon_pos = s.find(':')?;
    let name = s[..colon_pos].trim().trim_end_matches('?');
    let typ = s[colon_pos + 1..].trim();
    if name.is_empty() || typ.is_empty() {
        return None;
    }
    Some((name, typ))
}

/// Convert a Python object to an rmpv::Value using the declared Shape return type
/// for validation and coercion.
///
/// This is the typed marshalling path. It validates that the Python value matches
/// the declared type and coerces where the rules allow (e.g. float 3.0 -> int 3).
#[cfg(feature = "pyo3")]
pub fn pyobject_to_typed_msgpack(
    py: pyo3::Python<'_>,
    obj: &pyo3::Bound<'_, pyo3::PyAny>,
    target_type: &str,
) -> Result<rmpv::Value, String> {
    let inner = strip_result_wrapper(target_type);
    convert_with_type(py, obj, inner)
}

#[cfg(feature = "pyo3")]
fn convert_with_type(
    py: pyo3::Python<'_>,
    obj: &pyo3::Bound<'_, pyo3::PyAny>,
    target: &str,
) -> Result<rmpv::Value, String> {
    use pyo3::types::*;

    // Handle None/nil
    if obj.is_none() {
        return match target {
            "none" => Ok(rmpv::Value::Nil),
            _ => Err(format!("expected {}, got None", target)),
        };
    }

    match target {
        "int" => {
            // Bool is NOT coerced to int — reject it explicitly
            if obj.is_instance_of::<PyBool>() {
                return Err("expected int, got bool (bool is not coerced to int)".to_string());
            }
            if obj.is_instance_of::<PyInt>() {
                let i: i64 = obj
                    .extract()
                    .map_err(|e| format!("Failed to extract int: {}", e))?;
                return Ok(rmpv::Value::Integer(rmpv::Integer::from(i)));
            }
            // Coerce float with integer value (3.0) -> int
            if obj.is_instance_of::<PyFloat>() {
                let f: f64 = obj
                    .extract()
                    .map_err(|e| format!("Failed to extract float: {}", e))?;
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    return Ok(rmpv::Value::Integer(rmpv::Integer::from(f as i64)));
                }
                return Err(format!(
                    "expected int, got float {} (not an integer value)",
                    f
                ));
            }
            Err(format!("expected int, got {}", py_type_name(obj)))
        }

        "float" | "number" => {
            if obj.is_instance_of::<PyBool>() {
                return Err(format!("expected {}, got bool", target));
            }
            if obj.is_instance_of::<PyFloat>() {
                let f: f64 = obj
                    .extract()
                    .map_err(|e| format!("Failed to extract float: {}", e))?;
                return Ok(rmpv::Value::F64(f));
            }
            // Coerce int -> float
            if obj.is_instance_of::<PyInt>() {
                let i: i64 = obj
                    .extract()
                    .map_err(|e| format!("Failed to extract int: {}", e))?;
                return Ok(rmpv::Value::F64(i as f64));
            }
            Err(format!("expected {}, got {}", target, py_type_name(obj)))
        }

        "string" => {
            if obj.is_instance_of::<PyString>() {
                let s: String = obj
                    .extract()
                    .map_err(|e| format!("Failed to extract string: {}", e))?;
                return Ok(rmpv::Value::String(rmpv::Utf8String::from(s)));
            }
            Err(format!("expected string, got {}", py_type_name(obj)))
        }

        "bool" => {
            if obj.is_instance_of::<PyBool>() {
                let b: bool = obj
                    .extract()
                    .map_err(|e| format!("Failed to extract bool: {}", e))?;
                return Ok(rmpv::Value::Boolean(b));
            }
            Err(format!("expected bool, got {}", py_type_name(obj)))
        }

        "none" => Err(format!("expected none, got {}", py_type_name(obj))),

        // Array<T>
        s if strip_array_wrapper(s).is_some() => {
            let elem_type = strip_array_wrapper(s).unwrap();
            if !obj.is_instance_of::<PyList>() {
                return Err(format!("expected Array, got {}", py_type_name(obj)));
            }
            let list = obj
                .cast::<PyList>()
                .map_err(|e| format!("Failed to downcast list: {}", e))?;
            let items: Vec<rmpv::Value> = list
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    convert_with_type(py, &item, elem_type)
                        .map_err(|e| format!("Array element [{}]: {}", i, e))
                })
                .collect::<Result<_, _>>()?;
            Ok(rmpv::Value::Array(items))
        }

        // Object type: {f1: T1, f2: T2, ...}
        s if s.starts_with('{') && s.ends_with('}') => {
            if !obj.is_instance_of::<PyDict>() {
                return Err(format!("expected object, got {}", py_type_name(obj)));
            }
            let dict = obj
                .cast::<PyDict>()
                .map_err(|e| format!("Failed to downcast dict: {}", e))?;

            let declared_fields = parse_object_fields(s);
            if declared_fields.is_empty() {
                // Empty object declaration or parse failure — fall back to untyped
                return pyobject_to_msgpack(py, obj);
            }

            let mut entries = Vec::with_capacity(declared_fields.len());
            for (field_name, field_type) in &declared_fields {
                let key_obj = pyo3::types::PyString::new(py, field_name);
                let value_obj = dict
                    .get_item(&key_obj)
                    .map_err(|e| format!("Failed to get field '{}': {}", field_name, e))?;
                let value_obj = value_obj.ok_or_else(|| {
                    format!("missing required field '{}' in returned dict", field_name)
                })?;
                let typed_val = convert_with_type(py, &value_obj, field_type)
                    .map_err(|e| format!("field '{}': {}", field_name, e))?;
                entries.push((
                    rmpv::Value::String(rmpv::Utf8String::from(field_name.to_string())),
                    typed_val,
                ));
            }
            Ok(rmpv::Value::Map(entries))
        }

        // "any" or unknown — fall back to untyped
        _ => pyobject_to_msgpack(py, obj),
    }
}

/// Get a human-readable Python type name for error messages.
#[cfg(feature = "pyo3")]
fn py_type_name(obj: &pyo3::Bound<'_, pyo3::PyAny>) -> &'static str {
    use pyo3::types::*;
    if obj.is_instance_of::<PyBool>() {
        "bool"
    } else if obj.is_instance_of::<PyInt>() {
        "int"
    } else if obj.is_instance_of::<PyFloat>() {
        "float"
    } else if obj.is_instance_of::<PyString>() {
        "string"
    } else if obj.is_instance_of::<PyList>() {
        "list"
    } else if obj.is_instance_of::<PyDict>() {
        "dict"
    } else if obj.is_none() {
        "None"
    } else {
        "object"
    }
}
