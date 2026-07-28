//! Span-free indexes derived from parsed syntax, plus the separate provenance
//! index that carries spans.
//!
//! The split is what makes early cutoff real rather than decorative. A body
//! edit or a comment changes the parse tree, but the declaration index it
//! produces compares equal, so Salsa backdates it and nothing downstream of
//! meaning re-executes. Spans live in [`UnitProvenance`], which is allowed to
//! change without disturbing any contract.

use shape_ast::ast::functions::FunctionDef;
use shape_ast::ast::modules::{ExportItem, ImportItems};
use shape_ast::ast::program::{Item, Program};
use shape_ast::ast::span::Span;
use shape_ast::ast::statements::Statement;
use shape_ast::ast::types::TypeParam;
use shape_ast::ast::{Expr, Literal};

use crate::diagnostics::{DiagnosticSeverity, SemanticDiagnostic, codes};
use crate::facts::{CallableContract, ParamContract, Visibility};
use crate::types::NormalizedType;

/// A declaration as published by the index: name, disambiguating ordinal,
/// visibility and normalized base contract. No spans, no body.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CallableDeclaration {
    pub name: String,
    pub same_name_ordinal: u32,
    pub visibility: Visibility,
    pub contract: CallableContract,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

/// One `from <unit> use { name as local }` binding.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ImportBinding {
    pub local_name: String,
    pub from_unit: String,
    pub exported_name: String,
}

/// A supported call-site occurrence, with the statically determined type of
/// each argument. `None` means "this slice does not determine that type" —
/// published as an explicit gap, never guessed.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CallSiteSyntax {
    pub occurrence: u32,
    pub written_name: String,
    pub argument_types: Vec<Option<NormalizedType>>,
    pub named_argument_count: usize,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct DeclarationIndex {
    pub unit_path: String,
    pub callables: Vec<CallableDeclaration>,
    pub imports: Vec<ImportBinding>,
    pub call_sites: Vec<CallSiteSyntax>,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

impl DeclarationIndex {
    pub fn callable(&self, name: &str, ordinal: u32) -> Option<&CallableDeclaration> {
        self.callables
            .iter()
            .find(|decl| decl.name == name && decl.same_name_ordinal == ordinal)
    }

    /// The declaration a bare name binds to inside this unit: the first
    /// declaration with that name in canonical lexical order.
    pub fn local_binding(&self, name: &str) -> Option<&CallableDeclaration> {
        self.callable(name, 0)
    }

    pub fn import(&self, local_name: &str) -> Option<&ImportBinding> {
        self.imports
            .iter()
            .find(|binding| binding.local_name == local_name)
    }
}

/// Spans for one unit, keyed by the same structural keys the index uses.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct UnitProvenance {
    /// `(name, same_name_ordinal) -> (declaration span, name span)`
    pub declarations: Vec<((String, u32), Span, Span)>,
    /// Span of each call site, by occurrence ordinal.
    pub call_sites: Vec<Span>,
}

impl UnitProvenance {
    pub fn declaration(&self, name: &str, ordinal: u32) -> Option<(Span, Span)> {
        self.declarations
            .iter()
            .find(|((decl_name, decl_ordinal), _, _)| decl_name == name && *decl_ordinal == ordinal)
            .map(|(_, declaration, name_span)| (*declaration, *name_span))
    }

    pub fn call_site(&self, occurrence: u32) -> Option<Span> {
        self.call_sites.get(occurrence as usize).copied()
    }
}

/// Builds the span-free index for one unit.
pub fn build_index(unit_path: &str, program: &Program) -> DeclarationIndex {
    let mut index = DeclarationIndex {
        unit_path: unit_path.to_string(),
        ..Default::default()
    };
    let mut seen_names: Vec<(String, u32)> = Vec::new();

    for item in &program.items {
        match item {
            Item::Function(function, _) => {
                push_callable(&mut index, &mut seen_names, function, Visibility::Private);
            }
            Item::Export(export, _) => {
                if let ExportItem::Function(function) = &export.item {
                    push_callable(&mut index, &mut seen_names, function, Visibility::Public);
                }
            }
            Item::Import(import, _) => push_imports(&mut index, import),
            _ => {}
        }
    }

    for (name, count) in duplicate_counts(&seen_names) {
        index.diagnostics.push(SemanticDiagnostic::new(
            codes::DUPLICATE_CALLABLE_DECLARATION,
            DiagnosticSeverity::Warning,
            [
                ("name", name),
                ("unit", unit_path.to_string()),
                ("count", count.to_string()),
            ],
        ));
    }
    index.diagnostics.sort();

    index.call_sites = collect_call_sites(program).0;
    index
}

