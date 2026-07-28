//! Final closure-layout assembly from the canonical capture packs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use shape_ast::error::{Result, ShapeError};
use shape_value::v2::closure_layout::ClosureLayout;

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::capture_plan::CapturePack;

impl BytecodeCompiler {
    /// Build the `function_id -> ClosureLayout` table consumed by the VM and
    /// JIT. Capture packs are authoritative; registry layouts are independent
    /// interned-identity witnesses and never fallback sources.
    pub(super) fn finalize_closure_function_layouts(&mut self) -> Result<()> {
        let total_functions = self.program.functions.len();
        let mut layouts: Vec<Option<Arc<ClosureLayout>>> = vec![None; total_functions];
        let internal_error = |message: String| ShapeError::SemanticError {
            message: format!("internal compiler error (ADR-009 C1): {message}"),
            location: None,
        };

        // Function identity, never source span, keys the canonical pack. A
        // duplicate would otherwise be silently overwritten by `collect`.
        let mut packs_by_function: HashMap<u16, &CapturePack> = HashMap::new();
        for pack in &self.closure_capture_packs {
            if packs_by_function.insert(pack.closure, pack).is_some() {
                return Err(internal_error(format!(
                    "closure {} has more than one capture pack",
                    pack.closure
                )));
            }
        }

        let mut consumed_packs = HashSet::new();
        let mut seen_closure_functions = HashSet::new();
        for (function_index, type_id) in self.closure_type_ids.iter().copied() {
            if !seen_closure_functions.insert(function_index) {
                return Err(internal_error(format!(
                    "closure {function_index} has more than one ClosureTypeId entry"
                )));
            }
            let function = self
                .program
                .functions
                .get(usize::from(function_index))
                .ok_or_else(|| {
                    internal_error(format!(
                        "capture plan names missing function {function_index}"
                    ))
                })?;
            let pack = packs_by_function
                .get(&function_index)
                .copied()
                .ok_or_else(|| {
                    internal_error(format!("closure {function_index} has no capture pack"))
                })?;
            consumed_packs.insert(function_index);
            let registry_layout = self.closure_registry.get(type_id).ok_or_else(|| {
                internal_error(format!(
                    "closure {function_index} names unregistered ClosureTypeId {type_id:?}"
                ))
            })?;

            let capture_types = pack
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capture_type.clone())
                .collect::<Vec<_>>();
            let kinds = pack.kinds();
            if registry_layout.capture_types != capture_types
                || registry_layout.capture_kinds != kinds
            {
                return Err(internal_error(format!(
                    "closure {function_index}: interned registry layout disagrees with capture pack"
                )));
            }
            let rebuilt = ClosureLayout::from_capture_types(&capture_types, &kinds);

            let direct_instructions = self
                .program
                .direct_function_instructions(usize::from(function_index))
                .map_err(|error| {
                    internal_error(format!(
                        "closure {function_index}: invalid function instruction windows: {error}"
                    ))
                })?;
            pack.validate_emitted_artifact(registry_layout, function, direct_instructions)
                .map_err(&internal_error)?;
            layouts[usize::from(function_index)] = Some(Arc::new(rebuilt));
        }

        if let Some(unconsumed) = self
            .closure_capture_packs
            .iter()
            .find(|pack| !consumed_packs.contains(&pack.closure))
        {
            return Err(internal_error(format!(
                "closure {} has a capture pack but no ClosureTypeId",
                unconsumed.closure
            )));
        }
        self.program.closure_function_layouts = layouts;
        Ok(())
    }
}
