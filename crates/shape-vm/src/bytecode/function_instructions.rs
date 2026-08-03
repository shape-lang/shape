//! Checked views of the instructions directly owned by one function.
//!
//! Function bodies share one flat instruction stream. Compiling a nested
//! function emits its body inside the enclosing function's physical
//! `[entry_point, entry_point + body_length)` span, behind a jump-over. A
//! direct view keeps that enclosing jump but removes every contained function
//! body, so function-local instruction analysis cannot observe a nested
//! function's local-slot namespace.

use std::cmp::Reverse;
use std::fmt;
use std::ops::Range;

use super::{BytecodeProgram, Function, Instruction};

/// Malformed function metadata that prevents an unambiguous direct-owner view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FunctionWindowError {
    MissingFunction {
        function_index: usize,
        function_count: usize,
    },
    EndOverflow {
        function_index: usize,
        entry_point: usize,
        body_length: usize,
    },
    OutOfBounds {
        function_index: usize,
        window: Range<usize>,
        instruction_count: usize,
    },
    AmbiguousOverlap {
        target_function: usize,
        target_window: Range<usize>,
        other_function: usize,
        other_window: Range<usize>,
    },
    CrossingDescendants {
        target_function: usize,
        first_function: usize,
        first_window: Range<usize>,
        second_function: usize,
        second_window: Range<usize>,
    },
}

impl fmt::Display for FunctionWindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFunction {
                function_index,
                function_count,
            } => write!(
                formatter,
                "function {function_index} is missing from a table of {function_count} functions"
            ),
            Self::EndOverflow {
                function_index,
                entry_point,
                body_length,
            } => write!(
                formatter,
                "function {function_index} instruction window overflows: entry {entry_point}, \
                 length {body_length}"
            ),
            Self::OutOfBounds {
                function_index,
                window,
                instruction_count,
            } => write!(
                formatter,
                "function {function_index} instruction window [{}, {}) exceeds {instruction_count} \
                 instructions",
                window.start, window.end
            ),
            Self::AmbiguousOverlap {
                target_function,
                target_window,
                other_function,
                other_window,
            } => write!(
                formatter,
                "function {target_function} window [{}, {}) ambiguously overlaps function \
                 {other_function} window [{}, {})",
                target_window.start, target_window.end, other_window.start, other_window.end
            ),
            Self::CrossingDescendants {
                target_function,
                first_function,
                first_window,
                second_function,
                second_window,
            } => write!(
                formatter,
                "function {target_function} has crossing descendant windows: function \
                 {first_function} [{}, {}) and function {second_function} [{}, {})",
                first_window.start, first_window.end, second_window.start, second_window.end
            ),
        }
    }
}

impl std::error::Error for FunctionWindowError {}

#[derive(Debug)]
struct IndexedWindow {
    function_index: usize,
    range: Range<usize>,
}

/// Ascending iterator over instructions directly owned by one function.
pub(crate) struct DirectFunctionInstructions<'a> {
    instructions: &'a [Instruction],
    remaining_ranges: std::vec::IntoIter<Range<usize>>,
    current_range: Range<usize>,
}

impl<'a> Iterator for DirectFunctionInstructions<'a> {
    type Item = &'a Instruction;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(offset) = self.current_range.next() {
                return Some(&self.instructions[offset]);
            }
            self.current_range = self.remaining_ranges.next()?;
        }
    }
}

impl BytecodeProgram {
    /// Return the target function's instructions without physically embedded
    /// nested-function bodies.
    ///
    /// Every function-table row is range-checked before ownership is derived.
    /// Empty rows own no instructions. Non-empty windows must be laminar:
    /// disjoint, strictly containing, or strictly contained with a later entry.
    pub(crate) fn direct_function_instructions(
        &self,
        function_index: usize,
    ) -> Result<DirectFunctionInstructions<'_>, FunctionWindowError> {
        let owned_ranges = self.direct_function_windows(function_index)?;

