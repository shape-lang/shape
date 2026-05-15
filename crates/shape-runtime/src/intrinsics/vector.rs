//! Vector intrinsics — element-wise SIMD operations on f64 / i64 typed arrays.
//!
//! Twelve typed-marshal entry points (`__intrinsic_vec_*`) are registered via
//! `register_typed_fn_N` into the module returned by
//! [`create_vector_intrinsics_module`]. Inputs are zero-copy
//! `Arc<Vec<f64>>` (f64) / `Arc<Vec<i64>>` (i64) per the
//! per-storage-variant body-type map in `docs/defections.md` 2026-05-07
//! zero-copy entry. Outputs project through `ConcreteReturn::ArrayF64(Vec<f64>)`
//! / `ConcreteReturn::ArrayI64(Vec<i64>)` (output owned-clone — full output-
//! zero-copy is deferred follow-on per the same entry's α-`ToSlot`-dead-at-
//! marshal-layer subsection).
//!
//! Q2-marshal-fold-light migration of the vector cluster (intrinsics-typed-CC
//! cluster, M-A scope per the entry's three-stage Q2 lifecycle subsection).
//! shape-vm dispatcher routing in `vector_intrinsics.rs:25-39` is part of
//! shape-vm cleanup workstream's natural scope; not migrated here.
//!
//! SIMD via the `wide` crate's `f64x4`. Kernel SIMD helpers
//! (`simd_vec_*_f64`, `simd_vec_*_i64`, `i64_slice_to_f64`) below are also
//! used by shape-vm's executor arithmetic dispatch; kept `pub` for that
//! consumer.

use crate::marshal::{register_typed_fn_1, register_typed_fn_2, register_typed_fn_3};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::aligned_vec::AlignedVec;
use std::sync::Arc;
use wide::f64x4;

const SIMD_THRESHOLD: usize = 16;

// ───────────────────── Module factory ─────────────────────

