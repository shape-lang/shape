//! Conservative structural reachability for executable generation producers.

use shape_ast::ast::{
    Annotation, ExportItem, Expr, Item, MethodDef, Program, StructTypeDef, TraitDef, TraitMember,
    TypeParam,
};
use shape_runtime::visitor::{Visitor, walk_expr, walk_program, walk_stmt};

/// Whether the program's item tree can reach an executable generation
/// producer on any existing semantic compilation path.
///
/// This is deliberately a narrow structural walk, not an evaluator. A false
/// result proves that semantic compilation cannot generate. A true result only
/// delegates that decision to the compiler: module annotations and raw module
/// `comptime` blocks remain true here even though they mutate topology through
/// separate pass-2 APIs rather than the Decision-67 fixed point.
pub fn program_may_generate(program: &Program) -> bool {
    let mut visitor = GenerationReachability::default();
    walk_program(&mut visitor, program);
    visitor.reachable
}

#[derive(Default)]
struct GenerationReachability {
    reachable: bool,
}

impl Visitor for GenerationReachability {
    fn visit_item(&mut self, item: &Item) -> bool {
        self.reachable |= item_is_declaration_producer(item);
        if self.reachable {
            return false;
        }

        // shape_runtime's general item walker deliberately omits declaration
        // interiors that do not normally participate in expression analysis.
        // Reachability cannot omit them: defaults and method bodies can contain
        // executable annotated/comptime expressions.
        match item {
            Item::TypeAlias(def, _) => {
                walk_type_params(self, def.type_params.as_deref());
                for value in def
                    .meta_param_overrides
                    .iter()
                    .flat_map(|values| values.values())
                {
                    walk_expr(self, value);
                }
            }
            Item::Trait(def, _) => walk_trait(self, def),
            Item::Enum(def, _) => {
                walk_type_params(self, def.type_params.as_deref());
                walk_annotations(self, &def.annotations);
            }
            Item::Extend(def, _) => walk_method_omissions(self, &def.methods),
            Item::Impl(def, _) => walk_method_omissions(self, &def.methods),
            Item::Function(def, _) => {
                walk_type_params(self, def.type_params.as_deref());
                walk_annotations(self, &def.annotations);
            }
            Item::Module(def, _) => walk_annotations(self, &def.annotations),
            Item::Export(export, _) => {
                if let Some(value) = export
                    .source_decl
                    .as_ref()
                    .and_then(|decl| decl.value.as_ref())
                {
                    walk_expr(self, value);
                }
                walk_export_omissions(self, &export.item);
            }
            Item::AnnotationDef(def, _) => {
                walk_parameter_defaults(self, &def.params);
            }
            Item::StructType(def, _) => walk_struct(self, def),
            Item::ForeignFunction(def, _) => {
                walk_type_params(self, def.type_params.as_deref());
                walk_parameter_defaults(self, &def.params);
                walk_annotations(self, &def.annotations);
            }
            Item::Import(..)
            | Item::Query(..)
            | Item::VariableDecl(..)
            | Item::Assignment(..)
            | Item::Expression(..)
            | Item::Stream(..)
            | Item::Test(..)
            | Item::Optimize(..)
            | Item::DataSource(..)
            | Item::QueryDecl(..)
            | Item::Statement(..)
            | Item::Comptime(..)
            | Item::BuiltinTypeDecl(..)
            | Item::BuiltinFunctionDecl(..) => {}
        }
        !self.reachable
    }

    fn visit_expr(&mut self, expr: &Expr) -> bool {
        self.reachable |= expression_is_generation_producer(expr);
        !self.reachable
    }

    fn visit_expr_type_assertion(
        &mut self,
        expr: &Expr,
        _span: shape_ast::ast::Span,
    ) -> bool {
        if let Expr::TypeAssertion {
            meta_param_overrides: Some(overrides),
            ..
        } = expr
        {
            for value in overrides.values() {
                walk_expr(self, value);
            }
        }
        !self.reachable
    }
}

