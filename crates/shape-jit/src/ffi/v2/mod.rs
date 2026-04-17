//! v2 typed FFI functions for JIT-compiled code.
//!
//! These functions use native types (f64, i64, i32, raw pointers) instead of
//! NaN-boxed u64 values. They are called from JIT-compiled v2 code via direct
//! extern "C" calls.

pub mod typed_map;

pub use typed_map::{
    jit_v2_map_get_str_f64, jit_v2_map_get_str_i64, jit_v2_map_has_str, jit_v2_map_len,
    jit_v2_map_set_str_i64,
};

use shape_value::v2::heap_header::HeapHeader;
use shape_value::v2::typed_array::TypedArray;

// ============================================================================
// Array FFI — f64
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_f64(capacity: u32) -> *mut TypedArray<f64> {
    TypedArray::<f64>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_f64(arr: *const TypedArray<f64>, index: i64) -> f64 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array f64 index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_f64(arr: *mut TypedArray<f64>, index: i64, val: f64) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_f64(arr: *mut TypedArray<f64>, val: f64) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_f64(arr: *const TypedArray<f64>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

/// SIMD-accelerated sum over a `TypedArray<f64>` (Phase C.3).
///
/// Uses `wide::f64x4` for 4-lane parallel addition when `len >= 16`. Below
/// that threshold, the vector load/splat overhead exceeds the savings so we
/// fall back to scalar accumulation. Returns `0.0` for null or empty arrays.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return 0.0;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0.0;
    }
    unsafe { simd_sum_f64_inner(data, len) }
}

/// SIMD-accelerated sum over a `TypedArray<i64>` (Phase C.3). Uses wrapping
/// arithmetic (matches Shape's v2 int-sum semantics — no overflow panic).
///
/// # Safety
/// `arr` must be a valid `TypedArray<i64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_i64(arr: *const TypedArray<i64>) -> i64 {
    if arr.is_null() {
        return 0;
    }
    let (data, len) = unsafe { ((*arr).data as *const i64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0;
    }
    unsafe { simd_sum_i64_inner(data, len) }
}

/// SIMD reduction threshold — below this, setup cost dominates.
const SIMD_SUM_THRESHOLD: usize = 16;

// ── Min / Max / Mean / Sum-of-squares over Array<number> ─────────────────

/// SIMD-accelerated minimum over a `TypedArray<f64>`. Returns `NaN` for null
/// or empty arrays (matches `Vec<number>.min()` semantics on empty input).
/// NaN propagates naturally — `fast_min(NaN, v)` yields `NaN` on all
/// compliant backends.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_min_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return f64::NAN;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return f64::NAN;
    }
    unsafe { simd_min_f64_inner(data, len) }
}

/// SIMD-accelerated maximum over a `TypedArray<f64>`. Returns `NaN` for null
/// or empty arrays. NaN propagates.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_max_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return f64::NAN;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return f64::NAN;
    }
    unsafe { simd_max_f64_inner(data, len) }
}

/// SIMD-accelerated mean (arithmetic average) over a `TypedArray<f64>`.
/// Returns `NaN` for null or empty arrays.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_mean_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return f64::NAN;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return f64::NAN;
    }
    let sum = unsafe { simd_sum_f64_inner(data, len) };
    sum / (len as f64)
}

/// SIMD-accelerated sum-of-squares over a `TypedArray<f64>` — `Σ x²`.
/// Single-pass: load, multiply, accumulate. Useful as a building block for
/// variance/std and for `arr.map(|x| x*x).sum()` patterns. Returns `0.0`
/// for null or empty arrays.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_squares_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return 0.0;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0.0;
    }
    unsafe { simd_sum_squares_f64_inner(data, len) }
}

// ── Element-wise scalar ops returning a new Array<number> ────────────────

