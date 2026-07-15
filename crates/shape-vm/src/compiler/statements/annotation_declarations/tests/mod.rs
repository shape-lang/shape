use shape_ast::ast::{AnnotationDef, ExportItem, Item, Program};
use shape_ast::error::Result;

use crate::compiler::BytecodeCompiler;
use super::*;

fn parse(source: &str) -> Program {
    shape_ast::parse_program(source).expect("annotation declaration fixture parses")
}

fn only_definition(program: &Program) -> AnnotationDef {
    program
        .items
        .iter()
        .find_map(|item| match item {
            Item::AnnotationDef(definition, _) => Some(definition.clone()),
            Item::Export(export, _) => match &export.item {
                ExportItem::Annotation(definition) => Some(definition.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("fixture has an annotation definition")
}

mod identity;
mod phase;
mod transaction;
