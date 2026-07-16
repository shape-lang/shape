//! Conservative structural reachability for executable generation producers.

use shape_ast::ast::{ExportItem, Item, MethodDef, Program, TraitDef, TraitMember};

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
        Item::StructType(def, _) => {
            !def.annotations.is_empty() || methods_may_generate(&def.methods)
        }
        Item::Enum(def, _) => !def.annotations.is_empty(),
        Item::Trait(def, _) => trait_may_generate(def),
        Item::Extend(def, _) => methods_may_generate(&def.methods),
        Item::Impl(def, _) => methods_may_generate(&def.methods),
        Item::Module(def, _) => {
            !def.annotations.is_empty() || def.items.iter().any(item_may_generate)
        }
        Item::Export(export, _) => match &export.item {
            ExportItem::Function(def) => !def.annotations.is_empty(),
            ExportItem::ForeignFunction(def) => !def.annotations.is_empty(),
            ExportItem::Struct(def) => {
                !def.annotations.is_empty() || methods_may_generate(&def.methods)
            }
            ExportItem::Enum(def) => !def.annotations.is_empty(),
            ExportItem::Trait(def) => trait_may_generate(def),
            ExportItem::BuiltinFunction(_)
            | ExportItem::BuiltinType(_)
            | ExportItem::TypeAlias(_)
            | ExportItem::Named(_)
            | ExportItem::Annotation(_) => false,
        },
        _ => false,
    }
}

fn methods_may_generate(methods: &[MethodDef]) -> bool {
    methods
        .iter()
        .any(|method| !method.annotations.is_empty())
}

fn trait_may_generate(def: &TraitDef) -> bool {
    !def.annotations.is_empty()
        || def.members.iter().any(|member| {
            matches!(member, TraitMember::Default(method) if !method.annotations.is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::program_may_generate;
    use crate::compiler::executed_generated_items;
    use shape_ast::ast::{Item, TraitMember, TypeName};

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
    fn method_annotations_on_struct_and_trait_ast_carriers_are_reachable() {
        let method = parse(
            r#"
extend Number {
    @mark()
    method run() -> int { 0 }
}
"#,
        )
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Extend(def, _) => def.methods.into_iter().next(),
            _ => None,
        })
        .expect("annotated method fixture");

        let mut struct_program = parse("type Carrier { id: int }");
        let Item::StructType(def, _) = &mut struct_program.items[0] else {
            panic!("struct fixture")
        };
        def.methods.push(method.clone());
        assert!(program_may_generate(&struct_program));

        let mut trait_program = parse("trait Carrier { method run() -> int { 0 } }");
        let Item::Trait(def, _) = &mut trait_program.items[0] else {
            panic!("trait fixture")
        };
        let TraitMember::Default(default) = &mut def.members[0] else {
            panic!("default method fixture")
        };
        default.annotations = method.annotations;
        assert!(program_may_generate(&trait_program));
    }

    #[test]
    fn nested_annotated_extend_method_enters_executed_discovery() {
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