/// Allocate a new `TypedArray<f64>` with length equal to `arr.len`, populated
/// by multiplying each element by `factor`. Returns a null pointer when the
/// receiver is null.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_scale_f64(
    arr: *const TypedArray<f64>,
    factor: f64,
) -> *mut TypedArray<f64> {
    if arr.is_null() {
        return std::ptr::null_mut();
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    let out = TypedArray::<f64>::with_capacity(len as u32);
    if len == 0 || data.is_null() {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_scale_f64_inner(data, out_data, len, factor);
        (*out).len = len as u32;
    }
    out
}

/// Allocate a new `TypedArray<f64>` with length equal to `arr.len`, populated
/// by adding `offset` to each element. Returns a null pointer when the
/// receiver is null.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_add_scalar_f64(
    arr: *const TypedArray<f64>,
    offset: f64,
) -> *mut TypedArray<f64> {
    if arr.is_null() {
        return std::ptr::null_mut();
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    let out = TypedArray::<f64>::with_capacity(len as u32);
    if len == 0 || data.is_null() {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_add_scalar_f64_inner(data, out_data, len, offset);
        (*out).len = len as u32;
    }
    out
}

// ── Element-wise binary ops (two arrays) ─────────────────────────────────

/// Allocate a new `TypedArray<f64>` holding the element-wise sum of `a` and
/// `b`. Requires matching lengths — panics on mismatch to mirror Shape's
/// `dot()`/runtime length-mismatch semantics. Returns null for null inputs.
///
/// # Safety
/// `a` and `b` must be valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_add_f64(
    a: *const TypedArray<f64>,
    b: *const TypedArray<f64>,
) -> *mut TypedArray<f64> {
    if a.is_null() || b.is_null() {
        return std::ptr::null_mut();
    }
    let (a_data, a_len) = unsafe { ((*a).data as *const f64, (*a).len as usize) };
    let (b_data, b_len) = unsafe { ((*b).data as *const f64, (*b).len as usize) };
    if a_len != b_len {
        panic!(
            "v2 array_add_f64: length mismatch ({} vs {})",
            a_len, b_len
        );
    }
    let out = TypedArray::<f64>::with_capacity(a_len as u32);
    if a_len == 0 {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_binary_add_f64_inner(a_data, b_data, out_data, a_len);
        (*out).len = a_len as u32;
    }
    out
}

/// Allocate a new `TypedArray<f64>` holding the element-wise product of `a`
/// and `b`. Requires matching lengths.
///
/// # Safety
/// `a` and `b` must be valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_mul_f64(
    a: *const TypedArray<f64>,
    b: *const TypedArray<f64>,
) -> *mut TypedArray<f64> {
    if a.is_null() || b.is_null() {
        return std::ptr::null_mut();
    }
    let (a_data, a_len) = unsafe { ((*a).data as *const f64, (*a).len as usize) };
    let (b_data, b_len) = unsafe { ((*b).data as *const f64, (*b).len as usize) };
    if a_len != b_len {
        panic!(
            "v2 array_mul_f64: length mismatch ({} vs {})",
            a_len, b_len
        );
    }
    let out = TypedArray::<f64>::with_capacity(a_len as u32);
    if a_len == 0 {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_binary_mul_f64_inner(a_data, b_data, out_data, a_len);
        (*out).len = a_len as u32;
    }
    out
}

