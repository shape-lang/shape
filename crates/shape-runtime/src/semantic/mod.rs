//! Semantic analysis module for Shape
//!
//! This module performs type checking, symbol resolution, and validation
//! of the parsed AST before execution.

use shape_ast::error::{Result, ShapeError, span_to_location};

pub mod symbol_table;
pub mod types;
pub mod validator;

// Analysis modules
mod builtins;
mod control_flow;
mod function_analysis;
mod module_analysis;
mod pattern_analysis;
mod stream_analysis;
mod test_analysis;

use crate::extensions::ParsedModuleSchema;
use crate::pattern_library::PatternLibrary;
use crate::snapshot::SemanticSnapshot;
use crate::type_system::{TypeInferenceEngine, error_bridge::type_error_to_shape};
use shape_ast::ast::{
    Expr, FunctionParam, Item, ObjectTypeField, Program, Span, Spanned, TypeAnnotation, TypeName,
};
use symbol_table::SymbolTable;
use validator::Validator;

/// Convert a TypeAnnotation from the AST to a semantic Type
pub fn type_annotation_to_type(annotation: &TypeAnnotation) -> types::Type {
    type_annotation_to_type_with_aliases(annotation, None)
}

/// Convert a TypeAnnotation to a semantic Type, resolving type aliases via symbol table
pub fn type_annotation_to_type_with_aliases(
    annotation: &TypeAnnotation,
    symbol_table: Option<&SymbolTable>,
) -> types::Type {
    match annotation {
        TypeAnnotation::Basic(name) => match name.as_str() {
            "number" | "Number" | "float" | "int" => types::Type::Number,
            "string" | "String" => types::Type::String,
            "bool" | "Bool" | "boolean" => types::Type::Bool,
            "row" | "Row" => types::Type::Object(vec![]),
            "color" | "Color" => types::Type::Color,
            "timestamp" | "Timestamp" => types::Type::Timestamp,
            "timeframe" | "Timeframe" => types::Type::Timeframe,
            "duration" | "Duration" => types::Type::Duration,
            "pattern" | "Pattern" => types::Type::Pattern,
            "AnyError" | "anyerror" => types::Type::Error,
            _ => {
                // Try to resolve as a type alias (e.g., "Candle")
                if let Some(st) = symbol_table {
                    if let Some(alias_entry) = st.lookup_type_alias(name) {
                        return type_annotation_to_type_with_aliases(
                            &alias_entry.type_annotation,
                            symbol_table,
                        );
                    }
                }
                types::Type::Unknown
            }
        },
        TypeAnnotation::Array(elem) => types::Type::Array(Box::new(
            type_annotation_to_type_with_aliases(elem, symbol_table),
        )),
        TypeAnnotation::Generic { name, args } => match name.as_str() {
            "Column" if !args.is_empty() => types::Type::Column(Box::new(
                type_annotation_to_type_with_aliases(&args[0], symbol_table),
            )),
            "Vec" if !args.is_empty() => types::Type::Array(Box::new(
                type_annotation_to_type_with_aliases(&args[0], symbol_table),
            )),
            "Mat" if !args.is_empty() => types::Type::Matrix(Box::new(
                type_annotation_to_type_with_aliases(&args[0], symbol_table),
            )),
            "Result" if !args.is_empty() => types::Type::Result(Box::new(
                type_annotation_to_type_with_aliases(&args[0], symbol_table),
            )),
            // Option<T> is represented as nullable T in the legacy semantic layer.
            "Option" if !args.is_empty() => {
                type_annotation_to_type_with_aliases(&args[0], symbol_table)
            }
            _ => types::Type::Unknown,
        },
        TypeAnnotation::Function { params, returns } => {
            let param_types: Vec<types::Type> = params
                .iter()
                .map(|p| type_annotation_to_type_with_aliases(&p.type_annotation, symbol_table))
                .collect();
            let return_type = type_annotation_to_type_with_aliases(returns, symbol_table);
            types::Type::Function {
                params: param_types,
                returns: Box::new(return_type),
            }
        }
        TypeAnnotation::Object(fields) => {
            // Convert object type annotation to Type::Object with fields
            let type_fields: Vec<(String, types::Type)> = fields
                .iter()
                .map(|f| {
                    (
                        f.name.clone(),
                        type_annotation_to_type_with_aliases(&f.type_annotation, symbol_table),
                    )
                })
                .collect();
            types::Type::Object(type_fields)
        }
        TypeAnnotation::Reference(name) => {
            // Try to resolve type reference via symbol table
            if let Some(st) = symbol_table {
                if let Some(alias_entry) = st.lookup_type_alias(name) {
                    return type_annotation_to_type_with_aliases(
                        &alias_entry.type_annotation,
                        symbol_table,
                    );
                }
            }
            types::Type::Unknown
        }
        TypeAnnotation::Union(_) => types::Type::Unknown, // Union types not fully supported yet
        TypeAnnotation::Intersection(types) => {
            // Intersection types merge all fields from component object types
            let mut all_fields = Vec::new();
            for ty in types {
                if let types::Type::Object(fields) =
                    type_annotation_to_type_with_aliases(ty, symbol_table)
                {
                    all_fields.extend(fields);
                }
            }
            if all_fields.is_empty() {
                types::Type::Unknown
            } else {
                types::Type::Object(all_fields)
            }
        }
        TypeAnnotation::Tuple(_) => types::Type::Unknown, // Tuple types not fully supported yet
        TypeAnnotation::Void => types::Type::Unknown,
        TypeAnnotation::Never => types::Type::Error,
        TypeAnnotation::Null => types::Type::Unknown,
        TypeAnnotation::Undefined => types::Type::Unknown,
        TypeAnnotation::Dyn(_) => types::Type::Unknown,
    }
}

