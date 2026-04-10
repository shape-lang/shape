//! Method handlers for `Instant` values (std::time::Instant) — MethodFnV2.
//!
//! Methods: elapsed, elapsed_ms, elapsed_us, elapsed_ns, duration_since, to_string

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;

/// Reconstruct a `ValueWord` from raw bits WITHOUT taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Extract the receiver Instant from args[0].
fn recv_instant_v2(args: &[u64]) -> Result<&std::time::Instant, VMError> {
    let vw = borrow_vw(args[0]);
    let ptr = vw
        .as_instant()
        .ok_or_else(|| VMError::TypeError {
            expected: "instant",
            got: vw.type_name(),
        })? as *const std::time::Instant;
    Ok(unsafe { &*ptr })
}

/// .elapsed() -> number (seconds as f64)
pub fn v2_elapsed(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let instant = recv_instant_v2(args)?;
    let secs = instant.elapsed().as_secs_f64();
    Ok(ValueWord::from_f64(secs).into_raw_bits())
}

/// .elapsed_ms() -> number (milliseconds as f64)
pub fn v2_elapsed_ms(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let instant = recv_instant_v2(args)?;
    let ms = instant.elapsed().as_secs_f64() * 1000.0;
    Ok(ValueWord::from_f64(ms).into_raw_bits())
}

/// .elapsed_us() -> number (microseconds as f64)
pub fn v2_elapsed_us(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let instant = recv_instant_v2(args)?;
    let us = instant.elapsed().as_secs_f64() * 1_000_000.0;
    Ok(ValueWord::from_f64(us).into_raw_bits())
}

/// .elapsed_ns() -> int (nanoseconds)
pub fn v2_elapsed_ns(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let instant = recv_instant_v2(args)?;
    let ns = instant.elapsed().as_nanos() as i64;
    Ok(ValueWord::from_i64(ns).into_raw_bits())
}

/// .duration_since(other: Instant) -> number (milliseconds as f64)
pub fn v2_duration_since(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let this = recv_instant_v2(args)?;
    let vw1 = borrow_vw(args[1]);
    let other = vw1.as_instant().ok_or_else(|| VMError::TypeError {
        expected: "instant",
        got: vw1.type_name(),
    })?;
    let ms = this.duration_since(*other).as_secs_f64() * 1000.0;
    Ok(ValueWord::from_f64(ms).into_raw_bits())
}

/// .to_string() -> string representation
pub fn v2_to_string(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let instant = recv_instant_v2(args)?;
    let elapsed = instant.elapsed();
    let s = format!("Instant(elapsed: {:.6}s)", elapsed.as_secs_f64());
    Ok(ValueWord::from_string(std::sync::Arc::new(s)).into_raw_bits())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};

    fn create_test_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    #[test]
    fn test_elapsed_returns_number() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let mut raw_args = [ValueWord::from_instant(instant).into_raw_bits()];
        let result_raw = v2_elapsed(&mut vm, &mut raw_args, None).unwrap();
        let result = ValueWord::from_raw_bits(result_raw);
        let secs = result.as_f64().unwrap();
        assert!(secs >= 0.0);
        assert!(secs < 1.0); // Should be very fast
    }

    #[test]
    fn test_elapsed_ms_returns_milliseconds() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let mut raw_args = [ValueWord::from_instant(instant).into_raw_bits()];
        let result_raw = v2_elapsed_ms(&mut vm, &mut raw_args, None).unwrap();
        let result = ValueWord::from_raw_bits(result_raw);
        let ms = result.as_f64().unwrap();
        assert!(ms >= 0.0);
        assert!(ms < 1000.0);
    }

    #[test]
    fn test_elapsed_us_returns_microseconds() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let mut raw_args = [ValueWord::from_instant(instant).into_raw_bits()];
        let result_raw = v2_elapsed_us(&mut vm, &mut raw_args, None).unwrap();
        let result = ValueWord::from_raw_bits(result_raw);
        let us = result.as_f64().unwrap();
        assert!(us >= 0.0);
        assert!(us < 1_000_000.0);
    }

    #[test]
    fn test_elapsed_ns_returns_int() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let mut raw_args = [ValueWord::from_instant(instant).into_raw_bits()];
        let result_raw = v2_elapsed_ns(&mut vm, &mut raw_args, None).unwrap();
        let result = ValueWord::from_raw_bits(result_raw);
        // Should be a number (i48 stored as f64)
        let ns = result.as_number_coerce().unwrap();
        assert!(ns >= 0.0);
    }

    #[test]
    fn test_to_string_format() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let mut raw_args = [ValueWord::from_instant(instant).into_raw_bits()];
        let result_raw = v2_to_string(&mut vm, &mut raw_args, None).unwrap();
        let result = ValueWord::from_raw_bits(result_raw);
        let s = result.as_str().unwrap();
        assert!(s.starts_with("Instant(elapsed:"));
        assert!(s.ends_with("s)"));
    }

    #[test]
    fn test_duration_since() {
        let mut vm = create_test_vm();
        let earlier = std::time::Instant::now();
        // Small busy loop to ensure measurable difference
        std::hint::black_box(0u64.wrapping_add(1));
        let later = std::time::Instant::now();
        let mut raw_args = [
            ValueWord::from_instant(later).into_raw_bits(),
            ValueWord::from_instant(earlier).into_raw_bits(),
        ];
        let result_raw = v2_duration_since(&mut vm, &mut raw_args, None).unwrap();
        let result = ValueWord::from_raw_bits(result_raw);
        let ms = result.as_f64().unwrap();
        assert!(ms >= 0.0);
    }
}
