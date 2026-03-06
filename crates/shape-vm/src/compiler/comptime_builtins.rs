//! Comptime builtin functions.
//!
//! These functions are only callable inside `comptime { }` blocks.
//! They provide compile-time reflection, trait checking, and compiler messaging.
//!
//! Available builtins:
//! - `implements(T, Trait)` — returns true if T implements Trait
//! - `warning(msg)` — emits a compile-time warning
//! - `error(msg)` — emits a compile-time error
//! - `build_config()` — returns build-time configuration

use shape_runtime::module_exports::ModuleExports;
use shape_runtime::type_schema::typed_object_from_nb_pairs;
use shape_runtime::type_system::BuiltinTypes;
use shape_value::ValueWord;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

/// Directives emitted during comptime execution (e.g., from `extend target`).
#[derive(Debug, Clone)]
pub(crate) enum ComptimeDirective {
    Extend(shape_ast::ast::ExtendStatement),
    RemoveTarget,
    SetParamType {
        param_name: String,
        type_annotation: shape_ast::ast::TypeAnnotation,
    },
    SetReturnType {
        type_annotation: shape_ast::ast::TypeAnnotation,
    },
    ReplaceBody {
        body: Vec<shape_ast::ast::Statement>,
    },
    ReplaceModule {
        items: Vec<shape_ast::ast::Item>,
    },
}

thread_local! {
    static COMPTIME_DIRECTIVES: RefCell<Vec<ComptimeDirective>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn clear_comptime_directives() {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        directives.clear();
    });
}

pub(crate) fn take_comptime_directives() -> Vec<ComptimeDirective> {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        std::mem::take(&mut *directives)
    })
}

fn push_comptime_directive(directive: ComptimeDirective) -> Result<(), String> {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        directives.push(directive);
    });
    Ok(())
}

fn parse_type_annotation_payload(payload: &str) -> Result<shape_ast::ast::TypeAnnotation, String> {
    if let Ok(parsed) = serde_json::from_str::<shape_ast::ast::TypeAnnotation>(payload) {
        return Ok(parsed);
    }

    // Fallback for older callers that still pass textual type source.
    let snippet = format!("fn __type_probe(value: {}) {{ value }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid type payload '{}': {}", payload, e))?;

    let maybe_ann = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Function(func, _) => {
            func.params.first().and_then(|p| p.type_annotation.clone())
        }
        _ => None,
    });

    maybe_ann.ok_or_else(|| format!("could not parse type payload '{}'", payload))
}

fn parse_function_body_payload(payload: &str) -> Result<Vec<shape_ast::ast::Statement>, String> {
    if let Ok(parsed) = serde_json::from_str::<Vec<shape_ast::ast::Statement>>(payload) {
        return Ok(parsed);
    }

    // Fallback for older callers that still pass source text.
    let snippet = format!("fn __body_probe() {{ {} }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid replacement body payload: {}", e))?;

    let maybe_body = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Function(func, _) => Some(func.body),
        _ => None,
    });

    maybe_body.ok_or_else(|| "could not parse replacement function body payload".to_string())
}

fn parse_module_items_payload(payload: &str) -> Result<Vec<shape_ast::ast::Item>, String> {
    if let Ok(parsed) = serde_json::from_str::<Vec<shape_ast::ast::Item>>(payload) {
        return Ok(parsed);
    }

    let snippet = format!("mod __module_probe__ {{ {} }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid replacement module payload: {}", e))?;

    let maybe_items = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Module(module, _) if module.name == "__module_probe__" => {
            Some(module.items)
        }
        _ => None,
    });

    maybe_items.ok_or_else(|| "could not parse replacement module payload".to_string())
}

/// Helper: create a ValueWord string from a &str.
fn nb_str(s: &str) -> ValueWord {
    ValueWord::from_string(Arc::new(s.to_string()))
}