/// A warning generated during type checking (not an error)
#[derive(Debug, Clone)]
pub struct TypeWarning {
    /// Warning message
    pub message: String,
    /// Location in source code
    pub span: Span,
}

/// The main semantic analyzer
pub struct SemanticAnalyzer {
    /// Symbol table for tracking patterns, variables, and functions
    symbol_table: SymbolTable,
    /// Type inference engine (replaces legacy TypeChecker)
    inference_engine: TypeInferenceEngine,
    /// Validator for semantic rules
    validator: Validator,
    /// Built-in pattern library for validation
    pattern_library: PatternLibrary,
    /// Source code for error location reporting (optional for backward compatibility)
    source: Option<String>,
    /// Type warnings (not errors, compilation continues)
    warnings: Vec<TypeWarning>,
    /// Track exported symbols for module analysis
    pub(crate) exported_symbols: std::collections::HashSet<String>,
    /// Function names pre-registered for mutual recursion (consumed during main pass)
    pre_registered_functions: std::collections::HashSet<String>,
}

impl SemanticAnalyzer {
    /// Create a new semantic analyzer
    pub fn new() -> Self {
        let mut analyzer = Self {
            symbol_table: SymbolTable::new(),
            inference_engine: TypeInferenceEngine::new(),
            validator: Validator::new(),
            pattern_library: PatternLibrary::new(),
            source: None,
            warnings: Vec::new(),
            exported_symbols: std::collections::HashSet::new(),
            pre_registered_functions: std::collections::HashSet::new(),
        };

        // Register built-in functions
        builtins::register_builtins(&mut analyzer.symbol_table);

        // Register VM-native stdlib modules so their globals are visible
        // during semantic analysis (regex, http, crypto, env, log).
        analyzer.register_stdlib_module_globals();

        analyzer
    }