/// Create the vector intrinsics module with all 12 typed-marshal entry points.
pub fn create_vector_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::vector");
    module.description = "SIMD vector element-wise intrinsics".to_string();

    register_typed_fn_1::<_, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_abs",
        "Element-wise absolute value of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::ArrayNumber,
        |input, _ctx| {
            let result = unary_apply(input.as_slice(), |v| v.abs(), f64::abs);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_1::<_, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_sqrt",
        "Element-wise square root of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::ArrayNumber,
        |input, _ctx| {
            let result = unary_apply(input.as_slice(), |v| v.sqrt(), f64::sqrt);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    // ln/exp: `wide::f64x4` does not vectorize transcendentals; scalar fallback.
    register_typed_fn_1::<_, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_ln",
        "Element-wise natural logarithm of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::ArrayNumber,
        |input, _ctx| {
            let result: Vec<f64> = input.as_slice().iter().map(|x| x.ln()).collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_1::<_, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_exp",
        "Element-wise exponential of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::ArrayNumber,
        |input, _ctx| {
            let result: Vec<f64> = input.as_slice().iter().map(|x| x.exp()).collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<Vec<f64>>, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_add",
        "Element-wise addition of two Vec<number>",
        [("a", "Array<number>"), ("b", "Array<number>")],
        ConcreteType::ArrayNumber,
        |a, b, _ctx| {
            check_lens(a.len(), b.len(), "vec_add")?;
            let result = binary_apply(a.as_slice(), b.as_slice(), |va, vb| va + vb, |x, y| x + y);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<Vec<f64>>, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_sub",
        "Element-wise subtraction of two Vec<number>",
        [("a", "Array<number>"), ("b", "Array<number>")],
        ConcreteType::ArrayNumber,
        |a, b, _ctx| {
            check_lens(a.len(), b.len(), "vec_sub")?;
            let result = binary_apply(a.as_slice(), b.as_slice(), |va, vb| va - vb, |x, y| x - y);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<Vec<f64>>, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_mul",
        "Element-wise multiplication of two Vec<number>",
        [("a", "Array<number>"), ("b", "Array<number>")],
        ConcreteType::ArrayNumber,
        |a, b, _ctx| {
            check_lens(a.len(), b.len(), "vec_mul")?;
            let result = binary_apply(a.as_slice(), b.as_slice(), |va, vb| va * vb, |x, y| x * y);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<Vec<f64>>, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_div",
        "Element-wise division of two Vec<number>",
        [("a", "Array<number>"), ("b", "Array<number>")],
        ConcreteType::ArrayNumber,
        |a, b, _ctx| {
            check_lens(a.len(), b.len(), "vec_div")?;
            let result = binary_apply(a.as_slice(), b.as_slice(), |va, vb| va / vb, |x, y| x / y);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<Vec<f64>>, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_max",
        "Element-wise max of two Vec<number>",
        [("a", "Array<number>"), ("b", "Array<number>")],
        ConcreteType::ArrayNumber,
        |a, b, _ctx| {
            check_lens(a.len(), b.len(), "vec_max")?;
            let result = binary_apply(a.as_slice(), b.as_slice(), |va, vb| va.max(vb), f64::max);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<Vec<f64>>, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_min",
        "Element-wise min of two Vec<number>",
        [("a", "Array<number>"), ("b", "Array<number>")],
        ConcreteType::ArrayNumber,
        |a, b, _ctx| {
            check_lens(a.len(), b.len(), "vec_min")?;
            let result = binary_apply(a.as_slice(), b.as_slice(), |va, vb| va.min(vb), f64::min);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_3::<_, Arc<Vec<f64>>, Arc<Vec<f64>>, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_vec_select",
        "Element-wise select: cond[i] != 0 ? t[i] : f[i]",
        [
            ("cond", "Array<number>"),
            ("t", "Array<number>"),
            ("f", "Array<number>"),
        ],
        ConcreteType::ArrayNumber,
        |cond, t, f, _ctx| {
            let n = cond.len();
            if t.len() != n || f.len() != n {
                return Err(format!(
                    "vec_select: length mismatch cond={}, t={}, f={}",
                    n,
                    t.len(),
                    f.len()
                ));
            }
            let cond_data = cond.as_slice();
            let t_data = t.as_slice();
            let f_data = f.as_slice();
            let mut result = Vec::with_capacity(n);
            for i in 0..n {
                result.push(if cond_data[i] != 0.0 {
                    t_data[i]
                } else {
                    f_data[i]
                });
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<Vec<i64>>, Arc<Vec<i64>>>(
        &mut module,
        "__intrinsic_vec_add_i64",
        "Element-wise addition of two Vec<int>, overflow-checked",
        [("a", "Array<int>"), ("b", "Array<int>")],
        ConcreteType::ArrayInt,
        |a, b, _ctx| {
            check_lens(a.len(), b.len(), "vec_add_i64")?;
            simd_vec_add_i64(a.as_slice(), b.as_slice())
                .map(|r| TypedReturn::Concrete(ConcreteReturn::ArrayI64(r)))
                .map_err(|()| "Integer overflow in Vec<int> element-wise addition".to_string())
        },
    );

    module
}

// ───────────────────── helpers ─────────────────────

#[inline]
fn check_lens(a: usize, b: usize, name: &str) -> Result<(), String> {
    if a != b {
        Err(format!("Vector length mismatch in {}: {} vs {}", name, a, b))
    } else {
        Ok(())
    }
}

/// Apply a unary SIMD-or-scalar op element-wise; produces an owned `Vec<f64>`.
fn unary_apply(
    data: &[f64],
    simd_op: impl Fn(f64x4) -> f64x4,
    scalar_op: impl Fn(f64) -> f64,
) -> Vec<f64> {
    let len = data.len();
    let mut result = vec![0.0; len];
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f64x4::from(&data[idx..idx + 4]);
            let res = simd_op(v);
            result[idx..idx + 4].copy_from_slice(&res.to_array());
        }
        for i in (chunks * 4)..len {
            result[i] = scalar_op(data[i]);
        }
    } else {
        for i in 0..len {
            result[i] = scalar_op(data[i]);
        }
    }
    result
}

/// Apply a binary SIMD-or-scalar op element-wise; produces an owned `Vec<f64>`.
/// Caller must verify `a.len() == b.len()`.
fn binary_apply(
    a: &[f64],
    b: &[f64],
    simd_op: impl Fn(f64x4, f64x4) -> f64x4,
    scalar_op: impl Fn(f64, f64) -> f64,
) -> Vec<f64> {
    let len = a.len();
    let mut result = vec![0.0; len];
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = simd_op(va, vb);
            result[idx..idx + 4].copy_from_slice(&res.to_array());
        }
        for i in (chunks * 4)..len {
            result[i] = scalar_op(a[i], b[i]);
        }
    } else {
        for i in 0..len {
            result[i] = scalar_op(a[i], b[i]);
        }
    }
    result
}

// ───────────────────── Typed Vec<T> SIMD Kernels ─────────────────────
//
// These operate directly on raw slices from IntArray/FloatArray HeapValue
// variants, avoiding ValueWord materialization. Used by shape-vm's executor
// arithmetic dispatch (separate consumer from the typed-marshal entry points
// above).

/// Element-wise addition of two f64 slices using SIMD (f64x4).
/// Returns an `AlignedVec<f64>`. Panics if lengths differ (debug only).
pub fn simd_vec_add_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va + vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] + b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] + b[i]);
        }
    }
    result
}

/// Element-wise subtraction of two f64 slices using SIMD.
pub fn simd_vec_sub_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va - vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] - b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] - b[i]);
        }
    }
    result
}

/// Element-wise multiplication of two f64 slices using SIMD.
pub fn simd_vec_mul_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va * vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] * b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] * b[i]);
        }
    }
    result
}

