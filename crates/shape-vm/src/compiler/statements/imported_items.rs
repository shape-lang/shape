//! Fail-closed imported-module registration for tooling compilers.

use shape_ast::ast::{ExportItem, Item};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    /// Register one imported module under its canonical qualified identity.
    ///
    /// Qualification of the entire module is a pure staging pass. Only after
    /// every item qualifies do mutations begin: all ordinary function headers
    /// are registered first, then the remaining ordinary surfaces, and finally
    /// annotation declarations are installed as one transaction. Callers must
    /// discard this compiler on any `Err`; no best-effort or bare-name fallback
    /// is exposed.
    pub fn register_imported_items(&mut self, module_path: &str, items: &[Item]) -> Result<()> {
        self.ensure_annotation_compiler_usable()?;
        let local_functions = Self::module_local_function_names(items);
        let qualified_items = items
            .iter()
            .filter(|item| !matches!(item, Item::Import(..)))
            .map(|item| {
                self.qualify_module_item_with_local_function_calls(
                    item,
                    module_path,
                    &local_functions,
                )
            })
            .collect::<Result<Vec<_>>>()?;

        for item in &qualified_items {
            match item {
                Item::Function(function, _) => self.register_imported_function_header(function)?,
                Item::Export(export, _) => {
                    if let ExportItem::Function(function) = &export.item {
                        self.register_imported_function_header(function)?;
                    }
                }
                _ => {}
            }
        }
        for item in &qualified_items {
            self.register_missing_module_items(item)?;
        }
        self.prepare_annotation_scope(&qualified_items)
    }

    fn register_imported_function_header(
        &mut self,
        function: &shape_ast::ast::FunctionDef,
    ) -> Result<()> {
        match self.function_defs.get(&function.name) {
            None => self.register_function(function),
            Some(existing) if existing == function => Ok(()),
            Some(_) => Err(ShapeError::SemanticError {
                message: format!(
                    "Imported function '{}' conflicts with an already registered callable",
                    function.name
                ),
                location: Some(self.span_to_source_location(function.name_span)),
            }),
        }
    }
}

#[cfg(test)]
mod tests;