    /// Set the source code for better error location reporting (builder pattern)
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        let source_str = source.into();
        self.validator.set_source(source_str.clone());
        self.symbol_table.set_source(source_str.clone());
        self.source = Some(source_str);
        self
    }

    /// Set the source code for error location reporting (mutable reference)
    pub fn set_source(&mut self, source: impl Into<String>) {
        let source_str = source.into();
        self.validator.set_source(source_str.clone());
        self.symbol_table.set_source(source_str.clone());
        self.source = Some(source_str);
    }

    /// Check the type of an expression using the inference engine
    ///
    /// This is the main bridge method that replaces the legacy type checker.
    /// It infers the type and converts the result to the legacy Type enum.
    pub fn check_expr_type(&mut self, expr: &Expr) -> Result<types::Type> {
        // Sync symbol table to inference engine before type checking
        self.inference_engine
            .env
            .import_from_symbol_table(&self.symbol_table);
        self.inference_engine
            .sync_callable_defaults_from_symbol_table(&self.symbol_table);

        // Infer the expression type
        match self.inference_engine.infer_expr(expr) {
            Ok(inference_type) => {
                // Convert inference Type to legacy Type
                Ok(types::Type::from_inference_type(&inference_type))
            }
            Err(type_error) => {
                // Convert TypeError to ShapeError
                Err(type_error_to_shape(
                    type_error,
                    self.source.as_deref(),
                    expr.span(),
                ))
            }
        }
    }

    /// Register extension module namespaces with their export type schemas.
    /// Each module becomes a Symbol::Module with a concrete Object type annotation.
    /// Must be called before analyze() so the type system recognizes modules.
    pub fn register_extension_modules(&mut self, modules: &[ParsedModuleSchema]) {
        for module in modules {
            let export_names: Vec<String> =
                module.functions.iter().map(|f| f.name.clone()).collect();
            let type_ann = build_module_type(module);
            let _ = self
                .symbol_table
                .define_module(&module.module_name, export_names, type_ann);
        }
    }

    /// Register the VM-native stdlib modules as known globals.
    ///
    /// These modules (regex, http, crypto, env, log) have full Rust
    /// implementations in `shape_runtime::stdlib` and are auto-registered
    /// in `VirtualMachine::new()`. This method makes them visible to the
    /// semantic analyzer so that `regex.is_match(...)` etc. compile.
    fn register_stdlib_module_globals(&mut self) {
        let modules = crate::module_exports::ModuleExports::stdlib_module_schemas();
        self.register_extension_modules(&modules);
    }

    /// Create a semantic error with location information from a span
    pub fn error_at(&self, span: Span, message: impl Into<String>) -> ShapeError {
        let location = self
            .source
            .as_ref()
            .map(|src| span_to_location(src, span, None));
        ShapeError::SemanticError {
            message: message.into(),
            location,
        }
    }

    /// Create a semantic error with location and a hint
    pub fn error_at_with_hint(
        &self,
        span: Span,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> ShapeError {
        let location = self
            .source
            .as_ref()
            .map(|src| span_to_location(src, span, None).with_hint(hint));
        ShapeError::SemanticError {
            message: message.into(),
            location,
        }
    }

    /// Add a type warning (not an error, analysis continues)
    pub fn add_warning(&mut self, span: Span, message: impl Into<String>) {
        self.warnings.push(TypeWarning {
            message: message.into(),
            span,
        });
    }

    /// Snapshot semantic analyzer state for resumability.
    pub fn snapshot(&self) -> SemanticSnapshot {
        SemanticSnapshot {
            symbol_table: self.symbol_table.clone(),
            exported_symbols: self.exported_symbols.clone(),
        }
    }

    /// Restore semantic analyzer state from a snapshot.
    pub fn restore_from_snapshot(&mut self, snapshot: SemanticSnapshot) {
        self.symbol_table = snapshot.symbol_table;
        self.exported_symbols = snapshot.exported_symbols;
    }

    /// Get all warnings generated during analysis
    pub fn warnings(&self) -> &[TypeWarning] {
        &self.warnings
    }

    /// Take ownership of warnings (clears the internal list)
    pub fn take_warnings(&mut self) -> Vec<TypeWarning> {
        std::mem::take(&mut self.warnings)
    }

    /// Analyze a complete program (script mode - isolated execution)
    ///
    /// Creates a fresh scope for this program and pops it after analysis.
    /// Each call is isolated - no state persists between calls.
    /// Collects all semantic errors and reports them together.
    pub fn analyze(&mut self, program: &Program) -> Result<()> {
        // Push a new scope for user code so it can shadow built-in functions
        self.symbol_table.push_scope();

        // Run optimistic hoisting pre-pass to collect property assignments
        // This enables: let a = {x: 1}; a.y = 2; // y is hoisted
        self.inference_engine.run_hoisting_prepass(program);

        // Pre-register all top-level function signatures before analyzing bodies.
        // This enables mutual recursion: `is_even` can call `is_odd` and vice
        // versa, because both names are in scope when their bodies are checked.
        self.pre_register_functions(program);

        // Main pass: analyze all items, collecting errors
        let mut errors = Vec::new();
        for item in &program.items {
            if let Err(e) = self.analyze_item(item) {
                errors.push(e);
            }
        }

        self.symbol_table.pop_scope();

        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.into_iter().next().unwrap()),
            _ => Err(ShapeError::MultiError(errors)),
        }
    }

    /// Analyze a program incrementally (REPL mode - persistent state)
    ///
    /// Variables and functions defined in previous calls remain visible.
    /// Call `init_repl_scope()` once before the first `analyze_incremental()` call.
    pub fn analyze_incremental(&mut self, program: &Program) -> Result<()> {
        // Run optimistic hoisting pre-pass to collect property assignments
        self.inference_engine.run_hoisting_prepass(program);

        // Main pass: analyze all items
        for item in &program.items {
            self.analyze_item(item)?;
        }

        Ok(())
    }

    /// Initialize the REPL scope (call once before first analyze_incremental)
    ///
    /// Pushes a user scope that will persist across all incremental analyses.
    pub fn init_repl_scope(&mut self) {
        self.symbol_table.push_scope();
        self.symbol_table.set_allow_redefinition(true);
    }

    /// Pre-register all top-level function signatures so that mutual
    /// recursion is possible.  Only names and parameter/return types are
    /// recorded; bodies are NOT analyzed here.
    fn pre_register_functions(&mut self, program: &Program) {
        self.pre_registered_functions.clear();

        // Pre-register struct types so they can be referenced in any order
        for item in &program.items {
            let struct_def = match item {
                Item::StructType(s, _) => s,
                Item::Export(export, _) => {
                    if let shape_ast::ast::ExportItem::Struct(s) = &export.item {
                        s
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            self.inference_engine
                .struct_type_defs
                .insert(struct_def.name.clone(), struct_def.clone());
        }

        for item in &program.items {
            // Extract (name, params, return_type) from regular and foreign functions
            let (name, params, return_type_ann) = match item {
                Item::Function(f, _) => (&f.name, &f.params, &f.return_type),
                Item::ForeignFunction(f, _) => (&f.name, &f.params, &f.return_type),
                Item::Export(export, _) => match &export.item {
                    shape_ast::ast::ExportItem::Function(f) => (&f.name, &f.params, &f.return_type),
                    shape_ast::ast::ExportItem::ForeignFunction(f) => {
                        (&f.name, &f.params, &f.return_type)
                    }
                    _ => continue,
                },
                _ => continue,
            };

            // Skip duplicates — only pre-register the first occurrence.
            // The main pass will detect and report duplicates.
            if self.pre_registered_functions.contains(name) {
                continue;
            }

            let param_types: Vec<types::Type> = params
                .iter()
                .map(|p| {
                    p.type_annotation
                        .as_ref()
                        .map(|ann| {
                            type_annotation_to_type_with_aliases(ann, Some(&self.symbol_table))
                        })
                        .unwrap_or(types::Type::Unknown)
                })
                .collect();

            let return_type = return_type_ann
                .as_ref()
                .map(|ann| type_annotation_to_type_with_aliases(ann, Some(&self.symbol_table)))
                .unwrap_or(types::Type::Unknown);

            let defaults: Vec<bool> = params.iter().map(|p| p.default_value.is_some()).collect();

            // Ignore errors — the main pass will report them properly.
            let _ = self.symbol_table.define_function_with_defaults(
                name,
                param_types,
                return_type,
                defaults,
            );
            self.pre_registered_functions.insert(name.clone());
        }
    }

    /// Analyze a single item
    fn analyze_item(&mut self, item: &Item) -> Result<()> {
        match item {
            Item::Query(query, span) => self.analyze_query(query, *span),
            Item::VariableDecl(var_decl, _) => self.analyze_variable_decl(var_decl),
            Item::Assignment(assignment, _) => self.analyze_assignment(assignment),
            Item::Expression(expr, _) => {
                self.check_expr_type(expr)?;
                Ok(())
            }
            Item::Import(import, _) => self.analyze_import(import),
            Item::Export(export, _) => self.analyze_export(export),
            Item::Module(module_def, _) => {
                // Register the module name as a variable in the current scope
                // so that `module_name.member` access works
                self.symbol_table.define_variable(
                    &module_def.name,
                    types::Type::Unknown,
                    shape_ast::ast::VarKind::Const,
                    true,
                )?;
                self.symbol_table.push_scope();
                let result = (|| {
                    for inner in &module_def.items {
                        self.analyze_item(inner)?;
                    }
                    Ok(())
                })();
                self.symbol_table.pop_scope();
                result
            }
            Item::Function(function, _) => self.analyze_function(function),
            Item::Test(test, _) => self.analyze_test(test),
            Item::TypeAlias(alias, _) => self.analyze_type_alias(alias),
            Item::Interface(interface, _) => self.analyze_interface(interface),
            Item::Trait(trait_def, _) => {
                // Keep semantic inference env in sync so expression checks can
                // resolve trait requirements in the same file.
                self.inference_engine.env.define_trait(trait_def);
                Ok(())
            }
            Item::Enum(enum_def, _) => self.analyze_enum(enum_def),
            Item::Extend(extend_stmt, _) => self.analyze_extend(extend_stmt),
            Item::Impl(impl_block, span) => self.register_trait_impl_metadata(impl_block, *span),
            Item::Stream(stream_def, _) => self.analyze_stream(stream_def),
            Item::Statement(stmt, _) => {
                // Statements at top level are analyzed (e.g., variable declarations)
                self.analyze_statement(stmt)
            }
            Item::Optimize(_opt_stmt, _) => {
                // Optimize statements are validated during AI execution
                Ok(())
            }
            Item::AnnotationDef(_ann_def, _) => {
                // Annotation definitions are registered and processed separately
                Ok(())
            }
            Item::StructType(struct_def, _) => {
                // Register struct type in the inference engine so that
                // expressions like `Currency.symbol` can resolve the type name.
                self.inference_engine
                    .struct_type_defs
                    .insert(struct_def.name.clone(), struct_def.clone());
                Ok(())
            }
            Item::DataSource(_, _) | Item::QueryDecl(_, _) => {
                // Data source and query declarations are registered at compile time
                Ok(())
            }
            Item::Comptime(stmts, _) => {
                // Comptime blocks are evaluated at compile time with dedicated builtins.
                // Register those names in a temporary scope so semantic analysis matches
                // runtime compilation behavior without polluting normal scopes.
                self.symbol_table.push_scope();
                let result = (|| {
                    self.register_comptime_semantic_builtins()?;
                    for stmt in stmts {
                        self.analyze_statement(stmt)?;
                    }
                    Ok(())
                })();
                self.symbol_table.pop_scope();
                result
            }
            Item::BuiltinTypeDecl(_, _) | Item::BuiltinFunctionDecl(_, _) => {
                // Declaration-only intrinsics carry metadata only.
                Ok(())
            }
            Item::ForeignFunction(_, _) => {
                // Foreign function bodies are opaque to Shape semantic analysis.
                Ok(())
            }
        }
    }

    fn type_name_str(name: &TypeName) -> String {
        match name {
            TypeName::Simple(n) => n.clone(),
            TypeName::Generic { name, .. } => name.clone(),
        }
    }

    fn canonical_conversion_name(name: &str) -> String {
        match name {
            "boolean" | "Boolean" | "Bool" => "bool".to_string(),
            "String" => "string".to_string(),
            "Number" => "number".to_string(),
            "Int" => "int".to_string(),
            "Decimal" => "decimal".to_string(),
            _ => name.to_string(),
        }
    }

    fn conversion_name_from_annotation(annotation: &TypeAnnotation) -> Option<String> {
        match annotation {
            TypeAnnotation::Basic(name)
            | TypeAnnotation::Reference(name)
            | TypeAnnotation::Generic { name, .. } => Some(Self::canonical_conversion_name(name)),
            _ => None,
        }
    }

    fn register_trait_impl_metadata(
        &mut self,
        impl_block: &shape_ast::ast::ImplBlock,
        span: Span,
    ) -> Result<()> {
        match &impl_block.trait_name {
            TypeName::Generic { name, type_args } if name == "TryInto" || name == "Into" => {
                if type_args.len() != 1 {
                    return Err(self.error_at(
                        span,
                        format!(
                            "{} impl must declare exactly one target: `impl {}<Target> for Source as target`",
                            name, name
                        ),
                    ));
                }
                let target =
                    Self::conversion_name_from_annotation(&type_args[0]).ok_or_else(|| {
                        self.error_at(
                            span,
                            format!("{} target must be a concrete named type", name),
                        )
                    })?;
                let selector = impl_block.impl_name.as_deref().ok_or_else(|| {
                    self.error_at(
                        span,
                        format!("{} impl must declare named selector with `as target`", name),
                    )
                })?;
                let selector = Self::canonical_conversion_name(selector);
                if selector != target {
                    return Err(self.error_at(
                        span,
                        format!(
                            "{} target `{}` must match impl selector `{}`",
                            name, target, selector
                        ),
                    ));
                }
            }
            TypeName::Simple(name) if name == "TryInto" || name == "Into" => {
                return Err(self.error_at(
                    span,
                    format!(
                        "{} impl must use generic target form: `impl {}<Target> for Source as target`",
                        name, name
                    ),
                ));
            }
            _ => {}
        }

        let trait_name = Self::type_name_str(&impl_block.trait_name);
        let target_type = Self::type_name_str(&impl_block.target_type);
        let method_names = impl_block.methods.iter().map(|m| m.name.clone()).collect();
        let associated_types = impl_block
            .associated_type_bindings
            .iter()
            .map(|binding| (binding.name.clone(), binding.concrete_type.clone()))
            .collect();

        self.inference_engine
            .env
            .register_trait_impl_with_assoc_types_named(
                &trait_name,
                &target_type,
                impl_block.impl_name.as_deref(),
                method_names,
                associated_types,
            )
            .map_err(|msg| self.error_at(span, msg))
    }

    /// Look up a type alias (for runtime type resolution)
    pub fn lookup_type_alias(
        &self,
        name: &str,
    ) -> Option<&crate::type_system::environment::TypeAliasEntry> {
        self.symbol_table.lookup_type_alias(name)
    }

    fn register_comptime_semantic_builtins(&mut self) -> Result<()> {
        use crate::semantic::types::Type;

        self.symbol_table.define_function(
            "build_config",
            vec![],
            Type::Object(vec![
                ("debug".to_string(), Type::Unknown),
                ("target_arch".to_string(), Type::Unknown),
                ("target_os".to_string(), Type::Unknown),
                ("version".to_string(), Type::Unknown),
            ]),
        )?;
        self.symbol_table.define_function(
            "implements",
            vec![Type::Unknown, Type::Unknown],
            Type::Bool,
        )?;
        self.symbol_table
            .define_function("warning", vec![Type::Unknown], Type::Unknown)?;
        self.symbol_table
            .define_function("error", vec![Type::Unknown], Type::Unknown)?;

        Ok(())
    }
}

/// Build a module's object type from its function schemas.
/// Each export becomes an ObjectTypeField with a Function type annotation.
fn build_module_type(schema: &ParsedModuleSchema) -> TypeAnnotation {
    let fields: Vec<ObjectTypeField> = schema
        .functions
        .iter()
        .map(|function| {
            let params: Vec<FunctionParam> = function
                .params
                .iter()
                .enumerate()
                .map(|(idx, p)| FunctionParam {
                    name: Some(format!("arg{}", idx)),
                    // Module function params are marked optional since
                    // ParsedModuleSchema doesn't carry required/optional info.
                    // Runtime arg validation handles arity checks.
                    optional: true,
                    type_annotation: schema_type_to_annotation(p),
                })
                .collect();

            let returns = function
                .return_type
                .as_ref()
                .map(|r| schema_type_to_annotation(r))
                .unwrap_or(TypeAnnotation::Basic("unknown".to_string()));

            ObjectTypeField {
                name: function.name.clone(),
                optional: false,
                type_annotation: TypeAnnotation::Function {
                    params,
                    returns: Box::new(returns),
                },
                annotations: vec![],
            }
        })
        .collect();

    TypeAnnotation::Object(fields)
}

/// Convert a schema type name string to a TypeAnnotation.
fn schema_type_to_annotation(type_name: &str) -> TypeAnnotation {
    TypeAnnotation::Basic(type_name.to_string())
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
