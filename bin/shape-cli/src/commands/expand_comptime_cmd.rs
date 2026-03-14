use crate::{extension_loading, module_loading};
use anyhow::{Context, Result, bail};
use shape_ast::Item;
use shape_ast::{ExtendStatement, FunctionDef, TypeAnnotation, TypeName};
use shape_runtime::engine::ShapeEngine;
use shape_vm::BytecodeExecutor;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::fs;

/// Expand compile-time generated code from a Shape script.
///
/// This runs the full compile pipeline (including stdlib/extensions/comptime handlers)
/// and prints human-readable expansion artifacts.
pub async fn run_expand_comptime(
    script: PathBuf,
    module_filter: Option<String>,
    function_filter: Option<String>,
) -> Result<()> {
    let content = fs::read_to_string(&script)
        .await
        .with_context(|| format!("failed to read {}", script.display()))?;
    let (frontmatter, source) = shape_runtime::frontmatter::parse_frontmatter(&content);

    let project_root = extension_loading::detect_project_root_for_script(Some(&script));
    if project_root.is_some() && frontmatter.is_some() {
        bail!(
            "Frontmatter and shape.toml are mutually exclusive. Remove the frontmatter block or run this script outside a shape.toml project."
        );
    }

    let mut engine = ShapeEngine::new().context("failed to create Shape engine")?;
    engine
        .load_stdlib()
        .context("failed to load Shape stdlib")?;
    engine.set_script_path(script.display().to_string());

    let startup_specs = extension_loading::collect_startup_specs(
        &super::ProviderOptions::default(),
        project_root.as_ref(),
        if project_root.is_none() {
            frontmatter.as_ref()
        } else {
            None
        },
        Some(script.as_path()),
        &[],
    );

    let mut load_errors = Vec::new();
    extension_loading::load_specs(
        &mut engine,
        &startup_specs,
        |_spec, _info| {},
        |spec, err| {
            load_errors.push(format!("{}: {}", spec.display_name(), err));
        },
    );
    if !load_errors.is_empty() {
        bail!(
            "failed to load extension modules for expansion:\n{}",
            load_errors.join("\n")
        );
    }

    let program = engine.parse_and_analyze(source).with_context(|| {
        format!(
            "failed to parse/analyze source before comptime expansion: {}",
            script.display()
        )
    })?;
    let generated_extends = shape_ast::transform::collect_generated_annotation_extends(&program);
    let user_function_names = collect_program_function_names(&program);
    let generated_method_names = collect_generated_method_names(&generated_extends);

    let mut executor = BytecodeExecutor::new();
    extension_loading::register_extension_capability_modules(&engine, &mut executor);
    let module_info = executor.module_schemas();
    engine.register_extension_modules(&module_info);
    engine.register_language_runtime_artifacts();
    module_loading::wire_vm_executor_module_loading(
        &mut engine,
        &mut executor,
        Some(script.as_path()),
        Some(source),
    )?;

    let bytecode = executor.compile_program_for_inspection(&mut engine, &program);

    let expanded_functions = match bytecode {
        Ok(bytecode) => bytecode.expanded_function_defs,
        Err(err) => {
            eprintln!(
                "warning: expansion compile failed; showing pre-compile function/extend view"
            );
            let fallback_functions = collect_program_function_defs(&program);
            render_expansion_report(
                script.as_path(),
                &fallback_functions,
                &generated_extends,
                &user_function_names,
                &generated_method_names,
                module_filter.as_deref(),
                function_filter.as_deref(),
            );
            return Err(err).context("failed to compile program for expansion output");
        }
    };

    render_expansion_report(
        script.as_path(),
        &expanded_functions,
        &generated_extends,
        &user_function_names,
        &generated_method_names,
        module_filter.as_deref(),
        function_filter.as_deref(),
    );

    Ok(())
}

fn render_expansion_report(
    script: &std::path::Path,
    expanded_functions: &HashMap<String, FunctionDef>,
    generated_extends: &[ExtendStatement],
    user_function_names: &HashSet<String>,
    generated_method_names: &HashSet<String>,
    module_filter: Option<&str>,
    function_filter: Option<&str>,
) {
    let mut functions: Vec<&FunctionDef> = expanded_functions
        .values()
        .filter(|f| {
            user_function_names.contains(&f.name)
                || f.name.contains("__const_")
                || generated_method_names.contains(&f.name)
        })
        .filter(|f| function_matches_filters(&f.name, module_filter, function_filter))
        .collect();
    functions.sort_by(|a, b| a.name.cmp(&b.name));

    let mut extends: Vec<&ExtendStatement> = generated_extends
        .iter()
        .filter(|ext| extend_matches_filters(ext, module_filter, function_filter))
        .collect();
    extends
        .sort_by(|a, b| type_name_to_string(&a.type_name).cmp(&type_name_to_string(&b.type_name)));

    if functions.is_empty() && extends.is_empty() {
        println!("No comptime expansions found for {}.", script.display());
        return;
    }

    println!("Comptime expansion report: {}", script.display());
    if let Some(module) = module_filter {
        println!("filter module: {}", module);
    }
    if let Some(function) = function_filter {
        println!("filter function: {}", function);
    }

    println!();
    println!("Functions (post-comptime): {}", functions.len());
    for func in functions {
        println!("{}", format_function_signature(func));
    }

    println!();
    println!("Generated extends: {}", extends.len());
    for ext in extends {
        println!("extend {}:", type_name_to_string(&ext.type_name));
        for method in &ext.methods {
            println!("  method {}", method.name);
        }
    }
}

