//! Array-transform intrinsics — partial migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's partial-migration pattern (see
//! `docs/defections.md` 2026-05-07 intrinsics-typed-CC entry's partial-
//! migration subsection), 6 of 8 array-transform intrinsics migrate to
//! `register_typed_fn_N` typed entries via [`create_array_transforms_module`].
//! 2 polymorphic intrinsics (diff, cumsum) remain as legacy `IntrinsicFn`
//! bodies pending the M1-split sub-decision (per-element-type intrinsics
//! for polymorphic-return cases; cross-crate compiler change). diff
//! additionally needs a validity-aware return variant for its i64 fast
//! path (`option_i64_vec_to_nb` carries a validity bitmap; current
//! `ConcreteReturn::ArrayI64(Vec<i64>)` does not).

use crate::context::ExecutionContext;
use crate::marshal::{
    register_typed_fn_1, register_typed_fn_2, register_typed_fn_2_full, register_typed_fn_3,
};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::simd_i64;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_ast::error::{Result, ShapeError};
use shape_value::{AlignedTypedBuffer, ValueWord};
use std::sync::Arc;

use super::{
    extract_f64_array, i64_vec_to_nb_int_array, option_i64_vec_to_nb, try_extract_i64_slice,
};

// ───────────────────── Module factory (6 typed entries) ─────────────────────

/// Create the array-transforms intrinsics module with 6 typed-marshal entry
/// points. The 2 polymorphic intrinsics (diff, cumsum) remain as legacy
/// `IntrinsicFn` bodies in this module until their M1-split sub-decision lands.
pub fn create_array_transforms_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::array_transforms");
    module.description =
        "Array-transform intrinsics (typed entries; polymorphic-shape intrinsics stay as legacy bodies pending M1-split sub-decision)"
            .to_string();

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_series",
        "Identity column projection of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::ArrayNumber,
        |input, _ctx| {
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(
                input.as_slice().to_vec(),
            )))
        },
    );

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, i64>(
        &mut module,
        "__intrinsic_shift",
        "Shift a Vec<number> by N positions, padding with NaN",
        [("series", "Array<number>"), ("shift", "int")],
        ConcreteType::ArrayNumber,
        |series, shift, _ctx| {
            let data = series.as_slice();
            let mut result = Vec::with_capacity(data.len());
            if shift > 0 {
                let s = (shift as usize).min(data.len());
                for _ in 0..s {
                    result.push(f64::NAN);
                }
                result.extend_from_slice(&data[..data.len().saturating_sub(s)]);
            } else if shift < 0 {
                let s = ((-shift) as usize).min(data.len());
                result.extend_from_slice(&data[s..]);
                for _ in 0..s {
                    result.push(f64::NAN);
                }
            } else {
                result.extend_from_slice(data);
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2_full::<_, Arc<AlignedTypedBuffer>, i64>(
        &mut module,
        "__intrinsic_pct_change",
        "Percent change between consecutive (or period-spaced) Vec<number> elements",
        [
            ModuleParam {
                name: "series".to_string(),
                type_name: "Array<number>".to_string(),
                required: true,
                description: "Input series".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "period".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Period for change comparison (default 1)".to_string(),
                default_snippet: Some("1".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::ArrayNumber,
        |series, period, _ctx| {
            let data = series.as_slice();
            let period = period.max(0) as usize;
            let mut result = Vec::with_capacity(data.len());
            for i in 0..data.len() {
                if i < period {
                    result.push(f64::NAN);
                } else {
                    let prev = data[i - period];
                    if prev == 0.0 {
                        result.push(f64::NAN);
                    } else {
                        result.push((data[i] - prev) / prev);
                    }
                }
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, f64>(
        &mut module,
        "__intrinsic_fillna",
        "Replace NaN entries in a Vec<number> with a fill value",
        [("series", "Array<number>"), ("value", "number")],
        ConcreteType::ArrayNumber,
        |series, value, _ctx| {
            let result: Vec<f64> = series
                .as_slice()
                .iter()
                .map(|&v| if v.is_nan() { value } else { v })
                .collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_cumprod",
        "Cumulative product of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::ArrayNumber,
        |input, _ctx| {
            let data = input.as_slice();
            let mut result = Vec::with_capacity(data.len());
            let mut acc = 1.0;
            for &v in data {
                acc *= v;
                result.push(acc);
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_3::<_, Arc<AlignedTypedBuffer>, f64, f64>(
        &mut module,
        "__intrinsic_clip",
        "Clip Vec<number> elements to the [min, max] interval",
        [
            ("series", "Array<number>"),
            ("min", "number"),
            ("max", "number"),
        ],
        ConcreteType::ArrayNumber,
        |series, min, max, _ctx| {
            let result: Vec<f64> = series
                .as_slice()
                .iter()
                .map(|&v| v.max(min).min(max))
                .collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    module
}

// ───────────────────── Legacy bodies (2 polymorphic intrinsics) ─────────────────────

/// Intrinsic: Discrete difference of a series.
///
/// **Migration deferred** pending M1-split sub-decision (polymorphic
/// return: `Vec<int>` with validity bitmap for `Vec<int>` fast path vs
/// `Vec<f64>` with NaN sentinels for `Vec<number>`). The i64 fast path
/// uses `option_i64_vec_to_nb` (validity-bitmap-aware); a future M1-split
/// resolution for `diff` specifically needs a validity-aware return
/// variant (e.g. `ConcreteReturn::ArrayOptionI64`) since
/// `ConcreteReturn::ArrayI64(Vec<i64>)` does not carry validity. See
/// the intrinsics-typed-CC entry's sub-decision queue.
pub fn intrinsic_diff(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() || args.len() > 2 {
        return Err(ShapeError::RuntimeError {
            message: "diff() requires 1 or 2 arguments (series, [period])".to_string(),
            location: None,
        });
    }

    let period = if args.len() == 2 {
        super::extract_usize(&args[1], "diff() second argument")?
    } else {
        1
    };

    // i64 fast path
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        return Ok(option_i64_vec_to_nb(simd_i64::diff_i64(slice, period)));
    }

    let data = extract_f64_array(&args[0], "diff")?;

    let mut result = Vec::with_capacity(data.len());
    for i in 0..data.len() {
        if i < period {
            result.push(f64::NAN);
        } else {
            result.push(data[i] - data[i - period]);
        }
    }

    Ok(super::f64_vec_to_nb_array(result))
}

/// Intrinsic: Cumulative sum of a series.
///
/// **Migration deferred** pending M1-split sub-decision (polymorphic
/// input/return: `Vec<int>` for i64 fast path vs `Vec<number>` for f64
/// path). See the intrinsics-typed-CC entry's sub-decision queue.
pub fn intrinsic_cumsum(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "cumsum() requires 1 argument (series)".to_string(),
            location: None,
        });
    }

    // i64 fast path
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        return Ok(i64_vec_to_nb_int_array(simd_i64::cumsum_i64(slice)));
    }

    let data = extract_f64_array(&args[0], "cumsum")?;
    let mut result = Vec::with_capacity(data.len());
    let mut acc = 0.0;
    for &v in &data {
        acc += v;
        result.push(acc);
    }

    Ok(super::f64_vec_to_nb_array(result))
}
