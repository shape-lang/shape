//! Index-keyed (Vec) rollback for [`InstallTransaction`](super::InstallTransaction).
//!
//! Everything NAME-keyed is restored by the displaced-entry undo
//! [`journal`](super::journal); this file rolls back only the append-only
//! tables keyed by `program.functions` INDEX, where a length watermark / index
//! filter is exact (a fresh entry can never displace a below-watermark one).

use super::InstallTransaction;
use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    /// Truncate the function table to the watermark and complete that truncation
    /// over its function-index-keyed closure derivatives. Runs in BOTH modes.
    ///
    /// Closures ARE `program.functions` entries, and a cluster of tables is
    /// keyed by that function index (`finalize_closure_function_layouts` keys
    /// packs by `pack.closure` and iterates `closure_type_ids` by function
    /// index, `compiler_impl_reference_model/closure_layouts.rs`). Truncating the
    /// function table FREES those indices, so any cluster entry left at a freed
    /// index is dangling — a REUSED compiler re-registers a closure at the reused
    /// index and then holds two entries for it, tripping the "closure N has more
    /// than one ClosureTypeId entry" / "more than one capture pack" uniqueness
    /// checks. So the truncation is completed over these derivatives. The
    /// interned-identity registries (`closure_registry`, `function_type_registry`)
    /// are keyed by `ClosureTypeId`, never duplicate, and are looked up (not
    /// enumerated) by finalize, so they need no rollback. Truncation (not
    /// remove-by-name) on `program.functions` is mandatory: removing a non-last
    /// entry would shift every later `FunctionId`, corrupting already-emitted
    /// `Operand::Function` operands in other bodies.
    ///
    /// The five push-accumulated function-index tables are exactly these: see
    /// the module docs' completeness argument (every `.push((func_idx …))` is
    /// the one closure-registration block). `closure_capture_packs` is the fifth;
    /// it is query metadata (the LSP capture query reads it) so it rolls back
    /// batch-only, in [`rollback_capture_packs`].
    pub(in crate::compiler) fn rollback_indexed_publications(
        &mut self,
        transaction: &InstallTransaction,
    ) {
        let watermark = transaction.functions_watermark;
        self.program.functions.truncate(watermark);
        self.closure_type_ids
            .retain(|(function_index, _)| usize::from(*function_index) < watermark);
        self.closure_function_ids
            .retain(|(_, function_index)| usize::from(*function_index) < watermark);
        self.closure_capture_names
            .retain(|(function_index, _)| usize::from(*function_index) < watermark);
        self.function_type_ids
            .retain(|(function_index, _)| usize::from(*function_index) < watermark);
    }

    /// Drop the install's `closure_capture_packs` (the query-metadata cluster
    /// member the LSP capture query reads). Runs in BATCH mode only; the
    /// query-session retain mode keeps them so the capture query still answers.
    pub(in crate::compiler) fn rollback_capture_packs(&mut self, transaction: &InstallTransaction) {
        let watermark = transaction.functions_watermark;
        self.closure_capture_packs
            .retain(|pack| usize::from(pack.closure) < watermark);
    }
}