fn function_matches_filters(
    name: &str,
    module_filter: Option<&str>,
    function_filter: Option<&str>,
) -> bool {
    if let Some(function_name) = function_filter
        && name != function_name
        && !name.ends_with(&format!("::{}", function_name))
        && !name.ends_with(&format!(".{}", function_name))
    {
        return false;
    }

    if let Some(module_name) = module_filter
        && !name.starts_with(&format!("{}::", module_name))
        && !name.starts_with(&format!("{}.", module_name))
    {
        return false;
    }

    true
}

fn collect_program_function_names(program: &shape_ast::Program) -> HashSet<String> {
    let mut names = HashSet::new();
    for item in &program.items {
        if let Item::Function(func, _) = item {
            names.insert(func.name.clone());
        }
    }
    names
}

fn collect_generated_method_names(generated_extends: &[ExtendStatement]) -> HashSet<String> {
    let mut names = HashSet::new();
    for ext in generated_extends {
        let type_str = type_name_to_string(&ext.type_name);
        for method in &ext.methods {
            // Extend methods are compiled as "Type.method"
            names.insert(format!("{}.{}", type_str, method.name));
        }
    }
    names
}

fn collect_program_function_defs(program: &shape_ast::Program) -> HashMap<String, FunctionDef> {
    let mut defs = HashMap::new();
    for item in &program.items {
        if let Item::Function(func, _) = item {
            defs.insert(func.name.clone(), func.clone());
        }
    }
    defs
}

fn extend_matches_filters(
    ext: &ExtendStatement,
    module_filter: Option<&str>,
    function_filter: Option<&str>,
) -> bool {
    if let Some(function_name) = function_filter
        && !ext.methods.iter().any(|m| m.name == function_name)
    {
        return false;
    }

    if let Some(module_name) = module_filter {
        let type_name = type_name_to_string(&ext.type_name);
        if !type_name.starts_with(&format!("{}::", module_name))
            && !type_name.starts_with(&format!("{}.", module_name))
        {
            return false;
        }
    }

    true
}

fn format_function_signature(func: &FunctionDef) -> String {
    let mut params = Vec::with_capacity(func.params.len());
    for param in &func.params {
        let base_name = param
            .simple_name()
            .map(ToString::to_string)
            .unwrap_or_else(|| "_".to_string());
        let mut rendered = if param.is_const {
            format!("const {}", base_name)
        } else {
            base_name
        };
        if let Some(ann) = &param.type_annotation {
            rendered.push_str(": ");
            rendered.push_str(&format_type_annotation(ann));
        }
        params.push(rendered);
    }

    let ret = func
        .return_type
        .as_ref()
        .map(format_type_annotation)
        .unwrap_or_else(|| "()".to_string());
    format!("fn {}({}) -> {}", func.name, params.join(", "), ret)
}

fn type_name_to_string(ty: &TypeName) -> String {
    match ty {
        TypeName::Simple(name) => name.to_string(),
        TypeName::Generic {
            name, type_args, ..
        } => {
            let args = type_args
                .iter()
                .map(format_type_annotation)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}<{}>", name, args)
        }
    }
}

fn format_type_annotation(ta: &TypeAnnotation) -> String {
    match ta {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.to_string(),
        TypeAnnotation::Generic { name, args } => {
            let args = args
                .iter()
                .map(format_type_annotation)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}<{}>", name, args)
        }
        TypeAnnotation::Array(inner) => format!("Array<{}>", format_type_annotation(inner)),
        TypeAnnotation::Tuple(types) => {
            let inner = types
                .iter()
                .map(format_type_annotation)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({})", inner)
        }
        TypeAnnotation::Union(types) => types
            .iter()
            .map(format_type_annotation)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeAnnotation::Intersection(types) => types
            .iter()
            .map(format_type_annotation)
            .collect::<Vec<_>>()
            .join(" + "),
        TypeAnnotation::Function { params, returns } => {
            let params = params
                .iter()
                .map(|p| format_type_annotation(&p.type_annotation))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({}) -> {}", params, format_type_annotation(returns))
        }
        TypeAnnotation::Object(fields) => {
            let fields = fields
                .iter()
                .map(|f| format!("{}: {}", f.name, format_type_annotation(&f.type_annotation)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {} }}", fields)
        }
        TypeAnnotation::Void => "()".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "null".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(bounds) => format!("dyn {}", bounds.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(" + ")),
    }
}
