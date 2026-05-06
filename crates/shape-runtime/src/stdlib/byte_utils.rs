//! Shared byte array conversion utilities.
//!
//! Used by the `compress` and `archive` modules. With the option β
//! Array<T> marshal landing (cluster #3, 2026-05-06 defections.md
//! entry), the FromSlot layer extracts owned `Vec<i64>` directly
//! from `Array<int>`-typed slots (`HeapKind::TypedArray` /
//! `TypedArrayData::I64`). This module's job shrinks to the
//! per-element 0..=255 range check that turns a `Vec<i64>` of
//! arbitrary integers into a `Vec<u8>`.
//!
//! `bytes_to_array` is gone — bodies return `TypedReturn::Concrete(
//! ConcreteReturn::Bytes(vec))` directly and the dispatcher projects
//! the variant into a typed array slot.

/// Range-check a `Vec<i64>` (semantically `Array<int>` of bytes) into
/// a `Vec<u8>`.
///
/// Each element must be in `0..=255`. Returns an error message naming
/// the out-of-range value on the first violation.
pub fn bytes_from_i64_slice(arr: &[i64]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(arr.len());
    for &byte_val in arr.iter() {
        if !(0..=255).contains(&byte_val) {
            return Err(format!("byte value out of range: {}", byte_val));
        }
        bytes.push(byte_val as u8);
    }
    Ok(bytes)
}