/// Element-wise division of two f64 slices using SIMD.
pub fn simd_vec_div_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va / vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] / b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] / b[i]);
        }
    }
    result
}

/// Scalar broadcast: multiply each element by a scalar using SIMD.
pub fn simd_vec_scale_f64(a: &[f64], scalar: f64) -> AlignedVec<f64> {
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let s_vec = f64x4::splat(scalar);
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let res = va * s_vec;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] * scalar);
        }
    } else {
        for i in 0..len {
            result.push(a[i] * scalar);
        }
    }
    result
}

/// Element-wise addition of two i64 slices with checked overflow.
/// Returns Err if any element pair overflows.
pub fn simd_vec_add_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        match a[i].checked_add(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Element-wise subtraction of two i64 slices with checked overflow.
pub fn simd_vec_sub_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        match a[i].checked_sub(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Element-wise multiplication of two i64 slices with checked overflow.
pub fn simd_vec_mul_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        match a[i].checked_mul(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Element-wise division of two i64 slices with checked overflow.
pub fn simd_vec_div_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        if b[i] == 0 {
            return Err(());
        }
        match a[i].checked_div(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Coerce an i64 slice to f64 for mixed-type arithmetic.
pub fn i64_slice_to_f64(data: &[i64]) -> AlignedVec<f64> {
    let mut result = AlignedVec::with_capacity(data.len());
    for &v in data {
        result.push(v as f64);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Typed Vec<T> SIMD kernel tests (preserved across the
    // intrinsics-typed-CC migration; do not rely on ValueWord) =====

    #[test]
    fn test_simd_vec_add_f64_small() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 5.0, 6.0];
        let result = simd_vec_add_f64(&a, &b);
        assert_eq!(&*result, &[5.0, 7.0, 9.0]);
    }

    #[test]
    fn test_simd_vec_add_f64_large() {
        let a: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..20).map(|i| (i * 2) as f64).collect();
        let result = simd_vec_add_f64(&a, &b);
        for i in 0..20 {
            assert_eq!(result[i], (i * 3) as f64);
        }
    }

    #[test]
    fn test_simd_vec_sub_f64() {
        let a = [10.0, 20.0, 30.0];
        let b = [3.0, 5.0, 7.0];
        let result = simd_vec_sub_f64(&a, &b);
        assert_eq!(&*result, &[7.0, 15.0, 23.0]);
    }

    #[test]
    fn test_simd_vec_mul_f64() {
        let a = [2.0, 3.0, 4.0];
        let b = [5.0, 6.0, 7.0];
        let result = simd_vec_mul_f64(&a, &b);
        assert_eq!(&*result, &[10.0, 18.0, 28.0]);
    }

    #[test]
    fn test_simd_vec_div_f64() {
        let a = [10.0, 20.0, 30.0];
        let b = [2.0, 5.0, 6.0];
        let result = simd_vec_div_f64(&a, &b);
        assert_eq!(&*result, &[5.0, 4.0, 5.0]);
    }

    #[test]
    fn test_simd_vec_scale_f64() {
        let a = [1.0, 2.0, 3.0];
        let result = simd_vec_scale_f64(&a, 10.0);
        assert_eq!(&*result, &[10.0, 20.0, 30.0]);
    }

    #[test]
    fn test_simd_vec_add_i64_ok() {
        let a = [1i64, 2, 3];
        let b = [4i64, 5, 6];
        let result = simd_vec_add_i64(&a, &b).unwrap();
        assert_eq!(result, vec![5, 7, 9]);
    }

    #[test]
    fn test_simd_vec_add_i64_overflow() {
        let a = [i64::MAX];
        let b = [1i64];
        assert!(simd_vec_add_i64(&a, &b).is_err());
    }

    #[test]
    fn test_simd_vec_div_i64_zero() {
        let a = [10i64];
        let b = [0i64];
        assert!(simd_vec_div_i64(&a, &b).is_err());
    }

    #[test]
    fn test_i64_slice_to_f64() {
        let data = [1i64, -2, 100];
        let result = i64_slice_to_f64(&data);
        assert_eq!(&*result, &[1.0, -2.0, 100.0]);
    }

    // ===== Typed-marshal helper tests =====

    #[test]
    fn test_unary_apply_abs_simd() {
        let data: Vec<f64> = (0..20).map(|i| -(i as f64)).collect();
        let result = unary_apply(&data, |v| v.abs(), f64::abs);
        for i in 0..20 {
            assert_eq!(result[i], i as f64);
        }
    }

    #[test]
    fn test_binary_apply_add_simd() {
        let a: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..20).map(|i| (i * 2) as f64).collect();
        let result = binary_apply(&a, &b, |va, vb| va + vb, |x, y| x + y);
        for i in 0..20 {
            assert_eq!(result[i], (i * 3) as f64);
        }
    }
}
