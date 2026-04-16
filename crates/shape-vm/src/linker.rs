//! Linking pass: converts a content-addressed `Program` into a flat `LinkedProgram`.
//!
//! The linker topologically sorts function blobs by their dependency edges,
//! then flattens per-blob instruction/constant/string pools into merged arrays,
//! remapping operand indices so they reference the correct global positions.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::bytecode::{
    BytecodeProgram, Constant, DebugInfo, Function, FunctionBlob, FunctionHash, Instruction,
    LinkedFunction, LinkedProgram, Operand, Program, SourceMap,
};
use shape_abi_v1::PermissionSet;
use shape_value::{FunctionId, StringId, ValueWordExt};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("Missing function blob: {0}")]
    MissingBlob(FunctionHash),
    #[error("Circular dependency detected")]
    CircularDependency,
    #[error("Constant pool overflow: {0} constants exceeds u16 max")]
    ConstantPoolOverflow(usize),
    #[error("String pool overflow: {0} strings exceeds u32 max")]
    StringPoolOverflow(usize),
}

// ---------------------------------------------------------------------------
// Topological sort
// ---------------------------------------------------------------------------

/// Topologically sort blobs so that every dependency appears before
/// the blob that depends on it.  Returns blob hashes in dependency order
/// (leaves first, entry last).
fn topo_sort(program: &Program) -> Result<Vec<FunctionHash>, LinkError> {
    // States: 0 = unvisited, 1 = in-progress, 2 = done
    let mut state: HashMap<FunctionHash, u8> = HashMap::new();
    let mut order: Vec<FunctionHash> = Vec::with_capacity(program.function_store.len());

    fn visit(
        hash: FunctionHash,
        program: &Program,
        state: &mut HashMap<FunctionHash, u8>,
        order: &mut Vec<FunctionHash>,
    ) -> Result<(), LinkError> {
        match state.get(&hash).copied().unwrap_or(0) {
            2 => return Ok(()), // already done
            1 => return Err(LinkError::CircularDependency),
            _ => {}
        }
        state.insert(hash, 1); // mark in-progress

        let blob = program
            .function_store
            .get(&hash)
            .ok_or(LinkError::MissingBlob(hash))?;

        for dep in &blob.dependencies {
            // ZERO is an explicit self-recursion sentinel produced by the compiler.
            // It does not reference a separate blob in function_store.
            if *dep == FunctionHash::ZERO {
                continue;
            }
            visit(*dep, program, state, order)?;
        }

        state.insert(hash, 2); // done
        order.push(hash);
        Ok(())
    }

    // Visit all blobs reachable from the entry point.  We start from entry
    // so unreachable blobs are excluded (they could be present in the store
    // from incremental compilation).
    visit(program.entry, program, &mut state, &mut order)?;

    // Also visit any remaining blobs not reachable from entry
    // (e.g. blobs referenced only from constants or other metadata).
    let remaining: Vec<FunctionHash> = program
        .function_store
        .keys()
        .copied()
        .filter(|h| state.get(h).copied().unwrap_or(0) != 2)
        .collect();
    for hash in remaining {
        visit(hash, program, &mut state, &mut order)?;
    }

    Ok(order)
}

// ---------------------------------------------------------------------------
// Operand remapping
// ---------------------------------------------------------------------------