fn item_is_declaration_producer(item: &Item) -> bool {
    match item {
        Item::Comptime(..) => true,
        Item::Function(def, _) => !def.is_comptime && !def.annotations.is_empty(),
        Item::StructType(def, _) => !def.annotations.is_empty(),
        Item::Trait(def, _) => trait_default_may_generate(def),
        Item::Extend(def, _) => methods_may_generate(&def.methods),
        // The collector remains authoritative about which impl targets it can
        // materialize. This guard intentionally over-approximates all ordinary
        // impl annotations without interpreting raw trait-name strings.
        Item::Impl(def, _) => !def.is_comptime && methods_may_generate(&def.methods),
        Item::Module(def, _) => !def.annotations.is_empty(),
        Item::Export(export, _) => match &export.item {
            ExportItem::Function(def) => !def.is_comptime && !def.annotations.is_empty(),
            ExportItem::Struct(def) => !def.annotations.is_empty(),
            ExportItem::Trait(def) => trait_default_may_generate(def),
            ExportItem::BuiltinFunction(_)
            | ExportItem::BuiltinType(_)
            | ExportItem::TypeAlias(_)
            | ExportItem::Named(_)
            | ExportItem::Enum(_)
            | ExportItem::Annotation(_)
            | ExportItem::ForeignFunction(_) => false,
        },
        Item::Import(..)
        | Item::TypeAlias(..)
        | Item::Enum(..)
        | Item::Query(..)
        | Item::VariableDecl(..)
        | Item::Assignment(..)
        | Item::Expression(..)
        | Item::Stream(..)
        | Item::Test(..)
        | Item::Optimize(..)
        | Item::AnnotationDef(..)
        | Item::DataSource(..)
        | Item::QueryDecl(..)
        | Item::Statement(..)
        | Item::BuiltinTypeDecl(..)
        | Item::BuiltinFunctionDecl(..)
        | Item::ForeignFunction(..) => false,
    }
}

fn expression_is_generation_producer(expr: &Expr) -> bool {
    match expr {
        Expr::Annotated { .. } | Expr::Comptime(..) | Expr::ComptimeFor(..) => true,
        Expr::Literal(..)
        | Expr::Identifier(..)
        | Expr::DataRef(..)
        | Expr::DataDateTimeRef(..)
        | Expr::DataRelativeAccess { .. }
        | Expr::PropertyAccess { .. }
        | Expr::IndexAccess { .. }
        | Expr::BinaryOp { .. }
        | Expr::FuzzyComparison { .. }
        | Expr::UnaryOp { .. }
        | Expr::FunctionCall { .. }
        | Expr::QualifiedFunctionCall { .. }
        | Expr::EnumConstructor { .. }
        | Expr::TimeRef(..)
        | Expr::DateTime(..)
        | Expr::PatternRef(..)
        | Expr::Conditional { .. }
        | Expr::Object(..)
        | Expr::Array(..)
        | Expr::TableRows(..)
        | Expr::ListComprehension(..)
        | Expr::Block(..)
        | Expr::TypeSyntax(..)
        | Expr::TypeAssertion { .. }
        | Expr::InstanceOf { .. }
        | Expr::FunctionExpr { .. }
        | Expr::Duration(..)
        | Expr::Spread(..)
        | Expr::If(..)
        | Expr::While(..)
        | Expr::For(..)
        | Expr::Loop(..)
        | Expr::Let(..)
        | Expr::Assign(..)
        | Expr::Break(..)
        | Expr::Continue(..)
        | Expr::Return(..)
        | Expr::MethodCall { .. }
        | Expr::Match(..)
        | Expr::Unit(..)
        | Expr::Range { .. }
        | Expr::TimeframeContext { .. }
        | Expr::TryOperator(..)
        | Expr::UsingImpl { .. }
        | Expr::SimulationCall { .. }
        | Expr::WindowExpr(..)
        | Expr::FromQuery(..)
        | Expr::StructLiteral { .. }
        | Expr::Await(..)
        | Expr::Join(..)
        | Expr::AsyncLet(..)
        | Expr::AsyncScope(..)
        | Expr::Reference { .. } => false,
    }
}