/// Create a ModuleExports containing all comptime builtin functions.
///
/// These are registered as an extension module named "__comptime__" so they
/// are available during comptime execution but NOT during normal runtime.
///
/// `trait_impl_keys` contains the set of registered trait implementations.
/// Supported key forms:
/// - Legacy: "TraitName::TypeName"
/// - Canonical: "TraitName::TypeName::ImplNameOrDefault"
pub(crate) fn create_comptime_builtins_module(trait_impl_keys: HashSet<String>) -> ModuleExports {
    let mut module = ModuleExports::new("__comptime__");

    // implements(type_name: string, trait_name: string) -> bool
    // Checks the TypeRegistry's trait impl data captured at compile time.
    module.add_function(
        "implements",
        move |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            let type_name = match nb_args.first().and_then(|nb| nb.as_str()) {
                Some(s) => s.to_string(),
                None => return Ok(ValueWord::from_bool(false)),
            };
            let trait_name = match nb_args.get(1).and_then(|nb| nb.as_str()) {
                Some(s) => s.to_string(),
                None => return Ok(ValueWord::from_bool(false)),
            };

            let has_impl = |ty: &str| {
                let legacy = format!("{}::{}", trait_name, ty);
                let canonical_prefix = format!("{}::{}::", trait_name, ty);
                trait_impl_keys.contains(&legacy)
                    || trait_impl_keys
                        .iter()
                        .any(|key| key.starts_with(&canonical_prefix))
            };

            if has_impl(&type_name) {
                return Ok(ValueWord::from_bool(true));
            }

            // Numeric widening: integer-family aliases can satisfy number-family impls.
            if BuiltinTypes::is_integer_type_name(&type_name) {
                for widen_to in &["number", "float", "f64"] {
                    if has_impl(widen_to) {
                        return Ok(ValueWord::from_bool(true));
                    }
                }
            }

            Ok(ValueWord::from_bool(false))
        },
    );

    // warning(msg: string) -> Unit
    // Emits a compile-time warning message to stderr.
    module.add_function(
        "warning",
        |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            if let Some(msg) = nb_args.first().and_then(|nb| nb.as_str()) {
                eprintln!("[comptime warning] {}", msg);
            }
            Ok(ValueWord::unit())
        },
    );

    // error(msg: string) -> never (returns an error)
    // Emits a compile-time error. This aborts comptime execution.
    module.add_function(
        "error",
        |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            let msg = match nb_args.first() {
                Some(nb) => match nb.as_str() {
                    Some(s) => s.to_string(),
                    None => format!("{}", nb),
                },
                None => "comptime error".to_string(),
            };
            Err(format!("[comptime error] {}", msg))
        },
    );

    // build_config() -> Object with build configuration
    // Returns a structured object: { debug, version, target_os, target_arch }
    // Pre-register the schema so the comptime compiler can resolve field access.
    let _build_config_schema = shape_runtime::type_schema::register_predeclared_any_schema(&[
        "debug".to_string(),
        "target_arch".to_string(),
        "target_os".to_string(),
        "version".to_string(),
    ]);
    module.add_function(
        "build_config",
        |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            Ok(typed_object_from_nb_pairs(&[
                ("debug", ValueWord::from_bool(cfg!(debug_assertions))),
                ("version", nb_str(env!("CARGO_PKG_VERSION"))),
                ("target_os", nb_str(std::env::consts::OS)),
                ("target_arch", nb_str(std::env::consts::ARCH)),
            ]))
        },
    );

    // Internal comptime directive: emit an extend statement payload (JSON AST).
    module.add_function(
        "__emit_extend",
        |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            let json = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .ok_or_else(|| "__emit_extend expects a JSON string payload".to_string())?;
            let extend: shape_ast::ast::ExtendStatement =
                serde_json::from_str(json).map_err(|e| format!("invalid extend payload: {}", e))?;
            push_comptime_directive(ComptimeDirective::Extend(extend))?;
            Ok(ValueWord::unit())
        },
    );

    // Internal comptime directive: remove the current annotation target.
    module.add_function(
        "__emit_remove",
        |_nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            push_comptime_directive(ComptimeDirective::RemoveTarget)?;
            Ok(ValueWord::unit())
        },
    );

    // Internal comptime directive: set a parameter type by parameter name.
    // __emit_set_param_type(param_name: string, type_payload: string)
    module.add_function(
        "__emit_set_param_type",
        |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            let param_name = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .ok_or_else(|| {
                    "__emit_set_param_type expects param name as first string arg".to_string()
                })?
                .to_string();
            let payload = nb_args.get(1).and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_set_param_type expects type annotation as second string arg".to_string()
            })?;
            let type_annotation = parse_type_annotation_payload(payload)?;
            push_comptime_directive(ComptimeDirective::SetParamType {
                param_name,
                type_annotation,
            })?;
            Ok(ValueWord::unit())
        },
    );

    // Internal comptime directive: set function return type.
    // __emit_set_return_type(type_payload: string)
    module.add_function(
        "__emit_set_return_type",
        |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            let payload = nb_args.first().and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_set_return_type expects a type annotation string".to_string()
            })?;
            let type_annotation = parse_type_annotation_payload(payload)?;
            push_comptime_directive(ComptimeDirective::SetReturnType { type_annotation })?;
            Ok(ValueWord::unit())
        },
    );

    // Internal comptime directive: replace function body from serialized AST payload.
    // __emit_replace_body(body_payload: string)
    module.add_function(
        "__emit_replace_body",
        |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            let payload = nb_args.first().and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_replace_body expects a function body source string".to_string()
            })?;
            let body = parse_function_body_payload(payload)?;
            push_comptime_directive(ComptimeDirective::ReplaceBody { body })?;
            Ok(ValueWord::unit())
        },
    );

    // Internal comptime directive: replace module items from source payload.
    // __emit_replace_module(module_payload: string)
    module.add_function(
        "__emit_replace_module",
        |nb_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
            let payload = nb_args.first().and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_replace_module expects a module body source string".to_string()
            })?;
            let items = parse_module_items_payload(payload)?;
            push_comptime_directive(ComptimeDirective::ReplaceModule { items })?;
            Ok(ValueWord::unit())
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::TypeSchemaRegistry;

    fn test_ctx() -> shape_runtime::module_exports::ModuleContext<'static> {
        // Leak a registry so we get a &'static reference for tests
        let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
        shape_runtime::module_exports::ModuleContext {
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
    fn test_comptime_builtins_module_created() {
        let module = create_comptime_builtins_module(Default::default());
        assert_eq!(module.name, "__comptime__");
    }

    #[test]
    fn test_comptime_warning_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default());
        let func = module
            .exports
            .get("warning")
            .expect("warning function should exist");
        let args = vec![nb_str("test warning")];
        let result = func(&args, &ctx);
        assert!(result.is_ok());
        assert!(result.unwrap().is_unit());
    }

    #[test]
    fn test_comptime_error_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default());
        let func = module
            .exports
            .get("error")
            .expect("error function should exist");
        let args = vec![nb_str("something failed")];
        let result = func(&args, &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{}", err).contains("something failed"));
    }

    #[test]
    fn test_comptime_implements_returns_false_when_not_registered() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default());
        let func = module
            .exports
            .get("implements")
            .expect("implements function should exist");
        let args = vec![nb_str("Currency"), nb_str("Display")];
        let result = func(&args, &ctx);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_comptime_implements_returns_true_when_registered() {
        let ctx = test_ctx();
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        impls.insert("Display::Currency".to_string());
        let module = create_comptime_builtins_module(impls);
        let func = module
            .exports
            .get("implements")
            .expect("implements function should exist");

        // Exact match
        let result = func(&vec![nb_str("number"), nb_str("Serializable")], &ctx);
        assert_eq!(result.unwrap().as_bool(), Some(true));

        // Another exact match
        let result = func(&vec![nb_str("Currency"), nb_str("Display")], &ctx);
        assert_eq!(result.unwrap().as_bool(), Some(true));

        // Not registered
        let result = func(&vec![nb_str("string"), nb_str("Serializable")], &ctx);
        assert_eq!(result.unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_comptime_implements_numeric_widening() {
        let ctx = test_ctx();
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        let module = create_comptime_builtins_module(impls);
        let func = module
            .exports
            .get("implements")
            .expect("implements function should exist");

        // int should widen to number
        let result = func(&vec![nb_str("int"), nb_str("Serializable")], &ctx);
        assert_eq!(result.unwrap().as_bool(), Some(true));

        // i64 should also widen to number
        let result = func(&vec![nb_str("i64"), nb_str("Serializable")], &ctx);
        assert_eq!(result.unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_comptime_build_config_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default());
        let func = module
            .exports
            .get("build_config")
            .expect("build_config function should exist");
        let result = func(&[], &ctx);
        assert!(result.is_ok());
        // build_config now returns TypedObject
        assert_eq!(result.unwrap().clone().type_name(), "object");
    }
}
