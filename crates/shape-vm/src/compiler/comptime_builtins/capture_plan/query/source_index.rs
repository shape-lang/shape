//! Exact authored-source mapping for generated capture descriptors.
//!
//! A compiler-issued application anchor and generated-symbol node path first
//! select one generator definition and one direct typed `extend` method. The
//! canonical AST stamper then selects the exact closure path. Spans validate
//! that structural selection; they never select a source node globally.

use std::collections::HashSet;

use shape_ast::ast::{
    AnnotationDef, DestructurePattern, ExportItem, Expr, FunctionDef, FunctionParameter, Item,
    Pattern, Program, Span, Statement, TypeName,
};
use shape_ast::transform::{GeneratedClosureSourcePath, generated_closure_source_paths};
use shape_runtime::closure::EnvironmentAnalyzer;
use shape_runtime::visitor::{Visitor, walk_expr, walk_program, walk_stmt};

use crate::compiler::{BytecodeCompiler, SourceAnchor};

use super::super::CaptureDescriptor;
use super::GeneratedCaptureSourceMap;

#[derive(Debug, Clone)]
struct DirectGeneratedMethod {
    generator_span: Span,
    target_parameter: Option<String>,
    extended_type: String,
    name: String,
    params: Vec<FunctionParameter>,
    body: Vec<Statement>,
}

#[derive(Default)]
pub(super) struct AuthoredCaptureIndex {
    methods: Vec<DirectGeneratedMethod>,
}

impl AuthoredCaptureIndex {
    pub(super) fn build(program: &Program) -> Self {
        let mut index = Self::default();
        walk_program(
            &mut GeneratorCarrierCollector {
                methods: &mut index.methods,
            },
            program,
        );
        index
    }

    pub(super) fn source_map_for(
        &self,
        compiler: &BytecodeCompiler,
        origin: &shape_ast::ast::GeneratedNodeOrigin,
        descriptor: &CaptureDescriptor,
    ) -> Option<GeneratedCaptureSourceMap> {
        let (file_id, application_span) = origin.anchor();
        let application = SourceAnchor::new(file_id, application_span).ok()?;
        let generated_symbol = unique_enclosing_symbol(compiler, application, origin.node_path())?;
        let declaration_path = generated_symbol.node_path.segments();
        let generator = generated_symbol.generator;
        if generator.file_id() != file_id {
            return None;
        }

        let method = unique_direct_method(&self.methods, generator.span(), declaration_path)?;
        let closure = unique_closure_source(method, declaration_path, origin.node_path())?;
        validate_source_map(file_id, method, &closure, descriptor)
    }
}

struct GeneratorCarrierCollector<'index> {
    methods: &'index mut Vec<DirectGeneratedMethod>,
}

impl GeneratorCarrierCollector<'_> {
    fn collect_annotation(&mut self, definition: &AnnotationDef) {
        for handler in &definition.handlers {
            let mut collector = DirectExtendCollector {
                generator_span: handler.span,
                target_parameter: handler.params.first().map(|param| param.name.clone()),
                methods: &mut *self.methods,
            };
            walk_expr(&mut collector, &handler.body);
        }
    }

    fn collect_comptime(&mut self, statements: &[Statement], span: Span) {
        let mut collector = DirectExtendCollector {
            generator_span: span,
            target_parameter: None,
            methods: &mut *self.methods,
        };
        for statement in statements {
            walk_stmt(&mut collector, statement);
        }
    }
}

impl Visitor for GeneratorCarrierCollector<'_> {
    fn visit_item(&mut self, item: &Item) -> bool {
        match item {
            Item::AnnotationDef(definition, _) => self.collect_annotation(definition),
            Item::Export(export, _) => {
                if let ExportItem::Annotation(definition) = &export.item {
                    self.collect_annotation(definition);
                }
            }
            Item::Comptime(statements, span) => self.collect_comptime(statements, *span),
            _ => {}
        }
        true
    }

    fn visit_expr_comptime(&mut self, expression: &Expr, span: Span) -> bool {
        if let Expr::Comptime(statements, _) = expression {
            self.collect_comptime(statements, span);
        }
        true
    }
}

struct DirectExtendCollector<'index> {
    generator_span: Span,
    target_parameter: Option<String>,
    methods: &'index mut Vec<DirectGeneratedMethod>,
}

impl Visitor for DirectExtendCollector<'_> {
    fn visit_expr_comptime(&mut self, _expression: &Expr, _span: Span) -> bool {
        // A nested comptime expression is an independent generator carrier.
        false
    }

    fn visit_stmt(&mut self, statement: &Statement) -> bool {
        let Statement::Extend(extension, _) = statement else {
            return true;
        };
        let extended_type = type_name_spelling(&extension.type_name).to_string();
        self.methods.extend(
            extension
                .methods
                .iter()
                .map(|method| DirectGeneratedMethod {
                    generator_span: self.generator_span,
                    target_parameter: self.target_parameter.clone(),
                    extended_type: extended_type.clone(),
                    name: method.name.clone(),
                    params: method.params.clone(),
                    body: method.body.clone(),
                }),
        );
        // The generated method body is output, not another generator body.
        false
    }
}

fn type_name_spelling(type_name: &TypeName) -> &str {
    match type_name {
        TypeName::Simple(name) | TypeName::Generic { name, .. } => name.as_str(),
    }
}

