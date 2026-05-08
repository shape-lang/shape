//! Native `time` module for precision timing.
//!
//! Exports: time.now(), time.sleep(ms), time.benchmark(fn, iterations)
//!
//! Phase 1.B (ADR-006 §2.7.4): the variadic `register_typed_function` /
//! `register_typed_async_function` helpers are re-introduced at the
//! [`KindedSlot`] shape (see `marshal.rs`). Bodies are migrated to use
//! the current `TypedReturn` / `ConcreteReturn` taxonomy in place of
//! the deleted `TypedReturn::Instant` / `TypedReturn::Unit` /
//! `TypedReturn::F64` / `TypedReturn::TypedObject(_, TypedReturn::*)`
//! shapes.

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_async_function,
    register_typed_function,
};
use shape_value::KindedSlot;

/// Read a `KindedSlot` argument as `f64` for time-module APIs.
///
/// Phase 1.B variadic shim: the bodies receive `KindedSlot`s whose
/// per-position kind is the body's contract (Phase 2c lands proper
/// per-position kind threading from the schema). Until then, time-
/// module bodies treat each numeric arg as `f64` raw bits — the
/// pre-bulldozer `as_number_coerce()` shape.
fn slot_as_f64(slot: &KindedSlot) -> Option<f64> {
    Some(slot.slot().as_f64())
}

/// Create the `time` module with precision timing functions.
pub fn create_time_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::time");
    module.description = "Precision timing utilities".to_string();

    // time.now() -> Instant
    register_typed_function(
        &mut module,
        "now",
        "Return the current monotonic instant for measuring elapsed time",
        vec![],
        ConcreteType::Instant,
        |_args, _ctx| {
            Ok(TypedReturn::Concrete(ConcreteReturn::Instant(
                std::time::Instant::now(),
            )))
        },
    );

    // time.sleep(ms: number) -> unit (async via tokio).
    register_typed_async_function(
        &mut module,
        "sleep",
        "Sleep for the specified number of milliseconds (async)",
        vec![ModuleParam {
            name: "ms".to_string(),
            type_name: "number".to_string(),
            required: true,
            description: "Duration in milliseconds".to_string(),
            ..Default::default()
        }],
        ConcreteType::Unit,
        |args: Vec<KindedSlot>| async move {
            let ms = args.first().and_then(slot_as_f64).ok_or_else(|| {
                "time.sleep() requires a number argument (milliseconds)".to_string()
            })?;
            if ms < 0.0 {
                return Err("time.sleep() duration must be non-negative".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // time.sleep_sync(ms: number) -> unit
    register_typed_function(
        &mut module,
        "sleep_sync",
        "Sleep for the specified number of milliseconds (blocking)",
        vec![ModuleParam {
            name: "ms".to_string(),
            type_name: "number".to_string(),
            required: true,
            description: "Duration in milliseconds".to_string(),
            ..Default::default()
        }],
        ConcreteType::Unit,
        |args, _ctx| {
            let ms = args.first().and_then(slot_as_f64).ok_or_else(|| {
                "time.sleep_sync() requires a number argument (milliseconds)".to_string()
            })?;
            if ms < 0.0 {
                return Err("time.sleep_sync() duration must be non-negative".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // time.benchmark(fn, iterations?) -> object { elapsed_ms, iterations, avg_ms }
    register_typed_function(
        &mut module,
        "benchmark",
        "Benchmark a function over N iterations, returning timing statistics",
        vec![
            ModuleParam {
                name: "fn".to_string(),
                type_name: "function".to_string(),
                required: true,
                description: "Function to benchmark".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "iterations".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Number of iterations (default: 1000)".to_string(),
                default_snippet: Some("1000".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::TypedObject,
        |args, _ctx| {
            let _func = args
                .first()
                .ok_or_else(|| "time.benchmark() requires a function argument".to_string())?;

            let iterations = args.get(1).and_then(slot_as_f64).unwrap_or(1000.0) as u64;

            if iterations == 0 {
                return Err("time.benchmark() iterations must be > 0".to_string());
            }

            // Module-level benchmark just measures wrapper overhead. The
            // VM-level benchmark builtin executes the callable in a loop;
            // this body cannot call back into the VM.
            let start = std::time::Instant::now();
            let elapsed = start.elapsed();
            let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

            Ok(TypedReturn::TypedObject(vec![
                ("elapsed_ms".to_string(), ConcreteReturn::F64(elapsed_ms)),
                (
                    "iterations".to_string(),
                    ConcreteReturn::F64(iterations as f64),
                ),
                (
                    "avg_ms".to_string(),
                    ConcreteReturn::F64(elapsed_ms / iterations as f64),
                ),
            ]))
        },
    );

    // time.stopwatch() -> Instant (alias for now())
    register_typed_function(
        &mut module,
        "stopwatch",
        "Start a stopwatch (returns an Instant). Call .elapsed() to read.",
        vec![],
        ConcreteType::Instant,
        |_args, _ctx| {
            Ok(TypedReturn::Concrete(ConcreteReturn::Instant(
                std::time::Instant::now(),
            )))
        },
    );

    // time.millis() -> number (current epoch millis, for wall-clock timestamps)
    register_typed_function(
        &mut module,
        "millis",
        "Return current wall-clock time as milliseconds since Unix epoch",
        vec![],
        ConcreteType::Number,
        |_args, _ctx| {
            let now = std::time::SystemTime::now();
            let since_epoch = now
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| format!("SystemTime error: {}", e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(
                since_epoch.as_millis() as f64,
            )))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_module_creation() {
        let module = create_time_module();
        assert_eq!(module.name, "std::core::time");
        assert!(module.has_export("now"));
        assert!(module.has_export("sleep"));
        assert!(module.has_export("sleep_sync"));
        assert!(module.has_export("benchmark"));
        assert!(module.has_export("stopwatch"));
        assert!(module.has_export("millis"));
    }

    #[test]
    fn test_time_sleep_is_async() {
        let module = create_time_module();
        assert!(module.is_async("sleep"));
        assert!(!module.is_async("sleep_sync"));
    }

    #[test]
    fn test_time_schemas() {
        let module = create_time_module();
        let now_schema = module.get_schema("now").unwrap();
        assert_eq!(now_schema.return_type.as_deref(), Some("Instant"));

        let sleep_schema = module.get_schema("sleep").unwrap();
        assert_eq!(sleep_schema.params.len(), 1);
        assert_eq!(sleep_schema.params[0].name, "ms");

        let bench_schema = module.get_schema("benchmark").unwrap();
        assert_eq!(bench_schema.params.len(), 2);
        assert!(!bench_schema.params[1].required);
    }

    #[test]
    fn test_time_typed_registry_populated() {
        let module = create_time_module();
        let typed = module.typed_exports();
        assert!(typed.get("now").is_some());
        assert!(typed.get("sleep_sync").is_some());
        assert!(typed.get("benchmark").is_some());
        assert!(typed.get("stopwatch").is_some());
        assert!(typed.get("millis").is_some());
        // sleep is async — separate registry.
        assert!(typed.get_async("sleep").is_some());
        assert!(typed.get("sleep").is_none());
    }

    // Behavioural tests (`invoke_export(...)` / typed return bit-checks)
    // were deleted in Phase 2b alongside `module.invoke_export`. The
    // typed dispatch path lives in shape-vm now; behavioural coverage
    // returns when shape-vm Cluster #4 lands its kind-threaded slot
    // tests on top of the rebuilt typed module exports.
}
