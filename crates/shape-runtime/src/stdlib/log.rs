//! Native `log` module for structured logging.
//!
//! Exports: log.debug, log.info, log.warn, log.error, log.set_level

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use std::sync::atomic::{AtomicU8, Ordering};

/// Log level constants, ordered by severity.
const LEVEL_DEBUG: u8 = 0;
const LEVEL_INFO: u8 = 1;
const LEVEL_WARN: u8 = 2;
const LEVEL_ERROR: u8 = 3;

/// Global minimum log level. Messages below this level are silently dropped.
static MIN_LEVEL: AtomicU8 = AtomicU8::new(LEVEL_DEBUG);

/// Format optional fields object into a string suffix for the log message.
fn format_fields(args: &[ValueWord]) -> String {
    let fields_arg = args.get(1);
    match fields_arg {
        Some(f) if !f.is_none() && !f.is_unit() => {
            let json = f.to_json_value();
            if let serde_json::Value::Object(map) = json {
                if map.is_empty() {
                    return String::new();
                }
                let pairs: Vec<String> = map.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                format!(" {}", pairs.join(" "))
            } else {
                format!(" fields={}", json)
            }
        }
        _ => String::new(),
    }
}

fn parse_level(name: &str) -> Result<u8, String> {
    match name.to_lowercase().as_str() {
        "debug" => Ok(LEVEL_DEBUG),
        "info" => Ok(LEVEL_INFO),
        "warn" | "warning" => Ok(LEVEL_WARN),
        "error" => Ok(LEVEL_ERROR),
        _ => Err(format!(
            "log.set_level() unknown level '{}'. Use: debug, info, warn, error",
            name
        )),
    }
}