fn methods_may_generate(methods: &[MethodDef]) -> bool {
    methods.iter().any(|method| !method.annotations.is_empty())
}

fn trait_default_may_generate(definition: &TraitDef) -> bool {
    definition.members.iter().any(|member| {
        matches!(member, TraitMember::Default(method) if !method.annotations.is_empty())
    })
}

fn walk_annotations(visitor: &mut GenerationReachability, annotations: &[Annotation]) {
    for argument in annotations.iter().flat_map(|annotation| &annotation.args) {
        walk_expr(visitor, argument);
    }
}

fn walk_type_params(visitor: &mut GenerationReachability, params: Option<&[TypeParam]>) {
    for param in params.into_iter().flatten() {
        if let TypeParam::Const {
            default: Some(default),
            ..
        } = param
        {
            walk_expr(visitor, default);
        }
    }
}

fn walk_parameter_defaults(
    visitor: &mut GenerationReachability,
    params: &[shape_ast::ast::FunctionParameter],
) {
    for default in params
        .iter()
        .filter_map(|param| param.default_value.as_ref())
    {
        walk_expr(visitor, default);
    }
}

fn walk_method_omissions(visitor: &mut GenerationReachability, methods: &[MethodDef]) {
    for method in methods {
        walk_annotations(visitor, &method.annotations);
        walk_type_params(visitor, method.type_params.as_deref());
        walk_parameter_defaults(visitor, &method.params);
        if let Some(when_clause) = &method.when_clause {
            walk_expr(visitor, when_clause);
        }
    }
}

fn walk_complete_method(visitor: &mut GenerationReachability, method: &MethodDef) {
    walk_method_omissions(visitor, std::slice::from_ref(method));
    for statement in &method.body {
        walk_stmt(visitor, statement);
    }
}

fn walk_struct(visitor: &mut GenerationReachability, definition: &StructTypeDef) {
    walk_annotations(visitor, &definition.annotations);
    walk_type_params(visitor, definition.type_params.as_deref());
    for field in &definition.fields {
        walk_annotations(visitor, &field.annotations);
        if let Some(default) = &field.default_value {
            walk_expr(visitor, default);
        }
    }
}

fn walk_trait(visitor: &mut GenerationReachability, definition: &TraitDef) {
    walk_annotations(visitor, &definition.annotations);
    walk_type_params(visitor, definition.type_params.as_deref());
    for member in &definition.members {
        if let TraitMember::Default(method) = member {
            walk_complete_method(visitor, method);
        }
    }
}

fn walk_export_omissions(visitor: &mut GenerationReachability, item: &ExportItem) {
    match item {
        ExportItem::Function(def) => {
            walk_type_params(visitor, def.type_params.as_deref());
            walk_annotations(visitor, &def.annotations);
        }
        ExportItem::TypeAlias(def) => {
            walk_type_params(visitor, def.type_params.as_deref());
            for value in def
                .meta_param_overrides
                .iter()
                .flat_map(|values| values.values())
            {
                walk_expr(visitor, value);
            }
        }
        ExportItem::Enum(def) => {
            walk_type_params(visitor, def.type_params.as_deref());
            walk_annotations(visitor, &def.annotations);
        }
        ExportItem::Struct(def) => walk_struct(visitor, def),
        ExportItem::Trait(def) => walk_trait(visitor, def),
        ExportItem::Annotation(def) => walk_parameter_defaults(visitor, &def.params),
        ExportItem::ForeignFunction(def) => {
            walk_type_params(visitor, def.type_params.as_deref());
            walk_parameter_defaults(visitor, &def.params);
            walk_annotations(visitor, &def.annotations);
        }
        ExportItem::BuiltinFunction(_)
        | ExportItem::BuiltinType(_)
        | ExportItem::Named(_) => {}
    }
}