/// Builds the span index for one unit, using the same traversal order as
/// [`build_index`] so the structural keys line up.
pub fn build_provenance(program: &Program) -> UnitProvenance {
    let mut provenance = UnitProvenance::default();
    let mut seen_names: Vec<(String, u32)> = Vec::new();

    for item in &program.items {
        let (function, span) = match item {
            Item::Function(function, span) => (function, *span),
            Item::Export(export, span) => match &export.item {
                ExportItem::Function(function) => (function, *span),
                _ => continue,
            },
            _ => continue,
        };
        let ordinal = next_ordinal(&mut seen_names, &function.name);
        provenance
            .declarations
            .push(((function.name.clone(), ordinal), span, function.name_span));
    }

    provenance.call_sites = collect_call_sites(program).1;
    provenance
}

fn push_callable(
    index: &mut DeclarationIndex,
    seen_names: &mut Vec<(String, u32)>,
    function: &FunctionDef,
    visibility: Visibility,
) {
    let ordinal = next_ordinal(seen_names, &function.name);
    let (contract, diagnostics) = normalize_contract(function);
    index.callables.push(CallableDeclaration {
        name: function.name.clone(),
        same_name_ordinal: ordinal,
        visibility,
        contract,
        diagnostics,
    });
}

fn next_ordinal(seen_names: &mut Vec<(String, u32)>, name: &str) -> u32 {
    match seen_names.iter_mut().find(|(seen, _)| seen == name) {
        Some((_, count)) => {
            *count += 1;
            *count
        }
        None => {
            seen_names.push((name.to_string(), 0));
            0
        }
    }
}

fn duplicate_counts(seen_names: &[(String, u32)]) -> Vec<(String, u32)> {
    seen_names
        .iter()
        .filter(|(_, highest)| *highest > 0)
        .map(|(name, highest)| (name.clone(), highest + 1))
        .collect()
}

fn push_imports(index: &mut DeclarationIndex, import: &shape_ast::ast::modules::ImportStmt) {
    match &import.items {
        ImportItems::Named(specs) => {
            for spec in specs {
                // Annotation imports are outside this slice's stop line.
                if spec.is_annotation {
                    continue;
                }
                index.imports.push(ImportBinding {
                    local_name: spec.alias.clone().unwrap_or_else(|| spec.name.clone()),
                    from_unit: import.from.clone(),
                    exported_name: spec.name.clone(),
                });
            }
        }
        // Namespace imports bind a module, not a callable: qualified call
        // resolution is a later slice.
        ImportItems::Namespace { .. } => {}
    }
}

