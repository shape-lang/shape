//! Method handlers for `Instant` values (std::time::Instant).
//!
//! Methods: elapsed, elapsed_ms, elapsed_us, elapsed_ns, to_string

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};

/// Extract the receiver Instant from args[0].
fn recv_instant(args: &[ValueWord]) -> Result<&std::time::Instant, VMError> {
    args.first()
        .and_then(|a| a.as_instant())
        .ok_or_else(|| VMError::TypeError {
            expected: "instant",
            got: args.first().map_or("missing", |a| a.type_name()),
        })
}

/// .elapsed() -> number (seconds as f64)
pub fn handle_elapsed(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let instant = recv_instant(&args)?;
    let secs = instant.elapsed().as_secs_f64();
    vm.push_vw(ValueWord::from_f64(secs))?;
    Ok(())
}

/// .elapsed_ms() -> number (milliseconds as f64)
pub fn handle_elapsed_ms(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let instant = recv_instant(&args)?;
    let ms = instant.elapsed().as_secs_f64() * 1000.0;
    vm.push_vw(ValueWord::from_f64(ms))?;
    Ok(())
}

/// .elapsed_us() -> number (microseconds as f64)
pub fn handle_elapsed_us(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let instant = recv_instant(&args)?;
    let us = instant.elapsed().as_secs_f64() * 1_000_000.0;
    vm.push_vw(ValueWord::from_f64(us))?;
    Ok(())
}

/// .elapsed_ns() -> int (nanoseconds)
pub fn handle_elapsed_ns(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let instant = recv_instant(&args)?;
    let ns = instant.elapsed().as_nanos() as i64;
    vm.push_vw(ValueWord::from_i64(ns))?;
    Ok(())
}

/// .duration_since(other: Instant) -> number (milliseconds as f64)
pub fn handle_duration_since(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let this = recv_instant(&args)?;
    let other = args
        .get(1)
        .and_then(|a| a.as_instant())
        .ok_or_else(|| VMError::TypeError {
            expected: "instant",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })?;
    let ms = this.duration_since(*other).as_secs_f64() * 1000.0;
    vm.push_vw(ValueWord::from_f64(ms))?;
    Ok(())
}

/// .to_string() -> string representation
pub fn handle_to_string(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let instant = recv_instant(&args)?;
    let elapsed = instant.elapsed();
    let s = format!("Instant(elapsed: {:.6}s)", elapsed.as_secs_f64());
    vm.push_vw(ValueWord::from_string(std::sync::Arc::new(s)))?;
    Ok(())
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
        let args = vec![ValueWord::from_instant(instant)];
        handle_elapsed(&mut vm, args, None).unwrap();
        let result = vm.pop_vw().unwrap();
        let secs = result.as_f64().unwrap();
        assert!(secs >= 0.0);
        assert!(secs < 1.0); // Should be very fast
    }

    #[test]
    fn test_elapsed_ms_returns_milliseconds() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let args = vec![ValueWord::from_instant(instant)];
        handle_elapsed_ms(&mut vm, args, None).unwrap();
        let result = vm.pop_vw().unwrap();
        let ms = result.as_f64().unwrap();
        assert!(ms >= 0.0);
        assert!(ms < 1000.0);
    }

    #[test]
    fn test_elapsed_us_returns_microseconds() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let args = vec![ValueWord::from_instant(instant)];
        handle_elapsed_us(&mut vm, args, None).unwrap();
        let result = vm.pop_vw().unwrap();
        let us = result.as_f64().unwrap();
        assert!(us >= 0.0);
        assert!(us < 1_000_000.0);
    }

    #[test]
    fn test_elapsed_ns_returns_int() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let args = vec![ValueWord::from_instant(instant)];
        handle_elapsed_ns(&mut vm, args, None).unwrap();
        let result = vm.pop_vw().unwrap();
        // Should be a number (i48 stored as f64)
        let ns = result.as_number_coerce().unwrap();
        assert!(ns >= 0.0);
    }

    #[test]
    fn test_to_string_format() {
        let mut vm = create_test_vm();
        let instant = std::time::Instant::now();
        let args = vec![ValueWord::from_instant(instant)];
        handle_to_string(&mut vm, args, None).unwrap();
        let result = vm.pop_vw().unwrap();
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
        let args = vec![
            ValueWord::from_instant(later),
            ValueWord::from_instant(earlier),
        ];
        handle_duration_since(&mut vm, args, None).unwrap();
        let result = vm.pop_vw().unwrap();
        let ms = result.as_f64().unwrap();
        assert!(ms >= 0.0);
    }
}