fn unique_enclosing_symbol<'compiler>(
    compiler: &'compiler BytecodeCompiler,
    application: SourceAnchor,
    capture_path: &[String],
) -> Option<crate::compiler::GeneratedSymbolProvenance<'compiler>> {
    let mut matches = compiler
        .generated_symbol_query()
        .generated_symbols()
        .into_iter()
        .filter(|symbol| {
            symbol.application == application
                && capture_path.starts_with(symbol.node_path.segments())
        });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn unique_direct_method<'index>(
    methods: &'index [DirectGeneratedMethod],
    generator_span: Span,
    declaration_path: &[String],
) -> Option<&'index DirectGeneratedMethod> {
    let [extend_segment, method_segment] = declaration_path else {
        return None;
    };
    let generated_type = extend_segment.strip_prefix("extend:")?;
    let generated_method = method_segment.strip_prefix("method:")?;
    let mut matches = methods.iter().filter(|method| {
        method.generator_span == generator_span
            && method.name == generated_method
            && (method.extended_type == generated_type
                || method.target_parameter.as_deref() == Some(method.extended_type.as_str()))
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn unique_closure_source(
    method: &DirectGeneratedMethod,
    declaration_path: &[String],
    capture_path: &[String],
) -> Option<GeneratedClosureSourcePath> {
    let mut matches = generated_closure_source_paths(&method.body, declaration_path)
        .into_iter()
        .filter(|source| source.node_path == capture_path);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn validate_source_map(
    file_id: u16,
    method: &DirectGeneratedMethod,
    closure: &GeneratedClosureSourcePath,
    descriptor: &CaptureDescriptor,
) -> Option<GeneratedCaptureSourceMap> {
    let binding_span = descriptor.binding_span?;
    let declaration_span = descriptor.declaration_span?;
    if descriptor.use_spans.is_empty()
        || !method_has_binding(method, &descriptor.name, binding_span)
        || !closure_has_declaration(closure, descriptor, declaration_span)
        || closure_use_spans(closure, &descriptor.name) != normalized_spans(&descriptor.use_spans)
    {
        return None;
    }

    let binding = SourceAnchor::new(file_id, binding_span).ok()?;
    let declaration = SourceAnchor::new(file_id, declaration_span).ok()?;
    let uses = normalized_spans(&descriptor.use_spans)
        .into_iter()
        .map(|span| SourceAnchor::new(file_id, span))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(GeneratedCaptureSourceMap {
        binding,
        declaration,
        uses,
    })
}

fn method_has_binding(method: &DirectGeneratedMethod, name: &str, span: Span) -> bool {
    let mut bindings = BindingIndex::default();
    for param in &method.params {
        bindings.destructure_binding(&param.pattern);
    }
    for statement in &method.body {
        walk_stmt(&mut bindings, statement);
    }
    bindings.bindings.contains(&(name.to_string(), span))
}

fn closure_has_declaration(
    closure: &GeneratedClosureSourcePath,
    descriptor: &CaptureDescriptor,
    declaration_span: Span,
) -> bool {
    closure.captures.as_ref().is_some_and(|clause| {
        clause
            .entries
            .iter()
            .filter(|entry| {
                entry.name == descriptor.name
                    && Some(entry.mode) == descriptor.declared
                    && entry.name_span == declaration_span
            })
            .count()
            == 1
    })
}

fn closure_use_spans(closure: &GeneratedClosureSourcePath, name: &str) -> Vec<Span> {
    let function = FunctionDef {
        name: "<generated-capture-source-map>".to_string(),
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        type_params: None,
        params: closure.params.clone(),
        return_type: None,
        where_clause: None,
        body: closure.body.clone(),
        annotations: Vec::new(),
        is_async: false,
        is_comptime: false,
        effect_row: None,
    };
    let outer_vars = [name.to_string()];
    let analysis = EnvironmentAnalyzer::analyze_function_captures(&function, &outer_vars);
    normalized_spans(analysis.use_spans(name))
}

fn normalized_spans(spans: &[Span]) -> Vec<Span> {
    let mut spans = spans.to_vec();
    spans.sort_by_key(|span| (span.start, span.end));
    spans.dedup();
    spans
}

#[derive(Default)]
struct BindingIndex {
    bindings: HashSet<(String, Span)>,
}

impl BindingIndex {
    fn destructure_binding(&mut self, pattern: &DestructurePattern) {
        self.bindings.extend(pattern.get_bindings());
    }

    fn pattern_binding(&mut self, pattern: &Pattern) {
        self.bindings.extend(pattern.get_bindings());
    }
}

impl Visitor for BindingIndex {
    fn visit_stmt(&mut self, statement: &Statement) -> bool {
        match statement {
            Statement::VariableDecl(declaration, _) => {
                self.destructure_binding(&declaration.pattern)
            }
            Statement::For(for_loop, _) => {
                if let shape_ast::ast::ForInit::ForIn { pattern, .. } = &for_loop.init {
                    self.destructure_binding(pattern);
                }
            }
            Statement::Extend(extension, _) => {
                for method in &extension.methods {
                    for param in &method.params {
                        self.destructure_binding(&param.pattern);
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn visit_expr(&mut self, expression: &Expr) -> bool {
        match expression {
            Expr::FunctionExpr { params, .. } => {
                for param in params {
                    self.destructure_binding(&param.pattern);
                }
            }
            Expr::For(for_expr, _) => self.pattern_binding(&for_expr.pattern),
            Expr::Let(let_expr, _) => self.pattern_binding(&let_expr.pattern),
            Expr::Match(match_expr, _) => {
                for arm in &match_expr.arms {
                    self.pattern_binding(&arm.pattern);
                }
            }
            Expr::ListComprehension(comprehension, _) => {
                for clause in &comprehension.clauses {
                    self.destructure_binding(&clause.pattern);
                }
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
#[path = "source_index_tests.rs"]
mod tests;
