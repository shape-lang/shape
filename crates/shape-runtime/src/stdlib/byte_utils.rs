//! Shared byte array conversion utilities.
//!
//! Used by both `compress` and `archive` modules for converting between
//! Shape's `Array<int>` representation and Rust `Vec<u8>`.

use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

/// Extract a byte array (`Array<int>`) from a ValueWord into a `Vec<u8>`.
///
/// Each array element must be an integer in the range 0..=255.
pub fn bytes_from_array(val: &ValueWord) -> Result<Vec<u8>, String> {
    let arr = val
        .as_any_array()
        .ok_or_else(|| "expected an Array<int> of bytes".to_string())?
        .to_generic();
    let mut bytes = Vec::with_capacity(arr.len());
    for item in arr.iter() {
        let byte_val = item
            .as_i64()
            .or_else(|| item.as_f64().map(|n| n as i64))
            .ok_or_else(|| "array elements must be integers (0-255)".to_string())?;
        if !(0..=255).contains(&byte_val) {
            return Err(format!("byte value out of range: {}", byte_val));
        }
        bytes.push(byte_val as u8);
    }
    Ok(bytes)
}

/// Convert a `Vec<u8>` into a ValueWord `Array<int>`.
pub fn bytes_to_array(bytes: &[u8]) -> ValueWord {
    let items: Vec<ValueWord> = bytes
        .iter()
        .map(|&b| ValueWord::from_i64(b as i64))
        .collect();
    ValueWord::from_array(shape_value::vmarray_from_vec(items))
}