#[inline]
unsafe fn simd_sum_f64_inner(data: *const f64, len: usize) -> f64 {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s = 0.0_f64;
        for i in 0..len {
            s += unsafe { *data.add(i) };
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = f64x4::splat(0.0);
    for i in 0..chunks {
        let b = i * 4;
        let v = unsafe {
            f64x4::from([
                *data.add(b),
                *data.add(b + 1),
                *data.add(b + 2),
                *data.add(b + 3),
            ])
        };
        acc += v;
    }
    let parts = acc.to_array();
    let mut s = parts[0] + parts[1] + parts[2] + parts[3];
    for i in (chunks * 4)..len {
        s += unsafe { *data.add(i) };
    }
    s
}

#[inline]
unsafe fn simd_sum_i64_inner(data: *const i64, len: usize) -> i64 {
    use wide::i64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s: i64 = 0;
        for i in 0..len {
            s = s.wrapping_add(unsafe { *data.add(i) });
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = i64x4::splat(0);
    for i in 0..chunks {
        let b = i * 4;
        let v = unsafe {
            i64x4::from([
                *data.add(b),
                *data.add(b + 1),
                *data.add(b + 2),
                *data.add(b + 3),
            ])
        };
        // wide::i64x4 lacks AddAssign; rebind the accumulator.
        acc = acc + v;
    }
    let parts = acc.to_array();
    let mut s = parts[0]
        .wrapping_add(parts[1])
        .wrapping_add(parts[2])
        .wrapping_add(parts[3]);
    for i in (chunks * 4)..len {
        s = s.wrapping_add(unsafe { *data.add(i) });
    }
    s
}

/// Load four f64 values starting at `data[base]` into an `f64x4` lane.
///
/// # Safety
/// `data` must point to at least `base + 4` valid `f64` values.
#[inline]
unsafe fn load_f64x4(data: *const f64, base: usize) -> wide::f64x4 {
    unsafe {
        wide::f64x4::from([
            *data.add(base),
            *data.add(base + 1),
            *data.add(base + 2),
            *data.add(base + 3),
        ])
    }
}

/// Detect a NaN anywhere in a f64 buffer. Uses SIMD lanes via a
/// bitwise-equal comparison-against-self (NaN is the only value that is
/// not equal to itself under IEEE 754).
///
/// # Safety
/// `data` must point to at least `len` valid `f64` values.
#[inline]
unsafe fn contains_nan_f64(data: *const f64, len: usize) -> bool {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            if unsafe { *data.add(i) }.is_nan() {
                return true;
            }
        }
        return false;
    }
    let chunks = len / 4;
    for i in 0..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        // NaN != NaN — a SIMD self-compare leaves NaN lanes as 0x0, other
        // lanes as all-ones. `wide` doesn't expose a portable movemask, so
        // we materialize the 4 lanes and scalar-check.
        let arr = v.to_array();
        if arr[0].is_nan() || arr[1].is_nan() || arr[2].is_nan() || arr[3].is_nan() {
            return true;
        }
    }
    for i in (chunks * 4)..len {
        if unsafe { *data.add(i) }.is_nan() {
            return true;
        }
    }
    false
}

#[inline]
unsafe fn simd_min_f64_inner(data: *const f64, len: usize) -> f64 {
    use wide::f64x4;
    // Hardware `min_pd` does NOT reliably propagate NaN (it returns the
    // non-NaN operand in whichever slot based on the comparison order).
    // Do a cheap SIMD NaN scan first — if present, short-circuit to NaN
    // to match scalar `f64::min` semantics that our consumers expect.
    if unsafe { contains_nan_f64(data, len) } {
        return f64::NAN;
    }
    if len < SIMD_SUM_THRESHOLD {
        let mut m = unsafe { *data };
        for i in 1..len {
            let v = unsafe { *data.add(i) };
            if v < m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = unsafe { load_f64x4(data, 0) };
    for i in 1..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        acc = acc.fast_min(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p < m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        if v < m {
            m = v;
        }
    }
    m
}

#[inline]
unsafe fn simd_max_f64_inner(data: *const f64, len: usize) -> f64 {
    use wide::f64x4;
    if unsafe { contains_nan_f64(data, len) } {
        return f64::NAN;
    }
    if len < SIMD_SUM_THRESHOLD {
        let mut m = unsafe { *data };
        for i in 1..len {
            let v = unsafe { *data.add(i) };
            if v > m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = unsafe { load_f64x4(data, 0) };
    for i in 1..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        acc = acc.fast_max(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p > m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        if v > m {
            m = v;
        }
    }
    m
}

#[inline]
unsafe fn simd_sum_squares_f64_inner(data: *const f64, len: usize) -> f64 {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s = 0.0_f64;
        for i in 0..len {
            let v = unsafe { *data.add(i) };
            s += v * v;
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = f64x4::splat(0.0);
    for i in 0..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        acc += v * v;
    }
    let parts = acc.to_array();
    let mut s = parts[0] + parts[1] + parts[2] + parts[3];
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        s += v * v;
    }
    s
}

#[inline]
unsafe fn simd_scale_f64_inner(src: *const f64, dst: *mut f64, len: usize, factor: f64) {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *src.add(i) * factor };
        }
        return;
    }
    let chunks = len / 4;
    let splat = f64x4::splat(factor);
    for i in 0..chunks {
        let base = i * 4;
        let v = unsafe { load_f64x4(src, base) };
        let r = (v * splat).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *src.add(i) * factor };
    }
}