/// Normalizes a function declaration's *base* contract (ADR-011 §2). Annotation
/// contributions are outside the stop line and are not consulted.
fn normalize_contract(function: &FunctionDef) -> (CallableContract, Vec<SemanticDiagnostic>) {
    let mut diagnostics = Vec::new();

    let type_params = function
        .type_params
        .as_ref()
        .map(|params| {
            params
                .iter()
                .map(|param| match param {
                    TypeParam::Type { name, .. } => name.clone(),
                    TypeParam::Const { name, .. } => format!("const {name}"),
                    // ADR-014 §8.3: an effect binder is published as part of
                    // the generic schema, exactly as a type binder is.
                    TypeParam::Effect { name, .. } => format!("effect {name}"),
                })
                .collect()
        })
        .unwrap_or_default();

    let mut params = Vec::new();
    for (index, param) in function.params.iter().enumerate() {
        let name = match param.simple_name() {
            Some(name) => name.to_string(),
            None => {
                diagnostics.push(SemanticDiagnostic::note(
                    codes::PARAMETER_PATTERN_NOT_SUPPORTED,
                    [
                        ("index", index.to_string()),
                        ("name", function.name.clone()),
                    ],
                ));
                format!("#{index}")
            }
        };
        let ty = NormalizedType::from_annotation(param.type_annotation.as_ref());
        if ty == NormalizedType::NotDeclared {
            diagnostics.push(SemanticDiagnostic::note(
                codes::PARAMETER_TYPE_NOT_DECLARED,
                [("param", name.clone()), ("name", function.name.clone())],
            ));
        }
        params.push(ParamContract {
            name,
            ty,
            by_reference: param.is_reference,
            mutable_reference: param.is_mut_reference,
            is_const: param.is_const,
            has_default: param.default_value.is_some(),
        });
    }

    let result = NormalizedType::from_annotation(function.return_type.as_ref());
    if result == NormalizedType::NotDeclared {
        diagnostics.push(SemanticDiagnostic::note(
            codes::RESULT_TYPE_NOT_DECLARED,
            [("name", function.name.clone())],
        ));
    }

    diagnostics.sort();
    (
        CallableContract {
            type_params,
            params,
            result,
            is_async: function.is_async,
            is_comptime: function.is_comptime,
        },
        diagnostics,
    )
}

/// Collects call sites in the forms this slice supports, in source order.
///
/// The traversal is deliberately partial and non-exhaustive over `Expr`: this
/// slice publishes facts for the declared forms below and nothing else, so a
/// new AST variant must not silently break it. A call written in an
/// unsupported form is simply not published — it is never published with
/// approximated facts.
///
/// Supported: unit-level expressions and variable initializers, function-body
/// expression statements, variable initializers and returns, and calls nested
/// inside binary/unary operands and call arguments.
fn collect_call_sites(program: &Program) -> (Vec<CallSiteSyntax>, Vec<Span>) {
    let mut collector = CallSiteCollector::default();
    for item in &program.items {
        match item {
            Item::Function(function, _) => collector.walk_body(&function.body),
            Item::Export(export, _) => {
                if let ExportItem::Function(function) = &export.item {
                    collector.walk_body(&function.body);
                }
            }
            Item::Expression(expr, _) => collector.walk_expr(expr),
            Item::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    collector.walk_expr(value);
                }
            }
            Item::Statement(statement, _) => collector.walk_statement(statement),
            _ => {}
        }
    }
    (collector.sites, collector.spans)
}

#[derive(Default)]
struct CallSiteCollector {
    sites: Vec<CallSiteSyntax>,
    spans: Vec<Span>,
}

impl CallSiteCollector {
    fn walk_body(&mut self, body: &[Statement]) {
        for statement in body {
            self.walk_statement(statement);
        }
    }

    fn walk_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Expression(expr, _) => self.walk_expr(expr),
            Statement::Return(Some(expr), _) => self.walk_expr(expr),
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    self.walk_expr(value);
                }
            }
            _ => {}
        }
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::FunctionCall {
                name,
                args,
                named_args,
                span,
                ..
            } => {
                let occurrence = self.sites.len() as u32;
                self.sites.push(CallSiteSyntax {
                    occurrence,
                    written_name: name.clone(),
                    argument_types: args.iter().map(literal_type).collect(),
                    named_argument_count: named_args.len(),
                });
                self.spans.push(*span);
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::UnaryOp { operand, .. } => self.walk_expr(operand),
            _ => {}
        }
    }
}

