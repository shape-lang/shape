//! Transactional, registration-complete annotation declaration preparation.

use std::collections::BTreeMap;

use shape_ast::ast::{AnnotationDef, ExportItem, Item};
use shape_ast::error::{Result, ShapeError};

const POISONED_COMPILER_DIAGNOSTIC: &str =
    "Internal compiler error: annotation declaration installation failed; this compiler is poisoned and cannot be reused";

/// Opaque phase evidence for annotation declarations.
///
/// `Installing` owns the previously committed declarations while a transaction
/// is in flight. `Poisoned` is terminal: a partially mutated compiler is never
/// admitted to a second compilation or query session.
pub(in crate::compiler) struct AnnotationDeclarationState {
    phase: AnnotationDeclarationPhase,
}

enum AnnotationDeclarationPhase {
    Ready(BTreeMap<String, AnnotationDef>),
    Installing(BTreeMap<String, AnnotationDef>),
    Poisoned,
}

impl Default for AnnotationDeclarationState {
    fn default() -> Self {
        Self {
            phase: AnnotationDeclarationPhase::Ready(BTreeMap::new()),
        }
    }
}

impl AnnotationDeclarationState {
    pub(super) fn ready_declarations(&self) -> Result<&BTreeMap<String, AnnotationDef>> {
        match &self.phase {
            AnnotationDeclarationPhase::Ready(installed) => Ok(installed),
            AnnotationDeclarationPhase::Installing(_) | AnnotationDeclarationPhase::Poisoned => {
                Err(terminal_error())
            }
        }
    }

    pub(super) fn begin_installation(&mut self) -> Result<()> {
        let phase = std::mem::replace(&mut self.phase, AnnotationDeclarationPhase::Poisoned);
        match phase {
            AnnotationDeclarationPhase::Ready(installed) => {
                self.phase = AnnotationDeclarationPhase::Installing(installed);
                Ok(())
            }
            AnnotationDeclarationPhase::Installing(installed) => {
                self.phase = AnnotationDeclarationPhase::Installing(installed);
                Err(terminal_error())
            }
            AnnotationDeclarationPhase::Poisoned => Err(terminal_error()),
        }
    }

    pub(super) fn commit(&mut self, definitions: impl IntoIterator<Item = AnnotationDef>) {
        let phase = std::mem::replace(&mut self.phase, AnnotationDeclarationPhase::Poisoned);
        let AnnotationDeclarationPhase::Installing(mut installed) = phase else {
            return;
        };
        for definition in definitions {
            installed.insert(definition.name.clone(), definition);
        }
        self.phase = AnnotationDeclarationPhase::Ready(installed);
    }

    pub(super) fn poison(&mut self) {
        self.phase = AnnotationDeclarationPhase::Poisoned;
    }

    pub(super) fn queries_available(&self) -> bool {
        matches!(self.phase, AnnotationDeclarationPhase::Ready(_))
    }

    pub(super) fn require(&self, definition: &AnnotationDef) -> Result<()> {
        let installed = self.ready_declarations()?;
        match installed.get(&definition.name) {
            Some(prepared) if prepared == definition => Ok(()),
            Some(_) => Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal compiler phase-order error: annotation declaration '{}' changed between preparation and pass 2",
                    definition.name
                ),
                location: None,
            }),
            None => Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal compiler phase-order error: annotation declaration '{}' reached pass 2 before declaration preparation",
                    definition.name
                ),
                location: None,
            }),
        }
    }
}

pub(super) fn annotation_definition(item: &Item) -> Option<&AnnotationDef> {
    match item {
        Item::AnnotationDef(definition, _) => Some(definition),
        Item::Export(export, _) => match &export.item {
            ExportItem::Annotation(definition) => Some(definition),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn terminal_error() -> ShapeError {
    ShapeError::RuntimeError {
        message: POISONED_COMPILER_DIAGNOSTIC.to_string(),
        location: None,
    }
}

mod installer;
mod planner;
mod transaction;

#[cfg(test)]
mod tests;
