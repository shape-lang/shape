//! Module system types for Shape AST

use serde::{Deserialize, Serialize};

use super::functions::{Annotation, ForeignFunctionDef, FunctionDef};
use super::program::Item;
use super::span::Span;
use super::types::{EnumDef, InterfaceDef, StructTypeDef, TraitDef};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStmt {
    pub items: ImportItems,
    pub from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportItems {
    /// from module::path use { a, b as c }
    Named(Vec<ImportSpec>),
    /// use module::path / use module::path as alias (binds local tail segment or alias)
    Namespace { name: String, alias: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSpec {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportStmt {
    pub item: ExportItem,
    /// For `pub let/const/var`, the original variable declaration so the compiler
    /// can compile the initialization (ExportItem::Named only preserves the name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_decl: Option<super::program::VariableDecl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExportItem {
    /// pub fn name(...) { ... }
    Function(FunctionDef),
    /// pub type Name = Type;
    TypeAlias(super::types::TypeAliasDef),
    /// pub { name1, name2 as alias }
    Named(Vec<ExportSpec>),
    /// pub enum Name { ... }
    Enum(EnumDef),
    /// pub type Name { field: Type, ... }
    Struct(StructTypeDef),
    /// pub interface Name { ... }
    Interface(InterfaceDef),
    /// pub trait Name { ... }
    Trait(TraitDef),
    /// pub fn python name(...) { ... }
    ForeignFunction(ForeignFunctionDef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSpec {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDecl {
    pub name: String,
    pub name_span: Span,
    pub annotations: Vec<Annotation>,
    pub items: Vec<Item>,
}