/// Remap a single operand given the per-blob base offsets and function
/// hash-to-id mapping.
fn remap_operand(
    operand: Operand,
    const_base: usize,
    string_base: usize,
    blob: &FunctionBlob,
    current_function_id: usize,
    hash_to_id: &HashMap<FunctionHash, usize>,
    name_to_id: &HashMap<&str, usize>,
) -> Operand {
    match operand {
        Operand::Const(i) => Operand::Const((const_base + i as usize) as u16),
        Operand::Property(i) => Operand::Property((string_base + i as usize) as u16),
        Operand::Name(StringId(i)) => Operand::Name(StringId((string_base + i as usize) as u32)),
        Operand::Function(FunctionId(dep_idx)) => {
            if let Some(dep_hash) = blob.dependencies.get(dep_idx as usize) {
                if *dep_hash == FunctionHash::ZERO {
                    // ZERO sentinel: self-recursion or mutual recursion.
                    // Check callee_names to determine which function to target.
                    if let Some(callee_name) = blob.callee_names.get(dep_idx as usize) {
                        if callee_name != &blob.name {
                            // Mutual recursion: look up the target by name.
                            if let Some(target_id) = name_to_id.get(callee_name.as_str()) {
                                Operand::Function(FunctionId(*target_id as u16))
                            } else {
                                // Fallback: self (shouldn't happen for valid programs).
                                Operand::Function(FunctionId(current_function_id as u16))
                            }
                        } else {
                            // Self-recursion.
                            Operand::Function(FunctionId(current_function_id as u16))
                        }
                    } else {
                        // No callee name info; assume self-recursion.
                        Operand::Function(FunctionId(current_function_id as u16))
                    }
                } else {
                    let linked_id = hash_to_id[dep_hash];
                    Operand::Function(FunctionId(linked_id as u16))
                }
            } else {
                // Defensive fallback for blobs emitted with already-global function ids.
                Operand::Function(FunctionId(dep_idx))
            }
        }
        Operand::TypedMethodCall {
            method_id,
            arg_count,
            string_id,
            receiver_type_tag,
        } => Operand::TypedMethodCall {
            method_id,
            arg_count,
            string_id: (string_base + string_id as usize) as u16,
            receiver_type_tag,
        },
        // Unchanged operands:
        Operand::Offset(_)
        | Operand::Local(_)
        | Operand::ModuleBinding(_)
        | Operand::Builtin(_)
        | Operand::Count(_)
        | Operand::ColumnIndex(_)
        | Operand::TypedField { .. }
        | Operand::TypedObjectAlloc { .. }
        | Operand::TypedMerge { .. }
        | Operand::ColumnAccess { .. }
        | Operand::ForeignFunction(_)
        | Operand::MatrixDims { .. }
        | Operand::Width(_)
        | Operand::TypedLocal(_, _)
        | Operand::TypedModuleBinding(_, _)
        | Operand::FieldOffset(_) => operand,
    }
}

// ---------------------------------------------------------------------------
// Constant remapping
// ---------------------------------------------------------------------------