        Ok(DirectFunctionInstructions {
            instructions: &self.instructions,
            remaining_ranges: owned_ranges.into_iter(),
            current_range: 0..0,
        })
    }

    /// Ascending instruction-offset ranges directly owned by one function.
    ///
    /// Same ownership derivation as `direct_function_instructions`, exposed as
    /// ranges for callers that need the offsets themselves (the bytecode
    /// verifier reports them). Empty rows own no ranges.
    pub(crate) fn direct_function_windows(
        &self,
        function_index: usize,
    ) -> Result<Vec<Range<usize>>, FunctionWindowError> {
        let target = self.program_window(function_index)?;
        let mut descendants = Vec::new();

        for (other_index, function) in self.functions.iter().enumerate() {
            if other_index == function_index {
                continue;
            }
            let other = checked_window(other_index, function, self.instructions.len())?;
            if other.is_empty() {
                continue;
            }

            if windows_are_disjoint(&target, &other) {
                continue;
            }
            if target.start < other.start && other.end <= target.end {
                descendants.push(IndexedWindow {
                    function_index: other_index,
                    range: other,
                });
                continue;
            }
            if other.start < target.start && target.end <= other.end {
                continue;
            }
            return Err(FunctionWindowError::AmbiguousOverlap {
                target_function: function_index,
                target_window: target,
                other_function: other_index,
                other_window: other,
            });
        }

        descendants.sort_unstable_by_key(|window| {
            (
                window.range.start,
                Reverse(window.range.end),
                window.function_index,
            )
        });
        let excluded = maximal_laminar_descendants(function_index, &descendants)?;
        Ok(subtract_ranges(target, &excluded))
    }

    fn program_window(&self, function_index: usize) -> Result<Range<usize>, FunctionWindowError> {
        let function =
            self.functions
                .get(function_index)
                .ok_or(FunctionWindowError::MissingFunction {
                    function_index,
                    function_count: self.functions.len(),
                })?;
        checked_window(function_index, function, self.instructions.len())
    }
}

fn checked_window(
    function_index: usize,
    function: &Function,
    instruction_count: usize,
) -> Result<Range<usize>, FunctionWindowError> {
    let end = function
        .entry_point
        .checked_add(function.body_length)
        .ok_or(FunctionWindowError::EndOverflow {
            function_index,
            entry_point: function.entry_point,
            body_length: function.body_length,
        })?;
    let window = function.entry_point..end;
    if window.start > instruction_count || window.end > instruction_count {
        return Err(FunctionWindowError::OutOfBounds {
            function_index,
            window,
            instruction_count,
        });
    }
    Ok(window)
}

fn windows_are_disjoint(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.end <= right.start || right.end <= left.start
}

fn maximal_laminar_descendants(
    target_function: usize,
    descendants: &[IndexedWindow],
) -> Result<Vec<Range<usize>>, FunctionWindowError> {
    let mut open: Vec<usize> = Vec::new();
    let mut maximal = Vec::new();

    for (index, descendant) in descendants.iter().enumerate() {
        while open
            .last()
            .is_some_and(|parent| descendants[*parent].range.end <= descendant.range.start)
        {
            open.pop();
        }

        if let Some(parent_index) = open.last().copied() {
            let parent = &descendants[parent_index];
            if descendant.range.start == parent.range.start
                || descendant.range.end > parent.range.end
            {
                return Err(FunctionWindowError::CrossingDescendants {
                    target_function,
                    first_function: parent.function_index,
                    first_window: parent.range.clone(),
                    second_function: descendant.function_index,
                    second_window: descendant.range.clone(),
                });
            }
        } else {
            maximal.push(descendant.range.clone());
        }
        open.push(index);
    }

    Ok(maximal)
}