/// Create the `log` module with structured logging functions.
pub fn create_log_module() -> ModuleExports {
    let mut module = ModuleExports::new("log");
    module.description = "Structured logging utilities".to_string();

    let msg_param = ModuleParam {
        name: "message".to_string(),
        type_name: "string".to_string(),
        required: true,
        description: "Log message".to_string(),
        ..Default::default()
    };

    let fields_param = ModuleParam {
        name: "fields".to_string(),
        type_name: "object".to_string(),
        required: false,
        description: "Optional structured fields to attach to the log entry".to_string(),
        ..Default::default()
    };

    // log.debug(message: string, fields?: object) -> unit
    module.add_function_with_schema(
        "debug",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            if MIN_LEVEL.load(Ordering::Relaxed) > LEVEL_DEBUG {
                return Ok(ValueWord::unit());
            }
            let msg = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "log.debug() requires a message string".to_string())?;
            let fields = format_fields(args);
            tracing::debug!("[shape] {}{}", msg, fields);
            Ok(ValueWord::unit())
        },
        ModuleFunction {
            description: "Log a debug-level message".to_string(),
            params: vec![msg_param.clone(), fields_param.clone()],
            return_type: Some("unit".to_string()),
        },
    );

    // log.info(message: string, fields?: object) -> unit
    module.add_function_with_schema(
        "info",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            if MIN_LEVEL.load(Ordering::Relaxed) > LEVEL_INFO {
                return Ok(ValueWord::unit());
            }
            let msg = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "log.info() requires a message string".to_string())?;
            let fields = format_fields(args);
            tracing::info!("[shape] {}{}", msg, fields);
            Ok(ValueWord::unit())
        },
        ModuleFunction {
            description: "Log an info-level message".to_string(),
            params: vec![msg_param.clone(), fields_param.clone()],
            return_type: Some("unit".to_string()),
        },
    );

    // log.warn(message: string, fields?: object) -> unit
    module.add_function_with_schema(
        "warn",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            if MIN_LEVEL.load(Ordering::Relaxed) > LEVEL_WARN {
                return Ok(ValueWord::unit());
            }
            let msg = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "log.warn() requires a message string".to_string())?;
            let fields = format_fields(args);
            tracing::warn!("[shape] {}{}", msg, fields);
            Ok(ValueWord::unit())
        },
        ModuleFunction {
            description: "Log a warning-level message".to_string(),
            params: vec![msg_param.clone(), fields_param.clone()],
            return_type: Some("unit".to_string()),
        },
    );

    // log.error(message: string, fields?: object) -> unit
    module.add_function_with_schema(
        "error",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            if MIN_LEVEL.load(Ordering::Relaxed) > LEVEL_ERROR {
                return Ok(ValueWord::unit());
            }
            let msg = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "log.error() requires a message string".to_string())?;
            let fields = format_fields(args);
            tracing::error!("[shape] {}{}", msg, fields);
            Ok(ValueWord::unit())
        },
        ModuleFunction {
            description: "Log an error-level message".to_string(),
            params: vec![msg_param, fields_param],
            return_type: Some("unit".to_string()),
        },
    );

    // log.set_level(level: string) -> unit
    module.add_function_with_schema(
        "set_level",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let level_str = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "log.set_level() requires a level string argument".to_string())?;

            let level = parse_level(level_str)?;
            MIN_LEVEL.store(level, Ordering::Relaxed);
            Ok(ValueWord::unit())
        },
        ModuleFunction {
            description: "Set the minimum log level (debug, info, warn, error)".to_string(),
            params: vec![ModuleParam {
                name: "level".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Minimum log level: debug, info, warn, error".to_string(),
                allowed_values: Some(vec![
                    "debug".to_string(),
                    "info".to_string(),
                    "warn".to_string(),
                    "error".to_string(),
                ]),
                ..Default::default()
            }],
            return_type: Some("unit".to_string()),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(val: &str) -> ValueWord {
        ValueWord::from_string(std::sync::Arc::new(val.to_string()))
    }

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
    fn test_log_module_creation() {
        let module = create_log_module();
        assert_eq!(module.name, "log");
        assert!(module.has_export("debug"));
        assert!(module.has_export("info"));
        assert!(module.has_export("warn"));
        assert!(module.has_export("error"));
        assert!(module.has_export("set_level"));
    }

    #[test]
    fn test_log_debug() {
        let module = create_log_module();
        let ctx = test_ctx();
        // Reset level to debug for this test
        MIN_LEVEL.store(LEVEL_DEBUG, Ordering::Relaxed);
        let f = module.get_export("debug").unwrap();
        let result = f(&[s("test message")], &ctx);
        assert!(result.is_ok());
        assert!(result.unwrap().is_unit());
    }

    #[test]
    fn test_log_info() {
        let module = create_log_module();
        let ctx = test_ctx();
        let f = module.get_export("info").unwrap();
        let result = f(&[s("info message")], &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_log_warn() {
        let module = create_log_module();
        let ctx = test_ctx();
        let f = module.get_export("warn").unwrap();
        let result = f(&[s("warning message")], &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_log_error() {
        let module = create_log_module();
        let ctx = test_ctx();
        let f = module.get_export("error").unwrap();
        let result = f(&[s("error message")], &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_log_requires_string() {
        let module = create_log_module();
        let ctx = test_ctx();
        let f = module.get_export("info").unwrap();
        assert!(f(&[ValueWord::from_f64(42.0)], &ctx).is_err());
        assert!(f(&[], &ctx).is_err());
    }

    #[test]
    fn test_set_level_valid() {
        let module = create_log_module();
        let ctx = test_ctx();
        let f = module.get_export("set_level").unwrap();
        assert!(f(&[s("info")], &ctx).is_ok());
        assert_eq!(MIN_LEVEL.load(Ordering::Relaxed), LEVEL_INFO);
        // Reset
        assert!(f(&[s("debug")], &ctx).is_ok());
        assert_eq!(MIN_LEVEL.load(Ordering::Relaxed), LEVEL_DEBUG);
    }

    #[test]
    fn test_set_level_invalid() {
        let module = create_log_module();
        let ctx = test_ctx();
        let f = module.get_export("set_level").unwrap();
        assert!(f(&[s("critical")], &ctx).is_err());
    }

    #[test]
    fn test_set_level_case_insensitive() {
        let module = create_log_module();
        let ctx = test_ctx();
        let f = module.get_export("set_level").unwrap();
        assert!(f(&[s("WARN")], &ctx).is_ok());
        assert_eq!(MIN_LEVEL.load(Ordering::Relaxed), LEVEL_WARN);
        assert!(f(&[s("Warning")], &ctx).is_ok());
        assert_eq!(MIN_LEVEL.load(Ordering::Relaxed), LEVEL_WARN);
        // Reset
        let _ = f(&[s("debug")], &ctx);
    }

    #[test]
    fn test_log_level_filtering() {
        let module = create_log_module();
        let ctx = test_ctx();
        let set_level = module.get_export("set_level").unwrap();
        let debug_fn = module.get_export("debug").unwrap();
        let error_fn = module.get_export("error").unwrap();

        // Set level to error - debug should be silently dropped
        set_level(&[s("error")], &ctx).unwrap();
        let result = debug_fn(&[s("should be dropped")], &ctx);
        assert!(result.is_ok());
        assert!(result.unwrap().is_unit());

        // Error should still work
        let result = error_fn(&[s("error still works")], &ctx);
        assert!(result.is_ok());

        // Reset
        let _ = set_level(&[s("debug")], &ctx);
    }

    #[test]
    fn test_log_schemas() {
        let module = create_log_module();

        let info_schema = module.get_schema("info").unwrap();
        assert_eq!(info_schema.params.len(), 2);
        assert!(info_schema.params[0].required);
        assert!(!info_schema.params[1].required);
        assert_eq!(info_schema.return_type.as_deref(), Some("unit"));

        let level_schema = module.get_schema("set_level").unwrap();
        assert_eq!(level_schema.params.len(), 1);
        assert!(level_schema.params[0].allowed_values.is_some());
    }
}
