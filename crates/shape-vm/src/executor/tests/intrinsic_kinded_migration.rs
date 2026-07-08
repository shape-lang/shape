//! Regression tests for the wave7 intrinsic KindedSlot-ABI migration
//! (ADR-006 §2.7.10 / §2.7.9).
//!
//! These intrinsics previously lived in the phase-1b-vm-wave-5d
//! surface-and-stop block and failed on invocation ("not migrated to
//! kinded carrier"). This module drives each through the language surface
//! (`eval_with_prelude` → real `KindedSlot` carrier path — no is_heap
//! probe, no raw-u64 slice, no Bool-default) and asserts a correct result:
//!
//!   * `math.median`            (`__intrinsic_median`)
//!   * `math.spread`            (`__intrinsic_max` + `__intrinsic_min`)
//!   * `rolling.rolling_mean`   (`__intrinsic_rolling_mean`)
//!   * `rolling.linear_recurrence` (`__intrinsic_linear_recurrence`)
//!   * `distributions.dist_uniform` (`__intrinsic_dist_uniform`)
//!   * DateTime property access (`dt.year`, … — kinded GetProp)

use super::test_utils::{eval_with_prelude, eval_typed_i64};
use crate::executor::v2_handlers::v2_array_detect::{
    V2ElemType, V2TypedArrayView, as_v2_typed_array, read_element,
};
use shape_value::{HeapKind, KindedSlot, NativeKind};

fn f64_array(slot: &KindedSlot) -> Vec<f64> {
    assert_eq!(
        slot.kind,
        NativeKind::Ptr(HeapKind::TypedArray),
        "expected a typed-array result, got kind {:?}",
        slot.kind
    );
    let view: V2TypedArrayView =
        as_v2_typed_array(slot.slot.raw(), slot.kind).expect("v2 typed-array carrier");
    assert_eq!(view.elem_type, V2ElemType::F64);
    (0..view.len)
        .map(|i| {
            let (bits, kind) = read_element(&view, i).expect("element");
            assert_eq!(kind, NativeKind::Float64);
            f64::from_bits(bits)
        })
        .collect()
}

// ── math.median ───────────────────────────────────────────────────────────

#[test]
fn median_odd_length_returns_middle() {
    let r = eval_with_prelude(
        r#"
        from std::core::math use { median }
        median([3.0, 7.0, 2.0, 9.0, 1.0])
        "#,
    );
    assert_eq!(r.kind, NativeKind::Float64);
    assert_eq!(r.as_f64(), Some(3.0));
}

#[test]
fn median_even_length_averages_central_pair() {
    let r = eval_with_prelude(
        r#"
        from std::core::math use { median }
        median([1.0, 2.0, 3.0, 4.0])
        "#,
    );
    assert_eq!(r.kind, NativeKind::Float64);
    assert_eq!(r.as_f64(), Some(2.5));
}

// ── math.spread (max - min) ─────────────────────────────────────────────────

#[test]
fn spread_exercises_intrinsic_max_and_min() {
    let r = eval_with_prelude(
        r#"
        from std::core::math use { spread }
        spread([3.0, 7.0, 2.0, 9.0, 1.0])
        "#,
    );
    assert_eq!(r.kind, NativeKind::Float64);
    // max(9) - min(1)
    assert_eq!(r.as_f64(), Some(8.0));
}

// ── rolling.rolling_mean ────────────────────────────────────────────────────

#[test]
fn rolling_mean_window_two() {
    let r = eval_with_prelude(
        r#"
        use std::core::utils::rolling
        rolling::rolling_mean([100.0, 101.0, 102.5, 101.8, 103.0, 104.2], 2)
        "#,
    );
    let out = f64_array(&r);
    assert_eq!(out.len(), 6);
    // Last window: (103.0 + 104.2) / 2 == 103.6
    assert!((out[5] - 103.6).abs() < 1e-9, "got {}", out[5]);
}

// ── rolling.linear_recurrence ───────────────────────────────────────────────

#[test]
fn linear_recurrence_first_order() {
    let r = eval_with_prelude(
        r#"
        use std::core::utils::rolling
        rolling::linear_recurrence([3.0, 7.0, 2.0], 0.5, 0.0)
        "#,
    );
    let out = f64_array(&r);
    // y0 = init*decay + x0 = 0*0.5 + 3 = 3.0
    // y1 = y0*0.5 + 7 = 8.5
    // y2 = y1*0.5 + 2 = 6.25
    assert_eq!(out, vec![3.0, 8.5, 6.25]);
}

// ── distributions.dist_uniform ──────────────────────────────────────────────

#[test]
fn dist_uniform_sample_in_range() {
    let r = eval_with_prelude(
        r#"
        use std::core::distributions
        let u = distributions::dist_uniform(1.0, 2.0)
        u >= 1.0 and u < 2.0
        "#,
    );
    assert_eq!(r.kind, NativeKind::Bool);
    assert_eq!(r.as_bool(), Some(true));
}

// ── DateTime property access (kinded GetProp) ───────────────────────────────

#[test]
fn datetime_property_year() {
    let v = eval_typed_i64(
        r#"
        let dt = DateTime.parse("2024-03-15T10:20:30+00:00")
        dt.year
        "#,
    );
    assert_eq!(v, 2024);
}

#[test]
fn datetime_property_components() {
    // Each component read as a standalone final expression. (The GetProp
    // path is runtime-dispatched — the value is a correct Int64 at runtime,
    // but its static type is `unknown`, so it cannot feed strict-typed
    // arithmetic; the book fences read each component via `print`.)
    let component = |field: &str| -> i64 {
        eval_typed_i64(&format!(
            "let dt = DateTime.parse(\"2024-03-15T10:20:30+00:00\")\ndt.{field}"
        ))
    };
    assert_eq!(component("month"), 3);
    assert_eq!(component("day"), 15);
    assert_eq!(component("hour"), 10);
    assert_eq!(component("minute"), 20);
    assert_eq!(component("second"), 30);
}
