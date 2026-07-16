//! Conservative structural reachability for executable generation producers.

use shape_ast::ast::{ExportItem, Item, MethodDef, Program, TraitDef, TraitMember, TypeName};

/// Whether the program's item tree can reach an executable generation
/// producer on any existing semantic compilation path.
///
/// This is deliberately a narrow structural walk, not an evaluator. A false
/// result proves that semantic compilation cannot generate. A true result only
/// delegates that decision to the compiler: module annotations and raw module
/// `comptime` blocks remain true here even though they mutate topology through
/// separate pass-2 APIs rather than the Decision-67 fixed point.
pub fn program_may_generate(program: &Program) -> bool {
    program.items.iter().any(item_may_generate)
}

fn item_may_generate(item: &Item) -> bool {
    match item {
        Item::Comptime(..) => true,
        Item::Function(def, _) => !def.is_comptime && !def.annotations.is_empty(),
        Item::StructType(def, _) => !def.annotations.is_empty(),
        // Annotated trait defaults execute only when pass 2 installs the
        // default into an impl. They are a conservative compile trigger, not a
        // member of the v1 pre-analysis fixed-point frontier.
        Item::Trait(def, _) => trait_default_may_generate(def),
        Item::Extend(def, _) => methods_may_generate(&def.methods),
        Item::Impl(def, _) => {
            !def.is_comptime
                && !matches!(
                    &def.trait_name,
                    TypeName::Simple(name) | TypeName::Generic { name, .. }
                        if matches!(name.as_str(), "From" | "TryFrom")
                )
                && methods_may_generate(&def.methods)
        }
        Item::Module(def, _) => {
            !def.annotations.is_empty() || def.items.iter().any(item_may_generate)
        }
        Item::Export(export, _) => match &export.item {
            ExportItem::Function(def) => !def.is_comptime && !def.annotations.is_empty(),
            ExportItem::Struct(def) => !def.annotations.is_empty(),
            ExportItem::BuiltinFunction(_)
            | ExportItem::BuiltinType(_)
            | ExportItem::TypeAlias(_)
            | ExportItem::Named(_)
            | ExportItem::Enum(_)
            | ExportItem::Annotation(_)
            | ExportItem::ForeignFunction(_) => false,
            ExportItem::Trait(def) => trait_default_may_generate(def),
        },
        _ => false,
    }
}

fn methods_may_generate(methods: &[MethodDef]) -> bool {
    methods
        .iter()
        .any(|method| !method.annotations.is_empty())
}

