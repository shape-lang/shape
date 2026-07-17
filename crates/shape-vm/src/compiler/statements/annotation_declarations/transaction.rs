//! Cache-staged annotation installation transaction and terminal quarantine.

use shape_ast::ast::{AnnotationDef, Item};
use shape_ast::error::Result;

use crate::compiler::BytecodeCompiler;

use super::{installer, planner, terminal_error};

impl BytecodeCompiler {
    /// Prepare every annotation declaration in one registration-complete scope.
    pub(in crate::compiler) fn prepare_annotation_scope(&mut self, items: &[Item]) -> Result<()> {
        self.ensure_annotation_compiler_usable()?;

        // Planning is immutable and happens while the original cache remains
        // attached. A planning error poisons query publication, but it cannot
        // have changed cache content or any installation artifact.
        let plan = {
            let installed = self.annotation_declarations.ready_declarations()?;
            planner::build(self, items, installed)
        };
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.poison_annotation_compiler();
                return Err(error);
            }
        };
        if plan.is_empty() {
            return Ok(());
        }

        self.annotation_declarations.begin_installation()?;
        let mut staged_cache = self.blob_cache.take();
        let completed_blob_start = self.completed_blobs.len();
        let mut carriers = Vec::with_capacity(plan.pending.len());
        let mut definitions = Vec::with_capacity(plan.pending.len());

        for pending in &plan.pending {
            match installer::install(self, pending) {
                Ok(carrier) => {
                    carriers.push(carrier);
                    definitions.push(pending.definition.clone());
                }
                Err(error) => {
                    // The detached cache is byte-for-byte untouched. Do not
                    // replay transaction blobs from the poisoned compiler.
                    self.blob_cache = staged_cache;
                    self.poison_annotation_compiler();
                    return Err(error);
                }
            }
        }

        // Cache publication precedes semantic carrier publication. `put_blob`
        // is infallible; only this transaction's completed-blob delta is
        // replayed, exactly once, after every installation has succeeded.
        if let Some(cache) = staged_cache.as_mut() {
            for blob in &self.completed_blobs[completed_blob_start..] {
                cache.put_blob(blob);
            }
        }
        self.blob_cache = staged_cache;
        for carrier in carriers {
            self.program
                .compiled_annotations
                .insert(carrier.name.clone(), carrier);
        }
        self.annotation_declarations.commit(definitions);
        Ok(())
    }

    /// Pass 2 consumes preparation evidence; it never installs or falls back.
    pub(in crate::compiler::statements) fn require_prepared_annotation(
        &mut self,
        definition: &AnnotationDef,
    ) -> Result<()> {
        let result = self.annotation_declarations.require(definition);
        if result.is_err() && self.annotation_declarations.queries_available() {
            self.poison_annotation_compiler();
        }
        result
    }

    pub(in crate::compiler) fn ensure_annotation_compiler_usable(&self) -> Result<()> {
        if self.annotation_declarations.queries_available() {
            Ok(())
        } else {
            Err(terminal_error())
        }
    }

    /// Tooling must distinguish a legitimate empty query result from a
    /// terminally quarantined compiler and route the latter as unavailable.
    pub fn generated_queries_available(&self) -> bool {
        self.annotation_declarations.queries_available()
    }

    pub(in crate::compiler) fn poison_annotation_compiler(&mut self) {
        self.annotation_declarations.poison();
        self.generated_symbols =
            crate::compiler::comptime_builtins::expansion_provenance::GeneratedSymbolTable::new();
        self.generated_analysis_items.clear();
        self.closure_capture_packs.clear();
    }
}
