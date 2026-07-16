//! Conservative structural reachability for executable generation producers.

use shape_ast::ast::{ExportItem, Item, Program};

/// Whether the program's item tree can reach an executable generation
/// producer on the existing annotation/comptime materialization path.
///
/// This is deliberately a narrow structural walk, not an evaluator. A false
/// result proves that no supported declaration carries an annotation
/// application and no inline module contains a `comptime` item. A true result
/// only means the compiler must decide whether generation actually occurs.
pub fn program_may_generate(program: &Program) -> bool {
    program.items.iter().any(item_may_generate)
}

fn item_may_generate(item: &Item) -> bool {
    match item {
        Item::Comptime(..) => true,
        Item::Function(def, _) => !def.annotations.is_empty(),
        Item::ForeignFunction(def, _) => !def.annotations.is_empty(),
        Item::StructType(def, _) => !def.annotations.is_empty(),
        Item::Enum(def, _) => !def.annotations.is_empty(),
        Item::Trait(def, _) => !def.annotations.is_empty(),
        Item::Module(def, _) => {
            !def.annotations.is_empty() || def.items.iter().any(item_may_generate)
        }
        Item::Export(export, _) => match &export.item {
            ExportItem::Function(def) => !def.annotations.is_empty(),
            ExportItem::ForeignFunction(def) => !def.annotations.is_empty(),
            ExportItem::Struct(def) => !def.annotations.is_empty(),
            ExportItem::Enum(def) => !def.annotations.is_empty(),
            ExportItem::Trait(def) => !def.annotations.is_empty(),
            ExportItem::BuiltinFunction(_)
            | ExportItem::BuiltinType(_)
            | ExportItem::TypeAlias(_)
            | ExportItem::Named(_)
            | ExportItem::Annotation(_) => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::program_may_generate;
    use crate::compiler::executed_generated_items;
    use shape_ast::ast::{Item, TypeName};

    fn parse(source: &str) -> shape_ast::ast::Program {
        shape_ast::parse_program(source).expect("generation reachability fixture parses")
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
    fn unannotated_inline_module_reaches_nested_comptime_item() {
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
    fn nested_annotation_application_enters_executed_discovery() {
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
    @add_number_method()
    fn marker() { 0 }
}
"#,
        );
        let methods: Vec<_> = executed_generated_items(&program)
            .into_iter()
            .filter_map(|item| match item {
                Item::Extend(extend, _)
                    if matches!(&extend.type_name, TypeName::Simple(name) if name == "Number") =>
                {
                    Some(extend.methods)
                }
                _ => None,
            })
            .flatten()
            .map(|method| method.name)
            .collect();

        assert_eq!(methods, ["tripled"]);
    }
}