#[inline]
unsafe fn simd_add_scalar_f64_inner(src: *const f64, dst: *mut f64, len: usize, offset: f64) {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *src.add(i) + offset };
        }
        return;
    }
    let chunks = len / 4;
    let splat = f64x4::splat(offset);
    for i in 0..chunks {
        let base = i * 4;
        let v = unsafe { load_f64x4(src, base) };
        let r = (v + splat).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *src.add(i) + offset };
    }
}

#[inline]
unsafe fn simd_binary_add_f64_inner(
    a: *const f64,
    b: *const f64,
    dst: *mut f64,
    len: usize,
) {
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *a.add(i) + *b.add(i) };
        }
        return;
    }
    let chunks = len / 4;
    for i in 0..chunks {
        let base = i * 4;
        let va = unsafe { load_f64x4(a, base) };
        let vb = unsafe { load_f64x4(b, base) };
        let r = (va + vb).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *a.add(i) + *b.add(i) };
    }
}

#[inline]
unsafe fn simd_binary_mul_f64_inner(
    a: *const f64,
    b: *const f64,
    dst: *mut f64,
    len: usize,
) {
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *a.add(i) * *b.add(i) };
        }
        return;
    }
    let chunks = len / 4;
    for i in 0..chunks {
        let base = i * 4;
        let va = unsafe { load_f64x4(a, base) };
        let vb = unsafe { load_f64x4(b, base) };
        let r = (va * vb).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *a.add(i) * *b.add(i) };
    }
}