fn trait_default_may_generate(definition: &TraitDef) -> bool {
    definition.members.iter().any(|member| {
        matches!(member, TraitMember::Default(method) if !method.annotations.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::program_may_generate;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::{Item, TypeName};

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("generation reachability fixture parses")
    }

    fn discover_methods_before_analysis(
        program: &shape_ast::ast::Program,
        type_name: &str,
    ) -> Vec<String> {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .install_semantic_freeze()
            .expect("fixture installs the pre-analysis semantic freeze");
        compiler
            .materialize_computed_comptime_extends(program)
            .expect("declaration discovery succeeds")
            .into_iter()
            .filter_map(|item| match item {
                Item::Extend(extend, _)
                    if matches!(&extend.type_name, TypeName::Simple(name) if name == type_name) =>
                {
                    Some(extend.methods)
                }
                _ => None,
            })
            .flatten()
            .map(|method| method.name)
            .collect()
    }

    #[test]
    fn ordinary_item_tree_proves_generation_unreachable() {
        let program = parse(
            r#"
mod arithmetic {
    fn add(left: int, right: int) -> int { left + right }
}
fn identity(value: int) -> int { value }
"#,
        );

        assert!(!program_may_generate(&program));
    }

    #[test]
    fn unannotated_inline_module_reaches_nested_annotation_application() {
        let program = parse(
            r#"
mod generated {
    annotation mark() {
        targets: [function]
        comptime post(target, ctx) { 0 }
    }
    @mark()
    fn probe() -> int { 0 }
}
"#,
        );

        assert!(program_may_generate(&program));
    }

    #[test]
    fn raw_inline_module_comptime_delegates_to_semantic_compilation() {
        let program = parse(
            r#"
mod generated {
    comptime { 0 }
}
"#,
        );

        assert!(program_may_generate(&program));
    }

    #[test]
    fn annotated_module_delegates_to_semantic_compilation() {
        let program = parse(
            r#"
@mark()
mod generated {}
"#,
        );

        assert!(program_may_generate(&program));
    }

    #[test]
    fn annotated_trait_default_conservatively_delegates_to_pass_two() {
        let program = parse(
            r#"
trait Runnable {
    @mark()
    method run() -> int { 0 }
}
"#,
        );

        assert!(program_may_generate(&program));
    }

    #[test]
    fn annotated_exported_supported_target_is_reachable() {
        let program = parse(
            r#"
annotation mark() {
    targets: [function]
    comptime post(target, ctx) { 0 }
}
pub @mark() fn probe() -> int { 0 }
"#,
        );

        assert!(program_may_generate(&program));
    }

    #[test]
    fn annotated_impl_method_is_reachable() {
        let program = parse(
            r#"
annotation mark() {
    targets: [function]
    comptime post(target, ctx) { 0 }
}
trait Runnable { fn run() -> int; }
type Job { id: int }
impl Runnable for Job {
    @mark()
    method run() -> int { 0 }
}
"#,
        );

        assert!(program_may_generate(&program));
    }

    #[test]
    fn nested_free_function_target_executes_in_the_fixed_point() {
        let program = parse(
            r#"
mod generated {
    annotation add_number_method() {
        targets: [function]
        comptime post(target, ctx) {
            extend Number { method from_nested_function() { self } }
        }
    }
    @add_number_method()
    fn marker() -> int { 0 }
}
"#,
        );

        assert_eq!(
            discover_methods_before_analysis(&program, "Number"),
            ["from_nested_function"]
        );
    }

    #[test]
    fn nested_struct_target_executes_in_the_fixed_point() {
        let program = parse(
            r#"
mod generated {
    annotation add_number_method() {
        targets: [type]
        comptime post(target, ctx) {
            extend Number { method from_nested_struct() { self } }
        }
    }
    @add_number_method()
    type Marker { value: int }
}
"#,
        );

        assert_eq!(
            discover_methods_before_analysis(&program, "Number"),
            ["from_nested_struct"]
        );
    }

    #[test]
    fn nested_exported_function_and_struct_targets_execute_in_the_fixed_point() {
        let program = parse(
            r#"
mod generated {
    annotation add_from_function() {
        targets: [function]
        comptime post(target, ctx) {
            extend Number { method from_exported_function() { self } }
        }
    }
    annotation add_from_struct() {
        targets: [type]
        comptime post(target, ctx) {
            extend Number { method from_exported_struct() { self } }
        }
    }
    pub @add_from_function()
    fn marker() -> int { 0 }
    pub @add_from_struct()
    type Marker { value: int }
}
"#,
        );

        assert_eq!(
            discover_methods_before_analysis(&program, "Number"),
            ["from_exported_function", "from_exported_struct"]
        );
    }

    #[test]
    fn ordinary_impl_method_target_executes_in_the_fixed_point() {
        let program = parse(
            r#"
annotation add_number_method() {
    targets: [function]
    comptime post(target, ctx) {
        extend Number { method from_impl() { self } }
    }
}
trait Runnable { fn run() -> int; }
type Job { id: int }
impl Runnable for Job {
    @add_number_method()
    method run() -> int { 0 }
}
"#,
        );

        assert_eq!(
            discover_methods_before_analysis(&program, "Number"),
            ["from_impl"]
        );
    }

    #[test]
    fn nested_impl_inherited_return_reissues_one_discovery_identity() {
        let program = parse(
            r#"
mod generated {
    annotation add_number_method() {
        targets: [function]
        comptime post(target, ctx) {
            extend Number { method from_inherited_impl() { self } }
        }
    }
    trait Runnable { fn run() -> int; }
    type Job { id: int }
    impl Runnable for Job {
        @add_number_method()
        method run() { 0 }
    }
}

fn use_generated_method() -> number { 2.0.from_inherited_impl() }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&program)
            .expect("discovery and pass 2 agree on the inherited-return target descriptor");

        assert_eq!(
            compiler
                .generated_symbol_query()
                .symbols_named("from_inherited_impl")
                .len(),
            1,
            "the pass-2 application reissues the pre-analysis identity"
        );
        assert_eq!(
            compiler
                .program
                .functions
                .iter()
                .filter(|function| function.name == "Number.from_inherited_impl")
                .count(),
            1,
            "the inherited-return application never registers a duplicate function"
        );
    }

    #[test]
    fn nested_extend_generation_precedes_body_analysis_and_publishes_once() {
        let program = parse(
            r#"
mod generated {
    annotation add_number_method() {
        targets: [function]
        comptime post(target, ctx) {
            extend Number {
                method tripled() { self * 3.0 }
            }
        }
    }
    extend Number {
        @add_number_method()
        method marker() { self }
    }
}

fn use_tripled() -> number { 2.0.tripled() }
"#,
        );
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&program)
            .expect("ordinary body analysis sees the nested method generation");

        let methods: Vec<_> = compiler
            .generated_analysis_items()
            .iter()
            .filter_map(|item| match item {
                Item::Extend(extend, _)
                    if matches!(&extend.type_name, TypeName::Simple(name) if name == "Number") =>
                {
                    Some(&extend.methods)
                }
                _ => None,
            })
            .flatten()
            .map(|method| method.name.as_str())
            .collect();

        assert_eq!(methods, ["tripled"]);
        assert_eq!(
            compiler
                .generated_symbol_query()
                .symbols_named("tripled")
                .len(),
            1,
            "the one fixed point publishes one generated symbol"
        );
        assert_eq!(
            compiler
                .program
                .functions
                .iter()
                .filter(|function| function.name == "Number.tripled")
                .count(),
            1,
            "pass 2 fills the predeclared function instead of registering it twice"
        );
    }
}
