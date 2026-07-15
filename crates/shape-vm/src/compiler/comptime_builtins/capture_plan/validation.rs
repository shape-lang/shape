//! Final C1 artifact validation.
//!
//! The pack is the expected model. The registry layout, function metadata,
//! and instruction window are independent emitted artifacts and must agree
//! exactly before the layout is published on the bytecode program.

use shape_value::v2::closure_layout::ClosureLayout;

use crate::bytecode::{Function, Instruction, Operand};

use super::{CapturePack, artifact, exact_declared_kind};

impl CapturePack {
    /// Independently verify the final layout, function metadata, and emitted
    /// capture opcodes against this pack. The declaration is re-derived from
    /// retained source ownership; the emitted vector is never passed in as
    /// both expected and actual.
    pub(crate) fn validate_emitted_artifact<'a>(
        &self,
        layout: &ClosureLayout,
        function: &Function,
        instructions: impl IntoIterator<Item = &'a Instruction>,
    ) -> std::result::Result<(), String> {
        for (ordinal, descriptor) in self.descriptors.iter().enumerate() {
            if usize::from(descriptor.index) != ordinal {
                return Err(format!(
                    "closure {}: capture descriptor ordinal {ordinal} carries non-canonical \
                     index {}",
                    self.closure, descriptor.index
                ));
            }
        }

        let capture_types = self
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capture_type.clone())
            .collect::<Vec<_>>();
        let expected_layout = ClosureLayout::from_capture_types(&capture_types, &self.kinds());

        if layout.capture_count() != self.len()
            || layout.capture_types.len() != self.len()
            || layout.capture_kinds.len() != self.len()
            || layout.capture_native_kinds.len() != self.len()
        {
            return Err(format!(
                "closure {}: capture pack has {} entries but emitted layout has {} fields, {} \
                 types, {} kinds, and {} native kinds",
                self.closure,
                self.len(),
                layout.capture_count(),
                layout.capture_types.len(),
                layout.capture_kinds.len(),
                layout.capture_native_kinds.len(),
            ));
        }
        if !function.is_closure
            || usize::from(function.captures_count) != self.len()
            || function.mutable_captures.len() != self.len()
        {
            return Err(format!(
                "closure {}: pack has {} captures but function metadata says is_closure={}, {} \
                 captures, and {} mutability flags",
                self.closure,
                self.len(),
                function.is_closure,
                function.captures_count,
                function.mutable_captures.len(),
            ));
        }

        if layout.capture_native_kinds != expected_layout.capture_native_kinds {
            return Err(format!(
                "closure {}: emitted native-kind vector does not exactly match the capture pack",
                self.closure
            ));
        }
        for (index, (actual, expected)) in layout
            .captures
            .iter()
            .zip(&expected_layout.captures)
            .enumerate()
        {
            if actual.name != expected.name
                || actual.kind != expected.kind
                || actual.offset != expected.offset
                || actual.size != expected.size
            {
                return Err(format!(
                    "closure {} capture {index}: emitted field geometry does not exactly match \
                     the capture pack",
                    self.closure
                ));
            }
        }
        if layout.heap_capture_mask != expected_layout.heap_capture_mask
            || layout.owned_mutable_capture_mask != expected_layout.owned_mutable_capture_mask
            || layout.shared_capture_mask != expected_layout.shared_capture_mask
            || layout.captures_size != expected_layout.captures_size
            || layout.captures_align != expected_layout.captures_align
        {
            return Err(format!(
                "closure {}: emitted masks or aggregate geometry do not exactly match the \
                 capture pack",
                self.closure
            ));
        }

        let mut opcode_families = vec![None; self.len()];
        for instruction in instructions {
            let Some(family) = artifact::cell_capture_family(instruction.opcode) else {
                continue;
            };
            let Some(Operand::Local(index)) = instruction.operand else {
                return Err(format!(
                    "closure {}: capture-cell opcode {:?} has no Local capture operand",
                    self.closure, instruction.opcode
                ));
            };
            let Some(slot) = opcode_families.get_mut(usize::from(index)) else {
                return Err(format!(
                    "closure {}: capture-cell opcode {:?} names missing capture {}",
                    self.closure, instruction.opcode, index
                ));
            };
            if slot.is_some_and(|seen| seen != family) {
                return Err(format!(
                    "closure {} capture {}: emitted mixed capture-cell opcode families",
                    self.closure, index
                ));
            }
            *slot = Some(family);
        }

        for descriptor in &self.descriptors {
            let index = usize::from(descriptor.index);
            let actual_kind = layout.capture_storage_kind(index);
            if layout.capture_types[index] != descriptor.capture_type
                || actual_kind != descriptor.lowered
            {
                return Err(format!(
                    "closure {} capture {} ('{}'): emitted type/kind {:?}/{actual_kind:?} does \
                     not exactly match planned {:?}/{:?}",
                    self.closure,
                    index,
                    descriptor.name,
                    layout.capture_types[index],
                    descriptor.capture_type,
                    descriptor.lowered,
                ));
            }
            if function.mutable_captures[index] != descriptor.access.needs_cell() {
                return Err(format!(
                    "closure {} capture {} ('{}'): function cell flag disagrees with {:?}",
                    self.closure, index, descriptor.name, descriptor.access
                ));
            }

            match exact_declared_kind(descriptor) {
                Ok(Some(expected)) if actual_kind != expected => {
                    let mode = descriptor
                        .declared
                        .expect("an exact declared kind has a declared mode");
                    return Err(format!(
                        "closure {} capture {} ('{}'): declared `{}` requires exact kind \
                         {expected:?}, emitted {actual_kind:?}",
                        self.closure,
                        index,
                        descriptor.name,
                        mode.spelling()
                    ));
                }
                Err(reason) => {
                    return Err(format!(
                        "closure {} capture {} ('{}'): {reason}",
                        self.closure, index, descriptor.name
                    ));
                }
                Ok(_) => {}
            }

            let expected_family = artifact::family_for_access(descriptor.access);
            if opcode_families[index] != expected_family {
                return Err(format!(
                    "closure {} capture {} ('{}'): {:?} requires opcode family {:?}, emitted {:?}",
                    self.closure,
                    index,
                    descriptor.name,
                    descriptor.access,
                    expected_family,
                    opcode_families[index],
                ));
            }
        }
        Ok(())
    }
}