/// Remap function references inside `Constant::Function(idx)`.
/// These reference dependency indices within the blob, not global function IDs,
/// so they need the same treatment as `Operand::Function`.
fn remap_constant(
    constant: &Constant,
    blob: &FunctionBlob,
    current_function_id: usize,
    hash_to_id: &HashMap<FunctionHash, usize>,
    name_to_id: &HashMap<&str, usize>,
) -> Constant {
    match constant {
        Constant::Function(dep_idx) => {
            let dep_idx = *dep_idx as usize;
            if dep_idx < blob.dependencies.len() {
                let dep_hash = blob.dependencies[dep_idx];
                if dep_hash == FunctionHash::ZERO {
                    // ZERO sentinel: self-recursion or mutual recursion.
                    if let Some(callee_name) = blob.callee_names.get(dep_idx) {
                        if callee_name != &blob.name {
                            // Mutual recursion: look up the target by name.
                            if let Some(target_id) = name_to_id.get(callee_name.as_str()) {
                                Constant::Function(*target_id as u16)
                            } else {
                                Constant::Function(current_function_id as u16)
                            }
                        } else {
                            Constant::Function(current_function_id as u16)
                        }
                    } else {
                        Constant::Function(current_function_id as u16)
                    }
                } else {
                    let linked_id = hash_to_id[&dep_hash];
                    Constant::Function(linked_id as u16)
                }
            } else {
                // dep_idx doesn't map to a dependency — keep as-is.
                constant.clone()
            }
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Public API: link
// ---------------------------------------------------------------------------

/// Threshold for switching from sequential to parallel remap.
/// Below this count, the overhead of Rayon's thread pool is not worth it.
const PARALLEL_THRESHOLD: usize = 50;

/// Per-blob offset information computed in Pass 1.
struct BlobOffsets {
    instruction_base: usize,
    const_base: usize,
    string_base: usize,
}

/// Link a content-addressed `Program` into a flat `LinkedProgram`.
///
/// The linker:
/// 1. Topologically sorts function blobs by dependencies.
/// 2. **Pass 1 (sequential):** Computes cumulative base offsets for each blob
///    and builds the `hash_to_id` reverse index.
/// 3. **Pass 2 (parallel for >50 functions):** Each blob independently remaps
///    its instructions/constants/strings into pre-allocated output arrays at
///    non-overlapping offsets.
/// 4. Builds a `LinkedFunction` table and merged debug info.
pub fn link(program: &Program) -> Result<LinkedProgram, LinkError> {
    let sorted = topo_sort(program)?;

    // Resolve sorted hashes to blob references up-front.
    let blobs: Vec<&FunctionBlob> = sorted
        .iter()
        .map(|h| {
            program
                .function_store
                .get(h)
                .ok_or(LinkError::MissingBlob(*h))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // ------------------------------------------------------------------
    // Pass 1 (sequential): compute base offsets and hash_to_id
    // ------------------------------------------------------------------
    let mut offsets: Vec<BlobOffsets> = Vec::with_capacity(blobs.len());
    let mut hash_to_id: HashMap<FunctionHash, usize> = HashMap::with_capacity(blobs.len());
    let mut name_to_id: HashMap<&str, usize> = HashMap::with_capacity(blobs.len());

    let mut total_instructions: usize = 0;
    let mut total_constants: usize = 0;
    let mut total_strings: usize = 0;

    for (i, blob) in blobs.iter().enumerate() {
        offsets.push(BlobOffsets {
            instruction_base: total_instructions,
            const_base: total_constants,
            string_base: total_strings,
        });
        hash_to_id.insert(blob.content_hash, i);
        name_to_id.insert(&blob.name, i);

        total_instructions += blob.instructions.len();
        total_constants += blob.constants.len();
        total_strings += blob.strings.len();
    }

    // Overflow checks on totals.
    if total_constants > u16::MAX as usize + 1 {
        return Err(LinkError::ConstantPoolOverflow(total_constants));
    }
    if total_strings > u32::MAX as usize + 1 {
        return Err(LinkError::StringPoolOverflow(total_strings));
    }

    // Compute transitive union of all required permissions across all blobs.
    let total_required_permissions = blobs.iter().fold(PermissionSet::pure(), |acc, blob| {
        acc.union(&blob.required_permissions)
    });

    // ------------------------------------------------------------------
    // Pass 2: remap and write into pre-allocated arrays
    // ------------------------------------------------------------------
    let use_parallel = blobs.len() > PARALLEL_THRESHOLD;

    // Pre-allocate output arrays with exact sizes.
    let mut instructions: Vec<Instruction> = Vec::with_capacity(total_instructions);
    let mut constants: Vec<Constant> = Vec::with_capacity(total_constants);
    let mut strings: Vec<String> = Vec::with_capacity(total_strings);

    if use_parallel {
        // SAFETY: We write to non-overlapping regions of the output arrays.
        // Each blob writes to [base..base+len) which is disjoint from all
        // other blobs because the bases are cumulative sums of prior sizes.
        // We use `set_len` after all writes to make the Vecs aware of the data.

        // Extend vecs to their full capacity with uninitialized-safe defaults.
        // For Instructions (Copy type), use zeroed memory via MaybeUninit logic.
        // For Constant/String (non-Copy), we must use a different strategy:
        // collect per-blob results in parallel, then write sequentially.

        // Strategy: parallel map each blob to its (remapped_instructions,
        // remapped_constants, cloned_strings, source_map_entries), then
        // write them into the pre-allocated arrays sequentially (memcpy-fast).
        struct BlobResult {
            instructions: Vec<Instruction>,
            constants: Vec<Constant>,
            strings: Vec<String>,
            source_map: Vec<(usize, u16, u32)>,
        }

        let results: Vec<BlobResult> = blobs
            .par_iter()
            .zip(offsets.par_iter())
            .enumerate()
            .map(|(function_id, (blob, off))| {
                let remapped_instrs: Vec<Instruction> = blob
                    .instructions
                    .iter()
                    .map(|instr| {
                        let remapped_operand = instr.operand.map(|op| {
                            remap_operand(
                                op,
                                off.const_base,
                                off.string_base,
                                blob,
                                function_id,
                                &hash_to_id,
                                &name_to_id,
                            )
                        });
                        Instruction {
                            opcode: instr.opcode,
                            operand: remapped_operand,
                        }
                    })
                    .collect();

                let remapped_consts: Vec<Constant> = blob
                    .constants
                    .iter()
                    .map(|c| remap_constant(c, blob, function_id, &hash_to_id, &name_to_id))
                    .collect();

                let cloned_strings: Vec<String> = blob.strings.clone();

                let source_entries: Vec<(usize, u16, u32)> = blob
                    .source_map
                    .iter()
                    .map(|&(local_offset, file_id, line)| {
                        (off.instruction_base + local_offset, file_id as u16, line)
                    })
                    .collect();

                BlobResult {
                    instructions: remapped_instrs,
                    constants: remapped_consts,
                    strings: cloned_strings,
                    source_map: source_entries,
                }
            })
            .collect();

        // Now write results into the pre-allocated arrays (sequential, but
        // this is just memcpy/move of contiguous data -- very fast).
        let mut merged_line_numbers: Vec<(usize, u16, u32)> = Vec::new();
        for result in results {
            instructions.extend(result.instructions);
            constants.extend(result.constants);
            strings.extend(result.strings);
            merged_line_numbers.extend(result.source_map);
        }

        merged_line_numbers.sort_by_key(|&(offset, _, _)| offset);

        let functions: Vec<LinkedFunction> = blobs
            .iter()
            .zip(offsets.iter())
            .map(|(blob, off)| LinkedFunction {
                blob_hash: blob.content_hash,
                entry_point: off.instruction_base,
                body_length: blob.instructions.len(),
                name: blob.name.clone(),
                arity: blob.arity,
                param_names: blob.param_names.clone(),
                locals_count: blob.locals_count,
                is_closure: blob.is_closure,
                captures_count: blob.captures_count,
                is_async: blob.is_async,
                ref_params: blob.ref_params.clone(),
                ref_mutates: blob.ref_mutates.clone(),
                mutable_captures: blob.mutable_captures.clone(),
                frame_descriptor: blob.frame_descriptor.clone(),
            })
            .collect();

        let debug_info = DebugInfo {
            source_map: SourceMap {
                files: program.debug_info.source_map.files.clone(),
                source_texts: program.debug_info.source_map.source_texts.clone(),
            },
            line_numbers: merged_line_numbers,
            variable_names: program.debug_info.variable_names.clone(),
            source_text: String::new(),
        };

        return Ok(LinkedProgram {
            entry: program.entry,
            instructions,
            constants,
            strings,
            functions,
            hash_to_id,
            debug_info,
            data_schema: program.data_schema.clone(),
            module_binding_names: program.module_binding_names.clone(),
            top_level_locals_count: program.top_level_locals_count,
            top_level_local_storage_hints: program.top_level_local_storage_hints.clone(),
            type_schema_registry: program.type_schema_registry.clone(),
            module_binding_storage_hints: program.module_binding_storage_hints.clone(),
            function_local_storage_hints: program.function_local_storage_hints.clone(),
            top_level_frame: program.top_level_frame.clone(),
            trait_method_symbols: program.trait_method_symbols.clone(),
            foreign_functions: program.foreign_functions.clone(),
            native_struct_layouts: program.native_struct_layouts.clone(),
            total_required_permissions: total_required_permissions.clone(),
        });
    }

    // ------------------------------------------------------------------
    // Sequential path (≤ PARALLEL_THRESHOLD functions)
    // ------------------------------------------------------------------
    let mut merged_line_numbers: Vec<(usize, u16, u32)> = Vec::new();

    for (function_id, (blob, off)) in blobs.iter().zip(offsets.iter()).enumerate() {
        // Remap and copy instructions.
        for instr in &blob.instructions {
            let remapped_operand = instr.operand.map(|op| {
                remap_operand(
                    op,
                    off.const_base,
                    off.string_base,
                    blob,
                    function_id,
                    &hash_to_id,
                    &name_to_id,
                )
            });
            instructions.push(Instruction {
                opcode: instr.opcode,
                operand: remapped_operand,
            });
        }

        // Merge constants (remap Constant::Function).
        for c in &blob.constants {
            constants.push(remap_constant(
                c,
                blob,
                function_id,
                &hash_to_id,
                &name_to_id,
            ));
        }

        // Merge strings.
        strings.extend(blob.strings.iter().cloned());

        // Merge source map entries.
        for &(local_offset, file_id, line) in &blob.source_map {
            let global_offset = off.instruction_base + local_offset;
            merged_line_numbers.push((global_offset, file_id as u16, line));
        }
    }

    // Sort line numbers by instruction offset for correct binary-search lookup.
    merged_line_numbers.sort_by_key(|&(offset, _, _)| offset);

    let functions: Vec<LinkedFunction> = blobs
        .iter()
        .zip(offsets.iter())
        .map(|(blob, off)| LinkedFunction {
            blob_hash: blob.content_hash,
            entry_point: off.instruction_base,
            body_length: blob.instructions.len(),
            name: blob.name.clone(),
            arity: blob.arity,
            param_names: blob.param_names.clone(),
            locals_count: blob.locals_count,
            is_closure: blob.is_closure,
            captures_count: blob.captures_count,
            is_async: blob.is_async,
            ref_params: blob.ref_params.clone(),
            ref_mutates: blob.ref_mutates.clone(),
            mutable_captures: blob.mutable_captures.clone(),
            frame_descriptor: blob.frame_descriptor.clone(),
        })
        .collect();

    let debug_info = DebugInfo {
        source_map: SourceMap {
            files: program.debug_info.source_map.files.clone(),
            source_texts: program.debug_info.source_map.source_texts.clone(),
        },
        line_numbers: merged_line_numbers,
        variable_names: program.debug_info.variable_names.clone(),
        source_text: String::new(),
    };

    Ok(LinkedProgram {
        entry: program.entry,
        instructions,
        constants,
        strings,
        functions,
        hash_to_id,
        debug_info,
        data_schema: program.data_schema.clone(),
        module_binding_names: program.module_binding_names.clone(),
        top_level_locals_count: program.top_level_locals_count,
        top_level_local_storage_hints: program.top_level_local_storage_hints.clone(),
        type_schema_registry: program.type_schema_registry.clone(),
        module_binding_storage_hints: program.module_binding_storage_hints.clone(),
        function_local_storage_hints: program.function_local_storage_hints.clone(),
        top_level_frame: program.top_level_frame.clone(),
        trait_method_symbols: program.trait_method_symbols.clone(),
        foreign_functions: program.foreign_functions.clone(),
        native_struct_layouts: program.native_struct_layouts.clone(),
        total_required_permissions,
    })
}

// ---------------------------------------------------------------------------
// Public API: linked_to_bytecode_program
// ---------------------------------------------------------------------------

/// Convert a `LinkedProgram` back to the legacy `BytecodeProgram` format
/// for backward compatibility with the existing VM executor.
pub fn linked_to_bytecode_program(linked: &LinkedProgram) -> BytecodeProgram {
    let functions: Vec<Function> = linked
        .functions
        .iter()
        .map(|lf| Function {
            name: lf.name.clone(),
            arity: lf.arity,
            param_names: lf.param_names.clone(),
            locals_count: lf.locals_count,
            entry_point: lf.entry_point,
            body_length: lf.body_length,
            is_closure: lf.is_closure,
            captures_count: lf.captures_count,
            is_async: lf.is_async,
            ref_params: lf.ref_params.clone(),
            ref_mutates: lf.ref_mutates.clone(),
            mutable_captures: lf.mutable_captures.clone(),
            frame_descriptor: lf.frame_descriptor.clone(),
            osr_entry_points: Vec::new(),
            mir_data: None,
        })
        .collect();

    BytecodeProgram {
        instructions: linked.instructions.clone(),
        constants: linked.constants.clone(),
        strings: linked.strings.clone(),
        functions,
        debug_info: linked.debug_info.clone(),
        data_schema: linked.data_schema.clone(),
        module_binding_names: linked.module_binding_names.clone(),
        top_level_locals_count: linked.top_level_locals_count,
        top_level_local_storage_hints: linked.top_level_local_storage_hints.clone(),
        type_schema_registry: linked.type_schema_registry.clone(),
        module_binding_storage_hints: linked.module_binding_storage_hints.clone(),
        function_local_storage_hints: linked.function_local_storage_hints.clone(),
        top_level_frame: linked.top_level_frame.clone(),
        top_level_mir: None,
        compiled_annotations: HashMap::new(),
        trait_method_symbols: linked.trait_method_symbols.clone(),
        expanded_function_defs: HashMap::new(),
        string_index: HashMap::new(),
        foreign_functions: linked.foreign_functions.clone(),
        native_struct_layouts: linked.native_struct_layouts.clone(),
        content_addressed: None,
        function_blob_hashes: linked
            .functions
            .iter()
            .map(|lf| {
                if lf.blob_hash == FunctionHash::ZERO {
                    None
                } else {
                    Some(lf.blob_hash)
                }
            })
            .collect(),
        monomorphization_keys: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "linker_tests.rs"]
mod tests;