/// The type of a literal argument, or `None` when this slice does not
/// statically determine it.
fn literal_type(expr: &Expr) -> Option<NormalizedType> {
    match expr {
        Expr::Literal(literal, _) => match literal {
            Literal::Int(_) | Literal::UInt(_) | Literal::TypedInt(_, _) => {
                Some(NormalizedType::Int)
            }
            Literal::Number(_) => Some(NormalizedType::Number),
            Literal::Decimal(_) => Some(NormalizedType::Decimal),
            Literal::String(_) | Literal::FormattedString { .. } => Some(NormalizedType::String),
            Literal::Bool(_) => Some(NormalizedType::Bool),
            Literal::Unit => Some(NormalizedType::Void),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parse_program;

    fn index_of(source: &str) -> DeclarationIndex {
        let program = parse_program(source).expect("tracer source parses");
        build_index("app::test", &program)
    }

    #[test]
    fn indexes_the_tracer_declaration_and_call_site() {
        let index = index_of("fn add(a: int, b: int) -> int { a + b }\nlet total = add(1, 2)\n");
        let declaration = index.local_binding("add").expect("add is declared");
        assert_eq!(declaration.contract.params.len(), 2);
        assert_eq!(declaration.contract.params[0].ty, NormalizedType::Int);
        assert_eq!(declaration.contract.result, NormalizedType::Int);
        assert_eq!(declaration.visibility, Visibility::Private);
        assert_eq!(index.call_sites.len(), 1);
        assert_eq!(index.call_sites[0].written_name, "add");
        assert_eq!(
            index.call_sites[0].argument_types,
            vec![Some(NormalizedType::Int), Some(NormalizedType::Int)]
        );
    }

    #[test]
    fn pub_fn_is_indexed_as_public() {
        let index = index_of("pub fn add(a: int, b: int) -> int { a + b }\n");
        assert_eq!(
            index.local_binding("add").unwrap().visibility,
            Visibility::Public
        );
    }

    #[test]
    fn import_alias_binds_the_local_name_to_the_exported_name() {
        let index = index_of("from app::math use { add as plus }\nlet total = plus(1, 2)\n");
        let binding = index.import("plus").expect("alias binding exists");
        assert_eq!(binding.exported_name, "add");
        assert_eq!(binding.from_unit, "app::math");
        assert!(index.import("add").is_none());
    }

    #[test]
    fn body_edits_do_not_change_the_index() {
        let before = index_of("fn add(a: int, b: int) -> int { a + b }\n");
        let after = index_of("fn add(a: int, b: int) -> int { b + a }\n");
        assert_eq!(before, after);
    }

    #[test]
    fn comments_do_not_change_the_index() {
        let before = index_of("fn add(a: int, b: int) -> int { a + b }\n");
        let after =
            index_of("// leading comment\nfn add(a: int, b: int) -> int { a + b }\n// trailing\n");
        assert_eq!(before, after);
    }

    #[test]
    fn signature_edits_do_change_the_index() {
        let before = index_of("fn add(a: int, b: int) -> int { a + b }\n");
        let after = index_of("fn add(a: string, b: string) -> string { a + b }\n");
        assert_ne!(before, after);
    }

    #[test]
    fn leading_comment_shifts_provenance_but_not_the_index() {
        let plain = parse_program("fn add(a: int, b: int) -> int { a + b }\n").unwrap();
        let commented =
            parse_program("// note\nfn add(a: int, b: int) -> int { a + b }\n").unwrap();
        assert_eq!(
            build_index("app::test", &plain),
            build_index("app::test", &commented)
        );
        assert_ne!(build_provenance(&plain), build_provenance(&commented));
    }

    #[test]
    fn same_name_declarations_receive_distinct_ordinals() {
        let index = index_of("fn add(a: int) -> int { a }\nfn add(a: string) -> string { a }\n");
        assert_eq!(index.callables.len(), 2);
        assert_eq!(index.callables[0].same_name_ordinal, 0);
        assert_eq!(index.callables[1].same_name_ordinal, 1);
        assert!(
            index
                .diagnostics
                .iter()
                .any(|d| d.code == codes::DUPLICATE_CALLABLE_DECLARATION)
        );
    }

    #[test]
    fn unrelated_sibling_insertion_does_not_renumber_the_tracer() {
        let before = index_of("fn add(a: int, b: int) -> int { a + b }\n");
        let after = index_of(
            "fn unrelated() -> int { 1 }\nfn add(a: int, b: int) -> int { a + b }\nfn other() -> int { 2 }\n",
        );
        assert_eq!(
            before.local_binding("add").unwrap().same_name_ordinal,
            after.local_binding("add").unwrap().same_name_ordinal
        );
        assert_eq!(
            before.local_binding("add").unwrap().contract,
            after.local_binding("add").unwrap().contract
        );
    }
}