fn subtract_ranges(target: Range<usize>, excluded: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut owned = Vec::new();
    let mut cursor = target.start;
    for range in excluded {
        if cursor < range.start {
            owned.push(cursor..range.start);
        }
        cursor = range.end;
    }
    if cursor < target.end {
        owned.push(cursor..target.end);
    }
    owned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{OpCode, Operand};

    fn function(name: &str, entry_point: usize, body_length: usize) -> Function {
        Function {
            name: name.to_string(),
            arity: 0,
            param_names: Vec::new(),
            locals_count: 0,
            entry_point,
            body_length,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
        }
    }

    fn program(instruction_count: usize, windows: &[(usize, usize)]) -> BytecodeProgram {
        let mut program = BytecodeProgram::new();
        program.instructions = (0..instruction_count)
            .map(|offset| Instruction::new(OpCode::Nop, Some(Operand::Const(offset as u16))))
            .collect();
        program.functions = windows
            .iter()
            .enumerate()
            .map(|(index, (entry, length))| function(&format!("f{index}"), *entry, *length))
            .collect();
        program
    }

    fn direct_offsets(program: &BytecodeProgram, function_index: usize) -> Vec<u16> {
        program
            .direct_function_instructions(function_index)
            .expect("valid direct function window")
            .map(|instruction| match instruction.operand {
                Some(Operand::Const(offset)) => offset,
                operand => panic!("expected offset marker, got {operand:?}"),
            })
            .collect()
    }

    #[test]
    fn excludes_maximal_child_subtrees_and_preserves_parent_order() {
        let program = program(
            16,
            &[
                (1, 14), // target [1, 15)
                (3, 6),  // child [3, 9)
                (5, 2),  // grandchild [5, 7)
                (11, 4), // sibling child [11, 15)
            ],
        );

        assert_eq!(direct_offsets(&program, 0), vec![1, 2, 9, 10]);
    }

    #[test]
    fn result_is_independent_of_descendant_table_order() {
        let first = program(16, &[(1, 14), (3, 6), (5, 2), (11, 4)]);
        let permuted = program(16, &[(1, 14), (11, 4), (3, 6), (5, 2)]);

        assert_eq!(direct_offsets(&first, 0), direct_offsets(&permuted, 0));
    }

    #[test]
    fn ancestors_disjoint_siblings_and_touching_windows_leave_target_untouched() {
        let program = program(
            13,
            &[
                (4, 4), // target [4, 8)
                (1, 10),
                (0, 4),
                (8, 2),
                (11, 2),
            ],
        );

        assert_eq!(direct_offsets(&program, 0), vec![4, 5, 6, 7]);
    }

    #[test]
    fn empty_rows_own_nothing_at_any_target_position() {
        let non_empty_target = program(8, &[(2, 4), (2, 0), (4, 0), (6, 0), (8, 0)]);
        assert_eq!(direct_offsets(&non_empty_target, 0), vec![2, 3, 4, 5]);

        let empty_target = program(8, &[(4, 0), (4, 0)]);
        assert!(direct_offsets(&empty_target, 0).is_empty());
    }

    #[test]
    fn missing_overflowing_and_out_of_bounds_windows_reject() {
        let missing = program(1, &[(0, 1)]);
        assert!(matches!(
            missing.direct_function_instructions(1),
            Err(FunctionWindowError::MissingFunction { .. })
        ));

        let overflow = program(1, &[(usize::MAX, 1)]);
        assert!(matches!(
            overflow.direct_function_instructions(0),
            Err(FunctionWindowError::EndOverflow { .. })
        ));

        let other_overflow = program(1, &[(0, 1), (usize::MAX, 1)]);
        assert!(matches!(
            other_overflow.direct_function_instructions(0),
            Err(FunctionWindowError::EndOverflow {
                function_index: 1,
                ..
            })
        ));

        let target_out_of_bounds = program(4, &[(3, 2)]);
        assert!(matches!(
            target_out_of_bounds.direct_function_instructions(0),
            Err(FunctionWindowError::OutOfBounds {
                function_index: 0,
                ..
            })
        ));

        let other_out_of_bounds = program(4, &[(0, 2), (5, 0)]);
        assert!(matches!(
            other_out_of_bounds.direct_function_instructions(0),
            Err(FunctionWindowError::OutOfBounds {
                function_index: 1,
                ..
            })
        ));
    }

    #[test]
    fn identical_same_entry_and_target_crossing_windows_reject() {
        for windows in [
            vec![(2, 4), (2, 4)],
            vec![(2, 4), (2, 2)],
            vec![(2, 4), (2, 6)],
            vec![(2, 4), (0, 4)],
            vec![(2, 4), (4, 4)],
        ] {
            let program = program(8, &windows);
            assert!(matches!(
                program.direct_function_instructions(0),
                Err(FunctionWindowError::AmbiguousOverlap { .. })
            ));
        }
    }

    #[test]
    fn crossing_and_aliased_descendant_windows_reject() {
        for windows in [
            vec![(0, 12), (2, 5), (5, 5)],
            vec![(0, 12), (2, 5), (2, 3)],
            vec![(0, 12), (2, 5), (2, 5)],
        ] {
            let program = program(12, &windows);
            assert!(matches!(
                program.direct_function_instructions(0),
                Err(FunctionWindowError::CrossingDescendants { .. })
            ));
        }
    }
}