// ============================================================================
// Array FFI — i64
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_i64(capacity: u32) -> *mut TypedArray<i64> {
    TypedArray::<i64>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_i64(arr: *const TypedArray<i64>, index: i64) -> i64 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array i64 index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_i64(arr: *mut TypedArray<i64>, index: i64, val: i64) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_i64(arr: *mut TypedArray<i64>, val: i64) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_i64(arr: *const TypedArray<i64>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Array FFI — i32
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_i32(capacity: u32) -> *mut TypedArray<i32> {
    TypedArray::<i32>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_i32(arr: *const TypedArray<i32>, index: i64) -> i32 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array i32 index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_i32(arr: *mut TypedArray<i32>, index: i64, val: i32) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_i32(arr: *mut TypedArray<i32>, val: i32) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_i32(arr: *const TypedArray<i32>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Array FFI — bool (stored as u8 internally)
// ============================================================================
//
// Bool elements are stored as u8 (0 or 1) in the underlying TypedArray<u8>
// buffer. The Cranelift IR side uses i8 for bool slots (matching SlotKind::Bool
// → I8 in `cranelift_type_for_slot`), and the FFI translates u8 ↔ bool at the
// edges. This keeps the buffer compact (1 byte per element) and matches the
// JIT's native i8 width for bool locals.

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_bool(capacity: u32) -> *mut TypedArray<u8> {
    TypedArray::<u8>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_bool(arr: *const TypedArray<u8>, index: i64) -> u8 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array bool index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_bool(arr: *mut TypedArray<u8>, index: i64, val: u8) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_bool(arr: *mut TypedArray<u8>, val: u8) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_bool(arr: *const TypedArray<u8>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Struct field access FFI
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_f64(ptr: *const u8, offset: u32) -> f64 {
    unsafe { (ptr.add(offset as usize) as *const f64).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_i64(ptr: *const u8, offset: u32) -> i64 {
    unsafe { (ptr.add(offset as usize) as *const i64).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_i32(ptr: *const u8, offset: u32) -> i32 {
    unsafe { (ptr.add(offset as usize) as *const i32).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_ptr(ptr: *const u8, offset: u32) -> *const u8 {
    unsafe { (ptr.add(offset as usize) as *const *const u8).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_f64(ptr: *mut u8, offset: u32, val: f64) {
    unsafe {
        (ptr.add(offset as usize) as *mut f64).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_i64(ptr: *mut u8, offset: u32, val: i64) {
    unsafe {
        (ptr.add(offset as usize) as *mut i64).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_i32(ptr: *mut u8, offset: u32, val: i32) {
    unsafe {
        (ptr.add(offset as usize) as *mut i32).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_ptr(ptr: *mut u8, offset: u32, val: *const u8) {
    unsafe {
        (ptr.add(offset as usize) as *mut *const u8).write_unaligned(val);
    }
}

// ============================================================================
// Refcount FFI
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_retain(ptr: *const u8) {
    unsafe {
        let header = ptr as *const HeapHeader;
        (*header).retain();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_release(ptr: *const u8) {
    unsafe {
        let header = ptr as *const HeapHeader;
        if (*header).release() {
            // Refcount reached zero — deallocate.
            // For now, we only deallocate the struct itself.
            // Future: dispatch on kind for proper cleanup of nested resources.
            let kind = (*header).kind();
            let _ = kind; // TODO: dispatch cleanup based on kind
            std::alloc::dealloc(
                ptr as *mut u8,
                std::alloc::Layout::from_size_align(8, 8).unwrap(), // minimum — real size TBD
            );
        }
    }
}

// ============================================================================
// Struct allocation FFI
// ============================================================================

/// Allocate a v2 struct of the given total size (including header).
/// Initializes the HeapHeader with refcount=1 and the given kind.
/// Returns a pointer to the start of the struct (i.e., to the HeapHeader).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_alloc_struct(size: u32, kind: u16) -> *mut u8 {
    let align = 8; // all v2 structs are 8-byte aligned
    let layout = std::alloc::Layout::from_size_align(size as usize, align).unwrap();
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    // Initialize the header
    unsafe {
        let header = ptr as *mut HeapHeader;
        std::ptr::write(header, HeapHeader::new(kind));
    }
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::heap_header::HEAP_KIND_V2_STRUCT;

    // ── Phase C.3 SIMD sum tests ─────────────────────────────────────────

    #[test]
    fn test_simd_sum_f64_small_scalar_path() {
        // Below SIMD_SUM_THRESHOLD — exercises scalar accumulation.
        let arr = jit_v2_array_new_f64(8);
        for i in 0..8 {
            jit_v2_array_push_f64(arr, (i + 1) as f64); // 1..=8
        }
        let sum = jit_v2_array_sum_f64(arr);
        assert!((sum - 36.0).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_large_vector_path() {
        // Above SIMD_SUM_THRESHOLD with non-multiple-of-4 length (exercises
        // both the f64x4 loop and the scalar remainder).
        let arr = jit_v2_array_new_f64(128);
        let mut expected = 0.0_f64;
        for i in 0..101 {
            let v = i as f64 * 0.5;
            jit_v2_array_push_f64(arr, v);
            expected += v;
        }
        let sum = jit_v2_array_sum_f64(arr);
        assert!(
            (sum - expected).abs() < 1e-9,
            "sum={} expected={}",
            sum,
            expected
        );
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_empty() {
        let arr = jit_v2_array_new_f64(0);
        let sum = jit_v2_array_sum_f64(arr);
        assert_eq!(sum, 0.0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_null_safe() {
        assert_eq!(jit_v2_array_sum_f64(std::ptr::null()), 0.0);
    }

    #[test]
    fn test_simd_sum_i64_small_scalar_path() {
        let arr = jit_v2_array_new_i64(16);
        for i in 0..10 {
            jit_v2_array_push_i64(arr, (i + 1) as i64);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, 55);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_i64_large_vector_path() {
        let arr = jit_v2_array_new_i64(128);
        let mut expected: i64 = 0;
        for i in 0..103 {
            let v = i as i64;
            jit_v2_array_push_i64(arr, v);
            expected = expected.wrapping_add(v);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, expected);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_i64_wrapping_overflow() {
        // Two i64::MAX values should wrap without panicking. Padded to 16
        // elements so we go down the SIMD path that also uses wrapping adds.
        let arr = jit_v2_array_new_i64(16);
        jit_v2_array_push_i64(arr, i64::MAX);
        jit_v2_array_push_i64(arr, 1);
        for _ in 2..16 {
            jit_v2_array_push_i64(arr, 0);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, i64::MAX.wrapping_add(1));
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_f64_roundtrip() {
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        jit_v2_array_push_f64(arr, 2.5);
        jit_v2_array_push_f64(arr, 3.14);
        assert_eq!(jit_v2_array_len_f64(arr), 3);
        assert!((jit_v2_array_get_f64(arr, 0) - 1.0).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 1) - 2.5).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 2) - 3.14).abs() < f64::EPSILON);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_i64_roundtrip() {
        let arr = jit_v2_array_new_i64(4);
        jit_v2_array_push_i64(arr, 42);
        jit_v2_array_push_i64(arr, -100);
        assert_eq!(jit_v2_array_len_i64(arr), 2);
        assert_eq!(jit_v2_array_get_i64(arr, 0), 42);
        assert_eq!(jit_v2_array_get_i64(arr, 1), -100);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_i32_roundtrip() {
        let arr = jit_v2_array_new_i32(4);
        jit_v2_array_push_i32(arr, 7);
        jit_v2_array_push_i32(arr, -3);
        assert_eq!(jit_v2_array_len_i32(arr), 2);
        assert_eq!(jit_v2_array_get_i32(arr, 0), 7);
        assert_eq!(jit_v2_array_get_i32(arr, 1), -3);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_bool_roundtrip() {
        // Bool elements are stored as u8 internally (0 = false, 1 = true).
        let arr = jit_v2_array_new_bool(4);
        jit_v2_array_push_bool(arr, 1);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_push_bool(arr, 1);
        assert_eq!(jit_v2_array_len_bool(arr), 3);
        assert_eq!(jit_v2_array_get_bool(arr, 0), 1);
        assert_eq!(jit_v2_array_get_bool(arr, 1), 0);
        assert_eq!(jit_v2_array_get_bool(arr, 2), 1);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_set_bool() {
        let arr = jit_v2_array_new_bool(4);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_set_bool(arr, 0, 1);
        assert_eq!(jit_v2_array_get_bool(arr, 0), 1);
        assert_eq!(jit_v2_array_get_bool(arr, 1), 0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_set_f64() {
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        jit_v2_array_push_f64(arr, 2.0);
        jit_v2_array_set_f64(arr, 0, 99.0);
        assert!((jit_v2_array_get_f64(arr, 0) - 99.0).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 1) - 2.0).abs() < f64::EPSILON);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_get_oob_returns_none_via_typed_array() {
        // Can't use #[should_panic] on extern "C" functions (UB).
        // Instead, test bounds via the underlying TypedArray::get which returns None.
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        unsafe {
            assert_eq!(TypedArray::get(arr, 5), None);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_field_load_store_f64() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_f64(ptr, 8, 3.14);
        let val = jit_v2_field_load_f64(ptr, 8);
        assert!((val - 3.14).abs() < f64::EPSILON);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap()) };
    }

    #[test]
    fn test_field_load_store_i64() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_i64(ptr, 8, -42);
        assert_eq!(jit_v2_field_load_i64(ptr, 8), -42);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap()) };
    }

    #[test]
    fn test_field_load_store_i32() {
        let ptr = jit_v2_alloc_struct(16, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_i32(ptr, 8, 999);
        assert_eq!(jit_v2_field_load_i32(ptr, 8), 999);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(16, 8).unwrap()) };
    }

    #[test]
    fn test_alloc_struct_initializes_header() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        unsafe {
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.kind(), HEAP_KIND_V2_STRUCT);
            assert_eq!(header.get_refcount(), 1);
            std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap());
        }
    }

    // ── SIMD min/max/mean/sum-of-squares tests ────────────────────────────

    fn fill_f64(arr: *mut TypedArray<f64>, vals: &[f64]) {
        for &v in vals {
            jit_v2_array_push_f64(arr, v);
        }
    }

    #[test]
    fn test_simd_min_f64_small_scalar_path() {
        let arr = jit_v2_array_new_f64(8);
        fill_f64(arr, &[3.0, 1.5, 4.0, -2.0, 0.5, 7.0, -9.0, 2.0]);
        let m = jit_v2_array_min_f64(arr);
        assert_eq!(m, -9.0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_min_f64_large_vector_path() {
        let arr = jit_v2_array_new_f64(64);
        let mut expected = f64::INFINITY;
        for i in 0..33 {
            // non-multiple of 4 for remainder coverage
            let v = (17 - i) as f64 * 0.25;
            jit_v2_array_push_f64(arr, v);
            if v < expected {
                expected = v;
            }
        }
        let m = jit_v2_array_min_f64(arr);
        assert!((m - expected).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_min_f64_empty_and_null() {
        let arr = jit_v2_array_new_f64(0);
        assert!(jit_v2_array_min_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
        assert!(jit_v2_array_min_f64(std::ptr::null()).is_nan());
    }

    #[test]
    fn test_simd_min_f64_single_element() {
        let arr = jit_v2_array_new_f64(1);
        jit_v2_array_push_f64(arr, 42.5);
        assert_eq!(jit_v2_array_min_f64(arr), 42.5);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_min_f64_nan_propagates() {
        // Scalar path
        let arr = jit_v2_array_new_f64(4);
        fill_f64(arr, &[1.0, 2.0, f64::NAN, 4.0]);
        assert!(jit_v2_array_min_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
        // SIMD path (>= 16 elements)
        let arr = jit_v2_array_new_f64(20);
        let mut v = vec![1.0_f64; 20];
        v[7] = f64::NAN;
        fill_f64(arr, &v);
        assert!(jit_v2_array_min_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_max_f64_small_scalar_path() {
        let arr = jit_v2_array_new_f64(8);
        fill_f64(arr, &[3.0, 1.5, 4.0, -2.0, 0.5, 7.0, -9.0, 2.0]);
        let m = jit_v2_array_max_f64(arr);
        assert_eq!(m, 7.0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_max_f64_large_vector_path() {
        let arr = jit_v2_array_new_f64(64);
        let mut expected = f64::NEG_INFINITY;
        for i in 0..37 {
            let v = (i as f64 * 1.3) - 5.0;
            jit_v2_array_push_f64(arr, v);
            if v > expected {
                expected = v;
            }
        }
        let m = jit_v2_array_max_f64(arr);
        assert!((m - expected).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_max_f64_nan_propagates() {
        let arr = jit_v2_array_new_f64(20);
        let mut v = vec![1.0_f64; 20];
        v[3] = f64::NAN;
        fill_f64(arr, &v);
        assert!(jit_v2_array_max_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_mean_f64_small() {
        let arr = jit_v2_array_new_f64(4);
        fill_f64(arr, &[1.0, 2.0, 3.0, 4.0]);
        let m = jit_v2_array_mean_f64(arr);
        assert!((m - 2.5).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_mean_f64_large() {
        let arr = jit_v2_array_new_f64(64);
        let mut total = 0.0_f64;
        for i in 0..50 {
            let v = i as f64 + 0.5;
            jit_v2_array_push_f64(arr, v);
            total += v;
        }
        let expected = total / 50.0;
        let m = jit_v2_array_mean_f64(arr);
        assert!((m - expected).abs() < 1e-9, "mean={} expected={}", m, expected);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_mean_f64_empty_is_nan() {
        let arr = jit_v2_array_new_f64(0);
        assert!(jit_v2_array_mean_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
        assert!(jit_v2_array_mean_f64(std::ptr::null()).is_nan());
    }

    #[test]
    fn test_simd_sum_squares_f64_small() {
        let arr = jit_v2_array_new_f64(4);
        fill_f64(arr, &[1.0, 2.0, 3.0, 4.0]);
        // 1 + 4 + 9 + 16 = 30
        let s = jit_v2_array_sum_squares_f64(arr);
        assert!((s - 30.0).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_squares_f64_large() {
        let arr = jit_v2_array_new_f64(64);
        let mut expected = 0.0_f64;
        for i in 0..50 {
            let v = (i as f64 - 25.0) * 0.5;
            jit_v2_array_push_f64(arr, v);
            expected += v * v;
        }
        let s = jit_v2_array_sum_squares_f64(arr);
        assert!((s - expected).abs() < 1e-8);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_squares_f64_empty() {
        let arr = jit_v2_array_new_f64(0);
        assert_eq!(jit_v2_array_sum_squares_f64(arr), 0.0);
        unsafe { TypedArray::drop_array(arr) };
        assert_eq!(jit_v2_array_sum_squares_f64(std::ptr::null()), 0.0);
    }

    // ── SIMD allocating transforms ────────────────────────────────────────

    fn collect_f64(arr: *const TypedArray<f64>) -> Vec<f64> {
        let len = unsafe { (*arr).len } as usize;
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            out.push(unsafe { TypedArray::<f64>::get_unchecked(arr, i as u32) });
        }
        out
    }

    #[test]
    fn test_simd_scale_f64_small() {
        let a = jit_v2_array_new_f64(4);
        fill_f64(a, &[1.0, 2.0, 3.0, 4.0]);
        let out = jit_v2_array_scale_f64(a, 2.5);
        assert_eq!(collect_f64(out), vec![2.5, 5.0, 7.5, 10.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_scale_f64_large() {
        let a = jit_v2_array_new_f64(32);
        for i in 0..20 {
            jit_v2_array_push_f64(a, i as f64);
        }
        let out = jit_v2_array_scale_f64(a, -0.5);
        let got = collect_f64(out);
        for i in 0..20 {
            assert!((got[i] - (i as f64 * -0.5)).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_scale_f64_empty() {
        let a = jit_v2_array_new_f64(0);
        let out = jit_v2_array_scale_f64(a, 3.0);
        assert_eq!(unsafe { (*out).len }, 0);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_scalar_f64_small() {
        let a = jit_v2_array_new_f64(3);
        fill_f64(a, &[1.0, 2.0, 3.0]);
        let out = jit_v2_array_add_scalar_f64(a, 10.0);
        assert_eq!(collect_f64(out), vec![11.0, 12.0, 13.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_scalar_f64_large() {
        let a = jit_v2_array_new_f64(32);
        for i in 0..25 {
            jit_v2_array_push_f64(a, i as f64);
        }
        let out = jit_v2_array_add_scalar_f64(a, 100.0);
        let got = collect_f64(out);
        for i in 0..25 {
            assert!((got[i] - (i as f64 + 100.0)).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_small() {
        let a = jit_v2_array_new_f64(4);
        let b = jit_v2_array_new_f64(4);
        fill_f64(a, &[1.0, 2.0, 3.0, 4.0]);
        fill_f64(b, &[10.0, 20.0, 30.0, 40.0]);
        let out = jit_v2_array_add_f64(a, b);
        assert_eq!(collect_f64(out), vec![11.0, 22.0, 33.0, 44.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_large() {
        let a = jit_v2_array_new_f64(32);
        let b = jit_v2_array_new_f64(32);
        for i in 0..23 {
            jit_v2_array_push_f64(a, i as f64);
            jit_v2_array_push_f64(b, (i * 2) as f64);
        }
        let out = jit_v2_array_add_f64(a, b);
        let got = collect_f64(out);
        for i in 0..23 {
            assert!((got[i] - (i as f64 + (i * 2) as f64)).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_mul_f64_small() {
        let a = jit_v2_array_new_f64(4);
        let b = jit_v2_array_new_f64(4);
        fill_f64(a, &[1.0, 2.0, 3.0, 4.0]);
        fill_f64(b, &[10.0, 20.0, 30.0, 40.0]);
        let out = jit_v2_array_mul_f64(a, b);
        assert_eq!(collect_f64(out), vec![10.0, 40.0, 90.0, 160.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_mul_f64_large() {
        let a = jit_v2_array_new_f64(32);
        let b = jit_v2_array_new_f64(32);
        for i in 0..20 {
            jit_v2_array_push_f64(a, (i as f64 + 1.0) * 0.5);
            jit_v2_array_push_f64(b, (i as f64 + 1.0) * 2.0);
        }
        let out = jit_v2_array_mul_f64(a, b);
        let got = collect_f64(out);
        for i in 0..20 {
            let expected = ((i as f64 + 1.0) * 0.5) * ((i as f64 + 1.0) * 2.0);
            assert!((got[i] - expected).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_empty() {
        let a = jit_v2_array_new_f64(0);
        let b = jit_v2_array_new_f64(0);
        let out = jit_v2_array_add_f64(a, b);
        assert_eq!(unsafe { (*out).len }, 0);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_null_inputs() {
        assert!(jit_v2_array_add_f64(std::ptr::null(), std::ptr::null()).is_null());
    }

    #[test]
    fn test_retain_increments_refcount() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        unsafe {
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.get_refcount(), 1);
            jit_v2_retain(ptr);
            assert_eq!(header.get_refcount(), 2);
            jit_v2_retain(ptr);
            assert_eq!(header.get_refcount(), 3);
            // Clean up manually (don't use jit_v2_release which would dealloc wrong size)
            std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap());
        }
    }
}
