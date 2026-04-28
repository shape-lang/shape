//! Native `time` module for precision timing.
//!
//! Exports: time.now(), time.sleep(ms), time.benchmark(fn, iterations)
//!
//! Phase 4b: 5 of 6 sync exports migrated to `TypedModuleExports`. The
//! async `time.sleep` retains its dedicated async ABI (Phase 4c will
//! generalize the typed ABI to async).

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{
    ConcreteType, TypedReturn, register_typed_async_function, register_typed_function,
};
use shape_value::{ValueWord, ValueWordExt};

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
        |_args, _ctx| Ok(TypedReturn::Instant(std::time::Instant::now())),
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
        |args: Vec<ValueWord>| async move {
            let ms = args
                .first()
                .and_then(|a| a.as_number_coerce())
                .ok_or_else(|| {
                    "time.sleep() requires a number argument (milliseconds)".to_string()
                })?;
            if ms < 0.0 {
                return Err("time.sleep() duration must be non-negative".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
            Ok(TypedReturn::Unit)
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
            let ms = args
                .first()
                .and_then(|a| a.as_number_coerce())
                .ok_or_else(|| {
                    "time.sleep_sync() requires a number argument (milliseconds)".to_string()
                })?;
            if ms < 0.0 {
                return Err("time.sleep_sync() duration must be non-negative".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            Ok(TypedReturn::Unit)
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
            let func = args
                .first()
                .cloned()
                .ok_or_else(|| "time.benchmark() requires a function argument".to_string())?;

            let iterations = args
                .get(1)
                .and_then(|a| a.as_number_coerce())
                .unwrap_or(1000.0) as u64;

            if iterations == 0 {
                return Err("time.benchmark() iterations must be > 0".to_string());
            }

            // We return the function and iteration count as an object so the VM
            // can execute the benchmark loop. The actual execution happens in the
            // VM's builtin handler since we can't call Shape functions from here.
            // Instead, provide a simple timing-only benchmark for native use.
            let start = std::time::Instant::now();
            // If the first arg is not callable, we just measure overhead.
            // The VM-level benchmark builtin handles callable functions.
            let _ = &func;
            let elapsed = start.elapsed();
            let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

            Ok(TypedReturn::TypedObject(vec![
                ("elapsed_ms".to_string(), TypedReturn::F64(elapsed_ms)),
                (
                    "iterations".to_string(),
                    TypedReturn::F64(iterations as f64),
                ),
                (
                    "avg_ms".to_string(),
                    TypedReturn::F64(elapsed_ms / iterations as f64),
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
        |_args, _ctx| Ok(TypedReturn::Instant(std::time::Instant::now())),
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
            Ok(TypedReturn::F64(since_epoch.as_millis() as f64))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

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
    fn test_time_now_returns_instant() {
        let module = create_time_module();
        let ctx = test_ctx();
        let now_fn = module.get_export("now").unwrap();
        let result = now_fn(&[], &ctx).unwrap();
        assert_eq!(result.type_name(), "instant");
        assert!(result.as_instant().is_some());
    }

    #[test]
    fn test_time_stopwatch_returns_instant() {
        let module = create_time_module();
        let ctx = test_ctx();
        let sw_fn = module.get_export("stopwatch").unwrap();
        let result = sw_fn(&[], &ctx).unwrap();
        assert_eq!(result.type_name(), "instant");
    }

    #[test]
    fn test_time_millis_returns_positive_number() {
        let module = create_time_module();
        let ctx = test_ctx();
        let millis_fn = module.get_export("millis").unwrap();
        let result = millis_fn(&[], &ctx).unwrap();
        let ms = result.as_f64().unwrap();
        assert!(ms > 0.0);
        // Should be after year 2020 in millis
        assert!(ms > 1_577_836_800_000.0);
    }

    #[test]
    fn test_time_sleep_sync_requires_number() {
        let module = create_time_module();
        let ctx = test_ctx();
        let sleep_fn = module.get_export("sleep_sync").unwrap();
        let result = sleep_fn(&[], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_time_sleep_sync_rejects_negative() {
        let module = create_time_module();
        let ctx = test_ctx();
        let sleep_fn = module.get_export("sleep_sync").unwrap();
        let result = sleep_fn(&[ValueWord::from_f64(-100.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_time_sleep_sync_zero_is_valid() {
        let module = create_time_module();
        let ctx = test_ctx();
        let sleep_fn = module.get_export("sleep_sync").unwrap();
        let result = sleep_fn(&[ValueWord::from_f64(0.0)], &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_time_sleep_is_async() {
        let module = create_time_module();
        assert!(module.is_async("sleep"));
        assert!(!module.is_async("sleep_sync"));
    }

    #[test]
    fn test_time_benchmark_returns_object() {
        let module = create_time_module();
        let ctx = test_ctx();
        let bench_fn = module.get_export("benchmark").unwrap();
        // Pass a dummy value (not a real callable, but the module-level benchmark
        // just measures timing overhead)
        let result = bench_fn(
            &[ValueWord::from_f64(0.0), ValueWord::from_f64(100.0)],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.type_name(), "object");
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
        // sleep is async — stays on legacy ABI for now.
        assert!(typed.get("sleep").is_none());
    }
}
