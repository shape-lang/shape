//! Conservative structural reachability for executable generation producers.

#[path = "generation_reachability_walk.rs"]
mod walk;

pub use walk::program_may_generate;

#[cfg(test)]
mod tests {
    use super::program_may_generate;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::{Expr, Item, Statement, TypeName};

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
    annotation mark() on function {
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
    fn annotated_exported_supported_target_is_reachable() {
        let program = parse(
            r#"
annotation mark() on function {
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
annotation mark() on function {
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
    fn annotation_in_otherwise_unannotated_function_body_is_reachable() {
        let program = parse(
            r#"
annotation mark() on expression {
    comptime post(target, ctx) { 0 }
}
fn probe() -> int { @mark() 1 }
"#,
        );
        assert!(matches!(
            &program.items[1],
            Item::Function(def, _) if def.annotations.is_empty()
        ));
        assert!(program_may_generate(&program));
    }

    #[test]
    fn annotation_in_binding_initializer_is_reachable() {
        let program = parse(
            r#"
annotation mark() on expression {
    comptime post(target, ctx) { 0 }
}
let probe = @mark() 1
"#,
        );
        assert!(matches!(
            &program.items[1],
            Item::Statement(Statement::VariableDecl(declaration, _), _)
                if declaration.pattern.as_identifier() == Some("probe")
                    && matches!(declaration.value.as_ref(), Some(Expr::Annotated { .. }))
        ));
        assert!(program_may_generate(&program));
    }

    #[test]
    fn annotation_on_block_expression_is_reachable() {
        let program = parse(
            r#"
annotation mark() on block {
    comptime post(target, ctx) { 0 }
}
fn probe() -> int { @mark() { 1 } }
"#,
        );
        assert!(matches!(
            &program.items[1],
            Item::Function(def, _) if def.annotations.is_empty()
        ));
        assert!(program_may_generate(&program));
    }

    #[test]
    fn annotation_inside_await_expression_is_reachable() {
        let program = parse(
            r#"
annotation mark() on await_expr {
    comptime post(target, ctx) { 0 }
}
async function ready() { 1 }
async function probe() { await @mark() ready() }
"#,
        );
        assert!(
            program
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Function(def, _) => Some(def),
                    _ => None,
                })
                .all(|def| def.annotations.is_empty())
        );
        assert!(program_may_generate(&program));
    }

    #[test]
    fn comptime_expression_in_otherwise_unannotated_body_is_reachable() {
        let program = parse("fn probe() -> int { comptime { 1 } }");
        assert!(matches!(
            &program.items[0],
            Item::Function(def, _) if def.annotations.is_empty()
        ));
        assert!(program_may_generate(&program));
    }

    #[test]
    fn expression_annotation_in_trait_default_body_is_reachable() {
        let program = parse(
            r#"
annotation mark() on expression {
    comptime post(target, ctx) { 0 }
}
trait Runnable {
    method run() -> int { @mark() 0 }
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
    annotation add_number_method() on function {
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
    annotation add_number_method() on type {
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
    annotation add_from_function() on function {
        comptime post(target, ctx) {
            extend Number { method from_exported_function() { self } }
        }
    }
    annotation add_from_struct() on type {
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
annotation add_number_method() on function {
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
    annotation add_number_method() on function {
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
    annotation add_number_method() on function {
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
